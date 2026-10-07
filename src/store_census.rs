// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I67 — Opcode implementations

//! `LOFT_STORE_CENSUS=<file>` — the work a run does on its STORES, counted the same way on both
//! backends.
//!
//! The interpreter and a `--native` program share one store implementation (`Store`,
//! `Stores`), so a counter at each of its chokepoints answers *does the interpreter do more
//! work on stores than the compiled code?* without trusting either backend's own account of
//! itself.  Counted: stores created and freed, records claimed, deleted and grown, records
//! moved because they outgrew their claim (and the bytes that moved), block copies and text
//! written into a store (in bytes).
//!
//! Attribution is by `ticks()`: every call appends one line — the clock value it answers, then
//! the counters so far — so the work between two calls is the difference of two lines.  A
//! bench times each routine between two `ticks()` calls, and both backends call the same
//! runtime `ticks()`, so a routine's interval is the one whose clock difference is the time the
//! routine printed.  `scripts/interp_gap.py` does that matching.
//!
//! The counters sit in code the shipped runtime runs, so they are compiled only with the
//! `op-census` feature; without it the file's first line says nothing was counted.

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use std::io::Write as _;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering};

/// One kind of store work.
#[derive(Clone, Copy)]
pub enum Work {
    StoreNew = 0,
    StoreFree = 1,
    Claim = 2,
    Delete = 3,
    Grow = 4,
    Relocate = 5,
    RelocateBytes = 6,
    CopyBytes = 7,
    TextBytes = 8,
    /// Bytes copied INTO the interpreter's stack store — a return value sliding down, an
    /// argument copied in: work on the hot frame, which stays in cache.
    StackBytes = 9,
}

/// The column names, in `Work` order.
pub const COLUMNS: [&str; 10] = [
    "stores_new",
    "stores_freed",
    "claims",
    "deletes",
    "grows",
    "relocations",
    "relocated_bytes",
    "copied_bytes",
    "text_bytes",
    "stack_bytes",
];

static COUNTS: [AtomicU64; 10] = [const { AtomicU64::new(0) }; 10];

thread_local! {
    /// This thread's interpreter stack store, so a copy into it counts as `StackBytes`.
    static STACK_STORE: std::cell::Cell<u16> = const { std::cell::Cell::new(u16::MAX) };
}

/// A `State` on this thread keeps its stack in store `nr`.
pub fn set_stack_store(nr: u16) {
    STACK_STORE.with(|s| s.set(nr));
}

/// `bytes` block-copied into store `dest`: stack work when `dest` is this thread's
/// interpreter stack, copied bytes otherwise.
#[inline]
pub fn copied_into(dest: u16, bytes: u64) {
    if STACK_STORE.with(std::cell::Cell::get) == dest {
        note(Work::StackBytes, bytes);
    } else {
        note(Work::CopyBytes, bytes);
    }
}
/// 0 = not read yet, 1 = off, 2 = on.
static STATE: AtomicU8 = AtomicU8::new(0);
static STARTED: AtomicBool = AtomicBool::new(false);

fn target() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        std::env::var("LOFT_STORE_CENSUS")
            .ok()
            .filter(|p| !p.is_empty())
    })
    .as_deref()
}

#[cfg_attr(not(feature = "op-census"), allow(dead_code))]
fn armed() -> bool {
    match STATE.load(Ordering::Relaxed) {
        0 => {
            let on = target().is_some();
            STATE.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
        s => s == 2,
    }
}

/// `n` units of `work`.  Compiled only with the `op-census` feature; one relaxed load when the
/// census is off.
#[cfg(feature = "op-census")]
#[inline]
pub fn note(work: Work, n: u64) {
    if armed() {
        COUNTS[work as usize].fetch_add(n, Ordering::Relaxed);
    }
}

/// Without the feature the chokepoints count nothing.
#[cfg(not(feature = "op-census"))]
#[inline(always)]
pub fn note(_work: Work, _n: u64) {}

/// `ticks()` answered `value`: append the counters so far.  The first call of the run starts
/// the file afresh with its header.
pub fn at_ticks(value: i64) {
    let Some(path) = target() else { return };
    let first = !STARTED.swap(true, Ordering::Relaxed);
    let mut line = String::new();
    if first {
        line.push_str(if cfg!(feature = "op-census") {
            "# store work: counted\n"
        } else {
            "# store work: not counted (build with --features op-census)\n"
        });
        line.push_str("ticks");
        for c in COLUMNS {
            line.push('\t');
            line.push_str(c);
        }
        line.push('\n');
    }
    line.push_str(&value.to_string());
    for c in &COUNTS {
        line.push('\t');
        line.push_str(&c.load(Ordering::Relaxed).to_string());
    }
    line.push('\n');
    let file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(!first)
        .truncate(first)
        .open(path);
    if let Ok(mut f) = file {
        let _ = f.write_all(line.as_bytes());
    }
}
