// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! `@FR-R-Apart`, instance 3 — a `vector<float>` local that never leaves its function is held
//! as an INLINE buffer with a spill (`codegen_runtime::ApVec`): its pushes, element reads and
//! writes and its length touch no store.  Nothing crosses a boundary, so there is no
//! conversion and no twin, and the rewrite is local to one body — which a build that ships
//! the live tier may make too, since a flipped function runs its interpreted body whole.
//!
//! The gate answers per body which vector variables are APART and which variables only serve
//! them (COMPANIONS: the store a plain local is minted into, the save of a work buffer's
//! incoming value).  An apart vector is a local, or a hidden WORK-BUFFER parameter
//! (`@FR-R-WorkBuffer`) — whose content on entry the body never reads, because its own
//! declaration empties it.  Any other parameter carries the caller's elements and is never
//! admitted.  Every mention of an apart vector is one of:
//!
//! - its EMPTY INIT: a work buffer's `if OpRefIsNull(v) { <mint> } else { t_6vector_clear(v) }`,
//!   a local's `v = OpGetField(<companion>, …)`, or a `t_6vector_clear(v)`;
//! - `OpPutRef(<companion>, v)`, the save of a work buffer's incoming value;
//! - `OpPreAllocVector(v, …)`, `OpPushFloat(v, x)`;
//! - an element read `OpGetFloat(OpGetVector[Nullable](v, 8, i), 0)`, an element write
//!   `OpSetFloat(OpGetVector(v, 8, i), 0, x)`, a length (`OpLengthVector(v)`, `len(v)`);
//! - a free: `OpFreeRef(v)`, `OpFreeRefIfDistinct` against a companion.
//!
//! A companion's mentions are its null init, its mint (`OpDatabase`), its field emptied
//! (`OpSetInt4(c, 0, 0)`), its frees, and the ones above.  Any other mention keeps the vector
//! and its companions in the store, which costs the rewrite and never a value; an apart
//! vector is no `DbRef`, so a mention the emitter misses is a `rustc` type error.
use crate::data::{Data, Type, Value};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};

/// The elements an apart vector holds before it spills to the heap: the price measured a
/// 64-element stack buffer at −55 % on `rig_world_frame3`, where a bare heap vector was slower
/// than the store's pooled buffers.
pub const INLINE: usize = 64;

/// The apart vectors of one body and the companions that serve them.
#[derive(Clone, Debug, Default)]
pub struct VecRoles {
    pub apart: HashSet<u16>,
    pub companions: HashSet<u16>,
}

impl VecRoles {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.apart.is_empty()
    }

    /// Whether `v` is an apart vector or one of their companions: a root no store path of
    /// the body may plan anything on.
    #[must_use]
    pub fn holds(&self, v: u16) -> bool {
        self.apart.contains(&v) || self.companions.contains(&v)
    }
}

/// `LOFT_NO_APART_VECTOR=1` — every `vector<float>` local lives in the store (`@FR-R-Apart`'s
/// instance 3 off, instance 2 untouched): the finer bisect step under `LOFT_NO_APART=1`.
#[must_use]
pub fn disabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| {
        super::apart_value::apart_disabled()
            || std::env::var("LOFT_NO_APART_VECTOR").is_ok_and(|v| v != "0")
    })
}

fn op_name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

fn var_of(x: &Value) -> Option<u16> {
    match x.unspan() {
        Value::Var(v) => Some(*v),
        _ => None,
    }
}

/// An element address `OpGetVector[Nullable](v, 8, i)` of a plain vector variable →
/// `(v, i, nullable)`.
fn element_of<'a>(data: &Data, x: &'a Value) -> Option<(u16, &'a Value, bool)> {
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
        && let Some(v) = var_of(vec)
    {
        return Some((v, idx, nullable));
    }
    None
}

/// An element READ `OpGetFloat(<element of v>, 0)` → `(v, index, nullable)`.
#[must_use]
pub fn read<'a>(data: &Data, x: &'a Value) -> Option<(u16, &'a Value, bool)> {
    if let Value::Call(d, args) = x.unspan()
        && op_name(data, *d) == "OpGetFloat"
        && let [elem, fld] = args.as_slice()
        && matches!(fld.unspan(), Value::Int(0))
    {
        return element_of(data, elem);
    }
    None
}

/// An element WRITE `OpSetFloat(<element of v>, 0, val)` → `(v, index, val)`.
#[must_use]
pub fn write<'a>(data: &Data, x: &'a Value) -> Option<(u16, &'a Value, &'a Value)> {
    if let Value::Call(d, args) = x.unspan()
        && op_name(data, *d) == "OpSetFloat"
        && let [elem, fld, val] = args.as_slice()
        && matches!(fld.unspan(), Value::Int(0))
        && let Some((v, idx, false)) = element_of(data, elem)
    {
        return Some((v, idx, val));
    }
    None
}

/// The one-operand vector ops: `(op, v)` for `OpLengthVector(v)`, `t_6vector_len(v)`,
/// `t_6vector_clear(v)`, `OpFreeRef(v)`.
#[must_use]
pub fn unary(data: &Data, x: &Value) -> Option<(&'static str, u16)> {
    if let Value::Call(d, args) = x.unspan()
        && let [a] = args.as_slice()
        && let Some(v) = var_of(a)
    {
        return match op_name(data, *d) {
            "OpLengthVector" | "t_6vector_len" => Some(("len", v)),
            "t_6vector_clear" => Some(("clear", v)),
            "OpFreeRef" => Some(("free", v)),
            _ => None,
        };
    }
    None
}

/// A push `OpPushFloat(v, val)` → `(v, val)`.
#[must_use]
pub fn push<'a>(data: &Data, x: &'a Value) -> Option<(u16, &'a Value)> {
    if let Value::Call(d, args) = x.unspan()
        && op_name(data, *d) == "OpPushFloat"
        && let [vec, val] = args.as_slice()
        && let Some(v) = var_of(vec)
    {
        return Some((v, val));
    }
    None
}

/// A work buffer's empty init: `if OpRefIsNull(v) { … } else { t_6vector_clear(v) }`, whose
/// first branch mints a companion and binds `v` to its field → `v`, with the variables the
/// first branch names besides `v`.
#[must_use]
pub fn buffer_init(data: &Data, x: &Value) -> Option<(u16, Vec<u16>)> {
    let Value::If(test, mint, clear) = x.unspan() else {
        return None;
    };
    let Value::Call(test_op, targs) = test.unspan() else {
        return None;
    };
    let [tv] = targs.as_slice() else {
        return None;
    };
    let vec = var_of(tv)?;
    if op_name(data, *test_op) != "OpRefIsNull" {
        return None;
    }
    let clear_ok = |stmt: &Value| matches!(unary(data, stmt), Some(("clear", u)) if u == vec);
    let clears = match clear.unspan() {
        Value::Insert(items) => items.len() == 1 && clear_ok(&items[0]),
        Value::Block(bl) => bl.operators.len() == 1 && clear_ok(&bl.operators[0]),
        other => clear_ok(other),
    };
    if !clears {
        return None;
    }
    let items: &[Value] = match mint.unspan() {
        Value::Insert(items) => items,
        Value::Block(bl) => &bl.operators,
        _ => return None,
    };
    let mut others = Vec::new();
    for item in items {
        match item.unspan() {
            Value::Call(op, args)
                if matches!(
                    op_name(data, *op),
                    "OpFreeRefIfDistinct" | "OpDatabase" | "OpSetInt4"
                ) =>
            {
                for a in args {
                    if let Some(u) = var_of(a)
                        && u != vec
                    {
                        others.push(u);
                    }
                }
            }
            Value::Set(u, rhs) if *u == vec => {
                let Value::Call(get, args) = rhs.unspan() else {
                    return None;
                };
                if op_name(data, *get) != "OpGetField" {
                    return None;
                }
                others.extend(args.first().and_then(var_of));
            }
            _ => return None,
        }
    }
    Some((vec, others))
}

fn is_float_vector(tp: &Type) -> bool {
    !tp.peel_optional().1 && matches!(tp.base(), Type::Vector(e, _) if matches!(**e, Type::Float))
}

struct Walk<'a> {
    data: &'a Data,
    /// The candidate vectors.
    cand: HashSet<u16>,
    /// Candidate → the variables admitted as its companions.
    comp: HashMap<u16, HashSet<u16>>,
    /// Vectors a mention declined.
    out: HashSet<u16>,
    /// Every variable some statement mentions outside an admitted shape.
    loose: HashSet<u16>,
    /// The vectors with a push or a read: worth holding.
    used: HashSet<u16>,
}

impl Walk<'_> {
    fn tie(&mut self, v: u16, c: u16) {
        if self.cand.contains(&v) {
            self.comp.entry(v).or_default().insert(c);
        }
    }

    fn walk(&mut self, x: &Value) {
        let data = self.data;
        if let Some((v, idx, _)) = read(data, x)
            && self.cand.contains(&v)
        {
            self.used.insert(v);
            self.walk(idx);
            return;
        }
        if let Some((v, idx, val)) = write(data, x)
            && self.cand.contains(&v)
        {
            self.walk(idx);
            self.walk(val);
            return;
        }
        if let Some((v, val)) = push(data, x)
            && self.cand.contains(&v)
        {
            self.used.insert(v);
            self.walk(val);
            return;
        }
        if let Some((_, v)) = unary(data, x)
            && self.cand.contains(&v)
        {
            return;
        }
        if let Some((v, others)) = buffer_init(data, x)
            && self.cand.contains(&v)
        {
            for c in others {
                self.tie(v, c);
            }
            return;
        }
        match x.unspan() {
            Value::Var(v) => {
                self.loose.insert(*v);
            }
            Value::Set(v, rhs) if self.cand.contains(v) => match rhs.unspan() {
                Value::Null => {}
                Value::Call(d, args) if op_name(data, *d) == "OpGetField" => {
                    match args.first().and_then(var_of) {
                        Some(c) => self.tie(*v, c),
                        None => {
                            self.out.insert(*v);
                        }
                    }
                }
                _ => {
                    self.out.insert(*v);
                    self.walk(rhs);
                }
            },
            Value::Call(d, args)
                if matches!(
                    op_name(data, *d),
                    "OpPutRef" | "OpFreeRefIfDistinct" | "OpPreAllocVector"
                ) && args
                    .iter()
                    .any(|a| var_of(a).is_some_and(|u| self.cand.contains(&u))) =>
            {
                let vars: Vec<u16> = args.iter().filter_map(var_of).collect();
                let held: Vec<u16> = vars
                    .iter()
                    .copied()
                    .filter(|u| self.cand.contains(u))
                    .collect();
                for v in held {
                    for &c in vars.iter().filter(|u| **u != v) {
                        self.tie(v, c);
                    }
                }
                for a in args.iter().filter(|a| var_of(a).is_none()) {
                    self.walk(a);
                }
            }
            Value::Iter(v, ..)
            | Value::CallRef(v, _)
            | Value::FnRef(_, v, _)
            | Value::FnRefDnr(v) => {
                self.loose.insert(*v);
                x.for_each_child(&mut |c| self.walk(c));
            }
            Value::Set(v, rhs) => {
                // A bind of another variable: the vectors its value mentions are loose.
                let _ = v;
                self.walk(rhs);
            }
            _ => x.for_each_child(&mut |c| self.walk(c)),
        }
    }
}

/// The companions' own statements a body may hold: null init, mint, field emptied, free.
fn companion_statement(data: &Data, x: &Value, c: u16) -> bool {
    match x.unspan() {
        Value::Set(u, rhs) if *u == c => matches!(rhs.unspan(), Value::Null),
        Value::Call(d, args) => {
            let name = op_name(data, *d);
            matches!(name, "OpDatabase" | "OpSetInt4" | "OpFreeRef")
                && args.first().and_then(var_of) == Some(c)
                && args.iter().skip(1).all(|a| var_of(a).is_none())
        }
        _ => false,
    }
}

/// Every mention of `c` in `x` outside its own statements and the vector shapes the walk
/// admitted: the walk saw those shapes; this asks whether anything else names `c`.
fn companion_loose(data: &Data, x: &Value, c: u16, held: &HashSet<u16>) -> bool {
    if companion_statement(data, x, c) {
        return false;
    }
    if let Some((v, others)) = buffer_init(data, x)
        && held.contains(&v)
        && others.contains(&c)
    {
        return false;
    }
    match x.unspan() {
        Value::Var(u) => *u == c,
        Value::Set(u, rhs) if held.contains(u) => {
            if let Value::Call(d, args) = rhs.unspan()
                && op_name(data, *d) == "OpGetField"
                && args.first().and_then(var_of) == Some(c)
            {
                return false;
            }
            companion_loose(data, rhs, c, held)
        }
        Value::Set(u, rhs) => *u == c || companion_loose(data, rhs, c, held),
        Value::Call(d, args)
            if matches!(op_name(data, *d), "OpPutRef" | "OpFreeRefIfDistinct")
                && args
                    .iter()
                    .any(|a| var_of(a).is_some_and(|u| held.contains(&u))) =>
        {
            args.iter()
                .any(|a| var_of(a).is_none() && companion_loose(data, a, c, held))
        }
        _ => {
            let mut found = false;
            x.for_each_child(&mut |ch| {
                if !found && companion_loose(data, ch, c, held) {
                    found = true;
                }
            });
            found
        }
    }
}

/// The variables of `def` that may be held inline: `vector<float>` locals that no closure
/// captures, and the work-buffer parameters (any other parameter carries the caller's elements).
fn candidates(def: &crate::data::Definition) -> HashSet<u16> {
    let vars = def.variables();
    let mut cand: HashSet<u16> = HashSet::default();
    for v in 0..vars.count() {
        if !is_float_vector(vars.tp(v)) || vars.is_captured(v) {
            continue;
        }
        if vars.is_argument(v) {
            let name = vars.name(v);
            let work = def
                .attributes()
                .iter()
                .any(|a| a.name == name && a.work_buffer);
            if !work {
                continue;
            }
        }
        cand.insert(v);
    }
    cand
}

/// The apart vectors of `def_nr`'s body, or `None` when it holds none.
#[must_use]
pub fn roles(data: &Data, def_nr: u32) -> Option<VecRoles> {
    if disabled() {
        return None;
    }
    let def = data.def(def_nr);
    if matches!(def.returned().base(), Type::Iterator(..)) || *def.code() == Value::Null {
        return None;
    }
    let vars = def.variables();
    let cand = candidates(def);
    if cand.is_empty() {
        return None;
    }
    let mut yields = false;
    def.code().any_node(&mut |n| {
        if matches!(n, Value::Yield(_) | Value::Parallel(_)) {
            yields = true;
        }
        yields
    });
    if yields {
        return None;
    }
    let mut w = Walk {
        data,
        cand: cand.clone(),
        comp: HashMap::default(),
        out: HashSet::default(),
        loose: HashSet::default(),
        used: HashSet::default(),
    };
    w.walk(def.code());
    let trace = std::env::var("LOFT_TRACE_APART").is_ok_and(|v| v != "0");
    let mut held: HashSet<u16> = HashSet::default();
    for &v in &cand {
        let why = if w.out.contains(&v) {
            Some("a bind of a value that is no empty init")
        } else if w.loose.contains(&v) {
            Some("a mention the inline buffer does not serve")
        } else if !w.used.contains(&v) {
            Some("no push and no read")
        } else {
            None
        };
        match why {
            None => {
                held.insert(v);
            }
            Some(why) if trace => {
                eprintln!(
                    "[apart] {} keeps vector {} in the store: {why}",
                    def.name(),
                    vars.name(v)
                );
            }
            Some(_) => {}
        }
    }
    // A companion serves one vector, is no apart vector itself, and has no other mention; a
    // vector whose companion fails any of these stays in the store.  Narrowed to a fixpoint,
    // since a vector leaving changes which shapes count as admitted.
    loop {
        let mut drop: Vec<u16> = Vec::new();
        let mut owner: HashMap<u16, u16> = HashMap::default();
        for &v in &held {
            for &c in w.comp.get(&v).into_iter().flatten() {
                let shared = owner.insert(c, v).is_some_and(|o| o != v);
                if shared
                    || held.contains(&c)
                    || vars.is_argument(c)
                    || companion_loose(data, def.code(), c, &held)
                {
                    if trace {
                        eprintln!(
                            "[apart] {} keeps vector {} in the store: its companion {} has another use",
                            def.name(),
                            vars.name(v),
                            vars.name(c)
                        );
                    }
                    drop.push(v);
                }
            }
        }
        if drop.is_empty() {
            break;
        }
        for v in drop {
            held.remove(&v);
        }
    }
    if held.is_empty() {
        return None;
    }
    let companions = held
        .iter()
        .flat_map(|v| w.comp.get(v).into_iter().flatten().copied())
        .collect();
    Some(VecRoles {
        apart: held,
        companions,
    })
}
