// SPDX-License-Identifier: GPL-2.0-or-later
//! System metrics: the six legacy values plus load, CPU, memory, storage,
//! conntrack and temperatures.

use serde_json::Value as Json;

use super::{
    clock_is_set, hostname, CollectorState, Context, CpuTimes, GroupResult, Reading, Value,
};
use crate::catalog::*;
use crate::text::rfc3339_utc;

pub fn collect(ctx: &Context, state: &mut CollectorState, out: &mut GroupResult) {
    let src = ctx.source;
    let none = Instance::none;

    // Legacy values with the legacy fallbacks.
    out.push(&SYSTEM_HOSTNAME, none(), Reading::text(hostname(src)));
    let model = src
        .read("/tmp/sysinfo/model")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    out.push(
        &SYSTEM_MODEL,
        none(),
        Reading::text(if model.is_empty() {
            "Unknown Model".to_string()
        } else {
            model
        }),
    );
    let board = src
        .read("/tmp/sysinfo/board_name")
        .map(|s| s.trim().to_string())
        .unwrap_or_default();
    out.push(
        &SYSTEM_TARGET,
        none(),
        Reading::text(if board.is_empty() {
            "Unknown Platform".to_string()
        } else {
            board
        }),
    );
    if state.arch.is_none() {
        state.arch = src
            .command("uname", &["-m"])
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
    }
    out.push(
        &SYSTEM_ARCH,
        none(),
        match &state.arch {
            Some(arch) => Reading::text(arch.clone()),
            None => Reading::missing("uname -m failed"),
        },
    );
    let release = src.read("/etc/openwrt_release").unwrap_or_default();
    let version = release_field(&release, "DISTRIB_RELEASE")
        .or_else(|| {
            src.read("/etc/openwrt_version")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "Unknown Version".to_string());
    out.push(&SYSTEM_VERSION, none(), Reading::text(version));
    let uptime = src.read("/proc/uptime").ok().and_then(|s| {
        s.split(['.', ' '])
            .next()
            .and_then(|v| v.trim().parse::<u64>().ok())
    });
    match uptime {
        Some(up) => {
            out.push(
                &SYSTEM_UPTIME,
                none(),
                Reading::Value(Value::Int(to_i64(up))),
            );
            out.push(
                &SYSTEM_BOOT_TIME,
                none(),
                if clock_is_set(ctx.now) {
                    Reading::text(rfc3339_utc(ctx.now.saturating_sub(up)))
                } else {
                    Reading::missing("the system clock is not set")
                },
            );
        }
        None => {
            out.push(
                &SYSTEM_UPTIME,
                none(),
                Reading::missing("/proc/uptime is not readable"),
            );
            out.push(
                &SYSTEM_BOOT_TIME,
                none(),
                Reading::missing("/proc/uptime is not readable"),
            );
        }
    }

    out.push(
        &SYSTEM_REVISION,
        none(),
        Reading::non_empty(
            release_field(&release, "DISTRIB_REVISION").unwrap_or_default(),
            "no DISTRIB_REVISION",
        ),
    );
    let board_info = src.ubus("system", "board", None);
    out.push(
        &SYSTEM_KERNEL,
        none(),
        match &board_info {
            Ok(b) => Reading::non_empty(
                b.get("kernel")
                    .and_then(Json::as_str)
                    .unwrap_or("")
                    .to_string(),
                "no kernel version",
            ),
            Err(e) => Reading::from_error(e),
        },
    );

    load(ctx, out);
    cpu(ctx, state, out);
    memory_and_storage(ctx, out);
    conntrack(ctx, out);
    temperatures(ctx, out);
}

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// Value of `KEY='value'` in `/etc/openwrt_release`.
fn release_field(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.strip_prefix(key)?.strip_prefix('=')?;
        let value = rest.trim().trim_matches(['\'', '"']);
        (!value.is_empty()).then(|| value.to_string())
    })
}

fn load(ctx: &Context, out: &mut GroupResult) {
    let text = ctx.source.read("/proc/loadavg");
    let fields: Vec<f64> = text
        .as_deref()
        .unwrap_or("")
        .split_whitespace()
        .take(3)
        .filter_map(|v| v.parse().ok())
        .collect();
    for (i, def) in [&SYSTEM_LOAD1, &SYSTEM_LOAD5, &SYSTEM_LOAD15]
        .into_iter()
        .enumerate()
    {
        let reading = match fields.get(i) {
            Some(v) => Reading::Value(Value::Float(*v, 2)),
            None => Reading::missing("/proc/loadavg is not readable"),
        };
        out.push(def, Instance::none(), reading);
    }
}

/// Parse the `cpu*` lines of `/proc/stat`.
pub fn cpu_times(stat: &str) -> Vec<(String, CpuTimes)> {
    let mut out = Vec::new();
    for line in stat.lines() {
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else { continue };
        if !name.starts_with("cpu") {
            continue;
        }
        let values: Vec<u64> = fields.take(8).filter_map(|v| v.parse().ok()).collect();
        if values.len() < 5 {
            continue;
        }
        let total: u64 = values.iter().sum();
        let idle = values[3] + values[4];
        out.push((name.to_string(), CpuTimes { total, idle }));
    }
    out
}

fn usage(prev: CpuTimes, cur: CpuTimes) -> Option<f64> {
    let total = cur.total.checked_sub(prev.total)?;
    let idle = cur.idle.checked_sub(prev.idle)?;
    if total == 0 || idle > total {
        return None;
    }
    #[allow(clippy::cast_precision_loss)]
    Some((total - idle) as f64 * 100.0 / total as f64)
}

fn cpu(ctx: &Context, state: &mut CollectorState, out: &mut GroupResult) {
    let stat = match ctx.source.read("/proc/stat") {
        Ok(s) => s,
        Err(e) => {
            out.push(&SYSTEM_CPU, Instance::none(), Reading::from_error(&e));
            return;
        }
    };
    for (name, cur) in cpu_times(&stat) {
        let reading = match state
            .cpu
            .insert(name.clone(), cur)
            .and_then(|prev| usage(prev, cur))
        {
            Some(u) => Reading::Value(Value::Float(u, 1)),
            None => Reading::missing("waiting for a second sample"),
        };
        if name == "cpu" {
            out.push(&SYSTEM_CPU, Instance::none(), reading);
        } else {
            out.push(
                &SYSTEM_CPU_CORE,
                Instance::named(&name["cpu".len()..]),
                reading,
            );
        }
    }
}

fn percent(used: f64, total: f64, decimals: u8) -> Reading {
    if total > 0.0 {
        Reading::Value(Value::Float(used * 100.0 / total, decimals))
    } else {
        Reading::missing("total is zero")
    }
}

#[allow(clippy::cast_precision_loss)]
fn memory_and_storage(ctx: &Context, out: &mut GroupResult) {
    let info = match ctx.source.ubus("system", "info", None) {
        Ok(i) => i,
        Err(e) => {
            for def in [
                &SYSTEM_MEM_USED,
                &SYSTEM_MEM_AVAIL,
                &SYSTEM_ROOT_USED,
                &SYSTEM_TMP_USED,
                &SYSTEM_SWAP_USED,
            ] {
                out.push(def, Instance::none(), Reading::from_error(&e));
            }
            return;
        }
    };
    let num = |path: &[&str]| -> Option<f64> {
        let mut v = &info;
        for key in path {
            v = v.get(key)?;
        }
        v.as_u64().map(|n| n as f64)
    };
    match (num(&["memory", "total"]), num(&["memory", "available"])) {
        (Some(total), Some(avail)) => {
            out.push(
                &SYSTEM_MEM_USED,
                Instance::none(),
                percent(total - avail, total, 1),
            );
            out.push(
                &SYSTEM_MEM_AVAIL,
                Instance::none(),
                Reading::Value(Value::Float(avail / 1_048_576.0, 1)),
            );
        }
        _ => {
            out.push(
                &SYSTEM_MEM_USED,
                Instance::none(),
                Reading::missing("no memory information"),
            );
            out.push(
                &SYSTEM_MEM_AVAIL,
                Instance::none(),
                Reading::missing("no memory information"),
            );
        }
    }
    for (def, key) in [(&SYSTEM_ROOT_USED, "root"), (&SYSTEM_TMP_USED, "tmp")] {
        let reading = match (num(&[key, "total"]), num(&[key, "used"])) {
            (Some(total), Some(used)) => percent(used, total, 1),
            _ => Reading::missing(format!("no {key} filesystem information")),
        };
        out.push(def, Instance::none(), reading);
    }
    let swap = match (num(&["swap", "total"]), num(&["swap", "free"])) {
        (Some(total), Some(free)) if total > 0.0 => percent(total - free, total, 1),
        (Some(_), Some(_)) => Reading::impossible("no swap is configured"),
        _ => Reading::missing("no swap information"),
    };
    out.push(&SYSTEM_SWAP_USED, Instance::none(), swap);
}

fn conntrack(ctx: &Context, out: &mut GroupResult) {
    let read = |path: &str| {
        ctx.source
            .read(path)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
    };
    match read("/proc/sys/net/netfilter/nf_conntrack_count") {
        Some(count) => {
            out.push(
                &SYSTEM_CT_COUNT,
                Instance::none(),
                Reading::Value(Value::Int(to_i64(count))),
            );
            #[allow(clippy::cast_precision_loss)]
            let used = match read("/proc/sys/net/netfilter/nf_conntrack_max") {
                Some(max) => percent(count as f64, max as f64, 2),
                None => Reading::missing("nf_conntrack_max is not readable"),
            };
            out.push(&SYSTEM_CT_USED, Instance::none(), used);
        }
        None => {
            out.miss(&SYSTEM_CT_COUNT, "connection tracking is not loaded");
            out.miss(&SYSTEM_CT_USED, "connection tracking is not loaded");
        }
    }
}

fn temperatures(ctx: &Context, out: &mut GroupResult) {
    let src = ctx.source;
    let mut found = 0;
    if let Ok(hwmons) = src.list_dir("/sys/class/hwmon") {
        for hw in hwmons {
            let base = format!("/sys/class/hwmon/{hw}");
            let chip = src
                .read(&format!("{base}/name"))
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| hw.clone());
            let Ok(files) = src.list_dir(&base) else {
                continue;
            };
            for file in files
                .iter()
                .filter(|f| f.starts_with("temp") && f.ends_with("_input"))
            {
                let index = &file["temp".len()..file.len() - "_input".len()];
                let Some(milli) = src
                    .read(&format!("{base}/{file}"))
                    .ok()
                    .and_then(|s| s.trim().parse::<i64>().ok())
                else {
                    continue;
                };
                let label = src
                    .read(&format!("{base}/temp{index}_label"))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| format!("temp{index}"));
                found += 1;
                #[allow(clippy::cast_precision_loss)]
                let celsius = milli as f64 / 1000.0;
                out.push(
                    &SYSTEM_TEMP,
                    Instance::named(&format!("{chip} {label}")),
                    Reading::Value(Value::Float(celsius, 1)),
                );
            }
        }
    }
    if found == 0 {
        if let Ok(zones) = src.list_dir("/sys/class/thermal") {
            for zone in zones.iter().filter(|z| z.starts_with("thermal_zone")) {
                let base = format!("/sys/class/thermal/{zone}");
                let Some(milli) = src
                    .read(&format!("{base}/temp"))
                    .ok()
                    .and_then(|s| s.trim().parse::<i64>().ok())
                else {
                    continue;
                };
                let kind = src
                    .read(&format!("{base}/type"))
                    .map(|s| s.trim().to_string())
                    .unwrap_or_else(|_| zone.clone());
                found += 1;
                #[allow(clippy::cast_precision_loss)]
                let celsius = milli as f64 / 1000.0;
                out.push(
                    &SYSTEM_TEMP,
                    Instance::named(&kind),
                    Reading::Value(Value::Float(celsius, 1)),
                );
            }
        }
    }
    if found == 0 {
        out.miss(&SYSTEM_TEMP, "no temperature sensors found");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Group;
    use crate::collect::tests::{fixture, run, value_of};
    use crate::collect::CollectorState;
    use crate::config::Config;

    #[test]
    fn legacy_values_from_fixture() {
        let src = fixture("primary");
        let mut state = CollectorState::default();
        let r = run(Group::System, &src, &Config::default(), &mut state, 0.0);
        let text = |id: &str| match value_of(&r, id, "", "") {
            Some(Reading::Value(v)) => v.payload(Kind::Sensor),
            other => panic!("{id}: {other:?}"),
        };
        assert_eq!(text("system.hostname"), "router-primary");
        assert_eq!(text("system.model"), "QEMU Standard PC (Q35 + ICH9, 2009)");
        assert_eq!(
            text("system.target_platform"),
            "qemu-standard-pc-q35-ich9-2009"
        );
        assert_eq!(text("system.architecture"), "x86_64");
        assert_eq!(text("system.version"), "25.12-SNAPSHOT");
        assert!(text("system.uptime").parse::<u64>().unwrap() > 800_000);
        assert_eq!(text("system.kernel"), "6.12.107");
        assert!(matches!(
            value_of(&r, "system.cpu_usage", "", ""),
            Some(Reading::Unavailable {
                permanent: false,
                ..
            })
        ));
        assert!(matches!(
            value_of(&r, "system.swap_used", "", ""),
            Some(Reading::Unavailable {
                permanent: true,
                ..
            })
        ));
        assert!(r.missing.iter().any(|(d, _)| d.id == "system.temperature"));
    }

    #[test]
    fn cpu_usage_from_two_samples() {
        let first = "cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0 0 0\n";
        let second = "cpu  150 0 150 900 0 0 0 0 0 0\ncpu0 60 0 60 480 0 0 0 0 0 0\n";
        let a = cpu_times(first);
        let b = cpu_times(second);
        assert_eq!(a.len(), 2);
        let u = usage(a[0].1, b[0].1).unwrap();
        assert!((u - 50.0).abs() < 1e-9);
        let core = usage(a[1].1, b[1].1).unwrap();
        assert!((core - 20.0).abs() < 1e-9);
        assert_eq!(usage(b[0].1, a[0].1), None, "counter reset");
    }

    #[test]
    fn release_parsing() {
        let text = "DISTRIB_ID='OpenWrt'\nDISTRIB_RELEASE='25.12-SNAPSHOT'\nDISTRIB_REVISION='r0+1524-b6a820ea85'\n";
        assert_eq!(
            release_field(text, "DISTRIB_RELEASE").as_deref(),
            Some("25.12-SNAPSHOT")
        );
        assert_eq!(
            release_field(text, "DISTRIB_REVISION").as_deref(),
            Some("r0+1524-b6a820ea85")
        );
        assert_eq!(release_field(text, "DISTRIB_TARGET"), None);
    }

    // ---- audit regression tests ------------------------------------------

    #[test]
    fn nothing_is_derived_from_an_unset_clock() {
        let src = fixture("primary").with_now(1_000);
        let r = run(
            Group::System,
            &src,
            &Config::default(),
            &mut CollectorState::default(),
            0.0,
        );
        assert!(matches!(
            value_of(&r, "system.boot_time", "", ""),
            Some(Reading::Unavailable { .. })
        ));
        // Uptime does not depend on the wall clock.
        assert!(matches!(
            value_of(&r, "system.uptime", "", ""),
            Some(Reading::Value(_))
        ));
    }
}
