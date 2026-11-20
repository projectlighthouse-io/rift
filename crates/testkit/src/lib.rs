//! In-memory fake drivers, infrastructure fakes and per-capability conformance suites.

pub mod compare;
mod fake_kv;
mod fake_sql;
mod infra;
mod stores;
pub mod suites;

pub use fake_kv::{FakeKv, FakeKvFactory};
pub use fake_sql::{FakeSql, FakeSqlFactory};
pub use infra::{MemorySecretStore, ThreadSpawner};
pub use stores::{MemoryConnectionStore, MemoryDashboardStore, MemoryHistoryStore};
