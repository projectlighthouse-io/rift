//! The FoundationDB connection: KeyValue, Transactions, MemoryStats, KeyDirectories and
//! KeyEncoding. Every call runs one transaction on a blocking thread.

use std::sync::Arc;
use std::time::{Duration, Instant};

use fdb_ffi::{Database, FdbError, KeySelector, Transaction};
use futures::FutureExt;
use futures::future::BoxFuture;
use traits::{
    Connection, KeyDirectories, KeyEncoding, KeyValue, MemoryStats, Spawner, Transactions, spawner,
};
use types::{
    ConnectionInfo, Error, KeyBytes, KeyDirectory, KeyEntry, KeyKind, KeyMatch, KeyPage, KeyScan,
    KvValue, MemoryReport, ScanCursor, TxOp,
};

use crate::{directory, tuple};

/// Per-transaction timeout, the FoundationDB default limit.
pub(crate) const TIMEOUT_MS: i64 = 5_000;
/// Keys read per transaction while counting.
const COUNT_PAGE: i32 = 10_000;
/// System keyspace and the status document.
const SYSTEM: &[u8] = b"\xff";
const STATUS_KEY: &[u8] = b"\xff\xff/status/json";

pub(crate) fn map(e: FdbError) -> Error {
    match e.code {
        0 => Error::MissingDependency { message: e.message },
        1031 => Error::Timeout,
        1101 => Error::Cancelled,
        _ => Error::Query {
            message: e.message,
            code: Some(e.code.to_string()),
        },
    }
}

/// A FoundationDB cluster.
pub(crate) struct FdbConnection {
    pub(crate) db: Arc<Database>,
    pub(crate) spawner: Arc<dyn Spawner>,
    pub(crate) info: ConnectionInfo,
}

impl FdbConnection {
    /// Runs `f` in a retried transaction on a blocking thread.
    async fn run<T, F>(&self, f: F) -> Result<T, Error>
    where
        T: Send + 'static,
        F: FnMut(&Transaction) -> Result<T, FdbError> + Send + 'static,
    {
        let db = self.db.clone();
        spawner::run_blocking(&*self.spawner, move || db.run(TIMEOUT_MS, f))
            .await?
            .map_err(map)
    }

    /// Keys and their key + value bytes in `[begin, end)`, counted up to `cap` keys.
    async fn count_range(
        &self,
        begin: Vec<u8>,
        end: Vec<u8>,
        cap: u64,
    ) -> Result<(u64, u64), Error> {
        let (mut total, mut bytes) = (0u64, 0u64);
        let mut from = KeySelector::at_or_after(begin);
        loop {
            let (start, stop) = (from.clone(), KeySelector::at_or_after(end.clone()));
            let (keys, more) = self
                .run(move |tr| tr.get_range(&start, &stop, COUNT_PAGE, false, true))
                .await?;
            total += keys.len() as u64;
            bytes += keys
                .iter()
                .map(|kv| (kv.key.len() + kv.value.len()) as u64)
                .sum::<u64>();
            match keys.last() {
                Some(last) if more && total < cap => from = KeySelector::after(last.key.clone()),
                _ => return Ok((total.min(cap), bytes)),
            }
        }
    }
}

/// Reads the status document.
pub(crate) fn status(tr: &Transaction) -> Result<serde_json::Value, FdbError> {
    let raw = tr.get(STATUS_KEY, true)?.unwrap_or_default();
    Ok(serde_json::from_slice(&raw).unwrap_or_default())
}

impl Connection for FdbConnection {
    fn info(&self) -> &ConnectionInfo {
        &self.info
    }

    fn ping(&self) -> BoxFuture<'_, Result<Duration, Error>> {
        async move {
            let started = Instant::now();
            self.run(Transaction::read_version).await?;
            Ok(started.elapsed())
        }
        .boxed()
    }

    fn close(&self) -> BoxFuture<'_, Result<(), Error>> {
        async { Ok(()) }.boxed()
    }

    fn key_value(&self) -> Option<&dyn KeyValue> {
        Some(self)
    }

    fn key_directories(&self) -> Option<&dyn KeyDirectories> {
        Some(self)
    }

    fn key_encoding(&self) -> Option<&dyn KeyEncoding> {
        Some(self)
    }

    fn transactions(&self) -> Option<&dyn Transactions> {
        Some(self)
    }

    fn memory_stats(&self) -> Option<&dyn MemoryStats> {
        Some(self)
    }
}

fn bounds(scan: &KeyScan) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let top = if scan.include_system {
        b"\xff\xff".to_vec()
    } else {
        SYSTEM.to_vec()
    };
    Ok(match &scan.matching {
        KeyMatch::All => (Vec::new(), top),
        KeyMatch::Prefix(p) => (
            p.as_bytes().to_vec(),
            directory::strinc(p.as_bytes()).min(top),
        ),
        KeyMatch::Range { begin, end } => {
            (begin.as_bytes().to_vec(), end.as_bytes().to_vec().min(top))
        }
        _ => {
            return Err(Error::unsupported(
                "pattern scans (use a prefix or a range)",
            ));
        }
    })
}

impl KeyValue for FdbConnection {
    fn scan<'a>(&'a self, scan: &'a KeyScan) -> BoxFuture<'a, Result<KeyPage, Error>> {
        async move {
            let (begin, end) = bounds(scan)?;
            // Continue after the last key of the previous page.
            let (begin, end) = match (&scan.cursor, scan.reverse) {
                (Some(c), false) => (
                    KeySelector::after(c.0.clone()),
                    KeySelector::at_or_after(end),
                ),
                (Some(c), true) => (
                    KeySelector::at_or_after(begin),
                    KeySelector::at_or_after(c.0.clone()),
                ),
                (None, _) => (
                    KeySelector::at_or_after(begin),
                    KeySelector::at_or_after(end),
                ),
            };
            let (limit, reverse) = (
                i32::try_from(scan.limit.max(1)).unwrap_or(i32::MAX),
                scan.reverse,
            );
            let (pairs, more, version) = self
                .run(move |tr| {
                    let version = tr.read_version()?;
                    let (pairs, more) = tr.get_range(&begin, &end, limit, reverse, true)?;
                    Ok((pairs, more, version))
                })
                .await?;
            let cursor = pairs
                .last()
                .filter(|_| more)
                .map(|kv| ScanCursor(kv.key.clone()));
            Ok(KeyPage {
                entries: pairs
                    .into_iter()
                    .map(|kv| KeyEntry {
                        key: KeyBytes::new(kv.key),
                        kind: Some(KeyKind::String),
                        size: Some(kv.value.len() as u64),
                        value: Some(kv.value),
                    })
                    .collect(),
                cursor,
                read_version: u64::try_from(version).ok(),
            })
        }
        .boxed()
    }

    fn get<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<Option<KvValue>, Error>> {
        let key = key.as_bytes().to_vec();
        async move {
            Ok(self
                .run(move |tr| tr.get(&key, false))
                .await?
                .map(KvValue::String))
        }
        .boxed()
    }

    fn set<'a>(
        &'a self,
        key: &'a KeyBytes,
        value: &'a KvValue,
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let KvValue::String(bytes) = value else {
                return Err(Error::unsupported("FoundationDB values are plain bytes"));
            };
            let (key, bytes) = (key.as_bytes().to_vec(), bytes.clone());
            self.run(move |tr| tr.set(&key, &bytes)).await
        }
        .boxed()
    }

    fn delete<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<bool, Error>> {
        let key = key.as_bytes().to_vec();
        async move {
            self.run(move |tr| {
                let existed = tr.get(&key, false)?.is_some();
                tr.clear(&key)?;
                Ok(existed)
            })
            .await
        }
        .boxed()
    }

    fn rename<'a>(
        &'a self,
        from: &'a KeyBytes,
        to: &'a KeyBytes,
    ) -> BoxFuture<'a, Result<(), Error>> {
        let (from, to) = (from.as_bytes().to_vec(), to.as_bytes().to_vec());
        async move {
            self.run(move |tr| {
                if tr.get(&to, false)?.is_some() {
                    return Ok(Err(Error::invalid("the new key already exists")));
                }
                let Some(value) = tr.get(&from, false)? else {
                    return Ok(Err(Error::NotFound { what: "key".into() }));
                };
                tr.set(&to, &value)?;
                tr.clear(&from)?;
                Ok(Ok(()))
            })
            .await?
        }
        .boxed()
    }

    fn key_count(&self) -> BoxFuture<'_, Result<u64, Error>> {
        self.count_range(Vec::new(), SYSTEM.to_vec(), 1_000_000)
            .map(|r| r.map(|(keys, _)| keys))
            .boxed()
    }
}

impl Transactions for FdbConnection {
    fn atomic<'a>(&'a self, ops: &'a [TxOp]) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let mut writes = Vec::with_capacity(ops.len());
            for op in ops {
                writes.push(match op {
                    TxOp::Set {
                        key,
                        value: KvValue::String(v),
                    } => (key.as_bytes().to_vec(), Some(v.clone())),
                    TxOp::Delete { key } => (key.as_bytes().to_vec(), None),
                    _ => return Err(Error::unsupported("only byte values and deletes")),
                });
            }
            self.run(move |tr| {
                for (key, value) in &writes {
                    match value {
                        Some(v) => tr.set(key, v)?,
                        None => tr.clear(key)?,
                    }
                }
                Ok(())
            })
            .await
        }
        .boxed()
    }
}

impl MemoryStats for FdbConnection {
    fn memory_report(&self) -> BoxFuture<'_, Result<MemoryReport, Error>> {
        async move {
            let status = self.run(status).await?;
            let (keys, bytes) = self
                .count_range(Vec::new(), SYSTEM.to_vec(), 1_000_000)
                .await?;
            // The status estimate lags fresh writes; fall back to what the count read.
            let estimate = status["cluster"]["data"]["total_kv_size_bytes"]
                .as_u64()
                .unwrap_or(0);
            Ok(MemoryReport {
                total_keys: keys,
                used_bytes: estimate.max(bytes),
                by_prefix: Vec::new(),
                by_kind: Vec::new(),
                sampled: false,
            })
        }
        .boxed()
    }
}

impl KeyDirectories for FdbConnection {
    fn directories(&self) -> BoxFuture<'_, Result<Vec<KeyDirectory>, Error>> {
        self.run(directory::list).boxed()
    }

    fn count<'a>(&'a self, prefix: &'a KeyBytes, cap: u64) -> BoxFuture<'a, Result<u64, Error>> {
        let begin = prefix.as_bytes().to_vec();
        let end = directory::strinc(&begin);
        self.count_range(begin, end, cap)
            .map(|r| r.map(|(keys, _)| keys))
            .boxed()
    }
}

impl KeyEncoding for FdbConnection {
    fn name(&self) -> &'static str {
        "Tuple"
    }

    fn decode(&self, key: &[u8]) -> Option<String> {
        tuple::unpack(key).map(|items| tuple::display(&items))
    }

    fn encode(&self, text: &str) -> Result<Vec<u8>, Error> {
        tuple::parse(text)
            .map(|items| tuple::pack(&items))
            .map_err(Error::invalid)
    }
}
