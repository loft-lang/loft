// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Resolving a `use` target to a FILE (PACKAGES.md): the probes in their order — the
//! project's lib, the package's own name, declared path and registry dependencies, siblings,
//! the command-line and `LOFT_LIB` directories, the user's installed packages, the registry
//! cache — the lockfile and floors that govern a version, and the manifest's side effects
//! once a package is placed.

use super::{Level, Parser, Position, diagnostic_format, env, manifest, other_sep, sep, sep_str};

/// Does the file `path` names exist?  An empty name names nothing: `file_access` reads it
/// as `.`, which every resolution probe here would take for a hit.
pub(super) fn file_exists(path: &str) -> bool {
    !path.is_empty() && crate::file_access::exists(path)
}

/// The "you probably meant `pkg::name`" message for an unresolved bare call, or
/// `None` when no published package exports such a free function (@PLN13 phase 6,
/// diagnostics slice).
///
/// Bare calls into a library still do not RESOLVE — that is the rest of phase 6,
/// and it has to settle stdlib shadowing first (a bare `find(…)` already binds to
/// the stdlib `find`, silently). What this removes is the dead end: the name was
/// right, it just was not imported, and the message now says where it lives.
/// loft#789 — the advice has to be about the packages the BUILD resolved, not
/// the ones the registry index knows.
///
/// `resolved` names the packages this compilation already has, so a suggestion
/// to import one of them can be recognised as the misdirection it is: the file
/// says `use hex_world;` on its second line, the resolved `hex_world` came from
/// `--lib` and has no such function, and the registry's `hex_world` is a
/// different package that does. Telling the author to add an import they have
/// already written sends them to check the one thing that is right.
#[cfg(feature = "registry")]
pub(super) fn registry_fn_hint(name: &str, resolved: &[String]) -> Option<String> {
    let pkgs = crate::registry_index::packages_exporting_fn(name);
    let first = pkgs.first()?;
    // The useful diagnosis when the name collides: two packages share it, and
    // the one in this build is not the one the index describes.
    if let Some(here) = pkgs.iter().find(|p| resolved.iter().any(|r| &r == p)) {
        return Some(format!(
            "Unknown function {name} — the `{here}` this build resolved does not have it, \
             and the registry's `{here}` does. These are different packages of the same name"
        ));
    }
    let provider = if pkgs.len() == 1 {
        format!("the `{first}` package provides it")
    } else {
        let names: Vec<String> = pkgs.iter().map(|p| format!("`{p}`")).collect();
        format!("the {} packages provide it", names.join(" / "))
    };
    Some(format!(
        "Unknown function {name} — {provider}; call `{first}::{name}(…)`, or add `use {first}::({name});` and call it bare"
    ))
}

/// Registry-less build: no index to consult, so no hint (@PLN13 phase 6).
#[cfg(not(feature = "registry"))]
pub(super) fn registry_fn_hint(_name: &str, _resolved: &[String]) -> Option<String> {
    None
}

/// Directory portion of a normalised script path, or `""` if the path is bare.
pub(super) fn script_dir(cur_script: &str) -> &str {
    cur_script.rfind(sep()).map_or("", |p| &cur_script[0..p])
}

/// If `cur_dir` lives inside a `/tests/` tree, return the ancestor above
/// `tests/`; otherwise `""`.  Used to locate the project-root `lib/`
/// directory when running a script from `tests/`.
pub(super) fn tests_base_dir(cur_dir: &str) -> &str {
    let tests_infix = format!("{0}tests{0}", sep());
    if let Some(idx) = cur_dir.find(tests_infix.as_str()) {
        &cur_dir[..idx]
    } else {
        ""
    }
}

/// Pure-resolution result from [`Parser::lib_path_manifest_resolve`].
/// Callers decide when to apply side effects (native-lib registration,
/// dependency queueing, etc.).  The legacy `lib_path_manifest` adapter
/// applies them immediately; Phase A of the package-mode driver
/// consults the manifest to build its package graph, then defers
/// side-effect application until after pass-1 parsing.
pub(super) struct ResolvedPkg {
    pkg_dir: String,
    pub(super) entry: String,
    /// `None` when the package directory exists but has no `loft.toml`
    /// (pure multi-file package without a manifest).
    manifest: Option<manifest::Manifest>,
}

impl Parser {
    /// The entry file of a dependency declared by PATH — `dep = { path = "…" }` relative to
    /// the manifest's directory `d`, or the project's `lib/<dep>` — read from the package's
    /// own manifest and touching nothing else; `None` for a registry dependency.
    pub(super) fn dep_entry_by_path(d: &std::path::Path, dep: &str, value: &str) -> Option<String> {
        let root = match crate::manifest::extract_path_dep(value) {
            Some(path) => d.join(path),
            None => d.join("lib").join(dep),
        };
        let toml = root.join("loft.toml");
        if !crate::file_access::exists(&toml) {
            return None;
        }
        let man = crate::manifest::read_manifest(&toml.to_string_lossy())?;
        let entry = man.entry.unwrap_or_else(|| format!("src/{dep}.loft"));
        let entry = root.join(entry);
        crate::file_access::exists(&entry).then(|| entry.to_string_lossy().into_owned())
    }

    /// The extracted copy of a REGISTRY dependency `dep` declared as `value` by the manifest
    /// in `d`: the version `d/loft.lock` pins when that copy is in the cache, else the newest
    /// cached copy this loft can load that satisfies the declared range.  A pure read of the
    /// lock and the cache directory — no index, no network, nothing queued; `None` for a
    /// `path =` dependency (those resolve by path) and when nothing is extracted yet (the
    /// install that follows loads the package the ordinary way, and its triggers fire from
    /// the next parse).
    #[cfg(feature = "registry")]
    pub(super) fn registry_dep_root(
        d: &std::path::Path,
        dep: &str,
        value: &str,
    ) -> Option<std::path::PathBuf> {
        if crate::manifest::extract_path_dep(value).is_some() {
            return None;
        }
        if let Ok(Some(lock)) = crate::lockfile::read_lockfile(&d.join("loft.lock"))
            && let Some(pin) = lock.packages.iter().find(|p| p.name == dep)
        {
            let root = crate::registry_index::cache_dir().join(format!("{dep}-{}", pin.version));
            if crate::file_access::exists(root.join("loft.toml")) {
                return Some(root);
            }
        }
        let constraints: Vec<String> = crate::manifest::extract_version_req(value)
            .map(|v| vec![v.to_string()])
            .unwrap_or_default();
        crate::registry_index::newest_cached_loadable_satisfying(dep, &constraints)
            .map(|(_, dir)| dir)
    }

    #[cfg(not(feature = "registry"))]
    pub(super) fn registry_dep_root(
        _d: &std::path::Path,
        _dep: &str,
        _value: &str,
    ) -> Option<std::path::PathBuf> {
        None
    }

    /// Report the manifests read since the last call that the lexer could not read
    /// ([`crate::manifest::take_manifest_errors`]), each once per parse, at the manifest's own
    /// line.  Kept, such a string is a value with `?` where the author's characters were — a
    /// Windows path's `\a` — and the run then fails on what it should have declared.
    pub(super) fn report_manifest_errors(&mut self) {
        for e in crate::manifest::take_manifest_errors() {
            if self.reported_manifest_errors.contains(&e) {
                continue;
            }
            let at = Position {
                file: crate::lexer::FileName::from(e.file.as_str()),
                line: e.line,
                pos: e.col,
            };
            self.lexer.pos_diagnostic(Level::Error, &at, &e.message);
            self.reported_manifest_errors.push(e);
        }
    }

    pub(super) fn lib_path(&mut self, id: &str) -> String {
        // Under the `wasm` feature, VIRT_FS wins over filesystem lookups.
        #[cfg(feature = "wasm")]
        if crate::wasm::virt_fs_get(&format!("{id}.loft")).is_some() {
            return format!("{id}.loft");
        }

        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        let cur_dir_owned = script_dir(&cur_script).to_string();
        let base_dir_owned = tests_base_dir(&cur_dir_owned).to_string();
        let cur_dir = cur_dir_owned.as_str();
        let base_dir = base_dir_owned.as_str();

        // Try each strategy in order; first one that finds the file wins.
        // `f` starts as the cheapest guess (project-local `lib/<id>.loft` or
        // `<id>.loft`); subsequent strategies overwrite only when `f` does
        // not yet resolve to an existing file.
        // Dep-shadowing guard: a name declared under [dependencies] in the
        // current package's manifest must resolve as a dependency — never to
        // a same-named `.loft` file inside the declaring package.  Without
        // this, `use server` in a package that also contains `server.loft`
        // loads the package file (the package root sits in `lib_dirs` for
        // intra-package `use`), silently shadowing the library: its types
        // vanish and every consumer in the package breaks.
        let shadow_root = self
            .package_declared_deps(cur_dir)
            .filter(|(_, deps)| deps.contains(id))
            .map(|(root, _)| root);
        // loft#963 — except where the declaration points INTO the package.  A
        // `{ path = "lib/mylib" }` dep is in-tree by definition, so the guard above
        // matched the very directory the declaration named: `probe_manifest_path_dep`
        // resolved it, the next sweep wiped it, `--lib lib/` resolved it again, and the
        // sweep after that wiped it too.  The declaration was therefore strictly worse
        // than saying nothing — without it `--lib` worked — and the error still read
        // "searched lib/, lib_dirs, and sibling packages" while lib_dirs held the answer.
        //
        // The rule the guard is for is a stray same-named MODULE FILE (`use server` in a
        // package that also has `server.loft`); a nested package the manifest points at
        // is the opposite case, and it is named by exactly the path being exempted here.
        let dep_root = Self::declared_path_dep_root(id, cur_dir);
        let blocked = |candidate: &str| {
            let cand = std::path::Path::new(candidate);
            let cand = crate::file_access::plain_canonical(cand);
            if dep_root.as_ref().is_some_and(|r| cand.starts_with(r)) {
                return false;
            }
            shadow_root
                .as_deref()
                // Canonicalize so relative candidates (cwd inside the
                // package) and the canonical root compare in one space.
                .is_some_and(|root| cand.starts_with(root))
        };

        let mut f = Self::probe_project_lib(id);
        self.probe_own_package(id, cur_dir, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        Self::probe_dir_lib(id, cur_dir, &mut f);
        Self::probe_dir_lib(id, base_dir, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        self.probe_manifest_path_dep(id, cur_dir, &mut f);
        self.probe_root_path_dep(id, &cur_script, &mut f);
        self.probe_sibling_package(id, cur_dir, &mut f);
        Self::probe_script_sibling_dir(id, &cur_script, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        self.probe_cmdline_lib_dirs(id, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        self.probe_cmdline_lib_dirs_manifest(id, &mut f);
        Self::probe_loft_lib_flat(id, &mut f);
        self.probe_loft_lib_manifest(id, &mut f);
        self.probe_user_installed(id, &mut f);
        // A flat `<id>.loft` in the script's own directory is a file the author put beside
        // the program (PACKAGES.md § Resolution order, step 1), so it is probed before the
        // registry: probed after it, a box that happened to cache a same-named package
        // bound that package instead, and a fixture library beside a test asked the
        // network about its own name on every run.
        Self::probe_cur_dir_flat(id, cur_dir, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        // loft#968 — everything above resolves from something the project SAYS: a path it
        // declares, a `--lib` directory it was given, a sidecar lock it pinned.  What
        // follows resolves from the registry, which is a property of the BOX.  So this is
        // the one line where the two can be told apart.
        let mut named_by_the_project = file_exists(&f);
        // @PLN143 arc B — the scope is a property of the PROGRAM, so it is answered ONCE,
        // here, and handed to each probe that needs it.  Re-deriving it inside a probe
        // would put the old three-sites-must-agree brittleness back with one extra step
        // between it and the reader.
        if self.program_scope.is_none() {
            let entry = self.program_entry.map(crate::lexer::FileName::as_str);
            self.program_scope = Some(crate::resolution_scope::resolution_scope(
                entry.unwrap_or(&cur_script),
            ));
        }
        let scope = self
            .program_scope
            .clone()
            .unwrap_or(crate::resolution_scope::ResolutionScope::Bare);
        // A sidecar pin belongs on the project's side of that line: `loft pin <script>`
        // wrote it FOR this script, so it is a declaration the author made — even though
        // what it resolves to is a package in the registry cache.
        if self.probe_governing_lock(id, &mut f, &cur_script, &scope)
            && matches!(
                scope,
                crate::resolution_scope::ResolutionScope::PinnedScript(_)
            )
        {
            named_by_the_project = true;
        }
        self.probe_auto_install(id, &mut f, &cur_script, &scope);
        self.probe_cache_newest(id, &mut f, &cur_script, &scope);
        if !named_by_the_project && file_exists(&f) {
            self.undeclared_registry_dep(id, &cur_script);
        }
        Self::probe_base_dir_flat(id, base_dir, &mut f);
        if blocked(&f) {
            f = format!("{id}.loft");
        }
        self.lib_flag_outranked(id, &f);
        f
    }

    /// loft#1352 — a `--lib` directory provides `<id>`, and the `use` resolved elsewhere:
    /// say so.
    ///
    /// Resolution is first-wins, and a project-local `lib/`, a declared dependency and the
    /// script's own directory are all probed before the flag, so the flag never reaches a
    /// name one of those also provides.  That precedence may well be right — an
    /// intra-package `use` must not be hijackable by a stray flag — and this reports it
    /// rather than moving it.  The cost of the silence was a MEASUREMENT: three runs of a
    /// patched library copy passed through `--lib <copy>` from the repository root, and
    /// every one scored the unmodified tree, because `lib/` beside the working directory
    /// answered first and nothing said the flag had lost.
    ///
    /// `advice`, not `warning`: the program computes what the language promises for the
    /// file it resolved.  What is wrong is the operator's belief about WHICH file, and the
    /// cure is at the command line — run from a directory without a `lib/`, or put the
    /// override where the resolution looks first.  Both layouts a `--lib` directory can
    /// carry are checked, the flat `<dir>/<id>.loft` and the packaged
    /// `<dir>/<id>/src/<id>.loft`, and a flag directory the winner lies INSIDE is not a
    /// loss.  One report per id; `LOFT_NO_LIB_OUTRANKED` silences it.
    pub(super) fn lib_flag_outranked(&mut self, id: &str, resolved: &str) {
        if self.lib_dirs.is_empty() || std::env::var_os("LOFT_NO_LIB_OUTRANKED").is_some() {
            return;
        }
        if !file_exists(resolved) {
            return;
        }
        let canon = |p: &str| crate::file_access::plain_canonical(std::path::Path::new(p));
        let winner = canon(resolved);
        let lib_dirs = self.lib_dirs.clone();
        let Some((dir, provided)) = lib_dirs.iter().find_map(|l| {
            let flat = format!("{l}{}{id}.loft", sep_str());
            let packaged = format!("{l}{0}{id}{0}src{0}{id}.loft", sep_str());
            let provided = [flat, packaged]
                .into_iter()
                .find(|c| crate::file_access::exists(c))?;
            if winner.starts_with(canon(l)) {
                return None;
            }
            Some((l.clone(), provided))
        }) else {
            return;
        };
        if !self.lib_outranked_reported.insert(id.to_string()) {
            return;
        }
        diagnostic!(
            self.lexer,
            Level::Advice,
            code = "lib-flag-outranked",
            "`use {id}` resolved to `{resolved}`; `--lib {dir}` also provides it (`{provided}`) \
             and was not consulted — a project-local `lib/`, a declared dependency and the \
             script's own directory are searched before the flag"
        );
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Mechanical,
            title: "run from a directory without a `lib/` that provides this name, or put the \
                    override where the resolution looks first"
                .to_string(),
            condition: None,
            edit: None,
            concept: "packages",
            concept_ref: "@F55",
        });
    }

    /// loft#968 — `use <pkg>` resolved a registry package the project's manifest never
    /// declares.
    ///
    /// The resolution is deliberate and stays: a qualified `lib::fn()` auto-loads, and
    /// self-healing `loft.lock` is a convenience worth keeping.  The silence is the
    /// defect. Nothing in the project distinguished *"we depend on this"* from *"this
    /// happens to be installed on the box that built it"*, so a consumer could delete a
    /// `[dependencies]` line and every test still passed — measured on a 33 000-line
    /// project with ten declared dependencies. The negative gate they wanted (*drop the
    /// dependency and the tests must stop compiling*) could not be written, and a
    /// dependency deleted by accident was invisible to every gate.
    ///
    /// `advice`, not `warning`: the program computes exactly what the language promises,
    /// on this box, today.  What is wrong is that the manifest does not describe the
    /// project — a build-reproducibility fact, not a wrong answer.
    ///
    /// Silent in the two places where there is nothing to declare **into**: a bare script
    /// with no `loft.toml` above it (that is the `pip install` story working), and a
    /// package being parsed out of the registry cache — that manifest belongs to someone
    /// else, and its author is the only one who can act on it.
    #[cfg(feature = "registry")]
    pub(super) fn undeclared_registry_dep(&mut self, id: &str, cur_script: &str) {
        if std::env::var_os("LOFT_NO_UNDECLARED_DEP").is_some() {
            return;
        }
        if Self::script_in_registry_cache(cur_script) {
            return;
        }
        let Some(root) = crate::resolution_scope::project_root(cur_script) else {
            return;
        };
        let manifest_path = root.join("loft.toml");
        let Some(manifest) = crate::manifest::read_manifest(&manifest_path.to_string_lossy())
        else {
            return;
        };
        if manifest.dependencies.iter().any(|(name, _)| name == id) {
            return;
        }
        if !self.undeclared_reported.insert(id.to_string()) {
            return;
        }
        diagnostic!(
            self.lexer,
            Level::Advice,
            code = "undeclared-dependency",
            "`{id}` resolved from the registry, but `{}` does not declare it — so nothing \
             here says whether the project depends on `{id}` or merely runs on a box that \
             has it installed",
            manifest_path.display()
        );
        self.lexer.fix_last(crate::diagnostics::Fix {
            kind: crate::diagnostics::FixKind::Mechanical,
            title: format!("run `loft install {id}` to record it under `[dependencies]`"),
            condition: None,
            edit: None,
            concept: "packages",
            concept_ref: "@F55",
        });
    }

    /// No registry, no registry-resolved package to report (loft#968).
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    pub(super) fn undeclared_registry_dep(&mut self, _id: &str, _cur_script: &str) {}

    /// Do these two files belong to the same package — the same nearest `loft.toml`?
    ///
    /// Answered by walking up from each, which is the same rule `package_declared_deps` uses
    /// to decide what package a directory is in. A path with no manifest above it (a bare
    /// script) is in no package and so shares one with nothing.
    pub(super) fn same_package(a: &str, b: &str) -> bool {
        let root = |p: &str| -> Option<std::path::PathBuf> {
            let mut dir = crate::file_access::try_plain_canonical(std::path::Path::new(p))?;
            if crate::file_access::is_file(&dir) {
                dir = dir.parent()?.to_path_buf();
            }
            loop {
                if crate::file_access::exists(dir.join("loft.toml")) {
                    return Some(dir);
                }
                dir = dir.parent()?.to_path_buf();
            }
        };
        match (root(a), root(b)) {
            (Some(x), Some(y)) => x == y,
            _ => false,
        }
    }

    /// The package context owning `cur_dir`: the nearest ancestor directory
    /// holding a `loft.toml`, plus that manifest's declared dependency names.
    /// Cached per directory (manifests don't change mid-parse).
    pub(super) fn package_declared_deps(
        &mut self,
        cur_dir: &str,
    ) -> Option<(String, std::collections::HashSet<String>)> {
        if let Some(cached) = self.pkg_dep_cache.get(cur_dir) {
            return cached.clone();
        }
        let mut found = None;
        let start = if cur_dir.is_empty() { "." } else { cur_dir };
        let mut search = crate::file_access::try_plain_canonical(std::path::Path::new(start));
        while let Some(dir) = search {
            let manifest_path = dir.join("loft.toml");
            if crate::file_access::exists(&manifest_path) {
                if let Some(manifest) =
                    crate::manifest::read_manifest(&manifest_path.to_string_lossy())
                {
                    let deps: std::collections::HashSet<String> = manifest
                        .dependencies
                        .iter()
                        .map(|(name, _)| name.clone())
                        .collect();
                    found = Some((dir.to_string_lossy().to_string(), deps));
                }
                break;
            }
            search = dir.parent().map(std::path::Path::to_path_buf);
        }
        self.pkg_dep_cache
            .insert(cur_dir.to_string(), found.clone());
        found
    }

    /// The root project's declared version constraint for dependency `id`, if
    /// any — read once (cached) from the main script's nearest-ancestor
    /// `loft.toml` via `source_dir`.  Threaded into source-level auto-installs
    /// so a root pin overrides the default "resolve newest", including for a
    /// package pulled transitively by a lib that didn't pin it itself.  This is
    /// what makes `glb = "=0.1.0"` (exact) — or any range — an honoured option.
    ///
    /// The REQUIREMENT, never the raw value: an inline table (`{ version = "=0.1.0" }`) is
    /// read through [`crate::manifest::extract_version_req`] here, once, so no caller
    /// compares a version against the table's text.
    #[cfg(feature = "registry")]
    pub(super) fn root_dep_constraint(&mut self, id: &str) -> Option<String> {
        if self.root_dep_pins.is_none() {
            let mut map = std::collections::HashMap::new();
            if let Some(root) = crate::resolution_scope::project_root(&self.database.source_dir) {
                let manifest_path = root.join("loft.toml");
                if let Some(manifest) =
                    crate::manifest::read_manifest(&manifest_path.to_string_lossy())
                {
                    for (name, value) in manifest.dependencies {
                        if let Some(req) = crate::manifest::extract_version_req(&value) {
                            map.insert(name, req.to_string());
                        }
                    }
                }
            }
            self.root_dep_pins = Some(map);
        }
        self.root_dep_pins.as_ref().and_then(|m| m.get(id).cloned())
    }

    /// Initial guess: the project-supplied `lib/<id>.loft`, falling back to
    /// `<id>.loft` in the current working directory.
    pub(super) fn probe_project_lib(id: &str) -> String {
        let f = format!("lib{0}{id}.loft", sep_str());
        if file_exists(&f) {
            f
        } else {
            format!("{id}.loft")
        }
    }

    /// loft#2010 — the package this file is in answers for its own NAME, first: `use <id>`
    /// where `<id>` is its `[package] name` binds its own entry.
    ///
    /// A package is identified by its manifest's NAME, never by the directory it sits in.
    /// Before this probe the only route from a package's `tests/` to its own entry was the
    /// sibling probe, `<parent>/<id>/src/<id>.loft`, which finds the package itself only
    /// when its directory happens to be named `<id>`.  In a checkout called anything else
    /// the name fell through to whatever else answered to it — the installed registry
    /// copy, or a neighbouring directory called `<id>` — and the suite ran against that
    /// instead, green when the API had not changed.  Which CLI spelling ran the tests
    /// decided it too, since only some of them put the source directory on `--lib`.
    ///
    /// Only the package's own NAME, deliberately not every module of its `src/`: binding
    /// a package's other modules local-first is the precedence change loft#949 measured
    /// and declined (it rebinds the `graphics` ⟷ `mesh3d` overlap — see
    /// [`Self::module_name_clash`]).  No other package ships a module under this
    /// package's own name, so this branch moves nothing that overlap depends on.
    pub(super) fn probe_own_package(&mut self, id: &str, cur_dir: &str, f: &mut String) {
        let Some((root, _)) = self.package_declared_deps(cur_dir) else {
            return;
        };
        let root = std::path::PathBuf::from(root);
        let manifest_path = root.join("loft.toml");
        let Some(m) = manifest::read_manifest(&manifest_path.to_string_lossy()) else {
            return;
        };
        if m.name.as_deref() != Some(id) {
            return;
        }
        let Some(candidate) = m
            .entry_or_default()
            .map(|e| root.join(e))
            .filter(|c| crate::file_access::is_file(c))
        else {
            return;
        };
        *f = candidate.to_string_lossy().into_owned();
        self.register_native_manifest(&manifest_path, &root);
    }

    /// `<dir>/lib/<id>.loft` — a `lib/` directory relative to `dir`
    /// (called for the script's own dir, then for the base dir when the
    /// script lives inside a `/tests/` tree).
    pub(super) fn probe_dir_lib(id: &str, dir: &str, f: &mut String) {
        if !dir.is_empty() && !file_exists(f) {
            *f = format!("{dir}{0}lib{0}{id}.loft", sep_str());
        }
    }

    /// #337 / PACKAGES.md resolution step 2 — `[dependencies] <id> =
    /// { path = "…" }` in the consuming package's `loft.toml`.  Walk up
    /// from `cur_dir` to the owning manifest; if it declares `id` as a
    /// path dependency, resolve the path relative to the manifest's
    /// directory and locate the dep package's entry file (its own
    /// `[library] entry`, defaulting to `src/<id>.loft`).  Registers the
    /// dep's manifest so its `#native` symbols resolve, mirroring
    /// `probe_sibling_package`.
    /// Where the nearest enclosing manifest says dependency `id` lives, when it declares
    /// it as a `{ path = … }` dep — canonicalized, or `None` for a registry dep or no
    /// declaration at all.
    ///
    /// Read by the dep-shadowing guard in [`Self::lib_path`], which otherwise treats an
    /// in-tree path dep as a file shadowing the dependency it IS (loft#963).  Walks to
    /// the same manifest [`Self::probe_manifest_path_dep`] resolves against, so the
    /// exemption and the resolution cannot name different directories.
    pub(super) fn declared_path_dep_root(id: &str, cur_dir: &str) -> Option<std::path::PathBuf> {
        if cur_dir.is_empty() {
            return None;
        }
        let mut search_dir = std::path::Path::new(cur_dir).to_path_buf();
        loop {
            let manifest_path = search_dir.join("loft.toml");
            if crate::file_access::exists(&manifest_path) {
                let rel = crate::manifest::read_manifest(&manifest_path.to_string_lossy())
                    .and_then(|m| {
                        m.dependencies.iter().find_map(|(name, value)| {
                            (name == id)
                                .then(|| crate::manifest::extract_path_dep(value))
                                .flatten()
                                .map(str::to_string)
                        })
                    })?;
                let root = search_dir.join(rel);
                return Some(crate::file_access::plain_canonical(&root));
            }
            search_dir = search_dir.parent()?.to_path_buf();
        }
    }

    pub(super) fn probe_manifest_path_dep(&mut self, id: &str, cur_dir: &str, f: &mut String) {
        if file_exists(f) || cur_dir.is_empty() {
            return;
        }
        let mut search_dir = std::path::Path::new(cur_dir).to_path_buf();
        loop {
            let manifest_path = search_dir.join("loft.toml");
            if crate::file_access::exists(&manifest_path) {
                let dep_rel = crate::manifest::read_manifest(&manifest_path.to_string_lossy())
                    .and_then(|m| {
                        m.dependencies.iter().find_map(|(name, value)| {
                            (name == id)
                                .then(|| crate::manifest::extract_path_dep(value))
                                .flatten()
                                .map(str::to_string)
                        })
                    });
                if let Some(rel) = dep_rel {
                    let pkg_root = search_dir.join(rel);
                    let dep_manifest = pkg_root.join("loft.toml");
                    let entry = crate::file_access::exists(&dep_manifest)
                        .then(|| crate::manifest::read_manifest(&dep_manifest.to_string_lossy()))
                        .flatten()
                        .and_then(|m| m.entry)
                        .unwrap_or_else(|| format!("src{}{id}.loft", sep_str()));
                    let file = pkg_root.join(entry);
                    if crate::file_access::exists(&file) {
                        *f = file.to_string_lossy().to_string();
                        if crate::file_access::exists(&dep_manifest) {
                            self.register_native_manifest(&dep_manifest, &pkg_root);
                        }
                    }
                }
                break;
            }
            let Some(p) = search_dir.parent() else {
                break;
            };
            search_dir = p.to_path_buf();
        }
    }

    /// loft#1849 — the ROOT project's `{ path = … }` declaration of `id` answers a `use id`
    /// anywhere in the program, not only in the root's own files.
    ///
    /// PACKAGES.md: "The root project's declared constraints pin the whole tree, including
    /// packages pulled in transitively by a `use` inside a dependency", and a package loads
    /// once.  Every other project-side probe looks from the IMPORTING file's directory, so a
    /// `use graphics` inside a path dependency `input` never saw the root's declaration: it
    /// fell to the registry, and that copy — being loaded first — became the program's,
    /// whatever the root said.  Which copy a program compiled against depended on the order
    /// of its `use` lines.
    ///
    /// The importing package's own range must still hold for the copy the root names; when
    /// it does not, the two declarations disagree and the program is refused, naming both
    /// ([`Self::loaded_copy_meets_declaring_range`]).
    pub(super) fn probe_root_path_dep(&mut self, id: &str, cur_script: &str, f: &mut String) {
        if file_exists(f) {
            return;
        }
        let Some(root) = crate::resolution_scope::project_root(&self.database.source_dir) else {
            return;
        };
        let root_dir = root.to_string_lossy().to_string();
        self.probe_manifest_path_dep(id, &root_dir, f);
        if file_exists(f)
            && let Some(pkg_root) = Self::declared_path_dep_root(id, &root_dir)
        {
            self.loaded_copy_meets_declaring_range(id, cur_script, &pkg_root);
        }
    }

    /// The version of the copy of `id` this program loads (the package at `pkg_root`) against
    /// the range the package whose file says `use id` names for it — refused when it falls
    /// outside, because a copy the importer excludes is the silent mismatch loft#1849 is.
    /// Asked where the copy is chosen ([`Self::probe_root_path_dep`]) AND where a later
    /// `use` finds it already loaded, so the verdict cannot depend on which `use` came first.
    #[cfg(feature = "registry")]
    pub(super) fn loaded_copy_meets_declaring_range(
        &mut self,
        id: &str,
        cur_script: &str,
        pkg_root: &std::path::Path,
    ) {
        let Some(range) = Self::declaring_range(cur_script, id) else {
            return;
        };
        let Some(version) =
            crate::manifest::read_manifest(&pkg_root.join("loft.toml").to_string_lossy())
                .and_then(|m| m.version)
        else {
            return;
        };
        if crate::registry_index::satisfies(&version, &range) {
            return;
        }
        let importer = crate::resolution_scope::project_root(cur_script)
            .and_then(|r| crate::manifest::read_manifest(&r.join("loft.toml").to_string_lossy()))
            .and_then(|m| m.name)
            .unwrap_or_else(|| cur_script.to_string());
        diagnostic!(
            self.lexer,
            Level::Error,
            "`{importer}` needs `{id} {range}`, and the copy of `{id}` this program loads ({}) \
             is {version} — the two declarations disagree, so neither copy can stand in for \
             the other",
            pkg_root.display()
        );
    }

    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    pub(super) fn loaded_copy_meets_declaring_range(
        &mut self,
        _id: &str,
        _cur_script: &str,
        _pkg_root: &std::path::Path,
    ) {
    }

    /// Walk up from `cur_dir` looking for a `loft.toml`; on hit, the package's
    /// parent directory may contain sibling packages.  When the sibling is
    /// found directly (not via `lib_path_manifest`), the sibling's own
    /// `loft.toml` must be registered so its `#native` symbols resolve.
    pub(super) fn probe_sibling_package(&mut self, id: &str, cur_dir: &str, f: &mut String) {
        if file_exists(f) || cur_dir.is_empty() {
            return;
        }
        let mut search_dir = std::path::Path::new(cur_dir).to_path_buf();
        loop {
            if crate::file_access::exists(search_dir.join("loft.toml")) {
                if let Some(parent) = search_dir.parent()
                    && let Some(path) = Self::find_sibling_file(parent, id)
                {
                    *f = path.to_string_lossy().to_string();
                    let pkg_root = parent.join(id);
                    let manifest = pkg_root.join("loft.toml");
                    if crate::file_access::exists(&manifest) {
                        self.register_native_manifest(&manifest, &pkg_root);
                    }
                }
                break;
            }
            let Some(p) = search_dir.parent() else {
                break;
            };
            search_dir = p.to_path_buf();
        }
    }

    /// Sibling lookup: prefer `<parent>/<id>/src/<id>.loft`, fall back to
    /// flat `<parent>/<id>.loft`.
    pub(super) fn find_sibling_file(
        parent: &std::path::Path,
        id: &str,
    ) -> Option<std::path::PathBuf> {
        let nested = parent.join(id).join("src").join(format!("{id}.loft"));
        if crate::file_access::exists(&nested) {
            return Some(nested);
        }
        let flat = parent.join(format!("{id}.loft"));
        crate::file_access::exists(&flat).then_some(flat)
    }

    /// A directory named after the current script (minus the `.loft` suffix).
    pub(super) fn probe_script_sibling_dir(id: &str, cur_script: &str, f: &mut String) {
        if !file_exists(f) && cur_script.len() >= 5 {
            *f = format!(
                "{}{}{id}.loft",
                &cur_script[0..cur_script.len() - 5],
                sep_str()
            );
        }
    }

    /// `--lib` / `--project` command-line flag directories, flat layout.
    /// Registers any discovered `loft.toml` in the file's ancestry.
    pub(super) fn probe_cmdline_lib_dirs(&mut self, id: &str, f: &mut String) {
        if file_exists(f) {
            return;
        }
        let lib_dirs = self.lib_dirs.clone();
        for l in &lib_dirs {
            let candidate = format!("{l}{}{id}.loft", sep_str());
            if crate::file_access::exists(&candidate) {
                f.clone_from(&candidate);
                self.register_manifest_in_ancestors(&candidate);
                break;
            }
        }
    }

    /// Scan ancestor directories for a `loft.toml` and register the first one
    /// found.  Called when a file was resolved directly (not through
    /// `lib_path_manifest`), so its owning package's native crate would
    /// otherwise never be registered.
    pub(super) fn register_manifest_in_ancestors(&mut self, candidate: &str) {
        let mut search = std::path::Path::new(candidate)
            .parent()
            .map(std::path::Path::to_path_buf);
        while let Some(dir) = search {
            let manifest = dir.join("loft.toml");
            if crate::file_access::exists(&manifest) {
                self.register_native_manifest(&manifest, &dir);
                return;
            }
            search = dir.parent().map(std::path::Path::to_path_buf);
        }
    }

    /// `--lib` / `--project` directories, packaged layout
    /// (`<dir>/<id>/src/<id>.loft`).
    pub(super) fn probe_cmdline_lib_dirs_manifest(&mut self, id: &str, f: &mut String) {
        if file_exists(f) {
            return;
        }
        let lib_dirs = self.lib_dirs.clone();
        for l in &lib_dirs {
            if let Some(entry) = self.lib_path_manifest(l, id) {
                *f = entry;
                return;
            }
        }
    }

    /// `LOFT_LIB` env var, flat layout (`<dir>/<id>.loft`).
    pub(super) fn probe_loft_lib_flat(id: &str, f: &mut String) {
        if file_exists(f) {
            return;
        }
        let Some(v) = env::var_os("LOFT_LIB") else {
            return;
        };
        for l in env::split_paths(&v) {
            let candidate = l.join(format!("{id}.loft"));
            if crate::file_access::exists(&candidate) {
                *f = candidate.to_string_lossy().replace(other_sep(), sep_str());
                return;
            }
        }
    }

    /// `LOFT_LIB` env var, packaged layout (via `lib_path_manifest`).
    pub(super) fn probe_loft_lib_manifest(&mut self, id: &str, f: &mut String) {
        if file_exists(f) {
            return;
        }
        let Some(v) = env::var_os("LOFT_LIB") else {
            return;
        };
        for l in env::split_paths(&v) {
            let l = l.to_string_lossy().replace(other_sep(), sep_str());
            if let Some(entry) = self.lib_path_manifest(&l, id) {
                *f = entry;
                return;
            }
        }
    }

    /// `~/.loft/lib/<id>/src/<id>.loft` — packages installed via `loft install`.
    pub(super) fn probe_user_installed(&mut self, id: &str, f: &mut String) {
        if file_exists(f) {
            return;
        }
        let home = env::var("HOME")
            .or_else(|_| env::var("USERPROFILE"))
            .unwrap_or_default();
        if home.is_empty() {
            return;
        }
        let user_lib = format!("{home}/.loft/lib");
        if let Some(entry) = self.lib_path_manifest(&user_lib, id) {
            *f = entry;
        }
    }

    /// @PLAN12 Phase 6.7 — per-invocation advisory classifier.
    ///
    /// Called by each lockfile-based probe (sidecar / project /
    /// registry-installed) after resolving a `(name, version)`
    /// tuple.  Loads the cached advisory feed (lazily, once per
    /// process), classifies the tuple, and emits severity-tiered
    /// output:
    ///
    /// - `security_critical` → loud `error:` block; **continue
    ///   running** (the user may be running their code precisely to
    ///   test the upgrade, and security fixes can introduce
    ///   breaking changes; refusing here is worse than warning).
    ///   Opt-in refusal: `LOFT_STRICT_SECURITY=1` (env var) OR
    ///   `--strict-security` (CLI flag, intended for CI gates) — in
    ///   either, critical aborts with `process::exit(3)` UNLESS
    ///   `LOFT_SECURITY_OVERRIDE=<advisory-id>` is set (comma-
    ///   separated for multiple).
    /// - `security_high` → loud `warning:` block; continue.
    /// - `security_low` / `bug` → one-line warning.
    /// - `deprecated` → one-line note.
    ///
    /// The Cargo / npm precedent: `cargo audit` defaults to warn,
    /// `--deny warnings` is opt-in for CI.  Pure refusal blocks the
    /// user precisely when they're trying to ship a fix or assess
    /// the upgrade — the wrong default.
    ///
    /// Dedupes via `self.advisory_checked` so a package surfaced by
    /// multiple `use` paths (sidecar + auto-install re-probe) only
    /// reports once.  Feed loading is best-effort offline + cache-
    /// only: no network fetch fires during script execution, only
    /// during explicit `loft audit` / install commands.
    #[cfg(feature = "registry")]
    pub(super) fn check_advisory(&mut self, name: &str, version: &str) {
        let key = (name.to_string(), version.to_string());
        if !self.advisory_checked.insert(key) {
            return;
        }
        let Some(feed) = Self::advisory_feed() else {
            return;
        };
        let hits = crate::registry_advisories::classify(name, version, feed);
        if hits.is_empty() {
            return;
        }
        let strict = std::env::var("LOFT_STRICT_SECURITY").is_ok();
        let overrides: std::collections::HashSet<String> = std::env::var("LOFT_SECURITY_OVERRIDE")
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        for hit in &hits {
            use crate::registry_advisories::Severity;
            let overridden = overrides.contains(&hit.advisory_id);
            match hit.severity {
                Severity::SecurityCritical => {
                    // Loud error block — same in strict and non-strict
                    // mode.  The DIFFERENCE is only whether we then
                    // refuse to proceed.
                    eprintln!(
                        "error: {} {} was yanked for a security vulnerability",
                        hit.package, hit.version
                    );
                    eprintln!("  advisory: {}", hit.advisory_id);
                    eprintln!("  summary:  {}", hit.summary);
                    if let Some(fix) = &hit.fixed_in {
                        eprintln!(
                            "  fix:      {} >= {} (run `loft install {}@{}`)",
                            hit.package, fix, hit.package, fix
                        );
                    }
                    if strict && !overridden {
                        eprintln!("  refused under LOFT_STRICT_SECURITY=1");
                        eprintln!(
                            "  override (audit-trail required): LOFT_SECURITY_OVERRIDE={}",
                            hit.advisory_id
                        );
                        std::process::exit(3);
                    }
                    if strict && overridden {
                        eprintln!(
                            "[security] override applied: {} ({} {})",
                            hit.advisory_id, hit.package, hit.version
                        );
                    }
                    // Non-strict mode: error block printed, but we
                    // proceed.  User retains the right to run their
                    // code while they investigate / fix.
                }
                Severity::SecurityHigh => {
                    eprintln!(
                        "warning: {} {} has a known security issue",
                        hit.package, hit.version
                    );
                    eprintln!("  advisory: {}", hit.advisory_id);
                    eprintln!("  summary:  {}", hit.summary);
                    if let Some(fix) = &hit.fixed_in {
                        eprintln!("  fix:      {} >= {}", hit.package, fix);
                    }
                }
                Severity::SecurityLow | Severity::Bug => {
                    eprintln!(
                        "warning: {} {} — {} (advisory {})",
                        hit.package, hit.version, hit.summary, hit.advisory_id
                    );
                }
                Severity::Deprecated => {
                    eprintln!(
                        "note: {} {} is deprecated — {} (advisory {})",
                        hit.package, hit.version, hit.summary, hit.advisory_id
                    );
                }
            }
        }
    }

    /// No-op when registry feature is off.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self, dead_code)]
    pub(super) fn check_advisory(&mut self, _name: &str, _version: &str) {}

    /// Lazy process-global advisory feed loader.  Cached for the
    /// duration of the process; offline-respecting; never refetches
    /// from network mid-parse (the explicit `loft audit` flow does
    /// that).  Returns `None` when the registry doesn't yet host an
    /// advisory feed (current real state — HTTP 404 soft-fails to
    /// "no advisories").
    #[cfg(feature = "registry")]
    fn advisory_feed() -> Option<&'static crate::registry_advisories::AdvisoryFeed> {
        use std::sync::OnceLock;
        static FEED: OnceLock<Option<crate::registry_advisories::AdvisoryFeed>> = OnceLock::new();
        FEED.get_or_init(|| {
            let opts = crate::registry_advisories::LoadOptions {
                allow_unsigned: true,
                // Mid-parse, never hit the network — always cache-only.
                // `loft audit` does the freshness refresh on demand.
                offline: true,
                refresh: false,
            };
            crate::registry_advisories::load_or_fetch(&opts).unwrap_or(None)
        })
        .as_ref()
    }

    /// The lockfile that governs this program, read once — @PLN143 arc B.
    ///
    /// WHICH lockfile that is comes from [`crate::resolution_scope::resolution_scope`],
    /// not from where this probe sits in the chain: a package's root `loft.lock`, the
    /// `<script>.loft.lock` beside a pinned script, or — in `Bare` scope — nothing,
    /// because nothing is declared there.
    ///
    /// Three probes used to answer this, each re-deriving its own lockfile path
    /// independently (beside the script, at the project root, and in the **cwd**), and a
    /// disagreement between them was silent: a different version loads and nothing
    /// errors.  The cwd leg is the one that made the same script resolve two ways from
    /// two directories.
    ///
    /// Answers whether THIS probe resolved `id` — the caller needs to tell a version the
    /// AUTHOR declared (a sidecar pin) from one the box merely happens to hold
    /// (loft#968), and pairs that answer with the scope it already holds.
    #[cfg(feature = "registry")]
    pub(super) fn probe_governing_lock(
        &mut self,
        id: &str,
        f: &mut String,
        cur_script: &str,
        scope: &crate::resolution_scope::ResolutionScope,
    ) -> bool {
        if file_exists(f) {
            return false;
        }
        // `None` is `Bare` scope: nothing is declared, so nothing pins this run.  There
        // used to be a leg here reading the CWD's `loft.lock` — a file an earlier RUN
        // wrote, in a directory that is not even the program's, deciding which version
        // this run gets.
        let Some(version) = self.lock_pin_in_force(id, scope, cur_script) else {
            return false;
        };
        self.resolve_registry_installed(id, &version, f);
        if file_exists(f) {
            self.pin_behind_notice(id, &version, cur_script, scope);
            return true;
        }
        false
    }

    /// No-op when the registry feature is off — a lockfile pins a REGISTRY version, and
    /// without the registry machinery there is nothing for it to name.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    pub(super) fn probe_governing_lock(
        &mut self,
        _id: &str,
        _f: &mut String,
        _cur_script: &str,
        _scope: &crate::resolution_scope::ResolutionScope,
    ) -> bool {
        false
    }

    /// The governing lock's version for `id`, when no declaration in force has since
    /// excluded it: the root `loft.toml`'s, and that of the package whose file says
    /// `use id` (`cur_script`).
    ///
    /// [`crate::install::constraint_for`] is the one home of that rule (a lock records how
    /// the manifest was last RESOLVED, so a pin a declaration has moved away from is stale
    /// and the declaration wins); this asks it whether the pin still stands.  The load used
    /// to take the pin without asking, while the install beside it did ask: `=0.1.0`
    /// declared over a `0.1.2` lock loaded 0.1.2, and nothing said so.
    #[cfg(feature = "registry")]
    pub(super) fn lock_pin_in_force(
        &mut self,
        id: &str,
        scope: &crate::resolution_scope::ResolutionScope,
        cur_script: &str,
    ) -> Option<String> {
        let pinned = scope.pinned_version(id)?;
        let stands = |declared: Option<String>| {
            crate::install::constraint_for(Some(&pinned), declared.as_deref()).as_deref()
                == Some(pinned.as_str())
        };
        (stands(self.root_dep_constraint(id)) && stands(Self::declaring_range(cur_script, id)))
            .then_some(pinned.clone())
    }

    /// Point `f` at an installed registry package's extracted source, given the
    /// resolved `version`. The shared tail of every registry resolve path
    /// (`extract_dir` → `lib_path_manifest`). Separated (#634) so a transitive
    /// dep can be resolved straight from the install REPORT — a cache-internal
    /// auto-install writes no lockfile (it must not mutate the immutable cache),
    /// so a resolver that could only learn the version by reading a lockfile back
    /// would leave `use <dep>` unresolved even though the package is installed.
    #[cfg(feature = "registry")]
    pub(super) fn resolve_registry_installed(&mut self, id: &str, version: &str, f: &mut String) {
        if !f.is_empty() && file_exists(f) {
            return;
        }
        let install_dir = crate::registry_index::extract_dir(id, version);
        let Some(parent) = install_dir.parent().and_then(std::path::Path::to_str) else {
            return;
        };
        let parent = parent.to_string();
        let Some(versioned_name) = crate::file_access::file_name(&install_dir) else {
            return;
        };
        if let Some(entry) = self.lib_path_manifest(&parent, &versioned_name) {
            self.check_advisory(id, version);
            *f = entry;
        }
    }

    /// @PLAN12 Phase 6.6 — auto-install on `use`.
    ///
    /// When `id` doesn't resolve via any of the prior strategies
    /// (path-dep, sibling lookup, lockfile + cached registry install)
    /// AND `id` is a known package name in the registry catalog,
    /// fire `install_one` to fetch + extract + lockfile-update,
    /// then re-run the cached-registry-install probe.
    ///
    /// The Python comparison: `python my_script.py` with
    /// `import requests` works if `pip install requests` was done
    /// once.  Loft's equivalent — `loft my_script.loft` with
    /// `use gridmesh;` — Just Works on first run by doing the
    /// `pip install` step on the user's behalf.
    ///
    /// Off-switches: `LOFT_OFFLINE=1` and `LOFT_NO_AUTO_INSTALL=1`
    /// both suppress this probe.  Surprise reduction: every cold
    /// install prints `[registry] ...` lines (mirrors Cargo's
    /// "Downloading…" output); steady-state (cache hit, resolved by
    /// `probe_governing_lock`) is silent.
    #[cfg(feature = "registry")]
    pub(super) fn probe_auto_install(
        &mut self,
        id: &str,
        f: &mut String,
        cur_script: &str,
        scope: &crate::resolution_scope::ResolutionScope,
    ) {
        if file_exists(f) {
            return;
        }
        // Off-switches.
        if std::env::var("LOFT_OFFLINE").is_ok() {
            return;
        }
        if std::env::var("LOFT_NO_AUTO_INSTALL").is_ok() {
            return;
        }
        // Already decided this run: resolve to the SAME version rather than asking the
        // registry a second time.  Both parse passes walk this path, and a second
        // decision could answer differently — a run whose two passes read two different
        // files is a worse failure than the one this probe exists to prevent.
        if let Some(known) = self.auto_installed.get(id).cloned() {
            if let Some(version) = known {
                self.resolve_registry_installed(id, &version, f);
            }
            return;
        }
        // A transitive dep discovered while parsing an ALREADY-CACHED package
        // (`~/.loft/registry/<pkg>/src/...`) has no consumer project: the only
        // `loft.toml` the walk-up finds is the cached dep's own, so recording there
        // would mutate the immutable cache — a harmless stray file on Unix, but an
        // ENOENT that aborts the whole resolution on Windows (nightly
        // `moros_glb_cli_end_to_end`).  Install without recording.
        let in_registry_cache = Self::script_in_registry_cache(cur_script);
        // What this path installs UNDER — which lock governs it, which one it may write,
        // and the signature it never waives — is one tested fact rather than a literal
        // buried here (@PLN143).
        let opts = crate::install::options_for_use(
            scope,
            in_registry_cache,
            std::env::var_os("LOFT_OFFLINE").is_some(),
        );
        // Honour the declaration in force.  Two of them can be, and they are not the
        // same kind: the root project's `[dependencies]` range (which also reaches a
        // transitively-pulled `id` a library never pinned), and the governing lock's
        // EXACT version — the resolved form of that range.  @PLN143: the lock used to
        // decide only which file LOADED, so a pinned version that was not extracted yet
        // was installed as "newest", and a fresh box ran a different program than the
        // machine that pinned it.
        // And the range the package whose file says `use id` names for it — the same third
        // declaration `probe_cache_newest` honours (loft#1849).
        let pinned = self.lock_pin_in_force(id, scope, cur_script);
        let pin = crate::install::use_constraint(
            pinned.as_deref(),
            self.root_dep_constraint(id).as_deref(),
            Self::declaring_range(cur_script, id).as_deref(),
        );
        match crate::install::auto_install_if_in_catalog(id, pin.as_deref(), &opts) {
            Ok(Some(report)) => {
                // Resolve `f` straight from the install report's version — no
                // lockfile round-trip (#634): a cache-internal install writes no
                // lockfile, so the lockfile-based probes below would leave a
                // freshly-installed transitive dep unresolved (`use <dep>` → "not
                // found") even though it is on disk. The report carries the exact
                // version, whether newly installed or already cached.
                let resolved = report
                    .installed
                    .iter()
                    .chain(report.skipped_cached.iter())
                    .find(|(n, _)| n == id)
                    .map(|(_, v)| v.clone());
                if let Some(ref version) = resolved {
                    self.resolve_registry_installed(id, version, f);
                }
                self.auto_installed.insert(id.to_string(), resolved);
                // Fallback for a normal (non-cache) install that DID write a lockfile:
                // read back the lock this run just recorded the new entry in.
                self.probe_governing_lock(id, f, cur_script, scope);
            }
            Ok(None) => {
                // `id` is not a registry package; let the remaining
                // resolution strategies handle it (or fail with
                // the standard "library not found" diagnostic).  Recorded so the second
                // parse pass does not re-read the index to be told the same thing.
                self.auto_installed.insert(id.to_string(), None);
            }
            Err(e) => {
                // Network failure, sig mismatch, or similar.
                // Print a notice but let resolution fall through —
                // the user may have a path-dep or sibling that
                // resolves anyway, or they may want the standard
                // error.
                eprintln!("[registry] auto-install failed for {id}: {}", e.message);
                if e.registry_answered {
                    self.registry_refused.insert(id.to_string());
                }
            }
        }
    }

    /// No-op when registry feature is off.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    pub(super) fn probe_auto_install(
        &mut self,
        _id: &str,
        _f: &mut String,
        _cur_script: &str,
        _scope: &crate::resolution_scope::ResolutionScope,
    ) {
    }

    /// @PLN143 arc E — say once when the declaration in force has fallen behind.
    ///
    /// A `loft.lock` has no expiry, so a pin holds forever and nothing about the
    /// program's own code will ever say why: `cbor` 0.1.3 turned canonical map encode
    /// from O(n³) to O(n²) with no API change at all, and a consumer pinned at 0.1.2
    /// keeps hanging. `loft check` reports it, but that is a verb you have to think to
    /// run — it only helps someone who already suspects what it would tell them.
    ///
    /// What it costs is what shapes it:
    ///
    /// - **No fetch.** It speaks only when the cached index ALREADY knows (the index has
    ///   its own 1-hour TTL); with no cache there is nothing to compare and it is silent.
    /// - **Once per package per run**, not once per parse pass — and never for a
    ///   dependency's own `use`, because "latest" is about what the PROGRAM named, and a
    ///   deep graph would otherwise turn every build into a report.
    /// - **Silent under `LOFT_OFFLINE`**: someone who has opted out of the registry for
    ///   this run has not asked what the registry now holds.
    /// - **Silent where nothing is pinned.** A bare script re-decides every run, so it
    ///   cannot be behind — the notice exists for the scope that HAS a declaration.
    ///
    /// It names the cure the scope actually takes: `loft install <pkg>` updates a
    /// package's lock beside its manifest, while a pinned script is re-pinned with
    /// `loft pin <script>`.
    ///
    /// Off-switch: `LOFT_NO_UPGRADE_NOTICE=1`.
    #[cfg(feature = "registry")]
    pub(super) fn pin_behind_notice(
        &mut self,
        id: &str,
        version: &str,
        cur_script: &str,
        scope: &crate::resolution_scope::ResolutionScope,
    ) {
        if std::env::var_os("LOFT_NO_UPGRADE_NOTICE").is_some()
            || std::env::var_os("LOFT_OFFLINE").is_some()
        {
            return;
        }
        // A `use` inside an already-cached package is the dependency's own; its pins are
        // its author's business, and the consumer cannot act on them.
        if Self::script_in_registry_cache(cur_script) {
            return;
        }
        if !self.pin_behind_reported.insert(id.to_string()) {
            return;
        }
        let newest = self
            .newest_releases
            .get_or_insert_with(crate::install::newest_cached_releases);
        let Some(newest) = newest.get(id) else {
            return;
        };
        if crate::registry_index::compare_semver(newest, version) != std::cmp::Ordering::Greater {
            return;
        }
        let cure = match scope {
            crate::resolution_scope::ResolutionScope::PinnedScript(_) => {
                format!("loft pin {cur_script}")
            }
            _ => format!("loft install {id}"),
        };
        eprintln!(
            "[registry] {id} {version} is pinned; {newest} is the newest release — run: {cure}"
        );
    }

    /// A script falls back to the newest version already in the cache
    /// that the declaration governing it allows.
    ///
    /// The failure path this exists for: offline (or a registry that cannot be reached),
    /// and the package sitting right there under `~/.loft/registry/`.  That answered
    /// *"Library 'x' not found — searched lib/, lib_dirs, and sibling packages"* in a
    /// directory holding five extracted copies of it, which is the least true message
    /// available.
    ///
    /// **The fallback never answers past a declaration.**  A `Bare` scope declares nothing,
    /// so it takes the newest loadable copy.  A declared scope — a package, a pinned script,
    /// and every library parsed out of the cache, whose own manifest governs its `use`s —
    /// takes the newest loadable copy that satisfies the SAME constraint the online path
    /// resolves under (the governing lock's pin against the root project's range,
    /// [`crate::install::constraint_for`]) AND the range the declaring package itself names
    /// for `id` (PKG_REGISTRY.md § failure path 5: a transitive dep is resolved by the
    /// library's own manifest constraint).  Until loft#1687 a declared scope did not fall
    /// back at all, so `use graphics;` failed offline on `use mesh3d;` inside graphics with
    /// `mesh3d 0.1.1` cached and graphics asking `>=0.1.1`.  Where the cache holds copies and
    /// none satisfies, the constraint and the copies are recorded for the "not found"
    /// message, which then names what was unmet.
    ///
    /// Runs after `probe_auto_install`, so an online run still resolves the newest
    /// RELEASE; the cache only answers when the registry could not.
    #[cfg(feature = "registry")]
    pub(super) fn probe_cache_newest(
        &mut self,
        id: &str,
        f: &mut String,
        cur_script: &str,
        scope: &crate::resolution_scope::ResolutionScope,
    ) {
        if file_exists(f) {
            return;
        }
        let mut constraints: Vec<String> =
            if *scope == crate::resolution_scope::ResolutionScope::Bare {
                Vec::new()
            } else {
                let pinned = self.lock_pin_in_force(id, scope, cur_script);
                let root = self.root_dep_constraint(id);
                crate::install::constraint_for(pinned.as_deref(), root.as_deref())
                    .into_iter()
                    .collect()
            };
        // The package whose file says `use id` declared a range for it, and that holds in
        // every scope: a cached library used from a bare script is bound by its own
        // `[dependencies]` all the same (loft#1687).
        if let Some(own) = Self::declaring_range(cur_script, id)
            && !constraints.contains(&own)
        {
            constraints.push(own);
        }
        if let Some((version, _)) =
            crate::registry_index::newest_cached_loadable_satisfying(id, &constraints)
        {
            self.resolve_registry_installed(id, &version, f);
            return;
        }
        let cached = crate::registry_index::cached_versions(id);
        if !constraints.is_empty() && !cached.is_empty() {
            // An online auto-install already asked (and refreshed its index first), so the
            // registry was not unreachable: it has nothing in range, and its reason is the
            // `[registry]` line above.
            let registry = if self.registry_refused.contains(id) {
                "the registry has no version in range"
            } else {
                "the registry could not be asked"
            };
            self.cache_unmet.insert(
                id.to_string(),
                format!(
                    "{registry}, and no cached copy satisfies `{}` (cached: {})",
                    constraints.join("`, `"),
                    cached.join(", ")
                ),
            );
        }
    }

    /// No-op when the registry feature is off — there is no registry cache to fall back
    /// to.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    pub(super) fn probe_cache_newest(
        &mut self,
        _id: &str,
        _f: &mut String,
        _cur_script: &str,
        _scope: &crate::resolution_scope::ResolutionScope,
    ) {
    }

    /// The version range the package declaring `cur_script` names for `id` in its own
    /// `[dependencies]` — `None` when there is no such package, no such entry, or the entry
    /// is a pure path dependency (resolved by path, never from the registry cache).
    #[cfg(feature = "registry")]
    pub(super) fn declaring_range(cur_script: &str, id: &str) -> Option<String> {
        let root = crate::resolution_scope::project_root(cur_script)?;
        let m = crate::manifest::read_manifest(&root.join("loft.toml").to_string_lossy())?;
        let (_, value) = m.dependencies.iter().find(|(n, _)| n == id)?;
        crate::manifest::extract_version_req(value).map(str::to_string)
    }

    /// Final fallback: beside the parsed file itself.
    pub(super) fn probe_cur_dir_flat(id: &str, cur_dir: &str, f: &mut String) {
        if !cur_dir.is_empty() && !file_exists(f) {
            *f = format!("{cur_dir}{0}{id}.loft", sep_str());
        }
    }

    /// Final fallback for scripts inside a `/tests/` tree.
    pub(super) fn probe_base_dir_flat(id: &str, base_dir: &str, f: &mut String) {
        if !base_dir.is_empty() && !file_exists(f) {
            *f = format!("{base_dir}{0}{id}.loft", sep_str());
        }
    }

    /// @PLN146 F5/F4 — record what this package's `--html` page has to carry: its
    /// `[[font]]` families and its `[[embed]]` files.  Both manifest-registration
    /// paths call it, for the same reason `[wasm.bridge].host_js` is registered from
    /// both: a library reached through one path and not the other would lose its
    /// browser half silently.  Identical declarations are one entry — the app and a
    /// library it uses may name the same font or the same pack, and agreeing about
    /// it is not a conflict.
    pub(super) fn register_page_declarations(&mut self, m: &manifest::Manifest) {
        for f in &m.fonts {
            if !self.data.declared_fonts.contains(f) {
                self.data.declared_fonts.push(f.clone());
            }
        }
        // @PLN146 F4 — and the files this package's page must carry.  Same route,
        // same reason: a library reached through one registration path and not the
        // other would lose its browser half with nothing said.  Each declaration
        // already knows the manifest dir its `source` is relative to, so a
        // consumer's build finds the LIBRARY's file rather than looking for one of
        // its own with that name.
        for e in &m.embeds {
            if !self.data.declared_embeds.contains(e) {
                self.data.declared_embeds.push(e.clone());
            }
        }
    }

    /// Register native crate info from a loft.toml manifest.
    /// Called when a .loft file was found directly via lib_dirs (not through
    /// lib_path_manifest), so the manifest's native crate registration would
    /// otherwise be skipped.
    /// Does this loft satisfy the package's `loft = ">=…"` floor?  ONE home for the
    /// question, asked on EVERY path that adopts a manifest — a `--lib` resolution, a
    /// sibling package a library's own `use` reaches, the ancestor walk — because a floor
    /// checked on one path and not another is a guard the `use` ORDER decides: `use a; use b`
    /// refused `b`'s floor while `use b; use a` accepted it, `b` having been adopted as `a`'s
    /// sibling first and the direct `use` then deduplicated (@PLN174 F6).  A refusal is
    /// fatal: the floor names the first release whose runtime the package's code needs, and
    /// an older host reading past what it was given is the failure the floor exists to stop.
    pub(super) fn loft_floor_holds(&mut self, id: &str, m: &manifest::Manifest) -> bool {
        let Some(ref req) = m.loft_version else {
            return true;
        };
        let current = crate::manifest::LOFT_RUNNING_VERSION;
        match manifest::check_version(req, current) {
            manifest::VersionCheck::Satisfied => true,
            manifest::VersionCheck::Unsatisfied => {
                diagnostic!(
                    self.lexer,
                    Level::Fatal,
                    "Package '{id}' requires loft {req} but interpreter is {current}"
                );
                false
            }
            // @PLN102 arc B: a constraint the loader cannot honour is
            // rejected loudly, not silently treated as "any version".
            manifest::VersionCheck::Malformed(why) => {
                diagnostic!(
                    self.lexer,
                    Level::Fatal,
                    "Package '{id}' has an invalid loft version requirement '{req}': {why}"
                );
                false
            }
        }
    }

    pub(super) fn register_native_manifest(
        &mut self,
        manifest_path: &std::path::Path,
        pkg_dir: &std::path::Path,
    ) {
        let Some(m) = manifest::read_manifest(manifest_path.to_str().unwrap_or("")) else {
            return;
        };
        let id = m
            .name
            .clone()
            .unwrap_or_else(|| crate::file_access::file_name(pkg_dir).unwrap_or_default());
        if !self.loft_floor_holds(&id, &m) {
            return;
        }
        let pkg_dir = pkg_dir.to_string_lossy().to_string();
        // Register the dlopen-side native lib path (interpreter mode).  The
        // `[library] native = "..."` form registers the cdylib for dlopen;
        // the separate `[native] crate = "..."` block (handled below)
        // registers the rlib for the native-compile path.  Both must run
        // here for sibling-package and ancestor-walk paths, otherwise
        // packages depended on by a no-native parent (e.g. an examples
        // package that uses `lib/server`) lose their native bindings in
        // interpreter mode.
        if let Some(ref stem) = m.native
            && let Some(path) = crate::native_build::host_native_lib(&pkg_dir, stem)
            && !self.pending_native_libs.contains(&path)
        {
            self.pending_native_libs.push(path);
            self.native_lib_regs.push((stem.clone(), pkg_dir.clone()));
        }
        // @PLN11 N3 Step 3 (default-native) / F2 — mirror `apply_manifest_side_effects`:
        // a normal loft library reached via THIS direct-resolution / sibling-package /
        // ancestor-walk path must ALSO be recorded as a native-compile candidate.
        // Without it the two resolution paths diverge: a library pulled in transitively
        // (or used directly *after* it was already loaded transitively, so the direct
        // `use` dedups) is loaded here but never recorded — so it never builds its own
        // cdylib and its direct calls interpret.  A hand-written cdylib (`native`) serves
        // the library's `#native` calls; its LOFT functions compile too when the library
        // also declares the `[native] crate` that cdylib is built from, which is what the
        // generated code links its `#native` calls against (@PLN199 E2).
        if (m.native.is_none() || m.native_crate.is_some())
            && !self.pending_native_compile.iter().any(|d| d == &pkg_dir)
        {
            self.pending_native_compile.push(pkg_dir.clone());
        }
        self.record_placement(&m, &pkg_dir);
        self.register_c_libraries(&m, &pkg_dir);
        if let Some(ref crate_name) = m.native_crate {
            let rust_crate = crate_name.replace('-', "_");
            if !self
                .data
                .native_packages
                .iter()
                .any(|(c, _)| c == crate_name)
            {
                self.data
                    .native_packages
                    .push((crate_name.clone(), pkg_dir.clone()));
            }
            // @PLAN12 phase 2 step 2 (2026-05-24) — same manifest-driven
            // `native_symbols` + `def.native` population as
            // `apply_manifest_side_effects` (the legacy path).  The
            // sibling-package probe (used when a script reaches a library
            // via cwd ancestry rather than `--lib`) hits THIS function;
            // without the same population, `[native.functions]` entries
            // would be visible only to native codegen via the legacy
            // path's `native_symbols` lookup — the sibling path would
            // leave def.native empty and the interpreter dispatch path
            // would silently fall through to OpCall with an empty body,
            // returning null instead of dispatching to the cdylib.
            for (loft_name, rust_symbol) in &m.native_functions {
                self.data
                    .native_symbols
                    .insert(loft_name.clone(), rust_symbol.clone());
            }
            for (loft_name, rust_symbol) in &m.native_functions {
                let candidates = [format!("n_{loft_name}"), loft_name.clone()];
                for d_nr in 0..self.data.definitions() {
                    let def = self.data.def(d_nr);
                    if !def.native().is_empty() {
                        continue;
                    }
                    if !candidates.iter().any(|c| c == def.name()) {
                        continue;
                    }
                    if !crate::file_access::is_under(def.position().file.as_str(), &pkg_dir) {
                        continue;
                    }
                    rust_symbol.clone_into(&mut self.data.definitions[d_nr as usize].native);
                }
            }
            // P266: same ownership-driven restriction as
            // `apply_manifest_side_effects` above — only map `#native`
            // symbols whose definition lives in THIS package's source
            // tree, so out-of-order manifest/source loading can't make
            // one package claim another's symbols.
            for d_nr in 0..self.data.definitions() {
                let def = self.data.def(d_nr);
                let sym = def.native();
                if sym.is_empty() {
                    continue;
                }
                if !crate::file_access::is_under(def.position().file.as_str(), &pkg_dir) {
                    continue;
                }
                if self.data.native_symbol_crates.contains_key(sym) {
                    continue;
                }
                self.data
                    .native_symbol_crates
                    .insert(sym.to_string(), rust_crate.clone());
            }
        }
        // #453 — a `[wasm.bridge]` library must register its bridge crate +
        // routes + host_js even with NO `[native]`. This used to sit INSIDE the
        // `m.native_crate` guard above, so a browser-only bridge lib (no native
        // crate) reached through `--lib` / a `path` dep / the sibling walk
        // silently dropped its routes and `--html` failed with P269. Register it
        // ungated, mirroring `apply_manifest_side_effects` — the legacy resolution
        // path that always got this right. (lib_plan-29 W1c bridge crate + routes,
        // W2 host_js.)
        if let Some(ref bridge_crate) = m.wasm_bridge_crate {
            if !self
                .data
                .wasm_bridge_packages
                .iter()
                .any(|(c, _)| c == bridge_crate)
            {
                self.data
                    .wasm_bridge_packages
                    .push((bridge_crate.clone(), pkg_dir.clone()));
            }
            for (loft_sym, bridge_fn) in &m.wasm_bridge_routes {
                self.data
                    .wasm_bridge_routes
                    .insert(loft_sym.clone(), (bridge_crate.clone(), bridge_fn.clone()));
            }
        }
        if let Some(ref host_js_rel) = m.wasm_bridge_host_js {
            let abs = std::path::Path::new(&pkg_dir).join(host_js_rel);
            let abs_str = abs.to_string_lossy().to_string();
            if !self.data.wasm_bridge_host_js_files.contains(&abs_str) {
                self.data.wasm_bridge_host_js_files.push(abs_str);
            }
        }
        self.register_page_declarations(&m);
    }

    /// Check whether `<dir>/<id>` contains a valid loft package layout.
    /// Reads `loft.toml` when present and validates the interpreter version
    /// requirement.  Emits a fatal diagnostic on version mismatch.
    /// Returns `Some(entry_path)` when the layout exists and the version passes,
    /// `None` otherwise.
    ///
    /// Legacy path: delegates to [`lib_path_manifest_resolve`] for pure
    /// resolution, then applies side-effects via [`apply_manifest_side_effects`].
    /// Phase A calls `lib_path_manifest_resolve`
    /// directly and builds the package graph explicitly.
    pub(super) fn lib_path_manifest(&mut self, dir: &str, id: &str) -> Option<String> {
        let resolved = self.lib_path_manifest_resolve(dir, id)?;
        if let Some(m) = resolved.manifest.as_ref() {
            self.apply_manifest_side_effects(dir, &resolved.pkg_dir, m);
        }
        Some(resolved.entry)
    }

    /// Pure resolution of `<dir>/<id>` against disk + manifest.  No side
    /// effects on `self.data` / `self.lib_dirs` / `self.pending_*`; the only
    /// state touched is `self.lexer.diagnostics` on a version-mismatch fatal.
    ///
    /// This is the entry point used by Phase A of the package-mode driver
    /// , which needs to enumerate files + package edges without
    /// spilling symbol-table side-effects before pass-1 parsing begins.
    pub(super) fn lib_path_manifest_resolve(&mut self, dir: &str, id: &str) -> Option<ResolvedPkg> {
        // @P296-sibling (Windows) — build the package paths with
        // `Path::join` rather than `format!("{dir}/{id}")`.  When `dir` is
        // an absolute Windows path (`D:\a\loft\loft\lib`, e.g. a `--lib`
        // argument built via `PathBuf::join`), interpolating a literal `/`
        // produces a mixed-separator path; `Path::join` uses the platform
        // separator consistently so nested `--lib` packages resolve on
        // Windows (the crystal_gold CI failure: "Library 'audience_crystal'
        // not found").  No behavioural change on Linux/macOS.
        // #408 — accept `--lib` pointing AT the package dir itself (its basename
        // matches `id` and it carries a loft.toml), not only at its parent.
        // Without this, `--lib path/to/crypto` looks for `path/to/crypto/crypto`,
        // fails, and `use crypto` falls through to an installed registry copy —
        // the shadowing that made a local lib's `[wasm.bridge]` routes unreachable
        // in `--html`. Falls back to `<dir>/<id>` for the normal parent-dir search.
        let dir_pb = std::path::Path::new(dir);
        let pkg_dir_pb = if crate::file_access::file_name(dir_pb).as_deref() == Some(id)
            && crate::file_access::is_file(dir_pb.join("loft.toml"))
        {
            dir_pb.to_path_buf()
        } else {
            dir_pb.join(id)
        };
        if !crate::file_access::is_dir(&pkg_dir_pb) {
            return None;
        }
        let pkg_dir = pkg_dir_pb.to_string_lossy().into_owned();
        let manifest_pb = pkg_dir_pb.join("loft.toml");
        let nested_entry = || {
            pkg_dir_pb
                .join("src")
                .join(format!("{id}.loft"))
                .to_string_lossy()
                .into_owned()
        };
        let (entry, manifest) = if crate::file_access::exists(&manifest_pb) {
            let manifest_path = manifest_pb.to_string_lossy().into_owned();
            let m = manifest::read_manifest(&manifest_path)?;
            if !self.loft_floor_holds(id, &m) {
                return None;
            }
            // @PLN102 arc B-semantic — the compatibility `contract` axis (a
            // monotone integer; increments on a silent breaking change, distinct
            // from the calendar release tag above).  Too-old is a hard reject;
            // drift (loft advanced past the tested epoch) WARNS — the arc-C
            // deprecation channel — and loads, so the fix is the author
            // republishing, never a silent wrong answer for the consumer.
            if let Some(ref creq) = m.contract {
                let cur = manifest::CONTRACT_VERSION;
                match manifest::check_contract(creq, cur) {
                    manifest::ContractCheck::Ok => {}
                    manifest::ContractCheck::TooOld { required_min } => {
                        diagnostic!(
                            self.lexer,
                            Level::Fatal,
                            "Package '{id}' requires loft contract >= {required_min} but this loft is contract {cur}"
                        );
                        return None;
                    }
                    manifest::ContractCheck::Drifted { tested_max } => {
                        diagnostic!(
                            self.lexer,
                            Level::Warning,
                            code = "package-contract-drifted",
                            "Package '{id}' was tested against loft contract <= {tested_max} \
                             but this loft is contract {cur} — a breaking change since then \
                             may make it misbehave"
                        );
                        self.lexer.fix_last(crate::diagnostics::Fix {
                            kind: crate::diagnostics::FixKind::Conditional,
                            title: format!(
                                "ask its author to republish `{id}` against contract {cur}"
                            ),
                            condition: Some(
                                "you depend on behaviour a contract bump could have changed"
                                    .to_string(),
                            ),
                            edit: None,
                            concept: "package management",
                            concept_ref: "@F55",
                        });
                    }
                    manifest::ContractCheck::Malformed(why) => {
                        diagnostic!(
                            self.lexer,
                            Level::Fatal,
                            "Package '{id}' has an invalid loft contract requirement '{creq}': {why}"
                        );
                        return None;
                    }
                }
            }
            let entry = m.entry.as_ref().map_or_else(nested_entry, |e| {
                pkg_dir_pb.join(e).to_string_lossy().into_owned()
            });
            (entry, Some(m))
        } else {
            (nested_entry(), None)
        };
        if file_exists(&entry) {
            Some(ResolvedPkg {
                pkg_dir,
                entry,
                manifest,
            })
        } else {
            None
        }
    }

    /// Apply the parser-state side effects that the legacy `lib_path_manifest`
    /// performs for a resolved package: native-lib registration,
    /// native-symbol / native-crate bookkeeping, sibling-dependency search
    /// paths (`lib_dirs`), and queued transitive package loads
    /// (`pending_pkg_deps`).
    /// @PLN24 arc D — register a package's C libraries: the ones it DECLARES
    /// (`[c] libs`) and the one it SHIPS SOURCE FOR (`[c] shim`, built by `cc`).
    ///
    /// A `[c]` entry is the only way a `#c` binding reaches past libc and the
    /// cdylibs loft already loaded, and both parser paths into a manifest need
    /// it — which is exactly why it lives here once. The plan's own risk is
    /// `N × silence`: a fact restated at several sites goes wrong quietly at
    /// each, and this fact already had two homes before the shim would have
    /// made it four.
    ///
    /// **A built shim is registered as an ordinary library**, so the
    /// interpreter's `dlopen`, the `--native` link line and the symbol resolver
    /// stay one code path and cannot disagree about what a shim is.
    /// @PLN119 arc A — note a library that asked to run out of process.
    ///
    /// An unreadable `placement` value is an ERROR rather than a fallback to
    /// in-process: the two placements are meant to be indistinguishable in
    /// behaviour, so a typo would produce a program that runs correctly and
    /// isolates nothing, and no output would ever show the difference.
    pub(super) fn record_placement(&mut self, m: &manifest::Manifest, pkg_dir: &str) {
        let Some(ref spelled) = m.placement else {
            return;
        };
        match crate::manifest::Placement::parse(spelled) {
            Ok(p) if p.is_out_of_process() => {
                let name = m
                    .name
                    .clone()
                    .unwrap_or_else(|| crate::file_access::name_of(pkg_dir));
                if !self
                    .pending_placed_libs
                    .iter()
                    .any(|(_, d, _)| d == pkg_dir)
                {
                    self.pending_placed_libs
                        .push((name, pkg_dir.to_string(), p));
                }
            }
            Ok(_) => {}
            Err(why) => self.lexer.diagnostic(
                Level::Error,
                &format!("`[library] placement` for `{pkg_dir}`: {why}"),
            ),
        }
    }

    pub(super) fn register_c_libraries(&mut self, m: &manifest::Manifest, pkg_dir: &str) {
        let mut entries: Vec<String> = m.c_libs.clone();
        if !m.c_shim.is_empty() {
            match crate::c_shim::build(pkg_dir, &m.c_shim) {
                Ok(so) => entries.push(so.to_string_lossy().into_owned()),
                // Reported, not swallowed: without the shim its symbols are
                // absent, and the failure the author would otherwise meet is
                // "`#c` symbol not found" at the call — which names neither the
                // shim nor the compiler that could not build it.
                Err(why) => self
                    .lexer
                    .diagnostic(Level::Error, &format!("`[c] shim` for `{pkg_dir}`: {why}")),
            }
        }
        // @PLN24 arc G — the optional list joins here, flagged. A shim is never
        // optional: the package ships its source and loft just built it, so it
        // is present by construction and an absent one is a build failure the
        // author has already been told about.
        let entries = entries
            .into_iter()
            .map(|l| (l, false))
            .chain(m.c_optional_libs.iter().map(|l| (l.clone(), true)));
        for (lib, optional) in entries {
            if !self
                .data
                .c_libraries
                .iter()
                .any(|c| c.name == lib && crate::file_access::same_path(&c.pkg_dir, pkg_dir))
            {
                self.data.c_libraries.push(crate::data::CLibrary {
                    name: lib,
                    pkg_dir: pkg_dir.to_string(),
                    optional,
                });
            }
        }
    }

    #[expect(clippy::too_many_lines, reason = "inherited")]
    pub(super) fn apply_manifest_side_effects(
        &mut self,
        dir: &str,
        pkg_dir: &str,
        m: &manifest::Manifest,
    ) {
        // register native shared library path for loading after byte_code().
        // Pre-built location first, then auto-build from source (one home:
        // `extensions::resolve_native_lib`, shared with the warm-cache load).
        if let Some(ref stem) = m.native
            && let Some(path) = crate::native_build::host_native_lib(pkg_dir, stem)
            && !self.pending_native_libs.contains(&path)
        {
            self.pending_native_libs.push(path);
            self.native_lib_regs
                .push((stem.clone(), pkg_dir.to_string()));
        }
        // @PLN11 Arc N / N3 Step 3 — **default-native**.  Every `use`d normal loft
        // library is a native candidate: record the package dir; the driver marks +
        // builds + loads after scope analysis (see `pending_native_compile`).  No
        // opt-in, no flag — "libraries compile, scripts interpret" is the default.
        //   - `native` (a hand-written cdylib via `[library] native = ...`) serves the
        //     library's `#native` calls; its loft functions compile as well when the
        //     library declares the `[native] crate` that cdylib is built from (@PLN199 E2).
        //   - `LOFT_NO_NATIVE_LIBS=1` is the interpret escape (handled in the driver:
        //     it clears `pending_native_compile`), used by the dev/edit loop + the
        //     parity reference until Step 4 (dev-interpret-on-edit) lands.
        //   - A build failure silently interprets (Step 2), so recording a library
        //     that can't compile native is harmless.
        // The legacy `[library] compile = "native"` opt-in is now redundant but still
        // accepted (a no-op): default-native already records it.
        if (m.native.is_none() || m.native_crate.is_some())
            && !self.pending_native_compile.iter().any(|d| d == pkg_dir)
        {
            self.pending_native_compile.push(pkg_dir.to_string());
        }
        // `LOFT_TRACE_NATIVE_GATE=1` — the one library the partition trace never reaches: a
        // hand-written native cdylib with no `[native] crate` to link against, so its LOFT
        // functions (the wrappers around its natives) all run interpreted.
        if let Some(stem) = &m.native
            && m.native_crate.is_none()
            && std::env::var_os("LOFT_TRACE_NATIVE_GATE").is_some()
        {
            let pkg = crate::file_access::file_name(pkg_dir).unwrap_or_else(|| pkg_dir.to_string());
            eprintln!(
                "[native-gate] {pkg}: its loft functions are interpreted — the library ships a \
                 native crate (`{stem}`), which takes the place of compiling it"
            );
        }
        self.record_placement(m, pkg_dir);
        // PKG.4: register native function symbols and package crate info.
        self.register_c_libraries(m, pkg_dir);
        if let Some(ref crate_name) = m.native_crate {
            let rust_crate = crate_name.replace('-', "_");
            if !self
                .data
                .native_packages
                .iter()
                .any(|(c, _)| c == crate_name)
            {
                self.data
                    .native_packages
                    .push((crate_name.clone(), pkg_dir.to_string()));
            }
            for (loft_name, rust_symbol) in &m.native_functions {
                self.data
                    .native_symbols
                    .insert(loft_name.clone(), rust_symbol.clone());
            }
            // @PLAN12 phase 2 step 2 (2026-05-24) — make
            // `[native.functions]` entries fully equivalent to
            // `#native "symbol"` annotations.  For each manifest
            // entry whose loft fn name matches a definition owned by
            // this package, populate `def.native` so the interpreter's
            // `wire_native_fns` (which keys off `def.native`) and the
            // bytecode dispatch (`state/codegen.rs:2207-2217`, which
            // also reads `def.native`) see the binding without
            // requiring the redundant `#native` annotation.  Native
            // codegen already consulted `native_symbols` directly so
            // this isn't strictly required for the native path, but
            // unifying the two paths is the whole point of step 2.
            //
            // Ownership check (`def.position.file.starts_with(pkg_dir)`)
            // mirrors the @P266-style guard below: only populate defs
            // physically inside this package's source tree.
            for (loft_name, rust_symbol) in &m.native_functions {
                let candidates = [format!("n_{loft_name}"), loft_name.clone()];
                for d_nr in 0..self.data.definitions() {
                    let def = self.data.def(d_nr);
                    if !def.native().is_empty() {
                        continue;
                    }
                    if !candidates.iter().any(|c| c == def.name()) {
                        continue;
                    }
                    if !crate::file_access::is_under(def.position().file.as_str(), pkg_dir) {
                        continue;
                    }
                    rust_symbol.clone_into(&mut self.data.definitions[d_nr as usize].native);
                }
            }
            // P266: map only `#native` symbols whose definition lives in
            // THIS package's source tree, not every unmapped symbol in
            // the whole `data.definitions()` list.  The earlier
            // walk-and-claim shape over-assigned: when manifests were
            // registered out-of-order with their sources (e.g. lib/web
            // manifest registered before lib/server's source was parsed,
            // then lib/server's manifest registered with both packages'
            // defs already in the table), it left lib/web's symbols
            // pointing at `loft_server` and lib/server's symbols pointing
            // at `loft_web`.  Restricting by `position.file.starts_with(
            // pkg_dir)` makes the assignment ownership-driven instead of
            // call-order-driven.
            for d_nr in 0..self.data.definitions() {
                let def = self.data.def(d_nr);
                let sym = def.native();
                if sym.is_empty() {
                    continue;
                }
                if !crate::file_access::is_under(def.position().file.as_str(), pkg_dir) {
                    continue;
                }
                if self.data.native_symbol_crates.contains_key(sym) {
                    continue;
                }
                self.data
                    .native_symbol_crates
                    .insert(sym.to_string(), rust_crate.clone());
            }
        }
        // lib_plan-29 W1c (2026-05-29) — register WASM bridge crate +
        // routes for `--html` builds.  Mirrors the `[native]` block
        // above, scoped to the browser-WASM target.
        if let Some(ref bridge_crate) = m.wasm_bridge_crate {
            if !self
                .data
                .wasm_bridge_packages
                .iter()
                .any(|(c, _)| c == bridge_crate)
            {
                self.data
                    .wasm_bridge_packages
                    .push((bridge_crate.clone(), pkg_dir.to_string()));
            }
            for (loft_sym, bridge_fn) in &m.wasm_bridge_routes {
                self.data
                    .wasm_bridge_routes
                    .insert(loft_sym.clone(), (bridge_crate.clone(), bridge_fn.clone()));
            }
        }
        // lib_plan-29 W2 (2026-05-29) — resolve `[wasm.bridge].host_js`
        // relative to the package root and register the absolute path.
        // The `--html` driver concatenates each registered file into
        // the HTML preamble.
        if let Some(ref host_js_rel) = m.wasm_bridge_host_js {
            let abs = std::path::Path::new(pkg_dir).join(host_js_rel);
            let abs_str = abs.to_string_lossy().to_string();
            if !self.data.wasm_bridge_host_js_files.contains(&abs_str) {
                self.data.wasm_bridge_host_js_files.push(abs_str);
            }
        }
        self.register_page_declarations(m);
        // PKG.3: register dirs for dependency resolution.
        //
        // For plain-version deps (`foo = "0.1"`) and the legacy
        // sibling-probe shape (no path declared): register the
        // package's parent dir `dir` so `<dir>/<dep_name>` resolves
        // via sibling lookup.
        //
        // @PLAN12 phase 3.5b (2026-05-24) — for path-deps
        // (`foo = { path = "../external/foo" }`), resolve the path
        // relative to this package's directory (`pkg_dir`), then
        // register the PARENT of the resolved location.  This lets
        // the existing sibling-probe at
        // `<resolved-parent>/<dep_name>/loft.toml` find the external
        // package.  Without this, the inline-table path field was
        // decorative — manifests like `lib/audience_crystal/loft.toml`
        // worked by coincidence (their `path = "../gridmesh"`
        // happened to point at a sibling in `lib/`, also covered by
        // the `--lib lib` cmdline arg).  This change makes the path
        // field actually do what it says, unlocking external library
        // extractions.
        for (dep_name, dep_value) in &m.dependencies {
            let resolved_parent = if let Some(path) = manifest::extract_path_dep(dep_value) {
                let dep_pkg_path = std::path::Path::new(pkg_dir).join(path);
                dep_pkg_path
                    .parent()
                    .map_or_else(|| dir.to_string(), |p| p.to_string_lossy().into_owned())
            } else {
                dir.to_string()
            };
            if !self.lib_dirs.contains(&resolved_parent) {
                self.lib_dirs.push(resolved_parent.clone());
            }
            if !self.data.use_exists(dep_name) {
                let from = self.lexer.pos().file.to_string();
                self.pending_pkg_deps
                    .push((dep_name.clone(), resolved_parent, from));
            }
        }
    }
}
