// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: what it knows about the runtime it emits calls into

//! What the native generator needs to KNOW about the runtime emitted programs link: which
//! helpers `codegen_runtime` defines and the ABI each takes, and the op codes of the
//! fold-compare predicate.  A description of the runtime rather than the runtime, so the
//! generator reads it without depending on the layer above it (@PLN191).
/// ABI of a runtime helper function defined in `codegen_runtime`.
///
/// `Cell`: takes `cell: &UnsafeCell<Stores>` and derives a `&mut Stores`
/// at entry (the post-P199.A ABI used by all runtime fns that need
/// store access).
///
/// `None`: takes no implicit Stores parameter at all.  For pure helpers
/// whose body never touches `Stores` (e.g. clock reads, constant
/// returns, thread-local RNG access).  The codegen emits `fn_name(args)`
/// with no leading `cell` argument.  Strictly better than `Cell` for
/// these fns: zero unused parameter, zero borrow churn.
///
/// Every runtime fn uses either `Cell` or `None`; there is no variant
/// for a bare `&mut Stores` helper.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Abi {
    Cell,
    None,
}

/// Single source of truth for runtime-helper ABI tags.  Replaces the
/// twin hardcoded `LEGACY_STORES_FNS` constants that lived in
/// `src/generation/calls.rs` and `src/generation/dispatch.rs`, plus the
/// `CODEGEN_RUNTIME_FNS: &[&str]` list in `src/generation/mod.rs` that
/// drove "skip stub generation" for runtime fns.
///
/// Each entry must have a corresponding `pub fn <name>(...)` defined
/// in `codegen_runtime` with the matching ABI signature.  Adding a
/// new runtime fn to this registry is sufficient — `output_call_user_fn`
/// and the arg-hoist path consult `abi_of(name)` and pick `cell` vs
/// `stores` automatically.
pub struct RuntimeFn {
    pub name: &'static str,
    pub abi: Abi,
}

#[rustfmt::skip]
pub const CODEGEN_RUNTIME_FNS: &[RuntimeFn] = &[
    RuntimeFn { name: "n_now",                      abi: Abi::None },
    RuntimeFn { name: "n_ticks",                    abi: Abi::Cell },
    RuntimeFn { name: "n_get_store_lock",           abi: Abi::Cell },
    RuntimeFn { name: "n_set_store_lock",           abi: Abi::Cell },
    RuntimeFn { name: "n_protect_store_frees",      abi: Abi::Cell },
    RuntimeFn { name: "n_unprotect_store_frees",    abi: Abi::Cell },
    // n_rand / n_rand_seed / n_rand_indices live in lib/random/native/
    // (the `loft::native_call` helpers forward LoftStore for
    // store-allocating cdylib returns).  Native codegen routes these through
    // `native_symbol_crates` from random's [native.functions] manifest;
    // the interpreter routes via dlopen.  Single RNG state in the
    // cdylib; no compiler-crate fallback.
    RuntimeFn { name: "n_parallel_for_native",        abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_for_text_native",   abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_for_ref_native",    abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_block_native",      abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_discard_native",    abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_queue_native",      abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_queue_text_native", abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_queue_ref_native",  abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_queue_fn_native",   abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_fold_native",       abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_buf_get_native",    abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_buf_get_text_native", abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_buf_get_ref_native",  abi: Abi::Cell },
    // ARC.md A3.6 — typed Single/Float readers.
    RuntimeFn { name: "n_parallel_buf_get_single_native", abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_buf_get_float_native",  abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_buf_drop_native",     abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_buf_drop_text_native", abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_buf_drop_ref_native",  abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_buf_get_fn_native",  abi: Abi::Cell },
    RuntimeFn { name: "n_parallel_buf_drop_fn_native", abi: Abi::Cell },
    RuntimeFn { name: "n_path_sep",                   abi: Abi::None },
    RuntimeFn { name: "n_stack_trace",                abi: Abi::Cell },
    RuntimeFn { name: "n_reflect_type",               abi: Abi::Cell },
    RuntimeFn { name: "n_type_named",                 abi: Abi::Cell },
    RuntimeFn { name: "n_reflect_field",              abi: Abi::Cell },
    RuntimeFn { name: "n_reflect_field_path",         abi: Abi::Cell },
    RuntimeFn { name: "n_hash_sorted",                abi: Abi::Cell },
    RuntimeFn { name: "n_vector_sum_int",             abi: Abi::Cell },
    RuntimeFn { name: "n_hash_unsorted",              abi: Abi::Cell },
    RuntimeFn { name: "n_radix_sorted",               abi: Abi::Cell },
    RuntimeFn { name: "n_spatial_range",              abi: Abi::Cell },
    RuntimeFn { name: "n_trie_prefix",                abi: Abi::Cell },
    // P268 — JSON ecosystem (P54 sprint).  Native runtime wrappers
    // around `crate::native::json_parse_into_stores` + the JsonValue
    // method natives so `--native` programs can read/parse JSON
    // instead of hitting `todo!()` stubs.  Methods are registered
    // under their `t_<LEN><Type>_<method>` codegen names.
    RuntimeFn { name: "n_json_parse",                 abi: Abi::Cell },
    RuntimeFn { name: "t_9JsonValue_field",           abi: Abi::Cell },
    RuntimeFn { name: "t_9JsonValue_item",            abi: Abi::Cell },
    RuntimeFn { name: "t_9JsonValue_len",             abi: Abi::Cell },
    RuntimeFn { name: "t_9JsonValue_as_text",         abi: Abi::Cell },
    RuntimeFn { name: "t_9JsonValue_as_long",         abi: Abi::Cell },
    RuntimeFn { name: "t_9JsonValue_kind",            abi: Abi::Cell },
    // The remaining JSON natives, so any loft program can call json_*
    // under --native, not just the viewer's tag-page subset.
    RuntimeFn { name: "t_9JsonValue_as_number",       abi: Abi::Cell },
    RuntimeFn { name: "t_9JsonValue_as_bool",         abi: Abi::Cell },
    RuntimeFn { name: "t_9JsonValue_keys",            abi: Abi::Cell },
    RuntimeFn { name: "t_9JsonValue_fields",          abi: Abi::Cell },
    RuntimeFn { name: "t_9JsonValue_has_field",       abi: Abi::Cell },
    RuntimeFn { name: "t_9JsonValue_to_json",         abi: Abi::Cell },
    RuntimeFn { name: "t_9JsonValue_to_json_pretty",  abi: Abi::Cell },
    RuntimeFn { name: "n_json_null",                  abi: Abi::Cell },
    RuntimeFn { name: "n_json_bool",                  abi: Abi::Cell },
    RuntimeFn { name: "n_json_number",                abi: Abi::Cell },
    RuntimeFn { name: "n_json_string",                abi: Abi::Cell },
    RuntimeFn { name: "n_json_array",                 abi: Abi::Cell },
    RuntimeFn { name: "n_json_object",                abi: Abi::Cell },
    RuntimeFn { name: "n_struct_from_jsonvalue",      abi: Abi::Cell },
    RuntimeFn { name: "n_struct_to_json",             abi: Abi::Cell },
    RuntimeFn { name: "n_struct_to_json_pretty",      abi: Abi::Cell },
    // @PLN18 08-S1 — the engine-host kernel natives, typed twins in
    // `engine_host::typed` (re-exported below): a compiled kernel program
    // calls the SAME in-binary kernel the interpreter uses — one
    // implementation, two calling conventions.
    RuntimeFn { name: "n_git_query",                  abi: Abi::Cell },
    RuntimeFn { name: "n_process_run",                abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_listen",              abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_pump",                abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_next_event",          abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_event_cid",           abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_event_kind",          abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_event_payload",       abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_event_status",        abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_client_event_status", abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_http_fetch",          abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_tick_due",            abi: Abi::Cell },
    RuntimeFn { name: "n_send",                       abi: Abi::Cell },
    RuntimeFn { name: "n_broadcast",                  abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_idle",                abi: Abi::Cell },
    RuntimeFn { name: "n_clients",                    abi: Abi::Cell },
    // The same four under the names the loft surface uses now
    // that its public functions are WRAPPERS (a function that already carries a
    // native symbol cannot be placed).  The old names stay: this table is keyed
    // by loft def name, and a library elsewhere may still spell them the old way.
    RuntimeFn { name: "n_kernel_send",                abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_broadcast",           abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_clients",             abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_post",                abi: Abi::Cell },
    RuntimeFn { name: "n_udp_bound",                  abi: Abi::Cell },
    RuntimeFn { name: "n_sync_class",                 abi: Abi::Cell },
    RuntimeFn { name: "n_sync_class_keyed",           abi: Abi::Cell },
    RuntimeFn { name: "n_keyframe",                   abi: Abi::Cell },
    RuntimeFn { name: "n_sync_next",                  abi: Abi::Cell },
    RuntimeFn { name: "n_sync_cid",                   abi: Abi::Cell },
    RuntimeFn { name: "n_sync_seq",                   abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_sync_payload",        abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_client_frame",        abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_default_host",        abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_connect",             abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_local",               abi: Abi::Cell },
    RuntimeFn { name: "n_post",                       abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_alive",               abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_stop",                abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_frame",               abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_client_pump",         abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_client_alive",        abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_client_stop",         abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_client_next_event",   abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_client_event_kind",   abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_client_event_cid",    abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_client_event_payload", abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_client_tick_due",     abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_client_idle",         abi: Abi::Cell },
    RuntimeFn { name: "n_client_send",                abi: Abi::Cell },
    RuntimeFn { name: "n_client_sync_next",           abi: Abi::Cell },
    RuntimeFn { name: "n_client_sync_seq",            abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_client_sync_payload", abi: Abi::Cell },
    RuntimeFn { name: "n_client_udp_bound",           abi: Abi::Cell },
    // @PLN18 08-S2 — live dispatch: flip a fn to the interpreter at runtime.
    RuntimeFn { name: "n_live_flip",                  abi: Abi::Cell },
    // @PLN18 08-S4 — background rebuild of the served program.
    RuntimeFn { name: "n_rebuild_start",              abi: Abi::Cell },
    RuntimeFn { name: "n_rebuild_status",             abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_rebuild_artifact",    abi: Abi::Cell },
    // @PLN18 08-S5 — the native build swap.
    RuntimeFn { name: "n_swap_world",                 abi: Abi::Cell },
    RuntimeFn { name: "n_swap_start",                 abi: Abi::Cell },
    RuntimeFn { name: "n_kernel_swap_step",           abi: Abi::Cell },
    RuntimeFn { name: "n_swap_retired",               abi: Abi::Cell },
];

/// Look up the ABI of a runtime helper.  Returns `Abi::Cell` for
/// unknown names — user-defined functions and Op stubs default to the
/// post-P199.A cell ABI.
#[must_use]
pub fn abi_of(name: &str) -> Abi {
    CODEGEN_RUNTIME_FNS
        .iter()
        .find(|f| f.name == name)
        .map_or(Abi::Cell, |f| f.abi)
}

/// True if `name` is a runtime helper defined in this module.  Used by
/// `output_function` to skip stub generation (the real implementation
/// is imported via `use loft::codegen_runtime::*`).
#[must_use]
pub fn is_codegen_runtime_fn(name: &str) -> bool {
    CODEGEN_RUNTIME_FNS.iter().any(|f| f.name == name)
}

/// `@FR-R-FoldCompare` — which text predicate `codegen_runtime::fold_compare` answers.
pub mod fold_op {
    /// `F(t) == other`.
    pub const EQ: u8 = 0;
    /// `F(t) != other`.
    pub const NE: u8 = 1;
    /// `F(t).starts_with(other)`, or with `fold_right` `other.starts_with(F(t))`.
    pub const STARTS: u8 = 2;
    /// `F(t).ends_with(other)`, or with `fold_right` `other.ends_with(F(t))`.
    pub const ENDS: u8 = 3;
}
