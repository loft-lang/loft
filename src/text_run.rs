// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-TextRun` — a byte vector built only as a run of another, then read once as text, is
//! never built: the text is read off the run in place.
//!
//! `tb: vector<u8> = []; for k in 0..n { tb += [bytes[a + k] ?? 0]; } … text_from_bytes(tb)`
//! — how a binary decoder (cbor's `read_value`) takes a text out of its frame — claimed the
//! vector's wrapper and its element record, copied the run, converted it and released both:
//! two objects per text that exist only to be read once.  `(R-ByteCopy)`'s vector clause
//! (`byte_copy.rs`, which runs first) has already put the copy behind its in-range guard `g`;
//! this pass finishes the job where the vector is that and nothing more:
//!
//! - the declaration and the copy move under `if !g` — on the guard's path `tb` is never
//!   claimed, so its wrapper stays null and its frees are no-ops (`OpFreeRecordIn` and
//!   `OpFreeRef` both pass a null by);
//! - the one read becomes `if g { text_from_byte_range(bytes, a + lo, a + hi) } else {
//!   text_from_bytes(tb) }` — the same bytes read in place (both decode with
//!   `String::from_utf8(..).unwrap_or_default()` over the same range), the original call on
//!   every range the guard refuses.
//!
//! `g` is evaluated twice, so it must answer the same at the read as at the copy: every
//! statement between the two may only mint the frame's own return buffer or write a scalar
//! field of it — the record literal a decoder's text lands in — and nothing between assigns a
//! variable the guard reads.  `tb` must be declared immediately before its copy (empty when
//! the copy starts), mentioned nowhere but there, the copy and the one read, and its wrapper
//! only in its declaration and its frees.  Anything else keeps the vector: a wrong decline is
//! the vector the program already builds, a wrong admission would be a different text.
//!
//! `LOFT_NO_BYTE_COPY=1` turns this off with the copy it builds on.
use crate::compact::{call, int, significant, var};
use crate::data::{Data, Value, v_if};

/// The parts of one guarded run copy `(R-ByteCopy)` wrote:
/// `if g { OpSliceVector(t, v, lo, hi, tp) } else { the loop }`.
struct Run {
    guard: Value,
    t: u16,
    v: u16,
    lo: Value,
    hi: Value,
    fallback: Value,
}

struct Cx<'a> {
    data: &'a Data,
    d_nr: u32,
    slice: u32,
    from_bytes: u32,
    from_range: u32,
    get_field: u32,
    set_int4: u32,
    database: u32,
    place_record: u32,
    free_ref: u32,
    free_record_in: u32,
    /// Ops a statement between the copy and the read may hold: reads, and writes of a scalar
    /// field of the return buffer (`write_ok`).
    read_ok: Vec<u32>,
    write_ok: Vec<u32>,
    /// The frame's hidden return buffer, if it has one.
    retbuf: Option<u16>,
}

/// Rewrite every admitted text run in `code` (the body of `d_nr`, already through
/// `(R-ByteCopy)`).
pub(crate) fn rewrite(data: &Data, d_nr: u32, code: &mut Value) {
    let slice = data.def_nr("OpSliceVector");
    if !code.any_node(&mut |n| matches!(n, Value::Call(d, _) if *d == slice)) {
        return;
    }
    let def = data.def(d_nr);
    let retbuf = def
        .hidden_return_buffer_attr()
        .map(|i| def.variables().var(&def.attributes()[i].name));
    let names = |ns: &[&str]| ns.iter().map(|n| data.def_nr(n)).collect::<Vec<_>>();
    let cx = Cx {
        data,
        d_nr,
        slice,
        from_bytes: data.def_nr("n_text_from_bytes"),
        from_range: data.def_nr("n_text_from_byte_range"),
        get_field: data.def_nr("OpGetField"),
        set_int4: data.def_nr("OpSetInt4"),
        database: data.def_nr("OpDatabase"),
        place_record: data.def_nr("OpPlaceRecord"),
        free_ref: data.def_nr("OpFreeRef"),
        free_record_in: data.def_nr("OpFreeRecordIn"),
        read_ok: names(&[
            "OpGetField",
            "OpRefIsNull",
            "OpConvBoolFromRef",
            "OpConvBoolFromInt",
            "OpAddInt",
            "OpMinInt",
            "OpLeInt",
            "OpLtInt",
            "OpEqInt",
        ]),
        write_ok: names(&[
            "OpDatabase",
            "OpSetInt",
            "OpSetInt4",
            "OpSetBoolean",
            "OpSetEnum",
            "OpSetByte",
            "OpSetFloat",
            "OpSetSingle",
        ]),
        retbuf,
    };
    let whole = code.clone();
    visit(code, &cx, &whole);
}

fn visit(v: &mut Value, cx: &Cx, whole: &Value) {
    if let Value::Block(bl) = v.unspan_mut() {
        rewrite_list(&mut bl.operators, cx, whole);
    } else if let Value::Insert(ls) = v.unspan_mut() {
        rewrite_list(ls, cx, whole);
    }
    v.for_each_child_mut(&mut |c| visit(c, cx, whole));
}

fn rewrite_list(ops: &mut [Value], cx: &Cx, whole: &Value) {
    for j in 0..ops.len() {
        let Some(run) = parse_run(&ops[j], cx) else {
            continue;
        };
        let Some(decl) = declaration(ops, j, run.t, cx) else {
            decline(cx, run.t, "not declared empty right before its copy");
            continue;
        };
        let Some(use_at) = read_after(ops, j, &run, decl.vdb, cx) else {
            decline(cx, run.t, "no single text read the guard still holds for");
            continue;
        };
        if !mentions_ok(whole, &run, decl.vdb, cx) {
            decline(cx, run.t, "the vector is mentioned elsewhere");
            continue;
        }
        crate::rewrite_census::fired("R-TextRun", 1);
        if crate::keys::trace_byte_copy() {
            let vars = cx.data.def(cx.d_nr).variables();
            eprintln!(
                "[text-run] fn={} ADMITTED: `{}` is read as text off `{}` in place",
                cx.data.def(cx.d_nr).name(),
                vars.name(run.t),
                vars.name(run.v)
            );
        }
        // The read first: its index lies after `j`, and the statements moved below sit at
        // or before it.
        replace_read(&mut ops[use_at], &run, cx);
        let mut moved = Vec::new();
        for &k in &decl.stmts {
            moved.push(take_decl(&mut ops[k], decl.vdb, cx));
        }
        moved.push(run.fallback.clone());
        ops[j] = v_if(run.guard.clone(), Value::Null, Value::Insert(moved));
    }
}

/// `if g { Insert[OpSliceVector(Var t, Var v, lo, hi, tp)] } else { fallback }`.
fn parse_run(v: &Value, cx: &Cx) -> Option<Run> {
    let Value::If(g, fast, slow) = v.unspan() else {
        return None;
    };
    let Value::Insert(fast) = fast.unspan() else {
        return None;
    };
    let [one] = fast.as_slice() else {
        return None;
    };
    let a = call(one, cx.slice)?;
    Some(Run {
        guard: g.unspan().clone(),
        t: var(&a[0])?,
        v: var(&a[1])?,
        lo: a[2].clone(),
        hi: a[3].clone(),
        fallback: slow.unspan().clone(),
    })
}

struct Decl {
    vdb: u16,
    /// The list indices of the declaration's statements, in order.
    stmts: Vec<usize>,
}

/// The three statements right before the copy that make `t` an empty vector of its own:
/// the wrapper (`OpDatabase(vdb, tp)`, or a placement `vdb = OpPlaceRecord(host, tp)` —
/// alone or as the last statement of the scan's insert), `t = OpGetField(vdb, 0, tp)`,
/// `OpSetInt4(vdb, 0, 0)`.
fn declaration(ops: &[Value], j: usize, t: u16, cx: &Cx) -> Option<Decl> {
    let before: Vec<usize> = (0..j)
        .rev()
        .filter(|&k| !matches!(ops[k].unspan(), Value::Line(_) | Value::Null))
        .take(3)
        .collect();
    let [len0, bind, wrap] = before.as_slice() else {
        return None;
    };
    let l = call(&ops[*len0], cx.set_int4)?;
    let vdb = var(&l[0])?;
    if int(&l[1]) != Some(0) || int(&l[2]) != Some(0) {
        return None;
    }
    let Value::Set(bt, gf) = ops[*bind].unspan() else {
        return None;
    };
    let gf = call(gf, cx.get_field)?;
    if *bt != t || var(&gf[0]) != Some(vdb) || int(&gf[1]) != Some(0) {
        return None;
    }
    if !wraps(&ops[*wrap], vdb, cx) {
        return None;
    }
    Some(Decl {
        vdb,
        stmts: vec![*wrap, *bind, *len0],
    })
}

fn wraps(v: &Value, vdb: u16, cx: &Cx) -> bool {
    match v.unspan() {
        Value::Call(d, a) if *d == cx.database => var(&a[0]) == Some(vdb),
        Value::Set(x, rhs) => *x == vdb && call(rhs, cx.place_record).is_some(),
        Value::Insert(ls) => significant(ls).last().is_some_and(|(_, last)| {
            wraps(last, vdb, cx) && !matches!(last.unspan(), Value::Insert(_))
        }),
        _ => false,
    }
}

/// Take the declaration statement at `v` out of the list, leaving what must stay: a
/// placement's insert keeps its host mint in place and gives up only the placement, and a
/// variable's binding leaves its null initialiser behind.  The native generator declares a
/// variable (`let`) at its first assignment, so a first assignment moved into the arm would
/// leave the read beside the arm naming a variable out of scope.
fn take_decl(v: &mut Value, vdb: u16, cx: &Cx) -> Value {
    if let Value::Insert(ls) = v.unspan_mut()
        && let Some(&(k, _)) = significant(ls).last()
    {
        debug_assert!(wraps(&ls[k], vdb, cx));
        return std::mem::replace(&mut ls[k], Value::Set(vdb, Box::new(Value::Null)));
    }
    let keep = match v.unspan() {
        Value::Set(x, _) => Value::Set(*x, Box::new(Value::Null)),
        _ => Value::Null,
    };
    std::mem::replace(v, keep)
}

/// The index of the statement after `j` that holds the one `text_from_bytes(t)`, every
/// statement before it inert and the path down to the read inert too.
fn read_after(ops: &[Value], j: usize, run: &Run, vdb: u16, cx: &Cx) -> Option<usize> {
    let protected = protected_vars(run, vdb);
    for (k, op) in ops.iter().enumerate().skip(j + 1) {
        if holds_read(op, run.t, cx) {
            return path_ok(op, run.t, &protected, cx).then_some(k);
        }
        if !inert(op, &protected, cx) {
            return None;
        }
    }
    None
}

/// The variables the guard reads, and the vector and its wrapper: none may be assigned
/// between the copy and the read.
fn protected_vars(run: &Run, vdb: u16) -> Vec<u16> {
    let mut out = vec![run.t, run.v, vdb];
    run.guard.walk(&mut |n| {
        if let Value::Var(x) = n {
            out.push(*x);
        }
    });
    out
}

fn is_read(v: &Value, t: u16, cx: &Cx) -> bool {
    call(v, cx.from_bytes).is_some_and(|a| a.len() == 1 && var(&a[0]) == Some(t))
}

fn holds_read(v: &Value, t: u16, cx: &Cx) -> bool {
    v.any_node(&mut |n| is_read(n, t, cx))
}

/// The path from a statement down to the read: everything evaluated before it is inert, and
/// the read sits under blocks, sets of an unprotected variable, call arguments and the arms
/// of an `if` — never under a loop, which could evaluate it after something else.
fn path_ok(v: &Value, t: u16, protected: &[u16], cx: &Cx) -> bool {
    if is_read(v, t, cx) {
        return true;
    }
    let seq = |items: &[Value]| {
        for it in items {
            if holds_read(it, t, cx) {
                return path_ok(it, t, protected, cx);
            }
            if !inert(it, protected, cx) {
                return false;
            }
        }
        false
    };
    match v.unspan() {
        Value::Block(bl) => seq(&bl.operators),
        Value::Insert(ls) => seq(ls),
        Value::Set(x, rhs) => !protected.contains(x) && path_ok(rhs, t, protected, cx),
        Value::Call(_, args) => seq(args),
        Value::If(c, a, b) => {
            inert(c, protected, cx)
                && if holds_read(a, t, cx) {
                    path_ok(a, t, protected, cx)
                } else {
                    path_ok(b, t, protected, cx)
                }
        }
        _ => false,
    }
}

/// A statement or expression that cannot change the guard's answer or the vector: reads,
/// and the return buffer's mint and scalar field writes.  Anything unnamed — a user call, a
/// loop, an assignment of a protected variable, a write through another reference — is not
/// inert, which declines the site: the conservative answer.
fn inert(v: &Value, protected: &[u16], cx: &Cx) -> bool {
    match v.unspan() {
        Value::Line(_)
        | Value::Null
        | Value::Int(_)
        | Value::Long(_)
        | Value::Boolean(_)
        | Value::Float(_)
        | Value::Single(_)
        | Value::Var(_) => true,
        Value::If(c, a, b) => {
            inert(c, protected, cx) && inert(a, protected, cx) && inert(b, protected, cx)
        }
        Value::Block(bl) => bl.operators.iter().all(|o| inert(o, protected, cx)),
        Value::Insert(ls) => ls.iter().all(|o| inert(o, protected, cx)),
        Value::Set(x, rhs) => !protected.contains(x) && inert(rhs, protected, cx),
        Value::Call(d, args) if cx.read_ok.contains(d) => {
            args.iter().all(|a| inert(a, protected, cx))
        }
        Value::Call(d, args) if cx.write_ok.contains(d) => {
            !args.is_empty()
                && in_retbuf(&args[0], cx)
                && args[1..].iter().all(|a| inert(a, protected, cx))
        }
        _ => false,
    }
}

/// The frame's hidden return buffer, or a field path over it: a record no other name in this
/// frame reaches, so a scalar written there cannot be a byte of the vector the guard reads.
fn in_retbuf(v: &Value, cx: &Cx) -> bool {
    match v.unspan() {
        Value::Var(x) => cx.retbuf == Some(*x),
        Value::Call(d, a) if *d == cx.get_field && a.len() == 3 => {
            int(&a[1]).is_some() && int(&a[2]).is_some() && in_retbuf(&a[0], cx)
        }
        _ => false,
    }
}

/// `t` only in its bind, the copy (both arms) and the one read; its wrapper only in its
/// declaration, its null initialiser and its frees.
fn mentions_ok(whole: &Value, run: &Run, vdb: u16, cx: &Cx) -> bool {
    let mut reads = 0;
    let mut stray = false;
    whole.walk(&mut |n| {
        if is_read(n, run.t, cx) {
            reads += 1;
        }
    });
    check_mentions(whole, run, vdb, cx, &mut stray, false);
    reads == 1 && !stray && read_is_assigned(whole, run.t, cx)
}

/// Flags a mention of `t` or `vdb` outside the places [`mentions_ok`] allows.  `in_run` is
/// set inside the guarded copy itself, whose arms name both freely.
fn check_mentions(v: &Value, run: &Run, vdb: u16, cx: &Cx, stray: &mut bool, in_run: bool) {
    if *stray {
        return;
    }
    let in_run = in_run || parse_run(v, cx).is_some_and(|r| r.t == run.t);
    match v.unspan() {
        Value::Var(x) if (*x == run.t || *x == vdb) && !in_run => *stray = true,
        n if is_read(n, run.t, cx) => {}
        Value::Set(x, rhs) if *x == run.t && !in_run => {
            let bind = call(rhs, cx.get_field).is_some_and(|a| var(&a[0]) == Some(vdb));
            if !bind && !matches!(rhs.unspan(), Value::Null) {
                *stray = true;
            }
        }
        Value::Set(x, rhs) if *x == vdb && !in_run => {
            if !matches!(rhs.unspan(), Value::Null) && call(rhs, cx.place_record).is_none() {
                *stray = true;
            } else {
                check_mentions(rhs, run, vdb, cx, stray, in_run);
            }
        }
        Value::Call(d, a)
            if !in_run
                && (*d == cx.free_ref
                    || *d == cx.free_record_in
                    || *d == cx.set_int4
                    || *d == cx.database)
                && var(&a[0]) == Some(vdb) => {}
        Value::Call(d, a) if !in_run && *d == cx.get_field && var(&a[0]) == Some(vdb) => {}
        _ => v.for_each_child(&mut |c| check_mentions(c, run, vdb, cx, stray, in_run)),
    }
}

/// `x = text_from_bytes(t)` → `if g { x = text_from_byte_range(v, lo, hi) } else { x =
/// text_from_bytes(t) }`.  The choice sits ABOVE the assignment, never inside it: a text
/// native assigned to its destination is lowered destination-passing
/// (`state::codegen::is_text_dest_native`), which reads the call as the assignment's direct
/// value — an `if` between the two hid it, and the interpreter answered "".
fn replace_read(v: &mut Value, run: &Run, cx: &Cx) {
    if let Value::Set(x, rhs) = v.unspan()
        && is_read(rhs, run.t, cx)
    {
        let x = *x;
        let original = std::mem::replace(v, Value::Null);
        let range = Value::Call(
            cx.from_range,
            vec![Value::Var(run.v), run.lo.clone(), run.hi.clone()],
        );
        *v = v_if(run.guard.clone(), Value::Set(x, Box::new(range)), original);
        return;
    }
    v.for_each_child_mut(&mut |c| replace_read(c, run, cx));
}

/// Is the one read the direct value of an assignment, the only form [`replace_read`] writes?
fn read_is_assigned(whole: &Value, t: u16, cx: &Cx) -> bool {
    let mut assigned = 0;
    whole.walk(&mut |n| {
        if let Value::Set(_, rhs) = n
            && is_read(rhs, t, cx)
        {
            assigned += 1;
        }
    });
    assigned == 1
}

fn decline(cx: &Cx, t: u16, why: &str) {
    if crate::keys::trace_byte_copy() {
        eprintln!(
            "[text-run] fn={} t={} keeps its vector: {why}",
            cx.data.def(cx.d_nr).name(),
            cx.data.def(cx.d_nr).variables().name(t)
        );
    }
}
