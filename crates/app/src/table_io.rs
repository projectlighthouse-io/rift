//! Whole-table export (CSV, JSON, SQL) and CSV import with a column mapping and preview.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use dialect::Dialect;
use gpui::prelude::*;
use gpui::{Context, Entity, EventEmitter, FontWeight, SharedString, Task, Window, div, px};
use traits::Connection;
use types::{
    Column, ColumnName, ColumnValue, Error, Filter, Row, RowEdit, RowEditKind, RowsRequest,
    TableRef, Value,
};

use crate::dropdown::Dropdown;
use crate::grid::Grid;
use crate::theme::{self, c};
use crate::{export, runtime, ui};

/// Rows fetched per page while exporting.
const EXPORT_PAGE: u64 = 5_000;
/// Rows inserted per `EditRows::apply` call while importing.
const IMPORT_BATCH: usize = 500;
const PREVIEW_ROWS: usize = 20;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Format {
    Csv,
    Json,
    Sql,
}

impl Format {
    pub(crate) const ALL: [(Format, &'static str); 3] = [
        (Format::Csv, "CSV"),
        (Format::Json, "JSON"),
        (Format::Sql, "SQL"),
    ];

    pub(crate) fn extension(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Json => "json",
            Format::Sql => "sql",
        }
    }
}

/// Streams every row of `table` matching `filters` to `path`; returns the row count.
pub(crate) async fn export_table(
    conn: Arc<dyn Connection>,
    table: TableRef,
    filters: Vec<Filter>,
    format: Format,
    path: PathBuf,
) -> Result<u64, Error> {
    let browse = conn
        .browse()
        .ok_or_else(|| Error::unsupported("browsing rows"))?;
    let io = |e: std::io::Error| Error::query(format!("writing {}: {e}", path.display()));
    let mut out = std::io::BufWriter::new(std::fs::File::create(&path).map_err(io)?);
    let dialect = Dialect::for_driver(conn.info().driver.as_str());
    let mut offset = 0u64;
    loop {
        let request = RowsRequest {
            filters: filters.clone(),
            offset,
            limit: EXPORT_PAGE,
            ..RowsRequest::default()
        };
        let page = browse.rows(&table, &request).await?;
        let rows: Vec<&[Value]> = page.rows.iter().map(|r| r.values.as_slice()).collect();
        let first = offset == 0;
        let chunk = match format {
            Format::Csv => {
                let text = export::csv(&page.columns, rows.iter().copied());
                // The header line only once.
                if first {
                    text
                } else {
                    text.split_once('\n')
                        .map_or(String::new(), |(_, rest)| rest.to_owned())
                }
            }
            Format::Json => {
                let mut text = String::from(if first { "[\n" } else { "" });
                for (i, row) in rows.iter().enumerate() {
                    if !(first && i == 0) {
                        text.push_str(",\n");
                    }
                    let object: serde_json::Map<String, serde_json::Value> = page
                        .columns
                        .iter()
                        .zip(row.iter())
                        .map(|(c, v)| (c.name.to_string(), export::json_value(v)))
                        .collect();
                    text.push_str("  ");
                    text.push_str(&serde_json::to_string(&object).unwrap_or_default());
                }
                text
            }
            Format::Sql => {
                let d = dialect.unwrap_or(Dialect::Postgres);
                export::inserts(d, &d.table(&table), &page.columns, rows.iter().copied())
            }
        };
        out.write_all(chunk.as_bytes()).map_err(io)?;
        let fetched = page.rows.len() as u64;
        offset += fetched;
        if fetched < EXPORT_PAGE {
            break;
        }
    }
    if format == Format::Json {
        out.write_all(if offset == 0 { b"[]\n" } else { b"\n]\n" })
            .map_err(io)?;
    }
    out.flush().map_err(io)?;
    Ok(offset)
}

/// A parsed CSV file.
pub(crate) struct CsvFile {
    pub(crate) name: String,
    pub(crate) headers: Vec<String>,
    pub(crate) rows: Vec<Vec<String>>,
}

/// Header and rows of a CSV file. Empty fields become `NULL` on import.
pub(crate) fn parse_csv(text: &str) -> Result<(Vec<String>, Vec<Vec<String>>), String> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(text.as_bytes());
    let headers = reader
        .headers()
        .map_err(|e| e.to_string())?
        .iter()
        .map(str::to_owned)
        .collect();
    let mut rows = Vec::new();
    for record in reader.records() {
        rows.push(
            record
                .map_err(|e| e.to_string())?
                .iter()
                .map(str::to_owned)
                .collect(),
        );
    }
    Ok((headers, rows))
}

fn cell(text: &str) -> Value {
    if text.is_empty() {
        Value::Null
    } else {
        Value::Text(text.to_owned())
    }
}

/// One `INSERT` edit per CSV row, taking each table column from its mapped CSV column.
pub(crate) fn insert_edits(
    table: &TableRef,
    mapped: &[(ColumnName, usize)],
    rows: &[Vec<String>],
) -> Vec<RowEdit> {
    rows.iter()
        .map(|r| RowEdit {
            table: table.clone(),
            primary_key: Vec::new(),
            kind: RowEditKind::Insert {
                values: mapped
                    .iter()
                    .map(|(c, i)| {
                        ColumnValue::new(c.clone(), cell(r.get(*i).map_or("", String::as_str)))
                    })
                    .collect(),
            },
        })
        .collect()
}

/// The import finished (or was cancelled); `Some(rows)` when rows were inserted.
pub(crate) struct ImportDone(pub(crate) Option<u64>);

/// Maps CSV columns to table columns, previews, and inserts.
pub(crate) struct ImportPanel {
    conn: Arc<dyn Connection>,
    table: TableRef,
    file: String,
    rows: Vec<Vec<String>>,
    columns: Vec<Column>,
    /// Per table column: CSV column picker; option 0 skips.
    mapping: Vec<Entity<Dropdown>>,
    preview: Entity<Grid>,
    error: Option<String>,
    importing: bool,
    _task: Option<Task<()>>,
}

impl EventEmitter<ImportDone> for ImportPanel {}

impl ImportPanel {
    pub(crate) fn new(
        conn: Arc<dyn Connection>,
        table: TableRef,
        columns: Vec<Column>,
        csv: CsvFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let CsvFile {
            name: file,
            headers,
            rows,
        } = csv;
        let mut options: Vec<SharedString> = vec!["— skip —".into()];
        options.extend(headers.iter().map(|h| SharedString::from(h.clone())));
        let mapping: Vec<_> = columns
            .iter()
            .map(|col| {
                let pick = headers
                    .iter()
                    .position(|h| h.eq_ignore_ascii_case(col.name.as_str()))
                    .map_or(0, |i| i + 1);
                let options = options.clone();
                cx.new(|_| Dropdown::new(options, pick).height(28.))
            })
            .collect();
        for d in &mapping {
            cx.subscribe_in(d, window, |this, _, _: &crate::dropdown::Changed, _, cx| {
                this.refresh_preview(cx)
            })
            .detach();
        }
        let mut panel = Self {
            conn,
            table,
            file,
            rows,
            columns,
            mapping,
            preview: cx.new(|_| Grid::new()),
            error: None,
            importing: false,
            _task: None,
        };
        panel.refresh_preview(cx);
        panel
    }

    /// `(table column, CSV column index)` for every mapped column.
    fn mapped(&self, cx: &gpui::App) -> Vec<(Column, usize)> {
        self.columns
            .iter()
            .zip(&self.mapping)
            .filter_map(|(col, d)| {
                d.read(cx)
                    .selected()
                    .checked_sub(1)
                    .map(|i| (col.clone(), i))
            })
            .collect()
    }

    fn refresh_preview(&mut self, cx: &mut Context<Self>) {
        let mapped = self.mapped(cx);
        let columns: Vec<Column> = mapped.iter().map(|(c, _)| c.clone()).collect();
        let rows: Vec<Row> = self
            .rows
            .iter()
            .take(PREVIEW_ROWS)
            .map(|r| Row {
                values: mapped
                    .iter()
                    .map(|(_, i)| cell(r.get(*i).map_or("", String::as_str)))
                    .collect(),
            })
            .collect();
        self.preview.update(cx, |g, cx| {
            g.set_data(columns, rows, 1);
            cx.notify();
        });
        cx.notify();
    }

    fn import(&mut self, cx: &mut Context<Self>) {
        let mapped = self.mapped(cx);
        if mapped.is_empty() {
            self.error = Some("Map at least one column.".into());
            cx.notify();
            return;
        }
        let pairs: Vec<(ColumnName, usize)> =
            mapped.iter().map(|(c, i)| (c.name.clone(), *i)).collect();
        let edits = insert_edits(&self.table, &pairs, &self.rows);
        let conn = self.conn.clone();
        self.importing = true;
        self.error = None;
        self._task = Some(runtime::spawn_db(
            cx,
            async move {
                let editor = conn
                    .edit_rows()
                    .ok_or_else(|| (0, Error::unsupported("inserting rows")))?;
                let mut inserted = 0;
                // ponytail: one transaction per batch, so a failure keeps earlier batches.
                // Wrap everything in one transaction if partial imports turn out to hurt.
                for batch in edits.chunks(IMPORT_BATCH) {
                    inserted += editor.apply(batch).await.map_err(|e| (inserted, e))?;
                }
                Ok::<u64, (u64, Error)>(inserted)
            },
            |this: &mut Self, result, cx| {
                this.importing = false;
                match result {
                    Ok(n) => cx.emit(ImportDone(Some(n))),
                    Err((done, e)) => {
                        this.error =
                            Some(format!("Stopped after {} rows: {e}", ui::thousands(done)));
                        cx.notify();
                    }
                }
            },
        ));
        cx.notify();
    }
}

impl Render for ImportPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = tracing::debug_span!("ImportPanel::render").entered();
        let rows = self.rows.len() as u64;
        div()
            .id("import-panel")
            .occlude()
            .w(px(760.))
            .max_h(px(640.))
            .flex()
            .flex_col()
            .rounded(px(10.))
            .bg(c(theme::BG))
            .border_1()
            .border_color(c(theme::LINE))
            .shadow_lg()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(16.))
                    .h(px(48.))
                    .border_b_1()
                    .border_color(c(theme::LINE))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child("Import CSV"),
                    )
                    .child(div().text_size(px(12.5)).text_color(c(theme::MUTED)).child(
                        SharedString::from(format!(
                            "{} · {} rows into {}",
                            self.file,
                            ui::thousands(rows),
                            self.table
                        )),
                    )),
            )
            .child(
                div()
                    .id("import-mapping")
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .p(px(16.))
                    .max_h(px(220.))
                    .overflow_y_scroll()
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(c(theme::MUTED))
                            .child("Column mapping"),
                    )
                    .children(self.columns.iter().zip(&self.mapping).map(|(col, d)| {
                        div()
                            .flex()
                            .items_center()
                            .gap(px(12.))
                            .child(
                                div()
                                    .w(px(200.))
                                    .text_size(px(12.5))
                                    .child(SharedString::from(format!(
                                        "{}  {}",
                                        col.name, col.type_name
                                    ))),
                            )
                            .child(div().text_color(c(theme::FAINT)).child("←"))
                            .child(div().w(px(220.)).child(d.clone()))
                    })),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .px(px(16.))
                    .text_color(c(theme::MUTED))
                    .child(SharedString::from(format!(
                        "Preview (first {} rows)",
                        PREVIEW_ROWS.min(self.rows.len())
                    ))),
            )
            .child(
                div()
                    .h(px(220.))
                    .mx(px(16.))
                    .mt(px(6.))
                    .border_1()
                    .border_color(c(theme::LINE))
                    .child(self.preview.clone()),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .p(px(16.))
                    .children(self.error.clone().map(|e| {
                        div()
                            .flex_1()
                            .text_size(px(12.5))
                            .text_color(c(theme::DELETED_TEXT))
                            .child(SharedString::from(e))
                    }))
                    .child(
                        div()
                            .ml_auto()
                            .flex()
                            .gap(px(8.))
                            .child(
                                ui::ghost_button("import-cancel", "Cancel")
                                    .on_click(cx.listener(|_, _, _, cx| cx.emit(ImportDone(None)))),
                            )
                            .child(
                                ui::primary_button(
                                    "import-run",
                                    if self.importing {
                                        "Importing…".to_owned()
                                    } else {
                                        format!("Import {} rows", ui::thousands(rows))
                                    },
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if !this.importing {
                                            this.import(cx);
                                        }
                                    },
                                )),
                            ),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use types::{Column, Value};

    #[test]
    fn table_round_trips_through_csv_export_and_import() {
        use futures::executor::block_on;
        use std::sync::Arc;
        use traits::Connection;
        use types::{ColumnValue, RowEdit, RowEditKind};

        let conn = Arc::new(testkit::FakeSql::new());
        let table = testkit::FakeSql::fixture().table;
        let path = std::env::temp_dir().join(format!("rift-roundtrip-{}.csv", std::process::id()));
        let export = |path: std::path::PathBuf| {
            block_on(super::export_table(
                conn.clone(),
                table.clone(),
                Vec::new(),
                super::Format::Csv,
                path,
            ))
        };
        assert_eq!(export(path.clone()).expect("export"), 3);
        let before = std::fs::read_to_string(&path).expect("read");
        let deletes: Vec<RowEdit> = (1..=3)
            .map(|id| RowEdit {
                table: table.clone(),
                primary_key: vec![ColumnValue::new("id", Value::Int(id))],
                kind: RowEditKind::Delete,
            })
            .collect();
        block_on(conn.edit_rows().expect("edit").apply(&deletes)).expect("delete");
        let (headers, rows) = super::parse_csv(&before).expect("parse");
        let mapped: Vec<_> = headers
            .iter()
            .enumerate()
            .map(|(i, h)| (h.as_str().into(), i))
            .collect();
        let inserts = super::insert_edits(&table, &mapped, &rows);
        block_on(conn.edit_rows().expect("edit").apply(&inserts)).expect("import");
        export(path.clone()).expect("export again");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            before,
            "same table after round trip"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn csv_round_trips_through_export() {
        let columns = vec![Column::new("id", "int8"), Column::new("note", "text")];
        let rows = [
            vec![Value::Int(1), Value::Text("plain".into())],
            vec![
                Value::Int(2),
                Value::Text("comma, \"quote\"\nnewline".into()),
            ],
            vec![Value::Int(3), Value::Null],
        ];
        let text = crate::export::csv(&columns, rows.iter().map(Vec::as_slice));
        let (headers, parsed) = super::parse_csv(&text).expect("parse");
        assert_eq!(headers, ["id", "note"]);
        let back: Vec<Vec<Value>> = parsed
            .iter()
            .map(|r| r.iter().map(|c| super::cell(c)).collect())
            .collect();
        assert_eq!(back[1][1], Value::Text("comma, \"quote\"\nnewline".into()));
        assert_eq!(back[2][1], Value::Null);
        assert_eq!(back.len(), 3);
    }
}
