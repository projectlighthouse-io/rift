//! Conformance suites for app stores and the secret store.

use std::collections::BTreeMap;

use chrono::{TimeZone, Utc};
use traits::{ConnectionStore, DashboardStore, HistoryStore, SecretStore};
use types::{
    ConnectionId, Dashboard, DashboardId, HistoryEntry, SafeMode, SavedConnection, Secret, Tag,
    TlsMode,
};

fn connection(name: &str, group: &str) -> SavedConnection {
    SavedConnection {
        id: ConnectionId::random(),
        name: name.into(),
        driver: "postgres".into(),
        group: Some(group.into()),
        tag: Some(Tag::Green),
        values: BTreeMap::from([("host".into(), "127.0.0.1".into())]),
        tls: TlsMode::Disable,
        safe_mode: SafeMode::Off,
        ssh: None,
    }
}

/// [`ConnectionStore`]: save, update, order and delete.
pub fn connection_store_suite(store: &dyn ConnectionStore) {
    let mut b = connection("b-conn", "Local");
    let a = connection("a-conn", "Local");
    let p = connection("prod", "Production");
    for c in [&b, &a, &p] {
        store.save(c).expect("save");
    }
    let names: Vec<String> = store
        .list()
        .expect("list")
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(
        names,
        ["a-conn", "b-conn", "prod"],
        "ordered by group then name"
    );

    b.safe_mode = SafeMode::ReadOnly;
    store.save(&b).expect("update");
    let list = store.list().expect("list");
    assert_eq!(list.len(), 3, "update does not duplicate");
    assert_eq!(
        list.iter().find(|c| c.id == b.id).expect("b").safe_mode,
        SafeMode::ReadOnly
    );

    store.delete(a.id).expect("delete");
    assert_eq!(store.list().expect("list").len(), 2);
}

/// [`HistoryStore`]: newest first, filtered by connection, limited.
pub fn history_store_suite(store: &dyn HistoryStore) {
    let (one, two) = (ConnectionId::random(), ConnectionId::random());
    for (i, conn) in [one, two, one, one].into_iter().enumerate() {
        store
            .append(&HistoryEntry {
                connection: conn,
                sql: format!("SELECT {i}"),
                ran_at: Utc
                    .timestamp_opt(1_700_000_000 + i as i64, 0)
                    .single()
                    .expect("time"),
                duration_ms: 5,
                rows: Some(1),
                error: None,
            })
            .expect("append");
    }
    let recent: Vec<String> = store
        .recent(Some(one), 2)
        .expect("recent")
        .into_iter()
        .map(|e| e.sql)
        .collect();
    assert_eq!(recent, ["SELECT 3", "SELECT 2"]);
    assert_eq!(store.recent(None, 10).expect("all").len(), 4);
}

/// [`DashboardStore`]: save, update, list per connection, delete.
pub fn dashboard_store_suite(store: &dyn DashboardStore) {
    let conn = ConnectionId::random();
    let mut dash = Dashboard {
        id: DashboardId::random(),
        connection: conn,
        title: "Shop".into(),
        cards: Vec::new(),
    };
    store.save(&dash).expect("save");
    dash.title = "Shop overview".into();
    store.save(&dash).expect("update");
    let list = store.list(conn).expect("list");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].title, "Shop overview");
    assert!(
        store
            .list(ConnectionId::random())
            .expect("other")
            .is_empty()
    );
    store.delete(dash.id).expect("delete");
    assert!(store.list(conn).expect("list").is_empty());
}

/// [`SecretStore`]: set, get, replace and delete.
pub fn secret_store_suite(store: &dyn SecretStore) {
    let id = ConnectionId::random();
    assert_eq!(store.get(id).expect("get"), None);
    store.set(id, &Secret::new("one")).expect("set");
    store.set(id, &Secret::new("two")).expect("replace");
    assert_eq!(
        store.get(id).expect("get").map(|s| s.expose().to_owned()),
        Some("two".into())
    );
    store.delete(id).expect("delete");
    store.delete(id).expect("delete missing is ok");
    assert_eq!(store.get(id).expect("get"), None);
}
