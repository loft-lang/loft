// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I75 — Diagnostics collector

//! @PLN130 — the compile-time manifest of every deep copy the EMITTERS write.
//!
//! The copy diagnostic classifies copies it finds in the **IR**, during `scopes::check`.
//! Both code generators invent more copies *after* that — a whole-record bind `b = a` and a
//! call-return bind are minted at emission time and appear in no IR the analysis ever walks
//! — so they reach no diagnostic even in principle. Measured: with every copy flag on, a
//! program that provably deep-copies reports `none — every structure copy is a move, a
//! literal, or already borrowed` (loft#774, @PLN130 probes 10 and 11).
//!
//! This module closes that by recording what was actually EMITTED, so the guard can prove
//! the diagnostic covers it. The manifest is written where the copy is written, which is the
//! one place that cannot be wrong about whether a copy exists.
//!
//! **Compile-time only.** Nothing here reaches a compiled program: no op changes, no runtime
//! bookkeeping, no cost in the generated binary. A deep copy's *size* is deliberately out of
//! scope — it depends on runtime content (`copy_claims` walks nested vectors and texts), and
//! loft reports **where** a copy happens, never **how much** it moved. That is the same
//! bargain rustc makes with `.clone()`.

use crate::use_analysis::verdicts_for;
use std::cell::RefCell;

/// Which emitter wrote the copy — named so an uncovered site points straight at the code
/// that produced it rather than at a line number alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// Interpreter `gen_set_first_ref_var_copy` — a whole-record bind `b = a`.
    InterpRecordBind,
    /// Interpreter first-bind from a call whose return is not a fresh store.
    InterpCallReturn,
    /// Interpreter tuple-destructuring bind.
    InterpTupleBind,
    /// Interpreter REASSIGNMENT from a call. A lift temp inside an expression
    /// (`__lift_N = file(path)`) compiles here, never through a first-bind path — which is
    /// how the stdlib's `exists()` copy stayed off the manifest until a known-copying case
    /// was measured against it.
    InterpReassignCall,
    /// Interpreter reassignment `v = src` between same-struct references (#306).
    InterpReassignVar,
    /// Native whole-record bind (`generation::dispatch`, the `Value::Var(src)` arm).
    NativeRecordBind,
    /// Native call-return bind. Emits a runtime adopt-or-copy branch, so this is a *may
    /// copy* site — still one the diagnostic must account for.
    NativeCallReturn,
    /// A materialisation the PARSER writes into the IR: a projection copied into an owned
    /// store, a return materialised into the caller's buffer, or a `&` write-back that must
    /// publish an owned record rather than a view (loft#775).
    ///
    /// These are genuinely NECESSARY — publishing the view instead is the use-after-free
    /// #775 fixed — and, unlike the codegen-minted copies, they ARE classified: the analysis
    /// walks them out of the IR and buckets them `Implicit`
    /// (`MAT fn=n_take v=2(__ref_1) verdict=Copy bucket=implicit`). The user report stays
    /// quiet about them by design, because a model-inherent copy is not the author's to fix.
    ///
    /// Recorded anyway, so the guard's coverage claim covers the parser as well as the two
    /// generators: without them the manifest could only ever say "nothing uncovered *among
    /// the paths I watch*". They are expected to come back COVERED — a parser site appearing
    /// in the uncovered list means the classification stopped reaching them, which is worth
    /// hearing about.
    ///
    /// (Measured caution: a program whose only copies are these executes two record copies
    /// while `--report-copies` answers `none`. That is the `Implicit` silence working as
    /// designed, NOT a blind spot — "not shown to the user" and "not accounted for" are
    /// different, and conflating them is how this comment first got written wrong.)
    ParserMaterialise,
}

impl Origin {
    /// Backend that owns this emitter, for grouping a report.
    #[must_use]
    pub fn backend(self) -> &'static str {
        match self {
            Self::InterpRecordBind
            | Self::InterpCallReturn
            | Self::InterpTupleBind
            | Self::InterpReassignCall
            | Self::InterpReassignVar => "interpret",
            Self::NativeRecordBind | Self::NativeCallReturn => "native",
            Self::ParserMaterialise => "parser",
        }
    }

    /// Whether the emitted code copies unconditionally, or branches at runtime and copies
    /// only on one arm. Reported so a MAY-copy is never read as a definite one — native's
    /// call-return arm emits an adopt-or-copy guard on store identity.
    #[must_use]
    pub fn always_copies(self) -> bool {
        !matches!(self, Self::NativeCallReturn)
    }
}

/// One emitted deep copy.
#[derive(Clone, Debug)]
pub struct CopySite {
    /// Enclosing function definition.
    pub def_nr: u32,
    /// Destination variable the copy fills.
    pub var: u16,
    /// The copied record's type number.
    pub type_nr: u16,
    pub origin: Origin,
}

thread_local! {
    /// Emitted-copy sites for the current compilation. A `Vec` (not a set): two emitters
    /// legitimately write two copies for one binding, and collapsing them would hide one.
    static SITES: RefCell<Vec<CopySite>> = const { RefCell::new(Vec::new()) };
}

thread_local! {
    /// @PLN130 F2/F4/F8 — bindings whose alias was taken away because the container they view
    /// changed under them: `(binding, container, function)`. Recorded by `scopes`, reported to
    /// the author by [`report_materialised_views`].
    ///
    /// The function name is part of the key, not decoration. Entries dedupe on it, and without
    /// it `c` copied out of `bx` reported ONCE for a whole program — every other site with the
    /// same two names went silent, which is the one thing this diagnostic may not do.
    static MATERIALISED: RefCell<Vec<(String, String, String)>> =
        const { RefCell::new(Vec::new()) };
}

/// Record one materialised view, deduped on the whole key.
fn note(key: (String, String, String)) {
    MATERIALISED.with(|m| {
        let mut m = m.borrow_mut();
        if !m.contains(&key) {
            m.push(key);
        }
    });
}

/// Note that `var`'s element view was materialised because `container` is reshaped.
///
/// The author asked for an alias and is getting a copy, so this is not an internal detail —
/// a write through `var` no longer reaches the container. Reported unconditionally: a silent
/// copy is the one thing the model does not allow.
pub fn note_materialised_view(var: &str, container: &str, function: &str) {
    note((var.to_string(), container.to_string(), function.to_string()));
}

/// Note that `var` was copied out of keyed collection `coll` because a write to its KEY field
/// `field` would otherwise leave the element unreachable (@PLN130 F4).
///
/// Separate message from [`note_materialised_view`]: the cause is different (a key write, not
/// a removal) and so is the way out — re-insert with `coll[key] = value` rather than
/// restructure the loop.
pub fn note_rekeyed_view(var: &str, coll: &str, field: &str, function: &str) {
    note((
        format!("{var}\u{0}rekey\u{0}{field}"),
        coll.to_string(),
        function.to_string(),
    ));
}

/// Note that `var` was copied out of `owner` because `owner` is REASSIGNED while `var` is
/// live (@PLN130 F8).
///
/// Separate message from [`note_materialised_view`] because the cause is a different one and
/// so is the remedy: nothing about the container's shape changed — the NAME stopped meaning
/// the store, so the fix at the use site is to bind the whole value (which copies, C86) or
/// to take the view again after the reassignment.
/// Note that `var` was copied out of `container` because `container` GROWS while `var` is
/// live (loft#1373).
///
/// Separate message from [`note_materialised_view`] for the reason that rule split the event
/// in two: a removal renumbers the elements after the one removed, a growth can move all of
/// them at once, and a reader sent looking for the wrong statement pays for the difference.
pub fn note_grown_view(var: &str, container: &str, function: &str) {
    note((
        format!("{var}\u{0}grow"),
        container.to_string(),
        function.to_string(),
    ));
}

pub fn note_reassigned_view(var: &str, owner: &str, function: &str) {
    note((
        format!("{var}\u{0}reassign"),
        owner.to_string(),
        function.to_string(),
    ));
}

/// Tell the author which views lost their alias, and why. Drains.
///
/// Deliberately not gated: constraint 2 of @PLN130 — where rustc errors on a use-after-move,
/// loft copies and warns. The copy keeps the program correct; this is what keeps it honest.
pub fn report_materialised_views() {
    let rows: Vec<(String, String, String)> =
        MATERIALISED.with(|m| std::mem::take(&mut *m.borrow_mut()));
    for (var, container, function) in rows {
        if let Some(name) = var.strip_suffix("\u{0}reassign") {
            eprintln!(
                "advice: in `{function}`, `{name}` was copied out of `{container}` because \
                 `{container}` is reassigned while `{name}` is in use — a view names a place \
                 inside `{container}`, and giving `{container}` a new value leaves nothing for \
                 it to point at. Writes through `{name}` no longer reach `{container}`."
            );
        } else if let Some(name) = var.strip_suffix("\u{0}grow") {
            eprintln!(
                "advice: in `{function}`, `{name}` was copied out of `{container}` because \
                 `{container}` grows while `{name}` is in use — a container that outgrows its \
                 allocation moves every element, so the view could not stay valid. Writes \
                 through `{name}` no longer reach `{container}`."
            );
        } else if let Some((name, field)) = var.split_once("\u{0}rekey\u{0}") {
            eprintln!(
                "advice: in `{function}`, `{name}` was copied out of `{container}` because \
                 `{field}` is one of `{container}`'s keys — writing a key through an element \
                 would leave it reachable by no key at all. Writes through `{name}` no longer \
                 reach `{container}`; to change a key, re-insert with \
                 `{container}[key] = value`."
            );
        } else {
            eprintln!(
                "advice: in `{function}`, `{var}` was copied out of `{container}` because \
                 `{container}` is modified while `{var}` is in use — removing an element \
                 renumbers the others, so the view could not stay valid. Writes through \
                 `{var}` no longer reach `{container}`."
            );
        }
    }
}

/// Record a deep copy at the moment it is emitted.
///
/// Call this from the branch that actually WRITES the copy — never from the function that
/// decides whether to. `gen_set_first_ref_var_copy` returns early on a last-use move, and a
/// site recorded before that branch would claim a copy that never existed.
pub fn record(def_nr: u32, var: u16, type_nr: u16, origin: Origin) {
    // Gated at the RECORD, not just the report: with the guard off this is one cached bool
    // per emitted copy and nothing is accumulated, so compiling a large program does not
    // build a manifest nobody reads. (One cached env read — `OnceLock`.)
    if !crate::keys::copy_manifest_enabled() {
        return;
    }
    SITES.with(|s| {
        s.borrow_mut().push(CopySite {
            def_nr,
            var,
            type_nr,
            origin,
        });
    });
}

/// Every copy emitted so far this compilation.
#[must_use]
pub fn sites() -> Vec<CopySite> {
    SITES.with(|s| s.borrow().clone())
}

/// Drop the manifest (between compilations in one process — the test harness, the REPL).
pub fn clear() {
    SITES.with(|s| s.borrow_mut().clear());
    SELF_BINDS.with(|s| s.borrow_mut().clear());
}

thread_local! {
    /// `x = x` statements the parser erased (#330), by function, variable and line: no IR is left
    /// for the census to find, and the copy-lease rules judge the line all the same (@PLN163).
    static SELF_BINDS: RefCell<std::collections::BTreeSet<(u32, u16, u32)>> =
        const { RefCell::new(std::collections::BTreeSet::new()) };
}

/// Record that the parser erased `var = var` at `line` in `def_nr`.
pub fn note_self_bind(def_nr: u32, var: u16, line: u32) {
    if crate::keys::drop_copy_census_enabled() {
        SELF_BINDS.with(|s| s.borrow_mut().insert((def_nr, var, line)));
    }
}

/// The erased `x = x` statements of `def_nr`, as `(var, line)`.
#[must_use]
pub fn self_binds(def_nr: u32) -> Vec<(u16, u32)> {
    SELF_BINDS.with(|s| {
        s.borrow()
            .iter()
            .filter(|(d, _, _)| *d == def_nr)
            .map(|&(_, v, l)| (v, l))
            .collect()
    })
}

thread_local! {
    /// @PLN163 P2b — the copies the copy census gave an `(H-Move)` verdict, by
    /// `(function, destination variable)`.  Filled before code generation and never drained by
    /// [`report`], so the interpreter's and the native generator's manifests are both checked
    /// against it.
    static LEASE_SITES: RefCell<std::collections::HashSet<(u32, u16)>> =
        RefCell::new(std::collections::HashSet::new());
    /// The functions whose RESULT the census judged: a `return` verdict, or a copy into one of
    /// their return buffers.  A caller's copy of such a function's result is judged there.
    static LEASE_RETURNS: RefCell<std::collections::HashSet<u32>> =
        RefCell::new(std::collections::HashSet::new());
}

/// Note that the census gave the copy into `var` in function `def_nr` a lease verdict.
pub fn note_lease_site(def_nr: u32, var: u16) {
    LEASE_SITES.with(|s| s.borrow_mut().insert((def_nr, var)));
}

/// Note that the census judged what function `def_nr` returns.
pub fn note_lease_return(def_nr: u32) {
    LEASE_RETURNS.with(|s| s.borrow_mut().insert(def_nr));
}

/// Forget the census's verdicts (before a new census, between compilations in one process).
pub fn clear_lease() {
    LEASE_SITES.with(|s| s.borrow_mut().clear());
    LEASE_RETURNS.with(|s| s.borrow_mut().clear());
}

/// The emitted copies of a droppable that the copy census gave no lease verdict — the
/// completeness half of the refusal (@PLN163 P2b, `(H-Copy-Refuse)`): a copy a generator mints
/// that no verdict covers is a copy the refusal would let through.
///
/// A copy is covered when the census judged the same destination in the same function.  A bind
/// of a CALL result is covered when the census judged what the called function returns, because
/// that is where the copied value comes from.
///
/// A [`Origin::ParserMaterialise`] record is written while the IR is BUILT, and the scope pass
/// may rewrite its destination afterwards — a return's work variable merged into the return
/// buffer.  A record whose destination no longer occurs in the function's body names no copy that
/// is emitted, so it is not counted; the copy that is emitted is judged where it now writes.
#[must_use]
pub fn lease_uncovered(data: &crate::data::Data) -> (usize, Vec<CopySite>) {
    let mut total = 0;
    let mut out = Vec::new();
    for site in sites() {
        if !type_releases(data, site.type_nr) {
            continue;
        }
        if site.origin == Origin::ParserMaterialise
            && !data
                .def(site.def_nr)
                .code
                .any_node(&mut |n| matches!(n, crate::data::Value::Var(v) if *v == site.var))
        {
            continue;
        }
        total += 1;
        let judged = LEASE_SITES.with(|s| s.borrow().contains(&(site.def_nr, site.var)))
            || called_function(data, site.def_nr, site.var)
                .is_some_and(|callee| LEASE_RETURNS.with(|r| r.borrow().contains(&callee)));
        if !judged {
            out.push(site);
        }
    }
    (total, out)
}

/// Does the known type `type_nr` have a release to run — a hook, or a cascade over members that
/// have one?
fn type_releases(data: &crate::data::Data, type_nr: u16) -> bool {
    type_nr != u16::MAX
        && (0..data.definitions())
            .any(|d| data.def(d).known_type == type_nr && data.drop_cascade_nr(d) != u32::MAX)
}

/// The user function whose result an assignment of `var` in `def_nr` binds, if one does.
fn called_function(data: &crate::data::Data, def_nr: u32, var: u16) -> Option<u32> {
    let mut found = None;
    data.def(def_nr).code.walk(&mut |n| {
        if found.is_some() {
            return;
        }
        if let crate::data::Value::Set(v, rhs) = n
            && *v == var
        {
            rhs.walk(&mut |m| {
                if found.is_none()
                    && let crate::data::Value::Call(d, _) = m
                    && matches!(data.def(*d).def_type, crate::data::DefType::Function)
                    && !data.def(*d).is_operator()
                {
                    found = Some(*d);
                }
            });
        }
    });
    found
}

/// The guard: emitted copies the copy diagnostic produced no verdict for.
///
/// A site is COVERED when the analysis classified the same destination binding in the same
/// function — that is the fact the user-facing report is built from, so a site the analysis
/// never rowed can never be reported however the rendering changes.
///
/// Conservative in the safe direction: a covered site is dropped even if the report would
/// later filter it out for being `Implicit` or `Internal`, because those are *deliberate*
/// silences. Only a copy the analysis never saw at all is a blind spot.
#[must_use]
pub fn uncovered(data: &crate::data::Data) -> Vec<CopySite> {
    let mut out = Vec::new();
    for site in sites() {
        let classified = verdicts_for(data, site.def_nr)
            .into_iter()
            .any(|r| r.var_nr == site.var);
        if !classified {
            out.push(site);
        }
    }
    out
}

/// Render the guard's finding. Returns the number of uncovered sites so a caller can gate.
///
/// Reports the SITE and the TYPE — never a size. A copy is deep, so a flat record size is
/// not its cost, and a 12-byte-looking copy that moves a megabyte teaches the wrong thing.
///
/// **DRAINS the manifest**, so the two generators can each report the sites they wrote
/// without the second call repeating the first's. Interpreter codegen finishes at
/// `compile::byte_code`; native generation runs later, and in an `introspect` run both
/// happen in one process.
pub fn report(data: &crate::data::Data) -> usize {
    if !crate::keys::copy_manifest_enabled() {
        clear();
        return 0;
    }
    let bad = uncovered(data);
    if crate::keys::drop_copy_census_enabled() {
        let (total, unjudged) = lease_uncovered(data);
        eprintln!(
            "lease-manifest: {total} emitted {} of a droppable, {} without a lease verdict",
            if total == 1 { "copy" } else { "copies" },
            unjudged.len()
        );
        for s in &unjudged {
            let def = data.def(s.def_nr);
            eprintln!(
                "  lease-unjudged {} [{:?}]  fn {}  binding `{}`",
                s.origin.backend(),
                s.origin,
                def.name,
                def.variables.name(s.var)
            );
        }
    }
    clear();
    if bad.is_empty() {
        return 0;
    }
    eprintln!(
        "loft copy-manifest guard — {} emitted {} no diagnostic accounts for",
        bad.len(),
        if bad.len() == 1 { "copy" } else { "copies" }
    );
    for s in &bad {
        let def = data.def(s.def_nr);
        let var_name = def.variables.name(s.var);
        // A `_`-prefixed binding is compiler-generated: the author never wrote it, so it is
        // OUR worklist, not theirs. Still reported — the guard's audience is the compiler —
        // but marked, so a stdlib internal is not mistaken for a user-visible copy.
        let origin_note = if s.origin.always_copies() {
            "copies"
        } else {
            "may copy"
        };
        let who = if def.variables.is_compiler_generated(s.var) {
            " (compiler-generated binding)"
        } else {
            ""
        };
        eprintln!(
            "  {} [{:?}]  fn {}  binding `{}`  {} {}{}",
            s.origin.backend(),
            s.origin,
            def.name,
            var_name,
            origin_note,
            data.type_name_str(def.variables.tp(s.var)),
            who,
        );
    }
    bad.len()
}
