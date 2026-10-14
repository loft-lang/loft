// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! `@FR-R-DecodeView` (`@C141`), instance 1 — a text or byte-string field of a decoded enum
//! value holds a SPAN of the byte frame it was decoded from, not an owned copy, inside the
//! region of functions that decode the frame and read the result.
//!
//! The plan is decided here, from the IR alone, before anything is emitted:
//!
//! - a VIEW TYPE is an enum whose record leaves its last eight bytes free in every variant
//!   with a `text` or `vector<u8>` field.  Those eight bytes are the span word: the frame's
//!   bytes `[lo, lo + n)`, marked by the sign bit, or `0` for the owned form.  A span's own
//!   field holds nothing (a null text, an empty vector), so a release frees nothing for it
//!   and a copy carries the word with the record's bytes;
//! - a ROOT is a function with a byte-vector parameter `B` that it hands, with nothing but
//!   reads of it, to a call answering a value of a view type.  For the length of its call it
//!   names `B` as the frame (`codegen_runtime::DvFrame`);
//! - the REGION is every loft function a value of a view type — or a record, collection or
//!   variant holding one — reaches from the root: each is re-emitted as `<f>__dv`, with the
//!   writes of a span field and the reads of a text one rewritten (`decode_view_emit`).  The
//!   rest of the program never sees a span, because no value leaves the region: a root may
//!   not return one, and every function one is handed to is in the region;
//! - `B` is READ ONLY in the region: a length, an element read, the source of a slice or a
//!   text run, an argument of a region call.  Its ALIASES are the region's parameters it is
//!   handed to, held to the same.  A span is written only from a parameter that is `B` at
//!   every call (`bparams`), so its offsets are the frame's.
//!
//! What a region function may do with a value of a view type is a closed list: field reads
//! and writes, the enum tag, copies, clears, frees, mints of the enum, a hand-off to a loft
//! function (which joins the region), and a return.  A text field of a view type is read only
//! through `OpGetText` (the hook reads the span); a byte-string field is never read at all.
//! Why the fallback is sound: any other mention of such a value, of `B`, or of an expression
//! whose type the walk cannot tell, DECLINES the root — the program keeps the owned fields it
//! always had, so a decline costs the rewrite and never a value.
//!
//! `LOFT_NO_DECODE_VIEW=1` keeps every field owned; `LOFT_TRACE_DECODE_VIEW=1` names each
//! root admitted and each one declined with the mention that declined it.
use crate::data::{Data, DefType, Type, Value};
use crate::database::{Parts, Stores};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};

/// The bit a `__dv` twin's request carries on its function number in the twin queues.
pub const DV_BIT: u32 = 1 << 31;

/// The span fields of one view type.
#[derive(Clone, Debug, Default)]
pub struct DvType {
    /// The span word's offset: the record's last eight bytes.
    pub spare: u32,
    /// Offsets of the text fields a span may stand for.
    pub text: Vec<u32>,
    /// Offsets and vector types of the byte-string fields a span may stand for.
    pub bytes: Vec<(u32, u16)>,
}

/// The whole program's plan ([`plan`]).
#[derive(Default)]
pub struct DvPlan {
    /// The enum's store type → its span fields; its variants' types map to the same entry.
    pub types: HashMap<u16, DvType>,
    /// The variants' store types → their enum.
    pub variant_of: HashMap<u16, u16>,
    /// The functions emitted as `__dv` twins.
    pub region: HashSet<u32>,
    /// Their names, for `fn_ident`.
    pub names: HashSet<String>,
    /// Each root → its frame variable.
    pub roots: HashMap<u32, u16>,
    /// The variables of each twin (and root) that are the frame at every call.
    pub bparams: HashMap<u32, HashSet<u16>>,
    /// The byte locals of each twin held as a span ([`span_locals`]).
    pub span_locals: HashMap<u32, HashSet<u16>>,
    /// A span local's holder statements (by IR address) → the slice's guard: on the guard's
    /// path the local is never built, so its holder is placed only on the fallback's.
    pub lazy: HashMap<usize, Value>,
}

impl DvPlan {
    /// The view type a record of store type `tp` is, or is a variant of.
    #[must_use]
    pub fn view_type(&self, tp: u16) -> Option<&DvType> {
        self.types
            .get(&tp)
            .or_else(|| self.types.get(self.variant_of.get(&tp)?))
    }
}

/// `LOFT_NO_DECODE_VIEW=1` — every decoded field owned (`@FR-R-DecodeView` off): the
/// before-half of every A/B and the first bisect step for a native-only wrong decoded text.
#[must_use]
pub fn disabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_NO_DECODE_VIEW").is_ok_and(|v| v != "0"))
}

fn trace_on() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_TRACE_DECODE_VIEW").is_ok_and(|v| v != "0"))
}

fn op_name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

/// A function with a loft body the region can re-emit.  Its `native` name is no test: a
/// library function the program also links from the library's own cdylib carries one, and
/// its loft body is still the one this build emits.
fn loft_fn(data: &Data, d: u32) -> bool {
    if (d as usize) >= data.definitions.len() {
        return false;
    }
    let def = data.def(d);
    matches!(def.def_type, DefType::Function)
        && !def.name().starts_with("Op")
        && def.rust.is_empty()
        && !matches!(def.code, Value::Null)
        && !matches!(def.returned.base(), Type::Iterator(..))
}

/// `vector<u8>`: a vector of integers that fit a byte.
fn byte_vector(tp: &Type) -> bool {
    // A `vector<u8>?` is one too (`@FR-N-Shape`): a callee parameter of that type is a
    // vector the frame can be confused with, which is the safe answer for `not_frame`.
    // `root_frame` refuses a nullable frame itself.
    let Type::Vector(e, _) = tp.base() else {
        return false;
    };
    matches!(e.base(), Type::Integer(s) if s.min >= 0 && s.max <= 255)
}

/// The view types: enums whose every text- or byte-string-carrying variant leaves the
/// record's last eight bytes free.
fn view_types(stores: &Stores) -> (HashMap<u16, DvType>, HashMap<u16, u16>) {
    let text_tp = stores.name("text");
    let mut types = HashMap::default();
    let mut variant_of = HashMap::default();
    for (tp, t) in stores.types.iter().enumerate() {
        let Parts::Enum(variants) = &t.parts else {
            continue;
        };
        let size = u32::from(stores.size(tp as u16));
        if size < 16 || size % 8 != 0 {
            continue;
        }
        let spare = size - 8;
        let mut dv = DvType {
            spare,
            ..DvType::default()
        };
        let mut fits = true;
        for (vtp, _) in variants {
            let Some(Parts::EnumValue(_, fields)) =
                stores.types.get(*vtp as usize).map(|t| &t.parts)
            else {
                continue;
            };
            let mut spans = Vec::new();
            let mut end = 0;
            for f in fields {
                let pos = u32::from(f.position);
                end = end.max(pos + u32::from(stores.size(f.content)));
                if f.content == text_tp {
                    spans.push((pos, None));
                } else if let Some(Parts::Vector(e)) =
                    stores.types.get(f.content as usize).map(|t| &t.parts)
                    && stores.size(*e) == 1
                {
                    spans.push((pos, Some(f.content)));
                }
            }
            if spans.is_empty() {
                continue;
            }
            if end > spare {
                fits = false;
                break;
            }
            for (pos, vec) in spans {
                match vec {
                    None => dv.text.push(pos),
                    Some(vtp) => dv.bytes.push((pos, vtp)),
                }
            }
        }
        if !fits || (dv.text.is_empty() && dv.bytes.is_empty()) {
            continue;
        }
        for (vtp, _) in variants {
            variant_of.insert(*vtp, tp as u16);
        }
        types.insert(tp as u16, dv);
    }
    (types, variant_of)
}

/// A variant with a text or byte-string field.
fn span_variant(stores: &Stores, vtp: u16) -> bool {
    let text_tp = stores.name("text");
    matches!(stores.types.get(vtp as usize).map(|t| &t.parts), Some(Parts::EnumValue(_, fields))
        if fields.iter().any(|f| f.content == text_tp
            || matches!(stores.types.get(f.content as usize).map(|t| &t.parts),
                Some(Parts::Vector(e)) if stores.size(*e) == 1)))
}

/// The store types that hold a value of a view type: the view types and their variants, and
/// every record, variant and collection with such a field or element.
fn holding_types(stores: &Stores, seed: impl Iterator<Item = u16>) -> HashSet<u16> {
    let mut c: HashSet<u16> = seed.collect();
    loop {
        let before = c.len();
        for (tp, t) in stores.types.iter().enumerate() {
            let tp = tp as u16;
            if c.contains(&tp) {
                continue;
            }
            let holds = match &t.parts {
                Parts::Struct(fields) | Parts::EnumValue(_, fields) => {
                    fields.iter().any(|f| c.contains(&f.content))
                }
                Parts::Enum(vs) => vs.iter().any(|(v, _)| c.contains(v)),
                Parts::Vector(e)
                | Parts::Array(e)
                | Parts::Sorted(e, _)
                | Parts::Ordered(e, _)
                | Parts::Hash(e, _)
                | Parts::Index(e, _)
                | Parts::Radix(e, _) => c.contains(e),
                _ => false,
            };
            if holds {
                c.insert(tp);
            }
        }
        if c.len() == before {
            return c;
        }
    }
}

/// What the walk knows of an expression's type: it holds a view value, it does not, or the
/// walk cannot tell.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Holds {
    Yes,
    No,
    Unknown,
}

struct Types<'a> {
    data: &'a Data,
    stores: &'a Stores,
    c: &'a HashSet<u16>,
}

impl Types<'_> {
    fn known(&self, d: u32) -> Option<u16> {
        if (d as usize) >= self.data.definitions.len() {
            return None;
        }
        let kt = self.data.def(d).known_type;
        (kt != u16::MAX).then_some(kt)
    }

    /// The record store type a loft type names, if it names one.
    fn record_tp(&self, tp: &Type) -> Option<u16> {
        match tp.base() {
            Type::Reference(d, _) | Type::Enum(d, _, _) => self.known(*d),
            Type::RefVar(inner) => self.record_tp(inner),
            _ => None,
        }
    }

    fn holds_type(&self, tp: &Type) -> Holds {
        match tp.base() {
            Type::Reference(d, _) | Type::Enum(d, _, _) => match self.known(*d) {
                Some(k) if self.c.contains(&k) => Holds::Yes,
                Some(_) => Holds::No,
                None => Holds::Unknown,
            },
            Type::RefVar(inner) => self.holds_type(inner),
            Type::Vector(e, _) => self.holds_type(e),
            Type::Sorted(d, ..)
            | Type::Index(d, ..)
            | Type::Radix(d, ..)
            | Type::Trie(d, ..)
            | Type::Hash(d, ..) => match self.known(*d) {
                Some(k) if self.c.contains(&k) => Holds::Yes,
                Some(_) => Holds::No,
                None => Holds::Unknown,
            },
            Type::Tuple(items) => {
                if items.iter().any(|t| self.holds_type(t) != Holds::No) {
                    Holds::Unknown
                } else {
                    Holds::No
                }
            }
            Type::Iterator(..) | Type::Function(..) => Holds::Unknown,
            _ => Holds::No,
        }
    }

    fn holds_tp(&self, tp: u16) -> Holds {
        if self.c.contains(&tp) {
            Holds::Yes
        } else {
            Holds::No
        }
    }

    /// The element store type of a vector expression.
    fn elem_tp(&self, def_nr: u32, vec: &Value) -> Option<u16> {
        match vec.unspan() {
            Value::Var(v) => match self.data.def(def_nr).variables().tp(*v).base() {
                Type::Vector(e, _) => self.record_tp(e),
                _ => None,
            },
            Value::Call(d, args) if op_name(self.data, *d) == "OpGetField" => {
                let vtp = u16::try_from(int_of(args.get(2)?)?).ok()?;
                match self.stores.types.get(vtp as usize).map(|t| &t.parts) {
                    Some(Parts::Vector(e)) => Some(*e),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// The record store type of a record expression.
    fn rec_tp(&self, def_nr: u32, x: &Value) -> Option<u16> {
        match x.unspan() {
            Value::Var(v) => self.record_tp(self.data.def(def_nr).variables().tp(*v)),
            Value::Call(d, args) => match op_name(self.data, *d) {
                "OpGetField" => u16::try_from(int_of(args.get(2)?)?).ok(),
                "OpGetVector" | "OpGetVectorNullable" => self.elem_tp(def_nr, args.first()?),
                "OpDatabase" | "OpDatabaseNP" => u16::try_from(int_of(args.get(1)?)?).ok(),
                _ if loft_fn(self.data, *d) => self.record_tp(&self.data.def(*d).returned),
                _ => None,
            },
            Value::Block(bl) => self.record_tp(&bl.result),
            _ => None,
        }
    }

    /// Is the first argument a variable whose vector's elements hold no view type?
    fn numbers_vector(&self, def_nr: u32, args: &[Value]) -> bool {
        let Some(Value::Var(v)) = args.first().map(Value::unspan) else {
            return false;
        };
        let tp = self.data.def(def_nr).variables().tp(*v).base();
        matches!(tp, Type::Vector(e, _) if self.holds_type(e) == Holds::No)
    }

    fn holds(&self, def_nr: u32, x: &Value) -> Holds {
        match x.unspan() {
            Value::Var(v) => self.holds_type(self.data.def(def_nr).variables().tp(*v)),
            Value::Call(d, args) => match op_name(self.data, *d) {
                // An element of a vector of numbers holds nothing.
                "OpGetVector" | "OpGetVectorNullable" if self.numbers_vector(def_nr, args) => {
                    Holds::No
                }
                "OpGetField"
                | "OpGetVector"
                | "OpGetVectorNullable"
                | "OpDatabase"
                | "OpDatabaseNP" => self
                    .rec_tp(def_nr, x)
                    .map_or(Holds::Unknown, |t| self.holds_tp(t)),
                _ => {
                    let ret = &self.data.def(*d).returned;
                    match ret.base() {
                        // A generic op's `reference` result says nothing of what it points at.
                        Type::Reference(..) | Type::Vector(..) | Type::RefVar(_)
                            if !loft_fn(self.data, *d) =>
                        {
                            if args.iter().any(|a| self.holds(def_nr, a) != Holds::No) {
                                Holds::Unknown
                            } else {
                                Holds::No
                            }
                        }
                        _ => self.holds_type(ret),
                    }
                }
            },
            Value::Block(bl) => self.holds_type(&bl.result),
            Value::Insert(items) => items.last().map_or(Holds::No, |l| self.holds(def_nr, l)),
            Value::If(_, t, e) => match (self.holds(def_nr, t), self.holds(def_nr, e)) {
                (Holds::No, Holds::No) => Holds::No,
                (Holds::Yes, _) | (_, Holds::Yes) => Holds::Yes,
                _ => Holds::Unknown,
            },
            Value::Null
            | Value::Int(_)
            | Value::Long(_)
            | Value::Float(_)
            | Value::Single(_)
            | Value::Boolean(_)
            | Value::Text(_)
            | Value::Enum(..)
            | Value::Line(_)
            | Value::Set(..)
            | Value::Return(_)
            | Value::Break(_)
            | Value::Continue(_)
            | Value::Loop(_)
            | Value::Drop(_)
            | Value::Keys(_) => Holds::No,
            _ => Holds::Unknown,
        }
    }
}

/// The view type the record expression `rec` (in the body of `def_nr`) is, if any — the one
/// question the walk and the emitter both ask of a field read or write.
#[must_use]
pub fn view_of<'p>(
    plan: &'p DvPlan,
    data: &Data,
    stores: &Stores,
    def_nr: u32,
    rec: &Value,
) -> Option<&'p DvType> {
    let c = HashSet::default();
    let t = Types {
        data,
        stores,
        c: &c,
    };
    plan.view_type(t.rec_tp(def_nr, rec)?)
}

pub(crate) fn int_of(x: &Value) -> Option<i64> {
    match x.unspan() {
        Value::Int(i) => Some(i64::from(*i)),
        Value::Long(l) => Some(*l),
        _ => None,
    }
}

/// A text run's guarded read, as `(R-TextRun)` writes it: the value of a text field set
/// `{ if g { w = text_from_byte_range(b, lo, hi) } else { w = … }; w }`.  Answers
/// `(g, b, lo, hi)`.
pub struct TextRun<'a> {
    pub guard: &'a Value,
    pub frame: u16,
    pub lo: &'a Value,
    pub hi: &'a Value,
}

#[must_use]
pub fn text_run<'a>(data: &Data, val: &'a Value) -> Option<TextRun<'a>> {
    let Value::Block(bl) = val.unspan() else {
        return None;
    };
    let [first, last] = bl.operators.as_slice() else {
        return None;
    };
    let (Value::If(guard, then, _), Value::Var(w)) = (first.unspan(), last.unspan()) else {
        return None;
    };
    let Value::Set(tw, rhs) = only_op(then) else {
        return None;
    };
    if tw != w {
        return None;
    }
    let Value::Call(d, args) = rhs.unspan() else {
        return None;
    };
    if op_name(data, *d) != "n_text_from_byte_range" {
        return None;
    }
    let [b, lo, hi] = args.as_slice() else {
        return None;
    };
    let Value::Var(frame) = b.unspan() else {
        return None;
    };
    Some(TextRun {
        guard,
        frame: *frame,
        lo,
        hi,
    })
}

/// The one operator of an arm written as a block or an insert of one, or the arm itself.
fn only_op(x: &Value) -> &Value {
    match x.unspan() {
        Value::Block(b) if b.operators.len() == 1 => b.operators[0].unspan(),
        Value::Insert(items) if items.len() == 1 => items[0].unspan(),
        t => t,
    }
}

/// The walk of one body for one root: what it hands to the region, and whether anything in
/// it declines the root.
struct Scan<'a> {
    t: &'a Types<'a>,
    plan_types: &'a HashMap<u16, DvType>,
    variant_of: &'a HashMap<u16, u16>,
    def_nr: u32,
    root: bool,
    /// The frame and its aliases in this body.
    frame: &'a HashSet<u16>,
    /// Region calls: callee, and which arguments are the frame.
    calls: Vec<(u32, Vec<bool>)>,
    why: Option<String>,
    /// Span writes the region can make here.
    sites: usize,
}

impl<'a> Scan<'a> {
    fn data(&self) -> &'a Data {
        self.t.data
    }

    fn decline(&mut self, why: impl Into<String>) {
        if self.why.is_none() {
            self.why = Some(why.into());
        }
    }

    fn view(&self, x: &Value) -> Option<&'a DvType> {
        let tp = self.t.rec_tp(self.def_nr, x)?;
        self.plan_types
            .get(&tp)
            .or_else(|| self.plan_types.get(self.variant_of.get(&tp)?))
    }

    fn is_frame(&self, x: &Value) -> bool {
        matches!(x.unspan(), Value::Var(v) if self.frame.contains(v))
    }

    fn walk(&mut self, x: &'a Value) {
        if self.why.is_some() {
            return;
        }
        match x.unspan() {
            Value::Var(v) if self.frame.contains(v) => {
                self.decline("a mention of the frame no read serves");
            }
            Value::Set(v, rhs) => {
                if self.frame.contains(v) {
                    self.decline("the frame rebound");
                }
                self.walk(rhs);
            }
            Value::Return(inner) => {
                if self.root && self.t.holds(self.def_nr, inner) != Holds::No {
                    self.decline("a decoded value returned out of its frame");
                }
                self.walk(inner);
            }
            Value::Call(d, args) => self.call(*d, args),
            Value::Iter(..)
            | Value::CallRef(..)
            | Value::FnRef(..)
            | Value::FnRefDnr(_)
            | Value::Yield(_)
            | Value::Parallel(_) => {
                self.decline("an iterator, a function reference, a yield or a parallel block");
            }
            Value::Tuple(items) => {
                for i in items {
                    if self.t.holds(self.def_nr, i) != Holds::No || self.is_frame(i) {
                        self.decline("a decoded value or the frame in a tuple");
                    }
                    self.walk(i);
                }
            }
            Value::TupleGet(v, _) | Value::TuplePut(v, _, _)
                if self
                    .t
                    .holds_type(self.data().def(self.def_nr).variables().tp(*v))
                    != Holds::No =>
            {
                self.decline("a tuple holding a decoded value");
            }
            _ => x.for_each_child(&mut |c| self.walk(c)),
        }
    }

    fn call(&mut self, d: u32, args: &'a [Value]) {
        let data = self.data();
        let name = op_name(data, d);
        if !name.starts_with("Op") {
            self.function(d, args);
            return;
        }
        // An element of the frame READ: the address under a getter.  The address alone is
        // what a setter writes through, so anywhere else it declines below.
        if name.starts_with("OpGet")
            && let Some(Value::Call(g, gargs)) = args.first().map(Value::unspan)
            && matches!(op_name(data, *g), "OpGetVector" | "OpGetVectorNullable")
            && let [b, _, idx] = gargs.as_slice()
            && self.is_frame(b)
        {
            self.walk(idx);
            for a in &args[1..] {
                self.walk(a);
            }
            return;
        }
        match (name, args) {
            ("OpLengthVector", [b]) if self.is_frame(b) => {}
            ("OpGetVector" | "OpGetVectorNullable", [b, ..]) if self.is_frame(b) => {
                self.decline("an element address of the frame outside a read");
            }
            ("OpSliceVector", [dst, b, lo, hi, _]) if self.is_frame(b) => {
                self.sites += 1;
                self.walk(dst);
                self.walk(lo);
                self.walk(hi);
            }
            ("OpGetText", [rec, off]) => {
                if self.view(rec).is_none()
                    && self.t.holds(self.def_nr, rec) == Holds::Unknown
                    && int_of(off).is_some_and(|o| {
                        self.plan_types
                            .values()
                            .any(|v| v.text.contains(&(o as u32)))
                    })
                {
                    self.decline("a text read of a record the walk cannot type");
                }
                self.walk(rec);
            }
            ("OpGetField", [rec, off, tp]) => {
                if let (Some(v), Some(o), Some(t)) = (self.view(rec), int_of(off), int_of(tp))
                    && v.bytes.contains(&(o as u32, t as u16))
                {
                    self.decline("a read of a decoded byte string");
                }
                self.walk(rec);
            }
            ("OpMoveVector", [dst, src, _]) => {
                // The destination may be a byte-string field: a write, not a read.
                if let Value::Call(g, gargs) = dst.unspan()
                    && op_name(data, *g) == "OpGetField"
                    && let [rec, _, _] = gargs.as_slice()
                {
                    self.walk(rec);
                } else {
                    self.walk(dst);
                }
                self.walk(src);
            }
            ("OpSetText", [rec, off, val]) => self.text_set(rec, off, val),
            ("OpDatabase" | "OpDatabaseNP", [x, tp]) => {
                // A variant record minted alone is the variant's size: one with a span
                // field has no room for the span word.
                if let Some(t) = int_of(tp)
                    && self.variant_of.contains_key(&(t as u16))
                    && !self.plan_types.contains_key(&(t as u16))
                    && span_variant(self.t.stores, t as u16)
                {
                    self.decline("a span variant's record minted alone (it has no span word)");
                }
                self.walk(x);
            }
            _ => {
                if !ALLOWED.contains(&name)
                    && args
                        .iter()
                        .any(|a| self.t.holds(self.def_nr, a) != Holds::No)
                {
                    self.decline(format!("{name} on a decoded value"));
                }
                for a in args {
                    self.walk(a);
                }
            }
        }
    }

    /// A text field set: a text run of the frame into a view type's field is a span the
    /// region writes, and its read of the frame is the run's own.
    fn text_set(&mut self, rec: &'a Value, off: &'a Value, val: &'a Value) {
        if self.view(rec).is_some()
            && let Some(run) = text_run(self.data(), val)
            && self.frame.contains(&run.frame)
        {
            self.sites += 1;
            self.walk(rec);
            self.walk(off);
            self.walk(run.guard);
            self.walk(run.lo);
            self.walk(run.hi);
            // The run's fallback arm reads its own vector, not the frame.
            if let Value::Block(bl) = val.unspan()
                && let Value::If(_, _, e) = bl.operators[0].unspan()
            {
                self.walk(e);
            }
            return;
        }
        self.walk(rec);
        self.walk(off);
        self.walk(val);
    }

    fn function(&mut self, d: u32, args: &'a [Value]) {
        let data = self.data();
        let holds = args
            .iter()
            .any(|a| self.t.holds(self.def_nr, a) != Holds::No)
            || self.t.holds_type(&data.def(d).returned) != Holds::No;
        let frame = args.iter().any(|a| self.is_frame(a));
        if !holds && !frame {
            for a in args {
                self.walk(a);
            }
            return;
        }
        if !loft_fn(data, d) {
            // A text read straight off the frame is a read.
            if op_name(data, d) == "n_text_from_byte_range"
                && let [b, lo, hi] = args
                && self.is_frame(b)
                && !holds
            {
                self.walk(lo);
                self.walk(hi);
                return;
            }
            self.decline(format!(
                "{} handed a decoded value or the frame",
                data.def(d).name()
            ));
            return;
        }
        let mut fb = Vec::with_capacity(args.len());
        for a in args {
            if self.is_frame(a) {
                fb.push(true);
            } else {
                fb.push(false);
                self.walk(a);
            }
        }
        self.calls.push((d, fb));
    }
}

/// Ops a region body may apply to a value of a view type, beyond those [`Scan::call`] reads.
const ALLOWED: &[&str] = &[
    "OpGetField",
    "OpGetVector",
    "OpGetVectorNullable",
    "OpGetEnum",
    "OpSetEnum",
    "OpConvIntFromEnum",
    "OpGetInt",
    "OpGetInt4",
    "OpGetShort",
    "OpGetByte",
    "OpGetBoolean",
    "OpGetFloat",
    "OpGetSingle",
    "OpGetLong",
    "OpSetInt",
    "OpSetInt4",
    "OpSetShort",
    "OpSetByte",
    "OpSetBoolean",
    "OpSetFloat",
    "OpSetSingle",
    "OpSetLong",
    "OpSetText",
    "OpLengthVector",
    "OpCopyRecord",
    "OpClear",
    "OpFreeRef",
    "OpFreeRefIfDistinct",
    "OpFreeRecordIn",
    "OpRefIsNull",
    "OpConvBoolFromRef",
    "OpRefAlias",
    "OpNewRecord",
    "OpFinishRecord",
    "OpPreAllocVector",
    "OpPlaceRecord",
    "OpCreateStack",
    "OpDistinctStore",
    // A shallow move relocates the record's bytes whole, the span word with them.
    "OpMoveField",
    "OpMoveRecord",
];

/// The parameter variable of `d` at argument position `i`.
fn param_var(data: &Data, d: u32, i: usize) -> Option<u16> {
    let def = data.def(d);
    let a = def.attributes().get(i)?;
    let v = def.variables().var(&a.name);
    (v != u16::MAX).then_some(v)
}

/// One root's region, or the reason it declines.
struct Region {
    members: HashSet<u32>,
    frame: HashMap<u32, HashSet<u16>>,
    not_frame: HashMap<u32, HashSet<u16>>,
    sites: usize,
}

fn region_of(
    t: &Types<'_>,
    plan_types: &HashMap<u16, DvType>,
    variant_of: &HashMap<u16, u16>,
    root: u32,
    b: u16,
) -> Result<Region, String> {
    let data = t.data;
    let mut r = Region {
        members: HashSet::default(),
        frame: HashMap::default(),
        not_frame: HashMap::default(),
        sites: 0,
    };
    r.frame.entry(root).or_default().insert(b);
    let mut scanned: HashMap<u32, HashSet<u16>> = HashMap::default();
    let mut queue = vec![root];
    while let Some(g) = queue.pop() {
        let frame = r.frame.get(&g).cloned().unwrap_or_default();
        if scanned.get(&g) == Some(&frame) {
            continue;
        }
        let mut scan = Scan {
            t,
            plan_types,
            variant_of,
            def_nr: g,
            root: g == root,
            frame: &frame,
            calls: Vec::new(),
            why: None,
            sites: 0,
        };
        scan.walk(&data.def(g).code);
        if let Some(why) = scan.why {
            return Err(format!("{}: {why}", data.def(g).name()));
        }
        r.sites += scan.sites;
        for (callee, fb) in scan.calls {
            if callee == root {
                return Err("the root is called from its own region".into());
            }
            r.members.insert(callee);
            for (i, is_b) in fb.iter().enumerate() {
                let Some(pv) = param_var(data, callee, i) else {
                    continue;
                };
                if *is_b {
                    r.frame.entry(callee).or_default().insert(pv);
                } else if byte_vector(data.def(callee).variables().tp(pv)) {
                    r.not_frame.entry(callee).or_default().insert(pv);
                }
            }
            if scanned.get(&callee) != r.frame.get(&callee).or(Some(&HashSet::default())) {
                queue.push(callee);
            }
        }
        scanned.insert(g, frame);
    }
    Ok(r)
}

/// A root's frame: a byte-vector parameter that is not a `&` one, of a function whose other
/// parameters cannot alias it (numbers and texts only).
fn root_frame(t: &Types<'_>, d: u32) -> Option<u16> {
    let data = t.data;
    let def = data.def(d);
    let mut frame = None;
    for (i, a) in def.attributes().iter().enumerate() {
        let pv = param_var(data, d, i)?;
        let tp = def.variables().tp(pv);
        if byte_vector(tp) {
            // A nullable frame is not one: no read the region serves takes an absent vector.
            if frame.is_some() || a.hidden || tp.peel_optional().1 {
                return None;
            }
            frame = Some(pv);
        } else if a.hidden {
            // A return or work buffer the call hands in: a record or a text, never the frame.
        } else if !matches!(
            tp.base(),
            Type::Integer(_)
                | Type::Float
                | Type::Single
                | Type::Boolean
                | Type::Character
                | Type::Text(_)
        ) {
            return None;
        }
    }
    let b = frame?;
    // It hands the frame to a call that answers a decoded value.
    let hands = def.code.any_node(&mut |n| {
        matches!(n, Value::Call(g, args) if loft_fn(data, *g)
            && t.holds_type(&data.def(*g).returned) == Holds::Yes
            && args.iter().any(|a| matches!(a.unspan(), Value::Var(v) if *v == b)))
    });
    trace(|| {
        format!(
            "{}: frame candidate `{}`, hands it to a decoder: {hands}",
            def.name(),
            def.variables().name(b)
        )
    });
    hands.then_some(b)
}

/// The byte locals of `d` a span stands for: `bs` filled only by `if g { OpSliceVector(bs,
/// b, lo, hi) } else { … }` from a frame parameter `b`, then moved, by the next statement
/// that names it, into a byte-string field of a view type — `bs` named nowhere else but its
/// declaration, its placement and the fallback's pushes, and no `break` or `continue`
/// between the slice and the move.  Its span flag is set by the slice and taken by the move,
/// so a move the slice did not precede moves the vector.
fn span_locals(
    data: &Data,
    d: u32,
    frame: &HashSet<u16>,
    lazy: &mut HashMap<usize, Value>,
) -> HashSet<u16> {
    let mut out = HashSet::default();
    let code = &data.def(d).code;
    let mut cands: Vec<u16> = Vec::new();
    code.any_node(&mut |n| {
        if let Value::Call(g, args) = n
            && op_name(data, *g) == "OpSliceVector"
            && let [dst, b, ..] = args.as_slice()
            && let (Value::Var(bs), Value::Var(bv)) = (dst.unspan(), b.unspan())
            && frame.contains(bv)
        {
            cands.push(*bs);
        }
        false
    });
    for bs in cands {
        if span_local_ok(data, code, bs) {
            out.insert(bs);
            holders(data, code, bs, lazy);
        }
    }
    out
}

/// The statements before the slice's `if` that only build `bs`'s holder — its placement
/// (`__vdb = OpPlaceRecord(…)`, in an insert), `bs = OpGetField(__vdb, …)` and the holder's
/// `OpSetInt4` — under the slice's guard, when nothing from the first of them to the `if`
/// assigns a variable the guard reads.
fn holders(data: &Data, code: &Value, bs: u16, lazy: &mut HashMap<usize, Value>) {
    let mut found = None;
    code.any_node(&mut |n| {
        if let Value::Block(bl) = n
            && let Some(i) = bl.operators.iter().position(|s| is_slice_if(data, s, bs))
        {
            found = Some((&bl.operators, i));
            return true;
        }
        false
    });
    let Some((ops, i)) = found else {
        return;
    };
    let Value::If(guard, ..) = ops[i].unspan() else {
        return;
    };
    let vdb = ops[..i].iter().find_map(|s| match s.unspan() {
        Value::Set(u, rhs) if *u == bs => match rhs.unspan() {
            Value::Call(g, args) if op_name(data, *g) == "OpGetField" => {
                match args.first()?.unspan() {
                    Value::Var(h) => Some(*h),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    });
    let Some(vdb) = vdb else {
        return;
    };
    let holder = |s: &Value| match s.unspan() {
        Value::Set(u, _) if *u == bs => true,
        Value::Insert(items) => items.iter().any(|x| {
            let Value::Set(u, rhs) = x.unspan() else {
                return false;
            };
            *u == vdb
                && matches!(rhs.unspan(), Value::Call(g, _)
                    if op_name(data, *g) == "OpPlaceRecord")
        }),
        Value::Call(g, args) => {
            op_name(data, *g) == "OpSetInt4"
                && matches!(args.first().map(Value::unspan), Some(Value::Var(u)) if *u == vdb)
        }
        _ => false,
    };
    let Some(first) = ops[..i].iter().position(holder) else {
        return;
    };
    let mut read: HashSet<u16> = HashSet::default();
    guard.any_node(&mut |n| {
        if let Value::Var(v) = n {
            read.insert(*v);
        }
        false
    });
    let assigns = ops[first..i]
        .iter()
        .any(|s| s.any_node(&mut |n| matches!(n, Value::Set(v, _) if read.contains(v))));
    if assigns {
        return;
    }
    for s in &ops[first..i] {
        if holder(s) {
            lazy.insert(std::ptr::from_ref(s) as usize, (**guard).clone());
        }
    }
}

fn mentions(x: &Value, v: u16) -> bool {
    x.any_node(&mut |n| match n {
        Value::Var(u) | Value::Set(u, _) => *u == v,
        _ => false,
    })
}

fn is_slice_if(data: &Data, x: &Value, bs: u16) -> bool {
    let Value::If(_, then, _) = x.unspan() else {
        return false;
    };
    let then = only_op(then);
    matches!(then, Value::Call(g, args) if op_name(data, *g) == "OpSliceVector"
        && matches!(args.first().map(Value::unspan), Some(Value::Var(u)) if *u == bs))
}

fn is_move_of(data: &Data, x: &Value, bs: u16) -> bool {
    matches!(x.unspan(), Value::Call(g, args) if op_name(data, *g) == "OpMoveVector"
        && matches!(args.get(1).map(Value::unspan), Some(Value::Var(u)) if *u == bs))
}

/// Every mention of `bs` inside `x` is the one move.
fn only_the_move(data: &Data, x: &Value, bs: u16) -> (bool, usize) {
    if is_move_of(data, x, bs) {
        return (true, 1);
    }
    if let Value::Var(u) | Value::Set(u, _) = x.unspan()
        && *u == bs
    {
        return (false, 0);
    }
    let mut ok = true;
    let mut moves = 0;
    x.for_each_child(&mut |c| {
        let (o, m) = only_the_move(data, c, bs);
        ok &= o;
        moves += m;
    });
    (ok, moves)
}

fn has_jump(x: &Value) -> bool {
    x.any_node(&mut |n| matches!(n, Value::Break(_) | Value::Continue(_)))
}

fn span_local_ok(data: &Data, code: &Value, bs: u16) -> bool {
    // The block holding the slice's `if` as one of its statements.
    let mut found = None;
    code.any_node(&mut |n| {
        if let Value::Block(bl) = n
            && let Some(i) = bl.operators.iter().position(|s| is_slice_if(data, s, bs))
        {
            found = Some((&bl.operators, i));
            return true;
        }
        false
    });
    let Some((ops, i)) = found else {
        trace(|| {
            let mut shape = String::new();
            code.any_node(&mut |n| {
                if let Value::Block(bl) = n {
                    for st in &bl.operators {
                        if let Value::If(_, t, _) = st.unspan()
                            && format!("{t:?}").contains("Call(")
                            && mentions(t, bs)
                        {
                            shape = format!("{t:?}").chars().take(400).collect();
                        }
                    }
                }
                false
            });
            format!("span local {bs}: no slice `if` as a statement; then = {shape}")
        });
        return false;
    };
    // Before the `if`, in this block: its declaration and placement only — assignments.
    for s in &ops[..i] {
        if mentions(s, bs) && !matches!(s.unspan(), Value::Set(u, _) if *u == bs) {
            let Value::Insert(items) = s.unspan() else {
                trace(|| {
                    format!("span local {bs}: named before the slice: {s:?}")
                        .chars()
                        .take(300)
                        .collect()
                });
                return false;
            };
            if items.iter().any(|x| mentions(x, bs)) {
                trace(|| format!("span local {bs}: named in an insert before the slice"));
                return false;
            }
        }
    }
    // The `if`'s else arm: pushes only, which fill the owned vector.
    // After it: the next statement naming `bs` holds the one move and no other mention, with
    // no jump on the way; nothing after names it.
    let mut moved = false;
    for s in &ops[i + 1..] {
        if moved {
            if mentions(s, bs) {
                return false;
            }
            continue;
        }
        if !mentions(s, bs) {
            if has_jump(s) {
                return false;
            }
            continue;
        }
        let (ok, n) = only_the_move(data, s, bs);
        if !ok || n != 1 || matches!(s.unspan(), Value::Loop(_)) {
            trace(|| {
                format!(
                    "span local {bs}: the next statement naming it is no single move ({ok} {n})"
                )
            });
            return false;
        }
        moved = true;
    }
    // Nowhere else in the body but its declaration: only this block's statements name it.
    moved && mentions_only_in(code, ops, bs)
}

/// No mention of `bs` lies outside the statements `ops`, but a `bs = null` declaration.
fn mentions_only_in(code: &Value, ops: &Vec<Value>, bs: u16) -> bool {
    fn walk(x: &Value, ops: &Vec<Value>, bs: u16) -> bool {
        if let Value::Block(bl) = x.unspan()
            && std::ptr::eq(&raw const bl.operators, ops)
        {
            return true;
        }
        match x.unspan() {
            Value::Set(u, rhs) if *u == bs && matches!(rhs.unspan(), Value::Null) => return true,
            Value::Var(u) | Value::Set(u, _) if *u == bs => return false,
            _ => {}
        }
        let mut ok = true;
        x.for_each_child(&mut |c| ok &= walk(c, ops, bs));
        ok
    }
    walk(code, ops, bs)
}

fn trace(msg: impl FnOnce() -> String) {
    if trace_on() {
        eprintln!("[decode-view] {}", msg());
    }
}

/// The program's plan: its view types, the roots that admit a region, and the region.
#[must_use]
pub fn plan(data: &Data, stores: &Stores) -> DvPlan {
    let mut plan = DvPlan::default();
    if disabled() {
        return plan;
    }
    let (types, variant_of) = view_types(stores);
    if types.is_empty() {
        return plan;
    }
    let c = holding_types(
        stores,
        types.keys().copied().chain(variant_of.keys().copied()),
    );
    let t = Types {
        data,
        stores,
        c: &c,
    };
    let mut frame: HashMap<u32, HashSet<u16>> = HashMap::default();
    let mut not_frame: HashMap<u32, HashSet<u16>> = HashMap::default();
    for d in 0..data.definitions.len() as u32 {
        if !loft_fn(data, d) {
            continue;
        }
        let Some(b) = root_frame(&t, d) else {
            continue;
        };
        match region_of(&t, &types, &variant_of, d, b) {
            Ok(r) if r.sites > 0 => {
                trace(|| {
                    format!(
                        "{}: a root over `{}`, region {}",
                        data.def(d).name(),
                        data.def(d).variables().name(b),
                        r.members.len()
                    )
                });
                plan.roots.insert(d, b);
                plan.region.extend(r.members.iter().copied());
                for (g, s) in r.frame {
                    frame.entry(g).or_default().extend(s);
                }
                for (g, s) in r.not_frame {
                    not_frame.entry(g).or_default().extend(s);
                }
            }
            Ok(_) => trace(|| format!("{}: declines: no span to write", data.def(d).name())),
            Err(why) => trace(|| format!("{}: declines: {why}", data.def(d).name())),
        }
    }
    // A root no region reaches is emitted once, as a root; one that a region reaches is
    // also a twin, and its own frame is then whatever its twin's callers hand it.
    for (g, s) in frame {
        let nf = not_frame.get(&g);
        let exact: HashSet<u16> = s
            .into_iter()
            .filter(|v| nf.is_none_or(|n| !n.contains(v)))
            .collect();
        if plan.region.contains(&g) {
            let sl = span_locals(data, g, &exact, &mut plan.lazy);
            trace(|| {
                let names = |vs: &HashSet<u16>| {
                    let mut n: Vec<&str> = vs
                        .iter()
                        .map(|v| data.def(g).variables().name(*v))
                        .collect();
                    n.sort_unstable();
                    n.join(",")
                };
                format!(
                    "{}: frame [{}], span locals [{}]",
                    data.def(g).name(),
                    names(&exact),
                    names(&sl)
                )
            });
            if !sl.is_empty() {
                plan.span_locals.insert(g, sl);
            }
        }
        plan.bparams.insert(g, exact);
    }
    for d in &plan.region {
        plan.names.insert(data.def(*d).name().to_string());
    }
    plan.types = types;
    plan.variant_of = variant_of;
    plan
}
