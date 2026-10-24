// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I67 — Opcode implementations

//! `LOFT_VERIFY_STACK=1` — read a stack slot that no path wrote, and hear about it.
//!
//! @PLN154 phase 1.  The interpreter's frame is a raw byte store: nothing beside the
//! bytes says whether a slot holds a value at all.  A path that yields nothing therefore
//! leaves the consumer reading whatever the slot held before, and the program answers a
//! plausible number nobody computed — loft#1386 is the measured case, where a
//! value-position `match` arm that produced no value made the whole expression answer
//! `null`, silently, on both backends.
//!
//! The shadow is one tag byte per stack byte, carried by the stack [`Store`] itself
//! (`init_shadow`), so it grows and moves with the buffer it describes rather than being
//! kept in step by hand:
//!
//! * a write through [`Store::addr_mut`](crate::store::Store::addr_mut) TAGS the bytes it
//!   covers — phase 0 measured that 32 of the 33 sites that write the stack reach it
//!   there, where only 74.5 % of the bytes arrive through the typed accessor;
//! * a byte-for-byte move ([`Stores::copy_block`](crate::database::Stores::copy_block) and
//!   friends) carries the tags with the bytes, so a returned value stays as written — or
//!   as unwritten — as the callee left it;
//! * a slot that leaves the live frame loses its tag, because the next occupant of that
//!   offset inherits nothing from the last;
//! * and a read at [`State::get_stack`](crate::state::State::get_stack) /
//!   [`get_var`](crate::state::State::get_var) reports an untagged span.
//!
//! **Tag low, check high** — the low-level reader `Store::addr` is deliberately NOT
//! checked, because the debugger, the frame renderer and the anomaly scanner read
//! uninitialised slots for a living and a check there would report the diagnostics as
//! defects.
//!
//! Off by default and off in every release path: when the shadow is not armed the whole
//! mechanism is one length load and a not-taken branch per store write, and a runtime built
//! without the `stack-verify` feature (the installed one) carries not even that.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::{Mutex, OnceLock};

/// Is the shadow armed?  `LOFT_VERIFY_STACK=1`, on a runtime built with the `stack-verify`
/// feature.  Without it (an installed runtime) the write hooks are compiled out, so the
/// switch is refused once, out loud, rather than letting a silent run read as a clean one.
#[must_use]
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| {
        let asked = std::env::var("LOFT_VERIFY_STACK").is_ok_and(|v| v != "0");
        if asked && !cfg!(feature = "stack-verify") {
            crate::loft_eprintln!(
                "[verify-stack] this runtime was built without the stack shadow (the \
                 `stack-verify` feature): LOFT_VERIFY_STACK is ignored — run it from a \
                 development build"
            );
            return false;
        }
        asked
    })
}

/// `LOFT_VERIFY_STACK_INJECT=1` — the positive control.
///
/// A detector that cannot fire and a clean corpus look identical, so the detector ships
/// with the switch that makes it fire on a program known to be correct: writes stop
/// tagging, every checked read therefore sees an untagged span, and a silent run under
/// this means the CHECK is not reached — not that the program is clean.
///
/// It suppresses the tag at the write hook only.  The moves and the kills still run, so
/// what the control proves is that the check path is live and the report renders; it does
/// not exercise the tag-follows-the-value half, which is what the corpus sweep is for.
#[must_use]
pub fn inject() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_VERIFY_STACK_INJECT").is_ok_and(|v| v != "0"))
}

/// The width and kind a value was written at, packed into a byte's tag.
///
/// @PLN154 phase 2.  A stack slot is a byte offset and nothing more, so *"is the value I am
/// reading the value that was written here"* has no answer unless the write records what it
/// was.  Two facts answer it and both fit small: a FAMILY (is this a handle, a text, a
/// float, or a plain word) and a WIDTH.  Together they separate the mismatches the
/// monomorph-layout class produces — a 12-byte `DbRef` sentinel written into a slot a
/// monomorph reads as an 8-byte integer (loft#1028), a placeholder record defaulted into a
/// slot that turned out to be a float (loft#1016) — from a slot read at the width it was
/// written.
///
/// A FAMILY rather than a full type id because the tag has to be computed on the write path
/// and compared on the read path: hashing a `TypeId` costs more than the comparison it
/// feeds, and the distinctions that matter here are exactly the ones a family makes.
#[must_use]
pub fn kind_of<T: 'static>() -> u16 {
    use std::any::TypeId;
    // A raw byte BLOCK is not a value and carries no type: the fn-ref slot is written as
    // `[MaybeUninit<u8>; 20]` and read back as an `i64` and a `DbRef`, which is a composite
    // read and not a disagreement.  The block spelling is the fact — a type whose name
    // starts with `[` is an array of bytes, written whole by whoever assembled it — so it
    // joins the raw spans on [`OPAQUE`] rather than earning a family of its own.
    if std::any::type_name::<T>().starts_with('[') {
        return OPAQUE;
    }
    let t = TypeId::of::<T>();
    let family: u16 = if t == TypeId::of::<crate::keys::DbRef>() {
        1
    } else if t == TypeId::of::<crate::keys::Str>() {
        2
    } else if t == TypeId::of::<f64>() || t == TypeId::of::<f32>() {
        3
    } else if t == TypeId::of::<String>() {
        4
    } else {
        0
    };
    (family << 8) | (std::mem::size_of::<T>() as u16 & 0xFF)
}

/// The width half of a [`kind_of`] tag.
#[must_use]
pub fn kind_width(kind: u16) -> u32 {
    u32::from(kind & 0xFF)
}

/// The family of a tag whose handle names a record that has since MOVED.
///
/// @PLN154 phase 3.  A stale handle is not a different KIND from a live one — it is the same
/// twelve bytes, naming a record that is now a free block or somebody else's.  So the tag
/// keeps its width and its index and changes only its family, and the check reads it in the
/// same comparison as everything else.
pub const STALE: u16 = 7 << 8;

/// A tag that matches every read: the raw byte moves — a coroutine frame restored from its
/// own snapshot, a worker's stack overlaid with its parent's — carry no type, and inventing
/// one for them would report the restore rather than describe it.
pub const OPAQUE: u16 = u16::MAX;

/// Name a family for a report.  The width is printed beside it, so this says only what the
/// width cannot.
#[must_use]
pub fn kind_name(kind: u16) -> &'static str {
    match kind >> 8 {
        1 => "handle",
        2 => "text",
        3 => "float",
        4 => "string",
        7 => "stale handle",
        _ => "word",
    }
}

/// A record that moved: `(store, the number it had, the number it has)`.
type Relocation = (u16, u32, u32);

thread_local! {
    /// Records that relocated during the current operator.
    ///
    /// A list rather than an immediate scan, because [`Store::resize`](crate::store::Store)
    /// has the two record numbers and no view of the interpreter's frame: it is a `Store`
    /// method, and the frame lives in another store entirely.  The dispatch loop drains this
    /// after each operator, which is also the first moment the mutation that caused the move
    /// has finished updating the containers that legitimately track it.
    static MOVED: RefCell<Vec<Relocation>> = const { RefCell::new(Vec::new()) };
}

/// Has a stack shadow actually been armed on some store?
///
/// `LOFT_VERIFY_STACK` being SET is not the same as the shadow existing: the shadow is armed
/// on the interpreter's value-stack store, and a `--native` run never makes one.  Without
/// this the relocation log would grow for the whole of such a run with nothing ever draining
/// it — and the summary would close with "no … reads", which is true and misleading.
static ARMED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called by `Store::arm_init_shadow`.
pub fn note_armed() {
    ARMED.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Is a shadow armed on some store?
#[must_use]
pub fn armed() -> bool {
    ARMED.load(std::sync::atomic::Ordering::Relaxed)
}

/// Record that `(store, old)` became `(store, new)`.
pub fn note_relocation(store: u16, old: u32, new: u32) {
    if !armed() {
        return;
    }
    MOVED.with_borrow_mut(|m| m.push((store, old, new)));
    with_verify(|v| v.moves += 1);
}

/// Note that `n` frame slots were marked stale by the scan.
pub fn note_marked(n: u64) {
    with_verify(|v| v.marked += n);
}

/// Take the relocations recorded since the last drain.
#[must_use]
pub fn take_relocations() -> Vec<Relocation> {
    MOVED.with_borrow_mut(std::mem::take)
}

/// Is anything waiting?  Cheaper than draining, and the answer is `false` on almost every op.
#[must_use]
pub fn any_relocation() -> bool {
    MOVED.with_borrow(|m| !m.is_empty())
}

/// Report a read through a handle whose record has moved.
pub fn report_stale(what: &str, ty: &str, at: u32, rec: u32, pc: u32, line: u32, op: u16) {
    let ty = ty.rsplit("::").next().unwrap_or(ty);
    let print = with_verify(|v| {
        v.stale += 1;
        if !v.seen.insert((pc, at)) || v.printed >= MAX_REPORTS {
            return false;
        }
        v.printed += 1;
        true
    });
    if print {
        crate::loft_eprintln!(
            "stack verify: {what}<{ty}> at frame offset {at} names record {rec}, which has \
             MOVED since the handle was written — the container grew past its allocation \
             [pc={pc} op={op} line={line}]"
        );
    }
}

/// What the shadow knows about one read's span.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SlotState {
    /// Not one byte of the span was written — the slot has no occupant.  Phase 1's finding.
    Unwritten,
    /// A value was written at this base at a different width or kind.  Phase 2's finding.
    Mismatch { wrote: u16 },
    /// A live handle whose record has since moved.  Phase 3's finding.
    Stale,
    /// A disagreement that is not a handle crossing a value: a scalar narrower than its
    /// stepped slot, a null sentinel read as the type it stands for, a composite slot read
    /// one field at a time.  Counted, never reported.
    Partial,
    /// Every byte was written, at this width and kind.
    Written,
}

/// `LOFT_VERIFY_STACK_TRACE=1` — name every handle the phase-3 scan considered.
///
/// The question a silent stale check raises is *"did the scan see my view at all"*, and the
/// summary's `N record relocation(s); M frame slot(s) named one` answers only half of it.
/// This prints each handle-tagged frame slot the scan read, with the relocations it was
/// compared against — which is how the scan's own addressing bug was found, a `rec * 8`
/// counted twice so every "handle" it printed was a `store=8 rec=3414097922`.
#[must_use]
pub fn trace() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_VERIFY_STACK_TRACE").is_ok_and(|v| v != "0"))
}

/// How many reports to print before falling silent.  A read inside a loop reports every
/// iteration and the first few already name the site; the summary keeps the total.
const MAX_REPORTS: usize = 20;

#[derive(Default)]
struct Verify {
    /// Reads of a span no write had tagged.
    uninit: u64,
    /// Reads that disagreed with the write about width or kind without a handle crossing a
    /// value — the admitted puns, kept as a number because the frame's composite slots make
    /// them the common case rather than a finding.
    partial: u64,
    /// Reads where a handle crossed a value.  Phase 2's finding.
    mismatch: u64,
    /// Reads through a handle whose record has moved.  Phase 3's finding.
    stale: u64,
    /// Records that relocated, and frame slots the scan marked stale because of one.  Printed
    /// so a silent phase-3 run says which half was silent: no moves at all, or moves that no
    /// frame slot named.
    moves: u64,
    marked: u64,
    /// Reads whose span fell outside the shadow — a plumbing fault in the shadow itself,
    /// not a finding about the program.  Counted rather than reported, so it cannot hide
    /// behind a quiet run.
    out_of_range: u64,
    /// One report per `(bytecode position, frame offset)`, so a loop says it once.
    seen: HashSet<(u32, u32)>,
    printed: usize,
}

/// PROCESS-wide, not thread-local: a `par` arm and a placed-library worker each run on
/// their own `State` with their own stack store and their own shadow, and a per-thread
/// tally would print their findings and then close with a main-thread verdict of "no
/// uninitialised stack reads" — the one line a sweep reads.  Measured on
/// `1054-parallel-block-arms-run.loft`, which reported four and summed to none.
static VERIFY: Mutex<Option<Verify>> = Mutex::new(None);

fn with_verify<R>(f: impl FnOnce(&mut Verify) -> R) -> R {
    let mut guard = VERIFY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    f(guard.get_or_insert_with(Verify::default))
}

/// Report a read of a stack span nothing had written.
///
/// `at` is the frame offset the read landed on and `pc` the bytecode position of the
/// operator that made it — together they are the identity a repeat is deduped by.
pub fn report_uninit(what: &str, ty: &str, at: u32, width: usize, pc: u32, line: u32, op: u16) {
    // `std::any::type_name` spells a path (`loft::keys::Str`); the leaf is what names the
    // slot's width and kind to a reader, and the path is noise in a report that already
    // carries an offset and a pc.
    let ty = ty.rsplit("::").next().unwrap_or(ty);
    let print = with_verify(|v| {
        v.uninit += 1;
        if !v.seen.insert((pc, at)) || v.printed >= MAX_REPORTS {
            return false;
        }
        v.printed += 1;
        true
    });
    if print {
        crate::loft_eprintln!(
            "stack verify: {what}<{ty}> reads {width} uninitialised byte(s) at frame offset \
             {at} — nothing wrote this slot on the path that reached here \
             [pc={pc} op={op} line={line}]"
        );
    }
}

/// Report a read of a stack slot written at another width or kind.
#[expect(
    clippy::too_many_arguments,
    reason = "one report line, one argument each — the same shape as strict_store_violation"
)]
pub fn report_mismatch(
    what: &str,
    ty: &str,
    at: u32,
    width: usize,
    wrote: u16,
    interior: bool,
    pc: u32,
    line: u32,
    op: u16,
) {
    let ty = ty.rsplit("::").next().unwrap_or(ty);
    let print = with_verify(|v| {
        v.mismatch += 1;
        if !v.seen.insert((pc, at)) || v.printed >= MAX_REPORTS {
            return false;
        }
        v.printed += 1;
        true
    });
    if print {
        let wrote_desc = if interior {
            "it starts inside a value written earlier".to_string()
        } else {
            format!(
                "the value written here is {} {} byte(s) wide",
                kind_name(wrote),
                kind_width(wrote)
            )
        };
        crate::loft_eprintln!(
            "stack verify: {what}<{ty}> reads {width} byte(s) at frame offset {at} — \
             {wrote_desc} [pc={pc} op={op} line={line}]"
        );
    }
}

/// Note a checked read whose occupant is narrower than the read itself.
pub fn note_partial() {
    with_verify(|v| v.partial += 1);
}

/// Note a checked read whose span lies outside the shadow.
pub fn note_out_of_range() {
    with_verify(|v| v.out_of_range += 1);
}

/// How many findings this run has seen, of either state.
#[must_use]
pub fn violations() -> u64 {
    with_verify(|v| v.uninit + v.mismatch + v.stale)
}

/// Print the verdict.  Called at program exit when armed.
///
/// It prints on a clean run too: silence is the result this instrument exists to make
/// trustworthy, and a detector that says nothing when it found nothing is
/// indistinguishable from one that never ran.
pub fn report() {
    let (uninit, mismatch, stale, sites, partial, out_of_range, moves, marked) = with_verify(|v| {
        (
            v.uninit,
            v.mismatch,
            v.stale,
            v.seen.len(),
            v.partial,
            v.out_of_range,
            v.moves,
            v.marked,
        )
    });
    if !armed() {
        crate::loft_eprintln!(
            "stack verify: armed, but no interpreter stack was shadowed — the shadow lives on \
             the value-stack store, which only `--interpret` builds"
        );
        return;
    }
    if uninit == 0 && mismatch == 0 && stale == 0 {
        crate::loft_eprintln!("stack verify: no uninitialised, mistyped or stale stack reads");
    } else {
        crate::loft_eprintln!(
            "stack verify: {uninit} uninitialised, {mismatch} mistyped and {stale} stale \
             stack read(s), {sites} distinct site(s)"
        );
    }
    if moves > 0 {
        crate::loft_eprintln!(
            "  {moves} record relocation(s); {marked} frame slot(s) named one and were marked \
             stale"
        );
    }
    if partial > 0 {
        crate::loft_eprintln!(
            "  {partial} read(s) disagreed with the write about width or kind without a \
             handle crossing a value (the admitted puns)"
        );
    }
    if out_of_range > 0 {
        crate::loft_eprintln!(
            "  WARNING: {out_of_range} checked read(s) fell outside the shadow — the count \
             above is a floor"
        );
    }
}
