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

mod call;
mod decide;
mod ends;
mod eval;

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
    /// The MAY twin of `ended`: the variables that may point into it on SOME path where it is
    /// ended (`design-stale-names.md`).  In `ended`: a certain second end; in `stale` only: a
    /// possible one; in neither, with the store `LIVE|ENDED`: no finding — the name was rebound
    /// or reset on the paths that ended it.  `None` is every name (unknown: a store ended where
    /// this is not followed), which never silences a finding.
    stale: Option<BTreeSet<u16>>,
    /// The variables whose value, naming this store, was handed on (returned, captured by a
    /// closure that leaves) and that have not been rebound since.  Where one of them is also
    /// in `live`, the store is handed on wherever it is live.
    handed_by: BTreeSet<u16>,
    /// Closure-record variables of which at least one names a record on every path where
    /// it is live (one record per arm of a branch; the frame's "free the capture where no
    /// record was built" leaves it live only there).  A test finding one of them null drops
    /// it; the last one dropped finds this store not live.  Empty: nothing known.
    live_some: BTreeSet<u16>,
    /// The cover: on every path where the key is live, every live store under it is named by
    /// one of these variables (a key standing for several stores — an older generation — too).
    /// A bind adds its name, a rebind drops it, and a holder the cover cannot list (a capture
    /// slot, a record field, a hand-over) makes it `None`, unknown.  A certain free that empties
    /// it leaves the key not live: the names that held it are all freed (design-cover.md).
    cover: Option<BTreeSet<u16>>,
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
    /// Per site, how many stores it minted on this path: a join renames the side that minted
    /// fewer, so a site's current key names one generation on both sides.
    mints: Mints,
    /// Pairs of variables one a plain copy of the other (`k = h`, smaller number first): the
    /// same value until either is bound again, so a test that narrows one narrows both.
    copies: BTreeSet<(u16, u16)>,
    /// `(x, w)`: `x` names the store `w` names, or none of the stores `w` may name — `x` bound
    /// from a projection rooted at `w` (`x = __lift.code`), kept across a join whose other side
    /// has `w` null or `x` apart from `w`.  An identity test of the two then settles which side they are on: distinct
    /// only where `w` is null (`OpDistinctStore(x, __lift)` guarding `x`'s free).
    views: BTreeSet<(u16, u16)>,
    /// The function's closure-record variables (`___clos_N`), the same on every path: the
    /// `live_some` candidates of a store no record captures (`live_some_candidates`).
    records: BTreeSet<u16>,
}

/// A site's mint count per path ([`State::mints`]).  Only the ORDER of two sides' counts at a
/// join is read, so two states equal in everything else are equal: a loop's head converges
/// although every pass counts on.
#[derive(Clone, Debug, Default)]
struct Mints(BTreeMap<u32, u32>);

impl PartialEq for Mints {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Mints {}

/// `LOFT_RECORDS_NO_GENERATION_ALIGN=1`: a join takes each side's keys as they are.
fn no_generation_align() -> bool {
    crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_GENERATION_ALIGN").is_some())
}

/// Join two paths after renaming, per site, the side that minted fewer stores there: its
/// current store is the other side's older generation (one arm minted, the other did not).
fn align_generations(a: &State, b: &State) -> Option<(State, State)> {
    if no_generation_align() {
        return None;
    }
    let sites: BTreeSet<u32> = a.mints.0.keys().chain(b.mints.0.keys()).copied().collect();
    let behind = |x: &State, y: &State| -> Vec<u32> {
        sites
            .iter()
            .filter(|s| {
                x.mints.0.get(s).copied().unwrap_or(0) < y.mints.0.get(s).copied().unwrap_or(0)
            })
            .copied()
            .collect()
    };
    let (ra, rb) = (behind(a, b), behind(b, a));
    if ra.is_empty() && rb.is_empty() {
        return None;
    }
    let mut a = a.clone();
    let mut b = b.clone();
    for site in ra {
        demote(&mut a, site);
        a.mints.0.insert(site, b.mints.0[&site]);
    }
    for site in rb {
        demote(&mut b, site);
        b.mints.0.insert(site, a.mints.0[&site]);
    }
    Some((a, b))
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

/// `stale` for store `k` as it ends now: the variables naming it, joined to what an earlier
/// end on another path left (`None` stays `None`).
fn stale_on_end(st: &State, k: SKey, info: &SInfo) -> Option<BTreeSet<u16>> {
    let holders: BTreeSet<u16> = st
        .vars
        .iter()
        .filter(|(_, p)| p.contains(&Pt::Store(k)))
        .map(|(v, _)| *v)
        .collect();
    if info.has(ENDED) {
        info.stale.clone().map(|mut s| {
            s.extend(holders);
            s
        })
    } else {
        Some(holders)
    }
}

/// May `v` point into the ended store `info` on a path where it is ended?  The analysis's own
/// placeholder names always may; `LOFT_RECORDS_NO_STALE_NAMES=1` answers yes for every name.
fn maybe_stale(info: &SInfo, v: u16) -> bool {
    v >= OLD_NAME
        || crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_STALE_NAMES").is_some())
        || info.stale.as_ref().is_none_or(|s| s.contains(&v))
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

/// `v` stops naming the store: it is not a must-holder, nor in any alternative (one left
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

/// `LOFT_RECORDS_NO_FIELD_RECORD_HOLDS=1`: a fn field's record is no holder of a store its slot
/// was found to name, and leaves nothing with its host (the records' bisect switch).
fn field_record_holds() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_RECORDS_NO_FIELD_RECORD_HOLDS").is_none())
}

/// `LOFT_RECORDS_NO_COPY_ALIAS=1`: a plain copy `k = h` is no alias a narrowing follows (the
/// records' bisect switch).
fn copy_alias() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_RECORDS_NO_COPY_ALIAS").is_none())
}

/// `LOFT_RECORDS_NO_SLOT_NULL=1`: a null written into a capture slot leaves the slot as it was,
/// and a presence test of a slot decides nothing (the records' bisect switch).
fn slot_null() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_RECORDS_NO_SLOT_NULL").is_none())
}

/// The closure-record variables a join asks whether they are non-null wherever store `info`
/// is live (`SInfo::live_some`): those that captured it, or — a store no record captures, one
/// minted only on the runs that also build a record (a loop's pass, L-CapKeep/2357 `record`) —
/// every closure record of the function.  The one seam for the candidates.
fn live_some_candidates(info: &SInfo, st: &State) -> BTreeSet<u16> {
    if info.captured_by.is_empty() && live_some_uncaptured() {
        st.records.clone()
    } else {
        info.captured_by.clone()
    }
}

/// `LOFT_RECORDS_NO_LIVE_SOME_UNCAPTURED=1`: a store no record captures carries no
/// `live_some` (the records' bisect switch).
fn live_some_uncaptured() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_RECORDS_NO_LIVE_SOME_UNCAPTURED").is_none())
}

/// `LOFT_RECORDS_NO_SLOT_SOLE_HOLDER=1`: a store held only in a leaving record's slot is handed
/// on there only where the slot's correlation says so (the records' bisect switch).
fn slot_sole_holder() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_RECORDS_NO_SLOT_SOLE_HOLDER").is_none())
}

/// `LOFT_RECORDS_NO_IDENTITY_TEST_NO_USE=1`: an operand of a store identity test is a use of
/// its store (the records' bisect switch).
fn identity_test_no_use() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_RECORDS_NO_IDENTITY_TEST_NO_USE").is_none())
}

/// `LOFT_RECORDS_NO_COVER=1`: no key keeps a cover, so a free never finds its last holder gone.
fn cover_on() -> bool {
    !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_COVER").is_some())
}

/// The stores in `pts` gain a holder the cover does not name (a capture slot, a record field,
/// a hand-over): their cover is unknown from here on.
fn uncover_all(st: &mut State, pts: &Pts) {
    for p in pts {
        if let Pt::Store(k) = p
            && let Some(info) = st.stores.get_mut(k)
        {
            info.cover = None;
        }
    }
}

/// `LOFT_RECORDS_NO_GUARDED_REST_REPORTS=1`: a guarded free's stores its witness cannot name
/// end silently (the records' bisect switch).
fn guarded_rest_reports() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_RECORDS_NO_GUARDED_REST_REPORTS").is_none())
}

/// `LOFT_RECORDS_NO_SLOT_DEAD_ARM=1`: the dead arm of a slot liveness test leaves the store the
/// slot names as it was (the records' bisect switch).
fn slot_dead_arm() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_RECORDS_NO_SLOT_DEAD_ARM").is_none())
}

/// `LOFT_RECORDS_NO_SLOT_CORR_JOIN=1`: a slot's correlation is intersected at a join and
/// dropped at a re-mint of its store's site (the records' bisect switch).
fn slot_corr_join() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_RECORDS_NO_SLOT_CORR_JOIN").is_none())
}

/// `LOFT_RECORDS_NO_SLOT_CORR_NARROW=1`: a store narrowed out of a capture slot keeps its
/// liveness, and a slot is narrowed only against one store (the records' bisect switch).
fn slot_corr_narrow() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_RECORDS_NO_SLOT_CORR_NARROW").is_none())
}

/// `LOFT_RECORDS_NO_KEEP_DROP_CASCADE=1`: a keep temporary's guarded `OpDropFnRef` cascades
/// nothing, as the emitter's stand-down drop (the records' bisect switch).
fn keep_drop_cascade() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("LOFT_RECORDS_NO_KEEP_DROP_CASCADE").is_none())
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
        stale: match (a.has(ENDED), b.has(ENDED)) {
            (true, true) => match (&a.stale, &b.stale) {
                (Some(x), Some(y)) => Some(x.union(y).copied().collect()),
                _ => None,
            },
            (true, false) => a.stale.clone(),
            (false, true) => b.stale.clone(),
            (false, false) => None,
        },
        handed_by: meet(&a.handed_by, a.has(LIVE), &b.handed_by, b.has(LIVE)),
        cover: match (a.has(LIVE), b.has(LIVE)) {
            (true, true) => match (&a.cover, &b.cover) {
                (Some(x), Some(y)) => Some(x.union(y).copied().collect()),
                _ => None,
            },
            (true, false) => a.cover.clone(),
            (false, true) => b.cover.clone(),
            (false, false) => None,
        },
        live_some: BTreeSet::new(),
        live_null: BTreeSet::new(),
        captured_by: a.captured_by.union(&b.captured_by).copied().collect(),
    }
}

/// Site `site`'s current store becomes the older generation (joined with any already there):
/// every name of it is renamed, and a record's slots go with it.  The site then has no
/// current store until its next mint.
fn demote(st: &mut State, site: u32) {
    let cur = SKey { site, older: false };
    let old = SKey { site, older: true };
    if let Some(info) = st.stores.remove(&cur) {
        // No earlier generation live here: a slot correlated with the current store stays
        // correlated with the older key it becomes.
        let old_quiet = !info_of(st, old).has(LIVE);
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
        // A fn field's record captured the store this site named at its build: that is
        // the older generation now, as for a variable.
        for p in st.field_captures.values_mut() {
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
                if v.corr.remove(&cur) && slot_corr_join() && (old_quiet || v.corr.contains(&old)) {
                    v.corr.insert(old);
                } else if !old_quiet {
                    v.corr.remove(&old);
                }
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
}

/// Join the two arms of one branch.  They left the same state, so the arm that minted more at
/// a site holds its newer store under the current key: the other arm's current store is renamed
/// older first.  (Two passes of a loop, or two exits, need no renaming: on each path the current
/// key already names the latest store, and renaming the earlier pass's would name a record that
/// path already released.)
fn join_arms(a: &State, b: &State) -> State {
    if let Some((a, b)) = align_generations(a, b) {
        return join_state(&a, &b);
    }
    join_state(a, b)
}

/// A copy pair survives a join where, on each side, it was kept or both names are null there:
/// two nulls are the same value (`LOFT_RECORDS_NO_NULL_COPY_JOIN=1` keeps only kept pairs).
fn join_copies(a: &State, b: &State) -> BTreeSet<(u16, u16)> {
    let null = |st: &State, v: u16| {
        st.vars
            .get(&v)
            .is_none_or(|p| p.iter().all(|x| *x == Pt::Null))
    };
    let holds = |st: &State, p: &(u16, u16)| {
        st.copies.contains(p)
            || (!crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_NULL_COPY_JOIN").is_some())
                && null(st, p.0)
                && null(st, p.1))
    };
    a.copies
        .union(&b.copies)
        .filter(|p| holds(a, p) && holds(b, p))
        .copied()
        .collect()
}

/// The stores `v` may name on a path.
fn stores_named(st: &State, v: u16) -> Vec<SKey> {
    st.vars
        .get(&v)
        .map(|p| {
            p.iter()
                .filter_map(|x| if let Pt::Store(k) = x { Some(*k) } else { None })
                .collect()
        })
        .unwrap_or_default()
}

/// A view pair survives a join where, on each side, it was kept, its base is null there, or
/// the view names none of the base's stores there.
fn join_views(a: &State, b: &State) -> BTreeSet<(u16, u16)> {
    let null = |st: &State, v: u16| {
        st.vars
            .get(&v)
            .is_none_or(|p| p.iter().all(|x| *x == Pt::Null))
    };
    // The pair's claim, as the identity test reads it: `x` names `w`'s store, or none of the
    // stores `w` may name.  A side whose `x` shares no store with `w` (`x` rebound to a fresh
    // answer there) keeps it, as does a side where `w` is null.
    let disjoint = |st: &State, p: &(u16, u16)| {
        let w = stores_named(st, p.1);
        stores_named(st, p.0).iter().all(|k| !w.contains(k))
    };
    let holds =
        |st: &State, p: &(u16, u16)| st.views.contains(p) || null(st, p.1) || disjoint(st, p);
    a.views
        .union(&b.views)
        .filter(|p| holds(a, p) && holds(b, p))
        .copied()
        .collect()
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
            let candidates = live_some_candidates(&info, a);
            let side = |st: &State, i: &SInfo| -> Option<BTreeSet<u16>> {
                if !i.has(LIVE) {
                    None
                } else if !i.live_some.is_empty() {
                    Some(i.live_some.clone())
                } else {
                    Some(
                        candidates
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
        copies: join_copies(a, b),
        views: join_views(a, b),
        records: a.records.union(&b.records).copied().collect(),
        mints: a.mints.clone(),
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
    // A record that does not exist on one side (never minted on that path) holds nothing there:
    // that side's slot map is stale and does not widen the other side's.
    let exists = |st: &State, k: &SKey| info_of(st, *k).status & (LIVE | ENDED) != 0;
    let mut slots = a.slots.clone();
    slots.retain(|k, _| exists(a, k) || !b.slots.contains_key(k) || !exists(b, k));
    for (k, m) in &b.slots {
        if !exists(b, k) && a.slots.contains_key(k) && exists(a, k) {
            continue;
        }
        let Some(into) = slots.get_mut(k) else {
            slots.insert(*k, m.clone());
            continue;
        };
        for (off, v) in m {
            match into.get_mut(off) {
                Some(x) => {
                    x.pts.extend(v.pts.iter().copied());
                    // A store stays correlated with the slot where, on each side, it is
                    // correlated there or not live (as for a value, `join_out`).
                    x.corr = if slot_corr_join() {
                        x.corr
                            .union(&v.corr)
                            .copied()
                            .filter(|c| {
                                (x.corr.contains(c) || !info_of(a, *c).has(LIVE))
                                    && (v.corr.contains(c) || !info_of(b, *c).has(LIVE))
                            })
                            .collect()
                    } else {
                        x.corr.intersection(&v.corr).copied().collect()
                    };
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
fn join_opt_arms(a: Option<State>, b: Option<State>) -> Option<State> {
    match (a, b) {
        (None, x) | (x, None) => x,
        (Some(a), Some(b)) => Some(join_arms(&a, &b)),
    }
}

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
            Some((join_arms(&sa, &sb), Val { pts, corr }))
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
    copy_record: u32,
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
            copy_record: data.def_nr("OpCopyRecord"),
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

/// The passes of a loop analysed one by one, each from the state the one before left, before
/// the head joins them (`LOFT_RECORDS_PEEL`; 1 is the old single first pass).  A defect that
/// needs a store from an earlier pass to meet one from a later pass (a buffer rotated through
/// two passes, freed by the third) shows only on a pass the head has not yet joined.
fn peeled_passes() -> usize {
    static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
    *N.get_or_init(|| {
        std::env::var("LOFT_RECORDS_PEEL")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3)
            .max(1)
    })
}

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
    /// Each finding with the source line of the statement where it was first seen (0: before
    /// any line marker).  The line is no part of the finding's identity, so a finding seen again
    /// on a later pass or path stays one.
    findings: BTreeMap<Finding, u32>,
    /// The line of the last `Line` marker the walk passed.
    line: u32,
    /// Where each store first ended: its line and the free that ended it, for the re-free and
    /// use-after-end findings to name.  Beside the state, not in it: a fact about the walk.
    end_lines: BTreeMap<SKey, (u32, String)>,
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
    /// The call a code generator brackets around the call itself (`LOFT_NO_PROTECT_AFTER_CALL`),
    /// by node, with the stores its reference arguments held before it: a release inside the
    /// callee declines them.
    call_bracket: Option<(usize, BTreeSet<SKey>)>,
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

    /// ` (ended at line L by its free)` for a store whose first end the walk saw.
    fn ended_note(&self, k: SKey) -> String {
        self.end_lines
            .get(&k)
            .map(|(line, by)| {
                if *line == 0 {
                    format!(" (ended by {by})")
                } else {
                    format!(" (ended at line {line} by {by})")
                }
            })
            .unwrap_or_default()
    }

    fn find(&mut self, kind: &'static str, site: u32, detail: String) {
        let line = self.line;
        self.findings
            .entry(Finding { kind, site, detail })
            .or_insert(line);
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
        st.copies.retain(|&(a, b)| a != v && b != v);
        st.views.retain(|&(a, b)| a != v && b != v);
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
            if let Some(c) = info.cover.as_mut() {
                c.remove(&v);
            }
            info.ended.remove(&v);
            if let Some(sn) = info.stale.as_mut() {
                sn.remove(&v);
            }
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
                if let Some(c) = info.cover.as_mut() {
                    c.insert(v);
                }
                if (single || val.corr.contains(&k)) && info.status & LIVE != 0 {
                    info.live.insert(v);
                    for a in &mut info.alts {
                        a.insert(v);
                    }
                }
                if single && info.status & ENDED != 0 {
                    info.ended.insert(v);
                }
                // An alias made after the end points into it wherever it is ended.
                if info.status & ENDED != 0
                    && let Some(sn) = info.stale.as_mut()
                {
                    sn.insert(v);
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
        demote(st, site);
        *st.mints.0.entry(site).or_default() += 1;
        st.stores.insert(
            cur,
            SInfo {
                status: LIVE,
                cover: cover_on().then(BTreeSet::new),
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
            // …or a name that MAY still point into a store ended on every path: on the path where
            // it does, the read finds the freed store (`bl` viewing `b`, read by `b`'s own rebind
            // after its displaced free).  Only a KNOWN stale set counts here — an unknown one
            // names every variable.  `LOFT_RECORDS_NO_STALE_READS=1` reads `ended` alone.
            let stale_read = !info.has(LIVE)
                && info.stale.as_ref().is_some_and(|s| s.contains(&v))
                && !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_STALE_READS").is_some());
            if info.has(ENDED) && (info.ended.contains(&v) || stale_read) {
                let name = self.var_name(v);
                self.find(
                    "use-after-end",
                    k.site,
                    format!(
                        "`{name}` is read by {how} after its store was freed{}",
                        self.ended_note(k)
                    ),
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
    st.records = (0..def.variables.count())
        .filter(|&v| def.variables.name(v).starts_with("___clos_"))
        .collect();
    let mut run = Run {
        data,
        ops,
        summaries,
        d_nr,
        site_of: BTreeMap::new(),
        sites: Vec::new(),
        findings: BTreeMap::new(),
        line: 0,
        end_lines: BTreeMap::new(),
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
        call_bracket: None,
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
    standdown_drops(data, vars, &body, &mut Vec::new(), &mut run.standdown_drops);
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
/// A keep temporary's drop (`__fkeep_N`, `closure_keep`) is not one: it ends an earlier record a
/// fn-ref kept, whose own release stood down, so its cascade is where that record's captures end
/// (`LOFT_RECORDS_NO_KEEP_DROP_CASCADE=1` counts it as a stand-down).
fn standdown_drops(
    data: &Data,
    vars: &crate::variables::Function,
    node: &Value,
    guarded: &mut Vec<u16>,
    out: &mut BTreeSet<usize>,
) {
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
            && !(keep_drop_cascade() && *f < vars.count() && vars.name(*f).starts_with("__fkeep_"))
        {
            Some(*f)
        } else {
            None
        }
    };
    match node.unspan() {
        Value::If(c, t, e) => {
            standdown_drops(data, vars, c, guarded, out);
            if let Some(f) = guard(c) {
                guarded.push(f);
                standdown_drops(data, vars, t, guarded, out);
                guarded.pop();
            } else {
                standdown_drops(data, vars, t, guarded, out);
            }
            standdown_drops(data, vars, e, guarded, out);
        }
        Value::Call(op, a)
            if *op == drop
                && matches!(a.first().map(Value::unspan), Some(Value::Var(f)) if guarded.contains(f)) =>
        {
            out.insert(std::ptr::from_ref(node) as usize);
            out.insert(std::ptr::from_ref(node.unspan()) as usize);
        }
        _ => node.for_each_child(&mut |c| standdown_drops(data, vars, c, guarded, out)),
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
/// `LOFT_RECORDS_SET_DBREF_VALUE_USE=1` — the value an `OpSetDbRef` writes into a slot counts as a
/// use of its store again.
fn set_dbref_value_use() -> bool {
    crate::env_once!(std::env::var_os("LOFT_RECORDS_SET_DBREF_VALUE_USE").is_some())
}

/// `LOFT_RECORDS_BRACKET_EVERY_BIND=1` — the records read a call bracket at every first bind and
/// rebind of a call, not only at the copy arms the interpreter brackets.
fn bracket_every_bind() -> bool {
    crate::env_once!(std::env::var_os("LOFT_RECORDS_BRACKET_EVERY_BIND").is_some())
}

/// ` (line N)` after a finding seen at a source line; nothing before the first marker.
fn line_note(line: u32) -> String {
    if line == 0 {
        String::new()
    } else {
        format!(" (line {line})")
    }
}

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
                        .map(|(f, line)| {
                            format!(
                                "store-{} {}{}{}",
                                f.kind,
                                site_label(&run, f.site),
                                f.detail,
                                line_note(*line)
                            )
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
    for (f, line) in &run.findings {
        eprintln!(
            "[stores]   {} {}{}{}",
            f.kind,
            site_label(run, f.site),
            f.detail,
            line_note(*line)
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

/// The records driven from a hand-built function body: a shape no source spelling reaches
/// (no loft source frees) can still be checked to be reported, and each rule gets a unit test.
/// The view relation (`State::views`).  `LOFT_RECORDS_NO_VIEW_PAIRS=1` leaves it out.
fn views_enabled() -> bool {
    !crate::env_once!(std::env::var_os("LOFT_RECORDS_NO_VIEW_PAIRS").is_some())
}

/// The paths a value can end on, as `(arm, tail)`: the arms of an `if`, and in a block its last
/// statement, through spans.  `arm` is the whole arm, for asking what that path sets.
fn value_tails<'v>(data: &Data, v: &'v Value, out: &mut Vec<(&'v Value, &'v Value)>) {
    // A block's value is its last statement that is not a free: the frees the scan appends
    // after the value answer nothing (`{ __lift = f(); __lift.x; OpFreeRef(__lift_1) }`).
    fn is_free(data: &Data, op: &Value) -> bool {
        matches!(op.unspan(), Value::Call(d, _)
            if matches!(data.def(*d).name(),
                "OpFreeRef" | "OpFreeRefIfDistinct" | "OpFreeRefUnlessEntry" | "OpFreeText"))
    }
    fn last_value<'v>(data: &Data, ops: &'v [Value]) -> Option<&'v Value> {
        ops.iter().rev().find(|op| !is_free(data, op))
    }
    fn tail<'v>(data: &Data, v: &'v Value) -> &'v Value {
        match v.unspan() {
            Value::Block(bl) if !bl.operators.is_empty() => {
                last_value(data, &bl.operators).map_or(v.unspan(), |l| tail(data, l))
            }
            other => other,
        }
    }
    match v.unspan() {
        Value::If(_, t, e) => {
            value_tails(data, t, out);
            value_tails(data, e, out);
        }
        Value::Block(bl) if !bl.operators.is_empty() => {
            let last =
                last_value(data, &bl.operators).unwrap_or_else(|| bl.operators.last().unwrap());
            if matches!(last.unspan(), Value::If(..)) {
                value_tails(data, last, out);
            } else {
                out.push((v, tail(data, v)));
            }
        }
        _ => out.push((v, v)),
    }
}

/// Does `v` assign `w` anywhere inside it?
fn sets_var(v: &Value, w: u16) -> bool {
    let mut found = false;
    v.walk(&mut |n| {
        if matches!(n, Value::Set(x, _) if *x == w) {
            found = true;
        }
    });
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{Type, v_block, v_if};

    /// Parse `source` beside the stdlib, run the scope pass, replace `fn_name`'s body by what
    /// `body` builds from (the data, the function's number), and answer the records' findings
    /// for it.
    fn findings(
        source: &str,
        fn_name: &str,
        body: impl FnOnce(&Data, u32) -> Value,
    ) -> Vec<String> {
        let mut p = crate::parser::Parser::new();
        p.parse_stdlib(concat!(env!("CARGO_MANIFEST_DIR"), "/default"))
            .expect("read the stdlib");
        p.parse_str(source, "<records-test>", false);
        crate::scopes::check(&mut p.data, &mut p.database);
        let d = p.data.def_nr(fn_name);
        assert_ne!(d, u32::MAX, "no function {fn_name}");
        let code = body(&p.data, d);
        p.data.definitions[d as usize].code = code;
        let ops = Ops::new(&p.data);
        let mut sum = BTreeMap::new();
        analyse_all(&p.data, &ops, &mut sum)
            .remove(&d)
            .map(|(lines, _)| lines)
            .unwrap_or_default()
    }

    fn call(data: &Data, op: &str, args: Vec<Value>) -> Value {
        let d = data.def_nr(op);
        assert_ne!(d, u32::MAX, "no op {op}");
        Value::Call(d, args)
    }

    /// A plain free of a variable that names the caller's store on one path and its own on the
    /// other releases the caller's store on the first (`@FR-F-ParamRebind`): `x` is bound to
    /// parameter `p` where `c` holds and minted where it does not, then freed.  Falsified by
    /// `LOFT_RECORDS_NO_MAYBE_PARAM_END` (the finding goes away), and by binding `x` only
    /// to its own store (no `Param` point to report).
    #[test]
    fn a_free_of_a_maybe_parameter_store_is_a_wrong_end() {
        let src = "struct S2 { a: integer }\n\
                   fn t(p: S2, c: boolean) { x = S2 { a: 1 }; }\n\
                   fn main() { t(S2 { a: 2 }, true); }\n";
        let found = findings(src, "n_t", |data, d| {
            let vars = &data.def(d).variables;
            let var = |name: &str| Value::Var(vars.var(name));
            let tp = Value::Int(i32::from(data.def(data.def_nr("S2")).known_type()));
            v_block(
                vec![
                    v_if(
                        var("c"),
                        call(data, "OpPutRef", vec![var("x"), var("p")]),
                        call(data, "OpDatabase", vec![var("x"), tp]),
                    ),
                    call(data, "OpFreeRef", vec![var("x")]),
                ],
                Type::Void,
                "records-test",
            )
        });
        assert!(
            found
                .iter()
                .any(|l| l.contains("wrong-end") && l.contains("may be the caller's store")),
            "a plain free of `x`, the caller's store on one path, must read as a wrong end; the \
             records said:\n{}",
            found.join("\n")
        );
    }

    /// `design-stale-names.md` F6: an alias made AFTER a store's end is in `stale`, so a free
    /// through it is still a re-free.  `h` mints the store and `w` names it on one arm only
    /// (`{null, store}` after the join); the free through `h` ends it on every path; then `k` is
    /// bound from `w` — a MAY alias made after the end, so not in the must-set `ended` — and
    /// freed.  Where the arm ran, `k` names the freed store: the finding must stand.  Falsified
    /// by dropping the alias insertion in `bind`: `k` is then in neither set and the re-free is
    /// silenced as a name that never named the freed store.
    #[test]
    fn an_alias_made_after_the_end_still_reads_as_a_second_free() {
        let src = "struct S1 { a: integer }\n\
                   fn t(c: boolean) { h = S1 { a: 1 }; w = S1 { a: 2 }; k = S1 { a: 3 }; }\n\
                   fn main() { t(true); }\n";
        let found = findings(src, "n_t", |data, d| {
            let vars = &data.def(d).variables;
            let var = |name: &str| Value::Var(vars.var(name));
            let tp = Value::Int(i32::from(data.def(data.def_nr("S1")).known_type()));
            v_block(
                vec![
                    call(data, "OpDatabase", vec![var("h"), tp]),
                    v_if(
                        var("c"),
                        call(data, "OpPutRef", vec![var("w"), var("h")]),
                        Value::Null,
                    ),
                    call(data, "OpFreeRef", vec![var("h")]),
                    call(data, "OpPutRef", vec![var("k"), var("w")]),
                    call(data, "OpFreeRef", vec![var("k")]),
                ],
                Type::Void,
                "records-test",
            )
        });
        assert!(
            found
                .iter()
                .any(|l| l.contains("re-free") && l.contains("`k`")),
            "the free through `k`, bound from a stale name after the end, must read as a second \
             free; the records said:\n{}",
            found.join("\n")
        );
    }
}
