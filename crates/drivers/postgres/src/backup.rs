//! Backup and restore through `pg_dump` and `psql`: a plain SQL script with `--clean`, so
//! restoring replaces what the dump contains.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use futures::FutureExt;
use futures::future::BoxFuture;
use tokio_util::sync::CancellationToken;
use traits::{Backup, spawner};
use types::{BackupReport, Error};

use crate::connection::PgConnection;

/// Where client tools live when the app was not started from a shell.
const TOOL_DIRS: [&str; 4] = [
    "/opt/homebrew/opt/libpq/bin",
    "/usr/local/opt/libpq/bin",
    "/Applications/Postgres.app/Contents/Versions/latest/bin",
    "/usr/lib/postgresql/16/bin",
];

fn tool(name: &str) -> Result<PathBuf, Error> {
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(TOOL_DIRS.iter().map(PathBuf::from))
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
        .ok_or_else(|| Error::MissingDependency {
            message: format!(
                "{name} was not found. Install the PostgreSQL client tools (macOS: brew install \
                 libpq; Debian/Ubuntu: apt install postgresql-client)."
            ),
        })
}

/// Settings newer `pg_dump` versions write that older servers reject.
// ponytail: one known line; grow the list if other version skews show up.
const SKIP_LINES: [&str; 1] = ["SET transaction_timeout = 0;"];

/// Streams `src` to `stdin`, leaving out [`SKIP_LINES`].
fn feed(src: &Path, mut stdin: std::process::ChildStdin) -> std::io::Result<()> {
    let reader = BufReader::new(std::fs::File::open(src)?);
    for line in reader.split(b'\n') {
        let line = line?;
        if SKIP_LINES
            .iter()
            .any(|skip| line.as_slice() == skip.as_bytes())
        {
            continue;
        }
        stdin.write_all(&line)?;
        stdin.write_all(b"\n")?;
    }
    Ok(())
}

/// Runs `command` to completion, feeding it `input` if given, killing it when `cancel` fires.
fn run(
    mut command: Command,
    input: Option<PathBuf>,
    cancel: &CancellationToken,
) -> Result<(), Error> {
    let mut child = command
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| Error::query(format!("starting {:?}: {e}", command.get_program())))?;
    let feeder = match (input, child.stdin.take()) {
        (Some(src), Some(stdin)) => Some(std::thread::spawn(move || feed(&src, stdin))),
        _ => None,
    };
    let result = wait(&mut child, cancel);
    if let Some(feeder) = feeder {
        let _ = feeder.join();
    }
    result
}

fn wait(child: &mut std::process::Child, cancel: &CancellationToken) -> Result<(), Error> {
    loop {
        if cancel.is_cancelled() {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::Cancelled);
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                let last: Vec<&str> = stderr.lines().rev().take(3).collect();
                let message = last.into_iter().rev().collect::<Vec<_>>().join("\n");
                return Err(Error::query(if message.is_empty() {
                    format!("exited with {status}")
                } else {
                    message
                }));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(Error::query(e.to_string())),
        }
    }
}

impl PgConnection {
    fn command(&self, tool_name: &str) -> Result<Command, Error> {
        let mut command = Command::new(tool(tool_name)?);
        let o = &self.options;
        command
            .arg("--host")
            .arg(o.get_host())
            .arg("--port")
            .arg(o.get_port().to_string())
            .arg("--username")
            .arg(o.get_username())
            .arg("--dbname")
            .arg(o.get_database().unwrap_or("postgres"));
        if let Some(password) = &self.password {
            command.env("PGPASSWORD", password);
        }
        Ok(command)
    }
}

impl Backup for PgConnection {
    fn backup<'a>(
        &'a self,
        dest: &'a Path,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<BackupReport, Error>> {
        async move {
            let mut command = self.command("pg_dump")?;
            command
                .args(["--clean", "--if-exists", "--no-owner", "--file"])
                .arg(dest);
            spawner::run_blocking(&*self.spawner, move || run(command, None, &cancel)).await??;
            let bytes = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
            Ok(BackupReport {
                path: dest.to_path_buf(),
                bytes,
            })
        }
        .boxed()
    }

    fn restore<'a>(
        &'a self,
        src: &'a Path,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Result<(), Error>> {
        async move {
            let mut command = self.command("psql")?;
            command.args([
                "--quiet",
                "--single-transaction",
                "--set",
                "ON_ERROR_STOP=1",
                "--file",
                "-",
            ]);
            let src = src.to_path_buf();
            spawner::run_blocking(&*self.spawner, move || run(command, Some(src), &cancel)).await?
        }
        .boxed()
    }
}
