// SPDX-License-Identifier: GPL-2.0-or-later
//! Minimal leveled logging. procd forwards stdout and stderr to syslog under
//! the service name, so informational lines go to stdout and problems to
//! stderr. Log lines never contain secrets: callers pass only host names,
//! topics, counts and error descriptions.

use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Error = 0,
    Warn = 1,
    Info = 2,
    Debug = 3,
}

impl Level {
    pub fn parse(value: &str) -> Option<Level> {
        match value {
            "error" => Some(Level::Error),
            "warn" | "warning" => Some(Level::Warn),
            "info" => Some(Level::Info),
            "debug" => Some(Level::Debug),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Log {
    pub level: Level,
}

impl Log {
    pub fn new(level: Level) -> Log {
        Log { level }
    }

    pub fn error(&self, msg: &str) {
        self.emit(Level::Error, "error", msg);
    }

    pub fn warn(&self, msg: &str) {
        self.emit(Level::Warn, "warning", msg);
    }

    pub fn info(&self, msg: &str) {
        self.emit(Level::Info, "info", msg);
    }

    pub fn debug(&self, msg: &str) {
        self.emit(Level::Debug, "debug", msg);
    }

    fn emit(&self, level: Level, tag: &str, msg: &str) {
        if level > self.level {
            return;
        }
        // Logging must never take the daemon down; a closed pipe is ignored.
        if level <= Level::Warn {
            let _ = writeln!(std::io::stderr().lock(), "{tag}: {msg}");
        } else {
            let _ = writeln!(std::io::stdout().lock(), "{tag}: {msg}");
        }
    }
}
