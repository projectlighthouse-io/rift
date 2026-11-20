//! Runs the SQL conformance suites against a temporary SQLite file.

use std::sync::Arc;

use futures::future::BoxFuture;
use testkit::suites::{self, SqlFixture};
use traits::{ConnectDeps, Spawner};
use types::{ConnConfig, TableRef};

struct TokioSpawner;

impl Spawner for TokioSpawner {
    fn spawn(&self, task: BoxFuture<'static, ()>) {
        tokio::spawn(task);
    }

    fn spawn_blocking(&self, task: Box<dyn FnOnce() + Send + 'static>) {
        tokio::task::spawn_blocking(task);
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn sqlite_passes_sql_suites() {
    let dir = std::env::temp_dir().join(format!("rift-sqlite-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let path = dir.join("conformance.sqlite");
    let _ = std::fs::remove_file(&path);
    {
        let conn = rusqlite::Connection::open(&path).expect("create");
        conn.execute_batch(
            "CREATE TABLE rift_conformance (id INTEGER PRIMARY KEY, name TEXT NOT NULL, score REAL);
             INSERT INTO rift_conformance VALUES (1, 'alpha', 1.5), (2, 'beta', NULL), (3, 'gamma', 3.0);",
        )
        .expect("seed");
    }
    let config = ConnConfig::new("sqlite").with("path", path.to_string_lossy());
    let deps = ConnectDeps {
        spawner: Arc::new(TokioSpawner),
    };
    let conn = sqlite_driver::factory()
        .connect(&config, &deps)
        .await
        .expect("connect");
    let fixture = SqlFixture {
        table: TableRef::new(Some("main".into()), "rift_conformance"),
        select_all_sql: "SELECT id, name, score FROM rift_conformance ORDER BY id".into(),
        delete_first_sql: "DELETE FROM rift_conformance WHERE id = 1".into(),
        slow_sql: "WITH RECURSIVE c(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM c) \
                   SELECT count(*) FROM c"
            .into(),
    };
    suites::run_sql_suites(&*conn, &fixture).await;
    std::fs::remove_dir_all(dir).expect("cleanup");
}
