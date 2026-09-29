// SPDX-License-Identifier: GPL-2.0-or-later
//! TLS transport against an in-process rustls server with a certificate
//! generated at test time (no key material in the repository).
#![cfg(feature = "tls")]

use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use mqtt_metrics::mqtt::transport::TlsSettings;
use mqtt_metrics::mqtt::{Client, Options};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

struct Server {
    port: u16,
    ca_pem: String,
}

/// One-connection MQTT-over-TLS server that answers CONNECT with CONNACK.
fn server() -> Server {
    let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let ca_pem = cert.cert.pem();
    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(cert.key_pair.serialize_der()));
    let config = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .unwrap()
    .with_no_client_auth()
    .with_single_cert(vec![cert_der], key_der)
    .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let config = Arc::new(config);
    thread::spawn(move || {
        while let Ok((tcp, _)) = listener.accept() {
            let conn = rustls::ServerConnection::new(Arc::clone(&config)).unwrap();
            let mut tls = rustls::StreamOwned::new(conn, tcp);
            let mut header = [0u8; 2];
            if tls.read_exact(&mut header).is_err() {
                continue;
            }
            let mut body = vec![0u8; usize::from(header[1])];
            if tls.read_exact(&mut body).is_ok() && header[0] == 0x10 {
                let _ = tls.write_all(&[0x20, 2, 0, 0]);
                let _ = tls.flush();
                let mut rest = [0u8; 64];
                let _ = tls.read(&mut rest);
            }
        }
    });
    Server { port, ca_pem }
}

fn options(port: u16, tls: TlsSettings) -> Options {
    Options {
        host: "localhost".into(),
        port,
        client_id: "tls-test".into(),
        keepalive: 30,
        username: None,
        password: None,
        will: None,
        connect_timeout: Duration::from_secs(5),
        tls: Some(tls),
    }
}

fn ca_file(pem: &str, test: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "mqtt-metrics-tls-ca-{}-{test}.pem",
        std::process::id()
    ));
    std::fs::write(&path, pem).unwrap();
    path.to_string_lossy().into_owned()
}

#[test]
fn verified_connection_with_ca_file() {
    let s = server();
    let settings = TlsSettings {
        ca_file: Some(ca_file(&s.ca_pem, "verified")),
        verify: true,
        cert_file: None,
        key_file: None,
    };
    let client = Client::connect(&options(s.port, settings)).expect("TLS connect");
    client.disconnect();
}

#[test]
fn unknown_issuer_is_rejected() {
    let s = server();
    let other = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let settings = TlsSettings {
        ca_file: Some(ca_file(&other.cert.pem(), "unknown")),
        verify: true,
        cert_file: None,
        key_file: None,
    };
    let err = Client::connect(&options(s.port, settings))
        .err()
        .expect("must fail");
    assert!(
        err.to_string().to_lowercase().contains("certificate"),
        "{err}"
    );
}

#[test]
fn unverified_mode_accepts_self_signed() {
    let s = server();
    let settings = TlsSettings {
        ca_file: None,
        verify: false,
        cert_file: None,
        key_file: None,
    };
    Client::connect(&options(s.port, settings))
        .expect("TLS connect")
        .disconnect();
}

#[test]
fn a_malformed_key_file_is_never_quoted_in_the_error() {
    // The PEM parser quotes the line it rejects. When the header shares a
    // line with the key, that line is the key.
    let s = server();
    let pair = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
    let secret = "MIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgKEYMATERIAL";
    let settings = TlsSettings {
        ca_file: None,
        verify: false,
        cert_file: Some(ca_file(&pair.cert.pem(), "client-cert")),
        key_file: Some(ca_file(
            &format!("-----BEGIN PRIVATE KEY----- {secret}\n-----END PRIVATE KEY-----\n"),
            "client-key",
        )),
    };
    let err = Client::connect(&options(s.port, settings))
        .err()
        .expect("must fail");
    let text = err.to_string();
    assert!(text.contains("client-key"), "{text}");
    // The parser prints the rejected line as a list of byte values.
    assert!(
        text.ends_with("not a valid PEM private key"),
        "the error may quote the key: {text}"
    );
}
