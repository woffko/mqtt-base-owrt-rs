// SPDX-License-Identifier: GPL-2.0-or-later
//! Minimal MQTT 3.1.1 client (see ADR-0001).

pub mod client;
pub mod codec;
pub mod transport;

pub use client::{Client, Incoming, MqttError, Options, WillMessage};

/// Whether this build can connect over TLS.
pub const TLS_SUPPORTED: bool = cfg!(feature = "tls");
