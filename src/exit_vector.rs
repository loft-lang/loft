// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)
//! `(R-ExitVector)` — a LOCAL VECTOR returned inside the exit literal is built in the
//! return buffer's store.
//!
//! The decoder shape: `items: vector<T> = []; …pushes…; return Out { items: items, … }`.
//! The local's wrapper (`__vdb_N`, a `main_vector<T>` record) is minted as a store of its
//! own, and the exit literal deep-copies the whole vector into the return buffer's store —
//! one store cycle and one full copy of the vector per call, for a value that only ever
//! ends up in that buffer.  With the wrapper CLAIMED IN the return buffer's store
//! (`OpPlaceRecord(__retbuf, wtp)` after the buffer is ensured), the literal takes the
//! vector by a HANDLE MOVE (`OpMoveVector`: the slot's record number moves, the source slot
//! is zeroed — every element and every heap claim stays where it is), and the exit
//! releases the wrapper's block alone (`OpFreeRecordIn`), which finds nothing to walk on the
//! path that moved and the whole vector on a path that did not.
//!
//! Admission, per candidate: the wrapper is minted once, at a `__vdb_` name, immediately
//! followed by the local's init off it; the local is only ever a RECEIVER (the first
//! argument of a native op — a push, a read, a placement host) or a borrowed argument of a
//! loft-defined call; the wrapper is named only at its null init, its mint, its length
//! reset and its exit frees; every free sits in an exit block that returns the buffer; and
//! each `OpAppendVector` whose SOURCE is the local is the copy into a field rooted at the
//! buffer, inside such an exit block, with no mention of the local after it in that block.
//! Declines keep the store and the copy: a rebind or an alias of the local, a bare
//! `return xs`, a copy into any other destination, a free outside an exit, a wrapper freed
//! nowhere.  `LOFT_NO_EXIT_VECTOR=1` is the switch; `LOFT_TRACE_PLACE=1` names each
//! admission and decline.  Both backends: decided in the scope pass after `(R-Place)`, so a
//! loop placement whose host is this local lands in the buffer's store too.
use crate::data::{Block, Data, Type, Value};
use crate::variables::Function;

type Path = Vec<usize>;

struct Ops {
    database: u32,
    get_field: u32,
    set_int4: u32,
    append: u32,
    free_ref: u32,
    free_if_distinct: u32,
    place: u32,
    free_in: u32,
    move_vec: u32,
}

impl Ops {
    fn lookup(data: &Data) -> Option<Self> {
        let nr = |n: &str| {
            let d = data.def_nr(n);
            (d != u32::MAX).then_some(d)
        };
        Some(Self {
            database: nr("OpDatabase")?,
            get_field: nr("OpGetField")?,
            set_int4: nr("OpSetInt4")?,
            append: nr("OpAppendVector")?,
            free_ref: nr("OpFreeRef")?,
            free_if_distinct: nr("OpFreeRefIfDistinct")?,
            place: nr("OpPlaceRecord")?,
            free_in: nr("OpFreeRecordIn")?,
            move_vec: nr("OpMoveVector")?,
        })
    }
}

struct Plan {
    vdb: u16,
    wtp: u16,
    /// The `OpDatabase(vdb, wtp)` statement: becomes the ensure + placement.
    mint: Path,
    /// Each `OpAppendVector(buffer field, v, tp)`: becomes the handle move.
    moves: Vec<Path>,
    /// Each exit free of the wrapper: becomes the block release.
    frees: Vec<Path>,
}

/// Rewrite every admitted local vector of function `d_nr`; a no-op under the switch and for
/// a function with no admissible local.
pub fn rewrite(data: &mut Data, d_nr: u32) {
    if !crate::keys::exit_vector_enabled() {
        return;
    }
    let Some(ops) = Ops::lookup(data) else {
        return;
    };
    let Some((plans, rb, guard)) = admitted(data, d_nr, &ops) else {
        return;
    };
    let mut code = std::mem::replace(&mut data.definitions[d_nr as usize].code, Value::Null);
    for plan in &plans {
        apply(&mut code, plan, rb, &guard, &ops);
    }
    data.definitions[d_nr as usize].code = code;
}

fn own_return_buffer(data: &Data, d_nr: u32) -> Option<u16> {
    let def = data.def(d_nr);
    let idx = def.hidden_return_buffer_attr()?;
    let name = &def.attributes().get(idx)?.name;
    let v = def.variables().var(name);
    (v != u16::MAX && def.variables().is_argument(v)).then_some(v)
}

fn admitted(data: &Data, d_nr: u32, ops: &Ops) -> Option<(Vec<Plan>, u16, Value)> {
    let def = data.def(d_nr);
    if !def.is_loft_defined() {
        return None;
    }
    let rb = own_return_buffer(data, d_nr)?;
    let function = def.variables();
    let code = def.code();
    let mut cands = Vec::new();
    find_mints(data, function, code, ops, &mut Vec::new(), &mut cands);
    if cands.is_empty() {
        return None;
    }
    let guard = guard_template(code, rb, ops)?;
    let mut plans = Vec::new();
    for (vdb, v, wtp, mint) in cands {
        let mut cx = Cx {
            data,
            ops,
            rb,
            vdb,
            v,
            mint: &mint,
            path: Vec::new(),
            mints: 0,
            inits: 0,
            moves: Vec::new(),
            frees: Vec::new(),
            exit_depth: 0,
            after_move: false,
            foreign_depth: 0,
        };
        let verdict = cx.scan(code).and_then(|()| {
            if cx.mints != 1 || cx.inits != 1 {
                return Err("the wrapper is not minted once with the local's init beside it");
            }
            if cx.frees.is_empty() {
                return Err("the wrapper is freed nowhere");
            }
            Ok(())
        });
        match verdict {
            Ok(()) => {
                crate::rewrite_census::fired("R-ExitVector", 1);
                if crate::keys::trace_place() {
                    eprintln!(
                        "[exit-vector] fn={} v={}: ADMITTED wtp={wtp} moves={} frees={}",
                        def.name(),
                        function.name(v),
                        cx.moves.len(),
                        cx.frees.len()
                    );
                }
                plans.push(Plan {
                    vdb,
                    wtp,
                    mint: mint.clone(),
                    moves: cx.moves,
                    frees: cx.frees,
                });
            }
            Err(why) => {
                if crate::keys::trace_place() {
                    eprintln!(
                        "[exit-vector] fn={} v={}: DECLINED — {why}",
                        def.name(),
                        function.name(v)
                    );
                }
            }
        }
    }
    (!plans.is_empty()).then_some((plans, rb, guard))
}

/// Every `OpDatabase(__vdb_N, wtp)` statement immediately followed, in the same statement
/// list, by `v = OpGetField(__vdb_N, 0, _)` for a local `v` whose type depends on the
/// wrapper alone: `(vdb, v, wtp, path of the mint)`.
fn find_mints(
    data: &Data,
    function: &Function,
    node: &Value,
    ops: &Ops,
    path: &mut Path,
    out: &mut Vec<(u16, u16, u16, Path)>,
) {
    let stmts: &[Value] = match node {
        Value::Block(bl) | Value::Loop(bl) => &bl.operators,
        Value::Insert(list) => list,
        Value::Span(b) => {
            find_mints(data, function, &b.1, ops, path, out);
            return;
        }
        Value::If(c, t, e) => {
            for (i, child) in [c, t, e].into_iter().enumerate() {
                path.push(i);
                find_mints(data, function, child, ops, path, out);
                path.pop();
            }
            return;
        }
        Value::Set(_, x) | Value::Return(x) | Value::Drop(x) => {
            path.push(0);
            find_mints(data, function, x, ops, path, out);
            path.pop();
            return;
        }
        _ => return,
    };
    for (i, stmt) in stmts.iter().enumerate() {
        path.push(i);
        if let Value::Call(d, args) = stmt.unspan()
            && *d == ops.database
            && let [Value::Var(vdb), Value::Int(wtp)] = args.as_slice()
            && function.name(*vdb).starts_with("__vdb_")
            && let Ok(wtp) = u16::try_from(*wtp)
            && let Some(next) = stmts.get(i + 1)
            && let Value::Set(v, init) = next.unspan()
            && let Value::Call(g, ga) = init.unspan()
            && *g == ops.get_field
            && matches!(ga.first().map(Value::unspan), Some(Value::Var(w)) if w == vdb)
            && matches!(ga.get(1).map(Value::unspan), Some(Value::Int(0)))
            && function.tp(*v).depend() == [*vdb]
        {
            // The wrapper's one field must be the vector the local reads off it: a tuple
            // destructure `(a, b) = f()` mints its wrapper as `main_vector<vector<T>>` over
            // a `vector<T>` local (hex_shape's `wall_chain_walk`), and a release that walks
            // that wrapper by its type reads every integer as a vector handle.  A store of
            // its own never walks, so the mismatch was silent until now; here it declines.
            if wrapper_holds(data, function, *vdb, *v) {
                out.push((*vdb, *v, wtp, path.clone()));
            } else if crate::keys::trace_place() {
                eprintln!(
                    "[exit-vector] v={}: DECLINED — the wrapper is {}, not the local's vector",
                    function.name(*v),
                    wrapper_name(data, function, *vdb)
                );
            }
        }
        find_mints(data, function, stmt, ops, path, out);
        path.pop();
    }
}

/// Does the wrapper record's one field hold exactly the local's vector type (deps aside)?
fn wrapper_holds(data: &Data, function: &Function, vdb: u16, v: u16) -> bool {
    let (shape, _) = function.tp(vdb).peel_optional();
    let Type::Reference(td, _) = shape.peel_link() else {
        return false;
    };
    if (*td as usize) >= data.definitions.len() {
        return false;
    }
    let mut fields = data
        .def(*td)
        .attributes()
        .iter()
        .filter(|a| !a.name.starts_with("__"));
    let Some(field) = fields.next() else {
        return false;
    };
    if fields.next().is_some() {
        return false;
    }
    let (local_shape, _) = function.tp(v).peel_optional();
    field.typedef.without_deps() == local_shape.peel_link().without_deps()
}

/// The wrapper's type name, `main_vector<T>`, to hold against the local's `T`.
fn wrapper_name(data: &Data, function: &Function, vdb: u16) -> String {
    let (shape, _) = function.tp(vdb).peel_optional();
    match shape.peel_link() {
        Type::Reference(td, _) if (*td as usize) < data.definitions.len() => {
            data.def(*td).name().to_string()
        }
        _ => String::new(),
    }
}

/// The exit's own "ensure the buffer" statement — the first statement of the first exit
/// block, which names the buffer and mints it when absent — cloned to stand before the
/// wrapper's placement.
fn guard_template(code: &Value, rb: u16, ops: &Ops) -> Option<Value> {
    let mut found = None;
    code.walk(&mut |n| {
        if found.is_some() {
            return;
        }
        if let Value::Block(bl) = n
            && is_exit_block(bl, rb)
            && let Some(first) = bl.operators.first()
            && mentions(first, rb)
            && mints_buffer(first, rb, ops)
        {
            found = Some(first.clone());
        }
    });
    found
}

fn mints_buffer(node: &Value, rb: u16, ops: &Ops) -> bool {
    let mut hit = false;
    node.walk(&mut |n| {
        if let Value::Call(d, a) = n
            && *d == ops.database
            && matches!(a.first().map(Value::unspan), Some(Value::Var(w)) if *w == rb)
        {
            hit = true;
        }
    });
    hit
}

/// An `Object` block building into the buffer that ends by returning it (or, the tail
/// spelling, by yielding it).
fn is_exit_block(bl: &Block, rb: u16) -> bool {
    bl.name == "Object"
        && bl.result.depend() == [rb]
        && match bl.operators.last().map(Value::unspan) {
            Some(Value::Return(r)) => matches!(r.unspan(), Value::Var(w) if *w == rb),
            Some(Value::Var(w)) => *w == rb,
            _ => false,
        }
}

fn mentions(node: &Value, w: u16) -> bool {
    let mut found = false;
    node.walk(&mut |n| {
        if names_var(n, w) {
            found = true;
        }
    });
    found
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

fn is_var(n: &Value, w: u16) -> bool {
    matches!(n.unspan(), Value::Var(x) if *x == w)
}

/// `OpGetField(… OpGetField(rb, …) …)`: a field place rooted at the buffer.
fn rooted_at(node: &Value, rb: u16, ops: &Ops) -> bool {
    match node.unspan() {
        Value::Var(x) => *x == rb,
        Value::Call(d, a) if *d == ops.get_field => {
            a.first().is_some_and(|i| rooted_at(i, rb, ops))
        }
        _ => false,
    }
}

struct Cx<'a> {
    data: &'a Data,
    ops: &'a Ops,
    rb: u16,
    vdb: u16,
    v: u16,
    mint: &'a Path,
    path: Path,
    mints: usize,
    inits: usize,
    moves: Vec<Path>,
    frees: Vec<Path>,
    exit_depth: usize,
    after_move: bool,
    /// Inside a node the path enumeration does not cover: no site may be recorded there.
    foreign_depth: usize,
}

impl Cx<'_> {
    fn child(&mut self, i: usize, node: &Value) -> Result<(), &'static str> {
        self.path.push(i);
        let r = self.scan(node);
        self.path.pop();
        r
    }

    fn site(&self) -> Result<Path, &'static str> {
        if self.foreign_depth > 0 {
            return Err("a site inside a node the rewrite cannot address");
        }
        Ok(self.path.clone())
    }

    fn scan(&mut self, node: &Value) -> Result<(), &'static str> {
        if self.after_move && (names_var(node, self.v) || mentions(node, self.v)) {
            return Err("the local is named after the move in its exit");
        }
        match node {
            Value::Span(b) => self.child(0, &b.1),
            Value::Block(bl) | Value::Loop(bl) => {
                let exit = is_exit_block(bl, self.rb);
                let saved = self.after_move;
                if exit {
                    self.exit_depth += 1;
                }
                let mut r = Ok(());
                for (i, s) in bl.operators.iter().enumerate() {
                    r = self.child(i, s);
                    if r.is_err() {
                        break;
                    }
                }
                if exit {
                    self.exit_depth -= 1;
                }
                self.after_move = saved;
                r
            }
            Value::Insert(list) => {
                for (i, s) in list.iter().enumerate() {
                    self.child(i, s)?;
                }
                Ok(())
            }
            Value::If(c, t, e) => {
                self.child(0, c)?;
                self.child(1, t)?;
                self.child(2, e)
            }
            Value::Set(w, val) => {
                if *w == self.v {
                    // The null pre-init of a local minted inside a branch, and the init
                    // right after the mint: `v = OpGetField(vdb, 0, _)`.
                    if matches!(val.unspan(), Value::Null) {
                        return Ok(());
                    }
                    let mut expect = self.mint.clone();
                    if let Some(last) = expect.last_mut() {
                        *last += 1;
                    }
                    if self.path == expect {
                        self.inits += 1;
                        return Ok(());
                    }
                    return Err("the local is rebound");
                }
                if *w == self.vdb {
                    if matches!(val.unspan(), Value::Null) {
                        return Ok(());
                    }
                    return Err("the wrapper is bound outside its mint");
                }
                if is_var(val, self.v) || is_var(val, self.vdb) {
                    return Err("the local is aliased");
                }
                self.child(0, val)
            }
            Value::Return(x) | Value::Drop(x) => {
                if is_var(x, self.v) || is_var(x, self.vdb) {
                    return Err("the local is returned or dropped bare");
                }
                self.child(0, x)
            }
            Value::Call(d, args) => self.call(*d, args),
            Value::Var(x) => {
                if *x == self.v || *x == self.vdb {
                    return Err("the local is named outside a receiver position");
                }
                Ok(())
            }
            other => {
                if names_var(other, self.v) || names_var(other, self.vdb) {
                    return Err("the local is named outside a receiver position");
                }
                self.foreign_depth += 1;
                let mut r = Ok(());
                let mut i = 0;
                other.for_each_child(&mut |c| {
                    if r.is_ok() {
                        r = self.child(i, c);
                    }
                    i += 1;
                });
                self.foreign_depth -= 1;
                r
            }
        }
    }

    fn call(&mut self, d: u32, args: &[Value]) -> Result<(), &'static str> {
        let ops = self.ops;
        let first_is = |w: u16| args.first().is_some_and(|a| is_var(a, w));
        if d == ops.database && first_is(self.vdb) {
            if self.path == *self.mint {
                self.mints += 1;
                return Ok(());
            }
            return Err("the wrapper is minted twice");
        }
        if (d == ops.free_ref || d == ops.free_if_distinct) && first_is(self.vdb) {
            if self.exit_depth == 0 {
                return Err("the wrapper is freed outside an exit");
            }
            let p = self.site()?;
            self.frees.push(p);
            return Ok(());
        }
        if d == ops.set_int4 && first_is(self.vdb) {
            return if matches!(args.get(1).map(Value::unspan), Some(Value::Int(0)))
                && matches!(args.get(2).map(Value::unspan), Some(Value::Int(0)))
            {
                Ok(())
            } else {
                Err("the wrapper is written outside its length reset")
            };
        }
        if d == ops.append && args.get(1).is_some_and(|a| is_var(a, self.v)) {
            if self.exit_depth == 0 || !args.first().is_some_and(|a| rooted_at(a, self.rb, ops)) {
                return Err("the vector is copied somewhere other than the exit literal");
            }
            let p = self.site()?;
            self.moves.push(p);
            self.after_move = true;
            return Ok(());
        }
        let loft_defined =
            (d as usize) < self.data.definitions.len() && self.data.def(d).is_loft_defined();
        for (i, a) in args.iter().enumerate() {
            if is_var(a, self.vdb) {
                return Err("the wrapper is named outside its admitted sites");
            }
            if is_var(a, self.v) {
                if i == 0 || loft_defined {
                    continue;
                }
                return Err("the local is an argument the op may keep");
            }
            self.child(i, a)?;
        }
        Ok(())
    }
}

fn node_at_mut<'a>(node: &'a mut Value, path: &[usize]) -> Option<&'a mut Value> {
    let Some((&i, rest)) = path.split_first() else {
        return Some(node);
    };
    match node {
        Value::Span(b) if i == 0 => node_at_mut(&mut b.1, rest),
        Value::Block(bl) | Value::Loop(bl) => node_at_mut(bl.operators.get_mut(i)?, rest),
        Value::Insert(list) => node_at_mut(list.get_mut(i)?, rest),
        Value::If(c, t, e) => node_at_mut([c, t, e].into_iter().nth(i)?, rest),
        Value::Set(_, x) | Value::Return(x) | Value::Drop(x) if i == 0 => node_at_mut(x, rest),
        Value::Call(_, args) => node_at_mut(args.get_mut(i)?, rest),
        _ => None,
    }
}

fn apply(code: &mut Value, plan: &Plan, rb: u16, guard: &Value, ops: &Ops) {
    let m = node_at_mut(code, &plan.mint).expect("the mint path names a node");
    *m = Value::Insert(vec![
        guard.clone(),
        Value::Set(
            plan.vdb,
            Box::new(Value::Call(
                ops.place,
                vec![Value::Var(rb), Value::Int(i32::from(plan.wtp))],
            )),
        ),
    ]);
    for p in &plan.moves {
        let n = node_at_mut(code, p).expect("a move path names a node");
        let Value::Call(d, _) = n.unspan_mut() else {
            panic!("a move path names no call");
        };
        assert!(*d == ops.append, "a move path no longer names the copy");
        *d = ops.move_vec;
    }
    for p in &plan.frees {
        let n = node_at_mut(code, p).expect("a free path names a node");
        let Value::Call(d, args) = n.unspan_mut() else {
            panic!("a free path names no call");
        };
        assert!(
            (*d == ops.free_ref || *d == ops.free_if_distinct)
                && matches!(args.first().map(Value::unspan), Some(Value::Var(w)) if *w == plan.vdb),
            "a free path no longer names the wrapper's free"
        );
        *d = ops.free_in;
        args.truncate(1);
        args.push(Value::Int(i32::from(plan.wtp)));
    }
}
