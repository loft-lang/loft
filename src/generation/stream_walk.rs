// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! `@FR-R-StreamWalk` — a vector a callee BUILDS and its caller WALKS ONCE is never built: the
//! callee's STREAM TWIN runs its own body and hands each element it would have appended to the
//! caller's loop body, which runs as a closure at that append.
//!
//! ```text
//! runs = slice_runs(d, a, b);                 slice_runs__sw0(cell, d, a, b, &mut |cell, s, st| {
//! for i in 0..len(runs) {              ⟶        … s … st …
//!   r = runs[i]?;  … r.str … r.style …        });
//! }
//! ```
//!
//! The plan is decided here from the IR; `stream_walk_emit` writes it.
//!
//! - A PRODUCER answers `vector<T>` for a plain record `T` through its hidden result buffer,
//!   and the buffer and the vector local it names are mentioned only by the buffer's own
//!   plumbing (its alias, its clear and mint, the vector field), by the appends — a mint, a
//!   set of each field, a finish — and by a `return` of the buffer.  Each element's fields are
//!   set from values in the mint's block, so the twin evaluates exactly those values where
//!   the build would have stored them.  It is QUIET: it writes only stores and texts it made,
//!   or the buffers it is handed to write (see [`Quiet`]).
//! - A CONSUMER binds the producer's call to a local and walks it with
//!   `for i in 0..len(runs) { … }`, where every mention of the vector and the index is an
//!   element read `runs[i]?` — bound to an element local, or read in place — and every
//!   mention of an element is a read of one of its fields.  The walk's body is quiet in the
//!   same sense, every fn-ref it calls dispatches only to quiet functions without captures,
//!   and nothing in it leaves the walk (`break`, `continue`, `return`, `yield`).
//!
//! Why the order may change.  The unrewritten program runs the producer to its end and then
//! the body once per element; the twin interleaves them.  A quiet producer reports nothing and
//! writes only what it made, so the body cannot observe how far it has run; a quiet body
//! writes only the consumer's own stores — never a store the producer reads, which it reaches
//! only through its parameters — so the producer reads the same values at every step.  The
//! values handed over are the values the build would have stored, and every field kind the
//! rule admits reads back exactly what was stored, so the body reads the same values too.
//!
//! The twin is emitted without loop hoists: a header or base it held across its loop would
//! not survive a body that grows a store.  `LOFT_NO_STREAM_WALK=1` builds every vector;
//! `LOFT_TRACE_STREAM_WALK=1` names every producer and site admitted or declined, with why.
use crate::data::{Data, DefType, Type, Value};
use crate::fxhash::{FxHashMap as HashMap, FxHashSet as HashSet};

/// `LOFT_NO_STREAM_WALK=1` — every vector is built and walked (`@FR-R-StreamWalk` off): the
/// before-half of every A/B, and the first bisect step for a wrong value out of such a walk.
#[must_use]
pub fn disabled() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_NO_STREAM_WALK").is_ok_and(|v| v != "0"))
}

fn trace_on() -> bool {
    static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *F.get_or_init(|| std::env::var("LOFT_TRACE_STREAM_WALK").is_ok_and(|v| v != "0"))
}

fn trace(msg: impl FnOnce() -> String) {
    if trace_on() {
        eprintln!("[stream-walk] {}", msg());
    }
}

pub(crate) fn addr(v: &Value) -> usize {
    std::ptr::from_ref(v) as usize
}

fn op_name(data: &Data, d: u32) -> &str {
    if (d as usize) < data.definitions.len() {
        data.def(d).name()
    } else {
        ""
    }
}

fn var_of(v: &Value) -> Option<u16> {
    match v.unspan() {
        Value::Var(x) => Some(*x),
        _ => None,
    }
}

fn int_of(v: &Value) -> Option<i64> {
    match v.unspan() {
        Value::Int(i) => Some(i64::from(*i)),
        Value::Long(l) => Some(*l),
        _ => None,
    }
}

/// A field kind the walk hands over: one whose store round trip answers exactly the value
/// stored, so the value itself can stand in for the read.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Kind {
    Int,
    Float,
    Single,
    Text,
}

impl Kind {
    fn of_get(name: &str) -> Option<Kind> {
        Some(match name {
            "OpGetInt" => Kind::Int,
            "OpGetFloat" => Kind::Float,
            "OpGetSingle" => Kind::Single,
            "OpGetText" => Kind::Text,
            _ => return None,
        })
    }

    fn of_set(name: &str) -> Option<Kind> {
        Some(match name {
            "OpSetInt" => Kind::Int,
            "OpSetFloat" => Kind::Float,
            "OpSetSingle" => Kind::Single,
            "OpSetText" => Kind::Text,
            _ => return None,
        })
    }

    /// The closure parameter's Rust type.
    #[must_use]
    pub fn rust(self) -> &'static str {
        match self {
            Kind::Int => "i64",
            Kind::Float => "f64",
            Kind::Single => "f32",
            Kind::Text => "&str",
        }
    }
}

/// One append of a producer: the field sets after its mint, and its finish.
#[derive(Clone, Debug, Default)]
pub struct Group {
    /// `(offset, kind, value)` per field set, in statement order; the value is the set's
    /// third operand, by address.
    pub sets: Vec<(i64, Kind, usize)>,
    /// `(statement, text)` per release of a text a field value made: emitted after the hand-over
    /// when that value was evaluated.
    pub frees: Vec<(usize, u16)>,
}

/// A producer: what its stream twins drop and where they hand an element over.
#[derive(Clone, Debug, Default)]
pub struct Producer {
    /// The hidden result buffer's parameter position.
    pub buf_attr: usize,
    /// The buffer, its aliases, the vector local and the element locals.
    pub plumbing: HashSet<u16>,
    /// The element locals.
    pub elems: HashSet<u16>,
    /// Statements every twin drops: the buffer's plumbing, the reservations, the element
    /// locals' null inits, the field sets and the finishes.
    pub drop: HashSet<usize>,
    /// The mint statement of each append → its group.
    pub groups: HashMap<usize, Group>,
    /// The fields every group sets, with their kinds.
    pub fields: Vec<(i64, Kind)>,
}

/// A walk the plan streams.
#[derive(Clone, Debug)]
pub struct Site {
    pub producer: u32,
    /// The vector local.
    pub runs: u16,
    /// The `runs = producer(…)` statement.
    pub call: usize,
    /// The `for` block.
    pub walk: usize,
    /// The element local, when the body binds one, and its binding statement.
    pub elem: Option<(u16, usize)>,
    /// The loop index.
    pub index: u16,
    /// The twin this walk calls.
    pub twin: usize,
}

/// The program's stream plan.
#[derive(Clone, Debug, Default)]
pub struct SwPlan {
    pub producers: HashMap<u32, Producer>,
    /// Consumer → its sites.
    pub sites: HashMap<u32, Vec<Site>>,
    /// The twins to emit: the producer and the fields handed over, in parameter order.
    pub twins: Vec<(u32, Vec<(i64, Kind)>)>,
    /// Statement address → (consumer, site index), for the call and the walk statements.
    pub at: HashMap<usize, (u32, usize)>,
}

impl SwPlan {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.sites.is_empty()
    }
}

// ---- quiet functions ----

/// Operators that write a text or a store through one operand, beyond `pure_reuse`'s list.
fn text_write_target(name: &str) -> Option<usize> {
    match name {
        "OpAppendText"
        | "OpAppendStackText"
        | "OpAppendCharacter"
        | "OpAppendStackCharacter"
        | "OpAppendTextBytes"
        | "OpClearText"
        | "OpClearStackText"
        | "OpClearVector"
        | "OpSetTextReplace"
        | "OpFormatText"
        | "OpFormatStackText"
        | "OpFormatInt"
        | "OpFormatStackInt"
        | "OpFormatFloat"
        | "OpFormatStackFloat"
        | "OpFormatSingle"
        | "OpFormatStackSingle"
        | "OpFormatDatabase"
        | "OpFormatStackDatabase"
        | "OpPutRef"
        | "OpPutText"
        | "OpPutInt"
        | "OpPutFloat"
        | "OpPutSingle"
        | "OpPutBool"
        | "OpPutCharacter"
        | "OpPutEnum"
        | "OpPutNarrow" => Some(0),
        _ => crate::pure_reuse::write_target(name),
    }
}

/// Operators that release what their first operand names.
fn is_free(name: &str) -> bool {
    matches!(
        name,
        "OpFreeRef" | "OpFreeText" | "OpFreeRefIfDistinct" | "OpFreeStack" | "OpFreeScratch"
    )
}

/// The variable a written place is rooted at: a variable, a field or element of one, the last
/// value of a block, or the text a `OpCreateStack` hands on.
fn root(v: &Value, data: &Data) -> Option<u16> {
    match v.unspan() {
        Value::Var(x) => Some(*x),
        Value::Call(d, args) if !args.is_empty() => {
            let name = op_name(data, *d);
            if name.starts_with("OpGet") || name == "OpCreateStack" {
                root(&args[0], data)
            } else {
                None
            }
        }
        Value::Block(bl) => bl.operators.last().and_then(|l| root(l, data)),
        _ => None,
    }
}

/// The user functions this rule reasons about: loft bodies, not natives.
fn candidates(data: &Data) -> Vec<u32> {
    (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            matches!(def.def_type, DefType::Function)
                && def.name().starts_with("n_")
                && !matches!(def.code(), Value::Null)
        })
        .collect()
}

/// The parameter positions a callee may write through: its hidden result buffer, every
/// other hidden buffer the compiler hands it (a work buffer), and every by-reference
/// (`RefVar`) parameter — a text work buffer, a `&text`.
fn written_params(data: &Data, f: u32) -> Vec<usize> {
    let def = data.def(f);
    let ret = def.hidden_return_buffer_attr();
    def.attributes()
        .iter()
        .enumerate()
        .filter(|(i, a)| Some(*i) == ret || a.hidden || matches!(a.typedef.base(), Type::RefVar(_)))
        .map(|(i, _)| i)
        .collect()
}

/// The QUIET functions and the reasoning shared with a walk's body.
///
/// A fragment of `f`'s body is quiet when every write it makes lands in a place OWNED by `f`:
/// a local whose type borrows only owned places, or a parameter `f` is handed to write (its
/// hidden result buffer, a by-reference parameter).  A call writes the places its callee
/// writes through ([`written_params`]), so a call of a quiet function is quiet when those
/// arguments are owned; a fn-ref call is quiet when every function it can dispatch to is quiet
/// and captures nothing.  Everything else — a native outside `pure_reuse`'s read list, an
/// operator neither list knows, a `yield` — declines, which costs the rewrite, never a value.
/// A quiet function's only effect is its result and the buffers it was handed: it reports
/// nothing, prints nothing and writes no store its caller can reach any other way.
pub struct Quiet<'a> {
    data: &'a Data,
    reachable: &'a std::collections::HashSet<u32>,
    pub fns: HashSet<u32>,
}

impl<'a> Quiet<'a> {
    #[must_use]
    pub fn new(data: &'a Data, reachable: &'a std::collections::HashSet<u32>) -> Self {
        let cands = candidates(data);
        let mut q = Quiet {
            data,
            reachable,
            fns: cands.iter().copied().collect(),
        };
        loop {
            let drop: Vec<u32> = cands
                .iter()
                .copied()
                .filter(|f| {
                    q.fns.contains(f) && {
                        let own = q.owned(*f, &HashSet::default());
                        q.fragment(*f, data.def(*f).code(), &own).is_err()
                    }
                })
                .collect();
            if drop.is_empty() {
                return q;
            }
            for f in drop {
                q.fns.remove(&f);
            }
        }
    }

    /// The places `f` owns: its locals whose types borrow only owned places, and the
    /// parameters it is handed to write.  `not` names locals that never count as owned.
    #[must_use]
    pub fn owned(&self, f: u32, not: &HashSet<u16>) -> HashSet<u16> {
        let def = self.data.def(f);
        let vars = def.variables();
        let mut own: HashSet<u16> = written_params(self.data, f)
            .into_iter()
            .map(|i| vars.var(&def.attributes()[i].name))
            .filter(|&v| v != u16::MAX)
            .collect();
        let locals: Vec<u16> = (0..vars.count())
            .filter(|&v| !vars.is_argument(v) && !not.contains(&v))
            .collect();
        // Grow to the fixed point: a local is owned once everything it borrows is.
        loop {
            let before = own.len();
            for &v in &locals {
                if !own.contains(&v) && vars.tp(v).depend().iter().all(|d| own.contains(d)) {
                    own.insert(v);
                }
            }
            if own.len() == before {
                return own;
            }
        }
    }

    /// Is every write in `code` (a fragment of `f`'s body) to a place in `own`?
    ///
    /// # Errors
    ///
    /// Why it is not, as `LOFT_TRACE_STREAM_WALK=1` prints it.
    pub fn fragment(&self, f: u32, code: &Value, own: &HashSet<u16>) -> Result<(), String> {
        let data = self.data;
        let vars = data.def(f).variables();
        let mut why: Option<String> = None;
        let owned = |a: &Value| root(a, data).is_some_and(|r| own.contains(&r));
        code.any_node(&mut |n| {
            match n {
                Value::Set(v, _) if vars.is_argument(*v) && !own.contains(v) => {
                    if matches!(vars.tp(*v).base(), Type::RefVar(_)) {
                        why = Some(format!("writes parameter {}", vars.name(*v)));
                    }
                }
                Value::Call(d, args) => {
                    if (*d as usize) >= data.definitions.len() {
                        why = Some("an unknown call".into());
                        return true;
                    }
                    let callee = data.def(*d);
                    let name = callee.name();
                    if name.starts_with("n_") && !matches!(callee.code(), Value::Null) {
                        if !self.fns.contains(d) {
                            why = Some(format!("calls {name}, which is not quiet"));
                        } else if let Some(i) = written_params(data, *d)
                            .into_iter()
                            .find(|&i| args.get(i).is_some_and(|a| !owned(a)))
                        {
                            why = Some(format!("hands {name} a buffer it does not own ({i})"));
                        }
                    } else if crate::pure_reuse::reads_only(name) {
                    } else if name == "OpFreeText" {
                        // Releases the local text itself — never the store a borrowed text
                        // reads from.
                        if args
                            .first()
                            .and_then(var_of)
                            .is_none_or(|t| vars.is_argument(t))
                        {
                            why = Some("OpFreeText of a parameter".into());
                        }
                    } else if is_free(name) {
                        if args.first().is_some_and(|a| !owned(a)) {
                            why = Some(format!("{name} of a place it does not own"));
                        }
                    } else if let Some(t) = text_write_target(name) {
                        if args.get(t).is_none_or(|a| !owned(a)) {
                            why = Some(format!("{name} writes a place it does not own"));
                        }
                    } else if name == "OpCreateStack" || name == "OpConvTextFromNull" {
                    } else {
                        why = Some(format!("calls {name}"));
                    }
                }
                Value::CallRef(v, args) => {
                    let arms =
                        crate::fnref::dispatch_arms(data, self.reachable, vars.tp(*v), args.len());
                    match arms {
                        Some(arms) if !arms.is_empty() => {
                            if let Some(a) = arms
                                .iter()
                                .find(|a| a.has_closure || !self.fns.contains(&a.d_nr))
                            {
                                why = Some(format!("a fn-ref may call {}", a.name));
                            } else if args
                                .iter()
                                .any(|a| matches!(a.unspan(), Value::Block(_)) && !owned(a))
                            {
                                why = Some("a fn-ref call writes a buffer it does not own".into());
                            }
                        }
                        _ => why = Some("a fn-ref with no known target".into()),
                    }
                }
                Value::Yield(_) => why = Some("yields".into()),
                _ => {}
            }
            why.is_some()
        });
        why.map_or(Ok(()), Err)
    }
}

// ---- producers ----

/// Does `def` answer `vector<T>` for a record `T`?
fn vector_of_records(data: &Data, func: u32) -> bool {
    matches!(data.def(func).returned().base(),
        Type::Vector(elm, _) if matches!(elm.base(), Type::Reference(_, _)))
}

/// The producer plan of `func`, or why it declines.
fn producer(data: &Data, quiet: &Quiet, func: u32) -> Result<Producer, String> {
    if !vector_of_records(data, func) {
        return Err("does not answer a vector of records".into());
    }
    let def = data.def(func);
    let Some(buf_attr) = def.hidden_return_buffer_attr() else {
        return Err("no result buffer".into());
    };
    let buf = def.variables().var(&def.attributes()[buf_attr].name);
    let code = def.code();
    let (plumbing, elems, out) = plumbing(data, code, buf)?;
    let mut prod = Producer {
        buf_attr,
        plumbing,
        elems,
        ..Producer::default()
    };
    classify(data, code, buf, out, &mut prod)?;
    if prod.groups.is_empty() {
        return Err("appends nothing".into());
    }
    prod.fields = common_fields(&prod);
    // Quiet, with the plumbing its own.
    let mut own = quiet.owned(func, &HashSet::default());
    own.extend(prod.plumbing.iter().copied());
    quiet.fragment(func, code, &own)?;
    Ok(prod)
}

/// The result buffer's plumbing: the buffer, its aliases, the vector local and the element
/// locals — with the element locals and the vector local apart.
fn plumbing(
    data: &Data,
    code: &Value,
    buf: u16,
) -> Result<(HashSet<u16>, HashSet<u16>, u16), String> {
    let mut plumbing: HashSet<u16> = HashSet::default();
    plumbing.insert(buf);
    let mut out: Option<u16> = None;
    code.any_node(&mut |n| {
        if let Value::Set(v, rhs) = n
            && let Value::Call(callee, args) = rhs.unspan()
            && args.first().and_then(var_of) == Some(buf)
        {
            match op_name(data, *callee) {
                "OpRefAlias" => {
                    plumbing.insert(*v);
                }
                "OpGetField" => out = Some(*v),
                _ => {}
            }
        }
        false
    });
    let Some(out) = out else {
        return Err("no vector local in the buffer".into());
    };
    plumbing.insert(out);
    let mut elems: HashSet<u16> = HashSet::default();
    code.any_node(&mut |n| {
        if let Value::Set(v, rhs) = n
            && let Value::Call(callee, args) = rhs.unspan()
            && op_name(data, *callee) == "OpNewRecord"
            && args.first().and_then(var_of) == Some(out)
        {
            elems.insert(*v);
        }
        false
    });
    plumbing.extend(elems.iter().copied());
    // A mention inside a condition is one the statement walk cannot place.
    let mut stray = false;
    code.any_node(&mut |n| {
        if let Value::If(cond, _, _) = n {
            stray |= cond.any_node(&mut |m| matches!(m, Value::Var(x) if plumbing.contains(x)));
        }
        stray
    });
    if stray {
        return Err("the buffer is mentioned in a condition".into());
    }
    Ok((plumbing, elems, out))
}

/// Every statement that mentions the plumbing: dropped, kept, or an append's group.
fn classify(
    data: &Data,
    code: &Value,
    buf: u16,
    out: u16,
    prod: &mut Producer,
) -> Result<(), String> {
    let mut why: Option<String> = None;
    scan_blocks(code, &mut |ops| {
        let mut at = 0;
        while why.is_none() && at < ops.len() {
            let op = &ops[at];
            let mentions =
                op.any_node(&mut |n| matches!(n, Value::Var(x) if prod.plumbing.contains(x)));
            let container = matches!(
                op.unspan(),
                Value::Block(_) | Value::Loop(_) | Value::If(..) | Value::Insert(_)
            );
            if !mentions || container {
                at += 1;
                continue;
            }
            match producer_statement(data, prod, buf, out, ops, at) {
                Ok(Stmt::Drop) => {
                    prod.drop.insert(addr(op));
                    at += 1;
                }
                Ok(Stmt::Keep) => at += 1,
                Ok(Stmt::Group(group, finish)) => {
                    for o in &ops[at + 1..=finish] {
                        prod.drop.insert(addr(o));
                    }
                    prod.groups.insert(addr(op), group);
                    at = finish + 1;
                }
                Err(e) => why = Some(e),
            }
        }
    });
    why.map_or(Ok(()), Err)
}

/// The fields every append sets: a read of a field some append leaves unset reads its
/// default, which the twin has no value for.
fn common_fields(prod: &Producer) -> Vec<(i64, Kind)> {
    let mut fields: Option<Vec<(i64, Kind)>> = None;
    for group in prod.groups.values() {
        let mut set: Vec<(i64, Kind)> = group.sets.iter().map(|(o, k, _)| (*o, *k)).collect();
        set.sort_unstable();
        set.dedup();
        fields = Some(match fields {
            None => set,
            Some(f) => f.into_iter().filter(|x| set.contains(x)).collect(),
        });
    }
    fields.unwrap_or_default()
}

enum Stmt {
    Drop,
    Keep,
    Group(Group, usize),
}

/// Classify statement `at` of `ops`, which mentions the producer's plumbing.
fn producer_statement(
    data: &Data,
    prod: &Producer,
    buf: u16,
    out: u16,
    ops: &[Value],
    at: usize,
) -> Result<Stmt, String> {
    let is = |v: &Value, x: u16| var_of(v) == Some(x);
    match ops[at].unspan() {
        Value::Return(r) if is(r, buf) => Ok(Stmt::Keep),
        Value::Set(v, rhs) if prod.plumbing.contains(v) => match rhs.unspan() {
            Value::Null => Ok(Stmt::Drop),
            Value::Call(callee, args) => match op_name(data, *callee) {
                "OpRefAlias" | "OpGetField" if args.first().is_some_and(|a| is(a, buf)) => {
                    Ok(Stmt::Drop)
                }
                "OpNewRecord" if args.first().is_some_and(|a| is(a, out)) => {
                    group(data, prod, *v, out, ops, at)
                }
                other => Err(format!("binds plumbing with {other}")),
            },
            _ => Err("binds plumbing".into()),
        },
        Value::Call(callee, args) => {
            let first = args.first();
            match op_name(data, *callee) {
                "OpClearVector" | "OpDatabase" | "OpSetInt4"
                    if first.is_some_and(|a| is(a, buf)) =>
                {
                    Ok(Stmt::Drop)
                }
                "OpPreAllocVector" if first.is_some_and(|a| is(a, out)) => Ok(Stmt::Drop),
                // The free of another place, unless it is the buffer's: in the twin there is
                // no buffer, so the place is always distinct and the free is kept.
                "OpFreeRefIfDistinct"
                    if args.get(1).is_some_and(|a| is(a, buf))
                        && first
                            .and_then(var_of)
                            .is_some_and(|x| !prod.plumbing.contains(&x)) =>
                {
                    Ok(Stmt::Keep)
                }
                other => Err(format!("{other} mentions the buffer")),
            }
        }
        _ => Err("mentions the buffer".into()),
    }
}

/// The append minted at statement `at` into element `elem`: its field sets up to its finish.
fn group(
    data: &Data,
    prod: &Producer,
    elem: u16,
    out: u16,
    ops: &[Value],
    at: usize,
) -> Result<Stmt, String> {
    let mut grp = Group::default();
    for (pos, op) in ops.iter().enumerate().skip(at + 1) {
        match op.unspan() {
            Value::Line(_) => {}
            Value::Call(callee, args) => {
                let name = op_name(data, *callee);
                if name == "OpFinishRecord"
                    && args.first().and_then(var_of) == Some(out)
                    && args.get(1).and_then(var_of) == Some(elem)
                {
                    return Ok(Stmt::Group(grp, pos));
                }
                if name == "OpFreeText"
                    && let Some(t) = args.first().and_then(var_of)
                    && !prod.plumbing.contains(&t)
                {
                    grp.frees.push((addr(op), t));
                    continue;
                }
                let (Some(kind), Some(off), Some(val)) = (
                    Kind::of_set(name),
                    args.get(1).and_then(int_of),
                    args.get(2),
                ) else {
                    return Err(format!("{name} inside an append"));
                };
                if args.first().and_then(var_of) != Some(elem) {
                    return Err("an append sets another record".into());
                }
                if val.any_node(&mut |n| matches!(n, Value::Var(x) if prod.plumbing.contains(x))) {
                    return Err("a field value reads the vector".into());
                }
                grp.sets.push((off, kind, addr(val)));
            }
            _ => return Err("a statement inside an append".into()),
        }
    }
    Err("an append without its finish in the block".into())
}

/// Call `f` with the operator list of every block, loop body and insert in `code`.
fn scan_blocks(code: &Value, f: &mut dyn FnMut(&[Value])) {
    code.any_node(&mut |n| {
        match n {
            Value::Block(bl) | Value::Loop(bl) => f(&bl.operators),
            Value::Insert(ops) => f(ops),
            _ => {}
        }
        false
    });
}

// ---- consumers ----

/// Is `v` the element `runs[i]?` — the nullable read, or the read into a non-null place with
/// a default record for a null element, which an index under the length never takes?
#[must_use]
pub fn elem_expr(data: &Data, v: &Value, runs: u16, index: u16) -> bool {
    let plain = |v: &Value| {
        matches!(v.unspan(), Value::Call(d, a)
            if op_name(data, *d) == "OpGetVectorNullable"
                && a.first().and_then(var_of) == Some(runs)
                && a.get(2).and_then(var_of) == Some(index))
    };
    plain(v)
        || matches!(v.unspan(), Value::Block(bl) if {
            let ops: Vec<&Value> = bl
                .operators
                .iter()
                .filter(|o| !matches!(o.unspan(), Value::Line(_)))
                .collect();
            matches!(ops[..], [first, second] if matches!(
                (first.unspan(), second.unspan()),
                (Value::Set(t, rhs), Value::If(c, then, _))
                    if plain(rhs)
                        && var_of(then) == Some(*t)
                        && !c.any_node(&mut |m| matches!(m, Value::Var(x) if x != t))
            ))
        })
}

/// The statements that are not line markers.
fn significant(ops: &[Value]) -> Vec<&Value> {
    ops.iter()
        .filter(|o| !matches!(o.unspan(), Value::Line(_)))
        .collect()
}

/// What the producer's call READS: the variables of its arguments — not the buffers it is
/// handed to write — and every local of `func` whose type borrows from one of them (a field or
/// element view of it), to the fixed point.  The vector local is the call's result, never
/// something it reads.
fn call_reach(data: &Data, func: u32, call: &Value, runs: u16) -> HashSet<u16> {
    let vars = data.def(func).variables();
    let mut reach: HashSet<u16> = HashSet::default();
    if let Value::Set(_, rhs) = call.unspan()
        && let Value::Call(callee, args) = rhs.unspan()
    {
        let written = written_params(data, *callee);
        for (i, a) in args.iter().enumerate() {
            if written.contains(&i) {
                continue;
            }
            a.any_node(&mut |n| {
                if let Value::Var(x) = n {
                    reach.insert(*x);
                }
                false
            });
        }
    }
    reach.remove(&runs);
    loop {
        let before = reach.len();
        for v in 0..vars.count() {
            if v != runs
                && !reach.contains(&v)
                && vars.tp(v).depend().iter().any(|d| reach.contains(d))
            {
                reach.insert(v);
            }
        }
        if reach.len() == before {
            return reach;
        }
    }
}

/// Does `code` rebind a variable of `set`, or write through one — a store write, a free, a
/// call handed it to write?  A call through a function value handed one at all counts.
fn writes_into(data: &Data, code: &Value, set: &HashSet<u16>) -> bool {
    let rooted = |a: &Value| root(a, data).is_some_and(|r| set.contains(&r));
    code.any_node(&mut |n| match n {
        Value::Set(x, _) => set.contains(x),
        Value::Call(callee, args) => {
            let name = op_name(data, *callee);
            if name.starts_with("n_") && (*callee as usize) < data.definitions.len() {
                written_params(data, *callee)
                    .into_iter()
                    .any(|i| args.get(i).is_some_and(rooted))
            } else if let Some(t) = text_write_target(name) {
                args.get(t).is_some_and(rooted)
            } else {
                is_free(name) && args.first().is_some_and(rooted)
            }
        }
        Value::CallRef(_, args) => args.iter().any(rooted),
        _ => false,
    })
}

/// The shape of a counted walk over `runs`: `(index, body)`.
fn walk_shape<'v>(data: &Data, walk: &'v Value, runs: u16) -> Result<(u16, &'v Value), String> {
    let Value::Block(fb) = walk.unspan() else {
        return Err("the call is not followed by a walk".into());
    };
    let sig = significant(&fb.operators);
    let [
        Value::Set(end, len),
        Value::Set(counter, start),
        Value::Loop(lb),
    ] = sig.iter().map(|o| o.unspan()).collect::<Vec<_>>()[..]
    else {
        return Err("not a counted walk".into());
    };
    let is_len = matches!(len.unspan(), Value::Call(d, a)
        if matches!(op_name(data, *d), "t_6vector_len" | "OpLengthVector")
            && a.first().and_then(var_of) == Some(runs));
    if !is_len || int_of(start) != Some(-1) {
        return Err("not a walk of the whole vector".into());
    }
    let lsig = significant(&lb.operators);
    let [Value::Set(index, iter), Value::Block(_)] =
        lsig.iter().map(|o| o.unspan()).collect::<Vec<_>>()[..]
    else {
        return Err("not a counted loop".into());
    };
    // The iteration block reads only the counter and the end.
    if iter.any_node(&mut |n| matches!(n, Value::Var(x) if *x != *end && *x != *counter))
        || iter.any_node(&mut |n| {
            matches!(n, Value::Break(b) if *b != 0) || matches!(n, Value::Continue(_))
        })
    {
        return Err("an iteration with more than a counter".into());
    }
    // A mention of the end or the counter in the body is a mention of the walk's state.
    let body = lsig[1];
    let state = [*end, *counter];
    if body.any_node(&mut |n| matches!(n, Value::Var(x) | Value::Set(x, _) if state.contains(x))) {
        return Err("the body reads the walk's counter".into());
    }
    Ok((*index, body))
}

/// The fields the body reads of each element; every other mention of the vector, the index
/// or the element declines.
fn body_reads(
    data: &Data,
    prod: &Producer,
    body: &Value,
    runs: u16,
    index: u16,
    elem: Option<(u16, usize)>,
) -> Result<Vec<(i64, Kind)>, String> {
    let Value::Block(bl) = body.unspan() else {
        return Err("not a body".into());
    };
    let mut reads: Vec<(i64, Kind)> = Vec::new();
    let mut bad: Option<String> = None;
    let mut unset: Option<String> = None;
    for (n, o) in significant(&bl.operators).into_iter().enumerate() {
        if n == 0 && elem.is_some() {
            continue;
        }
        check_mentions(
            o,
            &mut |v| {
                let Value::Call(d, a) = v.unspan() else {
                    return false;
                };
                let Some(kind) = Kind::of_get(op_name(data, *d)) else {
                    return false;
                };
                let (Some(e), Some(off)) = (a.first(), a.get(1).and_then(int_of)) else {
                    return false;
                };
                let through = elem_expr(data, e, runs, index)
                    || elem.is_some_and(|(r, _)| var_of(e) == Some(r));
                if through {
                    if prod.fields.contains(&(off, kind)) {
                        reads.push((off, kind));
                    } else {
                        unset = Some(format!("reads field {off}, which not every append sets"));
                    }
                }
                through
            },
            &mut |x| x == runs || x == index || elem.is_some_and(|(r, _)| r == x),
            &mut bad,
        );
        if bad.is_some() || unset.is_some() {
            break;
        }
    }
    if let Some(b) = bad.or(unset) {
        return Err(b);
    }
    if reads.is_empty() {
        return Err("reads no field".into());
    }
    reads.sort_unstable();
    reads.dedup();
    Ok(reads)
}

/// The walk of the vector `runs` that statement `at` of `ops` binds, or why it declines.
#[allow(clippy::too_many_arguments)]
fn site(
    data: &Data,
    quiet: &Quiet,
    plan: &SwPlan,
    func: u32,
    ops: &[Value],
    at: usize,
    runs: u16,
    producer: u32,
) -> Result<(Site, Vec<(i64, Kind)>), String> {
    let def = data.def(func);
    // What the call reads: nothing between the call and the walk, and nothing in the walk,
    // may write it — the call moves down to the walk, and the producer runs inside it.
    let reach = call_reach(data, func, &ops[at], runs);
    let own = quiet.owned(func, &HashSet::default());
    let mut walk: Option<&Value> = None;
    for o in significant(&ops[at + 1..]) {
        if o.any_node(&mut |n| matches!(n, Value::Var(x) | Value::Set(x, _) if *x == runs)) {
            walk = Some(o);
            break;
        }
        let mut esc = false;
        escapes(o, 0, &mut esc);
        if esc || writes_into(data, o, &reach) {
            return Err("a statement between the call and the walk changes what it reads".into());
        }
        quiet.fragment(func, o, &own)?;
    }
    let Some(walk) = walk else {
        return Err("nothing after the call".into());
    };
    let (index, body) = walk_shape(data, walk, runs)?;
    let Value::Block(bl) = body.unspan() else {
        return Err("not a body".into());
    };
    let body_ops = significant(&bl.operators);
    let elem = match body_ops.first().map(|o| o.unspan()) {
        Some(Value::Set(r, rhs)) if elem_expr(data, rhs, runs, index) => {
            Some((*r, addr(body_ops[0])))
        }
        _ => None,
    };
    let reads = body_reads(data, &plan.producers[&producer], body, runs, index, elem)?;
    let mut esc = false;
    escapes(body, 0, &mut esc);
    if esc {
        return Err("the body leaves the walk".into());
    }
    // The body is quiet, and writes nothing the producer reads.
    let mut not: HashSet<u16> = HashSet::default();
    not.insert(runs);
    let own = quiet.owned(func, &not);
    for o in &body_ops {
        quiet.fragment(func, o, &own)?;
    }
    if writes_into(data, body, &reach) {
        return Err("the body writes what the call reads".into());
    }
    // The vector local is mentioned only by this call, this walk, its null init and frees.
    let outside = count_mentions(def.code(), runs) - count_mentions(walk, runs);
    let frees = count_free_mentions(data, def.code(), runs);
    let inits = count_null_inits(def.code(), runs);
    if outside != 1 + frees + inits {
        return Err(format!(
            "{} is mentioned outside the walk",
            def.variables().name(runs)
        ));
    }
    let site = Site {
        producer,
        runs,
        call: addr(&ops[at]),
        walk: addr(walk),
        elem,
        index,
        twin: 0,
    };
    Ok((site, reads))
}

/// Walk `v`; where `accept` answers true the node is a read the rule rewrites and its operands
/// are not searched.  Any other mention of a variable `watched` answers true for is recorded in
/// `bad`.
fn check_mentions(
    v: &Value,
    accept: &mut dyn FnMut(&Value) -> bool,
    watched: &mut dyn FnMut(u16) -> bool,
    bad: &mut Option<String>,
) {
    if bad.is_some() || accept(v) {
        return;
    }
    if let Value::Var(x) | Value::Set(x, _) = v.unspan()
        && watched(*x)
    {
        *bad = Some(format!("the walk mentions variable {x} otherwise"));
        return;
    }
    v.unspan()
        .for_each_child(&mut |c| check_mentions(c, accept, watched, bad));
}

/// Does a `break`, `continue`, `return` or `yield` in `v` leave the loop `depth` levels out?
fn escapes(v: &Value, depth: u16, found: &mut bool) {
    if *found {
        return;
    }
    match v.unspan() {
        Value::Break(n) | Value::Continue(n) if *n >= depth => *found = true,
        Value::Return(_) | Value::Yield(_) => *found = true,
        Value::Loop(lb) => {
            for o in &lb.operators {
                escapes(o, depth + 1, found);
            }
        }
        other => other.for_each_child(&mut |c| escapes(c, depth, found)),
    }
}

fn count_mentions(v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.any_node(&mut |m| {
        if matches!(m, Value::Var(y) | Value::Set(y, _) if *y == x) {
            n += 1;
        }
        false
    });
    n
}

fn count_free_mentions(data: &Data, v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.any_node(&mut |m| {
        if let Value::Call(d, a) = m
            && is_free(op_name(data, *d))
        {
            n += a.iter().filter(|y| var_of(y) == Some(x)).count();
        }
        false
    });
    n
}

fn count_null_inits(v: &Value, x: u16) -> usize {
    let mut n = 0;
    v.any_node(&mut |m| {
        if let Value::Set(y, rhs) = m
            && *y == x
            && matches!(rhs.unspan(), Value::Null)
        {
            n += 1;
        }
        false
    });
    n
}

/// Every `(consumer, producer)` pair of a call binding a vector of records.
fn calls_of(data: &Data, reachable: &std::collections::HashSet<u32>) -> Vec<(u32, u32)> {
    candidates(data)
        .into_iter()
        .filter(|g| reachable.is_empty() || reachable.contains(g))
        .flat_map(|g| {
            let mut out = Vec::new();
            data.def(g).code().any_node(&mut |n| {
                if let Value::Set(_, rhs) = n
                    && let Value::Call(d, _) = rhs.unspan()
                    && vector_of_records(data, *d)
                    && data.def(*d).name().starts_with("n_")
                {
                    out.push((g, *d));
                }
                false
            });
            out
        })
        .collect()
}

/// The program's plan.  `reachable` is the emitted functions (`fnref::dispatch_arms` reads it).
#[must_use]
pub fn plan(data: &Data, reachable: &std::collections::HashSet<u32>) -> SwPlan {
    let mut plan = SwPlan::default();
    if disabled() {
        return plan;
    }
    let calls = calls_of(data, reachable);
    if calls.is_empty() {
        return plan;
    }
    let quiet = Quiet::new(data, reachable);
    let name = |d: u32| data.def(d).name().to_string();
    for (_, pd) in &calls {
        if plan.producers.contains_key(pd) {
            continue;
        }
        match producer(data, &quiet, *pd) {
            Ok(prod) => {
                trace(|| format!("{}: a producer of {} appends", name(*pd), prod.groups.len()));
                plan.producers.insert(*pd, prod);
            }
            Err(e) => trace(|| format!("{}: no producer — {e}", name(*pd))),
        }
    }
    let mut consumers: Vec<u32> = calls.iter().map(|(g, _)| *g).collect();
    consumers.sort_unstable();
    consumers.dedup();
    for func in consumers {
        admit_sites(data, &quiet, &mut plan, func);
    }
    // A producer no site streams keeps no twin.
    plan.producers
        .retain(|pd, _| plan.twins.iter().any(|(p, _)| p == pd));
    plan
}

/// The streamed walks of consumer `func`, into the plan.
fn admit_sites(data: &Data, quiet: &Quiet, plan: &mut SwPlan, func: u32) {
    let name = |d: u32| data.def(d).name().to_string();
    let mut found: Vec<(usize, u16, u32)> = Vec::new();
    scan_blocks(data.def(func).code(), &mut |ops| {
        for o in ops {
            if let Value::Set(runs, rhs) = o.unspan()
                && let Value::Call(d, _) = rhs.unspan()
                && plan.producers.contains_key(d)
            {
                found.push((addr(o), *runs, *d));
            }
        }
    });
    for (stmt, runs, pd) in found {
        let mut res: Option<Result<(Site, Vec<(i64, Kind)>), String>> = None;
        scan_blocks(data.def(func).code(), &mut |ops| {
            if res.is_none()
                && let Some(at) = ops.iter().position(|o| addr(o) == stmt)
            {
                res = Some(site(data, quiet, plan, func, ops, at, runs, pd));
            }
        });
        match res {
            Some(Ok((mut site, reads))) => {
                site.twin =
                    if let Some(t) = plan.twins.iter().position(|(p, f)| *p == pd && *f == reads) {
                        t
                    } else {
                        plan.twins.push((pd, reads.clone()));
                        plan.twins.len() - 1
                    };
                trace(|| {
                    format!(
                        "{}: walk of {} streamed, fields {reads:?} (twin {})",
                        name(func),
                        name(pd),
                        site.twin
                    )
                });
                let idx = plan.sites.get(&func).map_or(0, Vec::len);
                plan.at.insert(site.call, (func, idx));
                plan.at.insert(site.walk, (func, idx));
                plan.sites.entry(func).or_default().push(site);
            }
            Some(Err(e)) => {
                trace(|| format!("{}: walk of {} declined — {e}", name(func), name(pd)))
            }
            None => {}
        }
    }
}
