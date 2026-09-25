// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F51 — debugger (breakpoints, frame capture, scripted RPC)

//! @PLN16 debugger — breakpoint registry + captured-frame records.
//!
//! The interpreter's execute loop (`src/state/mod.rs`) consults the optional
//! [`Debugger`] attached to a [`State`](crate::state::State): when the program
//! counter reaches a registered offset it pauses and captures the live frame as a
//! [`BreakHit`].  This is the tracer-bullet slice — record the frame and continue;
//! suspending into a REPL-at-frame and stepping build on the same hook.
//!
//! When a `State` has no debugger (`debug == None`, the normal case) the only
//! per-op cost is one `Option::is_some` branch — there is already a per-op
//! `crash_report` store in that loop, so this is in the noise.

use std::collections::HashSet;

/// The four @PLN16 F step verbs, driving
/// [`State::debug_step`](crate::state::State::debug_step).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StepMode {
    /// Pause at the next source line, **descending into** any call on the way.
    Into,
    /// Run any call on the current line to completion, then pause at the next line
    /// in the same (or a shallower) frame — **over** the call.
    Over,
    /// Run to the current function's return and pause in the caller — **out**.
    Out,
    /// Run to the next breakpoint (or program end) — **continue**.
    Continue,
}

/// A frame captured at a breakpoint hit.
#[derive(Debug, Clone)]
pub struct BreakHit {
    /// The function the breakpoint sits in (user name, no `n_` prefix).
    pub function: String,
    /// The in-scope variables at the pause, each rendered to loft source
    /// (`("n", "42")`).  The slice captures arguments (live at fn entry); a later
    /// slice that breaks mid-body adds the locals assigned so far.
    pub locals: Vec<(String, String)>,
    /// @PLN120 A — the names in `locals` the frame does **not** hold: their entry
    /// carries a reason (`<unset>`, `<reused by …>`) where a value would be.
    ///
    /// Carried rather than recognised from the rendering, because a rendering can
    /// look exactly like a marker: a keyed collection renders as
    /// `<hash<HRec,["name"]>>`.  Every path that rebuilds loft source from a captured
    /// frame filters on this list ([`held_locals`](Self::held_locals)); a marker in
    /// that position would poison the parse.
    pub unheld: Vec<String>,
    /// The source line this frame is parked on — the breakpoint line for the top
    /// frame, the call-site line for a caller (the multi-frame `stackTrace`, @PLN63
    /// SF).  0 when unknown.
    pub line: u32,
}

impl BreakHit {
    /// The locals worth SHOWING — everything except the compiler's own scratch:
    /// `__work_N` format buffers, `__ref_N` return buffers, `___`-prefixed
    /// internals, and the `#`-infixed loop machinery (`i#index`, `c#next`).
    /// @PLN120 D1.
    ///
    /// The `#` names became filterable with @PLN120 A: the iteration variable the
    /// user wrote (`i`) is now in the frame, so its hidden counter is no longer the
    /// only signal about loop position.  A `#` cannot occur in a loft identifier, so
    /// this can never hide a name the user chose.
    ///
    /// This is display-only: `debug_eval` still resolves any name, so a compiler
    /// temp remains readable by typing it, and `:vars all` shows the lot.
    #[must_use]
    pub fn user_locals(&self) -> Vec<&(String, String)> {
        self.locals
            .iter()
            .filter(|(n, _)| !n.starts_with("__") && !n.contains('#'))
            .collect()
    }

    /// The locals the frame actually HOLDS — everything whose entry is a value.
    /// @PLN120 A: the frame also lists locals it does not hold (`<unset>`,
    /// `<reused by …>`), and every path that rebuilds loft source from a captured
    /// frame must skip those, because a marker does not parse.
    pub fn held_locals(&self) -> impl Iterator<Item = &(String, String)> {
        self.locals
            .iter()
            .filter(|(n, _)| !self.unheld.iter().any(|u| u == n))
    }
}

/// @PLN16 M3 — a **watchpoint** (data breakpoint): a fixed heap region whose bytes are
/// re-read after each op of a resumed run; when they differ from `last`, execution
/// pauses.  `content` is the scalar primitive type number (the `ShowDb` map) for
/// rendering old → new.  A heap region (a struct field / vector element) survives frame
/// exit; a **stack** region (a bare scalar local — @PLN63 DB) is bound to the frame it was
/// set in (`frame`) and dropped the moment that frame returns, so a reused slot never fires.
#[derive(Debug, Clone)]
pub struct Watchpoint {
    /// The source expression the user watched (`pt.x`, `v[0]`, `n`), for display.
    pub label: String,
    pub store_nr: u16,
    pub rec: u32,
    pub off: u32,
    pub len: u32,
    /// Scalar primitive type number (0 integer · 2 single · 3 float · 4 boolean ·
    /// 6 character), used to render the value.
    pub content: u16,
    /// The region's bytes at the last poll; a change from these is what fires.
    pub last: Box<[u8]>,
    /// The call frame a **stack**-local watch is bound to; `None` for a heap watch (which
    /// survives frame exit).  A stack watch is dropped once its frame is no longer live.
    pub frame: Option<StackWatchFrame>,
    /// The field encoding a LINKED narrow local's slot holds (@PLN167 decision 1), or `None`
    /// for every other target.  `content` alone cannot carry it: it names the primitive KIND
    /// and two locals of one kind may store differently, so the decode rides beside it rather
    /// than inside it.  Without this the watch snapshotted eight bytes of a one-byte slot and
    /// reported the stored codes — an `i8` going `-1 -> 5 -> -7` read `127 -> 133 -> 121`.
    pub narrow: Option<crate::data::NarrowSlot>,
}

/// @PLN63 DB — the identity of the call frame a stack-local watch belongs to.  A watch is
/// live only while `call_stack[depth]` is still this exact frame; once it returns (the entry
/// shrinks away or is a different `(d_nr, args_base)`), the slot is dead and the watch drops.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StackWatchFrame {
    /// The watched frame's function.
    pub d_nr: u32,
    /// The frame's argument-region base (its stack identity at a given depth).
    pub args_base: u32,
    /// The frame's index in the `call_stack` when the watch was set.
    pub depth: u32,
}

/// @PLN120 F — one undoable edit: the journal that reverts it, plus what it edited.
///
/// The binding is what makes an entry survive a step. A `Journal` alone records raw
/// store regions, so after a step nothing could tell a slot that is still the edited
/// local's from one the allocator has re-handed to another local — which is why the
/// history used to be dropped wholesale at every step, silently discarding edits that
/// were perfectly valid. Carrying the frame identity and the local names lets each
/// entry be checked instead of all of them assumed dead.
#[derive(Debug)]
pub struct UndoEntry {
    /// Reverts (`revert`) / re-applies (`apply`) this one edit.
    pub journal: crate::database::Journal,
    /// The edit's left-hand side as the user typed it (`total`, `pt.x`, `v[1]`), for
    /// the message when the entry can no longer be applied.
    pub label: String,
    /// `Some` when the journal touched FRAME storage — the frame it was recorded in,
    /// bound exactly as a stack watchpoint binds (a recursive re-entry of the same
    /// function is a different frame, hence `args_base` + `depth`, not just `d_nr`).
    /// `None` for a pure-heap edit: a heap record's address survives frame exit and
    /// every step, so no frame change can invalidate it.
    pub frame: Option<StackWatchFrame>,
    /// The frame slots the journal wrote, as `(local name, slot offset)`.  The NAME is
    /// load-bearing: two locals share a slot whenever their live ranges do not overlap,
    /// so "some local is live at this slot" would let an undo of one write the other's
    /// value.
    pub slots: Vec<(String, u16)>,
}

/// @PLN16 M3 — a watchpoint that just fired: the label and the rendered old → new value.
#[derive(Debug, Clone)]
pub struct WatchHit {
    pub label: String,
    pub old: String,
    pub new: String,
}

/// Debug state attached to a [`State`](crate::state::State) while debugging.
/// Absent on normal runs.
#[derive(Debug, Default)]
pub struct Debugger {
    /// Bytecode offsets that trigger a pause when execution reaches them.
    breakpoints: HashSet<u32>,
    /// Frames captured at each hit, in hit order (record-and-continue mode).
    pub hits: Vec<BreakHit>,
    /// **Stepping mode**: when set, a breakpoint *suspends* execution (the loop
    /// returns to the driver) instead of recording-and-continuing — so a value can
    /// be edited and `resume`d.  Off by default (record-and-continue).
    pub stepping: bool,
    /// The frame captured at the current suspension (stepping mode); `None` while
    /// running.  The driver reads it, optionally writes a value back to the live
    /// frame, and calls `State::resume`.
    pub paused: Option<BreakHit>,
    /// Stable backing buffers for **text-argument** live edits.  A text argument's
    /// frame slot is a 16-byte `Str` borrow (ptr + len), so an edit must point it
    /// at bytes that outlive the edit; each edited string is kept here (owned by the
    /// `Debugger`, which lives for the whole run) and the slot's `Str` points into
    /// it.  A text *local* needs no entry — it owns its `String` and is overwritten
    /// in place.  Never mutated or removed after a push, so the pointers stay valid.
    pub edited_text: Vec<String>,
    /// @PLN16 M2 — undo/redo.  While one interactive edit runs, the before/after bytes
    /// of every frame/heap region it overwrites are recorded here (armed by
    /// `State::begin_edit_journal`); on success the journal moves to `undo_stack`.
    /// `None` between edits, so non-interactive writes pay nothing.
    pub recording_edit: Option<crate::database::Journal>,
    /// @PLN16 M2 — edits made at the current suspension, newest last.  `:undo` reverts
    /// the top entry onto `redo_stack`; a fresh edit forks the timeline (clears redo).
    /// Per-edit [`UndoEntry`]s — each a self-contained revert/apply unit — reuse the one
    /// store change journal engine.
    ///
    /// @PLN120 F: **validated** at each new pause rather than cleared. Stepping reuses
    /// frame slots, so an entry whose slot now belongs to another local (or whose frame
    /// has returned) is dropped *and reported*; one whose storage is still its own
    /// survives, which is the common case — a long-lived accumulator keeps its slot for
    /// the whole function.
    pub undo_stack: Vec<UndoEntry>,
    /// @PLN16 M2 — undone edits available to `:redo`, newest last (see `undo_stack`).
    /// Validated by the same rule: a redo replays the same regions forward.
    pub redo_stack: Vec<UndoEntry>,
    /// @PLN120 F — entries the last validation dropped, as `(label, reason)`, for the
    /// driver to report once.  An edit that silently stops being undoable is the
    /// failure this arc exists to close, so the drop is never wordless.
    pub dropped_undo: Vec<(String, String)>,
    /// @PLN16 M3 — active watchpoints, polled after each op of a resumed run.
    pub watchpoints: Vec<Watchpoint>,
    /// @PLN16 M3 — the watchpoint that fired during the most recent resume (for the
    /// driver to report), cleared at the start of each resume.
    pub last_watch: Option<WatchHit>,
    /// @PLN63 RX — reverse-stepping armed.  When set, each `debug_step` checkpoints the
    /// pre-step state into `reverse_ring` before executing, so `step_back` can restore it.
    /// Off by default (normal debug sessions pay no snapshot cost).
    pub reverse: bool,
    /// @PLN63 RX — the reverse-step checkpoint ring, newest at the back.  Distinct from the M2
    /// `undo_stack` (which a step self-clears): a `debug_step` PUSHES the back, `step_back`
    /// POPS the back + restores.  **Bounded** to `reverse_cap` (RX3): a push past the cap drops
    /// the oldest (front), so only the last N steps are reversible.
    pub reverse_ring: std::collections::VecDeque<crate::state::StepCheckpoint>,
    /// @PLN63 RX3 — the ring's capacity (the reversible depth), set when reverse is armed from
    /// `LOFT_REVERSE_DEPTH` (default 200).  0 = unset (never, while armed).
    pub reverse_cap: usize,
    /// @PLN140 arc B/C — the loft-level sampler, when `LOFT_PROFILE` /
    /// `LOFT_ALLOC_PATHS` armed it.
    ///
    /// It lives here rather than beside it on [`State`](crate::state::State) for one
    /// reason: the dispatch loop already runs `if self.debug.is_some()` on every op,
    /// so a profiler reached through that branch adds **nothing** to a run that is
    /// not profiling. A field of its own would have cost every program a branch to
    /// buy a facility almost no run uses.
    pub prof: Option<Box<crate::profiler::Profiler>>,
}

/// @PLN16 B1 — collect the **static** call targets in an IR body into `out`.
///
/// Exhaustive over every [`Value`](crate::data::Value) variant **with no `_`
/// wildcard**: a new variant must be added here or the build breaks, so a call
/// nested in a new construct can never be silently dropped (the under-reach hazard
/// — a missed caller would leave its frame non-interpreted, hence
/// non-introspectable).  Only `Value::Call` carries a static target; `CallRef` is
/// an indirect call through a fn-ref value, unresolvable statically (a documented
/// limitation — a breakpoint reached *only* via a fn-ref won't pull its caller in).
fn collect_calls(value: &crate::data::Value, out: &mut HashSet<u32>) {
    use crate::data::Value;
    match value {
        Value::Call(d_nr, args) => {
            out.insert(*d_nr);
            for a in args {
                collect_calls(a, out);
            }
        }
        Value::CallRef(_, args)
        | Value::Insert(args)
        | Value::Tuple(args)
        | Value::Parallel(args) => {
            for a in args {
                collect_calls(a, out);
            }
        }
        Value::Span(b) => collect_calls(&b.1, out),
        Value::Set(_, inner)
        | Value::Return(inner)
        | Value::Drop(inner)
        | Value::TuplePut(_, _, inner)
        | Value::Yield(inner) => collect_calls(inner, out),
        Value::If(c, t, f) => {
            collect_calls(c, out);
            collect_calls(t, out);
            collect_calls(f, out);
        }
        Value::Iter(_, a, b, c) => {
            collect_calls(a, out);
            collect_calls(b, out);
            collect_calls(c, out);
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for o in &bl.operators {
                collect_calls(o, out);
            }
        }
        // Leaves — no nested `Value`, nothing to collect.  Listed explicitly (no
        // `_`) so a future variant is a compile error, not a silent miss.
        Value::Null
        | Value::Line(_)
        | Value::Int(_)
        | Value::Enum(_, _)
        | Value::Boolean(_)
        | Value::Float(_)
        | Value::Long(_)
        | Value::Single(_)
        | Value::Text(_)
        | Value::Var(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Keys(_)
        | Value::TupleGet(_, _)
        | Value::FnRef(_, _, _)
        | Value::FnRefDnr(_)
        | Value::RawExpr(_) => {}
    }
}

/// @PLN16 B1 — the functions directly called by definition `d_nr` (its static
/// callees).
#[must_use]
pub fn callees(data: &crate::data::Data, d_nr: u32) -> HashSet<u32> {
    let mut out = HashSet::new();
    if (d_nr as usize) < data.definitions() as usize {
        collect_calls(&data.def(d_nr).code, &mut out);
    }
    out
}

/// @PLN16 B1 — the set of functions that must run **interpreted** for a breakpoint
/// in `bp_fn` to fire with an introspectable stack: `bp_fn` itself plus every
/// function that can **transitively reach** it (so whatever call path runs, every
/// frame from the entry down to the breakpoint is interpreted; `bp_fn`'s *callees*
/// may stay compiled).  A fixpoint over the static call graph: a function joins the
/// set once any function it calls is already in it.
#[must_use]
pub fn interpret_set(data: &crate::data::Data, bp_fn: u32) -> HashSet<u32> {
    let n = data.definitions();
    let callees: Vec<HashSet<u32>> = (0..n).map(|d| callees(data, d)).collect();
    let mut set = HashSet::new();
    set.insert(bp_fn);
    loop {
        let mut added = false;
        for (d, calls) in callees.iter().enumerate() {
            let d = d as u32;
            if !set.contains(&d) && calls.iter().any(|c| set.contains(c)) {
                set.insert(d);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    set
}

/// @PLN16 B2 — switch the functions a breakpoint needs **back to interpreted**
/// under mixed execution: clear the default-native mark (`def.native`) on every
/// **body-bearing** function in [`interpret_set`]`(bp_fn)` (the breakpoint fn plus
/// its transitive callers).  The next compile then routes their calls to the
/// **interpreted** bytecode body (`OpCall`) instead of the compiled cdylib bridge
/// (`OpStaticCall`) — and the breakpoint check lives in the interpreter loop, so a
/// frame that ran compiled could never pause.  Returns how many functions were
/// un-marked.
///
/// **Mechanism:** the compiled-vs-interpret choice is made at codegen, keyed on the
/// callee's `def.native` (set → `OpStaticCall`; empty + a user body → `OpCall`).
/// So the switch is *un-mark + recompile*, **not** a `library` swap — a library
/// dispatcher (`fn(&mut Stores, &mut DbRef)`) has no `State`, so it cannot re-enter
/// the interpreter to run a body.
///
/// **Body-bearing only:** a pure-cdylib function (no loft body, `code == Null`) has
/// nothing to interpret, so it stays marked — an **absolute boundary**, a breakpoint
/// inside it can never fire.  Idempotent (an already-interpreted fn is skipped).  The
/// caller recompiles afterwards (the per-fn engine does this each generation); on the
/// all-interpreted REPL every user fn is already unmarked, so this is a no-op there.
pub fn unmark_for_debug(data: &mut crate::data::Data, bp_fn: u32) -> usize {
    let set = interpret_set(data, bp_fn);
    let mut count = 0;
    for d in set {
        let def = data.def(d);
        if !def.native().is_empty() && *def.code() != crate::data::Value::Null {
            data.def_mut(d).native = String::new();
            count += 1;
        }
    }
    count
}

/// A pause asked for from OUTSIDE the running program — a DAP `pause`, read on another thread
/// while the debuggee runs.  The debug loops consume it at their next stop check and suspend
/// there, as at a breakpoint.  Process-wide because a debugger process debugs one program.
pub static INTERRUPT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Set by a debug loop when it suspended for [`INTERRUPT`] and for nothing else, so the report
/// names the stop `pause` rather than `breakpoint`.  Consumed by the report.
pub static STOPPED_BY_INTERRUPT: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Take a pending [`INTERRUPT`]: true once per pause request.
#[must_use]
pub fn take_interrupt() -> bool {
    INTERRUPT.swap(false, std::sync::atomic::Ordering::Relaxed)
}

impl Debugger {
    /// Register a bytecode offset as a breakpoint.
    pub fn add_offset(&mut self, offset: u32) {
        self.breakpoints.insert(offset);
    }

    /// Whether `offset` is a registered breakpoint.
    #[must_use]
    pub fn is_breakpoint(&self, offset: u32) -> bool {
        self.breakpoints.contains(&offset)
    }
}
