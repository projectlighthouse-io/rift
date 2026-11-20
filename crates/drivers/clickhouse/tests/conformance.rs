//! Runs the SQL conformance suites against Docker ClickHouse (`make dev-up`).
//! Skipped when ClickHouse is unreachable, unless `RIFT_REQUIRE_DOCKER=1`.

use std::sync::Arc;

use futures::future::BoxFuture;
use testkit::suites::{self, SqlFixture};
use tokio_util::sync::CancellationToken;
use traits::{ConnectDeps, Connection, Spawner};
use types::{ConnConfig, Secret, TableRef};

struct TokioSpawner;

impl Spawner for TokioSpawner {
    fn spawn(&self, task: BoxFuture<'static, ()>) {
        tokio::spawn(task);
    }

    fn spawn_blocking(&self, task: Box<dyn FnOnce() + Send + 'static>) {
        tokio::task::spawn_blocking(task);
    }
}

async fn run(conn: &dyn Connection, sql: &str) {
    let result = conn
        .query()
        .expect("query")
        .execute(sql, CancellationToken::new())
        .await;
    assert!(result.is_ok(), "{sql}: {:?}", result.err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn clickhouse_passes_sql_suites() {
    let port = std::env::var("RIFT_CLICKHOUSE_HTTP_PORT").unwrap_or_else(|_| "8123".into());
    let mut config = ConnConfig::new("clickhouse")
        .with("host", "127.0.0.1")
        .with("port", port)
        .with("user", "default");
    config.password = Some(Secret::new("clickhouse"));
    let deps = ConnectDeps {
        spawner: Arc::new(TokioSpawner),
    };
    let conn = match clickhouse_driver::factory().connect(&config, &deps).await {
        Ok(c) => c,
        Err(e) if std::env::var("RIFT_REQUIRE_DOCKER").is_err() => {
            eprintln!("skipping: ClickHouse unreachable ({e})");
            return;
        }
        Err(e) => panic!("ClickHouse unreachable: {e}"),
    };
    run(&*conn, "DROP DATABASE IF EXISTS rift_test").await;
    run(&*conn, "CREATE DATABASE rift_test").await;
    run(
        &*conn,
        "CREATE TABLE rift_test.rift_conformance (id UInt64, name String, score Nullable(Float64)) \
         ENGINE = MergeTree ORDER BY id",
    )
    .await;
    run(&*conn, "INSERT INTO rift_test.rift_conformance VALUES (1, 'alpha', 1.5), (2, 'beta', NULL), (3, 'gamma', 3.0)").await;
    let fixture = SqlFixture {
        table: TableRef::new(Some("rift_test".into()), "rift_conformance"),
        select_all_sql: "SELECT id, name, score FROM rift_test.rift_conformance ORDER BY id".into(),
        delete_first_sql: "ALTER TABLE rift_test.rift_conformance DELETE WHERE id = 1".into(),
        slow_sql: "SELECT count() FROM system.numbers WHERE number % 1000003 = 7".into(),
    };
    suites::run_sql_suites(&*conn, &fixture).await;

    run(
        &*conn,
        "CREATE TABLE rift_test.events (ts DateTime, n UInt8) ENGINE = MergeTree ORDER BY ts",
    )
    .await;
    suites::table_ttl_suite(&*conn, &TableRef::new(Some("rift_test".into()), "events")).await;
    run(&*conn, "DROP DATABASE rift_test").await;
}
