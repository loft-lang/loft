// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I76 — Logger runtime

// The public API of this module is consumed by the test crate, not by the
// loft binary.  Suppress the dead_code lint that fires for the binary.
#![allow(dead_code)]

//! Structured logging configuration for the loft test harness.
//!
//! Build a [`LogConfig`] directly or use one of the preset constructors.
//! At test time set the `LOFT_LOG` env-var to pick a preset:
//!
//! | Value | Preset |
//! |---|---|
//! | `full` | [`LogConfig::full`] — IR + bytecode + execution, slot annotations |
//! | `static` | [`LogConfig::static_only`] — IR + bytecode only |
//! | `minimal` | [`LogConfig::minimal`] — execution trace for `test` only |
//! | `ref_debug` | [`LogConfig::ref_debug`] — full + snapshots on Ref ops |
//! | `bridging` | [`LogConfig::bridging`] — bridging-invariant check |
//! | `crash_tail[:N]` | [`LogConfig::crash_tail`] — last N lines, flushed on panic |
//! | `fn:<name>` | [`LogConfig::function`] — single named function |
//! | `variables` | [`LogConfig::variables`] — variable table per function (slot assignment) |
//! | `all_fns` | [`LogConfig::all_fns`] — bytecode of all functions including `default/` built-ins |

use std::collections::VecDeque;
use std::io::{self, Write};

/// Selects which compiler phases are included in the output.
#[derive(Clone)]
pub struct LogPhase {
    /// Show IR (intermediate representation) for each function.
    pub ir: bool,
    /// Show bytecode disassembly for each function.
    pub bytecode: bool,
    /// Show the execution trace.
    pub execution: bool,
}

impl LogPhase {
    /// All phases enabled.
    #[must_use]
    pub fn all() -> Self {
        Self {
            ir: true,
            bytecode: true,
            execution: true,
        }
    }

    /// No phases enabled.
    #[must_use]
    pub fn none() -> Self {
        Self {
            ir: false,
            bytecode: false,
            execution: false,
        }
    }

    /// IR + bytecode only; no execution trace.
    #[must_use]
    pub fn static_only() -> Self {
        Self {
            ir: true,
            bytecode: true,
            execution: false,
        }
    }

    /// Execution trace only; skip IR and bytecode.
    #[must_use]
    pub fn execution_only() -> Self {
        Self {
            ir: false,
            bytecode: false,
            execution: true,
        }
    }
}

/// Controls what the test log files (`tests/dumps/*.txt`) include.
///
/// Build from a preset or from the `LOFT_LOG` environment variable with
/// [`LogConfig::from_env`].
#[allow(clippy::struct_excessive_bools)]
#[derive(Clone)]
pub struct LogConfig {
    /// Phase filter: which compilation/execution phases to log.
    pub phases: LogPhase,
    /// Only log IR/bytecode/execution for functions whose name contains one
    /// of these strings.  `None` = log all functions.
    pub show_functions: Option<Vec<String>>,
    /// Only include execution steps whose opcode name (without the `Op`
    /// prefix) contains one of these strings.  `None` = include all opcodes.
    pub trace_opcodes: Option<Vec<String>>,
    /// Keep only the last N lines of the execution trace in a ring buffer.
    /// On panic the buffer is flushed before re-raising.  `None` = unlimited.
    pub trace_tail: Option<usize>,
    /// Annotate bytecode and trace output with variable slot assignments
    /// (name + stack position + type).
    pub annotate_slots: bool,
    /// Capture a stack snapshot after every opcode whose name (without `Op`
    /// prefix) contains one of these strings.  `None` = never snapshot.
    pub snapshot_opcodes: Option<Vec<String>>,
    /// Number of bytes printed per snapshot.
    pub snapshot_window: usize,
    /// Emit a warning whenever the runtime `stack_pos` deviates from the
    /// compile-time expected value (bridging invariant).
    pub check_bridging: bool,
    /// Print the full variable table (name, type, scope, slot, live interval)
    /// for each function after its IR and bytecode.  Useful for diagnosing
    /// slot-assignment issues.
    pub show_variables: bool,
    /// Include functions from the `default/` built-in library in the bytecode
    /// dump.  Normally these are skipped because they are not user-authored.
    /// Enable with `LOFT_LOG=all_fns` to see the complete bytecode including
    /// built-in operators and standard-library helpers.
    pub show_all_functions: bool,
    /// Print scope-analysis diagnostics to stderr: which Reference variables are freed
    /// or skipped by `get_free_vars`, and any "orphaned" vars whose scope is not in the
    /// cleanup chain.  Enable with `LOFT_LOG=scope_debug`.
    /// This field is informational; `scopes.rs` reads `LOFT_LOG` directly so it works
    /// even in test runs that don't construct a `LogConfig`.
    pub scope_debug: bool,
    /// Dump live variables after every traced opcode.  Replaces the
    /// `LOFT_DUMP_VARS` env-var check, which was unsafe in parallel tests.
    pub dump_vars: bool,
    /// Annotate the execution trace with `[alloc #N at pc=…]` and
    /// `[free  #N (allocated at pc=…)]` lines so a later `Database N
    /// not correctly freed` panic can be traced back to the original
    /// allocation site by grepping for `alloc #N`.  Enabled with
    /// `LOFT_LOG=alloc_free`.
    pub trace_alloc_free: bool,
    /// Write a poison pattern into freed text/ref slots so subsequent
    /// reads through `Str::str()` / `DbRef::valid()` panic loudly with
    /// "read from poisoned slot" instead of the cryptic Rust UB
    /// dispatch from `ptr::copy_nonoverlapping`.  Enabled with
    /// `LOFT_LOG=poison_free`.
    pub poison_free: bool,
    /// Print every store-lock / store-unlock event to stderr with the
    /// store_nr, record nr, and the runtime caller's location.  Use
    /// `LOFT_LOG=locks` to activate.  Highest-leverage diagnostic for
    /// "Write to read-only store at rec=N fld=M (locked by: …)" panics — the lock-event
    /// trace immediately identifies which op acquired the lock.
    /// This field is informational; `database/allocation.rs` reads
    /// `LOFT_LOG` directly via `lock_trace_enabled()` so it works
    /// even in test runs that don't construct a `LogConfig`.
    pub trace_locks: bool,
    /// Print every variable type-mutation event for a specific named
    /// variable to stderr.  Set via `LOFT_LOG=type_timeline:<varname>`
    /// — only mutations of variables whose name matches `<varname>`
    /// are logged.  Each entry shows old type, new type, and the
    /// mutation site (e.g. `add_variable`, `change_var_type`,
    /// `set_type`, `subst_type`).
    /// This field is informational; `variables/mod.rs` reads
    /// `LOFT_LOG` directly via `type_timeline_target()`.
    pub trace_type_timeline: Option<String>,
}

/// Plan-22 phase 02d-vii follow-up — central check for the
/// `LOFT_LOG=locks` mode.  Inlined at every lock/unlock site.
/// Reads the env var on each call; fast-fail (Err return) when
/// the variable is unset or doesn't equal "locks".
#[must_use]
pub fn lock_trace_enabled() -> bool {
    std::env::var("LOFT_LOG").as_deref() == Ok("locks")
}

/// Plan-22 02d-vii follow-up — `LOFT_LOG=type_timeline:<varname>`
/// mode.  Returns `Some(varname)` when the env var has the
/// `type_timeline:NAME` form; the caller then logs only var
/// mutations matching `NAME`.  Returns `None` otherwise (fast
/// path for the common case).
#[must_use]
pub fn type_timeline_target() -> Option<String> {
    let raw = std::env::var("LOFT_LOG").ok()?;
    raw.strip_prefix("type_timeline:").map(str::to_string)
}

/// Plan-22 02d-vii follow-up — `LOFT_LOG=slots:<fn_name>`
/// mode.  Returns `Some(fn_name)` when the env var has the
/// `slots:NAME` form.  Caller dumps slot-allocation decisions
/// for fns matching `NAME` (substring match).
#[must_use]
pub fn slots_trace_target() -> Option<String> {
    let raw = std::env::var("LOFT_LOG").ok()?;
    raw.strip_prefix("slots:").map(str::to_string)
}

/// Plan-22 02d-vii follow-up — `LOFT_LOG=captures:<fn_name>`
/// mode.  Returns `Some(fn_name)` when the env var has the
/// `captures:NAME` form.  Caller dumps the capture-pipeline
/// state for fns matching `NAME` (substring match): per-fn
/// scalars_to_box, per-lambda mutated_captures + closure_record
/// d_nr + per-attribute auto-Reference status.
#[must_use]
pub fn captures_trace_target() -> Option<String> {
    let raw = std::env::var("LOFT_LOG").ok()?;
    raw.strip_prefix("captures:").map(str::to_string)
}

impl LogConfig {
    // ------------------------------------------------------------------ presets

    /// Log everything: IR, bytecode, full execution trace, with slot annotations.
    #[must_use]
    pub fn full() -> Self {
        Self {
            phases: LogPhase::all(),
            show_functions: None,
            trace_opcodes: None,
            trace_tail: None,
            annotate_slots: true,
            snapshot_opcodes: None,
            snapshot_window: 64,
            check_bridging: false,
            show_variables: false,
            show_all_functions: false,
            scope_debug: false,
            dump_vars: false,
            trace_alloc_free: false,
            poison_free: false,
            trace_locks: false,
            trace_type_timeline: None,
        }
    }

    /// IR + bytecode only — no execution trace.  Useful for debugging codegen.
    #[must_use]
    pub fn static_only() -> Self {
        Self {
            phases: LogPhase::static_only(),
            show_functions: None,
            trace_opcodes: None,
            trace_tail: None,
            annotate_slots: false,
            snapshot_opcodes: None,
            snapshot_window: 0,
            check_bridging: false,
            show_variables: false,
            show_all_functions: false,
            scope_debug: false,
            dump_vars: false,
            trace_alloc_free: false,
            poison_free: false,
            trace_locks: false,
            trace_type_timeline: None,
        }
    }

    /// Execution trace for the `test` entry point only, no IR or bytecode.
    #[must_use]
    pub fn minimal() -> Self {
        Self {
            phases: LogPhase::execution_only(),
            show_functions: Some(vec!["test".to_string()]),
            trace_opcodes: None,
            trace_tail: None,
            annotate_slots: false,
            snapshot_opcodes: None,
            snapshot_window: 0,
            check_bridging: false,
            show_variables: false,
            show_all_functions: false,
            scope_debug: false,
            dump_vars: false,
            trace_alloc_free: false,
            poison_free: false,
            trace_locks: false,
            trace_type_timeline: None,
        }
    }

    /// Full output with slot annotations and stack snapshots on every Ref /
    /// `CreateStack` opcode — useful for debugging reference lifetime bugs.
    #[must_use]
    pub fn ref_debug() -> Self {
        Self {
            phases: LogPhase::all(),
            show_functions: None,
            trace_opcodes: None,
            trace_tail: None,
            annotate_slots: true,
            snapshot_opcodes: Some(vec!["Ref".to_string(), "CreateStack".to_string()]),
            snapshot_window: 64,
            check_bridging: false,
            show_variables: false,
            show_all_functions: false,
            scope_debug: false,
            dump_vars: false,
            trace_alloc_free: false,
            poison_free: false,
            trace_locks: false,
            trace_type_timeline: None,
        }
    }

    /// Execution-only with the bridging invariant check enabled.
    #[must_use]
    pub fn bridging() -> Self {
        Self {
            phases: LogPhase::execution_only(),
            show_functions: None,
            trace_opcodes: None,
            trace_tail: None,
            annotate_slots: false,
            snapshot_opcodes: None,
            snapshot_window: 0,
            check_bridging: true,
            show_variables: false,
            show_all_functions: false,
            scope_debug: false,
            dump_vars: false,
            trace_alloc_free: false,
            poison_free: false,
            trace_locks: false,
            trace_type_timeline: None,
        }
    }

    /// Log only the named function (both static and execution phases).
    #[must_use]
    pub fn function(name: &str) -> Self {
        Self {
            phases: LogPhase::all(),
            show_functions: Some(vec![name.to_string()]),
            trace_opcodes: None,
            trace_tail: None,
            annotate_slots: false,
            snapshot_opcodes: None,
            snapshot_window: 0,
            check_bridging: false,
            show_variables: false,
            show_all_functions: false,
            scope_debug: false,
            dump_vars: false,
            trace_alloc_free: false,
            poison_free: false,
            trace_locks: false,
            trace_type_timeline: None,
        }
    }

    /// Keep only the last `n` execution-trace lines; flush on panic.
    #[must_use]
    pub fn crash_tail(n: usize) -> Self {
        Self {
            phases: LogPhase::execution_only(),
            show_functions: None,
            trace_opcodes: None,
            trace_tail: Some(n),
            annotate_slots: false,
            snapshot_opcodes: None,
            snapshot_window: 0,
            check_bridging: false,
            show_variables: false,
            show_all_functions: false,
            scope_debug: false,
            dump_vars: false,
            trace_alloc_free: false,
            poison_free: false,
            trace_locks: false,
            trace_type_timeline: None,
        }
    }

    /// Variable table per function: name, type, scope, stack slot, and live interval.
    ///
    /// Shows IR and bytecode but no execution trace.  Useful for diagnosing
    /// slot-assignment issues.
    #[must_use]
    pub fn variables() -> Self {
        Self {
            phases: LogPhase::static_only(),
            show_functions: None,
            trace_opcodes: None,
            trace_tail: None,
            annotate_slots: true,
            snapshot_opcodes: None,
            snapshot_window: 0,
            check_bridging: false,
            show_variables: true,
            show_all_functions: false,
            scope_debug: false,
            dump_vars: false,
            trace_alloc_free: false,
            poison_free: false,
            trace_locks: false,
            trace_type_timeline: None,
        }
    }

    /// Plan-22 02d-vii follow-up — IR-only dump for ONE function.
    /// `LOFT_LOG=ir:<fn_name>` builds this preset.
    ///
    /// Strips bytecode + execution trace down to just the parsed IR
    /// tree for the named function.  Used to verify "did the parser
    /// emit the IR shape I expected?" without the noise of bytecode
    /// or runtime trace from `LOFT_LOG=full`.  The function name is
    /// matched as a substring against fn names as parsed; e.g.
    /// `LOFT_LOG=ir:test` matches `n_test` and any sibling fns
    /// containing "test".  Use the full `n_<name>` form to disambiguate.
    #[must_use]
    pub fn ir_only(fn_name: &str) -> Self {
        Self {
            phases: LogPhase {
                ir: true,
                bytecode: false,
                execution: false,
            },
            show_functions: Some(vec![fn_name.to_string()]),
            trace_opcodes: None,
            trace_tail: None,
            annotate_slots: true,
            snapshot_opcodes: None,
            snapshot_window: 0,
            check_bridging: false,
            show_variables: false,
            show_all_functions: false,
            scope_debug: false,
            dump_vars: false,
            trace_alloc_free: false,
            poison_free: false,
            trace_locks: false,
            trace_type_timeline: None,
        }
    }

    /// Print scope-analysis diagnostics to stderr: freed/skipped Reference vars and
    /// any "orphaned" vars whose scope is unreachable from function-exit cleanup.
    ///
    /// Use `LOFT_LOG=scope_debug` to activate.  Output goes to stderr so it appears
    /// in `cargo test` output even when the dump file is suppressed.
    /// The `scopes.rs` module reads `LOFT_LOG` directly, so this preset is
    /// informational — you do not need to construct a `LogConfig` to enable the trace.
    #[must_use]
    pub fn scope_debug() -> Self {
        Self {
            phases: LogPhase::static_only(),
            show_functions: None,
            trace_opcodes: None,
            trace_tail: None,
            annotate_slots: false,
            snapshot_opcodes: None,
            snapshot_window: 0,
            check_bridging: false,
            show_variables: true,
            show_all_functions: false,
            scope_debug: true,
            dump_vars: false,
            trace_alloc_free: false,
            poison_free: false,
            trace_locks: false,
            trace_type_timeline: None,
        }
    }

    /// Bytecode dump for **all** functions including built-ins from `default/`.
    ///
    /// Use `LOFT_LOG=all_fns` to activate.  The output is large but lets you
    /// inspect built-in helpers at their absolute bytecode positions, which is
    /// essential for diagnosing crashes whose opcode address falls inside a
    /// `default/` function that the normal dump skips.
    #[must_use]
    pub fn all_fns() -> Self {
        Self {
            phases: LogPhase::static_only(),
            show_functions: None,
            trace_opcodes: None,
            trace_tail: None,
            annotate_slots: true,
            snapshot_opcodes: None,
            snapshot_window: 0,
            check_bridging: false,
            show_variables: false,
            show_all_functions: true,
            scope_debug: false,
            dump_vars: false,
            trace_alloc_free: false,
            poison_free: false,
            trace_locks: false,
            trace_type_timeline: None,
        }
    }

    // --------------------------------------------------------------- from env

    /// Select a preset based on the `LOFT_LOG` environment variable.
    ///
    /// | Value | Preset |
    /// |---|---|
    /// | `full` | [`Self::full`] |
    /// | `static` | [`Self::static_only`] |
    /// | `minimal` | [`Self::minimal`] |
    /// | `ref_debug` | [`Self::ref_debug`] |
    /// | `bridging` | [`Self::bridging`] |
    /// | `crash_tail` or `crash_tail:N` | [`Self::crash_tail`] (default N = 50) |
    /// | `fn:<name>` | [`Self::function`] |
    /// | `variables` | [`Self::variables`] |
    /// | `all_fns` | [`Self::all_fns`] — bytecode of every function including `default/` |
    /// | `scope_debug` | [`Self::scope_debug`] — scope-analysis diagnostics to stderr |
    ///
    /// Falls back to [`Self::full`] when the variable is absent or unrecognised.
    #[must_use]
    pub fn from_env() -> Self {
        match std::env::var("LOFT_LOG").as_deref() {
            Ok("full") => Self::full(),
            Ok("static") => Self::static_only(),
            Ok("minimal") => Self::minimal(),
            Ok("ref_debug") => Self::ref_debug(),
            Ok("bridging") => Self::bridging(),
            Ok("variables") => Self::variables(),
            Ok("scope_debug") => Self::scope_debug(),
            Ok("alloc_free") => {
                let mut c = Self::full();
                c.trace_alloc_free = true;
                c
            }
            Ok("poison_free") => {
                let mut c = Self::full();
                c.poison_free = true;
                c
            }
            Ok("locks") => {
                let mut c = Self::full();
                c.trace_locks = true;
                c
            }
            Ok(s) if s.starts_with("type_timeline:") => {
                let mut c = Self::full();
                c.trace_type_timeline = Some(s.trim_start_matches("type_timeline:").to_string());
                c
            }
            Ok(s) if s.starts_with("ir:") => Self::ir_only(&s[3..]),
            Ok(s) if s.starts_with("crash_tail") => {
                let n = s
                    .trim_start_matches("crash_tail")
                    .trim_start_matches(':')
                    .parse()
                    .unwrap_or(50);
                Self::crash_tail(n)
            }
            Ok(s) if s.starts_with("fn:") => Self::function(&s[3..]),
            Ok("all_fns") => Self::all_fns(),
            _ => Self::full(),
        }
    }

    // ----------------------------------------------------------- filter helpers

    /// Returns `true` if a function with the given name should appear in the log.
    #[must_use]
    pub fn show_function(&self, name: &str) -> bool {
        match &self.show_functions {
            None => true,
            Some(names) => names.iter().any(|n| name.contains(n.as_str())),
        }
    }

    /// Returns `true` if an execution step with the given opcode name (without
    /// the `Op` prefix) should be included in the trace.
    #[must_use]
    pub fn trace_opcode(&self, op_base: &str) -> bool {
        match &self.trace_opcodes {
            None => true,
            Some(ops) => ops.iter().any(|o| op_base.contains(o.as_str())),
        }
    }

    /// Returns `true` if a stack snapshot should be taken after this opcode.
    #[must_use]
    pub fn snapshot_opcode(&self, op_base: &str) -> bool {
        match &self.snapshot_opcodes {
            None => false,
            Some(ops) => ops.iter().any(|o| op_base.contains(o.as_str())),
        }
    }
}

// ---------------------------------------------------------------------- TailBuffer

/// A ring buffer that retains the last `capacity` lines written to it.
///
/// Use [`TailBuffer::flush_to`] to drain all retained lines to another writer.
/// This is used by [`LogConfig::crash_tail`] to keep only the final N lines
/// of the execution trace, flushing them after a panic is caught.
pub struct TailBuffer {
    buf: VecDeque<Vec<u8>>,
    capacity: usize,
    current: Vec<u8>,
}

impl TailBuffer {
    /// Create a new tail buffer retaining the last `capacity` complete lines.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            capacity,
            current: Vec::new(),
        }
    }

    /// Write all retained lines to `out` and clear the buffer.
    ///
    /// # Errors
    /// Propagates any I/O error from writing to `out`.
    pub fn flush_to(&mut self, out: &mut dyn Write) -> io::Result<()> {
        for line in self.buf.drain(..) {
            out.write_all(&line)?;
        }
        if !self.current.is_empty() {
            out.write_all(&std::mem::take(&mut self.current))?;
        }
        Ok(())
    }
}

impl Write for TailBuffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        for &byte in buf {
            self.current.push(byte);
            if byte == b'\n' {
                let line = std::mem::take(&mut self.current);
                self.buf.push_back(line);
                if self.buf.len() > self.capacity {
                    self.buf.pop_front();
                }
            }
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
