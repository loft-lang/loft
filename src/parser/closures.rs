// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! Closure records: what a lambda captures, the record that carries its captures, the
//! parameters a callee may write through a fn-ref, and the closures a stored type carries
//! and must release.

use super::{
    Data, DefType, HashMap, HashSet, Level, Parser, Type, Value, callee_param_reassigns,
    callee_param_rebinds_owned, diagnostic_format, find_written_vars, v_block, v_if, v_set,
};

/// loft#1874 — for a lambda whose closure record is a fn field's UNION, the releases of the
/// old record's capture slots this lambda's build does NOT write — `if old is live and the
/// slot holds a store { free it }` — and those slots' positions, which the new record must
/// start empty in.  The slots it does write are released against the new capture by the
/// build's own leading ops, as for any rebuild.
pub(super) fn displaced_union_captures(
    p: &mut Parser,
    lambda_d: u32,
    ops: &[Value],
    w_var: u16,
    old: &Value,
) -> (Vec<Value>, Vec<i32>) {
    let record = p.data.def(lambda_d).closure_record();
    if record == u32::MAX || !p.data.def(record).name.starts_with("__closure_u_") {
        return (Vec::new(), Vec::new());
    }
    let set_dbref = p.data.def_nr("OpSetDbRef");
    let mut written: Vec<i32> = Vec::new();
    for op in ops {
        op.walk(&mut |n| {
            if let Value::Call(d, args) = n.unspan()
                && *d == set_dbref
                && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == w_var)
                && let Some(Value::Int(pos)) = args.get(1).map(Value::unspan)
            {
                written.push(*pos);
            }
        });
    }
    let kt = p.data.def(record).known_type();
    let attrs: Vec<(String, Type)> = p
        .data
        .def(record)
        .attributes()
        .iter()
        .map(|a| (a.name.clone(), a.typedef.clone()))
        .collect();
    let mut out = Vec::new();
    let mut unwritten = Vec::new();
    for (name, tp) in attrs {
        if !matches!(tp.base(), Type::Reference(_, deps) if !deps.is_empty()) {
            continue;
        }
        let pos = i32::from(p.database.position(kt, &name));
        if pos == i32::from(u16::MAX) || written.contains(&pos) {
            continue;
        }
        unwritten.push(pos);
        let slot = p.cl("OpGetDbRef", &[old.clone(), Value::Int(pos)]);
        let held = p.cl("OpConvBoolFromRef", std::slice::from_ref(&slot));
        let free = p.cl("OpFreeRef", &[slot]);
        let live = p.cl("OpConvBoolFromRef", std::slice::from_ref(old));
        out.push(v_if(live, v_if(held, free, Value::Null), Value::Null));
    }
    (out, unwritten)
}

/// Every read of variable `v` in `code` becomes `with`.
pub(super) fn replace_var(code: &mut Value, v: u16, with: &Value) {
    if matches!(code.unspan(), Value::Var(x) if *x == v) {
        *code = with.clone();
        return;
    }
    code.for_each_child_mut(&mut |c| replace_var(c, v, with));
}

/// P213: emit the IR for writing a fn-ref to a struct field.
///
/// Three cases:
/// - Inline non-capturing lambda (`Value::FnRef(d, MAX, _)`): write
///   `OpSetInt4(d_nr)`; the closure_rec vector stays empty.
/// - Inline capturing lambda (a `fn_ref_with_closure` Block ending in
///   `Value::FnRef(d, w, _)`): run the lambda's existing alloc_steps
///   (allocate the closure record in parent's Store under `var(w)`),
///   write `OpSetInt4(d_nr)`, then `OpAppendVector` deep-copies the
///   parent-Store closure record into element [0] of the host's
///   `__closure_rec` vector field.  Parent's record is freed by the
///   existing parent-scope `OpFreeRef(w)`; host owns its own copy.
/// - Non-inline source (`Var` / `Call` returning a fn-ref) or unrecognised
///   shape: emit a placeholder `OpSetInt4(0)` write and a parse-time
///   diagnostic so the user sees the limitation in the second pass.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub(super) fn emit_fn_ref_field_write(
    p: &mut Parser,
    d_nr: u32,
    f_nr: usize,
    ref_code: Value,
    pos_val: Value,
    val_code: &Value,
) -> Value {
    let unspanned = val_code.unspan().clone();
    match unspanned {
        Value::FnRef(d, w, tp) if w == u16::MAX => {
            // Non-capturing inline lambda — just write the d_nr.
            p.fn_number_write(ref_code, pos_val, d, &tp)
        }
        Value::Block(bl) if bl.name == "fn_ref_with_closure" => {
            // Capturing inline lambda.  bl.operators = [
            //   alloc_steps...,
            //   FnRef(d, w, _)  // last element
            // ].
            let last = bl.operators.last().cloned().unwrap_or(Value::Null);
            let Value::FnRef(d, w, lambda_tp) = last.unspan() else {
                if !p.first_pass {
                    diagnostic!(
                        p.lexer,
                        Level::Error,
                        "internal: fn_ref_with_closure block did not end in FnRef"
                    );
                }
                return Value::Null;
            };
            let (lambda_d, w_var, lambda_tp) = (*d, *w, lambda_tp.as_ref().clone());
            let mut ops: Vec<Value> = bl
                .operators
                .iter()
                .take(bl.operators.len() - 1)
                .cloned()
                .collect();
            // `@FR-L-Escape`, `@FR-L-CapOwn` — the record is built IN PLACE in the host's child
            // slot (`OpChildRec`, loft#1867): `w` names that slot's record, the captures are
            // written straight into it, and it is the HOST's, released with it and by its
            // cascade.  Built apart and copied in, the frame then released the record it had
            // built, and its cascade released the captures the copy still named.
            let in_place = if w_var != u16::MAX && f_nr != usize::MAX && !p.first_pass {
                let closure_rec_d = p.data.def(lambda_d as u32).closure_record();
                (closure_rec_d != u32::MAX).then(|| {
                    let kt = p.data.def(closure_rec_d).known_type();
                    let crec_pos = match &pos_val {
                        Value::Int(pi) => Value::Int(pi + 4),
                        _ => Value::Int(0),
                    };
                    let field = p.cl(
                        "OpGetField",
                        &[ref_code.clone(), crec_pos, Value::Int(i32::from(kt))],
                    );
                    (kt, field)
                })
            } else {
                None
            };
            if let Some((kt, field)) = &in_place {
                let database = p.data.def_nr("OpDatabase");
                // The release of what the record held before reads that record where it lives,
                // in the field: the local names the record this SITE built last, which may sit
                // in another host or be gone.
                let old = p.cl("OpRefFromChildRec", std::slice::from_ref(field));
                let build_at = ops.iter().position(|op| {
                    matches!(op.unspan(), Value::Call(d, args) if *d == database
                        && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == w_var))
                });
                if let Some(at) = build_at {
                    for op in &mut ops[..at] {
                        replace_var(op, w_var, &old);
                    }
                    // loft#1874 — a fn field's UNION record (`unite_field_closure_records`) may
                    // hold another lambda's captures: each capture slot this build does not
                    // write is released from the old record as well, so the closure it replaces
                    // gives up what it held whichever lambda that was.
                    let (releases, unwritten) =
                        displaced_union_captures(p, lambda_d as u32, &ops, w_var, &old);
                    let n = releases.len();
                    for (i, r) in releases.into_iter().enumerate() {
                        ops.insert(at + i, r);
                    }
                    // …and the slots it does not write start as nothing: the new record is
                    // claimed with whatever bytes the store held there (C137), and its cascade
                    // reads every capture slot of the union.
                    let null = p.data.def_nr("OpNullRefSentinel");
                    for (i, pos) in unwritten.into_iter().enumerate() {
                        let clear = p.cl(
                            "OpSetDbRef",
                            &[
                                Value::Var(w_var),
                                Value::Int(pos),
                                Value::Call(null, Vec::new()),
                            ],
                        );
                        ops.insert(at + n + 1 + i, clear);
                    }
                }
                for op in &mut ops {
                    if matches!(op.unspan(), Value::Call(d, args) if *d == database
                        && matches!(args.first().map(Value::unspan), Some(Value::Var(x)) if *x == w_var))
                    {
                        let child =
                            p.cl("OpChildRec", &[field.clone(), Value::Int(i32::from(*kt))]);
                        *op = v_set(w_var, child);
                    }
                }
                p.vars.set_skip_free(w_var);
            }
            // Write the d_nr at the loft-attribute position (which maps
            // to the database-side `<attr>` field — the d_nr half).
            ops.push(p.fn_number_write(ref_code.clone(), pos_val.clone(), lambda_d, &lambda_tp));
            // Deep-copy the parent-Store closure record into the host's
            // `__closure_rec` vector at pos+4.  We need the host's
            // closure_rec field as a DbRef + the closure record's
            // known_type for `OpAppendVector`'s type parameter.
            v_block(ops, Type::Void, "fn_ref_field_set")
        }
        Value::Var(v) => {
            // P215: lift the deferred diagnostic when both sides are
            // non-capturing — target field is 4B int layout
            // (`assigned_lambda_d_nr == u32::MAX`) AND the source var
            // is not in `closure_vars` (presence in that map signals a
            // capturing-lambda assignment per
            // `parser/expressions.rs:1217`).  In that case writing
            // just the d_nr is lossless: the source's closure DbRef
            // component is the null sentinel and there's no
            // `__closure_rec` half on the target to receive it.
            //
            // Closure-record-internal fn-ref fields always satisfy the
            // target check (synthesize_closure_record never sets
            // `assigned_lambda_d_nr`), so capturing fn-ref names from
            // outer scopes now write through to the closure record
            // when the captured lambda itself is non-capturing —
            // which is the canonical P215 reproducer.
            // Like the read side, the split/legacy answer must come
            // from the database layout (`fn_ref_field_is_split`), not
            // the body-parse-order-dependent flag (#313).
            let target_is_4b = if (d_nr as usize) < p.data.definitions.len()
                && f_nr < p.data.def(d_nr).attributes().len()
            {
                !p.fn_ref_field_is_split(d_nr, f_nr)
            } else {
                false
            };
            let source_is_noncapturing =
                matches!(p.vars.tp(v), Type::Function(..)) && p.vars.closure_var_of(v).is_none();
            if target_is_4b && source_is_noncapturing {
                return p.cl("OpSetInt4", &[ref_code, pos_val, Value::FnRefDnr(v)]);
            }
            // loft#1869, `@FR-L-Fn` — a closure record's capture of a fn-ref keeps the d_nr here
            // and the closure half in the `<name>__clos` capture beside it.
            if target_is_4b
                && p.data
                    .attr(d_nr, &format!("{}__clos", p.data.attr_name(d_nr, f_nr)))
                    != usize::MAX
            {
                return p.cl("OpSetInt4", &[ref_code, pos_val, Value::FnRefDnr(v)]);
            }
            if !p.first_pass {
                diagnostic!(
                    p.lexer,
                    Level::Error,
                    "only inline lambda literals can be stored in fn-ref struct fields; \
                     non-inline (variable / call) sources are not yet supported when the \
                     source closure may capture (P215-deferred)"
                );
            }
            // Emit a no-op so the rest of construction proceeds.
            p.cl("OpSetInt4", &[ref_code, pos_val, Value::Int(0)])
        }
        Value::Call(_, _) => {
            // P213-deferred: non-inline source.  Capturing-call
            // sources need closure-DbRef copy; deferred.
            if !p.first_pass {
                diagnostic!(
                    p.lexer,
                    Level::Error,
                    "only inline lambda literals can be stored in fn-ref struct fields in this release; \
                     bind the lambda directly inside the struct constructor"
                );
            }
            p.cl("OpSetInt4", &[ref_code, pos_val, Value::Int(0)])
        }
        other => {
            // The INTERNAL shapes construction produces: a default-init `Null`, a raw
            // d_nr already lowered to an `Int`, and a `FnRef` with no closure half.  Each
            // really is four bytes wide, so writing four bytes is right for them.
            //
            // Anything ELSE reaching here is a non-inline SOURCE — an `if` or `match` arm,
            // a `??` — and it takes the same refusal `Var` and `Call` take above.  It used
            // to fall through and write four bytes of a value that is twenty wide: the
            // closure half was dropped, and calling through the field then SIGSEGV'd on
            // `--interpret` and hit `unreachable!("invalid fn-ref")` on `--native`, at
            // different arms of the same program.  The two spellings that WERE enumerated
            // got a diagnostic; the ones nobody thought of got the crash, which is the
            // difference this arm removes.
            // loft#1869 — a closure record relaying a captured fn-ref from the record of the
            // lambda it is built in: the d_nr half of that record's read lands here, and the
            // closure half fills the `<name>__clos` capture beside it.
            let relays_fn_ref = (d_nr as usize) < p.data.definitions.len()
                && f_nr < p.data.def(d_nr).attributes().len()
                && p.data
                    .attr(d_nr, &format!("{}__clos", p.data.attr_name(d_nr, f_nr)))
                    != usize::MAX;
            let d_nr_only = match other {
                Value::Block(bl)
                    if relays_fn_ref
                        && bl.name == "fn_ref_field_read"
                        && !bl.operators.is_empty() =>
                {
                    bl.operators[0].clone()
                }
                Value::FnRef(d, _, tp) => return p.fn_number_write(ref_code, pos_val, d, &tp),
                Value::Int(n) if n != 0 => {
                    let tp = p.data.attr_type(d_nr, f_nr);
                    return p.fn_number_write(ref_code, pos_val, n, &tp);
                }
                Value::Int(n) => Value::Int(n),
                Value::Null => Value::Int(0),
                _ => {
                    if !p.first_pass {
                        diagnostic!(
                            p.lexer,
                            Level::Error,
                            "only inline lambda literals can be stored in fn-ref struct \
                             fields; a non-inline source (a variable, a call, an \
                             `if`/`match` arm) is not yet supported when the source \
                             closure may capture (P215-deferred)"
                        );
                    }
                    // Emit a no-op so the rest of construction proceeds, exactly as the
                    // `Var` and `Call` refusals above do.
                    Value::Int(0)
                }
            };
            p.cl("OpSetInt4", &[ref_code, pos_val, d_nr_only])
        }
    }
}

/// The type of an `if`/`else` whose arms have been unified: the THEN arm's shape,
/// carrying what EITHER arm can alias.
///
/// The shape comes from `a` because the arms are already unified by the time this
/// runs; only the borrow list has two answers to merge.  It is a UNION, not a
/// pick: an arm yielding a fresh record has an empty dep list, which every free
/// site reads as owned, so taking that arm's answer for the whole join freed a
/// container view the other arm had delivered (loft#978 — silent, both backends).
/// Text was the only type this merged until then, so the rule existed but reached
/// one variant of the many that carry deps.
pub(super) fn merge_dependencies(a: &Type, b: &Type) -> Type {
    // Never (return/break/continue) defers to the other branch's type.
    if matches!(a, Type::Never) {
        return b.clone();
    }
    if matches!(b, Type::Never) {
        return a.clone();
    }
    a.joined_deps(b)
}

/// Collect all `Value::Var` indices reachable anywhere in `val`.
pub(super) fn collect_vars_in(val: &Value, result: &mut crate::fxhash::FxHashSet<u16>) {
    match val {
        // A tuple MEMBER names its tuple (`bump(t.0)` hands a place inside `t`).
        Value::Var(v) | Value::TupleGet(v, _) => {
            result.insert(*v);
        }
        Value::Set(_, body) => collect_vars_in(body, result),
        Value::Call(_, args) => {
            for a in args {
                collect_vars_in(a, result);
            }
        }
        Value::Block(b) | Value::Loop(b) => {
            for op in &b.operators {
                collect_vars_in(op, result);
            }
        }
        Value::Insert(list) => {
            for item in list {
                collect_vars_in(item, result);
            }
        }
        Value::If(c, t, e) => {
            collect_vars_in(c, result);
            collect_vars_in(t, result);
            collect_vars_in(e, result);
        }
        Value::Return(v) | Value::Drop(v) => collect_vars_in(v, result),
        Value::Iter(_, a, b, c) => {
            collect_vars_in(a, result);
            collect_vars_in(b, result);
            collect_vars_in(c, result);
        }
        Value::Span(b) => collect_vars_in(&b.1, result),
        _ => {}
    }
}

/// The capture NAMES that lambdas defined inside `code` write to.
///
/// `find_written_vars` walks one function's own body, and a lambda is a separate
/// definition — so a parameter whose only write happens inside a closure reads as never
/// written there.  For a `&` parameter that turns into an assertion that is not merely
/// unhelpful but false: `fn bump(p: &integer) { g = fn() { p += 1; }; g(); }` was refused
/// with *"Parameter 'p' has & but is never modified; remove the &"*, and following that
/// cure turns an out-parameter into a by-value one — so the reader loses the write-back
/// the signature exists for (loft#1276).
///
/// The lambda records what it writes as `Definition::mutated_captures`, by NAME, which is
/// why this answers in names: the caller matches them against its own parameter names.
/// Same class as @P320, where a keyed remove was the write `find_written_vars` could not
/// see; here it is the whole lambda body.
pub(super) fn lambda_mutated_captures(code: &Value, data: &Data, names: &mut HashSet<String>) {
    match code.unspan() {
        Value::FnRef(d_nr, _, _) if *d_nr >= 0 && (*d_nr as usize) < data.definitions.len() => {
            for n in data.def(*d_nr as u32).mutated_captures() {
                names.insert(n.clone());
            }
        }
        Value::Call(_, args) => {
            for a in args {
                lambda_mutated_captures(a, data, names);
            }
        }
        Value::Block(block) | Value::Loop(block) => {
            for item in &block.operators {
                lambda_mutated_captures(item, data, names);
            }
        }
        Value::Insert(list) | Value::Tuple(list) | Value::Parallel(list) => {
            for item in list {
                lambda_mutated_captures(item, data, names);
            }
        }
        Value::If(cond, then, els) => {
            lambda_mutated_captures(cond, data, names);
            lambda_mutated_captures(then, data, names);
            lambda_mutated_captures(els, data, names);
        }
        Value::Set(_, v)
        | Value::Return(v)
        | Value::Drop(v)
        | Value::Yield(v)
        | Value::TuplePut(_, _, v) => lambda_mutated_captures(v, data, names),
        Value::Iter(_, create, next, extra) => {
            lambda_mutated_captures(create, data, names);
            lambda_mutated_captures(next, data, names);
            lambda_mutated_captures(extra, data, names);
        }
        _ => {}
    }
}

/// for the given user-defined function, return a boolean per
/// parameter indicating whether its body writes that parameter
/// (directly or through a transitive call).  Results are memoised
/// in `cache`; a placeholder (all-false) is inserted before recursive
/// analysis so cycles are broken.  Caller should iterate to fixpoint
/// if precise transitive effects across recursion chains are needed;
/// for linear forwarding (the common case) one pass suffices.
pub(super) fn callee_param_writes(
    fn_nr: u32,
    data: &Data,
    cache: &mut crate::fxhash::FxHashMap<u32, Vec<bool>>,
) -> Vec<bool> {
    if let Some(v) = cache.get(&fn_nr) {
        return v.clone();
    }
    let def = data.def(fn_nr);
    let n = def.attributes().len();
    // Break recursion: insert a placeholder before walking the body.
    cache.insert(fn_nr, vec![false; n]);
    if *def.code() == Value::Null || n == 0 {
        return vec![false; n];
    }
    let body = def.code().clone();
    let mut written = crate::fxhash::FxHashSet::default();
    find_written_vars(&body, data, def.variables(), &mut written, cache);
    let result: Vec<bool> = (0..n).map(|i| written.contains(&(i as u16))).collect();
    // Monotone merge with any prior placeholder entry.
    let prev = cache.get(&fn_nr).cloned().unwrap_or_else(|| vec![false; n]);
    let merged: Vec<bool> = prev
        .iter()
        .zip(result.iter())
        .map(|(a, b)| *a || *b)
        .collect();
    cache.insert(fn_nr, merged.clone());
    merged
}

/// Walk `code` marking every parameter slot that is reassigned — directly by a `Value::Set`,
/// or by being handed on to a callee that reassigns the parameter it lands in.
pub(super) fn collect_param_reassigns(
    code: &Value,
    data: &Data,
    n_params: usize,
    out: &mut [bool],
    cache: &mut HashMap<u32, Vec<bool>>,
) {
    if let Value::Set(v, _) = code.unspan()
        && (*v as usize) < n_params
    {
        out[*v as usize] = true;
    }
    if let Value::Call(fn_nr, args) = code.unspan()
        && *data.def(*fn_nr).code() != Value::Null
    {
        let callee = callee_param_reassigns(*fn_nr, data, cache);
        for (i, arg) in args.iter().enumerate() {
            if i < callee.len()
                && callee[i]
                && let Value::Var(v) = arg.unspan()
                && (*v as usize) < n_params
            {
                out[*v as usize] = true;
            }
        }
    }
    code.for_each_child(&mut |child| collect_param_reassigns(child, data, n_params, out, cache));
}

/// Walk `code` marking every parameter slot rebound to an OWNED value — directly by a
/// `Value::Set`, or by being handed on to a callee that rebinds the parameter it lands in.
pub(super) fn collect_param_rebinds_owned(
    code: &Value,
    fn_nr: u32,
    data: &Data,
    n_params: usize,
    out: &mut [bool],
    cache: &mut HashMap<u32, Vec<bool>>,
) {
    if let Value::Set(v, rhs) = code.unspan()
        && (*v as usize) < n_params
        && !matches!(rhs.unspan(), Value::Null)
        && matches!(
            crate::use_analysis::ownership_of(data, fn_nr, rhs),
            crate::use_analysis::Own::Owned
        )
    {
        out[*v as usize] = true;
    }
    if let Value::Call(callee, args) = code.unspan()
        && *data.def(*callee).code() != Value::Null
    {
        let inner = callee_param_rebinds_owned(*callee, data, cache);
        for (i, arg) in args.iter().enumerate() {
            if i < inner.len()
                && inner[i]
                && let Value::Var(v) = arg.unspan()
                && (*v as usize) < n_params
            {
                out[*v as usize] = true;
            }
        }
    }
    code.for_each_child(&mut |child| {
        collect_param_rebinds_owned(child, fn_nr, data, n_params, out, cache);
    });
}

impl Parser {
    /// #318: does `tp` (transitively) store a capturing closure — a
    /// struct with a capturing-lambda fn field (`assigned_lambda_d_nr`
    /// set), reached through struct fields, vector/keyed-collection
    /// content, or tuple elements?
    ///
    /// Values of such types are frame-bound: the closure record holds
    /// raw DbRefs into the stores of the frame that owns the captures,
    /// and copying the value into storage that outlives that frame (a
    /// return value, another struct's field, a collection element)
    /// leaves dangling DbRefs — silent cross-object corruption once
    /// the store slot is reused.  The three escape sinks reject on
    /// this predicate; locals and downward argument passing stay free.
    /// loft#1877, `@FR-L-CapOwn` — the closure records an overwrite of the place `to` (a
    /// field holding a value of type `tp`) displaces, bound so their cascades run: `to`
    /// itself when the value holds none.  Each fn field of the value, and of every INLINE
    /// struct member of it, keeps its closure record as a child record in the host's store;
    /// the copy that overwrites the place frees those records' bytes, and what they ADOPTED
    /// is released by their cascades, which the scope pass places after each `___oldrec`
    /// bind (`capture_adoption::cascade_before_record_frees`).  Hooks do not run:
    /// `(H-Drop-Not)` leaves an overwritten field's droppables to the author, and adopted
    /// captures are not droppables but stores the closure owns.
    ///
    /// Built as the place argument of the overwriting copy, so it runs after the right-hand
    /// side is computed — which may read the old value — and before the copy replaces it.
    pub(crate) fn release_displaced_closures(&mut self, to: &Value, tp: &Type) -> Value {
        fn collect(
            p: &Parser,
            tp: &Type,
            path: &[(u16, u16)],
            out: &mut Vec<(Vec<(u16, u16)>, u16, u32)>,
            seen: &mut Vec<u32>,
        ) {
            let Type::Reference(d, deps) = tp.base() else {
                return;
            };
            if deps.contains(&u16::MAX) || seen.contains(d) {
                return;
            }
            seen.push(*d);
            let kt = p.data.def(*d).known_type();
            for a in p.data.def(*d).attributes() {
                let crec = p.database.position(kt, &format!("{}__closure_rec", a.name));
                if crec != u16::MAX && a.assigned_lambda_d_nr != u32::MAX {
                    let record = p.data.def(a.assigned_lambda_d_nr).closure_record();
                    if record != u32::MAX {
                        out.push((path.to_vec(), crec, record));
                    }
                } else if let Type::Reference(m, mdeps) = a.typedef.base()
                    && !mdeps.contains(&u16::MAX)
                {
                    let pos = p.database.position(kt, &a.name);
                    if pos != u16::MAX {
                        let mut deeper = path.to_vec();
                        deeper.push((pos, p.data.def(*m).known_type()));
                        collect(p, &a.typedef, &deeper, out, seen);
                    }
                }
            }
            seen.pop();
        }
        if self.first_pass || !to.is_place_read(&self.data) {
            return to.clone();
        }
        let mut fields = Vec::new();
        collect(self, tp, &[], &mut fields, &mut Vec::new());
        if fields.is_empty() {
            return to.clone();
        }
        let mut ops = Vec::new();
        for (path, crec, record) in fields {
            let mut place = to.clone();
            for (pos, kt) in path {
                place = self.cl(
                    "OpGetField",
                    &[place, Value::Int(i32::from(pos)), Value::Int(i32::from(kt))],
                );
            }
            let field = self.cl(
                "OpGetField",
                &[place, Value::Int(i32::from(crec)), Value::Int(0)],
            );
            let old = self.create_unique(
                "__oldrec",
                &Type::Reference(record, crate::data::Deps::none()),
            );
            self.vars.set_skip_free(old);
            ops.push(v_set(old, self.cl("OpRefFromChildRec", &[field])));
        }
        ops.push(to.clone());
        v_block(ops, tp.clone(), "displaced_closures")
    }

    /// The return deps an instance's `-> T` record return was declared without, from its body.
    ///
    /// @FR-F-Ret / @FR-O-Oracle — a template's `-> T` record return carries NO deps: its
    /// `ref_return` is skipped (the promotion is deferred to instantiation), so the
    /// `MergeAttr` that writes `-> Ctr["x"]` on a concrete twin never ran, and the instance
    /// read as a fresh owner while its body hands the ARGUMENT up.  The caller then bound
    /// the argument's own store and a write through the result wrote the argument, on both
    /// backends (QUALITY-history.md B7t: struct/vector, whole/local/early/arm).  Ask the ONE
    /// derivation the body itself answers to — the oracle's return summary — and let the
    /// instance's declared return say what its twin's says, so the caller copies a
    /// borrowed return exactly as it does for a named function (loft#1346).
    ///
    /// A pure BORROW only.  A `Join` — a mint on one arm and the argument on the other —
    /// is delivered through a return buffer on a named function (`ref_return`'s per-arm
    /// leg), which a monomorph does not have yet; a dep alone would make the caller copy
    /// the mint arm and orphan the minted store.  That shape stays as it is, named in
    /// QUALITY-history.md B7t as the residual with its cure.
    ///
    /// And only where every return LEAF is the parameter itself.  A local bound from it
    /// (`y: T = x; y`) COPIES at codegen for a record (@FR-B-Copy) — a copy the IR does
    /// not show, so the oracle reads the local as a borrow of `x` — and declaring that a
    /// borrow made the caller decline its lift and free nothing: three corpus generics
    /// leaked one record per call under `LOFT_STRICT_STORES`.  What comes back through a
    /// local is owned, and the caller adopts it as before.
    pub(super) fn derive_monomorph_return_deps(&mut self, d_nr: u32, g_nr: u32) {
        let new_returned = self.data.def(d_nr).returned().clone();
        if !(new_returned.depend().is_empty() && crate::data::has_lifetime_concern(&new_returned)) {
            return;
        }
        let attrs_n = self.data.def(d_nr).attributes().len();
        // loft#1880 — the oracle reads a SELF call as a mint, so a recursive instance that
        // only ever hands its argument's view along (`el(v, i - 1)`, base case `v[0]`)
        // reads as a `Join`; the leaf walk settles it by the induction the recursion is.
        // The self call still names the TEMPLATE here (`instantiate_nested_generics` runs
        // below), and handed the same parameter it can only reach this instance.
        if let crate::use_analysis::Own::Borrowed { base } | crate::use_analysis::Own::Join { base } =
            crate::use_analysis::return_ownership(&self.data, d_nr)
            && (base as usize) < attrs_n
            && !self.data.def(d_nr).attributes()[base as usize].hidden
            && matches!(&self.data.def(d_nr).code, Value::Block(bl)
                if Self::every_return_leaf_views_var(&self.data, &bl.operators, base, g_nr))
        {
            // Written directly: `set_returned` refuses a second write on purpose (a return
            // type must not change), and this does not change it — it adds the deps the
            // type was declared without.
            //
            // ATTR space, not frame: `base` is an attribute index — the guard above tests
            // it against `attributes().len()` — and `Definition.returned` is a DEF-space
            // home, so `Deps::attrs` is what states it.  `Type::depending` builds
            // `Deps::frame1`, which tags the same number as a caller FRAME variable, and
            // `call_dependencies` reads this list with `as_attr_indices`.
            let with_dep = self
                .data
                .def(d_nr)
                .returned()
                .clone()
                .with_deps(&crate::data::Deps::attrs(vec![base]));
            self.data.definitions[d_nr as usize].returned = with_dep;
        } else if let Some(params) = self.monomorph_views_of_params(d_nr, g_nr) {
            // loft#1880 — no ONE parameter, but a SET of them: a recursion that swaps its
            // arguments (`sw(b, a, i - 1)`, base case `a[0]`) hands back a view of `a` or of
            // `b`, which its twin declares `-> T["a", "b"]`.
            let with_dep = self
                .data
                .def(d_nr)
                .returned()
                .clone()
                .with_deps(&crate::data::Deps::attrs(params));
            self.data.definitions[d_nr as usize].returned = with_dep;
        }
    }

    /// loft#1880 — the visible heap parameters an instance's every return leaf is a VIEW into
    /// (`v[0]`, `w.h`), with self calls read by induction
    /// ([`Self::every_return_leaf_views_one_of`]), or `None`.  A leaf that is a bare parameter
    /// is left to `bind_monomorph_join_return`, which binds that Join into an owned local: the
    /// result is then the instance's own, and declaring it a borrow too would make the caller
    /// copy it and drop the instance's copy.
    pub(super) fn monomorph_views_of_params(&self, d_nr: u32, g_nr: u32) -> Option<Vec<u16>> {
        let def = self.data.def(d_nr);
        let Value::Block(bl) = &def.code else {
            return None;
        };
        let params: Vec<u16> = def
            .attributes()
            .iter()
            .enumerate()
            .filter(|(_, a)| !a.hidden && crate::data::has_lifetime_concern(&a.typedef))
            .filter_map(|(i, _)| u16::try_from(i).ok())
            .collect();
        if params.is_empty() {
            return None;
        }
        fn bare_param_leaf(v: &Value, vars: &crate::variables::Function, tail: bool) -> bool {
            match v.unspan() {
                Value::Return(inner) => bare_param_leaf(inner, vars, true),
                Value::If(_, t, e) => {
                    bare_param_leaf(t, vars, tail) || bare_param_leaf(e, vars, tail)
                }
                Value::Block(b) | Value::Loop(b) => {
                    let n = b.operators.len();
                    b.operators
                        .iter()
                        .enumerate()
                        .any(|(i, op)| bare_param_leaf(op, vars, tail && i + 1 == n))
                }
                Value::Var(x) => tail && vars.is_argument(*x),
                _ => false,
            }
        }
        // …a Join only: where no leaf mints (`if c { return a; } b`, a BORROW of one of
        // several parameters), nothing binds an owned local, and the instance declares the set
        // its twin does (`-> F["a", "b"]`) so the caller copies (loft#1884).
        let n = bl.operators.len();
        let borrows_only = matches!(
            crate::use_analysis::return_ownership(&self.data, d_nr),
            crate::use_analysis::Own::Borrowed { .. }
        );
        if !borrows_only
            && bl
                .operators
                .iter()
                .enumerate()
                .any(|(i, op)| bare_param_leaf(op, &def.variables, i + 1 == n))
        {
            return None;
        }
        Self::every_return_leaf_views_one_of(&self.data, &bl.operators, &params, g_nr)
            .then_some(params)
    }

    /// loft#1874 — point lambda `b` at the closure record lambda `a` already shares with any
    /// other lambda written into the same fn field, growing that union record by `b`'s captures
    /// (by name).  The first union gets a record of its own, `__closure_u_<a's record>`, so
    /// neither lambda's own record changes shape.  A captured name the two type differently
    /// gets a slot per lambda ([`Self::capture_alias`]).
    pub(super) fn unite_field_closure_records(&mut self, a: u32, b: u32) {
        let ra = self.data.def(a).closure_record();
        let rb = self.data.def(b).closure_record();
        if ra == u32::MAX || rb == u32::MAX || ra == rb {
            return;
        }
        let union = if self.data.def(ra).name.starts_with("__closure_u_") {
            ra
        } else {
            let name = format!("__closure_u_{}", self.data.def(ra).name);
            let pos = *self.data.def(ra).position();
            let u = self.data.add_def(&name, &pos, DefType::Struct);
            self.merge_closure_attrs(u, ra, a);
            u
        };
        self.merge_closure_attrs(union, rb, b);
        for l in [a, b] {
            self.data.definitions[l as usize].closure_record = union;
        }
    }

    /// Add closure record `from`'s attributes — lambda `lambda`'s captures — to `into` by name.
    /// A name `into` already holds as another type gets an attribute of its own for this
    /// lambda, `<name>__<lambda>`, recorded in [`Self::capture_alias`].
    pub(super) fn merge_closure_attrs(&mut self, into: u32, from: u32, lambda: u32) {
        let attrs: Vec<(String, Type, bool)> = self
            .data
            .def(from)
            .attributes()
            .iter()
            .map(|a| (a.name.clone(), a.typedef.clone(), a.value_const))
            .collect();
        for (name, tp, value_const) in attrs {
            let at = self.data.attr(into, &name);
            let slot = if at == usize::MAX {
                name.clone()
            } else if self.data.attr_type(into, at) == tp {
                continue;
            } else {
                let alias = format!("{name}__{lambda}");
                self.capture_alias
                    .insert((lambda, name.clone()), alias.clone());
                alias
            };
            let at = self.data.add_attribute(&mut self.lexer, into, &slot, tp);
            self.data.definitions[into as usize].attributes[at].value_const = value_const;
            if let Some(shared) = self
                .closure_shared_vectors
                .get(&(from, name.clone()))
                .cloned()
            {
                self.closure_shared_vectors.insert((into, slot), shared);
            }
        }
    }

    /// The attribute of closure record `rec` that holds lambda `lambda`'s capture `name` —
    /// `name` itself, or the alias a fn field's union record gave it (loft#1874).
    pub(crate) fn capture_attr(&self, lambda: u32, rec: u32, name: &str) -> usize {
        match self.capture_alias.get(&(lambda, name.to_string())) {
            Some(alias) => self.data.attr(rec, alias),
            None => self.data.attr(rec, name),
        }
    }

    pub(crate) fn type_carries_closure(&self, tp: &Type) -> bool {
        // Like `fn_ref_field_is_split`, derive from the registered
        // database layout (built from the COMPLETE first pass) rather
        // than `assigned_lambda_d_nr`, so the answer is independent of
        // pass-2 body-parse order.
        fn walk_def(
            data: &Data,
            db: &crate::database::Stores,
            d: u32,
            seen: &mut std::collections::HashSet<u32>,
        ) -> bool {
            if d == u32::MAX || (d as usize) >= data.definitions.len() || !seen.insert(d) {
                return false;
            }
            let kt = data.def(d).known_type();
            data.def(d).attributes().iter().any(|a| {
                db.position(kt, &format!("{}__closure_rec", a.name)) != u16::MAX
                    || walk(data, db, &a.typedef, seen)
            })
        }
        fn walk(
            data: &Data,
            db: &crate::database::Stores,
            tp: &Type,
            seen: &mut std::collections::HashSet<u32>,
        ) -> bool {
            match tp {
                // #328: a `reference<T>` POINTER field (the u16::MAX share
                // marker) copies only a 12-byte DbRef, never the record —
                // it cannot smuggle a closure record's bytes, so the
                // carrying-walk stops at the pointer boundary.  (Escaping
                // a pointer to frame-local state is the generic, documented
                // reference<T> borrow hazard — not the #318 copy class.)
                Type::Reference(_, deps) if deps.contains(&u16::MAX) => false,
                Type::Reference(d, _) => walk_def(data, db, *d, seen),
                Type::Vector(c, _) => walk(data, db, c, seen),
                Type::Hash(d, _, _)
                | Type::Sorted(d, _, _)
                | Type::Index(d, _, _)
                | Type::Radix(d, _, _)
                | Type::Trie(d, _, _) => walk_def(data, db, *d, seen),
                Type::Tuple(elems) => elems.iter().any(|e| walk(data, db, e, seen)),
                _ => false,
            }
        }
        walk(
            &self.data,
            &self.database,
            tp,
            &mut std::collections::HashSet::new(),
        )
    }
}
