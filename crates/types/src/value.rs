//! Cell values.

use std::fmt;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single cell value. Types a driver cannot map become [`Value::Unknown`] and render as raw
/// text instead of failing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Value {
    /// SQL `NULL` or a missing value.
    Null,
    /// Boolean.
    Bool(bool),
    /// Signed integer.
    Int(i64),
    /// Unsigned integer (MySQL `UNSIGNED`, ClickHouse `UInt*`).
    UInt(u64),
    /// Floating point.
    Float(f64),
    /// Exact decimal.
    Decimal(Decimal),
    /// Text.
    Text(String),
    /// Binary data.
    Bytes(Vec<u8>),
    /// JSON document.
    Json(serde_json::Value),
    /// UUID.
    Uuid(Uuid),
    /// Calendar date.
    Date(NaiveDate),
    /// Time of day.
    Time(NaiveTime),
    /// Timestamp without zone.
    Timestamp(NaiveDateTime),
    /// Timestamp with zone.
    TimestampTz(DateTime<FixedOffset>),
    /// Array of values.
    Array(Vec<Value>),
    /// A value of a type Rift does not model; shown as raw text.
    Unknown {
        /// Database type name.
        type_name: String,
        /// Text rendering from the database.
        raw: String,
    },
}

impl Value {
    /// True for [`Value::Null`].
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    /// True for numeric variants, which the grid right-aligns.
    #[must_use]
    pub fn is_numeric(&self) -> bool {
        matches!(
            self,
            Value::Int(_) | Value::UInt(_) | Value::Float(_) | Value::Decimal(_)
        )
    }

    /// Best-effort conversion to `f64` for charts.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(v) => Some(*v as f64),
            Value::UInt(v) => Some(*v as f64),
            Value::Float(v) => Some(*v),
            Value::Decimal(v) => v.to_string().parse().ok(),
            Value::Text(v) => v.parse().ok(),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("NULL"),
            Value::Bool(v) => write!(f, "{v}"),
            Value::Int(v) => write!(f, "{v}"),
            Value::UInt(v) => write!(f, "{v}"),
            Value::Float(v) => write!(f, "{v}"),
            Value::Decimal(v) => write!(f, "{v}"),
            Value::Text(v) => f.write_str(v),
            Value::Bytes(v) => {
                f.write_str("\\x")?;
                v.iter().try_for_each(|b| write!(f, "{b:02x}"))
            }
            Value::Json(v) => write!(f, "{v}"),
            Value::Uuid(v) => write!(f, "{v}"),
            Value::Date(v) => write!(f, "{v}"),
            Value::Time(v) => write!(f, "{v}"),
            Value::Timestamp(v) => write!(f, "{}", v.format("%Y-%m-%d %H:%M:%S")),
            Value::TimestampTz(v) => write!(f, "{}", v.format("%Y-%m-%d %H:%M:%S")),
            Value::Array(items) => {
                f.write_str("{")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(",")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("}")
            }
            Value::Unknown { raw, .. } => f.write_str(raw),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Value;

    #[test]
    fn display_renders_null_and_bytes() {
        assert_eq!(Value::Null.to_string(), "NULL");
        assert_eq!(Value::Bytes(vec![0xde, 0xad]).to_string(), "\\xdead");
        assert_eq!(
            Value::Array(vec![Value::Int(1), Value::Int(2)]).to_string(),
            "{1,2}"
        );
    }
}
