//! Conformance suites for key-value capabilities.

use std::collections::BTreeSet;
use std::time::Duration;

use traits::Connection;
use types::{
    HashField, KeyBytes, KeyKind, KeyMatch, KeyScan, KvValue, ScoredMember, StreamEntry, TxOp,
};

/// What the key-value suites may touch.
#[derive(Debug, Clone)]
pub struct KvFixture {
    /// Every key the suites create starts with this; it is cleared before and after.
    pub prefix: String,
    /// Value kinds the engine stores natively, besides `String`.
    pub kinds: Vec<KeyKind>,
}

impl KvFixture {
    fn key(&self, name: &str) -> KeyBytes {
        KeyBytes::from(format!("{}{name}", self.prefix).as_str())
    }
}

fn sample(kind: KeyKind) -> KvValue {
    let b = |s: &str| s.as_bytes().to_vec();
    match kind {
        KeyKind::Hash => KvValue::Hash(vec![HashField {
            field: b("f"),
            value: b("v"),
        }]),
        KeyKind::List => KvValue::List(vec![b("a"), b("b")]),
        KeyKind::Set => KvValue::Set(vec![b("a")]),
        KeyKind::ZSet => KvValue::ZSet(vec![ScoredMember {
            member: b("m"),
            score: 2.5,
        }]),
        KeyKind::Stream => KvValue::Stream(vec![StreamEntry {
            id: "1-1".into(),
            fields: vec![HashField {
                field: b("f"),
                value: b("v"),
            }],
        }]),
        _ => KvValue::String(b("v")),
    }
}

/// Every key under the prefix, following cursors.
async fn scan_all(conn: &dyn Connection, fx: &KvFixture) -> BTreeSet<KeyBytes> {
    let kv = conn.key_value().expect("key_value");
    let mut scan = KeyScan {
        matching: KeyMatch::Prefix(KeyBytes::from(fx.prefix.as_str())),
        limit: 2,
        reverse: false,
        cursor: None,
        include_system: false,
    };
    let mut keys = BTreeSet::new();
    for _ in 0..10_000 {
        let page = kv.scan(&scan).await.expect("scan");
        keys.extend(page.entries.into_iter().map(|e| e.key));
        match page.cursor {
            Some(cursor) => scan.cursor = Some(cursor),
            None => return keys,
        }
    }
    panic!("scan did not finish");
}

async fn clear(conn: &dyn Connection, fx: &KvFixture) {
    let kv = conn.key_value().expect("key_value");
    for key in scan_all(conn, fx).await {
        kv.delete(&key).await.expect("delete");
    }
}

/// Runs every key-value suite whose capability the connection reports.
pub async fn run_kv_suites(conn: &dyn Connection, fx: &KvFixture) {
    if conn.key_value().is_some() {
        key_value_suite(conn, fx).await;
    }
    if conn.key_expiry().is_some() {
        key_expiry_suite(conn, fx).await;
    }
    if conn.memory_stats().is_some() {
        memory_stats_suite(conn, fx).await;
    }
    if conn.transactions().is_some() {
        kv_transactions_suite(conn, fx).await;
    }
}

/// [`traits::KeyValue`]: set, get, scan with paging, rename and delete.
pub async fn key_value_suite(conn: &dyn Connection, fx: &KvFixture) {
    let kv = conn.key_value().expect("key_value");
    clear(conn, fx).await;

    let a = fx.key("a");
    kv.set(&a, &KvValue::String(b"1".to_vec()))
        .await
        .expect("set");
    assert_eq!(
        kv.get(&a).await.expect("get"),
        Some(KvValue::String(b"1".to_vec()))
    );
    assert_eq!(kv.get(&fx.key("missing")).await.expect("get missing"), None);

    let mut expected = BTreeSet::from([a.clone()]);
    for kind in &fx.kinds {
        let key = fx.key(kind.label());
        kv.set(&key, &sample(*kind)).await.expect("set kind");
        let read = kv.get(&key).await.expect("get kind").expect("kind exists");
        assert_eq!(read, sample(*kind), "{kind:?} round-trips");
        expected.insert(key);
    }
    for i in 0..5 {
        let key = fx.key(&format!("page:{i}"));
        kv.set(&key, &KvValue::String(vec![b'x']))
            .await
            .expect("set page key");
        expected.insert(key);
    }
    assert_eq!(
        scan_all(conn, fx).await,
        expected,
        "paged scan returns every key once"
    );

    let b = fx.key("b");
    kv.rename(&a, &b).await.expect("rename");
    assert_eq!(kv.get(&a).await.expect("get old"), None);
    assert!(kv.get(&b).await.expect("get new").is_some());

    assert!(
        kv.delete(&b).await.expect("delete"),
        "delete reports existing key"
    );
    assert!(
        !kv.delete(&b).await.expect("delete again"),
        "second delete is a no-op"
    );
    kv.key_count().await.expect("key count");
    clear(conn, fx).await;
}

/// [`traits::KeyExpiry`]: set, read and clear a TTL.
pub async fn key_expiry_suite(conn: &dyn Connection, fx: &KvFixture) {
    let kv = conn.key_value().expect("key_value");
    let expiry = conn.key_expiry().expect("key_expiry");
    let key = fx.key("ttl");
    kv.set(&key, &KvValue::String(b"v".to_vec()))
        .await
        .expect("set");
    assert_eq!(
        expiry.ttl(&key).await.expect("ttl"),
        None,
        "new keys never expire"
    );

    expiry
        .set_ttl(&key, Some(Duration::from_secs(100)))
        .await
        .expect("set ttl");
    let ttl = expiry.ttl(&key).await.expect("ttl").expect("has ttl");
    assert!(
        ttl > Duration::from_secs(90) && ttl <= Duration::from_secs(100),
        "ttl {ttl:?}"
    );

    expiry.set_ttl(&key, None).await.expect("persist");
    assert_eq!(expiry.ttl(&key).await.expect("ttl"), None);
    kv.delete(&key).await.expect("delete");
}

/// [`traits::MemoryStats`]: reports at least the keys present.
pub async fn memory_stats_suite(conn: &dyn Connection, fx: &KvFixture) {
    let kv = conn.key_value().expect("key_value");
    let key = fx.key("mem");
    kv.set(&key, &KvValue::String(vec![0; 64]))
        .await
        .expect("set");
    let report = conn
        .memory_stats()
        .expect("memory_stats")
        .memory_report()
        .await
        .expect("report");
    assert!(report.total_keys >= 1);
    assert!(report.used_bytes > 0);
    kv.delete(&key).await.expect("delete");
}

/// [`traits::Transactions`] for key-value: batches apply together or not at all.
pub async fn kv_transactions_suite(conn: &dyn Connection, fx: &KvFixture) {
    let kv = conn.key_value().expect("key_value");
    let tx = conn.transactions().expect("transactions");
    let (a, b) = (fx.key("tx:a"), fx.key("tx:b"));
    let v = KvValue::String(b"v".to_vec());
    tx.atomic(&[
        TxOp::Set {
            key: a.clone(),
            value: v.clone(),
        },
        TxOp::Set {
            key: b.clone(),
            value: v,
        },
    ])
    .await
    .expect("batch");
    assert!(kv.get(&a).await.expect("a").is_some() && kv.get(&b).await.expect("b").is_some());

    let failed = tx
        .atomic(&[
            TxOp::Delete { key: a.clone() },
            TxOp::Sql { sql: "x".into() },
        ])
        .await;
    assert!(failed.is_err(), "unsupported op fails the batch");
    assert!(kv.get(&a).await.expect("a").is_some(), "nothing applied");

    tx.atomic(&[TxOp::Delete { key: a }, TxOp::Delete { key: b }])
        .await
        .expect("cleanup");
}
