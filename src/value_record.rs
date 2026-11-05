// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I64 — Bytecode compiler (IR to bytecode): the value-record pass at its top

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
//! - an admitted function or parameter belongs to a function that is not `pub` and that
//!   loft calls somewhere (one nothing in loft calls is called from outside loft by name, so
//!   its signature is its API).  A `pub` function the analysis admits is first SPLIT
//!   ([`split_public`]): the API keeps its record for the callers outside, `(R-Escape)`, and
//!   every loft call reaches an internal copy, which is an ordinary function here;
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
//! (`Data::open_world`), which includes a compiled build that ships the live tier: its
//! runtime parses the program again for the interpreter it flips functions to.
//! `LOFT_NO_IR_VALUE_RECORD=1` is the switch.

use crate::data::{Block, Data, DefType, I64, Type, Value};
use crate::database::{Parts, Stores};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};
pub(crate) use crate::ir_shape::names_var;

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
    /// `@FR-O-ViewField` — a heap field the tuple carries as a VIEW: a reference to the place
    /// the exit's value lives in (`opt::value_records`' view leaf), never freed by the tuple.
    /// Its type is the field's own, kept in [`Layout::views`].
    View,
}

impl Kind {
    /// The tuple element of a scalar kind; a view's is its field's type ([`Layout::elem_type`]).
    fn tuple_type(self) -> Type {
        match self {
            Kind::Int => I64.clone(),
            Kind::Float => Type::Float,
            Kind::Single => Type::Single,
            Kind::Bool => Type::Boolean,
            Kind::View => unreachable!("a view field's type is the layout's"),
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
    clear: u32,
    set_int4: u32,
    append_vector: u32,
    null_ref: u32,
    get_vector: u32,
    not: u32,
    put_ref: u32,
    /// The default walk's records (`Stores::set_default_value`) on a scratch copy of the
    /// stores, one per record type: what a copy from a null source leaves in a fresh record.
    defaults: std::cell::RefCell<(Stores, HashMap<u16, crate::keys::DbRef>)>,
}

impl Ops {
    fn new(data: &Data, stores: &Stores) -> Ops {
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
            clear: data.def_nr("OpClear"),
            set_int4: data.def_nr("OpSetInt4"),
            append_vector: data.def_nr("OpAppendVector"),
            null_ref: data.def_nr("OpNullRefSentinel"),
            get_vector: data.def_nr("OpGetVector"),
            not: data.def_nr("OpNot"),
            put_ref: data.def_nr("OpPutRef"),
            defaults: std::cell::RefCell::new({
                let mut scratch = stores.clone();
                scratch.max = 0;
                (scratch, HashMap::default())
            }),
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

    /// The value field `off` of a new record of type `tp` holds: the default the record copy
    /// writes into a fresh destination when its source is null — read off the interpreter's
    /// own default walk, so the two cannot disagree.
    fn default_value(&self, tp: u16, off: i32, kind: Kind) -> Value {
        let mut cell = self.defaults.borrow_mut();
        let (scratch, made) = &mut *cell;
        let r = *made.entry(tp).or_insert_with(|| {
            let size = u32::from(scratch.size(tp));
            let r = scratch.database(size.div_ceil(8) + 1);
            scratch.set_default_value(tp, &r);
            r
        });
        let store = scratch.store(&r);
        let at = r.pos + off as u32;
        match kind {
            Kind::Int => Value::Long(store.get_int(r.rec, at)),
            Kind::Float => Value::Float(store.get_float(r.rec, at)),
            Kind::Single => Value::Single(store.get_single(r.rec, at)),
            Kind::Bool => Value::Boolean(store.get_byte(r.rec, at, 0) == 1),
            Kind::View => Value::Call(self.null_ref, Vec::new()),
        }
    }
}

/// A small record's tuple: its scalars in SCHEMA order, `(byte offset, kind)`, an inline
/// sub-record's scalars at their offset from the top record (the schema lays them there).
#[derive(Clone, PartialEq, Debug)]
struct Layout {
    /// The record's runtime type, what an `OpCopyRecord` of it names.
    tp: u16,
    fields: Vec<(i32, Kind)>,
    /// Each inline sub-record: `(offset, runtime type, end offset)`.
    subs: Vec<(i32, u16, i32)>,
    /// Each [`Kind::View`] field: `(offset, the field's type)`.
    views: Vec<(i32, Type)>,
    /// `@FR-R-ValueRecord`'s nullable clause (`(N-Road)`): the tuple ends in a PRESENCE word,
    /// `(f₁, …, fₙ, bool)` — absent, every field its null and `false`.
    presence: bool,
}

/// A layout's KEY: the record's definition number, with [`NULLABLE_KEY`] set for the
/// nullable layout — the dense and the nullable tuple of one record are two layouts, and a
/// site, a local and a parameter agree on one or decline.
const NULLABLE_KEY: u32 = 1 << 31;

/// The record definition a layout key carries.
fn rec_of(key: u32) -> u32 {
    key & !NULLABLE_KEY
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

    /// The tuple element carrying the field at `off`.
    fn elem_type(&self, off: i32) -> Type {
        match self.kind_at(off) {
            Some(Kind::View) => self
                .views
                .iter()
                .find(|(o, _)| *o == off)
                .map(|(_, t)| t.clone())
                .expect("a view field's type"),
            Some(k) => k.tuple_type(),
            None => unreachable!("no field at {off}"),
        }
    }

    fn has_views(&self) -> bool {
        !self.views.is_empty()
    }

    fn tuple_type(&self) -> Type {
        let mut elems: Vec<Type> = self
            .fields
            .iter()
            .map(|(o, _)| self.elem_type(*o))
            .collect();
        if self.presence {
            elems.push(Type::Boolean);
        }
        Type::Tuple(elems)
    }

    /// The presence word's tuple index, for a nullable layout.
    fn presence(&self) -> Option<u16> {
        self.presence.then_some(self.fields.len() as u16)
    }

    /// The fields a copy of runtime type `tp` into offset `base` fills: the whole record,
    /// or one inline sub-record.
    fn extent(&self, base: i32, tp: u16) -> Option<(i32, i32)> {
        if base == 0 && tp == self.tp {
            return Some((0, i32::MAX));
        }
        self.subs
            .iter()
            .find(|(b, t, _)| *b == base && *t == tp)
            .map(|(b, _, e)| (*b, *e))
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
    /// Per literal, in the order [`literals_to_tuples`] meets them, the value each
    /// [`Kind::View`] field carries: the leaf `opt::value_records` proved for that exit.
    leaves: Vec<HashMap<i32, Value>>,
    /// DELIVERY locals: returned, and set only from exit values — each becomes a tuple local.
    delivered: HashSet<u16>,
    /// The callees called bare at a result position, one entry per site.
    tail_calls: Vec<u32>,
}

/// What the pass works from: the admitted functions and parameters and the layouts of the
/// records they carry.
struct World {
    ops: Ops,
    layouts: HashMap<u32, Layout>,
    cands: HashMap<u32, Callee>,
    /// `(function, attribute index)` → the record the parameter carries.
    params: HashMap<(u32, usize), u32>,
    /// `opt::value_records`' answer, read for its value locals: a carrier of a record with a
    /// view field must be one, which is what proves its reads are reads and that nothing
    /// disturbs the viewed place before the last of them (`@FR-O-ViewField`).
    vr: crate::opt::value_records::layout::ValueRecords,
    /// Per function, its value locals (`value_locals_in`), computed on first ask.
    value_locals: std::cell::RefCell<HashMap<u32, HashSet<u16>>>,
    /// The plain record PARAMETERS a carrier can be LENT to as a record (`Rewrite::lend`):
    /// no `&`, no tuple, only read by the callee, and its result does not depend on them —
    /// so a record that lives as long as the carrier serves the call.  `(callee, index)` →
    /// the record definition.
    lendable: HashMap<(u32, usize), u32>,
}

/// [`World::lendable`] over every function of the program.
fn lendable_params(data: &Data) -> HashMap<(u32, usize), u32> {
    let mut out = HashMap::default();
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type != DefType::Function || matches!(def.code(), Value::Null) {
            continue;
        }
        for (i, a) in def.attributes.iter().enumerate() {
            let Type::Reference(rec, _) = &a.typedef else {
                continue;
            };
            let p = def.variables.var(&a.name);
            if !a.hidden
                && p != u16::MAX
                && !def.returned().depend().contains(&p)
                && crate::c::vr_param_read_only(data, def.code(), p)
            {
                out.insert((d, i), *rec);
            }
        }
    }
    out
}

impl World {
    fn is_value_local(&self, data: &Data, f: u32, l: u16) -> bool {
        let mut cache = self.value_locals.borrow_mut();
        cache
            .entry(f)
            .or_insert_with(|| {
                crate::opt::value_records::value_locals_in(
                    data,
                    f,
                    &self.vr.fns,
                    &self.vr.params,
                    &self.vr.view_offs,
                    &self.vr.fnref_sites,
                    &self.vr.sp,
                )
                .into_keys()
                .collect()
            })
            .contains(&l)
    }
}

/// Rewrite every admitted function, parameter and call site; answers how many functions
/// and parameters now carry a tuple.
#[expect(
    clippy::too_many_lines,
    reason = "one pass over the program: admit, twin, rewrite"
)]
pub fn rewrite_program(data: &mut Data, stores: &Stores) -> usize {
    if disabled() || data.open_world {
        return 0;
    }
    let mut laps = crate::opt::laps::PassLaps::new(std::env::var_os("LOFT_TIMING").is_some());
    let mut vr = crate::opt::value_records::value_records(data, stores, false);
    if split_public(data, &vr) {
        vr = crate::opt::value_records::value_records(data, stores, false);
    }
    laps.lap("vr_analysis");
    if vr.fns.is_empty() && vr.params.is_empty() {
        return 0;
    }
    let mut w = World {
        ops: Ops::new(data, stores),
        layouts: HashMap::default(),
        cands: HashMap::default(),
        params: HashMap::default(),
        vr: crate::opt::value_records::layout::ValueRecords::default(),
        value_locals: std::cell::RefCell::default(),
        lendable: lendable_params(data),
    };
    // `@FR-R-FnRefValue` admits a dispatch's arms on `--native` alone, where the dispatch
    // answers and passes the tuple: the interpreter calls them through the fn-ref with
    // records, so the IR keeps their signatures, result and parameters — and an arm whose IR result became a tuple would not
    // match its own dispatch (`fnref::dispatch_arms` compares result types).
    let arms = vr.fnref_arms();
    for (&d, &tp) in &vr.fns {
        // `@FR-R-ValueRecord`'s nullable clause is `--native`'s alone: a `τ?` tuple carries a
        // presence word this pass has no spelling for, so the IR keeps the record form there.
        let shape = if arms.contains(&d) {
            Err("a fn-ref arm")
        } else {
            callee_shape(data, stores, &w.ops, &mut w.layouts, &vr, d, tp)
        };
        match shape {
            Ok(c) => {
                w.cands.insert(d, c);
            }
            Err(why) if trace() => {
                eprintln!(
                    "[ir-valuerec] {}: not a candidate: {why}",
                    data.def(d).name()
                );
            }
            Err(_) => {}
        }
    }
    for (&d, ps) in &vr.params {
        if arms.contains(&d) {
            continue;
        }
        for (&idx, &tp) in ps {
            if let Some(record) = param_shape(data, stores, &mut w.layouts, d, idx, tp) {
                w.params.insert((d, idx), record);
            }
        }
    }
    w.vr = vr;
    let functions: Vec<u32> = (0..data.definitions())
        .filter(|&f| {
            let def = data.def(f);
            def.def_type == DefType::Function && !matches!(def.code(), Value::Null)
        })
        .collect();
    laps.lap("vr_shapes");
    let mut rounds = 0u32;
    // ADMISSION, a fixpoint: declining a function or a parameter turns what it carried into
    // an ordinary value, which declines the carriers that handed it on.  Every round only
    // removes.  A call site that cannot take the tuple declines nothing — it keeps calling
    // the record form (below).
    loop {
        rounds += 1;
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
        for d in w.cands.keys().filter(|d| !called.contains(d)) {
            if trace() {
                eprintln!(
                    "[ir-valuerec] {}: nothing in loft calls it",
                    data.def(*d).name()
                );
            }
            bad_fns.insert(*d);
        }
        bad_params.extend(w.params.keys().filter(|(d, _)| !called.contains(d)));
        if trace() {
            for (d, i) in &bad_params {
                eprintln!(
                    "[ir-valuerec] {}: parameter {i} declines (at a site, or by its own body)",
                    data.def(*d).name()
                );
            }
        }
        if bad_fns.is_empty() && bad_params.is_empty() {
            break;
        }
        w.cands.retain(|d, _| !bad_fns.contains(d));
        w.params.retain(|k, _| !bad_params.contains(k));
        if w.cands.is_empty() && w.params.is_empty() {
            return 0;
        }
    }
    laps.lap("vr_admission");
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
        rounds += 1000;
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
    laps.lap("vr_twins");
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
    // The literals first: a one-buffer chain makes the buffer a carrier too, and the carrier
    // rewrite below would otherwise turn the literal's buffer writes into tuple puts before
    // the literal is recognised.
    for (&d, c) in &w.cands {
        let target = twins.get(&d).copied().unwrap_or(d);
        rewrite_callee(data, &w, &twins, target, c);
        touched.insert(target);
    }
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
    laps.lap("vr_rewrite");
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
    laps.lap("vr_intervals");
    // `rounds`: admission rounds + 1000 x twin rounds.
    laps.report(
        &format!(
            "value_record functions={} cands={} params={} rounds={rounds}",
            functions.len(),
            w.cands.len(),
            w.params.len()
        ),
        data.definitions(),
    );
    w.cands.len() + w.params.len()
}

/// A copy of function `d` under a fresh name, for the tuple form while `d` keeps its record
/// form for the sites that owe one.
fn add_twin(data: &mut Data, d: u32) -> u32 {
    add_copy(
        data,
        d,
        "tuple",
        "the tuple twin of a small-record return (@PLN180)",
    )
}

/// A copy of function `d` named `<d>_<suffix>`: no `pub` API, no native symbol — a new
/// loft function the program alone calls.
fn add_copy(data: &mut Data, d: u32, suffix: &str, why: &'static str) -> u32 {
    let base = data.def(d).name().to_string();
    let mut name = format!("{base}_{suffix}");
    let mut n = 1;
    while data.def_nr(&name) != u32::MAX {
        n += 1;
        name = format!("{base}_{suffix}{n}");
    }
    let position = data.def(d).position;
    let t = data.add_def(&name, &position, DefType::Function);
    let mut def = data.def(d).clone();
    def.name = name;
    def.parent = u32::MAX;
    def.first_child = u32::MAX;
    def.next_sibling = u32::MAX;
    def.pub_visible = false;
    def.native = String::new();
    def.synthetic = Some(why);
    data.definitions[t as usize] = def;
    t
}

/// `(R-Escape)` — a `pub` function's signature is its library API: a caller outside the
/// program (a library's cdylib bridge) holds it to its records.  So every `pub` function the
/// analysis admits — a result or a parameter — is split: the API stays as written and calls
/// nobody's tuple, and every call loft makes reaches an internal copy, which this pass then
/// treats as any other function.  A function a cdylib carries (`Definition::native`) is not
/// split: its calls go through the bridge, record ABI and all.  Answers whether anything was
/// split, so the analysis is asked again over the copies.
fn split_public(data: &mut Data, vr: &crate::opt::value_records::layout::ValueRecords) -> bool {
    let mut public: Vec<u32> = vr
        .fns
        .keys()
        .chain(vr.params.keys())
        .copied()
        .filter(|&d| {
            let def = data.def(d);
            def.pub_visible && def.native.is_empty() && def.def_type == DefType::Function
        })
        .collect();
    public.sort_unstable();
    public.dedup();
    if public.is_empty() {
        return false;
    }
    let to: HashMap<u32, u32> = public
        .iter()
        .map(|&d| {
            (
                d,
                add_copy(
                    data,
                    d,
                    "in",
                    "the internal copy of a pub function (R-Escape)",
                ),
            )
        })
        .collect();
    for f in 0..data.definitions() {
        let def = &mut data.definitions[f as usize];
        if def.def_type != DefType::Function {
            continue;
        }
        redirect_calls(&mut def.code, &to);
    }
    true
}

/// Every `Call(d, …)` below `n` with `d` in `to` calls `to[d]` instead.  A function is
/// reached only through such a call or a fn-ref; a fn-ref keeps the API, whose record
/// signature the dispatch compares against.
fn redirect_calls(n: &mut Value, to: &HashMap<u32, u32>) {
    if let Value::Call(d, _) = n
        && let Some(t) = to.get(d)
    {
        *d = *t;
    }
    n.for_each_child_mut(&mut |c| redirect_calls(c, to));
}

/// The callee half of the shape, or `None` when `d` cannot return a tuple here.
fn callee_shape(
    data: &Data,
    stores: &Stores,
    ops: &Ops,
    layouts: &mut HashMap<u32, Layout>,
    vr: &crate::opt::value_records::layout::ValueRecords,
    d: u32,
    tp: u16,
) -> Result<Callee, &'static str> {
    let def = data.def(d);
    if def.def_type != DefType::Function {
        return Err("no function");
    }
    // A function a library's cdylib carries is called through its bridge, record ABI and
    // all; a twin cloned from it would inherit the symbol (`Definition::native`).
    if !def.native.is_empty() {
        return Err("a native function");
    }
    // The return buffer is the LAST attribute, so dropping it moves no other parameter.
    let idx = def.hidden_return_buffer_attr().ok_or("no return buffer")?;
    let buf = def.variables.var(&def.attributes[idx].name);
    if idx + 1 != def.attributes.len() || buf == u16::MAX || !def.variables.is_argument(buf) {
        return Err("the return buffer is not the last parameter");
    }
    let Type::Reference(record, _) = def.returned().peel_link().base() else {
        return Err("no record result");
    };
    let layout =
        flat_layout(data, stores, *record, tp, vr.view_offs.get(&tp)).ok_or("no flat layout")?;
    let key = key_of(*record, tp);
    let mut sh = BodyShape::default();
    if !body_shape(def.code(), buf, &layout, ops, true, &mut sh) {
        return Err("a body shape the tuple cannot spell");
    }
    if sh.buf_reads && !sh.bound.contains(&buf) {
        return Err("a field of the caller's buffer read");
    }
    // A returned local that no call handed `buf` binds: a delivery local when every set of it
    // is an exit value, else a VIEW of a record, read field-wise where it is returned.
    let mut delivered: HashSet<u16> = HashSet::default();
    let mut tail_calls = sh.tail_calls.clone();
    let mut returned = sh.returned.clone();
    let mut todo: Vec<u16> = returned.difference(&sh.bound).copied().collect();
    while let Some(l) = todo.pop() {
        let sets: Vec<&(u16, bool, Vec<u32>, bool, HashSet<u16>)> =
            sh.sets.iter().filter(|(x, ..)| *x == l).collect();
        if sets.is_empty() {
            if l == buf
                || !matches!(def.variables.tp(l).base(), Type::Reference(r, _) if r == record)
            {
                if trace() {
                    eprintln!(
                        "[ir-valuerec] {}: returned `{}` is no forward, delivery or view",
                        def.name(),
                        def.variables.name(l)
                    );
                }
                return Err("a returned local that is no forward, delivery or view");
            }
        } else if sets.iter().all(|(_, ok, ..)| *ok) {
            delivered.insert(l);
            for (_, _, calls, _, answers) in sets {
                tail_calls.extend(calls.iter().copied());
                for &a in answers {
                    // A forward's local answered here is returned as well: it must carry.
                    if returned.insert(a) && !sh.bound.contains(&a) {
                        todo.push(a);
                    }
                }
            }
        } else if !matches!(def.variables.tp(l).base(), Type::Reference(r, _) if r == record) {
            return Err("a returned local set from a value the tuple cannot spell");
        }
        // Otherwise a VIEW, set from any record: its fields are read where it is returned.
    }
    if sh
        .sets
        .iter()
        .any(|(l, _, _, names_buf, _)| *names_buf && !delivered.contains(l))
    {
        return Err("the buffer in a set that delivers nothing");
    }
    if layout.has_views()
        && (!delivered.is_empty()
            || !tail_calls.is_empty()
            || returned.difference(&sh.bound).next().is_some())
    {
        return Err("a view layout's exit of another kind");
    }
    let leaves = if layout.has_views() {
        // A forward's exit returns its local's tuple whole — the view the callee proved —
        // so only the literals need leaves here.
        let why = if let Some(plan) = vr.views.get(&d) {
            view_leaves(def.code(), buf, &layout, ops, stores, plan).ok_or("an exit without a leaf")
        } else {
            Err("no view plan")
        };
        why?
    } else {
        Vec::new()
    };
    layouts.insert(key, layout);
    let forwards = returned.intersection(&sh.bound).copied().collect();
    Ok(Callee {
        record: key,
        buf,
        forwards,
        leaves,
        delivered,
        tail_calls,
    })
}

/// Each literal's view values, in the order [`literals_to_tuples`] meets the literals: the
/// leaf the view plan proved for that exit (keyed by the exit block's address, on this very
/// IR), spelled in the IR — `Elem` the element's field, `Last` the field of the container's
/// last element (`v[-1]`, as the writers spell it), `Null` the null reference.  `None` when
/// an exit has no leaf this pass can spell.
fn view_leaves(
    code: &Value,
    buf: u16,
    layout: &Layout,
    ops: &Ops,
    stores: &Stores,
    plan: &crate::opt::value_records::layout::ViewPlan,
) -> Option<Vec<HashMap<i32, Value>>> {
    use crate::opt::value_records::layout::LeafSource;
    fn walk(n: &Value, buf: u16, layout: &Layout, ops: &Ops, out: &mut Vec<usize>) {
        let n = n.unspan();
        if literal(n, buf, layout, ops).is_some() {
            if let Value::Block(b) = n {
                out.push(std::ptr::from_ref(&**b) as usize);
            }
            return;
        }
        n.for_each_child(&mut |c| walk(c, buf, layout, ops, out));
    }
    let mut exits = Vec::new();
    walk(code, buf, layout, ops, &mut exits);
    let mut out = Vec::new();
    for addr in exits {
        let mut values = HashMap::default();
        for (off, _) in &layout.views {
            let value = match plan.leaves.get(&(addr, i64::from(*off)))? {
                LeafSource::Null => Value::Call(ops.null_ref, Vec::new()),
                LeafSource::Elem { elem, off, tp } => Value::Call(
                    ops.get_field,
                    vec![
                        Value::Var(*elem),
                        Value::Int(i32::try_from(*off).ok()?),
                        Value::Int(*tp),
                    ],
                ),
                LeafSource::Last {
                    root,
                    parent_tp,
                    fld,
                    off,
                } => {
                    // The container is a field of a parameter; a parameter that IS the vector
                    // has no `OpGetField` to spell it with here.
                    if root.1 == crate::use_analysis::ANY_FIELD || *fld < 0 {
                        return None;
                    }
                    let (ptp, f) = (u16::try_from(*parent_tp).ok()?, u16::try_from(*fld).ok()?);
                    let vec_tp = stores.field_type(ptp, f);
                    let elem_tp = stores.content(vec_tp);
                    let Parts::Struct(fields) = &stores.types.get(elem_tp as usize)?.parts else {
                        return None;
                    };
                    let field_tp = fields
                        .iter()
                        .find(|x| i64::from(x.position) == *off)?
                        .content;
                    let stride = crate::opt::hoist::element_stride(stores, *parent_tp, *fld);
                    let container = Value::Call(
                        ops.get_field,
                        vec![
                            Value::Var(root.0),
                            Value::Int(i32::try_from(root.1).ok()?),
                            Value::Int(i32::from(vec_tp)),
                        ],
                    );
                    let last = Value::Call(
                        ops.get_vector,
                        vec![
                            container,
                            Value::Int(i32::try_from(stride).ok()?),
                            Value::Int(-1),
                        ],
                    );
                    Value::Call(
                        ops.get_field,
                        vec![
                            last,
                            Value::Int(i32::try_from(*off).ok()?),
                            Value::Int(i32::from(field_tp)),
                        ],
                    )
                }
            };
            values.insert(*off, value);
        }
        out.push(values);
    }
    Some(out)
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
    let layout = flat_layout(data, stores, *record, tp, None)?;
    let key = key_of(*record, tp);
    layouts.insert(key, layout);
    Some(key)
}

/// The record's tuple, when it has two or more scalars and every field is a scalar a tuple
/// carries at the width the field stores it, or an INLINE sub-record of such fields.
///
/// `views` — the offsets of a RESULT's view-leaf fields (`opt::value_records`' `view_offs`):
/// a top-level heap field listed there is carried as a [`Kind::View`].
fn flat_layout(
    data: &Data,
    stores: &Stores,
    record: u32,
    key: u16,
    views: Option<&std::collections::HashSet<i64>>,
) -> Option<Layout> {
    use crate::opt::value_records::layout::{is_nullable_layout, layout_record};
    let tp = layout_record(key);
    let mut layout = Layout {
        tp,
        fields: Vec::new(),
        subs: Vec::new(),
        views: Vec::new(),
        presence: is_nullable_layout(key),
    };
    flatten(data, stores, record, tp, 0, views, &mut layout)?;
    layout.fields.sort_unstable_by_key(|(o, _)| *o);
    // A loft tuple has two or more elements: a one-element `Type::Tuple` has no spelling,
    // and `--native` renders it as the bare scalar its `.0` reads cannot index.
    (layout.fields.len() + usize::from(layout.presence) >= 2).then_some(layout)
}

/// The key of `record`'s layout under analysis key `tp`.
fn key_of(record: u32, tp: u16) -> u32 {
    if crate::opt::value_records::layout::is_nullable_layout(tp) {
        record | NULLABLE_KEY
    } else {
        record
    }
}

/// One level of [`flat_layout`]: the fields of `tp` (declared by `record`), `base` bytes into
/// the top record.
fn flatten(
    data: &Data,
    stores: &Stores,
    record: u32,
    tp: u16,
    base: i32,
    views: Option<&std::collections::HashSet<i64>>,
    layout: &mut Layout,
) -> Option<()> {
    let Parts::Struct(fields) = &stores.types.get(tp as usize)?.parts else {
        return None;
    };
    for f in fields {
        let a = data
            .def(record)
            .attributes
            .iter()
            .position(|a| a.name == f.name)?;
        let ftp = data.attr_type(record, a);
        let off = base + i32::from(f.position);
        let kind = match ftp.base() {
            Type::Integer(_) if stores.size(f.content) == 8 => Kind::Int,
            Type::Float => Kind::Float,
            Type::Single => Kind::Single,
            Type::Boolean => Kind::Bool,
            Type::Vector(..) if base == 0 && views.is_some_and(|v| v.contains(&i64::from(off))) => {
                layout.views.push((off, ftp.clone()));
                Kind::View
            }
            // An inline sub-record: the schema lays its fields into the parent.
            Type::Reference(sub, _)
                if data.def_type(*sub) == DefType::Struct
                    && data.def(*sub).known_type() == f.content
                    && matches!(
                        stores.types.get(f.content as usize).map(|t| &t.parts),
                        Some(Parts::Struct(_))
                    ) =>
            {
                let end = off + i32::from(stores.size(f.content));
                layout.subs.push((off, f.content, end));
                flatten(data, stores, *sub, f.content, off, None, layout)?;
                continue;
            }
            _ => return None,
        };
        layout.fields.push((off, kind));
    }
    Some(())
}

/// A place through a variable: the variable and the offset of the place in its record —
/// the variable itself, or an `OpGetField` path into its inline sub-records.
fn place_of(n: &Value, ops: &Ops) -> Option<(u16, i32)> {
    match n.unspan() {
        Value::Var(v) => Some((*v, 0)),
        Value::Call(op, args) if *op == ops.get_field => match args.as_slice() {
            [inner, Value::Int(o), _] => place_of(inner, ops).map(|(v, b)| (v, b + o)),
            _ => None,
        },
        _ => None,
    }
}

/// What a body does with its return buffer, beyond the object literals it fills.
#[derive(Default)]
struct BodyShape {
    /// Locals bound from a call handed the buffer — a FORWARD's bind.
    bound: HashSet<u16>,
    /// Locals in a result position: a forward's (`bound`), a DELIVERY local (set from exit
    /// values, `sets`), or a VIEW of a record, read field-wise at the exit.
    returned: HashSet<u16>,
    /// The callees called bare at a result position: the call's answer IS the exit.
    tail_calls: Vec<u32>,
    /// A field of the buffer is read: allowed only once a call rebinds the buffer (a
    /// promoted local, `n = g(…); n.ok`), when it is a carrier like any forward's local.
    buf_reads: bool,
    /// Per `l = E` that binds no call handed `buf`: `(l, E is an exit value, its tail calls,
    /// E names buf, the locals E answers)`.  A returned `l` whose every set is an exit value
    /// is a delivery local, and what its sets answer is returned in turn.
    sets: Vec<(u16, bool, Vec<u32>, bool, HashSet<u16>)>,
}

/// Is `n` a value an exit can deliver as the tuple ([`to_tuple`] spells each): the function's
/// own literal or one built into a local buffer, the absent record (a nullable layout), a
/// record a variable names (a forward's local, a delivery local, or a VIEW read field-wise),
/// a materialised view's source, a call whose answer is the exit (`tail_calls`), or a
/// branch or block whose result is one of those.  Anything else is no exit value: the
/// caller declines.
fn exit_value(n: &Value, buf: u16, layout: &Layout, ops: &Ops, sh: &mut BodyShape) -> bool {
    let n = n.unspan();
    if literal(n, buf, layout, ops).is_some() {
        return body_shape(n, buf, layout, ops, true, sh);
    }
    match n {
        Value::Null => layout.presence,
        Value::Call(op, a) if *op == ops.null_ref && a.is_empty() => layout.presence,
        Value::Var(l) => {
            sh.returned.insert(*l);
            true
        }
        Value::Return(x) => exit_value(x, buf, layout, ops, sh),
        Value::If(c, a, b) => {
            !mentions(c, buf)
                && body_shape(c, buf, layout, ops, false, sh)
                && exit_value(a, buf, layout, ops, sh)
                && exit_value(b, buf, layout, ops, sh)
        }
        Value::Block(_) if is_mv_return(n, buf, ops) || is_local_literal(n, buf, layout, ops) => {
            !layout.has_views()
        }
        Value::Block(b) => {
            let Some((last, rest)) = b.operators.split_last() else {
                return false;
            };
            rest.iter()
                .all(|op| body_shape(op, buf, layout, ops, false, sh))
                && exit_value(last, buf, layout, ops, sh)
        }
        Value::Call(d, args)
            if matches!(args.last().map(Value::unspan), Some(Value::Var(_)))
                && args[..args.len() - 1].iter().all(|a| !mentions(a, buf)) =>
        {
            sh.tail_calls.push(*d);
            args[..args.len() - 1]
                .iter()
                .all(|a| body_shape(a, buf, layout, ops, false, sh))
        }
        _ => false,
    }
}

/// `{ OpDatabase(b, tp); OpCopyRecord(src, b, tp); …frees…; b }` — or ending in `return b` —
/// the lowering of `return v` for a view `v`, `b` the result buffer: the exit is `src`'s
/// value, read field-wise.  `src` does not name `b`; the rest only frees what `b` witnesses.
fn mv_parts(n: &Value, ops: &Ops) -> Option<(u16, usize)> {
    let Value::Block(b) = n else {
        return None;
    };
    if b.name != "materialized_view_return" {
        return None;
    }
    let body: Vec<&Value> = b
        .operators
        .iter()
        .map(Value::unspan)
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect();
    let [Value::Call(db, da), Value::Call(cp, ca), ..] = body.as_slice() else {
        return None;
    };
    let Some(Value::Var(r)) = da.first().map(Value::unspan) else {
        return None;
    };
    let is_r = |v: &Value| matches!(v.unspan(), Value::Var(x) if x == r);
    if *db != ops.database
        || *cp != ops.copy_record
        || ca.len() != 3
        || !is_r(&ca[1])
        || mentions(&ca[0], *r)
    {
        return None;
    }
    let last = body.last()?;
    let ends = is_r(last) || matches!(last, Value::Return(x) if is_r(x));
    let middle_ok = body[2..body.len() - 1].iter().all(|o| {
        !mentions(o, *r)
            || matches!(o, Value::Call(op, a) if *op == ops.free_if_distinct
                && a.len() == 2 && is_r(&a[1]) && !mentions(&a[0], *r))
    });
    (ends && middle_ok).then_some((*r, b.operators.len()))
}

/// Does block `n` end in its own `return`?
fn mv_returns(n: &Value) -> bool {
    matches!(n, Value::Block(b) if matches!(b.operators.iter().rev()
        .find(|o| !matches!(o.unspan(), Value::Line(_))).map(Value::unspan), Some(Value::Return(_))))
}

/// A materialised view's return, into `buf` or a local buffer: [`mv_parts`].
fn is_mv_return(n: &Value, _buf: u16, ops: &Ops) -> bool {
    mv_parts(n.unspan(), ops).is_some()
}

/// The buffer a literal block mints into (`OpDatabase(b, tp)`, bare or under its header).
fn literal_buffer(n: &Value, ops: &Ops) -> Option<u16> {
    let Value::Block(b) = n else {
        return None;
    };
    let mut found = None;
    for o in &b.operators {
        o.any_node(&mut |c| {
            if found.is_none()
                && let Value::Call(op, a) = c
                && *op == ops.database
                && let Some(Value::Var(v)) = a.first().map(Value::unspan)
            {
                found = Some(*v);
            }
            found.is_some()
        });
        if found.is_some() {
            break;
        }
    }
    found
}

/// A literal built into a LOCAL buffer (`{ OpDatabase(b, tp); OpSetX(b, …); …; b }`): the
/// exit is the tuple of its fields, and `b` is never minted.
fn is_local_literal(n: &Value, buf: u16, layout: &Layout, ops: &Ops) -> bool {
    literal_buffer(n, ops).is_some_and(|b| b != buf && literal(n, b, layout, ops).is_some())
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
    if let Some(lit) = literal(n, buf, layout, ops) {
        // A literal that ends in its own `return buf` is an exit wherever it stands (an early
        // `if … { return S { … } }`); one answering `buf` as a value must be in a result
        // position.
        return (tail || lit.returns)
            && lit.parts.iter().all(|p| match p {
                Part::Header => true,
                Part::Fill(f) => f
                    .iter()
                    .all(|(_, v)| body_shape(v, buf, layout, ops, false, sh)),
                Part::Free(v) | Part::Other(v) => body_shape(v, buf, layout, ops, false, sh),
                Part::ViewCopy => true,
            });
    }
    match n {
        Value::Return(v) => body_shape(v, buf, layout, ops, true, sh),
        // A materialised view's return ending in its own `return` is an exit wherever it
        // stands, as a literal that does; a local literal answers its value at the tail.
        Value::Block(_)
            if (is_mv_return(n, buf, ops) && (tail || mv_returns(n)))
                || (tail && is_local_literal(n, buf, layout, ops)) =>
        {
            !layout.has_views()
        }
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
        _ if tail => exit_value(n, buf, layout, ops, sh),
        Value::Set(l, inner)
            if let Value::Call(_, args) = inner.unspan()
                && (matches!(args.last().map(Value::unspan), Some(Value::Var(b)) if *b == buf)
                    // A PROMOTED local: the buffer rebound from a call built in a local buffer.
                    || (*l == buf
                        && matches!(args.last().map(Value::unspan), Some(Value::Var(_))))) =>
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
        // A set that may be a DELIVERY local's (`__ret_N = if … { … } else { … }`): judged as
        // an exit value, and kept for `callee_shape` to settle once it knows which locals
        // are returned.
        Value::Set(l, inner) if *l != buf => {
            let mut s2 = BodyShape::default();
            let ok = !layout.has_views() && exit_value(inner, buf, layout, ops, &mut s2);
            let names_buf = mentions(inner, buf);
            sh.sets
                .push((*l, ok, s2.tail_calls, names_buf, s2.returned));
            // What the value holds of its own — a nested delivery local's sets, a forward's
            // bind — is the function's as well.
            sh.sets.extend(s2.sets);
            sh.bound.extend(s2.bound);
            ok || (!names_buf && body_shape(inner, buf, layout, ops, false, sh))
        }
        Value::If(cond, ..) if is_buffer_guard(n, cond, buf, ops) => true,
        // "Free this unless it is the result", the result being the buffer: a tuple is no
        // store's record, so `Exits` makes it a plain free.
        Value::Call(op, args)
            if *op == ops.free_if_distinct
                && matches!(args.as_slice(), [p, w] if !mentions(p, buf)
                    && matches!(w.unspan(), Value::Var(b) if *b == buf)) =>
        {
            true
        }
        Value::Call(op, args)
            if ops.get_kind(*op).is_some()
                && matches!(args.as_slice(), [t, Value::Int(_)] if matches!(t.unspan(), Value::Var(b) if *b == buf)) =>
        {
            sh.buf_reads = true;
            true
        }
        _ => {
            let mut ok = true;
            n.for_each_child(&mut |c| {
                ok = ok && body_shape(c, buf, layout, ops, false, sh);
            });
            ok && !names_var(n, buf)
        }
    }
}

/// `if OpDistinctStore(x, p) OpCopyRecord(x, p, tp) else null` — a call built into PLACE `p`
/// (a pushed element) whose answer `x` is copied there unless it already is `p`: `(x, p)`.
fn place_copy(n: &Value, ops: &Ops) -> Option<(u16, u16)> {
    let Value::If(c, t, e) = n else {
        return None;
    };
    let var = |v: &Value| match v.unspan() {
        Value::Var(x) => Some(*x),
        _ => None,
    };
    let Value::Call(op, a) = c.unspan() else {
        return None;
    };
    let Value::Call(cp, ca) = t.unspan() else {
        return None;
    };
    if *op != ops.distinct_store || *cp != ops.copy_record || a.len() != 2 || ca.len() != 3 {
        return None;
    }
    let (x, p) = (var(&a[0])?, var(&a[1])?);
    (matches!(e.unspan(), Value::Null) && var(&ca[0]) == Some(x) && var(&ca[1]) == Some(p))
        .then_some((x, p))
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

/// One statement of an object literal built into the return buffer.
enum Part<'a> {
    /// The opening test that mints the record.
    Header,
    /// Field writes: `(offset, value)` — a copy into a sub-record writes one per field.
    Fill(Vec<(i32, Value)>),
    /// `OpFreeRefIfDistinct(placeholder, buf)`: frees the placeholder unless it IS the
    /// result, which a tuple never is — so it frees unconditionally.
    Free(&'a Value),
    /// Anything else, which does not name the buffer.
    Other(&'a Value),
    /// A view field's copy into the buffer (`OpAppendVector(OpGetField(buf, off, …), src,
    /// …)`): the tuple carries the view instead, so the copy goes.
    ViewCopy,
}

/// An object literal built into `buf`, as its statements, and whether it ends in
/// `return buf` rather than answering `buf`.
struct Literal<'a> {
    parts: Vec<Part<'a>>,
    returns: bool,
    scope: u16,
}

impl Literal<'_> {
    /// The field offsets in the order the literal writes them.
    fn fill_order(&self) -> Vec<i32> {
        self.parts
            .iter()
            .filter_map(|p| match p {
                Part::Fill(f) => Some(f.iter().map(|(o, _)| *o)),
                _ => None,
            })
            .flatten()
            .collect()
    }
}

/// `{ if … OpDatabase(buf, tp); …; buf }` — or ending in `return buf` — whose statements
/// fill every field of `layout` exactly once.  A field is written by a setter on the buffer
/// or on a field path into its inline sub-records, or by a COPY of a record or sub-record
/// read from a place that may be named once per field.
#[expect(
    clippy::too_many_lines,
    reason = "one recogniser over the literal's statement forms; each arm is one form"
)]
fn literal<'a>(n: &'a Value, buf: u16, layout: &Layout, ops: &Ops) -> Option<Literal<'a>> {
    let Value::Block(b) = n else {
        return None;
    };
    let body: Vec<&Value> = b
        .operators
        .iter()
        .map(Value::unspan)
        .filter(|o| !matches!(o, Value::Line(_)))
        .collect();
    // A literal whose field values are staged before the record is minted
    // (`__stg_… = …; if … OpDatabase(buf, tp); OpSetX(buf, off, __stg_…)`) opens with the
    // staging statements: they do not name the buffer and run where they ran.
    let at = body
        .iter()
        .position(|o| is_header(o, buf, ops) || is_mint(o, buf, ops))?;
    let (staging, body) = body.split_at(at);
    if staging.iter().any(|o| mentions(o, buf)) {
        return None;
    }
    let (_, rest) = body.split_first()?;
    let (last, middle) = rest.split_last()?;
    let is_buf = |v: &Value| matches!(v.unspan(), Value::Var(x) if *x == buf);
    let returns = match last {
        Value::Var(v) if *v == buf => false,
        Value::Return(x) if is_buf(x) => true,
        _ => return None,
    };
    let mut parts: Vec<Part> = staging.iter().map(|o| Part::Other(o)).collect();
    parts.push(Part::Header);
    for o in middle {
        let part = match o {
            // A view field's handle zeroed: its value is the exit's leaf (`Callee::leaves`).
            Value::Call(op, args)
                if *op == ops.set_int4
                    && let [place, Value::Int(off), Value::Int(0)] = args.as_slice()
                    && let Some((v, base)) = place_of(place, ops)
                    && v == buf
                    && layout.kind_at(base + off) == Some(Kind::View) =>
            {
                Part::Fill(vec![(base + off, Value::Null)])
            }
            Value::Call(op, args)
                if *op == ops.append_vector
                    && let [field, src, _] = args.as_slice()
                    && let Some((v, at)) = place_of(field, ops)
                    && v == buf
                    && layout.kind_at(at) == Some(Kind::View)
                    && !mentions(src, buf) =>
            {
                Part::ViewCopy
            }
            Value::Call(op, args)
                if let Some(kind) = ops.set_kind(*op)
                    && let [place, Value::Int(off), value] = args.as_slice()
                    && let Some((v, base)) = place_of(place, ops)
                    && v == buf =>
            {
                let at = base + off;
                if layout.kind_at(at) != Some(kind) || mentions(value, buf) {
                    return None;
                }
                Part::Fill(vec![(at, value.clone())])
            }
            Value::Call(op, args)
                if *op == ops.copy_record
                    && let [src, place, Value::Int(tp)] = args.as_slice()
                    && let Some((v, base)) = place_of(place, ops)
                    && v == buf =>
            {
                let (lo, hi) = layout.extent(base, *tp as u16)?;
                if !pure_place(src, ops) || mentions(src, buf) {
                    return None;
                }
                // `@C137` — a copy into a fresh record: a NULL source leaves the record's
                // default, which a field read of the null record (its null) is not.
                let sub = (*tp as u16) & crate::keys::COPY_TP_MASK;
                Part::Fill(
                    layout
                        .fields
                        .iter()
                        .filter(|(o, _)| *o >= lo && *o < hi)
                        .map(|(o, k)| {
                            let read = vec![src.clone(), Value::Int(o - base)];
                            let value = Value::If(
                                Box::new(Value::Call(ops.bool_from_ref, vec![src.clone()])),
                                Box::new(Value::Call(ops.get_op(*k), read)),
                                Box::new(ops.default_value(sub, o - base, *k)),
                            );
                            (*o, value)
                        })
                        .collect(),
                )
            }
            Value::Call(op, args)
                if *op == ops.free_if_distinct
                    && let [p, witness] = args.as_slice()
                    && is_buf(witness)
                    && !mentions(p, buf) =>
            {
                Part::Free(p)
            }
            other if !mentions(other, buf) => Part::Other(other),
            _ => return None,
        };
        parts.push(part);
    }
    let lit = Literal {
        parts,
        returns,
        scope: b.scope,
    };
    let mut filled = lit.fill_order();
    filled.sort_unstable();
    let whole: Vec<i32> = layout.fields.iter().map(|(o, _)| *o).collect();
    (filled == whole).then_some(lit)
}

/// The literal's opening test — and the loop-hoisted buffer's mint guard, which is the same
/// shape: an `if` that mints a record into `buf` when it holds none, reading nothing but
/// `buf` and calling nothing but the operators it is built from.  A pooled buffer's guard
/// clears the record it still holds in the other arm (`@FR-H-ClearRelease`); with the buffer
/// gone there is nothing to clear either.
/// A bare `OpDatabase(buf, tp)` — a local literal's mint, which no earlier value can have.
fn is_mint(n: &Value, buf: u16, ops: &Ops) -> bool {
    matches!(n, Value::Call(op, a) if *op == ops.database
        && matches!(a.first().map(Value::unspan), Some(Value::Var(b)) if *b == buf))
}

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
                } else if *op != ops.ref_is_null && *op != ops.bool_from_ref && *op != ops.clear {
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
    /// Carriers LENT to a record parameter (`Rewrite::lend`): materialised into one record per
    /// carrier, released where the carrier's own scope ends.
    lent: HashSet<u16>,
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
    /// Field writes: `(variable, offset, kind)` — a carrier LOCAL's become `TuplePut`.
    writes: Vec<(u16, i32, Kind)>,
    /// The variables written field-wise: a written parameter or buffer still declines.
    written: HashSet<u16>,
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
    /// A field path is handed to an admitted parameter: the call reads it into the tuple.
    reads_place: bool,
    /// `(x, p)` per place copy (`place_copy`): `x` bound from a call built into place `p`.
    place_binds: Vec<(u16, u16)>,
    /// A variable handed to a RECORD parameter no tuple serves: `(variable, callee, index)`.
    lent: Vec<(u16, u32, usize)>,
    /// Presence tests `OpConvBoolFromRef(x)` / `OpRefIsNull(x)`, by variable: a nullable
    /// carrier reads its presence word for them.
    tests: Vec<u16>,
    /// When the scanned function returns the tuple: the locals it answers through — its
    /// forwards' and delivery locals — which, like its buffer, are no store's record.
    own_results: HashSet<u16>,
    /// `(buffer, record)` when the scanned function returns the tuple: its own object
    /// literals become tuples, so only what the rewrite keeps of them is scanned — the
    /// buffer writes, the mint and the placeholder test go with the buffer.
    own: Option<(u16, u32)>,
}

/// The plan for function `f`'s body, and what it decides against the admitted sets.
/// `tuple` says `f` itself returns the tuple — the only case a forward's local, built in
/// `f`'s own buffer, may carry it.
#[expect(
    clippy::too_many_lines,
    reason = "one verdict per carrier shape, in one place"
)]
#[expect(
    clippy::many_single_char_names,
    reason = "n/v/d/s/w name the node, variable, definition, scan and world throughout this module"
)]
fn plan_function(data: &Data, w: &World, f: u32, tuple: bool) -> (Plan, Verdict) {
    let def = data.def(f);
    let vars = &def.variables;
    let mut s = Scan {
        own: w.cands.get(&f).filter(|_| tuple).map(|c| (c.buf, c.record)),
        own_results: w
            .cands
            .get(&f)
            .filter(|_| tuple)
            .map(|c| c.forwards.union(&c.delivered).copied().collect())
            .unwrap_or_default(),
        ..Scan::default()
    };
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
    let mut plan = Plan {
        reads_at_call: s.reads_place,
        ..Plan::default()
    };
    // The handed uses of `x` all go to parameters of `record`.
    // A parameter of the same layout, or the NULLABLE layout of a dense carrier's record,
    // which the call widens with `true` (`Rewrite::args`).
    let handed_fits = |x: u16, record: u32| {
        s.handed
            .iter()
            .filter(|(h, _, _)| *h == x)
            .all(|(_, d, i)| {
                w.params.get(&(*d, *i)).is_some_and(|&k| {
                    k == record
                        || (rec_of(k) == rec_of(record)
                            && w.layouts[&k].presence
                            && !w.layouts[&record].presence)
                })
            })
    };
    let reads_fit = |x: u16, record: u32| {
        let layout = &w.layouts[&record];
        s.reads
            .iter()
            .chain(&s.writes)
            .filter(|(l, _, _)| *l == x)
            .all(|(_, off, kind)| layout.kind_at(*off) == Some(*kind))
    };
    let depended = |x: u16| (0..vars.count()).any(|y| vars.tp(y).depend().contains(&x));
    // Every copy of `x` into a place is one of its record into a place the rewrite may name
    // once per field.
    let copies_fit = |x: u16, record: u32| {
        let tp = w.layouts[&record].tp;
        // A view tuple has no record to copy out of: a copy keeps the record form.
        if w.layouts[&record].has_views() {
            return !s.copies.iter().any(|(c, _, _)| *c == x);
        }
        s.copies
            .iter()
            .filter(|(c, _, _)| *c == x)
            .all(|(_, t, _)| *t == tp)
    };
    // The admitted parameters of `f` itself.
    for (&(d, idx), &record) in &w.params {
        if d != f {
            continue;
        }
        let p = vars.var(&def.attributes[idx].name);
        let why = if s.other.contains(&p) || s.lent.iter().any(|(x, ..)| *x == p) {
            Some("a mention that is no read or hand-on")
        } else if !w.layouts[&record].presence && s.tests.contains(&p) {
            Some("a presence test of a dense tuple")
        } else if s.written.contains(&p) || s.null_inits.contains(&p) {
            Some("written")
        } else if s.returns.contains(&p) {
            Some("returned")
        } else if s.frees.contains(&p) || s.fids.iter().any(|&(a, b)| a == p || b == p) {
            Some("freed")
        } else if s.binds.iter().any(|(l, _, _)| *l == p) {
            Some("bound from a call")
        } else if !copies_fit(p, record) {
            Some("a copy into a place it cannot name per field")
        } else if !reads_fit(p, record) || !handed_fits(p, record) {
            Some("a read or hand-on of another shape")
        } else if depended(p) {
            Some("another variable depends on it")
        } else {
            None
        };
        if why.is_none() {
            plan.carriers.insert(p, record);
        } else {
            if let Some(why) = why
                && trace()
            {
                eprintln!(
                    "[ir-valuerec] {}: parameter `{}` declines: {why}",
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
        // The first condition a bind's buffer fails, if any.
        let buffer_why = binds.iter().find_map(|(d, r)| {
            if w.cands[d].record != record {
                return Some("a bind's buffer: binds of different records");
            }
            let Some(r) = *r else {
                return Some("a bind's buffer: no buffer variable");
            };
            // Built into a PLACE and copied there: the place lives on, written from the tuple.
            if s.place_binds.contains(&(l, r)) {
                return None;
            }
            let forward = own == Some(r);
            if buffer_uses[&r] != 1 && r != l {
                Some("a bind's buffer: shared by several binds")
            } else if s.other.contains(&r)
                || s.tests.contains(&r)
                || s.lent.iter().any(|(x, ..)| *x == r)
            {
                Some("a bind's buffer: another mention")
            } else if s.written.contains(&r) {
                Some("a bind's buffer: written field-wise")
            } else if s.reads.iter().any(|(v, _, k)| *v == r && *k == Kind::View) {
                Some("a bind's buffer: a heap field read")
            } else if !forward && vars.is_argument(r) {
                Some("a bind's buffer: a parameter, and no forward of a tuple")
            } else if !forward && (s.guards.contains(&r) || s.aliases.iter().any(|(_, b)| *b == r))
            {
                Some("a bind's buffer: guarded or aliased, and no forward")
            } else if s.fids.iter().any(|&(a, b)| b == r && a != l) {
                Some("a bind's buffer: witnesses another free")
            } else {
                None
            }
        });
        let why = if let Some(why) = buffer_why {
            Some(why)
        } else if s.returns.contains(&l) && own.is_none() {
            Some("returned by a function that keeps its record")
        } else if s.lent.iter().any(|&(x, d, i)| {
            x == l
                && (w.lendable.get(&(d, i)) != Some(&rec_of(record))
                    || s.returns.contains(&l)
                    || w.layouts[&record].has_views())
        }) {
            Some("lent to a record parameter it cannot serve")
        } else if s.other.contains(&l) || (s.tests.contains(&l) && !w.layouts[&record].presence) {
            Some("a mention that is no read, hand-on, bind or free")
        } else if (vars.is_argument(l) && own != Some(l)) || vars.is_captured(l) {
            Some("a parameter or a capture")
        } else if !matches!(vars.tp(l).peel_link().base(), Type::Reference(rd, _) if *rd == rec_of(record))
        {
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
        } else if w.layouts[&record].has_views() && !w.is_value_local(data, f, l) {
            Some("a view carrier the analysis does not hold to its reads (O-ViewField)")
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
            plan.buffers.extend(
                binds
                    .iter()
                    .filter_map(|(_, r)| *r)
                    .filter(|r| !s.place_binds.contains(&(l, *r))),
            );
            if s.lent.iter().any(|(x, ..)| *x == l) {
                plan.lent.insert(l);
            }
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
    // The calls at this function's own exits take the tuple when it returns one.
    if tuple && let Some(c) = w.cands.get(&f) {
        for d in &c.tail_calls {
            *carried_calls.entry(*d).or_default() += 1;
        }
    }
    for (d, n) in &s.cand_calls {
        if carried_calls.get(d).copied().unwrap_or(0) < *n {
            v.owed.insert(*d);
        }
    }
    // A call whose answer is the exit hands its tuple on: its callee must answer one of this
    // layout — or the dense tuple of its record, widened with `true` (`to_tuple`).
    if tuple && let Some(c) = w.cands.get(&f) {
        let fits = |d: &u32| {
            w.cands.get(d).is_some_and(|k| {
                k.record == c.record
                    || (rec_of(k.record) == rec_of(c.record) && !w.layouts[&k.record].presence)
            })
        };
        if !c.tail_calls.iter().all(fits) {
            if trace() {
                eprintln!(
                    "[ir-valuerec] {}: a call at an exit answers no tuple of this layout",
                    def.name()
                );
            }
            v.bad_fns.insert(f);
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
        // `@FR-N-Shape` — a nullable record local is declined on purpose (a null record has no
        // fields to read into the tuple); the test says so instead of missing the wrapper.
        let tp = vars.tp(x);
        if (w.layouts[&record].presence || !matches!(tp, Type::Optional(_)))
            && matches!(tp.base(), Type::Reference(rd, _) if *rd == rec_of(record))
        {
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
            // A field path into a plain record (`ue.prev`): its fields are read at the call.
            if !pure_place(a, &w.ops) || place_of(a, &w.ops).is_none() {
                s.unserved.insert((d, i));
            } else {
                s.reads_place = true;
            }
        } else if let Value::Var(x) = a.unspan()
            && w.lendable.contains_key(&(d, i))
        {
            s.lent.push((*x, d, i));
            continue;
        }
        scan(a, false, w, s);
    }
}

/// A value in a RESULT position: a variable there is returned, and a block's last value is
/// still the result.
fn scan_result(n: &Value, w: &World, s: &mut Scan) {
    // The function's own literal in a result position (`return S { … }`) is scanned whole,
    // as what the rewrite keeps of it ([`scan`]).
    if let Some((buf, record)) = s.own
        && literal(n.unspan(), buf, &w.layouts[&record], &w.ops).is_some()
    {
        scan(n, false, w, s);
        return;
    }
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

#[expect(
    clippy::too_many_lines,
    reason = "one arm per IR node kind the scan reads"
)]
#[expect(
    clippy::many_single_char_names,
    reason = "n/v/d/s/w name the node, variable, definition, scan and world throughout this module"
)]
fn scan(n: &Value, stmt: bool, w: &World, s: &mut Scan) {
    let ops = &w.ops;
    let n = n.unspan();
    // The function's own exit literal: the rewrite keeps its fill values, its other
    // statements and the placeholder free it makes unconditional (`literals_to_tuples`),
    // so those are what is scanned — the buffer it drops is no mention.
    if let Some((buf, record)) = s.own
        && let Some(lit) = literal(n, buf, &w.layouts[&record], ops)
    {
        for part in &lit.parts {
            match part {
                Part::Header | Part::ViewCopy => {}
                Part::Fill(f) => {
                    for (_, v) in f {
                        // A copy's own null arm (`literal`): its test belongs to the copy —
                        // the presence word of a nullable source, `true` for a dense one.
                        match v {
                            Value::If(c, a, b)
                                if matches!(c.unspan(), Value::Call(op, x) if *op == ops.bool_from_ref
                                    && matches!(x.as_slice(), [y] if matches!(y.unspan(), Value::Var(_)))) =>
                            {
                                scan(a, false, w, s);
                                scan(b, false, w, s);
                            }
                            _ => scan(v, false, w, s),
                        }
                    }
                }
                Part::Free(p) => match p.unspan() {
                    Value::Var(x) => {
                        s.frees.insert(*x);
                    }
                    other => scan(other, false, w, s),
                },
                Part::Other(o) => scan(o, true, w, s),
            }
        }
        return;
    }
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
            // A heap field read whole — a view field's read, when the variable carries one.
            if *op == ops.get_field
                && let Some((v, at)) = place_of(n, ops)
            {
                s.reads.push((v, at, Kind::View));
                return;
            }
            // A field read, through a path into an inline sub-record or not.
            if let Some(kind) = ops.get_kind(*op)
                && let [target, Value::Int(off)] = args.as_slice()
                && let Some((v, base)) = place_of(target, ops)
            {
                s.reads.push((v, base + off, kind));
                return;
            }
            // A field write, the same way; the value is scanned like any operand.
            if let Some(kind) = ops.set_kind(*op)
                && let [target, Value::Int(off), value] = args.as_slice()
                && let Some((v, base)) = place_of(target, ops)
            {
                s.writes.push((v, base + off, kind));
                s.written.insert(v);
                scan(value, false, w, s);
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
                // Witnessed by this function's own buffer, which the tuple drops: a plain
                // free (`Exits`).
                if s.own.is_some_and(|(buf, _)| buf == *b) || s.own_results.contains(b) {
                    s.frees.insert(*a);
                } else {
                    s.fids.push((*a, *b));
                }
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
            if (*op == ops.bool_from_ref || *op == ops.ref_is_null)
                && let [a] = args.as_slice()
                && let Value::Var(x) = a.unspan()
            {
                s.tests.push(*x);
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
        Value::If(..) if stmt && let Some((x, p)) = place_copy(n, ops) => {
            s.place_binds.push((x, p));
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
            n.for_each_child(&mut |c| scan(c, false, w, s));
        }
    }
}

/// Rewrite the body of `f` by `plan` — `f` itself, or the twin of the function whose body
/// it copied, which is the function the plan was made for.
fn rewrite_function(data: &mut Data, w: &World, twins: &HashMap<u32, u32>, f: u32, plan: &Plan) {
    let def = &mut data.definitions[f as usize];
    let mut code = std::mem::replace(&mut def.code, Value::Null);
    let scope = match code.unspan() {
        Value::Block(b) => b.scope,
        _ => 0,
    };
    // A record per LENT carrier, declared null at the body's top: minted at its first lend,
    // overwritten whole at each, released where the carrier's own scope ends.
    let mut lends: HashMap<u16, u16> = HashMap::default();
    let mut lent: Vec<u16> = plan.lent.iter().copied().collect();
    lent.sort_unstable();
    for x in lent {
        let rec = Type::Reference(rec_of(plan.carriers[&x]), crate::data::Deps::default());
        lends.insert(x, def.variables.add_unique("vr_lend", &rec, scope));
    }
    let mut decls: Vec<Value> = lends
        .values()
        .map(|t| Value::Set(*t, Box::new(Value::Null)))
        .collect();
    decls.sort_by_key(|v| match v {
        Value::Set(t, _) => *t,
        _ => 0,
    });
    Rewrite {
        w,
        plan,
        twins,
        vars: &mut def.variables,
        scope,
        lends,
    }
    .uses(&mut code);
    if !decls.is_empty()
        && let Value::Block(b) = code.unspan_mut()
    {
        b.operators.splice(0..0, decls);
    }
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
    vars: &'a mut crate::variables::Function,
    /// The body's scope, for the temporaries the rewrite declares.
    scope: u16,
    /// Per LENT carrier, the record it is materialised into (`Rewrite::lend`).
    lends: HashMap<u16, u16>,
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
                    **inner = null_tuple(&w.layouts[&plan.carriers[v]], ops);
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
            Value::Call(op, args)
                if (*op == ops.bool_from_ref || *op == ops.ref_is_null)
                    && let [a] = args.as_slice()
                    && let Value::Var(v) = a.unspan()
                    && let Some(record) = plan.carriers.get(v) =>
            {
                // `if x`, `x == null`: the presence word; a dense tuple is present.
                let present = match w.layouts[record].presence() {
                    Some(i) => Value::TupleGet(*v, i),
                    None => Value::Boolean(true),
                };
                *n = if *op == ops.ref_is_null {
                    Value::Call(ops.not, vec![present])
                } else {
                    present
                };
            }
            Value::Call(op, _) => {
                // A carrier's view field read whole: the tuple's reference IS the field.
                if *op == ops.get_field
                    && let Some((v, at)) = place_of(n, ops)
                    && let Some(record) = plan.carriers.get(&v)
                    && w.layouts[record].kind_at(at) == Some(Kind::View)
                    && let Some(i) = w.layouts[record].index_of(at)
                {
                    *n = Value::TupleGet(v, i);
                    return;
                }
                let Value::Call(op, args) = n else {
                    unreachable!()
                };
                if ops.get_kind(*op).is_some()
                    && let [target, Value::Int(off)] = args.as_slice()
                    && let Some((v, base)) = place_of(target, ops)
                    && let Some(record) = plan.carriers.get(&v)
                    && let Some(i) = w.layouts[record].index_of(base + off)
                {
                    *n = Value::TupleGet(v, i);
                    return;
                }
                // Slice 2: a carrier local's field write is a `TuplePut` of the same field.
                if ops.set_kind(*op).is_some()
                    && let [target, Value::Int(off), _] = args.as_slice()
                    && let Some((v, base)) = place_of(target, ops)
                    && let Some(record) = plan.carriers.get(&v)
                    && let Some(i) = w.layouts[record].index_of(base + off)
                {
                    let at = base + *off;
                    let set = *op;
                    let mut value = args.pop().unwrap_or(Value::Null);
                    self.uses(&mut value);
                    *n = match w.layouts[record].presence() {
                        // `@FR-E-Report` — a write through an ABSENT tuple lands nowhere and is
                        // reported, as the setter's null-record arm does: it IS that arm.
                        Some(p) => Value::If(
                            Box::new(Value::TupleGet(v, p)),
                            Box::new(Value::TuplePut(v, i, Box::new(value.clone()))),
                            Box::new(Value::Call(
                                set,
                                vec![Value::Call(ops.null_ref, Vec::new()), Value::Int(at), value],
                            )),
                        ),
                        None => Value::TuplePut(v, i, Box::new(value)),
                    };
                    return;
                }
                let d = *op;
                self.args(d, args);
            }
            Value::Block(b) | Value::Loop(b) => self.statements(&mut b.operators),
            Value::Insert(list) => self.statements(list),
            _ => n.for_each_child_mut(&mut |c| self.uses(c)),
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
                // A lent carrier's scope ends: the record it was lent as goes with it.
                if let Value::Call(op, a) = o.unspan()
                    && *op == w.ops.free_ref
                    && let [x] = a.as_slice()
                    && let Value::Var(x) = x.unspan()
                    && let Some(&t) = self.lends.get(x)
                {
                    list.push(Value::Call(w.ops.free_ref, vec![Value::Var(t)]));
                }
                continue;
            }
            if let Some((x, p)) = place_copy(o.unspan(), &w.ops)
                && let Some(record) = plan.carriers.get(&x)
            {
                // The call built into place `p`; the tuple is written there — never the same
                // store, so unconditionally.
                let layout = &w.layouts[record];
                let writes = write_fields(layout, &Value::Var(p), x, &w.ops);
                match layout.presence() {
                    Some(pi) => list.push(Value::If(
                        Box::new(Value::TupleGet(x, pi)),
                        Box::new(crate::data::v_block(writes, Type::Void, "vr_copy")),
                        Box::new(Value::Call(
                            w.ops.copy_record,
                            vec![
                                Value::Call(w.ops.null_ref, Vec::new()),
                                Value::Var(p),
                                Value::Int(i32::from(layout.tp)),
                            ],
                        )),
                    )),
                    None => list.extend(writes),
                }
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
                let layout = &w.layouts[record];
                // The place is evaluated ONCE, as the copy evaluated it: bound to a reference
                // when it is more than a variable, a constant or a field path.
                if !pure_place(&place, &w.ops) {
                    let rec = Type::Reference(rec_of(*record), crate::data::Deps::default());
                    let t = self.vars.add_unique("vr_dst", &rec, self.scope);
                    list.extend(bind_ref(t, place, &w.ops));
                    place = Value::Var(t);
                }
                let writes = write_fields(layout, &place, v, &w.ops);
                match layout.presence() {
                    // An ABSENT tuple copies as the record form copies a null source: that
                    // copy's own null arm (`@C137`), under the same type and flags.
                    Some(p) => list.push(Value::If(
                        Box::new(Value::TupleGet(v, p)),
                        Box::new(crate::data::v_block(writes, Type::Void, "vr_copy")),
                        Box::new(Value::Call(
                            w.ops.copy_record,
                            vec![
                                Value::Call(w.ops.null_ref, Vec::new()),
                                place,
                                args[2].clone(),
                            ],
                        )),
                    )),
                    None => list.extend(writes),
                }
                continue;
            }
            self.uses(&mut o);
            list.push(o);
        }
    }

    /// `x` as the record `t`: minted at the first lend, every field written — so a later pass
    /// of a loop reads no stale field — and handed on; an ABSENT nullable tuple lends the null
    /// record, as the record form handed its null.
    fn lend(&self, x: u16, t: u16) -> Value {
        let ops = &self.w.ops;
        let layout = &self.w.layouts[&self.plan.carriers[&x]];
        let rec = Type::Reference(rec_of(self.plan.carriers[&x]), crate::data::Deps::default());
        let mut stmts = vec![Value::If(
            Box::new(Value::Call(ops.ref_is_null, vec![Value::Var(t)])),
            Box::new(Value::Call(
                ops.database,
                vec![Value::Var(t), Value::Int(i32::from(layout.tp))],
            )),
            Box::new(Value::Null),
        )];
        stmts.extend(write_fields(layout, &Value::Var(t), x, ops));
        stmts.push(Value::Var(t));
        let made = crate::data::v_block(stmts, rec, "vr_lend");
        match layout.presence() {
            Some(p) => Value::If(
                Box::new(Value::TupleGet(x, p)),
                Box::new(made),
                Box::new(Value::Call(ops.null_ref, Vec::new())),
            ),
            None => made,
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
            // A field path into a plain record: read field-wise at the call.
            if let Some(record) = w.params.get(&(d, i))
                && !matches!(a.unspan(), Value::Var(_))
                && let Some((root, _)) = place_of(a, &w.ops)
                && !self.plan.carriers.contains_key(&root)
            {
                let place = a.clone();
                *a = read_place_into_tuple(&place, &w.layouts[record], &w.ops);
                continue;
            }
            // A carrier LENT to a record parameter: its record, written whole.
            if !w.params.contains_key(&(d, i))
                && w.lendable.contains_key(&(d, i))
                && let Value::Var(x) = a.unspan()
                && let Some(&t) = self.lends.get(x)
            {
                *a = self.lend(*x, t);
                continue;
            }
            // A dense carrier handed to its record's nullable parameter: widened with `true` —
            // a dense tuple is present.
            if let Some(k) = w.params.get(&(d, i))
                && let Value::Var(x) = a.unspan()
                && let Some(c) = self.plan.carriers.get(x)
                && *c != *k
                && w.layouts[k].presence
            {
                let x = *x;
                let mut elems: Vec<Value> = (0..w.layouts[c].fields.len())
                    .map(|j| Value::TupleGet(x, j as u16))
                    .collect();
                elems.push(Value::Boolean(true));
                *a = Value::Tuple(elems);
                continue;
            }
            self.uses(a);
        }
    }
}

/// `t` names the record `place` answers, without copying it: a record `Set` COPIES
/// (`(B-Copy)`), so the reference is declared null and bound by the raw slot store, as the
/// tail jump binds a pointer (`opt::tail_self`).
fn bind_ref(t: u16, place: Value, ops: &Ops) -> Vec<Value> {
    vec![
        Value::Set(t, Box::new(Value::Null)),
        Value::Call(ops.put_ref, vec![Value::Var(t), place]),
    ]
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

/// The tuple a null record reads as: each field's null — `i64::MIN`, NaN, `false`, and a
/// view's null reference, which reads as the empty vector.
fn null_tuple(layout: &Layout, ops: &Ops) -> Value {
    let mut elems: Vec<Value> = layout
        .fields
        .iter()
        .map(|(_, k)| match k {
            Kind::Int => Value::Long(i64::MIN),
            Kind::Float => Value::Float(f64::NAN),
            Kind::Single => Value::Single(f32::NAN),
            Kind::Bool => Value::Boolean(false),
            Kind::View => Value::Call(ops.null_ref, Vec::new()),
        })
        .collect();
    // Absent: the presence word is `false`.
    if layout.presence {
        elems.push(Value::Boolean(false));
    }
    Value::Tuple(elems)
}

/// The tuple of plain record local `x`'s fields, read at the call — what an admitted
/// parameter receives from an argument that is no carrier.  Reading now equals reading
/// inside the callee because nothing the callee reaches writes a record of this type (the
/// parameter gate `value_records` admits by).
fn read_into_tuple(x: u16, layout: &Layout, ops: &Ops) -> Value {
    read_place_into_tuple(&Value::Var(x), layout, ops)
}

/// [`read_into_tuple`] of a pure PLACE — a variable or a field path, evaluated once per field.
fn read_place_into_tuple(place: &Value, layout: &Layout, ops: &Ops) -> Value {
    let mut elems: Vec<Value> = layout
        .fields
        .iter()
        .map(|(off, k)| Value::Call(ops.get_op(*k), vec![place.clone(), Value::Int(*off)]))
        .collect();
    // Whether a record is there — what the record form's null test reads; a getter of the
    // null record answers the field's null, which is the absent tuple's field.
    if layout.presence {
        elems.push(Value::Call(ops.bool_from_ref, vec![place.clone()]));
    }
    Value::Tuple(elems)
}

fn rewrite_callee(data: &mut Data, w: &World, twins: &HashMap<u32, u32>, d: u32, c: &Callee) {
    let layout = &w.layouts[&c.record];
    let tuple = layout.tuple_type();
    let old = data.def(d).returned().clone();
    let def = &mut data.definitions[d as usize];
    let mut code = std::mem::replace(&mut def.code, Value::Null);
    let mut at = 0;
    literals_to_tuples(
        &mut code,
        c.buf,
        layout,
        &w.ops,
        &mut def.variables,
        (&c.leaves, &mut at),
    );
    debug_assert!(
        c.leaves.is_empty() || at == c.leaves.len(),
        "every literal met its view leaves"
    );
    let scope = match code.unspan() {
        Value::Block(b) => b.scope,
        _ => 0,
    };
    Exits {
        w,
        c,
        layout,
        tuple: tuple.clone(),
        twins,
        scope,
    }
    .body(&mut code, &mut def.variables);
    for &l in &c.delivered {
        def.variables.set_type(l, tuple.clone());
    }
    def.code = code;
    retype_results(&mut def.code, c.record, &old, &tuple);
    // A one-buffer chain made the buffer itself the forward's local.  As a parameter it was
    // declared by the signature; as a local it is declared by its first bind, which sits in
    // a branch — so it is bound to the null tuple first, at the body's own level.
    if c.forwards.contains(&c.buf)
        && let Value::Block(b) = def.code.unspan_mut()
    {
        b.operators
            .insert(0, Value::Set(c.buf, Box::new(null_tuple(layout, &w.ops))));
    }
    def.returned = tuple;
    def.attributes.pop();
    def.variables.drop_argument(c.buf);
}

/// The exits of an admitted function, each delivered as its tuple ([`exit_value`] judged
/// them): every `return`'s value, the body's tail and each set of a delivery local.  The
/// buffer's witness, its guarded free and every "free unless it is the result" whose result
/// is a delivery local or the buffer go — a tuple is never a store's record.
struct Exits<'a> {
    w: &'a World,
    c: &'a Callee,
    layout: &'a Layout,
    tuple: Type,
    twins: &'a HashMap<u32, u32>,
    scope: u16,
}

impl Exits<'_> {
    fn body(&self, code: &mut Value, vars: &mut crate::variables::Function) {
        match code.unspan_mut() {
            Value::Block(b) => {
                self.stmts(&mut b.operators, vars, true);
            }
            other => self.value(other, vars),
        }
    }

    /// A statement list; `tail` says its last value is an exit.
    fn stmts(&self, list: &mut Vec<Value>, vars: &mut crate::variables::Function, tail: bool) {
        let ops = &self.w.ops;
        let buf = self.c.buf;
        let old = std::mem::take(list);
        let final_at = old
            .iter()
            .rposition(|o| !matches!(o.unspan(), Value::Line(_)));
        for (i, mut o) in old.into_iter().enumerate() {
            let alias = matches!(o.unspan(), Value::Set(_, inner) if matches!(inner.unspan(),
                Value::Call(op, a) if *op == ops.ref_alias
                    && matches!(a.as_slice(), [x] if matches!(x.unspan(), Value::Var(b) if *b == buf))));
            let guard = matches!(o.unspan(), Value::If(cond, ..) if is_buffer_guard(o.unspan(), cond, buf, ops));
            if alias || guard {
                continue;
            }
            if tail && Some(i) == final_at && !matches!(o.unspan(), Value::Return(_)) {
                self.value(&mut o, vars);
            } else {
                self.walk(&mut o, vars);
            }
            list.push(o);
        }
    }

    /// A statement: its `return`s, the sets of delivery locals and the frees they witness.
    fn walk(&self, n: &mut Value, vars: &mut crate::variables::Function) {
        let ops = &self.w.ops;
        let witness = |v: &Value| {
            matches!(v.unspan(), Value::Var(x) if *x == self.c.buf
                || self.c.delivered.contains(x) || self.c.forwards.contains(x))
        };
        // A materialised view's return that ends in its own `return` is an exit where it stands.
        if mv_returns(n.unspan())
            && let Some((r, _)) = mv_parts(n.unspan(), ops)
        {
            self.mv(n.unspan_mut(), r, vars);
            return;
        }
        match n.unspan_mut() {
            Value::Return(x) => self.value(x, vars),
            Value::Set(l, e) if self.c.delivered.contains(l) => self.value(e, vars),
            Value::Call(op, a) if *op == ops.free_if_distinct && a.len() == 2 && witness(&a[1]) => {
                let p = a[0].clone();
                *n = Value::Call(ops.free_ref, vec![p]);
            }
            Value::Block(b) | Value::Loop(b) => self.stmts(&mut b.operators, vars, false),
            Value::Insert(l) => self.stmts(l, vars, false),
            other => other.for_each_child_mut(&mut |c| self.walk(c, vars)),
        }
    }

    /// `T`: exit value `n` as the tuple.
    fn value(&self, n: &mut Value, vars: &mut crate::variables::Function) {
        let ops = &self.w.ops;
        let layout = self.layout;
        let buf = self.c.buf;
        let n = n.unspan_mut();
        if let Some((r, _)) = mv_parts(n, ops) {
            self.mv(n, r, vars);
            return;
        }
        if let Some(lb) = literal_buffer(n, ops)
            && lb != buf
            && literal(n, lb, layout, ops).is_some()
        {
            literals_to_tuples(n, lb, layout, ops, vars, (&[], &mut 0));
            return;
        }
        match n {
            Value::Tuple(_) => {}
            Value::Block(b) if b.name == "Tuple" => {}
            Value::Null => *n = null_tuple(layout, ops),
            Value::Call(op, a) if *op == ops.null_ref && a.is_empty() => {
                *n = null_tuple(layout, ops);
            }
            Value::Return(x) => self.value(x, vars),
            Value::If(_, a, e) => {
                self.value(a, vars);
                self.value(e, vars);
            }
            Value::Var(v) => {
                let v = *v;
                if v != buf && !self.c.delivered.contains(&v) && !self.c.forwards.contains(&v) {
                    // A VIEW: its fields, read where it is returned.  A local that OWNS its
                    // record (no dependency: a lifted copy the record form handed to the
                    // caller) is freed once they are read — the caller does not take it.
                    // @FR-O-Proxy asks free — so @FR-O-Override is asked beside it: a local
                    // marked skip-free is a borrow nobody gave a dep, and its store is not ours.
                    let tp = vars.tp(v);
                    if !vars.is_argument(v)
                        && tp.depend().is_empty()
                        && !vars.is_skip_free(v)
                        && matches!(tp.base(), Type::Reference(..))
                    {
                        let tv = vars.add_unique("vr_t", &self.tuple, self.scope);
                        *n = crate::data::v_block(
                            vec![
                                Value::Set(tv, Box::new(read_into_tuple(v, layout, ops))),
                                Value::Call(ops.free_ref, vec![Value::Var(v)]),
                                Value::Var(tv),
                            ],
                            self.tuple.clone(),
                            "vr_owned_view",
                        );
                    } else {
                        *n = read_into_tuple(v, layout, ops);
                    }
                }
            }
            Value::Block(b) => {
                b.result = self.tuple.clone();
                self.stmts(&mut b.operators, vars, true);
            }
            Value::Call(d, args)
                if matches!(args.last().map(Value::unspan), Some(Value::Var(_))) =>
            {
                args.pop();
                let callee = self.w.cands.get(d).map(|k| k.record);
                if let Some(t) = self.twins.get(d) {
                    *d = *t;
                }
                // A dense callee's tuple where this nullable one is expected: widened with its
                // presence word — a dense tuple is present.
                if let Some(k) = callee
                    && layout.presence
                    && !self.w.layouts[&k].presence
                {
                    let dense = self.w.layouts[&k].tuple_type();
                    let t = vars.add_unique("vr_w", &dense, self.scope);
                    let call = std::mem::replace(n, Value::Null);
                    let mut elems: Vec<Value> = (0..self.w.layouts[&k].fields.len())
                        .map(|i| Value::TupleGet(t, i as u16))
                        .collect();
                    elems.push(Value::Boolean(true));
                    *n = crate::data::v_block(
                        vec![Value::Set(t, Box::new(call)), Value::Tuple(elems)],
                        self.tuple.clone(),
                        "vr_widen",
                    );
                }
            }
            _ => {}
        }
    }

    /// A materialised view's return: its source read field-wise — the copy into the buffer,
    /// the buffer's mint and the buffer go; the frees it witnessed run after the read.
    fn mv(&self, n: &mut Value, r: u16, vars: &mut crate::variables::Function) {
        let ops = &self.w.ops;
        let Value::Block(b) = n else { return };
        let body: Vec<Value> = std::mem::take(&mut b.operators)
            .into_iter()
            .filter(|o| !matches!(o.unspan(), Value::Line(_)))
            .collect();
        let returns = matches!(body.last().map(Value::unspan), Some(Value::Return(_)));
        let Some(Value::Call(_, ca)) = body.get(1).map(Value::unspan) else {
            return;
        };
        let src = ca[0].clone();
        let rec = Type::Reference(rec_of(self.c.record), crate::data::Deps::default());
        let rv = vars.add_unique("vr_src", &rec, self.scope);
        let tv = vars.add_unique("vr_t", &self.tuple, self.scope);
        let mut out = bind_ref(rv, src, ops);
        out.push(Value::Set(
            tv,
            Box::new(read_into_tuple(rv, self.layout, ops)),
        ));
        for o in &body[2..body.len() - 1] {
            match o.unspan() {
                Value::Call(op, a)
                    if *op == ops.free_if_distinct
                        && a.len() == 2
                        && matches!(a[1].unspan(), Value::Var(x) if *x == r) =>
                {
                    out.push(Value::Call(ops.free_ref, vec![a[0].clone()]));
                }
                _ => out.push(o.clone()),
            }
        }
        out.push(if returns {
            Value::Return(Box::new(Value::Var(tv)))
        } else {
            Value::Var(tv)
        });
        b.operators = out;
        b.result = self.tuple.clone();
    }
}

/// The blocks in RESULT positions — the body, a returned block and their tails — are typed
/// by the record the function returned (possibly with the buffer as its dep); they now
/// answer the tuple.  A block elsewhere keeps its type: a local record of the same type is
/// no result.
fn retype_results(n: &mut Value, record: u32, old: &Type, tuple: &Type) {
    match n.unspan_mut() {
        Value::Block(b) => {
            if b.result == *old
                || matches!(b.result.peel_link().base(), Type::Reference(r, _) if *r == rec_of(record))
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
    n.for_each_child_mut(&mut |c| retype_returns(c, record, old, tuple));
}

/// Every literal built into `buf` becomes its tuple, in SCHEMA order.  The statements run
/// where the literal ran them: when the literal writes its fields in schema order and nothing
/// follows its first write, the values are the tuple's own elements; otherwise each value is
/// bound to a temporary where the literal wrote it, and the tuple reads the temporaries — so
/// a field expression with an effect runs where the program put it.  A free the buffer only
/// witnessed frees unconditionally, and the opening test goes.  A [`Kind::View`] field
/// carries the literal's leaf (`leaves`, the next one by `.1`), read where the tuple is
/// built: it reads only a variable, so no statement it could follow changes it.
fn literals_to_tuples(
    n: &mut Value,
    buf: u16,
    layout: &Layout,
    ops: &Ops,
    vars: &mut crate::variables::Function,
    leaves: (&[HashMap<i32, Value>], &mut usize),
) {
    let (all, at) = leaves;
    let n = n.unspan_mut();
    let Some(lit) = literal(n, buf, layout, ops) else {
        n.for_each_child_mut(&mut |c| literals_to_tuples(c, buf, layout, ops, vars, (all, at)));
        return;
    };
    let views = all.get(*at);
    if layout.has_views() {
        *at += 1;
    }
    let schema: Vec<i32> = layout.fields.iter().map(|(o, _)| *o).collect();
    let first_fill = lit.parts.iter().position(|p| matches!(p, Part::Fill(_)));
    let after_fill = first_fill.is_some_and(|i| {
        lit.parts[i..]
            .iter()
            .any(|p| matches!(p, Part::Free(_) | Part::Other(_)))
    });
    let staged = after_fill || lit.fill_order() != schema;
    let mut stmts: Vec<Value> = Vec::new();
    let mut values: HashMap<i32, Value> = HashMap::default();
    for part in &lit.parts {
        match part {
            Part::Header => {}
            Part::Fill(f) => {
                for (off, v) in f {
                    if let Some(leaf) = views.and_then(|m| m.get(off)) {
                        values.insert(*off, leaf.clone());
                        continue;
                    }
                    let value = if staged {
                        let t = vars.add_unique("vf", &layout.elem_type(*off), lit.scope);
                        stmts.push(Value::Set(t, Box::new(v.clone())));
                        Value::Var(t)
                    } else {
                        v.clone()
                    };
                    values.insert(*off, value);
                }
            }
            Part::Free(p) => stmts.push(Value::Call(ops.free_ref, vec![(*p).clone()])),
            Part::Other(o) => stmts.push((*o).clone()),
            Part::ViewCopy => {}
        }
    }
    let mut elems: Vec<Value> = schema
        .iter()
        .map(|o| values.remove(o).expect("filled"))
        .collect();
    // A build is present.
    if layout.presence {
        elems.push(Value::Boolean(true));
    }
    let tuple = Value::Tuple(elems);
    let result = if lit.returns {
        Value::Return(Box::new(tuple))
    } else {
        tuple
    };
    let scope = lit.scope;
    *n = if stmts.is_empty() {
        result
    } else {
        stmts.push(result);
        Value::Block(Box::new(Block {
            name: "Tuple",
            operators: stmts,
            result: layout.tuple_type(),
            scope,
            var_size: 0,
        }))
    };
}
