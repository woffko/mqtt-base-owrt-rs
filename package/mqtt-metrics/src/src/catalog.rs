// SPDX-License-Identifier: GPL-2.0-or-later
//! Static metric catalog.
//!
//! Every metric is a template. Collectors pair a template with an instance
//! (an interface, a device, a peer, ...) and a reading. Templates expand these
//! placeholders:
//!
//! - `{p}` / `{P}`: instance family prefix, e.g. `wan` / `WAN` for interfaces
//!   selected by the legacy WAN rule and `iface` / `Interface` for others;
//! - `{n}`: raw instance name, `{i}`: sanitized instance name;
//! - `{m}`: raw sub-instance name, `{j}`: sanitized sub-instance name.
//!
//! Metrics flagged `legacy` reproduce the legacy `publish_metrics.sh` output
//! exactly (object id, topic, name, icon, classes) and are on by default.
//! Everything else is opt-in; `recommended` marks the set that the LuCI
//! "Recommended set" action turns on.

use crate::text::sanitize_id;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Group {
    System,
    Interface,
    Traffic,
    Mwan3,
    HaCluster,
    Dhcp,
    WireGuard,
    Wifi,
    Service,
    Packages,
}

impl Group {
    pub const ALL: [Group; 10] = [
        Group::System,
        Group::Interface,
        Group::Traffic,
        Group::Mwan3,
        Group::HaCluster,
        Group::Dhcp,
        Group::WireGuard,
        Group::Wifi,
        Group::Service,
        Group::Packages,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Group::System => "system",
            Group::Interface => "interface",
            Group::Traffic => "traffic",
            Group::Mwan3 => "mwan3",
            Group::HaCluster => "ha",
            Group::Dhcp => "dhcp",
            Group::WireGuard => "wireguard",
            Group::Wifi => "wifi",
            Group::Service => "service",
            Group::Packages => "packages",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Group::System => "System",
            Group::Interface => "Network interfaces",
            Group::Traffic => "Traffic",
            Group::Mwan3 => "Multi-WAN (mwan3)",
            Group::HaCluster => "HA cluster",
            Group::Dhcp => "DHCP",
            Group::WireGuard => "WireGuard",
            Group::Wifi => "Wi-Fi",
            Group::Service => "Services",
            Group::Packages => "Packages",
        }
    }

    pub fn parse(id: &str) -> Option<Group> {
        Group::ALL.iter().copied().find(|g| g.id() == id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Sensor,
    Binary { on: &'static str, off: &'static str },
}

impl Kind {
    pub fn component(self) -> &'static str {
        match self {
            Kind::Sensor => "sensor",
            Kind::Binary { .. } => "binary_sensor",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MetricDef {
    /// Stable template id, e.g. `iface.ipv4`.
    pub id: &'static str,
    pub group: Group,
    pub kind: Kind,
    pub object: &'static str,
    pub topic: &'static str,
    pub name: &'static str,
    pub icon: Option<&'static str>,
    pub device_class: Option<&'static str>,
    pub unit: Option<&'static str>,
    pub state_class: Option<&'static str>,
    pub diagnostic: bool,
    pub legacy: bool,
    pub recommended: bool,
    /// Group-specific default interval in seconds (slow sources).
    pub default_interval: Option<u32>,
}

const UP_DOWN: Kind = Kind::Binary {
    on: "up",
    off: "down",
};
const ON_OFF: Kind = Kind::Binary {
    on: "ON",
    off: "OFF",
};

const fn sensor(
    id: &'static str,
    group: Group,
    object: &'static str,
    topic: &'static str,
    name: &'static str,
) -> MetricDef {
    MetricDef {
        id,
        group,
        kind: Kind::Sensor,
        object,
        topic,
        name,
        icon: None,
        device_class: None,
        unit: None,
        state_class: None,
        diagnostic: false,
        legacy: false,
        recommended: false,
        default_interval: None,
    }
}

const fn binary(
    id: &'static str,
    group: Group,
    object: &'static str,
    topic: &'static str,
    name: &'static str,
) -> MetricDef {
    MetricDef {
        kind: ON_OFF,
        ..sensor(id, group, object, topic, name)
    }
}

impl MetricDef {
    const fn icon(mut self, icon: &'static str) -> MetricDef {
        self.icon = Some(icon);
        self
    }
    const fn class(mut self, device_class: &'static str) -> MetricDef {
        self.device_class = Some(device_class);
        self
    }
    const fn unit(mut self, unit: &'static str) -> MetricDef {
        self.unit = Some(unit);
        self
    }
    const fn measurement(mut self) -> MetricDef {
        self.state_class = Some("measurement");
        self
    }
    const fn total_increasing(mut self) -> MetricDef {
        self.state_class = Some("total_increasing");
        self
    }
    const fn diag(mut self) -> MetricDef {
        self.diagnostic = true;
        self
    }
    const fn legacy(mut self) -> MetricDef {
        self.legacy = true;
        self
    }
    const fn rec(mut self) -> MetricDef {
        self.recommended = true;
        self
    }
    const fn kind(mut self, kind: Kind) -> MetricDef {
        self.kind = kind;
        self
    }
    const fn every(mut self, seconds: u32) -> MetricDef {
        self.default_interval = Some(seconds);
        self
    }
}

use Group::*;

// System ---------------------------------------------------------------
pub static SYSTEM_HOSTNAME: MetricDef = sensor(
    "system.hostname",
    System,
    "system_hostname",
    "system/hostname",
    "Hostname",
)
.icon("mdi:router-network")
.legacy();
pub static SYSTEM_MODEL: MetricDef = sensor(
    "system.model",
    System,
    "system_model",
    "system/model",
    "Model",
)
.icon("mdi:router")
.legacy();
pub static SYSTEM_TARGET: MetricDef = sensor(
    "system.target_platform",
    System,
    "system_target_platform",
    "system/target_platform",
    "Target Platform",
)
.icon("mdi:chip")
.legacy();
pub static SYSTEM_ARCH: MetricDef = sensor(
    "system.architecture",
    System,
    "system_architecture",
    "system/architecture",
    "Architecture",
)
.icon("mdi:cpu-64-bit")
.legacy();
pub static SYSTEM_VERSION: MetricDef = sensor(
    "system.version",
    System,
    "system_version",
    "system/version",
    "Version",
)
.icon("mdi:tag")
.legacy();
pub static SYSTEM_UPTIME: MetricDef = sensor(
    "system.uptime",
    System,
    "system_uptime",
    "system/uptime",
    "Uptime",
)
.icon("mdi:clock-outline")
.class("duration")
.unit("s")
.legacy();
pub static SYSTEM_REVISION: MetricDef = sensor(
    "system.revision",
    System,
    "system_revision",
    "system/revision",
    "Firmware Revision",
)
.icon("mdi:source-commit")
.diag()
.rec();
pub static SYSTEM_KERNEL: MetricDef = sensor(
    "system.kernel",
    System,
    "system_kernel",
    "system/kernel",
    "Kernel Version",
)
.icon("mdi:linux")
.diag()
.rec();
pub static SYSTEM_BOOT_TIME: MetricDef = sensor(
    "system.boot_time",
    System,
    "system_boot_time",
    "system/boot_time",
    "Boot Time",
)
.class("timestamp")
.diag();
pub static SYSTEM_LOAD1: MetricDef = sensor(
    "system.load_1",
    System,
    "system_load_1",
    "system/load_1",
    "Load (1 min)",
)
.icon("mdi:gauge")
.measurement()
.rec();
pub static SYSTEM_LOAD5: MetricDef = sensor(
    "system.load_5",
    System,
    "system_load_5",
    "system/load_5",
    "Load (5 min)",
)
.icon("mdi:gauge")
.measurement()
.rec();
pub static SYSTEM_LOAD15: MetricDef = sensor(
    "system.load_15",
    System,
    "system_load_15",
    "system/load_15",
    "Load (15 min)",
)
.icon("mdi:gauge")
.measurement()
.rec();
pub static SYSTEM_CPU: MetricDef = sensor(
    "system.cpu_usage",
    System,
    "system_cpu_usage",
    "system/cpu_usage",
    "CPU Usage",
)
.icon("mdi:speedometer")
.unit("%")
.measurement()
.rec();
pub static SYSTEM_CPU_CORE: MetricDef = sensor(
    "system.cpu_core_usage",
    System,
    "system_cpu{i}_usage",
    "system/cpu{n}_usage",
    "CPU {n} Usage",
)
.icon("mdi:speedometer")
.unit("%")
.measurement();
pub static SYSTEM_MEM_USED: MetricDef = sensor(
    "system.memory_used",
    System,
    "system_memory_used",
    "system/memory_used",
    "Memory Used",
)
.icon("mdi:memory")
.unit("%")
.measurement()
.rec();
pub static SYSTEM_MEM_AVAIL: MetricDef = sensor(
    "system.memory_available",
    System,
    "system_memory_available",
    "system/memory_available",
    "Memory Available",
)
.class("data_size")
.unit("MiB")
.measurement()
.rec();
pub static SYSTEM_ROOT_USED: MetricDef = sensor(
    "system.root_used",
    System,
    "system_root_used",
    "system/root_used",
    "Root Filesystem Used",
)
.icon("mdi:harddisk")
.unit("%")
.measurement()
.rec();
pub static SYSTEM_TMP_USED: MetricDef = sensor(
    "system.tmp_used",
    System,
    "system_tmp_used",
    "system/tmp_used",
    "Temporary Filesystem Used",
)
.icon("mdi:folder-clock-outline")
.unit("%")
.measurement()
.rec();
pub static SYSTEM_SWAP_USED: MetricDef = sensor(
    "system.swap_used",
    System,
    "system_swap_used",
    "system/swap_used",
    "Swap Used",
)
.icon("mdi:swap-horizontal")
.unit("%")
.measurement();
pub static SYSTEM_CT_COUNT: MetricDef = sensor(
    "system.conntrack_count",
    System,
    "system_conntrack_count",
    "system/conntrack_count",
    "Conntrack Entries",
)
.icon("mdi:lan-connect")
.measurement()
.rec();
pub static SYSTEM_CT_USED: MetricDef = sensor(
    "system.conntrack_used",
    System,
    "system_conntrack_used",
    "system/conntrack_used",
    "Conntrack Used",
)
.icon("mdi:lan-connect")
.unit("%")
.measurement()
.rec();
pub static SYSTEM_TEMP: MetricDef = sensor(
    "system.temperature",
    System,
    "system_temp_{i}",
    "system/temp_{i}",
    "Temperature {n}",
)
.class("temperature")
.unit("°C")
.measurement()
.rec();

// Packages -------------------------------------------------------------
pub static PACKAGES_UPGRADABLE: MetricDef = sensor(
    "packages.upgradable",
    Packages,
    "system_upgradable_packages",
    "system/upgradable_packages",
    "Upgradable Packages",
)
.icon("mdi:package-up")
.measurement()
.every(86_400);

// Interfaces -----------------------------------------------------------
pub static IFACE_STATUS: MetricDef = sensor(
    "iface.status",
    Interface,
    "{p}_{i}_status",
    "{p}-{n}/status",
    "{P} {n} Status",
)
.kind(UP_DOWN)
.class("connectivity")
.legacy();
pub static IFACE_DEVICE: MetricDef = sensor(
    "iface.device",
    Interface,
    "{p}_{i}_device",
    "{p}-{n}/device",
    "{P} {n} Device",
)
.icon("mdi:ethernet")
.legacy();
pub static IFACE_IPV4: MetricDef = sensor(
    "iface.ipv4",
    Interface,
    "{p}_{i}_ipv4",
    "{p}-{n}/ipv4",
    "{P} {n} IPv4",
)
.icon("mdi:ip-network")
.legacy();
pub static IFACE_IPV6: MetricDef = sensor(
    "iface.ipv6",
    Interface,
    "{p}_{i}_ipv6",
    "{p}-{n}/ipv6",
    "{P} {n} IPv6",
)
.icon("mdi:ip-network-outline")
.legacy();
pub static IFACE_GW4: MetricDef = sensor(
    "iface.gateway4",
    Interface,
    "{p}_{i}_gateway4",
    "{p}-{n}/gateway4",
    "{P} {n} Gateway4",
)
.icon("mdi:routes")
.legacy();
pub static IFACE_GW6: MetricDef = sensor(
    "iface.gateway6",
    Interface,
    "{p}_{i}_gateway6",
    "{p}-{n}/gateway6",
    "{P} {n} Gateway6",
)
.icon("mdi:routes")
.legacy();
pub static IFACE_PROTO: MetricDef = sensor(
    "iface.proto",
    Interface,
    "{p}_{i}_proto",
    "{p}-{n}/proto",
    "{P} {n} Protocol",
)
.icon("mdi:lan")
.diag()
.rec();
pub static IFACE_UPTIME: MetricDef = sensor(
    "iface.uptime",
    Interface,
    "{p}_{i}_uptime",
    "{p}-{n}/uptime",
    "{P} {n} Uptime",
)
.icon("mdi:timer-outline")
.class("duration")
.unit("s")
.rec();
pub static IFACE_DNS: MetricDef = sensor(
    "iface.dns",
    Interface,
    "{p}_{i}_dns",
    "{p}-{n}/dns",
    "{P} {n} DNS Servers",
)
.icon("mdi:dns")
.diag()
.rec();
pub static IFACE_PREFIX: MetricDef = sensor(
    "iface.prefix",
    Interface,
    "{p}_{i}_prefix",
    "{p}-{n}/prefix",
    "{P} {n} IPv6 Prefix",
)
.icon("mdi:ip-network-outline")
.diag()
.rec();

// Traffic --------------------------------------------------------------
pub static DEV_RX_BYTES: MetricDef = sensor(
    "traffic.rx_bytes",
    Traffic,
    "dev_{i}_rx_bytes",
    "dev-{n}/rx_bytes",
    "Device {n} RX Bytes",
)
.class("data_size")
.unit("B")
.total_increasing()
.rec();
pub static DEV_TX_BYTES: MetricDef = sensor(
    "traffic.tx_bytes",
    Traffic,
    "dev_{i}_tx_bytes",
    "dev-{n}/tx_bytes",
    "Device {n} TX Bytes",
)
.class("data_size")
.unit("B")
.total_increasing()
.rec();
pub static DEV_RX_RATE: MetricDef = sensor(
    "traffic.rx_rate",
    Traffic,
    "dev_{i}_rx_rate",
    "dev-{n}/rx_rate",
    "Device {n} RX Rate",
)
.class("data_rate")
.unit("Mbit/s")
.measurement()
.rec();
pub static DEV_TX_RATE: MetricDef = sensor(
    "traffic.tx_rate",
    Traffic,
    "dev_{i}_tx_rate",
    "dev-{n}/tx_rate",
    "Device {n} TX Rate",
)
.class("data_rate")
.unit("Mbit/s")
.measurement()
.rec();
pub static DEV_RX_ERRORS: MetricDef = sensor(
    "traffic.rx_errors",
    Traffic,
    "dev_{i}_rx_errors",
    "dev-{n}/rx_errors",
    "Device {n} RX Errors",
)
.icon("mdi:alert-circle-outline")
.total_increasing()
.diag();
pub static DEV_TX_ERRORS: MetricDef = sensor(
    "traffic.tx_errors",
    Traffic,
    "dev_{i}_tx_errors",
    "dev-{n}/tx_errors",
    "Device {n} TX Errors",
)
.icon("mdi:alert-circle-outline")
.total_increasing()
.diag();
pub static DEV_RX_DROPPED: MetricDef = sensor(
    "traffic.rx_dropped",
    Traffic,
    "dev_{i}_rx_dropped",
    "dev-{n}/rx_dropped",
    "Device {n} RX Dropped",
)
.icon("mdi:package-variant-remove")
.total_increasing()
.diag();
pub static DEV_TX_DROPPED: MetricDef = sensor(
    "traffic.tx_dropped",
    Traffic,
    "dev_{i}_tx_dropped",
    "dev-{n}/tx_dropped",
    "Device {n} TX Dropped",
)
.icon("mdi:package-variant-remove")
.total_increasing()
.diag();
pub static DEV_SPEED: MetricDef = sensor(
    "traffic.speed",
    Traffic,
    "dev_{i}_speed",
    "dev-{n}/speed",
    "Device {n} Link Speed",
)
.class("data_rate")
.unit("Mbit/s")
.diag();
pub static DEV_CARRIER: MetricDef = binary(
    "traffic.carrier",
    Traffic,
    "dev_{i}_carrier",
    "dev-{n}/carrier",
    "Device {n} Carrier",
)
.class("connectivity")
.diag();

// Multi-WAN ------------------------------------------------------------
pub static MWAN3_STATUS: MetricDef = sensor(
    "mwan3.status",
    Mwan3,
    "mwan3_{i}_status",
    "mwan3-{n}/status",
    "MWAN3 {n} Status",
)
.icon("mdi:wan")
.rec();
pub static MWAN3_ONLINE: MetricDef = binary(
    "mwan3.online",
    Mwan3,
    "mwan3_{i}_online",
    "mwan3-{n}/online",
    "MWAN3 {n} Online",
)
.class("connectivity")
.rec();
pub static MWAN3_SCORE: MetricDef = sensor(
    "mwan3.score",
    Mwan3,
    "mwan3_{i}_score",
    "mwan3-{n}/score",
    "MWAN3 {n} Score",
)
.icon("mdi:counter")
.measurement()
.rec();
pub static MWAN3_ONLINE_TIME: MetricDef = sensor(
    "mwan3.online_time",
    Mwan3,
    "mwan3_{i}_online_time",
    "mwan3-{n}/online_time",
    "MWAN3 {n} Online Time",
)
.icon("mdi:timer-outline")
.class("duration")
.unit("s")
.rec();
pub static MWAN3_LOST: MetricDef = sensor(
    "mwan3.lost",
    Mwan3,
    "mwan3_{i}_lost",
    "mwan3-{n}/lost",
    "MWAN3 {n} Lost Probes",
)
.icon("mdi:close-network-outline")
.measurement()
.diag();
pub static MWAN3_TRACK_LATENCY: MetricDef = sensor(
    "mwan3.track_latency",
    Mwan3,
    "mwan3_{i}_{j}_latency",
    "mwan3-{n}/{j}/latency",
    "MWAN3 {n} {m} Latency",
)
.class("duration")
.unit("ms")
.measurement()
.rec();
pub static MWAN3_TRACK_LOSS: MetricDef = sensor(
    "mwan3.track_loss",
    Mwan3,
    "mwan3_{i}_{j}_loss",
    "mwan3-{n}/{j}/loss",
    "MWAN3 {n} {m} Packet Loss",
)
.icon("mdi:chart-bell-curve")
.unit("%")
.measurement()
.rec();
pub static MWAN3_TRACK_STATUS: MetricDef = sensor(
    "mwan3.track_status",
    Mwan3,
    "mwan3_{i}_{j}_status",
    "mwan3-{n}/{j}/status",
    "MWAN3 {n} {m} Status",
)
.icon("mdi:target")
.diag();

// HA cluster -----------------------------------------------------------
pub static HA_ROLE: MetricDef = sensor("ha.role", HaCluster, "ha_role", "ha/role", "HA Role")
    .icon("mdi:server-network")
    .rec();
pub static HA_STATE: MetricDef = sensor("ha.state", HaCluster, "ha_state", "ha/state", "HA State")
    .icon("mdi:heart-pulse")
    .rec();
pub static HA_PEER_ONLINE: MetricDef = binary(
    "ha.peer_online",
    HaCluster,
    "ha_peer_{i}_online",
    "ha/peer_{i}/online",
    "HA Peer {n} Online",
)
.class("connectivity")
.rec();
pub static HA_SERVICE: MetricDef = binary(
    "ha.service_running",
    HaCluster,
    "ha_service_{i}_running",
    "ha/service_{i}/running",
    "HA Service {n}",
)
.class("running")
.rec();
pub static HA_VRRP: MetricDef = sensor(
    "ha.vrrp_state",
    HaCluster,
    "ha_vrrp_{i}_state",
    "ha/vrrp_{i}/state",
    "HA VRRP {n} State",
)
.icon("mdi:swap-horizontal-bold")
.rec();
pub static HA_CONFIG_SYNC_AGE: MetricDef = sensor(
    "ha.config_sync_age",
    HaCluster,
    "ha_config_sync_age",
    "ha/config_sync_age",
    "HA Config Sync Age",
)
.class("duration")
.unit("s")
.diag();
pub static HA_LEASE_SYNC_AGE: MetricDef = sensor(
    "ha.lease_sync_age",
    HaCluster,
    "ha_lease_sync_age",
    "ha/lease_sync_age",
    "HA Lease Sync Age",
)
.class("duration")
.unit("s")
.diag();

// DHCP -----------------------------------------------------------------
pub static DHCP_LEASES4: MetricDef = sensor(
    "dhcp.leases_ipv4",
    Dhcp,
    "dhcp_leases_ipv4",
    "dhcp/leases_ipv4",
    "DHCP Leases IPv4",
)
.icon("mdi:lan")
.measurement()
.rec();
pub static DHCP_LEASES6: MetricDef = sensor(
    "dhcp.leases_ipv6",
    Dhcp,
    "dhcp_leases_ipv6",
    "dhcp/leases_ipv6",
    "DHCP Leases IPv6",
)
.icon("mdi:lan")
.measurement()
.rec();

// WireGuard ------------------------------------------------------------
pub static WG_PEERS: MetricDef = sensor(
    "wireguard.peers",
    WireGuard,
    "wg_{i}_peers",
    "wg-{n}/peers",
    "WireGuard {n} Peers",
)
.icon("mdi:vpn")
.measurement();
pub static WG_PEER_CONNECTED: MetricDef = binary(
    "wireguard.peer_connected",
    WireGuard,
    "wg_{i}_{j}_connected",
    "wg-{n}/{j}/connected",
    "WireGuard {n} {m} Connected",
)
.class("connectivity");
pub static WG_PEER_HANDSHAKE: MetricDef = sensor(
    "wireguard.peer_handshake_age",
    WireGuard,
    "wg_{i}_{j}_handshake_age",
    "wg-{n}/{j}/handshake_age",
    "WireGuard {n} {m} Handshake Age",
)
.class("duration")
.unit("s");
pub static WG_PEER_RX: MetricDef = sensor(
    "wireguard.peer_rx",
    WireGuard,
    "wg_{i}_{j}_rx_bytes",
    "wg-{n}/{j}/rx_bytes",
    "WireGuard {n} {m} RX Bytes",
)
.class("data_size")
.unit("B")
.total_increasing();
pub static WG_PEER_TX: MetricDef = sensor(
    "wireguard.peer_tx",
    WireGuard,
    "wg_{i}_{j}_tx_bytes",
    "wg-{n}/{j}/tx_bytes",
    "WireGuard {n} {m} TX Bytes",
)
.class("data_size")
.unit("B")
.total_increasing();

// Wi-Fi ----------------------------------------------------------------
pub static WIFI_CLIENTS: MetricDef = sensor(
    "wifi.clients",
    Wifi,
    "wifi_{i}_clients",
    "wifi-{n}/clients",
    "Wi-Fi {n} Clients",
)
.icon("mdi:wifi")
.measurement()
.rec();
pub static WIFI_CHANNEL: MetricDef = sensor(
    "wifi.channel",
    Wifi,
    "wifi_{i}_channel",
    "wifi-{n}/channel",
    "Wi-Fi {n} Channel",
)
.icon("mdi:wifi-cog")
.diag();
pub static WIFI_TXPOWER: MetricDef = sensor(
    "wifi.txpower",
    Wifi,
    "wifi_{i}_txpower",
    "wifi-{n}/txpower",
    "Wi-Fi {n} TX Power",
)
.icon("mdi:signal")
.unit("dBm")
.diag();

// Services -------------------------------------------------------------
pub static SERVICE_RUNNING: MetricDef = binary(
    "service.running",
    Service,
    "service_{i}_running",
    "service-{n}/running",
    "Service {n}",
)
.class("running");

/// Every template, in display order.
pub static ALL: &[&MetricDef] = &[
    &SYSTEM_HOSTNAME,
    &SYSTEM_MODEL,
    &SYSTEM_TARGET,
    &SYSTEM_ARCH,
    &SYSTEM_VERSION,
    &SYSTEM_UPTIME,
    &SYSTEM_REVISION,
    &SYSTEM_KERNEL,
    &SYSTEM_BOOT_TIME,
    &SYSTEM_LOAD1,
    &SYSTEM_LOAD5,
    &SYSTEM_LOAD15,
    &SYSTEM_CPU,
    &SYSTEM_CPU_CORE,
    &SYSTEM_MEM_USED,
    &SYSTEM_MEM_AVAIL,
    &SYSTEM_ROOT_USED,
    &SYSTEM_TMP_USED,
    &SYSTEM_SWAP_USED,
    &SYSTEM_CT_COUNT,
    &SYSTEM_CT_USED,
    &SYSTEM_TEMP,
    &PACKAGES_UPGRADABLE,
    &IFACE_STATUS,
    &IFACE_DEVICE,
    &IFACE_IPV4,
    &IFACE_IPV6,
    &IFACE_GW4,
    &IFACE_GW6,
    &IFACE_PROTO,
    &IFACE_UPTIME,
    &IFACE_DNS,
    &IFACE_PREFIX,
    &DEV_RX_BYTES,
    &DEV_TX_BYTES,
    &DEV_RX_RATE,
    &DEV_TX_RATE,
    &DEV_RX_ERRORS,
    &DEV_TX_ERRORS,
    &DEV_RX_DROPPED,
    &DEV_TX_DROPPED,
    &DEV_SPEED,
    &DEV_CARRIER,
    &MWAN3_STATUS,
    &MWAN3_ONLINE,
    &MWAN3_SCORE,
    &MWAN3_ONLINE_TIME,
    &MWAN3_LOST,
    &MWAN3_TRACK_LATENCY,
    &MWAN3_TRACK_LOSS,
    &MWAN3_TRACK_STATUS,
    &HA_ROLE,
    &HA_STATE,
    &HA_PEER_ONLINE,
    &HA_SERVICE,
    &HA_VRRP,
    &HA_CONFIG_SYNC_AGE,
    &HA_LEASE_SYNC_AGE,
    &DHCP_LEASES4,
    &DHCP_LEASES6,
    &WG_PEERS,
    &WG_PEER_CONNECTED,
    &WG_PEER_HANDSHAKE,
    &WG_PEER_RX,
    &WG_PEER_TX,
    &WIFI_CLIENTS,
    &WIFI_CHANNEL,
    &WIFI_TXPOWER,
    &SERVICE_RUNNING,
];

/// Family prefix for interface-like instances.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Interfaces selected by the legacy `wan*`/`wwan*` rule.
    Wan,
    /// Any other interface.
    Iface,
}

/// The instance a sample belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Instance {
    pub family: Option<Family>,
    pub name: String,
    pub sub: String,
}

impl Instance {
    pub fn none() -> Instance {
        Instance::default()
    }

    pub fn named(name: &str) -> Instance {
        Instance {
            family: None,
            name: name.to_string(),
            sub: String::new(),
        }
    }

    pub fn iface(family: Family, name: &str) -> Instance {
        Instance {
            family: Some(family),
            name: name.to_string(),
            sub: String::new(),
        }
    }

    pub fn with_sub(mut self, sub: &str) -> Instance {
        self.sub = sub.to_string();
        self
    }
}

/// Expand a template for an instance.
pub fn expand(template: &str, inst: &Instance) -> String {
    let (p, big_p) = match inst.family {
        Some(Family::Wan) => ("wan", "WAN"),
        Some(Family::Iface) => ("iface", "Interface"),
        None => ("", ""),
    };
    let mut out = String::with_capacity(template.len() + inst.name.len() * 2);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let tail = &rest[start..];
        let (value, used) = if tail.starts_with("{p}") {
            (p.to_string(), 3)
        } else if tail.starts_with("{P}") {
            (big_p.to_string(), 3)
        } else if tail.starts_with("{n}") {
            (inst.name.clone(), 3)
        } else if tail.starts_with("{i}") {
            (sanitize_id(&inst.name), 3)
        } else if tail.starts_with("{m}") {
            (inst.sub.clone(), 3)
        } else if tail.starts_with("{j}") {
            (sanitize_id(&inst.sub), 3)
        } else {
            ("{".to_string(), 1)
        };
        out.push_str(&value);
        rest = &tail[used..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_ids_are_unique() {
        let mut ids: Vec<&str> = ALL.iter().map(|d| d.id).collect();
        ids.sort_unstable();
        let before = ids.len();
        ids.dedup();
        assert_eq!(before, ids.len());
    }

    #[test]
    fn topics_reference_lists_every_template() {
        let doc = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../docs/TOPICS.md"),
        )
        .unwrap();
        for def in ALL {
            let row = format!(
                "| `{}` | `{}` | `{}` | {} |",
                def.id, def.object, def.topic, def.name
            );
            assert!(
                doc.contains(&row),
                "docs/TOPICS.md is stale for {}; run tools/gen-topics.py",
                def.id
            );
        }
    }

    #[test]
    fn legacy_set_matches_script() {
        let legacy: Vec<&str> = ALL.iter().filter(|d| d.legacy).map(|d| d.id).collect();
        assert_eq!(legacy.len(), 12);
    }

    #[test]
    fn expands_legacy_wan_templates() {
        let inst = Instance::iface(Family::Wan, "wan_6");
        assert_eq!(expand(IFACE_STATUS.object, &inst), "wan_wan_6_status");
        assert_eq!(expand(IFACE_STATUS.topic, &inst), "wan-wan_6/status");
        assert_eq!(expand(IFACE_STATUS.name, &inst), "WAN wan_6 Status");
        let lan = Instance::iface(Family::Iface, "lan");
        assert_eq!(expand(IFACE_IPV4.object, &lan), "iface_lan_ipv4");
        assert_eq!(expand(IFACE_IPV4.name, &lan), "Interface lan IPv4");
        let track = Instance::named("wan").with_sub("1.1.1.1");
        assert_eq!(
            expand(MWAN3_TRACK_LATENCY.object, &track),
            "mwan3_wan_1_1_1_1_latency"
        );
        assert_eq!(
            expand(MWAN3_TRACK_LATENCY.topic, &track),
            "mwan3-wan/1_1_1_1/latency"
        );
        assert_eq!(
            expand(MWAN3_TRACK_LATENCY.name, &track),
            "MWAN3 wan 1.1.1.1 Latency"
        );
    }
}
