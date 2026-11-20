//! Driver metadata and connecting.

use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use traits::{ConnectDeps, Connection, DriverFactory, spawner};
use types::{ConnConfig, ConnectionInfo, DriverInfo, Error, Family, FieldKind, FieldSpec};

use crate::connection::{self, FdbConnection, TIMEOUT_MS};

pub(crate) struct FdbFactory {
    info: DriverInfo,
}

impl FdbFactory {
    pub(crate) fn new() -> Self {
        Self {
            info: DriverInfo {
                id: "foundationdb".into(),
                name: "FoundationDB".into(),
                family: Family::KeyValue,
                fields: vec![
                    FieldSpec::new("cluster", "Cluster", FieldKind::Text).placeholder(
                        "docker:docker@127.0.0.1:4500 (empty for the default cluster file)",
                    ),
                    FieldSpec::new("cluster_file", "Cluster file", FieldKind::File)
                        .placeholder("/usr/local/etc/foundationdb/fdb.cluster"),
                ],
                url_schemes: Vec::new(),
                default_port: Some(4500),
            },
        }
    }
}

/// A cluster file for `description:id@host:port`, or the given path.
fn cluster_file(config: &ConnConfig) -> Result<Option<String>, Error> {
    if let Some(conn) = config.get("cluster").filter(|c| !c.trim().is_empty()) {
        if !conn.contains('@') {
            return Err(Error::invalid(
                "cluster must look like description:id@host:port",
            ));
        }
        let path = std::env::temp_dir().join(format!(
            "rift-{}.cluster",
            conn.replace(['/', ':', '@'], "_")
        ));
        std::fs::write(&path, conn.trim())
            .map_err(|e| Error::connect(format!("writing cluster file: {e}")))?;
        return Ok(Some(path.display().to_string()));
    }
    Ok(config
        .get("cluster_file")
        .filter(|p| !p.trim().is_empty())
        .map(|p| match p.strip_prefix("~/") {
            Some(rest) => {
                std::env::var("HOME").map_or_else(|_| p.to_owned(), |h| format!("{h}/{rest}"))
            }
            None => p.to_owned(),
        }))
}

impl DriverFactory for FdbFactory {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect<'a>(
        &'a self,
        config: &'a ConnConfig,
        deps: &'a ConnectDeps,
    ) -> BoxFuture<'a, Result<Arc<dyn Connection>, Error>> {
        async move {
            let path = cluster_file(config)?;
            let address = config.get("cluster").filter(|c| !c.is_empty()).map_or_else(
                || {
                    path.clone()
                        .unwrap_or_else(|| "default cluster file".into())
                },
                |c| c.rsplit('@').next().unwrap_or(c).to_owned(),
            );
            let db = spawner::run_blocking(&*deps.spawner, move || fdb_ffi::open(path.as_deref()))
                .await?
                .map_err(connection::map)?;
            let db = Arc::new(db);
            // One read proves the cluster is reachable and tells us its version.
            let probe = db.clone();
            let status = spawner::run_blocking(&*deps.spawner, move || {
                probe.run(TIMEOUT_MS, connection::status)
            })
            .await?
            .map_err(|e| match connection::map(e) {
                Error::Timeout => Error::connect("cluster did not answer within 5 s"),
                other => other,
            })?;
            let version = status["cluster"]["processes"]
                .as_object()
                .and_then(|p| p.values().next())
                .and_then(|p| p["version"].as_str())
                .map(|v| v.split('.').take(2).collect::<Vec<_>>().join("."))
                .unwrap_or_default();
            Ok(Arc::new(FdbConnection {
                db,
                spawner: deps.spawner.clone(),
                info: ConnectionInfo {
                    driver: "foundationdb".into(),
                    server_version: format!("FoundationDB {version}").trim().to_owned(),
                    address,
                    database: None,
                },
            }) as Arc<dyn Connection>)
        }
        .boxed()
    }
}
