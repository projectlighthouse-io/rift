//! The fakes must pass every suite they claim.

use futures::executor::block_on;
use testkit::suites::{self, KvFixture};
use testkit::{
    FakeKv, FakeSql, MemoryConnectionStore, MemoryDashboardStore, MemoryHistoryStore,
    MemorySecretStore,
};
use types::KeyKind;

#[test]
fn fake_sql_passes_sql_suites() {
    block_on(suites::run_sql_suites(&FakeSql::new(), &FakeSql::fixture()));
}

#[test]
fn fake_kv_passes_kv_suites() {
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
    block_on(suites::run_kv_suites(&FakeKv::default(), &fixture));
}

#[test]
fn memory_stores_pass_store_suites() {
    suites::connection_store_suite(&MemoryConnectionStore::default());
    suites::history_store_suite(&MemoryHistoryStore::default());
    suites::dashboard_store_suite(&MemoryDashboardStore::default());
    suites::secret_store_suite(&MemorySecretStore::default());
}
