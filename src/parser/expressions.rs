// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

use super::{
    AmpHead, Level, LexItem, Parser, Parts, Type, Value, diagnostic_format, v_block, v_if, v_loop,
    v_set,
};
use crate::data::Deps;

/// @PLN86 step 0.1 — maximum expression-nesting depth allowed inside a sandboxed
/// def's body.  Hostile deep nesting (`((((…))))`) drives the recursive-descent
/// parser into a native stack overflow (rc=139); past this bound the parser
/// rejects with a clean diagnostic at LOAD time instead.
///
/// Each nesting level costs ≈15 KB of native stack (measured; the
/// `expression → operators → part → single` chain — #559's null-flow parse logic
/// grew it from ≈10 KB), so the bound must be REACHABLE without overflowing the
/// stack the parser runs on: 128 levels ≈ 1.8 MB.  The parser runs on the process
/// main thread (8 MB default), so that is a ≈4.5× margin, and 128 is still far
/// deeper than any hand-written script nests.  (Host-configurable later, per the
/// plan.)  NB the margin against a bare ≥2 MB embedding is now thin — restoring it
/// would mean shrinking the per-level frame or lowering this bound.
pub(crate) const SANDBOX_MAX_PARSE_DEPTH: u32 = 128;

/// @PLN86 2.4 — the leftmost base variable of a field/index LHS: `s.heading` /
/// `v[i]` / `a.b.c` all descend through the `OpGet*` chain (the base is arg 0) to
/// the root `s` / `v` / `a`.  `None` when the base is not rooted in a variable.
fn lhs_root_var(v: &Value) -> Option<u16> {
    match v.unspan() {
        Value::Var(slot) => Some(*slot),
        Value::Call(_, args) => args.first().and_then(lhs_root_var),
        _ => None,
    }
}

/// P194 helper — extract the host reference and base position from
/// the leftmost `OpGet*` leaf of a tuple-typed field read.  Returns
/// `(host_ref, first_element_position)` where `first_element_position`
/// equals `host_field_pos + element_offset[0]`.  For nested-tuple
/// reads, recurses into the inner `Value::Tuple`.
fn leaf_tuple_lhs(v: &Value) -> Option<(Value, i32)> {
    // Plan-07 phase 1: unspan() so wraps on `.` (step 1.12) don't
    // hide the field-read shape of the tuple-LHS.
    match v.unspan() {
        Value::Call(_, args) if args.len() >= 2 => {
            if let Value::Int(p) = &args[1] {
                Some((args[0].clone(), *p))
            } else {
                None
            }
        }
        Value::Tuple(inner) if !inner.is_empty() => leaf_tuple_lhs(&inner[0]),
        _ => None,
    }
}

/// The value an uncomputable result takes in a slot of type `tp` whose range is `spec` —
/// the one place that chooses between the two rules `formal/operational.md` states.
///
/// @FR-E-Uncomp answers **null**, and that is the answer wherever the slot can hold one:
/// `null` says "this did not happen", which is what an overflow or a divide by zero is.
/// @FR-E-Uncomp-NN is the completion for a slot that CANNOT hold null — it takes the
/// type's DEFAULT instead, the value it would have had if nobody had assigned.
///
/// Which of the two applies is decided by the WRAPPER, not by the width spelling, and
/// that is the whole content of this function: `integer limit(0,255)?` and `u8?` are the
/// same range and the same nullability, so they take the same answer.  Read off the two
/// range paths separately they did not — the `limit(…)` spelling asked `Type::Optional`
/// and the narrow ALIAS asked nothing at all, so a `u8?` overflow answered `0` where the
/// rule requires null and `??`, the documented recovery, was inert on it (D-op-8,
/// loft#1246).  A local's slot is a full i64, and `OpRangeDefault` passes `i64::MIN`
/// through untouched, so the null reaches a narrow FIELD as the field store's own
/// sentinel rather than as a number.
///
/// The non-nullable ANSWER is not decided here: `IntegerSpec::default_value` is its one
/// home, shared with `data::to_default` and the native generator, which answered it
/// separately until loft#1254.
/// Does the slot this store TARGETS hold null?
///
/// The one home for a question `Type::Optional` on a write target cannot answer, because it
/// means two different things there.  For a variable or a field it is the DECLARED
/// nullability and the answer.  For an ELEMENT it is not: `(N-Domain)` makes an index
/// expression nullable for the MISS, so `v[i]` on a NON-nullable `vector<u8>` presents its
/// target as `u8?` — *"this read may miss"* wearing the spelling of *"this slot holds null"*.
///
/// `parent` is what the place is read out of, so a collection there carries the DECLARED
/// element type and `Type::content` is the existing home that unwraps it: `vector<u8>` says
/// no, `vector<u8?>` says yes, and everything that is not a collection falls back to the
/// target's own wrapper.
///
/// This blocked loft#1249 for a session.  Bounding the store seam by the target's `Optional`
/// looked right and wrote the null sentinel into a non-null `vector<u8>` element, which the
/// store flattened to `0` — measured in published `hex_field`, where `255` became `0`.
pub(crate) fn target_holds_null(target: &Type, parent: &Type) -> bool {
    // `&τ` first: `AssignPlace::parent_tp` is documented as `&S` for a write inside a
    // `&`-parameter, and the referent is what carries the element type.  Missing this peel
    // read a `&vector<u8>` as "not a collection" and fell through to the target's own
    // wrapper — which for an element write is the out-of-bounds one — so published `assets`
    // wrote a fully-opaque alpha of `255` into a non-null `vector<u8>` and got `null`.
    let parent = match parent {
        Type::RefVar(inner) => inner.as_ref(),
        other => other,
    };
    if crate::parser::vectors::is_collection(parent) {
        matches!(parent.content(), Type::Optional(_))
    } else {
        matches!(target, Type::Optional(_))
    }
}

fn uncomputable_default(nullable: bool, spec: &crate::data::IntegerSpec) -> i64 {
    // C85 says an overflow writes the RESERVED sentinel into a non-null slot, which then
    // reads as null — so a non-null slot answers null exactly when its type kept a code back
    // for one.  `i32` is `i32::MIN + 1 ..= i32::MAX` and `u32` is `0 ..= u32::MAX - 1`
    // whatever the `?`, so both have that code; `u8`/`i8`/`u16`/`i16` fill their width and
    // have none (loft#1296).
    if nullable || spec.reserves_sentinel_unconditionally() {
        i64::MIN
    } else {
        spec.default_value()
    }
}

/// loft#984 — the DECLARED range of a store target, and the default a value outside it
/// takes: `(lo, hi, default)`.  The default is `uncomputable_default`'s; this function
/// answers only which range applies.
///
/// `None` when the question does not arise — the target declares no range of its own (the
/// plain `integer` / i32 templates), or it is not an integer at all.
///
/// ⚠ **It answers the `limit(lo, hi)` spelling and NOT the narrow ALIASES.** It returns `None`
/// the moment `forced_size` is set, which is exactly what `u8` / `i8` / `u16` / `i16` / `u32`
/// are — so a caller reaching for this to guard a narrow width silently guards nothing, with
/// no error to notice. [`Parser::compound_range`] is the one that partitions both, and is what
/// a caller wanting *"whatever range this target declares, however it was spelled"* must use.
/// Measured: a guard written against this function claimed no store at all until it was
/// switched (@PLN152 step 2).
fn declared_range(tp: &Type, nullable: bool) -> Option<(i64, i64, i64)> {
    let Type::Integer(spec) = tp.base() else {
        return None;
    };
    if spec.is_wide_template() || spec.is_signed32_template() {
        return None;
    }
    // The `limit(lo, hi)` spelling, and a NULLABLE narrow alias.
    //
    // A NON-NULLABLE narrow ALIAS (`u8`/`i8`/`u16`/`i16`/`i32`, which is what `forced_size`
    // marks) is already guarded at COMPILE time — storing a plain integer into one is an
    // error demanding `as u8` — so a runtime default there would be redundant where the
    // check holds and WRONG where it does not: it fired 24 times inside the stdlib's own
    // `i8` stores and handed them `-128`.  Neither half of that reasoning survives the `?`.
    // The compile-time check does not fire (@FR-I-Narrow-Opt makes the narrowing implicit
    // and CHECKED for a nullable target), and the slot has a reserved edge a value can land
    // on — so this is the seam that has to bound it (@FR-N-Reserve, loft#1249).
    //
    // `nullable` is the CALLER's answer and not `matches!(tp, Optional(_))`, which is what
    // makes this safe: an element write on a non-null `vector<u8>` presents its target as
    // `u8?` for the out-of-bounds MISS, and bounding that by the usable range wrote the
    // sentinel into a slot that cannot hold one.  `target_holds_null` is the one home.
    if spec.forced_size.is_some() && !nullable {
        return None;
    }
    // @FR-N-Reserve — a nullable narrow slot's bound is its USABLE range: the sentinel is a
    // value of the type, so a `u8?` is `0..=254` (loft#1249).  `usable_*` answers the
    // declared bound unchanged for every non-nullable spec and for every width with a spare
    // code, so passing `nullable` here only adds the case that reserves an edge.
    let lo = i64::from(spec.usable_min(nullable));
    let hi = spec.usable_max(nullable);
    // @FR-E-Uncomp / @FR-E-Uncomp-NN — `uncomputable_default` is the one home for which
    // of the two applies, shared with the compound path.  The default is NOT the range's
    // floor: `lo` is zero for `u8` and so looked right, while an `i16` answered `-32768`
    // and an `i32` `-2147483647` — in range, type-correct, and as unrelated to the
    // computation as a wrapped value would be.
    Some((lo, hi, uncomputable_default(nullable, spec)))
}

/// The base variable at the root of an lvalue access chain, or `u16::MAX` if the
/// chain does not bottom out in a plain variable.  A field/element access lowers to
/// `Call(op, [inner, …])` whose FIRST argument is the object being accessed
/// (`s.a.b[i]` → `Call(idx, [Call(f_b, [Call(f_a, [Var(s), …]), …]), i])`), so the
/// base is found by walking `args[0]` to the leaf `Var`.  @PLN40 step 3 uses this to
/// find which binding a component write (`p.x = …`, `p[i] = …`) mutates THROUGH, so a
/// write through a value-const binding can be rejected at its root.
fn lhs_base_var(v: &Value, data: &crate::parser::Data) -> u16 {
    match v.unspan() {
        Value::Var(nr) => *nr,
        // Exactly two `if`s reach the left of an assignment, and both name their place through
        // the THEN arm.  loft#980's variant-field guard — `if tag(c) ∈ declaring { c } else
        // { null }` — carries the receiver there, and a bare-variable NULL DISCHARGE carries
        // its SUBJECT there (`v?.x = …`, whose else arm is the type's default instead of the
        // sentinel).  The subject is what a write through the discharge reaches on the path
        // that reaches anything, so resolving to it is what keeps `const` binding through
        // `h.i?.x = …`: while this answered "no variable at all", `validate_write` had no base
        // to check and mutated a `const` parameter in silence (loft#1211).
        Value::If(_, then, _) => lhs_base_var(then, data),
        // …and a discharge whose subject is NOT a bare variable binds it to a temp inside an
        // `ncc`/`ncr` block instead.  Only a discharge block is seen through: a
        // `fn_ref_field_read` / `tuple_unbox` block is a different question, and answering it
        // here would be guessing.
        Value::Block(_) => {
            null_discharge_subject(v, data).map_or(u16::MAX, |s| lhs_base_var(s, data))
        }
        Value::Call(_, args) if !args.is_empty() => lhs_base_var(&args[0], data),
        _ => u16::MAX,
    }
}

/// The SUBJECT a NULL DISCHARGE was applied to — `e` in `e?`, `e ?? d`, `e ?? return` — when
/// `v` is one, and `None` when it is not.
///
/// ONE home for the two shapes the discharge lowering produces, because two questions read it
/// and each used to carry its own matcher: which variable a place ROOTED in a discharge writes
/// through ([`lhs_base_var`], so `const` still binds through `h.i?.x = …`), and which place a
/// discharge that IS the target was reading ([`Parser::peel_place_discharge`], `@FR-E-Asgn-Discharge`).
/// A non-trivial subject is bound to a `__ncc_N` / `ncr_N` temp by the block's own head
/// statement; a bare VARIABLE subject can be read twice for free, so it lowers to a plain `if`
/// with no temp and the subject stands in the then arm.
fn null_discharge_subject<'a>(v: &'a Value, data: &crate::parser::Data) -> Option<&'a Value> {
    match v.unspan() {
        Value::Block(bl) if bl.name == "ncc" || bl.name == "ncr" => {
            match bl.operators.first().map(Value::unspan) {
                Some(Value::Set(_, subject)) => Some(subject),
                _ => None,
            }
        }
        // loft#980's variant-field guard is the other `if` that reaches a left-hand side; its
        // zero-argument null-sentinel ELSE arm is what tells the two apart.  A discharge's else
        // arm is the type's DEFAULT, so the guard is the only `if` this must not claim — its
        // then arm is the receiver, and peeling to it would write through the wrong place.
        //
        // ⚠ Sound on a LEFT-hand side only, where no author's `if` can stand.  A reader of a
        // right-hand side meets every value-`if` an author writes in this same node, and must
        // ask the builder instead (`Parser::bare_variable_discharge`, loft#1379).
        Value::If(_, then, els)
            if !matches!(els.unspan(), Value::Call(d, a)
                if a.is_empty() && data.def(*d).name() == "OpNullRefSentinel") =>
        {
            Some(then)
        }
        _ => None,
    }
}

/// Returns true if `val` contains a `Set(r, _)` node at any depth.
/// Used to find which block statement first assigns an inline-ref temporary.
/// Descent comes from `Value::for_each_child`, the one place that knows the IR
/// tree's shape, so every compound variant is reached and a new one is inherited
/// rather than needing an arm here (A15).  The answer is deliberately the WIDE
/// one: a `Set` at any depth counts, because the first statement that assigns the
/// temporary is the correct null-init insertion point wherever it sits.
fn inline_ref_set_in(val: &Value, r: u16) -> bool {
    val.any_node(&mut |n| matches!(n, Value::Set(v, _) if *v == r))
}

/// P248 — extracted nested-tuple LHS shape for assignment.
///
/// `t.0.1.2 = …` parses to either a single `Value::TupleGet` (depth 1)
/// or a chain of `Block[Set(w_k, …), TupleGet(w_k, idx)]` nodes (depth
/// ≥ 2 — operators.rs case 3 materialises the intermediate reads
/// through work vars).  This struct flattens both shapes so the
/// assignment dispatcher can rewrite them uniformly.
///
/// For `t.0.1 = 99`:
///   - root = t_var_nr
///   - chain = [(w0, 0)]   // w0 = t.0
///   - leaf_idx = 1        // …w0.1 = rhs
///
/// For `t.0 = 99`:
///   - root = t_var_nr
///   - chain = []
///   - leaf_idx = 0
pub(super) struct NestedTupleLhs {
    root: u16,
    /// Pairs of `(work_var, index_into_parent)`, ordered root → leaf.
    chain: Vec<(u16, u16)>,
    leaf_idx: u16,
}

/// Walk a Value that might be a chained tuple read (single `TupleGet`
/// or nested `Block[Set(w, source), TupleGet(w, idx)]`) and return a
/// flattened `NestedTupleLhs`.  Returns `None` for any other shape.
pub(super) fn extract_nested_tuple_lhs(code: &Value) -> Option<NestedTupleLhs> {
    match code.unspan() {
        Value::TupleGet(var_nr, idx) => Some(NestedTupleLhs {
            root: *var_nr,
            chain: vec![],
            leaf_idx: *idx,
        }),
        Value::Block(b) if b.operators.len() == 2 => {
            let (set_op, tail) = (&b.operators[0], &b.operators[1]);
            if let (Value::Set(w_set, source), Value::TupleGet(w_get, leaf_idx)) = (set_op, tail)
                && w_set == w_get
            {
                let inner = extract_nested_tuple_lhs(source)?;
                let mut chain = inner.chain;
                chain.push((*w_set, inner.leaf_idx));
                Some(NestedTupleLhs {
                    root: inner.root,
                    chain,
                    leaf_idx: *leaf_idx,
                })
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Build the assignment IR for `<lhs> = rhs`.  For depth-1 it's a
/// plain `TuplePut(root, leaf_idx, rhs)`.  For depth ≥ 2 we keep the
/// existing Block's `Set` ops (they read intermediates into the same
/// work vars), strip the trailing `TupleGet`, write the leaf via
/// `TuplePut(deepest_w, leaf_idx, rhs)`, then write each intermediate
/// back to its parent in reverse so the modification propagates up
/// to `root`.
pub(super) fn build_nested_tuple_assign(
    orig_code: &Value,
    lhs: &NestedTupleLhs,
    rhs: Value,
) -> Value {
    if lhs.chain.is_empty() {
        return Value::TuplePut(lhs.root, lhs.leaf_idx, Box::new(rhs));
    }
    // Reuse the Set ops the existing Block already emitted (read
    // chain into the same work-var slots).  Strip the trailing
    // TupleGet — we're replacing it with writes.
    let mut ops: Vec<Value> = if let Value::Block(b) = orig_code.unspan() {
        let mut clone = b.operators.clone();
        if matches!(clone.last(), Some(Value::TupleGet(_, _))) {
            clone.pop();
        }
        clone
    } else {
        // Defensive — shouldn't happen since chain.len() >= 1 implies
        // we matched the Block arm above.  Reconstruct the reads.
        let mut prev = lhs.root;
        let mut acc = Vec::with_capacity(lhs.chain.len());
        for (w, idx) in &lhs.chain {
            acc.push(crate::data::v_set(*w, Value::TupleGet(prev, *idx)));
            prev = *w;
        }
        acc
    };
    let last_w = lhs.chain.last().expect("chain non-empty").0;
    // Leaf: write rhs into the deepest work var at leaf_idx.
    ops.push(Value::TuplePut(last_w, lhs.leaf_idx, Box::new(rhs)));
    // Writebacks in reverse — propagate the modified intermediate
    // back up the chain so the change reaches `root`.
    for (k, (w, idx)) in lhs.chain.iter().enumerate().rev() {
        let parent = if k == 0 { lhs.root } else { lhs.chain[k - 1].0 };
        ops.push(Value::TuplePut(parent, *idx, Box::new(Value::Var(*w))));
    }
    crate::data::v_block(ops, Type::Void, "nested_tuple_assign")
}

/// Recursively replace every occurrence of `from` in `into` with a clone
/// of `to`.  Used by the `field += elem` keyed-collection branch to
/// retarget a struct-literal's field-init steps from the LHS field
/// expression onto a freshly-allocated element variable.
/// @P390 — does `val` reference `Value::Var(target)` anywhere (a READ of the
/// variable)?  Mirrors `replace_var_in_ir`'s traversal so it covers every `Value`
/// variant soundly.  Used to detect a self-aliasing slice-assign (`v = v[a..b]`),
/// where the materialise-into-iterator would read the same variable it is about
/// to `OpClearVector`.  `Set`/`TuplePut` slots are NOT counted (they are writes,
/// matching the walker) — only `Var(target)` reads.
fn ir_mentions_var(val: &Value, target: u16) -> bool {
    match val {
        Value::Var(v) => *v == target,
        Value::Int(_)
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
        | Value::Null => false,
        Value::Call(_, args)
        | Value::CallRef(_, args)
        | Value::Insert(args)
        | Value::Tuple(args)
        | Value::Parallel(args) => args.iter().any(|a| ir_mentions_var(a, target)),
        Value::Block(bl) | Value::Loop(bl) => {
            bl.operators.iter().any(|op| ir_mentions_var(op, target))
        }
        Value::Set(_, body)
        | Value::Return(body)
        | Value::Drop(body)
        | Value::TuplePut(_, _, body)
        | Value::Yield(body) => ir_mentions_var(body, target),
        Value::If(cond, t, f) => {
            ir_mentions_var(cond, target)
                || ir_mentions_var(t, target)
                || ir_mentions_var(f, target)
        }
        Value::Iter(_, a, b, c) => {
            ir_mentions_var(a, target) || ir_mentions_var(b, target) || ir_mentions_var(c, target)
        }
        Value::Span(b) => ir_mentions_var(&b.1, target),
    }
}

pub(crate) fn substitute_value(into: &mut Value, from: &Value, to: &Value) {
    if into == from {
        *into = to.clone();
        return;
    }
    match into {
        Value::Call(_, args) | Value::CallRef(_, args) => {
            for a in args {
                substitute_value(a, from, to);
            }
        }
        Value::Block(bl) | Value::Loop(bl) => {
            for s in &mut bl.operators {
                substitute_value(s, from, to);
            }
        }
        Value::Insert(steps) => {
            for s in steps {
                substitute_value(s, from, to);
            }
        }
        Value::If(c, t, e) => {
            substitute_value(c, from, to);
            substitute_value(t, from, to);
            substitute_value(e, from, to);
        }
        Value::Set(_, v)
        | Value::Return(v)
        | Value::Drop(v)
        | Value::Yield(v)
        | Value::TuplePut(_, _, v) => substitute_value(v, from, to),
        Value::Iter(_, a, b, c) => {
            substitute_value(a, from, to);
            substitute_value(b, from, to);
            substitute_value(c, from, to);
        }
        Value::Tuple(vs) | Value::Parallel(vs) => {
            for v in vs {
                substitute_value(v, from, to);
            }
        }
        // Plan-07 phase 1 — Span is transparent; recurse into the
        // wrapped node.
        Value::Span(b) => substitute_value(&mut b.1, from, to),
        // Plan-06 spine step 3 — recurse into all child Values.
        // Leaf variants.
        Value::Null
        | Value::Int(_)
        | Value::Enum(_, _)
        | Value::Boolean(_)
        | Value::Float(_)
        | Value::Long(_)
        | Value::Single(_)
        | Value::Text(_)
        | Value::Var(_)
        | Value::Line(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::Keys(_)
        | Value::TupleGet(_, _)
        | Value::FnRef(_, _, _)
        | Value::FnRefDnr(_) => {}
        // Phase 09 phase 00 step 0.7 — codegen-internal.
        Value::RawExpr(_) => {}
    }
}

/// @PLN85 D-own-1 / C86 — the whole-value VECTOR bind verdict (see
/// `classify_vec_bind`).  DESIGN_DECISIONS C86: a whole-value heap bind
/// COPIES by contract; aliasing exists only as the post-parse last-use
/// ELISION (`use_analysis::elision_plans` → `scopes::elide_borrows` —
/// the rustc rule, as an optimization).  Projections stay views (#426).
enum VecBind {
    /// `v = v` — the identity: emit nothing (a clear + re-append off the
    /// same storage would free the store the RHS is about to read).
    SelfAssign,
    /// `b = a` — a whole-var vector bind COPIES: give `b` its own store
    /// and deep-copy `a`'s elements (aliasing dangled when `a`'s scope
    /// exited, P292, and left `b` slot-less on first assignment, P394).
    CopyVar,
    /// `af = bx.v` where the base struct OWNS its store (empty deps) —
    /// the #415 whole-value field bind copies like `b = a` (C86).  A
    /// BORROWED base (non-empty deps) never reaches this verdict: its
    /// field read stays a view so an in-place write-through
    /// (`cells = sc.v; cells[i] = h`) reaches the source (@PLN25 p379).
    /// The owns-vs-borrows split is the `deps` ownership fact — the same
    /// answer `use_analysis::ownership_of` reconstructs post-parse
    /// (Owned ⇒ copy, Borrowed/Join ⇒ view); the parser reads the var's
    /// incrementally-maintained deps because the oracle's whole-body
    /// `Defs` walk does not exist mid-parse.
    CopyOwnedField,
    /// Not a whole-value vector bind: vector INDEX reads (`a = vv[0]`)
    /// and NESTED field reads (`c = o.inner.v`) stay ALIASED until the
    /// store-reuse substrate is fixed (#426 routed forward — widening
    /// the copy freed the source at the read and a 3-deep build into
    /// the recycled store corrupted, `185-nested-boolean-vector`), and
    /// every other RHS shape belongs to a different branch.
    NotABind,
}

impl Parser {
    /// Does this expression hand back storage it did NOT allocate — so a temp bound
    /// to it only NAMES that storage and must not free it?
    ///
    /// The question is about the expression's PRODUCER, so it looks at the call and
    /// not at the value's type: a `vector<T>` from a call and a `vector<T>` from an
    /// allocating cast have the same type and opposite ownership.
    ///
    /// There are two ways to be handed something you do not own, and the ownership
    /// fact lives in a different place for each.
    ///
    /// **A return buffer** (loft#906). A heap-returning function writes its result
    /// into a caller-allocated buffer passed as a hidden trailing `__retbuf`
    /// parameter (the NRVO ABI), so the value it answers IS that buffer — allocated
    /// once at the call site and, inside a loop, reused across every iteration.
    /// Binding it to a temp that owns therefore frees the buffer once per iteration
    /// while its real owner still holds it. The test is the parameter's `hidden`
    /// FLAG, not its name: a function whose tail promotes a work-ref to BE the
    /// buffer has that parameter RENAMED after the variable it promoted (`fn add(v,
    /// x, out)` rather than `…, __retbuf`), so a name test sees a retbuf on one
    /// shape of heap-returning function and not on the other — which is half a fix,
    /// and the half that still crashes.
    ///
    /// **A view of something else's record** (loft#939). `src.items` answers a
    /// projection into `src`'s record, and the synthetic `Set(tmp, src.items)` this
    /// arm builds after the fact bypasses the parse-time vector deep-copy lowering
    /// — the same reason the borrowed-`Var` arm below builds its temp by
    /// element-append instead — so the temp ALIASES that record rather than copying
    /// out of it. Freeing it at scope exit frees the record's owner: the caller's
    /// argument, or a local that outlives the temp's inner scope.
    ///
    /// The fact is the RHS type's own `deps`, which is what the parser already uses
    /// to answer owns-vs-borrows mid-parse (see [`VecBind::CopyOwnedField`]) —
    /// non-empty deps mean the value names storage reached through something else.
    /// It is read from the TYPE rather than from a list of projection opcodes
    /// because a list is a second copy of the same fact, and the projection op a
    /// later change forgets to add to it is exactly the one that then corrupts in
    /// silence. Both the wrapper and the peeled type are asked, so a `vector<T>?`
    /// (`Optional(Vector(…))`) cannot hide its deps behind the `?`.
    ///
    /// The two arms do not overlap: a return buffer's dep is on a HIDDEN parameter
    /// and does not reach the RHS type as a borrow, which is why the first arm
    /// exists and why neither subsumes the other.
    ///
    /// [`VecBind::CopyOwnedField`]: VecBind::CopyOwnedField
    fn borrows_its_storage(data: &crate::data::Data, rhs: &Value, rhs_tp: &Type) -> bool {
        if !rhs_tp.depend().is_empty() || !rhs_tp.base().depend().is_empty() {
            return true;
        }
        let Value::Call(d_nr, _) = rhs.unspan() else {
            return false;
        };
        data.def(*d_nr)
            .attributes()
            .last()
            .is_some_and(|a| a.hidden)
    }

    /// @PLN10 — wrap every text-dest native called in *value position* in a
    /// scope-bound work-text temp, so its result lives in a freed local instead
    /// of the never-cleared `stores.scratch` buffer.  Replaces
    /// `Call(native, args)` with `Block([Set(w, native()), Var(w)])` where `w`
    /// is a fresh `work_text` — reusing the proven `set_var` dest-pass for the
    /// inner `Set`.  The two codegen fast paths already dest-pass their shapes
    /// directly, so we skip wrapping a *bare* native that is the value of a
    /// `Set` (set_var) or the appended operand of `OpAppendText`
    /// (try_text_dest_pass) — but still recurse into their sub-arguments.
    fn wrap_value_text_dest(&mut self, v: &mut Value) {
        // A text-dest native reached here (not via a fast-path skip) is in
        // value position — recurse into its args (nested natives), then wrap.
        let wrap_here = matches!(
            &*v,
            Value::Call(op, _) if crate::state::codegen::is_text_dest_native(self.data.def(*op).name())
                || crate::state::codegen::is_cdylib_text_call(self.data.def(*op))
                // @PLN24 arc D — a `#c` binding returning text is a value-position
                // producer like the other two, and needs the same work-text temp.
                || crate::state::codegen::is_c_text_call(self.data.def(*op))
        );
        if wrap_here {
            if let Value::Call(_, args) = v {
                for a in args.iter_mut() {
                    self.wrap_value_text_dest(a);
                }
            }
            // Pass-2-only mint (loft#665 piece 2): drawing from the both-pass
            // `__work_N` sequence here would shift every later buffer relative to
            // pass 1.  Its own sequence cannot perturb anyone else's numbering.
            let w = self.vars.work_text_p2(&mut self.lexer);
            let call = std::mem::replace(v, Value::Null);
            *v = v_block(
                vec![v_set(w, call), Value::Var(w)],
                Type::Text(Deps::frame1(w)),
                "synth text dest",
            );
            return;
        }
        match v {
            Value::Call(op, args)
                if self.data.def(*op).name() == "OpAppendText" && args.len() == 2 =>
            {
                self.wrap_value_text_dest(&mut args[0]);
                self.descend_skip_direct(&mut args[1]);
            }
            Value::Call(_, args) | Value::CallRef(_, args) => {
                for a in args.iter_mut() {
                    self.wrap_value_text_dest(a);
                }
            }
            Value::Set(_, rhs) => self.descend_skip_direct(rhs),
            Value::Block(bl) | Value::Loop(bl) => {
                for s in &mut bl.operators {
                    self.wrap_value_text_dest(s);
                }
            }
            Value::Insert(steps) => {
                for s in steps {
                    self.wrap_value_text_dest(s);
                }
            }
            Value::If(c, t, e) => {
                self.wrap_value_text_dest(c);
                self.wrap_value_text_dest(t);
                self.wrap_value_text_dest(e);
            }
            Value::Return(x) | Value::Drop(x) | Value::Yield(x) | Value::TuplePut(_, _, x) => {
                self.wrap_value_text_dest(x)
            }
            Value::Iter(_, a, b, c) => {
                self.wrap_value_text_dest(a);
                self.wrap_value_text_dest(b);
                self.wrap_value_text_dest(c);
            }
            Value::Tuple(vs) | Value::Parallel(vs) => {
                for x in vs {
                    self.wrap_value_text_dest(x);
                }
            }
            Value::Span(b) => self.wrap_value_text_dest(&mut b.1),
            _ => {}
        }
    }

    /// Descend into a fast-path position (a `Set` value or `OpAppendText`'s
    /// appended operand): a *bare* text-dest native here is dest-passed
    /// directly by codegen, so don't wrap it — only recurse into its args
    /// (nested natives still need a temp).  Any other shape walks normally.
    fn descend_skip_direct(&mut self, v: &mut Value) {
        if let Value::Call(op, args) = v.unspan_mut() {
            let is_dest = crate::state::codegen::is_text_dest_native(self.data.def(*op).name())
                || crate::state::codegen::is_cdylib_text_call(self.data.def(*op));
            if is_dest {
                for a in args.iter_mut() {
                    self.wrap_value_text_dest(a);
                }
                return;
            }
        }
        self.wrap_value_text_dest(v);
    }

    // <code> = '{' <block> '}'
    /// Parse the code on the last inserted definition.
    /// This way we can use recursion with the definition itself.
    pub(crate) fn parse_code(&mut self) -> Type {
        let mut v = Value::Null;
        let result = if self.context == u32::MAX {
            Type::Void
        } else {
            self.data.def(self.context).returned().clone()
        };
        self.parse_block("return from block", &mut v, &result);
        self.finish_body(v, result)
    }

    /// Finish a function body: everything a parsed `{ … }` needs before it can be
    /// stored as the current definition's code — the entry preamble (work-text and
    /// work-ref null-inits, promoted-argument seeds, rebind witnesses), the
    /// value-position text-destination wrapping, and the loop return-buffer rotation.
    ///
    /// Split out of [`parse_code`](Self::parse_code) so a body the parser BUILDS can
    /// get the same treatment as one it reads from source: a field default that needs
    /// a temporary is lowered into a function of its own (loft#698), and its body is
    /// one expression rather than a braced block, so there is no `{` for
    /// `parse_block` to consume.  The preamble is exactly what such a body needs —
    /// its temporaries are work-refs, and an uninitialised work-ref is a wild store
    /// pointer — so sharing this is what makes the generated function identical to the
    /// hand-written one it stands in for.
    pub(crate) fn finish_body(&mut self, mut v: Value, result: Type) -> Type {
        // @PLN10 — synth a scope-bound work-text destination for every text-dest
        // native called in *value position*, so its result lives in a freed temp
        // instead of the never-cleared `stores.scratch` buffer.  Runs on the final
        // pass before the work_texts null-init loop below, so the synthesized
        // temps get null-inited + slot-allocated + freed like any work-text.
        if !self.first_pass {
            self.wrap_value_text_dest(&mut v);
        }
        if let Value::Block(bl) = &mut v {
            let ls = &mut bl.operators;
            // @PLN87 P2.1 — stash each rebindable heap param's caller-supplied
            // DbRef into its witness at function entry, as two ordered ops:
            //   Set(__orig, Null)  — first-def for slot assignment; the witness
            //                        is inline_ref so this lowers to a
            //                        non-allocating `OpInitRefSentinel`, NOT the
            //                        store-allocating `OpInitRef` (whose store the
            //                        stash would then orphan — a leak).
            //   OpPutRef(__orig, param) — a RAW DbRef copy (same store_nr as
            //                        param), NOT `Set(__orig, param)` which would
            //                        DEEP-COPY param into a fresh store and defeat
            //                        the distinctness check.
            // It snapshots param's ENTRY store; later rebinds change param's slot
            // but not the witness, so the function-exit `OpFreeRefIfDistinct`
            // (emitted by `scopes::check`) frees a rebound store and never the
            // caller's original.  Inserted before the null-init loops, so a `Set`
            // first-def is present for the inline_ref preamble to anchor on.
            for (param, orig) in self.vars.rebind_params() {
                ls.insert(
                    0,
                    self.cl("OpPutRef", &[Value::Var(orig), Value::Var(param)]),
                );
                ls.insert(0, v_set(orig, Value::Null));
            }
            for wt in self.vars.work_texts() {
                ls.insert(0, v_set(wt, Value::Text(String::new())));
            }
            // copy promoted arguments into their shadow locals at function entry.
            // #685 — a shadow that was flipped to a `__cell_<T>` (a scalar argument a
            // closure mutates) needs its cell ALLOCATED here as well: the seed is the
            // only assignment it is guaranteed to have, since the mutation lives
            // inside the closure and may be its sole write.  Text shadows keep the
            // plain copy.
            for (shadow, original) in self.vars.promoted_text_args() {
                let shadow_tp = self.vars.tp(shadow).clone();
                let seed = crate::parser::vectors::boxed_cell_def(&shadow_tp, &self.data).and_then(
                    |cell| self.boxed_cell_alloc_and_set(shadow, cell, Value::Var(original)),
                );
                ls.insert(
                    0,
                    seed.unwrap_or_else(|| v_set(shadow, Value::Var(original))),
                );
            }
            for r in self.vars.work_references() {
                if std::env::var("LOFT_TRACE_PREAMBLE").is_ok() {
                    eprintln!(
                        "[preamble] pass1={} r={r} name={} arg={} inline={} deps={:?} chb={}",
                        self.first_pass,
                        self.vars.name(r),
                        self.vars.is_argument(r),
                        self.vars.is_inline_ref(r),
                        self.vars.tp(r).depend(),
                        self.vars.is_caller_hidden_buf(r),
                    );
                }
                // @FR-O-Proxy asks alloc — this decides which work-refs get a null-init in
                // the preamble, which is the opposite direction from a free: it puts a slot
                // in a known-absent state, and releases nothing.
                if !self.vars.is_argument(r)
                    && !self.vars.is_inline_ref(r)
                    // @PLAN51 Cluster IV: also null-init caller-side hidden-
                    // buffer work-refs even when their typedef carries a
                    // non-empty dep list (e.g. Reference(td, [arg_idx]) for
                    // if-tail / recursion / explicit-return-in-if shapes).
                    // Without it, the slot allocator skips them ("no
                    // first_def") and codegen panics at codegen.rs:2529.
                    // Empty-dep refs still take this path (the original
                    // arm); caller_hidden_buf is the additional gate.
                    // #319: `__ncc_N` heap-DbRef temps likewise — their only
                    // Set is inside the ncc block, so they need the preamble
                    // init regardless of their dep list.
                    && (self.vars.tp(r).depend().is_empty()
                        || self.vars.is_caller_hidden_buf(r)
                        || self.vars.name(r).starts_with("__ncc_"))
                {
                    ls.insert(0, v_set(r, Value::Null));
                }
            }
            // Inline-ref temporaries (parse_part work-refs for chained ref calls):
            // Insert null-init for each temp immediately BEFORE the statement that
            // first assigns it (the statement containing {Set(r, call_result)}).
            // This ensures scan_set encounters them AFTER the body variables whose
            // stores precede theirs (e.g. `p`), so reversed var_order frees the
            // inline-ref temps BEFORE those body variables — satisfying LIFO.
            //
            // For temps used in the same statement we insert in descending var_nr order
            // so that lower var_nrs end up first in ls (allocated first = freed last).
            {
                let inline_refs = self.vars.inline_ref_references();
                // Build (first_use_position, var_nr) pairs.
                let mut insertions: Vec<(usize, u16)> = Vec::new();
                // Fallback position: after the first non-Line-marker stmt in ls.
                let mut fallback = 0usize;
                while fallback < ls.len() && matches!(ls[fallback], Value::Line(_)) {
                    fallback += 1;
                }
                if fallback < ls.len() {
                    fallback += 1;
                }
                // @FR-O-Proxy asks alloc — the same null-init question as above, for the
                // inline-ref temporaries, placed at each temp's first use rather than in the
                // preamble.  It chooses where an init lands, never whether a store is freed.
                for r in &inline_refs {
                    if !self.vars.is_argument(*r) && self.vars.tp(*r).depend().is_empty() {
                        let pos = ls
                            .iter()
                            .position(|stmt| inline_ref_set_in(stmt, *r))
                            .unwrap_or(fallback);
                        insertions.push((pos, *r));
                    }
                }
                // Insert from end to start to avoid index invalidation; within the
                // same position insert higher var_nr first so lower var_nr lands first.
                insertions.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
                for (pos, r) in insertions {
                    ls.insert(pos, v_set(r, Value::Null));
                }
            }
            // Auto-lock the stores for every const Reference/Vector argument at the very
            // start of the function body (after work-variable initialisations).
            // Applies in all build profiles so that writes to const parameters panic in
            // release builds too (S22 — previously guarded by #[cfg(debug_assertions)]).
            // detect variables that remain Unknown(0) after the second pass.
            // These are names from the first pass that were never resolved — likely typos.
            // Note: `known_var_or_type()` in objects.rs already emits "Unknown variable"
            // during expression parsing for variables with Unknown type or undefined status.
            // This post-parse check catches the complementary case: variables that were
            // assigned (is_defined) but whose type remained Unknown after both passes.
            if !self.first_pass {
                let n_vars = self.vars.next_var();
                for v_nr in 0..n_vars {
                    if self.vars.tp(v_nr).is_unknown()
                        && !self.vars.is_argument(v_nr)
                        && self.vars.is_defined(v_nr)
                        && !self.vars.name(v_nr).starts_with('_')
                    {
                        diagnostic!(
                            self.lexer,
                            Level::Error,
                            "Variable '{}' has unknown type — possible typo or missing definition",
                            self.vars.name(v_nr)
                        );
                    }
                }
            }
            // @P376 follow-up — formerly emitted `n_set_store_lock(p, true)`
            // at function entry for every `const` Reference/Vector parameter.
            // The intent was a defense-in-depth tripwire for compile-time
            // const-check bugs.  In practice, the compile-time check catches
            // every mutation path (`p = X`, `p.f = X`, `p.h[k] = v`,
            // `p.h += ...`, non-`const`-method calls on `p`), and the
            // function-entry lock fires SPURIOUSLY on legitimate iteration
            // over a hash field of `p`: `for x in p.h` calls
            // `build_hash_sorted_vec` which (by C60 piece-3 edit-A design)
            // allocates sort scratch IN THE HASH'S STORE — i.e. `p`'s store —
            // and panics on the locked claim.  Par-worker safety is
            // INDEPENDENT and untouched: `clone_locked` /
            // `borrow_locked_for_light_worker` set `read_only = true` on
            // the worker's borrowed store, so a
            // worker that writes through a `const` arg still panics on
            // `addr_mut`.  See PROBLEMS.md @P376 follow-up + PLANNING.md S22
            // (the S22 motivation — par-worker silent-mutation in release —
            // remains addressed by the clone-side lock).
        }
        if !self.first_pass {
            self.rotate_loop_retbufs(&mut v);
        }
        // Plan-22 phase 02a (2026-05-12): also save body in pass 1
        // so the closure mutation walker can run in pass 1 BEFORE
        // synthesize_closure_record sets attribute types.  The body
        // gets overwritten in pass 2 with the properly-typed
        // version; pass 1's body is only consulted by phase 01's
        // walker (`collect_mutated_captures`) and never by codegen.
        //
        // Risk: any other code path that assumes
        // `data.def(d_nr).code` is Null in pass 1 would break.
        // Verified clean against the regression net (633 issues +
        // 47 wrap + 22 closure_matrix + 6 mut_closure_matrix); if
        // a future regression surfaces, narrow this to only-save
        // when `self.in_lambda` is true.
        if self.context != u32::MAX {
            // loft#1023 — the body of `self.context` now exists for this pass.  A generic
            // instantiated BEFORE this point took the pass-1 body, which is what
            // `rebuild_stale_monomorphs` goes back for.
            if !self.first_pass {
                self.pass2_bodies.insert(self.context);
            }
            self.data.definitions[self.context as usize].code = v;
        }
        result
    }

    /// H7 — give a loop-carried return buffer a partner and rotate the two.
    ///
    /// A function returning a heap value takes a caller-allocated return buffer
    /// as a hidden trailing argument and CLEARS it on entry.  The caller mints
    /// one buffer per call SITE and binds the assignment target to that buffer
    /// (`a["__ref_1"] = n_add(a, x, __ref_1)`), so after one execution `a` *is*
    /// `__ref_1`.  Straight-line code is safe — every statement has its own
    /// buffer — but a loop runs the one site again: the callee clears the buffer
    /// that its own `v` argument now aliases, and every append but the last is
    /// lost, with no diagnostic.
    ///
    /// Mint a SECOND buffer for such a site and rotate the pair after each call:
    ///
    /// ```text
    ///   a = n_add(a, x, __ref_1);      // a now holds __ref_1's store
    ///   OpPutRef(__ref_1, __ref_2);    // the site's next call writes the OTHER store
    ///   OpPutRef(__ref_2, a);          // which parks the live one out of reach
    /// ```
    ///
    /// The two stores ping-pong for the life of the loop, so the buffer a call
    /// writes into is never the one the live target holds.  That costs one extra
    /// store per site and no copy at all — O(1) per iteration, where routing the
    /// result through a temp (the consumer workaround) is O(len) and so
    /// quadratic over the loop.  Both buffers are ordinary `__ref_N` work-refs,
    /// so each is freed once at scope exit and the target, a view, is not freed
    /// at all — the ownership plan is unchanged.
    fn rotate_loop_retbufs(&mut self, body: &mut Value) {
        let mut partners: Vec<(u16, u16)> = Vec::new();
        self.rotate_retbufs_in(body, false, &mut partners);
        if partners.is_empty() {
            return;
        }
        // Null-init each partner beside the buffer it rotates with, so the slot
        // allocator sees a first definition and the two stay adjacent in
        // `var_order` (scope exit frees in reverse declaration order).
        let Value::Block(bl) = body else { return };
        for (buf, partner) in partners {
            let init = v_set(partner, Value::Null);
            let at = bl
                .operators
                .iter()
                .position(
                    |op| matches!(op, Value::Set(s, val) if *s == buf && **val == Value::Null),
                )
                .map_or(0, |p| p + 1);
            bl.operators.insert(at, init);
        }
    }

    /// Walk one IR node for `rotate_loop_retbufs`, rewriting statement lists in
    /// place.  `in_loop` is true once the walk is inside a `Loop` body — the
    /// re-entry that makes a single per-site buffer unsafe.
    fn rotate_retbufs_in(
        &mut self,
        node: &mut Value,
        in_loop: bool,
        partners: &mut Vec<(u16, u16)>,
    ) {
        match node {
            Value::Loop(bl) => self.rotate_retbufs_list(&mut bl.operators, true, partners),
            Value::Block(bl) => self.rotate_retbufs_list(&mut bl.operators, in_loop, partners),
            Value::Insert(ops) | Value::Call(_, ops) | Value::CallRef(_, ops) => {
                for op in ops.iter_mut() {
                    self.rotate_retbufs_in(op, in_loop, partners);
                }
            }
            Value::If(c, t, e) => {
                self.rotate_retbufs_in(c, in_loop, partners);
                self.rotate_retbufs_in(t, in_loop, partners);
                self.rotate_retbufs_in(e, in_loop, partners);
            }
            Value::Span(b) => self.rotate_retbufs_in(&mut b.1, in_loop, partners),
            Value::Set(_, b) | Value::Return(b) | Value::Drop(b) => {
                self.rotate_retbufs_in(b, in_loop, partners);
            }
            _ => {}
        }
    }

    /// Statement-list half of `rotate_retbufs_in`: recurse into each statement,
    /// then splice the two rotate ops in after any statement that is a
    /// loop-carried self-feeding call.
    fn rotate_retbufs_list(
        &mut self,
        ops: &mut Vec<Value>,
        in_loop: bool,
        partners: &mut Vec<(u16, u16)>,
    ) {
        let mut i = 0;
        while i < ops.len() {
            self.rotate_retbufs_in(&mut ops[i], in_loop, partners);
            if in_loop && let Some((target, buf)) = self.self_feeding_call(&ops[i]) {
                let partner = if let Some((_, p)) = partners.iter().find(|(b, _)| *b == buf) {
                    *p
                } else {
                    let tp = self.vars.tp(buf).clone();
                    let p = self.vars.work_refs(&tp, &mut self.lexer);
                    self.vars.mark_caller_hidden_buf(p);
                    partners.push((buf, p));
                    p
                };
                let park = self.cl("OpPutRef", &[Value::Var(partner), Value::Var(target)]);
                let rotate = self.cl("OpPutRef", &[Value::Var(buf), Value::Var(partner)]);
                ops.insert(i + 1, rotate);
                ops.insert(i + 2, park);
                i += 2;
            }
            i += 1;
        }
    }

    /// Is `stmt` an assignment whose value is a user call that both writes into
    /// a hidden return buffer AND reads the variable being assigned?  Returns
    /// `(target, buffer)`.  Both halves are required: without the buffer there
    /// is nothing to alias, and without the self-read (`a = mk(k)`) the target
    /// aliasing the buffer is harmless.
    fn self_feeding_call(&self, stmt: &Value) -> Option<(u16, u16)> {
        let Value::Set(target, value) = stmt.unspan() else {
            return None;
        };
        let target = *target;
        // A hidden buffer or a parameter as the target is the callee-side NRVO
        // shape, which owns its storage differently — leave it alone.
        if self.vars.is_argument(target) || self.vars.is_caller_hidden_buf(target) {
            return None;
        }
        // Vector targets only.  A struct (`Reference`) target is already safe:
        // native's assignment-from-call frees the store the target held, so the
        // two handles cannot ping-pong — and it does not need to, because a
        // struct-returning callee copies its argument by value rather than
        // clearing the buffer it was handed (tests/scripts/303-ref-reassign-free).
        if !matches!(self.vars.tp(target), Type::Vector(_, _)) {
            return None;
        }
        let Value::Call(fn_nr, args) = value.unspan() else {
            return None;
        };
        // A loft-defined callee: only those take a caller-allocated buffer.
        let def = self.data.def(*fn_nr);
        if !def.is_loft_defined() {
            return None;
        }
        let buf = args.iter().rev().find_map(|a| match a.unspan() {
            Value::Var(w)
                if self.vars.is_caller_hidden_buf(*w)
                    && self.vars.name(*w).starts_with("__ref_") =>
            {
                Some(*w)
            }
            _ => None,
        })?;
        let feeds_itself = args.iter().any(|a| {
            !matches!(a.unspan(), Value::Var(w) if *w == buf) && ir_mentions_var(a, target)
        });
        if feeds_itself {
            Some((target, buf))
        } else {
            None
        }
    }

    // <expression> ::= <for> | 'continue' | 'break' | 'return' | 'yield' | '{' <block> | <operators>
    #[allow(clippy::too_many_lines)]
    /// @PLN86 step 0.1 — depth-guarded entry to expression parsing.  For trusted
    /// code (`!in_sandbox`) this is a single bool check then a tail call — zero
    /// cost.  Inside a sandboxed def it bounds the nesting depth so hostile
    /// `((((…))))` is a clean LOAD-time parse error, never a native stack
    /// overflow.  All recursion into nested sub-expressions routes back through
    /// here (parens, arithmetic, indexing → `parse_single` → `expression`), so
    /// one chokepoint bounds every nesting form.
    pub(crate) fn expression(&mut self, val: &mut Value) -> Type {
        if !self.in_sandbox {
            return self.expression_inner(val);
        }
        // Once the limit has tripped for this def, every further expression parse
        // is a no-op: the def is already rejected, so we stop recursing entirely
        // — this prevents a re-entry from re-walking the unconsumed deep tail and
        // guarantees the parser unwinds in O(remaining tokens).  Reset per-def in
        // `parse_function`.
        if self.depth_overflowed {
            return Type::Unknown(0);
        }
        self.parse_depth += 1;
        if self.parse_depth > SANDBOX_MAX_PARSE_DEPTH {
            // Stop recursing — emit once (latched) and unwind cleanly.
            self.depth_overflowed = true;
            diagnostic!(
                self.lexer,
                Level::Error,
                "expression nesting too deep in sandboxed code (limit {})",
                SANDBOX_MAX_PARSE_DEPTH
            );
            self.parse_depth -= 1;
            return Type::Unknown(0);
        }
        let result = self.expression_inner(val);
        self.parse_depth -= 1;
        result
    }

    fn expression_inner(&mut self, val: &mut Value) -> Type {
        // Start of the expression — an "Unknown variable" caret on a bare-Var
        // expression (e.g. a single call argument) must point here, not at the
        // cursor that has drifted to the closing `)` / `;` by detection time.
        let expr_pos = self.lexer.peek_pos().clone();
        if self.lexer.has_token("for") {
            self.parse_for(val);
            Type::Void
        } else if self.lexer.has_token("while") {
            self.parse_while(val);
            Type::Void
        // @F31 — break / continue (+ labelled forms)
        } else if self.lexer.has_token("continue") {
            if !self.in_loop {
                diagnostic!(self.lexer, Level::Error, "Cannot continue outside a loop");
            }
            *val = Value::Continue(0);
            Type::Never
        } else if self.lexer.has_token("break") {
            if !self.in_loop {
                diagnostic!(self.lexer, Level::Error, "Cannot break outside a loop");
            }
            // `break expr` — break with a value.  Desugars to `return expr`
            // since loft loops are currently void-typed.  Covers the common
            // find/search pattern where break-with-value exits the function.
            // TODO: implement for...else for the general case.
            if !self.lexer.peek_token("}")
                && !self.lexer.peek_token(";")
                && !matches!(self.lexer.peek().has, crate::lexer::LexItem::None)
            {
                let mut break_val = Value::Null;
                let break_tp = self.expression(&mut break_val);
                let ret_tp = self.data.def(self.context).returned().clone();
                if !self.first_pass && matches!(ret_tp, Type::Void) {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "`break <value>` requires a non-void function — \
                         the value is returned from the enclosing function"
                    );
                } else if !self.first_pass
                    && !matches!(ret_tp, Type::Void)
                    && !break_tp.is_same(&ret_tp)
                    && !break_tp.is_unknown()
                {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "`break` value type {} does not match function return type {}",
                        break_tp.name(&self.data),
                        ret_tp.name(&self.data)
                    );
                }
                *val = Value::Return(Box::new(break_val));
            } else {
                *val = Value::Break(0);
            }
            Type::Never
        } else if self.lexer.has_token("return") {
            self.parse_return(val);
            Type::Never
        } else if self.lexer.peek_token("debug_assert") {
            // `debug_assert` is a RESERVED name with no definition behind it yet: @PLN53 A2.3
            // adds `debug_assert(test, message)` as the `assert` companion that `--release`
            // elides, and reserving the word early is what keeps user code from taking it in
            // the meantime.  Every other keyword either has a parser arm or is defined in the
            // default library, so without this one a statement starting with `debug_assert`
            // fell through the whole chain and was reported as a missing `;` — at the
            // PREVIOUS statement's line, naming neither the word nor the reason (loft#1167).
            //
            // Consume the call so one clear refusal is the whole output, rather than the
            // first of a cascade.
            self.lexer.has_token("debug_assert");
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "`debug_assert` is reserved for a future release and does nothing yet — \
                     use `assert(…)`, which is checked in every build"
                );
            }
            if self.lexer.has_token("(") {
                while !self.lexer.peek_token(")") && !self.lexer.peek_token(";") {
                    let mut arg = Value::Null;
                    self.expression(&mut arg);
                    if !self.lexer.has_token(",") {
                        break;
                    }
                }
                self.lexer.has_token(")");
            }
            Type::Void
        } else if self.lexer.has_keyword("parallel") {
            self.parse_parallel(val);
            Type::Void
        } else if self.lexer.has_token("yield") {
            // CO1.3c: yield expr — only valid inside generator functions.
            // M11-a: also forbidden inside a par() body (worker runs in a
            // separate thread; there is no safe coroutine resumption path).
            if self.in_par_body && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "yield is not allowed inside a par(...) parallel body"
                );
                // Consume the expression tokens to keep the lexer in sync, but
                // emit Value::Null so no coroutine IR is generated — generating
                // yield state-machine code outside a coroutine context confuses
                // scope analysis (ref variables without matching OpFreeRef).
                if !self.lexer.has_keyword("from") {
                    let mut discarded = Value::Null;
                    self.expression(&mut discarded);
                }
                return Type::Void;
            }
            let r_type = self.data.def(self.context).returned().clone();
            if !matches!(r_type, Type::Iterator(_, _)) && !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "yield is only allowed inside generator functions (return type must be iterator<T>)"
                );
            }
            if self.lexer.has_keyword("from") {
                // CO1.4: yield from sub_gen — desugar to:
                //   __sub = sub;
                //   loop { __item = next(__sub); if exhausted(__sub) break; yield __item; }
                //
                // The break asks the SUB-GENERATOR whether it is exhausted.  It used to ask
                // whether the yielded VALUE is truthy (`if !__item break`), which is a
                // different question that only coincides with exhaustion for the types whose
                // falsy value happens to be their null sentinel.  Where it did not coincide,
                // the interpreter got it wrong twice over and `--native` — which compares
                // against a per-channel exhaust sentinel — stayed right:
                //
                //   iterator<boolean>  a delegation TRUNCATED at the first `false`
                //                      (`true,false,true` delivered `true`), silently
                //   iterator<float>    / `single`: NaN is not falsy, so the break never
                //                      fired and the loop ran forever
                //
                // `OpCoroutineExhausted` is the question actually being asked, and it is the
                // same pair the streaming for-loop uses (`parser/control.rs`), so the two
                // consumers of a generator cannot disagree about when one has ended.
                let mut sub = Value::Null;
                let sub_type = self.expression(&mut sub);
                if let Type::Iterator(inner, _) = &sub_type {
                    let elem_tp = (**inner).clone();
                    let sub_var = self.create_unique("__yf_sub", &sub_type);
                    self.vars.defined(sub_var);
                    // A yielded handle points INTO the sub-generator's frame store, so the
                    // item binds as a BORROW of `__yf_sub` — the same dep the streaming
                    // `for x in g()` gives its loop var (loft#481) — and no scope exit
                    // frees it: an enclosing `if` arm's did, and released the frame.
                    let item_tp = if crate::data::holds_dbref(&elem_tp) {
                        elem_tp.with_deps(&crate::data::Deps::frame1(sub_var))
                    } else {
                        elem_tp.clone()
                    };
                    let item_var = self.create_unique("__yf_item", &item_tp);
                    self.vars.defined(item_var);
                    let op = self.data.def_nr("OpCoroutineNext");
                    let value_size =
                        crate::variables::size(&elem_tp, &crate::data::Context::Argument);
                    let next_call = Value::Call(
                        op,
                        vec![Value::Var(sub_var), Value::Int(i32::from(value_size))],
                    );
                    let test = self.cl("OpCoroutineExhausted", &[Value::Var(sub_var)]);
                    let lp = vec![
                        crate::data::v_set(item_var, next_call),
                        crate::data::v_if(
                            test,
                            crate::data::v_block(vec![Value::Break(0)], Type::Void, "break"),
                            Value::Null,
                        ),
                        Value::Yield(Box::new(Value::Var(item_var))),
                    ];
                    let steps = vec![
                        crate::data::v_set(sub_var, sub),
                        crate::data::v_loop(lp, "yield from"),
                    ];
                    *val = crate::data::v_block(steps, Type::Void, "yield from block");
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "yield from requires an iterator expression"
                    );
                }
                Type::Void
            } else {
                let mut v = Value::Null;
                // loft#1130 — a yielded value LEAVES the function against a type the
                // declaration already names, exactly as a `return` does, so it takes the
                // same `⇐` seeding.  Without it a keyed literal (`yield [K { … }]`) had
                // nothing to resolve against and was built as a `vector<K>`: the consumer
                // got a collection with the wrong length and no keys, on both backends and
                // with no diagnostic, while the identical literal BOUND to a declared local
                // first was correct — that position reads its destination from `var_tp` and
                // never needed the channel.
                let saved_expected = std::mem::replace(&mut self.expected, Type::Unknown(0));
                if let Type::Iterator(elem_tp, _) = &r_type {
                    let elem = (**elem_tp).clone();
                    self.seed_leaving_value_hint(&elem);
                }
                self.expression(&mut v);
                self.expected = saved_expected;
                // @P328 — when yielding a NON-CAPTURING closure into an
                // `iterator<fn(...) -> ...>` generator, the expression
                // parser leaves the lambda as a bare `Value::Int(d_nr)`
                // (the closure DbRef would be the null sentinel anyway).
                // But the generator's yielded type is `Function(...)`
                // which the coroutine machinery treats as 20 bytes
                // (8B d_nr + 12B closure DbRef).  Pushing only 4 bytes
                // for the bare Int crashes the consumer's
                // `OpCoroutineNext(gen, 20)` (interp SIGBUS) and
                // mis-types the native channel.  Wrap the bare Int as
                // a proper `Value::FnRef(d_nr, u16::MAX, _)` so the
                // yielded value is a full 20-byte fn-ref.  Capturing
                // closures already arrive as a Block ending in
                // `FnRef(d_nr, closure_var, _)`, so no rewrite needed.
                if let Type::Iterator(elem_tp, _) = &r_type
                    && matches!(**elem_tp, Type::Function(_, _, _))
                {
                    let unspanned = v.unspan().clone();
                    if let Value::Int(d_nr) = unspanned {
                        v = Value::FnRef(d_nr, u16::MAX, Box::new((**elem_tp).clone()));
                    } else if let Value::Long(d_nr) = unspanned {
                        v = Value::FnRef(d_nr as i32, u16::MAX, Box::new((**elem_tp).clone()));
                    }
                }
                // A tuple YIELD materialises into the coroutine's buffer, and that buffer
                // copies its members — so a member the tuple literal already wrapped in a
                // frame-local copy (`tuple_member_copy`) is copied TWICE and the wrapper's
                // store is claimed by nobody.  loft#1109 records the same fact for the RETURN
                // path and unwraps it at `synthetic_tuple_return`; a yield does not go through
                // that rewrite, so it unwraps here.  Without it a generator yielding a keyed
                // collection leaks one store per kind.
                if !self.first_pass
                    && let Value::Tuple(members) = v.unspan_mut()
                {
                    for m in members.iter_mut() {
                        if let Some(src) = self.tuple_member_copy_source(m) {
                            *m = src;
                        }
                    }
                }
                *val = Value::Yield(Box::new(v));
                Type::Void
            }
        } else if self.lexer.peek_token("{") {
            self.parse_block("block", val, &Type::Void)
        } else {
            // `const x = expr` — mark the resulting local variable as const after initialisation.
            let const_decl = self.lexer.has_keyword("const");
            let res = self.parse_assign(val);
            if const_decl && !self.first_pass {
                let v_nr = match val {
                    Value::Set(nr, _) => Some(*nr),
                    Value::Insert(ls) => ls.iter().find_map(|v| {
                        if let Value::Set(nr, _) = v {
                            Some(*nr)
                        } else {
                            None
                        }
                    }),
                    _ => None,
                };
                if let Some(v_nr) = v_nr {
                    self.vars.set_const_binding(v_nr);
                } else if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "const keyword requires a variable assignment"
                    );
                }
            }
            // loft#960 — not for a name in a `( … ) =` LHS.  Those names are BEING
            // BOUND, and this check asks whether a name being READ resolves: it
            // reported `(a, b, c) = later(…)` as three unknown variables whenever
            // `later` was declared below the caller, because the binding's type is
            // only settled once the destructuring below has the callee's return
            // type — which pass 1 does not have for a forward reference.  The plain
            // `a = later(…)` form never came through here (its own LHS is the
            // assignment target), so the two spellings disagreed about a name that
            // is legal in both, and the message named the LEFT-hand names while the
            // cure was to move the callee.
            //
            // `at_binding_name` is the one home for "is this a binding occurrence?",
            // and its tuple arm is true exactly while the LHS list is being parsed
            // (`in_tuple_lhs`, cursor on the `,` or `)`).  A destructuring the
            // parser cannot lower is still refused below — "Cannot destructure a
            // non-tuple value" — so nothing is silenced, only re-homed.
            if !self.at_binding_name() {
                self.known_var_or_type(val, &expr_pos);
            }
            res
        }
    }

    /// L10: `while <cond> { <body> }` desugars to an infinite loop with a break guard.
    ///
    /// The emitted IR is equivalent to:
    ///   loop { if !cond { break }; body }
    pub(crate) fn parse_while(&mut self, code: &mut Value) {
        // @PLN86 3.1 — the `while`'s position, taken before the condition so an
        // unbounded-loop diagnostic points at the `while` itself.
        let while_pos = self.lexer.peek_pos().clone();
        let mut cond = Value::Null;
        // loft#986 — see `in_control_head`: the `{` after the condition opens the body.
        let outer_head = self.in_control_head;
        self.in_control_head = true;
        let cond_tp = self.expression(&mut cond);
        self.in_control_head = outer_head;
        // The same coercion `if` performs — a `while` over a collection handle is the
        // identical position, and reading a pointer's first byte as the flag is how the
        // loop ran the wrong number of times.
        self.convert_condition(&mut cond, &cond_tp);
        if !self.first_pass && matches!(cond, Value::Null) {
            diagnostic!(self.lexer, Level::Error, "Expected condition after 'while'");
            return;
        }
        // @PLN86 3.1 — keep the raw condition (pass 2 only) to check for a
        // decreasing variant once the body is parsed; the bound check needs both.
        let sandbox_cond = if self.in_sandbox && !self.first_pass {
            cond.clone()
        } else {
            Value::Null
        };
        let not_cond = self.cl("OpNot", &[cond]);
        let break_if = v_if(
            not_cond,
            v_block(vec![Value::Break(0)], Type::Void, "break"),
            Value::Null,
        );
        let loop_nr = self.vars.start_loop();
        let in_loop = self.in_loop;
        self.in_loop = true;
        let mut body = Value::Null;
        let loop_write_state = self.vars.save_and_clear_write_state();
        self.parse_block("while", &mut body, &Type::Void);
        self.vars.restore_write_state(&loop_write_state);
        self.in_loop = in_loop;
        self.vars.finish_loop(loop_nr);
        // @PLN86 3.1 — on pass 2 (complete IR), a sandboxed `while` is admitted only
        // if it carries a compiler-checked decreasing variant; otherwise it is an
        // unbounded loop, recorded for the totality admission.  The parser uniquely
        // knows this is a `while` (the IR can't tell it from a bounded comprehension
        // `Loop`).  Keyed by def; the bound result is stable so re-entry is safe.
        if self.in_sandbox
            && !self.first_pass
            && !crate::sandbox::while_is_bounded(&self.data, &sandbox_cond, &body)
        {
            self.sandbox_unbounded_loops
                .entry(self.context)
                .or_insert(while_pos);
        }
        *code = v_loop(vec![break_if, body], "while");
    }

    pub(crate) fn change_var(&mut self, code: &Value, tp: &Type) -> bool {
        if let Value::Var(v_nr) = code {
            let mut is_text = matches!(self.vars.tp(*v_nr), Type::Text(_));
            if let Type::RefVar(i) = self.vars.tp(*v_nr)
                && matches!(**i, Type::Text(_))
            {
                is_text = true;
            }
            // #328: `x = x.next` would give x a SELF-dep ("x borrows from
            // x") — a degenerate borrow that flips the var into the
            // dependent-view codegen class (InitCreateStack) and corrupts
            // the frame.  Strip the self-entry; the remaining deps (if
            // any) still carry the real borrow sources.  The pre-Set free
            // stays safe: codegen's S1 guard skips it whenever the RHS
            // reads `v` itself.
            //
            // Both RECORD kinds, and only those.  A struct-enum is a record reached
            // through a `DbRef` exactly as a struct is (`data::is_dbref` names the two
            // together), and `Type::Enum` was outside the pattern: `e: Sh = Circle{r: 1}`
            // gave `e` the dep `[e]`, `owns_displaced_store` read the non-empty list as
            // BORROWED (`@FR-O-Proxy`), and a later join reassignment never freed the store
            // it displaced — one leak per execution on both backends (loft#1389).  The
            // COLLECTION kinds are deliberately not here: for a `vector` or a `text` the
            // self-dep is the @P302 re-init-in-place ownership marker that
            // `formal/ownership.md` (g) reads as `Owned` (`t = "{t}x"`), not a degenerate
            // borrow — 6418 of them in the corpus against no `Enum` at all.  `retain`
            // rather than `Deps::frame`, so a list keeps the dep SPACE it arrived in.
            let stripped: Type;
            let tp = if let Some(deps) = tp.borrow_deps()
                && matches!(tp, Type::Reference(_, _) | Type::Enum(_, true, _))
                && deps.contains(v_nr)
            {
                let mut kept = deps.clone();
                kept.retain(|d2| d2 != v_nr);
                stripped = tp.with_deps(&kept);
                &stripped
            } else {
                tp
            };
            if !is_text || *tp != Type::Character {
                self.change_var_type(*v_nr, tp);
            }
            true
        } else {
            false
        }
    }

    pub(crate) fn change_var_type(&mut self, v_nr: u16, tp: &Type) {
        // Plan-22 phase 02d-iii.c — preserve the boxed scalar's
        // `Reference(__cell_<T>, _)` type when an assignment's
        // RHS would otherwise revert it.  `parse_assign_op`
        // calls `change_var(to, &s_type)` after parsing the
        // RHS; without this guard, every `n = expr` line in
        // the body would undo the type flip the moment the
        // body walk begins.  Fires only when the current type
        // is a `__cell_*` Reference AND the new type is the
        // cell's value type (the safe "scalar-into-boxed-scalar"
        // overwrite), leaving every other re-typing untouched.
        //
        // Dormant in production today (02d-iii.a's flip is
        // dormant — no variable carries `Reference(__cell_*, _)`).
        // Activates in 02d-iii.e together with the flip.
        if !self.first_pass
            && self.vars.exists(v_nr)
            && let Some(d) = crate::parser::vectors::boxed_cell_def(self.vars.tp(v_nr), &self.data)
            && let Some(value_attr) = self.data.def(d).attributes().first()
            && value_attr.name == "value"
            // Compared through `.base()`: assigning a dense `Ï` into a `Ï?` cell is the
            // scalar-into-boxed-scalar overwrite this guard exists for, and without the peel
            // the flip is reverted on the first `x = â¦` in the body (loft#1408).
            && (value_attr.typedef.base().is_equal(tp.base())
                || (matches!(value_attr.typedef.base(), Type::Integer(_))
                    && matches!(tp.base(), Type::Integer(_))))
        {
            return;
        }
        let chg = self
            .vars
            .change_var_type(v_nr, tp, &self.data, &mut self.lexer);
        if chg
            && !tp.is_unknown()
            && let Type::Vector(elm, _) = tp
        {
            self.data.vector_def(&mut self.lexer, elm);
        }
    }

    /// Check for iteration-safety violation on `+=` to collections; emit diagnostics.
    pub(crate) fn check_iter_safety(&mut self, to: &Value, f_type: &Type, op: &str) {
        if self.first_pass
            || op != "+="
            || !matches!(
                f_type,
                Type::Vector(_, _)
                    | Type::Sorted(_, _, _)
                    | Type::Index(_, _, _)
                    | Type::Radix(_, _, _)
                    | Type::Trie(_, _, _)
            )
        {
            return;
        }
        if let Value::Var(lhs_nr) = to
            && self.vars.is_iterated_var(*lhs_nr)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot add elements to '{}' while it is being iterated — \
use a separate collection or add after the loop",
                self.vars.name(*lhs_nr)
            );
        } else if !matches!(to, Value::Var(_)) && self.vars.is_iterated_value(to) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot add elements to a collection while it is being iterated — \
use a separate collection or add after the loop"
            );
        }
    }

    /// Validate `d#lock = expr` assignment; returns true if handled (caller should return Void).
    pub(crate) fn validate_lock_assign(&mut self, code: &Value, to: &Value) -> bool {
        if self.first_pass {
            return false;
        }
        let Value::Call(lock_nr, lock_args) = to.unspan() else {
            return false;
        };
        if self.data.def(*lock_nr).name() != "n_get_store_lock" {
            return false;
        }
        if !matches!(code, Value::Boolean(_)) {
            diagnostic!(
                self.lexer,
                Level::Error,
                "d#lock can only be assigned a constant boolean (true or false)"
            );
            return true;
        }
        if matches!(code, Value::Boolean(false))
            && let Some(Value::Var(v_nr)) = lock_args.first()
            && self.vars.is_const_any(*v_nr)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot unlock const variable '{}' via d#lock = false",
                self.vars.name(*v_nr)
            );
            return true;
        }
        false
    }

    /// Decides @FR-B-Copy vs @FR-B-View / @FR-B-View-Base for a whole-value vector bind:
    /// off an OWNED base a collection projection copies, off a BORROWED one it views.
    ///
    /// The pure selector for the C86 whole-value vector bind at `parse_assign_op`'s copy
    /// branch — the rule rationale lives on the `VecBind` variants, and the branch applies
    /// mechanics only.  Runs on BOTH passes: `change_var` re-types per pass, and emission
    /// is pass-2, so a pass-1 answer that differs is a re-type, not a disagreement.
    #[allow(clippy::too_many_arguments, clippy::too_many_lines)]
    fn classify_vec_bind(
        &self,
        code: &Value,
        op: &str,
        var_nr: u16,
        f_type: &Type,
        s_type: &Type,
    ) -> VecBind {
        // Read both sides through `base()`.  `vector<τ>?` is `Optional(Vector(τ))` — the
        // same storage behind a nullability marker (`@FR-L-Null`) — and matching the bare
        // spelling alone answered `NotABind` for it, so the whole-value copy this selector
        // exists to pick never ran and the bind stayed an ALIAS: `b = a; a[1] = 99` then
        // read 99 through `b`, against `@FR-B-Copy` (loft#1319).  Nullability is not one of
        // that rule's three exceptions — a struct PROJECTION, a BORROWED base, an INDEX or
        // nested read — and this selector already distinguishes all three.  The keyed kinds
        // took the same peel in loft#1143 and have been copying correctly since; this is
        // their sibling, and `@FR-L-Null` is the reason both are the same question.
        if op != "="
            || var_nr == u16::MAX
            || !matches!(f_type.base(), Type::Unknown(_) | Type::Vector(_, _))
            || !matches!(s_type.base(), Type::Vector(_, _))
        {
            return VecBind::NotABind;
        }
        // The bare-Var test is deliberately NOT unspanned (a Span-wrapped RHS
        // lowers elsewhere); the field-read and self-assign tests are.
        let is_bare_var = matches!(code, Value::Var(_));
        // @FR-O-Proxy asks copy — the verdict is a `VecBind` (`CopyVar` / `CopyOwnedField` /
        // `SelfAssign`), and every arm of it copies or does nothing.  A wrong answer picks the
        // wrong lowering, not a release.
        let owned_field_read = if let Value::Call(d, args) = code.unspan()
            && *d == self.data.def_nr("OpGetField")
            && let Some(Value::Var(bv)) = args.first().map(Value::unspan)
            && matches!(self.vars.tp(*bv), Type::Reference(_, _))
        {
            self.vars.tp(*bv).depend().is_empty()
        } else {
            false
        };
        // A vector member read off an OWNED tuple local (`a = t.0`) is the same one-level
        // projection — `@FR-L-Tuple` makes the tuple a struct — and copies for the same
        // reason (loft#1361); off a parameter or a linked local it stays the view
        // `@FR-B-View-Base` says.
        // The base's own member deps name its frame-local backings (a copied member depends
        // on the store it was copied into), never a borrow of another variable — a `&` link
        // is a `RefVar`, not a `Tuple` — so ownership here is "a local that is not a parameter".
        let owned_tuple_member = matches!(code.unspan(), Value::TupleGet(bv, _)
            if matches!(self.vars.tp(*bv), Type::Tuple(_)) && !self.vars.is_argument(*bv));
        if is_bare_var {
            if matches!(code.unspan(), Value::Var(rhs) if *rhs == var_nr) {
                return VecBind::SelfAssign;
            }
            return VecBind::CopyVar;
        }
        if owned_field_read || owned_tuple_member {
            return VecBind::CopyOwnedField;
        }
        VecBind::NotABind
    }

    /// Does the copy refill a FRESH store of the local's own, rather than the one the local
    /// holds?  `@FR-O-Proxy` answers by ownership (`vector_needs_db`: a local with no deps owns
    /// the store it holds, so that store is cleared and refilled), and two places cannot ask
    /// the proxy:
    ///
    ///   * an ARM of a value branch, where the local carries the branch's JOIN deps and so
    ///     reads as a borrower of a store it may not hold at all — a nullable local's
    ///     sentinel, or the other arm's source — so refilling it in place cleared through a
    ///     null or into a variable the other arm names;
    ///   * a PARAMETER's first rebind (calls.md `@FR-F-ParamRebind`): the store it holds is
    ///     the CALLER's, and a whole-value reassignment rebinds locally rather than writing
    ///     back — the carve-out that keeps a parameter from allocating a store exists to
    ///     avoid an allocation, not to let a refill reach the caller.  Once rebound the
    ///     local names its own `__vdb_N` and the proxy answers again.  A promoted return
    ///     buffer is the one argument that IS filled in place (F-Ret hands the value up
    ///     through it), and keeps the carve-out.
    fn vec_copy_needs_db(&self, var_nr: u16, elm_tp: &Type, in_arm: bool) -> bool {
        in_arm
            || self.vector_needs_db(var_nr, elm_tp, true)
            // @FR-O-Proxy asks alloc — a parameter with no deps still holds the caller's
            // store; the empty list here decides that its first rebind ALLOCATES, never
            // that anything is released.
            || (self.vars.is_argument(var_nr)
                && !self.is_hidden_param(var_nr)
                && !matches!(self.vars.tp(var_nr), Type::RefVar(_))
                && self.vars.tp(var_nr).depend().is_empty())
    }

    /// The lowering a whole-value vector bind's verdict asks for, applied in place to
    /// `code`, the source, for the local `to` (`var_nr`): nothing for the identity, else the
    /// local's own store — allocated when it owns none, cleared when it does — refilled with
    /// a deep copy of the source's elements.  Fires on BOTH passes (`change_var` types the
    /// local each pass and the field-read strip advances the `_elm_N` counter on the first,
    /// so the `__vdb_N` dep is created identically); the ops are emitted on the second.
    #[allow(clippy::too_many_arguments)] // the copy arm's own inputs, plus where it sits
    fn lower_vec_copy_bind(
        &mut self,
        code: &mut Value,
        vec_bind: &VecBind,
        to: &Value,
        var_nr: u16,
        s_type: &Type,
        lhs_parent_tp: &Type,
        in_arm: bool,
    ) {
        let Type::Vector(elm_tp, _) = s_type.base() else {
            return;
        };

        // `v = v` self-assign — emit nothing rather than clear+reappend
        // off the same storage.
        if matches!(vec_bind, VecBind::SelfAssign) {
            *code = Value::Insert(Vec::new());
            return;
        }
        let elm_tp_clone = (**elm_tp).clone();
        let dense = Type::Vector(Box::new(elm_tp_clone.clone()), Deps::none());
        // The COPY is what the selector decided; whether the value may be ABSENT is the
        // source's own fact and survives it.  Dropping the marker here would make the
        // destination non-null and lose the null arm.
        let vec_tp = if matches!(s_type, Type::Optional(_)) {
            Type::Optional(Box::new(dense))
        } else {
            dense
        };
        self.change_var(to, &vec_tp);
        let field_read = matches!(vec_bind, VecBind::CopyOwnedField);
        // #426 — a struct vector-FIELD read (`af = bx.v`, `field_read`) must
        // strip `af`'s inherited base dep on BOTH passes, then consume one
        // `elm`-name slot on both.  `Function::unique`'s per-prefix counter has
        // to advance IDENTICALLY across passes: a family created on only one
        // pass shifts every other family's numbering, so a later `_elm_N`
        // re-resolves to a pass-1 var backed by a different store — silent
        // corruption (a nested vector literal `[[…]]` after `af = bx.v` built
        // its inner element into an orphaned store, reading back len 0).  The
        // field-read used to take the whole pass-2-only branch below, so on
        // pass 1 its dep stayed non-empty (`vector_needs_db` false → no `elm`)
        // while pass 2 stripped it (`vector_needs_db` true → one `elm`): a
        // one-slot drift.  Stripping + advancing the counter on pass 1 too
        // removes the drift.  The var-copy case (`b = a`) is untouched here —
        // its strip stays pass-2-only (line below), since it never had the
        // pass-1 dep-mismatch (the RHS var already owns an independent store).
        if field_read && self.first_pass {
            let inherited = self.vars.tp(var_nr).depend();
            for d in inherited {
                self.vars.make_independent(var_nr, d);
            }
            if self.vec_copy_needs_db(var_nr, &elm_tp_clone, in_arm) {
                self.unique_elm_var(lhs_parent_tp, &elm_tp_clone, var_nr);
            }
            return;
        }
        if !self.first_pass {
            // Break the alias.  The standard type-inference copied the RHS
            // var's store dep onto v (making v *borrow* rhs's storage — the
            // dangling-on-scope-exit @P292 hazard, and the no-own-store @P394
            // crash on first assignment).  Strip rhs's deps from v so v gets
            // its OWN store.  This is a no-op when v already owns an
            // independent store (`v = [9]; v = a` reassignment), so that case
            // still takes the clear+refill path below.  (Mirrors the @P295
            // Var-to-var deep-copy dep-strip.)
            if let Value::Var(rhs_var) = code.unspan() {
                self.vars.make_independent(var_nr, *rhs_var);
                for d in self.vars.tp(*rhs_var).depend() {
                    self.vars.make_independent(var_nr, d);
                }
            } else {
                // #415 — field-read RHS (`af = bx.v`): there is no rhs_var,
                // but `af` inherited the base's dep ({bx}) during type
                // resolution.  Strip af's own inherited deps so
                // `vector_needs_db` below sees an empty dep and allocates af
                // its OWN store; otherwise it takes the reassignment/clear arm,
                // af never owns a store, and the alias to bx's field persists.
                // The OpAppendVector then deep-copies the field's elements in.
                let inherited = self.vars.tp(var_nr).depend();
                for d in inherited {
                    self.vars.make_independent(var_nr, d);
                }
            }
            // @P314 — narrow-aware element type (see `append_elem_tp`).
            let rec_tp = Value::Int(self.append_elem_tp(&elm_tp_clone));
            let mut stmts = Vec::new();
            if self.vec_copy_needs_db(var_nr, &elm_tp_clone, in_arm) {
                // First assignment: v owns no store yet — allocate one.
                let elm_var = self.unique_elm_var(lhs_parent_tp, &elm_tp_clone, var_nr);
                let db = self.insert_new(var_nr, elm_var, &elm_tp_clone, &mut stmts);
                self.vars.depend(var_nr, db);
            } else {
                // Reassignment: v already owns a store — clear, then refill.
                stmts.push(self.cl("OpClearVector", std::slice::from_ref(to)));
            }
            // A NULLABLE destination takes the whole-value REPLACE rather than the
            // append, for the one thing the append cannot express: an ABSENT source
            // must leave the destination absent, not holding the empty store the alloc
            // above just gave it.  `Stores::vector_replace` carries that rule, the same
            // one `replace_keyed` carries for the keyed kinds — which is why the keyed
            // sibling of this bind has been answering `b == null` correctly all along.
            // A dense destination cannot be absent, so it keeps the append and its IR is
            // unchanged.
            let op = if matches!(self.vars.tp(var_nr), Type::Optional(_)) {
                "OpReplaceVector"
            } else {
                "OpAppendVector"
            };
            stmts.push(self.cl(op, &[to.clone(), code.clone(), rec_tp]));
            *code = Value::Insert(stmts);
        }
    }

    /// `@FR-O-Complete` — a vector local bound from a value branch: write the bind out per
    /// arm, so every path binds the local the way a single bind of that arm's tail would.
    /// `true` when the rewrite happened; `false` where the branch has no arm a plain bind
    /// would copy (every arm keeps the value it has, and the value form is that already) or
    /// an arm cannot be written out as a bind at all.
    fn sink_vec_bind_into_arms(
        &mut self,
        code: &mut Value,
        to: &Value,
        var_nr: u16,
        f_type: &Type,
        s_type: &Type,
        lhs_parent_tp: &Type,
    ) -> bool {
        let mut hoists = Vec::new();
        if !matches!(
            code.unspan(),
            Value::If(_, _, _) | Value::Block(_) | Value::Insert(_)
        ) || self.vec_arm_copies(code, var_nr, f_type, s_type, &mut hoists) != Some(true)
        {
            return false;
        }
        let mut hoists = Vec::new();
        self.sink_vec_arms(code, to, var_nr, f_type, s_type, lhs_parent_tp, &mut hoists);
        self.branch_sunk_vectors
            .insert((self.context, self.vars.name(var_nr).to_string()));
        // A wrapper BLOCK opens a scope — a `??` hoist's, a `match` subject's — and a local
        // whose first `Set` now sits inside it would be homed there, out of reach of the
        // statement's own scope where its next use is.  The bind is made at the statement:
        // a null `Set` ahead of the block declares the local here, which is the pre-init
        // the post-parse scan writes for a local first assigned in sibling arms, and which
        // that scan elides on a local already in scope (a reassignment).  A parameter is
        // never first-bound here and holds the caller's value on a self arm.
        if matches!(code.unspan(), Value::Block(_)) && !self.vars.is_argument(var_nr) {
            let sunk = std::mem::replace(code, Value::Null);
            *code = Value::Insert(vec![Value::Set(var_nr, Box::new(Value::Null)), sunk]);
        }
        true
    }

    /// The compiler temps the operators AHEAD of a branch bind, with what each was bound
    /// from — a `??` hoist (`__ncc_N = s.v`) is the one this serves.  Pushed onto `hoists`,
    /// which the arm verdict reads.
    fn note_hoists(&self, ops: &[Value], hoists: &mut Vec<(u16, Value)>) {
        for op in ops {
            if let Value::Set(t, src) = op.unspan()
                && self.vars.is_compiler_generated(*t)
                // A collection's own buffer is bound from the store it minted
                // (`_vec_N = OpGetField(__vdb_N, 0, …)`, a comprehension's result the same):
                // a read rooted at a compiler temp is nothing the author wrote, and nothing
                // the selector copies.
                && !self
                    .field_place(src)
                    .is_some_and(|(root, _)| self.vars.is_compiler_generated(root))
            {
                hoists.push((*t, (**src).clone()));
            }
        }
    }

    /// Does any arm of this value branch end in a tail a plain bind would COPY?  `Some(true)`
    /// if one does, `Some(false)` where every arm keeps the value it has, `None` where an arm
    /// cannot be written out as a bind: a bare `null` (which the post-parse scan elides on a
    /// local already in scope), a block that yields nothing (a diverging arm), an empty one.
    /// A value branch is seen through its wrappers — a `Span`, a value block (a plain arm, or a
    /// `match` behind its subject binding), an `Insert` — each yielding its LAST operator.
    fn vec_arm_copies(
        &self,
        node: &Value,
        var_nr: u16,
        f_type: &Type,
        s_type: &Type,
        hoists: &mut Vec<(u16, Value)>,
    ) -> Option<bool> {
        match node.unspan() {
            Value::If(_, t, f) => {
                let then_copies = self.vec_arm_copies(t, var_nr, f_type, s_type, hoists)?;
                let else_copies = self.vec_arm_copies(f, var_nr, f_type, s_type, hoists)?;
                Some(then_copies || else_copies)
            }
            Value::Block(bl) => {
                if matches!(bl.result, Type::Void | Type::Null) {
                    return None;
                }
                let (last, ahead) = bl.operators.split_last()?;
                self.note_hoists(ahead, hoists);
                self.vec_arm_copies(last, var_nr, f_type, s_type, hoists)
            }
            Value::Insert(ops) => {
                let (last, ahead) = ops.split_last()?;
                self.note_hoists(ahead, hoists);
                self.vec_arm_copies(last, var_nr, f_type, s_type, hoists)
            }
            Value::Null => None,
            tail => Some(matches!(
                self.vec_arm_verdict(tail, var_nr, f_type, s_type, hoists),
                VecBind::CopyVar | VecBind::CopyOwnedField
            )),
        }
    }

    /// The selector's verdict for one arm tail.  A compiler temp is judged by what it was
    /// bound FROM: a `??` hoist of a projection (`__ncc_N = s.v`) stands for that read, so
    /// the arm copies from the temp exactly where `x = s.v` copies (`@FR-B-Copy`: off an
    /// owned base a collection projection copies) and views where it views; a temp bound
    /// from nothing the selector copies — a literal's `_vec_N`, the arm's own buffer — keeps
    /// the value it has.
    fn vec_arm_verdict(
        &self,
        tail: &Value,
        var_nr: u16,
        f_type: &Type,
        s_type: &Type,
        hoists: &[(u16, Value)],
    ) -> VecBind {
        let tail = tail.unspan();
        if let Value::Var(x) = tail
            && self.vars.is_compiler_generated(*x)
        {
            let copies_source = hoists.iter().any(|(t, src)| {
                *t == *x
                    && matches!(
                        self.classify_vec_bind(src.unspan(), "=", var_nr, f_type, s_type),
                        VecBind::CopyVar | VecBind::CopyOwnedField
                    )
            });
            return if copies_source {
                VecBind::CopyVar
            } else {
                VecBind::NotABind
            };
        }
        self.classify_vec_bind(tail, "=", var_nr, f_type, s_type)
    }

    /// The statement form of a vector local's value-branch bind: every arm tail becomes the
    /// bind of that tail — the copy lowering where the selector says copy, a plain `Set`
    /// otherwise — and the arms stop yielding a value.
    #[allow(clippy::too_many_arguments)] // the bind's inputs, threaded to every arm
    fn sink_vec_arms(
        &mut self,
        node: &mut Value,
        to: &Value,
        var_nr: u16,
        f_type: &Type,
        s_type: &Type,
        lhs_parent_tp: &Type,
        hoists: &mut Vec<(u16, Value)>,
    ) {
        match node {
            Value::Span(b) => {
                self.sink_vec_arms(&mut b.1, to, var_nr, f_type, s_type, lhs_parent_tp, hoists);
            }
            Value::If(_, t, f) => {
                self.sink_vec_arms(t, to, var_nr, f_type, s_type, lhs_parent_tp, hoists);
                self.sink_vec_arms(f, to, var_nr, f_type, s_type, lhs_parent_tp, hoists);
            }
            // A block that yields its OWN buffer — a literal's `_vec_N`, a comprehension's
            // result — is the arm's value as it stands: bound whole, so the buffer stays
            // homed with the binding (a void block would take it, and free it at the
            // block's end while the local still names it).
            Value::Block(bl)
                if bl.operators.last().is_some_and(|last| {
                    matches!(last.unspan(), Value::Var(x) if self.vars.is_compiler_generated(*x))
                        && matches!(
                            self.vec_arm_verdict(last, var_nr, f_type, s_type, hoists),
                            VecBind::NotABind
                        )
                }) =>
            {
                let t = std::mem::replace(node, Value::Null);
                *node = Value::Set(var_nr, Box::new(t));
            }
            Value::Block(bl) => {
                if let Some((last, ahead)) = bl.operators.split_last_mut() {
                    self.note_hoists(ahead, hoists);
                    self.sink_vec_arms(last, to, var_nr, f_type, s_type, lhs_parent_tp, hoists);
                }
                bl.result = Type::Void;
            }
            Value::Insert(ops) => {
                if let Some((last, ahead)) = ops.split_last_mut() {
                    self.note_hoists(ahead, hoists);
                    self.sink_vec_arms(last, to, var_nr, f_type, s_type, lhs_parent_tp, hoists);
                }
            }
            tail => {
                let verdict = self.vec_arm_verdict(tail, var_nr, f_type, s_type, hoists);
                if matches!(verdict, VecBind::NotABind) {
                    let t = std::mem::replace(tail, Value::Null);
                    *tail = Value::Set(var_nr, Box::new(t));
                } else {
                    self.lower_vec_copy_bind(
                        tail,
                        &verdict,
                        to,
                        var_nr,
                        s_type,
                        lhs_parent_tp,
                        true,
                    );
                }
            }
        }
    }

    /// Is the next thing in the source the struct literal `<name> { … }`, without
    /// consuming it?  Used to decide, BEFORE the right-hand side is parsed, that it is a
    /// fresh construction and can therefore be built directly into its destination
    /// record rather than into a throwaway one (@PLN135 arc A).
    ///
    /// The name is compared verbatim, so a qualified or generic spelling of the same
    /// type answers `false` — the caller then keeps its ordinary by-copy path.
    fn peek_literal_of(&mut self, name: &str) -> bool {
        if !matches!(&self.lexer.peek().has, crate::lexer::LexItem::Identifier(n) if n == name) {
            return false;
        }
        let saved = self.lexer.link();
        self.lexer.cont();
        let braced = self.lexer.peek_token("{");
        self.lexer.revert(saved);
        braced
    }

    /// Does this store put a value of the wrong type into a struct FIELD (loft#893)?
    ///
    /// A field store is the one assignment form with no variable to re-type, so the
    /// `change_var_type` rejection that refuses `v = make()` for a local never sees it.
    /// The checks that DO cover fields sit further down `parse_assign_op`, behind an
    /// early return that a `text` or collection target takes first — so `h.v = make()`
    /// stored nothing, `h.s = 3` on a `text` field walked into `OpSetText` with an
    /// integer and took SIGSEGV, and both went unreported.
    ///
    /// Answers on the pair of TYPES only, so that adding the diagnostic cannot move
    /// codegen. `convert` is the right predicate — it is what both the constructor path
    /// and the scalar-target check already ask — but it is a `&mut self` EMITTER, so it
    /// is asked here in the shape-only form it already understands: a `Value::Null`
    /// expression, which every rewriting arm guards against and no verdict depends on.
    /// `conv_owned_result` is saved and restored around the call because a cast arm sets
    /// it to mark an allocating conversion, and the next real conversion `take()`s it —
    /// a probe that left it set would hand its answer to an unrelated expression.
    ///
    /// Compound assignments are excluded — `h.v += x` appends an ELEMENT, so the
    /// source is legitimately not the field's type, and the operator's own attribute
    /// list types it.
    fn field_store_mismatch(
        &mut self,
        op: &str,
        var_nr: u16,
        f_type: &Type,
        s_type: &Type,
    ) -> bool {
        // Only a plain `=` into a non-variable target, once both types are resolved.
        // Pass 1 sees forward references as `Unknown`; pass 2 re-runs with every def
        // visible, which is where a genuine mismatch is reported (the @P279 tolerance).
        if op != "=" || var_nr != u16::MAX || self.first_pass {
            return false;
        }
        if f_type.is_unknown() || s_type.is_unknown() {
            return false;
        }
        // A bare `null` is exempt because the targets that legitimately take one reach
        // here: a `τ?` field, a `text` field, and a COLLECTION field, where `null` is the
        // empty collection (loft#922).  A `fn(…)` target is not one of them — a fn-ref
        // slot holds a d_nr and has no encoding for absence, which is why the struct
        // LITERAL refuses `Holder { f: null }` outright.  Without naming the exception the
        // assignment wrote d_nr `0` and the next call through the field SIGSEGV'd
        // (loft#1072: the literal and the assignment must accept the same values).
        if matches!(s_type, Type::Null) && !matches!(f_type.base(), Type::Function(_, _, _)) {
            return false;
        }
        // A narrowing integer store has its OWN diagnostic further down; running
        // `convert` here as well would report the same store twice.  The STORE test, to
        // stay paired with the site that reports (loft#931) — `integer` → `i32` is a
        // narrowing that range containment cannot see, and reading it with the narrower
        // test here let both sites fire for one assignment.
        if Self::is_narrowing_int_store(s_type, f_type) {
            return false;
        }
        if f_type.is_equal(s_type) {
            return false;
        }
        // Building a keyed collection FROM a vector of its elements is the supported
        // idiom (`h.m = [E{…}, E{…}]` for a `hash<E[k]>` field) and is what the
        // `OpReplaceKeyed` / keyed-append path exists to serve. The two spellings are
        // deliberately not `is_equal`, and `convert` has no arm for the pair, so the
        // carve-out is named here rather than widened into either of them.
        if let Type::Vector(src_elem, _) = s_type
            && crate::parser::vectors::is_keyed(f_type)
            && f_type.content().is_equal(src_elem)
        {
            return false;
        }
        let saved_owned = self.conv_owned_result.take();
        // A TRIAL of the fit, on a placeholder — @FR-N-Store is asked where the value is
        // actually stored, not here.
        let accepted = self.convert_admitting(&mut Value::Null, s_type, f_type);
        self.conv_owned_result = saved_owned;
        !accepted
    }

    /// The `(struct type, byte offset)` of `to` when it is an
    /// `OpGetField(base, off, ..)` read of a struct field — the pair every schema
    /// question about that field needs. `None` for any other lvalue shape.
    ///
    /// The struct type comes from `parent_tp`, which the assign already holds:
    /// deriving it from the base EXPRESSION instead only resolved a bare
    /// `Value::Var` base, so a group one level down (`o.inner.by_k`) read as "not
    /// a group" and took the unsafe clear. The offset still comes from the call,
    /// because it is relative to whatever struct `parent_tp` names.
    fn field_site(&self, to: &Value, parent_tp: &Type) -> Option<(u16, u16)> {
        let Value::Call(gf_nr, gf_args) = to.unspan() else {
            return None;
        };
        if self.data.def(*gf_nr).name() != "OpGetField" {
            return None;
        }
        let Some(Value::Int(byte_off)) = gf_args.get(1) else {
            return None;
        };
        let d_nr = match parent_tp.base() {
            Type::Reference(d, _) => *d,
            Type::RefVar(inner) => match inner.base() {
                Type::Reference(d, _) => *d,
                _ => return None,
            },
            // loft#1152 — an enum VARIANT's fields live in the variant's own
            // `Parts::EnumValue`, not in the enum, so the enum's own type id names no
            // field and every group question about a variant field answered "no group".
            // The variant is named by the DISCRIMINANT in the guard the field read is
            // already wrapped in.
            Type::Enum(e, _, _) => return self.variant_field_site(*e, to, *byte_off as u16),
            _ => return None,
        };
        let struct_tp = self.data.def(d_nr).known_type();
        if struct_tp == u16::MAX {
            return None;
        }
        Some((struct_tp, *byte_off as u16))
    }

    /// [`Self::field_site`] for a field of an enum VARIANT: the variant's runtime type id
    /// plus the byte offset, or `None` when the variant cannot be named.
    ///
    /// A variant field read is emitted as `OpGetField(if <disc == N> base else
    /// OpNullRefSentinel(), off)` — the guard that makes reading the wrong variant answer
    /// null rather than another variant's bytes. `N` is the only place the variant is
    /// written down at this point, so it is read back out of it and mapped through the
    /// enum's attribute order, which is where the discriminant came from.
    ///
    /// Unguarded shapes (an `is` / `match` binding, whose variant is already established)
    /// yield `None` and keep the behaviour they had.
    fn variant_field_site(&self, e_nr: u32, to: &Value, byte_off: u16) -> Option<(u16, u16)> {
        let Value::Call(_, gf_args) = to.unspan() else {
            return None;
        };
        let Value::If(test, _, _) = gf_args.first()?.unspan() else {
            return None;
        };
        let Value::Call(test_nr, test_args) = test.unspan() else {
            return None;
        };
        if self.data.def(*test_nr).name() != "OpEqInt" {
            return None;
        }
        let Some(Value::Int(disc)) = test_args.get(1).map(Value::unspan) else {
            return None;
        };
        for v in self.data.children_of(e_nr) {
            if self.data.def_type(v) != crate::parser::DefType::EnumValue {
                continue;
            }
            let vname = self.data.def(v).name().to_string();
            let Some(idx) = self
                .data
                .def(e_nr)
                .attributes()
                .iter()
                .position(|a| a.name == vname)
            else {
                continue;
            };
            if i32::try_from(idx).unwrap_or(-1) + 1 != *disc {
                continue;
            }
            let tp = self.data.def(v).known_type();
            return if tp == u16::MAX {
                None
            } else {
                Some((tp, byte_off))
            };
        }
        None
    }

    /// The database collection-type id behind a KEYED type — `sorted` / `hash` / `index` /
    /// `spatial` / `trie` — or `None` for anything else, including a keyed type whose
    /// element record has no runtime type yet (pass 1).
    ///
    /// [`Parser::keyed_known_type`] is the home; this is its name at the expression sites.
    /// It carries the one list of the keyed kinds and the `?` peel, so the two cannot give
    /// different answers about the same collection — which is what they did while each
    /// spelled the five kinds itself.
    fn keyed_type_id(&mut self, tp: &Type) -> Option<u16> {
        self.keyed_known_type(tp)
    }

    /// The clear that must run before a whole-collection literal is built into the
    /// keyed struct field `to` of collection type `kt` (loft#898).
    ///
    /// Two or more keyed fields over one element type are auto-linked by `types.rs`
    /// into several routes to a SINGLE record set (`Field.other_indexes`). **An
    /// operation spelled through any member acts on the group**, which is not a
    /// choice made here — `h.view += [e]` already appends to every member, and has
    /// since loft#843. So `=` replaces the group's whole record set whichever member
    /// it is spelled through, and the members never disagree.
    ///
    /// That decides the shape: reset every VIEW's spine (its hash table, `Ordered`
    /// slot list, or b-tree root), then clear the PRIMARY, which is the one member
    /// that owns the records and so the one free that may happen. Views first, so no
    /// step reads a spine naming records an earlier step already freed.
    ///
    /// The alternative — letting a view be emptied on its own — cannot be made
    /// coherent for a NON-EMPTY literal: the elements must still be inserted into
    /// the group, so `h.view = [e]` would leave the view holding `e` while the
    /// primary holds `e` and everything it had before. A model that works only for
    /// the empty literal is not a model, and the state it produces (an index that
    /// silently does not index the records) has no operation that repairs it.
    ///
    /// An ungrouped field yields the single clear it always had.
    pub(crate) fn keyed_group_clear(
        &mut self,
        to: &Value,
        kt: u16,
        parent_tp: &Type,
    ) -> Vec<Value> {
        let Some((struct_tp, byte_off)) = self.field_site(to, parent_tp) else {
            return vec![self.cl("OpClearKeyed", &[to.clone(), Value::Int(i32::from(kt))])];
        };
        let members = self.database.keyed_group_members(struct_tp, byte_off);
        if members.is_empty() {
            return vec![self.cl("OpClearKeyed", &[to.clone(), Value::Int(i32::from(kt))])];
        }
        let mut ops = Vec::with_capacity(members.len());
        for (off, coll_tp, is_view) in &members {
            if !is_view {
                continue;
            }
            let field = Self::field_at(to, *off);
            let tp = i32::from(coll_tp | crate::database::CLEAR_KEYED_VIEW);
            ops.push(self.cl("OpClearKeyed", &[field, Value::Int(tp)]));
        }
        if let Some((off, coll_tp, _)) = members.iter().find(|(_, _, is_view)| !is_view) {
            ops.push(self.clear_group_primary(to, *off, *coll_tp));
        }
        ops
    }

    /// Clear the member of a linked group that OWNS the records, picking the op its
    /// kind needs: a plain `vector` primary takes `OpClearVector`, every keyed kind
    /// takes `OpClearKeyed` (loft#898).
    ///
    /// A group's owner is whichever field was declared first, so it is as likely to
    /// be the `vector` of the documented `vector<T>` + `hash<T[k]>` pairing as a
    /// keyed collection — and the clear may be reached from either member's assign.
    fn clear_group_primary(&mut self, to: &Value, off: u16, coll_tp: u16) -> Value {
        let field = Self::field_at(to, off);
        let is_plain_vector = matches!(
            self.database.types[coll_tp as usize].parts,
            Parts::Vector(_) | Parts::Array(_)
        );
        if is_plain_vector {
            self.cl("OpClearVector", std::slice::from_ref(&field))
        } else {
            self.cl("OpClearKeyed", &[field, Value::Int(i32::from(coll_tp))])
        }
    }

    /// The view-spine resets that must accompany freeing the records reached
    /// through the struct field `to` (loft#898). Empty unless `to` is the PRIMARY
    /// of a linked collection group.
    ///
    /// Freeing the records is only half of a primary's clear: every sibling view
    /// still holds a spine naming them, and a view left alone reports its old
    /// length over memory that is now free. Both the keyed clear and the VECTOR
    /// clear need this — `vector<T>` + `hash<T[k]>` is the shape DATABASE.md
    /// documents, and there the record holder is the vector — so it lives here
    /// rather than in either one's branch.
    fn keyed_sibling_view_resets(&mut self, to: &Value, parent_tp: &Type) -> Vec<Value> {
        let Some((struct_tp, byte_off)) = self.field_site(to, parent_tp) else {
            return Vec::new();
        };
        let members = self.database.keyed_group_members(struct_tp, byte_off);
        let mut ops = Vec::new();
        for (off, coll_tp, is_view) in members {
            if !is_view || off == byte_off {
                continue;
            }
            let field = Self::field_at(to, off);
            let tp = i32::from(coll_tp | crate::database::CLEAR_KEYED_VIEW);
            ops.push(self.cl("OpClearKeyed", &[field, Value::Int(tp)]));
        }
        ops
    }

    /// loft#1152 — the group re-index that must FOLLOW a whole-vector write into a struct
    /// field: one `OpIndexGroup` per VIEW member.
    ///
    /// The mirror of [`Self::keyed_sibling_view_resets`], and deliberately the same SHAPE.
    /// `Stores::record_finish` keeps a group's members agreeing by walking `other_indexes`
    /// per record, and every route that adds records one at a time reaches it. A whole-vector
    /// write does not: `OpAppendVector` reaches `vector_add` → `vector_add_array`, which moves
    /// the records in bulk. The views were left empty and nothing said so — `len` answered `0`
    /// and a lookup answered `null`, both legal values for a group that happens to be empty.
    ///
    /// ⚠ **The obvious runtime fix is not available, and the reason decides this shape.**
    /// `record_finish` can maintain a group because it is handed `(data, rec, parent_tp,
    /// field)`. `vector_add_array` has only the vector field's `DbRef` and the element type;
    /// `OpAppendVector` carries neither the parent type nor the field index, and recovering
    /// them from the `DbRef` is not a route — `db.pos` is a byte offset into a record whose
    /// type would be a guess.
    ///
    /// The unit of work is what makes the call site the right home anyway: the MEMBERS are
    /// known at emit time, so the parser names them exactly as the clear does, while the
    /// per-RECORD loop lives inside the op, where the records exist. Naming the members here
    /// and looping there is the split loft#898 already made for the clear.
    /// `skip` names byte offsets that must NOT be indexed: members a struct LITERAL fills
    /// itself, which own their records and re-index nothing (loft#1266).  A statement-level
    /// caller writes ONE member and passes an empty set — the question cannot arise there,
    /// because only a constructor writes several members of one group at once.
    pub(crate) fn keyed_sibling_view_fills(
        &mut self,
        to: &Value,
        parent_tp: &Type,
        skip: &std::collections::HashSet<u16>,
    ) -> Vec<Value> {
        let Some((struct_tp, byte_off)) = self.field_site(to, parent_tp) else {
            return Vec::new();
        };
        let members = self.database.keyed_group_members(struct_tp, byte_off);
        let mut ops = Vec::new();
        for (off, coll_tp, _is_view) in members {
            if skip.contains(&off) {
                continue;
            }
            // ⚠ Every OTHER member, not only the views — the filter the RESET beside this
            // one uses answers a different question. A reset may touch only views, because
            // a view owns nothing and the primary's records are released once, by the
            // primary. A FILL has no such asymmetry: the members that need the records are
            // all of them, and which one happens to hold them is not the question.
            if off == byte_off {
                continue;
            }
            let field = Self::field_at(to, off);
            ops.push(self.cl(
                "OpIndexGroup",
                &[to.clone(), field, Value::Int(i32::from(coll_tp))],
            ));
        }
        ops
    }

    /// loft#1152 — wrap a statement that wrote a whole VECTOR VALUE into a grouped struct
    /// field with the group maintenance that write skipped.
    ///
    /// Runs on the finished IR of every assignment rather than at the individual arms,
    /// because a vector field is written from several of them — `=` from an owned var, from
    /// a borrowed var, from an arbitrary expression, `+=`, and the iterator
    /// materialisations — and each builds its own op list. The question they all answer the
    /// same way is *"did this statement `OpAppendVector` into that field?"*, which is a
    /// property of the emitted code, so it is asked once, here.
    ///
    /// The RESET is conditional and the reason is duplicates: the re-index walks the whole
    /// primary, so a view that still holds the previous records would be handed them twice.
    /// A `=` already reset its views (`clear_vector_field`), and an `OpClearKeyed` in the
    /// statement is exactly that receipt; a `+=` has none, so it gets one here. Resetting a
    /// VIEW frees only its spine — its hash table, `Ordered` slot list, or b-tree root — and
    /// never a record, so rebuilding it costs nothing the group owns.
    fn group_reindex_after_vector_write(&mut self, code: &mut Value, to: &Value, parent_tp: &Type) {
        let append_nr = self.data.def_nr("OpAppendVector");
        let clear_keyed_nr = self.data.def_nr("OpClearKeyed");
        let writes_field = |v: &Value| {
            matches!(v.unspan(), Value::Call(d, args)
                if *d == append_nr && args.len() == 3 && *args[0].unspan() == *to.unspan())
        };
        let (wrote, already_reset) = match &*code {
            Value::Insert(ls) => (
                ls.iter().any(writes_field),
                ls.iter()
                    .any(|o| matches!(o.unspan(), Value::Call(d, _) if *d == clear_keyed_nr)),
            ),
            other => (writes_field(other), false),
        };
        if !wrote {
            return;
        }
        let fills = self.keyed_sibling_view_fills(to, parent_tp, &std::collections::HashSet::new());
        if fills.is_empty() {
            return;
        }
        let resets = if already_reset {
            Vec::new()
        } else {
            self.keyed_sibling_view_resets(to, parent_tp)
        };
        let body = match std::mem::replace(code, Value::Null) {
            Value::Insert(ls) => ls,
            other => vec![other],
        };
        let mut ops = resets;
        ops.extend(body);
        ops.extend(fills);
        *code = Value::Insert(ops);
    }

    /// `OpClearVector(to)` for a struct field, preceded by the sibling-view resets
    /// of [`Self::keyed_sibling_view_resets`] when that vector is the record
    /// holder of a linked collection group (loft#898).
    ///
    /// Every arm of the vector-field replace goes through here, so a group whose
    /// primary is a `vector` cannot be emptied by one arm and left with live views
    /// by another.
    fn clear_vector_field(&mut self, to: &Value, parent_tp: &Type) -> Vec<Value> {
        let mut ops = self.keyed_sibling_view_resets(to, parent_tp);
        ops.push(self.cl("OpClearVector", std::slice::from_ref(to)));
        ops
    }

    /// The clear a `= null` performs, plus the mark that says *absent* (loft#917).
    ///
    /// Releasing the records is the same work `= []` does and is not in question — the
    /// field was told to let go of them either way. What `null` adds is the DISTINCTION:
    /// afterwards the field holds `DbRef::ABSENT_REC` rather than the `0` that means an
    /// empty collection, so `f.xs == null` can finally answer true without `f.xs = []`
    /// answering it too.
    ///
    /// Marked only when the field's declared type carries the `?`. The clear itself
    /// deliberately does NOT depend on it (loft#922 — a heap field is one type and one
    /// layout whichever way it is spelled, and gating the RELEASE on the `?` silently kept
    /// records the author had let go). The MARK is the opposite case: `?` is exactly the
    /// declaration that this field may be absent, so writing the marker into a field
    /// declared without one would let it read back as a null its own type forbids.
    fn clear_vector_field_as(
        &mut self,
        to: &Value,
        parent_tp: &Type,
        f_type: &Type,
        nullable: bool,
    ) -> Vec<Value> {
        let mut ops = self.clear_vector_field(to, parent_tp);
        if !nullable || !Self::is_collection_type(f_type.base()) {
            return ops;
        }
        // Write into the HOLDER's 4-byte field word, not through the field READ.
        // `to` is `OpGetField(holder, offset, struct_tp)`, and its VALUE is a reference to
        // the collection — writing at offset 0 of that lands in the collection record's own
        // header. The slot that has to carry the marker is the one the struct literal
        // writes, `OpSetInt4(holder, offset, …)`, so rebuild it from the same two arguments.
        if let Value::Call(_, args) = to.unspan()
            && args.len() >= 2
        {
            let holder = args[0].clone();
            let offset = args[1].clone();
            #[allow(clippy::cast_possible_wrap)]
            let absent = Value::Int(crate::keys::DbRef::ABSENT_REC as i32);
            ops.push(self.cl("OpSetInt4", &[holder, offset, absent]));
        }
        ops
    }

    /// `to` — an `OpGetField(var, off, struct_tp)` read — re-aimed at the sibling
    /// field at byte offset `off`. Rebuilt by swapping the offset in the SAME call
    /// so the base expression, its variable and the struct type all stay whatever
    /// the original site resolved them to.
    /// loft#1159 — name the keyed field `to` the way `OpFillKeyed` needs it: the owning
    /// struct ref, its type id, and the FIELD INDEX.
    ///
    /// The bulk fill places each record through `Stores::record_finish`, the same chokepoint
    /// the element-wise `+= [r]` spelling reaches, and that walk maintains a linked
    /// collection group only when it is given the field the write is spelled through — a
    /// field NUMBER, which a field ref (an `OpGetField` naming a byte offset) does not carry.
    /// Falling back to `(to, kt, u16::MAX)` is the lone-collection convention: the parent IS
    /// the collection and there are no siblings to maintain, which is exactly right wherever
    /// the site cannot be resolved.
    pub(crate) fn fill_keyed_site(
        &mut self,
        to: &Value,
        parent_tp: &Type,
        kt: u16,
    ) -> (Value, u16, u16) {
        if let Some((struct_tp, byte_off)) = self.field_site(to, parent_tp)
            && let Value::Call(_, args) = to.unspan()
            && let Some(parent) = args.first()
            && let Some(idx) = self.database.field_index_at(struct_tp, byte_off)
        {
            return (parent.clone(), struct_tp, idx);
        }
        (to.clone(), kt, u16::MAX)
    }

    fn field_at(to: &Value, off: u16) -> Value {
        let mut out = to.unspan().clone();
        if let Value::Call(_, args) = &mut out
            && let Some(slot) = args.get_mut(1)
        {
            *slot = Value::Int(i32::from(off));
        }
        out
    }

    /// The place a postfix `x?` on an assignment left-hand side was reading, if `to` is
    /// that discharge.
    ///
    /// [`Parser::build_null_coalesce_default`] emits two shapes for one meaning: a
    /// temp-bound `ncc` block when the subject is non-trivial (`b.d?` — the temp keeps the
    /// subject from being evaluated twice), and a bare null-check `if` when it is trivial
    /// (`v?`).  Both name their subject in the position this reads, and that subject is the
    /// place the assignment writes.
    ///
    /// Callers must check [`Parser::last_place_discharge`] first: an explicit `a ?? d`
    /// builds the identical shape and names no place (loft#1205).
    ///
    /// Reads [`null_discharge_subject`], the one home for what a discharge looks like, which
    /// [`lhs_base_var`] also consults — so "which place was this discharge reading?" and "which
    /// binding does a write through a discharge reach?" cannot drift apart.  That home also
    /// declines loft#980's variant-field guard, the OTHER `if` that reaches a left-hand side:
    /// its then arm is the receiver rather than a place, so a guard reached with the flag set
    /// by a `?` elsewhere in the same left-hand side is left alone instead of peeled into
    /// (loft#1211).
    fn peel_place_discharge(to: &Value, data: &crate::parser::Data) -> Option<Value> {
        null_discharge_subject(to, data).cloned()
    }

    /// The subject of a discharge sitting at the RECEIVER of a KEYED element accessor —
    /// `h?[k]`, `n.h?[k]` — when the place is one, and `None` when it is not.
    ///
    /// `@FR-E-Asgn-Discharge` says the write lands in `place`, and for a collection the `?`
    /// asks for exactly what the place already does.  That holds one level in as well: the
    /// discharge here is the RECEIVER of the target rather than the target, and the place the
    /// write must reach is still the collection the `?` was reading.  Left unpeeled, the null
    /// path writes into the fresh default the discharge builds for its `else` arm — a
    /// collection nobody holds, dropped at the end of the statement.
    ///
    /// A KEYED accessor only.  A keyed element write is the one element write that CREATES its
    /// slot, so it is the only one for which "build the empty collection first" changes the
    /// answer; a vector element write addresses an EXISTING slot, and on the null path there is
    /// no element either side of the peel.  A struct-field walk (`OpGetField`) is excluded on
    /// the rule's own grounds — `h.i?.x = …` has no "build the empty one first" on its null
    /// path, which is why loft#1211 resolves it through [`lhs_base_var`] instead.
    ///
    /// Reads [`null_discharge_subject`], the one home for what a discharge looks like, so this
    /// and the whole-place spelling cannot drift apart about which shapes carry one.
    fn keyed_receiver_discharge<'a>(
        to: &'a Value,
        data: &crate::parser::Data,
    ) -> Option<&'a Value> {
        let Value::Call(d_nr, args) = to.unspan() else {
            return None;
        };
        if data.def(*d_nr).name() != "OpGetRecord" {
            return None;
        }
        null_discharge_subject(args.first()?, data)
    }

    /// Does this assignment PLACE read through a null discharge, in either position?
    ///
    /// The one predicate both readers of a discharged left-hand side ask: the refusal of an
    /// explicit `(a ?? d)`, which names two values and no place, and the peel that rewrites a
    /// postfix `x?` to the place it was reading.  While the refusal saw only the whole-place
    /// spelling, `(n.h ?? [])[k] = v` was neither refused nor peeled — it lowered to a write
    /// into the coalesce's throwaway default and lost it in silence.
    fn place_reads_through_discharge(to: &Value, data: &crate::parser::Data) -> bool {
        null_discharge_subject(to, data).is_some()
            || Self::keyed_receiver_discharge(to, data).is_some()
    }

    /// Seed `place` with type `base`'s default when — and only when — it is null, so a
    /// compound assignment written `place? op= e` reads the default the `?` asked for.
    ///
    /// GUARDED, rather than the shorter `place = place?`: that spelling reads the place on
    /// its own right-hand side, and a `text` local's store CLEARS the destination before it
    /// copies, so the copy read a buffer the clear had just emptied — `t? += "cd"` on
    /// `t = "ab"` answered `"cd"`.  The guard's store takes a CONSTANT, so no place is
    /// ever its own source.  `None` when the type has no default (`(D-NoRef)`) or the place
    /// is a shape [`Parser::place_store`] cannot write, which leaves the statement as the
    /// peel alone made it.
    fn discharge_seed(&mut self, place: &Value, base: &Type) -> Option<Value> {
        let (default, _) = self.build_default(base)?;
        let store = self.place_store(place, default)?;
        let present = self.coalesce_not_null(place, base);
        let absent = self.cl("OpNot", &[present]);
        Some(v_if(absent, store, Value::Insert(Vec::new())))
    }

    /// Emit a store of `value` into `place`, where `place` is the left-hand side of an
    /// assignment as the expression parser built it.
    ///
    /// The two shapes an assignment place takes: a bare local (`Value::Var`), and a heap
    /// read (`OpGet<T>(base, offset)`) whose writing twin
    /// [`Parser::call_to_set_op`] already names.  `None` for anything else, which leaves
    /// the caller with no seed rather than a store to a place it could not identify.
    fn place_store(&mut self, place: &Value, value: Value) -> Option<Value> {
        match place.unspan().clone() {
            Value::Var(v) if v != u16::MAX => Some(v_set(v, value)),
            Value::Call(d, args) => {
                let name = self.data.def(d).name().to_string();
                if !name.starts_with("OpGet") {
                    return None;
                }
                Some(self.call_to_set_op(&name, &args, value, "="))
            }
            _ => None,
        }
    }

    /// Apply the operator `op` to an already-parsed LHS and parse the RHS, then rewrite
    /// `code` into the assignment IR.  Returns `Type::Void`.
    // threads LHS context (to, f_type, parent_tp, var_nr) alongside op and &mut self
    #[allow(clippy::too_many_arguments)]
    /// Every assignment form routes through here; the body is
    /// [`Self::parse_assign_op_inner`], and this wraps it with the linked-group
    /// maintenance a whole-vector field write skips (loft#1152 — see
    /// [`Self::group_reindex_after_vector_write`]).
    #[allow(clippy::too_many_arguments)] // the inner fn's parameter list, forwarded
    pub(crate) fn parse_assign_op(
        &mut self,
        code: &mut Value,
        op: &str,
        f_type: &Type,
        to: &Value,
        parent_tp: Type,
        var_nr: u16,
        skip_validate: bool,
    ) -> Type {
        let group_parent = parent_tp.clone();
        let group_to = to.clone();
        let already = std::mem::replace(&mut self.rebind_lowered, u16::MAX);
        let tp = self.parse_assign_op_inner(code, op, f_type, to, parent_tp, var_nr, skip_validate);
        self.rebind_local_heap_param(code, op, to, var_nr);
        self.rebind_lowered = already;
        self.group_reindex_after_vector_write(code, &group_to, &group_parent);
        tp
    }

    /// `(F-ParamRebind)` — a WHOLE-VALUE reassignment of a user-visible heap PARAMETER
    /// rebinds LOCALLY, and the rule is about the BINDING, not about the right-hand side's
    /// spelling: `p = other` is named in the rule's own text beside `p = [..]`.
    ///
    /// Only the struct-literal spelling had a lowering (@PLN87 P2.1, inside `parse_object`,
    /// where a literal that builds IN PLACE needs its detach between the construction's own
    /// ops).  Every other right-hand side reached codegen as a bare `Set`, which the
    /// interpreter lowers to a deep copy INTO the record the parameter's slot names — the
    /// caller's store — while `--native` reassigns its own by-value `DbRef` local.  So five
    /// of six spellings wrote back to the caller on at least one backend, and three of them
    /// disagreed between the two, against `ownership.md` `(O-NoDiverge)` (loft#1290).
    ///
    /// The lowering is P2.1's, wrapped around the finished statement: free a PRIOR rebind
    /// store (never the caller's original — that is what the entry witness is for), detach
    /// the slot WITHOUT freeing, and let the assignment mint into the emptied slot.
    ///
    /// A `&`/`RefVar` parameter is the opposite rule (`F-ParamRef` — write-back is what it
    /// is FOR), a compiler-generated or hidden parameter is a return buffer whose in-place
    /// write IS its purpose, and a field or element write is not a whole-binding
    /// reassignment.  Vectors and keyed collections keep their own P2.4 route.
    ///
    /// The shape test reads through `Optional`, because `τ?` and `τ` share sentinel storage
    /// and a rebind of `p: St?` displaces a store exactly as `p: St` does.  It kept the
    /// caller's VALUE either way — which is what made the gap easy to read as covered — while
    /// the store the callee minted had no owner: forty calls, forty leaked records.  A
    /// nullable parameter is still a parameter (loft#1295).
    fn rebind_local_heap_param(&mut self, code: &mut Value, op: &str, to: &Value, var_nr: u16) {
        if self.first_pass
            || op != "="
            || var_nr == u16::MAX
            || self.rebind_lowered == var_nr
            || !matches!(to.unspan(), Value::Var(v) if *v == var_nr)
            || !(matches!(
                self.vars.tp(var_nr).base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            ) || crate::parser::vectors::is_keyed(self.vars.tp(var_nr)))
            || !self.vars.is_argument(var_nr)
            || self.vars.is_compiler_generated(var_nr)
            || self.is_hidden_param(var_nr)
        {
            return;
        }
        let orig = self.ensure_rebind_witness(var_nr);
        let free = self.cl(
            "OpFreeRefIfDistinct",
            &[Value::Var(var_nr), Value::Var(orig)],
        );
        let detach = self.cl("OpInitRefSentinel", &[Value::Var(var_nr)]);
        let assign = std::mem::replace(code, Value::Null);
        // Enforces @FR-O-Detach — the detach is sequenced AFTER every read of the binding by
        // the value being assigned to it.  `free, detach, assign` is only correct while the
        // RHS does not read `var_nr`: the detach sentinels the slot, so a RHS that reads it
        // reads `null`.  Measured on `p = mk(p.a + 1)`, which answered `null` on BOTH backends
        // with no diagnostic while `p = Big{a: p.a + 1, …}` answered correctly — the literal
        // lowering in `objects.rs` is this same rule's other copy and hoists its field reads
        // into temporaries before detaching (loft#1312).
        //
        // The hoist is what that copy does, spelled for an arbitrary RHS: evaluate into a temp
        // while the binding is still intact, then detach, then adopt.  It is the compiler
        // applying the workaround a user would write by hand (`t = mk(p.a + 1); p = t`), which
        // is what makes the target shape a proven artifact rather than a guess.
        if let Value::Set(dst, rhs) = &assign
            && *dst == var_nr
            && rhs.reads_var(var_nr)
        {
            let tp = self.vars.tp(var_nr).clone();
            let hoisted = self.vars.work_refs(&tp, &mut self.lexer);
            let eval = Value::Set(hoisted, rhs.clone());
            let adopt = Value::Set(var_nr, Box::new(Value::Var(hoisted)));
            *code = Value::Insert(vec![eval, free, detach, adopt]);
            return;
        }
        *code = Value::Insert(vec![free, detach, assign]);
    }

    /// Record that this lambda rebinds a captured name whole-value.
    ///
    /// Only the FACT is recorded here; whether it is legal depends on what the name is
    /// bound to in the enclosing scope, which this frame cannot see — the lambda's own
    /// variable table holds a placeholder. `reject_rebound_heap_parameter_captures` asks
    /// that question at the lambda's exit.
    fn note_captured_rebind(&mut self, var_nr: u16, op: &str) {
        if op != "=" || var_nr == u16::MAX || self.captured_names.is_empty() {
            return;
        }
        let name = self.vars.name(var_nr).to_string();
        if !self.captured_names.iter().any(|(n, _)| *n == name) {
            return;
        }
        let noted = self.rebound_captures.entry(self.context).or_default();
        if !noted.contains(&name) {
            noted.push(name);
        }
    }

    #[allow(clippy::too_many_arguments)] // the wrapper's list, unchanged from before the split
    fn parse_assign_op_inner(
        &mut self,
        code: &mut Value,
        op: &str,
        f_type: &Type,
        to: &Value,
        mut parent_tp: Type,
        mut var_nr: u16,
        skip_validate: bool,
    ) -> Type {
        self.check_iter_safety(to, f_type, op);
        // @FR-Const-Value / @FR-Const-Bind — ask the const question ONCE, here, ahead of
        // every route below.  Whether a write is allowed is a property of the BINDING, not
        // of the route that lowers it, so a guard held inside a route is only as complete
        // as that route's target-shape test and every shape it declines falls through
        // unchecked.  Two did.
        self.guard_const_write(var_nr, op);
        // …and note a whole-value rebind of a CAPTURE here for the same reason: this is the
        // point that still knows the assignment replaces the whole binding.  By the time the
        // lambda closes, a vector rebind is a clear plus appends and the `Value::Set` that
        // said so is gone (loft#1281).
        self.note_captured_rebind(var_nr, op);
        // Save parent struct type before the RHS parse overwrites parent_tp.
        let lhs_parent_tp = parent_tp.clone();
        // …and, for the same reason, the attribute a `fn(…)` field read on the LEFT came
        // from: the RHS parsed below may itself read a fn-ref field (`a.f = b.g`) and
        // would leave that attribute behind instead (loft#1072).
        let lhs_fn_attr = self.fn_ref_read_attr.take();
        // #330: `x = x` is the identity — emit nothing.  Letting it through
        // produced a deep-copy-onto-self whose pre-Set free released the
        // store the RHS was about to read (silent corruption on the
        // interpreter, null on native — and the two diverged).  Killing it
        // here covers BOTH backends and keeps the store identity stable
        // for any live borrows.
        if op == "="
            && let Value::Var(lhs) = to
            && self.lexer.peek().has
                == crate::lexer::LexItem::Identifier(self.vars.name(*lhs).to_string())
        {
            let link = self.lexer.link();
            self.lexer.cont();
            if self.lexer.peek_token(";") {
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
            self.lexer.revert(link);
        }
        // @P277 — `local_keyed += [literal]`.  Without this branch, the
        // RHS parse below descends into `parse_vector` (vectors.rs:1372)
        // which at line ~1434 calls `change_var_type(vec, Vector<T>, …)`
        // and fires "Variable 'x' cannot change type from sorted<…> to
        // vector<…>" — the LHS's declared keyed-collection type is lost.
        // The P188 scalar branch below (line ~870) runs AFTER the RHS
        // parse, so by then the diagnostic has already fired.  Intercept
        // here, manually tokenise the literal, parse each item against
        // the element type, and dispatch via `new_record` (vectors.rs:1745)
        // which already routes per-kind (sorted_finish / hash::add /
        // tree::add / ordered_finish) via its P188-followup `lhs_known`
        // lookup at lines 1783-1822.  The struct-field twin at lines
        // ~1159-1194 handles the same shape for fields after-the-fact;
        // we cannot do that here because the LHS-local path errors out
        // before reaching it.
        //
        // loft#1233 — a keyed collection CAPTURED into a closure is a destination this
        // branch owns too.  It is reached through an `OpGetDbRef` of the closure record
        // rather than by name, so `var_nr == u16::MAX` and the local gate alone skipped it.
        // The RHS then parsed as a free-standing `[…]`, which against a keyed hint builds a
        // WHOLE `hash<E[k]>`, and the assignment REBOUND the capture to that fresh
        // one-element collection: every append destroyed the previous contents (and the
        // records the caller put there before the lambda existed), leaving only the last —
        // silent, both backends.  A one-append test reads correct, which is how it stayed
        // unfound.
        //
        // The other three place kinds all parse this literal per-element already — a local
        // here, a field through the after-the-fact twin — and the capture's own BARE
        // spelling (`h += E { … }`) is routed by the `dbref_append_target` arm below.  So
        // this is the one cell of place-kind × spelling that reached no per-element route,
        // and the cure is to let the capture in rather than to add a fourth route.
        //
        // `is_captured_dbref` alone is too weak — a struct-field read is an `OpGetDbRef`
        // too.  The gate is the one `dbref_append_target` uses below, so the interception
        // and the routes it feeds cannot disagree about what a captured collection is.
        let captured_keyed = var_nr == u16::MAX
            && self.closure_param != u16::MAX
            && Self::is_collection_type(f_type)
            && f_type.depend().contains(&self.closure_param)
            && self.is_captured_dbref(to);
        if op == "+="
            && (var_nr != u16::MAX || captured_keyed)
            && crate::parser::vectors::is_keyed(f_type)
            && self.lexer.peek_token("[")
        {
            let elm_tp = f_type.content();
            self.lexer.token("[");
            // Empty literal `+= []` — no-op append.
            if self.lexer.has_token("]") {
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
            let mut all_steps: Vec<Value> = Vec::new();
            loop {
                let elm = self.unique_elm_var(&lhs_parent_tp, &elm_tp, var_nr);
                let mut item = Value::Var(elm);
                let mut item_parent = Type::Null;
                let _ = self.parse_operators(&elm_tp, &mut item, &mut item_parent, 0);
                if !self.first_pass {
                    // A capture has no owning struct and no name: it is placed by the
                    // `OpGetDbRef` itself, and `record_new`'s kind dispatch reads the
                    // COLLECTION type when the field is `u16::MAX` — the same two
                    // substitutions the `dbref_append_target` routes below make.
                    let steps = if captured_keyed {
                        self.new_record(&mut to.clone(), f_type, elm, u16::MAX, &[item], &elm_tp)
                    } else {
                        self.new_record(
                            &mut Value::Var(var_nr),
                            f_type,
                            elm,
                            var_nr,
                            &[item],
                            &elm_tp,
                        )
                    };
                    all_steps.extend(steps);
                }
                if !self.lexer.has_token(",") {
                    break;
                }
                if self.lexer.peek_token("]") {
                    // trailing comma
                    break;
                }
            }
            self.lexer.token("]");
            *code = Value::Insert(all_steps);
            return Type::Void;
        }
        // @PLN135 arc A — `local_keyed += Struct { … }` (no brackets) builds the entry
        // IN PLACE, the way the bracketed `+= [Struct { … }]` above already does.
        //
        // The P188 branch below runs AFTER the RHS parse and retargets the literal's
        // field writes onto the fresh element (`substitute_value(Var(var_nr) →
        // Var(elm))`).  That retarget never fired: the RHS parses with the COLLECTION
        // local as its target, and `parse_object` rejects a target whose type is not
        // `Reference(<this struct>)` — a `hash<Entry[key]>` is not — so it allocates a
        // throwaway work-ref instead, and the append became claim-scratch, write fields,
        // `OpCopyRecord` into the entry, free scratch.  Handing the parse a target of the
        // ELEMENT type removes all three: the literal writes straight into the slot
        // `OpNewRecord` carved out of the collection.  Measured on 1M `integer`-keyed
        // inserts, `--native-release`: 933ms → 555ms.
        //
        // Gated on the RHS being a fresh literal of exactly the element type, peeked as
        // `<element-name> {`:
        //  - a variable / call / field read must keep the deep copy (it names a record
        //    that already exists elsewhere), and reaches `new_record`'s `OpCopyRecord`
        //    arm unchanged;
        //  - a qualified (`pkg::Entry {`) or generic (`Pair<integer> {`) spelling does
        //    not match the bare name and keeps the copy — slower, never wrong;
        //  - a `__nullable<S>` element (@PLN25 E2) is excluded: its literal is written
        //    `S { … }` but the element var is typed as the `Some` VARIANT, so the names
        //    do not correspond and the transparent-construction path owns that shape.
        if op == "+="
            && var_nr != u16::MAX
            && crate::parser::vectors::is_keyed(f_type)
            && let elm_tp = f_type.content()
            && let Type::Reference(elm_d, _) = &elm_tp
            && !self.data.def(*elm_d).name.starts_with("__nullable<")
            && self.peek_literal_of(&self.data.def(*elm_d).name.clone())
        {
            let elm = self.unique_elm_var(&lhs_parent_tp, &elm_tp, var_nr);
            let mut item = Value::Var(elm);
            let mut item_parent = Type::Null;
            let item_tp = self.parse_operators(&elm_tp, &mut item, &mut item_parent, 0);
            if !elm_tp.is_equal(&item_tp) {
                // Unreachable in practice — the peek matched the element type's own
                // name.  It becomes reachable if that name resolves to a DIFFERENT
                // definition (a shadowing import), and a silent `OpCopyRecord` against
                // the element's layout would then mis-lay the other struct's fields
                // into the entry.  Refuse instead.
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot append {item_tp} to a collection of {elm_tp}"
                );
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
            let mut steps: Vec<Value> = Vec::new();
            if !self.first_pass {
                steps = self.new_record(
                    &mut Value::Var(var_nr),
                    f_type,
                    elm,
                    var_nr,
                    &[item],
                    &elm_tp,
                );
            }
            *code = Value::Insert(steps);
            return Type::Void;
        }
        // Hint the RHS that the destination has this type — `f#read`
        // (no parens, no cast) picks it up so `s.field = f#read` matches
        // the symmetry of `f += s.field` (which already takes the field's
        // declared width).  Restored to Unknown after the RHS parse so
        // it doesn't leak into unrelated sub-expressions.
        // @PLN87 P2.4 — a `v = [..]` whole-binding REPLACE on a visible vector
        // PARAM rebinds LOCALLY.  Detect it BEFORE the RHS parse — the literal
        // materialises in parse_vector, which never reaches this fn's tail — by
        // peeking for the `[`: mark the param as a rebind so the RHS parse's
        // `vector_db` hands it a FRESH backing (instead of appending to the
        // caller's store), and the witness frees that backing at exit (the P2.1
        // rebind infra).  `+=` (op != "="), and `v = v + [..]` / `v = other` (RHS
        // is not a bare literal), keep the caller backing.  A `&`/RefVar vector
        // param is handled earlier by `assign_refvar_vector`.
        if !self.first_pass
            && op == "="
            && var_nr != u16::MAX
            && matches!(f_type, Type::Vector(_, _))
            && self.vars.is_argument(var_nr)
            && !self.vars.is_compiler_generated(var_nr)
            && !self.is_hidden_param(var_nr)
            && self.lexer.peek_token("[")
        {
            self.ensure_rebind_witness(var_nr);
        }
        // @PLN87 P2.4 — `v = [..]` whole-binding REPLACE on a `&`-vector param
        // WRITES BACK to the caller.  A `&`-vector ref shares the caller's backing
        // in place (it cannot repoint at a fresh store), so the write-back is a
        // CLEAR + refill of that backing: prepend `OpClearVector(v)` so the
        // literal — which appends to v's (the caller's) store — yields a replace,
        // not a grow.  `+=` keeps the grow (handled by `assign_refvar_vector` /
        // the parse_block expansion).  Detected by peeking the leading `[`.
        let amp_vector_replace = !self.first_pass
            && op == "="
            && var_nr != u16::MAX
            && matches!(f_type, Type::RefVar(inner) if matches!(**inner, Type::Vector(_, _)))
            && self.vars.is_argument(var_nr)
            && self.lexer.peek_token("[");
        let prev_read_target = std::mem::replace(&mut self.expected, f_type.clone());
        let rhs_pos = self.lexer.peek_pos().clone();
        // @PLN87 B-Ref-AnnotationOnly — a plain `=` RHS is the one expression position
        // where a leading `&` binds a reference, so open the head there.  A COMPOUND
        // assignment (`b += &a`) is excluded on purpose: it mutates `b`, it does not
        // give `b` a reference type, so it is not a bind site.
        self.amp_head = if op == "=" {
            AmpHead::AssignRhs
        } else {
            AmpHead::No
        };
        // Name the destination this assignment writes, for this RHS only, along with
        // whether it REPLACES it.  A comprehension that reads its own destination needs
        // both: `=` repoints the target at a fresh store, `+=` appends into what it already
        // holds, and the two need different deliveries (`I-Comp`).  Saved and restored
        // because the RHS may contain assignments of its own.
        let prev_target = std::mem::replace(&mut self.assign_target, var_nr);
        let prev_replaces = std::mem::replace(&mut self.assign_replaces, op == "=");
        let prev_snapshot_len = std::mem::take(&mut self.build_snapshot_len);
        let mut s_type = self.parse_operators(f_type, code, &mut parent_tp, 0);
        // `@FR-L-Null-Which` — a LOCAL spells `S?` as the POINTER (`Optional(Reference(S))`,
        // `nullref` for absence); the tagged `__nullable<S>` is a SLOT's spelling — an embedded
        // field, a vector element, a tuple member.  A projection of such a slot bound to a
        // local is therefore read THROUGH THE TAG here, at the bind, so the local's IR type is
        // the pointer in both passes (pass 1 types the field read as `Optional(Reference(S))`
        // already; the synth arrives in pass 2, after `fill_all`).  Left as the slot's
        // spelling, the binding took whichever spelling parsed LAST and the other assignment's
        // value went in unconverted: `x = y; x = o.opt` read the pointer `y` as a tagged record
        // and freed its store, `x = o.opt; x = y` wrote `x.n` onto the tag byte (loft#1367).
        // A `&` link keeps the slot (it IS the slot), and a field or element TARGET is a
        // slot-to-slot copy with its own lowering — a plain local target only.
        if op == "=" && !self.amp_pending && matches!(to.unspan(), Value::Var(_)) {
            self.read_through_tag(code, &mut s_type);
        }
        // `@FR-B-Copy` (with `@FR-T-Cons`) — a whole-tuple bind `u = t` COPIES every heap
        // member.  It is lowered onto the literal's own per-member copy: the source
        // becomes the tuple of its member reads and `tuple_member_owned_copy` gives each heap
        // member a frame-local backing, so both backends bind an INDEPENDENT value.  A bind
        // used to copy the tuple's stack words, and a heap member's word is a handle, so
        // `u = t; t.0 += [9]` grew `u.0` too (loft#1361).  A local TARGET only (a field or
        // element write has its own lowering), never a `&` link, and a parameter source copies
        // like every plain bind off a parameter does.
        if op == "="
            && !self.amp_pending
            && matches!(to, Value::Var(_))
            && let Value::Var(src) = code.unspan()
            && let Type::Tuple(elems) = self.vars.tp(*src).clone()
            && elems.iter().any(|e| !crate::data::is_scalar(e.base()))
        {
            let src = *src;
            let mut types = elems.clone();
            let mut members: Vec<Value> = Vec::with_capacity(elems.len());
            for (i, t) in elems.iter().enumerate() {
                let mut m = Value::TupleGet(src, i as u16);
                if let Some(owned) = self.tuple_member_owned_copy(&mut m, t) {
                    types[i] = owned;
                }
                members.push(m);
            }
            *code = Value::Tuple(members);
            s_type = Type::Tuple(types);
        }
        // tuples.md T-Ref — a tuple LITERAL bound to a local that is the SOURCE OF A `&` LINK
        // (recorded in pass 1 at the link or the call) and carries a heap element is built as
        // the `__tuple<…>` RECORD a heap-tuple return and a loop variable already are, so the
        // link can name it.  Every other tuple local keeps its stack form.  Built by the
        // return path's own builder.
        if op == "="
            && let Value::Var(lhs) = to
            && matches!(code.unspan(), Value::Tuple(_))
            && let Type::Tuple(ref types) = s_type
            && types.iter().any(|t| !crate::data::is_scalar(t.base()))
            && types.iter().all(crate::data::ref_tuple_record_element_ok)
            && self
                .ref_linked_tuple_locals
                .contains(&(self.context, self.vars.name(*lhs).to_string()))
        {
            let types = types.clone();
            let synth = self.data.tuple_def(&mut self.lexer, &types);
            if synth != u32::MAX {
                let synth_ref = Type::Reference(synth, Deps::none());
                let w = self.vars.work_refs(&synth_ref, &mut self.lexer);
                let kt = self.data.def(synth).known_type();
                self.rewrite_tail_tuple_with_work_ref(synth, kt, w, code);
                s_type = Type::Reference(synth, Deps::frame1(w));
                // The local was typed from the literal while the RHS parsed; it IS the record
                // now, and the stack-tuple type would otherwise unbox the record back.
                self.vars
                    .set_type(*lhs, Type::Reference(synth, Deps::none()));
            }
        }
        self.assign_target = prev_target;
        self.assign_replaces = prev_replaces;
        // The count the right-hand side's literal recorded, for the detach sites below.
        let snapshot_len = std::mem::replace(&mut self.build_snapshot_len, prev_snapshot_len);
        self.amp_head = AmpHead::No;
        self.expected = prev_read_target;
        // A `& vector` bind (`d = &v` / `d = &self.data`): the source is a vector lvalue
        // and the `&` opts INTO aliasing (B-Ref-Write — the write-through "north star" —
        // for a vector, which plain `d = v` deliberately does NOT give: it COPIES,
        // H-Copy).  Capture it BEFORE `amp_pending` is cleared below so the vector-copy
        // classifier is told to SHARE instead — `d` binds to the source's DbRef with no
        // deep copy and is NON-OWNING (its dep names the source, so `owns = dep.is_empty()`
        // is false and it never frees the source's store).  `d[i] = x` then writes THROUGH.
        // Asked through the `RefVar` wrapper: the ANNOTATED spelling (`pe: &vector<T> = e`)
        // arrives with `s_type` already coerced to the annotation's target, so an unpeeled
        // test saw no vector, took the whole vector COPY lowering, and still left the
        // variable carrying the annotation's `RefVar` over a value — the interpreter then
        // read the vector's buffer as a stack ref and panicked (loft#1371).
        let amp_vector_source = match &s_type {
            Type::RefVar(inner) => inner.base(),
            other => other.base(),
        };
        let amp_vector_bind =
            op == "=" && self.amp_pending && matches!(amp_vector_source, Type::Vector(_, _));
        // loft#1371 — the share aliases element writes and appends, but a WHOLE-VALUE write
        // (`pe = [2, 2]`) would mint a fresh store and re-point `pe` at it, leaving the
        // source untouched with nothing said.  Name the local here so `create_vector` clears
        // the SHARED store and refills it in place instead — `@FR-B-Ref-Write`, and the
        // lowering a `&vector` PARAMETER already takes.
        if amp_vector_bind && var_nr != u16::MAX && !self.first_pass {
            let name = self.vars.name(var_nr).to_string();
            self.amp_vector_locals.insert((self.context, name));
            // @FR-B-Ref-Alias — the link is live in BOTH directions, so the SOURCE's own
            // whole-value write is the same question as the link's.  A vector link shares the
            // source's `DbRef`, and a fresh backing on either side re-points one of them: with
            // only the link registered, `q = &v; v = [7, 8, 9]` left `q` on the store `v` held
            // AT THE BIND — `len(q)` answered 2 where `v` answered 3, on both backends and
            // with nothing said (loft#1392).  Registering the source makes its rebuild clear
            // and refill the shared store in place, which is what every other spelling of the
            // link already does: a `&` to a struct or a text local follows a rebind through
            // `OpCreateStack`, and a link to a FIELD follows one because a field rebuild is
            // in place already.  Only a plain local source needs it — a field or an element
            // source is that in-place case.
            if let Value::Var(src) = code.unspan() {
                let src_name = self.vars.name(*src).to_string();
                self.amp_vector_locals
                    .insert((self.context, src_name.clone()));
                // The pair is one place, so record it both ways: `(I-Comp)` asks whether a
                // build reads its destination, and under `r = &a` the build `a = [for i in
                // 0..r.len() { r[i] }]` reads it without naming it.
                let link_name = self.vars.name(var_nr).to_string();
                self.amp_vector_link_partners
                    .entry((self.context, link_name.clone()))
                    .or_default()
                    .push(src_name.clone());
                self.amp_vector_link_partners
                    .entry((self.context, src_name))
                    .or_default()
                    .push(link_name);
            }
            // `pe: &vector<T> = e` IS `pe = &e` (@PLN87 #2), and a vector link is the SHARED
            // `DbRef` rather than a stack deref — so both spellings give the variable the
            // same vector type.  Kept as `RefVar(vector<T>)` the annotation described a link
            // the bind never built, and every read went through a deref that had nothing
            // behind it.
            if let Type::RefVar(inner) = self.vars.tp(var_nr).clone() {
                self.change_var_type(var_nr, &inner);
            }
            if let Type::RefVar(inner) = s_type.clone() {
                s_type = *inner;
            }
        }
        // @PLN130 F9 step 2 — track whether the `&` finds a lowering below.  A STRUCT-typed
        // projection (`c = &v[0]`, `c = &o.inner`) finds none: it is already a VIEW under
        // B-View, so both spellings emit byte-identical IR and the `&` was dropped as
        // redundant.  It stopped being redundant when F2 made a view MATERIALISE on a
        // reshape — from then on `&` also says *"and do not silently copy it"*.
        let mut amp_unlowered = op == "=" && self.amp_pending && !amp_vector_bind;
        // @PLN87 L1 / #2 — a local `&`-binding to a SCALAR lvalue (`b = &a` or
        // `b: &integer = a`) makes `b` a LIVE reference to the source's stack slot:
        // lower it to `b: &T = OpCreateStack(a)` — the SAME stack-ref mechanism a `&T`
        // parameter uses, so reading (L1) and writing (L2) `b` deref to `a`'s slot.
        // `amp_pending` is set by the prefix `&` (`b = &a`) OR the typed-annotation
        // `&` (`b: &integer = a`); we gate on the SOURCE var's actual type (`s_type`
        // is coerced to the RefVar target in the annotated form).  A non-scalar source
        // keeps the single-indirect view from P1; a non-`Var` source is a later rung.
        if op == "=" && self.amp_pending {
            // Through `base()`: a nullable source reaches the `&` lowering like its non-null
            // twin — and is then DECLINED there (`ref_var_type`), loudly.  Asked bare, the `?`
            // fell past every arm below and the `&` was silently a copy: `p = 7` left
            // `x: integer?` unchanged on both backends (`@FR-B-Ref-Reshape`: loft declines a
            // link it cannot honour, it never downgrades it to a copy).
            let is_scalar = |t: &Type| {
                matches!(
                    t.base(),
                    Type::Integer(..)
                        | Type::Float
                        | Type::Single
                        | Type::Boolean
                        | Type::Character
                )
            };
            // L1 / #2 — a scalar stack LOCAL source (`b = &a` / `b: &T = a`).
            // L5 — a HEAP whole-value source (`p = &o`, `o: Reference`): a NON-OWNING
            // alias of the source's record.  A heap local COPIES on `p = o` (value type),
            // so the `&` makes `p` share `o` instead.  Both lower to `OpCreateStack(src)`
            // — a reference to `src`'s slot, the SAME stack-ref a `&T` PARAMETER uses
            // (interp: `GetStackRef` deref; native: the record DbRef by value, the #257
            // alias shape).  A heap source additionally marks `p` non-owning (skip_free):
            // `o` frees the record, not the alias.
            // A TUPLE local joins the scalars here rather than the heap sources below:
            // it lives in the frame, so `OpCreateStack` gives exactly the stack ref a
            // `&(…)` PARAMETER is already handed at its call site, and the element ops
            // read it at the same `(ref, offset)` pair.  Without this arm the `&` reached
            // no lowering at all — `b = &a` silently COPIED the tuple (B-Ref-Alias says
            // it must LINK), and `b: &(integer, integer) = a` typed `b` as a reference
            // over a value, so the interpreter read an element as a store index and
            // `--native` handed the user a raw `E0308` (D-tup-2).
            // @FR-B-Ref-Uniform — the link is carried by the TYPE, and the source's type
            // kind does not narrow which sources may have one: a `&` local bind takes the
            // SAME `RefVar(τ)` + `OpCreateStack(src)` form for every τ the `&` PARAMETER
            // channel already carries (calls.md F-ParamRef), TEXT and VECTOR included.
            // Asked for `Reference | Tuple` alone, a TEXT source reached no lowering at
            // all: the `&` was dropped and the bind COPIED, so neither a read nor a write
            // crossed the link, and the annotated spelling (`pc: &text = c`) typed the
            // variable as a link over a value and read the buffer as a stack ref
            // (loft#1371).  A VECTOR source keeps the DbRef share below — it aliases the
            // element writes and the appends already — and only its whole-value write
            // needs the link.
            let stack_src = match *code.unspan() {
                Value::Var(src)
                    if is_scalar(self.vars.tp(src))
                        || matches!(
                            self.vars.tp(src).base(),
                            Type::Reference(..) | Type::Tuple(_) | Type::Text(_)
                        ) =>
                {
                    Some(src)
                }
                _ => None,
            };
            // L3 / L4 — a scalar HEAP-place source: a vector ELEMENT (`c = &v[0]`) or a
            // struct FIELD (`r = &s.x`).  Both lower to a scalar value-read
            // `OpGet*(<base>, fld)`; the place's DbRef is:
            //   element — the inner `OpGetVector`/`OpVectorRef` accessor itself
            //             (`OpGet*(OpGetVector(v,..), 0)` → strip to the inner)
            //   field   — `OpGetField(<base>, fld)`, the record at the field's offset.
            // Bind `c`/`r` to that ref; reads/writes deref it the same as L1/L4.
            // tuples.md T-Ref — the source of a `&(…)` link with a heap element must be a
            // record; say so for pass 2's bind (see `ref_linked_tuple_locals`).
            if let Some(src) = stack_src
                && let Type::Tuple(elems) = self.vars.tp(src)
                && elems.iter().any(|e| !is_scalar(e.base()))
                && elems.iter().all(crate::data::ref_tuple_record_element_ok)
            {
                let name = self.vars.name(src).to_string();
                self.ref_linked_tuple_locals.insert((self.context, name));
            }
            let heap_ref = if stack_src.is_none() && is_scalar(&s_type) {
                match code.unspan() {
                    Value::Call(g, gargs) if self.data.def(*g).name().starts_with("OpGet") => {
                        if gargs.first().is_some_and(|a| {
                            matches!(a.unspan(), Value::Call(d, _)
                                if matches!(self.data.def(*d).name(), "OpGetVector" | "OpVectorRef"))
                        }) {
                            Some(gargs[0].clone())
                        } else if let [base, fld] = gargs.as_slice() {
                            Some(self.cl("OpGetField", &[base.clone(), fld.clone()]))
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
            } else {
                None
            };
            if let Some(src) = stack_src {
                amp_unlowered = false;
                let mut inner = self.vars.tp(src).clone();
                // tuples.md T-Ref — a linked tuple local with a heap element is the
                // `__tuple<…>` record.  On pass 1 its bind has not been rewritten yet (the fact
                // was recorded a moment ago), so derive the link's type from the record the
                // bind WILL build, or an annotated `c: &(…) = a` disagrees with itself.
                if let Type::Tuple(elems) = &inner
                    && elems.iter().any(|e| !is_scalar(e.base()))
                    && elems.iter().all(crate::data::ref_tuple_record_element_ok)
                {
                    let elems = elems.clone();
                    let d = self.data.tuple_def(&mut self.lexer, &elems);
                    if d != u32::MAX {
                        inner = Type::Reference(d, Deps::none());
                    }
                }
                // Every HEAP inner — a record, a text buffer, a vector — makes the link
                // NON-OWNING: the borrow fact rides `deps` (@FR-O-Borrow), so the source
                // frees its store and the link frees nothing.  A scalar inner carries no
                // `Deps` slot and owns no store, so there is no free decision to derive.
                let is_ref = matches!(inner, Type::Reference(..) | Type::Text(_));
                *code = self.cl("OpCreateStack", &[Value::Var(src)]);
                // @PLN85 D-own-5 — the borrow fact rides `deps` (O-Borrow), not a
                // side-flag: a heap whole-value alias (`p = &o`, L5) is NON-OWNING
                // because its type deps name the source, and the free suppression
                // derives from `owns = dep.is_empty()` (`scopes::get_free_vars`) —
                // the same read every other borrow uses (this replaced the
                // `set_skip_free(var_nr)` side-channel).  A scalar inner carries
                // no `Deps` slot (`depending` is the identity there), which is
                // consistent: a scalar `&` binder owns no store, so there is no
                // free decision to derive.
                let linked = if is_ref { inner.depending(src) } else { inner };
                // An ANNOTATED binding was already gated when the annotation was parsed —
                // and that is the site that has to hold it, because a `&(…) = <not a
                // variable>` never reaches this lowering at all.  Here the gate covers the
                // other spelling, `b = &a`, whose element types nothing has looked at yet.
                // …and a declined annotation (`r: &integer? = x`) left the binding PLAIN, so
                // the seam keeps it plain too rather than reporting the same link twice.
                s_type = if var_nr != u16::MAX && self.vars.is_annotated(var_nr) {
                    if matches!(self.vars.tp(var_nr), Type::RefVar(_)) {
                        Type::RefVar(Box::new(linked))
                    } else {
                        linked
                    }
                } else {
                    self.ref_var_type(linked)
                };
            } else if let Some(eref) = heap_ref {
                amp_unlowered = false;
                // `c`/`r` holds the field/element DbRef; interp reads/writes it via the
                // uniform RefVar deref (`OpGet*/OpSet*(c,0)`), and native keys its
                // pointer construction off this `OpGetField`/`OpGetVector` value — so no
                // per-variable flag is needed and the link survives an IR snapshot.
                // loft#1372 — a NULLABLE scalar place links like its non-null twin:
                // `Optional(τ)` shares `τ`'s storage, so the same `OpGet*/OpSet*(c, 0)`
                // deref serves it and the null rides the slot's own sentinel.  This site
                // used to decline it (D-bind-17).
                *code = eref;
                s_type = Type::RefVar(Box::new(s_type));
            }
        }
        // A `&` of a tuple PLACE (`b = &v[0]`, `b = &s.pair`) reaches no lowering above,
        // and unlike the struct projection below it cannot be left alone: a tuple place is
        // read ELEMENT-WISE into a fresh by-value tuple before the `&` is ever seen, so
        // there is no place left to link to.  Declining is what @FR-B-Ref-Reshape
        // prescribes where the link cannot be honoured — *"loft will not quietly downgrade
        // the reference to a copy"*.
        //
        // ⚠ The alternative is not a lesser `&`, it is a SILENT one: downgrading makes
        // `b.0 = 9` write the copy while the source stands, with no diagnostic, and both
        // backends agree — so the differential oracle cannot see it either (D-tup-2).
        if amp_unlowered
            && !self.first_pass
            && matches!(
                s_type.base(),
                Type::Tuple(_) | Type::RefVar(_) if matches!(
                    if let Type::RefVar(ref i) = *s_type.base() { i.base() } else { s_type.base() },
                    Type::Tuple(_)
                )
            )
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "a `&` reference to a tuple ELEMENT or FIELD is not a live link — a tuple \
                 place is read element by element, so there is nothing left to point at. \
                 Bind the tuple to a local first and take `&` of that, or write the \
                 element back explicitly"
            );
        }
        // @PLN130 F9 step 2 — the `&` reached no lowering, so record it on the VARIABLE:
        // the IR is about to lose it entirely.  A marker rather than `Type::RefVar` on
        // purpose — RefVar would re-route every read and write through the double
        // indirection parameters use, paying on every access to carry a compile-time
        // fact, which is exactly what loft's own advice warns about for a redundant `&`
        // param.  INERT: nothing reads it yet (step 3, the refusal, is what will).
        if amp_unlowered
            && var_nr != u16::MAX
            && matches!(s_type, Type::Reference(..) | Type::Enum(_, true, _))
        {
            self.vars.set_amp_link(var_nr);
        }
        // `amp_pending` is a one-shot per binding — clear it here so a `&` that did
        // NOT take the scalar-reference path (a heap `&`-view, a non-`Var` source, an
        // already-reported error) cannot leak the flag into the NEXT statement and
        // wrongly turn `y = scalarvar` into a reference.
        self.amp_pending = false;
        // @PLN85 poison-green — a fn-ref FIELD READ bound to a local
        // (`c = k.cb`) is a BORROWED fn-ref: its closure half points at the
        // base struct's child record, OWNED by the struct.  Without the mark,
        // `get_free_vars`' Function arm frees the closure through the alias at
        // scope exit ("fn-ref variables OWN their closure store") — freeing
        // the CALLER's record; the next `k.cb` read hit poisoned memory
        // (d_nr = 0xDEADBEEF, the issue_313 cross-fn shape).  `skip_free` is
        // the established borrowed-fn-ref carrier (the vector-element read
        // path marks its `__fn_ref_tmp` the same way).
        if op == "="
            && var_nr != u16::MAX
            && matches!(&s_type, Type::Function(_, _, _))
            && matches!(code, Value::Block(b) if b.name == "fn_ref_field_read")
        {
            self.vars.set_skip_free(var_nr);
        }
        if amp_vector_replace {
            let clear = self.cl("OpClearVector", &[Value::Var(var_nr)]);
            *code = Value::Insert(vec![clear, code.clone()]);
        }
        // @P376 — POISON an errored whole-RHS that resolves to `Unknown` (pass 2)
        // so the assigned variable doesn't cascade.  In the final pass an Unknown
        // whole-RHS is always an unresolved-name error — an undefined variable
        // (`p = qqq`), an undefined function (`p = nofn(1)`), or an unknown
        // struct (`p = Plyer {…}`, already `Never` via the construction).  The
        // root error is reported (by `known_var_or_type` / the call resolver /
        // the type lookup); leaving the variable `Unknown` makes every downstream
        // use (`p.name`, the `{p.name}` format string, the post-parse sweep)
        // re-report — an 8-9 error pileup ending in a format-string fatal.
        // `Never` makes it the silent poison.  Pass 1 defers, and a forward ref
        // resolves to a concrete type in pass 2, so its RHS is not `Unknown`
        // there; a partly-bad RHS that still types (`p = qqq + 1` → integer) is
        // not unknown, so it is untouched.
        let poison = !self.first_pass && s_type.is_unknown();
        // Check the RHS for unresolved variables — but NOT when `code` IS the
        // assignment target: a failed struct construction / call leaves its
        // discarded target `Var(var_nr)` as `code`, and re-validating it reports
        // the spurious "Unknown variable '<target>'" (the cascade tail).  A
        // genuine RHS variable (`p = qqq`) has `code != Var(var_nr)`, so its real
        // "Unknown variable 'qqq'" still fires.
        let code_is_target = matches!(code, Value::Var(c) if *c == var_nr);
        let skip_rhs_check = matches!(s_type, Type::Never) || (poison && code_is_target);
        if !skip_rhs_check {
            self.known_var_or_type(code, &rhs_pos);
        }
        if poison {
            s_type = Type::Never;
        }
        if let Type::Rewritten(tp) = s_type {
            s_type = *tp;
        }
        // Dead assignment check: after the RHS is parsed (so RHS reads of the
        // variable are already counted), check if the previous write was never read.
        if op == "=" && var_nr != u16::MAX && !self.first_pass && self.vars.exists(var_nr) {
            self.vars.track_write(var_nr, &mut self.lexer);
        }
        // Convert untyped null to typed null for scalar assignments (not collections).
        // `is_dbref`, not a spelled list: the list this used to carry named six heap kinds
        // and not `spatial` or `trie`, so `g.sp = null` took the SCALAR sentinel path —
        // found when @FR-N-Store's one home started asking here (the drifted-deny-list
        // shape QUALITY.md § Design P8 records).  The store face asks the bare-null half
        // for the scalar slot; a heap slot's `= null` is its clear, asked where it lowers.
        if s_type == Type::Null && op == "=" && !crate::data::is_dbref(f_type) {
            self.convert_store(code, &Type::Null, f_type, "the assignment target", None);
        }
        if var_nr == u16::MAX && !skip_validate {
            // Use the LHS target's parent type saved BEFORE the RHS parse — the RHS
            // parse above overwrites `parent_tp` (to the RHS's last field-access type),
            // which made a write like `arr[i] = … + e.const_field` falsely resolve to
            // the RHS struct and flag its const/key field (@PLN40 false positive).
            // `skip_validate` is set when the F2 place-once hoist already validated the
            // ORIGINAL place (the hoist rewrites `to` to a temp, hiding its const base).
            self.validate_write(to, &lhs_parent_tp, op);
        }
        // materialise a collection iterator (e.g. v[a..b] slice) into a vector
        // variable.  CO1.3c: the original "second type Null = coroutine, skip"
        // heuristic was wrong — `parse_vector_index` also returns
        // `Iterator<T, Null>` for slices, and slice → vector is exactly the
        // case we want to materialise.  Instead detect coroutine iters
        // structurally: a coroutine `yield`-driven iter does NOT carry a
        // `Value::Iter(_, _, Block(_), _)` shape — its `next` is the
        // resume-call.  Materialise only when the `next` is a Block (the
        // slice / range / collection-iteration shape).  (@P287)
        // Materialisable iter detection — by IR shape, NOT by s_type.  The
        // `s_type` is the type the RHS *evaluates to*, but parse_operators
        // may silently convert `Iterator<T>` to `Vector<T>` when the LHS
        // expects a vector (`+=` case especially) — leaving `code` as the
        // raw iter IR even though s_type now claims Vector.  We trust the
        // IR shape: a `Value::Iter` with a `Block` next is the
        // slice / range / collection-iter shape we can materialise into a
        // vector via the per-element record-allocator loop.  Coroutine
        // iters have a different `next` shape (resume-call) and don't
        // match.  (@P287)
        let materialisable_iter_shape = matches!(code,
            Value::Iter(_, _, n, _) if matches!(n.as_ref(), Value::Block(_)));
        // Recover the element type from either s_type or the iter's annotation.
        let iter_elm_tp: Option<Type> = if let Type::Iterator(elm, _) = &s_type {
            Some((**elm).clone())
        } else if let Type::Vector(elm, _) = &s_type {
            // s_type was silently converted; pull element type from there.
            Some((**elm).clone())
        } else {
            None
        };
        // @P390 — `v = v[a..b]` self-slice-assign.  The plain materialise emits
        // `OpClearVector(var)` BEFORE the loop reads `v[i]` from the SAME record,
        // so every element reads back null (length preserved, values lost).  When
        // the slice source IS this variable, route through a temp local first (the
        // proven @P287 struct-field pattern), then `OpClearVector + OpAppendVector`
        // on the destination — the temp breaks the alias.  Scoped to TRUE aliasing
        // (`ir_mentions_var`) so the non-aliased `t = v[a..b]` keeps the direct
        // fast path; `+=` never clears, so it stays on the direct path below too.
        if materialisable_iter_shape
            && let Some(elm_tp) = iter_elm_tp.clone()
            && matches!(f_type, Type::Unknown(_) | Type::Vector(_, _))
            && var_nr != u16::MAX
            && op == "="
            && !self.first_pass
            && ir_mentions_var(code, var_nr)
        {
            let iter_tp = Type::Iterator(Box::new(elm_tp.clone()), Box::new(Type::Null));
            let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
            let tmp = self.create_unique("__p390_tmp", &vec_tp);
            self.vars.defined(tmp);
            // (1) materialise the slice iterator into the fresh temp (reads the
            //     source — still intact — and appends to tmp; tmp != source).
            self.materialize_iterator(code, &iter_tp, &Value::Var(tmp), &lhs_parent_tp, tmp, "=");
            // (2) clear the destination and append the temp's contents.
            let dn = self.data.type_def_nr(&elm_tp);
            let rec_tp = Value::Int(i32::from(self.data.def(dn).known_type()));
            let clear = self.cl("OpClearVector", &[Value::Var(var_nr)]);
            let append = self.cl(
                "OpAppendVector",
                &[Value::Var(var_nr), Value::Var(tmp), rec_tp],
            );
            *code = Value::Insert(vec![code.clone(), clear, append]);
            return Type::Void;
        }
        if materialisable_iter_shape
            && let Some(elm_tp) = iter_elm_tp.clone()
            && matches!(f_type, Type::Unknown(_) | Type::Vector(_, _))
            && var_nr != u16::MAX
            && matches!(op, "=" | "+=")
        {
            // Rebuild a real Iterator type so materialize_iterator's destructure works.
            let iter_tp = Type::Iterator(Box::new(elm_tp), Box::new(Type::Null));
            self.materialize_iterator(code, &iter_tp, to, &lhs_parent_tp, var_nr, op);
            return Type::Void;
        }
        // #410 — a DIRECT `#native`-decl call returning a vector delivers a
        // FOREIGN-store value: the FFI bridge wraps the return into the
        // return-only null store (`extensions.rs::bridge_push_ref`).  Bound
        // to a local with a plain `Set`, the local BORROWS that foreign
        // store and never gets its own `__vdb` buffer — so the first
        // in-place `+=` runs `vector_db` (build_vector_list /
        // operators.rs), which allocates a FRESH EMPTY buffer and repoints
        // the local, silently DROPPING the returned elements (len 4 → 1).
        // #409 fixed the loft-WRAPPER shape at the return site; a direct
        // call has no wrapper, so materialise HERE at the assignment: mint
        // the local's own `vector_db` buffer and COPY the foreign return
        // into it (the clear+append delivery the @P390 / @P287 paths use),
        // so by the `+=` the local is shaped like any owned vector and
        // appends in place.  A `#native` decl is `code()==Null &&
        // !native().is_empty()`; an empty `native()` is a builtin op (e.g.
        // `split`), whose vector return is already owned (data.rs::
        // native_symbol_collisions draws the same line).
        let native_vec_elm: Option<Type> = if !self.first_pass
            && op == "="
            && var_nr != u16::MAX
            && !self.vars.is_argument(var_nr)
            && matches!(f_type, Type::Unknown(_) | Type::Vector(_, _))
        {
            match code.unspan() {
                Value::Call(d, _) => {
                    let def = self.data.def(*d);
                    if *def.code() == Value::Null
                        && !def.native().is_empty()
                        && let Type::Vector(elm, _) = def.returned()
                    {
                        Some((**elm).clone())
                    } else {
                        None
                    }
                }
                _ => None,
            }
        } else {
            None
        };
        if let Some(elm_tp) = native_vec_elm {
            let rec_tp = self.append_elem_tp(&elm_tp);
            // Route the foreign return through a named local `__fwd` (the
            // #409 shape, control.rs::fwd_copy_409) — NOT the call inline.
            // A named local is what scope analysis recognises as an owned
            // temporary and frees (`OpFreeRef(__fwd)`); appending the call
            // inline copies the elements but orphans the foreign source
            // store (a per-assignment leak, ×N in a loop).
            let fwd = self.create_unique(
                "__fwd",
                &Type::Vector(Box::new(elm_tp.clone()), Deps::none()),
            );
            self.vars.defined(fwd);
            let set_fwd = v_set(fwd, code.clone());
            // `vector_db` takes the ELEMENT type, mints a fresh owned store
            // for the local (OpDatabase + `Set(v, field)` + length 0) and
            // records the `__vdb` dep, so the later `+=` skips its own
            // (destructive) `vector_db`.
            let mut ls = vec![set_fwd];
            ls.extend(self.vector_db(&elm_tp, var_nr));
            ls.push(self.cl(
                "OpAppendVector",
                &[Value::Var(var_nr), Value::Var(fwd), Value::Int(rec_tp)],
            ));
            *code = Value::Insert(ls);
            return Type::Void;
        }
        // @P287 — same materialisation, but the LHS is a struct field
        // (`s.v = slice`) so var_nr is u16::MAX.  Three-step lower:
        //  (1) allocate a temp local vector,
        //  (2) materialise the iterator into the temp via the existing
        //      `materialize_iterator` helper,
        //  (3) emit `OpClearVector(field) + OpAppendVector(field, tmp)` —
        //      the same shape that lines ~1020-1033 below build for
        //      `s.v = fresh_vec` whole-vector field-replace.
        if materialisable_iter_shape
            && let Some(elm_tp) = iter_elm_tp
            && matches!(f_type, Type::Vector(_, _))
            && var_nr == u16::MAX
            && op == "="
            && !self.first_pass
            && self.is_field(to)
        {
            let iter_tp = Type::Iterator(Box::new(elm_tp.clone()), Box::new(Type::Null));
            let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
            let tmp = self.create_unique("__p287_tmp", &vec_tp);
            self.vars.defined(tmp);
            // (2) materialise iter → tmp (mutates *code into the materialise IR).
            //     Pass the rebuilt iter_tp so materialize_iterator's destructure
            //     succeeds even when s_type was silently converted upstream.
            self.materialize_iterator(code, &iter_tp, &Value::Var(tmp), &lhs_parent_tp, tmp, op);
            // (3) emit clear + append on the destination field.
            let dn = self.data.type_def_nr(&elm_tp);
            let rec_tp = Value::Int(i32::from(self.data.def(dn).known_type()));
            let clear = self.cl("OpClearVector", std::slice::from_ref(to));
            let append = self.cl("OpAppendVector", &[to.clone(), Value::Var(tmp), rec_tp]);
            *code = Value::Insert(vec![code.clone(), clear, append]);
            return Type::Void;
        }
        // P188 — local-var collection `+= elem` for keyed collections
        // (sorted/hash/index/spatial) ONLY.  Routes the singleton element
        // through OpNewRecord + OpFinishRecord (per-kind dispatch via
        // record_finish: hash::add / sorted_finish / tree::add /
        // ordered_finish).  Returns Type::Void before change_var fires
        // the "cannot change type from sorted<…> to T" diagnostic that
        // would otherwise reject this shape.
        //
        // @PLAN52 cluster IV-Vec-nested-field-push (2026-05-30): Vector
        // LHS REMOVED from this branch.  Vector `+= elem` is ambiguous
        // with concat for nested-vector cases (`vec<vec<T>> += vec<T>` —
        // bare element type matches both element_type AND a sub-shape of
        // the LHS type).  Strict rule: vector push MUST use `+= [elem]`
        // (explicit brackets).  Falls through to the diagnostic below
        // when the RHS doesn't match the concat shape.
        if op == "+=" && var_nr != u16::MAX && crate::parser::vectors::is_keyed(f_type) {
            let elm_tp = f_type.content();
            if !elm_tp.is_unknown() && elm_tp.is_equal(&s_type) {
                if !self.first_pass {
                    let elm = self.unique_elm_var(f_type, &elm_tp, var_nr);
                    let mut scalar = code.clone();
                    // Mirror of the field-+= retarget at line ~755:
                    // a struct-literal RHS pre-parses with the LHS local
                    // (`Var(var_nr)`) as its target.  After allocating a
                    // fresh element via new_record, retarget the inits
                    // onto `Var(elm)` so the writes land in the new
                    // record instead of the local-var's storage.
                    substitute_value(&mut scalar, &Value::Var(var_nr), &Value::Var(elm));
                    let ls = self.new_record(
                        &mut Value::Var(var_nr),
                        f_type,
                        elm,
                        var_nr,
                        &[scalar],
                        &elm_tp,
                    );
                    *code = Value::Insert(ls);
                }
                return Type::Void;
            }
        }
        // @PLAN52 cluster IV-Vec-nested-field-push (2026-05-30): strict
        // rule — VECTOR `+= elem` (bare element) is rejected.  Use
        // `+= [elem]` to push a single element, or `+= other_vec`
        // (typeof must equal LHS) to concatenate.  This eliminates the
        // ambiguity class where `vec<vec<T>> += vec<T>` is BOTH
        // "push one element of element-type" AND syntactically resembles
        // "concat" (RHS is vector-typed) — the parser branch order used
        // to misroute the latter.
        //
        // @PLN25 — matched on the target's STORAGE, so a `vector<T>?` is refused the same
        // way and gets the same cure.  Unpeeled, the ambiguity was not recognised on a
        // nullable vector and the reader met whatever the fall-through said instead —
        // *"Variable 'v' cannot change type from vector<integer>? to integer"* for a
        // local, *"No matching operator 'Add'"* for a field, neither of which mentions
        // `+= [elem]`.
        // loft#1221 — "is the source ONE element of this vector" is asked of the same home the
        // append routes ask (`holds_element`), because the element type has more than one
        // spelling and a bare `is_equal` knows only the first.  @FR-C-Var makes a VARIANT an
        // element of a vector over its enum, so `b.items += Named { … }` is a bare element
        // append and owes the reader this message.  Asked with `is_equal` it was none of the
        // routes' business: `Reference(Named)` and `Enum(Tagged, …)` read as unrelated, the
        // statement fell past every branch, and the generic path grew the vector by THREE.
        // loft#1223 — the SOURCE is read through `.base()` for exactly the reason @PLN25 gave
        // for the destination one paragraph up: the rule is a blanket requirement on the
        // SPELLING, and `τ?` occupies τ's storage plus one reserved null, so nullability does
        // not decide whether a bare element is ambiguous with a concat.  Asked unpeeled, the
        // `?` spelling of one statement was MORE permissive than the plain one — `d.c += n`
        // with `n: integer?` was accepted where the dense `d.c += i` is refused, which is
        // backwards for a nullability marker.  It reached the vector single-element push, and
        // was the ONLY shape in the corpus that did.
        //
        // The two questions stay separate and both still reach the reader: this one says WRITE
        // THE BRACKETS, and `(N-Store)` below says the value may be null where a non-null is
        // expected.  The cure named here (`+= [n]`) earns that warning on its own.
        if op == "+="
            && let Type::Vector(_, _) = f_type.base()
            && !s_type.is_unknown()
            && {
                let content = f_type.base().content();
                let src = s_type.base().clone();
                self.holds_element(&content, &src)
            }
            && !s_type.base().is_equal(f_type.base())
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "vector `+= elem` is ambiguous; use `+= [elem]` to push one \
                 element, or `+= other_vec` (typeof must match) to concatenate"
            );
            *code = Value::Insert(Vec::new());
            return Type::Void;
        }
        // @PLN25 (N-Store), loft#1210 — an UN-DISCHARGED nullable source appended to a
        // non-null collection.  `(N-Store)` rejects storing `e:τ?` into a `τ` slot, and the
        // rule is already enforced one place-kind over: the same store into a dense LOCAL is
        // refused with a message naming both cures.  A FIELD destination asked nobody, so the
        // value was written raw — the interpreter panicked writing a read-only const store and
        // `--native` emitted `set_int(…, v)` with a `DbRef` for `v`, neither of which describes
        // the statement that caused it.  A keyed field took the null quietly instead.
        //
        // Placed here because it is the ONE point where both destinations are still in play:
        // the vector-push, the vector-concat and the keyed-fill routes are all downstream, so a
        // check at any of them is a third copy of the same question.  Discharging is what the
        // reader is told to do (`c += s?` / `c += s ?? []`), and `n_store_violation` names it.
        //
        // The BASE has to be acceptable for this to be the null's fault rather than an
        // ordinary type error: a `text?` appended to a `vector<integer>` is a mismatch that
        // discharging does not fix, and it keeps the plain message it already had.
        let nullable_append_source = op == "+="
            && !self.first_pass
            && !s_type.is_unknown()
            && matches!(&s_type, Type::Optional(_))
            && crate::parser::vectors::is_collection(f_type.base())
            && {
                let base = s_type.base().clone();
                base.is_equal(f_type.base())
                    || matches!(f_type.base(), Type::Vector(elm, _) if (**elm).is_equal(&base))
                    || matches!(&base, Type::Vector(elm, _)
                        if crate::parser::vectors::is_keyed(f_type.base())
                            && (**elm).is_equal(&f_type.base().content()))
            };
        if nullable_append_source {
            // The rule decides the severity, and it is not a refusal.  `(N-Store)`'s split is
            // REPRESENTABILITY: a hard error only where the null sentinel collides with a real
            // value of τ — the narrow widths, and nothing else — and a warning everywhere the
            // null is representable-and-distinct, where the store COMPILES AND RUNS.  A
            // collection is stored out-of-band (`nullref`), so it is on the warning side, and
            // `d.c = s` one operator over already warns and proceeds.
            if self.n_store_violation(&s_type, f_type.base(), "the appended value", None) {
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
            // So make it proceed.  The append routes below read `s_type` to choose between
            // concat, single-element push and the keyed fill, and an `Optional` matched none of
            // them: the value fell to the push, which writes whatever it is given as ONE
            // element of the element type.  A `vector<integer>?` was written where an `integer`
            // belongs — the interpreter panicked into a read-only const store and `--native`
            // emitted `set_int(…, v)` with a `DbRef` for `v`, neither naming the statement.  A
            // keyed destination took it quietly.  Peeling here is what `convert` already does
            // for `=`, so both operators reach the same reading of the same value.
            let peeled = s_type.base().clone();
            // Asked just above; this peel is the store proceeding, not a second store.
            self.convert_admitting(code, &s_type, &peeled);
            s_type = peeled;
        }
        // loft#1215 — the append routes below are a partial list, and nothing said so.  A
        // source that matches none of them used to reach whichever route tests LEAST: the
        // vector single-element push, which compares its source with the element type
        // nowhere and writes whatever it is handed as one element.  A `float` came back as
        // its IEEE-754 bits read as an `i64`, a `boolean` as 8705, a `text` panicked the
        // allocator, a struct source and a `vector<text>` element both ended in a SIGSEGV,
        // and `--native` refused to compile any of them — so the two backends disagreed
        // about the same program.  A KEYED destination has no catch-all route, so the same
        // source fell past everything to a statement that emitted no write at all and the
        // append vanished with `len` reading 0.
        //
        // @FR-C-Only settles it without a design call: `⤳` is the only implicit coercion, and
        // no `⤳` relates a `float` to an `integer` slot.  So this is an ordinary type
        // error, and the reference route is one operator over — `d.c = f` has always been
        // one.
        //
        // Placed at the ONE point where every destination kind and every route are still in
        // play, beside `(N-Store)`'s check for the same reason: the vector push, the vector
        // concat, the keyed fill and the record-literal routes are all downstream, so a
        // check at any of them is one more copy of a question this file already asks in
        // four places.  [`Parser::append_source`] is that question's home.
        // ⚠ A bare `null` source is NOT the classifier's question and is excluded here.
        // `null` is not a type the destination can or cannot HOLD — it is a value every
        // element slot has a reading for, and the routes below have handled it since long
        // before this check: `c += null` appends one absent element, at a nullable element
        // type and at a dense one alike.  Whether it may occupy a DENSE slot is `(N-Store)`'s
        // question and belongs to loft#1232, not to a routing refusal.
        //
        // Measured the hard way: without this clause the classifier called `null` `Unrelated`
        // at every element type and refused four shapes the parent accepts — which broke the
        // published `arguments 0.2.1` on `self.results += null` into a `vector<text?>`.  Five
        // green `make ci` runs and a whole-corpus value differential said nothing, because no
        // `.loft` file in this repository appends a bare `null` to a collection; only
        // `scripts/revalidate_libs_local.sh` sees it.
        if op == "+="
            && !self.first_pass
            && !matches!(s_type, Type::Null)
            && crate::parser::vectors::is_collection(f_type.base())
        {
            let dest = f_type.base().clone();
            let kind = self.append_source(&dest, &s_type);
            // A keyed destination has no route for the WHOLE collection at any place kind, and
            // the two place kinds fail differently — which is why neither one alone settles it.
            // A keyed FIELD emitted no write: `d.h += other_h` left `len` reading 0 in silence.
            // Between two keyed LOCALS the statement is claimed, and what it does is REBIND:
            // the IR is a plain `b = a`, so `b` is repointed at `a`'s store and takes a dep on
            // it.  From an EMPTY destination that reads exactly like a successful merge; from a
            // POPULATED one the destination's own records are simply gone — measured,
            // `d[1] = …; d += c` leaves `d[1]` ABSENT and `d[9]` (the source's) present — and
            // mutating the source afterwards moves the destination, which is the cell that
            // shows the alias rather than either appearance.  Its own `= []` store is orphaned
            // (`1 stores not freed`).
            //
            // So there is no merge here to preserve.  @FR-Col-Insert is stated over records
            // joining a collection and says nothing about merging two; no rule admits an
            // aliasing rebind either, so refusing is the rule-consistent answer at every place
            // kind, and implementing a real merge is a design call and a new op.
            //
            // ⚠ This deliberately does NOT carry a `var_nr == u16::MAX` clause, and a build that
            // adds one has been told the LOCAL spelling works.  It does not: it looks correct
            // only from an empty destination, and no `.loft` file in the tree merges two keyed
            // locals, so a whole-corpus differential runs clean over the difference.
            //
            // The vector twin is the control — there `Whole` IS concatenation at every place
            // kind, it copies rather than rebinds, and it must stay.
            //
            // ...and only when the source NAMES a collection.  `append_source` answers with
            // TYPES, and a keyed literal is built THROUGH its destination (loft#703), so
            // `t.0 += [E { … }]` at a TUPLE ELEMENT reports the destination's own type and
            // reads as `Whole`.  That is a one-element append, not a merge — the three place
            // kinds above never reach here because they parse the literal per element first,
            // and the tuple element does because it builds through a `__kvb_N` accumulator.
            // A merge can only be written with a source that names an existing collection.
            let unroutable_whole = kind == crate::parser::vectors::AppendSource::Whole
                && crate::parser::vectors::is_keyed(&dest)
                && !s_type.is_unknown()
                && self.source_names_a_collection(code);
            if unroutable_whole {
                let content = dest.content().name(&self.data);
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot append a whole `{}` to another — a keyed collection's `+=` takes \
                     one `{}` element written `[…]`, or a `vector<{}>` of them",
                    dest.source_name(&self.data),
                    content,
                    content
                );
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
            if kind == crate::parser::vectors::AppendSource::Unrelated {
                // The two spellings named are the two that were MEASURED to work at every
                // destination kind: `+= [elem]` and `+= <vector of elem>`.  For a vector the
                // second IS the collection, so one sentence serves both kinds.  What is
                // deliberately NOT offered is "the whole collection, to concatenate" at a
                // KEYED destination: `d.h += other_h` between two `hash<E[k]>` is itself a
                // silent drop, and a refusal whose cure is broken sends the reader to a dead
                // end — that one is loft#1221, the routes that drop an ADMISSIBLE source.
                let content = dest.content().name(&self.data);
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot append `{}` to `{}` — a `+=` source must be one `{}` element \
                     written `[…]`, or a `vector<{}>` of them",
                    s_type.name(&self.data),
                    dest.source_name(&self.data),
                    content,
                    content
                );
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
        }
        // C54.A incremental 2a — if the variable carries an annotated
        // target type `: Long` with a narrower `Integer` RHS
        // (e.g. `x: u32 = 100` where `u32` promoted to Long), run
        // `convert()` BEFORE `change_var` fires the "cannot change
        // type" diagnostic.  Narrowly scoped to Integer → Long so
        // other cross-category conversions (e.g. Enum → Integer via
        // OpConvIntFromEnum, which would silently pass but IS a real
        // type mismatch) still fall through to the existing error
        // path.  Similar widens for single→float etc. stay handled
        // by the later convert at op == "=".
        // Post-2c round 10c: Type::Long was folded into Type::Integer;
        // any Integer↔Integer assignment is a no-op and needs no early widen.
        let _ = op;
        let _ = f_type;
        // @PLN25 single-payload — a dense-`S` target assigned a `__nullable<S>` value
        // (`d: S = v[i]`): unwrap the RHS via `convert` (→ a payload sub-ref `OpGetField`)
        // BEFORE `change_var`, then retype it as dense `S` so the var-type-change
        // check accepts (d:S ← S) instead of erroring "cannot change type from S to
        // __nullable<S>".  Mirrors the C54.A early-convert pattern above; NOT
        // pass-2-gated because the type error fires on pass 1 too (`convert`
        // self-guards emission to pass 2 and returns true on both).  Gate-off-inert.
        let nullable_to_dense_assign = op == "="
            && matches!(f_type, Type::Reference(struct_d, _)
                if matches!(&s_type, Type::Enum(syn, true, _)
                    if self.data.def(*syn).name
                        == format!("__nullable<{}>", self.data.def(*struct_d).name())));
        if nullable_to_dense_assign && self.convert(code, &s_type, f_type) {
            s_type = f_type.clone();
            // The unwrap (`convert`) produced a payload sub-ref `OpGetField(src, payload)`
            // — a VIEW into the nullable source's store. Assigning that view to an OWNING
            // dense Reference local aliases it; if the local is then returned the view
            // dangles once the source (e.g. a local `pool`) is freed (#462). Deep-COPY the
            // unwrapped payload into a fresh owned store so the target keeps value
            // semantics — it stays OWNED (so #316's pre-Set free of its prior store still
            // fires) AND owns an independent copy (so the return is not a dangling view).
            // This makes the imperative `chosen = pool[i] ?? mk()` reassign match the
            // already-correct if-EXPRESSION form. The work-ref is `skip_free` (the target
            // adopts + solely owns the copy), so there is no double-free.
            if !self.first_pass
                && let Type::Reference(td, _) = f_type
            {
                let td = *td;
                let kt = self.data.def(td).known_type();
                let w = self.vars.work_refs(
                    &Type::Reference(td, crate::data::Deps::none()),
                    &mut self.lexer,
                );
                if w != u16::MAX {
                    self.vars.set_skip_free(w);
                    let copy_d = self.data.def_nr("OpCopyRecord");
                    let orig = std::mem::replace(code, Value::Null);
                    *code = crate::data::v_block(
                        vec![
                            crate::data::v_set(w, Value::Null),
                            self.cl("OpDatabase", &[Value::Var(w), Value::Int(i32::from(kt))]),
                            Value::Call(
                                copy_d,
                                vec![orig, Value::Var(w), Value::Int(i32::from(kt))],
                            ),
                            Value::Var(w),
                        ],
                        Type::Reference(td, crate::data::Deps::none()),
                        "nullable_unwrap_copy",
                    );
                    // @PLN130 — parser-emitted materialisation; see `ParserMaterialise`.
                    crate::copy_manifest::record(
                        self.context,
                        w,
                        kt,
                        crate::copy_manifest::Origin::ParserMaterialise,
                    );
                    // The copy is what makes the target INDEPENDENT, so it must be typed as
                    // an owner.  Pass 1 typed the target off the un-copied expression — the
                    // nullable source is a VIEW of its holder, so the deps named the holder —
                    // and inheriting them here left the fresh store with nobody to free it:
                    // one leaked record per evaluation, on both backends.  The work-ref is
                    // `skip_free` precisely so this variable is the single owner.
                    //
                    // Stripped on the VARIABLE, not just on `s_type`: pass 1 already wrote
                    // the borrowing type into the frame, and `change_var_type` treats a deps
                    // difference as no change at all, so re-assigning the type is a no-op.
                    //
                    // ⚠ The dense `Reference` target is a PRECONDITION of the strip working, not
                    // an incidental fact about this arm.  `Type::depend` peels `Optional` /
                    // `RefVar` and reads a `Text` dep, and `Function::make_independent` — the
                    // CLEAR half — spells its own arm list with neither wrapper and without
                    // `Text`, so on those the read answers deps the clear then silently cannot
                    // remove.  `nullable_to_dense_assign` only fires with `f_type` a dense
                    // `Reference`, which is inside the arms it does list.
                    s_type = Type::Reference(td, crate::data::Deps::none());
                    if var_nr != u16::MAX {
                        for d in self.vars.tp(var_nr).depend() {
                            self.vars.make_independent(var_nr, d);
                        }
                    }
                }
            }
        }
        // loft#822 — a declared STACK-tuple target assigned the STORED spelling of the
        // same tuple: `t: (integer, text) = make_pair(3)`, where the heap-carrying return
        // is a `Reference(__tuple<…>)`.  Unbox through `convert` BEFORE `change_var`, then
        // retype so the var-type-change check compares the tuple with itself instead of
        // erroring "cannot change type from (integer, text) to __tuple<integer,text>".
        // Same early-convert shape as the nullable-to-dense assign above.
        // tuples.md T-Ref — a linked tuple local IS the record now; pass 1 typed it as the
        // stack tuple, and unboxing the record back to that would undo the representation the
        // link needs.  `f_type` is pass 1's answer for it.
        let keeps_record = var_nr != u16::MAX
            && self
                .ref_linked_tuple_locals
                .contains(&(self.context, self.vars.name(var_nr).to_string()));
        if op == "="
            && !keeps_record
            && self.unboxes_stored_tuple(&s_type, f_type)
            && self.convert(code, &s_type, f_type)
        {
            s_type = f_type.clone();
        }
        // @PLN85 cluster V — a vector `+=` is an IN-PLACE append: `buf` keeps its OWN
        // backing store; the appended source is COPIED in and consumed.  `change_var`
        // below copies the RHS type's deps onto `buf`, so for `+=` it re-points `buf`
        // onto the LAST appended source (e.g. a `["??"]`-returning call's hidden
        // `__ref_N`).  That mis-dep then collides with NRVO promotion: ref_return
        // promotes `__ref_N` to `__retbuf`, but `__ref_N` is ALSO that append call's
        // scratch buffer — so the call writes into `buf`'s own backing and clobbers it
        // (native: `encode_map_ic`'s second `+= head()` clips the result to len 1;
        // interp tolerated the aliasing).  Preserve `buf`'s pre-append backing dep
        // across `change_var` so the append never changes ownership.
        let preserve_append_backing =
            op == "+=" && var_nr != u16::MAX && matches!(self.vars.tp(var_nr), Type::Vector(_, _));
        let saved_backing: Vec<u16> = if preserve_append_backing {
            self.vars.tp(var_nr).depend()
        } else {
            Vec::new()
        };
        // `&τ` is a PARAMETER-PASSING MODE, not a value type: it says the callee shares
        // the caller's store in place, which only the argument slot can express.  Reading
        // a `&` parameter as a value (`w = v`) yields `τ` BORROWED FROM that parameter,
        // so the peel names the parameter as the dep — the declared `&τ` carries none of
        // its own, and dropping the mode without recording the borrow would make `w` an
        // owner, silently turning the shared store into a copy.  Without the peel the
        // local inherited `&τ` itself, and a variable read as an argument gets no stack
        // slot: `w` came out at slot 65535, which surfaced as a wrong answer (`w += […]`
        // appended into nothing), an ICE, or a SIGSEGV depending on what the body did
        // with it next (loft#772's sibling).
        //
        // Only a bare read of a `&` PARAMETER peels.  An explicit `&`-binding (`d = &c`,
        // @PLN87 L1/L2) runs the other way — the source is an ordinary local and the `&`
        // is what MAKES the reference — so it keeps `RefVar` and stays a live link.
        let s_type = match (&s_type, code.unspan()) {
            (Type::RefVar(inner), Value::Var(src))
                if self.vars.is_argument(*src)
                    && matches!(self.vars.tp(*src), Type::RefVar(_))
                    && !matches!(to.unspan(), Value::Var(d) if self.vars.is_argument(*d)) =>
            {
                inner.depending(*src)
            }
            _ => s_type,
        };
        // loft#893 — a wrong-typed FIELD store, reported at the one point every store
        // form still reaches. The checks further down cover a scalar target only, and
        // a `text` or collection target returns before them, so this is the chokepoint.
        if self.field_store_mismatch(op, var_nr, f_type, &s_type) {
            if matches!(s_type, Type::Null) {
                // The only mismatch a bare `null` can be here is a `fn(…)` target (every
                // other null-taking target is exempted in `field_store_mismatch`), and
                // "use `as fn(integer) -> integer` to cast explicitly" is advice that
                // cannot be followed — the cast does not exist and would not help if it
                // did.  Name what is actually true of the slot instead.
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot assign null to a slot of type {} — a fn-ref holds a \
                     function's identity and has no encoding for absence, so the slot \
                     cannot be cleared; assign another function, or keep the absent \
                     case in a separate field",
                    f_type.name(&self.data),
                );
            } else {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot assign {} to a field of type {} — use 'as {}' to cast explicitly",
                    s_type.name(&self.data),
                    f_type.name(&self.data),
                    f_type.name(&self.data),
                );
            }
        }
        // loft#1034 — a TUPLE target reaches `convert` too.
        //
        // `scalar_target` above lists the types whose annotation drives a conversion, and
        // a tuple is not one of them, so `c: (text?, integer) = ("c0", 3)` never converted
        // the literal against what the annotation asked for.  That had two consequences,
        // one loud and one silent:
        //
        //   * `change_var_type` then compared the declared `(text?, integer)` against the
        //     literal's own inferred `(text, integer)` and refused the declaration —
        //     "cannot change type from (text?, integer) to (text, integer)" — for a
        //     widening that is legal in every other position;
        //   * a `null` ELEMENT stayed a bare null instead of becoming the element type's
        //     sentinel, so `(null, 3)` stored the empty text (`h.0 == null` answered
        //     false) and would not compile at all on `--native`, which emitted `()`.
        //
        // The RETURN position always converted — `convert`'s own Tuple arm walks the
        // elements and applies each one's coercion — which is exactly why the identical
        // type was accepted there and the issue read as a local-only refusal.  Routing the
        // local through the same function is what makes the two positions agree, rather
        // than teaching this site a second opinion about tuples.
        //
        // Adopting `f_type` on success is what lets `change_var` below, and the rest of
        // this function, see the declared type rather than the literal's.
        //
        // ⚠ ORDER IS THE CONTRACT, and it is why this sits immediately above
        // `change_var`.  `change_var_type` decides acceptance with `decl_accepts`, which
        // answers `(N-Decl)` — a `τ?` slot admits a `τ` — and nothing else.  A member
        // needing a REAL coercion is not in that relation, so a conversion performed
        // AFTER the retype arrives too late to prevent the refusal: `t: (Shape, integer)
        // = (Shape::Circle { r: 7 }, 9)` was rejected *"cannot change type from
        // (Shape, integer) to (Circle, integer)"* even though `convert` — running a few
        // hundred lines below — then answered yes to the very same question.  The two
        // positions that always worked, RETURN and ARGUMENT, convert before anything
        // records a type; this is that same order.
        //
        // BOTH passes, for the same reason.  A pass-1 refusal aborts before pass 2 runs,
        // so gating the conversion on `!first_pass` left it permanently unreachable for
        // exactly the programs it exists to accept.  Pass-1 IR is rebuilt from source in
        // pass 2, so the coercions emitted here are discarded rather than duplicated.
        let s_type = if op == "="
            && matches!(f_type, Type::Tuple(_))
            && matches!(s_type, Type::Tuple(_))
            && !f_type.is_equal(&s_type)
            && self.convert(code, &s_type, f_type)
        {
            f_type.clone()
        } else {
            s_type
        };
        // loft#1145 — the other half of loft#915.  That issue gave a `for` loop's VARIABLE
        // its own binding per loop, so two loops may spell one name at their own element
        // types; a local declared in the BODY kept a single function-wide binding, so the
        // second loop's `e = y` re-typed the first loop's slot and was REFUSED.  #915's own
        // argument applies unchanged: the binding is what splits, not the scope — `e` is
        // still readable after the loop (measured: it is, and any fix has to keep that), and
        // the name resolves to whichever loop most recently bound it, exactly as the loop
        // variable does.
        //
        // ⚠ STRICTLY ADDITIVE, and that is the property the whole cut rests on.  It fires
        // only where `retype_would_be_refused` says the program is rejected TODAY, so no
        // program that compiles can change behaviour.  The gate matters: an unconditional
        // per-loop rebind would break the accumulator idiom — `for … { total = total + x.v }
        // for … { total = total + 1 }` re-types nothing, so it must keep ONE binding, and a
        // fresh one would read an unwritten slot.  That predicate is conservative on
        // purpose; see its doc for why it is not a second opinion about `change_var_type`.
        let cur_loop = self.vars.current_loop();
        let born_in = if var_nr == u16::MAX {
            u16::MAX
        } else {
            self.vars.created_in_loop(var_nr)
        };
        let rebound_to;
        let to = if op == "="
            && !s_type.is_unknown()
            && cur_loop != u16::MAX
            && born_in != u16::MAX
            && born_in != cur_loop
            && self
                .vars
                .retype_would_be_refused(var_nr, &s_type, &self.data)
        {
            // The SOURCE spelling, not the bound one.  A variable already split once is
            // named `e#b1`, and registering the next split under that would key the third
            // loop by a name no `names` lookup ever asks for — measured: two loops worked
            // and the third re-typed the second's binding.  `#` cannot occur in a loft
            // identifier, so the prefix before `#b` is exactly what the program wrote.
            let bound = self.vars.name(var_nr).to_string();
            let name = bound.split("#b").next().unwrap_or(&bound).to_string();
            var_nr = self
                .vars
                .body_local_binding(&name, var_nr, &s_type, &mut self.lexer);
            rebound_to = Value::Var(var_nr);
            &rebound_to
        } else {
            to
        };
        // loft#1237 — `keyed_local += <vector VALUE>`, the third place kind of loft#1159's
        // question.  That issue gave a keyed FIELD the route that inserts every record a
        // vector holds, each placed by its own key, and gated it on `var_nr == u16::MAX`.  A
        // keyed LOCAL is the other spelling and reached no route at all: the statement fell
        // through to `change_var` immediately below, which reads it as `h = <vector>` and
        // refuses it as a TYPE CHANGE — *"Variable 'h' cannot change type from hash<E,[k]> to
        // vector<E>"*.  Nothing in that message says `+=` was understood as an append, and
        // both cures it names are wrong here: a new variable name does not help, and `as`
        // cannot convert a `vector<E>` to a `hash<E[k]>`.
        //
        // `(Col-Insert)` is written over `c += [rec, …]` and says a keyed kind places each
        // record by its key.  It does not distinguish how the destination is REACHED, so the
        // local owes the same answer the field gives — the same argument loft#1159 made for
        // the field and loft#1233 made for the capture.
        //
        // It has to sit ABOVE `change_var` rather than beside its field twin further down,
        // because for a local the refusal fires first and the twin is never reached.  The
        // source is validated before this point: loft#1215's classifier answers
        // `ElementVector` for exactly the vector-of-the-element-type shape and refuses the
        // rest, so reaching here already means the records fit.
        //
        // Claims the statement on BOTH passes — pass 1 must not fall into the refusal either
        // — and emits only on pass 2, the way @P277's literal interception does.
        //
        // ⚠ The gate is the TYPE SHAPE, and the keyed type-table id is resolved only when
        // emitting.  `keyed_field_kt` reads the element def's `known_type`, which is still
        // `u16::MAX` during PASS 1 — so a gate that asked for the id refused to claim the
        // statement on the very pass whose refusal is the bug, and pass 2 was never reached.
        // `holds_element` is the same predicate the append classifier uses and answers on
        // both passes.
        let keyed_local_fill = op == "+="
            && var_nr != u16::MAX
            && crate::parser::vectors::is_keyed(f_type)
            && !matches!(code, Value::Insert(_))
            && match s_type.base() {
                Type::Vector(elm, _) => {
                    let content = f_type.base().content();
                    let elm = (**elm).clone();
                    self.holds_element(&content, &elm)
                }
                _ => false,
            };
        if keyed_local_fill {
            match (self.first_pass, self.keyed_field_kt(f_type)) {
                (false, Some(kt)) => {
                    #[cfg(not(feature = "wasm"))]
                    let tp_val = if self.is_struct_returning_call(code) {
                        i32::from(kt) | 0x8000
                    } else {
                        i32::from(kt)
                    };
                    #[cfg(feature = "wasm")]
                    let tp_val = i32::from(kt);
                    let src = code.clone();
                    let (parent, parent_tp_id, field_nr) =
                        self.fill_keyed_site(to, &lhs_parent_tp, kt);
                    *code = Value::Insert(vec![self.cl(
                        "OpFillKeyed",
                        &[
                            parent,
                            src,
                            Value::Int(tp_val),
                            Value::Int(i32::from(parent_tp_id)),
                            Value::Int(i32::from(field_nr)),
                        ],
                    )]);
                    return Type::Void;
                }
                // Pass 1 emits nothing and still CLAIMS the statement — pass 1's IR is
                // regenerated in pass 2, and letting it fall through is exactly the refusal
                // this route exists to remove.
                (true, _) => {
                    *code = Value::Insert(Vec::new());
                    return Type::Void;
                }
                // Pass 2 with no resolvable keyed id: fall through rather than emit a write
                // against an unknown type.  The statement then meets whatever the generic
                // path says, which is the behaviour it had before this route existed.
                (false, None) => {}
            }
        }
        // @FR-N-Decl / @FR-N-Store — a DECLARED local (or a parameter) written a nullable
        // value keeps its type and the store is asked, here, through the one store face: a
        // WARNING at full width (the store proceeds and the slot holds the null sentinel),
        // an ERROR at a narrow one.  Converted BEFORE `change_var` for the reason the tuple
        // coercion above gives — a retype decided first would refuse the program outright,
        // and "cannot change type from integer to integer?" was a third spelling of the rule
        // that erred where the rule warns.  Both passes, so pass 1's retype never sees the
        // `τ?` either.  An INFERRED local takes the other arm: `change_var_type` widens it to
        // the join (`@FR-N-Join`), and nothing is asked until that `τ?` reaches a non-null slot.
        // A write-back `&τ` parameter is the same slot one link away — `x = v[i]` through
        // `x: &integer` lands in the caller's non-null `integer` — so it is asked against the
        // type it links to.
        let slot_tp: &Type = match f_type {
            Type::RefVar(inner) => inner,
            other => other,
        };
        let declared_nullable_write = op == "="
            && var_nr != u16::MAX
            && matches!(to.unspan(), Value::Var(vn) if *vn == var_nr)
            && self.author_declared(var_nr)
            && matches!(s_type, Type::Optional(_))
            && !matches!(slot_tp, Type::Optional(_) | Type::RefVar(_))
            && !slot_tp.is_unknown()
            && slot_tp.is_equal(s_type.base());
        let s_type = if declared_nullable_write {
            let what = format!(
                "{} `{}`",
                if self.vars.is_argument(var_nr) {
                    "the parameter"
                } else {
                    "the local"
                },
                self.vars.name(var_nr)
            );
            if self.convert_store(code, &s_type, slot_tp, &what, None) {
                slot_tp.clone()
            } else {
                s_type
            }
        } else {
            s_type
        };
        self.change_var(to, &s_type);
        // @PLN110 3a — track `n = len(s)` so `for i in 0..n` keeps the strict-index
        // bound.  Any OTHER assignment to `n` drops the entry: a miss is the right
        // failure for an advisory lint, a false warning is not.
        if let Value::Var(lhs) = to.unspan() {
            let bound = match code.unspan() {
                Value::Call(d, largs)
                    if op == "="
                        && matches!(
                            self.data.def(*d).original_name().as_str(),
                            "len" | "LengthVector"
                        )
                        && largs.len() == 1 =>
                {
                    crate::parser::operators::vec_key(&largs[0], &self.data)
                }
                _ => None,
            };
            match bound {
                Some(vk) => {
                    self.len_bound_locals.insert(*lhs, vk);
                }
                None => {
                    self.len_bound_locals.remove(lhs);
                }
            }
        }
        if preserve_append_backing {
            for d in self.vars.tp(var_nr).depend() {
                self.vars.make_independent(var_nr, d);
            }
            self.vars.depend_on_all(var_nr, &saved_backing);
        }
        // Plan-22 phase 02d-vi — bypass the text-special branch
        // when the LHS is auto-deref'd boxed text.  The general
        // path (towards_set + call_to_set_op + maybe_prepend_cell_alloc)
        // already has the right OpGetText → OpSetText mapping
        // (added in 02d-iv) and alloc-preamble logic (02d-iii.d/v).
        // The text-special branch's argument-promotion + work-buffer
        // logic doesn't apply to boxed-text locals — they're
        // already a Reference(__cell_text, _), not an argument
        // and not a plain text Var.
        // `.base()`, matching the `+=` test six lines below: a boxed `text?` local is a
        // boxed text local, and the text-special branch does not apply to either.  Spelled
        // bare here, it sent a nullable one down that branch and emitted `Set(65535, â¦)` —
        // loft#1206's ICE reached through the cell instead of through a field (loft#1408).
        let is_boxed_text_lhs = matches!(f_type.base(), Type::Text(_))
            && self.extract_boxed_var_from_lhs(to).is_some_and(|v_nr| {
                self.vars.exists(v_nr)
                    && crate::parser::vectors::boxed_cell_def(self.vars.tp(v_nr), &self.data)
                        .is_some()
            });
        // loft#1228 — a TUPLE ELEMENT of text type takes its own lowering, because text concat
        // is inherently variable-based: every route below builds through a destination VARIABLE
        // (`OpAppendText(var, …)`) and a tuple slot is not one.  Minting a work variable and
        // appending into it — which is what this branch used to do here — produced an append
        // that was never written back: codegen then saw a variable naming no slot, SIGSEGV on
        // the interpreter and `E0425` from rustc.
        //
        // So build the sequence the `=` branch already builds for a self-reference — clear a
        // work text, append the CURRENT value, append the operand — and finish it with the
        // `TuplePut` a tuple slot is written through.  Reading the place for the append and
        // writing it back addresses it twice, which `(E-Asgn-Compound)` permits here: a tuple
        // root is a plain variable and the index is a constant, so there is no addressing to
        // duplicate.
        if op == "+="
            && matches!(f_type.base(), Type::Text(_))
            && !self.first_pass
            && let Some(tuple_lhs) = extract_nested_tuple_lhs(to)
        {
            let work = self.vars.work_text(&mut self.lexer);
            let mut ls = vec![
                self.cl("OpClearText", &[Value::Var(work)]),
                self.cl("OpAppendText", &[Value::Var(work), to.clone()]),
                self.cl("OpAppendText", &[Value::Var(work), code.clone()]),
            ];
            ls.push(build_nested_tuple_assign(to, &tuple_lhs, Value::Var(work)));
            *code = Value::Insert(ls);
            return Type::Void;
        }
        if matches!(f_type.base(), Type::Text(_)) && !is_boxed_text_lhs {
            // A text assignment needs somewhere to assign TO. Every other type
            // falls through to the general operator dispatch, which refuses a
            // non-place left side ("Not implemented operation = for type
            // integer" is what `5 = 6` gets); this arm intercepts text first and
            // used to hand `assign_text` a target that names no variable — which
            // reached codegen as a load of variable 65535 and took the whole
            // compiler down with an index-out-of-bounds.
            //
            // The case that finds this is not the silly one. A file-scope
            // `W: text = ""` is a CONSTANT, inlined at each use, so `W = "x"`
            // from inside a function is an assignment to a literal — and it is
            // an easy thing to write, because the same declaration for an
            // `integer` is refused with a message.
            if var_nr == u16::MAX && lhs_base_var(to, &self.data) == u16::MAX {
                if !self.first_pass {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "the left side of this assignment is a value, not a variable. A \
                         file-scope `NAME: text = …` is a CONSTANT — it is inlined at every \
                         use, so there is nothing to assign to. Declare it inside a function, \
                         or wrap it in a zero-argument function and call that"
                    );
                }
                return Type::Void;
            }
            // @PLN25 slice (c): `.base()` so a `text?` accumulator routes to assign_text
            // (`OpAppendText`) like plain text. `s += x` on a null `text?` is ignored at
            // runtime (the append skips a null dest — a non-null text can never be null);
            // a non-null `text?` appends normally. Propagate: null stays null.
            // auto-promote text argument to local String on first mutation.
            let effective_var = if self.first_pass
                && var_nr != u16::MAX
                && self.vars.is_argument(var_nr)
                && (op == "=" || op == "+=")
            {
                let name = self.vars.name(var_nr).to_string();
                let shadow = self.vars.add_variable(
                    &format!("__tp_{name}"),
                    &Type::Text(Deps::none()),
                    &mut self.lexer,
                );
                self.vars.set_promoted_from(shadow, var_nr);
                // @PLN40 — a const text arg still promotes to a local String so a rebind
                // (`p = …`) has a slot to write, but the promoted local INHERITS the
                // const axis so the const guard still fires: value-const blocks `+=`
                // (mutation) while allowing `=` (rebind); binding-const the reverse.
                if self.vars.is_value_const(var_nr) {
                    self.vars.set_value_const(shadow);
                }
                if self.vars.is_const_binding(var_nr) {
                    self.vars.set_const_binding(shadow);
                }
                self.vars.remap_name(&name, shadow);
                shadow
            } else {
                var_nr
            };
            self.assign_text(code, &s_type, to, op, effective_var);
            return Type::Void;
        }
        if self.assign_refvar_text(code, f_type, &s_type, op, var_nr) {
            return Type::Void;
        }
        if self.assign_refvar_vector(code, f_type, &s_type, op, var_nr) {
            return Type::Void;
        }
        // Rewrites `code` into an owned copy and returns false, so the general path
        // still emits the `Set(out, …)` that transfers it (loft#775).
        self.assign_refvar_reference(code, f_type, op, var_nr);
        self.assign_refvar_keyed(code, f_type, op, var_nr);
        if var_nr != u16::MAX && self.create_vector(code, f_type, op, var_nr, snapshot_len) {
            return Type::Void;
        }
        // P193: rewrite `local: keyed_collection<T> = []` to
        // `Set(v, Null)` so codegen's gen_set_first_keyed_null fires
        // at the declaration site (not lazily on first write).
        // Falls through to the standard assign path which emits
        // Set(v, code) — codegen then takes the Null arm.
        if var_nr != u16::MAX && !self.first_pass && Self::create_keyed(code, f_type, op, var_nr) {
            // Don't return here — let the standard pipeline emit
            // Set(v, Null) so codegen sees it.  No further special-
            // case handling needed: the rest of the pipeline tolerates
            // `code == Value::Null`.
        }
        // vector-typed field whole-replacement.
        // `s.v = fresh` (RHS is a vector variable/expr) used to silently drop
        // the assignment because towards_set's vector branch returned bare
        // `val`.  `s.v = []` likewise leaked through to the Insert bypass at
        // the end of this function with no SET operator emitted.  Both cases
        // are now rewritten to `OpClearVector(s.v)` (+ `OpAppendVector` when
        // there is RHS data to copy in).
        //
        // P261 (2026-05-13): the third case — `s.v = [literal items]` —
        // previously fell through to the Insert bypass without an
        // OpClearVector prefix, so the literal's element-construction
        // ops appended to the existing items instead of replacing them.
        // Fixed by prepending OpClearVector to the literal's
        // statement list — the existing element-construction ops then
        // run on a fresh-empty field.
        //
        // Skipped:
        // - RHS type is non-Vector (e.g. `b.data = f#read(...)` where f#read
        //   returns text) — preserve the historical silent no-op rather than
        //   emit a type-mismatched OpAppendVector.
        //
        // loft#917 — `.base()` peels the `?`.  A `vector<T>?` field is
        // `Optional(Vector(...))`, which this selector did not match, so the whole
        // replace was skipped and `q.xs = [9]` left the literal's element-construction
        // ops appending to what the field already held: `=` silently meant `+=`, and
        // assigning `[9]` over `[7, 8]` read back as three elements on both backends.
        // An `Optional(τ)` field lays out exactly like `τ` (@PLN25 slice (b)), so the
        // replace it needs is the same one — the `?` is about what the field may hold,
        // not about how it is stored.
        //
        // loft#1279 — and a CAPTURED collection is the same lvalue wearing a different op.
        // @PLN93 taught the APPEND path that a capture resolves to `OpGetDbRef` of the
        // closure-record field rather than to `OpGetField` (`is_captured_dbref` exists for
        // exactly that), and the REPLACE path was never told.  Both of this site's symptoms
        // followed from the one omission: a LITERAL right-hand side still had @PLN93's
        // build-into-the-target path to run, so it appended to what was already there
        // (`v = [7,7]` over `[1,2]` read back `[1,2,7,7]`), while every other right-hand
        // side had nothing to run at all and the statement collapsed to a bare read of its
        // RHS — the emitted lambda for `c = src` is one `OpGetDbRef` and no store, a write
        // dropped in silence.  This is P261's case and loft#917's case a third time: the
        // selector, not the lowering, is what keeps being too narrow.
        if !self.first_pass
            && op == "="
            && var_nr == u16::MAX
            && matches!(f_type.base(), Type::Vector(_, _))
            && (self.is_field(to) || self.is_captured_dbref(to))
        {
            // Read the `?` BEFORE `.base()` peels it away — it is the whole difference
            // between a field that may record absence and one that may not (loft#917).
            let declared_nullable = matches!(f_type, Type::Optional(_));
            let f_type = f_type.base();
            // loft#917 — `q.xs = null` on a `vector<T>?` field emitted a discarded null
            // sentinel: the field kept its records (leaking them) and kept its length, so
            // the clear the author asked for did not happen at all.  What the storage can
            // express today is exactly the empty vector — a vector field holds a record id
            // and `0` means "no records", with no room left to say *absent* rather than
            // *empty* — so `= null` does what `= []` does, and the reader half of this
            // issue (`q.xs == null` still answering false) waits on that representation.
            // Freeing what the field holds is the part the write side owns, and it is
            // strictly better than dropping the statement.
            //
            // loft#922 — and it does NOT depend on the `?`.  Under the null model only the
            // SCALAR default flips to non-null: a heap type stays nullable, so `vector<T>`
            // and `vector<T>?` are one type with one layout, and (N-Store) has nothing to
            // say about a `null` reaching either.  Gating the clear on the `?` therefore
            // split one type's behaviour in two — the declared-nullable field cleared while
            // the identical field spelled without the `?` dropped the statement in silence,
            // keeping the records it was told to release.
            if matches!(s_type, Type::Null) {
                // loft#917 — and now it also MARKS the field absent, which is what makes
                // the reader half work: `= null` and `= []` both release the records, and
                // only the first leaves the reserved id behind.
                let ops = self.clear_vector_field_as(to, &lhs_parent_tp, f_type, declared_nullable);
                *code = Value::Insert(ops);
                return Type::Void;
            }
            let is_empty_literal = matches!(code, Value::Insert(ls) if ls.is_empty());
            let is_nonempty_literal = matches!(code, Value::Insert(ls) if !ls.is_empty());
            // A vector LITERAL assigned to a CAPTURED collection arrives as a `Block` that
            // builds its elements STRAIGHT INTO the destination — @PLN93's build-into-target,
            // which is what makes `coll += [x]` work through a capture — where a struct field
            // gets a `Value::Insert` of the same ops.  Different wrapper, same situation, and
            // the cure is P261's either way: run the clear FIRST and let the construction
            // fill an emptied collection.  Treating it as an ordinary value RHS instead is
            // what read back EMPTY — the clear ran after the build and erased it.
            let literal_builds_into_dest =
                matches!(code, Value::Block(_)) && self.value_writes_into(code, to);
            let rhs_is_vector = matches!(s_type.base(), Type::Vector(_, _));
            if is_empty_literal {
                *code = Value::Insert(self.clear_vector_field(to, &lhs_parent_tp));
                return Type::Void;
            }
            if is_nonempty_literal {
                let clear = self.clear_vector_field(to, &lhs_parent_tp);
                if let Value::Insert(ls) = code {
                    // After the literal's snapshot of this field, when it took one
                    // (`snapshot_read_destination`): the clear is the detach it precedes.
                    let at = snapshot_len;
                    for (i, op) in clear.into_iter().enumerate() {
                        ls.insert(at + i, op);
                    }
                }
                return Type::Void;
            }
            if literal_builds_into_dest {
                let clear = self.clear_vector_field(to, &lhs_parent_tp);
                // After the literal's snapshot of this destination, when it took one: the
                // clear is the detach the snapshot must precede (`@FR-O-Detach`).  Put it
                // INSIDE the block at the snapshot's end rather than ahead of the whole
                // block, or the snapshot copies an already-emptied destination and the build
                // reads its own result — which is what a CAPTURED collection did, the one
                // destination that reaches this arm and reads itself (loft#1391).
                if snapshot_len > 0
                    && let Value::Block(bl) = code
                {
                    for (i, op) in clear.into_iter().enumerate() {
                        bl.operators.insert(snapshot_len + i, op);
                    }
                } else {
                    let mut ops = clear;
                    ops.push(code.clone());
                    *code = Value::Insert(ops);
                }
                return Type::Void;
            }
            if !is_nonempty_literal
                && rhs_is_vector
                && let Type::Vector(elm_tp, _) = f_type
            {
                // `s.v = s.v` is a no-op; don't
                // emit a clear (which would wipe the field) + append (which
                // would then see an empty source).
                // Plan-07 phase 1: compare via unspan() so the same
                // expression wrapped at two different source positions
                // (LHS `s.v` at one column, RHS `s.v` at another) still
                // compares equal.
                if *code.unspan() == *to.unspan() {
                    *code = Value::Insert(Vec::new());
                    return Type::Void;
                }
                let elm_tp_clone = (**elm_tp).clone();
                // @P314 — narrow-aware element type (see `append_elem_tp`).
                let rec_tp = Value::Int(self.append_elem_tp(&elm_tp_clone));
                // When the RHS may alias the destination, the OpClearVector
                // below would wipe that data before OpAppendVector can copy
                // it — capture the RHS into a fresh local first so its
                // storage is independent of the clear.  Two aliasing shapes:
                // - a non-Var RHS (e.g. `s.v = pop_tail(s.v, 1)`), and
                // - #320: a Var that BORROWS the destination (`w = s.v;
                //   w += [43]; s.v = w` — w and the field share one store;
                //   clear+append emptied the field).  A borrow is visible in
                //   the var's type deps; only a dep-free (owned) var is
                //   provably alias-free and may take the direct fast path.
                // @FR-O-Proxy asks copy — an ALIASING question: only a dep-free var is
                // provably independent of the destination, so only it may take the direct
                // clear+append.  The other arm materialises into a temp; neither frees.
                let owned_var_rhs = matches!(
                    code.unspan(),
                    Value::Var(rv) if self.vars.tp(*rv).depend().is_empty()
                );
                if owned_var_rhs {
                    let mut ops = self.clear_vector_field(to, &lhs_parent_tp);
                    let append = self.cl("OpAppendVector", &[to.clone(), code.clone(), rec_tp]);
                    ops.push(append);
                    *code = Value::Insert(ops);
                } else if matches!(code.unspan(), Value::Var(_)) {
                    // Borrowed Var: a synthetic `Set(tmp, Var(w))` would alias
                    // again (it bypasses the parse-time vector deep-copy
                    // lowering) — build the temp as a fresh empty vector and
                    // element-append the borrow into it BEFORE the clear.
                    let rhs_saved = code.clone();
                    let dep_free_tp = Type::Vector(Box::new(elm_tp_clone.clone()), Deps::none());
                    let tmp = self.vars.unique("_p154_rhs", &dep_free_tp, &mut self.lexer);
                    let init_tmp = v_set(tmp, Value::Null);
                    let fill_tmp = self.cl(
                        "OpAppendVector",
                        &[Value::Var(tmp), rhs_saved, rec_tp.clone()],
                    );
                    let clear = self.clear_vector_field(to, &lhs_parent_tp);
                    let append = self.cl("OpAppendVector", &[to.clone(), Value::Var(tmp), rec_tp]);
                    let mut ops = vec![init_tmp, fill_tmp];
                    ops.extend(clear);
                    ops.push(append);
                    *code = Value::Insert(ops);
                } else {
                    let rhs_saved = code.clone();
                    // Dep-free, like the borrowed-Var arm above: the temp holds the
                    // RHS's OWN storage, so its type must say it owns it.  Built from
                    // `f_type` — the destination FIELD's type — it inherited the
                    // field's deps, read as a borrow of the struct, and was never
                    // freed at scope end.  An RHS that allocates then leaked its store
                    // for the lifetime of the program: `b.d = f#read(8) as
                    // vector<single>` (loft#897), where binding the same expression to
                    // a local first was clean, because a user local has no such dep.
                    let dep_free_tp = Type::Vector(Box::new(elm_tp_clone.clone()), Deps::none());
                    let tmp = self.vars.unique("_p154_rhs", &dep_free_tp, &mut self.lexer);
                    // …but that is only true when the RHS ALLOCATED what it hands
                    // back.  Where it hands back storage someone else owns — a
                    // return buffer the caller allocated, or a projection into a
                    // record the temp merely names — the `Set` below aliases that
                    // storage, and owning it here frees it at the temp's scope exit
                    // while its real owner is still live.  Inside a loop that scope
                    // exit is once per ITERATION (loft#906); across a call it is the
                    // caller's argument (loft#939).  The real owner still frees it.
                    //
                    // Silent in a release build until the slot is REUSED, which is
                    // what turns it from a dangling read into a wrong answer: the
                    // `LOFT_POISON` gate makes the loop shape a SIGSEGV instead,
                    // because `b.items = add(b.items, k)` then reads a record
                    // overwritten with 0xDEADBEEF.
                    if Self::borrows_its_storage(&self.data, &rhs_saved, &s_type) {
                        self.vars.set_skip_free(tmp);
                    }
                    let set_tmp = v_set(tmp, rhs_saved);
                    let clear = self.clear_vector_field(to, &lhs_parent_tp);
                    let append = self.cl("OpAppendVector", &[to.clone(), Value::Var(tmp), rec_tp]);
                    let mut ops = vec![set_tmp];
                    ops.extend(clear);
                    ops.push(append);
                    *code = Value::Insert(ops);
                }
                return Type::Void;
            }
        }
        // @P307 — keyed-collection STRUCT FIELD replace: `s.h = [..]` where
        // `s.h: sorted`/`hash`/`index<T[K]>`.  The vector-field branch above
        // handles `s.v = [..]`; the keyed analog used to fall through to the
        // Insert bypass with no op emitted (silent no-op + leak) AND the
        // keyed-field write was never recognised by `check_ref_mutations`
        // (rejecting a `&` param as unmodified — see find_field_written_vars).
        // Prefix the literal with `OpClearKeyed(field, kt)`, which
        // `remove_claims`-frees the contents and zeroes the field's claim
        // pointer; the literal's element-construction ops then run on an empty
        // collection.  Mirrors the keyed-LOCAL clear (@P302, via OpDatabase)
        // but for the in-struct claim shape.
        //
        // loft#895 — this covers a NON-EMPTY literal for the same reason the
        // vector branch above does.  Only the empty one was cleared, so
        // `s.h = [a, b]` appended to whatever the field already held and `=`
        // silently meant `+=`: assigning twice left four elements rather than
        // the two just written.
        //
        // loft#898 — a MULTI-INDEXED field is no longer excluded.  Its records are
        // shared with a sibling view, so the clear used to free what the sibling
        // still held and the exclusion traded that use-after-free for an append
        // (`=` silently meaning `+=` on a group).  `keyed_group_clear` now emits a
        // clear that releases the records exactly once, through their owner, and
        // resets the other routes to them — so the group takes the same replace
        // every other keyed field gets.
        //
        // loft#922 — `s.h = null` takes the same clear, for the same reason the vector
        // field above does: a keyed field holds a claim pointer with no spelling for
        // *absent* that is not *empty*, so releasing what it holds is the closest honest
        // meaning the storage has, and dropping the statement left the field holding
        // records the author had said to let go.
        //
        // The kinds are all five, not the three this selector used to name: `spatial`
        // (Radix) and the text-keyed trie fell out of the match and so kept the whole
        // @P307/loft#895 defect this branch exists to fix — on a `spatial` field `=` still
        // meant `+=` (a one-element literal over one element read back as two) and `= []`
        // still did nothing at all.  The keyed-LOCAL path below already lists all five.
        //
        // loft#1326 — and a CAPTURED keyed collection is the same lvalue wearing a different
        // op, which is the fourth time this family's selector has been the narrow part while
        // the lowering was right (P261, loft#917, loft#1279, this).  A capture resolves to
        // `OpGetDbRef` of the closure-record field rather than to the `OpGetField` a struct
        // field gives, so `is_field` alone answered "not a field" and the whole branch was
        // skipped: `m = [Row { … }]` inside a closure read back EMPTY at every keyed kind,
        // while the identical rebind of a captured VECTOR was correct (loft#1279 taught the
        // vector branch above this same difference) and the identical keyed collection
        // reached through a captured STRUCT FIELD was correct throughout.
        //
        // The literal ARRIVES differently through a capture, and that is the second half: it
        // is a `Block` whose ops build straight into the destination (@PLN93's
        // build-into-target), where a struct field gets a `Value::Insert` of the same ops.
        // Both want the clear FIRST — the construction then fills an emptied collection —
        // and asking "does this right-hand side construct in place?" through
        // `value_writes_into` is the same question the vector branch asks, for the same
        // reason: a comprehension that merely NAMES its destination builds a fresh
        // collection and must not be cleared before it reads its source (loft#1195).
        let captured_dest = self.is_captured_dbref(to);
        let builds_into_dest =
            matches!(&*code, Value::Block(_)) && captured_dest && self.value_writes_into(code, to);
        let keyed_field_write = !self.first_pass
            && op == "="
            && var_nr == u16::MAX
            && (self.is_field(to) || captured_dest)
            && (matches!(&*code, Value::Insert(_))
                || builds_into_dest
                || matches!(s_type, Type::Null));
        if keyed_field_write && let Some(kt) = self.keyed_type_id(f_type) {
            let clear = self.keyed_group_clear(to, kt, &lhs_parent_tp);
            if builds_into_dest {
                // The construction is one opaque block: put the clear in front of it rather
                // than inside it, or the clear erases what the block has just built.
                let mut ops = clear;
                ops.push(code.clone());
                *code = Value::Insert(ops);
                return Type::Void;
            }
            match code {
                // A literal: run the clear FIRST, so the element-construction ops that
                // follow build into an empty collection instead of appending to the old one.
                Value::Insert(ls) => {
                    for (i, op) in clear.into_iter().enumerate() {
                        ls.insert(i, op);
                    }
                }
                // `= null`: the clear is the whole statement.
                _ => *code = Value::Insert(clear),
            }
            return Type::Void;
        }
        // @P292 / @P394 — `local_v = other_var_v` where the RHS is a bare vector
        // Var read (not a fresh-storage Block / Call / slice).  The standard Set
        // path would emit `Set(v, Var(rhs))`, which makes v ALIAS rhs's storage:
        //  - @P292: when rhs later goes out of scope its storage is freed → v
        //    dangles → next `len(v)` returns 0 or SEGVs.
        //  - @P394: when this is v's FIRST assignment (its own store not yet
        //    allocated) the alias path never gives v a stack slot → v lands on
        //    u16::MAX → `generate_var` asserts (codegen.rs:2669), or with a
        //    trailing `+=` v silently stays empty and rhs corrupts.
        // Fix (mirrors the slice-materialise branch above): give v its OWN store
        // — `insert_new` when v doesn't own one yet (first assignment), else
        // clear v's existing store (reassignment) — then `OpAppendVector` deep-
        // copies rhs's elements in.  Vectors are copy-semantics, so v is fully
        // independent of rhs afterwards.  Fires on BOTH passes (change_var sets
        // v's type each pass — preserving an existing dep; the alloc/append emit
        // on the second), so the __vdb_N dep is created consistently, exactly as
        // the materialise path at lines ~995 does.  Element type is read from the
        // RHS (s_type) so an untyped `b = a` (f_type Unknown) is covered too.
        // @PLN85 D-own-1 / C86 — classify ONCE (the pure selector), then apply
        // the one mechanism per verdict.  Rule rationale lives on the `VecBind`
        // variants (the C86 whole-value-copy contract, the p379 borrowed-base
        // view, the #426 routed-forward exclusions).
        // A `& vector` bind opts into aliasing (B-Ref-Write): SKIP the C86 deep-copy so
        // the plain-assign path shares the source's DbRef and marks `d` non-owning (its
        // dep names the source).  Plain `d = v` (no `&`) still classifies + copies.
        // `@FR-B-Copy` / `@FR-O-Complete` — a vector local bound from a VALUE BRANCH
        // (`x = if c { a } else { b }`, a `match`, a `??`) is written out per arm, so each
        // arm's tail gets the lowering a single bind of that tail has: a whole variable and
        // an owned field read are COPIED through the selector below, and every other arm
        // keeps the value it has (a literal's or a call's own buffer, an index read's view).
        // Bound as one value, the branch handed the local the chosen arm's STORE — a write
        // through `x` reached `a` — on every spelling, whether or not `x` held a value
        // before.  The vector copy has its one home in this routine, which is why the
        // rewrite happens here and not in the post-parse per-arm lift that serves records.
        if !amp_vector_bind
            && op == "="
            && var_nr != u16::MAX
            && matches!(to.unspan(), Value::Var(v) if *v == var_nr)
            && matches!(f_type.base(), Type::Unknown(_) | Type::Vector(_, _))
            && matches!(s_type.base(), Type::Vector(_, _))
            // Not a promoted return buffer (calls.md F-Ret): the caller receives the value
            // THROUGH it, so every arm fills it in place and the return adopts or
            // materialises the join — the value form is that mechanism.
            && !self.is_hidden_param(var_nr)
            && self.sink_vec_bind_into_arms(code, to, var_nr, f_type, &s_type, &lhs_parent_tp)
        {
            return Type::Void;
        }
        let vec_bind = if amp_vector_bind {
            VecBind::NotABind
        } else {
            self.classify_vec_bind(code, op, var_nr, f_type, &s_type)
        };
        if !matches!(vec_bind, VecBind::NotABind) && matches!(s_type.base(), Type::Vector(_, _)) {
            self.lower_vec_copy_bind(code, &vec_bind, to, var_nr, &s_type, &lhs_parent_tp, false);
            return Type::Void;
        }
        // @P295 — `local_s = keyed_expr` where the LHS is a KEYED-collection
        // LOCAL (`sorted`/`hash`/`index`).  The standard Set path emits
        // `Set(s, …)` which `gen_put_var` cannot lower (no `OpPut*` arm for
        // keyed kinds → `Unknown var … type sorted<…>` panic), and a naive
        // alias would dangle when a loop-local RHS is freed (the @P292 bug,
        // for keyed kinds).  Fix: deep-copy via `OpReplaceKeyed`, which does
        // `remove_claims(dest)` (frees s's prior collection + resets its
        // store header) then `copy_claims(src, dest)` — the per-kind
        // deep-copy that rebuilds the bucket/tree index from scratch
        // (`copy_claims_seq_vector` / `_hash_body` / `_index_body`).  This is
        // the same machinery that deep-copies a keyed FIELD when its owning
        // struct is copied; here we route the local-assignment shape through
        // it.  NOTE: unlike `OpCopyRecord` there is NO `copy_block` step —
        // a keyed local's slot is a `DbRef` to a dedicated store, so the
        // collection header lives at (store, 1, 8); copy_block'ing
        // `size(tp)` bytes there corrupts the store (the failure mode of the
        // first attempt).  `s = []` (empty literal) and first-declaration
        // go through `create_keyed` above and are unaffected.
        //
        // @PLN48 — a Radix (spatial) reassignment deep-copies through the same
        // `OpReplaceKeyed` as the other keyed kinds; `copy_claims_radix_body` backs it.
        // Replaces the old @P295 "not yet supported" gate.  All five kinds come from
        // `keyed_type_id`, the one list, so this site and the FIELD site above cannot
        // drift apart again (loft#922).
        let keyed_kt = if !self.first_pass && op == "=" && var_nr != u16::MAX {
            self.keyed_type_id(f_type)
        } else {
            None
        };
        // loft#895 — keyed-collection LOCAL replace: `s = [a, b]` where
        // `s: sorted`/`hash`/`index`/`radix`/`trie<T[K]>`.  The literal arrives
        // as element-construction ops that APPEND, so without a clear in front
        // `=` meant `+=` on a local exactly as it did on a field: assigning
        // twice left both literals' elements.  `Set(s, Null)` is the local
        // clear — the same lowering `s = []` takes (P193 `create_keyed`),
        // which codegen turns into the `OpDatabase` store reset.  It also gives
        // the slot its init when a literal is the local's FIRST assignment,
        // which is what `create_keyed` does for the empty one.
        //
        // The var-RHS branch below stays separate: `s = other` deep-copies via
        // `OpReplaceKeyed`, which clears as part of the copy.
        if keyed_kt.is_some() && matches!(code, Value::Insert(ls) if !ls.is_empty()) {
            let clear = v_set(var_nr, Value::Null);
            if let Value::Insert(ls) = code {
                ls.insert(0, clear);
            }
            return Type::Void;
        }
        if let Some(kt) = keyed_kt
            && crate::parser::vectors::is_keyed(&s_type)
            && !matches!(code, Value::Insert(_) | Value::Null)
        {
            // `s = s` self-assign — emit nothing rather than clear+recopy
            // off the same storage.
            if matches!(code.unspan(), Value::Var(rhs) if *rhs == var_nr) {
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
            // 0x8000 high bit frees the source store after the deep copy when
            // the RHS is a fresh-storage call (`s = build()`), matching
            // `copy_ref`'s leak guard.  Plain Var-RHS aliases a live local —
            // no source-free (its own scope frees it).
            //
            // @FR-O-Move — a store the callee only BORROWED is not the callee's to give
            // away, so releasing "the source" here has to know which it is.
            //
            // loft#1140 — "a fresh-storage call" is what this comment says and is NOT what
            // `is_struct_returning_call` asks: it answers *is the RHS a call*, and a call
            // that hands back a BORROW of its own parameter (`fn id(x: hash<T[k]>) ->
            // hash<T[k]> { x }`) passed it.  The bit then freed the caller's collection,
            // which every call after the first read as empty.  `call_return_frees_source`
            // is the canonical answer to exactly this question — it was written for this
            // bit (loft#981/#982) — it reads the callee's return deps AND whether this
            // site's bracket covers every ref argument.  Its licence is conditional on that
            // bracket actually being emitted, which this site did not do, so the bracket is
            // emitted below.  Answering the callee-side half alone would also close the
            // use-after-free, but conservatively: the minting arm of a borrowing signature
            // (`if c { x } else { […] }`) would then leak one store per call, which the
            // bracket instead resolves at runtime — protected store, free refused;
            // callee-minted store, freed.
            //
            // loft#1154 — a JOIN reaches the same decision through `join_source_frees`, which
            // answers per ARM: a fresh-storage call's store is nobody else's, and a nameable
            // arm is protected so the runtime refuses its free.  Without it the gate asked
            // *is the RHS a call*, a `Value::If` is not one, and the store the taken arm's
            // callee minted was copied out of and abandoned.
            // NOT feature-gated, for the reason `is_struct_returning_call` gives about itself:
            // the free bit's BEHAVIOUR differs under wasm, the query does not.  Gating the
            // binding and not its use below broke the wasm build alone — which `make ci`
            // catches and no targeted suite does.
            let join_witnesses = self.join_source_frees(code);
            #[cfg(not(feature = "wasm"))]
            let tp_val = if (self.is_struct_returning_call(code)
                && crate::use_analysis::call_return_frees_source(&self.data, self.context, code))
                || join_witnesses.is_some()
            {
                i32::from(kt) | 0x8000
            } else {
                i32::from(kt)
            };
            #[cfg(feature = "wasm")]
            let tp_val = i32::from(kt);
            let replace = self.cl(
                "OpReplaceKeyed",
                &[code.clone(), to.clone(), Value::Int(tp_val)],
            );
            // The deep-copy gives `s` its OWN store, so it no longer borrows
            // the RHS.  Strip the `s["ns"]` lifetime dep the assignment set
            // up — otherwise scope analysis treats `s` as a borrow and
            // suppresses its own `OpFreeRef` (store leak) while deferring the
            // RHS's free (loop accumulation).  Mirrors the Reference var-to-
            // var deep-copy dep-strip in `scopes.rs::scan_set`.
            //
            // @PLAN52 cluster IV (Vec/Hash/Sorted/Index `??`): the original
            // form below only stripped a single Var-RHS dep, but `??` lowers
            // to a `Block` RHS, so the dep stays in place.  Native
            // `emit_null_dbref` then sees `owns_store=false` and emits the
            // null-DbRef sentinel, which crashes `OpReplaceKeyed`'s
            // `stores.allocations[u16::MAX]` lookup.  Strip ALL deps for
            // any non-Var RHS — after the deep copy `var_nr` owns its store
            // regardless of how the RHS was shaped.
            //
            // `Type::depend` is the declared home for "which vars does this type borrow?"
            // and it is dep-transparent through `Optional` (@PLN25), which is why the list
            // is asked for rather than restated.  @FR-L-Null: `layout(τ) = layout(τ?)`, so
            // a nullable keyed local owns its store exactly as its dense twin does and must
            // be stripped by the same rule.  `make_independent` already peels the wrapper on
            // the WRITE side (loft#1106); a hand-rolled five-variant match here left the
            // READ side an `Optional` short, so `hash<S[k]>?` kept the borrow it had just
            // deep-copied away from and took the sentinel path the paragraph above describes
            // (loft#1143).
            if let Value::Var(rhs) = code.unspan() {
                self.vars.make_independent(var_nr, *rhs);
            } else {
                for d in self.vars.tp(var_nr).depend() {
                    self.vars.make_independent(var_nr, d);
                }
            }
            // @P300 — prepend `Set(v, Null)` so the destination keeps a
            // recordable `Value::Set` node.  `compute_intervals` only
            // records `first_def` from a `Set`, so without it a FIRST
            // assignment (`x = mk()`) leaves `x` slot-less → the
            // `Incorrect var x[65535]` panic in `generate_var`.
            // `scan_set` (`scopes.rs`) makes the prepend do the right
            // thing for free: on a FIRST assignment `x` is not yet in
            // scope so the `Set(x, Null)` survives → codegen's keyed-Null
            // arm allocates an empty store (`gen_set_first_keyed_null`);
            // on a REASSIGNMENT `v` is already in scope so `scan_set`
            // elides the redundant `Set(v, Null)`, leaving just
            // `OpReplaceKeyed` (whose `remove_claims` clears `v`'s
            // existing store before `copy_claims`).  No parse-time
            // first-vs-reassign discriminator needed.
            // loft#1140 — @P290 bracket, the same one `state/codegen.rs` emits around
            // `OpCopyRecord`'s source-free.  Whether a borrowing callee hands back the
            // argument's store or one it minted itself is a RUNTIME fact no static bit can
            // carry, so the free is decided by marking the argument stores: `OpReplaceKeyed`
            // refuses to free a source that is `is_free_protected`, and frees one that is
            // not.  Without it this site had to choose statically, and either choice is
            // wrong for one arm — free, and `fn id(x) -> hash<T[k]> { x }` takes the
            // caller's collection; do not free, and the minting arm leaks one store per
            // call.  `protectable_ref_args` is the same derivation the source-free gate
            // above consults for coverage, so the marks and the licence cannot drift.
            let mut seq = vec![Value::Set(var_nr, Box::new(Value::Null))];
            // A JOIN's witnesses are its ARMS' — `protectable_ref_args` reads a call's
            // arguments and a join has none of its own (loft#1154).
            let guarded: Vec<u16> = if tp_val & 0x8000 == 0 {
                Vec::new()
            } else if let Some(w) = join_witnesses {
                w
            } else {
                crate::use_analysis::protectable_ref_args(&self.data, self.context, code).0
            };
            for av in &guarded {
                seq.push(self.cl("n_protect_store_frees", &[Value::Var(*av)]));
            }
            seq.push(replace);
            for av in &guarded {
                seq.push(self.cl("n_unprotect_store_frees", &[Value::Var(*av)]));
            }
            *code = Value::Insert(seq);
            return Type::Void;
        }
        // `lhs += other_vec` where both sides are vectors: append all elements
        // in-place via OpAppendVector.
        //
        // @PLAN52 cluster IV-Vec-nested-field-push: STRICT rule — concat is
        // legal only when `typeof(rhs) == typeof(lhs)` exactly.  Any mismatch
        // (element-type, narrow-type, or otherwise) is rejected.  Single-
        // element-push case (RHS type == LHS element type) is already caught
        // by the diagnostic just before this branch.
        //
        // loft#1207 — the DESTINATION is read through `.base()`, the reading
        // `vectors::is_keyed` already takes: a `vector<τ>?` field is stored as the vector it
        // names plus one reserved null, so which vector to append to does not depend on the
        // wrapper.
        //
        // This is the PASS-2 half of that issue and it does not stand alone.  The refusal a
        // nullable vector append used to earn is decided a pass earlier, in
        // `vectors::is_collection`: `towards_set`'s collection interception is asked in pass 1,
        // and while that predicate matched its `Vector` arm bare it denied a `vector<τ>?`, so
        // the statement fell through to the generic operator lookup as *"No matching operator
        // 'Add' on 'vector<E>?'"* before any `!first_pass` branch could claim it.  With the
        // pass-1 half alone the statement compiles and routes here; with this half alone the
        // pass-1 refusal still fires.  Both are load-bearing, and each is measured so by
        // reverting it against the guard.
        //
        // The SOURCE is deliberately left unpeeled.  A `τ?` value stored into a `τ` slot is
        // `(N-Store)`'s question, not this branch's, and peeling here would ADMIT it: the
        // dense-destination direction is loft#1210, where the interpreter panics writing a
        // read-only store and `--native` emits Rust that will not compile.  Widening the
        // destination must not quietly widen the source with it.
        // loft#1228 — …unless the right-hand side ALREADY appended into the destination.
        //
        // A vector literal is parsed with the left-hand PLACE as its accumulator.  For a bare
        // variable or a struct field `build_vector_list` builds the elements straight into it
        // and hands back a `Value::Insert`, which the guard below excludes — that is why both
        // of those place-kinds were always correct.  A TUPLE ELEMENT is neither, so the literal
        // opens a block by ADOPTING the place's store (`_vec_N = t.0`) and builds into that;
        // the block is a `Value::Block`, the exclusion misses it, and this concat then appends
        // the destination to ITSELF.
        //
        // Measured: `t.0 += [7]` on an empty element answered `[7, 7]`, on `[1, 2]` it answered
        // `[1, 2, 7, 1, 2, 7]`, and `+= [7, 8]` answered `[7, 8, 7, 8]` — the correct result
        // concatenated with itself, which is the signature of one append too many rather than
        // of a wrong element.  `(E-Asgn-Compound)` is the rule: the place is addressed exactly
        // once, and here it was the accumulator AND the destination.
        //
        // The test is that the block's head adopts exactly `to`.  A destination is a PLACE, so
        // it can never be the fresh-storage temp the adopt exists for, and the two cannot be
        // confused.
        //
        // Gated on the PLACE-KIND, not on the adopt shape alone: a CAPTURED collection
        // (`OpGetDbRef`) reaches the same shape and NEEDS the append — suppressing it there
        // sent the write into the const store (`505-collection-capture.loft`: *"Write to
        // read-only store … (CONST_STORE init)"*).  The shape is shared; the correct lowering
        // is not.
        //
        // The place-kind is asked through [`extract_nested_tuple_lhs`], which is the one home
        // for what a tuple place looks like, because a tuple projection has TWO IR spellings —
        // a bare `TupleGet` at depth 1 and a `Block[Set(w, …), TupleGet(w, idx)]` chain deeper —
        // and matching `TupleGet` here saw only the first.  Measured: with that narrower test
        // `t.0 += [7]` was fixed while `n.0.0 += [7]` still answered `[7, 7]`.  QUALITY.md's
        // `spellings` screen is what named it — the audit row moved, and the cell built to
        // answer *why* found the half-fix.
        let rhs_built_into_place = extract_nested_tuple_lhs(to).is_some()
            && matches!(code.unspan(), Value::Block(bl)
                if matches!(bl.operators.first().map(Value::unspan), Some(Value::Set(_, adopted))
                    if adopted.unspan() == to.unspan()));
        if !self.first_pass
            && op == "+="
            && let Type::Vector(elm_tp, _) = &f_type.base().clone()
            && matches!(s_type, Type::Vector(_, _))
            && !matches!(code, Value::Insert(_))
            && !rhs_built_into_place
        {
            if !s_type.is_equal(f_type.base()) {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "vector `+= other_vec` requires equal types ({} != {})",
                    f_type.base().name(&self.data),
                    s_type.name(&self.data)
                );
                *code = Value::Insert(Vec::new());
                return Type::Void;
            }
            // @P314 — narrow-aware element type (see `append_elem_tp`).
            let elm = (**elm_tp).clone();
            let rec_tp = self.append_elem_tp(&elm);
            *code = Value::Insert(vec![self.cl(
                "OpAppendVector",
                &[to.clone(), code.clone(), Value::Int(rec_tp)],
            )]);
            return Type::Void;
        }
        // @PLN93 (#511): a BARE captured collection is a DbRef append target too — just
        // not a *field*.  `h += …` inside a closure reaches the collection via an
        // `OpGetDbRef` of the closure-record field, so `var_nr == u16::MAX` and
        // `is_field(to)` is false, which is why the keyed-`+=` insert below never fired
        // and the append silently no-op'd.  Treat a captured-collection target (closure
        // param in its deps) the same as a field target: `new_record(&mut to.clone(), …)`
        // inserts into the shared store (Step 0 proved inserts persist across grows with
        // no header write-back).
        // @PLN93 (#511): a struct-field target (`is_field`) OR a bare collection captured into a
        // closure (an `OpGetDbRef` whose type still depends on the closure param) is a
        // DbRef-producing append lvalue — the keyed-`+=` inserts below must target the shared
        // store, not a throwaway local (`var_nr == u16::MAX` for both).
        let dbref_append_target = self.is_field(to)
            || (self.closure_param != u16::MAX
                && Self::is_collection_type(f_type)
                && f_type.depend().contains(&self.closure_param));
        // Scalar `field += elem` where field is a vector field (var_nr == u16::MAX)
        // and the RHS is a single expression (variable, function call) — NOT
        // a struct literal.  Struct-literal RHS is handled by the keyed-
        // collection branch below, which also covers vectors.
        //
        // ⚠ **Measured UNREACHABLE as of loft#1223, and deliberately kept.**  Zero callers
        // across every `.loft` file in the repository (`tests/scripts`, `tests/docs`,
        // `tests/lib`, `default`, `examples`, `doc`, `bench`, `tools`), and unreachable by
        // argument too: for a `Type::Vector` destination every source shape is claimed
        // earlier — an ELEMENT by @PLAN52's bracket refusal (which reads both sides through
        // `.base()` since loft#1223, so the nullable spelling no longer slips past), a VECTOR
        // by the concat branch, and anything else by loft#1215's classifier.  Its last caller
        // was the un-bracketed nullable element, which is exactly what loft#1223 refused.
        //
        // Kept rather than deleted because the two failure modes are not symmetric.  A dead
        // branch costs a reader's attention.  A WRONG deletion costs a silent wrong answer:
        // the shape would fall through to the generic assignment path, which for a collection
        // destination emits no write — the precise failure loft#1221 was.  The argument above
        // rests on the ordering of four separate checks, any of which a later change may move,
        // and this same branch was called dead once before on a reading that a measurement
        // then contradicted.  Delete it behind a fresh probe, not behind this comment.
        if !self.first_pass
            && var_nr == u16::MAX
            && op == "+="
            && dbref_append_target
            && let Type::Vector(elm_tp, _) = f_type
            && !matches!(code, Value::Insert(_))
        {
            let elm_tp = (**elm_tp).clone();
            let elm = self.unique_elm_var(&lhs_parent_tp, &elm_tp, u16::MAX);
            let scalar = code.clone();
            // @PLN93 (#511): captured-vector target — pass the collection type so `new_record`
            // resolves the vector db-type against the shared store (see the keyed branch below).
            let np = if self.is_captured_dbref(to) {
                f_type.clone()
            } else {
                lhs_parent_tp.clone()
            };
            let ls = self.new_record(&mut to.clone(), &np, elm, u16::MAX, &[scalar], &elm_tp);
            *code = Value::Insert(ls);
            return Type::Void;
        }
        // loft#1159 — `keyed_field += <vector VALUE>`: insert every record the vector holds,
        // each placed by its own key.
        //
        // The keyed twin of the vector-field arm above, and it was simply absent.  That arm
        // is gated on `let Type::Vector(elm_tp, _) = f_type`, so a keyed destination never
        // matched it; the struct-literal arm below is gated on `matches!(code,
        // Value::Insert(_))` with a SINGLE element's type, so a vector-valued expression
        // never matched that either.  `h.a += rows()` fell past both to a statement that
        // emitted no write at all — `introspect` showed the call and then nothing — while
        // `h.a += [E{…}, E{…}]` was correct, because a literal is parsed into per-element
        // construction and never becomes a vector value.
        //
        // `formal/collections.md` (Col-Insert) is written over `c += [ rec, … ]` and says
        // keyed kinds place each record by key.  It does not distinguish how that vector is
        // spelled, so the two spellings owe the same answer.  No clear: `+=` adds.
        if !self.first_pass
            && var_nr == u16::MAX
            && op == "+="
            && dbref_append_target
            && !matches!(code, Value::Insert(_))
            && matches!(s_type.base(), Type::Vector(_, _))
            && let Some(kt) = self.keyed_field_kt(f_type)
        {
            #[cfg(not(feature = "wasm"))]
            let tp_val = if self.is_struct_returning_call(code) {
                i32::from(kt) | 0x8000
            } else {
                i32::from(kt)
            };
            #[cfg(feature = "wasm")]
            let tp_val = i32::from(kt);
            let src = code.clone();
            let (parent, parent_tp_id, field_nr) = self.fill_keyed_site(to, &lhs_parent_tp, kt);
            *code = Value::Insert(vec![self.cl(
                "OpFillKeyed",
                &[
                    parent,
                    src,
                    Value::Int(tp_val),
                    Value::Int(i32::from(parent_tp_id)),
                    Value::Int(i32::from(field_nr)),
                ],
            )]);
            return Type::Void;
        }
        // P192 follow-up: `field += elem` for keyed-collection fields
        // (hash/sorted/index/spatial<T[key]>) AND vector fields when
        // the RHS is a struct literal (Value::Insert).  Without this,
        // `db.h += Score{...}` emitted raw `OpSetText` / `OpSetInt`
        // writes targeting the field itself — which for hash/index
        // fields overwrote the root pointer (4-byte ref) and for
        // vector fields wrote into the length-prefix word.  The
        // pre-parsed struct-literal steps target `to` (the LHS field
        // expression); after allocating a new element via
        // `new_record_field_op`, walk the steps and substitute `to`
        // → `Var(elm)` so each field write lands in the new record.
        if !self.first_pass
            && var_nr == u16::MAX
            && op == "+="
            && dbref_append_target
            && crate::parser::vectors::is_collection(f_type)
            // loft#1221 — a KEYED destination takes an element however it is SPELLED.  The
            // `Insert` requirement is a vector's: for a vector a bare element is @PLAN52's
            // ambiguity and the brackets are required, so only a struct LITERAL can reach a
            // vector here.  A keyed kind has no such ambiguity — `h += rec` is the spelling
            // its own LOCAL has always accepted — and requiring `Insert` of it left the
            // FIELD form owned by nothing: `d.h += e` for a plain local `e` of the element
            // type fell past every route to a statement that emitted no write, and the
            // append vanished with `len` reading 0.
            && (matches!(code, Value::Insert(_)) || crate::parser::vectors::is_keyed(f_type))
        {
            let elm_tp = f_type.content();
            // Only fire for single-element append (RHS type matches the
            // collection's element type — e.g. `Score{}` for `hash<Score>`).
            // Multi-element append (RHS is a vector literal of element-typed
            // values like `[1, 2, 3]` for `vector<i32>`) keeps its
            // pre-existing handling further down (OpAppendVector / direct
            // bulk inits).
            if !elm_tp.is_unknown() && elm_tp.is_equal(&s_type) {
                let elm = self.unique_elm_var(&lhs_parent_tp, &elm_tp, u16::MAX);
                let mut scalar = code.clone();
                substitute_value(&mut scalar, to, &Value::Var(elm));
                // @PLN93 (#511): a captured-collection target (`to` = `OpGetDbRef`) has no owning
                // struct, so the record-kind dispatch (`record_new` keys off `parent_tp` when the
                // field is `u16::MAX`) must read the COLLECTION type, not the null lhs-parent —
                // pass `f_type` so `new_record` looks up the keyed `known_type` (hash/sorted/…).
                let np = if self.is_captured_dbref(to) {
                    f_type.clone()
                } else {
                    lhs_parent_tp.clone()
                };
                let ls = self.new_record(&mut to.clone(), &np, elm, u16::MAX, &[scalar], &elm_tp);
                *code = Value::Insert(ls);
                return Type::Void;
            }
        }
        // route the RHS of a simple assignment through `convert()`
        // so it picks up the same widening-with-explicit-narrowing policy
        // the constructor path (`handle_field`) and return-type path
        // (`parse_return`) already use.  `convert()` wraps `code` with an
        // `OpConv*FromY` call when a matching widening op is registered
        // (`integer → long`, `integer → float`, `single → float`, …) and
        // returns false on unrelated / narrowing mismatches — which we
        // surface as a clean diagnostic rather than silent runtime
        // corruption.  Guarded to `op == "="`: compound assignments
        // (`+=`, `-=`, …) flow through `compute_op_code` which type-checks
        // via the operator's own attribute list.  Skip when either side
        // is `Unknown` — generic / bounded-template bodies carry
        // placeholder types until monomorphisation; letting convert()
        // fire there would report spurious "cannot assign X to
        // unknown(0)" errors.
        // Restrict to scalar target types: collection targets
        // (`vector`, `hash`, `sorted`, `index`, `spatial`) and
        // `Reference` / `RefVar` compound types have their own
        // handling paths (`towards_set`, `copy_ref`, …).  Running
        // `convert()` on them would flag legitimate initialisations
        // (e.g. `h.field = [...]` for a hash field) as mismatches.
        let scalar_target = matches!(
            f_type,
            Type::Integer(_)
                | Type::Float
                | Type::Single
                | Type::Boolean
                | Type::Character
                | Type::Text(_)
        );
        // (I-Join) D4 — an INFERRED scalar integer local reassigned a WIDER integer widens
        // to the full `integer` (the join of its writes), instead of erroring on the
        // narrowing (the #433-residual: `arg = bytes[i]; arg = arg*256+…`).  An explicitly
        // annotated `x: u8` is NOT widened (it stays constrained).  Gated to a plain
        // whole-variable target (not `v[i]`/`s.field`).  Pass 1 widens the var directly
        // (`change_var_type` no-ops because `is_equal` collapses integer widths);
        // `add_variable` preserves the widened type into pass 2, so the convert / narrowing
        // checks below then see the joined type.
        let widened_int;
        let f_type: &Type = if op == "="
            && var_nr != u16::MAX
            && matches!(to.unspan(), Value::Var(vn) if *vn == var_nr)
            && matches!(f_type, Type::Integer(_))
            && !self.vars.is_annotated(var_nr)
            && Self::is_narrowing_int(&s_type, f_type)
            // Only widen when the value genuinely does NOT fit the narrow type — i.e.
            // exactly when the assignment would otherwise be a narrowing error.  A wider
            // value that PROVABLY fits (a constant) needs no widen; widening it anyway
            // over-widens width-sensitive locals, of which the engine_host kernel is one.
            && !self.int_value_fits(code, f_type)
        {
            self.vars.widen_int(var_nr, &crate::data::I64);
            widened_int = crate::data::I64.clone();
            &widened_int
        } else {
            f_type
        };
        // @PLN25 (N-Store): a typed scalar STORE — an un-discharged nullable into a non-null
        // target is a violation (emitted here; `convert` below still unwraps, so no
        // double-diagnose). This is a store site, NOT a comparison, so `x == null` is unaffected.
        let typed_scalar_store = op == "="
            && scalar_target
            && !self.first_pass
            && !f_type.is_unknown()
            && !s_type.is_unknown();
        // @FR-N-Store: a bare `null` is asked where its sentinel conversion lowers, a few
        // lines down; a value is asked by the store face where it converts.
        if typed_scalar_store
            && !matches!(s_type, Type::Null)
            && !f_type.is_equal(&s_type)
            && !self.convert_store(code, &s_type, f_type, "the assignment target", None)
        {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Cannot assign {} to a field of type {} — use 'as {}' to cast explicitly",
                s_type.name(&self.data),
                f_type.name(&self.data),
                f_type.name(&self.data),
            );
        }
        // A NULLABLE narrow target takes the implicit CHECKED narrowing instead of the
        // refusal below — `implicit_checked_narrow` is the one home, and this seam has to
        // ask it by hand because `is_equal` above kept it out of `convert` (loft#1246).
        if op == "=" && !matches!(s_type, Type::Null) {
            self.implicit_checked_narrow(code, &s_type, f_type);
        }
        // @PLAN48 P2: `x: i32 = some_integer` narrows (loses data) but integer and
        // i32 are `is_equal`, so it bypasses the convert-based check above.  Require
        // an explicit `as` unless the RHS is a constant that provably fits.
        //
        // The STORE test (loft#931): `integer` and `i32` share BOUNDS as well, differing
        // in `forced_size` alone, so range containment saw nothing here either and this
        // site — which covers both the annotated local and the field WRITE — was the last
        // place `b.v = n` could be caught before it silently stored 705032704.
        // @PLN152 — a `??` in the stored expression names what happens when the value does
        // not fit, so guard inside the discharge and leave this store alone: neither the
        // refusal below nor the outside-the-expression guard after it applies.
        let discharged =
            op == "=" && !self.first_pass && self.range_guard_inside_discharge(code, f_type);
        if !discharged
            && op == "="
            && !self.first_pass
            && Self::is_narrowing_int_store(&s_type, f_type)
        {
            let dst = self.int_type_name(f_type);
            if let Some(hint) = self.nullable_sentinel_hint(code, f_type, &dst) {
                // The literal fits the type but lands on the reserved null
                // sentinel of a nullable narrow FIELD — explain that, not "too big".
                diagnostic!(self.lexer, Level::Error, "{hint}");
            } else if !self.int_value_fits(code, f_type) {
                let src = self.int_type_name(&s_type);
                let cures = Self::narrowing_cures(code, &dst);
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "cannot implicitly narrow {src} to {dst} (may lose data) — {cures}"
                );
            }
        }
        // loft#984 — a store into a slot that DECLARES a range guards the value: one
        // outside `lo..=hi` takes the slot's default rather than being wrapped, aliased
        // or dropped.  This site is deliberately the same one the narrowing check above
        // uses, because its own comment records what it reaches: "both the annotated
        // local and the field WRITE".  A `limit(...)` slot never sets `forced_size`, so
        // `is_narrowing_int_store` above cannot see it — which is why a declared range on
        // a LOCAL went unenforced entirely, not merely mis-stored.
        if !discharged && op == "=" && !self.first_pass {
            let holds_null = target_holds_null(f_type, &lhs_parent_tp);
            self.guard_declared_range(code, f_type, &s_type, holds_null);
        }
        if self.validate_lock_assign(code, to) {
            return Type::Void;
        }
        // The const guard for `var_nr` is the one `guard_const_write` call above, asked
        // before any of these routes: the `Value::Insert` path (a struct constructor)
        // bypasses `towards_set` and used to need its own copy here.
        if !matches!(code, Value::Insert(_)) {
            let lhs = crate::parser::collections::AssignPlace {
                parent_tp: &lhs_parent_tp,
                fn_attr: lhs_fn_attr,
            };
            *code = self.towards_set(to, code, f_type, &s_type, &op[0..1], &lhs);
            // Plan-22 phase 02d-iii.e — wrap a first-set boxed-
            // scalar write with the cell-allocation preamble.
            // No-op for any other LHS shape (subsequent sets,
            // closure-body writes via `get_field`, non-boxed
            // locals, struct field writes).
            *code = self.maybe_prepend_cell_alloc(code.clone(), to);
        }
        // emit field constraint check after assignment to a constrained field.
        if !self.first_pass
            && let Type::Reference(struct_dnr, _) = &parent_tp
            && let Value::Call(_, to_args) = to.unspan()
            && to_args.len() >= 2
            && let Value::Int(field_offset) = to_args[1].unspan()
        {
            let sd = *struct_dnr;
            let off = *field_offset;
            // Find the field by matching its database offset.
            for a_nr in 0..self.data.def(sd).attributes().len() {
                let nm = self.data.attr_name(sd, a_nr);
                let fpos = self.database.position(self.data.def(sd).known_type(), &nm);
                if i32::from(fpos) == off
                    && self.data.def(sd).attributes()[a_nr].check != Value::Null
                {
                    let check = self.data.def(sd).attributes()[a_nr].check.clone();
                    let ref_val = to_args[0].clone();
                    let bound = Self::replace_record_ref(check, &ref_val);
                    let msg = match &self.data.def(sd).attributes()[a_nr].check_message {
                        Value::Text(s) => Value::Text(s.clone()),
                        _ => Value::Text(format!(
                            "field constraint failed on {}.{nm}",
                            self.data.def(sd).name()
                        )),
                    };
                    let assert_dnr = self.data.def_nr("n_assert");
                    let pos = self.lexer.pos();
                    let assert_call = Value::Call(
                        assert_dnr,
                        vec![
                            bound,
                            msg,
                            Value::Text(pos.file.clone()),
                            Value::Int(pos.line as i32),
                        ],
                    );
                    *code = Value::Insert(vec![code.clone(), assert_call]);
                    break;
                }
            }
        }
        Type::Void
    }

    /// @PLN25 E2/E3 — rewrite a nullable struct ELEMENT type `Reference(S)` to the
    /// synthetic `__nullable<S>` enum (the inline nullable representation).  Called
    /// at the ONE chokepoint where every inline `vector<S>` element resolves
    /// (`sub_type`'s `vector` arm, definitions.rs), so locals / params / returns /
    /// fields / nested all rewrite consistently.  **Default = nullable;** the
    /// caller skips this for an `S not null` element (E3 dense opt-out).  Gated on
    /// `LOFT_E2_SYNTH` — the inline-element ACCESS glue (par workers, native
    /// element reads, ref-params, casts, element-into-local mutation) is
    /// incomplete, so default-on breaks ~107 tree-wide tests (plan25 § Default-on
    /// trigger).  Stdlib excluded so the gated state never rewrites stdlib types.
    /// Creates the def ONLY — `fill_all` does the `register_enum_db` + layout in the
    /// correct order (registering mid-parse corrupts the shared enum: the
    /// `known_type != MAX` guard in fill_database then suppresses the correct
    /// in-order registration → `id × 512` reads).
    /// @PLN25 — whether the E2 vector-element rewrite (`vector<S>` →
    /// `vector<__nullable<S>>`) is active for the CURRENT source.  The native
    /// stdlib (`STD_SOURCE`) always stays DENSE: its `#rust` bodies write the
    /// dense struct ABI (e.g. `fields()` → `vector<JsonField>`), which an E2 wrap
    /// would desync.  E2 applies ABOVE the native layer — user files + libraries.
    /// THE default-on switch lives here (one place): drop the source check / add
    /// an `LOFT_E2_SYNTH` env gate to re-gate.
    pub(crate) fn e2_rewrite_enabled(&self) -> bool {
        // @PLN25 — GATE FLIP LANDED (2026-06-20): nullable-by-default is ON for all user files +
        // libraries; only the native stdlib (`STD_SOURCE`) stays dense.  No `LOFT_E2_SYNTH` env gate
        // any more — the full suite is 2416/2417 (the lone fail is the pre-existing environmental
        // `kernel_port`, unrelated to E2).  The flip tail was cleared: keyed collections reverted to
        // dense + shared-nullable record-sharing (Scope A/B), 86 (bounded-generic dispatch on a
        // nullable element), 371 (forward-ref local-vector synth-enum layout), imaging (native-managed
        // pixel vector marked `not null`); moros_glb was a cache-gotcha artifact.  See
        // single-payload-refactor.md § "Gate-ON flip tail".
        self.data.source != crate::data::STD_SOURCE
    }

    pub(crate) fn e2_nullable_elem(&mut self, elem: Type) -> Type {
        if !self.e2_rewrite_enabled() {
            return elem;
        }
        // Forward-ref `S?`: S is not laid out yet (still `Unknown`), so synth
        // `__nullable<S>` eagerly here — its `Some` payload is `Reference(S)`,
        // which resolves once S is known.  This carries the `?` opt-in through
        // the forward reference, so the deferred resolver (`copy_unknown_fields`)
        // only ever sees a DENSE `vector<S>` as a bare `Unknown` and resolves it
        // dense.  Without this, a forward-ref `vector<S?>` would lose its `?` and
        // resolve dense (silent loss of nullability).  Stdlib elements stay dense
        // (their `#rust` bodies use the dense ABI), matching `nullable_vector_elem`.
        if let Type::Unknown(was) = &elem
            && *was != 0
            && self.data.def(*was).source != crate::data::STD_SOURCE
        {
            let syn = self.data.nullable_enum_for(&mut self.lexer, *was);
            return Type::Enum(syn, true, Deps::none());
        }
        let Type::Reference(struct_d, _) = &elem else {
            // @PLN25 — a SCALAR nullable element (`vector<integer?>`, `vector<float?>`,
            // `vector<text?>`, …) has no `__nullable<S>` synth (that's for a struct /
            // enum ref); it rides the `Optional(τ)` marker instead, which shares the
            // base scalar's dense inline storage + typed-sentinel null (element_size /
            // element_align / type_elm all peel `Optional`). Wrapping here makes the
            // element type nullable, so the index store-check allows `v[i] = null` and
            // the read `v[i]` types as `τ?`. GATED on `pln25_optional_enabled` (unlike
            // the struct `__nullable<S>` synth, which is the default-on vectors-half):
            // gate-OFF a scalar `vector<τ?>` stays `== vector<τ>` (byte-identical),
            // which is behaviourally fine there since a bare scalar vector is still
            // nullable pre-DN1; the `?`↔non-null distinction only bites under DN1.
            if crate::keys::pln25_optional_enabled()
                && matches!(
                    elem,
                    Type::Integer(_)
                        | Type::Float
                        | Type::Single
                        | Type::Boolean
                        | Type::Character
                        | Type::Text(_)
                )
            {
                return Type::optional(elem);
            }
            // loft#1071 / loft#1416 — an ENUM element rides the `Optional` marker for the
            // same reason a scalar does: absence already has a bit pattern in the bytes the
            // slot occupies, so it needs no `__nullable<…>` tag and no extra storage — only
            // for the TYPE to keep saying it may be absent.  Both enum kinds qualify, and
            // each has its own reserved value: a STRUCT-enum slot is a four-byte record
            // pointer whose in-band absent value is `0`, and a VALUE enum is one byte whose
            // variants are numbered from 1 precisely because `0` and `255` are spent
            // (`OpConvBoolFromEnum` reads `@v1 != 255 && @v1 != 0`).  `(L-Null)` covers
            // every type that reserves a null value, and both do.
            //
            // Without the marker the `?` is lost AT THE DECLARATION: the element types as a
            // bare `Shape` / `Color`, so the store refuses `null` while naming
            // `vector<Color>` — a type the author did not write — and a loop binding over it
            // cannot be asked `e == null` at all, because the type no longer admits the
            // question.
            if crate::keys::pln25_optional_enabled() && matches!(elem, Type::Enum(_, _, _)) {
                return Type::optional(elem);
            }
            return elem;
        };
        let struct_d = *struct_d;
        // @PLN25 E2 — a generic `vector<T>` must stay dense: `T` is an opaque
        // type-variable stub (registered as a `DefType::Struct` so `parse_type`
        // resolves it), not a real struct, and nullability is decided at
        // instantiation by whatever concrete element type the caller's vector
        // carries (monomorphization substitutes T's bound type directly — see
        // `substitute_type`).  Rewriting here would bury T inside `__nullable<T>`
        // and break the "T appears in the first parameter" check + the return
        // type unification.
        if struct_d == self.cur_type_var {
            return elem;
        }
        // The eligibility (non-stdlib, non-synthetic struct) and the synth-enum
        // lookup live in ONE home — `typedef::nullable_vector_elem` — shared with the
        // deferred forward-ref resolver (`copy_unknown_fields`), so a `vector<S>`
        // element resolves identically whether `S` was known here or only later.
        // (A STDLIB struct stays DENSE even inside a consumer's collection — its
        // `#rust` bodies write the dense ABI — which the helper's struct-source check
        // enforces.)
        match crate::typedef::nullable_vector_elem(&mut self.data, &mut self.lexer, struct_d) {
            Some(syn) => Type::Enum(syn, true, Deps::none()),
            None => elem,
        }
    }

    /// @PLN86 2.4 — does this field/index write LHS target HOST data (reject) vs
    /// the script's OWN data (allow)?  HOST when the base root is a PARAMETER, or
    /// its type is anything but a script-defined struct — a host-library struct
    /// (which also catches aliasing, since `x: Player = player` is typed `Player`
    /// regardless of the value), a vector/scalar, or an unresolvable base.  A
    /// non-parameter local of a script-defined struct type is the script's own:
    /// mutable.  Conservative — when ownership can't be proven script, treat as
    /// host.  A struct type is "script-defined" iff its library is NOT a host
    /// allow-listed one (the script's own types live in the program's source,
    /// which is never `allow_libs`).
    fn raw_write_is_host_owned(&self, lhs: &Value) -> bool {
        let Some(root) = lhs_root_var(lhs) else {
            return true;
        };
        // A PARAMETER root is host data — `v[i] = …` / `e.f = …` on a parameter mutates the
        // CALLER's value (proven: `fn f(v){ v[0]=99 }` leaves the caller's `orig[0]==99`).
        let args = self.vars.arguments();
        if args.contains(&root) {
            return true;
        }
        match self.vars.tp(root) {
            // A script-defined struct LOCAL is the mod's own (mutable); a host-library struct
            // local (or one the profile does not include) is host — the TYPE catches aliasing
            // like `x = player; x.health = …`.
            Type::Reference(struct_def, _) => {
                let Some(lib) = crate::sandbox::def_library(&self.data, *struct_def) else {
                    return true;
                };
                let profile = self
                    .def_sandbox
                    .get(&self.context)
                    .and_then(|n| self.sandbox.profiles.get(n));
                profile.is_none_or(|p| p.allows_lib(&lib))
            }
            // @PLN86 D-cap-3 + @PLN102 F6 — a NON-parameter local VECTOR is script-owned, so
            // `v[i] = e` is `Cap-Own` — EXCEPT one that ALIASES a parameter through a `&`-bind
            // (`r = &v`, even transitively `b = &a; a = &v`).  Such a local is TYPED `Vector`
            // yet a write through it reaches the caller's vector (proven: `r = &v; r[0] = 99`
            // ⇒ `v[0] == 99` — the earlier "even `r = &v` copies" premise was FALSE).  A
            // genuine copy (`c = v`, a literal, a slice) deps on a FRESH local store and never
            // reaches an argument.  Follow the dep CHAIN: a vector whose deps reach a parameter
            // is host; a genuinely-owned one is script.
            Type::Vector(..) => self.root_aliases_argument(root, &args),
            // A `&`/`RefVar` borrow, a scalar, or an unresolvable base → host, conservatively.
            _ => true,
        }
    }

    /// True when a local vector `root` ALIASES a parameter through a `&`-bind chain — a write
    /// through it mutates the caller's vector, so it is host, not script-owned.  Follows the
    /// dep chain (`b` deps on `a`, `a` on the param `p`); a genuine copy deps only on a fresh
    /// local store and never reaches an argument.  @PLN102 F6 (closes the `r = &param` launder).
    fn root_aliases_argument(&self, root: u16, args: &[u16]) -> bool {
        let mut stack = vec![root];
        let mut seen: Vec<u16> = Vec::new();
        while let Some(v) = stack.pop() {
            if seen.contains(&v) {
                continue;
            }
            seen.push(v);
            if args.contains(&v) {
                return true;
            }
            for d in self.vars.tp(v).depend() {
                stack.push(d);
            }
        }
        false
    }

    // <assign> ::= <operators> [ '=' | '+=' | '-=' | '*=' | '%=' | '/=' <operators> ]
    #[allow(clippy::too_many_lines)]
    /// @PLN102 F2 — does this IR contain a call to a non-builtin (a user fn `n_*`
    /// or method `t_*`) — i.e. a potentially side-effecting / non-idempotent
    /// sub-expression?  Builtin `Op*` accessors/arithmetic are pure given stable
    /// args and may be re-evaluated freely; a place addressing sub-expression that
    /// reaches a user call must be bound once (compound-assign place-once, C92).
    pub(crate) fn ir_has_user_call(&self, v: &Value) -> bool {
        match v {
            Value::Call(d, args) => {
                !self.data.def(*d).name.starts_with("Op")
                    || args.iter().any(|a| self.ir_has_user_call(a))
            }
            // A dynamic call through a fn-ref is always a user call (non-idempotent).
            Value::CallRef(_, _) => true,
            // Every other shape descends through the keystone, so a call the callers must
            // not re-evaluate cannot hide under a wrapper nobody thought to name.  A lifted
            // container is the shape that reaches here: `getv(v)[0]` puts the call inside a
            // `Block("inline_container", [Set(tmp, getv(v)), Var(tmp)])` and hands that to
            // `OpGet…` as the accessor.  Answering FALSE there let @PLN102 F2 skip the
            // once-only hoist, and the accessor's call ran twice.
            other => {
                let mut found = false;
                other.for_each_child(&mut |c| {
                    if !found {
                        found = self.ir_has_user_call(c);
                    }
                });
                found
            }
        }
    }

    /// The ONE home for "is the identifier the lexer is parked on a BINDING
    /// occurrence?" — a name being declared, rather than a name being read.
    ///
    /// A binding name always wins over a definition of the same name: `len = 5`
    /// and `trim = 7` are ordinary locals even though `len` and `trim` are
    /// stdlib functions.  Two shapes bind, and before loft#756 only the first
    /// was recognised:
    ///
    /// * `name = …` — the next token is `=` (never `==`).
    /// * the TYPED local `name: T = …`, where the next token is the `:` of the
    ///   annotation.  loft#1079 — this arm was written into the doc above from
    ///   the day loft#756 was closed, but only ONE of the three call sites
    ///   actually peeked the `:` (@P392, in the bare-function-reference path of
    ///   `parse_var`).  The site above it — the flat `def_nr(name)` lookup —
    ///   never did, so a `both:` stdlib function, which registers a `Dynamic`
    ///   definition under its RAW spelling as well as `n_<name>`, matched there
    ///   and returned before the `:`-aware site was reached.  `exp: integer = 5`
    ///   then left the `:` unconsumed and the user saw *"Expect token ;"* on a
    ///   line whose syntax is correct.  A `self:` method (no `n_`/raw entry) and
    ///   a plain global (no raw entry) both bound fine, which is why the failure
    ///   read as "stdlib names are reserved" when the real axis is how the
    ///   receiver is declared.
    /// * an element of a tuple destructuring, `(a, trim) = pair()`, where the
    ///   next token is the `,` or `)` of the LHS list.  Missing this made the
    ///   two assignment forms disagree about what a legal binding name is:
    ///   `trim` resolved to the definition, the element was not a plain
    ///   variable, and the user got *"Tuple destructuring requires plain
    ///   variable names"* about a name that is exactly that.
    ///
    /// `in_tuple_lhs` is only ever set for a `( … ) =` statement, so the `,`
    /// and `)` arms cannot fire on an ordinary parenthesised expression.
    ///
    /// The `:` arm excludes `::` and `:=`, which START with a colon and bind
    /// nothing — the same pair [`crate::lexer::Lexer::peek_named_arg`] excludes
    /// when it decides whether an `ident :` opens a named argument.  A named
    /// argument is the other `ident :` that is not a binding, and it never
    /// reaches here: the argument-list parser consumes the name first.
    pub(crate) fn at_binding_name(&self) -> bool {
        (self.lexer.peek_token("=") && !self.lexer.peek_token("=="))
            || (self.lexer.peek_token(":")
                && !self.lexer.peek_token("::")
                && !self.lexer.peek_token(":="))
            || (self.in_tuple_lhs && (self.lexer.peek_token(",") || self.lexer.peek_token(")")))
    }

    /// Look ahead for a tuple-destructuring LHS: a statement opening with `(`
    /// whose matching `)` is followed by `=`.  Pure lookahead — the lexer is
    /// reverted to where it started, so nothing is parsed twice.
    ///
    /// The scan accepts ONLY what such an LHS can contain — names, commas and
    /// nesting — and gives up the moment it meets anything else.  That bound is
    /// load-bearing, not tidiness: the lexer carries state across a format
    /// string, which `revert` does not restore, so a lookahead that walked into
    /// one desynced the real parse.  `(s.value, "v{s.value}")` is an ordinary
    /// tuple expression and is rejected on the `.`, long before the string.
    fn peek_tuple_lhs(&mut self) -> bool {
        if !self.lexer.peek_token("(") {
            return false;
        }
        let lnk = self.lexer.link();
        self.lexer.cont(); // step over "("
        let mut depth: u32 = 1;
        let mut names_only = true;
        while depth > 0 {
            if self.lexer.peek_token("(") {
                depth += 1;
            } else if self.lexer.peek_token(")") {
                depth -= 1;
            } else if !self.lexer.peek_token(",")
                && !matches!(self.lexer.peek().has, LexItem::Identifier(_))
            {
                names_only = false;
                break;
            }
            self.lexer.cont();
        }
        let binds = names_only && self.lexer.peek_token("=") && !self.lexer.peek_token("==");
        self.lexer.revert(lnk);
        binds
    }

    /// Parse an assignment, keeping [`Parser::last_place_discharge`] the answer for the
    /// left-hand side that is being parsed HERE.
    ///
    /// The flag records which of the two spellings built the last discharge — a postfix `x?`,
    /// which names a place, or an explicit `(a ?? d)`, which names two values and none
    /// (`@FR-E-Asgn-Discharge`).  `parse_assign_inner` clears it before parsing its own left
    /// side so an earlier statement's answer cannot be read as this one's, and a left side
    /// re-enters that function for every sub-expression it contains — an index (`h?[k]`), a
    /// call argument.  A nested clear therefore erased the answer the OUTER left side had
    /// already recorded: `b.d? += […]` survived because nothing is parsed after its `?`,
    /// while `h?[k] = v` had its `?` forgotten by the time the place was judged, so the
    /// place read as an explicit coalesce and was refused (loft#1214).
    ///
    /// Restoring on the way out confines each nesting level to its own answer: the inner
    /// parse cannot leak one outward, and cannot destroy the one already standing.
    /// Does this type carry a `text` anywhere inside a tuple — at the top level or through
    /// a NESTED tuple member?
    ///
    /// The question is about TRANSPORT, not about the top-level shape: a tuple argument is
    /// passed with borrowed text elements however deeply they sit, so `((integer, text), …)`
    /// needs the owning promotion exactly as `(integer, text)` does (loft#1278, and the same
    /// one-level-in fact loft#1005 had to learn on the read side).
    fn tuple_carries_text(tp: &Type) -> bool {
        match tp.base() {
            Type::Tuple(members) => members
                .iter()
                .any(|m| matches!(m.base(), Type::Text(_)) || Self::tuple_carries_text(m)),
            _ => false,
        }
    }

    /// Does `hay` WRITE INTO `needle` — is there a mutating call in its tree whose target
    /// (first argument) is that exact place?
    ///
    /// Asked of a right-hand side and its destination, this is *"does this expression
    /// construct in place?"* — the shape a vector literal takes when the destination is a
    /// collection it can build straight into.  Such an RHS needs the clear BEFORE it and no
    /// append after it (loft#1279).
    ///
    /// ⚠ *Writes into*, not *mentions*.  Asked as "does the RHS name the destination
    /// anywhere?", this also answers yes for a comprehension that READS its own destination
    /// (`s.v = [… for x in s.v]`, loft#1195) — which builds a fresh vector and needs the
    /// ordinary clear-then-append.  Treating that as build-in-place clears the source before
    /// the comprehension reads it and assigns nothing back: seven cells of loft#1195's guard
    /// answered `[]`.  The mutating-op test is what separates reading the destination from
    /// filling it, and it shares [`crate::parser::op_writes_first_arg`] with the two mutation
    /// walkers so the three cannot drift.
    fn value_writes_into(&self, hay: &Value, needle: &Value) -> bool {
        let hay = hay.unspan();
        if let Value::Call(d, args) = hay
            && (*d as usize) < self.data.definitions.len()
            && crate::parser::op_writes_first_arg(self.data.def(*d).name())
            && let Some(first) = args.first()
            && *first.unspan() == *needle.unspan()
        {
            return true;
        }
        match hay {
            Value::Call(_, args)
            | Value::Insert(args)
            | Value::Tuple(args)
            | Value::Parallel(args)
            | Value::CallRef(_, args) => args.iter().any(|a| self.value_writes_into(a, needle)),
            Value::Block(b) | Value::Loop(b) => b
                .operators
                .iter()
                .any(|o| self.value_writes_into(o, needle)),
            Value::If(c, t, e) => {
                self.value_writes_into(c, needle)
                    || self.value_writes_into(t, needle)
                    || self.value_writes_into(e, needle)
            }
            Value::Set(_, v)
            | Value::Return(v)
            | Value::Drop(v)
            | Value::Yield(v)
            | Value::TuplePut(_, _, v) => self.value_writes_into(v, needle),
            Value::Iter(_, a, b, c) => {
                self.value_writes_into(a, needle)
                    || self.value_writes_into(b, needle)
                    || self.value_writes_into(c, needle)
            }
            _ => false,
        }
    }

    pub(crate) fn parse_assign(&mut self, code: &mut Value) -> Type {
        let outer_discharge = self.last_place_discharge;
        let tp = self.parse_assign_inner(code);
        self.last_place_discharge = outer_discharge;
        tp
    }

    fn parse_assign_inner(&mut self, code: &mut Value) -> Type {
        let mut parent_tp = Type::Null;
        // @PLN87 D-bind-7 — does THIS statement begin with a prefix `&`?  No valid
        // statement does: `&` is only ever a binding RHS (`x = &a`) or a type
        // annotation, both AFTER a name.  Captured before the parse so a nested
        // parse (`&(1+2)` re-enters parse_assign for the inner `1+2`, which begins
        // with `1`) doesn't see the outer `&` — `amp_pending` is a global flag that
        // would otherwise leak into it.  `&&` is its own token, so this never
        // mis-fires on logical-and.  The start position also points the caret below
        // at the `&` (the cursor has drifted to `;`/`}` by detection time).
        let stmt_start_pos = self.lexer.peek_pos().clone();
        let started_with_amp = self.lexer.peek_token("&");
        // loft#756 — mark the names in a `( … ) =` LHS as bindings for the whole
        // LHS parse.  Only ever SET here (never cleared): a nested parse_assign
        // inside the list must not un-mark the elements around it.  Restored
        // below, so the RHS — parsed further down — sees the outer state again.
        let saved_tuple_lhs = self.in_tuple_lhs;
        if self.peek_tuple_lhs() {
            self.in_tuple_lhs = true;
        }
        // @PLN87 B-Ref-AnnotationOnly — a statement may BEGIN with `&` in exactly one
        // shape, `name = &src`, whose `&` is reached through the RHS head opened in
        // `parse_assign_op`.  Open the head here too so a bare `&a;` / block-final
        // `{ &a }` still reaches the D-bind-7 guard below with its own message,
        // instead of being reported here as a sub-expression use.
        self.amp_head = if started_with_amp {
            AmpHead::AssignRhs
        } else {
            AmpHead::No
        };
        // loft#1205 — only a discharge built by THIS left-hand side may be peeled below,
        // so the flag starts clear rather than carrying an earlier statement's answer.
        self.last_place_discharge = false;
        let mut f_type = self.parse_operators(&Type::Unknown(0), code, &mut parent_tp, 0);
        self.amp_head = AmpHead::No;
        self.in_tuple_lhs = saved_tuple_lhs;
        if let (Type::RefVar(_), Value::Var(v_nr)) = (&f_type, &code) {
            self.vars.in_use(*v_nr, true);
        }
        // Type annotation: `v: type = expr`
        // Only attempt outside format-string expressions (where `:` is used for
        // format specifiers like `{c:#}`).  Consume `: type` only when `=`
        // follows, confirming this is an annotated declaration.
        // A capture a closure MUTATES is BOXED (plan-22 02d-iii): the local's type
        // becomes `Reference(__cell_<T>)` and `auto_deref_boxed_scalar` rewrites every
        // occurrence of the name into `OpGet<T>(Var, 0)` — the DECLARATION's own
        // occurrence included.  That shape is the boxed local's place (`towards_set`
        // maps it straight back to `OpSet<T>`), so the annotation is recognised through
        // it as well.  Asking only for a bare `Value::Var` left the `:` unconsumed and
        // reported the well-formed `t: integer = 0` as a missing `;`, on a line whose
        // only fault was that a closure further down assigned `t` (loft#1231).
        let annotated_var = match code {
            Value::Var(v_nr) if self.vars.exists(*v_nr) => Some(*v_nr),
            _ => self.extract_boxed_var_from_lhs(code).filter(|&v_nr| {
                self.vars.exists(v_nr)
                    && crate::parser::vectors::boxed_cell_def(self.vars.tp(v_nr), &self.data)
                        .is_some()
            }),
        };
        if let Some(v_nr) = annotated_var
            && !self.in_format_expr
            && self.lexer.peek_token(":")
        {
            let lnk = self.lexer.link();
            self.lexer.cont(); // consume ":"
            // @PLN87 #2 — a `&` in the declared type (`b: &T = src`) makes `b` a
            // reference to the addressable `src` — the same as `b = &src`, just with
            // the `&` on the type instead of the value.  Mirror the param parser.
            // @F21 — references &T (parameters + write-back bindings)
            let is_ref = self.lexer.has_token("&");
            // @PLN40 const-model — `x: const T` before the type = value-const: a
            // read-only borrow of the value (mutation through `x` is rejected; a
            // rebind is allowed).  Set the flag AFTER the type parse confirms this
            // is a real annotation (`= …` follows), mirroring the param parser.
            let is_value_const = self.lexer.has_keyword("const");
            let mut got_annotation = false;
            if let Some(tp) = self.parse_type_full(u32::MAX, false)
                && self.lexer.peek_token("=")
            {
                // @PLN25 E2/E3 — the nullable-element rewrite now happens at the
                // vector-type-resolution chokepoint (definitions.rs `sub_type`
                // `vector` arm), so a `vector<S>` annotation already arrives
                // rewritten; no per-site hook here.
                let tp = if is_ref {
                    // A `&`-linked vector LOCAL is the SHARED `DbRef`, not a stack link:
                    // `pe = &e` gives the variable the plain vector type and aliases
                    // through it, so the annotated spelling gives it the same type rather
                    // than a `RefVar` the bind never builds.  Wrapped, the annotation
                    // described a link that was not there and every read of `pe` derefed a
                    // vector buffer as a stack ref — an interpreter panic (loft#1371).  A
                    // `&vector` PARAMETER keeps its `RefVar`: there the link IS the call
                    // ABI.  `vector<T>?` is deliberately not matched here, so it still
                    // reaches the nullable-link refusal below (loft#1372).
                    if matches!(tp, Type::Vector(_, _)) {
                        tp
                    } else {
                        // D-tup-2 — the SAME admitted-element gate the signature uses;
                        // before it was shared, a `&(text, text)` LOCAL sailed past the
                        // refusal a `&(text, text)` PARAMETER got and reached codegen as
                        // an internal compiler error.
                        self.ref_var_type(tp)
                    }
                } else {
                    tp
                };
                self.change_var_type(v_nr, &tp);
                // (I-Join) — an EXPLICIT `: Type` annotation pins the variable's type, so
                // it stays constrained (a wider write is a narrowing error).  An inferred
                // local (no annotation) widens to the join instead (see parse_assign_op).
                self.vars.set_annotated(v_nr);
                if is_value_const {
                    self.vars.set_value_const(v_nr);
                }
                f_type = tp;
                got_annotation = true;
                // @PLN87 #2 — `b: &T = src` IS `b = &src`: flag the reference bind so
                // the scalar-reference lowering (the `amp_pending` path in
                // `parse_assign_op`) fires.  A later non-annotated `b = x` carries no
                // flag, so it correctly writes THROUGH the reference instead.
                if is_ref {
                    self.amp_pending = true;
                }
            }
            if !got_annotation {
                self.lexer.revert(lnk);
            }
        }
        // T1.2: LHS tuple destructuring — (a, b) = expr
        // P194: tuple-typed field reassignment — `p.v = (...)` where
        // v is a tuple-typed field.  `get_val::Type::Tuple` returns
        // `Value::Tuple([reads])` for a tuple field read; the reads
        // are `OpGet*(host_ref, base_pos + element_offset[i])`.  When
        // the LHS shape is "tuple of reads (not all Var)" AND f_type
        // is `Type::Tuple`, route to `emit_tuple_set_ops` instead of
        // the destructuring branch.
        if let Value::Tuple(vars) = code.unspan()
            && self.lexer.has_token("=")
        {
            if let Type::Tuple(elems) = &f_type
                && !vars.is_empty()
                && !vars.iter().all(|v| matches!(v, Value::Var(_)))
                && let Some((host_ref, first_pos)) = leaf_tuple_lhs(&vars[0])
            {
                let elems_vec = elems.clone();
                let tuple_d_nr = self.data.tuple_def(&mut self.lexer, &elems_vec);
                let offsets: Vec<u16> = crate::data::stored_tuple_offsets_for_def(
                    &self.data,
                    &self.database,
                    tuple_d_nr,
                    elems_vec.len(),
                )
                .unwrap_or_else(|| {
                    crate::data::element_stack_offsets(&elems_vec)
                        .into_iter()
                        .map(|x| x as u16)
                        .collect()
                });
                let host_field_pos = (first_pos as u16).saturating_sub(offsets[0]);
                let mut rhs = Value::Null;
                let _rhs_type = self.expression(&mut rhs);
                let ops = self.emit_tuple_set_ops(&host_ref, host_field_pos, &elems_vec, rhs);
                *code = crate::data::v_block(ops, Type::Void, "tuple_field_set_via_assign");
                return Type::Void;
            }
            let var_nrs: Vec<u16> = vars
                .iter()
                .filter_map(|v| {
                    if let Value::Var(nr) = v {
                        Some(*nr)
                    } else {
                        None
                    }
                })
                .collect();
            if var_nrs.len() != vars.len() {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Tuple destructuring requires plain variable names"
                );
            }
            let mut rhs = Value::Null;
            let rhs_type = self.expression(&mut rhs);
            // A7.1: accept both `Type::Tuple([…])` and the synthetic
            // `Reference(__tuple<…>)` shape that A7.1's parse_function
            // gate widen produces for tuple returns wider than 8B.
            // For the synthetic-struct shape, element reads go through
            // `get_val` with the struct's per-attribute offset — same
            // path P189b's `.0` / `.1` element access takes (mirrors
            // the for-loop destructure in collections.rs:1289-1304).
            let (rhs_elems_opt, ref_def_nr): (Option<Vec<Type>>, u32) = match &rhs_type {
                Type::Tuple(elems) => (Some(elems.clone()), u32::MAX),
                Type::Reference(d_nr, _) if self.data.def(*d_nr).name().starts_with("__tuple<") => {
                    let elems: Vec<Type> = self
                        .data
                        .def(*d_nr)
                        .attributes
                        .iter()
                        .map(|a| a.typedef.clone())
                        .collect();
                    (Some(elems), *d_nr)
                }
                _ => (None, u32::MAX),
            };
            if let Some(rhs_elems) = rhs_elems_opt {
                if rhs_elems.len() != var_nrs.len() {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Tuple arity mismatch: left has {} names, right has {} elements",
                        var_nrs.len(),
                        rhs_elems.len()
                    );
                }
                // `@FR-B-Copy` / `@FR-B-View-Base` — a COLLECTION member read off an OWNED
                // tuple local COPIES (`a = t.0` is `af = bx.v`, one level off an owned base), a
                // struct member VIEWS (`@FR-B-View`), and off a BORROWED base — a parameter, a
                // linked local — every member views.  Decided on the source before the temp
                // takes its place: the temp is never an argument. (loft#1361)
                let owned_base = matches!(rhs.unspan(), Value::Var(s)
                    if !self.vars.is_argument(*s) && matches!(self.vars.tp(*s), Type::Tuple(_)));
                // T1.4: create a temp variable for the RHS tuple, then read elements.
                let tmp_tp = rhs_type.clone();
                let tmp = self.vars.work_refs(&tmp_tp, &mut self.lexer);
                if !self.first_pass {
                    self.change_var_type(tmp, &tmp_tp);
                }
                let mut steps = vec![Value::Set(tmp, Box::new(rhs))];
                for (i, &v_nr) in var_nrs.iter().enumerate() {
                    // The arity mismatch above is already an Error, but it only
                    // REPORTED — lowering carried on and indexed the tuple with an
                    // LHS position it does not have (loft#959).  A scalar tuple got
                    // away with it: `ref_def_nr == u32::MAX` takes the `TupleGet`
                    // branch, which indexes nothing.  Widen the tuple past 8B — one
                    // `vector<T>` element is enough — and it becomes the synthetic
                    // `__tuple<…>` struct, where both `rhs_elems[i]` and `offs[i]`
                    // are real indexes: `(a, b, d, e, f) = <4-tuple>` then panicked
                    // with "the len is 4 but the index is 4" and the user got an ICE
                    // instead of the two errors the scalar form prints.
                    //
                    // Stop at the shorter side.  In a well-formed destructuring the
                    // two lengths are equal and this never fires; in a broken one the
                    // compile is already failing, so a partial lowering is never run.
                    if i >= rhs_elems.len() {
                        break;
                    }
                    // @FR-N-Decl / @FR-N-Store — a DECLARED name in the pattern keeps its
                    // type, and a member that may be null is asked through the store face on
                    // the member's read, exactly as the assignment seam asks for a plain
                    // local (`parse_assign_op_inner`): a WARNING at full width, an ERROR at a
                    // narrow one.  The retype below sees the peeled type, so it never refuses
                    // what the rule only warns about; an inferred name widens to the join.
                    // `@FR-L-Null-Which` — a tagged `__nullable<S>` member is read through the tag
                    // into the local, as the plain bind in `parse_assign_op_inner` does: the
                    // local's spelling is the pointer.
                    let tagged_member = self.tagged_pointer_type(&rhs_elems[i]);
                    let elem_tp = match &tagged_member {
                        Some((_, pointer)) => pointer.clone(),
                        None => rhs_elems[i].clone(),
                    };
                    let declared_nullable_member = self.vars.exists(v_nr)
                        && self.author_declared(v_nr)
                        && matches!(elem_tp, Type::Optional(_))
                        && !matches!(self.vars.tp(v_nr), Type::Optional(_) | Type::RefVar(_))
                        && !self.vars.tp(v_nr).is_unknown()
                        && self.vars.tp(v_nr).is_equal(elem_tp.base());
                    let member_tp = if declared_nullable_member {
                        elem_tp.base().clone()
                    } else {
                        elem_tp.clone()
                    };
                    let member_what = if declared_nullable_member {
                        format!(
                            "{} `{}`",
                            if self.vars.is_argument(v_nr) {
                                "the parameter"
                            } else {
                                "the local"
                            },
                            self.vars.name(v_nr)
                        )
                    } else {
                        String::new()
                    };
                    if self.vars.exists(v_nr) {
                        self.vars.defined(v_nr);
                        self.change_var_type(v_nr, &member_tp);
                    }
                    let step = if ref_def_nr == u32::MAX {
                        let mut read = Value::TupleGet(tmp, i as u16);
                        if let Some((syn, _)) = tagged_member {
                            read = self.emit_nullable_slot_read(syn, read, &rhs_elems[i]);
                        }
                        if declared_nullable_member {
                            self.convert_store(&mut read, &elem_tp, &member_tp, &member_what, None);
                        }
                        if owned_base
                            && Self::is_collection_type(rhs_elems[i].base())
                            && let Some(owned) =
                                self.tuple_member_owned_copy(&mut read, &rhs_elems[i])
                            && self.vars.exists(v_nr)
                        {
                            self.change_var_type(v_nr, &owned);
                        }
                        Value::Set(v_nr, Box::new(read))
                    } else {
                        let elem_offset = if let Some(offs) =
                            crate::data::stored_tuple_offsets_for_def(
                                &self.data,
                                &self.database,
                                ref_def_nr,
                                rhs_elems.len(),
                            ) {
                            u32::from(offs[i])
                        } else {
                            crate::data::element_stack_offsets(&rhs_elems)[i] as u32
                        };
                        // When destructuring a synthetic `__tuple<...>` struct (the
                        // wider-than-8B tuple-return shape), reading an element gives
                        // a VIEW into the tmp's storage: `OpGetField(tmp, offset, ...)`
                        // answers a DbRef sharing tmp's store_nr and rec.  That view
                        // must not become the binding, because tmp belongs to the CALL
                        // SITE, not to the binding — see `materialize_tuple_element`.
                        let mut view = self.get_val(
                            &rhs_elems[i],
                            false,
                            elem_offset,
                            Value::Var(tmp),
                            u32::MAX,
                        );
                        if let Some((syn, _)) = tagged_member {
                            view = self.emit_nullable_slot_read(syn, view, &rhs_elems[i]);
                        }
                        if declared_nullable_member {
                            self.convert_store(&mut view, &elem_tp, &member_tp, &member_what, None);
                        }
                        self.materialize_tuple_element(v_nr, tmp, &elem_tp, view)
                    };
                    steps.push(step);
                }
                *code = Value::Insert(steps);
            } else if !self.first_pass {
                // A NULLABLE tuple is a tuple, and saying it is not sends the author looking at
                // the wrong half of their program: `(a, b) = v[i]` is `(N-Index)`'s `τ?`, which
                // has no representation to destructure (`formal/types-history.md` D-Opt-NoNull),
                // and the cure is the same discharge the member read names (loft#1423).
                if let Some(elems) = self.nullable_tuple_elems(&rhs_type) {
                    let spelled = rhs_type.name(&self.data);
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "cannot destructure `{spelled}` — the tuple may be absent, and an \
                         absent tuple has no members; discharge it first (`(a, b) = t?`, or \
                         `(a, b) = t ?? (…)`)"
                    );
                    // Bind the targets to the member types anyway, so the refusal is the ONLY
                    // report: left undefined, every later use of a destructured name came back
                    // as "Unknown variable", burying the reason under one error per name.
                    for (v_nr, tp) in var_nrs.iter().zip(elems) {
                        self.vars.set_type(*v_nr, tp);
                        self.vars.defined(*v_nr);
                    }
                } else {
                    diagnostic!(
                        self.lexer,
                        Level::Error,
                        "Cannot destructure a non-tuple value"
                    );
                }
            }
            return Type::Void;
        }
        // T1.4-fix-a + P248 — tuple element assignment `t.<chain> = expr`,
        // including nested `t.0.1 = expr`.
        //
        // Plan-07 phase 1: unspan() so the wrap on `.` (step 1.12) does
        // not hide the TupleGet shape.  For depth ≥ 2 the parser
        // materialised the read as `Block[Set(w0, TupleGet(t, 0)),
        // TupleGet(w0, 1)]` (operators.rs case 3 — temporary tuple
        // work var).  Without P248's recursive extractor that Block
        // landed at the assignment dispatcher, didn't match the
        // single-level TupleGet arm, and fell through to the
        // "Not implemented operation = for type integer" diagnostic.
        if let Some(lhs) = extract_nested_tuple_lhs(code)
            && self.lexer.has_token("=")
        {
            let mut rhs = Value::Null;
            // loft#1067's member-ASSIGNMENT position.  The declared type of the member being
            // written is an inference context exactly as the tuple LITERAL's member is, so
            // `t.0 = |x| { x + 1 }` should read like `t = (|x| { x + 1 }, 1)` — before this
            // it was the only fn-ref spelling of the statement that did not work, refused
            // with "Cannot infer type for lambda parameter" while the long form beside it
            // compiled.  The container is the root's tuple for `t.0 = …` and the deepest
            // work var's for a chained `t.0.1 = …`, which is the same walk
            // `build_nested_tuple_assign` writes through.
            //
            // Touch the `⇐` channel ONLY where the destination really does name a `fn(…)`
            // member — the scoping loft#1069 paid a round trip to learn, because an
            // expression that merely passes through here INHERITS the ambient expectation
            // and clearing it unconditionally silently retypes one.
            let container = match lhs.chain.last() {
                Some((w, _)) => self.vars.tp(*w).clone(),
                None => self.vars.tp(lhs.root).clone(),
            };
            let member_tp = match container.base() {
                Type::Tuple(ms) => ms.get(lhs.leaf_idx as usize).cloned(),
                _ => None,
            };
            let member_for_null = member_tp.clone();
            let seeding = member_tp.as_ref().is_some_and(Self::seeds_lambda_hint);
            let saved_expected = if seeding {
                let m = member_tp
                    .expect("seeding implies a member type")
                    .base()
                    .clone();
                Some(std::mem::replace(&mut self.expected, m))
            } else {
                None
            };
            let rhs_tp = self.expression(&mut rhs);
            if let Some(prev) = saved_expected {
                self.expected = prev;
            }
            // loft#1282 — `t.1 = null` has to become the ELEMENT TYPE's null sentinel, the
            // same `OpConv<T>FromNull()` a struct field write emits.  Left as a bare
            // `Value::Null`, the value generator pushed NOTHING and the `OpPut<T>` below it
            // popped whatever sat beneath on the eval stack: `b: (integer, integer?)` read
            // back an address-shaped number, a `text?` element came back holding part of the
            // format template, and neighbouring shapes reached `Incorrect var` / `var_pos
            // underflow` / `attempt to subtract with overflow` in codegen.  Corruption, not a
            // wrong answer.
            //
            // Unconditional, exactly as the struct-field path is: whether the member is
            // DECLARED nullable is a separate question that `(N-Store)` already answers with
            // its own warning, and storing the sentinel is right either way.
            if let Some(member) = member_for_null.as_ref() {
                // loft#1284 — `(N-Store)` covers the direct store, the field, the
                // call-argument site and the branch join, and a TUPLE ELEMENT reached none of
                // them: this branch returns before the general assign path that asks.  So
                // `s.i = null` on a non-null field warned while `c.1 = null` on a non-null
                // element said nothing, for the same store into the same kind of slot.
                self.n_store_violation(&rhs_tp, member, "the tuple element", None);
                // loft#1282 — and the null itself becomes the ELEMENT TYPE's sentinel.  The
                // warning above is about whether the slot SHOULD hold null; this is what
                // makes it hold null rather than whatever the eval stack had.
                if matches!(rhs_tp, Type::Null) && !matches!(member.base(), Type::Null) {
                    self.convert(&mut rhs, &Type::Null, member);
                }
            }
            // loft#1278 — a by-value tuple PARAMETER carrying text is promoted to an owned
            // shadow local the first time an element is written, which is the same move a
            // plain `text` argument already makes (`__tp_<name>`, seeded at function entry).
            //
            // A tuple ARGUMENT is passed BORROWED — `--native` lowers it with `&str`
            // elements, the argument-passing representation TUPLES.md describes — and
            // nothing gave the callee an owned element when it wrote to one.  A literal
            // write tried to store a `String` into that `&str` slot (E0308) and a variable
            // write was refused by borrowck for the same reason, while `--interpret` gave
            // the copy semantics the reference promises.  Reading was fine, the integer
            // element was fine, and a tuple LOCAL was fine: the write to a text element
            // THROUGH the parameter is the whole of it.
            //
            // The promotion is what makes the two backends agree, and it agrees with the
            // documented meaning rather than papering over it: `(F-ParamScalar)` gives a
            // value parameter its own copy, so writing the callee's copy is exactly right
            // and the caller's tuple is untouched either way.
            let mut lhs = lhs;
            if self.first_pass
                && self.vars.is_argument(lhs.root)
                && Self::tuple_carries_text(self.vars.tp(lhs.root))
            {
                let name = self.vars.name(lhs.root).to_string();
                let tp = self.vars.tp(lhs.root).clone();
                let shadow = self
                    .vars
                    .add_variable(&format!("__tp_{name}"), &tp, &mut self.lexer);
                self.vars.set_promoted_from(shadow, lhs.root);
                // The promoted local inherits the const axis, so the const guard still
                // fires on it — the same pairing the text promotion keeps (@PLN40).
                if self.vars.is_value_const(lhs.root) {
                    self.vars.set_value_const(shadow);
                }
                if self.vars.is_const_binding(lhs.root) {
                    self.vars.set_const_binding(shadow);
                }
                self.vars.remap_name(&name, shadow);
                lhs.root = shadow;
            }
            *code = build_nested_tuple_assign(code, &lhs, rhs);
            return Type::Void;
        }
        // T1.11b: compound assignment on a tuple LHS is not supported.
        // (a, b) = expr is handled above; (a, b) += expr has no defined semantics.
        // Return early in both passes to prevent downstream "No matching operator" errors.
        // Consume the operator and RHS so the parser state stays clean after the early exit.
        if matches!(code, Value::Tuple(_))
            && ["+=", "-=", "*=", "%=", "/="]
                .iter()
                .any(|op| self.lexer.peek_token(op))
        {
            if !self.first_pass {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "compound assignment is not supported for tuple destructuring — use (a, b) = expr instead"
                );
            }
            for op in ["+=", "-=", "*=", "%=", "/="] {
                if self.lexer.has_token(op) {
                    break;
                }
            }
            let mut discard = Value::Null;
            self.expression(&mut discard);
            return Type::Void;
        }
        let mut to = code.clone();
        for op in ["=", "+=", "-=", "*=", "%=", "/="] {
            if self.lexer.has_token(op) {
                // loft#1212 — an EXPLICIT `??` coalesce is not a place.  `(E-Asgn-Discharge)`
                // (@FR-E-Asgn-Discharge) says so in as many words: *"an explicit `(a ?? d)`
                // names two values and no place; it takes no assignment at all"*, and
                // `(N-Default)` says why — on the absent path the coalesce answers a FRESHLY
                // CONSTRUCTED default, so a write through it lands somewhere nothing can read
                // back.  The rule was written and the site was missing, so the statement fell
                // through to the machinery below and produced four different wrong answers,
                // all on both backends: a present vector field was appended to ITSELF and its
                // keyed sibling never re-indexed (`vec=4 hash=1` where the group holds two),
                // a null one lost the write in silence, a `text` target reached codegen
                // through a minted work variable and raised an ICE, and a scalar was turned
                // away by arithmetic dispatch — *"Not implemented operation + for type
                // integer"* — a message about the operator when the target is what is wrong.
                //
                // Refused HERE, at the one point every assignment form still shares a target,
                // because each per-type path below answers its own way and `assign_var_nr`
                // mints the text work variable that turns the `text` face into an ICE.  The
                // postfix `x?` is the OTHER branch of this gate and is a place: it names one
                // slot and says what to READ when that slot is null, so it peels below.
                // Consume the right-hand side so the statement is finished before returning,
                // matching the tuple-compound refusal above.
                if !self.last_place_discharge
                    && Self::place_reads_through_discharge(&to, &self.data)
                {
                    if !self.first_pass {
                        diagnostic_at!(
                            self.lexer,
                            &stmt_start_pos,
                            Level::Error,
                            "the left side of this assignment is a `??` coalesce, which names \
                             two values and no place — on the null path it answers a fresh \
                             default, so there is nothing to write through. Name the place \
                             itself (`x {op} …`), with `?` if the read needs discharging \
                             (`x? {op} …`)"
                        );
                    }
                    let mut discard = Value::Null;
                    self.expression(&mut discard);
                    return Type::Void;
                }
                // Mark the variable as defined only once we have confirmed the `=` token
                // is actually present. Doing this before the token check caused any bare
                // `Value::Var` (e.g. `{cd}` inside a format string) to be marked defined
                // prematurely, hiding the "use before assignment" diagnostic.
                if op == "="
                    && let Value::Var(v_nr) = code
                    && !self.first_pass
                    && self.vars.exists(*v_nr)
                {
                    self.vars.defined(*v_nr);
                }
                // @PLN86 2.4 — a NON-`Var` LHS here is a field/index target
                // (`e.health = v` / `v[i] = v`).  Ownership-aware: a write to the
                // script's OWN data (a local of a script-defined struct type) is
                // fine — a mod must manage the entities it creates; only a write to
                // HOST data (a parameter, or any host-library struct/vector — the
                // type catches aliasing like `x = player; x.health = …`) is the
                // invariant-breaking raw write we reject.  Bare locals and struct
                // construction never route here.  Keyed by def (idempotent).
                if self.in_sandbox
                    && !self.first_pass
                    && !matches!(code.unspan(), Value::Var(_))
                    && self.raw_write_is_host_owned(code)
                {
                    let pos = self.lexer.peek_pos().clone();
                    // @PLN86 F5 — a ONE-LEVEL struct field write whose field carries an
                    // `#update` link is gated PER-FIELD (admission admits iff the token is
                    // granted).  A write to a field with NO update link, an index write, a
                    // nested field, or a stale stash falls through to the coarse 2.4 reject
                    // (a host field is read-only by default).  Field access uses
                    // type-specific ops (not just `OpGetField`), so we identify the write by
                    // the stash `field()` set, VERIFIED against the base var's struct type —
                    // a base that is not a `Var` of the stash's struct never qualifies.
                    let target = self.last_field_target.take();
                    let resolved = target.and_then(|(sd, field, read_count)| {
                        let base_var = match code.unspan() {
                            Value::Call(_, args) => match args.first().map(Value::unspan) {
                                Some(Value::Var(r)) => Some(*r),
                                _ => None,
                            },
                            _ => None,
                        };
                        let base_ok = base_var.is_some_and(
                            |r| matches!(self.vars.tp(r), Type::Reference(bsd, _) if *bsd == sd),
                        );
                        if !base_ok {
                            return None;
                        }
                        let has = |right: &str| {
                            self.member_access
                                .get(&(sd, field.clone()))
                                .is_some_and(|l| l.iter().any(|t| t.ends_with(right)))
                        };
                        // @PLN86 F6 — a `+=` to an `#append`-linked field is an APPEND
                        // (grow the collection); otherwise an `#update`-linked write is
                        // an UPDATE (F5).  Neither → the coarse 2.4 reject.
                        if op == "+=" && has("#append") {
                            Some((sd, field, read_count, true))
                        } else if has("#update") {
                            Some((sd, field, read_count, false))
                        } else {
                            None
                        }
                    });
                    if let Some((sd, field, read_count, is_append)) = resolved {
                        let ctx = self.context;
                        // un-record the spurious F4 read this field's LHS logged — a
                        // write is not a read (resolves the F4 read/update overlap).
                        if read_count > 0
                            && let Some(v) = self.sandbox_field_reads.get_mut(&ctx)
                        {
                            let n = v.len().saturating_sub(read_count);
                            v.truncate(n);
                        }
                        let map = if is_append {
                            &mut self.sandbox_field_appends
                        } else {
                            &mut self.sandbox_field_updates
                        };
                        map.entry(ctx).or_default().push((sd, field, pos));
                    } else {
                        self.sandbox_raw_writes.entry(self.context).or_insert(pos);
                    }
                }
                // loft#1205 — `P? op= e` / `P? = e`.  A `?` discharges a READ: on an
                // assignment place it says what to read when `P` is null, it does not make
                // the discharge itself the destination.  The LHS parse lowered it to the
                // same null-check the expression form uses, and that null-check is
                // RE-EVALUABLE, so every form below saw a value where a place should be.
                // Measured, all silent and on both backends: the vector `+=` path adopted
                // the check as the literal's own backing store and appended the destination
                // to itself (`b.d? += [r]` on a one-element field answered len 4), a null
                // place threw the write away entirely, a `text` place reached codegen with
                // no variable to write and took the compiler down, and a scalar place was
                // refused as *"Not implemented operation + for type integer"*.
                //
                // Peel to the place the discharge was reading, so the field / local being
                // written is what the machinery below is handed.  `(E-Asgn-Compound)` is
                // what that buys: the place's addressing then evaluates exactly once, and
                // the @PLN102 F2 hoist under this can still do its half for a place whose
                // addressing calls a function.
                //
                // Only the postfix `x?` peels.  An explicit `(a ?? b)` names two values and
                // no place — there is nothing to peel to — and stays refused.
                //
                // The rule this site enforces is `@FR-E-Asgn-Discharge`.
                //
                // Peeling alone answers for a COLLECTION and for `=`.  A collection's own
                // `op=` already reads through the discharge — `b.d += [r]` on a null field
                // builds the empty collection and appends into it — and a plain `=` has no
                // read to discharge, so in both the `?` asks for what the place already
                // does.  A scalar or `text` place PROPAGATES instead (`(N-Prop)`: null + 3
                // is null), so there the read is discharged explicitly by the seed built
                // below the F2 hoist.  `i? += 3` on a null `i` is 3, which is what the `?`
                // said; without the seed it would be null, and the `?` would be noise.
                let mut seed_wanted = false;
                if self.last_place_discharge
                    && let Some(place) = Self::peel_place_discharge(&to, &self.data)
                {
                    seed_wanted = op != "="
                        && !crate::parser::vectors::is_collection(f_type.base())
                        && (crate::data::is_scalar(f_type.base())
                            || matches!(f_type.base(), Type::Text(_)));
                    to = place.clone();
                    *code = place;
                } else if self.last_place_discharge
                    && let Some(subject) = Self::keyed_receiver_discharge(&to, &self.data).cloned()
                {
                    // The same rule one level in: the discharge is the RECEIVER of the
                    // target, and the write still lands in the collection the `?` was
                    // reading.  Unpeeled, the null path wrote into the fresh collection the
                    // discharge builds for its `else` arm — one nobody holds, dropped at the
                    // end of the statement, so `n.h?[k] = v` on an absent field answered
                    // length 0 with nothing reported (loft#1214).
                    //
                    // No seed, unlike the whole-place branch above: a keyed insert already IS
                    // "build the empty collection first", because loft#1213 materialises an
                    // absent keyed destination on the write itself.  That is also what makes
                    // the bare `n.h[k] = v` correct, and this peel is what makes the two
                    // spellings agree.
                    if let Value::Call(_, args) = to.unspan_mut()
                        && let Some(recv) = args.first_mut()
                    {
                        *recv = subject;
                    }
                    *code = to.clone();
                }
                // @PLN102 F2 (C92) — a compound assign evaluates its place ONCE.
                // When the place reads a heap scalar slot through a NON-idempotent
                // accessor (`w[idx()]`, `m[i()][j()]`, `getvec()[0]` — the index or
                // base calls a fn/method), bind that accessor's DbRef to a `_place`
                // RefVar temp evaluated ONCE and retarget the compound to it — the
                // internal form of `p = &w[idx()]; p op= rhs`.  A const/var index
                // carries no user call → no rewrite → byte-identical; `=` reads the
                // place once already.  The temp reads/writes via the uniform RefVar
                // deref (`OpGet*/OpSet*(place, 0)`), so the accessor's calls run once.
                // `to` is `OpGet<T>(accessor, offset)`: `accessor` produces the element
                // reference (and carries the fn/method call), `offset` is the field position
                // within it (0 for a scalar element, e.g. 8 for the 2nd field of a struct).
                let f2_place = if op != "=" && !self.first_pass {
                    match to.unspan() {
                        Value::Call(get_d, gargs)
                            if self.data.def(*get_d).name.starts_with("OpGet")
                                && gargs.len() == 2
                                && self.ir_has_user_call(&gargs[0]) =>
                        {
                            Some((*get_d, gargs[0].clone(), gargs[1].clone()))
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                let mut f2_setup: Option<Value> = None;
                let mut f2_hoisted = false;
                if let Some((get_d, accessor, offset)) = f2_place {
                    // The hoist rewrites `to` to reference a fresh temp, which hides the
                    // ORIGINAL place's const / value-const base from `validate_write` (it walks
                    // the expression). Run the write-legality check on the original place NOW,
                    // and tell parse_assign_op to skip its own (it would re-fire on the rewrite).
                    self.validate_write(&to, &parent_tp, op);
                    f2_hoisted = true;
                    // Hoist the ELEMENT reference and rebuild the field access on it,
                    // preserving `offset`.  Without this the accessor — and its call — was
                    // emitted twice, so a divergent index silently read from one element and
                    // wrote another (the exact C92 corruption).  The element is a
                    // `Reference(struct)`; strip the dynamic-index `?` (OOB safety on the
                    // read, not the slot type) via `base()`.
                    //
                    // ONE shape for every offset, including 0.  A separate offset-0 arm used
                    // to bind the accessor straight into a scalar `RefVar` on the reasoning
                    // that "the accessor IS a `&element` ref" — true for `w[idx()]`, false for
                    // a call that RETURNS a struct, where it aliased the callee's source
                    // instead of the returned copy.  `pick(ps).x += 5` then wrote into
                    // `ps[0]` while `.y` correctly did not, and native emitted a `_place` it
                    // never declared.  This arm answers both offsets correctly, so there is
                    // nothing for a second one to add.
                    let elem_tp = parent_tp.base().clone();
                    let elem_var = self.create_unique("_place", &elem_tp);
                    f2_setup = Some(v_set(elem_var, accessor));
                    let rebuilt = Value::Call(get_d, vec![Value::Var(elem_var), offset]);
                    to = rebuilt.clone();
                    *code = rebuilt;
                }
                // loft#1205 — the seed is built HERE, from the place the F2 hoist may just
                // have rewritten, because it reads the place a second time: once to ask
                // whether it is null, once to write the default in.  Read off the original
                // `w[idx()]`, that second read would call `idx()` again — measured, with the
                // seed built above instead: two calls, the value read from element 0 and
                // written to element 1, silently.  Through `_place` it is one call, which is
                // the whole point of the hoist and `(E-Asgn-Compound)`.
                let discharge_seed = if seed_wanted && !self.first_pass {
                    let base = f_type.base().clone();
                    self.discharge_seed(&to, &base)
                } else {
                    None
                };
                let var_nr = self.assign_var_nr(code, op, &f_type, &mut parent_tp);
                // Handle `f += X` for File variables before type-changing logic.
                if op == "+="
                    && self.is_file_var_type(&f_type)
                    && let Value::Var(file_v) = to
                {
                    self.append_to_file(code, file_v);
                    return Type::Void;
                }
                // record closure association if the RHS was a capturing lambda.
                // NOTE: must come AFTER parse_assign_op because that is where the RHS
                // lambda is parsed and last_closure_work_var gets set by emit_lambda_code.
                let result =
                    self.parse_assign_op(code, op, &f_type, &to, parent_tp, var_nr, f2_hoisted);
                // loft#1205 — the discharged read runs before the compound, which was built
                // for a place the seed has just made non-null.  Prepended FIRST so the F2
                // binding below ends up in front of it: the seed reads and writes THROUGH
                // that binding, so a `_place` bound after it is a `_place` it cannot see —
                // native reported `cannot find value var___place_1`, and the interpreter
                // quietly seeded the wrong slot.
                if let Some(seed) = discharge_seed {
                    let assign = std::mem::replace(code, Value::Null);
                    *code = Value::Insert(vec![seed, assign]);
                }
                // @PLN102 F2 — run the once-evaluated place binding before the compound.
                if let Some(setup) = f2_setup {
                    let assign = std::mem::replace(code, Value::Null);
                    *code = Value::Insert(vec![setup, assign]);
                }
                // #673 — a write to a struct-enum `text` payload binding lands in the
                // binding's own copy of the characters, so mirror it straight back into
                // the subject's field.  A heap payload binding is a DbRef into the
                // subject and needs nothing; only the text copy does.  See
                // `record_text_payload_view` for which bindings are registered.
                if let Value::Var(v) = to.unspan()
                    && let Some(read) = self.text_payload_views.get(&(self.context, *v)).cloned()
                    && let Value::Call(_, read_args) = &read
                {
                    let write = self.cl(
                        "OpSetText",
                        &[read_args[0].clone(), read_args[1].clone(), Value::Var(*v)],
                    );
                    let assign = std::mem::replace(code, Value::Null);
                    *code = Value::Insert(vec![assign, write]);
                }
                // @PLN25 DN3: reassigning a proven-non-null var invalidates its narrowing. The
                // RHS above was parsed WITH the narrowing (so `if a!=null { a = a+1 }` reads `a`
                // non-null), but any read AFTER this point widens back to `τ?` — the proof no
                // longer holds once the slot is overwritten.
                if var_nr != u16::MAX {
                    self.narrowed_non_null.retain(|&x| x != var_nr);
                    self.divisor_nonzero.retain(|&x| x != var_nr);
                }
                if op == "=" && self.last_closure_work_var != u16::MAX && var_nr != u16::MAX {
                    self.closure_vars.insert(var_nr, self.last_closure_work_var);
                    // store mapping in Function struct for native codegen.
                    self.vars
                        .set_closure_var_of(var_nr, self.last_closure_work_var);
                    self.last_closure_work_var = u16::MAX;
                }
                return result;
            }
        }
        // @PLN87 D-bind-7 — a statement that BEGAN with `&` whose `&` was not
        // consumed by an assignment: a bare `&a;` statement or a block-final
        // `{ &a }`.  Both are non-binding positions the VITAL rule (binding.md
        // B-Ref-AnnotationOnly) forbids — `&` binds a reference only as an
        // assignment RHS (`name = &a`); a standalone `&a` discards it.  The
        // operators.rs guard clears `amp_pending` when it has already reported the
        // `&` (a sub-expression `&a + 1`, a non-place `&(1+2)`), so the flag is
        // still set here ONLY in the unreported bare/block-final case; the
        // `started_with_amp` gate keeps a leaked flag from a nested `&(…)` parse
        // from mis-firing.  Report once on pass 2.
        if started_with_amp && self.amp_pending {
            if !self.first_pass {
                diagnostic_at!(
                    self.lexer,
                    &stmt_start_pos,
                    Level::Error,
                    "`&` is not a general operator — it binds a reference only as the \
                     whole right-hand side of an assignment (`a = &b`); a bare `&a` \
                     discards the reference. Drop it, or write `name = &a` to bind one"
                );
            }
            self.amp_pending = false;
        }
        *code = to;
        f_type
    }

    pub(crate) fn append_to_text(
        &mut self,
        code: &mut Value,
        op: &str,
        var_nr: u16,
        s_type: &Type,
    ) {
        // The const guard is `parse_assign_op_inner`'s, asked before it routed here.
        if let Value::Insert(ls) = code {
            // P217: same self-append handling as `Parser::assign_text`
            // (operators.rs).  When the RHS expression was `var + parts`,
            // `parse_append_text` emits the first op as
            // `OpAppendText(var, Var(var))` (self-append).  Without
            // detection, the downstream emit clears the destination
            // before the appends and so destroys the existing value
            // (interp's `out = out + "y"` produced "xxy"; native rejected
            // with E0502 because `*var += &*(&*var)` is borrow-conflict).
            // The check unspans both args so a `Span(Var(...))` operand
            // (the parser's source-position wrapper) doesn't slip past.
            if op == "=" {
                let self_append = ls.first().is_some_and(|first| {
                    if let Value::Call(_, args) = first.unspan() {
                        args.len() >= 2
                            && matches!(args[0].unspan(), Value::Var(v) if *v == var_nr)
                            && matches!(args[1].unspan(), Value::Var(v) if *v == var_nr)
                    } else {
                        false
                    }
                });
                if self_append {
                    ls.remove(0);
                }
            }
        } else if op == "=" {
            // P223: when the RHS Block (or any expression) reads the
            // destination var, the interpreter's text-Set clears the
            // destination before evaluating the RHS, so the read sees
            // the cleared value.  Wrap in a work-text so the Block is
            // fully evaluated into the work, then assigned to the
            // destination after the read has happened.  Mirrors the
            // analogous wrap in `Parser::assign_text` (operators.rs)
            // — local-text path — but for the RefVar(Text) parameter
            // path that lands here.
            // Same bind-site rule as the local-text path in `operators::assign_text`:
            // a branch RHS delivers per ARM into the destination.
            if self.try_branch_text_bind(code, var_nr) {
                // `code` is now a void branch of `Set(var_nr, …)`.
            } else if !self.first_pass && var_nr != u16::MAX && code.reads_var(var_nr) {
                // Pass-2-only mint (loft#665 piece 2) — this arm is `!first_pass`.
                let work = self.vars.work_text_p2(&mut self.lexer);
                let ls = vec![
                    self.cl("OpClearText", &[Value::Var(work)]),
                    self.cl("OpAppendText", &[Value::Var(work), code.clone()]),
                    v_set(var_nr, Value::Var(work)),
                ];
                *code = Value::Insert(ls);
            } else {
                *code = v_set(var_nr, code.clone());
            }
        } else if s_type == &Type::Character {
            *code = self.cl(
                "OpAppendStackCharacter",
                &[Value::Var(var_nr), code.clone()],
            );
        } else {
            *code = self.cl("OpAppendStackText", &[Value::Var(var_nr), code.clone()]);
        }
    }

    pub(crate) fn append_to_file(&mut self, code: &mut Value, file_v: u16) {
        let mut rhs_code = Value::Null;
        let mut unused = Type::Null; // parent_tp, this is normally used to unpack the vector fill
        // Clear any leftover cast marker so we only pick up a cast parsed as
        // part of THIS rhs expression.
        self.last_cast_alias = u32::MAX;
        let mut rhs_type = self.parse_operators(&Type::Unknown(0), &mut rhs_code, &mut unused, 0);
        if let Type::Rewritten(tp) = rhs_type {
            rhs_type = *tp;
        }
        let cast_alias = self.last_cast_alias;
        self.last_cast_alias = u32::MAX;
        *code = self.write_to_file(file_v, rhs_code, &rhs_type, cast_alias);
    }

    /// Plan-22 phase 02d-v — extract the boxed-scalar `v_nr`
    /// from an auto-deref'd LHS expression.  Recognises two
    /// shapes that phase 02d-iii.b's `auto_deref_boxed_scalar`
    /// produces:
    ///
    /// - **Direct** (Integer / Float / Single / Character /
    ///   Text / plain Enum):
    ///   `Call(OpGet<T>, [Var(v_nr), Int(0)])`
    /// - **Boolean** (byte-storage with bool conversion):
    ///   `Call(OpEqInt, [Call(OpGetByte, [Var(v_nr), Int(0),
    ///   Int(0)]), Int(1)])`
    ///
    /// Returns `None` for any other shape (struct field reads,
    /// non-zero offsets, plain `Var` LHS, closure-body `get_field`
    /// inner — the closure-body case doesn't need the alloc
    /// preamble because the cell was already allocated by the
    /// parent's first-set).
    /// loft#984 — wrap `code` in the declared-range guard when the value it computes is
    /// not PROVABLY inside the target's declared range.
    ///
    /// The proof is the range the source type already carries, so ordinary in-range code
    /// emits nothing and pays nothing: `x.b = 7` into a `limit(0, 255)` is a constant whose
    /// own range is `[7, 7]`, and `p.r = q.r` between two `limit(0, 255)` fields is
    /// range-for-range. What gets guarded is the case that actually goes wrong — a value
    /// whose range is wider than the slot's.
    ///
    /// Guarding the VALUE rather than the store is what lets one op reach both a FIELD and
    /// a VARIABLE. A field write carries its range in the store op (`min` only, which is
    /// why the top of the range escaped it); a local has no store op at all, which is why
    /// `a: integer limit(0,255) = 7; a = 300` kept 300.
    /// loft#1009 — clamp a COMPOUND assignment's result to a narrow-alias local's own range.
    ///
    /// Only for a plain whole-variable target: a field or an element goes through the store,
    /// which already applies this and is the oracle the bounds come from — an out-of-range
    /// compound write on a narrow FIELD leaves the type's minimum, on both backends and
    /// whether it overflowed or underflowed. A local is a stack slot and reaches no such
    /// layer, so `l: u8 = 250; l += 10;` kept 260 and `b: u8 = 5; b -= 10;` kept -5.
    ///
    /// Deliberately NOT the `=` path: there the value is judged at compile time
    /// (`is_narrowing_int_store`), and `declared_range`'s own comment records what happens
    /// when a runtime default is added on top of a check that already holds — it handed 24
    /// of the stdlib's own `i8` stores a `-128`.
    pub(crate) fn guard_compound_range(&mut self, code: &mut Value, target: &Type, nullable: bool) {
        let Some((lo, hi, dflt)) = Self::compound_range(target, nullable) else {
            return;
        };
        // Two seams can reach one store, so never wrap a guard in a guard: harmless
        // arithmetically (the inner already answers a value in range) but it would judge
        // the same write twice.  Mirrors `guard_declared_range`.
        if let Value::Call(d, _) = code.unspan()
            && self.data.def(*d).name() == "OpRangeDefault"
        {
            return;
        }
        let guarded = self.cl(
            "OpRangeDefault",
            &[
                code.clone(),
                Value::Long(lo),
                Value::Long(hi),
                Value::Long(dflt),
            ],
        );
        *code = guarded;
    }

    /// The bounded range a compound assignment's target declares — `None` when it
    /// declares none, which is the plain `integer` case and the only unbounded one.
    ///
    /// **One range, whatever the spelling.** `formal/types.md` `(C-Int)` puts width
    /// INSIDE the conversion relation — "an integer flows into another integer iff its
    /// range fits", with no separate width authority — so `u8` and `integer limit(0,255)`
    /// are the same range and must bound a write the same way.  Keying the guard on the
    /// width SPELLING instead is how they came apart: `guard_narrow_alias_local` tested
    /// `forced_size`, which only a narrow ALIAS sets, so the `limit(...)` spelling reached
    /// no guard on the compound path at all and `l: integer limit(0,255) = 250; l += 10`
    /// kept 260 while the `u8` spelling of that identical range clamped (loft#1030).
    ///
    /// The default a value outside the range takes is `uncomputable_default`'s, not this
    /// function's: @FR-E-Uncomp for a slot that can hold null and @FR-E-Uncomp-NN for one
    /// that cannot.  Both arms ask it, so the two spellings of one range cannot answer
    /// differently.
    /// The range a compound-assignment target declares, in EITHER spelling: the
    /// `limit(lo, hi)` form via [`declared_range`], and the narrow ALIASES (`u8`, `i8`, `u16`,
    /// `i16`, `u32`, `i32`) that `declared_range` deliberately cannot see because it stops at
    /// `forced_size`.
    ///
    /// Reach for THIS when the question is *"whatever range this target declares"*; reach for
    /// [`declared_range`] only when the `limit(...)` spelling is specifically what is meant.
    /// Getting that backwards is silent — the guard simply never fires (@PLN152 step 2).
    fn compound_range(tp: &Type, nullable: bool) -> Option<(i64, i64, i64)> {
        // `declared_range` answers the `limit(lo, hi)` spelling and deliberately nothing
        // else — it returns `None` the moment `forced_size` is set.  So the two arms
        // below partition the bounded types rather than overlapping.
        if let Some(r) = declared_range(tp, nullable) {
            return Some(r);
        }
        let Type::Integer(spec) = tp.base() else {
            return None;
        };
        // `forced_size` marks a narrow ALIAS (`u8`/`i8`/`u16`/`i16`/`i32`/`u32`).
        //
        // There is deliberately no `is_signed32_template()` test here.  It reads like a
        // guard against the plain `integer` type, but that carries no `forced_size` and
        // has already returned above — so by this line the only spec whose range IS the
        // signed-32 range is the `i32` ALIAS, and testing for it excluded exactly one
        // alias of the six (loft#1009).
        //
        // Plain `integer` and the wide template stay unbounded ON PURPOSE: 447564a1
        // measured that a guard clamping every integer satisfies every other assertion in
        // the regression file, so `integer` running past the 4-byte range is a live cell
        // there, not an oversight.
        if spec.forced_size.is_none() || spec.is_wide_template() {
            return None;
        }
        // @FR-N-Reserve, as in `declared_range` — a nullable narrow alias is bounded by its
        // usable range, and this arm is the one every `u8?` / `i16?` reaches (loft#1249).
        let lo = i64::from(spec.usable_min(nullable));
        let hi = spec.usable_max(nullable);
        // @FR-E-Uncomp / @FR-E-Uncomp-NN through the same home the `limit(…)` arm uses.
        // Asking it HERE rather than answering `range_default` directly is what makes a
        // nullable narrow alias answer null: this arm is the only one a `u8?` reaches.
        Some((lo, hi, uncomputable_default(nullable, spec)))
    }

    /// @PLN152 — an author-written `??` IS the answer for a value that does not fit, so
    /// guard INSIDE the discharge rather than around it, and report nothing.
    ///
    /// `x = (x + 10) ?? 255` into a `u8` is refused today as a narrowing that may lose data,
    /// which is true of the coalesce's RESULT — `x + 10` can be 265 — and false of what the
    /// author wrote: the `??` names exactly what happens when it does not fit. Wrapping the
    /// whole discharge cannot express that, because by then the out-of-range value has
    /// already been chosen as the coalesce's non-null answer.
    ///
    /// So the guard goes on the discharge's SUBJECT, with the SENTINEL as its default rather
    /// than the type's: an out-of-range subject then reads null, the existing coalesce
    /// supplies the author's value, and what reaches the slot is always in range. The
    /// sentinel never lands anywhere — it lives in the `__ncc_N` temp, which is a full
    /// `integer` — so no narrow storage is asked to hold a value it has no code for.
    ///
    /// Returns true when it claimed the store, so the caller skips both the narrowing
    /// refusal and the ordinary outside-the-expression guard.
    /// The cures for an implicit narrowing store that does not provably fit — @PLN152 N1.
    ///
    /// `as <dst>` is deliberately NOT among them, and it used to be the only thing offered.
    /// This error fires exactly when the value does not provably fit, which is also exactly
    /// when the explicit cast is refused (*"narrowing cast ... may not fit at runtime; use
    /// `<dst>?` for a checked cast"*). So the old advice cost the reader a second refusal to
    /// discover the first was unusable — two hops to a cure the first message could name.
    ///
    /// The `??` cure is real rather than aspirational: a discharge at the ROOT of the stored
    /// expression supplies the fallback (`range_guard_inside_discharge`), so the position it
    /// is offered in is the position it works in.
    pub(crate) fn narrowing_cures(code: &Value, dst: &str) -> String {
        let mut out = format!(
            "give it a fallback with `?? <value>`, take the checked cast `as {dst}?` (value or \
             null), or make the value provably fit (a mask, or an `if` range check)"
        );
        // @PLN152 N2 — a `??` that discharges a SUB-expression reads, to its author, as if it
        // should have covered the store. Only a discharge at the root does. Shallow on
        // purpose: the shape this is for is `(v[0] ?? 0) + 10`, where the discharge is a direct
        // operand of the root, and a deeper walk would start claiming coincidences.
        if !Self::is_root_discharge(code)
            && let Value::Call(_, args) = code.unspan()
            && args.iter().any(Self::is_root_discharge)
        {
            out.push_str(
                " — the `??` already here discharges a value INSIDE the expression, not the \
                 store, so it cannot supply the fallback; move it outside",
            );
        }
        out
    }

    /// Is this expression itself a null discharge (`a ?? b`)?
    fn is_root_discharge(v: &Value) -> bool {
        matches!(v.unspan(), Value::Block(bl) if bl.name == "ncc")
    }

    pub(crate) fn range_guard_inside_discharge(&mut self, code: &mut Value, target: &Type) -> bool {
        // A nullable target already answers null for a value that does not fit, and its own
        // `??` reads that today — there is nothing here to add and claiming it would move a
        // shape that works.
        if matches!(target, Type::Optional(_)) {
            return false;
        }
        // `compound_range`, not `declared_range`: the latter answers the `limit(lo, hi)`
        // spelling and returns None the moment `forced_size` is set, which is exactly what a
        // narrow ALIAS is — so it cannot see the five widths this exists for.
        if Self::compound_range(target, false).is_none() {
            return false;
        }
        let Type::Integer(spec) = *target.base() else {
            return false;
        };
        // Only the block-shaped discharge carries a subject that can be guarded once. A bare
        // variable subject lowers to a plain `if` that reads it twice (see
        // `null_discharge_subject`), and guarding there would evaluate the range test on one
        // read and not the other.
        // A discharge reaches here in TWO shapes, and taking only the first is why an
        // author's `b ?? 255` on a bare variable stayed refused while `(b + 1) ?? 255`
        // worked.  `null_discharge_subject` above is the one home for what they look like:
        // a non-trivial subject is bound to an `__ncc_N` temp by the block's head statement,
        // while a bare VARIABLE subject can be read twice for free and so lowers to a plain
        // `if` with the subject standing in the condition AND the then arm.
        //
        // Both reads are guarded in the second shape.  That is sound because the checked cast
        // is PURE and SILENT — it raises nothing, so evaluating it twice costs a range test
        // and reports nothing twice.  With `OpRangeDefault` it would have reported the same
        // store twice, which is one more reason this path does not use it.
        let subjects: Vec<Value> = match code.unspan() {
            Value::Block(bl) if bl.name == "ncc" => match bl.operators.first().map(Value::unspan) {
                Some(Value::Set(_, subject)) => vec![(**subject).clone()],
                _ => return false,
            },
            // The bare-variable spelling, and ONLY that: the `if` is asked whether it is the
            // lowering of `v ?? d` (`bare_variable_discharge`), never assumed to be one for
            // being an `if`.  Every value-`if` an author writes reaches this seam in the same
            // node, and claiming it guarded the then arm — a checked cast, so a `u8` local
            // read null and a `limit(…)` slot lost its default — and range-cast the first
            // operand of the author's own CONDITION, so `c: u8 = if k == 1000 { a } else { b }`
            // took the else arm (loft#1379).  For the coalesce itself both reads of the
            // subject are guarded: the condition `coalesce_not_null` built, and the then arm.
            Value::If(cond, then, _) => {
                if self.bare_variable_discharge(cond, then).is_none() {
                    return false;
                }
                let mut v = vec![(**then).clone()];
                if let Value::Call(_, args) = cond.unspan()
                    && let Some(first) = args.first()
                {
                    v.push(first.clone());
                }
                v
            }
            _ => return false,
        };
        // Already guarded — the two seams that reach one store must not judge it twice.
        if subjects.iter().any(|sub| {
            matches!(sub.unspan(), Value::Call(d, _) if self.data.def(*d).name() == "OpRangeDefault")
        }) {
            return true;
        }
        let guarded: Vec<Value> = subjects
            .into_iter()
            .map(|mut sub| {
                self.dn4_checked_cast(&mut sub, &Type::Integer(spec), &crate::data::I64);
                sub
            })
            .collect();
        match code.unspan_mut() {
            Value::Block(bl) => {
                if let Some(Value::Set(_, subject)) =
                    bl.operators.first_mut().map(Value::unspan_mut)
                {
                    **subject = guarded[0].clone();
                }
            }
            Value::If(cond, then, _) => {
                **then = guarded[0].clone();
                if let Some(g) = guarded.get(1)
                    && let Value::Call(_, args) = cond.unspan_mut()
                    && let Some(first) = args.first_mut()
                {
                    *first = g.clone();
                }
            }
            _ => return false,
        }
        true
    }

    /// `@FR-N-Coal` — the variable `v` when this `if` is the lowering of `v ?? d` for a BARE
    /// variable, and `None` for every other `if`.
    ///
    /// The `??` lowering reads a bare variable twice for free, so it builds a plain
    /// `if coalesce_not_null(v) { v } else { d }` and leaves no marker on it — the same node
    /// an author's value-`if` is.  So the shape is asked of the BUILDER rather than read off
    /// the node: the then arm must be a plain read of `v`, and the condition must be exactly
    /// what `coalesce_not_null` builds for `v`'s type.  An author's `if` fails one or the
    /// other — its condition is its own, or its then arm is an expression — and an author's
    /// `if x != null { x } else { d }` fails too, because `!= null` lowers to `OpNeInt(x, null)`
    /// where the builder spells `OpConvBoolFromInt(x)`: that spelling asked for no coalesce
    /// and is judged as the narrowing it is.
    ///
    /// Only an integer subject is asked, because only a narrow integer TARGET reaches the one
    /// caller; the builder's other arms (a collection, a tagged enum, a type variable) are
    /// not re-run here.  The LEFT-hand-side readers of a discharge ([`null_discharge_subject`])
    /// keep a looser test, and that is sound where they read: no author's `if` can stand on
    /// the left of `=`.
    fn bare_variable_discharge(&mut self, cond: &Value, then: &Value) -> Option<u16> {
        let Value::Var(v) = then.unspan() else {
            return None;
        };
        let v = *v;
        if v >= self.vars.count() {
            return None;
        }
        let tp = self.vars.tp(v).clone();
        if !matches!(tp.base(), Type::Integer(_)) {
            return None;
        }
        let expected = self.coalesce_not_null(&Value::Var(v), &tp);
        (*cond.unspan() == expected).then_some(v)
    }

    pub(crate) fn guard_declared_range(
        &mut self,
        code: &mut Value,
        target: &Type,
        source: &Type,
        nullable: bool,
    ) {
        let Some((lo, hi, dflt)) = declared_range(target, nullable) else {
            return;
        };
        // Two seams reach the same store — the assignment path and `convert` — so guard
        // against wrapping a guard.  Harmless arithmetically (the inner already answers a
        // value in range) but it would report the same out-of-range write twice.
        if let Value::Call(d, _) = code.unspan()
            && self.data.def(*d).name() == "OpRangeDefault"
        {
            return;
        }
        // Already inside the slot's range for every value the source can take → no guard.
        if let Type::Integer(src) = source.base()
            && i64::from(src.min) >= lo
            && i64::from(src.max) <= hi
        {
            return;
        }
        let guarded = self.cl(
            "OpRangeDefault",
            &[
                code.clone(),
                Value::Long(lo),
                Value::Long(hi),
                Value::Long(dflt),
            ],
        );
        *code = guarded;
    }

    fn extract_boxed_var_from_lhs(&self, lhs: &Value) -> Option<u16> {
        let Value::Call(op_d, args) = lhs.unspan() else {
            return None;
        };
        let op_name = self.data.def(*op_d).name();
        // Direct shape: Call(OpGet<T>, [Var(v_nr), Int(0)])
        if args.len() == 2
            && matches!(args[1].unspan(), Value::Int(0))
            && op_name.starts_with("OpGet")
            && let Value::Var(v_nr) = args[0].unspan()
        {
            return Some(*v_nr);
        }
        // Boolean shape: Call(OpEqInt, [Call(OpGetByte, [Var, 0, 0]), Int(1)])
        if op_name == "OpEqInt"
            && args.len() == 2
            && matches!(args[1].unspan(), Value::Int(1))
            && let Value::Call(inner_op, inner_args) = args[0].unspan()
            && self.data.def(*inner_op).name() == "OpGetByte"
            && inner_args.len() == 3
            && matches!(inner_args[1].unspan(), Value::Int(0))
            && matches!(inner_args[2].unspan(), Value::Int(0))
            && let Value::Var(v_nr) = inner_args[0].unspan()
        {
            return Some(*v_nr);
        }
        None
    }

    /// Plan-22 phase 02d-iii.d — prepend cell allocation when a
    /// first-set assignment targets an uninitialised boxed-scalar
    /// local in the parent body.
    ///
    /// Context: phase 02d-iii.b's auto-deref wraps every read of
    /// a boxed-scalar local as `Call(OpGet<T>, [Var(v_nr),
    /// Int(0)])`.  When that shape lands on the LHS of an
    /// assignment, the existing `towards_set` →
    /// `call_to_set_op` machinery (parser/operators.rs:283)
    /// correctly maps `OpGet<T>` → `OpSet<T>` and yields
    /// `Call(OpSet<T>, [Var(v_nr), Int(0), rhs])`.  But for the
    /// FIRST set, `v_nr`'s slot holds an uninitialised DbRef —
    /// writing through it would crash the runtime.
    ///
    /// This helper detects that pattern and wraps the result in
    /// `Insert([v_set(v_nr, Null), OpDatabase(v_nr, cell_kt),
    /// result])` so the cell is allocated before the first
    /// write.  Marks `v_nr` as defined; subsequent writes hit
    /// the same OpSet directly without the prepend.
    ///
    /// CLOSURE-BODY case: when the LHS auto-deref's `inner` is
    /// NOT a `Var` (e.g. `get_field(closure, n_field)` for a
    /// captured boxed scalar), the helper is a no-op — the
    /// cell was already allocated by the parent's first-set,
    /// the closure record holds the shared DbRef, and the
    /// closure's write goes through that DbRef directly via the
    /// existing `Call(OpSet<T>, [get_field, Int(0), rhs])` IR.
    /// This is the missing-write-rewrite that 02d-iii.d delivers
    /// FOR FREE via 02d-iii.b's auto-deref + the existing
    /// `call_to_set_op` machinery.
    ///
    /// Dormant in production (02d-iii.a's flip is dormant — no
    /// variable carries `Reference(__cell_*, _)`).  Activates in
    /// 02d-iii.e.
    #[allow(
        dead_code,
        reason = "Helper invoked from tests; parse_assign_op hook activates with 02d-iii.e."
    )]
    pub(crate) fn maybe_prepend_cell_alloc(&mut self, result: Value, lhs: &Value) -> Value {
        let Some(v_nr) = self.extract_boxed_var_from_lhs(lhs) else {
            return result;
        };
        if !self.vars.exists(v_nr) {
            return result;
        }
        let tp = self.vars.tp(v_nr).clone();
        let Some(cell_d_nr) = crate::parser::vectors::boxed_cell_def(&tp, &self.data) else {
            return result;
        };
        if self.vars.is_defined(v_nr) {
            // Subsequent: cell already exists, no alloc needed.
            return result;
        }
        // First-set: prepend cell allocation.
        let op_db = self.data.def_nr("OpDatabase");
        if op_db == u32::MAX {
            return result;
        }
        let cell_kt = i32::from(self.data.def(cell_d_nr).known_type());
        self.vars.defined(v_nr);
        Value::Insert(vec![
            v_set(v_nr, Value::Null),
            Value::Call(op_db, vec![Value::Var(v_nr), Value::Int(cell_kt)]),
            result,
        ])
    }

    /// Plan-22 phase 02d-iii.c — build the rewrite IR for an
    /// assignment to a boxed-scalar local.  Returns `Some(IR)`
    /// when `var_nr`'s type is `Reference(__cell_<T>, _)` and
    /// the cell's value-field type is one of the supported
    /// primitives; `None` otherwise.
    ///
    /// Shapes:
    /// - **First assignment** (`!is_defined(var_nr)`):
    ///   `Insert([v_set(n, Null), OpDatabase(n, cell_kt),
    ///   OpSet<T>(Var(n), 0, rhs)])` — allocate a fresh cell
    ///   record + initialise the `value` field.  Mirrors the
    ///   pattern `parse_object` uses for struct literals
    ///   (objects.rs:1306-1361).
    /// - **Subsequent assignment** (`is_defined(var_nr)`):
    ///   `Call(OpSet<T>, [Var(n), Int(0), rhs])` — write the
    ///   `value` field of the existing cell.
    ///
    /// Op handling: only `=` is rewritten by this helper.
    /// Compound `+=` / `-=` / etc. need read + compute + write,
    /// which the parser already builds via
    /// `n = n + 1` lowering (the read side uses 02d-iii.b's
    /// auto-deref; the write hits this helper as a plain `=`).
    ///
    /// Boolean cells fall through to None — `OpSetByte` takes
    /// 4 args (ref, fld, min, val) instead of 3, and the
    /// boolean-to-byte conversion needs different IR.  Phase
    /// 02d-iii.e (or later) extends to boolean if needed.
    ///
    /// Dormant in production today (02d-iii.a's flip is dormant).
    /// 02d-iii.e activates the flip + this helper fires for real
    /// on every assignment to a boxed scalar.
    #[allow(
        dead_code,
        reason = "Helper invoked from tests; parse_assign_op hook activates with 02d-iii.e."
    )]
    pub(crate) fn boxed_scalar_assign_rewrite(
        &self,
        var_nr: u16,
        op: &str,
        rhs: Value,
    ) -> Option<Value> {
        if op != "=" || var_nr == u16::MAX || !self.vars.exists(var_nr) {
            return None;
        }
        let tp = self.vars.tp(var_nr).clone();
        let cell_d_nr = crate::parser::vectors::boxed_cell_def(&tp, &self.data)?;
        let value_attr = self.data.def(cell_d_nr).attributes().first()?;
        if value_attr.name != "value" {
            return None;
        }
        let op_set_d_nr = self.cell_value_set_op(cell_d_nr)?;
        if self.vars.is_defined(var_nr) {
            // Subsequent: write value field of existing cell.
            Some(Value::Call(
                op_set_d_nr,
                vec![Value::Var(var_nr), Value::Int(0), rhs],
            ))
        } else {
            // First-set: allocate cell + fill value field.
            self.boxed_cell_alloc_and_set(var_nr, cell_d_nr, rhs)
        }
    }

    /// The `OpSet<T>` that writes a `__cell_<T>`'s `value` field, or `None` for a
    /// cell whose payload type has no such op.
    ///
    /// Boolean is in the table because the working path emits `OpSetBoolean` for a
    /// boxed boolean local — this is the same op, read off that lowering rather
    /// than re-derived (the earlier "boolean needs a 4-arg `OpSetByte`" note
    /// described a different write path).
    /// The `OpSet<T>` a boxed capture's `value` field is written through — the write half of
    /// `@FR-L-CapWrite`, whose read half is `Parser::auto_deref_boxed_scalar`.
    fn cell_value_set_op(&self, cell_d_nr: u32) -> Option<u32> {
        let value_attr = self.data.def(cell_d_nr).attributes().first()?;
        if value_attr.name != "value" {
            return None;
        }
        // `.base()` for the same reason the read side peels: a nullable cell's `value` shares
        // its dense twin's storage, so it takes the same WRITE op (loft#1408).
        let op_set_name = match value_attr.typedef.base() {
            Type::Integer(_) => "OpSetInt",
            Type::Float => "OpSetFloat",
            Type::Single => "OpSetSingle",
            Type::Text(_) => "OpSetText",
            Type::Character => "OpSetCharacter",
            Type::Boolean => "OpSetBoolean",
            Type::Enum(_, false, _) => "OpSetEnum",
            _ => return None,
        };
        let d = self.data.def_nr(op_set_name);
        if d == u32::MAX { None } else { Some(d) }
    }

    /// Allocate a fresh `__cell_<T>` for `var_nr` and initialise its `value` field
    /// from `rhs` — the ONE home for "a boxed scalar comes into existence".
    ///
    /// Two callers need exactly this: the first assignment to a boxed local
    /// ([`Self::boxed_scalar_assign_rewrite`]), and the function-entry seed for a
    /// boxed scalar PARAMETER promoted to a shadow local (#685), which has no
    /// assignment of its own to hang the allocation on.
    pub(crate) fn boxed_cell_alloc_and_set(
        &self,
        var_nr: u16,
        cell_d_nr: u32,
        rhs: Value,
    ) -> Option<Value> {
        let op_set_d_nr = self.cell_value_set_op(cell_d_nr)?;
        let op_db_d_nr = self.data.def_nr("OpDatabase");
        if op_db_d_nr == u32::MAX {
            return None;
        }
        let cell_kt = i32::from(self.data.def(cell_d_nr).known_type());
        Some(Value::Insert(vec![
            v_set(var_nr, Value::Null),
            Value::Call(op_db_d_nr, vec![Value::Var(var_nr), Value::Int(cell_kt)]),
            Value::Call(op_set_d_nr, vec![Value::Var(var_nr), Value::Int(0), rhs]),
        ]))
    }

    /// Determine the variable number for an assignment target.
    /// For text `+=`, creates a unique temporary variable.
    ///
    /// `.base()` on the text test, because a `text?` accumulator appends exactly like a
    /// dense one — the same reading `parse_assign_op_inner`'s routing already takes
    /// (@PLN25 slice (c)).  Spelled `Type::Text` here and `f_type.base()` there, the two
    /// disagreed about one notion: the router sent a nullable field's `+=` down the
    /// text-append path while this left it with no variable to append THROUGH, so
    /// `n.t += "cd"` on a `t: text?` field emitted `Set(65535, …)` and the scope pass
    /// asserted on it — an internal compiler error on both backends for an ordinary
    /// append to an ordinary field (loft#1206).
    pub(crate) fn assign_var_nr(
        &mut self,
        code: &mut Value,
        op: &str,
        f_type: &Type,
        parent_tp: &mut Type,
    ) -> u16 {
        if let Value::Var(v_nr) = *code {
            v_nr
        } else if extract_nested_tuple_lhs(code).is_some() {
            // loft#1228 — a TUPLE ELEMENT keeps `u16::MAX` and takes the general path, whose
            // `towards_set` now has a `TuplePut` route for it.  Minting the text work variable
            // below put the append in a local that is never written back, so the statement
            // reached codegen with a variable naming no slot — SIGSEGV on the interpreter and
            // `E0425` from rustc on `--native`.
            u16::MAX
        } else if op == "+=" && matches!(f_type.base(), Type::Text(_)) {
            // The temp holds what the field holds, NULL INCLUDED, so it is typed the way
            // the field is.  `--native` decides whether an append propagates a null from
            // the DESTINATION VARIABLE's static type (`generation/text.rs::append_text`),
            // while the interpreter tests the value it finds at run time; typed dense, the
            // temp told native there was nothing to propagate and `n.t += "cd"` on a null
            // `text?` field appended onto the null sentinel — `"\0cd"`, reported non-null,
            // where the interpreter left the field null.  One notion, two spellings, and a
            // shape that only became reachable when the `+=` above stopped being an ICE.
            let tmp_tp = if matches!(f_type, Type::Optional(_)) {
                Type::Optional(Box::new(Type::Text(Deps::none())))
            } else {
                Type::Text(Deps::none())
            };
            let v = self.vars.unique("field", &tmp_tp, &mut self.lexer);
            *code = Value::Var(v);
            *parent_tp = Type::Null;
            v
        } else {
            u16::MAX
        }
    }

    /// Handle assignment into a `RefVar(Text)` target; returns true if handled.
    pub(crate) fn assign_refvar_text(
        &mut self,
        code: &mut Value,
        f_type: &Type,
        s_type: &Type,
        op: &str,
        var_nr: u16,
    ) -> bool {
        let Type::RefVar(t) = f_type else {
            return false;
        };
        if !matches!(**t, Type::Text(_)) {
            return false;
        }
        self.append_to_text(code, op, var_nr, s_type);
        true
    }

    /// Handle `out = <struct field>` where `out: &T` and `T` is a struct; returns true
    /// if handled.
    ///
    /// A `&` parameter is the callee's second way to hand a value back (the first is
    /// `return`), and a whole-binding write TRANSFERS a store to the caller, who then
    /// owns and frees it (@PLN87 P2.2).  A FIELD READ is not a store — it is a VIEW
    /// into the record that holds the field, so `out = ld.wl_world` published a pointer
    /// into a local the frame frees on the way out.  The caller kept reading it, and the
    /// next allocation landed on top: a four-chunk world became a one-chunk world with
    /// its edit clock running BACKWARDS, across a call that never touched its argument
    /// (loft#775).  `--interpret` reported the store as unfreed and faulted under
    /// `LOFT_POISON=1`; `--native` silently answered with whatever was allocated next.
    ///
    /// Publish an owned COPY instead — the same materialise-the-view move
    /// `materialize_view_return` makes for `return ld.wl_world`, which is why that
    /// direction was already safe and this one was not.  The copy is marked
    /// `skip_free`: the caller owns it now, exactly as it owns a fresh record built in
    /// place (`o = Obj{…}`), so freeing it here would orphan the caller's binding.
    pub(crate) fn assign_refvar_reference(
        &mut self,
        code: &mut Value,
        f_type: &Type,
        op: &str,
        var_nr: u16,
    ) -> bool {
        let Type::RefVar(inner) = f_type else {
            return false;
        };
        // A struct-ENUM is the same record shape one former over — `Type::Enum(_, true, _)` is
        // exactly what the write-back emitter's own allow-list names beside `Type::Reference`
        // — and it reached the identical defect: `x = o` left the caller aliasing the source
        // and, from a callee local, reading a freed record (loft#1303).  A PLAIN enum
        // (`Enum(_, false, _)`) is a byte with no store and is not here.
        // `base()` rather than the bare inner: @FR-L-Null gives `layout(τ) = layout(τ?)`, so a
        // `&S?` displaces a store exactly as its dense twin does.  It is the peel
        // `Type::is_amp_rebindable_heap` — the one home for *which heap kinds does the `&`
        // write-back repoint?* — already uses to answer the same question (loft#1291).
        let (td, is_senum) = match inner.base() {
            Type::Reference(td, _) => (*td, false),
            Type::Enum(td, true, _) => (*td, true),
            _ => return false,
        };
        let record_tp = |td: u32, deps: Deps| {
            if is_senum {
                Type::Enum(td, true, deps)
            } else {
                Type::Reference(td, deps)
            }
        };
        // A bare VARIABLE right-hand side needs the same materialisation as a field, and
        // for the same reason one level up: @FR-B-Copy makes a plain heap whole-value bind
        // a COPY, so the caller's binding must end up owning a store no live binding in
        // the callee still names.  Installing the source's own store instead gives one
        // store two owners — @FR-O-Owner — and which half breaks depends only on who else
        // holds it: a caller-reachable source (`x = o`) leaves the caller aliasing its own
        // argument and orphans the displaced store, while a callee-LOCAL one (`x = m`) is
        // freed at the callee's scope exit and the caller reads a freed store (loft#1303).
        // `x = x` is excluded: it would copy a store onto itself and free the original.
        //
        // ⚠ Only for a `&` PARAMETER.  This function serves two constructs, and they want
        // opposite things from a bare-var right-hand side: @FR-F-ParamRef makes a parameter's
        // whole-value `x = e` a WRITE-BACK, which installs a store, while `(B-Ref-Alias)`
        // makes a `&` LOCAL bind (`q = &p`) a live LINK to the source, which must not copy.
        // The field arm above is unrestricted because it materialises a VIEW rather than a
        // link, which is right for both (loft#775).
        let bare_var_rhs = self.vars.is_argument(var_nr)
            && matches!(code.unspan(), Value::Var(rv) if *rv != var_nr);
        if op != "=" || !(self.is_field(code) || bare_var_rhs) {
            return false;
        }
        // Keyed on the IR SHAPE, not on the right-hand side's deps: deps accumulate
        // while a body parses, so a deps test would mint the work-ref on pass 2 only
        // and shift every later `__ref_N` — the cross-pass divergence the H5 contract
        // catches.  A field read is a field read on both passes, and so is a bare var.
        let kt = self.data.def(td).known_type();
        let w = self
            .vars
            .work_refs(&record_tp(td, Deps::none()), &mut self.lexer);
        self.vars.set_skip_free(w);
        if self.first_pass {
            return false;
        }
        let copy_d = self.data.def_nr("OpCopyRecord");
        let view = std::mem::replace(code, Value::Null);
        *code = v_block(
            vec![
                v_set(w, Value::Null),
                self.cl("OpDatabase", &[Value::Var(w), Value::Int(i32::from(kt))]),
                Value::Call(copy_d, vec![view, Value::Var(w), Value::Int(i32::from(kt))]),
                Value::Var(w),
            ],
            record_tp(td, Deps::frame1(w)),
            "materialized_amp_field",
        );
        // @PLN130 — a NECESSARY copy that was nonetheless invisible: a program whose only
        // copies are these executes two record copies and `--report-copies` still answers
        // `none`. Being emitted into the IR is not the same as producing a user-facing row.
        crate::copy_manifest::record(
            self.context,
            w,
            kt,
            crate::copy_manifest::Origin::ParserMaterialise,
        );
        false
    }

    /// Materialise a `&`-KEYED-parameter write-back's right-hand side into its own store.
    ///
    /// The keyed sibling of [`Self::assign_refvar_reference`], and the same rule: a
    /// whole-value `x = e` on a `&hash<T[k]>` (or `sorted`/`index`/`spatial`/`trie`)
    /// REPOINTS the caller's slot (@FR-F-ParamRef), so whatever store it installs the caller
    /// now owns.  @FR-B-Copy makes a plain heap whole-value bind a COPY, so that store has to
    /// be one no live binding in the callee still names — otherwise @FR-O-Owner's single owner
    /// is violated and one of two things goes wrong, depending only on who else holds it:
    ///
    /// * a source the CALLER can still reach (`x = o`, a plain heap parameter, which
    ///   `(F-ParamHeap)` makes an alias of the caller's argument) leaves the caller's two
    ///   bindings naming one store — mutating one is visible through the other — and
    ///   orphans the store the write-back displaced;
    /// * a source the CALLEE owns (`x = m`, its own minted local) is freed at the callee's
    ///   scope exit, and every later read in the caller is a use-after-free that answers
    ///   correctly until the slot is handed out again (loft#1303).
    ///
    /// The cure is the one the caller's frame needs either way: mint a fresh store, deep-copy
    /// the source into it, and let the general path install THAT — after which codegen's
    /// `amp_owned_writeback` leg sees an owned value and releases the displaced store, guarded
    /// by `Stores::free_displaced` where the caller's binding does not own it (loft#1287).
    ///
    /// Copying INTO the target's existing store instead — the in-place refill a `&vector`
    /// takes — is not available here: it never repoints, so through a plain forwarder the
    /// write reaches the caller's CALLER, which `(F-ParamRebind)` forbids (loft#1291's
    /// vector half, measured and backed out).
    ///
    /// Keyed on the IR SHAPE for [`Self::assign_refvar_reference`]'s reason — deps accumulate
    /// as a body parses, so a deps test would mint the work-ref on pass 2 alone and shift
    /// every later `__ref_N`. A field read and a bare variable are both the same shape on
    /// both passes. `x = x` is excluded: it would copy a store onto itself and then free the
    /// original as the displaced one.
    ///
    /// Answers `false` like its sibling, so the general path still emits the `Set(x, …)`
    /// that installs the materialised store.
    pub(crate) fn assign_refvar_keyed(
        &mut self,
        code: &mut Value,
        f_type: &Type,
        op: &str,
        var_nr: u16,
    ) -> bool {
        let Type::RefVar(inner) = f_type else {
            return false;
        };
        // `base()` for the sibling's reason: @FR-L-Null makes a `&hash<T[k]>?` the same
        // question as its dense twin — the storage is what gets displaced.
        let inner = inner.base();
        if !crate::parser::vectors::is_keyed(inner) {
            return false;
        }
        // The `&` PARAMETER write-back only — `(B-Ref-Alias)` makes a `&` local bind a live
        // link to its source, which a copy would break.  See the sibling's gate.
        //
        // A FIELD right-hand side (`x = h.hs`) is the same defect and is measured: it left the
        // caller aliasing the holder's collection, so `h.hs += …` was visible through `x`'s
        // caller and the displaced store leaked.  The struct sibling has materialised a field
        // since loft#775; no keyed site ever did.
        let materialisable =
            self.is_field(code) || matches!(code.unspan(), Value::Var(rv) if *rv != var_nr);
        if op != "=" || !self.vars.is_argument(var_nr) || !materialisable {
            return false;
        }
        let mut dep_free = inner.clone();
        if let Some(d) = dep_free.deps_mut() {
            *d = Deps::none();
        }
        let w = self.vars.work_refs(&dep_free, &mut self.lexer);
        self.vars.set_skip_free(w);
        if self.first_pass {
            return false;
        }
        let Some(kt) = self.keyed_type_id(&dep_free) else {
            return false;
        };
        let kt = i32::from(kt);
        // No `0x8000` source-free bit: the source is a live binding — the caller's, or the
        // callee's own local — and its own scope frees it.  That is the same reading
        // `OpReplaceKeyed`'s local-assignment site gives the bit (@FR-O-Move: a store the
        // callee only BORROWED is not the callee's to give away).
        let src = std::mem::replace(code, Value::Null);
        *code = v_block(
            vec![
                v_set(w, Value::Null),
                self.cl("OpDatabase", &[Value::Var(w), Value::Int(kt)]),
                self.cl("OpReplaceKeyed", &[src, Value::Var(w), Value::Int(kt)]),
                Value::Var(w),
            ],
            {
                let mut t = dep_free.clone();
                if let Some(d) = t.deps_mut() {
                    *d = Deps::frame1(w);
                }
                t
            },
            "materialized_amp_keyed",
        );
        false
    }

    /// Bind a destructured tuple element to `v_nr` as a value the binding OWNS,
    /// rather than as a view into the destructured buffer `tmp`.
    ///
    /// A tuple return wider than 8B arrives in a synthetic `__tuple<…>` record held
    /// by a work-ref belonging to the CALL SITE, so one site reuses one buffer.
    /// Reassigning that work-ref frees the store it named — and reassigning it is
    /// exactly what the next turn of a loop does, before the call it feeds.  A
    /// binding left pointing into the buffer therefore dangles from the second
    /// iteration on, in two shapes that are one defect: reading a freed store answers
    /// its cleared contents, so `(xs, n) = f(xs)` silently reports `len` 0 and a
    /// struct field 0 while `--native` answers correctly, and appending onto a record
    /// the arena has since recycled panics in `vector_append` (loft#941).
    ///
    /// P250 gave a `Reference` element a DEPENDENCY on `tmp` so scope analysis would
    /// not emit a second `OpFreeRef` for the binding.  That stops a double free, but a
    /// dependency cannot lengthen the buffer's life past the reassignment; the binding
    /// still outlives the record it was read from.  So copy it out — the same
    /// materialise-the-view move `return <field>` (#306) and `&out = <field>`
    /// (loft#775) already make, which is why those two directions were safe and this
    /// one was not.
    ///
    /// Value-typed elements (integer, boolean, …) are read by value and pass through
    /// untouched; only a record and a vector read back as a pointer into the buffer.
    ///
    /// Answers the whole assignment STATEMENT, not a value to assign: the allocation
    /// writes through `v_nr` itself, so wrapping it as `Set(v_nr, <block ending in
    /// v_nr>)` would make the binding its own initialiser — legal in the IR, but the
    /// native backend renders a first binding as `let mut var_v = <init>` and rustc
    /// rejects the `var_v` inside it.  A flat `Insert` is the shape
    /// [`Self::boxed_cell_alloc_and_set`] already uses for the same reason.
    fn materialize_tuple_element(
        &mut self,
        v_nr: u16,
        tmp: u16,
        elem: &Type,
        view: Value,
    ) -> Value {
        match elem {
            Type::Reference(td, _) => {
                let td = *td;
                let copy_d = self.data.def_nr("OpCopyRecord");
                if copy_d == u32::MAX {
                    // No copy op to reach for: fall back to P250's dependency, which
                    // at least stops scope analysis freeing the buffer's store a
                    // second time through the binding.  `def_nr` answers the same on
                    // both passes, so this arm cannot be a cross-pass difference.
                    self.vars.depend(v_nr, tmp);
                    return Value::Set(v_nr, Box::new(view));
                }
                // Both passes agree the element is a record and add NO dependency;
                // only pass 2 emits the copy, exactly as `assign_refvar_reference`
                // does.  Adding the dependency on pass 1 alone would make the
                // binding's deps differ by pass, which is the divergence the H5
                // contract catches.
                if self.first_pass {
                    return Value::Set(v_nr, Box::new(view));
                }
                let kt = i32::from(self.data.def(td).known_type());
                Value::Insert(vec![
                    v_set(v_nr, Value::Null),
                    self.cl("OpDatabase", &[Value::Var(v_nr), Value::Int(kt)]),
                    Value::Call(copy_d, vec![view, Value::Var(v_nr), Value::Int(kt)]),
                ])
            }
            Type::Vector(elm_tp, _) => {
                let elm_tp = (**elm_tp).clone();
                // `vector_db` gives `v_nr` its own backing store and repoints it there
                // (it is pass-2-only, and answers an empty list for a case that must
                // keep the caller's backing — an argument, a keyed local — where the
                // view is already not the buffer's).
                let mut ops = self.vector_db(elem, v_nr);
                if ops.is_empty() {
                    return Value::Set(v_nr, Box::new(view));
                }
                let rec_tp = Value::Int(self.append_elem_tp(&elm_tp));
                ops.push(self.cl("OpAppendVector", &[Value::Var(v_nr), view, rec_tp]));
                Value::Insert(ops)
            }
            _ => Value::Set(v_nr, Box::new(view)),
        }
    }

    /// Handle `v += expr` and `v = expr` where `v: &vector<T>`; returns true if handled.
    ///
    /// A `&` vector parameter shares the CALLER's store in place (`OpCreateStack` /
    /// `OpGetStackRef`), so there is no op that re-points it at a different store.
    /// `=` therefore means *replace what the caller sees*: clear the shared store and
    /// deep-copy the right-hand side back into it, which is exactly what the literal
    /// form (`v = [1, 2, 3]`, lowered by `create_vector`) already emits.  Every other
    /// right-hand side used to fall through to a plain `Set(v, …)` that codegen
    /// discarded — the write vanished with no diagnostic when some *other* statement
    /// kept the `&` alive, and was reported as "never modified" when it did not
    /// (loft#772).
    ///
    /// NOTE: does NOT intercept `Value::Insert` / `Value::Block` — bracket-form
    /// literals and comprehensions are already handled by the Insert-expansion in
    /// `parse_block` → `OpFinishRecord`, and `create_vector` puts the `=` clear in
    /// front of them.
    pub(crate) fn assign_refvar_vector(
        &mut self,
        code: &mut Value,
        f_type: &Type,
        s_type: &Type,
        op: &str,
        var_nr: u16,
    ) -> bool {
        let Type::RefVar(inner) = f_type else {
            return false;
        };
        let Type::Vector(elm_tp, _) = inner.as_ref() else {
            return false;
        };
        if op != "+=" && op != "=" {
            return false;
        }
        // Bracket-form [elem] and vector comprehensions produce Insert/Block; leave those
        // to the existing parse_block expansion path which uses OpFinishRecord.
        if matches!(code, Value::Insert(_) | Value::Block(_)) {
            return false;
        }
        // A non-vector right-hand side is a type error the general path reports; do not
        // lower it to a shape-mismatched `OpAppendVector` here.
        if op == "=" && !matches!(s_type, Type::Vector(_, _) | Type::RefVar(_)) {
            return false;
        }
        if self.first_pass {
            return true;
        }
        // @P314 — narrow-aware element type (see `append_elem_tp`).
        let elm = (**elm_tp).clone();
        let rec_tp = self.append_elem_tp(&elm);
        if op == "+=" {
            *code = self.cl(
                "OpAppendVector",
                &[Value::Var(var_nr), code.clone(), Value::Int(rec_tp)],
            );
            return true;
        }
        // `v = v` replaces the store with itself: a no-op.  Emitting the clear would
        // wipe it and the append would then read an empty source.
        if matches!(code.unspan(), Value::Var(r) if *r == var_nr) {
            *code = Value::Insert(Vec::new());
            return true;
        }
        let clear = self.cl("OpClearVector", &[Value::Var(var_nr)]);
        // The clear runs BEFORE the copy, so a right-hand side that reads `v` must be
        // materialised into its own store first.  Only a dep-free (owned) variable is
        // provably independent of `v`; a borrow (`w = v; v = w`) and any expression
        // (`v = tail(v)`) are not.  Same three-way split as the struct-field vector
        // replacement above, which is the same invariant one level down.
        // @FR-O-Proxy asks copy — the same aliasing question one level down, for a `&`-bound
        // vector rather than a struct field.  It chooses direct-append vs materialise.
        let owned_var_rhs = matches!(
            code.unspan(),
            Value::Var(rv) if self.vars.tp(*rv).depend().is_empty()
        );
        if owned_var_rhs {
            let append = self.cl(
                "OpAppendVector",
                &[Value::Var(var_nr), code.clone(), Value::Int(rec_tp)],
            );
            *code = Value::Insert(vec![clear, append]);
            return true;
        }
        let rhs_saved = code.clone();
        let dep_free_tp = Type::Vector(Box::new(elm), Deps::none());
        let tmp = self.create_unique("_refvec_rhs", &dep_free_tp);
        self.vars.defined(tmp);
        let mut ls = self.vector_db(&dep_free_tp.content(), tmp);
        ls.push(self.cl(
            "OpAppendVector",
            &[Value::Var(tmp), rhs_saved, Value::Int(rec_tp)],
        ));
        ls.push(clear);
        ls.push(self.cl(
            "OpAppendVector",
            &[Value::Var(var_nr), Value::Var(tmp), Value::Int(rec_tp)],
        ));
        *code = Value::Insert(ls);
        true
    }

    /// Reject a write that a `const` binding forbids, for a COMPONENT target.
    ///
    /// Enforces @FR-Const-Value where the mutation is *through* a name rather than *to*
    /// it — `p.x = …`, `p[i] = …`, `p.a.b = …` — by resolving the write back to its base
    /// binding.  The bare-variable case has no `Value::Var` target here and is
    /// `Parser::const_write_blocked`'s instead; between them the two cover every write.
    ///
    /// ⚠ Construction does NOT come through here (@FR-Const-ConstructExempt): a literal
    /// lowers via `Value::Insert`, so a const field is SET at construction rather than
    /// CHECKED there, and `T{ v: 1 }` is always admitted however `v` is qualified.
    pub(crate) fn validate_write(&mut self, to: &Value, parent_tp: &Type, op: &str) {
        // @PLN40 step 3 — value-const base-resolution.  `validate_write` fires only for
        // a COMPONENT write (`p.x = …`, `p[i] = …`, `p.a.b = …`; the whole-var case has
        // a `Value::Var` target and never reaches here), so any write whose base binding
        // is value-const is a mutation THROUGH a read-only value — reject it at the root.
        // A rebind of the binding itself (`p = other`) re-points the slot and is allowed;
        // it is a bare-`Var` write handled by `const_write_blocked`, not this path.
        if !self.first_pass {
            let base = lhs_base_var(to, &self.data);
            if base != u16::MAX && self.vars.is_value_const(base) {
                // `const_report_var` — see loft#1250.
                let report = self.vars.const_report_var(base);
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot modify {} '{}'; remove 'const' or use a local copy",
                    self.const_noun(report),
                    self.vars.name(report)
                );
            } else if let Some(frozen) = self.lhs_frozen_through(to) {
                // @PLN40 Phase 2 — the write DEREFERENCES THROUGH a value-const field
                // (`s.v[i]=`, `s.v.x=`, deeper): its value is read-only at every depth.
                // A rebind/append of the field ITSELF (`s.v=` / `s.v+=`) is the outermost
                // node — not flagged here — and is decided by the leaf-field block below.
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Cannot modify value-const field '{}'; its value is read-only",
                    frozen
                );
            }
        }
        if let Value::Call(_, vars) = to.unspan()
            && vars.len() > 1
            && let Value::Int(pos) = vars[1].unspan()
        {
            let pos = *pos;
            let d_nr = self.data.type_def_nr(parent_tp);
            if d_nr != u32::MAX {
                let known = self.data.def(d_nr).known_type();
                // @PLN102 K1 — a `const` / value-const field is enforced identically whether it
                // lives on a struct (`Parts::Struct`) or an enum VARIANT (`Parts::EnumValue`).
                // The variant def's `attributes()[f_nr]` aligns with its `EnumValue` field order,
                // so the same const_field / value_const checks below apply unchanged.
                if known != u16::MAX
                    && let Parts::Struct(fields) | Parts::EnumValue(_, fields) =
                        &self.database.types[known as usize].parts
                {
                    // @PLN102 K1 — name the owner accurately: an enum-variant owner is a
                    // "variant", a struct owner a "struct" (the struct wording is unchanged).
                    let owner_kind =
                        if self.data.def(d_nr).def_type() == crate::data::DefType::EnumValue {
                            "variant"
                        } else {
                            "struct"
                        };
                    for (f_nr, f) in fields.iter().enumerate() {
                        if f.position != pos as u16 {
                            continue;
                        }
                        if !self.data.def(d_nr).attributes()[f_nr].mutable {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Cannot write to key field {}.{} create a record instead",
                                self.data.def(d_nr).name(),
                                f.name
                            );
                        } else if self.data.def(d_nr).attributes()[f_nr].const_field {
                            // @PLN40 — a `const` field is write-once at construction.  The
                            // constructor lowers via Value::Insert (a separate path that does
                            // not reach here), so only a later write lands in this guard.
                            // Reject a rebind of the whole value: `=` (any type) or a compound
                            // op (`+=`) on a SCALAR.  ALLOW a compound op on a collection/text
                            // field — that is an in-place append (contents mutation), consistent
                            // with the already-allowed element write `t.v[0] = x`.
                            let contents_append = op != "="
                                && matches!(
                                    self.data.def(d_nr).attributes()[f_nr].typedef,
                                    Type::Text(_)
                                        | Type::Vector(_, _)
                                        | Type::Sorted(_, _, _)
                                        | Type::Index(_, _, _)
                                        | Type::Radix(_, _, _)
                                        | Type::Trie(_, _, _)
                                        | Type::Hash(_, _, _)
                                );
                            if !contents_append {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "cannot reassign const field '{}' of {} '{}' — const fields are write-once-at-construction",
                                    f.name,
                                    owner_kind,
                                    self.data.def(d_nr).name()
                                );
                            }
                        }
                        // @PLN40 Phase 2 — value-const field (`v: const T`).  This is the
                        // LEAF write to `s.v` itself.  Reject a contents mutation (a compound
                        // op `+=` append) while ALLOWING a rebind (`=`) that re-points the
                        // slot.  A by-value SCALAR collapses (no interior distinct from its
                        // binding), so value-const freezes it fully — reject `=` too.  Writes
                        // THROUGH the field (`s.v[i]=`, `s.v.x=`) are inner derefs already
                        // rejected by `lhs_frozen_through` above.  Independent `if` (not
                        // `else`): it COMPOSES with `const_field` so `const v: const T` is
                        // fully frozen — const_field blocks the rebind, value_const the append.
                        if self.data.def(d_nr).attributes()[f_nr].value_const {
                            let collapses = matches!(
                                self.data.def(d_nr).attributes()[f_nr].typedef.base(),
                                Type::Integer(_)
                                    | Type::Float
                                    | Type::Single
                                    | Type::Boolean
                                    | Type::Character
                            );
                            if op != "=" || collapses {
                                diagnostic!(
                                    self.lexer,
                                    Level::Error,
                                    "cannot mutate value-const field '{}' of {} '{}' — its value is read-only (rebind with '=' to re-point, or drop 'const')",
                                    f.name,
                                    owner_kind,
                                    self.data.def(d_nr).name()
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    /// @PLN40 Phase 2 — walk a COMPONENT write's LHS access chain from the base
    /// variable toward the leaf, tracking each node's type, and return the name of the
    /// first value-const FIELD the chain DEREFERENCES THROUGH (an inner step — the
    /// field has a further field-get or an index applied to it: `s.v[i]=`, `s.v.x=`,
    /// `s.v[i].x=`).  Such a write mutates a read-only value and must be rejected.
    ///
    /// The OUTERMOST node is the write TARGET itself, not a dereference, so it is
    /// deliberately skipped: a rebind `s.v = other` re-points the slot (allowed) and a
    /// leaf append `s.v += …` is an op-distinguished contents mutation handled at the
    /// leaf-field block below.  Base-variable value-const (`p.x=` where `p` is a
    /// value-const binding) is handled by the base-resolution block in `validate_write`.
    ///
    /// Read-only.  INERT until wired into `validate_write` (Step 3).  The type-tracking
    /// mirrors the leaf block's `parent_tp`→`known_type`→`Parts::Struct` field lookup,
    /// but applied at every node so an inner field's `value_const` is reachable.
    fn lhs_frozen_through(&self, to: &Value) -> Option<String> {
        if self.first_pass {
            return None;
        }
        // Collect the chain outermost→base by following `args[0]` (as `lhs_base_var`).
        let mut nodes: Vec<&Value> = Vec::new();
        let mut cur = to.unspan();
        while let Value::Call(_, args) = cur {
            if args.is_empty() {
                break;
            }
            nodes.push(cur);
            cur = args[0].unspan();
        }
        let Value::Var(base) = cur else {
            return None;
        };
        if !self.vars.exists(*base) {
            return None;
        }
        // Walk base→leaf, interpreting each node by the CURRENT type: a struct expects a
        // field-get (its `Int(pos)` names the field), a collection expects an index.
        let mut cur_type = self.vars.var_type(*base).clone();
        nodes.reverse();
        let n = nodes.len();
        for (i, node) in nodes.iter().enumerate() {
            let is_leaf = i + 1 == n;
            let Value::Call(_, args) = node.unspan() else {
                return None;
            };
            match cur_type.base() {
                Type::Reference(d_nr, _) => {
                    let d_nr = *d_nr;
                    let Some(Value::Int(pos)) = args.get(1).map(Value::unspan) else {
                        return None;
                    };
                    let known = self.data.def(d_nr).known_type();
                    if known == u16::MAX {
                        return None;
                    }
                    // @PLN102 K1 — a value-const field is read-only through whether it lives on a
                    // struct or an enum variant; walk both `Parts::Struct` and `Parts::EnumValue`.
                    let (Parts::Struct(fields) | Parts::EnumValue(_, fields)) =
                        &self.database.types[known as usize].parts
                    else {
                        return None;
                    };
                    let f_nr = fields.iter().position(|f| f.position == *pos as u16)?;
                    let attr = &self.data.def(d_nr).attributes()[f_nr];
                    if !is_leaf && attr.value_const {
                        return Some(format!("{}.{}", self.data.def(d_nr).name(), attr.name));
                    }
                    cur_type = attr.typedef.clone();
                }
                Type::Vector(_, _)
                | Type::Sorted(_, _, _)
                | Type::Index(_, _, _)
                | Type::Radix(_, _, _)
                | Type::Trie(_, _, _)
                | Type::Hash(_, _, _) => {
                    cur_type = cur_type.content();
                }
                _ => return None,
            }
        }
        None
    }

    /// Materialise an iterator (e.g. `v[a..b]` slice) into a vector variable.
    /// Promotes the LHS variable to `Vector<elm_tp>` and builds a loop that appends
    /// each element in-place.
    pub(crate) fn materialize_iterator(
        &mut self,
        code: &mut Value,
        s_type: &Type,
        to: &Value,
        lhs_parent_tp: &Type,
        var_nr: u16,
        op: &str,
    ) {
        let Type::Iterator(elm_tp, _) = s_type.clone() else {
            unreachable!()
        };
        let elm_tp = *elm_tp;
        let vec_tp = Type::Vector(Box::new(elm_tp.clone()), Deps::none());
        self.change_var(to, &vec_tp);
        if !self.first_pass
            && let Value::Iter(_, init, next, _) = code.clone()
            && matches!(*next, Value::Block(_))
        {
            let ed_nr = self.data.type_def_nr(&elm_tp);
            let fld = Value::Int(i32::from(u16::MAX));
            let elm_var = self.unique_elm_var(lhs_parent_tp, &elm_tp, var_nr);
            let for_var = self.create_unique("slice_elm", &elm_tp);
            // The per-element source is `for_var = next`, a READ of `subject[i]`.  `for_var`
            // carries the read's borrow-dep on the subject (`ref(T)["v"]`), so the free-analysis
            // treats it as BORROWED — never freeing it — and native emits a borrow (no owned
            // copy).  The element is materialised ONCE, by `set_field` deep-copying it into the
            // fresh `elm_var` record.  (Historically an intermediate `comp_var = for_var` sat
            // between the read and the copy; the `= for_var` assignment DROPPED the borrow-dep,
            // so `comp_var` typed as a plain `ref(T)` looked OWNED: the free-analysis emitted
            // `OpFreeRef(comp_var)` — a harmless no-op for an inline struct, but for a STRUCT-ENUM
            // (whose read DEREFERENCES the record pointer) it freed the subject's OWN record,
            // corrupting it on any later read of the same range.  That was an interpret-only
            // miscompile of `v[lo..hi]` on struct-enum vectors; native kept its owned copy so it
            // did not corrupt but paid a redundant copy+free.  Copying `for_var` directly keeps
            // the borrow-dep and removes both faults.)
            let for_next = v_set(for_var, *next);
            let mut lp = vec![for_next];
            // Two DIFFERENT types, one per role — sharing one id here is what let
            // the slice stride differently from the `s += [a[i]]` append it must
            // agree with:
            //   * `OpNewRecord` / `OpFinishRecord` take the CONTAINER being appended
            //     to and stride its slots by that type's content size, so they need
            //     `vector<elm_tp>`;
            //   * `OpCopyRecord` deep-copies ONE element record, so it needs the
            //     ELEMENT type itself.
            // Both come from the shared resolver, so a narrow element (#624 —
            // `u8`/`u16`/4-byte subtypes pack at 1/2/4 bytes, not the wide integer
            // row) and a nested `vector<T>` element (#553 — a 4-byte handle row)
            // land on the same ids the append uses.
            let container_id = Value::Int(i32::from(self.vector_of(&elm_tp)));
            let element_id = Value::Int(i32::from(
                self.data
                    .vector_element_type(&elm_tp, &mut self.database)
                    .unwrap_or(u16::MAX),
            ));
            lp.push(v_set(
                elm_var,
                self.cl(
                    "OpNewRecord",
                    &[Value::Var(var_nr), container_id.clone(), fld.clone()],
                ),
            ));
            // A `vector<T>` element is an AGGREGATE — deep-copy the whole element into the fresh
            // record (as the struct/Reference case does).  `set_field(ed_nr, f_nr=MAX, …)` PEELS
            // `vector<τ>` to its inner `τ` and emits a scalar `OpSetInt4`, storing the element's
            // 12-byte vector DbRef as a 4-byte int — SIGSEGV on interpret, `E0308` on native.
            // `OpCopyRecord` recurses through nesting.  Scalar / struct elements keep set_field.
            if matches!(elm_tp, Type::Vector(_, _)) {
                lp.push(self.cl(
                    "OpCopyRecord",
                    &[Value::Var(for_var), Value::Var(elm_var), element_id],
                ));
            } else if let Some(op) = self.narrow_elm_set(&elm_tp, elm_var, &Value::Var(for_var)) {
                // #624 — a narrow element needs the WIDTH-matched store op; `set_field`
                // below peels to the wide `OpSetInt`, whose 8-byte write covers eight
                // 1-byte element slots at once.  Shared with the `+=` append site.
                lp.push(op);
            } else {
                lp.push(self.set_field(
                    ed_nr,
                    usize::MAX,
                    0,
                    Value::Var(elm_var),
                    Value::Var(for_var),
                ));
            }
            lp.push(self.cl(
                "OpFinishRecord",
                &[Value::Var(var_nr), Value::Var(elm_var), container_id, fld],
            ));
            let needs_db = self.vector_needs_db(var_nr, &elm_tp, true);
            let mut stmts = Vec::new();
            if op == "=" && !needs_db {
                stmts.push(self.cl("OpClearVector", &[Value::Var(var_nr)]));
            }
            stmts.push(*init);
            // #493 — null-init the transient accumulator BEFORE the loop.  The
            // slice iterator's exhaustion branch frees the accumulator
            // (`OpFreeText(for_var)`), which on an EMPTY slice (loop body never
            // runs) would otherwise hit an uninitialised frame slot: garbage
            // DbRef under a normal build, a first-Set self-reference + free_text
            // double-free assert under debug-assertions.  Only text carries the
            // free; the null-init also makes the loop's `for_var = next` a
            // reassignment (text replace), not a self-referencing first-Set.
            if matches!(elm_tp, Type::Text(_)) {
                stmts.push(v_set(for_var, Value::Text(String::new())));
            }
            stmts.push(v_loop(lp, "Slice materialise"));
            if needs_db {
                let db = self.insert_new(var_nr, elm_var, &elm_tp, &mut stmts);
                self.vars.depend(var_nr, db);
            }
            *code = Value::Insert(stmts);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::inline_ref_set_in;
    use crate::data::{Block, Type, Value};

    /// Deep nesting must neither overflow the stack nor change the answer.
    #[test]
    fn inline_ref_set_in_deep_nesting_is_safe() {
        let mut v: Value = Value::Null;
        for _ in 0..1100 {
            v = Value::Block(Box::new(Block {
                name: "",
                operators: vec![v],
                result: Type::Void,
                scope: 0,
                var_size: 0,
            }));
        }
        assert!(!inline_ref_set_in(&v, 0), "no Set node anywhere");
        let mut w: Value = Value::Set(7, Box::new(Value::Null));
        for _ in 0..1100 {
            w = Value::Block(Box::new(Block {
                name: "",
                operators: vec![w],
                result: Type::Void,
                scope: 0,
                var_size: 0,
            }));
        }
        assert!(inline_ref_set_in(&w, 7), "deeply nested Set must be found");
    }
}
