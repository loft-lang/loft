// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

//! `use` inside a program: the import spec after a target, a package's own modules, the
//! pending imports applied when their file becomes current, the trigger maps that auto-`use` a
//! package for a method name, and the clashes and ambiguities a bare name can run into.

use super::{DefType, Level, Parser, diagnostic_format, other_sep, script_dir, sep_str};

/**
The number of defined reserved text worker variables. A worker variable is needed when
two texts are added or a formatting text is used, and the result is used as a parameter to a call.
These are reused when possible. However, when calculating a text, a new text expression
is used a next worker variable is needed.
This number indicated the depth of these expressions, not the number of these expressions in a
function.
*/
// The parser holds several independent boolean mode flags (in_loop, default, first_pass,
// reverse_iterator) that each track a distinct parse phase or context.  Combining them into
// an enum or state machine would add complexity without benefit.
/// Whether a `use lib::...` statement imports all names or a specific subset.
#[derive(Clone)]
pub(super) enum ImportSpec {
    Wildcard,
    /// Each entry is `(name_in_library, bind_name)`; `bind_name == name` for a
    /// plain `use lib::name`, or the alias for `use lib::name as bind` (@PLN22
    /// Phase 3).
    Names(Vec<(String, String)>),
}

/// A pending import queued when `use lib::spec` is parsed.
/// Applied after all definitions in `for_source` are fully parsed.
#[derive(Clone)]
pub(super) struct PendingImport {
    pub(super) for_source: u16,
    pub(super) lib_source: u16,
    pub(super) spec: ImportSpec,
    /// `pub use`: the names pass on to whoever imports `for_source` (@C98).
    pub(super) public: bool,
}

impl Parser {
    /// #322: switch the lexer to a `use`-resolved dependency file,
    /// recording it for the program startup-cache manifest first.
    /// Library files are parsed via an inline lexer switch (never a
    /// `parse()` entry), so without this record the manifest misses
    /// them and an edited library keeps executing from the stale
    /// cached program.
    #[track_caller]
    pub(super) fn switch_to_dep(&mut self, f: &str) {
        // A `pub` the use region consumed belongs to the file being left; that file is
        // re-parsed from its start when it resumes.
        self.pub_taken = false;
        if std::env::var("LOFT_LIB_ORDER").is_ok() {
            eprintln!(
                "[liborder] switch {} -> {} (from {})",
                self.lexer.pos().file,
                f.rsplit('/').next().unwrap_or(f),
                std::panic::Location::caller()
            );
        }
        if self.track_sources && !self.parsed_sources.iter().any(|s| s == f) {
            self.parsed_sources.push(f.to_string());
        }
        self.lexer.switch(f);
    }

    /// Apply all pending imports whose target source matches the currently active source.
    pub(super) fn apply_pending_imports(&mut self) {
        let cur = self.data.source;
        // Partition: imports targeting `cur` are applied now; others wait for their source.
        let mut to_apply = Vec::new();
        let mut remaining = Vec::new();
        for pi in self.pending_imports.drain(..) {
            if pi.for_source == cur {
                to_apply.push(pi);
            } else {
                remaining.push(pi);
            }
        }
        self.pending_imports = remaining;
        if !to_apply.is_empty() {
            // The file these imports serve: a definition written in it is never one of its
            // private imports (see `Data::note_source_file`).
            let here_file = self.lexer.pos().file;
            self.data.note_source_file(cur, here_file);
        }
        for pi in to_apply {
            // retain a copy so `resolve_deferred_unknowns` can re-apply
            // with overwrite semantics after a cyclic `use` has finished
            // registering the partner file's definitions.
            self.applied_imports.push(pi.clone());
            self.data.import_operators(pi.lib_source, cur);
            match pi.spec {
                ImportSpec::Wildcard => {
                    self.data.import_all(pi.lib_source, cur, pi.public);
                }
                ImportSpec::Names(names) => {
                    // loft#1766 — the library may be suspended at its own `use` of this
                    // file, with nothing declared yet: ask again once it has.
                    let suspended = self.todo_files.iter().any(|(_, s)| *s == pi.lib_source);
                    for (name, bind) in &names {
                        if self
                            .data
                            .import_name(pi.lib_source, cur, name, bind, pi.public)
                        {
                            continue;
                        }
                        if suspended {
                            if self.first_pass {
                                self.deferred_import_misses.push((
                                    pi.lib_source,
                                    cur,
                                    name.clone(),
                                    bind.clone(),
                                    pi.public,
                                    *self.lexer.pos(),
                                ));
                            }
                        } else {
                            diagnostic!(
                                self.lexer,
                                Level::Error,
                                "Name '{name}' not found in library"
                            );
                        }
                    }
                }
            }
        }
    }

    /// Read a source file's content for the Tier-0 auto-`use` pre-scan.
    /// Honours the wasm VirtFS; an empty string on a read error (the scan then
    /// finds nothing and normal resolution proceeds unchanged).
    pub(super) fn read_source(filename: &str) -> String {
        #[cfg(feature = "wasm")]
        if let Some(c) = crate::wasm::virt_fs_get(filename) {
            return c;
        }
        crate::file_access::read_to_string(filename).unwrap_or_default()
    }

    /// Tier-1: build (once, cached) the `method name -> providing package` map
    /// from the current package's declared triggers — `[triggers] enabled` plus
    /// the text-methods derived from its source — and those of its
    /// trigger-enabled dependencies.  The package is located by walking up from
    /// `from_file` to the nearest `loft.toml`.
    pub(super) fn trigger_map(
        &mut self,
        from_file: &str,
    ) -> std::collections::HashMap<String, String> {
        if let Some(m) = &self.auto_use_trigger_map {
            return m.clone();
        }
        let mut map = std::collections::HashMap::new();
        let mut types = std::collections::HashMap::new();
        let mut builders = std::collections::HashMap::new();
        let mut dir = std::path::Path::new(from_file)
            .parent()
            .map(std::path::Path::to_path_buf);
        while let Some(d) = dir {
            let toml = d.join("loft.toml");
            if crate::file_access::exists(&toml) {
                Self::add_pkg_triggers(&toml, &d, &mut map, &mut types, &mut builders);
                if let Some(man) = crate::manifest::read_manifest(&toml.to_string_lossy()) {
                    for (dep, value) in &man.dependencies {
                        // PURE lookups only: a sibling package, a `path =` dependency, the
                        // project's `lib/<dep>`, or a registry dependency's EXTRACTED copy
                        // under the cache (the lock's pin, else the newest cached copy its
                        // range admits) — by path, never through `lib_path`, whose side
                        // effects queue every dependency of the manifest found above the
                        // file as a dependency of that file (a root manifest with registry
                        // packages loaded `graphics` into a one-line script and crashed the
                        // compiler in `imaging`).  The catalogue is the fallback for a
                        // package nothing declares, and it carries no TYPE triggers: a
                        // registry dependency read only through it left `names:
                        // sorted<Word[key]>` undefined while the `path =` form of the same
                        // package fired (@PLN179 finding 018's registry half).
                        let entry = self
                            .lib_path_manifest_resolve(&d.to_string_lossy(), dep)
                            .map(|r| r.entry)
                            .or_else(|| Self::dep_entry_by_path(&d, dep, value));
                        let root = match entry {
                            Some(entry) => std::path::Path::new(&entry)
                                .parent()
                                .and_then(|p| p.parent())
                                .map(std::path::Path::to_path_buf),
                            None => Self::registry_dep_root(&d, dep, value),
                        };
                        if let Some(root) = root {
                            Self::add_pkg_triggers(
                                &root.join("loft.toml"),
                                &root,
                                &mut map,
                                &mut types,
                                &mut builders,
                            );
                        }
                    }
                }
                break;
            }
            dir = d.parent().map(std::path::Path::to_path_buf);
        }
        self.auto_use_trigger_map = Some(map.clone());
        self.auto_use_type_map = types;
        self.auto_use_builder_map = builders;
        map
    }

    /// Add a package's derived triggers to the three maps — its text methods
    /// (`method -> package`), its public types and its declared builders — when the
    /// package opts in via `[triggers] enabled`.
    pub(super) fn add_pkg_triggers(
        toml: &std::path::Path,
        pkg_root: &std::path::Path,
        map: &mut std::collections::HashMap<String, String>,
        types: &mut std::collections::HashMap<String, String>,
        builders: &mut std::collections::HashMap<String, String>,
    ) {
        let Some(man) = crate::manifest::read_manifest(&toml.to_string_lossy()) else {
            return;
        };
        if !man.trigger_enabled {
            return;
        }
        let Some(name) = man.name else { return };
        let entry = man.entry.unwrap_or_else(|| format!("src/{name}.loft"));
        let src = crate::file_access::read_to_string(pkg_root.join(&entry)).unwrap_or_default();
        let derived = crate::triggers::derive_triggers(&src, &man.trigger_builders);
        for mt in derived.methods {
            map.entry(mt.name).or_insert_with(|| name.clone());
        }
        for t in derived.types {
            types.entry(t).or_insert_with(|| name.clone());
        }
        for b in derived.builders {
            builders.entry(b).or_insert_with(|| name.clone());
        }
    }

    /// Tier-1 lazy *catalog* fallback (`method -> package`), read once from the
    /// registry's derived trigger sidecar.  This is what makes `line.matches(p)`
    /// work against a package the user has NOT declared as a dependency: the local
    /// `trigger_map` misses, so we look the method up across the whole catalog,
    /// get the package name, and hand it to `lib_path` — which resolves it via
    /// the same lockfile → installed → auto-install chain an explicit `use`
    /// would take.  Ambiguity is dropped by `trigger_providers` (registry
    /// submission already rejects true collisions).  Cached (empty on absent
    /// catalog) so the lookup happens at most once per parse.
    ///
    /// Reads [`registry_index::catalog_trigger_map`](crate::registry_index::catalog_trigger_map)
    /// rather than the index itself: the map is a few hundred bytes, the index is
    /// the whole published catalog, and parsing the second to answer the first put
    /// the entire registry's growth on the compile path.
    #[cfg(feature = "registry")]
    pub(super) fn catalog_trigger_map(&mut self) -> std::collections::HashMap<String, String> {
        if let Some(m) = &self.auto_use_catalog_map {
            return m.clone();
        }
        let map: std::collections::HashMap<String, String> =
            crate::registry_index::catalog_trigger_map()
                .into_iter()
                .collect();
        self.auto_use_catalog_map = Some(map.clone());
        map
    }

    /// No-op when the registry feature is off — there is no cached catalog to
    /// consult, so Tier-1 resolves only via declared dependencies.
    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    pub(super) fn catalog_trigger_map(&mut self) -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
    }

    /// The builder half of the catalog fallback (`function -> package`), from the same
    /// sidecar as [`Self::catalog_trigger_map`]: what lets a bare `set(words)` reach a
    /// package the user has not declared.
    #[cfg(feature = "registry")]
    pub(super) fn catalog_builder_map(&mut self) -> std::collections::HashMap<String, String> {
        if let Some(m) = &self.auto_use_catalog_builders {
            return m.clone();
        }
        let map: std::collections::HashMap<String, String> =
            crate::registry_index::catalog_builder_map()
                .into_iter()
                .collect();
        self.auto_use_catalog_builders = Some(map.clone());
        map
    }

    #[cfg(not(feature = "registry"))]
    #[allow(clippy::unused_self)]
    pub(super) fn catalog_builder_map(&mut self) -> std::collections::HashMap<String, String> {
        std::collections::HashMap::new()
    }

    /// Is `f` the source file this parser is currently parsing?
    ///
    /// Compared canonically, because the two paths reach here by different routes —
    /// the lexer's is whatever the caller passed, the candidate's is built from a
    /// probe directory — so `src/x.loft` and an absolute form must compare equal.
    pub(super) fn is_current_source(&self, f: &str) -> bool {
        let canon = |p: &str| crate::file_access::plain_canonical(std::path::Path::new(p));
        let cur = self.lexer.pos().file;
        !cur.is_empty() && canon(cur.as_str()) == canon(f)
    }

    /// The optional import spec after a `use` target: `::*`, a single `::name [as bind]`,
    /// or the grouped `::(a [as x], b, …)`.  `None` when no `::` follows.
    ///
    /// `path` is only for the diagnostic, so it reads back the spelling the author used
    /// (`lib` or `self::module`).
    ///
    /// One grammar for both spellings (loft#949): `use self::<m>::(a, b)` has to accept
    /// exactly what `use <lib>::(a, b)` accepts, and two copies of this loop would drift.
    ///
    /// @PLN22 Phase 4 — multiple names MUST be grouped in parentheses; the flat
    /// top-level comma list (`use lib::a, b`) is dropped (it read poorly — `b` didn't
    /// visually bind to `lib::`).  The names are still bound on that error so the rest
    /// of the file parses cleanly.
    pub(super) fn parse_import_spec(&mut self, path: &str) -> Option<ImportSpec> {
        if !self.lexer.has_token("::") {
            return None;
        }
        if self.lexer.has_token("*") {
            return Some(ImportSpec::Wildcard);
        }
        let grouped = self.lexer.has_token("(");
        let mut names = Vec::new();
        while let Some(name) = self.lexer.has_identifier() {
            // @PLN22 Phase 3 — `Name as Alias` binds the imported name under the bare
            // alias (works inside `(…)` too).
            let bind = if self.lexer.has_token("as") {
                self.lexer.has_identifier().unwrap_or_else(|| name.clone())
            } else {
                name.clone()
            };
            names.push((name, bind));
            if !self.lexer.has_token(",") {
                break;
            }
        }
        if grouped {
            self.lexer.token(")");
        } else if names.len() > 1 {
            // @C76 — imported names are grouped with `()`; a flat comma list is refused.
            diagnostic!(
                self.lexer,
                Level::Error,
                "import multiple names with parentheses: `use {path}::(a, b, …)`"
            );
        }
        if names.is_empty() {
            diagnostic!(
                self.lexer,
                Level::Error,
                "Expected name, '*', or '(' after '::'"
            );
            None
        } else {
            Some(ImportSpec::Names(names))
        }
    }

    /// `use self::<module> [as <alias>] [::<spec>];` — bind the CURRENT package's own
    /// module (loft#949).
    ///
    /// A module's short name is one slot shared by the whole dependency graph, so a
    /// package's own `use catalogue;` binds whichever `catalogue.loft` was registered
    /// first — and building a package reads every file under its `src/`, so a consumer
    /// that merely ADDS `src/catalogue.loft` can take the name before its dependency
    /// asks for it.  The dependency then calls the consumer's function: a published,
    /// versioned, tested package answering differently because of a file downstream.
    ///
    /// `self::` is the spelling that cannot be captured.  It registers the module under
    /// `<package>::<module>`, taken from `[package] name`, so two packages' `catalogue`
    /// modules occupy two different slots and both stay reachable — which is the half
    /// "prefer the local file" could not provide, since the flat `use_names` map has
    /// room for only one `catalogue` however precedence is decided.
    ///
    /// Bare `use <id>;` is untouched, so this is additive: nothing that builds today
    /// changes, and a package opts in where it wants the guarantee.
    ///
    /// Mirrors the bare-`use` protocol exactly, including its two-encounter shape: the
    /// first encounter loads the file and switches the lexer onto it, and the import is
    /// recorded on the second, when this file is re-parsed off `todo_files` and the key
    /// already exists.
    /// Consume the keyword of the next `use` statement, `use` or `pub use`, recording
    /// which in `use_public`.  A `pub` that turns out to open a definition is put back.
    /// A `pub` that does not open a `pub use` opens the file's first definition; it is
    /// left in `pub_taken` for the definitions loop rather than put back, because putting
    /// a token back costs a lexer link on every file (the stdlib's included).
    pub(super) fn use_keyword(&mut self) -> bool {
        self.use_public = self.lexer.has_token("pub");
        if self.lexer.has_token("use") {
            return true;
        }
        if self.use_public {
            self.pub_taken = true;
            self.use_public = false;
        }
        false
    }

    /// `pub use lib;` would pass nothing on: a bare `use` binds only the `lib::`
    /// qualifier, and a qualifier is not a name an importer can receive (@C98).
    pub(super) fn refuse_bare_pub_use(&mut self, target: &str) {
        if self.use_public {
            diagnostic!(
                self.lexer,
                Level::Error,
                "`pub use {target};` passes nothing on — a bare `use` binds only the \
                 `{target}::` qualifier.  Write `pub use {target}::*;` to pass on all its \
                 names, or `pub use {target}::(…);` for some"
            );
        }
    }

    pub(super) fn parse_use_self(&mut self) {
        if !self.lexer.has_token("::") {
            diagnostic!(
                self.lexer,
                Level::Error,
                "`use self` must name a module — write `use self::<module>;`"
            );
            self.lexer.has_token(";");
            return;
        }
        let Some(module) = self.lexer.has_identifier() else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "expected a module name after `use self::`"
            );
            self.lexer.has_token(";");
            return;
        };
        let alias = if self.lexer.has_token("as") {
            self.lexer.has_identifier()
        } else {
            None
        };
        let spec = self.parse_import_spec(&format!("self::{module}"));
        if spec.is_none() && self.use_public {
            self.refuse_bare_pub_use(&format!("self::{module}"));
        } else if spec.is_none() && alias.is_none() {
            // A bare `use` binds only a qualifier (@C98), and a package's own module has
            // none (loft#976: the short qualifier is one slot shared by every package), so
            // this form would bind nothing at all.
            diagnostic!(
                self.lexer,
                Level::Error,
                "`use self::{module};` binds nothing — a bare `use` brings in only a qualifier, \
                 and a package's own module has none.  Write `use self::{module}::*;` for its \
                 names, `use self::{module}::(…);` for some, or `use self::{module} as <alias>;` \
                 for a qualifier"
            );
        }
        let Some(pkg) = self.own_package_name() else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "`use self::{module}` needs a package: no `loft.toml` declaring \
                 `[package] name` was found above this file. Add one, or write \
                 `use {module};` to take the module by its shared name"
            );
            self.lexer.has_token(";");
            return;
        };
        // No fall-through to the wider `lib_path` search when the module is absent.
        // Bare `use` searches outward by design; `self::` must not, or a typo silently
        // binds some other package's file — the exact outcome it exists to prevent.
        let Some(f) = self.own_module_path(&module) else {
            diagnostic!(
                self.lexer,
                Level::Error,
                "package '{pkg}' has no module '{module}' — expected \
                 '{module}.loft' beside this file (or in its `lib/`)"
            );
            self.lexer.has_token(";");
            return;
        };
        self.bind_own_module(&module, &pkg, &f, alias.as_ref(), spec, "self::");
    }

    /// Bind `<pkg>::<module>` to this package's own `<module>.loft` at `f`.
    ///
    /// The body of `use self::<module>`, and — since loft#976 — of a BARE `use
    /// <module>` inside a package that ships one, which is the same request written
    /// the short way.  Keyed by `<pkg>::<module>` rather than the bare name, so no
    /// other package in the graph can take the name from under it.
    pub(super) fn bind_own_module(
        &mut self,
        module: &str,
        pkg: &str,
        f: &str,
        alias: Option<&String>,
        spec: Option<ImportSpec>,
        spelling: &str,
    ) {
        // The SHORT spelling (`use math;`) has always also supplied the `math::` qualifier,
        // and code inside the package writes it. Scoping the module must not take that
        // away, so the bare name is registered as an alias onto the same source. It is a
        // flat map, so two packages both spelling it short still share this ONE qualifier
        // — unchanged from before, and now the only thing they share: each package's own
        // module, and every bare name it exports, is its own.
        let bare_qualifier = spelling.is_empty();
        let bare_use = spec.is_none() && alias.is_none();
        let key = format!("{pkg}::{module}");
        // loft#1080 — the module may already be loaded under a DIFFERENT name.  This key
        // is deliberately `<pkg>::<module>` so no other package can take the name from
        // under it, and that is exactly what lets one FILE arrive here twice: an entry
        // script OUTSIDE the package `use`s the module by its bare name and loads it
        // flat, then a file INSIDE the package `use`s the same module, computes the
        // qualified key, finds it absent — and parses the same file into a SECOND source.
        //
        // Nothing downstream survives that.  `use_alias` below then rebinds the bare
        // name to the second source, so the first source's definitions are unreachable
        // but still present, and native codegen emits every one of them a second time:
        // `disambiguated_fn_ident` distinguishes same-named functions by hashing their
        // defining FILE, on the stated ground that "two same-named fns can only come
        // from different files" — which is precisely the assumption a second parse of
        // one file breaks.  The generated cdylib then failed to compile with 55 ×
        // `error[E0428]: the name `loft_shared_n_…_mafed3b7f` is defined multiple times`,
        // and each pair carried an IDENTICAL hash, which is the tell that it is one file
        // twice rather than #305's two different files.
        //
        // `use_paths` (loft#912) already records the canonical file behind every loaded
        // name for exactly this question — "is this the same module" — so ask it, and
        // take the same already-loaded path a repeated `use` of the qualified key takes:
        // alias the key onto the source that exists and do not read the file again.
        let existing = if self.data.use_exists(&key) {
            Some(self.data.get_source(&key))
        } else {
            self.source_loaded_from(f)
        };
        if let Some(lib_source) = existing {
            if bare_use {
                self.data
                    .record_bare_use(self.data.source, module, lib_source);
            }
            if !self.data.use_exists(&key) {
                self.data.use_alias(&key, lib_source);
                self.record_use_path(&key, f);
            }
            if let Some(a) = alias {
                self.data.use_alias(a, lib_source);
            } else if bare_qualifier {
                self.data.use_alias(module, lib_source);
            }
            // Same rule as a bare `use`: only an explicit `::` spec brings names in
            // unqualified; without one the module is reached through its qualifier
            // (@C98).
            let import_spec = spec.or(Some(ImportSpec::Names(Vec::new())));
            if let Some(import_spec) = import_spec {
                self.pending_imports.push(PendingImport {
                    for_source: self.data.source,
                    lib_source,
                    spec: import_spec,
                    public: self.use_public,
                });
            }
            if !self.lexer.has_token(";") {
                diagnostic!(
                    self.lexer,
                    Level::Error,
                    "Missing ';' after 'use {spelling}{module}' — use statements must end with a semicolon"
                );
            }
            return;
        }
        let cur = self.lexer.pos().file.to_string();
        let into = self.data.source;
        self.todo_files.push((cur, into));
        self.data.use_add(&key);
        if bare_use {
            self.data.record_bare_use(into, module, self.data.source);
        }
        self.record_use_path(&key, f);
        if let Some(a) = alias {
            self.data.use_alias(a, self.data.source);
        } else if bare_qualifier {
            self.data.use_alias(module, self.data.source);
        }
        self.switch_to_dep(f);
    }

    /// Remember which FILE a `use` short-name was loaded from (loft#912).
    ///
    /// Called at every site that pairs `Data::use_add` with a resolved path, so the
    /// name→file mapping cannot drift from the load it describes.  Paths are
    /// canonicalised, because the same file is reached by different spellings (a
    /// relative `../pkg_dep/src/x.loft` here, an absolute one there) and only the
    /// canonical form answers "is this the same module".
    /// The source a file is ALREADY loaded into, or `None` if it has not been read.
    ///
    /// The reverse of [`Self::record_use_path`], over the same canonical paths, and the
    /// answer to "is this the same module" that a name-keyed `use_exists` cannot give:
    /// one file is reachable under more than one name (a bare `use grid;` from outside a
    /// package and the `<pkg>::grid` key from inside it), and parsing it twice puts two
    /// copies of every definition in `Data` (loft#1080).
    /// Is `cur_script` a file inside the registry cache — a package someone else
    /// published, whose manifest is not this project's to fix?  Three diagnostics ask it,
    /// each behind the `registry` feature, as is the cache directory it reads.
    #[cfg(feature = "registry")]
    pub(super) fn script_in_registry_cache(cur_script: &str) -> bool {
        crate::file_access::is_under_canonical(
            crate::file_access::PathText::host(cur_script),
            crate::file_access::PathText::from_os(&crate::registry_index::cache_dir()),
        )
    }

    pub(super) fn record_use_path(&mut self, id: &str, f: &str) {
        let canonical = crate::file_access::plain_canonical_str(f);
        self.use_paths.insert(id.to_string(), canonical);
    }

    /// The file the CURRENT file means by `use <id>`, when that is a module of its
    /// own package — `<its dir>/<id>.loft` or `<its dir>/lib/<id>.loft`.
    ///
    /// `None` when no such file exists, or when `id` is a declared dependency of the
    /// enclosing package: a dependency name always wins over a same-named file inside
    /// the declaring package, which is the guard `lib_path` already applies.
    pub(super) fn own_module_file(&mut self, id: &str) -> Option<String> {
        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        let cur_dir = script_dir(&cur_script).to_string();
        if cur_dir.is_empty() {
            return None;
        }
        if self
            .package_declared_deps(&cur_dir)
            .is_some_and(|(_, deps)| deps.contains(id))
        {
            return None;
        }
        self.own_module_path(id)
    }

    /// Where a module of the CURRENT file's own package would live — `<its
    /// dir>/<id>.loft` or `<its dir>/lib/<id>.loft` — with no view on whether
    /// something else should win the name.
    ///
    /// Split out from [`own_module_file`] because `use self::<id>` (loft#949) wants
    /// exactly this probe and NOT its dependency guard: that guard exists so a bare
    /// `use <id>` prefers a declared dependency over a same-named local file, and
    /// `self::` is the spelling that says the author means the local one.
    pub(super) fn own_module_path(&self, id: &str) -> Option<String> {
        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        let cur_dir = script_dir(&cur_script).to_string();
        if cur_dir.is_empty() {
            return None;
        }
        let sep = sep_str();
        [
            format!("{cur_dir}{sep}{id}.loft"),
            format!("{cur_dir}{sep}lib{sep}{id}.loft"),
        ]
        .into_iter()
        .find(|c| crate::file_access::exists(c))
    }

    /// The `[package] name` of the package the current file belongs to (loft#949).
    ///
    /// This is the qualifier `use self::<module>` registers under, and it has to be
    /// the manifest NAME rather than the package's path: the key reaches a database
    /// type key, which must be identical on every machine that reads the store.  The
    /// registry already enforces that names are unique, so `<name>::<module>` is a
    /// qualifier nothing else in the graph can spell by accident.
    /// Does the package above this file DECLARE `id` as a dependency?
    ///
    /// A declared dependency beats a local file of the same name — the shadow guard in
    /// `lib_path` settles that on purpose — so the own-module preference (loft#976) must
    /// defer to it, or a package holding `src/<dep>.loft` would stop being able to reach
    /// the `<dep>` it depends on.
    pub(super) fn package_declares_dep(&mut self, id: &str) -> bool {
        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        let cur_dir = script_dir(&cur_script).to_string();
        self.package_declared_deps(&cur_dir)
            .is_some_and(|(_, deps)| deps.contains(id))
    }

    pub(super) fn own_package_name(&mut self) -> Option<String> {
        let cur_script = self.lexer.pos().file.replace(other_sep(), sep_str());
        let cur_dir = script_dir(&cur_script).to_string();
        let (root, _) = self.package_declared_deps(&cur_dir)?;
        let manifest = std::path::Path::new(&root).join("loft.toml");
        crate::manifest::read_manifest(&manifest.to_string_lossy())?.name
    }

    /// Report when `use <id>` here names a DIFFERENT file from the one already loaded
    /// under that name, so the import below binds the other package's module.
    ///
    /// The module namespace is flat across the dependency graph while the files are
    /// not.  Adding `src/catalogue.loft` to a package whose dependency already has one
    /// leaves only the first loadable, and the loser's functions are then simply
    /// absent — reported as `Unknown function part_list` at a line inside a package
    /// the consumer never edited and cannot fix, with nothing naming a collision
    /// (loft#912).  This says the collision out loud, and names both files.
    ///
    /// **The cure it names is `use self::<id>`, not a rename** (loft#949).  That
    /// spelling binds the local file under `<package>::<id>`, so both modules stay
    /// reachable and no downstream file can take the name — where a rename churns a
    /// file plus every `use` of it and leaves the next collision just as possible.
    /// This diagnostic is the signpost for that opt-in, so it has to say its name;
    /// the opt-in landing without the signpost naming it is what left "a reason to
    /// know you must" missing.  Keyed on whether the file HAS a package, because
    /// `self::` qualifies with `[package] name` and is refused without one — so a
    /// bare script hears the rename instead of a cure that would error.
    ///
    /// **Never an error, and the resolution is unchanged** — deliberately, on a
    /// measurement.  A hard refusal breaks code that builds today: `graphics` ≤ 0.4.2
    /// and `mesh3d` both ship `math` / `mesh` / `scene`, and this repo's own
    /// `tests/fixtures/libs/graphics` depends on the registry `mesh3d` while carrying
    /// its own copies of all three.  Scoping module names to their package is the fix
    /// this is a signpost for; it waits on two things this cannot reach — `use_add`
    /// deriving a new source id from the SIZE of the name map, and
    /// `qualified_type_name` deriving a DATABASE type key from a module's short name
    /// (a package-qualified key must stay machine-independent, so it cannot carry the
    /// package's path).
    ///
    /// **"Bind the local file first" is not the cheap way round that (loft#949).**  It
    /// reads as a change nobody would notice, and the corpus says otherwise: across 65
    /// packages, four module basenames are shipped by two DIFFERENT packages (`glb`,
    /// `math`, `mesh`, `scene`), and **19 sites in shipped library code** — plus 16 in
    /// tests — are a package `use`-ing a name it also ships.  Every one is the
    /// `graphics` ⟷ `mesh3d` overlap above.  Local-first would rebind all of them, and
    /// it does not avoid the blockers either: two modules called `math` then have to
    /// coexist, which is exactly what the flat `use_names` map cannot express.  So the
    /// design owes an answer for coexistence, not just for precedence.
    pub(super) fn module_name_clash(&mut self, id: &str) {
        let Some(own) = self.own_module_file(id) else {
            return;
        };
        let own_canonical = crate::file_access::plain_canonical_str(&own);
        let Some(loaded) = self.use_paths.get(id) else {
            return;
        };
        if *loaded == own_canonical {
            return;
        }
        let loaded = loaded.clone();
        // Both files in ONE package is not the collision this is about.  The text says it —
        // "shared across the whole dependency graph" — and the case that costs a person a
        // diagnosis is a CONSUMER's new file shadowing a DEPENDENCY's.  Two files inside one
        // package are the author's own, the `use` binds the one they meant, and nothing
        // breaks: `tests/<pkg>.loft` beside `src/<pkg>.loft` is an ordinary layout that two
        // of this repo's own fixtures use, and telling that author to rename a file is
        // advice they should ignore (loft#948).  A diagnostic that fires where there is
        // nothing to fix teaches people to skip the ones where there is.
        if Self::same_package(&own_canonical, &loaded) {
            return;
        }
        if !self.module_clash_reported.insert(id.to_string()) {
            return;
        }
        // loft#949 — WHICH WAY the capture runs decides both the tier and who is being
        // told.  Two shapes reach this line:
        //
        //   (a) a package `use`s a name that another package in the graph also ships and
        //       loaded first.  Both authors can see it in their own builds, and it is
        //       what the corpus is full of — `graphics` ⟷ `mesh3d` over `math` / `mesh` /
        //       `scene`, 19 sites in shipped library code.  Advice.
        //
        //   (b) the ROOT PROJECT added a file whose basename a DEPENDENCY was already
        //       using, so the dependency's own module never loads and its `use` binds the
        //       consumer's file.  The dependency is unchanged, published, and green on its
        //       own; its answer changes because something downstream added a file.  That
        //       is a wrong result — the filed case computed 100 where the package alone
        //       computes 42 — and by the tier rule a diagnostic gates exactly when
        //       ignoring it can produce one.  Warning.
        //
        // The split matters because advice is routinely filtered: the reporting consumer's
        // harness pipes through `grep -viE "^  Warning"` and its build scripts drop
        // `^advice`, so the only sign of a changed answer was being discarded by design.
        // Keeping (a) at advice is what lets (b) gate without failing the CI of every
        // library that ships an overlapping basename today.
        let root_project = crate::resolution_scope::project_root(&self.database.source_dir);
        let inside = |file: &str, root: &std::path::Path| {
            crate::file_access::plain_canonical(std::path::Path::new(file)).starts_with(root)
        };
        let captured_by_root = root_project
            .as_ref()
            .is_some_and(|root| inside(&loaded, root) && !inside(&own_canonical, root));
        if captured_by_root {
            // The reader here is the CONSUMER, and the file they can edit is their own —
            // they cannot add a `use self::` to a dependency they did not write.  Name the
            // durable cure too, because it is the one that ends the class for everyone
            // downstream of that package.
            diagnostic!(
                self.lexer,
                Level::Warning,
                code = "module-name-shadowed",
                "this project's '{loaded}' captured the module name '{id}', which \
                 dependency module '{own_canonical}' was already using — a module's file \
                 name is shared across the whole dependency graph, so that dependency's \
                 own '{id}' never loads and this `use` binds the project's file instead. \
                 The dependency now answers differently than it does on its own. Rename \
                 this project's '{id}.loft'; the dependency's author can end it for every \
                 consumer by writing `use self::{id}`"
            );
            return;
        }
        // Name the cure that KEEPS this package's answer, not just the one that
        // renames around the collision (loft#949).  `use self::<id>` binds the local
        // file under `<package>::<id>`, so both modules stay reachable and no
        // downstream file can capture the name — but it needs a `[package] name` to
        // qualify with, and without one it is refused.  A bare script therefore hears
        // only the cure that works for it.
        let cure = match self.own_package_name() {
            Some(_) => format!("Write `use self::{id}` to bind this package's own file"),
            None => "Rename one file and its `use` if you meant the other".to_string(),
        };
        diagnostic!(
            self.lexer,
            Level::Advice,
            code = "module-name-shadowed",
            "module '{id}' is declared by two files — '{own_canonical}' and '{loaded}' — \
             and a module's file name is shared across the whole dependency graph, so \
             this `use` binds the second one. {cure}"
        );
    }

    /// loft#788 — refuse a bare name that two imports both bind, naming both
    /// packages and what to write instead.
    ///
    /// The binding still happens (first import wins), so this is the ONLY thing
    /// standing between a consumer and a source line whose meaning depends on
    /// the order of the `use` block above it. Both orders compile for a function
    /// or a constant, and they answer differently — the struct case merely
    /// happens to error on a field the author never named.
    ///
    /// Reported at the USE and not at the `use`: two packages may share a name
    /// the program never writes bare, and that program is well-defined and
    /// compiles today (COMPATIBILITY.md — no functioning program breaks).
    ///
    /// Once per NAME, in whichever pass reaches it first. Not pass-2-only: a
    /// first-wins binding to the wrong package's struct raises "Unknown field"
    /// during pass 1 and the parse stops there, so a pass-2 check stays silent
    /// in the case it explains.
    pub(crate) fn refuse_ambiguous_import(&mut self, name: &str, winner: u32) {
        if winner == u32::MAX || self.ambiguity_reported.contains(name) {
            return;
        }
        // loft#826 — an unresolved forward-reference stub is not a declaration.
        //
        // A file that names a type its importer declares registers a `Unknown`
        // stub in its OWN source, and that stub is pub-visible, so it imports
        // back into the importer like any other public name.  With two such
        // files the second stub lands on the first and is recorded here as a
        // rival — and the message then told the author to write `helper::Thing`
        // or `second::Thing` when neither file declares `Thing` at all and the
        // one that does was the line above.  A qualification naming a package
        // that has no such type cannot resolve, so following the advice was a
        // dead end in both directions.
        //
        // Reported at USE (see above), so by the time this runs every stub that
        // was going to resolve has: one still `Unknown` here declares nothing
        // and never will.  Dropping them can only remove a claim that was
        // false; a genuine two-package collision has real defs on both sides.
        //
        // The WINNER can be such a stub too (loft#1766): in pass 1 a use can be reached
        // before `resolve_deferred_unknowns` has run, and a glob import of a file that
        // names a type from its mutual-`use` partner carries that file's stub in first.
        // So count real declarations on both sides; fewer than two is no collision.
        // Asked at every use of an imported name: no rival is the common answer, and it
        // allocates nothing.
        if self.data.ambiguous_with(name).is_empty() {
            return;
        }
        let is_stub = |d: u32| matches!(self.data.def(d).def_type(), DefType::Unknown);
        let candidates = || {
            std::iter::once(winner)
                .chain(self.data.ambiguous_with(name).iter().copied())
                .filter(|&d| !is_stub(d))
        };
        // Counted before anything is collected: a rival that is only a stub is the common
        // case under a mutual `use`, and it must not cost an allocation per use.
        let mut seen = candidates();
        let Some(first) = seen.next() else {
            return;
        };
        if !seen.any(|d| d != first) {
            return;
        }
        let mut real: Vec<u32> = candidates().collect();
        real.dedup();
        let winner = real[0];
        let others: Vec<u32> = real[1..].to_vec();
        self.ambiguity_reported.insert(name.to_string());
        // `n_` is the storage spelling of a function, never the source spelling —
        // a message telling someone to write `pkg::n_shared` names something
        // they cannot type.
        let mut spellings: Vec<String> = std::iter::once(winner)
            .chain(others.iter().copied())
            .map(|d| self.data.qualified_type_name(d).replace("::n_", "::"))
            .collect();
        spellings.sort();
        spellings.dedup();
        let bare = name.strip_prefix("n_").unwrap_or(name);
        // loft#949 — a module taken with `use self::<m>` registers under the
        // package-qualified key `<pkg>::<m>`, so its spelling here reads
        // `pkg::mod::name`: THREE segments, which the expression parser does not
        // accept.  Suggesting it would repeat the mistake the comment above this
        // function records — a cure that cannot be typed is a dead end in both
        // directions — so a scoped rival names the two cures that do work.
        let scoped: Vec<&String> = spellings
            .iter()
            .filter(|s| s.matches("::").count() > 1)
            .collect();
        if scoped.is_empty() {
            diagnostic!(
                self.lexer,
                Level::Error,
                "`{bare}` is declared by more than one package here — write {} to say which",
                spellings.join(" or ")
            );
            return;
        }
        // The module name out of `<pkg>::<module>::<name>`, for the alias example.
        let module = scoped
            .first()
            .and_then(|s| s.split("::").nth(1))
            .unwrap_or("that module");
        diagnostic!(
            self.lexer,
            Level::Error,
            "`{bare}` is declared by more than one module here — {}. A package's own \
             module is scoped to it and has no bare qualifier, so nothing here picks \
             between them: alias yours (`use self::{module} as m;` then `m::{bare}`), \
             or import the other one by name so only one `{bare}` is in scope",
            spellings.join(" and ")
        );
    }

    /// loft#826 — the message for a name that is declared, but by the file that
    /// `use`d this one.
    ///
    /// `use helper;` imports helper's public names into the importer.  It does
    /// not import the importer's names into helper, and helper is parsed BEFORE
    /// the importer reaches its own definitions — so the name genuinely is not
    /// there.  Nothing in "Unknown function make — did you mean 'move'?" says
    /// so, and the author is looking straight at `make` two files away.
    ///
    /// Returns `None` when no importer declares it, so every caller keeps its
    /// existing message for the ordinary case.  The cure is the one that
    /// actually compiles: a third file both `use`, verified end to end by
    /// `tests/package_layout.rs::shared_sibling_carries_types_and_functions`.
    pub(crate) fn importer_boundary_note(&self, storage_name: &str) -> Option<String> {
        let d_nr = self.data.declared_by_importer(storage_name)?;
        let bare = storage_name.strip_prefix("n_").unwrap_or(storage_name);
        let pos = self.data.def(d_nr).position();
        let file = crate::file_access::name_of(pos.file.as_str());
        // When this file ALSO has a bare `use` of that file, the two files already `use`
        // each other, and a mutual import resolves both ways (the p173 cycle): the cure is
        // to import the name, `use errand::*;` or `use errand::(Errand);`, with no file moved.
        let def_source = self.data.def(d_nr).source;
        if let Some(q) = self.data.bare_use_qualifier(self.data.source, def_source) {
            return Some(format!(
                "`{bare}` is declared in {file}:{}, which this file `use`s bare — a bare \
                 `use {q};` brings in only the `{q}::` qualifier.  Import the name: \
                 `use {q}::*;` or `use {q}::({bare});`.",
                pos.line
            ));
        }
        Some(format!(
            "`{bare}` is declared in {file}:{}, which `use`s this file — a `use` \
             imports the used file's names into the file that used it, never the \
             other way round.  Move `{bare}` into a file that BOTH `use`, and \
             `use` it here.",
            pos.line
        ))
    }
}
