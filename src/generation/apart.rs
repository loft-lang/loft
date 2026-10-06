// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-Apart`, instance 2 — a record whose heap is fixed-length scalar vectors held as Rust
//! arrays on `--native`, from a construction the gate counted to its boundary
//! (`doc/claude/APART_VALUES.md` § Instance 2, as built).
//!
//! This module is the ONE admission gate: [`apart_values`] decides, once per program, which
//! record types have an apart form and the length `N` of each of their vector fields, which
//! functions get an apart TWIN (`__ap`), and which variables of every function — in its plain
//! form and in its twin — hold an apart value, with the calls that take a twin.  The emitter
//! reads that answer; a site it forgets is a `rustc` type error, never a value, because the
//! apart form is a distinct Rust type.
use std::collections::{HashMap, HashSet};

use crate::data::{Data, DefType, Type, Value};
use crate::database::{Parts, Stores};

/// One field of an apart record type, in the record's field order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApartPart {
    /// A scalar field at byte `off`, carried as the Rust scalar `rt`.
    Scalar { off: i64, rt: &'static str },
    /// A `vector` field at byte `off` whose elements are the Rust scalar `rt`, `width` bytes
    /// each in the store, held as `[rt; n]`.  `n` is `None` until a construction is counted.
    Vector {
        off: i64,
        rt: &'static str,
        width: i32,
        n: Option<u32>,
    },
}

/// The apart form of one record type.
#[derive(Clone, Debug, Default)]
pub struct ApartLayout {
    pub name: String,
    pub parts: Vec<ApartPart>,
}

impl ApartLayout {
    fn vector_at(&self, off: i64) -> Option<&ApartPart> {
        self.parts
            .iter()
            .find(|p| matches!(p, ApartPart::Vector { off: o, .. } if *o == off))
    }

    fn scalar_at(&self, off: i64) -> Option<&'static str> {
        self.parts.iter().find_map(|p| match p {
            ApartPart::Scalar { off: o, rt } if *o == off => Some(*rt),
            _ => None,
        })
    }
}

/// The apart variables of one function in one form, and the calls in it that take a twin.
#[derive(Clone, Debug, Default)]
pub struct ApartFrame {
    /// Variable → its record type.
    pub vars: HashMap<u16, u16>,
    /// The argument slices (by address) of the calls that take the callee's twin.
    pub calls: HashSet<usize>,
}

/// The gate's answer for a whole program.
#[derive(Debug, Default)]
pub struct ApartValues {
    /// Record type → its apart form, every vector field's `n` known.
    pub types: HashMap<u16, ApartLayout>,
    /// Functions with an apart twin → the twin's frame.  Every parameter of an apart type
    /// and the result, when it is of one, are apart in the twin.
    pub twins: HashMap<u32, ApartFrame>,
    /// Functions whose PLAIN form holds an apart variable → that frame.  Only the hidden
    /// return buffer may be returned, and the return materialises it.
    pub hosts: HashMap<u32, ApartFrame>,
}

/// `LOFT_NO_APART=1` — no apart type, no twin, no apart variable: every value lives in the
/// store.  The before-half of every A/B and the first bisect step for a native-only wrong
/// answer in arithmetic over a small record's vector.
#[must_use]
pub fn apart_disabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_NO_APART").is_ok_and(|v| v != "0"))
}

fn trace_on() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_TRACE_APART").is_ok())
}

/// The Rust scalar a store scalar of loft type `tp` is carried as.
fn scalar_rt(tp: &Type) -> Option<&'static str> {
    match tp.base() {
        Type::Float => Some("f64"),
        Type::Single => Some("f32"),
        Type::Integer(_) => Some("i64"),
        Type::Boolean => Some("bool"),
        _ => None,
    }
}

/// The record type `tp` names when it is a plain struct reference.
fn record_of(data: &Data, tp: &Type) -> Option<u16> {
    if matches!(tp, Type::Optional(_)) {
        return None;
    }
    let Type::Reference(d, _) = tp.peel_link() else {
        return None;
    };
    if data.def_type(*d) != DefType::Struct {
        return None;
    }
    let known = data.def(*d).known_type();
    (known != u16::MAX).then_some(known)
}

/// The apart form of record type `tp` (declared by `rd`), when it has one: every field a
/// scalar or a vector of `float`, `single` or 8-byte `integer`, and at least one vector.
fn layout_of(data: &Data, stores: &Stores, rd: u32, tp: u16) -> Result<ApartLayout, &'static str> {
    let Some(Parts::Struct(fields)) = stores.types.get(tp as usize).map(|t| &t.parts) else {
        return Err("not a struct");
    };
    let mut parts = Vec::new();
    let mut vectors = 0usize;
    for f in fields {
        let Some(a_nr) = data
            .def(rd)
            .attributes
            .iter()
            .position(|a| a.name == f.name)
        else {
            return Err("a field with no declaration");
        };
        let ftp = data.attr_type(rd, a_nr);
        if matches!(ftp, Type::Optional(_)) || data.def(rd).attributes[a_nr].nullable {
            return Err("a nullable field");
        }
        let off = i64::from(f.position);
        if let Some(rt) = scalar_rt(&ftp) {
            if rt == "i64" && stores.size(f.content) != 8 {
                return Err("a narrow integer field");
            }
            parts.push(ApartPart::Scalar { off, rt });
            continue;
        }
        let Type::Vector(elem, _) = ftp.base() else {
            return Err("a field that is neither a scalar nor a vector");
        };
        let rt = match elem.base() {
            Type::Float => "f64",
            Type::Single => "f32",
            Type::Integer(_) => "i64",
            _ => return Err("a vector whose elements are not float, single or integer"),
        };
        let Some(Parts::Vector(etp)) = stores.types.get(f.content as usize).map(|t| &t.parts)
        else {
            return Err("a vector field the schema does not lay out as a vector");
        };
        let width = i32::from(stores.size(*etp));
        if (rt == "i64" || rt == "f64") && width != 8 || rt == "f32" && width != 4 {
            return Err("a narrow element");
        }
        vectors += 1;
        parts.push(ApartPart::Vector {
            off,
            rt,
            width,
            n: None,
        });
    }
    if vectors == 0 {
        return Err("no vector field");
    }
    Ok(ApartLayout {
        name: data.def(rd).name().to_string(),
        parts,
    })
}

fn op_name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

fn is_var(v: &Value, var: u16) -> bool {
    matches!(v.unspan(), Value::Var(x) if *x == var)
}

/// `OpGetField(Var(v), off, _)` → `(v, off)`.
fn field_of(v: &Value, data: &Data) -> Option<(u16, i64)> {
    let Value::Call(d, args) = v.unspan() else {
        return None;
    };
    if op_name(data, *d) != "OpGetField" {
        return None;
    }
    match (
        args.first().map(Value::unspan),
        args.get(1).map(Value::unspan),
    ) {
        (Some(Value::Var(var)), Some(Value::Int(off))) => Some((*var, i64::from(*off))),
        _ => None,
    }
}

/// `OpGetVector(OpGetField(Var(v), off, _), width, idx)` → `(v, off, width, idx)`.
fn element_of<'a>(v: &'a Value, data: &Data) -> Option<(u16, i64, i32, &'a Value)> {
    let Value::Call(d, args) = v.unspan() else {
        return None;
    };
    if op_name(data, *d) != "OpGetVector" || args.len() != 3 {
        return None;
    }
    let (var, off) = field_of(&args[0], data)?;
    let Value::Int(w) = args[1].unspan() else {
        return None;
    };
    Some((var, off, *w, &args[2]))
}

/// The Rust scalar an element or field getter / setter / push of this name moves.
fn op_rt(name: &str) -> Option<&'static str> {
    let kind = name
        .strip_prefix("OpGet")
        .or_else(|| name.strip_prefix("OpSet"))
        .or_else(|| name.strip_prefix("OpPush"))?;
    match kind {
        "Float" => Some("f64"),
        "Single" => Some("f32"),
        "Int" => Some("i64"),
        "Boolean" => Some("bool"),
        _ => None,
    }
}

/// The function's apart-type parameters (attribute index → variable, type) and the hidden
/// return buffer's variable and type when its result is of an apart type.
struct Shape {
    params: Vec<(usize, u16, u16)>,
    buffer: Option<(usize, u16, u16)>,
    /// A parameter of an apart type the twin cannot take (a `&` link, a nullable one).
    blocked: Option<&'static str>,
}

fn shape_of(data: &Data, d_nr: u32, types: &HashMap<u16, ApartLayout>) -> Shape {
    let def = data.def(d_nr);
    let vars = def.variables();
    let buf_attr = def.hidden_return_buffer_attr();
    let mut out = Shape {
        params: Vec::new(),
        buffer: None,
        blocked: None,
    };
    for (i, a) in def.attributes().iter().enumerate() {
        let v = vars.var(&a.name);
        if v == u16::MAX {
            continue;
        }
        if Some(i) == buf_attr {
            if let Some(tp) = record_of(data, &a.typedef)
                && types.contains_key(&tp)
            {
                out.buffer = Some((i, v, tp));
            }
            continue;
        }
        if matches!(a.typedef.peel_link(), Type::Reference(_, _))
            || matches!(a.typedef, Type::RefVar(_))
        {
            let inner = match &a.typedef {
                Type::RefVar(t) => t.as_ref(),
                t => t,
            };
            if let Some(tp) = record_of(data, inner)
                && types.contains_key(&tp)
            {
                if matches!(a.typedef, Type::RefVar(_)) || a.nullable {
                    out.blocked = Some("a `&` or nullable parameter of an apart type");
                } else {
                    out.params.push((i, v, tp));
                }
            }
        }
    }
    out
}

/// Which form of a function is judged.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Mode {
    /// The plain function: its parameters are store records; an apart return buffer is
    /// materialised at the return.
    Host,
    /// The `__ap` twin: every apart-type parameter and the result are apart.
    Twin,
}

/// What one judgement of one function form found.
struct Judged {
    frame: ApartFrame,
    /// `(type, field offset)` → the lengths its literal groups build.
    lengths: Vec<((u16, i64), u32)>,
}

/// The walk over one function form, given the variables still believed apart.
struct Walk<'a> {
    data: &'a Data,
    types: &'a HashMap<u16, ApartLayout>,
    /// Twin candidates: callee → its shape (apart-type parameter attributes, result).
    twins: &'a HashMap<u32, (Vec<usize>, bool)>,
    apart: &'a HashMap<u16, u16>,
    /// Variables that may be written element-wise (the frame's own, not a parameter).
    owned: &'a HashSet<u16>,
    mode: Mode,
    buffer: Option<u16>,
    /// The first variable found misused, and why.
    bad: Option<(u16, &'static str)>,
    calls: HashSet<usize>,
    lengths: Vec<((u16, i64), u32)>,
    /// Open literal groups: `(var, off)` → elements counted so far.
    open: HashMap<(u16, i64), u32>,
    in_format: u32,
    /// Rebind witnesses of apart variables that nothing reads.
    witnesses: &'a HashSet<u16>,
}

impl Walk<'_> {
    fn is_apart(&self, v: u16) -> bool {
        self.apart.contains_key(&v)
    }

    fn fail(&mut self, v: u16, why: &'static str) {
        if self.bad.is_none() {
            self.bad = Some((v, why));
        }
    }

    fn layout(&self, v: u16) -> Option<&ApartLayout> {
        self.apart.get(&v).and_then(|tp| self.types.get(tp))
    }

    /// Close every open literal group: each records the length it built.
    fn close_groups(&mut self) {
        for ((var, off), n) in std::mem::take(&mut self.open) {
            if let Some(&tp) = self.apart.get(&var) {
                self.lengths.push(((tp, off), n));
            }
        }
    }

    /// One statement of a statement list: a literal-group step keeps the groups open,
    /// anything else closes them first.
    fn statement(&mut self, stmt: &Value) {
        if matches!(stmt.unspan(), Value::Null | Value::Line(_)) {
            return;
        }
        if self.group_step(stmt) {
            return;
        }
        self.close_groups();
        self.node(stmt);
    }

    /// A literal-group step on an apart variable: the mint guard, a vector field's zero
    /// (opening its group), a push or a constant repeat into an open group, a reservation,
    /// a scalar field's set.  Answers whether `stmt` was one.
    fn group_step(&mut self, stmt: &Value) -> bool {
        match stmt.unspan() {
            Value::Insert(ops) => {
                if ops.iter().all(|o| {
                    matches!(o.unspan(), Value::Null | Value::Line(_)) || self.is_group_shape(o)
                }) {
                    for o in ops {
                        self.group_step(o);
                    }
                    return true;
                }
                false
            }
            Value::If(cond, _, mint) if self.mint_guard(cond, mint).is_some() => true,
            Value::Call(d, args) => {
                let name = op_name(self.data, *d).to_string();
                match name.as_str() {
                    "OpSetInt4" => {
                        if let [Value::Var(v), Value::Int(off), Value::Int(0)] =
                            [args[0].unspan(), args[1].unspan(), args[2].unspan()]
                            && self.is_apart(*v)
                            && self
                                .layout(*v)
                                .and_then(|l| l.vector_at(i64::from(*off)))
                                .is_some()
                        {
                            self.open.insert((*v, i64::from(*off)), 0);
                            return true;
                        }
                        false
                    }
                    "OpPreAllocVector" => {
                        if let Some((v, off)) = field_of(&args[0], self.data)
                            && self.is_apart(v)
                            && self.open.contains_key(&(v, off))
                        {
                            for a in &args[1..] {
                                self.node(a);
                            }
                            return true;
                        }
                        false
                    }
                    "OpAppendCopy" => {
                        if let Some((v, off)) = field_of(&args[0], self.data)
                            && self.is_apart(v)
                        {
                            // The count is the run's TOTAL and its template is already
                            // appended (`OpAppendCopy`): `k - 1` more, or the template
                            // dropped for `k == 0`.
                            match (self.open.get_mut(&(v, off)), args.get(1).map(Value::unspan)) {
                                (Some(n), Some(Value::Int(k))) if *k >= 1 => {
                                    *n += *k as u32 - 1;
                                }
                                (Some(n), Some(Value::Int(0))) if *n > 0 => {
                                    *n -= 1;
                                }
                                (Some(_), _) => {
                                    self.fail(v, "a repeat whose count is not a constant")
                                }
                                (None, _) => self.fail(v, "a vector grown after its literal"),
                            }
                            return true;
                        }
                        false
                    }
                    n if n.starts_with("OpPush") => {
                        if let Some((v, off)) = field_of(&args[0], self.data)
                            && self.is_apart(v)
                        {
                            for a in &args[1..] {
                                self.node(a);
                            }
                            match self.open.get_mut(&(v, off)) {
                                Some(n) => *n += 1,
                                None => self.fail(v, "a vector grown after its literal"),
                            }
                            return true;
                        }
                        false
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    /// Is `o` a statement [`Self::group_step`] would take, without taking it?
    fn is_group_shape(&self, o: &Value) -> bool {
        match o.unspan() {
            Value::Call(d, args) => {
                let name = op_name(self.data, *d);
                (name == "OpPreAllocVector" || name == "OpAppendCopy" || name.starts_with("OpPush"))
                    && field_of(&args[0], self.data).is_some_and(|(v, _)| self.is_apart(v))
            }
            _ => false,
        }
    }

    /// `if <OpRefIsNull / OpConvBoolFromRef of v> … else OpDatabase(v, tp)` → `v`.
    fn mint_guard(&self, cond: &Value, mint: &Value) -> Option<u16> {
        let Value::Call(d, args) = mint.unspan() else {
            return None;
        };
        if op_name(self.data, *d) != "OpDatabase" {
            return None;
        }
        let Some(Value::Var(v)) = args.first().map(Value::unspan) else {
            return None;
        };
        if !self.is_apart(*v) {
            return None;
        }
        // The condition names `v` only through the null and presence tests.
        let mut only_tests = true;
        cond.any_node(&mut |n| {
            if let Value::Call(cd, cargs) = n
                && matches!(op_name(self.data, *cd), "OpRefIsNull" | "OpConvBoolFromRef")
                && cargs.len() == 1
                && is_var(&cargs[0], *v)
            {
                return false;
            }
            if let Value::Var(x) = n
                && self.is_apart(*x)
            {
                only_tests = false;
            }
            false
        });
        // `any_node` visits the tests' own `Var` children too; count mentions instead.
        let tests = count_tests(cond, *v, self.data);
        let mentions = mentions(cond, *v);
        (tests == mentions && (only_tests || tests > 0)).then_some(*v)
    }

    /// A call that may take the callee's twin: every apart-type argument an apart variable.
    fn twin_call(&self, d: u32, args: &[Value]) -> bool {
        let Some((params, _)) = self.twins.get(&d) else {
            return false;
        };
        params.iter().all(
            |&p| matches!(args.get(p).map(Value::unspan), Some(Value::Var(v)) if self.is_apart(*v)),
        )
    }

    fn node(&mut self, node: &Value) {
        match node.unspan() {
            Value::Var(v) if self.is_apart(*v) => {
                self.fail(*v, "a use the apart form does not serve")
            }
            Value::Block(bl) => {
                let saved = std::mem::take(&mut self.open);
                for s in &bl.operators {
                    self.statement(s);
                }
                self.close_groups();
                self.open = saved;
            }
            Value::Loop(bl) => {
                let saved = std::mem::take(&mut self.open);
                for s in &bl.operators {
                    self.statement(s);
                }
                self.close_groups();
                self.open = saved;
            }
            Value::Insert(ops) => {
                for s in ops {
                    self.statement(s);
                }
            }
            Value::Set(x, rhs) if self.is_apart(*x) => self.assign(*x, rhs),
            // A rebind witness nothing reads (`__rbw_x = OpRefAlias(x)`): emitted as nothing.
            Value::Set(w, _) if self.witnesses.contains(w) => {}
            Value::Set(_, rhs)
                if matches!(rhs.unspan(), Value::Call(d, a)
                if op_name(self.data, *d) == "OpRefAlias" && a.len() == 1
                    && matches!(a[0].unspan(), Value::Var(v) if self.is_apart(*v))) =>
            {
                // The rebind witness of a renamed buffer: a reference the apart form does
                // not have.  Its own mentions are judged where they stand.
                self.fail(
                    match rhs.unspan() {
                        Value::Call(_, a) => match a[0].unspan() {
                            Value::Var(v) => *v,
                            _ => u16::MAX,
                        },
                        _ => u16::MAX,
                    },
                    "a witness alias",
                );
            }
            Value::Return(r) => self.exit(r),
            Value::If(cond, t, f) => {
                if self.mint_guard(cond, f).is_some() {
                    return;
                }
                self.node(cond);
                self.node(t);
                self.node(f);
            }
            Value::Call(d, args) => self.call(*d, args),
            _ => {
                node.for_each_child(&mut |c| self.node(c));
            }
        }
    }

    /// `x = rhs` for an apart `x`.
    fn assign(&mut self, x: u16, rhs: &Value) {
        match rhs.unspan() {
            Value::Null => {}
            Value::Var(y) if self.is_apart(*y) && self.apart.get(y) == self.apart.get(&x) => {}
            Value::Call(d, args) if self.twin_call(*d, args) && self.twins[d].1 => {
                self.calls.insert(args.as_ptr() as usize);
                self.call_args(*d, args);
            }
            _ => self.fail(x, "an assignment that is not a construction"),
        }
    }

    /// The arguments of a call taking a twin: the apart ones are handed over, the hidden
    /// buffer is dropped, the rest are judged as they stand.
    fn call_args(&mut self, d: u32, args: &[Value]) {
        let params = &self.twins[&d].0;
        let buf = self.data.def(d).hidden_return_buffer_attr();
        for (i, a) in args.iter().enumerate() {
            if params.contains(&i) {
                continue;
            }
            if Some(i) == buf && matches!(a.unspan(), Value::Var(_)) {
                continue;
            }
            self.node(a);
        }
    }

    fn exit(&mut self, r: &Value) {
        let answer = answered_var(r);
        match answer {
            Some(v) if self.is_apart(v) => {
                if self.mode == Mode::Host && Some(v) != self.buffer {
                    self.fail(v, "returned, and not the frame's return buffer");
                }
                // The block's own statements still stand.
                if let Value::Block(bl) = r.unspan() {
                    let ops = &bl.operators[..bl.operators.len() - 1];
                    let saved = std::mem::take(&mut self.open);
                    for s in ops {
                        self.statement(s);
                    }
                    self.close_groups();
                    self.open = saved;
                }
            }
            _ => self.node(r),
        }
    }

    fn call(&mut self, d: u32, args: &[Value]) {
        let name = op_name(self.data, d).to_string();
        if name.contains("Format") {
            self.in_format += 1;
            for a in args {
                self.node(a);
            }
            self.in_format -= 1;
            return;
        }
        // Frees: a free of an apart variable is nothing, and a store test against one is
        // always distinct.
        if name == "OpFreeRef"
            && args.len() == 1
            && matches!(args[0].unspan(), Value::Var(v) if self.is_apart(*v))
        {
            return;
        }
        if name == "OpFreeRefIfDistinct" && args.len() == 2 {
            for a in args {
                if !matches!(a.unspan(), Value::Var(_)) {
                    self.node(a);
                }
            }
            return;
        }
        // An element read or write, a length, a scalar field read or write.
        if let Some(rt) = op_rt(&name)
            && let Some(first) = args.first()
        {
            if let Some((v, off, w, idx)) = element_of(first, self.data)
                && self.is_apart(v)
            {
                let Some(ApartPart::Vector { rt: ert, width, .. }) =
                    self.layout(v).and_then(|l| l.vector_at(off)).cloned()
                else {
                    self.fail(v, "an element of a field that is not an apart vector");
                    return;
                };
                if ert != rt
                    || width != w
                    || !matches!(args.get(1).map(Value::unspan), Some(Value::Int(0)))
                {
                    self.fail(v, "an element read at another width");
                    return;
                }
                if name.starts_with("OpGet") {
                    if self.in_format > 0 {
                        self.fail(v, "an element read under a format");
                        return;
                    }
                    self.node(idx);
                    return;
                }
                if name.starts_with("OpSet") {
                    if !self.owned.contains(&v) {
                        self.fail(v, "an element write through a parameter");
                        return;
                    }
                    self.node(idx);
                    for a in &args[2..] {
                        self.node(a);
                    }
                    return;
                }
            }
            if let Value::Var(v) = first.unspan()
                && self.is_apart(*v)
                && let Some(Value::Int(off)) = args.get(1).map(Value::unspan)
                && name != "OpSetInt4"
            {
                if self.layout(*v).and_then(|l| l.scalar_at(i64::from(*off))) != Some(rt) {
                    self.fail(*v, "a field access that is not a scalar of the apart form");
                    return;
                }
                if name.starts_with("OpSet") && !self.owned.contains(v) {
                    self.fail(*v, "a field write through a parameter");
                    return;
                }
                for a in &args[2..] {
                    self.node(a);
                }
                return;
            }
        }
        if matches!(name.as_str(), "OpLengthVector" | "t_6vector_len")
            && let Some((v, off)) = args.first().and_then(|a| field_of(a, self.data))
            && self.is_apart(v)
        {
            if self.layout(v).and_then(|l| l.vector_at(off)).is_none() {
                self.fail(v, "a length of a field that is not an apart vector");
            }
            return;
        }
        // A call that takes a twin without answering an apart value.
        if self.twin_call(d, args) && !self.twins[&d].1 {
            self.calls.insert(args.as_ptr() as usize);
            self.call_args(d, args);
            return;
        }
        for a in args {
            self.node(a);
        }
    }
}

/// The rebind witnesses (`w = OpRefAlias(x)`) whose only mention is that assignment.
fn dead_witnesses(body: &Value, data: &Data) -> HashSet<u16> {
    let mut found: HashSet<u16> = HashSet::new();
    body.any_node(&mut |n| {
        if let Value::Set(w, rhs) = n
            && let Value::Call(d, a) = rhs.unspan()
            && op_name(data, *d) == "OpRefAlias"
            && a.len() == 1
            && matches!(a[0].unspan(), Value::Var(_))
        {
            found.insert(*w);
        }
        false
    });
    // The assignment's target is no `Var` node: a witness nothing reads has none.
    found.retain(|w| mentions(body, *w) == 0);
    found
}

/// The variable an exit answers: `v`, or a block whose last statement is `v`.
fn answered_var(v: &Value) -> Option<u16> {
    match v.unspan() {
        Value::Var(x) => Some(*x),
        Value::Block(bl) => bl.operators.last().and_then(answered_var),
        _ => None,
    }
}

fn mentions(node: &Value, v: u16) -> usize {
    let mut n = 0;
    node.any_node(&mut |x| {
        if matches!(x, Value::Var(y) if *y == v) {
            n += 1;
        }
        false
    });
    n
}

fn count_tests(node: &Value, v: u16, data: &Data) -> usize {
    let mut n = 0;
    node.any_node(&mut |x| {
        if let Value::Call(d, a) = x
            && matches!(op_name(data, *d), "OpRefIsNull" | "OpConvBoolFromRef")
            && a.len() == 1
            && is_var(&a[0], v)
        {
            n += 1;
        }
        false
    });
    n
}

/// Judge one form of `d_nr`: shrink its candidate apart variables until every remaining one
/// is used only as the apart form serves.  A twin needs its parameters and its result apart;
/// a host keeps whatever survives.
fn judge(
    data: &Data,
    d_nr: u32,
    mode: Mode,
    types: &HashMap<u16, ApartLayout>,
    twins: &HashMap<u32, (Vec<usize>, bool)>,
) -> Result<Judged, &'static str> {
    let def = data.def(d_nr);
    let vars = def.variables();
    let shape = shape_of(data, d_nr, types);
    if mode == Mode::Twin
        && let Some(why) = shape.blocked
    {
        return Err(why);
    }
    let mut cand: HashMap<u16, u16> = HashMap::new();
    let mut owned: HashSet<u16> = HashSet::new();
    let mut required: HashSet<u16> = HashSet::new();
    if let Some((_, v, tp)) = shape.buffer {
        cand.insert(v, tp);
        owned.insert(v);
        if mode == Mode::Twin {
            required.insert(v);
        }
    }
    if mode == Mode::Twin {
        for (_, v, tp) in &shape.params {
            cand.insert(*v, *tp);
            required.insert(*v);
        }
    }
    let witnesses = dead_witnesses(def.code(), data);
    for v in 0..vars.count() {
        if vars.is_argument(v) || cand.contains_key(&v) || witnesses.contains(&v) {
            continue;
        }
        // A local nothing reads is no value at all: it keeps its store declaration.
        if let Some(tp) = record_of(data, vars.tp(v))
            && types.contains_key(&tp)
            && mentions(def.code(), v) > 0
        {
            cand.insert(v, tp);
            owned.insert(v);
        }
    }
    loop {
        let mut w = Walk {
            data,
            types,
            twins,
            apart: &cand,
            owned: &owned,
            mode,
            buffer: shape.buffer.map(|(_, v, _)| v),
            bad: None,
            calls: HashSet::new(),
            lengths: Vec::new(),
            open: HashMap::new(),
            in_format: 0,
            witnesses: &witnesses,
        };
        w.node(def.code());
        w.close_groups();
        let (bad, calls, lengths) = (w.bad, w.calls, w.lengths);
        match bad {
            None => {
                return Ok(Judged {
                    frame: ApartFrame { vars: cand, calls },
                    lengths,
                });
            }
            Some((v, why)) => {
                if required.contains(&v) || v == u16::MAX {
                    if trace_on() {
                        crate::loft_eprintln!(
                            "apart: {} twin declined — `{}`: {why}",
                            def.name(),
                            if v == u16::MAX { "?" } else { vars.name(v) }
                        );
                    }
                    return Err(why);
                }
                if trace_on() && mode == Mode::Host {
                    crate::loft_eprintln!(
                        "apart: {} keeps `{}` in the store — {why}",
                        def.name(),
                        vars.name(v)
                    );
                }
                cand.remove(&v);
            }
        }
    }
}

/// `@FR-R-Apart` instance 2 — the gate (module docs).  Empty under `LOFT_NO_APART`.
#[must_use]
#[expect(
    clippy::too_many_lines,
    reason = "WIP: the gate is split when the twin emission lands"
)]
pub fn apart_values(data: &Data, stores: &Stores) -> ApartValues {
    let mut out = ApartValues::default();
    if apart_disabled() {
        return out;
    }
    // Candidate types: every struct with an apart form.
    let mut types: HashMap<u16, ApartLayout> = HashMap::new();
    for d_nr in 0..data.definitions.len() as u32 {
        let def = data.def(d_nr);
        if def.def_type() != DefType::Struct {
            continue;
        }
        let tp = def.known_type();
        if tp == u16::MAX {
            continue;
        }
        if let Ok(layout) = layout_of(data, stores, d_nr, tp) {
            types.insert(tp, layout);
        }
    }
    if types.is_empty() {
        return out;
    }
    // The functions with a body that name an apart type at all.
    let mut fns: Vec<u32> = Vec::new();
    for d_nr in 0..data.definitions.len() as u32 {
        let def = data.def(d_nr);
        if def.def_type() != DefType::Function || matches!(def.code(), Value::Null) {
            continue;
        }
        let vars = def.variables();
        if (0..vars.count())
            .any(|v| record_of(data, vars.tp(v)).is_some_and(|t| types.contains_key(&t)))
        {
            fns.push(d_nr);
        }
    }
    // Twin candidates start optimistic: every function with an apart-type parameter or result.
    let mut twins: HashMap<u32, (Vec<usize>, bool)> = HashMap::new();
    for &d_nr in &fns {
        let shape = shape_of(data, d_nr, &types);
        if shape.blocked.is_none() && (shape.buffer.is_some() || !shape.params.is_empty()) {
            twins.insert(
                d_nr,
                (
                    shape.params.iter().map(|(i, _, _)| *i).collect(),
                    shape.buffer.is_some(),
                ),
            );
        }
    }
    let mut twin_frames: HashMap<u32, Judged> = HashMap::new();
    let mut host_frames: HashMap<u32, Judged> = HashMap::new();
    loop {
        let before = (twins.len(), types.len());
        twin_frames.clear();
        host_frames.clear();
        let snapshot = twins.clone();
        for &d_nr in &fns {
            if snapshot.contains_key(&d_nr) {
                match judge(data, d_nr, Mode::Twin, &types, &snapshot) {
                    Ok(j) => {
                        twin_frames.insert(d_nr, j);
                    }
                    Err(_) => {
                        twins.remove(&d_nr);
                    }
                }
            }
            if let Ok(j) = judge(data, d_nr, Mode::Host, &types, &snapshot)
                && !j.frame.vars.is_empty()
            {
                host_frames.insert(d_nr, j);
            }
        }
        // A twin no admitted frame calls still stands: a later round may call it.  But a
        // type whose constructions disagree on a length has no apart form anywhere.
        let mut seen: HashMap<(u16, i64), HashSet<u32>> = HashMap::new();
        for j in twin_frames.values().chain(host_frames.values()) {
            for (key, n) in &j.lengths {
                seen.entry(*key).or_default().insert(*n);
            }
        }
        let mut lost: HashSet<u16> = HashSet::new();
        for (tp, layout) in &mut types {
            for p in &mut layout.parts {
                if let ApartPart::Vector { off, n, .. } = p {
                    match seen.get(&(*tp, *off)) {
                        Some(ns) if ns.len() == 1 => *n = ns.iter().next().copied(),
                        Some(_) => {
                            if trace_on() {
                                crate::loft_eprintln!(
                                    "apart: {} declined — its constructions build different lengths",
                                    layout.name
                                );
                            }
                            lost.insert(*tp);
                        }
                        None => *n = None,
                    }
                }
            }
        }
        types.retain(|tp, _| !lost.contains(tp));
        if (twins.len(), types.len()) == before {
            break;
        }
    }
    // A type no construction counted has no `N`: nothing of it is apart.
    let unknown: HashSet<u16> = types
        .iter()
        .filter(|(_, l)| {
            l.parts
                .iter()
                .any(|p| matches!(p, ApartPart::Vector { n: None, .. }))
        })
        .map(|(tp, _)| *tp)
        .collect();
    let uses_unknown = |j: &Judged| j.frame.vars.values().any(|tp| unknown.contains(tp));
    for (d_nr, j) in twin_frames {
        if !uses_unknown(&j) {
            out.twins.insert(d_nr, j.frame);
        }
    }
    for (d_nr, j) in host_frames {
        if !uses_unknown(&j) {
            out.hosts.insert(d_nr, j.frame);
        }
    }
    types.retain(|tp, _| !unknown.contains(tp));
    out.types = types;
    if trace_on() {
        trace(data, &out);
    }
    out
}

fn trace(data: &Data, out: &ApartValues) {
    let mut lines: Vec<String> = Vec::new();
    for l in out.types.values() {
        let fields: Vec<String> = l
            .parts
            .iter()
            .map(|p| match p {
                ApartPart::Scalar { off, rt } => format!("@{off}:{rt}"),
                ApartPart::Vector { off, rt, n, .. } => {
                    format!(
                        "@{off}:[{rt}; {}]",
                        n.map_or("?".to_string(), |n| n.to_string())
                    )
                }
            })
            .collect();
        lines.push(format!("apart: type {} {}", l.name, fields.join(" ")));
    }
    let names = |d: u32, f: &ApartFrame| {
        let vars = data.def(d).variables();
        let mut v: Vec<&str> = f.vars.keys().map(|x| vars.name(*x)).collect();
        v.sort_unstable();
        v.join(", ")
    };
    for (d, f) in &out.twins {
        lines.push(format!(
            "apart: {} twin — {} ({} twin calls)",
            data.def(*d).name(),
            names(*d, f),
            f.calls.len()
        ));
    }
    for (d, f) in &out.hosts {
        lines.push(format!(
            "apart: {} host — {} ({} twin calls)",
            data.def(*d).name(),
            names(*d, f),
            f.calls.len()
        ));
    }
    lines.sort();
    for l in lines {
        crate::loft_eprintln!("{l}");
    }
}
