// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I84 — Coroutine runtime / yield codec

//! Layout-driven yield codec — the single source both ends of a native
//! coroutine's value channel derive from (`plans/16-coroutine-validation/`,
//! phase 02).
//!
//! A composite yield value (a tuple) is transported through the unified
//! `next_into(stores, &mut [i64])` channel as `T`'s slots flattened into
//! *transport* form: each scalar slot inline as one `i64`; each reference
//! slot as its full absolute `DbRef` packed across two `i64`s.  The producer
//! (`generation/coroutine.rs`) derives the flatten-walk from `T` directly; the
//! consumer (`generation/ops/coroutine.rs`) receives the same kind list as
//! extra `OpCoroutineNext` args (the interpreter ignores them — it reads only
//! `gen` + `value_size`).  Because *both* lists come from [`tuple_kinds`] over
//! the *same* `T`, the two ends agree by construction — no runtime shape tag,
//! no per-shape codec template.
//!
//! Text elements are the one excluded kind: a yielded `text` is a `&str` in
//! native code, not a buffer-native value, so riding the buffer requires
//! interning the string into a store (`codegen_runtime::db_from_text`) with
//! the lifetime question that entails — a separate slice, not this one.  A
//! tuple containing a text (or other unclassifiable) element returns `None`
//! from [`tuple_kinds`], and the legacy per-type channel cannot carry it
//! either: that channel ends in an `as i64` cast.  [`channel_tag`] answers
//! [`CHANNEL_NONE`] for such a type so both ends refuse it instead of
//! emitting a cast rustc rejects (loft#1132).

use crate::data::Type;

/// One flattened transport slot of a yielded composite value.  Each variant
/// knows its transport width and is mapped to/from a small integer code so the
/// kind list can ride as `OpCoroutineNext` integer args.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YieldSlot {
    /// 64-bit integer (every loft `integer`/`long` in arg/var context is `i64`).
    Int,
    /// `character` — native repr is `i32`.
    CharI32,
    /// `boolean` — native repr is `bool`.
    Bool,
    /// `float` — native repr is `f64`, transported as its `to_bits()` image.
    F64,
    /// `single` — native repr is `f32`, transported as its `to_bits()` image.
    F32,
    /// `routine` — native repr is `u32`.
    Routine,
    /// Any `DbRef`-repr type (reference / vector / sorted / hash / index /
    /// boxed-enum / iterator).  Two slots: the full absolute `DbRef`.
    Ref,
}

impl YieldSlot {
    /// Classify a yield-element type into its transport slot, or `None` for a
    /// kind that cannot (yet) ride the unified buffer (`text`, `function`,
    /// nested tuple, …).  `None` anywhere in a tuple drops the whole tuple to
    /// the legacy channel.
    #[must_use]
    pub fn classify(tp: &Type) -> Option<YieldSlot> {
        match tp {
            Type::Integer(_) => Some(YieldSlot::Int),
            Type::Character | Type::Null => Some(YieldSlot::CharI32),
            Type::Boolean => Some(YieldSlot::Bool),
            Type::Float => Some(YieldSlot::F64),
            Type::Single => Some(YieldSlot::F32),
            Type::Routine(_) => Some(YieldSlot::Routine),
            // An iterator handle is not in the DbRef set — it is a coroutine state handle,
            // not a store handle — but it travels the same slot, so it stays named here.
            Type::Iterator(_, _) => Some(YieldSlot::Ref),
            // Every type carried as a `DbRef` takes the Ref slot.  Asked through
            // `data::is_dbref`, the declared home for @FR-Col-Store's store-backed set,
            // rather than restated — which is what that function's own doc asks for: *"a
            // short list is not a compile error anywhere — it routes a handle down the
            // scalar path — so call this function rather than restating it."*
            //
            // Restated here it had drifted SHORT in exactly the way that doc predicts:
            // seven kinds written out, `Radix` (`spatial`) and `Trie` missing.  A tuple
            // MEMBER of either then failed `classify`, so `tuple_kinds` answered `None` and
            // the yield lost the unified `next_into` channel it was entitled to — leaving a
            // `spatial` or `trie` tuple member REFUSED by `--native` (loft#1132's
            // `CHANNEL_NONE`) while `--interpret` answered correctly.  Both are store-backed
            // collections that already yield correctly on their own, so the refusal was a
            // deviation rather than a decision.
            //
            // This is the third site of one list: `generation/coroutine.rs` and
            // `parser/collections.rs` were folded onto `is_dbref` when the ORIGINAL
            // short-list bug was fixed, and `classify` is the residual that fix recorded and
            // left behind.
            _ if crate::data::is_dbref(tp) => Some(YieldSlot::Ref),
            // Text needs a store intern (lifetime); function is a (u32, DbRef)
            // pair still served by the dedicated fn-ref channel; a nested tuple
            // would need recursion the walk does not yet do.
            _ => None,
        }
    }

    /// Number of `i64` transport slots this kind occupies.
    #[must_use]
    pub fn width(self) -> usize {
        match self {
            YieldSlot::Ref => 2,
            _ => 1,
        }
    }

    /// Small integer code for transmission as an `OpCoroutineNext` arg.
    #[must_use]
    pub fn code(self) -> i32 {
        match self {
            YieldSlot::Int => 0,
            YieldSlot::CharI32 => 1,
            YieldSlot::Bool => 2,
            YieldSlot::F64 => 3,
            YieldSlot::F32 => 4,
            YieldSlot::Routine => 5,
            YieldSlot::Ref => 6,
        }
    }

    /// Inverse of [`code`](Self::code) — decode a transmitted arg back to its
    /// kind.  Unknown codes default to [`YieldSlot::Int`] (a plain `i64`
    /// passthrough) so a stale/garbled arg degrades to the identity slot
    /// rather than panicking codegen.
    #[must_use]
    pub fn from_code(code: i32) -> YieldSlot {
        match code {
            1 => YieldSlot::CharI32,
            2 => YieldSlot::Bool,
            3 => YieldSlot::F64,
            4 => YieldSlot::F32,
            5 => YieldSlot::Routine,
            6 => YieldSlot::Ref,
            _ => YieldSlot::Int,
        }
    }
}

/// The flatten-walk for a yielded type, or `None` when the type is not a
/// unified-channel composite (single scalars/refs/text keep their dedicated
/// channels; a tuple with any unclassifiable element falls back to legacy).
///
/// Only *tuples* take the unified walk today — single values already have
/// working dedicated channels (`next_i64` / `next_dbref` / `next_text`), and
/// fn-refs keep their `(u32, DbRef)` channel.  Returning `Some` here is the
/// single decision that both the producer's `is_tuple_into` and the consumer's
/// channel-1 selection share, so they never diverge.
#[must_use]
pub fn tuple_kinds(tp: &Type) -> Option<Vec<YieldSlot>> {
    let Type::Tuple(elems) = tp else {
        return None;
    };
    if elems.is_empty() {
        return None;
    }
    elems.iter().map(YieldSlot::classify).collect()
}

/// The native value-transport channel tag for a coroutine yield of `tp`.
///
/// ONE home for this decision: every coroutine *consumer* — the for-loop
/// (`parser/collections.rs`), manual `next()` (`parser/control.rs`) — and the
/// native producer/consumer emitters dispatch on it, so they must never
/// diverge (a duplicated copy that missed float/single/enum was #401's
/// manual-`next` E0308).  Packed into `value_size` as `(channel_tag << 8) |
/// byte_size`; the interpreter masks the tag off, native reads both bytes.
///   - `0` — legacy per-byte-size channel (i64 / i32 / bool / text / dbref)
///   - `1` — unified `next_into` (tuple)        - `2` — unified `next_into` (fn-ref)
///   - `3` — `f64::from_bits` (float)           - `4` — `f32::from_bits` (single)
///   - `5` — enum-as-`u{8·byte_size}` (NON-nullable only; a nullable enum is a
///     DbRef, so it stays on channel 0 → `next_dbref`)
///
/// 3/4/5 exist because these types' Rust value differs from the i64 transport,
/// which `byte_size` alone cannot distinguish (8 = i64 or f64, 4 = i32 or f32,
/// 1 = bool or u8 enum).
#[must_use]
pub fn channel_tag(tp: &Type) -> i32 {
    if tuple_kinds(tp).is_some() {
        1
    } else if matches!(tp, Type::Function(_, _, _)) {
        2
    } else if matches!(tp, Type::Float) {
        3
    } else if matches!(tp, Type::Single) {
        4
    } else if matches!(tp, Type::Enum(_, false, _)) {
        // Non-nullable enum = the variant index as uN.  A NULLABLE enum is a
        // DbRef (same null repr as Reference), so it must NOT come here — it
        // falls through to channel 0 / next_dbref.
        5
    } else if channel_0_carries(tp) {
        0
    } else {
        CHANNEL_NONE
    }
}

/// [`channel_tag`] for a yield type `--native` has no transport for at all.
///
/// Distinct from the other tags because it is not a channel: it is the answer both ends
/// dispatch on to REFUSE — the producer emits the `compile_error!` naming the type and
/// the workaround, the consumer emits a diverging expression so exactly one diagnostic
/// survives.  The interpreter never sees it: it masks the tag off `value_size` and reads
/// only the byte size.
pub const CHANNEL_NONE: i32 = 6;

/// Does the legacy per-byte-size channel (tag `0`) carry a yield of `tp`?
///
/// Its arms are `text`, a `DbRef`-carried handle, and a SCALAR — everything else reaches
/// an `as i64` cast whose unstated premise is *whatever is left is scalar-shaped*.  So the
/// question is `data::is_scalar` ([formal/types.md](../doc/claude/formal/types.md)'s
/// scalar/heap split, the one home named in
/// [formal/IMPLEMENTATIONS.md](../doc/claude/formal/IMPLEMENTATIONS.md) checklist #1)
/// widened by the two handle shapes.
///
/// A tuple [`tuple_kinds`] could not classify — one carrying a `text` element, or a nested
/// tuple — is exactly the case that violates the premise: the cast then reads
/// `(i64, &String) as i64`, and the author gets a rustc dump against generated source they
/// cannot read for a program `--interpret` runs correctly (loft#1132).
fn channel_0_carries(tp: &Type) -> bool {
    // `@FR-N-Shape` — all three tests ask the same question of the same value, so all three
    // peel.  The middle one did not: `Text` and the scalar list read `base()` while the handle
    // test read the wrapper, so one expression answered two ways about `τ?` depending on which
    // arm reached it.
    matches!(tp.base(), Type::Text(_))
        || crate::data::is_dbref(tp.base())
        || crate::data::is_scalar(tp.base())
}

/// The transport slots an EAGER for-body buffer can hold for a yield of `tp`, or `None`
/// when it cannot hold one.
///
/// The eager collector runs the whole loop up front and reads the buffer afterwards, so it
/// may only hold values carried BY VALUE.  A store handle pushed once per iteration aliases
/// the work record the next iteration overwrites — the unsoundness the struct/vector
/// loop-body refusal in `generation/emit.rs` already names.  A tuple asks the same question
/// one level in: `(integer, integer)` packs into flat slots and is sound, while
/// `(integer, P)` carries a [`YieldSlot::Ref`] and is not.
#[must_use]
pub fn eager_tuple_kinds(tp: &Type) -> Option<Vec<YieldSlot>> {
    let kinds = tuple_kinds(tp)?;
    kinds
        .iter()
        .all(|k| !matches!(k, YieldSlot::Ref))
        .then_some(kinds)
}

/// The `OpCoroutineNext` operands for a yield type: the packed `value_size`
/// (channel tag in the high byte, byte size in the low) and the per-slot kind
/// codes a tuple channel carries as extra arguments.
///
/// One home, because the decision is made TWICE: once where a `for` over a
/// generator is lowered, and again per MONOMORPH, where a generic's
/// `iterator<T>` finally learns what `T` is (loft#1032).  A template bakes
/// these against the type VARIABLE — 12 bytes on the DbRef channel — and
/// substitution rewrites the loop variable's type without revisiting the
/// accessor it was paired with, so a scalar `T` read a 12-byte DbRef out of an
/// 8-byte slot and walked off the end of the store.  Deriving both ends from
/// this one function is what keeps the size and the channel from drifting apart
/// the way the producer and consumer lists already must not.
#[must_use]
pub fn next_operands(yield_tp: &Type) -> (i32, Vec<i32>) {
    let byte_size = i32::from(crate::variables::size(
        yield_tp,
        &crate::data::Context::Argument,
    ));
    let value_size = (channel_tag(yield_tp) << 8) | byte_size;
    let kinds = tuple_kinds(yield_tp)
        .map(|ks| ks.iter().map(|k| k.code()).collect())
        .unwrap_or_default();
    (value_size, kinds)
}

/// Total `i64` transport slots a kind list occupies (the `[i64; N]` buffer
/// size both ends allocate / write).
#[must_use]
pub fn slot_count(kinds: &[YieldSlot]) -> usize {
    kinds.iter().map(|k| k.width()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_tag_distinguishes_float_kinds_from_same_size_scalars() {
        // #401 — float/single get their own native channels (`from_bits`); the
        // other scalars must stay on channel 0.  This is exactly the distinction
        // `byte_size` alone cannot make (8 = i64 or f64, 1 = bool or u8 enum).
        assert_eq!(channel_tag(&Type::Float), 3);
        assert_eq!(channel_tag(&Type::Single), 4);
        assert_eq!(channel_tag(&Type::Boolean), 0);
        assert_eq!(channel_tag(&Type::Character), 0);
    }
}
