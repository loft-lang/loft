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
//!   * `LOFT_OWN_ORACLE=check-records` — report every finding as a `RED` line;
//!   * `LOFT_STORE_PROFILE=1` — the analysis' cost, and the slowest functions.
//!
//! **The analysis.**  One state per program point, joined where paths meet, so its cost
//! grows with the size of the body rather than with the number of paths through it.  A state
//! maps every variable to the stores it MAY point into, and every store to the set of
//! statuses it may have (absent, live, ended, handed on) plus two MUST-holder sets: the
//! variables that hold it on every path where it is live, and on every path where it is
//! ended.  Those sets keep what separate paths would: a free of `x` definitely ends a store
//! only `x` holds whenever it is live, even when `x` is null on other paths, and a second free
//! or a read is a fault only where the variable is known to name the ended store.
//!
//! A store is keyed by its creation SITE, plus one older generation, so a loop that mints
//! every pass sees the previous pass's store when it mints the next.

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

/// A store: its creation site (an index into `Run::sites`) and whether it is the store
/// that site made most recently (`false`) or the one before (`true`).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
struct SKey {
    site: u32,
    older: bool,
}

/// The statuses a store may have, as bits.
const ABSENT: u8 = 1;
const LIVE: u8 = 2;
const ENDED: u8 = 4;
const HANDED: u8 = 8;

/// What a state knows about one store.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct SInfo {
    /// The statuses it may have on the paths reaching this point.
    status: u8,
    /// The variables that point into it on EVERY path where it is live.
    live: BTreeSet<u16>,
    /// The variables that point into it on EVERY path where it is ended.
    ended: BTreeSet<u16>,
}

impl SInfo {
    fn has(&self, bit: u8) -> bool {
        self.status & bit != 0
    }
}

type Pts = BTreeSet<Pt>;

#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct State {
    /// A variable missing from the map points nowhere (null).
    vars: BTreeMap<u16, Pts>,
    /// A store missing from the map is absent.
    stores: BTreeMap<SKey, SInfo>,
    /// Per closure record that stays in this frame: the stores its capture slots hold
    /// (`@FR-L-CapOwn` — such a record BORROWS what it captured; the frame releases it).
    slots: BTreeMap<u16, Pts>,
}

/// A value: the stores it may point into, and the stores CORRELATED with it — live only on
/// paths where the value names them (a fresh store, or one a variable must-holds).
#[derive(Clone, PartialEq, Eq, Debug, Default)]
struct Val {
    pts: Pts,
    corr: BTreeSet<SKey>,
}

impl Val {
    fn null() -> Self {
        Val {
            pts: [Pt::Null].into(),
            corr: BTreeSet::new(),
        }
    }
    fn unknown() -> Self {
        Val {
            pts: [Pt::Unknown].into(),
            corr: BTreeSet::new(),
        }
    }
}

fn info_of(st: &State, k: SKey) -> SInfo {
    st.stores.get(&k).cloned().unwrap_or(SInfo {
        status: ABSENT,
        ..Default::default()
    })
}

fn join_info(a: &SInfo, b: &SInfo) -> SInfo {
    let meet = |x: &BTreeSet<u16>, xa: bool, y: &BTreeSet<u16>, ya: bool| match (xa, ya) {
        (true, true) => x.intersection(y).copied().collect(),
        (true, false) => x.clone(),
        (false, true) => y.clone(),
        (false, false) => BTreeSet::new(),
    };
    SInfo {
        status: a.status | b.status,
        live: meet(&a.live, a.has(LIVE), &b.live, b.has(LIVE)),
        ended: meet(&a.ended, a.has(ENDED), &b.ended, b.has(ENDED)),
    }
}

fn join_state(a: &State, b: &State) -> State {
    let null: Pts = [Pt::Null].into();
    let mut vars = BTreeMap::new();
    for k in a.vars.keys().chain(b.vars.keys()) {
        let x = a.vars.get(k).unwrap_or(&null);
        let y = b.vars.get(k).unwrap_or(&null);
        vars.insert(*k, x.union(y).copied().collect());
    }
    let mut stores = BTreeMap::new();
    for k in a.stores.keys().chain(b.stores.keys()) {
        stores.insert(*k, join_info(&info_of(a, *k), &info_of(b, *k)));
    }
    let mut slots: BTreeMap<u16, Pts> = a.slots.clone();
    for (w, p) in &b.slots {
        slots.entry(*w).or_default().extend(p.iter().copied());
    }
    State {
        vars,
        stores,
        slots,
    }
}

/// Join two outcomes, where `None` is "no path".
fn join_opt(a: Option<State>, b: Option<State>) -> Option<State> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(a), Some(b)) => Some(join_state(&a, &b)),
    }
}

/// Join the outcomes of two arms: the states, and their values (a store stays correlated
/// with the value when, in each arm, it is either correlated there or not live).
fn join_out(a: Option<(State, Val)>, b: Option<(State, Val)>) -> Option<(State, Val)> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some((sa, va)), Some((sb, vb))) => {
            let corr = va
                .corr
                .union(&vb.corr)
                .copied()
                .filter(|k| {
                    (va.corr.contains(k) || !info_of(&sa, *k).has(LIVE))
                        && (vb.corr.contains(k) || !info_of(&sb, *k).has(LIVE))
                })
                .collect();
            let pts = va.pts.union(&vb.pts).copied().collect();
            Some((join_state(&sa, &sb), Val { pts, corr }))
        }
    }
}

/// How a store comes into being.
#[derive(Clone, Debug)]
enum Origin {
    /// `OpDatabase` into a variable.
    Mint,
    /// `OpDatabase` into the i-th parameter where it is null: a function that fills the buffer
    /// it was handed mints one of its own only when it was handed none.
    MintInto(u16),
    /// A call to a function whose answer is a store of its own.
    Call(u32),
    /// An assignment the code generator lowers as a copy into a store of the variable's
    /// own (`bind_lowering`).
    Copy,
    /// `OpPlaceRecord` with a null host: the record is the root of a fresh store, which
    /// `OpFreeRecordIn` releases whole (`@FR-R-Place`).
    Place,
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
    /// One of several arguments' stores, a bit per argument index (`if c { a } else { b }`, a
    /// recursion that swaps its arguments).
    Args(u32),
    /// The i-th argument's store where the argument names one, a fresh store where it is null:
    /// a function filling the buffer it is handed (`OpDatabase` mints only on null).
    Buf(u16),
    /// Not followed.
    Unknown,
}

impl Summary {
    fn join(self, other: Summary) -> Summary {
        use Summary::{Arg, Args, Bottom, Buf, Fresh, Join, Unknown};
        let bit = |i: u16| if i < 32 { Some(1u32 << i) } else { None };
        // Every rule naming an argument index names ONE index on both sides (or collects them
        // in `Args`): the answer does not depend on the order the returns are met in.
        match (self, other) {
            (Bottom, x) | (x, Bottom) => x,
            (a, b) if a == b => a,
            (Arg(i), Arg(j)) => match (bit(i), bit(j)) {
                (Some(a), Some(b)) => Args(a | b),
                _ => Unknown,
            },
            (Args(m), Arg(i)) | (Arg(i), Args(m)) => bit(i).map_or(Unknown, |b| Args(m | b)),
            (Args(a), Args(b)) => Args(a | b),
            (Buf(i), Arg(j)) | (Arg(j), Buf(i)) if i == j => Buf(i),
            (Fresh, Arg(i) | Buf(i) | Join(i))
            | (Arg(i) | Buf(i) | Join(i), Fresh) => Join(i),
            (Join(i), Arg(j) | Buf(j)) | (Arg(j) | Buf(j), Join(i)) if i == j => Join(i),
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
    distinct: u32,
    fnref_closure: u32,
    get_dbref: u32,
    ref_present: u32,
    place: u32,
    free_in: u32,
    store_live: u32,
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
            distinct: data.def_nr("OpDistinctStore"),
            fnref_closure: data.def_nr("OpFnRefClosure"),
            get_dbref: data.def_nr("OpGetDbRef"),
            ref_present: data.def_nr("OpConvBoolFromRef"),
            place: data.def_nr("OpPlaceRecord"),
            free_in: data.def_nr("OpFreeRecordIn"),
            store_live: data.def_nr("OpStoreLive"),
            not: data.def_nr("OpNot"),
            unconditional: s.unconditional_ref_frees.iter().copied().collect(),
            conditional: s.conditional_ref_frees.iter().copied().collect(),
            text_free: s.text_free,
        }
    }
}

/// The passes a loop may take before it is marked imprecise.
const LOOP_PASSES: usize = 30;

/// A tuple local's MEMBER is followed as a variable of its own: its key is the tuple's
/// variable and the member index packed above every real variable number.  A tuple whose
/// number or width does not fit is not followed (its members read as unknown, as before).
const MEMBER_BASE: u16 = 0x8000;

/// The name an `ended` set keeps for a variable that freed the store and was then reset.
const FREED_NAME: u16 = 0xFFFE;

fn member_key(t: u16, i: u16) -> Option<u16> {
    (t < 0x0800 && i < 16).then_some(MEMBER_BASE | (t << 4) | i)
}

/// The evaluation steps one function may take before it is marked imprecise.
const STEP_BUDGET: usize = 4_000_000;

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
    /// The analysis lost track somewhere (a construct it does not follow, or its budget):
    /// the function draws no findings and answers `Unknown`.
    imprecise: bool,
    steps: usize,
    /// Per enclosing loop: the state that leaves it (`break`) and that starts its next pass
    /// (`continue`).
    loops: Vec<(Option<State>, Option<State>)>,
    /// The parameter variables, in order.
    params: Vec<u16>,
    /// The `Set` nodes (by address) that are their variable's FIRST assignment in emission
    /// order, which is the order the code generator claims slots in.
    first_sets: BTreeSet<usize>,
    /// Variables a first-assignment MOVE made never-free during code generation: the code
    /// generator drops their later `OpFreeRef`.
    moved_away: BTreeSet<u16>,
    /// The tuple local a branch's TAIL literals bind the members of, while its value runs
    /// (`m = match k { 1 => (S { … }, 1), _ => (S { … }, 2) }`); cleared inside call arguments.
    tuple_target: Option<u16>,
    /// Each reassignment's lowering (by `Set` address), decided once per run.
    rebinds: BTreeMap<usize, crate::bind_lowering::ReBind>,
    /// The free nodes after which the code generator resets their variable to null: every
    /// `OpFreeRef` of a variable, and an `OpFreeRefIfDistinct` of a variable an EARLIER
    /// reassignment freed before its build (`owned_reassigned`) or of a lift temporary
    /// (`Codegen::generate_call`, @PLN118, loft#2385).
    resets: BTreeSet<usize>,
    /// Per fn-ref variable, the closure record its build names (`g = { …; FnRef(_, w) }`).
    fn_record: BTreeMap<u16, u16>,
    /// The closure records that leave the frame: the function returns their fn-ref, or the
    /// record is built inside another store (a fn field).  Such a record adopts what it
    /// captures (`@FR-L-CapOwn`); every other record only holds it.
    leaving: BTreeSet<u16>,
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
        if v == FREED_NAME {
            return "a freed name".to_string();
        }
        if v == FREED_NAME - 1 {
            return "the copy's source".to_string();
        }
        if v >= MEMBER_BASE {
            let (t, i) = ((v & !MEMBER_BASE) >> 4, v & 15);
            return format!("{}.{i}", self.data.def(self.d_nr).variables.name(t));
        }
        self.data.def(self.d_nr).variables.name(v).to_string()
    }

    /// The members of tuple local `t` this state follows, with what each names.
    fn members(st: &State, t: u16) -> Vec<(u16, Pts)> {
        let Some(lo) = member_key(t, 0) else {
            return Vec::new();
        };
        st.vars
            .range(lo..=lo | 15)
            .map(|(k, p)| (*k, p.clone()))
            .collect()
    }

    /// The variable a free or a read of `v` names: a variable, or a followed tuple member
    /// (`OpFreeRef(t.1)`).
    fn named_var(st: &State, v: &Value) -> Option<u16> {
        match v.unspan() {
            Value::Var(x) => Some(*x),
            Value::TupleGet(t, i) => member_key(*t, *i).filter(|k| st.vars.contains_key(k)),
            _ => None,
        }
    }

    fn find(&mut self, kind: &'static str, site: u32, detail: String) {
        self.findings.insert(Finding { kind, site, detail });
    }

    fn pts(st: &State, v: u16) -> Pts {
        st.vars
            .get(&v)
            .cloned()
            .unwrap_or_else(|| [Pt::Null].into())
    }

    fn stores_of(p: &Pts) -> Vec<SKey> {
        p.iter()
            .filter_map(|x| if let Pt::Store(k) = x { Some(*k) } else { None })
            .collect()
    }

    /// The value of reading `v`.
    fn read(st: &State, v: u16) -> Val {
        let pts = Self::pts(st, v);
        let single = pts.len() == 1;
        let corr = Self::stores_of(&pts)
            .into_iter()
            .filter(|k| single || info_of(st, *k).live.contains(&v))
            .collect();
        Val { pts, corr }
    }

    /// Bind `v` to `val`.  A live store only `v` named is left unreachable: a leak.
    fn bind(&mut self, st: &mut State, v: u16, val: &Val) {
        for k in Self::stores_of(&Self::pts(st, v)) {
            let info = info_of(st, k);
            if info.has(LIVE)
                && info.live.contains(&v)
                && !st
                    .vars
                    .iter()
                    .any(|(&w, p)| w != v && p.contains(&Pt::Store(k)))
                && !val.pts.contains(&Pt::Store(k))
                && !st.slots.values().any(|p| p.contains(&Pt::Store(k)))
            {
                let name = self.var_name(v);
                self.find(
                    "leak",
                    k.site,
                    format!("`{name}` is rebound while it holds the only reference"),
                );
            }
        }
        for info in st.stores.values_mut() {
            info.live.remove(&v);
            info.ended.remove(&v);
        }
        // A closure record bound anew is a new record: its slots start empty.
        st.slots.remove(&v);
        let single = val.pts.len() == 1;
        for k in Self::stores_of(&val.pts) {
            self.sites[k.site as usize].holders.insert(v);
            if let Some(info) = st.stores.get_mut(&k) {
                if (single || val.corr.contains(&k)) && info.status & LIVE != 0 {
                    info.live.insert(v);
                }
                if single && info.status & ENDED != 0 {
                    info.ended.insert(v);
                }
            }
        }
        if val.pts.len() == 1 && val.pts.contains(&Pt::Null) {
            st.vars.remove(&v);
        } else {
            st.vars.insert(v, val.pts.clone());
        }
    }

    /// A fresh store at `site`: the previous one from this site moves to the older
    /// generation (joined with any already there).
    fn fresh(st: &mut State, site: u32) -> SKey {
        let cur = SKey { site, older: false };
        let old = SKey { site, older: true };
        if let Some(info) = st.stores.remove(&cur) {
            let merged = match st.stores.get(&old) {
                Some(o) => join_info(o, &info),
                None => info,
            };
            st.stores.insert(old, merged);
            for p in st.vars.values_mut() {
                if p.remove(&Pt::Store(cur)) {
                    p.insert(Pt::Store(old));
                }
            }
        }
        st.stores.insert(
            cur,
            SInfo {
                status: LIVE,
                ..Default::default()
            },
        );
        cur
    }

    /// A read of `v` that goes through the store it names.
    fn use_var(&mut self, st: &State, v: u16, how: &str) {
        for k in Self::stores_of(&Self::pts(st, v)) {
            self.sites[k.site as usize].uses.insert(how.to_string());
            let info = info_of(st, k);
            if info.has(ENDED) && info.ended.contains(&v) {
                let name = self.var_name(v);
                self.find(
                    "use-after-end",
                    k.site,
                    format!("`{name}` is read by {how} after its store was freed"),
                );
            }
        }
    }

    /// End the stores `v` names.  `report` is off for a guarded free, whose skip the
    /// analysis may not have seen.
    fn end(&mut self, st: &mut State, v: u16, how: &str, report: bool) {
        self.end_only(st, v, how, report, None);
    }

    /// [`Self::end`], limited to the stores in `only` when it is given.
    fn end_only(
        &mut self,
        st: &mut State,
        v: u16,
        how: &str,
        report: bool,
        only: Option<&BTreeSet<SKey>>,
    ) {
        let pts = Self::pts(st, v);
        let label = format!("{how}({})", self.var_name(v));
        for k in Self::stores_of(&pts) {
            if only.is_some_and(|o| !o.contains(&k)) {
                continue;
            }
            self.sites[k.site as usize].ends.insert(label.clone());
            let info = info_of(st, k);
            // A variable naming only this store or null, where the store is ended on every
            // path: freed again wherever the variable is not null.
            let only_this = pts.iter().all(|p| *p == Pt::Null || *p == Pt::Store(k));
            // …and a DEFINITE name of a store a weak end already released on some path (a
            // maybe-null holder freed it first): freed again on that path.
            let weakly_ended = pts.len() == 1
                && info.has(LIVE)
                && !info.ended.is_empty()
                && !info.ended.contains(&v);
            if report
                && info.has(ENDED)
                && (info.ended.contains(&v) || (only_this && !info.has(LIVE)) || weakly_ended)
            {
                // Every end here is a free, which the runtime refuses on a free slot
                // (`@FR-H-FreeTwice`) and runs on whatever store took the slot since.
                let name = self.var_name(v);
                self.find(
                    "re-free",
                    k.site,
                    format!("`{name}` is freed by {how} after its store was already freed"),
                );
            }
            if report && info.has(HANDED) && pts.len() == 1 && !info.has(LIVE) {
                let name = self.var_name(v);
                self.find(
                    "wrong-end",
                    k.site,
                    format!("`{name}` is freed by {how} after its store was handed on"),
                );
            }
            if !info.has(LIVE) {
                continue;
            }
            let strong = pts.len() == 1 || info.live.contains(&v);
            let mut next = info.clone();
            if strong {
                next.ended = if info.has(ENDED) {
                    info.ended.intersection(&info.live).copied().collect()
                } else {
                    info.live.clone()
                };
                next.status = (info.status & !LIVE) | ENDED;
                next.live.clear();
            } else {
                next.ended = if info.has(ENDED) {
                    info.ended.intersection(&[v].into()).copied().collect()
                } else {
                    [v].into()
                };
                // A store a reset name (maybe) freed stays marked: another maybe-free does not
                // make that one not have happened.
                if info.ended.contains(&FREED_NAME) {
                    next.ended.insert(FREED_NAME);
                }
                next.status |= ENDED;
            }
            st.stores.insert(k, next);
        }
        // A free of a variable that names the caller's store — alone, or that or null.
        // The "or null" reading only for a plain free: a guarded one (`OpFreeRefIfDistinct`)
        // of a buffer that may be the caller's declines where its witness holds it.
        let params: Vec<&Pt> = if how == "OpFreeRef" {
            pts.iter().filter(|p| **p != Pt::Null).collect()
        } else {
            pts.iter().collect()
        };
        if report && params.len() == 1 && how != "OpFreeText" && matches!(params[0], Pt::Param(_)) {
            let name = self.var_name(v);
            self.find(
                "wrong-end",
                u32::MAX,
                format!("`{name}` is freed by {how}, but its store is the caller's"),
            );
        }
    }

    /// Release the stores closure record `w`'s capture slots hold — its cascade
    /// (`<record>_OpDropAll(w)`), or a free of a slot read (`OpFreeRef(OpGetDbRef(w, off))`,
    /// `scopes::rebound_captures`).  A slot store already ended on every path is ended twice.
    fn end_slots(&mut self, st: &mut State, w: u16, how: &str) {
        let held: Vec<SKey> = Self::stores_of(&st.slots.get(&w).cloned().unwrap_or_default());
        let label = format!("{how}({})", self.var_name(w));
        for k in held {
            self.sites[k.site as usize].ends.insert(label.clone());
            let info = info_of(st, k);
            if info.has(ENDED) && !info.has(LIVE) {
                let name = self.var_name(w);
                self.find(
                    if how == "its cascade" { "double-end" } else { "re-free" },
                    k.site,
                    format!("the record `{name}` releases a capture by {how} after its store was already freed"),
                );
            }
            if !info.has(LIVE) {
                continue;
            }
            let mut next = info.clone();
            next.ended.clone_from(&info.live);
            next.status = (info.status & !LIVE) | ENDED;
            next.live.clear();
            st.stores.insert(k, next);
        }
    }

    /// The value leaves this function's hands: returned, or stored where another owner
    /// holds it.
    fn hand_on(st: &mut State, val: &Val) {
        let single = val.pts.len() == 1;
        for k in Self::stores_of(&val.pts) {
            if let Some(info) = st.stores.get_mut(&k)
                && info.status & LIVE != 0
            {
                if single || val.corr.contains(&k) {
                    info.status = (info.status & !LIVE) | HANDED;
                    info.live.clear();
                } else {
                    info.status |= HANDED;
                }
            }
        }
    }

    /// Every store that may still be live at an exit is a leak.
    fn exit(&mut self, st: &State, how: &str) {
        for (k, info) in &st.stores {
            if !info.has(LIVE) {
                continue;
            }
            let holders: Vec<String> = st
                .vars
                .iter()
                .filter(|(_, p)| p.contains(&Pt::Store(*k)))
                .map(|(v, _)| format!("`{}`", self.var_name(*v)))
                .collect();
            let held = if holders.is_empty() {
                "nothing".to_string()
            } else {
                holders.join(", ")
            };
            let f = Finding {
                kind: "leak",
                site: k.site,
                detail: format!("a path reaches {how} with the store live, held by {held}"),
            };
            self.findings.insert(f);
        }
    }

    /// Drop what no later step can observe: a store that cannot be live any more once no
    /// variable names it.
    fn normalize(st: &mut State) {
        let named: BTreeSet<SKey> = st.vars.values().flat_map(Self::stores_of).collect();
        st.stores
            .retain(|k, s| s.status & LIVE != 0 || named.contains(k));
    }

    /// Run `v` from `st`: the state it ends in and its value; `None` when no path falls
    /// through (a `break`, `continue` or `return`).
    #[expect(
        clippy::too_many_lines,
        reason = "one arm per IR shape the records follow"
    )]
    fn eval(&mut self, v: &Value, st: State) -> Option<(State, Val)> {
        self.steps += 1;
        if self.steps > STEP_BUDGET {
            self.imprecise = true;
            return None;
        }
        match v {
            Value::Span(b) => self.eval(&b.1, st),
            Value::Block(b) => self.seq(&b.operators, st),
            Value::Insert(ops) => self.seq(ops, st),
            Value::Loop(b) => self.run_loop(&b.operators, st),
            Value::Var(x) => {
                let mut val = Self::read(&st, *x);
                // A tuple carries its members: handed on, copied or returned, they go with it.
                for (_, p) in Self::members(&st, *x) {
                    val.pts.extend(p.iter().copied().filter(|q| *q != Pt::Null));
                }
                Some((st, val))
            }
            Value::Set(x, val) => self.assign(v, *x, val, st),
            Value::If(cond, then_v, else_v) => {
                let (s, _) = self.eval(cond, st)?;
                let (then_st, else_st) = self.decide(cond, s);
                let then_out = then_st.and_then(|s| self.eval(then_v, s));
                let else_out = else_st.and_then(|s| self.eval(else_v, s));
                let mut out = join_out(then_out, else_out)?;
                Self::normalize(&mut out.0);
                Some(out)
            }
            Value::Return(val) => {
                let (mut s, value) = self.eval(val, st)?;
                if let Value::Var(x) = val.unspan() {
                    self.use_var(&s, *x, "the return");
                }
                self.answer = self.answer.join(self.summary_of(&value.pts));
                Self::hand_on(&mut s, &value);
                self.exit(&s, "a return");
                None
            }
            Value::Break(n) => {
                match self.loops.len().checked_sub(1 + *n as usize) {
                    Some(i) => {
                        let prev = self.loops[i].0.take();
                        self.loops[i].0 = join_opt(prev, Some(st));
                    }
                    None => self.imprecise = true,
                }
                None
            }
            Value::Continue(n) => {
                match self.loops.len().checked_sub(1 + *n as usize) {
                    Some(i) => {
                        let prev = self.loops[i].1.take();
                        self.loops[i].1 = join_opt(prev, Some(st));
                    }
                    None => self.imprecise = true,
                }
                None
            }
            Value::Drop(x) => self.eval(x, st).map(|(s, _)| (s, Val::null())),
            Value::Call(d, args) => self.call(v, *d, args, st),
            Value::CallRef(g, args) => {
                let (s, _) = self.args(args, st)?;
                // A call through a closure reads what its record captured.
                if let Some(w) = self.fn_record.get(g).copied() {
                    let held: Vec<SKey> =
                        Self::stores_of(&s.slots.get(&w).cloned().unwrap_or_default());
                    for k in held {
                        self.sites[k.site as usize]
                            .uses
                            .insert("a closure call".to_string());
                        let info = info_of(&s, k);
                        if info.has(ENDED) && !info.has(LIVE) {
                            let name = self.var_name(*g);
                            self.find(
                                "use-after-end",
                                k.site,
                                format!(
                                    "a call through `{name}` reads a capture whose store was freed"
                                ),
                            );
                        }
                    }
                }
                Some((s, Val::unknown()))
            }
            // A fn-ref holds its closure record.
            Value::FnRef(_, w, _) if *w != u16::MAX => {
                let val = Self::read(&st, *w);
                Some((st, val))
            }
            Value::TuplePut(t, i, x) => {
                let (mut s, value) = self.eval(x, st)?;
                match member_key(*t, *i).filter(|k| s.vars.contains_key(k)) {
                    Some(k) => self.bind(&mut s, k, &value),
                    None => Self::hand_on(&mut s, &value),
                }
                Some((s, Val::null()))
            }
            Value::Tuple(xs) => {
                let target = self.tuple_target;
                let (mut s, vals) = self.args(xs, st)?;
                match target {
                    // A branch arm's tail literal for a tuple local: its members.
                    Some(t) => {
                        for (i, value) in vals.iter().enumerate() {
                            if let Some(k) = u16::try_from(i).ok().and_then(|i| member_key(t, i)) {
                                self.bind(&mut s, k, value);
                            }
                        }
                    }
                    None => {
                        for value in &vals {
                            Self::hand_on(&mut s, value);
                        }
                    }
                }
                Some((s, Val::null()))
            }
            Value::TupleGet(t, i) => match member_key(*t, *i).filter(|k| st.vars.contains_key(k)) {
                Some(k) => {
                    let val = Self::read(&st, k);
                    Some((st, val))
                }
                None => Some((st, Val::unknown())),
            },
            // `(G-Own)` — a yield whose type hands over gives the consumer the stores its value
            // names, and the generator forgets the temps that held them
            // (`coroutine_layout::yield_handed_temps`, the one home both code generators read).
            Value::Yield(y)
                if matches!(self.data.def(self.d_nr).returned().base(),
                    crate::data::Type::Iterator(inner, _)
                        if crate::coroutine_layout::yield_handed_over(inner)) =>
            {
                let (mut s, value) = self.eval(y, st)?;
                Self::hand_on(&mut s, &value);
                for w in crate::coroutine_layout::yield_handed_temps(self.data, self.d_nr, y) {
                    s.vars.remove(&w);
                    for info in s.stores.values_mut() {
                        info.live.remove(&w);
                        info.ended.remove(&w);
                    }
                }
                Some((s, Val::null()))
            }
            Value::Yield(_) | Value::Iter(..) => {
                self.imprecise = true;
                Some((st, Val::unknown()))
            }
            _ => Some((st, Val::null())),
        }
    }

    /// A fresh store at this assignment, the variable's own: a copy the code generator makes.
    fn copied(&mut self, st: &mut State, node: &Value) -> Val {
        let site = self.site(node, Origin::Copy);
        let k = Self::fresh(st, site);
        Val {
            pts: [Pt::Store(k)].into(),
            corr: [k].into(),
        }
    }

    /// The stores a value owns outright (correlated fresh stores): consumed by a copy or an
    /// adopt into the destination's own store.
    fn consume(st: &mut State, value: &Val) {
        for k in &value.corr {
            if let Some(info) = st.stores.get_mut(k)
                && info.status & LIVE != 0
            {
                info.status = (info.status & !LIVE) | HANDED;
                info.live.clear();
            }
        }
    }

    /// `x = val`, as the code generator lowers it: the arm `bind_lowering` decides, with its
    /// store meaning — a bind, a copy into a store of `x`'s own, a copy-or-adopt, a move —
    /// and, for a reassignment, the free of the store `x` displaces.
    fn assign(&mut self, node: &Value, x: u16, val: &Value, st: State) -> Option<(State, Val)> {
        use crate::bind_lowering::FirstArm;
        let data = self.data;
        let function = &data.def(self.d_nr).variables;
        // A tuple literal into a tuple local: each member holds what its element names, and
        // the tuple's own release of a member (`OpFreeRef(t.i)`) ends it.
        // The literal may be the tail of a value block (a call's record unboxed into a stack
        // tuple: `{ __ref_2 = mp(k); (OpGetField(__ref_2, …), …) }`); its leading statements
        // run first.
        let (prefix, lit): (&[Value], &Value) = match val.unspan() {
            Value::Block(b) => match b.operators.split_last() {
                Some((last, init)) => (init, last),
                None => (&[], val),
            },
            Value::Insert(ops) => match ops.split_last() {
                Some((last, init)) => (init, last),
                None => (&[], val),
            },
            _ => (&[], val),
        };
        // A value BRANCH (an `if`, or a `match`'s block ending in its `if` chain) whose tails are
        // tuple literals binds the members on each arm.
        if !matches!(lit.unspan(), Value::Tuple(_))
            && matches!(function.tp(x).base(), crate::data::Type::Tuple(_))
            && member_key(x, 15).is_some()
            && branch_ends_in_tuples(val)
        {
            let prev = self.tuple_target.replace(x);
            let out = self.eval(val, st);
            self.tuple_target = prev;
            return out.map(|(s, _)| (s, Val::null()));
        }
        if let Value::Tuple(xs) = lit.unspan()
            && matches!(function.tp(x).base(), crate::data::Type::Tuple(_))
            && u16::try_from(xs.len())
                .ok()
                .and_then(|n| member_key(x, n.saturating_sub(1)))
                .is_some()
        {
            let st = if prefix.is_empty() {
                st
            } else {
                self.seq(prefix, st)?.0
            };
            let (mut s, vals) = self.args(xs, st)?;
            for (i, value) in vals.iter().enumerate() {
                let k = member_key(x, i as u16).expect("checked above");
                self.bind(&mut s, k, value);
            }
            return Some((s, Val::null()));
        }
        if self
            .first_sets
            .contains(&(std::ptr::from_ref(node) as usize))
        {
            let arm = crate::bind_lowering::first_arm(data, self.d_nr, function, x, val);
            if let FirstArm::RecordVarCopy { src, .. } = arm
                && function.uses(src) == 1
                && function.proxy_says_owned(src)
                && !function.is_argument(src)
                && !function.is_captured(src)
            {
                // The move: `x` takes `src`'s store, and `src`'s free is dropped.
                self.moved_away.insert(src);
                let value = Self::read(&st, src);
                let mut s = st;
                self.bind(&mut s, x, &value);
                return Some((s, Val::null()));
            }
            let (mut s, value) = self.eval(val, st)?;
            let bound = match arm {
                // A fn-ref holds the closure record its build names (`FnRef(_, w)` reads `w`),
                // so its first bind holds that store as a reassignment does.  One read out of a
                // record this function was handed (a cascade's `__held_fn`) is that record's
                // to release, not a store of this frame's.
                FirstArm::FnRef if value.pts.iter().any(|p| matches!(p, Pt::Store(_))) => value,
                FirstArm::FnRef => Val::null(),
                FirstArm::Text
                | FirstArm::RefVar
                | FirstArm::RefNull
                | FirstArm::IterNull
                | FirstArm::NullableVectorNull
                | FirstArm::TupleNull => Val::null(),
                // A keyed local's null init gives it a keyed store of its own, as a vector
                // local's does (`gen_set_first_keyed_null`).
                FirstArm::VectorNull | FirstArm::KeyedNull => {
                    if !function.is_skip_free(x) && function.tp(x).depend().is_empty() {
                        self.copied(&mut s, node)
                    } else {
                        Val::null()
                    }
                }
                FirstArm::RecordCopyCall { .. }
                | FirstArm::RecordVarCopy { .. }
                | FirstArm::ElementCopy { .. }
                | FirstArm::TupleElementCopy { .. } => self.copied(&mut s, node),
                FirstArm::CallCopy { .. } | FirstArm::NullableCallCopy { .. } => {
                    if crate::use_analysis::call_return_frees_source(data, self.d_nr, val) {
                        Self::consume(&mut s, &value);
                        self.unprotected_source_free(&mut s, val, &value);
                    }
                    self.copied(&mut s, node)
                }
                FirstArm::CallRefJoin { .. }
                | FirstArm::NullableCallJoin { .. }
                | FirstArm::CallJoin { .. }
                | FirstArm::OpaqueCallRef { .. } => {
                    Self::consume(&mut s, &value);
                    self.copied(&mut s, node)
                }
                FirstArm::CallAdopt | FirstArm::Plain => value,
            };
            self.bind(&mut s, x, &bound);
            return Some((s, Val::null()));
        }
        self.reassign(node, x, val, st)
    }

    /// `x = val` where `x` already holds a slot: the displaced store's free, then the arm.
    /// The variable the `&` link `x` was installed on (`x = OpCreateStack(t)`), when every
    /// install of `x` names that one variable; `None` for a link to a field or an element, or
    /// one re-pointed between variables.
    fn link_install(&self, x: u16) -> Option<u16> {
        let data = self.data;
        // A link to a store-backed kind: a scalar or text link holds no store to release.
        let crate::data::Type::RefVar(inner) = data.def(self.d_nr).variables.tp(x).base() else {
            return None;
        };
        if !(matches!(
            inner.base(),
            crate::data::Type::Vector(..)
                | crate::data::Type::Reference(..)
                | crate::data::Type::Enum(_, true, _)
        ) || crate::data::is_keyed(inner.base()))
        {
            return None;
        }
        let mut target: Option<u16> = None;
        let mut other = false;
        fn walk(node: &Value, link: u16, data: &Data, target: &mut Option<u16>, other: &mut bool) {
            if let Value::Set(set, rhs) = node
                && *set == link
            {
                match rhs.unspan() {
                    Value::Call(op, args) if data.def(*op).name() == "OpCreateStack" => {
                        match args.first().map(Value::unspan) {
                            Some(Value::Var(var)) if target.is_none_or(|p| p == *var) => {
                                *target = Some(*var);
                            }
                            _ => *other = true,
                        }
                    }
                    // A re-point to another place (`p = &v[i]`, `p = &o.f`, `p = q`).
                    Value::Call(op, _)
                        if matches!(
                            data.def(*op).name(),
                            "OpVarRef" | "OpGetField" | "OpGetVector" | "OpVectorRef"
                        ) =>
                    {
                        *other = true;
                    }
                    _ => {}
                }
            }
            node.for_each_child(&mut |c| walk(c, link, data, target, other));
        }
        walk(&data.def(self.d_nr).code, x, data, &mut target, &mut other);
        if other { None } else { target }
    }

    /// A whole write `x = val` THROUGH the link `x` to the variable `t` (`@FR-B-Ref-Write`):
    /// the value lands in `t`, and the code generator releases the store `t` displaced unless
    /// the new value lives in it — the slot move's write-back (`OpMoveSlotRef`) or the
    /// stash / install / `OpFreeRefIfDistinct` — but never through a link to a plain parameter
    /// (`Function::link_write_releases`, `@FR-F-ParamRebind`).  A read of `x` in `val` reads
    /// `t`.  A child read (`x = &x.next`) stays in `t`'s store when the child is present and
    /// leaves `t` null when it is absent, so the release is the absent path's alone.
    fn link_write(&mut self, x: u16, t: u16, val: &Value, st: State) -> Option<(State, Val)> {
        let data = self.data;
        let function = &data.def(self.d_nr).variables;
        let crate::data::Type::RefVar(inner) = function.tp(x).base() else {
            return None;
        };
        let owned = matches!(
            crate::use_analysis::ownership_of(data, self.d_nr, val),
            crate::use_analysis::Own::Owned
        );
        // The code generator's own test (`state/codegen.rs`'s `amp_displaces`): an owned value
        // of a store-backed kind, or any record.  What a `&` parameter displaces is the
        // caller's to give up (`@FR-F-ParamRef`), which no store here records; a plain
        // parameter is not released at all, unless the cure is switched off.
        let store_kind = matches!(
            inner.base(),
            crate::data::Type::Vector(..)
                | crate::data::Type::Reference(..)
                | crate::data::Type::Enum(_, true, _)
        ) || crate::data::is_keyed(inner.base());
        let releases = ((store_kind && owned) || inner.heap_def_nr().is_some())
            && function.link_write_releases(x)
            && !(function.is_argument(t)
                && matches!(function.tp(t).base(), crate::data::Type::RefVar(_)));
        let mut st = st;
        let old = Self::pts(&st, t);
        st.vars.insert(x, old.clone());
        let (mut s, value) = self.eval(val, st)?;
        let child_read = matches!(val.unspan(),
            Value::Call(d, _) if data.def(*d).name() == "OpRefFromChildRec");
        // The link reads what its target holds, so both are bound to the written value.
        let s = if !releases {
            self.bind(&mut s, t, &value);
            self.bind(&mut s, x, &value);
            s
        } else if child_read {
            let mut present = s.clone();
            self.bind(&mut present, t, &value);
            self.bind(&mut present, x, &value);
            let mut absent = s;
            self.end(&mut absent, t, "the link write-back", true);
            self.bind(&mut absent, t, &Val::null());
            self.bind(&mut absent, x, &Val::null());
            join_state(&present, &absent)
        } else {
            // Released only where the new value is not in it: the stores the value names stay.
            let gone: BTreeSet<SKey> = Self::stores_of(&old)
                .into_iter()
                .filter(|k| !value.pts.contains(&Pt::Store(*k)))
                .collect();
            if !old.iter().all(|p| value.pts.contains(p)) {
                self.end_only(&mut s, t, "the link write-back", true, Some(&gone));
            }
            self.bind(&mut s, t, &value);
            self.bind(&mut s, x, &value);
            s
        };
        Some((s, Val::null()))
    }

    fn reassign(&mut self, node: &Value, x: u16, val: &Value, st: State) -> Option<(State, Val)> {
        use crate::bind_lowering::{DisplacedFree, ReArm};
        let data = self.data;
        let function = &data.def(self.d_nr).variables;
        if let Some(t) = self.link_install(x)
            && !matches!(val.unspan(), Value::Call(d, _)
                if matches!(data.def(*d).name(), "OpCreateStack" | "OpVarRef"))
        {
            return self.link_write(x, t, val, st);
        }
        // A whole write through a `&` PARAMETER (`d = OrdDoc.parse(t)`) lands in the caller's
        // place: the caller holds what it names from here on.
        if function.is_argument(x) && matches!(function.tp(x).base(), crate::data::Type::RefVar(_))
        {
            let (mut s, value) = self.eval(val, st)?;
            Self::hand_on(&mut s, &value);
            return Some((s, Val::null()));
        }
        let rb = match self.rebinds.get(&(std::ptr::from_ref(node) as usize)) {
            Some(rb) => *rb,
            None => crate::bind_lowering::re_bind(data, self.d_nr, function, x, val),
        };
        let mut st = st;
        // A keyed local assigned null is cleared IN PLACE (`B0`): the code generator emits no
        // displaced free for it (`Codegen::generate_set_inner` returns at that arm).
        if let DisplacedFree::Pre { witness } = rb.free
            && rb.arm != ReArm::KeyedNull
        {
            let keep = witness
                .into_iter()
                .chain(rb.entry_w)
                .any(|w| {
                    // The runtime keeps the store when the witness names it.  A null `x` frees
                    // nothing, so the null `x` may also hold (a loop's first pass) is no reason
                    // to free: the two are compared without it.
                    let held: Pts = Self::pts(&st, x).into_iter().filter(|p| *p != Pt::Null).collect();
                    let named: Pts = Self::pts(&st, w).into_iter().filter(|p| *p != Pt::Null).collect();
                    !held.is_empty() && held == named
                });
            if !keep {
                self.end(&mut st, x, "the displaced free", true);
            }
        }
        let old = Self::pts(&st, x);
        let (mut s, value) = self.eval(val, st)?;
        let bound = match rb.arm {
            ReArm::KeyedNull => Self::read(&s, x),
            ReArm::Join { .. } | ReArm::OpaqueCallRef { .. } => {
                Self::consume(&mut s, &value);
                self.copied(&mut s, node)
            }
            ReArm::CallCopy { .. } => {
                if crate::use_analysis::call_return_frees_source(data, self.d_nr, val) {
                    Self::consume(&mut s, &value);
                    if rb.free != DisplacedFree::Stash {
                        self.unprotected_source_free(&mut s, val, &value);
                    }
                    // The copy's source-free releases what the call answered, and a call that
                    // reads `x` may answer `x`'s own store.  Unprotected (`ReBind::protects_old`),
                    // that store is then released by the copy AND by the stashed free after it.
                    let x_arg = crate::use_analysis::protectable_ref_args(data, self.d_nr, val)
                        .0
                        .contains(&x);
                    if rb.free == DisplacedFree::Stash
                        && rb.copies_into_fresh()
                        && rb.rhs_reads_v
                        && x_arg
                        && !rb.protects_old(x_arg)
                        && let Some(k) = Self::stores_of(&old).first()
                    {
                        let name = self.var_name(x);
                        self.find(
                            "re-free",
                            k.site,
                            format!(
                                "`{name}`'s old store is released by the copy's source-free and \
                                 again by the stashed free"
                            ),
                        );
                    }
                }
                // A stashed local re-initialises its own store in place after the call unless
                // the copy lands in a fresh store (`ReBind::copies_into_fresh`, the code
                // generator's own answer); anything else starts from a fresh one.
                let in_place = rb.free == DisplacedFree::Stash && !rb.copies_into_fresh();
                if in_place && Self::stores_of(&old).len() == 1 && old.len() == 1 {
                    Val {
                        pts: old.clone(),
                        corr: Self::stores_of(&old).into_iter().collect(),
                    }
                } else {
                    self.copied(&mut s, node)
                }
            }
            ReArm::VarCopy { .. } | ReArm::ElementCopy { .. } => self.copied(&mut s, node),
            ReArm::Plain => value,
        };
        if rb.free == DisplacedFree::Stash && rb.arm != ReArm::KeyedNull && bound.pts != old {
            // The stashed old reference, freed unless the new value names the same store.
            let tmp = u16::MAX;
            s.vars.insert(tmp, old.clone());
            for k in Self::stores_of(&old) {
                if let Some(info) = s.stores.get_mut(&k)
                    && info.live.contains(&x)
                {
                    info.live.insert(tmp);
                }
            }
            self.bind(&mut s, x, &bound);
            if !Self::stores_of(&old)
                .iter()
                .any(|k| bound.pts.contains(&Pt::Store(*k)))
            {
                self.end(&mut s, tmp, "the stashed free", false);
            }
            s.vars.remove(&tmp);
            for info in s.stores.values_mut() {
                info.live.remove(&tmp);
                info.ended.remove(&tmp);
            }
            return Some((s, Val::null()));
        }
        self.bind(&mut s, x, &bound);
        Some((s, Val::null()))
    }

    /// A copy's source-free (`COPY_FREE_SOURCE`) releases what the call answered unless the
    /// `@P290` bracket protects it: a store of the answer that no protectable argument names
    /// (`use_analysis::protectable_ref_args`, the code generator's own list) ends here.
    fn unprotected_source_free(&mut self, st: &mut State, call: &Value, value: &Val) {
        let protected: BTreeSet<SKey> =
            crate::use_analysis::protectable_ref_args(self.data, self.d_nr, call)
                .0
                .into_iter()
                .flat_map(|a| Self::stores_of(&Self::pts(st, a)))
                .collect();
        let freed: BTreeSet<SKey> = Self::stores_of(&value.pts)
            .into_iter()
            .filter(|k| !protected.contains(k) && !value.corr.contains(k))
            .filter(|k| info_of(st, *k).has(LIVE))
            .collect();
        if freed.is_empty() {
            return;
        }
        st.vars.insert(FREED_NAME - 1, value.pts.clone());
        self.end_only(
            st,
            FREED_NAME - 1,
            "the copy's source-free",
            true,
            Some(&freed),
        );
        st.vars.remove(&(FREED_NAME - 1));
    }

    /// The states the two arms of an `if` on `cond` start from, as (then, else); `None` is
    /// an arm the test rules out.  A test the records cannot read keeps both arms with the
    /// state unchanged, which is sound: narrowing only ever removes paths a test excludes.
    fn decide(&self, cond: &Value, st: State) -> (Option<State>, Option<State>) {
        let both = |st: State| (Some(st.clone()), Some(st));
        match cond.unspan() {
            Value::Boolean(true) => (Some(st), None),
            Value::Boolean(false) => (None, Some(st)),
            Value::Call(op, a) if *op == self.ops.not && a.len() == 1 => {
                let (t, f) = self.decide(&a[0], st);
                (f, t)
            }
            // A condition that is itself a choice: `a && b` and `a || b` lower to this.
            Value::If(c, t, e) => {
                let (ct, cf) = self.decide(c, st);
                let (tt, tf) = ct.map_or((None, None), |s| self.decide(t, s));
                let (et, ef) = cf.map_or((None, None), |s| self.decide(e, s));
                (join_opt(tt, et), join_opt(tf, ef))
            }
            Value::Call(op, a) if *op == self.ops.distinct => self.decide_distinct(a, st),
            Value::Call(op, a) if a.len() == 1 => {
                let Value::Var(x) = a[0].unspan() else {
                    return both(st);
                };
                if *op == self.ops.ref_is_null {
                    Self::null_test(*x, st)
                } else if *op == self.ops.ref_present {
                    // `OpConvBoolFromRef(x)`: true when `x` names a record.  A freed store
                    // still names one, so this is the null test turned round, never a
                    // liveness test.
                    let (null, present) = Self::null_test(*x, st);
                    (present, null)
                } else if *op == self.ops.store_live {
                    Self::live_test(*x, st)
                } else {
                    both(st)
                }
            }
            _ => both(st),
        }
    }

    /// `OpRefIsNull(x)`, as (null arm, store arm).  Where `x` is null, a store `x` points
    /// into on every path where that store is live is not live, and one it points into on
    /// every path where it is ended is not ended.
    fn null_test(x: u16, st: State) -> (Option<State>, Option<State>) {
        let pts = Self::pts(&st, x);
        let may_null = pts
            .iter()
            .any(|p| matches!(p, Pt::Null | Pt::Param(_) | Pt::Unknown));
        let may_store = pts.iter().any(|p| !matches!(p, Pt::Null));
        let null_arm = may_null.then(|| {
            let mut s = st.clone();
            for k in Self::stores_of(&pts) {
                if let Some(info) = s.stores.get_mut(&k) {
                    if info.live.contains(&x) {
                        info.status &= !LIVE;
                        info.live.clear();
                    }
                    if info.ended.contains(&x) {
                        info.status &= !ENDED;
                        info.ended.clear();
                    }
                    if info.status == 0 {
                        info.status = ABSENT;
                    }
                }
            }
            s.vars.remove(&x);
            s
        });
        let store_arm = may_store.then(|| {
            let mut s = st;
            if let Some(p) = s.vars.get_mut(&x) {
                p.remove(&Pt::Null);
            }
            s
        });
        (null_arm, store_arm)
    }

    /// `OpStoreLive(x)` (`@FR-L-CapOwn`'s guard): is the store `x` names still allocated?
    /// A store handed on keeps both answers: its new holder may or may not have freed it.
    fn live_test(x: u16, st: State) -> (Option<State>, Option<State>) {
        let pts = Self::pts(&st, x);
        let followed = !pts.iter().any(|p| matches!(p, Pt::Param(_) | Pt::Unknown));
        let stores: Vec<SKey> = Self::stores_of(&pts).into_iter().collect();
        // Live: `x` names a store, and that store is live or handed on.
        let live_arm = {
            let mut s = st.clone();
            if let Some(p) = s.vars.get_mut(&x) {
                p.remove(&Pt::Null);
            }
            let mut possible = !followed || !stores.is_empty();
            for k in &stores {
                if let Some(info) = s.stores.get_mut(k) {
                    let only = followed && stores.len() == 1;
                    if only || info.ended.contains(&x) {
                        info.status &= !(ENDED | ABSENT);
                        info.ended.clear();
                    }
                    if only && info.status == 0 {
                        possible = false;
                    }
                }
            }
            possible.then_some(s)
        };
        // Not live: `x` is null, or the store it names has ended (or was never made).
        let dead_arm = {
            let mut s = st;
            let mut possible = !followed || pts.contains(&Pt::Null) || stores.is_empty();
            for k in &stores {
                if let Some(info) = s.stores.get_mut(k) {
                    if info.live.contains(&x) {
                        info.status &= !LIVE;
                        info.live.clear();
                    }
                    if info.status & !LIVE != 0 {
                        possible = true;
                    }
                    if info.status == 0 {
                        info.status = ABSENT;
                    }
                } else {
                    possible = true;
                }
            }
            possible.then_some(s)
        };
        (live_arm, dead_arm)
    }

    /// `OpDistinctStore(a, b)`: where both operands name one single store (or both are
    /// null), the two are that store on every path and the DISTINCT arm cannot run; where
    /// they name two different single stores, the SAME arm cannot.  Anything wider keeps
    /// both arms.  An operand may be a variable, a closure's record (`OpFnRefClosure`) or a
    /// capture slot read out of a record that stays in this frame (`OpGetDbRef`).
    fn decide_distinct(&self, args: &[Value], st: State) -> (Option<State>, Option<State>) {
        let operand = |v: &Value| -> Option<Pts> {
            match v.unspan() {
                Value::Var(x) => Some(Self::pts(&st, *x)),
                Value::TupleGet(..) => Self::named_var(&st, v).map(|k| Self::pts(&st, k)),
                Value::Call(op, a) if *op == self.ops.fnref_closure => {
                    match a.first().map(Value::unspan) {
                        Some(Value::Var(g)) => Some(Self::pts(&st, *g)),
                        _ => None,
                    }
                }
                Value::Call(op, a) if *op == self.ops.get_dbref => {
                    match a.first().map(Value::unspan) {
                        Some(Value::Var(w)) => st.slots.get(w).filter(|p| !p.is_empty()).cloned(),
                        _ => None,
                    }
                }
                _ => None,
            }
        };
        let (Some(a), Some(b)) = (
            args.first().and_then(operand),
            args.get(1).and_then(operand),
        ) else {
            return (Some(st.clone()), Some(st));
        };
        let single = |p: &Pts| match p.iter().next() {
            Some(q @ (Pt::Store(_) | Pt::Null)) if p.len() == 1 => Some(*q),
            _ => None,
        };
        match (single(&a), single(&b)) {
            (Some(x), Some(y)) if x == y => (None, Some(st)),
            (Some(_), Some(_)) => (Some(st), None),
            _ => (Some(st.clone()), Some(st)),
        }
    }

    fn seq(&mut self, ops: &[Value], st: State) -> Option<(State, Val)> {
        let mut cur = (st, Val::null());
        for op in ops {
            cur = self.eval(op, cur.0)?;
        }
        Some(cur)
    }

    fn run_loop(&mut self, body: &[Value], st: State) -> Option<(State, Val)> {
        let mut head = st;
        let mut exits: Option<State> = None;
        for pass in 0..LOOP_PASSES {
            self.loops.push((None, None));
            let out = self.seq(body, head.clone());
            let (brk, cont) = self.loops.pop().unwrap_or_default();
            exits = join_opt(exits, brk);
            let again = join_opt(join_opt(Some(head.clone()), out.map(|(s, _)| s)), cont)
                .expect("the head state is a path");
            let mut again = again;
            Self::normalize(&mut again);
            if again == head || self.imprecise {
                break;
            }
            if pass + 1 == LOOP_PASSES {
                self.imprecise = true;
            }
            head = again;
        }
        exits.map(|s| (s, Val::null()))
    }

    /// Evaluate call arguments left to right.  A bare variable argument is a use of its
    /// store.
    fn args(&mut self, args: &[Value], st: State) -> Option<(State, Vec<Val>)> {
        let target = self.tuple_target.take();
        let out = self.args_inner(args, st);
        self.tuple_target = target;
        out
    }

    fn args_inner(&mut self, args: &[Value], st: State) -> Option<(State, Vec<Val>)> {
        let mut s = st;
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            let (s2, value) = self.eval(a, s)?;
            if let Value::Var(x) = a.unspan() {
                self.use_var(&s2, *x, "a call");
            }
            vals.push(value);
            s = s2;
        }
        Some((s, vals))
    }

    fn summary_of(&self, pts: &Pts) -> Summary {
        let mut out = Summary::Bottom;
        for p in pts {
            let one = match p {
                Pt::Null => Summary::Bottom,
                Pt::Store(k) => match self.sites[k.site as usize].origin {
                    Some(Origin::MintInto(i)) => Summary::Buf(i),
                    _ => Summary::Fresh,
                },
                Pt::Param(v) => self
                    .params
                    .iter()
                    .position(|x| x == v)
                    .map_or(Summary::Unknown, |i| Summary::Arg(i as u16)),
                Pt::Unknown => Summary::Unknown,
            };
            out = out.join(one);
        }
        out
    }

    fn first_var(args: &[Value], i: usize) -> Option<u16> {
        args.get(i).and_then(|a| {
            if let Value::Var(x) = a.unspan() {
                Some(*x)
            } else {
                None
            }
        })
    }

    /// The variable a value block hands back as its tail (`{#Object __ref_p2_N = …;
    /// __ref_p2_N}`): a copy that frees such a source releases that variable's store.
    fn tail_var(v: &Value) -> Option<u16> {
        match v.unspan() {
            Value::Var(x) => Some(*x),
            Value::Block(bl) => bl.operators.last().and_then(Self::tail_var),
            Value::Insert(ops) => ops.last().and_then(Self::tail_var),
            _ => None,
        }
    }

    /// A free op: its own argument is not a use.
    fn free_op(&mut self, d: u32, args: &[Value], st: State) -> State {
        let ops = self.ops;
        if ops.unconditional.contains(&d) || d == ops.text_free {
            let name = self.data.def(d).name().to_string();
            let mut st = st;
            if let Some(Value::Call(g, gargs)) = args.first().map(Value::unspan)
                && *g == ops.get_dbref
                && let Some(w) = Self::first_var(gargs, 0)
                && st.slots.contains_key(&w)
            {
                self.end_slots(&mut st, w, &name);
                return st;
            }
            if d == ops.free_in {
                // `OpFreeRecordIn` releases one record: a store of its own only where the
                // record is that store's root (a placed record whose host was null); a
                // record placed inside a host leaves the host's store live.
                if let Some(x) = Self::first_var(args, 0) {
                    let placed: BTreeSet<SKey> = Self::stores_of(&Self::pts(&st, x))
                        .into_iter()
                        .filter(|k| {
                            matches!(self.sites[k.site as usize].origin, Some(Origin::Place))
                        })
                        .collect();
                    if !placed.is_empty() {
                        self.end_only(&mut st, x, &name, true, Some(&placed));
                    }
                }
                return st;
            }
            if let Some(x) = args.first().and_then(|a| Self::named_var(&st, a)) {
                // The code generator emits no `OpFreeRef` of a never-free variable
                // (`Codegen::generate_call`, S34), including one a move made never-free.
                let dropped = name == "OpFreeRef"
                    && x < MEMBER_BASE
                    && (self.data.def(self.d_nr).variables.is_skip_free(x)
                        || self.moved_away.contains(&x));
                if !dropped {
                    self.end(&mut st, x, &name, true);
                }
            }
            return st;
        }
        if ops.conditional.contains(&d) {
            // A free that is a no-op where the freed variable and its witness name one store.
            let name = self.data.def(d).name().to_string();
            let mut st = st;
            if let Some(x) = Self::first_var(args, 0) {
                let w = Self::first_var(args, 1);
                let xp = Self::pts(&st, x);
                let wp = w.map(|w| Self::pts(&st, w)).unwrap_or_default();
                let shared: Vec<SKey> = Self::stores_of(&xp)
                    .into_iter()
                    .filter(|k| wp.contains(&Pt::Store(*k)))
                    .collect();
                if shared.is_empty() {
                    self.end(&mut st, x, &name, true);
                } else if xp.len() == 1 && wp.len() == 1 {
                    let label =
                        format!("{name}({}) skipped: its witness holds it", self.var_name(x));
                    self.sites[shared[0].site as usize].ends.insert(label);
                } else {
                    // Skipped on the paths where the witness names the store: a free of the
                    // rest, which is where the witness holds something else.
                    let witness_holds_it = shared
                        .iter()
                        .all(|k| w.is_some_and(|w| info_of(&st, *k).live.contains(&w)));
                    if !witness_holds_it {
                        self.end(&mut st, x, &name, false);
                    }
                }
            }
            return st;
        }
        st
    }

    /// `OpPlaceRecord(host, tp)`: a record inside the host's store, or the root of a fresh
    /// store where the host is null (`@FR-R-Place`).
    fn place(&mut self, node: &Value, host: &Val, mut s: State) -> (State, Val) {
        let mut pts: Pts = host
            .pts
            .iter()
            .copied()
            .filter(|p| *p != Pt::Null)
            .collect();
        let mut corr = BTreeSet::new();
        if host.pts.contains(&Pt::Null) {
            let site = self.site(node, Origin::Place);
            let k = Self::fresh(&mut s, site);
            if !pts.is_empty()
                && let Some(info) = s.stores.get_mut(&k)
            {
                info.status = LIVE | ABSENT;
            }
            pts.insert(Pt::Store(k));
            corr.insert(k);
        }
        (s, Val { pts, corr })
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one branch per op family the records give a meaning to"
    )]
    fn call(&mut self, node: &Value, d: u32, args: &[Value], st: State) -> Option<(State, Val)> {
        let ops = self.ops;
        if ops.unconditional.contains(&d) || d == ops.text_free || ops.conditional.contains(&d) {
            let mut st = self.free_op(d, args, st);
            if self.resets.contains(&(std::ptr::from_ref(node) as usize))
                && let Some(x) = Self::first_var(args, 0)
            {
                // The free's variable is reset to null right after it.  Where it had (maybe)
                // ended a store, the store keeps that fact under a placeholder name, so a
                // later free of the same store by another name still reads as a second one.
                for info in st.stores.values_mut() {
                    info.live.remove(&x);
                    if info.ended.remove(&x) {
                        info.ended.insert(FREED_NAME);
                    }
                }
                st.vars.remove(&x);
            }
            return Some((st, Val::null()));
        }
        if d == ops.database
            && let Some(x) = Self::first_var(args, 0)
        {
            // `OpDatabase` re-inits a LIVE store in place and mints only where the variable is
            // null (or names a freed store): a callee filling its hidden buffer answers the
            // caller's store, not one of its own (`state/io.rs::database`).
            let cur = Self::pts(&st, x);
            let kept: Pts = cur
                .iter()
                .copied()
                .filter(|p| match p {
                    Pt::Param(_) => true,
                    Pt::Store(k) => info_of(&st, *k).has(LIVE) && !info_of(&st, *k).has(ENDED),
                    _ => false,
                })
                .collect();
            let mut st = st;
            let val = if !kept.is_empty() && kept.len() == cur.len() {
                Val {
                    corr: Self::stores_of(&kept).into_iter().collect(),
                    pts: kept,
                }
            } else if let Some(own) = {
                // A loop's head joins the null of the first pass with the store an earlier
                // pass made here: on the null path this `OpDatabase` makes that same store
                // (later passes find it live and re-init it in place), not another generation.
                let site = self.site_of.get(&(std::ptr::from_ref(node) as usize)).copied();
                site.and_then(|site| {
                    kept.iter().find_map(|p| match p {
                        Pt::Store(k) if k.site == site && !k.older => Some(*k),
                        _ => None,
                    })
                })
            } {
                Val {
                    pts: kept,
                    corr: [own].into(),
                }
            } else {
                let origin = self
                    .params
                    .iter()
                    .position(|p| *p == x)
                    .map_or(Origin::Mint, |i| Origin::MintInto(i as u16));
                let site = self.site(node, origin);
                let k = Self::fresh(&mut st, site);
                let mut pts = kept;
                pts.insert(Pt::Store(k));
                Val {
                    pts,
                    corr: [k].into(),
                }
            };
            if val.pts != cur {
                self.bind(&mut st, x, &val);
            }
            return Some((st, Val::null()));
        }
        let (mut s, vals) = self.args(args, st)?;
        // A copy that frees its source store (`COPY_FREE_SOURCE`, `OpSets::freed_source`):
        // the source's stores end here, except one the destination names, whose release
        // the runtime declines.
        if let Some((src, dst)) = self.data.op_sets().freed_source(d, args)
            && let Some(x) =
                Self::first_var(args, src).or_else(|| args.get(src).and_then(Self::tail_var))
        {
            let kept: BTreeSet<SKey> = vals
                .get(dst)
                .map(|v| Self::stores_of(&v.pts).into_iter().collect())
                .unwrap_or_default();
            let freed: BTreeSet<SKey> = Self::stores_of(&Self::pts(&s, x))
                .into_iter()
                .filter(|k| !kept.contains(k))
                .collect();
            if !freed.is_empty() {
                let name = format!("{} (its source)", self.data.def(d).name());
                self.end_only(&mut s, x, &name, true, Some(&freed));
            }
        }
        let def = self.data.def(d);
        if def.name().ends_with("_OpDropAll")
            && let Some(w) = Self::first_var(args, 0)
            && s.slots.contains_key(&w)
        {
            self.end_slots(&mut s, w, "its cascade");
        }
        if d == ops.set_dbref
            && let Some(value) = vals.get(2)
        {
            match Self::first_var(args, 0) {
                // A capture into a closure record that stays in this frame: the record holds
                // the store for as long as it lives, and the frame still releases it.
                Some(w)
                    if self
                        .data
                        .def(self.d_nr)
                        .variables
                        .name(w)
                        .starts_with("___clos_")
                        && !self.leaving.contains(&w) =>
                {
                    s.slots.entry(w).or_default().extend(
                        value
                            .pts
                            .iter()
                            .copied()
                            .filter(|p| matches!(p, Pt::Store(_))),
                    );
                }
                // A reference written into another store's field: that store holds it now.
                _ => Self::hand_on(&mut s, value),
            }
        }
        if def.returned().heap_dep().is_none() {
            return Some((s, Val::null()));
        }
        let bodied = !matches!(def.code.unspan(), Value::Null) && !def.name().starts_with("Op");
        if !bodied {
            // An operator answering a reference answers one into its first argument's store;
            // a native's answer is not followed.
            if d == ops.place {
                let host = vals.into_iter().next().unwrap_or_else(Val::unknown);
                return Some(self.place(node, &host, s));
            }
            let val = if def.name() == "OpNullRefSentinel" {
                // The null reference: no store.  Read as an operator answering into its
                // (absent) first argument it was unknown, and every `-> S?` callee whose null
                // arm spells it answered Unknown instead of Fresh.
                Val::null()
            } else if def.name().starts_with("Op")
                && def
                    .attributes
                    .first()
                    .is_some_and(|a| matches!(a.typedef, crate::data::Type::Text(_)))
            {
                // …unless that argument holds no store to answer into: a parse
                // (`OpCastVectorFromText(text, tp)`) builds its answer in a store of its own.
                let site = self.site(node, Origin::Call(d));
                let k = Self::fresh(&mut s, site);
                Val {
                    pts: [Pt::Store(k)].into(),
                    corr: [k].into(),
                }
            } else if def.name().starts_with("Op") {
                vals.into_iter().next().unwrap_or_else(Val::unknown)
            } else {
                Val::unknown()
            };
            return Some((s, val));
        }
        let val = match self.summaries.get(&d).copied().unwrap_or(Summary::Unknown) {
            Summary::Bottom => Val::null(),
            Summary::Unknown => Val::unknown(),
            Summary::Fresh => {
                let site = self.site(node, Origin::Call(d));
                let k = Self::fresh(&mut s, site);
                Val {
                    pts: [Pt::Store(k)].into(),
                    corr: [k].into(),
                }
            }
            // The argument's store, BORROWED: the answer owns none of it, so a copy or a join
            // guard that consumes what its value owns leaves it with the argument's owner.
            Summary::Arg(i) => vals.get(i as usize).map_or_else(Val::unknown, |v| Val {
                pts: v.pts.clone(),
                corr: BTreeSet::new(),
            }),
            // A join over an argument that is NULL here is the callee's own mint: a callee
            // that answers its hidden buffer or a store it made, handed a null buffer, makes one
            // (`OpDatabase` on null).
            // One of several arguments' stores, borrowed: the answer owns none of them.
            Summary::Args(m) => {
                let mut pts = Pts::new();
                for (i, v) in vals.iter().enumerate() {
                    if i < 32 && m & (1u32 << i) != 0 {
                        pts.extend(v.pts.iter().copied());
                    }
                }
                Val {
                    pts,
                    corr: BTreeSet::new(),
                }
            }
            Summary::Join(i) | Summary::Buf(i)
                if vals
                    .get(i as usize)
                    .is_some_and(|v| v.pts.iter().all(|p| *p == Pt::Null)) =>
            {
                let site = self.site(node, Origin::Call(d));
                let k = Self::fresh(&mut s, site);
                Val {
                    pts: [Pt::Store(k)].into(),
                    corr: [k].into(),
                }
            }
            // A buffer-filling callee handed a store answers that store, borrowed, as `Arg`.
            Summary::Buf(i)
                if vals.get(i as usize).is_some_and(|v| {
                    !v.pts.is_empty() && v.pts.iter().all(|p| matches!(p, Pt::Store(_) | Pt::Param(_)))
                }) =>
            {
                let v = &vals[i as usize];
                Val {
                    pts: v.pts.clone(),
                    corr: BTreeSet::new(),
                }
            }
            Summary::Join(i) | Summary::Buf(i) => {
                let viewed = vals.get(i as usize).cloned().unwrap_or_else(Val::unknown);
                let site = self.site(node, Origin::Call(d));
                let k = Self::fresh(&mut s, site);
                if let Some(info) = s.stores.get_mut(&k) {
                    info.status = LIVE | ABSENT;
                }
                let mut pts = viewed.pts;
                pts.insert(Pt::Store(k));
                Val {
                    pts,
                    corr: [k].into(),
                }
            }
        };
        Some((s, val))
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
    let mut st = State::default();
    for &p in &params {
        st.vars.insert(p, [Pt::Param(p)].into());
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
        steps: 0,
        loops: Vec::new(),
        params,
        first_sets: BTreeSet::new(),
        moved_away: BTreeSet::new(),
        tuple_target: None,
        rebinds: BTreeMap::new(),
        resets: BTreeSet::new(),
        fn_record: BTreeMap::new(),
        leaving: BTreeSet::new(),
    };
    let body = def.code.clone();
    // One pass in emission order: which assignments are first, each reassignment's lowering,
    // and the frees the code generator follows with a reset to null.
    let mut seen: BTreeSet<u16> = run.params.iter().copied().collect();
    let mut owned_reassigned: BTreeSet<u16> = BTreeSet::new();
    let unconditional = data.def_nr("OpFreeRef");
    let if_distinct = data.def_nr("OpFreeRefIfDistinct");
    let vars = &def.variables;
    body.walk(&mut |n| {
        let at = std::ptr::from_ref(n) as usize;
        match n {
            Value::Set(x, val) => {
                if seen.insert(*x) {
                    run.first_sets.insert(at);
                } else {
                    let rb = crate::bind_lowering::re_bind(data, d_nr, vars, *x, val);
                    if matches!(rb.free, crate::bind_lowering::DisplacedFree::Pre { .. }) {
                        owned_reassigned.insert(*x);
                    }
                    run.rebinds.insert(at, rb);
                }
            }
            // The interpreter resets after EVERY `OpFreeRef` of a variable unless
            // `LOFT_NO_FREE_RESET` (`Codegen::generate_call`, the twin of this test).
            Value::Call(op, args)
                if (*op == unconditional || *op == if_distinct)
                    && let Some(Value::Var(x)) = args.first().map(Value::unspan)
                    && (owned_reassigned.contains(x)
                        || vars.is_lift_temp(*x)
                        || (*op == unconditional && !crate::keys::free_reset_off())) =>
            {
                run.resets.insert(at);
            }
            _ => {}
        }
    });
    closure_records(data, vars, &body, &mut run.fn_record, &mut run.leaving);
    if let Some((mut s, value)) = run.eval(&body, st) {
        run.answer = run.answer.join(run.summary_of(&value.pts));
        Run::hand_on(&mut s, &value);
        run.exit(&s, "the end of the function");
    }
    run
}

/// The closure records a body builds: per fn-ref variable the record its build names, and the
/// records that leave the frame — a returned fn-ref's, or one built inside another store
/// (`w = OpChildRec(…)`, a fn field), whose host decides its life.
fn closure_records(
    data: &Data,
    vars: &crate::variables::Function,
    body: &Value,
    fn_record: &mut BTreeMap<u16, u16>,
    leaving: &mut BTreeSet<u16>,
) {
    let child_rec = data.def_nr("OpChildRec");
    let mut returned: BTreeSet<u16> = BTreeSet::new();
    body.walk(&mut |n| match n {
        Value::Set(g, rhs) => {
            if vars.name(*g).starts_with("___clos_")
                && matches!(rhs.unspan(), Value::Call(d, _) if *d == child_rec)
            {
                leaving.insert(*g);
            }
            rhs.walk(&mut |m| {
                if let Value::FnRef(_, w, _) = m
                    && *w != u16::MAX
                {
                    fn_record.insert(*g, *w);
                }
            });
        }
        Value::Return(v) => {
            if let Value::Var(g) = v.unspan() {
                returned.insert(*g);
            }
        }
        _ => {}
    });
    if let Value::Block(bl) = body.unspan()
        && let Some(Value::Var(g)) = bl.operators.last().map(Value::unspan)
    {
        returned.insert(*g);
    }
    for g in returned {
        if let Some(&w) = fn_record.get(&g) {
            leaving.insert(w);
        }
    }
}

fn is_analysed(data: &Data, d: u32) -> bool {
    let def = data.def(d);
    matches!(def.def_type, DefType::Function)
        && !matches!(def.code.unspan(), Value::Null)
        && !def.name().starts_with("Op")
}

/// The analysed functions `d`'s body calls.
fn callees(data: &Data, d: u32) -> Vec<u32> {
    let mut out = BTreeSet::new();
    data.def(d).code.walk(&mut |v| {
        if let Value::Call(c, _) = v
            && is_analysed(data, *c)
        {
            out.insert(*c);
        }
    });
    out.into_iter().collect()
}

/// The strongly connected components of the call graph, callees before callers (Tarjan's
/// order), so each function is analysed after every function it calls outside its own cycle.
fn components(data: &Data, fns: &[u32]) -> Vec<Vec<u32>> {
    let edges: BTreeMap<u32, Vec<u32>> = fns.iter().map(|&d| (d, callees(data, d))).collect();
    let mut index: BTreeMap<u32, (usize, usize)> = BTreeMap::new();
    let mut on_stack: BTreeSet<u32> = BTreeSet::new();
    let mut stack: Vec<u32> = Vec::new();
    let mut out: Vec<Vec<u32>> = Vec::new();
    let mut next = 0usize;
    for &root in fns {
        if index.contains_key(&root) {
            continue;
        }
        // An explicit stack of (node, next edge to visit): a deep call chain must not
        // overflow the Rust stack.
        let mut work: Vec<(u32, usize)> = vec![(root, 0)];
        index.insert(root, (next, next));
        next += 1;
        stack.push(root);
        on_stack.insert(root);
        while let Some(&mut (v, ref mut i)) = work.last_mut() {
            let succ = &edges[&v];
            if *i < succ.len() {
                let w = succ[*i];
                *i += 1;
                if let Some(&(wi, _)) = index.get(&w) {
                    if on_stack.contains(&w) {
                        let e = index.get_mut(&v).expect("indexed");
                        e.1 = e.1.min(wi);
                    }
                } else {
                    index.insert(w, (next, next));
                    next += 1;
                    stack.push(w);
                    on_stack.insert(w);
                    work.push((w, 0));
                }
                continue;
            }
            work.pop();
            let (vi, vl) = index[&v];
            if let Some(&(p, _)) = work.last() {
                let pl = index[&p].1.min(vl);
                index.get_mut(&p).expect("indexed").1 = pl;
            }
            if vi == vl {
                let mut comp = Vec::new();
                while let Some(w) = stack.pop() {
                    on_stack.remove(&w);
                    comp.push(w);
                    if w == v {
                        break;
                    }
                }
                out.push(comp);
            }
        }
    }
    out
}

/// Every function's answer and its final run: the least fixpoint over the call graph, taken
/// component by component, callees first, so only a recursive component iterates.
fn analyse_all(
    data: &Data,
    ops: &Ops,
    sum: &mut BTreeMap<u32, Summary>,
) -> BTreeMap<u32, (Vec<String>, bool)> {
    let fns: Vec<u32> = (0..data.definitions())
        .filter(|&d| is_analysed(data, d))
        .collect();
    for &d in &fns {
        sum.insert(d, Summary::Bottom);
    }
    let profile = std::env::var_os("LOFT_STORE_PROFILE").is_some();
    let t_all = std::time::Instant::now();
    let mut slow: Vec<(u128, u32)> = Vec::new();
    let mut runs = 0usize;
    let mut found: BTreeMap<u32, (Vec<String>, bool)> = BTreeMap::new();
    for comp in components(data, &fns) {
        for _ in 0..8 {
            let mut changed = false;
            for &d in &comp {
                let t = std::time::Instant::now();
                if profile && std::env::var_os("LOFT_STORE_PROFILE_EACH").is_some() {
                    eprintln!("[stores] start {}", data.def(d).name());
                }
                let (answer, imprecise, lines) = {
                    let run = run_fn(data, ops, sum, d);
                    let lines: Vec<String> = run
                        .findings
                        .iter()
                        .map(|f| {
                            format!("store-{} {}{}", f.kind, site_label(&run, f.site), f.detail)
                        })
                        .collect();
                    (run.answer, run.imprecise, lines)
                };
                runs += 1;
                if profile {
                    let us = t.elapsed().as_micros();
                    if us > 200_000 {
                        eprintln!(
                            "[stores] slow: {} took {} ms",
                            data.def(d).name(),
                            us / 1000
                        );
                    }
                    slow.push((us, d));
                }
                let new = if imprecise {
                    Summary::Unknown
                } else {
                    sum[&d].join(answer)
                };
                if new != sum[&d] {
                    sum.insert(d, new);
                    changed = true;
                }
                found.insert(d, (lines, imprecise));
            }
            if !changed {
                break;
            }
        }
    }
    if profile {
        slow.sort_unstable_by(|a, b| b.cmp(a));
        let top: Vec<String> = slow
            .iter()
            .take(8)
            .map(|(us, d)| format!("{}={}ms", data.def(*d).name(), us / 1000))
            .collect();
        eprintln!(
            "[stores] {} fns, {runs} runs, {} ms; slowest {}",
            fns.len(),
            t_all.elapsed().as_millis(),
            top.join(" ")
        );
    }
    found
}

/// The observer entry point, at the end of `close_program`.
pub fn observe(data: &Data) {
    let table = std::env::var("LOFT_STORE_TABLE").ok();
    let check = std::env::var("LOFT_OWN_ORACLE").is_ok_and(|m| m == "check-records");
    if table.is_none() && !check {
        return;
    }
    let ops = Ops::new(data);
    let mut sum = BTreeMap::new();
    let found = analyse_all(data, &ops, &mut sum);
    let mut reds = 0usize;
    for d in (0..data.definitions()).filter(|&d| is_analysed(data, d)) {
        let name = data.def(d).name().to_string();
        let shown = table
            .as_deref()
            .is_some_and(|t| !t.is_empty() && name.contains(t));
        if !shown && !check {
            continue;
        }
        if shown {
            let run = run_fn(data, &ops, &sum, d);
            print_records(&run, &name, sum[&d]);
        }
        if check && let Some((lines, false)) = found.get(&d) {
            for l in lines {
                eprintln!("RED {name}: {l}");
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
        Some(Origin::Mint | Origin::MintInto(_)) => format!("[store #{site}, minted] "),
        Some(Origin::Copy) => format!("[store #{site}, copied] "),
        Some(Origin::Place) => format!("[store #{site}, placed] "),
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

/// Does `v` end — through its blocks' tails and both arms of every `if` — in tuple literals
/// only, with at least one branch on the way?
fn branch_ends_in_tuples(v: &Value) -> bool {
    fn tails(v: &Value, branched: bool) -> Option<bool> {
        match v.unspan() {
            Value::If(_, a, b) => Some(tails(a, true)? && tails(b, true)?),
            Value::Block(bl) => tails(bl.operators.last()?, branched),
            Value::Tuple(_) => Some(branched),
            _ => None,
        }
    }
    tails(v, false).unwrap_or(false)
}
