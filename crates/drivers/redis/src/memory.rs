//! MemoryStats: totals from `INFO`/`DBSIZE`, breakdowns from a `MEMORY USAGE` sample.

use std::collections::BTreeMap;

use futures::FutureExt;
use futures::future::BoxFuture;
use traits::MemoryStats;
use types::{Error, KeyKind, KindUsage, MemoryReport, PrefixUsage};

use crate::connection::{RedisConnection, map};

/// Keys sampled for the breakdowns.
const SAMPLE: usize = 2_000;

fn kind(name: &str) -> Option<KeyKind> {
    Some(match name {
        "string" => KeyKind::String,
        "hash" => KeyKind::Hash,
        "list" => KeyKind::List,
        "set" => KeyKind::Set,
        "zset" => KeyKind::ZSet,
        "stream" => KeyKind::Stream,
        _ => return None,
    })
}

impl MemoryStats for RedisConnection {
    fn memory_report(&self) -> BoxFuture<'_, Result<MemoryReport, Error>> {
        async move {
            let mut conn = self.conn.clone();
            let info: String = redis::cmd("INFO")
                .arg("memory")
                .query_async(&mut conn)
                .await
                .map_err(map)?;
            let used_bytes = info
                .lines()
                .find_map(|l| l.strip_prefix("used_memory:"))
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0);
            let total_keys: u64 = redis::cmd("DBSIZE")
                .query_async(&mut conn)
                .await
                .map_err(map)?;
            let mut keys: Vec<Vec<u8>> = Vec::new();
            let mut cursor = 0u64;
            loop {
                let (next, batch): (u64, Vec<Vec<u8>>) = redis::cmd("SCAN")
                    .arg(cursor)
                    .arg("COUNT")
                    .arg(1000)
                    .query_async(&mut conn)
                    .await
                    .map_err(map)?;
                keys.extend(batch);
                cursor = next;
                if cursor == 0 || keys.len() >= SAMPLE {
                    break;
                }
            }
            keys.truncate(SAMPLE);
            let mut pipe = redis::pipe();
            for k in &keys {
                pipe.cmd("TYPE").arg(k.as_slice());
                pipe.cmd("MEMORY").arg("USAGE").arg(k.as_slice());
            }
            let replies: Vec<redis::Value> = if keys.is_empty() {
                Vec::new()
            } else {
                pipe.query_async(&mut conn).await.map_err(map)?
            };
            let scale = if keys.is_empty() {
                0.0
            } else {
                total_keys as f64 / keys.len() as f64
            };
            let mut prefixes: BTreeMap<String, (u64, u64)> = BTreeMap::new();
            let mut kinds: BTreeMap<KeyKind, u64> = BTreeMap::new();
            for (key, pair) in keys.iter().zip(replies.chunks(2)) {
                let type_name = match &pair[0] {
                    redis::Value::SimpleString(s) => s.clone(),
                    redis::Value::BulkString(b) => String::from_utf8_lossy(b).into_owned(),
                    _ => String::new(),
                };
                let bytes = match pair.get(1) {
                    Some(redis::Value::Int(n)) => *n as u64,
                    _ => 0,
                };
                let text = String::from_utf8_lossy(key);
                let prefix = text
                    .split_once(':')
                    .map_or_else(|| text.to_string(), |(p, _)| format!("{p}:"));
                let entry = prefixes.entry(prefix).or_default();
                entry.0 += 1;
                entry.1 += bytes;
                if let Some(k) = kind(&type_name) {
                    *kinds.entry(k).or_default() += 1;
                }
            }
            let mut by_prefix: Vec<PrefixUsage> = prefixes
                .into_iter()
                .map(|(prefix, (n, b))| PrefixUsage {
                    prefix,
                    keys: (n as f64 * scale).round() as u64,
                    bytes: (b as f64 * scale).round() as u64,
                })
                .collect();
            by_prefix.sort_by_key(|p| std::cmp::Reverse(p.bytes));
            let mut by_kind: Vec<KindUsage> = kinds
                .into_iter()
                .map(|(kind, n)| KindUsage {
                    kind,
                    keys: (n as f64 * scale).round() as u64,
                })
                .collect();
            by_kind.sort_by_key(|k| std::cmp::Reverse(k.keys));
            Ok(MemoryReport {
                total_keys,
                used_bytes,
                by_prefix,
                by_kind,
                sampled: total_keys as usize > keys.len(),
            })
        }
        .boxed()
    }
}
