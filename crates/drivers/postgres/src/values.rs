//! Text-protocol cells to [`Value`].

use std::str::FromStr;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime};
use rust_decimal::Decimal;
use types::Value;

/// Parses a cell in Postgres text output format. Unknown types keep their raw text.
pub(crate) fn parse(type_name: &str, text: &str) -> Value {
    let parsed = match type_name {
        "bool" => match text {
            "t" => Some(Value::Bool(true)),
            "f" => Some(Value::Bool(false)),
            _ => None,
        },
        "int2" | "int4" | "int8" => text.parse().ok().map(Value::Int),
        "oid" | "xid" => text.parse().ok().map(Value::UInt),
        "float4" | "float8" => text.parse().ok().map(Value::Float),
        "numeric" => Decimal::from_str(text).ok().map(Value::Decimal),
        "text" | "varchar" | "bpchar" | "name" | "char" | "citext" => {
            Some(Value::Text(text.into()))
        }
        "json" | "jsonb" => serde_json::from_str(text).ok().map(Value::Json),
        "uuid" => uuid::Uuid::parse_str(text).ok().map(Value::Uuid),
        "date" => NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .ok()
            .map(Value::Date),
        "time" => NaiveTime::parse_from_str(text, "%H:%M:%S%.f")
            .ok()
            .map(Value::Time),
        "timestamp" => NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f")
            .ok()
            .map(Value::Timestamp),
        "timestamptz" => DateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f%#z")
            .ok()
            .map(Value::TimestampTz),
        "bytea" => text.strip_prefix("\\x").and_then(hex).map(Value::Bytes),
        _ => None,
    };
    parsed.unwrap_or_else(|| Value::Unknown {
        type_name: type_name.into(),
        raw: text.into(),
    })
}

fn hex(text: &str) -> Option<Vec<u8>> {
    (0..text.len())
        .step_by(2)
        .map(|i| {
            text.get(i..i + 2)
                .and_then(|b| u8::from_str_radix(b, 16).ok())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use types::Value;

    use super::parse;

    #[test]
    fn parses_common_types_and_keeps_unknown_raw() {
        assert_eq!(parse("int8", "42"), Value::Int(42));
        assert_eq!(parse("bool", "t"), Value::Bool(true));
        assert_eq!(parse("bytea", "\\xdead"), Value::Bytes(vec![0xde, 0xad]));
        assert!(matches!(
            parse("timestamptz", "2026-11-18 09:12:04+00"),
            Value::TimestampTz(_)
        ));
        assert!(matches!(parse("numeric", "129.00"), Value::Decimal(_)));
        assert_eq!(
            parse("interval", "1 day"),
            Value::Unknown {
                type_name: "interval".into(),
                raw: "1 day".into()
            }
        );
    }
}
