//! JSON cells to [`Value`], by ClickHouse type name.

use std::str::FromStr;

use chrono::{NaiveDate, NaiveDateTime};
use rust_decimal::Decimal;
use serde_json::Value as Json;
use types::Value;

/// Strips `Nullable(...)` and `LowCardinality(...)` wrappers.
pub(crate) fn base_type(name: &str) -> &str {
    let mut t = name;
    for wrapper in ["Nullable(", "LowCardinality("] {
        if let Some(inner) = t.strip_prefix(wrapper).and_then(|s| s.strip_suffix(')')) {
            t = inner;
        }
    }
    for wrapper in ["Nullable(", "LowCardinality("] {
        if let Some(inner) = t.strip_prefix(wrapper).and_then(|s| s.strip_suffix(')')) {
            t = inner;
        }
    }
    t
}

fn text(j: &Json) -> String {
    match j {
        Json::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Converts one cell.
pub(crate) fn value(type_name: &str, cell: &Json) -> Value {
    if cell.is_null() {
        return Value::Null;
    }
    let t = base_type(type_name);
    let name = t.split('(').next().unwrap_or(t);
    let parsed = match name {
        "UInt8" | "UInt16" | "UInt32" | "UInt64" => text(cell).parse().ok().map(Value::UInt),
        "Int8" | "Int16" | "Int32" | "Int64" => text(cell).parse().ok().map(Value::Int),
        "Float32" | "Float64" => cell
            .as_f64()
            .or_else(|| text(cell).parse().ok())
            .map(Value::Float),
        "Decimal" | "Decimal32" | "Decimal64" | "Decimal128" => {
            Decimal::from_str(&text(cell)).ok().map(Value::Decimal)
        }
        "Bool" => cell.as_bool().map(Value::Bool),
        "String" | "FixedString" | "Enum8" | "Enum16" | "IPv4" | "IPv6" => {
            Some(Value::Text(text(cell)))
        }
        "UUID" => uuid::Uuid::parse_str(&text(cell)).ok().map(Value::Uuid),
        "Date" | "Date32" => NaiveDate::parse_from_str(&text(cell), "%Y-%m-%d")
            .ok()
            .map(Value::Date),
        "DateTime" | "DateTime64" => {
            NaiveDateTime::parse_from_str(&text(cell), "%Y-%m-%d %H:%M:%S%.f")
                .ok()
                .map(Value::Timestamp)
        }
        "Array" => {
            let inner = t
                .strip_prefix("Array(")
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or("String");
            cell.as_array()
                .map(|items| Value::Array(items.iter().map(|i| value(inner, i)).collect()))
        }
        "Map" | "Tuple" | "JSON" | "Object" => Some(Value::Json(cell.clone())),
        _ => None,
    };
    parsed.unwrap_or_else(|| Value::Unknown {
        type_name: type_name.to_owned(),
        raw: text(cell),
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use types::Value;

    use super::{base_type, value};

    #[test]
    fn maps_clickhouse_types() {
        assert_eq!(base_type("LowCardinality(Nullable(String))"), "String");
        assert_eq!(
            value("UInt64", &json!("18446744073709551615")),
            Value::UInt(u64::MAX)
        );
        assert_eq!(value("Nullable(Float64)", &json!(null)), Value::Null);
        assert!(matches!(
            value("DateTime", &json!("2026-11-18 09:12:04")),
            Value::Timestamp(_)
        ));
        assert_eq!(
            value("Array(UInt8)", &json!([1, 2])),
            Value::Array(vec![Value::UInt(1), Value::UInt(2)])
        );
        assert!(matches!(
            value("AggregateFunction(sum, UInt64)", &json!("x")),
            Value::Unknown { .. }
        ));
    }
}
