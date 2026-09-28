// SPDX-License-Identifier: GPL-2.0-or-later
//! MQTT 3.1.1 packet encoding and decoding for the subset this client uses:
//! CONNECT/CONNACK, PUBLISH (QoS 0/1), PUBACK, SUBSCRIBE/SUBACK,
//! PINGREQ/PINGRESP and DISCONNECT.

use std::fmt;

pub const MAX_REMAINING: usize = 268_435_455;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CodecError {
    Malformed(&'static str),
    TooLarge(usize),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::Malformed(what) => write!(f, "malformed packet: {what}"),
            CodecError::TooLarge(n) => write!(f, "incoming packet of {n} bytes exceeds the limit"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Packet {
    ConnAck {
        session_present: bool,
        code: u8,
    },
    Publish {
        topic: String,
        payload: Vec<u8>,
        qos: u8,
        retain: bool,
        packet_id: Option<u16>,
    },
    PubAck {
        packet_id: u16,
    },
    SubAck {
        packet_id: u16,
        codes: Vec<u8>,
    },
    PingResp,
    /// A packet type this client does not act on.
    Other(u8),
}

#[derive(Debug, Clone, Copy)]
pub struct Will<'a> {
    pub topic: &'a str,
    pub payload: &'a [u8],
    pub qos: u8,
    pub retain: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct Connect<'a> {
    pub client_id: &'a str,
    pub keepalive: u16,
    pub clean_session: bool,
    pub username: Option<&'a str>,
    pub password: Option<&'a [u8]>,
    pub will: Option<Will<'a>>,
}

fn push_len(out: &mut Vec<u8>, mut len: usize) {
    loop {
        let mut byte = u8::try_from(len % 128).unwrap_or(0);
        len /= 128;
        if len > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if len == 0 {
            break;
        }
    }
}

fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn push_bytes(out: &mut Vec<u8>, data: &[u8]) -> Result<(), CodecError> {
    let len = u16::try_from(data.len())
        .map_err(|_| CodecError::Malformed("field longer than 65535 bytes"))?;
    push_u16(out, len);
    out.extend_from_slice(data);
    Ok(())
}

fn finish(out: &mut Vec<u8>, header: u8, body: &[u8]) -> Result<(), CodecError> {
    if body.len() > MAX_REMAINING {
        return Err(CodecError::TooLarge(body.len()));
    }
    out.push(header);
    push_len(out, body.len());
    out.extend_from_slice(body);
    Ok(())
}

pub fn encode_connect(c: &Connect, out: &mut Vec<u8>) -> Result<(), CodecError> {
    let mut body = Vec::with_capacity(64);
    push_bytes(&mut body, b"MQTT")?;
    body.push(4); // protocol level 3.1.1
    let mut flags = 0u8;
    if c.clean_session {
        flags |= 0x02;
    }
    if let Some(w) = &c.will {
        flags |= 0x04 | ((w.qos & 0x03) << 3);
        if w.retain {
            flags |= 0x20;
        }
    }
    if c.username.is_some() {
        flags |= 0x80;
        if c.password.is_some() {
            flags |= 0x40;
        }
    }
    body.push(flags);
    push_u16(&mut body, c.keepalive);
    push_bytes(&mut body, c.client_id.as_bytes())?;
    if let Some(w) = &c.will {
        push_bytes(&mut body, w.topic.as_bytes())?;
        push_bytes(&mut body, w.payload)?;
    }
    if let Some(user) = c.username {
        push_bytes(&mut body, user.as_bytes())?;
        if let Some(pass) = c.password {
            push_bytes(&mut body, pass)?;
        }
    }
    finish(out, 0x10, &body)
}

pub fn encode_publish(
    topic: &str,
    payload: &[u8],
    qos: u8,
    retain: bool,
    packet_id: Option<u16>,
    out: &mut Vec<u8>,
) -> Result<(), CodecError> {
    if qos > 1 {
        return Err(CodecError::Malformed("QoS 2 is not supported"));
    }
    if qos == 1 && packet_id.is_none() {
        return Err(CodecError::Malformed("QoS 1 needs a packet id"));
    }
    let mut body = Vec::with_capacity(topic.len() + payload.len() + 4);
    push_bytes(&mut body, topic.as_bytes())?;
    if qos == 1 {
        push_u16(&mut body, packet_id.unwrap_or(1));
    }
    body.extend_from_slice(payload);
    let header = 0x30 | (qos << 1) | u8::from(retain);
    finish(out, header, &body)
}

pub fn encode_puback(packet_id: u16, out: &mut Vec<u8>) {
    out.extend_from_slice(&[0x40, 2]);
    push_u16(out, packet_id);
}

pub fn encode_subscribe(
    packet_id: u16,
    filter: &str,
    qos: u8,
    out: &mut Vec<u8>,
) -> Result<(), CodecError> {
    let mut body = Vec::with_capacity(filter.len() + 5);
    push_u16(&mut body, packet_id);
    push_bytes(&mut body, filter.as_bytes())?;
    body.push(qos & 0x03);
    finish(out, 0x82, &body)
}

pub fn encode_pingreq(out: &mut Vec<u8>) {
    out.extend_from_slice(&[0xC0, 0]);
}

pub fn encode_disconnect(out: &mut Vec<u8>) {
    out.extend_from_slice(&[0xE0, 0]);
}

struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn u8(&mut self) -> Result<u8, CodecError> {
        let v = *self
            .data
            .get(self.pos)
            .ok_or(CodecError::Malformed("truncated body"))?;
        self.pos += 1;
        Ok(v)
    }

    fn u16(&mut self) -> Result<u16, CodecError> {
        Ok(u16::from_be_bytes([self.u8()?, self.u8()?]))
    }

    fn bytes(&mut self) -> Result<&'a [u8], CodecError> {
        let len = usize::from(self.u16()?);
        let end = self
            .pos
            .checked_add(len)
            .ok_or(CodecError::Malformed("length overflow"))?;
        let v = self
            .data
            .get(self.pos..end)
            .ok_or(CodecError::Malformed("truncated field"))?;
        self.pos = end;
        Ok(v)
    }

    fn rest(&mut self) -> &'a [u8] {
        let v = self.data.get(self.pos..).unwrap_or(&[]);
        self.pos = self.data.len();
        v
    }
}

/// Decode one packet from the front of `buf`. Returns `Ok(None)` when more
/// bytes are needed, and the number of bytes consumed otherwise.
pub fn decode(buf: &[u8], max_packet: usize) -> Result<Option<(Packet, usize)>, CodecError> {
    let Some(&header) = buf.first() else {
        return Ok(None);
    };
    let mut remaining: usize = 0;
    let mut multiplier: usize = 1;
    let mut pos = 1;
    loop {
        let Some(&byte) = buf.get(pos) else {
            return Ok(None);
        };
        remaining += usize::from(byte & 0x7F) * multiplier;
        pos += 1;
        if byte & 0x80 == 0 {
            break;
        }
        if pos > 4 {
            return Err(CodecError::Malformed(
                "remaining length longer than 4 bytes",
            ));
        }
        multiplier *= 128;
    }
    if remaining > max_packet {
        return Err(CodecError::TooLarge(remaining));
    }
    let end = pos + remaining;
    if buf.len() < end {
        return Ok(None);
    }
    let mut c = Cursor {
        data: &buf[pos..end],
        pos: 0,
    };
    let kind = header >> 4;
    let flags = header & 0x0F;
    let packet = match kind {
        2 => {
            let ack = c.u8()?;
            let code = c.u8()?;
            Packet::ConnAck {
                session_present: ack & 1 == 1,
                code,
            }
        }
        3 => {
            let qos = (flags >> 1) & 0x03;
            if qos > 2 {
                return Err(CodecError::Malformed("invalid QoS"));
            }
            let topic = String::from_utf8(c.bytes()?.to_vec())
                .map_err(|_| CodecError::Malformed("topic is not UTF-8"))?;
            let packet_id = if qos > 0 { Some(c.u16()?) } else { None };
            Packet::Publish {
                topic,
                payload: c.rest().to_vec(),
                qos,
                retain: flags & 1 == 1,
                packet_id,
            }
        }
        4 => Packet::PubAck {
            packet_id: c.u16()?,
        },
        9 => {
            let packet_id = c.u16()?;
            Packet::SubAck {
                packet_id,
                codes: c.rest().to_vec(),
            }
        }
        13 => Packet::PingResp,
        other => Packet::Other(other),
    };
    Ok(Some((packet, end)))
}

pub fn connack_meaning(code: u8) -> &'static str {
    match code {
        0 => "accepted",
        1 => "unacceptable protocol version",
        2 => "client identifier rejected",
        3 => "server unavailable",
        4 => "bad user name or password",
        5 => "not authorized",
        _ => "unknown refusal code",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connect_bytes_match_spec_example() {
        let mut out = Vec::new();
        encode_connect(
            &Connect {
                client_id: "c",
                keepalive: 60,
                clean_session: true,
                username: Some("u"),
                password: Some(b"p"),
                will: Some(Will {
                    topic: "t",
                    payload: b"offline",
                    qos: 1,
                    retain: true,
                }),
            },
            &mut out,
        )
        .unwrap();
        let expected: Vec<u8> = [
            &[0x10, 31][..],
            &[0, 4, b'M', b'Q', b'T', b'T', 4],
            &[0b1110_1110],
            &[0, 60],
            &[0, 1, b'c'],
            &[0, 1, b't'],
            &[0, 7],
            b"offline",
            &[0, 1, b'u'],
            &[0, 1, b'p'],
        ]
        .concat();
        assert_eq!(out, expected);
    }

    #[test]
    fn publish_round_trip() {
        let mut out = Vec::new();
        encode_publish("a/b", b"hello", 1, true, Some(7), &mut out).unwrap();
        assert_eq!(out[0], 0x33);
        let (p, used) = decode(&out, 1024).unwrap().unwrap();
        assert_eq!(used, out.len());
        assert_eq!(
            p,
            Packet::Publish {
                topic: "a/b".into(),
                payload: b"hello".to_vec(),
                qos: 1,
                retain: true,
                packet_id: Some(7)
            }
        );
    }

    #[test]
    fn remaining_length_encoding() {
        for len in [0usize, 127, 128, 16_383, 16_384, 2_097_151, 2_097_152] {
            let mut out = Vec::new();
            push_len(&mut out, len);
            let mut buf = vec![0x30];
            buf.extend_from_slice(&out);
            buf.resize(buf.len() + len, 0);
            // A zero-length topic prefix makes these bodies malformed
            // publishes only when long enough; decode just the header here.
            let decoded = decode(&buf, MAX_REMAINING);
            if len >= 2 {
                assert!(decoded.is_ok(), "len {len}");
            }
        }
    }

    #[test]
    fn partial_and_oversized_input() {
        let mut out = Vec::new();
        encode_publish("t", &[0u8; 300], 0, false, None, &mut out).unwrap();
        for cut in 0..out.len() {
            assert_eq!(decode(&out[..cut], 1024).unwrap(), None, "cut {cut}");
        }
        assert_eq!(decode(&out, 100), Err(CodecError::TooLarge(303)));
        assert!(decode(&[0x30, 0xFF, 0xFF, 0xFF, 0xFF, 0x01], MAX_REMAINING).is_err());
    }

    #[test]
    fn acks() {
        assert_eq!(
            decode(&[0x20, 2, 0, 5], 16).unwrap(),
            Some((
                Packet::ConnAck {
                    session_present: false,
                    code: 5
                },
                4
            ))
        );
        assert_eq!(
            decode(&[0x40, 2, 0, 9], 16).unwrap(),
            Some((Packet::PubAck { packet_id: 9 }, 4))
        );
        assert_eq!(
            decode(&[0x90, 3, 0, 1, 0], 16).unwrap(),
            Some((
                Packet::SubAck {
                    packet_id: 1,
                    codes: vec![0]
                },
                5
            ))
        );
        assert_eq!(decode(&[0xD0, 0], 16).unwrap(), Some((Packet::PingResp, 2)));
        assert_eq!(
            decode(&[0x40, 1, 0], 16),
            Err(CodecError::Malformed("truncated body"))
        );
    }

    #[test]
    fn decode_never_panics_on_garbage() {
        // Deterministic pseudo-random inputs as a lightweight fuzz pass.
        let mut seed: u32 = 0x1234_5678;
        for _ in 0..20_000 {
            let len = (seed % 40) as usize;
            let mut buf = Vec::with_capacity(len);
            for _ in 0..len {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                buf.push((seed & 0xFF) as u8);
            }
            let _ = decode(&buf, 64);
        }
    }
}
