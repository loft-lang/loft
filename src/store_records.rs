// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I60 — Scope & dependency/lifetime tracker (deps)

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
//!   * `LOFT_STORE_TRACE=<fn substring>` — print the state after each of their statements;
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
use std::fmt::Write as _;

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
    /// Alternatives to `live` (empty: none known): name sets such that on every path where
    /// the store is live, every name of at least one set points into it.  A join of paths
    /// holding it under different names keeps each side's holders (a two-buffer rotation,
    /// `__ref_2 ← __ref_3; __ref_3 ← v`), so freeing each of those names still ends it.
    alts: Vec<BTreeSet<u16>>,
    /// The variables that point into it on EVERY path where it is ended.
    ended: BTreeSet<u16>,
    /// The variables whose value, naming this store, was handed on (returned, captured by a
    /// closure that leaves) and that have not been rebound since.  Where one of them is also
    /// in `live`, the store is handed on wherever it is live.
    handed_by: BTreeSet<u16>,
    /// Closure-record variables of which at least one names a record on every path where
    /// it is live (one record per arm of a branch; the frame's "free the capture where no
    /// record was built" leaves it live only there).  A test finding one of them null drops
    /// it; the last one dropped finds this store not live.  Empty: nothing known.
    live_some: BTreeSet<u16>,
    /// Closure-record variables null on every path where it is live (a record built only
    /// where the capture went into it).  A test finding one of them naming a record then
    /// finds this store not live.
    live_null: BTreeSet<u16>,
    /// The closure-record variables a capture put it into on some path: the candidates for
    /// `live_some` and `live_null`.
    captured_by: BTreeSet<u16>,
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
    /// Per closure record STORE, per capture offset: what the slot holds.  A record holds
    /// its captures from the capture on, and hands them on only when a value naming the
    /// record leaves (`hand_on`): which record owns a capture is a per-run fact
    /// (`@FR-L-CapOne`), and the code generator settles it at each return by emptying the
    /// slots of the records that stay.  Keyed by the store, as the returned record's
    /// variable is nulled before the return.  A slot keeps the captured value whole: its
    /// `corr` stores are live only where the slot names them, as for a variable's read.
    slots: BTreeMap<SKey, BTreeMap<i32, Val>>,
    /// Per record store and offset, the variable a capture was read from, while that
    /// variable still names it: handing the record on hands the capture on through it, as a
    /// capture handed on in place would (`handed_by`).
    slot_src: BTreeMap<SKey, BTreeMap<i32, u16>>,
    /// Per record built inside another store (a fn field): what its capture slots name.
    /// Such a record takes its captures over at the capture; a guarded free reads a slot as
    /// its witness (`OpFreeRefIfDistinct(v, OpGetDbRef(___clos_1, 0))`).
    field_captures: BTreeMap<u16, Pts>,
    /// Boolean locals whose value this path knows: an ownership flag (`__rbo_<buf>`, a
    /// `__lbo_` displacement flag) written a constant, and read by the guard of a free.
    flags: BTreeMap<u16, bool>,
    /// Per holder: the stores a reference written into its fields names but does not own
    /// (`@FR-H-RefHome`).  The holder handed to a call reads them.
    points: BTreeMap<u16, Pts>,
    /// Variables known to name a record on this path whatever their store: bound from
    /// `OpChildRec` (a field's present child, or a fresh one), until rebound.
    nonnull: BTreeSet<u16>,
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

/// `LOFT_NO_SLOT_HOLDERS=1` (records only): a closure record holding a capture in a slot is no
/// holder of it, as before.
fn slot_holders_off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| std::env::var_os("LOFT_NO_SLOT_HOLDERS").is_some())
}

/// On a path where `x` names no store: every holder set naming `x` (a variable, or a closure
/// record whose slot holds the store) does not hold here.
fn rule_out_everywhere(st: &mut State, x: u16) {
    for info in st.stores.values_mut() {
        if info.live.contains(&x) || info.alts.iter().any(|a| a.contains(&x)) {
            rule_out_holder(info, x);
            if info.status == 0 {
                info.status = ABSENT;
            }
        }
    }
}

/// The holders one side of a join knows: its alternatives, or its must-holders as one.
fn holder_alts(x: &SInfo) -> Option<Vec<BTreeSet<u16>>> {
    if !x.alts.is_empty() {
        Some(x.alts.clone())
    } else if x.live.is_empty() {
        None
    } else {
        Some(vec![x.live.clone()])
    }
}

/// Alternatives kept minimal: a set holding another adds nothing, one equal to the
/// must-holders says nothing beyond them, and too many are dropped.
fn normal_alts(mut alts: Vec<BTreeSet<u16>>, live: &BTreeSet<u16>) -> Vec<BTreeSet<u16>> {
    alts.sort();
    alts.dedup();
    let keep: Vec<BTreeSet<u16>> = alts
        .iter()
        .filter(|a| !alts.iter().any(|b| b != *a && b.is_subset(a)))
        .cloned()
        .collect();
    if keep.len() < 2 || keep.len() > 6 || keep.iter().any(|a| a == live) {
        Vec::new()
    } else {
        keep
    }
}

/// `v` names the store no more: no longer a must-holder, nor in any alternative (one left
/// empty leaves nothing known).
fn forget_holder(info: &mut SInfo, v: u16) {
    info.live.remove(&v);
    for a in &mut info.alts {
        a.remove(&v);
    }
    if info.alts.iter().any(BTreeSet::is_empty) {
        info.alts.clear();
    }
}

/// On a path where `x` does not name the store: where `x` must-holds it, or every
/// alternative names `x`, it is not live here; otherwise the alternatives naming `x` drop.
fn rule_out_holder(info: &mut SInfo, x: u16) {
    if !info.has(LIVE) {
        return;
    }
    if info.live.contains(&x) {
        info.status &= !LIVE;
        info.live.clear();
        info.alts.clear();
        return;
    }
    if info.alts.is_empty() {
        return;
    }
    info.alts.retain(|a| !a.contains(&x));
    if info.alts.is_empty() {
        info.status &= !LIVE;
        info.live.clear();
        info.alts.clear();
    } else {
        info.live = info.alts.iter().skip(1).fold(info.alts[0].clone(), |m, a| {
            m.intersection(a).copied().collect()
        });
        let live = info.live.clone();
        info.alts = normal_alts(std::mem::take(&mut info.alts), &live);
    }
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
        alts: match (a.has(LIVE), b.has(LIVE)) {
            (true, true) => match (holder_alts(a), holder_alts(b)) {
                (Some(x), Some(y)) => normal_alts(
                    x.into_iter().chain(y).collect(),
                    &meet(&a.live, true, &b.live, true),
                ),
                _ => Vec::new(),
            },
            (true, false) => a.alts.clone(),
            (false, true) => b.alts.clone(),
            (false, false) => Vec::new(),
        },
        ended: meet(&a.ended, a.has(ENDED), &b.ended, b.has(ENDED)),
        handed_by: meet(&a.handed_by, a.has(LIVE), &b.handed_by, b.has(LIVE)),
        live_some: BTreeSet::new(),
        live_null: BTreeSet::new(),
        captured_by: a.captured_by.union(&b.captured_by).copied().collect(),
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
        let (ia, ib) = (info_of(a, *k), info_of(b, *k));
        let mut info = join_info(&ia, &ib);
        if info.has(LIVE) {
            // Per side: what it knows, or the records it holds non-null; `None` where the
            // store is not live there.
            let side = |st: &State, i: &SInfo| -> Option<BTreeSet<u16>> {
                if !i.has(LIVE) {
                    None
                } else if !i.live_some.is_empty() {
                    Some(i.live_some.clone())
                } else {
                    Some(
                        info.captured_by
                            .iter()
                            .copied()
                            .filter(|w| st.vars.get(w).is_some_and(|p| !p.contains(&Pt::Null)))
                            .collect(),
                    )
                }
            };
            // Per side where it is live: the records null there.
            let nulls = |st: &State, i: &SInfo| -> BTreeSet<u16> {
                let mut n = i.live_null.clone();
                n.extend(
                    info.captured_by
                        .iter()
                        .copied()
                        .filter(|w| !st.vars.contains_key(w)),
                );
                n
            };
            info.live_null = match (ia.has(LIVE), ib.has(LIVE)) {
                (true, true) => nulls(a, &ia)
                    .intersection(&nulls(b, &ib))
                    .copied()
                    .collect(),
                (true, false) => nulls(a, &ia),
                (false, true) => nulls(b, &ib),
                (false, false) => BTreeSet::new(),
            };
            info.live_some = match (side(a, &ia), side(b, &ib)) {
                (Some(x), Some(y)) if !x.is_empty() && !y.is_empty() => {
                    x.union(&y).copied().collect()
                }
                (Some(x), None) | (None, Some(x)) => x,
                _ => BTreeSet::new(),
            };
        }
        stores.insert(*k, info);
    }
    let flags = a
        .flags
        .iter()
        .filter(|(k, v)| b.flags.get(k) == Some(v))
        .map(|(k, v)| (*k, *v))
        .collect();
    let (slots, slot_src, field_captures) = join_slots(a, b);
    let mut points: BTreeMap<u16, Pts> = a.points.clone();
    for (w, p) in &b.points {
        points.entry(*w).or_default().extend(p.iter().copied());
    }
    State {
        vars,
        stores,
        slots,
        slot_src,
        field_captures,
        flags,
        points,
        nonnull: a.nonnull.intersection(&b.nonnull).copied().collect(),
    }
}

/// The closure records' part of a state: slots, their source variables, fn-field captures.
type SlotParts = (
    BTreeMap<SKey, BTreeMap<i32, Val>>,
    BTreeMap<SKey, BTreeMap<i32, u16>>,
    BTreeMap<u16, Pts>,
);

/// The closure records' part of a join: the slots, their source variables (kept where both
/// sides agree) and the fn-field captures.
fn join_slots(a: &State, b: &State) -> SlotParts {
    let mut slots = a.slots.clone();
    for (k, m) in &b.slots {
        let Some(into) = slots.get_mut(k) else {
            slots.insert(*k, m.clone());
            continue;
        };
        for (off, v) in m {
            match into.get_mut(off) {
                Some(x) => {
                    x.pts.extend(v.pts.iter().copied());
                    x.corr = x.corr.intersection(&v.corr).copied().collect();
                }
                None => {
                    into.insert(*off, v.clone());
                }
            }
        }
    }
    let mut field_captures = a.field_captures.clone();
    for (w, p) in &b.field_captures {
        field_captures
            .entry(*w)
            .or_default()
            .extend(p.iter().copied());
    }
    let mut slot_src = a.slot_src.clone();
    for (k, m) in &b.slot_src {
        let into = slot_src.entry(*k).or_default();
        for (off, x) in m {
            match into.get(off) {
                Some(y) if y != x => {
                    into.remove(off);
                }
                _ => {
                    into.insert(*off, *x);
                }
            }
        }
    }
    (slots, slot_src, field_captures)
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
            (Fresh, Arg(i) | Buf(i) | Join(i)) | (Arg(i) | Buf(i) | Join(i), Fresh) => Join(i),
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
    vector_is_null: u32,
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
            vector_is_null: data.def_nr("OpVectorIsNull"),
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

/// The name a reassignment parks the store it displaces under while the new value is built.
const OLD_NAME: u16 = 0xFFFC;

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
    /// `LOFT_STORE_TRACE` names this function: print the state after each statement.
    traced: bool,
    /// Inside a `fn_ref_field_reset` or `fn_ref_field_set` block: a fn field is being written.
    in_field_reset: u32,
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
    /// The guarded frees of one variable against another, after which the code generator resets
    /// the variable where the free ran (`Codegen::generate_call`, `@FR-O-NoDiverge`).
    cond_resets: BTreeSet<usize>,
    /// The `OpDropFnRef(f)` nodes inside the emitter's stand-down guard
    /// (`if OpDistinctStore(OpFnRefClosure(f), …)`, `get_free_vars::fnref_stand_down`): the
    /// fn-ref's own release, which runs only for a closure no local record releases, so its
    /// cascade is never the one a local record's captures end by.
    standdown_drops: BTreeSet<usize>,
    /// The closure-record variables (`___clos_N`) the body releases itself (`OpFreeRef`): only
    /// while one of those names a record is the record's own release its cascade.
    released_records: BTreeSet<u16>,
    /// Per fn-ref variable, the closure record its build names (`g = { …; FnRef(_, w) }`).
    fn_record: BTreeMap<u16, u16>,
    /// The closure records that leave the frame: the function returns their fn-ref, or the
    /// record is built inside another store (a fn field).  Such a record adopts what it
    /// captures (`@FR-L-CapOwn`); every other record only holds it.
    leaving: BTreeSet<u16>,
    /// The variables inside an IR `n_protect_store_frees` bracket: a source-free
    /// in it declines the stores they name.
    protected: Vec<u16>,
    /// The compiler temporaries the function frees with a plain `OpFreeRef` (a lift, a literal's
    /// work-ref): each still owns the record it holds, wherever a reference to it is written.
    freed_temps: BTreeSet<u16>,
    /// A tuple literal met as a MEMBER of an enclosing tuple's value (the tail of a
    /// `tuple_member_move` block): its stores are that member's, not handed on.
    member_tuple: bool,
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
                && !st
                    .slots
                    .values()
                    .any(|m| m.values().any(|v| v.pts.contains(&Pt::Store(k))))
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
            forget_holder(info, v);
            info.ended.remove(&v);
            info.handed_by.remove(&v);
            if info.live_some.contains(&v) {
                info.live_some.clear();
            }
            info.live_null.remove(&v);
        }
        st.field_captures.remove(&v);
        st.nonnull.remove(&v);
        for m in st.slot_src.values_mut() {
            m.retain(|_, x| *x != v);
        }
        st.points.remove(&v);
        let single = val.pts.len() == 1;
        for k in Self::stores_of(&val.pts) {
            self.sites[k.site as usize].holders.insert(v);
            if let Some(info) = st.stores.get_mut(&k) {
                if (single || val.corr.contains(&k)) && info.status & LIVE != 0 {
                    info.live.insert(v);
                    for a in &mut info.alts {
                        a.insert(v);
                    }
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
            // A record minted again is a new record: its slots start empty, and the
            // previous generation's move with it.
            for m in st.slots.values_mut() {
                for v in m.values_mut() {
                    if v.pts.remove(&Pt::Store(cur)) {
                        v.pts.insert(Pt::Store(old));
                    }
                    v.corr.remove(&cur);
                }
            }
            if let Some(m) = st.slots.remove(&cur) {
                let into = st.slots.entry(old).or_default();
                for (off, v) in m {
                    let x = into.entry(off).or_default();
                    x.pts.extend(v.pts);
                    x.corr.clear();
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

    /// A holder read by a call reads the records its stored references name: one freed on
    /// every path is read after its end (`@FR-H-RefHome`).
    fn use_points(&mut self, st: &State, h: u16) {
        for k in Self::stores_of(&st.points.get(&h).cloned().unwrap_or_default()) {
            let info = info_of(st, k);
            if info.has(ENDED) && !info.has(LIVE) {
                let name = self.var_name(h);
                self.find(
                    "use-after-end",
                    k.site,
                    format!(
                        "`{name}` is read by a call after a record its reference names was freed"
                    ),
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
            // The alternatives naming `v` are the paths this free ends it on.
            let rest_alts: Vec<BTreeSet<u16>> = info
                .alts
                .iter()
                .filter(|a| !a.contains(&v))
                .cloned()
                .collect();
            let strong = pts.len() == 1
                || info.live.contains(&v)
                || (!info.alts.is_empty() && rest_alts.is_empty());
            let mut next = info.clone();
            if strong {
                next.ended = if info.has(ENDED) {
                    info.ended.intersection(&info.live).copied().collect()
                } else {
                    info.live.clone()
                };
                next.status = (info.status & !LIVE) | ENDED;
                next.live.clear();
                next.alts.clear();
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
                if rest_alts.len() < info.alts.len() {
                    next.live = rest_alts.iter().skip(1).fold(rest_alts[0].clone(), |m, a| {
                        m.intersection(a).copied().collect()
                    });
                    let live = next.live.clone();
                    next.alts = normal_alts(rest_alts, &live);
                }
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

    /// Does `w` name a closure record that holds a capture?
    fn is_record(st: &State, w: u16) -> bool {
        Self::stores_of(&Self::pts(st, w))
            .iter()
            .any(|k| st.slots.contains_key(k))
    }

    /// What the capture slots of the record(s) `w` names hold: the slot at `off`, or every
    /// slot.  Stores only.
    fn held_by_record(st: &State, w: u16, off: Option<i32>) -> Pts {
        let mut out: Pts = st
            .field_captures
            .get(&w)
            .map(|p| {
                p.iter()
                    .copied()
                    .filter(|q| matches!(q, Pt::Store(_)))
                    .collect()
            })
            .unwrap_or_default();
        for k in Self::stores_of(&Self::pts(st, w)) {
            if let Some(m) = st.slots.get(&k) {
                for (o, v) in m {
                    if off.is_none_or(|x| x == *o) {
                        out.extend(v.pts.iter().copied().filter(|q| matches!(q, Pt::Store(_))));
                    }
                }
            }
        }
        out
    }

    /// What a guarded free's witness names, and whether it is a capture slot of a record
    /// built inside another store (a fn field): under such a slot a shared store is never
    /// left live — where the slot names it the record took it over, everywhere else the
    /// free ends it.  A slot of any closure record answers as the witness.
    fn guard_witness(&self, st: &State, w: Option<u16>, arg: Option<&Value>) -> (Pts, bool) {
        match (w, arg.map(Value::unspan)) {
            (Some(w), _) => (Self::pts(st, w), false),
            (None, Some(Value::Call(g, gargs))) if *g == self.ops.get_dbref => {
                let r = Self::first_var(gargs, 0);
                let field = r.is_some_and(|r| st.field_captures.contains_key(&r));
                let pts = r
                    .map(|r| Self::held_by_record(st, r, Self::const_offset(gargs.get(1))))
                    .unwrap_or_default();
                (pts, field)
            }
            _ => (Pts::default(), false),
        }
    }

    /// A constant slot offset (`OpGetDbRef(w, 8i32)`).
    fn const_offset(v: Option<&Value>) -> Option<i32> {
        match v.map(Value::unspan) {
            Some(Value::Int(n)) => Some(*n),
            _ => None,
        }
    }

    /// The capture `OpSetDbRef(w, off, value)`: the slot of each record `w` names holds the
    /// value's stores from here on — replacing what it held where `w` names one record and
    /// the offset is known (a null empties it: the code generator's give-up of a capture
    /// before a return, `@FR-L-CapOne`).
    fn hold(&self, st: &mut State, w: u16, off: Option<i32>, value: &Val, src: Option<u16>) {
        let recs = Self::stores_of(&Self::pts(st, w));
        let strong = recs.len() == 1 && off.is_some();
        let pts: Pts = value
            .pts
            .iter()
            .copied()
            .filter(|p| matches!(p, Pt::Store(_)))
            .collect();
        let held = Val {
            corr: value
                .corr
                .iter()
                .copied()
                .filter(|k| pts.contains(&Pt::Store(*k)))
                .collect(),
            pts,
        };
        for k in &held.pts {
            if let Pt::Store(k) = k
                && let Some(info) = st.stores.get_mut(k)
            {
                info.captured_by.insert(w);
            }
        }
        for k in recs {
            let at = off.unwrap_or(i32::MIN);
            match src.filter(|_| strong) {
                Some(x) => {
                    st.slot_src.entry(k).or_default().insert(at, x);
                }
                None => {
                    if let Some(m) = st.slot_src.get_mut(&k) {
                        m.remove(&at);
                    }
                }
            }
            if strong {
                // The slot no longer holds what it held: `w` is no holder of that any more.
                let before: Vec<SKey> = st
                    .slots
                    .get(&k)
                    .and_then(|m| m.get(&at))
                    .map(|v| Self::stores_of(&v.pts))
                    .unwrap_or_default();
                for old in before {
                    if !held.pts.contains(&Pt::Store(old))
                        && let Some(info) = st.stores.get_mut(&old)
                    {
                        forget_holder(info, w);
                    }
                }
                // A store the slot of a closure record now holds is held by that record wherever
                // it is live: a path on which `w` names no record does not hold it there.
                // Only an OWNER slot (`OpSetDbRef(w, off, __own_x)`, the capture hand-over)
                // takes its store over; a view capture beside the store's backing does not.
                let owner_src = src.is_some_and(|x| {
                    self.data
                        .def(self.d_nr)
                        .variables
                        .name(x)
                        .starts_with("__own_")
                });
                if self.is_closure_record(w)
                    && owner_src
                    && let [only] = Self::stores_of(&held.pts)[..]
                    && let Some(info) = st.stores.get_mut(&only)
                    && info.has(LIVE)
                    && !slot_holders_off()
                {
                    info.live.insert(w);
                    for a in &mut info.alts {
                        a.insert(w);
                    }
                }
            }
            let slot = st.slots.entry(k).or_default().entry(at).or_default();
            if strong {
                slot.clone_from(&held);
            } else {
                slot.pts.extend(held.pts.iter().copied());
                slot.corr = slot.corr.intersection(&held.corr).copied().collect();
            }
        }
    }

    /// The capture slots the closure record minted at `k`'s site releases when a fn-ref's drop
    /// runs its cascade: the offsets its type's live `_OpDropAll` frees
    /// (`OpFreeRef(OpGetDbRef(self, off))`).  `None` where the site minted no closure variable
    /// or its types disagree; an empty set for a retired cascade (`Data::retire_def_name`: an
    /// earlier build owns the capture), which releases nothing.
    fn cascade_frees(&self, k: SKey) -> Option<BTreeSet<i32>> {
        let function = &self.data.def(self.d_nr).variables;
        let mut out: Option<BTreeSet<i32>> = None;
        for &v in &self.sites[k.site as usize].holders {
            if v >= function.count() || !function.name(v).starts_with("___clos_") {
                continue;
            }
            let crate::data::Type::Reference(d, _) = function.tp(v).base() else {
                continue;
            };
            let name = self.data.def(*d).name();
            let drop = self
                .data
                .def_nr(&format!("t_{}{name}_OpDropAll", name.len()));
            let mut offs = BTreeSet::new();
            if drop != u32::MAX {
                let (free, get) = (
                    self.data.def_nr("OpFreeRef"),
                    self.data.def_nr("OpGetDbRef"),
                );
                self.data.def(drop).code.walk(&mut |n| {
                    if let Value::Call(op, a) = n.unspan()
                        && *op == free
                        && let [inner] = a.as_slice()
                        && let Value::Call(g, b) = inner.unspan()
                        && *g == get
                        && let [_, off] = b.as_slice()
                        && let Value::Int(o) = off.unspan()
                    {
                        offs.insert(*o);
                    }
                });
            }
            match &out {
                Some(o) if *o != offs => return None,
                _ => out = Some(offs),
            }
        }
        out
    }

    /// End the stores the slots `offs` of closure record `k` hold — a fn-ref's drop cascade
    /// (`OpDropFnRef`), named after the fn-ref `name`.  A slot store already ended on every
    /// path is ended twice.
    fn end_record_slots(&mut self, st: &mut State, k: SKey, offs: &BTreeSet<i32>, name: &str) {
        let held: Vec<SKey> = st
            .slots
            .get(&k)
            .map(|m| {
                m.iter()
                    .filter(|(o, _)| offs.contains(o))
                    .flat_map(|(_, v)| Self::stores_of(&v.pts))
                    .collect()
            })
            .unwrap_or_default();
        let label = format!("its cascade({name})");
        for h in held {
            self.sites[h.site as usize].ends.insert(label.clone());
            let info = info_of(st, h);
            if info.has(ENDED) && !info.has(LIVE) {
                self.find(
                    "double-end",
                    h.site,
                    format!("the record `{name}` releases a capture by its cascade after its store was already freed"),
                );
            }
            if !info.has(LIVE) {
                continue;
            }
            let mut next = info.clone();
            next.ended.clone_from(&info.live);
            next.status = (info.status & !LIVE) | ENDED;
            next.live.clear();
            next.alts.clear();
            st.stores.insert(h, next);
        }
    }

    /// Release the stores closure record `w`'s capture slots hold — its cascade
    /// (`<record>_OpDropAll(w)`), or a free of a slot read (`OpFreeRef(OpGetDbRef(w, off))`,
    /// `scopes::rebound_captures`).  A slot store already ended on every path is ended twice.
    fn end_slots(&mut self, st: &mut State, w: u16, how: &str) {
        let mut held: Vec<SKey> = Self::stores_of(&Self::held_by_record(st, w, None));
        // The cascade drops a closure record a slot holds (a captured fn-ref's record) with
        // what ITS slots hold.
        if how == "its cascade" {
            let mut i = 0;
            while i < held.len() {
                if let Some(m) = st.slots.get(&held[i]) {
                    for v in m.values() {
                        for k in Self::stores_of(&v.pts) {
                            if !held.contains(&k) {
                                held.push(k);
                            }
                        }
                    }
                }
                i += 1;
            }
        }
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
            next.alts.clear();
            st.stores.insert(k, next);
        }
    }

    /// The value leaves this function's hands: returned, or stored where another owner
    /// holds it.
    /// [`Self::hand_on`] of the value variable `x` holds: the stores it names also record
    /// that `x`'s value went on (`handed_by`).
    fn hand_on_var(st: &mut State, x: Option<u16>, val: &Val) {
        Self::hand_on(st, val);
        let Some(x) = x else {
            return;
        };
        for k in Self::stores_of(&Self::pts(st, x)) {
            if let Some(info) = st.stores.get_mut(&k)
                && info.has(LIVE)
            {
                info.handed_by.insert(x);
            }
        }
    }

    fn hand_on(st: &mut State, val: &Val) {
        let single = val.pts.len() == 1;
        let mut records = Vec::new();
        for k in Self::stores_of(&val.pts) {
            if st.slots.contains_key(&k) {
                records.push(k);
            }
            if let Some(info) = st.stores.get_mut(&k)
                && info.status & LIVE != 0
            {
                if single || val.corr.contains(&k) {
                    info.status = (info.status & !LIVE) | HANDED;
                    info.live.clear();
                    info.alts.clear();
                } else {
                    info.status |= HANDED;
                }
            }
        }
        // A closure record that leaves takes what its slots hold with it (`@FR-L-CapOne`).
        for k in records {
            if let Some(m) = st.slots.remove(&k) {
                let src = st.slot_src.remove(&k).unwrap_or_default();
                for (off, v) in m {
                    Self::hand_on_var(st, src.get(&off).copied(), &v);
                }
            }
        }
    }

    /// A closure record the function hands back takes what its slots hold to the caller, which
    /// calls it there: a capture whose store this frame already released — ended on every path
    /// where it exists — is read after its end by the caller (`@FR-L-CapOwn`; loft#2210's
    /// owner-slot hand-off under `LOFT_NO_WRITE_OUT_NCC_ARM`).  Records inside records are
    /// followed through their own slots.
    fn leaving_captures(&mut self, s: &State, value: &Val) {
        let mut todo: Vec<SKey> = Self::stores_of(&value.pts);
        let mut seen: BTreeSet<SKey> = BTreeSet::new();
        while let Some(rec) = todo.pop() {
            if !seen.insert(rec) {
                continue;
            }
            let Some(m) = s.slots.get(&rec) else {
                continue;
            };
            for slot in m.values() {
                for k in Self::stores_of(&slot.pts) {
                    let info = info_of(s, k);
                    if info.has(ENDED) && !info.has(LIVE) && !info.has(HANDED) {
                        self.find(
                            "use-after-end",
                            k.site,
                            "a closure record handed back captures a store this frame already \
                             released: the caller reads it through the closure"
                                .to_string(),
                        );
                    }
                    if s.slots.contains_key(&k) {
                        todo.push(k);
                    }
                }
            }
        }
    }

    /// Every store that may still be live at an exit is a leak.
    fn exit(&mut self, st: &State, how: &str) {
        if self.traced {
            self.trace(&Value::Text(format!("exit: {how}")), st);
        }
        for (k, info) in &st.stores {
            if !info.has(LIVE) || !info.live.is_disjoint(&info.handed_by) {
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
    #[expect(
        clippy::many_single_char_names,
        reason = "one arm per Value variant, each binding its fields by the IR's own short names"
    )]
    fn eval(&mut self, v: &Value, st: State) -> Option<(State, Val)> {
        self.steps += 1;
        if self.steps > STEP_BUDGET {
            self.imprecise = true;
            return None;
        }
        match v {
            Value::Span(b) => self.eval(&b.1, st),
            Value::Block(b) if matches!(b.name, "fn_ref_field_reset" | "fn_ref_field_set") => {
                self.in_field_reset += 1;
                let out = self.seq(&b.operators, st);
                self.in_field_reset -= 1;
                out
            }
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
            Value::Set(x, val) => {
                let mut st = st;
                if let Value::Boolean(c) = val.unspan() {
                    st.flags.insert(*x, *c);
                    return Some((st, Val::null()));
                }
                st.flags.remove(x);
                if self.in_field_reset > 0
                    && matches!(val.unspan(), Value::Call(n, _)
                        if self.data.def(*n).name() == "OpNullRefSentinel")
                {
                    Self::give_back_field_captures(&mut st, *x);
                }
                let copied = self.copied_from(val);
                // `x = f(…, x)` answering `x`'s own store (a value operator writing its result
                // into the buffer `x` hands it): `x` still names, and must-holds, what it did.
                let kept: Vec<SKey> = if copied == Some(*x) {
                    st.stores
                        .iter()
                        .filter(|(_, i)| i.has(LIVE) && i.live.contains(x))
                        .map(|(k, _)| *k)
                        .collect()
                } else {
                    Vec::new()
                };
                let child_rec = matches!(val.unspan(), Value::Call(n, _)
                    if self.data.def(*n).name() == "OpChildRec");
                let (mut st, out) = self.assign(v, *x, val, st)?;
                if child_rec {
                    st.nonnull.insert(*x);
                }
                if let Some(src) = copied
                    && src != *x
                {
                    Self::copy_alts(&mut st, *x, src);
                    // …and a store `src` must-holds that `x` now names, `x` must-holds too.
                    let named = Self::pts(&st, *x);
                    for (k, info) in &mut st.stores {
                        if info.has(LIVE)
                            && info.live.contains(&src)
                            && named.contains(&Pt::Store(*k))
                        {
                            info.live.insert(*x);
                        }
                    }
                }
                for k in kept {
                    if Self::pts(&st, *x).contains(&Pt::Store(k))
                        && let Some(info) = st.stores.get_mut(&k)
                        && info.has(LIVE)
                    {
                        info.live.insert(*x);
                        for a in &mut info.alts {
                            a.insert(*x);
                        }
                    }
                }
                Some((st, out))
            }
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
                let x = match val.unspan() {
                    Value::Var(x) => Some(*x),
                    _ => None,
                };
                self.leaving_captures(&s, &value);
                Self::hand_on_var(&mut s, x, &value);
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
                    let held: Vec<SKey> = Self::stores_of(&Self::held_by_record(&s, w, None));
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
                // A put into a local tuple's member assigns that member, followed or not: the
                // member's earlier store, where it was the only name, is lost by the put.
                let local =
                    !self.params.contains(t) && *t < self.data.def(self.d_nr).variables.count();
                match member_key(*t, *i).filter(|k| local || s.vars.contains_key(k)) {
                    Some(k) => self.bind(&mut s, k, &value),
                    None => Self::hand_on(&mut s, &value),
                }
                Some((s, Val::null()))
            }
            Value::Tuple(xs) if std::mem::take(&mut self.member_tuple) => {
                let (s, parts) = self.tuple_elems(xs, st)?;
                let mut whole = Val::null();
                for p in parts {
                    whole.pts.extend(p.pts);
                    whole.corr.extend(p.corr);
                }
                if whole.pts.len() > 1 {
                    whole.pts.remove(&Pt::Null);
                }
                Some((s, whole))
            }
            Value::Tuple(xs) => {
                let target = self.tuple_target;
                let (mut s, vals) = if target.is_some() {
                    self.tuple_elems(xs, st)?
                } else {
                    self.args(xs, st)?
                };
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
                // A heap member of a tuple PARAMETER lies in a store the caller holds, as the
                // parameter's own would: a callee that answers it borrows argument `t`
                // (`fn first(p: (R, integer)) -> R { p.0 }`).
                None if self.params.contains(t) && self.member_is_heap(*t, *i) => Some((
                    st,
                    Val {
                        pts: [Pt::Param(*t)].into(),
                        corr: BTreeSet::new(),
                    },
                )),
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
                        forget_holder(info, w);
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
                info.alts.clear();
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
            let (mut s, vals) = self.tuple_elems(xs, st)?;
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
            // @FR-O-Proxy asks oracle — the records' own reading; it drives no emission.
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
                // The code generator's own answer (`bind_lowering::vector_null_mints`,
                // `keyed_null_mints`): a lift temporary, a lazy buffer or a borrowed view gets no
                // store at its null init.
                FirstArm::VectorNull | FirstArm::KeyedNull if null_init_mints(arm, function, x) => {
                    self.copied(&mut s, node)
                }
                FirstArm::VectorNull | FirstArm::KeyedNull => Val::null(),
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
                    self.tuple_witness(&s, arm, &value);
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

    /// A join whose witness is a TUPLE slot (`use_analysis::join_witness_members` names no
    /// member): the slot holds no store, so the comparison cannot tell a member the callee
    /// handed back from one it minted — the member is adopted beside its holder and released
    /// twice (loft#2471).
    fn tuple_witness(&mut self, s: &State, arm: crate::bind_lowering::FirstArm, value: &Val) {
        use crate::bind_lowering::FirstArm;
        let (FirstArm::CallRefJoin { base, .. }
        | FirstArm::NullableCallJoin { base, .. }
        | FirstArm::CallJoin { base, .. }) = arm
        else {
            return;
        };
        let function = &self.data.def(self.d_nr).variables;
        if base == u16::MAX
            || !matches!(function.tp(base).base(), crate::data::Type::Tuple(_))
            || crate::use_analysis::join_witness_members(function, base)
                .is_some_and(|m| !m.is_empty())
        {
            return;
        }
        let name = self.var_name(base);
        for k in Self::stores_of(&value.pts) {
            if info_of(s, k).has(LIVE) {
                self.find(
                    "re-free",
                    k.site,
                    format!(
                        "a join's witness `{name}` is a tuple: no store to compare, so a member \
                         handed back is adopted beside its holder and released twice"
                    ),
                );
            }
        }
    }

    /// `x = val` where `x` already holds a slot: the displaced store's free, then the arm.
    /// The variable the `&` link `x` was installed on (`x = OpCreateStack(t)`), when every
    /// install of `x` names that one variable; `None` for a link to a field or an element, or
    /// one re-pointed between variables.
    fn link_install(&self, x: u16) -> Option<u16> {
        let data = self.data;
        // A link to a store-backed kind: a scalar or text link holds no store to release.  A
        // fn-ref is one: a capturing closure's value names its record's store, so a write
        // through `pd: &fn() = &g` lands that record in `g` (B-Ref-Intro/1454).
        // `LOFT_RECORDS_NO_FN_LINK_INSTALL=1` leaves a fn link unfollowed.
        let crate::data::Type::RefVar(inner) = data.def(self.d_nr).variables.tp(x).base() else {
            return None;
        };
        let fn_link = matches!(inner.base(), crate::data::Type::Function(..))
            && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_FN_LINK_INSTALL").is_some());
        if !(fn_link
            || matches!(
                inner.base(),
                crate::data::Type::Vector(..)
                    | crate::data::Type::Reference(..)
                    | crate::data::Type::Enum(_, true, _)
            )
            || crate::data::is_keyed(inner.base()))
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

    /// The reassignments that are not `bind_lowering`'s arms: a write through a local link, the
    /// write-back of a `&` parameter, a lazy buffer's mint.  `Err` hands the state back.
    fn reassign_special(
        &mut self,
        node: &Value,
        x: u16,
        val: &Value,
        st: State,
    ) -> Result<Option<(State, Val)>, Box<State>> {
        let data = self.data;
        let function = &data.def(self.d_nr).variables;
        let installs = matches!(val.unspan(), Value::Call(d, _)
            if matches!(data.def(*d).name(), "OpCreateStack" | "OpVarRef"));
        // `@FR-F-ParamRef` — an OWNED value written through a slot link (`OpSetSlotRef` with
        // `COPY_FREE_SOURCE`): copied into the field the cell names and its store released
        // there, or moved into the caller's variable.  Either way the frame gives it up.
        if !installs
            && data.slot_link_def(function.tp(x)).is_some()
            && matches!(
                crate::use_analysis::ownership_of(data, self.d_nr, val),
                crate::use_analysis::Own::Owned
            )
        {
            let Some((mut s, value)) = self.eval(val, st) else {
                return Ok(None);
            };
            Self::hand_on(&mut s, &value);
            return Ok(Some((s, Val::null())));
        }
        if let Some(t) = self.link_install(x)
            && !installs
        {
            return Ok(self.link_write(x, t, val, st));
        }
        // `@FR-F-ParamRef` — a whole write through a `&` PARAMETER is the write-back: the
        // value is the caller's from here (handed on), and the store it displaced is the
        // caller's to release (the generator's stash / install / `OpFreeRefIfDistinct`).
        if function.is_argument(x)
            && matches!(function.tp(x).base(), crate::data::Type::RefVar(_))
            && !matches!(val.unspan(), Value::Call(d, _)
                if matches!(data.def(*d).name(), "OpCreateStack" | "OpVarRef"))
        {
            let shown = Self::pts(&st, x);
            let Some((mut s, value)) = self.eval(val, st) else {
                return Ok(None);
            };
            // `@FR-L-CapOwn` — a closure this frame wrote through the same `&fn` link before
            // never reached the caller: the write displaces it, and it is this frame's again,
            // held by whatever still names it (its `___clos_N`, which the frame releases).
            // Only a store this frame made; the caller's own value (`Param`) stays the caller's.
            // `LOFT_RECORDS_NO_LINK_TAKE_BACK=1` leaves it handed on.
            if matches!(function.tp(x).base(), crate::data::Type::RefVar(inner)
                    if matches!(inner.base(), crate::data::Type::Function(..)))
                && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_LINK_TAKE_BACK").is_some())
            {
                for k in Self::stores_of(&shown) {
                    if value.pts.contains(&Pt::Store(k)) {
                        continue;
                    }
                    let holders: Vec<u16> = s
                        .vars
                        .iter()
                        .filter(|&(&w, p)| w != x && p.len() == 1 && p.contains(&Pt::Store(k)))
                        .map(|(&w, _)| w)
                        .collect();
                    if let Some(info) = s.stores.get_mut(&k)
                        && info.has(HANDED)
                        && !info.has(ENDED)
                        && !holders.is_empty()
                    {
                        info.status = (info.status & !HANDED) | LIVE;
                        info.live.extend(holders);
                    }
                }
            }
            Self::hand_on(&mut s, &value);
            // …and a read of the link from here answers what was written through it: the
            // caller's variable now holds it (`__fklink_N = out` after `out = h` names `h`'s
            // closure record).  Handed on, the store takes no live holder from this.
            // `LOFT_RECORDS_NO_LINK_READS_WRITE=1` leaves the link reading the parameter.
            // Inside a loop the link keeps reading the parameter: a write on some passes only
            // (`if i == 0 { p = f }`) joins into "the caller's value or this pass's closure",
            // and nothing here ties that choice back to the pass, so the read would split the
            // releases that guard on it both ways (L-CapKeep/1636, three findings more).
            if self.loops.is_empty()
                && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_LINK_READS_WRITE").is_some())
            {
                self.bind(&mut s, x, &value);
            }
            return Ok(Some((s, Val::null())));
        }
        // A lazy buffer's guarded `Set(v, Null)` is where its store is made (`set_var`).
        if crate::bind_lowering::lazy_buffer_mints_at(function, x, val) {
            let mut st = st;
            let site = self.site(node, Origin::Mint);
            let k = Self::fresh(&mut st, site);
            let minted = Val {
                pts: [Pt::Store(k)].into(),
                corr: [k].into(),
            };
            self.bind(&mut st, x, &minted);
            return Ok(Some((st, Val::null())));
        }
        Err(Box::new(st))
    }

    /// Does a displaced free's witness `w` keep the store `x` displaces?  The runtime keeps it
    /// when the witness names it.  A null `x` frees nothing, so the null `x` may also hold (a
    /// loop's first pass) is no reason to free: the two are compared without it.
    fn witness_keeps(st: &State, x: u16, w: u16) -> bool {
        let held: Pts = Self::pts(st, x)
            .into_iter()
            .filter(|p| *p != Pt::Null)
            .collect();
        let named: Pts = Self::pts(st, w)
            .into_iter()
            .filter(|p| *p != Pt::Null)
            .collect();
        !held.is_empty() && held == named
    }

    fn reassign(&mut self, node: &Value, x: u16, val: &Value, st: State) -> Option<(State, Val)> {
        use crate::bind_lowering::{DisplacedFree, ReArm};
        let data = self.data;
        let function = &data.def(self.d_nr).variables;
        let st = match self.reassign_special(node, x, val, st) {
            Ok(done) => return done,
            Err(st) => *st,
        };
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
                .any(|w| Self::witness_keeps(&st, x, w));
            if !keep {
                self.end(&mut st, x, "the displaced free", true);
            }
        }
        let (mut s, value) = self.eval(val, st)?;
        // What `x` held, read AFTER the value: a call that makes a store at a site `x`'s old
        // store came from moves that store to the older generation (`fresh`), and a key read
        // before it would name the new store instead (a loop's `fixed = step(i)`).
        let old = Self::pts(&s, x);
        // Parked in the state while the arm runs: a copy that mints at the site `x`'s old store
        // came from moves it to the older generation, and a key kept outside the state would
        // then name the new store (a loop's `class = pick(class, c)`).
        s.vars.insert(OLD_NAME, old.clone());
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
        let old = s.vars.remove(&OLD_NAME).unwrap_or(old);
        if rb.free == DisplacedFree::Stash && rb.arm != ReArm::KeyedNull && bound.pts != old {
            self.stashed_free(&mut s, x, &old, &bound);
            return Some((s, Val::null()));
        }
        self.bind(&mut s, x, &bound);
        Some((s, Val::null()))
    }

    /// The stashed old reference of a reassignment, freed unless the new value names the
    /// same store, with `x` bound to the new value.
    fn stashed_free(&mut self, s: &mut State, x: u16, old: &Pts, bound: &Val) {
        let tmp = u16::MAX;
        s.vars.insert(tmp, old.clone());
        for k in Self::stores_of(old) {
            if let Some(info) = s.stores.get_mut(&k)
                && info.live.contains(&x)
            {
                info.live.insert(tmp);
            }
        }
        self.bind(s, x, bound);
        let shared: Vec<SKey> = Self::stores_of(old)
            .into_iter()
            .filter(|k| bound.pts.contains(&Pt::Store(*k)))
            .collect();
        if shared.is_empty() {
            self.end(s, tmp, "the stashed free", false);
        } else {
            // A store the new value may name too is freed on the paths where it does not:
            // where `x` named it on every path it was live, it stays live exactly where
            // `x` names it now (`none = rec(k - 1, none)`, answering `none` or a fresh store).
            let rest: BTreeSet<SKey> = Self::stores_of(old)
                .into_iter()
                .filter(|k| !shared.contains(k))
                .collect();
            if !rest.is_empty() {
                self.end_only(s, tmp, "the stashed free", false, Some(&rest));
            }
            for k in &shared {
                let Some(info) = s.stores.get_mut(k) else {
                    continue;
                };
                if info.has(LIVE) && (old.len() == 1 || info.live.contains(&tmp)) {
                    info.status |= ENDED;
                    info.live.insert(x);
                }
            }
        }
        s.vars.remove(&tmp);
        for info in s.stores.values_mut() {
            forget_holder(info, tmp);
            info.ended.remove(&tmp);
        }
    }

    /// A copy's source-free (`COPY_FREE_SOURCE`) releases what the call answered unless the
    /// copy bracket protects it: a store of the answer that no protectable argument names
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
            Value::Var(b) if st.flags.contains_key(b) => {
                if st.flags[b] {
                    (Some(st), None)
                } else {
                    (None, Some(st))
                }
            }
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
            Value::Call(op, a) if *op == self.ops.distinct => {
                let (distinct, mut same) = self.decide_distinct(a, st);
                // Where the two are the SAME, every name set holding one holds the other.
                if let (Some(Value::Var(x)), Some(Value::Var(w))) =
                    (a.first().map(Value::unspan), a.get(1).map(Value::unspan))
                    && let Some(s) = &mut same
                {
                    Self::equate(s, *x, *w);
                }
                (distinct, same)
            }
            Value::Call(op, a) if a.len() == 1 => {
                let Value::Var(x) = a[0].unspan() else {
                    return both(st);
                };
                if *op == self.ops.ref_is_null {
                    Self::null_test(*x, st)
                } else if *op == self.ops.vector_is_null && self.only_call_answers(&st, *x) {
                    // A collection a call answered is null only as the null sentinel: a
                    // `-> hash<…>?` hands back no absent-marked store (loft#1150).  A local's
                    // null may be such a store, so for it the test decides nothing.
                    Self::null_test(*x, st)
                } else if *op == self.ops.ref_present {
                    // `OpConvBoolFromRef(x)`: true when `x` names a record.  A freed store
                    // still names one, so this is the null test turned round, never a
                    // liveness test.
                    let (null, present) = Self::null_test(*x, st);
                    (present, null)
                } else if *op == self.ops.store_live {
                    let (live, dead) = Self::live_test(*x, st);
                    (live, self.no_record_holds(*x, dead))
                } else {
                    both(st)
                }
            }
            _ => both(st),
        }
    }

    /// A fn field overwritten while it held the frame's own closure record (a field reset or
    /// set nulls `___clos_N` where the field's old record is it): the record leaves the field, and
    /// the stores it took over at the capture are the frame's again, held by whatever still
    /// names them (`h.f = fn() { v.a }; h.f = id` — the frame then frees `v`).
    fn give_back_field_captures(st: &mut State, w: u16) {
        let Some(taken) = st.field_captures.get(&w).cloned() else {
            return;
        };
        for k in Self::stores_of(&taken) {
            let holders: BTreeSet<u16> = st
                .vars
                .iter()
                .filter(|(_, p)| p.len() == 1 && p.contains(&Pt::Store(k)))
                .map(|(v, _)| *v)
                .collect();
            if let Some(info) = st.stores.get_mut(&k)
                && info.has(HANDED)
                && info.captured_by.contains(&w)
            {
                info.status = (info.status & !HANDED) | LIVE;
                info.live = holders;
                info.handed_by.clear();
            }
        }
    }

    /// A closure record variable (`___clos_N`): its capture slots hold stores.
    fn is_closure_record(&self, w: u16) -> bool {
        self.data
            .def(self.d_nr)
            .variables
            .name(w)
            .starts_with("___clos_")
    }

    /// On a path where the record closure record `x` names is not live, its slots hold nothing:
    /// the holder sets naming it do not hold there.  (A NULL `x` says less: the record may have
    /// been handed to a fn-ref and live on.)
    fn no_record_holds(&self, x: u16, st: Option<State>) -> Option<State> {
        let mut st = st?;
        if self.is_closure_record(x) && !slot_holders_off() {
            rule_out_everywhere(&mut st, x);
        }
        Some(st)
    }

    /// Does every store `x` may name come from a call's answer?
    fn only_call_answers(&self, st: &State, x: u16) -> bool {
        let stores = Self::stores_of(&Self::pts(st, x));
        !stores.is_empty()
            && stores
                .iter()
                .all(|k| matches!(self.sites[k.site as usize].origin, Some(Origin::Call(_))))
    }

    /// `OpRefIsNull(x)`, as (null arm, store arm).  Where `x` is null, a store `x` points
    /// into on every path where that store is live is not live, and one it points into on
    /// every path where it is ended is not ended.
    fn null_test(x: u16, st: State) -> (Option<State>, Option<State>) {
        let pts = Self::pts(&st, x);
        let may_null = !st.nonnull.contains(&x)
            && pts
                .iter()
                .any(|p| matches!(p, Pt::Null | Pt::Param(_) | Pt::Unknown));
        let may_store = pts.iter().any(|p| !matches!(p, Pt::Null));
        let null_arm = may_null
            .then(|| {
                let mut s = st.clone();
                for k in Self::stores_of(&pts) {
                    if let Some(info) = s.stores.get_mut(&k) {
                        rule_out_holder(info, x);
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
                Self::record_null_possible(&mut s, x).then_some(s)
            })
            .flatten();
        let store_arm = may_store
            .then(|| {
                let mut s = st;
                if let Some(p) = s.vars.get_mut(&x) {
                    p.remove(&Pt::Null);
                }
                Self::record_present_possible(&mut s, x).then_some(s)
            })
            .flatten();
        (null_arm, store_arm)
    }

    /// On a path where the closure-record variable `x` names no record: a store live only
    /// where one of its `live_some` records does is not live here once `x` was the last.  `false` when that leaves a store no status — the
    /// path does not happen.
    fn record_null_possible(st: &mut State, x: u16) -> bool {
        for info in st.stores.values_mut() {
            if info.has(LIVE) && info.live_some.remove(&x) && info.live_some.is_empty() {
                info.status &= !LIVE;
                info.live.clear();
                info.alts.clear();
                if info.status == 0 {
                    return false;
                }
            }
        }
        true
    }

    /// On a path where the closure-record variable `x` names a record: a store live only
    /// where it is null is not live here.  `false` when that leaves a store no status.
    fn record_present_possible(st: &mut State, x: u16) -> bool {
        for info in st.stores.values_mut() {
            if info.has(LIVE) && info.live_null.contains(&x) {
                info.status &= !LIVE;
                info.live.clear();
                info.alts.clear();
                info.live_null.clear();
                if info.status == 0 {
                    return false;
                }
            }
        }
        true
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
            if !Self::record_present_possible(&mut s, x) {
                possible = false;
            }
            possible.then_some(s)
        };
        // Not live: `x` is null, or the store it names has ended (or was never made).
        let dead_arm = {
            let mut s = st;
            let mut possible = !followed || pts.contains(&Pt::Null) || stores.is_empty();
            for k in &stores {
                if let Some(info) = s.stores.get_mut(k) {
                    rule_out_holder(info, x);
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
            // Where no store `x` names can have ended, the dead arm is `x` null.
            let as_null = followed && stores.iter().all(|k| info_of(&s, *k).status & !ABSENT == 0);
            if as_null && !Self::record_null_possible(&mut s, x) {
                possible = false;
            }
            possible.then_some(s)
        };
        (live_arm, dead_arm)
    }

    /// The variable whose value `val` is on every path where that variable names a store: a
    /// plain read or an alias (`OpRefAlias`), or a call answering an argument's store as it
    /// is (`Arg`, or `Buf` where the buffer it is handed names one).
    fn copied_from(&self, val: &Value) -> Option<u16> {
        match val.unspan() {
            Value::Var(y) => Some(*y),
            Value::Call(d, args)
                if (*d as usize) < self.data.definitions.len()
                    && self.data.def(*d).name() == "OpRefAlias" =>
            {
                Self::first_var(args, 0)
            }
            Value::Call(d, args) => match self.summaries.get(d) {
                Some(Summary::Arg(i) | Summary::Buf(i)) => Self::first_var(args, *i as usize),
                // `@FR-R-HostStore`: a callee handed a STORE buffer answers in that store.
                Some(Summary::Join(i)) => match args.get(*i as usize).map(Value::unspan) {
                    Some(Value::Call(sb, a)) if self.data.def(*sb).name() == "OpStoreBuf" => {
                        Self::first_var(a, 0)
                    }
                    _ => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    /// `dst` now names what `src` names wherever `src` names a store: an alternative holding
    /// `src` holds `dst` as well.
    fn copy_alts(st: &mut State, dst: u16, src: u16) {
        for info in st.stores.values_mut() {
            for a in &mut info.alts {
                if a.contains(&src) {
                    a.insert(dst);
                }
            }
        }
    }

    /// On a path where `x` and `w` name one store: a must-holder set or an alternative
    /// holding either holds both.
    fn equate(st: &mut State, x: u16, w: u16) {
        for info in st.stores.values_mut() {
            if !info.has(LIVE) {
                continue;
            }
            if info.live.contains(&x) || info.live.contains(&w) {
                info.live.insert(x);
                info.live.insert(w);
            }
            for a in &mut info.alts {
                if a.contains(&x) || a.contains(&w) {
                    a.insert(x);
                    a.insert(w);
                }
            }
        }
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
                        Some(Value::Var(w)) => {
                            Some(Self::held_by_record(&st, *w, Self::const_offset(a.get(1))))
                                .filter(|p| !p.is_empty())
                        }
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
            Some(q @ (Pt::Store(_) | Pt::Null | Pt::Param(_))) if p.len() == 1 => Some(*q),
            _ => None,
        };
        if let Some(arms) = self.slot_arms(args, &a, &b, &st) {
            return arms;
        }
        match (single(&a), single(&b)) {
            (Some(x), Some(y)) if x == y => (None, Some(st)),
            (Some(Pt::Store(_) | Pt::Null), Some(Pt::Store(_) | Pt::Null)) => (Some(st), None),
            // A store this frame made is never the caller's (`if OpDistinctStore(f, __rbw_f)`
            // after `f` was rebound to a fresh store).
            (Some(Pt::Store(_)), Some(Pt::Param(_))) | (Some(Pt::Param(_)), Some(Pt::Store(_))) => {
                (Some(st), None)
            }
            // The witness names one store: where the first operand is DISTINCT from it, it
            // does not name that store (`if OpDistinctStore(__ref_1, __rbw___ref_1)` — the
            // caller's buffer is never the one freed under it).
            (_, Some(y @ (Pt::Store(_) | Pt::Param(_))))
                if let Some(Value::Var(x)) = args.first().map(Value::unspan)
                    && a.len() > 1
                    && a.contains(&y) =>
            {
                let mut distinct = st.clone();
                let rest: Pts = a.iter().copied().filter(|p| *p != y).collect();
                Self::narrow(&mut distinct, *x, rest);
                let mut same = st;
                Self::narrow(&mut same, *x, [y].into());
                (Some(distinct), Some(same))
            }
            // The witness is null: where the first operand is the SAME, it is null too.
            (_, Some(Pt::Null))
                if let Some(Value::Var(x)) = args.first().map(Value::unspan)
                    && a.len() > 1
                    && a.contains(&Pt::Null)
                    && !a.contains(&Pt::Unknown) =>
            {
                let mut same = st.clone();
                Self::narrow(&mut same, *x, [Pt::Null].into());
                let mut distinct = st;
                let rest: Pts = a.iter().copied().filter(|p| *p != Pt::Null).collect();
                Self::narrow(&mut distinct, *x, rest);
                (Some(distinct), Some(same))
            }
            // Where the two are the SAME, the first names only what the witness may name too:
            // a store both may name, or null or a caller's store where the witness may be null
            // or a caller's store (two parameters may name one store, or both be null).
            _ if let Some(Value::Var(x)) = args.first().map(Value::unspan)
                && !a.contains(&Pt::Unknown)
                && !b.contains(&Pt::Unknown) =>
            {
                let b_null = b.iter().any(|p| matches!(p, Pt::Null | Pt::Param(_)));
                let same_pts: Pts = a
                    .iter()
                    .copied()
                    .filter(|p| {
                        b.contains(p)
                            || (*p == Pt::Null && b_null)
                            // A caller's store may be another parameter's, or null.
                            || (matches!(p, Pt::Param(_)) && b_null)
                    })
                    .collect();
                if same_pts.is_empty() {
                    return (Some(st), None);
                }
                let mut same = st.clone();
                if same_pts != a {
                    Self::narrow(&mut same, *x, same_pts);
                }
                (Self::distinct_narrowed(args, *x, &a, st), Some(same))
            }
            _ => (Some(st.clone()), Some(st)),
        }
    }

    /// The DISTINCT arm of `OpDistinctStore(x, w)`: `x` names no store the witness holds
    /// wherever it is live, as `x` does, since there the two are one store (`__ref_p2_1` and
    /// the tuple member `b.0` it built, after a loop that ran no pass).  Where the witness
    /// alone holds such a store, `x` names it only where it is live, so there too the two are
    /// one store.  Where that rules out every store `x` names, the arm cannot run.
    fn distinct_narrowed(args: &[Value], x: u16, a: &Pts, st: State) -> Option<State> {
        let w = args.get(1).and_then(|v| Self::named_var(&st, v));
        let distinct_pts: Pts = a
            .iter()
            .copied()
            .filter(|p| match (p, w) {
                (Pt::Store(k), Some(w)) => {
                    let info = info_of(&st, *k);
                    !(info.has(LIVE)
                        && info.live.contains(&w)
                        && (info.live.contains(&x) || !info.has(ENDED)))
                }
                _ => true,
            })
            .collect();
        if distinct_pts.is_empty() {
            return None;
        }
        let mut distinct = st;
        if distinct_pts != *a {
            Self::narrow(&mut distinct, x, distinct_pts);
        }
        Some(distinct)
    }

    /// On a path where `x` is known to name only `to`: a store it no longer names, and that
    /// is live only where `x` names it, is not live on this path.
    /// Keep, in the slot `off` of every record `w` names, only the stores `keep` admits — a
    /// path-sensitive refinement of what the slot holds on one arm of an identity test.  Its
    /// `corr` follows: a store dropped from the slot is no longer live through it.
    /// `OpDistinctStore(OpGetDbRef(w, off), v)` where `v` names one store `y` and the slot may
    /// hold `y` or others (a loop rebuild's guarded free of the record's old capture): where
    /// DISTINCT, the slot does not hold `y`; where SAME, only `y` — as a variable first operand
    /// is narrowed.  `LOFT_RECORDS_NO_SLOT_NARROW=1` keeps both arms unnarrowed.
    fn slot_arms(
        &self,
        args: &[Value],
        a: &Pts,
        b: &Pts,
        st: &State,
    ) -> Option<(Option<State>, Option<State>)> {
        let Some(Value::Call(op, ga)) = args.first().map(Value::unspan) else {
            return None;
        };
        let (Some(Value::Var(w)), Some(off)) =
            (ga.first().map(Value::unspan), Self::const_offset(ga.get(1)))
        else {
            return None;
        };
        let y = match b.iter().next() {
            Some(y @ Pt::Store(_)) if b.len() == 1 => *y,
            _ => return None,
        };
        if *op != self.ops.get_dbref
            || a.len() < 2
            || !a.contains(&y)
            || crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_SLOT_NARROW").is_some())
        {
            return None;
        }
        let mut distinct = st.clone();
        Self::narrow_slot(&mut distinct, *w, off, |p| *p != y);
        let mut same = st.clone();
        Self::narrow_slot(&mut same, *w, off, |p| *p == y);
        Some((Some(distinct), Some(same)))
    }

    fn narrow_slot(st: &mut State, w: u16, off: i32, keep: impl Fn(&Pt) -> bool) {
        for k in Self::stores_of(&Self::pts(st, w)) {
            if let Some(v) = st.slots.get_mut(&k).and_then(|m| m.get_mut(&off)) {
                v.pts.retain(|p| !matches!(p, Pt::Store(_)) || keep(p));
                v.corr.retain(|c| keep(&Pt::Store(*c)));
            }
        }
    }

    fn narrow(st: &mut State, x: u16, to: Pts) {
        let from = Self::pts(st, x);
        for k in Self::stores_of(&from) {
            if to.contains(&Pt::Store(k)) {
                continue;
            }
            if let Some(info) = st.stores.get_mut(&k)
                && info.has(LIVE)
            {
                rule_out_holder(info, x);
                if info.status == 0 {
                    st.stores.remove(&k);
                }
            }
        }
        if to.len() == 1 && to.contains(&Pt::Null) {
            st.vars.remove(&x);
        } else {
            st.vars.insert(x, to);
        }
    }

    fn seq(&mut self, ops: &[Value], st: State) -> Option<(State, Val)> {
        let mut cur = (st, Val::null());
        for op in ops {
            let value = cur.1;
            cur = self.eval(op, cur.0)?;
            if self.traced {
                self.trace(op, &cur.0);
            }
            // A block's value is its last value-producing statement's: the cleanup frees the
            // scope pass places after it (`{ if … ; OpFreeRefIfDistinct(…) }`) run while the
            // value waits, and answer nothing.
            if self.is_void_call(op) {
                cur.1 = value;
            }
        }
        Some(cur)
    }

    /// `LOFT_STORE_TRACE=<fn substring>`: each statement of a matching function, with what
    /// every variable names and every store's statuses and holders after it.
    fn trace(&self, op: &Value, st: &State) {
        let names = |s: &BTreeSet<u16>| s.iter().map(|v| self.var_name(*v)).collect::<Vec<_>>();
        let key = |k: &SKey| format!("#{}{}", k.site, if k.older { "'" } else { "" });
        let mut o: String = format!("{op:?}").chars().take(110).collect();
        o.push_str("\n    vars:");
        for (v, p) in &st.vars {
            let pts: Vec<String> = p
                .iter()
                .map(|p| match p {
                    Pt::Store(k) => key(k),
                    other => format!("{other:?}"),
                })
                .collect();
            let _ = write!(o, " {}={{{}}}", self.var_name(*v), pts.join(","));
        }
        o.push_str("\n    stores:");
        for (k, i) in &st.stores {
            let alts: Vec<Vec<String>> = i.alts.iter().map(names).collect();
            let _ = write!(
                o,
                " {}:st{} live{:?} alts{alts:?} end{:?}",
                key(k),
                i.status,
                names(&i.live),
                names(&i.ended)
            );
        }
        eprintln!("[store-trace] {o}");
    }

    /// A call to an operator or function that answers nothing (a free, a set).
    fn is_void_call(&self, op: &Value) -> bool {
        let Value::Call(d, _) = op.unspan() else {
            return false;
        };
        (*d as usize) < self.data.definitions.len()
            && matches!(self.data.def(*d).returned().base(), crate::data::Type::Void)
    }

    fn run_loop(&mut self, body: &[Value], st: State) -> Option<(State, Val)> {
        // The first pass on its own, from the state before the loop: the null a variable has
        // before its first bind, and the store it held before the loop, never meet the state
        // of a later pass at the head (a lazy buffer's mint, a store rebound away, a closure
        // built per pass).  Its exits — the zero-pass one included — join the others.
        self.loops.push((None, None));
        let first = self.seq(body, st);
        let (brk, cont) = self.loops.pop().unwrap_or_default();
        let mut exits: Option<State> = brk;
        let Some(mut head) = join_opt(first.map(|(s, _)| s), cont) else {
            return exits.map(|s| (s, Val::null()));
        };
        Self::normalize(&mut head);
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

    /// The elements of a tuple literal bound into a tuple local's members: a nested tuple
    /// literal is part of its member, so it answers the stores it holds rather than handing
    /// them on (`n = ((S { … }, 22), 8)`).
    fn tuple_elems(&mut self, xs: &[Value], st: State) -> Option<(State, Vec<Val>)> {
        let mut s = st;
        let mut vals = Vec::with_capacity(xs.len());
        for x in xs {
            if let Value::Tuple(inner) = x.unspan() {
                let (s2, parts) = self.tuple_elems(inner, s)?;
                s = s2;
                let mut whole = Val::null();
                for p in parts {
                    whole.pts.extend(p.pts);
                    whole.corr.extend(p.corr);
                }
                if whole.pts.len() > 1 {
                    whole.pts.remove(&Pt::Null);
                }
                vals.push(whole);
            } else if Self::tail_is_tuple(x) {
                // A member moved out of another tuple (`u = t`): a block ending in the
                // member's own tuple literal.  Its tail is joined like a bare nested literal.
                let target = self.tuple_target.take();
                let out = self.eval_member_block(x, s);
                self.tuple_target = target;
                let (s2, v) = out?;
                s = s2;
                vals.push(v);
            } else {
                let (s2, vs) = self.args(std::slice::from_ref(x), s)?;
                s = s2;
                vals.extend(vs);
            }
        }
        Some((s, vals))
    }

    /// Is `v` a block or insert whose value is a tuple literal?
    fn tail_is_tuple(v: &Value) -> bool {
        match v.unspan() {
            Value::Block(bl) => bl
                .operators
                .last()
                .is_some_and(|t| matches!(t.unspan(), Value::Tuple(_))),
            Value::Insert(ops) => ops
                .last()
                .is_some_and(|t| matches!(t.unspan(), Value::Tuple(_))),
            _ => false,
        }
    }

    /// Evaluate a [`Self::tail_is_tuple`] block with its tail read as a member.
    fn eval_member_block(&mut self, v: &Value, st: State) -> Option<(State, Val)> {
        let ops = match v.unspan() {
            Value::Block(bl) => &bl.operators,
            Value::Insert(ops) => ops,
            _ => return self.eval(v, st),
        };
        let (last, init) = ops.split_last()?;
        let mut s = st;
        for op in init {
            let (s2, _) = self.eval(op, s)?;
            s = s2;
        }
        self.member_tuple = true;
        let out = self.eval(last, s);
        self.member_tuple = false;
        out
    }

    /// Is member `i` of tuple variable `t` a store reference?
    fn member_is_heap(&self, t: u16, i: u16) -> bool {
        let vars = &self.data.def(self.d_nr).variables;
        t < vars.count()
            && matches!(vars.tp(t).base(), crate::data::Type::Tuple(elems)
                if elems.get(i as usize).is_some_and(|e| crate::data::is_dbref(e.base())))
    }

    /// A user function's arguments: a tuple literal argument is the callee's to READ, so its
    /// members' stores stay with the caller, who releases them (a call-minted member is lifted
    /// into a temp the scope pass frees) — where a tuple literal elsewhere hands them on.
    fn user_call_args(&mut self, args: &[Value], st: State) -> Option<(State, Vec<Val>)> {
        let mut s = st;
        let mut vals = Vec::with_capacity(args.len());
        for a in args {
            // A block ending in the literal (`{ __ref_8 = tp; (OpGetField(__ref_8, …), …) }`, a
            // boxed tuple unboxed into the argument) runs its leading statements first.
            let (prefix, lit): (&[Value], &Value) = match a.unspan() {
                Value::Block(b) => match b.operators.split_last() {
                    Some((last, init)) if matches!(last.unspan(), Value::Tuple(_)) => (init, last),
                    _ => (&[], a),
                },
                Value::Insert(ops) => match ops.split_last() {
                    Some((last, init)) if matches!(last.unspan(), Value::Tuple(_)) => (init, last),
                    _ => (&[], a),
                },
                _ => (&[], a),
            };
            if let Value::Tuple(xs) = lit.unspan() {
                if !prefix.is_empty() {
                    s = self.seq(prefix, s)?.0;
                }
                let target = self.tuple_target.take();
                let parts = self.tuple_elems(xs, s);
                self.tuple_target = target;
                let (s2, parts) = parts?;
                s = s2;
                let mut whole = Val::null();
                for p in parts {
                    whole.pts.extend(p.pts);
                }
                if whole.pts.len() > 1 {
                    whole.pts.remove(&Pt::Null);
                }
                vals.push(whole);
            } else {
                let (s2, vs) = self.args(std::slice::from_ref(a), s)?;
                s = s2;
                vals.extend(vs);
            }
        }
        Some((s, vals))
    }

    /// Evaluate call arguments left to right.  A bare variable argument is a use of its
    /// store.
    fn args(&mut self, args: &[Value], st: State) -> Option<(State, Vec<Val>)> {
        let target = self.tuple_target.take();
        let member = std::mem::take(&mut self.member_tuple);
        let out = self.args_inner(args, st);
        self.tuple_target = target;
        self.member_tuple = member;
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
            if let Some(h) = Self::tail_var(a) {
                self.use_points(&s2, h);
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

    /// `OpFreeRecordIn` releases one record: a store of its own only where the record is
    /// that store's root (a placed record whose host was null); a record placed inside a host
    /// leaves the host's store live.
    fn free_record_in(&mut self, args: &[Value], name: &str, st: &mut State) {
        if let Some(x) = Self::first_var(args, 0) {
            let placed: BTreeSet<SKey> = Self::stores_of(&Self::pts(st, x))
                .into_iter()
                .filter(|k| matches!(self.sites[k.site as usize].origin, Some(Origin::Place)))
                .collect();
            if !placed.is_empty() {
                self.end_only(st, x, name, true, Some(&placed));
            }
        }
    }

    /// A free op: its own argument is not a use.
    /// A capture SLOT freed unless the witness names its store (a closure build releasing the
    /// capture an earlier pass left in its record): the slot's stores the witness cannot name
    /// end, for certain where the slot names only that one.  `false` when the freed operand is
    /// no slot of a record.
    fn free_slot_unless_witness(&mut self, args: &[Value], name: &str, st: &mut State) -> bool {
        let Some(Value::Call(g, gargs)) = args.first().map(Value::unspan) else {
            return false;
        };
        if *g != self.ops.get_dbref {
            return false;
        }
        let Some(rec) = Self::first_var(gargs, 0) else {
            return false;
        };
        if !Self::is_record(st, rec) {
            return false;
        }
        let held = Self::held_by_record(st, rec, Self::const_offset(gargs.get(1)));
        let w = args.get(1).and_then(|a| Self::named_var(st, a));
        let wp = w.map(|w| Self::pts(st, w));
        let label = format!("{name}({})", self.var_name(rec));
        let stores = Self::stores_of(&held);
        let single = stores.len() == 1 && !held.contains(&Pt::Unknown);
        for k in stores {
            if wp.as_ref().is_some_and(|wp| wp.contains(&Pt::Store(k))) {
                // The witness may name it: it ends where the witness does not, and where the
                // slot names only this store it stays live exactly where the witness names it.
                if let Some(w) = w
                    && single
                    && let Some(info) = st.stores.get_mut(&k)
                    && info.has(LIVE)
                    && !info.live.contains(&w)
                    && wp.as_ref().is_some_and(|wp| wp.len() > 1)
                {
                    self.sites[k.site as usize].ends.insert(label.clone());
                    info.status |= ENDED;
                    info.live.insert(w);
                    for a in &mut info.alts {
                        a.insert(w);
                    }
                }
                continue;
            }
            self.sites[k.site as usize].ends.insert(label.clone());
            if let Some(info) = st.stores.get_mut(&k)
                && info.has(LIVE)
            {
                if single {
                    info.status = (info.status & !LIVE) | ENDED;
                    info.live.clear();
                    info.alts.clear();
                } else {
                    info.status |= ENDED;
                }
            }
        }
        true
    }

    /// A capture SLOT as a guarded free's witness: the store is live exactly where the slot
    /// names it, so it joins the slot's correlation and goes where the record goes.
    fn correlate_witness_slot(st: &mut State, get_dbref: u32, witness: Option<&Value>, k: SKey) {
        let Some(Value::Call(g, gargs)) = witness.map(Value::unspan) else {
            return;
        };
        if *g != get_dbref {
            return;
        }
        let (Some(r), Some(off)) = (Self::first_var(gargs, 0), Self::const_offset(gargs.get(1)))
        else {
            return;
        };
        for rec in Self::stores_of(&Self::pts(st, r)) {
            if let Some(slot) = st.slots.get_mut(&rec).and_then(|m| m.get_mut(&off))
                && slot.pts.contains(&Pt::Store(k))
            {
                slot.corr.insert(k);
            }
        }
    }

    /// A guarded free's stores the witness may name end only on the paths it does not: where
    /// `x` names one on every path it is live, it stays live exactly where the witness names
    /// it (`return if c { f } else { g }`).
    #[expect(clippy::too_many_arguments, reason = "the guarded free's own operands")]
    fn end_where_witness_does_not(
        &mut self,
        st: &mut State,
        x: u16,
        xp: &Pts,
        w: Option<u16>,
        field_slot: bool,
        shared: &[SKey],
        args: &[Value],
        name: &str,
    ) {
        let label = format!("{name}({})", self.var_name(x));
        for k in shared {
            self.sites[k.site as usize].ends.insert(label.clone());
            let Some(info) = st.stores.get_mut(k) else {
                continue;
            };
            if !info.has(LIVE) {
                continue;
            }
            if field_slot {
                info.status = (info.status & !LIVE) | ENDED;
                info.live.clear();
                info.alts.clear();
                info.ended = [x].into();
                continue;
            }
            let strong = xp.len() == 1 || info.live.contains(&x);
            info.ended = if info.has(ENDED) {
                info.ended.intersection(&[x].into()).copied().collect()
            } else {
                [x].into()
            };
            info.status |= ENDED;
            if strong && let Some(w) = w {
                info.live.insert(w);
            }
            if strong && w.is_none() {
                Self::correlate_witness_slot(st, self.ops.get_dbref, args.get(1), *k);
            }
        }
    }

    fn free_op(&mut self, d: u32, args: &[Value], st: State) -> State {
        let ops = self.ops;

        if ops.unconditional.contains(&d) || d == ops.text_free {
            let name = self.data.def(d).name().to_string();
            let mut st = st;
            if let Some(Value::Call(g, gargs)) = args.first().map(Value::unspan)
                && *g == ops.get_dbref
                && let Some(w) = Self::first_var(gargs, 0)
                && Self::is_record(&st, w)
            {
                self.end_slots(&mut st, w, &name);
                return st;
            }
            if d == ops.free_in {
                self.free_record_in(args, &name, &mut st);
                return st;
            }
            if let Some(x) = args.first().and_then(|a| Self::named_var(&st, a)) {
                // The code generator emits no `OpFreeRef` of a never-free variable
                // (`Codegen::generate_call`, S34), including one a move made never-free.
                let dropped = name == "OpFreeRef"
                    && x < MEMBER_BASE
                    && (self.data.def(self.d_nr).variables.is_skip_free(x)
                        || self.moved_away.contains(&x));
                // A text free of a tuple member releases the text in it; a nested member's
                // record stores (`u.2` of `(s, w, (h, w))`) are their work-refs' to free.
                let text_of_member = d == ops.text_free && x >= MEMBER_BASE;
                if !dropped && !text_of_member {
                    self.end(&mut st, x, &name, true);
                }
            }
            return st;
        }
        if ops.conditional.contains(&d) {
            // A free that is a no-op where the freed variable and its witness name one store.
            let name = self.data.def(d).name().to_string();
            let mut st = st;
            // A capture SLOT freed unless the witness names its store.
            if self.free_slot_unless_witness(args, &name, &mut st) {
                return st;
            }
            if let Some(x) = Self::first_var(args, 0) {
                let w = Self::first_var(args, 1);
                let xp = Self::pts(&st, x);
                let (wp, field_slot) = self.guard_witness(&st, w, args.get(1));
                let shared: Vec<SKey> = Self::stores_of(&xp)
                    .into_iter()
                    .filter(|k| wp.contains(&Pt::Store(*k)))
                    .collect();
                // Every store `x` may name is one the witness names — a caller's store (a
                // parameter's) as much as one of this frame's: the free declines on every path.
                let declines = xp.iter().any(|p| *p != Pt::Null)
                    && xp
                        .iter()
                        .all(|p| *p == Pt::Null || (!matches!(p, Pt::Unknown) && wp.contains(p)))
                    && xp.iter().any(|p| matches!(p, Pt::Param(_)));
                if declines {
                    // Nothing ends: the caller's store stays the caller's.
                } else if shared.is_empty() {
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
                    // The stores the witness cannot name end as any free ends them.
                    let rest: BTreeSet<SKey> = Self::stores_of(&xp)
                        .into_iter()
                        .filter(|k| !shared.contains(k))
                        .collect();
                    if !rest.is_empty() {
                        self.end_only(&mut st, x, &name, false, Some(&rest));
                    }
                    if !witness_holds_it {
                        self.end_where_witness_does_not(
                            &mut st, x, &xp, w, field_slot, &shared, args, &name,
                        );
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
        // `@FR-R-HostStore`: a result its function drops is released by record — its store whole
        // where that is not the answer's, as `OpFreeRefIfDistinct(rec, answer)` frees it; a record
        // of the answer's store goes and the store lives on.
        if self.data.def(d).name() == "OpReleaseRecIfDistinct"
            && let Some(free_if) = ops
                .conditional
                .iter()
                .copied()
                .find(|c| self.data.def(*c).name() == "OpFreeRefIfDistinct")
        {
            return Some((
                self.free_op(free_if, &args[..args.len().min(2)], st),
                Val::null(),
            ));
        }
        if ops.unconditional.contains(&d) || d == ops.text_free || ops.conditional.contains(&d) {
            let mut st = self.free_op(d, args, st);
            if self.resets.contains(&(std::ptr::from_ref(node) as usize))
                && let Some(x) = Self::first_var(args, 0)
            {
                // The free's variable is reset to null right after it.  Where it had (maybe)
                // ended a store, the store keeps that fact under a placeholder name, so a
                // later free of the same store by another name still reads as a second one.
                for info in st.stores.values_mut() {
                    forget_holder(info, x);
                    if info.ended.remove(&x) {
                        info.ended.insert(FREED_NAME);
                    }
                }
                st.vars.remove(&x);
            }
            // Reset where the guard let the free run: `x` names only what the witness may name
            // (the guard declined there), or null.
            if self
                .cond_resets
                .contains(&(std::ptr::from_ref(node) as usize))
                && let (Some(x), Some(w)) = (Self::first_var(args, 0), Self::first_var(args, 1))
            {
                let wp = Self::pts(&st, w);
                let kept: Pts = Self::pts(&st, x)
                    .into_iter()
                    .filter(|p| matches!(p, Pt::Store(_) | Pt::Param(_)) && wp.contains(p))
                    .chain(std::iter::once(Pt::Null))
                    .collect();
                for (k, info) in &mut st.stores {
                    if !kept.contains(&Pt::Store(*k)) {
                        forget_holder(info, x);
                        if info.ended.remove(&x) {
                            info.ended.insert(FREED_NAME);
                        }
                    }
                }
                if kept.len() == 1 {
                    st.vars.remove(&x);
                } else {
                    st.vars.insert(x, kept);
                }
            }
            return Some((st, Val::null()));
        }
        // `OpFnRefDetachShared(old, new)` (loft#1609): the closure half of `old` is nulled where
        // it shares a store with `new` — the rebind or link write handed that closure on, so
        // the release of `old` that follows finds nothing.  Followed where the share is
        // certain: `old` names exactly the one store `new` names.  Elsewhere `old` keeps its
        // stores, which can only keep a finding, never invent one.
        if self.data.def(d).name() == "OpFnRefDetachShared"
            && let (Some(old), Some(new)) = (Self::first_var(args, 0), Self::first_var(args, 1))
            && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_DETACH_SHARED").is_some())
        {
            let held = Self::pts(&st, old);
            let named = Self::pts(&st, new);
            let mut s = st;
            let one = matches!(Self::stores_of(&held)[..], [k]
                if held.iter().all(|p| *p == Pt::Store(k))
                    && named.iter().all(|p| *p == Pt::Store(k))
                    && !named.is_empty());
            // …or several, where both names must-hold each of them (a copy `k = h` of a
            // fn-ref built in branches): wherever one of those stores is live, both name it.
            let same = !held.is_empty()
                && held == named
                && held.iter().all(|p| matches!(p, Pt::Store(_)))
                && Self::stores_of(&held).iter().all(|k| {
                    let info = info_of(&s, *k);
                    info.has(LIVE) && info.live.contains(&old) && info.live.contains(&new)
                });
            if one || same {
                self.bind(&mut s, old, &Val::null());
            }
            return Some((s, Val::null()));
        }
        // `OpInitRefSentinel(v)`: `v` is reset to null and names no store from here.
        if self.data.def(d).name() == "OpInitRefSentinel"
            && let Some(x) = Self::first_var(args, 0)
        {
            let mut s = st;
            self.bind(&mut s, x, &Val::null());
            return Some((s, Val::null()));
        }
        // `OpPutRef(dst, src)`: a raw copy of the reference — `dst` names exactly the stores
        // `src` names.  A never-free `dst` is a witness (an entry snapshot `__ref_1 = v`,
        // `ensure_rebind_witness`) and owns none of them; any other takes them over (a buffer
        // rotation `__ref_1 = __ref_2; __ref_2 = l`).
        if self.data.def(d).name() == "OpPutRef"
            && let Some(dst) = Self::first_var(args, 0)
            && let Some(src) = args.get(1)
        {
            let (mut s, value) = self.eval(src, st)?;
            let witness = dst < self.data.def(self.d_nr).variables.count()
                && self.data.def(self.d_nr).variables.is_skip_free(dst);
            let bound = if witness {
                Val {
                    pts: value.pts,
                    corr: BTreeSet::new(),
                }
            } else {
                value
            };
            self.bind(&mut s, dst, &bound);
            if !witness && let Some(sv) = Self::first_var(args, 1) {
                Self::copy_alts(&mut s, dst, sv);
            }
            return Some((s, Val::null()));
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
                    // A store this frame handed on (written back through a `&` parameter,
                    // into a field) still exists: the slot naming it is re-initialised in place.
                    Pt::Store(k) => {
                        let info = info_of(&st, *k);
                        (info.has(LIVE) || info.has(HANDED)) && !info.has(ENDED)
                    }
                    _ => false,
                })
                .collect();
            let mut st = st;
            // A loop pass that freed this site's store and reset `x` joins, at the head, a pass
            // that kept it: live where `x` names it, ended where `x` is null.  Re-initialised in
            // place on the first paths and minted on the others, `x` names a live store from
            // this site after either, and only `x` must-holds it.  The ended store a stale name
            // may still reach moves to the older generation.
            let here = self
                .site_of
                .get(&(std::ptr::from_ref(node) as usize))
                .copied();
            if let Some(site) = here
                && let [k] = Self::stores_of(&cur)[..]
                && k.site == site
                && !k.older
                && cur.iter().all(|p| matches!(p, Pt::Null | Pt::Store(_)))
                && !st.slots.contains_key(&k)
                && let info = info_of(&st, k)
                && info.has(LIVE)
                && info.has(ENDED)
                && !info.has(HANDED)
                && info.live.contains(&x)
                && info.captured_by.is_empty()
            {
                let old = SKey { site, older: true };
                let mut ended = info.ended.clone();
                ended.remove(&x);
                let part = SInfo {
                    status: info.status & !LIVE,
                    ended,
                    ..Default::default()
                };
                let merged = match st.stores.get(&old) {
                    Some(o) => join_info(o, &part),
                    None => part,
                };
                st.stores.insert(old, merged);
                for (&v, p) in &mut st.vars {
                    if v != x && p.contains(&Pt::Store(k)) {
                        p.insert(Pt::Store(old));
                    }
                }
                for p in st.points.values_mut() {
                    if p.contains(&Pt::Store(k)) {
                        p.insert(Pt::Store(old));
                    }
                }
                st.stores.insert(
                    k,
                    SInfo {
                        status: LIVE,
                        ..Default::default()
                    },
                );
                let val = Val {
                    pts: [Pt::Store(k)].into(),
                    corr: [k].into(),
                };
                self.bind(&mut st, x, &val);
                return Some((st, Val::null()));
            }
            let val = if !kept.is_empty() && kept.len() == cur.len() {
                Val {
                    corr: Self::stores_of(&kept).into_iter().collect(),
                    pts: kept,
                }
            } else if let Some(own) = {
                // A loop's head joins the null of the first pass with the store an earlier
                // pass made here: on the null path this `OpDatabase` makes that same store
                // (later passes find it live and re-init it in place), not another generation.
                let site = self
                    .site_of
                    .get(&(std::ptr::from_ref(node) as usize))
                    .copied();
                site.and_then(|site| {
                    kept.iter().find_map(|p| match p {
                        Pt::Store(k) if k.site == site && !k.older => Some(*k),
                        _ => None,
                    })
                })
            } {
                // A store of another site `x` must-holds is re-initialised in place where `x`
                // names it, so it stays correlated with `x` as well.
                let mut corr: BTreeSet<SKey> = Self::stores_of(&kept)
                    .into_iter()
                    .filter(|k| info_of(&st, *k).live.contains(&x))
                    .collect();
                corr.insert(own);
                Val { pts: kept, corr }
            } else {
                let origin = self
                    .params
                    .iter()
                    .position(|p| *p == x)
                    .map_or(Origin::Mint, |i| Origin::MintInto(i as u16));
                let site = self.site(node, origin);
                // A store that may also have ended (a loop that ran no pass joins the store an
                // earlier pass freed) is still `x`'s where it is live, when `x` names it wherever
                // it is live: re-initialised in place there, minted on the other paths.
                let mut kept = kept;
                for p in &cur {
                    if let Pt::Store(k) = p
                        && k.site != site
                        && let info = info_of(&st, *k)
                        && info.has(LIVE)
                        && info.live.contains(&x)
                    {
                        kept.insert(*p);
                    }
                }
                // A store `x` holds is re-initialised in place, so it stays live exactly where
                // `x` names it — as correlated with `x` as the store the null path makes.
                let mut corr: BTreeSet<SKey> = Self::stores_of(&kept)
                    .into_iter()
                    .filter(|k| info_of(&st, *k).live.contains(&x))
                    .collect();
                let k = Self::fresh(&mut st, site);
                corr.insert(k);
                let mut pts = kept;
                pts.insert(Pt::Store(k));
                Val { pts, corr }
            };
            // Compared with what `x` names NOW: a mint at the site of a store `x` named moved
            // that store to the older generation, so `x` names the older key while the new
            // store took the key `x` named before.
            if val.pts != Self::pts(&st, x) {
                self.bind(&mut st, x, &val);
            }
            return Some((st, Val::null()));
        }
        match self.data.def(d).name() {
            "n_protect_store_frees" => self.protected.extend(Self::first_var(args, 0)),
            "n_unprotect_store_frees" => {
                if let Some(v) = Self::first_var(args, 0)
                    && let Some(i) = self.protected.iter().rposition(|p| *p == v)
                {
                    self.protected.remove(i);
                }
            }
            _ => {}
        }
        let (mut s, vals) = if self.summaries.contains_key(&d) {
            self.user_call_args(args, st)?
        } else if matches!(
            self.data.def(d).name(),
            "OpEqRef" | "OpNeRef" | "OpStoreLive" | "OpRefIsNull" | "OpConvBoolFromRef"
        ) {
            // A reference COMPARISON reads the two references, never the stores they name:
            // comparing a freed store's reference with another is no use of it.  So are the
            // tests of a reference itself — null, naming a record — and the liveness test
            // (`@FR-L-CapOwn`'s guard), which exists to be asked of a store that may be freed.
            let mut s = st;
            let mut vals = Vec::with_capacity(args.len());
            for a in args {
                let (s2, value) = self.eval(a, s)?;
                vals.push(value);
                s = s2;
            }
            (s, vals)
        } else {
            self.args(args, st)?
        };
        // A copy that frees its source store (`COPY_FREE_SOURCE`, `OpSets::freed_source`):
        // the source's stores end here, except one the destination names or a bracket
        // protects, whose release the runtime declines.  A source that is no variable (a
        // join `if c { keep } else { mk() }`) frees what its value names.
        if let Some((src, dst)) = self.data.op_sets().freed_source(d, args) {
            let x = Self::first_var(args, src).or_else(|| args.get(src).and_then(Self::tail_var));
            let kept: BTreeSet<SKey> = vals
                .get(dst)
                .map(|v| v.pts.clone())
                .into_iter()
                .chain(self.protected.iter().map(|p| Self::pts(&s, *p)))
                .flat_map(|pts| Self::stores_of(&pts))
                .collect();
            let name = format!("{} (its source)", self.data.def(d).name());
            if let Some(x) = x {
                let freed: BTreeSet<SKey> = Self::stores_of(&Self::pts(&s, x))
                    .into_iter()
                    .filter(|k| !kept.contains(k))
                    .collect();
                if !freed.is_empty() {
                    self.end_only(&mut s, x, &name, true, Some(&freed));
                }
            } else if let Some(v) = vals.get(src) {
                // A store correlated with the value is live only where the value names it, so
                // the free ends it outright; any other only where the value may name it.
                for k in Self::stores_of(&v.pts) {
                    if kept.contains(&k) {
                        continue;
                    }
                    let pts = if v.corr.contains(&k) {
                        [Pt::Store(k)].into()
                    } else {
                        v.pts.clone()
                    };
                    s.vars.insert(FREED_NAME - 1, pts);
                    self.end_only(&mut s, FREED_NAME - 1, &name, true, Some(&[k].into()));
                }
                s.vars.remove(&(FREED_NAME - 1));
            }
        }
        let def = self.data.def(d);
        if def.name().ends_with("_OpDropAll")
            && let Some(w) = Self::first_var(args, 0)
            && Self::is_record(&s, w)
        {
            self.end_slots(&mut s, w, "its cascade");
        }
        // `OpDropFnRef(f)` runs the same cascade on the record the fn-ref names (@FR-L-CapOwn,
        // loft#1609): where the record's own release stood down (`___clos_N` reset because a
        // fn-ref still names it), this is where its captures end.
        // `LOFT_RECORDS_NO_DROP_FNREF_CASCADE=1` leaves them live.
        // A closure type whose cascade was retired (`Data::retire_def_name`: an earlier build
        // owns the capture) releases nothing, so only a record every one of whose possible
        // types keeps a live `_OpDropAll` is cascaded; a mix is left alone, which can only
        // keep a finding.
        if def.name() == "OpDropFnRef"
            && !self
                .standdown_drops
                .contains(&(std::ptr::from_ref(node) as usize))
            && let Some(f) = Self::first_var(args, 0)
            && Self::is_record(&s, f)
            && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_DROP_FNREF_CASCADE").is_some())
        {
            // The emitter's own split (`get_free_vars::fnref_stand_down`): while a local
            // closure-record variable the frame releases still names the record, that record's
            // release (its hook) is the cascade, and the fn-ref's drop is the one that stands
            // down.  A record the frame never releases itself (written into a fn-ref through a
            // link) ends by the fn-ref's drop alone.
            // And a record another name may still hold (`k = h`, then `return k`) is not this
            // drop's: the detach ahead of it (`OpFnRefDetachShared`) nulls `f` wherever the two
            // share, which the records may not have followed.  A link onto `f`'s variable is not
            // another holder, nor is a closure variable the frame never releases.
            let function = &self.data.def(self.d_nr).variables;
            let records: Vec<SKey> = Self::stores_of(&Self::pts(&s, f))
                .into_iter()
                .filter(|k| {
                    !s.vars.iter().any(|(&v, p)| {
                        v != f
                            && p.contains(&Pt::Store(*k))
                            && (v >= function.count()
                                || self.released_records.contains(&v)
                                || !(function.name(v).starts_with("___clos_")
                                    || matches!(
                                        function.tp(v).base(),
                                        crate::data::Type::RefVar(_)
                                    )))
                    })
                })
                .collect();
            let freed: Option<Vec<(SKey, BTreeSet<i32>)>> = records
                .iter()
                .map(|k| self.cascade_frees(*k).map(|offs| (*k, offs)))
                .collect();
            if let Some(freed) = freed {
                let name = self.var_name(f);
                for (k, offs) in freed {
                    self.end_record_slots(&mut s, k, &offs, &name);
                }
            }
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
                    self.hold(
                        &mut s,
                        w,
                        Self::const_offset(args.get(1)),
                        value,
                        Self::first_var(args, 2),
                    );
                }
                // A reference written into another store's field: that store holds it now —
                // unless a variable the program named still holds it, or it is the host's own
                // store.  A stored reference then only points (`reference<T>`), and the named
                // owner still releases the store (`u`'s child linked into `k.to`).
                // A capture into a record built inside another store (a fn field): the
                // record takes it over, and its slot still answers a guard that reads it.
                Some(w)
                    if self
                        .data
                        .def(self.d_nr)
                        .variables
                        .name(w)
                        .starts_with("___clos_") =>
                {
                    s.field_captures
                        .entry(w)
                        .or_default()
                        .extend(value.pts.iter().copied());
                    for k in Self::stores_of(&value.pts) {
                        if let Some(info) = s.stores.get_mut(&k) {
                            info.captured_by.insert(w);
                        }
                    }
                    Self::hand_on_var(&mut s, Self::first_var(args, 2), value);
                }
                _ => {
                    let function = &self.data.def(self.d_nr).variables;
                    let host: BTreeSet<SKey> = vals
                        .first()
                        .map(|h| Self::stores_of(&h.pts).into_iter().collect())
                        .unwrap_or_default();
                    // …or a compiler temporary the function frees: a stored reference only points
                    // (`@FR-H-RefHome`), and the temporary is the record's home.
                    let freed_temps = &self.freed_temps;
                    let owned_elsewhere = |k: &SKey| {
                        host.contains(k)
                            || info_of(&s, *k).live.iter().any(|w| {
                                *w < function.count()
                                    && (!function.is_compiler_generated(*w)
                                        || freed_temps.contains(w))
                            })
                    };
                    let handed = Val {
                        pts: value
                            .pts
                            .iter()
                            .copied()
                            .filter(|p| !matches!(p, Pt::Store(k) if owned_elsewhere(k)))
                            .collect(),
                        corr: value
                            .corr
                            .iter()
                            .copied()
                            .filter(|k| !owned_elsewhere(k))
                            .collect(),
                    };
                    // What the frame still owns, the holder only points at.
                    let pointed: Pts = value
                        .pts
                        .iter()
                        .copied()
                        .filter(|p| matches!(p, Pt::Store(k) if owned_elsewhere(k)))
                        .collect();
                    Self::hand_on(&mut s, &handed);
                    if let Some(h) = Self::first_var(args, 0) {
                        s.points.entry(h).or_default().extend(pointed);
                    }
                }
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
            let val = if d == ops.get_dbref {
                // A stored reference read answers its REFERENT, which lives in another store
                // than the record holding it (`cur = cur.next`): what the records know that
                // slot holds, or nothing they follow.
                let held = match Self::first_var(args, 0) {
                    Some(w) => {
                        let mut h = Self::held_by_record(&s, w, Self::const_offset(args.get(1)));
                        // A reference this frame wrote into the holder (`a.next = b`).
                        h.extend(s.points.get(&w).into_iter().flatten().copied());
                        h
                    }
                    None => Pts::new(),
                };
                if held.is_empty() {
                    Val::unknown()
                } else {
                    Val {
                        pts: held,
                        corr: BTreeSet::new(),
                    }
                }
            } else if def.name() == "OpStoreBufOnly" {
                // `@FR-R-HostStore`: the buffer where it is a store buffer, else null.
                let mut v = vals.into_iter().next().unwrap_or_else(Val::unknown);
                v.pts.insert(Pt::Null);
                v.corr.clear();
                v
            } else if def.name() == "OpNullRefSentinel" {
                // The null reference: no store.  Read as an operator answering into its
                // (absent) first argument it was unknown, and every `-> S?` callee whose null
                // arm spells it answered Unknown instead of Fresh.
                Val::null()
            } else if def.name() == "OpCopyClosure" {
                // A fn-ref copied out of a field is a copy (`(L-CapKeep)`): the closure record
                // is copied into a store of its own, which the copy's holder frees.
                let site = self.site(node, Origin::Copy);
                let k = Self::fresh(&mut s, site);
                Val {
                    pts: [Pt::Store(k)].into(),
                    corr: [k].into(),
                }
            } else if def.name().starts_with("Op")
                && def
                    .attributes
                    .first()
                    .is_some_and(|a| matches!(a.typedef.base(), crate::data::Type::Text(_)))
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
                // A `-> T?` callee may answer null instead: the store is live only where the
                // answer names it.
                let mut pts: Pts = [Pt::Store(k)].into();
                if matches!(def.returned(), crate::data::Type::Optional(_)) {
                    pts.insert(Pt::Null);
                }
                Val {
                    pts,
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
            // `@FR-R-HostStore`: a callee handed a STORE buffer (`OpStoreBuf(host)`, never null)
            // claims every answer in that store, so it answers `host`'s store and mints none.
            Summary::Join(i) | Summary::Buf(i)
                if args.get(i as usize).is_some_and(|a| {
                    matches!(a.unspan(), Value::Call(sb, _)
                        if self.data.def(*sb).name() == "OpStoreBuf")
                }) && vals.get(i as usize).is_some_and(|v| !v.pts.is_empty()) =>
            {
                let pts: Pts = vals[i as usize]
                    .pts
                    .iter()
                    .copied()
                    .filter(|p| *p != Pt::Null)
                    .collect();
                Val {
                    pts,
                    corr: BTreeSet::new(),
                }
            }
            // A buffer-filler handed "null, or the store this same call made on an earlier
            // pass" (a loop's head): the null path makes that store's role again, so the call
            // answers that store — not one more generation of it.
            Summary::Buf(i)
                if vals.get(i as usize).is_some_and(|v| {
                    let site = self
                        .site_of
                        .get(&(std::ptr::from_ref(node) as usize))
                        .copied();
                    let stores = Self::stores_of(&v.pts);
                    !stores.is_empty()
                        && stores.iter().all(|k| Some(k.site) == site && !k.older)
                        && v.pts.iter().all(|p| matches!(p, Pt::Null | Pt::Store(_)))
                }) =>
            {
                let v = &vals[i as usize];
                let pts: Pts = v.pts.iter().copied().filter(|p| *p != Pt::Null).collect();
                Val {
                    corr: Self::stores_of(&pts).into_iter().collect(),
                    pts,
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
                    !v.pts.is_empty()
                        && v.pts
                            .iter()
                            .all(|p| matches!(p, Pt::Store(_) | Pt::Param(_)))
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
                // A null argument is the path the callee mints on: it answers the fresh store
                // there, never null — unless its answer is `-> T?` itself.
                let mut pts = viewed.pts;
                pts.remove(&Pt::Null);
                if matches!(def.returned(), crate::data::Type::Optional(_)) {
                    pts.insert(Pt::Null);
                }
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
        in_field_reset: 0,
        traced: {
            static TRACE: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
            TRACE
                .get_or_init(|| std::env::var("LOFT_STORE_TRACE").ok())
                .as_deref()
                .is_some_and(|f| def.name().contains(f))
        },
        steps: 0,
        loops: Vec::new(),
        params,
        first_sets: BTreeSet::new(),
        moved_away: BTreeSet::new(),
        tuple_target: None,
        rebinds: BTreeMap::new(),
        resets: BTreeSet::new(),
        cond_resets: BTreeSet::new(),
        standdown_drops: BTreeSet::new(),
        released_records: BTreeSet::new(),
        fn_record: BTreeMap::new(),
        leaving: BTreeSet::new(),
        protected: Vec::new(),
        freed_temps: BTreeSet::new(),
        member_tuple: false,
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
                if *op == unconditional && vars.is_compiler_generated(*x) {
                    run.freed_temps.insert(*x);
                }
            }
            // …and after a guarded free of one variable against another, where the free ran
            // (`LOFT_NO_GUARDED_FREE_RESET`, the twin of `Codegen::generate_call`'s test).
            Value::Call(op, args)
                if *op == if_distinct
                    && matches!(args.first().map(Value::unspan), Some(Value::Var(_)))
                    && matches!(args.get(1).map(Value::unspan), Some(Value::Var(_)))
                    && !crate::keys::guarded_free_reset_off() =>
            {
                run.cond_resets.insert(at);
            }
            _ => {}
        }
    });
    standdown_drops(data, &body, &mut Vec::new(), &mut run.standdown_drops);
    released_records(data, vars, &body, &mut run.released_records);
    closure_records(data, vars, &body, &mut run.fn_record, &mut run.leaving);
    if let Some((mut s, value)) = run.eval(&body, st) {
        run.answer = run.answer.join(run.summary_of(&value.pts));
        run.leaving_captures(&s, &value);
        Run::hand_on(&mut s, &value);
        run.exit(&s, "the end of the function");
    }
    run
}

/// The closure records a body builds: per fn-ref variable the record its build names, and the
/// records built inside another store (`w = OpChildRec(…)`, a fn field), whose host decides
/// their life.  A record the function returns is not among them: it holds its captures until
/// it leaves, and `hand_on` hands them on with it.
/// The closure-record variables (`___clos_N`) the body releases itself (`Run::released_records`).
fn released_records(
    data: &Data,
    vars: &crate::variables::Function,
    body: &Value,
    out: &mut BTreeSet<u16>,
) {
    let free_op = data.def_nr("OpFreeRef");
    body.walk(&mut |n| {
        if let Value::Call(op, a) = n.unspan()
            && *op == free_op
            && let Some(Value::Var(v)) = a.first().map(Value::unspan)
            && *v < vars.count()
            && vars.name(*v).starts_with("___clos_")
        {
            out.insert(*v);
        }
    });
}

/// Collect the `OpDropFnRef(f)` nodes under an `if OpDistinctStore(OpFnRefClosure(f), …)`
/// guard on the same `f` — the emitter's stand-down release (`Run::standdown_drops`).  Every
/// node of a guarded arm is recorded under its own address and, where wrapped in a span, the
/// span's, as the walk that reads them meets either.
fn standdown_drops(data: &Data, node: &Value, guarded: &mut Vec<u16>, out: &mut BTreeSet<usize>) {
    let (distinct, closure, drop) = (
        data.def_nr("OpDistinctStore"),
        data.def_nr("OpFnRefClosure"),
        data.def_nr("OpDropFnRef"),
    );
    let guard = |c: &Value| -> Option<u16> {
        if let Value::Call(op, a) = c.unspan()
            && *op == distinct
            && let Some(Value::Call(cl, b)) = a.first().map(Value::unspan)
            && *cl == closure
            && let Some(Value::Var(f)) = b.first().map(Value::unspan)
        {
            Some(*f)
        } else {
            None
        }
    };
    match node.unspan() {
        Value::If(c, t, e) => {
            standdown_drops(data, c, guarded, out);
            if let Some(f) = guard(c) {
                guarded.push(f);
                standdown_drops(data, t, guarded, out);
                guarded.pop();
            } else {
                standdown_drops(data, t, guarded, out);
            }
            standdown_drops(data, e, guarded, out);
        }
        Value::Call(op, a)
            if *op == drop
                && matches!(a.first().map(Value::unspan), Some(Value::Var(f)) if guarded.contains(f)) =>
        {
            out.insert(std::ptr::from_ref(node) as usize);
            out.insert(std::ptr::from_ref(node.unspan()) as usize);
        }
        _ => node.for_each_child(&mut |c| standdown_drops(data, c, guarded, out)),
    }
}

fn closure_records(
    data: &Data,
    vars: &crate::variables::Function,
    body: &Value,
    fn_record: &mut BTreeMap<u16, u16>,
    leaving: &mut BTreeSet<u16>,
) {
    let child_rec = data.def_nr("OpChildRec");
    body.walk(&mut |n| {
        if let Value::Set(g, rhs) = n {
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
    });
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

/// Does a vector or keyed local's first null init mint a store (`bind_lowering`'s answer)?
fn null_init_mints(
    arm: crate::bind_lowering::FirstArm,
    function: &crate::variables::Function,
    x: u16,
) -> bool {
    match arm {
        crate::bind_lowering::FirstArm::VectorNull => {
            crate::bind_lowering::vector_null_mints(function, x)
        }
        crate::bind_lowering::FirstArm::KeyedNull => {
            crate::bind_lowering::keyed_null_mints(function, x)
        }
        _ => false,
    }
}
