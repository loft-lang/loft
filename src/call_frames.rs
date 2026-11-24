// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: the frames a native program is running

//! What a generated program is running: the loft frames on its shadow call stack, and whether
//! it is inside a lazy store's driver.
//!
//! Generated code pushes and pops the frames (`cr_call_push` / `cr_call_pop`) on the `Stores`
//! it runs over; a fault report reads them there, which is why this sits in the core layer with
//! the stores and the error renderer (@PLN191).

use std::cell::UnsafeCell;

use crate::database::Stores;

/// `(name, file, declaration line, call site)` per frame: the call site is the line the frame
/// is calling from, written by [`cr_call_site`] before each call.
pub type NativeFrame = (&'static str, &'static str, u32, u32);

/// The shadow call stack of a generated program, and its lazy-driver window — the `--native`
/// twin of the interpreter's `State` call frames.  Owned by the program's `Stores`
/// (`Stores::frames`, `@FR-C-Own`): a `par` worker's stores start with none, and a fault
/// report reads the stores that faulted, never the thread (`@FR-C-Boundary`).
///
/// The live prefix is `frames[..depth]`: a pop only decrements the depth, leaving its entry
/// as garbage the next push at that depth overwrites, so the per-call cost is a bounds test,
/// three word stores and a depth update (loft#1426 M1, @PLN157 P1).  Every reader slices
/// `[..depth]`, clamped to the array — the LEAN tier advances the depth through
/// [`cr_call_push_lean`] without writing frames.
#[derive(Default)]
pub struct NativeFrames {
    depth: usize,
    frames: Vec<NativeFrame>,
    /// @PLN133 S8 — is this program inside a lazy DRIVER right now?
    ///
    /// It changes what a fault MEANS. An ordinary loft call that overflows the stack is a
    /// program bug and halting is right; the same overflow inside a FETCH is not, because
    /// @PLN129's contract is @C80 — a failed fetch reports through `store_lazy_error` and the
    /// lookup answers null.  A `par` worker running its own fetch has its own stores, so it
    /// is its own answer.
    in_lazy_driver: bool,
}

impl Clone for NativeFrames {
    /// A copied `Stores` is a new context: it runs no frame yet.
    fn clone(&self) -> Self {
        NativeFrames::default()
    }
}

impl NativeFrames {
    /// The live frames, outermost first.
    #[must_use]
    pub fn live(&self) -> &[NativeFrame] {
        &self.frames[..self.depth.min(self.frames.len())]
    }

    /// Is a fault here a driver's, and therefore containable?
    ///
    /// Read by `cr_stack_overflow` and `RuntimeError::report_and_exit_on`, which exit the
    /// process for an ordinary fault and unwind with a [`ContainedFault`] for this one.
    #[must_use]
    pub fn in_lazy_driver(&self) -> bool {
        self.in_lazy_driver
    }

    /// Mark whether this program runs a lazy driver — the window [`Self::in_lazy_driver`]
    /// answers for.
    pub fn set_in_lazy_driver(&mut self, inside: bool) {
        self.in_lazy_driver = inside;
    }

    /// Where the running frame was declared, for a RECOVERABLE fault's log line — the same
    /// position the halting path names on `--native` (`State::running_frame_declaration` is
    /// the interpreter's twin).  `None` on the lean tier, which pushes no frames, and
    /// outside any loft frame.
    #[must_use]
    pub fn running_position(&self) -> Option<crate::lexer::Position> {
        self.live()
            .last()
            .map(|(_, file, line, _)| crate::lexer::Position {
                file: crate::lexer::intern_file(file),
                line: *line,
                pos: 1,
            })
    }

    /// The loft frames this program is inside, innermost first.
    ///
    /// The `--native` counterpart of the interpreter's `State::current_call_chain`, in the
    /// same shape (bare function names) so that one renderer serves both backends —
    /// `RuntimeError::report_and_exit_on` attaches these to a fault raised by `assert` or
    /// `panic`.
    #[must_use]
    pub fn call_chain(&self) -> Vec<String> {
        self.live()
            .iter()
            .rev()
            .map(|(nm, _, _, _)| (*nm).to_string())
            .collect()
    }
}

/// The unwind payload of a fault CONTAINED inside a lazy driver: the driver's call answers
/// it to `store_lazy_error`, and the CLI's panic hook stays silent for it — the payload says
/// what the fault is, so nothing asks which thread raised it (`@FR-C-Boundary`).  Spelled the
/// way the interpreter's contained fetch spells it: `<kind label>: <message>`.
pub struct ContainedFault(pub String);

/// The frames of the program running over `cell`.
#[inline]
#[expect(
    clippy::mut_from_ref,
    reason = "the generated program's `UnsafeCell<Stores>` is its one context"
)]
fn frames_of(cell: &UnsafeCell<Stores>) -> &mut NativeFrames {
    // SAFETY: generated code holds `cell` for the whole run and calls this between its own
    // statements; the borrow ends with the statement that took it.
    unsafe { &mut (*cell.get()).frames }
}

/// Push a frame onto the shadow call stack.  Called at the start of every
/// generated function body.  The `&'static str` arguments are string literals
/// embedded in the generated Rust code.
///
/// Deliberately NOT `#[inline]`.  Only the semantics tier (`--native`, the default backend
/// and the test runner) emits this push — a `--ship` program pushes through
/// [`cr_call_push_lean`] — and that tier compiles the generated program at a low optimisation
/// level.  An `#[inline]` function is compiled again INSIDE the program at that level instead
/// of being called in its optimised form from the release `libloft`: measured on a
/// call-heavy program (48 M named calls), 141 s without the attribute and 255 s with it.
pub fn cr_call_push(cell: &UnsafeCell<Stores>, name: &'static str, file: &'static str, line: u32) {
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
    let f = frames_of(cell);
    let depth = f.depth;
    if depth >= crate::limits::MAX_CALL_DEPTH as usize {
        let (running_file, running_line) = f
            .live()
            .last()
            .map_or((file, line), |(_, fl, ln, _)| (*fl, *ln));
        cr_stack_overflow(cell, running_file, running_line);
    }
    if depth < f.frames.len() {
        f.frames[depth] = (name, file, line, 0);
    } else {
        f.frames.push((name, file, line, 0));
    }
    f.depth = depth + 1;
    // Refresh the shared breadcrumb at every native fn
    // entry so a watchdog-fired hard-kill identifies the most recent
    // loft fn we entered.  Single combined call.  When the timeout
    // is not armed, the body is just one relaxed atomic load + branch
    // (no allocation, no mutex) — ~1-2 ns per fn entry.
    crate::timeout::checkpoint_fn("run-native", name, file, line);
}

/// Record the line the running frame is calling FROM (loft#1753).  Emitted before each
/// statement that calls into a frame, in a function that pushed a named frame, so that
/// `n_stack_trace` reports each frame's `line` as the call site that entered it — the
/// interpreter's `CallFrame.line`.  One store into the program's frame array; the lean tier
/// never emits it.
///
/// Deliberately NOT `#[inline]`, for the reason [`cr_call_push`] gives: only the semantics tier
/// emits it, and inlined it is compiled again inside the program at that tier's low
/// optimisation level.  Measured on fib(40) (331 M calls): 15.2 s inlined, 12.8 s called
/// (loft#2009).  The lean tier never emits it, so its out-of-line form costs that tier nothing.
pub fn cr_call_site(cell: &UnsafeCell<Stores>, line: u32) {
    let f = frames_of(cell);
    if f.depth == 0 {
        return;
    }
    let at = f.depth - 1;
    if let Some(frame) = f.frames.get_mut(at) {
        frame.3 = line;
    }
}

/// Report the call-stack overflow and stop the program, through the renderer every
/// other halting fault uses.  `#[cold]` / `inline(never)` keeps it off the hot
/// call-entry path.
///
/// `file` / `line` are the string literals `cr_call_push` was emitted with — the
/// declaration of the function whose entry exceeded the cap, which is the position the
/// interpreter reports for the same fault (loft#1058).  The depth count lives in
/// `RuntimeErrorKind::describe()`, so BOTH backends carry it, and the frames come from
/// the same `render_call_chain` the interpreter's go through.
///
/// The lazy-driver bypass @PLN133 S8 asks for is `report_and_exit_on`'s, shared with
/// `panic` and `assert`: a fault inside a driver unwinds into `store_lazy_error` rather
/// than halting the program.
#[cold]
#[inline(never)]
fn cr_stack_overflow(cell: &UnsafeCell<Stores>, file: &str, line: u32) -> ! {
    crate::runtime_error::RuntimeError::stack_overflow(file, line)
        .report_and_exit_on(frames_of(cell))
}

/// The LEAN tier's frame entry (@PLN157, loft#1426 M1): the recursion cap
/// without the frame record.  A `--lean` binary trades the shadow stack's
/// diagnostics — `stack_trace()` answers no frames, a native `assert`/`panic`
/// carries no loft frame block, the watchdog breadcrumb is not refreshed —
/// for a per-call cost of one bounds test and a depth update (measured
/// ~1.2 ns against ~6.7 ns for the named prelude).  What it KEEPS is the
/// depth cap itself, at the same `MAX_CALL_DEPTH` both backends share; the
/// overflow report names this call's own entry site rather than the frame
/// below (there is no frame below to read — the one spelling difference from
/// the named tier).  [`CallGuard`] balances it unchanged: the pop is a bare
/// decrement either way.
#[inline]
pub fn cr_call_push_lean(cell: &UnsafeCell<Stores>, file: &'static str, line: u32) {
    let f = frames_of(cell);
    if f.depth >= crate::limits::MAX_CALL_DEPTH as usize {
        cr_stack_overflow(cell, file, line);
    }
    f.depth += 1;
}

/// Pop a frame from the shadow call stack.  Called at the end of every
/// generated function body (including early returns via a drop guard).
/// One decrement — the entry itself stays behind as garbage the next
/// push at this depth overwrites.
#[inline]
pub fn cr_call_pop(cell: &UnsafeCell<Stores>) {
    let f = frames_of(cell);
    if f.depth > 0 {
        f.depth -= 1;
    }
}

/// RAII guard that pops the shadow call stack on drop, ensuring the stack
/// stays balanced even when the function returns early.  The LEAN tier's guard, beside
/// [`cr_call_push_lean`]: that tier compiles the program fully optimised, and inlined the pop
/// is a bare decrement at every return — measured on fib(45) (1.8 G calls) 5.66 s inlined and
/// 10.57 s with the drop out of line (loft#2009).  The semantics tier uses [`NamedCallGuard`].
pub struct CallGuard<'a>(pub &'a UnsafeCell<Stores>);

impl Drop for CallGuard<'_> {
    #[inline]
    fn drop(&mut self) {
        cr_call_pop(self.0);
    }
}

/// The SEMANTICS tier's guard, beside [`cr_call_push`]: the same pop, called rather than inlined.
/// That tier compiles the program at a low optimisation level, where an inlined drop is
/// compiled again at every return of every function instead of called in its optimised form
/// from the release `libloft` — measured on fib(40) (331 M calls) 15.2 s with the drop inlined
/// and 9.8 s called, 7.5 s together with [`cr_call_site`] out of line (loft#2009).  The two
/// tiers want opposite answers for one body, so each tier names its own guard and the emitter
/// picks it.
pub struct NamedCallGuard<'a>(pub &'a UnsafeCell<Stores>);

impl Drop for NamedCallGuard<'_> {
    fn drop(&mut self) {
        cr_call_pop(self.0);
    }
}

/// loft#715 — the bridge symbol a `Stores` is dispatching to, given its `in_bridge` mark
/// (`lib_idx + 1`, or 0 outside any bridge call) and the symbols its program wired.
///
/// A fault raised while a shared-library bridge is on the stack is a BRIDGE
/// fault, but it surfaced with the same words as a program fault: `Cannot add to
/// none-structure '<type>'` names the type the index resolved to and nothing
/// about who supplied the index — and because it is a non-unwinding panic the
/// process aborts, so the surrounding harness reports only that the program
/// never started.  Appending this turns "a type is wrong somewhere" into "this
/// library's bridge passed something this loft cannot use".
#[must_use]
pub fn shared_bridge_label(marked: u16, labels: &[Option<String>]) -> Option<String> {
    #[cfg(feature = "native-extensions")]
    {
        if marked == 0 {
            return None;
        }
        let lib_idx = marked - 1;
        Some(
            labels
                .get(usize::from(lib_idx))
                .cloned()
                .flatten()
                .map_or_else(
                    || format!("shared-library bridge (slot {lib_idx})"),
                    |sym| format!("shared-library bridge `{sym}`"),
                ),
        )
    }
    #[cfg(not(feature = "native-extensions"))]
    {
        let _ = (marked, labels);
        None
    }
}

/// One eval-TOS / frame-reserve advance step.
///
/// When `aligned`, round `size` up to 8 (the max alignment on the
/// stack) so successive pushes stay 8-aligned and every typed write
/// lands on its required boundary; the LIFO pop reverses the same
/// rounded step (the design's § 3 "uniform-8 step" choice).  When not
/// `aligned`, returns `size` unchanged — V1's tight, real-size step.
///
/// This is the single seam the S4 work toggles: route every
/// `stack_pos += size` / `-= size` and `stack.position += size` site
/// through it so codegen and runtime advance in lockstep (S1).
#[must_use]
#[inline]
pub fn aligned_stack_step(size: u32) -> u32 {
    size.next_multiple_of(8)
}
