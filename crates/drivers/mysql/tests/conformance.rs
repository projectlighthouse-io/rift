//! Runs the SQL conformance suites against Docker MySQL (`make dev-up`).
//! Skipped when the database is unreachable, unless `RIFT_REQUIRE_DOCKER=1`.

use std::sync::Arc;

use futures::future::BoxFuture;
use testkit::suites::{self, SqlFixture};
use tokio_util::sync::CancellationToken;
use traits::{ConnectDeps, Connection, Spawner, StatementOutput};
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

async fn connect() -> Option<Arc<dyn Connection>> {
    let port = std::env::var("RIFT_MYSQL_PORT").unwrap_or_else(|_| "3306".into());
    let mut config = ConnConfig::new("mysql")
        .with("host", "127.0.0.1")
        .with("port", port)
        .with("user", "root")
        .with("database", "mysql");
    config.password = Some(Secret::new("mysql"));
    let deps = ConnectDeps {
        spawner: Arc::new(TokioSpawner),
    };
    match mysql_driver::factory().connect(&config, &deps).await {
        Ok(conn) => Some(conn),
        Err(e) if std::env::var("RIFT_REQUIRE_DOCKER").is_err() => {
            eprintln!("skipping: MySQL unreachable ({e})");
            None
        }
        Err(e) => panic!("MySQL unreachable: {e}"),
    }
}

async fn run(conn: &dyn Connection, sql: &str) {
    let output = conn
        .query()
        .expect("query")
        .execute(sql, CancellationToken::new())
        .await;
    assert!(
        matches!(output, Ok(StatementOutput::Affected { .. })),
        "{sql}: {:?}",
        output.err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn mysql_passes_sql_suites() {
    let Some(conn) = connect().await else { return };
    run(&*conn, "DROP DATABASE IF EXISTS rift_test").await;
    run(&*conn, "CREATE DATABASE rift_test").await;
    run(
        &*conn,
        "CREATE TABLE rift_test.rift_conformance \
         (id BIGINT PRIMARY KEY, name VARCHAR(64) NOT NULL, score DOUBLE)",
    )
    .await;
    run(
        &*conn,
        "INSERT INTO rift_test.rift_conformance VALUES (1, 'alpha', 1.5), (2, 'beta', NULL), \
         (3, 'gamma', 3.0)",
    )
    .await;
    let fixture = SqlFixture {
        table: TableRef::new(Some("rift_test".into()), "rift_conformance"),
        select_all_sql: "SELECT id, name, score FROM rift_test.rift_conformance ORDER BY id".into(),
        delete_first_sql: "DELETE FROM rift_test.rift_conformance WHERE id = 1".into(),
        slow_sql: "SELECT SLEEP(10)".into(),
    };
    suites::run_sql_suites(&*conn, &fixture).await;
    run(&*conn, "DROP DATABASE rift_test").await;
}
