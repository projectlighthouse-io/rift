//! The FoundationDB tuple layer: encode, decode, display and parse the `("user", 1000)` form.

use std::fmt::{self, Write};

/// One tuple element.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Item {
    Null,
    Bytes(Vec<u8>),
    String(String),
    Nested(Vec<Item>),
    Int(i64),
    Double(f64),
    Bool(bool),
    Uuid([u8; 16]),
    Versionstamp([u8; 12]),
}

const NULL: u8 = 0x00;
const BYTES: u8 = 0x01;
const STRING: u8 = 0x02;
const NESTED: u8 = 0x05;
const INT_ZERO: u8 = 0x14;
const FLOAT: u8 = 0x20;
const DOUBLE: u8 = 0x21;
const FALSE: u8 = 0x26;
const TRUE: u8 = 0x27;
const UUID: u8 = 0x30;
const VERSIONSTAMP: u8 = 0x33;

fn escaped(out: &mut Vec<u8>, bytes: &[u8]) {
    for b in bytes {
        out.push(*b);
        if *b == 0 {
            out.push(0xff);
        }
    }
    out.push(0);
}

fn encode(out: &mut Vec<u8>, item: &Item, nested: bool) {
    match item {
        Item::Null if nested => out.extend([NULL, 0xff]),
        Item::Null => out.push(NULL),
        Item::Bytes(b) => {
            out.push(BYTES);
            escaped(out, b);
        }
        Item::String(s) => {
            out.push(STRING);
            escaped(out, s.as_bytes());
        }
        Item::Nested(items) => {
            out.push(NESTED);
            for i in items {
                encode(out, i, true);
            }
            out.push(0);
        }
        Item::Int(0) => out.push(INT_ZERO),
        Item::Int(v) => {
            let magnitude = v.unsigned_abs();
            let n = 8 - (magnitude.leading_zeros() / 8) as usize;
            let bytes = if *v > 0 { magnitude } else { !magnitude };
            out.push(if *v > 0 {
                INT_ZERO + n as u8
            } else {
                INT_ZERO - n as u8
            });
            out.extend(&bytes.to_be_bytes()[8 - n..]);
        }
        Item::Double(d) => {
            let bits = d.to_bits();
            let bits = if d.is_sign_negative() {
                !bits
            } else {
                bits ^ (1 << 63)
            };
            out.push(DOUBLE);
            out.extend(bits.to_be_bytes());
        }
        Item::Bool(b) => out.push(if *b { TRUE } else { FALSE }),
        Item::Uuid(u) => {
            out.push(UUID);
            out.extend(u);
        }
        Item::Versionstamp(v) => {
            out.push(VERSIONSTAMP);
            out.extend(v);
        }
    }
}

/// Packs a tuple.
pub(crate) fn pack(items: &[Item]) -> Vec<u8> {
    let mut out = Vec::new();
    for i in items {
        encode(&mut out, i, false);
    }
    out
}

/// Reads escaped bytes up to the terminating `\x00`.
fn unescape(bytes: &[u8], mut pos: usize) -> Option<(Vec<u8>, usize)> {
    let mut out = Vec::new();
    loop {
        let b = *bytes.get(pos)?;
        if b == 0 {
            if bytes.get(pos + 1) == Some(&0xff) {
                out.push(0);
                pos += 2;
                continue;
            }
            return Some((out, pos + 1));
        }
        out.push(b);
        pos += 1;
    }
}

fn decode(bytes: &[u8], pos: usize, nested: bool) -> Option<(Item, usize)> {
    let code = *bytes.get(pos)?;
    let fixed = |n: usize| bytes.get(pos + 1..pos + 1 + n);
    Some(match code {
        NULL if nested => (Item::Null, pos + 2),
        NULL => (Item::Null, pos + 1),
        BYTES => {
            let (b, next) = unescape(bytes, pos + 1)?;
            (Item::Bytes(b), next)
        }
        STRING => {
            let (b, next) = unescape(bytes, pos + 1)?;
            (Item::String(String::from_utf8(b).ok()?), next)
        }
        NESTED => {
            let mut items = Vec::new();
            let mut p = pos + 1;
            loop {
                match (bytes.get(p), bytes.get(p + 1)) {
                    (Some(0), Some(0xff)) => {
                        items.push(Item::Null);
                        p += 2;
                    }
                    (Some(0), _) => break,
                    (Some(_), _) => {
                        let (item, next) = decode(bytes, p, true)?;
                        items.push(item);
                        p = next;
                    }
                    (None, _) => return None,
                }
            }
            (Item::Nested(items), p + 1)
        }
        0x0c..=0x1c => {
            let n = code.abs_diff(INT_ZERO) as usize;
            let raw = fixed(n)?;
            let mut buf = [0u8; 8];
            buf[8 - n..].copy_from_slice(raw);
            let magnitude = u64::from_be_bytes(buf);
            let value = if code >= INT_ZERO {
                i64::try_from(magnitude).ok()?
            } else {
                let mask = if n == 8 {
                    u64::MAX
                } else {
                    (1u64 << (8 * n)) - 1
                };
                -i64::try_from(!magnitude & mask).ok()?
            };
            (Item::Int(value), pos + 1 + n)
        }
        FLOAT => {
            let raw: [u8; 4] = fixed(4)?.try_into().ok()?;
            let bits = u32::from_be_bytes(raw);
            let bits = if bits & (1 << 31) != 0 {
                bits ^ (1 << 31)
            } else {
                !bits
            };
            (Item::Double(f64::from(f32::from_bits(bits))), pos + 5)
        }
        DOUBLE => {
            let raw: [u8; 8] = fixed(8)?.try_into().ok()?;
            let bits = u64::from_be_bytes(raw);
            let bits = if bits & (1 << 63) != 0 {
                bits ^ (1 << 63)
            } else {
                !bits
            };
            (Item::Double(f64::from_bits(bits)), pos + 9)
        }
        FALSE => (Item::Bool(false), pos + 1),
        TRUE => (Item::Bool(true), pos + 1),
        UUID => (Item::Uuid(fixed(16)?.try_into().ok()?), pos + 17),
        VERSIONSTAMP => (Item::Versionstamp(fixed(12)?.try_into().ok()?), pos + 13),
        _ => return None,
    })
}

/// Unpacks a whole key; `None` if it is not a valid tuple.
pub(crate) fn unpack(bytes: &[u8]) -> Option<Vec<Item>> {
    let mut items = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
        let (item, next) = decode(bytes, pos, false)?;
        items.push(item);
        pos = next;
    }
    Some(items)
}

/// `\x15\x02user\x00`: printable ASCII as is, everything else as `\xNN`.
pub(crate) fn printable(bytes: &[u8]) -> String {
    let mut out = String::new();
    for b in bytes {
        if b.is_ascii_graphic() && *b != b'\\' || *b == b' ' {
            out.push(char::from(*b));
        } else {
            let _ = write!(out, "\\x{b:02x}");
        }
    }
    out
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

struct Display<'a>(&'a [Item]);

impl fmt::Display for Display<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("(")?;
        for (i, item) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(", ")?;
            }
            match item {
                Item::Null => f.write_str("null")?,
                Item::Bytes(b) => write!(f, "b\"{}\"", printable(b))?,
                Item::String(s) => write!(f, "{s:?}")?,
                Item::Nested(items) => write!(f, "{}", Display(items))?,
                Item::Int(v) => write!(f, "{v}")?,
                Item::Double(d) => write!(f, "{d:?}")?,
                Item::Bool(b) => write!(f, "{b}")?,
                Item::Uuid(u) => write!(f, "uuid({})", hex(u))?,
                Item::Versionstamp(v) => write!(f, "versionstamp({})", hex(v))?,
            }
        }
        if self.0.len() == 1 {
            f.write_str(",")?;
        }
        f.write_str(")")
    }
}

/// `("user", 1000, "profile")`.
pub(crate) fn display(items: &[Item]) -> String {
    Display(items).to_string()
}

/// Parses the display form back; accepts strings, `b"…"`, integers, floats, `true`, `false`,
/// `null` and nested tuples. Outer parentheses are optional.
pub(crate) fn parse(text: &str) -> Result<Vec<Item>, String> {
    let mut p = Parser {
        s: text.trim().as_bytes(),
        pos: 0,
    };
    let items = if p.s.first() == Some(&b'(') {
        p.tuple()?
    } else {
        p.items(None)?
    };
    p.ws();
    if p.pos != p.s.len() {
        return Err(format!("unexpected text at {}", p.pos + 1));
    }
    Ok(items)
}

struct Parser<'a> {
    s: &'a [u8],
    pos: usize,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.s.get(self.pos).is_some_and(u8::is_ascii_whitespace) {
            self.pos += 1;
        }
    }

    fn tuple(&mut self) -> Result<Vec<Item>, String> {
        self.pos += 1;
        let items = self.items(Some(b')'))?;
        self.pos += 1;
        Ok(items)
    }

    fn items(&mut self, close: Option<u8>) -> Result<Vec<Item>, String> {
        let mut items = Vec::new();
        loop {
            self.ws();
            match self.s.get(self.pos) {
                None if close.is_none() => return Ok(items),
                None => return Err("missing )".into()),
                Some(c) if Some(*c) == close => return Ok(items),
                _ => {}
            }
            items.push(self.item()?);
            self.ws();
            if self.s.get(self.pos) == Some(&b',') {
                self.pos += 1;
            }
        }
    }

    fn quoted(&mut self) -> Result<Vec<u8>, String> {
        self.pos += 1;
        let mut out = Vec::new();
        while let Some(c) = self.s.get(self.pos) {
            self.pos += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let esc = *self.s.get(self.pos).ok_or("unfinished escape")?;
                    self.pos += 1;
                    match esc {
                        b'x' => {
                            let hex = self
                                .s
                                .get(self.pos..self.pos + 2)
                                .ok_or("short \\x escape")?;
                            let hex = std::str::from_utf8(hex).map_err(|_| "bad \\x escape")?;
                            out.push(u8::from_str_radix(hex, 16).map_err(|_| "bad \\x escape")?);
                            self.pos += 2;
                        }
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        other => out.push(other),
                    }
                }
                other => out.push(*other),
            }
        }
        Err("missing closing quote".into())
    }

    fn item(&mut self) -> Result<Item, String> {
        let rest = &self.s[self.pos..];
        if rest.first() == Some(&b'(') {
            return self.tuple().map(Item::Nested);
        }
        if rest.first() == Some(&b'"') {
            let bytes = self.quoted()?;
            return String::from_utf8(bytes)
                .map(Item::String)
                .map_err(|_| "string is not UTF-8".into());
        }
        if rest.starts_with(b"b\"") {
            self.pos += 1;
            return self.quoted().map(Item::Bytes);
        }
        let end = rest
            .iter()
            .position(|c| matches!(c, b',' | b')') || c.is_ascii_whitespace())
            .unwrap_or(rest.len());
        let word = std::str::from_utf8(&rest[..end]).map_err(|_| "bad text")?;
        self.pos += end;
        match word {
            "null" | "nil" | "None" => Ok(Item::Null),
            "true" | "True" => Ok(Item::Bool(true)),
            "false" | "False" => Ok(Item::Bool(false)),
            w if w.parse::<i64>().is_ok() => Ok(Item::Int(w.parse().map_err(|_| "bad integer")?)),
            w if w.parse::<f64>().is_ok() => Ok(Item::Double(w.parse().map_err(|_| "bad number")?)),
            w => Err(format!("unknown value `{w}` (quote strings)")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Item, display, pack, parse, printable, unpack};

    #[test]
    fn round_trips_and_matches_reference_bytes() {
        let items = vec![
            Item::String("user".into()),
            Item::Int(1000),
            Item::Int(-5),
            Item::Int(0),
            Item::Bytes(vec![0, 1]),
            Item::Nested(vec![Item::Null, Item::Bool(true)]),
            Item::Double(-1.5),
            Item::Int(i64::MIN + 1),
        ];
        let bytes = pack(&items);
        assert_eq!(unpack(&bytes), Some(items));
        // Reference encodings from the tuple layer spec.
        assert_eq!(
            pack(&[Item::String("user".into()), Item::Int(1000)]),
            b"\x02user\x00\x16\x03\xe8"
        );
        assert_eq!(pack(&[Item::Int(-1)]), b"\x13\xfe");
        assert_eq!(printable(b"\x15\x02user\x00"), "\\x15\\x02user\\x00");
    }

    #[test]
    fn displays_and_parses() {
        let items = parse("(\"user\", 1000, \"profile\")").expect("parse");
        assert_eq!(display(&items), "(\"user\", 1000, \"profile\")");
        assert_eq!(
            parse("\"a\", (1, null), b\"\\x00\"").expect("parse").len(),
            3
        );
        assert_eq!(display(&[Item::Int(1)]), "(1,)");
        assert!(parse("(user)").is_err());
        // Packing orders like the values.
        assert!(
            pack(&parse("(\"user\", 999)").expect("p"))
                < pack(&parse("(\"user\", 1000)").expect("p"))
        );
        assert!(pack(&[Item::Int(-2)]) < pack(&[Item::Int(-1)]));
    }
}
