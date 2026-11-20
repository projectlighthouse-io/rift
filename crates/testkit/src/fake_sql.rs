//! An in-memory SQL driver that understands just enough SQL for the conformance suites.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

use futures::future::BoxFuture;
use futures::{FutureExt, stream};
use tokio_util::sync::CancellationToken;
use traits::{
    Aggregate, Browse, ColumnStats, ConnectDeps, Connection, Describe, DriverFactory, EditRows,
    EditSchema, Explain, Query, RowStream, StatementOutput, Transactions,
};
use types::{
    AggFn, AggregateRequest, ChartData, ChartPoint, ChartSource, Column, ColumnDef, ColumnName,
    ColumnStatsReport, ColumnValue, ConnConfig, ConnectionInfo, DbObject, DriverInfo, Error,
    ExplainPlan, Family, FieldKind, FieldSpec, ObjectKind, PAGE_SIZE, Page, Row, RowEdit,
    RowEditKind, RowsRequest, SchemaChange, SchemaName, Series, SortDir, TableDescription,
    TableRef, TxOp, Value, ValueCount,
};

use crate::compare;
use crate::suites::SqlFixture;

const SCHEMA: &str = "main";
const FIXTURE_TABLE: &str = "rift_conformance";

#[derive(Clone)]
struct FakeTable {
    desc: TableDescription,
    rows: Vec<Vec<Value>>,
}

impl FakeTable {
    fn column_index(&self, name: &ColumnName) -> Result<usize, Error> {
        self.desc
            .columns
            .iter()
            .position(|c| &c.name == name)
            .ok_or_else(|| Error::query(format!("column \"{name}\" does not exist")))
    }

    fn columns(&self) -> Vec<Column> {
        self.desc
            .columns
            .iter()
            .map(|c| Column {
                name: c.name.clone(),
                type_name: c.type_name.clone(),
                nullable: Some(c.nullable),
                primary_key: c.primary_key,
                references: c.references.clone(),
            })
            .collect()
    }

    fn find_row(&self, key: &[ColumnValue]) -> Result<Option<usize>, Error> {
        let indexes = key
            .iter()
            .map(|kv| self.column_index(&kv.column))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(self.rows.iter().position(|row| {
            indexes
                .iter()
                .zip(key)
                .all(|(&i, kv)| compare::same(&row[i], &kv.value))
        }))
    }
}

/// An in-memory SQL connection seeded with the conformance table. Supports every SQL
/// capability so the suites can run without Docker.
pub struct FakeSql {
    info: ConnectionInfo,
    tables: Mutex<BTreeMap<TableRef, FakeTable>>,
}

impl Default for FakeSql {
    fn default() -> Self {
        Self::new()
    }
}

impl FakeSql {
    /// A connection holding `main.rift_conformance` with three rows.
    #[must_use]
    pub fn new() -> Self {
        let table = Self::fixture().table;
        let desc = TableDescription {
            table: table.clone(),
            columns: vec![
                col("id", "int8", false, true),
                col("name", "text", false, false),
                col("score", "float8", true, false),
            ],
            indexes: Vec::new(),
            foreign_keys: Vec::new(),
            row_estimate: Some(3),
            size_bytes: None,
        };
        let rows = vec![
            vec![
                Value::Int(1),
                Value::Text("alpha".into()),
                Value::Float(1.5),
            ],
            vec![Value::Int(2), Value::Text("beta".into()), Value::Null],
            vec![
                Value::Int(3),
                Value::Text("gamma".into()),
                Value::Float(3.0),
            ],
        ];
        Self {
            info: ConnectionInfo {
                driver: "fake-sql".into(),
                server_version: "Fake SQL 1.0".into(),
                address: "memory".into(),
                database: Some(SCHEMA.into()),
            },
            tables: Mutex::new(BTreeMap::from([(table, FakeTable { desc, rows })])),
        }
    }

    /// The fixture matching the seeded table.
    #[must_use]
    pub fn fixture() -> SqlFixture {
        SqlFixture {
            table: TableRef::new(Some(SchemaName::new(SCHEMA)), FIXTURE_TABLE),
            select_all_sql: format!("SELECT id, name, score FROM {FIXTURE_TABLE} ORDER BY id"),
            delete_first_sql: format!("DELETE FROM {FIXTURE_TABLE} WHERE id = 1"),
            slow_sql: "SELECT sleep".into(),
        }
    }

    fn lock(&self) -> MutexGuard<'_, BTreeMap<TableRef, FakeTable>> {
        self.tables.lock().expect("fake tables poisoned")
    }

    fn with_table<T>(
        &self,
        table: &TableRef,
        f: impl FnOnce(&mut FakeTable) -> Result<T, Error>,
    ) -> Result<T, Error> {
        let mut tables = self.lock();
        let found = tables
            .get_mut(table)
            .ok_or_else(|| Error::not_found(table.to_string()))?;
        f(found)
    }

    fn table_by_name(&self, name: &str) -> Result<TableRef, Error> {
        let name = name.trim_matches('"');
        self.lock()
            .keys()
            .find(|t| t.name.as_str() == name || t.to_string() == name)
            .cloned()
            .ok_or_else(|| Error::query(format!("relation \"{name}\" does not exist")))
    }

    fn run(&self, stmt: Stmt) -> Result<Outcome, Error> {
        match stmt {
            Stmt::Literal(n) => Ok(Outcome::Rows(Page {
                columns: vec![Column::new("?column?", "int8")],
                rows: vec![Row {
                    values: vec![Value::Int(n)],
                }],
            })),
            Stmt::Sleep => Ok(Outcome::Sleep),
            Stmt::Select {
                columns,
                table,
                order,
                limit,
            } => {
                let table = self.table_by_name(&table)?;
                self.with_table(&table, |t| {
                    let picks = match &columns {
                        None => (0..t.desc.columns.len()).collect(),
                        Some(names) => names
                            .iter()
                            .map(|n| t.column_index(&ColumnName::new(n.as_str())))
                            .collect::<Result<Vec<_>, _>>()?,
                    };
                    let mut rows = t.rows.clone();
                    if let Some((column, desc)) = &order {
                        let i = t.column_index(&ColumnName::new(column.as_str()))?;
                        rows.sort_by(|a, b| compare::cmp(&a[i], &b[i]));
                        if *desc {
                            rows.reverse();
                        }
                    }
                    rows.truncate(limit.unwrap_or(usize::MAX));
                    let all = t.columns();
                    Ok(Outcome::Rows(Page {
                        columns: picks.iter().map(|&i| all[i].clone()).collect(),
                        rows: rows
                            .into_iter()
                            .map(|r| Row {
                                values: picks.iter().map(|&i| r[i].clone()).collect(),
                            })
                            .collect(),
                    }))
                })
            }
            Stmt::Delete {
                table,
                column,
                value,
            } => {
                let table = self.table_by_name(&table)?;
                self.with_table(&table, |t| {
                    let i = t.column_index(&ColumnName::new(column.as_str()))?;
                    let before = t.rows.len();
                    t.rows.retain(|r| !compare::same(&r[i], &value));
                    Ok(Outcome::Affected((before - t.rows.len()) as u64))
                })
            }
        }
    }
}

fn col(name: &str, type_name: &str, nullable: bool, primary_key: bool) -> ColumnDef {
    ColumnDef {
        name: name.into(),
        type_name: type_name.into(),
        nullable,
        default: None,
        primary_key,
        references: None,
        comment: None,
    }
}

enum Stmt {
    Literal(i64),
    Sleep,
    Select {
        columns: Option<Vec<String>>,
        table: String,
        order: Option<(String, bool)>,
        limit: Option<usize>,
    },
    Delete {
        table: String,
        column: String,
        value: Value,
    },
}

enum Outcome {
    Rows(Page),
    Affected(u64),
    Sleep,
}

/// Parses the tiny SQL subset: `SELECT n`, `SELECT sleep`,
/// `SELECT cols FROM t [ORDER BY c [DESC]] [LIMIT n]`, `DELETE FROM t WHERE c = v`.
fn parse(sql: &str) -> Result<Stmt, Error> {
    let text = sql.trim().trim_end_matches(';').to_lowercase();
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let syntax = || Error::Query {
        message: format!("syntax error at or near \"{}\"", sql.trim()),
        code: Some("42601".into()),
    };
    match tokens.as_slice() {
        ["select", "sleep"] => Ok(Stmt::Sleep),
        ["select", n] => n.parse().map(Stmt::Literal).map_err(|_| syntax()),
        ["delete", "from", table, "where", column, "=", value] => Ok(Stmt::Delete {
            table: (*table).to_owned(),
            column: (*column).to_owned(),
            value: value
                .parse()
                .map(Value::Int)
                .unwrap_or_else(|_| Value::Text(value.trim_matches('\'').to_owned())),
        }),
        ["select", ..] => {
            let from = tokens
                .iter()
                .position(|t| *t == "from")
                .ok_or_else(syntax)?;
            let list = tokens[1..from].join(" ");
            let columns = (list != "*").then(|| {
                list.split(',')
                    .map(|c| c.trim().trim_matches('"').to_owned())
                    .collect()
            });
            let table = tokens.get(from + 1).ok_or_else(syntax)?.to_string();
            let mut rest = &tokens[from + 2..];
            let mut order = None;
            let mut limit = None;
            while !rest.is_empty() {
                match rest {
                    ["order", "by", column, "desc", tail @ ..] => {
                        order = Some(((*column).to_owned(), true));
                        rest = tail;
                    }
                    ["order", "by", column, "asc", tail @ ..]
                    | ["order", "by", column, tail @ ..] => {
                        order = Some(((*column).to_owned(), false));
                        rest = tail;
                    }
                    ["limit", n, tail @ ..] => {
                        limit = Some(n.parse().map_err(|_| syntax())?);
                        rest = tail;
                    }
                    _ => return Err(syntax()),
                }
            }
            Ok(Stmt::Select {
                columns,
                table,
                order,
                limit,
            })
        }
        _ => Err(syntax()),
    }
}

impl Connection for FakeSql {
    fn info(&self) -> &ConnectionInfo {
        &self.info
    }

    fn ping(&self) -> BoxFuture<'_, Result<Duration, Error>> {
        async { Ok(Duration::ZERO) }.boxed()
    }

    fn close(&self) -> BoxFuture<'_, Result<(), Error>> {
        async { Ok(()) }.boxed()
    }

    fn browse(&self) -> Option<&dyn Browse> {
        Some(self)
    }

    fn query(&self) -> Option<&dyn Query> {
        Some(self)
    }

    fn edit_rows(&self) -> Option<&dyn EditRows> {
        Some(self)
    }

    fn edit_schema(&self) -> Option<&dyn EditSchema> {
        Some(self)
    }

    fn describe(&self) -> Option<&dyn Describe> {
        Some(self)
    }

    fn transactions(&self) -> Option<&dyn Transactions> {
        Some(self)
    }

    fn explain(&self) -> Option<&dyn Explain> {
        Some(self)
    }

    fn aggregate(&self) -> Option<&dyn Aggregate> {
        Some(self)
    }

    fn column_stats(&self) -> Option<&dyn ColumnStats> {
        Some(self)
    }
}

impl Browse for FakeSql {
    fn schemas(&self) -> BoxFuture<'_, Result<Vec<SchemaName>, Error>> {
        async { Ok(vec![SchemaName::new(SCHEMA)]) }.boxed()
    }

    fn default_schema(&self) -> BoxFuture<'_, Result<Option<SchemaName>, Error>> {
        async { Ok(Some(SchemaName::new(SCHEMA))) }.boxed()
    }

    fn objects<'a>(
        &'a self,
        schema: &'a SchemaName,
    ) -> BoxFuture<'a, Result<Vec<DbObject>, Error>> {
        let objects = self
            .lock()
            .iter()
            .filter(|(t, _)| t.schema.as_ref() == Some(schema))
            .map(|(t, ft)| DbObject {
                table: t.clone(),
                kind: ObjectKind::Table,
                row_estimate: Some(ft.rows.len() as u64),
            })
            .collect();
        async move { Ok(objects) }.boxed()
    }

    fn rows<'a>(
        &'a self,
        table: &'a TableRef,
        request: &'a RowsRequest,
    ) -> BoxFuture<'a, Result<Page, Error>> {
        let result = self.with_table(table, |t| {
            let mut rows = filtered(t, request)?;
            for sort in request.sort.iter().rev() {
                let i = t.column_index(&sort.column)?;
                rows.sort_by(|a, b| compare::cmp(&a[i], &b[i]));
                if sort.dir == SortDir::Desc {
                    rows.reverse();
                }
            }
            let limit = if request.limit == 0 {
                usize::MAX
            } else {
                request.limit as usize
            };
            Ok(Page {
                columns: t.columns(),
                rows: rows
                    .into_iter()
                    .skip(request.offset as usize)
                    .take(limit)
                    .map(|values| Row { values })
                    .collect(),
            })
        });
        async move { result }.boxed()
    }

    fn count<'a>(
        &'a self,
        table: &'a TableRef,
        request: &'a RowsRequest,
    ) -> BoxFuture<'a, Result<u64, Error>> {
        let result = self.with_table(table, |t| Ok(filtered(t, request)?.len() as u64));
        async move { result }.boxed()
    }
}

fn filtered(t: &FakeTable, request: &RowsRequest) -> Result<Vec<Vec<Value>>, Error> {
    let filters = request
        .filters
        .iter()
        .map(|f| Ok((t.column_index(&f.column)?, f)))
        .collect::<Result<Vec<_>, Error>>()?;
    Ok(t.rows
        .iter()
        .filter(|r| filters.iter().all(|(i, f)| compare::matches(&r[*i], f)))
        .cloned()
        .collect())
}

impl Query for FakeSql {
    fn execute<'a>(
        &'a self,
        sql: &'a str,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<StatementOutput, Error>> {
        async move {
            if cancel.is_cancelled() {
                return Err(Error::Cancelled);
            }
            match self.run(parse(sql)?)? {
                Outcome::Sleep => {
                    cancel.cancelled().await;
                    Err(Error::Cancelled)
                }
                Outcome::Affected(rows) => Ok(StatementOutput::Affected { rows }),
                Outcome::Rows(page) => {
                    let pages: Vec<Result<Vec<Row>, Error>> = page
                        .rows
                        .chunks(PAGE_SIZE)
                        .map(|c| Ok(c.to_vec()))
                        .collect();
                    Ok(StatementOutput::Rows(RowStream {
                        columns: page.columns,
                        pages: Box::pin(stream::iter(pages)),
                    }))
                }
            }
        }
        .boxed()
    }
}

impl Explain for FakeSql {
    fn explain<'a>(&'a self, sql: &'a str) -> BoxFuture<'a, Result<ExplainPlan, Error>> {
        let result = parse(sql).map(|stmt| match stmt {
            Stmt::Select { table, .. } | Stmt::Delete { table, .. } => ExplainPlan {
                text: format!("Seq Scan on {table}"),
            },
            Stmt::Literal(_) | Stmt::Sleep => ExplainPlan {
                text: "Result".into(),
            },
        });
        async move { result }.boxed()
    }
}

impl EditRows for FakeSql {
    fn apply<'a>(&'a self, edits: &'a [RowEdit]) -> BoxFuture<'a, Result<u64, Error>> {
        let result = (|| {
            let mut tables = self.lock();
            let mut staged = tables.clone();
            for edit in edits {
                let t = staged
                    .get_mut(&edit.table)
                    .ok_or_else(|| Error::not_found(edit.table.to_string()))?;
                apply_edit(t, edit)?;
            }
            *tables = staged;
            Ok(edits.len() as u64)
        })();
        async move { result }.boxed()
    }
}

fn apply_edit(t: &mut FakeTable, edit: &RowEdit) -> Result<(), Error> {
    match &edit.kind {
        RowEditKind::Insert { values } => {
            let mut row = vec![Value::Null; t.desc.columns.len()];
            for cv in values {
                row[t.column_index(&cv.column)?] = cv.value.clone();
            }
            let key: Vec<ColumnValue> = t
                .desc
                .columns
                .iter()
                .enumerate()
                .filter(|(_, c)| c.primary_key)
                .map(|(i, c)| ColumnValue::new(c.name.clone(), row[i].clone()))
                .collect();
            if t.find_row(&key)?.is_some() {
                return Err(Error::Query {
                    message: "duplicate key value violates unique constraint".into(),
                    code: Some("23505".into()),
                });
            }
            t.rows.push(row);
        }
        RowEditKind::Update { changes } => {
            let i = t
                .find_row(&edit.primary_key)?
                .ok_or_else(|| Error::not_found("row"))?;
            for cv in changes {
                let c = t.column_index(&cv.column)?;
                t.rows[i][c] = cv.value.clone();
            }
        }
        RowEditKind::Delete => {
            let i = t
                .find_row(&edit.primary_key)?
                .ok_or_else(|| Error::not_found("row"))?;
            t.rows.remove(i);
        }
        _ => return Err(Error::unsupported("row edit kind")),
    }
    Ok(())
}

impl EditSchema for FakeSql {
    fn preview(&self, table: &TableRef, changes: &[SchemaChange]) -> Result<Vec<String>, Error> {
        changes
            .iter()
            .map(|change| {
                Ok(match change {
                    SchemaChange::AddColumn { column } => {
                        format!(
                            "ALTER TABLE {table} ADD COLUMN {} {}",
                            column.name, column.type_name
                        )
                    }
                    SchemaChange::DropColumn { name } => {
                        format!("ALTER TABLE {table} DROP COLUMN {name}")
                    }
                    SchemaChange::RenameColumn { from, to } => {
                        format!("ALTER TABLE {table} RENAME COLUMN {from} TO {to}")
                    }
                    SchemaChange::AddIndex { index } => {
                        format!(
                            "CREATE INDEX {} ON {table} ({})",
                            index.name,
                            index.columns.join(", ")
                        )
                    }
                    SchemaChange::DropIndex { name } => format!("DROP INDEX {name}"),
                    SchemaChange::Truncate => format!("TRUNCATE {table}"),
                    _ => return Err(Error::unsupported("schema change")),
                })
            })
            .collect()
    }

    fn apply_schema<'a>(
        &'a self,
        table: &'a TableRef,
        changes: &'a [SchemaChange],
    ) -> BoxFuture<'a, Result<(), Error>> {
        let result = self.with_table(table, |t| {
            let mut staged = t.clone();
            for change in changes {
                match change {
                    SchemaChange::AddColumn { column } => {
                        let mut column = column.clone();
                        column.primary_key = false;
                        staged.desc.columns.push(column);
                        staged.rows.iter_mut().for_each(|r| r.push(Value::Null));
                    }
                    SchemaChange::DropColumn { name } => {
                        let i = staged.column_index(name)?;
                        staged.desc.columns.remove(i);
                        staged.rows.iter_mut().for_each(|r| {
                            r.remove(i);
                        });
                    }
                    SchemaChange::RenameColumn { from, to } => {
                        let i = staged.column_index(from)?;
                        staged.desc.columns[i].name = to.clone();
                    }
                    SchemaChange::AddIndex { index } => staged.desc.indexes.push(index.clone()),
                    SchemaChange::DropIndex { name } => {
                        staged.desc.indexes.retain(|i| &i.name != name);
                    }
                    SchemaChange::Truncate => staged.rows.clear(),
                    _ => return Err(Error::unsupported("schema change")),
                }
            }
            *t = staged;
            Ok(())
        });
        async move { result }.boxed()
    }
}

impl Describe for FakeSql {
    fn describe<'a>(
        &'a self,
        table: &'a TableRef,
    ) -> BoxFuture<'a, Result<TableDescription, Error>> {
        let result = self.with_table(table, |t| {
            let mut desc = t.desc.clone();
            desc.row_estimate = Some(t.rows.len() as u64);
            Ok(desc)
        });
        async move { result }.boxed()
    }

    fn ddl<'a>(&'a self, table: &'a TableRef) -> BoxFuture<'a, Result<String, Error>> {
        let result = self.with_table(table, |t| {
            let columns: Vec<String> = t
                .desc
                .columns
                .iter()
                .map(|c| {
                    let mut line = format!("  {} {}", c.name, c.type_name);
                    if !c.nullable {
                        line.push_str(" NOT NULL");
                    }
                    if c.primary_key {
                        line.push_str(" PRIMARY KEY");
                    }
                    line
                })
                .collect();
            Ok(format!(
                "CREATE TABLE {table} (\n{}\n);",
                columns.join(",\n")
            ))
        });
        async move { result }.boxed()
    }
}

impl Transactions for FakeSql {
    fn atomic<'a>(&'a self, ops: &'a [TxOp]) -> BoxFuture<'a, Result<(), Error>> {
        let result = (|| {
            let stmts = ops
                .iter()
                .map(|op| match op {
                    TxOp::Sql { sql } => parse(sql),
                    _ => Err(Error::unsupported("key-value op in SQL transaction")),
                })
                .collect::<Result<Vec<_>, _>>()?;
            let snapshot = self.lock().clone();
            for stmt in stmts {
                if let Err(e) = self.run(stmt) {
                    *self.lock() = snapshot;
                    return Err(e);
                }
            }
            Ok(())
        })();
        async move { result }.boxed()
    }
}

impl Aggregate for FakeSql {
    fn aggregate<'a>(
        &'a self,
        request: &'a AggregateRequest,
        _cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<ChartData, Error>> {
        let result = (|| {
            let ChartSource::Table(table) = &request.source else {
                return Err(Error::unsupported(
                    "aggregate over a query in the fake driver",
                ));
            };
            self.with_table(table, |t| {
                let x = t.column_index(&request.x)?;
                let y = request.y.as_ref().map(|c| t.column_index(c)).transpose()?;
                let split = request
                    .split_by
                    .as_ref()
                    .map(|c| t.column_index(c))
                    .transpose()?;
                let mut groups: BTreeMap<String, BTreeMap<String, (Value, Vec<f64>)>> =
                    BTreeMap::new();
                for row in &t.rows {
                    let series = split.map_or_else(
                        || {
                            request
                                .y
                                .as_ref()
                                .map_or("count".into(), ToString::to_string)
                        },
                        |s| row[s].to_string(),
                    );
                    let entry = groups
                        .entry(series)
                        .or_default()
                        .entry(row[x].to_string())
                        .or_insert_with(|| (row[x].clone(), Vec::new()));
                    match y.map(|i| &row[i]) {
                        Some(v) => entry.1.extend(v.as_f64()),
                        None => entry.1.push(1.0),
                    }
                }
                let mut truncated = false;
                let series = groups
                    .into_iter()
                    .map(|(name, points)| {
                        let mut points: Vec<ChartPoint> = points
                            .into_values()
                            .map(|(x, ys)| ChartPoint {
                                x,
                                y: reduce(request.agg, &ys),
                            })
                            .collect();
                        points.sort_by(|a, b| compare::cmp(&a.x, &b.x));
                        truncated |= points.len() > request.limit as usize;
                        points.truncate(request.limit as usize);
                        Series { name, points }
                    })
                    .collect();
                Ok(ChartData { series, truncated })
            })
        })();
        async move { result }.boxed()
    }
}

fn reduce(agg: AggFn, ys: &[f64]) -> f64 {
    match agg {
        AggFn::Sum => ys.iter().sum(),
        AggFn::Avg if !ys.is_empty() => ys.iter().sum::<f64>() / ys.len() as f64,
        AggFn::Min => ys.iter().copied().fold(f64::INFINITY, f64::min),
        AggFn::Max => ys.iter().copied().fold(f64::NEG_INFINITY, f64::max),
        AggFn::None => ys.first().copied().unwrap_or_default(),
        _ => ys.len() as f64,
    }
}

impl ColumnStats for FakeSql {
    fn column_stats<'a>(
        &'a self,
        table: &'a TableRef,
        column: &'a ColumnName,
        top: u32,
    ) -> BoxFuture<'a, Result<ColumnStatsReport, Error>> {
        let result = self.with_table(table, |t| {
            let i = t.column_index(column)?;
            let mut counts: BTreeMap<String, ValueCount> = BTreeMap::new();
            let mut nulls = 0;
            for row in &t.rows {
                if row[i].is_null() {
                    nulls += 1;
                    continue;
                }
                counts
                    .entry(row[i].to_string())
                    .or_insert_with(|| ValueCount {
                        value: row[i].clone(),
                        count: 0,
                    })
                    .count += 1;
            }
            let distinct = counts.len() as u64;
            let mut top_values: Vec<ValueCount> = counts.into_values().collect();
            top_values.sort_by_key(|v| std::cmp::Reverse(v.count));
            top_values.truncate(top as usize);
            Ok(ColumnStatsReport {
                rows: t.rows.len() as u64,
                nulls,
                distinct,
                top: top_values,
            })
        });
        async move { result }.boxed()
    }
}

/// Factory for [`FakeSql`]. Every `connect` returns a fresh, seeded database.
pub struct FakeSqlFactory {
    info: DriverInfo,
}

impl Default for FakeSqlFactory {
    fn default() -> Self {
        Self {
            info: DriverInfo {
                id: "fake-sql".into(),
                name: "Fake SQL".into(),
                family: Family::Sql,
                fields: vec![FieldSpec::new("database", "Database", FieldKind::Text)],
                url_schemes: vec!["fake-sql".into()],
                default_port: None,
            },
        }
    }
}

impl DriverFactory for FakeSqlFactory {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect<'a>(
        &'a self,
        _config: &'a ConnConfig,
        _deps: &'a ConnectDeps,
    ) -> BoxFuture<'a, Result<Arc<dyn Connection>, Error>> {
        async { Ok(Arc::new(FakeSql::new()) as Arc<dyn Connection>) }.boxed()
    }
}
