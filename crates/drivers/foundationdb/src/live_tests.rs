//! Tests against a live cluster (`make fdb-test`), plus the dev seed (`make dev-seed`).

use std::sync::Arc;

use fdb_ffi::{Database, KeySelector};
use types::KeyBytes;

use crate::connection::TIMEOUT_MS;
use crate::directory::{self, ROOT};
use crate::tuple::{self, Item};

fn database() -> Option<Arc<Database>> {
    let required = std::env::var("RIFT_REQUIRE_DOCKER").is_ok();
    let Ok(cluster) = std::env::var("RIFT_FDB_CLUSTER") else {
        assert!(!required, "RIFT_FDB_CLUSTER is not set");
        return None;
    };
    let path = std::env::temp_dir().join("rift-live-test.cluster");
    std::fs::write(&path, cluster).expect("write cluster file");
    match fdb_ffi::open(path.to_str()) {
        Ok(db) => Some(Arc::new(db)),
        Err(e) if !required => {
            eprintln!("skipping: {e}");
            None
        }
        Err(e) => panic!("{e}"),
    }
}

fn key(prefix: &[u8], items: &[Item]) -> Vec<u8> {
    let mut k = prefix.to_vec();
    k.extend(tuple::pack(items));
    k
}

#[test]
fn lists_directories_and_reads_tuple_keys() {
    let Some(db) = database() else { return };
    let (root, child) = (b"\x15\x7f\x01".to_vec(), b"\x15\x7f\x02".to_vec());
    db.run(TIMEOUT_MS, |tr| {
        directory::create(tr, ROOT, "rift_test_dir", &root)?;
        directory::create(tr, &root, "items", &child)?;
        for i in 0..3 {
            tr.set(
                &key(&child, &[Item::String("item".into()), Item::Int(i)]),
                b"{}",
            )?;
        }
        Ok(())
    })
    .expect("write");
    let dirs = db.run(TIMEOUT_MS, directory::list).expect("list");
    let items = dirs
        .iter()
        .find(|d| d.path == ["rift_test_dir", "items"])
        .expect("child directory listed");
    assert_eq!(items.prefix, KeyBytes::new(child.clone()));
    let end = directory::strinc(&child);
    let (pairs, _) = db
        .run(TIMEOUT_MS, |tr| {
            tr.get_range(
                &KeySelector::at_or_after(child.clone()),
                &KeySelector::at_or_after(end.clone()),
                0,
                false,
                true,
            )
        })
        .expect("range");
    let shown: Vec<String> = pairs
        .iter()
        .filter_map(|kv| tuple::unpack(&kv.key[child.len()..]).map(|t| tuple::display(&t)))
        .collect();
    assert_eq!(shown, ["(\"item\", 0)", "(\"item\", 1)", "(\"item\", 2)"]);
    db.run(TIMEOUT_MS, |tr| {
        for kv in &pairs {
            tr.clear(&kv.key)?;
        }
        tr.clear(&directory::entry_key(ROOT, "rift_test_dir"))?;
        tr.clear(&directory::entry_key(&root, "items"))
    })
    .expect("clean up");
}

/// Loads the dev data behind `design/screens/10-foundationdb.png`. Idempotent.
#[test]
#[ignore = "writes dev seed data; run with make dev-seed"]
fn seed_dev_data() {
    let Some(db) = database() else { return };
    let dir = |n: u8| vec![0x15, n];
    let (app, users, orders, sessions, indexes, jobs) =
        (dir(1), dir(2), dir(3), dir(4), dir(5), dir(6));
    db.run(TIMEOUT_MS, |tr| {
        directory::create(tr, ROOT, "app", &app)?;
        directory::create(tr, ROOT, "jobs", &jobs)?;
        directory::create(tr, &app, "users", &users)?;
        directory::create(tr, &app, "orders", &orders)?;
        directory::create(tr, &app, "sessions", &sessions)?;
        directory::create(tr, &app, "indexes", &indexes)
    })
    .expect("directories");
    let names = [
        "amara", "ben", "chloe", "dev", "elif", "farah", "george", "hana", "ivan", "jun", "kemi",
        "leo", "maya", "nils",
    ];
    let write = |prefix: &[u8], count: i64, f: &dyn Fn(i64) -> (Vec<Item>, String)| {
        for chunk in (0..count).collect::<Vec<_>>().chunks(2_000) {
            db.run(TIMEOUT_MS, |tr| {
                for i in chunk {
                    let (items, value) = f(*i);
                    tr.set(&key(prefix, &items), value.as_bytes())?;
                }
                Ok(())
            })
            .expect("seed chunk");
        }
    };
    write(&users, 18_402, &|i| {
        let id = 1000 + i * 7;
        let plan = if i % 3 == 0 { "pro" } else { "free" };
        (
            vec![
                Item::String("user".into()),
                Item::Int(id),
                Item::String("profile".into()),
            ],
            format!(
                "{{\"name\":\"{}\",\"plan\":\"{plan}\"}}",
                names[(i as usize) % names.len()]
            ),
        )
    });
    write(&orders, 96_110, &|i| {
        (
            vec![Item::String("order".into()), Item::Int(50_000 + i)],
            format!(
                "{{\"user\":{},\"total\":{}.{:02}}}",
                1000 + (i % 18_402) * 7,
                10 + i % 300,
                i % 100
            ),
        )
    });
    write(&sessions, 4_377, &|i| {
        (
            vec![Item::String("session".into()), Item::Int(i)],
            format!("{{\"user\":{},\"ttl\":3600}}", 1000 + i * 7),
        )
    });
    write(&indexes, 500, &|i| {
        (
            vec![
                Item::String("by_plan".into()),
                Item::String("pro".into()),
                Item::Int(1000 + i * 21),
            ],
            String::new(),
        )
    });
    write(&jobs, 40, &|i| {
        (
            vec![Item::String("job".into()), Item::Int(i)],
            "{\"state\":\"queued\"}".into(),
        )
    });
    eprintln!("seeded FoundationDB dev data");
}
