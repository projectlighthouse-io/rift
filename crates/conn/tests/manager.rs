//! Manager behaviour against the fake drivers and in-memory stores.

use std::sync::Arc;

use conn::ConnectionManager;
use futures::executor::block_on;
use testkit::{FakeSqlFactory, MemoryConnectionStore, MemorySecretStore, ThreadSpawner};
use traits::{ConnectDeps, DriverFactory};
use types::{ConnectionId, SafeMode, SavedConnection, Secret, TlsMode};

fn manager() -> ConnectionManager {
    let drivers: Vec<Box<dyn DriverFactory>> = vec![Box::new(FakeSqlFactory::default())];
    ConnectionManager::new(
        drivers,
        Arc::new(MemoryConnectionStore::default()),
        Arc::new(MemorySecretStore::default()),
        ConnectDeps {
            spawner: Arc::new(ThreadSpawner),
        },
    )
}

#[test]
fn saves_password_outside_the_store_and_connects() {
    let m = manager();
    let saved = SavedConnection {
        id: ConnectionId::random(),
        name: "fake".into(),
        driver: "fake-sql".into(),
        group: None,
        tag: None,
        values: Default::default(),
        tls: TlsMode::Disable,
        safe_mode: SafeMode::Off,
        ssh: None,
    };
    m.save(&saved, Some(&Secret::new("pw"))).expect("save");
    assert_eq!(
        m.password(saved.id)
            .expect("pw")
            .map(|s| s.expose().to_owned()),
        Some("pw".into())
    );
    let config = ConnectionManager::config(&saved, m.password(saved.id).expect("pw"));
    assert!(block_on(m.test(&config)).is_ok());
    m.delete(saved.id).expect("delete");
    assert!(m.list().expect("list").is_empty());
    assert_eq!(m.password(saved.id).expect("pw"), None);
}

/// Touches the real keychain; run with `cargo test -p conn -- --ignored`.
#[test]
#[ignore = "uses the OS keychain"]
fn keychain_passes_secret_suite() {
    testkit::suites::secret_store_suite(&conn::KeychainSecretStore);
}
