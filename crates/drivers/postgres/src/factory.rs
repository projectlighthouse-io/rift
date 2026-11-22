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

use crate::connection::{Describer, PgConnection};

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
                // Every round trip counts on a far-away server: only ping a connection that has
                // sat idle long enough for a proxy or firewall to have dropped it.
                .test_before_acquire(false)
                .before_acquire(|conn, meta| {
                    Box::pin(async move {
                        if meta.idle_for > Duration::from_secs(60) {
                            sqlx::Connection::ping(conn).await?;
                        }
                        Ok(true)
                    })
                })
                .connect_lazy_with(options.clone());
            // One connection up front, bounded by `acquire_timeout`: it reports errors and the
            // server version (sent at startup, so no query).
            let first = pool.acquire().await.map_err(|e| match e {
                sqlx::Error::Database(db) => Error::connect(db.message()),
                other => Error::connect(other.to_string()),
            })?;
            // Since 10, `server_version_num` is major * 10000 + minor.
            let major = first.server_version_num().unwrap_or(0) / 10_000;
            drop(first);
            let describer = Describer::new(&options, None);
            let (warm, warm_describer) = (pool.clone(), describer.clone());
            deps.spawner.spawn(
                async move {
                    // The rest open in the background, all at once instead of one after another.
                    // ponytail: 4 covers the widest fan-out (describe); the pool grows past it.
                    let pool = futures::future::join_all((0..4).map(|_| warm.acquire()));
                    let _ = futures::join!(pool, warm_describer.columns("SELECT 1".into()));
                }
                .boxed(),
            );
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
                describer,
                options,
                spawner: deps.spawner.clone(),
                info,
                password: config.password.as_ref().map(|p| p.expose().to_owned()),
            }) as Arc<dyn Connection>)
        }
        .boxed()
    }
}
