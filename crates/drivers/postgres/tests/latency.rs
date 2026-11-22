//! Times each driver call through a proxy that adds `RIFT_BENCH_DELAY_MS` (default 100) each
//! way, so round trips show up the way they do against a far-away server.
//!
//! `RIFT_PG_PORT=5435 cargo test -p postgres-driver --test latency -- --ignored --nocapture`

use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::future::BoxFuture;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use traits::{ConnectDeps, Connection, Spawner, StatementOutput};
use types::{ConnConfig, RowsRequest, SchemaName, Secret, TableRef};

struct TokioSpawner;

impl Spawner for TokioSpawner {
    fn spawn(&self, task: BoxFuture<'static, ()>) {
        tokio::spawn(task);
    }

    fn spawn_blocking(&self, task: Box<dyn FnOnce() + Send + 'static>) {
        tokio::task::spawn_blocking(task);
    }
}

/// Forwards bytes after `delay`, keeping order and letting data in flight overlap (a pipe, not
/// a stop-and-wait link).
async fn pipe(
    mut from: tokio::net::tcp::OwnedReadHalf,
    mut to: tokio::net::tcp::OwnedWriteHalf,
    delay: Duration,
) {
    let (tx, mut rx) = mpsc::unbounded_channel::<(Instant, Vec<u8>)>();
    tokio::spawn(async move {
        while let Some((due, bytes)) = rx.recv().await {
            tokio::time::sleep_until(due.into()).await;
            if to.write_all(&bytes).await.is_err() {
                return;
            }
        }
    });
    let mut buf = vec![0u8; 64 * 1024];
    while let Ok(n) = from.read(&mut buf).await {
        if n == 0
            || tx
                .send((Instant::now() + delay, buf[..n].to_vec()))
                .is_err()
        {
            return;
        }
    }
}

async fn proxy(target: String, delay: Duration) -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let port = listener.local_addr().expect("addr").port();
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let Ok(server) = TcpStream::connect(&target).await else {
                continue;
            };
            let (cr, cw) = client.into_split();
            let (sr, sw) = server.into_split();
            tokio::spawn(pipe(cr, sw, delay));
            tokio::spawn(pipe(sr, cw, delay));
        }
    });
    port
}

async fn time<T>(label: &str, f: impl std::future::Future<Output = T>) -> T {
    let started = Instant::now();
    let out = f.await;
    println!("{label:<28} {:>6} ms", started.elapsed().as_millis());
    out
}

async fn query(conn: &dyn Connection, sql: &str) {
    let out = conn
        .query()
        .expect("query")
        .execute(sql, CancellationToken::new())
        .await
        .expect("execute");
    if let StatementOutput::Rows(mut rows) = out {
        while let Some(page) = rows.pages.next().await {
            page.expect("page");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "benchmark; needs `make dev-up`"]
async fn round_trip_latency() {
    let pg_port = std::env::var("RIFT_PG_PORT").unwrap_or_else(|_| "5432".into());
    let delay_ms: u64 = std::env::var("RIFT_BENCH_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100);
    let port = proxy(
        format!("127.0.0.1:{pg_port}"),
        Duration::from_millis(delay_ms),
    )
    .await;
    println!("round trip = {} ms", delay_ms * 2);

    let mut config = ConnConfig::new("postgres")
        .with("host", "127.0.0.1")
        .with("port", port.to_string())
        .with("user", "postgres")
        .with("database", "shop_dev");
    config.password = Some(Secret::new("postgres"));
    let deps = ConnectDeps {
        spawner: Arc::new(TokioSpawner),
    };
    let conn = time(
        "connect",
        postgres_driver::factory().connect(&config, &deps),
    )
    .await
    .expect("connect");
    let conn = conn.as_ref();
    let browse = conn.browse().expect("browse");
    let public = SchemaName::new("public");
    let users = TableRef::new(Some(public.clone()), "users");
    let request = RowsRequest {
        limit: 100,
        ..RowsRequest::default()
    };

    for pass in ["cold", "warm"] {
        println!("-- {pass}");
        time("sidebar (schema+objects)", async {
            let (schema, objects, schemas) = futures::join!(
                browse.default_schema(),
                browse.objects(&public),
                browse.schemas()
            );
            (schema.unwrap(), objects.unwrap(), schemas.unwrap())
        })
        .await;
        time("table open (rows+count)", async {
            let (rows, count) = futures::join!(
                browse.rows(&users, &request),
                browse.count(&users, &request)
            );
            (rows.unwrap(), count.unwrap())
        })
        .await;
        time(
            "describe",
            conn.describe().expect("describe").describe(&users),
        )
        .await
        .expect("describe");
        time(
            "query SELECT count(id)",
            query(conn, "SELECT count(id) FROM users"),
        )
        .await;
        time(
            "query SELECT *",
            query(conn, "SELECT * FROM users LIMIT 100"),
        )
        .await;
    }
}
