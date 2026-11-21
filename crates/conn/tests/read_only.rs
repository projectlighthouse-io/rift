//! Read-only safe mode blocks every write path the fakes offer.

use std::sync::Arc;

use conn::ReadOnlyConnection;
use futures::executor::block_on;
use testkit::{FakeKv, FakeSql};
use tokio_util::sync::CancellationToken;
use traits::Connection;
use types::{Error, KeyBytes, KvValue};

#[test]
fn read_only_sql_allows_reads_and_refuses_writes() {
    let conn = ReadOnlyConnection::new(Arc::new(FakeSql::new()), "postgres");
    assert!(conn.edit_rows().is_none(), "row edits hidden");
    assert!(conn.edit_schema().is_none(), "schema edits hidden");
    assert!(conn.transactions().is_none(), "transactions hidden");
    if let Some(backup) = conn.backup() {
        let restored =
            block_on(backup.restore(std::path::Path::new("x.sql"), CancellationToken::new()));
        assert!(
            matches!(restored, Err(Error::ReadOnly { .. })),
            "restore refused"
        );
    }
    let query = conn.query().expect("query kept");
    let cancel = CancellationToken::new;
    for write in [
        "DELETE FROM rift_conformance",
        "SET default_transaction_read_only = off",
        "nonsense",
    ] {
        let result = block_on(query.execute(write, cancel()));
        assert!(
            matches!(result, Err(Error::ReadOnly { .. })),
            "{write}: {:?}",
            result.err()
        );
    }
    assert!(
        !matches!(
            block_on(query.execute("SELECT * FROM rift_conformance", cancel())),
            Err(Error::ReadOnly { .. })
        ),
        "reads pass the guard"
    );
}

#[test]
fn read_only_key_value_refuses_writes() {
    let inner = Arc::new(FakeKv::default());
    let key = KeyBytes::from("k");
    block_on(
        inner
            .key_value()
            .expect("kv")
            .set(&key, &KvValue::String(b"v".to_vec())),
    )
    .expect("seed");
    let conn = ReadOnlyConnection::new(inner, "fake-kv");
    let kv = conn.key_value().expect("kv kept");
    assert_eq!(
        block_on(kv.get(&key)).expect("get"),
        Some(KvValue::String(b"v".to_vec()))
    );
    assert!(matches!(
        block_on(kv.set(&key, &KvValue::String(vec![]))),
        Err(Error::ReadOnly { .. })
    ));
    assert!(matches!(
        block_on(kv.delete(&key)),
        Err(Error::ReadOnly { .. })
    ));
    assert!(matches!(
        block_on(kv.rename(&key, &KeyBytes::from("k2"))),
        Err(Error::ReadOnly { .. })
    ));
    let expiry = conn.key_expiry().expect("expiry kept");
    assert!(matches!(
        block_on(expiry.set_ttl(&key, None)),
        Err(Error::ReadOnly { .. })
    ));
}
