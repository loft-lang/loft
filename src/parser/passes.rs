// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! The two-pass driver: every way a parse is started (a file, a directory, the standard
//! library, an in-memory source, a REPL statement, a live-reload snippet) and what each pass
//! owes the next — the forward references pass 1 leaves for pass 2, the signature work only
//! pass 2 can finish, and the H5 name-stability contract that checks pass 2 reproduced pass 1.

use super::{
    BTreeSet, Data, DefType, DiagEntry, Function, ImportSpec, Level, Lexer, ParseResult, Parser,
    Type, Value, diagnostic_format, other_sep, registry_type_hint, scopes, sep, sep_str,
};
use std::io::Write;

impl Parser {
    /// # Panics
    /// With filesystem problems.
    pub fn parse(&mut self, filename: &str, default: bool) -> bool {
        if !default {
            self.data.limits = self.data.limits.min(crate::limits::Limits::from_env());
        }
        // under the `wasm` feature, check VIRT_FS before trying the real filesystem.
        #[cfg(feature = "wasm")]
        if let Some(content) = crate::wasm::virt_fs_get(filename) {
            return self.parse_virtual(&content, filename, default);
        }
        self.parse_main(filename, None, default)
    }

    /// loft#925 — parse `content` through the whole-program path of
    /// [`parse`](Self::parse), as if it were the file at `filename`.
    ///
    /// Everything else is identical: `filename` decides the source directory,
    /// the owning package, and therefore how a `use` inside `content` resolves.
    /// The file itself is never read and need not exist.
    ///
    /// This is not [`parse_source`](Self::parse_source), which is the REPL /
    /// `--script` entry and skips the between-pass promotions a program parse
    /// runs (`reserve_late_return_buffers` and the rest) — a base parsed that
    /// way would hand its libraries on in a state no ordinary parse produces.
    pub fn parse_as(&mut self, filename: &str, content: &str, default: bool) -> bool {
        self.parse_main(filename, Some(content), default)
    }

    /// loft#925 — start this parser from `base`, an already-completed parse of
    /// the stdlib plus a set of libraries, so the program parsed next binds
    /// those libraries instead of loading them again.
    ///
    /// What travels is what a `use` of an already-loaded library would otherwise
    /// have produced: the definitions and the database schema (the pair a warm
    /// stdlib load restores), the file each short name was loaded from — which
    /// is what lets a module-name clash still be reported as a clash — and the
    /// native / placed-library registrations a manifest read queued.  Everything
    /// else is per-program and is left at its fresh value, `own_lib` and
    /// `database.source_dir` among them, so the program's own file still decides
    /// them.
    ///
    /// The parse-time side maps (`complexity`, `field_read_counts`, the sandbox
    /// designations) deliberately do NOT travel: they drive diagnostics that the
    /// base parse has already emitted, and copying them would emit each one a
    /// second time.  The caller carries the base's diagnostics instead.
    pub fn seed_from(&mut self, base: &Parser) {
        self.data = base.data.clone();
        self.data.freeze_uses();
        self.database.install_schema(base.database.types.clone());
        self.use_paths.clone_from(&base.use_paths);
        self.pending_native_libs
            .clone_from(&base.pending_native_libs);
        self.native_lib_regs.clone_from(&base.native_lib_regs);
        self.pending_native_compile
            .clone_from(&base.pending_native_compile);
        self.pending_placed_libs
            .clone_from(&base.pending_placed_libs);
    }

    pub(super) fn parse_main(
        &mut self,
        filename: &str,
        content: Option<&str>,
        default: bool,
    ) -> bool {
        // @PLN11 arc E — record the input file for the whole-program cache key.
        if self.track_sources {
            self.parsed_sources.push(filename.to_string());
        }
        // loft#1648 — the H5 report reads this to tell a source that MOVED under the two
        // passes from a genuine cross-pass bug.  Set in `parse_main`, which is the function
        // that drives BOTH passes, so it brackets exactly the window they share.
        self.pass_started = std::time::SystemTime::now().into();
        // loft#1648 — both passes of this parse read each file's bytes once (H5).
        self.lexer.begin_parse();
        // @PLAN49 T1 — set the breadcrumb phase + initial file/line so
        // a watchdog-fired hard-kill localises any parse-time hang.
        crate::timeout::checkpoint_parse(filename, 0);
        self.default = default;
        // #255 / @PLN9: establish `source_dir` (the running program's own
        // directory) once, at the first non-default parse.  This is the single
        // home every execution path inherits — CLI run, `loft --test`, the wrap
        // integration runner, and the wasm/native front-ends all reach this
        // `parse()`.  `data.reset()` below clears `Data` but not `Stores`, so the
        // value survives the two-pass re-parse; the `is_empty()` guard keeps the
        // *first* (main) file winning over later directory/import re-parses.
        // (main.rs additionally sets it for the startup-cache path, which loads a
        // pre-parsed snapshot and never calls `parse()`.)
        // loft#1260 — say who this compilation's lints are addressed to.  Both instances:
        // the lexer's is what the parse writes into, and `p.diagnostics` is what the
        // post-scope lints are handed.  Same place and same entry file as `source_dir`
        // just below, which answers the same shape of question.
        if !default {
            self.lexer.set_lint_scope_for(filename);
            self.diagnostics.set_lint_scope_for(filename);
        }
        if !default && self.database.source_dir.is_empty() {
            self.database.source_dir = std::path::Path::new(filename)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        if !default && self.own_lib.is_none() {
            // The manifest's `[package] name` when there is one, else the file
            // stem — the same answer for a conventional package, whose entry
            // defaults to `src/<name>.loft`.
            let path = std::path::Path::new(filename);
            let from_manifest = path
                .parent()
                .and_then(std::path::Path::parent)
                .map(|root| root.join("loft.toml"))
                .filter(|m| crate::file_access::exists(m))
                .and_then(|m| crate::manifest::read_manifest(&m.to_string_lossy()))
                .and_then(|m| m.name);
            self.own_lib = from_manifest.or_else(|| crate::file_access::file_stem(path));
        }
        self.vars.logging = false;
        Self::load_main_file(&mut self.lexer, filename, content);
        if !default && self.program_entry.is_none() {
            self.program_entry = Some(self.lexer.pos().file);
        }
        self.first_pass = true;
        crate::diagnostics::set_first_pass(true);
        self.pending_imports.clear();
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.deferred_import_misses.clear();
        self.resolutions.clear();
        // @PLN86 1.2 — the def→profile side-map is keyed by def_nr, which
        // `data.reset()` reassigns; clear it so a re-parse re-derives the
        // designation rather than reading a stale entry.
        self.def_sandbox.clear();
        self.sandbox_unbounded_loops.clear();
        self.sandbox_raw_writes.clear();
        self.sandbox_fn_refs.clear();
        self.declared_capabilities.clear();
        self.member_access.clear();
        self.sandbox_field_reads.clear();
        self.sandbox_field_updates.clear();
        self.sandbox_field_appends.clear();
        self.last_field_target = None;
        self.param_locks.clear();
        self.sandbox_param_overrides.clear();
        self.pending_param_locks.clear();
        self.amp_pending = false;
        self.data.reset();
        // @PLN22 Phase 2 — the main program parses under its own source
        // (MAIN_SOURCE), distinct from the stdlib prelude (source 0), so a user
        // definition shadows a prelude name instead of colliding on `(name, 0)`.
        // `reset()` set source to 0; the default stdlib parse stays at 0.
        if !default {
            self.data.source = crate::data::MAIN_SOURCE;
        }
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.parse_file();
        self.resolve_deferred_unknowns();
        // H5 — two-pass name-stability contract: snapshot every def's attribute
        // count at the end of pass 1.  `data.reset()` between passes preserves
        // `definitions`, so def numbers are stable and pass 2 must reproduce these
        // counts exactly.  Post-arity-cascade (signatures freeze at declaration)
        // this is an INVARIANT, not an aspiration — a divergence is a real
        // cross-pass bug, asserted below after pass 2.
        // #675 / @PLAN59 — reserve the heap-return buffer for the functions whose
        // return type was not yet KNOWABLE at signature-parse time.
        //
        // The signature-time reservation asks "is the declared return a heap type",
        // and for a type that lives in a dependency the honest pass-1 answer is
        // "no idea": `glb`'s `-> Vec3` reads `Unknown(659)` there, and the stub's def
        // number is LOWER than the real struct's, so the definitions it needs do not
        // exist yet.  The reservation then never fires and `ref_return` grows the
        // attribute in PASS 2 instead — the cross-pass arity growth @PLAN59 exists to
        // prevent, and the one that stopped a published library from compiling once
        // the H5 guard became always-on.
        //
        // Here — after pass 1 and its deferred-unknown resolution, before the H5
        // snapshot and before pass 2 parses a single body — every type IS resolved.
        // So the same question gets a real answer, the attribute lands before any
        // pass-2 caller lowers a call, and pass 2's `ref_return` finds a buffer to
        // RENAME instead of one to grow.  Reserving here rather than at the signature
        // is what makes this cheap: no signature pre-pass, one O(defs) walk.
        // loft#808 — a par worker DECLARED BEFORE its `par(...)` site was still a bare
        // tuple return when pass 1 parsed it, because the set that says "this def is a
        // worker" only fills as pass 1 reaches each par site.  Promote those here, so
        // the boxing lands before `reserve_late_return_buffers` hands the now-heap
        // return its `__retbuf` and before the H5 snapshot counts attributes.
        self.between_passes();
        let pass1_attr_counts: Vec<usize> = (0..self.data.definitions.len())
            .map(|d| self.data.attributes(d as u32))
            .collect();
        let lvl = self.lexer.diagnostics().level();
        if lvl != Level::Error && lvl != Level::Fatal {
            self.first_pass = false;
            crate::diagnostics::set_first_pass(false);
            self.reverse_iterator = false;
            self.iterable_context = false;
            self.applied_imports.clear();
            self.deferred_unknown.clear();
            self.deferred_import_misses.clear();
            self.resolutions.clear();
            self.data.reset();
            if !default {
                self.data.source = crate::data::MAIN_SOURCE;
            }
            self.lambda_counter = 0;
            self.fn_lambdas.clear();
            Self::load_main_file(&mut self.lexer, filename, content);
            self.parse_file();
            self.resolve_deferred_unknowns();
            // @PLN35 PC3 — reject a left-recursive sub-rule grammar (a cycle in the invocation
            // graph) at compile time, before it can hang at runtime.  Post-parse over pass-2 edges.
            self.check_subrule_wellformedness();
            self.assert_pass2_def_attr_stable(&pass1_attr_counts);
            self.after_pass2();
        }
        self.backfill_native_symbol_crates();
        // Plan-07 phase 4h — emit `not null` field-reminder hints
        // after the second pass completes (so all reads have been
        // counted).  Silenceable via `LOFT_NO_HINT_NOT_NULL=1`.
        if !self.first_pass && !default {
            self.emit_not_null_hints();
            // @PLN130 F9 (loft#779) — the whole world is parsed here, so a callee's body can
            // be asked whether it removes from a `&` parameter.
            self.check_reshape_under_reference();
            self.synth_drop_cascades();
        }
        self.diagnostics.fill(self.lexer.diagnostics());
        self.diagnostics.is_empty()
    }

    /// Point the lexer at the parse's own entry — the file at `filename`, or
    /// `content` claiming to be that file.  Both passes go through here, so a
    /// caller supplying content gets it on both.
    pub(super) fn load_main_file(lexer: &mut Lexer, filename: &str, content: Option<&str>) {
        match content {
            Some(text) => lexer.parse_string(text, filename),
            None => lexer.switch(filename),
        }
    }

    /// H5 (two-pass name-stability contract): assert pass 2 reproduced pass 1's
    /// per-def attribute counts exactly.  The parser is not re-entrant beyond two
    /// passes (the #339 third-pass experiment segfaulted on half-migrated variable
    /// tables) and pass 2 re-finds synthesized names by position, so a def whose
    /// attribute count changed between passes is a silent contract break.  Post the
    /// @PLAN59 arity cascade (signatures freeze at declaration) this is an
    /// INVARIANT — a firing assert is a real cross-pass bug, not noise.
    ///
    /// Runs in EVERY build, not just `cfg(debug_assertions)`.  Those are switched
    /// off inside this library by `[profile.dev.package.loft]`, so a cfg-gated
    /// check here was dead in `cargo test` AND `target/debug/loft` AND `--release`
    /// alike — live only in the nightly `-C debug-assertions=on` job.  That is how
    /// loft#662 reached a release: the contract broke, nothing said so, and the
    /// failure surfaced two subsystems later as a codegen panic naming a generated
    /// symbol.  The project's convention for a load-bearing invariant is a plain
    /// `assert!` (see `Data::rollback_to`), and this one costs a `Vec<usize>` of
    /// one entry per definition, twice per parse — unmeasurable against a parse.
    ///
    /// The H5 spec's other named residual — work-ref (`__ref_N`) counter equality
    /// per fn — was dissolved by H1 (@PLAN59), but NOT for the reason recorded here
    /// before: `work_refs()` does not "fire zero times across the debug corpus", it
    /// fires ~5500 times (measured, loft#665).  What actually dissolved it is the
    /// signature-time `__retbuf`: the attribute exists before ANY body parses, so
    /// `ref_return`'s promotion no longer re-finds it by work-ref NAME and therefore
    /// no longer depends on `__ref_N` numbering being pass-stable.  A counter-equality
    /// assert would be not vacuous but NOISY — 11 of the 19 `work_refs` call sites are
    /// pass-2-only, so the counters legitimately diverge.
    ///
    /// That is the durable shape for this whole class, and it is why the text side
    /// stayed vulnerable (loft#662): a `text_return` work buffer is discovered FROM
    /// the body, so nothing reserves it at signature time and its numbering must be
    /// kept pass-stable by hand instead (see `Function::work_text_p2`).
    /// #675 / @PLAN59 — give every heap-returning function a `__retbuf` attribute
    /// before pass 2 starts, including the ones whose return type only became
    /// knowable after pass 1 finished.
    ///
    /// Mirrors the signature-time reservation's gate exactly, because the point is
    /// that the two together cover every heap-returning function: a body-carrying,
    /// non-`#rust`, non-lambda plain function whose declared return is a heap type
    /// and which does not already carry a hidden heap buffer. The last condition is
    /// why this reads the ATTRIBUTE SHAPE rather than the name `__retbuf`: pass 1's
    /// `ref_return` RENAMES the reserved attribute to the promoted local, so a
    /// function that already got its buffer no longer has one by that name.
    ///
    /// The variable is created alongside the attribute. `arguments()` yields argument
    /// vars in number order, so appending puts it last — which is where the attribute
    /// is too, and the position the callee's ABI expects.
    /// loft#808 — box the pure-value tuple return of every def pass 1 saw used as a
    /// `par(...)` worker, so pass 2 parses its body against `Reference(__tuple<…>)`.
    ///
    /// Runs between the passes because par-workerhood is only knowable once the whole
    /// file is read: a worker declared above its par site was parsed while the set was
    /// still empty. By pass 2 the set is complete, so `parse_function` computes the
    /// same promoted type and the two passes agree. Idempotent — a worker declared
    /// BELOW its par site was already promoted in pass 1 and is skipped by the
    /// `Type::Tuple` guard.
    /// loft#944 — refuse a tuple RETURN whose member is declared lower in the file.
    ///
    /// Everything else about a forward reference inside a tuple works: locals, vector
    /// elements (declared and inferred), struct-literal members, parameters, nesting. A
    /// RETURN does not, and the reason is structural rather than a missing branch.
    ///
    /// A heap-carrying tuple return is BOXED into the synthetic `Reference(__tuple<...>)`
    /// and given a hidden `__retbuf`. `parse_function` decides that by asking whether an
    /// element "carries a lifetime concern", and an unresolved member carries none — so
    /// pass 1 says no, and the return type is stored on pass 1 ONLY. Promoting it between
    /// the passes gets the signature right and still miscompiles: the synthetic struct is
    /// minted after pass 1's `fill_all`, so it has no LAYOUT while pass 2 parses the bodies
    /// that read it, and every offset lands as `u16::MAX` — the emitted body reads
    /// `OpDatabase(__ref_2, 65535)` where the working spelling reads `79`. Making it work
    /// means minting and laying out that struct during pass 1, from members that do not
    /// resolve until the end of it.
    ///
    /// So this says so, at the declaration, with the one-line cure. It replaces an internal
    /// compiler error — and, once the rest of loft#944 was fixed, something worse: a body
    /// that ran on the interpreter and read its first member back as `0` on `--native`.
    /// Wrap `tp` as the reference type a user-written `&` denotes, refusing an element
    /// type a reference tuple cannot address.
    ///
    /// This is the ONE place a `&` in source becomes a `Type::RefVar`, so
    /// [`tuples.md` `T-Ref-El`](../../doc/claude/formal/formal/tuples.md) is asked wherever
    /// the annotation is written — a parameter, an annotated local, or an inferred
    /// `b = &a`.  It used to be asked at the signature only, so the same `&(text, text)`
    /// that a parameter refused reached codegen from a local and died there as an internal
    /// compiler error on both backends (D-tup-2).  Route every new `&` position through
    /// here rather than repeating the check beside it: a second copy of the admitted list
    /// is the shape loft#1006 already was.
    ///
    /// Which representation the `&(…)` names is decided here too (`@FR-T-Ref-Rep`): a
    /// reference to the `__tuple<…>` RECORD — what a `&S` is — for every member list the
    /// record can lay out, so a link to a tuple local, a vector element and a struct field is
    /// one mechanism (loft#1883).  The remaining refusal is for an element the record cannot
    /// spell or lay out as a field.
    pub(crate) fn ref_var_type(&mut self, tp: Type) -> Type {
        // The record form carries every element type a struct field can, which is what the
        // loop variable over a `vector<(…)>` and a heap-tuple RETURN already use.
        if let Type::Tuple(ref elems) = tp
            && crate::data::ref_tuple_is_record(elems)
        {
            let elems = elems.clone();
            let d = self.data.tuple_def(&mut self.lexer, &elems);
            if d != u32::MAX {
                return Type::RefVar(Box::new(Type::Reference(d, crate::data::Deps::none())));
            }
        }
        if !self.first_pass
            && let Type::Tuple(ref elems) = tp
            && let Some(bad) = elems
                .iter()
                .find(|e| !crate::data::ref_tuple_record_element_ok(e))
        {
            let bad_name = bad.source_name(&self.data);
            diagnostic!(
                self.lexer,
                Level::Error,
                "a `&` reference tuple cannot hold a `{bad_name}` element — a nullable, \
                 fn-ref or nested-tuple element has no record form the reference can \
                 address; take the tuple by value and return a new one, or use a struct"
            );
        }
        Type::RefVar(Box::new(tp))
    }

    /// `@FR-T-Ref-Rep` — a tuple LOCAL handed to a record-backed `&(…)` parameter is built as the
    /// `__tuple<…>` record, and the bind that builds it comes before the call.  Pass 1 records
    /// the local at the call (`ref_linked_tuple_locals`), which it can only do when the
    /// callee's signature is already known; a callee declared BELOW its caller was refused on
    /// pass 2 as "a variable this function did not declare" (loft#1883).  Pass 1 leaves no IR
    /// for a call to a name it cannot see yet, so the call site records each tuple-local
    /// argument (`pending_tuple_args`) and every signature is known here to judge them.
    pub(super) fn record_forward_ref_tuple_links(&mut self) {
        let pending = std::mem::take(&mut self.pending_tuple_args);
        for (context, callee, index, local) in pending {
            let d = self.data.def_nr(&callee);
            if d == u32::MAX {
                continue;
            }
            let Some(param) = self.data.def(d).attributes().get(index) else {
                continue;
            };
            let record_param = match param.typedef.base() {
                Type::RefVar(inner) => match inner.base() {
                    Type::Reference(t, _) => self.data.def(*t).name().starts_with("__tuple<"),
                    Type::Tuple(elems) => crate::data::ref_tuple_is_record(elems),
                    _ => false,
                },
                _ => false,
            };
            if record_param {
                self.ref_linked_tuple_locals.insert((context, local));
            }
        }
    }

    pub(super) fn refuse_forward_tuple_returns(&mut self, adopted: &[(u32, Type)]) {
        if adopted.is_empty() {
            return;
        }
        for d in 0..self.data.definitions.len() as u32 {
            if !matches!(self.data.def_type(d), crate::data::DefType::Function) {
                continue;
            }
            let Type::Tuple(elems) = self.data.def(d).returned().clone() else {
                continue;
            };
            let Some(late) = elems.iter().find_map(|e| match e {
                Type::Unknown(n) => adopted
                    .iter()
                    .find(|(stub, _)| stub == n)
                    .map(|(stub, _)| self.data.def(*stub).name().to_string()),
                _ => None,
            }) else {
                continue;
            };
            let fname = self.data.def(d).name().trim_start_matches("n_").to_string();
            let pos = self.data.def(d).position;
            self.lexer.pos_diagnostic(
                Level::Error,
                &pos,
                &format!(
                    "`{fname}` returns a tuple containing `{late}`, which is declared later \
                     in the file — move the declaration of `{late}` above `{fname}`. A tuple \
                     RETURN has to know at its declaration whether it carries heap storage, \
                     and it cannot ask a type that does not exist yet; every other use of a \
                     forward-declared type in a tuple (a local, a vector element, a struct \
                     field, a parameter) works"
                ),
            );
        }
    }

    /// tuples.md T-Ref — a `&(…)` PARAMETER with a heap element is backed by the synthesized
    /// `__tuple<…>` record, whose shape the signature must know; an element declared later in
    /// the file is a stub on pass 1, so the two passes would type the parameter two ways.  The
    /// same refusal a tuple RETURN gets (`refuse_forward_tuple_returns`), for the same reason.
    pub(super) fn refuse_forward_ref_tuple_params(&mut self, adopted: &[(u32, Type)]) {
        if adopted.is_empty() {
            return;
        }
        for d in 0..self.data.definitions.len() as u32 {
            if !matches!(self.data.def_type(d), crate::data::DefType::Function) {
                continue;
            }
            let late = self.data.def(d).attributes().iter().find_map(|a| {
                let Type::RefVar(inner) = &a.typedef else {
                    return None;
                };
                let Type::Tuple(elems) = &**inner else {
                    return None;
                };
                if !crate::data::ref_tuple_is_record(elems) {
                    return None;
                }
                // `resolve_adopted_stubs` has already pointed the stub at the real type, so a
                // forward-declared element is recognised by BEING one of the adopted types.
                elems.iter().find_map(|e| {
                    adopted
                        .iter()
                        .find(|(stub, real)| {
                            e == real || matches!(e, Type::Unknown(n) if n == stub)
                        })
                        .map(|(stub, _)| (a.name.clone(), self.data.def(*stub).name().to_string()))
                })
            });
            let Some((pname, late)) = late else {
                continue;
            };
            let fname = self.data.def(d).name().trim_start_matches("n_").to_string();
            let pos = self.data.def(d).position;
            self.lexer.pos_diagnostic(
                Level::Error,
                &pos,
                &format!(
                    "`{fname}` takes `{pname}: &(…)` containing `{late}`, which is declared later \
                     in the file — move the declaration of `{late}` above `{fname}`. A `&(…)` \
                     with a heap element is backed by a record whose shape the signature has \
                     to know, and it cannot ask a type that does not exist yet"
                ),
            );
        }
    }

    /// What pass 2 owes the program, in one place — the signature work that can only be
    /// judged on the lowered IR, and so grows a definition's attribute list AFTER every
    /// call to it was lowered:
    ///
    /// @PLN104 P2 — the oracle pass flags every frame-local text return the interpreter would
    /// orphan (#568) into `force_tret` (default-on; opt out with `LOFT_NO_TRET_FIX`); P3 — the
    /// targeted promotion gives each flagged def a `__tret` retbuf IN PLACE on the pass-2 IR
    /// and patches only its direct callers to push the buffer.  The set is decided BEFORE the
    /// promotion, so every caller — forward- or backward-ref — gets the retbuf without an
    /// ABI-growth crash; post-H5, so the extra attrs never trip the pass1==pass2 contract.
    /// This replaced a whole-file re-parse ("third pass") whose non-idempotent re-lowering
    /// corrupted unrelated defs; touching only the promoted defs + their callers removes that
    /// collateral class by construction — `plans/104-tret-promotion/targeted-promotion-design.md`.
    ///
    /// Like [`between_passes`](Self::between_passes), every two-pass entry runs it: a
    /// definition promoted here in the running program and NOT in a shadow session's parse
    /// of the same file has one more parameter in the running program than the shadow
    /// believes, and a body generated from the shadow reads its buffer off the wrong slot
    /// (@PLN162 step 14 — the synthesised dispatcher, an owned text return, was promoted by
    /// `parse` and not by `parse_str`).  `force_tret` is cleared first: the set is narrowed
    /// to the promoted defs and never drained, and promoting a def twice would give it two
    /// buffers.
    pub(crate) fn after_pass2(&mut self) {
        self.force_tret.clear();
        self.report_tret_promotions();
        if !self.force_tret.is_empty() {
            self.targeted_tret_promotion();
        }
        // `@FR-R-WorkBuffer` — after the text promotion, whose `a == v` renumbering settles
        // the argument order this reads, and before the store-text instances below clone
        // their function's final signature.
        self.promote_work_buffers();
        // After the promotion: a store instance clones its function's FINAL signature, and a
        // call it retargets already carries any buffer the promotion added.
        self.mint_store_text_instances();
    }

    /// What pass 1 owes pass 2, in one place: every declaration has been seen, so the
    /// signatures pass 2 will call through are completed here — a par worker's tuple return
    /// boxed, a forward-referenced stub adopted and the tuple returns it held back judged, a
    /// bound-method stub's hidden parameters re-derived (@PLN125), a late text tail's buffer
    /// promoted to a hidden parameter, and a heap return's `__retbuf` reserved.  Every
    /// two-pass entry runs it — `parse`, `parse_source`, `parse_virtual` AND `parse_snippet` —
    /// because the ATTRIBUTE LIST of a definition is the frame its callers push, and two
    /// parses of one program that disagree on it disagree on every call between them.
    /// Measured (@PLN162 step 14): the live-reload shadow session loaded the program through
    /// `parse_source`, which ran three of these six, so a text-returning definition had one
    /// hidden buffer there and two in the running program; a body generated from the shadow
    /// and called from the running program's sites read its return buffer off the wrong slot
    /// and faulted at the first `OpAppendText`.
    pub(super) fn between_passes(&mut self) {
        self.promote_par_worker_tuple_returns();
        // loft#944 — every declaration has now been seen, so a forward reference the parser
        // left as a stub has been adopted.  Point the stored `Type::Unknown(stub)` values at
        // the real type, then promote the tuple RETURNS that could not be judged while a
        // member was unresolved — both before `reserve_late_return_buffers` below, so a
        // newly-heap return gets its `__retbuf` in the same breath.
        self.resolve_forward_generic_returns();
        let adopted = self.data.resolve_adopted_stubs(&mut self.lexer);
        self.refuse_forward_tuple_returns(&adopted);
        self.refuse_forward_ref_tuple_params(&adopted);
        self.record_forward_ref_tuple_links();
        // @PLN125 — the same class, one step earlier in the chain: a bound-method stub's
        // hidden parameters are decided from the INTERFACE method's return type, which on
        // pass 1 can still be an unresolved forward reference.  Re-derive here, before the
        // buffers below and before the H5 snapshot.
        self.refresh_bound_method_stubs();
        self.promote_late_text_buffers();
        self.reserve_late_return_buffers();
    }

    /// @PLN165 D7 — a return type naming a generic struct declared below was a stub on pass
    /// 1 (`note_forward_return`).  Every declaration is seen now: the return becomes the
    /// instance its arguments name, laid out here, before `reserve_late_return_buffers` gives
    /// the record return the `__retbuf` its callers push on pass 2.
    pub(super) fn resolve_forward_generic_returns(&mut self) {
        for (f, stub, args) in std::mem::take(&mut self.forward_generic_returns) {
            if self.data.def_type(stub) != DefType::TypeTemplate
                || args.iter().any(|a| self.data.names_unresolved(a))
            {
                continue;
            }
            let inst = self.data.instance_def(&mut self.lexer, stub, &args);
            if inst == u32::MAX {
                continue;
            }
            crate::typedef::lay_out_late(&mut self.data, &mut self.database, inst);
            let target = Type::Reference(inst, crate::data::Deps::none());
            let ret = self.data.def(f).returned().clone();
            if let Some(t) = Data::rewrite_type_opt(&ret, stub, &target) {
                self.data.definitions[f as usize].returned = t;
            }
        }
    }

    pub(super) fn promote_par_worker_tuple_returns(&mut self) {
        let mut workers: Vec<u32> = self.par_worker_defs.iter().copied().collect();
        workers.sort_unstable(); // deterministic `__tuple<…>` def order across runs
        for d in workers {
            if d as usize >= self.data.definitions.len() {
                continue;
            }
            let Type::Tuple(elems) = self.data.def(d).returned().clone() else {
                continue;
            };
            // Mirror `parse_function`'s size-driven arm exactly; the lifetime-bearing
            // arm already promoted at signature time whether or not this is a worker.
            if u32::from(crate::variables::size(
                &Type::Tuple(elems.clone()),
                &crate::data::Context::Argument,
            )) <= 8
                || elems.iter().any(|e| matches!(e, Type::Function(..)))
            {
                continue;
            }
            let synthetic_d_nr = self.data.tuple_def(&mut self.lexer, &elems);
            // Assigned directly, not via `set_returned` — that one asserts the slot is
            // still UNKNOWN (it guards the signature-time write), and pass 1 has
            // already filled it with the bare tuple this promotion replaces.
            self.data.definitions[d as usize].returned =
                Type::Reference(synthetic_d_nr, crate::data::Deps::none());
        }
    }

    /// @PLN125 — re-derive every bound-method stub's signature now that all types resolve.
    ///
    /// A stub stands in for the concrete method `re_resolve_call` will substitute, so its
    /// ABI has to equal that method's ABI. The hidden parameters in that ABI are decided
    /// from the interface method's RETURN TYPE, and on pass 1 that type can still be an
    /// unresolved forward reference — an interface method returning a struct declared later
    /// in the file reads as `Unknown` there, so the `__retbuf` arm never fired. The
    /// monomorph then gained a `__ref_N` to match the concrete method's arity, with no
    /// scope and no initialiser, and codegen met a variable that never got a stack slot:
    /// *"Incorrect var `__ref_2[65535]`"*, an internal compiler error on both backends.
    ///
    /// The cure is the one #675 already found for the same question asked of a function's
    /// own return: ask it HERE, between the passes, where every type IS resolved — never in
    /// pass 2, which would grow an attribute after pass 1 counted them and break the H5
    /// two-pass contract. Idempotent wherever the two passes already agreed, which is what
    /// keeps `bytecode-comparisons/bound-stubs-corpus.loft` byte-identical.
    pub(super) fn refresh_bound_method_stubs(&mut self) {
        for (stub_nr, child_nr, holder_nr) in std::mem::take(&mut self.bound_method_stubs) {
            self.set_bound_stub_signature(stub_nr, child_nr, holder_nr);
        }
    }

    /// loft#918 — give a text-returning function the hidden work buffer its returned
    /// local needs, for the locals whose type pass 1 could not know.
    ///
    /// `text_return` promotes a returned text local into a hidden `&text` out-param, and
    /// it decides that from the local's TYPE.  A local bound to a call declared lower in
    /// the file has no type in pass 1 — the callee is not registered yet — so the
    /// promotion happened on pass 2 alone: the signature grew one argument after pass 2
    /// had already lowered calls against the old one.  With the caller below the callee
    /// that is "only" the H5 divergence; with the caller above it, the call really does
    /// pass one argument too few.
    ///
    /// Here, between the passes, every declaration exists, so the same promotion lands
    /// before any caller is lowered — and pass 2 finds the attribute already there and
    /// takes `classify_text_dep`'s `Attr` branch, which is exactly the state a pass-1
    /// promotion leaves behind.  `reserve_late_return_buffers` settles the heap return
    /// buffer (#675) in this slot for the same reason.
    pub(super) fn promote_late_text_buffers(&mut self) {
        let buf_tp = Type::RefVar(Box::new(Type::Text(crate::data::Deps::none())));
        for (d, v) in std::mem::take(&mut self.late_text_tails) {
            let def = self.data.def(d);
            if !matches!(self.data.def_type(d), DefType::Function)
                || *def.code() == Value::Null
                || def.is_operator()
                || !def.rust().is_empty()
                || !def.native().is_empty()
                || def.name().starts_with("n___lambda_")
            {
                continue;
            }
            // The declared return had to stay text for the promotion to apply, and the
            // local had to stay a local: a name that resolved to a parameter, or one an
            // earlier candidate already promoted, is served by its own attribute.
            if !matches!(def.returned().base(), Type::Text(_)) {
                continue;
            }
            let vars = def.variables();
            if !vars.exists(v) || vars.is_argument(v) {
                continue;
            }
            let name = vars.name(v).to_string();
            if def.attr_names.contains_key(&name) {
                continue;
            }
            let a = self
                .data
                .add_attribute(&mut self.lexer, d, &name, buf_tp.clone());
            self.data.definitions[d as usize].attributes[a].hidden = true;
            let f = &mut self.data.definitions[d as usize].variables;
            f.mark_nullable_text_buffer(v);
            f.set_type(v, buf_tp.clone());
            f.become_argument(v);
            f.mark_used(v);
        }
    }

    pub(super) fn reserve_late_return_buffers(&mut self) {
        for d in 0..self.data.definitions.len() as u32 {
            let def = self.data.def(d);
            if !matches!(self.data.def_type(d), DefType::Function)
                || *def.code() == Value::Null
                || def.is_operator()
                || !def.rust().is_empty()
                || !def.native().is_empty()
            {
                continue;
            }
            // loft#945 — a LAMBDA is included when, and only when, its return type was read
            // off its own body (`Parser::adopted_ret_defs`).  That is the case nothing could
            // reserve for at signature time, and without a buffer pass 2's `ref_return` GREW
            // the attribute instead of renaming onto it, so `xs.map(|x| { [x, x + 1] })` died
            // on the H5 two-pass contract.
            //
            // A lambda that DECLARED its return (`fn(i: integer) -> S { … }`) keeps the old
            // skip, and the narrowness is load-bearing rather than conservative: the
            // signature-time path already served it, and a second buffer is one store leaked
            // per closure — `717-closure-struct-return.loft` caught exactly that.
            //
            // loft#1177 / loft#1178 — a DECLARED `-> vector<…>` is included too, and the
            // sentence above needed correcting to say so: the signature-time path does not
            // serve a lambda at all (it excludes them by name, *"separate parse path, no
            // earlier callers"*), so *"already served"* was only ever true of the returns
            // that need no buffer.  A struct and a record-enum are those — a declared
            // `-> P` lambda carries no hidden attribute and answers correctly — while a
            // VECTOR is delivered through a backing store the caller supplies, so pass 2
            // GREW `__vdb_1` and `fn(v: integer) -> vector<integer> { [v] }` aborted the
            // compiler on the H5 contract.  `Vector` because that is the former measured to
            // grow one; a second buffer on the others is the leak per closure that
            // `717-closure-struct-return.loft` pins.
            //
            // Every declared-collection LAMBDA is reserved for, and the two things that
            // stopped that from shipping are closed: `State::fn_return` releases the buffer a
            // callee did not hand back (loft#1179) and the native dispatch now asks the same
            // question of the value that came back, so a reservation nobody fills costs
            // nothing on either backend.  Predicting which bodies fill it cannot work — the
            // pass-1 tail of `{ xs = [1, 2]; xs.map(…) }` is `Var(xs)`, the exact spelling of
            // the bodies that need no buffer, and pass 2 lowers the `map` into a fresh store
            // (loft#1178).  The two passes are not looking at the same tail.
            //
            // A CAPTURE tail was carved out here as *"a fact rather than a prediction"*, and it
            // is neither: it is the same pass-1 read this sweep exists to stop trusting.  Pass 1
            // does not desugar a comprehension, so `{ cap.v.map(…) }` leaves the very tail
            // `{ cap.xs }` leaves — a projection off the capture — while pass 2 lowers it into a
            // fresh store and grows the attribute.  Reserving for the capture tail as well costs
            // nothing: `classify_ret_promotion` still answers `SkipCaptured` for a body that
            // really does hand back the capture, so the buffer stays unfilled and out of the
            // return deps, which is what lets `State::fn_return` (loft#1179) and the native
            // dispatch's same value-question release it.  loft#1182's guard measures that.
            let delivers_a_collection =
                matches!(def.returned().ret_promo_base(), Type::Vector(_, _));
            // loft#1188 — a declared `-> S` / record-enum lambda is reserved for too, and for
            // the reason #675 gave: what pass 1 can classify is not a property of the
            // SPELLING but of what was RESOLVED when it read the body.  `fn(v: integer) -> P
            // { q.p }` reserves nothing while `Q.p` is a forward reference — pass 1 types the
            // tail `Unknown`, so the #306 view materialisation never fires — and pass 2, which
            // has the resolved field, mints `__ref_1` and grows the arity.  Declaration order
            // is free in loft, so the two orderings have to compile the same; and the type is
            // resolved HERE, between the passes, which is the whole reason this sweep exists.
            //
            // A record tail rooted in a CAPTURE is included, where the collection one above is
            // not, and the difference is a fact about the two legs rather than an omission: a
            // `-> S` return DELIVERS a copy (#306 materialises the view into the buffer), while
            // `-> vector<…> { q.xs }` hands back the capture's own store and has nothing to
            // place (loft#1182, `formal/closures.md` D-clo-14).
            //
            // A lambda pass 1 already served carries a hidden heap attribute and is skipped by
            // the guard below, so this cannot mint a second buffer.
            let delivers_a_record = matches!(
                def.returned().ret_promo_base(),
                Type::Reference(_, _) | Type::Enum(_, true, _)
            );
            if def.name().starts_with("n___lambda_")
                && !self.adopted_ret_defs.contains(&d)
                && !delivers_a_collection
                && !delivers_a_record
            {
                continue;
            }
            // Through `ret_promo_base`, which is the same read the two `delivers_a_*`
            // questions above are asked with.  Asking this one on the RAW type made the
            // sweep contradict itself on a nullable collection: `-> vector<τ>?` answered
            // "delivers a collection" and was then rejected here, so no buffer was
            // reserved — while the peel had already admitted it to the promotion pass,
            // which then GREW the attribute in pass 2 and aborted the compiler on the H5
            // two-pass contract.  One question, one spelling.
            //
            // The buffer's own type is the BASE, matching the signature-time reservation
            // in `definitions.rs`: the buffer is storage, and storage is never absent —
            // the `?` belongs to the RETURN, which is a value the caller reads.
            let ret = def.returned().ret_promo_base().clone();
            // The signature's own question (`Type::takes_ret_buffer`): a record takes none.
            if !def.returned().takes_ret_buffer() {
                continue;
            }
            // Already served — either the signature-time reservation fired, or pass 1
            // promoted a local into the buffer and renamed the attribute to it.
            if def.attributes().iter().any(|a| {
                a.hidden
                    && matches!(
                        a.typedef,
                        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _)
                    )
            }) {
                continue;
            }
            let a = self
                .data
                .add_attribute(&mut self.lexer, d, "__retbuf", ret.clone());
            self.data.definitions[d as usize].attributes[a].hidden = true;
            let f = &mut self.data.definitions[d as usize].variables;
            let v = f.add_temp_var("__retbuf", &ret);
            if v != u16::MAX {
                f.become_argument(v);
                f.mark_used(v);
            }
        }
    }

    pub(super) fn assert_pass2_def_attr_stable(&self, pass1_attr_counts: &[usize]) {
        // Pass-2 def GROWTH has exactly one legal form (the fuzzer's F1 catch):
        // the reduce/map/filter builtin family desugars on pass 2 only — pass 1
        // early-returns the result type because unresolved lambda/forward types
        // make the full desugar impossible there — and the desugar machinery
        // lazily mints synthetic vector wrapper defs (`vector<T>` +
        // `main_vector<T>`, `Data::vector_def`) that pass 1 never reached.  For
        // `map` the OUTPUT element wrapper is unknowable in pass 1 (the lambda's
        // return type), so symmetric pass-1 minting is structurally impossible.
        // These mints are name-keyed, idempotent APPENDS: every pass-1 def
        // number is untouched, so the numbering contract H5 protects holds.
        // Anything else appearing only in pass 2 is a real cross-pass bug.
        // The second legal append: a GENERIC INSTANTIATION.  Instantiation is
        // pass-2-only BY DESIGN (`parse_call`: pass 1 only predicts the return
        // type — instantiating there would capture the template's still-being-
        // built body IR), so the monomorphised `t_<LEN><Type>_<fn>` def first
        // exists in pass 2.  Recognized precisely: the mangled tail must name
        // an existing `n_<fn>` template of `DefType::Generic`.
        for d in pass1_attr_counts.len()..self.data.definitions.len() {
            let name = self.data.def(d as u32).name();
            let dt = self.data.def_type(d as u32);
            let lazy_wrapper = (matches!(dt, DefType::Vector) && name.starts_with("vector<"))
                || (matches!(dt, DefType::Struct) && name.starts_with("main_vector<"));
            let lazy_instantiation =
                matches!(dt, DefType::Function) && self.h5_names_a_generic_template(name);
            // loft#763 — the third legal append: a parametric BOUND STUB. A generic's
            // body calling a bound method (`d.db_exec(…)`) mints `t_<LEN><T>_<m>` for
            // the TYPE VARIABLE rather than for a concrete type, and like an
            // instantiation that is pass-2-only by design — pass 1 only predicts.
            let lazy_bound_stub =
                matches!(dt, DefType::Function) && self.h5_names_a_bound_stub(name);
            // loft#944 — the fourth legal append: a synthetic TUPLE struct whose member was
            // a forward reference.  `tuple_def` derives both the def NAME and the frozen
            // element layout from the members' spellings, so it refuses to mint one while a
            // member is still `Type::Unknown` — an unresolved member spells `"unknown"` and
            // sizes as ZERO WIDTH, and the early-return on the name lookup means neither is
            // ever revisited.  That makes the mint pass-2-only for exactly the same reason
            // `map`'s output wrapper is: the input the name is derived from does not exist
            // in pass 1.  Same append shape too — name-keyed, idempotent, at the end, so
            // every pass-1 def number is untouched and the numbering contract H5 protects
            // holds.
            let lazy_tuple = matches!(dt, DefType::Struct) && name.starts_with("__tuple<");
            // @PLN162 — the fifth legal append: a synthesised DYNAMIC DISPATCHER
            // (`parser::dispatch::dynamic_dispatcher`).  Pass-2-only by design for the
            // reason an instantiation is: the overload set it enumerates is complete only
            // after pass 1, so pass 1 keeps the static selection and never emits it.  The
            // same append shape too — name-keyed, idempotent, at the end.
            let lazy_dispatcher = matches!(dt, DefType::Function)
                && self.data.def(d as u32).synthetic() == Some("dynamic_dispatcher");
            // The sixth: a template LAMBDA's instance and its closure record, minted with the
            // instance that owns them — pass-2-only for the reason an instantiation is.
            let lazy_lambda_instance = matches!(
                self.data.def(d as u32).synthetic(),
                Some("lambda_instance" | "closure_instance")
            );
            // The seventh: an instance of a generic struct (@PLN165 D3/D4) whose argument
            // was a forward reference on pass 1 — `Box { v: later() }` with `later` declared
            // below, `x: Box<Q>` above `struct Q`.  `instance_def` refuses an unresolved
            // argument for `tuple_def`'s two reasons (the name and the layout are both
            // derived from it) and has its append shape: name-keyed, idempotent, at the end.
            let lazy_struct_instance =
                matches!(dt, DefType::Struct) && self.data.def(d as u32).instance_of != u32::MAX;
            // The eighth: a DESTRUCTURING `par` loop's worker wrapper,
            // `__par_destructure_w_<line>_<col>_<worker>` — pass-2-only by design (pass 1
            // answers with the user's worker; `parse_destructure_par_worker` mints the wrapper
            // once the tuple's element types and offsets are final), minted once per loop site
            // and appended at the end.  Unrecognised, every `for (a, b) in v par(…)` stopped
            // the compiler with this assertion on both backends, while the `code!` harness,
            // which does not run H5, kept its tests green (loft#1802's matrix).
            let lazy_par_wrapper =
                matches!(dt, DefType::Function) && name.starts_with("__par_destructure_w_");
            assert!(
                lazy_wrapper
                    || lazy_instantiation
                    || lazy_bound_stub
                    || lazy_tuple
                    || lazy_dispatcher
                    || lazy_lambda_instance
                    || lazy_struct_instance
                    || lazy_par_wrapper,
                "H5: pass-2-only definition `{name}` (#{d}, {dt:?}) is not a lazy vector \
                 wrapper or generic instantiation — a real cross-pass divergence \
                 (pass1={}, pass2={})\n{}",
                pass1_attr_counts.len(),
                self.data.definitions.len(),
                self.h5_divergence_report(pass1_attr_counts.len()),
            );
        }
        for (d, &c1) in pass1_attr_counts.iter().enumerate() {
            let c2 = self.data.attributes(d as u32);
            // The attr-level lazy appends (both PROVEN identical on origin/main,
            // i.e. long-latent, when the DA calibration first checked them):
            // `__closure` — the capture hidden-arg is positioned in pass 2 from
            // pass 1's closure record (`parse_lambda*`: captures are only known
            // after the body parses); `__work_N` — a text-return work-buffer
            // promotion the pass-1 classify could not yet see.  Both are
            // name-keyed TRAILING appends: every pass-1 attr keeps its index.
            // Anything else — notably `__ref_N` / `__retbuf` growth, the
            // ref_return drift class this assert was built for — stays fatal.
            if c2 > c1 {
                for a in c1..c2 {
                    let n = self.data.attr_name(d as u32, a);
                    // A heap RETURN buffer is NOT on this list, and #675 is why it does
                    // not need to be: `__ref_N` / `__retbuf` growth in pass 2 was a real
                    // divergence (`glb`'s `-> Vec3` is `Unknown` at pass-1 signature time,
                    // so @PLAN59's reservation never fired), and it is now fixed at the
                    // source — `reserve_late_return_buffers` reserves the buffer between
                    // the passes, where every type IS resolved.  This assert stayed strict
                    // through that fix, which is what proves the fix rather than a hole.
                    assert!(
                        n == "__closure" || n.starts_with("__work_"),
                        "H5 two-pass contract: def `{}` (#{d}) grew a pass-2-only \
                         attribute `{n}` (pass1={c1}, pass2={c2}) that is not a \
                         documented lazy append — a real cross-pass divergence",
                        self.data.def(d as u32).name(),
                    );
                    // The `__work_` exemption is only safe while NO call has been
                    // lowered against the old arity.  loft#662 was exactly this
                    // append on a SELF-recursive function: its own body had already
                    // emitted a call, so growing the signature left that call one
                    // argument short — and the exemption waved it through, which is
                    // why this assert never caught the bug it was built for.  A
                    // caller is already lowered iff it is this def itself or a def
                    // parsed before it, so that is the condition to reject.
                    assert!(
                        !n.starts_with("__work_") || !self.h5_has_lowered_caller(d as u32),
                        "H5 two-pass contract: def `{}` (#{d}) grew a pass-2-only \
                         work buffer `{n}` (pass1={c1}, pass2={c2}) AFTER a call to it \
                         had already been lowered — the call now passes {c1} arguments \
                         to a {c2}-argument signature (loft#662's class).  The buffer's \
                         name counter is not pass-stable for this def.",
                        self.data.def(d as u32).name(),
                    );
                }
                continue;
            }
            debug_assert_eq!(
                c1,
                c2,
                "H5 two-pass contract: def `{}` (#{d}) attribute count diverged across \
                 passes (pass1={c1}, pass2={c2})",
                self.data.def(d as u32).name(),
            );
        }
    }

    /// H5 helper: is there already-lowered code calling `d`?
    ///
    /// Pass 2 lowers defs in source order, so every def numbered at or below `d`
    /// has emitted its calls by the time `d`'s body finishes and its return
    /// promotion runs.  A signature that grows at that point leaves those calls
    /// short an argument.  Only scanned when a def actually grew a pass-2-only
    /// work buffer — rare (once across the whole test corpus), so the walk costs
    /// nothing in the common case.
    pub(super) fn h5_has_lowered_caller(&self, d: u32) -> bool {
        fn calls(node: &Value, target: u32) -> bool {
            // Direct calls only.  A `CallRef` goes through the fn-ref dispatch,
            // which allocates hidden buffers at runtime rather than baking the
            // arity into the call site, so it is not committed to the old
            // signature the way a lowered direct call is.
            let hit = matches!(node.unspan(), Value::Call(op, _) if *op == target);
            if hit {
                return true;
            }
            let mut found = false;
            node.for_each_child(&mut |c| {
                if !found && calls(c, target) {
                    found = true;
                }
            });
            found
        }
        (0..=d).any(|e| calls(&self.data.def(e).code, d))
    }

    /// H5 helper: does `name` carry the instantiation mangling
    /// `t_<LEN><SafeType>_<fn>` (see `try_generic_instantiation`) AND does the
    /// `n_<fn>` template exist as a `DefType::Generic`?  Only such defs are
    /// legal pass-2-only appends of the Function kind — a source-declared
    /// method parses in pass 1 and can never appear as a trailing pass-2 def.
    ///
    /// A METHOD template (`fn head<T>(self: vector<T>)`) is stored under its receiver's key,
    /// `t_6vector_head`, rather than as `n_head`, and instantiates on pass 2 the same way
    /// (loft#1539) — so a template of either spelling makes the name legal.
    pub(super) fn h5_names_a_generic_template(&self, name: &str) -> bool {
        // `D-Key` — an instance's key names its template outright.
        if let Some(key) =
            Data::split_key(name).filter(|k| k.kind == crate::data::KeyKind::Instance)
        {
            // The template may live only under its library's source: a qualified call
            // (`lib::f(…)`) instantiates it without the caller importing the name (@C98).
            let g = self.data.def_nr(key.rest);
            return (g != u32::MAX && matches!(self.data.def_type(g), DefType::Generic))
                || (0..self.data.definitions()).any(|d| {
                    matches!(self.data.def_type(d), DefType::Generic)
                        && self.data.def(d).name() == key.rest
                });
        }
        let Some((_, fn_name)) = Self::h5_split_mangled(name) else {
            return false;
        };
        let g_nr = self.data.def_nr(&format!("n_{fn_name}"));
        if g_nr != u32::MAX && matches!(self.data.def_type(g_nr), DefType::Generic) {
            return true;
        }
        (0..self.data.definitions()).any(|d| {
            matches!(self.data.def_type(d), DefType::Generic)
                && Self::h5_split_mangled(self.data.def(d).name())
                    .is_some_and(|(_, method)| method == fn_name)
        })
    }

    /// Split `t_<LEN><Type>_<fn>` into its type name and function name.
    pub(super) fn h5_split_mangled(name: &str) -> Option<(&str, &str)> {
        Data::split_key(name)
            .filter(|k| k.kind == crate::data::KeyKind::Method)
            .map(|k| (k.spelling, k.rest))
    }

    /// loft#763 — does `name` mangle a method on a generic's TYPE VARIABLE?
    ///
    /// `fn helper <D: SqlDb> (d: D, …) { d.db_exec(…) }` resolves `db_exec` against the
    /// bound and mints `t_1D_db_exec` — a stub keyed on the type PARAMETER, not on any
    /// concrete type, so `h5_names_a_generic_template` does not know it (there is no
    /// `n_db_exec` template; `db_exec` is an interface method). It is minted in pass 2
    /// for the same reason an instantiation is: pass 1 only predicts.
    ///
    /// Recognised precisely — the type part has to be the type variable of some generic
    /// template in THIS program, so an ordinary `t_<Type>_<fn>` appearing only in pass 2
    /// stays the fatal divergence the guard was built for.
    pub(super) fn h5_names_a_bound_stub(&self, name: &str) -> bool {
        // loft#1153 — EXACT, by the marker `Data::bound_stub_name` mints, rather than inferred
        // from the shape of whatever def the type name happens to resolve to.  A stub whose
        // holder has a FORWARD-REFERENCED bound is legitimately pass-2-only (`SqlDb.Both` gains
        // two of its three stubs on pass 2), and the inferential test below could only reach
        // that by looking the holder up — which the marked name defeats and no longer needs.
        if crate::data::Data::is_bound_stub_name(name) {
            return true;
        }
        let Some((type_name, _)) = Self::h5_split_mangled(name) else {
            return false;
        };
        let type_nr = self.data.def_nr(type_name);
        if type_nr == u32::MAX {
            return false;
        }
        // @PLN125 — the other holder of bound stubs is an interface's ASSOCIATED TYPE,
        // whose stubs are equally pass-2-only when the interface it is bounded by is
        // declared further down the file. Recognised the same way the rest of the parser
        // recognises one: a `Struct` child of an `Interface`.
        let parent = self.data.def(type_nr).parent();
        if matches!(self.data.def_type(type_nr), DefType::Struct)
            && parent != u32::MAX
            && matches!(self.data.def_type(parent), DefType::Interface)
        {
            return true;
        }
        (0..self.data.definitions()).any(|g| {
            matches!(self.data.def_type(g), DefType::Generic)
                && Self::template_vars(&self.data, g).contains(&type_nr)
        })
    }

    /// Plan-07 phase 4h — walk user struct definitions and emit
    /// `Level::Warning` for each non-`not_null` field whose read
    /// count exceeds `HINT_NOT_NULL_THRESHOLD` AND whose
    /// `defended_field_reads` set is empty.  The hint suggests
    /// adding `not null` at the field declaration so the
    /// constructor enforces non-null at write-time, eliminating
    /// the entire class of fault sites for that field.
    ///
    /// Threshold is conservative (10 reads) to keep the hint
    /// from firing on small / illustrative struct definitions.
    /// Lower thresholds + smarter defended-detection are slice 2.
    ///
    /// Stdlib (`self.default == true`) is exempt — its struct
    /// definitions are language-internal and the suggestion target
    /// is user code, not the host library.
    pub(super) fn emit_not_null_hints(&mut self) {
        const HINT_NOT_NULL_THRESHOLD: u32 = 10;
        // @PLN25 DN1/F2: the `not null` hint is RETIRED. Under the dense model a plain scalar is
        // NON-null by default (nullability rides `?`/`Optional`), so `not null` is redundant (and
        // being retired), and the only nullable form left is `τ?` — for which suggesting
        // `not null` is contradictory. Superseded like the div/index fault warnings. (Also, under
        // F2 a heavily-read plain field is non-null → never accrues, so the hint would only ever
        // fire on a `?` field.)
        if crate::keys::pln25_dn1_enabled()
            || crate::env_once!(
                std::env::var("LOFT_NO_HINT_NOT_NULL").is_ok_and(|v| v == "1" || v == "true")
            )
        {
            self.field_read_counts.clear();
            self.defended_field_reads.clear();
            return;
        }
        // Snapshot the keys so we can borrow `self.lexer` mutably
        // while iterating.
        let counts: Vec<((u32, u32), u32)> = self
            .field_read_counts
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        for ((d_nr, attr_idx), count) in counts {
            if count < HINT_NOT_NULL_THRESHOLD {
                continue;
            }
            if self.defended_field_reads.contains(&(d_nr, attr_idx)) {
                continue;
            }
            // Re-check `nullable` at emission time — definition
            // mutations between count + emit are unlikely but not
            // forbidden.
            let attrs = self.data.def(d_nr).attributes();
            if attr_idx as usize >= attrs.len() {
                continue;
            }
            let attr = &attrs[attr_idx as usize];
            if !attr.nullable {
                continue;
            }
            let struct_name = self.data.def(d_nr).name().to_string();
            let field_name = attr.name.clone();
            let pos = *self.data.def(d_nr).position();
            self.lexer.pos_diagnostic(
                Level::Warning,
                &pos,
                &format!(
                    "field `{struct_name}.{field_name}` is read {count} times and never \
                     defended with `??` or `if x.{field_name} != null` — consider \
                     marking it `not null` so the constructor enforces non-null at \
                     write-time"
                ),
            );
        }
        self.field_read_counts.clear();
        self.defended_field_reads.clear();
    }

    /// after `parse_file` has run (and all `todo_files` have drained,
    /// so every file in the recursion has had its definitions registered),
    /// reconcile any `DefType::Unknown` stubs that `actual_types_deferred`
    /// collected during parsing.
    ///
    /// The cyclic `use` case: file B references a type `Player` defined in
    /// file A, but B's `use A;` fires while A is suspended mid-parse — so
    /// B's body parsed with `Player` as a stub.  After the full recursion
    /// returns, A's `Player` is registered; Phase C re-applies imports with
    /// overwrite semantics (replacing B's stub binding with A's real def),
    /// then rewrites every `Type::Unknown(stub_nr)` occurrence to the real
    /// resolved type.
    ///
    /// Stubs that remain unresolved after this reconciliation surface as
    /// the original "Undefined type" error at the stored `Position`.
    pub(super) fn resolve_deferred_unknowns(&mut self) {
        // loft#1023 — every template body now exists for this pass, so any monomorph
        // derived from a PASS-1 body can be derived again from the real one.  Empty (and
        // so a no-op) on the first pass, which records nothing.
        self.rebuild_stale_monomorphs();
        // Step 1: re-apply all previously-applied imports with overwrite
        // semantics.  This replaces any target-source `Unknown` stub with
        // the now-registered real def in the library source.
        let applied = std::mem::take(&mut self.applied_imports);
        for pi in &applied {
            self.data.import_operators(pi.lib_source, pi.for_source);
            match &pi.spec {
                ImportSpec::Wildcard => {
                    self.data
                        .import_all_overwrite(pi.lib_source, pi.for_source, pi.public);
                }
                ImportSpec::Names(names) => {
                    for (name, bind) in names {
                        self.data.import_name_overwrite(
                            pi.lib_source,
                            pi.for_source,
                            name,
                            bind,
                            pi.public,
                        );
                    }
                }
            }
        }
        // Keep them on the list for any later pass (pass 2 re-populates).
        self.applied_imports = applied;
        // A by-name import deferred because its library was suspended: every file has
        // registered now, so a name still missing really is missing.
        for (lib, into, name, bind, public, pos) in std::mem::take(&mut self.deferred_import_misses)
        {
            if !self.data.import_name(lib, into, &name, &bind, public) {
                self.lexer.pos_diagnostic(
                    Level::Error,
                    &pos,
                    &format!("Name '{name}' not found in library"),
                );
            }
        }

        // Step 2: for each deferred stub, resolve via the post-import
        // def binding.  Three outcomes per stub:
        //
        //  (a) The stub def got UPGRADED in-place to a real type (most
        //      common — `parse_struct` does this when it finds an
        //      existing stub by name).  `def(stub_nr).def_type` is no
        //      longer `Unknown`; call `rewrite_unknown_refs(stub, stub)`
        //      so that `Type::Unknown(stub)` references resolve to
        //      `def(stub).returned`.
        //
        //  (b) The stub's source has a DIFFERENT real def (e.g. when
        //      `import_all_overwrite` just routed the source-level
        //      binding to a real def from another source).  Rewrite
        //      Unknown references to point at that real def.
        //
        //  (c) Still unresolved — emit the "Undefined type" error at
        //      the stored `Position`.
        let deferred = std::mem::take(&mut self.deferred_unknown);
        for (source, stub_nr, pos) in deferred {
            let stub_name = self.data.def(stub_nr).name().to_string();
            // Case (a): stub upgraded in place
            if !matches!(self.data.def(stub_nr).def_type(), DefType::Unknown) {
                self.data.rewrite_unknown_refs(stub_nr, stub_nr);
                continue;
            }
            // Case (b): lookup via post-import source binding
            let resolved_nr = self.data.source_nr(source, &stub_name);
            if resolved_nr != u32::MAX
                && resolved_nr != stub_nr
                && !matches!(self.data.def(resolved_nr).def_type(), DefType::Unknown)
            {
                self.data.rewrite_unknown_refs(stub_nr, resolved_nr);
                continue;
            }
            // A stub pass 1 registered on SPECULATION, from a `Name { … }` construction
            // alone, is not evidence that the author wrote a type name: the same shape is
            // what a typo makes.  It has served its purpose either way — a real
            // declaration would have adopted it in case (a) — so an unadopted one is left
            // to the construction site, which reports in pass 2 with the author's own
            // spelling and a suggestion.  Reporting here as well is one typo, two errors.
            if self.speculative_type_refs.contains(&stub_nr) {
                continue;
            }
            // Case (c): emit the deferred error.  `string` used to be special-cased
            // here (and in `typedef.rs`); it is now one row of the cross-language
            // alias table `suggest_type_name` consults, so both deferred and direct
            // sites word it identically from one home.
            // loft#826 — same boundary as the unknown-function case, reached the
            // other way.  A type the importer declares resolves here only for
            // whichever used file's stub the importer's declaration happened to
            // adopt; a second used file naming the same type is left with a stub
            // nothing upgrades, and "Undefined type Thing" about a type declared
            // one file away reads as a compiler fault rather than a scope rule.
            //
            // `self.data.source` drives `declared_by_importer`, and the stub's
            // own source is the file that asked — not necessarily the one the
            // parser rests on once every file has drained.
            let saved_source = self.data.source;
            self.data.source = source;
            let boundary = self.importer_boundary_note(&stub_name);
            self.data.source = saved_source;
            let msg = if let Some(q) = self.qualified_stubs.get(&stub_nr) {
                format!("Undefined type {q}::{stub_name} — `{q}` declares no type `{stub_name}`")
            } else if let Some(note) = boundary {
                note
            } else if let Some(msg) = self.data.import_cure("Undefined type", &stub_name, source) {
                msg
            } else if let Some(msg) = registry_type_hint(
                "Undefined type",
                &stub_name,
                &self.data.resolved_libraries(),
            ) {
                msg
            } else if self.data.is_type_var_spelling(&stub_name) {
                // `D-Scope` / @FR-G-Gen-Scope — a header's variable holds no name, so a
                // spelling outside every header that declares it reaches here as undefined.
                Self::type_var_out_of_scope(&stub_name)
            } else if let Some(s) = self.data.suggest_type_name(&stub_name) {
                format!("Undefined type {stub_name} — did you mean '{s}'?")
            } else {
                format!("Undefined type {stub_name}")
            };
            self.lexer.pos_diagnostic(Level::Error, &pos, &msg);
        }
    }

    /// After both parse passes, every `#native "<sym>"` annotation should map
    /// to its owning native package crate in `native_symbol_crates`.  If the
    /// manifest was registered before the .loft source that declared the
    /// native symbol was parsed, the original mapping pass in
    /// `lib_path_manifest` / `register_native_manifest` saw no definitions
    /// and left the symbol unmapped — which later surfaces as a `todo!()`
    /// stub in the `--native` output and a runtime panic.
    ///
    /// Walk every definition once more and bind each `#native` symbol still
    /// missing from the map to the native package that OWNS its definition:
    /// the registered package whose directory is a prefix of the def's source
    /// file (longest prefix wins, for nested packages).  Ownership-by-path is
    /// the same invariant the per-manifest pass enforces, run once at the end
    /// when every def exists, so it covers ANY number of native packages.
    /// (The earlier `len() == 1` shortcut silently skipped programs using two
    /// native packages — e.g. `graphics` + `random` — leaving BOTH unmapped:
    /// the interpreter still dispatched them via `def.native` + dlopen, but
    /// `--native` rejected the first reachable call with a P269 compile error.)
    pub(crate) fn backfill_native_symbol_crates(&mut self) {
        if self.data.native_packages.is_empty() {
            return;
        }
        let mut binds: Vec<(String, String)> = Vec::new();
        for d_nr in 0..self.data.definitions() {
            let def = self.data.def(d_nr);
            let sym = def.native();
            if sym.is_empty() || self.data.native_symbol_crates.contains_key(sym) {
                continue;
            }
            let sym = sym.to_string();
            let file = def.position().file;
            // Owner = the registered native package whose dir is the longest
            // prefix of this def's source file.
            if let Some((crate_name, _)) = self
                .data
                .native_packages
                .iter()
                .filter(|(_, pkg_dir)| crate::file_access::is_under(file.as_str(), pkg_dir))
                .max_by_key(|(_, pkg_dir)| pkg_dir.len())
            {
                binds.push((sym, crate_name.replace('-', "_")));
            }
        }
        for (sym, rust_crate) in binds {
            self.data.native_symbol_crates.insert(sym, rust_crate);
        }
    }

    /// Parse in-memory `content` as if it were the file at `filename` — the
    /// filesystem-free twin of [`parse`](Self::parse), available on every target
    /// (unlike the wasm-only `parse_virtual`).  `@PLN98` P3.1 uses it to bootstrap
    /// the live interpreter from EMBEDDED stdlib + program blobs (no `default/`
    /// dir, no `LOFT_LIVE_SRC` file) — the delivery a browser build needs.  Mirrors
    /// `parse`'s two-pass + `MAIN_SOURCE` assignment (a non-default file shadows a
    /// prelude name rather than colliding on `(name, 0)`), so an embedded bootstrap
    /// parks the SAME world the fs path does.  Returns `true` when the parse is
    /// diagnostic-clean.
    pub fn parse_source(&mut self, content: &str, filename: &str, default: bool) -> bool {
        // @PLN13 — establish `source_dir` from `filename` (like `parse`) so a `--script`
        // run resolves `use` imports + relative I/O against the script's own directory.
        // loft#1260 — say who this compilation's lints are addressed to.  Both instances:
        // the lexer's is what the parse writes into, and `p.diagnostics` is what the
        // post-scope lints are handed.  Same place and same entry file as `source_dir`
        // just below, which answers the same shape of question.
        if !default {
            self.lexer.set_lint_scope_for(filename);
            self.diagnostics.set_lint_scope_for(filename);
        }
        if !default && self.database.source_dir.is_empty() {
            self.database.source_dir = std::path::Path::new(filename)
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();
        }
        if !default && self.own_lib.is_none() {
            // The manifest's `[package] name` when there is one, else the file
            // stem — the same answer for a conventional package, whose entry
            // defaults to `src/<name>.loft`.
            let path = std::path::Path::new(filename);
            let from_manifest = path
                .parent()
                .and_then(std::path::Path::parent)
                .map(|root| root.join("loft.toml"))
                .filter(|m| crate::file_access::exists(m))
                .and_then(|m| crate::manifest::read_manifest(&m.to_string_lossy()))
                .and_then(|m| m.name);
            self.own_lib = from_manifest.or_else(|| crate::file_access::file_stem(path));
        }
        self.default = default;
        self.vars.logging = false;
        self.first_pass = true;
        crate::diagnostics::set_first_pass(true);
        self.pending_imports.clear();
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.deferred_import_misses.clear();
        self.resolutions.clear();
        self.data.reset();
        // @PLN22 — the main program parses under MAIN_SOURCE (not the prelude's
        // source 0), matching `parse`; `reset()` left source at STD_SOURCE.
        if !default {
            self.data.source = crate::data::MAIN_SOURCE;
        }
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.lexer.parse_string(content, filename);
        if !default && self.program_entry.is_none() {
            self.program_entry = Some(self.lexer.pos().file);
        }
        self.parse_file();
        self.resolve_deferred_unknowns();
        self.between_passes();
        let lvl = self.lexer.diagnostics().level();
        if lvl != Level::Error && lvl != Level::Fatal {
            self.first_pass = false;
            crate::diagnostics::set_first_pass(false);
            self.applied_imports.clear();
            self.deferred_unknown.clear();
            self.deferred_import_misses.clear();
            self.resolutions.clear();
            self.data.reset();
            if !default {
                self.data.source = crate::data::MAIN_SOURCE;
            }
            self.lambda_counter = 0;
            self.fn_lambdas.clear();
            self.lexer.parse_string(content, filename);
            self.parse_file();
            self.resolve_deferred_unknowns();
            // @PLN130 F9 (loft#779) — see `parse`.
            self.check_subrule_wellformedness();
            self.after_pass2();
            if !default {
                self.check_reshape_under_reference();
                self.synth_drop_cascades();
            }
        }
        self.diagnostics.fill(self.lexer.diagnostics());
        self.diagnostics.is_empty()
    }

    /// Parse `content` as if it were the file at `filename`.
    /// Used by the WASM virtual-FS path to bypass real filesystem access.
    #[cfg(feature = "wasm")]
    pub(super) fn parse_virtual(&mut self, content: &str, filename: &str, default: bool) -> bool {
        // @PLN11 arc E — record the input file for the whole-program cache key.
        if self.track_sources {
            self.parsed_sources.push(filename.to_string());
        }
        self.default = default;
        self.vars.logging = false;
        self.first_pass = true;
        crate::diagnostics::set_first_pass(true);
        self.pending_imports.clear();
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.deferred_import_misses.clear();
        self.resolutions.clear();
        self.data.reset();
        self.lambda_counter = 0;
        self.fn_lambdas.clear();
        self.lexer.parse_string(content, filename);
        self.parse_file();
        self.resolve_deferred_unknowns();
        self.between_passes();
        let lvl = self.lexer.diagnostics().level();
        if lvl != Level::Error && lvl != Level::Fatal {
            self.first_pass = false;
            crate::diagnostics::set_first_pass(false);
            self.applied_imports.clear();
            self.deferred_unknown.clear();
            self.deferred_import_misses.clear();
            self.resolutions.clear();
            self.data.reset();
            self.lambda_counter = 0;
            self.fn_lambdas.clear();
            self.lexer.parse_string(content, filename);
            self.parse_file();
            self.resolve_deferred_unknowns();
            // @PLN130 F9 (loft#779) — see `parse`.
            self.check_subrule_wellformedness();
            self.after_pass2();
            if !default {
                self.check_reshape_under_reference();
                self.synth_drop_cascades();
            }
        }
        self.diagnostics.fill(self.lexer.diagnostics());
        self.diagnostics.is_empty()
    }

    /// Parse all .loft files found in a directory tree in alphabetical ordering.
    /// # Errors
    /// With filesystem problems.
    pub fn parse_dir(&mut self, dir: &str, default: bool, debug: bool) -> std::io::Result<()> {
        self.parse_dir_inner(dir, default, debug)?;
        // The stdlib is checked ONCE, here at the top-level call and never inside the
        // recursion below, where the list is still being filled: its operator declarations
        // must be, slot for slot, the ones this binary's dispatch table was generated from,
        // or the refusal is the ONLY thing standing between the run and a corrupt
        // reference (`stdlib_ops`).
        if default && let Err(msg) = crate::stdlib_ops::verify(&self.data, dir) {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, msg));
        }
        Ok(())
    }

    /// A parser ready for a whole program to be loaded into it: the standard library from
    /// `stdlib_dir` parsed, `lib_dirs` as the `use` search path, and the world OPEN — it is
    /// parsed against again after compiling, so no whole-program signature rewrite.  The one
    /// way the REPL session and the live-reload shadow start a program parse.
    /// # Errors
    /// As [`Self::parse_stdlib`].
    pub fn open_program(stdlib_dir: &str, lib_dirs: &[String]) -> std::io::Result<Parser> {
        let mut parser = Parser::new();
        parser.data.open_world = true;
        parser.lib_dirs = lib_dirs.to_vec();
        parser.parse_stdlib(stdlib_dir)?;
        Ok(parser)
    }

    /// Load the standard library from `dir`, or — when there is no such directory — from
    /// the sources this binary embeds (loft#1801).  A binary copied onto the PATH or
    /// installed by `cargo install` has no `default/` beside it, but it carries the stdlib
    /// it was built from (`stdlib_sources`, the same text the browser build and `loft
    /// search` read), and that text cannot mismatch its dispatch table.  A directory that
    /// exists is read and checked exactly as [`Self::parse_dir`] does.
    /// # Errors
    /// As [`Self::parse_dir`]; and `InvalidData` if the embedded stdlib does not parse.
    pub fn parse_stdlib(&mut self, dir: &str) -> std::io::Result<()> {
        // An empty name names nothing (`file_access` would answer for `.`).
        if !dir.is_empty() && crate::file_access::is_dir(dir) {
            return self.parse_dir(dir, true, false);
        }
        for (name, content) in crate::stdlib_sources::STDLIB_SOURCES {
            if !self.parse_source(content, name, true) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("the embedded standard library `{name}` does not parse"),
                ));
            }
        }
        Ok(())
    }

    /// [`Self::parse_dir`] WITHOUT the stdlib check, for the one caller that must read a
    /// `default/` the dispatch table does not match yet: the `src/fill.rs` generator, whose
    /// job is to repair exactly that mismatch.  Checked, it refused the parse and the cure
    /// the refusal names (`make fill`) could not run — found joining two branches that each
    /// changed the operator list.  Every program load goes through [`Self::parse_dir`].
    /// # Errors
    /// With filesystem problems.
    pub fn parse_dir_unchecked(
        &mut self,
        dir: &str,
        default: bool,
        debug: bool,
    ) -> std::io::Result<()> {
        self.parse_dir_inner(dir, default, debug)
    }

    pub(super) fn parse_dir_inner(
        &mut self,
        dir: &str,
        default: bool,
        debug: bool,
    ) -> std::io::Result<()> {
        let paths = crate::file_access::read_dir(dir)?;
        let mut files: BTreeSet<String> = BTreeSet::new();
        for p in &paths {
            let name = p.file_name().unwrap_or_default();
            // A hidden entry is no part of the library: `.loft/` is the cache and log a run
            // writes beside its sources, `.git/` a checkout's history.  Walking one would make
            // what a load reads — and what it costs — depend on what earlier runs left in the
            // directory (loft#1761).
            if name.starts_with('.') {
                continue;
            }
            let own_file = p
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("loft"));
            // Spelled as `dir` was given (`./lib/x.loft` stays `./lib/x.loft`): it becomes the
            // source position every diagnostic names.
            let file_name = std::path::Path::new(dir)
                .join(name)
                .to_string_lossy()
                .to_string();
            let data = crate::file_access::metadata(&file_name)?;
            if own_file || data.is_dir() {
                files.insert(file_name);
            }
        }
        for f in files {
            let types = self.database.types.len();
            let from = self.data.definitions();
            let data = crate::file_access::metadata(&f)?;
            if data.is_dir() {
                self.parse_dir_inner(&f, default, debug)?;
            } else {
                self.parse(&f, default);
                // Errors stop the load; warnings and advice do not.  `parse`
                // answers the narrower question "was anything at all reported",
                // so gating on it refuses a directory over a deprecation notice
                // — against loft's own tier rule, which is that a diagnostic
                // gates if and only if ignoring it can produce a wrong result.
                //
                // The refusal was not academic.  This is the path a
                // process-placed library's worker loads through (@PLN119), so
                // one `never-read` warning in a library made its consumer exit
                // 1 under `placement = "process"` and 0 in-process — placement
                // deciding whether the program ran at all, which is exactly the
                // invariant it must not touch.
                if self.diagnostics.level() >= Level::Error {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("{}", self.diagnostics),
                    ));
                }
            }
            scopes::check(&mut self.data, &mut self.database);
            if debug {
                self.output(&f, types, from)?;
            }
        }
        Ok(())
    }

    pub(super) fn output(&mut self, f: &str, types: usize, from: u32) -> std::io::Result<()> {
        let f_norm = f.replace(other_sep(), sep_str());
        let file = f_norm.rsplit(sep()).next().unwrap_or(f);
        let to = format!("tests/dumps/{file}.txt");
        let _ = crate::file_access::create_dir_all("tests/dumps");
        if let Ok(mut w) = crate::file_access::create(&to) {
            let to = self.database.types.len();
            for tp in types..to {
                writeln!(w, "Type {tp}:{}", self.database.show_type(tp as u16, true))?;
            }
            for d_nr in from..self.data.definitions() {
                if *self.data.def(d_nr).code() == Value::Null {
                    continue;
                }
                write!(w, "{} ", self.data.def(d_nr).header(&self.data, d_nr))?;
                let mut vars = Function::copy(self.data.def(d_nr).variables());
                self.data
                    .show_code(&mut w, &mut vars, self.data.def(d_nr).code(), 0, false)?;
                writeln!(w, "\n")?;
            }
        } else {
            diagnostic!(self.lexer, Level::Error, "Could not write: {to}");
        }
        Ok(())
    }

    /// Only parse a specific string, only useful for parser tests.
    #[allow(dead_code)]
    pub fn parse_str(&mut self, text: &str, filename: &str, logging: bool) {
        self.data.limits = self.data.limits.min(crate::limits::Limits::from_env());
        // Start each standalone parse on a fresh lexer.  `restart` resets the
        // cursor but not the diagnostics or format-mode flags, and
        // `Diagnostics::level` is monotonic — so reusing the lexer would leak a
        // prior parse's errors into this one (poisoning REPL re-entrancy: a typo
        // would make every later input mis-parse).  A new lexer is fully clean.
        self.lexer = Lexer::default();
        self.first_pass = true;
        crate::diagnostics::set_first_pass(true);
        self.default = false;
        self.vars.logging = logging;
        self.lexer.parse_string(text, filename);
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.deferred_import_misses.clear();
        self.resolutions.clear();
        self.data.reset();
        // A REPL input reuses its virtual file name with new text.
        self.declared_fn_names.clear();
        // The source stays the stdlib's, 0: this is the REPL session's entry, where every
        // later input and a debugger's eval resolve their names under that scope, and where
        // a definition colliding with a stdlib one is a collision of ONE key.  A gate that
        // asks "is this the reader's code?" therefore reads the definition's FILE, not this
        // id.
        //
        // The lambda counter CONTINUES where the session left it, and only the two passes of
        // THIS input share a start (`lambda_base` below).  Reset to 0 here, a second REPL
        // input carrying a lambda minted `__lambda_0` again while the first input's
        // definition was still in the session: *"Cannot redefine '__lambda_0'"*, so a session
        // could hold one lambda and no more.
        let lambda_base = self.lambda_counter;
        self.fn_lambdas.clear();
        self.declared_capabilities.clear();
        self.member_access.clear();
        self.sandbox_field_reads.clear();
        self.sandbox_field_updates.clear();
        self.sandbox_field_appends.clear();
        self.last_field_target = None;
        self.param_locks.clear();
        self.sandbox_param_overrides.clear();
        self.pending_param_locks.clear();
        self.parse_file();
        self.resolve_deferred_unknowns();
        self.between_passes();
        let lvl = self.lexer.diagnostics().level();
        if lvl == Level::Error || lvl == Level::Fatal {
            self.diagnostics.fill(self.lexer.diagnostics());
            return;
        }
        self.applied_imports.clear();
        self.deferred_unknown.clear();
        self.deferred_import_misses.clear();
        self.resolutions.clear();
        self.data.reset();
        // Pass 2 mints the same names pass 1 did, so it starts where pass 1 started.
        self.lambda_counter = lambda_base;
        self.fn_lambdas.clear();
        self.lexer.parse_string(text, filename);
        self.first_pass = false;
        crate::diagnostics::set_first_pass(false);
        self.parse_file();
        self.resolve_deferred_unknowns();
        // @PLN35 PC3 — reject a left-recursive sub-rule grammar (see `check_subrule_termination`).
        self.check_subrule_wellformedness();
        self.after_pass2();
        // @PLN130 F9 (loft#779) — see `parse`.
        self.check_reshape_under_reference();
        self.synth_drop_cascades();
        self.diagnostics.fill(self.lexer.diagnostics());
    }

    /// Did this parse reach the SECOND pass?
    ///
    /// `parse_source` returns early when pass 1 produced an `Error`/`Fatal`, so every
    /// pass-2 diagnostic — casts, shifts, and most semantic lints — is structurally absent
    /// from a parse that stopped there. A caller comparing two parses has to know that, or
    /// it reads an UNMASKED diagnostic as one the change caused (@PLN131).
    #[must_use]
    pub fn reached_second_pass(&self) -> bool {
        !self.first_pass
    }

    /// Session-scope snippet parse (#350, live-reload): like
    /// [`parse_str`](Self::parse_str) but KEEPS the session's import scoping —
    /// `use_names` and the applied imports survive, and both passes run under
    /// `source` (the def-source of the fn being replaced) — so the snippet
    /// resolves the same library names its original file did.  `parse_str`'s
    /// `data.reset()` exists for whole-program loads, where each pass re-runs
    /// the `use` statements and re-registers scoping in order; a snippet has
    /// no `use` lines, so the reset left it with no library scope at all
    /// ("Unknown library" on every lib-qualified name).  No library loading
    /// happens here: every `use` the program needs already ran at install.
    pub fn parse_snippet(&mut self, text: &str, filename: &str, source: u16) {
        self.lexer = Lexer::default();
        self.first_pass = true;
        crate::diagnostics::set_first_pass(true);
        self.default = false;
        self.vars.logging = false;
        self.lexer.parse_string(text, filename);
        self.deferred_unknown.clear();
        self.deferred_import_misses.clear();
        self.resolutions.clear();
        // The counter continues, as `parse_str`'s does and for the same reason: the session
        // still holds the definitions the previous input minted.
        let lambda_base = self.lambda_counter;
        self.fn_lambdas.clear();
        self.data.source = source;
        self.parse_file();
        self.resolve_deferred_unknowns();
        self.between_passes();
        let lvl = self.lexer.diagnostics().level();
        if lvl == Level::Error || lvl == Level::Fatal {
            self.diagnostics.fill(self.lexer.diagnostics());
            return;
        }
        self.deferred_unknown.clear();
        self.deferred_import_misses.clear();
        self.resolutions.clear();
        self.lambda_counter = lambda_base;
        self.fn_lambdas.clear();
        self.lexer.parse_string(text, filename);
        self.first_pass = false;
        crate::diagnostics::set_first_pass(false);
        self.data.source = source;
        self.parse_file();
        self.resolve_deferred_unknowns();
        self.after_pass2();
        // @PLN130 F9 (loft#779) — see `parse`.
        self.check_reshape_under_reference();
        self.synth_drop_cascades();
        self.diagnostics.fill(self.lexer.diagnostics());
    }

    /// @PLN12 phase 02 — does `input` end mid-construct, so a REPL should read
    /// more lines before trying to parse it?
    ///
    /// Returns `true` when a bracket is still open (`(`, `[`, `{`), a `"…"`
    /// string literal is unterminated, or the last meaningful token is a
    /// binary / continuation operator (`1 +`, `x.`).  `//` line comments and
    /// escaped quotes are skipped.  It is deliberately conservative: a missed
    /// "incomplete" just produces the ordinary parse error the caller already
    /// handles, never a crash.  Pure over the input string — no parser state.
    #[must_use]
    pub fn statement_incomplete(input: &str) -> bool {
        let chars: Vec<char> = input.chars().collect();
        let mut depth: i32 = 0;
        let (mut in_str, mut esc) = (false, false);
        let mut last: Option<char> = None;
        let mut i = 0;
        while i < chars.len() {
            let c = chars[i];
            if in_str {
                if esc {
                    esc = false;
                } else if c == '\\' {
                    esc = true;
                } else if c == '"' {
                    in_str = false;
                    last = Some('"');
                }
                i += 1;
                continue;
            }
            // `//` to end of line is a comment — skip it.
            if c == '/' && chars.get(i + 1) == Some(&'/') {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
                continue;
            }
            match c {
                '"' => in_str = true,
                '(' | '[' | '{' => {
                    depth += 1;
                    last = Some(c);
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                    last = Some(c);
                }
                w if w.is_whitespace() => {}
                other => last = Some(other),
            }
            i += 1;
        }
        if in_str || depth > 0 {
            return true;
        }
        // A trailing binary / continuation operator means more is coming.
        matches!(
            last,
            Some('+' | '-' | '*' | '/' | '%' | '&' | '|' | '^' | '=' | '<' | '>' | ',' | '.')
        )
    }

    /// @PLN12 phase 02 — parse one REPL input against the live session.
    ///
    /// Reuses the parser's accumulated `data` + `database`, so a definition
    /// from an earlier call is visible to this one.  This works because
    /// `data.reset()` (which `parse_str` calls between its two passes) clears
    /// only import scoping, never `definitions` — so `parse_str` *appends* the
    /// input's new definitions to the existing stdlib + session.
    ///
    /// Returns [`ParseResult::NeedMore`] for input that ends mid-construct,
    /// [`ParseResult::Error`] (with `data` rolled back) on a parse error, or
    /// [`ParseResult::Ready`] with the new definition's number on success.
    ///
    /// A top-level definition (`struct`/`enum`/`fn`/`type`/…) is parsed as-is.
    /// Any other input (an expression, a call, an assignment) is wrapped in a
    /// synthetic runnable `fn repl_<n>()` so it parses and can be executed;
    /// `entry_def_nr` then points at that wrapper.  Locals declared in such a
    /// statement do NOT yet persist across inputs — the `__repl_session`
    /// local-persistence path is the remaining increment, paired with phase 03's
    /// runtime that keeps the session instance alive.
    pub fn parse_statement(&mut self, input: &str) -> ParseResult {
        // A `Data` that takes statements is parsed against again after it compiles, which is
        // what `open_world` names: whole-program rewrites (`@FR-R-ValueRecord`,
        // `@FR-R-InlineLeaf`) leave its signatures and calls as written.
        self.data.open_world = true;
        if Self::statement_incomplete(input) {
            return ParseResult::NeedMore;
        }
        let pre_defs = self.data.definitions();
        let pre_diag = self.diagnostics.entries().len();
        let is_def = Self::starts_top_level_def(input);
        // The wrapper's name is keyed on the pre-call def count, which is
        // monotonic across successful statements (each commit adds ≥1 def), so
        // it never collides; a rolled-back attempt truncates `definitions` back
        // to `pre_defs`, freeing the number for the next statement.
        let wrapper = format!("n_repl_{pre_defs}");
        if is_def {
            self.parse_str(input, "<repl>", false);
        } else {
            let src = format!("fn repl_{pre_defs}() {{\n{input}\n}}");
            self.parse_str(&src, "<repl>", false);
        }
        // Only diagnostics this statement produced — `Diagnostics::level` is
        // monotonic, so a prior error would otherwise mask a now-clean parse.
        let produced: Vec<DiagEntry> = self.diagnostics.entries()[pre_diag..].to_vec();
        if produced.iter().any(|e| e.level >= Level::Error) {
            self.data.rollback_to(pre_defs);
            return ParseResult::Error(produced);
        }
        // A definition isn't executed (nothing to run); a wrapped expression
        // returns its synthetic fn, resolved by name so a lambda appended inside
        // the body can't be mistaken for the entry point.
        let entry_def_nr = if is_def {
            u32::MAX
        } else {
            self.data.def_nr(&wrapper)
        };
        ParseResult::Ready { entry_def_nr }
    }

    /// True if `input` opens with a top-level definition keyword, so it can be
    /// parsed directly rather than wrapped in a synthetic REPL fn.
    pub(crate) fn starts_top_level_def(input: &str) -> bool {
        let word: String = input
            .trim_start()
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        matches!(
            word.as_str(),
            "struct"
                | "enum"
                | "fn"
                | "operator"
                | "type"
                | "pub"
                | "use"
                | "interface"
                | "typedef"
                | "const"
                | "capability"
        )
    }

    /// loft#1648 — everything a reader needs to attribute an H5 divergence, gathered where it
    /// fires.
    ///
    /// The assert names ONE definition and two counts, which says that something diverged and
    /// not why.  That gap is the whole cost of the flake it was written for: it reproduces
    /// about one suite start in eight, on a consumer's tree, so the run that fails is rarely
    /// the run anyone is watching, and by the time it is reported the only evidence is a
    /// sentence naming a struct nobody can place.  Two reports and ~53 starts produced no
    /// reproduction; what was missing each time was not persistence but PROVENANCE.
    ///
    /// So report the population and where it came from:
    ///
    ///   * the WHOLE pass-2-only range, each def with the facts the legality test reads.  The
    ///     first illegal def is not the informative one — in loft#1648 `#2708 Coord` is first
    ///     while `#2706` and `#2707` were legal lazy appends, and a group minted together
    ///     names its own origin.
    ///   * each def's SOURCE resolved to the FILE it was loaded from, and the whole id -> file
    ///     table.  A divergence is nearly always a file pass 2 read and pass 1 did not.
    ///   * any source whose mtime has MOVED since this parse began.  Both passes read every
    ///     file from disk independently, so a file rewritten under them is parsed from
    ///     different bytes and diverges for a reason that is not a compiler bug at all.  That
    ///     case is indistinguishable from a real one in today's message, and it is the one a
    ///     fresh checkout resolving its dependencies can actually produce.
    ///
    /// Costs nothing until it runs: the assert has already decided to panic.
    pub(super) fn h5_divergence_report(&self, pass1_defs: usize) -> String {
        use std::fmt::Write as _;
        // source id -> the file it was loaded from, inverted from `use_paths` (id -> path)
        // through `get_source`.  Built here rather than kept: this is the only reader, and a
        // map maintained for a panic would rot unnoticed.
        let mut files: std::collections::BTreeMap<u16, &str> = std::collections::BTreeMap::new();
        for (id, path) in &self.use_paths {
            let src = self.data.get_source(id);
            if src != u16::MAX {
                files.insert(src, path.as_str());
            }
        }
        let mut out =
            String::from("  pass-2-only definitions (the population, not just the first):\n");
        for d in pass1_defs..self.data.definitions.len() {
            let def = self.data.def(d as u32);
            let src = def.source;
            let _ = writeln!(
                out,
                "    #{d:<6} {:<34} {:?}  source={src}{}{}{}",
                def.name(),
                self.data.def_type(d as u32),
                files.get(&src).map_or_else(
                    // Not a `use`d library: the two sources every program has.  Left blank
                    // these read as "unknown", which sends the reader looking for a file
                    // that was never missing.
                    || match src {
                        0 => " (stdlib)".to_string(),
                        s if s == crate::data::MAIN_SOURCE =>
                            " (the program's own file)".to_string(),
                        _ => String::new(),
                    },
                    |f| format!(" ({f})")
                ),
                def.synthetic()
                    .map_or(String::new(), |s| format!(" synthetic={s}")),
                if def.instance_of == u32::MAX {
                    String::new()
                } else {
                    format!(" instance_of=#{}", def.instance_of)
                },
            );
        }
        let _ = writeln!(out, "  sources loaded, id -> file:");
        for (src, f) in &files {
            let _ = writeln!(out, "    {src:<4} {f}");
        }
        let moved = self.sources_changed_during_parse(files.values().copied());
        if files.is_empty() {
            // Say what was CHECKED, not what was found.  An empty table means no `use`d
            // library file was loaded, so nothing was re-stat'd — reporting that as "nothing
            // changed" would be the same overstatement this report exists to remove, and it
            // would point the reader at the compiler on no evidence.
            let _ = writeln!(
                out,
                "  no `use`d library file was loaded, so nothing could be re-stat'd — the \
                 divergence is in the stdlib or in the program's own file, and the source \
                 column above says which."
            );
        } else if moved.is_empty() {
            let _ = writeln!(
                out,
                "  none of the {} loaded source(s) above changed during this parse, so the \
                 two passes read the same bytes — this is a compiler-side divergence.",
                files.len()
            );
        } else {
            let _ = writeln!(
                out,
                "  ⚠ {} SOURCE(S) CHANGED DURING THIS PARSE — the two passes did not read the \
                 same bytes, so this is very likely not a compiler bug:",
                moved.len()
            );
            for f in &moved {
                let _ = writeln!(out, "    {f}");
            }
        }
        out
    }

    /// Which of `files` have been written since this parse began?
    ///
    /// Split out from the report so the question has one home and can be exercised without
    /// racing a parse: the report is only reachable by making two passes disagree, which is
    /// the very thing nobody can do on demand here.
    pub(super) fn sources_changed_during_parse<'a>(
        &self,
        files: impl Iterator<Item = &'a str>,
    ) -> Vec<String> {
        let Some(started) = self.pass_started else {
            return Vec::new();
        };
        files
            .filter(|f| {
                // An empty name names nothing (`file_access` would answer for `.`).
                !f.is_empty()
                    && crate::file_access::metadata(f)
                        .and_then(|m| m.modified())
                        // A clock that cannot answer is not evidence of a change: say no.
                        .is_ok_and(|m| m > started)
            })
            .map(str::to_string)
            .collect()
    }

    pub(super) fn source_loaded_from(&self, f: &str) -> Option<u16> {
        let canonical = crate::file_access::plain_canonical_str(f);
        // Every id that names this file, not just one of them: `use_paths` outlives a
        // single pass (it is the parser's, while `use_names` is reset between the two),
        // so it holds names from the previous pass that this pass has not re-bound yet.
        // Taking the FIRST match out of a `HashMap` and giving up when it happened to be
        // one of those made the answer depend on hash order — the two passes then loaded
        // different files and tripped the H5 cross-pass divergence guard.
        self.use_paths
            .iter()
            .filter(|(_, loaded)| **loaded == canonical)
            .map(|(id, _)| self.data.get_source(id))
            .find(|src| *src != u16::MAX)
    }
}
