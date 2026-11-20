//! KeyValue, KeyExpiry and Transactions.

use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use redis::aio::ConnectionManager;
use redis::{Pipeline, Value as RValue};
use traits::{KeyExpiry, KeyValue, Transactions};
use types::{
    Error, HashField, KeyBytes, KeyEntry, KeyKind, KeyMatch, KeyPage, KeyScan, KvValue, ScanCursor,
    ScoredMember, StreamEntry, TxOp,
};

use crate::connection::{RedisConnection, map};

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

/// Escapes glob metacharacters so a prefix matches literally.
fn glob_escape(prefix: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(prefix.len() + 1);
    for &b in prefix {
        if matches!(b, b'*' | b'?' | b'[' | b']' | b'\\') {
            out.push(b'\\');
        }
        out.push(b);
    }
    out
}

/// Appends the commands that write `value` to `key` (after a `DEL`).
fn write_value(pipe: &mut Pipeline, key: &[u8], value: &KvValue) -> Result<(), Error> {
    match value {
        KvValue::String(v) => {
            pipe.cmd("SET").arg(key).arg(v.as_slice()).ignore();
        }
        KvValue::Hash(fields) if !fields.is_empty() => {
            let mut cmd = redis::cmd("HSET");
            cmd.arg(key);
            for f in fields {
                cmd.arg(f.field.as_slice()).arg(f.value.as_slice());
            }
            pipe.add_command(cmd).ignore();
        }
        KvValue::List(items) | KvValue::Set(items) if !items.is_empty() => {
            let name = if matches!(value, KvValue::List(_)) {
                "RPUSH"
            } else {
                "SADD"
            };
            let mut cmd = redis::cmd(name);
            cmd.arg(key);
            for item in items {
                cmd.arg(item.as_slice());
            }
            pipe.add_command(cmd).ignore();
        }
        KvValue::ZSet(members) if !members.is_empty() => {
            let mut cmd = redis::cmd("ZADD");
            cmd.arg(key);
            for m in members {
                cmd.arg(m.score).arg(m.member.as_slice());
            }
            pipe.add_command(cmd).ignore();
        }
        KvValue::Stream(entries) => {
            for e in entries {
                let mut cmd = redis::cmd("XADD");
                cmd.arg(key).arg(&e.id);
                for f in &e.fields {
                    cmd.arg(f.field.as_slice()).arg(f.value.as_slice());
                }
                pipe.add_command(cmd).ignore();
            }
        }
        KvValue::Hash(_) | KvValue::List(_) | KvValue::Set(_) | KvValue::ZSet(_) => {}
        _ => return Err(Error::unsupported("value kind")),
    }
    Ok(())
}

fn bytes(v: &RValue) -> Vec<u8> {
    match v {
        RValue::BulkString(b) => b.clone(),
        RValue::SimpleString(s) => s.as_bytes().to_vec(),
        RValue::Int(n) => n.to_string().into_bytes(),
        RValue::Double(d) => d.to_string().into_bytes(),
        _ => Vec::new(),
    }
}

fn pairs(values: &[RValue]) -> Vec<HashField> {
    values
        .chunks(2)
        .filter(|c| c.len() == 2)
        .map(|c| HashField {
            field: bytes(&c[0]),
            value: bytes(&c[1]),
        })
        .collect()
}

async fn read(conn: &mut ConnectionManager, key: &[u8]) -> Result<Option<KvValue>, Error> {
    let kind_name: String = redis::cmd("TYPE")
        .arg(key)
        .query_async(conn)
        .await
        .map_err(map)?;
    let value = match kind_name.as_str() {
        "none" => return Ok(None),
        "string" => {
            let v: Vec<u8> = redis::cmd("GET")
                .arg(key)
                .query_async(conn)
                .await
                .map_err(map)?;
            KvValue::String(v)
        }
        "hash" => {
            let v: Vec<RValue> = redis::cmd("HGETALL")
                .arg(key)
                .query_async(conn)
                .await
                .map_err(map)?;
            KvValue::Hash(pairs(&v))
        }
        "list" => {
            let v: Vec<Vec<u8>> = redis::cmd("LRANGE")
                .arg(key)
                .arg(0)
                .arg(-1)
                .query_async(conn)
                .await
                .map_err(map)?;
            KvValue::List(v)
        }
        "set" => {
            let v: Vec<Vec<u8>> = redis::cmd("SMEMBERS")
                .arg(key)
                .query_async(conn)
                .await
                .map_err(map)?;
            KvValue::Set(v)
        }
        "zset" => {
            let v: Vec<(Vec<u8>, f64)> = redis::cmd("ZRANGE")
                .arg(key)
                .arg(0)
                .arg(-1)
                .arg("WITHSCORES")
                .query_async(conn)
                .await
                .map_err(map)?;
            KvValue::ZSet(
                v.into_iter()
                    .map(|(member, score)| ScoredMember { member, score })
                    .collect(),
            )
        }
        "stream" => {
            let v: Vec<(String, Vec<RValue>)> = redis::cmd("XRANGE")
                .arg(key)
                .arg("-")
                .arg("+")
                .query_async(conn)
                .await
                .map_err(map)?;
            KvValue::Stream(
                v.into_iter()
                    .map(|(id, fields)| StreamEntry {
                        id,
                        fields: pairs(&fields),
                    })
                    .collect(),
            )
        }
        other => return Err(Error::unsupported(format!("Redis type {other}"))),
    };
    Ok(Some(value))
}

impl KeyValue for RedisConnection {
    fn scan<'a>(&'a self, scan: &'a KeyScan) -> BoxFuture<'a, Result<KeyPage, Error>> {
        async move {
            let pattern: Vec<u8> = match &scan.matching {
                KeyMatch::All => b"*".to_vec(),
                KeyMatch::Prefix(p) => {
                    let mut v = glob_escape(p.as_bytes());
                    v.push(b'*');
                    v
                }
                KeyMatch::Pattern(p) => p.as_bytes().to_vec(),
                _ => return Err(Error::unsupported("range scans on Redis")),
            };
            let cursor: u64 = scan
                .cursor
                .as_ref()
                .and_then(|c| std::str::from_utf8(&c.0).ok()?.parse().ok())
                .unwrap_or(0);
            let mut conn = self.conn.clone();
            let (next, keys): (u64, Vec<Vec<u8>>) = redis::cmd("SCAN")
                .arg(cursor)
                .arg("MATCH")
                .arg(pattern)
                .arg("COUNT")
                .arg(scan.limit.max(1))
                .query_async(&mut conn)
                .await
                .map_err(map)?;
            let mut pipe = redis::pipe();
            for k in &keys {
                pipe.cmd("TYPE").arg(k.as_slice());
            }
            let kinds: Vec<String> = if keys.is_empty() {
                Vec::new()
            } else {
                pipe.query_async(&mut conn).await.map_err(map)?
            };
            let entries = keys
                .into_iter()
                .zip(kinds)
                .map(|(k, t)| KeyEntry {
                    key: KeyBytes::new(k),
                    kind: kind(&t),
                    size: None,
                    value: None,
                })
                .collect();
            Ok(KeyPage {
                entries,
                cursor: (next != 0).then(|| ScanCursor(next.to_string().into_bytes())),
            })
        }
        .boxed()
    }

    fn get<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<Option<KvValue>, Error>> {
        async move { read(&mut self.conn.clone(), key.as_bytes()).await }.boxed()
    }

    fn set<'a>(
        &'a self,
        key: &'a KeyBytes,
        value: &'a KvValue,
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let mut conn = self.conn.clone();
            let ttl: i64 = redis::cmd("PTTL")
                .arg(key.as_bytes())
                .query_async(&mut conn)
                .await
                .map_err(map)?;
            let mut pipe = redis::pipe();
            pipe.atomic().cmd("DEL").arg(key.as_bytes()).ignore();
            write_value(&mut pipe, key.as_bytes(), value)?;
            if ttl > 0 {
                pipe.cmd("PEXPIRE").arg(key.as_bytes()).arg(ttl).ignore();
            }
            pipe.query_async::<()>(&mut conn).await.map_err(map)
        }
        .boxed()
    }

    fn delete<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<bool, Error>> {
        async move {
            let n: i64 = redis::cmd("DEL")
                .arg(key.as_bytes())
                .query_async(&mut self.conn.clone())
                .await
                .map_err(map)?;
            Ok(n > 0)
        }
        .boxed()
    }

    fn rename<'a>(
        &'a self,
        from: &'a KeyBytes,
        to: &'a KeyBytes,
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let ok: i64 = redis::cmd("RENAMENX")
                .arg(from.as_bytes())
                .arg(to.as_bytes())
                .query_async(&mut self.conn.clone())
                .await
                .map_err(map)?;
            if ok == 1 {
                Ok(())
            } else {
                Err(Error::invalid(format!(
                    "key {} already exists",
                    to.to_display()
                )))
            }
        }
        .boxed()
    }

    fn key_count(&self) -> BoxFuture<'_, Result<u64, Error>> {
        async move {
            redis::cmd("DBSIZE")
                .query_async::<u64>(&mut self.conn.clone())
                .await
                .map_err(map)
        }
        .boxed()
    }
}

impl KeyExpiry for RedisConnection {
    fn ttl<'a>(&'a self, key: &'a KeyBytes) -> BoxFuture<'a, Result<Option<Duration>, Error>> {
        async move {
            let ms: i64 = redis::cmd("PTTL")
                .arg(key.as_bytes())
                .query_async(&mut self.conn.clone())
                .await
                .map_err(map)?;
            match ms {
                -2 => Err(Error::not_found(key.to_display())),
                n if n < 0 => Ok(None),
                n => Ok(Some(Duration::from_millis(n as u64))),
            }
        }
        .boxed()
    }

    fn set_ttl<'a>(
        &'a self,
        key: &'a KeyBytes,
        ttl: Option<Duration>,
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let mut conn = self.conn.clone();
            let n: i64 = match ttl {
                Some(d) => {
                    redis::cmd("PEXPIRE")
                        .arg(key.as_bytes())
                        .arg(d.as_millis() as u64)
                        .query_async(&mut conn)
                        .await
                }
                None => {
                    let exists: i64 = redis::cmd("EXISTS")
                        .arg(key.as_bytes())
                        .query_async(&mut conn)
                        .await
                        .map_err(map)?;
                    let _: i64 = redis::cmd("PERSIST")
                        .arg(key.as_bytes())
                        .query_async(&mut conn)
                        .await
                        .map_err(map)?;
                    Ok(exists)
                }
            }
            .map_err(map)?;
            if n == 0 {
                Err(Error::not_found(key.to_display()))
            } else {
                Ok(())
            }
        }
        .boxed()
    }
}

impl Transactions for RedisConnection {
    fn atomic<'a>(&'a self, ops: &'a [TxOp]) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let mut pipe = redis::pipe();
            pipe.atomic();
            for op in ops {
                match op {
                    TxOp::Set { key, value } => {
                        pipe.cmd("DEL").arg(key.as_bytes()).ignore();
                        write_value(&mut pipe, key.as_bytes(), value)?;
                    }
                    TxOp::Delete { key } => {
                        pipe.cmd("DEL").arg(key.as_bytes()).ignore();
                    }
                    _ => return Err(Error::unsupported("SQL in a Redis transaction")),
                }
            }
            if ops.is_empty() {
                return Ok(());
            }
            pipe.query_async::<()>(&mut self.conn.clone())
                .await
                .map_err(map)
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn escapes_glob_characters() {
        assert_eq!(super::glob_escape(b"a*b?[c]"), b"a\\*b\\?\\[c\\]".to_vec());
    }
}
