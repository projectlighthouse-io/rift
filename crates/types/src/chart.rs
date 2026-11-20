//! Chart and aggregation specs.

use serde::{Deserialize, Serialize};

use crate::{ColumnName, TableRef, Value};

/// Chart shape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ChartKind {
    /// Line.
    #[default]
    Line,
    /// Vertical bars.
    Bar,
    /// Filled line.
    Area,
    /// Points only.
    Scatter,
    /// Single number with sparkline.
    Number,
    /// Horizontal bars.
    BarList,
}

/// Aggregate applied to Y.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum AggFn {
    /// `count(*)`.
    #[default]
    Count,
    /// `sum(y)`.
    Sum,
    /// `avg(y)`.
    Avg,
    /// `min(y)`.
    Min,
    /// `max(y)`.
    Max,
    /// No aggregation; raw points.
    None,
}

/// Time bucket for date X axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum TimeBucket {
    /// Minute.
    Minute,
    /// Hour.
    Hour,
    /// Day.
    Day,
    /// Week.
    Week,
    /// Month.
    Month,
}

/// Where chart rows come from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub enum ChartSource {
    /// A table.
    Table(TableRef),
    /// Any `SELECT`, wrapped as a subquery.
    Query(String),
}

/// A `GROUP BY` the database runs for a chart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateRequest {
    /// Rows.
    pub source: ChartSource,
    /// X column.
    pub x: ColumnName,
    /// Bucket X when it is a timestamp.
    pub bucket: Option<TimeBucket>,
    /// Y column; ignored for `Count`.
    pub y: Option<ColumnName>,
    /// Aggregate.
    pub agg: AggFn,
    /// One series per distinct value of this column.
    pub split_by: Option<ColumnName>,
    /// Max points returned.
    pub limit: u32,
}

/// Points are capped at this many.
pub const MAX_CHART_POINTS: u32 = 10_000;

/// One point.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartPoint {
    /// X value.
    pub x: Value,
    /// Y value.
    pub y: f64,
}

/// One series.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    /// Split value label, or the Y name.
    pub name: String,
    /// Points ordered by X.
    pub points: Vec<ChartPoint>,
}

/// Aggregation output.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ChartData {
    /// Series.
    pub series: Vec<Series>,
    /// True when the point cap cut results.
    pub truncated: bool,
}

/// Saved chart settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChartSpec {
    /// Shape.
    pub kind: ChartKind,
    /// Aggregation.
    pub request: AggregateRequest,
    /// Draw points on lines.
    pub show_points: bool,
}
