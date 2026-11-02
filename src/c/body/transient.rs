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
//! function whose site cannot be written is written again without it.  A twin writes only the
//! records it builds itself (`TrFn::owned`), and a record copy seals its vectors (`lt_seal_*`):
//! a transient tree may share records where the store form copied them, so nothing reachable
//! from another tree is written.  `LOFT_NO_TRANSIENT=1` keeps every value in the store;
//! `LOFT_TRACE_TRANSIENT=1` names each twin and site written and each declined, with the reason.

use super::{E, FnWriter, K, var_name};
use crate::c::{CType, Cx, c_name, held, passed};
use crate::data::{Type, Value};
use crate::database::Parts;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write as _;

/// `LOFT_NO_TRANSIENT=1` turns the rule off: the same-build oracle of every A/B and the first
/// bisect step for a wrong answer out of a value read and dropped.
pub(crate) fn enabled() -> bool {
    !crate::env_once!(std::env::var("LOFT_NO_TRANSIENT").is_ok_and(|v| v != "0"))
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
            // Two variants may hold different records at one offset: the group names it.
            Member::Record(g) => return format!("r{off}_{g}"),
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
    /// The groups a record copy seals (`lt_seal_<g>`): each vector member's room cut to its
    /// length, so a push into the copy moves it rather than writing past the source's end.
    seals: BTreeSet<u16>,
    /// Each function asked for a `__wr` twin, and whether it was written.
    work_twins: HashMap<u32, bool>,
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
    /// The transient locals this twin builds itself — every binding a mint, or its result
    /// buffer.  Only these are written: a parameter, a call's answer, an element or an alias
    /// may share its records with a tree the store form would have copied (`pa_get` answers
    /// a pointer into its argument), so a write through one could reach that tree.
    owned: HashSet<u16>,
    /// Text locals served by another text: a lifted argument by the transient text it copies
    /// (`true`), a work-buffer call's result by the buffer (`false`).
    text_lifts: Option<HashMap<u16, (Value, bool)>>,
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
    /// A record of scalars and texts alone — the element a vector producer may build in the
    /// arena (zttext's `Run`); one that holds a vector or a record keeps the store form.
    fn tr_flat(&self, g: u16) -> bool {
        match &self.stores.types[g as usize].parts {
            Parts::Struct(fs) => fs.iter().all(|f| {
                !matches!(
                    self.stores.types[f.content as usize].parts,
                    Parts::Vector(_) | Parts::Struct(_) | Parts::EnumValue(..) | Parts::Enum(_)
                )
            }),
            _ => false,
        }
    }

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
            Ok((sig, body, inner)) => {
                let mut st = self.tr.borrow_mut();
                if let Some(inner) = inner {
                    let _ = writeln!(st.protos, "{inner};");
                }
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
        // A sealed copy's nested records are sealed with it.
        let mut seals: BTreeSet<u16> = BTreeSet::new();
        let mut todo: Vec<u16> = st.seals.iter().copied().collect();
        while let Some(g) = todo.pop() {
            if seals.insert(g) {
                for (_, m) in st.members.get(&g).into_iter().flatten() {
                    if let Member::Record(r) = m {
                        todo.push(*r);
                    }
                }
            }
        }
        for g in &seals {
            let _ = writeln!(out, "static inline void lt_seal_{g}(lt_{g} *s);");
        }
        for g in &seals {
            let none = BTreeSet::new();
            let body: Vec<String> = st
                .members
                .get(g)
                .unwrap_or(&none)
                .iter()
                .filter_map(|(o, m)| match m {
                    Member::Vector => Some(format!("s->{n}.cap = s->{n}.n;", n = m.name(*o))),
                    Member::Record(r) => Some(format!("lt_seal_{r}(&s->{});", m.name(*o))),
                    _ => None,
                })
                .collect();
            let _ = writeln!(
                out,
                "static inline void lt_seal_{g}(lt_{g} *s) {{ (void) s; {} }}",
                body.join(" ")
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
#[expect(
    clippy::too_many_lines,
    reason = "one twin: its result, its parameters, its locals' forms, its owned set, its signature"
)]
fn write_twin(cx: &Cx, key: TrKey, name: &str) -> Result<(String, String, Option<String>), String> {
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
    // `@FR-R-ValueRecord` holds a record result as a struct already, and the twin's body is the
    // ordinary writer's: its value-record locals and pointer parameters keep their shapes.
    if cx.value_records().fns.contains_key(&key.d) {
        return Err("a value-record result".into());
    }
    let mut shapes = crate::c::variable_shapes(cx, vars, &args, data.body_width32(key.d))?;
    let vr_locals = cx.vr_locals(key.d);
    let mut vr_sorted: Vec<(u16, u16)> = vr_locals.iter().map(|(v, k)| (*v, *k)).collect();
    vr_sorted.sort_unstable();
    for (v, k) in vr_sorted {
        if let Some(c) = cx.vr_ctype(k)
            && (v as usize) < shapes.len()
        {
            shapes[v as usize] = c;
        }
    }
    let vr_ptr = cx.vr_ptr_params(key.d);
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
        let tv = cx.tr_var(vars.tp(b));
        let Some(TrVar::Rec(g) | TrVar::Vec(Elem::Rec(g))) = tv else {
            return Err("a transient result that is not a record".into());
        };
        let tv = tv.ok_or("a transient result of no shape")?;
        tf.vars.insert(b, tv);
        tf.phantom = Some(b);
        phantom_at = Some(i);
        // A vector answered through its own record (`main_vector<T>`): the twin answers the
        // vector itself, that record's field 0.
        ret = if matches!(cx.tr_var(def.returned()), Some(TrVar::Vec(Elem::Rec(_)))) {
            CType::TrVec
        } else {
            tv.ctype()
        };
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
    let mut bindings: Vec<(u16, &Value)> = Vec::new();
    def.code().walk(&mut |n| {
        if let Value::Set(v, x) = n {
            bindings.push((*v, x));
        }
    });
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
    if let Some(v) = tf.vars.keys().find(|v| vr_locals.contains_key(v)) {
        return Err(format!("`{}` a value record", vars.name(*v)));
    }
    for (v, tv) in &tf.vars {
        shapes[*v as usize] = tv.ctype();
    }
    // Owned: every binding a mint, or a field of an owned local (the same memory) — a fixpoint
    // that only removes.
    tf.owned = tf
        .vars
        .keys()
        .copied()
        .filter(|v| !vars.is_argument(*v) || tf.phantom == Some(*v))
        .collect();
    loop {
        let out: Vec<u16> = bindings
            .iter()
            .filter(|(v, x)| tf.owned.contains(v) && !is_mint(cx, x) && !field_of(cx, x, &tf.owned))
            .map(|(v, _)| *v)
            .collect();
        if out.is_empty() {
            break;
        }
        for v in out {
            tf.owned.remove(&v);
        }
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
        .map(|(i, &a)| {
            let c = shapes[a as usize];
            let n = var_name(vars.name(a), a);
            if vars.is_store_text_link(a) {
                format!("loft_ref {n}")
            } else if matches!(vars.tp(a).base(), Type::RefVar(_)) {
                format!("{} *{n}", held(c))
            } else if vr_ptr.contains(&i) {
                format!("const {} *{n}", passed(c))
            } else {
                format!("{} {n}", passed(c))
            }
        })
        .collect();
    let call_args: Vec<String> = args
        .iter()
        .enumerate()
        .filter(|(i, _)| phantom_at != Some(*i))
        .map(|(_, &a)| var_name(vars.name(a), a))
        .collect();
    let params = if params.is_empty() {
        "void".to_string()
    } else {
        params.join(", ")
    };
    let rtype = held(ret);
    let sig = format!("static {rtype} {name}({params})");
    let (body, _) = FnWriter::new(cx, key.d, shapes, ret)
        .transient(tf)
        .write()?;
    // A twin that holds texts for a halt (`LOFT_HOLD`) gives them back on its way out, as the
    // ordinary function's wrapper does (`LOFT_HOLD_MARK` / `LOFT_HOLD_BACK`): left on the held
    // list, a halt after the twin returned freed texts in a frame that no longer exists (a
    // stack-use-after-scope under ASan, the halted-test C cell).
    if body.contains("LOFT_HOLD(") {
        let inner = format!("{name}__in");
        let call = format!("{inner}({})", call_args.join(", "));
        let wrapper = if rtype == "void" {
            format!("    LOFT_HOLD_MARK(hm);\n    {call};\n    LOFT_HOLD_BACK(hm);\n")
        } else {
            format!(
                "    LOFT_HOLD_MARK(hm);\n    {rtype} r = {call};\n    LOFT_HOLD_BACK(hm);\n    return r;\n"
            )
        };
        let inner_sig = format!("static {rtype} {inner}({params})");
        return Ok((
            sig,
            format!("{wrapper}}}\n\n{inner_sig} {{\n{body}"),
            Some(inner_sig),
        ));
    }
    Ok((sig, body, None))
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
            && match cx.tr_var(cx.data.def(*f).returned()) {
                Some(TrVar::Rec(_)) => true,
                Some(TrVar::Vec(Elem::Rec(g))) => cx.tr_flat(g),
                _ => false,
            }
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
    roots.first()?;
    let mut root_vars: Vec<u16> = roots.iter().map(|(v, _)| *v).collect();
    root_vars.sort_unstable();
    root_vars.dedup();
    let vars = def.variables();
    let mut tf = TrFn::default();
    // One arena mark for the frame, taken at its entry: every root's bind and free give it
    // back (`lives`).
    for &root in &root_vars {
        tf.vars.insert(root, cx.tr_var(vars.tp(root))?);
        tf.roots.insert(root, "loft_tmf".to_string());
    }
    tf.dead = roots.iter().map(|(_, b)| *b).collect();
    // The locals bound from a projection, an element or an alias of a transient local, each
    // with the root it reads.
    let mut owner: HashMap<u16, u16> = root_vars.iter().map(|r| (*r, *r)).collect();
    loop {
        let mut grew = false;
        code.walk(&mut |n| {
            if let Value::Set(v, x) = n
                && !tf.vars.contains_key(v)
                && derives(cx, x, &tf.vars)
                && let Some(tv) = cx.tr_var(vars.tp(*v))
            {
                let mut from = None;
                x.walk(&mut |m| {
                    if from.is_none()
                        && let Value::Var(w) = m
                    {
                        from = owner.get(w).copied();
                    }
                });
                tf.vars.insert(*v, tv);
                if let Some(r) = from {
                    owner.insert(*v, r);
                }
                grew = true;
            }
        });
        if !grew {
            break;
        }
    }
    if let Err(why) = lives(cx, code, &root_vars, &owner) {
        if trace() {
            eprintln!("[transient] site in {} declined: {why}", def.name());
        }
        return None;
    }
    // The producers' twins only for a site that can hold them: a declined site leaves none.
    for f in &calls {
        cx.tr_twin(TrKey {
            d: *f,
            params: 0,
            result: true,
        })?;
    }
    Some(tf)
}

/// The frame's one arena mark is given back at each root's bind and free, which is sound when
/// no transient value of the frame is still to be read there: each root is bound once, every
/// read of it — and of a local derived from it — lies inside the block of its bind and after
/// it, and the roots' spans from bind to last read, in the IR's order, are apart.
fn lives(cx: &Cx, code: &Value, roots: &[u16], owner: &HashMap<u16, u16>) -> Result<(), String> {
    struct Walk<'a> {
        cx: &'a Cx<'a>,
        roots: &'a [u16],
        owner: &'a HashMap<u16, u16>,
        at: usize,
        blocks: Vec<usize>,
        binds: HashMap<u16, Vec<(usize, usize)>>,
        reads: Vec<(u16, usize, Vec<usize>)>,
    }
    impl Walk<'_> {
        fn free_op(&self, d: u32) -> bool {
            (d as usize) < self.cx.data.definitions.len()
                && matches!(
                    self.cx.data.def(d).name(),
                    "OpFreeRef" | "OpFreeRefIfDistinct" | "OpFreeRecordIn" | "OpClear"
                )
        }
        fn go(&mut self, v: &Value) {
            self.at += 1;
            match v.unspan() {
                Value::Block(b) => {
                    self.blocks.push(std::ptr::from_ref(&**b) as usize);
                    for op in &b.operators {
                        self.go(op);
                    }
                    self.blocks.pop();
                    return;
                }
                Value::Set(x, e) if self.roots.contains(x) => {
                    if !matches!(e.unspan(), Value::Null) {
                        let blk = self.blocks.last().copied().unwrap_or(0);
                        self.binds.entry(*x).or_default().push((self.at, blk));
                    }
                    self.go(e);
                    return;
                }
                Value::Call(d, args) if self.free_op(*d) => {
                    // A free names its root without reading it.
                    for (i, a) in args.iter().enumerate() {
                        if i == 0 && matches!(a.unspan(), Value::Var(_)) {
                            continue;
                        }
                        self.go(a);
                    }
                    return;
                }
                Value::Var(w) => {
                    if let Some(r) = self.owner.get(w) {
                        self.reads.push((*r, self.at, self.blocks.clone()));
                    }
                }
                _ => {}
            }
            v.for_each_child(&mut |c| self.go(c));
        }
    }
    let mut w = Walk {
        cx,
        roots,
        owner,
        at: 0,
        blocks: Vec::new(),
        binds: HashMap::new(),
        reads: Vec::new(),
    };
    w.go(code);
    let mut spans: Vec<(usize, usize)> = Vec::new();
    for r in roots {
        let [(bind, blk)] = w.binds.get(r).map_or(&[][..], Vec::as_slice) else {
            return Err("a root bound more than once".into());
        };
        let mut last = *bind;
        for (_, at, blocks) in w.reads.iter().filter(|(o, _, _)| o == r) {
            if *at <= *bind || !blocks.contains(blk) {
                return Err("a root read outside its bind's block".into());
            }
            last = last.max(*at);
        }
        spans.push((*bind, last));
    }
    let apart = spans
        .iter()
        .enumerate()
        .all(|(i, a)| spans[i + 1..].iter().all(|b| a.1 < b.0 || b.1 < a.0));
    if apart {
        Ok(())
    } else {
        Err("its roots live together".into())
    }
}

/// The work text an argument hands a callee: the variable, or the `{ w = ""; OpCreateStack(w) }`
/// block a work argument is lowered to.
fn work_buffer(x: &Value) -> Option<u16> {
    match x.unspan() {
        Value::Var(w) => Some(*w),
        Value::Block(b) => match b.operators.last().map(Value::unspan) {
            Some(Value::Call(_, a)) if a.len() == 1 => match a[0].unspan() {
                Value::Var(w) => Some(*w),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    }
}

impl Cx<'_> {
    /// `d` answers its `&text` parameter — every `return` and its tail name it: that
    /// parameter's position.
    pub(crate) fn work_return(&self, func: u32) -> Option<usize> {
        if (func as usize) >= self.data.definitions.len() {
            return None;
        }
        let def = self.data.def(func);
        if !def.name().starts_with("n_")
            || matches!(def.code(), Value::Null)
            || !matches!(def.returned().base(), Type::Text(_))
        {
            return None;
        }
        let vars = def.variables();
        let args = vars.arguments();
        let mut found = None;
        for (pos, &arg) in args.iter().enumerate() {
            if matches!(vars.tp(arg).base(), Type::RefVar(t) if matches!(t.base(), Type::Text(_))) {
                if found.is_some() {
                    return None;
                }
                found = Some((pos, arg));
            }
        }
        let (at, param) = found?;
        let mut ok = true;
        def.code().walk(&mut |n| {
            if let Value::Return(x) = n
                && !matches!(x.unspan(), Value::Var(w) if *w == param)
            {
                ok = false;
            }
        });
        let Value::Block(body) = def.code() else {
            return None;
        };
        let tail_ok = match body.operators.last()?.unspan() {
            Value::Return(x) => matches!(x.unspan(), Value::Var(w) if *w == param),
            Value::Var(w) => *w == param,
            _ => false,
        };
        (ok && tail_ok).then_some(at)
    }

    /// `d`'s `__wr` twin: its body, answering nothing — the caller reads the buffer it filled.
    pub(crate) fn work_twin(&self, d: u32) -> Option<String> {
        let name = format!("{}__wr", c_name(self.data, d));
        if let Some(r) = self.tr.borrow().work_twins.get(&d) {
            return r.then(|| name.clone());
        }
        let r = self.write_work_twin(d, &name);
        if trace() {
            match &r {
                Ok(()) => eprintln!("[transient] work twin {name}"),
                Err(why) => eprintln!("[transient] work twin {name} declined: {why}"),
            }
        }
        self.tr.borrow_mut().work_twins.insert(d, r.is_ok());
        r.ok().map(|()| name)
    }

    fn write_work_twin(&self, d: u32, name: &str) -> Result<(), String> {
        let p = self.work_return(d).ok_or("no work return")?;
        let def = self.data.def(d);
        let vars = def.variables();
        let args = vars.arguments();
        if args.len() != def.attributes().len()
            || !self.vr_ptr_params(d).is_empty()
            || self.value_records().fns.contains_key(&d)
            || def.hidden_return_buffer_attr().is_some()
        {
            return Err("a signature the twin does not take".into());
        }
        let shapes = crate::c::variable_shapes(self, vars, &args, self.data.body_width32(d))?;
        let mut params = Vec::new();
        for &a in &args {
            if vars.is_store_text_link(a) {
                return Err("a store text link".into());
            }
            let c = shapes[a as usize];
            let n = var_name(vars.name(a), a);
            params.push(if matches!(vars.tp(a).base(), Type::RefVar(_)) {
                format!("{} *{n}", held(c))
            } else {
                format!("{} {n}", passed(c))
            });
        }
        let params = if params.is_empty() {
            "void".to_string()
        } else {
            params.join(", ")
        };
        let sig = format!("static inline void {name}({params})");
        let (body, _) = FnWriter::new(self, d, shapes, CType::Void)
            .work_return(args[p])
            .write()?;
        let _ = writeln!(self.gen_decls.borrow_mut(), "{sig};");
        let _ = write!(self.gen_bodies.borrow_mut(), "\n{sig} {{\n{body}}}\n");
        Ok(())
    }
}

/// Does binding `x` make a fresh record — a mint, a placement, a new element, or nothing yet?
fn is_mint(cx: &Cx, x: &Value) -> bool {
    match x.unspan() {
        Value::Null => true,
        Value::Call(op, _) if (*op as usize) < cx.data.definitions.len() => matches!(
            cx.data.def(*op).name(),
            "OpNewRecord" | "OpPlaceRecord" | "OpDatabase" | "OpDatabaseNP" | "OpNullRefSentinel"
        ),
        Value::Block(b) => b.operators.last().is_some_and(|t| is_mint(cx, t)),
        _ => false,
    }
}

/// `x` is the record a `?` builds where its read misses — `{ OpDatabase(r, tp); OpSet…(r, off,
/// zero)…; r }` of a type whose default is all zeros — and answers that type.
fn default_object(cx: &Cx, value: &Value) -> Option<u16> {
    let Value::Block(block) = value.unspan() else {
        return None;
    };
    let ops: Vec<&Value> = block
        .operators
        .iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect();
    let (last, body) = ops.split_last()?;
    let Value::Var(rec) = last.unspan() else {
        return None;
    };
    let is_r = |v: &Value| matches!(v.unspan(), Value::Var(w) if w == rec);
    let zero = |lit: &Value| {
        matches!(
            lit.unspan(),
            Value::Int(0) | Value::Long(0) | Value::Boolean(false) | Value::Null
        ) || matches!(lit.unspan(), Value::Text(t) if t.is_empty())
            || matches!(lit.unspan(), Value::Float(fl) if *fl == 0.0)
    };
    let mut tp = None;
    for op in body {
        let Value::Call(func, opargs) = op.unspan() else {
            return None;
        };
        if (*func as usize) >= cx.data.definitions.len() {
            return None;
        }
        match (cx.data.def(*func).name(), opargs.as_slice()) {
            ("OpDatabase" | "OpDatabaseNP", [target, ty]) if is_r(target) && tp.is_none() => {
                let Value::Int(ty) = ty.unspan() else {
                    return None;
                };
                tp = Some(u16::try_from(*ty).ok()?);
            }
            (
                "OpSetInt" | "OpSetText" | "OpSetBoolean" | "OpSetFloat" | "OpSetInt4",
                [target, _, val],
            ) if is_r(target) && zero(val) && tp.is_some() => {}
            _ => return None,
        }
    }
    let tp = tp?;
    cx.tr_zero_default(tp).then_some(tp)
}

/// Is `x` a field (at any depth) of one of `owned`?
fn field_of(cx: &Cx, x: &Value, owned: &HashSet<u16>) -> bool {
    match x.unspan() {
        Value::Call(op, a) if (*op as usize) < cx.data.definitions.len() => {
            cx.data.def(*op).name() == "OpGetField"
                && a.first().is_some_and(|b| match b.unspan() {
                    Value::Var(w) => owned.contains(w),
                    _ => field_of(cx, b, owned),
                })
        }
        Value::Block(b) => b.operators.last().is_some_and(|t| field_of(cx, t, owned)),
        _ => false,
    }
}

/// Does `x` answer a part of a transient value — a field, an element, an alias?
fn derives(cx: &Cx, x: &Value, vars: &HashMap<u16, TrVar>) -> bool {
    match x.unspan() {
        Value::Var(w) => vars.contains_key(w),
        Value::Block(b) => b.operators.last().is_some_and(|t| derives(cx, t, vars)),
        // `v[i]?`: the element, or the record the miss builds.
        Value::If(_, a, b) => {
            (derives(cx, a, vars) && default_object(cx, b).is_some())
                || (derives(cx, b, vars) && default_object(cx, a).is_some())
        }
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
    /// A `__wr` twin: `return p` answers nothing, the caller reads its buffer.
    pub(crate) fn work_return(mut self, p: u16) -> Self {
        self.work_ret = Some(p);
        self
    }

    /// Write this body with transient facts `tf`: a twin or a site.
    pub(crate) fn transient(mut self, tf: TrFn) -> Self {
        if let Some(p) = tf.phantom {
            self.params[p as usize] = false;
            // A vector result the IR never mints — its local promoted into the caller's
            // buffer, which exists already — is the twin's own from its entry.
            if matches!(tf.vars.get(&p), Some(TrVar::Vec(_))) {
                let n = self.var(p);
                self.decls.push(format!(
                    "{n} = (loft_ta_vec *) loft_ta_alloc(sizeof(loft_ta_vec));"
                ));
            }
        }
        if !tf.roots.is_empty() {
            self.decls
                .push("loft_ta_mark_t loft_tmf = loft_ta_mark();".to_string());
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

    /// A write into transient value `dst`: only into a record this twin built itself.
    fn tr_writable(&self, dst: &Value) -> Result<(), String> {
        let root = self.tr_root(dst);
        if root.is_some_and(|r| self.tr.as_ref().is_some_and(|t| t.owned.contains(&r))) {
            Ok(())
        } else {
            Err(format!(
                "a write into a transient value this body did not build (`{}`)",
                root.map_or("?", |r| self.vars.name(r))
            ))
        }
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
            Value::If(_, a, b) => self.tr_root(a).or_else(|| self.tr_root(b)),
            _ => None,
        }
    }

    /// Is `v` a choice (`v[i]?`), through a block's tail?  Its place is never null.
    fn tr_choice(v: &Value) -> bool {
        match v.unspan() {
            Value::Block(b) => b.operators.last().is_some_and(Self::tr_choice),
            Value::If(..) => true,
            _ => false,
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
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per place a transient value has"
    )]
    fn tr_place(&mut self, v: &Value, ind: usize) -> Result<Place, String> {
        match v.unspan() {
            Value::Var(w) => {
                let tv = *self
                    .tr
                    .as_ref()
                    .and_then(|t| t.vars.get(w))
                    .ok_or_else(|| {
                        format!(
                            "a store value where a transient one was meant (`{}`)",
                            self.vars.name(*w)
                        )
                    })?;
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
            // `v[i]?` read in place: the element, or the zero record nothing writes.
            Value::If(cond, yes, no) => {
                let (arm, tp, first) = if let Some(tp) = default_object(self.cx, no) {
                    (yes, tp, true)
                } else if let Some(tp) = default_object(self.cx, yes) {
                    (no, tp, false)
                } else {
                    return Err("a choice between two transient values".into());
                };
                let g = self.cx.tr_group(tp);
                let tmp = format!("trp{}", self.decls.len() + 1);
                self.decls.push(format!("lt_{g} *{tmp} = 0;"));
                let zero = format!("{tmp} = (lt_{g} *) loft_ta_zero(sizeof(lt_{g}));");
                let test = super::as_cond(self.expr(cond, ind)?)?;
                self.line(ind, &format!("if ({}) {{", super::bare(&test)));
                if first {
                    let p = self.tr_ptr(arm, ind + 1)?;
                    self.line(ind + 1, &format!("{tmp} = {p};"));
                    self.line(ind, "} else {");
                    self.line(ind + 1, &zero);
                } else {
                    self.line(ind + 1, &zero);
                    self.line(ind, "} else {");
                    let p = self.tr_ptr(arm, ind + 1)?;
                    self.line(ind + 1, &format!("{tmp} = {p};"));
                }
                self.line(ind, "}");
                Ok(Place::Rec(format!("(*{tmp})"), g))
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
                let place = self.tr_place(base, ind)?;
                if let Place::Vec(lv, e) = &place
                    && off == 0
                    && matches!(self.cx.stores.types[tp as usize].parts, Parts::Vector(_))
                {
                    return Ok(Place::Vec(lv.clone(), *e));
                }
                let Place::Rec(lv, g) = place else {
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
            other => Err(format!(
                "a transient value of another shape ({})",
                match other {
                    Value::Call(op, _) => self.cx.data.def(*op).name().to_string(),
                    Value::If(..) => "an if".to_string(),
                    Value::Insert(_) => "an insert".to_string(),
                    _ => format!("{other:?}").chars().take(60).collect(),
                }
            )),
        }
    }

    /// The vector `OpNewRecord(data, parent, field)` appends to: `data` itself (no field), or
    /// the vector field `field` of `parent`, the record `data` names.
    fn tr_new_record_vec(&mut self, cargs: &[Value], ind: usize) -> Result<String, String> {
        let field = match cargs.get(2).map(Value::unspan) {
            Some(Value::Int(f)) => *f,
            _ => return Err("an element of an unknown field".into()),
        };
        if field < 0 || field == i32::from(u16::MAX) {
            let Place::Vec(lv, _) = self.tr_place(&cargs[0], ind)? else {
                return Err("an element of a record".into());
            };
            return Ok(lv);
        }
        // An element appended to a record's vector field: not taken yet — its twins reach
        // enum layouts whose variants overlap, which the struct derivation does not merge.
        if crate::env_once!(std::env::var("LOFT_TRANSIENT_FIELD_APPEND").is_err()) {
            return Err("an element of a record's vector field".into());
        }
        let Some(Value::Int(parent)) = cargs.get(1).map(Value::unspan) else {
            return Err("an element of a record of no known type".into());
        };
        let parent = u16::try_from(*parent).map_err(|_| "a type out of range")?;
        let Parts::Struct(fs) = &self.cx.stores.types[parent as usize].parts else {
            return Err("an element of a field of no struct".into());
        };
        let f = fs
            .get(usize::try_from(field).map_err(|_| "a field out of range")?)
            .ok_or("a field index out of range")?;
        if !matches!(
            self.cx.stores.types[f.content as usize].parts,
            Parts::Vector(_)
        ) {
            return Err("an element of a field that is no vector".into());
        }
        let off = i64::from(f.position);
        let Place::Rec(rv, g) = self.tr_place(&cargs[0], ind)? else {
            return Err("a field of a vector".into());
        };
        self.cx.tr_member(g, off, Member::Vector);
        Ok(format!("{rv}.{}", Member::Vector.name(off)))
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
            "OpFreeText" if first.is_some_and(|v| self.tr_text_lift(v).is_some()) => void(),
            // A free guarded against a transient value: the two are always distinct.
            "OpFreeRefIfDistinct" if dead_or_tr(self, var_of(args.get(1))) => {
                Ok(Some(self.operator("OpFreeRef", &args[..1], ind)?))
            }
            "OpDistinctStore" if args.iter().any(|a| dead_or_tr(self, var_of(Some(a)))) => {
                Ok(Some(E::new("1", K::Cond)))
            }
            "OpRefIsNull" | "OpConvBoolFromRef" if args.len() == 1 => {
                let Some(v) = first else { return Ok(None) };
                // A dead buffer parameter is one its caller passed: present, and never minted.
                if self.tr_dead(v) && self.params[v as usize] {
                    let c = if name == "OpRefIsNull" { "0" } else { "1" };
                    return Ok(Some(E::new(c, K::Cond)));
                }
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
                self.tr_writable(&args[0])?;
                let Some(Value::Int(tp)) = args.get(1).map(Value::unspan) else {
                    return Err("a mint of no known type".into());
                };
                let tp = u16::try_from(*tp).map_err(|_| "a type out of range")?;
                // A vector's own record (`main_vector<T>`, its field 0 the vector): the arena
                // vector itself.
                if matches!(
                    self.tr.as_ref().and_then(|t| t.vars.get(&v)),
                    Some(TrVar::Vec(_))
                ) {
                    let n = self.var(v);
                    self.line(
                        ind,
                        &format!("{n} = (loft_ta_vec *) loft_ta_alloc(sizeof(loft_ta_vec));"),
                    );
                    return void();
                }
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
                let nullable = !Self::tr_choice(base)
                    && self.tr.as_ref().is_some_and(|t| t.nullable.contains(&root));
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
                self.tr_writable(base)?;
                let Value::Int(off) = off.unspan() else {
                    return Err("a setter at a computed offset".into());
                };
                let off = i64::from(*off);
                let place = self.tr_place(base, ind)?;
                // The length word of a vector's own record, zeroed at its mint: the arena
                // vector is born empty.
                if name == "OpSetInt4"
                    && off == 0
                    && matches!(val.unspan(), Value::Int(0))
                    && matches!(place, Place::Vec(..))
                {
                    return void();
                }
                let Place::Rec(lv, g) = place else {
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
                self.tr_writable(dst)?;
                let s = self.tr_ptr(src, ind)?;
                let Place::Rec(d, g) = self.tr_place(dst, ind)? else {
                    return Err("a record copied into a vector".into());
                };
                // The copy shares its vectors' elements with the source: sealed, a push into
                // it moves it first.
                self.cx.tr.borrow_mut().seals.insert(g);
                self.line(ind, &format!("{d} = *{s}; lt_seal_{g}(&{d});"));
                void()
            }
            "OpMoveVector" if args.first().and_then(|b| self.tr_root(b)).is_some() => {
                if !self.tr_builds() {
                    return Err("a site that writes a transient vector".into());
                }
                let [dst, src, ..] = args else {
                    return Err("a vector move of another shape".into());
                };
                self.tr_writable(dst)?;
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
                self.tr_writable(dst)?;
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
            "OpClearVector" if args.first().and_then(|b| self.tr_root(b)).is_some() => {
                if !self.tr_builds() {
                    return Err("a site that clears a transient vector".into());
                }
                self.tr_writable(&args[0])?;
                // The result buffer before its mint: a twin starts from nothing.
                if var_of(args.first())
                    .is_some_and(|v| self.tr.as_ref().is_some_and(|t| t.phantom == Some(v)))
                {
                    return void();
                }
                let Place::Vec(lv, _) = self.tr_place(&args[0], ind)? else {
                    return Err("a clear of a record".into());
                };
                // A vector local before its mint is no vector yet: nothing to clear.
                match lv.strip_prefix("(*").and_then(|r| r.strip_suffix(')')) {
                    Some(p) if p.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') => {
                        self.line(ind, &format!("if ({p}) {lv}.n = 0;"));
                    }
                    _ => self.line(ind, &format!("{lv}.n = 0;")),
                }
                void()
            }
            _ => {
                // Any other operator handed a transient value would read it as a store
                // reference.
                if args.iter().any(|a| {
                    self.tr_root(a).is_some()
                        || var_of(Some(a))
                            .is_some_and(|v| self.tr_dead(v) && !self.params[v as usize])
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
                // A buffer parameter's own mint when its caller passed none (`__ref = W.v`, under
                // an `if` that a dead parameter's null test answers false).
                Value::Call(op, a)
                    if self.params[var as usize]
                        && self.cx.data.def(*op).name() == "OpGetField"
                        && matches!(a.first().map(Value::unspan), Some(Value::Var(_))) =>
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
                        ("OpGetVectorNullable" | "OpGetVector", TrVar::Rec(g)) => {
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
                            self.tr_writable(&cargs[0])?;
                            let lv = self.tr_new_record_vec(cargs, ind)?;
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
                    // The frame's arena back to its entry: nothing transient is left to read.
                    if let Some(m) = self.tr.as_ref().and_then(|t| t.roots.get(&var)).cloned() {
                        self.line(ind, &format!("loft_ta_back({m});"));
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
                Value::If(c, t, e) => {
                    let cond = super::as_cond(self.expr(c, ind)?)?;
                    self.line(ind, &format!("if ({}) {{", super::bare(&cond)));
                    self.tr_set_arm(var, t, ind + 1)?;
                    self.line(ind, "} else {");
                    self.tr_set_arm(var, e, ind + 1)?;
                    self.line(ind, "}");
                    return Ok(());
                }
                _ => {}
            }
            Err("a transient local bound from a value of another shape".into())
        })())
    }

    /// One arm of `var = if c { a } else { b }`: the record a `?` builds where its read misses
    /// is the zero record nothing writes; any other arm binds as `tr_set` does.
    fn tr_set_arm(&mut self, var: u16, arm: &Value, ind: usize) -> Result<(), String> {
        if let Some(tp) = default_object(self.cx, arm) {
            let g = self.cx.tr_group(tp);
            let lhs = self.var(var);
            self.line(
                ind,
                &format!("{lhs} = (lt_{g} *) loft_ta_zero(sizeof(lt_{g}));"),
            );
            return Ok(());
        }
        match self.tr_set(var, arm, ind) {
            Some(r) => r,
            None => Err("a choice arm of another shape".into()),
        }
    }

    /// A `text argument copy` block — `{ w = ""; w += t; w }` — of a transient record's text
    /// `t`: that text.  Nothing writes a transient tree while it lives, so its bytes stay put for
    /// the argument's whole use.
    pub(super) fn tr_text_copy<'v>(&self, b: &'v crate::data::Block) -> Option<&'v Value> {
        if b.name != "text argument copy" {
            return None;
        }
        let ops: Vec<&Value> = b
            .operators
            .iter()
            .filter(|o| !matches!(o.unspan(), Value::Line(_)))
            .collect();
        let [set, app, tail] = ops.as_slice() else {
            return None;
        };
        let Value::Set(w, empty) = set.unspan() else {
            return None;
        };
        if !matches!(empty.unspan(), Value::Text(t) if t.is_empty())
            || !matches!(tail.unspan(), Value::Var(x) if x == w)
        {
            return None;
        }
        let Value::Call(op, a) = app.unspan() else {
            return None;
        };
        if self.cx.data.def(*op).name() != "OpAppendText"
            || !matches!(a.first().map(Value::unspan), Some(Value::Var(x)) if x == w)
        {
            return None;
        }
        let src = a.get(1)?;
        let Value::Call(g, ga) = src.unspan() else {
            return None;
        };
        (self.cx.data.def(*g).name() == "OpGetText"
            && ga.first().and_then(|b| self.tr_root(b)).is_some())
        .then_some(src)
    }

    /// A text local bound once to a `text argument copy` of a transient text (`tr_text_copy`),
    /// read only in the block of its bind, after it, before any root there is bound or freed:
    /// the text it copies.
    pub(super) fn tr_text_lift(&mut self, v: u16) -> Option<&Value> {
        if self.tr.as_ref()?.text_lifts.is_none() {
            let lifts = self.tr_find_text_lifts();
            if let Some(t) = self.tr.as_mut() {
                t.text_lifts = Some(lifts);
            }
        }
        self.tr
            .as_ref()?
            .text_lifts
            .as_ref()?
            .get(&v)
            .map(|(x, _)| x)
    }

    /// Is text lift `v` a copied argument (its bind skipped) rather than a work-buffer result?
    pub(super) fn tr_lift_is_copy(&self, v: u16) -> bool {
        self.tr
            .as_ref()
            .and_then(|t| t.text_lifts.as_ref())
            .and_then(|l| l.get(&v))
            .is_some_and(|(_, c)| *c)
    }

    fn tr_find_text_lifts(&self) -> HashMap<u16, (Value, bool)> {
        let mut out = HashMap::new();
        let roots: HashSet<u16> = self
            .tr
            .as_ref()
            .map(|t| t.roots.keys().copied().collect())
            .unwrap_or_default();
        let mut sets: HashMap<u16, usize> = HashMap::new();
        let mut uses: HashMap<u16, usize> = HashMap::new();
        self.code.walk(&mut |n| match n {
            Value::Set(v, _) => *sets.entry(*v).or_default() += 1,
            Value::Var(v) => *uses.entry(*v).or_default() += 1,
            _ => {}
        });
        self.code.walk(&mut |n| {
            let Value::Block(blk) = n else { return };
            for (at, op) in blk.operators.iter().enumerate() {
                let Value::Set(v, bound) = op.unspan() else { continue };
                if self.params[*v as usize] || sets.get(v) != Some(&1) {
                    continue;
                }
                // A copied transient text, or a call that answers the caller's work buffer.
                let (src, copy, buffer) = match bound.unspan() {
                    Value::Block(cb) => match self.tr_text_copy(cb) {
                        Some(src) => (src.clone(), true, None),
                        None => continue,
                    },
                    Value::Call(callee, cargs) => {
                        let Some(pos) = self.cx.work_return(*callee) else { continue };
                        let Some(w) = cargs.get(pos).and_then(work_buffer) else { continue };
                        if self.params[w as usize] {
                            continue;
                        }
                        (Value::Var(w), false, Some(w))
                    }
                    _ => continue,
                };
                // Every later mention in this block, none of a root's bind or free between.
                let mut seen = 0usize;
                let mut clean = true;
                for later in &blk.operators[at + 1..] {
                    let mut here = 0usize;
                    later.walk(&mut |m| {
                        if let Value::Var(w) = m
                            && w == v
                        {
                            here += 1;
                        }
                    });
                    if seen < uses.get(v).copied().unwrap_or(0) {
                        let mut touches_root = false;
                        later.walk(&mut |m| match m {
                            Value::Set(w, _) if roots.contains(w) => touches_root = true,
                            // The buffer itself, before the result's last read: another fill.
                            Value::Var(w) if Some(*w) == buffer => touches_root = true,
                            Value::Call(op, opargs)
                                if matches!(
                                    self.cx.data.def(*op).name(),
                                    "OpFreeRef" | "OpFreeRefIfDistinct"
                                ) && matches!(opargs.first().map(Value::unspan), Some(Value::Var(w)) if roots.contains(w)) =>
                            {
                                touches_root = true;
                            }
                            _ => {}
                        });
                        if touches_root {
                            clean = false;
                        }
                    }
                    seen += here;
                }
                if clean && seen == uses.get(v).copied().unwrap_or(0) {
                    out.insert(*v, (src, copy));
                }
            }
        });
        out
    }

    /// `len(v)` of a transient vector: its count.
    pub(super) fn tr_length(
        &mut self,
        route: &str,
        args: &[Value],
        ind: usize,
    ) -> Result<Option<E>, String> {
        if self.tr.is_none()
            || route != "t_6vector_len"
            || args.len() != 1
            || self.tr_root(&args[0]).is_none()
        {
            return Ok(None);
        }
        let Place::Vec(lv, _) = self.tr_place(&args[0], ind)? else {
            return Err("the length of a record".into());
        };
        Ok(Some(E::new(format!("((int64_t) ({lv}).n)"), K::Int)))
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
        // The body's tail: a block's statements, then the value it answers — an explicit
        // `return` there answers its operand.
        match v.unspan() {
            Value::Return(x) => return self.tr_return(x, ind),
            Value::Block(b) if !b.operators.is_empty() => {
                let n = b.operators.len();
                for op in &b.operators[..n - 1] {
                    self.stmt(op, ind)?;
                }
                return self.tr_return(&b.operators[n - 1], ind);
            }
            _ => {}
        }
        let p = if self.ret == CType::TrVec {
            match self.tr_place(v, ind)? {
                Place::Vec(lv, _) => format!("&{lv}"),
                Place::Rec(lv, g) => {
                    self.cx.tr_member(g, 0, Member::Vector);
                    format!("&{lv}.{}", Member::Vector.name(0))
                }
            }
        } else {
            self.tr_ptr(v, ind)?
        };
        let r = self.temp(self.ret);
        self.line(ind, &format!("{r} = {p};"));
        self.ret_held(&r, ind);
        Ok(())
    }
}
