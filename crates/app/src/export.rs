//! Result rows as CSV, JSON or INSERT statements.

use dialect::Dialect;
use types::{Column, Value};

fn csv_field(value: &Value) -> String {
    let text = match value {
        Value::Null => return String::new(),
        v => v.to_string(),
    };
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text
    }
}

/// RFC 4180 CSV with a header row; `NULL` is an empty field.
pub(crate) fn csv<'a>(columns: &[Column], rows: impl Iterator<Item = &'a [Value]>) -> String {
    let mut out = columns
        .iter()
        .map(|c| csv_field(&Value::Text(c.name.to_string())))
        .collect::<Vec<_>>()
        .join(",");
    out.push('\n');
    for row in rows {
        out.push_str(&row.iter().map(csv_field).collect::<Vec<_>>().join(","));
        out.push('\n');
    }
    out
}

pub(crate) fn json_value(value: &Value) -> serde_json::Value {
    match value {
        Value::Null => serde_json::Value::Null,
        Value::Bool(b) => (*b).into(),
        Value::Int(n) => (*n).into(),
        Value::UInt(n) => (*n).into(),
        Value::Float(f) => {
            serde_json::Number::from_f64(*f).map_or(serde_json::Value::Null, Into::into)
        }
        Value::Json(j) => j.clone(),
        Value::Array(items) => items.iter().map(json_value).collect(),
        other => other.to_string().into(),
    }
}

/// A JSON array of objects keyed by column name.
pub(crate) fn json<'a>(columns: &[Column], rows: impl Iterator<Item = &'a [Value]>) -> String {
    let rows: Vec<serde_json::Value> = rows
        .map(|row| {
            columns
                .iter()
                .zip(row)
                .map(|(c, v)| (c.name.to_string(), json_value(v)))
                .collect::<serde_json::Map<_, _>>()
                .into()
        })
        .collect();
    serde_json::to_string_pretty(&rows).unwrap_or_default()
}

/// One `INSERT` per row into `table`, using standard SQL quoting.
/// `INSERT` statements into `target` (already quoted for `dialect`).
pub(crate) fn inserts<'a>(
    dialect: Dialect,
    target: &str,
    columns: &[Column],
    rows: impl Iterator<Item = &'a [Value]>,
) -> String {
    let cols = columns
        .iter()
        .map(|c| dialect.ident(c.name.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    rows.map(|row| {
        let values = row
            .iter()
            .map(|v| dialect.literal(v))
            .collect::<Vec<_>>()
            .join(", ");
        format!("INSERT INTO {target} ({cols}) VALUES ({values});\n")
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use dialect::Dialect;
    use types::{Column, Value};

    fn data() -> (Vec<Column>, Vec<Vec<Value>>) {
        (
            vec![Column::new("id", "int8"), Column::new("note", "text")],
            vec![
                vec![Value::Int(1), Value::Text("a, \"b\"".into())],
                vec![Value::Int(2), Value::Null],
            ],
        )
    }

    #[test]
    fn csv_quotes_and_nulls() {
        let (cols, rows) = data();
        assert_eq!(
            super::csv(&cols, rows.iter().map(Vec::as_slice)),
            "id,note\n1,\"a, \"\"b\"\"\"\n2,\n"
        );
    }

    #[test]
    fn json_objects() {
        let (cols, rows) = data();
        let parsed: serde_json::Value =
            serde_json::from_str(&super::json(&cols, rows.iter().map(Vec::as_slice)))
                .expect("json");
        assert_eq!(parsed[1]["note"], serde_json::Value::Null);
        assert_eq!(parsed[0]["id"], 1);
    }

    #[test]
    fn insert_statements() {
        let (cols, rows) = data();
        assert_eq!(
            super::inserts(
                Dialect::Postgres,
                "result",
                &cols,
                rows.iter().map(Vec::as_slice)
            ),
            "INSERT INTO result (id, note) VALUES (1, 'a, \"b\"');\nINSERT INTO result (id, note) VALUES (2, NULL);\n"
        );
    }
}
