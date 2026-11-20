//! Runs the key-value conformance suites against Docker Redis (`make dev-up`).
//! Skipped when Redis is unreachable, unless `RIFT_REQUIRE_DOCKER=1`.

use std::sync::Arc;

use futures::future::BoxFuture;
use testkit::suites::{self, KvFixture};
use traits::{ConnectDeps, Spawner};
use types::{ConnConfig, KeyKind};

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
async fn redis_passes_kv_suites() {
    let port = std::env::var("RIFT_REDIS_PORT").unwrap_or_else(|_| "6379".into());
    // db 15 keeps the seed data in db 0 untouched.
    let config = ConnConfig::new("redis")
        .with("host", "127.0.0.1")
        .with("port", port)
        .with("database", "15");
    let deps = ConnectDeps {
        spawner: Arc::new(TokioSpawner),
    };
    let conn = match redis_driver::factory().connect(&config, &deps).await {
        Ok(c) => c,
        Err(e) if std::env::var("RIFT_REQUIRE_DOCKER").is_err() => {
            eprintln!("skipping: Redis unreachable ({e})");
            return;
        }
        Err(e) => panic!("Redis unreachable: {e}"),
    };
    let fixture = KvFixture {
        prefix: "rift:conformance:".into(),
        kinds: vec![
            KeyKind::Hash,
            KeyKind::List,
            KeyKind::Set,
            KeyKind::ZSet,
            KeyKind::Stream,
        ],
    };
    suites::run_kv_suites(&*conn, &fixture).await;
}
