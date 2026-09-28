// SPDX-License-Identifier: GPL-2.0-or-later
//! Local control socket between the rpcd plugin and the daemon.
//!
//! The socket lives in `/var/run/mqtt-metrics/` with mode 0600, so only root
//! (rpcd) can use it. It accepts three commands: `reload` (re-read
//! configuration and password), `republish` and `purge`. None of them is
//! reachable from MQTT.

use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::time::Duration;

use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Reload,
    Republish,
    Purge,
}

impl Command {
    pub fn parse(name: &str) -> Option<Command> {
        match name {
            "reload" => Some(Command::Reload),
            "republish" => Some(Command::Republish),
            "purge" => Some(Command::Purge),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Command::Reload => "reload",
            Command::Republish => "republish",
            Command::Purge => "purge",
        }
    }
}

pub struct Server {
    listener: UnixListener,
}

pub struct Request {
    pub command: Command,
    stream: UnixStream,
}

impl Request {
    pub fn reply(mut self, ok: bool, message: &str) {
        let line = json!({ "ok": ok, "message": message }).to_string();
        let _ = writeln!(self.stream, "{line}");
    }
}

impl Server {
    pub fn bind(path: &Path) -> io::Result<Server> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        let listener = UnixListener::bind(path)?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Server { listener })
    }

    /// Accept pending connections and read one command from each.
    pub fn poll(&self) -> Vec<Request> {
        let mut out = Vec::new();
        loop {
            match self.listener.accept() {
                Ok((stream, _)) => {
                    if let Some(req) = read_request(stream) {
                        out.push(req);
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
        out
    }
}

fn read_request(stream: UnixStream) -> Option<Request> {
    stream.set_nonblocking(false).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .ok()?;
    let mut line = String::new();
    let mut reader = BufReader::new(io::Read::take(stream.try_clone().ok()?, 4096));
    reader.read_line(&mut line).ok()?;
    let v: Value = serde_json::from_str(line.trim()).ok()?;
    let command = Command::parse(v.get("cmd")?.as_str()?)?;
    Some(Request { command, stream })
}

/// Send a command to the daemon and wait for its reply.
pub fn send(path: &Path, command: Command, timeout: Duration) -> io::Result<(bool, String)> {
    let mut stream = UnixStream::connect(path)?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    writeln!(stream, "{}", json!({ "cmd": command.name() }))?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    let v: Value = serde_json::from_str(line.trim())
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    Ok((
        v.get("ok").and_then(Value::as_bool).unwrap_or(false),
        v.get("message")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_round_trip() {
        let dir = std::env::temp_dir().join(format!("mqtt-metrics-ctl-{}", std::process::id()));
        let path = dir.join("control.sock");
        let server = Server::bind(&path).unwrap();
        let client_path = path.clone();
        let handle = std::thread::spawn(move || {
            send(&client_path, Command::Republish, Duration::from_secs(5))
        });
        let mut requests = Vec::new();
        for _ in 0..200 {
            requests = server.poll();
            if !requests.is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(requests.len(), 1);
        let req = requests.pop().unwrap();
        assert_eq!(req.command, Command::Republish);
        req.reply(true, "done");
        assert_eq!(handle.join().unwrap().unwrap(), (true, "done".to_string()));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
