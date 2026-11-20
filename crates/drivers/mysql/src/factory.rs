//! Driver metadata and connecting.

use std::sync::Arc;
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use sqlx::mysql::{MySqlConnectOptions, MySqlPoolOptions, MySqlSslMode};
use traits::{ConnectDeps, Connection, DriverFactory};
use types::{
    ConnConfig, ConnectionInfo, DriverInfo, Error, Family, FieldKind, FieldSpec, SafeMode, TlsMode,
};

use crate::connection::MySqlConnection;
use crate::error;

const DEFAULT_PORT: u16 = 3306;

pub(crate) struct MySqlFactory {
    info: DriverInfo,
}

impl MySqlFactory {
    pub(crate) fn new() -> Self {
        Self {
            info: DriverInfo {
                id: "mysql".into(),
                name: "MySQL".into(),
                family: Family::Sql,
                fields: vec![
                    FieldSpec::new("host", "Host", FieldKind::Host)
                        .required()
                        .default_value("127.0.0.1"),
                    FieldSpec::new("port", "Port", FieldKind::Port).default_value("3306"),
                    FieldSpec::new("user", "User", FieldKind::Text).default_value("root"),
                    FieldSpec::new("password", "Password", FieldKind::Password),
                    FieldSpec::new("database", "Database", FieldKind::Text),
                ],
                url_schemes: vec!["mysql".into(), "mariadb".into()],
                default_port: Some(DEFAULT_PORT),
            },
        }
    }
}

fn options(config: &ConnConfig) -> Result<MySqlConnectOptions, Error> {
    let port = match config.get("port") {
        Some(p) => p
            .parse()
            .map_err(|_| Error::invalid(format!("bad port: {p}")))?,
        None => DEFAULT_PORT,
    };
    let mut options = MySqlConnectOptions::new()
        .host(config.get("host").unwrap_or("127.0.0.1"))
        .port(port)
        .username(config.get("user").unwrap_or("root"))
        .statement_cache_capacity(0)
        .ssl_mode(match config.tls {
            TlsMode::Require => MySqlSslMode::Required,
            TlsMode::Prefer => MySqlSslMode::Preferred,
            _ => MySqlSslMode::Disabled,
        });
    if let Some(db) = config.get("database") {
        options = options.database(db);
    }
    if let Some(password) = &config.password {
        options = options.password(password.expose());
    }
    Ok(options)
}

impl DriverFactory for MySqlFactory {
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
            let read_only = config.safe_mode == SafeMode::ReadOnly;
            let pool = MySqlPoolOptions::new()
                .max_connections(8)
                .acquire_timeout(Duration::from_secs(10))
                .after_connect(move |conn, _| {
                    Box::pin(async move {
                        if read_only {
                            sqlx::raw_sql("SET SESSION TRANSACTION READ ONLY")
                                .execute(conn)
                                .await?;
                        }
                        Ok(())
                    })
                })
                .connect_with(options.clone())
                .await
                .map_err(|e| match e {
                    sqlx::Error::Database(db) => Error::connect(db.message()),
                    other => Error::connect(other.to_string()),
                })?;
            let version: String = sqlx::query_scalar("SELECT VERSION()")
                .fetch_one(&pool)
                .await
                .map_err(error::map)?;
            let short: String = version.split('.').take(2).collect::<Vec<_>>().join(".");
            let info = ConnectionInfo {
                driver: "mysql".into(),
                server_version: format!("MySQL {short}"),
                address: format!(
                    "{}:{}",
                    config.get("host").unwrap_or("127.0.0.1"),
                    config.get("port").unwrap_or("3306")
                ),
                database: config.get("database").map(str::to_owned),
            };
            Ok(Arc::new(MySqlConnection {
                pool,
                options,
                spawner: deps.spawner.clone(),
                info,
            }) as Arc<dyn Connection>)
        }
        .boxed()
    }
}
