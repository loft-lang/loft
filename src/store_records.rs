// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Store records — every store a function can hold: where it is created, which variables
//! hold it, which operations use it, and how each path ends it (doc/claude/STORE_RECORDS.md).
//!
//! Built from the CLOSED program's IR (the end of `compile::close_program`), so the records
//! describe exactly what both backends translate.  **Independence:** nothing here reads a fact
//! the code generator decided from — no `deps`, `skip_free`, `owns_store`, no
//! `return_adopts_fresh_store`.  A callee's answer comes from the callee's own body, through a
//! least-fixpoint summary over the call graph, so a recursive function gets the same answer
//! whichever parser pass read it first.
//!
//! An OBSERVER: it never rewrites IR.  Reached only through
//!   * `LOFT_STORE_TABLE=<fn substring>` — print the records of the matching functions;
//!   * `LOFT_OWN_ORACLE=check-records` — report every finding as a `RED` line.
//!
//! The analysis is an abstract run of each body over a SET of states.  A state maps every
//! variable to what it points into (nothing, one store of this function, a parameter's store,
//! or unknown) and every store to whether it is still live, ended or handed on.  States are
//! kept apart rather than joined, so an `if` that mints in one arm and frees in a later
//! matching arm does not read as a leak; identical states merge.  A store is keyed by its
//! creation SITE, plus one older generation, so a loop that mints every pass sees the last
//! pass's store when it mints the next.

use crate::data::{Data, DefType, Value};
use std::collections::{BTreeMap, BTreeSet};

/// What a variable points into.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Pt {
    /// No store: null, or a value that is not a reference.
    Null,
    /// A store this function created, keyed by its site and generation.
    Store(SKey),
    /// The store a parameter brought in: the caller's, never this function's to end.
    Param(u16),
    /// Something the analysis does not follow (a native's answer, a closure call): no finding
    /// is ever drawn from it.
    Unknown,
}

/// A store: its creation site (an index into `Records::sites`) and whether it is the store
/// that site made most recently (`false`) or the one before (`true`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
struct SKey {
    site: u32,
    older: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Status {
    Live,
    /// Freed on this path.
    Ended,
    /// Returned, or stored where another owner holds it.
    HandedOn,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
struct State {
    vars: BTreeMap<u16, Pt>,
    stores: BTreeMap<SKey, Status>,
}

/// How a store comes into being.
#[derive(Clone, Debug)]
enum Origin {
    /// `OpDatabase` into a variable.
    Mint,
    /// A call to a function whose answer is a store of its own.
    Call(u32),
}

/// A function's answer, as its callers see it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Summary {
    /// Not yet known (the fixpoint's start), or the function returns no store.
    Bottom,
    /// A store the function created and hands over.
    Fresh,
    /// The store the i-th argument brought in.
    Arg(u16),
    /// Fresh on some paths, the i-th argument's store on others.
    Join(u16),
    /// Not followed.
    Unknown,
}

impl Summary {
    fn join(self, other: Summary) -> Summary {
        use Summary::{Arg, Bottom, Fresh, Join, Unknown};
        match (self, other) {
            (Bottom, x) | (x, Bottom) => x,
            (a, b) if a == b => a,
            (Fresh, Arg(i))
            | (Arg(i), Fresh)
            | (Join(i), Fresh | Arg(_))
            | (Fresh | Arg(_), Join(i)) => Join(i),
            _ => Unknown,
        }
    }
}

/// One record per creation site.
#[derive(Default)]
struct SiteRec {
    origin: Option<Origin>,
    holders: BTreeSet<u16>,
    uses: BTreeSet<String>,
    ends: BTreeSet<String>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Finding {
    kind: &'static str,
    site: u32,
    detail: String,
}

/// The ops the analysis gives a meaning to, resolved once.
struct Ops {
    database: u32,
    set_dbref: u32,
    ref_is_null: u32,
    not: u32,
    unconditional: BTreeSet<u32>,
    conditional: BTreeSet<u32>,
    text_free: u32,
}

impl Ops {
    fn new(data: &Data) -> Self {
        let s = data.op_sets();
        Ops {
            database: data.def_nr("OpDatabase"),
            set_dbref: data.def_nr("OpSetDbRef"),
            ref_is_null: data.def_nr("OpRefIsNull"),
            not: data.def_nr("OpNot"),
            unconditional: s.unconditional_ref_frees.iter().copied().collect(),
            conditional: s.conditional_ref_frees.iter().copied().collect(),
            text_free: s.text_free,
        }
    }
}

/// The most states kept at one program point; past it the function is marked imprecise and
/// draws no findings.
const STATE_CAP: usize = 256;

struct Run<'a> {
    data: &'a Data,
    ops: &'a Ops,
    summaries: &'a BTreeMap<u32, Summary>,
    d_nr: u32,
    /// Creation site per IR node address: stable while the body is not rewritten.
    site_of: BTreeMap<usize, u32>,
    sites: Vec<SiteRec>,
    findings: BTreeSet<Finding>,
    /// The function's answer, joined over every return.
    answer: Summary,
    imprecise: bool,
    /// Per enclosing loop: the states that leave it (`break`) and that start its next pass
    /// (`continue`).
    loops: Vec<(Vec<State>, Vec<State>)>,
    /// The parameter variables, in order.
    params: Vec<u16>,
}

impl Run<'_> {
    fn site(&mut self, node: &Value, origin: Origin) -> u32 {
        let key = std::ptr::from_ref(node) as usize;
        if let Some(&s) = self.site_of.get(&key) {
            return s;
        }
        let s = self.sites.len() as u32;
        self.sites.push(SiteRec {
            origin: Some(origin),
            ..Default::default()
        });
        self.site_of.insert(key, s);
        s
    }

    fn var_name(&self, v: u16) -> String {
        self.data.def(self.d_nr).variables.name(v).to_string()
    }

    fn find(&mut self, kind: &'static str, site: u32, detail: String) {
        self.findings.insert(Finding { kind, site, detail });
    }

    fn pt(st: &State, v: u16) -> Pt {
        st.vars.get(&v).copied().unwrap_or(Pt::Null)
    }

    /// Does any variable other than `except` point into `k`?
    fn held_elsewhere(st: &State, k: SKey, except: u16) -> bool {
        st.vars
            .iter()
            .any(|(&v, &p)| v != except && p == Pt::Store(k))
    }

    /// `v` is about to point somewhere else: a live store only `v` held is now unreachable.
    fn drop_binding(&mut self, st: &State, v: u16) {
        if let Pt::Store(k) = Self::pt(st, v)
            && st.stores.get(&k) == Some(&Status::Live)
            && !Self::held_elsewhere(st, k, v)
        {
            let name = self.var_name(v);
            self.find(
                "leak",
                k.site,
                format!("`{name}` is rebound while it holds the only reference"),
            );
        }
    }

    /// Bind `v` to `p` in `st`.
    fn bind(&mut self, st: &mut State, v: u16, p: Pt) {
        self.drop_binding(st, v);
        if let Pt::Store(k) = p {
            self.sites[k.site as usize].holders.insert(v);
        }
        st.vars.insert(v, p);
    }

    /// A fresh store at `site` in `st`: an older store of the same site moves to the older
    /// generation; one already there is folded into it.
    fn mint(st: &mut State, site: u32) -> SKey {
        let cur = SKey { site, older: false };
        let old = SKey { site, older: true };
        if let Some(status) = st.stores.remove(&cur) {
            st.stores.insert(old, status);
            for p in st.vars.values_mut() {
                if *p == Pt::Store(cur) {
                    *p = Pt::Store(old);
                }
            }
        }
        st.stores.insert(cur, Status::Live);
        cur
    }

    /// A read of `v` that goes through the store it names.
    fn use_var(&mut self, st: &State, v: u16, how: &str) {
        if let Pt::Store(k) = Self::pt(st, v) {
            self.sites[k.site as usize].uses.insert(how.to_string());
            if st.stores.get(&k) == Some(&Status::Ended) {
                let name = self.var_name(v);
                self.find(
                    "use-after-end",
                    k.site,
                    format!("`{name}` is read by {how} after its store was freed"),
                );
            }
        }
    }

    /// End the store `v` names (an unconditional free).
    fn end(&mut self, st: &mut State, v: u16, how: &str) {
        match Self::pt(st, v) {
            Pt::Store(k) => {
                let label = format!("{how}({})", self.var_name(v));
                self.sites[k.site as usize].ends.insert(label);
                match st.stores.get(&k) {
                    Some(Status::Ended) => {
                        let name = self.var_name(v);
                        self.find(
                            "double-end",
                            k.site,
                            format!("`{name}` is freed by {how} after its store was already freed"),
                        );
                    }
                    Some(Status::HandedOn) => {
                        let name = self.var_name(v);
                        self.find(
                            "wrong-end",
                            k.site,
                            format!("`{name}` is freed by {how} after its store was handed on"),
                        );
                    }
                    _ => {
                        st.stores.insert(k, Status::Ended);
                    }
                }
            }
            Pt::Param(_) if how != "OpFreeText" => {
                let name = self.var_name(v);
                self.find(
                    "wrong-end",
                    u32::MAX,
                    format!("`{name}` is freed by {how}, but its store is the caller's"),
                );
            }
            _ => {}
        }
    }

    fn hand_on(st: &mut State, p: Pt) {
        if let Pt::Store(k) = p
            && st.stores.get(&k) == Some(&Status::Live)
        {
            st.stores.insert(k, Status::HandedOn);
        }
    }

    /// Every live store left at an exit is a leak.
    fn exit(&mut self, st: &State, how: &str) {
        let live: Vec<SKey> = st
            .stores
            .iter()
            .filter(|(_, s)| **s == Status::Live)
            .map(|(k, _)| *k)
            .collect();
        for k in live {
            let holders: Vec<String> = st
                .vars
                .iter()
                .filter(|(_, p)| **p == Pt::Store(k))
                .map(|(v, _)| format!("`{}`", self.var_name(*v)))
                .collect();
            let held = if holders.is_empty() {
                "nothing".to_string()
            } else {
                holders.join(", ")
            };
            self.find(
                "leak",
                k.site,
                format!("a path reaches {how} with the store live, held by {held}"),
            );
        }
    }

    fn cap(&mut self, mut states: Vec<(State, Pt)>) -> Vec<(State, Pt)> {
        states.sort();
        states.dedup();
        if states.len() > STATE_CAP {
            self.imprecise = true;
            states.truncate(STATE_CAP);
        }
        states
    }

    /// Run `v` from `st`: every state it can end in, with what its value points into.  An
    /// empty answer means no path falls through (a `break`, `continue` or `return`).
    fn eval(&mut self, v: &Value, st: State) -> Vec<(State, Pt)> {
        match v {
            Value::Span(b) => self.eval(&b.1, st),
            Value::Block(b) => self.seq(&b.operators, st),
            Value::Insert(ops) => self.seq(ops, st),
            Value::Loop(b) => self.run_loop(&b.operators, st),
            Value::Var(x) => {
                let p = Self::pt(&st, *x);
                vec![(st, p)]
            }
            Value::Set(x, val) => {
                let out = self.eval(val, st);
                let mut res = Vec::new();
                for (mut s, p) in out {
                    self.bind(&mut s, *x, p);
                    res.push((s, Pt::Null));
                }
                self.cap(res)
            }
            Value::If(c, t, f) => {
                let mut res = Vec::new();
                for (s, _) in self.eval(c, st) {
                    let (take_then, take_else) = self.decide(c, &s);
                    if take_then {
                        res.extend(self.eval(t, s.clone()));
                    }
                    if take_else {
                        res.extend(self.eval(f, s));
                    }
                }
                self.cap(res)
            }
            Value::Return(val) => {
                for (mut s, p) in self.eval(val, st) {
                    if let Value::Var(x) = val.unspan() {
                        self.use_var(&s, *x, "the return");
                    }
                    self.answer = self.answer.join(self.summary_of(p));
                    Self::hand_on(&mut s, p);
                    self.exit(&s, "a return");
                }
                Vec::new()
            }
            Value::Break(n) => {
                let i = self.loops.len().checked_sub(1 + *n as usize);
                if let Some(i) = i {
                    self.loops[i].0.push(st);
                } else {
                    self.imprecise = true;
                }
                Vec::new()
            }
            Value::Continue(n) => {
                let i = self.loops.len().checked_sub(1 + *n as usize);
                if let Some(i) = i {
                    self.loops[i].1.push(st);
                } else {
                    self.imprecise = true;
                }
                Vec::new()
            }
            Value::Drop(x) => self
                .eval(x, st)
                .into_iter()
                .map(|(s, _)| (s, Pt::Null))
                .collect(),
            Value::Call(d, args) => self.call(v, *d, args, st),
            Value::CallRef(_, args) => {
                let out = self.args(args, st);
                out.into_iter().map(|(s, _)| (s, Pt::Unknown)).collect()
            }
            Value::TuplePut(_, _, x) => {
                let mut res = Vec::new();
                for (mut s, p) in self.eval(x, st) {
                    Self::hand_on(&mut s, p);
                    res.push((s, Pt::Null));
                }
                res
            }
            Value::Tuple(xs) => {
                let out = self.args(xs, st);
                out.into_iter()
                    .map(|(mut s, ps)| {
                        for p in ps {
                            Self::hand_on(&mut s, p);
                        }
                        (s, Pt::Null)
                    })
                    .collect()
            }
            Value::TupleGet(..) => vec![(st, Pt::Unknown)],
            Value::Yield(_) | Value::Iter(..) => {
                self.imprecise = true;
                vec![(st, Pt::Unknown)]
            }
            _ => vec![(st, Pt::Null)],
        }
    }

    /// Which arms of an `if` on condition `c` can run in `st`: a null test of a variable
    /// whose binding is known takes one arm.
    fn decide(&self, c: &Value, st: &State) -> (bool, bool) {
        let mut c = c.unspan();
        let mut flip = false;
        while let Value::Call(d, a) = c
            && *d == self.ops.not
            && a.len() == 1
        {
            flip = !flip;
            c = a[0].unspan();
        }
        if let Value::Call(d, a) = c
            && *d == self.ops.ref_is_null
            && let Some(Value::Var(x)) = a.first().map(Value::unspan)
        {
            let is_null = match Self::pt(st, *x) {
                Pt::Null => Some(true),
                Pt::Store(_) | Pt::Param(_) => Some(false),
                Pt::Unknown => None,
            };
            if let Some(n) = is_null {
                return if n == flip {
                    (false, true)
                } else {
                    (true, false)
                };
            }
        }
        (true, true)
    }

    fn seq(&mut self, ops: &[Value], st: State) -> Vec<(State, Pt)> {
        let mut cur = vec![(st, Pt::Null)];
        for op in ops {
            let mut next = Vec::new();
            for (s, _) in cur {
                next.extend(self.eval(op, s));
            }
            cur = self.cap(next);
            if cur.is_empty() {
                break;
            }
        }
        cur
    }

    fn run_loop(&mut self, body: &[Value], st: State) -> Vec<(State, Pt)> {
        let mut seen: BTreeSet<State> = BTreeSet::new();
        let mut todo = vec![st];
        let mut exits = Vec::new();
        for _ in 0..8 {
            let fresh: Vec<State> = todo
                .into_iter()
                .filter(|s| seen.insert(s.clone()))
                .collect();
            if fresh.is_empty() {
                break;
            }
            todo = Vec::new();
            for s in fresh {
                self.loops.push((Vec::new(), Vec::new()));
                let out = self.seq(body, s);
                let (brk, cont) = self.loops.pop().unwrap_or_default();
                exits.extend(brk);
                todo.extend(cont);
                todo.extend(out.into_iter().map(|(s, _)| s));
            }
            if seen.len() > STATE_CAP {
                self.imprecise = true;
                break;
            }
        }
        let res = exits.into_iter().map(|s| (s, Pt::Null)).collect();
        self.cap(res)
    }

    /// Evaluate call arguments left to right: every state, with what each argument points
    /// into.  A bare variable argument is a use of its store.
    fn args(&mut self, args: &[Value], st: State) -> Vec<(State, Vec<Pt>)> {
        let mut cur = vec![(st, Vec::new())];
        for a in args {
            let mut next = Vec::new();
            for (s, ps) in cur {
                for (s2, p) in self.eval(a, s) {
                    if let Value::Var(x) = a.unspan() {
                        self.use_var(&s2, *x, "a call");
                    }
                    let mut ps2 = ps.clone();
                    ps2.push(p);
                    next.push((s2, ps2));
                }
            }
            next.sort();
            next.dedup();
            if next.len() > STATE_CAP {
                self.imprecise = true;
                next.truncate(STATE_CAP);
            }
            cur = next;
        }
        cur
    }

    fn summary_of(&self, p: Pt) -> Summary {
        match p {
            Pt::Null => Summary::Bottom,
            Pt::Store(_) => Summary::Fresh,
            Pt::Param(v) => self
                .params
                .iter()
                .position(|&x| x == v)
                .map_or(Summary::Unknown, |i| Summary::Arg(i as u16)),
            Pt::Unknown => Summary::Unknown,
        }
    }

    fn call(&mut self, node: &Value, d: u32, args: &[Value], st: State) -> Vec<(State, Pt)> {
        let ops = self.ops;
        let first_var = args.first().and_then(|a| {
            if let Value::Var(x) = a.unspan() {
                Some(*x)
            } else {
                None
            }
        });
        // A free: its own argument is not a use.
        if ops.unconditional.contains(&d) || d == ops.text_free {
            let name = self.data.def(d).name().to_string();
            let mut st = st;
            if let Some(x) = first_var {
                self.end(&mut st, x, &name);
            }
            return vec![(st, Pt::Null)];
        }
        if ops.conditional.contains(&d) {
            // A free that is a no-op where the freed variable and its witness name one store.
            let name = self.data.def(d).name().to_string();
            let witness = args.get(1).and_then(|a| {
                if let Value::Var(x) = a.unspan() {
                    Some(*x)
                } else {
                    None
                }
            });
            let mut st = st;
            if let Some(x) = first_var {
                let same = witness.is_some_and(|w| Self::pt(&st, x) == Self::pt(&st, w));
                if !same {
                    self.end(&mut st, x, &name);
                } else if let Pt::Store(k) = Self::pt(&st, x) {
                    let label = format!("{name}({}) skipped: witness holds it", self.var_name(x));
                    self.sites[k.site as usize].ends.insert(label);
                }
            }
            return vec![(st, Pt::Null)];
        }
        if d == ops.database
            && let Some(x) = first_var
        {
            let site = self.site(node, Origin::Mint);
            let mut st = st;
            self.drop_binding(&st, x);
            let k = Self::mint(&mut st, site);
            self.sites[site as usize].holders.insert(x);
            st.vars.insert(x, Pt::Store(k));
            return vec![(st, Pt::Null)];
        }
        let out = self.args(args, st);
        let def = self.data.def(d);
        let heap = def.returned().heap_dep().is_some();
        let bodied = !matches!(def.code.unspan(), Value::Null) && !def.name().starts_with("Op");
        let mut res = Vec::new();
        for (mut s, ps) in out {
            if d == ops.set_dbref
                && let Some(&p) = ps.get(2)
            {
                // A reference written into another store's field: that store holds it now.
                Self::hand_on(&mut s, p);
            }
            if !heap {
                res.push((s, Pt::Null));
                continue;
            }
            if !bodied {
                // An operator answering a reference answers one into its first argument's
                // store; a native's answer is not followed.
                let p = if def.name().starts_with("Op") {
                    ps.first().copied().unwrap_or(Pt::Unknown)
                } else {
                    Pt::Unknown
                };
                res.push((s, p));
                continue;
            }
            match self.summaries.get(&d).copied().unwrap_or(Summary::Unknown) {
                Summary::Bottom => res.push((s, Pt::Null)),
                Summary::Unknown => res.push((s, Pt::Unknown)),
                Summary::Fresh => {
                    let site = self.site(node, Origin::Call(d));
                    let k = Self::mint(&mut s, site);
                    res.push((s, Pt::Store(k)));
                }
                Summary::Arg(i) => {
                    res.push((s, ps.get(i as usize).copied().unwrap_or(Pt::Unknown)))
                }
                Summary::Join(i) => {
                    let viewed = ps.get(i as usize).copied().unwrap_or(Pt::Unknown);
                    res.push((s.clone(), viewed));
                    let site = self.site(node, Origin::Call(d));
                    let k = Self::mint(&mut s, site);
                    res.push((s, Pt::Store(k)));
                }
            }
        }
        self.cap(res)
    }
}

/// Run one function's body; its answer, findings and records.
fn run_fn<'a>(
    data: &'a Data,
    ops: &'a Ops,
    summaries: &'a BTreeMap<u32, Summary>,
    d_nr: u32,
) -> Run<'a> {
    let def = data.def(d_nr);
    let params = def.variables.arguments();
    let mut st = State {
        vars: BTreeMap::new(),
        stores: BTreeMap::new(),
    };
    for &p in &params {
        st.vars.insert(p, Pt::Param(p));
    }
    let mut run = Run {
        data,
        ops,
        summaries,
        d_nr,
        site_of: BTreeMap::new(),
        sites: Vec::new(),
        findings: BTreeSet::new(),
        answer: Summary::Bottom,
        imprecise: false,
        loops: Vec::new(),
        params,
    };
    let body = def.code.clone();
    let out = run.eval(&body, st);
    for (mut s, p) in out {
        run.answer = run.answer.join(run.summary_of(p));
        Run::hand_on(&mut s, p);
        run.exit(&s, "the end of the function");
    }
    run
}

fn is_analysed(data: &Data, d: u32) -> bool {
    let def = data.def(d);
    matches!(def.def_type, DefType::Function)
        && !matches!(def.code.unspan(), Value::Null)
        && !def.name().starts_with("Op")
}

/// The least fixpoint of every function's answer over the call graph.
fn summaries(data: &Data, ops: &Ops) -> BTreeMap<u32, Summary> {
    let fns: Vec<u32> = (0..data.definitions())
        .filter(|&d| is_analysed(data, d))
        .collect();
    let mut sum: BTreeMap<u32, Summary> = fns.iter().map(|&d| (d, Summary::Bottom)).collect();
    for _ in 0..12 {
        let mut changed = false;
        for &d in &fns {
            let run = run_fn(data, ops, &sum, d);
            let new = if run.imprecise {
                Summary::Unknown
            } else {
                sum[&d].join(run.answer)
            };
            if new != sum[&d] {
                sum.insert(d, new);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    sum
}

/// The observer entry point, at the end of `close_program`.
pub fn observe(data: &Data) {
    let table = std::env::var("LOFT_STORE_TABLE").ok();
    let check = std::env::var("LOFT_OWN_ORACLE").is_ok_and(|m| m == "check-records");
    if table.is_none() && !check {
        return;
    }
    let ops = Ops::new(data);
    let sum = summaries(data, &ops);
    let mut reds = 0usize;
    for d in (0..data.definitions()).filter(|&d| is_analysed(data, d)) {
        let name = data.def(d).name().to_string();
        let shown = table
            .as_deref()
            .is_some_and(|t| !t.is_empty() && name.contains(t));
        if !shown && !check {
            continue;
        }
        let run = run_fn(data, &ops, &sum, d);
        if shown {
            print_records(&run, &name, sum[&d]);
        }
        if check && !run.imprecise {
            for f in &run.findings {
                eprintln!(
                    "RED {name}: store-{} {}{}",
                    f.kind,
                    site_label(&run, f.site),
                    f.detail
                );
                reds += 1;
            }
        }
    }
    if check {
        eprintln!("STORE-RECORDS: {reds} RED finding(s)");
    }
}

fn site_label(run: &Run, site: u32) -> String {
    match run.sites.get(site as usize).and_then(|s| s.origin.as_ref()) {
        Some(Origin::Mint) => format!("[store #{site}, minted] "),
        Some(Origin::Call(d)) => {
            format!("[store #{site}, answered by {}] ", run.data.def(*d).name())
        }
        None => String::new(),
    }
}

fn print_records(run: &Run, name: &str, sum: Summary) {
    eprintln!(
        "[stores] fn {name}: answers {sum:?}{}",
        if run.imprecise {
            " (imprecise: no findings)"
        } else {
            ""
        }
    );
    for (i, s) in run.sites.iter().enumerate() {
        let holders: Vec<String> = s.holders.iter().map(|v| run.var_name(*v)).collect();
        eprintln!(
            "[stores]   #{i} {} holders=[{}] uses=[{}] ends=[{}]",
            site_label(run, i as u32).trim_end(),
            holders.join(", "),
            s.uses.iter().cloned().collect::<Vec<_>>().join(", "),
            s.ends.iter().cloned().collect::<Vec<_>>().join(", "),
        );
    }
    for f in &run.findings {
        eprintln!(
            "[stores]   {} {}{}",
            f.kind,
            site_label(run, f.site),
            f.detail
        );
    }
}
