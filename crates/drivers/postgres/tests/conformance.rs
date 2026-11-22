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

async fn rows(conn: &dyn Connection, sql: &str) -> (Vec<types::Column>, usize) {
    use futures::StreamExt;
    let out = conn
        .query()
        .expect("query")
        .execute(sql, CancellationToken::new())
        .await
        .expect(sql);
    let StatementOutput::Rows(mut stream) = out else {
        panic!("{sql}: expected rows");
    };
    let mut n = 0;
    while let Some(page) = stream.pages.next().await {
        n += page.expect("page").len();
    }
    (stream.columns, n)
}

/// Driver-specific: cancel stops the backend (found by its SQL tag), an empty result still has
/// columns, and custom types are named.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn postgres_query_edges() {
    let Some(conn) = connect().await else { return };
    let conn = &*conn;

    let token = CancellationToken::new();
    let trigger = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        trigger.cancel();
    });
    let query = conn.query().expect("query");
    let result = match query.execute("SELECT pg_sleep(9.123)", token).await {
        Ok(StatementOutput::Rows(mut s)) => {
            use futures::StreamExt;
            s.pages.next().await.transpose().map(|_| ())
        }
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    };
    assert_eq!(result, Err(types::Error::Cancelled));
    let (_, running) = rows(
        conn,
        "SELECT pid FROM pg_stat_activity WHERE query LIKE '%pg_sleep(9.123)%' \
         AND state = 'active' AND pid <> pg_backend_pid()",
    )
    .await;
    assert_eq!(running, 0, "the server stopped the cancelled query");

    let (columns, n) = rows(conn, "SELECT 1::int4 AS a, 'x'::text AS b WHERE false").await;
    assert_eq!(n, 0);
    let names: Vec<_> = columns
        .iter()
        .map(|c| (c.name.as_str(), c.type_name.as_str()))
        .collect();
    assert_eq!(names, [("a", "int4"), ("b", "text")]);

    run(conn, "DROP TYPE IF EXISTS rift_mood").await;
    run(conn, "CREATE TYPE rift_mood AS ENUM ('ok')").await;
    let (columns, n) = rows(conn, "SELECT 'ok'::rift_mood AS m").await;
    assert_eq!(n, 1);
    assert_eq!(columns[0].type_name, "rift_mood");
    run(conn, "DROP TYPE rift_mood").await;
}

/// A host that never answers fails within the pool's 10 s acquire timeout, not the OS's TCP
/// timeout. Ignored because it takes 10 s.
#[tokio::test]
#[ignore = "slow: waits out the connect timeout"]
async fn connect_gives_up_on_unreachable_host() {
    // 10.255.255.1 is unroutable on most networks: the SYN is never answered.
    let config = ConnConfig::new("postgres").with("host", "10.255.255.1");
    let deps = ConnectDeps {
        spawner: Arc::new(TokioSpawner),
    };
    let started = std::time::Instant::now();
    let result = postgres_driver::factory().connect(&config, &deps).await;
    assert!(result.is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(15));
}
