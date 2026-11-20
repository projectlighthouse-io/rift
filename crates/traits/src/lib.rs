//! Contracts only: the `Connection` base trait, optional capability traits, `DriverFactory`,
//! and infrastructure traits (`Spawner`, `SecretStore`, `Tunnel`, stores).
//!
//! Capability traits are used as `dyn` so the UI can ask a connection at runtime what it
//! supports (a documented exception to "prefer generics").

mod analysis;
mod backup;
mod browse;
mod connection;
mod describe;
mod driver;
mod edit;
mod infra;
mod key_value;
mod query;
pub mod spawner;
mod stores;
mod table_ttl;
mod transactions;

pub use analysis::{Aggregate, ColumnStats, Explain, MemoryStats};
pub use backup::Backup;
pub use browse::Browse;
pub use connection::Connection;
pub use describe::Describe;
pub use driver::{ConnectDeps, DriverFactory};
pub use edit::{EditRows, EditSchema};
pub use infra::{SecretStore, Tunnel, TunnelHandle};
pub use key_value::{KeyExpiry, KeyValue};
pub use query::{Query, RowStream, StatementOutput};
pub use spawner::Spawner;
pub use stores::{ConnectionStore, DashboardStore, HistoryStore};
pub use table_ttl::TableTtl;
pub use transactions::Transactions;
