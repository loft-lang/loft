// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the interpreter's entry

//! The interpreter's ENTRY (@PLN200 B2.6, `@C153`): everything code outside the interpreter
//! may name, grouped by the part that provides it.  The driver, the REPL, the debugger
//! front-ends and the browser host reach the interpreter only through these paths
//! (`scripts/crate_layers.py`'s entry rule), so this file is the interpreter's surface.
//! What it takes and what it provides is written down in INTERMEDIATE.md § The interpreter's
//! contract.

/// Optimised IR to bytecode.
pub mod compile {
    pub use crate::compile::{byte_code, byte_code_with_store, close_program, show_code};
}

/// The standard library's compiled bodies, routed at start-up.
pub mod compiled_stdlib {
    pub use crate::compiled_stdlib::{mark, names};
}

/// Breakpoints, stepping and watches.
pub mod debugger {
    pub use crate::debugger::{BreakHit, INTERRUPT, STOPPED_BY_INTERRUPT, StepMode, WatchHit};
}

/// Native extensions: loading a library's compiled functions and wiring them to calls.
pub mod extensions {
    #[cfg(feature = "registry")]
    pub use crate::extensions::prebuild_installed_natives;
    pub use crate::extensions::{
        auto_build_native, auto_build_native_target, browser_target, c_target, current_lib_idx,
        load_all, native_target_root, platform_lib_name, preload_native_libs,
        reachable_unimplemented_natives, resolve_native_impl_symbols, resolve_native_lib,
        set_browser_target, set_c_target, wire_native_fns, wire_native_fns_for,
        wire_shared_native_fns,
    };
}

/// `loft --introspect`: IR, bytecode, slots, types and ownership.
pub mod introspect {
    pub use crate::introspect::{Options, Section, emit_all};
}

/// Live code replacement in a running program.
pub mod live_dispatch {
    pub use crate::live_dispatch::wasm_host_log;
}

/// Reloading a running program's code when its source changes.
#[cfg(not(target_arch = "wasm32"))]
pub mod live_reload {
    pub use crate::live_reload::install;
}

/// Adopting a library's compiled artifact.
pub mod native_adopt {
    pub use crate::native_adopt::{Adopt, adopt_shared_cdylib, probe_and_mark_exports};
}

/// Function and line timing.
pub mod profiler {
    pub use crate::profiler::Totals;
}

/// The bytecode machine: its state, execution and run-time values.
pub mod state {
    pub use crate::state::{
        HostRetKind, HostReturn, LocalState, STRING_NULL, State, WorkerArg, text_timeline_summary,
    };
    pub(crate) use crate::state::{loft_float_literal, loft_text_literal};
}

/// The browser's GL natives.
pub mod wasm_gl {
    pub use crate::wasm_gl::register_wgl_natives;
}
