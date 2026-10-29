// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I85 — Engine-host kernel natives (the frame codec a test client speaks to loft's servers)

//! RFC 6455 frames over any byte stream, for a test that plays the client of a loft server
//! (`tests/server_natives.rs`).  loft's own codec is C, `loft_ws_write_frame` /
//! `loft_ws_read_frame` in `src/c/loft_https.h`.
//!
//! A read refuses a frame longer than the caller's cap BEFORE allocating it.

#![allow(dead_code)]

use std::io::{ErrorKind, Read, Write};

pub const OP_TEXT: u8 = 0x01;
pub const OP_BINARY: u8 = 0x02;
pub const OP_CLOSE: u8 = 0x08;
pub const OP_PING: u8 = 0x09;
pub const OP_PONG: u8 = 0x0A;

/// What one read found.
pub enum Read1 {
    /// The stream timed out or would block before the first header byte: nothing yet.
    NoData,
    /// A whole frame: its opcode and unmasked payload.
    Frame(u8, Vec<u8>),
    /// The peer is gone: an end, a failure, a partial frame or a length over the cap.
    Closed,
}

impl Read1 {
    /// The frame, if one was read.
    #[must_use]
    pub fn frame(self) -> Option<(u8, Vec<u8>)> {
        match self {
            Read1::Frame(op, p) => Some((op, p)),
            Read1::NoData | Read1::Closed => None,
        }
    }
}

/// One frame of at most `max` payload bytes.
pub fn read<S: Read>(s: &mut S, max: u64) -> Read1 {
    let mut h = [0u8; 2];
    if let Err(e) = s.read_exact(&mut h) {
        return match e.kind() {
            ErrorKind::WouldBlock | ErrorKind::TimedOut => Read1::NoData,
            _ => Read1::Closed,
        };
    }
    let mut len = u64::from(h[1] & 0x7F);
    if len == 126 {
        let mut b = [0u8; 2];
        if s.read_exact(&mut b).is_err() {
            return Read1::Closed;
        }
        len = u64::from(u16::from_be_bytes(b));
    } else if len == 127 {
        let mut b = [0u8; 8];
        if s.read_exact(&mut b).is_err() {
            return Read1::Closed;
        }
        len = u64::from_be_bytes(b);
    }
    if len > max {
        return Read1::Closed;
    }
    let mut mask = None;
    if h[1] & 0x80 != 0 {
        let mut m = [0u8; 4];
        if s.read_exact(&mut m).is_err() {
            return Read1::Closed;
        }
        mask = Some(m);
    }
    let Ok(n) = usize::try_from(len) else {
        return Read1::Closed;
    };
    let mut payload = vec![0u8; n];
    if s.read_exact(&mut payload).is_err() {
        return Read1::Closed;
    }
    if let Some(m) = mask {
        for (i, b) in payload.iter_mut().enumerate() {
            *b ^= m[i % 4];
        }
    }
    Read1::Frame(h[0] & 0x0F, payload)
}

/// One final frame, masked with `mask` when given (a client's frames must be, a server's must
/// not be), written in one call: true when every byte went out.
pub fn write<S: Write>(s: &mut S, opcode: u8, payload: &[u8], mask: Option<[u8; 4]>) -> bool {
    let bit = if mask.is_some() { 0x80 } else { 0 };
    let mut frame = Vec::with_capacity(payload.len() + 14);
    frame.push(0x80 | opcode);
    let len = payload.len();
    if len < 126 {
        frame.push(bit | len as u8);
    } else if let Ok(short) = u16::try_from(len) {
        frame.push(bit | 0x7E);
        frame.extend_from_slice(&short.to_be_bytes());
    } else {
        frame.push(bit | 0x7F);
        frame.extend_from_slice(&(len as u64).to_be_bytes());
    }
    match mask {
        Some(m) => {
            frame.extend_from_slice(&m);
            frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ m[i % 4]));
        }
        None => frame.extend_from_slice(payload),
    }
    s.write_all(&frame).is_ok() && s.flush().is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_round_trips_masked_and_unmasked_at_every_length_form() {
        for len in [0usize, 125, 126, 65_535, 65_536] {
            let payload: Vec<u8> = (0..len).map(|i| (i * 7) as u8).collect();
            for mask in [None, Some([1, 2, 3, 4])] {
                let mut wire = Vec::new();
                assert!(write(&mut wire, OP_BINARY, &payload, mask));
                let got = read(&mut wire.as_slice(), 1 << 20).frame();
                assert_eq!(
                    got,
                    Some((OP_BINARY, payload.clone())),
                    "len {len} mask {mask:?}"
                );
            }
        }
    }

    #[test]
    fn a_length_over_the_cap_is_refused_before_it_is_allocated() {
        let mut wire = vec![0x82, 0x7F];
        wire.extend_from_slice(&u64::MAX.to_be_bytes());
        assert!(matches!(read(&mut wire.as_slice(), 1 << 20), Read1::Closed));
    }
}
