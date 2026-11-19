// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator
//! Where each rewrite FIRES — the census a tightened condition cannot hide from.
//!
//! A rewrite's admission carries conditions, and a bug fix that adds a decline can switch it
//! off on real code by a margin no timing shows.  The number of times a rewrite is admitted
//! over a fixed body of programs is a pure function of the compiler, so a drop against a
//! committed baseline names the rule and the program that lost it, and nothing else moves it
//! (`scripts/rewrite_census.py`, `make rewrite-census`).
//!
//! `fired(rule, n)` is called where a rewrite is ADMITTED — once per site it will rewrite,
//! never at a decline — with the `R-…` name of its rule in `doc/claude/formal/rewrites.md`,
//! and `R-…/clause` for a clause worth counting apart.  `LOFT_REWRITE_CENSUS=<file>` writes
//! `rule<TAB>count` lines there when the native source has been generated; unset, a call is
//! one cached test.
//!
//! `LOFT_REWRITE_CENSUS_FN=<file>` writes the same admissions split by WHERE they were
//! decided — `phase<TAB>function<TAB>rule<TAB>count` — so a routine's rewrites can be read
//! off the functions it runs (`scripts/interp_gap.py`).  The phase is `native` for a body the
//! rustc writer emits and `c` for one the C writer emits (the interpreter never sees either
//! form), `interp` for a body the interpreter's bytecode generator writes (compiled code never
//! sees that form), `ir` for the scope pass's per-body rewrites (both backends run them), and
//! `parse` for everything decided before any of them.
//!
//! `declined(rule, site, why)` is the complement — called where a rewrite REFUSED a site it
//! was shaped for, with the reason the site's trace names — and `LOFT_DECLINE_CENSUS=<file>`
//! writes `rule<TAB>why<TAB>count` there.  A site is counted once per rule, under the first
//! reason it gave, however many probes ask about it, so the count is comparable with the
//! admissions: together they say what share of the shapes a rule is written for it reaches,
//! and which conditions hold the rest back (`scripts/decline_census.py`, `make
//! decline-census`).  Not every rule reports its declines yet; the report says which do.
//!
//! `LOFT_DECLINE_CENSUS_FN=<file>` writes the same refusals split by function —
//! `function<TAB>rule<TAB>why<TAB>count` — to read beside `LOFT_REWRITE_CENSUS_FN`: what a
//! rule rewrote in a function and what each test refused there.  It holds counts, never a
//! verdict for a function, and names no site (`doc/claude/REWRITE_REASONS.md`).

use std::collections::{BTreeMap, HashSet};
use std::fmt::Write as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock};

static COUNTS: Mutex<BTreeMap<&'static str, u64>> = Mutex::new(BTreeMap::new());
static PAUSED: AtomicUsize = AtomicUsize::new(0);
/// `(phase, function, rule) → count`, kept only when `LOFT_REWRITE_CENSUS_FN` is set.
type ByFn = BTreeMap<(&'static str, String, &'static str), u64>;
static BY_FN: Mutex<ByFn> = Mutex::new(BTreeMap::new());
/// The body an admission is being decided for: `(phase, function)`.
static WHERE: Mutex<Option<(&'static str, String)>> = Mutex::new(None);
/// `(rule, why) → sites declined for that reason`, kept only when `LOFT_DECLINE_CENSUS` is set.
static DECLINES: Mutex<BTreeMap<(&'static str, &'static str), u64>> = Mutex::new(BTreeMap::new());
/// `(function, rule, why) → sites declined`, kept only when `LOFT_DECLINE_CENSUS_FN` is set.
type DeclinesByFn = BTreeMap<(String, &'static str, &'static str), u64>;
static DECLINES_BY_FN: Mutex<DeclinesByFn> = Mutex::new(BTreeMap::new());
/// The `(rule, site)` pairs already counted: a site asked about twice is one decline.
static DECLINED_SITES: Mutex<Option<HashSet<(&'static str, String)>>> = Mutex::new(None);
/// The `(rule, site)` pairs admitted, for the rules that name their sites: `fired` counts
/// every emitted copy of a loop (a guarded loop is emitted twice), these count the loop.
static ADMITTED_SITES: Mutex<Option<HashSet<(&'static str, String)>>> = Mutex::new(None);

/// While one lives, admissions are credited to `function` in `phase` (`"ir"`, `"interp"`,
/// `"c"`, `"native"`);
/// outside every such scope they are `parse`-phase with no function.
pub struct InBody {
    prev: Option<(&'static str, String)>,
}

impl InBody {
    #[must_use]
    pub fn enter(phase: &'static str, function: &str) -> Self {
        let mut prev = None;
        if (by_fn_target().is_some() || refusals_kept())
            && let Ok(mut w) = WHERE.lock()
        {
            prev = w.replace((phase, function.to_owned()));
        }
        InBody { prev }
    }
}

impl Drop for InBody {
    fn drop(&mut self) {
        if (by_fn_target().is_some() || refusals_kept())
            && let Ok(mut w) = WHERE.lock()
        {
            *w = self.prev.take();
        }
    }
}

fn by_fn_target() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| crate::platform::env_output("LOFT_REWRITE_CENSUS_FN"))
        .as_deref()
}

/// Counting is suspended while one of these lives: an emission made FOR something other than
/// the program — a dependency package's native crate, built in-process on a cold artefact
/// cache — would otherwise count that package once per crate that includes it, and only on
/// the runs whose cache missed (measured: 1702 bodies against 868 warm, one process).
pub struct Pause;

impl Pause {
    #[must_use]
    pub fn new() -> Self {
        PAUSED.fetch_add(1, Ordering::Relaxed);
        Pause
    }
}

impl Default for Pause {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Pause {
    fn drop(&mut self) {
        PAUSED.fetch_sub(1, Ordering::Relaxed);
    }
}

fn target() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| crate::platform::env_output("LOFT_REWRITE_CENSUS"))
        .as_deref()
}

fn decline_target() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| crate::platform::env_output("LOFT_DECLINE_CENSUS"))
        .as_deref()
}

fn decline_fn_target() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| crate::platform::env_output("LOFT_DECLINE_CENSUS_FN"))
        .as_deref()
}

/// Is either refusal census being written?
fn refusals_kept() -> bool {
    decline_target().is_some() || decline_fn_target().is_some()
}

/// A refused match: the CLAUSE of the rule that failed — a fixed label, the one thing the
/// census counts — and, for a trace of one function, what at the site it failed on
/// (`doc/claude/REWRITE_REASONS.md`).  A gate answers `Err(Refusal)`; its caller reports
/// `clause` to [`declined`] and prints the whole refusal where a trace is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refusal {
    pub clause: &'static str,
    pub about: String,
}

impl Refusal {
    /// The clause alone.
    #[must_use]
    pub fn of(clause: &'static str) -> Self {
        Refusal {
            clause,
            about: String::new(),
        }
    }

    /// The clause, and the name or value at the site it failed on.
    #[must_use]
    pub fn about(clause: &'static str, about: impl Into<String>) -> Self {
        Refusal {
            clause,
            about: about.into(),
        }
    }
}

impl From<&'static str> for Refusal {
    fn from(clause: &'static str) -> Self {
        Refusal::of(clause)
    }
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.about.is_empty() {
            f.write_str(self.clause)
        } else {
            write!(f, "{} ({})", self.clause, self.about)
        }
    }
}

/// Is a decline census being written?  A reason that costs work to compute is asked only then.
pub fn declines_wanted() -> bool {
    PAUSED.load(Ordering::Relaxed) == 0 && refusals_kept()
}

/// Rewrite `rule` refused `site` — a loop, a local, a function the rule is shaped for —
/// because `why`.  One count per `(rule, site)`, under the first reason given; the site is
/// read inside the body being generated (`InBody`), so a loop's scope number, which every
/// function counts from one, names one loop.  A site one probe refuses and another admits
/// is in both counts, so a rule's reach reads as a floor.
///
/// The per-function census credits the refusal to the body being generated; outside one, a
/// site key is `function:rest` or a function's name, and anything else (an address) is
/// credited to no function.  [`declined_in`] names the function outright.
pub fn declined(rule: &'static str, site: &str, why: &'static str) {
    if !declines_wanted() {
        return;
    }
    match WHERE.lock().ok().and_then(|w| w.clone()) {
        Some((_, function)) if !function.is_empty() => {
            count(rule, &function, format!("{function}:{site}"), why);
        }
        _ => {
            let function = site.split(':').next().unwrap_or_default();
            let function = if function.starts_with("0x") {
                ""
            } else {
                function
            };
            count(rule, function, site.to_owned(), why);
        }
    }
}

/// [`declined`] for a site named by its function and a local, loop or block of it.  The key
/// is built only where a census is being written.
pub fn declined_in(rule: &'static str, function: &str, site: &str, why: &'static str) {
    if declines_wanted() {
        count(rule, function, format!("{function}:{site}"), why);
    }
}

/// One refusal of `rule` at the site `key` of `function`, unless that site was counted.
fn count(rule: &'static str, function: &str, key: String, why: &'static str) {
    if let Ok(mut seen) = DECLINED_SITES.lock() {
        let fresh = seen.get_or_insert_with(HashSet::new).insert((rule, key));
        if !fresh {
            return;
        }
    }
    if decline_target().is_some()
        && let Ok(mut d) = DECLINES.lock()
    {
        *d.entry((rule, why)).or_insert(0) += 1;
    }
    if decline_fn_target().is_some()
        && let Ok(mut d) = DECLINES_BY_FN.lock()
    {
        *d.entry((function.to_owned(), rule, why)).or_insert(0) += 1;
    }
}

/// Rewrite `rule` admitted `site` — the same site key a `declined` of the rule would carry,
/// counted once however many copies of it are emitted, so the decline census can set the
/// two against each other.  `fired` is still called beside it: that count is the rewrite
/// census's and keeps its meaning.
pub fn admitted(rule: &'static str, site: &str) {
    if PAUSED.load(Ordering::Relaxed) > 0 || decline_target().is_none() {
        return;
    }
    let site = match WHERE.lock().ok().and_then(|w| w.clone()) {
        Some((_, function)) => format!("{function}:{site}"),
        None => site.to_owned(),
    };
    if let Ok(mut seen) = ADMITTED_SITES.lock() {
        seen.get_or_insert_with(HashSet::new).insert((rule, site));
    }
}

/// Rewrite `rule` was admitted at `n` more sites.
pub fn fired(rule: &'static str, n: usize) {
    if n == 0 || PAUSED.load(Ordering::Relaxed) > 0 {
        return;
    }
    if target().is_some()
        && let Ok(mut c) = COUNTS.lock()
    {
        *c.entry(rule).or_insert(0) += n as u64;
    }
    if by_fn_target().is_some() {
        let (phase, function) = WHERE
            .lock()
            .ok()
            .and_then(|w| w.clone())
            .unwrap_or(("parse", String::new()));
        if let Ok(mut c) = BY_FN.lock() {
            *c.entry((phase, function, rule)).or_insert(0) += n as u64;
        }
    }
}

/// Write the counts to `LOFT_REWRITE_CENSUS`, sorted by rule, and the per-body split to
/// `LOFT_REWRITE_CENSUS_FN`; nothing for a switch that is unset.
pub fn write() {
    write_by_fn();
    write_declines();
    write_declines_by_fn();
    let Some(path) = target() else { return };
    let Ok(c) = COUNTS.lock() else { return };
    let mut out = String::new();
    for (rule, n) in c.iter() {
        let _ = writeln!(out, "{rule}\t{n}");
    }
    crate::platform::write_env_output(path, &out, "the rewrite census");
}

fn write_declines() {
    let Some(path) = decline_target() else { return };
    let Ok(d) = DECLINES.lock() else { return };
    let mut out = String::new();
    for ((rule, why), n) in d.iter() {
        let _ = writeln!(out, "{rule}\t{why}\t{n}");
    }
    // The admitted sites of the rules that name them, as `rule<TAB>admitted<TAB>count`.
    if let Ok(a) = ADMITTED_SITES.lock() {
        let mut per_rule: BTreeMap<&'static str, u64> = BTreeMap::new();
        for (rule, _) in a.iter().flatten() {
            *per_rule.entry(rule).or_insert(0) += 1;
        }
        for (rule, n) in per_rule {
            let _ = writeln!(out, "{rule}\tadmitted\t{n}");
        }
    }
    crate::platform::write_env_output(path, &out, "the decline census");
}

fn write_declines_by_fn() {
    let Some(path) = decline_fn_target() else {
        return;
    };
    let Ok(d) = DECLINES_BY_FN.lock() else { return };
    let mut out = String::new();
    for ((function, rule, why), n) in d.iter() {
        let _ = writeln!(out, "{function}\t{rule}\t{why}\t{n}");
    }
    crate::platform::write_env_output(path, &out, "the per-function decline census");
}

fn write_by_fn() {
    let Some(path) = by_fn_target() else { return };
    let Ok(c) = BY_FN.lock() else { return };
    let mut out = String::new();
    for ((phase, function, rule), n) in c.iter() {
        let _ = writeln!(out, "{phase}\t{function}\t{rule}\t{n}");
    }
    crate::platform::write_env_output(path, &out, "the rewrite census");
}
