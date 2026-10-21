// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! `@FR-R-Apart`, instance 4 — a record of RECORD VECTORS built in a frame and handed to the
//! functions that copy it into a collection, held as one byte buffer per field
//! (`codegen_runtime::AprVec`) in the element's own layout.  The store meets it once, at the
//! BOUNDARY: the copy of an append group, which writes the bytes into the blocks of the
//! appended element (`Stores::apart_write_fresh`) or, under `(R-HandOver)`'s rotate clause,
//! into the oldest element's (`Stores::rotate_from_parts`).
//!
//! The plan is decided here, from the IR alone:
//!
//! - an APART TYPE is a struct whose every field is a `vector` of a struct whose every field
//!   is an `integer`, a `float` or a `boolean` — the element is plain bytes, and each field is
//!   written by the op the store path runs (`OpSetInt` / `OpSetFloat` / `OpSetBoolean`);
//! - a TWIN `n_<f>__apr` is a function with exactly one parameter of such a type, every
//!   mention of which is a field's length, an argument of a twin's call at its apart
//!   position, or the copy `OpCopyRecord(p, elm, FRESH)` of an append group.  Twins are a
//!   greatest fixpoint: a candidate whose call of another candidate falls out falls out;
//! - an OWNER is a local of a body whose every mention is its null init, its mint
//!   (`OpDatabase`), a field emptied (`OpSetInt4(v, off, 0)`), a PUSH GROUP, a field's length,
//!   an argument of a twin, or its free — and which reaches at least one twin.
//!
//! A PUSH GROUP is `elm = OpNewRecord(v, tp, fld)`, writes `OpSet*(elm, off, x)` covering every
//! field of the element exactly once (no field left to the store's prefill), and
//! `OpFinishRecord(v, elm, tp, fld)`; `elm` is named by nothing else, and `x` names neither.
//!
//! Why the fallback is sound: an unlisted mention declines the variable (an owner) or the
//! function (a twin), which costs the rewrite and never a value; an apart value is a distinct
//! Rust type, so a mention this gate admits in error and the emitter does not rewrite is a
//! `rustc` type error, not a wrong answer.
use crate::data::{Data, DefType, Type, Value};
use crate::database::{Parts, Stores};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};

/// How one element field is written: the store's encoding of its type.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Enc {
    I64,
    F64,
    Bool,
}

/// One record-vector field of an apart type.
#[derive(Clone, Debug)]
pub struct RecField {
    /// The field's byte offset in the record: the vector's slot.
    pub off: u32,
    /// The element's store type and size.
    pub elem_tp: u16,
    pub esize: u32,
    /// The element's fields: byte offset and encoding.
    pub elem_fields: Vec<(u32, Enc)>,
}

/// The value form of one apart type.
#[derive(Clone, Debug)]
pub struct RecLayout {
    /// The Rust struct the value is (`Apr<n>`).
    pub rust: String,
    pub rec_tp: u16,
    pub fields: Vec<RecField>,
}

impl RecLayout {
    /// The field whose slot is at byte offset `off`.
    #[must_use]
    pub fn field(&self, off: i64) -> Option<usize> {
        self.fields.iter().position(|f| i64::from(f.off) == off)
    }
}

/// The program's plan.
#[derive(Clone, Debug, Default)]
pub struct RecPlan {
    /// Record definition → layout.
    pub types: HashMap<u32, RecLayout>,
    /// Twin function → (its apart parameter's variable, argument index, record definition).
    pub twins: HashMap<u32, (u16, usize, u32)>,
    /// Function → its apart owners (variable → record definition).
    pub owners: HashMap<u32, HashMap<u16, u32>>,
}

/// The apart variables of the body being emitted.
#[derive(Clone, Debug, Default)]
pub struct RecRoles {
    /// Owners and a twin's parameter → record definition.
    pub vars: HashMap<u16, u32>,
    /// The twin's parameter, which is a reference to the value.
    pub param: Option<u16>,
    /// A push group's element variable → (its owner, the field index).
    pub elems: HashMap<u16, (u16, usize)>,
}

impl RecRoles {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty()
    }

    /// Is `v` a variable the store does not hold (an apart value or an element of one)?
    #[must_use]
    pub fn holds(&self, v: u16) -> bool {
        self.vars.contains_key(&v) || self.elems.contains_key(&v)
    }
}

/// `LOFT_NO_APART_RECORDS=1` — every record of record vectors lives in the store
/// (`@FR-R-Apart`'s instance 4 off; `LOFT_NO_APART=1` turns it off too): the first bisect step
/// for a native-only wrong element after a frame-built record is copied into a collection.
#[must_use]
pub fn disabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| {
        super::apart_value::apart_disabled()
            || std::env::var("LOFT_NO_APART_RECORDS").is_ok_and(|v| v != "0")
    })
}

fn trace_on() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_TRACE_APART").is_ok_and(|v| v != "0"))
}

fn op_name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

fn int_of(v: &Value) -> Option<i64> {
    match v.unspan() {
        Value::Int(i) => Some(i64::from(*i)),
        Value::Long(l) => Some(*l),
        _ => None,
    }
}

fn var_of(v: &Value) -> Option<u16> {
    match v.unspan() {
        Value::Var(x) => Some(*x),
        _ => None,
    }
}

/// The candidate types: every field a vector of plain scalar records.
fn candidate_types(data: &Data, stores: &Stores) -> HashMap<u32, RecLayout> {
    let scalar = |t: u16| -> Option<(Enc, u16)> {
        let name = stores.types.get(t as usize)?.name.as_str();
        match name {
            "integer" => Some((Enc::I64, 8)),
            "float" => Some((Enc::F64, 8)),
            "boolean" => Some((Enc::Bool, 1)),
            _ => None,
        }
    };
    let mut out = HashMap::default();
    for (d, def) in data.definitions.iter().enumerate() {
        if !matches!(def.def_type, DefType::Struct) {
            continue;
        }
        let kt = def.known_type;
        let Some(Parts::Struct(fields)) = stores.types.get(kt as usize).map(|t| &t.parts) else {
            continue;
        };
        if fields.is_empty() {
            continue;
        }
        let mut rec_fields = Vec::new();
        for f in fields {
            let Some(Parts::Vector(e)) = stores.types.get(f.content as usize).map(|t| &t.parts)
            else {
                break;
            };
            let Some(Parts::Struct(efields)) = stores.types.get(*e as usize).map(|t| &t.parts)
            else {
                break;
            };
            let elem_fields: Option<Vec<(u32, Enc)>> = efields
                .iter()
                .map(|ef| {
                    let (enc, size) = scalar(ef.content)?;
                    (stores.size(ef.content) == size).then_some((u32::from(ef.position), enc))
                })
                .collect();
            let Some(elem_fields) = elem_fields.filter(|v| !v.is_empty()) else {
                break;
            };
            rec_fields.push(RecField {
                off: u32::from(f.position),
                elem_tp: *e,
                esize: u32::from(stores.size(*e)),
                elem_fields,
            });
        }
        if rec_fields.len() == fields.len() {
            out.insert(
                d as u32,
                RecLayout {
                    rust: format!("Apr{d}"),
                    rec_tp: kt,
                    fields: rec_fields,
                },
            );
        }
    }
    out
}

/// The record definition a variable's type is, when it is an apart type (never `τ?`).
fn record_of(tp: &Type, types: &HashMap<u32, RecLayout>) -> Option<u32> {
    if tp.peel_optional().1 {
        return None;
    }
    match tp.base() {
        Type::Reference(d, _) if types.contains_key(d) => Some(*d),
        _ => None,
    }
}

/// The walk of one body over the variables it tracks.
struct Walk<'a> {
    data: &'a Data,
    types: &'a HashMap<u32, RecLayout>,
    /// Candidate twins: function → (argument index, record definition).
    candidates: &'a HashMap<u32, (usize, u32)>,
    /// Tracked variables → record definition.
    tracked: HashMap<u16, u32>,
    /// Whether the tracked variable is a twin's parameter (no mint, no push).
    param: bool,
    /// Element variables → (owner, field index); the open groups' writes.
    elems: HashMap<u16, (u16, usize)>,
    open: HashMap<u16, Vec<u32>>,
    /// The variables a mention declined, and why.
    declined: HashMap<u16, &'static str>,
    /// The twins each tracked variable is handed to.
    calls: HashMap<u16, HashSet<u32>>,
}

impl Walk<'_> {
    fn decline(&mut self, v: u16, why: &'static str) {
        self.declined.entry(v).or_insert(why);
    }

    fn layout(&self, v: u16) -> &RecLayout {
        &self.types[&self.tracked[&v]]
    }

    /// A field's length `len(OpGetField(v, off, _))`: the tracked variable.
    fn length(&self, x: &Value) -> Option<u16> {
        let Value::Call(d, a) = x.unspan() else {
            return None;
        };
        if !matches!(op_name(self.data, *d), "t_6vector_len" | "OpLengthVector") {
            return None;
        }
        let [f] = &a[..] else { return None };
        let Value::Call(g, fa) = f.unspan() else {
            return None;
        };
        if op_name(self.data, *g) != "OpGetField" {
            return None;
        }
        let v = var_of(fa.first()?)?;
        let off = int_of(fa.get(1)?)?;
        (self.tracked.contains_key(&v) && self.layout(v).field(off).is_some()).then_some(v)
    }

    /// Walk one node: an admitted mention takes its tracked variable, and the walk goes on
    /// into the operands it does not take; any other `Var` of a tracked or element variable
    /// declines it.
    fn node(&mut self, x: &Value) {
        if self.length(x).is_some() {
            return;
        }
        match x.unspan() {
            Value::Var(v) => {
                if let Some(&(owner, _)) = self.elems.get(v) {
                    self.decline(owner, "an element named outside its push group");
                } else if self.tracked.contains_key(v) {
                    self.decline(*v, "a mention no clause admits");
                }
            }
            Value::Set(v, rhs) if self.tracked.contains_key(v) => {
                if !matches!(rhs.unspan(), Value::Null) || self.param {
                    self.decline(*v, "assigned");
                }
            }
            Value::Set(elm, rhs) => {
                if let Value::Call(d, a) = rhs.unspan()
                    && op_name(self.data, *d) == "OpNewRecord"
                    && let [p, ptp, fld] = &a[..]
                    && let Some(v) = var_of(p)
                    && self.tracked.contains_key(&v)
                {
                    let layout = self.layout(v);
                    let fld = int_of(fld).and_then(|f| usize::try_from(f).ok());
                    match fld.filter(|f| *f < layout.fields.len()) {
                        Some(f) if !self.param && int_of(ptp) == Some(i64::from(layout.rec_tp)) => {
                            self.elems.insert(*elm, (v, f));
                            self.open.insert(*elm, Vec::new());
                        }
                        _ => self.decline(v, "an element minted outside a push group"),
                    }
                    return;
                }
                if self.elems.contains_key(elm) {
                    let owner = self.elems[elm].0;
                    self.decline(owner, "an element variable reassigned");
                }
                self.node(rhs);
            }
            Value::Call(d, a) => self.call(*d, a),
            _ => x.for_each_child(&mut |c| self.node(c)),
        }
    }

    fn call(&mut self, d: u32, a: &[Value]) {
        let name = op_name(self.data, d);
        let first = a.first().and_then(var_of);
        match (name, first) {
            ("OpDatabase", Some(v)) if self.tracked.contains_key(&v) => {
                if self.param || a.get(1).and_then(int_of) != Some(i64::from(self.layout(v).rec_tp))
                {
                    self.decline(v, "a mint of another type");
                }
                return;
            }
            ("OpSetInt4", Some(v)) if self.tracked.contains_key(&v) => {
                let off = a.get(1).and_then(int_of).unwrap_or(-1);
                if self.param
                    || self.layout(v).field(off).is_none()
                    || a.get(2).and_then(int_of) != Some(0)
                {
                    self.decline(v, "a field written other than emptied");
                }
                return;
            }
            ("OpFreeRef", Some(v)) if self.tracked.contains_key(&v) => {
                if self.param {
                    self.decline(v, "a parameter freed");
                }
                return;
            }
            ("OpSetInt" | "OpSetFloat" | "OpSetBoolean", Some(elm))
                if self.elems.contains_key(&elm) =>
            {
                let (owner, f) = self.elems[&elm];
                let enc = match name {
                    "OpSetInt" => Enc::I64,
                    "OpSetFloat" => Enc::F64,
                    _ => Enc::Bool,
                };
                let off = a
                    .get(1)
                    .and_then(int_of)
                    .and_then(|o| u32::try_from(o).ok());
                let field = &self.layout(owner).fields[f];
                let fits = off.is_some_and(|o| field.elem_fields.contains(&(o, enc)));
                match (fits, self.open.get_mut(&elm)) {
                    (true, Some(written)) if !written.contains(&off.unwrap_or(0)) => {
                        written.push(off.unwrap_or(0));
                    }
                    _ => self.decline(
                        owner,
                        "an element field written twice, out of its group or in another encoding",
                    ),
                }
                if let Some(val) = a.get(2) {
                    self.node(val);
                }
                return;
            }
            ("OpFinishRecord", Some(v)) if self.tracked.contains_key(&v) => {
                let elm = a.get(1).and_then(var_of);
                match elm.and_then(|e| self.open.remove(&e).map(|w| (e, w))) {
                    Some((e, written)) if self.elems[&e].0 == v => {
                        let f = self.elems[&e].1;
                        if written.len() != self.layout(v).fields[f].elem_fields.len() {
                            self.decline(v, "an element whose fields are not all written");
                        }
                    }
                    _ => self.decline(v, "a finish without its group"),
                }
                return;
            }
            ("OpCopyRecord", Some(p)) if self.tracked.contains_key(&p) => {
                // The boundary: a FRESH destination that keeps its source.
                let flags = a
                    .get(2)
                    .and_then(int_of)
                    .and_then(|t| u16::try_from(t).ok());
                let rec_tp = self.layout(p).rec_tp;
                let ok = self.param
                    && flags.is_some_and(|t| {
                        t & crate::keys::COPY_FRESH_DEST != 0
                            && t & crate::keys::COPY_FREE_SOURCE == 0
                            && t & crate::keys::COPY_TP_MASK == rec_tp
                    })
                    && a.get(1).and_then(var_of).is_some();
                if !ok {
                    self.decline(p, "a copy that is no append's fresh element");
                }
                return;
            }
            _ => {}
        }
        // A user call: a tracked variable as an argument at a candidate twin's apart position.
        for (i, arg) in a.iter().enumerate() {
            if let Some(v) = var_of(arg)
                && self.tracked.contains_key(&v)
            {
                match self.candidates.get(&d) {
                    Some(&(at, rec)) if at == i && rec == self.tracked[&v] => {
                        self.calls.entry(v).or_default().insert(d);
                    }
                    _ => self.decline(v, "handed to a function that is no twin"),
                }
            } else {
                self.node(arg);
            }
        }
    }
}

/// The candidate twins: functions with exactly one parameter of an apart type.
fn candidate_twins(data: &Data, types: &HashMap<u32, RecLayout>) -> HashMap<u32, (usize, u32)> {
    let mut out = HashMap::default();
    for (d, def) in data.definitions.iter().enumerate() {
        if !matches!(def.def_type, DefType::Function)
            || data.is_placeholder_instance(d as u32)
            || *def.code() == Value::Null
        {
            continue;
        }
        let params: Vec<(usize, u32)> = def
            .attributes()
            .iter()
            .enumerate()
            .filter_map(|(i, a)| record_of(&a.typedef, types).map(|r| (i, r)))
            .collect();
        if let [(i, r)] = params[..] {
            out.insert(d as u32, (i, r));
        }
    }
    out
}

/// Walk `f`'s body tracking `tracked`.
fn walk<'a>(
    data: &'a Data,
    types: &'a HashMap<u32, RecLayout>,
    candidates: &'a HashMap<u32, (usize, u32)>,
    f: u32,
    tracked: HashMap<u16, u32>,
    param: bool,
) -> Walk<'a> {
    let mut w = Walk {
        data,
        types,
        candidates,
        tracked,
        param,
        elems: HashMap::default(),
        open: HashMap::default(),
        declined: HashMap::default(),
        calls: HashMap::default(),
    };
    w.node(data.def(f).code());
    for (elm, _) in std::mem::take(&mut w.open) {
        let owner = w.elems[&elm].0;
        w.decline(owner, "a push group left open");
    }
    w
}

/// The program's plan: the types, the twins (a greatest fixpoint) and each body's owners.
#[must_use]
pub fn plan(data: &Data, stores: &Stores) -> RecPlan {
    let mut plan = RecPlan::default();
    if disabled() {
        return plan;
    }
    let types = candidate_types(data, stores);
    if types.is_empty() {
        return plan;
    }
    let mut candidates = candidate_twins(data, &types);
    // Each candidate's own verdict, then the fixpoint over the twins it calls.
    let mut needs: HashMap<u32, HashSet<u32>> = HashMap::default();
    let mut params: HashMap<u32, u16> = HashMap::default();
    let mut refused: Vec<u32> = Vec::new();
    for (&f, &(i, rec)) in &candidates {
        let def = data.def(f);
        let v = def.variables().var(&def.attributes()[i].name);
        let mut t = HashMap::default();
        t.insert(v, rec);
        let w = walk(data, &types, &candidates, f, t, true);
        if let Some(why) = w.declined.get(&v) {
            if trace_on() {
                eprintln!("[apart-records] {} is no twin: {why}", def.name());
            }
            refused.push(f);
            continue;
        }
        needs.insert(f, w.calls.get(&v).cloned().unwrap_or_default());
        params.insert(f, v);
    }
    for f in refused {
        candidates.remove(&f);
    }
    loop {
        let drop: Vec<u32> = needs
            .iter()
            .filter(|(f, n)| {
                !candidates.contains_key(f) || n.iter().any(|g| !candidates.contains_key(g))
            })
            .map(|(f, _)| *f)
            .collect();
        if drop.is_empty() {
            break;
        }
        for f in drop {
            needs.remove(&f);
            candidates.remove(&f);
        }
    }
    candidates.retain(|f, _| needs.contains_key(f));
    for (&f, &(i, rec)) in &candidates {
        plan.twins.insert(f, (params[&f], i, rec));
    }
    // The owners: locals of an apart type in any body, each reaching a twin.
    for (d, def) in data.definitions.iter().enumerate() {
        let f = d as u32;
        if !matches!(def.def_type, DefType::Function)
            || data.is_placeholder_instance(f)
            || *def.code() == Value::Null
        {
            continue;
        }
        let vars = def.variables();
        let mut tracked: HashMap<u16, u32> = HashMap::default();
        for v in 0..vars.count() {
            if vars.is_argument(v) {
                continue;
            }
            if let Some(rec) = record_of(vars.tp(v), &types) {
                tracked.insert(v, rec);
            }
        }
        if tracked.is_empty() {
            continue;
        }
        let w = walk(data, &types, &candidates, f, tracked.clone(), false);
        let mut owners: HashMap<u16, u32> = HashMap::default();
        for (v, rec) in tracked {
            let why = w.declined.get(&v).copied().or_else(|| {
                (!w.calls.get(&v).is_some_and(|c| !c.is_empty())).then_some("reaches no twin")
            });
            if let Some(why) = why {
                if trace_on() {
                    eprintln!(
                        "[apart-records] {}: `{}` stays in the store: {why}",
                        def.name(),
                        vars.name(v)
                    );
                }
                continue;
            }
            owners.insert(v, rec);
        }
        if !owners.is_empty() {
            plan.owners.insert(f, owners);
        }
    }
    if plan.owners.is_empty() {
        plan.twins.clear();
    }
    plan.types = types;
    if trace_on() {
        for (f, (v, _, _)) in &plan.twins {
            let def = data.def(*f);
            eprintln!(
                "[apart-records] twin {}__apr takes `{}` apart",
                def.name(),
                def.variables().name(*v)
            );
        }
        for (f, o) in &plan.owners {
            let def = data.def(*f);
            for v in o.keys() {
                eprintln!(
                    "[apart-records] {}: `{}` is apart",
                    def.name(),
                    def.variables().name(*v)
                );
            }
        }
    }
    plan
}

/// The roles of `f`'s body: its owners and their push groups' elements, or — for its twin —
/// the apart parameter.
#[must_use]
pub fn roles(data: &Data, plan: &RecPlan, f: u32, twin: bool) -> RecRoles {
    let mut r = RecRoles::default();
    let tracked: HashMap<u16, u32> = if twin {
        let Some(&(v, _, rec)) = plan.twins.get(&f) else {
            return r;
        };
        r.param = Some(v);
        std::iter::once((v, rec)).collect()
    } else {
        match plan.owners.get(&f) {
            Some(o) => o.clone(),
            None => return r,
        }
    };
    let candidates: HashMap<u32, (usize, u32)> = plan
        .twins
        .iter()
        .map(|(&g, &(_, i, rec))| (g, (i, rec)))
        .collect();
    let w = walk(data, &plan.types, &candidates, f, tracked.clone(), twin);
    r.elems = w.elems;
    r.vars = tracked;
    r
}
