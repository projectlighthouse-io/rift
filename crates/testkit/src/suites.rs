//! Conformance suites, one per capability. Every driver returning `Some` for a capability must
//! pass its suite.

mod kv;
mod sql;
mod stores;

pub use kv::{
    KvFixture, key_expiry_suite, key_value_suite, kv_transactions_suite, memory_stats_suite,
    run_kv_suites,
};
pub use sql::{
    SqlFixture, aggregate_suite, backup_suite, browse_suite, column_stats_suite, describe_suite,
    edit_rows_suite, edit_schema_suite, explain_suite, query_suite, run_sql_suites,
    sql_transactions_suite,
};
pub use stores::{
    connection_store_suite, dashboard_store_suite, history_store_suite, secret_store_suite,
};
