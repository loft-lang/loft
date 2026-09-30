// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-R-ValueRecord` and `@FR-R-ValueLocal` decided in the IR phase, for both backends
//! (@PLN180 slices 1 and 3).
//!
//! A small record of scalars that only ever travels between loft functions is carried as a
//! TUPLE: a function returning one builds `(a, b, c)` where it filled a `__retbuf`, a local
//! bound from such a call is a tuple read by `TupleGet`, and a by-value parameter of such a
//! record is received as a tuple.  The record, the caller's buffer store and the frees that
//! released it leave the IR, so the interpreter claims no store for the call — the work
//! `--native` already avoided with its own generator-side rewrite
//! (`generation::hoist::value_records`).
//!
//! ONE admission: the candidates are exactly what `value_records` admits — its `fns` for
//! results and its `params` for parameters, whose soundness gate (nothing the callee reaches
//! writes a record of the parameter's type) makes reading the fields at the call equal to
//! reading them later.  This pass narrows them to the shapes it can rewrite and leaves the
//! rest to the native rewrite, which then finds only what this pass declined.
//!
//! A CARRIER is a local bound from a call of an admitted function, or an admitted tuple
//! parameter.  The shapes, all required, anything else declining the function or parameter
//! everywhere, to a fixpoint:
//!
//! - an admitted function or parameter belongs to a function that is not `pub` (a
//!   library's API keeps its record, `(R-Escape)`) and that loft calls somewhere (one
//!   nothing in loft calls is called from outside loft by name, so its signature is its
//!   API);
//! - its record is FLAT: at least two fields (a loft tuple has two or more elements), each
//!   an 8-byte integer, a float, a single or a boolean;
//! - an admitted function's buffer appears only in object literals filling each field
//!   once, in a return or as the body's tail;
//! - every call of an admitted function is the value of a `Set` of a local, whose last
//!   argument is the caller's `__ref_N` buffer;
//! - a carrier is mentioned only by field reads, by being handed whole to an admitted
//!   parameter of the same record, and — a local — by its binds, its scope-exit
//!   `OpFreeRef` and its buffer's `OpFreeRefIfDistinct`; it is no capture and no other
//!   variable's dependency, and a parameter is never assigned;
//! - an admitted parameter's argument is a carrier of the same record, or a plain local of
//!   that record, whose fields are read into a tuple at the call;
//! - the buffer is mentioned only by its null init, the call and that free — or, where the
//!   scope pass hoisted it out of a loop, by the guard that mints it once, the free of the
//!   local against it (either argument order) and its own free at function exit.
//!
//! The tuple lists a record's fields in SCHEMA order, one order per record type, so a tuple
//! built by any source fits any parameter.  A literal that writes its fields in another
//! order binds each value to a temporary first, in the order it wrote them, so a field
//! expression with an effect runs where the program put it.
//!
//! The pass changes SIGNATURES, so it runs where the program is closed — the top of
//! `compile::byte_code_from` — and not at all on a `Data` that is parsed against again
//! (`Data::open_world`).  `LOFT_NO_IR_VALUE_RECORD=1` is the switch.

use crate::data::{Block, Data, DefType, I64, Type, Value};
use crate::database::{Parts, Stores};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};

/// `LOFT_NO_IR_VALUE_RECORD=1` leaves every small record to the native rewrite and the
/// interpreter's buffer — the bisect step for a wrong field read out of a small-record call
/// or parameter on EITHER backend.
fn disabled() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_IR_VALUE_RECORD").is_ok_and(|v| v != "0"))
}

/// `LOFT_TRACE_IR_VALUEREC=1` names each carrier this pass declines and why — the twin of
/// the native rewrite's `LOFT_TRACE_VALUEREC`.
fn trace() -> bool {
    crate::env_once!(std::env::var_os("LOFT_TRACE_IR_VALUEREC").is_some())
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
    copy_record: u32,
    get_field: u32,
    ref_alias: u32,
    distinct_store: u32,
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
            copy_record: data.def_nr("OpCopyRecord"),
            get_field: data.def_nr("OpGetField"),
            ref_alias: data.def_nr("OpRefAlias"),
            distinct_store: data.def_nr("OpDistinctStore"),
        }
    }

    fn get_kind(&self, op: u32) -> Option<Kind> {
        self.get.iter().find(|(d, _)| *d == op).map(|(_, k)| *k)
    }

    fn get_op(&self, kind: Kind) -> u32 {
        self.get
            .iter()
            .find(|(_, k)| *k == kind)
            .map_or(u32::MAX, |(d, _)| *d)
    }

    fn set_op(&self, kind: Kind) -> u32 {
        self.set
            .iter()
            .find(|(_, k)| *k == kind)
            .map_or(u32::MAX, |(d, _)| *d)
    }

    fn set_kind(&self, op: u32) -> Option<Kind> {
        self.set.iter().find(|(d, _)| *d == op).map(|(_, k)| *k)
    }
}

/// A flat record's tuple: its fields in SCHEMA order, `(byte offset, kind)`.
#[derive(Clone, PartialEq, Eq, Debug)]
struct Layout {
    /// The record's runtime type, what an `OpCopyRecord` of it names.
    tp: u16,
    fields: Vec<(i32, Kind)>,
}

impl Layout {
    fn index_of(&self, off: i32) -> Option<u16> {
        self.fields
            .iter()
            .position(|(o, _)| *o == off)
            .map(|i| i as u16)
    }

    fn kind_at(&self, off: i32) -> Option<Kind> {
        self.fields.iter().find(|(o, _)| *o == off).map(|(_, k)| *k)
    }

    fn tuple_type(&self) -> Type {
        Type::Tuple(self.fields.iter().map(|(_, k)| k.tuple_type()).collect())
    }
}

/// An admitted callee: its record and its buffer parameter.
#[derive(Clone)]
struct Callee {
    record: u32,
    buf: u16,
    /// The locals a FORWARD returns: bound from a call built in this function's own buffer,
    /// they must carry the callee's tuple for this function to return one.
    forwards: HashSet<u16>,
}

/// What the pass works from: the admitted functions and parameters and the layouts of the
/// records they carry.
struct World {
    ops: Ops,
    layouts: HashMap<u32, Layout>,
    cands: HashMap<u32, Callee>,
    /// `(function, attribute index)` → the record the parameter carries.
    params: HashMap<(u32, usize), u32>,
}

/// Rewrite every admitted function, parameter and call site; answers how many functions
/// and parameters now carry a tuple.
pub fn rewrite_program(data: &mut Data, stores: &Stores) -> usize {
    if disabled() || data.open_world {
        return 0;
    }
    let vr = crate::generation::hoist::value_records(data, stores);
    if vr.fns.is_empty() && vr.params.is_empty() {
        return 0;
    }
    let mut w = World {
        ops: Ops::new(data),
        layouts: HashMap::default(),
        cands: HashMap::default(),
        params: HashMap::default(),
    };
    for (&d, &tp) in &vr.fns {
        if let Some(c) = callee_shape(data, stores, &w.ops, &mut w.layouts, d, tp) {
            w.cands.insert(d, c);
        }
    }
    for (&d, ps) in &vr.params {
        for (&idx, &tp) in ps {
            if let Some(record) = param_shape(data, stores, &mut w.layouts, d, idx, tp) {
                w.params.insert((d, idx), record);
            }
        }
    }
    let functions: Vec<u32> = (0..data.definitions())
        .filter(|&f| {
            let def = data.def(f);
            def.def_type == DefType::Function && !matches!(def.code(), Value::Null)
        })
        .collect();
    // ADMISSION, a fixpoint: declining a function or a parameter turns what it carried into
    // an ordinary value, which declines the carriers that handed it on.  Every round only
    // removes.  A call site that cannot take the tuple declines nothing — it keeps calling
    // the record form (below).
    loop {
        let mut bad_fns: HashSet<u32> = HashSet::default();
        let mut bad_params: HashSet<(u32, usize)> = HashSet::default();
        let mut called: HashSet<u32> = HashSet::default();
        for &f in &functions {
            let (_, v) = plan_function(data, &w, f, w.cands.contains_key(&f));
            bad_fns.extend(v.bad_fns);
            bad_params.extend(v.bad_params);
            called.extend(v.called);
        }
        // Nothing in loft calls it: it is called from outside, by name.
        bad_fns.extend(w.cands.keys().filter(|d| !called.contains(d)));
        bad_params.extend(w.params.keys().filter(|(d, _)| !called.contains(d)));
        if bad_fns.is_empty() && bad_params.is_empty() {
            break;
        }
        w.cands.retain(|d, _| !bad_fns.contains(d));
        w.params.retain(|k, _| !bad_params.contains(k));
        if w.cands.is_empty() && w.params.is_empty() {
            return 0;
        }
    }
    // TWINS, a fixpoint that only grows.  A function some site still calls for a RECORD — a
    // local that cannot carry the tuple, an element or a field it is built into, a forward
    // whose function keeps its record — keeps its record form for exactly those sites, and a
    // TWIN returning the tuple serves the carriers.  A site that owes a record therefore
    // runs the code it ran before, on both backends.  A twinned forward keeps its record,
    // so the function it forwards is owed there in turn.
    let mut twinned: HashSet<u32> = HashSet::default();
    let mut plans: Vec<(u32, Plan)>;
    let mut twin_plans: Vec<(u32, Plan)>;
    loop {
        plans = Vec::new();
        twin_plans = Vec::new();
        let mut owed: HashSet<u32> = HashSet::default();
        for &f in &functions {
            let tuple = w.cands.contains_key(&f) && !twinned.contains(&f);
            let (plan, v) = plan_function(data, &w, f, tuple);
            owed.extend(v.owed);
            plans.push((f, plan));
            if twinned.contains(&f) {
                let (plan, v) = plan_function(data, &w, f, true);
                owed.extend(v.owed);
                twin_plans.push((f, plan));
            }
        }
        let before = twinned.len();
        twinned.extend(owed);
        if twinned.len() == before {
            break;
        }
    }
    // The twins are cloned from the untouched bodies, before anything is rewritten.
    let mut twins: HashMap<u32, u32> = HashMap::default();
    let mut sorted: Vec<u32> = twinned.iter().copied().collect();
    sorted.sort_unstable();
    for d in sorted {
        let t = add_twin(data, d);
        twins.insert(d, t);
        let own: Vec<(usize, u32)> = w
            .params
            .iter()
            .filter(|((f, _), _)| *f == d)
            .map(|((_, i), r)| (*i, *r))
            .collect();
        for (i, r) in own {
            w.params.insert((t, i), r);
        }
    }
    let mut touched: HashSet<u32> = HashSet::default();
    for (f, plan) in plans {
        if plan.carriers.is_empty() && !plan.reads_at_call {
            continue;
        }
        rewrite_function(data, &w, &twins, f, &plan);
        touched.insert(f);
    }
    for (d, plan) in twin_plans {
        rewrite_function(data, &w, &twins, twins[&d], &plan);
        touched.insert(twins[&d]);
    }
    for (&d, c) in &w.cands {
        let target = twins.get(&d).copied().unwrap_or(d);
        rewrite_callee(data, &w, target, c);
        touched.insert(target);
    }
    crate::rewrite_census::fired("R-ValueRecord", w.cands.len());
    crate::rewrite_census::fired("R-ValueLocal", w.params.len());
    let mut touched: Vec<u32> = touched.into_iter().collect();
    touched.sort_unstable();
    for d in touched {
        let _census_body = crate::rewrite_census::InBody::enter("ir", data.def(d).name());
        data.definitions[d as usize].variables.reset_intervals();
        crate::scopes::compute_function_intervals(data, d);
        crate::scopes::assign_function_slots(data, d);
    }
    w.cands.len() + w.params.len()
}

/// A copy of function `d` under a fresh name, for the tuple form while `d` keeps its record
/// form for the sites that owe one.
fn add_twin(data: &mut Data, d: u32) -> u32 {
    let base = data.def(d).name().to_string();
    let mut name = format!("{base}_tuple");
    let mut n = 1;
    while data.def_nr(&name) != u32::MAX {
        n += 1;
        name = format!("{base}_tuple{n}");
    }
    let position = data.def(d).position.clone();
    let t = data.add_def(&name, &position, DefType::Function);
    let mut def = data.def(d).clone();
    def.name = name;
    def.parent = u32::MAX;
    def.first_child = u32::MAX;
    def.next_sibling = u32::MAX;
    def.pub_visible = false;
    def.synthetic = Some("the tuple twin of a small-record return (@PLN180)");
    data.definitions[t as usize] = def;
    t
}

/// The callee half of the shape, or `None` when `d` cannot return a tuple here.
fn callee_shape(
    data: &Data,
    stores: &Stores,
    ops: &Ops,
    layouts: &mut HashMap<u32, Layout>,
    d: u32,
    tp: u16,
) -> Option<Callee> {
    let def = data.def(d);
    if def.pub_visible || def.def_type != DefType::Function {
        return None;
    }
    // The return buffer is the LAST attribute, so dropping it moves no other parameter.
    let idx = def.hidden_return_buffer_attr()?;
    if idx + 1 != def.attributes.len() {
        return None;
    }
    let buf = def.variables.var(&def.attributes[idx].name);
    if buf == u16::MAX || !def.variables.is_argument(buf) {
        return None;
    }
    let Type::Reference(record, _) = def.returned().peel_link() else {
        return None;
    };
    let layout = flat_layout(data, stores, *record, tp)?;
    let mut sh = BodyShape::default();
    if !body_shape(def.code(), buf, &layout, ops, true, &mut sh)
        || !sh.returned.is_subset(&sh.bound)
    {
        return None;
    }
    layouts.insert(*record, layout);
    Some(Callee {
        record: *record,
        buf,
        forwards: sh.returned,
    })
}

/// The parameter half: parameter `idx` of `d`, admitted by `value_records` as a tuple of
/// record type `tp`, when its record is flat and its function may change its signature.
fn param_shape(
    data: &Data,
    stores: &Stores,
    layouts: &mut HashMap<u32, Layout>,
    d: u32,
    idx: usize,
    tp: u16,
) -> Option<u32> {
    let def = data.def(d);
    if def.pub_visible || def.def_type != DefType::Function {
        return None;
    }
    let a = def.attributes.get(idx)?;
    let Type::Reference(record, _) = a.typedef.base() else {
        return None;
    };
    let v = def.variables.var(&a.name);
    if v == u16::MAX || !def.variables.is_argument(v) || def.variables.is_captured(v) {
        return None;
    }
    let layout = flat_layout(data, stores, *record, tp)?;
    layouts.insert(*record, layout);
    Some(*record)
}

/// The record's tuple, when it has two or more fields and every one is a scalar a tuple
/// carries at the width the field stores it.
fn flat_layout(data: &Data, stores: &Stores, record: u32, tp: u16) -> Option<Layout> {
    let Parts::Struct(fields) = &stores.types.get(tp as usize)?.parts else {
        return None;
    };
    let mut out = Vec::new();
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
        out.push((i32::from(f.position), kind));
    }
    out.sort_unstable_by_key(|(o, _)| *o);
    // A loft tuple has two or more elements: a one-element `Type::Tuple` has no spelling,
    // and `--native` renders it as the bare scalar its `.0` reads cannot index.
    (out.len() >= 2).then_some(Layout { tp, fields: out })
}

/// What a body does with its return buffer, beyond the object literals it fills.
#[derive(Default)]
struct BodyShape {
    /// Locals bound from a call handed the buffer — a FORWARD's bind.
    bound: HashSet<u16>,
    /// Locals in a result position: each must be one of `bound`.
    returned: HashSet<u16>,
}

/// Does `n` use the return buffer `buf` only the ways the tuple form can drop?  A RESULT
/// position (`tail`: a `return`, the body's last value) is an object literal filling every
/// field, or a local bound from a call handed `buf` (a forward, `return g(…)` lowered).
/// Elsewhere `buf` appears only as such a call's last argument, in the witness a forward
/// keeps of it (`w = OpRefAlias(buf)`) and in the guarded free of it
/// (`if OpDistinctStore(buf, w) OpFreeRefIfDistinct(buf, …)`).  Anything else in a result
/// position — a view of a record, a branch — declines: the tuple form has no spelling for it
/// here.
fn body_shape(
    n: &Value,
    buf: u16,
    layout: &Layout,
    ops: &Ops,
    tail: bool,
    sh: &mut BodyShape,
) -> bool {
    let n = n.unspan();
    if let Some(fill) = literal_fill(n, buf, layout, ops) {
        return tail
            && fill
                .values
                .iter()
                .all(|v| !mentions(v, buf) && body_shape(v, buf, layout, ops, false, sh));
    }
    match n {
        Value::Return(v) => body_shape(v, buf, layout, ops, true, sh),
        Value::Block(b) => {
            let last = b.operators.len().saturating_sub(1);
            b.operators
                .iter()
                .enumerate()
                .all(|(i, op)| body_shape(op, buf, layout, ops, tail && i == last, sh))
        }
        // A forward's local — or the buffer itself, which a one-buffer chain
        // (`buf = g(…, buf); buf`) rebinds to the call's result.
        Value::Var(l) if tail => {
            sh.returned.insert(*l);
            true
        }
        _ if tail => false,
        Value::Set(l, inner)
            if let Value::Call(_, args) = inner.unspan()
                && matches!(args.last().map(Value::unspan), Some(Value::Var(b)) if *b == buf) =>
        {
            sh.bound.insert(*l);
            args[..args.len() - 1]
                .iter()
                .all(|a| !mentions(a, buf) && body_shape(a, buf, layout, ops, false, sh))
        }
        Value::Set(_, inner)
            if matches!(inner.unspan(), Value::Call(op, args) if *op == ops.ref_alias
                && matches!(args.as_slice(), [a] if matches!(a.unspan(), Value::Var(b) if *b == buf))) =>
        {
            true
        }
        Value::If(cond, ..) if is_buffer_guard(n, cond, buf, ops) => true,
        _ => {
            let mut ok = true;
            each_child(n, &mut |c| {
                ok = ok && body_shape(c, buf, layout, ops, false, sh);
            });
            ok && !names_var(n, buf)
        }
    }
}

/// `if OpDistinctStore(buf, w) OpFreeRefIfDistinct(buf, …) else null` — the free a forward
/// guards by the witness it keeps of its buffer.
fn is_buffer_guard(n: &Value, cond: &Value, buf: u16, ops: &Ops) -> bool {
    let is_buf = |a: &Value| matches!(a.unspan(), Value::Var(b) if *b == buf);
    let Value::If(_, then, other) = n else {
        return false;
    };
    matches!(cond.unspan(), Value::Call(op, args) if *op == ops.distinct_store
        && args.first().is_some_and(is_buf))
        && matches!(then.unspan(), Value::Call(op, args) if *op == ops.free_if_distinct
            && args.first().is_some_and(is_buf))
        && matches!(other.unspan(), Value::Null)
}

/// An object literal built into `buf`: the field offsets and values in the order it writes
/// them.
struct Fill<'a> {
    offsets: Vec<i32>,
    values: Vec<&'a Value>,
}

/// `{ if … OpDatabase(buf, tp); OpSetX(buf, off, value)…; buf }` filling every field of
/// `layout` exactly once.
fn literal_fill<'a>(n: &'a Value, buf: u16, layout: &Layout, ops: &Ops) -> Option<Fill<'a>> {
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
    let mut offsets = Vec::new();
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
            || layout.kind_at(*off) != Some(kind)
            || offsets.contains(off)
        {
            return None;
        }
        offsets.push(*off);
        values.push(value);
    }
    (offsets.len() == layout.fields.len()).then_some(Fill { offsets, values })
}

/// The literal's opening test — and the loop-hoisted buffer's mint guard, which is the same
/// shape: an `if` that mints a record into `buf` when it holds none, reading nothing but
/// `buf` and calling nothing but the three operators it is built from.
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
        Value::Call(_, a)
        | Value::CallRef(_, a)
        | Value::Insert(a)
        | Value::Tuple(a)
        | Value::Parallel(a) => a.iter().for_each(f),
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

/// What one function's rewrite needs.
#[derive(Default)]
struct Plan {
    /// Carrier → the record it carries: admitted locals and admitted parameters alike.
    carriers: HashMap<u16, u32>,
    /// The buffers of the carrier locals, whose init, mint and frees leave with them.
    buffers: HashSet<u16>,
    /// Some admitted parameter is handed a plain record local, whose fields the rewrite
    /// reads into a tuple at the call.
    reads_at_call: bool,
}

/// What one walk of a function decided against the admitted sets.
#[derive(Default)]
struct Verdict {
    bad_fns: HashSet<u32>,
    bad_params: HashSet<(u32, usize)>,
    /// Every function this one calls.
    called: HashSet<u32>,
    /// Admitted functions this one still calls for a RECORD: the site is no carrier's bind.
    owed: HashSet<u32>,
}

/// Everything one walk of a function learns.
#[derive(Default)]
struct Scan {
    /// `(local, callee, buffer)` per bind from an admitted call.
    binds: Vec<(u16, u32, Option<u16>)>,
    /// Field reads: `(variable, offset, kind)`.
    reads: Vec<(u16, i32, Kind)>,
    /// A variable handed whole to admitted parameter `(function, index)`.
    handed: Vec<(u16, u32, usize)>,
    /// Statement `OpFreeRef(variable)`.
    frees: HashSet<u16>,
    /// Statement `OpFreeRefIfDistinct(placeholder, witness)`: frees the placeholder's store
    /// unless the witness names it.
    fids: Vec<(u16, u16)>,
    /// Variables the function returns: `return x`, or `x` as the body's last value.
    returns: HashSet<u16>,
    /// Statement `w = OpRefAlias(buffer)`: `(w, buffer)` — the witness a forward keeps.
    aliases: Vec<(u16, u16)>,
    /// Statement `if OpDistinctStore(buffer, w) OpFreeRefIfDistinct(buffer, …)`, by buffer.
    guards: Vec<u16>,
    /// Statement `OpCopyRecord(variable, place, tp)`: `(variable, tp, place is pure)`.
    copies: Vec<(u16, u16, bool)>,
    /// Statement `variable = null`.
    null_inits: Vec<u16>,
    /// Statement `if OpRefIsNull(buffer) { OpDatabase(buffer, tp) }` — a hoisted buffer.
    mints: Vec<u16>,
    /// Every other mention, by variable.
    other: HashSet<u16>,
    /// Calls of each admitted function, binds included.
    cand_calls: HashMap<u32, usize>,
    /// Admitted parameters handed something that is not a variable.
    unserved: HashSet<(u32, usize)>,
    called: HashSet<u32>,
}

/// The plan for function `f`'s body, and what it decides against the admitted sets.
/// `tuple` says `f` itself returns the tuple — the only case a forward's local, built in
/// `f`'s own buffer, may carry it.
fn plan_function(data: &Data, w: &World, f: u32, tuple: bool) -> (Plan, Verdict) {
    let def = data.def(f);
    let vars = &def.variables;
    let mut s = Scan::default();
    match def.code().unspan() {
        Value::Block(b) => {
            let last = b.operators.len().saturating_sub(1);
            for (i, op) in b.operators.iter().enumerate() {
                if i == last
                    && let Value::Var(x) = op.unspan()
                {
                    s.returns.insert(*x);
                } else {
                    scan(op, true, w, &mut s);
                }
            }
        }
        code => scan(code, false, w, &mut s),
    }
    let mut v = Verdict {
        bad_fns: HashSet::default(),
        bad_params: s.unserved.clone(),
        called: s.called.clone(),
        owed: HashSet::default(),
    };
    let mut plan = Plan::default();
    // The handed uses of `x` all go to parameters of `record`.
    let handed_fits = |x: u16, record: u32| {
        s.handed
            .iter()
            .filter(|(h, _, _)| *h == x)
            .all(|(_, d, i)| w.params.get(&(*d, *i)) == Some(&record))
    };
    let reads_fit = |x: u16, record: u32| {
        let layout = &w.layouts[&record];
        s.reads
            .iter()
            .filter(|(l, _, _)| *l == x)
            .all(|(_, off, kind)| layout.kind_at(*off) == Some(*kind))
    };
    let depended = |x: u16| (0..vars.count()).any(|y| vars.tp(y).depend().contains(&x));
    // Every copy of `x` into a place is one of its record into a place the rewrite may name
    // once per field.
    let copies_fit = |x: u16, record: u32| {
        let tp = w.layouts[&record].tp;
        s.copies
            .iter()
            .filter(|(c, _, _)| *c == x)
            .all(|(_, t, pure)| *t == tp && *pure)
    };
    // The admitted parameters of `f` itself.
    for (&(d, idx), &record) in &w.params {
        if d != f {
            continue;
        }
        let p = vars.var(&def.attributes[idx].name);
        let ok = !s.other.contains(&p)
            && !s.returns.contains(&p)
            && !s.null_inits.contains(&p)
            && !s.frees.contains(&p)
            && !s.binds.iter().any(|(l, _, _)| *l == p)
            && !s.fids.iter().any(|&(a, b)| a == p || b == p)
            && copies_fit(p, record)
            && reads_fit(p, record)
            && handed_fits(p, record)
            && !depended(p);
        if ok {
            plan.carriers.insert(p, record);
        } else {
            if trace() {
                eprintln!(
                    "[ir-valuerec] {}: parameter `{}` declines",
                    def.name(),
                    vars.name(p)
                );
            }
            v.bad_params.insert((d, idx));
        }
    }
    // The locals bound from admitted calls.
    let mut by_local: HashMap<u16, Vec<(u32, Option<u16>)>> = HashMap::default();
    for &(l, d, r) in &s.binds {
        by_local.entry(l).or_default().push((d, r));
    }
    let mut buffer_uses: HashMap<u16, usize> = HashMap::default();
    for binds in by_local.values() {
        for r in binds.iter().filter_map(|(_, r)| *r) {
            *buffer_uses.entry(r).or_default() += 1;
        }
    }
    for (&l, binds) in &by_local {
        let record = w.cands[&binds[0].0].record;
        // A FORWARD's local is built in this function's own buffer, the one parameter a
        // bind may name — and only while this function returns the tuple too.
        let own = w
            .cands
            .get(&f)
            .filter(|c| tuple && c.forwards.contains(&l))
            .map(|c| c.buf);
        let buffer_ok = binds.iter().all(|(d, r)| {
            w.cands[d].record == record
                && r.is_some_and(|r| {
                    let forward = own == Some(r);
                    (buffer_uses[&r] == 1 || r == l)
                        && !s.other.contains(&r)
                        && (forward || !vars.is_argument(r))
                        && (forward
                            || (!s.guards.contains(&r) && !s.aliases.iter().any(|(_, b)| *b == r)))
                        && s.fids.iter().all(|&(a, b)| b != r || a == l)
                })
        });
        let why = if !buffer_ok {
            Some("a bind's buffer")
        } else if s.returns.contains(&l) && own.is_none() {
            Some("returned by a function that keeps its record")
        } else if s.other.contains(&l) {
            Some("a mention that is no read, hand-on, bind or free")
        } else if (vars.is_argument(l) && own != Some(l)) || vars.is_captured(l) {
            Some("a parameter or a capture")
        } else if !matches!(vars.tp(l).peel_link(), Type::Reference(rd, _) if *rd == record) {
            Some("its type")
        } else if !reads_fit(l, record) || !handed_fits(l, record) {
            Some("a read or hand-on of another shape")
        } else if !copies_fit(l, record) {
            Some("a copy into a place it cannot name per field")
        } else if !s
            .fids
            .iter()
            .all(|&(a, b)| b != l || binds.iter().any(|(_, r)| *r == Some(a)))
        {
            Some("a free it only witnesses")
        } else if depended(l) {
            Some("another variable depends on it")
        } else {
            None
        };
        if let Some(why) = why
            && trace()
        {
            eprintln!(
                "[ir-valuerec] {}: local `{}` declines: {why}",
                def.name(),
                vars.name(l)
            );
        }
        let ok = why.is_none();
        // A local that cannot carry the tuple keeps its record: its binds MATERIALISE the
        // tuple into the buffer they pass, and the callee still returns a tuple everywhere.
        if ok {
            plan.carriers.insert(l, record);
            plan.buffers.extend(binds.iter().filter_map(|(_, r)| *r));
        }
    }
    // Every call of an admitted function that is no carrier's bind still wants its record.
    let mut carried_calls: HashMap<u32, usize> = HashMap::default();
    for (l, binds) in &by_local {
        if plan.carriers.contains_key(l) {
            for (d, _) in binds {
                *carried_calls.entry(*d).or_default() += 1;
            }
        }
    }
    for (d, n) in &s.cand_calls {
        if carried_calls.get(d).copied().unwrap_or(0) < *n {
            v.owed.insert(*d);
        }
    }
    // A forward returns its local: a local that cannot carry the tuple leaves nothing to
    // return one with, so the function keeps its record.
    if tuple
        && let Some(c) = w.cands.get(&f)
        && !c.forwards.iter().all(|l| plan.carriers.contains_key(l))
    {
        if trace() {
            eprintln!(
                "[ir-valuerec] {}: a forwarded local cannot carry the tuple",
                def.name()
            );
        }
        v.bad_fns.insert(f);
    }
    // What is handed to an admitted parameter and is no carrier: a plain local of the
    // parameter's record has its fields read into a tuple at the call; anything else
    // declines the parameter.
    for &(x, d, i) in &s.handed {
        if plan.carriers.contains_key(&x) {
            continue;
        }
        let record = w.params[&(d, i)];
        if matches!(vars.tp(x), Type::Reference(rd, _) if *rd == record) {
            plan.reads_at_call = true;
        } else {
            v.bad_params.insert((d, i));
        }
    }
    (plan, v)
}

/// May `place` be evaluated once per field — variables, constants and field projections,
/// nothing that calls or writes?
fn pure_place(place: &Value, ops: &Ops) -> bool {
    !place.any_node(&mut |n| {
        !matches!(n, Value::Var(_) | Value::Int(_) | Value::Span(_))
            && !matches!(n, Value::Call(op, _) if *op == ops.get_field)
    })
}

/// The arguments of a call of `d`: one handed to an admitted parameter is recorded as
/// handed when it is a variable and as unserved otherwise; the rest are scanned as values.
fn scan_args(d: u32, args: &[Value], w: &World, s: &mut Scan) {
    for (i, a) in args.iter().enumerate() {
        if w.params.contains_key(&(d, i)) {
            if let Value::Var(x) = a.unspan() {
                s.handed.push((*x, d, i));
                continue;
            }
            s.unserved.insert((d, i));
        }
        scan(a, false, w, s);
    }
}

/// A value in a RESULT position: a variable there is returned, and a block's last value is
/// still the result.
fn scan_result(n: &Value, w: &World, s: &mut Scan) {
    match n.unspan() {
        Value::Var(x) => {
            s.returns.insert(*x);
        }
        Value::Block(b) => {
            let last = b.operators.len().saturating_sub(1);
            for (i, op) in b.operators.iter().enumerate() {
                if i == last {
                    scan_result(op, w, s);
                } else {
                    scan(op, true, w, s);
                }
            }
        }
        other => scan(other, false, w, s),
    }
}

fn scan(n: &Value, stmt: bool, w: &World, s: &mut Scan) {
    let ops = &w.ops;
    let n = n.unspan();
    match n {
        Value::Set(v, inner) => {
            if let Value::Call(d, args) = inner.unspan()
                && w.cands.contains_key(d)
            {
                let r = match args.last().map(Value::unspan) {
                    Some(Value::Var(r)) => Some(*r),
                    _ => None,
                };
                s.binds.push((*v, *d, r));
                *s.cand_calls.entry(*d).or_default() += 1;
                s.called.insert(*d);
                scan_args(*d, &args[..args.len().saturating_sub(1)], w, s);
                return;
            }
            if stmt && matches!(inner.unspan(), Value::Null) {
                s.null_inits.push(*v);
                return;
            }
            if stmt
                && let Value::Call(op, args) = inner.unspan()
                && *op == ops.ref_alias
                && let [a] = args.as_slice()
                && let Value::Var(b) = a.unspan()
            {
                s.aliases.push((*v, *b));
                return;
            }
            s.other.insert(*v);
            scan(inner, false, w, s);
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
                && let (Value::Var(a), Value::Var(b)) = (a.unspan(), b.unspan())
            {
                s.fids.push((*a, *b));
                return;
            }
            if stmt
                && *op == ops.copy_record
                && let [src, dst, Value::Int(tp)] = args.as_slice()
                && let Value::Var(v) = src.unspan()
            {
                s.copies.push((*v, *tp as u16, pure_place(dst, ops)));
                scan(dst, false, w, s);
                return;
            }
            s.called.insert(*op);
            if w.cands.contains_key(op) {
                *s.cand_calls.entry(*op).or_default() += 1;
            }
            scan_args(*op, args, w, s);
        }
        Value::Return(x) => scan_result(x, w, s),
        Value::Block(b) | Value::Loop(b) => {
            for op in &b.operators {
                scan(op, true, w, s);
            }
        }
        Value::Insert(list) => {
            for op in list {
                scan(op, true, w, s);
            }
        }
        Value::If(cond, ..)
            if stmt
                && let Value::Call(_, args) = cond.unspan()
                && let Some(Value::Var(b)) = args.first().map(Value::unspan)
                && is_buffer_guard(n, cond, *b, ops) =>
        {
            s.guards.push(*b);
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
            each_child(n, &mut |c| scan(c, false, w, s));
        }
    }
}

/// Rewrite the body of `f` by `plan` — `f` itself, or the twin of the function whose body
/// it copied, which is the function the plan was made for.
fn rewrite_function(data: &mut Data, w: &World, twins: &HashMap<u32, u32>, f: u32, plan: &Plan) {
    let def = &mut data.definitions[f as usize];
    let mut code = std::mem::replace(&mut def.code, Value::Null);
    Rewrite { w, plan, twins }.uses(&mut code);
    def.code = code;
    for (v, record) in &plan.carriers {
        def.variables.set_type(*v, w.layouts[record].tuple_type());
    }
    for (&(d, idx), record) in &w.params {
        if d == f {
            def.attributes[idx].typedef = w.layouts[record].tuple_type();
        }
    }
}

/// Is statement `n` one the tuple form no longer needs: a buffer's null init or mint guard,
/// a carrier's scope-exit free, or the free of the buffer it might have been built in?
fn dropped(n: &Value, ops: &Ops, plan: &Plan) -> bool {
    let carried = |v: &u16| plan.carriers.contains_key(v) || plan.buffers.contains(v);
    match n.unspan() {
        Value::Set(r, inner) if matches!(inner.unspan(), Value::Null) => plan.buffers.contains(r),
        // The PLACEHOLDER is what a free releases; a carrier or its buffer holds no store.
        Value::Call(op, args) if *op == ops.free_ref || *op == ops.free_if_distinct => {
            matches!(args.first().map(Value::unspan), Some(Value::Var(v)) if carried(v))
        }
        Value::Set(_, inner)
            if matches!(inner.unspan(), Value::Call(op, args) if *op == ops.ref_alias
                && matches!(args.as_slice(), [a] if matches!(a.unspan(), Value::Var(b) if plan.buffers.contains(b)))) =>
        {
            true
        }
        Value::If(cond, ..) => match cond.unspan() {
            Value::Call(op, args) if *op == ops.ref_is_null => {
                matches!(args.as_slice(), [Value::Var(r)] if plan.buffers.contains(r)
                    && is_header(n.unspan(), *r, ops))
            }
            Value::Call(_, args) => {
                matches!(args.first().map(Value::unspan), Some(Value::Var(b)) if plan.buffers.contains(b)
                    && is_buffer_guard(n.unspan(), cond, *b, ops))
            }
            _ => false,
        },
        _ => false,
    }
}

/// One function's rewrite: the admitted world, its plan, and the twins a carrier's bind
/// calls instead of a function that keeps its record for other sites.
struct Rewrite<'a> {
    w: &'a World,
    plan: &'a Plan,
    twins: &'a HashMap<u32, u32>,
}

impl Rewrite<'_> {
    fn uses(&mut self, n: &mut Value) {
        let (w, plan) = (self.w, self.plan);
        let ops = &w.ops;
        let n = n.unspan_mut();
        match n {
            Value::Set(v, inner) if plan.carriers.contains_key(v) => {
                // A carrier's null init reads, field by field, what a field read of a null
                // record answers — so a read before the first bind is unchanged.
                if matches!(inner.unspan(), Value::Null) {
                    **inner = null_tuple(&w.layouts[&plan.carriers[v]]);
                    return;
                }
                if let Value::Call(d, args) = inner.unspan_mut() {
                    args.pop();
                    if let Some(t) = self.twins.get(d) {
                        *d = *t;
                    }
                    let d = *d;
                    self.args(d, args);
                }
            }
            Value::Call(op, args) => {
                if ops.get_kind(*op).is_some()
                    && let [target, Value::Int(off)] = args.as_slice()
                    && let Value::Var(v) = target.unspan()
                    && let Some(record) = plan.carriers.get(v)
                    && let Some(i) = w.layouts[record].index_of(*off)
                {
                    *n = Value::TupleGet(*v, i);
                    return;
                }
                let d = *op;
                self.args(d, args);
            }
            Value::Block(b) | Value::Loop(b) => self.statements(&mut b.operators),
            Value::Insert(list) => self.statements(list),
            _ => each_child_mut(n, &mut |c| self.uses(c)),
        }
    }

    /// A statement list: the statements the tuple form no longer needs leave, a carrier
    /// copied into a place becomes that place's field writes IN the list — where a literal
    /// the copy fills reads them as its own writes — and the rest are rewritten.
    fn statements(&mut self, list: &mut Vec<Value>) {
        let (w, plan) = (self.w, self.plan);
        let old = std::mem::take(list);
        for mut o in old {
            if dropped(&o, &w.ops, plan) {
                continue;
            }
            if let Value::Call(op, args) = o.unspan()
                && *op == w.ops.copy_record
                && let [src, dst, _] = args.as_slice()
                && let Value::Var(v) = src.unspan()
                && let Some(record) = plan.carriers.get(v)
            {
                let v = *v;
                let mut place = dst.clone();
                self.uses(&mut place);
                list.extend(write_fields(&w.layouts[record], &place, v, &w.ops));
                continue;
            }
            self.uses(&mut o);
            list.push(o);
        }
    }

    /// The arguments of a call of `d`: a plain record local handed to an admitted parameter
    /// is read into a tuple at the call; every other argument is rewritten as a value.
    fn args(&mut self, d: u32, args: &mut [Value]) {
        let w = self.w;
        for (i, a) in args.iter_mut().enumerate() {
            if let Some(record) = w.params.get(&(d, i))
                && let Value::Var(x) = a.unspan()
                && !self.plan.carriers.contains_key(x)
            {
                *a = read_into_tuple(*x, &w.layouts[record], &w.ops);
                continue;
            }
            self.uses(a);
        }
    }
}

/// `OpSetX(place, off, t.i)` for every field: tuple `t` written into the record at `place`.
fn write_fields(layout: &Layout, place: &Value, t: u16, ops: &Ops) -> Vec<Value> {
    layout
        .fields
        .iter()
        .enumerate()
        .map(|(i, (off, k))| {
            Value::Call(
                ops.set_op(*k),
                vec![
                    place.clone(),
                    Value::Int(*off),
                    Value::TupleGet(t, i as u16),
                ],
            )
        })
        .collect()
}

/// The tuple a null record reads as: each field's null — `i64::MIN`, NaN, `false`.
fn null_tuple(layout: &Layout) -> Value {
    Value::Tuple(
        layout
            .fields
            .iter()
            .map(|(_, k)| match k {
                Kind::Int => Value::Long(i64::MIN),
                Kind::Float => Value::Float(f64::NAN),
                Kind::Single => Value::Single(f32::NAN),
                Kind::Bool => Value::Boolean(false),
            })
            .collect(),
    )
}

/// The tuple of plain record local `x`'s fields, read at the call — what an admitted
/// parameter receives from an argument that is no carrier.  Reading now equals reading
/// inside the callee because nothing the callee reaches writes a record of this type (the
/// parameter gate `value_records` admits by).
fn read_into_tuple(x: u16, layout: &Layout, ops: &Ops) -> Value {
    Value::Tuple(
        layout
            .fields
            .iter()
            .map(|(off, k)| Value::Call(ops.get_op(*k), vec![Value::Var(x), Value::Int(*off)]))
            .collect(),
    )
}

fn rewrite_callee(data: &mut Data, w: &World, d: u32, c: &Callee) {
    let layout = &w.layouts[&c.record];
    let tuple = layout.tuple_type();
    let old = data.def(d).returned().clone();
    let def = &mut data.definitions[d as usize];
    let mut code = std::mem::replace(&mut def.code, Value::Null);
    literals_to_tuples(&mut code, c.buf, layout, &w.ops, &mut def.variables);
    def.code = code;
    retype_results(&mut def.code, c.record, &old, &tuple);
    // A one-buffer chain made the buffer itself the forward's local.  As a parameter it was
    // declared by the signature; as a local it is declared by its first bind, which sits in
    // a branch — so it is bound to the null tuple first, at the body's own level.
    if c.forwards.contains(&c.buf)
        && let Value::Block(b) = def.code.unspan_mut()
    {
        b.operators
            .insert(0, Value::Set(c.buf, Box::new(null_tuple(layout))));
    }
    def.returned = tuple;
    def.attributes.pop();
    def.variables.drop_argument(c.buf);
}

/// The blocks in RESULT positions — the body, a returned block and their tails — are typed
/// by the record the function returned (possibly with the buffer as its dep); they now
/// answer the tuple.  A block elsewhere keeps its type: a local record of the same type is
/// no result.
fn retype_results(n: &mut Value, record: u32, old: &Type, tuple: &Type) {
    match n.unspan_mut() {
        Value::Block(b) => {
            if b.result == *old
                || matches!(b.result.peel_link(), Type::Reference(r, _) if *r == record)
            {
                b.result = tuple.clone();
            }
            for op in &mut b.operators {
                retype_returns(op, record, old, tuple);
            }
            if let Some(last) = b.operators.last_mut() {
                retype_results(last, record, old, tuple);
            }
        }
        Value::Return(x) => retype_results(x, record, old, tuple),
        _ => {}
    }
}

/// Every `return` below `n` — outside a nested function there is none — delivers the result.
fn retype_returns(n: &mut Value, record: u32, old: &Type, tuple: &Type) {
    if let Value::Return(x) = n.unspan_mut() {
        retype_results(x, record, old, tuple);
        return;
    }
    each_child_mut(n, &mut |c| retype_returns(c, record, old, tuple));
}

/// Every literal built into `buf` becomes its tuple, in SCHEMA order.  A literal written in
/// another order binds each value to a temporary first, in the order it wrote them, and the
/// tuple reads the temporaries — so a field expression with an effect runs where the
/// program put it.  The values come from the SAME [`literal_fill`] the admission read.
fn literals_to_tuples(
    n: &mut Value,
    buf: u16,
    layout: &Layout,
    ops: &Ops,
    vars: &mut crate::variables::Function,
) {
    let n = n.unspan_mut();
    if let Some(fill) = literal_fill(n, buf, layout, ops) {
        let offsets = fill.offsets.clone();
        let mut values: Vec<Value> = fill.values.into_iter().cloned().collect();
        let Value::Block(b) = &*n else { unreachable!() };
        let scope = b.scope;
        for v in &mut values {
            literals_to_tuples(v, buf, layout, ops, vars);
        }
        let in_order = offsets.iter().zip(&layout.fields).all(|(o, (f, _))| o == f);
        *n = if in_order {
            Value::Tuple(values)
        } else {
            let mut stmts = Vec::new();
            let mut temps: HashMap<i32, u16> = HashMap::default();
            for (off, value) in offsets.iter().zip(values) {
                let kind = layout.kind_at(*off).expect("a filled field");
                let t = vars.add_unique("vf", &kind.tuple_type(), scope);
                stmts.push(Value::Set(t, Box::new(value)));
                temps.insert(*off, t);
            }
            stmts.push(Value::Tuple(
                layout
                    .fields
                    .iter()
                    .map(|(o, _)| Value::Var(temps[o]))
                    .collect(),
            ));
            Value::Block(Box::new(Block {
                name: "Tuple",
                operators: stmts,
                result: layout.tuple_type(),
                scope,
                var_size: 0,
            }))
        };
        return;
    }
    each_child_mut(n, &mut |c| literals_to_tuples(c, buf, layout, ops, vars));
}
