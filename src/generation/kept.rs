// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! `@FR-R-Header`'s KEPT clause — a vector field of a record parameter whose header a body
//! holds from entry to exit, refreshed where the body itself changes it and handed down to
//! the callees that keep it too (worklist L7).
//!
//! The function clause holds a parameter's header when nothing the body runs can change it;
//! the twin clause hands a caller's header to a callee that writes no store.  Neither covers
//! a chain of callees that each walk `m.chunks` and one of which appends to it.  This clause
//! asks the narrower question the apart clause asks of a loop: what can change THIS header?
//! Only something that names its record — `m`, a value whose type reaches `m`'s record type,
//! or a vector of the field's element type, which may be a view of the field.  Two kinds of
//! statement may: an append to the field itself, and a call handed such a value.  After
//! either the holder is derived again ([`crate::generation::Output`]'s `refresh_kept`), unless
//! the call went to the callee's kept twin, which carries the holder by `&mut` and keeps it
//! current itself.  Every other writer must name none of those values.  A header's record and
//! length are the vector's own, so a claim anywhere else — the undo stack's push beside the
//! map — changes neither; no BASE is kept, since a claim may move the store's memory.
//!
//! The verdict ([`kept_paths`]) is memoised per function and read by the emitter for three
//! things: the holders a body binds at entry (or receives, in its `__kh` twin), the twin a
//! call may take, and the statements after which a holder is refreshed ([`refreshes`]).

use super::hoist::{
    IN_PLACE_SET_OPS, call_writes_store, in_place_copy, is_scalar, local_owns_store, mint_path,
    native_op_is_store_free, plain_record_type, rebound_vars, type_reaches, vector_path,
};
use crate::data::{Block, Data, DefType, Type, Value};
use crate::database::{Parts, Stores};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// The bit a ranged-variant request carries for the `__kh` twin's ranged form (beside
/// `decode_view::DV_BIT`): definition numbers stay far below it.
pub const KH_BIT: u32 = 1 << 30;

/// One kept field: the parameter, the field's offset, the path the holder is derived from
/// (`OpGetField(Var(param), off, vector_tp)`) and how many header reads it serves — a read
/// inside a loop counts twice, the bar for binding at entry being two.
#[derive(Debug, Clone)]
pub struct KeptPath {
    pub param: u16,
    pub off: i64,
    pub path: Value,
    pub uses: usize,
    /// The reads of the field in the body itself (a walk, an element, a length), loop-weighted.
    pub own: usize,
    /// The calls that hand the field on: `(callee, its parameter, weight, inside a loop)` —
    /// what the emitter weighs, knowing which twin each call will take
    /// (`Output::kept_effective_uses`).
    pub via: Vec<(u32, u16, usize, bool)>,
}

/// The memo of [`kept_paths`] and of the questions it asks along the way.
#[derive(Default)]
pub struct KeptCache {
    memo: HashMap<u32, Option<Rc<Vec<KeptPath>>>>,
    locks: HashMap<u32, bool>,
    writes: HashMap<u32, bool>,
    fields: HashMap<(u32, u16, u32, Option<u16>), bool>,
    clear: HashMap<u32, bool>,
}

/// `LOFT_NO_KEPT_HEADER=1` switches the clause off (`@FR-R-Switch`).
#[must_use]
pub fn kept_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| !std::env::var("LOFT_NO_KEPT_HEADER").is_ok_and(|v| v != "0"))
}

fn trace_enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("LOFT_TRACE_KEPT").is_ok_and(|v| v != "0"))
}

/// The fields `d_nr` keeps, in its twin's parameter order — `None` when it keeps none.
/// Memoised; the memo is seeded `None` while the walk is open, so a recursion sees no kept
/// field in itself and only ever withholds a twin.
pub fn kept_paths(
    d_nr: u32,
    data: &Data,
    stores: &Stores,
    cache: &mut KeptCache,
) -> Option<Rc<Vec<KeptPath>>> {
    if !kept_enabled() {
        return None;
    }
    if let Some(known) = cache.memo.get(&d_nr) {
        return known.clone();
    }
    cache.memo.insert(d_nr, None);
    let answer = kept_inner(d_nr, data, stores, cache).map(Rc::new);
    cache.memo.insert(d_nr, answer.clone());
    answer
}

/// A loft function whose body this clause can read: a block, no Rust template, no iterator.
fn walkable(data: &Data, d_nr: u32) -> Option<&Block> {
    if (d_nr as usize) >= data.definitions.len() {
        return None;
    }
    let def = data.def(d_nr);
    if def.def_type() != DefType::Function
        || !def.is_loft_defined()
        || !def.rust().is_empty()
        || matches!(def.returned().base(), Type::Iterator(_, _))
    {
        return None;
    }
    match def.code().unspan() {
        Value::Block(b) => Some(b),
        _ => None,
    }
}

/// The plain vector field of record type `t` at byte `off`: the vector's type number.
fn vector_field(stores: &Stores, t: u16, off: i64) -> Option<u16> {
    let Parts::Struct(fields) = &stores.types.get(t as usize)?.parts else {
        return None;
    };
    (0..fields.len()).find_map(|i| {
        let (pos, vtp) = stores.plain_vector_field(t, u16::try_from(i).ok()?)?;
        (i64::from(pos) == off).then_some(vtp)
    })
}

fn kept_inner(
    d_nr: u32,
    data: &Data,
    stores: &Stores,
    cache: &mut KeptCache,
) -> Option<Vec<KeptPath>> {
    let body = walkable(data, d_nr)?;
    let def = data.def(d_nr);
    let vars = def.variables();
    let params = u16::try_from(def.attributes().len()).ok()?;
    if may_lock(d_nr, data, &mut cache.locks, &mut HashSet::new()) {
        return None;
    }
    let keys = kept_keys(d_nr, body, data, stores, cache);
    let get_field = data.def_nr("OpGetField");
    let mut out = Vec::new();
    for (p, off, vtp) in keys {
        let Type::Reference(t_def, _) = vars.tp(p).peel_link() else {
            continue;
        };
        let elm = match &stores.types.get(usize::from(vtp)).map(|t| &t.parts) {
            Some(Parts::Vector(e)) => Some(*e),
            _ => None,
        };
        let mut walk = KeptWalk {
            data,
            stores,
            d_nr,
            vars,
            params,
            p,
            off,
            t_def: *t_def,
            elm,
            why: None,
            uses: 0,
            own: 0,
            via: Vec::new(),
            strict: false,
            cache,
        };
        walk.block(body, 0);
        if trace_enabled() {
            eprintln!(
                "kept: {} {}+{off}: {}",
                def.name(),
                vars.name(p),
                walk.why
                    .map_or_else(|| format!("kept, {} use(s)", walk.uses), str::to_string)
            );
        }
        if walk.why.is_none() && walk.uses > 0 {
            let (uses, own, via) = (walk.uses, walk.own, std::mem::take(&mut walk.via));
            out.push(KeptPath {
                param: p,
                off,
                path: Value::Call(
                    get_field,
                    vec![
                        Value::Var(p),
                        Value::Int(i32::try_from(off).ok()?),
                        Value::Int(i32::from(vtp)),
                    ],
                ),
                uses,
                own,
                via,
            });
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The candidate keys of `d_nr`: `(parameter, offset, vector type)` for a vector field of a
/// record parameter the body reads, or hands on to a callee that keeps it — the parameter
/// never rebound, no hidden buffer.
fn kept_keys(
    d_nr: u32,
    body: &Block,
    data: &Data,
    stores: &Stores,
    cache: &mut KeptCache,
) -> Vec<(u16, i64, u16)> {
    let def = data.def(d_nr);
    let vars = def.variables();
    let Ok(params) = u16::try_from(def.attributes().len()) else {
        return Vec::new();
    };
    let rebound = rebound_vars(body, data, vars);
    let mut keys: Vec<(u16, i64, u16)> = Vec::new();
    let add = |p: u16, off: i64, vtp: u16, keys: &mut Vec<(u16, i64, u16)>| {
        if p < params
            && !rebound.contains(&p)
            && !def.attributes()[usize::from(p)].hidden
            && !keys.iter().any(|(q, o, _)| (*q, *o) == (p, off))
            && let Some(t) = plain_record_type(data, vars.tp(p))
            && vector_field(stores, t, off) == Some(vtp)
        {
            keys.push((p, off, vtp));
        }
    };
    def.code().any_node(&mut |n| {
        if let Value::Call(d, args) = n
            && (*d as usize) < data.definitions.len()
            && data.def(*d).name() == "OpGetField"
            && let (Some(Value::Var(p)), Some(Value::Int(off)), Some(Value::Int(vtp))) = (
                args.first().map(Value::unspan),
                args.get(1).map(Value::unspan),
                args.get(2).map(Value::unspan),
            )
            && let Ok(vtp) = u16::try_from(*vtp)
        {
            add(*p, i64::from(*off), vtp, &mut keys);
        }
        false
    });
    let mut callees: Vec<(u32, Vec<Value>)> = Vec::new();
    def.code().any_node(&mut |n| {
        if let Value::Call(g, args) = n
            && walkable(data, *g).is_some()
            && *g != d_nr
        {
            callees.push((*g, args.clone()));
        }
        false
    });
    for (g, args) in &callees {
        let Some(gk) = kept_paths(*g, data, stores, cache) else {
            continue;
        };
        for k in gk.iter() {
            if let Some(Value::Var(p)) = args.get(usize::from(k.param)).map(Value::unspan)
                && let Value::Call(_, pargs) = k.path.unspan()
                && let Some(Value::Int(vtp)) = pargs.get(2).map(Value::unspan)
                && let Ok(vtp) = u16::try_from(*vtp)
            {
                add(*p, k.off, vtp, &mut keys);
            }
        }
    }
    keys
}

/// Can running `d_nr` lock or unlock a store — the one store-wide fact a header carries
/// (`VecHeader::locked`)?  `d#lock = …` is `set_store_lock`, a native; a loft callee is asked
/// the same, a cycle assuming no.  A call through a function value answers yes.
fn may_lock(
    d_nr: u32,
    data: &Data,
    memo: &mut HashMap<u32, bool>,
    active: &mut HashSet<u32>,
) -> bool {
    if let Some(&known) = memo.get(&d_nr) {
        return known;
    }
    if !active.insert(d_nr) {
        return false;
    }
    let mut found = false;
    let mut callees: Vec<u32> = Vec::new();
    data.def(d_nr).code().any_node(&mut |n| {
        match n {
            Value::Call(g, _) if (*g as usize) < data.definitions.len() => {
                let name = data.def(*g).name();
                if name.ends_with("set_store_lock") {
                    found = true;
                } else if walkable(data, *g).is_some() {
                    callees.push(*g);
                }
            }
            Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => found = true,
            _ => {}
        }
        found
    });
    if !found {
        found = callees.into_iter().any(|g| may_lock(g, data, memo, active));
    }
    active.remove(&d_nr);
    memo.insert(d_nr, found);
    found
}

/// The verdict for one key over one body.
struct KeptWalk<'a> {
    data: &'a Data,
    stores: &'a Stores,
    d_nr: u32,
    vars: &'a crate::variables::Function,
    params: u16,
    p: u16,
    off: i64,
    /// The definition of `p`'s record type.
    t_def: u32,
    /// The field's element type number, for a vector local that may view it.
    elm: Option<u16>,
    why: Option<&'static str>,
    uses: usize,
    own: usize,
    via: Vec<(u32, u16, usize, bool)>,
    /// [`untouched_field`]'s question: nothing may change the field at all — no append to
    /// it, no call that may — rather than every change being one a holder follows.
    strict: bool,
    cache: &'a mut KeptCache,
}

impl KeptWalk<'_> {
    fn fail(&mut self, why: &'static str) {
        if self.why.is_none() {
            self.why = Some(why);
        }
    }

    fn weight(depth: u32) -> usize {
        if depth > 0 { 2 } else { 1 }
    }

    fn is_key(&self, v: &Value) -> bool {
        vector_path(self.data, v).is_some_and(|(r, o)| r == self.p && o == [self.off])
    }

    /// May variable `x` name `p`'s record or the kept vector?  `p` itself; a value whose type
    /// can hold a record of `p`'s type; a vector whose element is the field's — a view of
    /// the field is one.  Never a scalar, a text, or a local that owns a store of its own.
    fn reaches(&self, x: u16) -> bool {
        if x == self.p {
            return true;
        }
        if x >= self.vars.count() {
            return true;
        }
        let tp = self.vars.tp(x);
        if is_scalar(tp) || matches!(tp.base(), Type::Text(_)) {
            return false;
        }
        if local_owns_store(self.data, self.d_nr, self.vars, self.params, x) {
            return false;
        }
        if type_reaches(self.data, tp, self.t_def, &mut HashSet::new()) {
            return true;
        }
        same_vector(self.data, tp, self.elm)
    }

    /// Does `v` mention a variable that [`Self::reaches`]?
    fn mentions(&self, v: &Value) -> bool {
        v.any_node(&mut |n| match n {
            Value::Var(x) | Value::TupleGet(x, _) | Value::TuplePut(x, _, _) | Value::Set(x, _) => {
                self.reaches(*x)
            }
            Value::Iter(x, _, _, _) => self.reaches(*x),
            _ => false,
        })
    }

    fn block(&mut self, b: &Block, depth: u32) {
        let last = b.operators.len().saturating_sub(1);
        for (i, op) in b.operators.iter().enumerate() {
            // A statement: an operator the block emits as one — any but the value of a
            // block that yields one.
            let stmt = i < last || matches!(b.result, Type::Void);
            self.stmt(op, stmt, depth);
        }
    }

    fn stmt(&mut self, op: &Value, stmt: bool, depth: u32) {
        match op.unspan() {
            Value::Set(v, rhs) => {
                if *v == self.p {
                    self.fail("the parameter is rebound");
                }
                if self.is_key(rhs) {
                    self.uses += Self::weight(depth);
                    self.own += Self::weight(depth);
                }
                match rhs.unspan() {
                    Value::Call(d, args) if stmt => self.call(*d, args, true, depth),
                    _ => self.expr(rhs, depth),
                }
            }
            Value::Call(d, args) if stmt => self.call(*d, args, true, depth),
            _ => self.expr(op, depth),
        }
    }

    fn expr(&mut self, v: &Value, depth: u32) {
        match v.unspan() {
            Value::Block(b) => self.block(b, depth),
            Value::Loop(lp) => {
                // A loop's own operators are not statements this clause refreshes after.
                for op in &lp.operators {
                    self.stmt(op, false, depth + 1);
                }
            }
            Value::If(c, t, e) => {
                self.expr(c, depth);
                self.expr(t, depth);
                self.expr(e, depth);
            }
            Value::Set(x, rhs) => {
                if *x == self.p {
                    self.fail("the parameter is rebound");
                }
                if self.is_key(rhs) {
                    self.uses += Self::weight(depth);
                    self.own += Self::weight(depth);
                }
                self.expr(rhs, depth);
            }
            Value::TuplePut(x, _, rhs) => {
                if *x == self.p {
                    self.fail("the parameter is rebound");
                }
                self.expr(rhs, depth);
            }
            Value::Call(d, args) => self.call(*d, args, false, depth),
            Value::CallRef(_, _) | Value::Parallel(_) | Value::Yield(_) => {
                self.fail("a call through a function value, a par or a yield");
            }
            Value::Return(x) | Value::Drop(x) => self.expr(x, depth),
            Value::Insert(vs) | Value::Tuple(vs) => {
                for x in vs {
                    self.expr(x, depth);
                }
            }
            Value::Iter(_, a, b, c) => {
                self.expr(a, depth);
                self.expr(b, depth);
                self.expr(c, depth);
            }
            _ => {}
        }
    }

    fn call(&mut self, d: u32, args: &[Value], stmt: bool, depth: u32) {
        for a in args {
            self.expr(a, depth);
        }
        if (d as usize) >= self.data.definitions.len() {
            self.fail("an unknown call");
            return;
        }
        let def = self.data.def(d);
        let name = def.name();
        // A header read of the key itself: an element address or a length.
        if matches!(
            name,
            "OpGetVector" | "OpGetVectorNullable" | "OpLengthVector" | "t_6vector_len"
        ) && args.first().is_some_and(|a| self.is_key(a))
        {
            self.uses += Self::weight(depth);
            self.own += Self::weight(depth);
        }
        if walkable(self.data, d).is_some() {
            let reaching: Vec<usize> = (0..args.len())
                .filter(|i| self.mentions(&args[*i]))
                .collect();
            // Reads through a callee that keeps the field at the position `p` is handed to.
            if let Some(gk) = kept_paths(d, self.data, self.stores, self.cache) {
                for k in gk.iter() {
                    if k.off == self.off
                        && matches!(args.get(usize::from(k.param)).map(Value::unspan), Some(Value::Var(x)) if *x == self.p)
                    {
                        self.uses += k.uses * Self::weight(depth);
                        self.via.push((d, k.param, Self::weight(depth), depth > 0));
                    }
                }
            }
            if reaching.is_empty() || (stmt && !self.strict) {
                // Names nothing that reaches the field, or a statement the holder is
                // refreshed after (or that hands the holder down).
                return;
            }
            for i in reaching {
                let untouched = u16::try_from(i).is_ok_and(|pos| {
                    untouched_field(
                        d,
                        pos,
                        self.t_def,
                        self.elm,
                        self.data,
                        self.stores,
                        self.cache,
                    )
                });
                if !untouched {
                    self.fail("a call inside an expression may change it");
                    return;
                }
            }
            return;
        }
        if matches!(name, "OpNewRecord" | "OpFinishRecord")
            && mint_path(self.data, self.stores, name, args, self.vars)
                .is_some_and(|(r, o)| r == self.p && o == [self.off])
        {
            if self.strict {
                self.fail("an append");
            } else if !stmt {
                self.fail("an append inside an expression");
            } else if depth > 0 {
                self.fail("an append inside a loop");
            }
            return;
        }
        if name.ends_with("set_store_lock") {
            self.fail("a store lock");
            return;
        }
        if native_op_is_store_free(def)
            || !call_writes_store(
                d,
                self.data,
                &mut self.cache.writes,
                &mut HashSet::new(),
                None,
            )
            || IN_PLACE_SET_OPS.contains(&name)
            || in_place_copy(Some(self.stores), name, args).is_some()
        {
            return;
        }
        if args.iter().any(|a| self.mentions(a)) {
            self.fail("a writer names the record or the vector");
        }
    }
}

/// Is `tp` a vector whose element may be `elm` — the same record type, or, for a field of
/// another element kind, any vector at all?
fn same_vector(data: &Data, tp: &Type, elm: Option<u16>) -> bool {
    let mut t = tp;
    while let Type::RefVar(e) | Type::Optional(e) | Type::Rewritten(e) = t {
        t = e;
    }
    let Type::Vector(e, _) = t else {
        return false;
    };
    match (e.peel_link(), elm) {
        (Type::Reference(d, _), Some(k)) => {
            let known = data.def(*d).known_type();
            known == k || known == u16::MAX
        }
        _ => true,
    }
}

/// Does statement `op` change the header of the vector at `(p, off)` in a way the holder
/// must be refreshed after — the append group's two ops on the field, or a call to a loft
/// function handed a value that reaches it?  The emitter asks this after each statement of
/// a body that holds the key; the verdict above admitted the body only if every change is
/// one of these.  `twin` answers whether the call was emitted as the callee's kept twin,
/// which keeps the holder current itself.
pub fn refreshes(
    op: &Value,
    data: &Data,
    stores: &Stores,
    d_nr: u32,
    p: u16,
    off: i64,
    twin: &dyn Fn(&[Value]) -> bool,
) -> bool {
    let call = match op.unspan() {
        Value::Set(_, rhs) => rhs.unspan(),
        other => other,
    };
    let Value::Call(d, args) = call else {
        return false;
    };
    if (*d as usize) >= data.definitions.len() {
        return false;
    }
    let def = data.def(d_nr);
    let vars = def.variables();
    let name = data.def(*d).name();
    if matches!(name, "OpNewRecord" | "OpFinishRecord") {
        return mint_path(data, stores, name, args, vars)
            .is_some_and(|(r, o)| r == p && o == [off]);
    }
    if walkable(data, *d).is_none() || twin(args) {
        return false;
    }
    let Type::Reference(t_def, _) = vars.tp(p).peel_link() else {
        return true;
    };
    let Some(t) = plain_record_type(data, vars.tp(p)) else {
        return true;
    };
    let elm = vector_field(stores, t, off).and_then(|vtp| {
        match &stores.types.get(usize::from(vtp)).map(|x| &x.parts) {
            Some(Parts::Vector(e)) => Some(*e),
            _ => None,
        }
    });
    let mut cache = KeptCache::default();
    let walk = KeptWalk {
        data,
        stores,
        d_nr,
        vars,
        params: u16::try_from(def.attributes().len()).unwrap_or(u16::MAX),
        p,
        off,
        t_def: *t_def,
        elm,
        why: None,
        uses: 0,
        own: 0,
        via: Vec::new(),
        strict: false,
        cache: &mut cache,
    };
    // A plain call to a callee that leaves the field alone through every reaching argument
    // needs no refresh.
    let (t_def, elm) = (walk.t_def, walk.elm);
    let reaching: Vec<usize> = (0..args.len())
        .filter(|i| walk.mentions(&args[*i]))
        .collect();
    reaching.into_iter().any(|i| {
        u16::try_from(i).map_or(true, |pos| {
            !untouched_field(*d, pos, t_def, elm, data, stores, &mut cache)
        })
    })
}

/// Does `d_nr` leave alone every vector of the kept kind that its parameter `pos` may name —
/// a record of type `t_def` and its fields, or a vector of element `elm`, which may be a view
/// of the field?  The kept walk over the callee in its strict form: no append to such a
/// vector, no writer naming one, no call that may change one, nothing rebinding the
/// parameter.  `param_untouched` asks this of a vector parameter and misses a local VIEW of a
/// record's field (`v = m.cs; v += […]`), which the mention test sees.  Memoised; a recursion
/// assumes the answer it is computing.
fn untouched_field(
    d_nr: u32,
    pos: u16,
    t_def: u32,
    elm: Option<u16>,
    data: &Data,
    stores: &Stores,
    cache: &mut KeptCache,
) -> bool {
    let key = (d_nr, pos, t_def, elm);
    if let Some(&known) = cache.fields.get(&key) {
        return known;
    }
    let Some(body) = walkable(data, d_nr) else {
        return false;
    };
    let def = data.def(d_nr);
    let Ok(params) = u16::try_from(def.attributes().len()) else {
        return false;
    };
    if pos >= params {
        return false;
    }
    cache.fields.insert(key, true);
    let mut walk = KeptWalk {
        data,
        stores,
        d_nr,
        vars: def.variables(),
        params,
        p: pos,
        off: -1,
        t_def,
        elm,
        why: None,
        uses: 0,
        own: 0,
        via: Vec::new(),
        strict: true,
        cache,
    };
    walk.block(body, 0);
    let answer = walk.why.is_none();
    cache.fields.insert(key, answer);
    answer
}

/// One holder of the function body being emitted: the key it serves, the expression a read
/// takes, the place a refresh assigns, the argument a kept twin is handed, and the path's
/// operand the refresh derives from.
#[derive(Debug, Clone)]
pub struct KeptHolder {
    pub key: super::hoist::PathKey,
    pub param: u16,
    pub off: i64,
    pub read: String,
    pub place: String,
    pub hand: String,
    pub operand: String,
}

impl super::Output<'_> {
    /// [`kept_paths`] for `d_nr` — none where the hoist is off.  A function the live tier
    /// flips into the interpreter is followed by its twin's entry check
    /// ([`Self::kh_live_check`]).
    pub(super) fn kept_of(&mut self, d_nr: u32) -> Option<Rc<Vec<KeptPath>>> {
        if self.hoist_disabled || !self.kept_rewrites_clear(d_nr) {
            return None;
        }
        kept_paths(d_nr, self.data, self.stores, &mut self.kept_cache)
    }

    /// No rewrite that emits a RUN of statements itself runs in `d_nr`'s body: the
    /// hand-over's rotate (an append and the drop beside it), `(R-Apart)`'s construction
    /// runs, a stream walk's consumer or producer, an element-first build (`(R-ElemFirst)`,
    /// whose append moves ahead of the call that fills the element).  Each emits the statements it takes
    /// outside the statement loop where a holder is refreshed, and reads inside the run
    /// would see the holder from before its own append.  A callee declined here is called
    /// plainly, and the holder is derived again after the call.
    fn kept_rewrites_clear(&mut self, d_nr: u32) -> bool {
        if let Some(&known) = self.kept_cache.clear.get(&d_nr) {
            return known;
        }
        let clear = !self.sw.sites.contains_key(&d_nr)
            && !self.sw.producers.contains_key(&d_nr)
            && !self.apart.regions.get(&d_nr).is_some_and(|r| r.profitable)
            && super::apart_vector::roles(self.data, d_nr).is_none_or(|v| v.is_empty())
            && super::apart_records::roles(self.data, &self.apr, d_nr, false).is_empty()
            && (self.element_first_disabled || {
                let ef = super::hoist::element_first(
                    self.data,
                    self.stores,
                    d_nr,
                    self.value_records.views.get(&d_nr),
                );
                ef.pairs.is_empty() && ef.buf_place.is_empty()
            })
            && (d_nr as usize) < self.data.definitions.len()
            && !self.data.def(d_nr).code().any_node(&mut |n| {
                matches!(n, Value::Block(b) if (0..b.operators.len()).any(|i| {
                    super::hand_over::site(&b.operators, i, self.data, self.stores).is_some()
                }))
            });
        self.kept_cache.clear.insert(d_nr, clear);
        clear
    }

    /// `(R-Header)`'s KEPT clause — the statements `from..to` of a block were taken by a
    /// rewrite that emitted them itself (they never reached [`Self::kept_after_statement`]).
    /// None may change a held field: no refresh followed it.  [`Self::kept_rewrites_clear`]
    /// keeps every such rewrite out of a body that holds a field, so reaching this is an
    /// emission fault, and it stops the generation rather than emit a stale read.
    pub(super) fn kept_check_taken(&self, stmts: &[Value], from: usize, to: usize) {
        if self.kept_holders.is_empty() {
            return;
        }
        for op in &stmts[from.min(to)..to] {
            for h in &self.kept_holders {
                assert!(
                    !refreshes(
                        op,
                        self.data,
                        self.stores,
                        self.def_nr,
                        h.param,
                        h.off,
                        &|_| false
                    ),
                    "@FR-R-Header kept clause: a statement that changes {}'s held field was emitted by another rewrite in {}",
                    self.data.def(self.def_nr).variables().name(h.param),
                    self.data.def(self.def_nr).name()
                );
            }
        }
    }

    /// Is the body being emitted the plain function, its ranged variant (`__rg`) or its `__kh`
    /// twin — the forms whose parameters are the declared ones?  Every other twin kind may
    /// take a record parameter as values or drop one, so it holds nothing.
    fn kept_form(&self) -> bool {
        self.twin.is_none()
            && self.refill_twin_buf.is_none()
            && self.append_twin_buf.is_none()
            && self.dest_twin.is_none()
            && !self.vr_twin
            && !self.apart_twin
            && !self.apr_twin
            && !self.nk_twin
            && self.ps_mode.is_none()
            && self.sw_twin.is_none()
            && !self.dv_active
    }

    /// Bind the holders of the body about to be emitted: in the `__kh` twin, the `&mut`
    /// parameters; in the plain body, a local derived here for each kept field the body reads
    /// twice or more.  A frame serves every read.  Answers whether one was pushed.
    pub(super) fn begin_kept_headers(
        &mut self,
        w: &mut dyn std::io::Write,
    ) -> std::io::Result<bool> {
        self.kept_holders.clear();
        self.kh_twin_sites.clear();
        let (paths, twin) = if let Some(t) = self.kh_twin.clone() {
            (t, true)
        } else if self.kept_form() {
            match self.kept_of(self.def_nr) {
                Some(p) => (p, false),
                None => return Ok(false),
            }
        } else {
            return Ok(false);
        };
        let mut frame: HashMap<super::hoist::PathKey, String> = HashMap::new();
        for (k, kp) in paths.iter().enumerate() {
            // The plain body binds a holder it hands down: its own reads alone are what a
            // loop's or the function clause's header already serves, lazily, where the read
            // is — a holder bound at entry is paid on every call, early exits included.
            if !twin
                && (self.kept_handed_down(self.def_nr, kp) == 0
                    || self.kept_effective_uses(self.def_nr, kp, &mut Vec::new()) < 2)
            {
                continue;
            }
            let key: super::hoist::PathKey = (kp.param, vec![kp.off]);
            let operand = self.expr_string(&kp.path)?;
            let (place, hand) = if twin {
                (format!("(*__kh_{k})"), format!("&mut *__kh_{k}"))
            } else {
                self.hoist_counter += 1;
                let name = format!("__kh_{}", self.hoist_counter);
                self.indent(w)?;
                writeln!(
                    w,
                    "let mut {name} = vector::vec_header(&({operand}), &stores.allocations); //@FR-R-Header kept clause"
                )?;
                (name.clone(), format!("&mut {name}"))
            };
            // Under `LOFT_HOIST_VERIFY=1` every read is checked against a fresh derivation.
            let read = if self.hoist_verify {
                format!("vector::kept_header({place}, &({operand}), &stores.allocations)")
            } else {
                place.clone()
            };
            frame.insert(key.clone(), read.clone());
            self.kept_holders.push(KeptHolder {
                key,
                param: kp.param,
                off: kp.off,
                read,
                place,
                hand,
                operand,
            });
        }
        if frame.is_empty() {
            return Ok(false);
        }
        crate::rewrite_census::fired("R-Kept", frame.len());
        self.vec_headers.push(frame);
        self.vec_bases.push(HashMap::new());
        Ok(true)
    }

    /// The live-dispatch entry check of a `__kh` twin: the plain function's, with every
    /// holder the twin was handed derived again before the parked call's answer returns — the
    /// interpreter ran the body and followed no holder.  The check runs before `stores` is
    /// bound, so the refresh reads the cell itself.
    pub(super) fn kh_live_check(&mut self, check: &str) -> std::io::Result<String> {
        use std::fmt::Write as _;
        let Some(kh) = self.kh_twin.clone() else {
            return Ok(check.to_string());
        };
        let mut refresh = String::from(" { let __ks: &Stores = unsafe { &*cell.get() };");
        for (k, kp) in kh.iter().enumerate() {
            let operand = self.expr_string(&kp.path)?;
            let _ = write!(
                refresh,
                " *__kh_{k} = vector::vec_header(&({operand}), &__ks.allocations);"
            );
        }
        refresh.push_str(" }");
        Ok(check
            .replace(" return __lr;", &format!("{refresh} return __lr;"))
            .replace(" return __t;", &format!("{refresh} return __t;")))
    }

    /// The reads a holder of `kp` in `d_nr`'s body would serve, as emitted: its own reads, and
    /// the reads of each callee it is handed to that will take the `__kh` twin — one
    /// [`Self::kept_of`] admits, and not one a loop calls with an `__inv` twin, which the
    /// loop's own holders serve instead.  The analysis count ([`KeptPath::uses`]) counts
    /// every keeping callee, and a holder bound for reads no call makes is a derivation for
    /// nothing (hex_body's `bone_shape_has`: three per call, +7.7 %).  A recursion counts
    /// nothing through itself.
    pub(super) fn kept_effective_uses(
        &mut self,
        d_nr: u32,
        kp: &KeptPath,
        active: &mut Vec<u32>,
    ) -> usize {
        if active.contains(&d_nr) {
            return 0;
        }
        active.push(d_nr);
        let mut n = kp.own;
        for &(g, gp, w, in_loop) in &kp.via {
            if in_loop && self.callee_inputs_of(g).is_some() {
                continue;
            }
            let Some(gk) = self.kept_of(g) else {
                continue;
            };
            if let Some(gkp) = gk.iter().find(|x| x.param == gp && x.off == kp.off) {
                let gkp = gkp.clone();
                n += w * self.kept_effective_uses(g, &gkp, active);
            }
        }
        active.pop();
        n
    }

    /// The reads of `kp` a plain body's holder would serve through the callees it hands it to
    /// ([`Self::kept_effective_uses`] without the body's own reads).
    fn kept_handed_down(&mut self, d_nr: u32, kp: &KeptPath) -> usize {
        let own = kp.own;
        self.kept_effective_uses(d_nr, kp, &mut Vec::new()) - own
    }

    /// Is the header an enclosing frame holds for `path` this body's kept holder — no loop's
    /// and no view's?  A `for` walk's hidden vector served from the holder counts too.
    pub(super) fn kept_holds_only(&self, path: &super::hoist::PathKey) -> bool {
        self.active_vec_header(path)
            .is_some_and(|held| self.kept_holders.iter().any(|h| h.read == held))
    }

    /// The holder a loop over `lp` may copy into a local for its extent: the one serving
    /// `path`, when nothing in the loop can change its field — no append to it and no call
    /// that may (a kept twin's call included, which changes it through the holder).
    pub(super) fn kept_loop_copy(
        &self,
        lp: &Block,
        path: &super::hoist::PathKey,
    ) -> Option<String> {
        let held = self.active_vec_header(path)?;
        let h = self.kept_holders.iter().find(|h| h.read == held)?;
        let changes = lp.operators.iter().any(|op| {
            op.any_node(&mut |n| {
                matches!(n, Value::Set(..) | Value::Call(..))
                    && refreshes(
                        n,
                        self.data,
                        self.stores,
                        self.def_nr,
                        h.param,
                        h.off,
                        &|_| false,
                    )
            })
        });
        (!changes).then(|| h.read.clone())
    }

    pub(super) fn end_kept_headers(&mut self, opened: bool) {
        if opened {
            self.vec_headers.pop();
            self.vec_bases.pop();
        }
        self.kept_holders.clear();
    }

    /// After statement `at` of a block: derive each holder again that the statement may have
    /// changed (`(R-Refresh)`).
    pub(super) fn kept_after_statement(
        &mut self,
        w: &mut dyn std::io::Write,
        stmts: &[Value],
        at: usize,
    ) -> std::io::Result<()> {
        if self.kept_holders.is_empty() {
            return Ok(());
        }
        let op = &stmts[at];
        let holders = self.kept_holders.clone();
        for h in &holders {
            let sites = &self.kh_twin_sites;
            if refreshes(
                op,
                self.data,
                self.stores,
                self.def_nr,
                h.param,
                h.off,
                &|args| sites.contains(&(args.as_ptr() as usize)),
            ) {
                self.indent(w)?;
                writeln!(
                    w,
                    "{} = vector::vec_header(&({}), &stores.allocations); //@FR-R-Header kept clause: refreshed",
                    h.place, h.operand
                )?;
            }
        }
        Ok(())
    }

    /// After statement `at`, and after a view header it may have bound
    /// (`Output::bind_view_header`): where the statement bound a local to a kept path that no
    /// frame holds yet — a `for` walk's hidden vector — the local's reads serve from the
    /// holder itself, which stays current through every refresh.  Answers the frames pushed,
    /// which the block pops at its end.
    pub(super) fn kept_alias(&mut self, stmts: &[Value], at: usize) -> usize {
        if let Value::Set(d, rhs) = stmts[at].unspan()
            && let Some(path) = vector_path(self.data, rhs)
            && let Some(read) = self
                .kept_holders
                .iter()
                .find(|h| h.key == path)
                .map(|h| h.read.clone())
            && !self
                .vec_headers
                .iter()
                .any(|f| f.contains_key(&(*d, Vec::new())))
        {
            self.vec_headers
                .push(HashMap::from([((*d, Vec::new()), read)]));
            self.vec_bases.push(HashMap::new());
            return 1;
        }
        0
    }

    /// The holders a call to `d_nr` hands its `__kh` twin, when this body holds every field
    /// the callee keeps at the argument it is given, each a different holder; `None` keeps
    /// the plain call.
    pub(super) fn kept_twin_call(&mut self, d_nr: u32, vals: &[Value]) -> Option<Vec<String>> {
        if self.kept_holders.is_empty() {
            return None;
        }
        let gk = self.kept_of(d_nr)?;
        let mut out: Vec<String> = Vec::with_capacity(gk.len());
        let mut used: Vec<usize> = Vec::new();
        for k in gk.iter() {
            let Value::Var(x) = vals.get(usize::from(k.param))?.unspan() else {
                return None;
            };
            let at = self
                .kept_holders
                .iter()
                .position(|h| h.key == (*x, vec![k.off]))?;
            if used.contains(&at) {
                return None;
            }
            used.push(at);
            out.push(self.kept_holders[at].hand.clone());
        }
        self.kh_twin_sites.insert(vals.as_ptr() as usize);
        crate::rewrite_census::fired("R-KeptTwin", 1);
        Some(out)
    }
}
