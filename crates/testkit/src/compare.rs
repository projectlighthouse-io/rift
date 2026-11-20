//! Value comparison shared by the fakes and the suites.

use std::cmp::Ordering;

use types::{Filter, FilterOp, Value};

/// Orders two values: numerically when both are numbers, else by text. `NULL` sorts first.
#[must_use]
pub fn cmp(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Null, Value::Null) => Ordering::Equal,
        (Value::Null, _) => Ordering::Less,
        (_, Value::Null) => Ordering::Greater,
        _ => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
            _ => a.to_string().cmp(&b.to_string()),
        },
    }
}

/// Equality that ignores the integer/float/decimal representation.
#[must_use]
pub fn same(a: &Value, b: &Value) -> bool {
    a.is_null() == b.is_null() && cmp(a, b) == Ordering::Equal
}

/// Whether `value` passes `filter`.
#[must_use]
pub fn matches(value: &Value, filter: &Filter) -> bool {
    let ord = cmp(value, &filter.value);
    match filter.op {
        FilterOp::IsNull => value.is_null(),
        FilterOp::IsNotNull => !value.is_null(),
        _ if value.is_null() => false,
        FilterOp::Eq => ord == Ordering::Equal,
        FilterOp::NotEq => ord != Ordering::Equal,
        FilterOp::Lt => ord == Ordering::Less,
        FilterOp::Lte => ord != Ordering::Greater,
        FilterOp::Gt => ord == Ordering::Greater,
        FilterOp::Gte => ord != Ordering::Less,
        FilterOp::Contains => value
            .to_string()
            .to_lowercase()
            .contains(&filter.value.to_string().to_lowercase()),
        _ => false,
    }
}
