// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: the frames a native thread is running

//! What a native thread is running: the loft frames on its shadow call stack, and whether it
//! is inside a lazy store's driver.
//!
//! Generated code pushes and pops the frames (`cr_call_push` / `cr_call_pop`, re-exported by
//! `codegen_runtime` under the paths emitted Rust names); a fault report reads them, which is
//! why this sits in the core layer with the stores and the error renderer rather than with the
//! rest of the native runtime (@PLN191).

use std::cell::{Cell, UnsafeCell};

thread_local! {
    /// @PLN133 S8 — is this thread inside a lazy DRIVER right now?
    ///
    /// It changes what a fault MEANS. An ordinary loft call that overflows the
    /// stack is a program bug and halting is right; the same overflow inside a
    /// FETCH is not, because @PLN129's contract is @C80 — a failed fetch reports
    /// through `store_lazy_error` and the lookup answers null. So a buggy driver
    /// must not turn a lookup into a program halt, which is what moving the
    /// source from Rust to loft would otherwise have cost.
    ///
    /// Thread-local rather than global: a `par` worker running its own fetch is
    /// its own answer.
    static IN_LAZY_DRIVER: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Is a fault on this thread a driver's, and therefore containable?
///
/// Read by `cr_stack_overflow`, which exits the process for an ordinary overflow
/// and unwinds for this one, and by the panic hook, which stays silent for a
/// fault that is about to be reported through arc C's channel instead.
#[must_use]
pub fn in_lazy_driver() -> bool {
    IN_LAZY_DRIVER.with(std::cell::Cell::get)
}

/// Mark this thread as running (or no longer running) a lazy driver — the window
/// [`in_lazy_driver`] answers for.
pub fn set_in_lazy_driver(inside: bool) {
    IN_LAZY_DRIVER.with(|f| f.set(inside));
}

// Thread-local shadow call stack used by native-compiled loft code.
// Each entry is (function_name, source_file, line_number).
// Generated code calls `cr_call_push` / `cr_call_pop` around every function body.
//
// The live prefix is `CALL_FRAMES[..CALL_DEPTH]`: a pop only decrements the
// depth, leaving its entry as garbage the next push at that depth overwrites.
// Split this way (loft#1426 M1, @PLN157 P1) the per-call cost is a bounds test,
// three word stores and two `Cell` updates — the `RefCell<Vec>` form's borrow
// flag, length update and re-borrowing pop measured ~7 ns of the ~18 ns a
// native call costs.  Every reader slices `[..depth]`; none can fail the way a
// `try_borrow` could, because a read never observes a half-made entry — the
// depth is bumped only after the entry is fully written, and the readers
// (frame renderers, `stack_trace()`) run between pushes, not inside one.
thread_local! {
    static CALL_DEPTH: Cell<usize> = const { Cell::new(0) };
    /// `(name, file, declaration line, call site)` per frame: the call site is the line
    /// this frame is calling from, written by [`cr_call_site`] before each call.
    static CALL_FRAMES: UnsafeCell<Vec<(&'static str, &'static str, u32, u32)>> =
        const { UnsafeCell::new(Vec::new()) };
}

/// Read the live frames — `CALL_FRAMES[..CALL_DEPTH]` — without copying.
///
/// The `min` clamp is load-bearing for the LEAN tier (@PLN157): a `--lean`
/// binary advances the depth through [`cr_call_push_lean`] without writing
/// frames, so the array can be shorter than the depth — its readers then see
/// whatever named frames exist (none, usually) instead of panicking inside
/// the diagnostic they serve.
///
/// SAFETY (of the internal `&*ptr`): the only `&mut` into `CALL_FRAMES` lives
/// inside `cr_call_push`'s statement on this same thread and is gone by the
/// time any reader runs; entries below `depth` are fully initialised because
/// the depth is published after the write.
pub(crate) fn with_call_frames<R>(
    f: impl FnOnce(&[(&'static str, &'static str, u32, u32)]) -> R,
) -> R {
    let depth = CALL_DEPTH.get();
    CALL_FRAMES.with(|frames| {
        let v = unsafe { &*frames.get() };
        f(&v[..depth.min(v.len())])
    })
}

/// Where the running native frame was declared, for a RECOVERABLE fault's log line — the same
/// position the halting path names on `--native` (`State::running_frame_declaration` is the
/// interpreter's twin), because the generated code keeps no per-statement line.  `None` on the
/// lean tier, which pushes no frames, and outside any loft frame.
pub(crate) fn running_frame_position() -> Option<crate::lexer::Position> {
    with_call_frames(|frames| {
        frames
            .last()
            .map(|(_, file, line, _)| crate::lexer::Position {
                file: crate::lexer::intern_file(file),
                line: *line,
                pos: 1,
            })
    })
}

/// The loft frames this native thread is inside, innermost first.
///
/// The `--native` counterpart of the interpreter's `State::current_call_chain`, in the
/// same shape (bare function names) so that one renderer serves both backends —
/// `RuntimeError::report_and_exit` attaches these to a fault raised by `assert` or
/// `panic`, neither of which can see a `State` to read frames from.
#[must_use]
pub fn native_call_chain() -> Vec<String> {
    with_call_frames(|b| {
        b.iter()
            .rev()
            .map(|(nm, _, _, _)| (*nm).to_string())
            .collect()
    })
}

/// Push a frame onto the shadow call stack.  Called at the start of every
/// generated function body.  The `&'static str` arguments are string literals
/// embedded in the generated Rust code.
pub fn cr_call_push(name: &'static str, file: &'static str, line: u32) {
    // @PLN28 — native call-depth guard.  Generated Rust recurses one native
    // frame per loft call, so unbounded recursion overflows the OS stack with
    // an opaque `fatal runtime error: stack overflow, aborting`.  The
    // interpreter caps recursion at `State::MAX_CALL_DEPTH` and renders a typed
    // `StackOverflow`; mirror that here so both backends agree and native emits
    // a clean, source-located error instead of an abort.  (The generated `main`
    // runs on a large-stack thread — see `NATIVE_MAIN_STACK` — so this depth
    // limit is what actually fires, well before the OS stack is exhausted.)
    //
    // loft#1058 — the check comes BEFORE the push, as `State::fn_call`'s does: a
    // refused call never runs, so its frame is not part of the stack the diagnostic
    // reports.  Pushing first and tripping after put the refused callee on top of the
    // chain and made it one frame longer than the interpreter's — the two backends
    // named different functions for one fault.  The position is read off the frame
    // BELOW for the same reason: it is the function that is running.
    let depth = CALL_DEPTH.get();
    if depth >= crate::limits::MAX_CALL_DEPTH as usize {
        let (running_file, running_line) =
            with_call_frames(|b| b.last().map_or((file, line), |(_, f, ln, _)| (*f, *ln)));
        cr_stack_overflow(running_file, running_line);
    }
    CALL_FRAMES.with(|frames| {
        // SAFETY: the exclusive borrow lasts only this statement, on this
        // thread, and nothing else can run inside it.
        let v = unsafe { &mut *frames.get() };
        if depth < v.len() {
            v[depth] = (name, file, line, 0);
        } else {
            v.push((name, file, line, 0));
        }
    });
    CALL_DEPTH.set(depth + 1);
    // @PLAN49 T1 — refresh the shared breadcrumb at every native fn
    // entry so a watchdog-fired hard-kill identifies the most recent
    // loft fn we entered.  Single combined call.  When the timeout
    // is not armed, the body is just one relaxed atomic load + branch
    // (no allocation, no mutex) — ~1-2 ns per fn entry.
    crate::timeout::checkpoint_fn("run-native", name, file, line);
}

/// Record the line the running frame is calling FROM (loft#1753).  Emitted before each
/// statement that calls into a frame, in a function that pushed a named frame, so that
/// [`n_stack_trace`] reports each frame's `line` as the call site that entered it — the
/// interpreter's `CallFrame.line`.  One store into this thread's frame array; the lean tier
/// never emits it.
#[inline]
pub fn cr_call_site(line: u32) {
    let depth = CALL_DEPTH.get();
    if depth == 0 {
        return;
    }
    CALL_FRAMES.with(|frames| {
        // SAFETY: as in `cr_call_push` — an exclusive borrow for this statement only.
        let v = unsafe { &mut *frames.get() };
        if let Some(frame) = v.get_mut(depth - 1) {
            frame.3 = line;
        }
    });
}

/// Report the call-stack overflow and stop the program, through the renderer every
/// other halting fault uses.  `#[cold]` / `inline(never)` keeps it off the hot
/// call-entry path.
///
/// `file` / `line` are the string literals `cr_call_push` was emitted with — the
/// declaration of the function whose entry exceeded the cap, which is the position the
/// interpreter reports for the same fault (loft#1058).  Before this the two backends
/// spelled one event two ways: no `-->` block or caret here, its own frame format, and
/// the depth count only on this side.  The count now lives in
/// `RuntimeErrorKind::describe()`, so BOTH backends carry it, and the frames come from
/// the same `render_call_chain` the interpreter's go through.
///
/// The lazy-driver bypass @PLN133 S8 asks for is `report_and_exit`'s, shared with
/// `panic` and `assert`: a fault inside a driver unwinds into `store_lazy_error` rather
/// than halting the program.
#[cold]
#[inline(never)]
fn cr_stack_overflow(file: &str, line: u32) -> ! {
    crate::runtime_error::RuntimeError::stack_overflow(file, line).report_and_exit()
}

/// The LEAN tier's frame entry (@PLN157, loft#1426 M1): the recursion cap
/// without the frame record.  A `--lean` binary trades the shadow stack's
/// diagnostics — `stack_trace()` answers no frames, a native `assert`/`panic`
/// carries no loft frame block, the watchdog breadcrumb is not refreshed —
/// for a per-call cost of one bounds test and two `Cell` updates (measured
/// ~1.2 ns against ~6.7 ns for the named prelude).  What it KEEPS is the
/// depth cap itself, at the same `MAX_CALL_DEPTH` both backends share; the
/// overflow report names this call's own entry site rather than the frame
/// below (there is no frame below to read — the one spelling difference from
/// the named tier).  [`CallGuard`] balances it unchanged: the pop is a bare
/// decrement either way.
#[inline]
pub fn cr_call_push_lean(file: &'static str, line: u32) {
    let depth = CALL_DEPTH.get();
    if depth >= crate::limits::MAX_CALL_DEPTH as usize {
        cr_stack_overflow(file, line);
    }
    CALL_DEPTH.set(depth + 1);
}

/// Pop a frame from the shadow call stack.  Called at the end of every
/// generated function body (including early returns via a drop guard).
/// One `Cell` decrement — the entry itself stays behind as garbage the next
/// push at this depth overwrites.
#[inline]
pub fn cr_call_pop() {
    let depth = CALL_DEPTH.get();
    if depth > 0 {
        CALL_DEPTH.set(depth - 1);
    }
}

/// RAII guard that pops the shadow call stack on drop, ensuring the stack
/// stays balanced even when the function returns early.
pub struct CallGuard;

impl Drop for CallGuard {
    #[inline]
    fn drop(&mut self) {
        cr_call_pop();
    }
}

thread_local! {
    /// loft#715 — `lib_idx + 1` while a shared-library bridge call is on this thread's
    /// stack, 0 otherwise.  Read by [`current_shared_bridge`] on the fault path.
    static IN_SHARED_BRIDGE: Cell<u16> = const { Cell::new(0) };
}

/// loft#715 — library slot → the bridge symbol it dispatches to.  Populated once
/// at wiring; read only when a fault is raised while a bridge call is on the
/// stack, so a bridge fault stops reading like a program fault.
#[cfg(feature = "native-extensions")]
static SHARED_LABELS: std::sync::Mutex<Option<std::collections::HashMap<u16, String>>> =
    std::sync::Mutex::new(None);

/// Record which bridge symbol each library slot dispatches to (`extensions`, at wiring).
#[cfg(feature = "native-extensions")]
pub fn label_shared_bridges(labels: impl Iterator<Item = (u16, String)>) {
    let mut guard = SHARED_LABELS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let table = guard.get_or_insert_with(std::collections::HashMap::new);
    for (lib_idx, sym) in labels {
        table.insert(lib_idx, sym);
    }
}

/// A shared-library bridge call on slot `lib_idx` starts on this thread.  One `Cell` write.
#[inline]
pub fn enter_shared_bridge(lib_idx: u16) {
    IN_SHARED_BRIDGE.with(|c| c.set(lib_idx.wrapping_add(1)));
}

/// The bridge call on this thread has returned.
#[inline]
pub fn leave_shared_bridge() {
    IN_SHARED_BRIDGE.with(|c| c.set(0));
}

/// loft#715 — the bridge symbol currently being dispatched on this thread, if any.
///
/// A fault raised while a shared-library bridge is on the stack is a BRIDGE
/// fault, but it surfaced with the same words as a program fault: `Cannot add to
/// none-structure '<type>'` names the type the index resolved to and nothing
/// about who supplied the index — and because it is a non-unwinding panic the
/// process aborts, so the surrounding harness reports only that the program
/// never started.  Appending this turns "a type is wrong somewhere" into "this
/// library's bridge passed something this loft cannot use".
#[must_use]
pub fn current_shared_bridge() -> Option<String> {
    #[cfg(feature = "native-extensions")]
    {
        let marked = IN_SHARED_BRIDGE.with(Cell::get);
        if marked == 0 {
            return None;
        }
        let lib_idx = marked - 1;
        let guard = SHARED_LABELS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Some(guard.as_ref().and_then(|t| t.get(&lib_idx)).map_or_else(
            || format!("shared-library bridge (slot {lib_idx})"),
            |sym| format!("shared-library bridge `{sym}`"),
        ))
    }
    #[cfg(not(feature = "native-extensions"))]
    None
}
