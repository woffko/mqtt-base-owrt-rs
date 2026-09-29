// SPDX-License-Identifier: GPL-2.0-or-later
//! MQTT client against an in-process fake broker.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use mqtt_metrics::mqtt::codec::{self, Packet};
use mqtt_metrics::mqtt::{Client, MqttError, Options, WillMessage};

/// One raw frame: packet type, flags, body.
fn read_frame(s: &mut TcpStream) -> Option<(u8, u8, Vec<u8>)> {
    let mut b = [0u8; 1];
    s.read_exact(&mut b).ok()?;
    let header = b[0];
    let mut len = 0usize;
    let mut mult = 1usize;
    loop {
        s.read_exact(&mut b).ok()?;
        len += usize::from(b[0] & 0x7F) * mult;
        if b[0] & 0x80 == 0 {
            break;
        }
        mult *= 128;
    }
    let mut body = vec![0u8; len];
    s.read_exact(&mut body).ok()?;
    Some((header >> 4, header & 0x0F, body))
}

fn field(body: &[u8], pos: &mut usize) -> Vec<u8> {
    let len = usize::from(u16::from_be_bytes([body[*pos], body[*pos + 1]]));
    let v = body[*pos + 2..*pos + 2 + len].to_vec();
    *pos += 2 + len;
    v
}

#[derive(Debug, Default, Clone)]
struct ConnectInfo {
    client_id: String,
    will_topic: Option<String>,
    will_payload: Option<String>,
    will_retain: bool,
    username: Option<String>,
    password: Option<String>,
}

fn parse_connect(body: &[u8]) -> ConnectInfo {
    let mut pos = 0;
    assert_eq!(field(body, &mut pos), b"MQTT");
    assert_eq!(body[pos], 4);
    let flags = body[pos + 1];
    pos += 4;
    let mut info = ConnectInfo {
        client_id: String::from_utf8(field(body, &mut pos)).unwrap(),
        ..ConnectInfo::default()
    };
    if flags & 0x04 != 0 {
        info.will_topic = Some(String::from_utf8(field(body, &mut pos)).unwrap());
        info.will_payload = Some(String::from_utf8(field(body, &mut pos)).unwrap());
        info.will_retain = flags & 0x20 != 0;
    }
    if flags & 0x80 != 0 {
        info.username = Some(String::from_utf8(field(body, &mut pos)).unwrap());
    }
    if flags & 0x40 != 0 {
        info.password = Some(String::from_utf8(field(body, &mut pos)).unwrap());
    }
    info
}

#[derive(Debug)]
enum Event {
    Connect(ConnectInfo),
    Publish {
        topic: String,
        payload: Vec<u8>,
        retain: bool,
        qos: u8,
    },
    Subscribe(String),
    Ping,
    Disconnect,
}

/// Behaviour switches for the fake broker.
#[derive(Clone, Copy)]
struct Behaviour {
    connack_code: u8,
    ack_publishes: bool,
    /// Return code granted in SUBACK (0x80 = failure, e.g. denied by an ACL).
    suback_code: u8,
}

fn broker(behaviour: Behaviour) -> (u16, mpsc::Receiver<Event>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let Ok((mut s, _)) = listener.accept() else {
            return;
        };
        while let Some((kind, flags, body)) = read_frame(&mut s) {
            match kind {
                1 => {
                    let _ = tx.send(Event::Connect(parse_connect(&body)));
                    s.write_all(&[0x20, 2, 0, behaviour.connack_code]).unwrap();
                    if behaviour.connack_code != 0 {
                        return;
                    }
                }
                3 => {
                    let (packet, _) = codec::decode(
                        &[&[0x30 | flags][..], &encode_len(body.len()), &body].concat(),
                        1 << 20,
                    )
                    .unwrap()
                    .unwrap();
                    if let Packet::Publish {
                        topic,
                        payload,
                        qos,
                        retain,
                        packet_id,
                    } = packet
                    {
                        if qos == 1 && behaviour.ack_publishes {
                            let id = packet_id.unwrap().to_be_bytes();
                            s.write_all(&[0x40, 2, id[0], id[1]]).unwrap();
                        }
                        let _ = tx.send(Event::Publish {
                            topic,
                            payload,
                            retain,
                            qos,
                        });
                    }
                }
                8 => {
                    let id = [body[0], body[1]];
                    let mut pos = 2;
                    let filter = String::from_utf8(field(&body, &mut pos)).unwrap();
                    s.write_all(&[0x90, 3, id[0], id[1], behaviour.suback_code])
                        .unwrap();
                    // Retained birth (must be ignored by the daemon) and a live one.
                    let mut out = Vec::new();
                    codec::encode_publish(&filter, b"online", 0, true, None, &mut out).unwrap();
                    codec::encode_publish(&filter, b"online", 0, false, None, &mut out).unwrap();
                    s.write_all(&out).unwrap();
                    let _ = tx.send(Event::Subscribe(filter));
                }
                12 => {
                    s.write_all(&[0xD0, 0]).unwrap();
                    let _ = tx.send(Event::Ping);
                }
                14 => {
                    let _ = tx.send(Event::Disconnect);
                    return;
                }
                _ => {}
            }
        }
    });
    (port, rx)
}

fn encode_len(mut len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut b = u8::try_from(len % 128).unwrap();
        len /= 128;
        if len > 0 {
            b |= 0x80;
        }
        out.push(b);
        if len == 0 {
            return out;
        }
    }
}

fn options(port: u16) -> Options {
    Options {
        host: "127.0.0.1".into(),
        port,
        client_id: "mqtt-metrics-test".into(),
        keepalive: 2,
        username: Some("user".into()),
        password: Some("secret".into()),
        will: Some(WillMessage {
            topic: "openwrt/r1/availability".into(),
            payload: "offline".into(),
            retain: true,
        }),
        connect_timeout: Duration::from_secs(5),
        tls: None,
    }
}

const WAIT: Duration = Duration::from_secs(5);

#[test]
fn connect_publish_subscribe_ping_disconnect() {
    let (port, events) = broker(Behaviour {
        connack_code: 0,
        ack_publishes: true,
        suback_code: 0,
    });
    let mut client = Client::connect(&options(port)).unwrap();
    match events.recv_timeout(WAIT).unwrap() {
        Event::Connect(info) => {
            assert_eq!(info.client_id, "mqtt-metrics-test");
            assert_eq!(info.username.as_deref(), Some("user"));
            assert_eq!(info.password.as_deref(), Some("secret"));
            assert_eq!(info.will_topic.as_deref(), Some("openwrt/r1/availability"));
            assert_eq!(info.will_payload.as_deref(), Some("offline"));
            assert!(info.will_retain);
        }
        other => panic!("{other:?}"),
    }

    client
        .publish("openwrt/r1/availability", b"online", 1, true)
        .unwrap();
    for i in 0..100 {
        client
            .publish(&format!("openwrt/r1/m{i}"), b"1", 1, true)
            .unwrap();
    }
    client.drain(WAIT).unwrap();
    assert_eq!(client.inflight(), 0);
    let mut published = 0;
    while published < 101 {
        match events.recv_timeout(WAIT).unwrap() {
            Event::Publish {
                topic,
                payload,
                qos,
                retain,
            } => {
                assert_eq!((qos, retain), (1, true));
                if published == 0 {
                    assert_eq!(
                        (topic.as_str(), payload.as_slice()),
                        ("openwrt/r1/availability", &b"online"[..])
                    );
                }
                published += 1;
            }
            other => panic!("{other:?}"),
        }
    }

    client.subscribe("homeassistant/status").unwrap();
    let mut received = Vec::new();
    for _ in 0..20 {
        received.extend(client.poll(Duration::from_millis(100)).unwrap());
        if received.len() >= 2 {
            break;
        }
    }
    assert_eq!(received.len(), 2);
    assert!(received[0].retain && !received[1].retain);
    assert_eq!(received[1].payload, b"online");
    assert!(
        matches!(events.recv_timeout(WAIT).unwrap(), Event::Subscribe(f) if f == "homeassistant/status")
    );

    // Keepalive 2 s: after 1 s idle the client pings.
    thread::sleep(Duration::from_millis(1100));
    client.maintain().unwrap();
    assert!(matches!(events.recv_timeout(WAIT).unwrap(), Event::Ping));
    client.poll(Duration::from_millis(200)).unwrap();
    client.maintain().unwrap();

    client.disconnect();
    assert!(matches!(
        events.recv_timeout(WAIT).unwrap(),
        Event::Disconnect
    ));
}

#[test]
fn refused_credentials() {
    let (port, _events) = broker(Behaviour {
        connack_code: 5,
        ack_publishes: true,
        suback_code: 0,
    });
    match Client::connect(&options(port)) {
        Err(MqttError::Refused(5)) => {}
        Err(e) => panic!("unexpected error {e}"),
        Ok(_) => panic!("connection must be refused"),
    }
}

#[test]
fn missing_acks_are_detected() {
    let (port, _events) = broker(Behaviour {
        connack_code: 0,
        ack_publishes: false,
        suback_code: 0,
    });
    let mut client = Client::connect(&options(port)).unwrap();
    client.publish("t", b"x", 1, false).unwrap();
    assert!(matches!(
        client.drain(Duration::from_millis(500)),
        Err(MqttError::Timeout("PUBACK"))
    ));
}

#[test]
fn unreachable_broker_fails_fast() {
    // Bind and drop to get a closed local port.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let started = std::time::Instant::now();
    assert!(Client::connect(&options(port)).is_err());
    assert!(started.elapsed() < Duration::from_secs(6));
}

#[test]
fn a_denied_subscription_does_not_drop_the_connection() {
    let (port, _events) = broker(Behaviour {
        connack_code: 0,
        ack_publishes: true,
        suback_code: 0x80,
    });
    let mut client = Client::connect(&options(port)).unwrap();
    client.subscribe("homeassistant/status").unwrap();
    for _ in 0..10 {
        // A broker ACL that denies the birth topic must not cost us the
        // connection: publishing is what matters.
        client
            .poll(Duration::from_millis(100))
            .expect("a denied subscription is not a connection error");
    }
    assert!(client.subscription_denied);
    client.publish("openwrt/r1/x", b"1", 1, true).unwrap();
    client.drain(WAIT).unwrap();
}
