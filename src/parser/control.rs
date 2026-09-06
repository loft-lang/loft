// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use crate::data::Deps;
use crate::data::IntegerSpec;
use std::collections::HashSet;

/// Which kind of return site `ref_return` is processing: a BODY-TAIL
/// site may NRVO-rename a local into the buffer attr (the local IS the
/// buffer for the whole fn), while a MID-BODY `return` is one site
/// among several — its named locals must never become arguments (the
/// 01b breakage), and its bare-call value needs the explicit
/// `Set + Var` shape or native loses it to the `Return(Null)`
/// fall-through (#356).
#[derive(Debug, PartialEq, Clone, Copy)]
pub(crate) enum RetSite {
    BlockTail,
    MidReturn,
}

/// @PLN35 — what a `( … )` group in a slice pattern desugars to, decided by ONE lexical
/// look-ahead (`peek_group_kind`).  A single peek (never two sequential save/reverts over the
/// same region — those corrupt the lexer's replay buffer) keeps the scan minimal and the
/// downstream diagnostic columns stable.
#[derive(PartialEq, Clone, Copy)]
enum SliceGroupKind {
    /// A single-element alternation `(A | B)` or a bare group — handled by the fall-through
    /// (`parse_slice_alternation_element`); the peek stops early so it does not over-scan.
    Other,
    /// A multi-element sequence alternation `(A B | C)` (Phase 4.3) or an optional `(a)?`
    /// (Phase 5, a degenerate `(a | ε)`) — `parse_multi_element_alternation`.
    Alt,
    /// A repetition `( [name:] V )*` / `…+` (Phase 6) — `parse_slice_repetition`.
    Repetition,
}

/// @PLN35 Phase 6.2 — how a repetition separator `( … )` matches: a variant TAG `(Comma)` or a
/// LEXEME literal `(",")` (a `#lexeme`/scalar equality, so a comma-separated token grammar reads
/// `(arg)*(",")` instead of needing a dedicated separator variant).
enum SepSpec {
    Variant(i32),
    Lexeme(Value, Type),
}

/// @PLN85 text-return analysis framework (SHADOW — not yet wired to codegen).
///
/// The ONE property behind the stacked per-shape promotion predicates (2d
/// native-call, 3a view-of-local, 3b user-call, 3c if/match arm) and the p281
/// borrow exclusion: *does a text return TAIL deliver a fresh OWNED text (→
/// promote to a hidden `&text` caller buffer) or BORROW a caller-owned value
/// (→ forward the borrow, never promote)?*  `classify_text_return` is the pure
/// selector; `OwnedVia`/`BorrowVia` record WHICH shape produced the verdict so
/// the corpus can verify it.  Verified beside the tests (via the `LOFT_TRA_DUMP`
/// dump) before any codegen switch — see
/// `plans/85-store-lifetime-retirement/probes/text-tail-return/framework/`.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum TextReturn {
    /// Fresh owned text — promote the return to a hidden `&text` caller buffer.
    Owned(OwnedVia),
    /// A borrow of a caller-owned value — forward it, do NOT promote.
    Borrow(BorrowVia),
    /// Not a promotable text-return shape: a literal-only tail, or a shape
    /// whose leak (if any) is NOT in the return delivery (a consumed local, a
    /// native-internal buffer) — the framework points AWAY from return promotion.
    Plain,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum OwnedVia {
    /// Native text-dest call in tail position (`u.to_json()`) — attempt 2d.
    NativeCall,
    /// User fn call that itself delivers owned text (`inner()`) — slice 3b.
    UserCall,
    /// Text field/index view of a LOCAL composite built in this body
    /// (`a.v.0`, `d.ts[0]`) — slice 3a.
    ViewOfLocal,
    /// A user fn call that returns a forward-borrow of a VISIBLE param
    /// (`extract(p) -> text["p"]`), but where THIS call site fills that param
    /// with a LOCAL composite built here (`extract(Pair{…})` / `extract(pr)`).
    /// The borrow is of a value that dies with this frame, so the tail is
    /// materialised (copied) into the promoted `&text` buffer before the local
    /// is freed — owned, exactly like [`ViewOfLocal`], just delivered through a
    /// call.  Distinct variant so `tret_bind_ok` can gate it on a BACKWARD-ref
    /// callee (same pass-stability rule as `UserCall`); the direct
    /// `ViewOfLocal` needs no such gate.  @PLN85 p54_b6.
    ViewOfLocalCall,
    /// An `if`/`match` (match lowers to `If`) with an owned-text arm — slice 3c.
    IfMatchArm,
    /// A built-up local text — accumulator / interpolation / literal-concat /
    /// rebind — delivered as the tail var; `text_return` already promotes these.
    BuiltLocal,
    /// A TUPLE-constructor return (`return (x.to_text(), "mid", …)`) at least
    /// one of whose text elements delivers owned text — p329/p330_pair.  Needs
    /// per-element `&text` buffer promotion (the `__ret_text_N` hoist).
    TupleElement,
    /// A fn-REF call in tail position (`f(42)`, `g.fmt(42)` — `Value::CallRef`)
    /// delivering owned text — p227.  Promotable, but the fn-ref dispatch ABI
    /// is @P387-adaptive, so the wiring must keep it uniform.
    FnRefCall,
}

#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum BorrowVia {
    /// A text field-view rooted at a caller-owned ARGUMENT (`fn f(p) { p.a }`).
    Argument,
    /// A call to a fn that itself returns an argument-borrow (`f(s){ g(s) }`,
    /// `g(s) -> text["s"]`) — the p281 shape; promoting it breaks the forward.
    ForwardArg,
}

impl TextReturn {
    /// Stable label for the `LOFT_TRA_DUMP` shadow dump + corpus verification.
    pub(crate) fn label(self) -> &'static str {
        match self {
            TextReturn::Owned(OwnedVia::NativeCall) => "Owned:NativeCall",
            TextReturn::Owned(OwnedVia::UserCall) => "Owned:UserCall",
            TextReturn::Owned(OwnedVia::ViewOfLocal) => "Owned:ViewOfLocal",
            TextReturn::Owned(OwnedVia::ViewOfLocalCall) => "Owned:ViewOfLocalCall",
            TextReturn::Owned(OwnedVia::IfMatchArm) => "Owned:IfMatchArm",
            TextReturn::Owned(OwnedVia::BuiltLocal) => "Owned:BuiltLocal",
            TextReturn::Owned(OwnedVia::TupleElement) => "Owned:TupleElement",
            TextReturn::Owned(OwnedVia::FnRefCall) => "Owned:FnRefCall",
            TextReturn::Borrow(BorrowVia::Argument) => "Borrow:Argument",
            TextReturn::Borrow(BorrowVia::ForwardArg) => "Borrow:ForwardArg",
            TextReturn::Plain => "Plain",
        }
    }

    /// True when this owned return is delivered by the `__tret` bind-and-promote.
    ///
    /// Scoped to the FORWARD-REFERENCE-SAFE verdicts:
    /// - `ViewOfLocal` — a field/index view resolves to `OpGetText` on PASS 1,
    ///   so the buffer lands in the signature before any forward-ref caller is
    ///   compiled (verified: `fref_view`).
    /// - `NativeCall` — a native text-dest CALL tail (attempt 2d).  A method
    ///   call resolves only on PASS 2, so this binds pass-2-only in practice
    ///   (identical to 2d) and carries 2d's latent forward-ref limitation.
    ///
    /// - `UserCall` — a user-fn text CALL tail, but the callee-side gate in
    ///   `tret_bind_ok` additionally restricts it to a BACKWARD reference (callee
    ///   defined before this fn).  A forward-referenced callee classifies `Plain`
    ///   on pass 1 (its return type is unresolved there) and `UserCall` on pass 2,
    ///   so promoting it pass-2-only diverges the ABI and crashes a forward-ref
    ///   caller ("Too few parameters" — the viewer/p281 class).  A backward ref is
    ///   `UserCall` on BOTH passes → pass-stable → safe.
    ///
    /// `IfMatchArm` is delivered by the per-arm accumulator (`push_text_arms_into`).
    /// `BuiltLocal` is already promoted by `text_return`.  `FnRefCall` promotes
    /// through the P227 adaptive hidden-`&text`-buffer fn-ref ABI (a `CallRef`
    /// tail is structurally stable across passes, so it needs no backward-ref
    /// gate).  `TupleElement` still needs its own per-element delivery.  The
    /// forward-ref `UserCall` case (and generic-monomorph callees, whose def_nr
    /// is minted pass-2 and so reads as forward) await the signature pre-pass.
    fn wants_tret_bind(self) -> bool {
        matches!(
            self,
            TextReturn::Owned(
                OwnedVia::NativeCall
                    | OwnedVia::ViewOfLocal
                    | OwnedVia::ViewOfLocalCall
                    | OwnedVia::UserCall
                    | OwnedVia::FnRefCall
            )
        )
    }
}

/// @PLN85 / D-own-1 — how an implicit-tail `t == Vector` return delivers its
/// value into the fn's one `__retbuf` buffer. The SELECTOR
/// (`classify_vector_delivery`) reads the deps fact + tail shape once and picks a
/// variant; the dispatch (`dispatch_vector_delivery`) emits the matching
/// mechanism. This collapses the per-branch shape re-handling the vector arm of
/// `block_result` used to inline (OWNERSHIP_MODEL.md: ownership read once, not
/// re-derived per tail-shape at the delivery site).
enum Delivery {
    /// Promote the tail's work-ref(s) to BE `__retbuf` (no copy):
    /// `ref_return(ws) + nrvo_collapse_tail_set(ws)`. The owned-fresh / hidden-ref
    /// recovery (#120) / multi-arm (#437, cluster-V) case.
    Rename(Vec<u16>),
    /// The tail BORROWS a visible argument (the whole vector arg A.2, or a struct
    /// vector FIELD #415) — copy it into `__retbuf` for value semantics. `ls` is
    /// carried for the fallback rename if the copy's work-var allocation fails.
    CopyBorrow(Vec<u16>),
    /// A `#native`/`#rust` callee delivers its OWN store and never writes the
    /// `__retbuf` it was handed (#409) — mint a local, run the call into it, copy
    /// in. A no-op when there is no buffer or no work-var.
    ForwardCopy,
    /// Per-arm / fresh-local element COPY into `__retbuf` via
    /// `materialize_vector_arms_into`: a `match`/`if` branch tail (#416,
    /// cluster-II) OR a fresh-local tail whose buffer is already TAKEN by a sibling
    /// (#448). Finalises `returned` to `{__retbuf}` (idempotent when already set).
    Materialize,
    /// The tail already writes `__retbuf` (or there is no buffer / nothing to
    /// recover) — emit nothing here.
    AsIs,
}

/// @PLN85 D-own-1 — the delivery mechanism for a `Type::Reference` (struct) return,
/// the Reference counterpart of [`Delivery`]. Two mechanisms keyed on the deps fact:
/// rename the tail's work-ref(s) onto `__retbuf`, or materialise-copy a tail that
/// borrows a LOCAL (#306) before it escapes. (The nullable-unwrap tail is handled by
/// its own earlier `block_result` arm and is NOT routed here.)
/// @PLN85 D-own-1 — the per-var verdict for `text_return`'s promotion of a
/// return-dep variable (see `classify_text_dep`).
enum TextDep {
    /// Already an attribute — record its index in the return dep.
    Attr(u16),
    /// Captured closure var — read from the closure record; never promoted.
    SkipCaptured,
    /// A tuple LOCAL (@P330) — no hoist; the B5-L3 temp covers the copy.
    SkipTupleLocal,
    /// A non-argument LOCAL (loft#771) — no hoist; promoting it would hand the
    /// free obligation to a caller that never receives the store.
    SkipOwnedLocal,
    /// A text local in a function that already carries a work buffer (loft#1113)
    /// — no hoist; the fn-ref call ABI passes exactly one, so a second is
    /// unreachable through it.
    SkipSecondTextBuf,
    /// A text local — promote to a hidden `RefVar(Text)` work-buffer param.
    PromoteHidden,
    /// Any other dep type — promote as a plain (visible) parameter.
    PromotePlain,
}

/// @PLN85 D-own-1 slice 3 — the per-var verdict of `ref_return`'s NRVO /
/// promotion ladder (see `classify_ret_promotion`).  One variant per rule; the
/// apply loop in `ref_return` carries only the emission mechanics.
#[derive(Debug)]
enum RetPromotion {
    /// Already DELIVERED into a separate `__retbuf` by the jo_arm pre-pass
    /// (@PLN85 match_return) — a local now, not the return; promoting it would
    /// re-alias the buffer onto the binding var.
    SkipDelivered,
    /// Plan-57 reassigned local, and the #355 named-local fall-through does not
    /// apply — never NRVO-promoted (each fresh literal would build INTO the
    /// buffer; the second would append, not replace).
    SkipReassigned,
    /// Name already an attribute — merge its index into the return deps.
    /// `chain_site` = the #356 mid-body bare-call site must have its value made
    /// explicit each pass (native loses it to the Return(Null) fall-through
    /// once argument lifting decomposes the call).
    MergeAttr { a: u16, chain_site: bool },
    /// Transitively-reached (#306) — dep merge only, never promote: hidden-ref
    /// promotion would change the call ABI for locals the NRVO machinery
    /// cannot host (e.g. a call-result vector).
    MergeOnly,
    /// loft#1182 — a CAPTURED closure variable, read out of the closure record: never
    /// promoted, which is what [`TextDep::SkipCaptured`] has said about the text half all
    /// along.  There is no store to place — the capture belongs to the frame that made it —
    /// so growing a buffer for it declares a delivery the body then ignores, and
    /// `--native`'s dispatch keeps that buffer because the candidate's return deps name it.
    SkipCaptured,
    /// An inner work ref that is not — and is not ADOPTED by (cluster I-d) —
    /// the site's value: stays a plain local; the outer call deep-copies its
    /// record into the destination before scope exit frees it.  Sentinel sweep
    /// 2026-07-03: 0 firings suite-wide (a non-site-value work ref only ever
    /// arrives transitively) — unreachable-suspected, kept as the guard.
    SkipInnerRef,
    /// @PLAN59/H1 NRVO — rename the signature-time `__retbuf` ATTR to this
    /// local's name (the attr↔var coupling is by name, probe C3) and retire
    /// the placeholder argument var (same last frame slot by var-number order,
    /// probe C6).  `chain_site` as in `MergeAttr` (#356).
    Rename { buf_attr: usize, chain_site: bool },
    /// ONE-BUFFER invariant (stability roadmap #1): a plain fn's arity is
    /// FIXED at signature parse — the site BINDS to the one existing buffer.
    /// `substitute` — a parser-minted work ref (referenced only at its own
    /// return site) is substituted BY the buffer var so the call writes
    /// directly into the caller's buffer (return paths are mutually
    /// exclusive); a named local (readable by sibling return sites —
    /// substitution could alias the buffer into another site's argument list)
    /// keeps its own store and is deep-copied into the buffer at the return.
    Bind {
        buf_attr: u16,
        buf_var: u16,
        substitute: bool,
    },
    /// loft#1078 — this candidate is one arm of a runtime JOIN whose OTHER arm was
    /// already renamed onto the return buffer, so the tail READS the buffer.  `Bind`'s
    /// copy leg would emit `OpDatabase(buf); OpCopyRecord(<tail reading buf>, buf)` —
    /// the re-mint destroys the very store the copy is about to read, and the renamed
    /// arm answers a zeroed record (`if c { u } else { w }` gave `0` for `u` on both
    /// backends, and a three-arm `match` broke only its first arm).  Stay a plain
    /// local instead: the join delivers one store, and the multi-source
    /// `OpFreeRefIfDistinct` leg in `scopes::free_vars` frees the losers — the same
    /// lowering the single-named-local shape already uses and which loft#1078's own
    /// leak fix proved out.
    SkipJoinArm,
    /// Vector / struct-Enum returns on LAMBDAS still grow the arity in place:
    /// a lambda is defined at its literal site and invoked via CallRef, so no
    /// earlier caller can hold a short arg list.  PASS-1 growth on a plain fn
    /// is sound (pass 2 re-parses every caller against the final arity);
    /// PASS-2 growth on a plain fn must never happen (asserted at the apply).
    Grow,
}

/// Shared per-return-site context for `classify_ret_promotion`.
struct RetPromoCtx<'a> {
    site: RetSite,
    ret: &'a Type,
    /// The ref carrying THE SITE'S VALUE (a tail call's buffer arg / a plain
    /// Var tail).  Only this ref may bind to the fn's one return buffer; an
    /// INNER call's work ref (`return wrap(mk(x))` carries two) must stay a
    /// plain local — binding both would alias the outer call's destination
    /// with its own argument.
    site_value: Option<u16>,
    is_plain_fn: bool,
    newrecord_nr: u32,
    jo_arm_skip: &'a std::collections::HashSet<u16>,
}

enum RefDelivery {
    /// Promote the tail's work-ref(s) to BE `__retbuf` — `ref_return(ws) +
    /// nrvo_collapse_tail_set(ws)`. Covers #120 hidden-ref recovery (`ls` empty,
    /// `ws` = recovered) AND the plain arg-borrow/owned rename (`ws` = `ls`).
    Rename(Vec<u16>),
    /// The tail borrows a LOCAL's store (#306) — copy it into an owned work-ref
    /// via `materialize_view_return` before it escapes, then rename that.
    MaterializeView,
    /// #409 — a `#native`/`#rust` callee delivers its OWN record and never writes
    /// the `__retbuf` it was handed: bind the forwarded store to a fresh `__fwd`
    /// local, copy the record into `__retbuf`, and let scope analysis free `__fwd`.
    /// The Reference twin of [`Delivery::ForwardCopy`].
    ForwardCopy,
    /// `ls` empty and no work-ref to recover — the tail already delivers; emit nothing.
    AsIs,
}

use super::{
    DefType, I32, Level, LexItem, Parser, Position, Type, Value, diagnostic_format,
    merge_dependencies, v_block, v_if, v_loop, v_set,
};

/// Why an enclosing-scope capture inside a `parallel {}` arm is rejected.
/// See `parse_parallel` — each arm runs in an isolated worker (read-only heap
/// clone + private stack), so only *reading* an enclosing local is sound.
#[derive(Clone, Copy, PartialEq)]
enum ParViolation {
    /// Capturing a function parameter (read or write) — SIGSEGVs at teardown.
    Param,
    /// Writing or mutating an enclosing local — write is silently dropped
    /// (scalar/text) or crashes on the read-only store clone (heap).
    Mutation,
}

/// True for the IR ops that MUTATE their host — the var reachable from the
/// op's `args[0]` spine.  `Set(v, _)` is handled separately (the target var is
/// the node's first field); these are the in-place / element / field forms that
/// hide the host inside a read-projection chain.  (See the plan-57 capture
/// investigation for the exhaustive shape list.)
fn is_mutating_op(name: &str) -> bool {
    matches!(
        name,
        "OpSetInt"
            | "OpSetByte"
            | "OpSetShort"
            | "OpSetInt4"
            | "OpSetFloat"
            | "OpSetSingle"
            | "OpSetEnum"
            | "OpSetCharacter"
            | "OpSetText"
            | "OpSetKeyed"
            | "OpReplaceKeyed"
            | "OpClearKeyed"
            | "OpAppendVector"
            | "OpClearVector"
            | "OpNewRecord"
            | "OpFinishRecord"
            | "OpCopyRecord"
    )
}

/// A7.1: walk a body-tail expression and report whether it ends in
/// a literal `Value::Tuple(...)` at any reachable tail position.  Used
/// by `block_result` to decide whether the synthetic-struct rewrite
/// should fire.  Mirrors the recursion shape of
/// `rewrite_tail_tuple_with_work_ref` so the gate and the rewrite stay
/// in sync.
fn tail_has_tuple_leaf(value: &Value, vars: &crate::variables::Function) -> bool {
    match value.tail() {
        Value::Tuple(_) => true,
        // loft#821 — the same stack tuple after something stashed it in a local: the `??`
        // lowering binds its subject to `__ncc_N`, so `v[i] ?? (0, "none")` reaches here as
        // a Var on one arm and a literal on the other.  Both are elements on the stack and
        // both need boxing; recognising only the literal left the two arms disagreeing at
        // the join — a DbRef on one side, three floats on the other.
        Value::Var(v) => matches!(vars.tp(*v).base(), Type::Tuple(_)),
        Value::If(_, then_branch, else_branch) => {
            tail_has_tuple_leaf(then_branch, vars) || tail_has_tuple_leaf(else_branch, vars)
        }
        _ => false,
    }
}

/// Check if the last meaningful expression in a block is divergent.
fn is_block_divergent(ops: &[Value]) -> bool {
    ops.iter()
        .rev()
        .any(|v| matches!(v, Value::Return(_) | Value::Break(_) | Value::Continue(_)))
}

/// Collected match arm data for enum/struct-enum match expressions.
struct EnumArm {
    /// Discriminants for this arm — Vec allows or-patterns (multiple variants per arm).
    discs: Vec<i32>,
    code: Value,
    tp: Type,
    guard: Option<Value>,
    bindings: Vec<Value>,
}

/// One arm of a vector or tuple `match`, collected before the if-chain is assembled.
///
/// `guard` stays SEPARATE from `cond` instead of being ANDed into it, because the arm's
/// captures are assigned by `bindings` and a guard that reads a capture has to see it
/// already assigned.  ANDing the two ran the guard against unassigned variables, so
/// `(n, _, true) if n > 10` compared `0 > 10` and the arm silently did not match — which
/// is why enabling the parse alone would have turned a clean compile error into a wrong
/// answer (loft#839).
struct PatternArm {
    /// The pattern test — a length check, element literals, or `None` for a total pattern.
    cond: Option<Value>,
    guard: Option<Value>,
    /// Assignments for this arm's captures.  Non-empty only when `guard` is `Some`;
    /// without a guard they are already folded into `code`, keeping the emitted IR of
    /// every existing match byte-identical.
    bindings: Vec<Value>,
    code: Value,
}

/// Assemble collected [`PatternArm`]s into the match's if-chain, last arm first.
///
/// A guarded arm lowers to `if <cond> { <bindings>; if <guard> { <body> } else { <rest> } }
/// else { <rest> }` — captures assigned once, inside the branch their pattern already
/// selected, before the guard reads them.  The `<rest>` appears twice, as it does in the
/// enum arm chain this mirrors; only an arm that actually carries a guard pays for it.
/// Bindings are never duplicated: a slice arm's are a `..name` materialisation and a PEG
/// cursor advance, neither of which survives being run twice.
/// Is this arm or branch body a `null` — written bare (`0 => null`) or as a BLOCK
/// (`0 => { null }`)?
///
/// loft#936 established that a branch-merge slot must carry the result type's typed null
/// sentinel, never a bare `Value::Null`, which pushes nothing where the merge reads a
/// 12-byte `DbRef`.  The repair then has to RECOGNISE a null arm, and the block spelling is
/// the half that kept getting missed: three of the four match lowerings tested only the bare
/// form, so `match n { 0 => { null }, _ => { [n] } }` answered null for every `n` while the
/// bare-arm spelling of the same function was right.
///
/// One predicate for all of them, because the previous arrangement was one copy each.
///
/// ⚠ Do NOT fold this together with `Parser::arm_is_null`, which looks similar and answers a
/// DIFFERENT question.  That one also counts `OpNullRefSentinel` — the shape a null arm has
/// AFTER this repair has run — because its caller (@PLN85's slice materialisation) runs later
/// in the pipeline and has to recognise the repaired form.  This one runs BEFORE the repair
/// and must match only unrepaired nulls; teaching it the sentinel would make it re-repair its
/// own output.  Two predicates, two lifecycle stages, measured to disagree exactly once over
/// the 858-program corpus — and that one disagreement is the nested-block tail `arm_is_null`
/// recurses into and this deliberately does not.
///
/// It also does not peel the TOP, unlike `arm_is_null`.  Measured before leaving it that way:
/// 4323 spanned values arrive here over the corpus and peeling changes the answer **0** times,
/// because a spanned arm is never a null arm — the same structural fact that keeps
/// `Return`/`Break`/`Continue` unspanned in block-operator position.
/// The null-arm recogniser of @FR-N-Match: which arm of a `match` on `τ?` is the `null` arm,
/// so the other arms bind the `τ`.
fn arm_body_is_null(code: &Value) -> bool {
    match code {
        Value::Null => true,
        Value::Block(bl) => bl
            .operators
            .last()
            .is_some_and(|o| matches!(o.unspan(), Value::Null)),
        _ => false,
    }
}

/// Put `typed_null` where [`arm_body_is_null`] found the null, in place.
///
/// A block keeps its block — only its LAST operator is replaced, and its result type is
/// restated, so the arm still delivers through the same slot the merge reads.
fn set_arm_null_typed(code: &mut Value, typed_null: &Value, result_type: &Type) {
    match code {
        Value::Block(bl) => {
            let last = bl.operators.len() - 1;
            bl.operators[last] = typed_null.clone();
            bl.result = result_type.clone();
        }
        other => *other = typed_null.clone(),
    }
}

fn chain_pattern_arms(arms: Vec<PatternArm>, fallback: Value, result_type: &Type) -> Value {
    let mut chain = fallback;
    for arm in arms.into_iter().rev() {
        let Some(guard) = arm.guard else {
            chain = match arm.cond {
                Some(cond) => v_if(cond, arm.code, chain),
                None => arm.code,
            };
            continue;
        };
        let rest = chain.clone();
        let mut guarded = v_if(guard, arm.code, rest);
        if !arm.bindings.is_empty() {
            let mut stmts = arm.bindings;
            stmts.push(guarded);
            guarded = v_block(stmts, result_type.clone(), "match_arm");
        }
        chain = match arm.cond {
            Some(cond) => v_if(cond, guarded, chain),
            None => guarded,
        };
    }
    chain
}

/// Returns true if the given AST value definitely returns on all code paths.
/// A block definitely-returns if its last statement is a `return`, or if it is
/// an `if` with an `else` where both branches definitely-return (recursive).
pub(crate) fn definitely_returns(val: &Value) -> bool {
    match val.tail() {
        Value::Return(_) => true,
        Value::If(_, t_branch, f_branch) => {
            // Both branches must definitely-return, and the else must not be null.
            !matches!(**f_branch, Value::Null)
                && definitely_returns(t_branch)
                && definitely_returns(f_branch)
        }
        _ => false,
    }
}

/// Match-arm type unification — strip `Type::RefVar(…)` wrappers before
/// delegating to `Type::is_same`.  Struct-enum pattern bindings yield a
/// `&T` borrow (e.g. `JString { value } => value` has type `&text`), while
/// sibling arms commonly return an owned `T` (`_ => ""`).  Requiring the
/// owned/borrow distinction to match exactly makes the straightforward
/// null-on-mismatch extractor pattern a compile error for no semantic
/// gain — the caller reads the value regardless of ownership.
fn match_arm_types_unify(a: &Type, b: &Type) -> bool {
    let strip = |t: &Type| -> Type {
        match t {
            Type::RefVar(inner) => (**inner).clone(),
            _ => t.clone(),
        }
    };
    strip(a).is_same(&strip(b))
}

impl Parser {
    /// Consume the `=>` separator that follows a match-arm pattern.
    ///
    /// If the user wrote `->` instead (a common slip — `->` is the lambda
    /// return-arrow and the historical TUPLES.md design draft used `->`
    /// for arms), emit a precise diagnostic and consume the wrong arrow
    /// so the arm-loop can continue and parse the body.  Without this
    /// recovery the surrounding loop spins on the unconsumed token —
    /// see PROBLEMS.md P206.
    fn expect_match_arm_arrow(&mut self) {
        // @PLN46/@PLN25 — `expr_not_null` is a TRANSIENT marker ("the field access just
        // parsed is non-null"), consumed by the very next operator.  A PATTERN reads the
        // subject's fields to bind its captures, so the last capture parsed leaves the
        // marker set with nothing in the pattern to consume it — and the arm BODY then
        // inherits it.  The first `??` in the body reports that stale name: an arm body
        // `(p ?? 0) + (q ?? 0)` under `[(A { p } B { q } |A { p } C { r })]` warned
        // "'r' is 'not null'", naming a capture the body never mentions, and on the
        // `(p ?? 0) + (r ?? 0)` shape it told the author to delete the `r ?? 0` that is
        // doing the work (dropping it yields null).  The `=>` is the pattern→body
        // boundary every arm passes through, so reset here — the same reset the
        // statement boundary already does (parse_block, "leak into a LATER statement").
        self.expr_not_null = false;
        self.expr_not_null_name.clear();
        // Trace point: match arm-arrow consumption.  Captures whether
        // the parser is looking at `->` (wrong), `=>` (right), or
        // something else (recover via `recover_to`).  Recurring
        // vantage during match-pattern debugging (P206, plan-18).
        // Enable with `LOFT_TRACE=match`.
        crate::loft_trace!(
            match_arm,
            "expect arrow: peek_arrow={} peek_eq={} first_pass={}",
            self.lexer.peek_token("->"),
            self.lexer.peek_token("=>"),
            self.first_pass,
        );
        if self.lexer.peek_token("->") {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "match arm separator is `=>`, not `->`"
                );
            }
            self.lexer.has_token("->");
        } else if !self.lexer.has_token("=>") {
            // P206 + plan-18: emit the missing-arrow diagnostic, then
            // recover to the next arm boundary.  Without recovery, a
            // malformed pattern like `x @ 1 | x @ 2 => …` (where the
            // or-pattern loop's `parse_match_pattern` doesn't consume
            // `x @ N`) leaves the lexer parked on an unexpected token;
            // the surrounding scalar/tuple/enum match loop then
            // re-enters pattern parsing on the same unconsumed token
            // and spins (PROBLEMS.md plan-18 phase 01 finding).
            //
            // `token("=>")` already emitted "Expect token =>"; here we
            // skip ahead until a `,`, `}`, or `;` so the outer loop
            // can pick up the next arm or exit cleanly instead of
            // looping forever.
            if !self.first_pass {
                diagnostic!(self.lexer, Level::Error, "Expect token =>");
            }
            self.lexer.recover_to(&[",", "}", ";"]);
        }
    }

    // <block> ::= '}' | <expression> {';' <expression} '}'
    #[allow(clippy::too_many_lines)]
    /// Does the definition being parsed take its return type FROM ITS BODY?
    ///
    /// True for a lambda, and only for a lambda. A lambda declares no return type, so its
    /// `returned()` reads `Void` while the body is parsed — the same value a function
    /// DECLARED with no return type has, and the opposite meaning: a placeholder waiting to
    /// be filled rather than a decision that nothing comes back.
    ///
    /// The name is the test the parser already uses for "is this a lambda" (`n___lambda_N`,
    /// minted by `lambda_counter` in the same order on both passes).
    fn def_infers_its_return(&self) -> bool {
        self.context != u32::MAX
            && self
                .data
                .def(self.context)
                .name()
                .starts_with("n___lambda_")
    }

    pub(crate) fn parse_block(&mut self, context: &str, val: &mut Value, result: &Type) -> Type {
        // Cognitive complexity, charged here because `parse_block`'s `context` already names
        // the construct — one hook instead of one per parser entry point.  `match_arm` is
        // deliberately free: `parse_match` charges once for the whole construct, so a wide
        // flat dispatch stays cheap while nesting does not.
        let cc = match context {
            "if" | "for" | "while" => Some(1 + self.cc_nest),
            "else" => Some(1), // a plain `else` costs 1, with no nesting bonus
            "match_arm" => Some(0),
            _ => None,
        };
        if let Some(add) = cc {
            // Pass 2 only: the parser runs twice over every definition, so charging on both
            // doubles every score (`seq_ifs` read 16 for eight flat `if`s).  The nesting
            // depth still has to track on BOTH passes, or pass-1 bodies would be charged at
            // the wrong depth by a stale counter.
            if add > 0 && !self.first_pass {
                *self.complexity.entry(self.context).or_insert(0) += add;
            }
            self.cc_nest += 1;
            if !self.first_pass {
                let line = self.lexer.pos().line;
                let deepest = self.cc_deepest.entry(self.context).or_insert((0, 0));
                if self.cc_nest > deepest.0 {
                    *deepest = (self.cc_nest, line);
                }
            }
        }
        let cc_ret = self.parse_block_inner(context, val, result);
        if cc.is_some() {
            self.cc_nest -= 1;
        }
        cc_ret
    }

    fn parse_block_inner(&mut self, context: &str, val: &mut Value, result: &Type) -> Type {
        if let Value::Var(v) = val
            && let Type::Reference(r, _) = self.vars.tp(*v).clone()
            && context == "block"
        {
            // We actually scan a record here instead of a block of statements
            // — the LHS is a pre-typed struct variable and `{ ... }` is its
            // body.  Disambiguate between struct-body `{ field: val, ... }`
            // and block-expression `{ expr }` (e.g. `{ S { a: 3 } }`) by peeking
            // at the first two tokens after `{`:
            //   - `ident :`  — struct body (canonical form).
            //   - `ident ,`  — likely struct-body typo (missing colons); keep
            //                  the struct-body path so parse_object's failure
            //                  produces the historical "Expect token ;"
            //                  diagnostic at the first bare identifier (test
            //                  INC#30 locks this wording).
            //   - anything else (e.g. `ident =`, `ident {`, `[`, a literal) —
            //                  block expression; fall through.
            let link = self.lexer.link();
            self.lexer.token("{");
            let looks_like_struct_body = self.lexer.has_identifier().is_some()
                && ((self.lexer.peek_token(":") && !self.lexer.peek_token(":="))
                    || self.lexer.peek_token(","));
            self.lexer.revert(link);
            if looks_like_struct_body {
                self.parse_object(r, val);
                return Type::Reference(r, Deps::none());
            }
        }
        self.lexer.token("{");
        if self.lexer.has_token("}") {
            *val = v_block(Vec::new(), Type::Void, "empty block");
            return Type::Void;
        }
        let mut t = Type::Void;
        let mut l = Vec::new();
        let mut terminated: Option<&str> = None;
        // @PLN25/#585 guard-clause flow-narrowing: a `narrowed_non_null` push made INSIDE this
        // block (by a fall-through guard, below) holds only for the rest of THIS block, so record
        // the entry depth and restore it before returning. Without the restore the proof leaks to
        // sibling blocks / later functions (whose var slots collide), silently suppressing real
        // `(N-Store)` warnings.
        let nn_base = self.narrowed_non_null.len();
        // T1.7: track the start-position of the last expression for not-null diagnostics.
        let mut last_expr_peek = self.lexer.peek();
        loop {
            let line = self.lexer.pos().line;
            if line > self.line {
                if matches!(l.last(), Some(Value::Line(_))) {
                    l.pop();
                }
                l.push(Value::Line(line));
                self.line = line;
                // loft#665 piece 3 — publish where the compiler is, so an internal
                // panic points at the user's line instead of a compiler source file.
                crate::crash_report::note_compile_pos(self.lexer.pos());
            }
            if self.lexer.has_token(";") {
                continue;
            }
            if self.lexer.peek_token("}") {
                break;
            }
            // detect file-scope-only declarations inside a block and
            // emit a single clean diagnostic instead of cascading parse
            // errors like "Expect token =" + "Expect constants to be in
            // upper case".  `fn` is special-cased because `fn(args) {...}`
            // is also a lambda expression — only reject `fn <name>(...)`.
            let bad_kw: Option<&'static str> = if self.lexer.peek_token("struct") {
                Some("struct")
            } else if self.lexer.peek_token("enum") {
                Some("enum")
            } else if self.lexer.peek_token("type") {
                Some("type")
            } else if self.lexer.peek_token("interface") {
                Some("interface")
            // @F47 — library imports / module system (use forms, pub)
            } else if self.lexer.peek_token("use") {
                Some("use")
            } else if self.lexer.peek_token("pub") {
                Some("pub")
            } else if self.lexer.peek_token("fn") {
                // distinguish `fn(args)` (lambda) from `fn name(args)`.
                let lexer_link = self.lexer.link();
                self.lexer.token("fn");
                let is_named_fn =
                    self.lexer.peek().has != crate::lexer::LexItem::Token("(".to_string());
                self.lexer.revert(lexer_link);
                if is_named_fn { Some("fn") } else { None }
            } else {
                None
            };
            if let Some(kw) = bad_kw {
                if !self.first_pass {
                    // About the token the parser is HOLDING, not about anything it has
                    // consumed: the offending keyword is still the current token here
                    // (`token(kw)` below is what consumes it).  So this site names its
                    // position rather than taking `report_pos`'s consumed-source default,
                    // which would put the caret on the line above.
                    let at = self.lexer.peek();
                    self.lexer.specific(
                        &at,
                        Level::Error,
                        &format!(
                            "'{kw}' definitions must be at file scope, not inside a function or block"
                        ),
                    );
                }
                // Consume the offending declaration: skip until the matching
                // top-level `}` or the outer block's `}`/`;`.
                self.lexer.token(kw);
                let mut depth: i32 = 0;
                while depth >= 0 {
                    if self.lexer.has_token("{") {
                        depth += 1;
                    } else if self.lexer.peek_token("}") {
                        if depth == 0 {
                            break;
                        }
                        self.lexer.token("}");
                        depth -= 1;
                    } else if self.lexer.peek().has == crate::lexer::LexItem::None {
                        break;
                    } else {
                        self.lexer.cont();
                    }
                }
                self.lexer.has_token(";");
                continue;
            }
            // Warn about unreachable code after an unconditional terminator.
            if let Some(kind) = terminated {
                if !self.first_pass {
                    // About the token the parser is HOLDING — the first token of the
                    // unreachable statement, which is what the caret should sit on.  So
                    // this site names its position rather than taking `report_pos`'s
                    // consumed-source default, which would point at the terminator above.
                    let at = self.lexer.peek().position;
                    self.lexer.pos_diagnostic_coded(
                        Level::Warning,
                        &at,
                        "unreachable-code",
                        &format!("Unreachable code after {kind}"),
                    );
                    self.lexer.fix_last(crate::diagnostics::Fix {
                        kind: crate::diagnostics::FixKind::Mechanical,
                        title: "delete the unreachable statements".to_string(),
                        condition: None,
                        edit: None,
                        concept: "functions",
                        concept_ref: "@F16",
                    });
                }
                // Only warn once per terminator
                terminated = None;
            }
            let mut n = Value::Null;
            last_expr_peek = self.lexer.peek();
            // @PLN22 Phase 1 — hint the block's expected enum so a bare
            // value-position variant tail (`fn f() -> Light { Red }`, or an
            // `if c { Red } else { Green }` block) resolves against it.  SAVE and
            // RESTORE the prior hint rather than clearing to Unknown, so sibling
            // statements / if-branches under the same expected type each still see
            // it (clearing made only the FIRST branch of an `if`-return resolve).
            let saved_expected = self.expected.clone();
            // @PLN90 W8 — thread a VECTOR result type into the block's statements too (not just
            // enum tails), so an empty `[]` value-tail (`match e { _ => { [] } }`) is TYPED and
            // materialises a REAL empty vector instead of folding to `Void`. Without it the empty
            // arm collapses, desyncing single-line (a `Null` else) vs multi-line (a `return null`
            // fallthrough) delivery — a layout-sensitive fragility. Mirrors the enum-tail hint and
            // the explicit-`return []` type-threading (@P365). The fresh-vector-in-arm double
            // `OpFreeRef` this exposes is PRE-EXISTING (a non-empty `{ [7,8,9] }` arm has it too)
            // and POISON-idempotent — not introduced here.
            // loft#703 — a KEYED result threads for the same reason: `[K { … }]` infers
            // `vector<K>` wherever it stands, so a keyed tail has no other way to say
            // which container to build.
            // @PLN124/loft#837 — an INTERPOLATION TARGET threads for the third time for
            // the same reason: `fn q(name: text) -> Query { "hi {name}" }` has no other
            // way to say which type the string builds, and LOFT.md § "Building a value
            // instead of text" lists the return type beside the assignment and the
            // parameter.  `interpolation_target` is a pure lookup (Reference → struct →
            // defines `lit`), so widening the gate costs a def-table probe on block tails
            // whose result is a struct.
            // loft#1067 — a `fn(…)` result threads for the FOURTH time for the same
            // reason: `fn make() -> fn(integer) -> integer { |x| { x * 2 } }` has no
            // other way to say what `x` is, and the return type is as much an expected
            // type as a parameter is.
            // loft#1122 — a TUPLE result threads for the fifth, and it is the entry that
            // says this list wants a rule rather than a fifth `||`: a member whose parse
            // needs the expected type (a bare variant, an empty collection literal) had
            // nothing to resolve against here, while a DECLARED LOCAL of the same type
            // accepted it — that position reads its destination from `var_tp` and never
            // needed the channel.  `seeds_tuple_hint` is the one home for the tuple
            // question, asked at the argument sites too; the general census of this
            // channel's ten push sites and six admission lists is QUALITY.md § B6t.
            // loft#1130 — and `yield` is the SAME question about the same declared type,
            // so the list is one home both spellings read (`seed_leaving_value_hint`).
            self.seed_leaving_value_hint(result);
            // @PLN46/@PLN25: `expr_not_null` is a TRANSIENT marker — "the field access just
            // parsed is non-null" — used by the very next operator (`p.field ?? d`'s defended
            // read, the redundant-null-check on `p.field == null`). A field access that ENDS a
            // statement (`m.value = 20;`) leaves it set with nothing to consume it, so it would
            // leak into a LATER statement's null-check and mis-fire "redundant null check" on an
            // unrelated operand (surfaced by F2: `m.value`/`cur.value` became non-null, so
            // `while cur != null` wrongly warned "always true" naming the stale 'value'). Reset
            // it at each statement boundary; the within-statement `?? d` / `== null` tracking is
            // untouched (both operand and consumer parse inside this one `self.expression`).
            self.expr_not_null = false;
            // loft#1382 — statement position, decided by the ONE reader that knows it.  A
            // statement beginning with `if` or `match` has its value discarded
            // (`@FR-F-Block`), so its arms need not agree with each other; a value-position
            // one never starts its statement (`v = if …` starts with `v`), which is what
            // makes a single peek sufficient.  `parse_if` consumes the flag, so a nested
            // value-`if` inside a statement one does not inherit it.
            let saved_stmt_if = self.stmt_if_pending;
            self.stmt_if_pending = self.lexer.peek_token("if") || self.lexer.peek_token("match");
            let pending_before = self.pending_arm_mismatch.take();
            t = self.expression(&mut n);
            self.stmt_if_pending = saved_stmt_if;
            // …and CONFIRM it here, where the `;` is finally visible.  `@FR-F-Block` discards
            // a block's value *"only where the BLOCK itself is a statement — a `;`-terminated
            // one"*, and a leading `if` does not prove that: a function TAIL also begins its
            // statement, and there the value is the function's, so its arms must still agree
            // (`fn t() { if c { 2 } else { "a" } }` is a real error, `parse_errors::wrong_if`).
            // The gate below therefore RECORDS the mismatch instead of reporting it, and this
            // is where it is either dropped — the construct was a statement — or reported.
            // Deciding here rather than by looking ahead is what keeps the lexer untouched:
            // a scan to the end of the construct has to re-lex it, and reverting that left
            // the parser mis-positioned on 250 tests.
            if let Some(m) = self.pending_arm_mismatch.take()
                && !self.lexer.peek_token(";")
            {
                self.arm_mismatch_report(&m);
            }
            self.pending_arm_mismatch = pending_before;
            self.expected = saved_expected;
            // Track unconditional terminators at block scope.
            // if/else/loop/match contain terminators inside branches — not unconditional.
            match &n {
                Value::Return(_) => terminated = Some("return"),
                Value::Break(_) => terminated = Some("break"),
                Value::Continue(_) => terminated = Some("continue"),
                _ => {}
            }
            // @PLN25/#585 guard-clause flow-narrowing: after `if <null-test> { <unconditional
            // exit> }` WITHOUT an else, the null case has already left the block, so the
            // fall-through proves the tested var non-null for the rest of THIS block — the same
            // fact `if v { … }` establishes for its THEN branch, applied to the fall-through.
            // Conservative: only a SIMPLE negated-nullness test of one name (`!v` / `v == null`,
            // i.e. `narrowing_from_condition` proving `v` non-null on the ELSE side) and only an
            // UNCONDITIONALLY divergent body with no else. A body that can fall through, a
            // `v != null` / truthy `v` guard (fall-through is the NULL case), or a compound
            // condition all correctly decline to narrow (they still warn).
            if let Value::If(test, true_code, false_code) = n.unspan()
                && matches!(false_code.unspan(), Value::Null)
                && let Value::Block(bl) = true_code.unspan()
                && is_block_divergent(&bl.operators)
                && let Some((v, false)) = self.narrowing_from_condition(test)
                && !self.narrowed_non_null.contains(&v)
            {
                self.narrowed_non_null.push(v);
            }
            if let Value::Insert(ls) = n {
                Self::move_insert_elements(&mut l, ls);
                // preserve `Type::Rewritten(_)` when flattening an
                // Insert.  A first-pass `parse_object` struct literal
                // returns `Type::Rewritten(Type::Reference(_))` together
                // with a Value::Insert body that has no terminating
                // Var — the Rewritten tag is the only signal that a
                // value of that type is produced.  Blindly resetting
                // `t = Void` here caused `x = { S { a: 3 } }` to infer
                // `x: void` in first_pass, which then tripped the
                // "cannot change type from void to S" diagnostic in
                // second_pass when the real Reference(S) type arrived.
                if !matches!(t, Type::Rewritten(_)) {
                    t = Type::Void;
                }
            } else if !matches!(t, Type::Void | Type::Never)
                && (self.lexer.peek_token(";") || *result == Type::Void)
            {
                l.push(Value::Drop(Box::new(n)));
                // A DROPPED value is not the block's value.  Every statement but the LAST
                // reaches the `t = Type::Void` at the foot of this loop, so only a tail
                // could keep a type here — and a tail is dropped exactly when the enclosing
                // definition returns nothing, which is the one case where the block's type
                // and the signature must agree and did not.  `--interpret` ignored the
                // mismatch and ran; `--native` takes the signature from the DECLARED return
                // (void, right) and the body's trailing value from the block's INFERRED type,
                // so it emitted `0 as u8` inside a `()` function and rustc refused the
                // generated file with a bare `E0308` (loft#1075).  On the interpreter the
                // same block type kept a dropped struct-literal tail alive to program exit
                // ("1 stores not freed").  `fn main() { f(); }` — the same IR down to the
                // `drop` — always worked on both, because the `;` sent the statement round
                // the loop to that reset.
                //
                // Narrow, because `result == Void` reaches here meaning two different
                // things and only one of them is a decision:
                //
                //   * a FUNCTION BODY (`context == "return from block"`) of a definition
                //     declared with no return type — the decision, and the case above;
                //   * a placeholder that something else will fill in.  A LAMBDA declares no
                //     return type either, so its body carries the same Void while its
                //     return type is INFERRED from this very `t`; and a `{ … }` in
                //     statement position is parsed against Void even when it is the TAIL of
                //     an enclosing block, which is the value that block yields.
                //
                // Both placeholders were measured, not reasoned about: zeroing `t` for a
                // lambda gave every stored short `|x| { … }` a void return (`parse_map`
                // then refuses it with D-clo-2's "cannot infer the type of the function
                // passed to `map`"), and zeroing it for a nested block made
                // `x = {{ …; count }}` infer void.
                if context == "return from block" && !self.def_infers_its_return() {
                    t = Type::Void;
                }
            } else {
                l.push(n);
            }
            if self.lexer.peek_token("}") {
                break;
            }
            // Preserve Never for blocks that end with return/break/continue.
            if !matches!(t, Type::Never) {
                t = Type::Void;
            }
            match l.last() {
                Some(
                    Value::If(_, _, _) | Value::Loop(_) | Value::Block(_) | Value::Parallel(_),
                ) => (),
                _ => {
                    if !self.lexer.token(";") {
                        // L1: recover to the next statement boundary or the
                        // block end so a missing `;` doesn't cascade into
                        // "Expect token }", "Expect constants to be in upper
                        // case style", etc. on the following lines.
                        if self.lexer.recover_to(&[";", "}"]) {
                            self.lexer.has_token(";");
                            continue;
                        }
                        break;
                    }
                }
            }
        }
        self.lexer.token("}");
        if matches!(l.last(), Some(Value::Line(_))) {
            l.pop();
        }
        // A block that YIELDS a value must not have dropped its own tail.
        //
        // Every `{ … }` reaching `expression` is parsed against `Type::Void` — it is a
        // STATEMENT block as far as that site can tell — so its tail is wrapped in a `Drop`.
        // That is right for a statement, and wrong for a block whose value someone reads:
        // `fn f() -> integer { { 5 } }` answered `null` on `--interpret` and `0` on
        // `--native`, silently, and `fn g() -> integer { n = 5; { n } }` answered `5` on one
        // backend and `0` on the other. The TYPE flowed out correctly the whole time — `t`
        // is the tail's type, which is how the function type-checked — and only the value
        // was thrown away.
        //
        // Undone here rather than prevented at the parse, because this is the first point
        // that knows which statement turned out to be the LAST one, and because the expected
        // type the parse site would need is not threaded for a plain scalar (the wider
        // question of loft#942/#943). The rule is local and total: a block typed as a value
        // ends in that value; whether anyone WANTS it is the enclosing level's question, and
        // the enclosing level wraps this whole block in a `Drop` of its own when the answer
        // is no.
        //
        // `t` is `Void` for a tail that was legitimately dropped — the `;` form, whose reset
        // runs at the foot of the loop, and the body of a function declared with no return
        // type (`formal/calls.md` `(F-Drop)`) — so both are untouched.
        //
        // Restricted to a bare `{ … }`, which is the only context that reaches here with a
        // `Void` it did not mean.  A `for` / `while` / `parallel for` / `fields` body is
        // handed `Void` because it IS a statement — its tail cannot be anyone's value — and
        // undoing the drop there leaks whatever the tail returned: measured, `for i in 0..8
        // { dcr_make(i) }` leaked one store per round, which is loft#725 reopened.
        if context == "block"
            && !matches!(t, Type::Void | Type::Never)
            && let Some(tail @ Value::Drop(_)) = l.last_mut()
            && let Value::Drop(inner) = std::mem::replace(tail, Value::Null)
        {
            *tail = *inner;
        }
        if matches!(t, Type::RefVar(_)) {
            let mut code = l.pop().unwrap().clone();
            self.un_ref(&mut t, &mut code);
            l.push(code);
        }
        // T1.7: check for null assigned to `integer not null` tuple elements in the
        // last expression of the block (the implicit return value).
        // After emitting the error, update the type to remove Null elements so that
        // type-conversion validation does not produce a redundant type-mismatch error.
        if !self.first_pass
            && !l.is_empty()
            && let Type::Tuple(expected) = result
            && let Type::Tuple(t_elems) = &t
        {
            let expected = expected.clone();
            let t_elems = t_elems.clone();
            let mut fixed = false;
            let new_elems: Vec<Type> = t_elems
                .iter()
                .zip(expected.iter())
                .map(|(te, ex)| {
                    if matches!(te, Type::Null)
                        && matches!(ex, Type::Integer(IntegerSpec { not_null: true, .. }))
                    {
                        fixed = true;
                        ex.clone()
                    } else {
                        te.clone()
                    }
                })
                .collect();
            if fixed && let Some(Value::Tuple(elems)) = l.last_mut() {
                let expected = expected.clone();
                for (elem_val, elem_tp) in elems.iter_mut().zip(expected.iter()) {
                    if matches!(elem_val, Value::Null)
                        && matches!(elem_tp, Type::Integer(IntegerSpec { not_null: true, .. }))
                    {
                        specific!(
                            &mut self.lexer,
                            &last_expr_peek,
                            Level::Error,
                            "cannot assign null to 'integer not null' element"
                        );
                        *elem_val = Value::Call(self.data.def_nr("OpConvIntFromNull"), vec![]);
                    }
                }
                t = Type::Tuple(new_elems);
            }
        }
        // Plan-07 phase 4d.2 — defensive-check flow-analysis.  When
        // a fault-prone op's result is assigned to a variable AND
        // the immediately-following sibling is an `if` whose
        // condition mentions that variable, the user has written
        // defensive code (`if x != null { … }`, `if x { … }`,
        // `if x > 10 { … }`, etc.) — swap the source op to its
        // Nullable peer at COMPILE TIME so neither runtime path
        // (production log + continue, OR development halt + render)
        // fires.  Both modes get the same silent-sentinel behaviour
        // because the Nullable peer never calls `s.raise`.
        //
        // Both `if x != null` and bare `if x` (truthy check) are
        // accepted; loft's `if x` lowers to a Reference→Boolean
        // conversion that's `false` for null DbRef / 0 / null int —
        // exactly the defensive shape we want to honor.  An over-
        // broad test like `if x > 10` also counts: any mention of
        // `Var(x)` in the if condition signals defensive intent.
        //
        // Single-block, single-step lookahead: covers the canonical
        // pattern.  Cross-function defenses or many-statement gaps
        // fall through to the raising peer + log path; phase 4e's
        // compile-time warning will nudge those toward the
        // recognised defenses.
        if !self.first_pass {
            self.rewrite_defended_fault_sites(&mut l);
        }
        // @PLN85 text-return analysis framework (SHADOW) — classify the raw tail
        // and print the verdict, WITHOUT changing codegen.  Verified beside the
        // tests via the corpus (framework/corpus.loft).  Fires on the codegen
        // pass for a text/`text?` return tail.
        if !self.first_pass
            && context == "return from block"
            && matches!(result.base(), Type::Text(_) | Type::Tuple(_))
            && let Ok(path) = std::env::var("LOFT_TRA_DUMP")
            && let Some(tail) = l.last()
        {
            let verdict = self.classify_text_return(tail, &l);
            let fname = self.data.def(self.context).original_name();
            // Append to the dump FILE (open+write+close per line, flushed on
            // drop) — a deterministic channel: loft's `eprintln!` stderr races
            // with `process::exit` and truncates unreliably.
            use std::io::Write;
            if let Ok(mut f) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                let _ = writeln!(f, "TRA {fname} => {}", verdict.label());
            }
        }
        // @PLN85 attempt 2d (text-tail-return-leak.md) — a text-returning fn whose
        // TAIL is a bare NATIVE text-dest CALL delivers a fresh owned text with no
        // promotable var, so it falls to the leaking owned-text `__ret_N` copy (and
        // the `-> text?` UAF).  Bind it to a synthetic local FLAT — two SIBLING ops
        // `Set(__tret, call); Var(__tret)` — so `block_result`'s existing var-tail
        // `text_return` promotes `__tret` to a hidden `&text` caller buffer and the
        // call dest-passes into it (no owned-text copy), matching the proven-clean
        // `t = call(); t` rebind (every native text-dest fn 0-leak).
        //
        // Driven by the analysis framework's verdict (`classify_text_return`):
        // one selector replaces the stacked per-shape predicates (native / view
        // of a local composite / user call / if-match owned arm).
        //
        // Fires in BOTH passes so the hidden `&text` buffer lands in the fn
        // SIGNATURE on pass 1 — every call site, INCLUDING a forward reference
        // compiled earlier in pass 2, then sees the promoted ABI.  (Pass-2-only
        // promotion was the "Too few parameters on n_<fn>" regression class: the
        // buffer was added after a forward-ref caller had already emitted its
        // call.)  On pass 2 the `__tret` name is already an attribute, so
        // `text_return`'s `Attr` arm re-applies `RefVar` to the var — the
        // double-classify 2d avoided by staying pass-2-only.
        // Exclude the debugger/REPL's throwaway `replmain_*` eval fns: the
        // frame-reenter eval path (`eval_frame_reenter`) expects an OWNED-text
        // return (it reads the value straight back / serialises with `.to_json()`
        // that survives frame teardown, @P293), NOT a hidden `&text` buffer — so
        // promoting one leaves the eval caller passing no buffer and the callee
        // reading an undefined arg slot (SIGSEGV in `rpc_eval_*`).  A run-once
        // eval fn gains nothing from the leak-opt anyway.
        let do_tret_bind = context == "return from block"
            && matches!(result.base(), Type::Text(_))
            && !self
                .data
                .def(self.context)
                .original_name()
                .starts_with("replmain_")
            && l.last().is_some_and(|tail| self.tret_bind_ok(tail, &l))
            // Pass-stability gate.  `do_tret_bind` promotes `__tret` to a hidden
            // `&text` SIGNATURE buffer, so it MUST fire IDENTICALLY on both passes
            // — a pass-2-only promotion grows the ABI after a forward-ref caller
            // already emitted its call, crashing it ("Too few parameters",
            // codegen.rs:272 / the H5 two-pass-contract assert).  `classify_text_
            // return` is not pass-stable for every tail: a fn-ref call (`f(x)` →
            // `CallRef` only after pass-2 lowering) and a local vector INDEX
            // (`tv[0]` → `OpGetText`-of-local only after pass-2 lowering) read as
            // `Plain`/`Borrow` on pass 1 but `Owned(FnRefCall)`/`Owned(ViewOfLocal)`
            // on pass 2 — whereas a field view (`res.name`) is `OpGetText`-of-local
            // on BOTH passes and stays stable.  Rather than enumerate which tail
            // shapes lower stably, make pass 2 FOLLOW pass 1: promote on pass 2
            // only if pass 1 already minted the `__tret` attribute.  (Pass 1 always
            // evaluates the verdict directly; the attr it mints persists into pass
            // 2, so a genuinely pass-stable tail like `res.name` still promotes on
            // both passes.)
            && (self.first_pass
                || self.def_has_tret_attr()
                || self.force_tret.contains(&self.context)
                || self.lambda_text_buffer_var().is_some());
        if do_tret_bind {
            let tv = self.create_unique("__tret", &Type::Text(Deps::none()));
            if tv != u16::MAX {
                let last = l.len() - 1;
                // The same move as the accumulator below: a lambda that already holds its
                // buffer keeps `__tret` a local and delivers through the buffer.
                let ret = match self.lambda_text_buffer_var() {
                    Some(buf) if buf != tv => buf,
                    _ => tv,
                };
                if matches!(l[last].unspan(), Value::Return(_)) {
                    // explicit `return <call>` → `Set(__tret, call); return __tret`
                    let call =
                        std::mem::replace(&mut l[last], Value::Return(Box::new(Value::Var(ret))));
                    l.insert(last, crate::data::v_set(tv, Self::peel_to_inner_call(call)));
                } else {
                    // bare `<call>` tail → `Set(__tret, call); __tret`
                    let call = std::mem::replace(&mut l[last], Value::Var(ret));
                    l.insert(last, crate::data::v_set(tv, call));
                }
                if ret != tv {
                    let last = l.len() - 1;
                    l.insert(last, crate::data::v_set(ret, Value::Var(tv)));
                }
                t = Type::Text(Deps::frame1(ret));
            }
        }
        // @PLN85 unified fix — a value-yielding `if`/`match` text tail (match
        // lowers to nested `If`).  Do NOT bind the whole `if` as one value
        // (`__tret = (if{a}else{b})`) — native rejects mixed arm Rust types
        // (E0308, 533/534).  Instead materialise PER-ARM into a text
        // accumulator: push `Set(__acc, <arm tail>)` into each arm and return
        // `__acc`.  Each arm becomes an independent buffer-write of uniform type
        // (native-safe) and `text_return` promotes `__acc` (BuiltLocal) to the
        // caller `&text` buffer (leak-free) — the proven `acc = ""; if c { acc =
        // … } else { acc = … }; acc` shape.  This is the text analogue of the
        // vector `materialize_vector_arms_into` per-arm delivery.
        //
        // Forward-ref-SAFE: an `if` is structurally an `If` on both passes.
        // Gated on BOTH arms YIELDING a text value (not a guard `if c { return
        // … }` — that would suppress the missing-return diagnostic).
        //
        // NULLABILITY DOES NOT GATE THIS PROMOTION — it is reported instead.
        //
        // The accumulator writes each arm into the caller's text buffer, and it retypes the
        // tail as the accumulator, so a `(N-Store)` check downstream then compares two
        // non-null types and stays silent.  Declining to promote is what used to keep that
        // diagnostic for a nullable tail in a `-> text` function, and it cost the program its
        // `--native` build: with no accumulator each arm stays `&*(callee(…))`, a borrow of
        // the `Str` temporary its callee returned and dead at the arm's `}` (E0716 — E0308
        // once the arms' Rust representations also disagree), while the interpreter ran it.
        //
        // `@FR-N-Store` does not leave that open: storing `e:τ?` into a non-null `τ` slot is a
        // WARNING that "compiles + runs, the slot holds null" wherever the null is
        // representable-and-distinct, which `text` is (types.md's per-type table); only the
        // narrow integer widths error, where the sentinel collides with a real value.  So the
        // store is reported below from the tail's OWN type, BEFORE the rewrite erases it, and
        // the promotion proceeds — a diagnostic describes the SOURCE program, and which
        // lowering the compiler picks for it cannot decide whether the program is diagnosed
        // (loft#1100, `calls.md` D-call-5).
        //
        // Both earlier keyings failed for the same reason: one tail type was answering two
        // questions.  Keying on the TAIL excluded a `-> text?` tail from per-arm delivery too,
        // so the whole `match` compiled as ONE Rust expression whose arms must unify — a
        // buffered call yields `Str` where a formatted string yields `&String` (loft#741,
        // E0308).  Keying on the DECLARED RETURN restored that and left this case refused.
        // Neither is needed once the report no longer rides on the gate.
        let nullable_into_nonnull =
            !matches!(result, Type::Optional(_)) && matches!(t, Type::Optional(_));
        let do_if_acc = !do_tret_bind
            && context == "return from block"
            && matches!(result.base(), Type::Text(_))
            && !self
                .data
                .def(self.context)
                .original_name()
                .starts_with("replmain_")
            && l.last().is_some_and(Self::if_tail_yields_text)
            // Pass-stability gate — the same one `do_tret_bind` carries above, for the same
            // reason and by the same means.  This promotion grows a hidden `&text`
            // SIGNATURE parameter (`text_return` promotes `__acc`), so a pass-2-only
            // verdict moves the ABI after pass 1 fixed it: `ownership.md`'s *"a verdict
            // that differed across passes would move the hidden buffer argument between
            // them"*, which the H5 two-pass contract turns into a fatal abort.
            //
            // The unstable term is the tail's INFERRED type just above.  Measured
            // (loft#1099): `fn f(k) -> text { a = "ab"; match k { -1 => null, _ => a } }`
            // reads `t = Optional(Text)` on pass 1 and `Text` on pass 2 from IR that is
            // byte-identical on both, so the accumulator was minted on pass 2 alone and
            // every such program aborted the compiler.  Rather than enumerate which tails
            // infer stably, make pass 2 FOLLOW pass 1: promote on pass 2 only where pass 1
            // already minted the `__acc` attribute.  Pass 1 evaluates the verdict directly
            // and the attribute it mints persists, so a stable tail still promotes twice.
            && (self.first_pass
                || self.def_has_acc_attr()
                || self.lambda_text_buffer_var().is_some());
        // `LOFT_DBG_ACC=1` — one line per pass per text-returning function, printing every
        // term of the gate above.  It is what located loft#1099: the two lines for one
        // function differ in `optOk` alone, which is how a non-pass-stable term shows
        // itself.  Read them in PAIRS; a single line says nothing about stability.
        if std::env::var_os("LOFT_DBG_ACC").is_some()
            && matches!(result.base(), Type::Text(_))
            && context == "return from block"
        {
            eprintln!(
                "[acc] fn={} pass1={} tret={} nstore={} yields={} acc_attr={} t={t:?} \
                 => {do_if_acc}",
                self.data.def(self.context).name(),
                self.first_pass,
                do_tret_bind,
                nullable_into_nonnull,
                l.last().is_some_and(Self::if_tail_yields_text),
                self.def_has_acc_attr(),
            );
        }
        if do_if_acc {
            // loft#1100 — report the nullable-into-non-null store from the tail's OWN type,
            // here, because the rewrite below retypes the tail as the accumulator (non-null
            // `Text`) and `block_result`'s `(N-Store)` check would then see two non-null
            // types and stay silent.
            //
            // Refusing the promotion is what USED to keep this diagnostic, and it cost the
            // program its `--native` build: each arm stayed a borrow of the `Str` temporary
            // its callee returned, which dies at the arm's `}` (E0716 — E0308 once the arms'
            // Rust types also disagree), while the interpreter ran it.  @FR-N-Store does not
            // leave that open: for a type whose null is representable-and-distinct it is a
            // WARNING that compiles and runs, so a backend REFUSING it is a deviation and
            // not the other half of a design choice.  A diagnostic describes the SOURCE
            // program; which lowering the compiler picks for it cannot decide whether the
            // program is diagnosed.
            if nullable_into_nonnull {
                let at = l
                    .last()
                    .and_then(Value::span_pos)
                    .cloned()
                    .or_else(|| Some(self.data.def(self.context).position().clone()));
                self.n_store_violation(&t, result, "the return value", at.as_ref());
            }
            // The accumulator IS the return value, so it carries the declared
            // return's nullability. Typing it non-null while a `-> text?` tail
            // writes nullable arms through it would leave the destination
            // lying about what it holds — and a nullable-into-non-null store
            // that no diagnostic fires on is exactly the hole this rewrite
            // must not open (loft#741).
            let acc_type = if matches!(result, Type::Optional(_)) {
                Type::Optional(Box::new(Type::Text(Deps::none())))
            } else {
                Type::Text(Deps::none())
            };
            // A lambda carries at most ONE hidden text buffer (`holds_text_work_buf`), and
            // `parse_return` may already have taken it before this tail asks, so `text_return`
            // declines to promote this accumulator (`SkipSecondTextBuf`) and it stays a
            // local.  That is the right outcome: the scope pass delivers a returned text
            // local through the buffer the lambda holds (`free_vars`, @FR-F-Ret), staged
            // where the value reads that buffer.  What must not happen is the gate above
            // declining to accumulate at all on pass 2 — that left `return cap ?? "x"` as a
            // view of its `??` temp, one orphaned `String` per call (loft#1357).  Writing
            // the arms straight into the buffer is not an option either: `return lloc ??
            // "x"` reads the very buffer `lloc` was promoted to.
            let av = self.create_unique("__acc", &acc_type);
            if av != u16::MAX {
                let last = l.len() - 1;
                let mut tail = std::mem::replace(&mut l[last], Value::Null);
                let is_ret = matches!(tail.unspan(), Value::Return(_));
                if let Value::Return(inner) = tail {
                    tail = *inner;
                }
                Self::push_text_arms_into(&mut tail, av, self.data.def_nr("OpCreateStack"));
                l[last] = tail;
                // loft#733 — introduce `av` at THIS scope, BEFORE the branch that
                // writes it. The per-arm `Set`s live INSIDE the branch, so without
                // this statement nothing introduces `av` here: codegen declares it
                // where it is first assigned — inside the `ncc` block — and the read
                // that follows the block is out of scope.
                //
                // Non-generic code hid this: `text_return` promotes `av` to the
                // hidden `&text` PARAMETER, so there is no local to misplace. A
                // generic MONOMORPH re-runs promotion in
                // `promote_monomorph_text_return`, which binds its own `__tret` and
                // promotes THAT — leaving `av` a block-local read from outside. The
                // interpreter then read an empty text (a wrong ANSWER, exit 0) and
                // native failed to compile: the accept/reject divergence in #733.
                //
                // The bind-site analogue (`push_text_arms_into`'s caller) already
                // documents this leading `Set` as load-bearing for exactly the same
                // reason; the tail promotion was missing it.
                l.insert(last, crate::data::v_set(av, Value::Text(String::new())));
                // A lambda already holding its one buffer cannot promote `av` to a second
                // (`SkipSecondTextBuf`), so `av` stays a local: move its bytes into that
                // buffer and return the buffer.  The local then takes its ordinary
                // scope-exit free.  Returning `av` itself handed up a view of a local the
                // frame never freed — one orphaned `String` per call (loft#1357).
                let ret = match self.lambda_text_buffer_var() {
                    Some(buf) if buf != av => {
                        l.push(crate::data::v_set(buf, Value::Var(av)));
                        buf
                    }
                    _ => av,
                };
                l.push(if is_ret {
                    Value::Return(Box::new(Value::Var(ret)))
                } else {
                    Value::Var(ret)
                });
                t = Type::Text(Deps::frame1(ret));
            }
        }
        // loft#1368 / `@FR-F-Ret` — a returned whole heap value is FRESH, never a view of a
        // parameter.  A function tail that is a value BRANCH hands back each arm's own borrow
        // (`fn pick(p, q, …) -> Node["p", "q"]?`), and a caller can witness only ONE of them,
        // so the arm that borrowed the OTHER source was adopted and a write through the result
        // reached the caller's argument — on both backends, silently.
        //
        // Bind the branch to a local FIRST, which is exactly the workaround the issue
        // documents (`r: Node = if first { p } else { q }; r`) and which works because a bind
        // COPIES each arm through its own temp (`@FR-B-Copy`, the join-arm lift of loft#1321).
        // Writing it here means every `-> S` / `-> S?` return gets it, not only the ones whose
        // author knew to.  The scope pass then sees an ordinary local return, which is the
        // shape its machinery is already correct for.
        // Not in a generic TEMPLATE: its body is cloned into each monomorph, and a local
        // minted here reaches codegen in the clone with no slot (`__ret_join[65535]`).  The
        // monomorph is parsed as an ordinary function and takes the rewrite there, which is
        // where the concrete return type is known anyway.
        let in_generic_template = self.context != u32::MAX
            && matches!(self.data.def(self.context).def_type(), DefType::Generic);
        if !self.first_pass
            && !in_generic_template
            && context == "return from block"
            && matches!(
                result.base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            )
            && self.tail_is_borrowing_branch(&l)
        {
            let last = l.len() - 1;
            let tmp = self.create_unique("__ret_join", result);
            self.vars.defined(tmp);
            let branch = std::mem::replace(&mut l[last], Value::Null);
            l[last] = crate::data::v_set(tmp, branch);
            l.push(Value::Var(tmp));
            t = result.clone();
        }
        t = self.block_result(context, result, &t, &mut l, &last_expr_peek.position);
        // @PLN25/#585: drop any guard-clause fall-through narrowing this block introduced — the
        // proof does not escape the block (see `nn_base` above).
        self.narrowed_non_null.truncate(nn_base);
        *val = v_block(l, t.clone(), "block");
        t
    }

    /// Find `Set(x, fault_op)` statements whose null a FOLLOWING null-check owns, and swap
    /// the fault op to its silent `*Nullable` peer.
    ///
    /// This is `(E-Report)`'s guarded half: *"a GUARDED site (the operand of `??` / a
    /// following null-check) emits the silent `*Nullable` op and reports NOTHING (the guard
    /// owns the null)"*.  The swap suppresses both the log entry and the development-mode
    /// halt, because the Nullable peers never call `s.raise`.
    ///
    /// The check need not be the very next statement — it needs only to come before anything
    /// else touches the variable.  That is the soundness condition, and it is what the scan
    /// below encodes: skip siblings that neither read nor write `x`, then require the first
    /// one that DOES mention it to be an `if` testing it.  A statement that reads `x` first
    /// has already consumed the null, so a check after it does not own it; a statement that
    /// REASSIGNS `x` means the check tests a different value, and `reads_var` counts a write
    /// target too, so both stop the scan at the same place.
    ///
    /// ⚠ Deliberately an UNDER-approximation.  Widening the notion of "guarded" SUPPRESSES a
    /// diagnostic, so a wrong answer here goes silent, while a missed one is merely noisy —
    /// the two failure directions are not equally bad and the predicate leans to the loud one.
    /// Known shapes it still reports on, both recorded as deviations in
    /// [`formal/operational.md`](../../doc/claude/formal/operational.md): a check with no
    /// intermediate binding (`if v[i] == null`), where the fault site is inside the test
    /// rather than before it; and a `match`, which lowers to a subject temp so the guard
    /// reaches the value through a COPY rather than by naming it.
    pub(crate) fn rewrite_defended_fault_sites(&self, ops: &mut [Value]) {
        // Two-pass to avoid borrow conflicts.  First pass: collect
        // the indices of statements that need rewriting.  Second
        // pass: apply rewrites.
        let mut to_rewrite: Vec<usize> = Vec::new();
        for i in 0..ops.len() {
            let Value::Set(var, _) = ops[i].unspan() else {
                continue;
            };
            let var = *var;
            // Skip siblings that do not mention the variable at all.
            let mut j = i + 1;
            while j < ops.len()
                && (matches!(ops[j].unspan(), Value::Line(_)) || !ops[j].reads_var(var))
            {
                j += 1;
            }
            if j >= ops.len() {
                continue;
            }
            // The first sibling that mentions it must be an `if` TESTING it — or a `match`,
            // which reaches the value through ONE copy and so needs following.
            match ops[j].unspan() {
                Value::If(test, _, _) if test.reads_var(var) => to_rewrite.push(i),
                Value::Block(bl) if Self::block_null_tests_a_copy(&bl.operators, var) => {
                    to_rewrite.push(i);
                }
                _ => {}
            }
        }
        for i in to_rewrite {
            if let Value::Set(_, source) = ops[i].unspan_mut() {
                Self::rewrite_outer_arith_to_nullable(source, &self.data);
            }
        }
        // The spelling with no binding at all — `if v[i] == null { … }`.  The loop above
        // keys on a `Set`, and here the fault site sits INSIDE the test rather than before
        // it, so there is nothing for it to hook and the site kept reporting a fault the
        // program had already defended (`D-op-5`).
        //
        // This needs no adjacency window and no dataflow: the guard is the SAME expression.
        for op in ops.iter_mut() {
            if let Value::If(test, _, _) = op.unspan_mut() {
                Self::rewrite_direct_null_test(test, &self.data);
            }
        }
    }

    /// Does this block copy `var` into a temp and then NULL-TEST that temp?
    ///
    /// `match x { null => … }` has no `Value::Match` in the IR: it lowers to a nested block
    /// whose first act is `_match_subj_N = x`, after which the arms test the TEMP. So the
    /// guard names a copy and the adjacency scan above, which looks for an `if` testing the
    /// variable itself, saw a `Block` and gave up — leaving a defended site reporting
    /// (`D-op-5`, the half the no-binding fix did not reach).
    ///
    /// Exactly ONE copy is followed, and only from the block's own first statement, so this
    /// stays adjacency rather than becoming dataflow.
    ///
    /// ⚠ The arm has to be a NULL test, not merely a test. `match x { 5 => … }` lowers to
    /// the same copy followed by `OpEqInt(subj, 5)` — the null flows into that comparison as
    /// an ordinary operand and on into the program, so that site still owes its report.
    fn block_null_tests_a_copy(ops: &[Value], var: u16) -> bool {
        let Some(Value::Set(tmp, source)) = ops.first().map(Value::unspan) else {
            return false;
        };
        if !matches!(source.unspan(), Value::Var(v) if *v == var) {
            return false;
        }
        let tmp = *tmp;
        ops.iter().skip(1).any(
            |op| matches!(op.unspan(), Value::If(test, _, _) if Self::is_null_test_of(test, tmp)),
        )
    }

    /// Is `test` a null check on `var` — in either spelling the IR uses?
    ///
    /// `x == null` reaches the IR as an equality against the null literal, and a `match`'s
    /// `null` arm as `OpNot(OpConvBoolFrom*(x))`. The second reads like a truthiness test and
    /// is not one: `op_conv_bool_from_long` is `val != i64::MIN`, so it asks precisely
    /// "is this not the null sentinel" and its negation is precisely "is this null".
    fn is_null_test_of(test: &Value, var: u16) -> bool {
        let Value::Call(_, args) = test.unspan() else {
            return false;
        };
        // `OpNot(OpConvBoolFrom*(var))`
        if args.len() == 1
            && let Value::Call(_, inner) = args[0].unspan()
            && inner.len() == 1
            && matches!(inner[0].unspan(), Value::Var(v) if *v == var)
        {
            return true;
        }
        // `var == null` / `null == var`, in either order.
        args.len() == 2
            && args
                .iter()
                .any(|a| matches!(a.unspan(), Value::Var(v) if *v == var))
            && args
                .iter()
                .any(|a| matches!(a.unspan(), Value::Call(_, inner) if inner.is_empty()))
    }

    /// Swap a fault-prone expression compared DIRECTLY against `null` to its silent
    /// Nullable peer, and answer whether one was found.
    ///
    /// `(E-Report)` gives the null to whatever guards it, and a test of the form
    /// `<expr> == null` guards `<expr>` completely: the null it produces is consumed by
    /// this comparison and nothing else can observe it. So the site owes no report.
    ///
    /// ⚠ Narrow deliberately, because widening "guarded" SUPPRESSES a diagnostic — an
    /// over-approximation goes quiet on real faults, while an under-approximation is only
    /// noisy. The test must be an equality against the null LITERAL (`OpConv*FromNull`,
    /// which is how a written `null` reaches the IR), so `if v[i] > 3` — which lowers to
    /// `OpLtInt(3, …)` with no such operand — cannot match it and still reports, as does a
    /// null that escapes to any other reader before its check.
    fn rewrite_direct_null_test(test: &mut Value, data: &crate::data::Data) -> bool {
        // `a || b` and `a && b` SHORT-CIRCUIT, so they reach the IR as a nested `if`
        // (`if a true else b`) rather than as a call — descend all three limbs, or a null
        // test in the second operand of an `||` is never seen.
        if let Value::If(cond, then, els) = test.unspan_mut() {
            let mut found = Self::rewrite_direct_null_test(cond, data);
            found |= Self::rewrite_direct_null_test(then, data);
            found |= Self::rewrite_direct_null_test(els, data);
            return found;
        }
        let Value::Call(def_nr, args) = test.unspan_mut() else {
            return false;
        };
        let name = data.def(*def_nr).original_name();
        // A condition is a tree, and the null test can sit anywhere in it —
        // `if v[i] == null || …` puts an `OpOr` on top.  Descend to FIND the comparison,
        // but rewrite only its DIRECT operand below, so a site whose null is consumed by
        // something else on the way (`f(v[i]) == null`, where `f` sees it) is not touched.
        if !(name.starts_with("Eq") || name.starts_with("Ne")) || args.len() != 2 {
            let mut found = false;
            for arg in args.iter_mut() {
                found |= Self::rewrite_direct_null_test(arg, data);
            }
            return found;
        }
        // A written `null` reaches the IR as a nullary `OpConv<Type>FromNull`, one per
        // scalar type — matched as the family so a new type joins without a list to update.
        let is_null_literal = |v: &Value| {
            matches!(v.unspan(), Value::Call(d, a) if a.is_empty() && {
                let n = data.def(*d).original_name();
                n.starts_with("Conv") && n.ends_with("FromNull")
            })
        };
        let guarded = if is_null_literal(&args[1]) {
            0
        } else if is_null_literal(&args[0]) {
            1
        } else {
            // An equality that is not against `null` guards nothing, but a null test may
            // still sit deeper inside its operands.
            let mut found = false;
            for arg in args.iter_mut() {
                found |= Self::rewrite_direct_null_test(arg, data);
            }
            return found;
        };
        Self::rewrite_outer_arith_to_nullable(&mut args[guarded], data);
        true
    }

    pub(crate) fn un_ref(&mut self, t: &mut Type, code: &mut Value) {
        if let Type::RefVar(tp) = t.clone() {
            self.convert(code, t, &tp);
            *t = *tp;
            for on in t.depend() {
                *t = t.depending(on);
            }
        }
    }

    pub(crate) fn move_insert_elements(l: &mut Vec<Value>, elms: Vec<Value>) {
        for el in elms {
            if let Value::Insert(ls) = el {
                Self::move_insert_elements(l, ls);
            } else {
                l.push(el);
            }
        }
    }

    /// Is this block's tail a value BRANCH handing back TWO OR MORE different parameters
    /// (loft#1368)?
    ///
    /// The shape no caller can witness: with one nameable source the caller's guarded bind
    /// copies correctly, and with two it can name only the first, so the arm that borrowed the
    /// other is adopted and the write lands on the caller's argument.
    ///
    /// Asked of the ARMS, not of the declared return deps.  A MONOMORPH's return type carries
    /// no deps at all — `t_4Node_pickg` returns a bare `ref(Node)?` where its concrete twin
    /// returns `Node["p", "q"]?` — so a deps test sees the generic instance as borrowing
    /// nothing and leaves exactly the shape a generic makes easiest to write unguarded.
    fn tail_is_borrowing_branch(&self, l: &[Value]) -> bool {
        let Some(tail) = l.last() else {
            return false;
        };
        self.is_borrowing_branch(tail)
    }

    /// [`Self::tail_is_borrowing_branch`] asked of one value.
    fn is_borrowing_branch(&self, tail: &Value) -> bool {
        fn arm_params(v: &Value, vars: &crate::variables::Function, out: &mut Vec<u16>) -> bool {
            match v.unspan() {
                Value::If(_, t, f) => {
                    let _ = arm_params(t, vars, out);
                    let _ = arm_params(f, vars, out);
                    true
                }
                Value::Block(bl) => bl
                    .operators
                    .last()
                    .is_some_and(|x| arm_params(x, vars, out)),
                Value::Var(x) => {
                    if vars.is_argument(*x) && !out.contains(x) {
                        out.push(*x);
                    }
                    false
                }
                _ => false,
            }
        }
        let mut params = Vec::new();
        arm_params(tail, &self.vars, &mut params) && params.len() > 1
    }

    pub(crate) fn block_result(
        &mut self,
        context: &str,
        result: &Type,
        t: &Type,
        l: &mut [Value],
        tail_pos: &Position,
    ) -> Type {
        // loft#945 — a short lambda whose return type nothing named takes it from its own
        // body, HERE: the tail's type is known for the first time, and the delivery
        // machinery below has not run yet.  Both halves matter.  Reading the type after
        // the body parsed instead would set it on pass 1 only after pass 1's body had
        // already been lowered as if it returned nothing, so the hidden text buffer
        // appeared on pass 2 alone — the H5 two-pass contract violation that turned an
        // honest `xs.map(|s| { "{s}!" })` into an internal compiler error.  Pass 2 finds
        // `returned` already set from pass 1, so the adoption is a pass-1-only event and
        // the two passes lower the same signature.
        let adopted;
        let result = if context == "return from block"
            && self.context != u32::MAX
            && self.infer_ret_defs.contains(&self.context)
            && matches!(self.data.def(self.context).returned(), Type::Void)
            && !t.is_unknown()
            && !matches!(t, Type::Void | Type::Never)
        {
            // `unrewritten` as well as `without_deps`: a struct-literal tail carries the
            // `Rewritten` marker that says it was built in place, and a marker is a fact
            // about the EXPRESSION, not a type a signature can name (loft#943).
            adopted = t.unrewritten().without_deps();
            self.data.definitions[self.context as usize].returned = adopted.clone();
            // Remembered past the body, where `infer_ret_defs` is cleared: the late
            // buffer reservation between the passes admits a lambda only if its return
            // came from here (see `Parser::adopted_ret_defs`).
            self.adopted_ret_defs.insert(self.context);
            &adopted
        } else {
            result
        };
        let mut tp = t.clone();
        // @PLN85 move-on-block-return (block-return-move.md): a block used as a
        // VALUE — NOT a function return (those flow through the delivery
        // classifier below) — whose tail views a struct LOCAL that is DEFINED
        // INSIDE this block must not escape as a borrow. Borrowing leaks the
        // local's store (the escaping return var is freed by nobody) and, via
        // slot reuse with the still-live consumer, corrupts (`a={z=P{1,2};z};
        // b={z=P{3,4};z}` → a reads b's values). Copy the view into an owned
        // work-ref (the proven #306 `materialize_view_return` path) so the
        // consumer ADOPTS its own store; the original local is then freed by the
        // normal block-scope sweep (it is no longer the escaping return var). A
        // tail that views an OUTER local (`a = { base }`, `base` defined before
        // the block) is a genuine borrow the consumer keeps — the block does not
        // define it, so `block_defines_var` is false and it is left alone.
        if !self.first_pass
            && context != "return from block"
            && let Type::Reference(td, ls) = t.base()
            && ls.iter().any(|&v| Self::block_defines_var(l, v))
        {
            let td = *td;
            let last = l.len() - 1;
            let w = self.materialize_view_value(td, &mut l[last]);
            return self.vars.tp(w).clone();
        }
        // #416 — set when the vector match/if tail below was materialised into the
        // return buffer; gates the type-keyed vector arm (which is reached only in
        // the IMPLICIT-tail `t = Vector` case) so it doesn't re-process / re-promote
        // an arm buffer the materialise already delivered.
        let mut vec_arm_handled = false;
        if *result != Type::Void && !matches!(*result, Type::Unknown(_)) {
            // An empty block (e.g. an empty comprehension body `[for i in r {}]`) has no
            // tail to convert/deliver; without this guard `l.len() - 1` underflows to
            // usize::MAX and the index below panics.  Leave the empty block to downstream
            // type-checking (which reports the real "expected <T>, produced nothing").
            if l.is_empty() {
                return tp;
            }
            let last = l.len() - 1;
            // CO1.3c: generator bodies return void (values come from yield), so the
            // void-vs-iterator mismatch is not an error.
            //
            // But "its tail is not the return value" is not the same as "do not look at its
            // tail", and this used to be written as the second.  A generator body whose tail
            // IS a value — `fn make() -> iterator<integer> { counting(1) }` — then sailed
            // through: the value was dropped, `--interpret` answered an EMPTY sequence with
            // no diagnostic, and `--native` panicked inside `alloc_coroutine`.  That is the
            // tail-expression spelling of the `return <expr>` this file refuses in
            // `parse_return`; refusing one and not the other would leave the rule asked at
            // one of its two construction sites, which is the shape loft#1006 already was.
            let is_generator = matches!(result, Type::Iterator(_, _));
            if is_generator && !self.first_pass && !matches!(*t, Type::Void | Type::Never) {
                let msg = "a generator's body produces values only through `yield`, so this \
                           tail value is discarded — `for v in <generator>() { yield v; }` \
                           forwards another generator's values, and a bare statement ends \
                           the body";
                // The tail is what the message is about, and a call already carries its own
                // span (wrapped at the `(` on pass 2).  Prefer it: the block-tail check can
                // only run once the block is closed, so the default lands on the `}`.
                if let Some(tail) = l[last].span_pos().cloned() {
                    self.lexer.pos_diagnostic(Level::Error, &tail, msg);
                } else {
                    diagnostic!(self.lexer, Level::Error, "{msg}");
                }
            }
            let ignore = is_generator
                || (matches!(*t, Type::Void | Type::Never)
                    && (matches!(l[last], Value::Return(_)) || definitely_returns(&l[last])));
            // Plan-14 phase 07 (P234 runtime): when the function's expected
            // return type is `Reference(__tuple<…>)` (rewritten in
            // `parse_function` for any tuple whose elements have lifetime
            // concerns) AND the body's tail expression is a literal
            // `Value::Tuple(elements)`, transform the tail into synthetic-
            // struct construction so the existing struct-return machinery
            // applies.  Without this rewrite, `convert` would fail
            // (Tuple is not assignable to Reference(__tuple<…>)) and the
            // user would see a confusing "expected __tuple<…>, got tuple([…])"
            // diagnostic.
            // A7.1: gate broadened to also fire for `If` / `Block` /
            // `Insert` tails — the recursive helper descends through
            // these wrappers and rewrites every leaf `Value::Tuple` that
            // lives at a tail position with a synthetic-struct
            // construction sharing one work-ref.  Without this, function
            // bodies whose final expression is `if cond { (a, b) } else
            // { (c, d) }` left two tuple leaves and convert would then
            // fail with Tuple → Reference(__tuple<…>).
            //
            // loft#1350 — the same boxing for an `else` ARM.  The else arm is parsed against
            // the then arm's type, and a then arm that CALLS a function returning a lifetime
            // tuple yields the synthetic struct; a literal or a tuple local on the other arm
            // is a stack tuple, and `convert` has no route from one spelling to the other —
            // "expected __tuple<vector<integer>,text>, got (vector<integer>, text) on else",
            // for a program that reads as one type to its author.  Box the arm into its own
            // work-ref, exactly as a function tail is, and let `parse_if` join two records.
            // Only a tuple whose element types spell the SAME synthetic name is boxed; a
            // different shape keeps the refusal, which is then about the elements.
            // The work-ref an `else` arm was boxed into (loft#1350) — the arm's own
            // ownership fact, read where the arm's type is settled below.
            let mut boxed_arm_w: Option<u16> = None;
            // A block handed a SIBLING EXPRESSION's type as its expected type: the `else`
            // arm (`parse_if` passes the then arm's), and the then arm of an `else if`
            // CHAIN (`parse_if_expecting` passes the enclosing then arm's; a top-level `if`
            // expects `Unknown`).  Every other caller's expected type is a DECLARED one.
            // The three carve-outs below are about the sibling, not about the keyword, so
            // they read this and not `context == "else"` (loft#1380).
            // A `match` ARM is an else arm too (loft#1380's twin): the destination checks the
            // whole construct, so each arm converts to the type its siblings answer in, with
            // the same carve-outs — the sibling-variant join, the statement-position discard,
            // the honest nullability.  Gated on a KNOWN expected type exactly as `if` is: the
            // first concrete arm has nothing to agree with and names the type instead.
            let arm_of_sibling = context == "else"
                || ((context == "if" || context == "match_arm") && !result.is_unknown());
            let tuple_rewritten = !self.first_pass
                && (context == "return from block" || arm_of_sibling)
                && matches!(t, Type::Tuple(_))
                && tail_has_tuple_leaf(l[last].unspan(), &self.vars)
                && matches!(result, Type::Reference(d, _) if self.data.def(*d).name().starts_with("__tuple<"))
                && {
                    let synthetic_d_nr = if let Type::Reference(d, _) = result {
                        *d
                    } else {
                        unreachable!()
                    };
                    if arm_of_sibling {
                        let same_shape = if let Type::Tuple(elems) = t {
                            let names: Vec<String> =
                                elems.iter().map(|e| e.name(&self.data)).collect();
                            format!("__tuple<{}>", names.join(","))
                                == self.data.def(synthetic_d_nr).name()
                        } else {
                            false
                        };
                        if same_shape {
                            let ref_tp = Type::Reference(synthetic_d_nr, Deps::none());
                            let w = self.vars.work_refs(&ref_tp, &mut self.lexer);
                            let kt = self.data.def(synthetic_d_nr).known_type();
                            self.rewrite_tail_tuple_with_work_ref(
                                synthetic_d_nr,
                                kt,
                                w,
                                &mut l[last],
                            );
                            boxed_arm_w = Some(w);
                        }
                        same_shape
                    } else {
                        self.rewrite_tail_tuple_to_synthetic_struct(synthetic_d_nr, &mut l[last]);
                        true
                    }
                };
            // P236: when the body's tail is a `Value::If(...)` (or
            // `match`, which lowers to nested `If`) and the function
            // returns a heap-owned reference, unify the branches'
            // work-refs so all paths share one return slot.  Without
            // this, native codegen drops the if/else's value and
            // returns the typed null sentinel.  See `unify_if_branches_work_refs`
            // for the full rationale.  Wrap in `Value::Return(...)` so
            // the existing `Return(If(...))` native codegen at
            // `src/generation/emit.rs:166-182` emits
            // `return if cond { ... } else { ... }` correctly.
            // `base`, because the question is whether the RETURN is heap-shaped, and a
            // nullable one is: `S?` holds the same record as `S` (@FR-L-Null, layout(τ) =
            // layout(τ?)).  Asked bare, every `τ?` tail kept a work-ref per arm plus a
            // separate result slot where its non-null twin shares one.
            let if_unified = !self.first_pass
                && context == "return from block"
                && crate::data::is_dbref(result.base())
                && matches!(l[last].unspan(), Value::If(_, _, _))
                && self.unify_if_branches_work_refs(&mut l[last]).is_some();
            if if_unified {
                let inner = std::mem::replace(&mut l[last], Value::Null);
                l[last] = Value::Return(Box::new(inner));
            }
            // @PLN85 cluster II — a VECTOR-returning fn whose tail is a `match`/`if`
            // with per-arm LOCAL buffers (arms are `_vec_N`, not the `__ref_N`
            // work-refs `if_unified` shares; the match types as `Never`, so the
            // type-keyed vector arm below — keyed on `t` — is skipped). Without
            // NRVO the result is delivered via a fresh local while the caller's
            // eagerly-allocated `__retbuf` work-ref store is orphaned and LEAKS on
            // the interpreter (Edge B / `init_ref`). Deliver per-arm into `__retbuf`.
            // Fires for an explicit `return match` (t = Never) AND an implicit
            // `{ match }` block tail (t = Vector — #416). `tail_terminal_is_branch`
            // keeps it to match/if tails. `tail_if_has_null_arm` EXCLUDES a nullable
            // return (`{ if b { [..] } else { null } }`): materialising it would set
            // `returned = Vector[__retbuf]` while a reachable arm yields null, which
            // native cannot represent. enc's exhaustive-match default-null is nested
            // and unreachable, so it is not a direct arm-null and still materialises.
            //
            // loft#938 — a return that PEELS (`-> vector<T>?` with the buffer ABI) is the
            // one case where a direct null arm is representable, so it drops the
            // exclusion: the delivery re-wraps the `?` (`returned = optional(vector[
            // __retbuf])`), the value arms yield the buffer and the null arm yields the
            // sentinel, which is what a nullable DbRef return already means on both
            // backends.  Non-nullable behaviour is untouched — `ret_promo_peels` is false
            // for it whatever the switch says.
            let vec_match_candidate = !tuple_rewritten
                && !if_unified
                && !self.first_pass
                && context == "return from block"
                && matches!(result.ret_promo_base(), Type::Vector(_, _))
                && matches!(
                    t.ret_promo_base(),
                    Type::Never | Type::Void | Type::Vector(_, _)
                )
                && Self::tail_terminal_is_branch(&l[last])
                // A DIRECT `null` arm was excluded outright (64bd0984, 2026-06-21) because
                // materialising set `returned = Vector[__retbuf]` on a path that yields
                // null; @PLN25 then let a DECLARED-nullable return through
                // (`ret_promo_peels`, which keeps the `?` around the buffer-dep'd base).
                // The remaining exclusion also suppressed the DELIVERY, and that is
                // loft#1098: at most ONE arm can BE the buffer (the promotion renames it),
                // so with TWO OR MORE non-null arms the rest are stores nobody delivers —
                // the callee's `OpFreeRefIfDistinct` sees the returned store and keeps it,
                // the caller's binding is typed as a borrow of ITS buffer and frees that
                // instead, and one store orphans per call. Measured over `-1 => null`
                // tails: one non-null arm is clean (the rename covers it), and two, three
                // or a local-plus-literal pair each leaked one per call.
                && (result.ret_promo_peels()
                    || !self.tail_if_has_null_arm(&l[last])
                    || self.tail_nonnull_arm_count(&l[last]) >= 2);
            if std::env::var_os("LOFT_DBG_VMC").is_some() && matches!(result, Type::Vector(_, _)) {
                eprintln!(
                    "[vmc] fn={} !tup={} !ifu={} !p1={} ctx={context:?} resV={} tOk={} \
                     branch={} !null={} peels={} nonnull={} => {vec_match_candidate}",
                    self.vars.name,
                    !tuple_rewritten,
                    !if_unified,
                    !self.first_pass,
                    matches!(result, Type::Vector(_, _)),
                    matches!(t, Type::Never | Type::Void | Type::Vector(_, _)),
                    Self::tail_terminal_is_branch(&l[last]),
                    !self.tail_if_has_null_arm(&l[last]),
                    result.ret_promo_peels(),
                    self.tail_nonnull_arm_count(&l[last]),
                );
            }
            if vec_match_candidate && let Type::Vector(elm, _) = result.ret_promo_base() {
                // #416 — a match/if branch tail materialises each arm into __retbuf.
                // Routed through the ONE vector dispatch (Delivery::Materialize); it
                // gates convert via vec_arm_handled on whether a rewritable arm was
                // found (no buffer / no terminal → false, convert runs as before).
                let elm_ty = (**elm).clone();
                vec_arm_handled = self.dispatch_vector_delivery(Delivery::Materialize, &elm_ty, l);
            }
            // (#448, the early-`return <call>` + tail-`return [literal]` shape, was a
            // SECOND upper materialise block here. It is now a CELL of the tail-return
            // handling below — `Delivery::Materialize` when the buffer is already
            // TAKEN by a sibling return — so it shares one fresh-owned-vector classifier
            // (`fresh_owned_vector_deps`) and one dispatch with the buffer-free #437/c5
            // rename. See the `tail_ret_owned` block.)
            // @PLN25 (N-Store): the IMPLICIT function-tail is a STORE into the
            // caller's non-null return slot, exactly like an explicit `return`.
            // Only at the genuine function tail (`context == "return from block"`),
            // not an `if`/`match` arm (whose `result` may legitimately be nullable).
            // A scalar `τ?` tail hits none of the vector/tuple special cases above,
            // and `convert` below peels `Optional`, so no double-diagnose.
            // A tail conversion is checked once the block is CLOSED, so `report_pos`
            // would attribute anything it raises to the `}`.  `tail_pos` is the tail
            // statement's own first token (captured per statement by the block loop), and
            // a bare-var tail carries no span of its own — measured: `fn f(n: integer) ->
            // i32 { n }` reported the narrowing on the closing brace while the same
            // narrowing in an assignment, an argument and a struct-literal field all
            // named their own line.  Seek for the duration and restore.
            // @FR-C-Var — an `else` arm that is a SIBLING variant of the then arm's is not
            // a conversion question at all.  `Reference(S) ⤳ Enum(E)` is licensed for each
            // of them and nothing is licensed BETWEEN them, so asking `convert` produced
            // *"expected A, got B on else"* for a join `match` accepts (loft#1117).  The
            // arm keeps its own type and `parse_if` joins the two to their enum.
            let sibling_variant = arm_of_sibling && self.arm_joins_to_enum(t, result);
            // @FR-F-Block — the arms of a construct in STATEMENT position yield nothing
            // anybody reads, so their types need not agree.  Only one ORDER used to compile:
            // a void THEN arm makes the expected type `void`, which accepts any else arm,
            // while a void ELSE arm arrived as a conversion `void ⤳ integer` that nothing
            // licenses — `{ println } else { 5 };` was accepted and its mirror refused
            // (loft#1382), on both backends, where `match` accepted both.
            //
            // Gated on POSITION, not on the arms' types alone: `Type::Void` on an arm is not
            // one fact — it is also what a block reports when its value travels through a
            // BUFFER — so keying on it dropped the retbuf delivery of twenty other tests.
            //
            // And only where one arm yields NOTHING.  The corpus pins
            // `if c { 2 } else { "a" };` as a refusal: two VALUES of different types is a
            // mistake worth reporting wherever it sits, and widening that is not what
            // loft#1382 asks.  A void arm beside a value arm is not a type mistake at all —
            // there is no value for the two to disagree about.
            let stmt_arm = arm_of_sibling
                && self.arms_of_statement_construct
                && (matches!(t, Type::Void) || matches!(result, Type::Void));
            let needs_convert = !tuple_rewritten
                && !if_unified
                && !vec_match_candidate
                && !vec_arm_handled
                && !sibling_variant;
            // @FR-N-Store — the tail is a STORE into the return slot.  The store face asks
            // where the tail converts; a tail that does not convert (a rewritten tuple, a
            // unified `if`, a vector match, a sibling variant) is asked here.  Anchored to
            // the tail's OWN span, because this runs at block FINALIZATION with `self.lexer`
            // on the `}` — reporting the NEXT function otherwise (nstore-position-fix.md);
            // a bare-var tail has no span and falls back to the enclosing function's.
            let is_return = context == "return from block";
            let ret_at = if is_return {
                l[last]
                    .span_pos()
                    .cloned()
                    .or_else(|| Some(self.data.def(self.context).position().clone()))
            } else {
                None
            };
            if is_return && !needs_convert {
                self.n_store_violation(t, result, "the return value", ret_at.as_ref());
            }
            let converted = if needs_convert {
                self.lexer.to((tail_pos.line, tail_pos.pos));
                let done = if is_return {
                    self.convert_store(&mut l[last], t, result, "the return value", ret_at.as_ref())
                } else {
                    // An arm meeting its sibling's type is the JOIN (`(N-Join)`: the `if` is
                    // `τ?` when an arm is), not a store — wherever the joined value lands is
                    // asked there.
                    self.convert_admitting(&mut l[last], t, result)
                };
                // `end_seek`, not a second `to`: the diagnostics raised just below are
                // block-tail checks too, and a seek left standing switches `report_pos`
                // off for them.
                self.lexer.end_seek();
                done
            } else {
                true
            };
            if needs_convert && !converted && !ignore {
                // for function bodies with `not null` return, downgrade to a warning.
                if context == "return from block"
                    && self.context != u32::MAX
                    && self.data.definitions[self.context as usize].returned_not_null
                {
                    if !self.first_pass {
                        let fn_name = self.data.definitions[self.context as usize].original_name();
                        diagnostic!(
                            self.lexer,
                            Level::Warning,
                            code = "missing-return-path",
                            "Not all code paths return a value — function '{fn_name}' may return null",
                        );
                        self.lexer.fix_last(crate::diagnostics::Fix {
                            kind: crate::diagnostics::FixKind::Conditional,
                            title: "return a value on every path".to_string(),
                            condition: Some("if returning null there is intended, declare the return type `T?` instead".to_string()),
                            edit: None,
                            concept: "functions",
                            concept_ref: "@F16",
                        });
                    }
                } else if stmt_arm {
                    // Deferred: the construct BEGAN its statement, but a function TAIL does
                    // that too and there the arms must still agree.  `parse_block`'s loop
                    // drops this when a `;` follows and reports it otherwise.  Recording the
                    // TYPES rather than a rendered string keeps `validate_convert`'s
                    // same-name-two-defs case (loft#1094) intact.
                    self.pending_arm_mismatch = Some(crate::parser::ArmMismatch {
                        test: t.clone(),
                        should: result.clone(),
                        context: context.to_string(),
                        at: tail_pos.clone(),
                    });
                } else {
                    self.validate_convert(context, t, result, tail_pos);
                }
            }
            // loft#978 — an `else` arm (and a chain's then arm, `arm_of_sibling`) is a block
            // handed a SIBLING EXPRESSION as its expected type, and an expected type
            // cannot say what the value in hand borrows: it was written before that value
            // existed.  Taking it whole republished the then-arm's dep list as the else
            // arm's, so a fresh-record then-arm erased the container view the else arm
            // actually delivered, the local read as owned, and scope exit freed the
            // container's record.  Every OTHER caller's expected type is a DECLARED one
            // (`return from block`, a `for` body), whose deps are attribute indices in a
            // different space entirely — grafting frame vars onto those is the
            // cross-space read loft#666 was made of, so the shape alone is taken there,
            // exactly as before.
            tp = if arm_of_sibling {
                // loft#1103 — the SHAPE comes from the expected type, but the arm's own
                // NULLABILITY does not: `(N-Join)` says a join is optional iff some arm is,
                // and this is the arm whose answer was being dropped.  `x: integer = if c
                // { 1 } else { maybe(k) }` took the then-arm's `integer` whole, so the join
                // typed non-null, the destination's `(N-Store)` teeth had nothing to bite,
                // and a declared `integer` held null with no diagnostic on either backend.
                //
                // The `null` LITERAL in the same position was always caught — by the DN1
                // walkers in `parse_if`, which match the `OpConv*FromNull` spelling.  One
                // notion with two spellings and only one of them asked about; this is the
                // other spelling, asked about here where the arm's own type is still in hand.
                // A sibling variant keeps its OWN shape: the expected type names the
                // then arm's variant, and taking it whole would report this arm as that
                // one — which is also what lets `parse_if` see that the two differ.
                let honest = if stmt_arm {
                    // loft#1382 — in STATEMENT position the arm keeps its OWN type.  Taking
                    // the then arm's would tell the native emitter both arms are non-void,
                    // and its `(F-Block)` discard gate (loft#1381) keys on exactly one arm
                    // being void — so the mirror would parse here and then hand rustc a raw
                    // E0308.  The `if`'s own join is decided in `parse_if` from the THEN
                    // arm and is unaffected.
                    t.clone()
                } else if sibling_variant {
                    t.clone()
                } else if let Some(w) = boxed_arm_w {
                    // The arm was boxed into `w` (loft#1350): its value is that work-ref,
                    // and naming it is the same mint marker a struct-literal arm carries.
                    result.with_deps(&Deps::frame1(w))
                } else {
                    result.with_deps_of(t)
                };
                if crate::keys::pln25_dn1_enabled()
                    && matches!(t, Type::Optional(_))
                    && !matches!(honest, Type::Optional(_))
                {
                    Type::optional(honest)
                } else {
                    honest
                }
            } else {
                result.clone()
            };
        }
        // I9-var: skip ref_return/text_return for generic templates.
        // The return type T = Reference(tv_nr) triggers ref_return which promotes local
        // variables to hidden parameters.  After specialization to a value type (Integer,
        // Float), those hidden params are wrong.  Specialized copies inherit the template's
        // body and variable table; struct-returning specializations work correctly because
        // they return arguments (not locals), so ref_return would be a no-op anyway.
        //
        // a7: this block PROMOTES a body-tail local to the function's hidden return
        // buffer (`ref_return` renames `__retbuf` to the local; `text_return` likewise).
        // That is only sound at the GENUINE function tail — `parse_code` parses it with
        // context "return from block". An `if`/`match` ARM (context "if"/"else"/
        // "match_arm") is NOT the function tail: promoting an arm's own `__vdb_N` makes
        // that arm both build into AND free the shared return buffer, so its value is lost
        // (interp reads the sibling arm, native reads empty — the two backends diverge).
        // The fn-body tail then delivers every arm into `__retbuf` (the `match` path,
        // `materialize_vector_arms_into`), so gating the promotion to the real return
        // context lets the `if` arms behave exactly like `match` arms already do.
        // @PLN85 generic-tuple-return-fix.md — this promotion is skipped for generic
        // templates because a `-> T` return promotes the wrong locals once T is a
        // value type.  NARROW exception: a generic template returning the synthetic
        // `__tuple<…>` struct (a concrete lifetime-tuple already rewritten at
        // definitions.rs) IS safe to promote here so the monomorph inherits the
        // synthetic-struct body.  Kept to `__tuple` only — enabling the general
        // text_return/ref_return path for ALL concrete generic returns panics on a
        // template whose var table isn't promotion-ready (`plan17_b`, `-> text`).
        let generic_promote_ok = self.data.def_type(self.context) != DefType::Generic
            || matches!(self.data.def(self.context).returned(),
                Type::Reference(d, _) if self.data.def(*d).name().starts_with("__tuple<"));
        if generic_promote_ok && context == "return from block" {
            // loft#918 — pass 1 cannot type a local bound to a call whose callee is
            // declared LOWER in the file: the call has nothing to resolve against there,
            // so the local reads `Unknown` and the text work-buffer promotion below never
            // fires.  Pass 2 types it `text` and promotes, which grows the signature after
            // callers were lowered — the H5 cross-pass divergence, and with the caller
            // declared above the callee a genuine arity mismatch.
            //
            // Record the tail here; `promote_late_text_buffers` settles it between the
            // passes, where every declaration exists.  Same treatment #675 gave the heap
            // return buffer in `reserve_late_return_buffers`.
            if self.first_pass
                && matches!(self.data.def(self.context).returned().base(), Type::Text(_))
                && let Some(v) = Self::tail_bare_var(l)
                && self.vars.tp(v).is_unknown()
                && !self.vars.is_argument(v)
            {
                self.late_text_tails.push((self.context, v));
            }
            // @PLN25 single-payload: the tail was just coerced `__nullable<S>` → dense `S`
            // via a payload sub-ref (`OpGetField`), so `t` is still the Enum tail type and
            // the type-keyed branches below (which match `t`) all miss it — the default
            // epilogue then demotes the unwrap to a discarded statement + `return null`
            // (native returns the null sentinel).  Key off the dense return type `result`
            // instead: materialise the unwrap tail into an owned work-ref (copy the viewed
            // `S`) and promote that — the #306 view-return shape.  Gate-off-inert.
            // #437 + c5/#448 residual — a TAIL explicit `return <fresh-owned vector>`
            // is semantically identical to the implicit tail `<expr>`, but
            // `parse_return` left it as a Never-typed `Value::Return(<expr>)`, so the
            // implicit-tail vector arm below (gated on `t == Vector`) never delivered
            // it: the signature stayed a BARE vector. A direct caller copes (it owns
            // the result), but an NRVO caller that CHAINS this return into its buffer
            // (`return wrap()` → `__retbuf = wrap(__retbuf)`) orphans the fresh store
            // wrap never wrote into __retbuf (#448 c5). `<expr>` is either a named
            // non-arg local (#437) OR a fresh literal / comprehension whose block owns
            // a `__vdb` store (the c5 residual). Strip the `return` → implicit tail and
            // route through the SAME ref_return + NRVO (renames its store onto
            // __retbuf, no copy); ref_return then delivers any sibling mid-body returns
            // via deliver_mid_vector_returns. A mid-body `if { return e }` is in
            // "if"/"match_arm" context, never "return from block", so it is untouched.
            // `!vec_arm_handled` — when the upper match/if (#416) or #448 path
            // already materialised this tail into __retbuf, its delivered block's
            // RESULT TYPE still reads the original `["__vdb"]` (the inner build),
            // so without this gate `fresh_owned_vector_deps` is fooled and delivers
            // it a SECOND time (appending __retbuf into itself → doubled length).
            // loft#938 — `ret_promo_base` so a NULLABLE collection return reaches this
            // arm too: `fn a1(i) -> vector<T>? { if … { return null } return [ … ]; }`
            // otherwise kept building into its own `__vdb` store and handed back a view,
            // leaking one store per call beside the buffer the caller had allocated.
            // Identity while `LOFT_NULLABLE_RETBUF` is off.
            let tail_ret_owned: Option<Vec<u16>> = if !self.first_pass
                && !vec_arm_handled
                && matches!(result.ret_promo_base(), Type::Vector(_, _))
                && let Some(Value::Return(inner)) = l.last().map(Value::unspan)
            {
                // loft#1101 — a viewing local is not fresh-owned, but it still has to be
                // DELIVERED; `tail_ret_view_local` supplies the candidate that copies.
                self.fresh_owned_vector_deps(inner)
                    .or_else(|| self.tail_ret_view_local(l, inner))
            } else {
                None
            };
            if let Some(ls) = tail_ret_owned {
                let last = l.len() - 1;
                // #448 (now a CELL, not a separate upper block) — when the buffer is
                // already TAKEN by a sibling return that delivers __retbuf (an early
                // `return <call>` NRVO-adopted it), RENAMING this fresh-owned tail onto
                // __retbuf would double-own the buffer, so COPY it in via the ONE vector
                // dispatch (Delivery::Materialize: clear + append + free the local; the
                // `returned` re-set to {__retbuf} is idempotent — returned_uses_buffer
                // checked it is already there). The buffer-FREE case RENAMES (#437/c5).
                // One fresh-owned-vector classifier (`fresh_owned_vector_deps`), the deps
                // fact deciding rename-vs-copy.
                let mut delivered = false;
                if !Self::tail_terminal_is_branch(&l[last])
                    && let Type::Vector(elm, _) = result.ret_promo_base()
                    && let Some((buf_attr, buf_var)) = self.return_buffer()
                    && self.returned_uses_buffer(buf_attr)
                    && Self::body_has_buffer_return(&l[..last], buf_var)
                {
                    let elm_ty = (**elm).clone();
                    delivered = self.dispatch_vector_delivery(Delivery::Materialize, &elm_ty, l);
                }
                if !delivered {
                    // buffer FREE (or the copy did not fire) → strip the `return` →
                    // implicit tail (peel any Span, then the Return, keeping the owned
                    // expr — a bare Var #437 or the literal block) and RENAME its store
                    // onto __retbuf via ref_return + NRVO.
                    let mut taken = std::mem::replace(&mut l[last], Value::Null);
                    loop {
                        taken = match taken {
                            Value::Span(b) => b.1,
                            Value::Return(inner) => {
                                l[last] = *inner;
                                break;
                            }
                            other => {
                                l[last] = other;
                                break;
                            }
                        };
                    }
                    self.ref_return(&ls, l, RetSite::BlockTail);
                    self.nrvo_collapse_tail_set(l, &ls);
                }
            } else if let Type::Reference(td, _) = result
                && !l.is_empty()
                && self.tail_is_nullable_unwrap(&l[l.len() - 1])
            {
                // @PLN85 D-own-1 — the nullable-unwrap arm keeps its own entry
                // (it keys on the DECLARED result + the unwrap tail shape, which
                // the `t == Reference` arm below cannot see), but its MECHANISM
                // is the selector's `MaterializeView` cell — one dispatch emits,
                // like the #416/#448 fold on the vector side.
                self.dispatch_reference_delivery(RefDelivery::MaterializeView, *td, l);
            } else if let Type::Text(ls) = t.base() {
                // @PLN25 slice (c): `.base()` — a `-> text?` return dispatches to the same
                // work-buffer conversion as `-> text` (text_return re-applies the `?`).
                self.text_return(ls);
                // @PLN104 — restore `a == v` for a late-promoted retbuf (the third-pass
                // `do_tret_bind` mints it AFTER the inherited `__work_1`, so its variable
                // index exceeds its attribute index; the returned dep, resolved in BOTH
                // spaces, then orphans the owned text — loft#568).  Shared with the targeted
                // promotion's Phase A (`av_renumber_retbuf`).  Gated on `force_tret`, so the
                // 2-pass flow is untouched.
                if self.force_tret.contains(&self.context) && ls.len() == 1 {
                    tp = self.av_renumber_retbuf(l, ls[0]);
                }
            } else if !vec_arm_handled && let Type::Vector(elm, ls) = t.ret_promo_base() {
                // @PLN85 / D-own-1 — classify ONCE from the deps fact + tail shape,
                // then emit. The three old inline branches (recover-hidden-refs /
                // arg-borrow-copy / multi-arm-rename) are now cells of one selector.
                //
                // loft#938 gate 6 of 6 — `ret_promo_base` peels `Optional(Vector)`, the
                // same peel the Text arm above and the Reference arm below already do
                // with `.base()`.  A nullable collection tail missed this arm entirely,
                // so it never reached `ref_return` and `classify_ret_promotion` was
                // never called for it — `LOFT_TRACE_RETPROMO` printed nothing at all.
                // Identity while `LOFT_NULLABLE_RETBUF` is off.
                let delivery = self.classify_vector_delivery(ls, l, context);
                let elm_ty = (**elm).clone();
                self.dispatch_vector_delivery(delivery, &elm_ty, l);
            } else if let Some(td) = t.base().heap_def_nr() {
                // @PLN85 D-own-1 — Reference return sub-thicket: classify ONCE from
                // the deps fact + tail shape, then dispatch to the ONE mechanism
                // (rename via ref_return, or materialise-copy a borrowed-local view).
                // Mirrors the vector `classify_vector_delivery` collapse.
                // `.base()` peel (@PLN85 poison-green): a `-> Item?` return is
                // `Optional(Reference)` and takes the SAME delivery — without
                // the peel it fell through EVERY arm, so an escaping borrowed
                // view (`xs = g.c; e = xs[i]; e`) was returned raw while its
                // block-local copy store was freed (the elision-borrower UAF;
                // silently stale without LOFT_POISON).
                //
                // The arm asks `heap_def_nr`, the one home for *"which record definition
                // does this type name"*, rather than spelling `Type::Reference` itself.
                // A record enum is the SECOND spelling of a struct-like heap store — the
                // whole promotion pass below it was built for both (`ref_return` rebuilds
                // `Type::Enum(td, true, dep)` beside `Type::Reference(td, dep)`) — and a
                // hand-written pattern here could only see the first, so a record-enum
                // return reached no arm at all and no delivery was ever classified for
                // it.  That is `is_keyed`'s story one former over: with no delivery there
                // is no return dep, and an empty dep list is what `returns_borrowed_view`
                // reads as OWNED, so a lambda handing back a captured enum had the caller
                // free the capture — @FR-L-CapHeap, whose *"a captured heap value is
                // SHARED"* this arm is what enforces for a record return (loft#1202,
                // `formal/closures.md` D-clo-17).
                let delivery = self.classify_reference_delivery(&t.base().depend(), l);
                self.dispatch_reference_delivery(delivery, td, l);
            } else if crate::parser::vectors::is_keyed(t) {
                // Enforces @FR-O-Move's second clause for the keyed kinds — *if the return
                // borrows a parameter, the return type records it*.
                //
                // loft#1140 — the five KEYED kinds reached no arm above, so no delivery was
                // classified for them and `ref_return` never ran.  Nothing recorded that a
                // returned keyed collection BORROWS a parameter, and an empty return-dep
                // list is what `Def::returns_borrowed_view` reads as *owned*: the caller
                // then set `OpCopyRecord`'s source-free bit on a store it still held, so
                // `fn id(x: hash<T[k]>) -> hash<T[k]> { x }` freed the caller's collection
                // and every call after the first read it empty, on both backends.
                //
                // A keyed collection needs the BORROW FACT and nothing else — it already
                // carries its own delivery, returning its store directly rather than
                // through a `__retbuf` the way a vector does.  `ref_return` treats it as
                // `signature_only` for exactly that reason, so this records the dep and
                // makes no placement decision; the runtime @P290 bracket then refuses the
                // free for a store belonging to a protected argument, which is the
                // borrow-vs-owned split `use_analysis::protectable_ref_args` describes and
                // has covered keyed arguments since loft#981.
                //
                // `is_keyed` is the one home for the kind list (@FR-Col-Hash · -Sorted ·
                // -Index · -Spatial · -Trie), so a sixth keyed kind arrives here already
                // handled instead of joining the four hand-spelled `Type::Vector` lists
                // this arm sits beside.
                let ls = t.base().depend();
                self.ref_return(&ls, l, RetSite::BlockTail);
            } else if let Type::Vector(elm, _) = result.ret_promo_base()
                && let Some((buf_attr, buf_var)) = self.return_buffer()
                && self.returned_uses_buffer(buf_attr)
            {
                // #448 mirror — the fn is buffer-bound but NONE of the cells above
                // handled this tail: it became buffer-bound via a tail `return <call>`
                // chain (parse_return sets that up as a MidReturn, which — unlike a tail
                // rename — never triggers deliver_mid_vector_returns). A mid-body
                // fresh-owned return (an early `return [literal]`) was deferred by
                // parse_return and would orphan its store on that path. Deliver every
                // mid-body return into __retbuf now that the binding is final. Cells
                // that DID handle the tail short-circuit this arm (their ref_return
                // already delivered the mid-body), so this never double-delivers.
                let elm_ty = (**elm).clone();
                self.deliver_mid_vector_returns(&elm_ty, l, buf_var);
            }
        }
        tp
    }

    /// @PLN104 — restore the `a == v` invariant for a promoted text retbuf `tv` (its return
    /// dep is resolved in BOTH attribute- and variable-space, so the two indices must agree,
    /// else the owned text orphans on the interpreter — loft#568).  Swap `tv` into the
    /// variable slot matching its attribute index (its position among `arguments()`), moving
    /// the body IR `l`, the typedef deps, and the variable table in tandem.  Returns the block
    /// type naming the retbuf's (post-swap) slot.  Shared by `block_result`'s third-pass swap
    /// and the targeted promotion's Phase A.  `self.vars` / `self.context` must already be the
    /// promoted def's (block_result is inside its parse; Phase A swaps them in first).
    fn av_renumber_retbuf(&mut self, l: &mut [Value], tv: u16) -> Type {
        let Some(a) = self
            .vars
            .arguments()
            .iter()
            .position(|&x| x == tv)
            .map(|p| p as u16)
        else {
            return Type::Text(Deps::frame1(tv));
        };
        if a != tv {
            // 3-way swap `a <-> tv` of every frame reference; `tmp` is a fresh scratch index
            // (one past the live vars), fully undone below.
            let tmp = self.vars.count();
            for op in l.iter_mut() {
                Self::renumber_frame_var(op, a, tmp);
                Self::renumber_frame_var(op, tv, a);
                Self::renumber_frame_var(op, tmp, tv);
            }
            self.vars.renumber_frame_in_types(a, tmp);
            self.vars.renumber_frame_in_types(tv, a);
            self.vars.renumber_frame_in_types(tmp, tv);
            self.vars.swap_variables(a, tv);
        }
        Type::Text(Deps::frame1(a))
    }

    /// @PLN85 / D-own-1 — the SELECTOR for an implicit-tail `t == Vector` return:
    /// read the deps fact `ls` and the tail shape ONCE and pick a [`Delivery`].
    /// Pure (`&self`) so classification and emission stay separable. Replaces the
    /// three inline branches the vector arm of `block_result` used to carry.
    /// #409 — does this tail call hand back a store of its OWN, ignoring the
    /// `__retbuf` it was given?
    ///
    /// A `#native`/`#rust` declaration with a heap return does: its body is foreign
    /// code that allocates and returns, so leaving the forward in place returns that
    /// value with `__retbuf` still empty. The caller must copy the record/elements
    /// across instead (`Delivery::ForwardCopy` / `RefDelivery::ForwardCopy`).
    ///
    /// Such a callee is PASS-STABLE (`code == Null` plus a symbol, identical on both
    /// parse passes), so a local minted off this answer is safe. One home for the
    /// fact because both delivery selectors ask it — they disagreed once, and the
    /// Reference side's silent `AsIs` is loft#867's null-filled struct.
    fn tail_forwards_own_store(tail: &Value, data: &crate::data::Data) -> bool {
        matches!(tail.unspan(), Value::Call(d, _) if {
            let cd = data.def(*d);
            *cd.code() == Value::Null
                && (!cd.native().is_empty() || !cd.rust().is_empty())
                && matches!(
                    cd.returned(),
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                )
        })
    }

    fn classify_vector_delivery(&self, ls: &[u16], l: &[Value], context: &str) -> Delivery {
        if ls.is_empty() && !l.is_empty() {
            // Issue #120 mirror (see the Reference arm): when filter_hidden
            // stripped the deps, recover the tail call's work refs so the site
            // still binds to the one buffer.
            let last = &l[l.len() - 1];
            let extra = Self::collect_hidden_ref_args(last, &self.data);
            // Chain the wrapper into its callee's buffer — UNLESS the callee
            // forwards a foreign store and never writes that buffer
            // (`fn f() -> vector { stack_trace() }`): chaining a grand-caller's
            // buffer through such a forwarder orphans it (#355 follow-up leak,
            // 55-stack-trace). The forwarder test reads the callee's BODY shape,
            // which is pass-stable (unlike its `returned` deps).
            let callee_forwards = matches!(last.unspan(), Value::Call(d, _)
                if self.callee_forwards_foreign_store(*d));
            let native_forwarder = Self::tail_forwards_own_store(last, &self.data);
            if !extra.is_empty() && !callee_forwards {
                Delivery::Rename(extra)
            } else if native_forwarder {
                Delivery::ForwardCopy
            } else {
                Delivery::AsIs
            }
        } else if !self.first_pass
            && ls.iter().any(|&d| self.vars.is_argument(d))
            && (self.tail_is_struct_field_read(l)
                // The whole-arg copy (a2) fires ONLY at the function-body tail: an
                // `if`/`match` ARM block also reaches block_result (context "if"/
                // "else"/"match_arm") with a `{ v }` tail, but the arm is already
                // delivered into the buffer by the outer if-unify / arm-materialise
                // path — copying it again here orphans the buffer (a11 leak).
                // `return from block` is the one funnelled return path (row 104).
                || (context == "return from block"
                    && self.tail_whole_arg_vector(l).is_some())
                // @PLN85 P14 — a borrowed match-arm binding (or local) returned
                // directly borrows a visible arg; copy it into __retbuf rather
                // than rename it onto (and alias) the caller's buffer.
                || (context == "return from block" && self.tail_borrows_arg(l)))
            && self
                .return_buffer()
                .is_some_and(|(_, buf_var)| !ls.contains(&buf_var))
        {
            // Row-104 — an implicit-tail return whose value BORROWS a visible
            // argument: a STRUCT vector FIELD of an arg (`fn getv(b: Box) ->
            // vector { b.v }`, #415) OR the whole vector arg itself
            // (`fn idv(v) -> vector { v }`, A.2/a2). Returning the tail as-is
            // ALIASES the caller's store, so copy it into `__retbuf` (value
            // semantics). The EXPLICIT `return v` / `return b.v` already does this
            // (parse_return), suite-proven. Narrowed by the two tail predicates to
            // whole-arg / struct-field tails: index / call tails stay on the rename
            // path, which already delivers them correctly. `ls` is carried for
            // the alloc-failure fallback rename.
            Delivery::CopyBorrow(ls.to_vec())
        } else {
            // #437/@PLN85 cluster V (O-Move): a multi-arm `match`/`if` vector tail
            // must deliver EVERY arm's buffer into the one return buffer. The
            // Vector type dep `ls` can be INCOMPLETE — it carries only the first
            // arm's `__ref_1`, while a later arm's `__ref_N` is allocated but
            // unregistered, so scope analysis frees it and the function returns a
            // dangling ref into a freed store. Union `ls` with every hidden
            // buffer-arg ref in the tail so ref_return renames each arm's buffer
            // onto the retbuf (the pre-#437 [__ref_1, __ref_2] shape).
            let mut full: Vec<u16> = ls.to_vec();
            if let Some(last) = l.last() {
                for w in Self::collect_hidden_ref_args(last, &self.data) {
                    if !full.contains(&w) {
                        full.push(w);
                    }
                }
            }
            Delivery::Rename(full)
        }
    }

    /// @PLN85 / D-own-1 — emit the mechanism the selector chose for a vector
    /// return. `elm` is the vector element type (for the element-copy ops); the
    /// tail of `l` is rewritten in place. Returns whether a `Materialize` actually
    /// delivered (the upper #416 / #448 callers gate `vec_arm_handled` / a fallback
    /// rename on it); the other variants always handle their tail.
    fn dispatch_vector_delivery(
        &mut self,
        delivery: Delivery,
        elm: &Type,
        l: &mut [Value],
    ) -> bool {
        match delivery {
            Delivery::Rename(ws) => {
                self.ref_return(&ws, l, RetSite::BlockTail);
                // @P377 / S1: collapse `cv = inner_call(...); cv` so the inner
                // call's hidden buffer arg points at cv directly.
                self.nrvo_collapse_tail_set(l, &ws);
                // NOTE: a `[]`/empty slice arm is already a REAL fresh vector here —
                // `parse_vector_match::materialize_null_slice_arms` rewrote it before
                // this delivery, so there is no bare `null` arm left to handle.
                true
            }
            Delivery::CopyBorrow(ls) => {
                // The buffer's existence was verified by the selector; re-fetch it
                // for the copy (idempotent — nothing mutated in between). Fall back
                // to the rename path if the copy's work-var allocation fails.
                if let Some((buf_attr, buf_var)) = self.return_buffer() {
                    // A buffer-bound vector fn delivers EVERY return site into the
                    // buffer, not only its tail. `copy_borrow_tail_into_retbuf` is a
                    // tail-only funnel, so a mid-body `return <fresh local>` would
                    // otherwise hand back a store the caller never adopts and nothing
                    // frees. `Rename` gets this for free from `ref_return`; this arm
                    // asks for it directly. Idempotent — the walker rewrites only
                    // `Return(Var(v))` with `v != buf_var`, and its own rewrite
                    // yields `Return(Var(buf_var))` — so the fallback below, which
                    // delivers again via `ref_return`, cannot double-deliver.
                    self.deliver_mid_vector_returns(elm, l, buf_var);
                    if !self.copy_borrow_tail_into_retbuf(elm, l, buf_attr, buf_var) {
                        self.ref_return(&ls, l, RetSite::BlockTail);
                        self.nrvo_collapse_tail_set(l, &ls);
                    }
                }
                true
            }
            Delivery::ForwardCopy => {
                self.emit_forward_copy_409(elm, l);
                true
            }
            Delivery::Materialize => {
                // The per-arm / fresh-local element copy (#416 branch tails, #448
                // buffer-taken tails). Materialise each arm/tail into __retbuf and
                // finalise the return-type dep to {__retbuf} — the step #416 always
                // did and #448 relied on (returned already `["__retbuf"]`, so the
                // set is idempotent). Returns false when there is no buffer or the
                // materialiser found no rewritable terminal, so the caller can fall
                // back (#448) or leave convert to run (#416).
                let last = l.len() - 1;
                if let Some((buf_attr, buf_var)) = self.return_buffer()
                    && self.materialize_vector_arms_into(elm, &mut l[last], buf_var)
                {
                    // loft#938 — the deps belong to the STORAGE and the `?` to the value,
                    // so re-typing one must not drop the other: a nullable collection
                    // return keeps its `?` around the buffer-dep'd base.  Identity while
                    // the switch is off.
                    let delivered =
                        Type::Vector(Box::new(elm.clone()), Deps::attrs(vec![buf_attr]));
                    let declared = self.data.def(self.context).returned();
                    self.data.definitions[self.context as usize].returned =
                        if declared.ret_promo_peels() {
                            Type::optional(delivered)
                        } else {
                            delivered
                        };
                    true
                } else {
                    false
                }
            }
            Delivery::AsIs => false,
        }
    }

    /// @PLN85 D-own-1 — the SELECTOR for a `Type::Reference` (struct) return: read
    /// the deps fact `ls` + the tail shape ONCE and pick a [`RefDelivery`]. Pure
    /// (`&self`). Replaces the three inline branches the Reference arm of
    /// `block_result` carried; mirrors `classify_vector_delivery`.
    fn classify_reference_delivery(&self, ls: &[u16], l: &[Value]) -> RefDelivery {
        if l.last()
            .is_some_and(|tail| Self::tail_calls_a_fnref_parameter(tail, &self.vars))
        {
            // loft#1185 — the tail calls through a fn-ref PARAMETER, so what comes back may be
            // the capture of whatever closure the caller passed: a store this frame does not
            // own and its caller cannot name.
            //
            // The fact exists one frame UP, where the closure was named, and it cannot travel
            // down: this function's return type is computed ONCE for every caller, so no
            // per-argument dep reaches it.  Reading the result as a borrow instead is not
            // available either — the same parameter carries a closure that MINTS, and a borrow
            // there leaks the mint (`call_it(fresh, 1)`, measured clean today).
            //
            // So the value is COPIED before it escapes, which is what `MaterializeView` already
            // does for a tail that points into something the callee frees.  The caller then owns
            // an ordinary fresh record and the capture is untouched, at the cost of one record
            // copy on the forwarding path — the cost `formal/closures.md` D-clo-12 already
            // named for closing this.
            return RefDelivery::MaterializeView;
        }
        if l.last()
            .is_some_and(|tail| self.return_projects_into_local(tail))
        {
            // The tail POINTS INTO something the callee frees, so it cannot be
            // renamed onto the caller's buffer — copy the record in first.
            //
            // #425 / H9 at the implicit tail: `fn get() -> Inner { mk().inn }`
            // projects a field of an inline-call temporary, lifted to `__lift_N`
            // and freed at scope exit, so returning it as-is hands the caller a ref
            // into a freed store — native discards it and returns null, the
            // interpreter reads the stale bytes and only looks right.
            //
            // moros H12 is the ELEMENT twin — `{ b = make_bag(); b.b_cells[i] }`.
            // It reached the `Rename` fallback below instead, which promoted the
            // local `b` (a `Bag`) onto a `Cell`-typed return buffer; `ls` is `[b]`
            // and `return_views_local` inspects each dep's own *further* deps, so
            // it cannot see that the tail merely projects into `b`.
            return RefDelivery::MaterializeView;
        }
        if ls.is_empty() {
            // Issue #120: deps stripped — recover the tail's hidden work-refs so the
            // site still binds to the one buffer. No work-ref to recover → AsIs.
            if let Some(last) = l.last() {
                let extra = Self::collect_hidden_ref_args(last, &self.data);
                if !extra.is_empty() {
                    return RefDelivery::Rename(extra);
                }
                // #409, the Reference twin of the vector selector's `native_forwarder`
                // leg: a `#native`/`#rust` callee with a heap return delivers a store
                // of its OWN and never writes the `__retbuf` it was handed. `AsIs`
                // claims the tail already wrote the buffer, so the call was emitted as
                // a bare statement and the untouched buffer returned — a null-filled
                // struct out of `text as Struct` on native (loft#867). Copy the
                // forwarded record into the buffer instead.
                if Self::tail_forwards_own_store(last, &self.data) {
                    return RefDelivery::ForwardCopy;
                }
            }
            RefDelivery::AsIs
        } else if self.return_views_local(ls) || !self.ls_can_be_record_buffer(ls) {
            // #306: the tail borrows a LOCAL's store — copy it before it escapes.
            RefDelivery::MaterializeView
        } else {
            // Owned / arg-borrow: rename the tail's work-ref(s) onto `__retbuf`.
            RefDelivery::Rename(ls.to_vec())
        }
    }

    /// Is this tail a call through a fn-typed PARAMETER?
    ///
    /// The one shape whose returned store the frame can say nothing about: a fn-ref LOCAL's
    /// type carries the closure it was built with, so a call through it publishes a dep the
    /// caller can map, while a PARAMETER's type is the declared `fn(…) -> τ` and carries
    /// nothing about any closure — which is exactly why loft#1185's fact has no route.
    ///
    /// Looks through the wrappers a tail can be sitting in, and asks only about the call
    /// itself: a tail that merely CONTAINS such a call somewhere is not this question.
    fn tail_calls_a_fnref_parameter(tail: &Value, vars: &crate::variables::Function) -> bool {
        let mut node = tail.unspan();
        loop {
            match node {
                Value::Return(inner) => node = inner.unspan(),
                Value::Insert(steps) | Value::Parallel(steps) => match steps.last() {
                    Some(last) => node = last.unspan(),
                    None => return false,
                },
                Value::Block(bl) => match bl.operators.last() {
                    Some(last) => node = last.unspan(),
                    None => return false,
                },
                Value::CallRef(v, _) => {
                    return *v < vars.count()
                        && vars.is_argument(*v)
                        && matches!(vars.tp(*v).base(), Type::Function(_, _, _));
                }
                _ => return false,
            }
        }
    }

    /// Can every candidate in `ls` BE the record return buffer?
    ///
    /// The buffer is what the CALLER allocates and the callee fills, so a candidate
    /// renamed onto `__retbuf` must have the return's own shape — a record.  A tail
    /// that indexes a local CONTAINER leaves that container in `ls`
    /// (`make(n)[0] ?? d`: the `??` block's value depends on the vector, and the
    /// vector itself has no further deps, so `return_views_local` reads it as owned).
    /// Renaming it gave a `vector<Cell>` producer a `Cell`-shaped buffer to build
    /// into: `make` cleared the caller's record as a vector, the index then read
    /// absent and the `??` answered its fallback for every input — a wrong value on
    /// the interpreter, an out-of-bounds store index on native (loft#877).  It is the
    /// promotion `moros H12` hit through a field projection, reached here through an
    /// index instead, which is why the guard is the SHAPE and not another tail walker.
    ///
    /// Only a COLLECTION blocks the rename.  It is what a record return can be a view
    /// INTO, so it is the shape whose presence here means "the tail indexed something",
    /// and the narrowness matters: a candidate is not always a container the return
    /// borrows from, and materialising one that is merely a null-valued record turns
    /// `null` into a freshly allocated empty record — a `-> Dialect?` that answers a
    /// dialect for a connection that was never made.
    ///
    /// A PARAMETER is not a placement question — the return borrows it, and
    /// `classify_ret_promotion`'s `MergeAttr` rung already says so by merging the attr
    /// into the return deps — so an attribute never blocks the rename.
    fn ls_can_be_record_buffer(&self, ls: &[u16]) -> bool {
        let attr_names = &self.data.def(self.context).attr_names;
        ls.iter().all(|&v| {
            v >= self.vars.count()
                || attr_names.contains_key(self.vars.name(v))
                || !crate::parser::vectors::is_collection(self.vars.tp(v))
        })
    }

    /// @PLN85 D-own-1 — emit the mechanism the Reference selector chose. The tail of
    /// `l` is rewritten in place; mirrors `dispatch_vector_delivery`.
    fn dispatch_reference_delivery(&mut self, delivery: RefDelivery, td: u32, l: &mut [Value]) {
        match delivery {
            RefDelivery::Rename(ws) => {
                self.ref_return(&ws, l, RetSite::BlockTail);
                self.nrvo_collapse_tail_set(l, &ws);
            }
            RefDelivery::MaterializeView => {
                let last = l.len() - 1;
                let w = self.materialize_view_return(td, &mut l[last]);
                self.ref_return(&[w], l, RetSite::BlockTail);
                self.nrvo_collapse_tail_set(l, &[w]);
            }
            RefDelivery::ForwardCopy => self.emit_forward_copy_ref_409(td, l),
            RefDelivery::AsIs => {}
        }
    }

    /// #409 — a `#native`/`#rust` callee delivers its OWN store and never writes
    /// the `__retbuf` it was handed; leaving the forward returns that foreign
    /// value with `__retbuf` empty, so the caller's later in-place `+=` rebuilds
    /// the empty buffer and drops the data. Mint a fresh `__fwd` local, run the
    /// call into it, then COPY into `__retbuf` (clear + element-append) — the
    /// shape a hand-written `r = native(); r` produces. Finalize the return-type
    /// dep to `{__retbuf}` so a caller binds its result var to the buffer it
    /// passed (else the signature stays bare-vector and `+=` drops data). A no-op
    /// #409 for a `Type::Reference` return — the record twin of
    /// [`Self::emit_forward_copy_409`].
    ///
    /// The tail forwards a store the callee allocated itself, so bind it to a fresh
    /// `__fwd` local and copy the RECORD into `__retbuf`; `__fwd` is an ordinary
    /// owned local, so the normal scope sweep frees the forwarded store rather than
    /// orphaning it. Finalise `returned` to `{__retbuf}` so a caller reads its result
    /// out of the buffer it passed.
    ///
    /// A no-op when there is no buffer, no work-var, or no tail — the caller's
    /// `AsIs` behaviour, which is what a fn with no heap-return ABI wants anyway.
    fn emit_forward_copy_ref_409(&mut self, td: u32, l: &mut [Value]) {
        let Some((buf_attr, buf_var)) = self.return_buffer() else {
            return;
        };
        let fwd = self.create_var("__fwd", &Type::Reference(td, Deps::none()));
        if fwd == u16::MAX {
            return;
        }
        let Some(last) = l.last_mut() else {
            return;
        };
        let orig = std::mem::replace(last, Value::Null);
        // `materialize_return_into` emits the proven copy-into-a-destination shape
        // (`dest = null; OpDatabase(dest, kt); OpCopyRecord(src, dest, kt); dest`),
        // the same one `ref_return`'s copy leg uses to fill the buffer from a named
        // local. Here the source is the `__fwd` we just bound.
        let mut copy_tail = Value::Var(fwd);
        self.materialize_return_into(td, &mut copy_tail, buf_var);
        l[l.len() - 1] = crate::data::v_block(
            vec![crate::data::v_set(fwd, orig), copy_tail],
            Type::Reference(td, Deps::frame1(buf_var)),
            "fwd_copy_409_ref",
        );
        let dep = Deps::attrs(vec![buf_attr]);
        self.data.definitions[self.context as usize].returned = Type::Reference(td, dep);
    }

    /// when there is no buffer, no work-var, or no tail.
    fn emit_forward_copy_409(&mut self, elm: &Type, l: &mut [Value]) {
        let elm_ty = elm.clone();
        let Some((buf_attr, buf_var)) = self.return_buffer() else {
            return;
        };
        let fwd = self.create_var(
            "__fwd",
            &Type::Vector(Box::new(elm_ty.clone()), Deps::none()),
        );
        if fwd == u16::MAX {
            return;
        }
        let rec_tp = self.append_elem_tp(&elm_ty);
        let clear = self.cl("OpClearVector", &[Value::Var(buf_var)]);
        let append = self.cl(
            "OpAppendVector",
            &[Value::Var(buf_var), Value::Var(fwd), Value::Int(rec_tp)],
        );
        let Some(last) = l.last_mut() else {
            return;
        };
        let orig = std::mem::replace(last, Value::Null);
        let set_fwd = crate::data::v_set(fwd, orig);
        *last = crate::data::v_block(
            vec![set_fwd, clear, append, Value::Var(buf_var)],
            Type::Vector(Box::new(elm_ty.clone()), Deps::frame1(buf_var)),
            "fwd_copy_409",
        );
        self.set_delivered_vector_return(elm_ty, buf_attr);
    }

    /// Plan-14 phase 07 (P234 runtime): rewrite a body-tail
    /// `Value::Tuple([elem_0, elem_1, …])` into the synthetic-struct
    /// construction sequence that an inline struct literal would
    /// produce — `(p, 5)` becomes
    ///
    /// ```text
    /// {
    ///     w = null;
    ///     OpDatabase(w, __tuple<…>_known_type);
    ///     w._0 = elem_0;     // OpSet* at field offset 0
    ///     w._1 = elem_1;     // OpSet* at field offset 16 (alignment-padded)
    ///     w
    /// }
    /// ```
    ///
    /// Mirrors `parse_object`'s allocation + per-field-init pattern.
    /// The work-ref `w` is created via `vars.work_refs(...)`; the
    /// resulting block carries `Reference(synthetic_d_nr, vec![w])`
    /// so scope analysis tracks `w`'s store as the source of the
    /// returned DbRef's lifetime — same machinery struct returns
    /// use today.
    /// P236: when a function body's tail is `Value::If(...)` (or `match`,
    /// which lowers to nested `If`) and each branch terminates with a
    /// fresh work-ref via Object/struct construction, the branches end
    /// up with DIFFERENT work-refs (`__ref_1`, `__ref_2`, …).  Native
    /// codegen then loses the if/else's value: each branch's local DbRef
    /// is dropped, both work-refs get freed, and the function returns the
    /// typed null sentinel.  Interp accidentally works because OpReturn
    /// reads from eval-stack top.
    ///
    /// Fix: pick the FIRST branch's terminal work-ref as the shared one
    /// and rewrite every other branch in place — substitute their
    /// work-ref `Var` references with the shared one, and rewrite Set
    /// LHS slots and Block.result deps so scope analysis tracks the
    /// shared work-ref as the unique source of the returned DbRef's
    /// lifetime.  After unification, `returned_var(If)` (extended in
    /// `scopes.rs::returned_var`) recognises the shared var and skips
    /// `OpFreeRef` on it; `ref_return` promotes it to a hidden caller
    /// arg as it would for a single-branch reference return.
    ///
    /// Returns `Some(shared_work_ref)` if the rewrite fired (so the
    /// caller can wrap the if/else in `Value::Return`), `None`
    /// otherwise (mixed shapes, no work-refs, or branches already
    /// share a var).
    pub(crate) fn unify_if_branches_work_refs(&mut self, tail: &mut Value) -> Option<u16> {
        let if_value = tail.unspan_mut();
        if !matches!(if_value, Value::If(_, _, _)) {
            return None;
        }
        // Collect EVERY arm's terminal var across the whole if-tree — an `else if`
        // chain nests the alternatives as `If(_, arm, If(_, arm, …))`, so a 3-arm
        // (or deeper) tail has three+ distinct terminals (`__ref_1`, `__ref_2`,
        // `__ref_3`). The 2-arm case is just N=2 of this. Without collecting the
        // whole chain the nested `If`'s terminals differ and the old pair-only
        // lookup bailed, leaving native to drop the value and return the typed
        // null sentinel for every arm (struct/ref 3-arm if returned the LAST arm
        // on native — the struct sibling of the vector a7 bug).
        let mut terms = Vec::new();
        Self::collect_branch_terminal_vars(if_value, &mut terms);
        // Need at least one terminal, and ALL must be parser-internal work-refs
        // (`__ref_N` / `__rref_N`). Renaming a user-named parameter (e.g.
        // `if c { gen_x } else { gen_y }`) would corrupt the result, so bail and
        // let the existing scope analysis handle the tail.
        let first = *terms.first()?;
        let all_work_refs = terms.iter().all(|&v| {
            let n = self.vars.name(v);
            n.starts_with("__ref_") || n.starts_with("__rref_")
        });
        if !all_work_refs {
            return None;
        }
        // Pick the FIRST arm's work-ref as the shared one; rewrite every OTHER arm
        // (Var references, Set LHS slots, Block.result deps) to it across the whole
        // tail, so all arms deliver through one return slot. Idempotent when the
        // arms already share `first`.
        for &other in terms.iter().skip(1) {
            if other != first {
                Self::substitute_work_ref(if_value, other, first);
            }
        }
        Some(first)
    }

    /// Collect the terminal `Value::Var` of EVERY arm reachable through an
    /// `if`/`else-if` chain — descends both branches of each nested `If` so an
    /// N-arm chain yields all N terminals. A non-`Var`-terminating arm
    /// contributes nothing, so a mixed tail (one arm not ending in a work-ref)
    /// is detected by the caller's `all_work_refs` check and left un-unified.
    fn collect_branch_terminal_vars(branch: &Value, out: &mut Vec<u16>) {
        match branch.unspan() {
            Value::Var(v) => {
                if !out.contains(v) {
                    out.push(*v);
                }
            }
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last() {
                    Self::collect_branch_terminal_vars(last, out);
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last() {
                    Self::collect_branch_terminal_vars(last, out);
                }
            }
            Value::If(_, t, f) => {
                Self::collect_branch_terminal_vars(t, out);
                Self::collect_branch_terminal_vars(f, out);
            }
            _ => {}
        }
    }

    /// Replace every reference to work-ref `from` with `to` in `val` —
    /// extends `replace_var_in_ir` semantics to also rewrite `Set`
    /// LHS slots and `Block.result` dep entries.  Used by
    /// `unify_branch_to` so the parser-level dep tracking (which feeds
    /// scope analysis and `ref_return`) sees only the shared work-ref
    /// after unification.
    fn substitute_work_ref(val: &mut Value, from: u16, to: u16) {
        match val {
            Value::Var(v) if *v == from => {
                *v = to;
            }
            Value::Set(slot, body) => {
                if *slot == from {
                    *slot = to;
                }
                Self::substitute_work_ref(body, from, to);
            }
            Value::TuplePut(slot, _, body) => {
                if *slot == from {
                    *slot = to;
                }
                Self::substitute_work_ref(body, from, to);
            }
            Value::Return(body) | Value::Drop(body) | Value::Yield(body) => {
                Self::substitute_work_ref(body, from, to);
            }
            Value::Call(_, args)
            | Value::CallRef(_, args)
            | Value::Insert(args)
            | Value::Tuple(args)
            | Value::Parallel(args) => {
                for a in args.iter_mut() {
                    Self::substitute_work_ref(a, from, to);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &mut bl.operators {
                    Self::substitute_work_ref(op, from, to);
                }
                Self::rewrite_dep_in_type(&mut bl.result, from, to);
            }
            Value::If(cond, t, f) => {
                Self::substitute_work_ref(cond, from, to);
                Self::substitute_work_ref(t, from, to);
                Self::substitute_work_ref(f, from, to);
            }
            Value::Iter(_, a, b, c) => {
                Self::substitute_work_ref(a, from, to);
                Self::substitute_work_ref(b, from, to);
                Self::substitute_work_ref(c, from, to);
            }
            Value::Span(b) => Self::substitute_work_ref(&mut b.1, from, to),
            Value::Var(_)
            | Value::Int(_)
            | Value::Long(_)
            | Value::Float(_)
            | Value::Single(_)
            | Value::Boolean(_)
            | Value::Text(_)
            | Value::Enum(_, _)
            | Value::Line(_)
            | Value::Break(_)
            | Value::Continue(_)
            | Value::Keys(_)
            | Value::TupleGet(_, _)
            | Value::FnRef(_, _, _)
            | Value::FnRefDnr(_)
            | Value::RawExpr(_)
            | Value::Null => {}
        }
    }

    /// Replace `from` with `to` in any dep list inside `tp`'s
    /// reference-bearing variants.  Mirrors how
    /// `Type::Reference(_, deps)` carries the source-of-lifetime var
    /// list that scope analysis reads.
    fn rewrite_dep_in_type(tp: &mut Type, from: u16, to: u16) {
        let deps_mut: Option<&mut Vec<u16>> = match tp {
            Type::Reference(_, d)
            | Type::Vector(_, d)
            | Type::Enum(_, true, d)
            | Type::Sorted(_, _, d)
            | Type::Hash(_, _, d)
            | Type::Index(_, _, d)
            | Type::Radix(_, _, d)
            | Type::Trie(_, _, d)
            | Type::Text(d) => Some(d),
            _ => None,
        };
        if let Some(d) = deps_mut {
            for v in d.iter_mut() {
                if *v == from {
                    *v = to;
                }
            }
        }
    }

    /// @P377 / S1 — parse-time NRVO for the intermediate-local return shape.
    ///
    /// After `ref_return` has promoted `cv` to the function's hidden return
    /// buffer attribute, an inner heap-returning call inside the body still
    /// targets its own parser-synthesised `__ref_N` work-ref, and the
    /// `Set(cv, …)` then copies that into `cv`.  `__ref_N`'s store has no
    /// remaining owner — that's the @P377 leak.
    ///
    /// S1 substitutes `__ref_N → cv` in the inner Call's hidden-buffer arg
    /// (and anywhere else `__ref_N` is referenced inside the inner Call)
    /// so the inner Call writes directly into the outer fn's hidden buffer.
    /// `Set(cv, …)` becomes a same-store self-copy — already a no-op in
    /// `OpCopyRecord` and exercised today by every direct-return shape.
    ///
    /// Preconditions — fires only when ALL hold:
    ///   1. `cv` is in `ls` (just promoted by `ref_return` immediately above).
    ///   2. Block tail is `Var(cv)` or `Return(Var(cv))` (modulo `Span`).
    ///   3. Penultimate statement is `Set(cv, Call(fn_nr, args))`.
    ///   4. `fn_nr` has a hidden Reference / Vector / struct-Enum attribute
    ///      at some index `i`.
    ///   5. `args[i]` is `Value::Var(work_ref)` and `vars.name(work_ref)`
    ///      starts with `__ref_` / `__rref_` (parser-internal, not a
    ///      user-named alias).
    ///   6. `work_ref != cv` (idempotency).
    ///
    /// Bails silently on any mismatch.  No warnings, no errors.  The
    /// Set/Var pair is left in place — the codegen treats a same-store
    /// `OpCopyRecord` as a no-op, so the IR shape stays uniform with the
    /// direct-return path.
    pub(crate) fn nrvo_collapse_tail_set(&mut self, l: &mut [Value], ls: &[u16]) {
        if self.first_pass || l.is_empty() || ls.is_empty() {
            return;
        }
        let last = l.len() - 1;

        // (1) Tail must be `Var(cv)` or `Return(Var(cv))`, modulo Span.
        let Some(cv) = Self::tail_var(&l[last]) else {
            return;
        };
        if !ls.contains(&cv) {
            return;
        }

        if last == 0 {
            // No prior op to substitute — only the tail Var(cv).
            return;
        }

        // (2) FAST PATH — the penultimate op is `Set(cv, Call(...))`: collapse
        //     it (and any earlier CONSECUTIVE `Set(cv, Call)` chain) below.
        //     When the defining call is EARLIER, with in-place mutation between
        //     it and the tail (`t = base(); t += …; t` — the @PLN85 cluster-462
        //     merge / `game_items()` shape), the penultimate is the mutation,
        //     not the call: fall back to redirecting cv's single top-level
        //     defining call instead of returning (else its `__ref_N` buffer is
        //     allocated, orphaned, and leaks one store per call).
        let penultimate_is_set_cv_call = matches!(
            l[last - 1].unspan(),
            Value::Set(slot, rhs) if *slot == cv && matches!(rhs.unspan(), Value::Call(_, _))
        );
        if !penultimate_is_set_cv_call {
            let collapsed = self.nrvo_collapse_defining_call(l, cv);
            self.suppress_collapsed_workrefs(l, collapsed);
            return;
        }
        let prev = l[last - 1].unspan_mut();
        let Value::Set(slot, rhs) = prev else { return };
        if *slot != cv {
            return;
        }
        let rhs_inner = rhs.unspan_mut();
        let Value::Call(fn_nr, args) = rhs_inner else {
            return;
        };
        let fn_nr_val = *fn_nr;

        // (3) The callee's return-buffer attribute (see `hidden_return_buffer_attr`).
        let hidden_idx = self.data.def(fn_nr_val).hidden_return_buffer_attr();
        let Some(i) = hidden_idx else { return };

        // (4) args[i] must be a parser-internal __ref_N / __rref_N work-ref,
        //     distinct from cv.
        if args.len() <= i {
            return;
        }
        let work_ref = match args[i].unspan() {
            Value::Var(v) => *v,
            _ => return,
        };
        if work_ref == cv {
            return;
        }
        let nm = self.vars.name(work_ref);
        if !nm.starts_with("__ref_") && !nm.starts_with("__rref_") {
            return;
        }

        // (5) Substitute work_ref → cv inside the call's args.
        for a in args.iter_mut() {
            Self::substitute_work_ref(a, work_ref, cv);
        }
        // Each work-ref collapsed onto `cv` is now redirected: the inner call
        // delivers into `cv` directly, so the work-ref's own buffer is orphaned.
        // Collect them and (below, once `l` is final) suppress their eager
        // allocation — without this they leak one store per call (the
        // adopt-and-re-return shape, @PLN85 cluster-462 / #462).
        let mut collapsed_refs = vec![work_ref];

        // (6) @PLAN51 Cluster II — extend the substitution backwards to
        //     EARLIER consecutive `Set(cv, Call(_))` ops (probes 02, 21).
        //     Stops at any non-Set/non-Line op (intervening stmt, If,
        //     etc.) — those are unsafe to swap through (the discard's
        //     RHS may read cv; conditional Sets need branch-aware
        //     reasoning).  Probes 03, 04, 07, 11, 25, 26, 28 remain
        //     leaky; their substitution requires extending into IR
        //     wrappers which is parser-invasive (an earlier attempt
        //     broke tests/scripts/87-store-leaks.loft because
        //     conditional Sets to cv interact with paired_witness in
        //     ways that a blanket "substitute every Set(cv, Call)"
        //     doesn't handle correctly).
        let mut idx = last - 1;
        while idx > 0 {
            idx -= 1;
            if matches!(l[idx], Value::Line(_)) {
                continue;
            }
            let earlier = l[idx].unspan_mut();
            let Value::Set(eslot, erhs) = earlier else {
                break;
            };
            if *eslot != cv {
                break;
            }
            let erhs_inner = erhs.unspan_mut();
            let Value::Call(efn, eargs) = erhs_inner else {
                break;
            };
            let efn = *efn;
            let ehidden_idx = {
                let def = self.data.def(efn);
                def.attributes().iter().enumerate().find_map(|(i, a)| {
                    if !a.hidden {
                        return None;
                    }
                    if !matches!(
                        &a.typedef,
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    ) {
                        return None;
                    }
                    Some(i)
                })
            };
            let Some(ei) = ehidden_idx else { break };
            if eargs.len() <= ei {
                break;
            }
            let ework_ref = match eargs[ei].unspan() {
                Value::Var(v) => *v,
                _ => break,
            };
            if ework_ref == cv {
                continue;
            }
            let enm = self.vars.name(ework_ref);
            if !enm.starts_with("__ref_") && !enm.starts_with("__rref_") {
                break;
            }
            for a in eargs.iter_mut() {
                Self::substitute_work_ref(a, ework_ref, cv);
            }
            collapsed_refs.push(ework_ref);
        }

        self.suppress_collapsed_workrefs(l, collapsed_refs);
    }

    /// A work-ref collapsed onto `cv` (its inner call now delivers into `cv`
    /// directly) no longer owns a store, so suppress its eager buffer
    /// allocation.  For a VECTOR work-ref `skip_free` flips
    /// `gen_set_first_vector_null` to the no-alloc `OpInitRefSentinel` path and
    /// tells scope analysis there is nothing to free — closing the
    /// adopt-and-re-return leak (#462) at its source instead of minting then
    /// orphaning a store.
    ///
    /// VECTOR-only on purpose: a Reference/struct-Enum work-ref is initialised
    /// through `gen_set_first_ref_*` (a deep-COPY path, not the sentinel
    /// branch), where its store is genuinely live and already freed balanced by
    /// scope analysis (the @P377 `render_struct(p) -> Canvas` shape).  Marking
    /// THAT skip_free both skips a real alloc and suppresses a real free →
    /// leak.  Skip a work-ref that still has any live use (a defensive guard; a
    /// freshly-minted call buffer never does).
    fn suppress_collapsed_workrefs(&mut self, l: &[Value], refs: Vec<u16>) {
        for w in refs {
            if matches!(self.vars.tp(w), Type::Vector(_, _)) && !Self::ir_var_has_live_use(l, w) {
                self.vars.set_skip_free(w);
            }
        }
    }

    /// General NRVO collapse for the "adopt → mutate in place → return" shape
    /// (`t = base(); t += …; t` — the @PLN85 cluster-462 merge / `game_items()`
    /// case, #462).  The penultimate-op fast path in `nrvo_collapse_tail_set`
    /// misses it because the call defining `cv` is followed by in-place
    /// mutation before the tail.  When `cv` has EXACTLY ONE defining
    /// `Set(cv, Call(fn, [..__ref_N..]))` and it sits at the TOP LEVEL of the
    /// body, redirect that call's hidden buffer arg onto `cv` (the promoted
    /// retbuf) so the inner call delivers there directly — no orphaned buffer.
    /// Returns the collapsed work-ref(s) for the caller's skip_free pass.
    ///
    /// Guarded against the conditional-reassign hazard the penultimate chain
    /// documents (a blanket substitution broke `87-store-leaks.loft`): a SECOND
    /// defining `Set(cv, Call)` ANYWHERE — including nested in an `if`/loop arm
    /// — means `cv` may be conditionally re-defined, so leave it untouched.  A
    /// sole defining call that is itself nested (assigned only inside a branch)
    /// is also skipped: redirecting a conditionally-run delivery into the
    /// retbuf is unsound.
    fn nrvo_collapse_defining_call(&self, l: &mut [Value], cv: u16) -> Vec<u16> {
        // VECTOR returns only.  The adopt-then-mutate-then-return shape is
        // proven safe for a vector tail (`t = base(); t += …; t`): the in-place
        // mutations append, never re-own.  A STRUCT (Reference/Enum) tail of the
        // same syntactic shape (`rs = new(); rs.f = …; rs`) instead OVERWRITES
        // owned fields after the defining call, and redirecting that call into
        // the retbuf perturbs the field-overwrite free ordering (a small Sim
        // field leak in the crawler).  That case is a separate, harder shape —
        // left on its existing (correct) delivery path until proven.
        if !matches!(self.vars.tp(cv), Type::Vector(_, _)) {
            return Vec::new();
        }
        // Count EVERY assignment to `cv` (`Set(cv, _)` with ANY rhs, incl.
        // nested in `if`/loop arms), and note the FIRST top-level buffer-call
        // assignment.  `cv` is eligible ONLY when it is assigned exactly once —
        // its defining buffer call — and that lone assignment is at the top
        // level.  A second assignment anywhere means `cv` is conditionally
        // re-defined (`best = mon_none(); … if … { best = cand } … best`, where
        // `cand` is a borrowed view): redirecting the first call into the retbuf
        // and freeing nothing would orphan the buffer the later value never
        // wrote.  Counting ALL assignments (not just call-assignments) is what
        // separates the safe merge shape from this hazard — a `cv = view`
        // re-define is a `Set(cv, Var)`, invisible to a call-only count.
        let mut assigns = 0usize;
        let mut top_level_idx: Option<usize> = None;
        for (idx, op) in l.iter().enumerate() {
            assigns += Self::count_cv_assignments(op, cv);
            if top_level_idx.is_none() && self.buffer_call_workref(op, cv).is_some() {
                top_level_idx = Some(idx);
            }
        }
        if assigns != 1 {
            return Vec::new();
        }
        let Some(idx) = top_level_idx else {
            // The sole assignment is not a top-level buffer call (a bare view
            // bind, or nested in a branch) — unsafe / nothing to redirect.
            return Vec::new();
        };
        // Re-resolve the work-ref against the (mutable) node, then substitute.
        let Some(work_ref) = self.buffer_call_workref(&l[idx], cv) else {
            return Vec::new();
        };
        if let Value::Set(_, rhs) = l[idx].unspan_mut()
            && let Value::Call(_, args) = rhs.unspan_mut()
        {
            for a in args.iter_mut() {
                Self::substitute_work_ref(a, work_ref, cv);
            }
            vec![work_ref]
        } else {
            Vec::new()
        }
    }

    /// Recursively count assignments to slot `cv` — every `Set(cv, _)` /
    /// `TuplePut(cv, …)`, with ANY right-hand side, anywhere in `node`
    /// (including nested `if`/loop arms).  In-place mutation of `cv` (vector
    /// append, struct field write) is NOT an assignment to `cv`'s slot, so it
    /// is correctly not counted.  `nrvo_collapse_defining_call` uses this to
    /// fire only when `cv` is assigned exactly once.
    fn count_cv_assignments(node: &Value, cv: u16) -> usize {
        let here = matches!(node.unspan(), Value::Set(s, _) | Value::TuplePut(s, _, _) if *s == cv);
        let children = match node {
            Value::Set(_, b)
            | Value::TuplePut(_, _, b)
            | Value::Return(b)
            | Value::Drop(b)
            | Value::Yield(b) => Self::count_cv_assignments(b, cv),
            Value::Span(b) => Self::count_cv_assignments(&b.1, cv),
            Value::Call(_, a)
            | Value::CallRef(_, a)
            | Value::Insert(a)
            | Value::Tuple(a)
            | Value::Parallel(a) => a.iter().map(|x| Self::count_cv_assignments(x, cv)).sum(),
            Value::Block(bl) | Value::Loop(bl) => bl
                .operators
                .iter()
                .map(|o| Self::count_cv_assignments(o, cv))
                .sum(),
            Value::If(c, t, f) => {
                Self::count_cv_assignments(c, cv)
                    + Self::count_cv_assignments(t, cv)
                    + Self::count_cv_assignments(f, cv)
            }
            Value::Iter(_, a, b, c) => {
                Self::count_cv_assignments(a, cv)
                    + Self::count_cv_assignments(b, cv)
                    + Self::count_cv_assignments(c, cv)
            }
            _ => 0,
        };
        usize::from(here) + children
    }

    /// Read-only probe: if `node` is `Set(cv, Call(fn, args))` whose callee has
    /// a hidden heap buffer attribute filled by a parser-internal
    /// `__ref_N`/`__rref_N` work-ref distinct from `cv`, return that work-ref.
    /// Mirrors steps (3)–(4) of `nrvo_collapse_tail_set`'s fast path without
    /// mutating, so it can drive both detection and the redirect.
    fn buffer_call_workref(&self, node: &Value, cv: u16) -> Option<u16> {
        let Value::Set(slot, rhs) = node.unspan() else {
            return None;
        };
        if *slot != cv {
            return None;
        }
        let Value::Call(fn_nr, args) = rhs.unspan() else {
            return None;
        };
        let def = self.data.def(*fn_nr);
        let i = def.attributes().iter().position(|a| {
            a.hidden
                && matches!(
                    &a.typedef,
                    Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                )
        })?;
        let work_ref = match args.get(i)?.unspan() {
            Value::Var(v) => *v,
            _ => return None,
        };
        if work_ref == cv {
            return None;
        }
        let nm = self.vars.name(work_ref);
        (nm.starts_with("__ref_") || nm.starts_with("__rref_")).then_some(work_ref)
    }

    /// True when `v` appears in `l` as anything other than its own
    /// null-init `Set(v, Null)` — i.e. a live read (`Var(v)`) or a real
    /// store-producing reassignment (`Set(v, non-null)`).  Used by
    /// `nrvo_collapse_tail_set` to confirm a collapsed work-ref is fully
    /// dead before suppressing its allocation/free (skipping the free of a
    /// still-live owned ref would itself leak).
    fn ir_var_has_live_use(l: &[Value], v: u16) -> bool {
        fn walk(val: &Value, v: u16) -> bool {
            match val {
                Value::Var(x) => *x == v,
                // `Set(v, Null)` is the work-ref's own init — not a live use;
                // any other `Set(v, …)` allocates/writes into it (live).
                Value::Set(slot, body) | Value::TuplePut(slot, _, body) => {
                    (*slot == v && !matches!(body.unspan(), Value::Null)) || walk(body, v)
                }
                Value::Return(b) | Value::Drop(b) | Value::Yield(b) => walk(b, v),
                Value::Span(b) => walk(&b.1, v),
                Value::Call(_, args)
                | Value::CallRef(_, args)
                | Value::Insert(args)
                | Value::Tuple(args)
                | Value::Parallel(args) => args.iter().any(|a| walk(a, v)),
                Value::Block(bl) | Value::Loop(bl) => bl.operators.iter().any(|op| walk(op, v)),
                Value::If(c, t, f) => walk(c, v) || walk(t, v) || walk(f, v),
                Value::Iter(_, a, b, c) => walk(a, v) || walk(b, v) || walk(c, v),
                _ => false,
            }
        }
        l.iter().any(|op| walk(op, v))
    }

    /// Walk past `Span` / `Return` wrappers to find a tail `Var(v)`.
    /// Used by `nrvo_collapse_tail_set` to recognise the two shapes the
    /// parser produces for "the body returns variable `v`".
    fn tail_var(v: &Value) -> Option<u16> {
        match v.unspan() {
            Value::Var(v) => Some(*v),
            Value::Return(inner) => Self::tail_var(inner),
            _ => None,
        }
    }

    pub(crate) fn rewrite_tail_tuple_to_synthetic_struct(
        &mut self,
        synthetic_d_nr: u32,
        tail: &mut Value,
    ) {
        // A7.1: allocate ONE shared work-ref up front, then descend
        // recursively through `If` / `Block` / `Insert` / `Span`
        // wrappers so every leaf `Value::Tuple` writes into the same
        // record.  Sharing avoids ref_return promoting two separate
        // hidden args (one per branch); the function then returns a
        // single work-ref whose value is well-defined at the join
        // point.  Mirrors the unification done by P236's
        // `unify_if_branches_work_refs` for struct returns.
        let synth_ref_type = Type::Reference(synthetic_d_nr, Deps::none());
        let w = self.vars.work_refs(&synth_ref_type, &mut self.lexer);
        let known_type = self.data.def(synthetic_d_nr).known_type();
        self.rewrite_tail_tuple_with_work_ref(synthetic_d_nr, known_type, w, tail);
    }

    pub(crate) fn rewrite_tail_tuple_with_work_ref(
        &mut self,
        synthetic_d_nr: u32,
        known_type: u16,
        w: u16,
        tail: &mut Value,
    ) {
        match tail {
            Value::Span(b) => {
                self.rewrite_tail_tuple_with_work_ref(synthetic_d_nr, known_type, w, &mut b.1);
                return;
            }
            Value::If(_, then_branch, else_branch) => {
                self.rewrite_tail_tuple_with_work_ref(synthetic_d_nr, known_type, w, then_branch);
                self.rewrite_tail_tuple_with_work_ref(synthetic_d_nr, known_type, w, else_branch);
                return;
            }
            Value::Block(b) => {
                if let Some(last) = b.operators.last_mut() {
                    self.rewrite_tail_tuple_with_work_ref(synthetic_d_nr, known_type, w, last);
                }
                b.result = Type::Reference(synthetic_d_nr, Deps::frame1(w));
                return;
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last_mut() {
                    self.rewrite_tail_tuple_with_work_ref(synthetic_d_nr, known_type, w, last);
                }
                return;
            }
            _ => {}
        }
        // loft#821 — a leaf that IS a stack tuple but is not written as one.  Reading it
        // element-by-element is what the literal path does too; `emit_tuple_set_ops`
        // stashes the source once and writes each element at the record's own offset.
        if let Value::Var(v) = tail.unspan()
            && let Type::Tuple(elems) = self.vars.tp(*v).base()
        {
            let elems = elems.clone();
            let src = tail.clone();
            let mut ops = vec![
                crate::data::v_set(w, Value::Null),
                self.cl(
                    "OpDatabase",
                    &[Value::Var(w), Value::Int(i32::from(known_type))],
                ),
            ];
            ops.extend(self.emit_tuple_set_ops(&Value::Var(w), 0, &elems, src));
            ops.push(Value::Var(w));
            *tail = crate::data::v_block(
                ops,
                Type::Reference(synthetic_d_nr, Deps::frame1(w)),
                "synthetic_tuple_return",
            );
            return;
        }
        let elements = match std::mem::replace(tail, Value::Null) {
            Value::Tuple(elems) => elems,
            other => {
                *tail = other;
                return;
            }
        };
        let mut ops: Vec<Value> = Vec::with_capacity(elements.len() + 3);
        ops.push(crate::data::v_set(w, Value::Null));
        ops.push(self.cl(
            "OpDatabase",
            &[Value::Var(w), Value::Int(i32::from(known_type))],
        ));
        for (i, elem) in elements.into_iter().enumerate() {
            // E1 (pre-existing, gate-OFF too): a bare-`null` tuple element must become the
            // field's TYPED null (`OpConvIntFromNull` → the i64 sentinel, etc.), exactly as
            // struct-field construction does. Raw `Value::Null` made native emit `()` into the
            // typed slot → E0308; interp tolerated it. `self.null` peels `Optional` to the base
            // sentinel, so a `τ?` element types correctly too.
            // A `null` for a nullable COLLECTION member is the reserved ABSENT id in the
            // slot, exactly as `H { xs: null }` writes it (loft#917) — `self.null(&ftp)`
            // appends nothing and leaves an EMPTY collection, so `(null, 2)` read back as
            // `[]` and `miss.0 == null` answered false, on both backends (QUALITY.md B7t).
            // The null arrives as a bare `Value::Null` from a concrete declaration and as the
            // TYPED null a template gave a `T?` element (`OpNullRefSentinel()`, since the
            // template compiled `T` as a record) from a monomorph — one absence, two
            // spellings, and the slot takes the same id for both.
            let is_null_elem = match elem.unspan() {
                Value::Null => true,
                Value::Call(d, args) if args.is_empty() => {
                    let n = self.data.def(*d).name();
                    n == "OpNullRefSentinel" || (n.starts_with("OpConv") && n.ends_with("FromNull"))
                }
                _ => false,
            };
            if is_null_elem
                && let stored = self.data.attr_type(synthetic_d_nr, i)
                && (matches!(stored.base(), Type::Vector(_, _))
                    || crate::parser::vectors::is_keyed(stored.base()))
                && (matches!(stored, Type::Optional(_))
                    || self.data.attr_nullable(synthetic_d_nr, i))
                && let Some(pos) = crate::data::stored_tuple_offsets_for_def(
                    &self.data,
                    &self.database,
                    synthetic_d_nr,
                    self.data.def(synthetic_d_nr).attributes().len(),
                )
                .and_then(|o| o.get(i).copied())
            {
                let mark = self.mark_collection_absent(&Value::Var(w), i32::from(pos));
                ops.push(mark);
                continue;
            }
            let elem = if matches!(elem.unspan(), Value::Null) {
                let ftp = self.data.attr_type(synthetic_d_nr, i).clone();
                self.null(&ftp)
            } else {
                elem
            };
            // loft#1109 — `set_field_no_check` copies a heap member INTO the record's own
            // storage, so the frame-local backing a tuple literal wraps the member in
            // (`tuple_member_copy`, loft#1102) is a copy this path immediately copies again.
            // Unwrapping to the source leaves exactly one copy and the same semantics: the
            // record still owns its member and the local still cannot alias it.
            let elem = match self.tuple_member_copy_source(&elem) {
                Some(src) => src,
                None => elem,
            };
            // A NULLABLE record member is a TAGGED slot (`__nullable<S>`: discriminant +
            // payload), and a dense `S` written into it by the plain field write lands on
            // the discriminant — presence becomes a data byte, and `(x, 1)` read back as
            // `4294967199` where `x.a` was `7`, on both backends (QUALITY.md B7t; the
            // loft#1134 shape, at the tuple return).  The tagged write the element-wise
            // path already uses (`tuple_elem_tag_write`) decides from the STORED type and
            // the member's own spelling, so ask it first and fall back to the field write.
            // The member's spelling is the ELEMENT's own static type, not the slot's: a
            // struct literal in a `S?` position is already lowered to the tagged
            // `__nullable<S>::Some` record (`#NullableSome`), and so is a nullable field
            // read (`#ncc`), and for those the plain copy IS the right write — wrapping one
            // a second time buried the real discriminant one payload deep and `(NvW { nv_n:
            // 7 }, 9)` read `2`, the tag, for `7` (tests 1123/1139 on the corpus census).
            // Only a dense pointer — a `S` or `S?` local, a parameter, a call — needs the tag.
            let tagged = match self.data.attr_type(synthetic_d_nr, i) {
                Type::Enum(syn, true, _) => self.nullable_payload_struct(syn).and_then(|payload| {
                    let in_slot_form =
                        |tp: &Type| matches!(tp.base(), Type::Enum(s, true, _) if *s == syn);
                    let already = match elem.unspan() {
                        Value::Var(v) => in_slot_form(self.vars.tp(*v)),
                        Value::Block(bl) => in_slot_form(&bl.result),
                        Value::Call(d, _) => in_slot_form(self.data.def(*d).returned()),
                        _ => false,
                    };
                    if already {
                        return None;
                    }
                    let spelled = Type::Optional(Box::new(Type::Reference(payload, Deps::none())));
                    let pos = crate::data::stored_tuple_offsets_for_def(
                        &self.data,
                        &self.database,
                        synthetic_d_nr,
                        self.data.def(synthetic_d_nr).attributes().len(),
                    )
                    .and_then(|o| o.get(i).copied())?;
                    self.tuple_elem_tag_write(
                        synthetic_d_nr,
                        i,
                        &Value::Var(w),
                        pos,
                        &spelled,
                        &elem,
                    )
                }),
                _ => None,
            };
            match tagged {
                Some(writes) => ops.extend(writes),
                None => {
                    ops.push(self.set_field_no_check(synthetic_d_nr, i, 0, Value::Var(w), elem));
                }
            }
        }
        ops.push(Value::Var(w));
        *tail = crate::data::v_block(
            ops,
            Type::Reference(synthetic_d_nr, Deps::frame1(w)),
            "synthetic_tuple_return",
        );
    }

    // <operator> ::= '..' ['='] |
    //                '||' | 'or' |
    //                '&&' | 'and' |
    //                '==' | '!=' | '<' | '<=' | '>' | '>=' |
    //                '|' |
    //                '^' |
    //                '&' |
    //                '<<' | '>>' |
    //                '-' | '+' |
    //                '*' | '/' | '%'
    // <operators> ::= <single>  { '.' <field> | '[' <index> ']' } | <operators> <operator> <operators>
    /// @PLN25 DN1 — does this branch value YIELD null at its tail? A bare `Value::Null`, or the
    /// typed-null sentinel a bare `null` lowers to when coerced to a scalar (`OpConv*FromNull`).
    /// Descends `Block`/`Insert`/`Span` to the tail. Used so an `if`/`match` whose branch is a
    /// bare null widens the result to `Optional(τ)` under DN1 (the absorbed-branch-null fix).
    ///
    /// A `Return`/`Drop` wrapper is deliberately NOT descended: the question is what this
    /// value hands to the JOIN, and a `return` hands it nothing — it leaves the function.
    /// The `scopes`-side siblings (`is_null_terminal`, `return_has_null_arm`) ask the same
    /// null question about a return EXPRESSION, where that wrapper is the subject rather
    /// than an escape, and pass through it.
    fn branch_yields_null(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Null => true,
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|o| self.branch_yields_null(o)),
            Value::Insert(ops) => ops.last().is_some_and(|o| self.branch_yields_null(o)),
            // A nested `if`/`else if` (and `match`, which lowers to nested `if`) yields null when
            // ANY arm does — descend both so `else if c { 6 } else { null }` widens the outer if.
            Value::If(_, then_b, else_b) => {
                self.branch_yields_null(then_b) || self.branch_yields_null(else_b)
            }
            Value::Call(d, args)
                if args.is_empty() && (*d as usize) < self.data.definitions.len() =>
            {
                let n = self.data.def(*d).name();
                n.starts_with("OpConv") && n.ends_with("FromNull")
            }
            _ => false,
        }
    }

    /// @PLN25 DN1 — like [`Self::branch_yields_null`] but does NOT descend into a nested
    /// `Value::If` (a lowered `match`/`if`). Used by the enum-`match` arm-widening: an arm
    /// whose value is a DIRECT bare null (`=> null`, lowered to `OpConv*FromNull`, or a block
    /// ending in one) must widen the match; but an arm whose value is a NESTED match carries
    /// its own nullability in `a.tp` (folded into `result_type` by the arm-join), and
    /// descending into its lowered chain would hit its synthesised unreachable
    /// `OpConv*FromNull` default and falsely widen (p54).
    ///
    /// A `Return`/`Drop` wrapper is deliberately NOT descended: the question is what this
    /// value hands to the JOIN, and a `return` hands it nothing — it leaves the function.
    /// The `scopes`-side siblings (`is_null_terminal`, `return_has_null_arm`) ask the same
    /// null question about a return EXPRESSION, where that wrapper is the subject rather
    /// than an escape, and pass through it.
    fn arm_yields_direct_null(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Null => true,
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|o| self.arm_yields_direct_null(o)),
            Value::Insert(ops) => ops.last().is_some_and(|o| self.arm_yields_direct_null(o)),
            Value::Call(d, args)
                if args.is_empty() && (*d as usize) < self.data.definitions.len() =>
            {
                let n = self.data.def(*d).name();
                n.starts_with("OpConv") && n.ends_with("FromNull")
            }
            _ => false,
        }
    }

    /// @PLN25 DN1 — widen a value's result type to `Optional(τ)` when its lowered `code` yields a
    /// bare null and `tp` is a non-null scalar. Safe ONLY where there is no SYNTHESISED unreachable
    /// `OpConv*FromNull` default (a scalar `match` whose `_` arm is user-written, an `if`); an enum
    /// `match` must instead inspect its USER `arms` (its exhaustive default is a false positive).
    fn dn1_widen_branch_null(&self, tp: Type, code: &Value) -> Type {
        if crate::keys::pln25_dn1_enabled()
            && Self::is_non_null_scalar(&tp)
            && self.branch_yields_null(code)
        {
            Type::optional(tp)
        } else {
            tp
        }
    }

    // @F27 — if / else as an expression
    /// @PLN25 DN3 flow-narrowing — read a non-null proof out of a parsed `if` condition.
    /// Returns `(var, non_null_in_then)`: `v != null` / `if v` (truthy) narrow `v` in the
    /// THEN branch (`true`); `v == null` narrows `v` in the ELSE branch (`false`). The null
    /// side of a comparison is any `OpConv*FromNull()` (the parser's typed-null lowering).
    fn narrowing_from_condition(&self, test: &Value) -> Option<(u16, bool)> {
        let Value::Call(op, args) = test.unspan() else {
            return None;
        };
        let name = self.data.def(*op).name();
        let is_null_conv = |a: &Value| {
            matches!(a.unspan(), Value::Call(c, ca)
                if ca.is_empty() && self.data.def(*c).name().ends_with("FromNull"))
        };
        // `v == null` / `v != null` (Var on either side of the null literal).
        if (name.starts_with("OpEq") || name.starts_with("OpNe")) && args.len() == 2 {
            let pair = match (args[0].unspan(), args[1].unspan()) {
                (Value::Var(v), other) | (other, Value::Var(v)) if is_null_conv(other) => Some(*v),
                _ => None,
            };
            if let Some(v) = pair {
                return Some((v, name.starts_with("OpNe")));
            }
        }
        // `if v` (truthy) — a bare nullable read converted to boolean → non-null in THEN.
        if name.starts_with("OpConvBoolFrom")
            && args.len() == 1
            && let Value::Var(v) = args[0].unspan()
        {
            return Some((*v, true));
        }
        // `if !v` — negated truthy: `!v` is the null/falsy test, so `v` is non-null on the
        // ELSE side (where `v` is truthy). This is the guard-clause condition `if !v { return }`
        // (#585): the THEN branch is the null case, and the fall-through (else) proves `v`
        // non-null. Mirrors `v == null` → non-null-in-ELSE.
        if name == "OpNot"
            && args.len() == 1
            && let Value::Call(inner_op, inner_args) = args[0].unspan()
            && self
                .data
                .def(*inner_op)
                .name()
                .starts_with("OpConvBoolFrom")
            && inner_args.len() == 1
            && let Value::Var(v) = inner_args[0].unspan()
        {
            return Some((*v, false));
        }
        None
    }

    /// @PLN25 DN3 fault-op — read a non-zero divisor proof out of a parsed `if` condition.
    /// Returns the var slot proven non-zero in the THEN branch by `if v != 0` (`v` on either
    /// side of a `0` literal). Only `!= 0` narrows (the else of `if v == 0` is a later slice).
    /// @PLN25 DN3 fault-op — read a "divisor `v` is non-zero" proof from an `if` condition, with
    /// the branch it holds in: `v != 0` proves it in the THEN branch (`Some((v, true))`); `v == 0`
    /// proves it in the ELSE branch (`Some((v, false))`) — the common `if b == 0 { … } else { a / b }`
    /// safe-division idiom. Mirrors `narrowing_from_condition`'s then/else convention.
    fn divisor_proof_from_condition(&self, test: &Value) -> Option<(u16, bool)> {
        let Value::Call(op, args) = test.unspan() else {
            return None;
        };
        let name = self.data.def(*op).name();
        let then_branch = if name.starts_with("OpNe") {
            true
        } else if name.starts_with("OpEq") {
            false
        } else {
            return None;
        };
        if args.len() != 2 {
            return None;
        }
        let is_zero = |a: &Value| matches!(a.unspan(), Value::Int(0) | Value::Long(0));
        match (args[0].unspan(), args[1].unspan()) {
            (Value::Var(v), other) | (other, Value::Var(v)) if is_zero(other) => {
                Some((*v, then_branch))
            }
            _ => None,
        }
    }

    pub(crate) fn parse_if(&mut self, code: &mut Value) -> Type {
        // loft#1382 — take statement position from the caller and CLEAR it, so a value-`if`
        // nested inside a statement one (`if c { v = if d { 1 } else { 2 } }`) does not
        // inherit it.  Held across the arms in `arms_of_statement_construct`, which is what
        // the arm-agreement gate reads, and restored at exit so a sibling construct in the
        // same statement is unaffected.  Cleared HERE and not in `parse_if_expecting`,
        // because an `else if` CHAIN recurses through that one and its arms belong to the
        // same statement construct.
        let is_stmt = std::mem::replace(&mut self.stmt_if_pending, false);
        let outer_arms = std::mem::replace(&mut self.arms_of_statement_construct, is_stmt);
        let r = self.parse_if_expecting(code, &Type::Unknown(0));
        self.arms_of_statement_construct = outer_arms;
        r
    }

    /// [`parse_if`](Self::parse_if) with the type its THEN arm is expected to answer in:
    /// `Unknown` for an `if` that names its own type — every one an author opens — and the
    /// enclosing then arm's type for an `else if` CHAIN, whose arms are else arms and convert
    /// at their tails exactly as a plain `else` block does (@FR-N-Decl, loft#1380).
    fn parse_if_expecting(&mut self, code: &mut Value, expected: &Type) -> Type {
        let mut test = Value::Null;
        // loft#986 — the `{` after this condition opens a BLOCK; an empty `{ }` must not
        // read as a struct literal here.
        let outer_head = self.in_control_head;
        self.in_control_head = true;
        let tp = self.expression(&mut test);
        self.in_control_head = outer_head;
        self.convert_condition(&mut test, &tp);
        // @PLN25 DN3: a non-null proof from the condition narrows the proven var inside the
        // matching branch (then for `!= null`/truthy, else for `== null`).
        let narrow = self.narrowing_from_condition(&test);
        let narrow_base = self.narrowed_non_null.len();
        if let Some((v, true)) = narrow {
            self.narrowed_non_null.push(v);
        }
        // @PLN25 DN3 fault-op: `if v != 0` proves the divisor `v` non-zero in the THEN branch
        // (and `if v == 0 … else` in the ELSE branch), so `a / v` / `a % v` there is provably fit
        // (types non-null). The THEN proof is pushed now; the ELSE proof is pushed below.
        let divisor = self.divisor_proof_from_condition(&test);
        let divisor_base = self.divisor_nonzero.len();
        if let Some((v, true)) = divisor {
            self.divisor_nonzero.push(v);
        }
        // @PLN25 DN3 fault-op (index): `if idx < len(vec) { … }` proves `vec[idx]` in-bounds in the
        // THEN branch (skip-pattern 5) — reuse the warning walk's guard-pair extractor. THEN-only
        // (the else side has idx >= len, no fit). The len-capture form (`n = len(v); if idx < n`)
        // needs the capture map, deferred — pass an empty map, so inline `len(vec)` guards match.
        let empty_caps = std::collections::HashMap::new();
        let index_base = self.index_bounded.len();
        let index_pairs =
            crate::parser::operators::collect_guard_pairs(&test, &self.data, &empty_caps);
        self.index_bounded.extend(index_pairs);
        let is_aliases: Vec<(String, Option<u16>)> = std::mem::take(&mut self.is_capture_aliases);
        let is_bindings: Vec<Value> = std::mem::take(&mut self.is_capture_bindings);
        let mut true_code = Value::Null;
        let write_state = self.vars.save_and_clear_write_state();
        self.vars.clear_write_state();
        let mut true_type = self.parse_block("if", &mut true_code, expected);
        if !is_bindings.is_empty()
            && let Value::Block(bl) = &mut true_code
        {
            let mut new_ops = is_bindings;
            new_ops.append(&mut bl.operators);
            bl.operators = new_ops;
        }
        for (name, old) in &is_aliases {
            if let Some(old_nr) = old {
                self.vars.set_name(name, *old_nr);
            } else {
                self.vars.remove_name(name);
            }
        }
        // @PLN25 DN3: leave the then-branch — drop its narrowing; the ELSE gets the `== null`
        // proof (the var is non-null on the else side of `if v == null { … } else { … }`).
        self.narrowed_non_null.truncate(narrow_base);
        // Leaving the THEN branch, drop its `!= 0` divisor proof; an `== 0` condition instead
        // proves the divisor non-zero on the ELSE side, pushed just below with the else narrowing.
        self.divisor_nonzero.truncate(divisor_base);
        // Leaving the THEN branch — drop its `idx < len(vec)` in-bounds proofs (THEN-only).
        self.index_bounded.truncate(index_base);
        if let Some((v, false)) = narrow {
            self.narrowed_non_null.push(v);
        }
        if let Some((v, false)) = divisor {
            self.divisor_nonzero.push(v);
        }
        let mut false_type = Type::Void;
        let mut false_code = Value::Null;
        // What an `else if` CHAIN borrows, when its type is not adopted as `false_type`.
        let mut chain_borrow: Option<Type> = None;
        // @PLN25 DN1: whether there is a REAL user `else`. An if-WITHOUT-else in value position is
        // already an error and synthesises a `null` else for recovery; the DN1 widening below must
        // NOT treat that synthesised null as a nullable branch (it would add a spurious `τ?`).
        let had_else = self.lexer.has_token("else");
        if had_else {
            self.vars.restore_write_state(&write_state);
            self.vars.clear_write_state();
            // A bare-`null` THEN arm has no type of its own — it adopts the sibling's,
            // whatever the sibling turns out to be.  Marked BEFORE the sibling is
            // parsed, because parsing it is what makes that type available.
            if matches!(true_type, Type::Null | Type::Never) {
                true_type = Type::Unknown(0);
            }
            if self.lexer.has_token("if") {
                // loft#936 — an `else if` CHAIN is a sibling like any other.  Its
                // result type used to be discarded here, so `if a { null } else if b
                // { null } else { [n] }` left the whole chain typed by its untyped
                // first arm and answered `null` for EVERY input — the value arm was
                // unreachable, on both backends and on released 2026.8.0.  A THEN arm
                // that already names the merged type keeps `false_type` at `Void`
                // exactly as before; nothing downstream reads the chain's type there.
                // @FR-N-Decl — the chain's arms are ELSE arms, and an else arm answers in
                // the then arm's type or is refused, at its own tail, through the same
                // `parse_block` conversion the plain `else` block below takes.  The chain
                // used to be parsed expecting nothing, so its value was never held to the
                // type this expression reports: `if a { 1 } else if b { 2.5 } else { 3 }`
                // into an `integer` read the float's bits as a number, and `if a { p } else
                // if b { p + q } else { q }` put 260 into a `u8` — a local, an argument and
                // a return alike, both backends (loft#1380).  A statement `if` (a `Void`
                // then arm) expects nothing of its chain, as before; a then arm that names
                // no type yet (`null`, a return) adopts the chain's, as before.
                let chain_expected = if matches!(true_type, Type::Void) {
                    Type::Unknown(0)
                } else {
                    true_type.clone()
                };
                let chain_type = self.parse_if_expecting(&mut false_code, &chain_expected);
                if true_type == Type::Unknown(0) {
                    false_type = chain_type;
                } else {
                    // @FR-C-Var — a chain whose arms are OTHER variants of the then arm's
                    // enum joins to that enum, exactly as the plain else below does; the
                    // inner `if` has already joined its own two arms, so the chain arrives
                    // as the enum or as one sibling variant.
                    let variant_enum = self.variant_parent_enum(&true_type);
                    if let Some(enum_tp) = &variant_enum
                        && self.joins_to_enum(enum_tp, &true_type, &chain_type)
                    {
                        true_type = enum_tp.clone();
                    }
                    // loft#978 — the chain's TYPE deliberately stays out of `false_type`
                    // (above), but what it BORROWS is still a value this if-expression can
                    // deliver, so it has to reach the join below.  Without it an
                    // `else if` arm yielding a container view was erased by a plain
                    // `if` arm yielding a fresh record, and the local read as owned.
                    chain_borrow = Some(chain_type);
                }
            } else {
                // @FR-C-Var — an `else` arm is checked against the ENUM, not against its
                // SIBLING.  `types.md (C-Var)` licenses `Reference(S) ⤳ Enum(E)` for
                // `S ∈ variants(E)` and licenses nothing between two variants, so handing
                // the then-arm's type down asked a question the rules do not answer:
                // `if c { E::A { … } } else { E::B { … } }` was refused *"expected A, got B
                // on else"* while `match` — which lowers to this very node — accepted the
                // identical join (loft#1117).  `match` gets this right by expecting
                // `Unknown` per arm; an `if` cannot, because its else arm needs the
                // sibling's type for `null` / `[]` / a bare variant name.  The enum is the
                // type that serves both: it is what the arms actually join to, and it still
                // carries the context those spellings need.
                let variant_enum = self.variant_parent_enum(&true_type);
                false_type = self.parse_block("else", &mut false_code, &true_type);
                // @FR-C-Var — two DIFFERENT variants of one enum join to the ENUM, and
                // that is this expression's type.  `parse_block` accepted the sibling arm
                // and kept its own type (see its `arm_joins_to_enum` carve-out); deciding
                // the join is this site's half, because only here are both arms in hand.
                //
                // The widening is what keeps the acceptance sound.  Left at the then-arm's
                // variant, `v: A = if c { E::A { … } } else { E::B { … } }` would be
                // accepted and a slot declared as one variant would hold another, read at
                // this variant's offsets (loft#980's class).  Two arms of the SAME variant
                // widen nothing, so a variant-typed destination stays legal for them.
                if let Some(enum_tp) = &variant_enum
                    && self.joins_to_enum(enum_tp, &true_type, &false_type)
                {
                    true_type = enum_tp.clone();
                }
                // ...and when the arms really are two DIFFERENT variants, the join is the
                // ENUM, so that is this expression's type.  Keeping the then-arm's variant
                // would let `v: A = if c { E::A { … } } else { E::B { … } }` through — a
                // slot declared as one variant holding another, whose fields are read at
                // this variant's offsets (loft#980's class).  `(C-Var)` converts the enum
                // to nothing narrower, so the declaration is refused where it belongs.
                //
                // Two arms of the SAME variant keep that variant: nothing was widened, and
                // a `v: A` destination stays legal for them.
                if let Some(enum_tp) = &variant_enum
                    && self.joins_to_enum(enum_tp, &true_type, &false_type)
                {
                    true_type = enum_tp.clone();
                }
            }
            if true_type == Type::Unknown(0) {
                if let Value::Block(bl) = &mut true_code {
                    let p = bl.operators.len() - 1;
                    if !is_block_divergent(&bl.operators) {
                        // loft#936 — `null_value`, not `null`: the arm's null travels
                        // to the merge on the eval stack, so a COLLECTION arm needs the
                        // DbRef sentinel.  `null`'s catch-all answers a bare
                        // `Value::Null`, which pushes nothing, and the join then read an
                        // uninitialised 12-byte slot as a live reference.
                        bl.operators[p] = self.null_value(&false_type);
                    }
                    bl.result = false_type.clone();
                }
                true_type = false_type.clone();
            }
        } else {
            self.vars.restore_write_state(&write_state);
            if !matches!(true_type, Type::Void | Type::Never) {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "If-expression produces a value but has no else clause; add an else branch or make the body a statement"
                    );
                }
                false_code = v_block(vec![self.null_value(&true_type)], true_type.clone(), "else");
            }
        }
        self.vars.restore_write_state(&write_state);
        // @PLN25 DN3: both branches parsed — drop any narrowing back to the enclosing level
        // (the proof holds only inside the if/else, not after it).
        self.narrowed_non_null.truncate(narrow_base);
        self.divisor_nonzero.truncate(divisor_base);
        // Belt-and-suspenders: `index_bounded` was already restored after the THEN block (it is
        // THEN-only, no else-push), so this is a no-op today — kept for parity with the two
        // narrowings above and to stay correct if an else-side in-bounds proof is added later.
        self.index_bounded.truncate(index_base);
        // @PLN25 DN1: a branch that yields a bare `null` makes the if-expression NULLABLE — its
        // result widens to `Optional(τ)` where τ is the non-null SCALAR sibling's type. The bare
        // null is otherwise coerced to the sibling's typed-null sentinel and the if types as a
        // plain `τ`, hiding the nullability; widening here lets the existing DN3 `(N-Store)` force
        // the caller to declare `τ?` or discharge. Only fires when exactly one branch yields null
        // and the other is a non-null scalar (heap types stay nullable; both-null stays as-is).
        let mut result_tp = merge_dependencies(&true_type, &false_type);
        if let Some(chain) = &chain_borrow {
            result_tp = result_tp.joined_deps(chain);
        }
        // loft#1103 — `(N-Join)`: the join is OPTIONAL iff SOME arm is.  `merge_dependencies`
        // takes the THEN arm's shape and merges only the deps, so a nullable arm on the other
        // side was joined away: `x: integer = if c { 1 } else { maybe(k) }` typed the join
        // `integer`, the destination's `(N-Store)` teeth had nothing to bite, and a declared
        // non-null slot held null with no diagnostic on either backend — including `u8`, where
        // `(N-Store)` is a hard ERROR precisely because the null collides with a real value.
        //
        // The `null` LITERAL in the same position was always caught, by the DN1 walkers just
        // below: they match the `OpConv*FromNull` node the literal lowers to.  A nullable-TYPED
        // value produces no null-shaped node at all, so nothing asked about it — one notion, two
        // spellings, one of them looked for.  This asks the TYPE, which is the spelling-free
        // form of the same question, and it runs BEFORE the DN1 block so neither double-wraps.
        //
        // Widening here rather than at the arm keeps `(N-Decl)` / `(N-Store)` as the things that
        // REPORT: the destination compares its declaration against an honest join and speaks for
        // itself, so every store site — a declared local, a field, a return — is covered by the
        // teeth it already had, and a `τ?` destination stays legal and silent.
        //
        // An `else if` CHAIN counts as an arm here, because it IS one — loft#936's words, and
        // its type reaches this point in `chain_borrow` rather than in `false_type` (the chain
        // deliberately keeps its SHAPE out of the join; only what it borrows was being read).
        // Left out, `if a { 1 } else if b { 2 } else { maybe(k) }` and every `match` with more
        // than two arms stayed silent, since a `match` lowers to exactly this nesting.
        if had_else
            && crate::keys::pln25_dn1_enabled()
            && !matches!(result_tp, Type::Optional(_))
            && (matches!(true_type, Type::Optional(_))
                || matches!(false_type, Type::Optional(_))
                || chain_borrow
                    .as_ref()
                    .is_some_and(|c| matches!(c, Type::Optional(_))))
        {
            result_tp = Type::optional(result_tp);
        }
        if had_else && crate::keys::pln25_dn1_enabled() && !matches!(result_tp, Type::Optional(_)) {
            let t_null = self.branch_yields_null(&true_code);
            let f_null = self.branch_yields_null(&false_code);
            if t_null != f_null {
                let other = if t_null { &false_type } else { &true_type };
                if Self::is_non_null_scalar(other) {
                    result_tp = Type::optional(other.clone());
                }
            }
        }
        *code = v_if(test, true_code, false_code);
        // loft#1019 — an arm that OWNS what it yields needs a home in this frame when
        // the merged type is a view (`Parser::own_joined_call_arms`).
        self.own_joined_call_arms(code, &result_tp);
        result_tp
    }

    /// @PLN85 match_return (LOFT_JOIN_OWN): an arm that yields a borrowed-view vector
    /// FIELD BINDING directly (`Filled { items } => { items }`) returns a view into the
    /// match subject — which the caller cannot free without an over-free. Wrap the yield
    /// in an OWNED copy `{ o = []; o += items; o }` (a fresh local `o`), so the value
    /// escapes OWNED. This is exactly the `deliver3` shape: the existing `ref_return`
    /// promotion then promotes `o` to the buffer arg + emits the `__retbuf` marker, the
    /// separate-buffer ABI the caller adopts (and so frees the argument). Done at PARSE
    /// time (re-parsed each pass) so `create_unique`/`vector_db` stay pass-consistent.
    /// Returns the new OWNED arm type when it rewrote (for cross-arm unification), else
    /// `None`. No-op unless the tail is a `skip_free`, non-empty-dep vector binding.
    #[allow(clippy::question_mark)]
    fn jo_copy_borrowed_arm_yield(&mut self, arm_body: &mut Value) -> Option<Type> {
        if !crate::keys::join_own_enabled() {
            return None;
        }
        let v = match arm_body.unspan() {
            Value::Var(v) => *v,
            Value::Block(bl) => match bl.operators.last().map(Value::unspan) {
                Some(Value::Var(v)) => *v,
                _ => return None,
            },
            _ => return None,
        };
        // @FR-O-Proxy asks copy — a non-empty dep list is what marks the arm's yield as a
        // BORROW, and the answer chooses whether to build the owned `mvcopy`.  It authorises
        // no free: the copy is a fresh binding, and the borrow keeps its own owner.
        if v >= self.vars.count()
            || !self.vars.skip_free(v)
            || !matches!(self.vars.tp(v), Type::Vector(_, _))
            || self.vars.tp(v).depend().is_empty()
        {
            return None;
        }
        let v_type = self.vars.tp(v).clone();
        let elm = match &v_type {
            Type::Vector(b, _) => (**b).clone(),
            _ => return None,
        };
        // Create `o` with the OWNED element type (no deps) — NOT `v_type`, which is the
        // binding's `vector<ref(E)>["e"]`; inheriting that `["e"]` would mark the copy as
        // borrowing `e` and re-propagate it to the return (the leak). `deliver3`'s `o` is
        // dep-free.
        let owned_create = Type::Vector(Box::new(elm.clone()), Deps::none());
        let o = self.create_unique("mvcopy", &owned_create);
        if o == u16::MAX {
            return None;
        }
        self.vars.defined(o);
        // `o = []` (pass-gated: empty on pass 1, the OpDatabase alloc on pass 2 — exactly
        // as a user-written `o: vector = []` lowers), then `o += <binding>`, then yield o.
        let mut ops = self.vector_db(&v_type, o);
        // `o = []` is a REPLACE, and `o` becomes the caller's return buffer once
        // `ref_return` promotes it — at which point `vector_db` above no-ops (an argument
        // keeps the caller's store) and the append below piles this call's elements onto
        // the LAST call's.  A two-borrowed-arm match returned 3, then 6, then 9 elements
        // from the same subject.  It went unnoticed while some OTHER arm cleared the
        // buffer as a side effect of its own `= []` and the clear got hoisted to entry —
        // so the bug appeared and vanished with the shape of an unrelated arm.  Clearing
        // here says it once, where the replace is: a no-op on a fresh store, correct on a
        // reused one.  Same reasoning as the `= <literal>` buffer clear in
        // `create_vector` (@PLN85 #492).
        ops.push(self.cl("OpClearVector", &[Value::Var(o)]));
        let elem_tp = self.append_elem_tp(&elm);
        ops.push(self.cl(
            "OpAppendVector",
            &[Value::Var(o), Value::Var(v), Value::Int(elem_tp)],
        ));
        ops.push(Value::Var(o));
        let owned_tp = Type::Vector(Box::new(elm), Deps::frame1(o));
        let copy_block = crate::data::v_block(ops, owned_tp.clone(), "jo_arm_copy");
        // Replace the WHOLE arm body (a bare `{ items }` yield) with the owned copy
        // block, NOT just its last op — wrapping inside leaves the outer block typed by
        // the borrowed binding (`["_mv_items_1"]`), which re-propagates the `["e"]` dep
        // to the return. Only the single-yield shape (the match-field binding IS the arm
        // value) is rewritten; a multi-statement arm is left alone.
        let n = arm_body.unspan_mut();
        match n {
            Value::Var(_) => *n = copy_block,
            Value::Block(bl) if bl.operators.len() == 1 => *n = copy_block,
            _ => return None,
        }
        Some(owned_tp)
    }

    // <match> ::= 'match' <expression> '{' { <pattern> '=>' <expression> } '}'
    // <pattern> ::= '_' | <variant> [ '{' <field> { ',' <field> } '}' ]
    #[allow(clippy::too_many_lines)]
    // @F29 — pattern matching (enum/scalar/tuple, guards, or-patterns, exhaustiveness)
    pub(crate) fn parse_match(&mut self, code: &mut Value) -> Type {
        // loft#1382 / loft#1386 — statement position comes from the caller (`parse_block`'s
        // loop is what sees the `;`), and the void-arm fact is scoped to THIS match so a
        // nested one cannot leak into its parent's verdict.
        let is_stmt = std::mem::replace(&mut self.stmt_if_pending, false);
        let outer_arms = std::mem::replace(&mut self.arms_of_statement_construct, is_stmt);
        let outer_void = std::mem::replace(&mut self.match_void_arm, false);
        let r = self.parse_match_inner(code);
        // @FR-F-Block discards a STATEMENT's arms, so a void one there is no defect.  In
        // VALUE position the path that ran yields nothing, and the exemption let the match
        // take the other arms' type: `v = match k { 1 => { 5 }, _ => { println(…) } }`
        // answered `v = null` with nothing said, where the `if` twin is refused (loft#1386).
        if !self.first_pass
            && !self.arms_of_statement_construct
            && self.match_void_arm
            && !matches!(r, Type::Void | Type::Null | Type::Never)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "expected {}, got void on a match arm — this `match` is used as a VALUE, so \
                 every arm has to produce one; give the arm a value, or make the `match` a \
                 statement by ending it with `;`",
                r.name(&self.data),
            );
        }
        self.match_void_arm = outer_void;
        self.arms_of_statement_construct = outer_arms;
        r
    }

    fn parse_match_inner(&mut self, code: &mut Value) -> Type {
        // One charge for the whole construct — arm count is not complexity (a 12-arm flat
        // dispatch reads straight down); its arms deepen via `parse_block("match_arm")`.
        if !self.first_pass {
            *self.complexity.entry(self.context).or_insert(0) += 1 + self.cc_nest;
        }
        // Save position of the match keyword for exhaustiveness diagnostics.
        let match_pos = self.lexer.pos().clone();
        // 1. Parse the subject expression.
        let mut subject = Value::Null;
        let subject_type = self.expression(&mut subject);
        // @PLN25: a `τ?` subject matches as its base (shared sentinel storage) — peel the marker
        // so `match` on an `integer?` routes to the scalar handler instead of falling to the `_`
        // arm ("match requires an enum, struct, or scalar type"). Gate-OFF inert (never Optional).
        let subject_type = subject_type.base().clone();
        // A subject whose type is not linked yet on the FIRST pass — `match p { … }` where
        // `p` came from an enum declared LOWER in the file.  The dispatch below cannot
        // recognise it, so the arms contribute nothing and `result_type` would stay `Void`;
        // a local bound to the match then locks to void and pass 2's real type is REFUSED
        // ("cannot change type from void to integer").  LOFT.md § File structure promises a
        // file may hold its declarations "in any order", so this must stay re-typeable —
        // the same first-pass escape `call_op` takes for an unresolved operand.
        let subject_unresolved = self.first_pass && subject_type.is_unknown();

        // @PLN35 PC1 — a CURSOR subject (a struct with a `vector<T>` source field + an integer
        // `pos` field) PREFIX-consumes: route to the vector-match over its source with cursor mode
        // on, so the arm pattern reads/advances relative to `pos`.  A plain struct without that
        // shape falls through to the normal struct handler.  Routes in BOTH passes so the arm
        // grammar (slice patterns, not struct patterns) is parsed consistently.
        if let Some((cursor_def, source_field, pos_field, celm_tp)) =
            self.cursor_shape(&subject_type)
        {
            return self.parse_cursor_match(
                subject,
                cursor_def,
                source_field,
                pos_field,
                &celm_tp,
                code,
            );
        }

        // Resolve type info from the subject.
        // Accepts: plain enums, struct-enums, struct-enum variants, and plain structs (T1-18).
        let (e_nr, is_struct, valid_enum, is_plain_struct) = match &subject_type {
            Type::Enum(nr, s, _) => (*nr, *s, true, false),
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::EnumValue => {
                let parent = self.data.def(*d_nr).parent();
                (parent, true, true, false)
            }
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::Enum => {
                // iterating a `vector<StructEnum>` yields loop variables
                // typed `Type::Reference(enum_def, _)` (via `for_type` in this
                // file, line 1952 — struct-enums degrade to a reference type
                // when carried through generic collections).  Without this
                // arm, matching a for-loop variable over a struct-enum vector
                // dropped into the error branch and every arm produced
                // 'Expect token }' cascades.
                (*d_nr, true, true, false)
            }
            Type::Reference(d_nr, _) if self.data.def_type(*d_nr) == DefType::Struct => {
                (*d_nr, true, true, true)
            }
            // scalar types — dispatch to scalar match handler.
            Type::Integer(_)
            | Type::Float
            | Type::Single
            | Type::Boolean
            | Type::Character
            | Type::Text(_) => {
                // @PLN25 DN1: a scalar match's `_` arm is USER-written (no synthesised default),
                // so the value-level widen is safe — a `_ => null` arm makes the match nullable.
                let tp = self.parse_scalar_match(subject, &subject_type, code);
                return self.dn1_widen_branch_null(tp, code);
            }
            // vector types — dispatch to vector match handler.
            Type::Vector(_, _) => {
                return self.parse_vector_match(subject, &subject_type, code);
            }
            // @PLN35 Phase 7 (Path B) — a coroutine `iterator<T>` subject: materialise it into a
            // fresh buffer `vector<T>` (eager pull), then run the SAME vector-match over the buffer.
            // Streaming stays entirely behind the Cursor seam; the match logic is untouched.
            Type::Iterator(elem_box, _) => {
                let elm_tp = (**elem_box).clone();
                let iter_tp = subject_type.clone();
                // Supported element types: scalars, `text`, and struct-enums (the token-stream
                // cases).  A plain enum / vector / tuple / struct element rides a different
                // coroutine `next` channel or append shape — deferred; a clean error points at the
                // collect idiom that works today.
                if !self.first_pass
                    && !matches!(
                        elm_tp.base(),
                        Type::Integer(_)
                            | Type::Float
                            | Type::Boolean
                            | Type::Character
                            | Type::Single
                            | Type::Text(_)
                            | Type::Enum(_, true, _)
                    )
                {
                    let en = elm_tp.name(&self.data);
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "streaming `match` over an `iterator<{en}>` is not yet supported (only scalar, text, or struct-enum element types) — collect it first: `match [for x in <iter> {{ x }}] {{ … }}`"
                    );
                }
                let (buf, vec_tp, setup) =
                    self.collect_iterator_subject(subject, &iter_tp, &elm_tp);
                let mut match_code = Value::Null;
                let result_tp = self.parse_vector_match(Value::Var(buf), &vec_tp, &mut match_code);
                let mut ops = setup;
                ops.push(match_code);
                *code = v_block(ops, result_tp.clone(), "stream match");
                return result_tp;
            }
            // T1.9: tuple types — dispatch to tuple match handler.
            Type::Tuple(_) => {
                return self.parse_tuple_match(subject, &subject_type, code);
            }
            _ => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "match requires an enum, struct, or scalar type"
                    );
                }
                (u32::MAX, false, false, false)
            }
        };

        // For plain enums (stack bytes), use a temp var to avoid re-evaluating the subject.
        // For struct enums (database references / DbRef), do NOT create a temp var — the
        // allocation system requires DbRefs to be freed in strict LIFO order and copying them
        // to a new variable breaks that invariant.  Instead, use the subject Value directly.
        let (subject_val, preamble): (Value, Option<(u16, Value)>) = if is_struct || !valid_enum {
            (subject, None)
        } else {
            let v = self.create_unique("match_subj", &subject_type);
            self.vars.defined(v);
            (Value::Var(v), Some((v, subject)))
        };

        // Build discriminant expression: integer representation of the active variant.
        let disc_expr = if is_struct {
            let get_enum = self.cl("OpGetEnum", &[subject_val.clone(), Value::Int(0)]);
            self.cl("OpConvIntFromEnum", &[get_enum])
        } else {
            self.cl("OpConvIntFromEnum", std::slice::from_ref(&subject_val))
        };

        self.lexer.token("{");

        let mut arms: Vec<EnumArm> = Vec::new();
        let mut covered: HashSet<u32> = HashSet::new();
        let mut has_wildcard = false;
        let mut result_type = Type::Void;
        // L2: field bindings in conditional arms are hoisted before the if-chain
        // to avoid codegen stack-layout issues with text operations inside branches.
        let mut hoisted_bindings: Vec<Value> = Vec::new();

        loop {
            if self.lexer.peek_token("}") {
                break;
            }
            // @PLN25 — a `null` pattern arm on a nullable inline enum element
            // (`match vr[i] { null => …, Some{…} => …/_ => … }`) matches the ABSENT
            // state: discriminant 0.  The synthetic `__nullable<S>` enum represents
            // null as disc 0 (not a produced variant), and `disc_expr` already reads
            // the discriminant, so this arm is just `discs == [0]`.  Scoped to the
            // synth enum (a regular enum's null is the variable store_nr sentinel,
            // not an inline disc — E1).  `null` is a keyword, not an identifier, so
            // it must be matched before the `has_identifier()` variant path below.
            if valid_enum
                && e_nr != u32::MAX
                && self.data.def(e_nr).name.starts_with("__nullable<")
                && self.lexer.has_token("null")
            {
                self.expect_match_arm_arrow();
                let arm_write_state = self.vars.save_and_clear_write_state();
                self.vars.clear_write_state();
                let mut arm_body = Value::Null;
                let arm_expected = Self::match_arm_expected(&result_type);
                let arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_body);
                self.vars.restore_write_state(&arm_write_state);
                // loft#978 — every arm can deliver this match's value, so the result carries
                // what ANY of them borrows.  A no-op on the first arm (nothing to join with);
                // on the later ones it stops an owned arm from erasing a borrowed sibling's dep.
                result_type = self.join_arm_into(&result_type, &arm_body, &arm_type);
                self.match_void_arm |= matches!(arm_type, Type::Void);
                if result_type == Type::Void || result_type == Type::Null {
                    result_type = arm_type.clone();
                } else if !self.first_pass
                    && arm_type != Type::Void
                    && arm_type != Type::Null
                    && !self.arm_convert_reported
                    && !self.match_arms_unify(&result_type, &arm_type)
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "cannot unify: {} and {}",
                        result_type.name(&self.data),
                        arm_type.name(&self.data)
                    );
                }
                // A `null` arm (disc 0) covers the synth enum's `Null` variant for
                // exhaustiveness — disc 1 (the vestigial `Null` variant) is never
                // produced (null is disc 0), so `null` + `Some` IS exhaustive.
                let null_variant = self.data.variant_of(e_nr, "Null");
                if null_variant != u32::MAX {
                    covered.insert(null_variant);
                }
                arms.push(EnumArm {
                    discs: vec![0],
                    code: arm_body,
                    tp: arm_type,
                    guard: None,
                    bindings: Vec::new(),
                });
                self.lexer.has_token(","); // optional trailing comma
                continue;
            }
            let Some(first_ident) = self.lexer.has_identifier() else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expect variant name or '_' in match arm"
                    );
                }
                break;
            };

            // accept `Library::Variant` or `EnumName::Variant` qualified patterns.
            // The `::` resolves the right-hand identifier in the named scope.
            let pattern_name = if self.lexer.has_token("::") {
                let Some(vname) = self.lexer.has_identifier() else {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "expect variant name after '::'");
                    }
                    break;
                };
                vname
            } else {
                first_ident.clone()
            };

            if pattern_name == "_" {
                let (arm, is_exhaustive) = self.parse_match_wildcard_arm(&mut result_type);
                has_wildcard = is_exhaustive;
                arms.push(arm);
                self.lexer.has_token(","); // optional trailing comma
                if !has_wildcard {
                    continue;
                }
                // A total `_` matches everything, so an arm written after it can never be
                // selected.  Say that here: leaving it to the closing-brace expectation at the
                // end of the loop reported "Expect token }" — the right caret with the wrong
                // reason, on the rule the Match chapter states as "put it last".
                //
                // `continue` rather than `break`, so the unreachable arms are parsed as the
                // arms they are and the `}` is consumed normally; breaking here produced a
                // second, spurious error about the brace.  A GUARDED `_ if cond` never reaches
                // this point — it is not total, so it took the `continue` above, and arms are
                // expected to follow it.
                if !self.lexer.peek_token("}") {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "a `_` arm matches everything, so this arm can never be selected \
                             — move `_` to the end"
                        );
                    }
                    continue;
                }
                break;
            }

            // @PLN22 Phase 1 — resolve the variant against the subject enum via
            // the variant_of chokepoint (the (enum, variant) scope key), not the
            // bare global def_nr.  This also subsumes the C53 fix (a library
            // variant not wildcard-imported is still a child of its enum).  A
            // plain-struct match's "pattern" is the struct TYPE itself (still
            // globally keyed, never an EnumValue), so fall back to def_nr when
            // variant_of finds nothing.
            let mut variant_def_nr = self.data.variant_of(e_nr, &pattern_name);
            if variant_def_nr == u32::MAX {
                variant_def_nr = self.data.def_nr(&pattern_name);
            }

            // for plain struct match, the pattern name must match the struct type.
            // There is no discriminant — the arm always matches.
            if is_plain_struct {
                if variant_def_nr != e_nr && !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' does not match struct type {}",
                        pattern_name,
                        self.data.def(e_nr).name()
                    );
                }
                let (arm, exhaustive) = self.parse_match_struct_arm(
                    e_nr,
                    &subject_val,
                    &mut result_type,
                    &mut hoisted_bindings,
                );
                has_wildcard = exhaustive;
                arms.push(arm);
                if has_wildcard {
                    break;
                }
                self.lexer.has_token(",");
                continue;
            }

            let bad_variant = e_nr == u32::MAX
                || variant_def_nr == u32::MAX
                || self.data.def_type(variant_def_nr) != DefType::EnumValue
                || self.data.def(variant_def_nr).parent() != e_nr;
            if bad_variant {
                if !self.first_pass && valid_enum && variant_def_nr != u32::MAX {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        pattern_name,
                        self.data.def(e_nr).name()
                    );
                }
                // Skip this arm gracefully.
                if self.lexer.peek_token("{") {
                    self.lexer.token("{");
                    while !self.lexer.peek_token("}") && !self.lexer.peek_token(";") {
                        self.lexer.has_identifier();
                        self.lexer.has_token(",");
                    }
                    self.lexer.token("}");
                }
                self.expect_match_arm_arrow();
                let mut arm_code = Value::Null;
                self.expression(&mut arm_code);
                // Consume the optional trailing comma, mirroring the wildcard /
                // struct arm paths.  Without it, the next loop iteration sees the
                // leading `,` instead of a variant name and breaks early, leaving
                // the lexer mid-arm-list — which desyncs into "Expect token }".
                // This skip path fires on pass 1 whenever `e_nr == u32::MAX`
                // (the subject enum is an unresolved cross-package forward
                // reference whose dependency parses later — #375); a clean skip
                // keeps pass 1 from aborting before the dependency registers, so
                // pass 2 (with the enum resolved) parses the arms normally.
                self.lexer.has_token(",");
                continue;
            }

            // Get the discriminant integer for this variant.
            let disc: i32 = if is_struct {
                // Struct enum: field-carrying variants store the discriminant
                // in attributes[0] (the synthetic "enum" attr added by
                // parse_enum_variants).  Unit variants (`pub enum E { Null,
                // Some { … } }`) carry no attributes of their own — fall
                // back to the parent enum's attribute for this variant name.
                let variant_attrs = self.data.def(variant_def_nr).attributes();
                if let Some(first) = variant_attrs.first()
                    && let Value::Enum(nr, _) = first.value
                {
                    i32::from(nr)
                } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&pattern_name) {
                    if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                        i32::from(nr)
                    } else {
                        0
                    }
                } else {
                    0
                }
            } else {
                // Plain enum: discriminant is stored in the parent enum's attributes.
                if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&pattern_name) {
                    if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                        i32::from(nr)
                    } else {
                        0
                    }
                } else {
                    0
                }
            };

            // or-patterns — collect additional variants separated by `|`.
            // Only for plain enum arms without field bindings.
            let mut all_discs = vec![disc];
            while self.lexer.has_token("|") {
                let Some(first_or) = self.lexer.has_identifier() else {
                    if !self.first_pass {
                        diagnostic!(self.lexer, Level::Error, "expect variant name after '|'");
                    }
                    break;
                };
                // accept Lib::Variant in or-patterns as well.
                let next_name = if self.lexer.has_token("::") {
                    let Some(vname) = self.lexer.has_identifier() else {
                        if !self.first_pass {
                            diagnostic!(self.lexer, Level::Error, "expect variant name after '::'");
                        }
                        break;
                    };
                    vname
                } else {
                    first_or.clone()
                };
                // @PLN22 Phase 1 — or-pattern variant resolves against the
                // subject enum via the variant_of chokepoint (or-patterns are
                // plain-enum only, so no struct fallback is needed).
                let next_def_nr = self.data.variant_of(e_nr, &next_name);
                if !self.first_pass
                    && (next_def_nr == u32::MAX
                        || self.data.def_type(next_def_nr) != DefType::EnumValue
                        || self.data.def(next_def_nr).parent() != e_nr)
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        next_name,
                        self.data.def(e_nr).name()
                    );
                } else {
                    let next_disc = if is_struct {
                        // B1-style guard (same shape as line 603): unit
                        // variants carry no attributes of their own; fall
                        // back to the parent enum's attr list.
                        let next_variant_attrs = self.data.def(next_def_nr).attributes();
                        if let Some(first) = next_variant_attrs.first()
                            && let Value::Enum(nr, _) = first.value
                        {
                            i32::from(nr)
                        } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&next_name) {
                            if let Value::Enum(nr, _) =
                                self.data.def(e_nr).attributes()[*a_nr].value
                            {
                                i32::from(nr)
                            } else {
                                0
                            }
                        } else {
                            0
                        }
                    } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(&next_name) {
                        if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                            i32::from(nr)
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    all_discs.push(next_disc);
                    // Each or-pattern variant counts for exhaustiveness.
                    if !self.first_pass {
                        covered.insert(next_def_nr);
                    }
                }
            }

            // Parse optional field bindings for struct-enum arms.
            let mut arm_stmts: Vec<Value> = Vec::new();
            let mut field_conditions: Vec<Value> = Vec::new();
            let mut name_aliases: Vec<(String, Option<u16>)> = Vec::new();
            if is_struct && self.lexer.peek_token("{") {
                self.parse_match_enum_field_bindings(
                    variant_def_nr,
                    &pattern_name,
                    &subject_val,
                    &mut arm_stmts,
                    &mut field_conditions,
                    &mut name_aliases,
                );
            }

            // @PLN35 Phase 3 (P-Multi): a `,` here — before the guard/`=>` — begins
            // ANOTHER whole pattern for the SAME arm (a multi-pattern arm).  In a
            // single-pattern arm the next token is `=>` or `if`; a `,` before the
            // arrow is otherwise a parse error, so this collection is purely
            // additive and leaves every existing path untouched.  Each listed
            // pattern binds the SAME captures (D-simple) into the shared slots the
            // first pattern established above; whichever variant matches assigns
            // those slots from ITS OWN offsets and the one arm body reads them.
            // Emitted as one `if disc==Vi { binds_i; body }` branch per pattern —
            // identical to hand-expanding into separate single-pattern arms.
            let mut multi_branches: Vec<(i32, Vec<Value>)> = Vec::new();
            if self.lexer.peek_token(",") && valid_enum && e_nr != u32::MAX {
                let shared: std::collections::HashMap<String, (u16, Type)> = name_aliases
                    .iter()
                    .filter_map(|(name, _)| {
                        let vn = self.vars.var(name);
                        (vn != u16::MAX).then(|| (name.clone(), (vn, self.vars.tp(vn).clone())))
                    })
                    .collect();
                let first_names: HashSet<String> = shared.keys().cloned().collect();
                while self.lexer.has_token(",") {
                    if self.lexer.peek_token("=>") || self.lexer.peek_token("}") {
                        break; // dangling comma / trailing arm separator
                    }
                    let Some(vid) = self.lexer.has_identifier() else {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "expect a variant name after ',' in a multi-pattern arm"
                            );
                        }
                        break;
                    };
                    let vname = if self.lexer.has_token("::") {
                        self.lexer.has_identifier().unwrap_or_else(|| vid.clone())
                    } else {
                        vid.clone()
                    };
                    let mut ev = self.data.variant_of(e_nr, &vname);
                    if ev == u32::MAX {
                        ev = self.data.def_nr(&vname);
                    }
                    if ev == u32::MAX
                        || self.data.def_type(ev) != DefType::EnumValue
                        || self.data.def(ev).parent() != e_nr
                    {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "'{}' is not a variant of {}",
                                vname,
                                self.data.def(e_nr).name()
                            );
                        }
                        continue;
                    }
                    let disc = self.variant_disc(e_nr, is_struct, ev, &vname);
                    let mut stmts_i: Vec<Value> = Vec::new();
                    let names_i = if self.lexer.peek_token("{") {
                        self.parse_multi_pattern_extra_bindings(
                            ev,
                            &vname,
                            &subject_val,
                            &shared,
                            &mut stmts_i,
                        )
                    } else {
                        HashSet::new()
                    };
                    if !self.first_pass && names_i != first_names {
                        let want: Vec<String> = first_names.iter().cloned().collect();
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "multi-pattern arm: every listed pattern must bind the same captures ({})",
                            want.join(", ")
                        );
                    }
                    // Union coverage (M-Total): each listed total pattern counts
                    // toward exhaustiveness, exactly like the `|` or-pattern arm.
                    if !self.first_pass {
                        covered.insert(ev);
                    }
                    multi_branches.push((disc, stmts_i));
                }
                // A field sub-pattern in the FIRST pattern makes its branch condition
                // non-trivial (a `field_conditions` guard); that combination is Phase 4.
                if !multi_branches.is_empty() && !field_conditions.is_empty() && !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a field sub-pattern is not yet supported in a multi-pattern arm (Phase 4)"
                    );
                }
            }

            // parse optional guard clause after pattern + field bindings.
            // Field-bound variables are in scope for the guard expression.
            let guard_opt = self.parse_optional_guard();
            // @PLN35 Phase 3: a guard on a multi-pattern arm must hold for whichever
            // pattern matched; replicating it per branch is Phase 4.  Reject for now.
            let guard_opt = if guard_opt.is_some() && !multi_branches.is_empty() {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a guard is not yet supported on a multi-pattern arm (Phase 4)"
                    );
                }
                None
            } else {
                guard_opt
            };
            // L2: combine field sub-pattern conditions with the explicit guard (if any).
            let guard_opt = if field_conditions.is_empty() {
                guard_opt
            } else {
                let mut combined = field_conditions.remove(0);
                for c in field_conditions {
                    combined = v_if(combined, c, Value::Boolean(false));
                }
                // If there's also an explicit `if` guard, AND them.
                if let Some(g) = guard_opt {
                    combined = v_if(combined, g, Value::Boolean(false));
                }
                Some(combined)
            };

            // Duplicate arm detection.
            // Guarded arms don't count as covering the variant for exhaustiveness.
            if guard_opt.is_none() {
                if covered.contains(&variant_def_nr) {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Warning,
                            code = "unreachable-match-arm",
                            "unreachable arm: {} already matched",
                            pattern_name
                        );
                        self.lexer.fix_last(crate::diagnostics::Fix {
                            kind: crate::diagnostics::FixKind::Mechanical,
                            title: "delete the arm — an earlier one already matches".to_string(),
                            condition: None,
                            edit: None,
                            concept: "pattern matching",
                            concept_ref: "@F29",
                        });
                    }
                } else {
                    covered.insert(variant_def_nr);
                }
            }

            self.expect_match_arm_arrow();

            // Parse the arm body expression.
            // If the body starts with `{`, parse it as a scoped block so
            // the closing `}` is not confused with the match's `}`.
            // Save/restore write tracking so writes in one arm don't cause
            // false dead-assignment warnings in sibling arms.
            let arm_write_state = self.vars.save_and_clear_write_state();
            self.vars.clear_write_state();
            let mut arm_body = Value::Null;
            let arm_expected = Self::match_arm_expected(&result_type);
            let mut arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_body);
            self.vars.restore_write_state(&arm_write_state);
            // @PLN85 match_return (LOFT_JOIN_OWN): if this arm yields a borrowed-view
            // vector field binding DIRECTLY (`Filled { items } => { items }`), wrap it in
            // an owned copy `{ o = []; o += items; o }` so the value ESCAPES OWNED — the
            // `deliver3` structure. The existing promotion then builds the separate-buffer
            // ABI the caller adopts + frees the argument. No-op for non-matching arms.
            // Updating `arm_type` keeps cross-arm unification reading the OWNED type.
            if let Some(owned) = self.jo_copy_borrowed_arm_yield(&mut arm_body) {
                arm_type = owned;
            }

            // S15: restore name mappings after arm body so the next arm can
            // create its own alias for the same field name.
            for (name, old) in name_aliases.drain(..) {
                if let Some(old_nr) = old {
                    self.vars.set_name(&name, old_nr);
                } else {
                    self.vars.remove_name(&name);
                }
            }

            // Type unification across arms.  A `null` arm (Type::Null) lowers to
            // the result type's null sentinel — it unifies with any sibling type
            // and never pins the result, so the first CONCRETE arm wins even when
            // a `null` arm comes first (`Jade => null, Crimson => S{…}`).  Treat a
            // current `Null` result like `Void` for promotion, and skip the unify
            // check when this arm is itself `null`.  Without this, struct-or-null
            // enum matches were rejected ("cannot unify: S and null", #365).
            // loft#978 — every arm can deliver this match's value, so the result carries
            // what ANY of them borrows.  A no-op on the first arm (nothing to join with);
            // on the later ones it stops an owned arm from erasing a borrowed sibling's dep.
            result_type = self.join_arm_into(&result_type, &arm_body, &arm_type);
            self.match_void_arm |= matches!(arm_type, Type::Void);
            if result_type == Type::Void || result_type == Type::Null {
                result_type = arm_type.clone();
            } else if !self.first_pass
                && arm_type != Type::Void
                && arm_type != Type::Null
                && !self.arm_convert_reported
                && !self.match_arms_unify(&result_type, &arm_type)
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot unify: {} and {}",
                    result_type.name(&self.data),
                    arm_type.name(&self.data)
                );
            }

            // @PLN35 Phase 3: capture the raw arm body + type for the extra
            // multi-pattern branches before the single-arm assembly below consumes
            // them.  A multi-pattern arm carries no guard and no field sub-pattern
            // conditions (both rejected above), so each branch is a plain
            // `block(binds_i; body)`.
            let multi_extra: Option<(Value, Type)> =
                (!multi_branches.is_empty()).then(|| (arm_body.clone(), arm_type.clone()));

            // When there is a guard, keep field bindings separate — they must
            // be emitted before the guard check so bound variables are available.
            // When there is no guard, wrap them into a block as before.
            let (arm_code, binding_stmts) = if guard_opt.is_some() && !arm_stmts.is_empty() {
                (arm_body, arm_stmts)
            } else if arm_stmts.is_empty() {
                (arm_body, Vec::new())
            } else {
                arm_stmts.push(arm_body);
                (
                    v_block(arm_stmts, arm_type.clone(), "match_arm"),
                    Vec::new(),
                )
            };

            arms.push(EnumArm {
                discs: all_discs,
                code: arm_code,
                tp: arm_type,
                guard: guard_opt,
                bindings: binding_stmts,
            });
            // @PLN35 Phase 3: emit one arm per EXTRA listed pattern, each binding
            // the shared slots from its own variant offsets then running a CLONE of
            // the arm body — the hand-expanded form the single arm above equals.
            if let Some((body, tp)) = multi_extra {
                for (disc, stmts_i) in multi_branches {
                    // #673 — the clone carries the FIRST pattern's `text` payload
                    // write-backs, but this branch binds the shared slots from its own
                    // variant's offsets.  Retarget the write-backs to the place this
                    // branch actually read, or `B { items }` reads its own field and
                    // writes the result into `A`'s.
                    let mut body_i = body.clone();
                    self.retarget_text_payload_writes(&mut body_i, &stmts_i);
                    let code_i = if stmts_i.is_empty() {
                        body_i
                    } else {
                        let mut ops = stmts_i;
                        ops.push(body_i);
                        v_block(ops, tp.clone(), "match_arm")
                    };
                    arms.push(EnumArm {
                        discs: vec![disc],
                        code: code_i,
                        tp: tp.clone(),
                        guard: None,
                        bindings: Vec::new(),
                    });
                }
            }
            if self.lexer.peek_token("}") {
                self.lexer.has_token(","); // optional trailing comma
            } else {
                self.lexer.token(","); // comma required between arms
            }
        }

        self.lexer.token("}");

        // Exhaustiveness check (second pass only, when no wildcard, when subject is a known enum).
        if !self.first_pass && !has_wildcard && valid_enum {
            let missing: Vec<String> = self
                .data
                .definitions
                .iter()
                .enumerate()
                .filter(|(_, d)| d.def_type == DefType::EnumValue && d.parent == e_nr)
                .filter(|(v_nr, _)| !covered.contains(&(*v_nr as u32)))
                .map(|(_, d)| d.name.clone())
                .collect();
            if !missing.is_empty() {
                let msg = format!(
                    "match on {} is not exhaustive — missing: {}; add the missing variants or a '_ =>' wildcard",
                    self.data.def(e_nr).name(),
                    missing.join(", ")
                );
                self.lexer.pos_diagnostic(Level::Error, &match_pos, &msg);
            }
        }

        // A `null` arm lowers to the result type's null sentinel — `parse_if`
        // and `build_scalar_chain` do the same.  Now that
        // result_type is final, convert bare-null (and block-trailing-null) arm
        // bodies, and keep `arm.tp` in step so the guarded-binding block wrapper
        // (below) declares the right result type.  Without this a `null` arm
        // pushes nothing and the if-chain join reads an unwritten, value-sized
        // slot (interp stack underflow / native lost value) — the #365 family.
        //
        // loft#936 — `null_value`, not `null`.  `null` doubles as a VARIABLE's
        // default-init, where a collection must be an allocated empty store, so
        // its catch-all answers a bare `Value::Null` for the whole collection
        // family — which is the very "pushes nothing" this paragraph forbids.
        // `match n { 0 => null, _ => [n] }` therefore left the ARMS at different
        // eval-stack depths and `gives(3)` read back an empty vector on
        // `--interpret` while `--native` answered `[3]`.
        let base = if matches!(result_type, Type::Void | Type::Null) {
            Value::Null
        } else {
            let typed_null = self.null_value(&result_type);
            for arm in &mut arms {
                let null_body = match &arm.code {
                    Value::Null => true,
                    Value::Block(bl) => bl
                        .operators
                        .last()
                        .is_some_and(|o| matches!(o, Value::Null)),
                    _ => false,
                };
                if !null_body {
                    continue;
                }
                match &mut arm.code {
                    Value::Block(bl) => {
                        let last = bl.operators.len() - 1;
                        bl.operators[last] = typed_null.clone();
                        bl.result = result_type.clone();
                    }
                    _ => arm.code = typed_null.clone(),
                }
                arm.tp = result_type.clone();
            }
            // Seed the chain base with the typed null too: an exhaustive enum
            // match's innermost else is unreachable, but codegen still emits it
            // and it must balance the value-sized stack slot the arms push.
            typed_null
        };

        // Build the if-chain from the collected arms (last to first).
        // `base` is reached only when no arm matches (only possible if
        // exhaustiveness fails, which is a compile error) — but it still has to
        // typecheck and balance the stack, so it carries the typed null.
        let mut chain = base;
        for arm in arms.iter().rev() {
            if arm.discs.is_empty() {
                // Wildcard — always taken; becomes the else branch of the chain.
                // guarded wildcard wraps body in If(guard, body, chain_rest).
                chain = match &arm.guard {
                    Some(guard) => v_if(guard.clone(), arm.code.clone(), chain),
                    None => arm.code.clone(),
                };
            } else {
                // build OR'd comparison for all discriminants in this arm.
                let mut cmp = self.cl("OpEqInt", &[disc_expr.clone(), Value::Int(arm.discs[0])]);
                for &d in &arm.discs[1..] {
                    let next = self.cl("OpEqInt", &[disc_expr.clone(), Value::Int(d)]);
                    cmp = v_if(cmp, Value::Boolean(true), next);
                }
                // guarded arms nest the guard inside the pattern branch.
                chain = match &arm.guard {
                    Some(guard) => {
                        let guarded = v_if(guard.clone(), arm.code.clone(), chain.clone());
                        let inner = if arm.bindings.is_empty() {
                            guarded
                        } else {
                            let mut stmts = arm.bindings.clone();
                            stmts.push(guarded);
                            v_block(stmts, arm.tp.clone(), "match_arm")
                        };
                        v_if(cmp, inner, chain)
                    }
                    None => v_if(cmp, arm.code.clone(), chain),
                };
            }
        }

        // When not a valid enum, just emit Null (errors were already reported).
        if !valid_enum {
            *code = Value::Null;
            if subject_unresolved {
                // ...except on the FIRST pass nothing was reported: the diagnostic above is
                // `!first_pass`-gated, because the subject may simply be declared lower in the
                // file.  `Void` here is what locks a local bound to the match, so pass 2's real
                // type is then REFUSED ("cannot change type from void to integer").
                return Type::Unknown(0);
            }
            return Type::Void;
        }

        // Emit the match:
        // - Plain enum: { match_subj = subject; chain }  (temp var to eval subject once)
        // - Struct enum: chain only  (subject_val is already the original expression/var)
        // L2: hoisted bindings are prepended so field reads happen before the if-chain.
        // @PLN25 DN1: a user `=> null` arm makes the match NULLABLE — widen the result to
        // `Optional(τ)` (τ the non-null scalar of the other arms), so the existing DN3
        // `(N-Store)` forces the caller to declare `τ?` or discharge. Check each USER arm's
        // TYPE (`a.tp == Null` for a bare-null arm); an arm whose value yields null through its
        // own sub-expression already carries that in `a.tp`, which the arm-join folded into
        // `result_type` (so it is `Optional`, and `is_non_null_scalar` is already false).
        // Do NOT `branch_yields_null(&a.code)` — descending the arm's LOWERED code reaches a
        // NESTED exhaustive match's synthesised unreachable `OpConv*FromNull` default and
        // falsely widens (p54: `Wrap { inner } => match inner { Leaf { v } => v }` typed `τ?`).
        if crate::keys::pln25_dn1_enabled()
            && Self::is_non_null_scalar(&result_type)
            && arms
                .iter()
                .any(|a| matches!(a.tp, Type::Null) || self.arm_yields_direct_null(&a.code))
        {
            result_type = Type::optional(result_type);
        }
        *code = if !hoisted_bindings.is_empty() || preamble.is_some() {
            let mut stmts = Vec::new();
            if let Some((v, init)) = preamble {
                stmts.push(v_set(v, init));
            }
            stmts.append(&mut hoisted_bindings);
            stmts.push(chain);
            v_block(stmts, result_type.clone(), "match")
        } else {
            chain
        };
        // loft#1019 — an arm that OWNS what it yields needs a home in this frame when
        // the merged type is a view (`Parser::own_joined_call_arms`).
        self.own_joined_call_arms(code, &result_type);
        result_type
    }

    /// Parse a wildcard (`_`) arm in a match expression.
    /// Returns the arm and whether it is exhaustive (no guard).
    /// The type a match arm is expected to answer in: what the arms have agreed on so
    /// far, or `Unknown` while nothing is settled yet.
    ///
    /// `Void` and `Null` are "not settled": a `null`-first arm must not pin the result
    /// (`match c { false => null, true => S{…} }` answers `S`), and a `Void` result is
    /// either the initial value or a statement `match` whose arms yield nothing — the
    /// same "expect nothing" an `else if` chain passes down for a `Void` then arm.
    fn match_arm_expected(result_type: &Type) -> Type {
        if result_type.is_unknown() || matches!(result_type, Type::Void | Type::Null) {
            Type::Unknown(0)
        } else {
            result_type.clone()
        }
    }

    /// Parse one match arm's body in the type its SIBLING arms answer in.
    ///
    /// A match arm is an else arm: `@FR-N-Decl` checks the destination against the whole
    /// construct, so every arm converts to the construct's type at its own tail exactly as
    /// `parse_block("else", …)` converts an `if`'s else arm.  Without the expected type an
    /// arm was neither converted nor refused — a float arm's bits read as an integer, an
    /// integer arm's as a float, and `250 + 10` reached a `u8` local holding 260, silently
    /// on both backends and for every subject kind (loft#1380's `match` twin).
    ///
    /// A `{ … }` arm is a block, and `block_result` performs the conversion with the
    /// carve-outs it already applies to `else`.  A BARE arm (`1 => x`) has no block tail,
    /// so the same conversion is asked here through the same `convert_admitting` /
    /// `validate_convert` pair, and the carve-outs are restated in the same order.
    fn parse_match_arm_body(&mut self, expected: &Type, arm_code: &mut Value) -> Type {
        // Cleared AFTER the body is parsed on both paths, never at entry: a nested `match`
        // inside this arm runs its own arms through here, so a flag set at entry would still
        // carry that inner arm's answer when THIS one is asked about.  From each clear to the
        // gate that reads it nothing else parses, so the field describes exactly this arm.
        if self.lexer.peek_token("{") {
            let tp = self.parse_block("match_arm", arm_code, expected);
            // A block arm reports through `block_result`, which hands back the EXPECTED type
            // on a failure — so the cross-arm gate sees the arms agreeing and stays silent
            // without being told.  Cleared all the same: the field is this arm's answer, not
            // the previous arm's, and a caller must not have to know which path reports.
            self.arm_convert_reported = false;
            return tp;
        }
        let at = self.lexer.pos().clone();
        let t = self.expression(arm_code);
        self.arm_convert_reported = false;
        if self.first_pass || expected.is_unknown() {
            return t;
        }
        // A void or null arm carries no value for the siblings to disagree about; a bare
        // `null` lowers to the result type's sentinel once the result is settled.
        if matches!(t, Type::Void | Type::Null) {
            return t;
        }
        // @FR-C-Var — two variants of one enum join to the ENUM and nothing is licensed
        // between them, so the arm keeps its own shape and the join above still sees that
        // the two differ.  `block_result` carves the same case out for `else`.
        if self.arm_joins_to_enum(&t, expected) {
            return t;
        }
        // A struct-enum pattern binding yields a BORROW — `Ship { carrier } => carrier` is
        // `&text` — while a sibling arm commonly yields the owned twin (`_ => "none"`).  That
        // is one type modulo ownership: the caller reads the value either way, which is why
        // `match_arm_types_unify` strips the wrapper rather than requiring the two to agree.
        // So it is not a conversion question, and asking `convert` for it re-points the
        // sibling arm's value — the extractor pattern's wildcard stopped answering its own
        // literal (`tests/scripts/35-nested-match.loft`).  A `&τ` on EITHER side is passed
        // through: an arm whose borrow also needs a width change keeps the gap the wrapper
        // already had, and narrowing it further needs the borrow to convert, not this site.
        if matches!(t, Type::RefVar(_)) || matches!(expected, Type::RefVar(_)) {
            return t;
        }
        // @FR-F-Block — the arms of a construct in STATEMENT position yield nothing anybody
        // reads, so their types need not agree.
        if self.arms_of_statement_construct && matches!(expected, Type::Void) {
            return t;
        }
        if !self.convert_admitting(arm_code, &t, expected) {
            self.validate_convert("match_arm", &t, expected, &at);
            self.arm_convert_reported = true;
            return t;
        }
        // loft#1103 — the SHAPE is the expected type's, the NULLABILITY the arm's own:
        // `(N-Join)` makes the construct optional iff some arm is.
        let honest = expected.with_deps_of(&t);
        if crate::keys::pln25_dn1_enabled()
            && matches!(t, Type::Optional(_))
            && !matches!(honest, Type::Optional(_))
        {
            Type::optional(honest)
        } else {
            honest
        }
    }

    fn parse_match_wildcard_arm(&mut self, result_type: &mut Type) -> (EnumArm, bool) {
        let guard_opt = self.parse_optional_guard();
        let is_exhaustive = guard_opt.is_none();
        self.expect_match_arm_arrow();
        let mut arm_code = Value::Null;
        let arm_expected = Self::match_arm_expected(result_type);
        let arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_code);
        // loft#978 — see the arm sites above: the wildcard is an arm like any other.
        let joined = self.join_arm_into(result_type, &arm_code, &arm_type);
        *result_type = joined;
        self.match_void_arm |= matches!(arm_type, Type::Void);
        if *result_type == Type::Void {
            *result_type = arm_type.clone();
        } else if !self.first_pass
            && arm_type != Type::Void
            && !self.arm_convert_reported
            && !self.match_arms_unify(result_type, &arm_type)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "cannot unify: {} and {}",
                result_type.name(&self.data),
                arm_type.name(&self.data)
            );
        }
        let arm = EnumArm {
            discs: vec![],
            code: arm_code,
            tp: arm_type,
            guard: guard_opt,
            bindings: Vec::new(),
        };
        (arm, is_exhaustive)
    }

    /// Parse a plain-struct match arm (field bindings + body).
    /// Returns the arm and whether it is exhaustive.
    fn parse_match_struct_arm(
        &mut self,
        e_nr: u32,
        subject_val: &Value,
        result_type: &mut Type,
        hoisted_bindings: &mut Vec<Value>,
    ) -> (EnumArm, bool) {
        let mut field_conditions: Vec<Value> = Vec::new();
        // @PLN35 L2 — a nested struct-enum field sub-pattern binds via
        // parse_match_enum_field_bindings, which uses the name-alias save list; a plain-struct
        // arm has no arm-level alias restore, so give it a local sink (the existing scalar /
        // plain-enum / wildcard sub-pattern branches never touch it → byte-identical).
        let mut name_aliases: Vec<(String, Option<u16>)> = Vec::new();
        if self.lexer.peek_token("{") {
            self.lexer.token("{");
            while !self.lexer.peek_token("}") {
                if let Some(field_name) = self.lexer.has_identifier() {
                    let attr_idx = self.data.attr(e_nr, &field_name);
                    if attr_idx != usize::MAX {
                        let field_val = self.get_field(e_nr, attr_idx, subject_val.clone());
                        let field_type = self.data.attr_type(e_nr, attr_idx);
                        if self.lexer.has_token(":") {
                            if let Some(cond) = self.parse_field_sub_pattern(
                                field_val,
                                &field_type,
                                hoisted_bindings,
                                &mut field_conditions,
                                &mut name_aliases,
                            ) {
                                field_conditions.push(cond);
                            }
                        } else {
                            let v = self.create_var(&field_name, &field_type);
                            self.vars.defined(v);
                            hoisted_bindings.push(v_set(v, field_val));
                        }
                    } else if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "unknown field '{}' on struct {}",
                            field_name,
                            self.data.def(e_nr).name()
                        );
                    }
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token("}");
        }
        self.expect_match_arm_arrow();
        let mut arm_code = Value::Null;
        let arm_expected = Self::match_arm_expected(result_type);
        let arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_code);
        let block = v_block(vec![arm_code], arm_type.clone(), "struct_match");
        if *result_type == Type::Void {
            *result_type = arm_type;
        }
        let (guard, exhaustive) = if field_conditions.is_empty() {
            (None, true)
        } else {
            let mut combined = field_conditions.remove(0);
            for c in field_conditions {
                combined = v_if(combined, c, Value::Boolean(false));
            }
            (Some(combined), false)
        };
        let arm = EnumArm {
            discs: vec![],
            code: block,
            tp: result_type.clone(),
            guard,
            bindings: Vec::new(),
        };
        (arm, exhaustive)
    }

    /// #429: the frame var a match-arm field binding BORROWS from — the
    /// subject's backing variable.  A heap match binding (`CMap { entries }`)
    /// is a DbRef into the subject's record, so its type must carry a borrow
    /// dep on this var (see the call site).
    ///
    /// A bare `Var` subject is the common `match m { … }` / `match self.f { … }`-
    /// into-a-temp case.  A subject reached through GETTERS (`Wrap { inner: Holder
    /// { items } }` binds from `OpGetField(w, …)`, and so does `match w.inner`)
    /// still borrows from one variable — the root the chain starts at.  Reporting
    /// `None` there left the binding dep-free, which reads as "owns its store", so
    /// an append allocated a FRESH backing and repointed the local: the write
    /// vanished, silently, on both backends (loft#664's shape).  A chain rooted in
    /// a CALL has no backing variable and still yields `None`.
    fn match_borrow_source(&self, subject_val: &Value) -> Option<u16> {
        match subject_val.unspan() {
            Value::Var(v) => Some(*v),
            // Only the GETTER family forwards a borrow: `OpGetField` / `OpGetVector` /
            // `OpGetEnum` read INTO their first argument's record, so the root of the
            // chain still owns the store.  A user call produces a value of its own, and
            // its first argument is just an argument.
            Value::Call(d_nr, args) if self.data.def(*d_nr).name().starts_with("OpGet") => {
                args.first().and_then(|a| self.match_borrow_source(a))
            }
            _ => None,
        }
    }

    /// #673 — record that `bind_nr` mirrors a struct-enum `text` payload, so a write
    /// through the binding can be written back into the subject.
    ///
    /// A heap payload binding holds a DbRef INTO the subject's record, so
    /// `items += …` reaches the subject with no help. A `text` payload binding is an
    /// owned copy of the characters (`_mv_items = OpGetText(subj, off)` — see the
    /// `skip_free` note at the call site), so the identical source line updated the
    /// copy and left the enum untouched, on both backends and with no diagnostic.
    /// Remembering the field read lets `parse_assign` mirror each write back with
    /// `OpSetText(subj, off, binding)`; a binding that is only read never reaches
    /// that path, so its bytecode is unchanged.
    ///
    /// Only a re-evaluable subject qualifies. The read is emitted once per write, so
    /// a subject carrying a user call (`match make_e() { … }`) would run that call
    /// again — and such a subject is a temporary the write could not outlive anyway.
    fn record_text_payload_view(&mut self, bind_nr: u16, field_read: &Value) {
        let Value::Call(d_nr, args) = field_read.unspan() else {
            return;
        };
        if self.data.def(*d_nr).name() != "OpGetText"
            || args.len() != 2
            || self.ir_has_user_call(&args[0])
        {
            return;
        }
        let read = field_read.unspan().clone();
        self.text_payload_views
            .insert((self.context, bind_nr), read);
    }

    /// #673 / @PLN35 Phase 3 — point a multi-pattern branch's cloned body at the
    /// field offsets THIS branch bound its captures from.
    ///
    /// Every extra listed pattern runs a clone of the one arm body, so a `text`
    /// payload write-back (`record_text_payload_view`) reaches the clone carrying the
    /// FIRST pattern's `(subject, offset)`. `stmts_i` — this branch's
    /// `capture = OpGetText(subject, off_i)` binds — names the place the branch really
    /// read, so swapping the write to match keeps read and write on one field.
    /// Untouched when the two patterns put the field at the same offset.
    fn retarget_text_payload_writes(&self, body: &mut Value, stmts_i: &[Value]) {
        let get_nr = self.data.def_nr("OpGetText");
        let set_nr = self.data.def_nr("OpSetText");
        if get_nr == u32::MAX || set_nr == u32::MAX {
            return;
        }
        for st in stmts_i {
            let Value::Set(v_nr, rhs) = st.unspan() else {
                continue;
            };
            let Value::Call(d_nr, args) = rhs.unspan() else {
                continue;
            };
            let Some(Value::Call(_, first)) = self.text_payload_views.get(&(self.context, *v_nr))
            else {
                continue;
            };
            if *d_nr != get_nr || args.len() != 2 || first[..] == args[..] {
                continue;
            }
            let from = Value::Call(
                set_nr,
                vec![first[0].clone(), first[1].clone(), Value::Var(*v_nr)],
            );
            let to = Value::Call(
                set_nr,
                vec![args[0].clone(), args[1].clone(), Value::Var(*v_nr)],
            );
            crate::parser::expressions::substitute_value(body, &from, &to);
        }
    }

    /// @PLN35 Phase 2 (P-Cap-View) — mark a slice-element capture that reads a HEAP
    /// element as a borrowed VIEW of the subject, the same way a struct-enum field
    /// binding is (`parse_match_enum_field_bindings`, #429). A slice binding
    /// `[first, ..] => first` (or `[tok:V, ..]`) reads `OpGetVector(subject, ..)` — a
    /// DbRef pointing INTO the subject's store, not an owned record. Untreated, it fails
    /// two ways: scope cleanup emits `OpFreeRef` for the binding at function exit, freeing
    /// a record the subject owns; and the binding carries empty deps, so when a value
    /// derived from it is returned, `ref_return` can't walk the borrow chain back to the
    /// subject parameter — the fn is mis-classified OWNED and the caller whole-store-frees
    /// the subject (a corrupted subject on BOTH backends when the capture escapes via
    /// return and the subject is reused afterwards).
    ///
    /// Scalars carry no DbRef and a `text` element is an owned copy (`OpGetText`), so
    /// neither needs this — only `Reference` / `Vector` / struct-enum element types do.
    /// `src` is the subject's source var (`Value::Var(v)` in `parse_vector_match`).
    fn mark_slice_element_view(&mut self, bind_nr: u16, elm_tp: &Type, src: u16) {
        if bind_nr == u16::MAX
            || !matches!(
                elm_tp,
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
        {
            return;
        }
        self.vars.set_skip_free(bind_nr);
        let bound_tp = match self.vars.tp(bind_nr).clone() {
            Type::Reference(td, _) => Type::Reference(td, crate::data::Deps::frame1(src)),
            Type::Vector(it, _) => Type::Vector(it, crate::data::Deps::frame1(src)),
            Type::Enum(td, su, _) => Type::Enum(td, su, crate::data::Deps::frame1(src)),
            other => other,
        };
        self.vars.set_type(bind_nr, bound_tp);
    }

    /// Parse field bindings for a struct-enum match arm.
    fn parse_match_enum_field_bindings(
        &mut self,
        variant_def_nr: u32,
        pattern_name: &str,
        subject_val: &Value,
        arm_stmts: &mut Vec<Value>,
        field_conditions: &mut Vec<Value>,
        name_aliases: &mut Vec<(String, Option<u16>)>,
    ) {
        self.lexer.token("{");
        let mut seen_fields: HashSet<String> = HashSet::new();
        while let Some(field_name) = self.lexer.has_identifier() {
            if !self.first_pass && seen_fields.contains(&field_name) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "duplicate field binding '{}' in match arm",
                    field_name
                );
            }
            seen_fields.insert(field_name.clone());

            let attr_idx_and_type = {
                let variant_def = self.data.def(variant_def_nr);
                variant_def.attributes[1..]
                    .iter()
                    .enumerate()
                    .find(|(_, a)| a.name == field_name)
                    .map(|(i, a)| (i + 1, a.typedef.clone()))
            };

            match attr_idx_and_type {
                Some((attr_idx, field_type)) => {
                    let field_read = self.get_field(variant_def_nr, attr_idx, subject_val.clone());
                    if self.lexer.has_token(":") {
                        if let Some(cond) = self.parse_field_sub_pattern(
                            field_read,
                            &field_type,
                            arm_stmts,
                            field_conditions,
                            name_aliases,
                        ) {
                            field_conditions.push(cond);
                        }
                    } else {
                        let v_nr = self.create_unique(&format!("mv_{field_name}"), &field_type);
                        if v_nr != u16::MAX {
                            self.vars.defined(v_nr);
                            // loft#1160 — remember which field this binding projects, so a
                            // write spelled through it can take the field path and reach the
                            // linked group the field belongs to.
                            self.vars.mv_field_origin.insert(
                                v_nr,
                                (
                                    field_read.clone(),
                                    Type::Reference(variant_def_nr, Deps::none()),
                                ),
                            );
                            arm_stmts.push(v_set(v_nr, field_read.clone()));
                            let old = self.vars.set_name(&field_name, v_nr);
                            name_aliases.push((field_name.clone(), old));
                            // B5 remaining half (2026-04-14): a HEAP match-arm
                            // binding is a field extraction from the subject —
                            // the subject owns the store and the binding is a
                            // borrowed view (a DbRef pointing into the subject's
                            // record).  Emitting OpFreeRef for it at function
                            // exit would decrement a store the binding doesn't
                            // own; worse, if the arm wasn't taken the slot is
                            // never assigned and the free reads garbage bytes as
                            // a DbRef (observed as out-of-bounds store_nr ≈ 4621
                            // in `p54_b5_recursive_struct_enum`).  Mark it
                            // `skip_free` so scope cleanup leaves it alone in
                            // both the taken and not-taken arms.
                            //
                            // @PLN85 Class B — but a TEXT payload binding is
                            // NOT a borrow: `_mv_<f> = OpGetText(subj, off)` is
                            // typed plain `text` (an OWNED copy), and it is
                            // default-initialised to `""` at block entry — so
                            // freeing it is correct (it owns an allocation) AND
                            // safe in the not-taken arm (`OpFreeText("")` is a
                            // no-op, no garbage read).  Leaving it `skip_free`
                            // leaked the copy 1/call whenever a text-payload arm
                            // was taken (the whole p54 struct-enum / json-match
                            // family).  So skip_free HEAP bindings only; let a
                            // text binding free through normal scope cleanup.
                            if matches!(field_type.base(), Type::Text(_)) {
                                self.record_text_payload_view(v_nr, &field_read);
                            } else {
                                self.vars.set_skip_free(v_nr);
                            }
                            // #429: the binding is a BORROWED VIEW of the
                            // subject, so its TYPE must record that borrow —
                            // otherwise a value derived from it and returned
                            // (`CMap { entries } => { r = entries[..]; return r }`)
                            // breaks the borrow chain at the binding: `ref_return`
                            // walks `r` → `entries` → <this binding> and stops (the
                            // binding has empty deps), never reaching the subject
                            // parameter, so the fn is mis-classified OWNED and the
                            // caller whole-store-frees the subject's record (#429
                            // interp-vs-native divergence).  Give a HEAP
                            // (DbRef-carrying) binding a frame dep on the subject's
                            // source var so the chain reaches the parameter — exactly
                            // the `["src"]` dep a `b = subj.field` bind already
                            // carries.  Scalars hold no DbRef, so they need no borrow
                            // dep (the `_mv_value` integer binding stays dep-free).
                            if matches!(
                                &field_type,
                                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                            ) && let Some(src) = self.match_borrow_source(subject_val)
                            {
                                if std::env::var_os("LOFT_MV_DEP_TRACE").is_some() {
                                    eprintln!(
                                        "[mv-dep] fn={} pass{} binding={}({}) src={}({})",
                                        self.data.def(self.context).name(),
                                        u8::from(!self.first_pass) + 1,
                                        self.vars.name(v_nr),
                                        v_nr,
                                        self.vars.name(src),
                                        src,
                                    );
                                }
                                let bound_tp = match self.vars.tp(v_nr).clone() {
                                    Type::Reference(td, _) => {
                                        Type::Reference(td, crate::data::Deps::frame1(src))
                                    }
                                    Type::Vector(it, _) => {
                                        Type::Vector(it, crate::data::Deps::frame1(src))
                                    }
                                    Type::Enum(td, su, _) => {
                                        Type::Enum(td, su, crate::data::Deps::frame1(src))
                                    }
                                    other => other,
                                };
                                self.vars.set_type(v_nr, bound_tp);
                            }
                        }
                    }
                }
                None => {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "variant {} has no field '{}'",
                            pattern_name,
                            field_name
                        );
                    }
                }
            }

            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token("}");
    }

    /// The discriminant integer for `variant_def_nr` (a variant of enum `e_nr`).
    /// Struct-enum variants store it in their own `attributes[0]`; unit variants
    /// and plain enums read it from the parent enum's attribute for the name.
    /// Mirrors the inline lookup the first pattern of an arm uses, factored out so
    /// the extra patterns of a `@PLN35` multi-pattern arm resolve their discs the
    /// same way.
    pub(crate) fn variant_disc(
        &self,
        e_nr: u32,
        is_struct: bool,
        variant_def_nr: u32,
        variant_name: &str,
    ) -> i32 {
        if is_struct
            && let Some(first) = self.data.def(variant_def_nr).attributes().first()
            && let Value::Enum(nr, _) = first.value
        {
            return i32::from(nr);
        }
        if let Some(a_nr) = self.data.def(e_nr).attr_names.get(variant_name)
            && let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value
        {
            return i32::from(nr);
        }
        0
    }

    /// The integer-discriminant read `OpConvIntFromEnum(OpGetEnum(elem, 0))` for a
    /// struct-enum element value — the same tag read a top-level struct-enum match
    /// emits, used by a slice-element alternation to tag-test each branch.
    pub(crate) fn elem_tag_int(&mut self, elem: Value) -> Value {
        let get_enum = self.cl("OpGetEnum", &[elem, Value::Int(0)]);
        self.cl("OpConvIntFromEnum", &[get_enum])
    }

    /// @PLN35 Phase 4.3 — the single `read(pos)` seam for a slice element.  EVERY element
    /// read in a slice pattern goes through here: `get_field(elm_type, whole, OpGetVector(v,
    /// size, pos))`.  `pos` is a `Value` — a compile-time `Int` for a fixed position today, a
    /// runtime `Var` (or `pos + offset`) once a variable-width alternation advances a cursor.
    /// Funnelling all reads here is what lets a streamed/accumulating backing (Phase 7)
    /// replace the read without touching the match logic — so do NOT open-code
    /// `OpGetVector` + `get_field` at a slice element site; call this.
    fn read_slice_elem(&mut self, v: u16, elm_size: &Value, elm_tp: &Type, pos: Value) -> Value {
        // @PLN35 PC1 — in CURSOR mode a forward read `v[i]` is relative to the cursor's current
        // `pos`, so it becomes `source[pos + i]`.  Only non-negative literal positions are offset
        // (a negative/tail read has no meaning for a prefix cursor and is rejected upstream).
        let pos = match self.match_cursor {
            Some((_, _, _, pos_var)) if matches!(&pos, Value::Int(i) if *i >= 0) => {
                self.conv_op("+", Value::Var(pos_var), pos, I32.clone(), I32.clone())
            }
            _ => pos,
        };
        let get = self.cl("OpGetVector", &[Value::Var(v), elm_size.clone(), pos]);
        let td = self.data.type_def_nr(elm_tp);
        self.get_field(td, usize::MAX, get)
    }

    /// @PLN35 Phase 7 — the LENGTH half of the match Cursor seam (`read_slice_elem` is the READ
    /// half).  The match engine asks the subject "how long are you?" ONLY through here, so a future
    /// STREAMING cursor can answer differently (pull-to-exhaustion / `max_lookahead`-bounded)
    /// without any match logic changing.  The vector cursor emits `OpLengthVector`, byte-identical
    /// to the inline calls this replaced.
    fn cursor_len(&mut self, v: u16) -> Value {
        self.cl("OpLengthVector", &[Value::Var(v)])
    }

    /// @PLN35 Phase 7 (Path B, step 2a) — materialise a coroutine `iterator<T>` match subject into a
    /// fresh `vector<T>` buffer (EAGER pull), so the existing vector-match machinery runs over it.
    /// Emits `gen = subject; done = false; buf = []; while !done { x = next(gen); if exhausted(gen)
    /// { done = true } else { buf += [x] } }` and returns `(buf_var, setup_ops)`.  The pull uses
    /// explicit `OpCoroutineNext`/`OpCoroutineExhausted` in a `while` — a `for` over a STORED
    /// coroutine hangs.  A lazy per-read pull is the step-2b refinement behind the Cursor seam.
    fn collect_iterator_subject(
        &mut self,
        subject: Value,
        iter_tp: &Type,
        elm_tp: &Type,
    ) -> (u16, Type, Vec<Value>) {
        let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
        let buf = self.create_unique("stream_buf", &vec_tp);
        self.vars.defined(buf);
        let gen_var = self.create_unique("stream_gen", iter_tp);
        self.vars.defined(gen_var);
        let done = self.create_unique("stream_done", &Type::Boolean);
        self.vars.defined(done);
        let x = self.create_unique("stream_x", elm_tp);
        self.vars.defined(x);
        // A RECORD-valued yield hands back a DbRef into the coroutine's own frame, which
        // lives in the STACK store — `x` names it, and the append below deep-copies it into
        // the buffer (`OpCopyRecord`), so the buffer owns the copy and `x` owns nothing.
        // Scope cleanup did not know that and emitted `OpFreeRef(_stream_x_1)` at the end of
        // every pull iteration, whole-store freeing a stack-record ref: `BUG (#306)` on each
        // run of `match <iterator<StructEnum>>`, and only the store-0 guard stopped it from
        // taking the eval stack with it (loft#920).  Same fact as `elm` below, one step
        // earlier in the same loop.
        //
        // Gated on the record case rather than set for every element type: `skip_free` is one
        // bit for all free kinds, and a `text` element's `x` holds a String the caller DOES
        // own — suppressing its `OpFreeText` would trade this wrong free for a leak of one
        // string per yield.  A scalar emits no free at all, so it never reaches either.
        if matches!(
            elm_tp.base(),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) {
            self.vars.set_skip_free(x);
        }
        let ed_nr = self.data.type_def_nr(elm_tp);
        let elm = self.create_unique("stream_elm", &Type::Reference(ed_nr, Deps::none()));
        self.vars.defined(elm);
        // `elm` is a TRANSIENT ref to the record just appended into `buf` — it belongs to the
        // buffer, not to `elm`.  Without skip_free, scope cleanup emits `OpFreeRef(elm)` after the
        // append and frees that record (harmless for an inline int, but it frees the string for a
        // `text` element — the null-value bug).  The comprehension avoids this via a buffer-db dep.
        self.vars.set_skip_free(elm);

        // `OpCoroutineNext` value_size: packed `(channel_tag << 8) | byte_size` of the yield type
        // (same encoding `next(gen)` computes — collections.rs `iterator`).
        let byte_size = i32::from(crate::variables::size(
            elm_tp,
            &crate::data::Context::Argument,
        ));
        let channel_tag = crate::coroutine_layout::channel_tag(elm_tp);
        let value_size = (channel_tag << 8) | byte_size;
        // Append triple (scalar/text element): the same `OpNewRecord` / `set_field` / `OpFinishRecord`
        // a `buf += [x]` comprehension emits.
        let elem_known = self.vector_of(elm_tp);
        let known = Value::Int(i32::from(if elem_known == u16::MAX {
            0
        } else {
            elem_known
        }));
        let fld = Value::Int(i32::from(u16::MAX));

        let mut setup: Vec<Value> = self.vector_db(&vec_tp, buf);
        setup.push(v_set(gen_var, subject));
        setup.push(v_set(done, Value::Boolean(false)));

        let next_call = self.cl(
            "OpCoroutineNext",
            &[Value::Var(gen_var), Value::Int(value_size)],
        );
        let exhausted = self.cl("OpCoroutineExhausted", &[Value::Var(gen_var)]);
        let new_rec = self.cl(
            "OpNewRecord",
            &[Value::Var(buf), known.clone(), fld.clone()],
        );
        let set_val = self.set_field(ed_nr, usize::MAX, 0, Value::Var(elm), Value::Var(x));
        let finish = self.cl(
            "OpFinishRecord",
            &[Value::Var(buf), Value::Var(elm), known, fld],
        );
        let append = v_block(
            vec![v_set(elm, new_rec), set_val, finish],
            Type::Void,
            "stream append",
        );
        let loop_body = vec![
            v_if(Value::Var(done), Value::Break(0), Value::Null),
            v_set(x, next_call),
            v_if(exhausted, v_set(done, Value::Boolean(true)), append),
        ];
        setup.push(v_loop(loop_body, "stream pull"));
        (buf, vec_tp, setup)
    }

    /// @PLN35 — materialise a named `..rest` sub-slice `v[lo .. hi]` into a FRESH independent
    /// `vector<T>` (`rest_var`), by reusing the proven compile-time slice-materialise path
    /// (`materialize_iterator` over a minimal index-range `Iter`).  `lo`/`hi` are `Value`s: a
    /// compile-time `Int(head_len)` / `len − tail_len` for a fixed-arity slice, or a runtime
    /// cursor `Var(pos)` / `len` once a variable-width alternation determines the head width
    /// (Phase 4.3 step 5).  Reads from the subject SOURCE var `v` (not the `_match_subj` copy),
    /// matching the `x = v[lo..hi]` path.  Pushes the materialisation into `bindings`.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    fn materialize_named_rest(
        &mut self,
        v: u16,
        elm_tp: &Type,
        elm_size: &Value,
        rest_var: u16,
        vec_tp: &Type,
        lo_val: Value,
        hi_val: Value,
        step: i32,
        bindings: &mut Vec<Value>,
    ) {
        let lo_slot = self.create_unique("rest_lo", &I32);
        let hi_slot = self.create_unique("rest_hi", &I32);
        let idx = self.create_unique("rest_idx", &I32);
        let null_idx = self.null(&I32);
        let init = Value::Insert(vec![
            v_set(lo_slot, lo_val),
            v_set(hi_slot, hi_val),
            v_set(idx, null_idx),
        ]);
        // index step: `idx = !idx ? lo : idx + step ; if hi <= idx break ; idx`.  `step` is 1 for
        // a contiguous sub-slice (`..rest`, a bare `(V)*` run) and 2 for a SEPARATED run
        // `(V)*(Sep)` — the V's sit at every other index (V Sep V Sep …), so a stride of 2 skips
        // the separators.  `!idx` is a NULL test (the null sentinel is i32::MIN, not 0), so a `lo`
        // of 0 seeds correctly and never re-triggers the first-iteration branch.
        let bump = self.conv_op(
            "+",
            Value::Var(idx),
            Value::Int(step),
            I32.clone(),
            I32.clone(),
        );
        let not_idx = self.single_op("!", Value::Var(idx), I32.clone());
        let advance = v_set(idx, v_if(not_idx, Value::Var(lo_slot), bump));
        let past_end = self.conv_op(
            "<=",
            Value::Var(hi_slot),
            Value::Var(idx),
            I32.clone(),
            I32.clone(),
        );
        let brk = v_if(past_end, Value::Break(0), Value::Null);
        let idx_block = v_block(
            vec![advance, brk, Value::Var(idx)],
            I32.clone(),
            "Iter range",
        );
        // Element read must match the slice-subscript builder in `fields.rs`
        // (`parse_vector_index`): a linked struct derefs its record pointer, a
        // base/primitive wraps with `get_val` (e.g. `OpGetInt`/`OpGetText`), a
        // tuple unboxes, and an inline struct stays the RAW record DbRef so
        // `materialize_iterator`'s `OpCopyRecord` deep-copies it.
        let elm_type_def = match self.data.type_elm(elm_tp) {
            u32::MAX => self.data.source_nr(0, "reference"),
            e => e,
        };
        let known = self.data.def(elm_type_def).known_type();
        let elm_read = if self.database.is_linked(known) {
            self.cl("OpVectorRefNullable", &[Value::Var(v), idx_block])
        } else {
            let mut r = self.cl(
                "OpGetVectorNullable",
                &[Value::Var(v), elm_size.clone(), idx_block],
            );
            if self.database.is_base(known) {
                r = self.get_val(elm_tp, matches!(elm_tp, Type::Optional(_)), 0, r, u32::MAX);
            } else if let Type::Tuple(elems) = elm_tp {
                let elems = elems.clone();
                r = self.unbox_tuple_from_dbref(r, &elems);
            }
            r
        };
        let next = v_block(vec![elm_read], elm_tp.clone(), "Vector Index");
        let mut mat = Value::Iter(
            u16::MAX,
            Box::new(init),
            Box::new(next),
            Box::new(Value::Null),
        );
        // A VIEW-read element type (a DbRef INTO the subject) needs a borrow dep on the transient
        // per-iteration read temp so the free-analysis frees each read once (via the copy source),
        // not double-freeing the subject; text/scalar take NO dep (owned copy / no DbRef).
        let elm_borrowed = match elm_tp {
            Type::Reference(td, _) => Type::Reference(*td, Deps::frame1(v)),
            Type::Vector(it, _) => Type::Vector(it.clone(), Deps::frame1(v)),
            Type::Enum(td, su, _) => Type::Enum(*td, *su, Deps::frame1(v)),
            other => other.clone(),
        };
        let iter_tp = Type::Iterator(Box::new(elm_borrowed), Box::new(Type::Null));
        self.materialize_iterator(
            &mut mat,
            &iter_tp,
            &Value::Var(rest_var),
            vec_tp,
            rest_var,
            "=",
        );
        // `rest`'s elements are deep-copied (OpCopyRecord) into its own store — INDEPENDENT of the
        // subject — so reset the ELEMENT type to the clean `elm_tp` while PRESERVING `rest`'s
        // vector-level deps (the fresh-store dep `materialize_iterator` set); leaving the element
        // borrow-dep would make the analysis treat `rest` as borrowing the subject → a leak.
        if let Type::Vector(_, vdeps) = self.vars.tp(rest_var).clone() {
            self.vars
                .set_type(rest_var, Type::Vector(Box::new(elm_tp.clone()), vdeps));
        }
        bindings.push(mat);
    }

    /// @PLN35 slice 2 — collect a per-iteration FIELD projection from a struct-enum repetition
    /// run `( V { field } )*`.  Like `materialize_named_rest`, but the per-element read projects
    /// `variant.field` (`get_field`) instead of the whole element, so the result is a fresh
    /// `vector<field_type>` — a scalar/text projection.  The run already tag-tested every element
    /// against `variant_def_nr`, so the field read is valid at each index.  A scalar/text field is
    /// an OWNED value (no DbRef into the subject), so — unlike the whole-element case — no borrow
    /// dep is needed and `materialize_iterator` deep-copies it into the projection's own store.
    /// Reads `v[lo..hi]` with `step` (2 for a separated run `(V)*(Sep)`).
    #[allow(clippy::too_many_arguments)]
    fn materialize_field_projection(
        &mut self,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        variant_def_nr: u32,
        attr_idx: usize,
        field_type: &Type,
        proj_var: u16,
        proj_vec_tp: &Type,
        lo_val: Value,
        hi_val: Value,
        step: i32,
        bindings: &mut Vec<Value>,
    ) {
        let lo_slot = self.create_unique("proj_lo", &I32);
        let hi_slot = self.create_unique("proj_hi", &I32);
        let idx = self.create_unique("proj_idx", &I32);
        let null_idx = self.null(&I32);
        let init = Value::Insert(vec![
            v_set(lo_slot, lo_val),
            v_set(hi_slot, hi_val),
            v_set(idx, null_idx),
        ]);
        // Same index step as `materialize_named_rest`: `idx = !idx ? lo : idx + step; if hi <= idx
        // break; idx`.  `!idx` is a NULL test (sentinel i32::MIN), so a `lo` of 0 seeds correctly.
        let bump = self.conv_op(
            "+",
            Value::Var(idx),
            Value::Int(step),
            I32.clone(),
            I32.clone(),
        );
        let not_idx = self.single_op("!", Value::Var(idx), I32.clone());
        let advance = v_set(idx, v_if(not_idx, Value::Var(lo_slot), bump));
        let past_end = self.conv_op(
            "<=",
            Value::Var(hi_slot),
            Value::Var(idx),
            I32.clone(),
            I32.clone(),
        );
        let brk = v_if(past_end, Value::Break(0), Value::Null);
        let idx_block = v_block(
            vec![advance, brk, Value::Var(idx)],
            I32.clone(),
            "Iter range",
        );
        // Read the enum element at `idx` the same way the head sub-pattern path does
        // (`read_slice_elem` → an enum value `get_field` can read from), then project the field.
        let elm_read = self.read_slice_elem(v, elm_size, elm_tp, idx_block);
        let field_read = self.get_field(variant_def_nr, attr_idx, elm_read);
        let next = v_block(vec![field_read], field_type.clone(), "Field projection");
        let mut mat = Value::Iter(
            u16::MAX,
            Box::new(init),
            Box::new(next),
            Box::new(Value::Null),
        );
        let iter_tp = Type::Iterator(Box::new(field_type.clone()), Box::new(Type::Null));
        self.materialize_iterator(
            &mut mat,
            &iter_tp,
            &Value::Var(proj_var),
            proj_vec_tp,
            proj_var,
            "=",
        );
        bindings.push(mat);
    }

    /// @PLN35 Phase 6.3 — is the next token the start of a LITERAL slice element (`[ 1, … ]`,
    /// `[ "kw", … ]`, `[ 'c', … ]`, `[ -1, … ]`)?  A pure peek — consumes nothing.
    fn peek_is_slice_literal(&self) -> bool {
        matches!(
            self.lexer.peek().has,
            LexItem::Integer(..)
                | LexItem::Long(_)
                | LexItem::Float(..)
                | LexItem::Single(_)
                | LexItem::CString(_)
                | LexItem::Character(_)
        ) || self.lexer.peek_token("-")
    }

    /// @PLN35 Phase 6.3 — may a literal of `lit_tp` match a slice element of `elm_tp`?  Same
    /// category (numeric / text / character / boolean), allowing the int→float widening the
    /// expression parser already applies to a numeric literal.
    fn slice_literal_compatible(elm_tp: &Type, lit_tp: &Type) -> bool {
        matches!(
            (elm_tp.base(), lit_tp.base()),
            (Type::Integer(_), Type::Integer(_))
                | (Type::Float, Type::Float | Type::Integer(_))
                | (Type::Single, Type::Single | Type::Float | Type::Integer(_))
                | (Type::Text(_), Type::Text(_))
                | (Type::Character, Type::Character)
                | (Type::Boolean, Type::Boolean)
        )
    }

    /// @PLN35 Phase 6.3 (`#lexeme`) — desugar a LITERAL slice element `[ "fn", … ]` on a
    /// struct-enum (token) element into a match against the `#lexeme` field.  A bare `"fn"`
    /// matches iff the element is SOME variant whose `#lexeme` field (compatible with the
    /// literal's type) equals the literal — an OR over the eligible variants of
    /// `tag(v[pos]) == disc && v[pos].<lexeme> == lit`.  The field read sits inside the tag
    /// test's `then`, so a non-matching variant never reads at the wrong offset.  Returns the
    /// condition, or `None` if the enum has no `#lexeme` field the literal can match.
    #[allow(clippy::too_many_arguments)]
    fn build_lexeme_literal_match(
        &mut self,
        e_nr: u32,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        pos: &Value,
        lit: &Value,
        lit_tp: &Type,
    ) -> Option<Value> {
        let variant_names: Vec<String> = self
            .data
            .def(e_nr)
            .attributes()
            .iter()
            .map(|a| a.name.clone())
            .collect();
        let mut acc: Option<Value> = None;
        for vname in variant_names {
            let vdef = self.data.variant_of(e_nr, &vname);
            if vdef == u32::MAX {
                continue;
            }
            // The variant's `#lexeme` field (skip attr 0, the discriminant) of a type the
            // literal can match.
            let lex = self
                .data
                .def(vdef)
                .attributes()
                .iter()
                .enumerate()
                .skip(1)
                .find(|(_, a)| a.lexeme && Self::slice_literal_compatible(&a.typedef, lit_tp))
                .map(|(i, a)| (i, a.typedef.clone()));
            let Some((lex_idx, lex_tp)) = lex else {
                continue;
            };
            let disc = self.variant_disc(e_nr, true, vdef, &vname);
            let tag_elem = self.read_slice_elem(v, elm_size, elm_tp, pos.clone());
            let tag = self.elem_tag_int(tag_elem);
            let tag_eq = self.cl("OpEqInt", &[tag, Value::Int(disc)]);
            let field_elem = self.read_slice_elem(v, elm_size, elm_tp, pos.clone());
            let field = self.get_field(vdef, lex_idx, field_elem);
            let field_eq = self.conv_op("==", field, lit.clone(), lex_tp, lit_tp.clone());
            let branch = v_if(tag_eq, field_eq, Value::Boolean(false)); // tag && field
            acc = Some(match acc {
                Some(a) => v_if(a, Value::Boolean(true), branch), // a || branch
                None => branch,
            });
        }
        acc
    }

    /// @PLN35 Phase 6.3 — the equality condition for a LITERAL element at `pos` (a position
    /// VALUE, so callers pass `Int(i)` for a head element or `Int(-(tail_len-j))` for a tail
    /// element read from the end).  A SCALAR element compares directly; a STRUCT-ENUM element
    /// matches its `#lexeme` field.  `None` = the literal cannot match this element type.
    fn build_literal_match(
        &mut self,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        pos: &Value,
        lit: &Value,
        lit_tp: &Type,
    ) -> Option<Value> {
        if let Type::Enum(e_nr, true, _) = elm_tp {
            self.build_lexeme_literal_match(*e_nr, v, elm_size, elm_tp, pos, lit, lit_tp)
        } else if Self::slice_literal_compatible(elm_tp, lit_tp) {
            let read = self.read_slice_elem(v, elm_size, elm_tp, pos.clone());
            Some(self.conv_op("==", read, lit.clone(), elm_tp.clone(), lit_tp.clone()))
        } else {
            None
        }
    }

    /// @PLN35 — the diagnostic when a literal slice element cannot match `elm_tp` (raised when
    /// `build_literal_match` returns `None`): a `#lexeme` hint for a struct-enum, a type mismatch
    /// for a scalar.
    fn slice_literal_mismatch(&mut self, elm_tp: &Type, lit_tp: &Type) {
        if let Type::Enum(e_nr, true, _) = elm_tp {
            diagnostic!(
                self.lexer,
                Level::Error,
                "{} has no `#lexeme` field a {} literal can match — mark a field `#lexeme` or write the variant pattern",
                self.data.def(*e_nr).name(),
                lit_tp.name(&self.data)
            );
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a {} literal cannot match a {} slice element",
                lit_tp.name(&self.data),
                elm_tp.name(&self.data)
            );
        }
    }

    /// @PLN35 — ONE lexical look-ahead (no parser side effects) at a `(` in a slice pattern
    /// that classifies the group in a single save/revert.  (Two sequential peeks over the same
    /// region corrupt the lexer's replay buffer — a later peek reads stale state — so all the
    /// group kinds share this one scan.)  Skips an optional `name:` capture prefix and a
    /// balanced `{ … }` body, then decides from the token following the FIRST variant
    /// sub-pattern — stopping there so a non-routed group `(A | B)` does not over-scan and
    /// shift a downstream diagnostic column (`revert` restores `peek` but, by design, not the
    /// forward scan `position` that error rendering reads):
    ///   • another identifier    → branch 1 is a sequence         → `Alt` (multi-element)
    ///   • `)` then `?`           → optional `(A)?`                → `Alt` (degenerate `(A|ε)`)
    ///   • `)` then `*` / `+`     → repetition `(A)*` / `(A)+`     → `Repetition`
    ///   • anything else (`|`, `)` bare, `,`) → single-element alt / bare group → `Other`
    fn peek_group_kind(&mut self) -> SliceGroupKind {
        let save = self.lexer.link();
        self.lexer.cont(); // past `(`
        let mut kind = SliceGroupKind::Other;
        if matches!(self.lexer.peek().has, LexItem::Identifier(_)) {
            self.lexer.cont(); // first identifier (a `name:` prefix or the variant itself)
            if self.lexer.peek_token(":") {
                // it was a `name:` capture prefix — the variant name follows.
                self.lexer.cont(); // `:`
                if matches!(self.lexer.peek().has, LexItem::Identifier(_)) {
                    self.lexer.cont(); // variant name
                }
            }
            if self.lexer.peek_token("::") {
                self.lexer.cont();
                if matches!(self.lexer.peek().has, LexItem::Identifier(_)) {
                    self.lexer.cont();
                }
            }
            if self.lexer.peek_token("{") {
                let mut depth = 0i32;
                loop {
                    match &self.lexer.peek().has {
                        LexItem::Token(t) if t == "{" => {
                            depth += 1;
                            self.lexer.cont();
                        }
                        LexItem::Token(t) if t == "}" => {
                            depth -= 1;
                            self.lexer.cont();
                            if depth == 0 {
                                break;
                            }
                        }
                        LexItem::None => break,
                        _ => self.lexer.cont(),
                    }
                }
            }
            if matches!(self.lexer.peek().has, LexItem::Identifier(_)) {
                // a second variant name → branch 1 is a sequence (multi-element alt).
                kind = SliceGroupKind::Alt;
            } else if self.lexer.peek_token(")") {
                // `(A)` — the suffix after `)` decides: `?` optional, `*`/`+` repetition.
                self.lexer.cont();
                if self.lexer.peek_token("?") {
                    kind = SliceGroupKind::Alt;
                } else if self.lexer.peek_token("*") || self.lexer.peek_token("+") {
                    kind = SliceGroupKind::Repetition;
                }
            }
        }
        self.lexer.revert(save);
        kind
    }

    /// @PLN35 slice 1 — is the upcoming slice element a scalar type-annotated capture
    /// `name : Type` (optionally with a `*` / `+` repetition postfix)?  Returns
    /// `Some(is_repetition)` when the shape matches, else `None`.  ONE save/revert look-ahead —
    /// robust to the `peek_named_arg` that follows it now the `cont()` replay-buffer duplicate
    /// is fixed (see `lexer::test::link_revert_repeatable_same_region`).
    fn peek_scalar_type_capture(&mut self) -> Option<bool> {
        if !matches!(self.lexer.peek().has, LexItem::Identifier(_)) {
            return None;
        }
        let save = self.lexer.link();
        let mut res = None;
        self.lexer.cont(); // name
        if self.lexer.peek_token(":") {
            self.lexer.cont(); // `:`
            // A `_` after `:` is the wildcard sub-pattern (`name:_`), not a type — leave it to
            // the `name:pat` path.  A real type name identifies the scalar capture.
            if matches!(&self.lexer.peek().has, LexItem::Identifier(id) if id != "_") {
                self.lexer.cont(); // Type
                res = Some(self.lexer.peek_token("*") || self.lexer.peek_token("+"));
            }
        }
        self.lexer.revert(save);
        res
    }

    /// @PLN35 slice 1 (L3.4 scalar repetition) — a scalar bare-postfix `name:Type*` / `+` slice
    /// element (no parens).  `Type` names the vector's scalar element type, so EVERY element
    /// matches and the run takes exactly the middle: `end = len - tail_len` (no run-loop — unlike
    /// a struct-enum variant run there is no per-element tag that could stop it early).  `name`
    /// collects `v[head_len .. end]` as a fresh `vector<elm_tp>`, reusing the `..rest`
    /// materialisation.  Fixed LITERAL tail elements after the run are matched from the END.
    /// `*` = zero-or-more (`head_len <= end`); `+` requires a non-empty run (`head_len < end`).
    /// Assumes the lexer is at `name`; consumes through `]`.  A `..rest` or non-literal tail after
    /// the run, and a `Type` that is not the element type, are rejected.
    fn parse_scalar_slice_repetition(
        &mut self,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        head_len: i32,
        cond: &mut Option<Value>,
        bindings: &mut Vec<Value>,
    ) {
        let cap_name = self.lexer.has_identifier().unwrap_or_default();
        self.lexer.token(":");
        let tname = self.lexer.has_identifier().unwrap_or_default();
        let elm_name = elm_tp.name(&self.data);
        if !self.first_pass && tname != elm_name {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a scalar repetition `{cap_name}:{tname}*` must match the vector's element type {elm_name}"
            );
        }
        let plus = self.lexer.has_token("+");
        if !plus {
            self.lexer.token("*");
        }
        // Tail: fixed LITERAL elements only (a bind / variant / `..rest` after a scalar run is
        // deferred, mirroring the struct-enum repetition's tail restriction).
        let mut tail_lits: Vec<(Value, Type)> = Vec::new();
        while self.lexer.has_token(",") {
            if self.lexer.peek_token("..") {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a `..rest` after a scalar repetition `{cap_name}:{tname}*` is not yet supported"
                    );
                }
                self.lexer.has_token("..");
                let _ = self.lexer.has_identifier();
                break;
            } else if self.peek_is_slice_literal() {
                let mut lit = Value::Null;
                let lit_tp = self.expression(&mut lit);
                tail_lits.push((lit, lit_tp));
            } else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "only literal elements are supported after a scalar repetition `{cap_name}:{tname}*`"
                    );
                }
                // Recover to the closing `]` so `token("]")` below succeeds and THIS diagnostic
                // is the reported error, not a first-pass cascade of "Expect token ]" that aborts
                // before the second pass ever emits it.
                while !self.lexer.peek_token("]") && !matches!(self.lexer.peek().has, LexItem::None)
                {
                    self.lexer.cont();
                }
                break;
            }
        }
        self.lexer.token("]");
        let tail_len = tail_lits.len() as i32;

        // `end = len - tail_len` — the run takes exactly the middle.
        let end_var = self.create_unique("srep_end", &I32);
        self.vars.defined(end_var);
        let len_call = self.cursor_len(v);
        let end_expr = self.conv_op(
            "-",
            len_call,
            Value::Int(tail_len),
            I32.clone(),
            I32.clone(),
        );
        let mut run_ops: Vec<Value> = vec![v_set(end_var, end_expr)];
        // `*` needs room for head+tail (`head_len <= end`); `+` needs a non-empty run
        // (`head_len < end`).  `>` has no Int form, so `+` is the swapped `<`.
        let match_bool = if plus {
            self.conv_op(
                "<",
                Value::Int(head_len),
                Value::Var(end_var),
                I32.clone(),
                I32.clone(),
            )
        } else {
            self.conv_op(
                "<=",
                Value::Int(head_len),
                Value::Var(end_var),
                I32.clone(),
                I32.clone(),
            )
        };
        run_ops.push(match_bool);
        let mut arm_cond = v_block(run_ops, Type::Boolean, "scalar rep cond");
        // AND the fixed tail-literal conditions (matched from the END, negative index) AFTER the
        // run boolean; `&&` short-circuits so a too-short slice never reads out of range.
        for (j, (lit, lit_tp)) in tail_lits.into_iter().enumerate() {
            let pos = Value::Int(-(tail_len - j as i32));
            match self.build_literal_match(v, elm_size, elm_tp, &pos, &lit, &lit_tp) {
                Some(tc) => arm_cond = v_if(arm_cond, tc, Value::Boolean(false)),
                None if !self.first_pass => self.slice_literal_mismatch(elm_tp, &lit_tp),
                None => {}
            }
        }
        *cond = match cond.take() {
            Some(existing) => Some(v_if(existing, arm_cond, Value::Boolean(false))),
            None => Some(arm_cond),
        };

        // Capture: `name = v[head_len .. end]` (a fresh `vector<elm_tp>`).  Runs once the arm
        // commits, after the condition set `end`.
        let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
        let cap_var = self.vars.add_variable(&cap_name, &vec_tp, &mut self.lexer);
        self.vars.defined(cap_var);
        if !self.first_pass && cap_var != u16::MAX {
            self.materialize_named_rest(
                v,
                elm_tp,
                elm_size,
                cap_var,
                &vec_tp,
                Value::Int(head_len),
                Value::Var(end_var),
                1,
                bindings,
            );
        }
    }

    /// @PLN35 Phase 6 (L3.4, P-Rep) — a repetition `[ head…, ( [name:] V )*[(Sep)] [tail…]
    /// [, ..rest] ]` / `…+`.  The body `V` is one variant of the struct-enum element type; the
    /// repetition matches the MAXIMAL run of `V` starting at `head_len` (the count of fixed head
    /// elements already parsed).  A runtime run-loop counts that run into `end`; `name` (if given)
    /// collects the run `v[head_len .. end]` as a FRESH `vector<ElemType>`.  Fixed LITERAL `tail`
    /// elements after the group are matched from the END, so the run must reach exactly
    /// `len - tail_len`; a trailing `..rest` (mutually exclusive with a fixed tail) picks up
    /// `v[end .. len]`.  `*` = zero-or-more (no-rest ⇒ `end == len - tail_len`); `+` ⇒
    /// `end > head_len` (≥1 body element).  The run-loop lives INSIDE the arm condition (before
    /// the bindings materialise), yielding the match boolean.  Assumes the lexer is at the `(`;
    /// consumes through `]`.  Per-iteration field capture inside the body, and non-literal tail
    /// elements, are deferred (rejected here).
    #[allow(clippy::too_many_arguments)]
    fn parse_slice_repetition(
        &mut self,
        e_nr: u32,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        head_len: i32,
        borrow_src: u16,
        cond: &mut Option<Value>,
        bindings: &mut Vec<Value>,
    ) {
        self.lexer.token("(");
        // Optional `name:` capture prefix.
        let mut cap_name: Option<String> = None;
        if let Some(named) = self.lexer.peek_named_arg() {
            cap_name = Some(named);
            self.lexer.has_identifier();
            self.lexer.token(":");
        }
        // The body variant.
        let vname = self.lexer.has_identifier().unwrap_or_default();
        let real = if self.lexer.has_token("::") {
            self.lexer.has_identifier().unwrap_or_else(|| vname.clone())
        } else {
            vname.clone()
        };
        let mut vdef = self.data.variant_of(e_nr, &real);
        if vdef == u32::MAX {
            vdef = self.data.def_nr(&real);
        }
        let valid = vdef != u32::MAX
            && self.data.def_type(vdef) == DefType::EnumValue
            && self.data.def(vdef).parent() == e_nr;
        if !valid && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "'{}' is not a variant of {}",
                real,
                self.data.def(e_nr).name()
            );
        }
        let disc = if valid {
            self.variant_disc(e_nr, true, vdef, &real)
        } else {
            0
        };
        // @PLN35 slice 2 — `( V { field, … } )*` captures a per-iteration FIELD PROJECTION: each
        // named scalar/text field collects into its own fresh `vector<field_type>` (vs a `name:`
        // prefix, which collects whole elements).  The run tag-tests `V`, so every element carries
        // the field.  A non-scalar field, or a name that is not a field of `V`, is rejected.
        let mut field_caps: Vec<(String, usize, Type)> = Vec::new();
        if self.lexer.has_token("{") {
            if valid {
                while let Some(fname) = self.lexer.has_identifier() {
                    let found = {
                        let vd = self.data.def(vdef);
                        vd.attributes[1..]
                            .iter()
                            .enumerate()
                            .find(|(_, a)| a.name == fname)
                            .map(|(i, a)| (i + 1, a.typedef.clone()))
                    };
                    match found {
                        Some((attr_idx, ftype))
                            if matches!(
                                ftype.base(),
                                Type::Integer(_)
                                    | Type::Boolean
                                    | Type::Float
                                    | Type::Single
                                    | Type::Character
                                    | Type::Text(_)
                            ) =>
                        {
                            field_caps.push((fname, attr_idx, ftype));
                        }
                        Some(_) if !self.first_pass => {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "per-iteration capture of the non-scalar field `{fname}` is not yet supported (only scalar/text fields project into a vector)"
                            );
                        }
                        None if !self.first_pass => {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "`{fname}` is not a field of {real}"
                            );
                        }
                        Some(_) | None => {}
                    }
                    if !self.lexer.has_token(",") {
                        break;
                    }
                }
                self.lexer.token("}");
            } else {
                // Invalid variant already diagnosed above; skip the body to recover.
                let mut depth = 1i32;
                while depth > 0 {
                    if self.lexer.has_token("{") {
                        depth += 1;
                    } else if self.lexer.has_token("}") {
                        depth -= 1;
                    } else if matches!(self.lexer.peek().has, LexItem::None) {
                        break;
                    } else {
                        self.lexer.cont();
                    }
                }
            }
        }
        self.lexer.token(")");
        let plus = self.lexer.has_token("+");
        if !plus {
            self.lexer.token("*");
        }
        // @PLN35 Phase 6.2 — an optional separator group `(Sep)` after the `*`/`+`: the run
        // becomes `V (Sep V)*`.  `Sep` is ONE variant, CONSUMED between elements but never
        // captured (`name` collects only the V's — a stride-2 read skips the separators).
        let sep_disc = self.parse_repetition_separator(e_nr);
        // After the group: a comma-separated tail of fixed elements matched from the END —
        // literals (`")"`), bare-name binds (`x`), and variant sub-patterns (`V { f }` / bare
        // `V`) — then optionally `, ..[name]`.  A fixed tail and `..rest` stay mutually exclusive.
        //
        // Count the tail elements FIRST (a robust link/revert look-ahead) so each is matched at a
        // fixed negative index `-(tail_len - j)`.  Reading from the run cursor `v[end + j]` instead
        // diverges on native: the run's `end`, set in the arm condition, is not reliably visible to
        // a tail read appended after it.
        let tail_len = {
            let save = self.lexer.link();
            let mut n = 0i32;
            while self.lexer.has_token(",") {
                if self.lexer.peek_token("..") {
                    break;
                }
                // Skip one element up to the next top-level `,` or `]` (tracking bracket depth so
                // a variant's `{ f, g }` counts as one element).
                let mut depth = 0i32;
                loop {
                    match &self.lexer.peek().has {
                        LexItem::None => break,
                        LexItem::Token(t) if t == "{" || t == "[" || t == "(" => {
                            depth += 1;
                            self.lexer.cont();
                        }
                        LexItem::Token(t) if t == "}" || t == "]" || t == ")" => {
                            if depth == 0 {
                                break;
                            }
                            depth -= 1;
                            self.lexer.cont();
                        }
                        LexItem::Token(t) if t == "," && depth == 0 => break,
                        _ => self.lexer.cont(),
                    }
                }
                n += 1;
            }
            self.lexer.revert(save);
            n
        };
        // Parse the tail with known positions: element `j` is at `-(tail_len - j)` from the end.
        // A literal / variant sub-pattern contributes a CONDITION (AND'd after the run bool); a
        // bare-name bind and a variant's field binds are BINDINGS (run once the arm commits).
        let mut tail_conds: Vec<Value> = Vec::new();
        let mut has_rest = false;
        let mut rest_name: Option<String> = None;
        let mut j = 0i32;
        while self.lexer.has_token(",") {
            if self.lexer.has_token("..") {
                has_rest = true;
                rest_name = self.lexer.has_identifier();
                break;
            }
            let pos = Value::Int(-(tail_len - j));
            if self.peek_is_slice_literal() {
                let mut lit = Value::Null;
                let lit_tp = self.expression(&mut lit);
                match self.build_literal_match(v, elm_size, elm_tp, &pos, &lit, &lit_tp) {
                    Some(tc) => tail_conds.push(tc),
                    None if !self.first_pass => self.slice_literal_mismatch(elm_tp, &lit_tp),
                    None => {}
                }
            } else if matches!(elm_tp, Type::Enum(te, true, _)
                if matches!(&self.lexer.peek().has, LexItem::Identifier(id)
                    if self.data.variant_of(*te, id) != u32::MAX))
            {
                // A variant sub-pattern `V { f }` / bare `V`, matched at `pos`.  The DIRECT read
                // (as the head sub-pattern path uses) drives both the tag-test and the field binds.
                let elem_read = self.read_slice_elem(v, elm_size, elm_tp, pos.clone());
                let mut te_binds: Vec<Value> = Vec::new();
                let mut te_conds: Vec<Value> = Vec::new();
                let mut aliases: Vec<(String, Option<u16>)> = Vec::new();
                if let Some(tag) = self.parse_field_sub_pattern(
                    elem_read,
                    elm_tp,
                    &mut te_binds,
                    &mut te_conds,
                    &mut aliases,
                ) {
                    tail_conds.push(tag);
                }
                tail_conds.append(&mut te_conds);
                bindings.append(&mut te_binds);
            } else if let Some(name) = self.lexer.has_identifier() {
                let bind_var = self.vars.add_variable(&name, elm_tp, &mut self.lexer);
                if bind_var != u16::MAX {
                    self.vars.defined(bind_var);
                    let elem_read = self.read_slice_elem(v, elm_size, elm_tp, pos.clone());
                    bindings.push(v_set(bind_var, elem_read));
                    self.mark_slice_element_view(bind_var, elm_tp, borrow_src);
                }
            } else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "unexpected element after a repetition `( … )*` — expected a literal, a bind name, a variant sub-pattern, or `..rest`"
                    );
                }
                // Recover to `]` so `token("]")` below succeeds and this is the primary error.
                while !self.lexer.peek_token("]") && !matches!(self.lexer.peek().has, LexItem::None)
                {
                    self.lexer.cont();
                }
                break;
            }
            j += 1;
        }
        if has_rest && tail_len > 0 && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a fixed tail after a repetition cannot combine with `..rest` (yet)"
            );
        }
        self.lexer.token("]");

        // @PLN35 PC — a CURSOR-mode repetition PREFIX-consumes: the run begins at the
        // ABSOLUTE index `pos + head_len` and, on a match, advances `cursor.pos` to the
        // run end — matching the MAXIMAL V run and LEAVING any following non-V tokens
        // (a plain vector must instead consume through to its end).  `base` is that
        // absolute start: plain `head_len` in vector mode (so the emitted IR stays
        // byte-identical there), a `pos + head_len` temp in cursor mode.  A fixed tail
        // matched from the END has no meaning for a prefix cursor — its "end" is the
        // whole source, not the consumed prefix — so reject it.
        let cursor_ctx = self.match_cursor;
        if cursor_ctx.is_some() && tail_len > 0 && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a fixed tail after a repetition is not supported in a cursor match \
                 (it would match from the source end, not the consumed prefix)"
            );
        }
        let (base_val, base_setup) = if let Some((_, _, _, pos_var)) = cursor_ctx {
            let b = self.create_unique("rep_base", &I32);
            self.vars.defined(b);
            let add = self.conv_op(
                "+",
                Value::Var(pos_var),
                Value::Int(head_len),
                I32.clone(),
                I32.clone(),
            );
            (Value::Var(b), Some(v_set(b, add)))
        } else {
            (Value::Int(head_len), None)
        };

        // `end` = the cursor after the last matched V (starts at `base`).
        let end_var = self.create_unique("rep_end", &I32);
        self.vars.defined(end_var);

        let mut run_ops: Vec<Value> = Vec::new();
        if let Some(bs) = base_setup {
            run_ops.push(bs);
        }
        run_ops.push(v_set(end_var, base_val.clone()));
        if let Some(sep) = &sep_disc {
            // Separated `V (Sep V)*`: match the first V, then loop `(Sep V)` pairs.
            // First V: `if head_len < len && tag(v[head_len]) == disc { end = head_len + 1 }`.
            let first_v = {
                let lc = self.cursor_len(v);
                let non_empty = self.conv_op("<", base_val.clone(), lc, I32.clone(), I32.clone());
                let elem = self.read_slice_elem(v, elm_size, elm_tp, base_val.clone());
                let tag = self.elem_tag_int(elem);
                let is_v = self.cl("OpEqInt", &[tag, Value::Int(disc)]);
                let one_more = if cursor_ctx.is_some() {
                    self.conv_op(
                        "+",
                        base_val.clone(),
                        Value::Int(1),
                        I32.clone(),
                        I32.clone(),
                    )
                } else {
                    Value::Int(head_len + 1)
                };
                v_if(
                    non_empty,
                    v_if(is_v, v_set(end_var, one_more), Value::Null),
                    Value::Null,
                )
            };
            // `(Sep V)*`: `loop { if len <= end+1 break; if tag(v[end]) != sep break;
            //             if tag(v[end+1]) != disc break; end += 2 }`.
            let need_pair_brk = {
                let lc = self.cursor_len(v);
                let next = self.conv_op(
                    "+",
                    Value::Var(end_var),
                    Value::Int(1),
                    I32.clone(),
                    I32.clone(),
                );
                let short = self.conv_op("<=", lc, next, I32.clone(), I32.clone());
                v_if(short, Value::Break(0), Value::Null)
            };
            let sep_brk = {
                let is_sep = self.sep_match_cond(sep, v, elm_size, elm_tp, Value::Var(end_var));
                v_if(is_sep, Value::Null, Value::Break(0))
            };
            let v_brk = {
                let next = self.conv_op(
                    "+",
                    Value::Var(end_var),
                    Value::Int(1),
                    I32.clone(),
                    I32.clone(),
                );
                let elem = self.read_slice_elem(v, elm_size, elm_tp, next);
                let tag = self.elem_tag_int(elem);
                let is_v = self.cl("OpEqInt", &[tag, Value::Int(disc)]);
                v_if(is_v, Value::Null, Value::Break(0))
            };
            let bump2 = {
                let b = self.conv_op(
                    "+",
                    Value::Var(end_var),
                    Value::Int(2),
                    I32.clone(),
                    I32.clone(),
                );
                v_set(end_var, b)
            };
            let sep_loop = v_loop(vec![need_pair_brk, sep_brk, v_brk, bump2], "rep sep run");
            run_ops.push(first_v);
            run_ops.push(sep_loop);
        } else {
            // Contiguous `V*`: `loop { if len <= end break; if tag(v[end]) != disc break; end += 1 }`.
            let len_brk = {
                let lc = self.cursor_len(v);
                let at_end = self.conv_op("<=", lc, Value::Var(end_var), I32.clone(), I32.clone());
                v_if(at_end, Value::Break(0), Value::Null)
            };
            let tag_brk = {
                let elem = self.read_slice_elem(v, elm_size, elm_tp, Value::Var(end_var));
                let tag = self.elem_tag_int(elem);
                let is_v = self.cl("OpEqInt", &[tag, Value::Int(disc)]);
                v_if(is_v, Value::Null, Value::Break(0))
            };
            let step = {
                let bump = self.conv_op(
                    "+",
                    Value::Var(end_var),
                    Value::Int(1),
                    I32.clone(),
                    I32.clone(),
                );
                v_set(end_var, bump)
            };
            run_ops.push(v_loop(vec![len_brk, tag_brk, step], "rep run"));
        }

        // Match boolean (after the loop): no-rest requires the run to reach exactly the start of
        // the fixed tail (`end == len - tail_len`); a rest just needs room for the head
        // (`head_len <= len`).  `+` additionally requires a non-empty run (`end > head_len`).
        let base = if has_rest || cursor_ctx.is_some() {
            // A cursor PREFIX-consumes: the run took the maximal V prefix and any
            // non-V tail simply stays unconsumed, so the arm matches whenever the head
            // fits (`base <= len`).  A vector `..rest` matches on the same "room for the
            // head" test.  Only a vector WITHOUT a rest must reach the fixed tail
            // exactly (`end == len - tail_len`, the whole-consume boundary).
            let lc = self.cursor_len(v);
            self.conv_op("<=", base_val.clone(), lc, I32.clone(), I32.clone())
        } else {
            let lc = self.cursor_len(v);
            let boundary = self.conv_op("-", lc, Value::Int(tail_len), I32.clone(), I32.clone());
            self.cl("OpEqInt", &[Value::Var(end_var), boundary])
        };
        let match_bool = if plus {
            // `end > base`, written `base < end` — `>` has no Int form (swapped `<`).
            let gt = self.conv_op(
                "<",
                base_val.clone(),
                Value::Var(end_var),
                I32.clone(),
                I32.clone(),
            );
            v_if(base, gt, Value::Boolean(false))
        } else {
            base
        };
        run_ops.push(match_bool);
        let mut arm_cond = v_block(run_ops, Type::Boolean, "rep cond");
        // AND the tail conditions (literal matches + variant tag-tests, matched from the END) AFTER
        // the run boolean — its `end == len - tail_len` already guarantees the slice is long
        // enough, and `&&` short-circuits so a too-short slice never reads out of range.  The tail
        // BINDINGS (bare-name binds, variant field binds) were pushed to `bindings` during the tail
        // parse above, so there is no separate binding pass here.
        for tc in tail_conds {
            arm_cond = v_if(arm_cond, tc, Value::Boolean(false));
        }
        *cond = match cond.take() {
            Some(existing) => Some(v_if(existing, arm_cond, Value::Boolean(false))),
            None => Some(arm_cond),
        };

        // Captures materialise AFTER the condition set `end` (bindings run once the arm commits).
        // `name` = the run `v[head_len .. end]`; a SEPARATED run puts the V's at every other index
        // (V Sep V Sep …), so it reads with a stride of 2 to skip separators (a bare run: 1).
        let cap_step = if sep_disc.is_some() { 2 } else { 1 };
        let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
        if let Some(name) = cap_name {
            let cap_var = self.vars.add_variable(&name, &vec_tp, &mut self.lexer);
            self.vars.defined(cap_var);
            if !self.first_pass && cap_var != u16::MAX {
                self.materialize_named_rest(
                    v,
                    elm_tp,
                    elm_size,
                    cap_var,
                    &vec_tp,
                    base_val.clone(),
                    Value::Var(end_var),
                    cap_step,
                    bindings,
                );
            }
        }
        // @PLN35 slice 2 — each `{ field }` projects the run's field into its own vector.
        for (fname, attr_idx, ftype) in field_caps {
            let fvec_tp = Type::Vector(Box::new(ftype.clone()), Deps::none());
            let proj_var = self.vars.add_variable(&fname, &fvec_tp, &mut self.lexer);
            self.vars.defined(proj_var);
            if !self.first_pass && proj_var != u16::MAX {
                self.materialize_field_projection(
                    v,
                    elm_size,
                    elm_tp,
                    vdef,
                    attr_idx,
                    &ftype,
                    proj_var,
                    &fvec_tp,
                    base_val.clone(),
                    Value::Var(end_var),
                    cap_step,
                    bindings,
                );
            }
        }
        if let Some(name) = rest_name {
            let rest_var = self.vars.add_variable(&name, &vec_tp, &mut self.lexer);
            self.vars.defined(rest_var);
            if !self.first_pass && rest_var != u16::MAX {
                let hi_val = self.cursor_len(v);
                self.materialize_named_rest(
                    v,
                    elm_tp,
                    elm_size,
                    rest_var,
                    &vec_tp,
                    Value::Var(end_var),
                    hi_val,
                    1,
                    bindings,
                );
            }
        }

        // @PLN35 PC — advance the cursor by the consumed prefix.  On a plain vector
        // there is no cursor and nothing to advance.  On a cursor the run consumed up
        // to `end`; a trailing `..rest` additionally consumed the remainder, so the
        // cursor lands on the source end.  Mirrors the fixed-arity advance in
        // `parse_vector_match` (including the PC5 `farthest` high-water update).
        if let Some((cursor_var, cursor_def, pos_field, pos_var)) = cursor_ctx {
            let _ = pos_var;
            let advance_to = if has_rest {
                let lc = self.cursor_len(v);
                let t = self.create_unique("rep_adv", &I32);
                self.vars.defined(t);
                bindings.push(v_set(t, lc));
                Value::Var(t)
            } else {
                Value::Var(end_var)
            };
            let adv = self.set_field(
                cursor_def,
                pos_field,
                0,
                Value::Var(cursor_var),
                advance_to.clone(),
            );
            bindings.push(adv);
            if let Some(ff) = self.match_cursor_farthest {
                let old_far = self.get_field(cursor_def, ff, Value::Var(cursor_var));
                let is_less = self.conv_op(
                    "<",
                    old_far.clone(),
                    advance_to.clone(),
                    I32.clone(),
                    I32.clone(),
                );
                let maxed = v_if(is_less, advance_to, old_far);
                let set_far = self.set_field(cursor_def, ff, 0, Value::Var(cursor_var), maxed);
                bindings.push(set_far);
            }
        }
    }

    /// @PLN35 Phase 6.2 — parse an OPTIONAL separator group after a repetition's `*` / `+`,
    /// consumed BETWEEN elements but never captured.  It is a variant TAG `(Comma)` OR a LEXEME
    /// literal `(",")` (a comma-separated token grammar → `(arg)*(",")`).  `None` = no `(` follows.
    fn parse_repetition_separator(&mut self, e_nr: u32) -> Option<SepSpec> {
        if !self.lexer.peek_token("(") {
            return None;
        }
        self.lexer.token("(");
        if self.peek_is_slice_literal() {
            // Lexeme separator `(",")` — matched by `#lexeme`/scalar equality per element.
            let mut lit = Value::Null;
            let lit_tp = self.expression(&mut lit);
            self.lexer.token(")");
            return Some(SepSpec::Lexeme(lit, lit_tp));
        }
        let sname = self.lexer.has_identifier().unwrap_or_default();
        let real = if self.lexer.has_token("::") {
            self.lexer.has_identifier().unwrap_or_else(|| sname.clone())
        } else {
            sname.clone()
        };
        let mut sdef = self.data.variant_of(e_nr, &real);
        if sdef == u32::MAX {
            sdef = self.data.def_nr(&real);
        }
        let valid = sdef != u32::MAX
            && self.data.def_type(sdef) == DefType::EnumValue
            && self.data.def(sdef).parent() == e_nr;
        if !valid && !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "separator '{}' is not a variant of {}",
                real,
                self.data.def(e_nr).name()
            );
        }
        let disc = if valid {
            self.variant_disc(e_nr, true, sdef, &real)
        } else {
            0
        };
        // A `{ … }` field body on the separator is deferred (like the repetition body): skip it.
        if self.lexer.has_token("{") {
            let mut depth = 1i32;
            while depth > 0 {
                if self.lexer.has_token("{") {
                    depth += 1;
                } else if self.lexer.has_token("}") {
                    depth -= 1;
                } else if matches!(self.lexer.peek().has, LexItem::None) {
                    break;
                } else {
                    self.lexer.cont();
                }
            }
        }
        self.lexer.token(")");
        Some(SepSpec::Variant(disc))
    }

    /// @PLN35 Phase 6.2 — the boolean "does `v[pos]` match the separator" for a repetition
    /// separator: a variant TAG test, or a `#lexeme`/scalar equality for a lexeme separator.
    fn sep_match_cond(
        &mut self,
        sep: &SepSpec,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        pos: Value,
    ) -> Value {
        match sep {
            SepSpec::Variant(disc) => {
                let elem = self.read_slice_elem(v, elm_size, elm_tp, pos);
                let tag = self.elem_tag_int(elem);
                self.cl("OpEqInt", &[tag, Value::Int(*disc)])
            }
            SepSpec::Lexeme(lit, lit_tp) => self
                .build_literal_match(v, elm_size, elm_tp, &pos, lit, lit_tp)
                .unwrap_or(Value::Boolean(false)),
        }
    }

    /// @PLN35 Phase 4.3 — a WHOLE-SLICE multi-element alternation
    /// `[ (seq₁ | seq₂ | …) [, ..rest] ]`, where each branch is a SEQUENCE of variant
    /// sub-patterns of (possibly) different width.  Dispatches PREDICTIVELY on the leading
    /// tags: an ordered `if / else if` over PURE per-branch conditions
    /// (`len {==|>=} wᵢ && tag(v[0])==d₀ && tag(v[1])==d₁ && …`), the first match committing.
    /// Captures bind conditionally (option-promoted across branches).  Assumes the lexer is at
    /// the `(` and the alternation is the WHOLE slice content (head empty); builds the arm
    /// `cond` + `bindings` and consumes through `]`.  `..rest` from the runtime cursor is
    /// Phase 4.3 step 5 (deferred here with a diagnostic).
    #[allow(clippy::too_many_arguments)]
    fn parse_multi_element_alternation(
        &mut self,
        e_nr: u32,
        v: u16,
        elm_size: &Value,
        elm_tp: &Type,
        borrow_src: u16,
        cond: &mut Option<Value>,
        bindings: &mut Vec<Value>,
    ) {
        self.lexer.token("(");
        // branches[i] = the sequence for branch i: [(disc, variant_def, fields)] per position.
        let mut branches: Vec<Vec<(i32, u32, Vec<(String, usize, Type)>)>> = Vec::new();
        loop {
            let mut branch: Vec<(i32, u32, Vec<(String, usize, Type)>)> = Vec::new();
            while let Some(vid) = self.lexer.has_identifier() {
                let vname = if self.lexer.has_token("::") {
                    self.lexer.has_identifier().unwrap_or_else(|| vid.clone())
                } else {
                    vid.clone()
                };
                let mut vdef = self.data.variant_of(e_nr, &vname);
                if vdef == u32::MAX {
                    vdef = self.data.def_nr(&vname);
                }
                let valid = vdef != u32::MAX
                    && self.data.def_type(vdef) == DefType::EnumValue
                    && self.data.def(vdef).parent() == e_nr;
                if !valid && !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        vname,
                        self.data.def(e_nr).name()
                    );
                }
                let disc = if valid {
                    self.variant_disc(e_nr, true, vdef, &vname)
                } else {
                    0
                };
                let mut fields: Vec<(String, usize, Type)> = Vec::new();
                if self.lexer.has_token("{") {
                    while let Some(fname) = self.lexer.has_identifier() {
                        let attr = if valid {
                            let vd = self.data.def(vdef);
                            vd.attributes[1..]
                                .iter()
                                .enumerate()
                                .find(|(_, a)| a.name == fname)
                                .map(|(i, a)| (i + 1, a.typedef.clone()))
                        } else {
                            None
                        };
                        match attr {
                            Some((idx, ty)) => fields.push((fname, idx, ty)),
                            None => {
                                if valid && !self.first_pass {
                                    diagnostic!(
                                        self.lexer,
                                        Level::Error,
                                        "variant {} has no field '{}'",
                                        vname,
                                        fname
                                    );
                                }
                            }
                        }
                        if !self.lexer.has_token(",") {
                            break;
                        }
                    }
                    self.lexer.token("}");
                }
                if valid {
                    branch.push((disc, vdef, fields));
                }
                if self.lexer.peek_token("|") || self.lexer.peek_token(")") {
                    break;
                }
            }
            if !branch.is_empty() {
                branches.push(branch);
            }
            if !self.lexer.has_token("|") {
                break;
            }
        }
        self.lexer.token(")");
        if branches.is_empty() {
            return;
        }

        // @PLN35 Phase 5 (P-Opt) — a trailing `?` makes the group OPTIONAL: append an EMPTY
        // branch (width 0).  It always matches (`len >= 0`), so the arm never fails and, when the
        // real branches do not match, the cursor stays put and every capture reads null — exactly
        // `(a)?`'s "try a; else bind null, cursor unmoved" contract, expressed as `(a | ε)`.
        if self.lexer.has_token("?") {
            branches.push(Vec::new());
        }

        // Optional `, ..rest` then `]`.
        let mut rest_name: Option<String> = None;
        if self.lexer.has_token(",") && self.lexer.has_token("..") {
            rest_name = self.lexer.has_identifier();
        }
        self.lexer.token("]");
        let has_rest = rest_name.is_some();

        // Per-branch PURE predicate: length + leading-tag sequence.  With a rest the branch may
        // be a PREFIX (`wᵢ <= len`); without, it must be the whole slice (`len == wᵢ`).
        let mut branch_conds: Vec<Value> = Vec::new();
        for branch in &branches {
            let w = branch.len() as i32;
            let lc = self.cursor_len(v);
            let mut c = if has_rest {
                self.conv_op("<=", Value::Int(w), lc, I32.clone(), I32.clone())
            } else {
                self.conv_op("==", lc, Value::Int(w), I32.clone(), I32.clone())
            };
            for (j, (disc, _, _)) in branch.iter().enumerate() {
                let elem = self.read_slice_elem(v, elm_size, elm_tp, Value::Int(j as i32));
                let tag = self.elem_tag_int(elem);
                let t = self.cl("OpEqInt", &[tag, Value::Int(*disc)]);
                c = v_if(c, t, Value::Boolean(false)); // c && t
            }
            branch_conds.push(c);
        }
        // arm cond = OR of the branch predicates, AND'd into any existing cond.
        let mut disjunction = branch_conds[0].clone();
        for bc in &branch_conds[1..] {
            disjunction = v_if(disjunction, Value::Boolean(true), bc.clone());
        }
        *cond = match cond.take() {
            Some(existing) => Some(v_if(existing, disjunction, Value::Boolean(false))),
            None => Some(disjunction),
        };

        // Captures: union the names; a name in every branch at a compatible type stays that
        // type, a name in only some promotes to `option<T>`.  Each capture reads at ITS
        // branch's position, guarded by that branch's predicate (untaken → null via else).
        let mut names: Vec<(String, Type)> = Vec::new();
        for branch in &branches {
            for (_, _, fields) in branch {
                for (fname, _, ftype) in fields {
                    if let Some((_, seen)) = names.iter().find(|(n, _)| n == fname) {
                        if !self.first_pass && !match_arm_types_unify(seen, ftype) {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "alternation capture '{}' is {} in one branch but {} in another",
                                fname,
                                ftype.name(&self.data),
                                seen.name(&self.data)
                            );
                        }
                    } else {
                        names.push((fname.clone(), ftype.clone()));
                    }
                }
            }
        }
        for (fname, ftype) in &names {
            let present_in_all = branches.iter().all(|br| {
                br.iter()
                    .any(|(_, _, fields)| fields.iter().any(|(n, _, _)| n == fname))
            });
            let var_type = if present_in_all {
                ftype.clone()
            } else {
                Type::optional(ftype.clone())
            };
            let v_nr = self.create_unique(&format!("mv_{fname}"), &var_type);
            if v_nr == u16::MAX {
                continue;
            }
            self.vars.defined(v_nr);
            let mut acc = self.null(&var_type);
            for (bi, branch) in branches.iter().enumerate().rev() {
                if let Some((pos, vdef, attr)) =
                    branch.iter().enumerate().find_map(|(j, (_, vd, fields))| {
                        fields
                            .iter()
                            .find(|(n, _, _)| n == fname)
                            .map(|(_, a, _)| (j, *vd, *a))
                    })
                {
                    let elem = self.read_slice_elem(v, elm_size, elm_tp, Value::Int(pos as i32));
                    let read = self.get_field(vdef, attr, elem);
                    acc = v_if(branch_conds[bi].clone(), read, acc);
                }
            }
            bindings.push(v_set(v_nr, acc));
            self.vars.set_name(fname, v_nr);
            // Heap capture = a borrowed view of the subject element (mirror the field-sub-pattern
            // path): skip its free, record the borrow dep — through an `option<T>` wrapper too.
            if !matches!(ftype.base(), Type::Text(_)) {
                self.vars.set_skip_free(v_nr);
            }
            let bs = borrow_src;
            let borrowed = |t: Type| -> Option<Type> {
                match t {
                    Type::Reference(td, _) => Some(Type::Reference(td, Deps::frame1(bs))),
                    Type::Vector(it, _) => Some(Type::Vector(it, Deps::frame1(bs))),
                    Type::Enum(td, su, _) => Some(Type::Enum(td, su, Deps::frame1(bs))),
                    _ => None,
                }
            };
            let bound_tp = match self.vars.tp(v_nr).clone() {
                Type::Optional(inner) => borrowed(*inner).map(Type::optional),
                other => borrowed(other),
            };
            if let Some(b) = bound_tp {
                self.vars.set_type(v_nr, b);
            }
        }

        // Step 5: `..rest` picks up after WHICHEVER branch matched.  The runtime cursor
        // `pos` = the matched branch's width (the pos-advance the contract calls for — here it
        // only moves forward, never reverts, because a tag branch is a pure test).  `rest`
        // materialises `v[pos .. len]` from that cursor via the shared `materialize_named_rest`.
        if let Some(name) = rest_name {
            let pos_var = self.create_unique("alt_pos", &I32);
            self.vars.defined(pos_var);
            // pos = if b0 { w0 } else if b1 { w1 } … else 0 (0 unreachable — cond already gated).
            let mut pos_acc = Value::Int(0);
            for (i, bc) in branch_conds.iter().enumerate().rev() {
                pos_acc = v_if(bc.clone(), Value::Int(branches[i].len() as i32), pos_acc);
            }
            bindings.push(v_set(pos_var, pos_acc));

            let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
            let rest_var = self.vars.add_variable(&name, &vec_tp, &mut self.lexer);
            self.vars.defined(rest_var);
            if !self.first_pass && rest_var != u16::MAX {
                let hi_val = self.cursor_len(v);
                self.materialize_named_rest(
                    v,
                    elm_tp,
                    elm_size,
                    rest_var,
                    &vec_tp,
                    Value::Var(pos_var),
                    hi_val,
                    1,
                    bindings,
                );
            }
        }
    }

    /// @PLN35 Phase 4 (P-Alt) — parse a single-element alternation
    /// `( V1 { f… } | V2 { f… } | … )` in a slice element position and emit its
    /// tag-disjunction condition (into `elem_conds`) + shared-slot capture bindings
    /// (into `bindings`).  Each alternative is a variant sub-pattern of the element
    /// enum; the element matches if ANY branch's tag matches, and each capture is
    /// read from WHICHEVER variant matched, at THAT variant's own offset (a
    /// conditional-offset read `f = if tag==V1 { f@V1 } else if tag==V2 { f@V2 } …`).
    /// Enum tags are disjoint, so ordered choice reduces to a disjunction here.
    ///
    /// Phase 4.1 scope: every branch binds the SAME captures at compatible types
    /// (partial overlap → `option<T>` is Phase 4.2; a varying-width MULTI-element
    /// alternative needs the slice cursor, Phase 4.3).  `elem` is the element value
    /// at this position; it is cloned for each tag test and field read.
    fn parse_slice_alternation_element(
        &mut self,
        e_nr: u32,
        elem: &Value,
        borrow_src: u16,
        bindings: &mut Vec<Value>,
        elem_conds: &mut Vec<Value>,
    ) {
        self.lexer.token("(");
        // (disc, variant_def_nr, fields: [(name, attr_idx, type)])
        let mut alts: Vec<(i32, u32, Vec<(String, usize, Type)>)> = Vec::new();
        loop {
            let Some(vname) = self.lexer.has_identifier() else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expect a variant name in an alternation `( … | … )`"
                    );
                }
                break;
            };
            let mut vdef = self.data.variant_of(e_nr, &vname);
            if vdef == u32::MAX {
                vdef = self.data.def_nr(&vname);
            }
            let valid = vdef != u32::MAX
                && self.data.def_type(vdef) == DefType::EnumValue
                && self.data.def(vdef).parent() == e_nr;
            if !valid && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'{}' is not a variant of {}",
                    vname,
                    self.data.def(e_nr).name()
                );
            }
            // Only touch the definition table when the variant resolved — `def(u32::MAX)`
            // panics (Unknown definition).  An invalid branch still consumes its `{ … }`
            // so the loop makes progress, but contributes no disc / fields / alt entry.
            let disc = if valid {
                self.variant_disc(e_nr, true, vdef, &vname)
            } else {
                0
            };
            let mut fields: Vec<(String, usize, Type)> = Vec::new();
            if self.lexer.has_token("{") {
                while let Some(fname) = self.lexer.has_identifier() {
                    let attr = if valid {
                        let vd = self.data.def(vdef);
                        vd.attributes[1..]
                            .iter()
                            .enumerate()
                            .find(|(_, a)| a.name == fname)
                            .map(|(i, a)| (i + 1, a.typedef.clone()))
                    } else {
                        None
                    };
                    match attr {
                        Some((idx, ty)) => fields.push((fname, idx, ty)),
                        None => {
                            if valid && !self.first_pass {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "variant {} has no field '{}'",
                                    vname,
                                    fname
                                );
                            }
                        }
                    }
                    if !self.lexer.has_token(",") {
                        break;
                    }
                }
                self.lexer.token("}");
            }
            if valid {
                alts.push((disc, vdef, fields));
            }
            if !self.lexer.has_token("|") {
                break;
            }
        }
        self.lexer.token(")");
        if alts.is_empty() {
            return;
        }

        // Tag-disjunction: OR of each branch's tag test on the element.
        let mut cond = {
            let tag = self.elem_tag_int(elem.clone());
            self.cl("OpEqInt", &[tag, Value::Int(alts[0].0)])
        };
        for (disc, _, _) in &alts[1..] {
            let tag = self.elem_tag_int(elem.clone());
            let t = self.cl("OpEqInt", &[tag, Value::Int(*disc)]);
            cond = v_if(cond, Value::Boolean(true), t);
        }
        elem_conds.push(cond);

        // Capture unification (P-Alt-Same ⊔ / P-Alt-Diff N-Join).  Union the capture
        // names across all branches; a name in EVERY branch (compatible type) binds at
        // that type, a name in only SOME becomes `option<T>` — the absent branches read
        // null through the else-chain.  Order follows first appearance.
        let mut names: Vec<(String, Type)> = Vec::new();
        for (_, _, fields) in &alts {
            for (fname, _, ftype) in fields {
                if let Some((_, seen)) = names.iter().find(|(n, _)| n == fname) {
                    if !self.first_pass && !match_arm_types_unify(seen, ftype) {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "alternation capture '{}' is {} in one branch but {} in another",
                            fname,
                            ftype.name(&self.data),
                            seen.name(&self.data)
                        );
                    }
                } else {
                    names.push((fname.clone(), ftype.clone()));
                }
            }
        }

        for (fname, ftype) in &names {
            let present_in_all = alts
                .iter()
                .all(|(_, _, fields)| fields.iter().any(|(n, _, _)| n == fname));
            let var_type = if present_in_all {
                ftype.clone()
            } else {
                Type::optional(ftype.clone())
            };
            let v_nr = self.create_unique(&format!("mv_{fname}"), &var_type);
            if v_nr == u16::MAX {
                continue;
            }
            self.vars.defined(v_nr);
            // if tag==V0 { f@V0 } else if tag==V1 { f@V1 } … else <null>.  A branch
            // lacking `f` is simply skipped — its tag falls through to the null else,
            // which is exactly the `option<T>` null a partial-overlap capture wants.
            let mut acc = self.null(&var_type);
            for (disc, vdef, fields) in alts.iter().rev() {
                if let Some((_, attr_idx, _)) = fields.iter().find(|(n, _, _)| n == fname) {
                    let read = self.get_field(*vdef, *attr_idx, elem.clone());
                    let tag = self.elem_tag_int(elem.clone());
                    let test = self.cl("OpEqInt", &[tag, Value::Int(*disc)]);
                    acc = v_if(test, read, acc);
                }
            }
            bindings.push(v_set(v_nr, acc));
            self.vars.set_name(fname, v_nr); // stays in scope for the arm body (as sub-patterns do)
            // A heap capture is a borrowed VIEW of the subject element: skip its free
            // and record the borrow dep on the subject source (mirrors the field-
            // sub-pattern path); a text payload is an OWNED copy (freed normally).
            // The underlying `ftype` drives this even when the slot is `option<T>`.
            if !matches!(ftype.base(), Type::Text(_)) {
                self.vars.set_skip_free(v_nr);
            }
            let bs = borrow_src;
            let borrowed = |t: Type| -> Option<Type> {
                match t {
                    Type::Reference(td, _) => Some(Type::Reference(td, Deps::frame1(bs))),
                    Type::Vector(it, _) => Some(Type::Vector(it, Deps::frame1(bs))),
                    Type::Enum(td, su, _) => Some(Type::Enum(td, su, Deps::frame1(bs))),
                    _ => None,
                }
            };
            let bound_tp = match self.vars.tp(v_nr).clone() {
                Type::Optional(inner) => borrowed(*inner).map(Type::optional),
                other => borrowed(other),
            };
            if let Some(b) = bound_tp {
                self.vars.set_type(v_nr, b);
            }
        }
    }

    /// @PLN35 Phase 3 (P-Multi) — parse the `{ field, … }` bindings of a NON-FIRST
    /// pattern in a comma-separated multi-pattern arm, REUSING the first pattern's
    /// capture slots (`shared`: name → (var, type)).  Whichever listed pattern
    /// matches assigns those shared slots from ITS OWN variant offsets, then the
    /// single arm body reads them — so a heap capture inherits the first pattern's
    /// `skip_free` + borrow-dep markings on the shared var for free.
    ///
    /// D-simple (P3): every pattern must bind the SAME names at a compatible type.
    /// A field here the first pattern lacks (partial overlap → `option<T>`) or a
    /// type that does not unify is a static error, deferred to Phase 4.  Returns
    /// the set of shared names this pattern bound so the caller can require the
    /// sets to match.
    fn parse_multi_pattern_extra_bindings(
        &mut self,
        variant_def_nr: u32,
        pattern_name: &str,
        subject_val: &Value,
        shared: &std::collections::HashMap<String, (u16, Type)>,
        stmts: &mut Vec<Value>,
    ) -> HashSet<String> {
        let mut bound: HashSet<String> = HashSet::new();
        self.lexer.token("{");
        while let Some(field_name) = self.lexer.has_identifier() {
            let attr_idx_and_type = {
                let variant_def = self.data.def(variant_def_nr);
                variant_def.attributes[1..]
                    .iter()
                    .enumerate()
                    .find(|(_, a)| a.name == field_name)
                    .map(|(i, a)| (i + 1, a.typedef.clone()))
            };
            match attr_idx_and_type {
                Some((attr_idx, field_type)) => {
                    // A field sub-pattern (`f: pat`) makes the branch condition
                    // non-trivial — that is Phase 4.  Reject cleanly and skip.
                    if self.lexer.has_token(":") {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "a field sub-pattern is not yet supported in a multi-pattern arm (Phase 4)"
                            );
                        }
                        self.lexer.has_identifier();
                    }
                    match shared.get(&field_name) {
                        Some((var_nr, shared_ty)) => {
                            let ok = match_arm_types_unify(shared_ty, &field_type);
                            if !ok && !self.first_pass {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "multi-pattern arm: capture '{}' is {} in this pattern but {} in the first — every listed pattern must bind the same captures at the same type",
                                    field_name,
                                    field_type.name(&self.data),
                                    shared_ty.name(&self.data)
                                );
                            }
                            // Skip the assignment into the shared slot on a confirmed
                            // type mismatch — a `text`→`integer` store is incoherent and
                            // the arm never runs (compile fails).  First pass still binds
                            // so the two-pass shapes agree.
                            if ok || self.first_pass {
                                let field_read =
                                    self.get_field(variant_def_nr, attr_idx, subject_val.clone());
                                stmts.push(v_set(*var_nr, field_read));
                            }
                            bound.insert(field_name.clone());
                        }
                        None => {
                            if !self.first_pass {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "multi-pattern arm: capture '{}' is not bound by the first pattern (partial overlap → option<T> is Phase 4)",
                                    field_name
                                );
                            }
                        }
                    }
                }
                None => {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "variant {} has no field '{}'",
                            pattern_name,
                            field_name
                        );
                    }
                }
            }
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token("}");
        bound
    }

    /// Parse an optional `if <expr>` guard clause.
    fn parse_optional_guard(&mut self) -> Option<Value> {
        if self.lexer.has_token("if") {
            let mut guard_code = Value::Null;
            let guard_type = self.expression(&mut guard_code);
            if !self.first_pass && guard_type != Type::Boolean {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "guard must be boolean, got {}",
                    guard_type.name(&self.data)
                );
            }
            Some(guard_code)
        } else {
            None
        }
    }

    /// Parse a sub-pattern in a match field position (L2).
    /// Given a field value expression and its type, returns a boolean condition; a nested
    /// struct-enum sub-pattern also pushes its payload bindings through the passed-in
    /// `arm_stmts` / `field_conditions` / `name_aliases` (the recursive pattern path).
    /// Handles: enum variant names (plain AND struct-enum, nested), scalar literals, ranges,
    /// `_` (wildcard).
    fn parse_field_sub_pattern(
        &mut self,
        field_val: Value,
        field_type: &Type,
        arm_stmts: &mut Vec<Value>,
        field_conditions: &mut Vec<Value>,
        name_aliases: &mut Vec<(String, Option<u16>)>,
    ) -> Option<Value> {
        // @PLN35 L2 — Struct-enum field: the sub-pattern is `Variant { subfields }` (or a bare
        // `Variant`).  Tag-test the field's discriminant and recurse into the variant's payload
        // bindings — the same tag-test + payload-bind a top-level struct-enum arm emits, applied to
        // the field-read value (`OpEqInt(OpConvIntFromEnum(OpGetEnum(field_val, 0)), disc)`).
        if let Type::Enum(e_nr, true, _) = field_type
            && let Some(name) = self.lexer.has_identifier()
        {
            if name == "_" {
                return None;
            }
            let disc = if let Some(a_nr) = self.data.def(*e_nr).attr_names.get(&name) {
                if let Value::Enum(nr, _) = self.data.def(*e_nr).attributes()[*a_nr].value {
                    i32::from(nr)
                } else {
                    0
                }
            } else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        name,
                        self.data.def(*e_nr).name()
                    );
                }
                return None;
            };
            let get_enum = self.cl("OpGetEnum", &[field_val.clone(), Value::Int(0)]);
            let disc_expr = self.cl("OpConvIntFromEnum", &[get_enum]);
            let tag_test = self.cl("OpEqInt", &[disc_expr, Value::Int(disc)]);
            // Nested payload: `Variant { subfields }` binds the variant's fields from field_val.
            if self.lexer.peek_token("{") {
                let variant_def_nr = self.data.variant_of(*e_nr, &name);
                self.parse_match_enum_field_bindings(
                    variant_def_nr,
                    &name,
                    &field_val,
                    arm_stmts,
                    field_conditions,
                    name_aliases,
                );
            }
            return Some(tag_test);
        }
        // Enum field: the sub-pattern is a variant name (or `_`).
        if let Type::Enum(e_nr, false, _) = field_type
            && let Some(name) = self.lexer.has_identifier()
        {
            // Wildcard — no condition.
            if name == "_" {
                return None;
            }
            // Look up variant discriminant.
            let disc = if let Some(a_nr) = self.data.def(*e_nr).attr_names.get(&name) {
                if let Value::Enum(nr, _) = self.data.def(*e_nr).attributes()[*a_nr].value {
                    i32::from(nr)
                } else {
                    0
                }
            } else {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'{}' is not a variant of {}",
                        name,
                        self.data.def(*e_nr).name()
                    );
                }
                return None;
            };
            // Build equality: field_val == Enum(disc)
            let variant_val = Value::Enum(disc as u8, *e_nr as u16);
            let mut cond = Value::Null;
            self.call_op(
                &mut cond,
                "==",
                &[field_val.clone(), variant_val],
                &[field_type.clone(), field_type.clone()],
            );
            // or-pattern: Paid | Refunded
            while self.lexer.has_token("|") {
                if let Some(next_name) = self.lexer.has_identifier() {
                    let next_disc = if let Some(a_nr) =
                        self.data.def(*e_nr).attr_names.get(&next_name)
                    {
                        if let Value::Enum(nr, _) = self.data.def(*e_nr).attributes()[*a_nr].value {
                            i32::from(nr)
                        } else {
                            0
                        }
                    } else {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "'{}' is not a variant of {}",
                                next_name,
                                self.data.def(*e_nr).name()
                            );
                        }
                        0
                    };
                    let next_variant = Value::Enum(next_disc as u8, *e_nr as u16);
                    let mut next_cond = Value::Null;
                    self.call_op(
                        &mut next_cond,
                        "==",
                        &[field_val.clone(), next_variant],
                        &[field_type.clone(), field_type.clone()],
                    );
                    // OR: if first matches → true, else check next.
                    cond = v_if(cond, Value::Boolean(true), next_cond);
                }
            }
            return Some(cond);
        }
        // Wildcard for non-enum fields.
        if matches!(&self.lexer.peek().has, LexItem::Identifier(id) if id == "_") {
            self.lexer.has_identifier(); // consume the `_`
            return None;
        }
        // Scalar field: store in a temp and use parse_match_pattern.
        let tmp = self.create_unique("fp_subj", field_type);
        self.vars.defined(tmp);
        let (pat, pat_type) = self.parse_match_pattern(field_type, tmp);
        // If parse_match_pattern returned a Block (range pattern or null pattern),
        // use it directly as a condition.
        if matches!(pat_type, Type::Boolean) || matches!(pat, Value::Block(_)) {
            return Some(v_block(
                vec![v_set(tmp, field_val), pat],
                Type::Boolean,
                "field_sub",
            ));
        }
        // Otherwise it's a literal — generate an equality comparison.
        let mut eq = Value::Null;
        self.call_op(
            &mut eq,
            "==",
            &[Value::Var(tmp), pat],
            &[field_type.clone(), field_type.clone()],
        );
        Some(v_block(
            vec![v_set(tmp, field_val), eq],
            Type::Boolean,
            "field_sub",
        ))
    }

    /// Parse a match pattern literal (integer, float, text, boolean) and optionally
    /// a range suffix `..` or `..=`. Returns the pattern Value and its type.
    fn parse_match_pattern(&mut self, subject_type: &Type, subject_var: u16) -> (Value, Type) {
        // INC#31: reject open-start ranges (`..hi =>`) in match arms with a
        // useful diagnostic.  The range-pattern codegen further down assumes
        // both `lo` and `hi` are real values — an absent `lo` would be
        // silently encoded as Value::Null and either never match
        // (interpreter) or crash native codegen (E0308: `()` vs i32).
        if self.lexer.peek_token("..") {
            // About the token the parser is HOLDING — the `..` that opens the pattern,
            // consumed just below.  So this site names its position rather than taking
            // `report_pos`'s consumed-source default, which is the `{` of the `match`
            // on the line above.
            let at = self.lexer.peek();
            self.lexer.specific(
                &at,
                Level::Error,
                "open-ended range pattern `..hi` is not supported in match arms — \
                 write the two-sided form `lo..hi` (exclusive) or `lo..=hi` (inclusive), \
                 or use a guard like `n if n < hi`",
            );
            // Consume the `..` so the rest of the arm parses cleanly.
            self.lexer.token("..");
            self.lexer.has_token("=");
            let mut hi = Value::Null;
            self.expression(&mut hi);
            return (Value::Boolean(false), Type::Boolean);
        }
        let pat_pos = self.lexer.pos().clone();
        let mut lit = Value::Null;
        let negate = self.lexer.has_token("-");
        let lit_type = if let Some(n) = self.lexer.has_integer() {
            let v = n as i32;
            lit = Value::Int(if negate { -v } else { v });
            Type::Integer(IntegerSpec::signed32())
        } else if let Some(n) = self.lexer.has_long() {
            let v = n as i64;
            lit = Value::Long(if negate { -v } else { v });
            crate::data::I64.clone()
        } else if let Some(n) = self.lexer.has_float() {
            lit = Value::Float(if negate { -n } else { n });
            Type::Float
        } else if let Some(s) = self.lexer.has_cstring() {
            lit = Value::Text(s);
            Type::Text(Deps::none())
        } else if let Some(c) = self.lexer.has_char() {
            lit = self.cl("OpConvCharacterFromInt", &[Value::Int(c as i32)]);
            Type::Character
        } else if self.lexer.has_token("true") {
            lit = Value::Boolean(true);
            Type::Boolean
        } else if self.lexer.has_token("false") {
            lit = Value::Boolean(false);
            Type::Boolean
        } else {
            self.expression(&mut lit)
        };
        // #493 — a pattern whose type cannot convert to the subject type (a text
        // literal against an integer subject, say) can never match.  Track it and
        // emit a dead `false` condition below so control falls through to the
        // wildcard, instead of a type-mismatched comparison (`OpEqInt(int,
        // "text")`) that pushes a 16 B Str into an 8 B slot — stack corruption
        // that trips the generate_call width assert under debug-assertions.
        // Preserves the existing lenient "silently doesn't match" behaviour.
        let incompatible = !self.first_pass
            && lit_type != Type::Null
            && !lit_type.is_same(subject_type)
            && !self.can_convert(&lit_type, subject_type);
        // Plan-07 phase 6 — a pattern whose type can never match the subject is a
        // static type error, not a silently-dead arm (a text literal against an
        // integer subject is almost always a typo).  Report it, then fall through
        // to the dead-`false` recovery below so codegen stays width-safe (#493).
        if incompatible {
            diagnostic_at!(
                self.lexer,
                &pat_pos,
                Level::Error,
                "cannot match {} against pattern of type {}",
                subject_type.name(&self.data),
                lit_type.name(&self.data)
            );
        }
        // check for range pattern `lo..hi` or `lo..=hi`.
        if self.lexer.has_token("..") {
            let inclusive = self.lexer.has_token("=");
            // INC#31: reject open-end range `lo..` in match arms — same
            // silent-never-matches / native-codegen-crash trap as open-start.
            if self.lexer.peek_token("=>")
                || self.lexer.peek_token("|")
                || self.lexer.peek_token("if")
            {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "open-ended range pattern `lo..` is not supported in match arms — \
                     write the two-sided form `lo..hi` (exclusive) or `lo..=hi` (inclusive), \
                     or use a guard like `n if n >= lo`"
                );
                return (Value::Boolean(false), Type::Boolean);
            }
            let mut hi = Value::Null;
            self.expression(&mut hi);
            let mut lo_cond = Value::Null;
            self.call_op(
                &mut lo_cond,
                "<=",
                &[lit, Value::Var(subject_var)],
                &[subject_type.clone(), subject_type.clone()],
            );
            let mut hi_cond = Value::Null;
            self.call_op(
                &mut hi_cond,
                if inclusive { "<=" } else { "<" },
                &[Value::Var(subject_var), hi],
                &[subject_type.clone(), subject_type.clone()],
            );
            let range_cond = v_if(lo_cond, hi_cond, Value::Boolean(false));
            // An incompatible pattern type can never match (see above): the range
            // bounds were consumed for a clean parse, but the condition is dead.
            let cond = if incompatible {
                Value::Boolean(false)
            } else {
                range_cond
            };
            (
                v_block(vec![cond], Type::Boolean, "range_pattern"),
                Type::Boolean,
            )
        } else if incompatible {
            (
                v_block(vec![Value::Boolean(false)], Type::Boolean, "range_pattern"),
                Type::Boolean,
            )
        } else {
            (lit, lit_type)
        }
    }

    /// Parse a match expression over a scalar (integer, text, boolean, etc.).
    /// Builds an if/else chain: `if subject == lit1 { arm1 } else if subject == lit2 { arm2 } else { wildcard }`
    #[allow(clippy::too_many_lines)] // match-arm dispatch with pattern/guard/binding logic
    fn parse_scalar_match(
        &mut self,
        subject: Value,
        subject_type: &Type,
        code: &mut Value,
    ) -> Type {
        // Store subject in a temp var to avoid re-evaluation.
        let v = self.create_unique("match_subj", subject_type);
        self.vars.defined(v);

        self.lexer.token("{");

        // Collect arms: (literal_value, arm_code, arm_type, optional guard)
        let mut arms: Vec<(Option<Value>, Value, Type, Option<Value>)> = Vec::new();
        let mut has_wildcard = false;
        let mut result_type = Type::Void;

        loop {
            if self.lexer.peek_token("}") {
                break;
            }

            // Parse pattern: literal, `true`, `false`, `_`, `name @ pattern`, or string.
            let mut pattern_val: Option<Value> = None;
            let mut is_wildcard = false;
            let mut arm_bindings: Vec<Value> = Vec::new();

            // null pattern — matches when subject is null.
            if self.lexer.has_token("null") {
                let mut null_cond = Value::Null;
                self.call_op(
                    &mut null_cond,
                    "!",
                    &[Value::Var(v)],
                    std::slice::from_ref(subject_type),
                );
                // Wrap as a Block so build_scalar_chain recognizes it as a pre-built condition.
                pattern_val = Some(v_block(vec![null_cond], Type::Boolean, "null_pattern"));
            // Check for wildcard `_` or binding `name @ pattern`.
            } else if let Some(id) = self.lexer.has_identifier() {
                if id == "_" {
                    is_wildcard = true;
                } else if self.lexer.has_token("@") {
                    // binding pattern `name @ pattern` — bind the subject to
                    // a variable and continue parsing the sub-pattern.
                    let bind_nr = self.vars.add_variable(&id, subject_type, &mut self.lexer);
                    self.vars.defined(bind_nr);
                    arm_bindings.push(v_set(bind_nr, Value::Var(v)));
                    // Parse the sub-pattern after `@`.
                    let (pat, _) = self.parse_match_pattern(subject_type, v);
                    pattern_val = Some(pat);
                } else {
                    // Bare identifier without `@` — wildcard binding (binds subject to name).
                    let bind_nr = self.vars.add_variable(&id, subject_type, &mut self.lexer);
                    self.vars.defined(bind_nr);
                    arm_bindings.push(v_set(bind_nr, Value::Var(v)));
                    is_wildcard = true;
                }
            } else {
                let (pat, _) = self.parse_match_pattern(subject_type, v);
                pattern_val = Some(pat);
            }

            // or-patterns in scalar match — `1 | 2 | 3 => ...`
            while self.lexer.has_token("|") && !is_wildcard {
                let (next_pat, _) = self.parse_match_pattern(subject_type, v);
                if let Some(prev) = pattern_val.take() {
                    // Combine: build equality condition for prev, equality for next,
                    // then OR them: If(prev_eq, true, next_eq).
                    let mut prev_cond = Value::Null;
                    self.build_scalar_cond(&mut prev_cond, v, subject_type, prev);
                    let mut next_cond = Value::Null;
                    self.build_scalar_cond(&mut next_cond, v, subject_type, next_pat);
                    let or_cond = v_if(prev_cond, Value::Boolean(true), next_cond);
                    pattern_val = Some(v_block(vec![or_cond], Type::Boolean, "or_pattern"));
                }
            }

            // parse optional guard clause.
            let mut guard_opt = if self.lexer.has_token("if") {
                let mut guard_code = Value::Null;
                let guard_type = self.expression(&mut guard_code);
                if !self.first_pass && guard_type != Type::Boolean {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "guard must be boolean, got {}",
                        guard_type.name(&self.data)
                    );
                }
                Some(guard_code)
            } else {
                None
            };

            // Only mark exhaustive if wildcard has no guard.
            if is_wildcard && guard_opt.is_none() {
                has_wildcard = true;
            }

            self.expect_match_arm_arrow();
            let mut arm_code = Value::Null;
            let arm_expected = Self::match_arm_expected(&result_type);
            let arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_code);
            // A `null`-first arm must NOT pin the result to `Null` — promote to
            // the first CONCRETE arm's type (else `match c { false => null, true
            // => S{…} }` resolves to `Null`, `build_scalar_chain` can't type the
            // null sentinel, and the value arm returns null — silently wrong).
            // loft#978 — every arm can deliver this match's value, so the result carries
            // what ANY of them borrows.  A no-op on the first arm (nothing to join with);
            // on the later ones it stops an owned arm from erasing a borrowed sibling's dep.
            result_type = self.join_arm_into(&result_type, &arm_code, &arm_type);
            self.match_void_arm |= matches!(arm_type, Type::Void);
            if result_type == Type::Void || result_type == Type::Null {
                result_type = arm_type.clone();
            }
            // P209 — when the arm has both a guard and pattern bindings
            // (e.g. `x if x < 0 => …`), the guard must see the bound
            // variable.  Prepend the binding assignments to the guard
            // expression so the bound name is initialised before the
            // guard reads it.  Without this the guard saw the
            // uninitialised slot (typically 0), causing `x if x < 0`
            // to mis-fire and either skip the arm (interp) or fall
            // through to a sibling guard (`x == 0`) silently.  The
            // enum-variant struct-field path at the call site of
            // `build_scalar_chain` already wraps guards this way.
            if !arm_bindings.is_empty()
                && let Some(guard) = guard_opt.take()
            {
                let mut stmts = arm_bindings.clone();
                stmts.push(guard);
                guard_opt = Some(v_block(stmts, Type::Boolean, "binding_guard"));
            }
            // prepend any binding assignments (from `name @ pattern` or bare `name`)
            // to the arm body so the variable is assigned before the body executes.
            if !arm_bindings.is_empty() {
                arm_bindings.push(arm_code);
                arm_code = v_block(arm_bindings, arm_type.clone(), "binding_arm");
            }
            arms.push((pattern_val, arm_code, arm_type, guard_opt));
            if has_wildcard {
                self.lexer.has_token(","); // optional trailing comma
                // The enum path's twin — a total `_` matches everything, so an arm after it can
                // never be selected.  Breaking straight to the closing-brace expectation
                // reported "Expect token }" and then cascaded into four more errors about the
                // rest of the line, none of which named the wildcard.
                if !self.lexer.peek_token("}") {
                    if !self.first_pass {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "a `_` arm matches everything, so this arm can never be selected \
                             — move `_` to the end"
                        );
                    }
                    continue;
                }
                break;
            }
            if self.lexer.peek_token("}") {
                self.lexer.has_token(","); // optional trailing comma
            } else {
                self.lexer.token(","); // comma required between arms
            }
        }
        self.lexer.token("}");

        // `(M-Exhaust)` is stated for an enum, and a boolean is the one scalar whose domain
        // a match can spell out in full: `true` and `false`, each an unguarded arm, leave
        // nothing for a fall-through to answer.  Without this the chain ended in the typed
        // null every wildcard-less scalar match carries for the value no arm matched, and
        // the join read that leaf as a nullable arm — `-> text { match w { true => …,
        // false => … } }` warned nullable-into-non-null on both backends while the same
        // choice spelled as an `if` was quiet (loft#1343).  The last arm becomes the
        // fallback, exactly as a trailing `_` would.
        let bool_exhaustive = *subject_type == Type::Boolean
            && !has_wildcard
            && arms.iter().all(|(_, _, _, guard)| guard.is_none())
            && [true, false].iter().all(|b| {
                arms.iter()
                    .any(|(pat, _, _, _)| matches!(pat, Some(Value::Boolean(x)) if x == b))
            });
        let chain = self.build_scalar_chain(
            v,
            subject_type,
            has_wildcard || bool_exhaustive,
            &result_type,
            arms,
        );
        *code = v_block(
            vec![v_set(v, subject), chain],
            result_type.clone(),
            "scalar_match",
        );
        // loft#1019 — an arm that OWNS what it yields needs a home in this frame when
        // the merged type is a view (`Parser::own_joined_call_arms`).
        self.own_joined_call_arms(code, &result_type);
        result_type
    }

    /// Parse a match expression over a vector subject.
    /// Slice patterns: `[a, b] =>`, `[first, ..] =>`, `[.., last] =>`, `_ =>`.
    /// Each arm generates a length check and element bindings.
    #[allow(clippy::too_many_lines)] // slice pattern parsing with head/tail/rest dispatch
    /// @PLN35 PC1 — is `subject_type` a CURSOR-shaped struct: a `vector<T>` source field + an
    /// integer field named `pos`?  Returns `(struct_def, source_field_idx, pos_field_idx, T)`.
    /// Matching such a subject prefix-consumes; any other struct falls through to the struct handler.
    fn cursor_shape(&self, subject_type: &Type) -> Option<(u32, usize, usize, Type)> {
        let d_nr = match subject_type {
            Type::Reference(d, _) if self.data.def_type(*d) == DefType::Struct => *d,
            _ => return None,
        };
        let attrs = self.data.def(d_nr).attributes();
        let mut source: Option<(usize, Type)> = None;
        let mut pos: Option<usize> = None;
        for (i, a) in attrs.iter().enumerate() {
            if a.constant {
                continue;
            }
            if let Type::Vector(elem, _) = &a.typedef {
                if source.is_none() {
                    source = Some((i, (**elem).clone()));
                }
            } else if matches!(a.typedef, Type::Integer(_)) && a.name == "pos" {
                pos = Some(i);
            }
        }
        match (source, pos) {
            (Some((si, elm)), Some(pi)) => Some((d_nr, si, pi, elm)),
            _ => None,
        }
    }

    /// @PLN35 PC1 — match over a CURSOR: prefix-consume its source from `pos`, advancing `cursor.pos`
    /// by the consumed count on a match.  Reads source + pos into temps, sets `match_cursor` so the
    /// slice machinery goes prefix-relative (`read_slice_elem` offsets by `pos`, the length gate is
    /// `pos + fixed <= len`), runs the normal vector-match over the source, then clears the mode.
    fn parse_cursor_match(
        &mut self,
        subject: Value,
        cursor_def: u32,
        source_field: usize,
        pos_field: usize,
        elm_tp: &Type,
        code: &mut Value,
    ) -> Type {
        // Use the subject's OWN var as the cursor — a struct is a DbRef, so advancing its `pos`
        // must reach the caller's cursor (same rule the struct-match path follows).  Copy only a
        // non-var subject (a temporary cursor's advance is then moot, which is fine).
        let (cursor_var, copy) = if let Value::Var(cv) = &subject {
            (*cv, false)
        } else {
            let t = self.create_unique("cursor", &Type::Reference(cursor_def, Deps::none()));
            self.vars.defined(t);
            (t, true)
        };
        // `cursor.src` is a vector living INSIDE the caller-owned cursor record, so the read
        // is a BORROWED view (Deps::frame1(cursor_var)), not an owned temporary. It must NOT
        // be freed: freeing it frees the cursor's whole record — a use-after-free that lets a
        // later allocation reuse the slot and silently corrupt the caller's cursor (surfaced
        // by an arm that returns a freshly-built struct then rebinds it). Mirrors the heap-
        // capture borrowed-VIEW path (skip_free + a borrow dep on the subject source).
        let borrowed_vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::frame1(cursor_var));
        let source_var = self.create_unique("cursor_src", &borrowed_vec_tp);
        self.vars.defined(source_var);
        self.vars.set_skip_free(source_var);
        let pos_var = self.create_unique("cursor_pos", &I32);
        self.vars.defined(pos_var);
        let read_source = self.get_field(cursor_def, source_field, Value::Var(cursor_var));
        let read_pos = self.get_field(cursor_def, pos_field, Value::Var(cursor_var));
        let mut setup = Vec::new();
        if copy {
            setup.push(v_set(cursor_var, subject));
        }
        setup.push(v_set(source_var, read_source));
        setup.push(v_set(pos_var, read_pos));
        self.match_cursor = Some((cursor_var, cursor_def, pos_field, pos_var));
        // @PLN35 PC5 — an OPTIONAL `farthest: integer` field is a monotonic high-water mark the
        // match maintains at every advance (opt-in; a plain `{ src, pos }` cursor has none).
        self.match_cursor_farthest = self.data.def(cursor_def).attributes().iter().position(|a| {
            !a.constant && matches!(a.typedef, Type::Integer(_)) && a.name == "farthest"
        });
        let mut match_code = Value::Null;
        let result_tp =
            self.parse_vector_match(Value::Var(source_var), &borrowed_vec_tp, &mut match_code);
        self.match_cursor = None;
        self.match_cursor_farthest = None;
        setup.push(match_code);
        *code = v_block(setup, result_tp.clone(), "cursor match");
        result_tp
    }

    /// @PLN35 PC2 — is the upcoming slice element `name : rule`, where `rule` names a sub-rule
    /// FUNCTION (`n_<rule>` returning a struct)?  ONE save/revert look-ahead, mirroring
    /// `peek_scalar_type_capture`.  A variant name (`name : Variant`) has no `n_<...>` fn, so this
    /// stays disjoint from the sub-pattern path.  Returns `(name, fn_def_nr, return_type)`.
    fn peek_subrule_capture(&mut self) -> Option<(String, u32, Type)> {
        let name = match &self.lexer.peek().has {
            LexItem::Identifier(id) if id != "_" => id.clone(),
            _ => return None,
        };
        let save = self.lexer.link();
        let mut result = None;
        self.lexer.cont(); // name
        if self.lexer.peek_token(":") {
            self.lexer.cont(); // `:`
            if let LexItem::Identifier(rule) = self.lexer.peek().has.clone() {
                let fn_nr = self.data.def_nr(&format!("n_{rule}"));
                if fn_nr != u32::MAX {
                    let ret = self.data.def(fn_nr).returned().clone();
                    if matches!(ret, Type::Reference(..)) {
                        result = Some((name, fn_nr, ret));
                    }
                }
            }
        }
        self.lexer.revert(save);
        result
    }

    /// @PLN35 PC2 — emit a sub-rule invocation slice element `[ name: rule ]` (cursor mode,
    /// whole-pattern).  Desugars to the proven hand-form `r = rule(cursor); if r != null { name = r
    /// … }`: the sub-rule prefix-matches over the SAME cursor (advancing its `pos` on a match,
    /// leaving it on a miss), so no fixed-length gate or pos-advance is needed here (`multi_alt`
    /// skips them).  The call is evaluated ONCE inside the arm cond (a side-effecting block) so a
    /// miss never double-advances; `name` binds the (owned) result and the arm gates on `name != null`.
    fn parse_subrule_slice_element(
        &mut self,
        name: &str,
        fn_nr: u32,
        ret_tp: &Type,
        cond: &mut Option<Value>,
        prechain: &mut Vec<Value>,
    ) {
        let cursor_var = self.match_cursor.map_or(0, |(cv, ..)| cv);
        // Re-read `name : rule` (the peek reverted); then require it be the whole pattern.
        self.lexer.has_identifier(); // name
        self.lexer.token(":");
        // @PLN35 PC3 — record the sub-rule edge (enclosing rule -> invoked rule) at the invocation
        // site, so the post-parse termination pass can reject a left-recursive cycle.
        let site = self.lexer.peek().position.clone();
        self.lexer.has_identifier(); // rule
        if !self.first_pass && self.context != u32::MAX {
            self.subrule_edges.push((self.context, fn_nr, site));
        }
        if !self.first_pass && !self.lexer.peek_token("]") {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a sub-rule element `{name}: rule` must currently be the whole slice pattern \
                 (mixing a sub-rule with fixed elements is deferred to a follow-up)"
            );
        }
        while !self.lexer.peek_token("]") && !matches!(self.lexer.peek().has, LexItem::None) {
            self.lexer.cont();
        }
        self.lexer.token("]");
        let name_var = self.vars.add_variable(name, ret_tp, &mut self.lexer);
        self.vars.defined(name_var);
        let cursor_tp = self.vars.tp(cursor_var).clone();
        let mut call = Value::Null;
        self.call_nr(
            &mut call,
            fn_nr,
            &[Value::Var(cursor_var)],
            &[cursor_tp],
            false,
            &[],
            None,
        );
        // Hoist `name = rule(cursor)` above the if-chain: the call runs ONCE (a miss leaves `pos`
        // unchanged, so unconditional evaluation is safe), and `name` is a match-level binding
        // visible to every arm body (a cond sub-block would scope it away on native).  The arm
        // then gates on `name != null`.
        prechain.push(v_set(name_var, call));
        // Presence test == the hand-form `name != null`: `OpNeRef(name, OpNullRefSentinel())`.
        // (NOT `OpRefIsNull` — that tests the store_nr sentinel, for `enum == null`, and misreads
        // a fn-returned null struct on the interpreter; `OpNeRef` vs the sentinel is the ref path.)
        let sentinel = self.cl("OpNullRefSentinel", &[]);
        let present = self.cl("OpNeRef", &[Value::Var(name_var), sentinel]);
        *cond = match cond.take() {
            Some(existing) => Some(v_if(existing, present, Value::Boolean(false))),
            None => Some(present),
        };
    }

    /// @PLN35 PC3+PC4 — well-formedness pass over the sub-rule invocation graph, run post-parse over
    /// the edges recorded on pass 2.  **PC3 (termination):** every PC2 invocation `[ name: rule ]` is
    /// a WHOLE pattern, so the sub-rule runs at cursor position 0 (nothing consumed) and its call is
    /// hoisted unconditionally — a CYCLE therefore recurses forever (no base-case arm can intervene),
    /// so reject any cycle as left recursion.  **PC4 (purity):** an invoked sub-rule must be pure —
    /// the hoisted, possibly-backtracked call makes any observable side effect leak.
    pub(crate) fn check_subrule_wellformedness(&mut self) {
        // Drain the edges so a re-parse (REPL / multiple files) starts fresh.
        let edges = std::mem::take(&mut self.subrule_edges);
        if edges.is_empty() {
            return;
        }
        let mut adj: std::collections::HashMap<u32, Vec<(u32, crate::lexer::Position)>> =
            std::collections::HashMap::new();
        for (from, to, pos) in &edges {
            adj.entry(*from).or_default().push((*to, pos.clone()));
        }
        let cycles = Self::find_subrule_cycles(&adj);
        for (site, cycle) in &cycles {
            let path = cycle
                .iter()
                .map(|nr| Self::rule_display_name(&self.data, *nr))
                .collect::<Vec<_>>()
                .join(" -> ");
            let head = Self::rule_display_name(&self.data, cycle[0]);
            diagnostic_at!(
                self.lexer,
                site,
                Level::Error,
                "sub-rule `{head}` is left-recursive ({path}): a cursor `match` invokes a sub-rule \
                 before consuming any input, so this cycle would recurse forever"
            );
        }
        // @PLN35 PC4 — an invoked sub-rule must be PURE: a cursor `match` hoists its call
        // unconditionally (so it runs even when the arm is not taken) and may backtrack over it, so
        // an observable side effect (I/O, host mutation, prng) would leak.  Skip callees already in
        // a rejected cycle (that error stands).  One report per impure callee.
        let in_cycle: HashSet<u32> = cycles.iter().flat_map(|(_, c)| c.iter().copied()).collect();
        let mut checked: HashSet<u32> = HashSet::new();
        for (_from, callee, site) in &edges {
            if in_cycle.contains(callee) || !checked.insert(*callee) {
                continue;
            }
            if !crate::scopes::sub_rule_is_pure(&self.data, *callee) {
                let name = Self::rule_display_name(&self.data, *callee);
                diagnostic_at!(
                    self.lexer,
                    site,
                    Level::Error,
                    "sub-rule `{name}` is not pure — a cursor `match` may invoke it speculatively \
                     (even when its arm is not taken) and backtrack over it, so its side effects \
                     would be observable; a sub-rule must only advance the cursor and return (no \
                     I/O, host mutation, or randomness)"
                );
            }
        }
    }

    /// @PLN130 F9 / [loft#779](https://github.com/loft-lang/loft/issues/779) — refuse the one
    /// program shape where a `&` reference could not write through: a container reshaped while
    /// a reference into it is still live.
    ///
    /// Post-parse over pass 2, like [`Self::check_subrule_wellformedness`], and for the same
    /// reason: the question is asked of a CALLEE's body (*"does it remove from `&` parameter
    /// k?"*), which is only complete once the whole file is parsed. Runs on the program parse
    /// only — the stdlib's definitions are still in `Data` then, so a separate pass over the
    /// `default` load would only report them twice.
    ///
    /// The analysis is [`crate::scopes::reshape_refusals`]; this only turns its findings into
    /// diagnostics, because the collector lives on the lexer and the analysis does not.
    pub(crate) fn check_reshape_under_reference(&mut self) {
        for r in crate::scopes::reshape_refusals(&self.data) {
            let pos = crate::lexer::Position {
                file: r.file,
                line: r.line,
                pos: 1,
            };
            diagnostic_at!(self.lexer, &pos, Level::Error, "{}", r.message);
        }
    }

    /// The user-facing name of a rule fn (`n_expr` -> `expr`).
    fn rule_display_name(data: &crate::data::Data, nr: u32) -> String {
        let n = data.def(nr).name();
        n.strip_prefix("n_").unwrap_or(n).to_string()
    }

    /// DFS the sub-rule graph; return one `(site, cycle-path)` per distinct back-edge.  `cycle` is
    /// the closed node sequence `callee … node callee`; `site` the invocation position closing it.
    /// Nodes visited in sorted order for a deterministic report.
    fn find_subrule_cycles(
        adj: &std::collections::HashMap<u32, Vec<(u32, crate::lexer::Position)>>,
    ) -> Vec<(crate::lexer::Position, Vec<u32>)> {
        let mut color: std::collections::HashMap<u32, u8> = std::collections::HashMap::new();
        let mut reported: HashSet<(u32, u32)> = HashSet::new();
        let mut out: Vec<(crate::lexer::Position, Vec<u32>)> = Vec::new();
        let mut nodes: Vec<u32> = adj.keys().copied().collect();
        nodes.sort_unstable();
        for start in nodes {
            if color.get(&start).copied().unwrap_or(0) == 0 {
                Self::dfs_subrule(
                    start,
                    adj,
                    &mut color,
                    &mut Vec::new(),
                    &mut reported,
                    &mut out,
                );
            }
        }
        out
    }

    fn dfs_subrule(
        node: u32,
        adj: &std::collections::HashMap<u32, Vec<(u32, crate::lexer::Position)>>,
        color: &mut std::collections::HashMap<u32, u8>,
        path: &mut Vec<u32>,
        reported: &mut HashSet<(u32, u32)>,
        out: &mut Vec<(crate::lexer::Position, Vec<u32>)>,
    ) {
        color.insert(node, 1); // gray = on the current DFS path
        path.push(node);
        if let Some(edges) = adj.get(&node) {
            for (callee, pos) in edges {
                match color.get(callee).copied().unwrap_or(0) {
                    1 => {
                        // back-edge into a node on the path — a cycle
                        if reported.insert((node, *callee)) {
                            let idx = path.iter().position(|x| x == callee).unwrap_or(0);
                            let mut cycle = path[idx..].to_vec();
                            cycle.push(*callee);
                            out.push((pos.clone(), cycle));
                        }
                    }
                    0 => Self::dfs_subrule(*callee, adj, color, path, reported, out),
                    _ => {}
                }
            }
        }
        path.pop();
        color.insert(node, 2); // black = fully explored
    }

    fn parse_vector_match(
        &mut self,
        subject: Value,
        subject_type: &Type,
        code: &mut Value,
    ) -> Type {
        let elm_tp = subject_type.content();
        let v = self.create_unique("match_subj", subject_type);
        self.vars.defined(v);
        let elm_size = Value::Int(self.element_store_size(&elm_tp));
        // @PLN35 P-Cap-View — a heap element view's borrow-dep must name the SUBJECT's
        // source var (the caller's param/local), not the internal `_match_subj` copy `v`,
        // so the return-ownership chain reaches the caller's value. Fall back to `v` when
        // the subject isn't a plain var.
        let borrow_src = self.match_borrow_source(&subject).unwrap_or(v);

        self.lexer.token("{");
        let mut result_type = Type::Void;
        let mut arms: Vec<PatternArm> = Vec::new();
        let mut has_wildcard = false;
        // @PLN35 PC2 — statements hoisted BEFORE the arm if-chain (evaluated once, at match-level
        // scope so a binding is visible to every arm body — native scopes cond sub-blocks away).
        // A sub-rule call goes here: `name = rule(cursor)` runs unconditionally, then each arm's
        // cond null-checks `name` (mirrors the hand-form `r = rule(c); if r != null { … }`).
        let mut prechain: Vec<Value> = Vec::new();
        loop {
            if self.lexer.peek_token("}") {
                break;
            }
            let mut bindings: Vec<Value> = Vec::new();
            let mut cond: Option<Value> = None;
            // Set by a `_` or bare-name arm head: this pattern matches every subject.  It only
            // makes the MATCH exhaustive if no guard follows, so `has_wildcard` is decided after
            // the guard is parsed — `_ if c` can fail, and counting it as total would let a
            // subject fall through with no arm selected (loft#839).
            let mut is_total = false;
            // @PLN35 L2 — conditions from element sub-patterns (`[Variant { f }, ..]`); empty for
            // bare-name / `..` / `_` slice patterns, so those stay byte-identical.
            let mut elem_conds: Vec<Value> = Vec::new();
            if self.lexer.has_token("[") {
                // Parse slice pattern elements
                // @PLN35 Phase 4.3: set when a whole-slice MULTI-element alternation built the
                // arm `cond` itself (predictive dispatch); suppresses the fixed-length gate below.
                let mut multi_alt = false;
                let mut head: Vec<String> = Vec::new();
                let mut tail: Vec<String> = Vec::new();
                let mut has_rest = false;
                let mut rest_name: Option<String> = None;
                loop {
                    if self.lexer.has_token("]") {
                        break;
                    }
                    // @PLN35 — classify a `( … )` group ONCE per element (a second look-ahead
                    // over the same region corrupts the lexer replay buffer).  A `Repetition`
                    // may sit MID-slice (fixed head before it); an `Alt` is whole-slice only, so
                    // its branch keeps the `head.is_empty()` guard.
                    let group_kind = if !has_rest && self.lexer.peek_token("(") {
                        self.peek_group_kind()
                    } else {
                        SliceGroupKind::Other
                    };
                    if self.lexer.has_token("..") {
                        has_rest = true;
                        // @PLN35 Phase 2 (P-Rest) — `..name` with the name ADJACENT to `..` (no
                        // comma) captures the tail sub-slice as a FRESH vector. `.., x` (a comma
                        // before the name) stays a bare gap plus a tail binding. Named rest is
                        // tail-only (enforced after the loop).
                        if let Some(name) = self.lexer.has_identifier() {
                            rest_name = Some(name);
                        }
                    } else if self.match_cursor.is_some()
                        && head.is_empty()
                        && !has_rest
                        && let Some((name, fn_nr, ret_tp)) = self.peek_subrule_capture()
                    {
                        // @PLN35 PC2 — sub-rule invocation `[ name: rule ]` in cursor mode: call
                        // `rule(cursor)` (which prefix-matches over the SAME cursor, advancing its
                        // `pos` on a match and leaving it on a miss — PC1 semantics), bind `name` to
                        // the result, and gate the arm on `name != null`.  Whole-pattern only for
                        // now (the running-pos + revert needed to mix a variable-width sub-rule with
                        // fixed elements is deferred).  `multi_alt` skips the fixed length gate +
                        // pos-advance — the sub-rule owns the pos.
                        self.parse_subrule_slice_element(
                            &name,
                            fn_nr,
                            &ret_tp,
                            &mut cond,
                            &mut prechain,
                        );
                        multi_alt = true;
                        break;
                    } else if !has_rest
                        && !matches!(&elm_tp, Type::Enum(..))
                        && let Some(is_rep) = self.peek_scalar_type_capture()
                    {
                        // @PLN35 slice 1 — a scalar type-annotated capture `name:Type` (single —
                        // a type-as-match that always holds for the element type) OR its
                        // `name:Type*` / `+` repetition collected into a `vector<elm_tp>`.  Only
                        // for a SCALAR element type; enum elements keep the `name:pat` /
                        // `(x: V)*` paths below.
                        if is_rep {
                            let head_len = head.len() as i32;
                            // Bind any BARE-NAME head element before the run (a literal / sub-
                            // pattern already emitted its own cond and left a "_").  Safe: the
                            // arm cond requires `head_len <= end <= len` before bindings run.
                            for (i, hname) in head.iter().enumerate() {
                                if hname == "_" {
                                    continue;
                                }
                                let bind_nr =
                                    self.vars.add_variable(hname, &elm_tp, &mut self.lexer);
                                self.vars.defined(bind_nr);
                                let val = self.read_slice_elem(
                                    v,
                                    &elm_size,
                                    &elm_tp,
                                    Value::Int(i as i32),
                                );
                                bindings.push(v_set(bind_nr, val));
                                self.mark_slice_element_view(bind_nr, &elm_tp, borrow_src);
                            }
                            self.parse_scalar_slice_repetition(
                                v,
                                &elm_size,
                                &elm_tp,
                                head_len,
                                &mut cond,
                                &mut bindings,
                            );
                            multi_alt = true;
                            break;
                        }
                        // Single `name:Type`: bind `name = v[pos]`; the annotation matches the
                        // element type so no condition is needed (a "_" keeps position alignment
                        // for following bare-name indices).
                        let position = head.len() as i32;
                        let name = self.lexer.has_identifier().unwrap();
                        self.lexer.token(":");
                        let tname = self.lexer.has_identifier().unwrap_or_default();
                        let elm_name = elm_tp.name(&self.data);
                        if !self.first_pass && tname != elm_name {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "a scalar capture `{name}:{tname}` must match the element type {elm_name}"
                            );
                        }
                        let bind_nr = self.vars.add_variable(&name, &elm_tp, &mut self.lexer);
                        self.vars.defined(bind_nr);
                        let val = self.read_slice_elem(v, &elm_size, &elm_tp, Value::Int(position));
                        bindings.push(v_set(bind_nr, val));
                        self.mark_slice_element_view(bind_nr, &elm_tp, borrow_src);
                        head.push("_".to_string());
                    } else if !has_rest && self.lexer.peek_named_arg().is_some() {
                        // @PLN35 Phase 2 (P-Cap) — `name:pat` element capture. Bind the whole
                        // element to `name` (a VIEW of the subject, exactly the read a bare head
                        // binding emits) AND require sub-pattern `pat` to match the same element.
                        // Head position only (before any `..`); a `_` placeholder keeps the
                        // position count aligned for following bare-name indices, as the variant
                        // sub-pattern branch does.
                        let name = self.lexer.has_identifier().unwrap();
                        self.lexer.token(":");
                        let position = head.len() as i32;
                        // Bind name = v[position] — the same read as the head-binding loop below.
                        let bind_nr = self.vars.add_variable(&name, &elm_tp, &mut self.lexer);
                        self.vars.defined(bind_nr);
                        let bval =
                            self.read_slice_elem(v, &elm_size, &elm_tp, Value::Int(position));
                        bindings.push(v_set(bind_nr, bval));
                        self.mark_slice_element_view(bind_nr, &elm_tp, borrow_src);
                        // Sub-pattern condition on the SAME element. Read v[position] AGAIN: the
                        // binding var is assigned only in the arm body (after the condition runs),
                        // so the condition must read the element directly, never the binding.
                        let cread =
                            self.read_slice_elem(v, &elm_size, &elm_tp, Value::Int(position));
                        let mut sub_conds: Vec<Value> = Vec::new();
                        let mut aliases: Vec<(String, Option<u16>)> = Vec::new();
                        if let Some(c) = self.parse_field_sub_pattern(
                            cread,
                            &elm_tp,
                            &mut bindings,
                            &mut sub_conds,
                            &mut aliases,
                        ) {
                            elem_conds.push(c);
                        }
                        elem_conds.append(&mut sub_conds);
                        head.push("_".to_string());
                    } else if !has_rest && {
                        // @PLN35 L2 — is this head element a VARIANT sub-pattern of the element
                        // enum type (`Ship { carrier }` / bare `Ship`)?  Peek without consuming so a
                        // plain binding name still falls through to the branch below.
                        if let Type::Enum(elm_e_nr, _, _) = &elm_tp {
                            if let LexItem::Identifier(pname) = &self.lexer.peek().has {
                                self.data.variant_of(*elm_e_nr, pname) != u32::MAX
                            } else {
                                false
                            }
                        } else {
                            false
                        }
                    } {
                        // Read v[pos] and tag-test + bind via parse_field_sub_pattern; a "_"
                        // placeholder keeps the position count so following bare-name indices align.
                        let position = head.len() as i32;
                        let read =
                            self.read_slice_elem(v, &elm_size, &elm_tp, Value::Int(position));
                        let mut sub_conds: Vec<Value> = Vec::new();
                        let mut aliases: Vec<(String, Option<u16>)> = Vec::new();
                        if let Some(c) = self.parse_field_sub_pattern(
                            read,
                            &elm_tp,
                            &mut bindings,
                            &mut sub_conds,
                            &mut aliases,
                        ) {
                            elem_conds.push(c);
                        }
                        elem_conds.append(&mut sub_conds);
                        head.push("_".to_string());
                    } else if group_kind == SliceGroupKind::Repetition {
                        // @PLN35 Phase 6 — a repetition `[ head…, ( [name:] V )*[(Sep)] [tail…]
                        // [, ..rest] ]` / `…+`.  `head` is any fixed prefix already parsed (its
                        // binds/conds are in `bindings`/`elem_conds`); the run starts at `head_len`
                        // and any fixed `tail` after the group is matched from the END.
                        // `parse_slice_repetition` builds the run-loop cond + collection and
                        // consumes through `]`, so break the element loop.
                        if let Type::Enum(elm_e_nr, true, _) = &elm_tp {
                            let e_nr = *elm_e_nr;
                            let head_len = head.len() as i32;
                            // Bind any BARE-NAME head element (a variant sub-pattern / literal
                            // already emitted its own bind/cond and left a "_").  These are safe:
                            // the arm's `cond` requires `len >= head_len` before bindings run.
                            for (i, name) in head.iter().enumerate() {
                                if name == "_" {
                                    continue;
                                }
                                let bind_nr =
                                    self.vars.add_variable(name, &elm_tp, &mut self.lexer);
                                self.vars.defined(bind_nr);
                                let val = self.read_slice_elem(
                                    v,
                                    &elm_size,
                                    &elm_tp,
                                    Value::Int(i as i32),
                                );
                                bindings.push(v_set(bind_nr, val));
                                self.mark_slice_element_view(bind_nr, &elm_tp, borrow_src);
                            }
                            self.parse_slice_repetition(
                                e_nr,
                                v,
                                &elm_size,
                                &elm_tp,
                                head_len,
                                borrow_src,
                                &mut cond,
                                &mut bindings,
                            );
                            multi_alt = true;
                        } else {
                            if !self.first_pass {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "a repetition `( … )*` slice element needs a struct-enum element type"
                                );
                            }
                            self.lexer.token("(");
                        }
                        break;
                    } else if group_kind == SliceGroupKind::Alt && head.is_empty() {
                        // @PLN35 Phase 4.3 / 5 — a WHOLE-SLICE MULTI-element alternation
                        // `[ (A B | C) [, ..rest] ]` OR an OPTIONAL group `[ (a)? [, ..rest] ]`
                        // (a degenerate alternation `(a | ε)`).  Predictive dispatch on the leading
                        // tags over a sequence per branch; it builds the arm `cond` itself and
                        // consumes through `]`, so break the element loop and skip the length gate.
                        if let Type::Enum(elm_e_nr, true, _) = &elm_tp {
                            let e_nr = *elm_e_nr;
                            self.parse_multi_element_alternation(
                                e_nr,
                                v,
                                &elm_size,
                                &elm_tp,
                                borrow_src,
                                &mut cond,
                                &mut bindings,
                            );
                            multi_alt = true;
                        } else {
                            if !self.first_pass {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "an alternation `( … | … )` slice element needs a struct-enum element type"
                                );
                            }
                            self.lexer.token("(");
                        }
                        break;
                    } else if !has_rest && self.lexer.peek_token("(") {
                        // @PLN35 Phase 4 (P-Alt) — a parenthesized single-element alternation
                        // `( V1 { f } | V2 { f } )` in a head slice-element position. Tag-test
                        // the element against each branch and bind the shared captures from the
                        // matching variant's offsets. A `_` placeholder keeps position alignment
                        // for following bare-name indices, as the variant sub-pattern branch does.
                        if let Type::Enum(elm_e_nr, true, _) = &elm_tp {
                            let e_nr = *elm_e_nr;
                            let position = head.len() as i32;
                            let read =
                                self.read_slice_elem(v, &elm_size, &elm_tp, Value::Int(position));
                            self.parse_slice_alternation_element(
                                e_nr,
                                &read,
                                borrow_src,
                                &mut bindings,
                                &mut elem_conds,
                            );
                            head.push("_".to_string());
                        } else {
                            if !self.first_pass {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "an alternation `( … | … )` slice element needs a struct-enum element type"
                                );
                            }
                            self.lexer.token("("); // consume to make progress
                            break;
                        }
                    } else if !has_rest && self.peek_is_slice_literal() {
                        // @PLN35 Phase 6.3 (P-Lit) — a LITERAL head element `[ 1, … ]` /
                        // `[ "kw", … ]`.  On a SCALAR element it matches by direct EQUALITY
                        // against `v[pos]`.  On a STRUCT-ENUM element (a token stream) it matches
                        // against the variant's `#lexeme` field — so `"fn"` reads like the
                        // grammar, standing in for `Keyword { name: "fn" }`.  A "_" placeholder
                        // keeps following bare-name indices AND the length gate aligned, and the
                        // condition is AND'd into the arm like a variant tag test.  Head only.
                        let position = head.len() as i32;
                        let mut lit = Value::Null;
                        let lit_tp = self.expression(&mut lit);
                        match self.build_literal_match(
                            v,
                            &elm_size,
                            &elm_tp,
                            &Value::Int(position),
                            &lit,
                            &lit_tp,
                        ) {
                            Some(c) => elem_conds.push(c),
                            None if !self.first_pass => {
                                self.slice_literal_mismatch(&elm_tp, &lit_tp)
                            }
                            None => {}
                        }
                        head.push("_".to_string());
                    } else if let Some(id) = self.lexer.has_identifier() {
                        if has_rest {
                            tail.push(id);
                        } else {
                            head.push(id);
                        }
                    } else {
                        // Unrecognized element token (`{`, an unsupported tail sub-pattern, …).
                        // None of the branches above consumed anything, so this MUST break to make
                        // progress — otherwise the loop spins forever on the same token in the
                        // first pass (where diagnostics are silent).
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "expected identifier, literal, or '..' in slice pattern"
                            );
                        }
                        break;
                    }
                    self.lexer.has_token(",");
                }
                // @PLN35 Phase 4.3: a whole-slice multi-element alternation built the arm cond +
                // bindings itself (predictive dispatch), so skip the fixed-position length gate,
                // the head/tail binds, and the rest materialisation below.
                if !multi_alt {
                    let fixed = (head.len() + tail.len()) as i32;
                    // Generate length condition
                    let len_call = self.cursor_len(v);
                    if let Some((cursor_var, cursor_def, pos_field, pos_var)) = self.match_cursor {
                        // @PLN35 PC1 — cursor PREFIX-consume: need `fixed` elements from `pos`
                        // (`pos + fixed <= len`), and on a match advance `cursor.pos` by `fixed`.
                        // `pos_var` stays the ENTRY position, so every read AND this write share it.
                        let ispec = Type::Integer(IntegerSpec {
                            min: 0,
                            max: 0,
                            not_null: false,
                            forced_size: None,
                        });
                        let need = self.conv_op(
                            "+",
                            Value::Var(pos_var),
                            Value::Int(fixed),
                            I32.clone(),
                            I32.clone(),
                        );
                        self.call_op(
                            cond.get_or_insert(Value::Null),
                            "<=",
                            &[need, len_call],
                            &[ispec.clone(), ispec],
                        );
                        let new_pos = self.conv_op(
                            "+",
                            Value::Var(pos_var),
                            Value::Int(fixed),
                            I32.clone(),
                            I32.clone(),
                        );
                        let adv = self.set_field(
                            cursor_def,
                            pos_field,
                            0,
                            Value::Var(cursor_var),
                            new_pos.clone(),
                        );
                        bindings.push(adv);
                        // @PLN35 PC5 — maintain the farthest high-water mark: `farthest =
                        // max(farthest, new_pos)`.  Monotonic across arms + sub-rules (the shared
                        // cursor carries it), so after a failed parse it names the deepest token
                        // reached.  Opt-in — only when the cursor has a `farthest` field.
                        if let Some(ff) = self.match_cursor_farthest {
                            let old_far = self.get_field(cursor_def, ff, Value::Var(cursor_var));
                            let is_less = self.conv_op(
                                "<",
                                old_far.clone(),
                                new_pos.clone(),
                                I32.clone(),
                                I32.clone(),
                            );
                            let maxed = v_if(is_less, new_pos, old_far);
                            let set_far =
                                self.set_field(cursor_def, ff, 0, Value::Var(cursor_var), maxed);
                            bindings.push(set_far);
                        }
                    } else if has_rest {
                        // length >= fixed  →  fixed <= length
                        self.call_op(
                            cond.get_or_insert(Value::Null),
                            "<=",
                            &[Value::Int(fixed), len_call],
                            &[
                                Type::Integer(IntegerSpec {
                                    min: 0,
                                    max: 0,
                                    not_null: false,
                                    forced_size: None,
                                }),
                                Type::Integer(IntegerSpec {
                                    min: 0,
                                    max: 0,
                                    not_null: false,
                                    forced_size: None,
                                }),
                            ],
                        );
                    } else {
                        // length == fixed
                        self.call_op(
                            cond.get_or_insert(Value::Null),
                            "==",
                            &[len_call, Value::Int(fixed)],
                            &[
                                Type::Integer(IntegerSpec {
                                    min: 0,
                                    max: 0,
                                    not_null: false,
                                    forced_size: None,
                                }),
                                Type::Integer(IntegerSpec {
                                    min: 0,
                                    max: 0,
                                    not_null: false,
                                    forced_size: None,
                                }),
                            ],
                        );
                    }
                    // Bind head elements: head[i] = v[i]
                    for (i, name) in head.iter().enumerate() {
                        if name == "_" {
                            continue;
                        }
                        let bind_nr = self.vars.add_variable(name, &elm_tp, &mut self.lexer);
                        self.vars.defined(bind_nr);
                        let val = self.read_slice_elem(v, &elm_size, &elm_tp, Value::Int(i as i32));
                        bindings.push(v_set(bind_nr, val));
                        self.mark_slice_element_view(bind_nr, &elm_tp, borrow_src);
                    }
                    // Bind tail elements: tail[j] = v[len - tail.len() + j]
                    for (j, name) in tail.iter().enumerate() {
                        if name == "_" {
                            continue;
                        }
                        let bind_nr = self.vars.add_variable(name, &elm_tp, &mut self.lexer);
                        self.vars.defined(bind_nr);
                        let idx = Value::Int(-((tail.len() - j) as i32));
                        let val = self.read_slice_elem(v, &elm_size, &elm_tp, idx);
                        bindings.push(v_set(bind_nr, val));
                        self.mark_slice_element_view(bind_nr, &elm_tp, borrow_src);
                    }
                    // @PLN35 Phase 2 (P-Rest) — `..name`: bind `name` to the FRESH sub-slice
                    // `v[head_len .. len - tail_len]`.  Reuse the proven compile-time slice
                    // materialisation (`materialize_iterator`): a minimal slice `Value::Iter` over the
                    // index range, copied element-type-aware into a fresh vector (P-Cap-Fresh — the
                    // result is INDEPENDENT of the subject, so it is safe to return or mutate). Named
                    // rest is tail-only.  Bounds are in range by the `fixed <= len` arm condition.
                    if let Some(name) = rest_name.clone() {
                        if !tail.is_empty() && !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "a named rest `..{name}` must be the last slice element"
                            );
                        }
                        let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
                        let rest_var = self.vars.add_variable(&name, &vec_tp, &mut self.lexer);
                        self.vars.defined(rest_var);
                        if !self.first_pass && rest_var != u16::MAX {
                            let lo_val = Value::Int(head.len() as i32);
                            let len_c = self.cursor_len(v);
                            let hi_val = self.conv_op(
                                "-",
                                len_c,
                                Value::Int(tail.len() as i32),
                                I32.clone(),
                                I32.clone(),
                            );
                            self.materialize_named_rest(
                                v,
                                &elm_tp,
                                &elm_size,
                                rest_var,
                                &vec_tp,
                                lo_val,
                                hi_val,
                                1,
                                &mut bindings,
                            );
                        }
                    }
                } // end `if !multi_alt`
                // @PLN35 L2 — AND element sub-pattern conditions into the arm condition AFTER the
                // length check, so `&&` short-circuit never reads past the end.
                for ec in elem_conds.drain(..) {
                    match cond.take() {
                        // `c && ec` via short-circuit if (the AND idiom used elsewhere in match
                        // lowering) — avoids reading v[pos] when the length check already failed.
                        Some(c) => cond = Some(v_if(c, ec, Value::Boolean(false))),
                        None => cond = Some(ec),
                    }
                }
            } else if let Some(id) = self.lexer.has_identifier() {
                if id == "_" {
                    is_total = true;
                } else {
                    // bare name — wildcard binding
                    let bind_nr = self.vars.add_variable(&id, subject_type, &mut self.lexer);
                    self.vars.defined(bind_nr);
                    bindings.push(v_set(bind_nr, Value::Var(v)));
                    is_total = true;
                }
            } else {
                // Unrecognized arm head (not `[…]`, `_`, or a binding). No branch above
                // consumed a token, so break to make progress — the arm loop would
                // otherwise spin forever on this token in the first pass (diagnostics silent).
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expected slice pattern '[...]' or '_' in vector match arm"
                    );
                }
                break;
            }
            // Parse the optional guard.  Captures bound by the pattern above are in scope
            // for it, and `chain_pattern_arms` assigns them before the guard runs.
            let mut guard_opt = if self.lexer.has_token("if") {
                let mut guard = Value::Null;
                let gt = self.expression(&mut guard);
                if !self.first_pass && gt != Type::Boolean {
                    self.convert(&mut guard, &gt, &Type::Boolean);
                }
                Some(guard)
            } else {
                None
            };
            // @PLN35 — a PEG cursor arm ADVANCES the shared cursor from its `bindings`, which
            // run before the guard.  A guard that then fails would leave the cursor consumed
            // while the next arm re-matches from the wrong position, so the whole parse would
            // silently read the wrong tokens.  Refuse the combination rather than ship that:
            // the guard's test belongs in the arm body, where the cursor is already committed.
            if guard_opt.is_some() && self.match_cursor.is_some() {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "a guard is not supported on a cursor match arm — it would advance the cursor before the guard is tested; test inside the arm body instead"
                    );
                }
                guard_opt = None;
            }
            if is_total && guard_opt.is_none() {
                has_wildcard = true;
            }
            self.expect_match_arm_arrow();
            let mut arm_code = Value::Null;
            // #556 — a `{ … }` arm body is a VALUE position (its result is the match value), so
            // parse it as a `match_arm` block with an INFERRED (`Unknown`) result type, exactly
            // as the enum/scalar match arm does.  Routing it through `self.expression` instead
            // delegated to `parse_block("block", …, &Type::Void)`, which DROPS the trailing
            // result expression (`{ c = 5; c }` → `c = 5; drop c`) — the block then yielded void,
            // so native delivered 0 (interpret happened to still surface the value).
            let arm_expected = Self::match_arm_expected(&result_type);
            let arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_code);
            // loft#978 — every arm can deliver this match's value, so the result carries
            // what ANY of them borrows.  A no-op on the first arm (nothing to join with);
            // on the later ones it stops an owned arm from erasing a borrowed sibling's dep.
            result_type = self.join_arm_into(&result_type, &arm_code, &arm_type);
            self.match_void_arm |= matches!(arm_type, Type::Void);
            if result_type == Type::Void {
                result_type = arm_type.clone();
            }
            // Without a guard the bindings fold into the body exactly as before; with one they
            // stay separate so `chain_pattern_arms` can run them ahead of the guard.
            let arm_bindings = if guard_opt.is_some() {
                std::mem::take(&mut bindings)
            } else {
                if !bindings.is_empty() {
                    bindings.push(arm_code);
                    arm_code = v_block(bindings, arm_type.clone(), "slice_binding");
                }
                Vec::new()
            };
            arms.push(PatternArm {
                cond,
                guard: guard_opt,
                bindings: arm_bindings,
                code: arm_code,
            });
            if has_wildcard {
                self.lexer.has_token(",");
                break;
            }
            if self.lexer.peek_token("}") {
                self.lexer.has_token(",");
            } else {
                self.lexer.token(",");
            }
        }
        self.lexer.token("}");

        // @PLN35 Phase 2 (F6 / M-Total): a slice pattern is length-constrained, so it is
        // non-total — a well-typed vector can always have a length no fixed arm matches.
        // The match is exhaustive only if its final arm is total (a `_` or a bare binding,
        // both of which set `has_wildcard` and force the arm last). Without one, a subject
        // would fall through with no arm selected; reject it statically, mirroring the enum
        // exhaustiveness gate in `parse_match`.
        if !self.first_pass && !has_wildcard {
            diagnostic!(
                self.lexer,
                Level::Error,
                "match on vector is not exhaustive — a slice pattern can fail (a length no arm matches); add a '_ =>' or a bare-binding final arm"
            );
        }

        // Build if-else chain from arms
        let fallback = if has_wildcard {
            arms.pop().unwrap().code
        } else {
            self.null_value(&result_type)
        };
        let mut chain = chain_pattern_arms(arms, fallback, &result_type);
        // @PLN85 — a bare `[]` arm (`_ => []`) lowers to a `null` of the result type, which the
        // native backend emits as `()` where a vector (`DbRef`) is expected.  In a RETURN context
        // the delivery renames it onto `__retbuf`; but when the match value is BOUND to a local
        // (`cap = match v { … , _ => [] }`, copy-on-bind in `parse_assign_op`) the copy's
        // `OpAppendVector(cap, <match>)` needs every arm to be a real vector.  Materialise each
        // null arm as a FRESH empty vector — ONLY when the match RESULT is a vector (a
        // cursor/prefix match that returns a struct/scalar, `[ n: rule ] => …, _ => null`, keeps
        // its genuine `null` arm; the result's OWN element type is used, not the subject's).
        if !self.first_pass
            && let Type::Vector(result_elm, _) = result_type.clone()
        {
            self.materialize_null_slice_arms(&mut chain, &result_elm);
        }
        let mut block_ops = vec![v_set(v, subject)];
        block_ops.append(&mut prechain);
        block_ops.push(chain);
        *code = v_block(block_ops, result_type.clone(), "vector_match");
        // loft#1019 — an arm that OWNS what it yields needs a home in this frame when
        // the merged type is a view (`Parser::own_joined_call_arms`).
        self.own_joined_call_arms(code, &result_type);
        result_type
    }

    /// Parse a `match` expression whose subject is a `Type::Tuple`.
    ///
    /// Arm syntax: `_ => expr` (wildcard) or `(pat0, pat1, ...) => expr` (element patterns).
    /// Element patterns: `_` (wildcard), `identifier` (binding), or a literal value.
    /// Arms are separated by `,` or `;` (optional after the last arm).
    #[allow(clippy::too_many_lines)]
    fn parse_tuple_match(&mut self, subject: Value, subject_type: &Type, code: &mut Value) -> Type {
        let Type::Tuple(elem_types) = subject_type else {
            unreachable!("parse_tuple_match called with non-tuple subject")
        };
        let elem_types = elem_types.clone();
        let arity = elem_types.len();

        // Store the tuple in a temp var so elements can be read multiple times.
        let tmp = self.create_unique("match_tuple", subject_type);
        self.vars.defined(tmp);

        self.lexer.token("{");

        let mut arms: Vec<PatternArm> = Vec::new();
        let mut has_wildcard = false;
        let mut result_type = Type::Void;

        loop {
            if self.lexer.peek_token("}") {
                break;
            }
            // Every iteration must consume at least one token.  A pattern the element
            // loop below cannot make sense of leaves the cursor parked mid-arm, and
            // `expect_match_arm_arrow`'s `recover_to` cannot rescue it: that helper
            // resynchronises, and returns WITHOUT consuming when the cursor already
            // sits on one of its stop tokens or on an unmatched closer.  The arm then
            // re-parses the same token forever — silently, because first-pass
            // diagnostics are suppressed (loft#832).  Compared against `arm_start` at
            // the bottom of the loop.
            let arm_start = self.lexer.at();

            let mut is_wildcard = false;
            // A pattern that was REFUSED binds nothing and tests nothing, which reads
            // exactly like `(_, _, _)` at the classification below — and a wildcard arm
            // ends the arm loop, so a rejected first arm would swallow every arm after
            // it and report a missing `}` instead of the refusal (loft#832).
            let mut bad_pattern = false;
            let mut bindings: Vec<Value> = Vec::new();
            let mut elem_conds: Vec<Value> = Vec::new();

            if let Some(id) = self.lexer.has_identifier() {
                if id == "_" {
                    is_wildcard = true;
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "expected '_' or a tuple pattern '(...)' in tuple match"
                    );
                }
            } else if self.lexer.has_token("(") {
                // Element-by-element pattern
                for (i, elem_type) in elem_types.iter().enumerate().take(arity) {
                    // The break is NOT gated on the pass: both passes must walk the
                    // arm the same way, or the first one wanders into positions the
                    // second never visits and stops making progress.
                    if i > 0 && !self.lexer.has_token(",") {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "expected ',' between tuple pattern elements"
                            );
                        }
                        break;
                    }
                    // A `..` rest is not part of the tuple design — arity is fixed
                    // (TUPLES.md § "What is NOT supported"), so there is nothing for a
                    // rest to stand for.  Refuse it by name and skip to the closing
                    // `)`, rather than letting it fall through to the literal branch
                    // below, where `expression` consumes the `..` but leaves the `)`
                    // unclaimed and the arm loop spinning (loft#832).
                    if self.lexer.peek_token("..") || self.lexer.peek_token("..=") {
                        bad_pattern = true;
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "a `..` rest pattern is not supported in a tuple pattern — \
                                 a tuple's arity is fixed, so write every position, \
                                 using `_` for the ones you do not bind"
                            );
                        }
                        self.lexer.has_token("..=");
                        self.lexer.has_token("..");
                        self.lexer.recover_to(&[")"]);
                        break;
                    }
                    let elem_type = elem_type.clone();
                    let elem_get = Value::TupleGet(tmp, i as u16);
                    if let Some(id) = self.lexer.has_identifier() {
                        if id == "_" {
                            // element wildcard — no condition, no binding
                        } else {
                            // binding variable — always matches, captures element value
                            let bind_nr = self.vars.add_variable(&id, &elem_type, &mut self.lexer);
                            self.vars.defined(bind_nr);
                            bindings.push(v_set(bind_nr, elem_get));
                        }
                    } else {
                        // literal: build elem_get == literal condition
                        let negate = self.lexer.has_token("-");
                        let lit: Value = if let Some(n) = self.lexer.has_integer() {
                            let v = n as i32;
                            Value::Int(if negate { -v } else { v })
                        } else if let Some(n) = self.lexer.has_long() {
                            let v = n as i64;
                            Value::Long(if negate { -v } else { v })
                        } else if let Some(n) = self.lexer.has_float() {
                            Value::Float(if negate { -n } else { n })
                        } else if let Some(s) = self.lexer.has_cstring() {
                            Value::Text(s)
                        } else if self.lexer.has_token("true") {
                            Value::Boolean(true)
                        } else if self.lexer.has_token("false") {
                            Value::Boolean(false)
                        } else {
                            let mut e = Value::Null;
                            self.expression(&mut e);
                            e
                        };
                        let mut elem_cond = Value::Null;
                        self.call_op(
                            &mut elem_cond,
                            "==",
                            &[elem_get, lit],
                            &[elem_type.clone(), elem_type],
                        );
                        elem_conds.push(elem_cond);
                    }
                }
                if !self.lexer.has_token(")") {
                    bad_pattern = true;
                    if !self.first_pass {
                        // A `,` here means the pattern listed MORE elements than the
                        // subject has; anything else is ordinary junk.  Naming the
                        // arity is what tells the author which side to change.
                        if self.lexer.peek_token(",") {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "tuple pattern has more elements than the {}-element subject tuple",
                                arity
                            );
                        } else {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "expected ')' to close tuple pattern"
                            );
                        }
                    }
                    // Skip the surplus so the arm reaches its `=>` — without this the
                    // cursor stays parked on the `,` and the arm loop spins (loft#832).
                    self.lexer.recover_to(&[")"]);
                    self.lexer.has_token(")");
                }
                // All element positions were wildcards/bindings with no literal conditions.
                // The arm is effectively unconditional (wildcard) when there are no bindings
                // either; if there are bindings it acts like a wildcard-with-capture.
                if elem_conds.is_empty() && bindings.is_empty() && !bad_pattern {
                    is_wildcard = true;
                }
            } else if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "expected '_' or a tuple pattern '(...)' in tuple match"
                );
            }

            // Optional guard clause — `tuple-arm ::= tuple-pattern [ guard ] '=>' expression`
            // (TUPLES.md § Grammar).  Element bindings are in scope for it, and
            // `chain_pattern_arms` assigns them before the guard runs.
            let guard_opt = if self.lexer.has_token("if") {
                let mut g = Value::Null;
                let gt = self.expression(&mut g);
                if !self.first_pass && gt != Type::Boolean {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "guard must be boolean, got {}",
                        gt.name(&self.data)
                    );
                }
                Some(g)
            } else {
                None
            };

            if is_wildcard && guard_opt.is_none() {
                has_wildcard = true;
            }

            self.expect_match_arm_arrow();

            let arm_write_state = self.vars.save_and_clear_write_state();
            self.vars.clear_write_state();
            let mut arm_body = Value::Null;
            let arm_expected = Self::match_arm_expected(&result_type);
            let arm_type = self.parse_match_arm_body(&arm_expected, &mut arm_body);
            self.vars.restore_write_state(&arm_write_state);

            // Combine element conditions with AND (short-circuit: if a { b } else { false })
            let cond: Option<Value> = if elem_conds.is_empty() {
                None
            } else {
                let mut combined = elem_conds.remove(0);
                for c in elem_conds {
                    combined = v_if(combined, c, Value::Boolean(false));
                }
                Some(combined)
            };

            // Without a guard the bindings fold into the body exactly as before; with one they
            // stay separate so `chain_pattern_arms` can run them ahead of the guard.
            let (arm_body, arm_bindings) = if guard_opt.is_some() {
                (arm_body, bindings)
            } else if bindings.is_empty() {
                (arm_body, Vec::new())
            } else {
                bindings.push(arm_body);
                (
                    v_block(bindings, arm_type.clone(), "tuple_binding"),
                    Vec::new(),
                )
            };

            // loft#978 — every arm can deliver this match's value, so the result carries
            // what ANY of them borrows.  A no-op on the first arm (nothing to join with);
            // on the later ones it stops an owned arm from erasing a borrowed sibling's dep.
            result_type = self.join_arm_into(&result_type, &arm_body, &arm_type);
            self.match_void_arm |= matches!(arm_type, Type::Void);
            if result_type == Type::Void {
                result_type = arm_type.clone();
            }
            arms.push(PatternArm {
                cond,
                guard: guard_opt,
                bindings: arm_bindings,
                code: arm_body,
            });

            if has_wildcard {
                self.lexer.has_token(",");
                self.lexer.has_token(";");
                break;
            }
            if self.lexer.peek_token("}") {
                self.lexer.has_token(",");
                self.lexer.has_token(";");
            } else {
                // optional arm separator
                self.lexer.has_token(",");
                self.lexer.has_token(";");
            }
            // Backstop for the invariant declared at `arm_start`: a whole arm parsed
            // without consuming a token means no later iteration can consume one
            // either, so stop instead of spinning.  The shapes known to reach here
            // are handled above with their own diagnostics; this catches the ones
            // nobody has written a probe for yet.  `token("}")` below reports the
            // arm loop's failure to reach a close brace, as the vector arm loop's
            // own no-progress `break` does.
            if self.lexer.at() == arm_start {
                break;
            }
        }
        self.lexer.token("}");

        // Build if-else chain (last arm is fallback / wildcard)
        let fallback = if has_wildcard {
            arms.pop().unwrap().code
        } else {
            self.null_value(&result_type)
        };
        let chain = chain_pattern_arms(arms, fallback, &result_type);

        *code = v_block(
            vec![v_set(tmp, subject), chain],
            result_type.clone(),
            "tuple_match",
        );
        // loft#1019 — an arm that OWNS what it yields needs a home in this frame when
        // the merged type is a view (`Parser::own_joined_call_arms`).
        self.own_joined_call_arms(code, &result_type);
        result_type
    }

    /// Build a boolean condition for a single scalar pattern value.
    fn build_scalar_cond(&mut self, cond: &mut Value, v: u16, subject_type: &Type, pat: Value) {
        // Reuse the same logic as build_scalar_chain for special block patterns.
        if let Value::Block(ref bl) = pat
            && bl.result == Type::Boolean
            && (bl.name == "range_pattern" || bl.name == "null_pattern" || bl.name == "or_pattern")
        {
            *cond = bl.operators[0].clone();
            return;
        }
        self.call_op(
            cond,
            "==",
            &[Value::Var(v), pat],
            &[subject_type.clone(), subject_type.clone()],
        );
    }

    /// Build the if-chain for a scalar match from collected arms.
    fn build_scalar_chain(
        &mut self,
        v: u16,
        subject_type: &Type,
        has_wildcard: bool,
        result_type: &Type,
        mut arms: Vec<(Option<Value>, Value, Type, Option<Value>)>,
    ) -> Value {
        // A bare `null` arm value (`false => null`) parses to `Value::Null`, which
        // lowers to NO push (Type::Void).  In a value-producing match the if-chain
        // join then reads an unwritten, value-sized slot — interp stack underflow
        // ("No elements left on the stack"), native a lost value.  Convert each
        // bare-null arm to the result type's typed null sentinel, the same
        // transform `parse_if` applies to a null branch and the
        // fallback gets just below.  `null_value` is a no-op (returns
        // `Value::Null`) for Void/Unknown result types, so a statement-style
        // match is untouched.  Compute the typed null once (it's the same for
        // every arm — `result_type` is fixed), releasing the `&mut self` borrow
        // before mutating `arms`.
        //
        // loft#936 — `null_value` and not `null`, at every branch-MERGE slot in
        // this file.  `null` also supplies a VARIABLE's default-init, where a
        // collection has to be an allocated empty store, so its catch-all
        // answers a bare `Value::Null` for the entire collection family and
        // silently reintroduces the no-push this paragraph exists to prevent.
        // A BLOCK-bodied null arm (`0 => { null }`) needs the same repair as the bare one:
        // it parses to `Value::Block` whose last operator is `Value::Null`, so a predicate
        // that only matches a bare `Value::Null` walks straight past it and the arm keeps a
        // value that pushes nothing.  The enum/struct match path already tests both forms;
        // this one tested only the bare form, so `match n { 0 => { null }, _ => { [n] } }`
        // answered null for EVERY n while the bare-arm spelling of the same function was
        // right — a wrong value with no diagnostic, on both backends.
        if arms.iter().any(|a| arm_body_is_null(&a.1)) {
            let typed_null = self.null_value(result_type);
            for arm in arms.iter_mut().filter(|a| arm_body_is_null(&a.1)) {
                set_arm_null_typed(&mut arm.1, &typed_null, result_type);
            }
        }
        let fallback = if has_wildcard {
            let (_, arm_code, _, _) = arms.pop().unwrap();
            arm_code
        } else {
            self.null_value(result_type)
        };

        let mut chain = fallback;
        for (pattern_val, arm_code, _, guard_opt) in arms.into_iter().rev() {
            if let Some(lit) = pattern_val {
                // range/null/or patterns stored as Block with Boolean result.
                if let Value::Block(ref bl) = lit
                    && bl.result == Type::Boolean
                    && (bl.name == "range_pattern"
                        || bl.name == "null_pattern"
                        || bl.name == "or_pattern")
                {
                    let range_cond = bl.operators[0].clone();
                    chain = match guard_opt {
                        Some(guard) => {
                            let guarded = v_if(guard, arm_code, chain.clone());
                            v_if(range_cond, guarded, chain)
                        }
                        None => v_if(range_cond, arm_code, chain),
                    };
                    continue;
                }
                let mut cond = Value::Null;
                let cond_tp = self.call_op(
                    &mut cond,
                    "==",
                    &[Value::Var(v), lit],
                    &[subject_type.clone(), subject_type.clone()],
                );
                if cond_tp == Type::Null {
                    chain = arm_code;
                } else {
                    chain = match guard_opt {
                        Some(guard) => {
                            let guarded = v_if(guard, arm_code, chain.clone());
                            v_if(cond, guarded, chain)
                        }
                        None => v_if(cond, arm_code, chain),
                    };
                }
            } else {
                // Wildcard or guarded wildcard (no pattern).
                chain = match guard_opt {
                    Some(guard) => v_if(guard, arm_code, chain),
                    None => arm_code,
                };
            }
        }
        chain
    }

    // <for> ::= <identifier> 'in' <expression> [ 'par' '(' <id> '=' <worker> ',' <threads> ')' ] '{' <block>
    //
    // The optional parallel clause `par(b=worker(a), N)` desugars to a parallel map
    // followed by an index-based loop over the results.  Three worker call forms
    // are supported — see `parse_parallel_for_loop` for details.
    /// Set up iterator variables for a for-loop header and return
    /// `(iter_var, pre_var, for_var, if_step, create_iter, iter_next)`.
    /// `expr is VariantName` — generates a boolean discriminant check.
    /// For plain enums: `OpConvIntFromEnum(expr) == disc`.
    /// For struct-enums: `OpConvIntFromEnum(OpGetEnum(expr, 0)) == disc`.
    // @F30 — is variant check (+ field capture)
    pub(crate) fn parse_is_variant(
        &mut self,
        code: &mut Value,
        subject_type: &Type,
        variant_name: &str,
    ) -> Type {
        let (e_nr, is_struct) = match subject_type {
            Type::Enum(nr, true, _) => (*nr, true),
            Type::Enum(nr, false, _) => (*nr, false),
            // EnumValue variant type (e.g. `s = Circle { ... }` has type
            // Reference(Circle_def_nr) where Circle's parent is Shape).
            Type::Reference(d_nr, _)
                if self.data.def_type(*d_nr) == DefType::EnumValue
                    && matches!(
                        self.data.def(self.data.def(*d_nr).parent).returned(),
                        Type::Enum(_, true, _)
                    ) =>
            {
                (self.data.def(*d_nr).parent(), true)
            }
            // Reference to an Enum itself (e.g. loop variable from
            // vector<Shape> iteration gets Type::Reference(Shape_nr, _)).
            Type::Reference(d_nr, _)
                if self.data.def_type(*d_nr) == DefType::Enum
                    && matches!(self.data.def(*d_nr).returned(), Type::Enum(_, true, _)) =>
            {
                (*d_nr, true)
            }
            _ => {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "'is' requires an enum type, got {}",
                        subject_type.name(&self.data)
                    );
                }
                return Type::Boolean;
            }
        };
        // @PLN22 Phase 1 — resolve the variant against the subject enum via the
        // variant_of chokepoint (the (enum, variant) scope key), not the bare
        // global def_nr.  `is` is always enum-typed here (see the match above).
        let variant_def_nr = self.data.variant_of(e_nr, variant_name);
        if variant_def_nr == u32::MAX || self.data.def_type(variant_def_nr) != DefType::EnumValue {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "'{}' is not a variant of {}",
                    variant_name,
                    self.data.def(e_nr).name()
                );
            }
            return Type::Boolean;
        }
        let disc: i32 = if is_struct {
            let variant_attrs = self.data.def(variant_def_nr).attributes();
            if let Some(first) = variant_attrs.first()
                && let Value::Enum(nr, _) = first.value
            {
                i32::from(nr)
            } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(variant_name) {
                if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                    i32::from(nr)
                } else {
                    0
                }
            } else {
                0
            }
        } else if let Some(a_nr) = self.data.def(e_nr).attr_names.get(variant_name) {
            if let Value::Enum(nr, _) = self.data.def(e_nr).attributes()[*a_nr].value {
                i32::from(nr)
            } else {
                0
            }
        } else {
            0
        };
        let subject_clone = code.clone();
        let disc_expr = if is_struct {
            let get_enum = self.cl("OpGetEnum", &[code.clone(), Value::Int(0)]);
            self.cl("OpConvIntFromEnum", &[get_enum])
        } else {
            self.cl("OpConvIntFromEnum", std::slice::from_ref(code))
        };
        let disc_check = self.cl("OpEqInt", &[disc_expr, Value::Int(disc)]);
        let is_field_capture = is_struct && self.lexer.peek_token("{") && {
            let link = self.lexer.link();
            self.lexer.token("{");
            let is_capture = self.lexer.has_identifier().is_some()
                && (self.lexer.peek_token(",") || self.lexer.peek_token("}"));
            self.lexer.revert(link);
            is_capture
        };
        if is_field_capture {
            let mut condition: Vec<Value> = Vec::new();
            let stable_subject = if matches!(subject_clone, Value::Var(_)) {
                subject_clone
            } else {
                let tmp = self.create_unique("is_subj", subject_type);
                if tmp != u16::MAX {
                    self.vars.defined(tmp);
                    condition.push(v_set(tmp, subject_clone));
                }
                Value::Var(tmp)
            };
            self.lexer.token("{");
            let mut seen_fields: HashSet<String> = HashSet::new();
            while let Some(field_name) = self.lexer.has_identifier() {
                if !self.first_pass && seen_fields.contains(&field_name) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "duplicate field binding '{}' in is-capture",
                        field_name
                    );
                }
                seen_fields.insert(field_name.clone());
                let attr_idx_and_type = {
                    let variant_def = self.data.def(variant_def_nr);
                    variant_def.attributes[1..]
                        .iter()
                        .enumerate()
                        .find(|(_, a)| a.name == field_name)
                        .map(|(i, a)| (i + 1, a.typedef.clone()))
                };
                match attr_idx_and_type {
                    Some((attr_idx, field_type)) => {
                        let field_read =
                            self.get_field(variant_def_nr, attr_idx, stable_subject.clone());
                        let v_nr = self.create_unique(&format!("mv_{field_name}"), &field_type);
                        if v_nr != u16::MAX {
                            self.vars.defined(v_nr);
                            // loft#1160 — the field this binding projects, so a write spelled
                            // through it takes the field path (see `mv_field_origin`).
                            self.vars.mv_field_origin.insert(
                                v_nr,
                                (
                                    field_read.clone(),
                                    Type::Reference(variant_def_nr, Deps::none()),
                                ),
                            );
                            // The capture binds a borrowed view into the
                            // subject's record — scope cleanup must not
                            // emit OpFreeRef for it (see the same
                            // note at parse_match_enum_field_bindings in
                            // this file for the match-arm path).
                            self.vars.set_skip_free(v_nr);
                            // ...and the borrow must be in the TYPE, which is the other half
                            // of that note and was applied only to the `match` path.  #429
                            // gave a HEAP payload binding a frame dep on its subject because
                            // an empty dep list is the @FR-O-Proxy proxy for OWNED, and the
                            // two backends then read the same bind differently: `--native`
                            // deep-COPIES it and the interpreter aliases.  The `is` spelling
                            // is the same bind and had no dep, so `if w.st is Holder { inner }
                            // { w.st = Empty{…}; inner.a }` answered 1 natively and 0 on the
                            // interpreter, with a leaked `Pay` record beside it — the same
                            // divergence #429 closed for `match`, at the sibling site
                            // (loft#1398).  Scalars carry no DbRef and need no dep, exactly as
                            // there.
                            if matches!(
                                &field_type,
                                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                            ) && let Some(src) = self.match_borrow_source(&stable_subject)
                            {
                                let bound_tp = match self.vars.tp(v_nr).clone() {
                                    Type::Reference(td, _) => {
                                        Type::Reference(td, Deps::frame1(src))
                                    }
                                    Type::Vector(it, _) => Type::Vector(it, Deps::frame1(src)),
                                    Type::Enum(td, su, _) => Type::Enum(td, su, Deps::frame1(src)),
                                    other => other,
                                };
                                self.vars.set_type(v_nr, bound_tp);
                            }
                            self.is_capture_bindings.push(v_set(v_nr, field_read));
                            let old = self.vars.set_name(&field_name, v_nr);
                            self.is_capture_aliases.push((field_name.clone(), old));
                        }
                    }
                    None => {
                        if !self.first_pass {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "variant {} has no field '{}'",
                                self.data.def(variant_def_nr).name(),
                                field_name
                            );
                        }
                    }
                }
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token("}");
            // loft#1007 — a capture list is only ever followed by the BODY it binds for, and
            // this is the one place that knows the braces just consumed were a capture rather
            // than a block.  Without saying so here the caller reports `Expect token {` at the
            // `else`, naming neither `is`, the capture, nor the variant — and the spelling that
            // provokes it is the one a reader writes first, `v = if c is Circle { radius } else
            // { 0 }`, because `{ radius }` reads as the then-branch.
            // Reported on BOTH passes on purpose: this is a SYNTAX fault, and pass 1 is where
            // it is met — a `!self.first_pass` gate made the message unreachable, because the
            // generic `Expect token {` the caller raises on pass 1 is the error the run stops
            // on and pass 2 never sees the file (slice 7's fallback-parser lesson, from the
            // other side).
            if !self.lexer.peek_token("{") {
                let names: Vec<&str> = seen_fields.iter().map(String::as_str).collect();
                let captured = names.join(", ");
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`{{ {captured} }}` after `is {}` is the field-capture list, not the body — \
                     a block has to follow it. Write `is {} {{ {captured} }} {{ … }}`, repeating \
                     the name if the body is just that value, or use `match`, which takes \
                     captures and IS an expression",
                    self.data.def(variant_def_nr).name(),
                    self.data.def(variant_def_nr).name(),
                );
            }
            if condition.is_empty() {
                *code = disc_check;
            } else {
                condition.push(disc_check);
                *code = Value::Insert(condition);
            }
        } else {
            *code = disc_check;
        }
        Type::Boolean
    }

    pub(crate) fn for_type(&mut self, in_type: &Type) -> Type {
        // unwrap &vector<T> so the element type resolves correctly.
        if let Type::RefVar(inner) = in_type {
            return self.for_type(inner);
        }
        // …and peel `τ?` for the same reason (@PLN25's dn1 audit named this site and
        // prescribed exactly this: *"`for x in nullable` misses Text/Integer arms"*).
        //
        // The ELEMENT type of a nullable collection is its element type; the `?` is a fact
        // about the collection, not about what it holds.  Left unpeeled, every arm below
        // missed and the fall-through reported *"Unknown in expression type vector<T>?"* —
        // a second error for one mistake, and the unhelpful one of the two, since
        // `Parser::iterator` already refuses the loop by naming the `?` and the discharge
        // that clears it.  Peeling does not make the loop legal: there is no `τ? ⤳ τ`
        // ([types.md](../../doc/claude/formal/types.md) N-Coal / N-Default), so the refusal
        // still stands — it just stands alone.
        if let Type::Optional(inner) = in_type {
            return self.for_type(inner);
        }
        if let Type::Vector(t_nr, dep) = &in_type {
            let mut t = *t_nr.clone();
            if let Type::Enum(nr, true, _) = t
                && !self.data.def(nr).name.starts_with("__nullable<")
            {
                // @PLN25 E2 — keep a synthetic `__nullable<S>` element in `Enum`
                // form for the loop variable: field access on `Type::Enum(.., true)`
                // unwraps to the `Some` variant via `find_poly_enum_field`
                // (fields.rs), whereas `Reference(enum_def)` does not (the enum
                // itself has no payload field) → "Unknown field __nullable<S>.f".
                // Hand-written struct-enums keep the Reference conversion (variant
                // field-access resolves against the variant def, not the parent).
                t = Type::Reference(nr, Deps::none());
            }
            // P189b: vector elements that are tuples live as inline bytes
            // in the vector record.  Iteration yields a 12-byte DbRef
            // pointing at those bytes; treat the loop var as a reference
            // to the synthetic `__tuple<...>` struct so per-element loads
            // happen through `OpVarRef` + `OpGet*(offset)` rather than the
            // stack-tuple `OpTupleGet` which would read DbRef bytes as
            // garbage integers.  parse_part recognises the def-name prefix
            // `__tuple<` and routes `.0` / `.1` to TupleGet IR.
            if let Type::Tuple(ref elems) = t {
                let elems_clone = elems.clone();
                let tuple_d = self.data.tuple_def(&mut self.lexer, &elems_clone);
                t = Type::Reference(tuple_d, Deps::none());
            }
            for d in dep {
                t = t.depending(*d);
            }
            t
        } else if let Type::Sorted(dnr, _, dep)
        | Type::Index(dnr, _, dep)
        | Type::Hash(dnr, _, dep)
        | Type::Radix(dnr, _, dep)
        | Type::Trie(dnr, _, dep) = &in_type
        {
            // C60 path 2c piece 2: hash iteration yields `reference<T>`,
            // same shape as Sorted/Index.  This is the parser-side
            // prerequisite before fill_iter (src/parser/fields.rs:599)
            // can flip the hash arm to `on = 4`.  Without this, for-loop
            // body parsing sees `e` as Type::Null and field access on
            // `e.name` fails with "Unknown type null".
            //
            // @PLN25 E2 — a synth `__nullable<S>` element keeps `Enum(.., true)`
            // so the loop body's field access unwraps through `Some` (mirrors
            // the Vector arm above and the `index_type` lookup path); without
            // it `e.field` errors "Unknown field __nullable<S>.field" because
            // the enum itself carries no payload field.  Inert gate-off (no
            // keyed element type is ever a `__nullable<` enum).
            if self.data.def(*dnr).name.starts_with("__nullable<") {
                Type::Enum(*dnr, true, dep.clone())
            } else {
                Type::Reference(*dnr, dep.clone())
            }
        } else if let Type::Iterator(i_tp, _) = &in_type {
            if **i_tp == Type::Null {
                I32.clone()
            } else {
                *i_tp.clone()
            }
        } else if let Type::Text(_) = in_type {
            Type::Character
        } else if let Type::Reference(_, _) | Type::Integer(_) = in_type {
            // I13: check for custom iterator protocol before falling back.
            let next_d_nr = self.data.find_fn(u16::MAX, "next", in_type);
            if next_d_nr != u32::MAX {
                let item = self.data.def(next_d_nr).returned().clone();
                // @PLN102 D1 — `next(self) -> Item?` uses null as the iteration TERMINATOR: the
                // loop stops the moment `next` yields null, so the body only ever binds a present
                // value. Type the loop variable as the non-null `Item`, not `Item?` — otherwise
                // N-Prop would spuriously nullify `sum += x` on a value that is never null here.
                // The null-check that ends the loop is structural (`parse_for_iter_setup`), so
                // peeling the marker does not disturb termination.
                return match item {
                    Type::Optional(inner) => *inner,
                    other => other,
                };
            }
            in_type.clone()
        } else if !self.first_pass {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Unknown in expression type {}",
                in_type.name(&self.data)
            );
            Type::Null
        } else {
            // First pass, iterable type not yet resolved — a forward or
            // cross-package reference whose definition registers later in the
            // recursion (#375: a dependency imported at a high source number is
            // parsed AFTER the importing package on pass 1).  Return Unknown,
            // not Null, so downstream field/method access on the loop variable
            // routes through the existing `Type::Unknown` defer-guard in
            // `field()` and DEFERS to pass 2, instead of hard-erroring
            // "Unknown type null" — which would abort pass 1 before the
            // dependency's definitions are registered, leaving pass 2 (which
            // would resolve cleanly) unreachable.
            Type::Unknown(0)
        }
    }

    /// @PLN85 D-own-1 — the per-var verdict of `text_return`'s promotion loop,
    /// as a PURE selector (the classify/apply split `classify_vector_delivery` /
    /// `classify_reference_delivery` use for tail shapes, applied to the per-var
    /// rule ladder).  One variant per documented rule; `text_return` applies.
    fn classify_text_dep(&self, v: u16) -> TextDep {
        let n = self.vars.name(v);
        // A name that is already an attribute records its index in the return
        // dep (dedup at the apply site).
        if let Some(a) = self.data.def(self.context).attr_names.get(n) {
            return TextDep::Attr(*a as u16);
        }
        // A captured text variable is read from the closure record at runtime —
        // it must NOT become a hidden RefVar(Text) work-buffer argument (that
        // would shift `__closure` to a wrong stack position).
        if self.captured_names.iter().any(|(name, _)| name == n) {
            return TextDep::SkipCaptured;
        }
        let tp = self.vars.tp(v);
        // A text local promotes to a HIDDEN RefVar(Text) out-param buffer.
        // @PLN25 DN1 (#487 / 449): `.base()` peel — an `Optional(Text)` local
        // shares `Text`'s sentinel storage (a null `text?` IS `STRING_NULL` in
        // the same slot), so it hoists to the SAME hidden buffer.  Without the
        // peel it fell to the plain-VALUE promotion below and call sites
        // pushed a `null` placeholder into a frame laid out for a work buffer
        // (`realloc(): invalid next size` in the multiplayer/ws consumers).
        if matches!(tp.base(), Type::Text(_)) {
            // loft#1113 — one text work buffer per lambda, and the first asker
            // takes it (`holds_text_work_buf`).  Two promotions can meet in one
            // body: `parse_return` promotes at the `return`, and the block tail
            // promotes the `??` / `?` / `if` accumulator afterwards.
            if self.holds_text_work_buf() {
                TextDep::SkipSecondTextBuf
            } else {
                TextDep::PromoteHidden
            }
        } else if matches!(tp, Type::Tuple(_)) {
            // @P330: a tuple local must NOT hoist to a parameter — call sites
            // would push a 12-byte null DbRef where the slot is 16+ bytes per
            // text element, corrupting the callee frame.  The dep drops; the
            // B5-L3 `__ret_N` deep-copy temp (scopes.rs) covers the value.
            TextDep::SkipTupleLocal
        } else if !self.vars.is_argument(v) {
            // A LOCAL must not be promoted.  Promotion makes it an ARGUMENT, and an
            // argument is the caller's to free — but the caller was never handed this
            // store: `add_defaults` fills a promoted slot with `null` unless the callee
            // writes into it (NRVO), which a freshly-allocated record does not.  So the
            // record the function itself allocated is orphaned, once per call, for as
            // long as the program runs (loft#771: 5000 calls retained 5000 records).
            // Dropping the dep leaves it a local, so `get_free_vars` frees it — and the
            // returned text does not need it, because a text return is delivered by
            // COPY into the caller's `&text` buffer.  Routing the same field read
            // through a local first (`x = v.nm; return x;`) always did exactly this and
            // freed, which is what says the promotion was never load-bearing.
            //
            // The verdict must read only PASS-STABLE facts.  Keying it on ownership
            // (`deps.is_empty()`) looked sharper and was not: deps accumulate while a
            // body parses, so a var could read as owned on pass 1 and borrowed on
            // pass 2 — the attribute then appeared on pass 2 only, which is the
            // cross-pass divergence the H5 two-pass contract exists to catch.
            // `is_argument` is stable, because a var promoted on an earlier pass is
            // an attribute and takes the `Attr` branch above.
            TextDep::SkipOwnedLocal
        } else {
            // Any other dep type promotes as a plain (visible) parameter.
            TextDep::PromotePlain
        }
    }

    /// Does the LAMBDA being parsed already carry a hidden `RefVar(Text)` work
    /// buffer?
    ///
    /// A lambda takes at most ONE, because that is what the fn-ref call ABI
    /// passes.  A call site holding a fn-typed slot cannot know which lambda is
    /// in it, so it injects exactly one buffer and the callee either uses it or
    /// has it popped (`State::fn_call_ref`).  A callee carrying TWO leaves the
    /// frame one DbRef span short, and the callee then reads its `__closure`
    /// slot from the wrong offset — loft#717's fault line, reached by a second
    /// route (loft#1113).
    ///
    /// So the FIRST promotion to ask for a buffer takes it, and every later text
    /// local stays a local — its value is delivered by copy into the caller's
    /// `&text` buffer, which is what `SkipOwnedLocal` already describes for a
    /// local it declines to hoist.
    ///
    /// Scoped to lambdas, and the narrowness is measured rather than cautious: a
    /// NAMED function's call sites lower against a known signature and carry as
    /// many buffers as it declares, and applying the rule to them instead moved
    /// five suite results (`float=0.25` came back `0` through the sqlite
    /// bridges).  A named function that shares a fn-ref's signature does reach
    /// the generated dispatch arm, which forwards one buffer twice and fails to
    /// compile — that is a separate, older defect and is not cured here.
    ///
    /// The verdict is pass-stable: a var promoted on pass 1 is an attribute by
    /// pass 2 and takes `classify_text_dep`'s `Attr` branch, so it re-acquires the
    /// same buffer rather than asking for a new one.
    /// Mint the hidden `&text` work buffers a fn-ref call of this signature must carry,
    /// and answer their variables.
    ///
    /// A `&text` is a pointer into the CALLER's frame, so only the caller can supply a
    /// buffer that outlives the call — and a call through a fn-typed slot cannot know which
    /// function the slot holds.  So it mints what the widest candidate of that signature
    /// could want and `State::fn_call_ref` trims the frame to what the actual target
    /// declares (loft#1116).
    ///
    /// **Every site that builds a text-returning `CallRef` must mint through here.**  The
    /// trim is computed from the same count, so a site that mints fewer has a REAL buffer
    /// trimmed away, and the callee then reads its `__closure` from the wrong offset — a
    /// corrupt `DbRef` rather than a diagnostic.  Minting happens on BOTH passes so the
    /// work-variable numbering does not shift across the pass boundary;
    /// [`Self::push_fnref_text_buffers`] builds the argument blocks on pass 2.
    pub(crate) fn fnref_text_buffer_vars(&mut self, params: usize, ret: &Type) -> Vec<u16> {
        (0..self.data.fnref_text_buffers(params, ret))
            .map(|_| self.vars.work_text(&mut self.lexer))
            .collect()
    }

    /// Append one `OpCreateStack` argument block per buffer from
    /// [`Self::fnref_text_buffer_vars`], cleared so a loop iteration starts fresh.
    ///
    /// Order matters and is the callee's: visible parameters, then work buffers, then the
    /// closure `fn_call_ref` reads out of the fn-ref slot.
    pub(crate) fn push_fnref_text_buffers(&mut self, args: &mut Vec<Value>, work_vars: &[u16]) {
        let ref_def = self.data.def_nr("reference");
        for &wv in work_vars {
            let create = self.cl("OpCreateStack", &[Value::Var(wv)]);
            args.push(v_block(
                vec![crate::data::v_set(wv, Value::Text(String::new())), create],
                Type::Reference(ref_def, Deps::frame1(wv)),
                "cref_work_buf",
            ));
        }
    }

    /// The variable of the one hidden text buffer a LAMBDA already holds — the `__work_ret`
    /// the fn-ref ABI hands it, or a buffer an earlier promotion in the same body took —
    /// or `None` for a named function or a lambda that has none yet.  The tail promotions
    /// read it as their pass-2 gate: a lambda whose `return` took the buffer on pass 1
    /// never minted an `__acc` / `__tret` attribute, so the attribute-based gate declined
    /// on pass 2 and the tail was never rewritten at all.
    fn lambda_text_buffer_var(&self) -> Option<u16> {
        if !self.holds_text_work_buf() {
            return None;
        }
        let def = self.data.def(self.context);
        def.attributes()
            .iter()
            .find(|a| {
                a.hidden && matches!(&a.typedef, Type::RefVar(t) if matches!(**t, Type::Text(_)))
            })
            .map(|a| self.vars.var(&a.name))
            .filter(|&v| v != u16::MAX)
    }

    fn holds_text_work_buf(&self) -> bool {
        self.data
            .def(self.context)
            .name()
            .starts_with("n___lambda_")
            && self.data.def(self.context).text_work_buffers() > 0
    }

    pub(crate) fn text_return(&mut self, ls: &[u16]) {
        // @PLN25 slice (c): peel `Optional` — a `-> text?` function needs the SAME
        // work-buffer conversion (`RefVar(Text)` hidden out-param + `__ret_N` capture) as
        // `-> text`; without it a `text?` tail whose arm is a concat/work-var fell to the
        // `Return(Null)` synthesis (the value was freed and null returned). The `?` is
        // re-applied when the returned type is rewritten below.
        let (mut dep, ret_is_optional) =
            match &self.data.definitions[self.context as usize].returned {
                Type::Text(cur) => (cur.clone(), false),
                Type::Optional(inner) if matches!(**inner, Type::Text(_)) => {
                    let Type::Text(cur) = &**inner else {
                        unreachable!()
                    };
                    (cur.clone(), true)
                }
                _ => return,
            };
        {
            // @PLN85 D-own-1 — classify ONCE per var (the pure selector), then
            // apply the one mechanism per verdict.  The rule rationale lives on
            // the `TextDep` variants; the arms carry only emission mechanics.
            for v in ls {
                match self.classify_text_dep(*v) {
                    TextDep::Attr(a) => {
                        if !dep.contains(&a) {
                            dep.push(a);
                        }
                        // @PLN85 — a var already registered as a HIDDEN
                        // RefVar(Text) work-buffer attribute (promoted on an
                        // earlier pass — the pass-1 `__tret` bind) must
                        // re-acquire the RefVar var-type + argument marking so
                        // pass 2 lowers its body as the promoted buffer, not a
                        // plain text.  Without this the both-pass `__tret` bind
                        // double-classifies (the empty-return bug 2d dodged by
                        // staying pass-2-only); WITH it the bind is
                        // signature-consistent, fixing forward-reference callers.
                        let promoted_buf = {
                            let at = &self.data.def(self.context).attributes()[a as usize];
                            at.hidden && matches!(at.typedef, Type::RefVar(_))
                        };
                        if promoted_buf {
                            self.vars.become_argument(*v);
                            self.vars
                                .set_type(*v, Type::RefVar(Box::new(Type::Text(Deps::none()))));
                        }
                    }
                    TextDep::SkipCaptured
                    | TextDep::SkipTupleLocal
                    | TextDep::SkipOwnedLocal
                    | TextDep::SkipSecondTextBuf => {
                        // SkipTupleLocal (@P330): the dep drops on purpose — the
                        // return type loses this local, which lets scopes'
                        // B5-L3 single-text branch deep-copy the tail into a
                        // `__ret_N` temp before the local frees (the @P329
                        // family one layer up; hoisting the tuple local was
                        // the wrong escape hatch — call sites would push a
                        // 12-byte null DbRef where the slot is 16+ bytes).
                    }
                    TextDep::PromoteHidden => {
                        let n = self.vars.name(*v);
                        let a = self.data.add_attribute(
                            &mut self.lexer,
                            self.context,
                            n,
                            Type::RefVar(Box::new(Type::Text(Deps::none()))),
                        );
                        // @P387 zero-cost: mark the work-buffer HIDDEN so it rides the
                        // same adaptive hidden-return-buffer dispatch struct/vector use
                        // (`fn_call_ref` pushes one per hidden buf — 0 for a fn with no
                        // promotable local).  This replaces the static `cref_work_buf`
                        // injection and keeps the buffer out of the fn-ref TYPE without
                        // the deps-based exclusion that wrongly dropped returned params.
                        self.data.definitions[self.context as usize].attributes[a].hidden = true;
                        self.vars.become_argument(*v);
                        dep.push(a as u16);
                        self.vars
                            .set_type(*v, Type::RefVar(Box::new(Type::Text(Deps::none()))));
                    }
                    TextDep::PromotePlain => {
                        let n = self.vars.name(*v);
                        let tp = self.vars.tp(*v).clone();
                        let a = self
                            .data
                            .add_attribute(&mut self.lexer, self.context, n, tp);
                        self.vars.become_argument(*v);
                        dep.push(a as u16);
                    }
                }
            }
            // P227: ensure every text-returning LAMBDA has at least one
            // `RefVar(Text)` hidden work-buffer attribute so the fn-ref
            // dispatch ABI is uniform — callers always allocate exactly
            // one buffer per text-returning fn-ref call, regardless of
            // whether the assigned lambda's body uses formatting.
            // Limited to lambdas (`n___lambda_*` prefix); the fix matches
            // the trio used by the existing text_return arm above:
            // (1) add_attribute, (2) create_var, (3) become_argument.
            // Gated on first_pass to avoid duplicate-add on the second
            // pass; the second-pass `__closure` injection (if any)
            // happens later in parse_lambda so the trailing position is
            // preserved.
            // Only LAMBDAS carry a `RefVar(Text)` work-buffer: their fn-ref
            // dispatch (control.rs) is the ONE text path that hands the callee a
            // caller-owned buffer.  Named/literal text fns return owned text (no
            // buffer) — giving them one (the reverted @P387 option A) broke par
            // workers (#273) and the markdown viewer, because not every call site
            // injects the buffer.  Zero-cost @P387: the fn-ref dispatch no longer
            // injects a text buffer (see `text_fn_ref_owned` below), so even a
            // named text fn works as a fn-value without one.
            let is_lambda = self
                .data
                .def(self.context)
                .name()
                .starts_with("n___lambda_");
            if self.first_pass && is_lambda && !self.holds_text_work_buf() {
                let work_tp = Type::RefVar(Box::new(Type::Text(Deps::none())));
                let a = self.data.add_attribute(
                    &mut self.lexer,
                    self.context,
                    "__work_ret",
                    work_tp.clone(),
                );
                // @P387 zero-cost: hidden like the text_return buffer above, so the
                // runtime fn-ref dispatch pushes it adaptively (no static injection).
                self.data.definitions[self.context as usize].attributes[a].hidden = true;
                let v = self.create_var("__work_ret", &work_tp);
                if v != u16::MAX {
                    self.vars.become_argument(v);
                }
                dep.push(a as u16);
            }
            let new_ret = if ret_is_optional {
                Type::optional(Type::Text(dep))
            } else {
                Type::Text(dep)
            };
            self.data.definitions[self.context as usize].returned = new_ret;
        }
    }

    /// @PLN85 category A — re-run the text-return promotion on a freshly-minted
    /// generic MONOMORPH so it delivers through a hidden `&text` caller buffer,
    /// identical to its non-generic twin.  Monomorphs are built by IR
    /// substitution (`try_generic_instantiation`), NOT by `parse_block`, so the
    /// parse-time `do_tret_bind` + `text_return` promotion never engages and the
    /// monomorph returns an owned `String` by value (the interpreter orphans it →
    /// leak; native RAII drops it).  Both promoters couple to exactly
    /// `self.context` + `self.vars`, so we swap those two onto the monomorph,
    /// replicate the `do_tret_bind` rebind (`Set(__tret, tail); __tret`), and call
    /// the identical `text_return` — then restore.  Called from
    /// `try_generic_instantiation` BEFORE it returns `d_nr`, so the promoted
    /// signature is in place when the call site lowers its call.
    ///
    /// `parse_block` decides this in TWO mutually exclusive branches and both are
    /// replicated here: `do_tret_bind` for a CALL tail, and `do_if_acc`
    /// ([`Self::monomorph_if_acc_ok`]) for a value-yielding `if`/`match` tail, whose
    /// arms are pushed into an accumulator rather than bound as one value.  Copying
    /// only the first is what made "identical to its non-generic twin" false for
    /// every `-> T` generic: an `a?` discharge is an `If`, so it landed on the
    /// missing branch and orphaned one `String` per call (loft#1026).
    ///
    /// Closes the BOUND/discarded cases (only the monomorph needs promoting); a
    /// RETURNED monomorph result (`run() -> text { first(nums) }`) also needs the
    /// caller to promote, which the def_nr backward-ref gate blocks (monomorph
    /// minted after the caller) — that half awaits the forward-ref pre-pass.
    /// @PLN104 — after pass 2, flag every user text-returning fn that returns `Owned`
    /// text (`use_analysis::return_ownership`) WITHOUT a hidden `&text` retbuf — the
    /// owned-by-value shape the interpreter orphans (loft-lang/loft#568). Flagged defs
    /// go into `force_tret`, which drives `targeted_tret_promotion` to re-lower them with
    /// a `&text` retbuf (`do_tret_bind` + the `a==v` renumber in `block_result`) IN PLACE
    /// and patch their callers. DEFAULT-ON — opt OUT with `LOFT_NO_TRET_FIX` (a debug escape
    /// hatch that restores the leak). `LOFT_TRET_REPORT` additionally prints each flagged
    /// def. When testing the flag directly, set `LOFT_NO_CACHE=1` (the whole-program cache
    /// is content-keyed and ignores env flags). See
    /// `doc/claude/plans/104-tret-promotion/targeted-promotion-design.md`.
    pub(crate) fn report_tret_promotions(&mut self) {
        let report = std::env::var_os("LOFT_TRET_REPORT").is_some();
        // The #568 leak fix is on by default; LOFT_NO_TRET_FIX is a debug escape hatch.
        let fix = std::env::var_os("LOFT_NO_TRET_FIX").is_none();
        if !report && !fix {
            return;
        }
        let mut flagged: Vec<u32> = Vec::new();
        for d in 0..self.data.definitions() {
            let def = self.data.def(d);
            if def.def_type != DefType::Function || def.source != crate::data::MAIN_SOURCE {
                continue;
            }
            // The #568 orphan predicate lives in ONE place (`use_analysis`) so this oracle
            // and the `--show-ownership` overlay flag exactly the same class.
            let Some(kind) = crate::use_analysis::text_return_orphan_risk(&self.data, d) else {
                continue;
            };
            if report {
                eprintln!(
                    "[tret-promote] def #{d} `{}` ({kind}), no retbuf — needs promotion (#568)",
                    def.original_name()
                );
            }
            flagged.push(d);
        }
        if fix {
            for d in flagged {
                self.force_tret.insert(d);
            }
        }
    }

    /// `parse_block`'s `do_if_acc` gate, asked about a MONOMORPH.
    ///
    /// Both halves matter and both come from the parse-time site.  The tail must be a
    /// value-yielding `if`/`match` over text (`if_tail_yields_text` — a guard arm that
    /// `return`s or yields `null` is excluded, so the missing-return diagnostic still
    /// fires).  And the nullability test reads the DECLARED RETURN, not the tail
    /// (loft#741): performing the per-arm store for a nullable tail into a non-null
    /// return is the nullable-into-non-null store `(N-Store)` exists to report, so it
    /// must stay unpromoted and be reported rather than silently written.
    fn monomorph_if_acc_ok(&self, d_nr: u32, l: &[Value], block_result: &Type) -> bool {
        let returned = self.data.def(d_nr).returned();
        matches!(returned.base(), Type::Text(_))
            && (matches!(returned, Type::Optional(_)) || !matches!(block_result, Type::Optional(_)))
            && l.last().is_some_and(Self::if_tail_yields_text)
    }

    /// The tuple twin of [`Self::promote_monomorph_text_return`] (@FR-F-Ret).
    ///
    /// A declaration whose tuple return SHAPE depends on `T` defers its boxing to
    /// instantiation; `tuple_return_rewrite` boxes the instance's signature to the synthetic
    /// `__tuple<…>` record, and this rewrites every tuple TAIL and every `return (…)` of the
    /// body into that record — the same `synthetic_tuple_return` block a concrete declaration
    /// gets from `block_result`, with the member copies `set_field_no_check` performs.  Signature
    /// without body was the mismatch @PLN85's generic-tuple-return-fix.md measured (a `__tuple`
    /// signature over a bare-tuple body: garbage on the interpreter, E0308 on native), which
    /// is why this runs where the signature is rewritten and not elsewhere.
    ///
    /// Runs in the MONOMORPH's frame (the same swap the text twin makes): the work-ref belongs
    /// to the function the code lands in.  A body that yields no stack tuple — a concrete-shaped
    /// template boxed at its declaration, whose body `block_result` already rewrote — is left
    /// untouched, so the work-ref is minted only where a tail is rewritten.
    pub(crate) fn promote_monomorph_tuple_return(&mut self, d_nr: u32) {
        let Type::Reference(synth, _) = self.data.def(d_nr).returned().base().clone() else {
            return;
        };
        if !self.data.def(synth).name().starts_with("__tuple<") {
            return;
        }
        let saved_ctx = self.context;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = d_nr;
        let mut code =
            std::mem::replace(&mut self.data.definitions[d_nr as usize].code, Value::Null);
        if let Value::Block(bl) = &mut code
            && bl
                .operators
                .iter()
                .any(|op| Self::yields_stack_tuple(op, &self.vars, true))
        {
            let synth_ref = Type::Reference(synth, Deps::none());
            let w = self.vars.work_refs(&synth_ref, &mut self.lexer);
            let kt = self.data.def(synth).known_type();
            for op in &mut bl.operators {
                self.rewrite_tuple_returns_with_work_ref(synth, kt, w, op);
            }
            if let Some(last) = bl.operators.last_mut() {
                self.rewrite_tail_tuple_with_work_ref(synth, kt, w, last);
            }
            bl.result = synth_ref;
        }
        self.data.definitions[d_nr as usize].code = code;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = saved_ctx;
    }
    /// Does `v` yield a STACK tuple — a tuple literal, a local of tuple type, or a
    /// branch/block whose tail does — where `tail` says the value is in tail position (so a
    /// bare leaf counts) rather than a statement (where only a `return` counts)?
    fn yields_stack_tuple(v: &Value, vars: &crate::variables::Function, tail: bool) -> bool {
        match v.unspan() {
            Value::Return(inner) => Self::yields_stack_tuple(inner, vars, true),
            Value::Tuple(_) => tail,
            Value::Var(x) => {
                tail && (*x as usize) < vars.count() as usize
                    && matches!(vars.tp(*x).base(), Type::Tuple(_))
            }
            Value::If(_, t, e) => {
                Self::yields_stack_tuple(t, vars, tail) || Self::yields_stack_tuple(e, vars, tail)
            }
            Value::Block(b) | Value::Loop(b) => {
                let n = b.operators.len();
                b.operators
                    .iter()
                    .enumerate()
                    .any(|(i, op)| Self::yields_stack_tuple(op, vars, tail && i + 1 == n))
            }
            Value::Insert(ops) => {
                let n = ops.len();
                ops.iter()
                    .enumerate()
                    .any(|(i, op)| Self::yields_stack_tuple(op, vars, tail && i + 1 == n))
            }
            _ => false,
        }
    }
    /// Rewrite every `return <tuple>` reachable from `node` into the synthetic record `w`
    /// — the early-return half of [`Self::promote_monomorph_tuple_return`]; the tail is the
    /// caller's own `rewrite_tail_tuple_with_work_ref`.
    fn rewrite_tuple_returns_with_work_ref(
        &mut self,
        synth: u32,
        kt: u16,
        w: u16,
        node: &mut Value,
    ) {
        match node {
            Value::Span(b) => self.rewrite_tuple_returns_with_work_ref(synth, kt, w, &mut b.1),
            Value::Return(inner) => self.rewrite_tail_tuple_with_work_ref(synth, kt, w, inner),
            Value::Block(b) | Value::Loop(b) => {
                for op in &mut b.operators {
                    self.rewrite_tuple_returns_with_work_ref(synth, kt, w, op);
                }
            }
            Value::Insert(ops) => {
                for op in ops.iter_mut() {
                    self.rewrite_tuple_returns_with_work_ref(synth, kt, w, op);
                }
            }
            Value::If(_, t, e) => {
                self.rewrite_tuple_returns_with_work_ref(synth, kt, w, t);
                self.rewrite_tuple_returns_with_work_ref(synth, kt, w, e);
            }
            _ => {}
        }
    }
    /// The vector twin of [`Self::promote_monomorph_text_return`] (@FR-F-Ret / @FR-B-Copy).
    ///
    /// A template binds `T` as a record, so a whole-value bind `s = x` and a returned `x` are
    /// lowered for a RECORD: the bind copies at codegen and the return hands the argument up
    /// raw, which the caller copies off the declared `["x"]` dep.  Substituting a VECTOR for
    /// `T` changes neither lowering, and a vector's are different: a vector bind copies at the
    /// parse (`OpClearVector` + `OpAppendVector` into the local's own store) and a vector
    /// return copies in the CALLEE (`borrow_tail_copy`), because a caller never copies a
    /// vector it is handed.  So the instance aliased on both counts — `s = x` bound the
    /// argument's store and the frame then FREED it, `{ x }` handed the argument's store up
    /// and a write through the result wrote the argument, on both backends (QUALITY.md B7t).
    ///
    /// Two rewrites, both on the substituted body: a `Set(v, Var(u))` whose target the TEMPLATE
    /// typed as the type variable and both sides now type as vectors becomes
    /// `OpReplaceVector(v, u)` — the copy into the store the vector local's null-init allocates
    /// (@FR-B-Copy); and where the return's own summary says the body hands a PARAMETER up
    /// (@FR-O-Oracle, a pure borrow), every such leaf is copied into one fresh local the frame
    /// then returns, so the caller adopts a mint as it does for the concrete twin.  Runs in the
    /// MONOMORPH's frame, like the other two.
    /// loft#1387 / `@FR-F-Ret` — give a MONOMORPH's return tail the per-arm copy the
    /// non-generic path takes (loft#1368).
    ///
    /// A return whose tail is a value BRANCH over two or more PARAMETERS hands back each
    /// arm's own borrow, and a caller can witness only one of them, so the other arm is
    /// adopted and a write through the result reaches the caller's argument.  `parse_block`
    /// binds such a tail to a local — the bind copies each arm through its own temp
    /// (`@FR-B-Copy`) — but that is SKIPPED inside a generic TEMPLATE: a local minted in a
    /// body that is then CLONED into every monomorph reaches codegen in the clone with no
    /// slot.  The monomorph does not re-parse its block either, so it took neither, and the
    /// shape a generic makes easiest to write was the one left aliasing.
    ///
    /// Runs in the MONOMORPH's frame like the other post-substitution rewrites: the local
    /// belongs to the function the code lands in, and minting it here gives it that
    /// function's own slot.
    pub(crate) fn bind_monomorph_join_return(&mut self, d_nr: u32) {
        let ret = self.data.def(d_nr).returned().clone();
        if !matches!(ret.base(), Type::Reference(_, _) | Type::Enum(_, true, _)) {
            return;
        }
        let saved_ctx = self.context;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = d_nr;
        let mut code =
            std::mem::replace(&mut self.data.definitions[d_nr as usize].code, Value::Null);
        if let Value::Block(bl) = &mut code
            && let Some(last) = bl.operators.len().checked_sub(1)
        {
            // The tail arrives either already wrapped in a `Return` or as the bare branch —
            // a monomorph's body is the substituted TEMPLATE's, whose delivery has not run.
            let branch_is_tail = match bl.operators[last].unspan() {
                Value::Return(inner) => self.is_borrowing_branch(inner),
                other => self.is_borrowing_branch(other),
            };
            if branch_is_tail {
                let tmp = self.create_unique("__ret_join", &ret);
                self.vars.defined(tmp);
                let wrapped = matches!(bl.operators[last].unspan(), Value::Return(_));
                let branch = match bl.operators[last].unspan_mut() {
                    Value::Return(inner) => std::mem::replace(&mut **inner, Value::Var(tmp)),
                    other => std::mem::replace(other, Value::Var(tmp)),
                };
                bl.operators[last] = crate::data::v_set(tmp, branch);
                bl.operators.push(if wrapped {
                    Value::Return(Box::new(Value::Var(tmp)))
                } else {
                    Value::Var(tmp)
                });
            }
        }
        self.data.definitions[d_nr as usize].code = code;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = saved_ctx;
    }

    pub(crate) fn promote_monomorph_vector_return(
        &mut self,
        d_nr: u32,
        tmpl_vars: &crate::variables::Function,
        holders: &[u32],
    ) {
        // The borrowed-parameter verdict is read while the body is still in place — the
        // oracle classifies `def.code`, which the swap below moves out.
        let borrowed_param: Option<u16> =
            if matches!(self.data.def(d_nr).returned().base(), Type::Vector(_, _)) {
                match crate::use_analysis::return_ownership(&self.data, d_nr) {
                    crate::use_analysis::Own::Borrowed { base }
                        if (base as usize) < self.data.def(d_nr).attributes().len()
                            && !self.data.def(d_nr).attributes()[base as usize].hidden =>
                    {
                        Some(base)
                    }
                    _ => None,
                }
            } else {
                None
            };
        let saved_ctx = self.context;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = d_nr;
        let mut code =
            std::mem::replace(&mut self.data.definitions[d_nr as usize].code, Value::Null);
        // Which locals did the template type as the type variable?  Those are the binds the
        // record lowering was written for; a local the template already typed as a vector
        // got the vector lowering at the parse and is left alone.
        let tv_typed = |v: u16, vars: &crate::variables::Function| -> bool {
            (v as usize) < tmpl_vars.count() as usize
                && matches!(tmpl_vars.tp(v).base(), Type::Reference(h, _) if holders.contains(h))
                && (v as usize) < vars.count() as usize
                && matches!(vars.tp(v).base(), Type::Vector(_, _))
        };
        if let Value::Block(bl) = &mut code {
            // B-Copy: the whole-value vector binds.
            let mut declared: Vec<u16> = Vec::new();
            for op in &mut bl.operators {
                self.rewrite_generic_vector_binds(op, &tv_typed, &mut declared);
            }
            // F-Ret: the borrowed return.
            if let Some(param) = borrowed_param
                && (param as usize) < self.vars.count() as usize
                && let Type::Vector(elm, _) = self.vars.tp(param).base().clone()
                && Self::yields_var(&bl.operators, param)
            {
                let owned = Type::Vector(elm.clone(), Deps::none());
                let copy = self.create_unique("__ret_copy", &owned);
                if copy != u16::MAX {
                    self.vars.defined(copy);
                    let rec_tp = self.append_elem_tp(&elm);
                    for op in &mut bl.operators {
                        self.copy_returned_var_into(op, param, copy, rec_tp, false);
                    }
                    if let Some(last) = bl.operators.last_mut() {
                        self.copy_returned_var_into(last, param, copy, rec_tp, true);
                    }
                    bl.operators
                        .insert(0, crate::data::v_set(copy, Value::Null));
                    bl.result = owned;
                }
            }
        }
        self.data.definitions[d_nr as usize].code = code;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = saved_ctx;
    }
    /// The bind half of [`Self::promote_monomorph_vector_return`]: every `Set(v, Var(u))` with
    /// `v` type-variable-typed in the template and both sides vectors now becomes the copy.
    /// The FIRST assignment of a local is also its declaration — the null-init that allocates
    /// an owned vector local's store on both backends — so a first bind keeps a `Set(v, null)`
    /// in front of the copy; a rebind copies into the store the local already has.
    fn rewrite_generic_vector_binds(
        &mut self,
        node: &mut Value,
        tv_typed: &dyn Fn(u16, &crate::variables::Function) -> bool,
        declared: &mut Vec<u16>,
    ) {
        match node {
            Value::Span(b) => self.rewrite_generic_vector_binds(&mut b.1, tv_typed, declared),
            Value::Set(v, rhs) => {
                let first = !declared.contains(v);
                declared.push(*v);
                if let Value::Var(u) = rhs.unspan()
                    && *u != *v
                    && tv_typed(*v, &self.vars)
                    && (*u as usize) < self.vars.count() as usize
                    && !self.vars.is_argument(*v)
                    && let Type::Vector(elm, _) = self.vars.tp(*u).base().clone()
                {
                    let (v, u) = (*v, *u);
                    let rec_tp = self.append_elem_tp(&elm);
                    let replace = self.cl(
                        "OpReplaceVector",
                        &[Value::Var(v), Value::Var(u), Value::Int(rec_tp)],
                    );
                    *node = if first {
                        Value::Insert(vec![crate::data::v_set(v, Value::Null), replace])
                    } else {
                        replace
                    };
                } else {
                    self.rewrite_generic_vector_binds(rhs, tv_typed, declared);
                }
            }
            Value::Block(b) | Value::Loop(b) => {
                for op in &mut b.operators {
                    self.rewrite_generic_vector_binds(op, tv_typed, declared);
                }
            }
            Value::Insert(ops) => {
                for op in ops.iter_mut() {
                    self.rewrite_generic_vector_binds(op, tv_typed, declared);
                }
            }
            Value::If(_, t, e) => {
                self.rewrite_generic_vector_binds(t, tv_typed, declared);
                self.rewrite_generic_vector_binds(e, tv_typed, declared);
            }
            _ => {}
        }
    }
    /// Is EVERY return leaf of the body — the tail, each `return`, each arm of a branch in
    /// either position — the variable `x` itself?  A body with no leaf at all answers no.
    pub(crate) fn every_return_leaf_is_var(ops: &[Value], x: u16) -> bool {
        // (found, all) over the leaves reached.
        fn walk(v: &Value, x: u16, tail: bool, acc: &mut (bool, bool)) {
            match v.unspan() {
                Value::Return(inner) => walk(inner, x, true, acc),
                Value::If(_, t, e) => {
                    walk(t, x, tail, acc);
                    walk(e, x, tail, acc);
                }
                Value::Block(b) | Value::Loop(b) => {
                    let n = b.operators.len();
                    for (i, op) in b.operators.iter().enumerate() {
                        walk(op, x, tail && i + 1 == n, acc);
                    }
                }
                Value::Insert(ops) => {
                    let n = ops.len();
                    for (i, op) in ops.iter().enumerate() {
                        walk(op, x, tail && i + 1 == n, acc);
                    }
                }
                Value::Var(y) if tail => {
                    acc.0 = true;
                    if *y != x {
                        acc.1 = false;
                    }
                }
                Value::Null if tail => {}
                _ if tail => {
                    acc.0 = true;
                    acc.1 = false;
                }
                _ => {}
            }
        }
        let mut acc = (false, true);
        let n = ops.len();
        for (i, op) in ops.iter().enumerate() {
            walk(op, x, i + 1 == n, &mut acc);
        }
        acc.0 && acc.1
    }
    /// Does the body hand `x` up — as the tail or through a `return`?
    fn yields_var(ops: &[Value], x: u16) -> bool {
        fn leaf(v: &Value, x: u16, tail: bool) -> bool {
            match v.unspan() {
                Value::Return(inner) => leaf(inner, x, true),
                Value::Var(y) => tail && *y == x,
                Value::If(_, t, e) => leaf(t, x, tail) || leaf(e, x, tail),
                Value::Block(b) | Value::Loop(b) => {
                    let n = b.operators.len();
                    b.operators
                        .iter()
                        .enumerate()
                        .any(|(i, op)| leaf(op, x, tail && i + 1 == n))
                }
                Value::Insert(ops) => {
                    let n = ops.len();
                    ops.iter()
                        .enumerate()
                        .any(|(i, op)| leaf(op, x, tail && i + 1 == n))
                }
                _ => false,
            }
        }
        let n = ops.len();
        ops.iter()
            .enumerate()
            .any(|(i, op)| leaf(op, x, i + 1 == n))
    }
    /// The return half of [`Self::promote_monomorph_vector_return`]: every leaf `x` in return
    /// position — the tail when `tail`, a `return x` anywhere — becomes a copy into `copy`
    /// followed by `copy`, so what leaves the frame is a store the frame minted.
    fn copy_returned_var_into(
        &mut self,
        node: &mut Value,
        x: u16,
        copy: u16,
        rec_tp: i32,
        tail: bool,
    ) {
        match node {
            Value::Span(b) => self.copy_returned_var_into(&mut b.1, x, copy, rec_tp, tail),
            Value::Return(inner) => {
                if matches!(inner.unspan(), Value::Var(y) if *y == x) {
                    let replace = self.cl(
                        "OpReplaceVector",
                        &[Value::Var(copy), Value::Var(x), Value::Int(rec_tp)],
                    );
                    *node = Value::Insert(vec![replace, Value::Return(Box::new(Value::Var(copy)))]);
                } else {
                    self.copy_returned_var_into(inner, x, copy, rec_tp, true);
                }
            }
            Value::Var(y) if tail && *y == x => {
                let replace = self.cl(
                    "OpReplaceVector",
                    &[Value::Var(copy), Value::Var(x), Value::Int(rec_tp)],
                );
                *node = Value::Insert(vec![replace, Value::Var(copy)]);
            }
            Value::If(_, t, e) => {
                self.copy_returned_var_into(t, x, copy, rec_tp, tail);
                self.copy_returned_var_into(e, x, copy, rec_tp, tail);
            }
            Value::Block(b) | Value::Loop(b) => {
                let n = b.operators.len();
                for (i, op) in b.operators.iter_mut().enumerate() {
                    self.copy_returned_var_into(op, x, copy, rec_tp, tail && i + 1 == n);
                }
                if tail {
                    b.result = Type::Vector(
                        match self.vars.tp(copy).base() {
                            Type::Vector(elm, _) => elm.clone(),
                            other => Box::new(other.clone()),
                        },
                        Deps::none(),
                    );
                }
            }
            Value::Insert(ops) => {
                let n = ops.len();
                for (i, op) in ops.iter_mut().enumerate() {
                    self.copy_returned_var_into(op, x, copy, rec_tp, tail && i + 1 == n);
                }
            }
            _ => {}
        }
    }
    pub(crate) fn promote_monomorph_text_return(&mut self, d_nr: u32) {
        // Only plain `text` / `text?` returns (tuple-of-text is a separate arc).
        if !matches!(
            self.data.definitions[d_nr as usize].returned.base(),
            Type::Text(_)
        ) {
            return;
        }
        // Swap parse context onto the monomorph: `create_unique` mutates
        // `self.vars`; `text_return` mutates `self.data.def(self.context)` +
        // `self.vars`.  `code` is moved OUT of `self.data` first so we can hold
        // `&mut code` while calling `&mut self` methods (code is a local, not
        // borrowed from self).
        let saved_ctx = self.context;
        // Swap the monomorph's variable table into `self.vars` (the old table is
        // parked in `def.variables` and swapped back below — `Function` has no
        // `Default`, so a swap, not a take).
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = d_nr;
        let mut code =
            std::mem::replace(&mut self.data.definitions[d_nr as usize].code, Value::Null);
        if let Value::Block(bl) = &mut code {
            let l = &mut bl.operators;
            // Same gate as parse_block's `do_tret_bind`: a promotable text tail.
            // ALSO promote a `BuiltLocal` tail (accumulator / concat / interpolation):
            // for a non-generic, `text_return` promotes those directly, but that path
            // is skipped for the generic template (control.rs I9-var guard), so a
            // generic monomorph's built-up text return would otherwise orphan — the
            // `run() -> text { label(42) }` / `label<T> -> text { x.to_text() + "!" }`
            // case (plan17_b).  The `__tret` bind + `text_return` below delivers it
            // through the hidden `&text` buffer exactly as the non-generic would.
            // @PLN85 forward-ref class.
            // @PLN85 corpus (p205 / 86) — ALSO promote a FORWARD-BORROW call tail
            // (`lab<T>(x) -> text { x.tolab() }`, where `tolab -> text["self"]`).
            // A non-generic forwards the borrow as a `text["x"]` view (the caller
            // frees the arg it borrows — leak-free), but a monomorph is built by IR
            // substitution and keeps the BARE declared `text` return: the arg-dep is
            // never derived, so the borrowed view is copied into an owned String and
            // orphaned on the interpreter (native RAII-drops it).  Deliver it through
            // the hidden `&text` buffer exactly as the manual rebind (`y = x.tolab();
            // y`) already does — a `Borrow::ForwardArg` verdict is only ever a call.
            let tail_promotable = l.last().is_some_and(|tail| {
                self.tret_bind_ok(tail, l)
                    || matches!(
                        self.classify_text_return(tail, l),
                        TextReturn::Owned(OwnedVia::BuiltLocal)
                            | TextReturn::Borrow(BorrowVia::ForwardArg)
                    )
            });
            // @PLN85 corpus (86 / if_describe) — an EARLY `return` (not the tail)
            // can be the orphaning path: `describe<T>(x) { if x.ok() { return
            // x.tolab() } ; "invalid" }` returns a forward-borrow through the guard
            // but a literal at the tail.  Promote on either signal, then deliver
            // EVERY return — tail and early — into the one `&text` buffer.
            let tail_ix = l.len().saturating_sub(1);
            let early_promotable = l[..tail_ix]
                .iter()
                .any(|op| self.early_text_return_orphans(op, l));
            // `LOFT_DBG_ACC=1` — the monomorph twin of `parse_block`'s gate line: every term
            // of the decision, one line per monomorph.
            if std::env::var_os("LOFT_DBG_ACC").is_some() {
                eprintln!(
                    "[acc-mono] fn={} pass1={} tail_promotable={tail_promotable}                      early_promotable={early_promotable} if_acc={} tail={:?}",
                    self.data.def(d_nr).name(),
                    self.first_pass,
                    self.monomorph_if_acc_ok(d_nr, l, &bl.result),
                    l.last().map(|t| self.classify_text_return(t, l)),
                );
            }
            if tail_promotable || early_promotable {
                let tv = self.create_unique("__tret", &Type::Text(Deps::none()));
                if tv != u16::MAX {
                    // Route every EARLY `return <e>` through the buffer:
                    // `return <e>` → `{ Set(__tret, <e>); return __tret }`.
                    let last = l.len() - 1;
                    for op in &mut l[..last] {
                        Self::rewrite_text_returns_into(op, tv);
                    }
                    if matches!(l[last].unspan(), Value::Return(_)) {
                        let ret = std::mem::replace(
                            &mut l[last],
                            Value::Return(Box::new(Value::Var(tv))),
                        );
                        l.insert(last, crate::data::v_set(tv, Self::peel_to_inner_call(ret)));
                    } else {
                        let call = std::mem::replace(&mut l[last], Value::Var(tv));
                        l.insert(last, crate::data::v_set(tv, call));
                    }
                    // `text_return([tv])` stamps the hidden `&text` buffer attr on
                    // the monomorph def and rewrites its returned type — the same
                    // call `block_result` makes for a non-generic text tail.
                    self.text_return(&[tv]);
                }
            } else if self.monomorph_if_acc_ok(d_nr, l, &bl.result) {
                // The `do_if_acc` half of `parse_block`.  A value-yielding `if`/`match`
                // text tail is not a `__tret` bind — binding the whole branch as one
                // value is what native rejects — so each ARM writes an accumulator
                // instead, and `text_return` promotes that accumulator to the caller's
                // hidden `&text` buffer.  Identical rewrite, identical gate.
                //
                // Without it a monomorph kept the owned-by-value tail its non-generic
                // twin never has: the scope pass materialised it into a `skipfree`
                // `__ret_N` the callee hands out and nobody frees — one orphaned String
                // per call (loft#1026, the loft#568 class).  An `a?` discharge is an
                // `If`, which is why a `-> T` generic reaches this and not the bind above.
                let acc_type = if matches!(self.data.def(d_nr).returned(), Type::Optional(_)) {
                    Type::Optional(Box::new(Type::Text(Deps::none())))
                } else {
                    Type::Text(Deps::none())
                };
                let av = self.create_unique("__acc", &acc_type);
                if av != u16::MAX {
                    let last = l.len() - 1;
                    let mut tail = std::mem::replace(&mut l[last], Value::Null);
                    let is_ret = matches!(tail.unspan(), Value::Return(_));
                    if let Value::Return(inner) = tail {
                        tail = *inner;
                    }
                    Self::push_text_arms_into(&mut tail, av, self.data.def_nr("OpCreateStack"));
                    l[last] = tail;
                    // Load-bearing, exactly as at the parse-time site: the per-arm `Set`s
                    // live INSIDE the branch, so nothing else introduces `av` here.
                    l.insert(last, crate::data::v_set(av, Value::Text(String::new())));
                    l.push(if is_ret {
                        Value::Return(Box::new(Value::Var(av)))
                    } else {
                        Value::Var(av)
                    });
                    self.text_return(&[av]);
                    bl.result = Type::Text(Deps::frame1(av));
                }
            }
        }
        // Restore: move code back, swap the (now-promoted) monomorph vars back
        // into `def.variables`, reset context.
        self.data.definitions[d_nr as usize].code = code;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = saved_ctx;
    }

    /// @PLN104 targeted promotion — Phase A (CALLEE): promote a `force_tret` def's owned-text
    /// return to a hidden `&text` retbuf IN PLACE on the pass-2 IR — NO whole-file re-parse
    /// (the third pass re-lowers every def non-idempotently, damaging unpromoted collateral —
    /// the `var__vec` / diagnostic / s5-s7 class).  Mirrors `promote_monomorph_text_return`
    /// (swap the def's var table + code onto `self`, rebind the tail + early returns through
    /// `__tret`, `text_return`), THEN the `a == v` renumber + stamps the block type.  The
    /// promotion is already DECIDED (`force_tret`), so there is no tail-promotability gate.
    /// Phase B (`patch_tret_callers`) pushes the retbuf arg at each call site.
    pub(crate) fn promote_text_return_def(&mut self, d_nr: u32) {
        if !matches!(
            self.data.definitions[d_nr as usize].returned.base(),
            Type::Text(_)
        ) {
            return;
        }
        let saved_ctx = self.context;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = d_nr;
        let mut code =
            std::mem::replace(&mut self.data.definitions[d_nr as usize].code, Value::Null);
        if let Value::Block(bl) = &mut code
            && !bl.operators.is_empty()
        {
            let tv = self.create_unique("__tret", &Type::Text(Deps::none()));
            if tv != u16::MAX {
                // Route every EARLY `return <e>` through the buffer, then the tail:
                // bare `<call>` → `Set(__tret, <call>); __tret`; `return <call>` likewise.
                let last = bl.operators.len() - 1;
                for op in &mut bl.operators[..last] {
                    Self::rewrite_text_returns_into(op, tv);
                }
                if matches!(bl.operators[last].unspan(), Value::Return(_)) {
                    let ret = std::mem::replace(
                        &mut bl.operators[last],
                        Value::Return(Box::new(Value::Var(tv))),
                    );
                    bl.operators
                        .insert(last, crate::data::v_set(tv, Self::peel_to_inner_call(ret)));
                } else if Self::ir_diverges(&bl.operators[last]) {
                    // The tail YIELDS NOTHING: every path through it returns
                    // (`fn f() -> text { if c { return a; } else { return b; } }`, and the
                    // same after a `match` lowers to nested `If`).  Binding it as a value
                    // emits `__tret = (if … { return … } else { return … })` — a store
                    // control can never reach.  The interpreter survives that (the
                    // `return` leaves before the store completes), but rustc cannot type
                    // `.to_string()` on `!` and rejects the whole function with E0282, so
                    // an ordinary shape compiled on one backend and not the other.
                    //
                    // It is the same job the loop above does for every EARLIER operator,
                    // just reached at the tail: route each inner `return <e>` through the
                    // buffer.  The trailing `Var(tv)` keeps the block's result the
                    // `text["__tret"]` shape `text_return` and `av_renumber_retbuf` expect;
                    // it is unreachable, exactly like the value it replaces.
                    Self::rewrite_text_returns_into(&mut bl.operators[last], tv);
                    bl.operators.push(Value::Var(tv));
                } else {
                    let call = std::mem::replace(&mut bl.operators[last], Value::Var(tv));
                    bl.operators.insert(last, crate::data::v_set(tv, call));
                }
                // Stamp the hidden `&text` retbuf attr + returned type, then align `a == v`.
                self.text_return(&[tv]);
                bl.result = self.av_renumber_retbuf(&mut bl.operators, tv);
            }
        }
        self.data.definitions[d_nr as usize].code = code;
        std::mem::swap(
            &mut self.vars,
            &mut self.data.definitions[d_nr as usize].variables,
        );
        self.context = saved_ctx;
    }

    /// @PLN104 targeted promotion — the entry point, replacing the whole-file third pass.
    /// Phase A promotes each `force_tret` callee IN PLACE (its signature gains the `&text`
    /// retbuf); Phase B then patches each direct caller to push the retbuf arg.  A must run
    /// before B (callers need the promoted signature).  Runs post-pass-2, pre-`scopes::check`,
    /// so the added work-text delivery/frees are woven in by the normal scope pass.
    pub(crate) fn targeted_tret_promotion(&mut self) {
        // v1 SCOPE — promote only the class Phase A/B lower correctly on BOTH backends:
        // an **owned-by-value** text return (the fn-ref-call / built-local #568 tail) on a
        // def that is CALLED directly.  Defer, with a loud log, the classes that need more
        // than a retbuf rebind (see targeted-promotion-design.md § Verification):
        //   - view-of-local / join-of-local: the return ALIASES a frame-local, so the retbuf
        //     must be filled by MATERIALISING the view (deep-copy), which Phase A does not
        //     emit — a bare rebind SIGSEGVs both backends (553 `textslice` returns `ts[0][0]`).
        //   - address-taken defs (used as a fn-ref VALUE `FnRef(d,…)`): promotion changes the
        //     signature, so every fn-pointer to it stops type-checking (native E0308/E0425).
        let mut addr_taken: std::collections::HashSet<u32> = std::collections::HashSet::new();
        for c in 0..self.data.definitions.len() as u32 {
            self.data.def(c).code.walk(&mut |v| {
                if let Value::FnRef(d, _, _) = v
                    && *d >= 0
                {
                    addr_taken.insert(*d as u32);
                }
            });
        }
        let promote: Vec<u32> = self
            .force_tret
            .iter()
            .copied()
            .filter(|&d| {
                crate::use_analysis::text_return_orphan_risk(&self.data, d).is_some()
                    && !addr_taken.contains(&d)
            })
            .collect();
        let deferred = self.force_tret.len() - promote.len();
        if deferred > 0 && std::env::var_os("LOFT_TRET_TRACE").is_some() {
            eprintln!(
                "[tret-v2] promoting {}/{} force_tret defs ({deferred} deferred: view/join/address-taken)",
                promote.len(),
                self.force_tret.len(),
            );
        }
        // Narrow force_tret to the promoted set so Phase B (`patch_tret_callers`) patches
        // callers of exactly the defs whose signature actually changed.
        self.force_tret = promote.iter().copied().collect();
        for &d in &promote {
            self.promote_text_return_def(d);
        }
        self.patch_tret_callers();
    }

    /// @PLN104 targeted promotion — Phase B (CALLER): for every def whose code calls a
    /// promoted `force_tret` def, push the retbuf arg at each such `Call` by re-running
    /// `add_defaults` against the now-promoted signature (its `RefVar(Text)` arm builds the
    /// caller-side work-text buffer — mod.rs).  No re-parse: the pass-2 call carries exactly
    /// the declared args (probed), so `add_defaults` appends precisely the one retbuf.
    fn patch_tret_callers(&mut self) {
        let force = self.force_tret.clone();
        if force.is_empty() {
            return;
        }
        for c in 0..self.data.definitions.len() as u32 {
            let calls_promoted = {
                let mut hit = false;
                self.data.def(c).code.walk(&mut |v| {
                    if let Value::Call(d, _) = v
                        && force.contains(d)
                    {
                        hit = true;
                    }
                });
                hit
            };
            if !calls_promoted {
                continue;
            }
            let saved_ctx = self.context;
            std::mem::swap(
                &mut self.vars,
                &mut self.data.definitions[c as usize].variables,
            );
            self.context = c;
            let before: std::collections::HashSet<u16> =
                self.vars.work_texts().into_iter().collect();
            // The caller was parsed against the UNPROMOTED callee, and storing it reset the
            // pooling counters, so they lag the work names already in `names`.  Sync them
            // ALL first — every retbuf `add_defaults` mints below has to be a FRESH buffer,
            // never an alias of a live buffer in the same call: a stale `__work_N` collides
            // with a format-arg buffer (native E0506/E0499), and a stale `__work_cN` hands
            // the callee the very buffer holding its own argument (loft#671).
            self.vars.sync_work_counters();
            let mut code =
                std::mem::replace(&mut self.data.definitions[c as usize].code, Value::Null);
            self.patch_tret_call(&mut code, &force);
            // `add_defaults` minted the caller-side retbuf as a fresh work-text but did
            // NOT declare it at the caller's top level; without a top-level first-def
            // `scopes::check` scopes it to the arg block and frees it there — before the
            // callee fills it — orphaning the delivered text (loft#568).  Re-parse hoists
            // these decls in `expression_value`; post-parse we replay that hoist here for
            // ONLY the newly-minted work-texts, so each frees at the caller's scope exit.
            if let Value::Block(bl) = &mut code {
                for wt in self.vars.work_texts() {
                    if !before.contains(&wt) {
                        bl.operators
                            .insert(0, v_set(wt, Value::Text(String::new())));
                    }
                }
            }
            self.data.definitions[c as usize].code = code;
            std::mem::swap(
                &mut self.vars,
                &mut self.data.definitions[c as usize].variables,
            );
            self.context = saved_ctx;
        }
    }

    /// Recurse `node`, appending the retbuf arg to every `Call(d, …)` with `d ∈ force`.
    ///
    /// Also used by `instantiate_nested_generics`: a call retargeted at a freshly created
    /// monomorph faces the same mismatch this solves — the promoted callee takes a hidden
    /// `&text` buffer the already-built call does not pass.
    pub(crate) fn patch_tret_call(
        &mut self,
        node: &mut Value,
        force: &std::collections::HashSet<u32>,
    ) {
        match node {
            Value::Call(d, args) => {
                for a in args.iter_mut() {
                    self.patch_tret_call(a, force);
                }
                if force.contains(d) {
                    let d = *d;
                    let n_attrs = self.data.attributes(d);
                    if args.len() < n_attrs {
                        let mut actual = std::mem::take(args);
                        let mut types = vec![Type::Unknown(0); actual.len()];
                        self.add_defaults(d, &mut actual, &mut types);
                        *args = actual;
                    }
                }
            }
            Value::CallRef(_, xs) | Value::Insert(xs) | Value::Tuple(xs) | Value::Parallel(xs) => {
                for x in xs {
                    self.patch_tret_call(x, force);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for op in &mut bl.operators {
                    self.patch_tret_call(op, force);
                }
            }
            Value::If(c, t, e) => {
                self.patch_tret_call(c, force);
                self.patch_tret_call(t, force);
                self.patch_tret_call(e, force);
            }
            Value::Set(_, inner)
            | Value::Return(inner)
            | Value::Drop(inner)
            | Value::Yield(inner)
            | Value::TuplePut(_, _, inner) => self.patch_tret_call(inner, force),
            Value::Span(b) => self.patch_tret_call(&mut b.1, force),
            Value::Iter(_, a, b, c) => {
                self.patch_tret_call(a, force);
                self.patch_tret_call(b, force);
                self.patch_tret_call(c, force);
            }
            _ => {}
        }
    }

    /// @PLN85 corpus — true when a NON-tail `return` in `op` delivers a text that
    /// would orphan on the interpreter for a generic monomorph: an owned call /
    /// built-local, or a forward-borrow call (`if c { return x.m() }`).  Recurses
    /// `if`/`match` arms + inline blocks (a return anywhere in them still returns
    /// from the fn); stops at other constructs.  `block` is the fn body for the
    /// `var_built_in_block` check inside `classify_text_return`.
    fn early_text_return_orphans(&self, op: &Value, block: &[Value]) -> bool {
        match op.unspan() {
            Value::Return(inner) => {
                matches!(
                    self.classify_text_return(inner, block),
                    TextReturn::Owned(_) | TextReturn::Borrow(BorrowVia::ForwardArg)
                )
                    // loft#1026 — `if a { return a? }` returns a value-yielding branch, the
                    // same shape `do_if_acc` promotes in TAIL position.  Its arms are an
                    // argument borrow and a literal, so `classify_text_return` reads `Plain`
                    // and the two verdicts above miss it — while a monomorph's return type
                    // carries no argument deps (the template never derived any), so the
                    // scope pass materialised it into a `skipfree` `__ret_N` nobody frees.
                    || Self::if_tail_yields_text(inner)
            }
            Value::If(_, t, e) => {
                self.early_text_return_orphans(t, block) || self.early_text_return_orphans(e, block)
            }
            Value::Block(bl) => bl
                .operators
                .iter()
                .any(|o| self.early_text_return_orphans(o, block)),
            Value::Insert(ops) => ops.iter().any(|o| self.early_text_return_orphans(o, block)),
            _ => false,
        }
    }

    /// @PLN85 corpus — rewrite every `return <e>` in `op` (recursing `if`/`match`
    /// arms + inline blocks) to deliver into the promoted `&text` buffer `tv`:
    /// `return <e>` → `{ Set(tv, <e>); return tv }`.  The companion of the tail
    /// rebind in `promote_monomorph_text_return`, applied to the EARLY returns so
    /// all paths write the one caller buffer (no orphaned owned copy).
    /// Does every path through `op` leave the block — i.e. does it yield no value?
    ///
    /// A `return` diverges; an `if` diverges when BOTH arms do (a one-armed `if` falls
    /// through); a block diverges when any operator in it does. Used to tell a tail that
    /// produces the function's value from one that only ever returns, which decide
    /// different lowerings (`promote_text_return_def`).
    fn ir_diverges(op: &Value) -> bool {
        match op.unspan() {
            Value::Return(_) => true,
            Value::If(_, t, e) => Self::ir_diverges(t) && Self::ir_diverges(e),
            Value::Block(bl) => bl.operators.iter().any(Self::ir_diverges),
            Value::Insert(ops) => ops.iter().any(Self::ir_diverges),
            _ => false,
        }
    }

    fn rewrite_text_returns_into(op: &mut Value, tv: u16) {
        match op {
            Value::Span(b) => Self::rewrite_text_returns_into(&mut b.1, tv),
            Value::Return(_) => {
                let ret = std::mem::replace(op, Value::Null);
                let expr = Self::peel_to_inner_call(ret);
                *op = Value::Insert(vec![
                    crate::data::v_set(tv, expr),
                    Value::Return(Box::new(Value::Var(tv))),
                ]);
            }
            Value::If(_, t, e) => {
                Self::rewrite_text_returns_into(t, tv);
                Self::rewrite_text_returns_into(e, tv);
            }
            Value::Block(bl) => {
                for o in &mut bl.operators {
                    Self::rewrite_text_returns_into(o, tv);
                }
            }
            Value::Insert(ops) => {
                for o in ops {
                    Self::rewrite_text_returns_into(o, tv);
                }
            }
            // A `return` inside a loop body is a delivery site like any other: `for v in it {
            // return v; } d` handed the loop variable up as a view of a local the loop's own
            // free never reached on that path (loft#1357), because this walk stopped at the
            // loop.  `Drop` wraps a statement whose value is discarded and can hold one too.
            Value::Loop(bl) => {
                for o in &mut bl.operators {
                    Self::rewrite_text_returns_into(o, tv);
                }
            }
            Value::Iter(_, a, b, c) => {
                Self::rewrite_text_returns_into(a, tv);
                Self::rewrite_text_returns_into(b, tv);
                Self::rewrite_text_returns_into(c, tv);
            }
            Value::Drop(inner) => Self::rewrite_text_returns_into(inner, tv),
            _ => {}
        }
    }

    /// Walk a return expression to find work-ref variables passed as hidden
    /// Reference arguments to struct-returning calls.  Used by `block_result`
    /// to recover deps that `filter_hidden` stripped from the return type.
    /// Issue #120: without this, the work-ref stays a local and gets freed
    /// before the caller reads the return value.
    /// True iff the callee returns a FOREIGN store it never writes into the
    /// hidden return buffer it was handed — `fn f() -> vector { g() }` where
    /// `g`'s result is delivered by `g` itself (a native builtin, or another
    /// forwarder), not built into `f`'s buffer.  Read off the callee's BODY
    /// (the tail is a `Call` whose own callee exposes no hidden heap buffer
    /// arg for `f`'s value), which is pass-stable.  A callee whose body is
    /// not parsed yet (forward ref, `code == Null`) is assumed to CONSUME —
    /// the common multi-site wrapper case #355 needs.
    fn callee_forwards_foreign_store(&self, d_nr: u32) -> bool {
        let def = self.data.def(d_nr);
        if *def.code() == Value::Null {
            return false; // unparsed / native stub — assume it consumes.
        }
        fn tail_forwards(node: &Value, data: &crate::data::Data) -> bool {
            match node.unspan() {
                Value::Call(d, _) => {
                    Parser::collect_hidden_ref_args(node, data).is_empty()
                        && matches!(
                            data.def(*d).returned(),
                            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                        )
                }
                Value::Block(bl) => bl.operators.last().is_some_and(|o| tail_forwards(o, data)),
                Value::Insert(ops) => ops.last().is_some_and(|o| tail_forwards(o, data)),
                Value::Return(inner) => tail_forwards(inner, data),
                _ => false,
            }
        }
        tail_forwards(def.code(), &self.data)
    }

    pub(crate) fn collect_hidden_ref_args(val: &Value, data: &crate::data::Data) -> Vec<u16> {
        match val {
            Value::Call(d_nr, args) => {
                let mut result = Vec::new();
                let attrs = data.def(*d_nr).attributes();
                for (i, attr) in attrs.iter().enumerate() {
                    if attr.hidden
                        && matches!(
                            attr.typedef,
                            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                        )
                        && let Some(Value::Var(v)) = args.get(i)
                    {
                        result.push(*v);
                    }
                }
                result
            }
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last() {
                    Self::collect_hidden_ref_args(last, data)
                } else {
                    vec![]
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last() {
                    Self::collect_hidden_ref_args(last, data)
                } else {
                    vec![]
                }
            }
            Value::Set(_, inner) => Self::collect_hidden_ref_args(inner, data),
            // P198 convention: the parser wraps most expressions — including a
            // body-tail call — in `Value::Span` for diagnostics.  Unwrap so a
            // `Span(Call(...))` tail is recognised, matching the Block/Insert/
            // Set/If arms.  Without this, a thin wrapper `fn f() -> S { g() }`
            // whose tail is `Span(Call(g, [..., __ref_N]))` recovers no hidden
            // work-ref, `ref_return` never promotes the placeholder to f's
            // hidden return dest, and the local `__ref_N` is freed at f's exit
            // — corrupting the returned struct's store once it is reused (#299).
            Value::Span(b) => Self::collect_hidden_ref_args(&b.1, data),
            Value::If(_, t, f) => {
                let mut r = Self::collect_hidden_ref_args(t, data);
                r.extend(Self::collect_hidden_ref_args(f, data));
                r
            }
            _ => vec![],
        }
    }

    /// #306: true when a ref-typed return value may alias a LOCAL's store —
    /// a transitive dep of `ls` resolves to a variable that is not a function
    /// attribute.  The direct `ls` entries themselves are the NRVO-promotion
    /// candidates (handled by `ref_return`); it is their *deps* that reveal a
    /// borrow.  Such a view dangles the moment the local owner's store is
    /// freed at function exit, so the return value must be materialised.
    /// @PLN85 move-on-block-return — does block body `l` DEFINE variable `v`
    /// (an assignment `Set(v, …)` at statement level, incl. through Span/Line/
    /// Insert wrappers)? True ⇒ `v` is a fresh block-local that dies at block
    /// exit, so a tail viewing it must be materialised (copied) rather than
    /// escaping as a borrow. False ⇒ `v` is defined in an enclosing scope (an
    /// outer local / param returned by the block) — a genuine borrow to keep.
    /// Is `v` BOUND to a branch join (`v = if c { … } else { … }`)?  Ask before renaming a
    /// local onto the caller's return buffer: a local this answers `true` for may not be.
    ///
    /// Enforces @FR-O-Owner / @FR-O-Move.  The rename is sound only where the local's value
    /// is BUILT INTO the buffer, as a literal is.  A join is not: each arm mints its own
    /// backing and the assignment REBINDS the slot (`PutRef`), so the buffer ends up with
    /// no owner and the arm's store is handed back to a caller whose binding does not name
    /// it — two stores with no owner, from one return.  Refusing the rename leaves the
    /// candidate on `Bind`, which keeps the local and copies it into a separate `__retbuf`;
    /// that is the delivery a join written at the function TAIL already takes.
    ///
    /// The question is STRUCTURAL rather than a reading of `deps` because the answer is
    /// needed on pass 1, where the deps are still empty (`vector_db` runs only on pass 2).
    /// The two passes must agree here — the verdict decides whether the function takes a
    /// hidden buffer argument, so a verdict that differed would move the ABI between them.
    /// `match` lowers to nested `If`, so one shape covers both spellings.  (loft#1081,
    /// D-own-8 in `formal/ownership.md`.)
    fn var_bound_to_branch(l: &[Value], v: u16) -> bool {
        fn rhs_is_branch(node: &Value) -> bool {
            match node.unspan() {
                Value::If(_, _, _) => true,
                Value::Block(bl) => bl.operators.last().is_some_and(rhs_is_branch),
                Value::Insert(ops) => ops.last().is_some_and(rhs_is_branch),
                _ => false,
            }
        }
        fn scan(op: &Value, v: u16) -> bool {
            match op.unspan() {
                Value::Set(w, rhs) => *w == v && rhs_is_branch(rhs),
                Value::Insert(ops) => ops.iter().any(|o| scan(o, v)),
                Value::Block(bl) => bl.operators.iter().any(|o| scan(o, v)),
                _ => false,
            }
        }
        l.iter().any(|op| scan(op, v))
    }

    fn block_defines_var(l: &[Value], v: u16) -> bool {
        l.iter().any(|op| Self::stmt_defines_var(op, v))
    }

    /// What an `if`/`match` arm contributes to its branch JOIN: the deps it borrows from
    /// OUTSIDE itself (loft#978).
    ///
    /// A dep naming a store the arm's own body MINTS is that arm's ownership marker, not
    /// a borrow: `[]` lowers to `OpDatabase(__vdb_N, …)` and the value types as a dep on
    /// `__vdb_N`, which is how it says *I own this store*. Joining that into a sibling's
    /// type publishes it as something the joined value VIEWS, and the return machinery
    /// then reads the result as a view of a local — a different delivery question, and
    /// one with no answer here (`deliver`'s return went from `["__retbuf", "e"]` to an
    /// unresolvable `["??"]`).
    ///
    /// Only the CONTRIBUTED side is filtered. The arm the result type is taken from
    /// keeps its own marker, because that is what says which store the result owns.
    /// Enforces @FR-O-Complete for the arm side: what an arm CONTRIBUTES to the
    /// join's borrow set.  The join's own owned-vs-borrowed reconciliation lives in
    /// [`Type::joined_deps`], which cites the same rule.
    fn arm_join_type(&self, arm: &Value, tp: &Type) -> Type {
        let Some(deps) = tp.borrow_deps() else {
            return tp.clone();
        };
        if deps.is_empty() {
            return tp.clone();
        }
        let minted = crate::use_analysis::minted_vars(&self.data, arm);
        if minted.is_empty() {
            return tp.clone();
        }
        let mut kept = deps.clone();
        kept.retain(|v| !minted.contains(v));
        if kept.len() == deps.len() {
            return tp.clone();
        }
        tp.rewrap_deps(&kept)
    }

    /// Fold one `match`/`if` arm into the branch JOIN — the deps it contributes
    /// ([`arm_join_type`](Self::arm_join_type)) and its NULLABILITY.
    ///
    /// `joined_deps` keeps the shape of the type it is called on, which is the FIRST arm's, and
    /// merges only the borrow set. That is right for the shape and wrong for the `?`:
    /// `(N-Join)` says a join has type `⨆ᵢ τᵢ`, *made OPTIONAL iff some `τᵢ` is optional*, and
    /// a later arm's `Optional` was being dropped rather than joined. `x: integer = match k
    /// { 9 => { 1 }, _ => { maybe(k) } }` typed the join `integer`, so the destination's
    /// `(N-Store)` teeth had nothing to bite and a declared non-null slot held null with no
    /// diagnostic on either backend (loft#1103, `types.md` D-Null-Join).
    ///
    /// A `null` LITERAL in the same arm was always caught, by the DN1 walkers that match the
    /// `OpConv*FromNull` node it lowers to. A nullable-TYPED value produces no null-shaped node
    /// at all, so nothing asked about it. Asking the TYPE is the spelling-free form of that
    /// question, which is why it belongs here rather than beside another walker.
    ///
    /// One home for the fact because there are SIX arm sites — the ordinary arm, the wildcard,
    /// the struct and enum arms, the vector-match arm — and a per-site fix would answer the
    /// same question six times and drift at the first one anybody forgets. `Void` / `Null` arms
    /// are left alone: they carry no type of their own to contribute, and the DN1 walkers own
    /// the bare-`null` arm.
    /// Do a `match`'s settled result type and one arm's type agree?
    ///
    /// [`match_arm_types_unify`] modulo ownership, plus the one case the JOIN creates:
    /// once an arm has widened the result to an enum ([`Self::join_arm_into`]), a later
    /// arm naming one of that enum's VARIANTS agrees with it — `@FR-C-Var` licenses
    /// `Reference(S) ⤳ Enum(E)` exactly there.  The arm keeps its own variant type on
    /// purpose (`arm_joins_to_enum` returns before converting it, so the join can still
    /// see the two differ), which is why the gate cannot read the arm's type alone.
    fn match_arms_unify(&self, result: &Type, arm: &Type) -> bool {
        match_arm_types_unify(result, arm)
            || matches!(
                (self.variant_parent_enum(arm), result),
                (Some(Type::Enum(a, _, _)), Type::Enum(b, _, _)) if a == *b
            )
    }

    /// Does this ARM join to an enum rather than convert to what its siblings have
    /// settled on so far?
    ///
    /// @FR-C-Var licenses `Reference(S) ⤳ Enum(E)` for each variant and nothing between
    /// two of them, so wherever the siblings have settled on ONE variant, "does this arm
    /// convert to that?" is the wrong question for two kinds of arm: another variant of
    /// the same enum, and the ENUM itself.  Both join to the enum, which is wider than the
    /// expected type, so `convert` is asked in the direction the rules do not license and
    /// answers *"expected Circle, got Sh"* for a join that is admissible (loft#1117 for the
    /// variant, loft#1390 for the enum — `match e { Circle{r} => Circle{r: r + 1}, _ => e }`,
    /// where `_ => e` is the binding this very statement assigns).
    ///
    /// One home with the site that DECIDES the join ([`Self::joins_to_enum`], read by
    /// `parse_if` and by [`Self::join_arm_into`]): an arm accepted here must be an arm the
    /// join widens for, or a slot declared as one variant ends up holding another.
    /// The SAME variant twice is not such a pair — that conversion is reflexive and takes
    /// the ordinary path.
    fn arm_joins_to_enum(&self, arm: &Type, expected: &Type) -> bool {
        self.variant_parent_enum(expected)
            .is_some_and(|e| self.joins_to_enum(&e, expected, arm))
    }

    /// Do a then-arm and an else-arm join to `enum_tp` rather than to the then-arm's own
    /// variant?
    ///
    /// True when the else arm is a DIFFERENT variant of the same enum, or the enum itself.
    /// False for the same variant (nothing widened), and false for a `Void` / `Never` /
    /// `Null` else arm — a diverging or valueless arm states no type to join with, and an
    /// `else if` chain deliberately keeps its shape out of `false_type` (loft#936).
    fn joins_to_enum(&self, enum_tp: &Type, true_type: &Type, false_type: &Type) -> bool {
        let Type::Enum(e, _, _) = enum_tp else {
            return false;
        };
        match (true_type, false_type) {
            // A sibling variant, and only a sibling: the arm's def must belong to THIS
            // enum.  The acceptance sites read this predicate too (`arm_joins_to_enum`),
            // so an unrelated struct reaching it would be waved past the conversion it
            // has to fail rather than widened.
            (Type::Reference(a, _), Type::Reference(b, _)) => {
                a != b
                    && matches!(self.data.def_type(*b), DefType::EnumValue)
                    && self.data.def(*b).parent == *e
            }
            (_, Type::Enum(f, _, _)) => f == e,
            _ => false,
        }
    }

    /// The ENUM a variant type belongs to — `Some(Enum(E))` for a `Reference(S)` whose def
    /// is one of `E`'s variants, `None` for anything else.
    ///
    /// @FR-C-Var is the rule: `Reference(S) ⤳ Enum(E)` when `S ∈ variants(E)`, and there is
    /// no conversion between two SIBLING variants.  So wherever one variant's type would be
    /// handed to a position that another variant may also fill, this is the type to hand
    /// down instead.  The deps travel with it: which store the value borrows is not changed
    /// by naming its type more widely.
    ///
    /// `Definition::parent` makes this O(1) — a variant records its enum — so it is cheap
    /// enough to ask on every `if` that yields a record.
    fn variant_parent_enum(&self, tp: &Type) -> Option<Type> {
        let Type::Reference(d, deps) = tp else {
            return None;
        };
        let def = self.data.def(*d);
        if !matches!(self.data.def_type(*d), DefType::EnumValue) || def.parent == u32::MAX {
            return None;
        }
        Some(Type::Enum(def.parent, true, deps.clone()))
    }

    fn join_arm_into(&self, so_far: &Type, arm: &Value, tp: &Type) -> Type {
        let joined = so_far.joined_deps(&self.arm_join_type(arm, tp));
        // @FR-C-Var — when an arm joins to an ENUM the join is that enum, not the variant
        // the earlier arms happened to name.  `parse_if` decides this for its two arms;
        // every `match` arm site reaches it here, which is the one place both the settled
        // type and the new arm are in hand.  Without it the result kept the FIRST arm's
        // variant while later arms were accepted into it, so `v: Circle = match e {
        // Circle{r} => …, Square{s} => Square{…} }` was admitted and read a `Square`'s
        // bytes at `Circle`'s offsets — loft#980's class, which the `if` twin refuses.
        // The deps are the joined ones: naming the type more widely does not change which
        // store the value borrows.
        let joined = match self.variant_parent_enum(so_far) {
            Some(enum_tp) if self.joins_to_enum(&enum_tp, so_far, tp) => {
                enum_tp.with_deps_of(&joined)
            }
            _ => joined,
        };
        if crate::keys::pln25_dn1_enabled()
            && matches!(tp, Type::Optional(_))
            && !matches!(joined, Type::Optional(_))
            && !matches!(
                joined,
                Type::Void | Type::Null | Type::Never | Type::Unknown(_)
            )
        {
            return Type::optional(joined);
        }
        joined
    }

    /// loft#918 — the variable a block hands back, when its tail is nothing but a
    /// name: `… ; w_t }` and `… ; return w_t; }` both answer `w_t`.
    ///
    /// Both spellings reach the same promotion, so both must be recognised — the
    /// explicit `return` form types as `Never` rather than as the variable, which is
    /// why the caller cannot read the tail's TYPE to find it.
    fn tail_bare_var(l: &[Value]) -> Option<u16> {
        let mut node = l.last()?;
        loop {
            match node.unspan() {
                Value::Var(v) => return Some(*v),
                Value::Return(inner) => node = inner,
                _ => return None,
            }
        }
    }

    fn stmt_defines_var(op: &Value, v: u16) -> bool {
        match op.unspan() {
            Value::Set(w, _) => *w == v,
            Value::Insert(ops) => ops.iter().any(|o| Self::stmt_defines_var(o, v)),
            _ => false,
        }
    }

    fn return_views_local(&self, ls: &[u16]) -> bool {
        let attr_names = &self.data.def(self.context).attr_names;
        // A SELF-dep on a user local is the mark a self-read rebind leaves (`cur = cur.next`,
        // #328): the local views a record reached through its own field, which may be any
        // store this frame frees — a sibling local's, as in a walker over a chain built
        // here.  The walk below cannot see it, because the local is already in `seen`, so
        // it read as an owner and the view was handed up raw (loft#1337, @FR-F-Ret).  A
        // mint's self-dep (a work-ref) is the ownership marker, not a view.
        if ls.iter().any(|&v| {
            v < self.vars.count()
                && !attr_names.contains_key(self.vars.name(v))
                && !self.var_is_mint(v)
                && self.vars.tp(v).depend().contains(&v)
        }) {
            return true;
        }
        let mut work: Vec<u16> = ls.to_vec();
        let mut seen: std::collections::HashSet<u16> = work.iter().copied().collect();
        let mut i = 0;
        while i < work.len() {
            let v = work[i];
            i += 1;
            if v >= self.vars.count() {
                continue;
            }
            for d in self.vars.tp(v).depend() {
                if d < self.vars.count() && seen.insert(d) {
                    if !attr_names.contains_key(self.vars.name(d)) {
                        return true; // borrows from a non-parameter local
                    }
                    work.push(d);
                }
            }
        }
        false
    }

    /// Is `d` a MINT — a compiler-generated slot that OWNS the store it points at,
    /// minted to back one binding (`__vdb_N`, a return work-ref)?
    ///
    /// The middle of @FR-O-Proxy's three readings, and the one that makes the other two
    /// separable: a dep on a mint says *I own a store*, a dep on anything else that is
    /// not a parameter says *I borrow that one*.  Both facts are read, because neither
    /// alone answers it — `_elm_N` and `__lift_N` are compiler-generated too and are
    /// borrows (`inline_ref`, so they own nothing), while a user local that owns a store
    /// is still somebody else's owner as far as this binding is concerned.
    fn var_is_mint(&self, d: u16) -> bool {
        self.vars.is_compiler_generated(d) && self.vars.owns_store(d)
    }

    /// Does expression `e` READ OUT OF a store this function frees — an element or field
    /// of a local, or of an inline call's temporary?
    ///
    /// The same shape `return_projects_into_local` recognises at a tail, with one
    /// difference that matters at a BINDING: a projection rooted at a MINT is not a
    /// borrow.  The vector backing rewrites an owned literal into `OpGetField(__vdb_N, 0)`
    /// on pass 2, so reading the tail predicate here verbatim called `vv = [[…]]` a view
    /// of something on pass 2 and not on pass 1 — a verdict that moves between the
    /// passes, which at this site moves the ABI with it.  @FR-O-Proxy via
    /// [`var_is_mint`](Self::var_is_mint) is what keeps the two passes saying the same
    /// thing.
    fn expr_borrows_local(&self, e: &Value) -> bool {
        let (get_field, get_vector) = (
            self.data.def_nr("OpGetField"),
            self.data.def_nr("OpGetVector"),
        );
        match e.unspan() {
            Value::Call(d, args) if *d == get_field || *d == get_vector => {
                match args.first().map(Self::projection_base) {
                    Some(Value::Var(base)) => {
                        !self.vars.is_argument(*base) && !self.var_is_mint(*base)
                    }
                    Some(inner @ Value::Call(bd, _)) if *bd == get_field || *bd == get_vector => {
                        self.expr_borrows_local(inner)
                    }
                    // An inline call's result is an owned temporary (`__lift_N`), freed
                    // on the way out — the H9 case.
                    Some(Value::Call(_, _)) => true,
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// The expression `body` most recently ASSIGNS to `v`, seen through the statement
    /// wrappers a body carries.  Walked in reverse so a rebound variable answers with the
    /// assignment that survives to the tail, not the first one.
    fn var_defining_expr(body: &[Value], v: u16) -> Option<&Value> {
        fn walk(node: &Value, v: u16) -> Option<&Value> {
            match node.unspan() {
                Value::Set(w, rhs) if *w == v => Some(rhs),
                Value::Insert(ops) => ops.iter().rev().find_map(|o| walk(o, v)),
                Value::Block(bl) => bl.operators.iter().rev().find_map(|o| walk(o, v)),
                _ => None,
            }
        }
        body.iter().rev().find_map(|s| walk(s, v))
    }

    /// Is `v` DEFINED by a projection into something this function frees?  The same
    /// question `return_projects_into_local` asks of a tail, asked of the statement that
    /// produced the tail's variable.
    ///
    /// It exists because one projection shape cannot answer through `deps`.  A read out
    /// of an inline call's result (`e = mk().items; e`) borrows a `__lift_N` temp, and
    /// loft#882/#889 record the container dep at the SUBSCRIPT only — a bare field read
    /// is left to "the delivery machinery already copies out", which is true of the tail
    /// `mk().items` and false the moment the read is BOUND: the tail is then a bare
    /// `Var` with an empty dep list, so the rename fires, `e` becomes `__retbuf`, and
    /// `OpFreeRef(__lift_1)` two statements later frees what it now points at.
    /// @FR-O-Borrow — the borrow is real whether or not a dep records it.
    fn var_defined_by_projection(&self, body: &[Value], v: u16) -> bool {
        Self::var_defining_expr(body, v).is_some_and(|rhs| self.expr_borrows_local(rhs))
    }

    /// Does `v` BORROW another local's store?  The BINDING form of the view a tail
    /// spells inline — `e = vv[0]; e` beside the bare `vv[0]`, `e = t.0; e` beside `t.0`.
    ///
    /// Ask before renaming a local onto the caller's return buffer.  The rename says
    /// *this local IS the buffer*, which is an ownership claim; for a view the owner is
    /// the base local, the callee frees it at scope exit, and the caller is handed a
    /// store that has been freed and recycled.  Enforces @FR-O-Move / @FR-O-Borrow: a
    /// value that aliases another must not be transferred out as owned — the caller
    /// COPIES instead, which is what refusing the rename leaves, on `Bind`.
    ///
    /// @FR-O-Oracle is the fact this wants, and it is structurally unavailable at a
    /// PARSER site: the oracle classifies a finished body from `data.def(d_nr).code`
    /// and the parser has no def handle (`formal/ownership.md` measured this for
    /// `vector_needs_db`).  So it reads the sharpened @FR-O-Proxy fact the same
    /// register writes down — a dep list carries THREE meanings, not two: EMPTY (no
    /// store yet), a dep on the binding's OWN mint (`__vdb_N`, which says *I own a
    /// store*), and a dep on ANOTHER LOCAL (*I borrow that one*).  Only the third is a
    /// borrow, so bare non-emptiness is the wrong reading.
    ///
    /// Skipping the mint is what makes the verdict agree on BOTH PARSER PASSES, and
    /// that is a correctness requirement rather than a refinement.  `vector_db` adds
    /// the mint dep on pass 2 only, while a borrow dep comes from the projection and
    /// is present on both: an owning `o` reads `[]` then `["__vdb_1"]`, a viewing `e`
    /// reads `["vv"]` on both.  Bare non-emptiness would therefore answer *owns* on
    /// pass 1 and *borrows* on pass 2 for one body — and this verdict decides whether
    /// the function takes a hidden buffer argument, so the ABI would move between the
    /// passes (the shape loft#1099 cost).  `var_bound_to_branch` states the same
    /// two-pass obligation and answers it structurally, for the same reason.
    ///
    /// An ARGUMENT dep is deliberately not a borrow here: the CALLER owns that store,
    /// so it outlives the call, and `classify_vector_delivery`'s `CopyBorrow` leg
    /// already gives that shape value semantics.
    fn var_views_local(&self, v: u16) -> bool {
        let mut work: Vec<u16> = vec![v];
        let mut seen: std::collections::HashSet<u16> = work.iter().copied().collect();
        let mut i = 0;
        while i < work.len() {
            let cur = work[i];
            i += 1;
            if cur >= self.vars.count() {
                continue;
            }
            for d in self.vars.tp(cur).depend() {
                if d >= self.vars.count() || !seen.insert(d) {
                    continue;
                }
                if self.vars.is_argument(d) {
                    continue;
                }
                if self.var_is_mint(d) {
                    // Its own mint: owns a store and borrows nothing.  Walk through it
                    // rather than stopping, so a mint that itself views a local counts.
                    work.push(d);
                    continue;
                }
                return true;
            }
        }
        false
    }

    /// #306: rewrite a return value that views a local's store into an owned
    /// copy — `{ __ref_N = null; OpDatabase(__ref_N, kt);
    /// OpCopyRecord(<orig>, __ref_N, kt); __ref_N }`.  The returned work-ref
    /// is then NRVO-promoted by `ref_return`, so the copy lands directly in
    /// the caller-provided buffer (no extra allocation in the adopt case).
    /// Returns the work-ref var so the caller passes `[w]` to `ref_return`.
    fn materialize_view_return(&mut self, td: u32, tail: &mut Value) -> u16 {
        let ref_tp = Type::Reference(td, Deps::none());
        let w = self.vars.work_refs(&ref_tp, &mut self.lexer);
        if self.return_buffer().is_none() {
            // A buffer-less return (`-> S?`) is delivered as the DbRef the tail yields, so
            // the copy is made only on the arms that VIEW something this frame frees: a
            // `null` arm stays null and an owned arm is handed up as it is (loft#1337).
            self.materialize_view_arms(td, tail, w);
        } else {
            self.materialize_return_into(td, tail, w);
        }
        w
    }

    /// The per-arm form of [`Self::materialize_return_into`]: walk the tail through its
    /// `if` arms and blocks, and rewrite exactly the leaves that view a store this frame
    /// frees — a projection rooted at a local, or a local whose deps view one (a self-dep
    /// included).  Every rewritten arm lands in the ONE work-ref `w`; only one arm runs, and
    /// an arm that was not rewritten never allocates it.
    fn materialize_view_arms(&mut self, td: u32, tail: &mut Value, w: u16) {
        match tail {
            Value::Return(inner) | Value::Drop(inner) => self.materialize_view_arms(td, inner, w),
            Value::Span(b) => self.materialize_view_arms(td, &mut b.1, w),
            Value::If(_, t, f) => {
                self.materialize_view_arms(td, t, w);
                self.materialize_view_arms(td, f, w);
            }
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last_mut() {
                    self.materialize_view_arms(td, last, w);
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last_mut() {
                    self.materialize_view_arms(td, last, w);
                }
            }
            leaf => {
                if !self.return_leaf_is_owned_or_null(leaf) {
                    self.materialize_return_into(td, leaf, w);
                }
            }
        }
    }

    /// Is this return leaf something a `-> S?` return may hand up AS IT IS — a `null`, an
    /// owned local or parameter, a struct literal, or a call that mints its own store?
    ///
    /// The leaf rule of [`Self::materialize_view_arms`], stated in the direction @FR-F-Ret
    /// makes safe: everything NOT provably owned is copied.  A projection of any kind, a keyed
    /// lookup, a lifted call temporary's element, a joined value — none of these can be shown
    /// to own the store they yield, so each is copied before it escapes.  Stating the rule
    /// the other way — copy what LOOKS like a view — is what let a keyed element of an
    /// inline call's temporary slip through as an owner (the `882` poison cells).
    fn return_leaf_is_owned_or_null(&self, leaf: &Value) -> bool {
        match leaf.unspan() {
            Value::Null => true,
            Value::Var(v) => {
                *v >= self.vars.count()
                    || self.vars.is_argument(*v)
                    || !self.return_views_local(&[*v])
            }
            Value::Call(d, args) => {
                let name = self.data.def(*d).name();
                if name == "OpNullRefSentinel" && args.is_empty() {
                    return true;
                }
                // A loft-defined callee that mints its own store hands it up; one whose
                // return is tied to a passed buffer or argument does not.
                self.data.def(*d).is_loft_defined() && self.data.def(*d).return_adopts_fresh_store()
            }
            // A struct literal builds into a work-ref of this frame, which the return
            // transfers (`collect_return_sources` names it); a block that ends in anything
            // else is judged by its tail.
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|t| self.return_leaf_is_owned_or_null(t)),
            _ => false,
        }
    }

    /// `materialize_view_return` for a block used as a VALUE — the same owned-copy
    /// rewrite, but drawing its work-ref from the PASS-2-ONLY `__ref_p2_N` sequence
    /// (`Vars::work_ref_p2`), because the move-on-block-return site that calls it is
    /// guarded by `!first_pass`.  Drawing from the shared counter handed this site
    /// the name pass 1 left on the RETURN BUFFER, so the block materialised its value
    /// into the buffer the real return re-mints — and the return copied from the
    /// destroyed store, answering `null` (loft#848).  A block VALUE is not a return
    /// and has no claim on the return buffer.
    fn materialize_view_value(&mut self, td: u32, tail: &mut Value) -> u16 {
        let ref_tp = Type::Reference(td, Deps::none());
        let w = self.vars.work_refs_p2(&ref_tp, &mut self.lexer);
        self.materialize_return_into(td, tail, w);
        w
    }

    /// Rewrite a return-site value so it lands in `w` (an existing DbRef
    /// var) as an owned copy: `{ w = null; OpDatabase(w, kt);
    /// OpCopyRecord(<orig>, w, kt); w }`.  Used with a fresh work-ref by
    /// `materialize_view_return` (#306 views) and with the fn's ONE return
    /// buffer by `ref_return`'s copy leg (a named local another return
    /// site can read must not alias the buffer, so it is copied at the
    /// return instead).
    fn materialize_return_into(&mut self, td: u32, tail: &mut Value, w: u16) {
        if let Value::Return(inner) = tail {
            return self.materialize_return_into(td, inner, w);
        }
        let kt = self.data.def(td).known_type();
        let copy_d = self.data.def_nr("OpCopyRecord");
        let orig = std::mem::replace(tail, Value::Null);
        // A NULLABLE local as the source (`cur: Node?` after a walk) may hold nothing, and
        // `OpCopyRecord` of a null source leaves the destination an allocated EMPTY record —
        // presence standing in for absence.  Copy only where the source is present; the
        // absent arm hands up the sentinel (loft#1337).
        let absent_guard = match orig.unspan() {
            Value::Var(v) if matches!(self.vars.tp(*v), Type::Optional(_)) => Some(*v),
            _ => None,
        };
        let copy = crate::data::v_block(
            vec![
                crate::data::v_set(w, Value::Null),
                self.cl("OpDatabase", &[Value::Var(w), Value::Int(i32::from(kt))]),
                Value::Call(copy_d, vec![orig, Value::Var(w), Value::Int(i32::from(kt))]),
                Value::Var(w),
            ],
            Type::Reference(td, Deps::frame1(w)),
            "materialized_view_return",
        );
        *tail = match absent_guard {
            Some(v) => crate::data::v_if(
                self.cl("OpRefIsNull", &[Value::Var(v)]),
                self.cl("OpNullRefSentinel", &[]),
                copy,
            ),
            None => copy,
        };
        // @PLN130 — parser-emitted materialisation: `return f.field` must publish an OWNED
        // record, not a view into a frame-local. See `ParserMaterialise`.
        crate::copy_manifest::record(
            self.context,
            w,
            kt,
            crate::copy_manifest::Origin::ParserMaterialise,
        );
    }

    /// The work-ref that carries a return site's VALUE: for a tail call,
    /// the `Var` in the callee's hidden heap-buffer argument slot; for a
    /// plain `Var` tail, the var itself.  Only this ref may bind to the
    /// fn's ONE return buffer — an INNER call's ref (`return wrap(mk(x))`
    /// has two) must stay a plain local, or the outer call's destination
    /// would alias its own argument (the callee's buffer clear then frees
    /// the record the argument still views).
    /// @PLN25 single-payload — is the return body-tail the `__nullable<S>` → dense `S`
    /// unwrap (now a payload sub-ref `OpGetField`, see `unwrap_source_is_nullable`)?  Such
    /// a tail's dense type doesn't match the still-`Enum` tail type `t`, so the type-keyed
    /// `ref_return` branches miss it and the default epilogue demotes it to `return null`.
    /// When this holds, `materialize_view_return` copies the viewed `S` into an owned buffer
    /// and promotes that — the #306 view-return shape.  Gate-off-inert.
    fn tail_is_nullable_unwrap(&self, tail: &Value) -> bool {
        match tail.unspan() {
            Value::Return(inner) => self.tail_is_nullable_unwrap(inner),
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|t| self.tail_is_nullable_unwrap(t)),
            // Single-payload: the `__nullable<S>` → dense `S` unwrap is now a payload
            // SUB-REF `OpGetField(<__nullable<S> value>, payload_offset, S_kt)` (the convert
            // emits it via `get_val`).  Materialise it — copy the viewed `S` into the return
            // buffer so the result is OWNED, not a dangling view into the caller's container —
            // ONLY when the unwrap source is a LOCAL (`Var`, e.g. `return chosen`) or a
            // materialised sub-expression (`Block`/`If`, e.g. `return v[i] ?? d`'s ncc block).
            // A direct `v[i]` index source is the sole returnable that the default epilogue
            // returns correctly; materialising it would NRVO-rename the work-ref onto the
            // caller's buffer and re-`OpDatabase` it → free-list corruption.  The
            // source-is-`__nullable<S>` check distinguishes the unwrap from an ordinary
            // struct-field read (whose source is a dense struct, not the synth enum).
            Value::Call(d, args) => {
                self.data.def(*d).name() == "OpGetField"
                    && args
                        .first()
                        .is_some_and(|s| self.unwrap_source_is_nullable(s))
            }
            _ => false,
        }
    }

    /// Is `src` a `__nullable<S>` value (the source of a payload-unwrap `OpGetField`)?
    /// Only a LOCAL (`Var`), a materialised `Block`/`If` tail qualifies — a direct
    /// index/call source returns `false` so its unwrap is NOT materialised (see
    /// `tail_is_nullable_unwrap`).  Gate-off-inert (no `__nullable<>` type exists).
    fn unwrap_source_is_nullable(&self, src: &Value) -> bool {
        let tp = match src.unspan() {
            Value::Var(v) => self.vars.tp(*v).clone(),
            Value::Block(bl) => bl.result.clone(),
            Value::If(_, t, _) => return self.unwrap_source_is_nullable(t),
            _ => return false,
        };
        matches!(&tp, Type::Enum(d, _, _) | Type::Reference(d, _)
            if self.data.def(*d).name().starts_with("__nullable<"))
    }

    /// #416 — does the tail's OUTERMOST `if`/`match` have a DIRECT `null` arm
    /// (`{ if b { [..] } else { null } }`)? Such a return is nullable, and the
    /// per-arm `__retbuf` materialise must not fire for it (it would force an
    /// owned-buffer return type onto a path that yields null — the native
    /// nullable-vector miscompile). An exhaustive `match`'s default-null is NESTED
    /// (the inner-most else after the variant tests), so it is not a direct arm and
    /// such a match still materialises. Only the outermost branch's arms are
    /// inspected — descending would also catch the unreachable match default.
    fn tail_if_has_null_arm(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Return(i) | Value::Drop(i) => self.tail_if_has_null_arm(i),
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|x| self.tail_if_has_null_arm(x)),
            Value::Insert(ops) => ops.last().is_some_and(|x| self.tail_if_has_null_arm(x)),
            Value::If(_, t, f) => self.arm_is_null(t) || self.arm_is_null(f),
            _ => false,
        }
    }

    /// How many LEAF arms of this branch tail deliver a value rather than the null
    /// sentinel?  The promotion can rename exactly ONE local onto the caller's return
    /// buffer, so a tail with two or more of these has arms that must MATERIALISE into
    /// the buffer instead — the alternative is a store the callee returns and neither
    /// side frees (loft#1098).  Counts through nested `If`s, which is how a `match`
    /// lowers, so an else-if chain and a `match` answer alike.
    fn tail_nonnull_arm_count(&self, v: &Value) -> usize {
        if self.arm_is_null(v) {
            return 0;
        }
        match v.unspan() {
            Value::Return(i) | Value::Drop(i) => self.tail_nonnull_arm_count(i),
            Value::Block(bl) => bl
                .operators
                .last()
                .map_or(0, |x| self.tail_nonnull_arm_count(x)),
            Value::Insert(ops) => ops.last().map_or(0, |x| self.tail_nonnull_arm_count(x)),
            Value::If(_, t, f) => self.tail_nonnull_arm_count(t) + self.tail_nonnull_arm_count(f),
            _ => 1,
        }
    }

    /// Does this branch arm reduce to a `null` value (descending through the arm's
    /// block/insert tail)? A `null` vector arm lowers to `{ OpNullRefSentinel() }`,
    /// not a bare `Value::Null`, so both forms count. A nested `if` arm is NOT null
    /// — that's how enc's nested match-default is distinguished from maybe's direct
    /// `else null`.
    ///
    /// A `Return`/`Drop` wrapper is deliberately NOT descended: the question is what this
    /// value hands to the JOIN, and a `return` hands it nothing — it leaves the function.
    /// The `scopes`-side siblings (`is_null_terminal`, `return_has_null_arm`) ask the same
    /// null question about a return EXPRESSION, where that wrapper is the subject rather
    /// than an escape, and pass through it.
    fn arm_is_null(&self, v: &Value) -> bool {
        match v.unspan() {
            Value::Null => true,
            Value::Call(d, _) => *d == self.data.def_nr("OpNullRefSentinel"),
            Value::Block(bl) => bl.operators.last().is_some_and(|x| self.arm_is_null(x)),
            Value::Insert(ops) => ops.last().is_some_and(|x| self.arm_is_null(x)),
            _ => false,
        }
    }

    /// @PLN85 — rewrite each `null` arm of a slice-match tail to a FRESH empty vector
    /// (`{ OpDatabase(o); o }`), so a `[]` arm bound to a local (whose copy-on-bind
    /// appends the whole match value) is a real `DbRef`, not a bare `null` native emits
    /// as `()`.  Descends `if`/block/insert; a non-null arm is left untouched.
    fn materialize_null_slice_arms(&mut self, tail: &mut Value, elm_tp: &Type) {
        match tail {
            Value::Span(b) => self.materialize_null_slice_arms(&mut b.1, elm_tp),
            Value::Return(inner) | Value::Drop(inner) => {
                self.materialize_null_slice_arms(inner, elm_tp);
            }
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last_mut() {
                    self.materialize_null_slice_arms(last, elm_tp);
                }
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last_mut() {
                    self.materialize_null_slice_arms(last, elm_tp);
                }
            }
            Value::If(_, t, f) => {
                self.rewrite_null_arm_fresh(t, elm_tp);
                self.rewrite_null_arm_fresh(f, elm_tp);
            }
            _ => {}
        }
    }

    fn rewrite_null_arm_fresh(&mut self, arm: &mut Value, elm_tp: &Type) {
        if self.arm_is_null(arm) {
            let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
            let o = self.create_unique("empty_arm", &vec_tp);
            if o != u16::MAX {
                self.vars.defined(o);
                let mut ops = self.vector_db(&vec_tp, o);
                ops.push(Value::Var(o));
                *arm = Value::Insert(ops);
            }
        } else {
            self.materialize_null_slice_arms(arm, elm_tp);
        }
    }

    /// #425 — if the return tail is a projection of a container held by a variable
    /// (a struct/enum FIELD `OpGetField(Var(base), …)`, or a TUPLE element
    /// `TupleGet(base, i)`, possibly wrapped in `Return`/`Block`), return the base var
    /// being projected. Used to decide whether the projected value's record is locally
    /// owned (and freed at scope exit) or caller-owned (a parameter).
    ///
    /// A tuple element is the same question in a different spelling: the read is a
    /// `Value` VARIANT rather than an op call, so it carries its base as a var NUMBER
    /// and never appears as `Call(OpGetField, [Var(base), …])`. Both spellings must
    /// answer, because the caller acts on the projection, not on how it is written.
    fn return_field_base_var(&self, tail: &Value) -> Option<u16> {
        match tail.unspan() {
            Value::Return(inner) | Value::Drop(inner) => self.return_field_base_var(inner),
            Value::TupleGet(base, _) => Some(*base),
            Value::Block(bl) => bl
                .operators
                .last()
                .and_then(|t| self.return_field_base_var(t)),
            Value::Insert(ops) => ops.last().and_then(|t| self.return_field_base_var(t)),
            Value::Call(d, args) if *d == self.data.def_nr("OpGetField") => {
                match args.first().map(Value::unspan) {
                    Some(Value::Var(b)) => Some(*b),
                    // @PLN85 (the chained field-of-local face) — a CHAINED
                    // projection (`return t.inner.value`) roots at the inner
                    // chain's base var (the #488/#489 recursion pattern).
                    // Single-hop-only here let the NRVO rename take `t` as
                    // the promotion candidate: the buffer was renamed onto
                    // the container local, the tail was DISCARDED, and
                    // native returned the typed null (interp survived via
                    // the eval-stack channel — a silent backend divergence
                    // LOFT_POISON made loud).
                    Some(inner @ Value::Call(bd, _)) if *bd == self.data.def_nr("OpGetField") => {
                        self.return_field_base_var(inner)
                    }
                    // An inline-ref capture block in base position
                    // (`mk().inner` captured to an owned `w` copy) roots at
                    // the block's tail var.
                    Some(inner @ (Value::Block(_) | Value::Insert(_))) => {
                        self.return_field_base_var(inner).or_else(|| {
                            if let Value::Block(bl) = inner
                                && let Some(Value::Var(w)) = bl.operators.last().map(Value::unspan)
                            {
                                Some(*w)
                            } else {
                                None
                            }
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// #425 sibling — is the return tail a heap-field projection of an INLINE
    /// CALL result (`return mk().value`), as opposed to a named local
    /// (`return d.value`)?  The base `mk()` is an owned temporary that scope
    /// analysis lifts to a `__lift_N` local and frees at scope exit; the
    /// projected sub-ref is a VIEW into it, so returning the projection as-is
    /// makes the buffer dangle (native re-reads the freed store → null
    /// sentinel; the `bound_field` workaround `t = mk(); return t.value`
    /// dodges it because the named local reaches `ref_return`'s copy leg).
    /// Returns `true` only for a DIRECT `OpGetField(Call(fn,…), …)` whose base
    /// is a plain function call — NOT a chained projection (`mk().a.b`, whose
    /// intermediate `Reference` is already materialised into a work-ref) and
    /// NOT a `Var`/parameter base (handled by `return_field_base_var`).  The
    /// caller copies the projected field into `__retbuf` via
    /// `materialize_view_return` so the field's record survives the lift's
    /// free — the same owned-copy `return d.value` already performs.
    /// @PLN85 2d — the tail delivers a fresh owned text from a NATIVE text-dest CALL,
    /// either as a bare `<call>` or an explicit `return <call>`.  `parse_block` binds
    /// it to a synthetic local so it promotes to a hidden `&text` caller buffer (the
    /// clean rebind shape).  Same predicate as `wrap_value_text_dest`; a forwarded
    /// USER fn is already promoted (excluded).  Peels `Span`/`Return`.
    fn native_text_call_tail(&self, tail: &Value) -> bool {
        match tail.unspan() {
            Value::Call(op, _) => {
                let def = self.data.def(*op);
                crate::state::codegen::is_text_dest_native(def.name())
                    || crate::state::codegen::is_cdylib_text_call(def)
            }
            Value::Return(inner) => self.native_text_call_tail(inner),
            _ => false,
        }
    }

    /// @PLN85 unified fix — the tail is a value-yielding `if`/`match` text
    /// return (match lowers to nested `If`): an `If` (peeling `Return`) whose
    /// BOTH arms yield a text value.  Drives the per-arm accumulator
    /// materialisation (`push_text_arms_into`).
    pub(super) fn if_tail_yields_text(tail: &Value) -> bool {
        match tail.unspan() {
            Value::Return(inner) | Value::Drop(inner) => Self::if_tail_yields_text(inner),
            Value::If(_, then, els) => Self::arm_yields_text(then) && Self::arm_yields_text(els),
            // @PLN85 corpus — a `??` coalesce (`build_null_coalesce_default`)
            // wraps its selecting `if` in an "ncc" block behind a `Set(__ncc, lhs)`
            // preamble; `?? return` builds the "ncr" twin.  A coalesce whose arms
            // yield text (`v[i] ?? "fb"`) is the SAME promotable value-yielding
            // if-text tail — see through the coalesce block so it takes the per-arm
            // `__acc` promotion (delivered through the caller `&text` buffer),
            // instead of falling to `Plain` and orphaning the owned `__ncc` /
            // result copy that a non-buffered return never frees on the interpreter.
            Value::Block(bl) if bl.name == "ncc" || bl.name == "ncr" => {
                bl.operators.last().is_some_and(Self::if_tail_yields_text)
            }
            // A `match` in tail position lowers to `Block["scalar_match"]{ Set(subj),
            // <nested If chain> }` — structurally the SAME value-yielding branch tail as
            // a bare `if`, just behind the subject-binding wrapper.  Without seeing
            // through it, `do_if_acc` never fires, the arms are not pushed into `__acc`,
            // and `text_return` promotes nothing: native then emits the arm's borrow of a
            // dead `Str` temporary (E0716) or drops the value and returns `()` (E0599),
            // while the interpreter — which keeps the value on its stack — is correct.
            // That accept/reject divergence is what the differential oracle caught in
            // `tests/oracle/27-native-tailcall-return-heap.loft`.  `push_text_arms_into`
            // already descends `Value::Block` and retypes it `Void`, so only the
            // RECOGNISER was blind.  Exactly the `ncc`/`ncr` see-through above.
            //
            // Forward-ref-SAFE for the same reason the `if` arm is: `parse_scalar_match`
            // emits this block on BOTH passes.  A `null` arm still yields `false` via
            // `arm_yields_text`, so a `text?` tail keeps its `(N-Store)` reject.
            Value::Block(bl) if bl.name == "scalar_match" => {
                bl.operators.last().is_some_and(Self::if_tail_yields_text)
            }
            // A `match v { [slice] => … }` in tail position lowers to
            // `Block["vector_match"]{ Set(subj), <arg init>, return <nested If> }` —
            // the same value-yielding branch tail as a scalar match, behind the
            // slice-binding + rest-materialise scaffolding. Without seeing through it,
            // `do_if_acc` never fires, so a text arm that VIEWS a captured repetition
            // group is promoted to a raw `&text` alias of the group's backing store
            // and freed under it — the captured-group element-access use-after-free
            // (plans/captured-group-elem-uaf.md). With it, each arm's text is COPIED
            // into the `__acc` buffer (owned) before the store is freed — the same
            // proven per-arm delivery the scalar match already takes. Forward-ref-safe
            // (the block is emitted on both passes); a `return`/guard arm still yields
            // `false` via `arm_yields_text`, so a non-text arm keeps its reject.
            Value::Block(bl) if bl.name == "vector_match" => {
                bl.operators.last().is_some_and(Self::if_tail_yields_text)
            }
            _ => false,
        }
    }

    /// True when an `if`/`match` ARM YIELDS a text value (a literal, call, var,
    /// view, or a nested `if` whose sub-arms all yield) — NOT a diverging
    /// `return`/`break`/`continue`, a `null`/empty arm, or an empty block.  A
    /// promotable value-yielding tail needs BOTH arms to yield; a guard `if c {
    /// return … }` does not, and must stay unbound so the missing-return
    /// diagnostic still fires.
    fn arm_yields_text(arm: &Value) -> bool {
        match arm.unspan() {
            Value::Return(_) | Value::Break(_) | Value::Continue(_) => false,
            Value::Null => false,
            Value::Block(bl) => bl.operators.last().is_some_and(Self::arm_yields_text),
            Value::Insert(ops) => ops.last().is_some_and(Self::arm_yields_text),
            Value::If(_, then, els) => Self::arm_yields_text(then) && Self::arm_yields_text(els),
            _ => true,
        }
    }

    /// Rewrite each ARM leaf of an `if`/`match` text tail to deliver into the
    /// text accumulator `av` — `<leaf>` becomes `Set(av, <leaf>)` (which lowers
    /// to a per-arm clear+append into `av`'s buffer).  Recurses `If`/`Block`/
    /// `Insert`/`Span`; the leaf is the arm's terminal value.  Each arm then
    /// writes `av` independently (uniform Rust type on native — no if-expression
    /// to unify) and `av` becomes the single owned text the caller buffer
    /// promotion delivers copy-free.
    /// Bind-site analogue of `do_if_acc`'s tail promotion (the @P323 sibling, 2026-07-10).
    ///
    /// `q = <branch producing text>` must deliver PER ARM into `q`; it must never lower to
    /// `Set(q, <branch>)`.  Lowered as an expression, each arm emits `&*(callee(…))` — a
    /// borrow of the `Str` temporary the callee returned — and that temporary dies at the
    /// arm's `}`, so the consumer reads a dangling borrow.  Native rejects with **E0716**
    /// (`match` on a scalar subject) or **E0308** when the arms' Rust reps disagree (`if`,
    /// `??`); the interpreter, which keeps the value on its stack, is correct in all of
    /// them — an accept/reject divergence.
    ///
    /// A `match` on a TEXT subject only *appeared* to work: freeing the subject copy emits
    /// an `OpFreeText` after the value, which incidentally trips `has_trailing_void` in
    /// `generation::emit`, which materialises the block.  The subject's type has nothing to
    /// do with the arm temporary's lifetime — that is what makes it a proxy, not the fact.
    ///
    /// Rewriting each arm leaf to `Set(q, leaf)` yields exactly the shape the `if` TAIL
    /// already emits (`*var_q = …;` per arm), which native compiles.  Returns `true` when
    /// it rewrote `code`, which is then a VOID branch of `Set`s.
    ///
    /// Declines when the RHS reads `q` (the P223 clear-before-read wrap owns that shape),
    /// when any arm yields `null` / `return` (`arm_yields_text`, so a `text?` bind keeps
    /// its `(N-Store)` reject), and on pass 1 — the shape must be stable across passes.
    ///
    /// The leading `Set(q, "")` is load-bearing, not defensive.  The per-arm `Set`s live
    /// INSIDE the branch, so without it this statement never introduces `q`: the
    /// interpreter silently read an empty text (a wrong ANSWER, worse than the reject it
    /// replaced) and native emitted an undeclared `var_q` (E0425).  The init defines the
    /// destination exactly where the old `Set(q, <branch>)` did.
    pub(super) fn try_branch_text_bind(&mut self, code: &mut Value, var_nr: u16) -> bool {
        if self.first_pass
            || var_nr == u16::MAX
            || code.reads_var(var_nr)
            || !Self::if_tail_yields_text(code)
        {
            return false;
        }
        Self::push_text_arms_into(code, var_nr, self.data.def_nr("OpCreateStack"));
        let branch = std::mem::replace(code, Value::Null);
        *code = Value::Insert(vec![v_set(var_nr, Value::Text(String::new())), branch]);
        true
    }

    /// `create_stack` is `OpCreateStack`'s def_nr, threaded in rather than looked
    /// up per leaf — see the leaf arm for what it decides.
    pub(crate) fn push_text_arms_into(op: &mut Value, av: u16, create_stack: u32) {
        match op {
            Value::Span(b) => Self::push_text_arms_into(&mut b.1, av, create_stack),
            Value::Return(inner) | Value::Drop(inner) => {
                Self::push_text_arms_into(inner, av, create_stack);
            }
            Value::If(_, then, els) => {
                Self::push_text_arms_into(then, av, create_stack);
                Self::push_text_arms_into(els, av, create_stack);
            }
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last_mut() {
                    Self::push_text_arms_into(last, av, create_stack);
                }
                // The arm now ends in a `Set(av, …)` (or a void nested `If`), so
                // the block yields VOID, not text — retype it, else native emits
                // a text trailing value that mismatches the sibling arm's `()`
                // (`if`/`else` incompatible types).
                bl.result = Type::Void;
            }
            Value::Insert(ops) => {
                if let Some(last) = ops.last_mut() {
                    Self::push_text_arms_into(last, av, create_stack);
                }
            }
            leaf => {
                let mut v = std::mem::replace(leaf, Value::Null);
                // A `text_ref` arm ends in `OpCreateStack(buf)` — a BORROW of the
                // work buffer, not a value, and the enclosing scope frees that
                // buffer before the accumulator is read. Deliver the BUFFER
                // instead, so the accumulator copies the bytes it is about to own.
                //
                // This is the shape `return x ?? "fallback"` takes when the
                // fallback is not a bare variable: the `??` lowers to an `if`
                // whose else arm is a text_ref block, and `av` then held a
                // dangling reference. The interpreter answered `""` — a wrong
                // value, silently — and native emitted `*var_acc = ().to_string()`,
                // which is not Rust. Binding the result to a local first
                // (`x = a ?? b; return x`) avoided it, which is what made the
                // failure look like it was about `??` rather than about delivery.
                if let Value::Call(d, args) = &v
                    && *d == create_stack
                    && args.len() == 1
                {
                    v = args[0].clone();
                }
                *leaf = crate::data::v_set(av, v);
            }
        }
    }

    // ── @PLN85 text-return analysis framework (SHADOW) ────────────────────
    // The single selector that replaces the stacked per-shape predicates.
    // Pure + read-only: it classifies a text return TAIL into `TextReturn`.
    // Verified beside the tests via `LOFT_TRA_DUMP`; not yet wired to codegen.

    /// Gate the `__tret` bind: the verdict must want it, AND — for the
    /// forward-reference-UNSTABLE `UserCall` verdict — the callee must be a
    /// BACKWARD reference (its def_nr precedes this fn's).  A later-defined callee
    /// classifies `Plain` on pass 1 (return type unresolved there) and `UserCall`
    /// on pass 2, so promoting it pass-2-only diverges the ABI and crashes a
    /// forward-ref caller compiled earlier in pass 2 (`page_landing` class); a
    /// backward ref is `UserCall` on both passes → pass-stable.  Native / view /
    /// built-local carry their own pass behavior and are not gated here.
    /// True when the current def already carries a `__tret` return-buffer
    /// attribute — i.e. `do_tret_bind` promoted it on a PRIOR pass.  Used to gate
    /// pass-2 promotion so it follows pass 1 (the pass-stability contract that
    /// `do_tret_bind`'s signature growth requires; see its call site).  The temp
    /// is minted as `___tret_<n>` (`vars::unique` prefixes `_` and suffixes the
    /// dedup counter onto `__tret`), and a def carries at most one.
    fn def_has_tret_attr(&self) -> bool {
        self.data
            .def(self.context)
            .attr_names
            .keys()
            .any(|k| k.starts_with("___tret"))
    }

    /// The `__acc` twin of [`Self::def_has_tret_attr`] — did PASS 1 promote this function's
    /// text accumulator to a hidden `&text` parameter?  It is the whole pass-2 verdict
    /// (loft#1099): the accumulator changes the signature, so pass 2 must not reach a
    /// different answer than the one pass 1 already published to every caller.
    fn def_has_acc_attr(&self) -> bool {
        self.data
            .def(self.context)
            .attr_names
            .keys()
            .any(|k| k.starts_with("___acc"))
    }

    fn tret_bind_ok(&self, tail: &Value, block: &[Value]) -> bool {
        let verdict = self.classify_text_return(tail, block);
        if !verdict.wants_tret_bind() {
            return false;
        }
        if matches!(
            verdict,
            TextReturn::Owned(OwnedVia::UserCall | OwnedVia::ViewOfLocalCall)
        ) {
            // Both depend on the callee's resolved return signature, so they are
            // forward-ref-UNSTABLE — promote ONLY for a backward-ref callee
            // (def_nr precedes this fn's), pass-stable on both passes.
            return Self::tail_call_op(tail)
                .is_some_and(|op| self.backward_ref_defnr(op) < self.context);
        }
        true
    }

    /// The def_nr that governs the backward-ref gate for a call tail.  Normally the
    /// callee itself — but a GENERIC MONOMORPH is minted at its call site (pass 2),
    /// so its OWN def_nr reads forward even when the generic is defined textually
    /// BEFORE the caller.  Map such a callee (`t_<Type>_<fn>`) back to its TEMPLATE
    /// (`n_<fn>`, `DefType::Generic`), which is where the callee is really defined and
    /// is pass-stable: pass 1 resolves the call to the template, pass 2 to the
    /// monomorph, and both then compare the SAME template def_nr.  @PLN85 forward-ref
    /// class (g1b — a `-> text` monomorph returned through a non-generic caller).
    fn backward_ref_defnr(&self, op: u32) -> u32 {
        if (op as usize) < self.data.definitions.len() {
            let def = self.data.def(op);
            if def.def_type() == DefType::Function && def.name().starts_with("t_") {
                let tmpl = self.data.def_nr(&format!("n_{}", def.original_name()));
                if tmpl != u32::MAX && self.data.def_type(tmpl) == DefType::Generic {
                    return tmpl;
                }
            }
        }
        op
    }

    /// The callee def_nr of a CALL return tail (peeling `Block`/`Insert`/
    /// `Return`/`Drop` wrappers), or `None` when the tail is not a direct call.
    fn tail_call_op(tail: &Value) -> Option<u32> {
        match tail.unspan() {
            Value::Block(bl) => bl.operators.last().and_then(Self::tail_call_op),
            Value::Insert(ops) => ops.last().and_then(Self::tail_call_op),
            Value::Return(inner) | Value::Drop(inner) => Self::tail_call_op(inner),
            Value::Call(op, _) => Some(*op),
            _ => None,
        }
    }

    /// Classify a text (or `text?`) return TAIL — see `TextReturn`.  `block`
    /// is the operator list the tail lives in (its siblings), needed to tell a
    /// LOCAL composite (constructed here) from a caller-owned argument.
    pub(crate) fn classify_text_return(&self, tail: &Value, block: &[Value]) -> TextReturn {
        match tail.unspan() {
            Value::Return(inner) | Value::Drop(inner) => self.classify_text_return(inner, block),
            Value::Block(bl) => bl.operators.last().map_or(TextReturn::Plain, |t| {
                self.classify_text_return(t, &bl.operators)
            }),
            Value::Insert(ops) => ops
                .last()
                .map_or(TextReturn::Plain, |t| self.classify_text_return(t, ops)),
            // A var tail.  A `RefVar(Text)` var is a promoted `&text` caller
            // buffer — a built-up text (accumulator / interpolation / concat /
            // rebind) `text_return` ALREADY promoted, so owned.  A plain-text
            // caller ARGUMENT returned directly is a borrow.  Any other local is
            // a built-up text — owned.
            Value::Var(v) => {
                if matches!(self.vars.tp(*v), Type::RefVar(_)) {
                    TextReturn::Owned(OwnedVia::BuiltLocal)
                } else if self.vars.is_argument(*v) {
                    TextReturn::Borrow(BorrowVia::Argument)
                } else {
                    TextReturn::Owned(OwnedVia::BuiltLocal)
                }
            }
            Value::Call(op, _) => self.classify_text_call(*op, tail, block),
            // A tuple-element view (`r.0` — `Value::TupleGet`): owned iff the
            // tuple LOCAL was built here (freed at scope exit → its text is
            // copied out), a borrow iff it is a caller-owned argument.  Same
            // rule as the `OpGetText` field view (3a).
            Value::TupleGet(root, _) => {
                if self.var_built_in_block(block, *root) {
                    TextReturn::Owned(OwnedVia::ViewOfLocal)
                } else {
                    TextReturn::Borrow(BorrowVia::Argument)
                }
            }
            // A fn-REF call (`f(42)`, `g.fmt(42)`) returning text delivers a
            // fresh owned text (p227) — promotable, with the adaptive fn-ref ABI.
            Value::CallRef(_, _) => TextReturn::Owned(OwnedVia::FnRefCall),
            Value::If(_, then, els) => {
                if self.arm_delivers_owned(then, block) || self.arm_delivers_owned(els, block) {
                    TextReturn::Owned(OwnedVia::IfMatchArm)
                } else {
                    TextReturn::Plain
                }
            }
            // A TUPLE-constructor return: owned iff at least one text element
            // delivers owned text (the others are literals / arg-borrows).
            Value::Tuple(elems) => {
                if elems
                    .iter()
                    .any(|e| matches!(self.classify_text_return(e, block), TextReturn::Owned(_)))
                {
                    TextReturn::Owned(OwnedVia::TupleElement)
                } else {
                    TextReturn::Plain
                }
            }
            _ => TextReturn::Plain,
        }
    }

    /// Classify a CALL tail (`classify_text_return`'s call arm).
    fn classify_text_call(&self, op: u32, tail: &Value, block: &[Value]) -> TextReturn {
        // Native text-dest call (2d).
        if self.native_text_call_tail(tail) {
            return TextReturn::Owned(OwnedVia::NativeCall);
        }
        let def = self.data.def(op);
        let name = def.name();
        // Text concatenation / append operator that BUILDS a fresh text.
        if name == "OpAddText" || name == "OpAppendText" || name == "OpAppendStackText" {
            return TextReturn::Owned(OwnedVia::BuiltLocal);
        }
        // A text field/index VIEW (`OpGetText` chain): owned iff rooted at a
        // LOCAL composite built here (3a); an argument-rooted view is a borrow.
        if op == self.data.def_nr("OpGetText") {
            return match self.text_view_root(tail) {
                Some(root) if self.var_built_in_block(block, root) => {
                    TextReturn::Owned(OwnedVia::ViewOfLocal)
                }
                Some(_) => TextReturn::Borrow(BorrowVia::Argument),
                None => TextReturn::Plain,
            };
        }
        // Any other store getter VIEWS an existing text — not a fresh delivery.
        if name.starts_with("OpGet") {
            return TextReturn::Plain;
        }
        // A user (or native-global) fn call returning text: owned iff its return
        // borrows nothing or only HIDDEN buffer attrs; a return that borrows a
        // VISIBLE argument is a forward-borrow (3b vs p281).
        if matches!(def.returned().base(), Type::Text(_)) {
            let attrs = def.attributes();
            let deps = def.returned().depend();
            let owned = deps
                .iter()
                .all(|&d| (d as usize) < attrs.len() && attrs[d as usize].hidden);
            if owned {
                return TextReturn::Owned(OwnedVia::UserCall);
            }
            // The return forward-borrows ≥1 VISIBLE param.  If EVERY such
            // borrowed param is filled at THIS call site with a LOCAL composite
            // built in this block (`extract(Pair{…})` / `extract(pr)`), the
            // borrow is of a value that dies with this frame, so the tail is
            // materialised into the promoted buffer before the local is freed —
            // owned (ViewOfLocalCall), NOT a true forward.  A borrowed param
            // filled with one of THIS fn's own arguments stays a real forward.
            // (A non-hidden dep is a positional param, so its attr index is the
            // argument position — hidden buffers, which come after, are excluded
            // by the `owned` test above.)
            let Value::Call(_, args) = tail.unspan() else {
                return TextReturn::Borrow(BorrowVia::ForwardArg);
            };
            let all_borrows_local = deps
                .iter()
                .filter(|&&d| !attrs[d as usize].hidden)
                .all(|&d| {
                    args.get(d as usize)
                        .and_then(Self::arg_root_var)
                        .is_some_and(|root| self.var_built_in_block(block, root))
                });
            return if all_borrows_local {
                TextReturn::Owned(OwnedVia::ViewOfLocalCall)
            } else {
                TextReturn::Borrow(BorrowVia::ForwardArg)
            };
        }
        TextReturn::Plain
    }

    /// True when an `if`/`match` ARM tail delivers a fresh owned text (a
    /// native/user owned-text call, recursively through nested `if`/blocks) —
    /// the signal that the branch return is the leaking owned shape (3c).
    fn arm_delivers_owned(&self, arm: &Value, block: &[Value]) -> bool {
        matches!(
            self.classify_text_return(arm, block),
            TextReturn::Owned(OwnedVia::NativeCall | OwnedVia::UserCall | OwnedVia::IfMatchArm)
        )
    }

    /// The root local var of a TEXT field/index view tail — `OpGetText` over a
    /// `OpGetText`/`OpGetVector`/`OpGetField` chain — or `None`.  NOT filtered
    /// by `is_argument` (an NRVO-promoted local reads as an argument here); the
    /// caller uses `var_built_in_block` to tell a local from a true argument.
    fn text_view_root(&self, tail: &Value) -> Option<u16> {
        let gt = self.data.def_nr("OpGetText");
        let gv = self.data.def_nr("OpGetVector");
        let gf = self.data.def_nr("OpGetField");
        fn root(v: &Value, gt: u32, gv: u32, gf: u32) -> Option<u16> {
            match v.unspan() {
                Value::Var(x) => Some(*x),
                Value::Call(d, args) if *d == gt || *d == gv || *d == gf => {
                    args.first().and_then(|a| root(a, gt, gv, gf))
                }
                _ => None,
            }
        }
        match tail.unspan() {
            Value::Return(inner) => self.text_view_root(inner),
            Value::Call(d, _) if *d == gt => root(tail, gt, gv, gf),
            _ => None,
        }
    }

    /// True when `var` is CONSTRUCTED in this block — the target of a `Set` or
    /// an `OpDatabase(var, …)` construction (recursively through
    /// `Block`/`Insert`).  Distinguishes an NRVO-promoted LOCAL composite from
    /// a genuine caller-owned parameter.
    fn var_built_in_block(&self, ops: &[Value], var: u16) -> bool {
        let db = self.data.def_nr("OpDatabase");
        ops.iter().any(|op| self.op_builds_var(op, var, db))
    }

    fn op_builds_var(&self, op: &Value, var: u16, db: u32) -> bool {
        match op.unspan() {
            Value::Set(v, _) => *v == var,
            Value::Call(d, args) if *d == db => {
                matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == var)
            }
            Value::Block(bl) => self.var_built_in_block(&bl.operators, var),
            Value::Insert(ops) => self.var_built_in_block(ops, var),
            Value::Call(_, args) => args.iter().any(|a| self.op_builds_var(a, var, db)),
            _ => false,
        }
    }

    /// The root VAR an argument expression evaluates to — a bare `Var`, or the
    /// tail var of a value-`Block`/`Insert` (an inline composite construction
    /// `Pair{…}` lowers to a block whose last op is `Var(__ref_N)`).  Used by
    /// the `ViewOfLocalCall` classification to check whether the value backing a
    /// forward-borrowed param is a LOCAL built in this block.  `None` when the
    /// argument is not a simple var/composite (e.g. a literal or a call).
    fn arg_root_var(arg: &Value) -> Option<u16> {
        match arg.unspan() {
            Value::Var(v) => Some(*v),
            Value::Block(bl) => bl.operators.last().and_then(Self::arg_root_var),
            Value::Insert(ops) => ops.last().and_then(Self::arg_root_var),
            Value::Return(inner) | Value::Drop(inner) => Self::arg_root_var(inner),
            _ => None,
        }
    }

    /// Peel `Span`/`Return` wrappers off an owned tail value, returning the inner
    /// call (spans dropped — codegen-irrelevant).  Used by the 2d bind to lift the
    /// call out of `return <call>` before rebinding it to `__tret`.
    fn peel_to_inner_call(v: Value) -> Value {
        match v {
            Value::Span(b) => Self::peel_to_inner_call(b.1),
            Value::Return(inner) => Self::peel_to_inner_call(*inner),
            other => other,
        }
    }

    /// H12 — true when the returned expression **projects into** something the
    /// callee owns rather than *being* it: a field (`OpGetField`) or element
    /// (`OpGetVector`) read whose base spine reaches a non-argument LOCAL, or an
    /// inline CALL's temporary.
    ///
    /// That distinction is what makes a `Rename` delivery sound or not. Renaming the
    /// tail's work-ref onto the caller's return buffer delivers the value only when
    /// the tail IS that work-ref; when it merely points *inside* it, the record lives
    /// in a store the callee frees at scope exit, so it must be copied into the buffer
    /// first ([`materialize_view_return`](Self::materialize_view_return)).
    ///
    /// Replaces the narrower `return_field_base_is_call`, which saw only one corner of
    /// it — the field-off-a-call case (H9).  The element case had no predicate at all —
    /// `return b.cells[i]` and its implicit-tail twin both handed the caller a
    /// uniformly-null record, which a consumer reads as "absent" rather than
    /// "broken" (moros H12: an unwritten world cell and a dead one became
    /// indistinguishable).
    ///
    /// An ARGUMENT-rooted projection is deliberately excluded: the caller owns that
    /// store, so the view outlives the call and the dep-driven path already handles it.
    /// What a projection's BASE really is, seen through the wrappers a base arrives in.
    ///
    /// `container_dep` (loft#889) binds an inline call to a work-ref and leaves a block
    /// whose result is that ref, so the base of `make_bag().h[k]` is a Block where the
    /// walkers expect the `Var`.  Reading it as neither a var nor a call answered "this
    /// projection is rooted at nothing", and the field was delivered as if it owned what
    /// it points at.
    fn projection_base(v: &Value) -> &Value {
        match v.unspan() {
            Value::Block(bl) => bl.operators.last().map_or(v, Self::projection_base),
            Value::Insert(ops) => ops.last().map_or(v, Self::projection_base),
            other => other,
        }
    }

    fn return_projects_into_local(&self, tail: &Value) -> bool {
        let (get_field, get_vector) = (
            self.data.def_nr("OpGetField"),
            self.data.def_nr("OpGetVector"),
        );
        match tail.unspan() {
            Value::Return(inner) | Value::Drop(inner) => self.return_projects_into_local(inner),
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|t| self.return_projects_into_local(t)),
            Value::Insert(ops) => ops
                .last()
                .is_some_and(|t| self.return_projects_into_local(t)),
            // An ARM of the tail is a tail too — `if take { t.l } else { null }` hands up
            // `t.l` on one path.  A function WITH a return buffer copies every arm into it
            // through `ref_return`'s copy leg, so only the buffer-less return (`-> S?`, a
            // nullable record has no buffer) asks here, and it materialises per arm
            // (loft#1337, @FR-F-Ret).
            Value::If(_, t, f) if self.return_buffer().is_none() => {
                self.return_projects_into_local(t) || self.return_projects_into_local(f)
            }
            // A TUPLE element read is a projection like the two op calls below, spelled as
            // a `Value` variant: it carries its base as a var NUMBER, so no call pattern
            // can see it. Rooted at a local, its store dies at scope exit like any other.
            Value::TupleGet(base, _) => !self.vars.is_argument(*base),
            Value::Call(d, args) if *d == get_field || *d == get_vector => {
                match args.first().map(Self::projection_base) {
                    // Rooted at a local: freed at scope exit, so the projection dangles.
                    Some(Value::Var(base)) => !self.vars.is_argument(*base),
                    // A field of a tuple element (`t.0.items`) roots at the tuple local.
                    Some(Value::TupleGet(base, _)) => !self.vars.is_argument(*base),
                    // A chained projection — recurse to find the root.
                    Some(inner @ Value::Call(bd, _)) if *bd == get_field || *bd == get_vector => {
                        self.return_projects_into_local(inner)
                    }
                    // An inline call's result is an owned temporary (`__lift_N`), freed
                    // on the way out — the H9 case.
                    Some(Value::Call(_, _)) => true,
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn site_value_ref(&self, tail: &Value) -> Option<u16> {
        match tail.unspan() {
            Value::Var(v) => Some(*v),
            Value::Return(inner) => self.site_value_ref(inner),
            Value::Block(bl) => bl.operators.last().and_then(|t| self.site_value_ref(t)),
            Value::Call(d, args) => {
                let def = self.data.def(*d);
                let i = def.attributes().iter().position(|a| {
                    a.hidden
                        && matches!(
                            &a.typedef,
                            Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                        )
                })?;
                match args.get(i).map(Value::unspan) {
                    Some(Value::Var(v)) => Some(*v),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Rewrite a chained return site whose tail is a bare `call(..., w)`
    /// into the canonical NRVO pair `{ w = call(..., w); w }`.  A bare
    /// call tail only delivers its value via eval-stack-top — interp reads
    /// it, but scopes' `returned_var` sees no `Var`, so the fn epilogue
    /// emits `Return(Null)` and native returns the null sentinel.  The
    /// `Set` is a same-store self-assign at runtime (the call already
    /// wrote into `w`'s buffer — the @P377 no-op shape every consumer
    /// understands); a plain `{ call; w }` block would instead make the
    /// call a DISCARD whose result the witness machinery frees.
    /// @PLN85 — true iff a return tail's terminal value is a `match`/`if`,
    /// descending through `Insert`/`Block`/`Span`/`Return` wrappers.
    fn tail_terminal_is_branch(v: &Value) -> bool {
        match v.unspan() {
            Value::If(_, _, _) => true,
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(Self::tail_terminal_is_branch),
            Value::Insert(ops) => ops.last().is_some_and(Self::tail_terminal_is_branch),
            Value::Return(inner) | Value::Drop(inner) => Self::tail_terminal_is_branch(inner),
            _ => false,
        }
    }

    /// #437 + c5/#448 residual — the fresh-local vector deps of a tail expression
    /// that OWNS a fresh store: a named non-argument local vector (#437), OR a
    /// literal / comprehension whose block result owns a `__vdb` store (every dep
    /// a non-argument local — the c5 residual). `None` if it borrows an argument,
    /// already delivers into `__retbuf` (its dep is the hidden buffer arg), or
    /// isn't a fresh-owned vector. The precondition for renaming its store onto
    /// `__retbuf` so the fn delivers via NRVO instead of returning a bare store an
    /// NRVO caller's chain would orphan.
    fn fresh_owned_vector_deps(&self, v: &Value) -> Option<Vec<u16>> {
        match v.unspan() {
            // #437 — a named non-argument local vector with a backing store.
            //
            // loft#938 — `ret_promo_base`, because the LOCAL's own type carries the `?` too.
            // `v = src(i); return v;` from a `-> vector<T>?` types `v` as
            // `Optional(Vector(…, [__ref_1]))`, which matched no arm here, so the tail
            // intercept in `block_result` never fired and `ref_return` was never reached at
            // ALL for that function — no `LOFT_TRACE_RETPROMO` line, the `returned` deps left
            // empty, and the callee handing back its own `__ref_1` store while the caller's
            // `__retbuf` stayed untouched and was freed empty.  Every neighbouring gate had
            // already been peeled (six of them); this one is on the RETURNED VALUE rather
            // than on the return TYPE, which is why it outlived the sweep that fixed them.
            // Identity while `LOFT_NULLABLE_RETBUF` is off.
            // loft#1101 — `!d.is_empty()` reads @FR-O-Proxy as "has a backing store", and a
            // VIEW of another local reads non-empty too (`e = vv[0]; return e;` deps on
            // `vv`).  This arm answers with the local's DEPS, which for an owner is its
            // own mint — the store to rename — and for a view is somebody else's store:
            // `vv`, a `vector<vector<T>>` container, was renamed onto the `vector<T>`
            // -shaped `__retbuf` and the store it abandoned leaked one per call.  A view
            // owns nothing to rename (@FR-O-Move), so it is not this arm's answer; it is
            // `tail_ret_view_local`'s, which delivers it by COPY instead.
            Value::Var(o)
                if self.vars.exists(*o)
                    && !self.vars.is_argument(*o)
                    && !self.var_views_local(*o)
                    && matches!(self.vars.tp(*o).ret_promo_base(), Type::Vector(_, d) if !d.is_empty()) =>
            {
                let Type::Vector(_, d) = self.vars.tp(*o).ret_promo_base() else {
                    unreachable!()
                };
                Some(d.iter().copied().collect())
            }
            // c5 residual — a fresh literal / comprehension block that owns its
            // store. Every dep must be a non-argument local; this excludes a block
            // already delivering into `__retbuf` (whose dep is the hidden buffer
            // arg) and an arg / struct-field borrow (copied, not renamed).
            Value::Block(bl) => match bl.result.ret_promo_base() {
                Type::Vector(_, d)
                    if !d.is_empty()
                        && d.iter()
                            .all(|&x| self.vars.exists(x) && !self.vars.is_argument(x)) =>
                {
                    Some(d.iter().copied().collect())
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// loft#1101 — the VIEW counterpart of [`fresh_owned_vector_deps`](Self::fresh_owned_vector_deps)
    /// for an explicit `return <local>` tail: a named non-argument local that BORROWS
    /// another local's store (`vv = […]; e = vv[0]; return e;`).  Reads the same two
    /// facts the promotion ladder's rung does — the dep list, and the statement that
    /// defined the local — so the explicit and implicit spellings cannot disagree.
    ///
    /// It answers the LOCAL ITSELF rather than that local's deps, and the difference is
    /// the whole point.  An owner's deps ARE the store to rename onto `__retbuf`; a
    /// view's deps are the store somebody else owns, so handing them over renames a
    /// container onto an element-shaped buffer and orphans what it abandoned.  Naming
    /// the local instead puts the explicit-return spelling on exactly the candidate the
    /// IMPLICIT tail already uses (`ls = ["e"]`), where the promotion ladder's
    /// `var_views_local` rung declines the rename and `Bind` copies the view into the
    /// buffer — value semantics, @FR-O-Move.
    ///
    /// Answering `None` here instead is not the same thing and was measured: with no
    /// candidate the `return` is never stripped, the vector arm below never delivers it,
    /// and the signature stays a BARE vector — the pre-#437 shape whose NRVO caller
    /// chains into a buffer the callee never writes, so `--native` answered an empty
    /// vector while the interpreter looked clean.
    fn tail_ret_view_local(&self, body: &[Value], v: &Value) -> Option<Vec<u16>> {
        match v.unspan() {
            Value::Var(o)
                if self.vars.exists(*o)
                    && !self.vars.is_argument(*o)
                    && (self.var_views_local(*o) || self.var_defined_by_projection(body, *o))
                    && matches!(self.vars.tp(*o).ret_promo_base(), Type::Vector(_, _)) =>
            {
                Some(vec![*o])
            }
            _ => None,
        }
    }

    /// #448 — is the fn's `returned` type already classified to deliver into the
    /// hidden return-buffer attribute `buf_attr`? When so, EVERY return path must
    /// deliver into `__retbuf`, or the caller's buffer free orphans a path that
    /// builds its own store. The precondition for the copy-into-buffer rewrite of
    /// a fresh-local tail (so it does not re-derive / overwrite the classification).
    fn returned_uses_buffer(&self, buf_attr: u16) -> bool {
        // loft#938 — a nullable collection return carries the same dep under its `?`, so
        // ask the base.  Identity while `LOFT_NULLABLE_RETBUF` is off.
        matches!(
            self.data.def(self.context).returned().ret_promo_base(),
            Type::Vector(_, d) if d.contains(&buf_attr)
        )
    }

    /// #448 — does any statement BEFORE the tail contain a `return` that already
    /// DELIVERS into the return buffer `buf` (its value's terminal is the `__retbuf`
    /// var — the lowered shape of an early `return <call>` that NRVO-adopted into
    /// the buffer)? This is the precise precondition for COPYING a fresh-local tail
    /// into `__retbuf`: the buffer is already TAKEN, so `tail_ret_local` cannot
    /// RENAME the tail onto it. When NO early return delivers the buffer (e.g. the
    /// stdlib `split_text`'s early `return [self]` literal, or a plain single-return
    /// fn), the tail must still be renamed — copying it instead orphans the original
    /// store (the 104-split-text regression). Stays within this fn's own control
    /// flow (does not descend into nested fn / closure bodies).
    fn body_has_buffer_return(stmts: &[Value], buf: u16) -> bool {
        fn terminal_is_buf(v: &Value, buf: u16) -> bool {
            match v.unspan() {
                Value::Var(o) => *o == buf,
                Value::Block(bl) => bl.operators.last().is_some_and(|x| terminal_is_buf(x, buf)),
                Value::Insert(ops) => ops.last().is_some_and(|x| terminal_is_buf(x, buf)),
                _ => false,
            }
        }
        fn walk(v: &Value, buf: u16) -> bool {
            match v.unspan() {
                Value::Return(inner) => terminal_is_buf(inner, buf),
                Value::Drop(inner) => walk(inner, buf),
                Value::If(_, t, f) => walk(t, buf) || walk(f, buf),
                Value::Block(bl) => bl.operators.iter().any(|x| walk(x, buf)),
                Value::Insert(ops) => ops.iter().any(|x| walk(x, buf)),
                _ => false,
            }
        }
        stmts.iter().any(|s| walk(s, buf))
    }

    /// @PLN85 cluster II — PER-ARM, native-safe vector NRVO delivery. Descends a
    /// `match`/`if` to each arm's terminal local-vector `Var` and rewrites it to
    /// `Insert([OpClearVector(w), OpAppendVector(w, <local>, rec_tp),
    /// OpFreeRef(<local's __vdb dep>)…, w])`: the arm's element copy is delivered
    /// into the caller's return buffer `w`, the now-dead local backing store is
    /// freed, and the arm yields `w`. So the `If` yields `w` (a hidden param) from
    /// every arm — a return-`if` the native generator already handles — and the
    /// append source is a bare `Var` (not an `if`, which native can't scope in
    /// expression position). This delivers into the eager `__retbuf` work-ref store
    /// (fixing the interp orphan leak) while staying native-compilable. `null` arms
    /// (an exhaustive match's unreachable fall-through) are left untouched.
    fn materialize_vector_arms_into(&mut self, elm: &Type, op: &mut Value, w: u16) -> bool {
        let mut consumed = Vec::new();
        self.materialize_vector_arms_collect(elm, op, w, &mut consumed)
    }

    /// Insert `OpFreeRef(local)` into an arm that did NOT consume `local` — the
    /// cross-arm half of the owned-fresh consume below: the preamble owned-init
    /// allocates the local's store on EVERY path, so the path that never appends
    /// from it must still free it (an already-freed/null ref free is a no-op).
    fn push_arm_free(&mut self, arm: &mut Value, local: u16) {
        let free = self.cl("OpFreeRef", &[Value::Var(local)]);
        match arm {
            Value::Span(b) => self.push_arm_free(&mut b.1, local),
            Value::Insert(ops) if !ops.is_empty() => {
                ops.insert(ops.len() - 1, free);
            }
            Value::Block(bl) if !bl.operators.is_empty() => {
                let n = bl.operators.len() - 1;
                bl.operators.insert(n, free);
            }
            other => {
                let value = std::mem::replace(other, Value::Null);
                *other = Value::Insert(vec![free, value]);
            }
        }
    }

    fn materialize_vector_arms_collect(
        &mut self,
        elm: &Type,
        op: &mut Value,
        w: u16,
        consumed: &mut Vec<u16>,
    ) -> bool {
        match op {
            Value::Span(b) => self.materialize_vector_arms_collect(elm, &mut b.1, w, consumed),
            Value::Return(inner) | Value::Drop(inner) => {
                self.materialize_vector_arms_collect(elm, inner, w, consumed)
            }
            Value::If(_, t, f) => {
                let mut ct = Vec::new();
                let mut cf = Vec::new();
                let a = self.materialize_vector_arms_collect(elm, t, w, &mut ct);
                let b2 = self.materialize_vector_arms_collect(elm, f, w, &mut cf);
                // An owned-fresh local consumed in ONE arm was preamble-allocated on
                // BOTH paths — free it on the sibling path too, else that path leaks
                // the untouched store (the `?? [literal]` then-path).
                for &l in &ct {
                    if !cf.contains(&l) {
                        self.push_arm_free(f, l);
                    }
                }
                for &l in &cf {
                    if !ct.contains(&l) {
                        self.push_arm_free(t, l);
                    }
                }
                consumed.append(&mut ct);
                consumed.append(&mut cf);
                a || b2
            }
            Value::Block(bl) => bl
                .operators
                .last_mut()
                .is_some_and(|last| self.materialize_vector_arms_collect(elm, last, w, consumed)),
            Value::Insert(ops) => ops
                .last_mut()
                .is_some_and(|last| self.materialize_vector_arms_collect(elm, last, w, consumed)),
            // A local whose value is a VIEW OF `w` — `_vec_N: vector<T>["__vdb_1"]` where
            // `__vdb_1` IS the buffer — already holds its answer in the buffer, so it is
            // `w` one indirection down and the `*v != w` guard above does not see it.
            // Delivering it emits `OpClearVector(w); OpAppendVector(w, v)`, which empties
            // the buffer before appending it to itself (the arm answered `[]`), and the
            // dep-free leg below then frees `w` — the CALLER's store.  Leave it alone.
            Value::Var(v)
                if *v != w
                    && !self.vars.tp(*v).depend().contains(&w)
                    && matches!(self.vars.tp(*v), Type::Vector(_, _)) =>
            {
                let local = *v;
                let deps = self.vars.tp(local).depend();
                let rec_tp = self.append_elem_tp(elm);
                let clear = self.cl("OpClearVector", &[Value::Var(w)]);
                let append = self.cl(
                    "OpAppendVector",
                    &[Value::Var(w), Value::Var(local), Value::Int(rec_tp)],
                );
                let mut seq = vec![clear, append];
                // Free the now-dead local backing store(s) the arm built (the
                // append copied their elements into `w`); without this the
                // interpreter orphans them. Idempotent with any scope-exit free.
                // EXCEPT a `skip_free` arm — a BORROWED VIEW (a match-field binding
                // `_mv_items_1 = OpGetField(e,…)`) that does NOT own its backing store
                // (it aliases the subject `e`); freeing its deps would over-free `e`
                // (@PLN85 match_return). The append already copied its elements into `w`.
                if !self.vars.skip_free(local) {
                    // @FR-O-Proxy asks free — the arm's own backing store is released here.
                    // @FR-O-Override is consulted by the enclosing test, which is where the
                    // borrowed-view case (a `_mv_` match-field binding) is turned away; the
                    // veto has to be read for a free on the proxy, and reading it once for
                    // the whole block is what that test is.
                    if deps.is_empty()
                        && self.vars.is_work_ref(local)
                        && !self.vars.is_argument(local)
                    {
                        // OWNED-FRESH local (dep-cleared — e.g. the `?? [literal]`
                        // default work-vector): it OWNS its store, and its only use
                        // is INSIDE this arm, which runs AFTER the pre-return frees
                        // scopes emits.  Consume it HERE (the append copied it into
                        // `w`); the matching pre-return free is DROPPED by
                        // `insert_free`'s reads-filter (scopes.rs) — NOT via
                        // skip_free, which would also suppress the work-ref
                        // preamble owned-alloc (`gen_set_first_vector_null` reads
                        // skip_free as "borrows, no store") and crash the arm's
                        // PreAlloc on a null store.  The early free was a genuine
                        // use-after-free: interp read the freed store silently
                        // (LOFT_POISON SIGSEGVs), native crashed on the 65535
                        // sentinel.  Any residual scope-exit free stays idempotent
                        // (a second free of the sentinelled ref is a no-op).
                        seq.push(self.cl("OpFreeRef", &[Value::Var(local)]));
                        self.vars.set_arm_consumed(local);
                        consumed.push(local);
                    } else {
                        for d in deps {
                            seq.push(self.cl("OpFreeRef", &[Value::Var(d)]));
                        }
                    }
                }
                seq.push(Value::Var(w));
                *op = Value::Insert(seq);
                true
            }
            // A PROJECTION arm — `q.items`, `v[i]`, a keyed lookup — yields a view of a
            // store this frame does not own, and @FR-F-Ret says a returned whole heap value
            // is owned, never a view.  The buffered non-null return copies such a tail
            // (`Delivery::CopyBorrow`), but a nullable return always branches on its null
            // arm, and the projecting arm reached this walk as a call carrying no hidden
            // buffer, which the arm below has nothing to substitute — so the view escaped
            // and the caller's bind aliased the callee's argument field (loft#1345).  Copy
            // the projection's elements into `w`; the source is the argument's store, so
            // nothing is freed here.
            //
            // Both spellings of a projection: the call (`OpGetField` / `OpGetVector` /
            // `OpVectorRef` / `OpGetRecord`) and a tuple element (`TupleGet`), which carries
            // its base as a variable number and is a view of that tuple's store the same way.
            Value::TupleGet(_, _) => {
                let rec_tp = self.append_elem_tp(elm);
                let proj = std::mem::replace(op, Value::Null);
                let clear = self.cl("OpClearVector", &[Value::Var(w)]);
                let append = self.cl("OpAppendVector", &[Value::Var(w), proj, Value::Int(rec_tp)]);
                *op = Value::Insert(vec![clear, append, Value::Var(w)]);
                true
            }
            Value::Call(d, _) if crate::use_analysis::is_projection_op(&self.data, *d) => {
                let rec_tp = self.append_elem_tp(elm);
                let proj = std::mem::replace(op, Value::Null);
                let clear = self.cl("OpClearVector", &[Value::Var(w)]);
                let append = self.cl("OpAppendVector", &[Value::Var(w), proj, Value::Int(rec_tp)]);
                *op = Value::Insert(vec![clear, append, Value::Var(w)]);
                true
            }
            Value::Call(_, _) => {
                // #437/@PLN85 cluster V cluster I-b (O-Move): a Call-terminal arm
                // (`head(0,value)`) writes its OWN hidden `__ref_N` buffer, which
                // this materialiser left untouched (only `Var` terminals above were
                // rewritten).  The epilogue then freed that `__ref_N` while it was
                // the arm's returned value — a dangling ref / silent clobber.
                // Substitute the arm's hidden buffer ref onto the shared return
                // buffer `w` and unregister it (no null-init, no scope-exit free),
                // exactly as `ref_return` does for a bare-call return — so EVERY
                // arm of a materialised single-tail vector match delivers into the
                // one buffer.  `buf == w` (an arm already writing the buffer) is a
                // no-op via the guard, so this is idempotent.
                let mut changed = false;
                for buf in Self::collect_hidden_ref_args(op, &self.data) {
                    if buf != w {
                        Self::substitute_work_ref(op, buf, w);
                        self.vars.unregister_work_ref(buf);
                        changed = true;
                    }
                }
                changed
            }
            _ => false,
        }
    }

    /// #415 — does the block's tail expression read a STRUCT vector field
    /// (`b.v` where the base is a `Reference`), as opposed to a whole var, a
    /// call, or a vector INDEX read (`vv[i]`, base is a `Vector`)? Gates the
    /// implicit-tail copy below: only a struct-field borrow of an argument needs
    /// copying into the return buffer.
    ///
    /// A vector INDEX / nested-element read (`OpGetField(OpGetVector …)`) is
    /// DELIBERATELY excluded.  #426 (A.1) probed generalizing this funnel to the
    /// index-read tail (`fn idx0(w) -> vector { w[0] }`): forcing it through this
    /// `__retbuf` copy path collides the forward temp's inner-element view
    /// store-nr with a freed sibling store once the caller frame has released a
    /// vector store (the `borrow_tail_copy_104` return-buffer model is proven only
    /// for whole-arg / struct-field tails).  The index-read RETURN (#426B) stays
    /// ALIASED until that store-reuse / return-buffer substrate is fixed (routed
    /// forward, the a7 class — see `STABILITY_REDFLAG_REMEDIATION.md` A.1).
    fn tail_is_struct_field_read(&self, l: &[Value]) -> bool {
        let mut v = match l.last() {
            Some(v) => v,
            None => return false,
        };
        loop {
            match v.unspan() {
                Value::Return(inner) | Value::Drop(inner) => v = inner,
                Value::Block(bl) => match bl.operators.last() {
                    Some(x) => v = x,
                    None => return false,
                },
                Value::Call(d, args) => {
                    return *d == self.data.def_nr("OpGetField")
                        && matches!(
                            args.first().map(Value::unspan),
                            Some(Value::Var(bv)) if matches!(self.vars.tp(*bv), Type::Reference(_, _))
                        );
                }
                _ => return false,
            }
        }
    }

    /// Row-104 funnel: does the body tail return a WHOLE vector PARAMETER
    /// directly (`fn idv(v) -> vector { v }` — the implicit-tail sibling of an
    /// explicit `return v`)?  Such a tail borrows the caller's store, so
    /// returning the param as-is ALIASES the argument.  Returns the param var
    /// so the caller can copy it into `__retbuf` — the same value-semantics
    /// COPY the explicit `return v` path (`parse_return`) and the struct-field
    /// tail (`tail_is_struct_field_read`, #415) both perform.  Narrowed to a
    /// bare `Var` that is a vector argument: index / call / field tails keep
    /// their existing handling, which already gives them value semantics (A.2).
    fn tail_whole_arg_vector(&self, l: &[Value]) -> Option<u16> {
        let mut v = l.last()?;
        loop {
            match v.unspan() {
                Value::Return(inner) | Value::Drop(inner) => v = inner,
                Value::Block(bl) => v = bl.operators.last()?,
                Value::Insert(ops) => v = ops.last()?,
                Value::Var(bv) => {
                    return (self.vars.is_argument(*bv)
                        && matches!(self.vars.tp(*bv), Type::Vector(_, _)))
                    .then_some(*bv);
                }
                _ => return None,
            }
        }
    }

    /// @PLN85 over-free class — does the body tail return a vector LOCAL that
    /// BORROWS a visible argument (its type deps name an arg)?  The canonical
    /// case is a match-arm field binding returned directly
    /// (`Filled { items } => items`, where `items` is a borrowed view of the
    /// subject's `items` field, deps `["c"]`).  Such a tail is neither an
    /// `OpGetField` struct-field read (`tail_is_struct_field_read`) nor a whole
    /// vector ARG (`tail_whole_arg_vector`), so without this it falls through to
    /// the `Rename` path — which promotes the borrowed binding onto the CALLER's
    /// return buffer, aliasing the buffer to the arg's store; the caller's later
    /// buffer free then corrupts the arg (P14 enum-field-vector crash).  Routing
    /// it through `CopyBorrow` copies the view into `__retbuf` (value semantics).
    fn tail_borrows_arg(&self, l: &[Value]) -> bool {
        let mut v = match l.last() {
            Some(v) => v,
            None => return false,
        };
        loop {
            match v.unspan() {
                Value::Return(inner) | Value::Drop(inner) => v = inner,
                Value::Block(bl) => match bl.operators.last() {
                    Some(x) => v = x,
                    None => return false,
                },
                Value::Insert(ops) => match ops.last() {
                    Some(x) => v = x,
                    None => return false,
                },
                Value::Var(bv) => {
                    return matches!(self.vars.tp(*bv), Type::Vector(_, _))
                        && self
                            .vars
                            .tp(*bv)
                            .depend()
                            .iter()
                            .any(|&d| self.vars.is_argument(d));
                }
                _ => return false,
            }
        }
    }

    /// Row-104 funnel: copy a BORROWED implicit-tail vector return into the
    /// function's one `__retbuf` buffer and finalize the return-type dep to
    /// `{buf_attr}`, so the caller adopts an independent copy (value
    /// semantics).  The single home for the "the tail borrows a visible arg →
    /// COPY" decision in `block_result`; the struct-field tail (#415) and the
    /// whole-arg param tail (a2) both route here instead of re-deriving the
    /// copy shape inline.  Mirrors the explicit `return <borrow>` copy in
    /// `parse_return` (~4651): capture the tail value into `__fwd`, then
    /// `OpClearVector(buf); OpAppendVector(buf, __fwd); buf`.  Returns true on
    /// success; false (var allocation failed / no tail) tells the caller to
    /// fall back to the `ref_return` path.
    fn copy_borrow_tail_into_retbuf(
        &mut self,
        elm: &Type,
        l: &mut [Value],
        buf_attr: u16,
        buf_var: u16,
    ) -> bool {
        let elm_ty = elm.clone();
        let Some(last) = l.last_mut() else {
            return false;
        };
        let rec_tp = self.append_elem_tp(&elm_ty);
        let clear = self.cl("OpClearVector", &[Value::Var(buf_var)]);
        // Append the borrowed tail value DIRECTLY into the buffer — no `__fwd`
        // local.  This function is only the BORROWED-arg case (the tail views a
        // visible param: a whole-arg vector, a struct-field of an arg), so `orig`
        // never owns its store and never aliases the hidden buffer.  A captured
        // `__fwd` local carried empty deps, so its scope-exit `OpFreeRef` freed
        // the borrowed source — i.e. the caller's vector (P462 over-free, recycled
        // under allocation pressure -> corruption).  Inlining matches the proven
        // explicit `return <borrow>` path in `parse_return`, which appends inline
        // and frees nothing.
        let orig = std::mem::replace(last, Value::Null);
        let append = self.cl(
            "OpAppendVector",
            &[Value::Var(buf_var), orig, Value::Int(rec_tp)],
        );
        *last = crate::data::v_block(
            vec![clear, append, Value::Var(buf_var)],
            Type::Vector(Box::new(elm_ty.clone()), Deps::frame1(buf_var)),
            "borrow_tail_copy_104",
        );
        self.set_delivered_vector_return(elm_ty, buf_attr);
        true
    }

    /// Record that this function's vector result is delivered through `buf_attr`, keeping
    /// the DECLARED `?`.
    ///
    /// The deps belong to the storage and the `?` to the value, so re-typing one must not
    /// drop the other (`ref_return` says the same).  Two delivery legs re-set the returned
    /// type as a bare vector, and a lambda declared `-> vector<T>?` whose tail was such a
    /// delivery then published `fn(Bag) -> vector<integer>` on the pass that delivered and
    /// `-> vector<integer>?` on the pass that did not — refused as a type change of the
    /// variable holding it, while the named twin compiled (loft#1347).
    fn set_delivered_vector_return(&mut self, elm_ty: Type, buf_attr: u16) {
        let delivered = Type::Vector(Box::new(elm_ty), Deps::attrs(vec![buf_attr]));
        let declared = self.data.def(self.context).returned();
        self.data.definitions[self.context as usize].returned = if declared.ret_promo_peels() {
            Type::optional(delivered)
        } else {
            delivered
        };
    }

    fn chain_site_set_shape(ret: &Type, tail: &mut Value, w: u16) {
        match tail {
            Value::Span(b) => Self::chain_site_set_shape(ret, &mut b.1, w),
            Value::Return(inner) => Self::chain_site_set_shape(ret, inner, w),
            // Argument lifting wraps the site call in `Insert([lifts…,
            // call])`; descend to the call so its value still surfaces.
            Value::Insert(ops) => {
                if let Some(last) = ops.last_mut() {
                    Self::chain_site_set_shape(ret, last, w);
                }
            }
            Value::Block(bl) => {
                if let Some(last) = bl.operators.last_mut() {
                    Self::chain_site_set_shape(ret, last, w);
                }
            }
            Value::Call(_, _) => {
                let block_tp = match ret {
                    Type::Reference(td, _) => Type::Reference(*td, Deps::frame1(w)),
                    Type::Vector(it, _) => Type::Vector(it.clone(), Deps::frame1(w)),
                    other => other.clone(),
                };
                let call = std::mem::replace(tail, Value::Null);
                *tail = crate::data::v_block(
                    vec![crate::data::v_set(w, call), Value::Var(w)],
                    block_tp,
                    "one_buffer_chain",
                );
            }
            _ => {}
        }
    }

    /// Vector counterpart of [`materialize_return_into`]: copy a return
    /// site's vector value into the fn's one buffer by element append —
    /// `{ OpClearVector(w); OpAppendVector(w, <orig>, rec_tp); w }` — the
    /// same element copy the explicit-return vector path has always used.
    /// The clear makes delivery REPLACE the buffer's content: a caller
    /// that re-passes the same buffer (a call inside a loop reuses the
    /// fn-scoped `__ref_N`) must see exactly this invocation's result,
    /// not an accumulation of every iteration's appends.
    fn materialize_vector_return_into(&mut self, elm: &Type, tail: &mut Value, w: u16) {
        if let Value::Return(inner) = tail {
            return self.materialize_vector_return_into(elm, inner, w);
        }
        let rec_tp = self.append_elem_tp(elm);
        let orig = std::mem::replace(tail, Value::Null);
        let clear = self.cl("OpClearVector", &[Value::Var(w)]);
        let append = self.cl("OpAppendVector", &[Value::Var(w), orig, Value::Int(rec_tp)]);
        *tail = crate::data::v_block(
            vec![clear, append, Value::Var(w)],
            Type::Vector(Box::new(elm.clone()), Deps::frame1(w)),
            "one_buffer_vec_copy",
        );
    }

    /// Rewrite every mid-body `return <named local vector>` of a
    /// buffer-bound fn into the delivering shape
    /// `Insert([OpClearVector(buf), OpAppendVector(buf, <local>, rec_tp),
    /// Return(buf)])` — the same element copy + replace semantics as
    /// [`materialize_vector_return_into`].  Sites already delivering
    /// (their innermost return value is the buffer var, whether from the
    /// chain shape or the legacy `__ref_1` injection) are left alone, so
    /// the walk is idempotent across parse passes.  Only bare `Var`
    /// values are rewritten: call-chain sites deliver through the callee,
    /// and every other shape keeps its existing behaviour.
    fn deliver_mid_vector_returns(&mut self, elm: &Type, body: &mut [Value], buf_var: u16) {
        for op in body.iter_mut() {
            self.deliver_mid_vector_walk(elm, op, buf_var);
        }
    }

    /// #457 — does `cv` get reassigned to a CALL result anywhere in `body`?
    /// `cv = some_fn(.., __ref_N)` ADOPTS the callee's delivery store, so at the
    /// tail `cv` holds a store DISTINCT from its NRVO buffer.  In-place
    /// `cv += [..]` does NOT count (its `Set` target is the element temp), nor
    /// does the initial `cv: vector = []`.
    fn body_reassigns_var_to_call(body: &[Value], cv: u16) -> bool {
        fn walk(node: &Value, cv: u16) -> bool {
            if let Value::Set(w, val) = node
                && *w == cv
                && matches!(val.unspan(), Value::Call(_, _))
            {
                return true;
            }
            match node {
                Value::Set(_, val) => walk(val, cv),
                Value::Call(_, args)
                | Value::Insert(args)
                | Value::Tuple(args)
                | Value::Parallel(args) => args.iter().any(|a| walk(a, cv)),
                Value::Block(bl) | Value::Loop(bl) => bl.operators.iter().any(|o| walk(o, cv)),
                Value::If(c, t, e) => walk(c, cv) || walk(t, cv) || walk(e, cv),
                Value::Iter(_, c, n, e) => walk(c, cv) || walk(n, cv) || walk(e, cv),
                Value::Return(x) | Value::Drop(x) | Value::Yield(x) => walk(x, cv),
                Value::Span(b) => walk(&b.1, cv),
                _ => false,
            }
        }
        body.iter().any(|o| walk(o, cv))
    }

    fn deliver_mid_vector_walk(&mut self, elm: &Type, op: &mut Value, buf_var: u16) {
        match op {
            Value::Return(inner) => {
                if let Value::Var(v) = inner.unspan()
                    && *v != buf_var
                    && matches!(self.vars.tp(*v), Type::Vector(_, _))
                {
                    let local = *v;
                    let rec_tp = self.append_elem_tp(elm);
                    // Aliasing-safe deliver: `local` may ALIAS `buf_var` (an
                    // un-reassigned `return out` where `out` borrows the buffer),
                    // and the old `clear(buf); append(buf, out)` then emptied it
                    // (the mid-body-return self-copy).  `OpReplaceVector` no-ops
                    // when the two name the same backing vector.
                    let replace = self.cl(
                        "OpReplaceVector",
                        &[Value::Var(buf_var), Value::Var(local), Value::Int(rec_tp)],
                    );
                    *op =
                        Value::Insert(vec![replace, Value::Return(Box::new(Value::Var(buf_var)))]);
                } else if self.fresh_owned_vector_deps(inner.unspan()).is_some() {
                    // c5/#448 residual sibling — a mid-body `return <fresh literal>`
                    // in an NRVO-promoted vector fn must ALSO deliver into __retbuf,
                    // or the buffer-classified caller frees __retbuf and orphans this
                    // path's store (the `dual` early-path leak). The literal block's
                    // terminal Var is a fresh `_vec`, so the cluster-I per-arm
                    // materialiser delivers it (clear+append+free the __vdb), leaving
                    // the block yielding __retbuf; wrap it back in the `return`.
                    self.materialize_vector_arms_into(elm, inner.unspan_mut(), buf_var);
                }
            }
            Value::Span(b) => self.deliver_mid_vector_walk(elm, &mut b.1, buf_var),
            Value::Insert(ops) | Value::Parallel(ops) => {
                for o in ops {
                    self.deliver_mid_vector_walk(elm, o, buf_var);
                }
            }
            Value::Block(bl) | Value::Loop(bl) => {
                for o in &mut bl.operators {
                    self.deliver_mid_vector_walk(elm, o, buf_var);
                }
            }
            Value::If(c, t, e) => {
                self.deliver_mid_vector_walk(elm, c, buf_var);
                self.deliver_mid_vector_walk(elm, t, buf_var);
                self.deliver_mid_vector_walk(elm, e, buf_var);
            }
            Value::Iter(_, c, n, e) => {
                self.deliver_mid_vector_walk(elm, c, buf_var);
                self.deliver_mid_vector_walk(elm, n, buf_var);
                self.deliver_mid_vector_walk(elm, e, buf_var);
            }
            _ => {}
        }
    }

    /// The fn's ONE hidden return buffer: the first hidden heap-typed
    /// attribute of the current context, as `(attr index, bound var)`.
    /// After the first promotion the attr carries the promoted local's
    /// name (the attr↔var coupling is by name), so the var is looked up
    /// through the attr's CURRENT name.  Returns None when the context
    /// has no hidden heap attr or its var is not in this fn's table.
    fn return_buffer(&self) -> Option<(u16, u16)> {
        let def = self.data.def(self.context);
        let a_idx = def.hidden_return_buffer_attr()?;
        let a = &def.attributes()[a_idx];
        let v = self.vars.var(&a.name);
        if v == u16::MAX {
            return None;
        }
        #[allow(clippy::cast_possible_truncation)]
        Some((a_idx as u16, v))
    }

    /// Plan-57: the count of DISTINCT element-temps (`Set(_elm_k,
    /// OpNewRecord(v, …))`) allocated as CHILDREN of `v`.  Visible on the FIRST
    /// pass (where promotion happens), unlike the later `OpPreAllocVector` form.
    ///
    /// Read it as what it measures — child allocations — not as "how often `v`
    /// was reassigned".  A rebind (`r = S{…}; r = S{…}`) lowers to
    /// `OpDatabase(r, …)` and is invisible here; what the count actually sees is
    /// a fresh vector literal (one temp per element) AND an ordinary append
    /// (`v.items += [x]`), which is why `classify_ret_promotion` reads ≥2 as
    /// "reassigned" only for a LOCAL still awaiting a placement decision, and
    /// carves out the vector body-tail that legitimately appends twice.
    fn child_allocs(body: &[Value], v: u16, nr: u32) -> usize {
        fn collect(node: &Value, v: u16, nr: u32, temps: &mut std::collections::HashSet<u16>) {
            if let Value::Set(w, val) = node
                && let Value::Call(op, args) = val.unspan()
                && *op == nr
                && matches!(args.first().map(Value::unspan), Some(Value::Var(s)) if *s == v)
            {
                temps.insert(*w);
            }
            match node {
                Value::Set(_, val) => collect(val, v, nr, temps),
                Value::Call(_, args)
                | Value::Insert(args)
                | Value::Tuple(args)
                | Value::Parallel(args) => {
                    for a in args {
                        collect(a, v, nr, temps);
                    }
                }
                Value::Block(bl) | Value::Loop(bl) => {
                    for o in &bl.operators {
                        collect(o, v, nr, temps);
                    }
                }
                Value::If(c, t, e) => {
                    collect(c, v, nr, temps);
                    collect(t, v, nr, temps);
                    collect(e, v, nr, temps);
                }
                Value::Iter(_, c, n, e) => {
                    collect(c, v, nr, temps);
                    collect(n, v, nr, temps);
                    collect(e, v, nr, temps);
                }
                Value::Return(x) | Value::Drop(x) | Value::Yield(x) => {
                    collect(x, v, nr, temps);
                }
                Value::Span(b) => collect(&b.1, v, nr, temps),
                _ => {}
            }
        }
        let mut temps = std::collections::HashSet::new();
        for o in body {
            collect(o, v, nr, &mut temps);
        }
        temps.len()
    }

    /// A1b (@PLN90 W1) — the return tail is a buffer-ABI CALL that borrows a
    /// TEMPORARY subject the fn constructs: `g(Filled{..})` where `g` returns a
    /// vector via a hidden buffer, and a VISIBLE heap param's arg is an inline
    /// construct (NOT a bare `Var` — a struct/enum/vector literal, a local freed at
    /// scope exit). Renamed onto `__retbuf` the borrowed view dangles once the temp
    /// is freed (`cell-escape-temp` UAF). Robust across both parse passes: the
    /// construct is a pre-lowered expression in pass 1 and an `Object` block in pass
    /// 2 — neither a `Var`, whereas the safe `g(c)` param arg IS a `Var` in both.
    fn tail_call_borrows_temp_subject(&self, body: &[Value]) -> bool {
        let Some(last) = body.last() else {
            return false;
        };
        if Self::collect_hidden_ref_args(last, &self.data).is_empty() {
            return false;
        }
        let mut node = last.unspan();
        loop {
            match node {
                Value::Block(bl) => match bl.operators.last() {
                    Some(x) => node = x.unspan(),
                    None => return false,
                },
                Value::Insert(ops) => match ops.last() {
                    Some(x) => node = x.unspan(),
                    None => return false,
                },
                Value::Return(inner) => node = inner.unspan(),
                _ => break,
            }
        }
        let Value::Call(d_nr, args) = node else {
            return false;
        };
        let callee = self.data.def(*d_nr);
        if !matches!(callee.returned(), Type::Vector(_, _)) {
            return false;
        }
        let attrs = callee.attributes();
        args.iter().enumerate().any(|(i, a)| {
            attrs.get(i).is_some_and(|at| {
                !at.hidden
                    && matches!(
                        at.typedef,
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    )
            }) && !matches!(a.unspan(), Value::Var(_))
        })
    }

    /// @PLN85 D-own-1 slice 3 — the per-var verdict of `ref_return`'s
    /// promotion loop as a PURE selector (the classify/apply split
    /// `classify_text_dep` / `classify_vector_delivery` use, applied to the
    /// NRVO/promotion ladder).  Rule rationale lives on the `RetPromotion`
    /// variants; ORDER of the rungs is load-bearing (e.g. `MergeOnly` guards
    /// `SkipInnerRef` — a transitive work ref must merge, not skip).
    ///
    /// `dep` is the return-dep list AS ACCUMULATED so far — a later
    /// candidate's `Rename` is suppressed once ANY earlier site chained into
    /// the placeholder (`bound_already`), so classification is per-var
    /// in-loop, not a pre-pass.
    /// Trace wrapper around [`classify_ret_promotion_inner`](Self::classify_ret_promotion_inner)
    /// — it prints the candidate AND the VERDICT, because those are two different questions.
    /// No line at all for a function means a gate UPSTREAM of the classifier; a line whose
    /// verdict is a `Skip*` means the classifier was asked and said no.
    ///
    /// It also prints the FACTS the verdict is read from — the parser pass, the candidate's
    /// dep NAMES, and the two borrow answers as `vl=<deps>/<statement>` — so a wrong verdict
    /// can be attributed without a second run.  Read the two passes in PAIRS: pass 1 is where
    /// a rename is decided and pass 2 normally answers `MergeAttr` for the same candidate
    /// (it IS the attribute by then).  A fact that differs between the pair is the bug to
    /// chase, because this verdict decides whether the function takes a hidden buffer
    /// argument — the deps alone do differ, since a mint dep is added on pass 2 only, which
    /// is why the borrow answers and not the raw list are what the rungs read.
    fn classify_ret_promotion(
        &self,
        v: u16,
        transitive: bool,
        body: &[Value],
        dep: &[u16],
        ctx: &RetPromoCtx,
    ) -> RetPromotion {
        let verdict = self.classify_ret_promotion_inner(v, transitive, body, dep, ctx);
        if crate::keys::trace_ret_promotion() {
            eprintln!(
                "[retpromo] fn={} v={} name={} pass={} vl={}/{} deps={:?} site={:?} ret={:?} plain={} buf={:?} => {verdict:?}",
                self.data.def(self.context).name(),
                v,
                self.vars.name(v),
                if self.first_pass { 1 } else { 2 },
                self.var_views_local(v),
                self.var_defined_by_projection(body, v),
                self.vars
                    .tp(v)
                    .depend()
                    .iter()
                    .map(|&d| self.vars.name(d).to_string())
                    .collect::<Vec<_>>(),
                ctx.site,
                ctx.ret,
                ctx.is_plain_fn,
                self.return_buffer(),
            );
        }
        verdict
    }

    fn classify_ret_promotion_inner(
        &self,
        v: u16,
        transitive: bool,
        body: &[Value],
        dep: &[u16],
        ctx: &RetPromoCtx,
    ) -> RetPromotion {
        if ctx.jo_arm_skip.contains(&v) {
            return RetPromotion::SkipDelivered;
        }
        let n = self.vars.name(v);
        let is_work_ref = n.starts_with("__ref_") || n.starts_with("__rref_");
        // A1b (@PLN90 W1, gated) — the tail borrows a temporary subject the fn
        // constructs. For the SITE-VALUE work-ref (g's buffer) suppress the Rename
        // and materialise into `__retbuf` (an owned copy); for the SUBJECT work-ref
        // (an inner ref the site adopts) SKIP promotion so it keeps its own store,
        // freed after the copy — the three stores stay distinct (no collapse UAF).
        let a1b =
            crate::keys::a1b_materialise_enabled() && self.tail_call_borrows_temp_subject(body);
        let a1b_site = a1b && ctx.site_value == Some(v);
        // A var that is ALREADY an attribute outranks every promotion rung below.
        // Those rungs decide where a LOCAL lives (rename it onto `__retbuf`, bind
        // it, promote it to a param); an attribute has nothing left to place, so
        // the only thing left to say about it is which attr the return borrows.
        // That is a FACT about the signature, not a placement choice — a promotion
        // rule that pre-empts it does not avoid a bad promotion, it deletes the
        // borrow (loft#677: `fn add(o: Outer, …) -> Outer { o.tags += […];
        // o.items += […]; o }` scored two child allocations under `o`, tripped the
        // reassignment skip below, and lost the `["o"]` dep — callers then read
        // the returned borrow as owned and freed the CALLER's store).
        if let Some(a) = self.data.def(self.context).attr_names.get(n) {
            let a = *a as u16;
            // #356: pass 2 re-finds a pass-1-promoted site work ref by name
            // here — the site STILL needs its value made explicit each pass.
            let chain_site = ctx.site == RetSite::MidReturn
                && is_work_ref
                && !transitive
                && self.data.def(self.context).attributes()[a as usize].hidden;
            return RetPromotion::MergeAttr { a, chain_site };
        }
        // A CAPTURE has nothing to place either, and for the same reason the attribute rung
        // above gives: the store belongs to the frame that made it, and the body reads it out
        // of the closure record.  `classify_text_dep` has answered this exact question with
        // `TextDep::SkipCaptured` since @PLN85 — one notion, and the ref ladder could not see
        // it, so `{ q.xs }` grew a hidden `q` buffer that the body then ignores.  The
        // interpreter hid that: `State::fn_return` releases any buffer the callee did not hand
        // back (loft#1179), a runtime check that does not care what the deps claim.  `--native`
        // reads the deps instead — `arm_frees_buf` frees an unfilled `__vc_hbuf` only when the
        // candidate's return deps do NOT name a hidden heap attr, and they do, because the
        // buffer exists — so it leaked one store per call (loft#1182).
        if self.captured_names.iter().any(|(name, _)| name == n) {
            return RetPromotion::SkipCaptured;
        }
        // A reassigned returned LOCAL must NOT be NRVO-promoted — but a NAMED
        // local at a vector fn's body tail still DELIVERS: it falls through to
        // the `Bind` copy leg (reassignment is irrelevant to a single
        // copy-at-exit).  Skipping it entirely leaves the fn value-returning
        // while callers — who can only consult the signature (a forward caller
        // parses before this body) — assume buffer delivery and free the
        // buffer alone: the returned store leaks (#355 fallout, the 93-vsort
        // suite leak).  `child_allocs` counts appends, so that vector carve-out
        // is the same false positive #677 hit on the attribute path above.
        let reassigned = Self::child_allocs(body, v, ctx.newrecord_nr) >= 2;
        if reassigned
            && !(!is_work_ref
                && ctx.is_plain_fn
                && ctx.site == RetSite::BlockTail
                && matches!(ctx.ret.ret_promo_base(), Type::Vector(_, _)))
        {
            return RetPromotion::SkipReassigned;
        }
        if transitive {
            return RetPromotion::MergeOnly;
        }
        // Cluster I-d (@PLN85 cluster V) EXCEPTION — the site value ADOPTS this
        // work ref: `buf = head(.., __ref_1); …; return buf`, where `buf`'s dep
        // is `__ref_1` (buf aliases head's returned store).  Here `buf ==
        // __ref_1` at runtime, so promoting `__ref_1` to `__retbuf` makes
        // `buf == __retbuf` (true NRVO) — the same end-state the `buf = []`
        // literal path reaches directly.  Left un-promoted the fn returns a
        // FRESH adopt store while a `["??"]` caller (e.g. a `match` wrapper)
        // frees the unused buffer and the adopted store LEAKS (the I-c
        // face-flip).  Only skip when the site value does NOT adopt `v`.
        let site_adopts_v = ctx
            .site_value
            .is_some_and(|sv| self.vars.tp(sv).depend().contains(&v));
        if is_work_ref
            && ctx.site_value.is_some()
            && ctx.site_value != Some(v)
            && (!site_adopts_v || a1b)
        {
            // A1b — `a1b` overrides `site_adopts_v`: the site borrows this subject
            // ref, but it must be COPIED (materialised), not aliased, so keep the
            // subject a distinct local instead of substituting it into the buffer.
            return RetPromotion::SkipInnerRef;
        }
        // A MID-BODY vector return never renames: the rename makes the site's
        // local the fn-wide buffer, which is only sound at the body tail (the
        // 01b breakage) — vector mid-returns bind through `Bind` instead.  And
        // once ANY earlier site chained into the placeholder (`dep` already
        // names the buffer attr), renaming would retire the placeholder var
        // those sites reference — the later candidate must copy instead.
        //
        // A RECORD mid-return still renames, and loft#688 is what that costs when
        // a sibling path returns a different store: the renamed local is minted by
        // this function but lives in scope 0, which never frees.  Refusing the
        // rename here fixes the leak but is too wide — an explicit `return out;`
        // is a `MidReturn` even when it is the function's ONLY exit, where the
        // rename is both sound and load-bearing (the sandbox admission walk reads
        // the promotion to know the write escapes).  So the leak is repaired where
        // the ownership invariant lives, in `scopes::free_vars`, and the promotion
        // ladder is left alone.
        let bound_already = self.return_buffer().is_some_and(|(a, _)| dep.contains(&a));
        // #425 — the return value is a struct/enum FIELD projection of THIS
        // candidate (`return d.value`, where `d` is the container local).
        // Renaming `d` to the return buffer is wrong: `d` holds the WHOLE
        // record while the fn returns its inner field, so the promoted buffer
        // would be the container, the field sub-ref dropped, and `d` freed at
        // scope exit — the returned value dangles (native re-encodes to 0
        // bytes).  Suppress the rename so the candidate falls through to the
        // `Bind` copy leg (`materialize_return_into` deep-copies `d.value`
        // into the separate `__retbuf`).  A field-of-ARGUMENT never reaches
        // here (a true parameter hits the earlier `MergeAttr`), and a
        // local-bind (`v = d.value; return v`) returns `v` itself (not a
        // projection), so this is field-projection-of-a-local only.
        let returns_own_field =
            self.return_field_base_var(body.last().unwrap_or(&Value::Null)) == Some(v);
        // A vector local bound to a branch join is REBOUND, not built into, so it cannot
        // carry the rename — `var_bound_to_branch` states the rule.  Vector only: a record
        // return re-mints its destination through `materialize_return_into`, so the rebind
        // has nothing to abandon there.
        let bound_to_vector_join = matches!(ctx.ret.ret_promo_base(), Type::Vector(_, _))
            && (Self::var_bound_to_branch(body, v)
                || self
                    .branch_sunk_vectors
                    .contains(&(self.context, n.to_string())));
        // loft#1101 — the candidate is a VIEW of another local (`e = vv[0]; e`), the
        // binding twin of the tail projection `returns_own_field` above suppresses.
        // That rung reads the tail SHAPE, so it only sees a projection written at the
        // return; once the projection happens at a binding the tail is a bare `Var` and
        // the fact lives in that binding's DEPS instead.  `var_views_local` reads it
        // there (@FR-O-Move / @FR-O-Borrow).  Vector only: the record return reaches
        // its own view repair earlier, through `classify_reference_delivery`'s
        // `return_views_local` leg (#306).
        let views_local = matches!(ctx.ret.ret_promo_base(), Type::Vector(_, _))
            && (self.var_views_local(v) || self.var_defined_by_projection(body, v));
        let allow_rename = !(bound_already
            || reassigned
            || returns_own_field
            || bound_to_vector_join
            || views_local
            // A1b — the site-value (g's buffer) must NOT rename onto __retbuf (that
            // aliases the borrowed subject into the return); fall through to Bind so
            // the return materialises an owned copy into a distinct __retbuf.
            || a1b_site
            || (ctx.site == RetSite::MidReturn
                && matches!(ctx.ret.ret_promo_base(), Type::Vector(_, _))));
        // loft#1188 — a LAMBDA whose buffer was RESERVED between the passes BINDS to it instead
        // of renaming onto it.  The placeholder is minted before pass 2 appends the `__closure`
        // argument, and the work-ref this tail mints comes after BOTH; the rename retires the
        // placeholder and makes that later var the argument, which puts the callee's argument
        // slots out of the attribute order the CALL SITE lowers against.  Measured on both
        // legs: `CallRef` wrote the closure into the buffer's slot, so a record return answered
        // a zeroed record, and a CAPTURE inside a comprehension-tailed collection lambda read
        // its integer as 0 — silently, and only on the interpreter, because `--native` derives
        // its argument list from the attributes alone.  Binding keeps the reserved var, so the
        // geometry is the one a lambda whose types all resolved in pass 1 already has.
        let lambda_binds_reserved_buffer = !ctx.is_plain_fn
            && matches!(
                ctx.ret.ret_promo_base(),
                Type::Reference(_, _) | Type::Enum(_, true, _) | Type::Vector(_, _)
            )
            && self
                .data
                .def(self.context)
                .attr_names
                .contains_key("__retbuf")
            && self.vars.var("__retbuf") != u16::MAX;
        if allow_rename
            && !lambda_binds_reserved_buffer
            && let Some(&buf_attr) = self.data.def(self.context).attr_names.get("__retbuf")
        {
            return RetPromotion::Rename {
                buf_attr,
                chain_site: ctx.site == RetSite::MidReturn && is_work_ref,
            };
        }
        // loft#1078 — the buffer a named local would be COPIED into is READ by the tail.
        // A RECORD return only: `materialize_return_into` re-mints the destination with
        // `OpDatabase` before the copy evaluates its source, so a destination the source
        // reads is destroyed first.  The collection and text returns carry their own
        // aliasing-aware delivery (`OpReplaceVector` is a documented no-op when the
        // source still aliases the buffer, and the B5-L3 text hoist copies first), and
        // both were measured clean on this shape — narrowing here keeps the guard on the
        // path that actually re-mints.
        let tail_reads_buffer = matches!(
            ctx.ret.ret_promo_base(),
            Type::Reference(_, _) | Type::Enum(_, true, _)
        ) && !is_work_ref
            && self.return_buffer().is_some_and(|(_, buf_var)| {
                buf_var != v && body.last().is_some_and(|t| t.reads_var(buf_var))
            });
        if tail_reads_buffer {
            return RetPromotion::SkipJoinArm;
        }
        // loft#938 gate 5 of 5 — the classification that EMITS the delivery into `__retbuf`.
        if (ctx.is_plain_fn || lambda_binds_reserved_buffer)
            && matches!(
                ctx.ret.ret_promo_base(),
                Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
            )
            && let Some((buf_attr, buf_var)) = self.return_buffer()
            && buf_var != v
        {
            return RetPromotion::Bind {
                buf_attr,
                buf_var,
                // A1b — the site-value ref materialises (copies) its borrowed result
                // into __retbuf rather than substituting the buffer var in place
                // (which would re-alias the freed subject).
                substitute: is_work_ref && !a1b_site,
            };
        }
        RetPromotion::Grow
    }

    /// Does any `return` in `body` READ `var` — directly, OR through a LOCAL
    /// that holds a VIEW of `var`?  A param a mid-body `return` genuinely
    /// BORROWS must not be pruned from the return dep as a stale copy-return
    /// artifact: the tail-source `expanded` set is built from the TAIL return
    /// sources only, so it misses a param a mid-body `return` borrows — either
    /// DIRECTLY (`if c { return t[i] ?? d; }` — 150-i306) or INDIRECTLY through
    /// a view local (`m = table[i]; return m as M;` — #496: the return names
    /// `m`, not `table`, yet `m: Reference(M, [table])` is a live view of the
    /// param; wrongly pruning `table` made the caller free the borrowed vector).
    ///
    /// A struct-COPY local (`r = x; return r`) carries EMPTY deps after the
    /// pass-2 copy-strip, so it is not a view of `x` and a stale copy-return
    /// dep still prunes — the D-own-2 behaviour this guard was added to keep.
    fn body_return_borrows(&self, body: &[Value], var: u16) -> bool {
        // Frame vars that ARE `var` or transitively hold a view of it (their
        // type deps reach `var`).  Fixpoint so a chain `a = t[i]; b = a; return b`
        // still counts `t` as borrowed.
        let mut borrowers: std::collections::HashSet<u16> = std::collections::HashSet::new();
        borrowers.insert(var);
        let mut changed = true;
        while changed {
            changed = false;
            for w in 0..self.vars.count() {
                if borrowers.contains(&w) {
                    continue;
                }
                if self
                    .vars
                    .tp(w)
                    .depend()
                    .iter()
                    .any(|d| borrowers.contains(d))
                {
                    borrowers.insert(w);
                    changed = true;
                }
            }
        }
        fn walk(op: &Value, borrowers: &std::collections::HashSet<u16>) -> bool {
            match op {
                Value::Return(inner) => borrowers.iter().any(|&w| inner.reads_var(w)),
                Value::Span(b) => walk(&b.1, borrowers),
                Value::Insert(ops) | Value::Parallel(ops) => ops.iter().any(|o| walk(o, borrowers)),
                Value::Block(bl) | Value::Loop(bl) => {
                    bl.operators.iter().any(|o| walk(o, borrowers))
                }
                Value::If(c, t, e) => {
                    walk(c, borrowers) || walk(t, borrowers) || walk(e, borrowers)
                }
                Value::Iter(_, c, n, e) => {
                    walk(c, borrowers) || walk(n, borrowers) || walk(e, borrowers)
                }
                _ => false,
            }
        }
        body.iter().any(|s| walk(s, &borrowers))
    }

    pub(crate) fn ref_return(&mut self, ls: &[u16], body: &mut [Value], site: RetSite) {
        // loft#938 — an ENTRY line, not just a verdict line.  `LOFT_TRACE_RETPROMO`
        // documented "no line for a function means a gate UPSTREAM of the classifier",
        // which is true and was not actionable: the pass has two upstream gates, and
        // silence could not tell "`ref_return` ran and classified nothing" from
        // "`ref_return` was never called".  Those are different bugs in different files,
        // and the second one is what gate 7 turned out to be.
        if crate::keys::trace_ret_promotion() {
            eprintln!(
                "[retpromo] ENTER fn={} site={site:?} ls={:?}",
                self.data.def(self.context).name(),
                ls.iter()
                    .map(|v| self.vars.name(*v).to_string())
                    .collect::<Vec<_>>(),
            );
        }
        let newrecord_nr = self.data.def_nr("OpNewRecord");
        let null_sentinel_nr = self.data.def_nr("OpNullRefSentinel");
        let ret = self.data.definitions[self.context as usize]
            .returned
            .clone();
        if std::env::var("LOFT_TRACE_RR").is_ok() {
            let fn_name = self.data.def(self.context).name();
            let ls_named: Vec<String> = ls
                .iter()
                .map(|v| format!("{}={:?}", self.vars.name(*v), self.vars.tp(*v)))
                .collect();
            eprintln!(
                "[rr] fn={fn_name} pass1={} ls={ls:?} ls_tps={ls_named:?} ret={ret:?}",
                self.first_pass
            );
        }
        // @PLN85 match_return (LOFT_JOIN_OWN): a borrowed-view match-field binding
        // (`_mv_items_1 = OpGetField(e,…)`, skip_free, deps `[e]`) returned directly must
        // NOT be NRVO-promoted to BE the retbuf — that aliases the caller's buffer onto
        // `e` (the over-free) or, materialised in place, reuses the binding var as the
        // buffer (a `["_mv_items_1"]`-typed store the lifetime analysis never tracks as
        // the owned return → churn UAF). Instead recover the PROVEN `deliver3` structure:
        // a SEPARATE canonical `__retbuf` buffer, the binding stays a local, and each arm
        // COPIES the binding into `__retbuf` via the proven per-arm machinery
        // (`materialize_vector_arms_into`, no-free for the borrowed binding). Then skip
        // promoting the binding below (it is now a delivered local, not the return).
        let mut jo_arm_skip: std::collections::HashSet<u16> = std::collections::HashSet::new();
        if crate::keys::join_own_enabled()
            && let Type::Vector(ret_elm, _) = &ret
            && let Some((buf_attr, buf_var)) = self.return_buffer()
        {
            let elm_ty = (**ret_elm).clone();
            let borrowed: Vec<u16> = ls
                .iter()
                .copied()
                .filter(|&v| {
                    v < self.vars.count()
                        && v != buf_var
                        && self.vars.skip_free(v)
                        && matches!(self.vars.tp(v), Type::Vector(_, _))
                        && !self.vars.tp(v).depend().is_empty()
                })
                .collect();
            if !borrowed.is_empty()
                && let Some(last) = body.last_mut()
            {
                self.materialize_vector_arms_into(&elm_ty, last, buf_var);
                jo_arm_skip.extend(borrowed);
                // Finalise the return dep to `{__retbuf}` — EXACTLY as the proven
                // `Delivery::Materialize` path does (`dispatch_vector_delivery`): the
                // arms now deliver an owned copy into `__retbuf`, so the return is that
                // buffer. Skipping the promotion dropped this, leaving an empty dep — the
                // caller then neither adopts nor frees, leaking `cell`+`inner`.
                self.data.definitions[self.context as usize].returned =
                    Type::Vector(Box::new(elm_ty.clone()), Deps::attrs(vec![buf_attr]));
            }
        }
        // @PLN85 L3 — re-read `ret` AFTER the jo pre-pass: the finalization
        // below rebuilds `returned` from this clone's deps, and the stale
        // pre-pass-less clone CLOBBERED the `{__retbuf}` dep the pre-pass just
        // finalised (every promotion candidate is jo_arm_skip'd, so the loop
        // re-adds nothing).  With empty returned deps the caller typed the
        // result OWNED: it double-freed the delivered buffer (result + buffer
        // var, same store) and — when a `??`-discharge read made scan_set's
        // dep-prefix null-init the result var at entry — the entry-allocated
        // store was orphaned by the call assignment (the corpus L3 leak).
        let ret = self.data.definitions[self.context as usize]
            .returned
            .clone();
        // B2-runtime / B3 / B7 unification (2026-04-13): struct-enums
        // (Type::Enum with struct-enum discriminator `true`) live as
        // heap-allocated records just like Reference and Vector do, so
        // their return-slot must also be promoted to a hidden caller
        // argument.  Without this arm the callee allocates its own
        // DbRef locally; the caller never reserves matching stack space;
        // OpReturn's value-width mismatches the reserved slot and the
        // interpreter loops on Return(ret=0, value=16) at PC=0.
        // loft#938 gate 2 of 5, and the one that hid the rest: this guards the WHOLE
        // promotion pass, so a nullable return matched no arm and `classify_ret_promotion`
        // was never called for it — no output from `LOFT_TRACE_RETPROMO` at all.
        //
        // loft#974 — it is `ret_dep_shape`, not `ret_promo_base`, because two different
        // questions were being asked with one selector.  A nullable STRUCT return
        // (`-> Item?` reading `b.items[k]`) still owes its caller the SIGNATURE fact that
        // the result borrows `b`; what it must NOT get is a second delivery.  Peeling it
        // here with `signature_only` set records the borrow and makes no placement
        // decision — widening the DELIVERY peel instead was measured, and it re-typed the
        // return non-nullable and diverged the backends on a missing key.
        let (dep_base, peel) = ret.ret_dep_shape();
        // loft#1140 — a KEYED collection return takes the borrow fact and no placement
        // decision, which is what `RetPeel::SignatureOnly` already means.  It cannot be
        // said through `peel`, because that answers two questions at once: *was a `?`
        // peeled* (which `rewrap` below reads, to put the `?` back) and *is this
        // signature-only*.  Those coincide for `Optional(Reference)` and come apart here —
        // a bare `hash<T[k]>` needs the second and must NOT be re-wrapped — so the
        // signature-only question gets its own answer and `rewrap` keeps reading `peel`.
        let signature_only = peel == crate::data::RetPeel::SignatureOnly
            || crate::parser::vectors::is_keyed(dep_base);
        // loft#1143 — the NULLABLE spelling takes the same borrow fact as the dense one.
        // `hash<T[k]>?` arrives as `Optional(Hash)`, and `ret_dep_shape` peels it to
        // `SignatureOnly` for the same reason @FR-L-Null gives everywhere else: `layout(τ) =
        // layout(τ?)`, so a `?` around a keyed collection changes what the slot may HOLD and
        // not what it borrows.  Recording the borrow is only half — the caller must also own
        // the store it copies into, which is the dep-strip in `expressions.rs`'s keyed
        // assignment; without that half the peel alone routes the copy through the u16::MAX
        // null sentinel.
        if let Type::Vector(_, cur)
        | Type::Reference(_, cur)
        | Type::Enum(_, true, cur)
        | Type::Hash(_, _, cur)
        | Type::Sorted(_, _, cur)
        | Type::Index(_, _, cur)
        | Type::Radix(_, _, cur)
        | Type::Trie(_, _, cur) = dep_base
        {
            let mut dep = cur.clone();
            // #306: a returned local can itself hold a view — its TYPE deps name
            // the vars it borrows from (`chosen = table[idx]; chosen` gives
            // `chosen: Reference(M, [table])`).  Walk deps transitively and merge
            // every PARAMETER the returned value may alias into the declared
            // return deps; otherwise the call site treats the value as owned and
            // frees the caller's store at scope exit.  Transitively-reached vars
            // are merge-only: promoting them to hidden ref args (as direct `ls`
            // entries are) would change the call ABI for locals the NRVO
            // machinery cannot host (e.g. a call-result vector), breaking callers.
            let mut expanded: Vec<u16> = ls.to_vec();
            let direct_count = expanded.len();
            let mut seen: std::collections::HashSet<u16> = expanded.iter().copied().collect();
            let mut i = 0;
            while i < expanded.len() {
                let v = expanded[i];
                i += 1;
                if v >= self.vars.count() {
                    continue; // foreign dep (e.g. closure work var) — not ours
                }
                // @PLN85 match_return: a binding delivered into `__retbuf` above is no
                // longer the return — do NOT walk its deps into the return type, else
                // the owned copy's return re-acquires the `["e"]` borrow (the caller then
                // skips freeing `e`'s owner → leak).
                if jo_arm_skip.contains(&v) {
                    continue;
                }
                for d in self.vars.tp(v).depend() {
                    if d < self.vars.count() && seen.insert(d) {
                        expanded.push(d);
                    }
                }
            }
            // The ref carrying THE SITE'S VALUE (a tail call's buffer arg /
            // a plain Var tail).  Only this ref may bind to the fn's one
            // return buffer; an INNER call's work ref (`return wrap(mk(x))`
            // carries two) must stay a plain local — binding both would
            // alias the outer call's destination with its own argument.
            let site_value = body.last().and_then(|t| self.site_value_ref(t));
            let is_plain_fn = !self.data.def(self.context).name().contains("__lambda")
                && self.data.def_type(self.context) == crate::data::DefType::Function;
            // @PLN85 D-own-1 slice 3 — the per-var verdict sentinel (trace only):
            // one line per promotion verdict so the corpus's coverage of every
            // ladder rung is PROVEN before the classify_ret_promotion cut.
            let rr = std::env::var("LOFT_TRACE_RR").is_ok();
            // @PLN85 D-own-1 slice 3 — classify ONCE per var (the pure
            // selector), then apply the one mechanism per verdict.  The rule
            // rationale lives on the `RetPromotion` variants; the arms carry
            // only emission mechanics.
            let ctx = RetPromoCtx {
                site,
                ret: &ret,
                site_value,
                is_plain_fn,
                newrecord_nr,
                jo_arm_skip: &jo_arm_skip,
            };
            for (e_idx, v) in expanded.iter().enumerate() {
                let verdict =
                    self.classify_ret_promotion(*v, e_idx >= direct_count, body, &dep, &ctx);
                if rr {
                    eprintln!("[rr]   v={} verdict={verdict:?}", self.vars.name(*v));
                }
                match verdict {
                    RetPromotion::SkipDelivered
                    | RetPromotion::SkipReassigned
                    | RetPromotion::MergeOnly
                    | RetPromotion::SkipInnerRef
                    | RetPromotion::SkipCaptured
                    | RetPromotion::SkipJoinArm => {}
                    // loft#974 — a shape that carries its own delivery takes the borrow
                    // fact and nothing else: no buffer rename, no bind, no arity growth.
                    // Every leg below rewrites where the value LIVES, and this return
                    // already has somewhere to live.
                    RetPromotion::Rename { .. } | RetPromotion::Bind { .. } if signature_only => {}
                    RetPromotion::Grow if signature_only => {}
                    RetPromotion::MergeAttr { a, chain_site } => {
                        if !dep.contains(&a) {
                            dep.push(a);
                        }
                        if chain_site
                            && !signature_only
                            && let Some(tail) = body.last_mut()
                        {
                            Self::chain_site_set_shape(&ret, tail, *v);
                        }
                    }
                    RetPromotion::Rename {
                        buf_attr,
                        chain_site,
                    } => {
                        let n = self.vars.name(*v);
                        let def = &mut self.data.definitions[self.context as usize];
                        def.attributes[buf_attr].name = n.to_string();
                        def.attr_names.remove("__retbuf");
                        def.attr_names.insert(n.to_string(), buf_attr);
                        let placeholder = self.vars.var("__retbuf");
                        if placeholder != u16::MAX {
                            self.vars.retire_argument(placeholder);
                        }
                        self.vars.become_argument(*v);
                        dep.push(buf_attr as u16);
                        // #356: give the freshly bound mid-body site the
                        // explicit `Set + Var` shape.  Body-tail sites keep
                        // their NRVO / unify wiring untouched (wrapping there
                        // broke if-arm emission).
                        if chain_site && let Some(tail) = body.last_mut() {
                            Self::chain_site_set_shape(&ret, tail, *v);
                        }
                    }
                    RetPromotion::Bind {
                        buf_attr,
                        buf_var,
                        substitute,
                    } => {
                        if substitute {
                            for op in body.iter_mut() {
                                Self::substitute_work_ref(op, *v, buf_var);
                            }
                            // The substituted-out ref must not get a null-init
                            // preamble or a scope-exit free (see
                            // `unregister_work_ref`).
                            self.vars.unregister_work_ref(*v);
                            // A bare-call site tail needs its value made
                            // explicit (see `chain_site_set_shape`).
                            if let Some(tail) = body.last_mut() {
                                Self::chain_site_set_shape(&ret, tail, buf_var);
                            }
                        } else if let Some(tail) = body.last_mut() {
                            // Named local: keep its own store; deliver a COPY
                            // in the buffer at the return.  #425 — a
                            // struct-enum (heap `Type::Enum`) field-of-local
                            // return copies the same way as a Reference:
                            // `materialize_return_into` emits
                            // `OpCopyRecord(d.field → buf)` (the record copy
                            // works for any heap record, enum or struct).
                            // loft#938 — deliberately NOT `ret_promo_base` here.  This leg
                            // COPIES the tail into the buffer and answers the buffer, which
                            // for a nullable collection return would turn a `null` answer
                            // into an empty collection.  A nullable return that reaches
                            // this leg keeps its own store instead (loft#948 tracks the
                            // one-store-per-call leak that leaves on a `return <call>`
                            // forward); a conditional delivery is what would close it.
                            match ret.clone() {
                                Type::Reference(td, _) | Type::Enum(td, true, _) => {
                                    self.materialize_return_into(td, tail, buf_var);
                                }
                                // The CONDITIONAL delivery the note above names as what
                                // would close it.  A tail that is a branch with a `null` arm
                                // must not be copied WHOLE into the buffer: the wrap is
                                // `OpClearVector(buf); OpAppendVector(buf, <join>)`, which
                                // answers the buffer on EVERY path and evaluates the join
                                // AFTER the clear.  Three faults at once, all of them silent
                                // — the null arm delivered an EMPTY vector instead of the
                                // sentinel; an arm naming the buffer answered the buffer the
                                // clear had just emptied (`a = [1,2]; if k<0 { null } else if
                                // k==0 { a } else { [k] }` gave `[]`); and an arm that had
                                // already delivered into the buffer was appended to itself
                                // and came back DOUBLED (`[3,4,5,3,4,5]`).
                                // `materialize_vector_arms_into` delivers ONE ARM AT A TIME
                                // and leaves the buffer var and its views alone, so all
                                // three answer correctly.  It is the same per-arm machinery
                                // the join pre-pass above already uses.
                                //
                                // Found by loft#1096's boundary probes; loft#1097 with the
                                // matching half in `scopes::free_vars`, which keeps the
                                // sentinel reachable when frees force the tail into
                                // statement position.  Per-arm DELIVERY here, conditional
                                // RETURN there — two changes, one shape.
                                Type::Vector(elm, _)
                                    if crate::scopes::return_has_null_arm(
                                        tail,
                                        null_sentinel_nr,
                                    ) =>
                                {
                                    // No fall-back to the whole-tail copy: with a null arm
                                    // present it is wrong on every path, and "no arm to
                                    // deliver" here means every arm ALREADY answers the
                                    // buffer (each was delivered into it by its own leg) or
                                    // is the sentinel — both of which want nothing added.
                                    // Wrapping those was the doubling: a four-arm chain
                                    // answered `[3,4,5,3,4,5]` because the outer append put
                                    // the buffer into itself.
                                    self.materialize_vector_arms_into(&elm, tail, buf_var);
                                }
                                Type::Vector(elm, _) => {
                                    self.materialize_vector_return_into(&elm, tail, buf_var);
                                }
                                _ => {}
                            }
                        } else {
                            // No body tail to rewrite (defensive) — keep the
                            // local unpromoted; the return-copy path handles
                            // it.
                        }
                        if !dep.contains(&buf_attr) {
                            dep.push(buf_attr);
                        }
                    }
                    RetPromotion::Grow => {
                        debug_assert!(
                            self.first_pass
                                || self.data.def(self.context).name().contains("__lambda")
                                || self.data.def_type(self.context)
                                    != crate::data::DefType::Function,
                            "@PLAN59: arity grew in PASS 2 on plain fn '{}'",
                            self.data.def(self.context).name()
                        );
                        let n = self.vars.name(*v);
                        let a =
                            self.data
                                .add_attribute(&mut self.lexer, self.context, n, ret.clone());
                        // mark as hidden return-mechanism parameter
                        self.data.definitions[self.context as usize].attributes[a].hidden = true;
                        self.vars.become_argument(*v);
                        dep.push(a as u16);
                        // Growth here is lambda-only (asserted in the classify
                        // rationale): a lambda is defined at its literal site
                        // and invoked via CallRef (fn-ref dispatch, never an
                        // arity-filled Call), so no earlier caller can hold a
                        // short arg list — the #339 retro-patch this branch
                        // once needed is deleted (@PLAN59 phase 2).
                    }
                }
            }
            // A buffer-bound vector fn must deliver at EVERY return site —
            // callers (a forward caller in particular) can only consult the
            // signature, so they free the buffer alone and read the value
            // from it.  Mid-body `return <named local>` sites parsed BEFORE
            // the tail's promotion ran could not know the fn would bind
            // (vsort's base case: the legacy `__ref_1` injection missed its
            // `__ref_3`-named buffer and the leaf vectors leaked, #355
            // fallout) — rewrite them here, where the binding decision is
            // final and the full body is in hand.
            if site == RetSite::BlockTail
                && let Type::Vector(elm, _) = &ret
                && let Some((buf_attr, buf_var)) = self.return_buffer()
                && dep.contains(&buf_attr)
            {
                let elm = (**elm).clone();
                self.deliver_mid_vector_returns(&elm, body, buf_var);
                // #457 — deliver the IMPLICIT tail too. `deliver_mid_vector_returns`
                // rewrites `Return(Var(cv))` sites, but a body ending in an implicit
                // `cv` (no `return` keyword) leaves a bare `Var(cv)` tail it does not
                // touch. When `cv` was reassigned to a call-ADOPT in an arm
                // (`cv = recurse(.., __ref_N)`), `cv` holds a store distinct from
                // `buf_var`; returning it as-is was the #457 adopt (the callee then
                // freed the buffer it returned, fixed previously by a per-site
                // free thicket). Deliver `cv` into `buf_var` via the aliasing-safe
                // `OpReplaceVector` (a NO-OP when `cv` still aliases the buffer, so a
                // single-arm / non-reassigned tail is untouched — this is why it no
                // longer self-copies), so the fn ALWAYS returns its buffer and the
                // dep is accurate: no adopt, no per-site free derivation.
                let tail_cv = body.last().and_then(Self::tail_var);
                if let Some(cv) = tail_cv
                    && cv != buf_var
                    && matches!(self.vars.tp(cv), Type::Vector(_, _))
                    && Self::body_reassigns_var_to_call(body, cv)
                {
                    let rec_tp = self.append_elem_tp(&elm);
                    let replace = self.cl(
                        "OpReplaceVector",
                        &[Value::Var(buf_var), Value::Var(cv), Value::Int(rec_tp)],
                    );
                    if let Some(last) = body.last_mut() {
                        *last = Value::Insert(vec![replace, Value::Var(buf_var)]);
                    }
                }
                // Clear the buffer ON ENTRY: a caller's loop re-passes the
                // same fn-scoped buffer every iteration, and the NRVO
                // literal build (unlike the copy/injection sites) appends
                // without resetting — without this, each iteration's
                // result piles on top of the previous one (silent wrong
                // results, not just leaks).
                if let Some(first) = body.first_mut() {
                    let clear = self.cl("OpClearVector", &[Value::Var(buf_var)]);
                    let old = std::mem::replace(first, Value::Null);
                    *first = Value::Insert(vec![clear, old]);
                }
            }
            // @PLN85 D-own-2 — prune a STALE visible-param return dep that no pass-2
            // return source justifies.  `r = x` on a struct is a C86 COPY (`r` owns
            // its store), but the copy dep-strip is pass-2-only, so PASS 1 records the
            // source param `x` in the return dep and PASS 2 carries it via
            // `cur.clone()` above — into a pass where `r` is owned and `x` is NOT in
            // `expanded` (the pass-2 transitive return-source set).  A VISIBLE attr the
            // caller reads through `returns_borrowed_view()` MUST name a real pass-2
            // borrow source; a stale one makes the caller treat an owned copy return as
            // a borrow and never free it (the struct-copy-return native leak, interp
            // clean).  `expanded` already includes transitive deps, so a GENUINE borrow
            // (`fn id(x) -> Box { x }`, or a returned view of `x`) keeps `x`; only the
            // unreachable stale entry drops.  Hidden buffer attrs are never pruned.
            let stale_visible: Vec<u16> = {
                let attrs = self.data.def(self.context).attributes();
                dep.iter()
                    .copied()
                    .filter_map(|a| {
                        attrs
                            .get(a as usize)
                            .filter(|at| !at.hidden)
                            .map(|at| (a, at.name.clone()))
                    })
                    .collect::<Vec<_>>()
            }
            .into_iter()
            .filter(|(_, name)| {
                let var = self.vars.var(name);
                // NOT in the tail-source `expanded` set AND not borrowed by a
                // MID-BODY `return` either.  `expanded` is built from the TAIL
                // return sources, so it misses a param a mid-body `return` borrows
                // (`fn f(t) -> M { if c { return t[i] ?? d; } d }` — the `t` view is
                // genuine, not a stale copy-return artifact; 150-i306 native
                // corruption when it was wrongly pruned).
                !expanded.contains(&var) && !self.body_return_borrows(body, var)
            })
            .map(|(a, _)| a)
            .collect();
            dep.retain(|d| !stale_visible.contains(d));
            // H2: the rebuilt return-type deps are ATTRIBUTE indices —
            // tag them so `as_attr_indices` readers verify in debug builds.
            let dep = Deps::attrs(dep.to_vec());
            // loft#938 — rebuild the deps on the BASE and re-wrap, so a nullable return keeps
            // its `?`.  Matching `ret` directly refused `vector<T>?` outright once promotion
            // began reaching it: the deps belong to the storage and the `?` to the value, and
            // re-typing one must not drop the other.  Both calls are the identity when the
            // switch is off, so this arm reads as it always did.
            let rewrap = |t: Type| {
                if peel == crate::data::RetPeel::None {
                    t
                } else {
                    Type::optional(t)
                }
            };
            self.data.definitions[self.context as usize].returned = match dep_base.clone() {
                Type::Vector(it, _) => rewrap(Type::Vector(it, dep)),
                Type::Reference(td, _) => rewrap(Type::Reference(td, dep)),
                Type::Enum(td, true, _) => rewrap(Type::Enum(td, true, dep)),
                // loft#1140 — the five keyed kinds, rebuilt with their key lists intact.
                // Only the dep list is replaced; a keyed return is signature-only above, so
                // nothing here has moved the value.
                Type::Hash(td, k, _) => rewrap(Type::Hash(td, k, dep)),
                Type::Sorted(td, k, _) => rewrap(Type::Sorted(td, k, dep)),
                Type::Index(td, k, _) => rewrap(Type::Index(td, k, dep)),
                Type::Radix(td, k, _) => rewrap(Type::Radix(td, k, dep)),
                Type::Trie(td, k, _) => rewrap(Type::Trie(td, k, dep)),
                _ => {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Unexpected return type in ref_return: {}",
                        ret.name(&self.data)
                    );
                    return;
                }
            };
        }
    }

    // <return> ::= [ <expression> ]
    pub(crate) fn parse_return(&mut self, val: &mut Value) {
        // validate if there is a defined return value
        let mut v = Value::Null;
        let r_type = self.data.def(self.context).returned().clone();
        if !self.lexer.peek_token(";") && !self.lexer.peek_token("}") {
            // T1.7: save the position of the first token in the return expression,
            // used to report `not null` violations at the tuple literal site.
            let expr_start = self.lexer.peek();
            // @P365: a `return [ … ]` vector literal needs the function's return
            // type threaded in as the element-type hint — exactly as an assignment
            // threads its declared LHS type (parse_assign_op → parse_operators).
            // Without it an EMPTY `return []` types as Unknown, skips the
            // Vector-construction lowering below, and emits `return ()` (native,
            // E0308) / a garbage DbRef (interpret).  Gated to a `[`-led literal
            // returned from a vector-typed fn so every other return keeps the
            // existing `expression` path verbatim (for a literal, `expression`
            // already reduces to `parse_operators(Unknown)` — only the hint differs).
            // loft#703 — a KEYED return type threads the same way: `return [K { … }]`
            // infers `vector<K>` on its own, so without the hint there is no way to
            // return a keyed collection built in place.
            let t = if crate::parser::vectors::is_collection(&r_type) && self.lexer.peek_token("[")
            {
                // Thread the element type but NOT the return type's dep: a
                // vector-returning fn carries `[__ref_1]` as its dep, and
                // inheriting that on the literal would fool the `Type::Vector`
                // arm below (`!dep.contains(ref1_var)`) into skipping the
                // OpAppendVector copy into __ref_1.  Element type only.
                let hint = r_type.without_deps();
                let mut parent_tp = Type::Null;
                self.parse_operators(&hint, &mut v, &mut parent_tp, 0)
            } else {
                self.expression(&mut v)
            };
            if r_type == Type::Void {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Expect no expression after return"
                );
                *val = Value::Return(Box::new(Value::Null));
                return;
            }
            // Enforces @FR-G-Return — the VALUED spelling.  @FR-G-Call makes any function
            // returning `iterator<T>` a generator, and in that model values leave only
            // through `yield`; @FR-G-Done says reaching the end without a further yield
            // leaves the iterator done.  So a returned value has nothing it could mean, and
            // ending early is what `break` already does.
            //
            // ⚠ Accepting it is not a lenient option — the value is DISCARDED, and silently.
            // `fn make() -> iterator<integer> { return counting(1); }` (an author delegating
            // to another generator) then yields an EMPTY sequence on `--interpret` with no
            // diagnostic, and faults inside `alloc_coroutine` on `--native`.  Refusing is
            // what binding.md's B-Ref-Reshape prescribes for the same situation elsewhere:
            // where the language cannot honour what was written, it declines the program
            // rather than answering quietly.
            //
            // The message names `break` and NOT a bare `return;` because only `break` is a
            // working cure — `detect_lazy_for` (generation/coroutine.rs) rejects a `return`
            // outright, so such a generator falls back to the eager buffer, whose factory
            // cannot emit a mid-body return either (its type is `Box<dyn LoftCoroutine>`).
            if !self.first_pass && matches!(r_type, Type::Iterator(_, _)) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "a generator has no `return` — it produces values only through `yield`, \
                     so this value would be discarded. Use `break` to end it early, or \
                     forward another generator's values with \
                     `for v in <generator>() {{ yield v; }}`"
                );
                *val = Value::Return(Box::new(Value::Null));
                return;
            }
            // T1.7: check for null assigned to `integer not null` tuple elements.
            if !self.first_pass
                && let (Value::Tuple(elems), Type::Tuple(expected)) = (&v, &r_type)
            {
                for (elem_val, elem_tp) in elems.iter().zip(expected.iter()) {
                    if matches!(elem_val, Value::Null)
                        && matches!(elem_tp, Type::Integer(IntegerSpec { not_null: true, .. }))
                    {
                        specific!(
                            &mut self.lexer,
                            &expr_start,
                            Level::Error,
                            "cannot assign null to 'integer not null' element"
                        );
                    }
                }
            }
            // @P374: mirror block_result's tuple→synthetic-struct rewrite for an
            // explicit `return (a, b);` whose declared return type is a
            // `Reference(__tuple<…>)` (a tuple of types with lifetime concerns —
            // e.g. structs — which `parse_function` rewrites that way).  Without
            // it, `convert(Tuple, Reference(__tuple<…>))` fails and the user sees
            // "expected __tuple<…>, got (…)" even though the SAME tuple as a
            // function's final expression compiles.  parse_return is the statement
            // path; block_result is the tail path — they must agree.
            let tuple_rewritten = !self.first_pass
                && matches!(t, Type::Tuple(_))
                && tail_has_tuple_leaf(v.unspan(), &self.vars)
                && matches!(&r_type, Type::Reference(d, _) if self.data.def(*d).name().starts_with("__tuple<"))
                && {
                    let synthetic_d_nr = if let Type::Reference(d, _) = &r_type {
                        *d
                    } else {
                        unreachable!()
                    };
                    self.rewrite_tail_tuple_to_synthetic_struct(synthetic_d_nr, &mut v);
                    true
                };
            // @FR-N-Store: an explicit `return` is a STORE into the caller's non-null return
            // slot.  The store face asks where the value converts; the two lowerings that do
            // not convert — a bare `null` (the sentinel) and a tuple rewritten into its
            // synthetic struct — are asked at the site.
            if t == Type::Null || tuple_rewritten {
                self.n_store_violation(&t, &r_type, "the return value", None);
            }
            if t == Type::Null {
                v = self.null_value(&r_type);
            } else if !tuple_rewritten
                && !self.convert_store(&mut v, &t, &r_type, "the return value", None)
            {
                self.validate_convert("return", &t, &r_type, &expr_start.position);
            }
            // loft#822 — `convert` can UNBOX a stored tuple into its stack spelling, and
            // `t` still names the spelling the value had BEFORE that.  The delivery
            // classification below branches on `t`'s kind: left stale it sees a
            // `Reference` where the value is now a stack tuple, decides the return views
            // a local, and materialises it with `OpCopyRecord` — which reads the tuple's
            // own float bytes as a DbRef (`return p` from `for p in v` SIGSEGV'd).
            // The unbox already IS the copy the materialisation wanted: it loads each
            // element by value.  Only a tuple whose declared return stayed the STACK
            // spelling reaches here — a heap-carrying tuple is `Reference(__tuple<…>)` on
            // both sides (`tuple_return_rewrite`), so no conversion happens and its
            // owned-copy delivery is untouched.
            let t = if self.unboxes_stored_tuple(&t, &r_type) {
                r_type.clone()
            } else {
                t
            };
            // Phase 1b (inline-lift-safety): mirror block_result's ref/enum
            // merge for mid-body `return` statements.  Without this, a function
            // like `fn f(c) -> Inner { if ... return c.items[i]; Inner{} }` loses
            // the `[c]` dep from the mid-body return path (only the owned-fresh
            // tail reaches block_result), and codegen's 0x8000 gate misses at
            // the call site → caller store corruption.  Skip for generic
            // templates (same I9-var rationale as block_result line 340).
            // Vector arm deliberately not mirrored: mid-body Vector returns can
            // reference globals/locals which ref_return would promote to hidden
            // ref args, breaking callers (see 01b for full analysis).
            // #355: set when the new one-buffer vector arm below handled
            // this site — the legacy `__ref_1` OpAppendVector injection
            // further down must then NOT fire a second copy.
            let mut vector_bound = false;
            if self.data.def_type(self.context) != DefType::Generic {
                // Explicit `return <expr>;`: the full body is not available
                // here, so pass the return expression itself as a one-element
                // body — `ref_return`'s one-buffer binding substitutes /
                // copy-rewrites inside it (the reassignment guard does not
                // apply: explicit return already copies the value).
                // H12 — a vector-element read types as `Optional(τ)` (`v[i]` is
                // `τ?`), and every delivery arm below matches a DENSE form.  So
                // `return b.cells[i]` from a `-> Cell` fn matched NO arm: nothing
                // bound the value to `__retbuf`, and the IR came out as the element
                // read evaluated for effect followed by `return null` — uniformly
                // null fields, which read as "absent" rather than "broken".  Peel
                // the marker for the delivery decision; `convert` above has already
                // discharged the nullability against the dense declared return.
                let t = match &t {
                    Type::Optional(inner) => (**inner).clone(),
                    other => other.clone(),
                };
                if let Type::Reference(td, ls) = &t {
                    if self.return_projects_into_local(&v) {
                        // The returned expression points INTO something this
                        // function frees — a field of an inline call's temporary
                        // (#425 / H9), or an element of a local's vector (H12).
                        // Copy the record into an owned buffer first (the same
                        // owned-copy `return d.field` gets from `ref_return`'s
                        // named-local leg).
                        let w = self.materialize_view_return(*td, &mut v);
                        self.ref_return(&[w], std::slice::from_mut(&mut v), RetSite::MidReturn);
                    } else if ls.is_empty() {
                        let extra = Self::collect_hidden_ref_args(&v, &self.data);
                        if !extra.is_empty() {
                            let ls_own = extra.clone();
                            self.ref_return(
                                &ls_own,
                                std::slice::from_mut(&mut v),
                                RetSite::MidReturn,
                            );
                        }
                    } else if self.return_views_local(ls) || !self.ls_can_be_record_buffer(ls) {
                        // #306: mid-body `return <view of a local>` — copy it
                        // into an owned work-ref before it escapes (mirrors
                        // block_result's tail handling).  The shape guard mirrors
                        // it too: `return make(n)[0] ?? d;` leaves the indexed
                        // CONTAINER in `ls`, and a `vector<Cell>` cannot be the
                        // buffer for a `Cell` return (loft#877).
                        let w = self.materialize_view_return(*td, &mut v);
                        self.ref_return(&[w], std::slice::from_mut(&mut v), RetSite::MidReturn);
                    } else {
                        let ls_own: Vec<u16> = ls.to_vec();
                        self.ref_return(&ls_own, std::slice::from_mut(&mut v), RetSite::MidReturn);
                    }
                } else if let Type::Enum(e_d, true, ls) = &t {
                    // @PLN25 single-payload: a mid-body `return <nullable-element>` whose value is
                    // coerced to a dense `S` keeps `t` as the synth `__nullable<S>` Enum tail type,
                    // while the fn's DECLARED return is a dense `Reference(S)`.  The value is a VIEW
                    // into a local (the unwrapped payload) — left as a view it makes the fn
                    // VIEW-classified, so a SIBLING OWNED return on another path (a fallback `S{}`)
                    // is never freed by the caller (149: `map_get_hex` fallback `make_hex(0,0)` × N
                    // leaks).  Detect it from the TYPES (a `__nullable<S>` Enum tail + dense
                    // `Reference(S)` declared) — NOT the IR source, so a DIRECT `v[i]` unwrap
                    // qualifies too — and copy the view into an OWNED buffer so the fn is
                    // owned-classified.  No `nrvo_collapse_tail_set` (its work-ref→caller-buffer
                    // rename + re-OpDatabase was the documented direct-`v[i]` free-list-corruption
                    // hazard, now also defused by zero-on-claim; the plain copy here does not
                    // rename).  Gated on `__nullable<>` so a real user struct-enum return is
                    // untouched.
                    let declared_ret = self.data.def(self.context).returned().clone();
                    if let Type::Reference(rtd, _) = declared_ret
                        && self.data.def(*e_d).name().starts_with("__nullable<")
                    {
                        let w = self.materialize_view_return(rtd, &mut v);
                        self.ref_return(&[w], std::slice::from_mut(&mut v), RetSite::MidReturn);
                    } else if self.return_projects_into_local(&v) {
                        // #425 sibling — `return mk().field` where `field` is a
                        // struct-enum (heap record): the inline-call base is freed
                        // at scope exit, so copy the field's record into an owned
                        // buffer first.  The Reference arm above does the same for
                        // a struct field; this is the struct-enum twin.
                        let ed = *e_d;
                        let w = self.materialize_view_return(ed, &mut v);
                        self.ref_return(&[w], std::slice::from_mut(&mut v), RetSite::MidReturn);
                    } else if self.return_views_local(ls) {
                        // #306's payload-enum twin.  The predicate above reads the
                        // returned EXPRESSION, so it sees `return e.value` but not
                        // `r = e.value; return r` — there the tail is a bare `Var`,
                        // and the fact that `r` points into the local `e` lives in
                        // its deps, which is what `return_views_local` reads.  The
                        // Reference arm has carried this leg since #306; without it
                        // here the payload-enum twin renamed `r` onto `__retbuf`,
                        // so the caller got a pointer into a store this function
                        // frees on the way out and read a corrupt record.
                        let ed = *e_d;
                        let w = self.materialize_view_return(ed, &mut v);
                        self.ref_return(&[w], std::slice::from_mut(&mut v), RetSite::MidReturn);
                    } else {
                        let ls_own: Vec<u16> = ls.to_vec();
                        self.ref_return(&ls_own, std::slice::from_mut(&mut v), RetSite::MidReturn);
                    }
                } else if crate::parser::vectors::is_keyed(&t) {
                    // @FR-O-Move, the same clause the tail arm records — this is the second
                    // entry point that has to record it.
                    //
                    // loft#1140, the EXPLICIT-return twin of the keyed arm in
                    // `block_result`.  `return h;` never reaches that one — the tail
                    // dispatch sees a bare `Var`, and an explicit return is routed here
                    // instead — so a function written `{ return h; }` kept an empty return
                    // dep list while `{ h }` recorded the borrow, and only the first
                    // spelling still freed the caller's collection.  `LOFT_TRACE_RETPROMO`
                    // printed no ENTER line at all for it, which is what named the second
                    // entry point.
                    //
                    // Same treatment as the tail: hand `ref_return` the returned value's
                    // deps so `MergeAttr` records any borrowed parameter, and let it make
                    // no placement decision (a keyed return is signature-only there).
                    let ls_own: Vec<u16> = t.depend();
                    self.ref_return(&ls_own, std::slice::from_mut(&mut v), RetSite::MidReturn);
                } else if let Type::Vector(_, ls) = &t
                    && self.return_buffer().is_some()
                {
                    // #355: a mid-body VECTOR return whose value comes from
                    // a CALL (a site work ref backs it) binds to the one
                    // buffer; `RetSite::MidReturn` keeps `ref_return` from
                    // renaming a site local into the fn-wide buffer (the
                    // 01b hazard that kept this arm un-mirrored).  Literal /
                    // named-local returns keep the legacy `__ref_1` append
                    // path below — its element-copy handles nested rows,
                    // which a plain buffer append would shallow-copy.
                    // loft#938 — UNION the hidden buffer args with the deps rather than
                    // preferring the deps.  A nullable collection callee whose own return
                    // already names its `__retbuf` resolves to a dep here, so the
                    // `ls.is_empty()` branch was skipped and the site's own `__ref_N` never
                    // entered the list — the site then bound nothing, `unregister_work_ref`
                    // never ran, and the work-ref leaked one store per call on a
                    // `return <call>` forward.  Adding to the list can only widen what the
                    // filter below considers; it still keeps only `__ref_`/`__rref_` names.
                    let mut ls_own: Vec<u16> = ls.to_vec();
                    for w in Self::collect_hidden_ref_args(&v, &self.data) {
                        if !ls_own.contains(&w) {
                            ls_own.push(w);
                        }
                    }
                    let site_refs: Vec<u16> = ls_own
                        .iter()
                        .copied()
                        .filter(|w| {
                            let nm = self.vars.name(*w);
                            nm.starts_with("__ref_") || nm.starts_with("__rref_")
                        })
                        .collect();
                    if !site_refs.is_empty() {
                        vector_bound = true;
                        self.ref_return(
                            &site_refs,
                            std::slice::from_mut(&mut v),
                            RetSite::MidReturn,
                        );
                    }
                }
            }
            if let Type::Text(ls) = &t {
                self.text_return(ls);
            } else if !self.first_pass {
                // When a function returns a vector and the caller provides an output
                // buffer (__ref_1 as a function argument), an explicit `return expr`
                // where `expr` is backed by a local __vdb_N store would return a
                // dangling DbRef: __vdb_N is freed before the return.
                //
                // Fix: if __ref_1 is a function argument and the returned expression
                // is NOT already backed by __ref_1 (dep does not contain ref1_var),
                // inject OpAppendVector to copy the elements into __ref_1 and return
                // __ref_1 instead.
                if let Type::Vector(elm_tp, dep) = &t {
                    let ref1_var = self.vars.var("__ref_1");
                    // `__ref_1` is the promoted-local name after ref_return renames
                    // the signature-time `__retbuf` placeholder.  When a function
                    // returns a PARAMETER directly (`return v`) without going through
                    // ref_return (because the parameter is not a work-ref), the
                    // buffer stays named `__retbuf` and vars.var("__ref_1") returns
                    // MAX.  Fall back to return_buffer() only when the returned value
                    // is backed by a PARAMETER variable — a fresh LOCAL vector
                    // (`return o`) is NOT delivered here: copying it into __retbuf
                    // would orphan the local on a MID-BODY return (it never reaches
                    // its scope-free).  A fresh-local TAIL return is instead promoted
                    // by `block_result`'s #437 tail-intercept (strip the `return`,
                    // route through the implicit-tail ref_return + NRVO — no copy).
                    // (a, _) keeps the buffer-attr index for the #437 dep finalize.
                    let (buf_attr, buf_var) =
                        if ref1_var != u16::MAX && self.vars.is_argument(ref1_var) {
                            (self.return_buffer().map_or(u16::MAX, |(a, _)| a), ref1_var)
                        } else if let Some((a, bv)) = self.return_buffer()
                            && (dep.iter().any(|&d| d != bv && self.vars.is_argument(d))
                                // #488: a field VIEW rooted at a non-argument LOCAL
                                // (`return r.pts` — the struct local is freed at scope
                                // exit) needs the same element-copy into the caller's
                                // buffer as the field-of-param case; without it the value
                                // was emitted as a discarded statement and the fn returned
                                // null (empty on native — the interpreter masked it by
                                // reading top-of-stack, with a UAF read + a store leak).
                                // Shape-matched, NOT dep-matched: a `return match {…}` /
                                // fresh-local return also carries local deps but must keep
                                // its NRVO delivery (its construction block cannot sit in
                                // OpAppendVector's argument position).
                                //
                                // The predicate covers a projection rooted at an inline
                                // CALL's temporary too (`return mk().lines`), which #488's
                                // Var-only version missed — the third sibling of the same
                                // defect, after the struct (#425) and element (H12) forms.
                                // The lift temp is freed at scope exit, so the returned
                                // vector aliased a freed store: empty on native, and on the
                                // interpreter a live value that a LATER allocating call in
                                // the caller transiently clobbered to length 0.  Reported
                                // by the zero-trust consumer as a one-line accessor that
                                // silently corrupted its result.
                                || self.return_projects_into_local(&v))
                        {
                            (a, bv)
                        } else {
                            (u16::MAX, u16::MAX)
                        };
                    if !vector_bound && buf_var != u16::MAX && !dep.contains(&buf_var) {
                        // @P314 — narrow-aware element type (see `append_elem_tp`).
                        let elm = (**elm_tp).clone();
                        let rec_tp = self.append_elem_tp(&elm);
                        // Clear first: delivery REPLACES the buffer content
                        // (a caller's loop reuses the same fn-scoped buffer;
                        // without the clear each iteration's elements pile
                        // on top of the previous ones).
                        let clear = self.cl("OpClearVector", &[Value::Var(buf_var)]);
                        let append = self.cl(
                            "OpAppendVector",
                            &[Value::Var(buf_var), v, Value::Int(rec_tp)],
                        );
                        *val = Value::Insert(vec![
                            clear,
                            append,
                            Value::Return(Box::new(Value::Var(buf_var))),
                        ]);
                        // #437 — finalize the return-type dep to {__retbuf}, the step
                        // the implicit-tail path does (fwd_copy_409, ~825) and this
                        // explicit path omitted.  An arg / struct-field return
                        // (`return v` / `return b.v`) already element-copied its value
                        // INTO __retbuf above, but left the SIGNATURE a bare vector —
                        // so a caller (which consults only the signature) rebound its
                        // result var to a fresh empty store and the first in-place
                        // `+=` DROPPED the returned elements (#437).  Finalizing the
                        // dep makes the caller bind to the buffer it passed, so the
                        // result owns an appendable store and `+=` grows it in place.
                        if buf_attr != u16::MAX {
                            self.data.definitions[self.context as usize].returned =
                                Type::Vector(Box::new(elm), Deps::attrs(vec![buf_attr]));
                        }
                        return;
                    }
                }
                // @PLAN51 probe 39 — Reference parallel of the Vector
                // arm above.  A function returning a heap struct that
                // has been ref_return-promoted to a caller-side hidden
                // buffer (`__ref_1`) leaks ONE store per mid-body
                // `return borrowed_slice` when the returned DbRef is
                // NOT backed by `__ref_1`.  `OpReturn` then writes the
                // borrowed 12-byte DbRef into the caller's buffer slot
                // — orphaning the buffer's pre-allocated store.
                //
                // Pattern: `for x in vec { ... return x.field[i]; } default`.
                // probe 39's `map_get_hex` is the canonical case
                // (lib/moros_map's deep-slice borrow).
                //
                // Fix: deep-copy the borrowed slice into `__ref_1` via
                // OpCopyRecord, then return `__ref_1`.  Mirrors the
                // Vector arm's OpAppendVector treatment.
            }
        } else if !self.first_pass && matches!(r_type, Type::Iterator(_, _)) {
            // Enforces @FR-G-Return — the BARE spelling, which the rule names alongside the
            // valued one above, and which gets the same message.
            //
            // ⚠ It needs its own arm even though the generic path already rejects it.  That
            // path reports "Expect expression after return", because it reads "the declared
            // type is not Void" as "a value is required" — and a generator's declared type
            // is `iterator<T>`.  So the generic message sends the author to ADD a value,
            // which is the one thing this rule forbids.
            diagnostic!(
                self.lexer,
                Level::Error,
                "a generator has no `return` — use `break` to end it early, or forward \
                 another generator's values with `for v in <generator>() {{ yield v; }}`"
            );
        } else if !self.first_pass && r_type != Type::Void {
            diagnostic!(self.lexer, Level::Error, "Expect expression after return");
        }
        *val = Value::Return(Box::new(v));
    }

    /// Parse an assert or panic keyword call: `assert(expr, msg)` / `panic(msg)`.
    /// The opening `(` is consumed by the caller; this function parses args and `)`.
    pub(crate) fn parse_intrinsic_call(&mut self, val: &mut Value, name: &str) -> Type {
        let call_pos = self.lexer.pos().clone();
        let mut list = Vec::new();
        let mut types = Vec::new();
        if !self.lexer.has_token(")") {
            loop {
                let mut p = Value::Null;
                let t = self.expression(&mut p);
                types.push(t);
                list.push(p);
                if !self.lexer.has_token(",") {
                    break;
                }
            }
            self.lexer.token(")");
        }
        let ret = self.parse_call_diagnostic(val, name, &list, &types, &call_pos);
        // Plan-07 phase 1, step 1.13 — wrap intrinsic-keyword calls
        // (`assert(...)`, `panic(...)`) at the `(` token so runtime
        // failure inside `n_panic` / `n_assert` carries the call site
        // position into `state.source_spans`.  Mirrors the wrap in
        // `parse_call` for the regular fn-call dispatch path.
        if !self.first_pass && matches!(val, Value::Call(_, _) | Value::CallRef(_, _)) {
            let inner = std::mem::replace(val, Value::Null);
            *val = Value::with_span(call_pos, inner);
        }
        ret
    }

    /// Extract the assert condition expression from the source line.
    /// Reads the line at `pos.file:pos.line`, finds `assert(`, and extracts
    /// the text up to the matching `)`.
    fn extract_assert_expr(pos: &crate::lexer::Position) -> String {
        let line = Self::read_source_line(&pos.file, pos.line);
        // Find "assert(" and extract the condition
        if let Some(start) = line.find("assert(") {
            let after = start + 7; // skip "assert("
            let bytes = line.as_bytes();
            let mut depth = 1;
            let mut end = after;
            while end < bytes.len() && depth > 0 {
                match bytes[end] {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    b'"' => {
                        // Skip string literals
                        end += 1;
                        while end < bytes.len() && bytes[end] != b'"' {
                            if bytes[end] == b'\\' {
                                end += 1;
                            }
                            end += 1;
                        }
                    }
                    _ => {}
                }
                if depth > 0 {
                    end += 1;
                }
            }
            let expr = line[after..end].trim();
            // If it contains a comma, only take up to the first top-level comma
            // (the rest is the user message argument).
            let mut comma_depth = 0;
            for (i, b) in expr.bytes().enumerate() {
                match b {
                    b'(' | b'[' | b'{' => comma_depth += 1,
                    b')' | b']' | b'}' => comma_depth -= 1,
                    b',' if comma_depth == 0 => return expr[..i].trim().to_string(),
                    b'"' => {
                        // skip — don't count commas inside strings
                        // (simplified: the expression without message has no commas at top level)
                    }
                    _ => {}
                }
            }
            expr.to_string()
        } else {
            "assert failure".to_string()
        }
    }

    /// Read a single source line from a file (or VirtFS under WASM).
    fn read_source_line(file: &str, line: u32) -> String {
        #[cfg(feature = "wasm")]
        {
            if let Some(content) = crate::wasm::virt_fs_get(file) {
                return content
                    .lines()
                    .nth(line as usize - 1)
                    .unwrap_or("")
                    .to_string();
            }
        }
        if let Ok(content) = std::fs::read_to_string(file) {
            content
                .lines()
                .nth(line as usize - 1)
                .unwrap_or("")
                .to_string()
        } else {
            String::new()
        }
    }

    // <call> ::= [ <expression> { ',' <expression> } ] ')'
    pub(crate) fn parse_call_diagnostic(
        &mut self,
        val: &mut Value,
        name: &str,
        list: &[Value],
        types: &[Type],
        call_pos: &Position,
    ) -> Type {
        if name == "assert" {
            let mut test = list[0].clone();
            // A CONDITION, not an ordinary argument — LOFT.md § Conversions names `assert`
            // beside `if` and `while` for the any-type coercion, and a plain `convert` left
            // a heap handle raw here: the interpreter accepted `assert(v)` while `--native`
            // refused to compile it (`(DbRef) as u8`), which is one program and two drivers
            // disagreeing about whether it is a program at all.
            self.convert_condition(&mut test, &types[0]);
            let message = if list.len() > 1 {
                list[1].clone()
            } else {
                // Extract the assert expression from the source line.
                let expr = Self::extract_assert_expr(call_pos);
                Value::str(&expr)
            };
            if self.first_pass {
                *val = Value::Null;
                return Type::Void;
            }
            // loft#1147 — the declared signature carries `file` / `line` and the doc says
            // *"do not pass them manually"*, which is right for a call in user code and
            // wrong for the one case that must: a stdlib FORWARDER (`assert_eq`) has
            // already been handed the caller's position and has to hand it on, or every
            // failure it reports names `01_code.loft` instead of the test.  When the two
            // are supplied they are honoured; the injection stays the default.
            let (a_file, a_line) = if list.len() >= 4 {
                (list[2].clone(), list[3].clone())
            } else {
                (Value::str(&call_pos.file), Value::Int(call_pos.line as i32))
            };
            let d_nr = self.data.def_nr("n_assert");
            *val = Value::Call(d_nr, vec![test, message, a_file, a_line]);
            Type::Void
        } else if name == "panic" {
            let message = if list.is_empty() {
                Value::str("panic")
            } else {
                list[0].clone()
            };
            if self.first_pass {
                *val = Value::Null;
                return Type::Void;
            }
            let d_nr = self.data.def_nr("n_panic");
            *val = Value::Call(
                d_nr,
                vec![
                    message,
                    Value::str(&call_pos.file),
                    Value::Int(call_pos.line as i32),
                ],
            );
            Type::Void
        } else {
            // log_info / log_warn / log_error / log_fatal
            let message = if list.is_empty() {
                Value::str("")
            } else {
                list[0].clone()
            };
            if self.first_pass {
                *val = Value::Null;
                return Type::Void;
            }
            let fn_name = format!("n_{name}");
            let d_nr = self.data.def_nr(&fn_name);
            *val = Value::Call(
                d_nr,
                vec![
                    message,
                    Value::str(&call_pos.file),
                    Value::Int(call_pos.line as i32),
                ],
            );
            Type::Void
        }
    }

    #[allow(clippy::too_many_lines)] // pre-existing length; A5.6b.2 added ~9 lines
    pub(crate) fn parse_call(
        &mut self,
        val: &mut Value,
        source: u16,
        name: &str,
        name_pos: &Position,
    ) -> Type {
        let call_pos = self.lexer.pos().clone();
        let mut list = Vec::new();
        let mut types = Vec::new();
        let mut arg_pos: Vec<Position> = Vec::new();
        if self.lexer.has_token(")") {
            // Check for zero-argument fn-ref call
            if self.vars.name_exists(name) {
                let v_nr = self.vars.var(name);
                if let Type::Function(param_types, ret_type, _) = self.vars.tp(v_nr).clone()
                    && param_types.is_empty()
                {
                    // @PLN85 L1 — callee-attr-space deps must not leak into the
                    // caller (see `fnref_result_type`), and an index naming no visible
                    // argument names the closure this slot carries (loft#1180).
                    let ret_type = Box::new(Self::fnref_result_type(
                        *ret_type,
                        &[],
                        Self::capturing_fnref_var(&self.vars, v_nr),
                    ));
                    // P227: a text-returning fn-ref call carries its target's `&text`
                    // work buffers at caller-function scope, because a `&text` is a
                    // pointer into the CALLER's frame — the callee cannot conjure one
                    // that outlives its own return.  The call site cannot know which
                    // function the slot holds, so it pushes what the widest candidate of
                    // this signature wants and `State::fn_call_ref` pops the excess once
                    // the target IS known (loft#1116).
                    let work_vars = self.fnref_text_buffer_vars(0, ret_type.as_ref());
                    if !self.first_pass {
                        self.var_usages(v_nr, true);
                        let mut args = vec![];
                        // inject work-buffer DbRef blocks before __closure (zero-param case).
                        // clear the work buffer before each call so loop iterations start fresh.
                        self.push_fnref_text_buffers(&mut args, &work_vars);
                        // closure is embedded in the 16-byte fn-ref slot; fn_call_ref
                        // pushes it automatically — no explicit injection needed here.
                        // mark captured vars as read at the call site
                        for &cv in &std::mem::take(&mut self.last_closure_captured_vars) {
                            self.var_usages(cv, true);
                        }
                        *val = Value::CallRef(v_nr, args);
                    }
                    return *ret_type;
                }
            }
            return self.call(val, source, name, &list, &Vec::new(), &[], &[], name_pos);
        }
        let fn_def_nr = if self.first_pass {
            None
        } else {
            let d_nr = self.data.def_nr(&format!("n_{name}"));
            (d_nr != u32::MAX).then_some(d_nr)
        };
        let mut arg_idx = 0usize;
        let mut named_args: Vec<(String, Value, Type)> = Vec::new();
        let mut in_named = false;
        loop {
            // @F17 — collect named arguments (`name: expr`) at the call site
            // Check for named argument: `name: expr`
            if let Some(arg_name) = self.lexer.peek_named_arg() {
                in_named = true;
                self.lexer.has_identifier(); // consume name
                self.lexer.has_token(":"); // consume :
                // #432 — a named vector-literal argument (`f(v: [10, 255, 20])`)
                // builds at the parameter's element width too.  Map the name to its
                // parameter to seed the hint, then clear it after parsing.
                let hint_d_nr = self.data.def_nr(&format!("n_{name}"));
                if hint_d_nr != u32::MAX {
                    for a in 0..self.data.attributes(hint_d_nr) {
                        if self.data.attr_name(hint_d_nr, a) == arg_name {
                            let expected = self.data.attr_type(hint_d_nr, a);
                            // loft#1067 — `takes(f: |x| { x * 2 })` names the same
                            // parameter the positional form does, so it must infer the
                            // same way; the spelling of the argument is not the axis.
                            if Self::seeds_collection_hint(&expected)
                                || self.interpolation_target(&expected) != u32::MAX
                                || Self::seeds_lambda_hint(&expected)
                            {
                                self.expected = expected;
                            } else if let Some(tuple) = self.tuple_hint_type(&expected) {
                                self.expected = tuple;
                            }
                            break;
                        }
                    }
                }
                let mut p = Value::Null;
                let t = self.expression(&mut p);
                self.expected = Type::Unknown(0);
                named_args.push((arg_name, p, t));
                // accept trailing comma on the last named arg.
                if !self.lexer.has_token(",") || self.lexer.peek_token(")") {
                    break;
                }
                continue;
            }
            if in_named && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Positional argument after named argument"
                );
            }
            // The `⇐` push belongs to THIS argument. Whatever is still in the
            // channel is the ENCLOSING call's expectation, and a nested call must
            // not inherit it: in `take(build_one("arg"))` the string is
            // `build_one`'s `text` parameter, but the channel still held `take`'s
            // parameter type, so the literal was checked against the wrong one.
            // The hints below each set it when they apply; none of them cleared it
            // when they did not.
            self.expected = Type::Unknown(0);
            if let Some(d_nr) = fn_def_nr
                && arg_idx < self.data.attributes(d_nr)
            {
                let expected = self.data.attr_type(d_nr, arg_idx);
                if matches!(expected, Type::Function(_, _, _)) {
                    self.expected = expected;
                }
            }
            // @PLN22 Phase 1 — hint the expected enum so a bare value-position
            // variant argument (`f(Red)`) resolves against the parameter's enum.
            // Resolved on BOTH passes (unlike the pass-2-only `fn_def_nr` above):
            // on pass 1 the callee is already registered, and skipping the hint
            // there would let a bare variant become a stray placeholder var that
            // shadows the real variant on pass 2.
            if !in_named {
                // The parameter type this argument is checked against, from whichever
                // spelling of the callee is in scope: a NAMED function's attribute, or a
                // fn-ref VARIABLE's parameter list.  Both answer "the callee's parameter at
                // this position", and reading only the named one is why the whole chain
                // below was unreachable through a lambda — `axis(North)` resolved the bare
                // variant against `fn axis(d: D)` while `lam(North)` was refused for the
                // IDENTICAL declared parameter type, with a message whose cure ("give the
                // target an enum type") the target already satisfied (loft#1280).  It is
                // the fn-ref call-site position of loft#1122's family.
                let hint_d_nr = self.data.def_nr(&format!("n_{name}"));
                let hinted = if hint_d_nr != u32::MAX && arg_idx < self.data.attributes(hint_d_nr) {
                    Some(self.data.attr_type(hint_d_nr, arg_idx))
                } else {
                    self.fnref_param_hint(name, arg_idx)
                };
                if let Some(expected) = hinted {
                    if Self::seeds_lambda_hint(&expected) {
                        // A `fn(…)` parameter, so a SHORT-form lambda argument can infer its
                        // parameter types — the fn-ref position of the push the `fn_def_nr`
                        // block above makes for a named callee.
                        //
                        // This arm was held CLOSED when loft#1280 landed, because seeding it
                        // made the short form parse and land in a dispatch that could not
                        // carry a fn-ref argument at all (loft#1285: no output and exit 0 on
                        // `--interpret`, E0308 on `--native`).  With that dispatch fixed —
                        // the 20-byte pair at the interpreter's call site, the `fn_ref_context`
                        // binding in the native emitter, and the `CallRef` arm in the
                        // reachability walk — the refusal has nothing left to protect.
                        self.expected = expected;
                    } else if self.enum_context(&expected) {
                        self.expected = expected;
                    } else if Self::seeds_collection_hint(&expected) {
                        // #432 — seed a bare vector-literal argument's element width
                        // from the parameter type, so it builds at the callee's
                        // stride instead of `vector<integer>`.  Both passes (like the
                        // enum hint): the literal's element type must agree across
                        // passes, and the callee is already registered on pass 1.
                        self.expected = expected;
                    } else if self.interpolation_target(&expected) != u32::MAX {
                        // @PLN124 — seed a format-string argument's target type, so
                        // `f("… {x} …")` BUILDS the parameter's type instead of
                        // rendering text the call would then reject. Both passes, for
                        // the same reason the two hints above are: taking the branch
                        // mints an accumulator, and a one-pass mint would shift the
                        // name-keyed variable tables.
                        self.expected = expected;
                    } else if let Some(tuple) = self.tuple_hint_type(&expected) {
                        // loft#1122 — seed a tuple argument's MEMBER types, so
                        // `f(([], 9))` and `f((Dot, 9))` resolve against the parameter
                        // the way the same literal does in a declared local.  Both
                        // passes, for the reason the enum hint above states: a bare
                        // variant seeded on one pass only becomes a stray placeholder
                        // var that shadows the real variant on the other.
                        self.expected = tuple;
                    }
                }
            }
            // for map/filter/reduce, infer lambda hint from the vector
            // element type so that short-form |x| lambdas can infer types.
            if fn_def_nr.is_none()
                && !types.is_empty()
                && let Type::Vector(elm, _) = &types[0]
            {
                let elem = *elm.clone();
                let hint = match (name, arg_idx) {
                    // loft#945 — `map` is `fn(T) -> U`: the PARAMETER is the element type,
                    // the return is free.  See the twin hint in `parse_vector_method`.
                    ("map", 1) => Some(Type::Function(
                        vec![elem.clone()],
                        Box::new(Type::Unknown(0)),
                        Deps::none(),
                    )),
                    ("filter" | "any" | "all" | "count_if", 1) => Some(Type::Function(
                        vec![elem],
                        Box::new(Type::Boolean),
                        Deps::none(),
                    )),
                    ("reduce", 2) => {
                        let init_tp = types.get(1).cloned().unwrap_or(elem.clone());
                        Some(Type::Function(
                            vec![init_tp.clone(), elem],
                            Box::new(init_tp),
                            Deps::none(),
                        ))
                    }
                    _ => None,
                };
                if let Some(h) = hint {
                    self.expected = h;
                }
            }
            let mut p = Value::Null;
            // Capture each argument's start so a later type-mismatch diagnostic
            // (in `process_call_args`) points the caret at the argument, not at
            // the cursor drifted to `)` / `,`.
            arg_pos.push(self.lexer.peek_pos().clone());
            let t = self.expression(&mut p);
            self.expected = Type::Unknown(0);
            types.push(t);
            list.push(p);
            arg_idx += 1;
            // accept trailing comma on the last positional arg —
            // matching the struct-enum field list and enum variant list.
            if !self.lexer.has_token(",") || self.lexer.peek_token(")") {
                break;
            }
        }
        self.lexer.token(")");
        let ret = self.dispatch_call(
            val,
            source,
            name,
            &list,
            &types,
            &named_args,
            &call_pos,
            &arg_pos,
            name_pos,
        );
        // Plan-07 phase 1, step 1.13 — wrap user-typed Call / CallRef
        // at the `(` token position so runtime errors inside the call
        // (panic, divide-by-zero in callee, etc.) can be reported with
        // the call site's source location.  Skip on first pass and skip
        // when dispatch left val unchanged (e.g. early-return paths).
        if !self.first_pass && matches!(val, Value::Call(_, _) | Value::CallRef(_, _)) {
            let inner = std::mem::replace(val, Value::Null);
            *val = Value::with_span(call_pos, inner);
        }
        ret
    }

    /// loft#757 — `store_persist_bind(x, path)` persists the whole STORE `x` lives
    /// in, not `x`.  A keyed collection reached through a struct FIELD shares its
    /// struct's store, so binding it writes a file rooted at that struct, holding
    /// the struct and every sibling collection — and the file then refuses to load
    /// into a bare collection of the same type, which is how every other tool in a
    /// pipeline reads it.
    ///
    /// Measured: `struct Wrap { recs: hash<Rec[k]>, other: hash<Oth[j]> }` bound via
    /// `w.recs` writes a sidecar naming `Rec`, `Oth` AND `Wrap`; binding a bare
    /// `hash<Rec[k]>` local names only `Rec`.  The records are never damaged and the
    /// binding program reads its own output perfectly, so nothing surfaces until a
    /// DIFFERENT program loads the file — in the report, three pipeline steps later.
    ///
    /// ADVICE, not a warning: binding a field is CORRECT as written for a program
    /// that also loads through that container — loft's own `store_persist_bind` doc
    /// shows exactly that (`store_persist_bind(pw.painted, …)`), and dryopea reads
    /// its own store back through `pw` perfectly.  What goes wrong is a SECOND
    /// program binding the same collection type as a bare local, and no compile-time
    /// check can see that program.  Gating here would fail the CI of libraries whose
    /// use is self-consistent, which is precisely the split the two tiers exist for.
    fn check_persist_bind_root(&mut self, name: &str, list: &[Value], arg_pos: &[Position]) {
        if self.first_pass || name != "store_persist_bind" {
            return;
        }
        // A root local reads as `Var`; anything reached THROUGH something else — a
        // field read, an index — is a call to a getter op, and that is exactly the
        // shape whose store belongs to the container rather than to the collection.
        let Some(arg) = list.first() else { return };
        if matches!(arg.unspan(), Value::Var(_)) {
            return;
        }
        let Some(pos) = arg_pos.first().cloned() else {
            return;
        };
        diagnostic_at!(
            self.lexer,
            &pos,
            Level::Advice,
            code = "persist-bind-through-field",
            "store_persist_bind persists the whole store this collection lives in, and a \
             collection reached through a field shares its container's store — so the file \
             is written for the container (with every sibling collection in it) and will \
             not load back into a bare collection of this type"
        );
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Conditional,
            title: "bind a local of the collection\'s own type".to_string(),
            condition: Some("another program loads this file — it is fine as-is when the same container reads it back".to_string()),
            edit: None,
            concept: "durable store binding",
            concept_ref: "@F40",
        });
    }

    /// Dispatch a parsed call to the appropriate handler: diagnostics, special
    /// forms (`map/filter/reduce/sort/parallel_for`), fn-ref calls, or normal calls.
    #[allow(clippy::too_many_arguments)]
    fn dispatch_call(
        &mut self,
        val: &mut Value,
        source: u16,
        name: &str,
        list: &[Value],
        types: &[Type],
        named_args: &[(String, Value, Type)],
        call_pos: &Position,
        arg_pos: &[Position],
        name_pos: &Position,
    ) -> Type {
        if matches!(
            name,
            "assert" | "panic" | "log_info" | "log_warn" | "log_error" | "log_fatal"
        ) {
            return self.parse_call_diagnostic(val, name, list, types, call_pos);
        }
        self.check_persist_bind_root(name, list, arg_pos);
        match name {
            // @PLN105 Phase 1 — deliver(tag, value): hand the value's descriptor
            // handle to the host. Lower to OpDeliver(tag, value, db_tp), filling
            // db_tp from the value's static type. The value is passed by VALUE (its
            // code), so `@val` is the record DbRef on both backends (no OpCreateStack
            // slot indirection) — one #rust body ⇒ interpret == native.
            "deliver" if types.len() == 2 => {
                if self.first_pass {
                    return Type::Void;
                }
                let val_type = types[1].clone();
                let db_tp = self.get_type(&val_type);
                let op = self.data.def_nr("OpDeliver");
                *val = Value::Call(
                    op,
                    vec![
                        list[0].clone(),
                        list[1].clone(),
                        Value::Int(i32::from(db_tp)),
                    ],
                );
                return Type::Void;
            }
            // @PLN105 Phase 3 — expose(tag, value) / release(tag, value): the long-lived deliver
            // + its unpin. Lowered like `deliver`, filling `db_tp` from the value's static type.
            "expose" | "release" if types.len() == 2 => {
                if self.first_pass {
                    return Type::Void;
                }
                let db_tp = self.get_type(&types[1].clone());
                let op = self.data.def_nr(if name == "expose" {
                    "OpExpose"
                } else {
                    "OpRelease"
                });
                *val = Value::Call(
                    op,
                    vec![
                        list[0].clone(),
                        list[1].clone(),
                        Value::Int(i32::from(db_tp)),
                    ],
                );
                return Type::Void;
            }
            // @PLN127 arc B — `type_of(x)`: the declared shape of x's TYPE.
            //
            // The id is resolved HERE, so it is a parse-time constant on both
            // backends — the same mechanism `to_json` uses, and the reason the
            // answer needs no runtime name lookup (which `--native` could not
            // give, since it REPLAYS the type table rather than minting it).
            //
            // The argument is not evaluated. Nothing about the answer depends on
            // the value, and evaluating it would mean discarding a result — the
            // one operation loft's ownership model gets wrong most easily. The
            // contract is C's `sizeof`, and it is stated in the doc comment.
            "type_of" if types.len() == 1 => {
                if self.first_pass {
                    // First pass still needs the RESULT type, so an enclosing
                    // `t = type_of(v)` infers `TypeInfo` on both passes and the
                    // name-keyed variable tables do not shift underneath.
                    let ti = self.data.def_nr("TypeInfo");
                    return if ti == u32::MAX {
                        Type::Unknown(0)
                    } else {
                        Type::Reference(ti, crate::data::Deps::none())
                    };
                }
                // `get_type` answers the STORAGE type, which is right for its own
                // callers and wrong here for two scalars: a character is stored as
                // an integer, and a boolean has no entry at all. Reflection must
                // answer what the author DECLARED where the descriptor can express
                // it, so those two are named directly; everything else keeps the one
                // derivation, because a second one would be a second thing to drift.
                let kt = match &types[0] {
                    Type::Boolean => self.database.name("boolean"),
                    Type::Character => self.database.name("character"),
                    other => self.get_type(&other.clone()),
                };
                let d_nr = self.data.def_nr("n_reflect_type");
                let ti = self.data.def_nr("TypeInfo");
                if d_nr == u32::MAX || ti == u32::MAX {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "type_of is unavailable — default/07_reflect.loft did not load"
                    );
                    return Type::Unknown(0);
                }
                *val = Value::Call(d_nr, vec![Value::Int(i32::from(kt))]);
                return Type::Reference(ti, crate::data::Deps::none());
            }
            // @PLN23 S5 — `field_value(x, position)`: the VALUE half of
            // reflection, and the other half of the same parse-time trick.
            //
            // The type id is resolved HERE for the reason `type_of` resolves it
            // here: `--native` REPLAYS the type table rather than minting it, so
            // a runtime name lookup would have nothing to answer from. Unlike
            // `type_of`, the argument IS evaluated — the value is what is being
            // read.
            "field_value" if types.len() == 2 => {
                let fv = self.data.def_nr("ValueInfo");
                // Every refusal below still answers `ValueInfo`. The error is
                // already reported, and handing back `Unknown` would cascade a
                // second, misleading complaint about the enclosing variable
                // changing type — which buries the one that says what to fix.
                let answer = if fv == u32::MAX {
                    Type::Unknown(0)
                } else {
                    Type::Reference(fv, crate::data::Deps::none())
                };
                if self.first_pass {
                    // First pass needs the RESULT type so an enclosing
                    // `v = field_value(row, p)` infers `ValueInfo` on both
                    // passes and the name-keyed variable tables do not shift
                    // underneath — the same reason `type_of` does this.
                    return answer;
                }
                // Inside a generic the body is parsed ONCE against its type
                // variable, so there is no concrete type to read positions out
                // of and every call would answer `OtherKind` — an empty row
                // rather than an error, which is the silent under-delivery this
                // API exists to avoid. Say so where the author can see it.
                if let Some(tv) = self.generic_type_name(&types[0]) {
                    let tv = tv.to_string();
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "field_value cannot read a value of the type variable `{tv}` — a generic \
                         body is parsed once, so the concrete type is not known here. Call it \
                         where the type is known, and pass the values in."
                    );
                    return answer;
                }
                // Only a record has fields to read. Refusing here beats
                // answering `OtherKind` at run time: the author wrote a type
                // that can never have a field at any position, and that is a
                // mistake the compiler can see.
                if !matches!(types[0], Type::Reference(_, _)) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "field_value needs a record — {} has no fields to read",
                        types[0].name(&self.data)
                    );
                    return answer;
                }
                // @PLN23 S7b — a PATH reads through inline records, and it is
                // the same operation at a greater depth rather than a second
                // one: a one-element path answers what the bare position does.
                // So the two share a name and the argument's type picks the
                // lowering, which is the same decision `types.len()` already
                // makes one line up.
                let path_form = matches!(&types[1], Type::Vector(_, _));
                if !path_form && !matches!(types[1], Type::Integer(_)) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "field_value takes a position or a path of positions — {} is neither",
                        types[1].name(&self.data)
                    );
                    return answer;
                }
                let kt = self.get_type(&types[0].clone());
                let fname = if path_form {
                    "n_reflect_field_path"
                } else {
                    "n_reflect_field"
                };
                let d_nr = self.data.def_nr(fname);
                if d_nr == u32::MAX || fv == u32::MAX {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "field_value is unavailable — default/07_reflect.loft did not load"
                    );
                    return answer;
                }
                *val = Value::Call(
                    d_nr,
                    vec![list[0].clone(), list[1].clone(), Value::Int(i32::from(kt))],
                );
                return Type::Reference(fv, crate::data::Deps::none());
            }
            "parallel_for" => return self.parse_parallel_for(val, list, types),
            "par_fold" => return self.parse_par_fold(val, list, types),
            "map" => return self.parse_map(val, list, types),
            "filter" => return self.parse_filter(val, list, types),
            "reduce" => return self.parse_reduce(val, list, types),
            "sort" => return self.parse_sort(val, list, types),
            "insert" => return self.parse_insert(val, list, types),
            "reverse" => return self.parse_reverse(val, list, types),
            "reserve" => return self.parse_reserve(val, list, types),
            "any" => return self.parse_any(val, list, types),
            "all" => return self.parse_all(val, list, types),
            "count_if" => return self.parse_count_if(val, list, types),
            "next" if types.len() == 1 => {
                // CO1.6a: next(gen) — advance a coroutine iterator.
                // Encode value_size as second parameter so codegen can emit it.
                // @P327 — same encoding as the for-loop's `iterator()` path
                // in `parser/collections.rs`: high byte = channel tag
                // (1 = unified `next_into` for tuple yields).  Without this,
                // manual `next()` on `iterator<(integer, integer)>` routes
                // through the legacy text channel (size 16 ≡ `&str`) and
                // returns a `String` where Rust expected a tuple.
                if let Type::Iterator(inner, _) = &types[0] {
                    let yield_tp = (**inner).clone();
                    let byte_size = i32::from(crate::variables::size(
                        &yield_tp,
                        &crate::data::Context::Argument,
                    ));
                    // @PLAN16 phase 02 — same packed encoding as the
                    // for-loop's `iterator()` path; tag 1 = layout-driven
                    // tuple walk (kind codes appended as extra args), tag 2 =
                    // fn-ref.  `tuple_kinds` is the shared gate so the consumer
                    // and the generator's producer never diverge.
                    let tkinds = crate::coroutine_layout::tuple_kinds(&yield_tp);
                    // #401 — shared channel decision (float/single/enum get their
                    // own tags); the for-loop path uses the same helper, so manual
                    // `next()` no longer diverges (it dropped float/single/enum →
                    // native E0308 on `let var: f64 = coroutine_next_i64(..)`).
                    let channel_tag = crate::coroutine_layout::channel_tag(&yield_tp);
                    let value_size: i32 = (channel_tag << 8) | byte_size;
                    let op = self.data.def_nr("OpCoroutineNext");
                    let mut args = list.to_vec();
                    args.push(Value::Int(value_size));
                    if let Some(kinds) = &tkinds {
                        args.extend(kinds.iter().map(|k| Value::Int(k.code())));
                    }
                    *val = Value::Call(op, args);
                    return yield_tp;
                }
                if self.first_pass {
                    return Type::Unknown(0);
                }
            }
            "exhausted" if types.len() == 1 && matches!(&types[0], Type::Iterator(_, _)) => {
                // CO1.3c: exhausted(gen) on a coroutine iterator.
                let op = self.data.def_nr("OpCoroutineExhausted");
                *val = Value::Call(op, list.to_vec());
                return Type::Boolean;
            }
            _ => {}
        }
        // loft#1147 — `assert_eq` / `assert_ne` take the CALLER's position, exactly as
        // `assert` does and for the same reason: a failure that names `01_code.loft` names
        // the forwarder rather than the test that broke.  Unlike `assert` they are NOT
        // rewritten here — they are ordinary generic loft functions whose bodies live in the
        // stdlib, and their bound is what makes them work on any type — so this supplies the
        // two position arguments and nothing else.  The label is optional: with two
        // arguments the message is the two values alone, which a source position already
        // qualifies.
        if matches!(name, "assert_eq" | "assert_ne")
            && (list.len() == 2 || list.len() == 3)
            && named_args.is_empty()
            // `n_`-prefixed: user-level functions live under that namespace (CODE.md), and
            // the bare name resolves to nothing.  The guard keeps a project that has not
            // loaded the stdlib — or one defining its own two-argument `assert_eq` — on the
            // ordinary call path instead of being handed arguments its signature lacks.
            && self.data.def_nr(&format!("n_{name}")) != u32::MAX
        {
            let mut args = list.to_vec();
            let mut tps = types.to_vec();
            if args.len() == 2 {
                args.push(Value::str(""));
                tps.push(Type::Text(Deps::none()));
            }
            args.push(Value::str(&call_pos.file));
            tps.push(Type::Text(Deps::none()));
            args.push(Value::Int(call_pos.line as i32));
            tps.push(Type::Integer(IntegerSpec::wide()));
            return self.call(
                val, source, name, &args, &tps, named_args, arg_pos, name_pos,
            );
        }
        if let Some(tp) = self.try_fn_ref_call(val, name, list, types) {
            return tp;
        }
        self.call(
            val, source, name, list, types, named_args, arg_pos, name_pos,
        )
    }

    /// The parameter type at `arg_idx` when `name` is a fn-ref in scope — a local of
    /// `Type::Function`, or one this lambda can capture from the enclosing scope.
    ///
    /// The `⇐` expected-type channel's argument push reads a NAMED function's attribute, and
    /// a lambda has no `n_<name>` definition to read.  Its declared parameter list says the
    /// same thing, so this is the second spelling of one question rather than a second
    /// question (loft#1280).
    fn fnref_param_hint(&self, name: &str, arg_idx: usize) -> Option<Type> {
        let v_nr = self.vars.var(name);
        let tp = if v_nr == u16::MAX {
            // A fn-ref reached only through the capture context — the same lookup
            // `try_fn_ref_call` makes before the variable exists in this scope.
            self.capture_context
                .iter()
                .find(|(n, t)| n == name && matches!(t, Type::Function(_, _, _)))
                .map(|(_, t)| t.clone())?
        } else {
            self.vars.tp(v_nr).clone()
        };
        match tp.base() {
            Type::Function(params, _, _) => params.get(arg_idx).cloned(),
            _ => None,
        }
    }

    /// Try to dispatch as a call through a function-reference variable.
    /// Returns `Some(return_type)` if `name` is a fn-ref variable, `None` otherwise.
    fn try_fn_ref_call(
        &mut self,
        val: &mut Value,
        name: &str,
        list: &[Value],
        types: &[Type],
    ) -> Option<Type> {
        // P215: name lookup for outer-scope fn-ref captures.
        //
        // Bare-name reads route through `parser/objects.rs:162-200`
        // which scans `capture_context`; call syntax `name(args)`
        // bypasses that path and lands here.  When `name` matches a
        // `Type::Function` capturable from the outer scope, we need
        // to (a) push it to `captured_names` (drives
        // `synthesize_closure_record`'s attribute set), (b) create a
        // placeholder local var on the first pass so subsequent
        // lookups find it.  At call-emit time below, an
        // `is_outer_fnref` test on `capture_context` decides whether
        // to wrap the CallRef in a closure-record load.
        let outer_fnref_type = self
            .capture_context
            .iter()
            .find(|(n, t)| n == name && matches!(t, Type::Function(_, _, _)))
            .cloned()
            .map(|(_, t)| t);
        if !self.vars.name_exists(name) {
            let ctype = outer_fnref_type.clone()?;
            if !self.captured_names.iter().any(|(n, _)| n == name) {
                self.captured_names.push((name.to_string(), ctype.clone()));
            }
            let v_nr = self.create_var(name, &ctype);
            self.var_usages(v_nr, true);
        } else if outer_fnref_type.is_some() && !self.captured_names.iter().any(|(n, _)| n == name)
        {
            // Second-pass: var exists from first pass but
            // captured_names is fresh (reset per-lambda).  Re-record.
            if let Some(ctype) = outer_fnref_type.clone() {
                self.captured_names.push((name.to_string(), ctype));
            }
        }
        let v_nr = self.vars.var(name);
        let Type::Function(param_types, ret_type, _) = self.vars.tp(v_nr).clone() else {
            return None;
        };
        // @PLN85 L1 — callee-attr-space deps must not leak into the caller
        // (see `fnref_result_type`): map visible-param deps through the actual
        // argument types; an index naming no visible argument names the closure this slot
        // carries, and only a CAPTURING slot has one (loft#1180).
        let mut ret_type = Box::new(Self::fnref_result_type(
            *ret_type,
            types,
            Self::capturing_fnref_var(&self.vars, v_nr),
        ));
        // loft#1327 — an OPAQUE target's heap return may be one of the arguments, and the fn
        // TYPE cannot say so.
        //
        // `(O-Move)` puts the obligation on the return type: *"if the return borrows a
        // parameter, the return type records it"*.  A DEFINITION records it — that is what
        // `fnref_result_type` above maps into the caller's space.  A fn TYPE has nowhere to
        // write it: `fn(vector<integer>?) -> vector<integer>` is the whole of what the author
        // may spell, so the deps arrive empty whatever the target does.  Empty deps then read
        // as *"the callee minted this"*, `u` is typed an owner, and scope exit frees it —
        // releasing the CALLER's vector on the arm where the closure handed its argument back,
        // silently, with the next allocation reusing the slot.
        //
        // A fn-typed PARAMETER is the case where the target is unknowable from this body: no
        // assignment in it can be read to resolve one.  So the return borrows what it might
        // borrow — every heap argument, rooted through the same walk the @P290 bracket uses —
        // and a non-empty dep is what stops the free.  It costs the MINTING arm its owner (one
        // store per call, announced at exit); that trade is the standing one, because a leak is
        // recoverable where a premature free is not.
        //
        // Narrow to a parameter deliberately: a fn-ref LOCAL is resolved from its assignment by
        // `Scopes::fnref_target`, and every route built on that (the lift, the identity free)
        // reads the empty deps this leaves alone.  A local assigned two different lambdas is
        // opaque too and is NOT covered here — the parser cannot see that, and the same free
        // reaches it.
        if self.vars.is_argument(v_nr)
            && crate::data::is_dbref(ret_type.base())
            && ret_type.depend().is_empty()
        {
            let borrows: Vec<u16> = list
                .iter()
                .zip(types.iter())
                .filter(|(_, t)| crate::data::is_dbref(t.base()))
                .filter_map(
                    |(v, _)| match crate::use_analysis::view_root_slots(&self.data, v) {
                        Some(roots) => roots.first().copied(),
                        None => None,
                    },
                )
                .collect();
            if !borrows.is_empty() {
                *ret_type = ret_type.with_deps(&Deps::frame(borrows));
            }
        }
        let ret_type = ret_type;
        // P227 — see the zero-argument twin above: the call site pushes the widest
        // candidate's `&text` buffer count and the dispatcher pops the excess, because a
        // `&text` points into the CALLER's frame and only the caller can supply one that
        // outlives the call (loft#1116).
        let work_vars = self.fnref_text_buffer_vars(param_types.len(), ret_type.as_ref());
        if !self.first_pass {
            if list.len() != param_types.len() {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Function reference '{name}' expects {} argument(s), got {}",
                    param_types.len(),
                    list.len()
                );
                return Some(*ret_type);
            }
            let mut converted = list.to_vec();
            for (i, expected) in param_types.iter().enumerate() {
                self.convert(&mut converted[i], &types[i], expected);
            }
            // inject hidden work-buffer DbRef args for text-returning lambdas.
            // Each block emits OpCreateStack → 12-byte DbRef, matching callee's &text param.
            // Order: visible params → work bufs → __closure (must match callee slot layout).
            // prepend v_set(wv, "") to clear the buffer so loop iterations start fresh.
            self.push_fnref_text_buffers(&mut converted, &work_vars);
            // inject hidden __closure argument — the closure allocation
            // expression is generated inline so it runs at the call site, avoiding
            // closure is embedded in the 16-byte fn-ref slot; fn_call_ref
            // pushes it automatically — no explicit injection needed at call sites.
            // mark captured vars as read at the call site
            for &cv in &std::mem::take(&mut self.last_closure_captured_vars) {
                self.var_usages(cv, true);
            }
            self.var_usages(v_nr, true);
            // P215: if we just captured this name from outer scope,
            // populate the placeholder var from the closure record's
            // field BEFORE the CallRef.  Without this the CallRef
            // reads garbage from the uninitialised local slot.  The
            // closure-record attribute was registered in
            // `synthesize_closure_record` (parser/vectors.rs:762);
            // `closure_param` (parser/vectors.rs:419) holds the DbRef
            // at runtime.  `get_field` produces the (d_nr,
            // closure_DbRef) tuple via the new fn_ref_field_read
            // gate added in P215 (parser/mod.rs::get_field).
            let call_ir = Value::CallRef(v_nr, converted);
            // P215: detect "this name was captured from outer scope" by
            // checking `captured_names` (populated either in this turn
            // through Step 1 above, or in a prior pass).  `name_exists`
            // returns true on the second pass even for captured names
            // (placeholder var was created in first pass), so we can't
            // gate just on `captured_via_closure` — that flag only
            // fires on the first pass when the var is created fresh.
            // P215: detect captured-from-outer status via
            // `capture_context` rather than `captured_names`, since
            // `captured_names` only tracks captures ADDED during the
            // current pass — second-pass `inner(y)` lookups don't
            // re-add and the flag would miss them.  `capture_context`
            // is populated at `parse_lambda` entry from the outer
            // scope's all-names (parser/vectors.rs:364) and is stable
            // across both passes.
            let was_captured = self
                .capture_context
                .iter()
                .any(|(n, t)| n == name && matches!(t, Type::Function(_, _, _)));
            if was_captured
                && self.closure_param != u16::MAX
                && let closure_rec_d = self.data.def(self.context).closure_record()
                && closure_rec_d != u32::MAX
            {
                let f_nr = self.data.attr(closure_rec_d, name);
                if f_nr != usize::MAX {
                    let load = self.get_field(closure_rec_d, f_nr, Value::Var(self.closure_param));
                    *val = v_block(
                        vec![crate::data::v_set(v_nr, load), call_ir],
                        *ret_type.clone(),
                        "captured_fn_ref_call",
                    );
                    return Some(*ret_type);
                }
            }
            *val = call_ir;
            // for void-return capturing lambdas, write updated closure
            // record fields back to the corresponding outer variables so the caller
            // observes mutations made inside the lambda body (e.g. `count += x`).
            // Non-void returns are not handled here — they require a temp to hold
            // the return value while writing back, which is left for A5.6 (1.1+).
            if matches!(*ret_type, Type::Void)
                && let Some(&closure_w) = self.closure_vars.get(&v_nr)
                && let Type::Reference(closure_rec_d, _) = self.vars.tp(closure_w).clone()
            {
                let n_attrs = self.data.attributes(closure_rec_d);
                let mut block: Vec<Value> = vec![val.clone()];
                for aid in 0..n_attrs {
                    let cap_name = self.data.attr_name(closure_rec_d, aid).clone();
                    let outer_v = self.vars.var(&cap_name);
                    // Plan-22 phase 02d-iii.e + @P319 — skip the
                    // write-back for ALL shared-reference captures,
                    // i.e. those stored in the closure record via the
                    // auto-Reference 12-byte DbRef encoding (the
                    // closure attribute is `Reference(d, deps)` with
                    // NON-EMPTY deps).  This covers boxed `__cell_<T>`
                    // scalars (02d-iii.e, the original case) AND struct
                    // / reference captures such as a captured `Mesh`
                    // whose `.vertices` vector is appended to inside the
                    // lambda (@P319).
                    //
                    // For these the closure holds a DbRef into the LIVE
                    // outer value, so body mutations already propagate
                    // through the shared store.  A bare
                    // `v_set(outer, OpGetDbRef(rec, off))` copies that
                    // 12-byte DbRef back over itself — a value no-op —
                    // but the reassignment's free-old-ref step releases
                    // the store the closure record still references.
                    // That premature free lets the next call reuse the
                    // store, clobbering the captured value: silent data
                    // loss when the trampled field is at offset 0 (a
                    // `len` reads back 0), or a SIGSEGV in `new_record`
                    // when it is at a non-zero offset.  Native compiles
                    // the same IR without the free, so this corrupted
                    // only the interpreter.  Only genuine by-VALUE
                    // captures (inline-bytes encoding, empty deps) need
                    // the write-back to observe their mutations.
                    if matches!(
                        self.data.attr_type(closure_rec_d, aid),
                        Type::Reference(_, ref deps) if !deps.is_empty()
                    ) {
                        continue;
                    }
                    // The binding may be no variable of THIS scope at all: the lambda being
                    // called reaches past it, and this scope is itself a lambda that captured
                    // the name on its behalf.  Then the write-back goes into this lambda's own
                    // closure record, so the next level out observes it the same way.  Asked
                    // BEFORE the value is built, because emitting the read and discarding it
                    // changes what every other function compiles to.
                    let relayed = if outer_v == u16::MAX {
                        self.relayed_capture_attr(&cap_name)
                    } else {
                        None
                    };
                    if outer_v == u16::MAX && relayed.is_none() {
                        continue;
                    }
                    let field_val = self.get_field(closure_rec_d, aid, Value::Var(closure_w));
                    if outer_v != u16::MAX {
                        block.push(v_set(outer_v, field_val));
                    } else if let Some((rec, fnr)) = relayed {
                        let back = self.set_field_no_check(
                            rec,
                            fnr,
                            0,
                            Value::Var(self.closure_param),
                            field_val,
                        );
                        block.push(back);
                    }
                }
                if block.len() > 1 {
                    // Use Insert rather than Block: we must NOT create a new scope
                    // here because ___clos_1 (closure_w) is owned by the outer scope.
                    // A Block would cause scopes.rs to emit OpFreeRef at the inner
                    // scope exit, leaving a dangling ref for the next call.
                    *val = Value::Insert(block);
                }
            }
        }
        Some(*ret_type)
    }

    // Validate and rewrite a user-friendly `parallel_for(fn f, vec, threads)` call
    // into a `Value::Call(n_parallel_for_d_nr, [input, elem_size, return_size, threads, func])`.
    //
    // The parser intercepts calls by name "parallel_for" before normal overload
    // resolution.  Compile-time checks performed here:
    // - First arg must be `Type::Function(args, ret)` (produced by `fn <name>` expression).
    // - Second arg must be `Type::Vector(T, _)`.
    // - Worker's first parameter must be a reference to T (type checked by name).
    // - Return type must be a primitive: integer, long, float, or boolean.
    // - Extra arg count must match the worker's extra parameters (args[1..]).
    /// Compiler special-case for `reduce(v: vector<T>, init: U, f: fn(U, T) -> U) -> U`.
    /// Generates inline bytecode equivalent to a left-fold over the vector.
    /// Does a value of this type live in a STORE (or a text buffer) rather than in the
    /// stack slot itself?  The question the callback builtins ask (loft#945/#951): a
    /// callee answering one of these is handed a caller-allocated buffer, and the
    /// hand-built per-element call each builtin lowers cannot always supply one.
    pub(crate) fn is_heap_storage(t: &Type) -> bool {
        matches!(
            t.base(),
            Type::Text(_)
                | Type::Vector(_, _)
                | Type::Reference(_, _)
                | Type::Enum(_, true, _)
                | Type::RefVar(_)
                | Type::Sorted(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _)
                | Type::Trie(_, _, _)
        )
    }

    pub(crate) fn parse_reduce(&mut self, val: &mut Value, list: &[Value], types: &[Type]) -> Type {
        if self.first_pass {
            // On first pass, return the accumulator type (second arg) if available.
            if types.len() >= 2 {
                return types[1].clone();
            }
            return Type::Unknown(0);
        }
        if list.len() != 3 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce requires 3 arguments: reduce(vector, init, fn f)"
            );
            return Type::Unknown(0);
        }
        let _in_elem_type = if let Type::Vector(elm, _) = &types[0] {
            *elm.clone()
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce: first argument must be a vector"
            );
            return Type::Unknown(0);
        };
        let (fn_param_types, _fn_ret_type) = if let Type::Function(params, ret, _) = &types[2] {
            (params.clone(), *ret.clone())
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce: third argument must be a function reference (use fn <name>)"
            );
            return Type::Unknown(0);
        };
        if fn_param_types.len() != 2 {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce: function must take exactly two arguments (accumulator, element)"
            );
            return Type::Unknown(0);
        }
        // loft#956 — the FOLD FUNCTION's first parameter is what the accumulator type is.
        //
        // An empty `[]` written as the init argument carries no element type of its own, so
        // it arrives here as `Unknown(0)` — and every question asked below is asked of the
        // init's type.  `is_heap_storage` answers false for `Unknown`, so the collection
        // refusal did not fire, and `reduce_acc` was minted with no type at all: the
        // interpreter read it back EMPTY and `--native` reached `rust_type` with a `Never`
        // and panicked (*"Incorrect type Never"*).  Both silent about the real cause, which
        // is that nothing had said what `[]` was empty OF.
        //
        // The signature has always known.  `f: fn(U, T) -> U` names `U` in its first
        // parameter, and the init only has to be assignable to it, so read the accumulator
        // off the fold rather than off the literal.  That makes the diagnostic below name
        // the type the program actually meant, and it is the inference the fold needs
        // anyway on the day a collection accumulator is supported.
        let mut acc_type = types[1].clone();
        if acc_type.is_unknown() && !fn_param_types[0].is_unknown() {
            acc_type = fn_param_types[0].without_deps();
        }
        // loft#951 — a COLLECTION accumulator is refused rather than mis-compiled.  Text
        // is not: it is handled below, by the same work-buffer route an ordinary
        // `acc = f(acc, x)` assignment takes.
        //
        // A collection is a different defect with a different shape, not the same one one
        // size larger, which is why it is still refused after loft#951: the fold has to
        // hand the callee a buffer per step, and a collection's is not the text one.
        if Self::is_heap_storage(&acc_type) && !matches!(acc_type.base(), Type::Text(_)) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "`reduce` cannot fold into a `{}` accumulator yet — only scalars and \
                 `text`. Write the loop instead: \
                 `acc = <init>; for x in v {{ acc = f(acc, x); }}`",
                acc_type.source_name(&self.data)
            );
            return acc_type;
        }
        // Still unresolved: neither the init nor the signature said what this folds into.
        // Refuse and name the hole — a variable with no type reaches codegen as `Never`,
        // which is an internal compiler error on `--native` and an empty read on the
        // interpreter (loft#956).
        if acc_type.is_unknown() {
            diagnostic!(
                self.lexer,
                Level::Error,
                "`reduce` cannot tell what its accumulator holds — the initial value gives \
                 no type (`[]` is empty of nothing in particular) and the fold function's \
                 first parameter does not say either. Annotate it: \
                 `acc: <type> = <init>; v.reduce(acc, f)`"
            );
            return Type::Unknown(0);
        }
        // Extract the compile-time d_nr from the fn-ref value (always Value::Int(d_nr)).
        let fn_d_nr = if let Value::Int(d) = &list[2] {
            *d as u32
        } else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "reduce: function must be a compile-time constant (use fn <name>)"
            );
            return Type::Unknown(0);
        };

        // loft#951 — a TEXT accumulator is minted as a WORK BUFFER, which puts it at
        // FUNCTION scope rather than inside the `reduce` block.  The block's tail is the
        // accumulator, and generated Rust ends a text block with a BORROW of it
        // (`&var…`), so a block-scoped accumulator is dropped while still borrowed:
        // `error[E0597]: var__reduce_acc_1 does not live long enough`.  Every other
        // text-valued block loft emits — a formatted string, say — already builds into a
        // function-scope `__work_N` for exactly this reason.
        //
        // `work_text_p2`, not `work_text`: this function returns early on pass 1, so a
        // mint here fires on pass 2 ONLY.  Drawing from the shared sequence would shift
        // every later `__work_N` relative to pass 1, and the variable tables persist BY
        // NAME — which is loft#662, pass 2 re-finding pass 1's variables under the wrong
        // roles.  The `_p2` sequence has no pass-1 counterpart to collide with.
        let acc_var = if matches!(acc_type.base(), Type::Text(_)) {
            self.vars.work_text_p2(&mut self.lexer)
        } else {
            self.create_unique("reduce_acc", &acc_type)
        };
        self.vars.defined(acc_var);

        let mut in_type = types[0].clone();
        let vec_copy_var = self.create_unique("reduce_vec", &in_type);
        in_type = in_type.depending(vec_copy_var);

        let iter_var = self.create_unique("reduce_idx", &I32);
        self.vars.defined(iter_var);

        let var_tp = self.for_type(&in_type);
        let for_var = self.create_unique("reduce_elm", &var_tp);
        self.vars.defined(for_var);

        let mut create_iter_code = Value::Var(vec_copy_var);
        let it = Type::Iterator(Box::new(var_tp.clone()), Box::new(Type::Null));
        let loop_nr = self.vars.start_loop();
        let iter_next = self.iterator(&mut create_iter_code, &in_type, &it, iter_var, None);
        self.vars.loop_var(for_var);
        self.vars.finish_loop(loop_nr);
        let for_next = v_set(for_var, iter_next);

        // loft#1000 — a VECTOR ends on its LENGTH, not on the element's value: a null the
        // vector really holds shares the out-of-bounds sentinel, and a `value struct`
        // element is deep-copied into a fresh record on bind (@PLN101), so the bound local
        // is never null and the test could never fire — `reduce` hung forever.
        let break_if_null = if matches!(in_type, Type::Vector(_, _)) {
            Value::Insert(self.vector_loop_break(&Value::Var(vec_copy_var), iter_var))
        } else {
            let mut test_for = Value::Var(for_var);
            self.convert(&mut test_for, &var_tp, &Type::Boolean);
            let not_test = self.cl("OpNot", &[test_for]);
            v_if(
                not_test,
                v_block(vec![Value::Break(0)], Type::Void, "break"),
                Value::Null,
            )
        };

        // Use Value::Call(d_nr, ...) directly — no fn_ref_var local needed.  loft#945:
        // through `callback_call`, so a fold whose accumulator is TEXT gets the hidden
        // buffer its callee takes (`xs.reduce("", |a, x| { "{a}{x}" })` was an ICE).
        // The ELEMENT parameter takes the same unboxing map/filter/any do: a tuple loop
        // var is a `Reference(__tuple<…>)` and the callee declares the tuple by VALUE, so
        // the fold read a packed DbRef as its element (loft#1074).  The accumulator is
        // untouched — it is a local of the init's own type, never an element reference.
        let (elem_arg, elem_arg_tp) = self.callback_element_arg(&in_type, for_var, &var_tp);
        let fold_call = self.callback_call(
            fn_d_nr,
            vec![Value::Var(acc_var), elem_arg],
            vec![acc_type.clone(), elem_arg_tp],
        );
        // loft#951 — a TEXT accumulator cannot take the bare `acc = f(acc, x)`.  A callee
        // answering text writes into ONE caller-allocated buffer and CLEARS it on entry,
        // so binding `acc` straight to that buffer means the next turn erases the fold so
        // far: the interpreter answered the LAST element and `--native` did not compile
        // (`var__reduce_acc_1 does not live long enough`).
        //
        // The cure already exists and is what the hand-written loop has always compiled
        // to.  `assign_text` sees that a right-hand side READS the variable being assigned
        // and routes it through a second, caller-owned work buffer, so the buffer the
        // callee clears is never the one the live accumulator holds.  This is that same
        // sequence — spelled out rather than called, because `assign_text` draws from the
        // shared `__work_N` sequence and this site is pass-2-only (see `acc_var` above).
        //
        // A VECTOR accumulator needs none of it: its assignment target is exactly what
        // H7's `self_feeding_call` looks for, so `rotate_loop_retbufs` already gives the
        // site a partner buffer and ping-pongs the pair.
        let fold_step = if matches!(acc_type.base(), Type::Text(_)) {
            let work = self.vars.work_text_p2(&mut self.lexer);
            Value::Insert(vec![
                self.cl("OpClearText", &[Value::Var(work)]),
                self.cl("OpAppendText", &[Value::Var(work), fold_call]),
                v_set(acc_var, Value::Var(work)),
            ])
        } else {
            v_set(acc_var, fold_call)
        };

        let loop_body = vec![for_next, break_if_null, fold_step];

        *val = v_block(
            vec![
                v_set(acc_var, list[1].clone()),
                v_set(vec_copy_var, list[0].clone()),
                create_iter_code,
                v_loop(loop_body, "reduce loop"),
                Value::Var(acc_var),
            ],
            acc_type.clone(),
            "reduce",
        );
        acc_type
    }

    // <size> ::= ( <type> | <var> ) ')'
    pub(crate) fn parse_size(&mut self, val: &mut Value) -> Type {
        let mut found = false;
        let lnk = self.lexer.link();
        if let Some(id) = self.lexer.has_identifier() {
            let d_nr = self.data.def_nr(&id);
            if d_nr != u32::MAX && self.data.def_type(d_nr) != DefType::EnumValue {
                if !self.first_pass && self.data.def_type(d_nr) == DefType::Unknown {
                    // Still unresolved after pass 2, so this is not a forward reference:
                    // those resolve in `resolve_deferred_unknowns` and take the branch
                    // below with a real size.  It is a typo.  Accepting it silently left
                    // `*val` at its `Null` initialiser, so `sizeof(NoSuchType)` answered
                    // `null` and that null flowed on as a value (loft#933).  Still marked
                    // `found`, so the expression path below adds no cascade.
                    found = true;
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Undefined type {id} — sizeof needs a variable or a declared type"
                    );
                } else if let Some(tp) = self.parse_type(u32::MAX, &id, false) {
                    found = true;
                    if !self.first_pass {
                        // Post-2c: prefer the alias's forced size(N) annotation.
                        // `d_nr` (local above) is the def_nr of the alias the user
                        // typed — e.g. i32 — not the base integer it collapses to
                        // via type_elm.  Only forced_size on the alias applies.
                        let forced = self.data.forced_size(d_nr);
                        let packed = tp.size(false);
                        *val = if let Some(n) = forced {
                            Value::Int(i32::from(n))
                        } else if packed > 0 {
                            // Range-constrained integer: use packed field size
                            Value::Int(i32::from(packed))
                        } else {
                            Value::Int(i32::from(
                                self.database
                                    .size(self.data.def(self.data.type_elm(&tp)).known_type()),
                            ))
                        };
                    }
                }
            }
        }
        if !found {
            let mut drop = Value::Null;
            self.lexer.revert(lnk);
            let tp = self.expression(&mut drop);
            let e_tp = self.data.type_elm(&tp);
            if e_tp != u32::MAX {
                found = true;
                if matches!(tp, Type::Enum(_, true, _) | Type::Reference(_, _)) && !self.first_pass
                {
                    // Polymorphic enum or reference: size depends on runtime variant.
                    *val = self.cl("OpSizeofRef", &[drop]);
                } else {
                    *val = Value::Int(i32::from(
                        self.database.size(self.data.def(e_tp).known_type()),
                    ));
                }
            }
        }
        if !self.first_pass && !found {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expect a variable or type after sizeof"
            );
        }
        self.lexer.token(")");
        I32.clone()
    }

    /// `type_name(expr)` — compile-time intrinsic that returns the static type
    /// of `expr` as a text constant.  Works on both type names and expressions:
    /// `type_name(integer)`, `type_name(my_var)`, `type_name(1 + 2)`.
    pub(crate) fn parse_type_name(&mut self, val: &mut Value) -> Type {
        // Try parsing as a type name first (like sizeof does).
        let mut found = false;
        let lnk = self.lexer.link();
        if let Some(id) = self.lexer.has_identifier() {
            let d_nr = self.data.def_nr(&id);
            if d_nr != u32::MAX && self.data.def_type(d_nr) != DefType::EnumValue {
                if !self.first_pass && self.data.def_type(d_nr) == DefType::Unknown {
                    // Same unresolved-name case `parse_size` handles above, and it was
                    // silent here for the same reason: `*val` kept its `Null` initialiser,
                    // so `type_name(NoSuchType)` rendered `null` as if that were the name
                    // of a type (loft#933).
                    found = true;
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Undefined type {id} — type_name needs a variable or a declared type"
                    );
                } else if let Some(tp) = self.parse_type(u32::MAX, &id, false) {
                    found = true;
                    if !self.first_pass {
                        *val = Value::Text(self.data.type_name_str(&tp));
                    }
                }
            }
        }
        if !found {
            let mut drop = Value::Null;
            self.lexer.revert(lnk);
            let tp = self.expression(&mut drop);
            if !self.first_pass {
                *val = Value::Text(self.data.type_name_str(&tp));
            }
        }
        self.lexer.token(")");
        Type::Text(Deps::none())
    }

    /// #432 — should a bare vector-literal argument be seeded with this parameter
    /// type's element width (`vector_hint`)?  Only for a CONCRETE narrow-integer
    /// element (`vector<u8>` … `vector<i32>`): an untyped integer literal infers
    /// `vector<integer>` (8-byte stride) and the callee would reinterpret it at the
    /// narrow stride.  Each branch below is deliberately NOT covered:
    /// - A generic `vector<T>` (element is a `Reference` to a type-var) must NOT
    ///   seed — the literal cannot be built at an abstract element type, and seeding
    ///   it wrongly fails `min_of([3, 1, 2])` with "would lose precision".
    /// - `vector<single>` is excluded on purpose: a float literal infers
    ///   `vector<float>` and f64→f32 is rejected as precision-loss regardless of the
    ///   constant, so seeding would turn the (separate, pre-existing) stride bug
    ///   into a fresh compile error — out of #432's "integer-vector literal" scope.
    /// - Struct/enum element vectors already build from their own literal.
    ///
    /// Recurses through nested vector layers so `vector<vector<u8>>` seeds too (the
    /// outer literal is seeded; inner literals thread their element type through
    /// `var_tp`).  The leaf must be a narrow integer.
    pub(crate) fn seeds_vector_hint(expected: &Type) -> bool {
        match expected {
            Type::Vector(elem, _) => {
                // @PLN25: peel `Optional(τ)` so a `vector<u8?>` literal seeds its narrow
                // stride like `vector<u8>` (else #432 stride-reinterpretation corruption).
                matches!(elem.base(), Type::Integer(_)) || Self::seeds_vector_hint(elem)
            }
            _ => false,
        }
    }

    /// loft#703 — may a bare `[…]` argument take its CONTAINER from this parameter type?
    ///
    /// `seeds_vector_hint` above answers the narrower #432 question — may the parameter
    /// override the element WIDTH the literal already inferred.  A keyed parameter is a
    /// different question with no trade-off in it: `[K { … }]` infers `vector<K>`, which
    /// is not a `hash<K[k]>` at any width, so the parameter type is the only thing that
    /// can say what to build and passing one was simply impossible without it.
    pub(crate) fn seeds_collection_hint(expected: &Type) -> bool {
        Self::seeds_vector_hint(expected) || crate::parser::vectors::is_keyed(expected)
    }

    // <call> ::= [ <expression> { ',' <expression> } ] ')'
    pub(crate) fn parse_method(&mut self, val: &mut Value, md_nr: u32, on: Type) -> Type {
        let mut list = vec![val.clone()];
        let mut types = vec![on];
        // arg_pos aligns with `list` by index; slot 0 is the receiver (its
        // position is the method-name token, the best available caret).
        let mut arg_pos: Vec<Position> = vec![self.lexer.peek_pos().clone()];
        // @F17 — named arguments reach the METHOD spelling too.  `parse_call` and
        // this loop are the language's two argument lists, and only the free one
        // collected `name: value`, so `show(c, loud: true)` compiled while
        // `c.show(loud: true)` was a parse error — the same function, the same
        // argument, the same default.  The gap was only ever this parse loop:
        // `call_with_named` already takes `is_method` and resolves names against the
        // callee's own attributes, so both spellings now emit the same call.
        // It is the dot spelling that needs this most, because `advice[trailing-
        // boolean-parameters]` sends a method's author here ("give them defaults so
        // callers pass only what they change") and only a named argument can change
        // one that is not a prefix.
        let mut named_args: Vec<(String, Value, Type)> = Vec::new();
        let mut in_named = false;
        if self.lexer.has_token(")") {
            return self.call_nr(val, md_nr, &list, &types, true, &arg_pos, None);
        }
        loop {
            if let Some(arg_name) = self.lexer.peek_named_arg() {
                in_named = true;
                self.lexer.has_identifier(); // consume name
                self.lexer.has_token(":"); // consume :
                // #432 seeding, keyed by NAME rather than by position — a named
                // vector-literal or format-string argument builds at its own
                // parameter's type, not at the one this slot would have held.
                self.expected = Type::Unknown(0);
                if md_nr != u32::MAX {
                    let a = self.data.attr(md_nr, &arg_name);
                    if a != usize::MAX {
                        let expected = self.data.attr_type(md_nr, a);
                        if Self::seeds_collection_hint(&expected)
                            || self.interpolation_target(&expected) != u32::MAX
                        {
                            self.expected = expected;
                        }
                    }
                }
                let mut p = Value::Null;
                let t = self.expression(&mut p);
                self.expected = Type::Unknown(0);
                named_args.push((arg_name, p, t));
                // accept a trailing comma on the last named arg.
                if !self.lexer.has_token(",") || self.lexer.peek_token(")") {
                    break;
                }
                continue;
            }
            if in_named && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Positional argument after named argument"
                );
            }
            // #432 — `list[0]` is the receiver (attribute 0), so `list.len()` is the
            // attribute index of the explicit argument about to be parsed.  Seed a
            // bare vector-literal argument's element width from that parameter type,
            // matching the free-function path in `parse_call`.
            // Same rule as the free-function path: the channel is this argument's,
            // so a nested call does not inherit the enclosing one's expectation.
            self.expected = Type::Unknown(0);
            if md_nr != u32::MAX && list.len() < self.data.attributes(md_nr) {
                let expected = self.data.attr_type(md_nr, list.len());
                // @PLN124 — a format-string argument to a METHOD builds the
                // parameter's type too (`db.run("… {id} …")`), which is the shape a
                // library API actually presents.
                if Self::seeds_collection_hint(&expected)
                    || self.interpolation_target(&expected) != u32::MAX
                {
                    self.expected = expected;
                }
            }
            let mut p = Value::Null;
            arg_pos.push(self.lexer.peek_pos().clone());
            let t = self.expression(&mut p);
            self.expected = Type::Unknown(0);
            types.push(t);
            list.push(p);
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        if md_nr == u32::MAX {
            // No callee to resolve names against — `call_with_named` would index a
            // definition that is not there.  Hand it on unchanged; the missing method
            // is what gets reported.
            return self.call_nr(val, md_nr, &list, &types, true, &arg_pos, None);
        }
        self.call_with_named(val, md_nr, &list, &types, &named_args, true, &arg_pos, None)
    }

    pub(crate) fn parse_parameters(&mut self) -> (Vec<Type>, Vec<Value>) {
        let mut list = vec![];
        let mut types = vec![];
        if self.lexer.has_token(")") {
            return (types, list);
        }
        loop {
            let mut p = Value::Null;
            types.push(self.expression(&mut p));
            list.push(p);
            if !self.lexer.has_token(",") {
                break;
            }
        }
        self.lexer.token(")");
        (types, list)
    }

    /// Parse `parallel { arm1; arm2; ... }`.
    /// Each semicolon-separated expression in the block becomes one concurrent arm.
    // @F33 — par(...) parallel for-loop
    pub(crate) fn parse_parallel(&mut self, code: &mut Value) {
        self.lexer.token("{");
        // INVARIANT (load-bearing — the capture check below depends on it).
        // Snapshot which vars are already DEFINED at the block's opening brace.
        // The two-pass parser pre-populates the whole function's var table in
        // pass 1, so var-nr ordering CANNOT separate an enclosing var from an
        // arm-local one.  The signal that does is: **in this (non-first) pass,
        // `is_defined` is set in source order**, so a var defined *before* the
        // block reads defined here (enclosing) and one first defined *inside* an
        // arm reads undefined (arm-local).  Params read defined (`become_argument`)
        // ⇒ enclosing.  Two known exceptions read defined-at-entry despite being
        // arm-local — for-loop vars (handled by the `was_loop_var` exclusion) and
        // compiler temps (the `_`/`#` name exclusion); both live in `is_user_var`.
        // If a future change sets `is_defined` out of this source order, the
        // monotonicity `debug_assert!` in `note_mutation`/`note_param` and the
        // `tests/scripts/171-parallel-armlocal-ok.loft` arm-local-compiles guard
        // are the alarms.
        let enclosing: Vec<bool> = (0..self.vars.count())
            .map(|v| self.vars.is_defined(v))
            .collect();
        let mut arms = Vec::new();
        while !self.lexer.peek_token("}") {
            let mut arm = Value::Null;
            self.expression(&mut arm);
            if arm != Value::Null {
                arms.push(arm);
            }
            self.lexer.has_token(";");
        }
        self.lexer.token("}");
        if !self.first_pass {
            if arms.is_empty() {
                diagnostic!(
                    self.lexer,
                    Level::Warning,
                    code = "empty-parallel-block",
                    "Empty parallel block"
                );
                self.lexer.fix_last(crate::diagnostics::Fix {
                    kind: crate::diagnostics::FixKind::Mechanical,
                    title: "delete the empty `parallel` block".to_string(),
                    condition: None,
                    edit: None,
                    concept: "par",
                    concept_ref: "@F33",
                });
            }
            self.reject_unsound_parallel_captures(&arms, &enclosing);
        }
        *code = Value::Parallel(arms);
    }

    /// Soundness floor for `parallel {}` (plan-57 Bug 2).  An arm runs in an
    /// isolated worker — a read-only clone of the heap plus a private stack — so
    /// only *reading* an enclosing local is sound (the value is copied in).
    /// Everything else is the unbuilt/broken surface and must be a clean compile
    /// error, not a silent no-op or a crash:
    /// - **writing or mutating** an enclosing local — the write is dropped
    ///   (scalar/text) or crashes on the read-only store clone (heap);
    /// - **capturing a parameter** (read or write) — SIGSEGVs at teardown.
    ///
    /// Reads of enclosing locals (any position/type) stay legal — that is the
    /// proven-sound P245 surface that test-81 guards.  Known residual: passing a
    /// captured heap value to a function that mutates it is transitive and is not
    /// caught here (it still faults at runtime); catching it needs callee
    /// analysis.  The full capture model is deferred to its driving consumer (the
    /// server/client library) — see the plan-57 deferred-follow-ups.
    fn reject_unsound_parallel_captures(&mut self, arms: &[Value], enclosing: &[bool]) {
        let mut viol: Vec<(u16, ParViolation)> = Vec::new();
        for arm in arms {
            self.collect_parallel_violations(arm, enclosing, &mut viol);
        }
        let mut reported: Vec<u16> = Vec::new();
        for (v, _) in &viol {
            if reported.contains(v) {
                continue;
            }
            reported.push(*v);
            // A var flagged as both Param and Mutation reads clearest as Param.
            let is_param = viol
                .iter()
                .any(|(v2, k)| v2 == v && *k == ParViolation::Param);
            let name = self.vars.name(*v).to_string();
            if is_param {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot capture function parameter '{name}' inside a parallel arm — \
                     a parallel arm runs in an isolated worker with no safe access to the \
                     parent frame; copy '{name}' into a local before the block, or pass it \
                     to a function-call arm"
                );
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot write or mutate enclosing-scope variable '{name}' inside a \
                     parallel arm — an arm runs in an isolated worker, so the write does not \
                     propagate (and heap mutation crashes); a parallel arm may only READ \
                     enclosing state, not write it"
                );
            }
        }
    }

    /// Recursive walk collecting capture violations in one arm (see
    /// `reject_unsound_parallel_captures`).  Reads of enclosing non-parameter
    /// locals are sound and are never flagged.
    fn collect_parallel_violations(
        &self,
        node: &Value,
        encl: &[bool],
        out: &mut Vec<(u16, ParViolation)>,
    ) {
        match node.unspan() {
            // Direct assignment target (scalar `=`/`+=`, vector concat-reassign).
            Value::Set(v, rhs) => {
                self.note_mutation(*v, encl, out);
                self.collect_parallel_violations(rhs, encl, out);
            }
            Value::TuplePut(v, _, rhs) => {
                self.note_mutation(*v, encl, out);
                self.collect_parallel_violations(rhs, encl, out);
            }
            // In-place / element / field mutation hides the host in args[0].
            Value::Call(d, args) => {
                if is_mutating_op(self.data.def(*d).name())
                    && let Some(host) = args.first().and_then(Value::base_var)
                {
                    self.note_mutation(host, encl, out);
                }
                for a in args {
                    self.collect_parallel_violations(a, encl, out);
                }
            }
            // CallRef's first field is the var holding the fn-ref — a capture if
            // it is an enclosing parameter.
            Value::CallRef(v, args) => {
                self.note_param(*v, encl, out);
                for a in args {
                    self.collect_parallel_violations(a, encl, out);
                }
            }
            // Any reference to a var: flagged only if it is a captured parameter.
            Value::Var(v) | Value::TupleGet(v, _) | Value::FnRefDnr(v) => {
                self.note_param(*v, encl, out);
            }
            Value::FnRef(_, v, _) => self.note_param(*v, encl, out),
            // Container recursion.
            Value::Insert(ls) | Value::Tuple(ls) | Value::Parallel(ls) => {
                for x in ls {
                    self.collect_parallel_violations(x, encl, out);
                }
            }
            Value::Block(b) | Value::Loop(b) => {
                for x in &b.operators {
                    self.collect_parallel_violations(x, encl, out);
                }
            }
            Value::Return(b) | Value::Drop(b) | Value::Yield(b) => {
                self.collect_parallel_violations(b, encl, out);
            }
            Value::If(c, t, e) => {
                self.collect_parallel_violations(c, encl, out);
                self.collect_parallel_violations(t, encl, out);
                self.collect_parallel_violations(e, encl, out);
            }
            Value::Iter(_, a, b, c) => {
                self.collect_parallel_violations(a, encl, out);
                self.collect_parallel_violations(b, encl, out);
                self.collect_parallel_violations(c, encl, out);
            }
            _ => {}
        }
    }

    /// True if `v` was already defined when the parallel block opened — i.e. it
    /// is an enclosing-scope variable, not one declared inside an arm.
    fn is_enclosing(v: u16, encl: &[bool]) -> bool {
        (v as usize) < encl.len() && encl[v as usize]
    }

    /// Flag a write/mutation of an enclosing **user** local.  Compiler temps
    /// (`__work`/`__vdb`) carry the codegen for reads/format-strings and are not
    /// user captures.
    fn note_mutation(&self, v: u16, encl: &[bool], out: &mut Vec<(u16, ParViolation)>) {
        if Self::is_enclosing(v, encl) && self.is_user_var(v) {
            self.assert_enclosing_invariant(v);
            out.push((v, ParViolation::Mutation));
        }
    }

    /// Flag a capture of an enclosing **parameter** (read or write both fault).
    fn note_param(&self, v: u16, encl: &[bool], out: &mut Vec<(u16, ParViolation)>) {
        if Self::is_enclosing(v, encl) && self.is_user_var(v) && self.vars.is_argument(v) {
            self.assert_enclosing_invariant(v);
            out.push((v, ParViolation::Param));
        }
    }

    /// Guard the `parse_parallel` enclosing-snapshot invariant.  A var the
    /// block-entry snapshot marked enclosing (`is_defined` was true at entry) must
    /// still read defined now — `is_defined` is monotonic across the block parse.
    /// If it does not, the snapshot has desynced from the var table and the
    /// enclosing/arm-local split is unsound.  (This catches `is_defined` being
    /// *cleared* mid-parse; it cannot catch it being *set* out of source order —
    /// that failure is undetectable from `is_defined` alone, which is what the
    /// `171-parallel-armlocal-ok.loft` compile guard exists for.)
    fn assert_enclosing_invariant(&self, v: u16) {
        debug_assert!(
            self.vars.is_defined(v),
            "parallel-capture invariant broken: enclosing var '{}' lost is_defined \
             mid-parse — the block-entry snapshot no longer matches the var table \
             (see parse_parallel)",
            self.vars.name(v)
        );
    }

    /// Whether `v` is a user variable that an arm could genuinely capture —
    /// excludes the codegen artefacts that `is_defined` would otherwise misread
    /// as enclosing:
    ///   * compiler temps, named with a leading `_` (`__work`, `__vdb`,
    ///     `_match_subj`, `_elm`, `_vector`) or a `#` (`i#index`, `i#next`);
    ///   * for-loop iteration variables (`was_loop_var`) — the loop desugar
    ///     marks them defined in pass 1, but the loop's own advance of its var
    ///     must not read as an enclosing write.
    fn is_user_var(&self, v: u16) -> bool {
        let n = self.vars.name(v);
        !n.starts_with('_') && !n.contains('#') && !self.vars.was_loop_var(v)
    }
}
