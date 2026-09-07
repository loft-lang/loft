// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I75 — Diagnostics collector

//! Plan-07 phase 4 — typed runtime errors.
//!
//! Replaces the implicit "panic = bug; sentinel = user error" coin-flip
//! with an explicit [`RuntimeError`] raised at a small set of well-known
//! fault sites.  After phase 4, every fault attributable to user code is
//! a `RuntimeError` with a source position and a stable kind; anything
//! NOT attributable to user code stays a hard panic and is treated as
//! an interpreter bug.
//!
//! Lives on [`crate::database::Stores`] (`runtime_error: Option<Box<RuntimeError>>`)
//! so native fns — which only see `&mut Stores`, not `&mut State` — can
//! populate it.  The interpreter's dispatch loop in
//! `src/state/mod.rs::execute_argv` checks
//! `database.runtime_error.is_some()` after each op and breaks the loop
//! by setting `code_pos = u32::MAX`.  `main.rs` then renders the error
//! through the phase-2 pretty renderer.
//!
//! See `doc/claude/plans/07-error-messages/04-runtime-error-kinds.md`
//! for the per-site conversion list and the rationale for raising vs
//! returning a sentinel (sentinel for `??` lhs; raise otherwise).

// Phase 4 ships infrastructure + the first two fault sites
// (UserPanic, AssertionFailed); the remaining variants and the
// `op_pc` / `kind` field are exercised once steps 4.3-4.10 land.
// Suppress dead-code warnings module-wide until then to keep the
// `bin` target's clippy gate green without splattering per-variant
// allows.
#![allow(dead_code)]

use crate::diagnostics::{DiagEntry, Level};
use crate::lexer::Position;

/// A typed runtime fault attributable to user code.
///
/// Constructed by a fault-site opcode or native fn via the
/// `RuntimeError::*` constructors, stored on `Stores::runtime_error`,
/// surfaced by the dispatch loop, and rendered via
/// [`RuntimeError::to_diag_entry`] through the phase-2 pretty renderer.
#[derive(Debug, Clone)]
pub struct RuntimeError {
    pub kind: RuntimeErrorKind,
    /// Source position of the offending construct, when known.  Resolved
    /// at raise time via `State::source_loc_for(op_pc)` for opcode
    /// faults, or passed directly by the loft surface call (e.g.
    /// `panic("msg")` injects file/line via stdlib stubs).
    pub position: Option<Position>,
    /// Bytecode `pc` of the dispatching op when the error was raised.
    /// `u32::MAX` when not raised from a bytecode op (e.g. native-only
    /// path, future native-runtime conversions).
    pub op_pc: u32,
    /// Free-form human-readable detail.  Kind-specific structured fields
    /// live on `kind`; this string is the rendered presentation tail
    /// shown after the kind label (e.g. `"divide by zero in `attack /
    /// armour`"`).
    pub message: String,
    /// Plan-07 phase 4g.1 / 4g.2 — call-chain at raise time
    /// (innermost first).  Each entry is the function's name as it
    /// appeared in source (the `n_<name>` registry strips its prefix).
    /// Empty when raised outside a function (e.g., top-level script
    /// scope) or when the State call_stack wasn't available (the
    /// Stores-side `raise_runtime` path leaves it empty — native
    /// codegen lacks call-stack capture today, slice-2 work).
    /// Rendered as `  in fn <innermost>() ← fn <next>() ← …` after
    /// the typed-error block in main.rs.
    pub call_chain: Vec<String>,
    /// This fault fired on the far side of a library PLACEMENT boundary, so
    /// `call_chain` holds the frames the library was in and the caller's own
    /// frames are still to come.
    ///
    /// A call chain spans both processes — `main()` is in the caller and
    /// `refuse()` is in the worker — and neither side can see the whole of it.
    /// [`crate::state::State::note_runtime_error_halt`] is where the two halves
    /// meet, and this is what tells it to APPEND rather than leave a chain that
    /// is already non-empty alone.
    pub crossed_placement: bool,
}

#[derive(Debug, Clone)]
pub enum RuntimeErrorKind {
    /// Integer / float `/` or `%` with right-hand side `0`.
    DivideByZero,
    /// Vector / text positive-index access past the end.
    IndexOutOfBounds { idx: i64, len: u32 },
    /// Vector / text index < 0.
    NegativeIndex { idx: i64 },
    /// Field / method access through a null `DbRef`.
    NullDereference,
    /// A write to a store the author locked with `d#lock = true`.  Only the USER lock
    /// reaches here: the const store and a worker borrow share `Store::read_only` but
    /// are the compiler's to get right, and keep their assert.
    WriteToLockedStore { rec: u32, fld: u32 },
    /// Narrowing cast (e.g. `i64 -> i32`) overflowed the target range.
    NarrowCastOverflow { value: i64, target: &'static str },
    /// A `<<` / `>>` whose amount is outside `[0, 64)`, or whose result is the
    /// reserved `i64::MIN` null sentinel (@PLN102 null-model keystone,
    /// D-op-null-2): the shift cannot produce a representable non-null value, so
    /// it yields null and continues (like `÷0`), loudly rather than silently
    /// masking or nulling.
    ShiftOutOfRange,
    /// An `as` cast whose value cannot be represented in the target: a `float`
    /// outside the integer range, an integer that is not a valid Unicode code
    /// point, or a text that parses to exactly the reserved `i64::MIN` sentinel
    /// (@PLN102 D-op-null-2). Yields null and continues (like `÷0`), loudly
    /// rather than silently saturating or nulling a real value.
    CastOutOfRange,
    /// loft#984 — a write left the DECLARED range of a range-limited slot
    /// (`integer limit(lo, hi)`, a narrow width), so the slot took its type's
    /// DEFAULT instead: the lowest value in range, or null where the type admits
    /// it.  Reported rather than silent, because the value the program computed is
    /// not the value the slot now holds — but recoverable, like every other
    /// uncomputable: one value degrades and the run continues (C80).
    RangeDefaulted { value: i64, lo: i64, hi: i64 },
    /// A call would have taken the stack past `State::MAX_CALL_DEPTH` frames.
    StackOverflow,
    /// `panic("msg")` builtin called from loft code.
    UserPanic { message: String },
    /// `assert(test, "msg", file, line)` builtin called with `test == false`.
    AssertionFailed { message: String },
    /// A fault raised inside a PLACED library and relayed across the crossing.
    ///
    /// `label` and `detail` are the ORIGINAL fault's, carried over the wire
    /// rather than re-derived, so a relayed divide-by-zero stays a
    /// `divide_by_zero` in a log instead of being flattened into whatever kind
    /// the relay happened to build.  The variant exists because the kind's own
    /// structured fields do not cross — only what renders does.
    Relayed { label: String, detail: String },
}

impl RuntimeErrorKind {
    /// Stable, machine-readable kind label for log / test output.
    /// Mirrors the variant name verbatim in lower_snake_case.
    #[must_use]
    pub fn label(&self) -> &str {
        match self {
            RuntimeErrorKind::DivideByZero => "divide_by_zero",
            RuntimeErrorKind::IndexOutOfBounds { .. } => "index_out_of_bounds",
            RuntimeErrorKind::NegativeIndex { .. } => "negative_index",
            RuntimeErrorKind::NullDereference => "null_dereference",
            RuntimeErrorKind::NarrowCastOverflow { .. } => "narrow_cast_overflow",
            RuntimeErrorKind::ShiftOutOfRange => "shift_out_of_range",
            RuntimeErrorKind::CastOutOfRange => "cast_out_of_range",
            RuntimeErrorKind::RangeDefaulted { .. } => "range_defaulted",
            RuntimeErrorKind::WriteToLockedStore { .. } => "write_to_locked_store",
            RuntimeErrorKind::StackOverflow => "stack_overflow",
            RuntimeErrorKind::UserPanic { .. } => "user_panic",
            RuntimeErrorKind::AssertionFailed { .. } => "assertion_failed",
            RuntimeErrorKind::Relayed { label, .. } => label,
        }
    }

    /// Human-readable one-line description with structured-field detail
    /// inline.  Used as the `message` of the rendered `DiagEntry`.
    #[must_use]
    pub fn describe(&self) -> String {
        match self {
            RuntimeErrorKind::DivideByZero => "divide by zero".to_string(),
            RuntimeErrorKind::IndexOutOfBounds { idx, len } => {
                format!("index {idx} out of bounds for length {len}")
            }
            RuntimeErrorKind::NegativeIndex { idx } => {
                format!("negative index {idx}")
            }
            RuntimeErrorKind::NullDereference => "null dereference".to_string(),
            RuntimeErrorKind::NarrowCastOverflow { value, target } => {
                format!("value {value} overflows target type {target}")
            }
            RuntimeErrorKind::RangeDefaulted { value, lo, hi } => {
                format!(
                    "value {value} is outside the declared range {lo}..={hi}, so the slot \
                     took its default instead"
                )
            }
            RuntimeErrorKind::ShiftOutOfRange => {
                "shift amount out of range [0,64) or result is the reserved null value".to_string()
            }
            RuntimeErrorKind::CastOutOfRange => {
                "cast value cannot be represented in the target type".to_string()
            }
            RuntimeErrorKind::WriteToLockedStore { .. } => {
                "write to a locked store — unlock it with `#lock = false` before writing"
                    .to_string()
            }
            RuntimeErrorKind::StackOverflow => format!(
                "call stack overflow — exceeded {} stack frames",
                crate::state::State::MAX_CALL_DEPTH
            ),
            RuntimeErrorKind::UserPanic { message } => format!("panic: {message}"),
            RuntimeErrorKind::AssertionFailed { message } => {
                format!("assertion failed: {message}")
            }
            // Already rendered on the far side; describing it again would
            // prefix a second `panic:` onto one that is already there.
            RuntimeErrorKind::Relayed { detail, .. } => detail.clone(),
        }
    }
}

impl RuntimeError {
    /// Construct a `UserPanic` error at the loft surface call site.
    /// `file` / `line` come from the `panic("msg", file, line)` stub
    /// arguments injected by the parser at the loft call site.
    #[must_use]
    pub fn user_panic(message: String, file: String, line: u32) -> Self {
        let position = if file.is_empty() {
            None
        } else {
            Some(Position { file, line, pos: 1 })
        };
        let kind = RuntimeErrorKind::UserPanic { message };
        let detail = kind.describe();
        Self {
            kind,
            position,
            op_pc: u32::MAX,
            message: detail,
            call_chain: Vec::new(),
            crossed_placement: false,
        }
    }

    /// Render an explicit halt — `panic("msg")` or a failed `assert` — the way the
    /// interpreter does, then stop the program.
    ///
    /// The `--native` backend has no bytecode loop to notice `had_fatal` between
    /// statements — the generated `main` only checks it after `n_main` RETURNS — so a
    /// native halt has to report and exit at the call site or it does not halt at all.
    /// Before this existed the generator emitted `fn n_panic(..) {}`, an empty body: on
    /// the DEFAULT backend `panic` printed nothing, halted nothing, and exited 0, while
    /// `--interpret` printed the error and exited 1.  `assert` had the opposite fault —
    /// a real body, but a bare `panic!` naming the generated temp file — and joined this
    /// path with loft#1056.
    ///
    /// Shared with the interpreter's reporting path (`main.rs`) through [`Self::render`],
    /// so both backends emit byte-identical text for the same fault.  There is no
    /// production-mode branch here, unlike `native.rs::n_panic`: a generated binary boots
    /// a plain `Stores` with no logger, so the log-and-continue mode is not reachable on
    /// this path.
    pub fn report_and_exit(&self) -> ! {
        // @PLN133 S8 — a fault inside a lazy DRIVER is the driver's, not the program's.
        // The generated driver call runs under `catch_unwind` and turns the payload into
        // `store_lazy_error`, so this UNWINDS into it instead of exiting: a buggy driver
        // makes the lookup answer null and the program carry on, which is what the
        // interpreter does, and the two backends must not disagree about whether a buggy
        // driver halts a program.  The payload is spelled the way the interpreter's
        // contained-fetch spells it (`<kind label>: <message>`) so `store_lazy_error`
        // reads identically on both.  Before the lock below, deliberately: taking a lock
        // that is never released and then unwinding past it would deadlock the next
        // genuine halt.
        //
        // `cr_stack_overflow` and the crash-report panic hook already carry this test;
        // this is the third site, and the one `assert` reached when loft#1056 moved it
        // onto this path — `panic` had been exiting the process from inside a driver
        // since it started using `report_and_exit`.
        if crate::codegen_runtime::in_lazy_driver() {
            std::panic::panic_any(format!("{}: {}", self.kind.label(), self.message));
        }
        // loft#1056 — a halting fault is the PROGRAM's halt, so it is reported ONCE
        // however many `par` workers reach it in the same instant.  Before this, six
        // items over two workers printed the same diagnostic twice on `--native` and
        // once on `--interpret`.  The lock is never released: the first reporter exits
        // the process while holding it, so a second worker parks here rather than
        // racing the print or exiting out from under it.
        static REPORTING: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = REPORTING
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let rendered = self.with_native_frames().render();
        eprint!("{rendered}");
        // loft#950 — and to the PAGE on the browser target, where `eprint!` is a sink.
        // A `--html` build that faulted printed nothing at all and the trap reached the
        // console as a bare `RuntimeError: unreachable`, so the one thing a fault has to
        // do — say what went wrong — was exactly what it could not do there.  A no-op on
        // every other target.
        crate::live_dispatch::wasm_host_log(&rendered);
        std::process::exit(1);
    }

    /// This error with the `--native` shadow call stack attached, when it carries no
    /// frames of its own.
    ///
    /// The generated `n_assert` / `n_panic` bodies build the error from the stub's
    /// `file` / `line` arguments alone — they have no `State` to read frames from — so
    /// the frames are picked up here, at the one point on the native path that reports.
    /// An error that already has a chain (the interpreter filled it) is left alone.
    #[must_use]
    fn with_native_frames(&self) -> std::borrow::Cow<'_, Self> {
        if !self.call_chain.is_empty() {
            return std::borrow::Cow::Borrowed(self);
        }
        let chain = crate::codegen_runtime::native_call_chain();
        if chain.is_empty() {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut owned = self.clone();
        owned.call_chain = chain;
        std::borrow::Cow::Owned(owned)
    }

    /// The complete text of a halting fault: the typed diagnostic, then the loft call
    /// frames it happened under.
    ///
    /// ONE renderer for every backend — the interpreter reaches it from `main.rs` once
    /// `execute_argv` has returned, the generated binary from
    /// [`Self::report_and_exit`] at the fault site.  They used to print their own
    /// spellings of the same fact, and the two disagreed: a `--native` `assert` emitted
    /// a bare Rust panic naming the generated temp file, with no loft rendering and no
    /// frames at all (loft#1056).
    #[must_use]
    pub fn render(&self) -> String {
        let entry = self.to_diag_entry();
        let loader = crate::diagnostic_render::FileSourceLoader::new();
        let mut out = crate::diagnostic_render::render_entry_pretty(
            &entry,
            &loader,
            crate::diagnostic_render::ColorMode::Auto,
        );
        out.push_str(&self.render_call_chain());
        out
    }

    /// The call frames as the trailing block of [`Self::render`] — innermost first, so
    /// the eye lands on the function the fault fired in, with the chevron pointing
    /// outward along the call sequence.
    ///
    /// A single-frame chain renders as nothing: the fault's own `file:line:col` already
    /// names where it fired, and a chain of one says only "called from nowhere".  Deep
    /// chains are cut at five frames with a count of the rest, because a runaway
    /// recursion's chain is ten thousand copies of one name.
    #[must_use]
    pub fn render_call_chain(&self) -> String {
        if self.call_chain.len() <= 1 {
            return String::new();
        }
        use std::fmt::Write as _;
        let mut out = String::new();
        let shown = self.call_chain.iter().take(5);
        let _ = writeln!(out, "  in fn {}() ← called from", self.call_chain[0]);
        for name in shown.skip(1) {
            let _ = writeln!(out, "        fn {name}()");
        }
        if self.call_chain.len() > 5 {
            let _ = writeln!(out, "        … ({} more frames)", self.call_chain.len() - 5);
        }
        out
    }

    /// Rebuild a fault that fired inside a PLACED library, on the caller's side.
    ///
    /// Only what RENDERS crosses the wire — the detail line, the position, and the
    /// frames the library was in — because that is exactly what
    /// [`Self::to_diag_entry`] and [`Self::render_call_chain`] read.  The kind's
    /// structured fields stay behind; [`RuntimeErrorKind::Relayed`] carries its
    /// label so a log still names what kind of fault it was.
    ///
    /// `position` is the LIBRARY's, not the caller's, which is the whole point: a
    /// fault has one true site and it is on the other side of the crossing.
    #[must_use]
    pub fn relayed(
        label: String,
        detail: String,
        position: Option<Position>,
        call_chain: Vec<String>,
    ) -> Self {
        Self {
            kind: RuntimeErrorKind::Relayed {
                label,
                detail: detail.clone(),
            },
            position,
            op_pc: u32::MAX,
            message: detail,
            call_chain,
            crossed_placement: true,
        }
    }

    /// Construct an `AssertionFailed` error at the loft surface call site.
    #[must_use]
    pub fn assertion_failed(message: String, file: String, line: u32) -> Self {
        let position = if file.is_empty() {
            None
        } else {
            Some(Position { file, line, pos: 1 })
        };
        let kind = RuntimeErrorKind::AssertionFailed { message };
        let detail = kind.describe();
        Self {
            kind,
            position,
            op_pc: u32::MAX,
            message: detail,
            call_chain: Vec::new(),
            crossed_placement: false,
        }
    }

    /// Construct a `StackOverflow` error against the DECLARATION of the function whose
    /// entry exceeded [`crate::state::State::MAX_CALL_DEPTH`].
    ///
    /// Both backends detect the overflow at that entry and both know the callee's
    /// declaration, so this is the position they can agree on — the interpreter reads it
    /// off `Definition::position`, the generated binary off the string literals
    /// `cr_call_push` was emitted with, and one renderer prints both (loft#1058).  It is
    /// also the position worth printing: the fault is a property of ten thousand frames,
    /// not of a point, so naming the function that recursed without bound says more than
    /// naming whichever of ten thousand identical call sites happened to be last.
    ///
    /// The column is 1 — the start of the declaration line — as it is for `panic` and
    /// `assert`.  A definition's stored `position.pos` is the parser's cursor partway
    /// through the signature (after `->`), so a caret there lands on whitespace and
    /// points at nothing.
    /// A write refused because the AUTHOR locked the store (`d#lock = true`).
    ///
    /// No `position`: `Store` knows the record and the field, not the source line, and a
    /// wrong `-->` block is worse than none — the call chain the renderer adds is what
    /// names the author's code.  `op_pc` is `u32::MAX` for the same reason
    /// `stack_overflow` uses it: this is raised below the bytecode dispatch, on the path
    /// both backends share.
    #[must_use]
    pub fn locked_store_write(rec: u32, fld: u32, origin: &str) -> Self {
        // The lock ORIGIN is deliberately not in the message.  It spells an internal
        // store number that differs between the backends for the same program
        // (`store_nr=2` interpreted, `store_nr=0` native), so putting it here would make
        // the two render different text for one event — the property `report_and_exit`
        // exists to hold.  It is a debugging fact, and `LOFT_LOG=locks` is where it lives.
        let _ = origin;
        let kind = RuntimeErrorKind::WriteToLockedStore { rec, fld };
        let detail = kind.describe();
        Self {
            kind,
            position: None,
            op_pc: u32::MAX,
            message: detail,
            call_chain: Vec::new(),
            crossed_placement: false,
        }
    }

    #[must_use]
    pub fn stack_overflow(file: String, line: u32) -> Self {
        let position = if file.is_empty() {
            None
        } else {
            Some(Position { file, line, pos: 1 })
        };
        let kind = RuntimeErrorKind::StackOverflow;
        let detail = kind.describe();
        Self {
            kind,
            position,
            op_pc: u32::MAX,
            message: detail,
            call_chain: Vec::new(),
            crossed_placement: false,
        }
    }

    /// Convert to a `DiagEntry` so the existing phase-2 pretty renderer
    /// can format the error with `--> file:line:col` + source line +
    /// caret.  Errors without a position emit just the level + message.
    #[must_use]
    pub fn to_diag_entry(&self) -> DiagEntry {
        let (file, line, col) = self.position.as_ref().map_or_else(
            || (String::new(), 0, 0),
            |p| (p.file.clone(), p.line, p.pos),
        );
        DiagEntry {
            level: Level::Error,
            message: self.message.clone(),
            file,
            line,
            col,
            code: None,
            suggestion: None,
            fixes: Vec::new(),
        }
    }
}

/// In production mode, record a halting fault and let the program carry on.
///
/// `true` means the fault has been logged and the caller must RETURN rather than halt;
/// `false` means production mode is off and the caller halts as it otherwise would.
///
/// This is the whole of `DESIGN_DECISIONS.md § C66` — *"a single error should not bring
/// everything down"* — and it lives here because BOTH backends have to reach the same
/// answer.  The interpreter's `native.rs::n_panic` / `n_assert` and the bodies the native
/// generator emits for those two are separate code that implements one documented
/// behaviour, and while each decided it for itself only one of them did: `--native`, the
/// DEFAULT backend, aborted and wrote nothing, so `loft app.loft` got the opposite of the
/// contract that `loft --interpret app.loft` honoured (loft#1263).
///
/// `had_fatal` is still set, so the run exits non-zero.  Production mode changes WHEN the
/// program stops, not whether the fault counted.
///
/// Takes the position in pieces because the generated Rust must be able to call this and
/// `crate::lexer` is not part of the public surface — and because building the `Position`
/// here is one fewer thing for three call sites to spell the same way.
pub fn logged_in_production(
    stores: &mut crate::database::Stores,
    kind: &RuntimeErrorKind,
    file: &str,
    line: u32,
) -> bool {
    let Some(logger) = stores.logger.clone() else {
        return false;
    };
    if !logger.lock().is_ok_and(|l| l.config.production) {
        return false;
    }
    // Routed through `log_runtime_kind` rather than `log` so the entry carries the same
    // `[user_panic]` / `[assertion_failed]` label and the same C66 severity as every other
    // production-mode runtime event, on whichever backend produced it.
    let position = Position {
        file: file.to_string(),
        line,
        pos: 1,
    };
    if let Ok(mut lg) = logger.lock() {
        lg.log_runtime_kind(kind, Some(&position));
    }
    stores.had_fatal = true;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_panic_carries_message_and_position() {
        let err = RuntimeError::user_panic("oops".into(), "test.loft".into(), 42);
        assert!(matches!(err.kind, RuntimeErrorKind::UserPanic { .. }));
        assert_eq!(err.kind.label(), "user_panic");
        assert!(err.message.contains("oops"));
        let pos = err.position.as_ref().expect("position present");
        assert_eq!(pos.file, "test.loft");
        assert_eq!(pos.line, 42);
    }

    #[test]
    fn assertion_failed_carries_message_and_position() {
        let err = RuntimeError::assertion_failed("x == 5".into(), "fixture.loft".into(), 7);
        assert!(matches!(err.kind, RuntimeErrorKind::AssertionFailed { .. }));
        assert_eq!(err.kind.label(), "assertion_failed");
        assert!(err.message.contains("x == 5"));
        let pos = err.position.as_ref().expect("position present");
        assert_eq!(pos.line, 7);
    }

    #[test]
    fn empty_file_means_no_position() {
        let err = RuntimeError::user_panic("oops".into(), String::new(), 0);
        assert!(err.position.is_none());
    }

    #[test]
    fn to_diag_entry_renders_level_and_position() {
        let err = RuntimeError::user_panic("boom".into(), "x.loft".into(), 3);
        let entry = err.to_diag_entry();
        assert_eq!(entry.level, Level::Error);
        assert_eq!(entry.file, "x.loft");
        assert_eq!(entry.line, 3);
        assert_eq!(entry.col, 1);
        assert!(entry.message.contains("boom"));
    }

    #[test]
    fn kinds_have_distinct_labels() {
        let kinds = [
            RuntimeErrorKind::DivideByZero,
            RuntimeErrorKind::IndexOutOfBounds { idx: 5, len: 3 },
            RuntimeErrorKind::NegativeIndex { idx: -1 },
            RuntimeErrorKind::NullDereference,
            RuntimeErrorKind::WriteToLockedStore { rec: 1, fld: 8 },
            RuntimeErrorKind::NarrowCastOverflow {
                value: 99_999,
                target: "i8",
            },
            RuntimeErrorKind::RangeDefaulted {
                value: 300,
                lo: 0,
                hi: 255,
            },
            RuntimeErrorKind::StackOverflow,
            RuntimeErrorKind::UserPanic {
                message: "u".into(),
            },
            RuntimeErrorKind::AssertionFailed {
                message: "a".into(),
            },
        ];
        let mut labels: Vec<&str> = kinds.iter().map(RuntimeErrorKind::label).collect();
        labels.sort_unstable();
        let n = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), n, "kind labels collided");
    }

    /// A fault relayed from a placed library keeps what it was, and is not
    /// described a second time.
    ///
    /// Describing it again is what printed `panic: runtime error: panic: …` — the
    /// detail arriving here has already been rendered on the far side, so the label
    /// and the detail are carried, never rebuilt.
    #[test]
    fn a_relayed_fault_keeps_its_label_and_its_rendered_detail() {
        let err = RuntimeError::relayed(
            "divide_by_zero".into(),
            "divide by zero".into(),
            Some(Position {
                file: "lib.loft".into(),
                line: 7,
                pos: 1,
            }),
            vec!["inner".into()],
        );
        assert_eq!(err.kind.label(), "divide_by_zero");
        assert_eq!(err.message, "divide by zero");
        assert_eq!(err.kind.describe(), "divide by zero");
        assert_eq!(err.position.as_ref().expect("position").line, 7);
        // The caller's own frames are still to come; this is what says so.
        assert!(err.crossed_placement);
        assert_eq!(err.call_chain, vec!["inner".to_string()]);
    }
}
