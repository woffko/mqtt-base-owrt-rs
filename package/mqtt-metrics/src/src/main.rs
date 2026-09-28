// SPDX-License-Identifier: GPL-2.0-or-later
//! `mqtt-metricsd`: daemon, rpcd plugin and maintenance commands in one
//! binary.

#![forbid(unsafe_code)]

use std::path::PathBuf;

use mqtt_metrics::config::Config;
use mqtt_metrics::engine::Engine;
use mqtt_metrics::log::Level;
use mqtt_metrics::source::{FixtureSource, Source, SystemSource};
use mqtt_metrics::state::TopicState;
use mqtt_metrics::{collect, daemon, legacy, rpcd};

/// Print a line to stdout; a closed pipe (for example `| head`) is not an
/// error for a command-line tool.
macro_rules! out {
    ($($arg:tt)*) => {{
        use std::io::Write as _;
        let _ = writeln!(std::io::stdout().lock(), $($arg)*);
    }};
}

const USAGE: &str = "\
usage: mqtt-metricsd [--log-level LEVEL]      run the publisher (procd)
       mqtt-metricsd --check-config            validate /etc/config/mqtt-metrics
       mqtt-metricsd --dump [--fixtures DIR]    print one full publish pass as JSON lines
       mqtt-metricsd --catalog [--fixtures DIR] print the metric catalog
       mqtt-metricsd --templates                print every metric template as JSON lines
       mqtt-metricsd --import-legacy [SCRIPT]   copy broker settings from publish_metrics.sh
       mqtt-metricsd --purge                    clear every retained topic of this router
       mqtt-metricsd list | call METHOD         rpcd plugin interface
       mqtt-metricsd --version";

fn fixture_arg(args: &[String]) -> Option<PathBuf> {
    args.iter()
        .position(|a| a == "--fixtures")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
}

fn dump(args: &[String]) -> i32 {
    let fixture = fixture_arg(args);
    let source: Box<dyn Source> = match &fixture {
        Some(dir) => Box::new(FixtureSource::new(dir)),
        None => Box::new(SystemSource),
    };
    let cfg = if fixture.is_some() {
        Config::default()
    } else {
        let (cfg, errors) = rpcd::load_config(source.as_ref());
        if !errors.is_empty() {
            for e in errors {
                eprintln!("configuration: {e}");
            }
            return 1;
        }
        cfg
    };
    let hostname = collect::hostname(source.as_ref());
    let mut engine = Engine::new(cfg, &hostname, TopicState::default());
    for p in engine.full_pass(source.as_ref(), source.now(), 0.0) {
        out!(
            "{}",
            serde_json::json!({ "topic": p.topic, "payload": p.payload, "retain": p.retain, "qos": p.qos })
        );
    }
    0
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.first().map(String::as_str) {
        None => daemon::run(None),
        Some("--log-level") => match args.get(1).and_then(|l| Level::parse(l)) {
            Some(level) => daemon::run(Some(level)),
            None => {
                eprintln!("{USAGE}");
                2
            }
        },
        Some("list" | "call") => rpcd::main(&args),
        Some("--check-config") => {
            let (_, errors) = rpcd::load_config(&SystemSource);
            if errors.is_empty() {
                out!("configuration is valid");
                0
            } else {
                for e in errors {
                    out!("{e}");
                }
                1
            }
        }
        Some("--dump") => dump(&args),
        Some("--catalog") => {
            let doc = match fixture_arg(&args) {
                Some(dir) => rpcd::catalog(&FixtureSource::new(&dir)),
                None => rpcd::catalog(&SystemSource),
            };
            out!("{doc}");
            0
        }
        Some("--templates") => {
            for d in mqtt_metrics::catalog::ALL {
                out!(
                    "{}",
                    serde_json::json!({
                        "id": d.id,
                        "group": d.group.id(),
                        "component": d.kind.component(),
                        "object": d.object,
                        "topic": d.topic,
                        "name": d.name,
                        "unit": d.unit,
                        "device_class": d.device_class,
                        "state_class": d.state_class,
                        "diagnostic": d.diagnostic,
                        "legacy": d.legacy,
                        "recommended": d.recommended,
                        "default_interval": d.default_interval,
                    })
                );
            }
            0
        }
        Some("--import-legacy") => {
            let script = args.get(1).map_or(legacy::SCRIPT, String::as_str);
            match legacy::import(std::path::Path::new(script)) {
                Ok(b) => {
                    out!(
                        "imported broker {}:{} user {} (password {})",
                        b.host,
                        b.port
                            .map_or_else(|| "default".to_string(), |p| p.to_string()),
                        b.username.as_deref().unwrap_or("-"),
                        if b.password.is_some() {
                            "set"
                        } else {
                            "not set"
                        }
                    );
                    0
                }
                Err(e) => {
                    eprintln!("{e}");
                    1
                }
            }
        }
        Some("--purge") => daemon::purge_once(),
        Some("--version") => {
            out!("mqtt-metricsd {}", mqtt_metrics::VERSION);
            0
        }
        Some("--help" | "-h") => {
            out!("{USAGE}");
            0
        }
        Some(_) => {
            eprintln!("{USAGE}");
            2
        }
    };
    std::process::exit(code);
}
