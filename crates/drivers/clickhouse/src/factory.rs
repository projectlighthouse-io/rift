//! Driver metadata and connecting.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use traits::{ConnectDeps, Connection, DriverFactory};
use types::{
    ConnConfig, ConnectionInfo, DriverInfo, Error, Family, FieldKind, FieldSpec, SafeMode, TlsMode,
};

use crate::client::Client;
use crate::connection::ClickHouseConnection;

const DEFAULT_PORT: u16 = 8123;

pub(crate) struct ClickHouseFactory {
    info: DriverInfo,
}

impl ClickHouseFactory {
    pub(crate) fn new() -> Self {
        Self {
            info: DriverInfo {
                id: "clickhouse".into(),
                name: "ClickHouse".into(),
                family: Family::Sql,
                fields: vec![
                    FieldSpec::new("host", "Host", FieldKind::Host)
                        .required()
                        .default_value("127.0.0.1"),
                    FieldSpec::new("port", "HTTP port", FieldKind::Port).default_value("8123"),
                    FieldSpec::new("user", "User", FieldKind::Text).default_value("default"),
                    FieldSpec::new("password", "Password", FieldKind::Password),
                    FieldSpec::new("database", "Database", FieldKind::Text)
                        .default_value("default"),
                ],
                url_schemes: vec!["clickhouse".into(), "http".into(), "https".into()],
                default_port: Some(DEFAULT_PORT),
            },
        }
    }
}

impl DriverFactory for ClickHouseFactory {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect<'a>(
        &'a self,
        config: &'a ConnConfig,
        deps: &'a ConnectDeps,
    ) -> BoxFuture<'a, Result<Arc<dyn Connection>, Error>> {
        async move {
            let host = config.get("host").unwrap_or("127.0.0.1");
            let port = config.get("port").unwrap_or("8123");
            let scheme = if config.tls == TlsMode::Require {
                "https"
            } else {
                "http"
            };
            let http = reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| Error::connect(e.to_string()))?;
            let client = Client {
                http,
                url: format!("{scheme}://{host}:{port}/"),
                user: config.get("user").unwrap_or("default").to_owned(),
                password: config
                    .password
                    .as_ref()
                    .map(|p| p.expose().to_owned())
                    .unwrap_or_default(),
                database: config.get("database").map(str::to_owned),
                read_only: config.safe_mode == SafeMode::ReadOnly,
            };
            let version = client
                .values("SELECT version()")
                .await
                .map_err(|e| match e {
                    Error::Query { message, .. } => Error::connect(message),
                    other => other,
                })?
                .into_iter()
                .next()
                .and_then(|r| r.into_iter().next())
                .map(|v| v.to_string())
                .unwrap_or_default();
            let short: String = version.split('.').take(2).collect::<Vec<_>>().join(".");
            Ok(Arc::new(ClickHouseConnection {
                client,
                spawner: deps.spawner.clone(),
                info: ConnectionInfo {
                    driver: "clickhouse".into(),
                    server_version: format!("ClickHouse {short}"),
                    address: format!("{host}:{port}"),
                    database: config.get("database").map(str::to_owned),
                },
            }) as Arc<dyn Connection>)
        }
        .boxed()
    }
}
