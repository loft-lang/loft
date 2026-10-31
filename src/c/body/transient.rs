// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `@FR-R-Transient` in the C: a record or enum value built, read and dropped without being
//! written into a store record is built outside the store — every record a struct in the
//! transient arena (`loft_ta_*`, `loft_rt.h`), every vector of records an arena array, every
//! text and byte payload an arena copy.  The store is untouched; nothing can tell.
//!
//! A TWIN `f__tr<params>[r]` is `f`'s IR written with its transient variables as pointers
//! `lt_<group> *`: the result (`r`: the return buffer becomes a local the twin answers), the
//! parameters the key names (a bit per parameter), and every local of a type those reach.  A
//! ROOT SITE is a call to a producer twin in an ordinary function: its local holds the tree,
//! the arena mark is taken where it is bound and given back where the scope pass frees it.
//!
//! The writer is the allow-list.  An operation on a transient value that no arm here answers
//! is an `Err`, and so is a transient variable reaching the ordinary expression path, so a
//! twin that cannot be written is declined whole (and every twin tried inside it), and a
//! function whose site cannot be written is written again without it.  Opt-in while it is
//! built (`LOFT_TRANSIENT=1`); `LOFT_TRACE_TRANSIENT=1` names each twin and site written and
//! each declined, with the reason.

use super::{E, FnWriter, K, var_name};
use crate::c::{CType, Cx, c_name, held, passed};
use crate::data::{Type, Value};
use crate::database::Parts;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

/// `LOFT_TRANSIENT=1` turns the rule on; it is off by default while it is built (S8 of
/// `bench/portal/analysis/transient.md`), so the default emission is unchanged.
pub(crate) fn enabled() -> bool {
    crate::env_once!(std::env::var("LOFT_TRANSIENT").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_TRANSIENT").is_ok_and(|v| v != "0"))
}

/// One twin: the function, the parameters it takes transient (a bit each) and whether it
/// answers its result transient.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct TrKey {
    pub(crate) d: u32,
    pub(crate) params: u64,
    pub(crate) result: bool,
}

/// What a struct member holds, named by its kind and the field's byte offset in the store.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Member {
    Int,
    Bool,
    Enum,
    Text,
    Vector,
    Record(u16),
}

impl Member {
    fn name(self, off: i64) -> String {
        let p = match self {
            Member::Int => "i",
            Member::Bool => "b",
            Member::Enum => "e",
            Member::Text => "t",
            Member::Vector => "v",
            Member::Record(_) => "r",
        };
        format!("{p}{off}")
    }
}

/// The unit's transient twins and the structs they use.
#[derive(Default)]
pub(crate) struct TrState {
    twins: HashMap<TrKey, Option<String>>,
    order: Vec<TrKey>,
    protos: String,
    bodies: String,
    members: BTreeMap<u16, BTreeSet<(i64, Member)>>,
    zeros: BTreeSet<(u16, i64)>,
    groups: BTreeSet<u16>,
    parents: Option<HashMap<u16, u16>>,
}

/// How a transient variable is held: a record of a group, or a vector of records or scalars.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum TrVar {
    Rec(u16),
    Vec(Elem),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Elem {
    Rec(u16),
    /// A scalar element of this byte size.
    Scalar(u32),
}

impl TrVar {
    fn ctype(self) -> CType {
        match self {
            TrVar::Rec(g) => CType::Tr(g),
            TrVar::Vec(_) => CType::TrVec,
        }
    }
}

/// One function's transient facts, for its writer.
#[derive(Default)]
pub(crate) struct TrFn {
    vars: HashMap<u16, TrVar>,
    /// Hidden buffers handed to a producer twin: never minted, never read.
    dead: HashSet<u16>,
    /// Root locals with their arena-mark variable.
    roots: HashMap<u16, String>,
    /// Locals that may hold no record (an element read past the end).
    nullable: HashSet<u16>,
    /// A twin builds its transient values; a site only reads them.
    builds: bool,
    /// The return buffer a result twin answers instead of taking.
    phantom: Option<u16>,
    /// Set while a producer call is evaluated for a transient local.
    want_result: bool,
}

impl TrFn {
    /// The shapes a site's transient locals are declared in.
    pub(crate) fn shape(&self, shapes: &mut [CType]) {
        for (v, tv) in &self.vars {
            shapes[*v as usize] = tv.ctype();
        }
    }
}

/// A site the writer could not write transient: the function keeps the store form.
pub(crate) fn declined(cx: &Cx, d: u32, why: &str) {
    if trace() {
        eprintln!(
            "[transient] site in {} declined: {why}",
            cx.data.def(d).name()
        );
    }
}

/// A place a transient value lives: a struct (its C lvalue and group) or a vector.
enum Place {
    Rec(String, u16),
    Vec(String, Elem),
}

impl Cx<'_> {
    /// The struct group a record type is held in: an enum variant shares its parent's.
    fn tr_group(&self, tp: u16) -> u16 {
        let mut st = self.tr.borrow_mut();
        let parents = st.parents.get_or_insert_with(|| {
            let mut m = HashMap::new();
            for (i, t) in self.stores.types.iter().enumerate() {
                if let Parts::Enum(vals) = &t.parts {
                    for (v, _) in vals {
                        if *v != u16::MAX {
                            m.insert(*v, u16::try_from(i).unwrap_or(u16::MAX));
                        }
                    }
                }
            }
            m
        });
        let g = parents.get(&tp).copied().unwrap_or(tp);
        st.groups.insert(g);
        g
    }

    fn tr_member(&self, g: u16, off: i64, m: Member) {
        self.tr
            .borrow_mut()
            .members
            .entry(g)
            .or_default()
            .insert((off, m));
    }

    /// Does a new record of `tp` read as the arena's fresh struct — every byte the store's
    /// default walk writes a zero?  A declared default or a nonzero sentinel would read
    /// otherwise, so such a type is never minted in the arena.
    fn tr_zero_default(&self, tp: u16) -> bool {
        self.default_image(tp);
        self.defaults
            .borrow()
            .get(&tp)
            .is_some_and(|b| b.iter().all(|x| *x == 0))
    }

    /// How a variable of loft type `tp` is held transient, when it can be.
    fn tr_var(&self, tp: &Type) -> Option<TrVar> {
        match tp.base() {
            Type::Reference(rd, _) | Type::Enum(rd, true, _) => {
                let kt = self.data.def(*rd).known_type();
                (kt != u16::MAX).then(|| TrVar::Rec(self.tr_group(kt)))
            }
            Type::Vector(elem, _) => match elem.base() {
                Type::Reference(rd, _) | Type::Enum(rd, true, _) => {
                    let kt = self.data.def(*rd).known_type();
                    (kt != u16::MAX).then(|| TrVar::Vec(Elem::Rec(self.tr_group(kt))))
                }
                _ => None,
            },
            _ => None,
        }
    }

    /// The record groups reachable from group `g` through fields, variants and vector
    /// elements.
    fn tr_reach(&self, g: u16, out: &mut BTreeSet<u16>) {
        if !out.insert(g) {
            return;
        }
        let mut next = Vec::new();
        let field_types = |tp: u16, next: &mut Vec<u16>| match &self.stores.types[tp as usize].parts
        {
            Parts::Struct(fs) | Parts::EnumValue(_, fs) => {
                next.extend(fs.iter().map(|f| f.content));
            }
            Parts::Enum(vals) => {
                next.extend(vals.iter().map(|(v, _)| *v).filter(|v| *v != u16::MAX))
            }
            _ => {}
        };
        field_types(g, &mut next);
        let mut i = 0;
        while i < next.len() {
            let c = next[i];
            i += 1;
            match &self.stores.types[c as usize].parts {
                Parts::Struct(_) | Parts::EnumValue(..) | Parts::Enum(_) => {
                    let cg = self.tr_group(c);
                    self.tr_reach(cg, out);
                    // A variant's own fields belong to its parent's struct.
                    if cg != c {
                        field_types(c, &mut next);
                    }
                }
                Parts::Vector(e)
                    if matches!(
                        self.stores.types[*e as usize].parts,
                        Parts::Struct(_) | Parts::EnumValue(..) | Parts::Enum(_)
                    ) =>
                {
                    let eg = self.tr_group(*e);
                    self.tr_reach(eg, out);
                }
                _ => {}
            }
        }
    }

    /// Is group `g` one of `groups`, or the holder record of a vector of one (the store a
    /// collection local is built in, `main_vector<T>`)?
    fn tr_in_groups(&self, v: TrVar, groups: &BTreeSet<u16>) -> bool {
        match v {
            TrVar::Rec(g) => {
                groups.contains(&g)
                    || matches!(&self.stores.types[g as usize].parts, Parts::Struct(fs)
                        if fs.len() == 1
                            && matches!(self.stores.types[fs[0].content as usize].parts,
                                Parts::Vector(e) if groups.contains(&self.tr_group(e))))
            }
            TrVar::Vec(Elem::Rec(g)) => groups.contains(&g),
            TrVar::Vec(Elem::Scalar(_)) => false,
        }
    }

    /// The twin of `key`, written on first ask; `None` when it cannot be written.
    pub(crate) fn tr_twin(&self, key: TrKey) -> Option<String> {
        if let Some(v) = self.tr.borrow().twins.get(&key) {
            return v.clone();
        }
        let name = format!(
            "{}__tr{:x}{}",
            c_name(self.data, key.d),
            key.params,
            if key.result { "r" } else { "" }
        );
        let (protos, bodies, order) = {
            let mut st = self.tr.borrow_mut();
            st.twins.insert(key, Some(name.clone()));
            st.order.push(key);
            (st.protos.len(), st.bodies.len(), st.order.len() - 1)
        };
        match write_twin(self, key, &name) {
            Ok((sig, body)) => {
                let mut st = self.tr.borrow_mut();
                let _ = writeln!(st.protos, "{sig};");
                let _ = write!(st.bodies, "\n{sig} {{\n{body}}}\n");
                if trace() {
                    eprintln!("[transient] twin {name}");
                }
                Some(name)
            }
            Err(why) => {
                if trace() {
                    eprintln!("[transient] {name} declined: {why}");
                }
                let mut st = self.tr.borrow_mut();
                st.protos.truncate(protos);
                st.bodies.truncate(bodies);
                let gone: Vec<TrKey> = st.order.drain(order..).collect();
                for k in gone {
                    st.twins.insert(k, None);
                }
                None
            }
        }
    }

    /// The twins' prototypes, after their structs.
    pub(crate) fn tr_protos(&self) -> String {
        self.tr.borrow().protos.clone()
    }

    /// The twins' bodies.
    pub(crate) fn tr_bodies(&self) -> String {
        self.tr.borrow().bodies.clone()
    }

    /// The struct of every group a twin names: one member per `(offset, kind)` the twins
    /// read or write, a group held by value written before the struct that holds it.
    pub(crate) fn tr_typedefs(&self) -> String {
        let st = self.tr.borrow();
        if st.protos.is_empty() {
            return String::new();
        }
        let mut out = String::new();
        let all: BTreeSet<u16> = st.groups.iter().chain(st.members.keys()).copied().collect();
        for g in &all {
            let _ = writeln!(out, "typedef struct lt_{g} lt_{g};");
        }
        fn emit(g: u16, st: &TrState, done: &mut BTreeSet<u16>, out: &mut String) {
            if !done.insert(g) {
                return;
            }
            let none = BTreeSet::new();
            let ms = st.members.get(&g).unwrap_or(&none);
            for (_, m) in ms {
                if let Member::Record(r) = m {
                    emit(*r, st, done, out);
                }
            }
            let _ = writeln!(out, "struct lt_{g} {{");
            if ms.is_empty() {
                let _ = writeln!(out, "    char lt_none;");
            }
            for (off, m) in ms {
                let t = match m {
                    Member::Int => "int64_t".to_string(),
                    Member::Bool | Member::Enum => "uint8_t".to_string(),
                    Member::Text => "loft_str".to_string(),
                    Member::Vector => "loft_ta_vec".to_string(),
                    Member::Record(r) => format!("lt_{r}"),
                };
                let _ = writeln!(out, "    {t} {};", m.name(*off));
            }
            let _ = writeln!(out, "}};");
        }
        let mut done = BTreeSet::new();
        for g in &all {
            emit(*g, &st, &mut done, &mut out);
        }
        for (g, off) in &st.zeros {
            let none = BTreeSet::new();
            let ms = st.members.get(g).unwrap_or(&none);
            let parts: Vec<String> = ms
                .iter()
                .filter(|(o, _)| o == off)
                .map(|(o, m)| {
                    format!(
                        "memset(&(s)->{}, 0, sizeof((s)->{}));",
                        m.name(*o),
                        m.name(*o)
                    )
                })
                .collect();
            let _ = writeln!(
                out,
                "#define LT_ZERO_{g}_{off}(s) do {{ {} }} while (0)",
                parts.join(" ")
            );
        }
        out
    }
}

/// Hidden buffers `code` hands to a function answering a record of `groups`: such a call
/// goes to its producer twin, so the buffer is never minted or read.
fn dead_buffers(cx: &Cx, code: &Value, groups: &BTreeSet<u16>) -> HashSet<u16> {
    let mut dead = HashSet::new();
    code.walk(&mut |n| {
        if let Value::Call(f, args) = n
            && let Some(b) = producer_buffer(cx, *f, args)
            && cx
                .tr_var(cx.data.def(*f).returned())
                .is_some_and(|v| matches!(v, TrVar::Rec(g) if groups.contains(&g)))
        {
            dead.insert(b);
        }
    });
    dead
}

/// The buffer variable a call to a loft function returning a record hands it.
fn producer_buffer(cx: &Cx, f: u32, args: &[Value]) -> Option<u16> {
    if (f as usize) >= cx.data.definitions.len() {
        return None;
    }
    let def = cx.data.def(f);
    if !def.name().starts_with("n_") || matches!(def.code(), Value::Null) {
        return None;
    }
    let k = def.hidden_return_buffer_attr()?;
    match args.get(k)?.unspan() {
        Value::Var(b) => Some(*b),
        _ => None,
    }
}

/// Write twin `key` as `name`: its signature and body.
fn write_twin(cx: &Cx, key: TrKey, name: &str) -> Result<(String, String), String> {
    let data = cx.data;
    let def = data.def(key.d);
    if matches!(def.code(), Value::Null) || !def.name().starts_with("n_") {
        return Err("not a loft function".into());
    }
    if matches!(def.returned().base(), Type::Iterator(..)) {
        return Err("a generator".into());
    }
    let vars = def.variables();
    let args = vars.arguments();
    if args.len() != def.attributes().len() || args.len() > 63 {
        return Err("parameters that do not match their arguments".into());
    }
    let mut shapes = crate::c::variable_shapes(cx, vars, &args, data.body_width32(key.d))?;
    let mut tf = TrFn {
        builds: true,
        ..TrFn::default()
    };
    let mut groups = BTreeSet::new();
    let mut ret = cx
        .ctype(def.returned())
        .ok_or("a result the C has no shape for")?;
    let mut phantom_at = None;
    if key.result {
        let i = def
            .hidden_return_buffer_attr()
            .ok_or("a transient result with no return buffer")?;
        let b = args[i];
        let Some(TrVar::Rec(g)) = cx.tr_var(vars.tp(b)) else {
            return Err("a transient result that is not a record".into());
        };
        tf.vars.insert(b, TrVar::Rec(g));
        tf.phantom = Some(b);
        phantom_at = Some(i);
        ret = CType::Tr(g);
        cx.tr_reach(g, &mut groups);
    }
    for (i, &a) in args.iter().enumerate() {
        if key.params >> i & 1 == 1 {
            let tv = cx
                .tr_var(vars.tp(a))
                .ok_or("a transient parameter that is not a record")?;
            if let TrVar::Rec(g) | TrVar::Vec(Elem::Rec(g)) = tv {
                cx.tr_reach(g, &mut groups);
            }
            tf.vars.insert(a, tv);
        }
    }
    tf.dead = dead_buffers(cx, def.code(), &groups);
    for v in 0..vars.count() {
        if vars.is_argument(v) || tf.dead.contains(&v) {
            continue;
        }
        if let Some(tv) = cx.tr_var(vars.tp(v))
            && cx.tr_in_groups(tv, &groups)
        {
            tf.vars.insert(v, tv);
        }
    }
    for (v, tv) in &tf.vars {
        shapes[*v as usize] = tv.ctype();
    }
    if trace() {
        let mut vs: Vec<String> = tf
            .vars
            .iter()
            .map(|(v, t)| format!("{}:{t:?}", vars.name(*v)))
            .collect();
        vs.sort();
        let mut ds: Vec<&str> = tf.dead.iter().map(|v| vars.name(*v)).collect();
        ds.sort_unstable();
        eprintln!("[transient] {name}: groups {groups:?} vars {vs:?} dead {ds:?}");
    }
    let params: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(i, _)| phantom_at != Some(*i))
        .map(|(_, &a)| {
            let c = shapes[a as usize];
            let n = var_name(vars.name(a), a);
            if matches!(vars.tp(a).base(), Type::RefVar(_)) {
                format!("{} *{n}", held(c))
            } else {
                format!("{} {n}", passed(c))
            }
        })
        .collect();
    let params = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };
    let sig = format!("static {} {name}({params})", held(ret));
    let (body, _) = FnWriter::new(cx, key.d, shapes, ret)
        .transient(tf)
        .write()?;
    Ok((sig, body))
}

/// The site plan of ordinary function `d`: its one root local — bound from a call to a
/// function whose producer twin can be written — and the locals derived from it.
pub(crate) fn site_plan(cx: &Cx, d: u32) -> Option<TrFn> {
    if !enabled() {
        return None;
    }
    let def = cx.data.def(d);
    let code = def.code();
    let mut roots: Vec<(u16, u16)> = Vec::new();
    let mut calls: Vec<u32> = Vec::new();
    code.walk(&mut |n| {
        if let Value::Set(v, x) = n
            && let Value::Call(f, args) = x.unspan()
            && let Some(b) = producer_buffer(cx, *f, args)
            && matches!(cx.tr_var(cx.data.def(*f).returned()), Some(TrVar::Rec(_)))
            && args
                .iter()
                .all(|a| !a.any_node(&mut |m| matches!(m, Value::Var(w) if *w == *v)))
        {
            if !roots.contains(&(*v, b)) {
                roots.push((*v, b));
            }
            calls.push(*f);
        }
    });
    let root = roots.first()?.0;
    if roots.iter().any(|(v, _)| *v != root) {
        return None;
    }
    for f in &calls {
        cx.tr_twin(TrKey {
            d: *f,
            params: 0,
            result: true,
        })?;
    }
    let vars = def.variables();
    let mut tf = TrFn::default();
    tf.vars.insert(root, cx.tr_var(vars.tp(root))?);
    tf.roots.insert(root, format!("loft_tm{root}"));
    tf.dead = roots.iter().map(|(_, b)| *b).collect();
    // The locals bound from a projection, an element or an alias of a transient local.
    loop {
        let mut grew = false;
        code.walk(&mut |n| {
            if let Value::Set(v, x) = n
                && !tf.vars.contains_key(v)
                && derives(cx, x, &tf.vars)
                && let Some(tv) = cx.tr_var(vars.tp(*v))
            {
                tf.vars.insert(*v, tv);
                grew = true;
            }
        });
        if !grew {
            break;
        }
    }
    Some(tf)
}

/// Does `x` answer a part of a transient value — a field, an element, an alias?
fn derives(cx: &Cx, x: &Value, vars: &HashMap<u16, TrVar>) -> bool {
    match x.unspan() {
        Value::Var(w) => vars.contains_key(w),
        Value::Call(op, a) if (*op as usize) < cx.data.definitions.len() => {
            matches!(
                cx.data.def(*op).name(),
                "OpGetField" | "OpGetVectorNullable" | "OpRefAlias"
            ) && a.first().is_some_and(|b| derives(cx, b, vars))
        }
        _ => false,
    }
}

impl FnWriter<'_> {
    /// Write this body with transient facts `tf`: a twin or a site.
    pub(crate) fn transient(mut self, tf: TrFn) -> Self {
        if let Some(p) = tf.phantom {
            self.params[p as usize] = false;
        }
        for r in tf.roots.values() {
            self.decls.push(format!("loft_ta_mark_t {r};"));
        }
        self.vr = None;
        self.adopt = None;
        self.tr = Some(Box::new(tf));
        self
    }

    pub(super) fn tr_is(&self, v: u16) -> bool {
        self.tr.as_ref().is_some_and(|t| t.vars.contains_key(&v))
    }

    pub(super) fn tr_dead(&self, v: u16) -> bool {
        self.tr.as_ref().is_some_and(|t| t.dead.contains(&v))
    }

    fn tr_builds(&self) -> bool {
        self.tr.as_ref().is_some_and(|t| t.builds)
    }

    /// The transient variable `v` is rooted at — a variable, or the base of a field or element
    /// read — when it is one.
    fn tr_root(&self, v: &Value) -> Option<u16> {
        match v.unspan() {
            Value::Var(w) if self.tr_is(*w) => Some(*w),
            Value::Call(op, a)
                if matches!(
                    self.cx.data.def(*op).name(),
                    "OpGetField" | "OpGetVectorNullable"
                ) =>
            {
                a.first().and_then(|b| self.tr_root(b))
            }
            Value::Block(b) => b.operators.last().and_then(|t| self.tr_root(t)),
            _ => None,
        }
    }

    /// The type a field read hands back (`OpGetField`'s third operand).
    fn tr_field_type(x: &Value) -> Result<u16, String> {
        match x.unspan() {
            Value::Int(t) => u16::try_from(*t).map_err(|_| "a field type out of range".into()),
            _ => Err("a field read of no known type".into()),
        }
    }

    /// Where transient value `v` lives.
    fn tr_place(&mut self, v: &Value, ind: usize) -> Result<Place, String> {
        match v.unspan() {
            Value::Var(w) => {
                let tv = *self
                    .tr
                    .as_ref()
                    .and_then(|t| t.vars.get(w))
                    .ok_or("a store value where a transient one was meant")?;
                let n = self.var(*w);
                Ok(match tv {
                    TrVar::Rec(g) => Place::Rec(format!("(*{n})"), g),
                    TrVar::Vec(e) => Place::Vec(format!("(*{n})"), e),
                })
            }
            Value::Block(b) => {
                let n = b.operators.len();
                for op in &b.operators[..n.saturating_sub(1)] {
                    self.stmt(op, ind)?;
                }
                self.tr_place(b.operators.last().ok_or("an empty block")?, ind)
            }
            Value::Call(op, a) if self.cx.data.def(*op).name() == "OpGetField" => {
                let [base, off, tp] = a.as_slice() else {
                    return Err("a field read of another shape".into());
                };
                let Value::Int(off) = off.unspan() else {
                    return Err("a field at a computed offset".into());
                };
                let off = i64::from(*off);
                let tp = Self::tr_field_type(tp)?;
                let Place::Rec(lv, g) = self.tr_place(base, ind)? else {
                    return Err("a field of a vector".into());
                };
                match &self.cx.stores.types[tp as usize].parts {
                    Parts::Vector(e) => {
                        let e = *e;
                        let elem = match &self.cx.stores.types[e as usize].parts {
                            Parts::Struct(_) | Parts::EnumValue(..) | Parts::Enum(_) => {
                                Elem::Rec(self.cx.tr_group(e))
                            }
                            _ => Elem::Scalar(u32::from(self.cx.stores.size(e))),
                        };
                        self.cx.tr_member(g, off, Member::Vector);
                        Ok(Place::Vec(
                            format!("{lv}.{}", Member::Vector.name(off)),
                            elem,
                        ))
                    }
                    Parts::Struct(_) | Parts::EnumValue(..) | Parts::Enum(_) => {
                        let r = self.cx.tr_group(tp);
                        self.cx.tr_member(g, off, Member::Record(r));
                        Ok(Place::Rec(
                            format!("{lv}.{}", Member::Record(r).name(off)),
                            r,
                        ))
                    }
                    _ => Err("a field of a kind the transient form does not hold".into()),
                }
            }
            _ => Err("a transient value of another shape".into()),
        }
    }

    /// A pointer to the struct transient value `v` names: `&place`, or the variable itself.
    fn tr_ptr(&mut self, v: &Value, ind: usize) -> Result<String, String> {
        if let Value::Var(w) = v.unspan()
            && self.tr_is(*w)
        {
            return Ok(self.var(*w));
        }
        if let Value::Call(f, _) = v.unspan()
            && !super::is_operator(self.cx.data, *f)
        {
            if let Some(t) = self.tr.as_mut() {
                t.want_result = true;
            }
            let e = self.expr(v, ind);
            if let Some(t) = self.tr.as_mut() {
                t.want_result = false;
            }
            return Ok(e?.code);
        }
        match self.tr_place(v, ind)? {
            Place::Rec(lv, _) | Place::Vec(lv, _) => Ok(format!("&{lv}")),
        }
    }

    /// The null a getter of `m` answers through a missing record.
    fn tr_null(m: Member) -> &'static str {
        match m {
            Member::Int => "LOFT_NULL",
            Member::Bool => "LOFT_BOOL_NULL",
            Member::Enum => "(uint8_t) 255",
            _ => "",
        }
    }

    /// The operators a transient value changes, or `None` for one that names none.
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per operator, as `vr_operator`"
    )]
    pub(super) fn tr_operator(
        &mut self,
        name: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        let var_of = |a: Option<&Value>| match a.map(Value::unspan) {
            Some(Value::Var(v)) => Some(*v),
            _ => None,
        };
        let first = var_of(args.first());
        let void = || Ok(Some(E::new("((void) 0)", K::Void)));
        let dead_or_tr = |s: &Self, v: Option<u16>| v.is_some_and(|v| s.tr_dead(v) || s.tr_is(v));
        match name {
            "OpFreeRef" | "OpFreeRefIfDistinct" | "OpClear" | "OpFreeRecordIn"
                if dead_or_tr(self, first) =>
            {
                let v = first.unwrap_or(u16::MAX);
                if name != "OpClear"
                    && let Some(m) = self.tr.as_ref().and_then(|t| t.roots.get(&v)).cloned()
                {
                    self.line(ind, &format!("loft_ta_back({m});"));
                }
                void()
            }
            // A free guarded against a transient value: the two are always distinct.
            "OpFreeRefIfDistinct" if dead_or_tr(self, var_of(args.get(1))) => {
                Ok(Some(self.operator("OpFreeRef", &args[..1], ind)?))
            }
            "OpDistinctStore" if args.iter().any(|a| dead_or_tr(self, var_of(Some(a)))) => {
                Ok(Some(E::new("1", K::Cond)))
            }
            "OpRefIsNull" | "OpConvBoolFromRef" if args.len() == 1 => {
                let Some(v) = first else { return Ok(None) };
                if self.tr_dead(v) {
                    return Ok(Some(E::new("1", K::Cond)));
                }
                if !self.tr_is(v) {
                    return Ok(None);
                }
                let op = if name == "OpRefIsNull" { "==" } else { "!=" };
                Ok(Some(E::new(format!("({} {op} 0)", self.var(v)), K::Cond)))
            }
            "OpDatabase" | "OpDatabaseNP" if dead_or_tr(self, first) => {
                let v = first.unwrap_or(u16::MAX);
                if self.tr_dead(v) {
                    return void();
                }
                if !self.tr_builds() {
                    return Err("a site that mints into a transient value".into());
                }
                let Some(Value::Int(tp)) = args.get(1).map(Value::unspan) else {
                    return Err("a mint of no known type".into());
                };
                let tp = u16::try_from(*tp).map_err(|_| "a type out of range")?;
                if !self.cx.tr_zero_default(tp) {
                    return Err("a record whose default is not all zeros".into());
                }
                let g = self.cx.tr_group(tp);
                let n = self.var(v);
                self.line(
                    ind,
                    &format!("{n} = (lt_{g} *) loft_ta_alloc(sizeof(lt_{g}));"),
                );
                void()
            }
            "OpGetInt" | "OpGetBoolean" | "OpGetEnum" | "OpGetText"
                if args.first().and_then(|b| self.tr_root(b)).is_some() =>
            {
                let [base, off] = args else {
                    return Err("a getter of another shape".into());
                };
                let Value::Int(off) = off.unspan() else {
                    return Err("a getter at a computed offset".into());
                };
                let off = i64::from(*off);
                let (m, k) = match name {
                    "OpGetInt" => (Member::Int, K::Int),
                    "OpGetBoolean" => (Member::Bool, K::Bool),
                    "OpGetEnum" => (Member::Enum, K::Enum),
                    _ => (Member::Text, K::View),
                };
                let root = self.tr_root(base).unwrap_or(u16::MAX);
                let Place::Rec(lv, g) = self.tr_place(base, ind)? else {
                    return Err("a getter of a vector".into());
                };
                self.cx.tr_member(g, off, m);
                let read = format!("{lv}.{}", m.name(off));
                let nullable = self.tr.as_ref().is_some_and(|t| t.nullable.contains(&root));
                Ok(Some(if nullable {
                    let r = self.var(root);
                    let null = if m == Member::Text {
                        "((loft_str) { 0, 0 })".to_string()
                    } else {
                        Self::tr_null(m).to_string()
                    };
                    E::new(format!("({r} ? {read} : {null})"), k)
                } else {
                    E::new(read, k)
                }))
            }
            "OpLengthVector" if args.first().and_then(|b| self.tr_root(b)).is_some() => {
                let Place::Vec(lv, _) = self.tr_place(&args[0], ind)? else {
                    return Err("the length of a record".into());
                };
                Ok(Some(E::new(format!("((int64_t) ({lv}).n)"), K::Int)))
            }
            "OpSetInt" | "OpSetBoolean" | "OpSetEnum" | "OpSetText" | "OpSetInt4"
                if args.first().and_then(|b| self.tr_root(b)).is_some() =>
            {
                if !self.tr_builds() {
                    return Err("a site that writes a transient value".into());
                }
                let [base, off, val] = args else {
                    return Err("a setter of another shape".into());
                };
                let Value::Int(off) = off.unspan() else {
                    return Err("a setter at a computed offset".into());
                };
                let off = i64::from(*off);
                let Place::Rec(lv, g) = self.tr_place(base, ind)? else {
                    return Err("a setter of a vector".into());
                };
                if name == "OpSetInt4" {
                    if !matches!(val.unspan(), Value::Int(0)) {
                        return Err("a narrow field written".into());
                    }
                    self.cx.tr.borrow_mut().zeros.insert((g, off));
                    self.line(ind, &format!("LT_ZERO_{g}_{off}(&{lv});"));
                    return void();
                }
                let m = match name {
                    "OpSetInt" => Member::Int,
                    "OpSetBoolean" => Member::Bool,
                    "OpSetEnum" => Member::Enum,
                    _ => Member::Text,
                };
                self.cx.tr_member(g, off, m);
                let e = self.expr(val, ind)?;
                let x = match m {
                    Member::Int => super::as_int(e)?,
                    Member::Bool => super::as_bool(e)?,
                    Member::Enum => super::as_enum(e)?,
                    _ => format!("loft_ta_str({})", super::as_view(e)?),
                };
                self.line(ind, &format!("{lv}.{} = {x};", m.name(off)));
                self.flush(ind);
                void()
            }
            "OpCopyRecord" | "OpMoveField"
                if args.iter().take(2).any(|a| self.tr_root(a).is_some()) =>
            {
                if !self.tr_builds() {
                    return Err("a site that copies a transient value".into());
                }
                let [src, dst, ..] = args else {
                    return Err("a record copy of another shape".into());
                };
                if self.tr_root(src).is_none() || self.tr_root(dst).is_none() {
                    return Err("a record copied between a store and a transient value".into());
                }
                let s = self.tr_ptr(src, ind)?;
                let Place::Rec(d, _) = self.tr_place(dst, ind)? else {
                    return Err("a record copied into a vector".into());
                };
                self.line(ind, &format!("{d} = *{s};"));
                void()
            }
            "OpMoveVector" if args.first().and_then(|b| self.tr_root(b)).is_some() => {
                if !self.tr_builds() {
                    return Err("a site that writes a transient vector".into());
                }
                let [dst, src, ..] = args else {
                    return Err("a vector move of another shape".into());
                };
                let Place::Vec(d, _) = self.tr_place(dst, ind)? else {
                    return Err("a vector moved into a record".into());
                };
                let Place::Vec(s, _) = self.tr_place(src, ind)? else {
                    return Err("a record moved into a vector".into());
                };
                self.line(ind, &format!("{d} = {s};"));
                void()
            }
            "OpAppendVector" if args.first().and_then(|b| self.tr_root(b)).is_some() => {
                if !self.tr_builds() {
                    return Err("a site that appends to a transient vector".into());
                }
                let [dst, src, ..] = args else {
                    return Err("an append of another shape".into());
                };
                let Place::Vec(d, Elem::Scalar(size)) = self.tr_place(dst, ind)? else {
                    return Err("an append of records".into());
                };
                if self.tr_root(src).is_some() {
                    return Err("a transient vector appended".into());
                }
                let e = self.expr(src, ind)?;
                let s = super::as_ref(e)?;
                self.line(ind, &format!("loft_ta_vec_copy(&{d}, {s}, {size});"));
                self.flush(ind);
                void()
            }
            "OpPreAllocVector" | "OpFinishRecord"
                if args.first().and_then(|b| self.tr_root(b)).is_some() =>
            {
                void()
            }
            _ => {
                // Any other operator handed a transient value would read it as a store
                // reference.
                if args.iter().any(|a| {
                    self.tr_root(a).is_some() || var_of(Some(a)).is_some_and(|v| self.tr_dead(v))
                }) {
                    return Err(format!("{name} of a transient value"));
                }
                Ok(None)
            }
        }
    }

    /// `var = x` for a transient or dead `var`; `None` for any other.
    #[expect(clippy::too_many_lines, reason = "one arm per binding form")]
    pub(super) fn tr_set(&mut self, var: u16, x: &Value, ind: usize) -> Option<Result<(), String>> {
        if self.tr_dead(var) {
            return Some(match x.unspan() {
                Value::Null => Ok(()),
                Value::Call(op, _)
                    if matches!(
                        self.cx.data.def(*op).name(),
                        "OpPlaceRecord" | "OpDatabase" | "OpNullRefSentinel"
                    ) =>
                {
                    Ok(())
                }
                _ => Err("a dead buffer bound to a value".into()),
            });
        }
        let tv = *self.tr.as_ref()?.vars.get(&var)?;
        Some((|| {
            let lhs = self.var(var);
            let data = self.cx.data;
            match x.unspan() {
                Value::Null => {
                    self.line(ind, &format!("{lhs} = 0;"));
                    return Ok(());
                }
                Value::Var(src) if self.tr_is(*src) => {
                    let src = self.var(*src);
                    self.line(ind, &format!("{lhs} = {src};"));
                    return Ok(());
                }
                Value::Call(op, cargs) if super::is_operator(data, *op) => {
                    let opn = data.def(*op).name();
                    match (opn, tv) {
                        ("OpGetField", _) => {
                            let ptr = self.tr_ptr(x, ind)?;
                            self.line(ind, &format!("{lhs} = {ptr};"));
                        }
                        ("OpRefAlias", _) => {
                            let ptr = self.tr_ptr(&cargs[0], ind)?;
                            self.line(ind, &format!("{lhs} = {ptr};"));
                        }
                        ("OpGetVectorNullable", TrVar::Rec(g)) => {
                            let Place::Vec(lv, _) = self.tr_place(&cargs[0], ind)? else {
                                return Err("an element of a record".into());
                            };
                            let idx = self.expr(&cargs[2], ind)?;
                            let idx = super::as_int(idx)?;
                            self.line(
                                ind,
                                &format!(
                                    "{lhs} = (lt_{g} *) loft_ta_vec_at(&{lv}, {idx}, sizeof(lt_{g}));"
                                ),
                            );
                            if let Some(t) = self.tr.as_mut() {
                                t.nullable.insert(var);
                            }
                        }
                        ("OpNewRecord", TrVar::Rec(g)) => {
                            if !self.tr_builds() {
                                return Err("a site that adds to a transient vector".into());
                            }
                            let Place::Vec(lv, _) = self.tr_place(&cargs[0], ind)? else {
                                return Err("an element of a record".into());
                            };
                            self.line(
                                ind,
                                &format!(
                                    "{lhs} = (lt_{g} *) loft_ta_vec_push(&{lv}, sizeof(lt_{g}));"
                                ),
                            );
                        }
                        ("OpPlaceRecord", TrVar::Rec(g)) => {
                            if !self.tr_builds() {
                                return Err("a site that places into a transient value".into());
                            }
                            self.line(
                                ind,
                                &format!("{lhs} = (lt_{g} *) loft_ta_alloc(sizeof(lt_{g}));"),
                            );
                        }
                        (opn, _) => return Err(format!("a transient local bound from {opn}")),
                    }
                    return Ok(());
                }
                Value::Call(..) => {
                    if let Some(m) = self.tr.as_ref().and_then(|t| t.roots.get(&var)).cloned() {
                        self.line(ind, &format!("{m} = loft_ta_mark();"));
                    }
                    let ptr = self.tr_ptr(x, ind)?;
                    self.line(ind, &format!("{lhs} = {ptr};"));
                    self.flush(ind);
                    return Ok(());
                }
                Value::Block(blk) => {
                    let k = blk.operators.len();
                    for op in &blk.operators[..k.saturating_sub(1)] {
                        self.stmt(op, ind)?;
                    }
                    let tail = blk.operators.last().ok_or("an empty block")?;
                    return match self.tr_set(var, tail, ind) {
                        Some(r) => r,
                        None => Err("a block of another shape".into()),
                    };
                }
                _ => {}
            }
            Err("a transient local bound from a value of another shape".into())
        })())
    }

    /// The arguments a call hands transient values: each a pointer into `xs`, the call going
    /// to the callee's twin.  Answers the twin's name with the positions left to the ordinary
    /// values, and the return buffer it takes no more when it answers its result transient.
    pub(super) fn tr_args(
        &mut self,
        d: u32,
        args: &[Value],
        values: Vec<usize>,
        xs: &mut [String],
        ind: usize,
    ) -> Result<(Option<String>, Vec<usize>, Option<usize>), String> {
        let Some(t) = self.tr.as_mut() else {
            return Ok((None, values, None));
        };
        let result = std::mem::take(&mut t.want_result);
        let mut params = 0u64;
        for (i, a) in args.iter().enumerate() {
            if self.tr_root(a).is_some() {
                params |= 1 << i;
            }
        }
        if params == 0 && !result {
            return Ok((None, values, None));
        }
        let key = TrKey { d, params, result };
        if trace() {
            eprintln!(
                "[transient] {} asks {} params {params:#x} result {result}: {}",
                self.cx.data.def(self.d).name(),
                self.cx.data.def(d).name(),
                args.iter()
                    .map(|a| match a.unspan() {
                        Value::Var(v) => format!(
                            "{}{}{}",
                            self.vars.name(*v),
                            if self.tr_is(*v) { "(tr)" } else { "" },
                            if self.tr_dead(*v) { "(dead)" } else { "" }
                        ),
                        _ => "_".to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        let name = self
            .cx
            .tr_twin(key)
            .ok_or_else(|| format!("no transient twin of {}", self.cx.data.def(d).name()))?;
        for (i, a) in args.iter().enumerate() {
            if params >> i & 1 == 1 {
                xs[i] = self.tr_ptr(a, ind)?;
            }
        }
        let dropped = if result {
            self.cx.data.def(d).hidden_return_buffer_attr()
        } else {
            None
        };
        let values = values
            .into_iter()
            .filter(|i| params >> i & 1 == 0 && dropped != Some(*i))
            .collect();
        Ok((Some(name), values, dropped))
    }

    /// `return v` from a twin answering its result transient.
    pub(super) fn tr_return(&mut self, v: &Value, ind: usize) -> Result<(), String> {
        let p = self.tr_ptr(v, ind)?;
        let r = self.temp(self.ret);
        self.line(ind, &format!("{r} = {p};"));
        self.ret_held(&r, ind);
        Ok(())
    }
}
