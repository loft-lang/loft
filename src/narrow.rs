// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The encoding of a narrow integer at rest — the bytes a `u8`, `i8`, `u16`, `i16`, `i32`,
//! `u32` or `limit(lo, hi)` value occupies in a store, spelled once for both backends.
//!
//! A store field holds a narrow integer biased by the type's minimum, with one code kept
//! back for absence where the slot is nullable (`data::NarrowIntKind` names the kinds; the
//! `Store` getters and setters read and write them in place).  A LINKED narrow local —
//! one a `&` bind, a `&` argument or a re-point names (@PLN167 decision 1) — holds the same
//! bytes, so that one `&u8` pointer or `DbRef` reads a local and a field alike
//! (`@FR-B-Ref-Uniform`).  Native keeps such a local in a Rust variable of the storage
//! width and calls these functions at every read and write; the interpreter's
//! `OpVarNarrow` / `OpPutNarrow` call them on the frame slot's bytes.
//!
//! `i64::MIN` is the language's null in a wide value; each kind's absent CODE is what the
//! store writes for it.  A non-nullable kind has no code for absence and never sees one.

/// The kinds as the interpreter's op operand spells them — `NarrowIntKind::code`.
pub const BYTE: u8 = 0;
pub const BYTE_NULLABLE: u8 = 1;
pub const SHORT_RAW: u8 = 2;
pub const SHORT: u8 = 3;
pub const SHORT_FULL: u8 = 4;
pub const INT4: u8 = 5;
pub const INT4_RAW: u8 = 6;
pub const INT4_FULL: u8 = 7;

/// The storage width of a kind, in bytes.
#[must_use]
pub const fn width(kind: u8) -> u32 {
    match kind {
        BYTE | BYTE_NULLABLE => 1,
        SHORT_RAW | SHORT | SHORT_FULL => 2,
        _ => 4,
    }
}

// --- one byte ---

/// A non-nullable byte: the value biased by the type's minimum.
#[must_use]
pub fn enc_byte(v: i64, min: i32) -> u8 {
    (v - i64::from(min)) as u8
}
#[must_use]
pub fn dec_byte(b: u8, min: i32) -> i64 {
    i64::from(b) + i64::from(min)
}

/// A nullable byte: `255` is absence, as `Store::set_byte` writes it.
#[must_use]
pub fn enc_byte_nullable(v: i64, min: i32) -> u8 {
    if v == i64::MIN { 255 } else { enc_byte(v, min) }
}
#[must_use]
pub fn dec_byte_nullable(b: u8, min: i32) -> i64 {
    if b == 255 { i64::MIN } else { dec_byte(b, min) }
}

// --- two bytes ---

/// A nullable short: the value shifted by one so that `0` is absence
/// (`Store::set_short` / `get_short`).
#[must_use]
pub fn enc_short(v: i64, min: i32) -> u16 {
    if v == i64::MIN {
        0
    } else {
        (v - i64::from(min) + 1) as u16
    }
}
#[must_use]
pub fn dec_short(s: u16, min: i32) -> i64 {
    if s == 0 {
        i64::MIN
    } else {
        i64::from(s) + i64::from(min) - 1
    }
}

/// A non-nullable short: the full 65536 codes are values (`Store::get_short_full`).
#[must_use]
pub fn enc_short_full(v: i64, min: i32) -> u16 {
    (v - i64::from(min)) as u16
}
#[must_use]
pub fn dec_short_full(s: u16, min: i32) -> i64 {
    i64::from(s) + i64::from(min)
}

/// A narrow-vector short: direct encoding with `u16::MAX` for absence
/// (`Store::set_i16_raw`).  A local never takes this kind; it is here so every kind the
/// op operand can name decodes.
#[must_use]
pub fn enc_short_raw(v: i64, min: i32) -> u16 {
    if v == i64::MIN {
        u16::MAX
    } else {
        (v - i64::from(min)) as u16
    }
}
#[must_use]
pub fn dec_short_raw(s: u16, min: i32) -> i64 {
    if s == u16::MAX {
        i64::MIN
    } else {
        i64::from(s) + i64::from(min)
    }
}

// --- four bytes ---

/// A signed 4-byte slot (`i32`): two's complement, `i32::MIN` for absence.
#[must_use]
pub fn enc_int4(v: i64) -> i32 {
    if v == i64::MIN { i32::MIN } else { v as i32 }
}
#[must_use]
pub fn dec_int4(s: i32) -> i64 {
    if s == i32::MIN {
        i64::MIN
    } else {
        i64::from(s)
    }
}

/// An unsigned 4-byte slot with `u32::MAX` for absence (a nullable `u32`).
#[must_use]
pub fn enc_int4_raw(v: i64) -> u32 {
    if v == i64::MIN { u32::MAX } else { v as u32 }
}
#[must_use]
pub fn dec_int4_raw(s: u32) -> i64 {
    if s == u32::MAX {
        i64::MIN
    } else {
        i64::from(s)
    }
}

/// An unsigned 4-byte slot with no code for absence (a non-nullable `u32`).
#[must_use]
pub fn enc_int4_full(v: i64) -> u32 {
    v as u32
}
#[must_use]
pub fn dec_int4_full(s: u32) -> i64 {
    i64::from(s)
}

// --- by kind code, for the interpreter ---

/// Encode `v` for `kind` into the low `width(kind)` bytes of the answer.
#[must_use]
pub fn encode(kind: u8, min: i32, v: i64) -> u32 {
    match kind {
        BYTE => u32::from(enc_byte(v, min)),
        BYTE_NULLABLE => u32::from(enc_byte_nullable(v, min)),
        SHORT_RAW => u32::from(enc_short_raw(v, min)),
        SHORT => u32::from(enc_short(v, min)),
        SHORT_FULL => u32::from(enc_short_full(v, min)),
        INT4 => enc_int4(v) as u32,
        INT4_RAW => enc_int4_raw(v),
        _ => enc_int4_full(v),
    }
}

/// Decode the low `width(kind)` bytes of `bits` for `kind`.
#[must_use]
pub fn decode(kind: u8, min: i32, bits: u32) -> i64 {
    match kind {
        BYTE => dec_byte(bits as u8, min),
        BYTE_NULLABLE => dec_byte_nullable(bits as u8, min),
        SHORT_RAW => dec_short_raw(bits as u16, min),
        SHORT => dec_short(bits as u16, min),
        SHORT_FULL => dec_short_full(bits as u16, min),
        INT4 => dec_int4(bits as i32),
        INT4_RAW => dec_int4_raw(bits),
        _ => dec_int4_full(bits),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every kind round-trips its whole range and its absence code, and the codes are the
    /// store's: a byte `-1` at `min = -128` is `0x7F`, a nullable short `min` is `1`.
    #[test]
    fn every_kind_round_trips_its_range_and_its_absence() {
        for (kind, min, lo, hi) in [
            (BYTE, 0, 0, 255),
            (BYTE, -128, -128, 127),
            (BYTE, 1000, 1000, 1255),
            (BYTE_NULLABLE, 0, 0, 254),
            (BYTE_NULLABLE, -127, -127, 127),
            (SHORT_FULL, 0, 0, 65535),
            (SHORT_FULL, -32768, -32768, 32767),
            (SHORT, 0, 0, 65534),
            (SHORT_RAW, 0, 0, 65534),
            (
                INT4,
                i32::MIN + 1,
                i64::from(i32::MIN) + 1,
                i64::from(i32::MAX),
            ),
            (INT4_RAW, 0, 0, i64::from(u32::MAX) - 1),
            (INT4_FULL, 0, 0, i64::from(u32::MAX)),
        ] {
            for v in [lo, lo + 1, (lo + hi) / 2, hi - 1, hi] {
                assert_eq!(
                    decode(kind, min, encode(kind, min, v)),
                    v,
                    "kind {kind} value {v}"
                );
            }
            if matches!(kind, BYTE_NULLABLE | SHORT | SHORT_RAW | INT4 | INT4_RAW) {
                assert_eq!(
                    decode(kind, min, encode(kind, min, i64::MIN)),
                    i64::MIN,
                    "kind {kind} null"
                );
            }
        }
        assert_eq!(enc_byte(-1, -128), 0x7F);
        assert_eq!(enc_byte_nullable(i64::MIN, 0), 255);
        assert_eq!(enc_short(0, 0), 1);
        assert_eq!(enc_short(i64::MIN, 0), 0);
        assert_eq!(enc_int4(i64::MIN), i32::MIN);
        assert_eq!(enc_int4_raw(i64::MIN), u32::MAX);
    }
}
