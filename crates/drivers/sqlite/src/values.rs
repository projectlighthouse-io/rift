//! SQLite cells to [`Value`].

use rusqlite::types::ValueRef;
use types::Value;

/// Converts a cell, using the declared column type to recognise booleans.
pub(crate) fn value(cell: ValueRef<'_>, decl_type: Option<&str>) -> Value {
    let decl = decl_type.unwrap_or_default().to_ascii_uppercase();
    match cell {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(n) if decl.starts_with("BOOL") => Value::Bool(n != 0),
        ValueRef::Integer(n) => Value::Int(n),
        ValueRef::Real(f) => Value::Float(f),
        ValueRef::Text(t) => Value::Text(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) => Value::Bytes(b.to_vec()),
    }
}

#[cfg(test)]
mod tests {
    use rusqlite::types::ValueRef;
    use types::Value;

    #[test]
    fn booleans_follow_declared_type() {
        assert_eq!(
            super::value(ValueRef::Integer(1), Some("boolean")),
            Value::Bool(true)
        );
        assert_eq!(
            super::value(ValueRef::Integer(1), Some("INTEGER")),
            Value::Int(1)
        );
        assert_eq!(super::value(ValueRef::Null, None), Value::Null);
    }
}
