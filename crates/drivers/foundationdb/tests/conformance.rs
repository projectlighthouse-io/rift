//! Runs the key-value conformance suites against Docker FoundationDB (`make fdb-test`).
//! Skipped when `RIFT_FDB_CLUSTER` is unset or the client library is missing, unless
//! `RIFT_REQUIRE_DOCKER=1`.

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
async fn foundationdb_passes_kv_suites() {
    let required = std::env::var("RIFT_REQUIRE_DOCKER").is_ok();
    let Ok(cluster) = std::env::var("RIFT_FDB_CLUSTER") else {
        assert!(!required, "RIFT_FDB_CLUSTER is not set");
        eprintln!("skipping: RIFT_FDB_CLUSTER is not set");
        return;
    };
    let config = ConnConfig::new("foundationdb").with("cluster", cluster);
    let deps = ConnectDeps {
        spawner: Arc::new(TokioSpawner),
    };
    let conn = match foundationdb_driver::factory().connect(&config, &deps).await {
        Ok(c) => c,
        Err(e) if !required => {
            eprintln!("skipping: FoundationDB unreachable ({e})");
            return;
        }
        Err(e) => panic!("FoundationDB unreachable: {e}"),
    };
    let fixture = KvFixture {
        prefix: "rift_test:".into(),
        kinds: vec![KeyKind::String],
    };
    suites::run_kv_suites(&*conn, &fixture).await;
}
