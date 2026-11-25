// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I75 — Diagnostics collector
//
// Minimal crash reporter: installs a SIGSEGV / SIGABRT / SIGBUS
// signal handler that prints the last-executed opcode, bytecode
// position, and source position of every running interpreter.
//
// Goal: when the interpreter crashes inside an opcode that corrupts
// memory (heap overflow, stack overflow, use-after-free), the
// glibc / kernel error arrives OUT of Rust's panic path — `set_hook`
// only catches panics, not signals.  Without a native handler the
// process just aborts with no context.
//
// Usage: call [`install`] once near program start (or at test
// harness init).  Each interpreter keeps its crash context in its
// `Stores` ([`CrashCx`]): the full dispatch loop writes the op per
// dispatch, the lean loop registers where it keeps its position
// ([`LeanSource`]).  A running dispatch loop enters its context into
// a table of atomic slots ([`register`]); on a crash the handler
// reports every context in that table — it cannot know, and never
// asks, which thread crashed (`@FR-C-Boundary`).
//
// Design notes:
//
// - Async-signal safety: signal handlers are extremely restricted —
//   we cannot call `format!`, `println!`, or allocate.  We format
//   directly into a fixed `[u8; N]` buffer on the stack and write it
//   with `libc::write(STDERR_FILENO, ...)`.  The handler reads only
//   atomics and a published `OnceLock`.
// - A panic is not a signal: the interpreter and the compile steps
//   catch their own panics and add the loft position they know
//   (`State::dispatch`, [`compile_panicked`]), so no position is ever
//   looked up by thread.
// - No allocation: the buffer is fixed-size; the fields are all
//   fixed-width types (u32 pc, u16 op code) or a `&'static str`
//   (the op name, from the published opcode table).

#![allow(clippy::module_name_repetitions)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicUsize, Ordering};

/// A program's fault-site spans: the first pc of a construct → its position and its end pc.
pub type Spans = BTreeMap<u32, (crate::lexer::Position, u32)>;

/// The op a context was dispatching.
#[derive(Clone, Copy)]
struct Ctx {
    /// `u32::MAX` until an op has been dispatched in this context.
    pc: u32,
    fn_d_nr: u32,
    /// The full opcode — `255 + ext` for a two-byte one, never the escape byte alone.
    op_code: u16,
}

/// `@PLN226` — one interpreter's crash context: the op it dispatches, the lean loop's
/// registration it derives that op from, its fault-site spans and its store-trace names.
/// Owned by the interpreter's `Stores` (`Stores::crash`, `@FR-C-Own`): the store layer's own
/// fault reports read it there.  A fatal-signal report, which runs with no context in hand,
/// reads it through the table of running interpreters ([`register`]), never by asking which
/// thread runs (`@FR-C-Boundary`) — so the words that report reads are atomics.
pub struct CrashCx {
    pc: AtomicU32,
    fn_d_nr: AtomicU32,
    op_code: AtomicU32,
    /// `@FR-R-DispatchPublish` — a running lean dispatch loop's position, bytecode and length
    /// ([`LeanSource`]); null when no lean loop runs.
    lean_pc: AtomicPtr<u32>,
    lean_code: AtomicPtr<*const u8>,
    lean_len: AtomicPtr<u32>,
    /// The fault-site spans, and the same table for the fatal-signal report.
    spans: Option<Arc<Spans>>,
    spans_at: AtomicPtr<Spans>,
    /// The store-lifetime instruments' names (`store_trace::PcNames`).
    pub(crate) pc_names: Option<Arc<crate::store_trace::PcNames>>,
    /// The slot of [`RUNNING`] this context holds, `usize::MAX` when none.
    slot: AtomicUsize,
}

impl Default for CrashCx {
    fn default() -> Self {
        CrashCx {
            pc: AtomicU32::new(u32::MAX),
            fn_d_nr: AtomicU32::new(u32::MAX),
            op_code: AtomicU32::new(0),
            lean_pc: AtomicPtr::new(std::ptr::null_mut()),
            lean_code: AtomicPtr::new(std::ptr::null_mut()),
            lean_len: AtomicPtr::new(std::ptr::null_mut()),
            spans: None,
            spans_at: AtomicPtr::new(std::ptr::null_mut()),
            pc_names: None,
            slot: AtomicUsize::new(usize::MAX),
        }
    }
}

impl Clone for CrashCx {
    /// A new context (a worker's `Stores`) has dispatched nothing yet.
    fn clone(&self) -> Self {
        CrashCx::default()
    }
}

impl CrashCx {
    /// The op `pc` (`op_code`, in function `fn_d_nr`) is about to dispatch: the full loop's
    /// per-op publish, twelve bytes.
    #[inline]
    pub fn set_dispatch(&self, pc: u32, op_code: u16, fn_d_nr: u32) {
        self.pc.store(pc, Ordering::Relaxed);
        self.op_code.store(u32::from(op_code), Ordering::Relaxed);
        self.fn_d_nr.store(fn_d_nr, Ordering::Relaxed);
    }

    /// The op this context is dispatching: derived from a running lean loop's registration, or
    /// the one the full loop wrote.
    fn current(&self) -> Ctx {
        let pc = self.lean_pc.load(Ordering::Relaxed);
        if pc.is_null() {
            return Ctx {
                pc: self.pc.load(Ordering::Relaxed),
                fn_d_nr: self.fn_d_nr.load(Ordering::Relaxed),
                op_code: self.op_code.load(Ordering::Relaxed) as u16,
            };
        }
        let code = self.lean_code.load(Ordering::Relaxed);
        let len = self.lean_len.load(Ordering::Relaxed);
        // SAFETY: registered by the running loop, which keeps all three alive and in place
        // until its `LeanSource` drops (see `LeanSource::register`).
        let (pc, base, len) = unsafe {
            (
                pc.read_volatile(),
                code.read_volatile(),
                len.read_volatile(),
            )
        };
        // SAFETY: `base` holds `len` bytes — the cache `State::edit_code` keeps.
        let byte = |at: u32| (at < len).then(|| unsafe { *base.add(at as usize) });
        let op_code = match byte(pc) {
            Some(255) => 255 + u16::from(byte(pc + 1).unwrap_or(0)),
            Some(b) => u16::from(b),
            None => u16::MAX,
        };
        Ctx {
            pc,
            fn_d_nr: u32::MAX,
            op_code,
        }
    }

    /// The last-dispatched op: `(pc, op_code, fn_d_nr)`, `pc` `u32::MAX` before the first.
    /// Used by debug sentinels (the `put_stack`/`get_stack` DbRef-bounds check) and the
    /// store layer's fault reports to say which op was executing.
    #[must_use]
    pub fn last_context(&self) -> (u32, u16, u32) {
        let ctx = self.current();
        (ctx.pc, ctx.op_code, ctx.fn_d_nr)
    }

    /// The name of the op being dispatched, or `""` if no table has been published yet.
    ///
    /// The signal handler is not the only reader that needs this.  A runtime invariant
    /// breach reported from deep inside the store layer — `BUG (#306)`, a strict-store
    /// violation — has a `pc` and nothing else, and a bare `pc` is a position in the WHOLE
    /// bytecode stream (stdlib included) that no tool a reader has in hand can resolve.
    /// Naming the op turns "something freed store 0" into "this op freed store 0", which is
    /// the difference between a report and a lead (loft#920).
    #[must_use]
    pub fn last_op_name(&self) -> &'static str {
        op_name_of(self.current().op_code)
    }

    /// The interpreter's fault-site spans, so a fault
    /// can print `at file:line:col` for the offending pc.  `None` clears them.
    pub fn set_spans(&mut self, spans: Option<Arc<Spans>>) {
        let at = spans
            .as_ref()
            .map_or(std::ptr::null_mut(), |s| Arc::as_ptr(s).cast_mut());
        self.spans_at.store(at, Ordering::Release);
        self.spans = spans;
    }

    /// The source position of the construct that
    /// CONTAINS `pc`.
    ///
    /// `None` when no spans are set or no recorded span covers `pc`.  Answering `None` is the
    /// point: the table is sparse, so most pcs have a nearest preceding entry that belongs to
    /// an unrelated earlier statement, and rendering that as `at file:line:col` sends a reader
    /// to a line the fault is not on with nothing to mark it a guess (loft#1262).  A caller
    /// that wants the approximate answer, and can present it as one, calls
    /// [`Self::nearest_source_loc_for_pc`].
    ///
    /// Spans nest, so the walk is backwards and the first covering entry is the innermost —
    /// the most precise position available.
    #[must_use]
    pub fn source_loc_for_pc(&self, pc: u32) -> Option<crate::lexer::Position> {
        self.spans.as_ref().and_then(|m| {
            m.range(..=pc)
                .rev()
                .find(|(_, (_, end))| *end > pc)
                .map(|(_, (p, _))| *p)
        })
    }

    /// The nearest span recorded at or before `pc`, with the pc it was recorded at.
    ///
    /// This is the approximate reading, and a caller must present it as one — the distance
    /// between the two pcs is how much to trust it.  Use it only where a rough location beats
    /// none; where the output looks like an exact position, [`Self::source_loc_for_pc`] is the
    /// one that can be trusted.
    #[must_use]
    pub fn nearest_source_loc_for_pc(&self, pc: u32) -> Option<(u32, crate::lexer::Position)> {
        self.spans
            .as_ref()
            .and_then(|m| m.range(..=pc).next_back().map(|(at, (p, _))| (*at, *p)))
    }
}

/// Opcode number → name, published once per process by [`set_op_names`].
///
/// The report used to print a bare `op=249`, which names nothing a reader can act
/// on — the whole point of a crash report is to say WHAT was running.  The name is
/// known (`Data::operator_name`), it was simply never on the path a signal handler
/// can reach: a handler cannot allocate, cannot lock, and cannot borrow anything
/// the crashing thread might hold.  A `OnceLock` of leaked strings can be read from
/// one (a relaxed atomic load and an index — the same shape as `PROGRAM` above),
/// and it is built once, off the signal path, from the definitions table.
static OP_NAMES: OnceLock<Vec<&'static str>> = OnceLock::new();

/// Publish the opcode-name table, BUILDING it only when nothing has published one
/// yet.  Idempotent; the first call wins, which is what makes it safe to call from
/// every execute entry rather than one blessed site.
///
/// `build` is a closure rather than a ready-made table because building one costs
/// 256 `Box::leak`s, and a leak is only acceptable as a ONE-OFF.  Taking the table
/// by value let every caller pay that price and then hand it to a `OnceLock` that
/// dropped all but the first: `execute_argv` runs once per program, so a test binary
/// running a hundred of them leaked a hundred tables — 386 612 bytes in the nightly
/// fuzz sweep, and a red ASan leak gate (loft#820).  The invariant was written down
/// here ("published once per process") while the caller re-derived it every run.
pub fn set_op_names(build: impl FnOnce() -> Vec<&'static str>) {
    let _ = OP_NAMES.get_or_init(build);
}

/// The name of opcode `op`, or `""` when no table was published.  Read by the fatal-signal
/// report ([`report_fatal`]), so it must stay async-signal-safe: one `OnceLock` read.
fn op_name_of(op: u16) -> &'static str {
    OP_NAMES
        .get()
        .and_then(|v| v.get(op as usize).copied())
        .unwrap_or("")
}

#[derive(Clone, Copy)]
struct LeanSrc {
    pc: *mut u32,
    code: *mut *const u8,
    len: *mut u32,
}

/// `@FR-R-DispatchPublish` — a lean dispatch loop's registration in its context's
/// [`CrashCx`], alive for exactly the loop: built before its first op, dropped on every way out
/// (a return, a break, an unwind), when it publishes the last op into the context — so a reader
/// after the loop still finds it — and restores the registration it replaced, which is how a
/// nested loop leaves its caller's.
pub struct LeanSource {
    cx: *const CrashCx,
    prev: LeanSrc,
}

impl LeanSource {
    /// Register the loop's op position and its bytecode's cached base and length
    /// (`@FR-R-CodeBase`) in `cx`.  All three are fields of the running `State`, which does
    /// not move while its loop runs; they are read at report time, so code written during the
    /// loop is read as written.
    #[must_use]
    pub fn register(
        cx: &CrashCx,
        pc: *const u32,
        code: *const *const u8,
        len: *const u32,
    ) -> LeanSource {
        let prev = LeanSrc {
            pc: cx.lean_pc.swap(pc.cast_mut(), Ordering::Relaxed),
            code: cx.lean_code.swap(code.cast_mut(), Ordering::Relaxed),
            len: cx.lean_len.swap(len.cast_mut(), Ordering::Relaxed),
        };
        LeanSource { cx, prev }
    }
}

impl Drop for LeanSource {
    fn drop(&mut self) {
        // SAFETY: the context is a field of the `Stores` of the `State` whose loop built this,
        // which outlives the loop and does not move while it runs.
        let cx = unsafe { &*self.cx };
        let last = cx.current();
        cx.set_dispatch(last.pc, last.op_code, last.fn_d_nr);
        cx.lean_pc.store(self.prev.pc, Ordering::Relaxed);
        cx.lean_code.store(self.prev.code, Ordering::Relaxed);
        cx.lean_len.store(self.prev.len, Ordering::Relaxed);
    }
}

/// How many interpreters a fatal-signal report can name at once.
const SLOTS: usize = 64;

/// `@PLN226` — the running interpreters' crash contexts, for the fatal-signal report: a slot
/// per running dispatch loop, each one atomic word (`@FR-C-Publish`), taken by compare-and-swap
/// when the loop starts ([`register`]) and given back when it ends.  The report names every
/// interpreter in the table; it never asks which thread crashed (`@FR-C-Boundary`).
static RUNNING: [AtomicPtr<CrashCx>; SLOTS] =
    [const { AtomicPtr::new(std::ptr::null_mut()) }; SLOTS];

/// A context's place in [`RUNNING`], given back on drop.
pub struct Running {
    cx: *const CrashCx,
}

/// Enter `cx` into the table of running interpreters, for the fatal-signal report.  `None`
/// when it is already there (a nested loop of the same interpreter) or the table is full.
#[must_use]
pub fn register(cx: &CrashCx) -> Option<Running> {
    if cx.slot.load(Ordering::Relaxed) != usize::MAX {
        return None;
    }
    let me = std::ptr::from_ref(cx).cast_mut();
    for (at, slot) in RUNNING.iter().enumerate() {
        if slot
            .compare_exchange(
                std::ptr::null_mut(),
                me,
                Ordering::AcqRel,
                Ordering::Relaxed,
            )
            .is_ok()
        {
            cx.slot.store(at, Ordering::Relaxed);
            return Some(Running { cx });
        }
    }
    None
}

impl Drop for Running {
    fn drop(&mut self) {
        // SAFETY: the context outlives the loop that registered it (see [`register`]).
        let cx = unsafe { &*self.cx };
        let at = cx.slot.swap(usize::MAX, Ordering::Relaxed);
        if let Some(slot) = RUNNING.get(at) {
            slot.store(std::ptr::null_mut(), Ordering::Release);
        }
    }
}

/// Whether an interpreter that panics prints `at file:line:col` for the op it was
/// dispatching (`State::dispatch`).  The CLI asks for it ([`tell_panic_positions`]); a test
/// binary or an embedder, whose panics are its own to report, does not.
static PANIC_POSITIONS: AtomicBool = AtomicBool::new(false);

/// An interpreter that panics names the loft source
/// position of the op it was dispatching, after the panic's own message.  It prints a position
/// only when a recorded span COVERS the pc, and otherwise says nothing: the span table is
/// sparse, so an inherited answer names whatever statement happened to be wrapped last
/// (loft#1262).
pub fn tell_panic_positions() {
    PANIC_POSITIONS.store(true, Ordering::Relaxed);
}

/// Did the CLI ask for panic positions ([`tell_panic_positions`]), and not for quiet
/// ([`quiet_panics`])?
#[must_use]
pub fn panic_positions() -> bool {
    PANIC_POSITIONS.load(Ordering::Relaxed) && !QUIET.load(Ordering::Relaxed)
}

/// Whether the process keeps panic reports to itself ([`quiet_panics`]).
static QUIET: AtomicBool = AtomicBool::new(false);

/// Keep Rust's raw `thread panicked at …` report, and the loft position after it, off stderr
/// for the rest of the process.  For the CLI's REPL and file debugger, which catch a program's
/// runtime panic and report it cleanly, so the raw report is noise.
///
/// A panic hook is the process's, and can know only the process, never the session a panic
/// came from (`@FR-C-Boundary`): so quiet is asked for by a process that is nothing but the
/// session — the CLI, before its session starts.  A REPL run inside another process (a test,
/// an embedder) keeps the raw report, which that process's own harness owns; swapping the hook
/// per session would silence its unrelated threads for the duration.
pub fn quiet_panics() {
    QUIET.store(true, Ordering::Relaxed);
    static INSTALL: OnceLock<()> = OnceLock::new();
    INSTALL.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !QUIET.load(Ordering::Relaxed) {
                previous(info);
            }
        }));
    });
}

/// Used by the installer to ensure we only install once per process.
static INSTALLED: AtomicBool = AtomicBool::new(false);
static PANIC_HOOK_INSTALLED: AtomicBool = AtomicBool::new(false);

/// Holds the program name for the diagnostic prefix.
static PROGRAM: OnceLock<&'static str> = OnceLock::new();

/// Where the crash diagnostic is ALSO written, resolved once by [`install`].
///
/// A signal handler's output is the one thing a build must not lose to a
/// pipeline: the run that produces it is by definition the run you cannot
/// repeat.  loft#717 lost a SIGSEGV's opcode/pc/function exactly that way — the
/// package sweep piped stderr through a filter and the diagnostic went out with
/// the warnings, leaving one unactionable line.
///
/// The path is resolved (and NUL-terminated) at install time because building it
/// allocates; the handler itself only calls `open`/`write`/`close`, which POSIX
/// lists as async-signal-safe.  Nothing is created unless a crash actually fires.
///
/// Set only where the host catches fatal signals (`platform::catches_fatal_signals`), like
/// the handler that fills it: a target with no fatal signals to catch has no report to lose.
static CRASH_FILE: OnceLock<CrashFile> = OnceLock::new();

/// A crash-report destination: the same path twice, once NUL-terminated for
/// `open(2)` and once printable for the stderr line that names it.
struct CrashFile {
    /// NUL-terminated, ready to hand to `open(2)` without formatting.
    c_path: Vec<u8>,
    /// The same path, for the "report written to" line.
    display: String,
}

/// Choose where a crash report should land, or `None` when the user turned the
/// file off with an empty `LOFT_CRASH_FILE`.
///
/// `LOFT_CRASH_FILE` wins (a CI run or a test pins an exact path).  Otherwise a
/// project-local `.loft/` takes it when one already exists — that is the
/// `loft test` case the report is most wanted in, and it keeps the file beside
/// the package that crashed.  Everything else falls back to the temp directory,
/// which is always writable.  The directory is never CREATED: `mkdir` is not
/// async-signal-safe, and a run that does not crash should leave no trace.
///
/// The pid keeps concurrent packages in one sweep from overwriting each other.
fn choose_crash_file() -> Option<CrashFile> {
    crash_file_from(std::env::var("LOFT_CRASH_FILE").ok())
}

/// The destination for a given `LOFT_CRASH_FILE` setting — the decision in
/// [`choose_crash_file`], separated from reading the environment so it can be
/// tested without mutating process-wide state (which races every other test in
/// the binary, and is `unsafe` besides).
fn crash_file_from(setting: Option<String>) -> Option<CrashFile> {
    let path = match setting {
        // Explicitly emptied — the caller wants stderr only.
        Some(p) if p.is_empty() => return None,
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let name = format!("loft-crash-{}.txt", std::process::id());
            let dot_loft = std::path::Path::new(".loft");
            if crate::file_access::is_dir(dot_loft) {
                dot_loft.join(name)
            } else {
                std::env::temp_dir().join(name)
            }
        }
    };
    let display = path.display().to_string();
    let mut c_path = display.clone().into_bytes();
    // An interior NUL would silently truncate the path `open` sees; refuse it
    // rather than write somewhere the user did not name.
    if c_path.contains(&0) {
        return None;
    }
    c_path.push(0);
    Some(CrashFile { c_path, display })
}

/// The resolved crash-report path, once [`install`] has run.  `None` when no
/// file is configured — the diagnostic then goes to stderr alone, as before,
/// which is also every host that catches no fatal signal (there is no handler to feed it).
#[must_use]
pub fn crash_file_path() -> Option<String> {
    CRASH_FILE.get().map(|c| c.display.clone())
}

/// The prefix that marks a panic as a program past one of the compiler's hard limits, not as a
/// compiler fault.
const LIMIT_MARK: &str = "loft limit: ";

/// Stop compiling: the program at `pos` passes a hard limit of the compiler, described by
/// `message`.  For a limit a pass meets after the parser, where no diagnostics list is in hand
/// (the 64 KiB stack frame, `crate::limits`).  Under the CLI's panic hook
/// ([`install_panic_hook`]) the program sees one `error:` and exit 1; elsewhere a panic carries
/// the same text.
///
/// # Panics
/// Outside the CLI: the panic is how the refusal leaves a pass that has no diagnostics list.
pub fn limit_exceeded(pos: &crate::lexer::Position, message: &str) -> ! {
    if PANIC_HOOK_INSTALLED.load(Ordering::Relaxed) {
        render_at(crate::diagnostics::Level::Error, message, pos);
        std::process::exit(1);
    }
    panic!("{LIMIT_MARK}{message}")
}

/// loft#665 piece 3 — a compile step that panicked at `pos` in the user's program: under the
/// CLI's panic hook ([`install_panic_hook`]), say so as a loft diagnostic at that position,
/// after the Rust-side report the hook printed.  Called by the compile steps that catch their
/// own panics (`Parser::guarded`, `compile::byte_code`) with the position they were at — a
/// position is the compile's own, never looked up by thread (`@FR-C-Boundary`).
pub fn compile_panicked(pos: &crate::lexer::Position, payload: &(dyn std::any::Any + Send)) {
    if !PANIC_HOOK_INSTALLED.load(Ordering::Relaxed) {
        return;
    }
    let detail = payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "internal error".to_string());
    let msg = format!(
        "internal compiler error — please report this program at \
         https://github.com/loft-lang/loft/issues\n  {detail}"
    );
    render_at(crate::diagnostics::Level::Fatal, &msg, pos);
}

/// Print `msg` as a diagnostic of `level` at `pos`, in the CLI's error mode.
fn render_at(level: crate::diagnostics::Level, msg: &str, pos: &crate::lexer::Position) {
    let mut diags = crate::diagnostics::Diagnostics::new();
    diags.add_at(level, msg, pos.file.as_str(), pos.line, pos.pos);
    let mode = crate::diagnostic_render::ErrorMode::from_cli_and_env(None);
    if matches!(mode, crate::diagnostic_render::ErrorMode::Pretty) {
        let loader = crate::diagnostic_render::FileSourceLoader::new();
        eprint!(
            "{}",
            crate::diagnostic_render::render_pretty_all(
                &diags,
                &loader,
                crate::diagnostic_render::ColorMode::Auto,
            )
        );
    } else {
        for entry in diags.entries() {
            eprintln!("{}", entry.to_string_compact());
        }
    }
}

/// The CLI's panic hook: a closed stdout ends the run quietly, a fault inside a lazy driver is
/// not announced, and everything else gets the Rust report.  It also arms the loft-side
/// report of a compile step's panic ([`compile_panicked`]) and of a hard limit
/// ([`limit_exceeded`]).
///
/// A compiler-internal invariant that breaks is still something the USER triggered with a
/// specific program, and `thread 'main' panicked at src/state/codegen.rs:2955` alone names a
/// generated symbol and an argument count they never wrote (loft#662).  Wrapping the hook
/// rather than replacing it keeps the Rust location and backtrace, which are what make the
/// report actionable; the compile step that caught the panic adds the user's line.
pub fn install_panic_hook() {
    if PANIC_HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // loft#1289 — the CLI's own `println!` (`--version | head -1`) meets a closed reader
        // exactly as a program's `print` does, and gets the same answer: the end of the run.
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .copied()
            .or_else(|| info.payload().downcast_ref::<String>().map(String::as_str));
        if message.is_some_and(crate::host_print::is_closed_stdout_panic) {
            std::process::exit(0);
        }
        // @PLN133 S8 — a fault inside a lazy DRIVER is contained and reported
        // through `store_lazy_error`, so it is not a crash and must not be
        // announced as one. Printing it would be worse than noise: a lookup that
        // correctly answered null would look like a program that fell over.  The
        // payload names it (`@FR-C-Boundary`: nothing asks which thread raised it).
        if info.payload().is::<crate::call_frames::ContainedFault>() {
            return;
        }
        prev(info);
    }));
}

/// Install the fatal-signal report for SIGSEGV / SIGABRT / SIGBUS
/// (`platform::on_fatal_signal`).
///
/// No-op where the host catches no fatal signal (Windows: `platform::catches_fatal_signals`)
/// and when called more than once.
pub fn install(program: &'static str) {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = PROGRAM.set(program);
    if !crate::platform::catches_fatal_signals() {
        return;
    }
    // Resolve the report path BEFORE the handlers are armed, so a crash the
    // instant after this call still has somewhere to write.
    if let Some(f) = choose_crash_file() {
        let _ = CRASH_FILE.set(f);
    }
    crate::platform::on_fatal_signal(report_fatal);
}

/// The op label a crash report prints: the opcode's own name when the table reached us, else
/// the dispatch loop's — never just the number, which names nothing.
fn op_label(op_code: u16) -> &'static str {
    let named = op_name_of(op_code);
    if named.is_empty() {
        "(opcode dispatch)"
    } else {
        named
    }
}

/// The fatal-signal report, called INSIDE the signal handler (`platform::on_fatal_signal`)
/// with the signal's name.  Names every running interpreter ([`RUNNING`]) — one, unless `par`
/// workers or a host run beside the program — by its last op and position.  Async-signal-safe:
/// it reads atomics and writes a diagnostic to stderr (and the crash file) from a fixed buffer;
/// the default handler then takes over (which produces a core dump if `ulimit -c` is set).
fn report_fatal(sig_name: &'static str) {
    let program = PROGRAM.get().copied().unwrap_or("loft");
    // Build message into a fixed-size buffer, async-signal-safe.
    let mut buf = [0u8; 4096];
    let mut w = Writer::new(&mut buf);
    let _ = w.str("\n=== loft crash (");
    let _ = w.str(program);
    let _ = w.str(") ");
    let _ = w.str(sig_name);
    let _ = w.str(" caught ===\n");
    let mut told = 0;
    for slot in &RUNNING {
        let cx = slot.load(Ordering::Acquire);
        if cx.is_null() {
            continue;
        }
        if told > 0 {
            let _ = w.str("  ---\n");
        }
        // SAFETY: a context stays in the table only while its loop runs (`Running`), and the
        // report reads its atomics alone: the owning thread may be mid-op, never mid-free.
        report_context(&mut w, unsafe { &*cx });
        told += 1;
    }
    if told == 0 {
        // Still useful: it confirms the signal fired.
        let _ = w.str("  last op:  (none — crash outside interpreter)\n");
    }
    let _ = w.str("===\n");
    let bytes = w.as_bytes();
    crate::platform::signal_safe_stderr(bytes);
    // loft#717 — the same bytes to a FILE, so a pipeline that filters stderr
    // cannot swallow the one diagnostic that can never be regenerated.  Write
    // it BEFORE announcing the path, so the line below is only printed when
    // there is really something at the other end of it.  `c_path` was
    // NUL-terminated at install time, so nothing here allocates or formats.
    if let Some(target) = CRASH_FILE.get()
        && crate::platform::signal_safe_write_file(&target.c_path, bytes)
    {
        let mut note = [0u8; 1024];
        let mut nw = Writer::new(&mut note);
        let _ = nw.str("  report also written to: ");
        let _ = nw.str(&target.display);
        let _ = nw.str("\n");
        crate::platform::signal_safe_stderr(nw.as_bytes());
    }
    // SA_RESETHAND → the default handler fires next, producing the
    // core dump and terminating the process.
}

/// One running interpreter's lines in the fatal-signal report: its last op, pc and function,
/// and the source position of that op.
fn report_context(w: &mut Writer, cx: &CrashCx) {
    let ctx = cx.current();
    let _ = w.str("  last op:  ");
    if ctx.pc == u32::MAX {
        let _ = w.str("(none yet)\n");
        return;
    }
    let _ = w.str(op_label(ctx.op_code));
    let _ = w.str(" (op=");
    let _ = w.u32(u32::from(ctx.op_code));
    let _ = w.str(")\n  pc:       ");
    let _ = w.u32(ctx.pc);
    let _ = w.str("\n  fn:       (?)");
    // The lean loop does not track the function (`LeanSource`); the `at:` line below names it.
    if ctx.fn_d_nr != u32::MAX {
        let _ = w.str(" (d_nr=");
        let _ = w.u32(ctx.fn_d_nr);
        let _ = w.str(")");
    }
    let _ = w.str("\n");
    // The span AND the pc it was recorded at.  The lookup is "the last span
    // at or before pc", which is only the crash site when a span actually covers that pc — and
    // the table is sparse (one entry per statement, and none at all for regions no statement
    // produced).  Unqualified, it reported loft#806's fault at `default/05_coroutine.loft:18:27`,
    // a stdlib line the program never calls, and sent the reader there.  Carrying the DISTANCE
    // turns a confident wrong answer into a reading that says how much to trust it: `+0` is
    // the statement, `+3121` is "the nearest thing recorded, three thousand bytes back".
    let spans = cx.spans_at.load(Ordering::Acquire);
    // SAFETY: the table the context holds (`CrashCx::spans`) while it runs.
    let source_loc = (!spans.is_null())
        .then(|| unsafe { &*spans })
        .and_then(|m| m.range(..=ctx.pc).next_back())
        .map(|(at, (p, _))| (*at, *p));
    // Emit `at file:line:col` when the source span lookup succeeded.
    // Truncate file path to fit; the user can still grep for it.
    if let Some((span_pc, pos)) = source_loc.as_ref() {
        let _ = w.str("  at:      ");
        let _ = w.str(pos.file.as_str());
        let _ = w.str(":");
        let _ = w.u32(pos.line);
        let _ = w.str(":");
        let _ = w.u32(pos.pos);
        // How far the crashing pc is PAST the statement this line describes.  Zero means the
        // line is the site; anything large means the table had nothing nearer and the line is
        // a lower bound, not an answer.
        let gap = ctx.pc.saturating_sub(*span_pc);
        if gap == 0 {
            let _ = w.str("\n");
        } else {
            let _ = w.str("  (nearest span, pc+");
            let _ = w.u32(gap);
            let _ = w.str(" — NOT necessarily this line)\n");
        }
    } else {
        let _ = w.str("  at:      (no source span covers this pc)\n");
    }
}

// `Writer` and its methods are pure Rust, so the fatal-signal report can format without
// allocating; on a target that never arms the report (e.g. WASM release) it looks dead.
#[allow(dead_code)]
struct Writer<'a> {
    buf: &'a mut [u8],
    pos: usize,
}

#[allow(dead_code)]
impl<'a> Writer<'a> {
    fn new(buf: &'a mut [u8]) -> Self {
        Writer { buf, pos: 0 }
    }
    fn str(&mut self, s: &str) -> Result<(), ()> {
        for &b in s.as_bytes() {
            if self.pos >= self.buf.len() {
                return Err(());
            }
            self.buf[self.pos] = b;
            self.pos += 1;
        }
        Ok(())
    }
    fn u32(&mut self, mut n: u32) -> Result<(), ()> {
        if n == 0 {
            return self.str("0");
        }
        let mut digits = [0u8; 10];
        let mut i = 0;
        while n > 0 {
            digits[i] = b'0' + (n % 10) as u8;
            n /= 10;
            i += 1;
        }
        while i > 0 {
            i -= 1;
            if self.pos >= self.buf.len() {
                return Err(());
            }
            self.buf[self.pos] = digits[i];
            self.pos += 1;
        }
        Ok(())
    }
    fn as_bytes(&self) -> &[u8] {
        &self.buf[..self.pos]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn writer_basic() {
        let mut buf = [0u8; 64];
        let mut w = Writer::new(&mut buf);
        w.str("pc=").unwrap();
        w.u32(42).unwrap();
        w.str(" op=").unwrap();
        w.str("OpReturn").unwrap();
        assert_eq!(w.as_bytes(), b"pc=42 op=OpReturn");
    }

    #[test]
    fn writer_zero() {
        let mut buf = [0u8; 8];
        let mut w = Writer::new(&mut buf);
        w.u32(0).unwrap();
        assert_eq!(w.as_bytes(), b"0");
    }

    #[test]
    fn writer_max() {
        let mut buf = [0u8; 16];
        let mut w = Writer::new(&mut buf);
        w.u32(u32::MAX).unwrap();
        assert_eq!(w.as_bytes(), b"4294967295");
    }

    #[test]
    fn context_updates() {
        let cx = CrashCx::default();
        assert_eq!(cx.last_context().0, u32::MAX, "nothing dispatched yet");
        cx.set_dispatch(10, 7, 42);
        assert_eq!(cx.last_context(), (10, 7, 42));
    }

    /// A running interpreter holds a slot of the table the fatal-signal report reads, once
    /// however deeply its loops nest, and gives it back when its loop ends.
    #[test]
    fn a_running_context_holds_one_slot_while_it_runs() {
        let cx = CrashCx::default();
        let held = |cx: &CrashCx| {
            RUNNING
                .iter()
                .filter(|s| std::ptr::eq(s.load(Ordering::Relaxed), cx))
                .count()
        };
        let outer = register(&cx).expect("a free slot");
        assert_eq!(held(&cx), 1);
        assert!(
            register(&cx).is_none(),
            "a nested loop takes no second slot"
        );
        assert_eq!(held(&cx), 1);
        drop(outer);
        assert_eq!(held(&cx), 0, "the slot is given back");
    }

    #[test]
    fn install_is_idempotent() {
        // Calling twice should not panic or misbehave.
        install("test");
        install("test");
    }

    /// loft#717 — where a crash report lands.  The end-to-end guarantee (a real
    /// signal, stderr discarded, a readable file afterwards) is
    /// `tests/crash_report_file.rs`; this pins the choice itself, which has three
    /// outcomes and only one of them is exercised by that test.
    #[test]
    fn crash_file_choice() {
        // An explicit setting is taken verbatim, and NUL-terminated for `open`.
        let named = crash_file_from(Some("/tmp/loft-report.txt".to_string()))
            .expect("an explicit path is used");
        assert_eq!(named.display, "/tmp/loft-report.txt");
        assert_eq!(named.c_path.last(), Some(&0), "ready for open(2)");
        assert_eq!(
            &named.c_path[..named.c_path.len() - 1],
            named.display.as_bytes(),
            "the bytes open(2) sees are the path the user was shown"
        );

        // Emptied on purpose — stderr only, nothing written and nothing claimed.
        assert!(
            crash_file_from(Some(String::new())).is_none(),
            "an empty setting turns the file off rather than writing to \"\""
        );

        // Unset — a default that always resolves, and never to a bare directory.
        let fallback = crash_file_from(None).expect("a default destination always exists");
        assert!(
            fallback.display.contains(&std::process::id().to_string()),
            "the pid keeps concurrent packages in one sweep from overwriting \
             each other's report: {}",
            fallback.display
        );

        // An interior NUL would make `open` see a silently truncated path, so it
        // is refused rather than written somewhere the user did not name.
        assert!(
            crash_file_from(Some("/tmp/a\0b".to_string())).is_none(),
            "a path open(2) cannot represent faithfully is declined"
        );
    }

    /// Plan-07 phase 3 — source-position lookup for the panic hook + signal
    /// handler.  A span answers for the pcs it COVERS and for no others.
    ///
    /// The rows past each construct's end are the point.  They used to answer the
    /// nearest preceding entry, so a fault in an unwrapped statement was rendered
    /// as an exact position on an unrelated earlier line — or, with nothing
    /// preceding it in the user's file, on a stdlib line the program never
    /// mentions (loft#1262).
    #[test]
    fn a_span_answers_only_for_the_pcs_it_covers() {
        use crate::lexer::Position;
        use std::collections::BTreeMap;
        use std::sync::Arc;
        let at = |line: u32| Position {
            file: "a.loft".into(),
            line,
            pos: 1,
        };
        let mut spans: BTreeMap<u32, (Position, u32)> = BTreeMap::new();
        spans.insert(5, (at(10), 15));
        spans.insert(20, (at(20), 30));
        // A nested pair: the inner construct sits inside the outer one.
        spans.insert(100, (at(40), 200));
        spans.insert(120, (at(50), 130));
        let mut cx = CrashCx::default();
        cx.set_spans(Some(Arc::new(spans)));
        let source_loc_for_pc = |pc| cx.source_loc_for_pc(pc);
        let nearest_source_loc_for_pc = |pc| cx.nearest_source_loc_for_pc(pc);

        // Inside a span — its first pc, its middle, its last — is the position.
        assert_eq!(source_loc_for_pc(5).unwrap().line, 10);
        assert_eq!(source_loc_for_pc(7).unwrap().line, 10);
        assert_eq!(source_loc_for_pc(14).unwrap().line, 10);

        // One past the end is outside it.  This is the defect in miniature: the
        // construct is over, and its position no longer describes the pc.
        assert!(source_loc_for_pc(15).is_none());
        // A gap between two constructs used to answer the earlier one.
        assert!(source_loc_for_pc(45).is_none());
        // Past every span used to answer the last entry.
        assert!(source_loc_for_pc(250).is_none());
        // Before the first entry there was never anything to inherit.
        assert!(source_loc_for_pc(0).is_none());
        assert!(source_loc_for_pc(4).is_none());

        // Nesting: the innermost covering span wins, being the most precise.
        assert_eq!(source_loc_for_pc(125).unwrap().line, 50);
        // …and past the inner one the OUTER still covers.  This is why the walk
        // cannot stop at the first entry that fails to cover: the nearest
        // preceding span here is the inner one, which has ended, while the
        // enclosing construct is still the right answer.
        assert_eq!(source_loc_for_pc(150).unwrap().line, 40);
        assert_eq!(source_loc_for_pc(100).unwrap().line, 40);

        // The approximate reading stays available for the callers that present
        // it as approximate, and reports the pc it was recorded at so the
        // distance can be shown.
        let (recorded_at, pos) = nearest_source_loc_for_pc(45).expect("nearest");
        assert_eq!((recorded_at, pos.line), (20, 20));
        assert!(nearest_source_loc_for_pc(4).is_none());
    }

    #[test]
    fn source_loc_lookup_returns_none_when_no_snapshot() {
        assert!(CrashCx::default().source_loc_for_pc(42).is_none());
    }

    /// loft#820 — the op-name table costs 256 `Box::leak`s, so it may be built
    /// ONCE per process.  `execute_argv` publishes it once per PROGRAM, and a
    /// table taken by value was therefore built (and leaked) on every run, with
    /// only the first reaching the `OnceLock`.
    ///
    /// The assertion is on the SECOND call rather than on a build count, because
    /// this runs in a shared test binary where another test may already have
    /// published one — "a published table is never rebuilt" holds either way.
    #[test]
    fn a_published_op_name_table_is_never_rebuilt() {
        // Whether this publishes or finds one already published, a table exists
        // from here on.
        set_op_names(|| vec![""; 256]);

        let mut rebuilt = false;
        set_op_names(|| {
            rebuilt = true;
            vec![""; 256]
        });
        assert!(
            !rebuilt,
            "the table was rebuilt over a published one — every rebuild leaks 256 strings"
        );
    }
}
