//! Driver metadata and connecting.

use std::path::PathBuf;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use traits::{ConnectDeps, Connection, DriverFactory};
use types::{
    ConnConfig, ConnectionInfo, DriverInfo, Error, Family, FieldKind, FieldSpec, SafeMode,
};

use crate::connection::{SqliteConnection, open};

pub(crate) struct SqliteFactory {
    info: DriverInfo,
}

impl SqliteFactory {
    pub(crate) fn new() -> Self {
        Self {
            info: DriverInfo {
                id: "sqlite".into(),
                name: "SQLite".into(),
                family: Family::Sql,
                fields: vec![
                    FieldSpec::new("path", "Database file", FieldKind::File)
                        .required()
                        .placeholder("~/code/scratch.sqlite"),
                ],
                url_schemes: vec!["sqlite".into(), "file".into()],
                default_port: None,
            },
        }
    }
}

/// Expands a leading `~/`.
fn expand(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

impl DriverFactory for SqliteFactory {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect<'a>(
        &'a self,
        config: &'a ConnConfig,
        deps: &'a ConnectDeps,
    ) -> BoxFuture<'a, Result<Arc<dyn Connection>, Error>> {
        async move {
            let raw = config
                .get("path")
                .ok_or_else(|| Error::invalid("Database file is required."))?;
            let path = expand(raw);
            let read_only = config.safe_mode == SafeMode::ReadOnly;
            let check = path.clone();
            let version = traits::spawner::run_blocking(&*deps.spawner, move || {
                let conn = open(&check, read_only)?;
                conn.query_row("SELECT sqlite_version()", [], |r| r.get::<_, String>(0))
                    .map_err(|e| Error::connect(e.to_string()))
            })
            .await??;
            let info = ConnectionInfo {
                driver: "sqlite".into(),
                server_version: format!("SQLite {version}"),
                address: raw.to_owned(),
                database: path.file_name().map(|n| n.to_string_lossy().into_owned()),
            };
            Ok(Arc::new(SqliteConnection {
                path,
                read_only,
                spawner: deps.spawner.clone(),
                info,
            }) as Arc<dyn Connection>)
        }
        .boxed()
    }
}
