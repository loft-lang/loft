// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I75 — Diagnostics collector
//
// Minimal crash reporter: installs a SIGSEGV / SIGABRT / SIGBUS
// signal handler that prints the last-executed opcode, bytecode
// position, and function name from a thread-local context marker
// updated by the interpreter's execute loop.
//
// Goal: when the interpreter crashes inside an opcode that corrupts
// memory (heap overflow, stack overflow, use-after-free), the
// glibc / kernel error arrives OUT of Rust's panic path — `set_hook`
// only catches panics, not signals.  Without a native handler the
// process just aborts with no context.
//
// Usage: call [`install`] once near program start (or at test
// harness init).  The interpreter's execute loop calls
// [`set_context`] before each opcode dispatch to publish the
// current PC / function / op-name.  On a crash the handler reads
// that context and prints a one-line diagnostic to stderr before
// the default handler runs.
//
// Design notes:
//
// - Async-signal safety: signal handlers are extremely restricted —
//   we cannot call `format!`, `println!`, or allocate.  We format
//   directly into a thread-local `[u8; N]` buffer and write it with
//   `libc::write(STDERR_FILENO, ...)`.  Everything is
//   reentrant/async-signal-safe.
// - Thread-local context: each thread publishes its own context.
//   On crash we read the local thread's context only; worker threads
//   get their own trace.
// - No allocation: the buffer is fixed-size; the fields are all
//   fixed-width types (u32 pc, u16 op code) or a `&'static str`
//   (the op name, which is a compile-time constant from the
//   interpreter's opcode table).

#![allow(clippy::module_name_repetitions)]

use std::cell::Cell;
use std::cell::RefCell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    /// Last opcode dispatched on this thread.  (pc, op_name_ptr, op_name_len).
    /// Updated by [`set_context`] from the interpreter's inner loop.
    static LAST_CTX: Cell<Ctx> = const { Cell::new(Ctx::EMPTY) };

    /// Plan-07 phase 1 step 1.20 / phase 3 — pc → source-position table
    /// snapshot for the running interpreter.  `State::execute_argv`
    /// publishes a clone here on entry so the panic hook (a process-wide
    /// non-allocating-restricted callback) can resolve the offending
    /// pc to `file:line:col` when a Rust panic fires inside a loft
    /// runtime fault (panic builtin, arithmetic overflow, future
    /// div-by-zero kinds).  `None` outside the execute window.
    static SOURCE_SPANS: std::cell::RefCell<Option<std::sync::Arc<std::collections::BTreeMap<u32, (crate::lexer::Position, u32)>>>> = const { std::cell::RefCell::new(None) };
}

// On non-Unix platforms the signal-handler consumer is compiled
// out, so the fields look unread to rustc.  The thread-local
// `set_context` writes still happen (and the tests read them),
// so `#[allow(dead_code)]` is correct here for both targets.
#[derive(Clone, Copy)]
#[allow(dead_code)]
struct Ctx {
    pc: u32,
    fn_d_nr: u32,
    op_code: u8,
    op_name: &'static str,
    fn_name: &'static str,
}

impl Ctx {
    const EMPTY: Ctx = Ctx {
        pc: u32::MAX,
        fn_d_nr: u32::MAX,
        op_code: 0,
        op_name: "",
        fn_name: "",
    };
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

/// The name of opcode `op`, or `""` when no table was published.
///
/// Gated to match its only caller, the `cfg(unix)` signal handler.  The table is still
/// PUBLISHED on every target — `set_op_names` runs from `execute_argv` unconditionally —
/// but nothing reads it where there is no handler to read it from, so an ungated
/// definition is dead code on Windows and warns there while building clean on Linux.
#[cfg(unix)]
fn op_name_of(op: u8) -> &'static str {
    OP_NAMES
        .get()
        .and_then(|v| v.get(op as usize).copied())
        .unwrap_or("")
}

/// The name of the opcode that was dispatching on this thread, or `""` if no table has
/// been published yet.
///
/// The signal handler is not the only reader that needs this. A runtime invariant
/// breach reported from deep inside the store layer — `BUG (#306)`, a strict-store
/// violation — has a `pc` and nothing else, and a bare `pc` is a position in the WHOLE
/// bytecode stream (stdlib included) that no tool a reader has in hand can resolve.
/// Naming the op turns "something freed store 0" into "this op freed store 0", which is
/// the difference between a report and a lead (loft#920).
#[must_use]
pub fn last_op_name() -> &'static str {
    let op = LAST_CTX.with(|c| c.get().op_code);
    OP_NAMES
        .get()
        .and_then(|v| v.get(op as usize).copied())
        .unwrap_or("")
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
/// Unix only, like the signal handler that fills it: a target with no fatal
/// signals to catch has no report to lose.
#[cfg(unix)]
static CRASH_FILE: OnceLock<CrashFile> = OnceLock::new();

/// A crash-report destination: the same path twice, once NUL-terminated for
/// `libc::open` and once printable for the stderr line that names it.
#[cfg(unix)]
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
#[cfg(unix)]
fn choose_crash_file() -> Option<CrashFile> {
    crash_file_from(std::env::var("LOFT_CRASH_FILE").ok())
}

/// The destination for a given `LOFT_CRASH_FILE` setting — the decision in
/// [`choose_crash_file`], separated from reading the environment so it can be
/// tested without mutating process-wide state (which races every other test in
/// the binary, and is `unsafe` besides).
#[cfg(unix)]
fn crash_file_from(setting: Option<String>) -> Option<CrashFile> {
    let path = match setting {
        // Explicitly emptied — the caller wants stderr only.
        Some(p) if p.is_empty() => return None,
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let name = format!("loft-crash-{}.txt", std::process::id());
            let dot_loft = std::path::Path::new(".loft");
            if dot_loft.is_dir() {
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
/// which is also every non-unix target (there is no signal handler to feed it).
#[must_use]
pub fn crash_file_path() -> Option<String> {
    #[cfg(unix)]
    {
        CRASH_FILE.get().map(|c| c.display.clone())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// Update the per-thread context just before an opcode dispatches.
///
/// Call this AT MOST ONCE per opcode.  The inner loop overhead is
/// a single thread-local store (one atomic-less write on most
/// platforms) so the hot path stays cheap.
#[inline]
pub fn set_context(
    pc: u32,
    op_code: u8,
    op_name: &'static str,
    fn_d_nr: u32,
    fn_name: &'static str,
) {
    LAST_CTX.with(|c| {
        c.set(Ctx {
            pc,
            fn_d_nr,
            op_code,
            op_name,
            fn_name,
        });
    });
}

/// Read the last-dispatched opcode context on this thread.
/// Returns (pc, op_code, fn_d_nr).  Used by debug sentinels (e.g.
/// the `put_stack`/`get_stack` DbRef-bounds check) to report which
/// op was executing when a stack value turned out to be corrupt.
#[must_use]
#[allow(dead_code)]
pub fn last_context() -> (u32, u8, u32) {
    LAST_CTX.with(|c| {
        let ctx = c.get();
        (ctx.pc, ctx.op_code, ctx.fn_d_nr)
    })
}

/// Plan-07 phase 1 step 1.20 / phase 3 — publish a snapshot of the
/// current `State::source_spans` so the panic hook can look up
/// `at file:line:col` for the offending pc.  Pass `None` to clear.
pub fn set_source_spans(
    spans: Option<std::sync::Arc<std::collections::BTreeMap<u32, (crate::lexer::Position, u32)>>>,
) {
    SOURCE_SPANS.with(|s| {
        *s.borrow_mut() = spans;
    });
}

/// Plan-07 phase 1 step 1.20 / phase 3 — the source position of the construct
/// that CONTAINS `pc`, from the current thread's published span snapshot.
///
/// `None` when no snapshot is active or no recorded span covers `pc`.  Answering
/// `None` is the point: the table is sparse, so most pcs have a nearest preceding
/// entry that belongs to an unrelated earlier statement, and rendering that as
/// `at file:line:col` sends a reader to a line the fault is not on with nothing to
/// mark it a guess (loft#1262).  A caller that wants the approximate answer, and
/// can present it as one, calls [`nearest_source_loc_for_pc`].
///
/// Spans nest, so the walk is backwards and the first covering entry is the
/// innermost — the most precise position available.
#[must_use]
pub fn source_loc_for_pc(pc: u32) -> Option<crate::lexer::Position> {
    SOURCE_SPANS.with(|s| {
        let borrow = s.borrow();
        borrow.as_ref().and_then(|m| {
            m.range(..=pc)
                .rev()
                .find(|(_, (_, end))| *end > pc)
                .map(|(_, (p, _))| p.clone())
        })
    })
}

/// The nearest span recorded at or before `pc`, with the pc it was recorded at.
///
/// This is the approximate reading, and a caller must present it as one — the
/// distance between the two pcs is how much to trust it.  Use it only where a
/// rough location beats none; where the output looks like an exact position,
/// [`source_loc_for_pc`] is the one that can be trusted.
#[must_use]
pub fn nearest_source_loc_for_pc(pc: u32) -> Option<(u32, crate::lexer::Position)> {
    SOURCE_SPANS.with(|s| {
        let borrow = s.borrow();
        borrow.as_ref().and_then(|m| {
            m.range(..=pc)
                .next_back()
                .map(|(at, (p, _))| (*at, p.clone()))
        })
    })
}

thread_local! {
    /// The loft source position the compiler is currently working on.
    ///
    /// A signal handler cannot allocate, so the SIGSEGV path above resolves its
    /// position from a pc.  A PANIC handler runs on the normal stack and may
    /// allocate freely, which is what lets this one carry a real `Position`.
    static COMPILE_POS: RefCell<Option<crate::lexer::Position>> = const { RefCell::new(None) };
}

/// Publish the loft source position the compiler is currently working on, so an
/// internal panic can point at the USER's program instead of at a compiler
/// source file (loft#665 piece 3).
///
/// Called at two chokepoints — per source line while parsing, and per definition
/// while generating code — rather than at the thousands of individual `assert!` /
/// `unwrap` sites, which is the only reason this is one mechanism instead of a
/// spray that would be silently incomplete the moment anyone adds an assert.
pub fn note_compile_pos(pos: &crate::lexer::Position) {
    COMPILE_POS.with(|p| {
        if let Ok(mut slot) = p.try_borrow_mut() {
            *slot = Some(pos.clone());
        }
    });
}

/// Forget the current compile position — call once compilation is done, so a
/// RUNTIME panic is not misattributed to the last line that was compiled.
pub fn clear_compile_pos() {
    COMPILE_POS.with(|p| {
        if let Ok(mut slot) = p.try_borrow_mut() {
            *slot = None;
        }
    });
}

#[must_use]
pub fn compile_pos() -> Option<crate::lexer::Position> {
    COMPILE_POS.with(|p| p.try_borrow().ok().and_then(|b| b.clone()))
}

/// Render an internal compiler panic as a loft diagnostic pointing at the user's
/// source, then defer to the previous hook for the Rust-side detail.
///
/// A compiler-internal invariant that breaks is still something the USER
/// triggered with a specific program, and until now they were shown only
/// `thread 'main' panicked at src/state/codegen.rs:2955` naming a generated
/// symbol and an argument count they never wrote (loft#662).  Nothing in that
/// says which line of their program to look at, or that a bug report is wanted.
///
/// Wrapping the hook rather than replacing it keeps the Rust location and
/// backtrace, which are what make the report actionable.
pub fn install_panic_hook() {
    if PANIC_HOOK_INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // @PLN133 S8 — a fault inside a lazy DRIVER is contained and reported
        // through `store_lazy_error`, so it is not a crash and must not be
        // announced as one. Printing it would be worse than noise: a lookup that
        // correctly answered null would look like a program that fell over.
        if crate::codegen_runtime::in_lazy_driver() {
            return;
        }
        if let Some(pos) = compile_pos() {
            let detail = info
                .payload()
                .downcast_ref::<&str>()
                .map(|s| (*s).to_string())
                .or_else(|| info.payload().downcast_ref::<String>().cloned())
                .unwrap_or_else(|| "internal error".to_string());
            let at = info
                .location()
                .map_or_else(String::new, |l| format!(" [{}:{}]", l.file(), l.line()));
            let msg = format!(
                "internal compiler error — please report this program at \
                 https://github.com/loft-lang/loft/issues\n  {detail}{at}"
            );
            let mut diags = crate::diagnostics::Diagnostics::new();
            diags.add_at(
                crate::diagnostics::Level::Fatal,
                &msg,
                &pos.file,
                pos.line,
                pos.pos,
            );
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
        prev(info);
    }));
}

/// Install signal handlers for SIGSEGV / SIGABRT / SIGBUS.
///
/// No-op on non-Unix platforms and when called more than once.
pub fn install(program: &'static str) {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = PROGRAM.set(program);
    // Resolve the report path BEFORE the handlers are armed, so a crash the
    // instant after this call still has somewhere to write.
    #[cfg(unix)]
    if let Some(f) = choose_crash_file() {
        let _ = CRASH_FILE.set(f);
    }
    #[cfg(unix)]
    unsafe {
        for &sig in &[libc::SIGSEGV, libc::SIGABRT, libc::SIGBUS] {
            let mut act: libc::sigaction = std::mem::zeroed();
            act.sa_sigaction = handler as *const () as libc::sighandler_t;
            // SA_SIGINFO for the siginfo/ucontext args we ignore here; SA_RESETHAND so
            // the default handler runs after we print (produces the core dump).
            act.sa_flags = libc::SA_SIGINFO | libc::SA_RESETHAND;
            libc::sigemptyset(&raw mut act.sa_mask);
            libc::sigaction(sig, &raw const act, std::ptr::null_mut());
        }
    }
}

/// Async-signal-safe handler.  Reads the thread-local context and
/// writes a one-line diagnostic to stderr; the default handler
/// then takes over (which produces a core dump if `ulimit -c` is
/// set).
#[cfg(unix)]
extern "C" fn handler(sig: libc::c_int, _info: *mut libc::siginfo_t, _ucontext: *mut libc::c_void) {
    // Read the context.  If the interpreter wasn't running, EMPTY
    // fields produce a "no context" message — still useful to
    // confirm the signal fired.
    let ctx = LAST_CTX.with(Cell::get);
    // Plan-07 phase 3 — try to resolve the offending pc to a loft
    // source position.  This is technically not async-signal-safe
    // (`RefCell::try_borrow` reads a counter that another borrow
    // could be mutating mid-signal), but in practice the
    // SOURCE_SPANS RefCell is mutated only once per `execute_argv`
    // entry — a microsecond window — and `try_borrow` returns Err
    // (we then skip the source-loc print) instead of panicking on
    // a conflict.  Worst case: the crash report omits the source
    // line, which is the same as the pre-phase-3 behaviour.
    // The span AND the pc it was recorded at.  The lookup is "the last span at or
    // before pc", which is only the crash site when a span actually covers that pc
    // — and the table is sparse (one entry per statement, and none at all for
    // regions no statement produced).  Unqualified, it reported loft#806's fault
    // at `default/05_coroutine.loft:18:27`, a stdlib line the program never calls,
    // and sent the reader there.  Carrying the DISTANCE turns a confident wrong
    // answer into a reading that says how much to trust it: `+0` is the statement,
    // `+3121` is "the nearest thing recorded, three thousand bytes back".
    let source_loc = SOURCE_SPANS.with(|s| {
        s.try_borrow().ok().and_then(|borrow| {
            borrow
                .as_ref()
                .and_then(|m| m.range(..=ctx.pc).next_back())
                .map(|(at, (p, _))| (*at, p.clone()))
        })
    });
    let sig_name = match sig {
        libc::SIGSEGV => "SIGSEGV",
        libc::SIGABRT => "SIGABRT",
        libc::SIGBUS => "SIGBUS",
        _ => "signal",
    };
    let program = PROGRAM.get().copied().unwrap_or("loft");
    // Build message into a fixed-size buffer, async-signal-safe.
    let mut buf = [0u8; 768];
    let mut w = Writer::new(&mut buf);
    let _ = w.str("\n=== loft crash (");
    let _ = w.str(program);
    let _ = w.str(") ");
    let _ = w.str(sig_name);
    let _ = w.str(" caught ===\n  last op:  ");
    if ctx.op_name.is_empty() {
        let _ = w.str("(none — crash outside interpreter)\n");
    } else {
        // The opcode's own name when the table reached us, else the dispatch
        // label — never just the number, which names nothing.
        let named = op_name_of(ctx.op_code);
        let _ = w.str(if named.is_empty() { ctx.op_name } else { named });
        let _ = w.str(" (op=");
        let _ = w.u32(u32::from(ctx.op_code));
        let _ = w.str(")\n  pc:       ");
        let _ = w.u32(ctx.pc);
        let _ = w.str("\n  fn:       ");
        let _ = w.str(if ctx.fn_name.is_empty() {
            "(?)"
        } else {
            ctx.fn_name
        });
        let _ = w.str(" (d_nr=");
        let _ = w.u32(ctx.fn_d_nr);
        let _ = w.str(")\n");
        // Plan-07 phase 3 — emit `at file:line:col` when the source
        // span lookup succeeded.  Truncate file path to fit; the
        // user can still grep for it.
        if let Some((span_pc, pos)) = source_loc.as_ref() {
            let _ = w.str("  at:      ");
            let _ = w.str(&pos.file);
            let _ = w.str(":");
            let _ = w.u32(pos.line);
            let _ = w.str(":");
            let _ = w.u32(pos.pos);
            // How far the crashing pc is PAST the statement this line describes.
            // Zero means the line is the site; anything large means the table had
            // nothing nearer and the line is a lower bound, not an answer.
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
    let _ = w.str("===\n");
    let bytes = w.as_bytes();
    unsafe {
        let _ = libc::write(
            libc::STDERR_FILENO,
            bytes.as_ptr().cast::<libc::c_void>(),
            bytes.len(),
        );
    }
    // loft#717 — the same bytes to a FILE, so a pipeline that filters stderr
    // cannot swallow the one diagnostic that can never be regenerated.  Write
    // it BEFORE announcing the path, so the line below is only printed when
    // there is really something at the other end of it.
    if let Some(target) = CRASH_FILE.get() {
        let wrote = unsafe {
            // `open`/`write`/`close` are all on POSIX's async-signal-safe list,
            // and `c_path` was NUL-terminated at install time, so nothing here
            // allocates or formats.  0o600: a crash dump names internals, so it
            // is readable by its owner only.
            let fd = libc::open(
                target.c_path.as_ptr().cast::<libc::c_char>(),
                libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC,
                0o600 as libc::c_int,
            );
            if fd < 0 {
                false
            } else {
                let n = libc::write(fd, bytes.as_ptr().cast::<libc::c_void>(), bytes.len());
                libc::close(fd);
                n > 0
            }
        };
        if wrote {
            let mut note = [0u8; 1024];
            let mut nw = Writer::new(&mut note);
            let _ = nw.str("  report also written to: ");
            let _ = nw.str(&target.display);
            let _ = nw.str("\n");
            let note_bytes = nw.as_bytes();
            unsafe {
                let _ = libc::write(
                    libc::STDERR_FILENO,
                    note_bytes.as_ptr().cast::<libc::c_void>(),
                    note_bytes.len(),
                );
            }
        }
    }
    // SA_RESETHAND → the default handler fires next, producing the
    // core dump and terminating the process.
}

// `Writer` and its methods are pure Rust — available on every
// platform so the `#[cfg(test)]` unit tests compile uniformly.
// Only the signal-handler path that invokes it is `#[cfg(unix)]`,
// so on non-unix non-test builds (e.g. WASM release) it looks dead.
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
        set_context(10, 7, "OpVarInt", 42, "main");
        LAST_CTX.with(|c| {
            let ctx = c.get();
            assert_eq!(ctx.pc, 10);
            assert_eq!(ctx.op_code, 7);
            assert_eq!(ctx.op_name, "OpVarInt");
            assert_eq!(ctx.fn_d_nr, 42);
            assert_eq!(ctx.fn_name, "main");
        });
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
    #[cfg(unix)]
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
        set_source_spans(Some(Arc::new(spans)));

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

        // Cleanup so other tests don't see this snapshot.
        set_source_spans(None);
    }

    #[test]
    fn source_loc_lookup_returns_none_when_no_snapshot() {
        // Ensure the thread-local is unset at the start.
        set_source_spans(None);
        assert!(source_loc_for_pc(42).is_none());
    }

    /// loft#665 piece 3 — the compile position an internal panic reports.  It must
    /// be publishable, readable, and above all CLEARABLE: a runtime panic that
    /// still saw a stale compile position would point the user at whatever line
    /// happened to compile last, which is worse than saying nothing.
    #[test]
    fn compile_pos_round_trips_and_clears() {
        clear_compile_pos();
        assert!(compile_pos().is_none(), "starts unset");

        let pos = crate::lexer::Position {
            file: "prog.loft".into(),
            line: 12,
            pos: 5,
        };
        note_compile_pos(&pos);
        let got = compile_pos().expect("published position is readable");
        assert_eq!((&*got.file, got.line, got.pos), ("prog.loft", 12, 5));

        // A later position replaces the earlier one — the report wants where the
        // compiler IS, not where it started.
        let later = crate::lexer::Position {
            file: "prog.loft".into(),
            line: 30,
            pos: 1,
        };
        note_compile_pos(&later);
        assert_eq!(compile_pos().unwrap().line, 30);

        // Handing off to the runtime drops it, so a runtime panic is not
        // misattributed to the last line compiled.
        clear_compile_pos();
        assert!(
            compile_pos().is_none(),
            "cleared at the compile/run boundary"
        );
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
