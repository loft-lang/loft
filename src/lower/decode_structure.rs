// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! `@FR-R-DecodeView`'s STRUCTURE clause (`@C141`), instance 1 — a decoded value held as its
//! POSITION: the arguments of the decoding call that answers it.  Nothing of it is built.
//!
//! `decode(bytes).value` is `read_value(bytes, 0, 0).value`, and a part of it is
//! `read_value(bytes, p, d).value` for the `(p, d)` the decoder handed the call that answered
//! it.  The frame is not written while a position lives (`@FR-R-DecodeView` (1), the region's
//! read-only walk), and the decoder answers from its arguments and the frame alone, so running
//! it again at a position takes the same path and answers the same value.  The plan is
//! decided here from the IR; `decode_structure_emit` writes it:
//!
//! - the PRODUCER `P` is a region function answering a record `D` — one field of the view
//!   enum `T` (the value) and scalar fields (`next`, `ok`) — from a frame parameter `B` and
//!   scalar parameters (the POSITION).  Its SHAPE TWIN `P__ps` is its body with every
//!   statement that only builds dropped: every test, loop and scalar stays, so its scalar
//!   fields are the plain form's, and each `return` answers the variant's TAG, its one number
//!   or text-span SLOT and the scalar fields.  Its WALK TWIN `P__psw<k>` runs the same body
//!   and hands each element appended to the vector field `k` to a closure — the element's
//!   parts as positions — stopping when the closure answers;
//! - a CONSUMER is every other function of the root's chain: `decode`, the lookups, the root
//!   itself.  Its values of `T`, of `D` and of the element records are held as positions
//!   (`(0, p…)`) or unit literals (`(tag, 0…)`), and a closed list of reads is rewritten: the
//!   enum tag and a text field through the shape twin, a scalar field of `D` from the tuple,
//!   a walk of a vector field through the walk twin;
//! - (6) READ ONCE, in instance 1: no consumer is recursive and no position made outside a
//!   loop is read inside one — each part is re-decoded a bounded number of times;
//! - anything else that mentions a decoded value declines the root, which keeps instance 1's
//!   built form.
//!
//! `LOFT_NO_DECODE_STRUCTURE=1` builds every decoded value; `LOFT_TRACE_DECODE_STRUCTURE=1`
//! names each producer, consumer and root admitted or declined, with the reason.
use super::decode_view::{DvPlan, int_of, text_run};
use crate::data::{Block, Data, DefType, Type, Value};
use crate::database::{Parts, Stores};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};

/// `LOFT_NO_DECODE_STRUCTURE=1` — every decoded value built (instance 1 of
/// `@FR-R-DecodeView` only): the before-half of every A/B and the first bisect step for a
/// native-only wrong answer out of a decoder.
#[must_use]
pub fn disabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_NO_DECODE_STRUCTURE").is_ok_and(|v| v != "0"))
}

fn trace_on() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_TRACE_DECODE_STRUCTURE").is_ok_and(|v| v != "0"))
}

fn trace(msg: impl FnOnce() -> String) {
    if trace_on() {
        eprintln!("[decode-structure] {}", msg());
    }
}

pub(crate) fn op_name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

pub(crate) fn addr(v: &Value) -> usize {
    std::ptr::from_ref(v) as usize
}

/// Scalar operators a twin keeps: each answers from its operands alone, reports no fault and
/// writes nothing — integer arithmetic and comparison (a fault answers null), the boolean
/// connectives, and the null conversions.
pub(crate) fn scalar_op(name: &str) -> bool {
    const SINGLE: &[&str] = &[
        "OpNot",
        "OpAnd",
        "OpOr",
        "OpEqBool",
        "OpNeBool",
        "OpConvBoolFromInt",
        "OpConvBoolFromLong",
        "OpConvIntFromNull",
        "OpConvLongFromNull",
        "OpConvBoolFromNull",
        "OpConvLongFromInt",
        "OpConvIntFromLong",
    ];
    if SINGLE.contains(&name) {
        return true;
    }
    let Some(rest) = name.strip_prefix("Op") else {
        return false;
    };
    let rest = rest.strip_suffix("Nullable").unwrap_or(rest);
    let Some(op) = rest
        .strip_suffix("Int")
        .or_else(|| rest.strip_suffix("Long"))
    else {
        return false;
    };
    matches!(
        op,
        "Add"
            | "Min"
            | "Mul"
            | "Div"
            | "Rem"
            | "Eq"
            | "Ne"
            | "Lt"
            | "Le"
            | "Gt"
            | "Ge"
            | "LogicalAnd"
            | "LogicalOr"
            | "ExclusiveOr"
            | "ShiftLeft"
            | "ShiftRight"
    )
}

/// Operators a dropped statement may contain: each builds, moves, reads or releases a value
/// of the decoded tree, writes no store a kept value reaches and reports no fault.
const BUILD_OPS: &[&str] = &[
    "OpDatabase",
    "OpDatabaseNP",
    "OpPlaceRecord",
    "OpNewRecord",
    "OpFinishRecord",
    "OpPreAllocVector",
    "OpPushByte",
    "OpSliceVector",
    "OpSetInt",
    "OpSetInt4",
    "OpSetBoolean",
    "OpSetEnum",
    "OpSetText",
    "OpCopyRecord",
    "OpMoveField",
    "OpMoveVector",
    "OpClear",
    "OpFreeRef",
    "OpFreeRefIfDistinct",
    "OpFreeRecordIn",
    "OpFreeText",
    "OpGetField",
    "OpGetVectorNullable",
    "OpGetByte",
    "OpGetInt",
    "OpGetBoolean",
    "OpLengthVector",
    "OpRefIsNull",
    "OpConvBoolFromRef",
    "OpDistinctStore",
    "n_text_from_byte_range",
    "n_text_from_bytes",
    "t_6vector_len",
];

/// One scalar field of the producer's record, answered in the twin's tuple after the tag and
/// the two slot words.
#[derive(Clone, Debug)]
pub struct PsScalar {
    pub off: i64,
    /// `i64` for an integer, `u8` for a boolean's storage.
    pub rust: &'static str,
}

/// One element appended to a walked vector: its record's parts, each the value of a
/// recursive call (by the call-result variable whose arguments are its position).
#[derive(Clone, Debug)]
pub struct Yield {
    /// The walked local.
    pub local: u16,
    /// `(field offset in the element, call-result variable)`; one part at offset `-1` when
    /// the element IS the enum.
    pub parts: Vec<(i64, u16)>,
}

/// The producer: its shape and walk twins' plan.
#[derive(Clone, Debug, Default)]
pub struct Producer {
    pub frame: u16,
    /// The position: the scalar parameters, in order, with their Rust types.
    pub params: Vec<(u16, &'static str)>,
    pub retbuf: u16,
    /// The value field's offset in the record.
    pub vo: i64,
    pub t_tp: u16,
    pub d_tp: u16,
    pub scalars: Vec<PsScalar>,
    /// The call-result variables (`kd = P(…)`).
    pub calls: HashSet<u16>,
    /// Statements dropped from both twins (IR addresses).
    pub drop: HashSet<usize>,
    /// `OpFinishRecord` statements (IR addresses) → the element they append.
    pub yields: HashMap<usize, Yield>,
    /// `(field offset in its variant, vector store type)` → the locals the value's vector
    /// field is moved from at a `return`.
    pub walked: HashMap<(i64, u16), HashSet<u16>>,
    /// The element type of each walked vector store type, and whether it is the enum itself.
    pub elem: HashMap<u16, (u16, bool)>,
}

impl Producer {
    /// The shape tuple's Rust type: tag, two slot words, the scalar fields.
    #[must_use]
    pub fn shape_tuple(&self) -> String {
        let mut parts = vec!["u8", "i64", "i64"];
        parts.extend(self.scalars.iter().map(|s| s.rust));
        format!("({})", parts.join(", "))
    }

    /// A position's Rust type: the kind byte — `0` a position whose shape is not known, `1`
    /// one whose shape is, `2` a unit literal — then the position's parameters, then the
    /// shape's tag and two slot words.  A position decoded where it was made carries the
    /// shape that decode answered, so no reader decodes its head again.
    #[must_use]
    pub fn rep_tuple(&self) -> String {
        let mut parts = vec!["u8"];
        parts.extend(self.params.iter().map(|p| p.1));
        parts.extend(["u8", "i64", "i64"]);
        format!("({})", parts.join(", "))
    }

    /// A unit literal's position tuple.
    #[must_use]
    pub fn unit_rep(&self, tag: u8) -> String {
        format!(
            "(2u8{}, {tag}u8, 0_i64, 0_i64)",
            ", Default::default()".repeat(self.params.len())
        )
    }

    /// The position `args` (an expression whose fields `.0…` are the parameters) with the
    /// shape `shape` (whose `.0`, `.1`, `.2` are the tag and slots) known.
    #[must_use]
    pub fn known_rep(&self, args: &str, shape: &str) -> String {
        use std::fmt::Write as _;
        let mut out = "(1u8".to_string();
        for i in 0..self.params.len() {
            let _ = write!(out, ", {args}.{i}");
        }
        let _ = write!(out, ", {shape}.0, {shape}.1, {shape}.2)");
        out
    }

    /// Index of the scalar field at `off` in the shape tuple.
    #[must_use]
    pub fn scalar_index(&self, off: i64) -> Option<usize> {
        self.scalars.iter().position(|s| s.off == off)
    }
}

/// What a `return` of the producer answers, read off its `Object` block.
pub struct RetForm<'a> {
    pub tag: u8,
    pub slot: Slot<'a>,
    /// `(offset, value)` of each scalar field.
    pub scalars: Vec<(i64, &'a Value)>,
}

pub enum Slot<'a> {
    None,
    /// A number or boolean field.
    Num(&'a Value),
    /// A text run over the frame: `(guard, lo, hi)`.
    Text(&'a Value, &'a Value, &'a Value),
    /// A field only the built value answers (a byte string, a text that is no run).
    Built,
}

fn scalar_type(tp: &Type) -> Option<&'static str> {
    match tp.base() {
        Type::Integer(_) | Type::Character => Some("i64"),
        Type::Boolean => Some("u8"),
        _ => None,
    }
}

fn known(data: &Data, d: u32) -> Option<u16> {
    if (d as usize) >= data.definitions.len() {
        return None;
    }
    let kt = data.def(d).known_type;
    (kt != u16::MAX).then_some(kt)
}

/// The record store type a loft type names.
pub(crate) fn record_tp(data: &Data, tp: &Type) -> Option<u16> {
    match tp.base() {
        Type::Reference(d, _) | Type::Enum(d, true, _) => known(data, *d),
        Type::RefVar(inner) => record_tp(data, inner),
        _ => None,
    }
}

fn fields(stores: &Stores, tp: u16) -> Option<&Vec<crate::database::Field>> {
    match stores.types.get(tp as usize).map(|t| &t.parts) {
        Some(Parts::Struct(f) | Parts::EnumValue(_, f)) => Some(f),
        _ => None,
    }
}

fn enum_tag(v: &Value) -> Option<u8> {
    match v.unspan() {
        Value::Enum(t, _) => Some(*t),
        _ => None,
    }
}

/// `{ OpDatabase(x, tp); OpSetEnum(x, 0, tag); x }` — a unit variant literal.  Answers its tag.
pub(crate) fn unit_literal(data: &Data, v: &Value) -> Option<u8> {
    let Value::Block(bl) = v.unspan() else {
        return None;
    };
    let ops: Vec<&Value> = bl
        .operators
        .iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    let [db, set, last] = ops.as_slice() else {
        return None;
    };
    let (Value::Call(d1, a1), Value::Call(d2, a2), Value::Var(x)) =
        (db.unspan(), set.unspan(), last.unspan())
    else {
        return None;
    };
    if !matches!(op_name(data, *d1), "OpDatabase" | "OpDatabaseNP")
        || op_name(data, *d2) != "OpSetEnum"
        || !matches!(a1.first().map(Value::unspan), Some(Value::Var(y)) if y == x)
        || !matches!(a2.first().map(Value::unspan), Some(Value::Var(y)) if y == x)
        || a2.len() != 3
        || int_of(&a2[1]) != Some(0)
    {
        return None;
    }
    enum_tag(&a2[2])
}

/// `{#EnumUnitLit OpDatabase(x, tp); OpSetEnum(x, 0, tag); frees…; return x }` — a unit
/// literal the scope pass returned from inside its own block.  Answers its tag.
pub(crate) fn unit_return(data: &Data, bl: &Block) -> Option<u8> {
    let ops: Vec<&Value> = bl
        .operators
        .iter()
        .map(Value::unspan)
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect();
    let [
        Value::Call(d1, a1),
        Value::Call(d2, a2),
        middle @ ..,
        Value::Return(r),
    ] = ops.as_slice()
    else {
        return None;
    };
    let Value::Var(x) = r.unspan() else {
        return None;
    };
    if !matches!(op_name(data, *d1), "OpDatabase" | "OpDatabaseNP")
        || op_name(data, *d2) != "OpSetEnum"
        || !matches!(a1.first().map(Value::unspan), Some(Value::Var(y)) if y == x)
        || !matches!(a2.first().map(Value::unspan), Some(Value::Var(y)) if y == x)
        || a2.len() != 3
        || int_of(&a2[1]) != Some(0)
    {
        return None;
    }
    let frees_only = middle.iter().all(|o| {
        matches!(o, Value::Call(g, _) if matches!(op_name(data, *g), "OpFreeRef" | "OpFreeRefIfDistinct" | "OpFreeText"))
    });
    if !frees_only {
        return None;
    }
    enum_tag(&a2[2])
}

fn is_var(v: &Value, x: u16) -> bool {
    matches!(v.unspan(), Value::Var(y) if *y == x)
}

/// `OpGetField(Var(rec), off, _)` — answers `off`.
fn field_of(data: &Data, v: &Value, rec: u16) -> Option<i64> {
    match v.unspan() {
        Value::Call(d, a)
            if op_name(data, *d) == "OpGetField" && a.len() == 3 && is_var(&a[0], rec) =>
        {
            int_of(&a[1])
        }
        _ => None,
    }
}

/// Every variable `v` mentions, reads and writes alike.
fn mentions(v: &Value, out: &mut HashMap<u16, usize>) {
    v.any_node(&mut |n| {
        match n {
            Value::Var(x) | Value::Set(x, _) => *out.entry(*x).or_default() += 1,
            _ => {}
        }
        false
    });
}

fn mentions_var(v: &Value, x: u16) -> bool {
    v.any_node(&mut |n| matches!(n, Value::Var(y) | Value::Set(y, _) if *y == x))
}

/// A frame read a twin keeps.
enum FrameRead<'v> {
    /// `bytes[index]`.
    Byte(&'v Value),
    /// `len(bytes)`.
    Length,
}

/// The producer's classification of its variables.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Frame,
    Scalar,
    Call,
    Build,
}

struct PWalk<'a> {
    data: &'a Data,
    d: u32,
    p: Producer,
    kinds: HashMap<u16, Kind>,
    /// Mentions of each variable in the whole body.
    total: HashMap<u16, usize>,
    /// `kv` → `(call variable, sequence at the bind)`: `kv = OpGetField(kd, vo)`.
    binds: HashMap<u16, (u16, usize)>,
    /// Sequence number of each call variable's latest assignment.
    set_seq: HashMap<u16, usize>,
    seq: usize,
    /// The element record each walk mints (`_elm`) → its parts so far.
    elms: HashMap<u16, Vec<(i64, u16)>>,
    why: Option<String>,
}

impl PWalk<'_> {
    fn fail(&mut self, why: impl FnOnce() -> String) {
        if self.why.is_none() {
            self.why = Some(why());
        }
    }

    fn name(&self, v: u16) -> String {
        self.data.def(self.d).variables().name(v).to_string()
    }

    fn kind(&self, v: u16) -> Kind {
        self.kinds.get(&v).copied().unwrap_or(Kind::Build)
    }

    /// A read of the frame a twin keeps: a byte, its length.
    fn frame_read<'v>(&self, node: &'v Value) -> Option<FrameRead<'v>> {
        let Value::Call(op, args) = node.unspan() else {
            return None;
        };
        match (op_name(self.data, *op), args.as_slice()) {
            ("OpGetByte", [inner, field, size])
                if int_of(field) == Some(0) && int_of(size) == Some(0) =>
            {
                let Value::Call(get, get_args) = inner.unspan() else {
                    return None;
                };
                if op_name(self.data, *get) != "OpGetVectorNullable" || get_args.len() != 3 {
                    return None;
                }
                (is_var(&get_args[0], self.p.frame) && int_of(&get_args[1]) == Some(1))
                    .then_some(FrameRead::Byte(&get_args[2]))
            }
            ("OpLengthVector" | "t_6vector_len", [frame]) if is_var(frame, self.p.frame) => {
                Some(FrameRead::Length)
            }
            _ => None,
        }
    }

    /// An expression a twin keeps: scalars, frame reads, scalar fields of a call's result.
    fn scalar(&mut self, v: &Value) {
        if self.why.is_some() {
            return;
        }
        if let Some(read) = self.frame_read(v) {
            if let FrameRead::Byte(index) = read {
                self.scalar(index);
            }
            return;
        }
        match v.unspan() {
            Value::Int(_)
            | Value::Long(_)
            | Value::Boolean(_)
            | Value::Float(_)
            | Value::Single(_)
            | Value::Null
            | Value::Line(_)
            | Value::Break(_)
            | Value::Continue(_) => {}
            Value::Var(x) => {
                if self.kind(*x) != Kind::Scalar {
                    let n = self.name(*x);
                    self.fail(|| format!("a kept expression reads `{n}`"));
                }
            }
            Value::Set(x, rhs) => {
                if self.kind(*x) == Kind::Scalar {
                    self.scalar(rhs);
                } else {
                    let n = self.name(*x);
                    self.fail(|| format!("a kept expression writes `{n}`"));
                }
            }
            Value::Call(d, a) => {
                let name = op_name(self.data, *d);
                if matches!(name, "OpGetInt" | "OpGetBoolean")
                    && let [Value::Var(cr), off] = a.as_slice()
                    && self.kind(*cr) == Kind::Call
                    && int_of(off).is_some_and(|o| self.p.scalar_index(o).is_some())
                {
                    return;
                }
                if !scalar_op(name) {
                    let n = name.to_string();
                    self.fail(|| format!("a kept expression calls `{n}`"));
                    return;
                }
                for x in a {
                    self.scalar(x);
                }
            }
            Value::Block(bl) => {
                for o in &bl.operators {
                    self.scalar(o);
                }
            }
            Value::If(c, t, e) => {
                self.scalar(c);
                self.scalar(t);
                self.scalar(e);
            }
            _ => self.fail(|| "a kept expression of an unknown form".into()),
        }
    }

    /// A statement whose every effect is building: dropped from both twins.  The scalars it
    /// writes are its own (no mention of them outside it).
    fn build_only(&self, v: &Value) -> bool {
        let mut inner: HashMap<u16, usize> = HashMap::default();
        mentions(v, &mut inner);
        for (x, n) in &inner {
            match self.kind(*x) {
                Kind::Frame | Kind::Build => {}
                Kind::Call => {
                    // Read only, never assigned here.
                    if v.any_node(&mut |m| matches!(m, Value::Set(y, _) if y == x)) {
                        return false;
                    }
                }
                Kind::Scalar => {
                    let written = v.any_node(&mut |m| matches!(m, Value::Set(y, _) if y == x));
                    if written && self.total.get(x).copied().unwrap_or(0) != *n {
                        return false;
                    }
                }
            }
        }
        !v.any_node(&mut |m| match m {
            Value::Return(_) | Value::Yield(_) | Value::CallRef(..) => true,
            // A `break` or `continue` of a loop of its own is part of it; one that leaves the
            // statement is a kept effect.
            Value::Break(_) | Value::Continue(_) => {
                !v.any_node(&mut |l| matches!(l, Value::Loop(_)))
            }
            Value::Call(d, _) => {
                let name = op_name(self.data, *d);
                !BUILD_OPS.contains(&name) && !scalar_op(name)
            }
            _ => false,
        })
    }

    fn stmt(&mut self, v: &Value) {
        if self.why.is_some() {
            return;
        }
        self.seq += 1;
        match v.unspan() {
            Value::Line(_) | Value::Null => {}
            Value::Block(bl) if returns_buffer(bl, self.p.retbuf) => {
                if parse_return(self.data, &self.p, bl).is_none() {
                    self.fail(|| {
                        "a return whose record is not a variant, a unit literal and scalar fields"
                            .into()
                    });
                }
                self.walked_returns(bl);
            }
            Value::Block(bl) => {
                for o in &bl.operators {
                    self.stmt(o);
                }
            }
            Value::Loop(bl) => {
                for o in &bl.operators {
                    self.stmt(o);
                }
            }
            Value::Insert(items) => {
                if self.build_only(v) {
                    self.p.drop.insert(addr(v));
                } else {
                    for o in items {
                        self.stmt(o);
                    }
                }
            }
            Value::If(c, t, e) => {
                if self.build_only(v) {
                    self.p.drop.insert(addr(v));
                    return;
                }
                self.scalar(c);
                self.stmt(t);
                self.stmt(e);
            }
            Value::Set(x, rhs) => self.set(v, *x, rhs),
            Value::Break(_) | Value::Continue(_) => {}
            Value::Call(..) => {
                self.element_part(v);
                if self.build_only(v) {
                    self.p.drop.insert(addr(v));
                } else {
                    self.fail(|| "a statement that is neither kept nor only building".into());
                }
            }
            Value::Var(x) if self.kind(*x) == Kind::Scalar => {}
            _ => self.fail(|| "a statement of an unknown form".into()),
        }
    }

    fn set(&mut self, v: &Value, x: u16, rhs: &Value) {
        match self.kind(x) {
            Kind::Scalar => {
                if self.build_only(v) && self.total.get(&x).copied().unwrap_or(0) == 1 {
                    self.p.drop.insert(addr(v));
                } else {
                    self.scalar(rhs);
                }
            }
            Kind::Call => {
                self.seq += 1;
                self.set_seq.insert(x, self.seq);
                match rhs.unspan() {
                    Value::Null => {
                        self.p.drop.insert(addr(v));
                    }
                    Value::Call(d, args) if *d == self.d => self.call_args(args),
                    _ => {
                        let n = self.name(x);
                        self.fail(|| {
                            format!("`{n}` holds something other than the decoder's result")
                        });
                    }
                }
            }
            Kind::Build => {
                if let Some(cr) = field_of_call(self, rhs) {
                    self.binds.insert(x, (cr, self.seq));
                }
                if let Value::Call(d, a) = rhs.unspan()
                    && op_name(self.data, *d) == "OpNewRecord"
                {
                    let _ = a;
                    self.elms.insert(x, Vec::new());
                }
                if self.build_only(v) {
                    self.p.drop.insert(addr(v));
                } else {
                    let n = self.name(x);
                    self.fail(|| format!("`{n}` is built from a kept value"));
                }
            }
            Kind::Frame => self.fail(|| "the frame is assigned".into()),
        }
    }

    fn call_args(&mut self, args: &[Value]) {
        let def = self.data.def(self.d);
        for (i, a) in args.iter().enumerate() {
            let Some(attr) = def.attributes().get(i) else {
                self.fail(|| "a recursive call with more arguments than parameters".into());
                return;
            };
            let pv = def.variables().var(&attr.name);
            if pv == self.p.frame {
                if !is_var(a, self.p.frame) {
                    self.fail(|| "a recursive call hands on another frame".into());
                }
            } else if pv == self.p.retbuf {
                if !matches!(a.unspan(), Value::Var(b) if self.kind(*b) == Kind::Build) {
                    self.fail(|| "a recursive call's buffer is not a buffer".into());
                }
            } else {
                self.scalar(a);
            }
        }
    }

    /// `OpMoveField(src, OpGetField(elm, off, T) | elm, T)` and `OpFinishRecord(L, elm, …)`:
    /// the element parts a walk hands over.
    fn element_part(&mut self, stmt: &Value) {
        let Value::Call(op, args) = stmt.unspan() else {
            return;
        };
        match (op_name(self.data, *op), args.as_slice()) {
            ("OpMoveField", [src, dst, _]) => {
                let source = match src.unspan() {
                    Value::Var(kv) => self.binds.get(kv).copied(),
                    _ => field_of_call(self, src).map(|call| (call, self.seq)),
                };
                let target = match dst.unspan() {
                    Value::Var(elm) => Some((*elm, -1)),
                    Value::Call(get, get_args)
                        if op_name(self.data, *get) == "OpGetField"
                            && let Some(Value::Var(elm)) = get_args.first().map(Value::unspan) =>
                    {
                        int_of(&get_args[1]).map(|off| (*elm, off))
                    }
                    _ => None,
                };
                if let (Some((call, bound_at)), Some((elm, off))) = (source, target)
                    && let Some(parts) = self.elms.get_mut(&elm)
                {
                    if self.set_seq.get(&call).copied().unwrap_or(0) > bound_at {
                        self.why.get_or_insert_with(|| {
                            "a part's call is assigned again before it is appended".into()
                        });
                    }
                    parts.push((off, call));
                }
            }
            ("OpFinishRecord", [Value::Var(local), elm, ..]) => {
                if let Value::Var(elm) = elm.unspan()
                    && let Some(parts) = self.elms.remove(elm)
                {
                    self.p.yields.insert(
                        addr(stmt),
                        Yield {
                            local: *local,
                            parts,
                        },
                    );
                }
            }
            _ => {}
        }
    }

    /// The vector fields a return moves a local into: the locals a walk of that field reads.
    fn walked_returns(&mut self, bl: &Block) {
        for stmt in &bl.operators {
            if let Value::Call(op, args) = stmt.unspan()
                && op_name(self.data, *op) == "OpMoveVector"
                && let [dst, Value::Var(local), _] = args.as_slice()
                && let Value::Call(get, get_args) = dst.unspan()
                && op_name(self.data, *get) == "OpGetField"
                && get_args.len() == 3
                && let (Some(off), Some(vtp)) = (int_of(&get_args[1]), int_of(&get_args[2]))
                && let Ok(vtp) = u16::try_from(vtp)
                && self.p.elem.contains_key(&vtp)
            {
                self.p.walked.entry((off, vtp)).or_default().insert(*local);
            }
        }
    }
}

/// `kd`'s value: `OpGetField(Var(kd), vo, _)` of a call variable.
fn field_of_call(walk: &PWalk<'_>, node: &Value) -> Option<u16> {
    let Value::Call(op, args) = node.unspan() else {
        return None;
    };
    if op_name(walk.data, *op) != "OpGetField" || args.len() != 3 {
        return None;
    }
    let Value::Var(call) = args[0].unspan() else {
        return None;
    };
    (walk.kind(*call) == Kind::Call && int_of(&args[1]) == Some(walk.p.vo)).then_some(*call)
}

/// An `Object` block that answers the producer's buffer: its last statement returns it.
pub(crate) fn returns_buffer(bl: &Block, retbuf: u16) -> bool {
    matches!(bl.operators.last().map(Value::unspan), Some(Value::Return(r)) if is_var(r, retbuf))
}

/// The value field of the buffer: `OpGetField(Var(retbuf), vo, _)`.
fn value_field(data: &Data, p: &Producer, v: &Value) -> bool {
    field_of(data, v, p.retbuf) == Some(p.vo)
}

/// Read a producer `return` off its `Object` block.  `None` when the block does anything the
/// twins cannot answer.
pub(crate) fn parse_return<'a>(data: &Data, prod: &Producer, bl: &'a Block) -> Option<RetForm<'a>> {
    let mut tag = None;
    let mut slot = Slot::None;
    let mut slots = 0;
    let mut scalars = Vec::new();
    for stmt in &bl.operators {
        let args = match stmt.unspan() {
            Value::Line(_) | Value::Null | Value::Return(_) | Value::If(..) | Value::Insert(_) => {
                continue;
            }
            Value::Call(op, args) => (op_name(data, *op), args.as_slice()),
            _ => return None,
        };
        match args {
            ("OpCopyRecord", [src, dst, _]) if value_field(data, prod, dst) => {
                tag = Some(unit_literal(data, src)?);
            }
            ("OpSetEnum", [dst, zero, variant])
                if value_field(data, prod, dst) && int_of(zero) == Some(0) =>
            {
                tag = Some(enum_tag(variant)?);
            }
            ("OpSetInt" | "OpSetBoolean", [dst, off, val]) if value_field(data, prod, dst) => {
                int_of(off)?;
                slots += 1;
                slot = Slot::Num(val);
            }
            ("OpSetText", [dst, _, val]) if value_field(data, prod, dst) => {
                slots += 1;
                slot = match text_run(data, val) {
                    Some(run) if run.frame == prod.frame => Slot::Text(run.guard, run.lo, run.hi),
                    _ => Slot::Built,
                };
            }
            ("OpMoveVector", [dst, _, _]) => {
                // A byte string is a slot only the built value answers; a vector of the enum
                // or its records is a walk.
                let Value::Call(get, get_args) = dst.unspan() else {
                    return None;
                };
                if op_name(data, *get) != "OpGetField" || !value_field(data, prod, &get_args[0]) {
                    return None;
                }
                let vtp = u16::try_from(int_of(&get_args[2])?).ok()?;
                if !prod.elem.contains_key(&vtp) {
                    slots += 1;
                    slot = Slot::Built;
                }
            }
            ("OpSetInt4", [dst, _, zero]) if int_of(zero) == Some(0) => {
                if !is_var(dst, prod.retbuf) && !value_field(data, prod, dst) {
                    return None;
                }
            }
            ("OpSetInt" | "OpSetBoolean", [dst, off, val]) if is_var(dst, prod.retbuf) => {
                let off = int_of(off)?;
                prod.scalar_index(off)?;
                scalars.push((off, val));
            }
            ("OpFreeText" | "OpFreeRef" | "OpFreeRefIfDistinct" | "OpFreeRecordIn", _) => {}
            _ => return None,
        }
    }
    if slots > 1 || scalars.len() != prod.scalars.len() {
        return None;
    }
    Some(RetForm {
        tag: tag?,
        slot,
        scalars,
    })
}

/// The producer's parameters: one frame, the rest numbers (the position).
fn producer_params(
    data: &Data,
    dv: &DvPlan,
    def_nr: u32,
    rb: usize,
    prod: &mut Producer,
) -> Result<(), String> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let frames = dv.bparams.get(&def_nr).cloned().unwrap_or_default();
    let mut frame = None;
    for (i, attr) in def.attributes().iter().enumerate() {
        if i == rb {
            continue;
        }
        let pv = vars.var(&attr.name);
        if frames.contains(&pv) {
            if frame.replace(pv).is_some() {
                return Err("two frame parameters".into());
            }
        } else if let Some(rust) = scalar_type(&attr.typedef) {
            prod.params.push((pv, rust));
        } else {
            return Err(format!(
                "parameter `{}` is no frame and no number",
                attr.name
            ));
        }
    }
    prod.frame = frame.ok_or("no frame parameter")?;
    Ok(())
}

/// The record: one field of a view enum, the rest scalars; and the element types of the
/// enum's vector fields.
fn producer_layout(stores: &Stores, dv: &DvPlan, prod: &mut Producer) -> Result<(), String> {
    let Some(record) = fields(stores, prod.d_tp) else {
        return Err("its result is no struct".into());
    };
    for field in record {
        if dv.types.contains_key(&field.content) {
            if prod.t_tp != 0 {
                return Err("two decoded fields".into());
            }
            prod.t_tp = field.content;
            prod.vo = i64::from(field.position);
        } else {
            let rust = match stores.types.get(field.content as usize).map(|t| &t.parts) {
                Some(Parts::Base) if stores.name("integer") == field.content => "i64",
                Some(Parts::Base) if stores.name("boolean") == field.content => "u8",
                _ => return Err(format!("field `{}` is no number", field.name)),
            };
            prod.scalars.push(PsScalar {
                off: i64::from(field.position),
                rust,
            });
        }
    }
    if prod.t_tp == 0 {
        return Err("no decoded field".into());
    }
    prod.scalars.sort_by_key(|sc| sc.off);
    if let Some(Parts::Enum(variants)) = stores.types.get(prod.t_tp as usize).map(|t| &t.parts) {
        for (vtp, _) in variants {
            for field in fields(stores, *vtp).into_iter().flatten() {
                let Some(Parts::Vector(elem)) =
                    stores.types.get(field.content as usize).map(|t| &t.parts)
                else {
                    continue;
                };
                if *elem == prod.t_tp {
                    prod.elem.insert(field.content, (*elem, true));
                } else if fields(stores, *elem).is_some_and(|ef| {
                    !ef.is_empty() && ef.iter().all(|part| part.content == prod.t_tp)
                }) {
                    prod.elem.insert(field.content, (*elem, false));
                }
            }
        }
    }
    Ok(())
}

/// Each variable's class: the frame, a scalar, a recursive call's result, or a value of the
/// tree being built.
fn producer_kinds(data: &Data, def_nr: u32, prod: &Producer) -> HashMap<u16, Kind> {
    let def = data.def(def_nr);
    let vars = def.variables();
    let mut kinds: HashMap<u16, Kind> = HashMap::default();
    for v in 0..vars.count() {
        let tp = vars.tp(v);
        let kind = if v == prod.frame {
            Kind::Frame
        } else if scalar_type(tp).is_some() {
            Kind::Scalar
        } else if v != prod.retbuf
            && record_tp(data, tp) == Some(prod.d_tp)
            && !vars.is_argument(v)
            && def.code().any_node(&mut |n| {
                matches!(n, Value::Set(x, rhs) if *x == v
                    && matches!(rhs.unspan(), Value::Call(op, _) if *op == def_nr))
            })
        {
            Kind::Call
        } else {
            Kind::Build
        };
        kinds.insert(v, kind);
    }
    for (pv, _) in &prod.params {
        kinds.insert(*pv, Kind::Scalar);
    }
    kinds
}

/// A walked local is only appended to, and only moved into its field: the elements a walk
/// hands over are the vector's, in order.
fn appended_only(data: &Data, def_nr: u32, prod: &Producer) -> Result<(), String> {
    let def = data.def(def_nr);
    for locals in prod.walked.values() {
        for local in locals {
            let mut only = true;
            def.code().any_node(&mut |n| {
                if let Value::Call(op, args) = n
                    && args.iter().any(|arg| is_var(arg, *local))
                    && !matches!(
                        op_name(data, *op),
                        "OpPreAllocVector"
                            | "OpNewRecord"
                            | "OpFinishRecord"
                            | "OpMoveVector"
                            | "OpPlaceRecord"
                            | "OpFreeRef"
                            | "OpFreeRecordIn"
                    )
                {
                    only = false;
                }
                false
            });
            if !only {
                return Err(format!(
                    "`{}` is changed other than by an append",
                    def.variables().name(*local)
                ));
            }
        }
    }
    Ok(())
}

/// Admit `def_nr` as a producer.
fn producer(data: &Data, stores: &Stores, dv: &DvPlan, def_nr: u32) -> Result<Producer, String> {
    let def = data.def(def_nr);
    let Some(rb) = crate::opt::attrs::ret_buffer_attr(def) else {
        return Err("answers no record through a buffer".into());
    };
    let mut prod = Producer {
        retbuf: def.variables().var(&def.attributes()[rb].name),
        d_tp: record_tp(data, def.returned()).ok_or("its result names no record")?,
        ..Producer::default()
    };
    producer_params(data, dv, def_nr, rb, &mut prod)?;
    producer_layout(stores, dv, &mut prod)?;
    let kinds = producer_kinds(data, def_nr, &prod);
    prod.calls = kinds
        .iter()
        .filter(|(_, kind)| **kind == Kind::Call)
        .map(|(v, _)| *v)
        .collect();
    let mut total = HashMap::default();
    mentions(def.code(), &mut total);
    let mut walk = PWalk {
        data,
        d: def_nr,
        p: prod,
        kinds,
        total,
        binds: HashMap::default(),
        set_seq: HashMap::default(),
        seq: 0,
        elms: HashMap::default(),
        why: None,
    };
    walk.stmt(def.code());
    if let Some(why) = walk.why {
        return Err(why);
    }
    appended_only(data, def_nr, &walk.p)?;
    Ok(walk.p)
}

/// The whole program's plan.
#[derive(Default)]
pub struct PsPlan {
    pub producers: HashMap<u32, Producer>,
    pub consumers: HashMap<u32, super::decode_structure_scan::Consumer>,
    /// The roots emitted in their positional form (also consumers).
    pub roots: HashSet<u32>,
    /// `(producer, field offset, vector type)` → the walk twin's index.
    pub walks: HashMap<(u32, i64, u16), usize>,
}

/// Decide the plan from the decode-view plan's roots and region.
#[must_use]
pub fn plan(data: &Data, stores: &Stores, dv: &DvPlan) -> PsPlan {
    let mut plan = PsPlan::default();
    if disabled() || dv.roots.is_empty() {
        return plan;
    }
    let mut region: Vec<u32> = dv.region.iter().copied().collect();
    region.sort_unstable();
    for d in &region {
        if !matches!(data.def(*d).def_type(), DefType::Function) {
            continue;
        }
        // A producer calls itself: a decoder is recursive.
        let recursive = data
            .def(*d)
            .code()
            .any_node(&mut |n| matches!(n, Value::Call(g, _) if g == d));
        if !recursive {
            continue;
        }
        match producer(data, stores, dv, *d) {
            Ok(p) => {
                trace(|| {
                    format!(
                        "{}: a producer, position {}, {} dropped, walks {:?}",
                        data.def(*d).name(),
                        p.rep_tuple(),
                        p.drop.len(),
                        p.walked.keys().collect::<Vec<_>>()
                    )
                });
                plan.producers.insert(*d, p);
            }
            Err(why) => trace(|| format!("{}: no producer: {why}", data.def(*d).name())),
        }
    }
    if plan.producers.is_empty() {
        return plan;
    }
    super::decode_structure_scan::roots(data, stores, dv, &mut plan);
    plan
}

pub(crate) fn mentions_any(v: &Value, xs: &HashSet<u16>) -> bool {
    xs.iter().any(|x| mentions_var(v, *x))
}
