// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution

//! `loft.toml` manifest reader for external package support (T2-11).
//!
//! Tokenised with the SAME [`crate::lexer::Lexer`] loft source uses — via a
//! config lexicon ([`LexConfig::config`]) that treats `#` as the comment marker
//! and disables `{…}` string interpolation — so there is no second lexer in the
//! codebase.  A small walk over the token stream fills the [`Manifest`] fields.

use crate::lexer::{LexConfig, LexItem, Lexer};

/// @PLN100 Slice 2 — one declared build target from `[build.target.<name>]`: a
/// named (shape × triple × feature-set) the `loft build` driver can produce, with
/// the toolchain it `requires`.  Overlays a built-in target of the same name
/// (`native` / `html` / `wasi`) or adds a new one; empty fields keep the built-in
/// default (`build_phase::resolve_target`).
#[derive(Debug, Default, Clone)]
pub struct BuildTarget {
    /// The target's name — the key in `[build.target.<name>]` and what
    /// `loft build <name>` / `default-targets` refer to.
    pub name: String,
    /// The compile shape that names the driver action: `native` (→ `--native`),
    /// `html` (→ `--html`), or `wasi` (→ `--native-wasm`).  `None` → defaults to
    /// the built-in of the same name, else `native`.
    pub shape: Option<String>,
    /// The rustc/rustup target triple (informational + the default entry of
    /// `requires.rust-targets`).
    pub triple: Option<String>,
    /// Cargo features the shape's runtime rlib is built with (`[…] features = [...]`).
    pub features: Vec<String>,
    /// `[build.target.<name>.requires] rust-targets = [...]` — rustup targets that
    /// must be installed for this target to build.
    pub requires_rust_targets: Vec<String>,
    /// `[build.target.<name>.requires] tools = [...]` — external tools that must be
    /// on `PATH` (e.g. `wasm-opt`).
    pub requires_tools: Vec<String>,
}

/// @PLN100 Slice 3 — one `[[build.asset]]` step: a custom command that turns
/// declared `inputs` into `outputs`, re-run only when stale.  Staleness =
/// `outputs missing` OR `inputs content changed` OR (for an external-source asset)
/// the output is older than `lifetime` (a freshness TTL).  See
/// `build_phase::run_asset`.
#[derive(Debug, Default, Clone)]
pub struct BuildAsset {
    /// The asset's name (`name = "atlas"`) — used in progress output and the
    /// fingerprint sidecar filename.
    pub name: Option<String>,
    /// The command to (re)build the asset (`run = "scripts/pack_atlas.loft"`): a
    /// single `.loft` script (run with this loft binary) or any shell command.
    pub run: Option<String>,
    /// Input path globs (`inputs = ["art/**/*.png"]`) — content-fingerprinted so
    /// the step re-runs only when an input changes.  Empty for an
    /// external-source asset (then `lifetime` drives rebuilds).
    pub inputs: Vec<String>,
    /// The files this step produces (`outputs = ["assets/atlas.bin"]`) — a missing
    /// output forces a rebuild.
    pub outputs: Vec<String>,
    /// The build targets this asset is needed for (`targets = ["html"]`).  Empty →
    /// the asset applies to every build.
    pub targets: Vec<String>,
    /// A freshness TTL (`lifetime = "30d"`) for an externally-sourced output:
    /// rebuild when the output is older than this even if no input changed.  Units
    /// span `s`/`m`/`h`/`d`/`w`/`mo`/`y` (`build_phase::parse_duration`).
    pub lifetime: Option<String>,
}

/// @PLN100 Slice 4 — one `[[test]]` entry: a declared test script run over one or
/// more execution-backend `targets` after the build + asset phases, gated on the
/// asset outputs it `needs`.  A green run is cached by the (run-script + inputs +
/// target) fingerprint so `loft check` is incremental.  See
/// `build_phase::run_test_phase`.
#[derive(Debug, Default, Clone)]
pub struct BuildTest {
    /// The test's name (`name = "smoke"`) — used in output and the cache stamp.
    pub name: Option<String>,
    /// The `.loft` test script to run (`run = "tests/smoke.loft"`).
    pub run: Option<String>,
    /// The execution backends to run the script through (`targets = ["interpret",
    /// "native"]`).  Empty → `["interpret"]`.  `html` / `wasi` have no headless
    /// runner yet and are reported as skipped.
    pub targets: Vec<String>,
    /// Asset names that must be built first (`needs = ["atlas"]`) — the test is
    /// skipped (and the gate fails) if a needed asset's outputs are missing.
    pub needs: Vec<String>,
    /// Data files the test reads (`inputs = ["assets/atlas.bin"]`) — folded into
    /// the cache key so the test re-runs when a data file it reads changes.
    pub inputs: Vec<String>,
}

/// @PLN146 F5 — one `[[font]]` declaration: a font family the program draws with,
/// and where each target gets it from.
///
/// The declaration exists because the browser cannot be handed a font file the way
/// the desktop backend can: `gl_load_font("fonts/Foo.ttf")` opens no file under
/// `--html`, it resolves the path's base name to a CSS family. So the page has to
/// carry the family already, and the name it carries must be the name the program
/// asks for — drift between the two is a silent fallback, never an error
/// (`plans/146-content-delivery/FONTS.md`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FontDecl {
    /// `family = "PressStart2P"` — the CSS family name AND the key the program
    /// looks the font up by. Required.
    pub family: Option<String>,
    /// `native = "fonts/PressStart2P.ttf"` — the file the program passes to
    /// `gl_load_font`, used by every target except the browser. Its base name must
    /// equal `family`, because that base name is what the browser resolves.
    pub native: Option<String>,
    /// `url = "fonts/PressStart2P.woff2"` — a font FILE this page fetches, emitted
    /// as an `@font-face` rule. Our own file server, or a packed asset served by
    /// range like every other one.
    pub url: Option<String>,
    /// `stylesheet = "https://fonts.googleapis.com/css2?family=Press+Start+2P"` —
    /// a provider's stylesheet, emitted as a `<link>`. Zero bytes of ours.
    pub stylesheet: Option<String>,
}

/// @PLN146 F4 — one `[[embed]]` declaration: a file the `--html` page carries in
/// its own filesystem, so a program reads it with the same call on every target.
///
/// The pipeline for assets is a store on a file server, range-read
/// (`plans/146-content-delivery/ASSETS.md`); this is the exception that doc names —
/// the bytes a page needs before its first fetch, and a gallery page that has to be
/// one self-contained file.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EmbedDecl {
    /// `path = "assets/game.pack"` — the path the PROGRAM passes, and therefore the
    /// key the page carries the file under. Relative and in normal form: the page
    /// resolves it against `/`, so `/assets/game.pack` is what `store_load(q,
    /// "assets/game.pack")` asks for. Required.
    pub path: Option<String>,
    /// `source = "build/game.pack"` — where the bytes are on the BUILD box,
    /// relative to the declaring `loft.toml`. Defaults to `path`. A pack is usually
    /// generated into a build dir while the program reads it from where it ships.
    pub source: Option<String>,
    /// The directory `path` and `source` are resolved against, filled by
    /// [`read_manifest`] with the declaring `loft.toml`'s own directory — a library's
    /// declaration names a file in the LIBRARY, and the consumer's page has no way to
    /// guess that.
    ///
    /// The `--html` driver REPLACES it for the entry program's own manifest with the
    /// PROGRAM's directory, because `path` is the string that program passes and loft
    /// resolves such a string relative to the program file (`crate::html_embed`).
    pub root: String,
}

/// Content of a library's `loft.toml` manifest file.
#[derive(Debug, Default)]
pub struct Manifest {
    /// Package name from `[package] name = "..."`.
    pub name: Option<String>,
    /// Package version from `[package] version = "..."`.
    pub version: Option<String>,
    /// Entry `.loft` file path, relative to the package root.
    /// Defaults to `src/<name>.loft` when absent.
    pub entry: Option<String>,
    /// Interpreter version requirement from the `[package]` section,
    /// e.g. `">=1.0"`.  `None` means no constraint.
    pub loft_version: Option<String>,
    /// Compatibility-contract requirement from `[package] contract = "..."`
    /// (@PLN102 arc B-semantic).  The `contract` axis is a monotone integer
    /// that increments on a *silent* breaking change to loft's contract —
    /// distinct from the calendar release tag `loft_version` bounds.  A bare
    /// integer (`"1"`) is a "tested-at" epoch; ranges (`">=1"`, `">=1, <=3"`)
    /// assert forward-coverage.  `None` means the library declares no contract
    /// (grandfathered — no gate).  See `check_contract` / `CONTRACT_VERSION`.
    pub contract: Option<String>,
    /// The oldest release of THIS package whose public API this one is still a drop-in
    /// for, from `[package] api_compatible_with = "0.3.0"`.
    ///
    /// A real version, not an abstract epoch: it means something on sight, it can be
    /// recorded in `loft.lock`, and — the load-bearing part — it NAMES AN ARTIFACT THAT
    /// EXISTS, so the claim is verifiable by fetching that release and running its own
    /// tests against this source. An opaque counter would be none of those things.
    ///
    /// Raising it is the ONLY way to declare an API break, and it is a deliberate edit in
    /// the same commit as the break. The release `version` is deliberately NOT an
    /// indicator — releases mean "newer" and nothing else.
    ///
    /// `None` means the package declares no floor (grandfathered — no gate).
    /// Design: `doc/claude/plans/library-compat-contract/README.md`.
    pub api_compatible_with: Option<String>,
    /// The oldest release of THIS package whose stored/wire data this one still reads,
    /// from `[package] data_compatible_with = "0.1.0"`.
    ///
    /// Separate from [`Manifest::api_compatible_with`] because the two failures differ in
    /// kind: an API break costs a recompile, a data break costs someone's stored file.
    /// `hex_terrain` is the worked example — it kept its API and changed what it computed
    /// over stored heights, which one number cannot express.
    ///
    /// `None` means the package declares no floor (grandfathered — no gate).
    pub data_compatible_with: Option<String>,
    /// GitHub repository that publishes this package's releases, from
    /// `[package] repository = "..."`.  Either a bare repo name under the
    /// `loft-lang` org (`"loft-libs-core"`) or a full `"owner/repo"`.  When
    /// set, the package lives in a **monorepo**, so `loft package` emits the
    /// `<name>-v<version>` release-tag scheme; absent → the legacy
    /// one-repo-per-package `loft-<name>` + `v<version>` fallback.
    pub repository: Option<String>,
    /// One-line package description from `[package] description = "..."`.  The
    /// official source for the registry catalog (`loft search` / `loft api
    /// --registry`); registry tooling prefers this over scraping the README.
    pub description: Option<String>,
    /// Native shared-library stem from `[library] native = "..."`.
    /// `None` for pure-loft packages.  The interpreter resolves this to the
    /// platform-correct filename (`lib<stem>.so` / `.dylib` / `.dll`).
    pub native: Option<String>,
    /// @PLN11 Arc N / N3 — `[library] compile = "native"` opts a **normal** loft
    /// library (no hand-written `#native` / Rust crate) into auto-compilation to a
    /// shared-store cdylib: its native-compilable public functions are marked
    /// native and dispatched (`use <lib>` then runs the library native, the script
    /// interprets).  `None`/absent → the library is interpreted as before.  (B-phase
    /// makes this automatic; today it is the explicit opt-in.)
    pub compile: Option<String>,
    /// @PLN119 arc A — `[library] placement = "inproc" | "process"`: where this
    /// library's functions RUN.  `process` puts them in a worker process that
    /// calls reach over a shared memory-mapped file, so a crash in the library
    /// cannot corrupt its consumer's stores.
    ///
    /// The library declares it, not the consumer, because the library is what
    /// knows whether isolating it is safe and worth the crossing.  Consumers
    /// call the same typed `pub fn` surface either way — that indistinguishability
    /// is the plan's one invariant.
    ///
    /// `None`/absent → in-process, which is what every existing library gets.
    /// Parsed and validated by `lib_placement::Placement::parse`.
    pub placement: Option<String>,
    /// PKG.3: package dependencies from `[dependencies]` section.
    /// Key = package name, value = version requirement or path.
    pub dependencies: Vec<(String, String)>,
    /// PKG.4: Rust crate directory name from `[native] crate = "..."`.
    pub native_crate: Option<String>,
    /// @PLN18 — `[native] in_binary = true`: this library's `#native` symbols
    /// live INSIDE the loft binary (registered in `src/native.rs`), not in a
    /// cdylib.  Readers: the auto-native driver (skips the doomed cdylib
    /// compile + its warning) and the extraction-hygiene gate (the symbols
    /// are sanctioned in `src/**`).  One mark, every consumer.
    pub native_in_binary: bool,
    /// @PLN21 Phase 2 — `[native] runtime-libs = "libGL.so.1, libasound.so.2"`:
    /// the system shared libraries this package's cdylib needs at RUNTIME (its
    /// `dlopen` NEEDED set beyond libc).  Turns a load failure into an actionable
    /// install hint; a missing one of these can NOT be fixed by building from
    /// source (the source cdylib links the same lib) — Phase 3 / decision C3.
    pub runtime_libs: Vec<String>,
    /// @PLN21 Phase 2 — `[native] build-deps = "libgl-dev, libasound2-dev"`:
    /// the system DEV packages needed to BUILD the cdylib from source (the
    /// fallback when no prebuilt matches).  Diagnoses a failed `auto_build_native`
    /// ("install `<dev-lib>`").  Distinct from `runtime_libs` (the `-dev` headers
    /// vs the runtime `.so`).
    pub build_deps: Vec<String>,
    /// @PLN24 arc D — `[c] libs = "liblc_types.so, libpq.so.5"`: the C shared
    /// libraries this package's `#c` bindings resolve against.
    ///
    /// Distinct from `[native] runtime-libs`, which names what a Rust cdylib
    /// needs present and only PROBES for it. These are `dlopen`ed and kept
    /// loaded, because a `#c` symbol is looked up in them — and they are linked
    /// on `--native` for the same reason. A binding to libc needs no entry: it
    /// is already in the process.
    ///
    /// Names are sonames as the dynamic linker knows them (`libpq.so.5`), the
    /// same spelling `runtime-libs` uses; a bare path resolves against the
    /// package directory so a library can ship its own `.so`.
    pub c_libs: Vec<String>,
    /// @PLN24 arc G — `[c] optional-libs = "libduckdb.so, libpq.so.5"`: C
    /// libraries this package binds but does NOT require.
    ///
    /// Same spelling and same resolution rules as `c_libs`; the difference is
    /// entirely in when the library is needed. A required library is opened at
    /// load and linked by `--native`, so its absence is an early, actionable
    /// failure. An optional one is neither: it is opened when a symbol from it
    /// is first looked up, and `--native` resolves it at that moment too rather
    /// than putting it on the link line — so a program that never calls into it
    /// BUILDS AND RUNS on a machine where it is not installed.
    ///
    /// That is what lets one package bind several backends (a database client
    /// over sqlite, postgres and duckdb) without making a user install all of
    /// them to use one. The cost is that "is it there?" becomes a question the
    /// program must ask before calling — `c_library_available`.
    pub c_optional_libs: Vec<String>,
    /// @PLN24 arc D — `[c] shim = "src/shim.c"`: ANSI-C sources this package
    /// ships for the signatures the fixed trampolines cannot express (a float
    /// argument, a struct by value, varargs, an out-parameter, a caller-frees
    /// `char *` return).
    ///
    /// Compiled by `cc` — never rustc, which is the whole point of `#c` — and
    /// the result is then registered exactly like a `[c] libs` entry, so
    /// nothing downstream can tell a shim from any other C library.
    ///
    /// Paths resolve against the package directory.
    pub c_shim: Vec<String>,
    /// PKG.4: loft function name → Rust symbol path from `[native.functions]`.
    pub native_functions: Vec<(String, String)>,
    /// PKG.5: WASM-specific overrides from `[native.wasm]`.
    pub native_wasm: Vec<(String, String)>,
    /// lib_plan-29 W1c: bridge crate from `[wasm.bridge] crate = "..."`.
    /// `None` for libraries with no browser-WASM bridge.  When set, the
    /// `--html` driver builds `<pkg>/wasm/` (cargo path-rel) to a
    /// `wasm32-unknown-unknown` rlib and links it via `--extern`.
    pub wasm_bridge_crate: Option<String>,
    /// lib_plan-29 W1c: loft `#native` symbol → bridge `pub fn` name from
    /// `[wasm.bridge.routes]`.  Read by `src/generation/mod.rs::output_
    /// native_direct_call` (replaces the hard-coded `WASM_BRIDGE_FNS`).
    pub wasm_bridge_routes: Vec<(String, String)>,
    /// lib_plan-29 W2: per-library JS host-imports file from
    /// `[wasm.bridge] host_js = "..."` (path relative to the package
    /// root).  The `--html` driver reads the file and concatenates it
    /// into the HTML preamble; the bundled JS is expected to push a
    /// registration callback onto `globalThis.LOFT_WASM_EXTENSIONS`
    /// (see `lib/imaging/wasm/host.js` for the canonical shape).
    pub wasm_bridge_host_js: Option<String>,
    /// `[triggers] enabled = true` (lib_plans/59-lazy-stdlib) — opt this
    /// library into lazy auto-load, making `use <pkg>;` optional.  This is the
    /// ONLY trigger metadata: when set, loft DERIVES the actual trigger surface
    /// (the `<pkg>::` namespace, the library's public text-methods, any public
    /// types) from the library's own source — nothing is hand-listed, so there
    /// is no parallel table to drift (same source-of-truth principle as the
    /// source-scanned `#native` symbols).  The resolver side that acts on this
    /// is separate, future work; today this is parsed metadata.
    pub trigger_enabled: bool,
    /// @PLN100 Slice 2 — `[build] default-targets = [...]`: which targets
    /// `loft build` makes when given no target names.  Empty → the driver falls
    /// back to `["native"]` so a zero-config project still builds.
    pub build_default_targets: Vec<String>,
    /// @PLN100 Slice 2 — the `[build.target.<name>]` entries.  Each overlays a
    /// built-in target (`native` / `html` / `wasi`) or declares a new one.
    pub build_targets: Vec<BuildTarget>,
    /// @PLN100 Slice 3 — the `[[build.asset]]` custom asset steps, in file order.
    pub build_assets: Vec<BuildAsset>,
    /// @PLN100 Slice 4 — the `[[test]]` declared test entries, in file order.
    pub build_tests: Vec<BuildTest>,
    /// @PLN146 F5 — the `[[font]]` declarations, in file order.
    pub fonts: Vec<FontDecl>,
    /// @PLN146 F4 — the `[[embed]]` declarations, in file order.
    pub embeds: Vec<EmbedDecl>,
}

impl Manifest {
    /// @PLN100 Slice 2 — the `[build.target.<name>]` entry for `name`, creating an
    /// empty one on first mention so later `[build.target.<name>.requires]` lines
    /// attach to the same target regardless of section order.
    fn build_target_mut(&mut self, name: &str) -> &mut BuildTarget {
        if let Some(pos) = self.build_targets.iter().position(|t| t.name == name) {
            return &mut self.build_targets[pos];
        }
        self.build_targets.push(BuildTarget {
            name: name.to_string(),
            ..Default::default()
        });
        self.build_targets
            .last_mut()
            .expect("just pushed a build target")
    }
}

/// Read and parse a `loft.toml` file at `path`.
/// Returns `None` when the file does not exist or cannot be read.
#[must_use]
pub fn read_manifest(path: &str) -> Option<Manifest> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut m = parse_manifest(&content, path);
    // @PLN146 F4 — an `[[embed]] source` is relative to the file that declares it,
    // and only the reader knows which file that was.  Recording it here rather than
    // at each call site keeps that fact in one home: a library's declaration and an
    // app's reach the `--html` driver through different paths, and a base dir
    // derived twice is a base dir that can differ.
    let root = std::path::Path::new(path)
        .parent()
        .map_or_else(String::new, |d| d.to_string_lossy().to_string());
    for e in &mut m.embeds {
        e.root.clone_from(&root);
    }
    Some(m)
}

/// A parsed manifest value: a `Scalar` (string / bool / bareword) or an array
/// `List`.  Keeps the token walk from caring which the caller wants.
enum MValue {
    Scalar(String),
    List(Vec<String>),
}

impl MValue {
    /// The value as a single string (a list joins on `,`).
    fn scalar(&self) -> String {
        match self {
            MValue::Scalar(s) => s.clone(),
            MValue::List(v) => v.join(","),
        }
    }
    /// The value as a list (a non-empty scalar is a one-item list).
    fn list(&self) -> Vec<String> {
        match self {
            MValue::List(v) => v.clone(),
            MValue::Scalar(s) if s.is_empty() => Vec::new(),
            MValue::Scalar(s) => vec![s.clone()],
        }
    }
    /// The value as a boolean (`true` bareword).
    fn is_true(&self) -> bool {
        self.scalar() == "true"
    }
}

/// Parse `content` (a `loft.toml`) into a [`Manifest`], tokenising with the loft
/// [`Lexer`] under a TOML lexicon (`#` comments, no string interpolation).
fn parse_manifest(content: &str, filename: &str) -> Manifest {
    let mut m = Manifest::default();
    // The punctuation TOML needs; `#` (the comment marker) is added by `config`.
    // `{`/`}` are listed so a stray inline table lexes rather than faulting; we
    // don't interpret them.
    let cfg = LexConfig::config(&["[", "]", "{", "}", "=", ".", ",", "-"], "#");
    let mut lex = Lexer::from_str_with(content, filename, cfg);
    let mut section = String::new();
    loop {
        match lex.peek().has.clone() {
            LexItem::None => break,
            LexItem::Token(t) if t == "[" => section = read_section(&mut lex, &mut m),
            LexItem::Identifier(_) => {
                let key = read_key(&mut lex);
                if !lex.has_token("=") {
                    continue; // malformed line; read_key already advanced
                }
                let value = read_value(&mut lex);
                apply_kv(&mut m, &section, &key, &value);
            }
            // Any stray token (or a value orphaned by a malformed line): skip one,
            // guaranteeing forward progress.
            _ => lex.cont(),
        }
    }
    m
}

/// Read a `[section]` or `[[array-of-tables]]` header (current token is `[`) and
/// return the section string.  An `[[build.asset]]` / `[[test]]` / `[[font]]` /
/// `[[embed]]` header
/// also pushes a fresh element and returns the `[[…]]` sentinel the key-walk keys on.
fn read_section(lex: &mut Lexer, m: &mut Manifest) -> String {
    lex.has_token("["); // consume the opening `[`
    let array_of_tables = lex.has_token("["); // a second `[` => `[[name]]`
    let name = read_dotted(lex);
    lex.has_token("]"); // closing `]`
    if array_of_tables {
        lex.has_token("]"); // the second closing `]`
        match name.as_str() {
            "build.asset" => m.build_assets.push(BuildAsset::default()),
            "test" => m.build_tests.push(BuildTest::default()),
            "font" => m.fonts.push(FontDecl::default()),
            "embed" => m.embeds.push(EmbedDecl::default()),
            _ => {}
        }
        return format!("[[{name}]]");
    }
    name
}

/// Read a dotted, possibly-hyphenated name (a section name or a bare key), joining
/// identifiers with the `.` / `-` tokens between them, until a non-name token.
fn read_dotted(lex: &mut Lexer) -> String {
    let mut name = String::new();
    loop {
        match lex.peek().has.clone() {
            LexItem::Identifier(s) => {
                name.push_str(&s);
                lex.cont();
            }
            LexItem::Token(ref t) if t == "." || t == "-" => {
                name.push_str(t);
                lex.cont();
            }
            _ => break,
        }
    }
    name
}

/// Read a bare key: identifiers joined by `-` (TOML bare keys allow hyphens, which
/// the lexer splits; underscores are already part of an identifier), up to `=`.
fn read_key(lex: &mut Lexer) -> String {
    let mut key = String::new();
    loop {
        match lex.peek().has.clone() {
            LexItem::Identifier(s) => {
                key.push_str(&s);
                lex.cont();
            }
            LexItem::Token(ref t) if t == "-" => {
                key.push('-');
                lex.cont();
            }
            _ => break,
        }
    }
    key
}

/// Read a value: a `["…", …]` array, a `"…"` string, a bareword (`true` / bare
/// value), or a number.  Consumes exactly the value's tokens.
fn read_value(lex: &mut Lexer) -> MValue {
    match lex.peek().has.clone() {
        LexItem::Token(ref t) if t == "[" => {
            lex.cont(); // consume `[`
            let mut items = Vec::new();
            loop {
                match lex.peek().has.clone() {
                    LexItem::None => break,
                    LexItem::Token(ref t) if t == "]" => {
                        lex.cont();
                        break;
                    }
                    LexItem::CString(s) | LexItem::Identifier(s) => {
                        items.push(s);
                        lex.cont();
                    }
                    // Commas and any stray tokens between items: skip.
                    _ => lex.cont(),
                }
            }
            MValue::List(items)
        }
        LexItem::Token(ref t) if t == "{" => {
            // An inline table (e.g. a dependency `{ path = "../x" }`).  Reconstruct
            // the raw `{ … }` text — the form `extract_path_dep` parses — rather
            // than interpret it here; this is the only inline-table use.
            lex.cont(); // consume `{`
            let mut raw = String::from("{ ");
            loop {
                match lex.peek().has.clone() {
                    LexItem::None => break,
                    LexItem::Token(ref t) if t == "}" => {
                        lex.cont();
                        break;
                    }
                    LexItem::CString(s) => {
                        raw.push('"');
                        raw.push_str(&s);
                        raw.push_str("\" ");
                        lex.cont();
                    }
                    LexItem::Identifier(s) => {
                        raw.push_str(&s);
                        raw.push(' ');
                        lex.cont();
                    }
                    LexItem::Token(t) => {
                        raw.push_str(&t);
                        raw.push(' ');
                        lex.cont();
                    }
                    _ => lex.cont(),
                }
            }
            raw.push('}');
            MValue::Scalar(raw)
        }
        LexItem::CString(s) | LexItem::Identifier(s) => {
            lex.cont();
            MValue::Scalar(s)
        }
        LexItem::Integer(n, _) => {
            lex.cont();
            MValue::Scalar(n.to_string())
        }
        LexItem::Long(n) => {
            lex.cont();
            MValue::Scalar(n.to_string())
        }
        _ => {
            lex.cont();
            MValue::Scalar(String::new())
        }
    }
}

/// Apply one `key = value` to `m` under `section` — the dispatch table for every
/// manifest field.
fn apply_kv(m: &mut Manifest, section: &str, key: &str, value: &MValue) {
    // @PLN100 Slice 3/4 — the current `[[build.asset]]` / `[[test]]` element.
    if section == "[[build.asset]]" {
        if let Some(a) = m.build_assets.last_mut() {
            match key {
                "name" => a.name = Some(value.scalar()),
                "run" => a.run = Some(value.scalar()),
                "lifetime" => a.lifetime = Some(value.scalar()),
                "inputs" => a.inputs = value.list(),
                "outputs" => a.outputs = value.list(),
                "targets" => a.targets = value.list(),
                _ => {}
            }
        }
        return;
    }
    if section == "[[font]]" {
        if let Some(f) = m.fonts.last_mut() {
            match key {
                "family" => f.family = Some(value.scalar()),
                "native" => f.native = Some(value.scalar()),
                "url" => f.url = Some(value.scalar()),
                "stylesheet" => f.stylesheet = Some(value.scalar()),
                _ => {}
            }
        }
        return;
    }
    if section == "[[embed]]" {
        if let Some(e) = m.embeds.last_mut() {
            match key {
                "path" => e.path = Some(value.scalar()),
                "source" => e.source = Some(value.scalar()),
                _ => {}
            }
        }
        return;
    }
    if section == "[[test]]" {
        if let Some(t) = m.build_tests.last_mut() {
            match key {
                "name" => t.name = Some(value.scalar()),
                "run" => t.run = Some(value.scalar()),
                "targets" => t.targets = value.list(),
                "needs" => t.needs = value.list(),
                "inputs" => t.inputs = value.list(),
                _ => {}
            }
        }
        return;
    }
    // @PLN100 Slice 2 — the dynamic [build] / [build.target.<name>] sections.
    if section == "build" {
        if key == "default-targets" {
            m.build_default_targets = value.list();
        }
        return;
    }
    if let Some(rest) = section.strip_prefix("build.target.") {
        if let Some(tname) = rest.strip_suffix(".requires") {
            let t = m.build_target_mut(tname);
            match key {
                "rust-targets" => t.requires_rust_targets = value.list(),
                "tools" => t.requires_tools = value.list(),
                _ => {}
            }
        } else {
            let t = m.build_target_mut(rest);
            match key {
                "shape" => t.shape = Some(value.scalar()),
                "triple" => t.triple = Some(value.scalar()),
                "features" => t.features = value.list(),
                _ => {}
            }
        }
        return;
    }
    match (section, key) {
        ("package", "name") => m.name = Some(value.scalar()),
        ("package", "version") => m.version = Some(value.scalar()),
        ("package", "loft") => m.loft_version = Some(value.scalar()),
        ("package", "contract") => m.contract = Some(value.scalar()),
        ("package", "api_compatible_with") => m.api_compatible_with = Some(value.scalar()),
        ("package", "data_compatible_with") => m.data_compatible_with = Some(value.scalar()),
        ("package", "repository") => m.repository = Some(value.scalar()),
        ("package", "description") => m.description = Some(value.scalar()),
        ("library", "entry") => m.entry = Some(value.scalar()),
        ("library", "native") => m.native = Some(value.scalar()),
        ("library", "compile") => m.compile = Some(value.scalar()),
        ("library", "placement") => m.placement = Some(value.scalar()),
        ("dependencies", _) => m.dependencies.push((key.to_string(), value.scalar())),
        ("native", "crate") => m.native_crate = Some(value.scalar()),
        ("native", "in_binary") => m.native_in_binary = value.is_true(),
        ("c", "libs") => m.c_libs = split_list(&value.scalar()),
        ("c", "optional-libs") => m.c_optional_libs = split_list(&value.scalar()),
        ("c", "shim") => m.c_shim = split_list(&value.scalar()),
        ("native", "runtime-libs") => m.runtime_libs = split_list(&value.scalar()),
        ("native", "build-deps") => m.build_deps = split_list(&value.scalar()),
        ("native.functions", _) => m.native_functions.push((key.to_string(), value.scalar())),
        ("native.wasm", _) => m.native_wasm.push((key.to_string(), value.scalar())),
        ("wasm.bridge", "crate") => m.wasm_bridge_crate = Some(value.scalar()),
        ("wasm.bridge", "host_js") => m.wasm_bridge_host_js = Some(value.scalar()),
        ("wasm.bridge.routes", _) => m.wasm_bridge_routes.push((key.to_string(), value.scalar())),
        ("triggers", "enabled") => m.trigger_enabled = value.is_true(),
        _ => {}
    }
}

/// @PLN21 Phase 2 — parse a comma-separated manifest value into a trimmed,
/// non-empty list: `"libGL.so.1, libasound.so.2"` → `["libGL.so.1",
/// "libasound.so.2"]`.  Backs `[native] runtime-libs` / `build-deps`.
fn split_list(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

/// Outcome of checking a `loft = "<constraint>"` manifest requirement against
/// the running interpreter version.  Three distinct cases so the loader can
/// reject the two kinds of failure with different diagnostics (@PLN102 arc B).
#[derive(Debug, PartialEq, Eq)]
pub enum VersionCheck {
    /// The interpreter satisfies every predicate in the constraint (or the
    /// constraint is empty).
    Satisfied,
    /// The constraint parsed, but the interpreter version does not satisfy it
    /// (e.g. `>=9999.0`, an unmet upper bound, or an exact pin that differs).
    Unsatisfied,
    /// The constraint is not valid loft version-requirement syntax.  A
    /// constraint the loader cannot honour must be rejected LOUDLY, never
    /// silently accepted as "any version" — that silent-accept was the
    /// category-S failure @PLN102 arc B removes.  The payload is the reason.
    Malformed(String),
}

/// One comparison predicate parsed from a constraint (`>=0.8`, `<2027`, …).
struct Predicate {
    op: VersionOp,
    ver: (u32, u32, u32),
}

#[derive(Clone, Copy)]
enum VersionOp {
    Ge,
    Le,
    Gt,
    Lt,
    Eq,
}

impl Predicate {
    fn matches(&self, cur: (u32, u32, u32)) -> bool {
        match self.op {
            VersionOp::Ge => cur >= self.ver,
            VersionOp::Le => cur <= self.ver,
            VersionOp::Gt => cur > self.ver,
            VersionOp::Lt => cur < self.ver,
            VersionOp::Eq => cur == self.ver,
        }
    }
}

/// Check whether the `required` version constraint is satisfied by `current`.
///
/// Supported syntax: comparison predicates `>=`, `<=`, `>`, `<`, `=` over
/// `X` / `X.Y` / `X.Y.Z` versions, AND-combined by comma (`">=0.8, <2027"`).
/// A bare version with no operator (`"0.8"`) is a lower bound (`>=`), for
/// backward compatibility with the many libraries that carry `loft = ">=0.8"`.
/// An empty constraint is satisfied by anything.
///
/// Anything else — an unsupported operator (`^`, `~`, `!=`), a non-numeric or
/// malformed version, an empty predicate — is [`VersionCheck::Malformed`], NOT
/// a silent accept.  This is the @PLN102 arc-B contract: honour the constraint
/// exactly as written, or reject it loudly.
#[must_use]
pub fn check_version(required: &str, current: &str) -> VersionCheck {
    let required = required.trim();
    if required.is_empty() {
        return VersionCheck::Satisfied;
    }
    let Some(cur) = parse_version(current) else {
        // `current` is `env!("CARGO_PKG_VERSION")` at the real call site, so
        // this is defensive; a bad build version is itself a hard error.
        return VersionCheck::Malformed(format!(
            "interpreter version '{current}' is not a valid version"
        ));
    };
    // Comma-separated predicates are AND-ed into a range.
    for part in required.split(',') {
        let pred = match parse_predicate(part.trim()) {
            Ok(p) => p,
            Err(why) => return VersionCheck::Malformed(why),
        };
        if !pred.matches(cur) {
            return VersionCheck::Unsatisfied;
        }
    }
    VersionCheck::Satisfied
}

/// Parse one predicate: an optional operator prefix followed by a version.  A
/// leading non-digit that is not a recognised operator (`^`, `garbage`) is
/// malformed — not a bare version — so unsupported syntax rejects loudly.
fn parse_predicate(s: &str) -> Result<Predicate, String> {
    if s.is_empty() {
        return Err("empty version predicate (a stray comma?)".to_string());
    }
    let (op, rest) = if let Some(r) = s.strip_prefix(">=") {
        (VersionOp::Ge, r)
    } else if let Some(r) = s.strip_prefix("<=") {
        (VersionOp::Le, r)
    } else if let Some(r) = s.strip_prefix('>') {
        (VersionOp::Gt, r)
    } else if let Some(r) = s.strip_prefix('<') {
        (VersionOp::Lt, r)
    } else if let Some(r) = s.strip_prefix('=') {
        (VersionOp::Eq, r)
    } else if s.starts_with(|c: char| c.is_ascii_digit()) {
        (VersionOp::Ge, s) // bare version → lower bound (backward compat)
    } else {
        return Err(format!(
            "unsupported version constraint '{s}' — use >=, <=, >, <, =, or a comma-separated range"
        ));
    };
    let rest = rest.trim();
    let ver = parse_version(rest)
        .ok_or_else(|| format!("'{rest}' is not a valid version in constraint predicate '{s}'"))?;
    Ok(Predicate { op, ver })
}

/// Outcome of validating a declared compatibility floor
/// (`api_compatible_with` / `data_compatible_with`).
#[derive(Debug, PartialEq, Eq)]
pub enum FloorCheck {
    /// Absent — the package declares no floor (grandfathered).
    Undeclared,
    /// A well-formed floor at or below the package's own version.
    Ok((u32, u32, u32)),
    /// Not a version. Rejected loudly rather than treated as "no floor": a claim
    /// nobody can parse must never read as a claim nobody needs to check — the
    /// mistake `check_version` made when it accepted `"garbage"` as "any version".
    Malformed(String),
    /// A floor NEWER than the package's own version — a package cannot be a drop-in
    /// for a release that does not exist yet, and the artifact the claim names could
    /// never be fetched to verify it.
    AboveSelf { floor: String, own: String },
}

/// Validate a declared compatibility floor against the package's own `version`.
///
/// Both floors use this: they differ in what they promise (API surface vs stored data),
/// not in how they are written. See
/// `doc/claude/plans/library-compat-contract/README.md`.
#[must_use]
pub fn check_floor(declared: Option<&str>, own_version: Option<&str>) -> FloorCheck {
    let Some(raw) = declared else {
        return FloorCheck::Undeclared;
    };
    let Some(floor) = parse_version(raw) else {
        return FloorCheck::Malformed(format!(
            "`{raw}` is not a version — a compatibility floor names a RELEASE of this \
             package (e.g. \"0.3.0\"), because the claim is checked by fetching that \
             release and running its own tests"
        ));
    };
    // An unparseable OWN version is not this check's business to report — `check_version`
    // owns that — so treat it as "cannot compare" rather than inventing a second error.
    if let Some(own_raw) = own_version
        && let Some(own) = parse_version(own_raw)
        && floor > own
    {
        return FloorCheck::AboveSelf {
            floor: raw.to_string(),
            own: own_raw.to_string(),
        };
    }
    FloorCheck::Ok(floor)
}

/// Parse `X` / `X.Y` / `X.Y.Z` into a numeric triple (missing parts default to
/// 0).  Returns `None` for any non-numeric component or a fourth component —
/// the malformed signal the old `unwrap_or(0)` swallowed.
fn parse_version(s: &str) -> Option<(u32, u32, u32)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut parts = s.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().map_or(Some(0), |x| x.parse().ok())?;
    let patch = parts.next().map_or(Some(0), |x| x.parse().ok())?;
    if parts.next().is_some() {
        return None; // more than three components → malformed
    }
    Some((major, minor, patch))
}

// ── @PLN102 arc B-semantic — the compatibility `contract` axis ───────────────
//
// The RELEASE tag stays calendar-versioned (`CARGO_PKG_VERSION`, checked by
// `check_version` above).  The COMPATIBILITY contract is a separate monotone
// integer that increments iff loft makes a *silent* breaking change — old code
// that still compiles and runs, but now produces a different result (the C86
// plain-bind-copy class that broke `hex_terrain`).  Additive and loud-failing
// changes (a missing symbol errors at compile) do NOT bump it, so a library's
// declared contract stays valid exactly as long as loft's silent contract is
// unchanged.  Full rationale: doc/claude/plans/102-stability-contract/versioning-decision.md.

/// loft's current compatibility contract.
///
/// **Currently 0 — pre-freeze**: the mechanism is live but the value has not yet
/// been declared, so existing libraries (which carry no `contract` field) are
/// grandfathered and nothing gates.  **Ratified to flip to 1 — the 1.0 contract
/// baseline — once the last open syntax changes land** (those changes are part of
/// defining what contract 1 *is*, so declaring the baseline before they settle
/// would freeze a moving contract).  After the freeze it increments to 2 on the
/// first silent breaking change, and so on.  The 0→1 flip is a one-line,
/// deliberate follow-up at the freeze; bumping it thereafter is a maintainer act
/// paired with a CHANGELOG note (layout-hash / golden-corpus CI gates make an
/// omitted bump loud — see the versioning decision doc).
pub const CONTRACT_VERSION: u32 = 0;

/// Outcome of checking a library's `contract` requirement against the running
/// interpreter's [`CONTRACT_VERSION`].
#[derive(Debug, PartialEq, Eq)]
pub enum ContractCheck {
    /// loft's contract is within the range the library was tested against.
    Ok,
    /// loft's contract is BELOW the library's minimum — loft is too old for the
    /// library's epoch; the library may rely on semantics not yet in this loft.
    /// Hard reject.
    TooOld { required_min: u32 },
    /// loft's contract has advanced PAST the library's tested ceiling — a silent
    /// break may have landed since it was tested.  Accept, but WARN (the arc-C
    /// deprecation channel): the fix is the author republishing against the
    /// current contract, never a silent wrong answer for the consumer.
    Drifted { tested_max: u32 },
    /// The `contract` requirement is not valid syntax (non-integer, unsupported
    /// operator, contradictory range) — rejected loudly, never silently ignored.
    Malformed(String),
}

/// Check a library's `contract` requirement against loft's `current` contract.
///
/// The requirement is an integer window folded from comma-separated predicates:
/// a **bare integer `N` or `>=N` is a lower bound `[N, ∞)`** — the capability
/// floor a program needs; `=K` pins the exact epoch `[K, K]`; `<=M`/`<M`/`>K`
/// bound the range; `">=K, <=M"` is `[K, M]`.  Then `current < lo` →
/// [`ContractCheck::TooOld`], `current > hi` → [`ContractCheck::Drifted`], else
/// [`ContractCheck::Ok`].  An empty requirement is `Ok` (library declares no
/// contract — grandfathered).
///
/// A bare contract is **forward-open (`>=`), NOT "tested-at"**: the compatibility
/// promise ([COMPATIBILITY.md](../doc/claude/COMPATIBILITY.md)) guarantees loft
/// never breaks a working program forward, so there is nothing to warn about on a
/// newer loft — `Drifted` only ever fires for an *explicit* upper bound an author
/// opted into. (This supersedes the original "tested-at + drift" default, which
/// predated the absolute-compat commitment.)
#[must_use]
pub fn check_contract(required: &str, current: u32) -> ContractCheck {
    let required = required.trim();
    if required.is_empty() {
        return ContractCheck::Ok;
    }
    let mut lo: u32 = 0;
    let mut hi: u32 = u32::MAX;
    for part in required.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return ContractCheck::Malformed(format!(
                "empty contract predicate in '{required}' (a stray comma?)"
            ));
        }
        let (op, num) = if let Some(r) = part.strip_prefix(">=") {
            (VersionOp::Ge, r)
        } else if let Some(r) = part.strip_prefix("<=") {
            (VersionOp::Le, r)
        } else if let Some(r) = part.strip_prefix('>') {
            (VersionOp::Gt, r)
        } else if let Some(r) = part.strip_prefix('<') {
            (VersionOp::Lt, r)
        } else if let Some(r) = part.strip_prefix('=') {
            (VersionOp::Eq, r)
        } else if part.starts_with(|c: char| c.is_ascii_digit()) {
            (VersionOp::Ge, part) // bare contract → lower bound (forward-open; the promise guarantees no forward break)
        } else {
            return ContractCheck::Malformed(format!(
                "unsupported contract constraint '{part}' — use an integer, or >=, <=, >, <, =, or a comma range"
            ));
        };
        let Ok(k) = num.trim().parse::<u32>() else {
            return ContractCheck::Malformed(format!(
                "'{}' is not an integer contract version in '{part}'",
                num.trim()
            ));
        };
        match op {
            VersionOp::Eq => {
                lo = lo.max(k);
                hi = hi.min(k);
            }
            VersionOp::Ge => lo = lo.max(k),
            VersionOp::Gt => lo = lo.max(k.saturating_add(1)),
            VersionOp::Le => hi = hi.min(k),
            VersionOp::Lt => hi = hi.min(k.saturating_sub(1)),
        }
    }
    if lo > hi {
        return ContractCheck::Malformed(format!(
            "contradictory contract range in '{required}' (lower bound {lo} exceeds upper bound {hi})"
        ));
    }
    if current < lo {
        ContractCheck::TooOld { required_min: lo }
    } else if current > hi {
        ContractCheck::Drifted { tested_max: hi }
    } else {
        ContractCheck::Ok
    }
}

/// Extract the `path` value from an inline-table dependency value
/// of the form `{ path = "X" }` (with optional whitespace + other
/// fields).  Returns `None` for plain version strings (`"0.1"`,
/// `">=0.2"`) or other shapes.
///
/// @PLAN12 phase 3.5b (2026-05-24) — path-deps in `loft.toml`
/// `[dependencies]` were decorative before this; the manifest
/// parser stored the entire inline-table as an opaque "version"
/// string.  This helper extracts the path so the caller in
/// `src/parser/mod.rs::apply_manifest_side_effects` can resolve
/// it relative to the package directory and register the parent
/// in `lib_dirs`.
#[must_use]
pub fn extract_path_dep(value: &str) -> Option<&str> {
    inline_field(value, "path")
}

/// Extract the version requirement from a dependency value: the whole string for
/// a plain `"0.1"` / `">=0.2"`, or the `version` field of an inline table
/// (`{ path = "X", version = ">=0.2" }`).  `None` for an inline table that states
/// no version — a pure path dependency, which is resolved by path and never by
/// a registry lookup.
#[must_use]
pub fn extract_version_req(value: &str) -> Option<&str> {
    let v = value.trim();
    if v.starts_with('{') {
        inline_field(v, "version")
    } else if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// One field out of an inline-table dependency value (`{ path = "X", version = "Y" }`).
///
/// Both extractors above read the same syntax, so they share the reader: two copies
/// drift, and a dependency value that one of them understood and the other did not
/// would resolve differently depending on which asked.
///
/// Commas inside a quoted value would break the split — accept the limitation, paths
/// and version requirements typically contain none.
fn inline_field<'a>(value: &'a str, field: &str) -> Option<&'a str> {
    let v = value.trim();
    let inner = v.strip_prefix('{')?.strip_suffix('}')?.trim();
    for part in inner.split(',') {
        let part = part.trim();
        if let Some(rhs) = part.strip_prefix(field) {
            let rhs = rhs.trim().strip_prefix('=')?.trim();
            return Some(rhs.trim_start_matches('"').trim_end_matches('"'));
        }
    }
    None
}

/// Record `name = "<requirement>"` under `[dependencies]` in the manifest at `path`.
///
/// loft#968 — the manifest is what makes a dependency PROVABLE. `loft install <pkg>`
/// wrote only `loft.lock`, so a project could not distinguish *"we depend on this"* from
/// *"this happens to be installed on the box that built it"*: dropping a `[dependencies]`
/// line changed nothing, and the negative gate a consumer wanted — remove the dependency,
/// the tests must stop compiling — could not be written. PKG_REGISTRY.md has described
/// `loft install <name>` as "adds to `loft.toml`'s `[dependencies]` and installs" since
/// 2026-05; this is that half.
///
/// A text edit rather than a re-serialisation: the manifest reader is a token walk with no
/// writer, and rewriting the file from a parsed `Manifest` would drop every comment and
/// reorder every key — a package's manifest is hand-written, and a tool that reformats it
/// is a tool people stop running.
///
/// Answers whether it wrote: `false` when `name` is already declared (in any form,
/// including a path dependency) or the file cannot be read.
pub fn record_dependency(path: &str, name: &str, requirement: &str) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let lines: Vec<&str> = text.lines().collect();
    let mut in_deps = false;
    let mut section_end: Option<usize> = None;
    for (i, line) in lines.iter().enumerate() {
        let t = line.split('#').next().unwrap_or("").trim();
        if t.starts_with('[') {
            if in_deps {
                section_end = Some(i);
                break;
            }
            in_deps = t == "[dependencies]";
            continue;
        }
        if in_deps && t.split('=').next().is_some_and(|k| k.trim() == name) {
            return false; // already declared — the manifest is already authoritative
        }
    }
    let entry = format!("{name} = \"{requirement}\"");
    let mut out: Vec<String> = lines.iter().map(|l| (*l).to_string()).collect();
    if in_deps {
        // Land at the end of the section's declarations, before whatever blank lines
        // separate it from the next section — so the file keeps its shape.
        let mut at = section_end.unwrap_or(out.len());
        while at > 0 && out[at - 1].trim().is_empty() {
            at -= 1;
        }
        out.insert(at, entry);
    } else {
        // No `[dependencies]` at all: open one at the end.
        if out.last().is_some_and(|l| !l.trim().is_empty()) {
            out.push(String::new());
        }
        out.push("[dependencies]".to_string());
        out.push(entry);
    }
    let mut body = out.join("\n");
    body.push('\n');
    std::fs::write(path, body).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn write_temp(name: &str, content: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir();
        let path = dir.join(format!(
            "loft_manifest_test_{}_{}.toml",
            name,
            std::process::id()
        ));
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    #[test]
    fn parses_loft_version_requirement() {
        let p = write_temp("ver", "[package]\nloft = \">=1.0\"\n");
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.loft_version.as_deref(), Some(">=1.0"));
    }

    #[test]
    fn parses_custom_entry() {
        let p = write_temp("entry", "[library]\nentry = \"src/mylib.loft\"\n");
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.entry.as_deref(), Some("src/mylib.loft"));
    }

    #[test]
    fn version_current_passes() {
        assert_eq!(check_version(">=0.1", "0.1.0"), VersionCheck::Satisfied);
        assert_eq!(check_version(">=1.0", "1.2.3"), VersionCheck::Satisfied);
        assert_eq!(check_version(">=0.1.0", "0.1.0"), VersionCheck::Satisfied);
        assert_eq!(check_version("", "0.1.0"), VersionCheck::Satisfied);
    }

    #[test]
    fn parses_package_name_and_version() {
        let p = write_temp(
            "pkg",
            "[package]\nname = \"graphics\"\nversion = \"0.2.1\"\nloft = \">=0.8\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.name.as_deref(), Some("graphics"));
        assert_eq!(m.version.as_deref(), Some("0.2.1"));
        assert_eq!(m.loft_version.as_deref(), Some(">=0.8"));
        assert_eq!(m.repository, None);
    }

    /// `[package] repository` parses (drives `loft package`'s monorepo URL/tag).
    #[test]
    fn parses_package_repository() {
        let p = write_temp(
            "repo",
            "[package]\nname = \"crypto\"\nversion = \"0.2.1\"\nrepository = \"loft-libs-core\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.repository.as_deref(), Some("loft-libs-core"));
    }

    #[test]
    fn parses_package_description() {
        let p = write_temp(
            "desc",
            "[package]\nname = \"crypto\"\nversion = \"0.2.1\"\ndescription = \"SHA-256, HMAC, base64.\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.description.as_deref(), Some("SHA-256, HMAC, base64."));
    }

    // @PLN21 Phase 2 — `[native] runtime-libs` / `build-deps` parse into trimmed
    // lists (with and without spaces), and are empty when absent.
    #[test]
    fn parses_native_runtime_and_build_deps() {
        let p = write_temp(
            "ndeps",
            "[native]\ncrate = \"loft-graphics\"\nruntime-libs = \"libGL.so.1, libasound.so.2\"\nbuild-deps = \"libgl-dev,libasound2-dev\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.runtime_libs, vec!["libGL.so.1", "libasound.so.2"]);
        assert_eq!(m.build_deps, vec!["libgl-dev", "libasound2-dev"]);

        let p2 = write_temp("nodeps", "[native]\ncrate = \"loft-random\"\n");
        let m2 = read_manifest(p2.to_str().unwrap()).unwrap();
        assert!(m2.runtime_libs.is_empty() && m2.build_deps.is_empty());
    }

    #[test]
    fn parses_dependencies() {
        let p = write_temp(
            "deps",
            "[dependencies]\nmath = \">=0.2\"\nutils = \"../utils\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.dependencies.len(), 2);
        assert_eq!(m.dependencies[0], ("math".to_string(), ">=0.2".to_string()));
        assert_eq!(
            m.dependencies[1],
            ("utils".to_string(), "../utils".to_string())
        );
    }

    // The lexer-based reader reconstructs an inline-table dependency so
    // `extract_path_dep` still finds the path (regression: i337).
    #[test]
    fn parses_inline_table_path_dep() {
        let p = write_temp(
            "inlinedep",
            "[package]\nname = \"b\"\n[dependencies]\na = { path = \"../elsewhere/nested/a\" }\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        let (name, value) = m
            .dependencies
            .iter()
            .find(|(n, _)| n == "a")
            .expect("dependency `a` present");
        assert_eq!(name, "a");
        assert_eq!(
            extract_path_dep(value),
            Some("../elsewhere/nested/a"),
            "raw value was {value:?}"
        );
    }

    #[test]
    fn parses_trigger_enabled() {
        let p = write_temp(
            "triggers",
            "[package]\nname = \"regex\"\n\n[triggers]\nenabled = true\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert!(m.trigger_enabled, "[triggers] enabled = true should parse");
    }

    #[test]
    fn trigger_disabled_by_default() {
        let p = write_temp("notrig", "[package]\nname = \"plain\"\n");
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert!(!m.trigger_enabled, "no [triggers] section -> disabled");
    }

    #[test]
    fn parses_compile_native() {
        let p = write_temp(
            "compile",
            "[package]\nname = \"mathlib\"\n\n[library]\ncompile = \"native\"\n",
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.compile.as_deref(), Some("native"));
    }

    #[test]
    fn compile_absent_by_default() {
        let p = write_temp("nocompile", "[package]\nname = \"plain\"\n");
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.compile, None);
    }

    #[test]
    fn parses_native_section() {
        let p = write_temp(
            "native",
            r#"[package]
name = "graphics"
version = "0.1.0"

[native]
crate = "graphics_native"

[native.functions]
save_png = "graphics_native::save_png"
load_font = "graphics_native::load_font"

[native.wasm]
gl_create = "graphics_native::webgl::create_canvas"
"#,
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.native_crate.as_deref(), Some("graphics_native"));
        assert_eq!(m.native_functions.len(), 2);
        assert_eq!(
            m.native_functions[0],
            (
                "save_png".to_string(),
                "graphics_native::save_png".to_string()
            )
        );
        assert_eq!(m.native_wasm.len(), 1);
        assert_eq!(
            m.native_wasm[0],
            (
                "gl_create".to_string(),
                "graphics_native::webgl::create_canvas".to_string()
            )
        );
    }

    // @PLN100 Slice 2 — the [build] section: default-targets array,
    // [build.target.<name>] keys, and the [build.target.<name>.requires] subtable
    // (out-of-order lines attach to the same target).
    #[test]
    fn parses_build_section() {
        let p = write_temp(
            "build",
            r#"[package]
name = "game"

[build]
default-targets = ["native", "html"]

[build.target.html.requires]
rust-targets = ["wasm32-unknown-unknown"]
tools = ["wasm-opt"]

[build.target.html]
shape = "html"
triple = "wasm32-unknown-unknown"
features = ["random", "png"]
"#,
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.build_default_targets, vec!["native", "html"]);
        assert_eq!(m.build_targets.len(), 1);
        let html = &m.build_targets[0];
        assert_eq!(html.name, "html");
        assert_eq!(html.shape.as_deref(), Some("html"));
        assert_eq!(html.triple.as_deref(), Some("wasm32-unknown-unknown"));
        assert_eq!(html.features, vec!["random", "png"]);
        assert_eq!(html.requires_rust_targets, vec!["wasm32-unknown-unknown"]);
        assert_eq!(html.requires_tools, vec!["wasm-opt"]);
    }

    #[test]
    fn build_section_absent_by_default() {
        let p = write_temp("nobuild", "[package]\nname = \"plain\"\n");
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert!(m.build_default_targets.is_empty() && m.build_targets.is_empty());
    }

    // @PLN146 F4 — [[embed]] array-of-tables: the files a --html page carries.
    #[test]
    fn parses_embed_declarations() {
        let p = write_temp(
            "embeds",
            r#"[package]
name = "game"

# ships where the program reads it
[[embed]]
path = "assets/game.pack"

# generated into a build dir, read from where it ships
[[embed]]
path   = "assets/atlas.pack"
source = "build/atlas.pack"
"#,
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.embeds.len(), 2);
        assert_eq!(m.embeds[0].path.as_deref(), Some("assets/game.pack"));
        assert_eq!(m.embeds[0].source, None);
        assert_eq!(m.embeds[1].source.as_deref(), Some("build/atlas.pack"));
        // The reader is the only place that knows which file declared these, and
        // `source` is relative to it — so every entry carries that directory.
        let dir = p.parent().unwrap().to_string_lossy().to_string();
        assert_eq!(m.embeds[0].root, dir);
        assert_eq!(m.embeds[1].root, dir);
        // The control: a manifest that declares none has none.
        let bare = write_temp("noembed", "[package]\nname = \"plain\"\n");
        assert!(
            read_manifest(bare.to_str().unwrap())
                .unwrap()
                .embeds
                .is_empty()
        );
    }

    // @PLN146 F5 — [[font]] array-of-tables: the three sources, in file order.
    #[test]
    fn parses_font_declarations() {
        let p = write_temp(
            "fonts",
            r#"[package]
name = "game"

# a family the browser already has: nothing to bring
[[font]]
family = "DejaVu Sans"
native = "fonts/DejaVu Sans.ttf"

# our own file server
[[font]]
family = "PressStart2P"
native = "fonts/PressStart2P.ttf"
url    = "fonts/PressStart2P.woff2"

# a provider's stylesheet
[[font]]
family     = "Inter"
stylesheet = "https://fonts.example.test/css2?family=Inter"
"#,
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.fonts.len(), 3);
        assert_eq!(m.fonts[0].family.as_deref(), Some("DejaVu Sans"));
        assert_eq!(m.fonts[0].native.as_deref(), Some("fonts/DejaVu Sans.ttf"));
        assert!(m.fonts[0].url.is_none() && m.fonts[0].stylesheet.is_none());
        assert_eq!(m.fonts[1].url.as_deref(), Some("fonts/PressStart2P.woff2"));
        assert_eq!(
            m.fonts[2].stylesheet.as_deref(),
            Some("https://fonts.example.test/css2?family=Inter")
        );
        assert!(m.fonts[2].native.is_none());
    }

    // @PLN100 Slice 3 — [[build.asset]] array-of-tables, inline + full-line
    // comments, array and scalar fields, and multiple elements.
    #[test]
    fn parses_build_assets() {
        let p = write_temp(
            "assets",
            r#"[package]
name = "game"

# an asset built from local files
[[build.asset]]
name    = "atlas"
run     = "scripts/pack_atlas.loft"
inputs  = ["art/**/*.png"]      # fingerprinted -> rebuild only on change
outputs = ["assets/atlas.bin"]
targets = ["html"]

[[build.asset]]
name     = "dataset"
run      = "scripts/fetch.loft"
outputs  = ["assets/dataset.pak"]
lifetime = "30d"
"#,
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.build_assets.len(), 2);
        let atlas = &m.build_assets[0];
        assert_eq!(atlas.name.as_deref(), Some("atlas"));
        assert_eq!(atlas.run.as_deref(), Some("scripts/pack_atlas.loft"));
        assert_eq!(atlas.inputs, vec!["art/**/*.png"]); // inline comment stripped
        assert_eq!(atlas.outputs, vec!["assets/atlas.bin"]);
        assert_eq!(atlas.targets, vec!["html"]);
        assert!(atlas.lifetime.is_none());
        let dataset = &m.build_assets[1];
        assert_eq!(dataset.name.as_deref(), Some("dataset"));
        assert_eq!(dataset.lifetime.as_deref(), Some("30d"));
        assert!(dataset.inputs.is_empty());
    }

    // @PLN100 Slice 4 — [[test]] entries: run/targets/needs/inputs.
    #[test]
    fn parses_build_tests() {
        let p = write_temp(
            "tests",
            r#"[package]
name = "game"

[[test]]
name    = "smoke"
run     = "tests/smoke.loft"
targets = ["interpret", "native"]
needs   = ["atlas"]

[[test]]
name   = "atlas-integrity"
run    = "tests/check_atlas.loft"
inputs = ["assets/atlas.bin"]
"#,
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.build_tests.len(), 2);
        let smoke = &m.build_tests[0];
        assert_eq!(smoke.name.as_deref(), Some("smoke"));
        assert_eq!(smoke.run.as_deref(), Some("tests/smoke.loft"));
        assert_eq!(smoke.targets, vec!["interpret", "native"]);
        assert_eq!(smoke.needs, vec!["atlas"]);
        let integrity = &m.build_tests[1];
        assert_eq!(integrity.inputs, vec!["assets/atlas.bin"]);
        assert!(integrity.needs.is_empty());
    }

    #[test]
    fn parses_wasm_bridge_section() {
        // Use synthetic names rather than real loft-library symbols
        // (`n_load_png` etc.) so the extraction-hygiene gate doesn't
        // flag this test as a library-symbol mention in `src/**`.
        let p = write_temp(
            "wasm-bridge",
            r#"[package]
name = "demo"
version = "0.1.0"

[wasm.bridge]
crate = "demo-wasm"
host_js = "wasm/host.js"

[wasm.bridge.routes]
n_demo_fn_a = "demo_fn_a"
n_demo_fn_b = "demo_fn_b"
"#,
        );
        let m = read_manifest(p.to_str().unwrap()).unwrap();
        assert_eq!(m.wasm_bridge_crate.as_deref(), Some("demo-wasm"));
        assert_eq!(m.wasm_bridge_host_js.as_deref(), Some("wasm/host.js"));
        assert_eq!(m.wasm_bridge_routes.len(), 2);
        assert_eq!(
            m.wasm_bridge_routes[0],
            ("n_demo_fn_a".to_string(), "demo_fn_a".to_string())
        );
        assert_eq!(
            m.wasm_bridge_routes[1],
            ("n_demo_fn_b".to_string(), "demo_fn_b".to_string())
        );
    }

    #[test]
    fn version_too_high_fails() {
        assert_eq!(check_version(">=2.0", "1.9.9"), VersionCheck::Unsatisfied);
        assert_eq!(check_version(">=1.1", "1.0.0"), VersionCheck::Unsatisfied);
        assert_eq!(check_version(">=1.0.1", "1.0.0"), VersionCheck::Unsatisfied);
    }

    /// @PLN102 arc B — the composition matrix (plan README § Composition
    /// matrix) probed against the shipping interpreter version `2026.7.1`.
    /// The pre-arc-B `check_version` silently ACCEPTED every row but the
    /// positive control by degrading a non-`>=` constraint to `0.0.0`; each
    /// bound must now BIND, and an unparseable constraint must reject LOUDLY
    /// (`Malformed`) rather than pass.
    #[test]
    fn arc_b_version_constraint_matrix() {
        let cur = "2026.7.1";
        use VersionCheck::{Malformed, Satisfied, Unsatisfied};

        // Lower bounds — the grandfathered form every published library carries.
        assert_eq!(check_version(">=0.8", cur), Satisfied);
        assert_eq!(check_version("0.8", cur), Satisfied); // bare = lower bound
        // Positive control: the one cell that fired even before arc B.
        assert!(matches!(check_version(">=9999.0", cur), Unsatisfied));

        // Upper bounds and exact pins must now BIND (were silent ACCEPT before).
        assert_eq!(check_version("<=0.1", cur), Unsatisfied);
        assert_eq!(check_version("<2026.0.0", cur), Unsatisfied);
        assert_eq!(check_version("=0.1", cur), Unsatisfied);
        // A satisfiable upper bound still accepts.
        assert_eq!(check_version("<2027", cur), Satisfied);
        assert_eq!(check_version("=2026.7.1", cur), Satisfied);

        // Ranges (comma-separated AND).
        assert_eq!(check_version(">=0.8, <2027", cur), Satisfied);
        assert_eq!(check_version(">=0.8, <2026", cur), Unsatisfied);

        // Unparseable / unsupported → REJECT LOUDLY, never silent accept.
        assert!(matches!(check_version("garbage", cur), Malformed(_)));
        assert!(matches!(check_version("^0.9", cur), Malformed(_))); // caret unsupported
        assert!(matches!(check_version("~0.9", cur), Malformed(_)));
        assert!(matches!(check_version(">=1.x", cur), Malformed(_)));
        assert!(matches!(check_version(">=", cur), Malformed(_)));
        assert!(matches!(check_version(">=0.8,", cur), Malformed(_))); // empty predicate
        assert!(matches!(check_version("1.2.3.4", cur), Malformed(_))); // too many parts
    }

    /// @PLN102 arc B-semantic — the `contract` axis (a monotone integer,
    /// separate from the calver release tag).  Bare = "tested-at" (exact), so a
    /// newer loft DRIFTS (warn) rather than silently accepting; `>=K` opens the
    /// ceiling (forward-coverage, no drift); below the floor is a hard reject.
    #[test]
    fn arc_b_semantic_contract_check() {
        use ContractCheck::{Drifted, Malformed, Ok, TooOld};

        // Bare integer = lower bound (forward-open): below → TooOld, at or above → Ok.
        // The compat promise guarantees no forward break, so a newer loft never drifts.
        assert_eq!(check_contract("1", 0), TooOld { required_min: 1 });
        assert_eq!(check_contract("1", 1), Ok);
        assert_eq!(check_contract("1", 3), Ok);
        assert_eq!(check_contract("1", 99), Ok);
        // An EXPLICIT exact/upper bound still binds (opt-in; rarely needed under the promise).
        assert_eq!(check_contract("=2", 2), Ok);
        assert_eq!(check_contract("=2", 4), Drifted { tested_max: 2 });

        // `>=K` asserts forward-coverage: no drift on a newer loft.
        assert_eq!(check_contract(">=1", 1), Ok);
        assert_eq!(check_contract(">=1", 9), Ok);
        assert_eq!(check_contract(">=2", 1), TooOld { required_min: 2 });

        // Ranges fold to [lo, hi].
        assert_eq!(check_contract(">=1, <=3", 2), Ok);
        assert_eq!(check_contract(">=1, <=3", 0), TooOld { required_min: 1 });
        assert_eq!(check_contract(">=1, <=3", 5), Drifted { tested_max: 3 });
        assert_eq!(check_contract(">1", 2), Ok); // >1 → lo=2
        assert_eq!(check_contract(">1", 1), TooOld { required_min: 2 });
        assert_eq!(check_contract("<=2", 2), Ok);
        assert_eq!(check_contract("<=2", 3), Drifted { tested_max: 2 });

        // Empty → no gate (library declares no contract).
        assert_eq!(check_contract("", 7), Ok);

        // Malformed → loud, never silent.
        assert!(matches!(check_contract("1.5", 0), Malformed(_))); // not an integer
        assert!(matches!(check_contract("^1", 0), Malformed(_))); // unsupported op
        assert!(matches!(check_contract("garbage", 0), Malformed(_)));
        assert!(matches!(check_contract(">=3, <=1", 2), Malformed(_))); // contradictory
        assert!(matches!(check_contract(">=1,", 1), Malformed(_))); // empty predicate
    }

    #[test]
    fn extract_path_dep_recognizes_inline_table() {
        assert_eq!(
            extract_path_dep(r#"{ path = "../gridmesh" }"#),
            Some("../gridmesh")
        );
        assert_eq!(
            extract_path_dep(r#"{path="../foo/bar"}"#),
            Some("../foo/bar")
        );
        // Multi-field inline table.
        assert_eq!(
            extract_path_dep(r#"{ path = "../X", version = "0.1" }"#),
            Some("../X")
        );
        // version-only entry.
        assert_eq!(extract_path_dep(r#"{ version = "0.1" }"#), None);
        // Plain version string (no inline table).
        assert_eq!(extract_path_dep(">=0.2"), None);
        assert_eq!(extract_path_dep("0.1"), None);
    }

    /// loft#966 — bare `loft install` asks each dependency value for the version it
    /// requires, so the two shapes a value can take must give the same answer.
    #[test]
    fn extract_version_req_reads_both_dependency_shapes() {
        // A plain string IS the requirement.
        assert_eq!(extract_version_req(">=0.2"), Some(">=0.2"));
        assert_eq!(extract_version_req("0.1.0"), Some("0.1.0"));
        // …and so is the `version` field of an inline table, in either order.
        assert_eq!(extract_version_req(r#"{ version = "0.1" }"#), Some("0.1"));
        assert_eq!(
            extract_version_req(r#"{ path = "../X", version = ">=0.2" }"#),
            Some(">=0.2")
        );
        // A pure path dependency states no version: it is resolved by its path, and
        // handing `install_one` a `{ path = … }` string as a requirement would send it
        // to the registry for something that is not there.
        assert_eq!(extract_version_req(r#"{ path = "../gridmesh" }"#), None);
        assert_eq!(extract_version_req(""), None);
    }

    fn temp_manifest(tag: &str, body: &str) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!(
            "loft_968_{tag}_{}_{}.toml",
            std::process::id(),
            tag.len()
        ));
        std::fs::write(&p, body).expect("write manifest");
        p
    }

    /// loft#968 — `loft install <pkg>` records the dependency, so removing the line is a
    /// change the project can notice.
    #[test]
    fn record_dependency_opens_a_section_and_keeps_the_rest() {
        let p = temp_manifest("new", "[package]\nname = \"app\"\nversion = \"0.1.0\"\n");
        assert!(record_dependency(p.to_str().unwrap(), "cbor", "^0.1.3"));
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("[dependencies]"), "{text}");
        assert!(text.contains("cbor = \"^0.1.3\""), "{text}");
        // Everything that was there is still there, unreordered.
        assert!(text.starts_with("[package]\nname = \"app\""), "{text}");
        let _ = std::fs::remove_file(&p);
    }

    /// Into an existing section, at the end of its declarations — a hand-written manifest
    /// that a tool reformats is a tool people stop running.
    #[test]
    fn record_dependency_appends_inside_an_existing_section() {
        let p = temp_manifest(
            "existing",
            "[package]\nname = \"app\"\n\n[dependencies]\n# what we depend on\n             glb = \"^0.1\"\n\n[library]\nentry = \"src/app.loft\"\n",
        );
        assert!(record_dependency(p.to_str().unwrap(), "cbor", "0.1.2"));
        let text = std::fs::read_to_string(&p).unwrap();
        let deps_at = text.find("[dependencies]").expect("section");
        let lib_at = text.find("[library]").expect("library section");
        let cbor_at = text.find("cbor = \"0.1.2\"").expect("the new entry");
        assert!(
            deps_at < cbor_at && cbor_at < lib_at,
            "the entry must land inside [dependencies]\n{text}"
        );
        assert!(text.contains("# what we depend on"), "comment kept\n{text}");
        assert!(text.contains("glb = \"^0.1\""), "sibling kept\n{text}");
        let _ = std::fs::remove_file(&p);
    }

    /// Idempotent, and blind to the shape a declaration takes: a path dependency IS a
    /// declaration, so re-recording it as a registry version would contradict the manifest.
    #[test]
    fn record_dependency_leaves_a_declared_name_alone() {
        let p = temp_manifest(
            "declared",
            "[package]\nname = \"app\"\n\n[dependencies]\ncbor = { path = \"../cbor\" }\n",
        );
        assert!(!record_dependency(p.to_str().unwrap(), "cbor", "^0.1.3"));
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("path = \"../cbor\""), "{text}");
        assert!(!text.contains("^0.1.3"), "{text}");
        let _ = std::fs::remove_file(&p);
    }
}
