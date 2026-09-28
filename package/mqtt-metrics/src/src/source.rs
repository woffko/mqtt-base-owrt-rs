// SPDX-License-Identifier: GPL-2.0-or-later
//! Access to router data sources.
//!
//! Collectors read everything through [`Source`], so the same code runs on a
//! router ([`SystemSource`]) and against sanitized captures in tests and in
//! `--dump --fixtures` ([`FixtureSource`]).

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::Value;

use crate::exec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SourceError {
    /// The object, file or program does not exist on this router.
    Missing(String),
    /// It exists but the call failed or returned unusable data.
    Failed(String),
}

impl fmt::Display for SourceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SourceError::Missing(what) => write!(f, "{what} is not available"),
            SourceError::Failed(why) => f.write_str(why),
        }
    }
}

pub type SourceResult<T> = Result<T, SourceError>;

pub trait Source {
    /// `ubus call <object> <method> [<args>]`, parsed as JSON.
    fn ubus(&self, object: &str, method: &str, args: Option<&str>) -> SourceResult<Value>;
    /// Read a whole (small) file.
    fn read(&self, path: &str) -> SourceResult<String>;
    /// Names of the entries of a directory, sorted.
    fn list_dir(&self, path: &str) -> SourceResult<Vec<String>>;
    /// Run a helper program and return its stdout.
    fn command(&self, program: &str, args: &[&str]) -> SourceResult<String>;
    /// Current Unix time in seconds.
    fn now(&self) -> u64;
}

const UBUS_TIMEOUT_S: u64 = 5;
const MAX_OUTPUT: usize = 4 * 1024 * 1024;
/// `ubus` exit status for UBUS_STATUS_NOT_FOUND.
const UBUS_NOT_FOUND: i32 = 4;

#[derive(Debug, Default)]
pub struct SystemSource;

impl Source for SystemSource {
    fn ubus(&self, object: &str, method: &str, args: Option<&str>) -> SourceResult<Value> {
        let timeout = UBUS_TIMEOUT_S.to_string();
        let mut argv = vec!["-t", timeout.as_str(), "call", object, method];
        if let Some(args) = args {
            argv.push(args);
        }
        let out = exec::run(
            "ubus",
            &argv,
            None,
            Duration::from_secs(UBUS_TIMEOUT_S + 2),
            MAX_OUTPUT,
        )
        .map_err(|e| SourceError::Failed(format!("ubus call {object} {method}: {e}")))?;
        if out.code == Some(UBUS_NOT_FOUND) {
            return Err(SourceError::Missing(format!("ubus object {object}")));
        }
        if !out.success() {
            return Err(SourceError::Failed(format!(
                "ubus call {object} {method} exited with {:?}",
                out.code
            )));
        }
        if out.truncated {
            return Err(SourceError::Failed(format!(
                "ubus call {object} {method}: output too large"
            )));
        }
        serde_json::from_slice(&out.stdout)
            .map_err(|e| SourceError::Failed(format!("ubus call {object} {method}: {e}")))
    }

    fn read(&self, path: &str) -> SourceResult<String> {
        std::fs::read_to_string(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => SourceError::Missing(path.to_string()),
            _ => SourceError::Failed(format!("{path}: {e}")),
        })
    }

    fn list_dir(&self, path: &str) -> SourceResult<Vec<String>> {
        let entries = std::fs::read_dir(path).map_err(|e| match e.kind() {
            std::io::ErrorKind::NotFound => SourceError::Missing(path.to_string()),
            _ => SourceError::Failed(format!("{path}: {e}")),
        })?;
        let mut names: Vec<String> = entries
            .filter_map(Result::ok)
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        Ok(names)
    }

    fn command(&self, program: &str, args: &[&str]) -> SourceResult<String> {
        let out =
            exec::run(program, args, None, Duration::from_secs(20), MAX_OUTPUT).map_err(|e| {
                if e.kind() == std::io::ErrorKind::NotFound {
                    SourceError::Missing(program.to_string())
                } else {
                    SourceError::Failed(format!("{program}: {e}"))
                }
            })?;
        if !out.success() {
            return Err(SourceError::Failed(format!(
                "{program} exited with {:?}",
                out.code
            )));
        }
        String::from_utf8(out.stdout)
            .map_err(|_| SourceError::Failed(format!("{program}: output is not UTF-8")))
    }

    fn now(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    }
}

/// Serves captures written by `tools/capture-fixtures.sh` and sanitized by
/// `tools/sanitize-fixtures.py`.
#[derive(Debug, Clone)]
pub struct FixtureSource {
    dir: PathBuf,
    now: u64,
}

impl FixtureSource {
    pub fn new(dir: &Path) -> FixtureSource {
        let now = std::fs::read_to_string(dir.join("system_info.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get("localtime").and_then(Value::as_u64))
            .unwrap_or(1_790_000_000);
        FixtureSource {
            dir: dir.to_path_buf(),
            now,
        }
    }

    pub fn with_now(mut self, now: u64) -> FixtureSource {
        self.now = now;
        self
    }

    fn file(&self, name: &str) -> SourceResult<String> {
        std::fs::read_to_string(self.dir.join(name))
            .map_err(|_| SourceError::Missing(name.to_string()))
    }
}

fn fixture_name_for_ubus(object: &str, method: &str) -> Option<&'static str> {
    Some(match (object, method) {
        ("system", "board") => "system_board.json",
        ("system", "info") => "system_info.json",
        ("network.interface", "dump") => "network_interface_dump.json",
        ("network.device", "status") => "network_device_status.json",
        ("mwan3", "status") => "mwan3_status.json",
        ("ha-cluster", "status") => "ha_cluster_status.json",
        ("keepalived", "dump") => "keepalived_dump.json",
        ("luci-rpc", "getDHCPLeases") => "dhcp_leases.json",
        ("luci.wireguard", "getWgInstances") => "wireguard.json",
        ("iwinfo", "devices") => "iwinfo_devices.json",
        ("iwinfo", "info") => "iwinfo_info.json",
        ("iwinfo", "assoclist") => "iwinfo_assoclist.json",
        ("service", "list") => "service_list.json",
        _ => return None,
    })
}

fn fixture_name_for_path(path: &str) -> Option<&'static str> {
    Some(match path {
        "/proc/stat" => "proc_stat",
        "/proc/loadavg" => "proc_loadavg",
        "/proc/uptime" => "proc_uptime",
        "/proc/sys/net/netfilter/nf_conntrack_count" => "conntrack_count",
        "/proc/sys/net/netfilter/nf_conntrack_max" => "conntrack_max",
        "/etc/openwrt_release" => "openwrt_release",
        "/tmp/sysinfo/model" => "sysinfo_model",
        "/tmp/sysinfo/board_name" => "sysinfo_board_name",
        "/proc/sys/kernel/hostname" => "hostname",
        _ => return None,
    })
}

impl Source for FixtureSource {
    fn ubus(&self, object: &str, method: &str, _args: Option<&str>) -> SourceResult<Value> {
        let name = fixture_name_for_ubus(object, method)
            .ok_or_else(|| SourceError::Missing(format!("ubus object {object}")))?;
        let text = self
            .file(name)
            .map_err(|_| SourceError::Missing(format!("ubus object {object}")))?;
        serde_json::from_str(&text).map_err(|e| SourceError::Failed(format!("{name}: {e}")))
    }

    fn read(&self, path: &str) -> SourceResult<String> {
        match fixture_name_for_path(path) {
            Some(name) => self
                .file(name)
                .map_err(|_| SourceError::Missing(path.to_string())),
            None => Err(SourceError::Missing(path.to_string())),
        }
    }

    fn list_dir(&self, path: &str) -> SourceResult<Vec<String>> {
        Err(SourceError::Missing(path.to_string()))
    }

    fn command(&self, program: &str, args: &[&str]) -> SourceResult<String> {
        let name = match (program, args) {
            ("uname", ["-m"]) => "uname_m",
            ("uci", ["-q", "show", "network"]) => "uci_network",
            ("uci", ["-q", "get", "system.@system[0].hostname"]) => "hostname",
            _ => return Err(SourceError::Missing(program.to_string())),
        };
        self.file(name)
            .map_err(|_| SourceError::Missing(program.to_string()))
    }

    fn now(&self) -> u64 {
        self.now
    }
}
