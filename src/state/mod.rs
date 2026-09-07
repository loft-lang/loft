// Copyright (c) 2024-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I66 — Bytecode VM / executor

#![allow(dead_code)]

pub(crate) mod codegen;
pub mod debug;
mod io;
mod text;
pub use text::text_timeline_summary;

use crate::data::{Context, Data, Type};
pub use crate::database::Call;
use crate::database::{ParallelCtx, Stores, WorkerStores};
use crate::fill::OPERATORS;
use crate::keys::{DbRef, Str};
use crate::lexer::Position;
use crate::log_config::LogConfig;
use crate::variables::size as var_size;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{Error, Write};
use std::sync::Arc;
use std::sync::OnceLock;

/// @PLN11 N3 — a **usage sentinel** on the auto-native shared-store bridge
/// (`extensions::shared_store_dispatch`): every C71 default-native library call that
/// dispatches to its compiled cdylib increments this.  It backs the liveness
/// regression guard (`tests/n2_cdylib.rs::f3_body_bearing_marked_fn_dispatch_vs_interpret`):
/// output-parity tests can't tell a real dispatch from interpreting the body (both
/// give correct output), so the guard asserts this counter *moved*.  Relaxed + a
/// cold path (once per native-lib call, not per opcode) → negligible.
pub static SHARED_DISPATCH_HITS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Plan-07 phase 4g.3 — read once at first raise.  When set
/// (env var `LOFT_DEV_SOFT_HALT=1` or CLI flag `--dev-soft-halt`
/// which exports the env var), demote dev-mode raises to
/// log-and-continue so a single run surfaces every fault site.
fn dev_soft_halt_enabled() -> bool {
    static FLAG: OnceLock<bool> = OnceLock::new();
    *FLAG.get_or_init(|| std::env::var("LOFT_DEV_SOFT_HALT").is_ok_and(|v| v == "1" || v == "true"))
}

pub const STRING_NULL: &str = "\0";

/// One entry in the shadow call-frame vector (TR1.1).
/// Pushed by `fn_call`, popped by `fn_return`.  Stores enough information for
/// `stack_trace()` to reconstruct function names, source lines, and argument
/// values without walking the raw bytecode stack.
#[derive(Clone, Debug)]
pub struct CallFrame {
    /// Definition number of the called function.
    pub d_nr: u32,
    /// Bytecode position of the call instruction (for line-number lookup).
    pub call_pos: u32,
    /// Absolute stack position of the first argument byte.
    pub args_base: u32,
    /// Total byte size of all parameters.
    pub args_size: u16,
    /// Source line number of the call site (TR1.4).  0 if unknown.
    pub line: u32,
}

/// Reserved store number for coroutine `DbRef` encoding (CO1.1).
/// Cannot clash with real Stores allocations (limited by `Stores::max`).
pub const COROUTINE_STORE: u16 = u16::MAX;

/// Lifecycle state of a coroutine frame (CO1.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CoroutineStatus {
    Created,
    Suspended,
    Running,
    Exhausted,
}

/// @PLN63 RX1 — a checkpoint of the execution-mutable [`State`] for the reverse-step ring: the
/// heap ([`HeapSnapshot`](crate::database::HeapSnapshot)) plus every register a step moves.  The
/// compile-time maps (`bytecode`, `vars`, `calls`, `line_numbers`, `const_refs`, …) are
/// execution-invariant, so they are NOT captured.  Restoring one (`State::restore_checkpoint`)
/// makes the state byte-identical to when it was taken — the basis for `step_back` (RX2).
pub struct StepCheckpoint {
    heap: crate::database::HeapSnapshot,
    code_pos: u32,
    call_stack: Vec<CallFrame>,
    stack_cur: DbRef,
    stack_high: u32,
    stack_pos: u32,
    stack_cap_bytes: u32,
    arguments: u16,
    coroutines: Vec<Option<Box<CoroutineFrame>>>,
    active_coroutines: Vec<usize>,
}

impl std::fmt::Debug for StepCheckpoint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "StepCheckpoint(pc={}, frames={})",
            self.code_pos,
            self.call_stack.len()
        )
    }
}

/// @PLN63 RX3 — the reverse-step ring depth (how many steps back are retained), from
/// `LOFT_REVERSE_DEPTH`, defaulting to 200.  A non-positive / unparseable value → the default.
fn reverse_depth_from_env() -> usize {
    const DEFAULT_REVERSE_DEPTH: usize = 200;
    std::env::var("LOFT_REVERSE_DEPTH")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_REVERSE_DEPTH)
}

/// @PLN120 A — whether a paused frame still holds a local's own value.
///
/// A local can be in lexical scope and yet have nothing readable in the frame, for
/// two different reasons — and a debugger that answers both with silence reads as
/// broken, while one that answers with the slot's contents reads as *wrong*.  So the
/// frame carries the reason.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalState {
    /// The frame holds this local's value; reading its slot is correct.
    Held,
    /// In lexical scope, but no assignment to it has completed on this path yet.
    /// `reserve_frame` does not zero locals, so its slot holds stack garbage — or,
    /// on a slot it shares, another local's live value.  It must not be read.
    Unset,
    /// In lexical scope, but its slot now holds the named local's value: the slot
    /// allocator is scope-blind, so two locals in one scope share a slot whenever
    /// their live ranges do not overlap.  Break earlier to read this one.
    Reused(String),
    /// Not in lexical scope at this line, so not part of the frame a reader of the
    /// source would expect.  [`frame_view`](State::frame_view) reports it anyway —
    /// the slot-table dump wants every local — and the captured frame drops it.
    OutOfScope,
}

impl LocalState {
    /// What to show in place of a value, or `None` for [`Held`](LocalState::Held)
    /// (whose value is read from the slot).
    #[must_use]
    pub fn marker(&self) -> Option<String> {
        match self {
            LocalState::Held => None,
            LocalState::Unset => Some("<unset>".to_string()),
            LocalState::Reused(by) => Some(format!("<reused by {by}>")),
            LocalState::OutOfScope => Some("<out of scope>".to_string()),
        }
    }

    /// Whether the frame holds this local's own value — the gate on every read or
    /// write of its slot.
    #[must_use]
    pub fn is_held(&self) -> bool {
        matches!(self, LocalState::Held)
    }
}

/// @PLN120 A — one local of a paused frame, as [`State::frame_view`] reports it.
pub(crate) struct FrameEntry {
    pub var_nr: u16,
    pub name: String,
    /// Frame-relative slot offset (`Variables::stack`).
    pub slot: u16,
    pub tp: Type,
    pub is_argument: bool,
    pub state: LocalState,
    /// First / last bytecode position that *references* this local, or `u32::MAX`.
    /// The pre-@PLN120 liveness signal: still the fallback where the scope fact is
    /// unavailable, and still reported because compiler work reads it.
    pub bc_first: u32,
    pub bc_last: u32,
}

/// Runtime state of a single coroutine instance (CO1.1).
/// Holds the serialised stack and metadata needed to suspend and resume.
#[derive(Clone, Debug)]
pub struct CoroutineFrame {
    /// Generator function definition number.
    pub d_nr: u32,
    /// Current lifecycle state.
    pub status: CoroutineStatus,
    /// Bytecode position to resume from (set by yield).
    pub code_pos: u32,
    /// Absolute stack position during execution.
    pub stack_base: u32,
    /// Return address in the consumer.
    pub caller_return_pos: u32,
    /// Serialised stack locals (copied on suspend, restored on resume).
    pub stack_bytes: Vec<u8>,
    /// Owned text slot copies (offset, content) taken on suspend.
    pub text_owned: Vec<(u32, String)>,
    /// Saved call stack entries from the generator's call frames.
    pub call_frames: Vec<CallFrame>,
    /// Call depth baseline when the coroutine was last running.
    pub call_depth: usize,
    /// S27 (debug-only): `text_positions` entries for this frame's locals, saved at
    /// yield and restored at resume.  Prevents stale entries from masking
    /// double-free or missing-free bugs in the consumer while the frame is suspended.
    #[cfg(debug_assertions)]
    pub saved_text_positions: std::collections::BTreeSet<u32>,
    /// CO1.9/S28: snapshot of `(store_nr, generation)` for all live stores at the moment
    /// of `coroutine_yield`.  Checked at `coroutine_next`; a mismatch means a store was
    /// mutated between yields and any `DbRef` locals held by the generator may be stale.
    /// Always compiled in (was debug-only before CO1.9) so the guard fires in release too.
    pub saved_store_generations: Vec<(u16, u32)>,
    /// Which occupant of this slot the frame is.
    ///
    /// A frame is freed on exhaustion (S26) and its slot handed to the next generator, so
    /// the index alone does not identify a coroutine for longer than it lives.  Every handle
    /// carries the stamp of the frame it was made for (`DbRef::pos`), and each entry point
    /// compares it before touching the slot — otherwise the scope-exit free of an exhausted
    /// handle would release whichever generator inherited its index (loft#835).
    pub generation: u32,
}

/// Internal State of the interpreter to run bytecode.
pub struct State {
    pub(crate) bytecode: Arc<Vec<u8>>,
    pub(crate) stack_cur: DbRef,
    /// @PLN18 02 — the stack store's high-water mark: the highest `stack_pos`
    /// ever reached.  `reenter` builds its synthetic frame ABOVE this, because
    /// the CURRENT `stack_pos` is only the transient eval height — a frame's
    /// variable slots live at fixed positions the eval stack dips below
    /// between statements (probe-proven: live text slots at [watermark..+48)
    /// at a yield point).
    pub stack_high: u32,
    pub stack_pos: u32,
    /// @PLAN53 cluster 2 / S4 — when true (`LOFT_ALIGN=1`), the eval-TOS
    /// @PLN154 — is the stack shadow armed?  A field rather than
    /// [`stack_verify::enabled`](crate::stack_verify::enabled) at each site: the check sits
    /// in `get_stack` and `get_var`, which run for every operand of every operator, and
    /// reading the `OnceLock` there cost 2.4 % of the interpreter's instructions on a
    /// field-write loop.  Set once, at construction.
    pub(crate) verify_on: bool,
    /// @P294: cached byte-capacity of the value-stack store (`stack_cur`).
    /// The stack store is allocated once and never re-`claim`s, so its
    /// buffer only grows through `ensure_stack`; this cache lets the hot
    /// push/reserve paths skip the store lookup when no growth is needed.
    pub(crate) stack_cap_bytes: u32,
    pub code_pos: u32,
    pub(crate) def_pos: u32,
    pub(crate) source: u16,
    // The current source during the generation of code.
    pub database: Stores,
    // Stack size of the arguments
    pub arguments: u16,
    // Local function stack positions of individual byte-code statements.
    pub stack: HashMap<u32, u16>,
    // Variables from byte code, used to also gain stack position
    pub vars: HashMap<u32, u16>,
    // Calls of function definitions from byte code.
    pub calls: HashMap<u32, Vec<u32>>,
    // Information for enumerate-types and database (record, vectors and fields) types.
    pub types: HashMap<u32, u16>,
    pub library: Arc<Vec<Call>>,
    pub library_names: HashMap<String, u16>,
    /// `#native` symbols THIS program registered a panic stub for, so
    /// `extensions::wire_native_fns` knows which ones it may replace with an
    /// auto-marshalled wrapper (hand-written glue must be left alone).
    ///
    /// Per-`State`, not a process-global: it describes the program that was just
    /// compiled. It used to live in a `static` that every `compile::byte_code`
    /// OVERWROTE, so in one process compiling several programs — a test binary, the
    /// REPL loading a second file, an embedder — a compile landing between another
    /// program's compile and its wiring replaced the set. `wire_native_fns` then hit
    /// `!stubs.contains(sym) → continue`, skipped resolution, and left the panicking
    /// stub in place, which surfaces much later as "native function not loaded".
    pub native_stub_symbols: std::collections::HashSet<String>,
    pub(crate) text_positions: BTreeSet<u32>,
    pub(crate) line_numbers: BTreeMap<u32, u32>,
    /// @PLN120 A — **scope spans**: `(start_pc, end_pc, scope_nr)` for every
    /// `Block` / `Loop` codegen walks, in emission order.  A child block's code is
    /// emitted *inside* its parent's, so **containment is the nesting relation** —
    /// the scopes open at `pc` are every span covering it, and no scope tree is
    /// needed.  Joined to [`Variables::scope`](crate::variables::Function::scope)
    /// this is which locals are in lexical scope at a pause; see
    /// [`frame_view`](State::frame_view).
    pub(crate) scope_spans: Vec<(u32, u32, u16)>,
    /// @PLN120 A — **store spans**: `(start_pc, end_pc, var_nr)` for every
    /// assignment codegen emits.  `end_pc` is the pc *after* the store, so
    /// `end_pc <= pause_pc` means the write has completed — the fact that decides
    /// whether a local's slot may be read at all.  A local with no completed store
    /// at the pause renders `<unset>`; `reserve_frame` does not zero locals, so
    /// reading such a slot is a garbage-pointer hazard, not a cosmetic one.
    ///
    /// This cannot come from `vars` (the `code_pos → var_nr` map): that is keyed by
    /// pc alone and a *read* at the same pc as an assignment's start overwrites the
    /// assignment's entry, which is why it is read-dominated.
    pub(crate) store_spans: Vec<(u32, u32, u16)>,
    /// Plan-07 phase 1 step 1.20 / phase 3 — pc → (source position, end pc) table
    /// populated by codegen on every `Value::Span` it walks.  Runtime
    /// fault printers (div-by-zero, OOB, null deref, panic call) look up
    /// the offending pc here to print `at file:line:col` alongside the
    /// existing op-name + bytecode-pos context.  Sparse — only fault-prone
    /// IR constructs (the wrapped ones in steps 1.B.1, 1.11, 1.12, 1.13)
    /// produce entries.
    ///
    /// The end pc is what makes the table answerable.  A start alone cannot
    /// distinguish "this pc is inside the construct that owns the nearest
    /// preceding span" from "this pc is past that construct entirely", and the
    /// two want opposite answers: the first is the position to report, the
    /// second is an unrelated earlier statement (loft#1262).  Entries nest but
    /// never partially overlap, since they come from the IR tree.
    pub(crate) source_spans: BTreeMap<u32, (Position, u32)>,
    /// The shared snapshot of `source_spans` handed to the crash hook, built
    /// once and then handed out as a refcount bump.
    ///
    /// Publishing used to deep-clone the whole map per entry, which is invisible
    /// for a program that is entered once and ruinous for one entered in a loop:
    /// a `loft::host` call — and so every call to a process-placed library, which
    /// travels the same path — cost **4.7 µs, of which 4.4 µs was this clone**.
    /// Cleared by the one place that writes `source_spans`, so a stale snapshot
    /// cannot be published.
    published_spans: Option<Arc<BTreeMap<u32, (Position, u32)>>>,
    /// Function coverage — `Some(bitmap)` records which definitions this run actually
    /// entered, indexed by `d_nr`.  `None` on a normal run, so the hook in
    /// [`State::fn_call`] costs one branch and nothing else.  The test runner turns it
    /// on to report the functions a suite never reached: a test suite that never enters
    /// a function has not checked it, and until this existed that silence was
    /// indistinguishable from coverage — the same shape as the backend-scope note.
    pub entered_fns: Option<Vec<bool>>,
    pub(crate) fn_positions: Vec<u32>,
    /// @PLN16 debugger — present only while debugging; the execute loop pauses at
    /// a registered breakpoint offset and captures the frame.  `None` on normal
    /// runs (the only per-op cost is one `is_some` branch).
    pub(crate) debug: Option<Box<crate::debugger::Debugger>>,
    /// Shadow call-frame vector (TR1.1).  One entry per active loft function call.
    pub call_stack: Vec<CallFrame>,
    /// Return buffers [`State::fn_call_ref`] allocated for a callee's hidden attributes,
    /// paired with the call-stack DEPTH of the frame they were pushed for — the store the
    /// caller must free when the callee hands back something else.
    ///
    /// A direct call site mints its return buffer as a caller LOCAL and frees it at scope
    /// exit.  A fn-ref call site cannot: it does not know which function the slot holds, so
    /// the buffer is allocated HERE, and a callee that delivers its return some other way
    /// (it minted its own store, or the delivery slot got rebound to a borrow) leaves it
    /// owned by nobody — one store per call, unbounded in a loop
    /// (`formal/closures.md` D-clo-7).  `--native` never had it because its dispatch passes
    /// the null sentinel for a Reference return and frees an unfilled `__vc_hbuf` for a
    /// vector one.
    ///
    /// Empty on every run that makes no fn-ref call to a buffer-taking callee, which is the
    /// overwhelming majority — the cost when unused is one `is_empty` branch per return.
    fnref_bufs: Vec<(u32, DbRef)>,
    /// The allocation counter as each live fn-ref call found it, by callee frame depth.
    ///
    /// Paired with `Store::alloc_serial`, this is what separates a store the callee MINTED
    /// from one that was already there — the question no static dep list can answer, and the
    /// one loft#1185 turns on: a capture handed back belongs to a frame further up and must be
    /// left alone, while a mint handed back belongs to nobody until this says so.
    fnref_calls: Vec<(u32, u64)>,
    /// Raw pointer to the `Data` the running program was compiled from, or null before
    /// any program has run.
    ///
    /// The frame walkers and the native-call paths need the definition table while the
    /// interpreter is inside a call, where a `&Data` cannot be threaded through without
    /// borrowing `self` for the whole run.
    ///
    /// **Set once, never cleared.** [`Self::execute_argv`] stores its `&Data` here and
    /// nothing writes null again, so the pointer stays set after that call returns. The
    /// `is_null()` guard every read carries therefore covers exactly two cases — no
    /// program has run yet, and a parallel worker that was never given one. It says
    /// nothing about whether the `Data` is still there.
    ///
    /// **Soundness is the caller's: a `State` must not outlive the `Data` it was run
    /// against.** A caller holding both as locals gets that from drop order, by declaring
    /// the `Data` FIRST so the `State` drops before it — `test_runner` and `repl` both do,
    /// inside a per-test loop body, so each iteration gets a fresh pair. Keeping a `State`
    /// somewhere its `Data` does not reach breaks every `unsafe` deref below at once, and
    /// no check on this side can detect it.
    pub(crate) data_ptr: *const crate::data::Data,
    /// Fix #87: cached library index for `n_stack_trace`.  `u16::MAX` = not yet resolved.
    pub(crate) stack_trace_lib_nr: u16,
    /// Coroutine frame storage (CO1.1).  Index 0 is always `None` (null sentinel).
    pub coroutines: Vec<Option<Box<CoroutineFrame>>>,
    /// Stamp handed to the next coroutine frame, so a recycled slot is distinguishable
    /// from the frame that held it before — see [`CoroutineFrame::generation`].
    pub(crate) coroutine_generation: u32,
    /// Indices of currently-running coroutines in `coroutines`.
    pub active_coroutines: Vec<usize>,
    /// Recursion depth counter for `generate`; reset to 0 when code generation starts.
    pub(crate) generate_depth: usize,
    /// @PLN114 — set while generating a call ARGUMENT, so the tuple-placement check
    /// in `ValueType::Tuple` knows the block it is building will be consumed
    /// directly as the callee's frame.  A tuple bound to a local instead flows
    /// through `emit_tuple_put_ops`, which relocates every element into the
    /// variable's slots, so its intermediate eval-stack placement is free and must
    /// NOT be checked.  Diagnostic only — absent from release builds.
    #[cfg(debug_assertions)]
    pub(crate) in_call_arg: bool,
    /// Number of arms in the current `parallel {}` block.
    pub(crate) parallel_n_arms: u8,
    /// Bytecode offsets for each arm (relative to join point).
    pub(crate) parallel_arm_positions: Vec<u16>,
    /// DbRef for each pre-built vector constant, indexed by definition number.
    /// Zeroed entries for non-constant definitions. Populated during `byte_code()`.
    pub const_refs: Vec<DbRef>,
    /// #629 follow-up — set when the CALLER claims the entry fn's hidden return
    /// buffer and will read it after [`execute_argv`](State::execute_argv)
    /// returns.  Default `false`, so the buffer is freed with the entry frame the
    /// way an ordinary call site frees the one it allocated; the REPL's capture
    /// wrapper is the exception (see [`keep_entry_return`](State::keep_entry_return)).
    pub(crate) keep_entry_return: bool,
}

pub(crate) fn new_ref(data: &DbRef, pos: u32, arg: u16) -> DbRef {
    DbRef {
        store_nr: data.store_nr,
        rec: pos,
        pos: u32::from(arg),
    }
}

/// Ensure `s` — a number already formatted with `to_string` — re-parses as a loft
/// `float`/`single`, **not** `integer`: append `.0` when it carries no decimal
/// point, exponent, or non-digit marker (`inf`/`NaN` pass through unchanged).  The
/// single home for the float round-trip rule — the breakpoint frame renderer
/// ([`State::render_frame_local`]) and the REPL value snapshot (`repl::float_literal`)
/// both go through it, so a `2.0` never renders as a bare `2` that re-types as
/// `integer` when seeded back.
pub(crate) fn loft_float_literal(s: &str) -> String {
    let has_dot_or_exp = s.bytes().any(|b| b == b'.' || b == b'e' || b == b'E');
    let has_digit = s.bytes().any(|b| b.is_ascii_digit());
    if has_dot_or_exp || !has_digit {
        s.to_string()
    } else {
        format!("{s}.0")
    }
}

/// @PLN98 P1b — render a keyed-collection [`Type`](crate::data::Type) as its
/// PARSEABLE loft-source form (`hash<Ent[k]>`, `sorted<Row[a, -b]>`,
/// `index<Rec[nr, -key]>`), so the live-frame eval fn can declare a paused
/// keyed local as a typed argument.  `Type::name` can't be used here: it
/// Debug-renders the key spec (`["k"]`), which the parser rejects.  `None` for a
/// non-keyed type (those never need this path).  Descending keys render with a
/// leading `-` (the `bool` is `true` for ascending — `parse_fields`' `!desc`).
fn keyed_type_source(tp: &crate::data::Type, data: &crate::data::Data) -> Option<String> {
    use crate::data::Type;
    let render_dir = |keys: &[(String, bool)]| {
        keys.iter()
            .map(|(n, asc)| if *asc { n.clone() } else { format!("-{n}") })
            .collect::<Vec<_>>()
            .join(", ")
    };
    match tp {
        Type::Hash(elem, keys, _) => Some(format!(
            "hash<{}[{}]>",
            data.def(*elem).name,
            keys.join(", ")
        )),
        Type::Sorted(elem, keys, _) => Some(format!(
            "sorted<{}[{}]>",
            data.def(*elem).name,
            render_dir(keys)
        )),
        Type::Index(elem, keys, _) => Some(format!(
            "index<{}[{}]>",
            data.def(*elem).name,
            render_dir(keys)
        )),
        _ => None,
    }
}

/// @PLN98 P3.4 — a frame local's value read out of the paused frame, tagged by its
/// storage width so [`State::eval_frame_reenter`] can push it back as the right
/// argument type (heap → `DbRef`, else the inline scalar).
enum FrameArg {
    Ref(crate::keys::DbRef),
    I64(i64),
    F64(f64),
    F32(f32),
    U8(u8),
    U32(u32),
}

/// Render `raw` as a quoted, escaped loft `text` literal — the form the parser
/// re-reads.  Escapes the characters loft shares with JSON (`"`, `\`, newline,
/// CR, tab); other characters pass through.  The single home for the text
/// round-trip rule: the breakpoint frame renderer
/// ([`State::render_frame_local`]) and the REPL value snapshot
/// (`repl::escape_loft_text`) both go through it, so a captured text reads back
/// exactly as [`unescape_loft_text`] parses it on a live edit.
pub(crate) fn loft_text_literal(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 2);
    out.push('"');
    for c in raw.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Parse a quoted loft `text` literal back to its bytes — the inverse of
/// [`loft_text_literal`], used by the @PLN16 debugger to write a text edited at
/// a breakpoint (`msg = "bye"`) back into the live frame.  Returns `None` when
/// `lit` is not a `"…"`-quoted literal.  Unknown escapes keep the following
/// character verbatim (lenient, matching the lexer's tolerance).
pub(crate) fn unescape_loft_text(lit: &str) -> Option<String> {
    let inner = lit.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('"') => out.push('"'),
            Some('\\') => out.push('\\'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    Some(out)
}

/// Bytecode encoding: ops 0–254 are one byte.  Byte 255 is an escape
/// prefix — the interpreter reads a second byte `ext` and dispatches
/// `OPERATORS[255 + ext]`.  The OPERATORS table is flat; this function
/// handles the encoding transparently for the codegen.
pub fn emit_op(op_code: u16, state: &mut State) {
    if op_code < 255 {
        state.code_add(op_code as u8);
    } else {
        state.code_add(255u8);
        state.code_add((op_code - 255) as u8);
    }
}

/// How a parallel worker's first (element) argument is delivered.  Text-returning
/// par dispatch (`run_parallel_text` → `execute_at_text`) uses this to push slot 0
/// the way the worker's parameter expects — mirroring the input ladder the
/// integer path (`run_parallel_queue`) already applies.  Without it, a primitive
/// or text element was always pushed as a 12-byte `DbRef`, feeding the worker
/// garbage (or, for text input, a wild pointer → SIGSEGV).
/// @PLN133 S8 — one argument to a re-entrant loft call
/// ([`State::run_until_return`]).
///
/// Spelled out per kind because the stack layout is: an integer is 8 bytes, a
/// `DbRef` is 12, and a text is a pointer-sized `Str` that BORROWS its bytes.
/// The borrow is why this carries a `&str` rather than a `String` — the caller
/// keeps the backing bytes alive for the length of the call, which is the same
/// contract `execute_at_raw_text_input` has for a par worker's text argument.
#[derive(Clone, Copy)]
pub enum LoftArg<'a> {
    Int(i64),
    Ref(DbRef),
    Text(&'a str),
}

#[derive(Clone, Copy)]
pub enum WorkerArg {
    /// Struct / reference element — a 12-byte `DbRef` into the element record.
    Ref(DbRef),
    /// Primitive element pushed at its native width (`size` = 1 / 4 / 8).
    Primitive { value: u64, size: u32 },
    /// Text element — a 16-byte `Str`.
    Text(crate::keys::Str),
    /// Wide INLINE element — a tuple, or any other 9..=64 byte value the worker
    /// reads as one contiguous slot rather than through a pointer.  `size` is the
    /// worker's argument-slot width; only the first `size` bytes of `buf` are live.
    ///
    /// Without this spelling a wide row had no answer but [`WorkerArg::Ref`], so a
    /// worker taking `(integer, integer)` was handed the row's `DbRef` and read the
    /// pointer's bits as its tuple — loft#1055.
    Wide { buf: [u8; 64], size: u32 },
}

/// What a host call expects the target function to return — selects how the
/// return value is read off the stack after the call.  Used by `execute_host`
/// (the `loft::host` Rust→loft entry).
#[derive(Clone, Copy)]
pub enum HostRetKind {
    /// No return value read.
    Void,
    /// A 1/4/8-byte primitive (integer family, boolean, single) read at `size`.
    Prim(u32),
    /// A 16-byte `Str`, materialised into an owned `String`.
    Text,
    /// @PLN119 arc B — a struct / vector: the 12-byte `DbRef` naming the record
    /// the answer was built in.  Which store that is, is the CALLEE's answer, not
    /// the caller's guess: it may be the hidden destination the caller offered,
    /// or a store the callee minted when it ignored one.
    Ref,
}

/// The value a host call read back from a loft function.
pub enum HostReturn {
    Void,
    /// A primitive zero-extended into a `u64`; the host re-narrows by the return type.
    Prim(u64),
    Text(String),
    /// A struct / vector, named by the record holding it.  Nothing is copied —
    /// the record lives wherever the callee built it, and it is the caller's job
    /// to read it before that store is reset or freed.
    Ref(DbRef),
}

impl State {
    /**
    Create a new interpreter state
    # Panics
    When the statically defined alignment is not correct.
    */
    #[must_use]
    pub fn new(mut db: Stores) -> State {
        let stack_cur = db.database(1000);
        db.stack_store_at_zero = true; // #306 — protect slot 0 from whole-store frees
        // @PLN154 — arm the stack shadow.  Here and only here: the shadow is what makes a
        // store the one whose slots are checked, so a heap store never carries one.
        if crate::stack_verify::enabled() {
            db.store_mut(&stack_cur).arm_init_shadow();
        }
        let stack_cap_bytes = db.store(&stack_cur).byte_capacity() as u32;
        // Allocate the constant store (CONST_STORE = 1). Starts empty,
        // populated during byte_code(), locked before execution.
        let _const_store = db.database(100);
        debug_assert_eq!(
            _const_store.store_nr,
            crate::database::CONST_STORE,
            "Constant store must be at index {}",
            crate::database::CONST_STORE
        );
        State {
            bytecode: Arc::new(Vec::new()),
            stack_cur,
            stack_pos: 4,
            stack_high: 4,
            stack_cap_bytes,
            verify_on: crate::stack_verify::enabled(),
            code_pos: 0,
            def_pos: 0,
            source: u16::MAX,
            database: db,
            arguments: 0,
            stack: HashMap::new(),
            vars: HashMap::new(),
            calls: HashMap::new(),
            types: HashMap::new(),
            library: Arc::new(Vec::new()),
            library_names: HashMap::new(),
            native_stub_symbols: std::collections::HashSet::new(),
            text_positions: BTreeSet::new(),
            line_numbers: BTreeMap::new(),
            scope_spans: Vec::new(),
            store_spans: Vec::new(),
            source_spans: BTreeMap::new(),
            published_spans: None,
            entered_fns: None,
            fn_positions: Vec::new(),
            debug: None,
            call_stack: Vec::new(),
            fnref_bufs: Vec::new(),
            fnref_calls: Vec::new(),
            data_ptr: std::ptr::null(),
            stack_trace_lib_nr: u16::MAX,
            coroutines: vec![None], // index 0 = null sentinel
            coroutine_generation: 1,
            active_coroutines: Vec::new(),
            generate_depth: 0,
            #[cfg(debug_assertions)]
            in_call_arg: false,
            parallel_n_arms: 0,
            parallel_arm_positions: Vec::new(),
            const_refs: Vec::new(),
            keep_entry_return: false,
        }
    }

    /// Most frames a loft call stack may hold — `main` included, since `main` is a
    /// frame like any other and `stack_trace()` reports it as one.
    ///
    /// Set below the store stack limit (~8000 bytes / ~8 bytes per frame) so the depth
    /// check fires before a store out-of-bounds panic.  Both backends enforce it against
    /// the same quantity: `fn_call` reads `call_stack.len()`, the generated binary reads
    /// its shadow stack in `cr_call_push` (loft#1058).
    pub const MAX_CALL_DEPTH: u32 = 10_000;

    pub fn static_fn(&mut self, name: &str, call: Call) {
        let lib = Arc::make_mut(&mut self.library);
        let nr = lib.len() as u16;
        self.library_names.insert(name.to_string(), nr);
        lib.push(call);
    }

    /// Replace the implementation of an already-registered native function.
    /// Used by the WASM GL bridge to replace panic stubs with real implementations.
    /// No-op if `name` is not registered.
    pub fn replace_native(&mut self, name: &str, call: Call) {
        if let Some(&nr) = self.library_names.get(name) {
            let lib = Arc::make_mut(&mut self.library);
            lib[nr as usize] = call;
        }
    }

    /// Register a native Rust function under `symbol` for use by `#native "symbol"` loft
    /// functions.  Alias for `static_fn` with an external-extension naming convention.
    pub fn register_native(&mut self, symbol: &str, call: Call) {
        self.static_fn(symbol, call);
    }

    /// PKG.1: Replace a previously registered function (e.g. a stub) with a
    /// real implementation.  Used by `extensions::load_all()` to swap stubs
    /// created during `byte_code()` with actual native library functions.
    /// Returns `true` if the function was found and replaced.
    pub fn replace_static_fn(&mut self, name: &str, call: Call) -> bool {
        if let Some(&nr) = self.library_names.get(name) {
            let lib = Arc::make_mut(&mut self.library);
            lib[nr as usize] = call;
            true
        } else {
            false
        }
    }

    /// Call a function, remember the current code position on the stack.
    ///
    /// * `d_nr` - definition number of the called function.
    /// * `args_size` - total byte size of all parameters.
    /// * `to` - the code position where the called function resides.
    ///
    /// # Panics
    /// When call depth exceeds `MAX_CALL_DEPTH` (possible infinite recursion).
    pub fn fn_call(&mut self, d_nr: u32, args_size: u16, to: i64) {
        let args_base = self.stack_pos - u32::from(args_size);
        // Find the nearest source line at or before the current code position.
        // line_numbers entries are emitted before the first instruction on each line,
        // so after consuming a Call instruction code_pos is past the entry — use
        // range(..=code_pos).next_back() to recover the most recent line.
        let line = self
            .line_numbers
            .range(..=self.code_pos)
            .next_back()
            .map_or(0, |(_, &v)| v);
        // Plan-07 phase 4f.12 — stack overflow becomes a typed
        // RuntimeError instead of an opaque Rust panic.  Detect at
        // call entry, raise StackOverflow.  Production logs +
        // continues per C66 (host frame loop decides whether to
        // restart); dev mode halts + renders.
        //
        // loft#1058 — the guard counts the FRAMES ON THE STACK, which is what
        // `--native`'s `cr_call_push` tests and what `stack_trace()` reports on both
        // backends.  It used to test a separate `call_depth` counter that did not
        // count `main` and was left untouched when a coroutine truncated the stack, so
        // one cap meant two different things: `rec(9999)` answered on `--interpret` and
        // overflowed on `--native`.  Reading the stack removes the drift rather than
        // correcting for it — there is no second counter left to keep in step.
        if self.call_stack.len() >= Self::MAX_CALL_DEPTH as usize {
            // loft#1058 — report against the declaration of the function that is
            // RUNNING, not the call op and not the callee: it is the one thing both
            // backends can name here.  `--native` detects the same overflow inside
            // `cr_call_push`, at a callee entry, where the caller's current line is not
            // in reach — and the two backends describe the same full stack from
            // opposite ends ("about to make call N+1" here, "just entered frame N+1"
            // there), so the innermost FRAME is common to both while the callee is not.
            // It is also what the frame block below names first, which the call-op
            // position did not: a mutual recursion said `--> alpha` over
            // `in fn beta()`.
            let position = self.running_frame_declaration();
            self.raise_at(
                crate::runtime_error::RuntimeErrorKind::StackOverflow,
                position,
            );
            return;
        }
        // Coverage: record the entry AFTER the depth check, so a call that overflows
        // the stack — and therefore never runs the body — is not counted as reached.
        if let Some(seen) = &mut self.entered_fns
            && let Some(slot) = seen.get_mut(d_nr as usize)
        {
            *slot = true;
        }
        // loft#952 — the timeout breadcrumb, in the same place and for the same reason
        // native's `cr_call_push` carries it: this is the last loft function the run
        // entered, so it is what a hang should be reported against.  Two relaxed stores
        // when armed, one load and a branch when not.
        crate::timeout::checkpoint_interp_call(d_nr);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: self.code_pos,
            args_base,
            args_size,
            line,
        });
        self.put_stack(self.code_pos);
        self.code_pos = to as u32;
    }

    /// Call a function through a runtime function reference.
    ///
    /// Reads the definition number stored in the fn-ref variable at `fn_var` bytes below the
    /// current stack top, looks up its bytecode position, then delegates to `fn_call`.
    ///
    /// # Panics
    ///
    /// Panics if `fn_var < 16` (the fn-ref slot is 16 bytes: `d_nr` + closure `DbRef`), or if
    /// the slot holds a negative definition number (un-initialised / null sentinel).
    pub fn fn_call_ref(&mut self, fn_var: u16, arg_size: u16) {
        // fn-ref slot is 20B ([d_nr:i64][closure:DbRef]); fn_var must be ≥ 20.
        assert!(
            fn_var >= 20,
            "fn_call_ref: fn_var={fn_var} < 20 — fn-ref slot is 20B (d_nr i64 + closure DbRef)"
        );
        let d_nr_i64 = *self.get_var::<i64>(fn_var);
        // Negative d_nr = un-initialised slot (integer null sentinel = i64::MIN).
        assert!(
            d_nr_i64 >= 0,
            "fn_call_ref: d_nr={d_nr_i64} is negative — fn-ref slot was never assigned"
        );
        let d_nr = d_nr_i64 as usize;
        assert!(
            d_nr < self.fn_positions.len(),
            "fn_call_ref: d_nr={d_nr} out of range (fn_positions.len={})",
            self.fn_positions.len()
        );
        // @P387 zero-cost: a text-returning fn-ref call site injects `&text` work
        // buffers without knowing which function the slot holds, so it pushes what the
        // widest candidate of that signature wants (`Data::fnref_text_buffers`).  Here
        // the target IS known, so the frame is trimmed to what it actually declares: a
        // callee that builds into fewer buffers — a literal/forward body, one that
        // returns a parameter directly, or simply a narrower candidate — would otherwise
        // mis-read a spurious DbRef as its closure and crash.
        //
        // The excess comes off the TOP, which keeps the callee's own buffers in place:
        // the site pushes visible args first and then buffers in attribute order, so the
        // ones a narrower callee wants are the ones pushed FIRST.  Interpreter-only —
        // native delivers text owned and never threads these buffers (loft#1116).
        let (fn_var, arg_size) = {
            let mut fv = fn_var;
            let mut asz = arg_size;
            // A work-buffer occupies one STEPPED DbRef span (16B under 8-byte
            // alignment, not the raw 12) — pop exactly that per buffer.
            let buf_span = self.stack_step(size_ref()) as u16;
            if !self.data_ptr.is_null() && buf_span > 0 {
                // SAFETY: the `Data` outlives this `State` — see `data_ptr`.
                let data = unsafe { &*self.data_ptr };
                let def = data.def(d_nr as u32);
                let visible = def
                    .attributes()
                    .iter()
                    .filter(|a| {
                        !a.hidden
                            && a.name != "__closure"
                            && !matches!(&a.typedef,
                                crate::data::Type::RefVar(t) if matches!(**t, crate::data::Type::Text(_)))
                    })
                    .count();
                let pushed = data.fnref_text_buffers(visible, def.returned());
                let wanted = def.text_work_buffers();
                let extra = u16::try_from(pushed.saturating_sub(wanted)).unwrap_or(0) * buf_span;
                if extra > 0 && asz >= extra {
                    self.stack_pos -= u32::from(extra);
                    fv -= extra;
                    asz -= extra;
                }
            }
            (fv, asz)
        };
        // PLAN51 V-c interp — when the callee was promoted by `ref_return`
        // (parser/control.rs:3175) it has hidden Reference / Vector /
        // struct-Enum attribute(s) appended to its signature, but the
        // bytecode call site (codegen.rs:2489 generate_call_ref) emits
        // only the user-visible args (the parser's CallRef IR omits
        // hidden bufs because injecting them at IR-level conflicts with
        // the native dispatch's per-candidate handling at emit.rs:670-686
        // and breaks Flat-struct lambdas whose return doesn't engage
        // ref_return).  Reconcile here by pushing one EMPTY allocated
        // DbRef per hidden attr (via `stores.null()`) onto the eval
        // stack.  The callee's body either:
        //   (a) reassigns the slot via OpDatabase (struct literal /
        //       nested call) — OpDatabase's `clear + claim` reuses our
        //       allocated store; no leak.
        //   (b) uses the slot directly (vector body's
        //       `pre_alloc_vector`) — needs a real store_nr; we
        //       provided one.
        // We use `null()` (size=u32::MAX dynamic marker) so both shapes
        // work uniformly.  Sentinel (`store_nr=u16::MAX`) is NOT safe
        // here: bytecode VM's OpDatabase at src/state/io.rs:708 calls
        // `clear(db)` unconditionally → OOB on u16::MAX (allocation.rs:421).
        let mut hidden_bufs_size: u16 = 0;
        let mut allocated_bufs: Vec<DbRef> = Vec::new();
        if !self.data_ptr.is_null() {
            // SAFETY: the `Data` outlives this `State` — see `data_ptr`.
            let data = unsafe { &*self.data_ptr };
            let attr_count = data.def(d_nr as u32).attributes().len();
            for a_idx in 0..attr_count {
                let attr = &data.def(d_nr as u32).attributes()[a_idx];
                if !attr.hidden {
                    continue;
                }
                let buf = match &attr.typedef {
                    crate::data::Type::Reference(_, _) | crate::data::Type::Enum(_, true, _) => {
                        // For struct returns, the body's `cv = Type{...}`
                        // OpDatabase reuses the slot (clear + claim).
                        // `null()` provides a real slot with rec=0 — the
                        // claim succeeds and sets rec=1.
                        self.database.null()
                    }
                    crate::data::Type::Vector(elm_tp, _) => {
                        // Vector body's `v = []` does NOT OpDatabase var_v;
                        // it expects rec != 0 (else pre_alloc_vector is a
                        // no-op, leaving the vector empty — probe 59 INT
                        // would read seq[0] as null).  Allocate a fresh
                        // store with a properly-claimed vector record.
                        let elm_name = elm_tp.name(data);
                        let tp_name = format!("main_vector<{elm_name}>");
                        let tp_id = self.database.name(&tp_name);
                        if tp_id == u16::MAX {
                            self.database.null()
                        } else {
                            let sz = u32::from(self.database.size(tp_id));
                            let r = self.database.database(sz);
                            self.database.allocations[r.store_nr as usize].set_known_type(tp_id);
                            self.database
                                .store_mut(&r)
                                .set_u32_raw(r.rec, 4, u32::from(tp_id));
                            self.database.set_default_value(tp_id, &r);
                            r
                        }
                    }
                    _ => continue,
                };
                // loft#717 — accumulate the stack movement the push ACTUALLY made
                // rather than the 12 bytes a `DbRef` measures.  `put_stack` steps a
                // slot to its alignment, so a `DbRef` occupies 16, and the closure
                // read below — which re-expresses `fn_var` against the shifted TOS —
                // landed 4 bytes short for every buffer pushed here.
                //
                // Neither half shows it alone: with no closure there is nothing to
                // misread, and with no hidden buffer the shift is zero.  It takes a
                // capturing closure that ALSO returns a struct, and then the callee
                // receives a garbage closure `DbRef` and faults.
                // loft#1179 / `formal/closures.md` D-clo-7 — remember the store, because
                // this call site is the only owner it will ever have.  A DIRECT call mints
                // its return buffer as a caller local that scope exit frees; here the
                // buffer is minted per call and handed to a callee that may deliver its
                // return some other way (it minted its own store, or the delivery slot was
                // rebound to a borrow), and then nothing frees it.  [`Self::fn_return`]
                // does, against the value that actually came back.
                allocated_bufs.push(buf);
                let before = self.stack_pos;
                self.put_stack(buf);
                hidden_bufs_size += u16::try_from(self.stack_pos - before).unwrap_or(0);
            }
        }
        // Read closure DbRef from bytes 8..20 of the fn-ref slot.
        // fn_var is distance from fn_ref slot START to TOS; slot+8 = TOS-(fn_var-8).
        // NOTE: read closure BEFORE pushing hidden bufs above would have
        // worked too, but we read it AFTER so the hidden-buf inserts
        // don't shift our reference points; fn_var is computed against
        // the PRE-call TOS, so reading it after any put_stack would use
        // the wrong offset.  The hidden-buf pushes above DO shift TOS,
        // so we must read closure AFTER them but compute its offset
        // against the shifted TOS.  Adjust fn_var by hidden_bufs_size.
        let closure = *self.get_var::<DbRef>(fn_var + hidden_bufs_size - 8);
        let has_closure = closure.rec != 0;
        // Measured, not assumed, for the same reason as the hidden buffers above:
        // the callee's frame must be told the span these pushes really occupy.
        let before_closure = self.stack_pos;
        if has_closure {
            self.put_stack(closure);
        }
        let closure_span = u16::try_from(self.stack_pos - before_closure).unwrap_or(0);
        let total = arg_size + hidden_bufs_size + closure_span;
        let code_pos = i64::from(self.fn_positions[d_nr]);
        // The depth the callee's frame is about to occupy, recorded BEFORE `fn_call` pushes
        // it so the two agree by construction rather than by an off-by-one.
        let frame_depth = u32::try_from(self.call_stack.len()).unwrap_or(u32::MAX);
        for buf in allocated_bufs {
            self.fnref_bufs.push((frame_depth, buf));
        }
        // loft#1185 — the allocation counter as it stands BEFORE the callee runs.  A store the
        // callee hands back whose stamp is above this was minted inside the call and has no
        // owner anywhere; one at or below it was already there (a capture, an argument) and is
        // somebody else's.  `release_fnref_bufs` is where that comparison is made.
        self.fnref_calls
            .push((frame_depth, self.database.stores_allocated));
        self.fn_call(d_nr as u32, total, code_pos);
    }

    pub fn static_call(&mut self) {
        let call = self.code::<u16>();
        // Fix #87: resolve n_stack_trace index lazily, then only snapshot for that call.
        if self.stack_trace_lib_nr == u16::MAX
            && let Some(&nr) = self.library_names.get("n_stack_trace")
        {
            self.stack_trace_lib_nr = nr;
        }
        // TR1.3: snapshot call_stack only when n_stack_trace is being called.
        // Fix #92: also works in parallel workers where data_ptr may be null;
        // frames with d_nr == u32::MAX (synthetic worker frame) get a placeholder name.
        if call == self.stack_trace_lib_nr && !self.call_stack.is_empty() {
            // SAFETY: the `Data` outlives this `State` — see `data_ptr`.
            let data_opt: Option<&Data> = if self.data_ptr.is_null() {
                None
            } else {
                Some(unsafe { &*self.data_ptr })
            };
            self.database.call_stack_snapshot = self
                .call_stack
                .iter()
                .enumerate()
                .map(|(idx, f)| {
                    if let Some(data) = data_opt
                        && f.d_nr != u32::MAX
                        && (f.d_nr as usize) < data.definitions.len()
                    {
                        let def = &data.definitions[f.d_nr as usize];
                        let name = if def.name().starts_with("n_") {
                            def.name()[2..].to_string()
                        } else {
                            def.name().to_owned()
                        };
                        let file = def.position().file.clone();
                        // Fix #92: line resolution for parallel-worker frames.
                        // The CallFrame.line is only updated by `fn_call`, which
                        // never runs for the worker's entry frame.  Fall back to
                        // looking up the current bytecode position in
                        // `line_numbers` so workers report the actual source
                        // line they're executing rather than 0.
                        let line = if f.line != 0 {
                            f.line
                        } else {
                            let cp = if idx + 1 == self.call_stack.len() {
                                self.code_pos
                            } else {
                                self.call_stack[idx + 1].call_pos
                            };
                            self.line_numbers.get(&cp).copied().unwrap_or(0)
                        };
                        (name, file, line)
                    } else {
                        // Worker frame without Data context — use placeholder.
                        ("<worker>".to_string(), String::new(), f.line)
                    }
                })
                .collect();
            // TR1.4: snapshot variables for each frame.  Each frame's bytecode
            // position is its `call_pos` (where the next call was made), or
            // for the topmost frame, the current `code_pos`.
            if let Some(data) = data_opt {
                let frames: Vec<(u32, u32, u32)> = self
                    .call_stack
                    .iter()
                    .enumerate()
                    .map(|(idx, f)| {
                        let cp = if idx + 1 == self.call_stack.len() {
                            // Topmost frame: use current code_pos
                            self.code_pos
                        } else {
                            // Non-top frame: use the call instruction position
                            // of the next frame's call
                            self.call_stack[idx + 1].call_pos
                        };
                        (f.d_nr, f.args_base, cp)
                    })
                    .collect();
                self.database.variables_snapshot = frames
                    .into_iter()
                    .map(|(d_nr, args_base, cp)| {
                        let frame_vars = self.iter_frame_variables_at(data, d_nr, args_base, cp);
                        frame_vars
                            .into_iter()
                            .filter(|fv| fv.live)
                            .map(|fv| {
                                let type_name = fv.typedef.name(data);
                                let value = match fv.value {
                                    crate::state::debug::VariableValue::Integer(n) => {
                                        crate::database::VarValueSnapshot::Int(n)
                                    }
                                    crate::state::debug::VariableValue::Long(n) => {
                                        crate::database::VarValueSnapshot::Long(n)
                                    }
                                    crate::state::debug::VariableValue::Single(n) => {
                                        crate::database::VarValueSnapshot::Single(n)
                                    }
                                    crate::state::debug::VariableValue::Float(n) => {
                                        crate::database::VarValueSnapshot::Float(n)
                                    }
                                    crate::state::debug::VariableValue::Boolean(b) => {
                                        crate::database::VarValueSnapshot::Bool(b)
                                    }
                                    crate::state::debug::VariableValue::Character(c) => {
                                        crate::database::VarValueSnapshot::Char(c)
                                    }
                                    crate::state::debug::VariableValue::Text {
                                        content, ..
                                    }
                                    | crate::state::debug::VariableValue::StrView {
                                        content, ..
                                    } => crate::database::VarValueSnapshot::Text(
                                        content.unwrap_or_default(),
                                    ),
                                    crate::state::debug::VariableValue::Reference(r)
                                    | crate::state::debug::VariableValue::Vector(r) => {
                                        crate::database::VarValueSnapshot::Ref {
                                            store: i32::from(r.store_nr),
                                            rec: r.rec as i32,
                                            pos: r.pos as i32,
                                        }
                                    }
                                    crate::state::debug::VariableValue::OutOfFrame => {
                                        crate::database::VarValueSnapshot::Other(
                                            "<out-of-frame>".to_string(),
                                        )
                                    }
                                    crate::state::debug::VariableValue::Unreadable(why) => {
                                        crate::database::VarValueSnapshot::Other(format!(
                                            "<unreadable: {why}>"
                                        ))
                                    }
                                    crate::state::debug::VariableValue::Unsupported => {
                                        crate::database::VarValueSnapshot::Other(
                                            "<unsupported>".to_string(),
                                        )
                                    }
                                };
                                crate::database::VarSnapshot {
                                    name: fv.name,
                                    type_name,
                                    value,
                                }
                            })
                            .collect()
                    })
                    .collect();
            }
        }
        // @P294: native lib fns push their results through the stack store
        // via the `stack` DbRef, bypassing `put_stack`'s growth check.
        // Reserve generous headroom so a result-pushing native fn cannot
        // run past the buffer.  Native fns push bounded results (scalars /
        // Str / DbRef / small structs), so 1 KiB is ample.
        self.ensure_stack(1024);
        let mut stack = self.stack_cur;
        stack.pos = 8 + self.stack_pos;
        // PKG.5: set library index for auto-marshal dispatch.
        crate::extensions::set_current_lib_idx(call);
        self.library[call as usize](&mut self.database, &mut stack);
        self.stack_pos = stack.pos - 8;
    }

    /**
    Returns from a function, the data structures that went out of scope should already have
    been freed at this point.
    * `ret` - Size of the parameters to get the return address after it.
    * `value` - Size of the return value.
    * `discard` - The amount of space claimed on the stack at this point.
    # Panics
    When there are claimed texts that are not freed yet.
    */
    pub fn fn_return(&mut self, ret: u16, value: u8, discard: u16) {
        let pos = self.stack_pos;
        self.stack_pos -= u32::from(discard);
        if cfg!(debug_assertions) {
            let orphans: Vec<u32> = self
                .text_positions
                .range(self.stack_pos..=pos)
                .copied()
                .collect();
            for p in orphans {
                self.text_positions.remove(&p);
            }
        }
        let fn_stack = self.stack_pos;
        self.stack_pos += u32::from(ret);
        self.code_pos = *self.get_var::<u32>(0);
        self.copy_result(value, pos, fn_stack);
        // Both lists are consumed here, and the OR is load-bearing: a fn-ref call whose callee
        // allocated no buffer still leaves a snapshot, and skipping the release for it left that
        // snapshot to be matched against a LATER frame at the same depth — a store then read as
        // "minted during the call" that was not, handed up, and freed under a live binding.
        if !self.fnref_bufs.is_empty() || !self.fnref_calls.is_empty() {
            self.release_fnref_bufs(value);
        }
        self.call_stack.pop();
    }

    /// Hand the store this frame is RETURNING to whoever will hold it.
    ///
    /// `OpFreeRefOrHandUp`'s adoption leg: the callee is returning a store it minted itself,
    /// and its caller reads the result as a borrow — so neither of them frees it, and without
    /// an owner it leaks one store per call (`formal/closures.md` D-clo-13, loft#1186).
    ///
    /// It joins the SAME list a delivered return buffer uses, at this frame's depth, so
    /// [`Self::release_fnref_bufs`] carries it up on the way out by the rule it already
    /// applies: the store is owned by the frame that holds it, and ownership travels with the
    /// return value.  Nothing new decides when it dies.
    pub fn hand_up_returned(&mut self, returned: DbRef) {
        if returned.store_nr == u16::MAX || returned.rec == 0 {
            return;
        }
        let depth = u32::try_from(self.call_stack.len())
            .unwrap_or(u32::MAX)
            .saturating_sub(1);
        self.fnref_bufs.push((depth, returned));
    }

    /// The allocation counter as the fn-ref call into `depth` found it, consuming the entry.
    ///
    /// `None` when this frame was not entered through a fn-ref — an ordinary call's return is
    /// the caller's by its own type, and nothing here has a claim on it.
    fn fnref_call_snapshot(&mut self, depth: u32) -> Option<u64> {
        let at = self.fnref_calls.iter().rposition(|(d, _)| *d == depth)?;
        Some(self.fnref_calls.remove(at).1)
    }

    /// Free the return buffers [`Self::fn_call_ref`] allocated for the frame now returning,
    /// keeping the one the callee actually handed back.
    ///
    /// The kept one is identified by STORE, not by the whole `DbRef`: a callee that
    /// delivered through the buffer may answer a record or a position inside it, and that is
    /// still the store the caller now owns as its result. Everything else this call site
    /// allocated was never used for the delivery — the buffer variable is compiler-generated
    /// and appears only in return-delivery code, so nothing outside the callee can be
    /// holding it — and is released here.
    ///
    /// Entries left behind by a frame that unwound some other way (a coroutine truncation)
    /// are DROPPED rather than freed: the value they name is no longer this frame's to
    /// judge, and leaking is the recoverable direction.
    fn release_fnref_bufs(&mut self, value: u8) {
        let depth = u32::try_from(self.call_stack.len())
            .unwrap_or(u32::MAX)
            .saturating_sub(1);
        let returned: Option<DbRef> = (usize::from(value) == size_of::<DbRef>()).then(|| {
            let back = u16::try_from(self.stack_step(u32::from(value))).unwrap_or(0);
            *self.get_var::<DbRef>(back)
        });
        // loft#1185 — a store this frame MINTED and is handing back has no owner: the callee
        // does not free what it returns, and its caller may be a forwarding function whose
        // return type says nothing about it.  Its stamp is above the snapshot taken when the
        // call began; a CAPTURE's is not, and a capture belongs to a frame further up and is
        // not ours to place.  That comparison is the whole of what no static dep list could
        // answer, and it costs one `u64` per allocation and one lookup per fn-ref return.
        let minted_here = self.fnref_call_snapshot(depth).is_some_and(|since| {
            returned.is_some_and(|r| {
                (r.store_nr as usize) < self.database.allocations.len()
                    && self.database.allocations[r.store_nr as usize].alloc_serial > since
            })
        });
        if minted_here
            && let Some(r) = returned
            && !self
                .fnref_bufs
                .iter()
                .any(|(d, b)| *d == depth && b.store_nr == r.store_nr)
        {
            self.fnref_bufs.push((depth, r));
        }
        // The buffer the callee HANDED BACK moves up one frame rather than being forgotten.
        // The call site is the only owner it will ever have, and the caller's static type may
        // say it owns nothing — a local that borrows on one assignment and receives this store
        // on another (loft#1183), a `??` whose other arm hands back the capture (loft#1186), a
        // forwarding frame in the way (loft#1185).  None of those can be answered statically,
        // and none has to be: the store is owned by the frame that currently HOLDS it, and
        // ownership travels with the return value.  A caller that goes on to free it itself
        // leaves the handle stale, which the already-free guard below skips.
        let mut handed_up: Vec<DbRef> = Vec::new();
        while let Some(&(d, buf)) = self.fnref_bufs.last() {
            if d < depth {
                break;
            }
            self.fnref_bufs.pop();
            if d != depth {
                continue;
            }
            if returned.is_some_and(|r| r.store_nr == buf.store_nr) {
                handed_up.push(buf);
                continue;
            }
            // A store the callee already released leaves a stale handle here; freeing it
            // twice would hand a live slot back to the pool.
            if buf.store_nr == COROUTINE_STORE
                || (buf.store_nr as usize) >= self.database.allocations.len()
                || self.database.allocations[buf.store_nr as usize].free
            {
                continue;
            }
            self.free_ref_db(buf);
        }
        if depth > 0 {
            for buf in handed_up {
                self.fnref_bufs.push((depth - 1, buf));
            }
        }
    }

    // ── CO1.1 — Coroutine frame helpers ─────────────────────────────────────

    /// Allocate a coroutine frame.  Returns its index (always >= 1) and the generation
    /// stamp that, together with the index, names THIS frame rather than whatever later
    /// inherits its slot — see [`CoroutineFrame::generation`].
    pub fn allocate_coroutine(&mut self, mut frame: CoroutineFrame) -> (usize, u32) {
        let generation = self.coroutine_generation;
        self.coroutine_generation = self.coroutine_generation.wrapping_add(1).max(1);
        frame.generation = generation;
        // Reuse the first free slot (index >= 1).
        for (i, slot) in self.coroutines.iter_mut().enumerate().skip(1) {
            if slot.is_none() {
                *slot = Some(Box::new(frame));
                return (i, generation);
            }
        }
        let idx = self.coroutines.len();
        self.coroutines.push(Some(Box::new(frame)));
        (idx, generation)
    }

    /// Free a coroutine frame, making the slot available for reuse.
    ///
    /// S25.3 (C24): for `Suspended` frames, drop any text-local `String` objects
    /// embedded in `stack_bytes` before the `Vec<u8>` backing is freed.  Without
    /// this, an early `break` from a generator loop leaks every text local that was
    /// live at the last yield point.
    pub fn free_coroutine(&mut self, gen_ref: &DbRef) {
        if self.coroutine_slot_matches(gen_ref) {
            let idx = gen_ref.rec as usize;
            let mut owned_stores: Vec<DbRef> = Vec::new();
            if let Some(frame) = self.coroutines[idx].as_mut()
                && frame.status == CoroutineStatus::Suspended
            {
                let d_nr = frame.d_nr;
                let data_ptr = self.data_ptr; // raw ptr — no borrow conflict with frame
                Self::drop_text_locals_in_bytes(d_nr, &mut frame.stack_bytes, data_ptr);
                owned_stores =
                    Self::owned_store_locals_in_bytes(d_nr, &mut frame.stack_bytes, data_ptr);
            }
            self.coroutines[idx] = None;
            // After the slot is cleared, so a nested generator handle among these frees its
            // own frame without re-entering this one.
            for db in owned_stores {
                // A local the generator's own scope exit already freed leaves a stale
                // reference in the slot; skip anything whose store is no longer allocated
                // rather than free it twice.
                if db.store_nr != COROUTINE_STORE
                    && ((db.store_nr as usize) >= self.database.allocations.len()
                        || self.database.allocations[db.store_nr as usize].free)
                {
                    continue;
                }
                self.free_ref_db(db);
            }
        }
    }

    /// The store-backed locals a SUSPENDED generator still owns, read out of its
    /// serialised frame so they can be freed along with it.
    ///
    /// A generator frees its own heap locals from the tail of its body, and a generator
    /// whose consumer stopped early never reaches that tail — so
    /// `for x in steps() { … break … }` left the vector the generator was walking
    /// allocated for the rest of the program (loft#835).  Stopping early is ordinary code,
    /// not misuse: iterating until a match is found and breaking is the main reason to
    /// reach for a generator, and `GOALS.md` § Goal E says a scope's heap memory is freed
    /// with no exceptions the programmer has to learn.
    ///
    /// Only abandoned frames reach here — an exhausted one already ran its own
    /// `OpFreeRef`s — and only locals the generator OWNS are collected, so a yielded view
    /// of somebody else's store is left alone.  Each slot is zeroed once read, so a second
    /// pass over the same frame frees nothing.
    ///
    /// # Safety
    /// Must only be called for `Suspended` frames whose local region was zeroed at first
    /// resume (Step 1 of S25.3): that is what makes a never-assigned slot read back as an
    /// empty reference instead of as garbage.
    fn owned_store_locals_in_bytes(
        d_nr: u32,
        bytes: &mut [u8],
        data_ptr: *const Data,
    ) -> Vec<DbRef> {
        let mut owned = Vec::new();
        if data_ptr.is_null() {
            return owned;
        }
        // SAFETY: `data_ptr` is the caller's `State::data_ptr`, whose `Data` outlives
        // that `State` — see the field.
        let data = unsafe { &*data_ptr };
        let Some(def) = data.definitions.get(d_nr as usize) else {
            return owned;
        };
        let vars = &def.variables();
        for v in 0..vars.count() {
            if vars.is_argument(v) {
                continue;
            }
            let tp = vars.tp(v);
            // `Iterator` joins the heap types here: a generator local holding another
            // generator's handle is freed through the same path, which `free_ref_db`
            // routes back to this function for that frame.
            if !Self::is_heap_type(tp) && !matches!(tp, Type::Iterator(_, _)) {
                continue;
            }
            if !vars.owns_store(v) {
                continue;
            }
            let slot = vars.stack(v);
            if slot == u16::MAX {
                continue;
            }
            let off = slot as usize;
            if off + std::mem::size_of::<DbRef>() > bytes.len() {
                continue; // local beyond the yield snapshot — never assigned
            }
            // SAFETY: stack_bytes holds each local at its original stack offset, written
            // by the same `DbRef` layout `get_stack::<DbRef>` reads.  Unaligned on purpose.
            #[allow(clippy::cast_ptr_alignment)]
            let db: DbRef =
                unsafe { std::ptr::read_unaligned(bytes.as_ptr().add(off).cast::<DbRef>()) };
            unsafe {
                std::ptr::write_bytes(bytes.as_mut_ptr().add(off), 0, std::mem::size_of::<DbRef>());
            }
            // `rec == 0` covers both the null sentinel and a slot the zeroed local region
            // left untouched; neither names a store to free.
            if db.rec == 0 {
                continue;
            }
            owned.push(db);
        }
        owned
    }

    /// S25.3 (C24): compute the size of the local-variable region above the
    /// args+return-slot area for generator function `d_nr`.
    ///
    /// Zone 1 and Zone 2 local slots start at `local_start = arg_size + 4`.
    /// Returns the number of bytes in `[local_start, max(slot+size))` for all
    /// non-argument variables.  This region is zeroed at first resume so that
    /// uninitialised text-local slots carry a null ptr, enabling safe
    /// `drop_text_locals_in_bytes` in `free_coroutine`.
    fn generator_zone2_size(d_nr: u32, data_ptr: *const Data) -> usize {
        if data_ptr.is_null() {
            return 0;
        }
        // SAFETY: `data_ptr` is the caller's `State::data_ptr`, whose `Data` outlives
        // that `State` — see the field.
        let data = unsafe { &*data_ptr };
        let Some(def) = data.definitions.get(d_nr as usize) else {
            return 0;
        };
        let vars = &def.variables();
        // local_start = total argument bytes + return-address slot.
        // @PLAN53 cluster 2 / S4: 8-rounded stepped spans, mirroring
        // scopes.rs's local_start.
        let step = |s: u16| crate::variables::aligned_stack_step(u32::from(s)) as u16;
        let local_start: u16 = vars
            .arguments()
            .iter()
            .map(|&a| step(var_size(vars.tp(a), &Context::Argument)))
            .sum::<u16>()
            .saturating_add(step(4));
        // top = absolute end of the last local variable (from frame base 0).
        let mut top: u16 = local_start;
        for v in 0..vars.count() {
            if vars.is_argument(v) {
                continue;
            }
            let slot = vars.stack(v);
            if slot == u16::MAX {
                continue;
            }
            let sz = vars.size(v, &Context::Variable);
            top = top.max(slot.saturating_add(sz));
        }
        // Return the SIZE of the local region (subtract the args+return-slot prefix).
        top.saturating_sub(local_start) as usize
    }

    /// S25.3 (C24): drop `String` objects embedded at text-local slots in a
    /// suspended generator's `stack_bytes`.
    ///
    /// Guards against uninitialised slots via null-ptr check: `generator_zone2_size`
    /// zeros the local region at first resume, so every text-local slot that was
    /// never written holds a zero ptr and is skipped here.
    ///
    /// # Safety
    /// Must only be called for `Suspended` frames whose local region was zeroed at
    /// first resume (Step 1 of S25.3).  Double-drop is prevented by zeroing each
    /// slot after `drop_in_place`.
    fn drop_text_locals_in_bytes(d_nr: u32, bytes: &mut Vec<u8>, data_ptr: *const Data) {
        if data_ptr.is_null() {
            return;
        }
        // SAFETY: `data_ptr` is the caller's `State::data_ptr`, whose `Data` outlives
        // that `State` — see the field.
        let data = unsafe { &*data_ptr };
        let Some(def) = data.definitions.get(d_nr as usize) else {
            return;
        };
        let vars = &def.variables();
        for v in 0..vars.count() {
            if vars.is_argument(v) {
                continue;
            }
            if !matches!(vars.tp(v), Type::Text(_)) {
                continue;
            }
            let slot = vars.stack(v);
            if slot == u16::MAX {
                continue;
            }
            let off = slot as usize;
            if off + std::mem::size_of::<String>() > bytes.len() {
                continue; // text local beyond yield snapshot — never assigned
            }
            // Read the String's ptr field (first word on any platform).
            // Null means uninitialised (zeroed at first resume); skip safely.
            let ptr_val: usize =
                unsafe { std::ptr::read_unaligned(bytes.as_ptr().add(off).cast::<usize>()) };
            if ptr_val == 0 {
                continue;
            }
            // Drop the String heap buffer and zero the slot to prevent double-drop.
            // SAFETY: stack_bytes stores Strings at their original stack offsets; the
            // slot is aligned as it was when pushed.  Unaligned cast is intentional.
            #[allow(clippy::cast_ptr_alignment)]
            unsafe {
                std::ptr::drop_in_place(bytes.as_mut_ptr().add(off).cast::<String>());
                std::ptr::write_bytes(
                    bytes.as_mut_ptr().add(off),
                    0,
                    std::mem::size_of::<String>(),
                );
            }
        }
    }

    /// Get a mutable reference to a coroutine frame.
    ///
    /// # Panics
    /// Panics if `idx` is 0 (null), out of range, or the slot is empty.
    pub fn coroutine_frame_mut(&mut self, idx: usize) -> &mut CoroutineFrame {
        assert!(idx > 0, "coroutine_frame_mut: null index");
        self.coroutines[idx]
            .as_mut()
            .expect("coroutine_frame_mut: empty slot")
    }

    /// S25.1 (CO1.3d): check whether a raw text pointer is inside a static text pool.
    /// Static Str values (from bytecode or the constant store) are permanently
    /// live and need no ownership transfer.
    fn is_in_text_code(&self, ptr: *const u8) -> bool {
        // Check the constant store (long strings >= 256 bytes stored via set_str).
        let cs = crate::database::CONST_STORE as usize;
        if cs < self.database.allocations.len() {
            let store = &self.database.allocations[cs];
            let store_base = store.ptr;
            let store_end = unsafe { store_base.add(store.capacity_words() as usize * 8) };
            if ptr >= store_base && ptr < store_end {
                return true;
            }
        }
        false
    }

    /// S25.1 (CO1.3d / P2-R1): scan the first `args_size` bytes of `stack_bytes` for
    /// text (`Str`) arguments.  For each non-null, non-static `Str`, clone the backing
    /// data into an owned `String`, update `stack_bytes` to point to the owned buffer,
    /// and record `(byte_offset, owned_string)` in the returned vec.
    ///
    /// After this call, the `Str` pointers in `stack_bytes` are independent of the
    /// caller's `String` allocations, so `OpFreeText` on the caller's side cannot
    /// dangle the coroutine's copy.
    fn serialise_text_args(
        &self,
        d_nr: u32,
        stack_bytes: &mut Vec<u8>,
        args_size: u32,
    ) -> Vec<(u32, String)> {
        if self.data_ptr.is_null() {
            return Vec::new();
        }
        // SAFETY: the `Data` outlives this `State` — see `data_ptr`.
        // `coroutine_create` is only reached from fill.rs during a run.
        let data = unsafe { &*self.data_ptr };
        if d_nr as usize >= data.definitions.len() {
            return Vec::new();
        }
        let def = &data.definitions[d_nr as usize];
        let mut text_owned: Vec<(u32, String)> = Vec::new();
        let mut byte_offset: usize = 0;

        for attr in def.attributes() {
            if byte_offset >= args_size as usize {
                break; // only scan the arg region
            }
            let attr_size = var_size(&attr.typedef, &Context::Argument) as usize;
            if matches!(&attr.typedef, Type::Text(_)) {
                // Read the Str from stack_bytes at byte_offset.
                // SAFETY: byte_offset + size_of::<Str>() <= args_size <= stack_bytes.len().
                // Str is stored unaligned in the byte-packed stack; read_unaligned is correct.
                #[allow(clippy::cast_ptr_alignment)]
                let str_val: Str = unsafe {
                    let src = stack_bytes.as_ptr().add(byte_offset).cast::<Str>();
                    std::ptr::read_unaligned(src)
                };
                // Skip null sentinel and static text (pointer lives in text_code).
                let is_null = str_val.ptr == STRING_NULL.as_ptr() || str_val.len == 0;
                if !is_null && !self.is_in_text_code(str_val.ptr) {
                    let owned = str_val.str().to_owned();
                    let new_str = Str::new(owned.as_str());
                    // Patch stack_bytes to point to the owned buffer.
                    #[allow(clippy::cast_ptr_alignment)]
                    unsafe {
                        let dst = stack_bytes.as_mut_ptr().add(byte_offset).cast::<Str>();
                        std::ptr::write_unaligned(dst, new_str);
                    }
                    text_owned.push((byte_offset as u32, owned));
                }
            }
            byte_offset += attr_size;
        }
        text_owned
    }

    // CO1.2: Create a coroutine frame — copy arguments into the frame without
    // entering the function body.
    pub fn coroutine_create(&mut self, d_nr: u32, args_size: u32, entry_pos: u32) {
        let args_base = self.stack_pos - args_size;
        let mut stack_bytes = vec![0u8; args_size as usize];
        let store = self.database.store(&self.stack_cur);
        let src = store.addr::<u8>(self.stack_cur.rec, self.stack_cur.pos + args_base);
        unsafe {
            std::ptr::copy_nonoverlapping(src, stack_bytes.as_mut_ptr(), args_size as usize);
        }
        // S25.1 (CO1.3d / P2-R1): serialise text args to owned Strings before the
        // caller's OpFreeText can free the backing allocations.
        let text_owned = self.serialise_text_args(d_nr, &mut stack_bytes, args_size);
        // CO1.3d: append the return-address slot expected by the function body.
        // fn_call pushes this slot for regular calls; coroutines must include it so that
        // get_var offsets computed at codegen time remain valid after resume.
        // @PLAN53 cluster 2 / S4 (2a): codegen lays the return slot at a STEPPED span
        // (codegen.rs frame setup: `position += step(4)`), so the captured frame must
        // reserve step(4) bytes here too.  With a raw 4 the captured `bytes.len()` is
        // `args_size + 4`, which is step(4)-4 = 4 bytes short of the stepped `local_start`;
        // the Created-resume TOS in `coroutine_next` then under-advances and every
        // argument reads 4 bytes high (n=42 → 42<<32).  Identity when LOFT_ALIGN is off
        // (step(4) == 4), so flag-OFF is byte-for-byte unchanged.
        let ret_slot = self.stack_step(4) as usize;
        stack_bytes.resize(stack_bytes.len() + ret_slot, 0);
        self.stack_pos = args_base;

        let frame = CoroutineFrame {
            d_nr,
            status: CoroutineStatus::Created,
            code_pos: entry_pos,
            stack_base: 0,
            caller_return_pos: 0,
            stack_bytes,
            text_owned, // S25.1: populated by serialise_text_args above
            call_frames: Vec::new(),
            call_depth: 0,
            #[cfg(debug_assertions)]
            saved_text_positions: std::collections::BTreeSet::new(),
            saved_store_generations: Vec::new(),
            generation: 0, // stamped by allocate_coroutine
        };
        let (idx, generation) = self.allocate_coroutine(frame);

        // `pos` carries the generation: a coroutine reference addresses no store, so the
        // field is free, and every handle to this frame then names the frame rather than
        // the index it happens to occupy.
        let db_ref = DbRef {
            store_nr: COROUTINE_STORE,
            rec: idx as u32,
            pos: generation,
        };
        self.put_stack(db_ref);
    }

    /// CO1.2: Advance a coroutine — restore stack, resume execution.
    /// # Panics
    /// Panics on re-entrant advance (coroutine already running).
    #[allow(clippy::too_many_lines)] // borrow-checker constraints prevent splitting this function
    pub fn coroutine_next(&mut self, value_size: u32) {
        let gen_ref = *self.get_stack::<DbRef>();

        if gen_ref.store_nr != COROUTINE_STORE || gen_ref.rec == 0 {
            // CO1.6c: push typed null sentinel.
            self.push_null_value(value_size);
            return;
        }
        let idx = gen_ref.rec as usize;
        // S23: defense-in-depth runtime guard — coroutine DbRefs must not cross
        // thread boundaries.  Worker State instances have only a null slot at index
        // 0; a rec from the main thread would be out-of-bounds here.
        assert!(
            idx < self.coroutines.len(),
            "coroutine DbRef (rec={idx}) out of range — \
             iterator<T> values must not cross thread boundaries \
             (use a non-generator worker function in par())"
        );
        // S26: slot may be None — freed on exhaustion by coroutine_return.
        // Treat as exhausted (same as the Exhausted variant).  A generation mismatch is
        // the same answer for the same reason: this handle's frame is gone, and whatever
        // now occupies the slot belongs to somebody else.
        if !self.coroutine_slot_matches(&gen_ref) {
            self.push_null_value(value_size);
            return;
        }
        let status = self.coroutine_frame_mut(idx).status;

        match status {
            CoroutineStatus::Exhausted => {
                self.push_null_value(value_size);
            }
            CoroutineStatus::Running => {
                panic!("re-entrant advance on coroutine {idx}");
            }
            CoroutineStatus::Created | CoroutineStatus::Suspended => {
                let caller_return_pos = self.code_pos;
                let call_depth = self.call_stack.len();
                let stack_base = self.stack_pos;
                {
                    let f = self.coroutine_frame_mut(idx);
                    f.caller_return_pos = caller_return_pos;
                    f.call_depth = call_depth;
                    f.stack_base = stack_base;
                    f.status = CoroutineStatus::Running;
                }

                let d_nr = self.coroutine_frame_mut(idx).d_nr;
                // Coverage: a generator's body does not enter through `fn_call` — it
                // resumes here — so without this every `iterator<T>` function reported as
                // never entered however thoroughly it was iterated.  Recorded on RESUME,
                // not on `coroutine_create`: creating a generator that is never iterated
                // runs none of its body, and a report that claims otherwise is worse than
                // one that misses it.
                if let Some(seen) = &mut self.entered_fns
                    && let Some(slot) = seen.get_mut(d_nr as usize)
                {
                    *slot = true;
                }
                let mut bytes = self.coroutine_frame_mut(idx).stack_bytes.clone();
                let code_pos = self.coroutine_frame_mut(idx).code_pos;
                let saved_frames: Vec<_> =
                    std::mem::take(&mut self.coroutine_frame_mut(idx).call_frames);

                // S27 (debug-only): restore the generator's text_positions entries
                // that were removed at yield.  The generator's locals are live again
                // once the stack bytes are copied back below.
                #[cfg(debug_assertions)]
                {
                    let saved: std::collections::BTreeSet<u32> =
                        std::mem::take(&mut self.coroutine_frame_mut(idx).saved_text_positions);
                    self.text_positions.extend(saved);
                }

                // S28: detect store mutations between yield and resume.  Any
                // live store whose generation changed since the last yield
                // MAY have invalidated DbRef locals held by the suspended
                // generator.  Because the guard snapshots EVERY live store
                // at yield (not just stores the frame's DbRef locals point
                // at), it false-positives on the most basic
                // iterator-consumer idioms — e.g. `out += [v]` inside
                // `for v in gen()` over an integer generator (the consumer
                // mutates `out`'s store; the generator holds no DbRefs;
                // there's no UAF hazard).  @P324 narrows this by demoting
                // the assert to a debug-only warning until a precise
                // narrowing (snapshot only stores reachable via the
                // suspended frame's DbRef variables) lands.  Production
                // code reads a stale DbRef as garbage rather than panicking,
                // which matches the rest of the interpreter's UAF-class
                // failure mode (Stores::valid only bounds-checks).
                #[cfg(debug_assertions)]
                {
                    let saved_gens: Vec<(u16, u32)> = self
                        .coroutine_frame_mut(idx)
                        .saved_store_generations
                        .clone();
                    for (store_nr, saved_gen) in saved_gens {
                        let cur_gen = self
                            .database
                            .allocations
                            .get(store_nr as usize)
                            .map_or(0, |s| s.generation);
                        if cur_gen != saved_gen {
                            eprintln!(
                                "S28 warning: store {store_nr} was mutated between coroutine \
                                 yields (generation at yield: {saved_gen}, now: {cur_gen}). \
                                 If the suspended generator holds a DbRef into this store, \
                                 it may read stale data on resume — see CAVEATS.md S28 / @P324."
                            );
                        }
                    }
                }

                // S25.1 (CO1.3d / M6-b): patch Str pointers in the cloned bytes to
                // reflect the current buffer addresses of the owned Strings.  Collect
                // (offset, Str) pairs while the frame borrow is live, then apply them
                // to the local `bytes` clone.  String heap buffers are stable: they are
                // not pushed, reallocated, or dropped between here and the copy below.
                let text_patches: Vec<(u32, Str)> = self
                    .coroutine_frame_mut(idx)
                    .text_owned
                    .iter()
                    .map(|(off, s)| (*off, Str::new(s.as_str())))
                    .collect();
                #[allow(clippy::cast_ptr_alignment)]
                for (offset, new_str) in &text_patches {
                    unsafe {
                        let dst = bytes.as_mut_ptr().add(*offset as usize).cast::<Str>();
                        std::ptr::write_unaligned(dst, *new_str);
                    }
                }

                // @P294: the frame-snapshot copy + the Created-status zone
                // zeroing below both write above the current stack top; grow
                // the stack store to fit before any direct write.
                self.ensure_stack(
                    bytes.len() as u32 + Self::generator_zone2_size(d_nr, self.data_ptr) as u32,
                );
                let dest = self.database.store_mut(&self.stack_cur).addr_span_mut(
                    self.stack_cur.rec,
                    self.stack_cur.pos + self.stack_pos,
                    bytes.len(),
                );
                unsafe {
                    std::ptr::copy_nonoverlapping(bytes.as_ptr(), dest, bytes.len());
                }
                self.stack_pos += bytes.len() as u32;

                // S25.3 (C24 / Step 1): on first resume, zero the local-variable
                // region so that uninitialised text-local slots carry a null ptr.
                // This is the prerequisite for safe `drop_text_locals_in_bytes` in
                // `free_coroutine` (Step 2): a null ptr means "not yet assigned;
                // skip drop".  Only needed for `Created` — `Suspended` frames have
                // already been through this path and their locals were live-assigned
                // before the preceding yield.
                if status == CoroutineStatus::Created {
                    let zone_size = Self::generator_zone2_size(d_nr, self.data_ptr);
                    if zone_size > 0 {
                        let zone_abs = self.stack_cur.pos + stack_base + bytes.len() as u32;
                        let store = self.database.store_mut(&self.stack_cur);
                        let ptr = store.addr_span_mut(self.stack_cur.rec, zone_abs, zone_size);
                        // SAFETY: zone_abs points inside the stack store; zone_size
                        // bytes there are within the pre-reserved frame region.
                        unsafe {
                            std::ptr::write_bytes(ptr, 0, zone_size);
                        }
                    }
                }

                self.call_stack.extend(saved_frames);
                self.active_coroutines.push(idx);
                self.code_pos = code_pos;
            }
        }
    }

    // CO1.6: check if a coroutine is exhausted.
    #[must_use]
    pub fn coroutine_exhausted(&self, gen_ref: &DbRef) -> bool {
        if gen_ref.store_nr != COROUTINE_STORE || gen_ref.rec == 0 {
            return true; // null iterator is exhausted
        }
        if !self.coroutine_slot_matches(gen_ref) {
            return true; // the frame this handle named is gone
        }
        match &self.coroutines[gen_ref.rec as usize] {
            Some(frame) => frame.status == CoroutineStatus::Exhausted,
            None => true,
        }
    }

    /// Does `gen_ref` still name a live frame — the right index AND the right occupant?
    ///
    /// A handle outlives the frame it points at: the frame is freed on exhaustion (S26)
    /// while the variable holding the handle lives to the end of its scope, where its
    /// `OpFreeRef` fires.  Comparing the generation stamp in `pos` is what stops that free
    /// — and any late advance — from reaching the generator that inherited the slot.
    pub(crate) fn coroutine_slot_matches(&self, gen_ref: &DbRef) -> bool {
        if gen_ref.store_nr != COROUTINE_STORE || gen_ref.rec == 0 {
            return false;
        }
        let idx = gen_ref.rec as usize;
        self.coroutines
            .get(idx)
            .and_then(Option::as_ref)
            .is_some_and(|frame| frame.generation == gen_ref.pos)
    }

    // CO1.6c: push a typed null sentinel onto the stack.
    fn push_null_value(&mut self, value_size: u32) {
        match value_size {
            4 => self.put_stack(i32::MIN), // integer null sentinel
            8 => self.put_stack(i64::MIN), // long null sentinel
            // Text Str sentinel: use STRING_NULL ("\0") so that the ptr is non-null
            // and conv_bool_from_text / append_text / str() don't crash on ptr=0.
            v if v == std::mem::size_of::<Str>() as u32 => {
                self.put_stack(Str::new(STRING_NULL));
            }
            _ => {
                // @PLAN53 cluster 2 / S4 (R4): push the value_size zero bytes as
                // ONE stepped slot.  A per-byte put_stack would round EACH byte
                // to 8 under LOFT_ALIGN, over-advancing catastrophically.  Off:
                // step is identity → advances value_size, same as the old loop.
                let step = self.stack_step(value_size);
                self.ensure_stack(step);
                let dst = self.database.store_mut(&self.stack_cur).addr_span_mut(
                    self.stack_cur.rec,
                    self.stack_cur.pos + self.stack_pos,
                    value_size as usize,
                );
                unsafe {
                    std::ptr::write_bytes(dst, 0, value_size as usize);
                }
                self.stack_pos += step;
            }
        }
    }

    /// Scan `locals_bytes` for text locals whose first
    /// 8 bytes (the `Str.ptr` field) fall inside a live non-stack store allocation.
    /// Emits a diagnostic warning; does not panic.  See COROUTINE.md CL-2b.
    #[cfg(all(debug_assertions, target_pointer_width = "64"))]
    fn warn_store_backed_text(&self, locals_bytes: &[u8], base_abs: u32, value_start_abs: u32) {
        // Collect first to release the borrow on self.text_positions.
        let positions: Vec<u32> = self
            .text_positions
            .range(base_abs..value_start_abs)
            .copied()
            .collect();
        for p in positions {
            let off = (p - base_abs) as usize;
            if off + 8 > locals_bytes.len() {
                continue;
            }
            let ptr_val = u64::from_ne_bytes(locals_bytes[off..off + 8].try_into().unwrap());
            if ptr_val == 0 {
                continue; // null / STRING_NULL — not store-backed
            }
            for (store_idx, store) in self.database.allocations.iter().enumerate() {
                if store.free || store_idx as u16 == self.stack_cur.store_nr {
                    continue;
                }
                let start = store.ptr as u64;
                let end = start + store.byte_capacity();
                if ptr_val >= start && ptr_val < end {
                    eprintln!(
                        "[P2-R5] coroutine_yield: text local at abs offset {p} holds \
                         a store-backed Str (ptr={ptr_val:#x}, store {store_idx}). \
                         If store {store_idx} or its backing record is freed before \
                         the next resume this Str will dangle (COROUTINE.md CL-2b)."
                    );
                    break;
                }
            }
        }
    }

    /// CO1.3b: suspend a running coroutine — serialise stack, return yielded value.
    /// # Panics
    /// Panics if no coroutine is currently active.
    pub fn coroutine_yield(&mut self, value_size: u32) {
        let idx = *self
            .active_coroutines
            .last()
            .expect("OpYield outside active coroutine");

        // Compute regions.
        let stack_top = self.stack_pos;
        let frame = self.coroutine_frame_mut(idx);
        let base = frame.stack_base;
        // @PLAN53 cluster 2 / S4: the yielded value sits in a stepped slot at TOS.
        let value_start = stack_top - self.stack_step(value_size);
        let locals_len = (value_start - base) as usize;

        // Serialise locals (CO1.3d: text locals are String objects — bitwise copy is safe
        // because String owns its heap buffer and no external code frees it while suspended).
        let mut locals_bytes = vec![0u8; locals_len];
        let vs = value_size as usize;
        let mut value_bytes = vec![0u8; vs];
        {
            let store = self.database.store(&self.stack_cur);
            let src = store.addr::<u8>(self.stack_cur.rec, self.stack_cur.pos + base);
            unsafe {
                std::ptr::copy_nonoverlapping(src, locals_bytes.as_mut_ptr(), locals_len);
            }
            let val_src = store.addr::<u8>(self.stack_cur.rec, self.stack_cur.pos + value_start);
            unsafe {
                std::ptr::copy_nonoverlapping(val_src, value_bytes.as_mut_ptr(), vs);
            }
        }

        // warn if any text local is a store-backed Str.
        // See COROUTINE.md CL-2b and SAFE.md § P2-R5.
        #[cfg(all(debug_assertions, target_pointer_width = "64"))]
        self.warn_store_backed_text(
            &locals_bytes,
            self.stack_cur.pos + base,
            self.stack_cur.pos + value_start,
        );

        // Extract frame fields before mutable borrow conflicts.
        let call_depth = self.coroutine_frame_mut(idx).call_depth;
        let caller_return_pos = self.coroutine_frame_mut(idx).caller_return_pos;

        // Save call frames above the base depth.
        let saved_frames = self.call_stack[call_depth..].to_vec();
        self.call_stack.truncate(call_depth);

        let code_pos = self.code_pos;
        {
            let frame = self.coroutine_frame_mut(idx);
            frame.stack_bytes = locals_bytes;
            // CO1.3d: text locals are String objects (24 B) in stack_bytes.
            // Bitwise copy is safe — no external code frees the heap buffers while
            // suspended.  At resume, coroutine_next restores the raw bytes.
            // At exhaustion, OpFreeText fires before OpCoroutineReturn (no leak).
            // Early-break leak fixed by free_coroutine / S25.3.
            // frame.text_owned holds text-arg clones from coroutine_create; unchanged.
            frame.call_frames = saved_frames;
            frame.code_pos = code_pos;
            frame.status = CoroutineStatus::Suspended;
        }

        // S27 (debug-only): remove text_positions entries for the generator's locals
        // [base, value_start) and save them in the frame.  While suspended, the consumer
        // may create text values at the same absolute stack positions; keeping the
        // generator's entries would mask missing or double OpFreeText calls.
        // CO1.3d (text locals): the raw-bytes copy in stack_bytes is safe across
        // yield/resume cycles — String heap buffers are not freed while suspended and
        // are restored intact by coroutine_next.  At exhaustion, OpFreeText is emitted
        // before OpCoroutineReturn so live-stack Strings are freed normally.  The one
        // remaining leak (early break → free_coroutine) is fixed by S25.3.
        #[cfg(debug_assertions)]
        {
            let locals_range = base..value_start;
            let to_save: std::collections::BTreeSet<u32> =
                self.text_positions.range(locals_range).copied().collect();
            for p in &to_save {
                self.text_positions.remove(p);
            }
            self.coroutine_frame_mut(idx).saved_text_positions = to_save;
        }

        // CO1.9/S28: snapshot all live, unlocked store generations at the yield point.
        // `coroutine_next` compares these on resume and panics if any store was mutated
        // while the generator was suspended.  Locked stores are worker snapshots that
        // can never change; skip them.  Always compiled in after CO1.9.
        {
            let gens: Vec<(u16, u32)> = self
                .database
                .allocations
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.free && !s.read_only)
                .map(|(i, s)| (i as u16, s.generation))
                .collect();
            self.coroutine_frame_mut(idx).saved_store_generations = gens;
        }

        self.active_coroutines.pop();

        // Slide the yielded value to stack_base.
        let dest = self.database.store_mut(&self.stack_cur).addr_span_mut(
            self.stack_cur.rec,
            self.stack_cur.pos + base,
            vs,
        );
        unsafe {
            std::ptr::copy_nonoverlapping(value_bytes.as_ptr(), dest, vs);
        }
        // @PLAN53 cluster 2 / S4: the value slid to `base` occupies a stepped slot.
        self.stack_pos = base + self.stack_step(value_size);

        // Return to consumer.
        self.code_pos = caller_return_pos;
    }

    /// CO1.3a: exhaust a running coroutine — cleanup and return null to consumer.
    /// # Panics
    /// Panics if no coroutine is currently active.
    pub fn coroutine_return(&mut self, value_size: u32) {
        let idx = *self
            .active_coroutines
            .last()
            .expect("OpCoroutineReturn outside active coroutine");
        let frame = self.coroutine_frame_mut(idx);

        // Drop serialised state.
        frame.text_owned.clear();
        frame.stack_bytes.clear();

        let call_depth = frame.call_depth;
        let stack_base = frame.stack_base;
        let caller_return_pos = frame.caller_return_pos;

        // Exhaust and immediately free the slot (S26).
        // Setting the slot to None prevents unbounded growth of the coroutines table
        // when many generators are created over a program's lifetime.
        // coroutine_exhausted() treats None as exhausted, so callers see no difference.
        frame.status = CoroutineStatus::Exhausted;
        self.active_coroutines.pop();
        // Free the slot: coroutine_exhausted() returns true for None entries.
        self.coroutines[idx] = None;

        // Restore call stack to consumer depth.
        self.call_stack.truncate(call_depth);

        // Rewind stack to frame base; push typed null.
        self.stack_pos = stack_base;
        self.push_null_value(value_size);

        // Return to consumer.
        self.code_pos = caller_return_pos;
    }

    /**
    Clear the stack of local variables, possibly return a value.
    * `value` - Size of the return value.
    * `discard` - The amount of space claimed on the stack at this point.
    # Panics
    When texts are not freed from the stack beforehand.
    */
    pub fn free_stack(&mut self, value: u8, discard: u16) {
        let pos = self.stack_pos;
        self.stack_pos -= u32::from(discard);
        if cfg!(debug_assertions) {
            let orphans: Vec<u32> = self
                .text_positions
                .range(self.stack_pos..=pos)
                .copied()
                .collect();
            for p in orphans {
                self.text_positions.remove(&p);
            }
        }
        self.copy_result(value, pos, self.stack_pos);
    }

    /// Advance the stack pointer by `size` bytes, reserving space for pre-claimed variables.
    /// @P294: ensure the value-stack store (`stack_cur`) buffer can hold a
    /// write reaching `extra` bytes above the current `stack_pos`.  The
    /// stack store is allocated once in `new()` / `new_worker()` and never
    /// re-`claim`s, so its buffer only grows here; without this, deep call
    /// nesting silently wrote past the initial 1000-word buffer (the bounds
    /// check in `Store::addr_mut` is a `debug_assert!`, compiled out in the
    /// release library), corrupting the heap.  Cheap in the common case:
    /// one comparison against the cached `stack_cap_bytes`.
    #[inline]
    pub(crate) fn ensure_stack(&mut self, extra: u32) {
        // Highest byte offset a write at the current top may touch:
        // addr_mut computes `rec * 8 + (pos + stack_pos)`.
        let top = self.stack_cur.rec * 8 + self.stack_cur.pos + self.stack_pos + extra;
        if top < self.stack_cap_bytes {
            return;
        }
        // Grow with a word of slack so the exact-fit boundary still passes
        // `addr_mut`'s `offset + size <= size * 8` check.
        let needed_words = top.div_ceil(8) + 1;
        let store = self.database.store_mut(&self.stack_cur);
        store.grow_words(needed_words);
        // loft#935 — the buffer grew; the RECORD the stack lives in has to grow
        // with it. Without this every frame byte above the initial claim is
        // outside record 1, which `Store::valid` reports under debug assertions
        // and a release build simply writes.
        store.extend_primary_to_store_end();
        self.stack_cap_bytes = store.byte_capacity() as u32;
    }

    /// @PLAN53 cluster 2 / S4 — one eval-TOS / frame-reserve advance, always
    /// rounded up to 8.  Codegen's `Stack::step` mirrors this so emitted `pos`
    /// operands match the runtime `stack_pos` (S1 lockstep invariant).
    #[inline]
    #[allow(clippy::unused_self)]
    pub(crate) fn stack_step(&self, size: u32) -> u32 {
        crate::variables::aligned_stack_step(size)
    }

    /// @PLAN53 cluster 2 / S4 — homegrown alignment guard (debug + aligned only).
    /// `pos` is the byte offset passed to `addr`/`addr_mut` on the stack store
    /// (i.e. `stack_cur.pos + frame_offset`).  A typed `T` accessed there must
    /// sit on its natural alignment boundary; an unaligned `&T` is the cluster-2
    /// UB.  Asserts it loudly AT the access site on ANY rustc — no Miri needed.
    /// The store buffer base + `rec*8` are 8-aligned, so `(rec*8 + pos) % align`
    /// is the true access alignment.  NB: this checks the SLOT address, never
    /// `Str.ptr` (string slices legitimately start at any byte).
    ///
    /// Gated on the `stack_align_guard` cargo feature — NOT `debug_assertions`,
    /// because `[profile.dev.package.loft]` sets `debug-assertions = false`
    /// (so `cfg(debug_assertions)` is off inside this crate even under `cargo
    /// test`, which silently disabled an earlier version of this guard).  The
    /// method (and its callers' `pos` computation) does not exist unless the
    /// feature is on, so the hot push/pop path has ZERO footprint by default.
    /// Run the S4 aligned-stack work with `cargo test --features
    /// stack_align_guard` to arm it.  Uses `assert_eq!` (not `debug_assert!`)
    /// since this crate's debug-assertions are off.
    #[cfg(feature = "stack_align_guard")]
    #[inline]
    pub(crate) fn check_stack_align<T>(&self, pos: u32) {
        let al = std::mem::align_of::<T>() as u32;
        let abs = self.stack_cur.rec * 8 + pos;
        assert_eq!(
            abs % al,
            0,
            "S4 unaligned stack access: {} at abs offset {abs} (align {al}) — \
             cluster-2 UB: a typed value landed off its alignment boundary",
            std::any::type_name::<T>(),
        );
    }

    pub fn reserve_frame(&mut self, size: u16) {
        let step = self.stack_step(u32::from(size));
        self.ensure_stack(step);
        let base = self.stack_pos;
        self.stack_pos += step;
        if self.stack_pos > self.stack_high {
            self.stack_high = self.stack_pos;
        }
        // LOFT_POISON=1 (@PLN54 S3, stack half): fill the freshly-reserved frame
        // region `[base, base+step)` with 0xDEADBEEF.  That region is ABOVE the
        // old TOS — provably dead by the stack discipline — so this cannot touch
        // any live value (unlike poisoning at *free*, where the pop primitive and
        // the transient return value still occupy the vacated region; see
        // plans/54-sanitizer-coverage-expansion/STACK_POISON_DESIGN.md).  Every
        // slot is written (`OpInit*` / push) before it is read (definite
        // assignment), so a correct program never observes the sentinel; a read
        // of an unwritten slot (uninitialised, or a cross-frame stale read whose
        // bytes a prior frame left) hits it — a `DbRef` read trips the
        // `get_stack<DbRef>` OOB guard (store_nr=0xBEEF).  Off by default.
        // @PLN154 — the frame just reserved holds no values yet.  This is the same region
        // and the same argument `LOFT_POISON` uses below: above the old TOS, provably dead
        // by the stack discipline, so clearing its tags cannot lose a live one.  The
        // difference is that the shadow does not need the slot to hold a distinguishable
        // byte pattern to say so.
        if step != 0 && self.verify_on {
            let at = (self.stack_cur.rec * 8 + self.stack_cur.pos + base) as usize;
            self.database
                .store_mut(&self.stack_cur)
                .shadow_kill(at, step as usize);
        }
        if step != 0 && crate::keys::poison_enabled() {
            const POISON: [u8; 4] = [0xEF, 0xBE, 0xAD, 0xDE];
            let rec = self.stack_cur.rec;
            let field_base = self.stack_cur.pos + base;
            let store = self.database.store_mut(&self.stack_cur);
            for off in 0..step {
                *store.addr_mut::<u8>(rec, field_base + off) = POISON[(off & 3) as usize];
            }
        }
    }

    pub(crate) fn copy_result(&mut self, value: u8, pos: u32, fn_stack: u32) {
        let size = u32::from(value);
        if value > 0 {
            // @PLAN53 cluster 2 / S4: the returned value sits at the LOW end of
            // a stepped slot on the callee's TOS — `put_stack` wrote the real
            // `size` bytes at `pos - step(size)` and advanced TOS by `step(size)`
            // (e.g. a 12-byte `Reference` in a 16-byte slot, 4 bytes padding on
            // top).  Back up by `step(size)`, NOT the raw `size`, or the read is
            // shifted into the padding → a 4-byte-garbled `DbRef` whose later
            // `OpFreeRef` corrupts the heap (the p117 runaway).  Identity off.
            let from_pos = self.stack_cur.plus(pos).min(self.stack_step(size));
            let to_pos = self.stack_cur.plus(fn_stack);
            self.database.copy_block(&from_pos, &to_pos, size);
        }
        // LOFT_UAF_GEN (c): the shadow must FOLLOW the value.  This slide is a raw
        // `copy_block`, so it never passes through `put_stack` — the one writer that
        // keeps the shadow in step with the eval stack.  Left alone, the destination
        // offset keeps whatever stamp its previous occupant left, and the very next
        // pop compares a returned DbRef against a gen belonging to some earlier value
        // — a different store, or the same slot an allocation ago.  That mismatch was
        // the detector's residual false positive: a loop calling a struct-returning
        // function reported `gen 0 at push` on every backend, on programs with no
        // stale read at all (`LOFT_NO_SLOT_REUSE=1` + `LOFT_POISON=1` stay clean).
        //
        // Move the stamp with the bytes instead: the SOURCE stamp is the real one
        // (the callee's `put_stack` wrote it when it pushed the return value), so it
        // transfers to the destination and the source is cleared.  Anything else
        // clears the destination — a stamp is only ever kept for a DbRef that is live
        // on the stack right now.  Detection is unaffected: a returned ref whose store
        // was freed since the callee pushed it still carries the older gen, and still
        // reports at the consuming pop.
        if crate::keys::uaf_gen_enabled() {
            if size == size_of::<DbRef>() as u32 {
                crate::keys::uaf_move_shadow(pos - self.stack_step(size), fn_stack);
            } else {
                crate::keys::uaf_clear_shadow(fn_stack);
            }
        }
        // @PLAN53 cluster 2 / S4: the returned value occupies a stepped slot
        // on the caller's TOS (matching codegen's `position += step(ret)`);
        // the copy itself moves the real `size` bytes.  Identity when off.
        self.stack_pos = fn_stack + self.stack_step(size);
    }

    /**
    Write to the byte code.
    # Panics
    When that was problematic
    */
    pub fn code_put<T>(&mut self, on: u32, value: T) {
        unsafe {
            let off = Arc::make_mut(&mut self.bytecode)
                .as_mut_ptr()
                .offset(on as isize)
                .cast::<T>();
            // The bytecode buffer is byte-granular (`Vec<u8>`); a `T` wider than
            // 1 byte usually lands at an unaligned offset.  Constructing `&mut T`
            // there is UB even where the hardware tolerates the access (the
            // @PLAN53 cluster-1 Miri finding) — write through the unaligned
            // intrinsic instead, which is defined at any alignment.
            off.write_unaligned(value);
        }
    }

    /** Remember the stack position for the current code. */
    pub fn remember_stack(&mut self, position: u16) {
        self.stack.insert(self.code_pos, position);
    }

    /**
    Add to the byte code.
    # Panics
    When that was problematic
    */
    pub fn code_add<T: std::fmt::Display>(&mut self, value: T) {
        let bc = Arc::make_mut(&mut self.bytecode);
        if self.code_pos as usize + size_of::<T>() > bc.len() {
            bc.resize(self.code_pos as usize + size_of::<T>(), 0);
        }
        unsafe {
            let off = bc.as_mut_ptr().offset(self.code_pos as isize).cast::<T>();
            self.code_pos += u32::try_from(size_of::<T>()).expect("Problem");
            // Unaligned by construction — see code_put (@PLAN53 cluster 1).
            off.write_unaligned(value);
        }
    }

    pub fn code_add_str(&mut self, value: &str) {
        self.code_add(value.len() as u8);
        let bc = Arc::make_mut(&mut self.bytecode);
        if self.code_pos as usize + value.len() > bc.len() {
            bc.resize(self.code_pos as usize + value.len(), 0);
        }
        unsafe {
            let off = bc.as_mut_ptr().offset(self.code_pos as isize);
            value.as_ptr().copy_to(off, value.len());
        }
        self.code_pos += value.len() as u32;
    }

    /** Get a value from the byte-code increasing the position to after this value
    # Panics
    When the position is outside the byte-code
    */
    pub fn code<T: Copy>(&mut self) -> T {
        assert!(
            self.code_pos + (size_of::<T>() as u32) <= self.bytecode.len() as u32,
            "Position {} + {} outside generated code {}",
            self.code_pos,
            size_of::<T>(),
            self.bytecode.len()
        );
        unsafe {
            let off = self
                .bytecode
                .as_ptr()
                .offset(self.code_pos as isize)
                .cast::<T>();
            self.code_pos += size_of::<T>() as u32;
            // Returns the operand BY VALUE via the unaligned read intrinsic.
            // The buffer is byte-granular, so a `&T` into it would be an
            // unaligned reference — UB (the @PLAN53 cluster-1 Miri finding) —
            // even on x86 where the load itself is tolerated.
            off.read_unaligned()
        }
    }

    pub fn code_str(&mut self) -> &str {
        let len = self.code::<u8>();
        unsafe {
            let off = self.bytecode.as_ptr().offset(self.code_pos as isize);
            self.code_pos += u32::from(len);
            std::str::from_utf8_unchecked(std::slice::from_raw_parts(off, len as usize))
        }
    }

    /**
    Pull a value from stack
    # Panics
    When the stack has no values left
    */
    #[must_use]
    pub fn get_stack<T: 'static>(&mut self) -> &T {
        assert!(
            (size_of::<T>() as u32) < self.stack_pos,
            "No elements left on the stack {} < {}",
            self.stack_pos,
            size_of::<T>() as u32
        );
        self.stack_pos -= self.stack_step(size_of::<T>() as u32);
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<T>(self.stack_cur.pos + self.stack_pos);
        // @PLN154 — check high.  The pop is also a LIFO consume, so the slot's tags go
        // with it: the next occupant of this offset inherits nothing from this value, and
        // a later read there has to be earned by a later write.
        if self.verify_on {
            self.verify_slot::<T>("get_stack", self.stack_pos);
            let at = (self.stack_cur.rec * 8 + self.stack_cur.pos + self.stack_pos) as usize;
            let step = self.stack_step(size_of::<T>() as u32) as usize;
            self.database
                .store_mut(&self.stack_cur)
                .shadow_kill(at, step);
        }
        let r = self
            .database
            .store(&self.stack_cur)
            .addr::<T>(self.stack_cur.rec, self.stack_cur.pos + self.stack_pos);
        #[cfg(debug_assertions)]
        {
            if std::any::TypeId::of::<T>() == std::any::TypeId::of::<DbRef>() {
                let db: &DbRef = unsafe { &*(r as *const T as *const DbRef) };
                if !(db.store_nr == u16::MAX
                    || (db.store_nr as usize) < self.database.allocations.len())
                {
                    let (op_pc, op_code, fn_d_nr) = crate::crash_report::last_context();
                    panic!(
                        "get_stack<DbRef>: OOB store_nr={} (allocations.len()={}) \
                         rec={} pos={} code_pos={} — corrupt DbRef on interpreter stack \
                         [last op: pc={} op_code={} fn_d_nr={}]",
                        db.store_nr,
                        self.database.allocations.len(),
                        db.rec,
                        db.pos,
                        self.code_pos,
                        op_pc,
                        op_code,
                        fn_d_nr,
                    );
                }
            }
        }
        // LOFT_UAF_GEN (c): a popped DbRef whose stamped push-gen differs from the slot's
        // CURRENT gen was freed+reused between push and pop — a stale read store_nr alone
        // cannot see, and (unlike the free-site scan) sound: the gen tells old occupant
        // from a re-claimed new one. Deduped by read site.
        if crate::keys::uaf_gen_enabled()
            && std::any::TypeId::of::<T>() == std::any::TypeId::of::<DbRef>()
        {
            let db: &DbRef = unsafe { &*std::ptr::from_ref::<T>(r).cast::<DbRef>() };
            // Read the stamp, then CONSUME it (LIFO): the stack is last-in-first-out, so a
            // stamp belongs to exactly the pop that matches its push. Clearing on pop stops
            // a stale stamp from surviving to a later unrelated read once a non-DbRef push
            // reuses the offset — the main residual-false-positive source.
            let stamped = crate::keys::uaf_shadow_gen(self.stack_pos, db.store_nr);
            crate::keys::uaf_clear_shadow(self.stack_pos);
            // Only stamped < current is a genuine reuse-SINCE-push (gen increases
            // monotonically per slot). A stamped >= current is not a reuse-since-push; the
            // `<` test drops stale stamps too.
            if db.store_nr != u16::MAX
                && (db.store_nr as usize) < self.database.allocations.len()
                && let Some(stamped) = stamped
                && stamped < crate::keys::uaf_slot_gen(db.store_nr)
            {
                thread_local! {
                    static REPORTED_GEN: std::cell::RefCell<std::collections::HashSet<u32>> =
                        std::cell::RefCell::new(std::collections::HashSet::new());
                }
                if REPORTED_GEN.with(|s| s.borrow_mut().insert(self.code_pos)) {
                    let line = self
                        .line_numbers
                        .range(..=self.code_pos)
                        .next_back()
                        .map_or(0, |(_, &v)| v);
                    // @PLN118 arc B — attribute the FREE site. Prefer the CAUSAL free (the one
                    // that took the slot to `stamped + 1`, making THIS ref stale) over the
                    // last free — a heavily-reused slot's last free is a different occupant's.
                    // Names the chokepoint (the dropped dep) rather than only the read symptom.
                    // @PLN118 arc E — also name the freeing OP (recorded with the free site) so
                    // the report says WHICH free to fix, not just where; needs `Data`, valid
                    // throughout execution (null only in a parallel worker, tolerated below).
                    // SAFETY: the `Data` outlives this `State` — see `data_ptr`.
                    // `as_ref` folds the null a parallel worker leaves behind into None,
                    // so no path here dereferences an unchecked pointer.
                    let data: Option<&Data> = unsafe { self.data_ptr.as_ref() };
                    let op_name = |opc: u16| -> String {
                        data.and_then(|d| d.operator_name(opc))
                            .map_or_else(|| format!("op#{opc}"), str::to_string)
                    };
                    let free_str = crate::keys::uaf_freed_pc_at_gen(db.store_nr, stamped + 1)
                        .or_else(|| crate::keys::uaf_freed_pc(db.store_nr))
                        .map_or_else(String::new, |(fpc, _d, op)| {
                            let fline = self
                                .line_numbers
                                .range(..=fpc)
                                .next_back()
                                .map_or(0, |(_, &v)| v);
                            format!(
                                " (freed at code_pos={fpc}, line {fline}, by {})",
                                op_name(op)
                            )
                        });
                    // @PLN118 arc E — the READING op is the one currently dispatching (this
                    // pop happens inside its handler); `last_context` holds its opcode byte.
                    let (_rpc, read_op, _fd) = crate::crash_report::last_context();
                    let read_op_str = op_name(u16::from(read_op));
                    // @PLN118 arc B refinement — a stale read DURING a record deep-copy is the
                    // copy reading a stale SUB-reference: the copy is incomplete and the source
                    // free is CORRECT — so name the COPY as the op to fix, not the free. A stale
                    // read outside a copy IS a premature-free candidate. This is the distinction
                    // that turns "prematurely freed" (which points at a correctly-placed temp
                    // free) into "incomplete record-copy" (which points at the actual op).
                    let verdict = if crate::keys::uaf_in_copy() {
                        "INCOMPLETE RECORD-COPY — a copy read a stale sub-reference; the copy \
                         did not finish deep-copying before the source was freed (the free is \
                         correct — fix the copy, not the free)"
                    } else {
                        "PREMATURE FREE — a plain deref of a store freed while this ref was live \
                         (fix the free / the dropped dep)"
                    };
                    eprintln!(
                        "[uaf-gen] stale DbRef popped: store #{} (rec={}, pos={}) was gen {stamped} \
                         at push but is now gen {} (freed+reused since) — read at code_pos={} \
                         (line {line}) by {read_op_str}{free_str} — {verdict}",
                        db.store_nr,
                        db.rec,
                        db.pos,
                        crate::keys::uaf_slot_gen(db.store_nr),
                        self.code_pos,
                    );
                }
            }
        }
        r
    }

    /// `parallel {}` — read the arm count for `parallel_arm`/`parallel_join`.
    pub fn parallel_begin(&mut self) {
        let n_arms = self.code::<u8>();
        self.parallel_n_arms = n_arms;
        self.parallel_arm_positions.clear();
    }

    /// `parallel {}` — read the arm's bytecode offset and record it.
    pub fn parallel_arm(&mut self) {
        let offset = self.code::<u16>();
        self.parallel_arm_positions.push(offset);
    }

    /// `parallel {}` — spawn threads for all recorded arms and join.
    /// Each arm runs as a void function at its bytecode position.
    ///
    /// P245: captures the parent's stack contents (offsets 4..stack_pos)
    /// as a snapshot and hands it to each worker.  This lets arm
    /// bodies reference outer-scope variables — the arm's bytecode
    /// addresses them at offsets that match the parent's layout, and
    /// without the snapshot the worker reads garbage from an empty
    /// stack.  Arms still get isolated `Stores` clones, so writes
    /// inside an arm don't propagate back to the parent.
    pub fn parallel_join(&mut self) {
        let positions: Vec<u32> = self
            .parallel_arm_positions
            .drain(..)
            .map(|off| self.code_pos + u32::from(off))
            .collect();
        let parent_snapshot: Arc<Vec<u8>> = if self.stack_pos > 4 {
            let store = self.database.store(&self.stack_cur);
            let ptr = store.addr::<u8>(self.stack_cur.rec, self.stack_cur.pos + 4);
            let len = (self.stack_pos - 4) as usize;
            let bytes = unsafe { std::slice::from_raw_parts(ptr, len).to_vec() };
            Arc::new(bytes)
        } else {
            Arc::new(Vec::new())
        };
        let program = crate::parallel::WorkerProgram {
            bytecode: Arc::clone(&self.bytecode),
            library: Arc::clone(&self.library),
            stack_trace_lib_nr: self.stack_trace_lib_nr,
            data_ptr: self.data_ptr,
            fn_positions: Arc::new(self.fn_positions.clone()),
            line_numbers: Arc::new(self.line_numbers.clone()),
        };
        crate::parallel::run_parallel_block(&self.database, program, &positions, &parent_snapshot);
        // The worker's halt is re-raised by the dispatch loop's own check, which every par
        // family passes through — this site had its own copy first, and keeping both would
        // be two homes for one decision (and did hide, in the bite proof, that the block
        // form was covered while the other three were not).
    }

    pub fn get_var<T: 'static>(&mut self, pos: u16) -> &T {
        // get_var reads T at (stack_pos - pos); pos > stack_pos would underflow.
        // pos < size_of::<T>() is also invalid (read extends before the frame base).
        // Note: pos == 0 is valid when accessing a pre-reserved frame slot above the
        // current evaluation stack (e.g. immediately after ReserveFrame).
        debug_assert!(
            u32::from(pos) <= self.stack_pos,
            "get_var: pos={pos} exceeds stack_pos={} (frame underflow)",
            self.stack_pos
        );
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<T>(self.stack_cur.pos + self.stack_pos - u32::from(pos));
        // @PLN154 — a frame read.  No kill: the slot stays live, and a local read twice is
        // read twice.
        if self.verify_on {
            self.verify_slot::<T>("get_var", self.stack_pos - u32::from(pos));
        }
        self.database.store(&self.stack_cur).addr::<T>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        )
    }

    /// @PLN154 — report a read of `width` bytes at frame offset `at` that no write tagged.
    ///
    /// Named where the check is made rather than where the tag was set, because the answer
    /// a reader needs is *which read got a value nobody wrote* — the write that did not
    /// happen has no site.
    #[cold]
    #[inline(never)]
    fn verify_slot<T: 'static>(&self, what: &str, at: u32) {
        use crate::stack_verify::SlotState;
        let width = size_of::<T>();
        let abs = (self.stack_cur.rec * 8 + self.stack_cur.pos + at) as usize;
        let store = self.database.store(&self.stack_cur);
        if !store.shadow_armed() {
            return;
        }
        let state = store.shadow_state(abs, width, crate::stack_verify::kind_of::<T>());
        match state {
            SlotState::Written => return,
            SlotState::Partial => {
                crate::stack_verify::note_partial();
                return;
            }
            SlotState::Unwritten | SlotState::Mismatch { .. } | SlotState::Stale => {}
        }
        let line = self
            .line_numbers
            .range(..=self.code_pos)
            .next_back()
            .map_or(0, |(_, &v)| v);
        let (pc, op, _fn_d_nr) = crate::crash_report::last_context();
        let ty = std::any::type_name::<T>();
        match state {
            SlotState::Unwritten => {
                crate::stack_verify::report_uninit(what, ty, at, width, pc, line, u16::from(op));
            }
            SlotState::Mismatch { wrote } => {
                crate::stack_verify::report_mismatch(
                    what,
                    ty,
                    at,
                    width,
                    wrote,
                    false,
                    pc,
                    line,
                    u16::from(op),
                );
            }
            SlotState::Stale => {
                let abs = (self.stack_cur.rec * 8 + self.stack_cur.pos + at) as usize;
                let rec = self
                    .database
                    .store(&self.stack_cur)
                    .addr::<DbRef>(self.stack_cur.rec, self.stack_cur.pos + at)
                    .rec;
                let _ = abs;
                crate::stack_verify::report_stale(what, ty, at, rec, pc, line, u16::from(op));
            }
            SlotState::Partial | SlotState::Written => {}
        }
    }

    /// @PLN154 phase 3 — a record moved during the operator that just ran, so every frame
    /// slot naming it is now stale.
    ///
    /// The scan is exact rather than a guess, and that is what phases 1-2 bought: the shadow
    /// already says which bytes are the BASE of a handle, so this reads the twelve bytes it
    /// knows are a `DbRef` and compares, instead of treating every eight-byte-aligned word in
    /// the frame as a possible reference.  The whole live frame is scanned, not the top one:
    /// a view handed down two calls is exactly the case a caller-frame scan would miss.
    ///
    /// It runs at the END of the operator, which is also the first moment the containers that
    /// legitimately track the move have finished updating themselves — scanning earlier would
    /// report the vector's own header on its way to being rewritten.
    #[cold]
    #[inline(never)]
    fn mark_stale_handles(&mut self) {
        let moved = crate::stack_verify::take_relocations();
        if moved.is_empty() {
            return;
        }
        let base = self.stack_cur.rec * 8 + self.stack_cur.pos;
        let top = self.stack_pos;
        let mut hits: Vec<u32> = Vec::new();
        {
            let store = self.database.store(&self.stack_cur);
            let mut off = 0u32;
            while off + size_of::<DbRef>() as u32 <= top {
                if store.shadow_handle_base_at((base + off) as usize) {
                    // The shadow is indexed ABSOLUTELY (`rec * 8 + fld`) and `addr` takes the
                    // FIELD, so the record's own bytes are counted once here and not twice.
                    let db = store.addr::<DbRef>(self.stack_cur.rec, self.stack_cur.pos + off);
                    if crate::stack_verify::trace() {
                        eprintln!(
                            "[stale-scan] off={off} handle store={} rec={} pos={} moved={:?}",
                            db.store_nr, db.rec, db.pos, moved
                        );
                    }
                    if moved
                        .iter()
                        .any(|&(st, old, _)| st == db.store_nr && old == db.rec)
                    {
                        hits.push(off);
                    }
                }
                off += 4;
            }
        }
        crate::stack_verify::note_marked(hits.len() as u64);
        let store = self.database.store_mut(&self.stack_cur);
        for off in hits {
            store.shadow_stale((base + off) as usize, size_of::<DbRef>());
        }
    }

    pub fn mut_var<T: 'static>(&mut self, pos: u16) -> &mut T {
        debug_assert!(
            u32::from(pos) <= self.stack_pos,
            "mut_var: pos={pos} exceeds stack_pos={} (frame underflow)",
            self.stack_pos
        );
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<T>(self.stack_cur.pos + self.stack_pos - u32::from(pos));
        self.database.store_mut(&self.stack_cur).addr_mut::<T>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        )
    }

    pub fn put_var<T: 'static>(&mut self, pos: u16, value: T) {
        // @PLAN53 cluster 2 / S4: the value's footprint on the stack is its
        // stepped span (matches the get_stack/put_stack steps it pairs with);
        // identity when LOFT_ALIGN off.
        let step = self.stack_step(size_of::<T>() as u32);
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<T>(self.stack_cur.pos + self.stack_pos + step - u32::from(pos));
        *self.database.store_mut(&self.stack_cur).addr_mut::<T>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos + step - u32::from(pos),
        ) = value;
    }

    /// Plan-04 Phase 2h: positional variant of `conv_ref_from_null`.
    /// Writes a null `DbRef` (12 bytes) at the frame slot reached by
    /// `self.stack_pos - pos` (matches `get_var::<DbRef>`'s
    /// addressing).  Does NOT push onto the eval stack — the frame
    /// is assumed to be already reserved.
    ///
    /// `pos` is read from the bytecode stream as a `const u16`.
    pub fn init_ref(&mut self) {
        let pos = self.code::<u16>();
        // A ref local's null-init writes the NULL sentinel — the same thing
        // `--native` writes (`let mut var_x: DbRef = DbRef::NULL`).
        //
        // It used to ALLOCATE an empty store, and that is what made a call-site
        // return buffer (`__ref_N`) a real store the caller kept and freed at scope
        // exit, while the callee — which cannot tell a caller-supplied buffer from
        // one the runtime handed it (`CallRef`, the cdylib bridge) — freed it too.
        // The second free released a slot recycled since, and the next allocation
        // took it back (loft#1085).  `OpDatabase` accepts the sentinel and allocates
        // fresh from it; `free` ignores it.
        let null_ref = if crate::keys::alloc_init_ref() {
            self.database.null()
        } else {
            DbRef::NULL
        };
        *self.database.store_mut(&self.stack_cur).addr_mut::<DbRef>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        ) = null_ref;
    }

    /// Plan-04 Phase B: positional variant of `null_ref_sentinel`.
    /// Writes `DbRef{store_nr: u16::MAX, rec: 0, pos: 0}` at the
    /// frame slot reached by `self.stack_pos - pos`.  Does NOT push
    /// onto the eval stack — used for first-init of inline-ref
    /// placeholders when the frame is pre-reserved, or combined with
    /// `OpReserveFrame(12)` to replicate `OpNullRefSentinel`'s
    /// push-and-init effect.
    ///
    /// `pos` is read from the bytecode stream as a `const u16`.
    pub fn init_ref_sentinel(&mut self) {
        let pos = self.code::<u16>();
        *self.database.store_mut(&self.stack_cur).addr_mut::<DbRef>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        ) = DbRef::NULL;
    }

    /// Plan-04 Phase B: positional variant of `create_stack`.
    /// Writes a stack-frame DbRef pointing into dep's slot
    /// (`stack_cur.pos + self.stack_pos - dep_pos`) at frame slot
    /// `self.stack_pos - pos`.  Does NOT push onto the eval stack.
    /// Used for first-init of borrowed-ref slots when the frame is
    /// pre-reserved, or combined with `OpReserveFrame(12)` to
    /// replicate `OpCreateStack`'s push-and-init effect.  The
    /// resulting DbRef must be overwritten by `OpPutRef` before any
    /// field access (same contract as `OpCreateStack`).
    ///
    /// Both `pos` and `dep_pos` are read from the bytecode stream
    /// as `const u16`.
    pub fn init_create_stack(&mut self) {
        let pos = self.code::<u16>();
        let dep_pos = self.code::<u16>();
        let db = DbRef {
            store_nr: self.stack_cur.store_nr,
            rec: self.stack_cur.rec,
            pos: self.stack_cur.pos + self.stack_pos - u32::from(dep_pos),
        };
        *self.database.store_mut(&self.stack_cur).addr_mut::<DbRef>(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos - u32::from(pos),
        ) = db;
    }

    pub fn put_stack<T: 'static>(&mut self, val: T) {
        #[cfg(debug_assertions)]
        {
            if std::any::TypeId::of::<T>() == std::any::TypeId::of::<DbRef>() {
                let db: &DbRef = unsafe { &*(&val as *const T as *const DbRef) };
                if !(db.store_nr == u16::MAX
                    || (db.store_nr as usize) < self.database.allocations.len())
                {
                    let (op_pc, op_code, fn_d_nr) = crate::crash_report::last_context();
                    panic!(
                        "put_stack<DbRef>: OOB store_nr={} (allocations.len()={}) \
                         rec={} pos={} code_pos={} — corrupt DbRef being pushed \
                         [last op: pc={} op_code={} fn_d_nr={}]",
                        db.store_nr,
                        self.database.allocations.len(),
                        db.rec,
                        db.pos,
                        self.code_pos,
                        op_pc,
                        op_code,
                        fn_d_nr,
                    );
                }
            }
        }
        self.ensure_stack(self.stack_step(size_of::<T>() as u32));
        #[cfg(feature = "stack_align_guard")]
        self.check_stack_align::<T>(self.stack_cur.pos + self.stack_pos);
        // @PLN154 phase 0 — tell the census this span arrived through the typed writer.
        // Absolute offset, the same `rec * 8 + fld` arithmetic `addr_mut` does below, so
        // the claim and the byte diff cannot be in different coordinate systems.
        if crate::stack_census::enabled() {
            crate::stack_census::claim(
                self.stack_cur.rec * 8 + self.stack_cur.pos + self.stack_pos,
                size_of::<T>() as u32,
            );
        }
        let m = self
            .database
            .store_mut(&self.stack_cur)
            .addr_mut::<T>(self.stack_cur.rec, self.stack_cur.pos + self.stack_pos);
        // LOFT_UAF_GEN (c): keep the offset's shadow stamp in sync with what is pushed
        // (BEFORE the move below). A DbRef push STAMPS its slot-gen so a later pop after a
        // free+reuse is caught; ANY non-DbRef push CLEARS the offset, so a stale stamp left
        // by an earlier DbRef cannot survive to a later unrelated pop — that staleness was
        // the detector's whole false-positive source (the "gen 0 at push" / huge-delta
        // residual). After both, the shadow holds a stamp only for a DbRef live right now.
        if crate::keys::uaf_gen_enabled() {
            if std::any::TypeId::of::<T>() == std::any::TypeId::of::<DbRef>() {
                let db: &DbRef = unsafe { &*(&raw const val).cast::<DbRef>() };
                if db.store_nr == u16::MAX {
                    crate::keys::uaf_clear_shadow(self.stack_pos);
                } else {
                    crate::keys::uaf_stamp_shadow(
                        self.stack_pos,
                        db.store_nr,
                        crate::keys::uaf_slot_gen(db.store_nr),
                    );
                    // Positive control (LOFT_UAF_GEN_INJECT): age EVERY ref just AFTER
                    // stamping it, so the stamp keeps the older gen and the ref is stale
                    // while live — exactly what a premature free does. Any checked pop must
                    // then report. Aging one ref is not enough: whether that particular ref
                    // is ever popped through this path depends on the program, so a silent
                    // run would still be ambiguous — which is the very thing the control
                    // exists to rule out.
                    if crate::keys::uaf_gen_inject_enabled() {
                        crate::keys::uaf_bump_gen(db.store_nr);
                    }
                }
            } else {
                crate::keys::uaf_clear_shadow(self.stack_pos);
            }
        }
        *m = val;
        self.stack_pos += self.stack_step(size_of::<T>() as u32);
        if self.stack_pos > self.stack_high {
            self.stack_high = self.stack_pos;
        }
    }

    /**
    Execute a function inside the `byte_code`.
    # Panics
    When too many steps were taken, this might indicate an unending loop.
    */
    pub fn execute(&mut self, name: &str, data: &Data) {
        self.execute_argv(name, data, &[]);
    }

    /// Plan-07 phase 1 step 1.20 / phase 3 — the source position of the construct
    /// that CONTAINS bytecode `pc`, or `None` when no recorded span does.
    ///
    /// Used by runtime fault printers (the `panic` builtin, div-by-zero, OOB,
    /// null deref) to surface `at file:line:col` beside the bytecode-level
    /// context.  A pc inside a wrapped construct still resolves — that is the
    /// inheritance these printers rely on, since a fault lands on some op within
    /// the statement, not on its first.  A pc no span covers now answers `None`
    /// rather than the nearest preceding entry, which belonged to whatever
    /// unrelated statement happened to be wrapped last (loft#1262).
    ///
    /// The walk is backwards because spans nest: the first covering entry found
    /// is the innermost, which is the most precise answer.  It cannot stop early
    /// on a miss — an outer span may still cover a pc the inner ones do not — so
    /// an uncovered pc costs a scan of the preceding entries.  That is a
    /// fault-reporting path, taken once.
    #[must_use]
    pub fn source_loc_for(&self, pc: u32) -> Option<&Position> {
        self.source_spans
            .range(..=pc)
            .rev()
            .find(|(_, (_, end))| *end > pc)
            .map(|(_, (p, _))| p)
    }

    /// Hand the fault-site span table to the crash hook, so a Rust panic inside
    /// any opcode dispatch can print `at file:line:col` for the offending pc.
    ///
    /// The snapshot is built on first use and then shared, because this runs on
    /// every ENTRY into loft — including each `loft::host` call and each call to
    /// a process-placed library. See [`State::published_spans`].
    pub(crate) fn publish_source_spans(&mut self) {
        if self.published_spans.is_none() {
            self.published_spans = Some(Arc::new(self.source_spans.clone()));
        }
        crate::crash_report::set_source_spans(self.published_spans.clone());
    }

    // ── @PLN16 debugger ──────────────────────────────────────────────────────

    /// Turn on debugging (idempotent).  The execute loop then consults the
    /// [`Debugger`](crate::debugger::Debugger) at each op.
    pub fn enable_debug(&mut self) {
        if self.debug.is_none() {
            self.debug = Some(Box::default());
        }
    }

    /// Register a breakpoint at the entry of function `d_nr` (its first bytecode
    /// op).  Enables debugging if not already on.  Returns `false` if `d_nr` is
    /// out of range.  Reads the entry offset from `def.code_position` (set during
    /// codegen) — `State::fn_positions` is not populated until `execute_argv`
    /// runs, so a breakpoint set before the run must consult `data` directly.
    pub fn set_breakpoint_fn_entry(&mut self, d_nr: u32, data: &crate::data::Data) -> bool {
        if d_nr >= data.definitions() {
            return false;
        }
        let offset = data.def(d_nr).code_position;
        self.enable_debug();
        if let Some(dbg) = self.debug.as_mut() {
            dbg.add_offset(offset);
        }
        true
    }

    /// Register a breakpoint at source `line` **within function `d_nr`** — the
    /// correct primitive: a bare line number matches that line in *every* function
    /// (stdlib included), so a breakpoint must be scoped to its function.  Scans the
    /// dense per-line table `line_numbers` within `[d_nr.code_position, end)` for the
    /// first op mapped to `line`.  Returns `false` if no op in that range is on
    /// `line`.  (Uses `line_numbers`, not `source_spans`: the latter is emitted only
    /// at fault-prone arithmetic, so a line with no such op — a pure `if`, a call, a
    /// bare assignment — would otherwise be unbreakable.)
    pub fn set_breakpoint_fn_line(
        &mut self,
        d_nr: u32,
        line: u32,
        data: &crate::data::Data,
    ) -> Option<u32> {
        if d_nr >= data.definitions() {
            return None;
        }
        let start = data.def(d_nr).code_position;
        let end = start + data.def(d_nr).code_length;
        let &offset = self
            .line_numbers
            .range(start..end)
            .find(|(_, l)| **l == line)
            .map(|(off, _)| off)?;
        self.enable_debug();
        if let Some(dbg) = self.debug.as_mut() {
            dbg.add_offset(offset);
        }
        Some(offset)
    }

    /// Register a breakpoint at the **start of a named function's body** — the
    /// human-friendly form (`:break foo`).  Resolves `name` → its def → the *first*
    /// per-line offset inside its bytecode (`[code_position, +code_length)`), which
    /// is the body's first statement, **post-prologue** (args are in their slots —
    /// unlike `set_breakpoint_fn_entry`, which pauses pre-prologue where the frame
    /// isn't set up; `line_numbers` entries land after any frame-setup op).  Returns
    /// `false` if `name` isn't a defined function or its body has no line mapping.
    pub fn set_breakpoint_fn_start(&mut self, name: &str, data: &crate::data::Data) -> Option<u32> {
        let d_nr = data.def_nr(&format!("n_{name}"));
        if d_nr >= data.definitions() {
            return None;
        }
        let def = data.def(d_nr);
        let (start, end) = (def.code_position, def.code_position + def.code_length);
        let &offset = self
            .line_numbers
            .range(start..end)
            .next()
            .map(|(off, _)| off)?;
        self.enable_debug();
        if let Some(dbg) = self.debug.as_mut() {
            dbg.add_offset(offset);
        }
        Some(offset)
    }

    /// @PLN16 M5a — register a breakpoint at **`file:line`** for the file-run debugger.
    /// Unlike the REPL's function-scoped form, a real source file gives line numbers
    /// that are unique within it, so the user names a line directly.  Scoped to the
    /// **user file's** function defs (matched by `position.file`'s basename — stdlib
    /// lives in other files, so its identical line numbers are excluded), it sets the
    /// breakpoint in the one whose body has emitted code on `line` (reusing
    /// [`set_breakpoint_fn_line`](Self::set_breakpoint_fn_line)).  Returns `false` when
    /// no user-file function has a breakable op on `line`.
    pub fn set_breakpoint_file_line(
        &mut self,
        file: &str,
        line: u32,
        data: &crate::data::Data,
    ) -> Option<u32> {
        let want = std::path::Path::new(file).file_name()?;
        for d in 0..data.definitions() {
            let def = data.def(d);
            if def.def_type != crate::data::DefType::Function {
                continue;
            }
            if std::path::Path::new(&def.position.file).file_name() != Some(want) {
                continue;
            }
            if let Some(off) = self.set_breakpoint_fn_line(d, line, data) {
                return Some(off);
            }
        }
        None
    }

    /// @PLN16 M5a — the breakable source lines in `file` (its basename), sorted: the
    /// lines a `file:line` breakpoint can land on.  Powers the "no breakable op on line
    /// N — try one of these" hint in the file-run debugger.  Drawn from the dense
    /// `line_numbers` table, scoped to the user file's function defs.
    #[must_use]
    pub fn breakable_lines_in_file(&self, file: &str, data: &crate::data::Data) -> Vec<u32> {
        let Some(want) = std::path::Path::new(file).file_name() else {
            return Vec::new();
        };
        let mut ls: Vec<u32> = Vec::new();
        for d in 0..data.definitions() {
            let def = data.def(d);
            if def.def_type != crate::data::DefType::Function
                || std::path::Path::new(&def.position.file).file_name() != Some(want)
            {
                continue;
            }
            let (start, end) = (def.code_position, def.code_position + def.code_length);
            ls.extend(self.line_numbers.range(start..end).map(|(_, l)| *l));
        }
        ls.sort_unstable();
        ls.dedup();
        ls
    }

    /// The distinct source lines that carry a bytecode mapping — the lines a
    /// breakpoint can pause on, sorted.  Drawn from the dense `line_numbers` table
    /// (every line with emitted code), not the sparse arithmetic-only `source_spans`.
    #[must_use]
    pub fn breakable_lines(&self) -> Vec<u32> {
        let mut ls: Vec<u32> = self.line_numbers.values().copied().collect();
        ls.sort_unstable();
        ls.dedup();
        ls
    }

    /// The source line of bytecode `pc` from the dense per-line table — the nearest
    /// `line_numbers` entry at or before `pc` (entries land on each line's first op).
    /// `0` when no mapping precedes `pc`.  The debugger's line granularity for
    /// stepping; distinct from [`source_loc_for`](Self::source_loc_for), which gives
    /// a full `Position` (line+col) from the sparse fault-site `source_spans`.
    #[must_use]
    pub fn line_at(&self, pc: u32) -> u32 {
        self.line_numbers
            .range(..=pc)
            .next_back()
            .map_or(0, |(_, &l)| l)
    }

    /// The frames captured at breakpoint hits so far (empty when not debugging).
    #[must_use]
    pub fn debug_hits(&self) -> &[crate::debugger::BreakHit] {
        self.debug.as_deref().map_or(&[], |d| d.hits.as_slice())
    }

    /// Turn on **stepping mode** (idempotent; enables debugging): a breakpoint
    /// *suspends* execution — the run returns with the frame in
    /// [`paused_frame`](Self::paused_frame) instead of recording-and-continuing.
    /// Edit a value with [`set_frame_value`](Self::set_frame_value), then continue
    /// with [`resume`](Self::resume).
    pub fn enable_stepping(&mut self) {
        self.enable_debug();
        if let Some(d) = self.debug.as_mut() {
            d.stepping = true;
        }
    }

    /// Whether execution is currently suspended at a breakpoint (stepping mode).
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.debug.as_deref().is_some_and(|d| d.paused.is_some())
    }

    /// The frame captured at the current suspension, or `None` if not paused.
    #[must_use]
    pub fn paused_frame(&self) -> Option<&crate::debugger::BreakHit> {
        self.debug.as_deref().and_then(|d| d.paused.as_ref())
    }

    /// The source line the current suspension is stopped **on**, or `None` if not paused
    /// (or the line is unknown).  `code_pos` is the op about to execute, so `line_at` gives
    /// the line the debugger is parked on.  Unlike
    /// [`paused_at_breakpoint`](Self::paused_at_breakpoint) this is set for a **step** pause
    /// too — it doesn't require a registered breakpoint at the stop — which is what lets the
    /// browser debugger move its current-line marker as you step.
    #[must_use]
    pub fn paused_line(&self) -> Option<u32> {
        if !self.is_paused() {
            return None;
        }
        let line = self.line_at(self.code_pos);
        (line != 0).then_some(line)
    }

    /// @PLN16 rich-bp — the bytecode offset of the current pause **iff** it is at a
    /// registered breakpoint (so the driver can look up its condition / tracepoint),
    /// else `None` (a step or watch pause is always a real stop).  The pause pc is
    /// `code_pos`: the suspend hook returns *before* executing the breakpoint op.
    #[must_use]
    pub fn paused_at_breakpoint(&self) -> Option<u32> {
        if !self.is_paused() {
            return None;
        }
        let pc = self.code_pos;
        self.debug
            .as_deref()
            .filter(|d| d.is_breakpoint(pc))
            .map(|_| pc)
    }

    /// Resolve frame local `name` to `(record, frame-absolute address, type,
    /// is_argument)` in the current suspension's frame — the shared slot lookup
    /// behind both the frame-value reads and writes.  The address is the variable's
    /// fixed slot (`stack_cur.pos + args_base + vars.stack(i)`), the same one
    /// [`render_frame_local`](Self::render_frame_local) reads.  `is_argument`
    /// distinguishes a text **arg** (a 16-byte `Str` borrow) from a text **local**
    /// (a 24-byte owned `String`), which the edit path writes differently.  `None`
    /// if there is no current frame or no local of that name.
    fn frame_slot(
        &self,
        name: &str,
        data: &crate::data::Data,
    ) -> Option<(u32, u32, crate::data::Type, bool)> {
        let frame = self.call_stack.last()?;
        if frame.d_nr == u32::MAX {
            return None;
        }
        let vars = &data.def(frame.d_nr).variables;
        let i = (0..vars.count()).find(|&i| vars.name(i) == name)?;
        let at = self.stack_cur.pos + frame.args_base + u32::from(vars.stack(i));
        Some((
            self.stack_cur.rec,
            at,
            vars.tp(i).clone(),
            vars.is_argument(i),
        ))
    }

    /// @PLN14 arc D — the live frame's slot for local `name`, as a [`DbRef`]
    /// addressing it inside the stack store, plus its declared type and whether it
    /// is an argument.  `None` when there is no current frame or no such local.
    ///
    /// This is the write end of the **frame-seed**: a store-resident binding is
    /// loaded into its slot through this address, after which the ordinary
    /// slot-based codegen runs untouched (Q1 — seeding needs no new opcodes).  It
    /// is the same slot [`set_frame_literal`](Self::set_frame_literal) edits, but
    /// exposed as an address so a **heap** value can be seeded too: that path has
    /// a real `DbRef` to install (materialized from the session store) rather than
    /// a literal to reconstruct, which is precisely what `set_frame_literal`
    /// cannot do.
    #[must_use]
    pub fn frame_slot_addr(
        &self,
        name: &str,
        data: &crate::data::Data,
    ) -> Option<(DbRef, crate::data::Type, bool)> {
        let (rec, at, tp, is_arg) = self.frame_slot(name, data)?;
        Some((
            DbRef {
                store_nr: self.stack_cur.store_nr,
                rec,
                pos: at,
            },
            tp,
            is_arg,
        ))
    }

    /// Whether the frame **holds** `name`'s own value at the current suspension —
    /// the gate on every read or write of a frame slot by name.
    ///
    /// @PLN120 A: this asks [`frame_view`](Self::frame_view) rather than testing for
    /// the name's *presence* in the captured frame, because the captured frame now
    /// also lists locals it does not hold (`<unset>`, `<reused by …>`).  Presence
    /// would let a heap read index a garbage `DbRef`, and would let a text edit
    /// `Drop` a `String` that was never constructed.
    fn frame_local_is_live(&self, name: &str, data: &crate::data::Data) -> bool {
        self.frame_local_state(name, data)
            .is_some_and(|st| st.is_held())
    }

    /// Whether a frame local's declared type is a **heap** value — a `DbRef` slot
    /// (struct / vector / struct-enum / collection), as opposed to an inline scalar,
    /// text, or simple enum.
    fn is_heap_type(tp: &crate::data::Type) -> bool {
        crate::data::is_dbref(tp)
    }

    /// @PLN16 M1a — if frame local `name` holds a **heap** value (a `DbRef` slot:
    /// struct / vector / struct-enum / collection), its loft-source type name — so a
    /// constructor of the same type can be built and grafted in.  `None` for an inline
    /// scalar / text / simple-enum local (edited in place by
    /// [`set_frame_literal`](Self::set_frame_literal)) or an unknown local.  Routes the
    /// debugger's whole-value heap edit.
    #[must_use]
    pub fn frame_heap_type(&self, name: &str, data: &crate::data::Data) -> Option<String> {
        let (_, _, tp, _) = self.frame_slot(name, data)?;
        Self::is_heap_type(&tp).then(|| tp.name(data))
    }

    /// @PLN16 D2 — **live-frame read** of a bare heap local: render frame local `name`
    /// straight from its **live `DbRef`** in the paused store — own-format
    /// (`json=false`) or RFC-8259 JSON (`json=true`).  This is the faithful eval path:
    /// no reconstruct, no clone, no fn-return deep-copy — `show_json` / `show_loft` read
    /// the value where it lives, so a `vector` renders correctly (the reconstruct-eval
    /// path faults returning one from a cloned state) **and** the read shows what is
    /// *actually* in the store, never a copy of it — load-bearing for a consumer
    /// hunting store-lifetime bugs where a *copy* can drop/desync a field.  `None` for
    /// a non-heap or unknown local, so the caller falls through to the reconstruct path
    /// for scalars / computed expressions.  Mirrors [`render_frame_local`]'s heap arm.
    #[must_use]
    pub fn eval_frame_heap(
        &self,
        name: &str,
        json: bool,
        data: &crate::data::Data,
    ) -> Option<String> {
        // Only read a local that is **live** at the pause (shown in the captured frame):
        // an un-live heap slot holds stack garbage, and reading it as a `DbRef` would
        // index a garbage store (the very OOB this read exists to avoid). The D0
        // variables panel gates the same way; an un-live name falls through to the
        // reconstruct path (which likewise lacks it → a clean `None`, never a crash).
        if !self.frame_local_is_live(name, data) {
            return None;
        }
        let (rec, at, tp, _is_arg) = self.frame_slot(name, data)?;
        if !Self::is_heap_type(&tp) {
            return None;
        }
        let tp_known = self.database.name(&tp.name(data));
        if tp_known == u16::MAX {
            return None;
        }
        let db = *self
            .database
            .store(&self.stack_cur)
            .addr::<crate::keys::DbRef>(rec, at);
        let mut out = String::new();
        if json {
            self.database.show_json(&mut out, &db, tp_known, false);
        } else {
            self.database.show_loft(&mut out, &db, tp_known);
        }
        Some(out)
    }

    /// @PLN98 P1b — if frame local `name` is a live **keyed collection**
    /// (`hash` / `sorted` / `index`), its PARSEABLE loft-source type — e.g.
    /// `hash<Ent[k]>`, `index<Rec[nr, -key]>`.  Unlike [`Type::name`](crate::data::Type::name)
    /// (which Debug-renders the key spec as `["k"]`, not loft source), this
    /// round-trips through the parser, so the synthetic live-frame eval fn can
    /// declare the local as a typed argument and receive its live `DbRef`.
    /// `None` for a non-keyed / unknown / un-live local.
    #[must_use]
    pub fn frame_keyed_type_source(&self, name: &str, data: &crate::data::Data) -> Option<String> {
        if !self.frame_local_is_live(name, data) {
            return None;
        }
        let (_, _, tp, _) = self.frame_slot(name, data)?;
        keyed_type_source(&tp, data)
    }

    /// @PLN98 P3.4 — a live frame local's PARSEABLE loft type, for binding it as an
    /// argument of a synthetic eval fn (the browser client's full-expression eval).
    /// Keyed collections use [`keyed_type_source`] (their `Type::name` isn't loft
    /// source); everything else uses `Type::name` (integer / float / boolean /
    /// character / `vector<T>` / a struct or enum name — all reparseable).  `None`
    /// for a `text` local (a borrowed `Str` arg is @P293-unsafe to push) or an
    /// un-live / unknown local — the caller drops the expression to a graceful
    /// fallback rather than mis-push it.
    #[must_use]
    pub fn frame_local_arg_type(&self, name: &str, data: &crate::data::Data) -> Option<String> {
        if !self.frame_local_is_live(name, data) {
            return None;
        }
        let (_, _, tp, _) = self.frame_slot(name, data)?;
        if matches!(tp, crate::data::Type::Text(_)) {
            return None;
        }
        Some(keyed_type_source(&tp, data).unwrap_or_else(|| tp.name(data)))
    }

    /// @PLN98 P1b — the true live-frame eval: run the already-compiled synthetic
    /// fn `eval_dnr` (built as `fn __eval(k1: K1, …) -> RT { … expr }`) over THIS
    /// paused State, with its keyed-collection arguments `arg_names` bound to the
    /// paused frame's **live** locals.  This is the invariant-honouring form the
    /// text-reconstruct path can't reach: a referenced `hash`/`sorted`/`index`
    /// renders non-reparseable, so instead of seeding a literal we pass the live
    /// `DbRef` straight into the eval and read the collection where it lives.
    ///
    /// The fn's bytecode is appended **append-only** to the running stream (like
    /// [`live_reload`](crate::live_reload)): the paused frame's live PC and stack
    /// slots are untouched, so the run resumes correctly after.  `reenter_ret`
    /// allocates the eval's frame above the high-water mark, pushes the pre-read
    /// arg `DbRef`s, runs to completion, and restores the watermark.  Returns the
    /// rendered value (`json` = RFC-8259 vs own-format), or `None` for an
    /// unsupported return type.
    pub fn eval_frame_reenter(
        &mut self,
        data: &mut crate::data::Data,
        eval_dnr: u32,
        arg_names: &[String],
        ret: &crate::data::Type,
        json: bool,
    ) -> Option<String> {
        use crate::data::Type;
        // 1. Read each arg's live value from the paused frame BEFORE reentering —
        //    `reenter_ret` pushes a fresh frame, so `frame_slot` (which reads
        //    `call_stack.last()`) would then see the eval's frame.  Each is read at
        //    its storage width (heap → `DbRef`, else the inline scalar) so it can be
        //    pushed back as the right argument type — the browser client binds ANY
        //    referenced frame local as an arg, not just keyed collections.
        let mut args: Vec<FrameArg> = Vec::with_capacity(arg_names.len());
        for name in arg_names {
            let (rec, at, tp, _is_arg) = self.frame_slot(name, data)?;
            let store = self.database.store(&self.stack_cur);
            let val = match &tp {
                Type::Reference(_, _)
                | Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Index(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Radix(_, _, _)
                | Type::Trie(_, _, _)
                | Type::Enum(_, true, _) => {
                    FrameArg::Ref(*store.addr::<crate::keys::DbRef>(rec, at))
                }
                Type::Float => FrameArg::F64(*store.addr::<f64>(rec, at)),
                Type::Single => FrameArg::F32(*store.addr::<f32>(rec, at)),
                Type::Boolean | Type::Enum(_, false, _) => FrameArg::U8(*store.addr::<u8>(rec, at)),
                Type::Character => FrameArg::U32(*store.addr::<u32>(rec, at)),
                // Integer (and anything else that fits) rides an `i64` slot.
                _ => FrameArg::I64(*store.addr::<i64>(rec, at)),
            };
            args.push(val);
        }
        // 2. Sync def positions from the live dispatch table into `data` so a Call
        //    the eval body emits (a stdlib/user fn in `expr`) targets the LIVE body
        //    (mirrors `live_reload::reload_fn`; without it a call jumps to 0).
        let pre = (self.fn_positions.len() as u32).min(data.definitions());
        for d in 0..pre {
            data.definitions[d as usize].code_position = self.fn_positions[d as usize];
        }
        // 3. Append the eval fn's bytecode at the end of the running stream. The
        //    live PC is saved/restored; the append never moves an existing offset.
        let saved_pc = self.code_pos;
        self.code_pos = self.bytecode.len() as u32;
        self.database.allocations[crate::database::CONST_STORE as usize].unlock();
        self.def_code(eval_dnr, data, None);
        self.database.allocations[crate::database::CONST_STORE as usize]
            .lock_with_origin("eval_frame_reenter (CONST_STORE relock)");
        self.code_pos = saved_pc;
        while (self.fn_positions.len() as u32) <= eval_dnr {
            let d = self.fn_positions.len() as u32;
            self.fn_positions.push(data.def(d).code_position);
        }
        let pos = data.def(eval_dnr).code_position;
        // 4. Reenter over the paused world, pushing the live args in declared
        //    order (each as its own type), and render the return by its type.
        let push = |st: &mut State| {
            for a in &args {
                match a {
                    FrameArg::Ref(v) => st.put_stack(*v),
                    FrameArg::I64(v) => st.put_stack(*v),
                    FrameArg::F64(v) => st.put_stack(*v),
                    FrameArg::F32(v) => st.put_stack(*v),
                    FrameArg::U8(v) => st.put_stack(*v),
                    FrameArg::U32(v) => st.put_stack(*v),
                }
            }
        };
        // `reenter_ret` restores `code_pos`/`stack_pos` but NOT the call stack or
        // high-water mark.  On a *parked* State (live_dispatch) that is fine — the
        // call stack starts empty.  Here the State is PAUSED mid-run with `main`
        // (and its callers) live on the call stack, and the eval callee's return
        // pops one frame that `reenter_ret` doesn't push back, so it would strand
        // the paused frame (`frame_slot` reads `call_stack.last()`).  Snapshot and
        // restore so the eval is fully transparent to the paused run.
        let saved_stack = self.call_stack.clone();
        let saved_high = self.stack_high;
        let rendered = match ret {
            Type::Integer(_) => Some(self.reenter_ret::<i64>(eval_dnr, pos, push).to_string()),
            Type::Float => Some(loft_float_literal(
                &self.reenter_ret::<f64>(eval_dnr, pos, push).to_string(),
            )),
            Type::Single => {
                let v = self.reenter_ret::<f32>(eval_dnr, pos, push);
                Some(if json { v.to_string() } else { format!("{v}f") })
            }
            Type::Boolean => {
                let v = self.reenter_ret::<u8>(eval_dnr, pos, push) != 0;
                Some(if v { "true" } else { "false" }.to_string())
            }
            Type::Character => {
                char::from_u32(self.reenter_ret::<u32>(eval_dnr, pos, push)).map(|c| {
                    if json {
                        format!("\"{c}\"")
                    } else {
                        format!("'{c}'")
                    }
                })
            }
            // A text return rides the frame base as a 16-byte `Str` (like a scalar,
            // unlike a heap `DbRef`).  Safe ONLY for a **call-returned-owned** text
            // — a `.to_json()` result — whose buffer moves out as the return value
            // and so survives the frame teardown.  A borrowed-local / work text (a
            // bare var, a `+` concat, an interpolation) is freed on teardown and
            // would be @P293-UAF here, so the caller sends ONLY `.to_json()` results
            // down this path.  Returned RAW (the already-serialised JSON string).
            Type::Text(_) => Some(
                self.reenter_ret::<crate::keys::Str>(eval_dnr, pos, push)
                    .str()
                    .to_string(),
            ),
            // A heap value (struct / vector / struct-enum) is destination-passed,
            // NOT copied to the frame base, so `reenter_ret` can't retrieve it (it
            // would read back the first pushed arg).  The caller serialises such a
            // result in-fn — boxed in a one-field record, so a `vector` and a `text` ride the
            // same `.to_json()` the struct case always did (loft#1187) — and routes it through
            // the `Text` arm instead.  So this path is a deliberate `None`, never a wrong read.
            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => None,
            Type::Enum(_, false, _) => {
                let schema = self.database.name(&ret.name(data));
                if schema == u16::MAX {
                    None
                } else {
                    let disc = self.reenter_ret::<u8>(eval_dnr, pos, push);
                    if disc == 0 {
                        Some("null".to_string())
                    } else {
                        let name = ret.name(data);
                        let v = self.database.enum_val(schema, disc);
                        Some(if json {
                            format!("\"{name}.{v}\"")
                        } else {
                            format!("{name}.{v}")
                        })
                    }
                }
            }
            _ => None,
        };
        // Restore the paused run's call stack + watermark (see the snapshot above).
        self.call_stack = saved_stack;
        self.stack_high = saved_high;
        rendered
    }

    /// @PLN16 M1a — point a **heap** frame local at an already-materialised value by
    /// writing the root `DbRef` into the live frame slot.  The value must already live
    /// in this `State`'s stores (built + grafted by the debugger's whole-value edit —
    /// [`Stores::adopt_value_stores`](crate::database::Stores::adopt_value_stores)).
    /// Returns `false` for an unknown or non-heap (inline) local.  The prior value's
    /// stores are left allocated — a debug-session-only leak, like the text-local edit;
    /// **undo (M2)** restores the old `DbRef` (a journaled slot `Modify`), so it points
    /// back at the still-allocated original (the grafted value's stores leak symmetrically).
    pub fn set_frame_dbref(
        &mut self,
        name: &str,
        db: crate::keys::DbRef,
        data: &crate::data::Data,
    ) -> bool {
        let Some((rec, at, tp, _is_arg)) = self.frame_slot(name, data) else {
            return false;
        };
        if !Self::is_heap_type(&tp) {
            return false;
        }
        let store_nr = self.stack_cur.store_nr;
        let len = std::mem::size_of::<crate::keys::DbRef>() as u32;
        let before = self.edit_before(store_nr, rec, at, len);
        *self
            .database
            .store_mut(&self.stack_cur)
            .addr_mut::<crate::keys::DbRef>(rec, at) = db;
        self.edit_after(store_nr, rec, at, before);
        true
    }

    /// @PLN16 M2 — whether an interactive edit's undo journal is armed (recording the
    /// before/after bytes of the regions it overwrites).  False on non-interactive
    /// writes, so they pay nothing.
    fn edit_recording(&self) -> bool {
        self.debug
            .as_deref()
            .is_some_and(|d| d.recording_edit.is_some())
    }

    /// @PLN16 M2 — snapshot `len` bytes of a frame/heap region *before* an edit
    /// overwrites them — the `before` half of a journaled `Modify`.  `None` (no
    /// snapshot, no cost) when no edit journal is armed.  Pair with
    /// [`edit_after`](Self::edit_after) once the write has landed.
    fn edit_before(&self, store_nr: u16, rec: u32, off: u32, len: u32) -> Option<Box<[u8]>> {
        self.edit_recording()
            .then(|| crate::database::Journal::snapshot(&self.database, store_nr, rec, off, len))
    }

    /// @PLN16 M2 — after an edit's write lands, record the `Modify` (the `before` from
    /// [`edit_before`](Self::edit_before) → the region's current bytes) into the armed
    /// journal, so the edit can be reverted (`:undo`) and replayed (`:redo`).  No-op
    /// when `before` is `None` (not recording).
    fn edit_after(&mut self, store_nr: u16, rec: u32, off: u32, before: Option<Box<[u8]>>) {
        let Some(before) = before else {
            return;
        };
        // `self.debug` and `self.database` are distinct fields → disjoint borrows.
        if let Some(j) = self
            .debug
            .as_deref_mut()
            .and_then(|d| d.recording_edit.as_mut())
        {
            let _ = j.record_modify(&self.database, store_nr, rec, off, &before);
        }
    }

    /// @PLN16 M2 — arm a fresh per-edit journal so the next frame write records its
    /// before/after bytes for undo.  No-op when not debugging.  A blob-file failure
    /// leaves it disarmed (the edit still runs, just without an undo entry).
    pub fn begin_edit_journal(&mut self) {
        if let Some(d) = self.debug.as_deref_mut() {
            d.recording_edit = crate::database::Journal::create().ok();
        }
    }

    /// @PLN16 M2 — finish the armed edit journal: if it recorded anything, push it onto
    /// the undo stack and clear the redo stack (a fresh edit forks the timeline);
    /// otherwise discard it.  Returns whether an undoable edit was recorded.
    ///
    /// @PLN120 F — `label` is the edit's LHS as typed, and the entry is **bound** to
    /// what it wrote: which frame, and which locals' slots.  That binding is what lets
    /// the entry outlive a step (see [`validate_undo_history`](Self::validate_undo_history)).
    pub fn commit_edit_journal(&mut self, label: &str, data: &crate::data::Data) -> bool {
        // Classify the recorded regions BEFORE moving the journal into the entry:
        // anything in the stack store is frame storage, anything else is heap.
        let (frame, slots) = {
            let Some(j) = self
                .debug
                .as_deref()
                .and_then(|d| d.recording_edit.as_ref())
            else {
                return false;
            };
            if j.is_empty() {
                self.discard_edit_journal();
                return false;
            }
            self.classify_edit_regions(&j.regions(), data)
        };
        let Some(d) = self.debug.as_deref_mut() else {
            return false;
        };
        let Some(journal) = d.recording_edit.take() else {
            return false;
        };
        d.undo_stack.push(crate::debugger::UndoEntry {
            journal,
            label: label.to_string(),
            frame,
            slots,
        });
        d.redo_stack.clear();
        true
    }

    /// @PLN120 F — which frame (if any) and which locals' slots a set of journal
    /// regions wrote.  Classified by **store**, not by the edit's syntax: a bare-name
    /// edit of a heap local grafts a value into the heap *and* writes the slot's
    /// `DbRef`, so reading the LHS shape would mis-file it.
    fn classify_edit_regions(
        &self,
        regions: &[(u16, u32, u32, u32)],
        data: &crate::data::Data,
    ) -> (Option<crate::debugger::StackWatchFrame>, Vec<(String, u16)>) {
        let Some(cf) = self.call_stack.last() else {
            return (None, Vec::new());
        };
        let base = self.stack_cur.pos + cf.args_base;
        let mut slots: Vec<(String, u16)> = Vec::new();
        let mut touched_frame = false;
        // The frame's locals at the pause, so a written offset can be attributed to the
        // local that owns it (name included — a slot alone is ambiguous between two
        // locals that share it at different lines).
        let view = self.frame_view(cf.d_nr, self.code_pos, data);
        for &(store_nr, rec, off, _len) in regions {
            if store_nr != self.stack_cur.store_nr || rec != self.stack_cur.rec || off < base {
                continue; // heap (or another store) — no frame binding needed
            }
            touched_frame = true;
            let rel = u16::try_from(off - base).unwrap_or(u16::MAX);
            for e in &view {
                let size = crate::variables::size(
                    &e.tp,
                    if e.is_argument {
                        &Context::Argument
                    } else {
                        &Context::Variable
                    },
                );
                if rel >= e.slot && u32::from(rel) < u32::from(e.slot) + u32::from(size) {
                    if !slots.iter().any(|(n, s)| n == &e.name && *s == e.slot) {
                        slots.push((e.name.clone(), e.slot));
                    }
                    break;
                }
            }
        }
        let frame = touched_frame.then(|| crate::debugger::StackWatchFrame {
            d_nr: cf.d_nr,
            args_base: cf.args_base,
            depth: u32::try_from(self.call_stack.len().saturating_sub(1)).unwrap_or(u32::MAX),
        });
        (frame, slots)
    }

    /// @PLN120 F — keep the undo/redo entries whose storage is still what they wrote,
    /// drop the rest, and record WHY each was dropped in `dropped_undo`.
    ///
    /// Called at every new pause. An entry survives iff it is heap-only (no frame
    /// binding — a heap address survives any step), or its frame is still live at the
    /// same depth with the same `(d_nr, args_base)` **and** every local it wrote is
    /// still `Held` at the same slot here. Anything else would write bytes that are no
    /// longer that local's, which is the hazard the old blanket clear avoided by
    /// throwing away the valid entries too.
    pub fn validate_undo_history(&mut self, data: &crate::data::Data) {
        if self
            .debug
            .as_deref()
            .is_none_or(|d| d.undo_stack.is_empty() && d.redo_stack.is_empty())
        {
            return;
        }
        // The live frame's identity + view, computed once for every entry.
        let live = self.call_stack.last().map(|cf| {
            (
                crate::debugger::StackWatchFrame {
                    d_nr: cf.d_nr,
                    args_base: cf.args_base,
                    depth: u32::try_from(self.call_stack.len().saturating_sub(1))
                        .unwrap_or(u32::MAX),
                },
                self.frame_view(cf.d_nr, self.code_pos, data),
            )
        });
        let verdict = |e: &crate::debugger::UndoEntry| -> Result<(), String> {
            let Some(bound) = e.frame.as_ref() else {
                return Ok(()); // heap-only — no frame change can invalidate it
            };
            let Some((now, view)) = live.as_ref() else {
                return Err("the frame it was made in has returned".to_string());
            };
            if now != bound {
                return Err("the frame it was made in has returned".to_string());
            }
            for (name, slot) in &e.slots {
                match view
                    .iter()
                    .find(|v| &v.name == name && v.slot == *slot)
                    .map(|v| &v.state)
                {
                    Some(LocalState::Held) => {}
                    Some(LocalState::Reused(by)) => {
                        return Err(format!("`{name}`'s stack slot is now `{by}`'s"));
                    }
                    _ => {
                        return Err(format!("`{name}` no longer holds that slot here"));
                    }
                }
            }
            Ok(())
        };
        let mut dropped: Vec<(String, String)> = Vec::new();
        {
            let Some(d) = self.debug.as_deref_mut() else {
                return;
            };
            for stack in [&mut d.undo_stack, &mut d.redo_stack] {
                stack.retain(|e| match verdict(e) {
                    Ok(()) => true,
                    Err(why) => {
                        dropped.push((e.label.clone(), why));
                        false
                    }
                });
            }
        }
        if let Some(d) = self.debug.as_deref_mut() {
            d.dropped_undo = dropped;
        }
    }

    /// @PLN120 F — the entries THIS pause dropped, as `(label, reason)`.  Empty when
    /// nothing was dropped.
    ///
    /// Deliberately non-consuming: the list is cleared when the next resume starts, so
    /// reading it is still once-per-pause, and it stays available for the whole pause —
    /// which is what lets `:undo` explain an empty stack by naming the edit that was
    /// dropped instead of falling back to "you made no edits".
    #[must_use]
    pub fn dropped_undo(&self) -> Vec<(String, String)> {
        self.debug
            .as_deref()
            .map(|d| d.dropped_undo.clone())
            .unwrap_or_default()
    }

    /// @PLN16 M2 — drop the armed edit journal without recording (a failed edit).
    pub fn discard_edit_journal(&mut self) {
        if let Some(d) = self.debug.as_deref_mut() {
            d.recording_edit = None;
        }
    }

    /// @PLN16 M2 — undo the last edit at this suspension: revert its journal and move it
    /// to the redo stack.  Returns `false` when the undo stack is empty.  Refresh the
    /// paused frame after, so `:vars` shows the restored value.
    pub fn debug_undo(&mut self) -> bool {
        let Some(mut e) = self.debug.as_deref_mut().and_then(|d| d.undo_stack.pop()) else {
            return false;
        };
        let ok = e.journal.revert(&mut self.database).is_ok();
        if let Some(d) = self.debug.as_deref_mut() {
            d.redo_stack.push(e);
        }
        ok
    }

    /// @PLN16 M2 — redo the last undone edit: re-apply its journal and move it back to
    /// the undo stack.  Returns `false` when the redo stack is empty.
    pub fn debug_redo(&mut self) -> bool {
        let Some(mut e) = self.debug.as_deref_mut().and_then(|d| d.redo_stack.pop()) else {
            return false;
        };
        let ok = e.journal.apply(&mut self.database).is_ok();
        if let Some(d) = self.debug.as_deref_mut() {
            d.undo_stack.push(e);
        }
        ok
    }

    /// Write an integer `value` into the **live** frame's local `name` at a
    /// suspension — the @PLN16 F write-back, so a value edited at the breakpoint is
    /// picked up when execution `resume`s.  Returns `false` if no integer local of
    /// that name is in the current frame.  (The typed-`i64` primitive; the REPL
    /// edits via [`set_frame_literal`](Self::set_frame_literal), which covers every
    /// inline scalar.)
    pub fn set_frame_value(&mut self, name: &str, value: i64, data: &crate::data::Data) -> bool {
        // @PLN120 A — a local the frame does not hold must not be written either: its
        // slot belongs to another local (or to nothing yet), so the write would land
        // in someone else's value.  Latent before A, because such a local was not
        // shown and so nobody named it; A shows it, which makes the edit likely.
        if !self.frame_local_is_live(name, data) {
            return false;
        }
        let Some((rec, at, tp, _is_arg)) = self.frame_slot(name, data) else {
            return false;
        };
        if !matches!(tp, crate::data::Type::Integer(_)) {
            return false;
        }
        *self
            .database
            .store_mut(&self.stack_cur)
            .addr_mut::<i64>(rec, at) = value;
        true
    }

    /// Write the value rendered by own-format `literal` into the **live** frame's
    /// local `name`, **type-directed** by the local's declared type — the general
    /// @PLN16 F edit-and-continue write the REPL uses (`f = 2.0`, `msg = "hi"`).
    /// Covers every **inline scalar** (integer / float / single / boolean /
    /// character), a **simple enum** (its 1-based discriminant byte), and **text**
    /// (a text *local* overwrites its owned `String`; a text *argument* repoints its
    /// `Str` at a stable [`Debugger`](crate::debugger::Debugger)-owned buffer).
    /// `literal` is parsed in that type's own-format form (the same form
    /// [`render_frame_local`](Self::render_frame_local) produces, so an edited value
    /// round-trips).
    ///
    /// A **text** edit requires the local be **live** at the pause (shown in the
    /// captured frame): overwriting a text local runs `Drop` on the old `String`, so
    /// the slot must hold a valid one — guaranteed only once its first assignment has
    /// run, which is exactly when the liveness gate shows it.
    ///
    /// Returns `false` for an unknown local, a `literal` that doesn't parse as the
    /// local's type (a type-mismatched edit is rejected), a not-yet-live text local,
    /// or a **heap** local (struct / vector / struct-enum): those hold a `DbRef` into
    /// the store, and reconstructing one in the *live* store from a literal needs a
    /// literal→store materialiser (`Stores::clone` empties `allocations`, so a value
    /// built in the REPL's store cannot be aliased across) — the remaining work.
    pub fn set_frame_literal(
        &mut self,
        name: &str,
        literal: &str,
        data: &crate::data::Data,
    ) -> bool {
        use crate::data::Type;
        // @PLN120 A — one gate for every type, replacing the `Text`-arm-only check.
        // A local the frame does not hold shares its slot with another local (or has
        // no completed write yet), so ANY edit through its name lands in bytes that
        // are not its own — and for `text` it `Drop`s a `String` that was never
        // constructed.
        if !self.frame_local_is_live(name, data) {
            return false;
        }
        let Some((rec, at, tp, is_arg)) = self.frame_slot(name, data) else {
            return false;
        };
        let lit = literal.trim();
        // @PLN16 M2 — snapshot the slot for undo before the typed write.  Width by type
        // (text arg = 16-byte `Str`, text local = 24-byte `String`); a heap `_` slot
        // gets 0 here and returns below — that case is `set_frame_dbref`'s.  A failing
        // arm returns without committing, so the snapshot is simply dropped.
        let store_nr = self.stack_cur.store_nr;
        let len = match &tp {
            Type::Integer(_) | Type::Float => 8,
            Type::Single | Type::Character => 4,
            Type::Boolean | Type::Enum(_, false, _) => 1,
            Type::Text(_) => {
                if is_arg {
                    16
                } else {
                    24
                }
            }
            _ => 0,
        };
        let before = self.edit_before(store_nr, rec, at, len);
        match &tp {
            Type::Integer(_) => {
                let Ok(v) = lit.parse::<i64>() else {
                    return false;
                };
                *self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<i64>(rec, at) = v;
            }
            Type::Float => {
                let Ok(v) = lit.parse::<f64>() else {
                    return false;
                };
                *self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<f64>(rec, at) = v;
            }
            Type::Single => {
                let Ok(v) = lit.trim_end_matches('f').parse::<f32>() else {
                    return false;
                };
                *self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<f32>(rec, at) = v;
            }
            Type::Boolean => {
                let v = match lit {
                    "true" => 1u8,
                    "false" => 0u8,
                    _ => return false,
                };
                *self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<u8>(rec, at) = v;
            }
            Type::Character => {
                // Own-format is `'c'`; take the single char between the quotes.
                let inner = lit
                    .strip_prefix('\'')
                    .and_then(|s| s.strip_suffix('\''))
                    .unwrap_or(lit);
                let mut cs = inner.chars();
                let (Some(c), None) = (cs.next(), cs.next()) else {
                    return false;
                };
                *self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<u32>(rec, at) = c as u32;
            }
            Type::Text(_) => {
                let Some(owned) = unescape_loft_text(lit) else {
                    return false;
                };
                if is_arg {
                    // 16-byte `Str` borrow → point it at a stable Debugger-owned buffer.
                    let Some(dbg) = self.debug.as_mut() else {
                        return false;
                    };
                    dbg.edited_text.push(owned);
                    let Some(s) = dbg.edited_text.last() else {
                        return false; // unreachable after push — no panic path
                    };
                    let new = Str {
                        ptr: s.as_ptr(),
                        len: s.len() as u32,
                    };
                    *self
                        .database
                        .store_mut(&self.stack_cur)
                        .addr_mut::<Str>(rec, at) = new;
                } else {
                    // 24-byte owned `String`.  Use `ptr::write`, not assignment:
                    // assignment drops the slot's prior `String`, but the liveness
                    // gate can show a text local still at its own assignment op
                    // (slot uninitialised — `reserve_frame` does not zero), so the
                    // prior bytes may be garbage and dropping them is UB.
                    // `ptr::write` overwrites without dropping; a genuinely-prior
                    // `String`'s buffer leaks (a tiny, debug-session-only cost) and
                    // scope-exit `OpFreeText` frees the new one.
                    let dst: *mut String = std::ptr::from_mut(
                        self.database
                            .store_mut(&self.stack_cur)
                            .addr_mut::<String>(rec, at),
                    );
                    // SAFETY: `dst` is the live frame slot for a text local (24-byte
                    // `String`); `ptr::write` neither reads nor drops its prior value.
                    unsafe {
                        core::ptr::write(dst, owned);
                    }
                }
            }
            // Simple enum: an inline 1-based discriminant byte.  `literal` is
            // `Enum.Variant`; take the variant after the last `.`.
            Type::Enum(_, false, _) => {
                let variant = lit.rsplit('.').next().unwrap_or(lit);
                let tname = tp.name(data);
                let tp_known = self.database.name(&tname);
                if tp_known == u16::MAX {
                    return false;
                }
                let disc = self.database.to_enum(tp_known, variant);
                if disc == 0 {
                    return false;
                }
                *self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<u8>(rec, at) = disc;
            }
            // Heap (struct / vector / struct-enum): a `DbRef` slot — see the doc note.
            _ => return false,
        }
        // Reached only on a successful write (every failing arm returns above) — record
        // the Modify for undo.
        self.edit_after(store_nr, rec, at, before);
        true
    }

    /// @PLN16.J — edit a **scalar at a struct-field path** in place (`pt.x = 9`,
    /// `pt.inner.x = 9`).  `base` is the struct local; `fields` is the dotted chain.
    /// Resolves `base`'s `DbRef` from the frame slot, then walks the chain summing
    /// each field's byte offset — a nested struct is **inlined** into the same
    /// record, so descending is just offset addition (the same address `ShowDb`
    /// reads, so the write round-trips), with no `DbRef`-follow.  The leaf must be a
    /// scalar; the write is pure in-place (no allocation, no `DbRef` change), correct
    /// by construction like the other modify edits.  Returns `false` for an
    /// unknown / non-struct `base`, a null struct, an unknown field, an intermediate
    /// field that is not an inline struct (a vector / linked ref — routed to the
    /// element / whole-value slice), a non-scalar leaf, or an unparseable `lit`.
    pub fn set_frame_path(
        &mut self,
        base: &str,
        fields: &[&str],
        lit: &str,
        data: &crate::data::Data,
    ) -> bool {
        let Some((store_nr, rec, off, content)) = self.path_region(base, fields, data) else {
            return false;
        };
        self.write_scalar_at(store_nr, rec, off, content, lit.trim())
    }

    /// @PLN16 — resolve a struct-field path (`pt.x`, `pt.inner.x`) from the paused frame
    /// to the heap region it occupies + its primitive type: `(store_nr, rec, off,
    /// content)`.  Nested structs are inlined, so descending is offset addition in the
    /// same record (the address `ShowDb` reads).  Shared by the field edit
    /// ([`set_frame_path`](Self::set_frame_path)) and the watchpoint resolver — read-only.
    /// `None` for a non-struct base / null struct / unknown field / non-inline
    /// intermediate.  The leaf's `content` may be any type; the caller decides whether a
    /// non-scalar leaf is acceptable.
    fn path_region(
        &self,
        base: &str,
        fields: &[&str],
        data: &crate::data::Data,
    ) -> Option<(u16, u32, u32, u16)> {
        let (rec, at, tp, _is_arg) = self.frame_slot(base, data)?;
        // Only a struct (`Reference`) local has named fields to descend.
        if fields.is_empty() || !matches!(tp, crate::data::Type::Reference(_, _)) {
            return None;
        }
        let db = *self
            .database
            .store(&self.stack_cur)
            .addr::<crate::keys::DbRef>(rec, at);
        if db.rec == 0 {
            return None; // null struct
        }
        let mut tp_known = self.database.name(&tp.name(data));
        let mut off = db.pos;
        for (i, field) in fields.iter().enumerate() {
            let (position, content) = self.database.struct_field(tp_known, field)?;
            off += u32::from(position);
            if i + 1 == fields.len() {
                return Some((db.store_nr, db.rec, off, content));
            }
            // Intermediate fields must be inline nested structs (summed offset, same
            // record); a vector / linked ref is the element / whole-value slice.
            if !self.database.is_struct(content) {
                return None;
            }
            tp_known = content;
        }
        None // unreachable — the leaf returns above
    }

    /// Edit a single scalar struct field — the one-level case of
    /// [`set_frame_path`](Self::set_frame_path).
    pub fn set_frame_field(
        &mut self,
        base: &str,
        field: &str,
        lit: &str,
        data: &crate::data::Data,
    ) -> bool {
        self.set_frame_path(base, &[field], lit, data)
    }

    /// @PLN16 — live edit of a scalar vector **element** (`v[i] = x`).  The cheapest
    /// heap edit: the element lives in the vector's backing record, so this is one
    /// in-place scalar write — no materialisation.  Mirrors the interpreter's own
    /// element access (`codegen_runtime.rs`): the element type is `content(vec_tp)`,
    /// the stride is `size(elem_tp)`, and the slot is `8 + i * stride` within the
    /// backing record.  Returns `false` (no write) on a non-vector base, a null / empty
    /// vector, an out-of-range index, or a non-scalar element type — never writes past
    /// the end.
    pub fn set_frame_element(
        &mut self,
        base: &str,
        index: i64,
        lit: &str,
        data: &crate::data::Data,
    ) -> bool {
        let Some((store_nr, rec, off, content)) = self.element_region(base, index, data) else {
            return false;
        };
        self.write_scalar_at(store_nr, rec, off, content, lit.trim())
    }

    /// @PLN16 — resolve a vector element (`v[i]`) from the paused frame to its heap
    /// region + element type: `(store_nr, vec_rec, 8 + i·stride, elem_tp)`.  Mirrors the
    /// interpreter's own element access.  Shared by the element edit
    /// ([`set_frame_element`](Self::set_frame_element)) and the watchpoint resolver —
    /// read-only.  `None` on a non-vector base / null / empty vector / out-of-range or
    /// negative index.
    fn element_region(
        &self,
        base: &str,
        index: i64,
        data: &crate::data::Data,
    ) -> Option<(u16, u32, u32, u16)> {
        let (rec, at, tp, _is_arg) = self.frame_slot(base, data)?;
        if !matches!(tp, crate::data::Type::Vector(_, _)) {
            return None;
        }
        let vec_tp = self.database.name(&tp.name(data));
        let elem_tp = self.database.content(vec_tp);
        let stride = u32::from(self.database.size(elem_tp));
        if stride == 0 {
            return None;
        }
        // The frame slot holds the vector handle `DbRef`; its (rec, pos) cell holds the
        // backing record number, and elements live at `8 + i * stride` within it.
        let db = *self
            .database
            .store(&self.stack_cur)
            .addr::<crate::keys::DbRef>(rec, at);
        if db.rec == 0 {
            return None; // null vector
        }
        let vec_rec = self.database.store(&db).get_u32_raw(db.rec, db.pos);
        if vec_rec == 0 {
            return None; // empty vector — every index is out of range
        }
        let length = self.database.store(&db).get_u32_raw(vec_rec, 4);
        let idx = u32::try_from(index).ok()?;
        if idx >= length {
            return None; // out of range / negative
        }
        Some((db.store_nr, vec_rec, 8 + idx * stride, elem_tp))
    }

    /// Write `lit` as a scalar into record `(store_nr, rec)` at byte offset `off`,
    /// dispatched by the field's primitive type number (the `ShowDb` scalar map):
    /// 0 integer · 2 single · 3 float · 4 boolean · 6 character.  long (1) / text (5)
    /// / nested heap (≥ 7) are rejected — they belong to the whole-value slice.
    /// `false` on a non-scalar type or a `lit` that doesn't parse.
    fn write_scalar_at(
        &mut self,
        store_nr: u16,
        rec: u32,
        off: u32,
        content: u16,
        lit: &str,
    ) -> bool {
        // Width of the scalar by primitive type — also rejects the non-scalar types
        // (long / text / heap) up front, before snapshotting for undo.
        let len = match content {
            0 | 3 => 8, // integer / float
            2 | 6 => 4, // single / character
            4 => 1,     // boolean
            _ => return false,
        };
        let before = self.edit_before(store_nr, rec, off, len);
        let probe = crate::keys::DbRef {
            store_nr,
            rec,
            pos: 0,
        };
        // Scope the store borrow so `edit_after` (which re-reads the store) can run.
        let ok = {
            let store = self.database.store_mut(&probe);
            match content {
                0 => match lit.parse::<i64>() {
                    Ok(v) => {
                        *store.addr_mut::<i64>(rec, off) = v;
                        true
                    }
                    Err(_) => false,
                },
                2 => match lit.trim_end_matches('f').parse::<f32>() {
                    Ok(v) => {
                        *store.addr_mut::<f32>(rec, off) = v;
                        true
                    }
                    Err(_) => false,
                },
                3 => match lit.parse::<f64>() {
                    Ok(v) => {
                        *store.addr_mut::<f64>(rec, off) = v;
                        true
                    }
                    Err(_) => false,
                },
                4 => match lit {
                    "true" => {
                        *store.addr_mut::<u8>(rec, off) = 1;
                        true
                    }
                    "false" => {
                        *store.addr_mut::<u8>(rec, off) = 0;
                        true
                    }
                    _ => false,
                },
                6 => {
                    let inner = lit
                        .strip_prefix('\'')
                        .and_then(|s| s.strip_suffix('\''))
                        .unwrap_or(lit);
                    let mut cs = inner.chars();
                    match (cs.next(), cs.next()) {
                        (Some(c), None) => {
                            *store.addr_mut::<u32>(rec, off) = c as u32;
                            true
                        }
                        _ => false,
                    }
                }
                _ => false,
            }
        };
        if ok {
            self.edit_after(store_nr, rec, off, before);
        }
        ok
    }

    /// @PLN16 M3 — byte width of a scalar primitive type (the `ShowDb` map): 0 integer /
    /// 3 float → 8, 2 single / 6 character → 4, 4 boolean → 1.  `None` for a non-scalar
    /// type — watchpoints (like the scalar edits) only target a scalar region.
    fn scalar_len(content: u16) -> Option<u32> {
        match content {
            0 | 3 => Some(8),
            2 | 6 => Some(4),
            4 => Some(1),
            _ => None,
        }
    }

    /// @PLN16 M3 — render a scalar region's raw bytes as a value, for a watchpoint's
    /// `old → new` report.  Dispatched by the primitive type number.
    fn render_scalar_bytes(bytes: &[u8], content: u16) -> String {
        let b4 = |b: &[u8]| <[u8; 4]>::try_from(&b[..4]).unwrap_or([0; 4]);
        let b8 = |b: &[u8]| <[u8; 8]>::try_from(&b[..8]).unwrap_or([0; 8]);
        match content {
            0 if bytes.len() >= 8 => i64::from_le_bytes(b8(bytes)).to_string(),
            2 if bytes.len() >= 4 => format!("{}f", f32::from_le_bytes(b4(bytes))),
            3 if bytes.len() >= 8 => loft_float_literal(&f64::from_le_bytes(b8(bytes)).to_string()),
            4 if !bytes.is_empty() => if bytes[0] == 0 { "false" } else { "true" }.to_string(),
            6 if bytes.len() >= 4 => char::from_u32(u32::from_le_bytes(b4(bytes)))
                .map_or_else(|| "?".to_string(), |c| format!("'{c}'")),
            _ => "?".to_string(),
        }
    }

    /// @PLN16 M3 — resolve a watch expression (`pt.x`, `pt.inner.x`, `v[i]`) from the
    /// paused frame to its scalar heap region: `(store_nr, rec, off, len, content)`.
    /// Reuses the field / element resolvers ([`path_region`](Self::path_region) /
    /// [`element_region`](Self::element_region)).  `None` for a bare local (a stack slot
    /// — not a stable watch target), a non-scalar leaf, or any case the edits reject.
    fn resolve_watch_region(
        &self,
        expr: &str,
        data: &crate::data::Data,
    ) -> Option<(
        u16,
        u32,
        u32,
        u32,
        u16,
        Option<crate::debugger::StackWatchFrame>,
    )> {
        let (store_nr, rec, off, content) = if let Some(open) = expr.find('[') {
            let base = expr[..open].trim();
            if base.contains('.') {
                return None; // `s.items[0]` (path + index) not handled
            }
            let idx = expr[open + 1..]
                .strip_suffix(']')?
                .trim()
                .parse::<i64>()
                .ok()?;
            self.element_region(base, idx, data)?
        } else if expr.contains('.') {
            let mut segs = expr.split('.').map(str::trim);
            let base = segs.next()?;
            let path: Vec<&str> = segs.collect();
            self.path_region(base, &path, data)?
        } else {
            // @PLN63 DB0 — a bare scalar local: watch its **stack** slot, bound to the
            // current frame (dropped when that frame returns).  Unlike a heap region this
            // isn't a stable target across frame exit, so it carries a `StackWatchFrame`.
            let (rec, at, tp, _is_arg) = self.frame_slot(expr.trim(), data)?;
            let content = Self::scalar_content(&tp)?;
            let len = Self::scalar_len(content)?;
            let frame = self.call_stack.last()?;
            let watch_frame = crate::debugger::StackWatchFrame {
                d_nr: frame.d_nr,
                args_base: frame.args_base,
                depth: u32::try_from(self.call_stack.len() - 1).ok()?,
            };
            return Some((
                self.stack_cur.store_nr,
                rec,
                at,
                len,
                content,
                Some(watch_frame),
            ));
        };
        let len = Self::scalar_len(content)?;
        Some((store_nr, rec, off, len, content, None))
    }

    /// The scalar primitive type number (the `Watchpoint.content` / `ShowDb` code) for a
    /// watchable scalar `Type`, or `None` for a non-scalar (struct / vector / text / enum).
    fn scalar_content(tp: &crate::data::Type) -> Option<u16> {
        use crate::data::Type;
        Some(match tp {
            Type::Integer(_) => 0,
            Type::Single => 2,
            Type::Float => 3,
            Type::Boolean => 4,
            Type::Character => 6,
            _ => return None,
        })
    }

    /// @PLN16 M3 — set a **watchpoint** on the scalar heap region named by `expr`
    /// (`pt.x`, `v[i]`): snapshot its current bytes and register it, so a resumed run
    /// pauses when a later write changes it.  Returns `false` for an unwatchable
    /// expression (a bare local, a non-scalar / null / out-of-range target).
    pub fn add_watchpoint(&mut self, expr: &str, data: &crate::data::Data) -> bool {
        let Some((store_nr, rec, off, len, content, frame)) = self.resolve_watch_region(expr, data)
        else {
            return false;
        };
        let last = self.database.allocations[store_nr as usize].read_span(rec, off, len);
        let wp = crate::debugger::Watchpoint {
            label: expr.trim().to_string(),
            store_nr,
            rec,
            off,
            len,
            content,
            last,
            frame,
        };
        self.enable_debug();
        if let Some(d) = self.debug.as_deref_mut() {
            d.watchpoints.push(wp);
        }
        true
    }

    /// @PLN16 M3 — re-read every watchpoint's region; on the **first** that changed,
    /// update its snapshot and return the hit (label + old → new).  Called after each op
    /// of a resumed run ([`debug_step`](Self::debug_step)).  Skips a watch whose store
    /// was freed (the value was deallocated) rather than reading stale memory.
    fn poll_watchpoints(&mut self) -> Option<crate::debugger::WatchHit> {
        // @PLN63 DB1 — drop stack-local watches whose frame has returned (the slot is now
        // dead / reused), BEFORE reading — so a popped local never fires a spurious hit.  A
        // heap watch (`frame: None`) survives frame exit and is kept.
        {
            let call_stack = &self.call_stack;
            if let Some(d) = self.debug.as_deref_mut() {
                d.watchpoints.retain(|w| match w.frame {
                    None => true,
                    Some(f) => call_stack
                        .get(f.depth as usize)
                        .is_some_and(|c| c.d_nr == f.d_nr && c.args_base == f.args_base),
                });
            }
        }
        let count = self.debug.as_deref().map_or(0, |d| d.watchpoints.len());
        for i in 0..count {
            let (store_nr, rec, off, len, content) = {
                let w = &self.debug.as_deref()?.watchpoints[i];
                (w.store_nr, w.rec, w.off, w.len, w.content)
            };
            if store_nr as usize >= self.database.allocations.len()
                || self.database.allocations[store_nr as usize].free
            {
                continue;
            }
            let cur = self.database.allocations[store_nr as usize].read_span(rec, off, len);
            let old = self.debug.as_deref()?.watchpoints[i].last.clone();
            if *old != *cur {
                let hit = crate::debugger::WatchHit {
                    label: self.debug.as_deref()?.watchpoints[i].label.clone(),
                    old: Self::render_scalar_bytes(&old, content),
                    new: Self::render_scalar_bytes(&cur, content),
                };
                self.debug.as_deref_mut()?.watchpoints[i].last = cur;
                return Some(hit);
            }
        }
        None
    }

    /// @PLN16 M3 — the watchpoint that fired during the most recent resume, taken (and
    /// cleared) by the driver to report it.  `None` if no watch fired.
    pub fn take_watch_hit(&mut self) -> Option<crate::debugger::WatchHit> {
        self.debug.as_deref_mut().and_then(|d| d.last_watch.take())
    }

    /// @PLN16 M3 — the labels of the active watchpoints, for `:watch` (list).
    #[must_use]
    pub fn watchpoint_labels(&self) -> Vec<String> {
        self.debug.as_deref().map_or_else(Vec::new, |d| {
            d.watchpoints.iter().map(|w| w.label.clone()).collect()
        })
    }

    /// @PLN16 M3 — remove all watchpoints.
    pub fn clear_watchpoints(&mut self) {
        if let Some(d) = self.debug.as_deref_mut() {
            d.watchpoints.clear();
        }
    }

    /// Re-capture the current suspension's frame so [`paused_frame`](Self::paused_frame)
    /// reflects a value just written with [`set_frame_value`](Self::set_frame_value) —
    /// the user edits `n` at the paused prompt and `:vars` shows the new value.
    /// No-op when not paused.  The pause pc is `code_pos`: the suspend hook returns
    /// *before* executing the breakpoint op, so `code_pos` still names it (the same
    /// pc the frame was first captured at).
    pub fn refresh_paused_frame(&mut self, data: &crate::data::Data) {
        if !self.is_paused() {
            return;
        }
        let hit = self.capture_break_frame(self.code_pos, data);
        if let Some(d) = self.debug.as_mut() {
            d.paused = Some(hit);
        }
    }

    /// @PLN63 RX1 — capture the execution-mutable state (heap + registers) for the reverse-step
    /// ring.  `None` when a durable (file-backed) store is live — its file cannot be reversed,
    /// so reverse-stepping is refused for that session (a normal debug session is all
    /// in-memory).  Read-only; take it at a step boundary.
    #[must_use]
    pub fn snapshot_checkpoint(&self) -> Option<StepCheckpoint> {
        Some(StepCheckpoint {
            heap: self.database.snapshot_heap()?,
            code_pos: self.code_pos,
            call_stack: self.call_stack.clone(),
            stack_cur: self.stack_cur,
            stack_high: self.stack_high,
            stack_pos: self.stack_pos,
            stack_cap_bytes: self.stack_cap_bytes,
            arguments: self.arguments,
            coroutines: self.coroutines.clone(),
            active_coroutines: self.active_coroutines.clone(),
        })
    }

    /// @PLN63 RX1 — restore a [`StepCheckpoint`]: the heap bytes + every execution register, so
    /// the state is byte-identical to when the checkpoint was taken.  The checkpoint is left
    /// intact (copied, not consumed).  Refresh the paused frame after, so the drill-down
    /// reflects the restored values.
    pub fn restore_checkpoint(&mut self, cp: &StepCheckpoint) {
        self.database.restore_heap(&cp.heap);
        self.code_pos = cp.code_pos;
        self.call_stack.clone_from(&cp.call_stack);
        self.stack_cur = cp.stack_cur;
        self.stack_high = cp.stack_high;
        self.stack_pos = cp.stack_pos;
        self.stack_cap_bytes = cp.stack_cap_bytes;
        self.arguments = cp.arguments;
        self.coroutines.clone_from(&cp.coroutines);
        self.active_coroutines.clone_from(&cp.active_coroutines);
    }

    /// @PLN63 RX2 — arm (or disarm) reverse stepping: while on, each `debug_step` checkpoints
    /// the pre-step state so `step_back` can return to it.  Off costs nothing.
    pub fn set_reverse(&mut self, on: bool) {
        self.enable_debug();
        if let Some(d) = self.debug.as_mut() {
            d.reverse = on;
            if on {
                if d.reverse_cap == 0 {
                    d.reverse_cap = reverse_depth_from_env();
                }
            } else {
                d.reverse_ring.clear();
            }
        }
    }

    /// @PLN63 RX3 — set the reverse ring's capacity (the reversible depth, ≥ 1), trimming the
    /// oldest steps if it shrinks below the current length.  The DAP layer / tests use this to
    /// override `LOFT_REVERSE_DEPTH`.
    pub fn set_reverse_depth(&mut self, depth: usize) {
        self.enable_debug();
        if let Some(d) = self.debug.as_mut() {
            d.reverse_cap = depth.max(1);
            while d.reverse_ring.len() > d.reverse_cap {
                d.reverse_ring.pop_front();
            }
        }
    }

    /// @PLN63 RX2 — step **backward**: pop the most recent checkpoint and restore it (heap +
    /// registers), landing on exactly the state before the last forward step; refresh the
    /// paused frame so the drill-down reflects it.  Returns `false` when the ring is empty
    /// (no earlier state retained) — a clean floor, never a wrong state.
    pub fn step_back(&mut self, data: &crate::data::Data) -> bool {
        let Some(cp) = self.debug.as_mut().and_then(|d| d.reverse_ring.pop_back()) else {
            return false;
        };
        self.restore_checkpoint(&cp);
        self.refresh_paused_frame(data);
        true
    }

    /// Execute-loop hook: if `pc` is a registered breakpoint, capture the live
    /// frame.  Returns `true` to **suspend** the loop (stepping mode — the frame is
    /// stashed in `debug.paused` for the driver to read / edit, then `resume`);
    /// `false` to continue (record-and-continue mode — the frame is appended to
    /// `debug.hits`).  Always `false` when `pc` is not a breakpoint.
    fn debug_check(&mut self, pc: u32, data: &crate::data::Data) -> bool {
        // @PLN140 arc B — the sample tick rides the branch that got us here, which is
        // the whole reason the profiler lives on the `Debugger`: a run that is not
        // profiling pays for none of it.
        if self.debug.as_deref().is_some_and(|d| d.prof.is_some()) {
            self.profile_tick(pc);
        }
        self.profile_flush_check(data);
        let is_bp = self.debug.as_ref().is_some_and(|d| d.is_breakpoint(pc));
        if !is_bp {
            return false;
        }
        let hit = self.capture_break_frame(pc, data);
        let suspended = if let Some(d) = self.debug.as_mut() {
            if d.stepping {
                d.paused = Some(hit);
                true // suspend the loop
            } else {
                d.hits.push(hit);
                false
            }
        } else {
            false
        };
        if suspended {
            // @PLN120 F — same decision as the step landing: a breakpoint reached
            // mid-`:continue` is a new pause, so re-check which edits it still owns.
            self.validate_undo_history(data);
        }
        suspended
    }

    /// loft#1089 — render the profile mid-run when something asked for one.
    ///
    /// The report is built from the samples on THIS `State`, resolved against the `Data`
    /// they were compiled from, so the execute loop is the only place it can be rendered
    /// — a signal handler can only raise the request, and a timer can only be read here.
    /// That is why a program whose one exit is a `kill` could not produce a profile at
    /// all: the report ran at process exit and the process never reached it.
    ///
    /// Rendering does not reset the samples: each report covers the run so far, so a
    /// periodic one reads as a growing picture rather than as a slice whose neighbours
    /// have to be added up by hand.
    fn profile_flush_check(&mut self, data: &crate::data::Data) {
        let periodic = self
            .debug
            .as_deref_mut()
            .and_then(|d| d.prof.as_mut())
            .is_some_and(|p| p.periodic_flush_due());
        let asked = crate::profiler::take_pending();
        if periodic || asked != crate::profiler::Flush::None {
            self.report_alloc_sites(data);
            self.report_profile(data);
            // The two instruments compound for a networked server: the CPU profiler
            // cannot be REACHED and the network profiler cannot SEE it, so a socket
            // server had neither (loft#1088).  One flush answers both.
            crate::net_profile::report();
        }
        if let crate::profiler::Flush::ReportAndExit(code) = asked {
            // The signal that asked for this is a shutdown request, and the report it
            // wanted is now out.  Leaving here rather than unwinding keeps the exit code
            // the one the signal implies.
            std::process::exit(code);
        }
    }

    /// @PLN140 arc B — one op of a profiled run: count it, and when it is a sample
    /// point credit the interval since the previous sample to this position and this
    /// call path.
    fn profile_tick(&mut self, pc: u32) {
        // The frames are read BEFORE the profiler is borrowed mutably, so the two
        // never overlap and the sampler needs no access to the rest of `State`.
        let due = self
            .debug
            .as_deref_mut()
            .and_then(|d| d.prof.as_mut())
            .is_some_and(|p| p.cpu_armed() && p.tick());
        if !due {
            return;
        }
        let frames = self.profile_frames();
        if let Some(p) = self.debug.as_deref_mut().and_then(|d| d.prof.as_mut()) {
            p.record(pc, &frames);
        }
    }

    /// @PLN140 arc C — the op at `pc` allocated `stores` stores; hand the path that
    /// reached them to the sampler.
    fn profile_alloc(&mut self, pc: u32, stores: u64) {
        let frames = self.profile_frames();
        if let Some(p) = self.debug.as_deref_mut().and_then(|d| d.prof.as_mut()) {
            p.record_alloc(pc, &frames, stores);
        }
    }

    /// The innermost frames of the live call stack, as definition numbers.
    ///
    /// Only the innermost window is collected: a recursive program's stack is
    /// hundreds deep and the sampler keeps that window anyway, so copying the rest
    /// would be work thrown away once per sample.
    fn profile_frames(&self) -> Vec<u32> {
        let start = self
            .call_stack
            .len()
            .saturating_sub(crate::profiler::PATH_DEPTH);
        self.call_stack[start..].iter().map(|f| f.d_nr).collect()
    }

    /// Read the current (topmost) frame's in-scope variables into a
    /// [`BreakHit`](crate::debugger::BreakHit) at breakpoint offset `pc`.  Captures
    /// arguments (always live) plus the **non-argument locals that are live at
    /// `pc`** — gated by each variable's bytecode reference range (Q6).
    ///
    /// Liveness is derived from `self.vars` — the codegen `code_pos → var_nr` map
    /// (the same one `debug.rs` uses for slot dumps).  It is **read-dominated**
    /// (every `Var` read records its pc; scalar first-assignments do not), so a
    /// local is shown iff its **reference range** contains the breakpoint:
    /// `first_ref <= pc <= last_ref`.  This is *safe* — a variable inside its
    /// read-range has necessarily been assigned, so it never reads zero/garbage —
    /// and it picks the right owner of a **reused** slot (disjoint ranges, at most
    /// one contains `pc`).  It under-shows only a defined-but-not-yet-read local
    /// before its first read; that is an acceptable v1 limitation, not a hazard.
    fn capture_break_frame(&self, pc: u32, data: &crate::data::Data) -> crate::debugger::BreakHit {
        let (d_nr, frame_base) = self
            .call_stack
            .last()
            .map_or((u32::MAX, 0), |f| (f.d_nr, f.args_base));
        self.capture_frame_at(d_nr, frame_base, pc, data)
    }

    /// @PLN16 B3 — capture the **full runtime call stack**, one `BreakHit` per
    /// frame, innermost first.  The frames come from the live `call_stack`, so the
    /// chain is exactly the one that *actually ran* — **including frames reached
    /// via indirect (fn-ref) calls that the static call graph (B1) cannot see**.
    /// Each frame's liveness `pc` is the breakpoint pc (top frame) or the call site
    /// into the frame above it (`call_pos`).  Read-only — call it at a suspension.
    #[must_use]
    pub fn break_stack(&self, data: &crate::data::Data) -> Vec<crate::debugger::BreakHit> {
        let n = self.call_stack.len();
        (0..n)
            .rev()
            .map(|i| {
                let frame = &self.call_stack[i];
                let pc = if i + 1 == n {
                    self.code_pos
                } else {
                    self.call_stack[i + 1].call_pos
                };
                self.capture_frame_at(frame.d_nr, frame.args_base, pc, data)
            })
            .collect()
    }

    /// @PLN120 A — **the** frame query: every local of `d_nr` in lexical scope at
    /// `pc`, each tagged with whether the frame still holds its value.  One source
    /// for the captured frame, the slot dump, the `:eval` gate, the edit gate and
    /// the store-UAF detector — the tag is part of the entry precisely so no caller
    /// can forget to ask.
    ///
    /// Three facts, joined here (§ A.3 of the plan):
    ///
    /// * **in scope** — `pc` falls inside a [`scope_spans`](Self::scope_spans) span
    ///   whose scope is the local's. A child block is emitted inside its parent, so
    ///   containment is the nesting relation.
    /// * **assigned** — a [`store_spans`](Self::store_spans) entry for it *ends* at
    ///   or before `pc`, i.e. a write has completed. Otherwise
    ///   [`Unset`](LocalState::Unset): `reserve_frame` does not zero locals, so the
    ///   slot holds stack garbage or (on a shared slot) another local's value.
    /// * **owns its slot** — no other local sharing those bytes has a *later*
    ///   completed store. Otherwise [`Reused`](LocalState::Reused): the allocator is
    ///   scope-blind, so two locals in the same scope share a slot whenever their
    ///   live ranges do not overlap.
    ///
    /// **Fallback.** A local whose scope cannot be placed — `scope == u16::MAX` (an
    /// uninstantiated generic template; also any future warm-loaded `Data`, whose
    /// `VarSnapshot` does not carry `scope`) — or a `pc` no span covers at all (the
    /// function's entry preamble) falls back to the pre-@PLN120 test: the local is
    /// shown, as `Held`, exactly while `pc` is inside its bytecode reference range.
    /// That degrades to the old behaviour rather than to an empty frame.
    fn frame_view(&self, d_nr: u32, pc: u32, data: &crate::data::Data) -> Vec<FrameEntry> {
        let def = data.def(d_nr);
        let vars = &def.variables;
        let n = def.variables.count();
        let (start, end) = (def.code_position, def.code_position + def.code_length);
        // Per-var bytecode reference range, scanning `self.vars` within this
        // function's bytecode span.  Still reported on every entry (compiler work
        // reads it) and still the fallback filter above.
        let mut first = vec![u32::MAX; n as usize];
        let mut last = vec![u32::MAX; n as usize];
        for (&bc, &v) in &self.vars {
            if bc < start || bc >= end || v as usize >= n as usize {
                continue;
            }
            let i = v as usize;
            if first[i] == u32::MAX || bc < first[i] {
                first[i] = bc;
            }
            if last[i] == u32::MAX || bc > last[i] {
                last[i] = bc;
            }
        }
        // Fact 1 — the scopes open at `pc`.
        let open: Vec<u16> = self
            .scope_spans
            .iter()
            .filter(|&&(s, e, _)| s >= start && s < end && pc >= s && pc < e)
            .map(|&(_, _, sc)| sc)
            .collect();
        // Fact 2 — per local, the end pc of its latest COMPLETED store at or before
        // `pc`.  `u32::MAX` = never written on this path yet.
        let mut stored = vec![u32::MAX; n as usize];
        for &(s, e, v) in &self.store_spans {
            if s < start || s >= end || v as usize >= n as usize || e > pc {
                continue;
            }
            let i = v as usize;
            if stored[i] == u32::MAX || e > stored[i] {
                stored[i] = e;
            }
        }
        let byte_range = |i: u16| -> (u32, u32) {
            let at = u32::from(vars.stack(i));
            let ctx = if vars.is_argument(i) {
                crate::data::Context::Argument
            } else {
                crate::data::Context::Variable
            };
            (at, at + u32::from(crate::variables::size(vars.tp(i), &ctx)))
        };
        let mut out = Vec::new();
        for i in 0..n {
            let slot = vars.stack(i);
            let (name, tp, is_arg) = (vars.name(i), vars.tp(i), vars.is_argument(i));
            // A local the allocator gave no slot (unused after lowering) has no
            // bytes at all, whatever its scope says — it is not in the frame in any
            // sense, so it is dropped rather than tagged.
            if slot == u16::MAX {
                continue;
            }
            // Arguments are written by the caller, so they are held throughout.
            let state = if is_arg {
                LocalState::Held
            } else if vars.scope(i) == u16::MAX || open.is_empty() {
                // Fallback — the pre-@PLN120 reference-range filter.
                let (f, l) = (first[i as usize], last[i as usize]);
                if f == u32::MAX || pc < f || pc > l {
                    LocalState::OutOfScope
                } else {
                    LocalState::Held
                }
            } else if !open.contains(&vars.scope(i)) {
                LocalState::OutOfScope
            } else if stored[i as usize] == u32::MAX {
                LocalState::Unset
            } else {
                // Whoever wrote these bytes most recently owns them.
                let (lo, hi) = byte_range(i);
                let usurper = (0..n)
                    .filter(|&j| {
                        j != i
                            && !vars.is_argument(j)
                            && vars.stack(j) != u16::MAX
                            && stored[j as usize] != u32::MAX
                            && stored[j as usize] > stored[i as usize]
                    })
                    .filter(|&j| {
                        let (jlo, jhi) = byte_range(j);
                        jlo < hi && lo < jhi
                    })
                    .max_by_key(|&j| stored[j as usize]);
                match usurper {
                    Some(j) => LocalState::Reused(vars.name(j).to_string()),
                    None => LocalState::Held,
                }
            };
            out.push(FrameEntry {
                var_nr: i,
                name: name.to_string(),
                slot,
                tp: tp.clone(),
                is_argument: is_arg,
                state,
                bc_first: first[i as usize],
                bc_last: last[i as usize],
            });
        }
        out
    }

    /// @PLN120 A — the state of one named local in the current (topmost) frame at
    /// the live `code_pos`, or `None` when the frame has no such local.  The gate
    /// every read/write of a frame slot by name goes through — and what a client
    /// asks to explain *why* a name it can see is not readable.
    #[must_use]
    pub fn frame_local_state(&self, name: &str, data: &crate::data::Data) -> Option<LocalState> {
        let frame = self.call_stack.last()?;
        if frame.d_nr == u32::MAX {
            return None;
        }
        self.frame_view(frame.d_nr, self.code_pos, data)
            .into_iter()
            .find(|e| e.name == name)
            .map(|e| e.state)
        // Note `frame_view` reports OUT-OF-SCOPE locals too, so a caller can tell "not
        // in scope here" from "no such local" — a distinction the generic
        // "couldn't evaluate" used to collapse.
    }

    /// Capture one frame — function `d_nr`, whose variable region starts at
    /// `stack_cur.pos + frame_base` — with its variables in lexical scope at `pc`.
    /// Shared by the top-frame breakpoint capture and the full-stack walk.  A local
    /// the frame no longer holds is rendered as its
    /// [`LocalState`] marker rather than dropped (@PLN120 A) — and never as the
    /// contents of a slot that is not its own.
    fn capture_frame_at(
        &self,
        d_nr: u32,
        frame_base: u32,
        pc: u32,
        data: &crate::data::Data,
    ) -> crate::debugger::BreakHit {
        if d_nr == u32::MAX {
            return crate::debugger::BreakHit {
                function: "?".to_string(),
                locals: Vec::new(),
                unheld: Vec::new(),
                line: self.line_at(pc),
            };
        }
        let raw = data.def(d_nr).name();
        let function = raw.strip_prefix("n_").unwrap_or(raw).to_string();
        let mut locals = Vec::new();
        let mut unheld = Vec::new();
        for e in self.frame_view(d_nr, pc, data) {
            if e.state == LocalState::OutOfScope {
                continue;
            }
            let rendered = match e.state.marker() {
                Some(m) => {
                    unheld.push(e.name.clone());
                    m
                }
                None => self.render_frame_local(frame_base, e.slot, &e.tp, e.is_argument, data),
            };
            locals.push((e.name, rendered));
        }
        crate::debugger::BreakHit {
            function,
            locals,
            unheld,
            line: self.line_at(pc),
        }
    }

    /// Render a frame variable at frame offset `off` (its `vars.stack(i)`) of type
    /// `tp` to loft source.  Reads at the **frame-absolute** position
    /// `stack_cur.pos + frame_base + off` — the variable's fixed slot — rather than
    /// via `get_var`, whose `pos` operand is stack-depth-relative (correct only at
    /// the exact op the codegen emitted it for, not at an arbitrary pause).  Covers
    /// every value type: scalars + text inline, `DbRef`-backed heap values
    /// (struct / vector / struct-enum) via [`show_loft`](crate::database::Stores),
    /// and a simple enum via its discriminant byte — the same dispatch the REPL's
    /// value-snapshot uses, but reading a frame slot instead of the stack top.
    fn render_frame_local(
        &self,
        frame_base: u32,
        off: u16,
        tp: &crate::data::Type,
        is_arg: bool,
        data: &crate::data::Data,
    ) -> String {
        use crate::data::Type;
        let rec = self.stack_cur.rec;
        let at = self.stack_cur.pos + frame_base + u32::from(off);
        // Each scalar read takes a fresh `store` borrow (released at the end of the
        // arm) so the heap arms can re-borrow `self.database` for `show_loft`.
        match tp {
            Type::Integer(_) => self
                .database
                .store(&self.stack_cur)
                .addr::<i64>(rec, at)
                .to_string(),
            // 255 is @PLN17's three-state-boolean null sentinel (C73); rendering it as
            // "null" is inert pre-merge (two-state writes only 0/1) and correct after.
            Type::Boolean => match *self.database.store(&self.stack_cur).addr::<u8>(rec, at) {
                0 => "false",
                255 => "null",
                _ => "true",
            }
            .to_string(),
            // Force a decimal point so the literal re-parses as `float`/`single`,
            // not `integer` (a bare `2` would re-type-infer to `integer` when the
            // D1 bridge seeds the frame) — the same round-trip guarantee
            // `render_capture` makes via `float_literal`.
            Type::Float => loft_float_literal(&f64::to_string(
                self.database.store(&self.stack_cur).addr::<f64>(rec, at),
            )),
            Type::Single => format!(
                "{}f",
                loft_float_literal(&f32::to_string(
                    self.database.store(&self.stack_cur).addr::<f32>(rec, at)
                ))
            ),
            Type::Character => {
                char::from_u32(*self.database.store(&self.stack_cur).addr::<u32>(rec, at))
                    .map_or_else(|| "?".to_string(), |c| format!("'{c}'"))
            }
            // A text **argument** is a 16-byte `Str` borrow (`OpArgText`); a text
            // **local** is a 24-byte owned `String` (`OpVarText`).  Read each at its
            // true width — reading a local's `String` as a `Str` mis-takes the
            // capacity word for the length and renders garbage / `""`.
            Type::Text(_) => {
                let raw = if is_arg {
                    self.database
                        .store(&self.stack_cur)
                        .addr::<crate::keys::Str>(rec, at)
                        .str()
                        .to_string()
                } else {
                    // `reserve_frame` does not zero locals, so a text local shown
                    // at its own (not-yet-run) assignment op holds stack garbage.
                    // `as_ptr()`/`len()` only read fields (no deref), so they are
                    // safe on garbage; guard them like `Str::str` before building
                    // the slice, rather than `.clone()` (which would deref).
                    let s = self.database.store(&self.stack_cur).addr::<String>(rec, at);
                    let (ptr, len) = (s.as_ptr(), s.len());
                    if ptr.is_null() || (ptr as usize) < (1 << 16) || len > 10_000_000 {
                        String::new()
                    } else {
                        // SAFETY: ptr passed the same low-address / length guard
                        // `Str::str` uses; the bytes are valid UTF-8 (loft text).
                        unsafe {
                            std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len))
                        }
                        .to_string()
                    }
                };
                loft_text_literal(&raw)
            }
            // Heap value backed by a `DbRef` in the slot: struct, vector,
            // struct-enum variant → `show_loft` renders its own-format literal.
            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => {
                let tname = tp.name(data);
                let tp_known = self.database.name(&tname);
                if tp_known == u16::MAX {
                    return format!("<{tname}>");
                }
                let db = *self
                    .database
                    .store(&self.stack_cur)
                    .addr::<crate::keys::DbRef>(rec, at);
                let mut out = String::new();
                // Bounded glance for the variables panel so a big struct/vector doesn't
                // flood it (the LOFT_DUMP_DEPTH/ELEMENTS trace defaults); the full value is
                // one `eval` away via `eval_frame_heap`, which stays unbounded.
                self.database
                    .show_loft_bounded(&mut out, &db, tp_known, 2, 8);
                out
            }
            // Simple enum: an inline 1-based discriminant byte → `Enum.Variant`.
            Type::Enum(_, false, _) => {
                let tname = tp.name(data);
                let tp_known = self.database.name(&tname);
                let disc = *self.database.store(&self.stack_cur).addr::<u8>(rec, at);
                if tp_known == u16::MAX || disc == 0 {
                    "null".to_string()
                } else {
                    format!("{tname}.{}", self.database.enum_val(tp_known, disc))
                }
            }
            other => format!("<{}>", other.name(data)),
        }
    }

    /// Plan-07 phase 4 step 4.1 — raise a typed runtime error from a
    /// fault-site opcode (div-by-zero, OOB, null-deref, narrow cast, …).
    /// Resolves the offending pc back to a `Position` via the phase-1
    /// `source_spans` table, populates `database.runtime_error`, and
    /// sets `had_fatal = true` so `main.rs`'s exit-1 path fires after
    /// the dispatch loop terminates.
    ///
    /// The dispatch loop in `execute_argv` / `resume` checks
    /// `database.runtime_error.is_some()` AFTER each op and short-
    /// circuits via `code_pos = u32::MAX` — callers don't have to
    /// touch the loop machinery, just call `s.raise(kind)` and let
    /// the op finish (e.g. with a placeholder result on the stack).
    ///
    /// **Production-vs-development split (DESIGN_DECISIONS.md § C66).**
    /// When `database.logger.config.production == true` the production
    /// branch fires: log the typed event through `Logger::log_runtime_kind`,
    /// set `had_fatal = true`, and **return without populating
    /// `runtime_error`**.  The dispatch loop's short-circuit check
    /// (which only fires on `runtime_error.is_some()`) does NOT trigger
    /// — execution continues past the fault site, the op produces its
    /// sentinel value (null DbRef / `i64::MIN` / char 0), and the
    /// program stays alive.  This is the contract for production
    /// deployments (games, servers, browser embeds) where halting on
    /// an edge case would be strictly worse than a wrong-pixel
    /// recovery.
    /// A runaway worker trips this many operations before its `debug_assert` fires.
    ///
    /// `execute_argv` has `LOFT_MAX_OPS`, which names the last sixteen ops it ran; the
    /// worker dispatchers have only this, so it stays where it was rather than being
    /// generalised into a second hang guard.  The two callers that pass `0` — a host
    /// call and a `parallel { }` arm — had no ceiling before and keep none: neither is
    /// bounded by a row count, and a ceiling a legitimate run can reach reports itself
    /// as an infinite loop (loft#919).
    const WORKER_OP_CEILING: u64 = 10_000_000;

    /// Run bytecode from `code_pos` until the function returns or a typed fault halts it.
    ///
    /// The body of every worker, arm and host-call dispatcher — ten textually identical
    /// copies before this, none of which checked for a fault.  That is why a failed
    /// `assert` inside a `par` worker ran the worker's remaining rows to the end, and why
    /// the frames it was finally reported with were the PARENT's: by the time the parent
    /// re-raised the worker's halt, the frames the fault fired under were long gone
    /// (loft#1056).  One home, so the check cannot be present in some families and absent
    /// in others — the way loft#1053's first fix covered `parallel { }` and left the other
    /// three par families silent.
    ///
    /// `op_ceiling` of `0` switches off the runaway-worker `debug_assert`; see
    /// [`Self::WORKER_OP_CEILING`].
    fn run_to_return(&mut self, op_ceiling: u64) {
        let mut step: u64 = 0;
        let bytecode_len = self.bytecode.len() as u32;
        while self.code_pos < bytecode_len {
            let op = self.code::<u8>();
            if op == 255 {
                let ext = self.code::<u8>();
                OPERATORS[255 + ext as usize](self);
            } else {
                OPERATORS[op as usize](self);
            }
            step += 1;
            debug_assert!(
                op_ceiling == 0 || step < op_ceiling,
                "Worker: too many operations"
            );
            self.note_runtime_error_halt();
            if self.code_pos == u32::MAX {
                break;
            }
        }
    }

    /// The loft call frames the interpreter is currently inside, innermost first.
    ///
    /// Each entry is the function's name as the source spells it (the registry's `n_`
    /// prefix stripped).  Empty at top-level script scope, and empty when `data_ptr` is
    /// not set — there is then no definition table to resolve a frame's name against.
    ///
    /// One producer for the whole interpreter: [`Self::raise`] captures it at the fault,
    /// and [`Self::note_runtime_error_halt`] backfills it for a fault raised somewhere
    /// no `State` was in reach.
    #[must_use]
    fn current_call_chain(&self) -> Vec<String> {
        if self.data_ptr.is_null() {
            return Vec::new();
        }
        // SAFETY: the `Data` outlives this `State` — see `data_ptr`.
        let data = unsafe { &*self.data_ptr };
        self.call_stack
            .iter()
            .rev() // innermost first
            .map(|frame| {
                let name = data.def(frame.d_nr).name().to_owned();
                name.strip_prefix("n_").unwrap_or(&name).to_string()
            })
            .collect()
    }

    /// Turn a pending typed fault into a halt of the running dispatch loop, with the
    /// frames it fired under attached.
    ///
    /// Every dispatch loop calls this after each op — one home for the decision, which
    /// is what lets the backfill exist at all.  `assert` and `panic` are native fns and
    /// every `Stores`-side raise sees only `&mut Stores`, so all of them leave the chain
    /// empty; a failed assertion therefore named its line but never the call that
    /// reached it, on either backend (loft#1056).  This is the first point that holds
    /// BOTH the `State` and the error, so the frames go on here rather than at thirteen
    /// raise sites.  A chain the raiser already filled is left as it is.
    ///
    /// A fault relayed from a PLACED library is the one chain that is filled and still
    /// incomplete: it holds the frames the library was in, and the frames that CALLED
    /// the library are in this process.  Neither side can see the whole chain, so the
    /// two halves are joined here — the library's innermost-first, then the caller's —
    /// which is what makes a placed fault read exactly like an in-process one.
    fn note_runtime_error_halt(&mut self) {
        if self.database.runtime_error.is_none() {
            return;
        }
        let wants_frames = self
            .database
            .runtime_error
            .as_ref()
            .is_some_and(|e| e.call_chain.is_empty() || e.crossed_placement);
        if wants_frames {
            let chain = self.current_call_chain();
            if let Some(err) = self.database.runtime_error.as_mut() {
                err.call_chain.extend(chain);
                // Joined once: a later op must not append this process's frames
                // a second time.
                err.crossed_placement = false;
            }
        }
        self.code_pos = u32::MAX;
    }

    pub fn raise(&mut self, kind: crate::runtime_error::RuntimeErrorKind) {
        let position = self.source_loc_for(self.code_pos).cloned();
        self.raise_at(kind, position);
    }

    /// Where the innermost frame on the call stack was declared, at column 1 — the
    /// function that is running right now.
    ///
    /// The one position `--native` can name for the same fault: its shadow call stack
    /// holds that same frame, pushed with the string literals the generator read off
    /// this `Definition`.  `None` outside a run (`data_ptr` is set for the duration of
    /// `execute_argv`) or above the first frame, which leaves the diagnostic without a
    /// `-->` block rather than pointing it somewhere wrong.
    fn running_frame_declaration(&self) -> Option<Position> {
        if self.data_ptr.is_null() {
            return None;
        }
        // SAFETY: the `Data` outlives this `State` — see `data_ptr`.
        let data = unsafe { &*self.data_ptr };
        let frame = self.call_stack.last()?;
        let declared = &data.def(frame.d_nr).position;
        Some(Position {
            file: declared.file.clone(),
            line: declared.line,
            pos: 1,
        })
    }

    /// [`Self::raise`] with the position supplied rather than read off the dispatching
    /// op — for a fault whose op is not where the author should look.
    pub fn raise_at(
        &mut self,
        kind: crate::runtime_error::RuntimeErrorKind,
        position: Option<Position>,
    ) {
        // Production: log + had_fatal + return.  Do NOT populate
        // runtime_error so the dispatch loop continues (per C66).
        // The check matches `n_panic` / `n_assert`'s production check
        // shape from `src/native.rs`.
        let production = self
            .database
            .logger
            .as_ref()
            .and_then(|l| l.lock().ok())
            .is_some_and(|l| l.config.production);
        // Plan-07 phase 4g.3 — `--dev-soft-halt` (CLI flag /
        // `LOFT_DEV_SOFT_HALT=1` env var) demotes development-mode
        // raises to log-and-continue (matches production
        // semantics) so a single run surfaces every fault site
        // instead of halting on the first.  Useful when porting
        // scripts: get the full pattern of breakage in one shot.
        // The flag forces the production branch regardless of the
        // logger's `production` flag.  When no logger is attached
        // the fault is rendered to stderr via main.rs's
        // pretty-print path on exit (had_fatal is still set so
        // main.rs exits non-zero).
        let dev_soft_halt = dev_soft_halt_enabled();
        if production || dev_soft_halt {
            if let Some(logger) = &self.database.logger
                && let Ok(mut lg) = logger.lock()
            {
                lg.log_runtime_kind(&kind, position.as_ref());
            }
            // In dev-soft-halt mode, ALWAYS render the fault to
            // stderr so the developer sees each one as it
            // happens — they ran `--dev-soft-halt` specifically
            // to see the full pattern of breakage in one run.
            // Logger emission (above) doubles to the log file
            // when a logger is attached but does NOT replace
            // the stderr surface.
            if dev_soft_halt {
                eprintln!(
                    "soft-halt: {} at {}",
                    kind.describe(),
                    position.as_ref().map_or_else(
                        || "?".to_string(),
                        |p| format!("{}:{}:{}", p.file, p.line, p.pos)
                    )
                );
            }
            self.database.had_fatal = true;
            return;
        }
        // Development (default for CLI / tests / no logger / non-production
        // logger): populate runtime_error so the dispatch loop
        // short-circuits and main.rs renders the typed error.
        let message = kind.describe();
        let op_pc = self.code_pos;
        let call_chain = self.current_call_chain();
        self.database.runtime_error = Some(Box::new(crate::runtime_error::RuntimeError {
            kind,
            position,
            op_pc,
            message,
            call_chain,
            crossed_placement: false,
        }));
        self.database.had_fatal = true;
    }

    /// @P356 — record a RECOVERABLE fault (out-of-bounds / negative vector
    /// or text index) WITHOUT halting.  Project policy: runtime aborts for
    /// reversible faults belong only in opt-in debugging, not release runs.
    /// The fault returns the type's null sentinel and logs a `Warn`-level
    /// entry, then execution CONTINUES — matching `--native` and the
    /// documented `v[i] ?? <fallback>` idiom.  `LOFT_DEV_SOFT_HALT` opts into
    /// fail-fast surfacing for debugging (delegates to `raise`: stderr
    /// `soft-halt:` + `had_fatal` so the run exits non-zero, still continuing
    /// so one run surfaces every fault site).  The compile-time
    /// undefended-`v[i]` warning already nudges toward `??`; this is the
    /// runtime mirror.
    pub fn raise_recoverable(&mut self, kind: crate::runtime_error::RuntimeErrorKind) {
        if dev_soft_halt_enabled() {
            self.raise(kind);
            return;
        }
        let position = self.source_loc_for(self.code_pos).cloned();
        if let Some(logger) = &self.database.logger
            && let Ok(mut lg) = logger.lock()
        {
            lg.log_runtime_kind(&kind, position.as_ref());
        }
    }

    /// Plan-07 phase 4 step 4.6 — bounds-checked vector index that raises
    /// `IndexOutOfBounds` / `NegativeIndex` instead of returning the null
    /// Interpreter-side accessor mirrored by
    /// `Stores::const_ref_at_runtime`.  The template
    /// `#rust"s.const_ref_at(@d_nr as usize)"` resolves to this method
    /// in the bytecode interpreter context (where `s: &mut State`).
    /// Native codegen rewrites the call to `stores.const_ref_at_runtime`
    /// (see `src/generation/calls.rs`).  @P275.
    #[must_use]
    pub fn const_ref_at(&self, d_nr: usize) -> crate::keys::DbRef {
        self.const_refs[d_nr]
    }

    /// DbRef sentinel that `vector::get_vector` produced on OOB today.
    /// Used by `OpGetVector`'s annotation; the dispatch loop's
    /// `runtime_error.is_some()` check halts execution after the op
    /// returns.  Negative indices use Python-style addressing
    /// (`v[-1]` == last); only after addressing yields a still-out-of-
    /// range value does the raise fire.
    ///
    /// @FR-H-Index — this is where the index becomes an address, so it is where "negative
    /// counts from the end" and "out of range answers nullref" are decided for the
    /// interpreter.  `Stores::vec_get_or_raise_runtime` is the native twin, and
    /// `vec_get_hoisted_or_raise_runtime` sends every non-fast-path index back to it, so the
    /// normalisation has one definition per backend rather than one per call site.
    #[must_use]
    pub fn vec_get_or_raise(
        &mut self,
        db: &crate::keys::DbRef,
        size: u32,
        index: i64,
    ) -> crate::keys::DbRef {
        let len = crate::vector::length_vector(db, &self.database.allocations);
        let normalized = if index < 0 {
            index + i64::from(len)
        } else {
            index
        };
        if normalized < 0 {
            self.raise_recoverable(crate::runtime_error::RuntimeErrorKind::NegativeIndex {
                idx: index,
            });
            // The value an index that names no element answers is `nullref`, the same
            // value `vector::get_vector` answers: a reference leaving its slot has ONE
            // spelling of absence (`DbRef::or_null`, @FR-L-Null).  Every typed reader
            // tests `rec == 0` before it resolves a store, so the log-and-continue path
            // reads the typed null off it exactly as it read the old container-store
            // sentinel.
            return crate::keys::DbRef::NULL;
        }
        if normalized >= i64::from(len) {
            self.raise_recoverable(crate::runtime_error::RuntimeErrorKind::IndexOutOfBounds {
                idx: index,
                len,
            });
            return crate::keys::DbRef::NULL;
        }
        crate::vector::get_vector(db, size, index, &self.database.allocations)
    }

    /// Plan-07 phase 4 step 4.6 — bounds-checked variant of
    /// `OpVectorRef`'s body: same bounds check as `vec_get_or_raise`,
    /// then dereferences the resulting DbRef via `Stores::get_ref`.
    /// Single helper keeps the OpVectorRef annotation a one-liner.
    #[must_use]
    pub fn vec_ref_or_raise(&mut self, db: &crate::keys::DbRef, index: i64) -> crate::keys::DbRef {
        let inner = self.vec_get_or_raise(db, 4, index);
        // `get_ref` already short-circuits to a null DbRef when
        // `inner.rec == 0` (which is the OOB sentinel after the
        // store_nr-preserving sentinel change), so no extra guard
        // is required here.
        self.database.get_ref(&inner, 0)
    }

    /// Plan-07 phase 4 step 4.8 — bounds-checked text index.  `text[i]`
    /// today returns `char(0)` on OOB (silent wrong-answer); raise
    /// `IndexOutOfBounds` / `NegativeIndex` for the non-nullable path.
    /// Negative addressing mirrors `vec_get_or_raise`.
    ///
    /// @FR-H-Index for `text`, whose index is a BYTE offset — so the count from the end is in
    /// bytes, and `ops::text_character` under this snaps back to the character containing
    /// that byte.  The value-slice bound `s[a..b]` follows the same rule
    /// (`@FR-Slice-Value`, `State::get_text_sub`).
    #[must_use]
    pub fn text_char_or_raise(&mut self, val: &str, index: i64) -> char {
        let len = val.len() as i64;
        let normalized = if index < 0 { index + len } else { index };
        if normalized < 0 {
            self.raise_recoverable(crate::runtime_error::RuntimeErrorKind::NegativeIndex {
                idx: index,
            });
            return char::from(0);
        }
        if normalized >= len {
            self.raise_recoverable(crate::runtime_error::RuntimeErrorKind::IndexOutOfBounds {
                idx: index,
                len: len as u32,
            });
            return char::from(0);
        }
        crate::ops::text_character(val, index)
    }

    /// Execute entry-point `name`, optionally passing `argv` as a `vector<text>` argument.
    ///
    /// If the named function has exactly one `vector<…>` parameter, the strings in `argv`
    /// are built into a `vector<text>` and pushed onto the stack before the return address.
    /// If the function takes no parameters, `argv` is ignored.
    ///
    /// # Panics
    /// Panics if the program executes more than 10 000 000 operations (infinite-loop guard).
    pub fn execute_argv(&mut self, name: &str, data: &Data, argv: &[String]) {
        // @PLAN49 T1 — runtime phase breadcrumb.  One call per
        // program; runtime cost is irrelevant.
        crate::timeout::checkpoint_fn("run-interpret", "<entry>", "", 0);
        // loft#952 — and the vocabulary its per-call breadcrumb reports in.  `"<entry>"`
        // above is a placeholder that names nothing; from here on a hard-kill names the
        // loft function the run was in.  Skipped entirely when no watchdog is armed.
        crate::timeout::note_interp_entry(name);
        crate::timeout::publish_interp_fns(data.definitions.iter().map(|def| {
            (
                def.name()
                    .strip_prefix("n_")
                    .unwrap_or(def.name())
                    .to_string(),
                def.position().file.clone(),
                def.position().line,
            )
        }));
        // loft#665 piece 3 — compilation is over, so drop the compile position: a
        // RUNTIME panic must not be attributed to whatever line was compiled last.
        // The runtime has its own, better attribution (pc -> source span).
        crate::crash_report::clear_compile_pos();
        // Same rule for a `par` worker's halt: this run starts owing nothing to the last
        // one.  A halt recorded by a worker whose parent never reached the dispatch-loop
        // check below (a panic unwound past it) would otherwise be raised here, against a
        // program that did not produce it.
        crate::parallel::clear_worker_fatal();
        // Give the memory-ceiling report its vocabulary before anything can trip it,
        // so a refused growth names `Layer` rather than `kt=112`.  Costs nothing when
        // no ceiling is set, which is every ordinary run.
        let () = self.database.publish_type_names();
        let _ = name;
        let d_nr = data.def_nr(&format!("n_{name}"));
        // A missing entry function (e.g. running a file with no `fn main()`, or a
        // test-runner entry that did not survive compilation) must be a clean message,
        // not a `def(u32::MAX)` "Unknown definition" panic.
        if d_nr == u32::MAX {
            eprintln!("loft: no `{name}` function to run");
            return;
        }
        let pos = data.def(d_nr).code_position;

        // Expose bytecode, library, and Data to native functions
        // that need to spawn worker threads (e.g. n_parallel_for / _light).
        let bc_ptr = &raw const self.bytecode;
        let lib_ptr = &raw const self.library;
        let data_ptr = std::ptr::from_ref::<Data>(data);
        self.data_ptr = data_ptr;
        let stk_lib_nr = self
            .library_names
            .get("n_stack_trace")
            .copied()
            .unwrap_or(u16::MAX);
        self.database.parallel_ctx = Some(Box::new(ParallelCtx {
            bytecode: bc_ptr,
            library: lib_ptr,
            data: data_ptr,
            stack_trace_lib_nr: stk_lib_nr,
        }));

        self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
        self.code_pos = pos;
        // @PLAN53 cluster 2 / S4: the entry frame base must be 8-aligned in
        // aligned mode (step(4)=8) so the entry function's locals — and every
        // frame it calls — land on their alignment boundary; with the V1 base
        // of 4 the whole entry frame is misaligned by 4.  Identity when off.
        // #629: a TEXT return is `ref_return`-promoted like a vector, but its
        // buffer is a `String` the CALLER owns, not a store record — an ordinary
        // call site declares a `__work_N` local, `OpCreateStack`s a ref to it,
        // and frees it after.  The entry supplied nothing, so the callee wrote
        // through an uninitialised slot and teardown double-freed it (SIGABRT on
        // EVERY `fn main() -> text`, including a literal).  Reserve one real
        // `String` per hidden text attr BELOW the argument area, so the frame's
        // argument offsets are exactly what they were.
        let attrs = &data.def(d_nr).attributes();
        let mut text_bufs: Vec<u32> = Vec::new();
        self.stack_pos = crate::variables::aligned_stack_step(4);
        for a in *attrs {
            if a.hidden
                && matches!(&a.typedef, Type::RefVar(t) if matches!(t.base(), Type::Text(_)))
            {
                text_bufs.push(self.stack_pos);
                self.put_stack(String::new());
            }
        }
        // @PLAN53 cluster 2 / S4: the entry frame base must be 8-aligned in
        // aligned mode (step(4)=8) so the entry function's locals — and every
        // frame it calls — land on their alignment boundary; with the V1 base
        // of 4 the whole entry frame is misaligned by 4.  Identity when off.
        let entry_base = crate::variables::aligned_stack_step(self.stack_pos.max(4));
        self.stack_pos = entry_base;
        // Plan-07 phase 1 step 1.20 / phase 3 — publish source_spans
        // to the panic hook so a Rust panic inside any opcode dispatch
        // (e.g. arithmetic overflow in `checked_long!`, the `panic`
        // builtin) can print `at file:line:col` for the offending pc.
        self.publish_source_spans();
        // loft#806 — and the opcode NAMES, so a crash report says which op was
        // dispatching instead of a bare `op=249`.  Leaked once per process: a
        // signal handler cannot borrow the definitions table (the crashing thread
        // may hold it), so the names have to already be `'static` when it runs.
        // Handed over as a CLOSURE, because "once per process" is the whole licence
        // for the leak and this runs once per PROGRAM (loft#820).
        crate::crash_report::set_op_names(|| {
            (0..=u16::from(u8::MAX))
                .map(|op| {
                    data.operator_name(op)
                        .map_or("", |n| &*Box::leak(n.to_owned().into_boxed_str()))
                })
                .collect()
        });
        // Fix #88: push a synthetic CallFrame for the entry function so it
        // appears in stack_trace() output.
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: entry_base,
            args_size: 0,
            line: 0,
        });
        // If fn main declares a vector<text> parameter, push argv before the return address.
        //
        // The store is the entry frame's to release: nothing in the program owns it, because
        // no loft code allocated it.  Kept here and freed beside the frame's other cleanup
        // below, or it is still live at exit — one leaked store per run of any program that
        // reads its arguments (loft#1172).
        let mut argv_store: Option<DbRef> = None;
        if attrs.len() == 1 && !attrs[0].hidden && matches!(attrs[0].typedef, Type::Vector(_, _)) {
            let args_vec = self.database.text_vector(argv);
            argv_store = Some(args_vec);
            self.put_stack(args_vec);
        }
        // @PLAN59: the entry fn may carry hidden heap return-buffer attrs
        // (ref_return promotion / the signature-time `__retbuf`).  Push one
        // dest per hidden heap attr so the entry frame layout matches its
        // argument vars.  (The REPL's capture wrapper `fn replmain_N() -> P
        // { … }` is the canonical caller of a heap-returning entry fn.)
        //
        // #618: the buffer must be ALLOCATED, not a bare null sentinel.  Only a
        // struct/enum body opens with `OpDatabase` (alloc-from-sentinel); a
        // vector-returning body writes straight into the caller's buffer
        // (`OpClearVector` / `OpPreAllocVector` / `OpFinishRecord`), because at
        // an ordinary call site the caller allocated it — so a sentinel left
        // every element write pointing at `stores[u16::MAX]`.  Allocating here
        // is exactly what the `OpDatabase`-before-the-call a real caller emits
        // does, so entry and non-entry frames now honour the same contract.
        let mut next_text_buf = 0;
        let mut heap_ret_slots: Vec<u32> = Vec::new();
        for a in *attrs {
            if !a.hidden {
                continue;
            }
            if matches!(&a.typedef, Type::RefVar(t) if matches!(t.base(), Type::Text(_))) {
                // Hand the callee a ref to the `String` reserved above — the
                // same shape `OpCreateStack` builds for an ordinary caller.
                let slot = text_bufs[next_text_buf];
                next_text_buf += 1;
                let db = crate::keys::DbRef {
                    store_nr: self.stack_cur.store_nr,
                    rec: self.stack_cur.rec,
                    pos: self.stack_cur.pos + slot,
                };
                self.put_stack(db);
            } else if matches!(
                a.typedef,
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            ) {
                // Only the VECTOR contract is caller-allocates.  A struct /
                // data-enum body opens with its own `OpDatabase`, which turns the
                // sentinel into a store itself — pre-allocating for those would
                // just strand a second, unused record in the store.
                let dest = if matches!(a.typedef, Type::Vector(_, _)) {
                    self.alloc_hidden_return_buffer(data, &a.typedef)
                        .unwrap_or(crate::keys::DbRef::NULL)
                } else {
                    crate::keys::DbRef::NULL
                };
                // Remember WHERE the dest ref sits so the frame teardown below can
                // read back whatever the body left there and free it.  Read back
                // rather than reuse `dest`: a struct / data-enum body opens with its
                // own `OpDatabase`, so the record it actually returns is allocated
                // during the run and the sentinel pushed here is not it.
                heap_ret_slots.push(self.stack_pos);
                self.put_stack(dest);
            }
        }
        self.put_stack(u32::MAX);
        #[cfg(debug_assertions)]
        let mut step: u64 = 0;
        // loft#919 — read the ceiling once, outside the loop: the guard runs on every
        // op, and an env lookup per op would dominate a debug-assertions run.
        #[cfg(debug_assertions)]
        let max_ops = crate::keys::max_ops();
        #[cfg(debug_assertions)]
        let mut trail_pos = [u32::MAX; 16usize];
        #[cfg(debug_assertions)]
        let mut trail_op = [0u8; 16usize];
        #[cfg(debug_assertions)]
        let mut trail_head: usize = 0;
        #[allow(unused_mut)]
        let mut bytecode_len = self.bytecode.len() as u32;
        let uaf_on = crate::keys::uaf_check_enabled();
        let uaf_src_on = crate::keys::uaf_src_enabled();
        let uaf_gen_on = crate::keys::uaf_gen_enabled();
        // @PLN18 phase 02 — tier-0 live reload: a counter-gated poll so a file
        // save can swap one fn's dispatch targets mid-run (append-only code, so
        // the cached length refreshes after a swap).  One decrement + one
        // predictable branch per op; only ever true under LOFT_LIVE_RELOAD=1.
        const RELOAD_POLL_OPS: u32 = 32_768;
        #[cfg(not(target_arch = "wasm32"))]
        let reload_on = crate::live_reload::active();
        #[cfg(target_arch = "wasm32")]
        let reload_on = false;
        let mut reload_tick: u32 = RELOAD_POLL_OPS;
        // @PLN140 arc C — allocation PATHS. Hoisted out of the loop like `reload_on`
        // and `uaf_on` above, so an ordinary run pays one never-taken branch per op
        // rather than a lookup. The op is the finest granularity a path can be read
        // at: `database_named` allocates from inside `Stores`, which has no view of
        // the loft call stack, so the store count is compared across the op instead.
        let alloc_paths_on = self
            .debug
            .as_deref()
            .is_some_and(|d| d.prof.as_ref().is_some_and(|p| p.alloc_armed()));
        let mut last_allocs = self.database.stores_allocated;
        // @PLN154 phase 0 — hoisted like `alloc_paths_on` above: one never-taken branch
        // per op when the census is not armed.
        let census_on = crate::stack_census::enabled();
        // @PLN154 phase 1 — the same hoist for the shadow's one KILL chokepoint.
        let verify_on = self.verify_on;
        while self.code_pos < bytecode_len {
            if reload_on {
                reload_tick -= 1;
                if reload_tick == 0 {
                    reload_tick = RELOAD_POLL_OPS;
                    #[cfg(not(target_arch = "wasm32"))]
                    if crate::live_reload::poll(self) {
                        bytecode_len = self.bytecode.len() as u32;
                    }
                }
            }
            let op_pos_rt = self.code_pos;
            // @PLN154 phase 1 — where the stack top stood before the op.  Every route that
            // lowers it (a pop, a discard, a return, a work-buffer trim) leaves the span
            // above dead, and the shadow learns it here rather than at each of the sites:
            // the ground truth is the pointer, so a route nobody has enumerated is covered
            // like any other.  Phase 0's lesson, applied to the read side.
            let verify_sp = if verify_on { self.stack_pos } else { 0 };
            // @PLN105 leak provenance — republish the current op position so any store
            // allocated while executing this op records it as its `created_at` (one u32
            // write per op, alongside the existing crash-report context publish below).
            self.database.alloc_pc = op_pos_rt;
            // @PLN16 debugger — at a registered breakpoint, capture the frame (and
            // in stepping mode, suspend: return to the driver with the frame in
            // `debug.paused`, to be resumed via `resume`).  Inert (one branch) when
            // not debugging.
            if self.debug.is_some() && self.debug_check(op_pos_rt, data) {
                return;
            }
            #[cfg(debug_assertions)]
            let op_pos = self.code_pos;
            let op = self.code::<u8>();
            // Publish the current bytecode position + op byte so that a
            // subsequent SIGSEGV/SIGABRT prints the crash location.
            // Cheap: one thread-local store per op.
            {
                let fn_d_nr = self.call_stack.last().map_or(u32::MAX, |f| f.d_nr);
                crate::crash_report::set_context(op_pos_rt, op, "(opcode dispatch)", fn_d_nr, "");
            }
            #[cfg(debug_assertions)]
            {
                trail_pos[trail_head] = op_pos;
                trail_op[trail_head] = op;
                trail_head = (trail_head + 1) % 16;
            }
            // @PLN154 phase 0 — snapshot the stack store, run the op, diff.  The
            // ground truth is the memory: a write through a route nobody has listed is
            // counted the same as one through `put_stack`, which is the whole point of
            // measuring rather than auditing the callers.
            let census_span = if census_on {
                let origin = (self.stack_cur.rec * 8 + self.stack_cur.pos) as usize;
                // Watch the frame, not the buffer.  The whole buffer is the sound
                // region and it is what this started as, but the allocator's slack
                // above the frame grows without bound and diffing it cost 42x on
                // `1248b`, which puts the corpus out of reach.  `stack_high` is the
                // frame's high-water mark, and `MARGIN` covers the reserve an op makes
                // before the mark catches up; a write past that is reported as
                // `beyond the watched span` rather than being silently dropped.
                const MARGIN: usize = 4096;
                let end = (origin + self.stack_high as usize + MARGIN)
                    .min(self.database.store(&self.stack_cur).raw_bytes().len());
                let bytes = self.database.store(&self.stack_cur).raw_bytes();
                crate::stack_census::before_op(bytes, origin, end);
                (origin, end)
            } else {
                (0, 0)
            };
            let ran_opcode: u16 = if op == 255 {
                let ext = self.code::<u8>();
                let opc = 255 + u16::from(ext);
                OPERATORS[opc as usize](self);
                opc
            } else {
                OPERATORS[op as usize](self);
                u16::from(op)
            };
            if census_on {
                let bytes = self.database.store(&self.stack_cur).raw_bytes();
                crate::stack_census::after_op(ran_opcode, bytes, census_span.0, census_span.1);
                if crate::stack_census::budget_spent() {
                    // The budget is a corpus-sweep instrument: report on what ran and
                    // leave, so a program with tens of millions of ops still contributes
                    // its routes instead of being killed by a timeout with nothing said.
                    crate::stack_census::report(Some(data));
                    crate::loft_eprintln!("stack census: op budget spent — stopping here");
                    std::process::exit(0);
                }
            }
            if verify_on && crate::stack_verify::any_relocation() {
                self.mark_stale_handles();
            }
            if verify_on && self.stack_pos < verify_sp {
                let at = (self.stack_cur.rec * 8 + self.stack_cur.pos + self.stack_pos) as usize;
                let len = (verify_sp - self.stack_pos) as usize;
                self.database
                    .store_mut(&self.stack_cur)
                    .shadow_kill(at, len);
            }
            // @PLN140 arc C — the op just ran; if it took stores, record the path
            // that reached them.
            if alloc_paths_on {
                let allocs = self.database.stores_allocated;
                if allocs != last_allocs {
                    self.profile_alloc(op_pos_rt, allocs - last_allocs);
                    last_allocs = allocs;
                }
            }
            // LOFT_UAF: the op freed store slots — scan live frame variables
            // for one that still reads a freed slot (premature free).  Under the
            // cheap LOFT_UAF_SRC variant, only stamp each freed slot's pc (no
            // frame scan) so a later copy of a freed source can name the free site.
            if !self.database.uaf_freed_this_op.is_empty() {
                if uaf_on {
                    self.uaf_scan_freed(data);
                } else if uaf_src_on || uaf_gen_on {
                    // Stamp each freed slot's site so the gen-detector's stale-read report
                    // (@PLN118 arc B) can name WHERE the store was prematurely freed, not
                    // just where the stale ref is read.
                    let freed = std::mem::take(&mut self.database.uaf_freed_this_op);
                    let d_nr = self.call_stack.last().map_or(u32::MAX, |f| f.d_nr);
                    for &slot in &freed {
                        crate::keys::uaf_record_free(slot, op_pos_rt, d_nr, u16::from(op));
                    }
                }
            }
            // @PLAN53 cluster 2 / S4 — alignment invariant guard.  In aligned
            // mode the entry base is 8 and every push/pop/reserve advances by a
            // multiple of 8, so `stack_pos` must stay 8-aligned after EVERY op.
            // The first op that leaves it unaligned is the bug — name it with
            // its pc + fn instead of waiting for a distant garbage-deref SIGSEGV.
            #[cfg(feature = "stack_align_guard")]
            {
                assert_eq!(
                    self.stack_pos % 8,
                    0,
                    "S4 alignment broken: op_code={op} at pc={op_pos_rt} left \
                     stack_pos={} (not 8-aligned), fn_d_nr={}",
                    self.stack_pos,
                    self.call_stack.last().map_or(u32::MAX, |f| f.d_nr),
                );
            }
            // FY.1: frame yield — return to caller (JS requestAnimationFrame).
            if self.database.frame_yield {
                return;
            }
            #[cfg(debug_assertions)]
            {
                step += 1;
            }
            // loft#919 — a development hang guard, not a correctness one: a count cannot
            // tell a long run from a hung one, and the only signal it had for the former
            // was the wording of the latter.  Two tests of the library suite legitimately
            // ran past the old 100M ceiling, so the debug-assertions gate read as "known
            // red" for a reason that was never about those tests — and a gate read that
            // way stops being run.  The ceiling now clears the project's own suite with
            // room to spare, says what it observed rather than what it suspects, and
            // names the way to change it.
            #[cfg(debug_assertions)]
            if max_ops != 0 && step >= max_ops {
                use std::fmt::Write as _;
                let mut msg = format!(
                    "ran {max_ops} operations without finishing — this is a development \
                     guard against a hung program, not a limit on how long a program may \
                     run.  Raise or remove it with LOFT_MAX_OPS=<count|0>.  Last 16 ops:\n"
                );
                for i in 0..16usize {
                    let idx = (trail_head + i) % 16;
                    if trail_pos[idx] == u32::MAX {
                        continue;
                    }
                    let pos = trail_pos[idx];
                    let fn_nr = Self::fn_d_nr_for_pos(pos, data);
                    let (label, offset) = if fn_nr == u32::MAX {
                        ("?".to_owned(), pos)
                    } else {
                        (
                            data.def(fn_nr).name().trim_start_matches("n_").to_owned(),
                            pos - data.def(fn_nr).code_position,
                        )
                    };
                    let op_name = (0..data.definitions())
                        .find(|&d| data.def(d).op_code() == u16::from(trail_op[idx]))
                        .map_or("?", |d| data.def(d).name());
                    let _ = writeln!(msg, "  {label}+{offset}: {op_name}");
                }
                panic!("{msg}");
            }
            // Plan-07 phase 4 — typed runtime error halt.  Native fns
            // and fault-site opcodes set `database.runtime_error` then
            // signal halt by short-circuiting `code_pos` here.  The
            // outer caller (main.rs) reads `database.runtime_error`
            // after `execute_argv` returns and renders it.
            // A worker that halted raised against its own `Stores` clone, which is dropped
            // at join; re-raise it here so the WHOLE program stops, which is the decided
            // semantics for a failed assert.  Checked beside the parent's own halt because
            // this is the one place every `par` family passes through — wiring it per
            // call site left three of the four families silent (loft#1053).
            if crate::parallel::worker_fatal_pending()
                && let Some(err) = crate::parallel::take_worker_fatal()
            {
                self.database.runtime_error = Some(err);
                self.database.had_fatal = true;
            }
            self.note_runtime_error_halt();
            if self.code_pos == u32::MAX {
                break;
            }
        }

        // Fix #88: pop the synthetic entry-function frame.
        if !self.database.frame_yield {
            self.free_entry_return(&heap_ret_slots, &text_bufs);
            // The argv vector dies with the frame that was handed it.  Guarded by
            // `frame_yield` like the rest of this block: a yielded frame is still live and
            // will read its arguments again when it resumes.
            if let Some(args_vec) = argv_store {
                self.database.free(&args_vec);
            }
            self.call_stack.pop();
            self.database.parallel_ctx = None;
        }
    }

    /// #629 follow-up — free the entry fn's hidden return buffer(s) as the entry
    /// frame is torn down.
    ///
    /// `execute_argv` IS the caller of a heap-returning entry, and the ordinary
    /// contract is caller-allocates / caller-frees: at a real call site the buffer
    /// is a `__work_N` local that scope exit frees.  The entry frame is synthetic —
    /// no bytecode ever emits that free — so #629's fix, which made the buffer a
    /// real allocation instead of a null sentinel, traded a corruption for a leak:
    /// one store per run for EVERY heap aggregate return (`vector` of any element
    /// type, `struct`, data enum), plus one `String` per text return.  Bounded, but
    /// it is the entry's return value, so a long-lived host that runs many programs
    /// on one `Stores` accumulates them.
    ///
    /// Read the ref back from the slot rather than trusting the value pushed there:
    /// only a vector is pre-allocated by the caller, while a struct / data-enum body
    /// opens with its own `OpDatabase` and installs the record it allocated.
    ///
    /// Skipped when [`keep_entry_return`](Self::keep_entry_return) is set — the REPL
    /// reads the returned value off the stack AFTER this returns, so freeing here
    /// would hand it a dangling ref.
    fn free_entry_return(&mut self, heap_slots: &[u32], text_slots: &[u32]) {
        if self.keep_entry_return {
            return;
        }
        for &slot in heap_slots {
            let db = *self
                .database
                .store(&self.stack_cur)
                .addr::<DbRef>(self.stack_cur.rec, self.stack_cur.pos + slot);
            if db.store_nr != u16::MAX {
                self.free_ref_db(db);
            }
        }
        for &slot in text_slots {
            // The `String` reserved below the argument area, freed by absolute
            // offset — see `free_text_at` for why it cannot go through `free_text`.
            self.free_text_at(slot);
        }
    }

    /// #629 follow-up — declare that THIS caller will read the entry fn's return
    /// value after [`execute_argv`](Self::execute_argv) returns, so the entry frame
    /// must not free the hidden return buffer.  Ownership passes to the caller.
    ///
    /// The REPL's capture wrapper (`fn replmain_N() -> P { … }`) is the one such
    /// caller in-tree: it runs the generation and then reads the value straight off
    /// the stack.  Its `State` is a throwaway whose `Stores` is dropped immediately
    /// after, which is what makes claiming the buffer without freeing it safe there.
    pub fn keep_entry_return(&mut self) {
        self.keep_entry_return = true;
    }

    /// Check that all stores have been freed. Call after the last
    /// `execute_argv` to detect store leaks. Panics in debug builds
    /// if any stores are still alive (except the stack store).
    pub fn check_store_leaks(&self) {
        // @PLN101 Slice 0 — alloc-count harness: report total heap record allocations at
        // exit. A `value struct` program (Slice 1+) must drive its struct's contribution to
        // zero vs the reference-struct baseline. Gated so normal runs are unaffected.
        if std::env::var_os("LOFT_ALLOC_REPORT").is_some() {
            // `peak` = max LIVE stores (memory; bounded by slot reuse — a per-variable store
            // reused each loop iteration keeps this flat). `stores_allocated` = alloc/free
            // CYCLES (the per-construction abstraction cost value structs must drive to zero).
            eprintln!(
                "loft-alloc: peak={} allocs={} records={}",
                self.database.peak, self.database.stores_allocated, self.database.records_created
            );
        }
        let leaked = self.collect_store_leaks();
        // @PLN103 P3.3 — the store-timeline working-set-vs-leak summary (no-op unless
        // `LOFT_STORES=timeline`), reconciled with the authoritative leak count so the
        // interp eval-stack/const infrastructure is not false-positived as a leak.
        crate::database::timeline_summary(leaked.len());
        // @PLN104 — the text-buffer analogue: orphaned stack-frame `String`s (loft#568),
        // which the store timeline above cannot see (they are not loft stores). No-op unless
        // `LOFT_TEXT_TIMELINE`.
        crate::state::text::text_timeline_summary();
        if !leaked.is_empty() {
            let count = leaked.len();
            let preview = if count <= 5 {
                leaked.join(", ")
            } else {
                format!("{} ... and {} more", leaked[..5].join(", "), count - 5)
            };
            let msg = format!("{count} stores not freed at program exit: {preview}");
            // @PLN130 F8 — under LOFT_STRICT_STORES a store that is never freed is an
            // ERROR, not a warning. It is the other half of the same question: strict mode
            // asks that every store be freed exactly once, so both "freed then used" and
            // "never freed" have to fail, or a probe could pass by leaking.
            if crate::keys::strict_stores() {
                eprintln!("[strict-store] NEVER FREED: {msg}");
                crate::keys::strict_store_leaks(count);
            } else {
                eprintln!("Warning: {msg}");
            }
            // LOFT_LEAK_SITES — group leaked stores by ALLOCATION site (created_at →
            // source line) so the leak's where-from is named, not just its type. Gated.
            if std::env::var_os("LOFT_LEAK_SITES").is_some() {
                let mut by_site: std::collections::BTreeMap<(u32, u16, bool), usize> =
                    std::collections::BTreeMap::new();
                for (s_nr, s) in self.database.allocations.iter().enumerate() {
                    if (s_nr == 0 && self.database.stack_store_at_zero)
                        || s.is_locked()
                        || self.const_refs.iter().any(|cr| cr.store_nr == s_nr as u16)
                        || s.free
                    {
                        continue;
                    }
                    *by_site
                        .entry((s.created_at, s.known_type, s.is_free_protected()))
                        .or_default() += 1;
                }
                let mut sites: Vec<_> = by_site.into_iter().collect();
                sites.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
                for ((created_at, kt, protd), n) in sites {
                    let line = self
                        .line_numbers
                        .range(..=created_at)
                        .next_back()
                        .map_or(0, |(_, &v)| v);
                    let tn = self
                        .database
                        .types
                        .get(kt as usize)
                        .map_or("?", |t| t.name.as_str());
                    eprintln!(
                        "  [leak-site] {n}× {tn} (kt={kt}) allocated at pc={created_at} (line {line}) \
                         free_protected={protd}"
                    );
                }
            }
        }
    }

    /// @PLN140 arc B/C — arm the loft-level sampler from the environment, if it is
    /// asked for. Idempotent, and a no-op when a debug session already owns the run
    /// (the two would fight over the same per-op hook, and the debugger was asked for
    /// explicitly).
    pub fn arm_profiler(&mut self) {
        if self.debug.as_deref().is_some_and(|d| d.prof.is_some()) {
            return;
        }
        let prof = crate::profiler::Profiler::from_env();
        if prof.is_none() && !crate::net_profile::enabled() {
            return;
        }
        // The network profiler has no per-op work of its own, but its report is reached
        // the same way the CPU one is — from the execute loop — so a run with only
        // `LOFT_NET_PROFILE` set still needs the `Debugger` the hook hangs off.
        if prof.is_none() {
            if self.debug.is_none() {
                self.debug = Some(Box::default());
            }
            crate::profiler::install_signal_flush();
            return;
        }
        let Some(prof) = prof else { return };
        if self.debug.is_none() {
            self.debug = Some(Box::default());
        }
        if let Some(d) = self.debug.as_deref_mut() {
            d.prof = Some(Box::new(prof));
        }
        // loft#1089 — armed, so a program with no clean shutdown can still be asked for
        // its profile: `SIGUSR1` dumps and continues, `SIGINT`/`SIGTERM` dump and leave.
        // Only here, so an unprofiled run's shutdown behaviour is untouched.
        crate::profiler::install_signal_flush();
    }

    /// @PLN140 arc B/C — what this run spent its time on, and what path reached each
    /// allocation. Silent unless [`arm_profiler`](Self::arm_profiler) armed it.
    ///
    /// The single-run convenience over [`fold_profile`](Self::fold_profile): a run
    /// that IS the whole program has nothing to merge with.
    pub fn report_profile(&self, data: &crate::data::Data) {
        let mut totals = crate::profiler::Totals::default();
        self.fold_profile(data, &mut totals);
        totals.report();
    }

    /// @PLN140 arc B/C — resolve this run's samples against `data` and add them to
    /// `totals`. A no-op unless [`arm_profiler`](Self::arm_profiler) armed it.
    ///
    /// Resolution has to happen HERE, per run, because a `pc` only means something
    /// inside the `Data` it was compiled from — and a test run compiles a fresh one
    /// per test function (loft#860). `totals` therefore accumulates
    /// `(function, file:line)` strings, never positions.
    pub fn fold_profile(&self, data: &crate::data::Data, totals: &mut crate::profiler::Totals) {
        let Some(prof) = self.debug.as_deref().and_then(|d| d.prof.as_ref()) else {
            return;
        };
        totals.add_run(prof);
        if prof.cpu_armed() {
            for (pc, site) in prof.sites_ranked() {
                let (func, place) = self.site_label(data, pc);
                totals.add_site(&func, &place, site);
            }
            for (chain, site) in prof.paths_ranked() {
                totals.add_path(&Self::chain_label(data, &chain), site);
            }
        }
        if prof.alloc_armed() {
            for ((pc, chain), (n, stores)) in prof.alloc_paths_ranked() {
                let (func, place) = self.site_label(data, pc);
                totals.add_alloc(&func, &place, &Self::chain_label(data, &chain), n, stores);
            }
        }
    }

    /// A call chain rendered outermost-first, with a leading `…` when frames above
    /// the retained window were dropped.
    fn chain_label(data: &crate::data::Data, chain: &[u32]) -> String {
        if chain.is_empty() {
            return "(no loft frame)".to_string();
        }
        let names: Vec<String> = chain
            .iter()
            .map(|&d| {
                if d == u32::MAX || d as usize >= data.definitions.len() {
                    "<worker>".to_string()
                } else {
                    let n = &data.def(d).name;
                    n.strip_prefix("n_").unwrap_or(n).to_string()
                }
            })
            .collect();
        let prefix = if chain.len() == crate::profiler::PATH_DEPTH {
            "… → "
        } else {
            ""
        };
        format!("{prefix}{}", names.join(" → "))
    }

    /// @PLN140 arc A — where the heap went: live store bytes at the run's **peak**,
    /// grouped by the loft line that allocated them.  Silent unless
    /// `LOFT_ALLOC_SITES` is set.
    ///
    /// Three things separate this from the reports it grew out of, and each was a
    /// way the old one answered a question nobody asked:
    ///
    /// * **Live stores, not leaked ones.** `LOFT_LEAK_SITES` groups by the same key
    ///   but over what was *never freed*, so a program that frees everything — the
    ///   normal case — gets an empty report however much memory it used.
    /// * **At the peak, not at exit.** Everything else fires after the run, by which
    ///   time the peak is long over: a program that peaks at 1.5 GiB and exits at
    ///   10 MB has nothing left to report.
    /// * **Bytes, not store counts.** `LOFT_ALLOC_REPORT` counts allocations, which
    ///   weighs one 40 MiB vector the same as one 32-byte record.
    ///
    /// The capture is taken at most of the peak rather than exactly at it (see
    /// [`peak_sites`](crate::store_budget::peak_sites)), and the banner says which —
    /// a table that silently described a different moment than its headline number is
    /// exactly the plausible-looking wrong answer this instrument exists to refuse.
    pub fn report_alloc_sites(&self, data: &crate::data::Data) {
        if !crate::store_budget::sites_armed() {
            return;
        }
        use crate::store_budget::human;
        let (peak, captured_at, mut rows) = crate::store_budget::peak_sites();
        if rows.is_empty() {
            eprintln!(
                "[alloc-sites] this run held no store heap ({}).",
                human(peak)
            );
            return;
        }
        rows.sort_by_key(|&(pc, kt, bytes, _)| (std::cmp::Reverse(bytes), pc, kt));
        let held: u64 = rows.iter().map(|&(_, _, b, _)| b).sum();
        #[allow(clippy::cast_precision_loss)] // display only
        let pct = if peak == 0 {
            100.0
        } else {
            captured_at as f64 * 100.0 / peak as f64
        };
        eprintln!(
            "\n════ allocation hot spots — peak {}, captured at {} ({pct:.0} % of peak) ════",
            human(peak),
            human(captured_at)
        );
        // Every site at pc 0 means nothing stamped one: the native backend never
        // publishes a bytecode position, so a table of `line 0` would be a table of
        // nothing wearing a report's clothes.
        if rows.iter().all(|&(pc, _, _, _)| pc == 0) {
            eprintln!(
                "  Nothing here carries an allocation site, so there is nothing to attribute:\n  \
                 every byte was taken either before the first op ran (the interpreter's own\n  \
                 stores) or under --native, where the dispatch loop that publishes `alloc_pc`\n  \
                 never runs. Total held at the peak: {}",
                human(held)
            );
            return;
        }
        const TOP: usize = 12;
        for &(pc, kt, bytes, stores) in rows.iter().take(TOP) {
            let tn = self
                .database
                .types
                .get(kt as usize)
                .map_or("?", |t| t.name.as_str());
            let (func, place) = self.site_label(data, pc);
            let plural = if stores == 1 { "store" } else { "stores" };
            eprintln!(
                "  {:>10}  {stores:>6} {plural:<6}  {tn:<24} {func:<20} {place}",
                human(bytes)
            );
        }
        if rows.len() > TOP {
            let rest: u64 = rows[TOP..].iter().map(|&(_, _, b, _)| b).sum();
            eprintln!(
                "  … and {} more sites holding {}",
                rows.len() - TOP,
                human(rest)
            );
            // Roll up by function once the site list stops fitting on a screen — the
            // question at that size is which routine to look at, not which line.
            let mut by_fn: std::collections::BTreeMap<String, (u64, u32)> =
                std::collections::BTreeMap::new();
            for &(pc, _, bytes, stores) in &rows {
                let e = by_fn.entry(self.site_label(data, pc).0).or_insert((0, 0));
                e.0 += bytes;
                e.1 += stores;
            }
            let mut fns: Vec<_> = by_fn.into_iter().collect();
            fns.sort_by_key(|(name, (b, _))| (std::cmp::Reverse(*b), name.clone()));
            eprintln!("  ── rolled up by function ──");
            for (name, (bytes, stores)) in fns.into_iter().take(TOP) {
                eprintln!("  {:>10}  {stores:>6} stores  {name}", human(bytes));
            }
        }
        eprintln!(
            "  (text buffers are Rust Strings, not stores — this ledger does not count them)"
        );
    }

    /// The loft function and `file:line` that bytecode position `pc` belongs to, for
    /// a report that has to point somewhere a reader can open.
    ///
    /// `line_numbers` is the dense per-run table, so an arbitrary pc resolves to the
    /// line that was executing — unlike the call-site span table, whose nearest entry
    /// below an arbitrary pc is routinely in an unrelated function.
    fn site_label(&self, data: &crate::data::Data, pc: u32) -> (String, String) {
        // pc 0 is the "never stamped" sentinel, not a position: resolving it lands on
        // whichever function happens to start the bytecode, which reads as a real
        // answer and is not one.
        if pc == 0 {
            return (
                "(runtime)".to_string(),
                "no site — before the first op".into(),
            );
        }
        let d_nr = Self::fn_d_nr_for_pos(pc, data);
        if d_nr == u32::MAX {
            let line = self
                .line_numbers
                .range(..=pc)
                .next_back()
                .map_or(0, |(_, &v)| v);
            return ("?".to_string(), format!("pc={pc} (line {line})"));
        }
        let def = data.def(d_nr);
        let name = def.name.strip_prefix("n_").unwrap_or(&def.name).to_string();
        let file = def.position.file.rsplit('/').next().unwrap_or("?");
        (
            name,
            format!("{file}:{}", self.line_at_in_fn(pc, d_nr, data)),
        )
    }

    /// The source line at bytecode position `pc`, scoped to the function that owns it.
    ///
    /// An unscoped `line_numbers.range(..=pc).next_back()` is wrong at the top of a
    /// function: entries land *after* the frame-setup ops, so a `pc` in the prologue
    /// has no entry at or below it inside its own body and picks up the last line of
    /// whichever function precedes it in the bytecode. That is not a rounding error —
    /// it names a different function's line, in a different file, with no sign that
    /// anything went wrong. @PLN140 arc C found it by reporting `make` at a line in
    /// `hot`, and it is the shape a store allocated by a prologue always takes.
    fn line_at_in_fn(&self, pc: u32, d_nr: u32, data: &crate::data::Data) -> u32 {
        let def = data.def(d_nr);
        let (start, end) = (def.code_position, def.code_position + def.code_length);
        self.line_numbers
            .range(start..=pc)
            .next_back()
            // Before the body's first mapped op — the prologue — so the body's first
            // line is the honest answer.
            .or_else(|| self.line_numbers.range(start..end).next())
            .map_or(0, |(_, &v)| v)
    }

    /// Collect a description for every leaked store at program exit
    /// (same filtering as `check_store_leaks`: skip stack store 0,
    /// locked constants, and `const_refs`).  Used by tests that need
    /// to assert leak-free without driving `execute_log`'s full trace
    /// machinery (which can hang on certain multi-fn iterations under
    /// rustc 1.95.0+).  The leak warning's `eprintln!` path is built
    /// on top of this helper.
    #[must_use]
    pub fn collect_store_leaks(&self) -> Vec<String> {
        // @P317 — aggregate leaked stores BY TYPE, most-leaked first, so the
        // exit warning names the culprit (`kt=68 ChunkKey×6026`) instead of a
        // truncated list of store numbers.  Mirrors `Stores::collect_store_leaks`
        // (uses State's own `const_refs` filter, which differs from the
        // database's).
        let mut by_type: std::collections::BTreeMap<(u16, &str), usize> =
            std::collections::BTreeMap::new();
        for (s_nr, s) in self.database.allocations.iter().enumerate() {
            if s_nr == 0 && self.database.stack_store_at_zero {
                continue; // stack store — always alive
            }
            if s.is_locked() || self.const_refs.iter().any(|cr| cr.store_nr == s_nr as u16) {
                continue;
            }
            if !s.free {
                let tn = self
                    .database
                    .types
                    .get(s.known_type as usize)
                    .map_or("?", |t| t.name.as_str());
                *by_type.entry((s.known_type, tn)).or_default() += 1;
            }
        }
        let mut leaked: Vec<((u16, &str), usize)> = by_type.into_iter().collect();
        leaked.sort_by_key(|&(_, n)| std::cmp::Reverse(n));
        leaked
            .into_iter()
            .map(|((kt, tn), n)| format!("kt={kt} {tn}×{n}"))
            .collect()
    }

    /// FY.2: Resume execution after a frame yield.  Returns `true` while the
    /// program is still running, `false` when it finishes.
    /// @PLN18 02 (the heart's aorta) — the compiled→interp RE-ENTRY thunk:
    /// call one interpreted function from HOST code (generated native code,
    /// the debugger, a test) over the live State, between or inside frames.
    ///
    /// Contract: the caller has pushed the args in declaration order via
    /// [`put_stack`](Self::put_stack) (scalars as `i64`, records/vectors as
    /// `DbRef`, integer null = `i64::MIN`).  `reenter` pushes the synthetic
    /// return address, jumps to the callee, runs it to completion, and
    /// restores the PC and stack watermark — the surrounding execution (a
    /// paused `main`, a yielded frame loop) continues unperturbed.
    ///
    /// v1 bounds: the callee must not yield (asserted) and stack-returning
    /// results are not retrieved (store-writing callees — the shared store
    /// is the ABI; a result record arg is the supported result path).
    ///
    /// # Panics
    /// When the callee yields mid-call (not supported in v1).
    /// `reenter` for a value-returning callee: after completion the result
    /// sits at the synthetic frame's BASE (`copy_result` copies it to
    /// `fn_stack` = the base and leaves `stack_pos = base + step(size)`).
    /// Read it before restoring the watermark.
    ///
    /// # Panics
    /// When the callee yields mid-call (not supported in v1).
    pub fn reenter_ret<T: Copy + 'static>(
        &mut self,
        d_nr: u32,
        code_position: u32,
        push_args: impl FnOnce(&mut Self),
    ) -> T {
        let saved_pos = self.code_pos;
        let saved_sp = self.stack_pos;
        let base = self.stack_high.next_multiple_of(8);
        self.stack_pos = base;
        push_args(self);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: base,
            args_size: 0,
            line: 0,
        });
        self.put_stack(u32::MAX);
        self.code_pos = code_position;
        let yielded = self.resume();
        assert!(!yielded, "reenter_ret: the callee yielded mid-call");
        let result = *self
            .database
            .store(&self.stack_cur)
            .addr::<T>(self.stack_cur.rec, self.stack_cur.pos + base);
        self.code_pos = saved_pos;
        self.stack_pos = saved_sp;
        result
    }

    ///
    /// # Panics
    /// When the callee yields mid-call (not supported in v1).
    /// @PLN18 08-S7 — `reenter` under the debugger: drive the call with
    /// [`debug_step`](Self::debug_step) so registered breakpoints SUSPEND it;
    /// each suspension hands the live `&mut State` plus the captured frame to
    /// `on_pause` (called BETWEEN resume steps — the legal aliasing seam),
    /// which blocks until the debugger resumes.  `T = ()` serves void callees
    /// (a zero-sized read at the frame base).
    ///
    /// # Panics
    /// When the callee yields mid-call (not supported under a dispatch).
    pub fn reenter_dbg<T: Copy + 'static>(
        &mut self,
        d_nr: u32,
        code_position: u32,
        data: &crate::data::Data,
        push_args: impl FnOnce(&mut Self),
        mut on_pause: impl FnMut(&mut Self, &crate::data::Data, &crate::debugger::BreakHit),
    ) -> T {
        let saved_pos = self.code_pos;
        let saved_sp = self.stack_pos;
        let base = self.stack_high.next_multiple_of(8);
        self.stack_pos = base;
        push_args(self);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: base,
            args_size: 0,
            line: 0,
        });
        self.put_stack(u32::MAX);
        self.code_pos = code_position;
        // An entry breakpoint sits exactly where this call STARTS —
        // `debug_step`'s first-op skip (correct when resuming FROM a pause)
        // would silently step over it, so check the entry explicitly.
        if self
            .debug
            .as_ref()
            .is_some_and(|d| d.is_breakpoint(self.code_pos))
        {
            let hit = self.capture_break_frame(self.code_pos, data);
            on_pause(self, data, &hit);
        }
        loop {
            let suspended = self.debug_step(crate::debugger::StepMode::Continue, data);
            if !suspended {
                break;
            }
            assert!(
                !self.database.frame_yield,
                "reenter_dbg: the callee yielded mid-call"
            );
            let Some(hit) = self.debug.as_mut().and_then(|d| d.paused.take()) else {
                break;
            };
            on_pause(self, data, &hit);
        }
        let result = *self
            .database
            .store(&self.stack_cur)
            .addr::<T>(self.stack_cur.rec, self.stack_cur.pos + base);
        self.code_pos = saved_pos;
        self.stack_pos = saved_sp;
        result
    }

    /// @PLN18 08-S7 — register a breakpoint at the CURRENT body of `d_nr`,
    /// resolved through `fn_positions` (the live dispatch table) rather than
    /// `data.def().code_position`: after a tier-0 reload the body moved to the
    /// appended bytecode and the def's recorded position is stale.  This is
    /// the re-resolution primitive — breakpoint identity is the FN, offsets
    /// move.  Returns the resolved offset.
    pub fn set_breakpoint_fn_current(&mut self, d_nr: u32) -> Option<u32> {
        let start = *self.fn_positions.get(d_nr as usize)?;
        let &offset = self
            .line_numbers
            .range(start..)
            .next()
            .map(|(off, _)| off)?;
        self.enable_debug();
        if let Some(dbg) = self.debug.as_mut() {
            dbg.add_offset(offset);
        }
        Some(offset)
    }

    /// Re-enter the interpreter for one call over the live State (the 02
    /// frame contract; see `reenter_ret` for the value-returning form).
    ///
    /// # Panics
    /// When the callee yields mid-call (not supported in v1).
    pub fn reenter(&mut self, d_nr: u32, code_position: u32, push_args: impl FnOnce(&mut Self)) {
        let saved_pos = self.code_pos;
        let saved_sp = self.stack_pos;
        // The synthetic frame starts ABOVE the high-water mark, never at the
        // transient eval height: live variable slots of the paused frames sit
        // between `stack_pos` and `stack_high` (the frame contract, mapped by
        // tests/dispatch_reentry.rs — stomping them corrupts Strings whose
        // later drop aborts).  Args are pushed via the closure AFTER the lift,
        // so they land above the mark too.
        let base = self.stack_high.next_multiple_of(8);
        self.stack_pos = base;
        push_args(self);
        // Balance: the callee's return pops a CallFrame unconditionally
        // (`fn_return`) — without this push the FIRST re-entry pops the
        // paused program's own frame (probe-caught: heap corruption at
        // teardown after 200k imbalanced pops).
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: base,
            args_size: 0,
            line: 0,
        });
        self.put_stack(u32::MAX);
        self.code_pos = code_position;
        let yielded = self.resume();
        assert!(!yielded, "reenter: the callee yielded mid-call");
        self.code_pos = saved_pos;
        self.stack_pos = saved_sp;
    }

    pub fn resume(&mut self) -> bool {
        self.database.frame_yield = false;
        let bytecode_len = self.bytecode.len() as u32;
        while self.code_pos < bytecode_len {
            let op = self.code::<u8>();
            if op == 255 {
                let ext = self.code::<u8>();
                OPERATORS[255 + ext as usize](self);
            } else {
                OPERATORS[op as usize](self);
            }
            if self.database.frame_yield {
                return true; // yielded again — still running
            }
            // Plan-07 phase 4 — typed runtime error halt (mirrors
            // execute_argv).  Resume path needs the same check so a
            // post-yield panic / failed assert halts gracefully.
            self.note_runtime_error_halt();
            if self.code_pos == u32::MAX {
                break;
            }
        }
        // Loop finished — pop the synthetic frame (the entry frame for the
        // frame-yield drivers, `reenter`'s push for a re-entered call).
        // `parallel_ctx` is NOT cleared here: its lifetime is the PROGRAM's
        // (who wired it unwires it — `execute_argv` / the frame-yield
        // drivers).  Clearing on every loop completion tore down the
        // standing ctx the live-dispatch host wired, so the SECOND call of
        // a flipped fn using `par_*` panicked in `n_parallel_*`'s expect
        // (probe: a flipped `par_fold` killed the kernel on ping 2).
        self.call_stack.pop();
        false
    }

    /// Resume from a suspension and stop per `mode` — the @PLN16 F step verbs
    /// ([`StepMode`](crate::debugger::StepMode)).  Drives the same re-enterable
    /// loop as [`resume`](Self::resume) but, after each op, decides whether to pause
    /// from the current **source line** (`line_at`, the dense per-line table) and
    /// **call depth** (`call_stack.len()`) relative to where the step began.  A
    /// registered breakpoint always pauses, whatever the mode.  Returns `true` if it
    /// paused again (frame in [`paused_frame`](Self::paused_frame)), `false` if the
    /// program finished.
    pub fn debug_step(
        &mut self,
        mode: crate::debugger::StepMode,
        data: &crate::data::Data,
    ) -> bool {
        use crate::debugger::StepMode;
        // @PLN63 RX2 — when reverse-stepping is armed, checkpoint the pre-step state before
        // executing anything, so `step_back` can restore it.  Skipped (nothing to reverse to)
        // when a durable store makes the heap non-snapshotable.
        if self.debug.as_ref().is_some_and(|d| d.reverse)
            && let Some(cp) = self.snapshot_checkpoint()
            && let Some(d) = self.debug.as_mut()
        {
            // RX3 — bounded ring: a push past the cap drops the oldest step.
            if d.reverse_cap > 0 && d.reverse_ring.len() >= d.reverse_cap {
                d.reverse_ring.pop_front();
            }
            d.reverse_ring.push_back(cp);
        }
        self.database.frame_yield = false;
        let start_line = self.line_at(self.code_pos);
        let start_depth = self.call_stack.len();
        if let Some(d) = self.debug.as_mut() {
            d.paused = None;
            // The undo/redo history survives a step; only an edit still in flight
            // (armed, not committed) is dropped, because a half-recorded edit has no
            // meaning at the next pause.
            //
            // Stepping reuses frame slots, so an entry MAY become stale — but most do
            // not: a long-lived accumulator keeps its slot for the whole function, and
            // a heap edit's address survives every step.  So each entry is checked at
            // the new pause by `validate_undo_history` rather than all of them being
            // assumed dead.
            d.recording_edit = None;
            // @PLN16 M3 — a fresh resume reports only watch hits it produces.
            d.last_watch = None;
            // Only THIS resume's drops are worth reporting.
            d.dropped_undo.clear();
        }
        let bytecode_len = self.bytecode.len() as u32;
        // Skip the pause-check on the very first op — it is the breakpoint/line we
        // are stepping *from*; we must execute it, not immediately re-pause.
        let mut first = true;
        // @PLN16 M3 — set when a watchpoint's region changed after an op; the next stop
        // check pauses, so the user lands one op past the mutating write.
        let mut watch_fired = false;
        while self.code_pos < bytecode_len {
            if !first {
                let pc = self.code_pos;
                let at_bp = self.debug.as_ref().is_some_and(|d| d.is_breakpoint(pc));
                let stop = at_bp || watch_fired || {
                    let depth = self.call_stack.len();
                    let line = self.line_at(pc);
                    match mode {
                        StepMode::Into => depth != start_depth || line != start_line,
                        StepMode::Over => depth <= start_depth && line != start_line,
                        StepMode::Out => depth < start_depth,
                        StepMode::Continue => false,
                    }
                };
                if stop {
                    let hit = self.capture_break_frame(pc, data);
                    if let Some(d) = self.debug.as_mut() {
                        d.paused = Some(hit);
                    }
                    // @PLN120 F — the step landed: decide which undo entries the new
                    // frame still owns, rather than having dropped them all on the way
                    // in.  Here, not at the clear above, because the verdict needs the
                    // pc we stopped AT.
                    self.validate_undo_history(data);
                    return true;
                }
            }
            first = false;
            let op = self.code::<u8>();
            if op == 255 {
                let ext = self.code::<u8>();
                OPERATORS[255 + ext as usize](self);
            } else {
                OPERATORS[op as usize](self);
            }
            if self.database.frame_yield {
                return true;
            }
            self.note_runtime_error_halt();
            if self.code_pos == u32::MAX {
                break;
            }
            // @PLN16 M3 — poll watchpoints after each op; the first change arms a stop.
            if !watch_fired && let Some(hit) = self.poll_watchpoints() {
                if let Some(d) = self.debug.as_mut() {
                    d.last_watch = Some(hit);
                }
                watch_fired = true;
            }
        }
        // Same contract as `resume`: pop the synthetic frame only —
        // `parallel_ctx` is program-scoped and unwired by its wirer.
        self.call_stack.pop();
        false
    }

    /// Snapshot the bytecode, text segment, and native-function library for
    /// use in a parallel worker thread.  All three are `Arc`-cloned — O(1).
    #[must_use]
    pub fn worker_program(&self) -> crate::parallel::WorkerProgram {
        // Resolve n_stack_trace now so workers can call stack_trace() (fix #92).
        let stack_trace_lib_nr = self
            .library_names
            .get("n_stack_trace")
            .copied()
            .unwrap_or(u16::MAX);
        crate::parallel::WorkerProgram {
            bytecode: Arc::clone(&self.bytecode),
            library: Arc::clone(&self.library),
            stack_trace_lib_nr,
            data_ptr: self.data_ptr,
            fn_positions: Arc::new(self.fn_positions.clone()),
            line_numbers: Arc::new(self.line_numbers.clone()),
        }
    }

    /// Create a `State` for use in a parallel worker thread.
    ///
    /// `worker` must be produced by [`Stores::clone_for_light_worker`]; the
    /// `WorkerStores` newtype is the compile-time proof of that invariant (S30).
    /// This call allocates a fresh stack store at the next available index.
    #[must_use]
    pub fn new_worker(
        worker: WorkerStores,
        bytecode: Arc<Vec<u8>>,
        library: Arc<Vec<Call>>,
    ) -> State {
        let mut db = worker.stores;
        let stack_cur = db.database(1000);
        // @PLN154 — a worker runs the same bytecode on its own frame, so it gets its own
        // shadow; without this the detector would be blind to exactly the `par` arms.
        if crate::stack_verify::enabled() {
            db.store_mut(&stack_cur).arm_init_shadow();
        }
        let stack_cap_bytes = db.store(&stack_cur).byte_capacity() as u32;
        State {
            stack_cur,
            stack_pos: 4,
            stack_high: 4,
            stack_cap_bytes,
            verify_on: crate::stack_verify::enabled(),
            code_pos: 0,
            def_pos: 0,
            source: u16::MAX,
            database: db,
            arguments: 0,
            bytecode,
            library,
            library_names: HashMap::new(),
            native_stub_symbols: std::collections::HashSet::new(),
            stack: HashMap::new(),
            vars: HashMap::new(),
            calls: HashMap::new(),
            types: HashMap::new(),
            text_positions: BTreeSet::new(),
            line_numbers: BTreeMap::new(),
            scope_spans: Vec::new(),
            store_spans: Vec::new(),
            source_spans: BTreeMap::new(),
            published_spans: None,
            entered_fns: None,
            fn_positions: Vec::new(),
            debug: None,
            call_stack: Vec::new(),
            fnref_bufs: Vec::new(),
            fnref_calls: Vec::new(),
            data_ptr: std::ptr::null(),
            stack_trace_lib_nr: u16::MAX,
            coroutines: vec![None],
            coroutine_generation: 1,
            active_coroutines: Vec::new(),
            generate_depth: 0,
            #[cfg(debug_assertions)]
            in_call_arg: false,
            parallel_n_arms: 0,
            parallel_arm_positions: Vec::new(),
            const_refs: Vec::new(),
            keep_entry_return: false,
        }
    }

    /// @PLN133 S8 — run a loft function to completion from INSIDE an opcode
    /// handler, and answer the integer it returned.
    ///
    /// This is what lets a lazy fetch be loft code rather than Rust. `Stores`
    /// cannot run a loft function; `State` can, which is why @PLN133 S8 lifts
    /// the miss-then-fetch decision out of `Stores` into its two callers — the
    /// `&mut Stores` borrow has to end before this runs.
    ///
    /// **It is the ORDINARY call machinery, not a second one.** `fn_call`
    /// pushes the frame and stores the return address exactly as a `Call` op
    /// does, and the loop below runs until that frame pops — so the callee
    /// returns through the same path every other call uses, and the outer
    /// frame's locals are untouched because nothing resets the stack. The
    /// `execute_at*` family cannot serve here: each of those RESETS `stack_pos`
    /// for a fresh par worker, which is correct there and would discard the
    /// caller's frame here.
    ///
    /// **A fault inside the callee is CONTAINED, not propagated.** For an
    /// ordinary call, propagating is right. For a fetch it is not: @PLN129's
    /// contract is C80 — a failed fetch reports through `store_lazy_error` and
    /// the lookup answers null — so a buggy source function must not turn a
    /// lookup into a program halt. The reason comes back as `Err`, the outer
    /// program keeps its own fault slot, and execution continues.
    ///
    /// # Errors
    /// When the definition has no bytecode, when the call could not be entered
    /// (a stack that is already at its depth limit), or when the callee raised —
    /// the string is what arc C reports.
    pub fn run_until_return(&mut self, d_nr: u32, args: &[LoftArg]) -> Result<i64, String> {
        let Some(&to) = self.fn_positions.get(d_nr as usize) else {
            return Err(format!("definition {d_nr} has no bytecode to run"));
        };
        let depth = self.call_stack.len();
        let saved_code_pos = self.code_pos;
        let saved_stack_pos = self.stack_pos;
        // The OUTER program's fault slot is set aside for the duration. Without
        // this the containment below would swallow a fault the caller was
        // already carrying, which is the opposite of what it is for.
        let outer_error = self.database.runtime_error.take();
        let outer_fatal = self.database.had_fatal;

        let args_base = self.stack_pos;
        for a in args {
            match a {
                LoftArg::Int(v) => self.put_stack(*v),
                LoftArg::Ref(r) => self.put_stack(*r),
                // A `Str` BORROWS its bytes, so the caller keeps the backing
                // string alive across the call — the same contract
                // `execute_at_raw_text_input` has for a par worker's text
                // argument, and the reason `LoftArg` carries a `&str`.
                LoftArg::Text(s) => self.put_stack(Str::new(s)),
            }
        }
        let args_size = (self.stack_pos - args_base) as u16;
        self.fn_call(d_nr, args_size, i64::from(to));
        if self.call_stack.len() == depth {
            // `fn_call` refused: the depth limit was already reached, and it
            // raised rather than pushing. Restore and report — a fetch that
            // cannot be entered is unreachable, not absent.
            self.database.runtime_error = outer_error;
            self.database.had_fatal = outer_fatal;
            self.stack_pos = saved_stack_pos;
            self.code_pos = saved_code_pos;
            return Err("the call stack is too deep to run a fetch here".to_string());
        }

        let bytecode_len = self.bytecode.len() as u32;
        while self.call_stack.len() > depth && self.code_pos < bytecode_len {
            let op = self.code::<u8>();
            if op == 255 {
                let ext = self.code::<u8>();
                OPERATORS[255 + ext as usize](self);
            } else {
                OPERATORS[op as usize](self);
            }
            if self.database.runtime_error.is_some() || self.code_pos == u32::MAX {
                break;
            }
        }

        let inner_error = self.database.runtime_error.take();
        self.database.runtime_error = outer_error;
        self.database.had_fatal = outer_fatal;
        if let Some(err) = inner_error {
            // Contained. The frames the callee left behind are dropped and the
            // caller's position restored, so the outer program continues with
            // every local intact.
            //
            // **What it does NOT do is release what those frames held.** A
            // frame's locals are freed by the scope-exit bytecode the fault
            // skipped, so there is no runtime teardown to run here, and
            // truncating abandons whatever the aborted callee had allocated
            // (@PLN133 P4 measured exactly one store per contained fault). A
            // traversal over an unreachable source therefore leaks once per
            // failed fetch — which is the long-running case arc C's sticky
            // counter exists for, so it is a real debt rather than a cosmetic
            // one, and it is recorded as such rather than papered over.
            self.call_stack.truncate(depth);
            self.stack_pos = saved_stack_pos;
            self.code_pos = saved_code_pos;
            return Err(format!("{}: {}", err.kind.label(), err.message));
        }

        let result = *self.get_stack::<i64>();
        self.stack_pos = saved_stack_pos;
        self.code_pos = saved_code_pos;
        Ok(result)
    }

    /// Execute the bytecode function at `fn_pos` passing one `DbRef` argument,
    /// then return the `i32` result left on the stack.
    ///
    /// Stack layout built here:
    /// ```text
    ///   [arg: DbRef (12 bytes)][return-addr u32::MAX (4 bytes)]
    /// ```
    /// This matches what `fn_return(ret=12, value=4, discard=D)` expects.
    ///
    /// # Panics
    /// Panics if the worker executes more than 10 000 000 operations (infinite-loop guard).
    pub fn execute_at(&mut self, fn_pos: u32, arg: &DbRef) -> i64 {
        // Fix #92: propagate data_ptr, stack_trace_lib_nr, and fn_positions from
        // ParallelCtx so that stack_trace() works inside parallel workers called via
        // the n_parallel_for / _light dispatch.  When parallel_ctx is None (direct
        // run_parallel_* path), stack_trace_lib_nr is already set by
        // WorkerProgram::new_state — don't clobber it.
        if let Some(ctx) = &self.database.parallel_ctx {
            self.data_ptr = ctx.data;
            self.stack_trace_lib_nr = ctx.stack_trace_lib_nr;
            if self.fn_positions.is_empty() && !ctx.data.is_null() {
                let data = unsafe { &*ctx.data };
                self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
            }
        }
        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: self.stack_step(4),
            args_size: 12,
            line: 0,
        });
        self.stack_pos = self.stack_step(4); // @PLAN53 2j: stepped par-worker entry base (guard-clean; identity flag-OFF)
        self.put_stack(*arg); // 12 bytes → stack_pos = 16
        self.put_stack(u32::MAX); // 4 bytes  → stack_pos = 20
        self.code_pos = fn_pos;
        self.run_to_return(Self::WORKER_OP_CEILING);
        *self.get_stack::<i64>()
    }

    /// Execute a worker function at `fn_pos`, return raw result bits as `u64`.
    pub fn execute_at_raw(
        &mut self,
        fn_pos: u32,
        arg: &DbRef,
        extra_args: &[u64],
        return_size: u32,
    ) -> u64 {
        if let Some(ctx) = &self.database.parallel_ctx {
            self.data_ptr = ctx.data;
            self.stack_trace_lib_nr = ctx.stack_trace_lib_nr;
            if self.fn_positions.is_empty() && !ctx.data.is_null() {
                let data = unsafe { &*ctx.data };
                self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
            }
        }
        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: self.stack_step(4),
            args_size: 12,
            line: 0,
        });
        self.stack_pos = self.stack_step(4); // @PLAN53 2j: stepped par-worker entry base (guard-clean; identity flag-OFF)
        // Push extra context args first (they precede the element arg in the
        // function's parameter list: fn worker(element, extra1, extra2, ...)).
        // The stack grows upward; the function reads params from low to high offset.
        // Element arg (DbRef) occupies the first parameter slot; extras follow.
        self.put_stack(*arg); // 12 bytes
        for &extra in extra_args {
            // Push each extra as a raw i64 (integer context args, post-2c).
            self.put_stack(extra as i64);
        }
        self.put_stack(u32::MAX); // return address sentinel
        self.code_pos = fn_pos;
        self.run_to_return(Self::WORKER_OP_CEILING);
        match return_size {
            8 => *self.get_stack::<u64>(),
            1 => u64::from(*self.get_stack::<u8>()),
            _ => u64::from(*self.get_stack::<u32>()),
        }
    }

    /// Plan-06 phase 1 G2 — primitive-input worker dispatch.
    /// Same as `execute_at_raw` but pushes a primitive value of
    /// `input_size` bytes (1, 4, or 8) instead of a 12-byte
    /// `DbRef`, so workers whose first param is a primitive type
    /// (bool/byte/integer/single/float/character/non-payload enum)
    /// see the actual element value in slot 0 instead of a ref to
    /// the row record.  Used by `run_parallel_*` when the dispatch
    /// detects a primitive input.
    pub fn execute_at_raw_primitive_input(
        &mut self,
        fn_pos: u32,
        input_value: u64,
        input_size: u32,
        extra_args: &[u64],
        return_size: u32,
    ) -> u64 {
        if let Some(ctx) = &self.database.parallel_ctx {
            self.data_ptr = ctx.data;
            self.stack_trace_lib_nr = ctx.stack_trace_lib_nr;
            if self.fn_positions.is_empty() && !ctx.data.is_null() {
                let data = unsafe { &*ctx.data };
                self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
            }
        }
        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: self.stack_step(4),
            args_size: input_size as u16,
            line: 0,
        });
        self.stack_pos = self.stack_step(4); // @PLAN53 2j: stepped par-worker entry base (guard-clean; identity flag-OFF)
        // Push the primitive input value at its native byte width.
        // `put_stack` advances stack_pos by `size_of::<T>()`, so we
        // pick the type by `input_size` to match the worker's
        // expected slot 0 width.
        match input_size {
            1 => self.put_stack(input_value as u8),
            4 => self.put_stack(input_value as u32),
            _ => self.put_stack(input_value),
        }
        for &extra in extra_args {
            self.put_stack(extra as i64);
        }
        self.put_stack(u32::MAX); // return address sentinel
        self.code_pos = fn_pos;
        self.run_to_return(Self::WORKER_OP_CEILING);
        match return_size {
            8 => *self.get_stack::<u64>(),
            1 => u64::from(*self.get_stack::<u8>()),
            _ => u64::from(*self.get_stack::<u32>()),
        }
    }

    /// Run a worker whose return is read as a raw `u64`, delivering its slot-0
    /// argument by kind.
    ///
    /// The dispatch twin of [`crate::parallel::worker_row_arg`]: that function decides
    /// what the argument IS, this one decides which entry point takes it.  Two families
    /// used to spell this ladder out themselves, which is how they came to cover
    /// different sets of input shapes.
    pub fn execute_at_raw_worker_arg(
        &mut self,
        fn_pos: u32,
        arg: WorkerArg,
        extra_args: &[u64],
        return_size: u32,
    ) -> u64 {
        match arg {
            WorkerArg::Text(t) => {
                self.execute_at_raw_text_input(fn_pos, t, extra_args, return_size)
            }
            WorkerArg::Primitive { value, size } => {
                self.execute_at_raw_primitive_input(fn_pos, value, size, extra_args, return_size)
            }
            WorkerArg::Wide { buf, size } => self.execute_at_raw_primitive_input_wide(
                fn_pos,
                &buf[..size as usize],
                extra_args,
                return_size,
            ),
            WorkerArg::Ref(r) => self.execute_at_raw(fn_pos, &r, extra_args, return_size),
        }
    }

    /// Plan-06 phase 4d.A — wide-inline-input worker dispatch.
    /// Same as `execute_at_raw_primitive_input` but accepts an
    /// arbitrary-width slot 0 (1..=64 bytes) instead of a u64.
    /// Used for tuple inputs, fn-ref inputs, and any inline-typed
    /// first arg whose stack representation exceeds 8 bytes.
    ///
    /// `input_bytes.len()` is the slot 0 width; `args_size` becomes
    /// `input_bytes.len() + 8 * extra_args.len()` so the call frame
    /// accounting matches what the worker frame's variable table
    /// expects.
    ///
    /// Return value is read as a u64 (worker returning ≤ 8 bytes).
    /// Wide-return cases use `execute_at_raw_to` instead.
    pub fn execute_at_raw_primitive_input_wide(
        &mut self,
        fn_pos: u32,
        input_bytes: &[u8],
        extra_args: &[u64],
        return_size: u32,
    ) -> u64 {
        debug_assert!(
            input_bytes.len() <= 64,
            "wide input slot {} exceeds 64-byte cap",
            input_bytes.len()
        );
        if let Some(ctx) = &self.database.parallel_ctx {
            self.data_ptr = ctx.data;
            self.stack_trace_lib_nr = ctx.stack_trace_lib_nr;
            if self.fn_positions.is_empty() && !ctx.data.is_null() {
                let data = unsafe { &*ctx.data };
                self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
            }
        }
        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        // @PLAN53 cluster 2 / 2i: the worker body's frame reserves the tuple arg
        // at a STEPPED span (codegen advances each arg by stack_step(size)), so a
        // tuple whose raw total is not a multiple of 8 (e.g. (integer, character) =
        // 12) must occupy stack_step(12) = 16 here too.  Providing the raw 12 left
        // the body's frame 4 bytes short and underflowed the worker stack.  The
        // copied DATA is still the raw `input_bytes`; only the reserved frame span
        // (args_size + the TOS advance) is rounded up.  Identity flag-OFF (step==id).
        let stepped_size = self.stack_step(input_bytes.len() as u32);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: self.stack_step(4),
            args_size: stepped_size as u16,
            line: 0,
        });
        self.stack_pos = self.stack_step(4); // @PLAN53 2j: stepped par-worker entry base (guard-clean; identity flag-OFF)
        self.ensure_stack(stepped_size);
        // @PLAN53 cluster 2 / 2h: copy the input bytes as ONE CONTIGUOUS chunk.
        // A byte-by-byte `put_stack::<u8>` advanced stack_pos by stack_step(1) = 8
        // PER BYTE under LOFT_ALIGN, smearing the packed tuple buffer (p.0@0, p.1@8)
        // across 16 separate 8-byte slots; the worker body then reads tuple fields at
        // the raw `element_stack_offsets` [0, 8] into that smeared layout → padding zeros →
        // every worker returned 0 (sum collected as 0).  A block copy keeps the buffer
        // byte-identical to `read_tuple_at_wide`'s raw layout the worker reads.
        // Identity when off (step(1) == 1, so the old byte loop was already contiguous).
        self.ensure_stack(input_bytes.len() as u32);
        let dst = self.database.store_mut(&self.stack_cur).addr_span_mut(
            self.stack_cur.rec,
            self.stack_cur.pos + self.stack_pos,
            input_bytes.len(),
        );
        unsafe {
            std::ptr::copy_nonoverlapping(input_bytes.as_ptr(), dst, input_bytes.len());
        }
        // Advance past the STEPPED arg span (not the raw byte count) so locals /
        // extra args / the return sentinel land where the body's frame expects them.
        self.stack_pos = self.stack_step(4) + stepped_size;
        for &extra in extra_args {
            self.put_stack(extra as i64);
        }
        self.put_stack(u32::MAX); // return address sentinel
        self.code_pos = fn_pos;
        self.run_to_return(Self::WORKER_OP_CEILING);
        match return_size {
            8 => *self.get_stack::<u64>(),
            1 => u64::from(*self.get_stack::<u8>()),
            _ => u64::from(*self.get_stack::<u32>()),
        }
    }

    /// Plan-06 phase 1 G4 — variable-width-return worker dispatch.
    /// Same as `execute_at_raw` but copies the worker's return
    /// value as `return_size` raw bytes from the top of the stack
    /// to `dst`, instead of reading at most 8 bytes.  Used for
    /// fn-ref returns (20 bytes), large-tuple returns, and other
    /// inline returns that exceed the u64 width.
    ///
    /// # Panics
    /// Panics if `return_size` exceeds the worker's current stack
    /// depth at the moment of return — that would mean the worker
    /// fn left less data on the stack than its declared return
    /// width, which is a codegen bug rather than a runtime input.
    ///
    /// # Safety
    /// Caller must ensure `dst` points to at least `return_size`
    /// writable bytes.  The function does no bounds checking on
    /// the destination buffer.
    pub unsafe fn execute_at_raw_to(
        &mut self,
        fn_pos: u32,
        arg: &DbRef,
        extra_args: &[u64],
        return_size: u32,
        dst: *mut u8,
    ) {
        if let Some(ctx) = &self.database.parallel_ctx {
            self.data_ptr = ctx.data;
            self.stack_trace_lib_nr = ctx.stack_trace_lib_nr;
            if self.fn_positions.is_empty() && !ctx.data.is_null() {
                let data = unsafe { &*ctx.data };
                self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
            }
        }
        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: self.stack_step(4),
            args_size: 12,
            line: 0,
        });
        self.stack_pos = self.stack_step(4); // @PLAN53 2j: stepped par-worker entry base (guard-clean; identity flag-OFF)
        self.put_stack(*arg);
        for &extra in extra_args {
            self.put_stack(extra as i64);
        }
        self.put_stack(u32::MAX); // return address sentinel
        self.code_pos = fn_pos;
        self.run_to_return(Self::WORKER_OP_CEILING);
        // Copy `return_size` bytes from the top of the worker
        // stack to `dst`.  Stack grows upward; the return value
        // occupies the topmost `return_size` bytes.
        // @PLAN53 cluster 2: the worker's `copy_result`/`fn_return` advanced TOS by
        // the STEPPED width (`fn_stack + stack_step(size)` = +24 for a 20-byte fn-ref
        // under V2), so the real value sits at the LOW end of a stepped slot with
        // padding on top.  Backing up by the RAW `return_size` read 4 bytes into the
        // value → a 4-byte-shifted (garbage) closure DbRef → OOB free.  Back up by the
        // stepped width to land on the real bytes.  Identity flag-OFF (step == size).
        assert!(
            self.stack_step(return_size) <= self.stack_pos,
            "execute_at_raw_to: return_size {} exceeds stack_pos {}",
            return_size,
            self.stack_pos
        );
        let src_offset = self.stack_pos - self.stack_step(return_size);
        let store = self.database.store(&self.stack_cur);
        unsafe {
            let src = store.base_ptr().offset(
                self.stack_cur.rec as isize * 8 + self.stack_cur.pos as isize + src_offset as isize,
            );
            std::ptr::copy_nonoverlapping(src, dst, return_size as usize);
        }
        self.stack_pos = src_offset;
    }

    /// Plan-06 phase 1 G3 — text-input worker dispatch.
    /// Same as `execute_at_raw` but the first param is a `text`
    /// argument: pushes a 16-byte `Str { ptr, len }` slot built
    /// from the input row's `&str` instead of a 12-byte `DbRef`.
    /// `args_size` is fixed at 16 (Str width).
    pub fn execute_at_raw_text_input(
        &mut self,
        fn_pos: u32,
        input_str: crate::keys::Str,
        extra_args: &[u64],
        return_size: u32,
    ) -> u64 {
        if let Some(ctx) = &self.database.parallel_ctx {
            self.data_ptr = ctx.data;
            self.stack_trace_lib_nr = ctx.stack_trace_lib_nr;
            if self.fn_positions.is_empty() && !ctx.data.is_null() {
                let data = unsafe { &*ctx.data };
                self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
            }
        }
        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: self.stack_step(4),
            // The one argument is the `Str` pushed below, and a `Str` is
            // pointer-sized: 16 bytes natively, 8 on wasm32.  Stack-trace and
            // variable-snapshot readers scan `args_size` bytes of the frame, so a
            // hardcoded 16 sends them past the argument in a browser build.
            args_size: size_of::<Str>() as u16,
            line: 0,
        });
        self.stack_pos = self.stack_step(4); // @PLAN53 2j: stepped par-worker entry base (guard-clean; identity flag-OFF)
        self.put_stack(input_str);
        for &extra in extra_args {
            self.put_stack(extra as i64);
        }
        self.put_stack(u32::MAX); // return address sentinel
        self.code_pos = fn_pos;
        self.run_to_return(Self::WORKER_OP_CEILING);
        match return_size {
            8 => *self.get_stack::<u64>(),
            1 => u64::from(*self.get_stack::<u8>()),
            _ => u64::from(*self.get_stack::<u32>()),
        }
    }

    /// The stack width of a worker's slot-0 argument — the ONE place that fact lives.
    ///
    /// Every `execute_at_*` entry point needs it twice (for the frame's `args_size` and
    /// for the push), and it used to be spelled out at each of them.  Four hand-written
    /// copies is how a wide element ended up with no answer at three of them and the
    /// worker read a `DbRef` as its tuple (loft#1055).
    ///
    /// A wide slot is STEPPED, matching `execute_at_raw_primitive_input_wide`: the
    /// worker's own frame advances each argument by `stack_step`, so a tuple whose raw
    /// total is not a multiple of 8 (`(integer, character)` = 12) must occupy
    /// `stack_step(12)` here too, or the body's frame is short and the stack underflows.
    fn worker_arg_size(&self, arg: &WorkerArg) -> u16 {
        match *arg {
            WorkerArg::Ref(_) => 12,
            WorkerArg::Primitive { size, .. } => size as u16,
            WorkerArg::Text(_) => 16,
            WorkerArg::Wide { size, .. } => self.stack_step(size) as u16,
        }
    }

    /// Push a worker's argument at `stack_pos`, advancing it by [`Self::worker_arg_size`].
    ///
    /// The twin of that function, and its only caller-visible partner: whatever width the
    /// frame reserved is exactly what this writes, so the two cannot drift.
    fn push_worker_arg(&mut self, arg: WorkerArg) {
        match arg {
            WorkerArg::Ref(r) => self.put_stack(r),
            WorkerArg::Primitive { value, size } => match size {
                1 => self.put_stack(value as u8),
                4 => self.put_stack(value as u32),
                _ => self.put_stack(value),
            },
            WorkerArg::Text(s) => self.put_stack(s),
            WorkerArg::Wide { buf, size } => {
                // ONE contiguous copy, never a byte-at-a-time `put_stack::<u8>` — under
                // `LOFT_ALIGN` that advances by `stack_step(1)` = 8 PER BYTE and smears a
                // packed tuple across 16 separate slots, which the worker then reads at
                // the raw element offsets and finds padding.  Same rationale, and the same
                // shape, as `execute_at_raw_primitive_input_wide`.
                let stepped = self.stack_step(size);
                self.ensure_stack(stepped);
                let n = size as usize;
                let dst = self.database.store_mut(&self.stack_cur).addr_span_mut(
                    self.stack_cur.rec,
                    self.stack_cur.pos + self.stack_pos,
                    n,
                );
                unsafe {
                    std::ptr::copy_nonoverlapping(buf.as_ptr(), dst, n);
                }
                self.stack_pos += stepped;
            }
        }
    }

    /// Execute a worker function that returns a struct reference (`DbRef`).
    /// Returns the 12-byte `DbRef` from the worker's stack.  The referenced
    /// record lives in `self.database` (the worker's cloned stores).
    ///
    /// `hidden_dests` is the slice of pre-allocated destination `DbRef`s
    /// for the worker's hidden caller-supplied destination params (added
    /// by `ref_return` for `Type::Vector` / `Type::Reference` /
    /// `Type::Enum(_, true, _)` returns).  Each destination is pushed
    /// as 12 bytes after the input arg and before regular extras —
    /// matching the parameter order the codegen assumes.  Pass `&[]`
    /// when the worker has no hidden args.
    pub fn execute_at_ref(
        &mut self,
        fn_pos: u32,
        arg: WorkerArg,
        hidden_dests: &[DbRef],
        extra_args: &[u64],
    ) -> DbRef {
        if let Some(ctx) = &self.database.parallel_ctx {
            self.data_ptr = ctx.data;
            self.stack_trace_lib_nr = ctx.stack_trace_lib_nr;
            if self.fn_positions.is_empty() && !ctx.data.is_null() {
                let data = unsafe { &*ctx.data };
                self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
            }
        }
        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        let args_size = self.worker_arg_size(&arg);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: self.stack_step(4),
            args_size,
            line: 0,
        });
        self.stack_pos = self.stack_step(4); // @PLAN53 2j: stepped par-worker entry base (guard-clean; identity flag-OFF)
        self.push_worker_arg(arg);
        for &dest in hidden_dests {
            // ARC.md A6.a — push hidden destination DbRefs as 12 bytes
            // (NOT 8-byte i64 like extras).  The codegen for the
            // worker's body computes the next param's offset assuming
            // 12-byte hidden DbRefs; pushing 8 bytes here would
            // misalign every subsequent slot read in the worker.
            self.put_stack(dest);
        }
        for &extra in extra_args {
            self.put_stack(extra as i64);
        }
        self.put_stack(u32::MAX);
        self.code_pos = fn_pos;
        self.run_to_return(Self::WORKER_OP_CEILING);
        *self.get_stack::<DbRef>()
    }

    /// Execute a text-returning worker function; copy the `Str` result to an owned
    /// `String` before the worker state is dropped. Allocates `String` buffers in the
    /// stack store for hidden `__work_N` parameters.
    pub fn execute_at_text(
        &mut self,
        fn_pos: u32,
        arg: WorkerArg,
        extra_args: &[u64],
        n_hidden_text: usize,
    ) -> String {
        if let Some(ctx) = &self.database.parallel_ctx {
            self.data_ptr = ctx.data;
            self.stack_trace_lib_nr = ctx.stack_trace_lib_nr;
            if self.fn_positions.is_empty() && !ctx.data.is_null() {
                let data = unsafe { &*ctx.data };
                self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
            }
        }
        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        let args_size = self.worker_arg_size(&arg);
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: self.stack_step(4),
            args_size,
            line: 0,
        });
        // Allocate String buffers for hidden RefVar(Text) params in the stack store.
        let mut work_crs: Vec<DbRef> = Vec::with_capacity(n_hidden_text);
        for _ in 0..n_hidden_text {
            let cr = self.database.claim(&self.stack_cur, 4); // 32 bytes; String needs 24
            unsafe {
                let p = self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<String>(cr.rec, cr.pos);
                let p = std::ptr::from_mut(p);
                std::ptr::write(p, String::new());
            }
            work_crs.push(cr);
        }

        self.stack_pos = self.stack_step(4); // @PLAN53 2j: stepped par-worker entry base (guard-clean; identity flag-OFF)
        self.push_worker_arg(arg);
        for &extra in extra_args {
            self.put_stack(extra as i64);
        }
        // Push the work buffer DbRefs as the hidden parameters.
        for cr in &work_crs {
            self.put_stack(*cr);
        }
        self.put_stack(u32::MAX);
        self.code_pos = fn_pos;
        self.run_to_return(Self::WORKER_OP_CEILING);
        // Pop the Str return value (16 bytes) and copy into owned String.
        let s = *self.get_stack::<Str>();
        let result = s.str().to_owned();
        // Drop the String buffers to free their heap allocations.
        for cr in work_crs.iter().rev() {
            unsafe {
                let p = self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<String>(cr.rec, cr.pos);
                let p = std::ptr::from_mut(p);
                std::ptr::drop_in_place(p);
            }
        }
        result
    }

    /// Rust→loft host call (the `loft::host` API's engine).  Invoke the function
    /// at `fn_pos` with `args` already marshalled into the stack ABI, and read the
    /// return per `ret`.  This is the GENERAL, top-level analogue of the par-worker
    /// `execute_at_*` family: same stack convention (frame push → args → hidden text
    /// buffers → return sentinel → run → read return), generalised to N arguments.
    ///
    /// `n_hidden_text` is the count of hidden text work-buffer params the callee
    /// carries (a `-> text` return needs them); allocated / pushed / dropped exactly
    /// as `execute_at_text` does.  Priming (the `parallel_ctx` / `fn_positions` /
    /// source-span setup `execute_argv` performs) is refreshed here each call, so the
    /// self-referential context pointers stay valid across an `Instance` move.
    pub fn execute_host(
        &mut self,
        data: &Data,
        fn_pos: u32,
        args: &[WorkerArg],
        n_hidden_text: usize,
        ret: HostRetKind,
    ) -> HostReturn {
        // Prime — mirror the top-level setup in `execute_argv`.  Refreshed every
        // call so the `&raw const self.bytecode` / `self.library` pointers point at
        // THIS State's fields (an `Instance` may have moved after `State::new`).
        let bc_ptr = &raw const self.bytecode;
        let lib_ptr = &raw const self.library;
        let data_ptr = std::ptr::from_ref::<Data>(data);
        self.data_ptr = data_ptr;
        let stk_lib_nr = self
            .library_names
            .get("n_stack_trace")
            .copied()
            .unwrap_or(u16::MAX);
        self.database.parallel_ctx = Some(Box::new(crate::database::ParallelCtx {
            bytecode: bc_ptr,
            library: lib_ptr,
            data: data_ptr,
            stack_trace_lib_nr: stk_lib_nr,
        }));
        if self.fn_positions.is_empty() {
            self.fn_positions = data.definitions.iter().map(|d| d.code_position).collect();
        }
        self.publish_source_spans();

        let d_nr = self
            .fn_positions
            .iter()
            .position(|&p| p == fn_pos)
            .map_or(u32::MAX, |i| i as u32);
        let args_size: u16 = args.iter().map(|a| self.worker_arg_size(a)).sum();
        self.call_stack.push(CallFrame {
            d_nr,
            call_pos: 0,
            args_base: self.stack_step(4),
            args_size,
            line: 0,
        });
        // Hidden text work-buffers (String allocated in the stack store), mirroring
        // `execute_at_text` — a `-> text` callee reads/writes these before the return.
        let mut work_crs: Vec<DbRef> = Vec::with_capacity(n_hidden_text);
        for _ in 0..n_hidden_text {
            let cr = self.database.claim(&self.stack_cur, 4); // 32 bytes; String needs 24
            unsafe {
                let p = self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<String>(cr.rec, cr.pos);
                std::ptr::write(std::ptr::from_mut(p), String::new());
            }
            work_crs.push(cr);
        }
        self.stack_pos = self.stack_step(4);
        for a in args {
            self.push_worker_arg(*a);
        }
        for cr in &work_crs {
            self.put_stack(*cr);
        }
        self.put_stack(u32::MAX); // return address sentinel
        self.code_pos = fn_pos;
        self.run_to_return(0);
        let out = match ret {
            HostRetKind::Void => HostReturn::Void,
            HostRetKind::Prim(sz) => HostReturn::Prim(match sz {
                8 => *self.get_stack::<u64>(),
                1 => u64::from(*self.get_stack::<u8>()),
                _ => u64::from(*self.get_stack::<u32>()),
            }),
            HostRetKind::Text => {
                let s = *self.get_stack::<Str>();
                HostReturn::Text(s.str().to_owned())
            }
            HostRetKind::Ref => HostReturn::Ref(*self.get_stack::<DbRef>()),
        };
        // Drop the hidden text buffers to free their heap allocations.
        for cr in work_crs.iter().rev() {
            unsafe {
                let p = self
                    .database
                    .store_mut(&self.stack_cur)
                    .addr_mut::<String>(cr.rec, cr.pos);
                std::ptr::drop_in_place(std::ptr::from_mut(p));
            }
        }
        out
    }

    /// Execute a void function at `fn_pos` with no arguments.
    /// Used by `parallel {}` arms.
    pub fn execute_at_void(&mut self, fn_pos: u32) {
        self.execute_at_void_with_snapshot(fn_pos, &[]);
    }

    /// Execute a void function at `fn_pos` after seeding the worker's
    /// stack with `parent_snapshot` (P245).
    ///
    /// `parent_snapshot` is the parent's stack contents from offset 4
    /// onwards (skipping the parent's leading sentinel slot).  When a
    /// `parallel {}` arm references an outer-scope variable, the
    /// arm's bytecode addresses it relative to the parent's stack
    /// layout — without copying the parent's bytes into the worker,
    /// those reads return whatever was previously at that offset
    /// (typically 0 / garbage / a stale pointer that triggers
    /// downstream SIGSEGV).
    ///
    /// The snapshot is overlaid on the worker's stack starting at
    /// offset 4 (replacing the worker's freshly-pushed return
    /// sentinel — the parent's bytes at that offset already encode
    /// whatever return PC the parent function was using, which is
    /// the right value for `fn_return` to read at the end of the
    /// arm body).  An empty snapshot keeps the worker's own
    /// `u32::MAX` sentinel and runs as before.
    pub fn execute_at_void_with_snapshot(&mut self, fn_pos: u32, parent_snapshot: &[u8]) {
        self.stack_pos = 4;
        self.put_stack(u32::MAX); // return sentinel — possibly overwritten below
        if !parent_snapshot.is_empty() {
            // Overlay parent's bytes on the worker's stack starting
            // at offset 4.  This includes the parent's own sentinel
            // slot (offset 4..8) — replacing the worker's u32::MAX
            // with whatever the parent had there.
            let store = self.database.store_mut(&self.stack_cur);
            let dst = store.addr_span_mut(
                self.stack_cur.rec,
                self.stack_cur.pos + 4,
                parent_snapshot.len(),
            );
            unsafe {
                std::ptr::copy_nonoverlapping(parent_snapshot.as_ptr(), dst, parent_snapshot.len());
            }
            // After the overlay, stack_pos must mirror the parent's
            // stack_pos so variable reads resolve at the right
            // offsets.  Snapshot length = parent_stack_pos - 4.
            self.stack_pos = 4 + parent_snapshot.len() as u32;
        }
        self.code_pos = fn_pos;
        self.run_to_return(0);
    }

    /**
    Execute a function inside the `byte_code` with logging each step.

    The `config` parameter controls which phases, functions, and opcodes appear
    in the output.  When `config.trace_tail` is set the execution trace is held
    in a ring buffer; if a panic occurs the buffer is flushed to `log` before
    the panic is re-raised, giving you the last N lines at the crash site.

    When `config.phases.execution` is `false`, or the function name does not
    match `config.show_functions`, the function is executed silently (same as
    [`Self::execute`]).

    # Errors
    When the log cannot be written.
    # Panics
    On too many steps or when the stack or claimed structures are not correctly
    cleared afterward.
    */
    pub fn execute_log(
        &mut self,
        log: &mut dyn Write,
        name: &str,
        config: &LogConfig,
        data: &Data,
    ) -> Result<(), Error> {
        debug::execute_log_impl(self, log, name, config, data)
    }

    /// Dump IR / bytecode / variables without executing.
    /// Respects the `LogConfig` phases (ir, bytecode, variables).
    ///
    /// # Errors
    /// Returns an error if the writer fails.
    pub fn dump_bytecode(
        &mut self,
        log: &mut dyn Write,
        config: &LogConfig,
        data: &mut Data,
    ) -> Result<(), Error> {
        crate::compile::show_code(log, self, data, config)
    }
}

#[inline]
#[must_use]
pub fn size_ptr() -> u32 {
    size_of::<crate::keys::Str>() as u32
}

#[inline]
#[must_use]
pub fn size_str() -> u32 {
    size_of::<String>() as u32
}

#[inline]
#[must_use]
pub fn size_ref() -> u32 {
    size_of::<DbRef>() as u32
}
