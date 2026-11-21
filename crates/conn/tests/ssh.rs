//! SSH tunnel against the Docker bastion (`make dev-up`): forwards to Postgres and checks the
//! server answers. Skipped unless `RIFT_SSH_PORT` is set (or required by `RIFT_REQUIRE_DOCKER`).

use conn::RusshTunnel;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use traits::Tunnel;
use types::SshConfig;

#[tokio::test]
async fn tunnel_reaches_postgres_through_the_bastion() {
    let Ok(port) = std::env::var("RIFT_SSH_PORT") else {
        assert!(
            std::env::var("RIFT_REQUIRE_DOCKER").is_err(),
            "RIFT_SSH_PORT is not set"
        );
        eprintln!("skipping: RIFT_SSH_PORT is not set");
        return;
    };
    let key =
        std::env::var("RIFT_SSH_KEY").unwrap_or_else(|_| "../../dev/ssh/keys/id_ed25519".into());
    let target = std::env::var("RIFT_SSH_TARGET").unwrap_or_else(|_| "postgres:5432".into());
    let (target_host, target_port) = target.rsplit_once(':').expect("host:port");
    let known_hosts = std::env::temp_dir().join(format!("rift-known-hosts-{}", std::process::id()));
    let tunnel = RusshTunnel {
        known_hosts: Some(known_hosts.clone()),
    };
    let ssh = SshConfig {
        host: "127.0.0.1".into(),
        port: port.parse().expect("port"),
        user: "rift".into(),
        key_path: Some(key),
    };
    let handle = tunnel
        .open(
            &ssh,
            None,
            target_host,
            target_port.parse().expect("target port"),
        )
        .await
        .expect("open tunnel");
    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", handle.local_port()))
        .await
        .expect("connect");
    // Postgres SSLRequest: length 8, code 80877103; the server answers 'S' or 'N'.
    socket
        .write_all(&[0, 0, 0, 8, 4, 210, 22, 47])
        .await
        .expect("write");
    let mut answer = [0u8; 1];
    socket.read_exact(&mut answer).await.expect("read");
    assert!(matches!(answer[0], b'S' | b'N'), "answer {answer:?}");
    // The host key was recorded, so a second tunnel is trusted without learning again.
    assert!(
        std::fs::read_to_string(&known_hosts)
            .expect("known_hosts")
            .contains("[127.0.0.1]:")
    );
    let _ = std::fs::remove_file(known_hosts);
}
