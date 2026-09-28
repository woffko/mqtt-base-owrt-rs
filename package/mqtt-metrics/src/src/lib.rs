// SPDX-License-Identifier: GPL-2.0-or-later
//! Read-only router metrics publisher for MQTT and Home Assistant.
//!
//! The crate is split into cold-path modules (configuration, catalog,
//! collectors, discovery) and the long-running daemon loop. Nothing in this
//! crate turns an incoming MQTT message into an action on the router; the only
//! accepted message is the Home Assistant birth announcement.

#![forbid(unsafe_code)]

pub mod catalog;
pub mod collect;
pub mod config;
pub mod control;
pub mod daemon;
pub mod engine;
pub mod exec;
pub mod ha;
pub mod legacy;
pub mod log;
pub mod mqtt;
pub mod rpcd;
pub mod source;
pub mod state;
pub mod status;
pub mod text;
pub mod uci;

/// Package version reported in discovery `origin` and status output.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
