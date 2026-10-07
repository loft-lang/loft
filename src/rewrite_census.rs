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
//! generator emits (the interpreter never sees that form), `ir` for the scope pass's per-body
//! rewrites (both backends run them), and `parse` for everything decided before either.

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use std::collections::BTreeMap;
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

/// While one lives, admissions are credited to `function` in `phase` (`"ir"`, `"native"`);
/// outside every such scope they are `parse`-phase with no function.
pub struct InBody {
    prev: Option<(&'static str, String)>,
}

impl InBody {
    #[must_use]
    pub fn enter(phase: &'static str, function: &str) -> Self {
        let mut prev = None;
        if by_fn_target().is_some()
            && let Ok(mut w) = WHERE.lock()
        {
            prev = w.replace((phase, function.to_owned()));
        }
        InBody { prev }
    }
}

impl Drop for InBody {
    fn drop(&mut self) {
        if by_fn_target().is_some()
            && let Ok(mut w) = WHERE.lock()
        {
            *w = self.prev.take();
        }
    }
}

fn by_fn_target() -> Option<&'static str> {
    static PATH: OnceLock<Option<String>> = OnceLock::new();
    PATH.get_or_init(|| {
        std::env::var("LOFT_REWRITE_CENSUS_FN")
            .ok()
            .filter(|p| !p.is_empty())
    })
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
    PATH.get_or_init(|| {
        std::env::var("LOFT_REWRITE_CENSUS")
            .ok()
            .filter(|p| !p.is_empty())
    })
    .as_deref()
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
    let Some(path) = target() else { return };
    let Ok(c) = COUNTS.lock() else { return };
    let mut out = String::new();
    for (rule, n) in c.iter() {
        let _ = writeln!(out, "{rule}\t{n}");
    }
    if let Err(e) = std::fs::write(path, out) {
        eprintln!("loft: cannot write the rewrite census to '{path}': {e}");
    }
}

fn write_by_fn() {
    let Some(path) = by_fn_target() else { return };
    let Ok(c) = BY_FN.lock() else { return };
    let mut out = String::new();
    for ((phase, function, rule), n) in c.iter() {
        let _ = writeln!(out, "{phase}\t{function}\t{rule}\t{n}");
    }
    if let Err(e) = std::fs::write(path, out) {
        eprintln!("loft: cannot write the rewrite census to '{path}': {e}");
    }
}
