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
#[cfg(feature = "op-census")]
use std::sync::atomic::AtomicU32;
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

/// `@PLN226` — one context's store instruments: the op its interpreter runs and the program's
/// line table (`LOFT_ALLOC_TRACE`'s `@ <line>`), the `par` worker it is, its interpreter's stack
/// store (`StackBytes`) and the bytes its stores moved (`LOFT_OP_CENSUS`).  A `Stores` owns one
/// (`Stores::tap`) and every store it makes live carries a share (`Store::tap`), so store code
/// reaches its context without asking which thread runs (`@FR-C-Boundary`).  Only the owning
/// context's thread writes it (`@FR-C-Own`); the words are atomics only so a `Store`, which is
/// `Send`, can hold the share.
#[cfg(feature = "op-census")]
pub struct Tap {
    pc: AtomicU32,
    last_line: AtomicU32,
    lines: OnceLock<std::sync::Arc<std::collections::BTreeMap<u32, u32>>>,
    worker: AtomicU32,
    stack_store: AtomicU32,
    moved: [AtomicU64; 3],
}

#[cfg(feature = "op-census")]
impl Default for Tap {
    fn default() -> Self {
        Tap {
            pc: AtomicU32::new(0),
            last_line: AtomicU32::new(0),
            lines: OnceLock::new(),
            worker: AtomicU32::new(0),
            stack_store: AtomicU32::new(u32::MAX),
            moved: [const { AtomicU64::new(0) }; 3],
        }
    }
}

#[cfg(feature = "op-census")]
impl Tap {
    /// A new context's tap (a worker's `Stores`): the program's line table carried over, every
    /// count fresh.
    #[must_use]
    pub fn child(&self) -> Self {
        let tap = Tap::default();
        if let Some(lines) = self.lines.get() {
            let _ = tap.lines.set(lines.clone());
        }
        tap
    }

    /// The context's interpreter keeps its stack in store `nr`.
    pub fn set_stack_store(&self, nr: u16) {
        self.stack_store.store(u32::from(nr), Ordering::Relaxed);
    }

    /// `bytes` block-copied into store `dest`: stack work when `dest` is the interpreter's
    /// stack, copied bytes otherwise.
    #[inline]
    pub fn copied_into(&self, dest: u16, bytes: u64) {
        if self.stack_store.load(Ordering::Relaxed) == u32::from(dest) {
            note(Work::StackBytes, bytes);
        } else {
            note(Work::CopyBytes, bytes);
        }
    }

    /// `bytes` moved through block route `route` (`op_census::Moved`).
    pub fn count_moved(&self, route: crate::op_census::Moved, bytes: u64) {
        self.moved[route as usize].fetch_add(bytes, Ordering::Relaxed);
    }

    /// The bytes moved so far, per route.
    #[must_use]
    pub fn moved(&self) -> [u64; 3] {
        [0, 1, 2].map(|k| self.moved[k].load(Ordering::Relaxed))
    }

    /// The `par` worker this context is, 0 for the main thread's (`Store::trace_worker`).
    #[must_use]
    pub fn worker(&self) -> u32 {
        self.worker.load(Ordering::Relaxed)
    }

    /// The op at `pc` is about to run in this context.
    #[inline]
    pub fn at_pc(&self, pc: u32) {
        self.pc.store(pc, Ordering::Relaxed);
    }

    /// The program's line table, for the sites of the events that follow; the first table
    /// a context is handed stays.
    pub fn set_lines(&self, lines: std::collections::BTreeMap<u32, u32>) {
        let _ = self.lines.set(std::sync::Arc::new(lines));
    }

    /// The `@ <line>` to write before an event, when the context's line moved since the last
    /// one.
    fn site_line(&self) -> Option<u32> {
        let pc = self.pc.load(Ordering::Relaxed);
        let lines = self.lines.get()?;
        let line = lines.range(..=pc).next_back().map_or(0, |(_, &l)| l);
        (line != 0 && self.last_line.swap(line, Ordering::Relaxed) != line).then_some(line)
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

// `LOFT_ALLOC_TRACE=<file>` — every allocator EVENT of the run, one line each, in the order
// they happen: `N <store> <capacity>` a store made (or a freed slot made again), `F <store>`
// a store freed, `C <store> <words> <position>` a claim (the words the record CLAIMED, which a
// whole free block can make more than were asked), `D <store> <position>` a delete,
// `R <store> <position> <words> <position>` a resize that grew in place or moved the record
// (the moving one after the claim and delete it makes), `S <store>` a store the interpreter
// state makes for itself (its value stack, its constant store), which a C unit has no twin of,
// and `@ <line>` the source line of the events after it, written when it changes.  A store is named by the order it was made (`alloc_serial`), not
// by its slot: the interpreter's slot 0 is its value stack, native's an ordinary store, so
// the two number stores differently while placing every record alike.  It is what a second
// runtime compares itself against to keep loft's memory model rather than one of its own
// (@PLN200 B5) — the C runtime writes the same lines.  Compiled only with `op-census`; the
// file is written line by line, unbuffered, so a run that aborts keeps what it did.  Every
// call site is itself `#[cfg(feature = "op-census")]`: a build without the feature compiles
// no trace code at all.
/// A par worker's context is being built: its stores take a worker's names
/// (`<worker>.<serial>`), each worker numbering its stores on its own.
#[cfg(feature = "op-census")]
pub fn enter_worker(tap: &Tap) {
    static NEXT: AtomicU32 = AtomicU32::new(1);
    tap.worker
        .store(NEXT.fetch_add(1, Ordering::Relaxed), Ordering::Relaxed);
}

/// `LOFT_ALLOC_TRACE`'s par regions: `P` where a `par` loop's queue starts (`n_parallel_queue…`)
/// and `Q` where its buffer is dropped (`n_parallel_buf_drop…`), as the C runtime marks the same
/// two calls — what lies between is the par machinery, each runtime's own (STORE_MODEL D46).
#[inline]
pub fn par_mark(mark: char) {
    #[cfg(feature = "op-census")]
    alloc_event(None, format_args!("{mark}"));
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

/// One `LOFT_ALLOC_TRACE` event, preceded by its source line when that moved; `tap` is the
/// context of the store the event happened in, `None` for a store no context made live.
#[cfg(feature = "op-census")]
pub fn alloc_event(tap: Option<&Tap>, line: std::fmt::Arguments) {
    use std::sync::Mutex;
    let text = line.to_string();
    let site = if text.starts_with(['M', 'S', 'P', 'Q']) {
        None
    } else {
        tap.and_then(Tap::site_line)
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
