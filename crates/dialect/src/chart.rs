//! Chart `GROUP BY` and column-stats builders, run in the database.

use std::collections::BTreeMap;

use types::{
    AggFn, AggregateRequest, ChartData, ChartPoint, ChartSource, ColumnName, ColumnStatsReport,
    Error, Series, TableRef, TimeBucket, Value, ValueCount,
};

use crate::Dialect;

fn bucket(dialect: Dialect, column: &str, bucket: TimeBucket) -> String {
    match dialect {
        Dialect::Postgres => {
            let unit = match bucket {
                TimeBucket::Minute => "minute",
                TimeBucket::Hour => "hour",
                TimeBucket::Week => "week",
                TimeBucket::Month => "month",
                _ => "day",
            };
            format!("date_trunc('{unit}', {column})")
        }
        Dialect::ClickHouse => {
            let f = match bucket {
                TimeBucket::Minute => "toStartOfMinute",
                TimeBucket::Hour => "toStartOfHour",
                TimeBucket::Week => "toStartOfWeek",
                TimeBucket::Month => "toStartOfMonth",
                _ => "toStartOfDay",
            };
            format!("{f}({column})")
        }
        Dialect::MySql => match bucket {
            TimeBucket::Minute => format!("DATE_FORMAT({column}, '%Y-%m-%d %H:%i:00')"),
            TimeBucket::Hour => format!("DATE_FORMAT({column}, '%Y-%m-%d %H:00:00')"),
            TimeBucket::Week => format!("DATE_SUB(DATE({column}), INTERVAL WEEKDAY({column}) DAY)"),
            TimeBucket::Month => format!("DATE_FORMAT({column}, '%Y-%m-01')"),
            _ => format!("DATE({column})"),
        },
        _ => match bucket {
            TimeBucket::Minute => format!("strftime('%Y-%m-%d %H:%M:00', {column})"),
            TimeBucket::Hour => format!("strftime('%Y-%m-%d %H:00:00', {column})"),
            TimeBucket::Week => format!("date({column}, 'weekday 0', '-6 days')"),
            TimeBucket::Month => format!("strftime('%Y-%m-01', {column})"),
            _ => format!("strftime('%Y-%m-%d', {column})"),
        },
    }
}

fn source(dialect: Dialect, source: &ChartSource) -> Result<String, Error> {
    Ok(match source {
        ChartSource::Table(t) => dialect.table(t),
        ChartSource::Query(sql) => {
            let sql = sql.trim().trim_end_matches(';');
            format!("({sql}) AS src")
        }
        _ => return Err(Error::unsupported("chart source")),
    })
}

/// The aggregation query for a chart. Columns: `x`, optional `s` (split), `y`. Asks for one
/// row more than `limit` so callers can tell the result was capped.
///
/// # Errors
/// [`Error::Invalid`] when the aggregate needs a Y column and none is set.
pub fn aggregate_sql(dialect: Dialect, request: &AggregateRequest) -> Result<String, Error> {
    let from = source(dialect, &request.source)?;
    let x_col = dialect.ident(request.x.as_str());
    let x = match request.bucket {
        Some(b) => bucket(dialect, &x_col, b),
        None => x_col.clone(),
    };
    let y_col = request.y.as_ref().map(|y| dialect.ident(y.as_str()));
    let y = match (request.agg, &y_col) {
        (AggFn::Count, _) => "count(*)".to_owned(),
        (AggFn::Sum, Some(y)) => format!("sum({y})"),
        (AggFn::Avg, Some(y)) => format!("avg({y})"),
        (AggFn::Min, Some(y)) => format!("min({y})"),
        (AggFn::Max, Some(y)) => format!("max({y})"),
        (AggFn::None, Some(y)) => y.clone(),
        _ => return Err(Error::invalid("choose a Y column for this aggregate")),
    };
    let split = request.split_by.as_ref().map(|s| dialect.ident(s.as_str()));
    let limit = request.limit.min(types::MAX_CHART_POINTS) as u64 + 1;
    let select_split = split
        .as_ref()
        .map_or(String::new(), |s| format!(", {s} AS s"));
    let mut sql =
        format!("SELECT {x} AS x{select_split}, {y} AS y FROM {from} WHERE {x_col} IS NOT NULL");
    if request.agg != AggFn::None {
        sql.push_str(" GROUP BY 1");
        if split.is_some() {
            sql.push_str(", 2");
        }
    }
    sql.push_str(" ORDER BY 1");
    sql.push_str(&dialect.limit(limit, 0));
    Ok(sql)
}

/// Turns aggregation rows (`x`, optional `s`, `y`) into series, enforcing the point cap.
#[must_use]
pub fn chart_data(rows: Vec<Vec<Value>>, request: &AggregateRequest) -> ChartData {
    let limit = request.limit.min(types::MAX_CHART_POINTS) as usize;
    let truncated = rows.len() > limit;
    let split = request.split_by.is_some();
    let default_name = match (&request.y, request.agg) {
        (_, AggFn::Count) => "count".to_owned(),
        (Some(y), _) => y.to_string(),
        (None, _) => "value".to_owned(),
    };
    let mut series: BTreeMap<String, Vec<ChartPoint>> = BTreeMap::new();
    for mut row in rows.into_iter().take(limit) {
        let y = row.pop().and_then(|v| v.as_f64());
        let name = if split {
            row.pop().map_or_else(String::new, |v| v.to_string())
        } else {
            default_name.clone()
        };
        let (Some(y), Some(x)) = (y, row.into_iter().next()) else {
            continue;
        };
        series.entry(name).or_default().push(ChartPoint { x, y });
    }
    ChartData {
        series: series
            .into_iter()
            .map(|(name, points)| Series { name, points })
            .collect(),
        truncated,
    }
}

/// Two queries: totals (`rows`, `non_null`, `distinct`), then the `top` most common values.
#[must_use]
pub fn column_stats_sql(
    dialect: Dialect,
    table: &TableRef,
    column: &ColumnName,
    top: u32,
) -> (String, String) {
    let t = dialect.table(table);
    let c = dialect.ident(column.as_str());
    (
        format!("SELECT count(*), count({c}), count(DISTINCT {c}) FROM {t}"),
        format!(
            "SELECT {c} AS v, count(*) AS n FROM {t} WHERE {c} IS NOT NULL GROUP BY {c} ORDER BY n DESC{}",
            dialect.limit(u64::from(top), 0)
        ),
    )
}

/// Builds the report from the two result sets of [`column_stats_sql`].
#[must_use]
pub fn column_stats(totals: &[Value], top: Vec<Vec<Value>>) -> ColumnStatsReport {
    let n = |i: usize| totals.get(i).and_then(Value::as_f64).unwrap_or(0.0) as u64;
    let rows = n(0);
    ColumnStatsReport {
        rows,
        nulls: rows.saturating_sub(n(1)),
        distinct: n(2),
        top: top
            .into_iter()
            .filter_map(|mut r| {
                let count = r.pop()?.as_f64()? as u64;
                Some(ValueCount {
                    value: r.pop()?,
                    count,
                })
            })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use types::{AggFn, AggregateRequest, ChartSource, SchemaName, TableRef, TimeBucket, Value};

    use super::{aggregate_sql, chart_data, column_stats_sql};
    use crate::Dialect;

    fn request() -> AggregateRequest {
        AggregateRequest {
            source: ChartSource::Table(TableRef::new(Some(SchemaName::new("analytics")), "events")),
            x: "ts".into(),
            bucket: Some(TimeBucket::Day),
            y: None,
            agg: AggFn::Count,
            split_by: Some("event".into()),
            limit: 10_000,
        }
    }

    #[test]
    fn chart_sql_per_dialect() {
        let all: Vec<String> = [
            Dialect::Postgres,
            Dialect::MySql,
            Dialect::Sqlite,
            Dialect::ClickHouse,
        ]
        .into_iter()
        .map(|d| aggregate_sql(d, &request()).expect("sql"))
        .collect();
        insta::assert_snapshot!(all.join("\n"));
    }

    #[test]
    fn sum_over_query_without_bucket() {
        let mut r = request();
        r.source = ChartSource::Query("SELECT * FROM orders;".into());
        r.bucket = None;
        r.split_by = None;
        r.agg = AggFn::Sum;
        r.y = Some("total".into());
        r.x = "status".into();
        insta::assert_snapshot!(aggregate_sql(Dialect::Postgres, &r).expect("sql"));
    }

    #[test]
    fn stats_sql() {
        let t = TableRef::new(Some(SchemaName::new("public")), "orders");
        let (a, b) = column_stats_sql(Dialect::Postgres, &t, &"status".into(), 6);
        insta::assert_snapshot!(format!("{a}\n{b}"));
    }

    #[test]
    fn caps_points_and_splits_series() {
        let mut r = request();
        r.limit = 2;
        let rows = vec![
            vec![
                Value::Text("d1".into()),
                Value::Text("click".into()),
                Value::Int(3),
            ],
            vec![
                Value::Text("d1".into()),
                Value::Text("view".into()),
                Value::Int(5),
            ],
            vec![
                Value::Text("d2".into()),
                Value::Text("click".into()),
                Value::Int(4),
            ],
        ];
        let data = chart_data(rows, &r);
        assert!(data.truncated);
        assert_eq!(data.series.len(), 2);
        assert_eq!(data.series[0].name, "click");
    }
}
