//! An in-memory key-value driver with Redis-like value types and TTLs.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use futures::FutureExt;
use futures::future::BoxFuture;
use traits::{
    ConnectDeps, Connection, DriverFactory, KeyExpiry, KeyValue, MemoryStats, Transactions,
};
use types::{
    ConnConfig, ConnectionInfo, DriverInfo, Error, Family, FieldKind, FieldSpec, KeyBytes,
    KeyEntry, KeyKind, KeyMatch, KeyPage, KeyScan, KindUsage, KvValue, MemoryReport, PrefixUsage,
    ScanCursor, TxOp,
};

struct Entry {
    value: KvValue,
    expires: Option<Instant>,
}

/// An in-memory key-value connection supporting every key-value capability.
pub struct FakeKv {
    info: ConnectionInfo,
    keys: Mutex<BTreeMap<KeyBytes, Entry>>,
}

impl Default for FakeKv {
    fn default() -> Self {
        Self {
            info: ConnectionInfo {
                driver: "fake-kv".into(),
                server_version: "Fake KV 1.0".into(),
                address: "memory".into(),
                database: Some("0".into()),
            },
            keys: Mutex::new(BTreeMap::new()),
        }
    }
}

impl FakeKv {
    /// Locks the map after dropping expired keys.
    fn lock(&self) -> MutexGuard<'_, BTreeMap<KeyBytes, Entry>> {
        let mut keys = self.keys.lock().expect("fake keys poisoned");
        let now = Instant::now();
        keys.retain(|_, e| e.expires.is_none_or(|t| t > now));
        keys
    }
}

/// Glob match supporting `*` and `?`.
fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match (pattern.first(), text.first()) {
        (None, None) => true,
        (Some(b'*'), _) => {
            glob(&pattern[1..], text) || (!text.is_empty() && glob(pattern, &text[1..]))
        }
        (Some(b'?'), Some(_)) => glob(&pattern[1..], &text[1..]),
        (Some(p), Some(t)) if p == t => glob(&pattern[1..], &text[1..]),
        _ => false,
    }
}

fn size_of(value: &KvValue) -> u64 {
    let bytes: usize = match value {
        KvValue::String(b) => b.len(),
        KvValue::Hash(fields) => fields.iter().map(|f| f.field.len() + f.value.len()).sum(),
        KvValue::List(items) | KvValue::Set(items) => items.iter().map(Vec::len).sum(),
        KvValue::ZSet(members) => members.iter().map(|m| m.member.len() + 8).sum(),
        KvValue::Stream(entries) => entries
            .iter()
            .map(|e| {
                e.id.len()
                    + e.fields
                        .iter()
                        .map(|f| f.field.len() + f.value.len())
                        .sum::<usize>()
            })
            .sum(),
        _ => 0,
    };
    bytes as u64
}

impl Connection for FakeKv {
    fn info(&self) -> &ConnectionInfo {
        &self.info
    }

    fn ping(&self) -> BoxFuture<'_, Result<Duration, Error>> {
        async { Ok(Duration::ZERO) }.boxed()
    }

    fn close(&self) -> BoxFuture<'_, Result<(), Error>> {
        async { Ok(()) }.boxed()
    }

    fn key_value(&self) -> Option<&dyn KeyValue> {
        Some(self)
    }

    fn key_expiry(&self) -> Option<&dyn KeyExpiry> {
        Some(self)
    }

    fn memory_stats(&self) -> Option<&dyn MemoryStats> {
        Some(self)
    }

    fn transactions(&self) -> Option<&dyn Transactions> {
        Some(self)
    }
}

impl KeyValue for FakeKv {
    fn scan<'a>(&'a self, scan: &'a KeyScan) -> BoxFuture<'a, Result<KeyPage, Error>> {
        let keys = self.lock();
        let after = scan.cursor.as_ref().map(|c| KeyBytes::new(c.0.clone()));
        let wanted = |k: &KeyBytes| match &scan.matching {
            KeyMatch::All => true,
            KeyMatch::Prefix(p) => k.as_bytes().starts_with(p.as_bytes()),
            KeyMatch::Pattern(p) => glob(p.as_bytes(), k.as_bytes()),
            KeyMatch::Range { begin, end } => k >= begin && k < end,
            _ => false,
        };
        let iter: Box<dyn Iterator<Item = (&KeyBytes, &Entry)>> = if scan.reverse {
            Box::new(keys.iter().rev())
        } else {
            Box::new(keys.iter())
        };
        let mut entries: Vec<KeyEntry> = iter
            .filter(|(k, _)| match &after {
                Some(a) if scan.reverse => *k < a,
                Some(a) => *k > a,
                None => true,
            })
            .filter(|(k, _)| wanted(k))
            .take(scan.limit as usize + 1)
            .map(|(k, e)| KeyEntry {
                key: k.clone(),
                kind: Some(e.value.kind()),
                size: Some(size_of(&e.value)),
                value: None,
            })
            .collect();
        let cursor = (entries.len() > scan.limit as usize).then(|| {
            entries.pop();
            ScanCursor(
                entries
                    .last()
                    .map(|e| e.key.as_bytes().to_vec())
                    .unwrap_or_default(),
            )
        });
        async move { Ok(KeyPage { entries, cursor }) }.boxed()
    }

    fn get<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<Option<KvValue>, Error>> {
        let value = self.lock().get(key).map(|e| e.value.clone());
        async move { Ok(value) }.boxed()
    }

    fn set<'a>(
        &'a self,
        key: &'a KeyBytes,
        value: &'a KvValue,
    ) -> BoxFuture<'a, Result<(), Error>> {
        let mut keys = self.lock();
        let expires = keys.get(key).and_then(|e| e.expires);
        keys.insert(
            key.clone(),
            Entry {
                value: value.clone(),
                expires,
            },
        );
        async { Ok(()) }.boxed()
    }

    fn delete<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<bool, Error>> {
        let existed = self.lock().remove(key).is_some();
        async move { Ok(existed) }.boxed()
    }

    fn rename<'a>(
        &'a self,
        from: &'a KeyBytes,
        to: &'a KeyBytes,
    ) -> BoxFuture<'a, Result<(), Error>> {
        let mut keys = self.lock();
        let result = if keys.contains_key(to) {
            Err(Error::invalid(format!(
                "key {} already exists",
                to.to_display()
            )))
        } else {
            match keys.remove(from) {
                Some(entry) => {
                    keys.insert(to.clone(), entry);
                    Ok(())
                }
                None => Err(Error::not_found(from.to_display())),
            }
        };
        async move { result }.boxed()
    }

    fn key_count(&self) -> BoxFuture<'_, Result<u64, Error>> {
        let count = self.lock().len() as u64;
        async move { Ok(count) }.boxed()
    }
}

impl KeyExpiry for FakeKv {
    fn ttl<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<Option<Duration>, Error>> {
        let result = match self.lock().get(key) {
            Some(e) => Ok(e
                .expires
                .map(|t| t.saturating_duration_since(Instant::now()))),
            None => Err(Error::not_found(key.to_display())),
        };
        async move { result }.boxed()
    }

    fn set_ttl<'a>(
        &'a self,
        key: &'a KeyBytes,
        ttl: Option<Duration>,
    ) -> BoxFuture<'a, Result<(), Error>> {
        let result = match self.lock().get_mut(key) {
            Some(e) => {
                e.expires = ttl.map(|d| Instant::now() + d);
                Ok(())
            }
            None => Err(Error::not_found(key.to_display())),
        };
        async move { result }.boxed()
    }
}

impl MemoryStats for FakeKv {
    fn memory_report(&self) -> BoxFuture<'_, Result<MemoryReport, Error>> {
        let keys = self.lock();
        let mut prefixes: BTreeMap<String, PrefixUsage> = BTreeMap::new();
        let mut kinds: BTreeMap<KeyKind, u64> = BTreeMap::new();
        let mut used = 0;
        for (k, e) in keys.iter() {
            let text = k.to_display();
            let prefix = text
                .split_once(':')
                .map_or(text.clone(), |(p, _)| format!("{p}:"));
            let bytes = size_of(&e.value) + k.as_bytes().len() as u64;
            used += bytes;
            let usage = prefixes.entry(prefix.clone()).or_insert(PrefixUsage {
                prefix,
                keys: 0,
                bytes: 0,
            });
            usage.keys += 1;
            usage.bytes += bytes;
            *kinds.entry(e.value.kind()).or_default() += 1;
        }
        let mut by_prefix: Vec<PrefixUsage> = prefixes.into_values().collect();
        by_prefix.sort_by_key(|u| std::cmp::Reverse(u.bytes));
        let report = MemoryReport {
            total_keys: keys.len() as u64,
            used_bytes: used,
            by_prefix,
            by_kind: kinds
                .into_iter()
                .map(|(kind, keys)| KindUsage { kind, keys })
                .collect(),
            sampled: false,
        };
        async move { Ok(report) }.boxed()
    }
}

impl Transactions for FakeKv {
    fn atomic<'a>(&'a self, ops: &'a [TxOp]) -> BoxFuture<'a, Result<(), Error>> {
        let result = if ops
            .iter()
            .any(|op| !matches!(op, TxOp::Set { .. } | TxOp::Delete { .. }))
        {
            Err(Error::unsupported("SQL in a key-value transaction"))
        } else {
            let mut keys = self.lock();
            for op in ops {
                match op {
                    TxOp::Set { key, value } => {
                        keys.insert(
                            key.clone(),
                            Entry {
                                value: value.clone(),
                                expires: None,
                            },
                        );
                    }
                    TxOp::Delete { key } => {
                        keys.remove(key);
                    }
                    _ => {}
                }
            }
            Ok(())
        };
        async move { result }.boxed()
    }
}

/// Factory for [`FakeKv`]. Every `connect` returns an empty store.
pub struct FakeKvFactory {
    info: DriverInfo,
}

impl Default for FakeKvFactory {
    fn default() -> Self {
        Self {
            info: DriverInfo {
                id: "fake-kv".into(),
                name: "Fake KV".into(),
                family: Family::KeyValue,
                fields: vec![FieldSpec::new("database", "Database", FieldKind::Text)],
                url_schemes: vec!["fake-kv".into()],
                default_port: None,
            },
        }
    }
}

impl DriverFactory for FakeKvFactory {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect<'a>(
        &'a self,
        _config: &'a ConnConfig,
        _deps: &'a ConnectDeps,
    ) -> BoxFuture<'a, Result<Arc<dyn Connection>, Error>> {
        async { Ok(Arc::new(FakeKv::default()) as Arc<dyn Connection>) }.boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::glob;

    #[test]
    fn glob_matches_star_and_question() {
        assert!(glob(b"cart:*", b"cart:12"));
        assert!(glob(b"a?c", b"abc"));
        assert!(!glob(b"cart:*", b"session:1"));
    }
}
