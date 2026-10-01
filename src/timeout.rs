// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F48 — loft CLI (--timeout execution watchdog)

//! @PLAN49 T1+T3 — process-wide execution timeout with a watchdog
//! thread and a shared breadcrumb.
//!
//! The watchdog is the **guaranteed-termination** layer: a background
//! thread sleeps until `T + grace` and then `process::abort()`s.  It
//! does not depend on the executing thread cooperating, so it fires
//! even when control is stuck inside arbitrary Rust/native code, a
//! blocking syscall, or a deadlock.
//!
//! The shared breadcrumb is the *only* diagnostic the watchdog can
//! report — it runs on a different thread and can't safely read the
//! main thread's call stack.  Parser, interpreter, and native codegen
//! call `checkpoint_*` at coarse-grained checkpoints (file entry, fn
//! entry, lexer recovery loop) so when the watchdog fires the printed
//! breadcrumb localises the hang as far as possible.
//!
//! ## Cost when disabled (no `--timeout` / `LOFT_TIMEOUT`)
//!
//! Every checkpoint starts with `if !ARMED.load(Relaxed) { return; }`.
//! That's **one relaxed atomic load + one branch-predicted not-taken
//! branch** — the optimizer keeps the load (it has acquire semantics
//! for thread visibility) but the branch is essentially free.  No
//! allocation, no mutex, no syscall: ~1–2 ns per call site.  Designed
//! to be sprinkled at fn-entry without measurable runtime cost.
//!
//! ## Cost when armed
//!
//! The mutex-backed checkpoints take one `try_lock` + a few field
//! writes (~50–100 ns): native fn-entry (once per loft fn call),
//! parse (once per file), lexer recovery (throttled to every 256
//! iterations).  None is in a hot opcode dispatch loop.
//!
//! The interpreter's per-call breadcrumb ([`checkpoint_interp_call`],
//! loft#952) is built differently *because* it is the frequent one:
//! two relaxed atomic stores, no mutex and no allocation, with the
//! clock-reading deadline test throttled to every 1024th call.  At
//! 20 M calls, armed and disarmed are within run-to-run noise.

use std::sync::Mutex;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Master "is the timeout active?" flag.  Every checkpoint's first
/// instruction is an atomic load of this — when `false` the
/// breadcrumb update is skipped entirely.
static ARMED: AtomicBool = AtomicBool::new(false);

/// When was the timeout armed?  `None` once means timeout disabled —
/// `deadline_reached` always returns `false`, the watchdog never
/// fires.  Set once via `arm`.
static DEADLINE: OnceLock<Instant> = OnceLock::new();

/// The configured timeout in seconds, stored at `arm` so the cooperative
/// (T2) exit can report it.  `0` while disarmed.
static TIMEOUT_SECS: AtomicU64 = AtomicU64::new(0);

/// Shared breadcrumb — read by the watchdog before it aborts, written
/// by parse-time / runtime / codegen checkpoints.  Held under a
/// `Mutex` so the watchdog can `try_lock` and skip without blocking
/// if the main thread is mid-update (rare, and the watchdog is the
/// last word — losing one byte of breadcrumb is acceptable).
static BREADCRUMB: Mutex<Breadcrumb> = Mutex::new(Breadcrumb {
    phase: "",
    fn_name: "",
    file: String::new(),
    line: 0,
});

/// loft#952 — the interpreter's most recently entered loft function, as a `d_nr`
/// into [`INTERP_FNS`].  `u32::MAX` = none entered yet.
///
/// The interpreter cannot use [`checkpoint_fn`]'s `&'static str`: its function names
/// live in the `Data` table, so passing one would mean leaking a `String` per call.
/// An index costs a relaxed store and resolves on the watchdog's side, where the cost
/// is irrelevant because the process is about to die.
static INTERP_FN: AtomicU32 = AtomicU32::new(u32::MAX);

/// Calls counted by [`checkpoint_interp_call`], so it can test the deadline on a
/// fraction of them rather than reading the clock on every loft call.
static INTERP_CALLS: AtomicU32 = AtomicU32::new(0);

/// The names [`INTERP_FN`] indexes, as `(fn name, file, line)` — the function's own
/// DECLARATION position, which is what `--native` reports too, so a hang localises the
/// same way on both backends.
///
/// Published once per program by [`publish_interp_fns`] and only when the watchdog is
/// armed, so an ordinary run neither builds nor holds it.
static INTERP_FNS: OnceLock<Vec<(String, String, u32)>> = OnceLock::new();

/// loft#952 — the entry point the interpreter was last asked to run, which under
/// `--tests` is the TEST function's own name.
///
/// Separate from [`INTERP_FN`] because they answer different questions and the reporter
/// needed both: that one says which function the run was executing, this one says which
/// test it got there from.  `--tests` sweeps in every sibling file it can reach, so
/// "which of dozens of tests" was the part that had to be recovered by grepping raw
/// output.  Written once per test, so a `Mutex<String>` costs nothing that matters.
static INTERP_ENTRY: Mutex<String> = Mutex::new(String::new());

struct Breadcrumb {
    /// Coarse execution phase: "parse", "run-interpret", "run-native",
    /// or "" before any checkpoint fires.  Always a string literal.
    phase: &'static str,
    /// Most recent fn name we entered.  Always a string literal (lib
    /// names or codegen-emitted literals); no leak risk.  "" before
    /// any checkpoint fires.
    fn_name: &'static str,
    /// Source filename of whatever code is currently being processed.
    /// Owned because the parser passes a runtime `&str` (filename
    /// from argv); we copy on update so it stays valid.
    file: String,
    /// Line in `file` of the last advanced checkpoint.
    line: u32,
}

/// Arm the watchdog for `timeout_secs` total execution time.  The
/// graceful diagnostic deadline is at `timeout_secs`; the hard kill
/// is at `timeout_secs + grace_secs`.  Idempotent — second call is
/// a no-op (the first deadline + watchdog remain in effect).
///
/// Pass `0` to leave the timeout disarmed.
///
/// # Panics
/// Panics if the OS refuses to spawn the `loft-watchdog` thread.
pub fn arm(timeout_secs: u64, grace_secs: u64) {
    if timeout_secs == 0 {
        return;
    }
    if ARMED.swap(true, Ordering::SeqCst) {
        return;
    }
    TIMEOUT_SECS.store(timeout_secs, Ordering::SeqCst);
    let now = Instant::now();
    let deadline = now + Duration::from_secs(timeout_secs);
    let hard = deadline + Duration::from_secs(grace_secs);
    let _ = DEADLINE.set(deadline);
    std::thread::Builder::new()
        .name("loft-watchdog".to_string())
        .spawn(move || {
            // Two sleeps, not a polling loop.  At the deadline the running interpreter is
            // asked to stop: its dispatch loop tests one flag after every op, so a loop
            // stops gracefully whether or not it calls anything.  At the hard deadline the
            // process is aborted, for code that never returns to the dispatch loop.
            if let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                std::thread::sleep(remaining);
            }
            request_stop();
            if let Some(remaining) = hard.checked_duration_since(Instant::now()) {
                std::thread::sleep(remaining);
            }
            print_breadcrumb_and_abort(timeout_secs, grace_secs);
        })
        .expect("loft: failed to spawn watchdog thread");
}

/// The dispatch-stop flag of the interpreter running on the main thread, while it runs
/// (`StopFlag`); the watchdog sets it at the deadline.  Behind a mutex so the flag cannot be
/// unpublished — and its `Stores` dropped — between the watchdog reading the pointer and
/// writing through it.
static STOP_FLAG: Mutex<usize> = Mutex::new(0);

/// Published while a dispatch loop of an armed run executes; unpublished on drop.
pub struct StopFlag {
    published: bool,
}

/// Publish `flag` for the watchdog (`@FR-R-DispatchStop`), if a deadline is armed and no
/// other loop has published one — a nested or worker loop leaves the outermost's in place.
#[must_use]
pub fn publish_stop_flag(flag: &std::sync::atomic::AtomicBool) -> StopFlag {
    if !ARMED.load(Ordering::Relaxed) {
        return StopFlag { published: false };
    }
    let mut slot = STOP_FLAG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *slot != 0 {
        return StopFlag { published: false };
    }
    *slot = std::ptr::from_ref(flag) as usize;
    // A deadline that passed before this loop started is a request already.
    if deadline_reached() {
        flag.store(true, Ordering::Relaxed);
    }
    StopFlag { published: true }
}

impl Drop for StopFlag {
    fn drop(&mut self) {
        if self.published {
            *STOP_FLAG
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = 0;
        }
    }
}

fn request_stop() {
    let slot = STOP_FLAG
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if *slot != 0 {
        // SAFETY: published by a live `StopFlag` and unpublished, under this lock, before the
        // flag's `Stores` can go away.
        let flag = unsafe { &*(*slot as *const std::sync::atomic::AtomicBool) };
        flag.store(true, Ordering::Relaxed);
    }
}

/// The dispatch loop's graceful stop at the deadline, naming the function whose frame runs.
pub fn stop_at_deadline(d_nr: u32) -> ! {
    if d_nr != u32::MAX {
        INTERP_FN.store(d_nr, Ordering::Relaxed);
    }
    graceful_exit();
}

/// True iff a deadline was armed and has been reached.  Cheap (one
/// atomic load + Instant compare) — callers can sprinkle this in
/// hot paths without observable cost when the timeout is disabled.
#[must_use]
#[inline]
pub fn deadline_reached() -> bool {
    if !ARMED.load(Ordering::Relaxed) {
        return false;
    }
    DEADLINE.get().is_some_and(|d| Instant::now() >= *d)
}

/// @PLAN49 T2 — cooperative graceful exit.  A `checkpoint_*` on the
/// executing thread observed the deadline has passed: print the (current,
/// thread-local-accurate) breadcrumb and exit cleanly with `124` (the GNU
/// `timeout` convention) — *before* the watchdog's `T + grace` hard-kill, so
/// in the common case the watchdog is a no-op and the user gets a clean,
/// localised timeout report.  When execution is genuinely wedged in Rust
/// (no checkpoint runs past `T`), only the watchdog fires — the guarantee.
#[cold]
fn graceful_exit() -> ! {
    let (phase, fn_name, file, line) = report_fields();
    eprintln!(
        "[timeout] deadline reached after {}s (graceful): phase={} fn={} file={}:{}{}{}",
        TIMEOUT_SECS.load(Ordering::Relaxed),
        phase,
        fn_name,
        file,
        line,
        entry_suffix(&fn_name),
        blocked_suffix()
    );
    std::process::exit(124);
}

/// What this process is BLOCKED on, if anything, and since when (loft#1238).
///
/// The breadcrumb answers *where in the program are we*; a process waiting on a cross-process
/// lock is still "in parse" by that measure, and both timeout reports said so — `phase=parse`
/// for a process that was not parsing, was not going to parse, and would have been unblocked by
/// nothing it could do. The one fact that explains the kill is the one neither report carried.
///
/// Measured: a feature example hit its 60s budget with `lockwait` recorded and no `lockheld`,
/// i.e. killed while queued behind another process's cold cdylib build. Reading that took the
/// timing ledger, two lines of it, and knowing the convention that an unmatched `lockwait` means
/// "still waiting". It should take reading the kill message.
static BLOCKED: std::sync::Mutex<Option<(String, std::time::Instant)>> =
    std::sync::Mutex::new(None);

/// Note that this process is about to block on `what` — a cross-process lock, a subprocess build.
/// Pair with [`unblocked`]. Cheap and inert when no timeout is armed.
pub fn blocked_on(what: &str) {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut b) = BLOCKED.try_lock() {
        *b = Some((what.to_string(), std::time::Instant::now()));
    }
}

/// Time left before the armed deadline, or `None` when no timeout is armed.
#[must_use]
pub fn remaining() -> Option<std::time::Duration> {
    DEADLINE
        .get()
        .map(|d| d.saturating_duration_since(Instant::now()))
}

/// Has this process already reported giving up on `key`?  One report per subject per process.
///
/// The resolution is attempted more than once in a run (pass 1 and pass 2 both resolve `use`),
/// and each attempt legitimately gives up — the second having waited only what was left of the
/// budget. Printing the full explanation each time turns one fact into a wall, and the second
/// figure (`after 0.8s`) reads like a different, faster failure.
pub fn first_giveup_report(key: &str) -> bool {
    static SEEN: std::sync::Mutex<Option<Vec<String>>> = std::sync::Mutex::new(None);
    let Ok(mut g) = SEEN.lock() else {
        return true;
    };
    let seen = g.get_or_insert_with(Vec::new);
    if seen.iter().any(|k| k == key) {
        return false;
    }
    seen.push(key.to_string());
    true
}

/// Acquire an advisory file lock, giving up rather than letting the wait consume the run's
/// budget.  `Err(waited)` means it gave up; the caller reports and fails.
///
/// **Why a bounded wait (loft#1238).**  The global native-build lock serialises cold cdylib
/// builds across processes, which is right.  Under a parallel test runner every process reaches
/// a stale artifact at once, and the ones at the back of the queue were killed by their own
/// timeout while still waiting — SIGABRT, `phase=parse`, nothing built, nothing stamped, so the
/// next attempt repeated it.  An unbounded wait inside a time-budgeted run cannot succeed; it can
/// only convert a queue into a hard kill.
///
/// So when a deadline is armed the wait is bounded by what is left of it, minus a margin big
/// enough for the caller to report and exit cleanly.  The run still fails — the artifact really
/// is missing — but it fails SAYING SO, with the lock named and the wait measured, instead of
/// being aborted mid-queue.
///
/// With no deadline armed this blocks exactly as before: an interactive build should wait for
/// the lock however long the other build takes, because it has no budget to lose.
///
/// # Errors
///
/// `Err(waited)` when a deadline is armed and the lock was still held with too little budget
/// left to use it — `waited` is how long this process queued before giving up.
pub fn lock_within_budget(f: &std::fs::File, what: &str) -> Result<(), std::time::Duration> {
    let started = Instant::now();
    let Some(budget) = remaining() else {
        // Unarmed: the historical behaviour, and the right one — nothing is counting.
        blocked_on(what);
        let _ = f.lock();
        unblocked();
        return Ok(());
    };
    // Leave room to report and unwind. Never more than half the remaining budget, so a short
    // deadline still gets a real (if brief) attempt rather than an instant refusal.
    let margin = std::time::Duration::from_millis(1500).min(budget / 2);
    let deadline = started + budget.saturating_sub(margin);
    blocked_on(what);
    loop {
        if f.try_lock().is_ok() {
            unblocked();
            return Ok(());
        }
        if Instant::now() >= deadline {
            unblocked();
            return Err(started.elapsed());
        }
        // Coarse on purpose: this is a queue measured in seconds, and a tight spin would burn
        // the CPU the holder needs to finish.
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// Clear the blocked note set by [`blocked_on`].
pub fn unblocked() {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut b) = BLOCKED.try_lock() {
        *b = None;
    }
}

/// ` blocked=<what> for <n>s` when this process is waiting on something, else empty.
///
/// `try_lock` like [`report_fields`]: a held or poisoned mutex costs the note, never the kill.
fn blocked_suffix() -> String {
    match BLOCKED.try_lock() {
        Ok(b) => match &*b {
            Some((what, since)) => {
                format!(" blocked={what} for {:.1}s", since.elapsed().as_secs_f64())
            }
            None => String::new(),
        },
        Err(_) => String::new(),
    }
}

/// The `phase` / `fn` / `file` / `line` both timeout reports print, with the
/// interpreter's per-call breadcrumb (loft#952) preferred over the coarse one.
///
/// The interpreter position wins where it exists because it is strictly more specific:
/// the coarse breadcrumb's last word during an interpreted run is `execute_argv`'s
/// one-shot `"<entry>"`, which names nothing.  Unset fields render as `?` rather than
/// empty, so a placeholder is visibly a placeholder.
fn report_fields() -> (&'static str, String, String, u32) {
    let (phase, fn_name, file, line) = match BREADCRUMB.try_lock() {
        Ok(bc) => (bc.phase, bc.fn_name, bc.file.clone(), bc.line),
        Err(_) => ("", "", String::new(), 0),
    };
    let (fn_name, file, line) = match interp_position() {
        Some((n, f, l)) => (n.to_string(), f.to_string(), l),
        None => (fn_name.to_string(), file, line),
    };
    (
        if phase.is_empty() { "?" } else { phase },
        if fn_name.is_empty() {
            "?".to_string()
        } else {
            fn_name
        },
        if file.is_empty() {
            "?".to_string()
        } else {
            file
        },
        line,
    )
}

/// Native / interpret function-entry checkpoint — combines phase +
/// fn name + (file, line) into ONE mutex acquisition.  All inputs are
/// `&'static str` (string literals from generated code or library
/// names), so we store them by pointer without leaking.
///
/// Hot path when disabled: one relaxed atomic load + branch, return.
#[inline]
pub fn checkpoint_fn(phase: &'static str, name: &'static str, file: &'static str, line: u32) {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut bc) = BREADCRUMB.try_lock() {
        bc.phase = phase;
        bc.fn_name = name;
        if bc.file != file {
            bc.file.clear();
            bc.file.push_str(file);
        }
        bc.line = line;
    }
    if deadline_reached() {
        graceful_exit();
    }
}

/// loft#952 — give the interpreter the per-function breadcrumb `--native` has had all
/// along, so a watchdog hard-kill names the loft function it died in.
///
/// Before this, the interpreter checkpointed ONCE, with the literal `"<entry>"`, so
/// every interpreted hang reported `fn=<entry> file=?:0` — a placeholder, not a
/// location.  A slow test therefore became an undebuggable `SIGABRT`, and the only way
/// to find the culprit was grepping the raw output for repeating text.
///
/// Cheaper than [`checkpoint_fn`] rather than more expensive: two relaxed stores
/// instead of a mutex `try_lock`.  Disarmed it is the same single load and branch every
/// other checkpoint costs.
///
/// Also the interpreter's cooperative deadline check.  `--native` exits gracefully at
/// `T` because its fn-entry checkpoint tests the deadline; the interpreter had nowhere
/// to test it and so could only ever be hard-killed at `T + grace`.  Now the common
/// case is a clean `124` naming the function, and the watchdog stays the guarantee for
/// a hang with no loft call in it.
#[inline]
pub fn checkpoint_interp_call(d_nr: u32) {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    INTERP_FN.store(d_nr, Ordering::Relaxed);
    // The deadline test is THROTTLED, the breadcrumb above is not.  `deadline_reached`
    // reads the clock, and a loft call is frequent enough that doing so on every one
    // would be a measurable tax on interpreted code — while the breadcrumb, two relaxed
    // stores, is not.  Every 1024th call bounds the graceful exit's lateness by
    // microseconds of loft execution, which is far inside the grace window the watchdog
    // gives it.
    let n = INTERP_CALLS.fetch_add(1, Ordering::Relaxed);
    if n.is_multiple_of(1024) && deadline_reached() {
        graceful_exit();
    }
}

/// Tell [`checkpoint_interp_call`]'s breadcrumb what the program's functions are called,
/// so the watchdog can turn a `d_nr` into a name and a source position.
///
/// A whole-table sweep once per program rather than a hook on each call — the same shape
/// (and the same reason) as `Stores::publish_type_names`: it cannot miss a function the
/// way a per-site hook can, and it is inert when nothing will read it.
pub fn publish_interp_fns(names: impl IntoIterator<Item = (String, String, u32)>) {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    let _ = INTERP_FNS.set(names.into_iter().collect());
}

/// Record the entry point about to run — under `--tests`, the test's own name.
///
/// Unlike [`publish_interp_fns`] this is set per RUN, not per program: a `--tests`
/// invocation executes many entries against one name table.
pub fn note_interp_entry(name: &str) {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut e) = INTERP_ENTRY.try_lock() {
        e.clear();
        e.push_str(name);
    }
}

/// ` entry=<name>` for the run's entry point, or empty when there is nothing to add.
///
/// Suppressed when the entry IS the reported function, so the common single-`main` run
/// does not print the same name twice — the field earns its place only under `--tests`,
/// where it names the test a stuck helper was reached from.
fn entry_suffix(fn_name: &str) -> String {
    match INTERP_ENTRY.try_lock() {
        Ok(e) if !e.is_empty() && *e != fn_name => format!(" entry={e}"),
        _ => String::new(),
    }
}

/// The interpreter breadcrumb as `(fn name, file, line)`, or `None` when no loft
/// function has been entered or the name table was never published.
fn interp_position() -> Option<(&'static str, &'static str, u32)> {
    let d_nr = INTERP_FN.load(Ordering::Relaxed);
    if d_nr == u32::MAX {
        return None;
    }
    let (name, file, line) = INTERP_FNS.get()?.get(d_nr as usize)?;
    // `INTERP_FNS` is a `OnceLock` that is never cleared, so its contents live as long
    // as the process — which outlives every reader, all of which are about to end it.
    Some((name.as_str(), file.as_str(), *line))
}

/// Parser checkpoint — `file` is a runtime `&str` (filename from
/// argv / `use` resolution), so we copy into the breadcrumb's owned
/// `String`.  Called once per parse() entry plus once every N lexer
/// recovery iterations.
///
/// Hot path when disabled: one relaxed atomic load + branch, return.
#[inline]
pub fn checkpoint_parse(file: &str, line: u32) {
    if !ARMED.load(Ordering::Relaxed) {
        return;
    }
    if let Ok(mut bc) = BREADCRUMB.try_lock() {
        bc.phase = "parse";
        bc.fn_name = "";
        if bc.file != file {
            bc.file.clear();
            bc.file.push_str(file);
        }
        bc.line = line;
    }
    if deadline_reached() {
        graceful_exit();
    }
}

fn print_breadcrumb_and_abort(timeout: u64, grace: u64) {
    // `report_fields` try_locks rather than locks, so a poisoned or held mutex costs
    // the breadcrumb, not the hard-kill (the GUARANTEE).
    let (phase, fn_name, file, line) = report_fields();
    eprintln!(
        "[timeout] hard-kill after {}s+{}s grace: \
         phase={} fn={} file={}:{}{}{}",
        timeout,
        grace,
        phase,
        fn_name,
        file,
        line,
        entry_suffix(&fn_name),
        blocked_suffix()
    );
    // `process::abort()` raises SIGABRT — useful for debugging
    // (core dump on `ulimit -c`).  Test/CI runs may prefer the
    // cleaner `_exit`; pick via env `LOFT_TIMEOUT_CLEAN_EXIT`.
    if std::env::var("LOFT_TIMEOUT_CLEAN_EXIT").is_ok() {
        std::process::exit(124); // GNU `timeout` convention
    }
    std::process::abort();
}

/// Parse `LOFT_TIMEOUT=<seconds>` from the env.  Returns `0` if
/// unset / malformed.  Companion to the `--timeout` CLI flag (which
/// the main binary parses directly and passes to `arm`).
#[must_use]
pub fn env_timeout_secs() -> u64 {
    std::env::var("LOFT_TIMEOUT")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

/// Grace margin between the cooperative diagnostic deadline `T` and
/// the hard-kill `T + grace`.  Default 2 seconds — long enough for a
/// graceful T2 dump to complete, short enough to keep the user-visible
/// kill snappy.  Overridable via `LOFT_TIMEOUT_GRACE=<seconds>`.
#[must_use]
pub fn env_grace_secs() -> u64 {
    std::env::var("LOFT_TIMEOUT_GRACE")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(2)
}
