// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I67 — Opcode implementations

//! `LOFT_OP_CENSUS=<file>` — what the interpreter EXECUTES, per routine, and the bytes it moves.
//!
//! The question it answers is the one a timing cannot: of the time `--interpret` spends where
//! `--native` is fast, how much is work and how much is the interpreter shuffling values — onto
//! the stack and back, record by record, buffer by buffer.  Every operator is counted exactly
//! (no sampling), keyed by
//!
//! - the SEGMENT: the line of the entry function (`main`) the op ran under — the line of the
//!   call that reached it, or its own line when it ran in `main` itself.  A bench times each
//!   routine between two lines of `main`, so a segment is a routine with no bench-specific code;
//! - the FUNCTION the op belongs to, so a routine's cost splits by the bodies it ran — the unit
//!   the rewrite census counts a native rewrite in (`LOFT_REWRITE_CENSUS_FN`);
//! - the OPERATOR.
//!
//! Beside the count, the bytes the op moved through the store's three block routes:
//! `copy` (`copy_block`: a value copied into place), `relocate` (a record that outgrew its
//! claim moved to a new one) and `text` (text bytes written into a store).  Those counters
//! sit in store code the native runtime shares, so they are compiled in only with the
//! `op-census` feature; without it the bytes read as not counted.
//!
//! The per-op hook rides the debugger branch the sampler already uses (`State::debug_check`),
//! so an unarmed interpreter pays nothing for it.
//!
//! Output, written when the run ends: a `# bytes: counted|not counted` line, then
//! `line<TAB>function<TAB>operator<TAB>count<TAB>copy<TAB>relocate<TAB>text`.  Only the thread that runs `main` is counted: a `par` worker's ops
//! are not in the file.  Read by `scripts/interp_gap.py`.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

/// The block routes a byte can move through.
#[derive(Clone, Copy)]
pub enum Moved {
    Copy = 0,
    Relocate = 1,
    Text = 2,
}

static ARMED: AtomicBool = AtomicBool::new(false);

thread_local! {
    static MOVED: Cell<[u64; 3]> = const { Cell::new([0; 3]) };
    static CENSUS: RefCell<Census> = RefCell::new(Census::default());
}

/// Operators are `u8` plus an extension page: 255 + 256 opcodes at most.
const OPS: usize = 512;

#[derive(Default)]
struct Census {
    index: HashMap<(u32, u32), usize>,
    rows: Vec<Row>,
    cur_key: Option<(u32, u32)>,
    cur: usize,
    /// The op that ran last — `(row, operator)` — and the byte counters when it started:
    /// its bytes are known only when the next op begins.
    last: Option<(usize, usize)>,
    moved_at_last: [u64; 3],
    /// The call from the entry function the current callee chain hangs under, and its line.
    site: Option<(u32, u32)>,
}

struct Row {
    line: u32,
    d_nr: u32,
    count: Vec<u64>,
    bytes: Vec<[u64; 3]>,
}

impl Census {
    /// Credit the bytes moved since the last op began to that op.
    fn settle(&mut self, moved: [u64; 3]) {
        if let Some((row, op)) = self.last.take() {
            for (k, b) in self.rows[row].bytes[op].iter_mut().enumerate() {
                *b += moved[k] - self.moved_at_last[k];
            }
        }
        self.moved_at_last = moved;
    }
}

fn target() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        let p = std::env::var("LOFT_OP_CENSUS")
            .ok()
            .filter(|p| !p.is_empty());
        if p.is_some() {
            ARMED.store(true, Ordering::Relaxed);
        }
        p
    })
    .as_deref()
}

/// Was `LOFT_OP_CENSUS` set?  Read when the run is armed; it also sets [`armed`].
#[must_use]
pub fn enabled() -> bool {
    target().is_some()
}

/// Is the census counting?  One relaxed load: the per-op hook sits on the debugger's
/// branch, so it is only asked when a `Debugger` exists at all.
#[inline]
#[must_use]
pub fn armed() -> bool {
    ARMED.load(Ordering::Relaxed)
}

/// `bytes` moved through `route` by the op now running.  Its call sites are compiled only
/// with the `op-census` feature: the block routes are shared with the native runtime, and a
/// shipped runtime carries no census.  Without the feature the census still counts every
/// op, and says the bytes were not counted.
#[cfg(feature = "op-census")]
#[inline]
pub fn moved(route: Moved, bytes: usize) {
    if ARMED.load(Ordering::Relaxed) {
        count_moved(route, bytes);
    }
}

#[cfg(feature = "op-census")]
#[cold]
#[inline(never)]
fn count_moved(route: Moved, bytes: usize) {
    MOVED.with(|m| {
        let mut v = m.get();
        v[route as usize] += bytes as u64;
        m.set(v);
    });
}

/// The line of the entry function's call at `call_pos`, resolved by `resolve` once per call
/// rather than once per op of the callee chain under it.
pub fn site_line(call_pos: u32, resolve: impl FnOnce() -> u32) -> u32 {
    let known = CENSUS.with_borrow(|c| c.site.filter(|s| s.0 == call_pos).map(|s| s.1));
    known.unwrap_or_else(|| {
        let line = resolve();
        CENSUS.with_borrow_mut(|c| c.site = Some((call_pos, line)));
        line
    })
}

/// Operator `opcode` of `function` (a definition number) is about to run under entry-function
/// `line`: count it, and settle the bytes the previous op moved.  Keyed before the op runs,
/// because a call or a return changes both.
pub fn step(line: u32, function: u32, opcode: u16) {
    let moved = MOVED.with(Cell::get);
    CENSUS.with_borrow_mut(|c| {
        c.settle(moved);
        if c.cur_key != Some((line, function)) {
            let next = c.rows.len();
            let at = *c.index.entry((line, function)).or_insert(next);
            if at == next {
                c.rows.push(Row {
                    line,
                    d_nr: function,
                    count: vec![0; OPS],
                    bytes: vec![[0; 3]; OPS],
                });
            }
            c.cur_key = Some((line, function));
            c.cur = at;
        }
        let op = usize::from(opcode).min(OPS - 1);
        let cur = c.cur;
        c.rows[cur].count[op] += 1;
        c.last = Some((cur, op));
    });
}

/// Write the census to `LOFT_OP_CENSUS`, one line per (segment, function, operator).
pub fn write(data: &crate::data::Data) {
    let Some(path) = target() else { return };
    let mut out = String::from(if cfg!(feature = "op-census") {
        "# bytes: counted\n"
    } else {
        "# bytes: not counted (build with --features op-census)\n"
    });
    let moved = MOVED.with(Cell::get);
    CENSUS.with_borrow_mut(|c| c.settle(moved));
    CENSUS.with_borrow(|c| {
        for row in &c.rows {
            let function = if row.d_nr == u32::MAX {
                "?"
            } else {
                data.def(row.d_nr).name()
            };
            for (op, &n) in row.count.iter().enumerate() {
                if n == 0 {
                    continue;
                }
                let name = u16::try_from(op)
                    .ok()
                    .and_then(|o| data.operator_name(o))
                    .map_or_else(|| format!("op#{op}"), str::to_owned);
                let [copy, relocate, text] = row.bytes[op];
                let _ = writeln!(
                    out,
                    "{}\t{function}\t{name}\t{n}\t{copy}\t{relocate}\t{text}",
                    row.line
                );
            }
        }
    });
    if let Err(e) = std::fs::write(path, out) {
        crate::loft_eprintln!("loft: cannot write the op census to '{path}': {e}");
    }
}
