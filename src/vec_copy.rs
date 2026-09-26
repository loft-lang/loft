// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-VecCopy` — a vector copied into another one element at a time is ONE append.
//!
//! `nb: vector<character> = []; for c in d.buf { nb += [c]; }` — the only way to say "a
//! copy of this buffer, then more" without a second statement — paid a bounds-checked
//! element read, a length re-read and a fused push PER ELEMENT against the block copy
//! `nb += d.buf` makes.  Where the loop is exactly that — the forward walk of a vector PLACE
//! `V` (a local, or a field chain over one) whose body is the one push of the loop value
//! into `t`, the element a kind whose read and push are exact inverses on the stored bytes
//! (`integer`, `i32`, `float`, `single`, `boolean`, an enum, `character`, a raw byte at the
//! same bias) — it becomes `OpAppendVector(t, V, elem)`, the op `t += V` lowers to.
//!
//! The walk and the append answer the same vector for every `V`: the walk reads each element
//! in index order while its index is below the length, and nothing in the body can move that
//! length, because `t` is EXCLUSIVE — a local whose every binding is a fresh mint in this
//! frame, or the frame's hidden return or work buffer — so no push into it can reach `V`.  A
//! `&` parameter is not exclusive: `cp(v, v)` hands one vector in twice, and the walk then
//! never ends where the append doubles it once; that destination keeps its loop, as does
//! anything else — a second statement, a transformed value, an element that owns heap or
//! reads through a null sentinel, a source that is a call.  A wrong decline is the loop the
//! program already pays; a wrong admission would be a different vector.
//!
//! `LOFT_NO_VEC_COPY=1` keeps every loop; `LOFT_TRACE_VEC_COPY=1` names each site.
use crate::compact::{call, int, var};
use crate::data::{Block, Data, DefType, Type, Value};

/// Rewrite every admitted element-wise copy in `d_nr`; a no-op under the switch, and a cheap
/// one for a body that walks no vector.
pub fn rewrite(data: &mut Data, database: &mut crate::database::Stores, d_nr: u32) {
    if !crate::keys::vec_copy_enabled() || data.def_type(d_nr) != DefType::Function {
        return;
    }
    let get_nullable = data.def_nr("OpGetVectorNullable");
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    if code.any_node(&mut |n| matches!(n, Value::Call(d, _) if *d == get_nullable)) {
        let found = {
            let cx = Cx {
                data,
                d_nr,
                whole: &code,
                get_nullable,
                add_int: data.def_nr("OpAddInt"),
                le_int: data.def_nr("OpLeInt"),
                lt_int: data.def_nr("OpLtInt"),
                length_vector: data.def_nr("OpLengthVector"),
                pre_alloc: data.def_nr("OpPreAllocVector"),
                get_field: data.def_nr("OpGetField"),
                kinds: KINDS.map(|(g, p, biased)| (data.def_nr(g), data.def_nr(p), biased)),
            };
            let mut found = Vec::new();
            find(&code, &cx, &mut found);
            found
        };
        // Decided on the unchanged body, then applied in place: a body with no site is
        // neither cloned nor touched.
        for site in found {
            let Some(elem_tp) = element_type(data, database, d_nr, site.t, site.src) else {
                continue;
            };
            let append = Value::Call(
                data.def_nr("OpAppendVector"),
                vec![Value::Var(site.t), site.place.clone(), Value::Int(elem_tp)],
            );
            replace_loop(&mut code, site.src, site.index, &append);
        }
    }
    data.definitions[d_nr as usize].code = code;
}

/// The (read, push, carries-a-bias) op pairs whose push stores exactly the bytes the read
/// decoded.  `OpGetShortRaw`, `OpGetByteNullable` and the text and record reads are absent:
/// their elements go through a null sentinel or own heap.
const KINDS: [(&str, &str, bool); 8] = [
    ("OpGetInt", "OpPushInt", false),
    ("OpGetInt4", "OpPushInt4", false),
    ("OpGetFloat", "OpPushFloat", false),
    ("OpGetSingle", "OpPushSingle", false),
    ("OpGetBoolean", "OpPushBoolean", false),
    ("OpGetEnum", "OpPushEnum", false),
    ("OpGetCharacter", "OpPushCharacter", false),
    ("OpGetByte", "OpPushByte", true),
];

struct Cx<'a> {
    data: &'a Data,
    d_nr: u32,
    whole: &'a Value,
    get_nullable: u32,
    add_int: u32,
    le_int: u32,
    lt_int: u32,
    length_vector: u32,
    pre_alloc: u32,
    get_field: u32,
    kinds: [(u32, u32, bool); 8],
}

/// One admitted copy, keyed for the second pass by its walk's two private variables.
struct Site {
    t: u16,
    src: u16,
    index: u16,
    place: Value,
}

fn find(v: &Value, cx: &Cx, out: &mut Vec<Site>) {
    if let Some(site) = match_copy(v, cx) {
        out.push(site);
        return;
    }
    v.for_each_child(&mut |c| find(c, cx, out));
}

/// Replace the `For block` whose walk uses `src` and `index` by `append`.
fn replace_loop(v: &mut Value, src: u16, index: u16, append: &Value) -> bool {
    if is_walk_of(v, src, index) {
        *v = append.clone();
        return true;
    }
    let mut done = false;
    v.for_each_child_mut(&mut |c| {
        if !done {
            done = replace_loop(c, src, index, append);
        }
    });
    done
}

/// Exactly `N` statements that carry code — a `Line` marker or a `Null` between two is
/// position bookkeeping (`compact::significant`'s reading) — without allocating: the matcher
/// asks this of every `for` block of every function.
fn sig<const N: usize>(ls: &[Value]) -> Option<[&Value; N]> {
    let mut out: [&Value; N] = [&Value::Null; N];
    let mut n = 0;
    for v in ls {
        if matches!(v.unspan(), Value::Line(_) | Value::Null) {
            continue;
        }
        *out.get_mut(n)? = v;
        n += 1;
    }
    (n == N).then_some(out)
}

fn is_walk_of(v: &Value, src: u16, index: u16) -> bool {
    let Value::Block(bl) = v.unspan() else {
        return false;
    };
    bl.name == "For block"
        && sig::<3>(&bl.operators).is_some_and(|[pre, start, _]| {
            matches!(pre.unspan(), Value::Set(x, _) if *x == src)
                && matches!(start.unspan(), Value::Set(x, _) if *x == index)
        })
}

/// A vector PLACE the append may read once: a local, or a field chain over one.
fn is_place(v: &Value, cx: &Cx) -> bool {
    match v.unspan() {
        Value::Var(_) => true,
        Value::Call(d, args) if *d == cx.get_field && args.len() == 3 => {
            int(&args[1]).is_some() && int(&args[2]).is_some() && is_place(&args[0], cx)
        }
        _ => false,
    }
}

fn body(v: &Value) -> Option<&Block> {
    match v.unspan() {
        Value::Block(bl) => Some(bl),
        _ => None,
    }
}

/// `if <cond> { break } else null`; answers the condition.
fn break_if(v: &Value) -> Option<&Value> {
    let Value::If(c, brk, no) = v.unspan() else {
        return None;
    };
    let brk = match brk.unspan() {
        Value::Block(bl) if let Some([one]) = sig::<1>(&bl.operators) => one,
        other => other,
    };
    (matches!(brk.unspan(), Value::Break(0)) && matches!(no.unspan(), Value::Null)).then_some(c)
}

fn match_copy(v: &Value, cx: &Cx) -> Option<Site> {
    let Value::Block(bl) = v.unspan() else {
        return None;
    };
    if bl.name != "For block" {
        return None;
    }
    // `_vector = V; e#index = -1; loop { … }`
    let [pre, start, lp] = sig::<3>(&bl.operators)?;
    let Value::Set(src, place) = pre.unspan() else {
        return None;
    };
    let Value::Set(index, init) = start.unspan() else {
        return None;
    };
    if int(init) != Some(-1) || !is_place(place, cx) {
        return None;
    }
    let Value::Loop(lp) = lp.unspan() else {
        return None;
    };
    let [next, end, neg, work] = sig::<4>(&lp.operators)?;
    // `e = { e#index = e#index + 1; READ(OpGetVectorNullable(_vector, size, e#index), 0[, min]) }`
    let Value::Set(e, step) = next.unspan() else {
        return None;
    };
    let [bump, read] = sig::<2>(&body(step)?.operators)?;
    let Value::Set(bumped, add) = bump.unspan() else {
        return None;
    };
    let add = call(add, cx.add_int)?;
    if *bumped != *index || var(&add[0]) != Some(*index) || int(&add[1]) != Some(1) {
        return None;
    }
    let (read_op, read_args) = match read.unspan() {
        Value::Call(d, a) => (*d, a),
        _ => return None,
    };
    let &(_, push_op, biased) = cx.kinds.iter().find(|(g, _, _)| *g == read_op)?;
    let elm = call(&read_args[0], cx.get_nullable)?;
    if var(&elm[0]) != Some(*src) || var(&elm[2]) != Some(*index) || int(&read_args[1]) != Some(0) {
        return None;
    }
    // `if len(_vector) <= e#index { break }`, `if e#index < 0 { break }`
    let past_end = call(break_if(end)?, cx.le_int)?;
    let len = call(&past_end[0], cx.length_vector)?;
    if var(&len[0]) != Some(*src) || var(&past_end[1]) != Some(*index) {
        return None;
    }
    let low = call(break_if(neg)?, cx.lt_int)?;
    if var(&low[0]) != Some(*index) || int(&low[1]) != Some(0) {
        return None;
    }
    // `{ [OpPreAllocVector(t, …);] PUSH(t, [min,] e) }`
    let work = &body(work)?.operators;
    let (reserve, push) = match (sig::<1>(work), sig::<2>(work)) {
        (Some([one]), _) => (None, one),
        (_, Some([first, second])) => (Some(call(first, cx.pre_alloc)?), second),
        _ => return None,
    };
    let pargs = call(push, push_op)?;
    let t = var(&pargs[0])?;
    if biased {
        // The push's bias is the read's: `OpGetByte(elm, 0, min)` / `OpPushByte(t, min, e)`.
        if pargs.len() != 3 || int(&pargs[1]).is_none() || int(&pargs[1]) != int(&read_args[2]) {
            return None;
        }
    } else if pargs.len() != 2 {
        return None;
    }
    if var(pargs.last()?) != Some(*e) {
        return None;
    }
    if reserve.is_some_and(|r| var(&r[0]) != Some(t)) {
        return None;
    }
    if t == *src || place.any_node(&mut |n| matches!(n, Value::Var(x) if *x == t)) {
        decline(cx, t, "the source is the destination");
        return None;
    }
    if !exclusive(cx, t) {
        decline(cx, t, "the destination may share a vector with the source");
        return None;
    }
    Some(Site {
        t,
        src: *src,
        index: *index,
        place: place.unspan().clone(),
    })
}

/// Is `t` a vector no other name can reach: a local, or the frame's hidden return or work
/// buffer (whose caller hands it a store of its own), every binding of which is `OpGetField`
/// of a store minted in this frame — a work buffer's lazy mint is one?
fn exclusive(cx: &Cx, t: u16) -> bool {
    let def = cx.data.def(cx.d_nr);
    let vars = def.variables();
    let mut binds: Vec<&Value> = Vec::new();
    cx.whole.walk(&mut |n| {
        if let Value::Set(x, rhs) = n
            && *x == t
        {
            binds.push(rhs);
        }
    });
    if vars.is_argument(t) {
        let slot = vars.arguments().iter().position(|&a| a == t);
        let hidden = slot
            .and_then(|i| def.attributes().get(i))
            .is_some_and(|a| a.work_buffer)
            || def
                .hidden_return_buffer_attr()
                .is_some_and(|i| vars.var(&def.attributes()[i].name) == t);
        if !hidden {
            return false;
        }
    } else if binds.is_empty() {
        return false;
    }
    binds.iter().all(|rhs| {
        call(rhs, cx.get_field).is_some_and(|a| {
            var(&a[0]).is_some_and(|vdb| {
                !vars.is_argument(vdb) && {
                    // Its null initialiser is no binding; anything else could name a
                    // store someone else reaches.
                    let mut rebound = false;
                    cx.whole.walk(&mut |n| {
                        if let Value::Set(x, rhs) = n
                            && *x == vdb
                            && !matches!(rhs.unspan(), Value::Null)
                        {
                            rebound = true;
                        }
                    });
                    !rebound
                }
            }) && int(&a[1]) == Some(0)
        })
    })
}

/// The element type `t += V` names — the parser's own derivation (`append_elem_tp`) — when
/// the source's element is the destination's.
fn element_type(
    data: &Data,
    database: &mut crate::database::Stores,
    d_nr: u32,
    t: u16,
    src: u16,
) -> Option<i32> {
    let vars = data.def(d_nr).variables();
    let (Type::Vector(tc, _), Type::Vector(sc, _)) = (vars.tp(t).base(), vars.tp(src).base())
    else {
        return None;
    };
    if tc.base() != sc.base() {
        return None;
    }
    let tp = data.vector_element_type(tc, database)?;
    if crate::keys::trace_vec_copy() {
        eprintln!(
            "[vec-copy] fn={} ADMITTED: `{}` takes the elements of `{}` as one append",
            data.def(d_nr).name(),
            vars.name(t),
            vars.name(src)
        );
    }
    Some(i32::from(tp))
}

fn decline(cx: &Cx, t: u16, why: &str) {
    if crate::keys::trace_vec_copy() {
        eprintln!(
            "[vec-copy] fn={} t={} keeps its loop: {why}",
            cx.data.def(cx.d_nr).name(),
            cx.data.def(cx.d_nr).variables().name(t)
        );
    }
}
