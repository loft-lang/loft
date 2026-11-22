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
    PATH.get_or_init(|| crate::platform::env_output("LOFT_STORE_CENSUS"))
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
    if path == "-" {
        eprint!("{line}");
        return;
    }
    let file = crate::file_access::open_with(
        path,
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(!first)
            .truncate(first),
    );
    if let Ok(mut f) = file {
        let _ = f.write_all(line.as_bytes());
    }
}

/// `LOFT_ALLOC_TRACE=<file>` — every allocator EVENT of the run, one line each, in the order
/// they happen: `N <store> <capacity>` a store made (or a freed slot made again), `F <store>`
/// a store freed, `C <store> <words> <position>` a claim (the words the record CLAIMED, which a
/// whole free block can make more than were asked), `D <store> <position>` a delete,
/// `R <store> <position> <words> <position>` a resize that grew in place or moved the record
/// (the moving one after the claim and delete it makes), `S <store>` a store the interpreter
/// state makes for itself (its value stack, its constant store), which a C unit has no twin of,
/// and `@ <line>` the source line of the events after it, written when it changes.  A store is named by the order it was made (`alloc_serial`), not
/// by its slot: the interpreter's slot 0 is its value stack, native's an ordinary store, so
/// the two number stores differently while placing every record alike.  It is what a second
/// runtime compares itself against to keep loft's memory model rather than one of its own
/// (@PLN200 B5) — the C runtime writes the same lines.  Compiled only with `op-census`; the
/// file is written line by line, unbuffered, so a run that aborts keeps what it did.  Every
/// call site is itself `#[cfg(feature = "op-census")]`: a build without the feature compiles
/// no trace code at all.
/// `LOFT_ALLOC_TRACE`'s sites: the bytecode position of the op running on this thread, and the
/// interpreter's line table (`State::line_numbers`, handed over when the program starts), so an
/// event can be preceded by `@ <line>` when its source line is not the last one written — the
/// line a comparison names beside a difference (the C runtime writes the same).  A worker
/// thread has no table and writes no site.
#[cfg(feature = "op-census")]
thread_local! {
    static SITE_PC: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
    static SITE_LINES: std::cell::RefCell<Option<(std::collections::BTreeMap<u32, u32>, u32)>> =
        const { std::cell::RefCell::new(None) };
}

/// `LOFT_ALLOC_TRACE`'s par workers: each worker thread numbers its stores on its own, so the
/// trace names a worker's store `<worker>.<serial>` — one name per store however the threads
/// interleave (0 is the main thread, whose names stay bare).
#[cfg(feature = "op-census")]
thread_local! {
    static WORKER: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// A par worker's state is being built on this thread: its stores take a worker's names.
#[cfg(feature = "op-census")]
pub fn enter_worker() {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);
    WORKER.with(|w| w.set(NEXT.fetch_add(1, Ordering::Relaxed)));
}

/// `LOFT_ALLOC_TRACE`'s par regions: `P` where a `par` loop's queue starts (`n_parallel_queue…`)
/// and `Q` where its buffer is dropped (`n_parallel_buf_drop…`), as the C runtime marks the same
/// two calls — what lies between is the par machinery, each runtime's own (STORE_MODEL D46).
#[inline]
pub fn par_mark(mark: char) {
    #[cfg(feature = "op-census")]
    alloc_event(format_args!("{mark}"));
    #[cfg(not(feature = "op-census"))]
    let _ = mark;
}

/// Is `LOFT_ALLOC_TRACE` set?  The interpreter then runs its full loop, which publishes every
/// op's position ([`at_pc`]).
#[cfg(feature = "op-census")]
pub fn alloc_trace_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| crate::platform::env_output("LOFT_ALLOC_TRACE").is_some())
}

/// The op at `pc` is about to run on this thread.
#[cfg(feature = "op-census")]
#[inline]
pub fn at_pc(pc: u32) {
    SITE_PC.with(|c| c.set(pc));
}

/// The program's line table, for the sites of the events that follow.
#[cfg(feature = "op-census")]
pub fn set_lines(lines: std::collections::BTreeMap<u32, u32>) {
    SITE_LINES.with_borrow_mut(|l| *l = Some((lines, 0)));
}

/// The `@ <line>` to write before an event, when this thread's line moved since the last one.
#[cfg(feature = "op-census")]
fn site_line() -> Option<u32> {
    let pc = SITE_PC.with(std::cell::Cell::get);
    SITE_LINES.with_borrow_mut(|l| {
        let (lines, last) = l.as_mut()?;
        let line = lines.range(..=pc).next_back().map_or(0, |(_, &l)| l);
        (line != 0 && line != *last).then(|| {
            *last = line;
            line
        })
    })
}

#[cfg(feature = "op-census")]
pub fn alloc_event(line: std::fmt::Arguments) {
    use std::sync::Mutex;
    let mut text = line.to_string();
    let worker = WORKER.with(std::cell::Cell::get);
    if worker != 0 && !text.starts_with(['M', 'P', 'Q']) {
        let mut parts = text.splitn(3, ' ');
        if let (Some(kind), Some(store)) = (parts.next(), parts.next()) {
            let rest = parts.next().map_or(String::new(), |r| format!(" {r}"));
            text = format!("{kind} {worker}.{store}{rest}");
        }
    }
    let site = if text.starts_with(['M', 'S', 'P', 'Q']) {
        None
    } else {
        site_line()
    };
    static FILE: OnceLock<Option<Mutex<crate::file_access::File>>> = OnceLock::new();
    static STDERR: OnceLock<bool> = OnceLock::new();
    if *STDERR
        .get_or_init(|| crate::platform::env_output("LOFT_ALLOC_TRACE").is_some_and(|p| p == "-"))
    {
        if let Some(at) = site {
            eprintln!("@ {at}");
        }
        eprintln!("{text}");
        return;
    }
    let file = FILE.get_or_init(|| {
        let path = crate::platform::env_output("LOFT_ALLOC_TRACE")?;
        crate::file_access::create(path.as_str())
            .ok()
            .map(Mutex::new)
    });
    if let Some(f) = file
        && let Ok(mut f) = f.lock()
    {
        if let Some(at) = site {
            let _ = writeln!(f, "@ {at}");
        }
        let _ = writeln!(f, "{text}");
    }
}
