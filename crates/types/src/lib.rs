//! Plain data shared across Rift: values, columns, pages, table refs, connection config,
//! driver info and errors. Depends on no other workspace crate.

mod chart;
mod column;
mod config;
mod conn_info;
mod describe;
mod driver_info;
mod edit;
mod error;
mod ids;
mod kv;
mod misc;
mod names;
mod page;
mod stats;
mod store;
mod table;
mod ttl;
mod value;

pub use chart::{
    AggFn, AggregateRequest, ChartData, ChartKind, ChartPoint, ChartSource, ChartSpec,
    MAX_CHART_POINTS, Series, TimeBucket,
};
pub use column::{Column, ColumnRef};
pub use config::{ConnConfig, SafeMode, Secret, SshConfig, TlsMode};
pub use conn_info::ConnectionInfo;
pub use describe::{ColumnDef, ForeignKeyDef, IndexDef, SchemaChange, TableDescription};
pub use driver_info::{DriverInfo, Family, FieldKind, FieldSpec};
pub use edit::{ColumnValue, RowEdit, RowEditKind};
pub use error::Error;
pub use ids::{CardId, ConnectionId, DashboardId};
pub use kv::{
    HashField, KeyDirectory, KeyEntry, KeyKind, KeyMatch, KeyPage, KeyScan, KvValue, ScanCursor,
    ScoredMember, StreamEntry, TxOp,
};
pub use misc::{BackupReport, ExplainPlan};
pub use names::{ColumnName, DriverId, FieldKey, IndexName, KeyBytes, SchemaName, TableName};
pub use page::{Filter, FilterOp, PAGE_SIZE, Page, Row, RowsRequest, Sort, SortDir};
pub use stats::{ColumnStatsReport, KindUsage, MemoryReport, PrefixUsage, ValueCount};
pub use store::{AlertOp, AlertRule, ChartCard, Dashboard, HistoryEntry, SavedConnection, Tag};
pub use table::{DbObject, ObjectKind, TableRef};
pub use ttl::TtlRule;
pub use value::Value;
