//! The SQLite store passes the same suites as the in-memory fakes.

use store::SqliteStore;
use testkit::suites;

#[test]
fn sqlite_store_passes_store_suites() {
    suites::connection_store_suite(&SqliteStore::in_memory().expect("db"));
    suites::history_store_suite(&SqliteStore::in_memory().expect("db"));
    suites::dashboard_store_suite(&SqliteStore::in_memory().expect("db"));
}

#[test]
fn data_survives_reopen() {
    let dir = std::env::temp_dir().join(format!("rift-store-{}", std::process::id()));
    let path = dir.join("rift.db");
    suites::connection_store_suite(&SqliteStore::open(&path).expect("open"));
    let reopened = SqliteStore::open(&path).expect("reopen");
    assert_eq!(
        traits::ConnectionStore::list(&reopened)
            .expect("list")
            .len(),
        2
    );
    std::fs::remove_dir_all(dir).expect("cleanup");
}
