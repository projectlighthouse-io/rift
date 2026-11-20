//! Reads the directory layer: node metadata under `\xfe`, children at
//! `\xfe + pack((parent_prefix, 0, name)) = child_prefix`.

use fdb_ffi::{FdbError, KeySelector, Transaction};
use types::{KeyBytes, KeyDirectory};

use crate::tuple::{self, Item};

const NODE: u8 = 0xfe;
const SUBDIRS: i64 = 0;
/// Directory trees deeper than this are cut off (guards against cycles in bad data).
const MAX_DEPTH: usize = 16;

/// The first key after every key starting with `prefix`.
pub(crate) fn strinc(prefix: &[u8]) -> Vec<u8> {
    let mut end = prefix.to_vec();
    while let Some(last) = end.pop() {
        if last != 0xff {
            end.push(last + 1);
            return end;
        }
    }
    vec![0xff]
}

fn subdirs_key(parent: &[u8]) -> Vec<u8> {
    let mut key = vec![NODE];
    key.extend(tuple::pack(&[
        Item::Bytes(parent.to_vec()),
        Item::Int(SUBDIRS),
    ]));
    key
}

/// Every directory, parents before children, children sorted by name.
pub(crate) fn list(tr: &Transaction) -> Result<Vec<KeyDirectory>, FdbError> {
    let mut out = Vec::new();
    walk(tr, &[NODE], &[], &mut out)?;
    Ok(out)
}

fn walk(
    tr: &Transaction,
    node: &[u8],
    path: &[String],
    out: &mut Vec<KeyDirectory>,
) -> Result<(), FdbError> {
    if path.len() >= MAX_DEPTH {
        return Ok(());
    }
    let begin = subdirs_key(node);
    let end = strinc(&begin);
    let (children, _) = tr.get_range(
        &KeySelector::at_or_after(begin.clone()),
        &KeySelector::at_or_after(end),
        0,
        false,
        true,
    )?;
    for child in children {
        let Some(Item::String(name)) =
            tuple::unpack(&child.key[1..]).and_then(|items| items.into_iter().nth(2))
        else {
            continue;
        };
        let mut child_path = path.to_vec();
        child_path.push(name);
        out.push(KeyDirectory {
            path: child_path.clone(),
            prefix: KeyBytes::new(child.value.clone()),
        });
        walk(tr, &child.value, &child_path, out)?;
    }
    Ok(())
}

#[cfg(test)]
/// Writes a directory entry (used by the seed and tests; real apps allocate prefixes with
/// the high-contention allocator).
pub(crate) fn create(
    tr: &Transaction,
    parent: &[u8],
    name: &str,
    prefix: &[u8],
) -> Result<(), FdbError> {
    tr.set(&entry_key(parent, name), prefix)
}

#[cfg(test)]
/// The key naming `name` under `parent`.
pub(crate) fn entry_key(parent: &[u8], name: &str) -> Vec<u8> {
    let mut key = vec![NODE];
    key.extend(tuple::pack(&[
        Item::Bytes(parent.to_vec()),
        Item::Int(SUBDIRS),
        Item::String(name.into()),
    ]));
    key
}

#[cfg(test)]
/// The root node's id, the parent of top-level directories.
pub(crate) const ROOT: &[u8] = &[NODE];

#[cfg(test)]
mod tests {
    #[test]
    fn strinc_skips_trailing_ff() {
        assert_eq!(super::strinc(b"a"), b"b");
        assert_eq!(super::strinc(b"a\xff"), b"b");
        assert_eq!(super::strinc(b"\x15\x02"), b"\x15\x03");
    }
}
