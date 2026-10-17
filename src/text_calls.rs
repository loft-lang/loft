// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I65 — Bytecode code generator: which natives return text through a destination

//! Which native calls hand back their `text` through a destination the caller supplies
//! rather than a buffer they allocate: a fact about a definition's native ABI that the parser,
//! the bytecode emitter and the native generator all ask, so it sits in the core layer
//! (@PLN191).
use crate::data::Type;

/// Text-returning natives that accept a destination buffer instead of allocating one.
pub(crate) fn is_text_dest_native(name: &str) -> bool {
    matches!(
        name,
        "t_4text_replace"
            | "t_4text_to_lowercase"
            | "t_4text_to_uppercase"
            // @PLN10 — text producers with `_dest` variants (registered in
            // `native.rs FUNCTIONS`).  `as_text` is INCLUDED (Phase 2): text-null
            // is content-based ("\0"), so a dest record carries it — the premise
            // that "a dest can't represent null" was probe-falsified on both
            // backends.
            | "n_source_dir"
            | "n_directory"
            | "n_user_directory"
            | "n_program_directory"
            // #635 — os_temp_dir / os_cache_dir (the private natives temp_dir /
            // cache_dir wrap); non-null text, "" only on a filesystem-less target.
            | "n_os_temp_dir"
            | "n_os_cache_dir"
            | "n_kernel_event_payload"
            | "n_kernel_sync_payload"
            | "n_kernel_client_event_payload"
            | "n_kernel_client_sync_payload"
            | "n_kernel_default_host"
            | "n_kernel_rebuild_artifact"
            | "n_json_errors"
            | "t_9JsonValue_kind"
            | "t_9JsonValue_to_json"
            | "t_9JsonValue_to_json_pretty"
            // @PLN10 Phase 1 — always-non-null `#rust`-template producers.
            | "n_ymd_days_ago"
            | "n_store_memory"
            // Always-non-null: "" IS the healthy answer.
            | "n_store_lazy_error"
            // @PLN10 Phase 1 batch 2 — always-non-null codegen_runtime producers.
            | "n_struct_to_json"
            | "n_struct_to_json_pretty"
            // @PLN10 Phase 1 — par-text buffer reader (interp; native is owned
            // String already, native par-text loops are blocked by #273).
            | "n_parallel_buf_get_text"
            // @PLN10 Phase 2 — as_text (null carried as the "\0" sentinel content).
            | "t_9JsonValue_as_text"
            // @PLN10 Phase 2 — env_variable (non-null; os_variable owns its String).
            | "n_env_variable"
            // host_input — non-null program-input reader (stdin / --html host).
            | "n_host_input"
            // text_from_bytes — owned text decoded from a vector<u8>.
            | "n_text_from_bytes"
            | "n_text_from_byte_range"
    )
}

/// @PLN10 N2b — a **cdylib FFI** text call: an EXTERNAL `#native "symbol"`
/// function returning text.  Built-in text natives are `#pure` / `#rust` (empty
/// `def.native`); only loaded cdylib functions carry a non-empty native symbol,
/// so this is disjoint from `is_text_dest_native` (no `default/` decl uses
/// `#native`).  Such calls dispatch through the runtime bridge, which is
/// dest-passed via `n_set_bridge_dest` + `gen_cdylib_text_dest_call` instead of
/// `stores.scratch`.
pub(crate) fn is_cdylib_text_call(def: &crate::data::Definition) -> bool {
    !def.native().is_empty() && matches!(def.returned(), Type::Text(_))
}

/// A **`#c` binding** returning text: a C `char *` brought back
/// as a loft `text`.
///
/// Routed exactly like [`is_cdylib_text_call`] (`n_set_bridge_dest` +
/// `gen_cdylib_text_dest_call`), and for the same reason: a body-less external
/// declaration has no `_dest` sibling to call and no promoted work buffer of its
/// own, so the destination has to be handed over as a separate step. Disjoint
/// from both neighbours — a `#c` definition carries no `native` symbol (it is
/// left empty so the Rust dispatch path cannot claim it) and no `default/`
/// declaration carries a `c_sig`.
pub(crate) fn is_c_text_call(def: &crate::data::Definition) -> bool {
    // `.base()` peels `Optional`: a `text?` return uses the same owned-text ABI,
    // and for a `char *` it is the spelling that lets the null-flow analysis see
    // the NULL the crossing carries either way.
    !def.c_sig.is_empty() && matches!(def.returned().base(), Type::Text(_))
}

/// Op-name rewrite for `RefVar(Text)` destinations.
///
/// `text_return` (`src/parser/control.rs:2919`) promotes the work-text
/// buffer of any text-returning function from a local `Type::Text(_)`
/// to a hidden `Type::RefVar(Type::Text(_))` argument (`&mut String`).
/// Self-reference assignments inside such a function emit
/// `OpAppendText` / `OpClearText` / `OpFormat{Text,Int,Float,Single,
/// Database}` / `OpAppendCharacter` IR on that buffer, but those ops
/// expect a local `String` destination.  The `Stack` variants exist
/// for the refvar case — this table maps a base op name to its Stack
/// variant.  Both `src/state/codegen.rs::generate_call` (interp
/// bytecode dispatch) and `src/generation/dispatch.rs::output_call_inner`
/// (native emit dispatch) consult it.  Kept here, out-of-band of
/// `output_call_inner`'s `Op` match-arm budget gate.
#[must_use]
pub fn refvar_text_stack_variant(name: &str) -> Option<&'static str> {
    match name {
        "OpAppendText" => Some("OpAppendStackText"),
        "OpClearText" => Some("OpClearStackText"),
        "OpAppendCharacter" => Some("OpAppendStackCharacter"),
        "OpFormatText" => Some("OpFormatStackText"),
        "OpFormatInt" => Some("OpFormatStackInt"),
        "OpFormatFloat" => Some("OpFormatStackFloat"),
        "OpFormatSingle" => Some("OpFormatStackSingle"),
        "OpFormatDatabase" => Some("OpFormatStackDatabase"),
        _ => None,
    }
}
