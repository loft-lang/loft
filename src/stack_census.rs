// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I67 — Opcode implementations

//! `LOFT_STACK_CENSUS=1` — which code actually writes the interpreter stack?
//!
//! @PLN154 phase 0.  The plan's shadow keys its tag on the two typed accessors,
//! [`State::put_stack`](crate::state::State::put_stack) and
//! [`State::get_stack`](crate::state::State::get_stack).  That is sound only if those
//! accessors are where the bytes actually arrive, and the repo already knows of one
//! route that goes around them: the return-value slide in
//! `State::return_value` is a raw `copy_block`, which `LOFT_UAF_GEN` had to be taught
//! by hand after an untracked slide produced its residual false positive.  A shadow
//! built on an incomplete list of writers reports the language's own code as a defect.
//!
//! So this measures the question instead of assuming an answer, and it takes no list of
//! sites on trust.  After every operator it **diffs** the stack store's live bytes
//! against a snapshot, subtracts the spans `put_stack` says it wrote, and attributes
//! what is left to the opcode that ran.  A route nobody knows about is therefore
//! counted, because the ground truth is the memory rather than an inventory of callers.
//!
//! Two numbers come out: the share of stack bytes that arrive through `put_stack`, and
//! the opcodes responsible for the rest.  The second is the actionable one — each
//! opcode names an implementation in `fill.rs` or `state/`, which is the site list the
//! phase owes.
//!
//! Off by default and off in every release path: one `OnceLock` read per op when armed,
//! nothing when not.
//!
//! A byte counts as changed against whatever the slot held before, and a reused slot can
//! hold a stale heap pointer, so the byte totals move with the address layout: between two
//! builds of the same program, or two runs, by up to a word per op.  The op count, the
//! claims and which ops write are exact; compare those, not the totals, across builds.
//! Each interpreter keeps its own census (`State::stack_census`).

use std::collections::HashMap;
use std::sync::OnceLock;

/// Is the census armed?  `LOFT_STACK_CENSUS=1`.
#[must_use]
pub fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_STACK_CENSUS").is_ok_and(|v| v != "0"))
}

/// `LOFT_STACK_CENSUS_MAX_OPS=N` — report after `N` operators, then stop the process.
///
/// A census over a corpus wants BREADTH: which routes exist, across as many programs as
/// possible.  Without a budget the sweep spends its time in the handful of scripts that
/// run tens of millions of ops, and a script a timeout kills reports nothing at all — so
/// the heaviest programs, which exercise the most routes, are exactly the ones that
/// contribute no data.  A budget makes every program report, deterministically, on its
/// first `N` ops.  `0` (the default) is unlimited.
#[must_use]
pub fn max_ops() -> u64 {
    static N: OnceLock<u64> = OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("LOFT_STACK_CENSUS_MAX_OPS")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0)
    })
}

/// Has the op budget been spent?
#[must_use]
pub fn budget_spent(c: &Census) -> bool {
    let n = max_ops();
    n > 0 && c.ops >= n
}

/// One span `put_stack` wrote during the current op: `(absolute byte offset, length)`.
///
/// Absolute means the index into the stack store's own buffer — `rec * 8 + fld`, the
/// same arithmetic `Store::addr_mut` does — so a claim and a diff are in one coordinate
/// system and cannot silently disagree.
type Claim = (u32, u32);

/// `@PLN226` — one interpreter's census (`State::stack_census`).
#[derive(Default)]
pub struct Census {
    /// The stack store's bytes as they stood before the current op, from `origin`.
    snapshot: Vec<u8>,
    /// Absolute offset the snapshot starts at — the stack record's first data byte.
    origin: usize,
    /// Spans `put_stack` claimed during the current op.
    claims: Vec<Claim>,
    ops: u64,
    /// Bytes that changed and lie inside a `put_stack` claim.
    put_bytes: u64,
    /// Bytes that changed and do not.
    other_bytes: u64,
    /// Unattributed bytes and op count, keyed by opcode.
    by_op: HashMap<u16, (u64, u64)>,
    /// Ops during which the buffer itself was reallocated, so no diff was possible.
    grew: u64,
    /// `put_stack` spans that fell outside the watched region — the margin was too
    /// small, and the diff therefore under-reports.  Non-zero means the numbers below
    /// are a floor, not a measurement.
    claims_beyond_span: u64,
    /// Claimed spans that did NOT change — a `put_stack` writing the value already
    /// there.  Counted rather than dropped: it is the reason a claim total and a diff
    /// total do not have to match, and a reader who does not know that will read the
    /// difference as a missing route.
    claimed_unchanged: u64,
}

/// Record that `put_stack` wrote `len` bytes at absolute offset `off`.
pub fn claim(c: &mut Census, off: u32, len: u32) {
    c.claims.push((off, len));
}

/// Snapshot the stack store's bytes before an operator runs.
///
/// The region is `bytes[origin..end]`.  `origin` is the stack record's first data byte,
/// which skips the store header — those bookkeeping bytes belong to the allocator rather
/// than to any opcode.  `end` reaches past the eval top to cover the whole FRAME, because
/// a slot above the top is exactly where a stale value sits; it stops short of the
/// buffer's end only because the allocator's slack grows without bound and diffing it
/// costs more than the corpus can pay.  A `put_stack` span landing outside is counted
/// and reported, so the bound is visible in the output rather than silently narrowing it.
pub fn before_op(c: &mut Census, bytes: &[u8], origin: usize, end: usize) {
    let origin = origin.min(bytes.len());
    let end = end.clamp(origin, bytes.len());
    c.origin = origin;
    c.snapshot.clear();
    c.snapshot.extend_from_slice(&bytes[origin..end]);
    c.claims.clear();
}

/// Diff the stack store against the snapshot and attribute what changed.
pub fn after_op(c: &mut Census, op: u16, bytes: &[u8], origin: usize, end: usize) {
    c.ops += 1;
    let origin = origin.min(bytes.len());
    let end = end.clamp(origin, bytes.len());
    if origin != c.origin || end - origin != c.snapshot.len() {
        // `ensure_stack` grew the buffer, or the frame high-water moved: the two
        // images are not comparable, so count the op and diff nothing rather than
        // reporting the whole resize as a write.
        c.grew += 1;
        return;
    }
    let mut changed_in_claim = 0u64;
    let mut changed_outside = 0u64;
    // A bitmask, not a `Vec<bool>`: this runs once per operator, and an allocation
    // there is the difference between a probe you can point at a corpus and one you
    // cannot.  An op pushes a handful of values, so 64 claims is far above the real
    // maximum; past that the extra claims simply do not record a hit, which can only
    // over-report `claimed_unchanged` and never mis-attribute a byte.
    let mut hit: u64 = 0;
    for i in 0..c.snapshot.len() {
        if bytes[origin + i] == c.snapshot[i] {
            continue;
        }
        let abs = origin + i;
        let idx = c
            .claims
            .iter()
            .position(|&(off, len)| abs >= off as usize && abs < off as usize + len as usize);
        if let Some(k) = idx {
            if k < 64 {
                hit |= 1 << k;
            }
            changed_in_claim += 1;
        } else {
            changed_outside += 1;
        }
    }
    for (k, &(off, len)) in c.claims.iter().enumerate() {
        if k < 64 && hit & (1 << k) != 0 {
            continue;
        }
        if (off as usize) < origin || off as usize + len as usize > end {
            c.claims_beyond_span += 1;
        } else {
            c.claimed_unchanged += u64::from(len);
        }
    }
    c.put_bytes += changed_in_claim;
    c.other_bytes += changed_outside;
    if changed_outside > 0 {
        let e = c.by_op.entry(op).or_insert((0, 0));
        e.0 += changed_outside;
        e.1 += 1;
    }
}

/// Print the census.  Called at program exit when armed.
///
/// The byte totals are cast to `f64` for the percentages; the shares are a reading aid on
/// a report, so a mantissa that cannot hold 2^53 bytes is not a concern here.
#[allow(clippy::cast_precision_loss)]
///
/// `data` resolves an opcode byte to its name; without it the rows still print, keyed by
/// number, because a census that says nothing when the definition table has already gone
/// is worse than one that names the opcodes by number.
pub fn report(c: &Census, data: Option<&crate::data::Data>) {
    if c.ops == 0 {
        crate::loft_eprintln!("stack census: armed, but no operator ran");
        return;
    }
    let total = c.put_bytes + c.other_bytes;
    let pct = |n: u64| {
        if total == 0 {
            0.0
        } else {
            100.0 * n as f64 / total as f64
        }
    };
    crate::loft_eprintln!(
        "stack census: {} ops, {} bytes changed on the stack store",
        c.ops,
        total
    );
    crate::loft_eprintln!(
        "  via put_stack : {:>10}  ({:.2} %)",
        c.put_bytes,
        pct(c.put_bytes)
    );
    crate::loft_eprintln!(
        "  other routes  : {:>10}  ({:.2} %)",
        c.other_bytes,
        pct(c.other_bytes)
    );
    if c.grew > 0 {
        crate::loft_eprintln!("  ops not diffed (buffer or frame resized): {}", c.grew);
    }
    if c.claims_beyond_span > 0 {
        crate::loft_eprintln!(
            "  WARNING: {} put_stack spans fell beyond the watched frame span — \
                 the shares above are a floor",
            c.claims_beyond_span
        );
    }
    if c.claimed_unchanged > 0 {
        crate::loft_eprintln!(
            "  claimed but unchanged bytes (a push of the value already there): {}",
            c.claimed_unchanged
        );
    }
    if c.by_op.is_empty() {
        return;
    }
    let mut rows: Vec<(u16, u64, u64)> = c.by_op.iter().map(|(&op, &(b, n))| (op, b, n)).collect();
    rows.sort_by_key(|r| std::cmp::Reverse(r.1));
    crate::loft_eprintln!("  bytes written by a route other than put_stack, by opcode:");
    for (op, b, n) in rows.iter().take(25) {
        crate::loft_eprintln!(
            "    op {:>3}  {:<28} {:>10} bytes  over {:>8} ops",
            op,
            data.and_then(|d| d.operator_name(*op)).unwrap_or("?"),
            b,
            n
        );
    }
    if rows.len() > 25 {
        crate::loft_eprintln!("    … and {} further opcodes", rows.len() - 25);
    }
}
