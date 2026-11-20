//! Driver metadata and connecting.

use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use redis::IntoConnectionInfo;
use traits::{ConnectDeps, Connection, DriverFactory};
use types::{ConnConfig, ConnectionInfo, DriverInfo, Error, Family, FieldKind, FieldSpec, TlsMode};

use crate::connection::{RedisConnection, map};

const DEFAULT_PORT: u16 = 6379;

pub(crate) struct RedisFactory {
    info: DriverInfo,
}

impl RedisFactory {
    pub(crate) fn new() -> Self {
        Self {
            info: DriverInfo {
                id: "redis".into(),
                name: "Redis".into(),
                family: Family::KeyValue,
                fields: vec![
                    FieldSpec::new("host", "Host", FieldKind::Host)
                        .required()
                        .default_value("127.0.0.1"),
                    FieldSpec::new("port", "Port", FieldKind::Port).default_value("6379"),
                    FieldSpec::new("user", "User", FieldKind::Text).placeholder("default"),
                    FieldSpec::new("password", "Password", FieldKind::Password),
                    FieldSpec::new("database", "Database", FieldKind::Text).default_value("0"),
                ],
                url_schemes: vec!["redis".into(), "rediss".into()],
                default_port: Some(DEFAULT_PORT),
            },
        }
    }
}

impl DriverFactory for RedisFactory {
    fn info(&self) -> &DriverInfo {
        &self.info
    }

    fn connect<'a>(
        &'a self,
        config: &'a ConnConfig,
        _deps: &'a ConnectDeps,
    ) -> BoxFuture<'a, Result<Arc<dyn Connection>, Error>> {
        async move {
            let host = config.get("host").unwrap_or("127.0.0.1").to_owned();
            let port: u16 = match config.get("port") {
                Some(p) => p
                    .parse()
                    .map_err(|_| Error::invalid(format!("bad port: {p}")))?,
                None => DEFAULT_PORT,
            };
            let db: i64 = config
                .get("database")
                .unwrap_or("0")
                .parse()
                .map_err(|_| Error::invalid("Database must be a number."))?;
            let addr = if config.tls == TlsMode::Require {
                redis::ConnectionAddr::TcpTls {
                    host: host.clone(),
                    port,
                    insecure: false,
                    tls_params: None,
                }
            } else {
                redis::ConnectionAddr::Tcp(host.clone(), port)
            };
            let mut settings = redis::RedisConnectionInfo::default().set_db(db);
            if let Some(user) = config.get("user") {
                settings = settings.set_username(user);
            }
            if let Some(password) = &config.password {
                settings = settings.set_password(password.expose());
            }
            let info = addr
                .into_connection_info()
                .map_err(|e| Error::connect(e.to_string()))?
                .set_redis_settings(settings);
            let client = redis::Client::open(info).map_err(|e| Error::connect(e.to_string()))?;
            let mut conn = client
                .get_connection_manager()
                .await
                .map_err(|e| Error::connect(e.to_string()))?;
            let server: String = redis::cmd("INFO")
                .arg("server")
                .query_async(&mut conn)
                .await
                .map_err(map)?;
            let version = server
                .lines()
                .find_map(|l| l.strip_prefix("redis_version:"))
                .unwrap_or("?")
                .trim();
            let short: String = version.split('.').take(2).collect::<Vec<_>>().join(".");
            Ok(Arc::new(RedisConnection {
                conn,
                info: ConnectionInfo {
                    driver: "redis".into(),
                    server_version: format!("Redis {short}"),
                    address: format!("{host}:{port}"),
                    database: Some(format!("db{db}")),
                },
            }) as Arc<dyn Connection>)
        }
        .boxed()
    }
}
