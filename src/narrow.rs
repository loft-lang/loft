// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F4 — Ranged/width integer types: the bytes one occupies at rest
//! The encoding of a narrow integer at rest — the bytes a `u8`, `i8`, `u16`, `i16`, `i32`,
//! `u32` or `limit(lo, hi)` value occupies in a store, spelled once for both backends.
//!
//! A store field holds a narrow integer biased by the type's minimum, with one code kept
//! back for absence where the slot is nullable (`data::NarrowIntKind` names the kinds; the
//! `Store` getters and setters read and write them in place).  `@FR-L-Narrow-Linked` is the
//! rule this module is the bytes of.  A LINKED narrow local —
//! one a `&` bind, a `&` argument or a re-point names (@PLN167 decision 1) — holds the same
//! bytes, so that one `&u8` pointer or `DbRef` reads a local and a field alike
//! (`@FR-B-Ref-Uniform`).  Native keeps such a local in a Rust variable of the storage
//! width and calls these functions at every read and write; the interpreter's
//! `OpVarNarrow` / `OpPutNarrow` call them on the frame slot's bytes.
//!
//! `i64::MIN` is the language's null in a wide value; each kind's absent CODE is what the
//! store writes for it.  A non-nullable kind has no code for absence and never sees one —
//! including one whose declared range leaves codes unused inside its width, which C127
//! settled: `limit(-100, 100) size(1)` leaves 55 codes and none of them is a null, because a
//! value that does not fit takes the type's DEFAULT before it reaches a slot.  The
//! `*_SPARE` kinds below are that retired encoding.
//!
//! @C83 — a narrow value keeps its declared width at rest; storage is never widened for the
//! implementation's convenience.

use std::fmt::{Debug, Formatter};
use std::num::NonZeroU8;
/// The kinds as the interpreter's op operand spells them — `NarrowIntKind::code`.
pub const BYTE: u8 = 0;
pub const BYTE_NULLABLE: u8 = 1;
pub const SHORT_RAW: u8 = 2;
pub const SHORT: u8 = 3;
pub const SHORT_FULL: u8 = 4;
pub const INT4: u8 = 5;
pub const INT4_RAW: u8 = 6;
pub const INT4_FULL: u8 = 7;
/// ⚠ RETIRED by C127, and kept only so the op operand's numbering is stable until an op
/// removal can renumber the table.  It named a non-nullable byte whose type left a code spare
/// inside its width (`limit(-100, 100) size(1)`, 201 values in 256) and decoded that code as
/// null.  C127 gives such a type no null: a value that does not fit takes the type's DEFAULT
/// before it reaches any slot, so no slot needs a code for one.  Nothing selects this kind.
pub const BYTE_SPARE: u8 = 8;
/// The two-byte twin, retired with it.
pub const SHORT_SPARE: u8 = 9;

/// The storage width of a kind, in bytes.
#[must_use]
pub const fn width(kind: u8) -> u32 {
    match kind {
        BYTE | BYTE_NULLABLE | BYTE_SPARE => 1,
        SHORT_RAW | SHORT | SHORT_FULL | SHORT_SPARE => 2,
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

/// ⚠ RETIRED by C127 with [`BYTE_SPARE`] — nothing calls this.  It mapped `255` to the
/// overflow's null; such a type now has no null to map.
#[must_use]
pub fn enc_byte_spare(v: i64, min: i32) -> u8 {
    if v == i64::MIN { 255 } else { enc_byte(v, min) }
}
#[must_use]
pub fn dec_byte_spare(b: u8, min: i32) -> i64 {
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

/// ⚠ RETIRED by C127 with [`SHORT_SPARE`].  `OpGetShortSpare`'s body still names
/// `dec_short_spare`, which is the only reason the pair is still compiled; nothing emits
/// that op.
#[must_use]
pub fn enc_short_spare(v: i64, min: i32) -> u16 {
    if v == i64::MIN {
        u16::MAX
    } else {
        enc_short_full(v, min)
    }
}
#[must_use]
pub fn dec_short_spare(s: u16, min: i32) -> i64 {
    if s == u16::MAX {
        i64::MIN
    } else {
        dec_short_full(s, min)
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
        BYTE_SPARE => u32::from(enc_byte_spare(v, min)),
        SHORT_SPARE => u32::from(enc_short_spare(v, min)),
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
        BYTE_SPARE => dec_byte_spare(bits as u8, min),
        SHORT_SPARE => dec_short_spare(bits as u16, min),
        _ => dec_int4_full(bits),
    }
}

/// Specification of an `integer`-family type — bounds, nullability,
/// and optional forced storage width.
///
/// `Debug` is implemented manually (instead of derived) so
/// `format!("{tp:?}")` on `Type::Integer(spec)` prints
/// `Integer(min, max, not_null)` — matching the tuple shape diagnostic
/// output was built around.  The optional `forced_size` is printed
/// only when present, as a trailing `, size(N)`.
///
/// `PartialEq` / `Eq` / `Hash` are implemented manually to ignore
/// `forced_size` — the annotation is a storage hint, not a value-type
/// difference, and the rest of the compiler uses `==` / `is_equal`
/// to match integer-valued types uniformly.  Code that cares about
/// the storage width reads `spec.forced_size` or calls
/// `spec.byte_width()` directly.
#[derive(Clone, Copy)]
pub struct IntegerSpec {
    /// Inclusive lower bound.  `i32::MIN` is reserved as the null
    /// sentinel; plain-integer templates use `i32::MIN + 1`.
    pub min: i32,
    /// Inclusive upper bound.  Signed, so a range lying wholly below zero has one
    /// (`limit(-100, -1)`, loft#1630); the wide / former-`long` template keeps
    /// `u32::MAX` as its "wider than i32" marker.
    pub max: i64,
    /// When true, the value cannot be null — frees the null sentinel
    /// and widens the usable range by 1 on narrow types.
    pub not_null: bool,
    /// When `Some(n)`, storage width is `n` bytes regardless of
    /// bounds.  Set by the parser from an integer alias's `size(N)`
    /// annotation (`i32` → `Some(4)`, `u8` → `Some(1)`).  `None`
    /// means "use the bounds-range heuristic in `byte_width()`".
    pub forced_size: Option<NonZeroU8>,
}

impl IntegerSpec {
    // ── Canonical templates (constructors) ──────────────────────────────

    /// Plain `integer` / former `long`: full i64 range, 8-byte storage
    /// via the default heuristic.  No forced size.
    pub const fn wide() -> Self {
        IntegerSpec {
            min: i32::MIN + 1,
            max: u32::MAX as i64,
            not_null: false,
            forced_size: None,
        }
    }

    /// The I32 template — i32 bounds, nullable, no forced size.
    pub const fn signed32() -> Self {
        IntegerSpec {
            min: i32::MIN + 1,
            // `as`, not `i64::from`: this is a `const fn` and `From` is not a const trait
            // yet (rust-lang#143874).  Clippy's cast lint does not fire here for that
            // reason; it fires on the non-const `i32()` below, which does use `From`.
            max: i32::MAX as i64,
            not_null: false,
            forced_size: None,
        }
    }

    /// `u8` alias — `0..=255`, forced 1-byte storage.
    pub fn u8() -> Self {
        IntegerSpec {
            min: 0,
            max: 255,
            not_null: false,
            forced_size: NonZeroU8::new(1),
        }
    }

    /// `i8` alias — `-128..=127`, forced 1-byte storage.
    pub fn i8() -> Self {
        IntegerSpec {
            min: -128,
            max: 127,
            not_null: false,
            forced_size: NonZeroU8::new(1),
        }
    }

    /// `u16` alias — `0..=65535`, forced 2-byte storage.
    pub fn u16() -> Self {
        IntegerSpec {
            min: 0,
            max: 65535,
            not_null: false,
            forced_size: NonZeroU8::new(2),
        }
    }

    /// `i16` alias — `-32768..=32767`, forced 2-byte storage.
    pub fn i16() -> Self {
        IntegerSpec {
            min: -32768,
            max: 32767,
            not_null: false,
            forced_size: NonZeroU8::new(2),
        }
    }

    /// `i32` alias — full i32 range, forced 4-byte storage.
    pub fn i32() -> Self {
        IntegerSpec {
            min: i32::MIN + 1,
            max: i64::from(i32::MAX),
            not_null: false,
            forced_size: NonZeroU8::new(4),
        }
    }

    /// @PLN204 — `i64` and an `l` literal: the WIDE template ([`Self::wide`], the type a
    /// literal past `i32` already has).  Its `u32::MAX` bound is what tells a 64-bit value a
    /// file asked for apart from a plain `integer`, which is 64-bit only outside the 32-bit
    /// regime ([`Self::is_width32`]).  Not a forced width: a forced width means a NARROW alias
    /// to every reader of `forced_size`.
    pub fn i64() -> Self {
        Self::wide()
    }

    /// @PLN204 — does a value of this type fit 32 bits by its declaration: a forced width
    /// of 4 bytes or less (`i32`, `u16`, `u8`, …) or a `limit` range inside `i32`?  A plain
    /// `integer` (the default template, no forced width) and an `i64` do not; nor does
    /// `u32`, whose upper half does not fit an `int32_t`.
    #[must_use]
    pub fn is_width32(&self) -> bool {
        if self.max > i64::from(i32::MAX) {
            return false;
        }
        match self.forced_size.map(NonZeroU8::get) {
            Some(n) => n <= 4,
            None => !(self.min == i32::MIN + 1 && self.max == i64::from(i32::MAX)),
        }
    }

    /// `u32` alias — `0..=u32::MAX - 1`, wide storage.
    pub fn u32() -> Self {
        IntegerSpec {
            min: 0,
            max: i64::from(u32::MAX) - 1,
            not_null: false,
            forced_size: None,
        }
    }

    // ── Query methods (consolidate scattered bounds arithmetic) ─────────

    /// Storage width in bytes — honours `forced_size` first, falls back
    /// to the bounds-range heuristic otherwise.
    #[must_use]
    pub fn byte_width(&self, nullable: bool) -> u8 {
        if let Some(n) = self.forced_size {
            return n.get();
        }
        self.range_to_width(nullable)
    }

    /// Map the value RANGE to a narrow storage width (1/2/8 bytes), reserving
    /// one extra code for the null sentinel when `nullable`.
    ///
    /// The ONE home for the range→width fact (H6).  Both [`Self::byte_width`]
    /// (after `forced_size`) and `Type::size` derive from it, so a nullable
    /// narrow field's WRITE width (`Type::size`, used by `set_field_check`)
    /// cannot drift from its READ width (`byte_width`, used by `get_val`).
    /// That drift silently corrupted a nullable FULL-range narrow field's null:
    /// the two sites disagreed at `range == 256`/`257`, so the write stored the
    /// 1-byte `255` sentinel into a field the read decoded as a 2-byte Short →
    /// null read back as `max-1`.
    #[must_use]
    pub fn range_to_width(&self, nullable: bool) -> u8 {
        // #334: a nullable narrow field reserves one code as the null sentinel,
        // so it must hold `distinct + 1` codes; `range()` is the distinct count.
        let codes = self.range() + i64::from(nullable);
        if codes <= 256 {
            1
        } else if codes <= 65536 {
            2
        } else {
            8
        }
    }

    /// Usable LOWER bound for a field/element of this spec at the given
    /// nullability — the ONE home for the nullable-narrow range fact, so the read
    /// op's `min`, the write op's `min`, and the literal range-check cannot drift.
    ///
    /// Null is stored as the all-ones byte (`255`/`65535`), uniformly for every
    /// narrow type — one type-independent store/test in generated Rust.  A value
    /// decodes as `read + min`.  When a NULLABLE narrow field's range exactly
    /// FILLS its storage width, the all-ones byte would otherwise be a real value,
    /// so the field sacrifices ONE edge: SIGNED drops the BOTTOM (`min+1`, keeping
    /// the positive range — so a nullable `i8` is `-127..=127`); UNSIGNED keeps
    /// `min` and drops the top via [`Self::usable_max`].  Not-null specs, and any
    /// range that does not fill its width, use the full declared bounds.
    #[must_use]
    pub fn usable_min(&self, nullable: bool) -> i32 {
        if self.reserves_narrow_sentinel(nullable) && self.min < 0 {
            self.min + 1
        } else {
            self.min
        }
    }

    /// @FR-E-Uncomp-NN — the value a slot of this type holds when nobody assigned one, and
    /// the value an uncomputable result takes where the slot cannot hold null.
    ///
    /// Zero wherever the range admits it, else the bound nearest zero.  A declared range that
    /// excludes zero is the whole content: `integer limit(10, 20)` defaults to `10`, and `0`
    /// is not a value that type has — a slot holding it is holding something its own
    /// declaration forbids.
    ///
    /// The ONE home, because there were three and they agreed only where the range contains
    /// zero, which is nearly every width anyone tests: the parser's `range_default` (this
    /// answer), `data::to_default` and the native `default_native_value` (both `0` flat).
    /// loft#1246 already recorded two of them drifting apart; loft#1254 found the third.
    #[must_use]
    pub fn default_value(&self) -> i64 {
        let (lo, hi) = (i64::from(self.min), self.max);
        if lo <= 0 && 0 <= hi {
            0
        } else if lo > 0 {
            lo
        } else {
            hi
        }
    }

    /// Usable UPPER bound — the [`Self::usable_min`] companion.  A nullable narrow
    /// UNSIGNED spec whose range fills its width drops the TOP code (`max-1`, so a
    /// nullable `u8` is `0..=254`); signed keeps `max`.
    #[must_use]
    pub fn usable_max(&self, nullable: bool) -> i64 {
        if self.reserves_narrow_sentinel(nullable) && self.min >= 0 {
            self.max - 1
        } else {
            self.max
        }
    }

    /// Can a value held in a NON-nullable slot of this spec ever read back as null?
    ///
    /// `!x` tests whether `x` is the null sentinel, so this is the question
    /// `redundant-null-negation` asks of its operand: a spec that answers `false` kept no
    /// code back for a failure, so `!x` on it is always false and a fit-failure leaves
    /// nothing in the value to see (@PLN152).
    ///
    /// Two families, and the split is a DECISION rather than a property — it is where C127's
    /// pragmatic exemption falls, not a line the null model draws:
    ///
    /// 1. the plain `integer` and `i32` templates keep a code for absence, so `!x` on either
    ///    is a real test and must not be flagged;
    /// 2. every other non-nullable spec is a declared NARROW range, and by C127 it has no
    ///    null at all: a value that does not fit takes the type's DEFAULT, so `!x` on it is
    ///    always false.
    ///
    /// `not_null` is NOT read.  It once was, as "the same answer reached by declaration", but
    /// every non-nullable struct field's spec carries it and `not null` itself has no effect,
    /// so on a wide template it claimed a null the bytes still hold: `s.i = a * b` overflowing
    /// into an `i: integer` field reads null with nothing reported (@C85), and the lint called
    /// the only test that sees it always false.  The width families above are the whole answer.
    ///
    /// ⚠ Do NOT re-derive clause 1 from "which code the type kept back".  `u32` keeps one
    /// too — `limit(0, 4294967294) size(4)`, at the top of its range, which is what refuses
    /// `u32 = 4294967295` — and it is in clause 2.  The reason `i32` is in clause 1 is that
    /// the owner allowed the exemption on COST (one value in 2^32, for the only narrow type
    /// whose overflow is detectable), so this boundary moves by decision and not by a fact
    /// about representations (C127 § Decision; `tests/scripts/1615-…`'s `templates()` row
    /// pins all three types across all three slot kinds for exactly that reason).
    ///
    /// ⚠ Before C127 clause 2 read *"kept a code back inside its width"* — a predicate
    /// (`reserves_sentinel_unconditionally`) that made the lint's answer depend on whether
    /// `hi - lo + 1` fell short of `2^(8*size)`, so it fired on
    /// `limit(-128, 127) size(1)` and went silent on `limit(-100, 100) size(1)`, two
    /// declarations one value apart.
    ///
    /// ⚠ This is NOT the same question as `uncomputable_default`'s, and the two must not be
    /// routed through one predicate — they differ on `not_null`.  That flag is a claim about
    /// the SLOT (every non-nullable struct field's spec carries it); the stored default asks
    /// what the REPRESENTATION keeps a code for, and a field declared `c: i32` still has the
    /// range `[i32::MIN + 1, MAX]`, so `i32::MIN` is still spare in its bytes.  Measured when
    /// they were unified: the `i32` FIELD stored `0` where its local and its element stored
    /// null — loft#1296's disagreement reopened from the other side.  An earlier version of
    /// this comment claimed *"two readers, one fact"*; that was never true.
    /// Is this the FULL `integer` — the one range nothing narrows to?  `IntegerSpec`'s i32/u32
    /// bounds cannot hold the i64 range, so it has two bound encodings (the `signed32`
    /// template ending at `i32::MAX`, the `wide` one at `u32::MAX`, formal/types.md D2) and is
    /// told from a range by those and by carrying no `forced_size`: the `i32` ALIAS has the
    /// signed-32 template's exact range in 4 bytes, and is a narrow range like any other.
    #[must_use]
    pub fn is_full_integer(&self) -> bool {
        self.forced_size.is_none() && (self.is_signed32_template() || self.is_wide_template())
    }

    #[must_use]
    pub fn non_null_reads_null(&self) -> bool {
        self.is_wide_template() || self.is_signed32_template()
    }

    fn reserves_narrow_sentinel(&self, nullable: bool) -> bool {
        if !nullable {
            return false;
        }
        // Only a FIXED-width (`forced_size`) field can't widen to make room — an
        // un-annotated `limit(...)` instead widens via `range_to_width(+nullable)`,
        // so it never reaches here needing a sacrifice.  Reduce iff the range fills
        // the fixed 1- or 2-byte width (256 / 65536 codes).
        match self.forced_size.map(NonZeroU8::get) {
            Some(1) => self.range() == 256,
            Some(2) => self.range() == 65_536,
            _ => false,
        }
    }

    /// The `min` a narrow slot's schema Part must carry — the SAME offset the READ op
    /// (`get_val`) and the WRITE op (`set_field_check` / `narrow_elm_set`) encode
    /// against, which is [`Self::usable_min`] under the kind's sentinel rule.
    ///
    /// A nullable 1- or 2-byte slot reserves one code for null, and when the declared
    /// range FILLS the width a SIGNED spec pays for that by dropping its bottom edge
    /// (`min + 1`).  The ops moved with it; the schema registration passed the raw
    /// `min`, so a present `i16?` rendered exactly one too LOW — `-300` stored, `-301`
    /// printed — while reading the same slot answered `-300`.  Unsigned specs drop the
    /// TOP edge instead, which is why `u8?` / `u16?` looked fine.
    #[must_use]
    pub fn part_min(&self, width: u8, nullable: bool) -> i32 {
        // `reserves_sentinel()` is true exactly for the nullable 1- and 2-byte kinds
        // (`ByteNullable` / `Short`); the raw-vs-full choice a narrow-vector element
        // makes never reserves one, so the width and nullability decide it alone.
        self.usable_min(nullable && matches!(width, 1 | 2))
    }

    /// element stride for narrow vectors, matching
    /// what `typedef.rs::fill_database`'s Vector arm registers.
    /// Returns `Some(n)` for the direct-encoded widths:
    /// - 1 → `Parts::Byte` (u8 / i8)
    /// - 2 → `Parts::ShortRaw` (u16 / i16)
    /// - 4 → `Parts::Int` (i32)
    ///
    /// All three use direct raw encoding (no `+1` shift), so
    /// `vector_add`'s raw-byte copy path works across source literal
    /// vectors and destination fields without re-encoding.
    ///
    /// Returns `None` when the element stores at the wide 8-byte stride, so the
    /// caller keeps the default wide-integer path.  `u16` struct fields continue
    /// to use `Parts::Short` (the legacy `+1` encoding) via the
    /// `alias != u32::MAX` path in `get_val` / `set_field_check`.
    ///
    /// Callers use this at compile time to emit matching `elm_size` in
    /// `OpGetVector` / `OpSetVector`, and in `get_val` to choose the
    /// right-width scalar-read opcode.  Keeping the predicate in one
    /// place avoids the narrow-read / wide-storage skew that bit the
    /// first Phase 3 attempt.
    ///
    /// loft#1036 — the width comes from [`Self::byte_width`], the ONE
    /// range→width home, NOT from `forced_size` alone.  `formal/layout.md`
    /// settles which: `(L-Narrow)` stores a range-annotated integer in the
    /// smallest width that holds its RANGE, and `(L-Ref)` makes a collection's
    /// element stride exactly `width(element)` — so `integer limit(10, 255)`
    /// is a 1-byte element whether or not it was also spelled `size(1)`.
    /// While this asked `forced_size`, the `limit(...)` spelling answered
    /// `None` here (element stride + schema stayed wide 8-byte) while the READ
    /// (`get_val`) already asked `byte_width` and emitted a 1-byte `OpGetByte`
    /// with the `- min` offset decode.  Two homes, two layouts for one type —
    /// `(L-Total)`'s "the layout is decided by the type alone, never by which
    /// call site reads it" — so every element read came back exactly `lo` too
    /// high (12 stored, 22 returned) while a struct FIELD of the identical type
    /// was correct.
    ///
    /// `nullable` is the element's own nullability (`vector<u8?>`): a nullable
    /// narrow element reserves one code for the null sentinel, which can widen
    /// the slot (`limit(0, 255)?` needs 257 codes → 2 bytes).  Passing `false`
    /// where the caller has already peeled `Optional` is only correct when that
    /// caller routes the nullable case elsewhere — `Data::vector_element_type`
    /// is the one that answers for it.
    #[must_use]
    pub fn vector_narrow_width(&self, nullable: bool) -> Option<u8> {
        match self.byte_width(nullable) {
            w @ (1 | 2 | 4) => Some(w),
            _ => None,
        }
    }

    /// True when the value range exceeds the signed-32-bit range.
    #[must_use]
    pub fn is_wide(&self) -> bool {
        self.max > i64::from(i32::MAX)
    }

    /// Number of distinct representable values (inclusive range + 1).
    #[must_use]
    pub fn range(&self) -> i64 {
        self.max - i64::from(self.min) + 1
    }

    /// Can `width` bytes hold every value this range admits? — `@FR-L-Narrow`, read as the
    /// question a DECLARED width has to answer.
    ///
    /// The narrow encodings store the OFFSET `value - min` ([`NarrowIntKind`]'s
    /// `(val - min)` stores), so a range of `range()` distinct values needs
    /// `range() <= 256^width` codes.  A declared `size(…)` narrower than that is a
    /// CONTRADICTION rather than a tight fit: the type promises values its storage cannot
    /// represent, and every one past the window is lost with nothing said — measured on
    /// `integer limit(0, 100000) size(1)`, where `300` and `100000` both read back as the
    /// range's own `min` on both backends (loft#1501).
    ///
    /// The companion of [`Self::range_to_width`], which answers the same relation the other
    /// way round; it is a separate method because it cannot borrow that one's answer —
    /// `range_to_width` maps to 1, 2 or 8 and has no 4, so asking it about a `size(4)` type
    /// (`u32`, `i32`) would report a contradiction that is not there.  `nullable` reserves a
    /// code exactly as it does there, so the two count the same codes.
    #[must_use]
    pub fn range_fits_width(&self, width: u8, nullable: bool) -> bool {
        if width >= 8 {
            return true;
        }
        self.range() + i64::from(nullable) <= 1i64 << (8 * u32::from(width))
    }

    /// True when this is the I32 template (plain `integer` post-2c).
    #[must_use]
    pub fn is_signed32_template(&self) -> bool {
        self.min == i32::MIN + 1 && self.max == i64::from(i32::MAX)
    }

    /// True when this is the wide I64 template.
    #[must_use]
    pub fn is_wide_template(&self) -> bool {
        self.min == i32::MIN + 1 && self.max == i64::from(u32::MAX)
    }

    /// The loft-SOURCE spelling of this spec, or `None` when it has no name of
    /// its own and must be written as the explicit `integer(min, max)` form.
    ///
    /// The ONE home for this fact: `Type::name`] and `Display for Type` both
    /// render integers through it, so the two cannot drift.  They had — each
    /// named the signed32 template `integer` but let the WIDE template (equally
    /// `integer` in source, equally 8 bytes) fall through to the `integer(min,
    /// max)` debug form.  The REPL builds a capture function from that name, so
    /// it emitted `-> vector<integer(-2147483647, 4294967295)>`, which does not
    /// parse (#618).
    ///
    /// Narrow aliases (`u8`/`i16`/…) carry a `forced_size`, so they keep a distinct
    /// name and a distinct storage width. Note that carrying one does not stop a
    /// spec matching a template *predicate*: `i32`'s range is exactly the signed-32
    /// template's, so [`IntegerSpec::is_signed32_template`] answers true for it.
    /// Test `forced_size` to tell an alias from a template; the range alone cannot.
    #[must_use]
    pub fn source_name(&self) -> Option<&'static str> {
        if self.is_signed32_template() || self.is_wide_template() {
            Some("integer")
        } else if self.min == 0 && self.max == 256 {
            Some("byte")
        } else {
            None
        }
    }

    /// True when the declared range is non-negative AND runs past `i32::MAX` — a value
    /// no *signed* 4-byte slot can hold.  Such a type needs the unsigned 4-byte op pair
    /// (`OpGetInt4Raw` / `OpGetInt4Full`); the signed `OpGetInt4` sign-extends it on load
    /// and, worse, its `i32::MIN` sentinel is the legal value 2147483648.
    ///
    /// Both halves are load-bearing.  The `min >= 0` half is what excludes the WIDE
    /// (8-byte) template, which sets `max == u32::MAX` purely as a "wider than i32"
    /// marker while keeping a negative `min` — an `max > i32::MAX` test alone would
    /// misroute every plain `integer`.
    #[must_use]
    pub fn unsigned_wide(&self) -> bool {
        self.min >= 0 && self.max > i64::from(i32::MAX)
    }
}

/// The narrow-integer storage op KIND a field of a resolved storage width uses —
/// the ONE home for the width→op decision (H4-medium).  Both the READ (`get_val`
/// → `OpGet*`) and the WRITE (`set_field_check` → `OpSet*`) emitters derive their
/// op from `NarrowIntKind::of`, so a field's read op and write op can't drift to
/// mismatched widths/encodings — the H6-class hazard one level up (a `Byte` write
/// decoded under a `Short` read).  Width itself comes from the single
/// `IntegerSpec::range_to_width`/`byte_width` home; this maps that width (+ the
/// nullable / narrow-vector context) to the matched `OpGet*`/`OpSet*` pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NarrowIntKind {
    /// 1-byte raw — `not null`, or a narrow-vector element.
    Byte,
    /// 1-byte with the reserved 255 null sentinel — a nullable struct field.
    ByteNullable,
    /// 2-byte direct encoding — a narrow-vector element (raw, no `+1`).
    ShortRaw,
    /// 2-byte `+1` sentinel encoding — a NULLABLE struct field (reserves `0`).
    Short,
    /// 2-byte direct encoding, NO sentinel — a NOT-NULL struct field, so the
    /// full 65536-value range round-trips (`Short`'s `+1` and `ShortRaw`'s
    /// `u16::MAX` both swallow the max as null).  Read via `OpGetShortFull`; the
    /// write reuses `OpSetShortRaw` (same `(val - min)` store).
    ShortFull,
    /// 4-byte raw, SIGNED — `i32` and any `size(4)` range inside `i32`'s bounds.
    /// `i32::MIN` is the null sentinel.
    Int4,
    /// 4-byte UNSIGNED with the reserved `u32::MAX` null sentinel — a nullable slot, or
    /// a narrow-vector element, of a type whose range runs past `i32::MAX` (`u32`).  The
    /// 4-byte twin of `ShortRaw`.
    Int4Raw,
    /// 4-byte UNSIGNED, NO sentinel — a NOT-NULL field of such a type, so the full 2^32
    /// round-trips.  Read via `OpGetInt4Full`; the write reuses `OpSetInt4Raw` (same
    /// unsigned store).  The 4-byte twin of `ShortFull`.
    Int4Full,
    /// 8-byte raw — the wide default.
    Int,
}

impl NarrowIntKind {
    /// Map a resolved storage `width` (1/2/4/8) to its op kind.  `nullable` = the
    /// slot reserves a null sentinel — a nullable struct field OR a nullable
    /// narrow-vector element (both encode null the same way).  `narrow_vec` = a
    /// direct-encoded narrow-vector element, which selects the RAW 2-byte read
    /// (`ShortRaw`) over the not-null field's full-range read (`ShortFull`) —
    /// but only when the slot is NOT nullable; a nullable slot always reserves a
    /// sentinel regardless of field-vs-vector.
    #[must_use]
    pub fn of(width: u8, nullable: bool, narrow_vec: bool, unsigned_wide: bool) -> Self {
        match width {
            1 if nullable => NarrowIntKind::ByteNullable,
            1 => NarrowIntKind::Byte,
            2 if nullable => NarrowIntKind::Short,
            2 if narrow_vec => NarrowIntKind::ShortRaw,
            2 => NarrowIntKind::ShortFull,
            // The 4-byte split mirrors the 2-byte one directly above, but applies ONLY
            // to a range that runs past `i32::MAX` (`unsigned_wide`).  Everything else
            // 4-byte — `i32` above all — stays on the signed `Int4` pair, whose stored
            // bytes are two's complement and are relied on beyond this module.
            4 if unsigned_wide && (nullable || narrow_vec) => NarrowIntKind::Int4Raw,
            4 if unsigned_wide => NarrowIntKind::Int4Full,
            4 => NarrowIntKind::Int4,
            _ => NarrowIntKind::Int,
        }
    }

    /// True for the kinds that reserve an in-band null sentinel (the `255` byte
    /// code / the `+1`-shifted short).  The read/write `min` derives from THIS —
    /// not a re-derived `nullable && !narrow_vec` — so a nullable narrow-vector
    /// element (also `ByteNullable`/`Short` now) decodes against the same shrunk
    /// `min` its write encodes with.
    #[must_use]
    pub fn reserves_sentinel(self) -> bool {
        matches!(self, NarrowIntKind::ByteNullable | NarrowIntKind::Short)
    }

    /// The kind as the interpreter's `OpVarNarrow` / `OpPutNarrow` operand spells it — the
    /// constants in `crate::narrow`, which decode by this code.
    #[must_use]
    pub fn code(self) -> u8 {
        match self {
            NarrowIntKind::Byte => crate::narrow::BYTE,
            NarrowIntKind::ByteNullable => crate::narrow::BYTE_NULLABLE,
            NarrowIntKind::ShortRaw => crate::narrow::SHORT_RAW,
            NarrowIntKind::Short => crate::narrow::SHORT,
            NarrowIntKind::ShortFull => crate::narrow::SHORT_FULL,
            NarrowIntKind::Int4 => crate::narrow::INT4,
            NarrowIntKind::Int4Raw => crate::narrow::INT4_RAW,
            NarrowIntKind::Int4Full => crate::narrow::INT4_FULL,
            NarrowIntKind::Int => u8::MAX,
        }
    }

    /// The `OpGet*` op name for this kind.
    #[must_use]
    pub fn get_op(self) -> &'static str {
        match self {
            NarrowIntKind::Byte => "OpGetByte",
            NarrowIntKind::ByteNullable => "OpGetByteNullable",
            NarrowIntKind::ShortRaw => "OpGetShortRaw",
            NarrowIntKind::Short => "OpGetShort",
            NarrowIntKind::ShortFull => "OpGetShortFull",
            NarrowIntKind::Int4 => "OpGetInt4",
            NarrowIntKind::Int4Raw => "OpGetInt4Raw",
            NarrowIntKind::Int4Full => "OpGetInt4Full",
            NarrowIntKind::Int => "OpGetInt",
        }
    }

    /// Every `OpGet*` name [`Self::get_op`] can produce, as a set — derived from the same
    /// match, so a new kind cannot be missed.
    ///
    /// It exists because `Parser::is_amp_place` kept its own hand-written list of readable
    /// place ops, and the narrow family grew past it: the list had `OpGetByte` and
    /// `OpGetShort` while a `u16` field reads `OpGetShortFull`, an `i32` field `OpGetInt4`,
    /// and a `u8` element `OpGetVectorNullable` — so lifting D-bind-39's refusal fell straight
    /// into *"`&` requires an addressable operand"* for exactly the places that had just been
    /// admitted.  Adding seven more names by hand is the failure mode, not the cure.
    #[must_use]
    pub fn is_get_op(name: &str) -> bool {
        [
            NarrowIntKind::Byte,
            NarrowIntKind::ByteNullable,
            NarrowIntKind::ShortRaw,
            NarrowIntKind::Short,
            NarrowIntKind::ShortFull,
            NarrowIntKind::Int4,
            NarrowIntKind::Int4Raw,
            NarrowIntKind::Int4Full,
            NarrowIntKind::Int,
        ]
        .iter()
        .any(|k| k.get_op() == name)
    }

    /// The `OpSet*` op name for this kind — the matched write twin of [`Self::get_op`].
    #[must_use]
    pub fn set_op(self) -> &'static str {
        match self {
            NarrowIntKind::Byte => "OpSetByte",
            NarrowIntKind::ByteNullable => "OpSetByteNullable",
            NarrowIntKind::ShortRaw => "OpSetShortRaw",
            NarrowIntKind::Short => "OpSetShort",
            // not-null 2-byte reuses the raw `(val - min)` store; only the READ
            // differs (`OpGetShortFull`, no sentinel decode).
            NarrowIntKind::ShortFull => "OpSetShortRaw",
            NarrowIntKind::Int4 => "OpSetInt4",
            // not-null 4-byte reuses the raw unsigned store; only the READ differs
            // (`OpGetInt4Full`, no sentinel decode) — as `ShortFull` does one width down.
            NarrowIntKind::Int4Raw | NarrowIntKind::Int4Full => "OpSetInt4Raw",
            NarrowIntKind::Int => "OpSetInt",
        }
    }

    /// The schema `Parts` type id a slot of this kind stores under — the schema-side
    /// twin of [`Self::get_op`] / [`Self::set_op`], registering the type if it is new.
    ///
    /// @FR-L-Narrow-Enc — the encoding is part of the layout, so the schema and the ops
    /// take it from one choice.
    ///
    /// This is the ONE home for the width→Part decision, and it is deliberately keyed on
    /// the same [`NarrowIntKind`] the read and write ops come from: a slot's Part and its
    /// ops then cannot name different encodings, because there is only one choice and all
    /// three read it.  The five call sites — the struct field, the two element paths, and
    /// both halves of the native generator — share it, because a site deriving the Part from
    /// the width alone answers `database.int(…)` flat at 4 bytes, so an unsigned 4-byte slot is
    /// WRITTEN by `OpSetInt4Raw` (unsigned, `u32::MAX` for absence) and read back through
    /// `Parts::Int` (sign-extending, `i32::MIN` for absence).  Every schema-driven route
    /// — the record render, `to_json`, the store round-trip, and every keyed lookup —
    /// then answered a `u32` at or above 2147483648 as a negative number, while a direct
    /// field read of the same field was correct (loft#1437).
    ///
    /// `None` for [`NarrowIntKind::Int`], the wide 8-byte default, which has no narrow
    /// Part: the caller keeps its own wide path.
    #[must_use]
    pub fn part(
        self,
        database: &mut crate::database::Stores,
        min: i32,
        nullable: bool,
    ) -> Option<u16> {
        Some(match self {
            NarrowIntKind::Byte | NarrowIntKind::ByteNullable => database.byte(min, nullable),
            NarrowIntKind::Short => database.short(min, nullable),
            // Both 2-byte direct kinds decode the same way; `nullable` is what tells the
            // Part whether the top code is absence.
            NarrowIntKind::ShortRaw | NarrowIntKind::ShortFull => database.short_raw(min, nullable),
            NarrowIntKind::Int4 => database.int(min, nullable),
            // As one width down: the raw and full 4-byte kinds share an encoding and
            // differ only in whether the reserved top code means absence.
            NarrowIntKind::Int4Raw | NarrowIntKind::Int4Full => database.int_raw(min, nullable),
            NarrowIntKind::Int => return None,
        })
    }

    /// @FR-L-Narrow-Enc — the schema NAME this kind's Part is registered under: the key the `Stores`
    /// constructors dedupe on, and the key the native generator looks a registered
    /// element type up by.
    ///
    /// It lives here because those are two different pieces of code that must produce
    /// the SAME string: the generator reconstructing it by hand is how `vector<u32>`
    /// came out as `int<0,false>` in generated `init()` while the compiler had
    /// registered `int_raw<0,false>`, which mints an extra type and renames every id
    /// after it (`LOFT_STRICT_SCHEMA_IDS` reports it; loft#739 is the class).
    ///
    /// `None` for [`NarrowIntKind::Int`], which has no narrow Part.
    #[must_use]
    pub fn part_name(self, min: i32, nullable: bool) -> Option<String> {
        Some(match self {
            // The unqualified `byte` is the historical key for the common shape and is
            // the name that store is registered under; keeping it is not cosmetic.
            NarrowIntKind::Byte | NarrowIntKind::ByteNullable => {
                if min == 0 && !nullable {
                    "byte".to_string()
                } else {
                    format!("byte<{min},{nullable}>")
                }
            }
            NarrowIntKind::Short => format!("short<{min},{nullable}>"),
            NarrowIntKind::ShortRaw | NarrowIntKind::ShortFull => {
                format!("short_raw<{min},{nullable}>")
            }
            NarrowIntKind::Int4 => format!("int<{min},{nullable}>"),
            NarrowIntKind::Int4Raw | NarrowIntKind::Int4Full => {
                format!("int_raw<{min},{nullable}>")
            }
            NarrowIntKind::Int => return None,
        })
    }

    /// @FR-L-Narrow-Enc — the [`crate::database::Stores`] constructor that registers this
    /// kind's Part, by name: what the native generator writes into `init()`.
    ///
    /// It comes from here so the generated schema and the interpreter's are chosen by
    /// the SAME rule.  Registering a different Part in `init()` renames every type id
    /// after it, which `LOFT_STRICT_SCHEMA_IDS` reports as schema-id drift (loft#739).
    ///
    /// `None` for [`NarrowIntKind::Int`], which has no narrow Part.
    #[must_use]
    pub fn part_ctor(self) -> Option<&'static str> {
        Some(match self {
            NarrowIntKind::Byte | NarrowIntKind::ByteNullable => "byte",
            NarrowIntKind::Short => "short",
            NarrowIntKind::ShortRaw | NarrowIntKind::ShortFull => "short_raw",
            NarrowIntKind::Int4 => "int",
            NarrowIntKind::Int4Raw | NarrowIntKind::Int4Full => "int_raw",
            NarrowIntKind::Int => return None,
        })
    }

    /// True when the 1/2-byte ops take a trailing `min` arg; the 4/8-byte ops
    /// (`Int4`/`Int`) do not.
    #[must_use]
    pub fn takes_min(self) -> bool {
        matches!(
            self,
            NarrowIntKind::Byte
                | NarrowIntKind::ByteNullable
                | NarrowIntKind::ShortRaw
                | NarrowIntKind::Short
                | NarrowIntKind::ShortFull
        )
    }
}

impl Debug for IntegerSpec {
    /// Matches the `Integer(min, max, not_null)` tuple shape so the
    /// `Display for Type` fallback (`format!("{self:?}").to_lowercase()`)
    /// and any other `{:?}` consumer produces the expected output.
    /// A `forced_size` annotation, when present, is appended as `, size(N)`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}, {}, {}", self.min, self.max, self.not_null)?;
        if let Some(n) = self.forced_size {
            write!(f, ", size({})", n.get())?;
        }
        Ok(())
    }
}

impl PartialEq for IntegerSpec {
    /// Equality ignores `forced_size`: the annotation is a storage
    /// hint, not a semantic difference.  `vector<i32>` and
    /// `vector<integer limit(-2147483647, 2147483647)>` have the same
    /// value-type even though the former is stored in 4 bytes.  Code
    /// that needs the storage distinction reads `spec.forced_size`
    /// or calls `spec.byte_width()`.
    fn eq(&self, other: &Self) -> bool {
        self.min == other.min && self.max == other.max && self.not_null == other.not_null
    }
}

impl Eq for IntegerSpec {}

impl std::hash::Hash for IntegerSpec {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.min.hash(state);
        self.max.hash(state);
        self.not_null.hash(state);
        // forced_size intentionally omitted — see PartialEq.
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
            (BYTE_SPARE, -100, -100, 100),
            (SHORT_SPARE, -1000, -1000, 1000),
        ] {
            for v in [lo, lo + 1, i64::midpoint(lo, hi), hi - 1, hi] {
                assert_eq!(
                    decode(kind, min, encode(kind, min, v)),
                    v,
                    "kind {kind} value {v}"
                );
            }
            if matches!(
                kind,
                BYTE_NULLABLE | SHORT | SHORT_RAW | INT4 | INT4_RAW | BYTE_SPARE | SHORT_SPARE
            ) {
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
