// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I78 — Live-reload dispatch

//! @PLN18 08-S2 — live function dispatch: a compiled binary that can push
//! individual functions to the interpreter at runtime.
//!
//! The heart-of-the-engine tier flip.  Under `LOFT_LIVE_FLIP=1` a `--native`
//! binary bootstraps a parked interpreter [`State`] from the SAME sources the
//! binary was generated from (the loft driver hands the paths down via
//! `LOFT_LIVE_SRC` / `LOFT_LIVE_STDLIB` / `LOFT_LIVE_LIBS`), and the program
//! world — the `Stores` every generated fn runs over — is the BOOTSTRAP's
//! world, taken out of the parked State.  Every generated user fn opens with
//! a one-atomic-load entry check; flipping a fn routes its calls into the
//! interpreter over those same stores.
//!
//! **The sharing model is a swap, not an alias**: the parked State keeps a
//! placeholder `Stores`; each dispatched call swaps the program world into
//! `state.database`, runs [`State::reenter`]/[`State::reenter_ret`] (the 02
//! frame contract), and swaps it back out.  Single-threaded by construction —
//! the check fires inside the callee on the thread that bootstrapped; worker
//! threads fall through to the compiled body.
//!
//! Failure posture: every bootstrap problem WARNS and falls back to fully
//! compiled execution — live mode is an instrument, never a halt.

use std::cell::{RefCell, UnsafeCell};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::database::Stores;
use crate::keys::DbRef;
use crate::state::State;

/// Set once at a successful bootstrap; the first word of every entry check.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// `LOFT_DISPATCH_DEBUG=1` — one stderr line per dispatched call (the S2
/// observable: a flip must be visible ONLY here and in timing).
static DEBUG: AtomicBool = AtomicBool::new(false);
/// Per generated-fn-index flip switches (index = position in the emitted
/// `LOFT_LIVE_FNS` table).  Settable only for resolved fns.
static FLIPS: OnceLock<Box<[AtomicBool]>> = OnceLock::new();
/// Which indices resolved to a bootstrap (d_nr, code_position) — a flip on an
/// unresolved fn would dispatch into nothing, so `set_flip` refuses it.
static RESOLVED: OnceLock<Box<[bool]>> = OnceLock::new();
/// Total dispatched calls (all fns) — the S2 sentinel's counter.
static DISPATCHED: AtomicU64 = AtomicU64::new(0);

/// The parked interpreter.  The `Parser` (owning `Data`) and the `State` are
/// boxed BEFORE the raw `data_ptr`/`parallel_ctx` pointers are wired, so the
/// addresses those pointers capture stay stable for the program's lifetime.
struct Live {
    /// Never read after bootstrap, but load-bearing: it OWNS the `Data` the
    /// parked State's `data_ptr`/`parallel_ctx` raw pointers reference.
    _parser: Box<crate::parser::Parser>,
    state: Box<State>,
    /// Per generated fn index: the def number in the bootstrap world.  The
    /// CODE POSITION is deliberately not cached here — every dispatch
    /// resolves it through `state.fn_positions[d_nr]`, the one dispatch
    /// home tier-0 live reload patches (@PLN18 08-S3): an edit moves the
    /// target there, and a cached position would silently pin the old body.
    fns: Vec<u32>,
    names: &'static [&'static str],
}

thread_local! {
    static LIVE: RefCell<Option<Live>> = const { RefCell::new(None) };
}

/// The entry check every generated user fn opens with.  Cold path cost when
/// live mode is off: one relaxed atomic load.
#[inline]
pub fn live_flipped(idx: usize) -> bool {
    if !ENABLED.load(Ordering::Relaxed) {
        return false;
    }
    if !FLIPS
        .get()
        .is_some_and(|f| f.get(idx).is_some_and(|b| b.load(Ordering::Relaxed)))
    {
        return false;
    }
    // Worker threads have no parked State — they run the compiled body.
    LIVE.with(|l| l.borrow().is_some())
}

/// True after a successful bootstrap — generated `main` uses this to skip
/// `init(&cell)` (the bootstrap world is already fully seeded by the parse).
#[inline]
#[must_use]
pub fn live_enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Generated `main` calls this instead of `Stores::new()`.  Under
/// `LOFT_LIVE_FLIP=1` the returned stores are the bootstrap world (the parse
/// seeded types, CONST_STORE, const vectors — `init` must be skipped);
/// otherwise a plain `Stores::new()` (caller runs `init` as always).
///
/// `embedded_src` is the program's own source, emitted as a `LOFT_SRC` blob
/// (@PLN98 P3.1).  Source selection: when `LOFT_LIVE_SRC` is set (the loft
/// driver requesting the FILESYSTEM path — which also arms the tier-0 reload
/// watcher), bootstrap from the file; otherwise bootstrap from the EMBEDDED
/// bytes (a browser/wasm build, or a standalone binary run without the driver).
/// Either way the parked world is id-compatible with the compiled code.
#[must_use]
pub fn boot_stores(fn_names: &'static [&'static str], embedded_src: Option<&str>) -> Stores {
    if std::env::var("LOFT_LIVE_FLIP").is_ok_and(|v| v == "1") {
        let result = match embedded_src {
            // A live-reload FILE was named → the fs path (it watches for edits).
            Some(_) if std::env::var("LOFT_LIVE_SRC").is_ok() => bootstrap(fn_names),
            Some(src) => bootstrap_from_bytes(fn_names, src),
            None => bootstrap(fn_names),
        };
        match result {
            Ok(stores) => return stores,
            Err(msg) => eprintln!("loft-live: disabled — {msg}"),
        }
    }
    Stores::new()
}

/// Parse + byte-code the program this binary was generated from (reading its
/// source + stdlib from the FILESYSTEM via the `LOFT_LIVE_*` env the loft driver
/// sets), park the State, and hand its fully-seeded world out to the compiled
/// code.  The embedded-source twin is [`bootstrap_from_bytes`].
fn bootstrap(fn_names: &'static [&'static str]) -> Result<Stores, String> {
    let src = std::env::var("LOFT_LIVE_SRC")
        .map_err(|_| "LOFT_LIVE_SRC not set (run through the loft driver)".to_string())?;
    let stdlib = std::env::var("LOFT_LIVE_STDLIB").unwrap_or_else(|_| "default".to_string());
    let mut p = Box::new(crate::parser::Parser::new());
    p.parse_dir(&stdlib, true, false)
        .map_err(|e| format!("stdlib `{stdlib}`: {e}"))?;
    if let Ok(libs) = std::env::var("LOFT_LIVE_LIBS") {
        p.lib_dirs = libs
            .split(':')
            .filter(|s| !s.is_empty())
            .map(String::from)
            .collect();
    }
    p.parse(&src, false);
    if p.diagnostics.level() >= crate::diagnostics::Level::Error {
        return Err(format!("`{src}` does not parse clean"));
    }
    Ok(bootstrap_core(p, fn_names, Some((&src, &stdlib))))
}

/// @PLN98 P3.1 — bootstrap the live interpreter from EMBEDDED source: the baked-in
/// stdlib ([`stdlib_sources::STDLIB_SOURCES`](crate::stdlib_sources::STDLIB_SOURCES))
/// plus the program's own source passed as `program_src`, with no `default/`
/// directory and no `LOFT_LIVE_SRC` file to read.  This is the delivery a
/// browser/wasm live build needs (no filesystem): a generated `loft_start_live`
/// hands its emitted `LOFT_SRC` blob down here.  Parses the SAME world the fs
/// [`bootstrap`] would, then shares [`bootstrap_core`] — so the parked interpreter
/// is byte-for-byte the fs path's, minus the tier-0 reload watcher (no source file
/// to watch).
///
/// # Errors
/// When the embedded stdlib or the program does not parse clean.
pub fn bootstrap_from_bytes(
    fn_names: &'static [&'static str],
    program_src: &str,
) -> Result<Stores, String> {
    let mut p = Box::new(crate::parser::Parser::new());
    for (name, content) in crate::stdlib_sources::STDLIB_SOURCES {
        if !p.parse_source(content, name, true) {
            return Err(format!("embedded stdlib `{name}`: {}", p.diagnostics));
        }
    }
    p.parse_source(program_src, "program.loft", false);
    if p.diagnostics.level() >= crate::diagnostics::Level::Error {
        return Err("embedded program does not parse clean".to_string());
    }
    Ok(bootstrap_core(p, fn_names, None))
}

/// The shared tail of both bootstraps: byte-code the parsed world, wire the
/// interpreter's raw pointers, resolve the generated fn table, and PARK the State
/// (handing the seeded `Stores` out to the compiled code).  `reload` carries the
/// `(src, stdlib)` paths for the fs path's tier-0 live-reload watcher; `None` for
/// the embedded path (no source file to watch).
fn bootstrap_core(
    mut p: Box<crate::parser::Parser>,
    fn_names: &'static [&'static str],
    reload: Option<(&str, &str)>,
) -> Stores {
    crate::scopes::check(&mut p.data, &mut p.database);
    let mut state = Box::new(State::new(p.database.clone()));
    crate::compile::byte_code(&mut state, &mut p.data);

    // The prologue `execute_argv` would have done, minus running `main` —
    // `reenter` needs fn_positions (Call ops), and natives that spawn interp
    // workers need data_ptr / parallel_ctx.  Wired AFTER boxing: the raw
    // pointers capture the boxes' stable addresses.
    state.fn_positions = p.data.definitions.iter().map(|d| d.code_position).collect();
    let data_ptr = std::ptr::from_ref(&p.data);
    state.data_ptr = data_ptr;
    let stk_lib_nr = state
        .library_names
        .get("n_stack_trace")
        .copied()
        .unwrap_or(u16::MAX);
    state.database.parallel_ctx = Some(Box::new(crate::database::ParallelCtx {
        bytecode: &raw const state.bytecode,
        library: &raw const state.library,
        data: data_ptr,
        stack_trace_lib_nr: stk_lib_nr,
    }));
    crate::crash_report::set_source_spans(Some(std::sync::Arc::new(state.source_spans.clone())));

    // Resolve the generated fn table against the bootstrap world.
    let mut fns = Vec::with_capacity(fn_names.len());
    let mut resolved = Vec::with_capacity(fn_names.len());
    for name in fn_names {
        let d_nr = p.data.def_nr(name);
        fns.push(d_nr);
        resolved.push(d_nr != u32::MAX);
    }

    // @PLN18 08-S3 — tier-0 live reload over the parked interpreter: an
    // edit to a FLIPPED fn lands as new bytecode + a fn_positions patch
    // (live_reload.rs), which the per-dispatch resolution picks up.  The
    // compiled tier never changes — that is S4/S5's rebuild-and-swap.  Only the
    // fs bootstrap watches a source file; the embedded path passes `None`.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some((src, stdlib)) = reload
        && std::env::var("LOFT_LIVE_RELOAD").is_ok_and(|v| v != "0")
    {
        crate::live_reload::install(src, stdlib, &p.lib_dirs, &p.data);
    }
    #[cfg(target_arch = "wasm32")]
    let _ = reload;
    let _ = RESOLVED.set(resolved.into_boxed_slice());
    let _ = FLIPS.set(
        (0..fn_names.len())
            .map(|_| AtomicBool::new(false))
            .collect::<Vec<_>>()
            .into_boxed_slice(),
    );
    DEBUG.store(
        std::env::var("LOFT_DISPATCH_DEBUG").is_ok_and(|v| v == "1"),
        Ordering::Relaxed,
    );

    // The handover: the program runs over the BOOTSTRAP's world.  The parked
    // State keeps a placeholder; every dispatch swaps the world back in.
    let stores = std::mem::take(&mut state.database);
    LIVE.with(|l| {
        *l.borrow_mut() = Some(Live {
            _parser: p,
            state,
            fns,
            names: fn_names,
        });
    });
    ENABLED.store(true, Ordering::Relaxed);

    // Startup flips: LOFT_FLIP_FNS="tick,reply" (loft names, no n_ prefix).
    if let Ok(list) = std::env::var("LOFT_FLIP_FNS") {
        for name in list.split(',').filter(|s| !s.is_empty()) {
            if !set_flip(name, true) {
                eprintln!("loft-live: LOFT_FLIP_FNS names unknown fn `{name}` — ignored");
            }
        }
    }
    stores
}

/// Flip one fn (loft name, without the `n_` prefix) to the interpreter (`on`)
/// or back to compiled (`!on`).  Returns false when the name is unknown or
/// unresolved — a refused flip, never a halt.
pub fn set_flip(name: &str, on: bool) -> bool {
    LIVE.with(|l| {
        l.borrow()
            .as_ref()
            .is_some_and(|live| set_flip_in(live, name, on))
    })
}

/// The flip body against an already-borrowed [`Live`] — shared by the public
/// `set_flip` and the debug-control path (which may already hold the borrow).
fn set_flip_in(live: &Live, name: &str, on: bool) -> bool {
    let (Some(flips), Some(resolved)) = (FLIPS.get(), RESOLVED.get()) else {
        return false;
    };
    let full = format!("n_{name}");
    let Some(idx) = live
        .names
        .iter()
        .position(|n| **n == full)
        .filter(|&i| resolved[i])
    else {
        return false;
    };
    flips[idx].store(on, Ordering::Relaxed);
    if DEBUG.load(Ordering::Relaxed) {
        eprintln!(
            "live-flip: {full} -> {}",
            if on { "interp" } else { "compiled" }
        );
    }
    true
}

/// Total dispatched calls so far (the S2 sentinel's counter).
#[must_use]
pub fn dispatch_count() -> u64 {
    DISPATCHED.load(Ordering::Relaxed)
}

/// @PLN98 P3.4 — flip EVERY dispatchable fn to the interpreter and turn on the
/// dispatch trace.  A debug client runs its program over the parked interpreter +
/// the SHARED store (the tier's core primitive) so any fn can be breakpointed /
/// inspected; the browser `loft_start` debug variant calls this after
/// [`bootstrap_from_bytes`].  Each flipped call then routes through [`dispatch`]
/// (counted by [`dispatch_count`]).  Reports the flip count to the browser host so
/// a headless run can observe the tier switched on.
pub fn flip_all_dispatch_debug() {
    DEBUG.store(true, Ordering::Relaxed);
    let names: Vec<String> = LIVE.with(|l| {
        l.borrow().as_ref().map_or_else(Vec::new, |live| {
            live.names
                .iter()
                .filter_map(|n| n.strip_prefix("n_").map(String::from))
                .collect()
        })
    });
    let mut flipped = 0usize;
    for name in &names {
        if set_flip(name, true) {
            flipped += 1;
        }
    }
    wasm_host_log(&format!(
        "loft-debug: flipped {flipped} fn(s) to the interpreter\n"
    ));
}

/// @PLN98 P3.4 — write `msg` to the browser host's stdout (`loft_host_print`), so a
/// headless `--html` run can observe the debug tier's activity.  A no-op off the
/// browser wasm target (native / wasip2 / the `wasm` feature route their own way).
pub fn wasm_host_log(msg: &str) {
    #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
    crate::loft_host_print(msg.as_ptr(), msg.len());
    #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))))]
    let _ = msg;
}

/// @PLN98 P3.4 — run the cooperative debug cycle (P3.2) entirely inside the
/// current runtime and REPORT it, so a headless `--html` run can confirm the
/// interpreter's breakpoint pause + frame read + resume work on wasm32 (the
/// make-or-break the browser debugger needs).  Parses a tiny self-contained
/// program, sets a breakpoint, runs it — `execute_argv` returns at the breakpoint
/// (a cooperative yield, NOT a block) — reads the paused frame's arg, steps one
/// line (computing `m`), and resumes to completion.  The returned string encodes
/// the outcome (`PAUSE n=… STEP m=… DONE=…`); identical on native and wasm.
#[must_use]
pub fn wasm_debug_selftest() -> String {
    use crate::debugger::StepMode;
    let mut p = Box::new(crate::parser::Parser::new());
    for (name, content) in crate::stdlib_sources::STDLIB_SOURCES {
        if !p.parse_source(content, name, true) {
            return format!("FAIL parse-stdlib {name}");
        }
    }
    let prog = "fn compute(n: integer) -> integer {\n  m = n + 2;\n  m\n}\n\
                fn main() { compute(40); }\n";
    p.parse_source(prog, "selftest.loft", false);
    if p.diagnostics.level() >= crate::diagnostics::Level::Error {
        return "FAIL parse-prog".to_string();
    }
    crate::scopes::check(&mut p.data, &mut p.database);
    let mut state = Box::new(State::new(p.database.clone()));
    crate::compile::byte_code(&mut state, &mut p.data);
    let d = p.data.def_nr("n_compute");
    if d == u32::MAX || state.set_breakpoint_fn_line(d, 2, &p.data).is_none() {
        return "FAIL no-breakpoint".to_string();
    }
    state.enable_stepping();
    // execute_argv returns at the breakpoint (cooperative pause).
    state.execute_argv("main", &p.data, &[]);
    if !state.is_paused() {
        return "FAIL no-pause".to_string();
    }
    let n = state
        .paused_frame()
        .and_then(|f| {
            f.locals
                .iter()
                .find(|(k, _)| k == "n")
                .map(|(_, v)| v.clone())
        })
        .unwrap_or_else(|| "?".to_string());
    // Resume one step — the `m = n + 2` line — and read the computed value.
    let still = state.debug_step(StepMode::Into, &p.data);
    let m = if still {
        state
            .paused_frame()
            .and_then(|f| {
                f.locals
                    .iter()
                    .find(|(k, _)| k == "m")
                    .map(|(_, v)| v.clone())
            })
            .unwrap_or_else(|| "?".to_string())
    } else {
        "gone".to_string()
    };
    // Resume to completion.
    let done = !state.debug_step(StepMode::Continue, &p.data);
    format!("PAUSE n={n} STEP m={m} DONE={done}")
}

/// The dispatch chokepoint: swap the program world into the parked State,
/// re-enter the interpreter, swap it back out.  Every `live_call_*` routes
/// through here — the sentinel and the sharing model live in ONE place.
fn dispatch<R>(
    cell: &UnsafeCell<Stores>,
    idx: usize,
    run: impl FnOnce(&mut State, &crate::data::Data, u32, u32) -> R,
) -> R {
    LIVE.with(|l| {
        let mut borrow = l.borrow_mut();
        let live = borrow
            .as_mut()
            .expect("live_flipped() gates dispatch on a parked State");
        let d_nr = live.fns[idx];
        let n = DISPATCHED.fetch_add(1, Ordering::Relaxed) + 1;
        if DEBUG.load(Ordering::Relaxed) {
            eprintln!("live-dispatch: {} #{n}", live.names[idx]);
        }
        // The swap: generated callers up the stack hold `&mut Stores` derived
        // from this same UnsafeCell — the cell's ADDRESS stays put, only the
        // value moves, and no native frame touches it until we return.
        let in_cell = unsafe { &mut *cell.get() };
        std::mem::swap(&mut live.state.database, in_cell);
        // @PLN18 08-S3 — the reload poll runs with the world swapped IN
        // (an edit's new const texts append to the REAL CONST_STORE) and
        // BEFORE the position resolution, so the dispatch that follows an
        // edit already runs the new body.  Internally throttled (200 ms).
        #[cfg(not(target_arch = "wasm32"))]
        if crate::live_reload::active() && crate::live_reload::poll(&mut live.state) {
            // @PLN18 08-S7 — offsets moved; breakpoint identity must not.
            debug_reapply_bps(&mut live.state, &live._parser.data);
        }
        let pos = live.state.fn_positions[d_nr as usize];
        let state = &mut live.state;
        let data = &live._parser.data;
        let r = run(state, data, d_nr, pos);
        std::mem::swap(&mut live.state.database, in_cell);
        r
    })
}

/// Dispatch a `Void` fn.  `push` lands the args via [`State::put_stack`] in
/// declared order — the 02 probe's contract (and the generated signature's).
pub fn live_call_void(cell: &UnsafeCell<Stores>, idx: usize, push: impl FnOnce(&mut State)) {
    dispatch(cell, idx, |st, data, d_nr, pos| {
        if st.debug.is_some() {
            st.reenter_dbg::<()>(d_nr, pos, data, push, debug_pause_loop);
        } else {
            st.reenter(d_nr, pos, push);
        }
    });
}

/// One dispatched typed call, debug-aware: with breakpoints registered the
/// call runs under [`State::reenter_dbg`] (suspensions block in the pause
/// loop — mechanics stay alive); without, the plain fast path.
macro_rules! call_typed {
    ($cell:expr, $idx:expr, $push:expr, $t:ty) => {
        dispatch($cell, $idx, |st, data, d_nr, pos| {
            if st.debug.is_some() {
                st.reenter_dbg::<$t>(d_nr, pos, data, $push, debug_pause_loop)
            } else {
                st.reenter_ret::<$t>(d_nr, pos, $push)
            }
        })
    };
}

/// Dispatch an `integer`-returning fn.
pub fn live_call_i64(cell: &UnsafeCell<Stores>, idx: usize, push: impl FnOnce(&mut State)) -> i64 {
    call_typed!(cell, idx, push, i64)
}

/// Dispatch a `float`-returning fn.
pub fn live_call_f64(cell: &UnsafeCell<Stores>, idx: usize, push: impl FnOnce(&mut State)) -> f64 {
    call_typed!(cell, idx, push, f64)
}

/// Dispatch a `boolean`-returning fn (storage byte: 0/1/255).
pub fn live_call_u8(cell: &UnsafeCell<Stores>, idx: usize, push: impl FnOnce(&mut State)) -> u8 {
    call_typed!(cell, idx, push, u8)
}

/// Dispatch a record/vector/hash-returning fn (a `DbRef` into the shared world).
pub fn live_call_ref(
    cell: &UnsafeCell<Stores>,
    idx: usize,
    push: impl FnOnce(&mut State),
) -> DbRef {
    call_typed!(cell, idx, push, DbRef)
}

// ── @PLN18 08-S7 — the debugger loop: control-channel semantics ────────────
//
// The kernel (engine_host) owns the TRANSPORT (`D!:` frames, loopback +
// LOFT_DEBUG_CONTROL-gated); THIS module owns the semantics.  Commands that
// need the parked State arrive through a MAILBOX: when a dispatch is paused
// at a breakpoint, `LIVE` is mutably borrowed, so the control handler (which
// runs inside the pause loop's mini-pump) must never borrow it again — it
// posts, and the pause loop (which legally holds `&mut State`) applies.

/// A control command that needs the parked State (applied by the pause loop)
/// or arrived while one was borrowed.  `i64` = the requesting client's cid.
#[cfg(not(target_arch = "wasm32"))]
pub enum DebugCmd {
    SetBp(String, i64),
    Eval(String, i64),
    Resume(i64),
    Reload(i64),
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static DEBUG_MAILBOX: RefCell<Vec<DebugCmd>> = const { RefCell::new(Vec::new()) };
    /// Breakpoint NAMES — the identity that survives reloads and re-resolution.
    static DEBUG_BPS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// The control client that hears `D:hit` notifications.
    static DEBUG_SUB: std::cell::Cell<i64> = const { std::cell::Cell::new(-1) };
}

#[cfg(not(target_arch = "wasm32"))]
pub fn debug_mailbox_push(cmd: DebugCmd) {
    DEBUG_MAILBOX.with(|m| m.borrow_mut().push(cmd));
}

#[cfg(not(target_arch = "wasm32"))]
fn debug_mailbox_drain() -> Vec<DebugCmd> {
    DEBUG_MAILBOX.with(|m| std::mem::take(&mut *m.borrow_mut()))
}

/// Set a breakpoint at the entry of `name` (and flip it to the interpreter —
/// a compiled body cannot pause).  Direct when the parked State is free;
/// posted to the mailbox when a pause holds it (the pause loop applies and
/// acks).  Returns false only when the fn is unknown at direct application.
#[cfg(not(target_arch = "wasm32"))]
pub fn debug_set_bp(name: &str, cid: i64) -> bool {
    DEBUG_SUB.with(|s| s.set(cid));
    let direct = LIVE.with(|l| match l.try_borrow_mut() {
        Ok(mut guard) => guard.as_mut().map(|live| debug_apply_bp(live, name)),
        Err(_) => None, // paused: the pause loop applies it
    });
    if let Some(ok) = direct {
        ok
    } else {
        debug_mailbox_push(DebugCmd::SetBp(name.to_string(), cid));
        true
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn debug_apply_bp(live: &mut Live, name: &str) -> bool {
    let d_nr = live._parser.data.def_nr(&format!("n_{name}"));
    if d_nr == u32::MAX || live.state.set_breakpoint_fn_current(d_nr).is_none() {
        return false;
    }
    DEBUG_BPS.with(|b| {
        let mut b = b.borrow_mut();
        if !b.iter().any(|n| n == name) {
            b.push(name.to_string());
        }
    });
    set_flip_in(live, name, true);
    eprintln!("loft-debug: breakpoint at {name} (flipped to interp)");
    true
}

/// Re-resolve every name-keyed breakpoint against the CURRENT bodies — the
/// offsets move (a reload appends code and patches `fn_positions`); the
/// identity must not.  A fresh `Debugger` drops the stale offsets.
#[cfg(not(target_arch = "wasm32"))]
fn debug_reapply_bps(st: &mut State, data: &crate::data::Data) {
    let names = DEBUG_BPS.with(|b| b.borrow().clone());
    if names.is_empty() {
        return;
    }
    st.debug = None;
    st.enable_debug();
    for name in &names {
        let d_nr = data.def_nr(&format!("n_{name}"));
        if d_nr == u32::MAX || st.set_breakpoint_fn_current(d_nr).is_none() {
            eprintln!("loft-debug: breakpoint on {name} could not re-resolve");
        }
    }
}

/// On wasm the debug branch is dead code (breakpoints can only be set
/// through the native control channel), but the call sites still name the
/// pause handler — a typed no-op keeps the wasm build whole.
#[cfg(target_arch = "wasm32")]
fn debug_pause_loop(_st: &mut State, _data: &crate::data::Data, _hit: &crate::debugger::BreakHit) {}

/// THE PAUSE LOOP — runs between interpreter steps while a breakpoint holds
/// the dispatch.  Mechanics stay alive (the kernel mini-pump answers
/// keepalives and queues game events for after the resume); control commands
/// needing the State are applied here, where `&mut State` is legal.
#[cfg(not(target_arch = "wasm32"))]
fn debug_pause_loop(st: &mut State, data: &crate::data::Data, hit: &crate::debugger::BreakHit) {
    let sub = DEBUG_SUB.with(std::cell::Cell::get);
    let bindings = hit
        .locals
        .iter()
        .map(|(n, v)| format!("{n}={v}"))
        .collect::<Vec<_>>()
        .join("|");
    eprintln!("loft-debug: paused in {}", hit.function);
    crate::engine_host::debug_send(sub, &format!("D:hit {} {bindings}", hit.function));
    loop {
        crate::engine_host::debug_pump();
        for cmd in debug_mailbox_drain() {
            match cmd {
                DebugCmd::Resume(cid) => {
                    crate::engine_host::debug_send(cid, "D:resumed");
                    eprintln!("loft-debug: resumed");
                    return;
                }
                DebugCmd::Eval(name, cid) => {
                    let val = hit
                        .locals
                        .iter()
                        .find(|(n, _)| *n == name)
                        .map_or("<unknown>", |(_, v)| v.as_str());
                    crate::engine_host::debug_send(cid, &format!("D:eval {name}={val}"));
                }
                DebugCmd::Reload(cid) => {
                    // The S3 poll-now: the world is swapped IN at a pause, so
                    // the reload host sees the real CONST_STORE.  Offsets the
                    // edit moved re-resolve before anything fires again.
                    let grew = crate::live_reload::poll(st);
                    if grew {
                        debug_reapply_bps(st, data);
                    }
                    crate::engine_host::debug_send(
                        cid,
                        if grew {
                            "D:reload applied"
                        } else {
                            "D:reload unchanged"
                        },
                    );
                }
                DebugCmd::SetBp(name, cid) => {
                    // Applied with the State at hand (LIVE is borrowed, so
                    // no set_flip — the fn is interpreted ALREADY if we are
                    // paused inside it; otherwise the flip lands on the next
                    // un-paused set).
                    let d_nr = data.def_nr(&format!("n_{name}"));
                    let ok = d_nr != u32::MAX && st.set_breakpoint_fn_current(d_nr).is_some();
                    if ok {
                        DEBUG_BPS.with(|b| b.borrow_mut().push(name.clone()));
                    }
                    crate::engine_host::debug_send(
                        cid,
                        &format!("D:{} bp {name}", if ok { "ok" } else { "err" }),
                    );
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

// ── @PLN18 08-S4 — the background rebuild ──────────────────────────────────
//
// The serve host compiles the full project while the old build keeps
// serving.  The build pipeline is the loft DRIVER's own `--check --native`
// (it compiles into the content-hash cache and prints the artifact path on
// the ok line); this module only orchestrates: spawn it as a background
// child, snapshot the loft source at request time, and on completion
// REQUEUE when the source changed mid-build (the staleness contract).
// Stdout/stderr go to temp FILES, not pipes — a failed rustc prints more
// than a pipe buffer holds, and a blocked child would hang the poll.

/// Rebuild status codes (the loft-visible `rebuild_status()` contract).
#[cfg(not(target_arch = "wasm32"))]
const REBUILD_IDLE: i64 = 0;
#[cfg(not(target_arch = "wasm32"))]
const REBUILD_BUILDING: i64 = 1;
#[cfg(not(target_arch = "wasm32"))]
const REBUILD_READY: i64 = 2;
#[cfg(not(target_arch = "wasm32"))]
const REBUILD_FAILED: i64 = 3;

#[cfg(not(target_arch = "wasm32"))]
enum Rebuild {
    Idle,
    Building {
        child: std::process::Child,
        /// The loft source bytes at spawn — completion compares against the
        /// CURRENT file; a mismatch means the artifact is already stale.
        snapshot: Vec<u8>,
        out_path: std::path::PathBuf,
        err_path: std::path::PathBuf,
    },
    Ready {
        artifact: String,
    },
    Failed,
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static REBUILD: RefCell<Rebuild> = const { RefCell::new(Rebuild::Idle) };
    static REBUILD_SEQ: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Spawn one background `--check --native` build of `src` via the driver.
#[cfg(not(target_arch = "wasm32"))]
fn spawn_build(driver: &str, src: &str) -> Result<Rebuild, String> {
    let snapshot = std::fs::read(src).map_err(|e| format!("cannot read {src}: {e}"))?;
    let seq = REBUILD_SEQ.with(|c| {
        c.set(c.get() + 1);
        c.get()
    });
    let base = std::env::temp_dir().join(format!("loft_rebuild_{}_{seq}", std::process::id()));
    let out_path = base.with_extension("out");
    let err_path = base.with_extension("err");
    let out = std::fs::File::create(&out_path).map_err(|e| e.to_string())?;
    let err = std::fs::File::create(&err_path).map_err(|e| e.to_string())?;
    let mut cmd = std::process::Command::new(driver);
    cmd.arg("--no-warnings").arg("--check").arg("--native");
    if let Ok(libs) = std::env::var("LOFT_LIVE_LIBS") {
        for d in libs.split(':').filter(|s| !s.is_empty()) {
            cmd.arg("--lib").arg(d);
        }
    }
    // A build legitimately takes minutes — never under the run watchdog.
    cmd.arg(src)
        // Ask the driver for the machine form of its ok line: "ok <src> <artifact>".
        // Without it the driver answers a person, and prints `ok` alone.
        .env("LOFT_CHECK_ARTIFACT", "1")
        .env_remove("LOFT_TIMEOUT")
        .stdout(out)
        .stderr(err)
        .stdin(std::process::Stdio::null());
    let child = cmd
        .spawn()
        .map_err(|e| format!("cannot spawn {driver}: {e}"))?;
    Ok(Rebuild::Building {
        child,
        snapshot,
        out_path,
        err_path,
    })
}

/// Start a background rebuild of the program this binary serves.  True =
/// building (idempotent while one runs); false = the host lacks the live
/// env (run through the loft driver) or the spawn failed — warned, never
/// a halt.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_start() -> bool {
    let (Ok(driver), Ok(src)) = (
        std::env::var("LOFT_LIVE_DRIVER"),
        std::env::var("LOFT_LIVE_SRC"),
    ) else {
        eprintln!(
            "loft-live: rebuild needs LOFT_LIVE_DRIVER/LOFT_LIVE_SRC (run through the loft driver)"
        );
        return false;
    };
    REBUILD.with(|r| {
        let mut r = r.borrow_mut();
        if matches!(*r, Rebuild::Building { .. }) {
            return true; // idempotent: one build at a time
        }
        match spawn_build(&driver, &src) {
            Ok(b) => {
                eprintln!("loft-live: rebuild started ({src})");
                *r = b;
                true
            }
            Err(msg) => {
                eprintln!("loft-live: rebuild failed to start — {msg}");
                *r = Rebuild::Failed;
                false
            }
        }
    })
}

/// Poll the rebuild: 0 idle, 1 building, 2 ready, 3 failed.  Completion
/// with a source that changed mid-build REQUEUES automatically (status
/// stays 1) — the artifact must correspond to the source as of the check.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_status() -> i64 {
    REBUILD.with(|r| {
        let mut r = r.borrow_mut();
        let Rebuild::Building {
            child,
            snapshot,
            out_path,
            err_path,
        } = &mut *r
        else {
            return match &*r {
                Rebuild::Idle => REBUILD_IDLE,
                Rebuild::Ready { .. } => REBUILD_READY,
                _ => REBUILD_FAILED,
            };
        };
        let status = match child.try_wait() {
            Ok(Some(st)) => st,
            Ok(None) => return REBUILD_BUILDING,
            Err(e) => {
                eprintln!("loft-live: rebuild wait failed — {e}");
                *r = Rebuild::Failed;
                return REBUILD_FAILED;
            }
        };
        let src = std::env::var("LOFT_LIVE_SRC").unwrap_or_default();
        if !status.success() {
            let tail: String = std::fs::read_to_string(err_path)
                .unwrap_or_default()
                .lines()
                .rev()
                .take(5)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            eprintln!("loft-live: rebuild FAILED (old build keeps serving):\n{tail}");
            *r = Rebuild::Failed;
            return REBUILD_FAILED;
        }
        // The artifact path rides the driver's ok line: "ok <src> <artifact>".
        let out = std::fs::read_to_string(out_path).unwrap_or_default();
        let artifact = out
            .lines()
            .find_map(|l| l.strip_prefix(&format!("ok {src} ")))
            .unwrap_or("")
            .trim()
            .to_string();
        let now = std::fs::read(&src).unwrap_or_default();
        if now != *snapshot {
            // Stale: the source changed while rustc ran.  Requeue with the
            // current content — the cache makes an already-built version
            // instant, so this converges as soon as the source settles.
            eprintln!("loft-live: rebuild stale (source changed during build) — requeued");
            let driver = std::env::var("LOFT_LIVE_DRIVER").unwrap_or_default();
            *r = match spawn_build(&driver, &src) {
                Ok(b) => b,
                Err(msg) => {
                    eprintln!("loft-live: requeue failed — {msg}");
                    Rebuild::Failed
                }
            };
            return match &*r {
                Rebuild::Building { .. } => REBUILD_BUILDING,
                _ => REBUILD_FAILED,
            };
        }
        if artifact.is_empty() || !std::path::Path::new(&artifact).exists() {
            eprintln!("loft-live: rebuild succeeded but no artifact on the ok line ({out:?})");
            *r = Rebuild::Failed;
            return REBUILD_FAILED;
        }
        eprintln!("loft-live: rebuild ready — {artifact}");
        *r = Rebuild::Ready { artifact };
        REBUILD_READY
    })
}

/// Control-channel shims (@PLN18 08-S7) — the kernel's debug handler drives
/// the rebuild machinery without reaching the private internals.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn n_rebuild_start_ctl() -> bool {
    rebuild_start()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rebuild_status_ctl() -> i64 {
    rebuild_status()
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn rebuild_artifact_ctl() -> String {
    rebuild_artifact()
}

/// The READY build's artifact path; "" unless `rebuild_status() == 2`.
#[cfg(not(target_arch = "wasm32"))]
fn rebuild_artifact() -> String {
    REBUILD.with(|r| match &*r.borrow() {
        Rebuild::Ready { artifact } => artifact.clone(),
        _ => String::new(),
    })
}

// ── loft-visible surface (lib/engine_host: `live_flip(name, on)`) ──────────

/// Typed twin for generated code: `live_flip(name: text, on: boolean) -> boolean`.
pub fn n_live_flip(_cell: &UnsafeCell<Stores>, name: &str, on: u8) -> u8 {
    u8::from(set_flip(name, on == 1))
}

/// Interp stack native: under the interpreter everything is already
/// interpreted, so a flip is a no-op `false` — the differential stays clean.
pub fn n_live_flip_stack(stores: &mut Stores, stack: &mut DbRef) {
    let _on = stores.get::<u8>(stack);
    let _name = stores.get::<crate::keys::Str>(stack).str().to_owned();
    stores.put(stack, false);
}

/// Typed twin: `rebuild_start() -> boolean`.
#[cfg(not(target_arch = "wasm32"))]
pub fn n_rebuild_start(_cell: &UnsafeCell<Stores>) -> u8 {
    u8::from(rebuild_start())
}

/// Typed twin: `rebuild_status() -> integer`.
#[cfg(not(target_arch = "wasm32"))]
pub fn n_rebuild_status(_cell: &UnsafeCell<Stores>) -> i64 {
    rebuild_status()
}

/// Typed twin: `kernel_rebuild_artifact() -> text` (owned String).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_rebuild_artifact(_cell: &UnsafeCell<Stores>) -> String {
    rebuild_artifact()
}

/// Interp stack natives.  An interpreted serve host CAN rebuild too — the
/// machinery is env-driven, not tier-driven (the driver sets the live env
/// on native spawns only, so today these fire there as a warned no-op; an
/// interp host that exports the env gets the real thing for free).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_rebuild_start_stack(stores: &mut Stores, stack: &mut DbRef) {
    stores.put(stack, rebuild_start());
}

#[cfg(not(target_arch = "wasm32"))]
pub fn n_rebuild_status_stack(stores: &mut Stores, stack: &mut DbRef) {
    stores.put(stack, rebuild_status());
}

/// Destination-passing text return (the `is_text_dest_native` route).
#[cfg(not(target_arch = "wasm32"))]
pub fn n_kernel_rebuild_artifact_dest(stores: &mut Stores, stack: &mut DbRef) {
    let dest = stores.get::<DbRef>(stack);
    let v = rebuild_artifact();
    stores
        .store_mut(&dest)
        .addr_mut::<String>(dest.rec, dest.pos)
        .push_str(&v);
}

#[cfg(test)]
mod tests {
    use crate::parser::Parser;

    /// Parse the fs stdlib + `program_path` the way [`bootstrap`] does
    /// (`parse_dir` + `parse`), through the final `scopes::check`, and
    /// return `(def_count, n_main_resolves)`.
    fn parse_fs(program_path: &str) -> (u32, bool) {
        let mut p = Parser::new();
        p.parse_dir("default", true, false).expect("parse default/");
        assert!(p.parse(program_path, false), "fs program parses");
        crate::scopes::check(&mut p.data, &mut p.database);
        (p.data.definitions(), p.data.def_nr("n_main") != u32::MAX)
    }

    /// Parse the EMBEDDED stdlib + `program_src` the way [`bootstrap_from_bytes`]
    /// does (`parse_source` over `STDLIB_SOURCES`), through the final
    /// `scopes::check`.
    fn parse_embedded(program_src: &str) -> (u32, bool) {
        let mut p = Parser::new();
        for (name, content) in crate::stdlib_sources::STDLIB_SOURCES {
            assert!(
                p.parse_source(content, name, true),
                "embedded stdlib {name} parses"
            );
        }
        assert!(
            p.parse_source(program_src, "program.loft", false),
            "embedded program parses"
        );
        crate::scopes::check(&mut p.data, &mut p.database);
        (p.data.definitions(), p.data.def_nr("n_main") != u32::MAX)
    }

    // @PLN98 P3.1 — the embedded-source parse must park the SAME world the fs path
    // does (the browser build has no `default/` dir / `LOFT_LIVE_SRC` file): same
    // def-count, `n_main` resolved, stdlib present.
    #[test]
    fn bootstrap_from_bytes_parses_a_fs_identical_world() {
        let program = "fn main() {\n  print(\"hi\")\n}\n";
        let path = std::env::temp_dir().join(format!("loft_p31_{}.loft", std::process::id()));
        std::fs::write(&path, program).unwrap();
        let (fs_defs, fs_main) = parse_fs(path.to_str().unwrap());
        let (emb_defs, emb_main) = parse_embedded(program);
        let _ = std::fs::remove_file(&path);
        assert!(fs_main && emb_main, "n_main resolves in both paths");
        assert!(
            fs_defs > 100,
            "stdlib is present (fs def count = {fs_defs})"
        );
        assert_eq!(
            emb_defs, fs_defs,
            "embedded parse parks the same def-count as fs"
        );
    }

    // @PLN98 P3.1 — the full embedded bootstrap (parse + byte-code + wire + park)
    // succeeds and hands out a seeded world.  Parks the process-global LIVE
    // interpreter, so it relies on nextest's per-test process isolation.
    #[test]
    fn bootstrap_from_bytes_ok() {
        static FNS: &[&str] = &["n_main"];
        let stores = super::bootstrap_from_bytes(FNS, "fn main() {\n  print(\"hi\")\n}\n")
            .expect("bootstrap_from_bytes succeeds");
        assert!(!stores.types.is_empty(), "parked world carries a schema");
    }
}

#[cfg(test)]
mod p98_selftest_tests {
    // The debug selftest (cooperative pause + frame read + resume) must produce
    // the expected outcome natively; the SAME code runs on wasm (verified via the
    // html_wasm harness), so a divergence there is a wasm bug.
    #[test]
    fn wasm_debug_selftest_runs_the_cooperative_cycle() {
        let r = super::wasm_debug_selftest();
        assert_eq!(r, "PAUSE n=40 STEP m=42 DONE=true", "debug cycle: {r}");
    }
}
