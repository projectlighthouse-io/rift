//! Text-protocol cells to [`Value`].

use std::str::FromStr;

use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rust_decimal::Decimal;
use types::Value;

/// Parses a cell in MySQL text format. `type_name` is sqlx's upper-case type name; `BOOLEAN`
/// is how it reports `tinyint(1)`.
pub(crate) fn parse(type_name: &str, text: &str) -> Value {
    let unsigned = type_name.contains("UNSIGNED");
    let base = type_name.split_whitespace().next().unwrap_or(type_name);
    let parsed = match base {
        "BOOLEAN" => match text {
            "1" => Some(Value::Bool(true)),
            "0" => Some(Value::Bool(false)),
            _ => None,
        },
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" | "YEAR" if unsigned => {
            text.parse().ok().map(Value::UInt)
        }
        "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" | "YEAR" => {
            text.parse().ok().map(Value::Int)
        }
        "FLOAT" | "DOUBLE" => text.parse().ok().map(Value::Float),
        "DECIMAL" => Decimal::from_str(text).ok().map(Value::Decimal),
        "DATE" => NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .ok()
            .map(Value::Date),
        "TIME" => NaiveTime::parse_from_str(text, "%H:%M:%S%.f")
            .ok()
            .map(Value::Time),
        "DATETIME" | "TIMESTAMP" => NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.f")
            .ok()
            .map(Value::Timestamp),
        "JSON" => serde_json::from_str(text).ok().map(Value::Json),
        "CHAR" | "VARCHAR" | "TEXT" | "TINYTEXT" | "MEDIUMTEXT" | "LONGTEXT" | "ENUM" | "SET" => {
            Some(Value::Text(text.into()))
        }
        _ => None,
    };
    parsed.unwrap_or_else(|| Value::Unknown {
        type_name: type_name.to_lowercase(),
        raw: text.into(),
    })
}

#[cfg(test)]
mod tests {
    use types::Value;

    use super::parse;

    #[test]
    fn maps_mysql_types() {
        assert_eq!(parse("BOOLEAN", "1"), Value::Bool(true));
        assert_eq!(
            parse("BIGINT UNSIGNED", "18446744073709551615"),
            Value::UInt(u64::MAX)
        );
        assert_eq!(parse("INT", "-3"), Value::Int(-3));
        assert_eq!(parse("ENUM", "paid"), Value::Text("paid".into()));
        assert!(matches!(
            parse("DATETIME", "2026-11-18 09:12:04"),
            Value::Timestamp(_)
        ));
        assert!(matches!(parse("GEOMETRY", "x"), Value::Unknown { .. }));
    }
}
