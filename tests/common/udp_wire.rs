// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I85 — Engine-host kernel natives (the sealed datagram a test client speaks to the kernel)

//! The engine host's state-sync datagram, for a test that plays a native client of the kernel
//! (`tests/engine_host_udp.rs`, `tests/c_emit.rs`).  Written here from its description
//! (`src/engine_host.rs` § the seal, `src/c/loft_engine.h`) rather than borrowed from either
//! kernel, so a kernel that drifts from it fails the test:
//!
//! `kind | cid (u32 BE) | seq (u64 BE) | payload | tag`, the tag the first 16 bytes of
//! HMAC-SHA256 under the connection's session key over a direction byte (`c` toward the
//! listener, `s` toward the client) and everything before the tag.  The key and cid arrive in
//! the 101 response: `X-Loft-UDP: <cid>:<64 hex digits>`.

#![allow(dead_code)]

use sha2::{Digest, Sha256};
use std::cell::Cell;

pub const TO_LISTENER: u8 = b'c';
pub const TO_CLIENT: u8 = b's';

fn hmac(key: &[u8; 32], msg: &[u8]) -> [u8; 32] {
    let mut block = [0u8; 64];
    block[..32].copy_from_slice(key);
    let inner: Vec<u8> = block
        .iter()
        .map(|b| b ^ 0x36)
        .chain(msg.iter().copied())
        .collect();
    let ih = Sha256::digest(&inner);
    let outer: Vec<u8> = block
        .iter()
        .map(|b| b ^ 0x5c)
        .chain(ih.iter().copied())
        .collect();
    Sha256::digest(&outer).into()
}

pub fn seal(key: &[u8; 32], dir: u8, kind: u8, cid: u32, seq: u64, payload: &[u8]) -> Vec<u8> {
    let mut d = vec![kind];
    d.extend_from_slice(&cid.to_be_bytes());
    d.extend_from_slice(&seq.to_be_bytes());
    d.extend_from_slice(payload);
    let mut m = vec![dir];
    m.extend_from_slice(&d);
    d.extend_from_slice(&hmac(key, &m)[..16]);
    d
}

/// `(kind, cid, seq, payload)` of a datagram whose tag holds.
pub fn unseal(key: &[u8; 32], dir: u8, d: &[u8]) -> Option<(u8, u32, u64, Vec<u8>)> {
    if d.len() < 29 {
        return None;
    }
    let (body, tag) = d.split_at(d.len() - 16);
    let mut m = vec![dir];
    m.extend_from_slice(body);
    if hmac(key, &m)[..16] != *tag {
        return None;
    }
    let cid = u32::from_be_bytes(body[1..5].try_into().unwrap());
    let seq = u64::from_be_bytes(body[5..13].try_into().unwrap());
    Some((body[0], cid, seq, body[13..].to_vec()))
}

/// What the 101 response hands the client for its UDP path: its cid and session key.
pub struct Seat {
    pub cid: u32,
    pub key: [u8; 32],
    /// The client's last outbound seq: one space for every datagram it sends.
    pub seq: Cell<u64>,
}

impl Seat {
    /// The `X-Loft-UDP` value `<cid>:<64 hex digits>`.
    pub fn of(header: &str) -> Seat {
        let (cid, hex) = header
            .split_once(':')
            .unwrap_or_else(|| panic!("`<cid>:<key>`: {header:?}"));
        assert_eq!(hex.len(), 64, "a 32-byte session key: {header:?}");
        let mut key = [0u8; 32];
        for (i, b) in key.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).expect("hex key");
        }
        Seat {
            cid: cid.parse().expect("numeric cid"),
            key,
            seq: Cell::new(0),
        }
    }

    fn next(&self) -> u64 {
        self.seq.set(self.seq.get() + 1);
        self.seq.get()
    }

    /// The client's hello.
    pub fn hello(&self) -> Vec<u8> {
        seal(&self.key, TO_LISTENER, b'H', self.cid, self.next(), b"")
    }

    /// The client's keepalive.
    pub fn keepalive(&self) -> Vec<u8> {
        seal(&self.key, TO_LISTENER, b'K', self.cid, self.next(), b"")
    }

    /// A sync sample the client sends with seq `seq`.
    pub fn sync(&self, seq: u64, payload: &str) -> Vec<u8> {
        self.seq.set(self.seq.get().max(seq));
        seal(
            &self.key,
            TO_LISTENER,
            b'S',
            self.cid,
            seq,
            payload.as_bytes(),
        )
    }

    /// A datagram of `kind` written by someone without the session key, every way the forger
    /// has: sealed under another key, sealed for the other direction (a reflection of the
    /// server's own datagram), and with no tag at all.
    pub fn forged(&self, kind: u8, seq: u64, payload: &str) -> Vec<Vec<u8>> {
        let other = [0x55u8; 32];
        let p = payload.as_bytes();
        let mut untagged = seal(&self.key, TO_LISTENER, kind, self.cid, seq, p);
        untagged.truncate(untagged.len() - 16);
        vec![
            seal(&other, TO_LISTENER, kind, self.cid, seq, p),
            seal(&self.key, TO_CLIENT, kind, self.cid, seq, p),
            untagged,
        ]
    }

    /// A datagram from the server: `(kind, seq, payload)`, `None` when it does not open.
    pub fn open(&self, d: &[u8]) -> Option<(char, i64, String)> {
        let (kind, cid, seq, payload) = unseal(&self.key, TO_CLIENT, d)?;
        assert_eq!(cid, self.cid, "the server's datagram names the seat's cid");
        Some((
            kind as char,
            i64::try_from(seq).ok()?,
            String::from_utf8(payload).ok()?,
        ))
    }
}
