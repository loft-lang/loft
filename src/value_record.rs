// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-ValueRecord` decided in the IR phase, for both backends (@PLN180 slice 1).
//!
//! A function whose result is a small record of scalars, and whose every caller only reads
//! fields off the local it binds, RETURNS A TUPLE: the callee builds `(a, b, c)` where it
//! built a record into its `__retbuf`, and the caller reads `p.0` where it read a field.
//! The record, its buffer store and the two frees that released it are gone from the IR,
//! so the interpreter claims no store for the call — the work `--native` already avoided
//! with its own generator-side rewrite (`generation::hoist::value_records`).
//!
//! ONE admission: the candidates are the functions `value_records` admits, so the rule
//! has one definition; this pass narrows them to the shape it can rewrite and leaves the
//! rest to the native rewrite, which then finds only what this pass declined.
//!
//! The shape, all of it required, anything else declining the function everywhere:
//!
//! - the callee is not `pub` (a library's API keeps its record, `(R-Escape)`), its record
//!   is FLAT (every field an 8-byte integer, a float, a single or a boolean), and every
//!   place its buffer appears is an object literal filling each field once, in a return
//!   or as the body's tail;
//! - every call is the value of a `Set` of a local, whose last argument is the caller's
//!   `__ref_N` buffer;
//! - the local is mentioned only by field reads, by binds from calls of admitted functions
//!   building the same record in the same field order, by its scope-exit `OpFreeRef` and
//!   by its buffer's `OpFreeRefIfDistinct`; it is no parameter, no capture and no other
//!   variable's dependency;
//! - the buffer is mentioned only by its null init, the call and that free — or, where the
//!   scope pass hoisted it out of a loop, by the guard that mints it once, the free of the
//!   local against it (either argument order) and its own free at function exit.
//!
//! The tuple's element order is the order the literal FILLS the fields in, which is the
//! source order: building it in schema order would evaluate the field expressions in
//! another order than the program wrote them.
//!
//! The pass changes SIGNATURES, so it runs where the program is closed — the top of
//! `compile::byte_code_from` — and not at all on a `Data` that is parsed against again
//! (`Data::open_world`).  `LOFT_NO_IR_VALUE_RECORD=1` is the switch.

use crate::data::{Data, DefType, I64, Type, Value};
use crate::database::{Parts, Stores};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};

/// `LOFT_NO_IR_VALUE_RECORD=1` leaves every record return to the native rewrite and the
/// interpreter's buffer — the bisect step for a wrong field read out of a small-record call
/// on EITHER backend.
fn disabled() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_IR_VALUE_RECORD").is_ok_and(|v| v != "0"))
}

/// The scalar a field is carried as.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Int,
    Float,
    Single,
    Bool,
}

impl Kind {
    fn tuple_type(self) -> Type {
        match self {
            Kind::Int => I64.clone(),
            Kind::Float => Type::Float,
            Kind::Single => Type::Single,
            Kind::Bool => Type::Boolean,
        }
    }
}

/// The operators the shape is spelled in, by definition number.
struct Ops {
    get: [(u32, Kind); 4],
    set: [(u32, Kind); 4],
    free_ref: u32,
    free_if_distinct: u32,
    database: u32,
    ref_is_null: u32,
    bool_from_ref: u32,
}

impl Ops {
    fn new(data: &Data) -> Ops {
        let k = [Kind::Int, Kind::Float, Kind::Single, Kind::Bool];
        let names = ["Int", "Float", "Single", "Boolean"];
        let get = std::array::from_fn(|i| (data.def_nr(&format!("OpGet{}", names[i])), k[i]));
        let set = std::array::from_fn(|i| (data.def_nr(&format!("OpSet{}", names[i])), k[i]));
        Ops {
            get,
            set,
            free_ref: data.def_nr("OpFreeRef"),
            free_if_distinct: data.def_nr("OpFreeRefIfDistinct"),
            database: data.def_nr("OpDatabase"),
            ref_is_null: data.def_nr("OpRefIsNull"),
            bool_from_ref: data.def_nr("OpConvBoolFromRef"),
        }
    }

    fn get_kind(&self, op: u32) -> Option<Kind> {
        self.get.iter().find(|(d, _)| *d == op).map(|(_, k)| *k)
    }

    fn set_kind(&self, op: u32) -> Option<Kind> {
        self.set.iter().find(|(d, _)| *d == op).map(|(_, k)| *k)
    }
}

/// An admitted callee: its record, its buffer parameter and the tuple it returns.
#[derive(Clone)]
struct Callee {
    record: u32,
    buf: u16,
    /// `(byte offset, kind)` in the order the literal fills them — the tuple's order.
    order: Vec<(i32, Kind)>,
}

impl Callee {
    fn index_of(&self, off: i32) -> Option<u16> {
        self.order
            .iter()
            .position(|(o, _)| *o == off)
            .map(|i| i as u16)
    }

    fn tuple_type(&self) -> Type {
        Type::Tuple(self.order.iter().map(|(_, k)| k.tuple_type()).collect())
    }
}

/// Rewrite every admitted function and its call sites; answers how many functions now
/// return a tuple.
pub fn rewrite_program(data: &mut Data, stores: &Stores) -> usize {
    if disabled() || data.open_world {
        return 0;
    }
    let vr = crate::generation::hoist::value_records(data, stores);
    if vr.fns.is_empty() {
        return 0;
    }
    let ops = Ops::new(data);
    let mut cands: HashMap<u32, Callee> = HashMap::default();
    for (&d, &tp) in &vr.fns {
        if let Some(c) = callee_shape(data, stores, &ops, d, tp) {
            cands.insert(d, c);
        }
    }
    // A FIXPOINT: declining a function turns its calls into ordinary values, which in turn
    // declines a local another admitted function also binds.  Every round only removes.
    let mut plans: Vec<(u32, CallerPlan)>;
    loop {
        plans = Vec::new();
        let mut declined: HashSet<u32> = HashSet::default();
        for caller in 0..data.definitions() {
            let def = data.def(caller);
            if def.def_type != DefType::Function || matches!(def.code(), Value::Null) {
                continue;
            }
            let (plan, bad) = plan_caller(data, &ops, &cands, caller);
            declined.extend(bad);
            if !plan.locals.is_empty() {
                plans.push((caller, plan));
            }
        }
        if declined.is_empty() {
            break;
        }
        cands.retain(|d, _| !declined.contains(d));
        if cands.is_empty() {
            return 0;
        }
    }
    let mut touched: HashSet<u32> = HashSet::default();
    for (caller, plan) in plans {
        rewrite_caller(data, &ops, &cands, caller, &plan);
        touched.insert(caller);
    }
    for (&d, c) in &cands {
        rewrite_callee(data, &ops, d, c);
        touched.insert(d);
    }
    crate::rewrite_census::fired("R-ValueRecord", cands.len());
    let mut touched: Vec<u32> = touched.into_iter().collect();
    touched.sort_unstable();
    for d in touched {
        let _census_body = crate::rewrite_census::InBody::enter("ir", data.def(d).name());
        data.definitions[d as usize].variables.reset_intervals();
        crate::scopes::compute_function_intervals(data, d);
        crate::scopes::assign_function_slots(data, d);
    }
    cands.len()
}

/// The callee half of the shape, or `None` when `d` cannot be rewritten here.
fn callee_shape(data: &Data, stores: &Stores, ops: &Ops, d: u32, tp: u16) -> Option<Callee> {
    let def = data.def(d);
    if def.pub_visible || def.def_type != DefType::Function {
        return None;
    }
    if def.attributes.last()?.name != "__retbuf" {
        return None;
    }
    let buf = def.variables.var("__retbuf");
    if buf == u16::MAX || !def.variables.is_argument(buf) {
        return None;
    }
    let Type::Reference(record, _) = def.returned().peel_link() else {
        return None;
    };
    let schema = flat_schema(data, stores, *record, tp)?;
    let mut order: Option<Vec<(i32, Kind)>> = None;
    if !literals_only(def.code(), buf, &schema, ops, true, &mut order) {
        return None;
    }
    Some(Callee {
        record: *record,
        buf,
        order: order?,
    })
}

/// The record's fields by offset, when every one is a scalar a tuple carries at the width
/// the field stores it.
fn flat_schema(data: &Data, stores: &Stores, record: u32, tp: u16) -> Option<HashMap<i32, Kind>> {
    let Parts::Struct(fields) = &stores.types.get(tp as usize)?.parts else {
        return None;
    };
    let mut out = HashMap::default();
    for f in fields {
        let a = data
            .def(record)
            .attributes
            .iter()
            .position(|a| a.name == f.name)?;
        let kind = match data.attr_type(record, a).base() {
            Type::Integer(_) if stores.size(f.content) == 8 => Kind::Int,
            Type::Float => Kind::Float,
            Type::Single => Kind::Single,
            Type::Boolean => Kind::Bool,
            _ => return None,
        };
        out.insert(i32::from(f.position), kind);
    }
    (!out.is_empty()).then_some(out)
}

/// Does every mention of `buf` in `n` sit inside an object literal in a result position?
/// Records the literals' fill order in `order`; they must all agree.  `tail` says `n` is
/// the value the function returns.
fn literals_only(
    n: &Value,
    buf: u16,
    schema: &HashMap<i32, Kind>,
    ops: &Ops,
    tail: bool,
    order: &mut Option<Vec<(i32, Kind)>>,
) -> bool {
    let n = n.unspan();
    if let Some(fill) = literal_fill(n, buf, schema, ops) {
        if !tail {
            return false;
        }
        if order.as_ref().is_some_and(|o| *o != fill.order) {
            return false;
        }
        *order = Some(fill.order);
        return fill
            .values
            .iter()
            .all(|v| !mentions(v, buf) && literals_only(v, buf, schema, ops, false, order));
    }
    match n {
        Value::Var(v) => *v != buf,
        Value::Return(v) => literals_only(v, buf, schema, ops, true, order),
        Value::Block(b) => {
            let last = b.operators.len().saturating_sub(1);
            b.operators
                .iter()
                .enumerate()
                .all(|(i, op)| literals_only(op, buf, schema, ops, tail && i == last, order))
        }
        _ => {
            let mut ok = true;
            each_child(n, &mut |c| {
                ok = ok && literals_only(c, buf, schema, ops, false, order);
            });
            ok && !names_var(n, buf)
        }
    }
}

/// An object literal built into `buf`: the values in fill order.
struct Fill<'a> {
    order: Vec<(i32, Kind)>,
    values: Vec<&'a Value>,
}

/// `{ if … OpDatabase(buf, tp); OpSetX(buf, off, value)…; buf }` filling every field of
/// `schema` exactly once.
fn literal_fill<'a>(
    n: &'a Value,
    buf: u16,
    schema: &HashMap<i32, Kind>,
    ops: &Ops,
) -> Option<Fill<'a>> {
    let Value::Block(b) = n else {
        return None;
    };
    let body: Vec<&Value> = b
        .operators
        .iter()
        .map(Value::unspan)
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect();
    let (head, rest) = body.split_first()?;
    let (last, sets) = rest.split_last()?;
    if !matches!(last, Value::Var(v) if *v == buf) || !is_header(head, buf, ops) {
        return None;
    }
    let mut order = Vec::new();
    let mut values = Vec::new();
    for s in sets {
        let Value::Call(op, args) = s else {
            return None;
        };
        let kind = ops.set_kind(*op)?;
        let [target, Value::Int(off), value] = args.as_slice() else {
            return None;
        };
        if !matches!(target.unspan(), Value::Var(v) if *v == buf)
            || schema.get(off) != Some(&kind)
            || order.iter().any(|(o, _)| o == off)
        {
            return None;
        }
        order.push((*off, kind));
        values.push(value);
    }
    (order.len() == schema.len()).then_some(Fill { order, values })
}

/// The literal's opening test: an `if` that mints a record into `buf` when it holds none,
/// reading nothing but `buf` and calling nothing but the three operators it is built from.
fn is_header(n: &Value, buf: u16, ops: &Ops) -> bool {
    if !matches!(n, Value::If(..)) {
        return false;
    }
    let mut mints = false;
    let mut clean = true;
    n.any_node(&mut |c| {
        match c {
            Value::Call(op, args) => {
                if *op == ops.database {
                    mints |=
                        matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if *v == buf);
                } else if *op != ops.ref_is_null && *op != ops.bool_from_ref {
                    clean = false;
                }
            }
            Value::Var(v) if *v != buf => clean = false,
            _ => {}
        }
        false
    });
    mints && clean
}

/// Does `n` mention variable `v` anywhere, in any spelling?
fn mentions(n: &Value, v: u16) -> bool {
    n.any_node(&mut |c| names_var(c, v))
}

/// Does node `n` ITSELF name variable `v` — as a `Var`, or as one of the variants that
/// carry a variable number outside a `Var` node?  Exhaustive, so a variant added later is a
/// compile error here instead of a mention this pass cannot see.
fn names_var(n: &Value, v: u16) -> bool {
    match n {
        Value::Var(x)
        | Value::Set(x, _)
        | Value::CallRef(x, _)
        | Value::Iter(x, ..)
        | Value::TupleGet(x, _)
        | Value::TuplePut(x, ..)
        | Value::FnRef(_, x, _)
        | Value::FnRefDnr(x) => *x == v,
        Value::Null
        | Value::Line(_)
        | Value::Span(_)
        | Value::Int(_)
        | Value::Enum(..)
        | Value::Boolean(_)
        | Value::Float(_)
        | Value::Long(_)
        | Value::Single(_)
        | Value::Text(_)
        | Value::Call(..)
        | Value::Block(_)
        | Value::Insert(_)
        | Value::Return(_)
        | Value::Break(_)
        | Value::Continue(_)
        | Value::If(..)
        | Value::Loop(_)
        | Value::Drop(_)
        | Value::Keys(_)
        | Value::Tuple(_)
        | Value::Yield(_)
        | Value::Parallel(_)
        | Value::RawExpr(_) => false,
    }
}

/// Call `f` on each direct child of `n`.
fn each_child<'a>(n: &'a Value, f: &mut impl FnMut(&'a Value)) {
    match n {
        Value::Span(s) => f(&s.1),
        Value::Call(_, a) | Value::CallRef(_, a) | Value::Insert(a) | Value::Tuple(a) => {
            a.iter().for_each(f);
        }
        Value::Parallel(a) => a.iter().for_each(f),
        Value::Block(b) | Value::Loop(b) => b.operators.iter().for_each(f),
        Value::Set(_, x)
        | Value::Return(x)
        | Value::Drop(x)
        | Value::Yield(x)
        | Value::TuplePut(_, _, x) => f(x),
        Value::If(a, b, c) | Value::Iter(_, a, b, c) => {
            f(a);
            f(b);
            f(c);
        }
        _ => {}
    }
}

/// What one caller binds: per admitted local, the callee it was bound from and its buffers.
#[derive(Default)]
struct CallerPlan {
    /// Local → the callee whose tuple it carries (all its binds agree).
    locals: HashMap<u16, u32>,
    buffers: HashSet<u16>,
}

/// Everything one walk of a caller learns.
#[derive(Default)]
struct Scan {
    /// `(local, callee, buffer)` per bind from an admitted call.
    binds: Vec<(u16, u32, Option<u16>)>,
    /// Field reads: `(local, offset, kind)`.
    reads: Vec<(u16, i32, Kind)>,
    /// Statement `OpFreeRef(local)`.
    frees: HashSet<u16>,
    /// Statement `OpFreeRefIfDistinct(buffer, local)`.
    fids: Vec<(u16, u16)>,
    /// Statement `buffer = null`.
    null_inits: Vec<u16>,
    /// Statement `if OpRefIsNull(buffer) { OpDatabase(buffer, tp) }` — a hoisted buffer.
    mints: Vec<u16>,
    /// Every other mention, by variable.
    other: HashSet<u16>,
    /// Admitted functions called somewhere that is not a bind.
    unbound_calls: HashSet<u32>,
}

fn plan_caller(
    data: &Data,
    ops: &Ops,
    cands: &HashMap<u32, Callee>,
    caller: u32,
) -> (CallerPlan, HashSet<u32>) {
    let def = data.def(caller);
    let mut s = Scan::default();
    scan(def.code(), false, ops, cands, &mut s);
    let mut bad: HashSet<u32> = s.unbound_calls.clone();
    let mut plan = CallerPlan::default();
    let vars = &def.variables;
    let mut by_local: HashMap<u16, Vec<(u32, Option<u16>)>> = HashMap::default();
    for &(v, d, r) in &s.binds {
        by_local.entry(v).or_default().push((d, r));
    }
    let mut buffer_uses: HashMap<u16, usize> = HashMap::default();
    for binds in by_local.values() {
        for (_, r) in binds {
            if let Some(r) = r {
                *buffer_uses.entry(*r).or_default() += 1;
            }
        }
    }
    for (&v, binds) in &by_local {
        let first = &cands[&binds[0].0];
        let ok = binds.iter().all(|(d, r)| {
            let c = &cands[d];
            c.record == first.record
                && c.order == first.order
                && r.is_some_and(|r| {
                    buffer_uses[&r] == 1
                        && !s.other.contains(&r)
                        && !vars.is_argument(r)
                        && s.fids
                            .iter()
                            .all(|&(a, b)| (a != r && b != r) || pairs(a, b, r, v))
                })
        }) && !s.other.contains(&v)
            && !s.null_inits.contains(&v)
            && !vars.is_argument(v)
            && !vars.is_captured(v)
            && matches!(vars.tp(v).peel_link(), Type::Reference(rd, _) if *rd == first.record)
            && s.reads
                .iter()
                .filter(|(l, _, _)| *l == v)
                .all(|(_, off, kind)| first.order.contains(&(*off, *kind)))
            && s.fids.iter().all(|&(a, b)| {
                binds
                    .iter()
                    .any(|(_, r)| r.is_some_and(|r| pairs(a, b, r, v)))
                    || (a != v && b != v)
            })
            && (0..vars.count()).all(|x| !vars.tp(x).depend().contains(&v));
        if ok {
            plan.locals.insert(v, binds[0].0);
            plan.buffers.extend(binds.iter().filter_map(|(_, r)| *r));
        } else {
            bad.extend(binds.iter().map(|(d, _)| *d));
        }
    }
    (plan, bad)
}

/// Does the free `OpFreeRefIfDistinct(a, b)` pair buffer `r` with local `v`, in either
/// argument order (the loop-hoisted form swaps them)?
fn pairs(a: u16, b: u16, r: u16, v: u16) -> bool {
    (a == r && b == v) || (a == v && b == r)
}

fn scan(n: &Value, stmt: bool, ops: &Ops, cands: &HashMap<u32, Callee>, s: &mut Scan) {
    let n = n.unspan();
    match n {
        Value::Set(v, inner) => {
            if let Value::Call(d, args) = inner.unspan()
                && cands.contains_key(d)
            {
                let r = match args.last().map(Value::unspan) {
                    Some(Value::Var(r)) => Some(*r),
                    _ => None,
                };
                s.binds.push((*v, *d, r));
                let keep = args.len().saturating_sub(1);
                for a in &args[..keep] {
                    scan(a, false, ops, cands, s);
                }
                return;
            }
            if stmt && matches!(inner.unspan(), Value::Null) {
                s.null_inits.push(*v);
                return;
            }
            s.other.insert(*v);
            scan(inner, false, ops, cands, s);
        }
        Value::Call(op, args) => {
            if let Some(kind) = ops.get_kind(*op)
                && let [target, Value::Int(off)] = args.as_slice()
                && let Value::Var(v) = target.unspan()
            {
                s.reads.push((*v, *off, kind));
                return;
            }
            if stmt
                && *op == ops.free_ref
                && let [target] = args.as_slice()
                && let Value::Var(v) = target.unspan()
            {
                s.frees.insert(*v);
                return;
            }
            if stmt
                && *op == ops.free_if_distinct
                && let [a, b] = args.as_slice()
                && let (Value::Var(r), Value::Var(v)) = (a.unspan(), b.unspan())
            {
                s.fids.push((*r, *v));
                return;
            }
            if cands.contains_key(op) {
                s.unbound_calls.insert(*op);
            }
            for a in args {
                scan(a, false, ops, cands, s);
            }
        }
        Value::Block(b) | Value::Loop(b) => {
            for op in &b.operators {
                scan(op, true, ops, cands, s);
            }
        }
        Value::Insert(list) => {
            for op in list {
                scan(op, true, ops, cands, s);
            }
        }
        Value::If(cond, ..)
            if stmt
                && let Value::Call(op, args) = cond.unspan()
                && *op == ops.ref_is_null
                && let [Value::Var(r)] = args.as_slice()
                && is_header(n, *r, ops) =>
        {
            s.mints.push(*r);
        }
        _ => {
            if let Value::Var(x)
            | Value::CallRef(x, _)
            | Value::Iter(x, ..)
            | Value::TupleGet(x, _)
            | Value::TuplePut(x, ..)
            | Value::FnRef(_, x, _)
            | Value::FnRefDnr(x) = n
            {
                s.other.insert(*x);
            }
            each_child(n, &mut |c| scan(c, false, ops, cands, s));
        }
    }
}

fn rewrite_caller(
    data: &mut Data,
    ops: &Ops,
    cands: &HashMap<u32, Callee>,
    caller: u32,
    plan: &CallerPlan,
) {
    let shapes: HashMap<u16, Callee> = plan
        .locals
        .iter()
        .map(|(v, d)| (*v, cands[d].clone()))
        .collect();
    let def = &mut data.definitions[caller as usize];
    rewrite_uses(&mut def.code, ops, &shapes, &plan.buffers);
    for (v, c) in &shapes {
        def.variables.set_type(*v, c.tuple_type());
    }
}

/// Is statement `n` one the tuple form no longer needs: a buffer's null init, a local's
/// scope-exit free, or the free of the buffer it might have been built in?
fn dropped(n: &Value, ops: &Ops, locals: &HashMap<u16, Callee>, buffers: &HashSet<u16>) -> bool {
    match n.unspan() {
        Value::Set(r, inner) => buffers.contains(r) && matches!(inner.unspan(), Value::Null),
        Value::Call(op, args) if *op == ops.free_ref || *op == ops.free_if_distinct => {
            args.iter().any(|a| {
                matches!(a.unspan(), Value::Var(v) if locals.contains_key(v) || buffers.contains(v))
            })
        }
        Value::If(cond, ..) => {
            matches!(cond.unspan(), Value::Call(op, args) if *op == ops.ref_is_null
                && matches!(args.as_slice(), [Value::Var(r)] if buffers.contains(r)))
                && is_header(n.unspan(), buffer_of(cond), ops)
        }
        _ => false,
    }
}

/// The variable an `OpRefIsNull(var)` test reads.
fn buffer_of(cond: &Value) -> u16 {
    match cond.unspan() {
        Value::Call(_, args) => match args.first().map(Value::unspan) {
            Some(Value::Var(r)) => *r,
            _ => u16::MAX,
        },
        _ => u16::MAX,
    }
}

fn rewrite_uses(n: &mut Value, ops: &Ops, locals: &HashMap<u16, Callee>, buffers: &HashSet<u16>) {
    let n = n.unspan_mut();
    match n {
        Value::Set(v, inner) if locals.contains_key(v) => {
            if let Value::Call(_, args) = inner.unspan_mut() {
                args.pop();
                for a in args {
                    rewrite_uses(a, ops, locals, buffers);
                }
            }
        }
        Value::Call(op, args) => {
            if ops.get_kind(*op).is_some()
                && let [target, Value::Int(off)] = args.as_slice()
                && let Value::Var(v) = target.unspan()
                && let Some(c) = locals.get(v)
                && let Some(i) = c.index_of(*off)
            {
                *n = Value::TupleGet(*v, i);
                return;
            }
            for a in args {
                rewrite_uses(a, ops, locals, buffers);
            }
        }
        Value::Block(b) | Value::Loop(b) => {
            b.operators.retain(|o| !dropped(o, ops, locals, buffers));
            for o in &mut b.operators {
                rewrite_uses(o, ops, locals, buffers);
            }
        }
        Value::Insert(list) => {
            list.retain(|o| !dropped(o, ops, locals, buffers));
            for o in list {
                rewrite_uses(o, ops, locals, buffers);
            }
        }
        _ => each_child_mut(n, &mut |c| rewrite_uses(c, ops, locals, buffers)),
    }
}

/// Call `f` on each direct child of `n`, mutably.
fn each_child_mut(n: &mut Value, f: &mut impl FnMut(&mut Value)) {
    match n {
        Value::Span(s) => f(&mut s.1),
        Value::Call(_, a)
        | Value::CallRef(_, a)
        | Value::Insert(a)
        | Value::Tuple(a)
        | Value::Parallel(a) => a.iter_mut().for_each(f),
        Value::Block(b) | Value::Loop(b) => b.operators.iter_mut().for_each(f),
        Value::Set(_, x)
        | Value::Return(x)
        | Value::Drop(x)
        | Value::Yield(x)
        | Value::TuplePut(_, _, x) => f(x),
        Value::If(a, b, c) | Value::Iter(_, a, b, c) => {
            f(a);
            f(b);
            f(c);
        }
        _ => {}
    }
}

fn rewrite_callee(data: &mut Data, ops: &Ops, d: u32, c: &Callee) {
    let schema: HashMap<i32, Kind> = c.order.iter().copied().collect();
    let tuple = c.tuple_type();
    let old = data.def(d).returned().clone();
    let def = &mut data.definitions[d as usize];
    literals_to_tuples(&mut def.code, c.buf, &schema, ops);
    if let Value::Block(b) = def.code.unspan_mut()
        && b.result == old
    {
        b.result = tuple.clone();
    }
    def.returned = tuple;
    def.attributes.pop();
    def.variables.drop_argument(c.buf);
}

fn literals_to_tuples(n: &mut Value, buf: u16, schema: &HashMap<i32, Kind>, ops: &Ops) {
    let n = n.unspan_mut();
    if literal_fill(n, buf, schema, ops).is_some() {
        let Value::Block(b) = std::mem::replace(n, Value::Null) else {
            unreachable!()
        };
        let mut values = Vec::new();
        for o in b.operators {
            if let Value::Call(op, mut args) = o.unspan().clone()
                && ops.set_kind(op).is_some()
                && let Some(mut v) = args.pop()
            {
                literals_to_tuples(&mut v, buf, schema, ops);
                values.push(v);
            }
        }
        *n = Value::Tuple(values);
        return;
    }
    each_child_mut(n, &mut |c| literals_to_tuples(c, buf, schema, ops));
}
