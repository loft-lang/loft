// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! `@FR-R-Apart`, instance 2 — a record of fixed-length `vector<float>` fields held as a
//! Rust value (`[f64; N]` per field) while it travels between loft functions that only build
//! it, read and write its elements, and hand it on.  The store meets it once, at a BOUNDARY:
//! a `return` through the function's buffer, which converts it with the ordinary constructor.
//!
//! The plan is decided here, from the IR alone, before anything is emitted:
//!
//! - an APART TYPE is a struct whose every field is a `vector<float>`, and whose every
//!   construction in admitted code builds each field as a literal of one length — the
//!   length is a fact about the type, so a value read anywhere has the layout every
//!   function assumes;
//! - a TWIN is a function returning such a record through its buffer, re-emitted as
//!   `n_<f>__apv` taking its record parameters as `&` values and answering the value.  Every
//!   variable of the type in its body must take a role below, and every `return` must answer
//!   an apart value, or the function has no twin;
//! - a REGION is the set of variables of the type in a function's ordinary body that take a
//!   role, by connected component: a variable is apart only with every variable it is built
//!   from, handed to, freed against or aliased by.
//!
//! The roles, and every mention each may have — anything else is a VIOLATION that keeps its
//! whole component in the store, which costs the rewrite and never a value:
//!
//! - APART: its construction run (`OpDatabase` guard, `OpSetInt4` per field, the element
//!   pushes, a repeat literal's `OpAppendCopy`), an element read `OpGetFloat(OpGetVector(
//!   OpGetField(v, …), 8, i), 0)`, an element write `OpSetFloat(OpGetVector(…), 0, x)`, a
//!   length `OpLengthVector(OpGetField(v, …))`, the destination or a record argument of a
//!   twin's call, a free, a null init, and a `return`.  A record PARAMETER is apart only in
//!   a twin, and only if nothing writes or rebuilds it;
//! - DEAD: a variable only ever a twin call's buffer argument, an `OpRefAlias` of an apart
//!   variable, null-initialised or freed — under the rewrite nothing is minted into it.
//!
//! A twin's other parameters must be numbers (`integer`, `float`, `single`), so its call
//! hands them over as their values.
//! Why the fallback is sound: an unlisted mention declines the component, and an apart
//! value is a distinct Rust type, so a mention this gate admits in error and the emitter
//! does not rewrite is a `rustc` type error, not a wrong answer (APART_VALUES.md
//! requirement 2).
use crate::data::{Data, DefType, Type, Value};
use crate::database::{Parts, Stores};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};

/// One `vector<float>` field of an apart type: its byte offset in the record and its length.
#[derive(Clone, Debug)]
pub struct ApField {
    pub off: i64,
    pub len: usize,
}

/// The value form of one apart type.
#[derive(Clone, Debug)]
pub struct ApLayout {
    /// The Rust struct the value is (`Ap_<n>`), one per type.
    pub rust: String,
    /// The record's store type, which a boundary builds.
    pub rec_tp: u16,
    /// The vector type of each field, in field order — what a boundary's elements append to.
    pub vec_tps: Vec<u16>,
    pub fields: Vec<ApField>,
}

impl ApLayout {
    /// The field at byte offset `off`.
    #[must_use]
    pub fn field(&self, off: i64) -> Option<usize> {
        self.fields.iter().position(|f| f.off == off)
    }
}

/// The variables of one body and the role each takes.
#[derive(Clone, Debug, Default)]
pub struct Roles {
    /// Apart variables → their record definition.
    pub apart: HashMap<u16, u32>,
    pub dead: HashSet<u16>,
    /// Whether the body calls a twin.  A body holding a region is emitted with the vector
    /// hoists off, so a region whose values are only built, read or written in place would
    /// trade the hoists of the body's store reads for a conversion; the gain is in a chain
    /// of calls that never meets the store, so only a region with a twin call is used.
    pub profitable: bool,
}

impl Roles {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.apart.is_empty()
    }
}

/// The whole program's plan ([`plan`]).
#[derive(Default)]
pub struct ApartPlan {
    /// Record definition → its value form.
    pub types: HashMap<u32, ApLayout>,
    /// Functions with a twin → the record definition they answer.
    pub twins: HashMap<u32, u32>,
    /// The roles inside each twin's body.
    pub twin_roles: HashMap<u32, Roles>,
    /// The roles inside each function's ordinary body, where any variable is apart.
    pub regions: HashMap<u32, Roles>,
}

/// `LOFT_NO_APART=1` — every record lives in the store from its construction (`@FR-R-Apart`
/// off): the before-half of every A/B and the first bisect step for a native-only wrong
/// answer out of a record of fixed-length vectors.
#[must_use]
pub fn apart_disabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_NO_APART").is_ok_and(|v| v != "0"))
}

/// `LOFT_TRACE_APART=1` — every type, twin and region the plan admits, and each component it
/// declines with the variable whose mention declined it, on stderr.
fn trace_on() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_TRACE_APART").is_ok_and(|v| v != "0"))
}

/// The candidate types: structs whose fields are all `vector<float>`, lengths not yet known.
fn candidate_types(data: &Data, stores: &Stores) -> HashMap<u32, (u16, Vec<(i64, u16)>)> {
    let float_tp = stores.name("float");
    let mut out = HashMap::default();
    for (d, def) in data.definitions.iter().enumerate() {
        if !matches!(def.def_type, DefType::Struct) {
            continue;
        }
        let kt = def.known_type;
        let Some(t) = stores.types.get(kt as usize) else {
            continue;
        };
        let Parts::Struct(fields) = &t.parts else {
            continue;
        };
        if fields.is_empty() {
            continue;
        }
        let vectors: Option<Vec<(i64, u16)>> = fields
            .iter()
            .map(
                |f| match stores.types.get(f.content as usize).map(|t| &t.parts) {
                    Some(Parts::Vector(e)) if *e == float_tp => {
                        Some((i64::from(f.position), f.content))
                    }
                    _ => None,
                },
            )
            .collect();
        if let Some(v) = vectors {
            out.insert(d as u32, (kt, v));
        }
    }
    out
}

/// One element of a field's construction: a value, or `n` copies of the element before it.
#[derive(Clone, Debug)]
pub enum Elem<'a> {
    Val(&'a Value),
    Copies(usize),
}

/// A construction run over variable `v`: its elements in the ORDER the run writes them,
/// each with its field's layout index, so a value expression runs where the program put it.
#[derive(Debug)]
pub struct Construction<'a> {
    pub v: u16,
    pub order: Vec<(usize, Elem<'a>)>,
    /// The index of the first statement after the run.
    pub end: usize,
}

fn op_name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

/// `OpGetField(Var(v), Int(off), _)` → `(v, off)`.
fn field_of(data: &Data, x: &Value) -> Option<(u16, i64)> {
    if let Value::Call(d, args) = x.unspan()
        && op_name(data, *d) == "OpGetField"
        && let [vv, off, _] = args.as_slice()
        && let (Value::Var(v), Value::Int(off)) = (vv.unspan(), off.unspan())
    {
        return Some((*v, i64::from(*off)));
    }
    None
}

/// An element ADDRESS of a float field: `OpGetVector(OpGetField(v, off, _), 8, i)` — the
/// form that reports an index out of range — or `OpGetVectorNullable`, the quiet one `v[i]?`
/// takes.  Answers `(v, off, index, nullable)`.
fn element_of<'a>(data: &Data, x: &'a Value) -> Option<(u16, i64, &'a Value, bool)> {
    let Value::Call(d, args) = x.unspan() else {
        return None;
    };
    let nullable = match op_name(data, *d) {
        "OpGetVector" => false,
        "OpGetVectorNullable" => true,
        _ => return None,
    };
    if let [vec, size, idx] = args.as_slice()
        && matches!(size.unspan(), Value::Int(8))
        && let Some((v, off)) = field_of(data, vec)
    {
        return Some((v, off, idx, nullable));
    }
    None
}

/// An element READ `OpGetFloat(<element>, 0)` → `(v, off, index, nullable)`.
#[must_use]
pub fn element_read<'a>(data: &Data, x: &'a Value) -> Option<(u16, i64, &'a Value, bool)> {
    if let Value::Call(d, args) = x.unspan()
        && op_name(data, *d) == "OpGetFloat"
        && let [elem, fld] = args.as_slice()
        && matches!(fld.unspan(), Value::Int(0))
    {
        return element_of(data, elem);
    }
    None
}

/// An element WRITE `OpSetFloat(<element>, 0, val)` → `(v, off, index, val, nullable)`.
#[must_use]
pub fn element_write<'a>(
    data: &Data,
    x: &'a Value,
) -> Option<(u16, i64, &'a Value, &'a Value, bool)> {
    if let Value::Call(d, args) = x.unspan()
        && op_name(data, *d) == "OpSetFloat"
        && let [elem, fld, val] = args.as_slice()
        && matches!(fld.unspan(), Value::Int(0))
        && let Some((v, off, idx, nullable)) = element_of(data, elem)
    {
        return Some((v, off, idx, val, nullable));
    }
    None
}

/// A field's LENGTH `OpLengthVector(OpGetField(v, off, _))` → `(v, off)`.
#[must_use]
pub fn length_of(data: &Data, x: &Value) -> Option<(u16, i64)> {
    if let Value::Call(d, args) = x.unspan()
        && op_name(data, *d) == "OpLengthVector"
        && let [vec] = args.as_slice()
    {
        return field_of(data, vec);
    }
    None
}

/// The construction guard `if <test of v> null else OpDatabase(v, tp)` → `(v, tp)`.
fn guard_of(data: &Data, x: &Value) -> Option<(u16, i32)> {
    if let Value::If(test, then, els) = x.unspan()
        && matches!(then.unspan(), Value::Null)
        && let Value::Call(d, args) = els.unspan()
        && op_name(data, *d) == "OpDatabase"
        && let [vv, tp] = args.as_slice()
        && let (Value::Var(v), Value::Int(tp)) = (vv.unspan(), tp.unspan())
    {
        // The test reads the variable and nothing else.
        let mut only_v = true;
        test.any_node(&mut |n| {
            match n {
                Value::Var(u) if u != v => only_v = false,
                Value::Var(_) | Value::Boolean(_) | Value::Null => {}
                Value::Call(c, _)
                    if matches!(op_name(data, *c), "OpRefIsNull" | "OpConvBoolFromRef") => {}
                Value::If(..) => {}
                _ => only_v = false,
            }
            false
        });
        if only_v {
            return Some((*v, *tp));
        }
    }
    None
}

/// The construction run of a layout's record starting at `ops[at]`, if it is one: the
/// guard, an `OpSetInt4(v, off, 0)` per field, and each field's pushes (a literal's
/// `OpPushFloat`, an `Insert` of `OpPreAllocVector` + `OpPushFloat`, a repeat literal's
/// `OpAppendCopy` with a constant count of at least one), `null` statements between them.
#[must_use]
pub fn construction<'a>(
    data: &Data,
    layout: &ApLayout,
    ops: &'a [Value],
    at: usize,
    v_ok: &dyn Fn(u16) -> bool,
) -> Option<Construction<'a>> {
    let (var, rec_tp) = guard_of(data, ops.get(at)?)?;
    if !v_ok(var) || i64::from(rec_tp) != i64::from(layout.rec_tp) {
        return None;
    }
    let mut run = Run {
        data,
        layout,
        var,
        started: vec![false; layout.fields.len()],
        order: Vec::new(),
    };
    let mut pos = at + 1;
    while let Some(op) = ops.get(pos) {
        if !run.step(op) {
            break;
        }
        pos += 1;
    }
    run.started.iter().all(|s| *s).then_some(Construction {
        v: var,
        order: run.order,
        end: pos,
    })
}

/// A construction run being read, statement by statement.
struct Run<'a, 'd> {
    data: &'d Data,
    layout: &'d ApLayout,
    var: u16,
    /// Which fields the run has emptied (`OpSetInt4`), so their pushes belong to it.
    started: Vec<bool>,
    order: Vec<(usize, Elem<'a>)>,
}

impl<'a> Run<'a, '_> {
    /// The field of the run's variable that `vec` addresses (`OpGetField(v, off, _)`).
    fn field(&self, vec: &Value) -> Option<usize> {
        let (owner, off) = field_of(self.data, vec)?;
        (owner == self.var).then(|| self.layout.field(off))?
    }

    fn is(&self, x: &Value, op: &str) -> bool {
        matches!(x.unspan(), Value::Call(d, _) if op_name(self.data, *d) == op)
    }

    /// An element push of a started field.
    fn push(&mut self, x: &'a Value) -> bool {
        if let Value::Call(d, args) = x.unspan()
            && op_name(self.data, *d) == "OpPushFloat"
            && let [vec, val] = args.as_slice()
            && let Some(field) = self.field(vec)
            && self.started[field]
        {
            self.order.push((field, Elem::Val(val)));
            return true;
        }
        false
    }

    /// Takes statement `op` into the run; `false` ends it before `op`.
    fn step(&mut self, op: &'a Value) -> bool {
        match op.unspan() {
            Value::Null => true,
            Value::Call(d, args) if op_name(self.data, *d) == "OpSetInt4" => {
                let [owner, off, zero] = args.as_slice() else {
                    return false;
                };
                let (Value::Var(owner), Value::Int(off), Value::Int(0)) =
                    (owner.unspan(), off.unspan(), zero.unspan())
                else {
                    return false;
                };
                match self.layout.field(i64::from(*off)) {
                    Some(field) if *owner == self.var && !self.started[field] => {
                        self.started[field] = true;
                        true
                    }
                    _ => false,
                }
            }
            Value::Call(d, args) if op_name(self.data, *d) == "OpAppendCopy" => {
                let [vec, count, _] = args.as_slice() else {
                    return false;
                };
                let (Some(field), Value::Int(count)) = (self.field(vec), count.unspan()) else {
                    return false;
                };
                // The copies repeat the element just written, which must be this field's.
                if *count < 1
                    || !matches!(self.order.last(), Some((g, Elem::Val(_))) if *g == field)
                {
                    return false;
                }
                self.order.push((field, Elem::Copies(*count as usize - 1)));
                true
            }
            Value::Insert(items) => {
                let allocs_own = items.iter().all(|x| {
                    if self.is(x, "OpPreAllocVector")
                        && let Value::Call(_, args) = x.unspan()
                    {
                        return args.first().and_then(|v| self.field(v)).is_some();
                    }
                    true
                });
                if !allocs_own || !items.iter().any(|x| self.is(x, "OpPushFloat")) {
                    return false;
                }
                for x in items {
                    if self.is(x, "OpPushFloat") && !self.push(x) {
                        return false;
                    }
                }
                true
            }
            _ => self.push(op),
        }
    }
}

/// The length a construction gives each of `n` fields.
#[must_use]
pub fn lengths(c: &Construction<'_>, n: usize) -> Vec<usize> {
    let mut out = vec![0; n];
    for (f, e) in &c.order {
        out[*f] += match e {
            Elem::Val(_) => 1,
            Elem::Copies(k) => *k,
        };
    }
    out
}

/// The record definition a variable's type is, when it is a candidate type (never `τ?`).
fn record_of(tp: &Type, types: &HashMap<u32, (u16, Vec<(i64, u16)>)>) -> Option<u32> {
    if tp.peel_optional().1 {
        return None;
    }
    match tp.base() {
        Type::Reference(d, _) if types.contains_key(d) => Some(*d),
        _ => None,
    }
}

/// A scalar a twin's call can hand over by value.
fn scalar_param(tp: &Type) -> bool {
    matches!(tp.base(), Type::Integer(..) | Type::Float | Type::Single) && !tp.peel_optional().1
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Twin,
    Region,
}

/// The walk of one body: which variables each mention admits, and which it declines.
struct Gate<'a> {
    data: &'a Data,
    types: &'a HashMap<u32, (u16, Vec<(i64, u16)>)>,
    layouts: &'a HashMap<u32, ApLayout>,
    twins: &'a HashMap<u32, u32>,
    mode: Mode,
    def_nr: u32,
    /// Record-typed variables → their record.
    rec: HashMap<u16, u32>,
    /// The function's return buffer variable.
    rb: Option<u16>,
    violated: HashSet<u16>,
    why: HashMap<u16, &'static str>,
    union: Vec<(u16, u16)>,
    /// Variables used as an apart value (built, read, written, returned, handed on, bound).
    used: HashSet<u16>,
    /// Variables only a buffer argument or an alias.
    buffer: HashSet<u16>,
    written: HashSet<u16>,
    /// `(record, field lengths)` of every construction.
    built: Vec<(u32, Vec<usize>)>,
    bad_return: bool,
    /// The destinations of twin calls: what makes a region worth its emission.
    profit: HashSet<u16>,
}

impl<'a> Gate<'a> {
    fn is_r(&self, v: u16) -> bool {
        self.rec.contains_key(&v)
    }

    fn violate(&mut self, v: u16, why: &'static str) {
        if self.is_r(v) {
            self.violated.insert(v);
            self.why.entry(v).or_insert(why);
        }
    }

    fn layout(&self, v: u16) -> Option<&'a ApLayout> {
        self.layouts.get(self.rec.get(&v)?)
    }

    /// A twin call `dest = g(args…)`, or a `return g(args…)` when `dest` is the buffer.
    fn twin_call(&mut self, dest: u16, g: u32, args: &'a [Value]) -> bool {
        if self.twins.get(&g) != self.rec.get(&dest) {
            return false;
        }
        let gdef = self.data.def(g);
        let Some(rbi) = super::hoist::ret_buffer_attr(gdef) else {
            return false;
        };
        if args.len() != gdef.attributes().len() {
            return false;
        }
        for (i, arg) in args.iter().enumerate() {
            let atp = &gdef.attributes()[i].typedef;
            if i == rbi {
                match arg.unspan() {
                    // The FORWARD form `b = g(…, b)`: the call writes into its destination,
                    // which is the value, not a separate buffer.
                    Value::Var(b) if *b == dest => {}
                    Value::Var(b) if self.is_r(*b) => {
                        self.union.push((dest, *b));
                        self.buffer.insert(*b);
                    }
                    // A call handed no buffer of its own (`null`) mints nothing either.
                    Value::Null => {}
                    _ => return false,
                }
            } else if record_of(atp, self.types).is_some() {
                match arg.unspan() {
                    Value::Var(u) if self.rec.get(u) == record_of(atp, self.types).as_ref() => {
                        self.union.push((dest, *u));
                        self.used.insert(*u);
                    }
                    _ => return false,
                }
            } else if !scalar_param(atp) {
                return false;
            }
        }
        // Only now: the call is a twin call, so its scalar arguments are walked as values.
        for (i, arg) in args.iter().enumerate() {
            if i != rbi && record_of(&gdef.attributes()[i].typedef, self.types).is_none() {
                self.walk(arg);
            }
        }
        self.used.insert(dest);
        self.profit.insert(dest);
        true
    }

    fn walk_ops(&mut self, ops: &'a [Value]) {
        let mut i = 0;
        while i < ops.len() {
            if let Some(c) = self.construction_at(ops, i) {
                let end = c.end;
                self.record_construction(&c);
                i = end;
                continue;
            }
            self.walk(&ops[i]);
            i += 1;
        }
    }

    fn construction_at(&self, ops: &'a [Value], at: usize) -> Option<Construction<'a>> {
        let (v, _) = guard_of(self.data, ops.get(at)?)?;
        let layout = self.layout(v)?;
        construction(self.data, layout, ops, at, &|u| u == v)
    }

    fn record_construction(&mut self, c: &Construction<'a>) {
        let rec = self.rec[&c.v];
        let n = self.layouts.get(&rec).map_or(0, |l| l.fields.len());
        self.built.push((rec, lengths(c, n)));
        self.used.insert(c.v);
        self.written.insert(c.v);
        for (_, e) in &c.order {
            if let Elem::Val(x) = e {
                self.walk(x);
            }
        }
    }

    /// An object block `{ <construction of v>; v }`, the value of a `return` or a bind.
    fn construction_block(&self, x: &'a Value) -> Option<Construction<'a>> {
        let Value::Block(bl) = x.unspan() else {
            return None;
        };
        let ops = &bl.operators;
        let c = self.construction_at(ops, 0)?;
        (c.end + 1 == ops.len() && matches!(ops[c.end].unspan(), Value::Var(u) if *u == c.v))
            .then_some(c)
    }

    #[expect(clippy::too_many_lines, reason = "one arm per admitted shape")]
    fn walk(&mut self, x: &'a Value) {
        let data = self.data;
        if let Some((v, off, idx, _)) = element_read(data, x)
            && self.is_r(v)
        {
            if self.layout(v).and_then(|l| l.field(off)).is_none() {
                self.violate(v, "a read of no layout field");
            }
            self.used.insert(v);
            self.walk(idx);
            return;
        }
        if let Some((v, off, idx, val, _)) = element_write(data, x)
            && self.is_r(v)
        {
            if self.layout(v).and_then(|l| l.field(off)).is_none() {
                self.violate(v, "a write of no layout field");
            }
            self.used.insert(v);
            self.written.insert(v);
            self.walk(idx);
            self.walk(val);
            return;
        }
        if let Some((v, off)) = length_of(data, x)
            && self.is_r(v)
        {
            if self.layout(v).and_then(|l| l.field(off)).is_none() {
                self.violate(v, "a length of no layout field");
            }
            self.used.insert(v);
            return;
        }
        match x.unspan() {
            Value::Var(v) => self.violate(*v, "a mention the value form does not serve"),
            Value::Set(v, rhs) if self.is_r(*v) => match rhs.unspan() {
                Value::Null => {}
                Value::Call(d, args)
                    if op_name(data, *d) == "OpRefAlias"
                        && matches!(args.as_slice(), [a] if matches!(a.unspan(), Value::Var(u) if self.is_r(*u))) =>
                {
                    if let [a] = args.as_slice()
                        && let Value::Var(u) = a.unspan()
                    {
                        self.union.push((*v, *u));
                        self.buffer.insert(*v);
                    }
                }
                Value::Call(g, args) if self.twins.contains_key(g) => {
                    if !self.twin_call(*v, *g, args) {
                        self.violate(*v, "a call that is no twin call");
                        rhs.for_each_child(&mut |c| self.walk(c));
                    }
                }
                _ => {
                    if let Some(c) = self.construction_block(rhs)
                        && c.v == *v
                    {
                        self.record_construction(&c);
                    } else {
                        self.violate(*v, "a bind of a value the form cannot hold");
                        self.walk(rhs);
                    }
                }
            },
            Value::Call(d, args) if op_name(data, *d) == "OpFreeRef" => {
                if let [a] = args.as_slice()
                    && let Value::Var(v) = a.unspan()
                    && self.is_r(*v)
                {
                    return;
                }
                for a in args {
                    self.walk(a);
                }
            }
            Value::Call(d, args) if op_name(data, *d) == "OpFreeRefIfDistinct" => {
                let vars: Vec<Option<u16>> = args
                    .iter()
                    .map(|a| match a.unspan() {
                        Value::Var(v) => Some(*v),
                        _ => None,
                    })
                    .collect();
                match vars.as_slice() {
                    [Some(a), Some(b)] if self.is_r(*a) && self.is_r(*b) => {
                        self.union.push((*a, *b));
                    }
                    _ => args.iter().for_each(|a| self.walk(a)),
                }
            }
            Value::Return(inner) => match inner.unspan() {
                Value::Var(v) if self.is_r(*v) => {
                    self.used.insert(*v);
                    if self.mode == Mode::Region {
                        match self.rb {
                            Some(rb) => self.union.push((*v, rb)),
                            None => self.violate(*v, "a return with no buffer to convert into"),
                        }
                    }
                }
                Value::Call(g, args) if self.twins.contains_key(g) && self.rb.is_some() => {
                    let rb = self.rb.unwrap_or(u16::MAX);
                    if !self.twin_call(rb, *g, args) {
                        self.violate(rb, "a returned call that is no twin call");
                        self.bad_return = true;
                        inner.for_each_child(&mut |c| self.walk(c));
                    }
                }
                _ => {
                    if let Some(c) = self.construction_block(inner) {
                        if let Some(rb) = self.rb {
                            self.union.push((c.v, rb));
                        }
                        self.record_construction(&c);
                    } else {
                        if self.mode == Mode::Twin {
                            self.bad_return = true;
                        }
                        self.walk(inner);
                    }
                }
            },
            Value::Block(bl) | Value::Loop(bl) => self.walk_ops(&bl.operators),
            Value::Insert(items) => self.walk_ops(items),
            Value::Iter(v, ..) => {
                self.violate(*v, "an iterator over the value");
                x.for_each_child(&mut |c| self.walk(c));
            }
            Value::CallRef(v, _) | Value::FnRef(_, v, _) | Value::FnRefDnr(v) => {
                self.violate(*v, "a function reference");
                x.for_each_child(&mut |c| self.walk(c));
            }
            Value::Yield(_) | Value::Parallel(_) => {
                // A value across a `yield` or into a worker is not admitted at all.
                let all: Vec<u16> = self.rec.keys().copied().collect();
                for v in all {
                    self.violate(v, "a yield or a parallel block in the body");
                }
                x.for_each_child(&mut |c| self.walk(c));
            }
            _ => x.for_each_child(&mut |c| self.walk(c)),
        }
    }
}

fn find(parent: &mut HashMap<u16, u16>, x: u16) -> u16 {
    let p = *parent.get(&x).unwrap_or(&x);
    if p == x {
        return x;
    }
    let r = find(parent, p);
    parent.insert(x, r);
    r
}

/// Walk one body in `mode` and answer its roles, or why it has none (a twin is all or
/// nothing: one declined variable, or one `return` that answers no apart value, and the
/// function has no twin).
fn roles_of(
    data: &Data,
    types: &HashMap<u32, (u16, Vec<(i64, u16)>)>,
    layouts: &HashMap<u32, ApLayout>,
    twins: &HashMap<u32, u32>,
    def_nr: u32,
    mode: Mode,
    built: &mut Vec<(u32, Vec<usize>)>,
) -> Result<Roles, String> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let mut rec: HashMap<u16, u32> = HashMap::default();
    for v in 0..vars.count() {
        if let Some(r) = record_of(vars.tp(v), types)
            && layouts.contains_key(&r)
        {
            rec.insert(v, r);
        }
    }
    if rec.is_empty() {
        return Err("no variable of an apart type".into());
    }
    let rb = super::hoist::ret_buffer_attr(def).map(|i| vars.var(&def.attributes()[i].name));
    let mut g = Gate {
        data,
        types,
        layouts,
        twins,
        mode,
        def_nr,
        rec: rec.clone(),
        rb: rb.filter(|b| rec.contains_key(b)),
        violated: HashSet::default(),
        why: HashMap::default(),
        union: Vec::new(),
        used: HashSet::default(),
        buffer: HashSet::default(),
        written: HashSet::default(),
        built: Vec::new(),
        bad_return: false,
        profit: HashSet::default(),
    };
    for &v in rec.keys() {
        if vars.is_captured(v) {
            g.violate(v, "captured by a closure");
        }
    }
    g.walk(def.code());
    // A parameter (not the buffer) of the record: apart only in a twin, never written.
    let params: Vec<u16> = rec
        .keys()
        .copied()
        .filter(|v| vars.is_argument(*v) && Some(*v) != rb)
        .collect();
    for p in params {
        if mode == Mode::Region {
            g.violate(p, "a record parameter outside a twin");
        } else if g.written.contains(&p) {
            g.violate(p, "a written record parameter");
        }
    }
    // A variable both a buffer and a value takes neither role.
    let both: Vec<u16> = g.buffer.intersection(&g.used).copied().collect();
    for v in both {
        g.violate(v, "both a buffer and a value");
    }
    let _ = g.def_nr;
    // Components: a violation anywhere declines every variable joined to it.
    let mut parent: HashMap<u16, u16> = HashMap::default();
    for &(a, b) in &g.union {
        let (ra, rb2) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb2 {
            parent.insert(ra, rb2);
        }
    }
    let bad_roots: HashSet<u16> = g.violated.iter().map(|v| find(&mut parent, *v)).collect();
    let mut roles = Roles::default();
    let mut declined: Vec<(u16, &'static str)> = Vec::new();
    for (&v, &r) in &rec {
        if bad_roots.contains(&find(&mut parent, v)) {
            if let Some(w) = g.why.get(&v) {
                declined.push((v, w));
            }
            continue;
        }
        if g.used.contains(&v) {
            roles.apart.insert(v, r);
        } else if g.buffer.contains(&v) || !vars.is_argument(v) {
            roles.dead.insert(v);
        }
    }
    roles.profitable = roles.apart.keys().any(|v| g.profit.contains(v));
    if mode == Mode::Twin {
        if g.bad_return {
            return Err("a return that answers no apart value".into());
        }
        if let Some((v, w)) = declined.first() {
            return Err(format!("{}: {w}", vars.name(*v)));
        }
    } else if trace_on() {
        for (v, w) in &declined {
            eprintln!(
                "[apart] {} keeps {} in the store: {w}",
                def.name(),
                vars.name(*v)
            );
        }
    }
    // The region's constructions count only for the variables it admitted.
    built.extend(g.built);
    Ok(roles)
}

/// One length per field of every type, from every admitted construction — or the types whose
/// constructions disagree, and those no admitted code builds (which have no length at all).
fn agreed_lengths(
    built: Vec<(u32, Vec<usize>)>,
    types: &HashMap<u32, (u16, Vec<(i64, u16)>)>,
) -> Result<HashMap<u32, Vec<usize>>, Vec<u32>> {
    let mut lens: HashMap<u32, Vec<usize>> = HashMap::default();
    let mut conflict: Vec<u32> = Vec::new();
    for (rec, l) in built {
        match lens.get(&rec) {
            Some(have) if *have != l => conflict.push(rec),
            Some(_) => {}
            None => {
                lens.insert(rec, l);
            }
        }
    }
    conflict.extend(types.keys().filter(|d| !lens.contains_key(d)));
    if conflict.is_empty() {
        Ok(lens)
    } else {
        Err(conflict)
    }
}

fn trace_plan(
    data: &Data,
    layouts: &HashMap<u32, ApLayout>,
    twins: &HashMap<u32, u32>,
    regions: &HashMap<u32, Roles>,
) {
    for (d, l) in layouts {
        eprintln!(
            "[apart] type {} as {}: {:?}",
            data.def(*d).name(),
            l.rust,
            l.fields.iter().map(|f| f.len).collect::<Vec<_>>()
        );
    }
    for f in twins.keys() {
        eprintln!("[apart] twin {}", data.def(*f).name());
    }
    for (f, r) in regions {
        let vars = data.def(*f).variables();
        let mut names: Vec<&str> = r.apart.keys().map(|v| vars.name(*v)).collect();
        names.sort_unstable();
        eprintln!(
            "[apart] region {}: {}",
            data.def(*f).name(),
            names.join(", ")
        );
    }
}

/// The twins: every function answering a candidate record through its buffer, narrowed to a
/// fixpoint (a twin's body may call another twin, so one that loses its twin can cost its
/// callers theirs), with the roles inside each and the constructions they admit.
#[expect(clippy::type_complexity, reason = "the three answers of one fixpoint")]
fn admit_twins(
    data: &Data,
    types: &HashMap<u32, (u16, Vec<(i64, u16)>)>,
    layouts: &HashMap<u32, ApLayout>,
    functions: &[u32],
) -> (
    HashMap<u32, u32>,
    HashMap<u32, Roles>,
    Vec<(u32, Vec<usize>)>,
) {
    let mut twins: HashMap<u32, u32> = functions
        .iter()
        .filter_map(|d| {
            let def = data.def(*d);
            super::hoist::ret_buffer_attr(def)?;
            Some((*d, record_of(def.returned(), types)?))
        })
        .collect();
    loop {
        let mut out: Vec<u32> = Vec::new();
        let mut roles: HashMap<u32, Roles> = HashMap::default();
        let mut built: Vec<(u32, Vec<usize>)> = Vec::new();
        for &f in twins.keys() {
            match roles_of(data, types, layouts, &twins, f, Mode::Twin, &mut built) {
                Ok(r) => {
                    roles.insert(f, r);
                }
                Err(why) => {
                    if trace_on() {
                        eprintln!("[apart] {} has no twin: {why}", data.def(f).name());
                    }
                    out.push(f);
                }
            }
        }
        if out.is_empty() {
            return (twins, roles, built);
        }
        for f in out {
            twins.remove(&f);
        }
    }
}

/// The program's plan: the types, the twins (a fixpoint — a twin's body may call another
/// twin), the field lengths every admitted construction agrees on, and every region.
#[must_use]
pub fn plan(data: &Data, stores: &Stores, value_fns: &HashSet<u32>) -> ApartPlan {
    let mut plan = ApartPlan::default();
    if apart_disabled() {
        return plan;
    }
    let mut types = candidate_types(data, stores);
    // Lengths are learned from the admitted constructions; a type whose constructions
    // disagree leaves the candidates and the plan is decided again without it.
    loop {
        let mut layouts: HashMap<u32, ApLayout> = types
            .iter()
            .map(|(&d, (kt, fields))| {
                (
                    d,
                    ApLayout {
                        // Named by the definition's source and name, which a program
                        // around a library does not change, where its number does.
                        rust: format!(
                            "Apart_{}_{}",
                            ident_part(&data.source_label(data.def(d).source)),
                            ident_part(data.def(d).name())
                        ),
                        rec_tp: *kt,
                        vec_tps: fields.iter().map(|f| f.1).collect(),
                        fields: fields
                            .iter()
                            .map(|f| ApField { off: f.0, len: 0 })
                            .collect(),
                    },
                )
            })
            .collect();
        if layouts.is_empty() {
            return plan;
        }
        let functions: Vec<u32> = (0..data.definitions.len() as u32)
            .filter(|d| {
                let def = data.def(*d);
                matches!(def.def_type, DefType::Function)
                    && !data.is_placeholder_instance(*d)
                    && *def.code() != Value::Null
                    && !matches!(def.returned().base(), Type::Iterator(_, _))
                    && !value_fns.contains(d)
            })
            .collect();
        let (twins, twin_roles, twin_built) = admit_twins(data, &types, &layouts, &functions);
        let mut regions: HashMap<u32, Roles> = HashMap::default();
        let mut built = twin_built;
        for &f in &functions {
            let mut region_built = Vec::new();
            if let Ok(r) = roles_of(
                data,
                &types,
                &layouts,
                &twins,
                f,
                Mode::Region,
                &mut region_built,
            ) && !r.is_empty()
                && r.profitable
            {
                built.extend(region_built);
                regions.insert(f, r);
            }
        }
        let lens = match agreed_lengths(built, &types) {
            Ok(lens) => lens,
            Err(conflict) => {
                for d in conflict {
                    if trace_on() && types.contains_key(&d) {
                        eprintln!(
                            "[apart] {} is no apart type: its constructions give no one length",
                            data.def(d).name()
                        );
                    }
                    types.remove(&d);
                }
                continue;
            }
        };
        for (d, l) in &lens {
            if let Some(layout) = layouts.get_mut(d) {
                for (f, n) in layout.fields.iter_mut().zip(l) {
                    f.len = *n;
                }
            }
        }
        if trace_on() {
            trace_plan(data, &layouts, &twins, &regions);
        }
        plan.types = layouts;
        plan.twins = twins;
        plan.twin_roles = twin_roles;
        plan.regions = regions;
        return plan;
    }
}

/// `s` as part of a Rust identifier: a source label can be a module path (`mesh3d::math`).
fn ident_part(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
