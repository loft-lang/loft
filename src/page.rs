// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@PLN226` — what a browser page keeps between its calls into the wasm module: its debug
//! session (`wasm_debug`), its game session (`wasm_entry`) and the world snapshot it staged
//! for the next run (`wasm::swap_stage`).
//!
//! A page calls the module's exports with no context to hand back (`compile_and_start`, then
//! `resume_frame` each frame), and what it keeps is bound to its thread (an interpreter
//! `State` is not `Send`), so this is the one state loft keeps per thread outside the rustc
//! writer's runtime: the page's thread is the page.  `@FR-C-Boundary`'s form is a handle the
//! page receives from its first call and passes to every later one — a change to the page's
//! API (the gallery pages, the `--html` glue), left to the owner (@PLN226 § Open).  A native
//! test that drives the entries plays one page per test thread.

use std::cell::RefCell;

/// A page's state between its calls (see the module documentation).  Each part is its own
/// cell: a game frame that runs `swap_world` reads the staged snapshot while the frame holds
/// the game session.
#[derive(Default)]
pub(crate) struct Page {
    pub(crate) debug: RefCell<Option<crate::wasm_debug::Session>>,
    #[cfg(host_fs)]
    pub(crate) game: RefCell<Option<crate::wasm_entry::GameSession>>,
    #[cfg(host_fs)]
    pub(crate) swap_stage: RefCell<Option<String>>,
    /// The `par` worker tracer's lines (`LOFT_TRACE_PAR_WORKERS`), which the page reads with
    /// the program's output (`par_pool::WorkerTrace::report`).
    #[cfg(feature = "wasm")]
    pub(crate) trace: RefCell<String>,
}

/// The `par` tracer's lines since the last call, for the output the page reads.
#[must_use]
pub(crate) fn take_trace() -> String {
    #[cfg(feature = "wasm")]
    {
        with(|p| std::mem::take(&mut *p.trace.borrow_mut()))
    }
    #[cfg(not(feature = "wasm"))]
    {
        String::new()
    }
}

thread_local! {
    static PAGE: Page = Page::default();
}

/// `f` over the page's state.
pub(crate) fn with<R>(f: impl FnOnce(&Page) -> R) -> R {
    PAGE.with(f)
}
