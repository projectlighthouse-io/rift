//! SSH tunnels with russh: a local port forwarded through a bastion to the database.
//!
//! Host keys are checked against `~/.ssh/known_hosts`: a known host must match, an unknown
//! host is trusted on first use and recorded. Authentication uses a key file (the configured
//! one, else `~/.ssh/id_ed25519` or `~/.ssh/id_rsa`), with the stored secret as passphrase.

use std::path::PathBuf;
use std::sync::Arc;

use futures::FutureExt;
use futures::future::BoxFuture;
use russh::client::{self, Handle};
use russh::keys::known_hosts::{learn_known_hosts, learn_known_hosts_path};
use russh::keys::{
    PrivateKeyWithHashAlg, PublicKeyOrCertificate, check_known_hosts, check_known_hosts_path,
    load_secret_key,
};
use tokio::net::TcpListener;
use traits::{Tunnel, TunnelHandle};
use types::{Error, Secret, SshConfig};

struct Client {
    host: String,
    port: u16,
    known_hosts: Option<PathBuf>,
}

impl client::Handler for Client {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        // ponytail: host certificates are refused; add CA checking if bastions use them.
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return Ok(false);
        };
        let checked = match &self.known_hosts {
            Some(path) => check_known_hosts_path(&self.host, self.port, key, path),
            None => check_known_hosts(&self.host, self.port, key),
        };
        match checked {
            Ok(true) => Ok(true),
            // A recorded key for this host that does not match: possible man in the middle.
            Err(russh::keys::Error::KeyChanged { .. }) => Ok(false),
            _ => {
                let learned = match &self.known_hosts {
                    Some(path) => learn_known_hosts_path(&self.host, self.port, key, path),
                    None => learn_known_hosts(&self.host, self.port, key),
                };
                if let Err(e) = learned {
                    tracing::warn!("could not record the SSH host key: {e}");
                }
                Ok(true)
            }
        }
    }
}

fn ssh_error(e: impl std::fmt::Display) -> Error {
    Error::connect(format!("SSH: {e}"))
}

fn key_paths(config: &SshConfig) -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_default();
    match config.key_path.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(p) => vec![
            p.strip_prefix("~/")
                .map_or_else(|| PathBuf::from(p), |rest| home.join(rest)),
        ],
        None => vec![home.join(".ssh/id_ed25519"), home.join(".ssh/id_rsa")],
    }
}

/// An open tunnel; dropping it stops forwarding and closes the SSH session.
struct Open {
    port: u16,
    accept: tokio::task::JoinHandle<()>,
}

impl TunnelHandle for Open {
    fn local_port(&self) -> u16 {
        self.port
    }
}

impl Drop for Open {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

/// [`Tunnel`] over russh. Needs a tokio runtime.
#[derive(Default)]
pub struct RusshTunnel {
    /// Known-hosts file; `None` for `~/.ssh/known_hosts`.
    pub known_hosts: Option<PathBuf>,
}

impl RusshTunnel {
    async fn connect(
        &self,
        ssh: &SshConfig,
        secret: Option<Secret>,
    ) -> Result<Handle<Client>, Error> {
        let config = Arc::new(client::Config::default());
        let handler = Client {
            host: ssh.host.clone(),
            port: ssh.port,
            known_hosts: self.known_hosts.clone(),
        };
        let mut session = client::connect(config, (ssh.host.as_str(), ssh.port), handler)
            .await
            .map_err(ssh_error)?;
        let passphrase = secret.as_ref().map(Secret::expose);
        let mut last = "no key file found (set one in the SSH section)".to_owned();
        for path in key_paths(ssh).into_iter().filter(|p| p.is_file()) {
            let key = match load_secret_key(&path, passphrase) {
                Ok(key) => key,
                Err(e) => {
                    last = format!("{}: {e}", path.display());
                    continue;
                }
            };
            let hash = session
                .best_supported_rsa_hash()
                .await
                .map_err(ssh_error)?
                .flatten();
            let result = session
                .authenticate_publickey(&ssh.user, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
                .await
                .map_err(ssh_error)?;
            if result.success() {
                return Ok(session);
            }
            last = format!("{} was not accepted for {}", path.display(), ssh.user);
        }
        Err(ssh_error(format!("authentication failed: {last}")))
    }
}

impl Tunnel for RusshTunnel {
    fn open<'a>(
        &'a self,
        ssh: &'a SshConfig,
        secret: Option<Secret>,
        target_host: &'a str,
        target_port: u16,
    ) -> BoxFuture<'a, Result<Box<dyn TunnelHandle>, Error>> {
        async move {
            let session = Arc::new(self.connect(ssh, secret).await?);
            let listener = TcpListener::bind(("127.0.0.1", 0))
                .await
                .map_err(ssh_error)?;
            let port = listener.local_addr().map_err(ssh_error)?.port();
            let target = target_host.to_owned();
            let accept = tokio::spawn(async move {
                while let Ok((mut socket, peer)) = listener.accept().await {
                    let (session, target) = (session.clone(), target.clone());
                    tokio::spawn(async move {
                        let channel = session
                            .channel_open_direct_tcpip(
                                target,
                                u32::from(target_port),
                                peer.ip().to_string(),
                                u32::from(peer.port()),
                            )
                            .await;
                        match channel {
                            Ok(channel) => {
                                let mut stream = channel.into_stream();
                                let _ =
                                    tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
                            }
                            Err(e) => tracing::warn!("SSH forward failed: {e}"),
                        }
                    });
                }
            });
            Ok(Box::new(Open { port, accept }) as Box<dyn TunnelHandle>)
        }
        .boxed()
    }
}
