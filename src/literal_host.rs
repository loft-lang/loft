// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `@FR-R-MoveLast`'s host clause — a record literal whose vector field takes an owned local
//! at its last use is built in that local's store.
//!
//! The cursor shape: `toks = classify(argv); cur = Cur { src: toks, pos: 0 }; …` mints a
//! store for `cur` and deep-copies every element of `toks` into it, though `toks` is never
//! read again — `(R-MoveLast)` relocates only where source and destination share a store,
//! and a fresh literal never shares one.  With the literal's record CLAIMED IN the vector's
//! store (`OpPlaceRecord(toks, tp)` where `OpDatabase(cur, tp)` stood), the field takes the
//! vector by a HANDLE MOVE (`OpMoveVector`: the slot's record number moves and the source
//! slot is zeroed, so no element and no heap claim moves), and each exit's free of `cur`
//! becomes `OpFreeRefIfDistinct(cur, owner)` — nothing on a path where the placement held,
//! because the owner's free just after it releases the store the record lives in, and the
//! store-level free a null host's fallback store still needs.
//!
//! Admission, all asked on the function's OWN statement list (so the run executes once, at
//! most, and "after the move" is the statements after it): `cur = null; OpDatabase(cur, tp);`
//! then the literal's writes — calls whose first argument is rooted at `cur` — among which
//! `OpAppendVector(OpGetField(cur, f, _), src, _)` for a vector local `src` whose store is
//! OWNED by this frame (`src` itself, or the one local it depends on, freed by a plain
//! `OpFreeRef`).  After the move neither `src` nor its owner is named except by the owner's
//! exit frees.  `cur` is bound only by its null init and its mint, never aliased, returned or
//! handed to an op that would consume it as an owner (a move, a placement, a block release),
//! and named elsewhere only as the receiver of a native op or as a borrowed argument of a
//! loft-defined call.  Every free of `cur` stands in a statement list where a free of the
//! owner FOLLOWS it, and every free of the owner has one before it: the distinct-free reads
//! the owner's store number, which the owner's own free clears.  A stored reference naming a
//! record of any of the three declines, however long its holder lasts (`@FR-H-RefHome`), and
//! so does a view of the vector named after the move: either would name the literal's element
//! where `@FR-B-Copy` owes the vector's own.  Every decline keeps the copy.  `LOFT_NO_LITERAL_HOST=1` is the
//! switch; `LOFT_TRACE_PLACE=1` names each admission and decline.  Both backends: decided in
//! the scope pass on the settled IR.
use crate::data::{Data, Type, Value};
use crate::variables::Function;

struct Ops {
    database: u32,
    get_field: u32,
    append: u32,
    free_ref: u32,
    free_if_distinct: u32,
    place: u32,
    move_vec: u32,
    /// The ops that would take `cur` as an OWNER — consume, place in, or release it; an op
    /// this build lacks is `u32::MAX`, which no call names.  An array: the lookup runs once
    /// per function, and the front end's allocation count is pinned.
    consuming: [u32; 8],
}

impl Ops {
    fn lookup(data: &Data) -> Option<Self> {
        let nr = |n: &str| {
            let d = data.def_nr(n);
            (d != u32::MAX).then_some(d)
        };
        let consuming = [
            "OpFreeRefIfDistinct",
            "OpFreeRefIfDistinctBorrow",
            "OpMoveRecord",
            "OpMoveField",
            "OpMoveVector",
            "OpFreeRecordIn",
            "OpPlaceRecord",
            "OpEvacuate",
        ]
        .map(|n| nr(n).unwrap_or(u32::MAX));
        Some(Self {
            database: nr("OpDatabase")?,
            get_field: nr("OpGetField")?,
            append: nr("OpAppendVector")?,
            free_ref: nr("OpFreeRef")?,
            free_if_distinct: nr("OpFreeRefIfDistinct")?,
            place: nr("OpPlaceRecord")?,
            move_vec: nr("OpMoveVector")?,
            consuming,
        })
    }
}

/// One admitted literal: its local, the vector local its field takes, that vector's store
/// owner, and the positions of the mint and the move in the function's statement list.
struct Plan {
    cur: u16,
    init: usize,
    src: u16,
    owner: u16,
    tp: i32,
    mint: usize,
    mv: usize,
}

/// Rewrite every admitted literal of function `d_nr`; a no-op under the switch.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if !crate::keys::literal_host_enabled() || !data.def(d_nr).is_loft_defined() {
        return;
    }
    let Some(ops) = Ops::lookup(data) else {
        return;
    };
    let plans = admitted(data, d_nr, &ops);
    if plans.is_empty() {
        return;
    }
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    for p in &plans {
        apply(&mut code, p, &ops);
    }
    data.definitions[d_nr as usize].code = code;
    crate::rewrite_census::fired("R-MoveLast/host", plans.len());
}

fn is_var(n: &Value, w: u16) -> bool {
    matches!(n.unspan(), Value::Var(x) if *x == w)
}

fn names_var(n: &Value, w: u16) -> bool {
    match n.unspan() {
        Value::Var(x)
        | Value::Set(x, _)
        | Value::TupleGet(x, _)
        | Value::TuplePut(x, _, _)
        | Value::CallRef(x, _)
        | Value::FnRefDnr(x)
        | Value::Iter(x, _, _, _)
        | Value::FnRef(_, x, _) => *x == w,
        _ => false,
    }
}

fn mentions(node: &Value, w: u16) -> usize {
    let mut n = 0;
    node.walk(&mut |c| {
        if names_var(c, w) {
            n += 1;
        }
    });
    n
}

/// `OpGetField(… OpGetField(cur, …) …)` or `cur` itself: a place rooted at the literal.
fn rooted_at(node: &Value, cur: u16, ops: &Ops) -> bool {
    match node.unspan() {
        Value::Var(x) => *x == cur,
        Value::Call(d, a) if *d == ops.get_field => {
            a.first().is_some_and(|i| rooted_at(i, cur, ops))
        }
        _ => false,
    }
}

/// `OpFreeRef(w)` as a statement.
fn frees(n: &Value, w: u16, ops: &Ops) -> bool {
    matches!(n.unspan(), Value::Call(d, a) if *d == ops.free_ref && a.len() == 1 && is_var(&a[0], w))
}

fn top_list(code: &Value) -> Option<&[Value]> {
    match code.unspan() {
        Value::Block(bl) => Some(&bl.operators),
        _ => None,
    }
}

fn admitted(data: &Data, d_nr: u32, ops: &Ops) -> Vec<Plan> {
    let def = data.def(d_nr);
    let function = def.variables();
    let code = def.code();
    let Some(list) = top_list(code) else {
        return Vec::new();
    };
    let mut plans = Vec::new();
    for at in 0..list.len() {
        let Some(cand) = candidate(function, list, at, ops) else {
            continue;
        };
        let verdict = check(data, function, code, list, &cand, ops);
        if crate::keys::trace_place() {
            match &verdict {
                Ok(()) => eprintln!(
                    "[literal-host] fn={} v={}: ADMITTED — built in `{}`'s store, `{}` moved",
                    def.name(),
                    function.name(cand.cur),
                    function.name(cand.owner),
                    function.name(cand.src)
                ),
                Err(why) => eprintln!(
                    "[literal-host] fn={} v={}: DECLINED — {why}",
                    def.name(),
                    function.name(cand.cur)
                ),
            }
        }
        if verdict.is_ok()
            && !plans
                .iter()
                .any(|p: &Plan| p.cur == cand.cur || p.owner == cand.owner)
        {
            plans.push(cand);
        }
    }
    plans
}

/// `cur = null; OpDatabase(cur, tp); <writes rooted at cur>…` starting at `at`, one write of
/// which appends a plain vector local: the run's shape alone, before any whole-function ask.
fn candidate(function: &Function, list: &[Value], at: usize, ops: &Ops) -> Option<Plan> {
    let Value::Set(cur, init) = list[at].unspan() else {
        return None;
    };
    let cur = *cur;
    if !matches!(init.unspan(), Value::Null)
        || function.is_argument(cur)
        || function.is_captured(cur)
        // A nullable local is not a literal's plain record (`@FR-N-Shape`).
        || matches!(function.tp(cur), Type::Optional(_))
        || !matches!(function.tp(cur).base(), Type::Reference(_, _))
    {
        return None;
    }
    let stmt = |i: usize| list.get(i).map(Value::unspan);
    let mut i = at + 1;
    while matches!(stmt(i), Some(Value::Line(_))) {
        i += 1;
    }
    let Some(Value::Call(d, a)) = stmt(i) else {
        return None;
    };
    let [Value::Var(w), Value::Int(tp)] = a.as_slice() else {
        return None;
    };
    if *d != ops.database || *w != cur {
        return None;
    }
    let (mint, tp) = (i, *tp);
    i += 1;
    while let Some(s) = stmt(i) {
        let Value::Call(d, a) = s else {
            break;
        };
        if !a.first().is_some_and(|f| rooted_at(f, cur, ops)) {
            break;
        }
        if *d == ops.append
            && let Some(Value::Call(g, ga)) = a.first().map(Value::unspan)
            && *g == ops.get_field
            && ga.first().is_some_and(|f| is_var(f, cur))
            && let Some(Value::Var(src)) = a.get(1).map(Value::unspan)
            && let Some(owner) = store_owner(function, *src)
        {
            return Some(Plan {
                cur,
                init: at,
                src: *src,
                owner,
                tp,
                mint,
                mv: i,
            });
        }
        i += 1;
    }
    None
}

/// The local whose free releases `src`'s store: `src` itself when it depends on nothing, or
/// the one local it depends on when that one depends on nothing.  Parameters and captures
/// own nothing this frame may place into.
fn store_owner(function: &Function, src: u16) -> Option<u16> {
    let local =
        |v: u16| v < function.count() && !function.is_argument(v) && !function.is_captured(v);
    if !local(src)
        || matches!(function.tp(src), Type::Optional(_))
        || !matches!(function.tp(src).base(), Type::Vector(_, _))
    {
        return None;
    }
    // @FR-O-Proxy asks alloc — which store the literal is CLAIMED in; whether that store is
    // released is read off the frees the scope pass already wrote (`pairing`), never off this.
    match function.tp(src).depend().as_slice() {
        [] => Some(src),
        [o] if local(*o) && function.tp(*o).depend().is_empty() => Some(*o),
        _ => None,
    }
}

fn check(
    data: &Data,
    function: &Function,
    code: &Value,
    list: &[Value],
    p: &Plan,
    ops: &Ops,
) -> Result<(), &'static str> {
    // `@FR-H-RefHome` — a stored reference naming a record of any of the three stores, held
    // anywhere: after the move the record it names is the literal's element, so a write
    // through either side shows through the other (loft#2118).  Asked of every hold, not
    // only of a home moved out for a holder that outlives it.
    if [p.cur, p.src, p.owner]
        .iter()
        .any(|&v| function.is_ref_named(v))
    {
        return Err("a stored reference is homed in the literal's or the vector's store");
    }
    // A VIEW of the vector — a local whose deps reach `src` or its owner (`e = v[1]`, `r:
    // reference<T> = v[2]`, a view of such a view) — names the vector's records under another
    // name, so naming it after the move reads the literal's element where `@FR-B-Copy` owes
    // the vector's own (loft#2118).  A view made THROUGH the literal (`e = cur.src[1]`, a
    // slice match on it) names the literal's elements, which the copy and the move answer
    // alike, so it is no hazard (loft#2135).
    let views = views_of(function, &[p.src, p.owner]);
    if list[p.mv + 1..]
        .iter()
        .any(|s| views.iter().any(|&w| mentions(s, w) > 0))
    {
        return Err("a view of the vector is named after the move");
    }
    // After the move: the vector and its owner are named only by the owner's frees.
    for s in &list[p.mv + 1..] {
        let mut owner_frees = 0;
        s.walk(&mut |n| {
            if frees(n, p.owner, ops) {
                owner_frees += 1;
            }
        });
        let named = mentions(s, p.src)
            + if p.src == p.owner {
                0
            } else {
                mentions(s, p.owner)
            };
        if named != owner_frees {
            return Err("the vector or its store's owner is named after the move");
        }
    }
    if p.src != p.owner && count_frees(code, p.src, ops) > 0 {
        return Err("the vector is freed apart from its store's owner");
    }
    let mut cx = Cx {
        data,
        ops,
        cur: p.cur,
        mint: &list[p.mint],
        inits: 0,
        mints: 0,
    };
    cx.scan(code)?;
    if cx.inits != 1 || cx.mints != 1 {
        return Err("the literal is not bound once, by its null init and its mint");
    }
    pairing(code, list, p, ops)
}

/// Every local other than `roots` whose deps reach one of `roots`, directly or through
/// another such local.
fn views_of(function: &Function, roots: &[u16]) -> Vec<u16> {
    let mut reach: Vec<u16> = roots.to_vec();
    let mut views = Vec::new();
    loop {
        let before = views.len();
        for w in 0..function.count() {
            if !reach.contains(&w) && function.tp(w).depend().iter().any(|d| reach.contains(d)) {
                reach.push(w);
                views.push(w);
            }
        }
        if views.len() == before {
            return views;
        }
    }
}

fn count_frees(code: &Value, w: u16, ops: &Ops) -> usize {
    let mut n = 0;
    code.walk(&mut |c| {
        if frees(c, w, ops) {
            n += 1;
        }
    });
    n
}

/// From the literal's null init on, every free of `cur` is a statement followed, in its own
/// list, by a free of the owner, and every free of the owner is a statement with one of `cur`
/// before it.  An exit BEFORE the init frees the owner alone and is no concern: the literal
/// does not exist on that path, and its distinct-free there reads a null literal.
fn pairing(code: &Value, list: &[Value], p: &Plan, ops: &Ops) -> Result<(), &'static str> {
    let (mut seen_cur, mut seen_owner) = (0, 0);
    let mut bad = false;
    let mut pair = |stmts: &[Value]| {
        let mut pending = 0usize;
        for s in stmts {
            if frees(s, p.cur, ops) {
                pending += 1;
                seen_cur += 1;
            } else if frees(s, p.owner, ops) {
                seen_owner += 1;
                if pending == 0 {
                    bad = true;
                } else {
                    pending -= 1;
                }
            }
        }
        if pending > 0 {
            bad = true;
        }
    };
    let region = &list[p.init..];
    pair(region);
    for s in region {
        s.walk(&mut |n| match n {
            Value::Block(bl) | Value::Loop(bl) => pair(&bl.operators),
            Value::Insert(l) => pair(l),
            _ => {}
        });
    }
    if bad {
        return Err("a free of the literal is not followed by its store owner's free");
    }
    if seen_cur == 0 {
        return Err("the literal is freed nowhere");
    }
    let in_region = |w: u16| region.iter().map(|s| count_frees(s, w, ops)).sum::<usize>();
    if seen_cur != count_frees(code, p.cur, ops) || seen_owner != in_region(p.owner) {
        return Err("a free stands outside a statement list");
    }
    Ok(())
}

struct Cx<'a> {
    data: &'a Data,
    ops: &'a Ops,
    cur: u16,
    mint: &'a Value,
    inits: usize,
    mints: usize,
}

impl Cx<'_> {
    /// Every mention of `cur` is its null init, its mint, a free, the receiver of a native op
    /// or a borrowed argument of a loft-defined call.  The fallback recurses: a node that names
    /// `cur` without being one of those spellings declines, so an unlisted shape can only
    /// refuse the rewrite.
    fn scan(&mut self, node: &Value) -> Result<(), &'static str> {
        let node = node.unspan();
        match node {
            Value::Set(w, val) if *w == self.cur => {
                if matches!(val.unspan(), Value::Null) {
                    self.inits += 1;
                    Ok(())
                } else {
                    Err("the literal is rebound")
                }
            }
            Value::Set(_, val) | Value::Return(val) | Value::Drop(val) if is_var(val, self.cur) => {
                Err("the literal is aliased, returned or dropped bare")
            }
            Value::Call(d, args) => self.call(node, *d, args),
            Value::Var(x) if *x == self.cur => {
                Err("the literal is named outside a receiver position")
            }
            other => {
                if !matches!(other, Value::Set(..) | Value::Return(_) | Value::Drop(_))
                    && names_var(other, self.cur)
                {
                    return Err("the literal is named outside a receiver position");
                }
                let mut r = Ok(());
                other.for_each_child(&mut |c| {
                    if r.is_ok() {
                        r = self.scan(c);
                    }
                });
                r
            }
        }
    }

    fn call(&mut self, node: &Value, d: u32, args: &[Value]) -> Result<(), &'static str> {
        let ops = self.ops;
        let first_is = args.first().is_some_and(|a| is_var(a, self.cur));
        if d == ops.database && first_is {
            if std::ptr::eq(node, self.mint.unspan()) {
                self.mints += 1;
                return Ok(());
            }
            return Err("the literal is minted twice");
        }
        if d == ops.free_ref && first_is {
            return Ok(());
        }
        if ops.consuming.contains(&d) && args.iter().any(|a| rooted_at(a, self.cur, ops)) {
            return Err("the literal is handed to an op that takes it as an owner");
        }
        let loft_defined =
            (d as usize) < self.data.definitions.len() && self.data.def(d).is_loft_defined();
        for (i, a) in args.iter().enumerate() {
            if is_var(a, self.cur) {
                if i == 0 || loft_defined {
                    continue;
                }
                return Err("the literal is an argument the op may keep");
            }
            self.scan(a)?;
        }
        Ok(())
    }
}

fn apply(code: &mut Value, p: &Plan, ops: &Ops) {
    let Value::Block(bl) = code.unspan_mut() else {
        return;
    };
    let mint = bl.operators[p.mint].unspan_mut();
    *mint = Value::Set(
        p.cur,
        Box::new(Value::Call(
            ops.place,
            vec![Value::Var(p.src), Value::Int(p.tp)],
        )),
    );
    if let Value::Call(d, _) = bl.operators[p.mv].unspan_mut() {
        assert!(*d == ops.append, "the move site no longer names the copy");
        *d = ops.move_vec;
    }
    let (cur, owner, free_ref, distinct) = (p.cur, p.owner, ops.free_ref, ops.free_if_distinct);
    code.map_nodes(&mut |n| {
        if let Value::Call(d, a) = n
            && *d == free_ref
            && a.len() == 1
            && is_var(&a[0], cur)
        {
            *d = distinct;
            a.push(Value::Var(owner));
        }
    });
}
