//! Runs the SQL conformance suites against Docker Postgres (`make dev-up`).
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
    let port = std::env::var("RIFT_PG_PORT").unwrap_or_else(|_| "5432".into());
    let mut config = ConnConfig::new("postgres")
        .with("host", "127.0.0.1")
        .with("port", port)
        .with("user", "postgres")
        .with("database", "shop_dev");
    config.password = Some(Secret::new("postgres"));
    let deps = ConnectDeps {
        spawner: Arc::new(TokioSpawner),
    };
    match postgres_driver::factory().connect(&config, &deps).await {
        Ok(conn) => Some(conn),
        Err(e) if std::env::var("RIFT_REQUIRE_DOCKER").is_err() => {
            eprintln!("skipping: Postgres unreachable ({e})");
            None
        }
        Err(e) => panic!("Postgres unreachable: {e}"),
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
async fn postgres_passes_sql_suites() {
    let Some(conn) = connect().await else { return };
    run(&*conn, "DROP SCHEMA IF EXISTS rift_test CASCADE").await;
    run(&*conn, "CREATE SCHEMA rift_test").await;
    run(
        &*conn,
        "CREATE TABLE rift_test.rift_conformance \
         (id int8 PRIMARY KEY, name text NOT NULL, score float8)",
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
        slow_sql: "SELECT pg_sleep(10)".into(),
    };
    suites::run_sql_suites(&*conn, &fixture).await;
    run(&*conn, "DROP SCHEMA rift_test CASCADE").await;
}
