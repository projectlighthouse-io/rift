//! The Chart tab of a query result: settings on the left, the chart on the right. Aggregation
//! runs in the database through `Aggregate` over the statement as a subquery.

use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::prelude::*;
use gpui::{
    Context, Entity, EventEmitter, FontWeight, SharedString, Subscription, Task, Window, div, px,
};
use tokio_util::sync::CancellationToken;
use traits::Connection;
use types::{
    AggFn, AggregateRequest, ChartData, ChartKind, ChartSource, ChartSpec, Column, Error,
    MAX_CHART_POINTS, TimeBucket,
};

use crate::chart::{self, ChartModel};
use crate::dropdown::{Changed, Dropdown};
use crate::theme::{self, c};
use crate::{runtime, ui};

const KINDS: [(&str, ChartKind); 4] = [
    ("Line", ChartKind::Line),
    ("Bar", ChartKind::Bar),
    ("Area", ChartKind::Area),
    ("Scatter", ChartKind::Scatter),
];
const AGGS: [(&str, AggFn); 6] = [
    ("none (already grouped)", AggFn::None),
    ("count", AggFn::Count),
    ("sum", AggFn::Sum),
    ("avg", AggFn::Avg),
    ("min", AggFn::Min),
    ("max", AggFn::Max),
];

/// Asks the session to pin this chart to a dashboard.
pub(crate) struct PinChart {
    pub(crate) title: String,
    pub(crate) subtitle: String,
    pub(crate) spec: ChartSpec,
}

/// Chart settings and the last aggregation result.
pub(crate) struct ChartPanel {
    conn: Arc<dyn Connection>,
    sql: String,
    columns: Vec<Column>,
    kind: ChartKind,
    x: Entity<Dropdown>,
    y: Entity<Dropdown>,
    agg: Entity<Dropdown>,
    split: Entity<Dropdown>,
    show_points: bool,
    data: Option<ChartData>,
    elapsed: Option<Duration>,
    error: Option<String>,
    note: Option<String>,
    cancel: Option<CancellationToken>,
    _task: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PinChart> for ChartPanel {}

fn is_time(column: &Column) -> bool {
    let t = column.type_name.to_lowercase();
    ["date", "time", "timestamp"].iter().any(|k| t.contains(k))
}

fn is_number(column: &Column) -> bool {
    let t = column.type_name.to_lowercase();
    [
        "int", "numeric", "decimal", "float", "double", "real", "number",
    ]
    .iter()
    .any(|k| t.contains(k))
}

impl ChartPanel {
    pub(crate) fn new(
        conn: Arc<dyn Connection>,
        sql: String,
        columns: Vec<Column>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let labels: Vec<SharedString> = columns
            .iter()
            .map(|c| format!("{}  ({})", c.name, c.type_name).into())
            .collect();
        let names: Vec<SharedString> = columns.iter().map(|c| c.name.to_string().into()).collect();
        let y_ix = columns
            .iter()
            .enumerate()
            .skip(1)
            .find(|(_, c)| is_number(c))
            .map_or(0, |(i, _)| i);
        let x = cx.new(|_| Dropdown::new(labels, 0).mono());
        let y = cx.new(|_| Dropdown::new(names.clone(), y_ix).mono());
        let agg = cx.new(|_| Dropdown::new(AGGS.iter().map(|a| a.0.into()).collect(), 0));
        let mut split_names: Vec<SharedString> = vec!["—".into()];
        split_names.extend(names);
        let split = cx.new(|_| Dropdown::new(split_names, 0).mono());
        let subscriptions = [&x, &y, &agg, &split]
            .into_iter()
            .map(|d| cx.subscribe_in(d, window, |this, _, _: &Changed, _, cx| this.refresh(cx)))
            .collect();
        let mut panel = Self {
            conn,
            sql,
            columns,
            kind: ChartKind::Line,
            x,
            y,
            agg,
            split,
            show_points: true,
            data: None,
            elapsed: None,
            error: None,
            note: None,
            cancel: None,
            _task: None,
            _subscriptions: subscriptions,
        };
        panel.refresh(cx);
        panel
    }

    fn request(&self, cx: &gpui::App) -> AggregateRequest {
        let x = &self.columns[self.x.read(cx).selected().min(self.columns.len() - 1)];
        let agg = AGGS[self.agg.read(cx).selected()].1;
        let split = self.split.read(cx).selected();
        AggregateRequest {
            source: ChartSource::Query(self.sql.clone()),
            x: x.name.clone(),
            bucket: (agg != AggFn::None && is_time(x)).then_some(TimeBucket::Day),
            y: self
                .columns
                .get(self.y.read(cx).selected())
                .map(|c| c.name.clone()),
            agg,
            split_by: (split > 0).then(|| self.columns[split - 1].name.clone()),
            limit: MAX_CHART_POINTS,
        }
    }

    /// Runs the aggregation again for the current settings.
    pub(crate) fn refresh(&mut self, cx: &mut Context<Self>) {
        if self.columns.is_empty() {
            return;
        }
        if let Some(token) = self.cancel.take() {
            token.cancel();
        }
        let token = CancellationToken::new();
        self.cancel = Some(token.clone());
        let request = self.request(cx);
        let conn = self.conn.clone();
        let started = Instant::now();
        self._task = Some(runtime::spawn_db(
            cx,
            async move {
                match conn.aggregate() {
                    Some(a) => a.aggregate(&request, token).await,
                    None => Err(Error::unsupported("charts")),
                }
            },
            move |this, result, cx| {
                this.elapsed = Some(started.elapsed());
                match result {
                    Ok(data) => {
                        this.data = Some(data);
                        this.error = None;
                    }
                    Err(Error::Cancelled) => {}
                    Err(e) => this.error = Some(e.to_string()),
                }
                cx.notify();
            },
        ));
        cx.notify();
    }

    fn title(&self, cx: &gpui::App) -> String {
        let req = self.request(cx);
        let y = match req.agg {
            AggFn::Count => "Rows".to_owned(),
            _ => req.y.map_or_else(|| "Values".to_owned(), |y| y.to_string()),
        };
        let mut y = y.replace('_', " ");
        if let Some(first) = y.get(..1) {
            y = format!("{}{}", first.to_uppercase(), &y[1..]);
        }
        format!("{y} per {}", req.x.as_str().replace('_', " "))
    }

    fn model(&self) -> Option<ChartModel> {
        Some(ChartModel {
            kind: self.kind,
            data: self.data.clone()?,
            show_points: self.show_points,
        })
    }

    /// Status line for the editor: `14 points · 182 ms · grouped in ClickHouse`.
    pub(crate) fn status(&self) -> Option<(bool, String)> {
        if let Some(e) = &self.error {
            return Some((false, e.clone()));
        }
        let data = self.data.as_ref()?;
        let points: usize = data.series.iter().map(|s| s.points.len()).sum();
        let engine = self
            .conn
            .info()
            .server_version
            .split_whitespace()
            .next()
            .unwrap_or("the database")
            .to_owned();
        let mut text = format!(
            "{} points · {} ms · grouped in {engine}",
            ui::thousands(points as u64),
            self.elapsed.map_or(0, |d| d.as_millis())
        );
        if data.truncated {
            text.push_str(&format!(
                " · capped at {} points",
                ui::thousands(u64::from(MAX_CHART_POINTS))
            ));
        }
        Some((true, text))
    }

    fn export(&mut self, cx: &mut Context<Self>) {
        let Some(model) = self.model() else { return };
        let title = self.title(cx);
        let dir = dirs::download_dir().unwrap_or_else(std::env::temp_dir);
        let path = cx.prompt_for_new_path(&dir, Some("chart.png"));
        cx.spawn(async move |this, cx| {
            if let Ok(Ok(Some(path))) = path.await {
                let result = chart::png(&model, &title)
                    .and_then(|bytes| std::fs::write(&path, bytes).map_err(|e| e.to_string()));
                let _ = this.update(cx, |p, cx| {
                    p.note = Some(match result {
                        Ok(()) => format!("Saved {}", path.display()),
                        Err(e) => format!("Export failed: {e}"),
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn render_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = |t: &'static str| {
            div()
                .text_size(px(12.5))
                .font_weight(FontWeight::MEDIUM)
                .text_color(c(theme::MUTED))
                .child(t)
        };
        let group = |t: &'static str, control: gpui::AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(px(6.))
                .child(label(t))
                .child(control)
        };
        let kinds: [(&'static str, bool); 4] = KINDS.map(|(l, k)| (l, k == self.kind));
        div()
            .id("chart-settings")
            .overflow_y_scroll()
            .flex()
            .flex_col()
            .gap(px(16.))
            .flex_none()
            .w(px(272.))
            .h_full()
            .p(px(16.))
            .bg(c(theme::BG))
            .border_r_1()
            .border_color(c(theme::LINE))
            .child(group(
                "Chart type",
                ui::segmented(
                    kinds,
                    cx.listener(|this, ix: &usize, _, cx| {
                        this.kind = KINDS[*ix].1;
                        cx.notify();
                    }),
                )
                .into_any_element(),
            ))
            .child(group("X axis", self.x.clone().into_any_element()))
            .child(group("Y axis", self.y.clone().into_any_element()))
            .child(group("Aggregate", self.agg.clone().into_any_element()))
            .child(group("Split by", self.split.clone().into_any_element()))
            .child(
                div()
                    .id("show-points")
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .cursor_pointer()
                    .text_size(px(13.))
                    .child(
                        div()
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.))
                            .bg(c(if self.show_points {
                                theme::PRIMARY
                            } else {
                                theme::BG
                            }))
                            .border_1()
                            .border_color(c(if self.show_points {
                                theme::PRIMARY
                            } else {
                                theme::LINE
                            }))
                            .children(self.show_points.then(|| ui::icon("check", 12., theme::BG))),
                    )
                    .child("Show points")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_points = !this.show_points;
                        cx.notify();
                    })),
            )
    }
}

impl Render for ChartPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _span = tracing::debug_span!("ChartPanel::render").entered();
        let title = self.title(cx);
        let range = self.data.as_ref().and_then(chart::date_range);
        let body = match (self.model(), &self.error) {
            (_, Some(e)) => div()
                .p(px(16.))
                .text_color(c(theme::DELETED_TEXT))
                .child(SharedString::from(e.clone()))
                .into_any_element(),
            (Some(model), None) => chart::chart_element(model).into_any_element(),
            (None, None) => div()
                .p(px(16.))
                .text_color(c(theme::FAINT))
                .child("Aggregating…")
                .into_any_element(),
        };
        let spec_title = title.clone();
        div()
            .flex()
            .size_full()
            .child(self.render_settings(cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .p(px(20.))
                    .gap(px(12.))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(10.))
                            .child(
                                div()
                                    .text_size(px(15.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(SharedString::from(title)),
                            )
                            .children(range.map(|r| {
                                div()
                                    .text_size(px(12.))
                                    .text_color(c(theme::FAINT))
                                    .child(SharedString::from(r))
                            }))
                            .children(self.note.clone().map(|n| {
                                div()
                                    .text_size(px(12.))
                                    .text_color(c(theme::MUTED))
                                    .child(SharedString::from(n))
                            }))
                            .child(
                                div()
                                    .ml_auto()
                                    .flex()
                                    .gap(px(8.))
                                    .child(
                                        ui::ghost_button("export-png", "Export PNG")
                                            .flex_row_reverse()
                                            .child(ui::icon("download", 14., theme::MUTED))
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.export(cx)),
                                            ),
                                    )
                                    .child(
                                        ui::secondary_button("pin", "Pin to dashboard")
                                            .flex_row_reverse()
                                            .child(ui::icon("pin", 14., theme::TEXT))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                let spec = ChartSpec {
                                                    kind: this.kind,
                                                    request: this.request(cx),
                                                    show_points: this.show_points,
                                                };
                                                let subtitle =
                                                    match (spec.request.agg, &spec.request.y) {
                                                        (AggFn::None, Some(y)) => y.to_string(),
                                                        (AggFn::Count, _) => "count(*)".to_owned(),
                                                        (f, y) => format!(
                                                            "{}({})",
                                                            format!("{f:?}").to_lowercase(),
                                                            y.as_ref().map_or(
                                                                "*".to_owned(),
                                                                ToString::to_string
                                                            )
                                                        ),
                                                    };
                                                cx.emit(PinChart {
                                                    title: spec_title.clone(),
                                                    subtitle,
                                                    spec,
                                                });
                                                this.note = Some("Pinned".into());
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    )
                    .child(div().flex_1().min_h_0().child(body)),
            )
    }
}
