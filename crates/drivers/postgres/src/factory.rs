//! Driver metadata and connecting.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use sqlx::postgres::{PgConnectOptions, PgPoolOptions, PgSslMode};
use traits::{ConnectDeps, Connection, DriverFactory};
use types::{
    ConnConfig, ConnectionInfo, DriverInfo, Error, Family, FieldKind, FieldSpec, SafeMode, TlsMode,
};

use crate::connection::PgConnection;
use crate::error;

const DEFAULT_PORT: u16 = 5432;

pub(crate) struct PgFactory {
    info: DriverInfo,
}

impl PgFactory {
    pub(crate) fn new() -> Self {
        Self {
            info: DriverInfo {
                id: "postgres".into(),
                name: "PostgreSQL".into(),
                family: Family::Sql,
                fields: vec![
                    FieldSpec::new("host", "Host", FieldKind::Host)
                        .required()
                        .default_value("127.0.0.1"),
                    FieldSpec::new("port", "Port", FieldKind::Port).default_value("5432"),
                    FieldSpec::new("user", "User", FieldKind::Text).default_value("postgres"),
                    FieldSpec::new("password", "Password", FieldKind::Password),
                    FieldSpec::new("database", "Database", FieldKind::Text)
                        .default_value("postgres"),
                ],
                url_schemes: vec!["postgres".into(), "postgresql".into()],
                default_port: Some(DEFAULT_PORT),
            },
        }
    }
}

fn options(config: &ConnConfig) -> Result<PgConnectOptions, Error> {
    let port = match config.get("port") {
        Some(p) => p
            .parse()
            .map_err(|_| Error::invalid(format!("bad port: {p}")))?,
        None => DEFAULT_PORT,
    };
    let mut options = PgConnectOptions::new()
        .host(config.get("host").unwrap_or("127.0.0.1"))
        .port(port)
        .username(config.get("user").unwrap_or("postgres"))
        .database(config.get("database").unwrap_or("postgres"))
        .application_name("rift")
        .ssl_mode(match config.tls {
            TlsMode::Require => PgSslMode::Require,
            TlsMode::Prefer => PgSslMode::Prefer,
            _ => PgSslMode::Disable,
        });
    if let Some(password) = &config.password {
        options = options.password(password.expose());
    }
    if config.safe_mode == SafeMode::ReadOnly {
        options = options.options([("default_transaction_read_only", "on")]);
    }
    Ok(options)
}

impl DriverFactory for PgFactory {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect<'a>(
        &'a self,
        config: &'a ConnConfig,
        deps: &'a ConnectDeps,
    ) -> BoxFuture<'a, Result<Arc<dyn Connection>, Error>> {
        async move {
            let options = options(config)?;
            let pool = PgPoolOptions::new()
                .max_connections(8)
                .acquire_timeout(Duration::from_secs(10))
                .connect_with(options.clone())
                .await
                .map_err(|e| match e {
                    sqlx::Error::Database(db) => Error::connect(db.message()),
                    other => Error::connect(other.to_string()),
                })?;
            let version: String = sqlx::query_scalar("SHOW server_version")
                .fetch_one(&pool)
                .await
                .map_err(error::map)?;
            let major = version
                .split_whitespace()
                .next()
                .unwrap_or(&version)
                .to_owned();
            let info = ConnectionInfo {
                driver: "postgres".into(),
                server_version: format!("PostgreSQL {major}"),
                address: format!(
                    "{}:{}",
                    config.get("host").unwrap_or("127.0.0.1"),
                    config.get("port").unwrap_or("5432")
                ),
                database: config.get("database").map(str::to_owned),
            };
            Ok(Arc::new(PgConnection {
                pool,
                options,
                spawner: deps.spawner.clone(),
                info,
            }) as Arc<dyn Connection>)
        }
        .boxed()
    }
}
