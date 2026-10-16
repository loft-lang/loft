// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The sandbox admission walk (@PLN86): which definitions a `[sandbox]` policy designates,
//! what the sandboxed code can reach (definitions, FFI bridges, capability links), and the
//! admission, totality, raw-write and complexity reports a host reads before running it.

use super::{Parser, Value};

impl Parser {
    /// @PLN86 step 1.2 — install the host's sandbox policy before parsing.  The
    /// designation is host-controlled; a script can never mark itself sandboxed.
    pub fn set_sandbox_config(&mut self, config: crate::sandbox::SandboxConfig) {
        self.sandbox = config;
    }

    /// @PLN86 — does the loaded `[sandbox]` policy designate any sandboxed source?
    /// The CLI uses this to disable the program warm-cache for a sandboxed program:
    /// a warm load restores the IR without re-parsing, so `def_sandbox` would never
    /// form and admission (+ the force-interpret guard) would be silently bypassed.
    #[must_use]
    pub fn sandbox_is_active(&self) -> bool {
        self.sandbox.is_active()
    }

    /// @PLN86 step 1.2 — the sandbox profile a function was parsed under, or
    /// `None` for unrestricted (trusted) code.  The admission walk reads this to
    /// apply the totality / capability rules only to sandboxed defs.
    #[must_use]
    pub fn def_sandbox_profile(&self, def_nr: u32) -> Option<&str> {
        self.def_sandbox.get(&def_nr).map(String::as_str)
    }

    /// @PLN86 step 1.3 — the sandbox-reachable set: every def reachable from the
    /// sandboxed entries via calls + fn-ref literals, descending only into
    /// sandboxed defs (trusted symbols are leaves).  Step 2.3 capability-checks
    /// the non-sandboxed members.  Call after parsing, when bodies are resolved.
    #[must_use]
    pub fn sandbox_reachable_set(&self) -> std::collections::HashSet<u32> {
        crate::sandbox::reachable_set(&self.data, &self.def_sandbox, &self.sandbox_fn_refs)
    }

    /// @PLN86 step 1.4 — external `[native]` cdylib bridges reachable from the
    /// sandboxed entries.  Empty unless a sandboxed def reaches an external FFI
    /// symbol; admission rejects a non-empty result when the profile forbids
    /// native FFI (RCE by construction).  Backend force-interpret + the
    /// no-default-features cdylib removal are the remaining 1.4 work.
    #[must_use]
    pub fn sandbox_ffi_bridges(&self) -> Vec<u32> {
        crate::sandbox::reachable_ffi_bridges(&self.data, &self.sandbox_reachable_set())
    }

    /// @PLN86 — the `group#right` call-gate link a def declares (in its signature),
    /// or `None` if unlinked.  Admission gates each trusted symbol in the reachable
    /// set against the profile via `SandboxConfig::allows`.
    #[must_use]
    pub fn def_cap_group(&self, def_nr: u32) -> Option<&str> {
        let cap = self.data.def(def_nr).cap();
        (!cap.is_empty()).then_some(cap)
    }

    /// @PLN86 step 2.3 — the capability-admission walk: every trusted symbol a
    /// sandboxed def reaches must carry a `group#right` call-gate link its profile
    /// grants (or be `native_ffi`-allowed).  An empty result means the sandboxed code is
    /// admitted; otherwise each `CapViolation` names the offending reference for
    /// a diagnostic.  Run after parsing.
    #[must_use]
    pub fn sandbox_admit(&self) -> Vec<crate::sandbox::CapViolation> {
        crate::sandbox::admit_capabilities(
            &self.data,
            &self.sandbox,
            &self.def_sandbox,
            &self.sandbox_fn_refs,
        )
    }

    /// @PLN86 step 2.2 — the capability-coverage lint: public functions lacking a
    /// `group#right` call-gate link.  The host runs this over the stdlib + libraries to find the
    /// surface still to tag; an empty result is full coverage (L3-cap).
    #[must_use]
    pub fn untagged_public_symbols(&self) -> Vec<u32> {
        crate::sandbox::untagged_public_symbols(&self.data)
    }

    /// @PLN86 — the library a def belongs to (derived from its source), the
    /// wholesale-admission key for a profile's `allow_libs`.  `None` for a
    /// synthetic / sourceless def.
    #[must_use]
    pub fn def_library(&self, def_nr: u32) -> Option<String> {
        crate::sandbox::def_library(&self.data, def_nr)
    }

    /// @PLN86 step P3 — totality violations: the sandboxed script is rejected if
    /// it cannot be proven to terminate (an unbounded `while`, or a recursion
    /// cycle).  Empty for a provably-total script.
    #[must_use]
    pub fn sandbox_totality(&self) -> Vec<crate::sandbox::TotalityViolation> {
        crate::sandbox::admit_totality(
            &self.data,
            &self.def_sandbox,
            &self.sandbox_unbounded_loops,
            &self.sandbox_fn_refs,
        )
    }

    /// @PLN86 step 2.4 — no-raw-write violations: sandboxed defs that directly
    /// mutate heap data (`x.field = v` / `v[i] = v`).  Empty when every mutation
    /// goes through an allow-listed `*.write` op.
    #[must_use]
    pub fn sandbox_raw_writes(&self) -> Vec<crate::sandbox::RawWriteViolation> {
        crate::sandbox::raw_write_violations(&self.def_sandbox, &self.sandbox_raw_writes)
    }

    /// @PLN86 P6.1 — true iff `group` (the part before `#` in a `group#right`
    /// capability link) is covered by a declared `capability`: either declared
    /// exactly, or a sub-namespace of one (a `capability game` declaration covers a
    /// `game.entity#read` link, matching the grant-side namespacing).  A link to an
    /// uncovered group is a load error; admission resolves links against the declared
    /// set once every declaration is registered (parsing complete), so forward +
    /// cross-file references are fine.
    #[must_use]
    pub fn cap_is_declared(&self, group: &str) -> bool {
        self.declared_capabilities
            .iter()
            .any(|d| crate::sandbox::cap_prefix_match(d, group))
    }

    /// @PLN86 P6.8 (F8) — every capability link in the **main program** whose group is
    /// NOT covered by a declared `capability` (a typo'd / forgotten declaration).  Scans
    /// the three link homes: a function call gate (`Definition.cap`), a struct-field link
    /// (`member_access`), and a parameter `#default` lock (`param_locks`).  An uncovered
    /// group can never match a grant, so today it denies SILENTLY — this turns it into a
    /// clean, named load error.  Reported once per distinct group.
    ///
    /// Scoped to `MAIN_SOURCE` — the program the author is iterating on (the §6 modder
    /// feedback loop), which is always parsed fresh so its declarations are present in
    /// the registry.  The stdlib + installed libraries (other sources) are TRUSTED: their
    /// links were validated when authored as a main program, and they may be loaded from
    /// the IR cache (where the parser-side `capability` registry is not restored), so
    /// re-checking them here would falsely reject a clean program.  Registry IR
    /// persistence — needed to widen this to cached library links — is a later step.
    #[must_use]
    pub fn sandbox_undeclared_links(&self) -> Vec<String> {
        let mut undeclared: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut check = |token: &str| {
            if let Some((group, _)) = token.split_once('#')
                && !group.is_empty()
                && !self.cap_is_declared(group)
            {
                undeclared.insert(group.to_string());
            }
        };
        let is_main = |d_nr: u32| {
            (d_nr as usize) < self.data.definitions.len()
                && self.data.definitions[d_nr as usize].source == crate::data::MAIN_SOURCE
        };
        for d in &self.data.definitions {
            if !d.cap.is_empty() && d.source == crate::data::MAIN_SOURCE {
                check(&d.cap);
            }
        }
        for ((struct_nr, _), tokens) in &self.member_access {
            if is_main(*struct_nr) {
                for t in tokens {
                    check(t);
                }
            }
        }
        for ((fn_nr, _), t) in &self.param_locks {
            if is_main(*fn_nr) {
                check(t);
            }
        }
        undeclared
            .into_iter()
            .map(|g| {
                format!(
                    "undeclared capability `{g}` — a `group#right` link references it but no \
                     matching `capability` is declared.\n  fix: add `capability {g}` at top \
                     level, or correct the link."
                )
            })
            .collect()
    }

    /// @PLN86 P6.4 — record a `group#right` capability link on a struct field
    /// (`struct def_nr`, field name).  Called as each field's links are parsed.
    pub(crate) fn record_member_link(&mut self, struct_nr: u32, field: &str, token: String) {
        self.member_access
            .entry((struct_nr, field.to_string()))
            .or_default()
            .push(token);
    }

    /// @PLN86 P6.4 — the `group#right` capability links on a struct field, or empty
    /// if the field carries none (read-allow / no-update / no-append by default).
    #[must_use]
    pub fn member_links(&self, struct_nr: u32, field: &str) -> &[String] {
        self.member_access
            .get(&(struct_nr, field.to_string()))
            .map_or(&[], Vec::as_slice)
    }

    /// @PLN86 step 2.5 — ALL sandbox admission errors (capability + totality +
    /// no-raw-write), each rendered as a correct, specific, actionable message
    /// (position + the rule + the fix).  Empty when the script is admitted.  This
    /// is the host's primary surface — the contract a modder iterates against.
    #[must_use]
    pub fn sandbox_admission_errors(&self) -> Vec<String> {
        let caps = self.sandbox_admit().into_iter().map(|v| {
            crate::sandbox::describe_violation(&self.data, &self.sandbox, &self.def_sandbox, &v)
        });
        let totality = self
            .sandbox_totality()
            .into_iter()
            .map(|v| crate::sandbox::describe_totality_violation(&self.data, &v));
        let raw_writes = self
            .sandbox_raw_writes()
            .into_iter()
            .map(|v| crate::sandbox::describe_raw_write_violation(&self.data, &v));
        let field_reads = crate::sandbox::field_read_violations(
            &self.sandbox,
            &self.def_sandbox,
            &self.sandbox_field_reads,
        )
        .into_iter()
        .map(|v| crate::sandbox::describe_field_read_violation(&self.data, &v));
        let field_updates = crate::sandbox::field_update_violations(
            &self.sandbox,
            &self.def_sandbox,
            &self.member_access,
            &self.sandbox_field_updates,
        )
        .into_iter()
        .map(|v| crate::sandbox::describe_field_update_violation(&self.data, &v));
        let field_appends = crate::sandbox::field_append_violations(
            &self.sandbox,
            &self.def_sandbox,
            &self.member_access,
            &self.sandbox_field_appends,
        )
        .into_iter()
        .map(|v| crate::sandbox::describe_field_append_violation(&self.data, &v));
        let param_locks = crate::sandbox::param_lock_violations(
            &self.sandbox,
            &self.def_sandbox,
            &self.sandbox_param_overrides,
        )
        .into_iter()
        .map(|v| crate::sandbox::describe_param_lock_violation(&self.data, &v));
        // @PLN86 P6.8 (F8) — an authoring error in the host's OWN links: a `group#right`
        // referencing a capability that was never declared.  Surfaced alongside the
        // grant violations so the host fixes its contract before a modder iterates.
        let undeclared = self.sandbox_undeclared_links();
        // @PLN86 §8 (F11) — the data envelope: the proven peak-heap footprint must fit
        // the profile's declared data_budget (or be provable at all).
        let data_env =
            crate::sandbox::data_envelope_violations(&self.data, &self.sandbox, &self.def_sandbox)
                .into_iter()
                .map(|v| crate::sandbox::describe_data_violation(&v));
        // #631 — a profile that allow-lists the library its own sandboxed code lives
        // in.  Listed FIRST: it disables the checks that produce the other findings,
        // so any verdict computed under it is unsound and fixing it comes before
        // anything else the walk reports.
        let self_allow = crate::sandbox::self_allow_list_violations(
            &self.data,
            &self.sandbox,
            &self.def_sandbox,
        )
        .into_iter()
        .map(|v| crate::sandbox::describe_self_allow_list_violation(&self.data, &v));
        self_allow
            .chain(caps)
            .chain(totality)
            .chain(raw_writes)
            .chain(field_reads)
            .chain(field_updates)
            .chain(field_appends)
            .chain(param_locks)
            .chain(undeclared)
            .chain(data_env)
            .collect()
    }

    /// @PLN86 — does this program designate ANY sandboxed def?  True gates the
    /// load-time admission walk (`sandbox_admission_errors`), which is **backend-
    /// agnostic**: an admitted script is total and fault-free on the interpreter AND
    /// on `--native` (bounded loops + an acyclic call graph + partial ops that yield
    /// null on both backends — div/mod-zero, OOB, overflow), so the host keeps its
    /// choice of backend.  (The earlier forced interpret-only was dropped: it rested
    /// on a false "native traps where the interpreter is total" premise — verified
    /// untrue.  A deployment that wants to forbid host-side `rustc` on mod-derived
    /// input can reintroduce it as a per-profile opt-in; the cdylib-FFI surface stays
    /// gated by `native_ffi`.)
    #[must_use]
    pub fn has_sandboxed_defs(&self) -> bool {
        !self.def_sandbox.is_empty()
    }

    /// @PLN86 step 3.4 — the worst-case complexity DEGREE of the sandboxed code:
    /// its step count is `O(n^degree)` in the largest input size.  An admitted
    /// script is total, so this is finite; the host reads it to bound the inputs
    /// so no single frame stalls (L5).
    #[must_use]
    pub fn sandbox_complexity_degree(&self) -> u32 {
        crate::sandbox::sandbox_complexity_degree(&self.data, &self.def_sandbox)
    }

    /// @PLN86 space budget — the worst-case SPACE (peak-heap) degree of the
    /// sandboxed code: `O(n^degree)` memory in the largest input.  The host bounds
    /// inputs against it so a bounded loop building a structure cannot OOM.
    #[must_use]
    pub fn sandbox_space_degree(&self) -> u32 {
        crate::sandbox::sandbox_space_degree(&self.data, &self.def_sandbox)
    }

    /// @PLN86 P7.1 (F9) — the worst-case peak-heap footprint `(degree, coeff)`: peak
    /// heap is bounded by `coeff · n^degree` bytes in the largest input `n`.  The
    /// coefficient is `Σ record_size` over the accumulating allocation sites; F11
    /// compares `coeff · max_input_n^degree` against the profile's `data_budget`.
    #[must_use]
    pub fn sandbox_space_footprint(&self) -> (u32, u32) {
        crate::sandbox::sandbox_space_footprint(&self.data, &self.def_sandbox)
    }

    /// @PLN86 prevention #3 — host capabilities that are NOT total: every
    /// `#cap`-tagged function whose call tree can reach an abort op.  A host-side
    /// lint (the mirror of the script-side 3.3 exclusion); an empty result means the
    /// loft-bodied capability surface cannot fault the host on a script value.
    #[must_use]
    pub fn sandbox_capability_totality_violations(
        &self,
    ) -> Vec<crate::sandbox::CapTotalityViolation> {
        crate::sandbox::capability_totality_violations(&self.data, &self.sandbox_fn_refs)
    }

    /// @PLN86 — the human-readable worst-case complexity report (time + space).
    #[must_use]
    pub fn sandbox_complexity_report(&self) -> String {
        crate::sandbox::complexity_report(
            self.sandbox_complexity_degree(),
            self.sandbox_space_degree(),
        )
    }

    /// @PLN86 L4 — record that the current def references `fn_d_nr` as a fn-ref
    /// VALUE (a function name used as a value).  No-op outside sandboxed code.
    /// Called at the fn-ref creation site so every flow is caught.
    pub(crate) fn record_sandbox_fn_ref(&mut self, fn_d_nr: u32) {
        if self.in_sandbox {
            self.sandbox_fn_refs
                .entry(self.context)
                .or_default()
                .insert(fn_d_nr);
        }
    }

    /// @PLN86 §7.2 (F7) — record, for a sandboxed call to `d_nr`, each argument that
    /// OVERRIDES a `…#default`-locked parameter.  A parameter is overridden when its slot
    /// was supplied explicitly (`actual[i]` present and non-`Null` — defaults have not
    /// filled the gaps yet at the call site) AND the value DIFFERS from the parameter's
    /// declared default; an argument equal to the default is exactly what the lock pins
    /// to, so it is free.  Admission (`param_lock_violations`) then rejects an override
    /// whose lock token the calling profile does not grant.
    pub(super) fn record_param_lock_overrides(&mut self, d_nr: u32, actual: &[Value]) {
        if self.param_locks.is_empty() {
            return;
        }
        for i in 0..self.data.attributes(d_nr) {
            let Some(token) = self.param_locks.get(&(d_nr, i as u32)) else {
                continue;
            };
            let token = token.clone();
            let Some(arg) = actual.get(i) else { continue };
            if matches!(arg.unspan(), Value::Null) {
                continue; // not supplied → keeps its default, free
            }
            if arg.unspan() == self.data.def(d_nr).attributes()[i].value.unspan() {
                continue; // explicitly the default → not an override
            }
            let pos = *self.lexer.peek_pos();
            self.sandbox_param_overrides
                .entry(self.context)
                .or_default()
                .push((token, pos));
        }
    }
}
