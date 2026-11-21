//! Conformance suites for SQL capabilities.

use std::thread;
use std::time::{Duration, Instant};

use futures::StreamExt;
use tokio_util::sync::CancellationToken;
use traits::{Connection, StatementOutput};
use types::{
    AggFn, AggregateRequest, ChartSource, ColumnDef, ColumnName, ColumnValue, Error, Filter,
    FilterOp, ObjectKind, Page, Row, RowEdit, RowEditKind, RowsRequest, SchemaChange, Sort,
    SortDir, TableRef, TxOp, Value,
};

/// What the SQL suites expect to find. The driver's test harness creates `table` before running
/// the suites, with columns `id` (integer primary key), `name` (text, not null) and `score`
/// (float, nullable), holding exactly `(1,'alpha',1.5)`, `(2,'beta',NULL)`, `(3,'gamma',3.0)`.
#[derive(Debug, Clone)]
pub struct SqlFixture {
    /// The fixture table.
    pub table: TableRef,
    /// `SELECT id, name, score FROM <table> ORDER BY id` in the driver's dialect.
    pub select_all_sql: String,
    /// `DELETE FROM <table> WHERE id = 1` in the driver's dialect.
    pub delete_first_sql: String,
    /// A statement that runs for at least five seconds unless cancelled.
    pub slow_sql: String,
}

fn by_id() -> RowsRequest {
    RowsRequest {
        sort: vec![Sort {
            column: "id".into(),
            dir: SortDir::Asc,
        }],
        limit: 100,
        ..RowsRequest::default()
    }
}

fn ids(rows: &[Row]) -> Vec<i64> {
    rows.iter()
        .map(|r| r.values[0].as_f64().expect("id is numeric") as i64)
        .collect()
}

async fn read_all(conn: &dyn Connection, table: &TableRef) -> Page {
    let browse = conn.browse().expect("suite needs browse");
    browse
        .rows(table, &by_id())
        .await
        .expect("read fixture rows")
}

/// Runs every SQL suite whose capability the connection reports. Leaves the fixture as found.
pub async fn run_sql_suites(conn: &dyn Connection, fx: &SqlFixture) {
    if conn.browse().is_some() {
        browse_suite(conn, fx).await;
    }
    if conn.describe().is_some() {
        describe_suite(conn, fx).await;
    }
    if conn.query().is_some() {
        query_suite(conn, fx).await;
    }
    if conn.explain().is_some() {
        explain_suite(conn, fx).await;
    }
    if conn.aggregate().is_some() {
        aggregate_suite(conn, fx).await;
    }
    if conn.column_stats().is_some() {
        column_stats_suite(conn, fx).await;
    }
    if conn.edit_rows().is_some() {
        edit_rows_suite(conn, fx).await;
    }
    if conn.edit_schema().is_some() {
        edit_schema_suite(conn, fx).await;
    }
    if conn.transactions().is_some() {
        sql_transactions_suite(conn, fx).await;
    }
    if conn.backup().is_some() {
        backup_suite(conn, fx).await;
    }
}

/// Runs one statement and returns its rows (empty for statements without a result).
async fn rows_of(conn: &dyn Connection, sql: &str) -> Vec<types::Row> {
    match conn
        .query()
        .expect("query")
        .execute(sql, CancellationToken::new())
        .await
    {
        Ok(StatementOutput::Rows(mut stream)) => {
            let mut rows = Vec::new();
            while let Some(page) = stream.pages.next().await {
                rows.extend(page.expect("page"));
            }
            rows
        }
        Ok(_) => Vec::new(),
        Err(e) => panic!("{sql}: {e}"),
    }
}

/// [`traits::Backup`]: a deleted row comes back after restoring a backup taken before.
/// Skipped when the dump tool is missing, unless `RIFT_REQUIRE_TOOLS=1`.
pub async fn backup_suite(conn: &dyn Connection, fx: &SqlFixture) {
    let backup = conn.backup().expect("backup");
    let probe = match &fx.table.schema {
        Some(schema) => format!("{schema}.rift_backup_probe"),
        None => "rift_backup_probe".to_owned(),
    };
    rows_of(conn, &format!("DROP TABLE IF EXISTS {probe}")).await;
    rows_of(conn, &format!("CREATE TABLE {probe} (id integer)")).await;
    rows_of(conn, &format!("INSERT INTO {probe} VALUES (7)")).await;
    let path = std::env::temp_dir().join(format!("rift-backup-{}.sql", std::process::id()));
    let report = match backup.backup(&path, CancellationToken::new()).await {
        Err(types::Error::MissingDependency { message })
            if std::env::var("RIFT_REQUIRE_TOOLS").is_err() =>
        {
            eprintln!("skipping backup suite: {message}");
            rows_of(conn, &format!("DROP TABLE {probe}")).await;
            return;
        }
        other => other.expect("backup"),
    };
    assert!(report.bytes > 0, "backup wrote nothing");
    rows_of(conn, &format!("DELETE FROM {probe}")).await;
    backup
        .restore(&path, CancellationToken::new())
        .await
        .expect("restore");
    let rows = rows_of(conn, &format!("SELECT id FROM {probe}")).await;
    assert_eq!(rows.len(), 1, "restored row");
    rows_of(conn, &format!("DROP TABLE {probe}")).await;
    let _ = std::fs::remove_file(&path);
}

/// [`traits::Browse`]: schemas, objects, paging, filters, sorting and counts.
pub async fn browse_suite(conn: &dyn Connection, fx: &SqlFixture) {
    let browse = conn.browse().expect("browse");
    let schemas = browse.schemas().await.expect("schemas");
    if let Some(schema) = &fx.table.schema {
        assert!(
            schemas.contains(schema),
            "schemas {schemas:?} missing {schema}"
        );
        let objects = browse.objects(schema).await.expect("objects");
        let found = objects
            .iter()
            .find(|o| o.table == fx.table)
            .expect("fixture table listed");
        assert_eq!(found.kind, ObjectKind::Table);
    }

    let page = browse.rows(&fx.table, &by_id()).await.expect("rows");
    let names: Vec<&str> = page.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "name", "score"]);
    assert!(page.columns[0].primary_key, "id is the primary key");
    assert_eq!(ids(&page.rows), [1, 2, 3]);
    assert_eq!(page.rows[0].values[1], Value::Text("alpha".into()));
    assert!(page.rows[1].values[2].is_null(), "beta has NULL score");

    let paged = RowsRequest {
        offset: 1,
        limit: 1,
        ..by_id()
    };
    assert_eq!(
        ids(&browse.rows(&fx.table, &paged).await.expect("page").rows),
        [2]
    );

    let desc = RowsRequest {
        sort: vec![Sort {
            column: "id".into(),
            dir: SortDir::Desc,
        }],
        ..by_id()
    };
    assert_eq!(
        ids(&browse.rows(&fx.table, &desc).await.expect("desc").rows),
        [3, 2, 1]
    );

    let cases = [
        (FilterOp::Eq, Value::Text("beta".into()), "name", vec![2]),
        (
            FilterOp::NotEq,
            Value::Text("beta".into()),
            "name",
            vec![1, 3],
        ),
        (
            FilterOp::Contains,
            Value::Text("AMM".into()),
            "name",
            vec![3],
        ),
        (FilterOp::Gt, Value::Int(1), "id", vec![2, 3]),
        (FilterOp::IsNull, Value::Null, "score", vec![2]),
        (FilterOp::IsNotNull, Value::Null, "score", vec![1, 3]),
    ];
    for (op, value, column, expected) in cases {
        let request = RowsRequest {
            filters: vec![Filter {
                column: column.into(),
                op,
                value,
            }],
            ..by_id()
        };
        let rows = browse
            .rows(&fx.table, &request)
            .await
            .expect("filtered rows")
            .rows;
        assert_eq!(ids(&rows), expected, "filter {column} {op:?}");
        let count = browse.count(&fx.table, &request).await.expect("count");
        assert_eq!(count, expected.len() as u64, "count {column} {op:?}");
    }
}

/// [`traits::Describe`]: columns, keys and DDL.
pub async fn describe_suite(conn: &dyn Connection, fx: &SqlFixture) {
    let describe = conn.describe().expect("describe");
    let desc = describe.describe(&fx.table).await.expect("describe");
    let names: Vec<&str> = desc.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["id", "name", "score"]);
    assert_eq!(desc.primary_key(), [&ColumnName::new("id")]);
    assert!(!desc.columns[1].nullable, "name is NOT NULL");
    assert!(desc.columns[2].nullable, "score is nullable");
    let ddl = describe.ddl(&fx.table).await.expect("ddl");
    assert!(
        ddl.contains(fx.table.name.as_str()),
        "ddl mentions the table: {ddl}"
    );
}

async fn collect(output: StatementOutput) -> Result<Vec<Row>, Error> {
    match output {
        StatementOutput::Rows(mut stream) => {
            let mut rows = Vec::new();
            while let Some(page) = stream.pages.next().await {
                rows.extend(page?);
            }
            Ok(rows)
        }
        _ => Ok(Vec::new()),
    }
}

/// [`traits::Query`]: results, errors and cancellation.
pub async fn query_suite(conn: &dyn Connection, fx: &SqlFixture) {
    let query = conn.query().expect("query");
    let output = query
        .execute(&fx.select_all_sql, CancellationToken::new())
        .await
        .expect("run");
    let StatementOutput::Rows(stream) = output else {
        panic!("SELECT must return rows")
    };
    assert_eq!(stream.columns.len(), 3);
    let rows = collect(StatementOutput::Rows(stream)).await.expect("rows");
    assert_eq!(ids(&rows), [1, 2, 3]);

    let bad = query
        .execute("SELEKT nonsense", CancellationToken::new())
        .await;
    let bad = match bad {
        Ok(output) => collect(output).await.map(|_| ()),
        Err(e) => Err(e),
    };
    assert!(
        matches!(bad, Err(Error::Query { .. })),
        "bad SQL is a query error: {bad:?}"
    );

    let token = CancellationToken::new();
    let trigger = token.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(300));
        trigger.cancel();
    });
    let started = Instant::now();
    let result = match query.execute(&fx.slow_sql, token).await {
        Ok(output) => collect(output).await.map(|_| ()),
        Err(e) => Err(e),
    };
    assert_eq!(result, Err(Error::Cancelled), "slow query is cancelled");
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "cancel is prompt"
    );
}

/// [`traits::Explain`]: returns a non-empty plan.
pub async fn explain_suite(conn: &dyn Connection, fx: &SqlFixture) {
    let plan = conn
        .explain()
        .expect("explain")
        .explain(&fx.select_all_sql)
        .await
        .expect("plan");
    assert!(!plan.text.trim().is_empty());
}

/// [`traits::Aggregate`]: count and sum grouped by a column.
pub async fn aggregate_suite(conn: &dyn Connection, fx: &SqlFixture) {
    let aggregate = conn.aggregate().expect("aggregate");
    let mut request = AggregateRequest {
        source: ChartSource::Table(fx.table.clone()),
        x: "name".into(),
        bucket: None,
        y: None,
        agg: AggFn::Count,
        split_by: None,
        limit: 100,
    };
    let data = aggregate
        .aggregate(&request, CancellationToken::new())
        .await
        .expect("count");
    assert_eq!(data.series.len(), 1);
    let points = &data.series[0].points;
    assert_eq!(points.len(), 3);
    assert!(points.iter().all(|p| (p.y - 1.0).abs() < 1e-9));

    request.agg = AggFn::Sum;
    request.y = Some("score".into());
    let data = aggregate
        .aggregate(&request, CancellationToken::new())
        .await
        .expect("sum");
    let alpha = data.series[0]
        .points
        .iter()
        .find(|p| p.x == Value::Text("alpha".into()))
        .expect("alpha point");
    assert!((alpha.y - 1.5).abs() < 1e-9);

    request.limit = 2;
    let data = aggregate
        .aggregate(&request, CancellationToken::new())
        .await
        .expect("capped");
    assert!(data.series[0].points.len() <= 2);
    assert!(data.truncated, "cap is reported");
}

/// [`traits::ColumnStats`]: rows, nulls, distinct and top values.
pub async fn column_stats_suite(conn: &dyn Connection, fx: &SqlFixture) {
    let stats = conn.column_stats().expect("column_stats");
    let name = stats
        .column_stats(&fx.table, &"name".into(), 10)
        .await
        .expect("name stats");
    assert_eq!((name.rows, name.nulls, name.distinct), (3, 0, 3));
    assert_eq!(name.top.len(), 3);
    let score = stats
        .column_stats(&fx.table, &"score".into(), 1)
        .await
        .expect("score stats");
    assert_eq!((score.rows, score.nulls, score.distinct), (3, 1, 2));
    assert_eq!(score.top.len(), 1);
}

fn key(id: i64) -> Vec<ColumnValue> {
    vec![ColumnValue::new("id", Value::Int(id))]
}

/// [`traits::EditRows`]: insert, update, delete, and all-or-nothing batches.
pub async fn edit_rows_suite(conn: &dyn Connection, fx: &SqlFixture) {
    let edit = conn.edit_rows().expect("edit_rows");
    let insert = |id: i64, name: &str| RowEdit {
        table: fx.table.clone(),
        primary_key: Vec::new(),
        kind: RowEditKind::Insert {
            values: vec![
                ColumnValue::new("id", Value::Int(id)),
                ColumnValue::new("name", Value::Text(name.into())),
            ],
        },
    };

    edit.apply(&[insert(4, "delta")]).await.expect("insert");
    edit.apply(&[RowEdit {
        table: fx.table.clone(),
        primary_key: key(4),
        kind: RowEditKind::Update {
            changes: vec![
                ColumnValue::new("name", Value::Text("delta2".into())),
                ColumnValue::new("score", Value::Float(4.5)),
            ],
        },
    }])
    .await
    .expect("update");
    let page = read_all(conn, &fx.table).await;
    assert_eq!(ids(&page.rows), [1, 2, 3, 4]);
    assert_eq!(page.rows[3].values[1], Value::Text("delta2".into()));

    let failed = edit
        .apply(&[insert(5, "epsilon"), insert(1, "duplicate")])
        .await;
    assert!(failed.is_err(), "duplicate key fails the batch");
    assert_eq!(
        ids(&read_all(conn, &fx.table).await.rows),
        [1, 2, 3, 4],
        "batch rolled back"
    );

    edit.apply(&[RowEdit {
        table: fx.table.clone(),
        primary_key: key(4),
        kind: RowEditKind::Delete,
    }])
    .await
    .expect("delete");
    assert_eq!(ids(&read_all(conn, &fx.table).await.rows), [1, 2, 3]);
}

/// [`traits::EditSchema`]: preview, then add and drop a column.
pub async fn edit_schema_suite(conn: &dyn Connection, fx: &SqlFixture) {
    let schema = conn.edit_schema().expect("edit_schema");
    let column = ColumnDef {
        name: "extra".into(),
        type_name: "text".into(),
        nullable: true,
        default: None,
        primary_key: false,
        references: None,
        comment: None,
    };
    let add = [SchemaChange::AddColumn { column }];
    let sql = schema.preview(&fx.table, &add).expect("preview");
    assert!(
        sql.iter().any(|s| s.contains("extra")),
        "preview names the column: {sql:?}"
    );
    schema
        .apply_schema(&fx.table, &add)
        .await
        .expect("add column");

    let names = |page: &Page| {
        page.columns
            .iter()
            .map(|c| c.name.to_string())
            .collect::<Vec<_>>()
    };
    assert!(names(&read_all(conn, &fx.table).await).contains(&"extra".to_owned()));

    schema
        .apply_schema(
            &fx.table,
            &[SchemaChange::DropColumn {
                name: "extra".into(),
            }],
        )
        .await
        .expect("drop column");
    assert_eq!(
        names(&read_all(conn, &fx.table).await),
        ["id", "name", "score"]
    );
}

/// [`traits::Transactions`] for SQL: a failing batch changes nothing.
pub async fn sql_transactions_suite(conn: &dyn Connection, fx: &SqlFixture) {
    let tx = conn.transactions().expect("transactions");
    tx.atomic(&[]).await.expect("empty batch");
    let failed = tx
        .atomic(&[
            TxOp::Sql {
                sql: fx.delete_first_sql.clone(),
            },
            TxOp::Sql {
                sql: "SELEKT nonsense".into(),
            },
        ])
        .await;
    assert!(failed.is_err(), "bad statement fails the batch");
    assert_eq!(
        ids(&read_all(conn, &fx.table).await.rows),
        [1, 2, 3],
        "delete rolled back"
    );
}

/// [`traits::TableTtl`]: set, read back and remove a table TTL. `table` needs a `DateTime`
/// column named `ts`.
pub async fn table_ttl_suite(conn: &dyn Connection, table: &TableRef) {
    let ttl = conn.table_ttl().expect("table_ttl");
    let rule = types::TtlRule {
        column: None,
        expression: "ts + INTERVAL 30 DAY".into(),
    };
    ttl.set_table_ttl(table, Some(&rule))
        .await
        .expect("set ttl");
    let rules = ttl.ttl_rules(table).await.expect("read ttl");
    assert!(
        rules
            .iter()
            .any(|r| r.column.is_none() && r.expression.contains("ts")),
        "rules: {rules:?}"
    );
    ttl.set_table_ttl(table, None).await.expect("remove ttl");
    assert!(
        ttl.ttl_rules(table).await.expect("read ttl").is_empty(),
        "TTL removed"
    );
}
