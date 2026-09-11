// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Per-Op emitter dispatch (plan 09 scaffold).
//!
//! Native code generation today substitutes `#rust"…@v0…"` templates from
//! `default/*.loft` declarations.  Plan 09 introduces a dispatch layer:
//! every Op-emission call site routes through `emit_op(ctx, name, args)`.
//! When no custom emitter is registered for `name`, dispatch falls through
//! to `DefaultEmitter`, which delegates back to
//! `Output::substitute_template_body` (the byte-identical extraction of
//! the original `output_call_template` body).
//!
//! Custom emitters live in `src/generation/ops/<op>.rs` and override the
//! default for Ops that need context-aware emission (field widths, ref
//! flavour, generic bindings) the template can't supply.
//!
//! After phase 00 (this scaffold + step 0.7b's let-bind-on-repeat already
//! shipped + steps 0.4-0.7 hoisting call sites), the registry is empty —
//! no Op is intercepted.  The infrastructure is in place for future phases
//! (and external contributors) to opt in per Op.
//!
//! Plan reference: `doc/claude/plans/finished/09-native-runtime-rewrite/00-scaffold.md`.

// Phase 00 step 0.2 shipped this surface without consumers; step 0.4
// wires the first call site (`output_call_template`).  Future phases
// add more call sites and custom emitters.
#![allow(dead_code)]

pub mod coroutine;
pub mod default;
pub mod float_compare;
pub mod int_arith;
pub mod int_compare;
pub mod key_ops;
pub mod misc_ops;
pub mod parallel;
pub mod ref_ops;
pub mod text_ops;
pub mod vector_ops;

use super::Output;
use crate::data::{Definition, Type, Value};
use std::io::{self, Write};

/// Context passed to every emitter.  Carries the writer, the Op
/// definition, and a back-reference to the codegen state (`Output`)
/// so emitters can call helpers like `substitute_template_body` or
/// `generate_expr_buf` without losing access to per-function state.
///
/// Two lifetime parameters:
///   - `'a` — the borrow scope of EmitCtx itself (the lifetime of the
///     mutable references to the writer and `Output`).
///   - `'b` — `Output`'s data/stores lifetime (longer-lived).
pub struct EmitCtx<'a, 'b> {
    pub w: &'a mut dyn Write,
    pub def_fn: &'a Definition,
    pub output: &'a mut Output<'b>,
}

impl EmitCtx<'_, '_> {
    /// Convenience: emit a `Value` to `self.w`.  Equivalent to
    /// `self.output.output_code_inner(&mut *self.w, v)` but cuts the
    /// reborrow boilerplate at every emitter call site.  Phase 03
    /// learned that real emitters call this pattern many times per
    /// `emit()` body — the wrapper makes those call sites readable.
    pub fn emit(&mut self, v: &Value) -> io::Result<()> {
        self.output.output_code_inner(&mut *self.w, v)
    }

    /// Convenience: emit a `Value` as an i32-typed slot.  Used when
    /// the call-site context expects i32 (tp-numbers, field offsets,
    /// flag-enum slots) and the runtime helper signature is fixed at
    /// i32.  Equivalent to
    /// `self.output.emit_i32_slot(&mut *self.w, v)`.
    pub fn emit_i32_slot(&mut self, v: &Value) -> io::Result<()> {
        self.output.emit_i32_slot(&mut *self.w, v)
    }

    /// Emit a `Value` that the runtime helper takes as a `DbRef`.
    ///
    /// [`Self::emit`] renders a bare `Value::Null` as `()`, because it knows the value and
    /// not the slot it is going into.  At a reference-typed operand that is Rust which does
    /// not compile, so ask the one home for what a null of this kind looks like
    /// (`Output::write_typed_null_in`) instead of spelling `DbRef::NULL` at each emitter —
    /// the same repair the two call paths in `calls.rs` make for a typed parameter
    /// (loft#1234).
    ///
    /// Reach for this in a hand-written emitter wherever the generated call takes a
    /// `DbRef`; `emit` stays right for operands whose Rust type follows the value.
    pub fn emit_ref(&mut self, v: &Value) -> io::Result<()> {
        if matches!(v.unspan(), Value::Null) {
            return crate::generation::Output::write_typed_null_in(
                &mut *self.w,
                &Type::Reference(u32::MAX, crate::data::Deps::none()),
                true,
            );
        }
        self.emit(v)
    }
}

/// Trait every per-Op emitter implements.  The default emitter
/// dispatches to `#rust` template substitution unchanged.
pub trait OpEmitter: Send + Sync {
    fn emit(&self, ctx: &mut EmitCtx<'_, '_>, args: &[Value]) -> io::Result<()>;
}

/// Dispatch entry point — every Op-emission call site routes here.
///
/// When `name` is registered, calls the custom emitter; otherwise falls
/// through to `DefaultEmitter` which delegates to
/// `Output::substitute_template_body`.
pub fn emit_op(ctx: &mut EmitCtx<'_, '_>, name: &str, args: &[Value]) -> io::Result<()> {
    if let Some(emitter) = registry().get(name) {
        emitter.emit(ctx, args)
    } else {
        default::DefaultEmitter.emit(ctx, args)
    }
}

/// True when a custom emitter is registered for `name`.
///
/// Used by `dispatch.rs::output_call_inner` to give registered emitters
/// priority over the special-case match arms (phase 00 step 0.6).
/// While the registry is empty this always returns false, so the
/// dispatch falls through to today's match unchanged.
pub fn has_custom_emitter(name: &str) -> bool {
    registry().contains_key(name)
}

/// @P283 — Op-name rewrite for `RefVar(Text)` destinations.
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

/// Registry of custom emitters, keyed by Op name.
///
/// Phase 00 ships an empty registry — every Op falls through to the
/// default template substitution.  Future phases populate this map by
/// inserting boxed trait objects.
fn registry() -> &'static std::collections::HashMap<&'static str, Box<dyn OpEmitter>> {
    static R: std::sync::OnceLock<std::collections::HashMap<&'static str, Box<dyn OpEmitter>>> =
        std::sync::OnceLock::new();
    R.get_or_init(build_registry)
}

fn build_registry() -> std::collections::HashMap<&'static str, Box<dyn OpEmitter>> {
    let mut r: std::collections::HashMap<&'static str, Box<dyn OpEmitter>> =
        std::collections::HashMap::new();

    // Plan 09 phase 00's forwarding-smoke emitters were retired by
    // plan-12 phase 02 — their job (proving the registry → emit_op →
    // DefaultEmitter dispatch fires across many Op shapes) is now
    // done load-bearingly by the 5 production emitters below.
    // Forwarding entries persisted as pure overhead with conceptually
    // misleading "custom emitter registered" status.

    // Phase 03 — parallel-for emitter family.  Replaces the 95-line
    // special case in dispatch.rs::output_call_inner.  Both Op names
    // share the same emitter (the special case treated them
    // identically; n_parallel_for_light is a thread-count hint that
    // doesn't change emission shape).
    r.insert("n_parallel_for", Box::new(parallel::ParallelForEmitter));
    r.insert(
        "n_parallel_for_light",
        Box::new(parallel::ParallelForEmitter),
    );

    // Coroutine family (N8b native generators) — migrated out of the
    // dispatch match (`dispatch_op_arm_budget` ratchet).  Both are
    // pass-throughs to `loft::codegen_runtime::coroutine_*`.
    r.insert(
        "OpCoroutineNext",
        Box::new(coroutine::OpCoroutineNextEmitter),
    );
    r.insert(
        "OpCoroutineExhausted",
        Box::new(coroutine::OpCoroutineExhaustedEmitter),
    );

    // Phase 04 — key-keyed Op emitters.  Replaces ~70 lines of two
    // arms in dispatch.rs (`"OpGetRecord" =>` + `"OpIterate" =>`).
    // @PLN157 P3 — float/single compares go plain and integer arithmetic
    // drops its operand pre-tests when the operands are provably
    // non-sentinel; the template form otherwise.
    for op in [
        "OpAddInt",
        "OpMinInt",
        "OpMulInt",
        "OpMinSingleInt",
        "OpLandInt",
        "OpLorInt",
        "OpEorInt",
        "OpSRightInt",
        "OpConvFloatFromInt",
        "OpConvBoolFromInt",
    ] {
        r.insert(op, Box::new(int_arith::IntArithEmitter));
    }
    for op in [
        "OpEqFloat",
        "OpNeFloat",
        "OpLtFloat",
        "OpLeFloat",
        "OpEqSingle",
        "OpNeSingle",
        "OpLtSingle",
        "OpLeSingle",
    ] {
        r.insert(op, Box::new(float_compare::FloatCompareEmitter));
    }
    // @PLN157 P4b — scalar element writes fuse against a hoisted header;
    // the same ops fall back to their template for record-field writes and
    // outside hoisted loops.
    for op in ["OpSetInt", "OpSetSingle", "OpSetFloat"] {
        r.insert(op, Box::new(vector_ops::FusedElementWriteEmitter));
    }
    // @PLN157 § V-q — a scalar push goes through the loop's push header; the same ops
    // fall back to their template outside a loop that hoisted one.
    for (op, _, _) in crate::generation::hoist::FUSABLE_PUSHES {
        r.insert(op, Box::new(vector_ops::HoistedPushEmitter));
    }
    r.insert("OpPreAllocVector", Box::new(vector_ops::PreAllocEmitter));
    r.insert("OpNewRecord", Box::new(vector_ops::NewRecordEmitter));
    r.insert("OpFinishRecord", Box::new(vector_ops::FinishRecordEmitter));
    r.insert("OpGetRecord", Box::new(key_ops::OpGetRecordEmitter));
    r.insert("OpIterate", Box::new(key_ops::OpIterateEmitter));

    // Keyed-WRITE family — migrated out of dispatch.rs::output_call_inner to
    // keep that match under the `dispatch_op_arm_budget` ratchet (added as arms
    // during the @PLN6 dogfood: @P305 OpSetKeyed, @P307 OpClearKeyed,
    // OpReplaceKeyed).  Pure pass-throughs to their runtime helpers.
    r.insert("OpReplaceKeyed", Box::new(key_ops::OpReplaceKeyedEmitter));
    r.insert("OpSetKeyed", Box::new(key_ops::OpSetKeyedEmitter));
    r.insert("OpClearKeyed", Box::new(key_ops::OpClearKeyedEmitter));

    // Reference-lifetime family — the irreducible cases.  OpFreeRef /
    // OpFreeRefIfDistinct read per-function variable metadata (conditional
    // emission a template can't express).  OpCopyRecord / OpSizeofRef are
    // cell-passthroughs kept here pending the Phase-D template move.
    // OpEqRef / OpNeRef / OpNullRefSentinel were dropped — their `#rust`
    // templates (pure-expr / literal, no non-rewritable `s.`) are native-valid
    // via DefaultEmitter.
    r.insert("OpFreeRef", Box::new(ref_ops::OpFreeRefEmitter));
    r.insert(
        "OpFreeRefIfDistinct",
        Box::new(ref_ops::OpFreeRefIfDistinctEmitter),
    );
    r.insert(
        "OpFreeRefOrHandUp",
        Box::new(ref_ops::OpFreeRefOrHandUpEmitter),
    );
    // Plan-57 store-identity gate (LOFT_STORE_TAG only): native parity for the
    // verifying store ops.  Absent from normal builds (the IR post-pass emits them
    // only under the gate), so registering them is inert otherwise.
    r.insert("OpStoreTag", Box::new(ref_ops::OpStoreTagEmitter));
    r.insert("OpFreeRefTag", Box::new(ref_ops::OpFreeRefTagEmitter));
    // @PLN87 P2.1 — native twins for the param-rebind sequence (interp-only
    // opcodes otherwise): sentinel-null a slot and raw-copy a DbRef.
    r.insert(
        "OpInitRefSentinel",
        Box::new(ref_ops::OpInitRefSentinelEmitter),
    );
    r.insert("OpPutRef", Box::new(ref_ops::OpPutRefEmitter));
    r.insert("OpCopyRecord", Box::new(ref_ops::OpCopyRecordEmitter));
    r.insert("OpSizeofRef", Box::new(ref_ops::OpSizeofRefEmitter));

    // Match elimination — the text/format/buffer family relocated VERBATIM from
    // the output_call_inner match into one self-contained emitter (it does the
    // @P283 refvar->Stack rewrite + the sub-match internally).  Registered for
    // every base + Stack name so the registry-first guard routes them here.
    for name in [
        "OpFormatInt",
        "OpFormatStackInt",
        "OpFormatFloat",
        "OpFormatStackFloat",
        "OpFormatSingle",
        "OpFormatStackSingle",
        "OpFormatText",
        "OpFormatStackText",
        "OpAppendText",
        "OpAppendStackText",
        "OpAppendCharacter",
        "OpAppendStackCharacter",
        "OpClearText",
        "OpClearStackText",
        "OpClearVector",
        "OpAppendVector",
        "OpFreeText",
        "OpCreateStack",
        "OpFormatDatabase",
        "OpFormatStackDatabase",
    ] {
        r.insert(name, Box::new(text_ops::TextDispatchEmitter));
    }

    // Match elimination — misc pass-throughs relocated verbatim from the match.
    r.insert(
        "OpConvRefFromNull",
        Box::new(misc_ops::OpConvRefFromNullEmitter),
    );
    r.insert("OpGetTextSub", Box::new(misc_ops::OpGetTextSubEmitter));
    r.insert("OpDatabase", Box::new(misc_ops::OpDatabaseEmitter));
    r.insert("OpStep", Box::new(misc_ops::OpStepEmitter));
    r.insert("OpRemove", Box::new(misc_ops::OpRemoveEmitter));

    // Phase 06 — parallel-queue emitter family (P202 close).  Mirrors
    // phase 03's for-par emitter for the queue protocol: `n_parallel_queue`
    // / `_text` / `_ref` build a worker closure and call the
    // `n_parallel_queue_*_native` runtime fns; `n_parallel_buf_get` /
    // `_text` / `_ref` and `n_parallel_buf_drop` / `_text` / `_ref` are
    // pass-through renames to their `_native` runtime counterparts.
    r.insert("n_parallel_queue", Box::new(parallel::ParallelQueueEmitter));
    r.insert(
        "n_parallel_queue_text",
        Box::new(parallel::ParallelQueueEmitter),
    );
    r.insert(
        "n_parallel_queue_ref",
        Box::new(parallel::ParallelQueueEmitter),
    );
    // Plan-06 ARC.md A3 — narrow-Integer queue routes through the
    // same emitter (which checks the worker return type and swaps
    // to `n_parallel_queue_narrow_native`).  buf_get_narrow and
    // buf_drop_narrow are pass-through renames.
    r.insert(
        "n_parallel_queue_narrow",
        Box::new(parallel::ParallelQueueEmitter),
    );
    // #281 — fn-ref-return queue routes through the same emitter (Fn shape
    // → n_parallel_queue_fn_native); buf_get_fn / buf_drop_fn are
    // pass-through renames to their _native counterparts.
    r.insert(
        "n_parallel_queue_fn",
        Box::new(parallel::ParallelQueueEmitter),
    );
    // Plan-06 ARC.md A5b — fold emitter.  Closure-based bridge to
    // `n_parallel_fold_native` (runtime helper in
    // `src/codegen_runtime.rs`).  Different arg layout from the
    // for/queue family — see `ParallelFoldEmitter` doc.
    // The Discard route (`par(...)` with a body that never names the result).  It has
    // to be here for the same reason the rest of the family is: the declaration-driven
    // default emits `n_parallel_discard`'s loft declaration, whose body is empty, so
    // the workers silently never ran (loft#987).
    r.insert(
        "n_parallel_discard",
        Box::new(parallel::ParallelDiscardEmitter),
    );
    r.insert("n_parallel_fold", Box::new(parallel::ParallelFoldEmitter));
    for name in [
        "n_parallel_buf_get",
        "n_parallel_buf_get_text",
        "n_parallel_buf_get_ref",
        "n_parallel_buf_get_narrow",
        // ARC.md A3.6 — typed Single/Float readers.  Same buffer
        // stacks as buf_get_narrow / buf_get respectively; the
        // emitter is a pass-through rename to the n_<name> impl.
        "n_parallel_buf_get_single",
        "n_parallel_buf_get_float",
        "n_parallel_buf_drop",
        "n_parallel_buf_drop_text",
        "n_parallel_buf_drop_ref",
        "n_parallel_buf_drop_narrow",
        // #281 — fn-ref-return row reader + buffer drop.
        "n_parallel_buf_get_fn",
        "n_parallel_buf_drop_fn",
    ] {
        r.insert(name, Box::new(parallel::ParallelBufRenameEmitter));
    }

    // loft#885 — indexed reads inside a loop that writes no store are emitted against a
    // header the loop derived once.  Both emitters delegate to the `#rust` template for
    // every read the analysis did not cover, which is every read outside such a loop.
    // @PLN157 P4c — the same emitter serves every scalar field getter: a read the loop
    // hoisted as a record scalar emits the local the prelude bound, and the three
    // fusable kinds try the element fusion first.
    for name in crate::generation::hoist::SCALAR_GETTERS {
        r.insert(name, Box::new(vector_ops::FusedElementReadEmitter));
    }
    r.insert("OpGetVector", Box::new(vector_ops::OpGetVectorEmitter));
    // @PLN157 — the loop bound reads the hoisted header's length, not the store table.
    r.insert("OpLengthVector", Box::new(vector_ops::HoistedLengthEmitter));
    r.insert(
        "OpGetVectorNullable",
        Box::new(vector_ops::OpGetVectorNullableEmitter),
    );

    // Phase 10 step 10.3 — integer comparison emitter family.
    // Closes P200 read-side: the default `@v1 == @v2` template
    // produces E0308 when the LHS is a narrowed block (`(...) as u8`)
    // and the RHS is an `_i64` literal.  This emitter wraps each
    // operand in `(... as i64)` so both sides match width.
    for name in ["OpEqInt", "OpNeInt", "OpLtInt", "OpLeInt"] {
        r.insert(name, Box::new(int_compare::IntCompareEmitter));
    }

    r
}

#[cfg(test)]
mod tests {
    //! Smoke-test the `OpEmitter` trait shape so phase 00's claim that
    //! "custom emitters can opt in per Op" is checked at compile time
    //! before phase 05 lands the first real one.
    //!
    //! These tests are compile-only — they prove that:
    //! - The trait can be impl'd by an external struct.
    //! - `EmitCtx` field access works from inside an impl (split-borrow
    //!   between `ctx.output` and `ctx.w` doesn't fight the borrow
    //!   checker).
    //! - The trait dispatch (`emit_op`) accepts a `&mut EmitCtx<'_, '_>`
    //!   without lifetime gymnastics at the call site.
    //!
    //! What they do NOT cover (deferred until a real Output fixture
    //! exists in the test surface):
    //! - Calling `ctx.output.<method>` from inside an emitter.
    //! - Verifying that the registry lookup actually finds a registered
    //!   emitter (the real registry is `OnceLock`-backed; tests can't
    //!   mutate it without a feature flag).
    //!
    //! Phase 05's first real custom emitter is the integration test for
    //! the deferred parts.

    use super::{EmitCtx, OpEmitter, has_custom_emitter, registry};
    use crate::data::Value;
    use std::io;

    /// Minimal emitter that doesn't touch any context state.  Compiles
    /// only if the trait's method signature is satisfiable by an
    /// external impl (no hidden lifetime constraints).
    struct StubEmitter;
    impl OpEmitter for StubEmitter {
        fn emit(&self, _ctx: &mut EmitCtx<'_, '_>, _args: &[Value]) -> io::Result<()> {
            Ok(())
        }
    }

    /// Emitter that writes to `ctx.w`.  Compiles only if the field
    /// access pattern works without re-borrow conflicts.
    struct WriterEmitter;
    impl OpEmitter for WriterEmitter {
        fn emit(&self, ctx: &mut EmitCtx<'_, '_>, _args: &[Value]) -> io::Result<()> {
            write!(ctx.w, "/* writer-emitter stub */")
        }
    }

    /// Emitter that reads `ctx.def_fn` while writing to `ctx.w`.
    /// Compiles only if the split-borrow rules let us hold both an
    /// immutable ref to `def_fn` and a mutable ref to `w` from the
    /// same `&mut EmitCtx`.
    struct DefAccessEmitter;
    impl OpEmitter for DefAccessEmitter {
        fn emit(&self, ctx: &mut EmitCtx<'_, '_>, _args: &[Value]) -> io::Result<()> {
            let name = ctx.def_fn.name();
            write!(ctx.w, "/* {name} */")
        }
    }

    #[test]
    fn op_emitter_trait_is_impl_able() {
        // If any of the three structs above don't compile, this test
        // doesn't compile.  The body just instantiates them so the
        // structs aren't dead-code-eliminated.
        let _: &dyn OpEmitter = &StubEmitter;
        let _: &dyn OpEmitter = &WriterEmitter;
        let _: &dyn OpEmitter = &DefAccessEmitter;
    }

    /// Verify the registry is queryable and (currently) empty.  When a
    /// future phase populates it, this test's count goes up — that's
    /// expected behaviour, not a regression, but the diff makes it
    /// visible.
    #[test]
    fn registry_count_matches_sanctioned_set() {
        let count = registry().len();
        // The registry holds every Op whose native emission lives in code
        // rather than a `#rust` template.  It grew substantially when the
        // `output_call_inner` match was eliminated — the text/format/buffer
        // family (one `TextDispatchEmitter` registered for ~19 base+Stack
        // names) and the misc pass-throughs (OpConvRefFromNull / OpGetTextSub /
        // OpDatabase / OpStep / OpRemove) moved into the registry, on top of
        // the parallel family, key_ops, ref-lifetime, coroutine and IntCompare
        // emitters.  This is the intended end state (the registry IS the
        // dispatch mechanism); the cap just makes a surprising jump visible.
        // @PLN157 P3 raised it by 18: the eight Eq/Ne/Lt/Le × Single/Float
        // compares (FloatCompareEmitter) and the ten integer
        // arithmetic/conversion ops (IntArithEmitter) — both emit the
        // template form unless the non-sentinel proof holds.  @PLN157 P4c
        // raised it by 11: `FusedElementReadEmitter` now serves all fourteen
        // scalar field getters (a hoisted record scalar emits its local), not
        // only the three fusable kinds.
        assert!(
            count <= 109,
            "registry has {count} custom emitters — bump the cap if \
             this is intentional and document here"
        );
    }

    /// Verify `has_custom_emitter` returns false for unregistered
    /// names.  Uses a synthetic name guaranteed not to be in the
    /// registry (real Op names like `OpAddInt` are forwarded via
    /// phase 00's runtime smoke test, so they ARE registered).
    #[test]
    fn has_custom_emitter_returns_false_for_unregistered() {
        assert!(!has_custom_emitter("ThisOpDoesNotExist"));
        assert!(!has_custom_emitter("OpSyntheticUnregisteredForTest"));
    }
}
