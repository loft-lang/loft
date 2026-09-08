// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![warn(clippy::pedantic)]
#![allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    clippy::match_same_arms,
    clippy::collapsible_if,
    clippy::redundant_closure,
    clippy::used_underscore_binding,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::single_match_else,
    clippy::if_not_else,
    clippy::implicit_hasher,
    clippy::unnecessary_wraps,
    clippy::semicolon_if_nothing_returned,
    clippy::uninlined_format_args,
    clippy::let_underscore_untyped,
    clippy::must_use_candidate,
    clippy::option_if_let_else,
    clippy::manual_let_else,
    clippy::redundant_closure_for_method_calls,
    clippy::too_many_lines,
    clippy::type_complexity,
    clippy::map_unwrap_or,
    clippy::format_push_string,
    clippy::map_entry
)]

use loft::base64;
use loft::compile;
use loft::data;
use loft::extensions;
use loft::generation;
use loft::log_config;
use loft::logger;
use loft::manifest;
mod android;
mod native_utils;
use loft::parser;
use loft::platform;
use loft::portable_path;
use loft::scopes;
use loft::state;
mod test_runner;

use crate::native_utils::{
    build_script_native_lib_dirs, default_artifact_path, deps_dir_of, html_wasm_import_modules_ok,
    html_wasm_named_functions, is_output_path, loft_lib_dir, loft_lib_dir_for, project_dir,
};
use crate::test_runner::run_tests;
use loft::diagnostics::Level;
use loft::state::State;
use std::env;
use std::sync::{Arc, Mutex};

/// loft#680 — print the per-target builtin surface: which stdlib builtins are NOT
/// available on a target, and why that answer can be trusted.
///
/// The data is `index/target_surface.json`, DERIVED by asking rustc which runtime methods
/// exist per target (`scripts/gen_target_surface.py`), so it cannot drift from the `cfg`s
/// the real build obeys. It is embedded here so an installed loft can answer without the
/// source tree.
fn print_target_surface(want: Option<&str>) {
    use loft::json::Parsed;
    const SURFACE: &str = include_str!("../index/target_surface.json");
    let Ok(Parsed::Object(root)) = loft::json::parse(SURFACE) else {
        eprintln!("loft targets: embedded surface data is not valid JSON");
        return;
    };
    let field = |obj: &Vec<(String, usize, Parsed)>, key: &str| -> Option<Parsed> {
        obj.iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, _, v)| v.clone())
    };
    let Some(Parsed::Object(targets)) = field(&root, "targets") else {
        eprintln!("loft targets: embedded surface data has no targets");
        return;
    };
    let mut any = false;
    for (_, _, entry) in &targets {
        let Parsed::Object(t) = entry else { continue };
        let text = |key: &str| match field(t, key) {
            Some(Parsed::Str(v)) => v,
            _ => String::new(),
        };
        let (triple, describe) = (text("triple"), text("describe"));
        if want.is_some_and(|w| !triple.contains(w) && !describe.contains(w)) {
            continue;
        }
        any = true;
        let names: Vec<String> = match field(t, "unavailable_builtins") {
            Some(Parsed::Array(items)) => items
                .into_iter()
                .filter_map(|i| match i {
                    Parsed::Str(v) => Some(v),
                    _ => None,
                })
                .collect(),
            _ => Vec::new(),
        };
        println!("{triple} — {describe}");
        if names.is_empty() {
            println!("  every stdlib builtin is available here.");
        } else {
            println!("  {} builtin(s) NOT available:", names.len());
            for n in &names {
                println!("    {n}");
            }
        }
    }
    if any {
        println!(
            "\nDerived by asking rustc which runtime methods exist per target, so it \
             cannot drift from the cfgs (scripts/gen_target_surface.py)."
        );
    } else {
        eprintln!("loft targets: no such target (try `loft targets` for all)");
    }
}

fn print_help() {
    println!("usage: loft [options] <file>     run a loft program");
    println!("       loft                       start the interactive REPL");
    println!("       loft --tests [dir]         run a directory of tests");
    println!();
    // The option list below is long; a newcomer's two most likely actions are
    // "run this file" and "poke at the language", so both are named up here
    // rather than left to be found ~60 lines down.
    println!("Getting started:");
    println!("  loft hello.loft               run a program (compiles via rustc when available,");
    println!("                                otherwise interprets — a downloaded release always");
    println!("                                interprets, which is normal, not a fallback to fix)");
    println!("  loft                          type loft and see results immediately");
    println!();
    println!("Options:");
    println!("  --version                     print version information");
    println!("  -h, --help, -?                print this help message");
    println!(
        "  --path <dir>                  directory containing the default/ library (default: binary location)"
    );
    println!(
        "  --project <dir>               run the script as if launched from <dir>; file I/O is"
    );
    println!(
        "                                sandboxed there and its lib/ sub-directory is searched"
    );
    println!(
        "                                for 'use' imports (useful when the script lives in /tmp)"
    );
    println!("  --lib <dir>                   add <dir> to the 'use' import search path; may be");
    println!("                                repeated for multiple directories");
    println!("  --log-conf <path>             use this log config file instead of the default");
    println!(
        "  --timeout <secs>              hard-kill the process after <secs>+grace seconds (PLAN49)"
    );
    println!(
        "                                LOFT_TIMEOUT=<secs> as env equivalent; grace defaults to"
    );
    println!("                                2s, override via LOFT_TIMEOUT_GRACE=<secs>");
    println!(
        "  --production                  enable production mode (panic/assert log instead of abort)"
    );
    println!(
        "  --generate-log-config [path]  write a documented config file with defaults and exit"
    );
    println!(
        "  --interpret                   run in interpreter/bytecode mode (native is default)\n\
         \x20                               LOFT_NO_NATIVE_LIBS=1 additionally makes every `use`d\n\
         \x20                               library interpret (skips its auto-native cdylib);\n\
         \x20                               LOFT_REQUIRE_NATIVE=1 is the inverse — refuse to run\n\
         \x20                               anything that would fall back to the interpreter"
    );
    println!(
        "  --script                      force beginner-script mode (loose top-level\n\
         \x20                               statements, no `fn main`) — auto-detected otherwise"
    );
    println!(
        "  --dump                        compile to bytecode, dump to stderr, and exit (no execution)"
    );
    println!("  --native                      compile to native Rust via rustc and run (default)");
    println!("  --native-release              like --native but emit only reachable functions,");
    println!("                                compile fully optimised (opt-level 3, one codegen");
    println!("                                unit) and strip the live/debug tier as --lean does:");
    println!("                                a SHIPPED binary.  --native keeps every tier — the");
    println!("                                semantics run");
    println!(
        "  --native-debug                like --native but compile with -Cdebuginfo=2 (DWARF)"
    );
    println!("  --lean                        strip the live/debug tier — smallest binary, no");
    println!("                                live-flip/breakpoints");
    println!("  --dev-soft-halt               demote dev-mode runtime raises to log-and-continue");
    println!("                                (also: LOFT_DEV_SOFT_HALT=1) — surfaces every fault");
    println!(
        "                                site in a single run instead of halting on the first"
    );
    println!(
        "                                and preserve the generated .rs on disk; combine with"
    );
    println!("                                --native-release for optimised + debug-info");
    println!("  --native-emit [out.rs]        write generated Rust source and exit");
    println!("                                (default: .loft/<script>.rs beside the script)");
    println!("  --native-wasm [out.wasm]      compile to WebAssembly (wasm32-wasip2)");
    println!(
        "  --native-android [out.apk]    cross-compile to a signed Android APK (needs \
         ANDROID_NDK_HOME + ANDROID_HOME); a *.so output builds just the library"
    );
    println!("                                (default: .loft/<script>.wasm beside the script)");
    println!("                                for headless/WASI (wasmtime); NOT the browser build");
    println!(
        "  --html [out.html]             compile to a self-contained browser page (the browser"
    );
    println!(
        "                                target: WebGL2 canvas + keyboard/mouse, println output;"
    );
    println!(
        "                                default: <script>.html) — guide: doc/claude/WEB_APPS.md"
    );
    println!(
        "  --host-provided               with --html: you drive the emitted wasm from your own"
    );
    println!("                                JS host, so an import loft's page shim lacks is a");
    println!("                                warning, not a refusal (alias: --no-host-check)");
    println!(
        "  --names                       with --html: keep the wasm name section, so the frames"
    );
    println!("                                a browser prints for a trap resolve to function");
    println!("                                names instead of bare indices (a larger page)");
    println!("  targets [<target>]            which stdlib builtins are NOT available on a target");
    println!("                                (ask before designing, not after the build fails)");
    println!(
        "  --tests [dir]                 discover and run fn test*() functions in .loft files"
    );
    println!("                                recursively (default dir: current directory)");
    println!("  --tests file.loft             run all tests in a single file");
    println!("  --tests file.loft::name       run a single test function");
    println!("  --tests file.loft::{{a,b}}      run specific test functions");
    println!(
        "  --tests --native              like --tests but compile each file to native Rust and"
    );
    println!("                                run the binary (skips @EXPECT_FAIL tests)");
    println!("  --no-warnings                 suppress warnings (in run mode and --tests output)");
    println!(
        "  --explain                     under each diagnostic that has one, print what to
                                write instead — the fix, what it needs you to confirm,
                                and the capability it uses.  Shows only; applies nothing."
    );
    println!(
        "  fix [--apply] <file…>         check each suggested fix by APPLYING it and re-running
                                the analysis, and report what that measured.  --apply writes
                                the ones that are mechanical and verified; a fix resting on a
                                condition only you can affirm is reported, never written."
    );
    println!(
        "  --deny-warnings               under --tests/`loft test`, fail any file with an
                                unexpected warning.  LOFT_DENY_WARNINGS=1 as env equivalent.
                                Used by extracted library chunks' CI to lock in cleanliness."
    );
    println!(
        "  --deps[=direct|=transitive]   under `loft test`, also run `loft test` in every
                                dependency directory listed in loft.toml.  Default is
                                =transitive; =direct walks only first-level deps.  A dep
                                resolves to a path-form dep `{{ path = \"...\" }}`, a sibling
                                directory, or a version pinned by loft.lock (installed in
                                ~/.loft/registry).  Returns non-zero if the host project's
                                tests OR any dep's tests fail."
    );
    println!(
        "  --lock=PATH                   with --deps, resolve registry deps through THIS
                                lockfile instead of the project's loft.lock, so a candidate
                                lock can be tested before it is committed.  While given, it
                                outranks a sibling directory of the same name."
    );
    println!(
        "  --strict-deps                 with --deps, hold dependencies to the same warning
                                bar as the project itself.  Off by default: lint debt
                                inside a package you do not own is not your build's
                                failure."
    );
    println!(
        "  --skip=name[,name]            with --deps, do not run these packages' tests
                                (known-broken on this platform, say).  Their own
                                dependencies are still walked, so nothing reachable only
                                through a skipped package is silently dropped."
    );
    println!(
        "  --check                       parse and compile only; report errors without running
                                can be combined with --native to also verify rustc compilation"
    );
    println!();
    println!("Subcommands:");
    println!("  repl                          start the interactive REPL (also: bare `loft`)");
    println!("                                resumes your last session automatically;");
    println!("                                --fresh starts clean (ignores the saved session)");
    println!("  debug <file>:<line>           run <file> under the debugger, breaking at <line>");
    println!(
        "                                (inspect/edit/step the live frame; :help at the prompt)"
    );
    println!("  check <file>                  same as --check <file>");
    println!("  fmt [--check|--write] <file…> format loft source (parser-driven, written in loft)");
    println!("                                default prints; --write rewrites in place; --check");
    println!(
        "                                exits non-zero if unformatted (CI gate); `-` = stdin"
    );
    println!("  symbols <file> [--json]       list a file's top-level definitions (outline)");
    println!("  def <name> [file] [--json]    signature + doc + location of a symbol by name");
    println!("                                (free fn / type / const + every `Type.name` method)");
    println!("  hover <file> <ln> <col>       the symbol under a cursor (1-based); add --json");
    println!("  tag <@TAG> [--json]           what the tracker index knows about a tag");
    println!("                                (@F/@I feature, @P problem, @PLN/@GH issue)");
    println!("  refs <name> [root] [--json]   every occurrence of an identifier in the .loft tree");
    println!(
        "  sandbox-check <file>          report the @PLN86 sandbox admission verdict and STOP"
    );
    println!("                                (Admitted / Rejected + diagnostics; never executes)");
    println!("  build [target...]             build the project's declared / default targets");
    println!("                                build            — build [build] default-targets");
    println!("                                build html wasi  — build the named targets");
    println!("                                (targets: native | html | wasi | [build.target.*])");
    println!("  check [target...]             build + run the declared [[test]] phase (the gate)");
    println!(
        "                                (in a project; `check <file.loft>` compile-checks it)"
    );
    println!("  test [target]                 run package tests (requires loft.toml in cwd)");
    println!("                                test         — run all tests in tests/");
    println!("                                test draw    — run tests/draw.loft");
    println!("                                test draw::f — run a single test function");
    println!("  install [target]              resolve this project's [dependencies]");
    println!("                                install          — every dep the manifest declares");
    println!("                                install name     — download latest from registry");
    println!("                                install name@v   — download specific version");
    println!("                                install .        — install THIS package into");
    println!("                                                   ~/.loft/lib/ for global use");
    println!("                                install /p       — install the package at /p");
    println!("  pin <script.loft>             pin every registry library the script uses");
    println!("                                writes <script>.loft.lock next to the script;");
    println!("                                subsequent runs use the pinned versions");
    println!(
        "  self-update [--dry-run]       report whether a newer release is published for this"
    );
    println!("                                platform (resolve + report only; downloading is not");
    println!("                                implemented yet — nothing is fetched or changed)");
    println!(
        "    --from <dir> [--force]      install an unpacked release bundle you already have,"
    );
    println!("                                with no registry and no network — for anyone who");
    println!(
        "                                cannot or will not compile loft.  Checked against the"
    );
    println!("                                bundle's own manifests; --force installs regardless");
    println!("  verify-self                   check this installation against the manifests its");
    println!(
        "                                release bundle shipped (SHA256SUMS), and the\n\
         signed registry index"
    );
    println!(
        "                                — detects corruption and partial upgrades; read-only.\n\
         Exits 0 verified, 1 mismatch, 2 not a release bundle (nothing checked)"
    );
    println!("  list-installed                list every registry package installed locally");
    println!("                                (from ~/.loft/registry/), annotated with sha256");
    println!("                                + size + index status (active / yanked / orphan)");
    println!("  api [name]                    discover library APIs without leaving the shell:");
    println!("                                api            — list libraries reachable from here");
    println!("                                api <name>     — print its public API surface");
    println!(
        "                                api --registry [--refresh] — the whole installable catalog"
    );
    println!("                                (pub signatures + doc comments, bodies stripped)");
    println!("  audit                         check every installed package against the");
    println!("                                advisory feed; exit 0 if clean, 1 if any low/bug,");
    println!("                                2 if any high, 3 if any security_critical");
    println!("  update [pkg]                  refresh lockfile to latest active versions");
    println!("                                that satisfy each dep's loft.toml range;");
    println!("                                --dry-run reports changes without writing;");
    println!("                                --check exits 1 if any updates are available");
    println!("  bundle export [opts] <dir>    write a self-contained offline bundle of");
    println!("                                registry packages (--all or --packages X,Y,Z);");
    println!("                                ship via USB/scp; import on the target machine");
    println!("  bundle import <dir>           install a bundle into ~/.loft/registry/");
    println!("                                (verifies sha256 + signature per artifact)");
    println!("  publish [pkg]                 author helper: repackage locally + verify the");
    println!("                                GitHub release exists + emit the index.json");
    println!("                                entry to paste into a registry PR");
    println!("  new <name> [opts]             scaffold a fresh library: loft.toml + src/ +");
    println!("                                tests/ + README; --native adds native/ skeleton;");
    println!("                                --chunk adds .github/workflows/library-ci.yml");
    println!("  yank <pkg>@<ver> --severity   author helper: draft a registry yank PR — emits");
    println!("    <tier> --advisory <id>      the typed `status` entry for index.json + the");
    println!("    --summary <text>            advisories.json row, cross-referenced");
    println!("    --affected <range>");
    println!("    --fixed-in <ver>");
    println!("  registry <subcommand>         manage the local package registry");
    println!(
        "                                sync             — pull latest registry from source URL"
    );
    println!(
        "                                check            — report updates, deprecations, yanks"
    );
    println!("                                list             — browse all packages in registry");
    println!("                                list --installed — show only installed packages");
    println!("  cache <subcommand>            the on-disk build caches (~/.loft)");
    println!(
        "                                status           — footprint, and what is reclaimable"
    );
    println!("                                prune            — drop what this loft cannot reuse");
    println!("                                prune --all      — drop the live generation too");
    println!("                                warm [--from <dir>] [pkg…]");
    println!(
        "                                                 — build what a run will need, first"
    );
    println!("  generate [path]               generate Rust stubs for #native declarations");
    println!("                                writes native/src/generated.rs in the package");
    println!("  package [path]                build a publishable <pkg>-<version>.tar.gz");
    println!(
        "    --tarball-only              build the tarball only — no registry entry, and none"
    );
    println!("                                of the checks that registering requires");
    println!("                                prints sha256 + size + the registry index entry");
    println!("                                (PKG.REG R1 — see doc/claude/PKG_REGISTRY.md)");
    println!("  build-native [pkg-dir]        build the package's native cdylib for this host");
    println!("                                + print its path, triple, and loft-ffi fp, so CI");
    println!(
        "                                can publish it as a prebuilt/<triple>/ binary (@PLN21)"
    );
    println!("  search [query]                client-side search of the package registry");
    println!(
        "                                matches name / description / categories (case-insensitive);"
    );
    println!("                                no query lists all; --json for machine output");
    println!("                                (PKG.REG R8)");
    println!(
        "  info <name>                   per-package details (versions, latest, deps, homepage)"
    );
    println!("                                (PKG.REG R8)");
    println!();
    println!("install flags (PKG.REG R4):");
    println!("  --refresh                     force re-fetch the registry index");
    println!("  --offline                     use cache only; fail if anything's missing");
    println!("  --prerelease                  resolve prerelease versions too");
    println!("  --allow-unsigned              proceed even if the index has no valid signature");
    println!("                                (default while no trust root is embedded;");
    println!("                                 flips after src/registry_keys.rs is populated)");
    println!(
        "  --require-signature           refuse to proceed unless the index signature verifies"
    );
    println!("  doc [path|library] [-o dir]   generate HTML documentation for a package");
    println!("                                doc           — the package in the cwd");
    println!("                                doc lib/pkg   — a package directory");
    println!(
        "                                doc graphics  — an installed library, into ~/.loft/doc"
    );
    println!("                                output: <pkg>/doc/*.html");
}

fn handle_generate_log_config(path_opt: Option<&str>) {
    let content = logger::generate_config();
    match path_opt {
        Some(path) => {
            if let Err(e) = std::fs::write(path, content) {
                println!("Error writing config to '{path}': {e}");
                std::process::exit(1);
            }
            println!("Log config written to: {path}");
        }
        None => {
            print!("{content}");
        }
    }
}

#[allow(clippy::too_many_lines)]
/// PKG.2: Install a local package to ~/.loft/lib/<name>/.
///
/// Reads loft.toml from `pkg_path`, copies src/*.loft and loft.toml to
/// the user's library directory.  The package is then available via `use <name>;`.
/// Walk the current project's dep tree and invoke `loft test` in each dep's
/// directory.  Direct mode walks only `manifest.dependencies` of the cwd;
/// transitive mode recurses into every walked dep's own `loft.toml`.  Returns 1
/// if any dep failed, 0 otherwise.
///
/// # How a dependency becomes a directory
///
/// Four sources, tried in this order, and the order is the contract:
///
/// 1. **A path dep** (`{ path = ".." }`) — an explicit local override, so it
///    outranks everything.
/// 2. **An explicit `--lock=PATH` pin** — pre-flighting a candidate lockfile is
///    the whole purpose of that flag, so while it is given the lock is the
///    authority for every package it names.
/// 3. **A sibling directory** (`../<name>/loft.toml`) — inside a multi-package
///    repo this is the WORKING COPY, which is what someone running the suite
///    there means by "the dependency".
/// 4. **The project's own `loft.lock`** — read when present, and only where the
///    three above found nothing.
///
/// Ranking the implicit lock last is what keeps this additive: with no lockfile
/// anywhere, every edge that resolved before resolves the same way, and a repo
/// whose siblings already resolved keeps testing its working copies rather than
/// silently switching to published tarballs out of the cache.
///
/// A registry dep with no pin anywhere is still reported and skipped — it is the
/// one case that cannot be resolved, and saying so is the point.
fn run_dep_tests(
    transitive: bool,
    native_mode: bool,
    lock_override: Option<&str>,
    skip: &[String],
    strict_deps: bool,
) -> i32 {
    use std::collections::{BTreeMap, HashSet, VecDeque};
    use std::path::PathBuf;
    let cwd = std::env::current_dir().unwrap_or_default();
    let loft_bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("loft"));

    // `name -> version` for every package a lockfile pins, plus whether that
    // lock was ASKED for.  An unreadable `--lock` is a hard error: the flag says
    // "resolve against this file", and walking on with an empty pin set would
    // answer a different question while looking like success.
    let (lock_pins, lock_is_authoritative): (BTreeMap<String, String>, bool) = match lock_override {
        Some(raw) => {
            let path = PathBuf::from(raw);
            match loft::lockfile::read_lockfile(&path) {
                Ok(Some(l)) => (
                    l.packages
                        .into_iter()
                        .map(|pkg| (pkg.name, pkg.version))
                        .collect(),
                    true,
                ),
                // Unreachable in practice — the flag parser already read this
                // file.  Kept because the caller folds any RETURNED code into
                // "tests failed", so the honest exit for a lock that vanished
                // mid-run is the same usage code the parser gives.
                Ok(None) => {
                    eprintln!("  --lock: no lockfile at {}", path.display());
                    std::process::exit(2);
                }
                Err(e) => {
                    eprintln!("  --lock: cannot read {} — {e}", path.display());
                    std::process::exit(2);
                }
            }
        }
        None => match loft::lockfile::read_lockfile(&cwd.join("loft.lock")) {
            Ok(Some(l)) => (
                l.packages
                    .into_iter()
                    .map(|pkg| (pkg.name, pkg.version))
                    .collect(),
                false,
            ),
            // A missing lock is ordinary; an unreadable one is worth a word, but
            // not fatal when nobody asked for it.
            Ok(None) => (BTreeMap::new(), false),
            Err(e) => {
                eprintln!("  --deps: ignoring unreadable loft.lock — {e}");
                (BTreeMap::new(), false)
            }
        },
    };

    // Where a locked version lives once installed.  `None` when the lock names a
    // version this box has never extracted — reported at the call site, because
    // "pinned but not installed" is a different answer from "not pinned".
    // Without the registry feature nothing consumes the pins, but the read above still
    // happens so that `--lock` is validated identically on every build.
    #[cfg(not(feature = "registry"))]
    let _ = &lock_pins;

    // `registry_index` is behind the `registry` feature, and a build without it has
    // no package cache to point at — so there, a lock pin resolves to nothing and the
    // dep falls through to the "cannot resolve" report rather than to a path that
    // cannot exist.  Deriving `~/.loft/registry` by hand here instead would put a
    // second copy of that rule in the tree, which is how the two drift.
    #[cfg(feature = "registry")]
    let locked_dir = |name: &str| -> Option<PathBuf> {
        let version = lock_pins.get(name)?;
        Some(loft::registry_index::cache_dir().join(format!("{name}-{version}")))
    };
    #[cfg(not(feature = "registry"))]
    let locked_dir = |_name: &str| -> Option<PathBuf> { None };

    // Every `--skip` name that never matched anything in the walk.  A skip that
    // matches nothing is almost always a typo, and a typo that silently widens
    // the run is the wrong way for this flag to fail.
    let mut skip_unused: HashSet<String> = skip.iter().cloned().collect();

    // Resolve a dep name + value to a directory, in the four-source order the
    // doc-comment above states.  `locked` is the lockfile's answer for this name
    // (already turned into a path); `lock_first` is whether an explicit `--lock`
    // makes that answer outrank the sibling directory.
    let resolve_dep = |name: &str,
                       value: &str,
                       from_pkg: &std::path::Path,
                       locked: Option<PathBuf>,
                       lock_first: bool|
     -> Option<PathBuf> {
        // 1 — an explicit path dep.
        if let Some(p) = loft::manifest::extract_path_dep(value) {
            let candidate = from_pkg.join(p);
            if candidate.join("loft.toml").exists() {
                return Some(portable_path::plain_canonical(&candidate));
            }
        }
        let usable = |d: &Option<PathBuf>| -> Option<PathBuf> {
            d.as_ref()
                .filter(|c| c.join("loft.toml").exists())
                .map(|c| portable_path::plain_canonical(c))
        };
        // 2 — an asked-for lock outranks the working copy.
        if lock_first && let Some(d) = usable(&locked) {
            return Some(d);
        }
        // 3 — the sibling working copy.
        let sibling = from_pkg.join("..").join(name);
        if sibling.join("loft.toml").exists() {
            return Some(portable_path::plain_canonical(&sibling));
        }
        // 4 — the project's own lock, filling what nothing above reached.
        usable(&locked)
    };

    let mut queue: VecDeque<PathBuf> = VecDeque::new();
    queue.push_back(cwd.clone());
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut total_fail: i32 = 0;
    let mut tested = 0usize;

    while let Some(pkg) = queue.pop_front() {
        if !visited.insert(pkg.clone()) {
            continue;
        }
        let manifest_path = pkg.join("loft.toml");
        let Some(manifest) = loft::manifest::read_manifest(manifest_path.to_str().unwrap_or(""))
        else {
            continue;
        };
        for (dep_name, dep_value) in &manifest.dependencies {
            let locked = locked_dir(dep_name);
            let Some(dep_dir) = resolve_dep(
                dep_name,
                dep_value,
                &pkg,
                locked.clone(),
                lock_is_authoritative,
            ) else {
                if pkg == cwd {
                    // Two different answers, and they need different words: a
                    // package the lock PINS but this box never installed is one
                    // `loft install` away, while one the lock does not name at
                    // all cannot be resolved by any amount of installing.
                    if let Some(d) = locked {
                        eprintln!(
                            "  --deps: skipping {dep_name} (locked, but not installed at {}) — run `loft install`",
                            d.display()
                        );
                    } else {
                        eprintln!(
                            "  --deps: skipping {dep_name} (no path-dep and no lockfile pin — `loft install` writes one)"
                        );
                    }
                }
                continue;
            };
            if visited.contains(&dep_dir) {
                continue;
            }
            // A skipped package still has its dependencies WALKED — only its own
            // tests are dropped.  Skipping the subtree instead would silently
            // drop every package reachable only through this one, and a skip is
            // asked for because a package is broken here, not because the things
            // it depends on are.
            if skip.iter().any(|s| s == dep_name) {
                skip_unused.remove(dep_name);
                println!("  --deps: skipping {dep_name} (--skip)");
                if transitive {
                    queue.push_back(dep_dir);
                }
                continue;
            }
            if dep_dir.join("tests").is_dir() {
                tested += 1;
                let mut cmd = std::process::Command::new(&loft_bin);
                cmd.arg("test").current_dir(&dep_dir);
                if native_mode {
                    cmd.arg("--native");
                }
                // A dep's warnings are suppressed by default: a consumer should
                // not be blocked by lint debt inside a package it does not own,
                // and the project's OWN tests still honour `LOFT_DENY_WARNINGS`.
                // `--strict-deps` opts back in, for the one who DOES own them.
                // (Errors surface through the exit code either way.)
                //
                // This comment used to promise a `LOFT_DENY_WARNINGS_DEPS=1`
                // opt-in that was read nowhere — the flag is the opt-in, and now
                // it exists.
                if !strict_deps {
                    cmd.arg("--no-warnings");
                    // `--no-warnings` only silences the PRINTING; whether a
                    // warning is fatal is decided separately, and
                    // `LOFT_DENY_WARNINGS` is read from the environment the
                    // child inherits.  So without this the promise above held
                    // only for a consumer who happened not to export it: with
                    // `LOFT_DENY_WARNINGS=1` set, a dep's lint debt failed the
                    // consumer's run — the exact thing the default exists to
                    // prevent.  Measured, not reasoned: exit 1 where 0 was owed.
                    cmd.env("LOFT_DENY_WARNINGS", "0");
                }
                let label = dep_dir
                    .file_name()
                    .map(|s| s.to_string_lossy().to_string())
                    .unwrap_or_else(|| dep_name.clone());
                println!("  --deps: testing {label}");
                let status = cmd.status();
                let ok = status.as_ref().map(|s| s.success()).unwrap_or(false);
                if !ok {
                    eprintln!("  --deps: FAILED {label}");
                    total_fail += 1;
                }
            }
            if transitive {
                queue.push_back(dep_dir);
            }
        }
    }

    if tested > 0 {
        println!("  --deps: {tested} dep(s) tested, {total_fail} failed");
    } else {
        println!("  --deps: no deps with tests/ directory found");
    }
    // A `--skip` name that matched nothing is nearly always a misspelling, and
    // the way it fails otherwise is silent: the package it was meant to exclude
    // runs anyway, and the run looks exactly like one where the flag worked.
    if !skip_unused.is_empty() {
        let mut unused: Vec<&String> = skip_unused.iter().collect();
        unused.sort();
        let names: Vec<&str> = unused.iter().map(|s| s.as_str()).collect();
        eprintln!(
            "  --deps: --skip named {} which no dependency matched — check the spelling",
            names.join(", ")
        );
    }
    i32::from(total_fail > 0)
}

fn install_package(pkg_path: &std::path::Path) {
    let manifest_file = pkg_path.join("loft.toml");
    if !manifest_file.exists() {
        println!("loft install: no loft.toml found in {}", pkg_path.display());
        std::process::exit(1);
    }
    // loft#966 — the name is the MANIFEST's, not the checkout directory's.  A package
    // whose directory differs from `[package] name` installed under a name nothing else
    // refers to: `loft api` kept reporting the dependency unresolved, because the copy
    // was filed under a name no `use` can reach.  The directory is only the fallback for
    // a manifest that declares no name.
    let pkg_name = loft::manifest::read_manifest(&manifest_file.to_string_lossy())
        .and_then(|m| m.name)
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            pkg_path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string()
        });
    if pkg_name.is_empty() {
        println!("loft install: cannot determine package name from path");
        std::process::exit(1);
    }
    // `pkg_name` is about to become a directory component under `~/.loft/lib/`, and
    // it came from the manifest — which, for a package fetched from anywhere, is a
    // string somebody else chose.  Without this a `name = "../../x"` writes the whole
    // package tree wherever it points: the package would be choosing where it lands.
    // `loft new` enforces the same rule when a package is created, so no name that
    // could legitimately reach here is refused by it.
    if !loft::libscan::is_valid_package_name(&pkg_name) {
        println!(
            "loft install: `{pkg_name}` is not a usable package name (lowercase ascii, \
             digits and `_` only) — the manifest's `[package] name` decides the install \
             directory, so it has to be a plain name"
        );
        std::process::exit(1);
    }
    // Target: ~/.loft/lib/<name>/
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    let target = std::path::Path::new(&home)
        .join(".loft")
        .join("lib")
        .join(&pkg_name);
    // Copy exactly what `loft package` bundles — ONE include rule for both
    // (`package::copy_package_tree`).  A whitelist here re-derived "what a package consists
    // of" a second time, and the two answers disagreed twice: first `native/`, so a local
    // install of an FFI library dropped its `n_*` symbols at link time, then `wasm/`, so a
    // local install of a `[wasm.bridge]` library dropped the bridge — and since
    // `~/.loft/lib/<name>` is searched BEFORE the registry cache, that incomplete copy
    // shadowed a complete registry one, failing every `--html` build against it with an
    // error pointing at the library rather than at the install (loft#667).
    let copied = match loft::package_layout::copy_package_tree(pkg_path, &target) {
        Ok(n) => n,
        Err(e) => {
            println!("loft install: cannot copy {}: {e}", pkg_path.display());
            std::process::exit(1);
        }
    };
    println!(
        "installed {pkg_name} ({copied} files) → {}",
        target.display()
    );
}

/// loft#966 — bare `loft install`: resolve what the manifest DECLARES.
///
/// The npm/cargo reading of the verb, and the one `loft api` names when it reports a
/// dependency unresolved. Bare install used to install the PROJECT into
/// `~/.loft/lib/<name>`, so the tool's only hint pointed at the one command that does
/// not fetch a dependency — and the copy it leaves behind shadows the registry copy of
/// the same name (loft#667). `loft install .` still installs this package; that spelling
/// always meant it.
///
/// A path dependency is resolved BY ITS PATH and needs no install, so it is reported only
/// when the path does not lead to a package — which is the one thing a reader can act on.
#[cfg(feature = "registry")]
fn install_manifest_dependencies(opts: &loft::install::InstallOptions) {
    use loft::install::{InstallReport, format_report, install_one};

    let cwd = std::env::current_dir().unwrap_or_default();
    let manifest_file = cwd.join("loft.toml");
    if !manifest_file.exists() {
        eprintln!("loft install: no loft.toml in {}", cwd.display());
        eprintln!("  loft install <pkg>   install a package from the registry");
        eprintln!("  loft install .       install a package directory into ~/.loft/lib");
        std::process::exit(1);
    }
    let manifest =
        loft::manifest::read_manifest(&manifest_file.to_string_lossy()).unwrap_or_default();
    let project = manifest
        .name
        .clone()
        .unwrap_or_else(|| "this package".into());

    if manifest.dependencies.is_empty() {
        // Nothing to resolve — and the one reader who is surprised by that is the one who
        // meant the old behaviour, so name its spelling here rather than on every run.
        println!(
            "{project} declares no dependencies.  \
             (`loft install .` installs this package into ~/.loft/lib.)"
        );
        return;
    }

    let mut merged = InstallReport {
        installed: Vec::new(),
        skipped_cached: Vec::new(),
        surface: Vec::new(),
    };
    let mut unresolved_paths: Vec<String> = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    for (name, value) in &manifest.dependencies {
        if let Some(rel) = loft::manifest::extract_path_dep(value) {
            // A path dep with a `version` too is still resolved by path; the version is a
            // publish-time claim, not something to fetch.
            let dir = cwd.join(rel);
            if !dir.join("loft.toml").exists() {
                unresolved_paths.push(format!("  {name}  NOT FOUND at `{rel}` — check the path"));
            }
            continue;
        }
        let req = loft::manifest::extract_version_req(value);
        match install_one(name, req, opts) {
            Ok(report) => {
                merged.installed.extend(report.installed);
                merged.skipped_cached.extend(report.skipped_cached);
                merged.surface.extend(report.surface);
            }
            // Keep going: one unreachable package should not hide the state of the rest,
            // which is what the reader needs to decide what to do next.
            Err(e) => failed.push(format!("  {name}  {e}")),
        }
    }

    if !merged.installed.is_empty() || !merged.skipped_cached.is_empty() {
        print!("{}", format_report(&merged));
    }
    if !unresolved_paths.is_empty() {
        eprintln!("loft install: path dependencies that do not lead to a package:");
        for line in &unresolved_paths {
            eprintln!("{line}");
        }
    }
    if !failed.is_empty() {
        eprintln!("loft install: could not resolve:");
        for line in &failed {
            eprintln!("{line}");
        }
    }
    if !unresolved_paths.is_empty() || !failed.is_empty() {
        std::process::exit(1);
    }
    // PKG.STUB — the in-project API stubs follow the lockfile this install just wrote,
    // exactly as they do for `loft install <pkg>`.
    write_api_stubs(&cwd.join("loft.lock"), &cwd);
}

/// @PLN143 arc D — write the minimal `loft.toml` that makes `dir` a package, so the lock
/// `loft install` is about to write has a root that governs it.
///
/// Minimal on purpose: a `[package]` name and version, and nothing else. This directory
/// is where someone installed a dependency, not a library being authored — `loft new`
/// writes the library skeleton, and inventing an `entry` here would claim a source file
/// that does not exist.
///
/// The name is the directory's own, folded to what a package name may hold (lowercase
/// ascii, digits, `_`); a directory whose name yields nothing usable is called `app`,
/// because the name is a label for the manifest, not an identity anyone publishes.
///
/// Answers the name written, or `None` when the file could not be written — the install
/// itself still stands, and the missing declaration is visible in the next run.
#[cfg(feature = "registry")]
fn manifest_for_new_package(dir: &std::path::Path, manifest: &std::path::Path) -> Option<String> {
    let raw = dir.file_name().and_then(|s| s.to_str()).unwrap_or_default();
    let folded: String = raw
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .skip_while(|c| !c.is_ascii_alphabetic() && *c != '_')
        .collect();
    let name = if folded.is_empty() || loft::libscan::is_reserved_package_name(&folded) {
        "app".to_string()
    } else {
        folded
    };
    let body = format!(
        "# Written by `loft install`: this directory's dependency declaration.\n\
         [package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[dependencies]\n"
    );
    std::fs::write(manifest, body).ok().map(|()| name)
}

/// REG.2: Install a package from the registry by name (optionally with `@version`).
///
/// PKG.REG R4 (2026-05-24): the `loft install <name>` entry point in
/// `main` calls [`install_from_registry_with_opts`] directly with the
/// parsed flag bag.  When `LOFT_LEGACY_REGISTRY` is set, that helper
/// delegates to [`install_from_registry_legacy`] (the older
/// text-format flow).
#[cfg(feature = "registry")]
fn install_from_registry_with_opts(args: &[String], opts: &loft::install::InstallOptions) {
    use loft::install::{format_report, install_one};

    if std::env::var("LOFT_LEGACY_REGISTRY").is_ok() {
        for arg in args {
            install_from_registry_legacy(arg);
        }
        return;
    }

    if args.is_empty() {
        eprintln!("loft install: no package name given");
        std::process::exit(1);
    }

    // Process each arg sequentially.  Each `install_one` invocation
    // merges its packages into the cwd's `loft.lock`, so the final
    // lockfile lists every package the user asked for.
    for arg in args {
        let (name, version) = if let Some((n, v)) = arg.split_once('@') {
            (n, Some(v))
        } else {
            (arg.as_str(), None)
        };
        match install_one(name, version, opts) {
            Ok(report) => {
                print!("{}", format_report(&report));
                // loft#968 — declare it.  Only `loft.lock` was written, so nothing in the
                // project distinguished a dependency from a package that happened to be
                // installed on the box: dropping the `[dependencies]` line changed
                // nothing that could be observed.  An explicit version stays exactly as
                // asked; without one, the compatible range around what was resolved —
                // `loft.lock` is where the exact pin belongs.
                let requirement = version.map_or_else(
                    || {
                        report
                            .installed
                            .iter()
                            .chain(report.skipped_cached.iter())
                            .find(|(n, _)| n == name)
                            .map_or_else(|| "*".to_string(), |(_, v)| format!("^{v}"))
                    },
                    ToString::to_string,
                );
                let cwd = std::env::current_dir().unwrap_or_default();
                let manifest = cwd.join("loft.toml");
                // @PLN143 arc D — install into a directory that is not a package, and the
                // verb CREATES the declaration it needs. Without a `loft.toml` the walk-up
                // finds no root, so the lock this install writes governs nothing: an
                // explicit `loft install <pkg>@<version>` would be silently ignored on the
                // next run, which is worse than the stray-lockfile defect it replaces.
                if !manifest.exists() {
                    if let Some(pkg) = manifest_for_new_package(&cwd, &manifest) {
                        println!("  created loft.toml (package `{pkg}`)");
                    }
                }
                if manifest.exists()
                    && loft::manifest::record_dependency(
                        &manifest.to_string_lossy(),
                        name,
                        &requirement,
                    )
                {
                    println!("  declared in loft.toml: {name} = \"{requirement}\"");
                }
            }
            Err(e) => {
                eprintln!("loft install: {e}");
                std::process::exit(1);
            }
        }
    }
}

/// PKG.REG R8 — `loft search [query]`: client-side filter against the cached
/// index (reuses `loft install`'s fetch/verify path via `install::load_index`).
/// Ranks hits exact-name > name-prefix > description/category; marks a package
/// whose latest version declares lazy-load `triggers` with `⚡auto-use`; and
/// prints a `loft install` hint under each hit when a query is given.  `json`
/// emits the same result set as a JSON array for tooling.
#[cfg(feature = "registry")]
fn search_registry(query: &str, json: bool) {
    use loft::install::InstallOptions;
    use loft::registry_index;

    let opts = InstallOptions {
        allow_unsigned: true,
        ..Default::default()
    };
    let loaded = match loft::install::load_index_reporting(&opts) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("loft search: {e}");
            std::process::exit(1);
        }
    };
    if loaded.stale_fallback {
        eprintln!("loft search: registry unreachable — showing cached index");
    }
    let index = loaded.index;

    // S8 — the stdlib's function-level surface, extracted from the binary's
    // EMBEDDED `default/*.loft` at search time (one source of truth shared with
    // the WASM runtime, no disk dependency): stdlib functions appear as hits
    // identical in shape to a library's, and never bloat the fetched index.
    let stdlib: Vec<registry_index::ApiItem> = loft::stdlib_sources::STDLIB_SOURCES
        .iter()
        .flat_map(|(_, content)| loft::documentation::extract_api_items(content))
        .collect();

    let q = query.to_ascii_lowercase();
    let results = registry_index::search_results(&index, &stdlib, &q);

    if json {
        println!(
            "{}",
            loft::json::to_json_string(&search_results_json(&results))
        );
        return;
    }

    if results.is_empty() {
        println!("No packages or functions match `{query}`.");
        return;
    }
    let querying = !q.is_empty();
    for r in &results {
        // S9 — package header (stdlib reads "built in", no version/install).
        if r.is_stdlib {
            println!("stdlib (built in)");
        } else {
            let marker = if r.auto_use { " ⚡auto-use" } else { "" };
            let mut line = format!("{} {}{marker}", r.name, r.version);
            if let Some(d) = r.description.as_deref().filter(|d| !d.is_empty()) {
                line.push_str(" — ");
                line.push_str(d);
            }
            if !r.categories.is_empty() {
                line.push_str(&format!("  ({})", r.categories.join(", ")));
            }
            println!("{line}");
        }
        // The matching functions: what it does (doc) + how to call it (sig).
        for item in &r.fns {
            println!("    {}", item.sig);
            // Display only the first line of the (now full) doc paragraph; the
            // whole paragraph stays searchable and travels in `--json`.
            if let Some(summary) = item.doc.lines().next().filter(|l| !l.is_empty()) {
                println!("        {summary}");
            }
        }
        // How to get it.
        if querying {
            if r.is_stdlib {
                println!("    built in — use directly, no install");
            } else {
                println!("    → loft install {}", r.name);
            }
        }
    }
}

/// One `loft search --json` record (S9): everything an agent needs to decide
/// and call — `source` (`stdlib`|`registry`), `package`, `version`, `signature`
/// (null for a metadata-only hit), one-line `doc`, and `get` (the install line,
/// or "built in" for the stdlib).
#[cfg(feature = "registry")]
fn search_record(
    source: &str,
    name: &str,
    version: &str,
    signature: loft::json::Parsed,
    doc: loft::json::Parsed,
    get: &str,
) -> loft::json::Parsed {
    use loft::json::Parsed;
    Parsed::Object(vec![
        ("source".to_string(), 0, Parsed::Str(source.to_string())),
        ("package".to_string(), 0, Parsed::Str(name.to_string())),
        ("version".to_string(), 0, Parsed::Str(version.to_string())),
        ("signature".to_string(), 0, signature),
        ("doc".to_string(), 0, doc),
        ("get".to_string(), 0, Parsed::Str(get.to_string())),
    ])
}

/// Build the `loft search --json` payload (S9): a FLAT array of function-level
/// records (see [`search_record`]).  Replaces the S0–S5 per-package shape: each
/// matching function is its own record carrying its package context; a
/// metadata-only hit contributes one `signature: null` record (description as
/// `doc`) so nothing is dropped.
#[cfg(feature = "registry")]
fn search_results_json(results: &[loft::registry_index::SearchResult]) -> loft::json::Parsed {
    use loft::json::Parsed;
    let mut arr: Vec<Parsed> = Vec::new();
    for r in results {
        let source = if r.is_stdlib { "stdlib" } else { "registry" };
        let get = if r.is_stdlib {
            "built in — use directly".to_string()
        } else {
            format!("loft install {}", r.name)
        };
        if r.fns.is_empty() {
            let doc = r.description.clone().map_or(Parsed::Null, Parsed::Str);
            arr.push(search_record(
                source,
                &r.name,
                &r.version,
                Parsed::Null,
                doc,
                &get,
            ));
        } else {
            for item in &r.fns {
                arr.push(search_record(
                    source,
                    &r.name,
                    &r.version,
                    Parsed::Str(item.sig.clone()),
                    Parsed::Str(item.doc.clone()),
                    &get,
                ));
            }
        }
    }
    Parsed::Array(arr)
}

/// The `api` array for one source dir, as `[{ "sig": …, "doc": … }, …]` — the
/// shared shape the index `api` field carries and `loft api --json` emits, so
/// the registry CI can re-derive it from source and reject a pasted mismatch
/// (S7-CI).  Used by `loft publish` (the entry) and `loft api --json`.
#[cfg(feature = "registry")]
fn api_items_json(items: &[loft::registry_index::ApiItem]) -> loft::json::Parsed {
    use loft::json::Parsed;
    Parsed::Array(
        items
            .iter()
            .map(|item| {
                Parsed::Object(vec![
                    ("sig".to_string(), 0, Parsed::Str(item.sig.clone())),
                    ("doc".to_string(), 0, Parsed::Str(item.doc.clone())),
                ])
            })
            .collect(),
    )
}

/// PKG.REG R8 — `loft info <name>`: full info for one package.
/// Prints homepage, categories, available versions (yanked /
/// prerelease tags inline), deps for the latest.
#[cfg(feature = "registry")]
fn package_info(name: &str) {
    use loft::install::InstallOptions;
    use loft::registry_index;

    let opts = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: None,
    };
    let loaded = match loft::install::load_index_reporting(&opts) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("loft info: {e}");
            std::process::exit(1);
        }
    };
    if loaded.stale_fallback {
        eprintln!("loft info: registry unreachable — showing cached index");
    }
    let index = loaded.index;

    let Some(pkg) = index.packages.get(name) else {
        eprintln!("loft info: package `{name}` not found in registry");
        std::process::exit(1);
    };
    println!("{name}");
    if let Some(d) = &pkg.description {
        println!("  description: {d}");
    }
    if let Some(h) = &pkg.homepage {
        println!("  homepage:    {h}");
    }
    if !pkg.categories.is_empty() {
        println!("  categories:  {}", pkg.categories.join(", "));
    }
    let latest = registry_index::find_best_version(pkg, "*", false);
    if let Some(v) = latest {
        println!("  latest:      {}", v.semver);
        if !v.deps.is_empty() {
            println!("  deps:");
            for (n, c) in &v.deps {
                println!("    {n} {c}");
            }
        }
    }
    println!("  versions:");
    for ver in pkg.versions.values() {
        let mut tags = Vec::new();
        if pkg.yanked.iter().any(|y| y == &ver.semver) {
            tags.push("yanked");
        }
        if ver.prerelease {
            tags.push("prerelease");
        }
        let tag_str = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", tags.join(", "))
        };
        println!("    {}{tag_str}", ver.semver);
    }
}

/// PKG.STUB — resolve a library name to a readable package directory, trying
/// the places a consumer's `use` would be served from: an explicit path,
/// project-local `lib/<name>/`, user `~/.loft/lib/<name>/`, then the newest
/// installed registry copy.
#[cfg(feature = "registry")]
fn api_resolve_pkg_dir(name: &str) -> Option<std::path::PathBuf> {
    if name.contains('/') || name == "." {
        let direct = std::path::PathBuf::from(name);
        return direct.join("loft.toml").exists().then_some(direct);
    }
    let project = std::path::PathBuf::from("lib").join(name);
    if project.join("loft.toml").exists() {
        return Some(project);
    }
    let user = loft_home().join("lib").join(name);
    if user.join("loft.toml").exists() {
        return Some(user);
    }
    let mut hits: Vec<(Vec<u32>, std::path::PathBuf)> = loft::registry_index::installed_packages()
        .into_iter()
        .filter(|(n, _, _)| n == name)
        .map(|(_, v, p)| (numeric_version(&v), p))
        .collect();
    hits.sort();
    hits.pop().map(|(_, p)| p)
}

/// `~/.loft/` honouring `LOFT_HOME` (the same base `registry_index::cache_dir`
/// resolves against).
///
/// Deliberately NOT behind the `registry` feature: `cache_areas` (the `loft cache`
/// command) resolves the build cache through it, and that command is unconditional —
/// gating this made a `--no-default-features` build fail to compile.  The `dirs`
/// dependency it uses is unconditional too.
fn loft_home() -> std::path::PathBuf {
    std::env::var_os("LOFT_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join(".loft")
}

/// Where `loft doc <installed-library>` writes: `~/.loft/doc/<name>-<version>`.
///
/// An installed package lives in the immutable registry cache, so its generated docs
/// cannot go beside its source; and the current working directory is not loft's to
/// write to — `loft doc graphics` used to leave a `graphics/` tree in whatever repo
/// the user was standing in, which a later `git add -A` then swept up (loft#911).
fn doc_cache_dir() -> std::path::PathBuf {
    loft_home().join("doc")
}

/// Order-comparable version key; non-numeric parts sort as 0.
#[cfg(feature = "registry")]
fn numeric_version(v: &str) -> Vec<u32> {
    v.split('.').map(|p| p.parse().unwrap_or(0)).collect()
}

/// PKG.STUB — `loft api`: agent-facing library discovery.
///
/// - `loft api` lists every library discoverable from the cwd — project
///   dependencies (`loft.toml`), installed registry packages, user libraries —
///   each with the directory its source lives in.
/// - `loft api <name>` (or a package path) prints that library's public API
///   surface: `pub` signatures + doc comments, bodies stripped.
#[cfg(feature = "registry")]
fn api_command(target: Option<&str>) {
    if let Some(name) = target {
        let Some(dir) = api_resolve_pkg_dir(name) else {
            eprintln!(
                "loft api: no library `{name}` found (project lib/, ~/.loft/lib, installed registry packages)."
            );
            eprintln!(
                "  `loft api` lists what is discoverable; `loft search {name}` queries the registry."
            );
            std::process::exit(1);
        };
        match loft::documentation::render_pkg_api_text(&dir) {
            Ok(text) => print!("{text}"),
            Err(e) => {
                eprintln!("loft api: cannot read {}: {e}", dir.display());
                std::process::exit(1);
            }
        }
        return;
    }

    // No name: one listing of everything reachable from here.
    let manifest_path = std::path::Path::new("loft.toml");
    if manifest_path.exists() {
        let manifest = loft::manifest::read_manifest("loft.toml").unwrap_or_default();
        println!("== project dependencies (loft.toml) ==");
        if manifest.dependencies.is_empty() {
            println!("  (none)");
        }
        for (name, constraint) in &manifest.dependencies {
            match api_resolve_pkg_dir(name) {
                Some(dir) => println!("  {name}  {}", dir.display()),
                // loft#966 — name a command that resolves THIS dependency.  This used to
                // say `run \`loft install\`` for every unresolved dep, and bare
                // `loft install` installs the PROJECT into `~/.loft/lib`; it does not
                // fetch anything the manifest declares.  So the one hint the tool gave
                // was for the one case it does not address — and following it leaves a
                // copy in `~/.loft/lib/<name>` that shadows the registry, which is
                // loft#667.
                //
                // A path dep needs no install at all: it resolves from the path it names
                // (loft#963).  Unresolved means the path is wrong, so say that instead of
                // sending the reader to the registry for a package that is not there.
                None => {
                    if let Some(rel) = loft::manifest::extract_path_dep(constraint) {
                        println!("  {name}  NOT FOUND at `{rel}` — check the path");
                    } else {
                        println!("  {name}  NOT INSTALLED — run `loft install {name}`");
                    }
                }
            }
        }
    }
    let installed = loft::registry_index::installed_packages();
    println!("== installed registry packages ==");
    if installed.is_empty() {
        println!("  (none)");
    }
    for (name, version, path) in installed {
        println!("  {name} {version}  {}", path.display());
    }
    let user_lib = loft_home().join("lib");
    if let Ok(read) = std::fs::read_dir(&user_lib) {
        println!("== user libraries ({}) ==", user_lib.display());
        for ent in read.filter_map(Result::ok) {
            if ent.path().join("loft.toml").exists() {
                println!(
                    "  {}  {}",
                    ent.file_name().to_string_lossy(),
                    ent.path().display()
                );
            }
        }
    }
    println!();
    println!("`loft api <name>` prints a library's public API (signatures + doc comments).");
}

/// `loft api --registry` — list the whole installable catalog from the registry
/// index (so an agent can discover what EXISTS, not just what's installed).
/// Mirrors `loft search`'s trust posture (`allow_unsigned: true`): a *missing*
/// signature is tolerated, but an *invalid* one still hard-fails.
#[cfg(feature = "registry")]
fn api_registry_catalog(refresh: bool) {
    // The catalog is cached with a 1-hour TTL; `--refresh` forces a re-fetch so an
    // agent sees registry changes (new packages, fixed descriptions) immediately
    // rather than waiting out the TTL.
    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh,
        offline: false,
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: None,
    };
    match loft::install::load_index(&opts) {
        Ok(index) => print!("{}", loft::registry_index::render_catalog(&index)),
        Err(e) => {
            eprintln!("loft api --registry: {e}");
            std::process::exit(1);
        }
    }
}

/// PKG.STUB — write `.loft/api/<name>.api` stubs (the public surface of every
/// locked dependency) under `project_dir`.  Called by the lockfile-writing
/// commands, so stub freshness rides the same trigger as `loft.lock` itself:
/// agents exploring the project tree see the dependency APIs without leaving
/// it.  Best-effort: a missing install or unreadable package skips silently —
/// the stub layer must never break an install.
#[cfg(feature = "registry")]
fn write_api_stubs(lock_path: &std::path::Path, project_dir: &std::path::Path) {
    let Ok(Some(lock)) = loft::lockfile::read_lockfile(lock_path) else {
        return;
    };
    if lock.packages.is_empty() {
        return;
    }
    let api_dir = project_dir.join(".loft").join("api");
    if std::fs::create_dir_all(&api_dir).is_err() {
        return;
    }
    let mut written = 0u32;
    for p in &lock.packages {
        let dir = loft::registry_index::extract_dir(&p.name, &p.version);
        if !dir.join("loft.toml").exists() {
            continue;
        }
        if let Ok(text) = loft::documentation::render_pkg_api_text(&dir) {
            if std::fs::write(api_dir.join(format!("{}.api", p.name)), text).is_ok() {
                written += 1;
            }
        }
    }
    if written > 0 {
        println!(
            "wrote {written} API stub(s) to {} (agent-readable; commit them)",
            api_dir.display()
        );
    }

    // Alongside the per-dep stubs, write the registry CATALOG (_available.api) so
    // an agent reading `.loft/api/` sees not just what the project depends on but
    // everything else it could `loft install`.  Best-effort + cache-first: if the
    // index can't load (offline / invalid signature) just skip — the install
    // already succeeded, and discovery is a convenience, never a blocker.
    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: None,
    };
    if let Ok(index) = loft::install::load_index(&opts) {
        let _ = std::fs::write(
            api_dir.join("_available.api"),
            loft::registry_index::render_catalog(&index),
        );
    }
}

/// @PLN78 step 5 — report advisories against the loft version now running.
///
/// Reports; never restricts.  Whether to keep running a flagged release is the user's
/// call on their machine — a tool that refused to start would be worked around rather
/// than heeded.  What it owes them is the id, what it is, and where the fix landed.
///
/// Silent when the registry hosts no feed yet (`Ok(None)` — a 404), because that is
/// "nothing known", not "nothing wrong", and a warning there would train people to
/// ignore this line before it ever carries a real one.
#[cfg(feature = "registry")]
fn report_advisories(current: &str, refresh: bool, allow_unsigned: bool) {
    use loft::registry_advisories::LoadOptions;
    let opts = LoadOptions {
        allow_unsigned,
        offline: false,
        refresh,
    };
    let Ok(Some(feed)) = loft::registry_advisories::load_or_fetch(&opts) else {
        return;
    };
    // Silent when clean.  A line saying "nothing is wrong" is a line the reader
    // learns to skip, and the day it says something else they will skip that too.
    let flags = loft::self_update::flags_for(&feed, current);
    if flags.is_empty() {
        return;
    }
    for f in &flags {
        println!("  ADVISORY [{}] {} — {}", f.severity, f.id, f.summary);
        if let Some(fixed) = &f.fixed_in {
            println!("            fixed in {fixed}");
        }
    }
}

/// @PLN78 steps 3-4 — how `loft self-update` was invoked.
///
/// A struct rather than five positional arguments: `self_update_cmd(true, false, false,
/// None, true)` at the call site is unreadable, and a transposed pair of bools here
/// would mean silently forcing an install the user asked to dry-run.
// These really are five independent switches a user may combine freely, so a struct of
// bools is the readable form; folding them into an enum would invent states that do not
// exist.  The lint's usual cure — a builder or flag type — would be more machinery than
// the thing it guards.
#[allow(clippy::struct_excessive_bools)]
#[cfg(feature = "registry")]
struct SelfUpdateArgs<'a> {
    dry_run: bool,
    refresh: bool,
    allow_unsigned: bool,
    /// Install this unpacked bundle instead of resolving from the registry.
    from: Option<&'a str>,
    force: bool,
}

/// @PLN78 step 4 — install a bundle the user already has (`--from <dir>`).
///
/// This route must always exist, for the same reason a library can be installed from a
/// local path: the people who need it are the ones who cannot or will not compile loft,
/// and a registry-only updater strands them the moment the network, the firewall, or
/// the registry is not there.  A hand-carried release is a first-class way to install.
///
/// **The strictness belongs on us, not on the user.**  We never publish a release that
/// cannot be fully verified — that is a rule about `make-release.sh` and the registry
/// entry, enforced where we publish.  What someone installs on their own machine is
/// theirs to decide, so nothing here refuses a bundle its owner wants: an unverifiable
/// bundle installs with a clear note, and one that actively contradicts its manifest
/// needs `--force`, because that is nearly always a truncated copy rather than an
/// intention.  Informed, not obstructed.
#[cfg(feature = "registry")]
fn self_update_from_local(dir: &str, dry_run: bool, force: bool) -> i32 {
    let staged = std::path::Path::new(dir);
    if !staged.is_dir() {
        eprintln!(
            "loft self-update --from: {dir} is not a directory (unpack the release zip first)"
        );
        return 1;
    }
    install_staged_bundle(&StagedInstall {
        staged,
        label: format!("--from {dir}"),
        // A directory the user supplied: intactness is checkable, origin is theirs.
        verified_release: None,
        dry_run,
        force,
    })
}

/// A bundle staged on disk, ready to replace the installation.
#[cfg(feature = "registry")]
struct StagedInstall<'a> {
    staged: &'a std::path::Path,
    /// What to echo after `loft self-update`.
    label: String,
    /// `Some(version)` when the bundle was downloaded and its hash matched the signed
    /// index — the difference between "this is intact" and "this is the release we
    /// published", which the closing message must not blur.
    verified_release: Option<String>,
    dry_run: bool,
    force: bool,
}

/// Check a staged bundle and, unless this is a dry run, install it.
///
/// Shared by both routes so the checks, the refusals and the rollback cannot drift
/// apart: a downloaded bundle and a hand-supplied one differ in what is known about
/// their ORIGIN, and in nothing else.
#[cfg(feature = "registry")]
fn install_staged_bundle(a: &StagedInstall) -> i32 {
    use loft::verify_self::{Check, bundle_root, local_checks};
    let (staged, dir) = (a.staged, a.staged.display().to_string());
    let (dry_run, force) = (a.dry_run, a.force);
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("loft self-update: cannot locate the running binary");
        return 1;
    };
    let Some(root) = bundle_root(&exe) else {
        eprintln!(
            "loft self-update: cannot resolve an installation root from {}",
            exe.display()
        );
        return 1;
    };
    println!("loft self-update {}", a.label);
    println!("  target  {}", root.display());
    if let Some(v) = &a.verified_release {
        println!("  ok      {v} downloaded and matches the signed registry index");
    }
    let checks = local_checks(staged);
    for c in &checks {
        match c {
            Check::Ok(m) => println!("  ok      staged {m}"),
            Check::Skipped(m) => println!("  --      staged {m}"),
            Check::Failed(m) => println!("  FAILED  staged {m}"),
        }
    }
    let contradicts = checks.iter().any(Check::failed);
    let unverifiable = checks.iter().all(|c| matches!(c, Check::Skipped(_)));
    if contradicts && !force {
        eprintln!(
            "\nThis bundle contradicts its own manifest — usually a truncated or partly\n\
             copied directory.  Nothing changed.  Re-copy it, or pass --force if you\n\
             meant to install it as it is."
        );
        return 1;
    }
    if dry_run {
        println!("\n--dry-run: nothing was changed.");
        return 0;
    }
    match loft::self_update::apply_bundle(&root, staged, force) {
        Ok(files) => {
            println!("\n  ok      replaced {} file(s)", files.len());
            if force {
                println!("\nInstalled with --force, past a manifest this bundle does not match.");
            } else if a.verified_release.is_some() {
                println!(
                    "\nInstalled.  The chain held end to end: the signed index named this \n\
                     bundle's hash, the download matched it, and every file matches the \n\
                     manifest inside it."
                );
            } else if unverifiable {
                println!(
                    "\nInstalled.  Nothing could be checked: {dir} carries no\n\
                     SHA256SUMS, so this was your artifact taken at your word."
                );
            } else {
                println!(
                    "\nInstalled.  The bundle is INTACT — every file matches the manifest it\n\
                     shipped with.  Where it came from is your assertion, not something this\n\
                     checked; `loft verify-self` re-checks the result at any time."
                );
            }
            0
        }
        Err(e) => {
            eprintln!("\nloft self-update: {e}");
            1
        }
    }
}

/// @PLN78 step 3 — `loft self-update`: report what an update WOULD do.
///
/// Read-only in this step: it resolves and reports, and the replacement itself is
/// step 4.  Plain `self-update` and `--dry-run` behave identically for now, so a
/// script written today keeps meaning what it meant once step 4 lands — the flag is
/// accepted rather than required, and mutation will be the thing that has to be
/// asked for, never the default that arrived by surprise.
///
/// The index comes from `install::load_index`, the same signature-verified loader
/// `loft install` uses.  That is the point of site 3 in the design's table: a second
/// fetch-and-check here would be a second place to forget the signature.
#[cfg(feature = "registry")]
fn self_update_cmd(args: &SelfUpdateArgs<'_>) -> i32 {
    let SelfUpdateArgs {
        dry_run,
        refresh,
        allow_unsigned,
        from,
        force,
    } = *args;
    use loft::install::InstallOptions;
    use loft::self_update::{Plan, host_triple, plan};
    let current = env!("CARGO_PKG_VERSION");
    let triple = host_triple();
    // `--from <dir>` — install a bundle the user already has, with no registry at all.
    //
    // This route must always exist, for exactly the reason libraries can be installed
    // from a local path: the people who need it are the ones who cannot or will not
    // compile loft themselves, and a registry-only updater strands them the moment the
    // network, the firewall, or the registry is not available.  A hand-carried release
    // is a first-class way to install, not a fallback.
    //
    // What it can promise is narrower, and it says so: the bundle verifies against its
    // OWN manifests (intact — no corrupt or partial copy is installed), but nothing
    // here establishes WHERE it came from.  That is the user's assertion, which is why
    // it takes an explicit flag and prints what it did and did not check.
    if let Some(dir) = from {
        return self_update_from_local(dir, dry_run, force);
    }
    // `--refresh` / `--allow-unsigned` mirror `loft install`, so an offline mirror or
    // a pre-bootstrap registry is reachable here too.  Step 4 must revisit
    // `allow_unsigned` before it REPLACES anything: waiving the signature to read a
    // report is a different risk from waiving it to overwrite the running binary.
    let opts = InstallOptions {
        refresh,
        allow_unsigned,
        ..InstallOptions::default()
    };
    let index = match loft::install::load_index(&opts) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("loft self-update: {e}");
            return 1;
        }
    };
    println!("loft self-update — running {current} ({triple})");
    // @PLN78 step 5 — check the version we are RUNNING, not only the one on offer.
    // A stalled or pinned registry offers no update at all, and a user sitting on a
    // flagged release would otherwise be told "up to date" — true, and exactly wrong.
    report_advisories(current, refresh, allow_unsigned);
    match plan(&index, current, &triple) {
        Plan::NoEntry => {
            // Not "you are up to date" — nothing was compared.  Said plainly, without
            // making our own roadmap the reader's problem.
            println!("  no releases published to compare against");
            0
        }
        Plan::Current { version } => {
            println!("  {version} is the newest release");
            0
        }
        Plan::NoBuildForTarget {
            to,
            triple,
            built_for,
        } => {
            println!(
                "  --      {to} is published, but not built for {triple}\n\
                 \x20         built for: {}",
                if built_for.is_empty() {
                    "(none)".to_string()
                } else {
                    built_for.join(", ")
                }
            );
            0
        }
        Plan::Available {
            from,
            to,
            url,
            sha256,
        } => {
            println!("  ok      {to} is available (running {from})");
            if dry_run {
                // Print what WOULD be fetched, so a dry run is auditable rather than a
                // promise: the hash below is the one the install would enforce.
                println!("            url     {url}");
                println!("            sha256  {sha256}");
                println!("\n--dry-run: nothing was changed.");
                return 0;
            }
            // Staged beside nothing else of ours, and removed on every exit path: a
            // half-unpacked bundle left in a temp directory is the kind of debris a
            // later run would happily pick up.
            let tmp =
                std::env::temp_dir().join(format!("loft-self-update-{to}-{}", std::process::id()));
            let staged = match loft::self_update::fetch_bundle(&url, &sha256, &tmp) {
                Ok(p) => p,
                Err(e) => {
                    let _ = std::fs::remove_dir_all(&tmp);
                    eprintln!("loft self-update: {e}");
                    return 1;
                }
            };
            let code = install_staged_bundle(&StagedInstall {
                staged: &staged,
                label: to.clone(),
                verified_release: Some(to.clone()),
                dry_run: false,
                force,
            });
            let _ = std::fs::remove_dir_all(&tmp);
            code
        }
    }
}

/// The manifest digest the signed registry index publishes for THIS build, if it can
/// be read.  `Ok(None)` = the index carries no digest for this version + triple;
/// `Err` = the index could not be consulted at all.  The two are different answers and
/// the caller reports them differently — "we could not check" must never read as "we
/// checked and it was fine".
#[cfg(feature = "registry")]
fn published_manifest_digest() -> Result<Option<String>, String> {
    use loft::install::InstallOptions;
    let index = loft::install::load_index(&InstallOptions::default())?;
    let pkg = index
        .packages
        .get(loft::self_update::TOOLCHAIN_PKG)
        .ok_or_else(|| "the registry carries no toolchain entry".to_string())?;
    Ok(pkg
        .versions
        .get(env!("CARGO_PKG_VERSION"))
        .and_then(|v| v.binaries.get(&loft::self_update::host_triple()))
        .and_then(|b| b.manifest_sha256.clone()))
}

#[cfg(not(feature = "registry"))]
fn published_manifest_digest() -> Result<Option<String>, String> {
    Err("built without registry support".to_string())
}

/// @PLN78 step 2 — `loft verify-self`: is this installation the one that was released?
/// Read-only.
///
/// Three exits, because *verified intact* and *could not verify anything* are the two
/// answers a caller most needs to tell apart and they used to be the same answer
/// (loft#1012).  `loft verify-self && deploy` was green on an install the command could
/// not examine — a check that silently did not run, which is the same shape as the
/// install-that-is-not-what-you-think this command exists to catch, one level up.
///
/// | `0` | verified against the shipped manifests — intact |
/// | `1` | verified, and something does not match |
/// | `2` | could not verify: not a release bundle, so there was nothing to check against |
///
/// `loft audit` already grades its exits this way (`0` clean, `1` low, `2` high,
/// `3` security_critical), so the CLI has the precedent; the message was always honest
/// and it is the exit code that gets read.
///
/// Three questions, in `verify_self`'s terms: every listed file still matches, no
/// unlisted `*.loft` sits in `default/`, and the manifest itself matches the signed
/// registry index.  The first two ship inside the bundle they describe, so they can
/// only establish INTACT; the third is what makes the answer AUTHENTIC.  The output
/// keeps them apart, because a check that sounds like more than it did is exactly what
/// @PLN78 step 0 removed from the catalogue.
/// `verify-self`'s third exit: the checks could not run at all.
///
/// Distinct from `1` (ran and something is wrong) and from `0` (ran and everything
/// matched), so `loft verify-self && …` no longer proceeds on an unverifiable install.
const NOTHING_TO_VERIFY: i32 = 2;

fn verify_self_cmd() -> i32 {
    use loft::verify_self::{Check, bundle_root, check_anchor, local_checks};
    let Ok(exe) = std::env::current_exe() else {
        eprintln!("loft verify-self: cannot locate the running binary");
        return 1;
    };
    let Some(root) = bundle_root(&exe) else {
        eprintln!(
            "loft verify-self: cannot resolve a bundle root from {}",
            exe.display()
        );
        return 1;
    };
    let mut checks = local_checks(&root);
    // Only consult the registry for something that IS a bundle; a source checkout has
    // no manifest to anchor, and a network round-trip to say so would be noise.
    if !checks.iter().all(|c| matches!(c, Check::Skipped(_))) {
        checks.push(match published_manifest_digest() {
            Ok(published) => check_anchor(&root, published.as_deref()),
            Err(e) => Check::Skipped(format!(
                "origin: could not consult the registry ({e}) — intact, but not traced \
                 to a signature"
            )),
        });
    }
    // A source checkout is the common case for anyone working ON loft, and it has
    // nothing to check.  One line, not three saying it separately.
    if checks.iter().all(|c| matches!(c, Check::Skipped(_))) {
        println!(
            "{}: not a release bundle — nothing to check against",
            root.display()
        );
        // Not 0: nothing was verified, and a caller that reads the exit code would
        // otherwise take this for a pass (loft#1012).
        return NOTHING_TO_VERIFY;
    }
    println!("loft verify-self — {}", root.display());
    let mut failed = false;
    let mut skipped = 0;
    for c in &checks {
        match c {
            Check::Ok(m) => println!("  ok      {m}"),
            Check::Skipped(m) => {
                skipped += 1;
                println!("  --      {m}");
            }
            Check::Failed(m) => {
                failed = true;
                println!("  FAILED  {m}");
            }
        }
    }
    println!();
    if failed {
        println!(
            "This installation does not match the manifests it shipped with.  A changed\n\
             stdlib file with an unchanged binary is the usual cause — a partial upgrade —\n\
             and loft loads its stdlib from <binary-dir>/../default, so it would run with\n\
             the mismatch.  Reinstall the release rather than replacing single files."
        );
        return 1;
    }
    if skipped == checks.len() {
        println!("not a release bundle — nothing to check against");
        return NOTHING_TO_VERIFY;
    }
    // Bound what "ok" meant.  Anchored, the chain runs signature -> manifest -> files
    // and the answer is about origin; unanchored, it is a bundle vouching for itself.
    if checks
        .iter()
        .any(|c| matches!(c, Check::Ok(m) if m.starts_with("origin:")))
    {
        println!("matches the release published in the signed registry index");
    } else {
        println!("matches the manifest it shipped with (detects corruption, not substitution)");
    }
    0
}

/// @PLAN12 Phase 6.6 — `loft list-installed` enumerates packages
/// in `~/.loft/registry/` and annotates each with its sha256 +
/// size + index status (active / yanked / orphan-from-index).
///
/// Pure query — touches no network, writes nothing.  Useful for
/// "what's in my cache?" investigations + as a starting point for
/// `loft audit` once Phase 6.7's advisory channel ships.
#[cfg(feature = "registry")]
fn list_installed() {
    use loft::install::InstallOptions;
    use loft::registry_index;
    use std::path::PathBuf;

    let cache = registry_index::cache_dir();
    if !cache.exists() {
        println!("No registry cache at {}.", cache.display());
        return;
    }

    let entries: Vec<(String, String, PathBuf)> = registry_index::installed_packages();

    if entries.is_empty() {
        println!(
            "No registry packages installed (cache: {}).",
            cache.display()
        );
        return;
    }

    // Try to load the cached index so we can show sha256 + size +
    // status.  If the index isn't cached (cold loft binary), skip
    // the annotations but still list the dirs.
    let opts = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: true, // never hit the network for this query
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: None,
    };
    let index = loft::install::load_index(&opts).ok();

    println!("Installed packages (in {}):", cache.display());
    for (name, version, path) in &entries {
        let on_disk_bytes = dir_size_bytes(path).unwrap_or(0);
        let mut sha = String::new();
        let mut status_tag = String::new();
        if let Some(idx) = &index {
            if let Some(pkg) = idx.packages.get(name) {
                if let Some(v) = pkg.versions.get(version) {
                    sha.clone_from(&v.sha256);
                    if pkg.yanked.iter().any(|y| y == version) {
                        status_tag.push_str(" [YANKED]");
                    }
                } else {
                    status_tag.push_str(" [version not in current index]");
                }
            } else {
                status_tag.push_str(" [orphan: package not in current index]");
            }
        }
        let sha_short: String = sha.chars().take(12).collect();
        let sha_disp = if sha_short.is_empty() {
            "(unknown sha)".to_string()
        } else {
            format!("sha256:{sha_short}…")
        };
        println!(
            "  {name} {version}{status_tag} — {} bytes on disk, {sha_disp}",
            on_disk_bytes
        );
    }
    println!("{} package(s) total", entries.len());
}

/// @PLAN12 Phase 6.8 — knob set for the `loft update` command.
#[cfg(feature = "registry")]
#[derive(Debug, Default, Clone)]
struct UpdateOpts {
    /// Specific package name; `None` updates every entry in the
    /// lockfile.
    target: Option<String>,
    /// Compute + print the diff without writing the lockfile.
    dry_run: bool,
    /// Exit non-zero if any updates are available (CI gate).
    /// Implies dry_run.
    check_only: bool,
}

/// @PLAN12 Phase 6.8 — `loft update` driver.
///
/// Resolves every package the project depends on — the union of `loft.lock`'s
/// entries and `loft.toml`'s declared (non-path) dependencies — looks each up
/// in the registry index, picks the highest active non-yanked version that
/// satisfies the declared range (a lock entry with no matching declaration is
/// treated as exact and never updated), and — unless in dry-run/check mode —
/// calls `install_one` to fetch + extract + merge into the lockfile.
///
/// The union is the point (loft#830): starting from the lock alone made a
/// dependency added to `loft.toml` invisible to the one command whose job is
/// to make the lock describe the manifest, and the summary counted lock
/// entries, so the omission reported itself as `all N packages up-to-date`.
///
/// Exit codes:
/// - 0  → up-to-date (no updates needed or all updates applied
///   successfully).
/// - 1  → updates available (`--check`) OR a declared dependency that cannot
///   be resolved (`--check`) OR install failure.
/// - 2  → nothing to update: no lockfile AND no declared dependencies.
#[cfg(feature = "registry")]
fn update_packages(opts: &UpdateOpts) -> i32 {
    use loft::install::{InstallOptions, install_one};
    use loft::lockfile;
    use loft::manifest;
    use loft::registry_index;

    // Find the project root or fall back to cwd.  Mirror the
    // parser's walk-up logic (Phase 6.6) — start at cwd, walk up
    // looking for loft.toml.
    let cwd = match std::env::current_dir() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("loft update: cwd: {e}");
            return 1;
        }
    };
    let project_root = loft::resolution_scope::project_root_from(&cwd);
    let lock_dir = project_root.as_ref().unwrap_or(&cwd);
    let lock_path = lock_dir.join("loft.lock");

    // Read project loft.toml deps: they give the version range for each entry,
    // AND they are half the work list.  Path dependencies are resolved from
    // disk and never carry a lock entry, so they are filtered out here.
    // A lock entry with no declaration (a transitive dep) defaults to "*".
    let declared: Vec<(String, String)> = project_root
        .as_ref()
        .and_then(|root| manifest::read_manifest(root.join("loft.toml").to_str().unwrap_or("")))
        .map(|m| {
            m.dependencies
                .into_iter()
                .filter(|(_, v)| manifest::extract_path_dep(v).is_none())
                .collect()
        })
        .unwrap_or_default();
    let toml_deps: std::collections::HashMap<String, String> = declared.iter().cloned().collect();

    // A missing lock is not "nothing to update" when the manifest declares
    // dependencies — that is exactly the state `loft update` should resolve.
    let lock = match lockfile::read_lockfile(&lock_path) {
        Ok(Some(l)) => l,
        Ok(None) => lockfile::LockFile {
            schema_version: lockfile::SCHEMA_VERSION,
            packages: Vec::new(),
        },
        Err(e) => {
            eprintln!("loft update: cannot read {}: {e}", lock_path.display());
            return 1;
        }
    };
    let worklist = lockfile::update_worklist(&lock.packages, &declared);
    if worklist.is_empty() {
        if lock_path.exists() {
            eprintln!("loft update: lockfile has no packages, and loft.toml declares none.");
            return 0;
        }
        eprintln!(
            "loft update: no loft.lock at {} and no dependencies in loft.toml — nothing to update.",
            lock_path.display()
        );
        eprintln!(
            "  Run `loft install <pkg>` first (or `loft pin <script>` for one-file scripts)."
        );
        return 2;
    }
    if let Some(t) = &opts.target
        && !worklist.iter().any(|w| &w.name == t)
    {
        eprintln!(
            "loft update {t}: not a dependency of this project — \
             it is in neither loft.toml nor loft.lock."
        );
        eprintln!("  Add it to [dependencies] first, or run `loft install {t}`.");
        return 1;
    }

    // Load index (offline-respecting, allow_unsigned for the
    // bootstrap window).
    let install_opts = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: std::env::var("LOFT_OFFLINE").is_ok(),
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: Some(lock_path.clone()),
    };
    let index = match loft::install::load_index(&install_opts) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("loft update: {e}");
            return 1;
        }
    };

    let dry = opts.dry_run || opts.check_only;
    let mut updates_available = false;
    // A dependency the manifest declares that the lock cannot be made to
    // describe.  Not an "update", but it means the lock does NOT match the
    // manifest — which is what `--check` exists to catch.
    let mut manifest_gap = false;
    let mut install_failures: Vec<String> = Vec::new();
    let mut diff: Vec<String> = Vec::new();

    for target in &worklist {
        if let Some(t) = &opts.target {
            if t != &target.name {
                continue;
            }
        }
        // What the lock says today, and how to name it in a report line.
        let held = target.locked.as_deref();
        let at = held.map_or_else(|| "(not locked)".to_string(), ToString::to_string);
        let pkg = match index.packages.get(&target.name) {
            Some(p) => p,
            None => {
                if held.is_none() {
                    // Declared in loft.toml, absent from the index: the lock
                    // cannot be completed.  Skipping this silently is the
                    // failure loft#830 reported, so it is always named.
                    manifest_gap = true;
                    diff.push(format!(
                        "  {pkg} — declared in loft.toml but not in the registry index; \
                         cannot be added to loft.lock",
                        pkg = target.name
                    ));
                } else {
                    diff.push(format!(
                        "  {pkg} {at} — not in current index (orphan; skipped)",
                        pkg = target.name
                    ));
                }
                continue;
            }
        };
        let constraint = toml_deps
            .get(&target.name)
            .cloned()
            .unwrap_or_else(|| "*".to_string());
        // Step 6: an upgrade must not hand the consumer a release that has
        // DECLARED it breaks them.  `held` is what they hold, so a candidate
        // whose `api_compatible_with` is above it is passed over — and named,
        // because a resolver that silently stops at an older release teaches
        // its consumer that no upgrade exists.  A package with no lock entry
        // holds nothing, so nothing can be held back from it.
        let resolved = registry_index::find_compatible_version(pkg, &constraint, false, held);
        for held_back in &resolved.withheld {
            let floor = held_back.api_compatible_with.as_deref().unwrap_or("?");
            diff.push(format!(
                "  {pkg} {at} — {new} held back: declares a break past {at} \
                 (api_compatible_with = {floor}). Upgrade deliberately, or stay.",
                pkg = target.name,
                new = held_back.semver
            ));
        }
        let Some(best) = resolved.best else {
            // Distinguish "nothing satisfies the range" from "everything that
            // does declares a break" — the fixes are different.
            let why = if resolved.withheld.is_empty() {
                format!("no version satisfies range `{constraint}` (skipped)")
            } else {
                format!("every version satisfying `{constraint}` declares a break past it")
            };
            if held.is_none() {
                manifest_gap = true;
            }
            diff.push(format!("  {pkg} {at} — {why}", pkg = target.name));
            continue;
        };
        if held == Some(best.semver.as_str()) {
            // Already on the highest satisfying version this consumer can take.
            continue;
        }
        // Higher OR lower (e.g. rollback after yank) — both are
        // "updates" in the sense of "lockfile would change."
        updates_available = true;
        if held.is_none() {
            diff.push(format!(
                "  {pkg} → {new} (declared in loft.toml, missing from loft.lock)",
                pkg = target.name,
                new = best.semver
            ));
        } else {
            diff.push(format!(
                "  {pkg} {at} → {new}",
                pkg = target.name,
                new = best.semver
            ));
        }
        if !dry {
            match install_one(&target.name, Some(&best.semver), &install_opts) {
                Ok(_) => {}
                Err(e) => {
                    eprintln!("  FAILED {} {}: {e}", target.name, best.semver);
                    install_failures.push(target.name.clone());
                }
            }
        }
    }

    if diff.is_empty() {
        if let Some(t) = &opts.target {
            println!("loft update {t}: already on the highest satisfying version.");
        } else {
            // Count what was actually resolved — declarations included.  Counting
            // `lock.packages` reported success for a lock that was missing one of
            // them (loft#830).
            println!("loft update: all {} packages up-to-date.", worklist.len());
        }
        return 0;
    }

    if opts.check_only {
        if updates_available || manifest_gap {
            println!("loft update --check: the lockfile does not match loft.toml:");
        } else {
            // Reachable when the only lines are held-back or skipped notes.  A
            // consumer correctly staying put must not turn a CI check red —
            // that is the pressure that gets a floor ignored or a gate removed.
            println!("loft update --check: no updates available, but note:");
        }
        for line in &diff {
            println!("{line}");
        }
        return i32::from(updates_available || manifest_gap);
    }
    if opts.dry_run {
        println!("loft update --dry-run: would update:");
        for line in &diff {
            println!("{line}");
        }
        return 0;
    }

    println!("loft update:");
    for line in &diff {
        println!("{line}");
    }
    if !install_failures.is_empty() {
        eprintln!(
            "loft update: {} package(s) failed to install",
            install_failures.len()
        );
        return 1;
    }
    let _ = updates_available;
    0
}

/// @PLAN12 Phase 6.11 — `loft bundle export <outdir>` writes a
/// self-contained directory of registry artifacts (index +
/// advisories + tarballs) that can be carried via USB / scp and
/// imported on an air-gapped machine via `loft bundle import`.
///
/// Layout:
/// ```
/// <outdir>/
/// ├── index.json + index.json.sig
/// ├── advisories.json + advisories.json.sig  (when present)
/// ├── packages/
/// │   ├── <pkg>-<ver>.tar.gz
/// │   └── ...
/// └── manifest.json   (bundle metadata: timestamp, source URL,
///                      loft binary version, package list)
/// ```
///
/// Selection:
/// - `--all` (or omit both flags) → every package in index.
/// - `--packages X,Y,Z` → just those (transitive deps not yet
///   auto-resolved; transitive harvest is a follow-up if useful).
#[cfg(feature = "registry")]
fn bundle_export(outdir: &str, packages: Option<&[String]>, all: bool) -> i32 {
    use loft::install::InstallOptions;
    use loft::registry_index;
    use std::path::Path;

    let out = Path::new(outdir);
    if let Err(e) = std::fs::create_dir_all(out.join("packages")) {
        eprintln!("loft bundle export: cannot create {}: {e}", out.display());
        return 1;
    }

    let opts = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: None,
    };
    let index = match loft::install::load_index(&opts) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("loft bundle export: {e}");
            return 1;
        }
    };

    // Pick which packages to export.
    let want: Vec<String> = if all || packages.is_none() {
        index.packages.keys().cloned().collect()
    } else {
        packages.unwrap_or(&[]).to_vec()
    };
    if want.is_empty() {
        eprintln!("loft bundle export: nothing to export (empty package list)");
        return 1;
    }

    // Copy index.json + sig.
    let (idx_path, sig_path, _) = registry_index::index_paths();
    if idx_path.exists() {
        if let Err(e) = std::fs::copy(&idx_path, out.join("index.json")) {
            eprintln!("loft bundle export: copy index.json: {e}");
            return 1;
        }
    }
    if sig_path.exists() {
        let _ = std::fs::copy(&sig_path, out.join("index.json.sig"));
    }
    // Advisories (optional — registry may not host one yet).
    let (adv_path, adv_sig_path) = loft::registry_advisories::advisories_paths();
    if adv_path.exists() {
        let _ = std::fs::copy(&adv_path, out.join("advisories.json"));
        if adv_sig_path.exists() {
            let _ = std::fs::copy(&adv_sig_path, out.join("advisories.json.sig"));
        }
    }

    // For each requested package: pick its latest active version,
    // ensure the tarball is downloaded, copy into bundle/packages/.
    let mut exported: Vec<(String, String)> = Vec::new();
    for name in &want {
        let pkg = match index.packages.get(name) {
            Some(p) => p,
            None => {
                eprintln!("  warning: {name} not in current index — skipped");
                continue;
            }
        };
        let Some(ver) = registry_index::find_best_version(pkg, "*", false) else {
            eprintln!("  warning: {name} has no installable version — skipped");
            continue;
        };
        let tarball = format!("{name}-{}.tar.gz", ver.semver);
        let dest = out.join("packages").join(&tarball);
        // Download to the bundle's packages/.
        eprintln!("[bundle] fetching {} {}", name, ver.semver);
        let bytes = match registry_index::download_tarball(&ver.url, &dest) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("  FAILED: {e}");
                return 1;
            }
        };
        if let Err(e) = loft::integrity::verify_sha256(&bytes, &ver.sha256) {
            eprintln!("  FAILED: {e}");
            return 1;
        }
        exported.push((name.clone(), ver.semver.clone()));
    }

    // Write manifest.json — bundle metadata for the import side.
    let manifest_pkgs: String = exported
        .iter()
        .map(|(n, v)| format!("    {{\"name\":\"{n}\",\"version\":\"{v}\"}}"))
        .collect::<Vec<_>>()
        .join(",\n");
    let registry_url = registry_index::registry_url();
    let loft_version = env!("CARGO_PKG_VERSION");
    let manifest = format!(
        "{{\n  \"schema_version\": 1,\n  \"created\": \"{}\",\n  \"registry_url\": \"{}\",\n  \"loft_version\": \"{}\",\n  \"packages\": [\n{}\n  ]\n}}\n",
        chrono_iso8601_utc(),
        registry_url,
        loft_version,
        manifest_pkgs
    );
    if let Err(e) = std::fs::write(out.join("manifest.json"), manifest) {
        eprintln!("loft bundle export: write manifest: {e}");
        return 1;
    }

    println!(
        "[bundle] exported {} package(s) to {}",
        exported.len(),
        out.display()
    );
    0
}

/// @PLAN12 Phase 6.11 — `loft bundle import <indir>` installs a
/// previously-exported bundle into `~/.loft/registry/`.
///
/// Steps per artifact:
/// 1. Copy `index.json` + `.sig` (sig kept; loader verifies via
///    `allow_unsigned` for the bootstrap window).
/// 2. Copy `advisories.json` + `.sig` (when present).
/// 3. For each `packages/<pkg>-<ver>.tar.gz`:
///    - Verify sha256 matches the index entry.
///    - Extract to `~/.loft/registry/<pkg>-<ver>/`.
/// 4. Print summary.
#[cfg(feature = "registry")]
fn bundle_import(indir: &str) -> i32 {
    use loft::registry_index;
    use std::path::Path;

    let inp = Path::new(indir);
    let bundle_index = inp.join("index.json");
    if !bundle_index.exists() {
        eprintln!("loft bundle import: {} has no index.json", inp.display());
        return 1;
    }

    // Read + parse the bundle's index so we know the per-tarball sha256.
    let idx_bytes = match std::fs::read(&bundle_index) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("loft bundle import: read index: {e}");
            return 1;
        }
    };
    let idx_text = match std::str::from_utf8(&idx_bytes) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("loft bundle import: index not UTF-8: {e}");
            return 1;
        }
    };
    let index = match registry_index::parse_index(idx_text) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("loft bundle import: parse index: {e}");
            return 1;
        }
    };

    // Copy index + sig + (optional) advisories into the cache.
    let cache = registry_index::cache_dir();
    if let Err(e) = std::fs::create_dir_all(&cache) {
        eprintln!("loft bundle import: cannot create {}: {e}", cache.display());
        return 1;
    }
    let _ = std::fs::copy(&bundle_index, cache.join("index.json"));
    let bundle_sig = inp.join("index.json.sig");
    if bundle_sig.exists() {
        let _ = std::fs::copy(&bundle_sig, cache.join("index.json.sig"));
    }
    let bundle_adv = inp.join("advisories.json");
    if bundle_adv.exists() {
        let _ = std::fs::copy(&bundle_adv, cache.join("advisories.json"));
        let bundle_adv_sig = inp.join("advisories.json.sig");
        if bundle_adv_sig.exists() {
            let _ = std::fs::copy(&bundle_adv_sig, cache.join("advisories.json.sig"));
        }
    }

    // Extract each tarball; verify sha256 against the index entry.
    let pkg_dir = inp.join("packages");
    let read = match std::fs::read_dir(&pkg_dir) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("loft bundle import: read {}: {e}", pkg_dir.display());
            return 1;
        }
    };
    let mut imported: Vec<(String, String)> = Vec::new();
    for ent in read.filter_map(Result::ok) {
        let path = ent.path();
        let fname = match path.file_name().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        if !fname.ends_with(".tar.gz") {
            continue;
        }
        let stem = &fname[..fname.len() - ".tar.gz".len()];
        // Split last `-<digit>` boundary for (name, version).
        let bytes = stem.as_bytes();
        let mut at: Option<usize> = None;
        let mut idx = 1;
        while idx < bytes.len() {
            if bytes[idx - 1] == b'-' && bytes[idx].is_ascii_digit() {
                at = Some(idx - 1);
                break;
            }
            idx += 1;
        }
        let Some(split) = at else { continue };
        let (name, rest) = stem.split_at(split);
        let version = rest.trim_start_matches('-');

        // Look up sha256 in the imported index.
        let Some(pkg) = index.packages.get(name) else {
            eprintln!("  skip {fname}: not in bundle's index");
            continue;
        };
        let Some(ver) = pkg.versions.get(version) else {
            eprintln!("  skip {fname}: version not in bundle's index");
            continue;
        };
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("  read {fname}: {e}");
                return 1;
            }
        };
        if let Err(e) = loft::integrity::verify_sha256(&bytes, &ver.sha256) {
            eprintln!("  sha256 MISMATCH for {fname}: {e}");
            return 1;
        }
        if let Err(e) = registry_index::extract_tarball(&path, &cache) {
            eprintln!("  extract {fname}: {e}");
            return 1;
        }
        imported.push((name.to_string(), version.to_string()));
    }

    if imported.is_empty() {
        eprintln!(
            "loft bundle import: no packages found in {}",
            pkg_dir.display()
        );
        return 1;
    }
    println!(
        "[bundle] imported {} package(s) into {}",
        imported.len(),
        cache.display()
    );
    for (n, v) in &imported {
        println!("  {n} {v}");
    }
    0
}
/// The packages this package's own source `use`s that `loft.toml` does not declare.
///
/// `loft publish` reads `deps` from the manifest and has no other source for them, so when the
/// manifest declares none it prints `"deps": {}` — a claim, where the honest answer may be
/// "not stated here".  A multi-package repo keeps its registry deps out of `loft.toml` on
/// purpose: declared there, loft resolves them from the registry instead of the `--lib` path
/// and multi-library consumption breaks.  For those packages the emitted entry is silently
/// incomplete, and a consumer only finds out at `loft install` (loft#1136).
///
/// The source settles which case it is.  A `use X` naming neither a sibling module of this
/// package nor the package itself is a registry dependency, and one the manifest does not
/// declare is exactly what an empty `deps` would drop.
///
/// Deliberately syntactic: `publish` does not parse the package, and the question is only
/// *"is `{}` believable here?"*.  Over-reporting costs a note the author can ignore;
/// under-reporting is the defect.
// Its one caller is `registry` gated; the unit tests below reach it directly.
#[cfg(any(test, feature = "registry"))]
fn undeclared_source_deps(
    pkg_path: &std::path::Path,
    pkg_name: &str,
    declared: &[(String, String)],
) -> Vec<String> {
    let src_dir = pkg_path.join("src");
    let Ok(entries) = std::fs::read_dir(&src_dir) else {
        return Vec::new();
    };
    let files: Vec<std::path::PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "loft"))
        .collect();
    let local: std::collections::HashSet<String> = files
        .iter()
        .filter_map(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
        .collect();
    let mut out: Vec<String> = Vec::new();
    for f in &files {
        let Ok(text) = std::fs::read_to_string(f) else {
            continue;
        };
        for line in text.lines() {
            let Some(rest) = line.trim_start().strip_prefix("use ") else {
                continue;
            };
            // `use pkg;`, `use pkg::*;`, `use pkg::item;` — the package is the head.
            let id: String = rest
                .trim()
                .trim_end_matches(';')
                .split("::")
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if id.is_empty()
                || id == pkg_name
                || local.contains(&id)
                || declared.iter().any(|(n, _)| *n == id)
                || out.contains(&id)
            {
                continue;
            }
            out.push(id);
        }
    }
    out.sort();
    out
}

/// Minimal ISO-8601 UTC timestamp.  Avoid pulling in a date crate;
/// loft already does its own time formatting elsewhere via
/// `std::time::SystemTime`.  Format: `YYYY-MM-DDTHH:MM:SSZ`.
#[cfg(feature = "registry")]
fn chrono_iso8601_utc() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // Days since 1970-01-01 (Unix epoch).
    let day = secs / 86_400;
    let sod = secs % 86_400;
    let hour = sod / 3600;
    let minute = (sod % 3600) / 60;
    let second = sod % 60;
    // Convert day count to Y/M/D via the standard algorithm.
    // Cribbed from https://howardhinnant.github.io/date_algorithms.html
    let z = day as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = i64::from(yoe) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02}T{hour:02}:{minute:02}:{second:02}Z",
        y = y,
        m = m,
        d = d,
        hour = hour,
        minute = minute,
        second = second
    )
}

/// @PLAN12 Phase 6.7a — `loft yank` author helper.
///
/// Closes the security loop on the author side.  Phase 6.7
/// ships the consumer-side classifier (loft binary checks
/// `~/.loft/registry/advisories.json` against installed
/// versions and refuses/warns per severity); 6.7a provides
/// the CLI an author runs to FILE an advisory.
///
/// Emits the two edits the registry PR needs:
///
/// 1. `index.json` — adds the typed `status` field to the
///    affected version entry.
/// 2. `advisories.json` — appends a row with cross-referenced
///    `id`, `packages[]`, `severity`, `summary`, `published`.
///
/// Auto-PR-open (clone registry + splice in the edits +
/// `gh pr create`) is the next iteration; MVP emits the two
/// blocks ready for paste into a manual PR.
#[cfg(feature = "registry")]
fn yank_package(
    target: &str,
    severity: Option<&str>,
    advisory: Option<&str>,
    summary: Option<&str>,
    affected: Option<&str>,
    fixed_in: Option<&str>,
) -> i32 {
    let Some((name, version)) = target.split_once('@') else {
        eprintln!("loft yank: target must be `<pkg>@<version>` (got `{target}`)");
        return 1;
    };
    if name.is_empty() || version.is_empty() {
        eprintln!("loft yank: target must be `<pkg>@<version>` (got `{target}`)");
        return 1;
    }
    let severity = match severity {
        Some(s) => match s {
            "security_critical" | "security_high" | "security_low" | "bug" | "deprecated" => s,
            other => {
                eprintln!(
                    "loft yank: --severity must be one of \
                     security_critical / security_high / security_low / bug / deprecated \
                     (got `{other}`)"
                );
                return 1;
            }
        },
        None => {
            eprintln!("loft yank: --severity required");
            return 1;
        }
    };
    let Some(advisory) = advisory else {
        eprintln!("loft yank: --advisory <id> required (e.g. GHSA-xxxx-yyyy-zzzz)");
        return 1;
    };
    let Some(summary) = summary else {
        eprintln!("loft yank: --summary <text> required");
        return 1;
    };
    let affected_range = affected.unwrap_or(version);
    let published = chrono_iso8601_utc();

    println!("# 1) Edit `index.json` — set the typed `status` on the affected version.");
    println!("# Replace the existing `\"{version}\": {{ ... }}` entry for `{name}` with:");
    println!();
    println!("\"{version}\": {{");
    println!("  // ...existing fields (url, sha256, size, loft, subpath, deps, published)...");
    println!("  \"status\": {{");
    println!("    \"kind\": \"yanked\",");
    println!("    \"severity\": \"{severity}\",");
    println!("    \"advisory\": \"{advisory}\",");
    println!("    \"summary\": {}", escape_json_string(summary));
    println!("  }}");
    println!("}}");
    println!();
    println!("# 2) Edit `advisories.json` — append the cross-referenced row.");
    println!("# Insert at the top of the `\"advisories\": [ ... ]` array:");
    println!();
    println!("{{");
    println!("  \"id\": \"{advisory}\",");
    println!(
        "  \"packages\": [{{\"name\": \"{name}\", \"affected\": \"{affected_range}\"{}}}],",
        if let Some(fix) = fixed_in {
            format!(", \"fixed_in\": \"{fix}\"")
        } else {
            String::new()
        }
    );
    println!("  \"severity\": \"{severity}\",");
    println!("  \"summary\": {},", escape_json_string(summary));
    println!("  \"published\": \"{published}\",");
    println!("  \"references\": []");
    println!("}}");
    println!();
    println!("# Then:");
    println!("#   git checkout -b yank-{name}-{version}");
    println!("#   $EDITOR index.json   # apply edit 1");
    println!("#   $EDITOR advisories.json   # apply edit 2");
    println!("#   git commit -am 'yank: {name} {version} ({advisory})'");
    println!("#   gh pr create --title 'yank: {name} {version}' --body \"<rationale>\"");
    0
}

/// Quote + JSON-escape a string.  Minimal — handles `"`, `\\`,
/// `\n`, `\t`.
#[cfg(feature = "registry")]
fn escape_json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// @PLAN12 — `loft new <name>` scaffolds a fresh loft library
/// package, ready for development + `loft publish`.
///
/// Layout produced:
///
/// ```
/// <name>/
/// ├── loft.toml           — package manifest with [package] + [library]
/// ├── README.md           — placeholder header pointing at the registry
/// ├── release.sh          — one-command release: bump → test → tag → package → GH release
/// ├── src/<name>.loft     — empty entry file with a `pub fn hello()` stub
/// └── tests/
///     └── 01-smoke.loft   — single `test_smoke` exercising the stub
/// ```
///
/// With `--native`: also creates `native/Cargo.toml` + `native/src/lib.rs`
/// + `native/build.rs` for the cdylib bindings.
///
/// With `--chunk`: also creates `.github/workflows/library-ci.yml`
/// using the canonical template from
/// `doc/claude/lib_plans/12-library-extraction/library-ci.yml.example`
/// — for when the dir is becoming a fresh chunk-repo's first
/// library.
///
/// Refuses if `<name>/` already exists.
fn scaffold_library(name: &str, native: bool, chunk: bool) -> i32 {
    use std::io::Write as _;

    // The package-name rule lives in ONE place (`libscan::is_valid_package_name`)
    // because it is also what keeps a manifest-supplied name from walking out of
    // the directory it is joined into — see that function.
    if !loft::libscan::is_valid_package_name(name) {
        eprintln!(
            "loft new: library name must be lowercase ascii + digits + underscore (got `{name}`)"
        );
        return 1;
    }
    // A library may not claim a language-namespace name (`std`, `core`): those
    // resolve to a built-in namespace, not a package (@PLN13 / C101).
    if loft::libscan::is_reserved_package_name(name) {
        eprintln!(
            "loft new: `{name}` is a reserved namespace name (the standard library is `std::`); choose another library name"
        );
        return 1;
    }

    let pkg_dir = std::path::PathBuf::from(name);
    if pkg_dir.exists() {
        eprintln!("loft new: `{name}/` already exists; refusing to overwrite");
        return 1;
    }

    // Helper closures.
    let write_file = |rel: &str, content: &str| -> std::io::Result<()> {
        let path = pkg_dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut f = std::fs::File::create(&path)?;
        f.write_all(content.as_bytes())?;
        Ok(())
    };

    // loft.toml — includes [native] declaration when --native.
    let loft_toml = if native {
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
             description = \"One-line description of {name}.\"\n\n\
             [library]\nentry = \"src/{name}.loft\"\nnative = \"loft_{name}\"\n\n\
             [native]\ncrate = \"loft-{name}\"\n\n[dependencies]\n"
        )
    } else {
        format!(
            "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\nloft = \">=0.8\"\n\
             description = \"One-line description of {name}.\"\n\n\
             [library]\nentry = \"src/{name}.loft\"\n\n[dependencies]\n"
        )
    };

    // src/<name>.loft — placeholder stub.
    let src_loft = format!(
        "// Copyright (c) 2026\n// SPDX-License-Identifier: LGPL-3.0-or-later\n\n\
         // {name} — replace with a one-line description of the library.\n\n\
         // Returns the greeting.  Replace with the library's actual API.\n\
         pub fn hello() -> text {{\n  \"hello from {name}\"\n}}\n"
    );

    // tests/01-smoke.loft — minimal regression guard.
    let test_loft = format!(
        "// Smoke test for {name}.\n\
         use {name};\n\n\
         fn test_hello() {{\n  \
           greeting = {name}::hello();\n  \
           assert(greeting == \"hello from {name}\", \"hello() returned {{greeting ?? \\\"null\\\"}}\");\n\
         }}\n"
    );

    let readme = format!(
        "# {name}\n\n\
         <One-line description of the library.>\n\n\
         ## Install\n\n\
         ```\n\
         loft install {name}\n\
         ```\n\n\
         ## Usage\n\n\
         ```loft\n\
         use {name};\n\n\
         fn main() {{\n  \
           println({name}::hello());\n\
         }}\n\
         ```\n"
    );

    // release.sh — one command to cut a release: bump → test → tag → package →
    // GitHub release, with immutability + deterministic-package gates.  A plain
    // &str (no interpolation): it reads name + version from loft.toml at runtime.
    let release_sh = r#"#!/usr/bin/env bash
# release.sh — cut a release of THIS loft library so the registry can publish it.
# Run from the library directory (the one containing loft.toml).
#
#   ./release.sh            # release the version currently in loft.toml
#   ./release.sh 0.2.0      # set loft.toml to 0.2.0, commit, then release
#
# Reads name + version from loft.toml, runs the test gate + a determinism check,
# commits any version bump, tags <name>-v<version>, pushes the branch + tag,
# packages the tarball, and creates the GitHub release.  Releases are immutable:
# it refuses to re-cut an existing tag — bump the version instead.
#
# After a successful release:
#   * loft-lang family lib -> run scripts/registry_maintain.sh in loft-lang/loft
#     (publishes every stale/missing own lib + signs the registry index).
#   * external lib         -> `loft publish` then open a registry PR
#     (see LIBRARY_AUTHORING.md / REGISTRY_SUBMIT.md).
#
# Env: LOFT=/path/to/loft to use a non-PATH binary; SKIP_NATIVE=1 to skip the
#      `loft --native test` gate (e.g. no Rust toolchain on this machine).
set -euo pipefail
cd "$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"

LOFT="${LOFT:-loft}"
sha()   { if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1"; else shasum -a 256 "$1"; fi | cut -d' ' -f1; }
field() { sed -n "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*\"\(.*\)\".*/\1/p" loft.toml | head -1; }

[ -f loft.toml ] || { echo "release.sh: no loft.toml here — run from the library dir." >&2; exit 1; }
command -v gh >/dev/null 2>&1 || { echo "release.sh: needs the GitHub CLI (gh)." >&2; exit 1; }

# Push without a username/password prompt by reusing your gh login as git's
# credential helper (HTTPS remotes); harmless on SSH remotes (which use your
# key).  `gh release create` works via gh's API auth, but plain `git push` uses
# git's credential system — without a helper it prompts, and GitHub no longer
# accepts a password there.  GIT_TERMINAL_PROMPT=0 fails fast instead of hanging.
export GIT_TERMINAL_PROMPT=0
gitpush() {
    git -c credential.helper='!gh auth git-credential' push "$@" || {
        echo "release.sh: git push failed — set up auth once with 'gh auth setup-git'," >&2
        echo "             or use an SSH remote (git@github.com:OWNER/REPO.git)." >&2
        exit 1
    }
}
name=$(field name); [ -n "$name" ] || { echo "release.sh: no package name in loft.toml." >&2; exit 1; }

# Optional version bump.
if [ "${1:-}" != "" ]; then
    case "$1" in
        [0-9]*.[0-9]*.[0-9]*) : ;;
        *) echo "release.sh: '$1' is not an x.y.z version." >&2; exit 1 ;;
    esac
    tmp=$(mktemp)
    awk -v v="$1" '!d && /^[[:space:]]*version[[:space:]]*=/ {sub(/"[^"]*"/, "\"" v "\""); d=1} {print}' loft.toml >"$tmp"
    mv "$tmp" loft.toml
fi
ver=$(field version); [ -n "$ver" ] || { echo "release.sh: no version in loft.toml." >&2; exit 1; }
tag="$name-v$ver"
echo "== releasing $name $ver ($tag) =="

# Immutability — never re-cut an existing release; bump the version instead.
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null 2>&1 || gh release view "$tag" >/dev/null 2>&1; then
    echo "release.sh: $tag already exists. Bump first: ./release.sh <new x.y.z>" >&2
    exit 1
fi

# Gate 1 — tests pass (both backends unless SKIP_NATIVE=1).
echo "-- loft test"; "$LOFT" test
if [ "${SKIP_NATIVE:-}" != 1 ]; then echo "-- loft --native test (SKIP_NATIVE=1 to skip)"; "$LOFT" --native test; fi

# Commit the bump FIRST so the tag points at exactly the bytes we package.
if ! git diff --quiet -- loft.toml; then git add loft.toml && git commit -m "release: $name $ver"; fi
git diff --quiet && git diff --cached --quiet || {
    echo "release.sh: uncommitted changes — commit them so the tag matches the release." >&2; exit 1; }

# Gate 2 — packaging is deterministic (two clean builds must hash equal); this is
# the registry's gate-3 reproducible-build invariant, checked locally first.
"$LOFT" package >/dev/null; a=$(sha "$name-$ver.tar.gz"); rm -f "$name-$ver.tar.gz"
"$LOFT" package >/dev/null; b=$(sha "$name-$ver.tar.gz")
[ "$a" = "$b" ] || { echo "release.sh: non-deterministic package ($a != $b)." >&2; rm -f "$name-$ver.tar.gz"; exit 1; }

git tag "$tag"
gitpush origin HEAD
gitpush origin "$tag"
gh release create "$tag" "$name-$ver.tar.gz" --title "$name v$ver" --notes "Release $name $ver."
rm -f "$name-$ver.tar.gz"

echo
echo "released $tag."
echo "  loft-lang family lib -> run scripts/registry_maintain.sh in loft-lang/loft."
echo "  external lib         -> loft publish + open a registry PR."
"#;

    if let Err(e) = (|| -> std::io::Result<()> {
        write_file("loft.toml", &loft_toml)?;
        write_file(&format!("src/{name}.loft"), &src_loft)?;
        write_file("tests/01-smoke.loft", &test_loft)?;
        write_file("README.md", &readme)?;
        write_file("release.sh", release_sh)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let p = pkg_dir.join("release.sh");
            let mut perm = std::fs::metadata(&p)?.permissions();
            perm.set_mode(0o755);
            std::fs::set_permissions(&p, perm)?;
        }
        if native {
            let cargo_toml = format!(
                "[package]\nname = \"loft-{name}\"\nversion = \"0.1.0\"\nedition = \"2024\"\nlicense = \"LGPL-3.0-or-later\"\n\n\
                 [lib]\ncrate-type = [\"cdylib\", \"rlib\"]\n\n\
                 [dependencies]\nloft-ffi = \"0.1\"\n\n\
                 [build-dependencies]\nloft-ffi-build = \"0.2\"\n"
            );
            let build_rs =
                "fn main() {\n    loft_ffi_build::generate_register_from_loft(\"../src\");\n}\n";
            let lib_rs = "// Native bindings for the loft library.\n\
                 // Add `#[unsafe(no_mangle)] pub extern \"C\" fn n_<name>(...)` here for each\n\
                 // function whose loft signature is annotated `#native`.\n\n\
                 include!(concat!(env!(\"OUT_DIR\"), \"/loft_register_gen.rs\"));\n";
            write_file("native/Cargo.toml", &cargo_toml)?;
            write_file("native/build.rs", build_rs)?;
            write_file("native/src/lib.rs", lib_rs)?;
        }
        if chunk {
            // Canonical CI YAML — single-package matrix; user edits when adding more.
            let yml = format!(
                "name: library-ci\n\n\
                 on:\n  push:\n    branches: [main]\n  pull_request:\n\n\
                 jobs:\n  test:\n    runs-on: ubuntu-latest\n    \
                 strategy:\n      fail-fast: false\n      matrix:\n        package: [{name}]\n    \
                 steps:\n      - uses: actions/checkout@v4\n\n      \
                 - name: Install mold (loft's pinned linker)\n        \
                 run: sudo apt-get update -y && sudo apt-get install -y mold\n\n      \
                 - name: Clone loft source\n        uses: actions/checkout@v4\n        \
                 with:\n          repository: loft-lang/loft\n          path: loft-src\n\n      \
                 - name: Cache cargo registry + loft build\n        uses: actions/cache@v4\n        \
                 with:\n          path: |\n            ~/.cargo/registry\n            ~/.cargo/git\n            loft-src/target\n          \
                 key: loft-${{{{ hashFiles('loft-src/Cargo.lock') }}}}\n\n      \
                 - name: Build loft\n        working-directory: loft-src\n        \
                 run: |\n          cargo build --release --lib --bin loft\n          \
                 echo \"$PWD/target/release\" >> $GITHUB_PATH\n\n      \
                 - name: Interpreter — loft test\n        working-directory: ${{{{ matrix.package }}}}\n        \
                 env:\n          LOFT_DENY_WARNINGS: ${{{{ hashFiles(format('{{0}}/.allow_warnings', matrix.package)) != '' && '0' || '1' }}}}\n        \
                 run: loft --interpret --tests tests\n\n      \
                 - name: Native — loft --native test\n        working-directory: ${{{{ matrix.package }}}}\n        \
                 env:\n          LOFT_DENY_WARNINGS: ${{{{ hashFiles(format('{{0}}/.allow_warnings', matrix.package)) != '' && '0' || '1' }}}}\n        \
                 run: loft --native --tests tests\n"
            );
            write_file(".github/workflows/library-ci.yml", &yml)?;
        }
        Ok(())
    })() {
        eprintln!("loft new: failed to write scaffolding: {e}");
        // Best-effort cleanup
        let _ = std::fs::remove_dir_all(&pkg_dir);
        return 1;
    }

    println!("Created library `{name}/`:");
    println!("  loft.toml");
    println!("  src/{name}.loft");
    println!("  tests/01-smoke.loft");
    println!("  README.md");
    println!("  release.sh");
    if native {
        println!("  native/Cargo.toml");
        println!("  native/build.rs");
        println!("  native/src/lib.rs");
    }
    if chunk {
        println!("  .github/workflows/library-ci.yml");
    }
    println!();
    println!("Next steps:");
    println!("  cd {name}");
    println!("  loft test           # exercises the smoke test");
    println!("  $EDITOR src/{name}.loft   # add your library's API");
    println!("  ./release.sh        # when ready: test -> tag -> package -> GH release");
    0
}

/// @PLAN12 Phase 6.16 — `loft publish` author helper.
///
/// Closes the publish-by-hand friction.  After the author has
/// tagged + released their library on GitHub (`<pkg>-v<ver>` tag
/// and `gh release create` with the `loft package` tarball as an
/// asset), `loft publish` from the package dir:
///
/// 1. Re-packages locally via `package::package_create` — gets
///    the deterministic sha256 + size + name + version.
/// 2. Detects the chunk repo from `git remote get-url origin`.
/// 3. Verifies the GitHub release exists at `<pkg>-v<ver>` and
///    carries the expected tarball asset (via `gh release view`).
/// 4. Emits the `index.json` entry block, ready to paste into
///    the registry PR.  Includes the auto-generated `url`,
///    `sha256`, `size`, `subpath`, `deps` from the loft.toml.
///
/// `--dry-run` skips the GitHub-release verification (the rest
/// of the flow runs).
///
/// Auto-PR open via `gh pr create` against `loft-lang/registry`
/// is a follow-up; today the author copies the emitted block
/// into a manual PR.
/// Author-side pre-check for `loft publish`: warn when any trigger the package
/// is about to claim is already owned by a DIFFERENT package in the locally
/// cached registry catalog.  The registry CI (`validate.py` gate 4) enforces the
/// same uniqueness as a hard error; this is the early, offline heads-up so the
/// author fixes the clash before opening the PR.  Best-effort: silent when no
/// catalog is cached (nothing to check against) or the index won't parse.
#[cfg(feature = "registry")]
fn warn_trigger_collisions(pkg_name: &str, triggers: &[String]) {
    if triggers.is_empty() {
        return;
    }
    let (idx_path, _, _) = loft::registry_index::index_paths();
    let Ok(content) = std::fs::read_to_string(&idx_path) else {
        return;
    };
    let Ok(index) = loft::registry_index::parse_index(&content) else {
        return;
    };
    let owners = loft::registry_index::trigger_owners(&index);
    for trig in triggers {
        if let Some(owner) = owners.get(trig)
            && owner != pkg_name
        {
            eprintln!(
                "warning: trigger `{trig}` is already claimed by `{owner}` in the registry; the \
                 submission PR will be REJECTED — a method-on-type trigger must be unique. Rename \
                 the method in `{pkg_name}` or drop its `[triggers]` opt-in."
            );
        }
    }
}

#[cfg(feature = "registry")]
fn publish_package(pkg_path: &std::path::Path, dry_run: bool) -> i32 {
    use loft::package;

    let pkg = match package::package_create(pkg_path, None) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("loft publish: package: {e}");
            return 1;
        }
    };
    // Step 5a: refuse to emit a registry entry for a package that has not
    // declared its three compatibility levels.  Checked BEFORE the GitHub
    // release lookup, so the author learns it needs two more lines while the
    // fix is still a commit — not after they have tagged and released.
    let levels_manifest =
        loft::manifest::read_manifest(&pkg_path.join("loft.toml").to_string_lossy())
            .unwrap_or_default();
    let levels = match package::declared_levels(&levels_manifest, &pkg.version) {
        Ok(l) => l,
        Err(problems) => {
            eprintln!(
                "loft publish: {}",
                package::declared_levels_error(&pkg.name, &pkg.version, &problems)
            );
            return 1;
        }
    };

    let tag = format!("{}-v{}", pkg.name, pkg.version);
    let tarball_filename = format!("{}-{}.tar.gz", pkg.name, pkg.version);

    let (org, repo) = match package::git_remote_org_repo(pkg_path) {
        Some(v) => v,
        None => {
            eprintln!(
                "loft publish: cannot detect GitHub org/repo from `git remote get-url origin`"
            );
            eprintln!("  Run from inside a chunk-repo working tree with an `origin` remote.");
            return 1;
        }
    };
    let release_url =
        format!("https://github.com/{org}/{repo}/releases/download/{tag}/{tarball_filename}");
    let homepage = format!("https://github.com/{org}/{repo}/tree/main/{}", pkg.name);

    if !dry_run {
        if !github_release_has_asset(&org, &repo, &tag, &tarball_filename) {
            eprintln!(
                "loft publish: release `{tag}` not found, OR doesn't carry the asset `{tarball_filename}`."
            );
            eprintln!("  Tag + release first:");
            eprintln!("    git tag {tag}");
            eprintln!("    git push origin {tag}");
            eprintln!(
                "    gh release create {tag} {} --title \"{} v{}\"",
                pkg.tarball.display(),
                pkg.name,
                pkg.version
            );
            eprintln!("  Or pass --dry-run to skip this check.");
            return 1;
        }
    }

    // Emit the index.json entry.  Read deps from loft.toml.
    let manifest_path = pkg_path.join("loft.toml");
    let manifest =
        loft::manifest::read_manifest(manifest_path.to_str().unwrap_or("")).unwrap_or_default();
    let registry_deps: Vec<(String, String)> = manifest
        .dependencies
        .iter()
        .filter(|(_, v)| loft::manifest::extract_path_dep(v).is_none())
        .map(|(n, v)| (n.clone(), v.clone()))
        .collect();
    let published = chrono_iso8601_utc();
    // loft#1136 — `{}` is a CLAIM, and the manifest is the only thing this command can read
    // it from.  A multi-package repo deliberately keeps its registry deps out of `loft.toml`
    // (declaring them there resolves from the registry instead of the `--lib` path, which
    // breaks multi-library consumption), so for those packages an empty `[dependencies]`
    // means "not stated here", not "none" — and pasting the entry verbatim publishes a
    // version that resolves nothing.  The package's own source says which it is.
    let undeclared = undeclared_source_deps(pkg_path, &pkg.name, &registry_deps);

    println!("# Paste this entry into `loft-lang/registry/index.json` under");
    println!(
        "# `\"packages\": {{ \"{}\": {{ \"versions\": {{ ... }} }} }}`:",
        pkg.name
    );
    println!();
    println!("\"{}\": {{", pkg.version);
    println!("  \"url\": \"{release_url}\",");
    println!("  \"sha256\": \"{}\",", pkg.sha256);
    println!("  \"size\": {},", pkg.size);
    println!("  \"loft\": \"{}\",", levels.loft);
    // The two floors travel WITH the version they describe, so a resolver can
    // read a release's promises straight from the index instead of downloading
    // and unpacking the tarball to find its loft.toml (step 6).
    println!("  \"api_compatible_with\": \"{}\",", levels.api);
    println!("  \"data_compatible_with\": \"{}\",", levels.data);
    println!("  \"subpath\": \"{}\",", pkg.name);
    if registry_deps.is_empty() {
        println!("  \"deps\": {{}},");
        if !undeclared.is_empty() {
            // On STDERR, beside the other `[publish]` status: stdout is the entry a
            // script parses as JSON, and JSON has no comment syntax — a note written
            // between the members made the whole entry unparseable, which is how a
            // publish run died after it had already cut the GitHub release.  The
            // first line is the machine-readable half (`registry_maintain.sh` reads
            // it to reconcile `deps` against the previous entry); the rest is for
            // whoever is pasting by hand.
            eprintln!("[publish] deps-incomplete: {}", undeclared.join(", "));
            eprintln!(
                "  `deps` above is `{{}}`, but this package's source uses {}, and `loft.toml`",
                undeclared.join(", ")
            );
            eprintln!("  declares none of them, so there was nothing here to read the versions");
            eprintln!("  from.  Copy `deps` from this package's existing index entries before");
            eprintln!("  pasting; an entry with empty deps installs a version that resolves none");
            eprintln!("  of them.");
        }
    } else {
        let mut deps_lines: Vec<String> = Vec::new();
        for (n, v) in &registry_deps {
            // Strip `{ path = "..." }` -> bare; we already filtered
            // those, so `v` is a plain version string.
            deps_lines.push(format!("    \"{n}\": \"{v}\""));
        }
        println!("  \"deps\": {{");
        for (idx, line) in deps_lines.iter().enumerate() {
            if idx + 1 == deps_lines.len() {
                println!("{line}");
            } else {
                println!("{line},");
            }
        }
        println!("  }},");
    }
    // Tier-1 trigger surface — derived from the package source at publish time
    // (lib_plans/59-lazy-stdlib), so a CONSUMER's resolver can map
    // `obj.method()` to this package without the source.  Emitted only when the
    // package opts in via `[triggers] enabled`; nothing is hand-listed.
    if manifest.trigger_enabled {
        let entry = manifest
            .entry
            .clone()
            .unwrap_or_else(|| format!("src/{}.loft", pkg.name));
        let src = std::fs::read_to_string(pkg_path.join(&entry)).unwrap_or_default();
        let triggers: Vec<String> = loft::triggers::derive_triggers(&src)
            .methods
            .iter()
            .map(|m| format!("{}:{}", m.name, m.receiver))
            .collect();
        warn_trigger_collisions(&pkg.name, &triggers);
        let quoted: Vec<String> = triggers.iter().map(|t| format!("\"{t}\"")).collect();
        println!("  \"triggers\": [{}],", quoted.join(", "));
    }
    // Function-level API surface (S6/S7) — derived from ALL of the package's
    // `src/*.loft` at publish time (the same `parse_pkg_api` extractor `loft api`
    // uses), so `loft search` can surface THIS version's functions without the
    // source.  Emitted automatically (nothing hand-written), the exact mirror of
    // `triggers`; the registry CI re-derives + verifies it from source (S7), so
    // the pasted field is a pure function of the code and cannot drift.
    let api_json = api_items_json(&loft::documentation::pkg_api_items(pkg_path));
    println!("  \"api\": {},", loft::json::to_json_string(&api_json));
    println!("  \"published\": \"{published}\"");
    println!("}}");
    println!();
    println!("# If this is the first version, also add the package block:");
    println!("# \"{}\": {{", pkg.name);
    println!("#   \"description\": \"<one-liner>\",");
    println!("#   \"homepage\": \"{homepage}\",");
    println!("#   \"categories\": [\"<category>\"],");
    println!("#   \"yanked\": [],");
    println!("#   \"versions\": {{ ... the version block above ... }}");
    println!("# }}");
    if dry_run {
        eprintln!("\n[publish] dry-run: GitHub release verification skipped.");
    } else {
        eprintln!("\n[publish] verified release {tag} exists with asset {tarball_filename}");
        eprintln!("[publish] next step: open registry PR with the entry above");
    }
    0
}

/// Check whether a GitHub release at `<tag>` exists and carries
/// an asset named `<asset>` (the deterministic tarball).  Uses
/// the `gh` CLI; returns false if `gh` is missing or the API
/// call fails.
#[cfg(feature = "registry")]
fn github_release_has_asset(org: &str, repo: &str, tag: &str, asset: &str) -> bool {
    let out = std::process::Command::new("gh")
        .args([
            "release",
            "view",
            tag,
            "--repo",
            &format!("{org}/{repo}"),
            "--json",
            "assets",
        ])
        .output();
    let Ok(out) = out else {
        return false;
    };
    if !out.status.success() {
        return false;
    }
    let body = String::from_utf8(out.stdout).unwrap_or_default();
    // Simple substring check — JSON parse would be more
    // principled but `gh release view --json assets` returns
    // `"name":"<asset>"` lines we can match directly.
    body.contains(&format!("\"name\":\"{asset}\""))
}

/// @PLAN12 Phase 6.7 — `loft audit` walks every installed package
/// in `~/.loft/registry/` and classifies each against the cached
/// advisory feed.  Exit code reflects worst severity found:
/// - 0 → clean (no matches)
/// - 1 → at least one low / bug / deprecated match
/// - 2 → at least one security_high match
/// - 3 → at least one security_critical match
///
/// Refreshes the advisory feed if stale (24h TTL) UNLESS
/// `LOFT_OFFLINE=1` is set, in which case it falls back to the
/// cached copy (or warns + returns code 0 if no cache).
///
/// Pure deep scan — no installs, no writes.  The natural
/// companion to `loft list-installed`.
#[cfg(feature = "registry")]
fn audit_installed() -> i32 {
    use loft::registry_advisories::{self, LoadOptions, Severity};

    let offline = std::env::var("LOFT_OFFLINE").is_ok();
    let opts = LoadOptions {
        allow_unsigned: true,
        offline,
        refresh: false,
    };
    let feed = match registry_advisories::load_or_fetch(&opts) {
        Ok(Some(f)) => f,
        Ok(None) => {
            if offline {
                eprintln!(
                    "[audit] advisory feed not cached and offline; can't audit (exit 0 as no-evidence)"
                );
            } else {
                eprintln!(
                    "[audit] advisory feed unavailable (registry may not host one yet); nothing to check"
                );
            }
            return 0;
        }
        Err(e) => {
            eprintln!("loft audit: {e}");
            return 4;
        }
    };

    // Enumerate cached installs (same logic as list-installed).
    let cache = loft::registry_index::cache_dir();
    let read = match std::fs::read_dir(&cache) {
        Ok(r) => r,
        Err(_) => {
            println!("No registry cache; nothing to audit.");
            return 0;
        }
    };
    let mut entries: Vec<(String, String)> = Vec::new();
    for ent in read.filter_map(Result::ok) {
        let path = ent.path();
        if !path.is_dir() {
            continue;
        }
        let dirname = match path.file_name().and_then(|s| s.to_str()) {
            Some(n) => n.to_string(),
            None => continue,
        };
        let mut split: Option<usize> = None;
        let bytes = dirname.as_bytes();
        let mut i = 1;
        while i < bytes.len() {
            if bytes[i - 1] == b'-' && bytes[i].is_ascii_digit() {
                split = Some(i - 1);
                break;
            }
            i += 1;
        }
        let Some(at) = split else { continue };
        let (name, rest) = dirname.split_at(at);
        let version = rest.trim_start_matches('-').to_string();
        if !name.is_empty() && !version.is_empty() {
            entries.push((name.to_string(), version));
        }
    }
    entries.sort();

    let mut all_classifications = Vec::new();
    for (name, version) in &entries {
        let hits = registry_advisories::classify(name, version, &feed);
        for hit in hits {
            all_classifications.push(hit);
        }
    }

    if all_classifications.is_empty() {
        println!(
            "{} installed package(s) audited against {} advisory entries — clean",
            entries.len(),
            feed.advisories.len()
        );
        return 0;
    }

    // Compute worst severity → exit code.
    let mut worst = 0u8;
    println!(
        "Audit found {} advisory match(es) across {} installed package(s):",
        all_classifications.len(),
        entries.len()
    );
    for hit in &all_classifications {
        worst = worst.max(hit.severity.rank());
        let prefix = match hit.severity {
            Severity::SecurityCritical => "ERROR",
            Severity::SecurityHigh => "WARN",
            Severity::SecurityLow | Severity::Bug => "NOTE",
            Severity::Deprecated => "INFO",
        };
        println!(
            "  [{prefix}] {pkg} {ver}  {sev}  {summary}",
            pkg = hit.package,
            ver = hit.version,
            sev = hit.severity.as_str(),
            summary = hit.summary,
        );
        println!("         advisory: {}", hit.advisory_id);
        if let Some(fix) = &hit.fixed_in {
            println!(
                "         fix: {pkg} >= {fix} (run `loft install {pkg}@{fix}`)",
                pkg = hit.package
            );
        }
        for r in &hit.references {
            println!("         reference: {r}");
        }
    }

    // Worst severity → exit code:
    //   Deprecated (rank 1) → 1
    //   Low / Bug (rank 2)  → 1
    //   High (rank 3)       → 2
    //   Critical (rank 4)   → 3
    match worst {
        1 | 2 => 1,
        3 => 2,
        4 => 3,
        _ => 0,
    }
}

/// Total size in bytes of a directory's contents (recursive).
/// Used by `list-installed`; cheap enough for the typical
/// ~/.loft/registry/<pkg>-<ver>/ layout (a few hundred files at
/// most per package).  Returns None on read errors so callers
/// can degrade to "size unknown" rather than failing the whole
/// listing.
#[cfg(feature = "registry")]
fn dir_size_bytes(dir: &std::path::Path) -> Option<u64> {
    let mut total: u64 = 0;
    let mut stack: Vec<std::path::PathBuf> = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let read = std::fs::read_dir(&d).ok()?;
        for ent in read.filter_map(Result::ok) {
            let p = ent.path();
            let m = match ent.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };
            if m.is_dir() {
                stack.push(p);
            } else if m.is_file() {
                total += m.len();
            }
        }
    }
    Some(total)
}

/// @PLAN12 Phase 6.6 — `loft pin <script>` writes a sidecar
/// `<script>.loft.lock` next to the script.  Subsequent runs of
/// that script resolve registry libraries via the sidecar (see
/// `src/parser/mod.rs::probe_sidecar_lockfile`), so the script
/// becomes reproducible regardless of cwd or registry drift.
///
/// Implementation: scans the script source for `use <name>;`
/// declarations, installs each name that exists in the registry
/// catalog (calls `install_one` with `lock_path` redirected to
/// the sidecar), and prints a summary.  Imports that aren't
/// registry packages (path-deps, sibling packages, stdlib) are
/// ignored — only registry libraries need pinning; everything
/// else either lives in the workspace or is part of loft itself.
#[cfg(feature = "registry")]
fn pin_script(script: &str) {
    use loft::install::{InstallOptions, install_one};
    use loft::registry_index;
    use std::path::PathBuf;

    let script_path = PathBuf::from(script);
    if !script_path.exists() {
        eprintln!("loft pin: script `{}` not found", script_path.display());
        std::process::exit(1);
    }
    let source = match std::fs::read_to_string(&script_path) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("loft pin: cannot read `{}`: {e}", script_path.display());
            std::process::exit(1);
        }
    };

    // Extract `use <name>;` declarations.  Simple line-scan rather
    // than a full parser pass — we want to pin even scripts that
    // have parse errors elsewhere.  Loft `use` syntax forms:
    //   use foo;
    //   use foo::bar;
    //   use foo::{a, b};
    //   use foo::*;
    // All start with `use <ident>` after stripping leading whitespace
    // + skipping comment lines.
    let mut uses: Vec<String> = Vec::new();
    for line in source.lines() {
        let trimmed = line.trim_start();
        // Skip comments and blank lines.
        if trimmed.is_empty() || trimmed.starts_with("//") {
            continue;
        }
        let Some(rest) = trimmed.strip_prefix("use ") else {
            continue;
        };
        // Take the leading identifier (alphanumeric + underscore).
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if name.is_empty() || uses.contains(&name) {
            continue;
        }
        uses.push(name);
    }

    if uses.is_empty() {
        eprintln!(
            "loft pin: no `use` declarations found in `{}`",
            script_path.display()
        );
        std::process::exit(1);
    }

    // Sidecar path next to the script.
    let mut sidecar = script_path.clone();
    let sidecar_name = format!(
        "{}.lock",
        script_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("script.loft")
    );
    sidecar.set_file_name(sidecar_name);

    // Load index once so we can filter out non-registry names
    // (path-deps + stdlib + sibling packages) before invoking
    // install_one — saves a network hit + a confusing error.
    let opts_for_index = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: None,
    };
    let index = match loft::install::load_index(&opts_for_index) {
        Ok(i) => i,
        Err(e) => {
            eprintln!("loft pin: {e}");
            std::process::exit(1);
        }
    };

    let opts = InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
        skip_lockfile: false,
        lock_path: Some(sidecar.clone()),
    };

    let mut pinned: Vec<(String, String)> = Vec::new();
    let mut skipped: Vec<String> = Vec::new();
    for name in &uses {
        if !index.packages.contains_key(name) {
            skipped.push(name.clone());
            continue;
        }
        match install_one(name, None, &opts) {
            Ok(report) => {
                for (n, v) in report.installed.iter().chain(report.skipped_cached.iter()) {
                    if !pinned.iter().any(|(pn, _)| pn == n) {
                        pinned.push((n.clone(), v.clone()));
                    }
                }
            }
            Err(e) => {
                eprintln!("loft pin: install `{name}` failed: {e}");
                std::process::exit(1);
            }
        }
    }

    if pinned.is_empty() {
        eprintln!(
            "loft pin: no registry libraries in `{}` (only sibling / stdlib uses)",
            script_path.display()
        );
        std::process::exit(1);
    }

    println!("Pinned to {}:", sidecar.display());
    for (n, v) in &pinned {
        println!("  {n} {v}");
    }
    if !skipped.is_empty() {
        println!("Not from registry (left unresolved):");
        for n in &skipped {
            println!("  {n}");
        }
    }
    println!("{} library(ies) pinned", pinned.len());
    // PKG.STUB — stubs ride the sidecar lockfile, landing next to the script.
    let script_dir = script_path.parent().map_or_else(
        || std::path::PathBuf::from("."),
        std::path::Path::to_path_buf,
    );
    write_api_stubs(&sidecar, &script_dir);
    // Keep registry_index in the symbol table so the cfg above
    // doesn't drop the import.
    let _ = registry_index::cache_dir();
}

#[cfg(feature = "registry")]
fn install_from_registry_legacy(arg: &str) {
    use loft::registry;

    // Parse name[@version].
    let (name, version) = if let Some((n, v)) = arg.split_once('@') {
        (n, Some(v))
    } else {
        (arg, None)
    };

    // Find and read registry file.
    let Some(reg_path) = registry::registry_path() else {
        eprintln!(
            "loft install: no registry file found.\n  \
             Run 'loft registry sync' to download the package registry.\n  \
             Or set LOFT_REGISTRY to a local registry file path."
        );
        std::process::exit(1);
    };
    let (entries, _) = registry::read_registry(reg_path.to_str().unwrap_or(""));

    // Find matching entry.
    let Some(entry) = registry::find_package(&entries, name, version) else {
        let available: Vec<&str> = entries
            .iter()
            .filter(|e| e.name == name && !e.is_yanked())
            .map(|e| e.version.as_str())
            .collect();
        if available.is_empty() {
            eprintln!("loft install: package '{name}' not found in registry.");
        } else {
            eprintln!(
                "loft install: package '{name}@{}' not found in registry.\n  Available versions: {}",
                version.unwrap_or("?"),
                available.join(", ")
            );
        }
        std::process::exit(1);
    };

    if entry.is_yanked() && version.is_some() {
        eprintln!(
            "warning: {name}@{} is yanked ({})",
            entry.version,
            entry.status_slug()
        );
    }

    // Check if already installed.
    let lib = registry::lib_dir();
    let installed_toml = lib.join(name).join("loft.toml");
    if installed_toml.exists()
        && let Ok(content) = std::fs::read_to_string(&installed_toml)
    {
        let installed_ver = extract_toml_version(&content);
        if installed_ver == entry.version {
            println!(
                "loft install: {name} {} is already installed.",
                entry.version
            );
            return;
        }
    }

    // Download and extract.
    let tmp = std::env::temp_dir().join("loft_install");
    let _ = std::fs::create_dir_all(&tmp);
    match registry::download_and_extract(entry, &tmp) {
        Ok(pkg_root) => {
            install_package(&pkg_root);
            // Clean up temp.
            let _ = std::fs::remove_dir_all(&tmp);
        }
        Err(e) => {
            eprintln!("loft install: {e}");
            let _ = std::fs::remove_dir_all(&tmp);
            std::process::exit(1);
        }
    }
}

#[cfg(not(feature = "registry"))]
fn install_from_registry(arg: &str) {
    eprintln!(
        "loft install: registry support is not compiled in.\n  \
         Rebuild with: cargo build --features registry\n  \
         Trying to install: {arg}"
    );
    std::process::exit(1);
}

/// Extract version string from `loft.toml` content.
#[cfg(feature = "registry")]
fn extract_toml_version(content: &str) -> String {
    for line in content.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("version") {
            let rest = rest.trim();
            if let Some(rest) = rest.strip_prefix('=') {
                return rest.trim().trim_matches('"').to_string();
            }
        }
    }
    String::new()
}

/// PKG.6a: Generate Rust stubs for all `#native` declarations in a package.
///
/// Reads the package's `.loft` entry file, finds all `#native "symbol"`
/// declarations, and emits a Rust source file with the correct C-ABI
/// signatures plus `todo!()` bodies.
fn generate_native_stubs(pkg_path: &std::path::Path) {
    use loft::data::{DefType, Type};

    let toml_path = pkg_path.join("loft.toml");
    if !toml_path.exists() {
        eprintln!("Error: no loft.toml in {}", pkg_path.display());
        std::process::exit(1);
    }
    let manifest = match loft::manifest::read_manifest(&toml_path.to_string_lossy()) {
        Some(m) => m,
        None => {
            eprintln!("Error: cannot read {}", toml_path.display());
            std::process::exit(1);
        }
    };
    let entry = manifest
        .entry
        .as_deref()
        .map(|e| pkg_path.join(e))
        .unwrap_or_else(|| {
            let name = manifest.name.as_deref().unwrap_or("lib");
            pkg_path.join(format!("src/{name}.loft"))
        });
    if !entry.exists() {
        eprintln!("Error: entry file {} not found", entry.display());
        std::process::exit(1);
    }

    // Parse just enough to read definitions.
    let abs = portable_path::plain_canonical(&entry);
    let dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_default();
    let default_dir = dir.join("../default");
    let default_str = if default_dir.exists() {
        default_dir.to_string_lossy().to_string()
    } else {
        "default".to_string()
    };

    let mut p = parser::Parser::new();
    if let Some(src_dir) = entry.parent() {
        p.lib_dirs.push(src_dir.to_string_lossy().to_string());
    }
    // Load default definitions so types are known.
    let _ = p.parse_dir(&default_str, true, false);
    p.parse(&abs.to_string_lossy(), false);

    // Collect #native declarations.
    let mut stubs: Vec<String> = Vec::new();
    // Map struct name → (d_nr, fields) for generating field offset constants.
    let mut struct_field_mods: std::collections::HashMap<
        String,
        (u32, Vec<(String, usize, Type)>),
    > = std::collections::HashMap::new();
    for d_nr in 0..p.data.definitions() {
        let def = p.data.def(d_nr);
        if def.native.is_empty() || !matches!(def.def_type, DefType::Function) {
            continue;
        }
        let sym = &def.native;

        let mut c_params: Vec<String> = Vec::new();
        let mut body_lines: Vec<String> = Vec::new();
        let mut param_names: Vec<String> = Vec::new();

        for attr in &def.attributes {
            let name = &attr.name;
            match &attr.typedef {
                // Post-2c round 10c: wide Type::Integer (former Type::Long) → i64.
                Type::Integer(s) if s.is_wide() => {
                    c_params.push(format!("{name}: i64"));
                    param_names.push(name.clone());
                }
                Type::Integer(_) | Type::Character => {
                    c_params.push(format!("{name}: i32"));
                    param_names.push(name.clone());
                }
                Type::Float => {
                    c_params.push(format!("{name}: f64"));
                    param_names.push(name.clone());
                }
                Type::Single => {
                    c_params.push(format!("{name}: f32"));
                    param_names.push(name.clone());
                }
                Type::Boolean => {
                    c_params.push(format!("{name}: bool"));
                    param_names.push(name.clone());
                }
                Type::Text(_) => {
                    c_params.push(format!("{name}_ptr: *const u8, {name}_len: usize"));
                    body_lines.push(format!(
                        "    let {name} = unsafe {{ loft_ffi::text({name}_ptr, {name}_len) }};"
                    ));
                    param_names.push(name.clone());
                }
                Type::Enum(_, false, _) => {
                    // Simple enum (tag only) — passed as u8.
                    c_params.push(format!("{name}: u8"));
                    param_names.push(name.clone());
                }
                Type::Reference(_, _)
                | Type::Vector(_, _)
                | Type::Enum(_, true, _)
                | Type::Sorted(_, _, _)
                | Type::Index(_, _, _)
                | Type::Hash(_, _, _)
                | Type::Radix(_, _, _)
                | Type::Trie(_, _, _) => {
                    let type_name = p.data.type_name_str(&attr.typedef);
                    c_params.push(format!("{name}: loft_ffi::LoftRef /* {type_name} */"));
                    param_names.push(name.clone());
                }
                other => {
                    let type_name = p.data.type_name_str(other);
                    c_params.push(format!(
                        "{name}: () /* {type_name} — not supported in native */"
                    ));
                    param_names.push(name.clone());
                }
            }
        }

        // Return type classification: text, ref, or scalar.
        enum RetKind {
            None,
            Scalar(String),
            Text,
            Ref(String),
        }
        let ret_type_name = p.data.type_name_str(&def.returned);
        let ret_kind = match &def.returned {
            Type::Void | Type::Null => RetKind::None,
            // Post-2c round 10c: wide Type::Integer (former Type::Long) → i64.
            Type::Integer(s) if s.is_wide() => RetKind::Scalar(" -> i64".into()),
            Type::Integer(_) | Type::Character => RetKind::Scalar(" -> i32".into()),
            Type::Float => RetKind::Scalar(" -> f64".into()),
            Type::Single => RetKind::Scalar(" -> f32".into()),
            Type::Boolean => RetKind::Scalar(" -> bool".into()),
            Type::Text(_) => RetKind::Text,
            Type::Enum(_, false, _) => RetKind::Scalar(" -> u8".into()),
            Type::Reference(_, _)
            | Type::Vector(_, _)
            | Type::Enum(_, true, _)
            | Type::Sorted(_, _, _)
            | Type::Index(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Trie(_, _, _) => {
                RetKind::Ref(format!(" -> loft_ffi::LoftRef /* {ret_type_name} */"))
            }
            _ => RetKind::Scalar(format!(
                " -> () /* {ret_type_name} — not supported in native */"
            )),
        };
        let ret_ty = match &ret_kind {
            RetKind::None => String::new(),
            RetKind::Scalar(s) | RetKind::Ref(s) => s.clone(),
            RetKind::Text => " -> loft_ffi::LoftStr".into(),
        };
        let has_return = !matches!(ret_kind, RetKind::None);

        let has_text_param = def
            .attributes
            .iter()
            .any(|a| matches!(a.typedef, Type::Text(_)));
        let has_ref_param = def
            .attributes
            .iter()
            .any(|a| crate::data::is_dbref(&a.typedef));
        let has_ref_ret = crate::data::is_dbref(&def.returned);

        // If any param or return is a Ref, prepend LoftStore as first C-ABI param.
        if has_ref_param || has_ref_ret {
            c_params.insert(0, "store: loft_ffi::LoftStore".to_string());
        }

        let needs_unsafe = has_text_param || has_ref_param || has_ref_ret;
        let unsafe_kw = if needs_unsafe { "unsafe " } else { "" };

        // Collect struct types referenced as params for field offset generation.
        for attr in &def.attributes {
            if let Type::Reference(d_nr, _) = &attr.typedef {
                let struct_def = p.data.def(*d_nr);
                if !struct_def.attributes.is_empty() {
                    let sname = struct_def.name.to_lowercase();
                    if !struct_field_mods.contains_key(&sname) {
                        let mut fields = Vec::new();
                        for (i, f) in struct_def.attributes.iter().enumerate() {
                            fields.push((f.name.clone(), i, f.typedef.clone()));
                        }
                        struct_field_mods.insert(sname, (*d_nr, fields));
                    }
                }
            }
        }

        // Format parameter list — wrap if longer than 90 chars.
        let params_joined = c_params.join(", ");
        let sig_line = format!("pub {unsafe_kw}extern \"C\" fn {sym}({params_joined}){ret_ty}");
        let params_str = if sig_line.len() > 95 && c_params.len() > 1 {
            format!("\n    {},\n", c_params.join(",\n    "))
        } else {
            params_joined
        };

        let mut stub = format!(
            "#[unsafe(no_mangle)]\npub {unsafe_kw}extern \"C\" fn {sym}({params_str}){ret_ty} {{\n"
        );
        for line in &body_lines {
            stub.push_str(line);
            stub.push('\n');
        }

        let args = param_names.join(", ");
        match &ret_kind {
            RetKind::Text => {
                stub.push_str(&format!(
                    "    let result: String = todo!(\"implement {sym}({args})\");\n"
                ));
                stub.push_str("    loft_ffi::ret(result)\n");
            }
            RetKind::Ref(_) => {
                stub.push_str(&format!(
                    "    let result: loft_ffi::LoftRef = todo!(\"implement {sym}({args})\");\n"
                ));
                stub.push_str("    result\n");
            }
            _ if has_return => {
                stub.push_str(&format!("    todo!(\"implement {sym}({args})\")\n"));
            }
            _ if !param_names.is_empty() => {
                stub.push_str(&format!("    todo!(\"implement {sym}({args})\")\n"));
            }
            _ => {}
        }
        stub.push_str("}\n");
        stubs.push(stub);
    }

    if stubs.is_empty() {
        println!("No #native declarations found.");
        return;
    }

    // Generate field offset modules for referenced struct types.
    //
    // Offsets and record size come from the SAME canonical struct schema the
    // interpreter and native codegen consult (`Stores::position` /
    // `Stores::size`) — never a layout re-derived here.  A separate
    // size/offset calculation drifts from the real runtime layout (e.g. a
    // plain `integer` field is an 8-byte i64, not 4 bytes, and loft reorders
    // 8-byte fields ahead of 4-byte record refs for alignment), which
    // silently corrupts every native struct read/write.  @P321c.
    let mut field_modules = String::new();
    for (sname, (d_nr, fields)) in &struct_field_mods {
        let struct_tp = p.data.def(*d_nr).known_type;
        let total_size = p.database.size(struct_tp);

        field_modules.push_str(&format!("/// Field offsets for struct `{sname}`.\n"));
        field_modules.push_str(&format!(
            "/// Record size: {total_size} bytes ({} words).\n",
            total_size.div_ceil(8)
        ));
        field_modules.push_str("#[allow(dead_code)]\n");
        field_modules.push_str(&format!("pub mod {sname}_fields {{\n"));
        for (fname, _, tp) in fields {
            let offset = p.database.position(struct_tp, fname);
            // Skip names that aren't real record fields (e.g. methods that
            // leak into the collected attribute list) — `position` returns
            // u16::MAX for them; emitting a `= 65535` const would be bogus.
            if offset == u16::MAX {
                continue;
            }
            let type_comment = match tp {
                Type::Integer(_) => "integer",
                Type::Float => "float",
                Type::Single => "single",
                Type::Boolean => "boolean",
                Type::Text(_) => "text (record ref)",
                Type::Reference(_, _) => "struct ref",
                Type::Vector(_, _) => "vector ref",
                _ => "other",
            };
            let upper = fname.to_uppercase();
            field_modules.push_str(&format!(
                "    pub const {upper}: u16 = {offset}; // {type_comment}\n"
            ));
        }
        field_modules.push_str("}\n\n");
    }

    let mut output = String::from(
        "// Auto-generated by `loft generate`. Fill in the todo!() bodies.\n\
         // Functions with text parameters use loft_ffi helpers.\n\
         // Struct field offsets are in *_fields modules.\n\n\
         #![allow(clippy::missing_safety_doc)]\n\n",
    );
    if stubs.iter().any(|s| s.contains("loft_ffi")) {
        output.push_str("// Add to Cargo.toml: loft-ffi = { path = \"../../../loft-ffi\" }\n\n");
    }

    // Emit field offset modules first.
    output.push_str(&field_modules);

    for (i, stub) in stubs.iter().enumerate() {
        if i > 0 {
            output.push('\n');
        }
        output.push_str(stub);
    }

    let out_dir = pkg_path.join("native/src");
    if out_dir.exists() {
        let out_file = out_dir.join("generated.rs");
        std::fs::write(&out_file, &output).unwrap_or_else(|e| {
            eprintln!("Error writing {}: {e}", out_file.display());
            std::process::exit(1);
        });
        println!("Wrote {} stubs to {}", stubs.len(), out_file.display());
    } else {
        print!("{output}");
    }
}

/// loft#865 — say that a profiling variable was read and cannot be honoured, when the
/// program is about to run as a compiled binary.
///
/// All three instruments hang off the interpreter's dispatch loop: the CPU sampler
/// walks `State::call_stack`, and the allocation ones key on `alloc_pc`, the bytecode
/// position the loop republishes per op. A native binary has no dispatch loop, so
/// there is nothing to hook and nothing to attribute — this is a real limit, not a
/// missing feature, and `scripts/profile.sh --engine` is the instrument for that side.
///
/// Silent unless one of the variables is actually set, so an ordinary native run is
/// unchanged.
fn announce_profiler_cannot_follow_native() {
    // @PLN154 — the stack shadow has the same shape of limit and the same reason to say so:
    // it lives on the interpreter's value-stack store, and a native binary has no such store
    // to shadow.  Announced here rather than at the exit report, because a native run never
    // reaches that report and would otherwise say nothing at all.
    if loft::stack_verify::enabled() {
        eprintln!(
            "loft: LOFT_VERIFY_STACK set, but the stack shadow is interpreter-only — this \
             program runs native,\n  so no slot will be checked. Add --interpret to verify \
             it."
        );
    }
    let asked: Vec<&str> = ["LOFT_PROFILE", "LOFT_ALLOC_PATHS", "LOFT_ALLOC_SITES"]
        .into_iter()
        .filter(|v| std::env::var_os(v).is_some())
        .collect();
    if asked.is_empty() {
        return;
    }
    eprintln!(
        "loft: {} set, but the loft-level profiler is interpreter-only — this program \
         runs native,\n  so nothing will be sampled. Add --interpret to profile it (it \
         burns the same loft\n  lines), or `make profile PROFILE_FLAGS=--engine` to \
         profile the generated binary with perf.",
        asked.join(" + ")
    );
}

/// loft#861: Handle `loft cache <status|prune>`.
///
/// The auto-native caches are keyed on a value that moves with every installed loft
/// build, so a reinstall orphans the previous generation. Nothing collected them, and
/// the documented remedy was a hand-typed `rm -rf` — which also removes the LIVE
/// generation, so the next build pays a full cold rebuild (545 s of rustc CPU on one
/// measured project gate). `status` says what is there; `prune` takes the part that
/// cannot be used again.
fn handle_cache(argv: &[String], i: &mut usize) {
    let sub = if argv.get(*i).is_some_and(|s| !s.starts_with('-')) {
        *i += 1;
        argv[*i - 1].as_str()
    } else {
        "status"
    };
    let all = argv[*i..].iter().any(|a| a == "--all");
    let force = argv[*i..].iter().any(|a| a == "--force");
    match sub {
        "status" => cache_report(&cache_areas(), false, all),
        "prune" => {
            // The dead-generation test is "the entry's stamp is not THIS loft's key",
            // and a development build has a different key from the installed one (its
            // BUILD_ID is the git HEAD).  So pruning with one reports the installed
            // loft's LIVE generation as unusable and deletes it — correct about the
            // binary that asked, and a full cold rebuild for every project on the
            // machine.  Measured while building this: a dev binary called 49 of 50
            // build trees reclaimable, against 0 for the installed one.
            if loft::cache_gc::running_is_the_installed_loft() != Some(true) && !force {
                eprintln!(
                    "loft cache prune: this is not the installed loft, so it answers for a \
                     different\n  cache key — it would drop the generation the installed loft \
                     is still using,\n  and every project on this machine would rebuild from \
                     cold.\n\n  `loft cache status` is safe from here. Use the installed \
                     binary to prune, or\n  --force if this build IS the one you want the \
                     cache keyed to."
                );
                std::process::exit(1);
            }
            cache_report(&cache_areas(), true, all);
        }
        "warm" => cache_warm(argv, *i),
        _ => {
            eprintln!("usage: loft cache <status|prune|warm> [--all] [--force]");
            std::process::exit(1);
        }
    }
}

/// loft#1238 — build the native artifacts a run is about to need, once, up front.
///
/// Loft's own wasm runtime rlib is keyed on `loft_build_fingerprint` (the rlib's content hash),
/// so rebuilding loft invalidates it — deliberately: that is what makes a codegen fix reach the
/// browser build.  A package's hand-written `native/` cdylib is keyed on the loft-ffi ABI,
/// RUSTFLAGS and the version only (`native_artifact_cache_key`, @PLN159 phase C), so it survives
/// a loft rebuild; the generated `loft_auto_*` cdylib carries the rlib hash in its NAME.  Where
/// an artifact IS stale, the FIRST user pays a full rebuild, and under a parallel test runner
/// every user arrives at once and serialises on the global build lock. Measured on this box: loft's wasm runtime rlib is 25.6 s
/// and the `random` cdylib 1.8 s idle, against a 60 s per-test budget that a loaded CI box blew
/// twice.
///
/// Warming is not a cache-correctness change: each artifact is built by the SAME call a test
/// would make (`ensure_loft_runtime_rlib` / `resolve_native_lib`), so it is stamped with the same
/// key and a test running afterwards takes the ordinary hit path. It only moves the cost out of
/// the parallel section, where it is charged to whichever test happened to arrive first.
///
/// ⚠ SCOPE IS THE WHOLE DESIGN. The first version warmed every stale tree under
/// `~/.loft/build-cache/`, which on this box is 61 of 71 — generations belonging to OTHER loft
/// builds, none of which this run would touch. It was still going minutes later. The set that
/// costs a test its budget is the intersection of two much smaller ones: packages the corpus
/// actually `use`s, and trees already on disk under a previous key. Neither alone is right —
/// used-but-absent is a first build that no warming can avoid, and stale-but-unused is somebody
/// else's cache.
///
/// Never fails the caller. A package that cannot be warmed is simply built by the test that
/// needs it, exactly as before, so this is an optimisation with no new failure mode.
fn cache_warm(argv: &[String], from: usize) {
    let mut wanted: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut idx = from;
    while idx < argv.len() {
        let a = argv[idx].as_str();
        if a == "--from" {
            if let Some(dir) = argv.get(idx + 1) {
                collect_used_packages(std::path::Path::new(dir), &mut wanted);
                idx += 1;
            }
        } else if !a.starts_with('-') {
            wanted.insert(a.to_string());
        }
        idx += 1;
    }

    let mut built = 0usize;
    let mut skipped = 0usize;

    // 1. loft's own wasm runtime rlibs — one per shape, and only where the target is installed.
    //    A missing target is a skip, not a failure: a box without that target has no wasm test
    //    to warm for. `HtmlThreads` is deliberately absent — it needs a nightly `-Z build-std`,
    //    which is far too big a thing to start implicitly before a test run.
    let installed = std::process::Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default();
    for shape in [
        native_utils::WasmRuntimeShape::Wasi,
        native_utils::WasmRuntimeShape::Html,
    ] {
        if !installed.lines().any(|l| l.trim() == shape.triple()) {
            println!(
                "  skip  {} — {} not installed",
                shape.name(),
                shape.triple()
            );
            skipped += 1;
            continue;
        }
        if native_utils::ensure_loft_runtime_rlib(shape).is_some() {
            built += 1;
        } else {
            println!(
                "  warn  {} — could not build its runtime rlib",
                shape.name()
            );
            skipped += 1;
        }
    }

    // 2. Package cdylibs: USED by the corpus, and already on disk under a stale key.
    let home = loft_home();
    let build_cache = home.join("build-cache");
    let registry = home.join("registry");
    let mut entries: Vec<String> = std::fs::read_dir(&build_cache)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().is_dir())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect()
        })
        .unwrap_or_default();
    entries.sort();
    // Keep only the NEWEST version of each wanted package.  A bare `use <pkg>` in a script with
    // no manifest above it resolves to the newest installed, so the older trees on disk are
    // generations nothing in this run can select — warming them is pure waste, and it showed:
    // the first gate run through here rebuilt `graphics` seventeen times, once per version back
    // to 0.1.0, and printed a warning for each.  A package that is actually PINNED by a
    // manifest is the case this heuristic can miss, and missing it costs one first build in the
    // test that needs it — the cost this command exists to move, not to create.
    let mut newest: std::collections::BTreeMap<String, (Vec<u64>, String)> =
        std::collections::BTreeMap::new();
    for name in entries {
        // `<pkg>-<ver>` — the package name is everything before the LAST hyphen.
        let Some((pkg, ver)) = name.rsplit_once('-') else {
            continue;
        };
        if !wanted.contains(pkg) {
            continue;
        }
        // Compare numerically, not lexically: `0.10.0` is newer than `0.9.0`.
        let parts: Vec<u64> = ver
            .split(['.', '-', '+'])
            .map(|c| c.parse::<u64>().unwrap_or(0))
            .collect();
        let e = newest.entry(pkg.to_string());
        match e {
            std::collections::btree_map::Entry::Vacant(v) => {
                v.insert((parts, name.clone()));
            }
            std::collections::btree_map::Entry::Occupied(mut o) => {
                if parts > o.get().0 {
                    o.insert((parts, name.clone()));
                }
            }
        }
    }
    for (_pkg, (_parts, name)) in newest {
        let Some((pkg, _ver)) = name.rsplit_once('-') else {
            continue;
        };
        // ⚠ NO staleness pre-filter here, deliberately.  The obvious optimisation — skip a
        // package whose stamp already matches — needs to know WHICH key the loader compares,
        // and there is more than one: the cdylib check is keyed on the loft-ffi ABI
        // (`loft_ffi_fingerprint`), while loft's own wasm runtime rlib is keyed on
        // `loft_build_fingerprint`, a content hash of the binary.  Asking with the wrong one
        // SILENTLY SKIPS a package that is genuinely stale, which is the one failure this
        // command must not have — measured: a pre-filter on `native_artifact_cache_key`
        // reported a stale `random` as current and warmed nothing.
        //
        // `resolve_native_lib` is the one home for that question and already returns fast on a
        // hit, so asking it unconditionally is both correct and cheap (0.1s for the whole set
        // when everything is warm).  Duplicating its test bought nothing and could only be
        // wrong.
        let pkg_dir = registry.join(&name);
        if !pkg_dir.is_dir() {
            continue; // a build tree whose package is gone; `loft cache prune` owns that
        }
        let stem = format!("loft_{pkg}");
        if loft::extensions::resolve_native_lib(&pkg_dir.to_string_lossy(), &stem).is_some() {
            built += 1;
        } else {
            println!("  warn  {name} — could not rebuild {stem}");
            skipped += 1;
        }
    }
    println!("loft cache warm: {built} artifact(s) current, {skipped} skipped");
}

/// Every `use <name>;` under `dir` — the packages a run there can ask for.
///
/// A plain scan rather than a manifest read on purpose: the corpus is thousands of standalone
/// `.loft` files with no `loft.toml` between them, so `use` is the only statement of what they
/// need. Over-collecting is harmless here (an unused name simply matches no stale build tree);
/// under-collecting would leave the cost where it was.
fn collect_used_packages(dir: &std::path::Path, out: &mut std::collections::BTreeSet<String>) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_used_packages(&p, out);
        } else if p
            .extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("loft"))
            && let Ok(src) = std::fs::read_to_string(&p)
        {
            for line in src.lines() {
                let t = line.trim();
                if let Some(rest) = t.strip_prefix("use ")
                    && let Some(name) = rest.strip_suffix(';')
                {
                    let name = name.trim();
                    if !name.is_empty()
                        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    {
                        out.insert(name.to_string());
                    }
                }
            }
        }
    }
}

/// The cache areas this loft can speak for, surveyed against ITS OWN key — a prune run
/// with one loft build reports and removes what that build cannot select, which is the
/// only question a given binary can answer.
fn cache_areas() -> Vec<loft::cache_gc::Area> {
    let home = loft_home();
    vec![
        loft::cache_gc::survey_build_cache(
            &home.join("build-cache"),
            loft::cache::native_artifact_cache_key(),
        ),
        loft::cache_gc::survey_native_auto(&home.join("registry"), loft::cache_gc::KEEP_ARTIFACTS),
    ]
}

/// Print the footprint, and (when `remove`) take the reclaimable part.
///
/// `status` is the dry run — same figures, nothing touched — so there is no need for a
/// `--dry-run` flag on `prune`, and no way to discover the numbers only by deleting.
fn cache_report(areas: &[loft::cache_gc::Area], remove: bool, all: bool) {
    use loft::cache_gc::human;
    let total: u64 = areas.iter().map(|a| a.bytes).sum();
    let dead: u64 = areas.iter().map(|a| a.dead_bytes).sum();
    if total == 0 {
        println!("loft cache: nothing cached yet.");
        return;
    }
    for a in areas {
        let target = if all { a.bytes } else { a.dead_bytes };
        let target_items = if all { a.items } else { a.dead_items };
        println!("{:<22} {:>10}  {} entries", a.name, human(a.bytes), a.items);
        if target_items > 0 {
            println!(
                "  {:>20} {:>10}  {} reclaimable — {}",
                "",
                human(target),
                target_items,
                if all {
                    "everything (--all)"
                } else {
                    a.basis.label()
                }
            );
        }
    }
    println!("{:<22} {:>10}", "total", human(total));
    if !remove {
        let would = if all { total } else { dead };
        if would == 0 {
            // Nothing to say beyond the footprint. Silence on "nothing to do" is the
            // house rule; a line saying so would be the only output on every clean run.
            return;
        }
        println!(
            "\n{} reclaimable — `loft cache prune{}` to take it.",
            human(would),
            if all { " --all" } else { "" }
        );
        // Whose answer this is. A development build has its own cache key, so what it
        // calls dead includes the installed loft's live generation — the figure is
        // right about the binary that asked and wrong about the machine.
        if loft::cache_gc::running_is_the_installed_loft() != Some(true) {
            println!(
                "  (this is not the installed loft: it answers for its own cache key, so \
                 the figure\n   above counts generations the installed loft is still using.)"
            );
        }
        if !all {
            println!(
                "  (--all also drops the LIVE generation: correct, but the next build \
                 of each package pays a full rustc rebuild.)"
            );
        }
        return;
    }
    let mut freed_bytes = 0;
    let mut freed_items = 0;
    for a in areas {
        let (items, bytes) = if all {
            loft::cache_gc::prune_all(a)
        } else {
            loft::cache_gc::prune(a)
        };
        freed_items += items;
        freed_bytes += bytes;
    }
    // What was actually given back, not what was intended: a busy or vanished entry
    // makes those differ, and reporting the intent is how a tool claims space it did
    // not free.
    if freed_items == 0 {
        println!("\nnothing to prune.");
    } else {
        println!(
            "\nfreed {} across {freed_items} entries.",
            human(freed_bytes)
        );
    }
}

/// REG.3/REG.4: Handle `loft registry <subcommand>`.
fn handle_registry(argv: &[String], i: &mut usize) {
    let sub = if argv.get(*i).is_some_and(|s| !s.starts_with('-')) {
        *i += 1;
        argv[*i - 1].as_str()
    } else {
        ""
    };

    match sub {
        "sync" => registry_sync(),
        "check" => registry_check(),
        "list" => {
            let installed_only = argv.get(*i).is_some_and(|s| s == "--installed");
            registry_list(installed_only);
        }
        _ => {
            eprintln!("usage: loft registry <sync|check|list>");
            std::process::exit(1);
        }
    }
}

/// REG.3: Refresh the local registry so the next `install` / `search` sees what was published.
///
/// loft#1137 — this used to fetch `registry.txt`, the flat-text format `registry.rs` parses,
/// and the live registry has not served that file for a long time: it holds `index.json`.  So
/// the obviously-named refresh 404'd and signed off with *"local registry is unchanged"*,
/// which reads as *nothing to do* rather than *this command cannot work*.  The next pinned
/// install then failed with *"no version satisfies constraint"* against a stale local index —
/// two misleading messages in a row, and neither naming the real state.
///
/// It now refreshes what every other command reads: `install::load_index` with `refresh`, the
/// same signature-verified loader behind `loft install`, `loft search` and
/// `loft api --registry --refresh` (the workaround users had to find).  One home, so the
/// command that is NAMED for the job cannot drift from the one that does it.
///
/// The flat-text path stays reachable for an explicitly configured source — `LOFT_REGISTRY_URL`
/// or a `source:` header in a local `registry.txt` — because that is the only case where such
/// a file exists to fetch.
#[cfg(feature = "registry")]
fn registry_sync() {
    use loft::registry;

    let existing_source = registry::registry_path().and_then(|p| {
        let (_, src) = registry::read_registry(p.to_str().unwrap_or(""));
        src
    });
    let custom = std::env::var("LOFT_REGISTRY_URL").is_ok_and(|u| !u.is_empty())
        || existing_source.as_deref().is_some_and(|u| !u.is_empty());
    if !custom {
        let opts = loft::install::InstallOptions {
            allow_unsigned: true,
            refresh: true,
            offline: false,
            allow_prerelease: false,
            skip_lockfile: false,
            lock_path: None,
        };
        match loft::install::load_index(&opts) {
            Ok(index) => {
                let pkgs = index.packages.len();
                let versions: usize = index.packages.values().map(|p| p.versions.len()).sum();
                println!("registry synced: {pkgs} packages, {versions} versions");
            }
            Err(e) => {
                eprintln!("loft registry sync: {e}\n  local registry is unchanged.");
                std::process::exit(1);
            }
        }
        return;
    }
    registry_sync_flat_file(existing_source.as_deref());
}

#[cfg(not(feature = "registry"))]
fn registry_sync() {
    eprintln!("loft registry sync: registry feature not compiled in.");
    std::process::exit(1);
}

/// The legacy flat-text sync, for an explicitly configured `registry.txt` source.
#[cfg(feature = "registry")]
fn registry_sync_flat_file(existing_source: Option<&str>) {
    use loft::registry;

    let url = registry::source_url(existing_source);

    eprintln!("syncing registry from {url} ...");

    // Download to a temp file first, then validate and move.
    let dst = registry::default_registry_path();
    if let Some(parent) = dst.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = dst.with_extension("tmp");

    #[cfg(feature = "registry")]
    {
        if let Err(e) = registry::download_file(&url, &tmp) {
            eprintln!("loft registry sync: {e}\n  local registry is unchanged.");
            std::process::exit(1);
        }
    }
    #[cfg(not(feature = "registry"))]
    {
        let _ = url;
        let _ = tmp;
        eprintln!("loft registry sync: registry feature not compiled in.");
        std::process::exit(1);
    }

    // Validate content. (Cfg-gated: under `--no-default-features` the block
    // above exits; gating the rest keeps clippy's `unreachable_code` /
    // `dead_code` quiet without a blanket `#[allow]`.)
    #[cfg(feature = "registry")]
    {
        let content = match std::fs::read_to_string(&tmp) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("loft registry sync: cannot read downloaded file: {e}");
                let _ = std::fs::remove_file(&tmp);
                std::process::exit(1);
            }
        };
        if let Err(e) = registry::validate_registry_content(&content) {
            eprintln!(
                "loft registry sync: invalid registry content: {e}\n  local registry is unchanged."
            );
            let _ = std::fs::remove_file(&tmp);
            std::process::exit(1);
        }

        // Move into place.
        if let Err(e) = std::fs::rename(&tmp, &dst) {
            eprintln!("loft registry sync: cannot write {}: {e}", dst.display());
            let _ = std::fs::remove_file(&tmp);
            std::process::exit(1);
        }

        let (entries, _) = registry::parse_registry(&content);
        let (pkgs, versions) = registry::registry_stats(&entries);
        let today = chrono_date();
        println!("registry synced: {pkgs} packages, {versions} versions  ({today})");
    }
}

/// REG.4: Compare installed packages against the registry.
fn registry_check() {
    use loft::registry;

    let Some(reg_path) = registry::registry_path() else {
        eprintln!(
            "loft registry check: no registry file found.\n  \
             Run 'loft registry sync' to download the package registry."
        );
        std::process::exit(1);
    };
    let (entries, _) = registry::read_registry(reg_path.to_str().unwrap_or(""));
    let (pkgs, versions) = registry::registry_stats(&entries);

    // Staleness warning.
    if let Some(warning) = registry::staleness_warning(&reg_path) {
        eprintln!("{warning}");
    }
    let age_str = registry_age_str(&reg_path);
    println!("registry: {pkgs} packages, {versions} versions  ({age_str})");
    println!();

    let lib = registry::lib_dir();
    let installed = registry::installed_packages(&lib);

    if installed.is_empty() {
        println!("no packages installed.");
        println!("\nnew packages in registry: {pkgs}");
        println!("  run 'loft registry list' to browse");
        return;
    }

    println!("installed packages ({}):", installed.len());
    let mut yanked_count = 0;
    for (name, version) in &installed {
        let status = registry::classify(&entries, name, version);
        match status {
            registry::PackageStatus::Yanked { entry } => {
                println!(
                    "  {name:<12} {version:<8} YANKED      {} — run: loft install {name}",
                    entry.status_slug()
                );
                yanked_count += 1;
            }
            registry::PackageStatus::Deprecated { entry, .. } => {
                println!(
                    "  {name:<12} {version:<8} deprecated  {} — run: loft install {name}",
                    entry.status_slug()
                );
            }
            registry::PackageStatus::Outdated { latest } => {
                println!(
                    "  {name:<12} {version:<8} outdated    → {} — run: loft install {name}",
                    latest.version
                );
            }
            registry::PackageStatus::Current => {
                println!("  {name:<12} {version:<8} current");
            }
            registry::PackageStatus::Unknown => {
                println!("  {name:<12} {version:<8} (not in registry)");
            }
        }
    }

    let not_installed = pkgs.saturating_sub(installed.len());
    if not_installed > 0 {
        println!("\nnew packages in registry not installed: {not_installed}");
        println!("  run 'loft registry list' to browse");
    }

    if yanked_count > 0 {
        println!(
            "\n{yanked_count} security issue{} — yanked packages must be updated.",
            if yanked_count == 1 { "" } else { "s" }
        );
        std::process::exit(1);
    } else if installed.iter().all(|(name, version)| {
        matches!(
            registry::classify(&entries, name, version),
            registry::PackageStatus::Current
        )
    }) {
        println!("\nall installed packages are up to date.");
    }
}

/// `loft registry list [--installed]`
fn registry_list(installed_only: bool) {
    use loft::registry;

    let Some(reg_path) = registry::registry_path() else {
        eprintln!(
            "loft registry list: no registry file found.\n  \
             Run 'loft registry sync' to download the package registry."
        );
        std::process::exit(1);
    };
    let (entries, _) = registry::read_registry(reg_path.to_str().unwrap_or(""));
    let lib = registry::lib_dir();
    let installed = registry::installed_packages(&lib);

    let names = registry::package_names(&entries);

    println!(
        "{:<12} {:<28} {:<12} status",
        "name", "versions", "installed"
    );
    println!("{:-<12} {:-<28} {:-<12} {:-<20}", "", "", "", "");

    for name in &names {
        let versions = registry::package_versions(&entries, name);
        let inst_ver = installed
            .iter()
            .find(|(n, _)| n == name)
            .map_or("\u{2014}", |(_, v)| v.as_str());
        if installed_only && inst_ver == "\u{2014}" {
            continue;
        }
        let ver_str: Vec<&str> = versions.iter().map(|e| e.version.as_str()).collect();
        // Determine status column.
        let status = if inst_ver == "\u{2014}" {
            String::new()
        } else if let Some(e) = versions.iter().find(|e| e.version == inst_ver) {
            if e.is_yanked() {
                format!("YANKED ({inst_ver})")
            } else if e.is_deprecated() {
                "deprecated".to_string()
            } else {
                let latest = registry::find_package(&entries, name, None);
                if latest.is_some_and(|l| l.version != inst_ver) {
                    "outdated".to_string()
                } else {
                    String::new()
                }
            }
        } else {
            String::new()
        };
        println!(
            "{:<12} {:<28} {:<12} {}",
            name,
            ver_str.join("  "),
            inst_ver,
            status
        );
    }
}

/// Get a simple date string without pulling in the chrono crate.
#[cfg(feature = "registry")]
fn chrono_date() -> String {
    // Use file modification time of a temp file as a proxy for "now".
    let tmp = std::env::temp_dir().join(".loft_date_probe");
    let _ = std::fs::write(&tmp, "");
    let date = std::fs::metadata(&tmp)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| {
            let dur = t.duration_since(std::time::UNIX_EPOCH).ok()?;
            let secs = dur.as_secs();
            // Simple date calculation from unix timestamp.
            let days = secs / 86400;
            let (year, month, day) = days_to_ymd(days);
            Some(format!("{year}-{month:02}-{day:02}"))
        })
        .unwrap_or_else(|| "unknown date".to_string());
    let _ = std::fs::remove_file(&tmp);
    date
}

/// Convert days since Unix epoch to (year, month, day).
#[cfg(feature = "registry")]
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    // Algorithm from http://howardhinnant.github.io/date_algorithms.html
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Human-readable age of the registry file.
fn registry_age_str(path: &std::path::Path) -> String {
    let age = std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
        .map(|d| d.as_secs() / 86400);
    match age {
        Some(0) => "synced today".to_string(),
        Some(1) => "synced 1 day ago".to_string(),
        Some(d) => format!("synced {d} days ago"),
        None => "sync date unknown".to_string(),
    }
}

// `rlibs_in_dir` + `add_native_extern_flags` moved to `native_utils.rs` so the
// native test runner (`test_runner.rs`, in the library crate) can link a
// package's `#native` crate identically to the standalone + WASM native
// compiles (LibCI native library gate).

/// @PLN12 phase 04 — resolve the stdlib dir and run the interactive REPL, then
/// exit.  Used by `loft repl` and by a bare `loft` with no file/subcommand.
fn start_repl() -> ! {
    // `--fresh` (anywhere on the command line): discard the saved session so
    // this launch starts clean.  Handled here by an env scan so the flag needs
    // no thread-through the arg loop, and works for both `loft repl --fresh` and
    // a bare `loft --fresh`.  @PLN12 REPL.S — text-replay auto-resume.
    if std::env::args().any(|a| a == "--fresh") {
        if let Some(path) = loft::repl::session_file_path() {
            loft::repl::ReplSession::clear_session(&path);
        }
    }
    let base = project_dir();
    let default_dir = std::path::Path::new(&base).join("default");
    let stdlib = if default_dir.exists() {
        default_dir.to_string_lossy().into_owned()
    } else {
        "default".to_string()
    };
    // `--lib <dir>` reaches the REPL too.  It did not before: this was the FOURTH
    // entry point of the @PLN120 E.1 defect, so `loft repl --lib d` answered
    // "Library 'x' not found" for a library every other surface could load.  One
    // `ResolutionContext` now carries both inputs, and `:reset` re-opens with it
    // instead of degrading to stdlib-only.
    let ctx = loft::repl::ResolutionContext {
        stdlib_dir: stdlib.clone(),
        lib_dirs: collect_lib_dirs(&std::env::args().collect::<Vec<_>>()),
    };
    let stdin = std::io::stdin();
    let mut stderr = std::io::stderr();
    let code = match loft::repl::run_repl(&ctx, stdin.lock(), &mut stderr) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("loft repl: {e}");
            1
        }
    };
    std::process::exit(code);
}

/// Collect every `--lib <dir>` import path from `args`, canonicalised (so a relative dir
/// resolves against the launch cwd) and de-duplicated.  Shared by the debugger's `--rpc`
/// and `--serve` servers so the debugged file can `use` libraries — the `use`-resolution
/// the normal run path's `--lib` handling already gives a plain `loft <file>` run.
fn collect_lib_dirs(args: &[String]) -> Vec<String> {
    let mut dirs = Vec::new();
    let mut i = 0;
    while i + 1 < args.len() {
        if args[i] == "--lib" {
            let raw = &args[i + 1];
            let abs = portable_path::plain_canonical_str(raw);
            if !dirs.contains(&abs) {
                dirs.push(abs);
            }
            i += 1;
        }
        i += 1;
    }
    dirs
}

/// Parse a wasm-bridge crate's `Cargo.toml` text, returning each non-`loft`
/// `[dependencies]` entry as `(crate_ident, full TOML line)`.  The `loft`
/// dependency is dropped on purpose (#446): the `--html` driver links the SHARED
/// prebuilt loft via `--extern`, never through this manifest — so the bridge's
/// `loft = { path = "../../../loft" }` (which does not resolve for a
/// registry-installed package) must never reach cargo.  Only inline `[dependencies]`
/// entries are recognised (a `[dependencies.foo]` sub-table is not), matching the
/// shape every shipped bridge manifest uses.
fn bridge_nonloft_deps(cargo_text: &str) -> Vec<(String, String)> {
    let mut deps = Vec::new();
    let mut in_deps = false;
    for line in cargo_text.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix('[') {
            in_deps = rest.starts_with("dependencies]");
        } else if in_deps && !t.is_empty() && !t.starts_with('#') {
            if let Some(k) = t.split('=').next().map(str::trim) {
                if !k.is_empty() && k != "loft" {
                    deps.push((k.replace('-', "_"), t.to_string()));
                }
            }
        }
    }
    deps
}

/// Build a throwaway "deps-only" `Cargo.toml` whose `[dependencies]` are exactly
/// `nonloft_deps` (each full TOML line copied verbatim).  Building this crate
/// produces the bridge's non-`loft` dependency rlibs for wasm32 WITHOUT cargo ever
/// resolving the bridge manifest's `loft` path dep (#446).  The empty `src/lib.rs`
/// uses none of the deps, but cargo compiles every declared dependency regardless,
/// so every dep rlib still lands in the crate's deps dir.
fn synth_bridge_deps_manifest(nonloft_deps: &[(String, String)]) -> String {
    let mut manifest = String::from(
        "[package]\nname = \"loft_html_bridge_deps\"\nversion = \"0.0.0\"\n\
         edition = \"2021\"\n\n[lib]\ncrate-type = [\"rlib\"]\n\n[dependencies]\n",
    );
    for (_, line) in nonloft_deps {
        manifest.push_str(line);
        manifest.push('\n');
    }
    manifest
}

/// @PLN16 M5a — `loft debug <file>:<line>`: load the file, break at `file:line`, run
/// `main()` to the breakpoint, and drop into the interactive `(dbg)` prompt.  Reads its
/// `<file>:<line>` argument by scanning the command line (so it needs no thread-through
/// the arg loop), the same way `start_repl` reads `--fresh`.
fn run_file_debugger() -> ! {
    let args: Vec<String> = std::env::args().collect();
    let base = project_dir();
    let default_dir = std::path::Path::new(&base).join("default");
    let stdlib = if default_dir.exists() {
        default_dir.to_string_lossy().into_owned()
    } else {
        "default".to_string()
    };
    // Explicit `--lib <dir>` import paths so the debugged file can `use` libraries (not
    // just the stdlib) — the same `use`-resolution the normal run path has.
    let lib_dirs = collect_lib_dirs(&args);
    // @PLN16 M5d phase 2 — `loft debug --rpc`: the NDJSON wire-protocol server on stdio.
    // The file is supplied by the `launch` request, so no `<file>:<line>` target is
    // needed; the protocol owns stdout, program output rides `output` events.
    if args.iter().any(|a| a == "--rpc") {
        let stdin = std::io::stdin();
        let mut stdout = std::io::stdout();
        let code = match loft::rpc::run_rpc(&stdlib, &lib_dirs, stdin.lock(), &mut stdout) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("loft debug --rpc: {e}");
                1
            }
        };
        std::process::exit(code);
    }
    // @PLN16 M5e slice 1 — `loft debug <file> --serve [--port <n>]`: the browser IDE
    // foundation (HTTP shell + WebSocket protocol).  The file is the `debug` target;
    // default port 8770 (distinct from the plan-35 viewer's 8765).
    if args.iter().any(|a| a == "--serve") {
        let port = args
            .iter()
            .position(|a| a == "--port")
            .and_then(|p| args.get(p + 1))
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(8770);
        let file = args
            .iter()
            .position(|a| a == "debug")
            .and_then(|p| args.get(p + 1))
            .filter(|a| !a.starts_with('-'));
        let Some(file) = file else {
            eprintln!("usage: loft debug <file> --serve [--port <n>]");
            std::process::exit(2);
        };
        let code = match loft::serve::run_serve(&stdlib, &lib_dirs, port, file) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("loft debug --serve: {e}");
                1
            }
        };
        std::process::exit(code);
    }
    // Find the target after `debug`, skipping flags AND the value of a flag that
    // takes one.  `loft debug --lib dir prog.loft:12` used to take `--lib` as the
    // target and then complain that `:<line>` was missing — a message about a token
    // the user never meant as the target (@PLN120 E2).  Skipping only `-`-prefixed
    // tokens is not enough: it would then take `dir`, which is just as wrong.
    let target = args.iter().position(|a| a == "debug").and_then(|p| {
        let rest = &args[p + 1..];
        let mut i = 0;
        while i < rest.len() {
            if rest[i].starts_with('-') {
                // `--flag=value` carries its value; the spaced forms below do not.
                i += if matches!(rest[i].as_str(), "--lib" | "--path" | "--port") {
                    2
                } else {
                    1
                };
            } else {
                return Some(&rest[i]);
            }
        }
        None
    });
    let Some(target) = target else {
        eprintln!("usage: loft debug <file>:<line>  (or: loft debug --rpc / --serve)");
        std::process::exit(2);
    };
    // `rsplit_once` so a path that itself contains a colon (e.g. a Windows drive) keeps
    // it; the last `:` separates the line number.
    let Some((file, line_s)) = target.rsplit_once(':') else {
        eprintln!("usage: loft debug <file>:<line>  (missing `:<line>`)");
        std::process::exit(2);
    };
    let Ok(line) = line_s.parse::<u32>() else {
        eprintln!("loft debug: not a line number: {line_s:?}");
        std::process::exit(2);
    };
    let stdin = std::io::stdin();
    let mut stderr = std::io::stderr();
    let code =
        match loft::repl::run_file_debug(&stdlib, &lib_dirs, file, line, stdin.lock(), &mut stderr)
        {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("loft debug: {e}");
                1
            }
        };
    std::process::exit(code);
}

/// @PLN102 C1 — load `file` (stdlib + the file), parse, and return its public surface PLUS its
/// @PLN97 layout identity (the two verdict axes), or an error string on a missing file / parse
/// failure. Shared by the descriptor and `--diff` modes.
fn api_surface_of(
    file: &str,
) -> Result<
    (
        Vec<loft::api_surface::Member>,
        loft::schema_sidecar::LayoutIdentity,
    ),
    String,
> {
    let entry = std::path::PathBuf::from(file);
    if !entry.exists() {
        return Err(format!("file {file} not found"));
    }
    let abs = portable_path::plain_canonical(&entry);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    let default_dir = exe_dir.join("../default");
    let default_str = if default_dir.exists() {
        default_dir.to_string_lossy().to_string()
    } else {
        format!("{}/default", project_dir())
    };
    let mut p = parser::Parser::new();
    if let Some(src_dir) = entry.parent() {
        p.lib_dirs.push(src_dir.to_string_lossy().to_string());
    }
    let _ = p.parse_dir(&default_str, true, false);
    let abs_str = abs.to_string_lossy().to_string();
    p.parse(&abs_str, false);
    if p.diagnostics.level() >= loft::diagnostics::Level::Error {
        return Err(format!("{file} did not parse cleanly"));
    }
    let surface = loft::api_surface::surface(&p.data, &abs_str);
    // Layout axis (@PLN97): the per-type store-layout identity, computed from the COMPILED
    // database — kept OUT of the descriptor so a layout-only edit (a field reorder) does not
    // perturb the API-axis determinism, and compared as its own axis in `--diff`.
    let roots = loft::schema_sidecar::program_roots(&p.data);
    // @PLN102 arc-E F9 — `Data` is live here, so pin full-width nullability too
    // (`integer` vs `integer?` share a byte layout; the schema axis distinguishes them).
    let identity = loft::schema_sidecar::LayoutIdentity::of_scoped(&p.database, &roots, &p.data);
    Ok((surface, identity))
}

/// @PLN102 C96 — `loft ship [args…]`: the maintainer ship verb.  Locates
/// `scripts/registry_maintain.sh` (env override, exe-relative, or CWD) and runs it.
/// On a KEY-PRESENT machine (`~/.loft/trust-root/registry-signing-key.bin` exists) it
/// defaults the local file signer + non-interactive `--yes` — autonomous package →
/// sign → push of every own lib newer than the index (registry-sign.sh does the
/// CAS-retry push).  On a KEY-ABSENT machine it can't sign; it says so and runs the
/// routine in review mode (which reports what a key holder must fold in).  Explicit
/// `LOFT_REGISTRY_SIGNER` / passing `--dry-run` / `--yes` are respected as given.
/// Exit code is the script's.
fn run_ship_command(args: &[String]) -> i32 {
    use std::path::PathBuf;
    let script = std::env::var("LOFT_SHIP_SCRIPT")
        .ok()
        .map(PathBuf::from)
        .filter(|p| p.is_file())
        .or_else(|| {
            let mut cands: Vec<PathBuf> = Vec::new();
            if let Ok(exe) = std::env::current_exe() {
                for up in [2usize, 3, 4] {
                    let mut d = exe.clone();
                    for _ in 0..up {
                        d.pop();
                    }
                    cands.push(d.join("scripts/registry_maintain.sh"));
                }
            }
            if let Ok(cwd) = std::env::current_dir() {
                cands.push(cwd.join("scripts/registry_maintain.sh"));
            }
            cands.into_iter().find(|p| p.is_file())
        });
    let Some(script) = script else {
        eprintln!(
            "loft ship: cannot find scripts/registry_maintain.sh — run from the loft repo, or set LOFT_SHIP_SCRIPT."
        );
        return 1;
    };

    let key_present = std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join(".loft/trust-root/registry-signing-key.bin"))
        .is_some_and(|p| p.is_file());

    let mut cmd = std::process::Command::new("bash");
    cmd.arg(&script);
    let passthrough_yes = args.iter().any(|a| a == "--yes" || a == "--dry-run");
    if key_present {
        if std::env::var_os("LOFT_REGISTRY_SIGNER").is_none() {
            cmd.env("LOFT_REGISTRY_SIGNER", "file"); // C96: local file key is the default signer
        }
        if !passthrough_yes {
            cmd.arg("--yes"); // autonomous — no prompt on a key-present machine
        }
    } else {
        eprintln!(
            "loft ship: no trust-root key here (~/.loft/trust-root/registry-signing-key.bin) — this machine cannot sign."
        );
        eprintln!(
            "           Prepare a submission for a key holder to fold in (see REGISTRY_SUBMIT.md); running in review mode below."
        );
        if !passthrough_yes {
            cmd.arg("--dry-run"); // key-absent → don't attempt to sign/push, just report
        }
    }
    for a in args {
        cmd.arg(a);
    }
    match cmd.status() {
        Ok(s) => s.code().unwrap_or(1),
        Err(e) => {
            eprintln!("loft ship: failed to run {}: {e}", script.display());
            1
        }
    }
}

/// @PLN102 C1 — `loft api-surface <file>` prints the surface descriptor; `loft api-surface
/// --diff <base> <new> [--json]` prints the compatibility verdict (human text, or machine JSON
#[cfg(feature = "registry")]
/// `loft compat api [<version>]` — is this working tree still a drop-in for a PUBLISHED
/// release of itself?
///
/// The claim being checked is the package's own `api_compatible_with` / `data_compatible_with`
/// floor: a real version of this package, which is what makes the claim verifiable — the
/// release it names can be fetched and diffed. An abstract epoch could not be.
///
/// Reuses `api-surface --diff` wholesale, which already reports BOTH axes the two floors
/// need: `api_diff::diff` (public surface) and `schema_sidecar::classify` (value-type layout
/// — a silent DATA break the API axis alone green-lights).
///
/// **Advisory in this step**: it reports and always exits 0 unless it could not run. Turning a
/// break into a failure is a later step, after the noise has been measured across every
/// published package — the discipline every check in this repo that had to be walked back did
/// not follow. Design: `doc/claude/plans/library-compat-contract/README.md`.
///
/// Exit: 0 = ran (verdict on stdout) · 2 = could not run (no manifest, no such release,
/// unreadable source).
fn run_compat_command(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();
    // The dispatcher strips `compat`, so positional[0] is the sub-verb.
    let sub = positional.first().map(|s| s.as_str()).unwrap_or("");
    if !matches!(sub, "api" | "test" | "check" | "levels" | "floor") {
        eprintln!(
            "loft compat: usage: loft compat <api|test|check|levels|floor> [<version>] \
             [--json] [--with-tests] [--full]"
        );
        return 2;
    }

    let Some(manifest) = loft::manifest::read_manifest("loft.toml") else {
        eprintln!(
            "loft compat: no `loft.toml` here — run this from a package root, since the \
             comparison is against a published release of THIS package"
        );
        return 2;
    };
    let Some(name) = manifest.name.clone() else {
        eprintln!("loft compat: `loft.toml` has no [package] name");
        return 2;
    };

    // `levels` — the REGISTRY-ADMISSION gate, and the only place the three-level
    // requirement is fatal.  It is deliberately its own verb rather than a side effect of
    // packaging: a library in its current form must keep building, testing and packaging
    // exactly as before, and only asking to enter the registry requires the declaration.
    if sub == "levels" {
        let Some(version) = manifest.version.clone() else {
            eprintln!("loft compat levels: `loft.toml` has no [package] version");
            return 2;
        };
        return match loft::package::declared_levels(&manifest, &version) {
            Ok(l) => {
                println!(
                    "compat levels `{name}` {version}: loft = {}, api_compatible_with = {}, \
                     data_compatible_with = {}",
                    l.loft, l.api, l.data
                );
                0
            }
            Err(problems) => {
                eprintln!(
                    "loft compat levels: {}",
                    loft::package::declared_levels_error(&name, &version, &problems)
                );
                1
            }
        };
    }

    if sub == "floor" {
        return compat_floor(
            &name,
            manifest.version.as_deref(),
            args.iter().any(|a| a == "--with-tests"),
        );
    }

    if sub == "check" {
        return compat_check(
            &name,
            manifest.version.as_deref(),
            manifest.api_compatible_with.as_deref(),
            args.iter().any(|a| a == "--full"),
        );
    }

    // Which release to compare against: an explicit argument, else the declared API floor.
    // No silent default to "latest" — the floor is the claim, and comparing against something
    // the author did not declare would report a verdict about a promise nobody made.
    let target = match positional.get(1) {
        Some(v) => (*v).clone(),
        None => match manifest.api_compatible_with.clone() {
            Some(v) => v,
            None => {
                eprintln!(
                    "loft compat: `{name}` declares no `api_compatible_with`, so there is no \
                     claim to check. Add one naming the oldest release this is still a \
                     drop-in for, or pass a version explicitly."
                );
                return 2;
            }
        },
    };

    // The published source must be on disk to diff against. Reuse the install cache rather
    // than fetching a second way, so this sees exactly what a consumer would get.
    let dir = loft::registry_index::extract_dir(&name, &target);
    if !dir.join("loft.toml").exists() {
        eprintln!(
            "loft compat: `{name}` {target} is not available locally — install it first \
             (`loft install {name}@{target}`).\n\
             If it cannot be installed at all, the claim naming it is unverifiable, which is \
             itself the finding: a floor must name a release that still exists."
        );
        return 2;
    }

    if sub == "test" {
        return compat_test(&name, &target, &dir);
    }

    let old_entry = entry_file_of(&dir, &name);
    let new_entry = entry_file_of(std::path::Path::new("."), &name);
    let ((old_s, old_id), (new_s, new_id)) =
        match (api_surface_of(&old_entry), api_surface_of(&new_entry)) {
            (Ok(o), Ok(n)) => (o, n),
            (Err(e), _) | (_, Err(e)) => {
                eprintln!("loft compat: {e}");
                return 2;
            }
        };
    let api = loft::api_diff::diff(&old_s, &new_s);
    let layout = loft::schema_sidecar::classify(&old_id, &new_id);
    if !json {
        println!("compat: `{name}` working tree vs published {target}");
    }
    print_verdict(&api, &layout, json);
    // Advisory: report the two floors beside the two verdicts, so the author sees which claim
    // each axis is measured against rather than having to remember.
    if !json {
        println!(
            "  api_compatible_with  = {}",
            manifest.api_compatible_with.as_deref().unwrap_or("<unset>")
        );
        println!(
            "  data_compatible_with = {}",
            manifest
                .data_compatible_with
                .as_deref()
                .unwrap_or("<unset>")
        );
    }
    0
}

#[cfg(feature = "registry")]
/// `loft compat check` — the CI entry point: sample a few published releases and report.
///
/// Cost is **O(1) per run** by construction, which is the only shape that survives a mature
/// registry: a library with 50 releases and slow suites must cost the same per PR as one with
/// two. The sample is the latest release, the declared floor, and **one random release in
/// between** — the random pick is what makes it mean, because a break cannot hide in a version
/// nobody ever looks at. Coverage accumulates across runs rather than within one.
///
/// The random pick is PRINTED and can be pinned with `LOFT_COMPAT_SAMPLE` (failure path F5).
/// Without that a real break reads as a flake: the job goes red, someone re-runs it, a
/// different release is drawn, it goes green, and a genuine contract break is dismissed as CI
/// noise — strictly worse than having no check.
///
/// **Blocking only when a floor is declared.** Declaring `api_compatible_with` IS the act of
/// entering the contract: a library that declares nothing has made no promise, so there is
/// nothing to enforce and this stays advisory for it. That is not timidity — it is the model,
/// where a library may break its consumers as long as the break is an explicit choice. It also
/// makes the flip safe by construction: no published package declares a floor today, so
/// turning this into a gate cannot fail anyone's CI until they opt in.
///
/// Exit: 1 when a DECLARED floor is violated · 0 otherwise.
/// `loft compat floor` — MEASURE how far back this package's current API still reaches, and
/// print the declaration to paste.
///
/// The migration tool. Every library starts the contract with the same problem: it has to name
/// a floor, and the honest answer is a fact about its own history that nobody remembers. Left
/// to guess, an author writes the version they are cutting — true, but claiming nothing, and a
/// registry full of self-referential floors carries no information at all.
///
/// So this derives it. Scanning from the NEWEST installed release downward and stopping at the
/// first incompatible one is the only correct direction: a floor `F` claims drop-in for
/// *everything* at or above `F`, so a single failure above a candidate disqualifies it — even
/// if older releases happen to pass. That is why this is not a bisect; compatibility is not
/// guaranteed monotone, and a bisect would happily return a floor with a break sitting above it.
///
/// `--with-tests` adds the behaviour axis (each candidate's own published suite, run against
/// the working tree). Worth the time for the migration itself: an API diff proves the SHAPE of
/// a surface, and the `arguments::parse` cell in step 3 is a release that kept its signature
/// and inverted its result — `API: drop-in` on this axis, a break on that one.
#[cfg(feature = "registry")]
fn compat_floor(name: &str, own_version: Option<&str>, with_tests: bool) -> i32 {
    let mut versions: Vec<String> = loft::registry_index::installed_packages()
        .into_iter()
        .filter(|(n, v, _)| n == name && Some(v.as_str()) != own_version)
        .map(|(_, v, _)| v)
        .collect();
    versions.sort_by(|a, b| loft::registry_index::compare_semver(a, b));

    let own = own_version.unwrap_or("0.0.0");
    if versions.is_empty() {
        println!(
            "compat floor `{name}`: no earlier release installed, so this release is the only \
             thing it can claim to be a drop-in for.\n\n  api_compatible_with  = \"{own}\"\n  \
             data_compatible_with = \"{own}\"\n\n  That is the correct FIRST declaration, not a \
             placeholder: it is true, and it becomes meaningful the moment a later release \
             keeps it."
        );
        return 0;
    }

    // Read the WORKING TREE's surface once, before the walk.  Inside the loop a failure here
    // is indistinguishable from the old release being unreadable, and the loop would blame
    // the release — reporting a fact about someone else's published version when the problem
    // is the source in front of you.  It also fails identically at every step, so the floor
    // would read as "reaches back to nothing" for a package that was never compared at all.
    let new_entry = entry_file_of(std::path::Path::new("."), name);
    let new_surface = match api_surface_of(&new_entry) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "loft compat floor: cannot read THIS package's own surface ({}): {e}\n  \
                 Nothing was measured — fix the working tree first.  No floor is being \
                 claimed, and none should be until this parses.",
                new_entry
            );
            return 2;
        }
    };
    let (new_s, new_id) = new_surface;

    println!(
        "compat floor `{name}` {own}: walking {} installed release(s), newest first",
        versions.len()
    );
    let mut floor: Option<String> = None;
    let mut stopped_at: Option<(String, String)> = None;
    // Versions the behaviour axis could not judge (stale corpus / suite would not run).
    let mut no_behaviour: Vec<String> = Vec::new();

    for v in versions.iter().rev() {
        let dir = loft::registry_index::extract_dir(name, v);
        if !dir.join("loft.toml").exists() {
            // A gap is not a pass.  Treat it as the end of the reachable window rather than
            // stepping over it, or the floor would claim a version nobody looked at.
            stopped_at = Some((v.clone(), "not installed — cannot be verified".to_string()));
            break;
        }
        let old_entry = entry_file_of(&dir, name);
        let Ok((old_s, old_id)) = api_surface_of(&old_entry) else {
            // Only the PUBLISHED side can fail here — the working tree was read once above.
            stopped_at = Some((
                v.clone(),
                "its published source no longer parses — cannot be verified".to_string(),
            ));
            break;
        };
        let api = loft::api_diff::diff(&old_s, &new_s);
        let layout = loft::schema_sidecar::classify(&old_id, &new_id);
        if let loft::api_diff::Verdict::Break(why) = &api {
            // Every reason, not the first: the point of the migration run is to show the
            // author exactly what stops the claim from reaching further back.
            stopped_at = Some((v.clone(), format!("API break — {}", why.join("; "))));
            break;
        }
        if layout_reshaped(&layout) {
            stopped_at = Some((v.clone(), "DATA break — stored layout reshaped".to_string()));
            break;
        }
        if with_tests && dir.join("tests").is_dir() {
            // ONLY a Break lowers the floor.  A Break is evidence about the LIBRARY: the
            // release's tests pass against its own source and fail against this tree, so
            // released behaviour changed.  Unverifiable and CouldNotRun are evidence about
            // the ENVIRONMENT — a corpus written against an older loft, a suite that would
            // not start — and say nothing about compatibility.
            //
            // Letting those lower the floor is failure path F4, and it is not hypothetical:
            // the first full sweep produced 0 Breaks, 17 drop-ins and 5 Unverifiables, and
            // every floor the axis moved was moved by a stale corpus.  Treating them as
            // failures makes each loft language change quietly shorten every library's
            // history, which is precisely how a check earns its way into being switched off.
            // The API and layout axes verified these versions; the behaviour axis merely has
            // nothing to add, and that is recorded rather than punished.
            match compat_test_verdict(name, v, &dir) {
                TestVerdict::Break => {
                    stopped_at = Some((
                        v.clone(),
                        "its published tests FAIL against this tree — released behaviour changed"
                            .to_string(),
                    ));
                    break;
                }
                // Recorded, not punished: the report must say which versions the behaviour
                // axis could not speak for, so nobody reads the floor as fully verified.
                TestVerdict::Unverifiable | TestVerdict::CouldNotRun => {
                    no_behaviour.push(v.clone())
                }
                TestVerdict::DropIn => {}
            }
        }
        println!("  {v}: drop-in");
        floor = Some(v.clone());
    }

    if let Some((v, why)) = &stopped_at {
        println!("  {v}: STOP — {why}");
    }
    if !no_behaviour.is_empty() {
        println!(
            "  note: the behaviour axis could not judge {} — their suites no longer pass \
             against their OWN source on this loft, which is a fact about the corpus, not \
             about compatibility. The API and layout axes still verified them.",
            no_behaviour.join(", ")
        );
    }
    let axes = if with_tests {
        "API surface, stored layout, and each release's own tests"
    } else {
        "API surface and stored layout (pass --with-tests to add the behaviour axis)"
    };
    match floor {
        Some(f) => println!(
            "\ncompat floor `{name}`: reaches back to {f}, verified on {axes}.\n\n  \
             api_compatible_with  = \"{f}\"\n  data_compatible_with = \"{f}\"\n\n  \
             Check `data_compatible_with` by hand before pasting: it is about STORED data, and \
             a release can keep every signature while changing what it computes over a file \
             somebody already has."
        ),
        None => println!(
            "\ncompat floor `{name}`: reaches back to nothing — the newest earlier release \
             already differs, so this release can only claim itself.\n\n  \
             api_compatible_with  = \"{own}\"\n  data_compatible_with = \"{own}\"\n\n  \
             That is a DECLARED break, which is allowed: the registry keeps the older releases \
             installable, so a consumer that cannot follow stays where it is."
        ),
    }
    0
}

/// Step 7 — verify a package's ENTIRE claim: every installed release, under a wall-clock
/// budget, with overrun reported as failure rather than smuggled in as success.
///
/// The gate a release passes through, where the O(1) per-PR sample is not enough. A sampled
/// check answers "did this change break something"; a release has to answer "is everything
/// this package promises actually true", and those are different questions.
///
/// **Overrun fails.** The tempting alternative — verify what fits, report green — produces a
/// release that claims a floor it did not check, which is worse than no check at all because
/// it carries the authority of one. Cost is proportional to the CLAIM, so the remedy is in the
/// author's hands and is named in the message: narrow the floor, or make the suite faster.
#[cfg(feature = "registry")]
fn compat_check_full(name: &str, versions: &[String], floor: Option<&str>) -> i32 {
    let budget = std::env::var("LOFT_COMPAT_BUDGET")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(RELEASE_WINDOW_BUDGET_SECS);
    let started = std::time::Instant::now();
    println!(
        "compat check `{name}` --full: verifying all {} installed release(s), budget {budget}s",
        versions.len()
    );

    let new_entry = entry_file_of(std::path::Path::new("."), name);
    let Ok((new_s, new_id)) = api_surface_of(&new_entry) else {
        eprintln!(
            "compat check `{name}` --full: cannot read this package's own surface ({}) — \
             nothing was verified",
            new_entry
        );
        return 2;
    };

    let mut violated: Vec<String> = Vec::new();
    let mut declared: Vec<String> = Vec::new();
    let mut unreadable: Vec<String> = Vec::new();
    let mut checked = 0usize;
    // Oldest first, so a timeout leaves the DEEPEST part of the claim — the part a floor is
    // actually asserting, and the part nobody else looks at — already proven.
    for v in versions {
        if started.elapsed().as_secs() >= budget {
            let remaining = versions.len() - checked;
            eprintln!(
                "\ncompat check `{name}` --full: BUDGET EXCEEDED after {checked} of {} \
                 release(s) — {remaining} never checked.\n  \
                 This release claims more than it proved, so the claim is NOT verified and this \
                 is a failure rather than a partial pass.\n  \
                 Narrow the claim (raise `api_compatible_with`, which shrinks the window) or \
                 split the suite. Raise the ceiling with LOFT_COMPAT_BUDGET=<seconds> only when \
                 the window is genuinely that large.",
                versions.len()
            );
            return 1;
        }
        let dir = loft::registry_index::extract_dir(name, v);
        if !dir.join("loft.toml").exists() {
            unreadable.push(v.clone());
            continue;
        }
        let old_entry = entry_file_of(&dir, name);
        let Ok((old_s, old_id)) = api_surface_of(&old_entry) else {
            unreadable.push(v.clone());
            continue;
        };
        checked += 1;
        let api = loft::api_diff::diff(&old_s, &new_s);
        let layout = loft::schema_sidecar::classify(&old_id, &new_id);
        let broke = matches!(api, loft::api_diff::Verdict::Break(_)) || layout_reshaped(&layout);
        // Below the declared floor a break is ANNOUNCED, not failed — the promise never
        // covered it.  Above the floor it is the unclaimed break F1 exists for.
        let below = floor.is_some_and(|f| {
            matches!(
                loft::registry_index::compare_semver(v, f),
                std::cmp::Ordering::Less
            )
        });
        let verdict = match (broke, below) {
            (false, _) => "drop-in",
            (true, true) => {
                declared.push(v.clone());
                "DECLARED BREAK (below the floor)"
            }
            (true, false) => {
                violated.push(v.clone());
                "BREAK"
            }
        };
        println!("  {v}: {verdict}");
    }

    let elapsed = started.elapsed().as_secs();
    // Named, never silently dropped: a release that could not read part of its own history has
    // not verified that part, and the report has to say so even when nothing failed.
    if !unreadable.is_empty() {
        println!(
            "  note: {} release(s) could not be read and were NOT verified: {}",
            unreadable.len(),
            unreadable.join(", ")
        );
    }
    if !declared.is_empty() {
        println!(
            "  {} release(s) below the declared floor differ, as declared: {}",
            declared.len(),
            declared.join(", ")
        );
    }
    if violated.is_empty() {
        println!(
            "compat check `{name}` --full: whole claim verified in {elapsed}s ({checked} release(s))"
        );
        return 0;
    }
    eprintln!(
        "\ncompat check `{name}` --full: {} release(s) at or above the declared floor are NOT \
         drop-in: {}\n  Either restore compatibility, or raise `api_compatible_with` past them \
         to declare the break.",
        violated.len(),
        violated.join(", ")
    );
    1
}

/// Step 7 — how long the RELEASE gate may spend proving a package's whole claim.
///
/// A budget is what keeps "verify everything" from decaying into "verify a prefix and call it
/// everything". Overrun is a FAILURE, never a truncation: a release that ran out of time has
/// proved less than it claims, and reporting that as proven is the exact dishonesty the floors
/// exist to prevent.
#[cfg(feature = "registry")]
const RELEASE_WINDOW_BUDGET_SECS: u64 = 600;

#[cfg(feature = "registry")]
fn compat_check(name: &str, own_version: Option<&str>, floor: Option<&str>, full: bool) -> i32 {
    // Candidates come from the install cache, because that is what `compat api` / `compat
    // test` can actually read. Anything not installed is REPORTED as skipped, never silently
    // dropped — a check that quietly narrows its own scope reports "clean" for work it did
    // not do.
    let mut versions: Vec<String> = loft::registry_index::installed_packages()
        .into_iter()
        .filter(|(n, v, _)| n == name && Some(v.as_str()) != own_version)
        .map(|(_, v, _)| v)
        .collect();
    versions.sort_by(|a, b| loft::registry_index::compare_semver(a, b));
    // Releases BELOW the declared floor stay in the comparison, reported but never gating.
    // Dropping them would make raising the floor buy SILENCE, which is the wrong gradient
    // entirely: the reflex to bump the number until the check shuts up is what turns floors
    // into decoration. Keeping the promise must be the quiet path and declaring a break the
    // loud one, so a raise converts a failure into an announcement rather than into nothing.
    let below_floor: Vec<String> = match floor {
        Some(f) => versions
            .iter()
            .filter(|v| {
                matches!(
                    loft::registry_index::compare_semver(v, f),
                    std::cmp::Ordering::Less
                )
            })
            .cloned()
            .collect(),
        None => Vec::new(),
    };
    if versions.is_empty() {
        println!("compat check `{name}`: no other release installed — nothing to compare against");
        return 0;
    }

    // The RELEASE gate proves the whole claim; every other caller pays O(1).  Two different
    // questions: a PR asks "did this change break something", a release asks "is everything
    // this package promises actually true".  Sampling answers the first honestly and the
    // second not at all.
    if full {
        return compat_check_full(name, &versions, floor);
    }

    let latest = versions.last().cloned().expect("non-empty");
    let mut sample: Vec<String> = vec![latest.clone()];
    if let Some(f) = floor
        && versions.iter().any(|v| v == f)
        && !sample.contains(&f.to_string())
    {
        sample.push(f.to_string());
    }
    // One random interior pick: anything not already sampled.
    let interior: Vec<&String> = versions.iter().filter(|v| !sample.contains(v)).collect();
    if !interior.is_empty() {
        let pinned = std::env::var("LOFT_COMPAT_SAMPLE").ok();
        let chosen = match pinned.as_deref() {
            Some(p) if interior.iter().any(|v| v.as_str() == p) => p.to_string(),
            _ => {
                // Seeded from the clock; the point is not cryptographic quality but that the
                // draw VARIES across runs and is reported, so coverage accumulates and any
                // red can be reproduced with LOFT_COMPAT_SAMPLE.
                let n = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as usize);
                interior[n % interior.len()].clone()
            }
        };
        println!(
            "compat check `{name}`: sampled interior release {chosen} (pin with LOFT_COMPAT_SAMPLE={chosen})"
        );
        sample.push(chosen);
    }

    println!(
        "compat check `{name}`: {} of {} installed release(s) sampled{}",
        sample.len(),
        versions.len(),
        if versions.len() > sample.len() {
            format!(
                " — {} not looked at this run",
                versions.len() - sample.len()
            )
        } else {
            String::new()
        }
    );
    let mut violated: Vec<String> = Vec::new();
    for v in &sample {
        let dir = loft::registry_index::extract_dir(name, v);
        if !dir.join("loft.toml").exists() {
            println!("  {v}: SKIPPED (not installed)");
            continue;
        }
        let old_entry = entry_file_of(&dir, name);
        let new_entry = entry_file_of(std::path::Path::new("."), name);
        match (api_surface_of(&old_entry), api_surface_of(&new_entry)) {
            (Ok((old_s, old_id)), Ok((new_s, new_id))) => {
                let api = loft::api_diff::diff(&old_s, &new_s);
                let layout = loft::schema_sidecar::classify(&old_id, &new_id);
                let api_word = match api {
                    loft::api_diff::Verdict::Break(_) => "API BREAK",
                    loft::api_diff::Verdict::Superset => "api ok",
                };
                let layout_word = if layout_reshaped(&layout) {
                    "DATA BREAK"
                } else {
                    "data ok"
                };
                println!("  {v}: {api_word}, {layout_word}");
                if matches!(api, loft::api_diff::Verdict::Break(_)) || layout_reshaped(&layout) {
                    violated.push(v.clone());
                }
            }
            (Err(e), _) | (_, Err(e)) => println!("  {v}: could not read ({e})"),
        }
    }

    let Some(f) = floor else {
        if violated.is_empty() {
            println!(
                "  `{name}` declares no `api_compatible_with`. Nothing is enforced: add one \
                 naming the oldest release this is still a drop-in for, and this becomes a \
                 promise consumers can rely on."
            );
        } else {
            println!(
                "  advisory only: `{name}` declares no `api_compatible_with`, so no promise \
                 was made to break. Declare one to have this enforced."
            );
        }
        return 0;
    };
    // A break against a release the floor already excludes is the DECLARED one. Say so out
    // loud — it is the thing a reviewer most needs to see, and saying it is what keeps the
    // raise honest rather than a way to go quiet.
    let declared: Vec<&String> = violated
        .iter()
        .filter(|v| below_floor.contains(v))
        .collect();
    if !declared.is_empty() {
        println!(
            "  DECLARED BREAK — `{name}` no longer works with {} (floor raised to {f}). \
             Consumers on those releases keep resolving to the last version that suits them; \
             this is the supported move, but it is a promise withdrawn, not a free one.",
            declared
                .iter()
                .map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }
    let violated: Vec<String> = violated
        .into_iter()
        .filter(|v| !below_floor.contains(v))
        .collect();
    if violated.is_empty() {
        return 0;
    }
    println!(
        "  FAIL — `{name}` claims to be a drop-in for >= {f}, but breaks against {}. \
         Either restore compatibility, or raise `api_compatible_with` past the release you \
         broke — declaring the break is the supported move, hiding it is not.",
        violated.join(", ")
    );
    1
}

#[cfg(feature = "registry")]
/// `loft compat test <version>` — run a PUBLISHED release's tests against the working tree.
///
/// This is the half `loft compat api` cannot see. An API diff proves the SHAPE of the surface
/// is unchanged; it says nothing about whether the functions still DO the same thing. The
/// published version's own tests are the only description of that behaviour written before
/// this change existed — and unlike the working tree's tests they cannot have been edited to
/// match the new behaviour in the same commit, which is exactly why library CI running the
/// CURRENT tests can never catch a self-inflicted break.
///
/// **The control comes first, and it is not optional.** Those tests were written against the
/// loft of their day, so a failure has two possible causes: this change broke them, or they no
/// longer run against today's loft at all. Running them first against their OWN source
/// separates the two. Without it the first language change turns every library's history red,
/// everyone learns the check lies, and it gets switched off — failure path F4 in the design.
///
/// Advisory in this step: reports, exits 0 unless it could not run.
/// What a published release's own test suite says about the working tree.
///
/// Separate from the CLI's exit code because the two answer different questions. `loft compat
/// test` is advisory and exits 0 whatever it finds; a CALLER deciding a compatibility floor has
/// to tell the three verdicts apart, and collapsing them into one exit code is how the
/// `--with-tests` axis silently checked nothing.
#[cfg(feature = "registry")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TestVerdict {
    /// The release's tests still pass against the working tree.
    DropIn,
    /// They pass against their own source and FAIL here — released behaviour changed.
    Break,
    /// They no longer pass against their OWN source on this loft, so they judge nothing.
    /// A floor must treat this as unverified, never as a pass.
    Unverifiable,
    /// The comparison could not be set up (no `tests/`, staging failed).
    CouldNotRun,
}

#[cfg(feature = "registry")]
fn compat_test(name: &str, version: &str, published: &std::path::Path) -> i32 {
    // Advisory by contract: report the verdict, always exit 0 unless it could not run.
    match compat_test_verdict(name, version, published) {
        TestVerdict::CouldNotRun => 2,
        _ => 0,
    }
}

#[cfg(feature = "registry")]
fn compat_test_verdict(name: &str, version: &str, published: &std::path::Path) -> TestVerdict {
    if !published.join("tests").is_dir() {
        eprintln!("loft compat: `{name}` {version} ships no tests/ — nothing to check against");
        return TestVerdict::CouldNotRun;
    }
    let base = std::env::temp_dir().join(format!("loft_compat_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);

    // CONTROL: the published tests against the published source. Establishes that this corpus
    // can pass at all on today's loft, so a subject failure means something.
    // The staged directory MUST be named after the package: a test does `use <name>;`, which
    // resolves by DIRECTORY NAME, so a differently-named dir silently falls back to the
    // installed copy in ~/.loft/registry — and the check then compares that release against
    // itself and reports drop-in no matter what the working tree says. Caught by the matrix:
    // a deliberate behaviour break read as drop-in until the directories were renamed.
    let ctl = base.join("control").join(name);
    if let Err(e) = stage_package(published, published, &ctl) {
        eprintln!("loft compat: {e}");
        return TestVerdict::CouldNotRun;
    }
    let control_ok = run_package_tests(&ctl);

    // SUBJECT: the same tests against the working tree's source.
    let subj = base.join("subject").join(name);
    if let Err(e) = stage_package(std::path::Path::new("."), published, &subj) {
        eprintln!("loft compat: {e}");
        return TestVerdict::CouldNotRun;
    }
    let subject_ok = run_package_tests(&subj);
    let _ = std::fs::remove_dir_all(&base);

    println!("compat: `{name}` — {version} tests against the working tree");
    match (control_ok, subject_ok) {
        (false, _) => {
            println!(
                "  UNVERIFIABLE — the {version} tests do not pass against {version} own source on \
             this loft, so they cannot judge anything. The corpus is stale (a language change \
             since it was written), not the working tree broken."
            );
            TestVerdict::Unverifiable
        }
        (true, false) => {
            println!(
                "  BREAK — the {version} tests pass against {version} but FAIL against the working \
             tree. Behaviour a released version promised has changed. Either fix it, or raise \
             `api_compatible_with` past {version} to declare the break."
            );
            TestVerdict::Break
        }
        (true, true) => {
            println!("  drop-in — the {version} tests still pass against the working tree.");
            TestVerdict::DropIn
        }
    }
}

/// Assemble a runnable package in `dst`: sources from `src_from`, tests from `tests_from`.
///
/// The mix is the point — the published tests have to run against the working tree's source,
/// and a test does `use <name>;`, so the two must sit in one package for the name to resolve.
/// Staged in a temp directory because neither input may be written to: the install cache is
/// shared, and the working tree is the user's.
#[cfg(feature = "registry")]
fn stage_package(
    src_from: &std::path::Path,
    tests_from: &std::path::Path,
    dst: &std::path::Path,
) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("cannot create {}: {e}", dst.display()))?;
    for item in ["loft.toml", "src", "native"] {
        let from = src_from.join(item);
        if from.exists() {
            copy_tree(&from, &dst.join(item))?;
        }
    }
    copy_tree(&tests_from.join("tests"), &dst.join("tests"))
}

/// Recursive copy. Skips per-checkout build state, which would otherwise carry a stale build
/// into the staged package and have it test something other than the source beside it.
#[cfg(feature = "registry")]
fn copy_tree(from: &std::path::Path, to: &std::path::Path) -> Result<(), String> {
    let meta = std::fs::metadata(from).map_err(|e| format!("{}: {e}", from.display()))?;
    if meta.is_file() {
        if let Some(parent) = to.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(from, to).map_err(|e| format!("{}: {e}", from.display()))?;
        return Ok(());
    }
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| format!("read_dir: {e}"))?;
        let nm = entry.file_name();
        if matches!(nm.to_str(), Some(".loft" | "native-auto" | "target")) {
            continue;
        }
        copy_tree(&entry.path(), &to.join(&nm))?;
    }
    Ok(())
}

/// Run a staged package's suite, returning whether it passed. Bounded by `LOFT_TIMEOUT` (the
/// same bound library CI uses) so one hung old test cannot stall the check.
#[cfg(feature = "registry")]
fn run_package_tests(dir: &std::path::Path) -> bool {
    let exe = std::env::current_exe().unwrap_or_else(|_| std::path::PathBuf::from("loft"));
    std::process::Command::new(exe)
        .arg("--interpret")
        .arg("--tests")
        .arg("tests")
        .current_dir(dir)
        .env(
            "LOFT_TIMEOUT",
            std::env::var("LOFT_TIMEOUT").unwrap_or_else(|_| "120".into()),
        )
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A package's entry `.loft` file: `[library] entry` when declared, else the
/// `src/<name>.loft` default that `loft.toml`'s documentation specifies.
#[cfg(feature = "registry")]
fn entry_file_of(root: &std::path::Path, name: &str) -> String {
    let entry =
        loft::manifest::read_manifest(root.join("loft.toml").to_str().unwrap_or("loft.toml"))
            .and_then(|m| m.entry)
            .unwrap_or_else(|| format!("src/{name}.loft"));
    root.join(entry).to_string_lossy().into_owned()
}

/// for a CI check). Exit: 0 = drop-in / printed · 1 = a BREAK (so a non-required CI check goes
/// red) · 2 = a usage / load error.
fn run_api_surface_command(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with("--")).collect();

    // Commit 7 — the PR check: emit a checked-in baseline, and check current-vs-baseline.
    if args.iter().any(|a| a == "--emit-baseline") {
        let Some(file) = positional.first() else {
            eprintln!("loft api-surface: usage: loft api-surface <file> --emit-baseline");
            return 2;
        };
        return match api_surface_of(file) {
            Ok((surface, identity)) => {
                print!("{}", emit_baseline(&surface, &identity));
                0
            }
            Err(e) => {
                eprintln!("loft api-surface: {e}");
                2
            }
        };
    }
    if args.iter().any(|a| a == "--check") {
        let (Some(baseline_path), Some(file)) = (positional.first(), positional.get(1)) else {
            eprintln!(
                "loft api-surface: usage: loft api-surface --check <baseline> <file> [--json]"
            );
            return 2;
        };
        let Ok(text) = std::fs::read_to_string(baseline_path.as_str()) else {
            eprintln!("loft api-surface: cannot read baseline {baseline_path}");
            return 2;
        };
        let Some((old_s, old_id)) = parse_baseline(&text) else {
            eprintln!("loft api-surface: {baseline_path} is not a valid api-surface baseline");
            return 2;
        };
        let (new_s, new_id) = match api_surface_of(file) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("loft api-surface: {e}");
                return 2;
            }
        };
        let api = loft::api_diff::diff(&old_s, &new_s);
        let layout = loft::schema_sidecar::classify(&old_id, &new_id);
        print_verdict(&api, &layout, json);
        let broke = matches!(api, loft::api_diff::Verdict::Break(_)) || layout_reshaped(&layout);
        return i32::from(broke);
    }

    if args.iter().any(|a| a == "--diff") {
        let (Some(base), Some(new)) = (positional.first(), positional.get(1)) else {
            eprintln!("loft api-surface: usage: loft api-surface --diff <base> <new> [--json]");
            return 2;
        };
        let ((old_s, old_id), (new_s, new_id)) = match (api_surface_of(base), api_surface_of(new)) {
            (Ok(o), Ok(n)) => (o, n),
            (Err(e), _) | (_, Err(e)) => {
                eprintln!("loft api-surface: {e}");
                return 2;
            }
        };
        let api = loft::api_diff::diff(&old_s, &new_s);
        let layout = loft::schema_sidecar::classify(&old_id, &new_id);
        print_verdict(&api, &layout, json);
        // Red on EITHER axis: a public API break, or a value-type reshape (a silent DATA break
        // for a persisting consumer that the API axis alone green-lights).
        let broke = matches!(api, loft::api_diff::Verdict::Break(_)) || layout_reshaped(&layout);
        return i32::from(broke);
    }

    let Some(file) = positional.first() else {
        eprintln!("loft api-surface: usage: loft api-surface <file>  |  --diff <base> <new>");
        return 2;
    };
    match api_surface_of(file) {
        Ok((surface, _identity)) => {
            for m in surface {
                println!("{}", m.to_line());
            }
            0
        }
        Err(e) => {
            eprintln!("loft api-surface: {e}");
            2
        }
    }
}

/// True iff the layout diff RESHAPED a value type — a silent DATA break for a persisting
/// consumer, distinct from a pure add/drop (which the API axis handles).
fn layout_reshaped(handoff: &loft::schema_sidecar::Handoff) -> bool {
    matches!(handoff, loft::schema_sidecar::Handoff::Changed(d) if !d.changed.is_empty())
}

/// Print the TWO-axis compat verdict as machine JSON (a CI check parses it) or human text (a PR
/// comment). JSON: `{"api":{"verdict":…,"broken":[…]},"layout":{"verdict":"stable|changed",
/// "types":[…]}}`. The axes are distinct consumer concerns — a method-only consumer reads
/// `api`; one that persists a value struct reads `layout`.
fn print_verdict(
    api: &loft::api_diff::Verdict,
    layout: &loft::schema_sidecar::Handoff,
    json: bool,
) {
    use loft::api_diff::Verdict;
    use loft::schema_sidecar::Handoff;
    let reshaped: Vec<&str> = match layout {
        Handoff::Changed(d) => d.changed.iter().map(String::as_str).collect(),
        Handoff::Identical => Vec::new(),
    };
    if json {
        let api_json = match api {
            Verdict::Superset => r#"{"verdict":"superset","broken":[]}"#.to_string(),
            Verdict::Break(broken) => {
                let items: Vec<String> = broken.iter().map(|s| api_json_string(s)).collect();
                format!(r#"{{"verdict":"break","broken":[{}]}}"#, items.join(","))
            }
        };
        let layout_json = if reshaped.is_empty() {
            r#"{"verdict":"stable","types":[]}"#.to_string()
        } else {
            let items: Vec<String> = reshaped.iter().copied().map(api_json_string).collect();
            format!(r#"{{"verdict":"changed","types":[{}]}}"#, items.join(","))
        };
        println!(r#"{{"api":{api_json},"layout":{layout_json}}}"#);
    } else {
        match api {
            Verdict::Superset => {
                println!("API: drop-in — the new surface is a superset (additions only).");
            }
            Verdict::Break(broken) => {
                println!("API: BREAK — {} symbol(s):", broken.len());
                for b in broken {
                    println!("  - {b}");
                }
            }
        }
        if reshaped.is_empty() {
            println!("Layout: stable.");
        } else {
            println!(
                "Layout: CHANGED — {} type(s) reshaped: {}",
                reshaped.len(),
                reshaped.join(", ")
            );
        }
    }
}

/// Minimal JSON string encoding (escape `"` and `\`; the symbols are identifiers + backticks).
fn api_json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Serialize a surface + its layout identity to a checked-in baseline (regenerated on release):
/// the API-surface member lines, a `--layout--` divider, then the @PLN97 layout sidecar.
fn emit_baseline(
    surface: &[loft::api_surface::Member],
    identity: &loft::schema_sidecar::LayoutIdentity,
) -> String {
    let mut s = String::from(
        "# loft api-surface baseline v1 — regenerate on release: \
         `loft api-surface <file> --emit-baseline`\n",
    );
    for m in surface {
        s.push_str(&m.to_line());
        s.push('\n');
    }
    s.push_str("--layout--\n");
    s.push_str(&identity.to_sidecar());
    s
}

/// Parse a baseline back into (members, layout identity); `None` on a malformed file.
fn parse_baseline(
    text: &str,
) -> Option<(
    Vec<loft::api_surface::Member>,
    loft::schema_sidecar::LayoutIdentity,
)> {
    let (surf, layout) = text.split_once("\n--layout--\n")?;
    let members: Vec<loft::api_surface::Member> = surf
        .lines()
        .filter_map(loft::api_surface::Member::from_line)
        .collect();
    let identity = loft::schema_sidecar::LayoutIdentity::from_sidecar(layout)?;
    Some((members, identity))
}

/// @PLN97 Phase F — `loft layout <accept|check> <file>`: the compiler migration
/// aid as an explicit, opt-in command (a normal build pays nothing). `accept`
/// records the program's current layout as the baseline (`.loft/layout.lock`);
/// `check` diffs the current layout against it and, on an actionable change,
/// prints the diagnostic + writes a migration outline to fill.
fn run_layout_command(sub: &str, file: &str) -> i32 {
    use loft::schema_sidecar as ss;
    let entry = std::path::PathBuf::from(file);
    if !entry.exists() {
        eprintln!("loft layout: file {file} not found");
        return 1;
    }
    let abs = portable_path::plain_canonical(&entry);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    let default_dir = exe_dir.join("../default");
    let default_str = if default_dir.exists() {
        default_dir.to_string_lossy().to_string()
    } else {
        format!("{}/default", project_dir())
    };
    let mut p = parser::Parser::new();
    if let Some(src_dir) = entry.parent() {
        p.lib_dirs.push(src_dir.to_string_lossy().to_string());
    }
    let _ = p.parse_dir(&default_str, true, false);
    p.parse(&abs.to_string_lossy(), false);

    let roots = ss::program_roots(&p.data);
    // @PLN102 arc-E F9 — pin full-width nullability (`Data` is live at the CLI site).
    let identity = ss::LayoutIdentity::of_scoped(&p.database, &roots, &p.data);
    let project = abs
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .to_path_buf();

    match sub {
        "accept" => match identity.write_baseline(&project) {
            Ok(()) => {
                println!(
                    "loft layout: recorded baseline ({} user types)",
                    roots.len()
                );
                0
            }
            Err(e) => {
                eprintln!("loft layout: could not write baseline: {e}");
                1
            }
        },
        "check" => match ss::check_against_baseline(&project, &identity) {
            Ok(ss::SchemaVerdict::Fresh) => {
                println!("loft layout: no baseline yet — run `loft layout accept {file}`.");
                0
            }
            Ok(ss::SchemaVerdict::Match) => {
                println!("loft layout: unchanged.");
                0
            }
            Ok(ss::SchemaVerdict::Changed(diff)) => {
                println!("{}", ss::describe_change(&diff));
                if diff.is_actionable() {
                    let path = project.join(".loft").join("migration_outline.loft");
                    if let Some(parent) = path.parent() {
                        let _ = std::fs::create_dir_all(parent);
                    }
                    match std::fs::write(&path, ss::migration_outline(&diff)) {
                        Ok(()) => println!("  migration outline written to {}", path.display()),
                        Err(e) => eprintln!("  could not write migration outline: {e}"),
                    }
                }
                0
            }
            Ok(ss::SchemaVerdict::Unreadable) => {
                eprintln!(
                    "loft layout: baseline unreadable — delete `.loft/layout.lock` and re-accept."
                );
                1
            }
            Err(e) => {
                eprintln!("loft layout: {e}");
                1
            }
        },
        other => {
            eprintln!("loft layout: unknown subcommand `{other}` (use `accept` or `check`)");
            1
        }
    }
}

/// `loft fix [--apply] <file…>` — @PLN131 steps 3–4: check each suggested fix by running
/// it, and write the ones that are safe unattended.
///
/// Without `--apply` this reports and changes nothing: each fix is applied to an in-memory
/// copy, the analysis is re-run, and the fix is labelled by what that measured. A
/// suggestion that has been TRIED is a different class of artefact from one that was
/// pattern-matched, and this is the command that tells them apart.
///
/// `--apply` writes only fixes that are **mechanical** and **verified**. A conditional one
/// is never written here however sound it looks: its correctness rests on something only
/// the author can affirm, and an unattended run has nobody to affirm it. Those stay in the
/// report with their condition, for a human or an editor's quick-fix to accept.
fn run_fix_command(args: &[String]) -> i32 {
    let mut apply = false;
    let mut files: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--apply" => apply = true,
            s if s.starts_with('-') => {
                eprintln!("loft fix: unknown option `{s}`");
                return 1;
            }
            s => files.push(s.to_string()),
        }
    }
    if files.is_empty() {
        eprintln!("loft fix: usage: loft fix [--apply] <file…>");
        return 1;
    }

    // Same stdlib resolution as `run_fmt_command`: beside the binary in a release layout,
    // else the source tree.
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    let default_dir = exe_dir.join("../default");
    let default_str = if default_dir.exists() {
        default_dir.to_string_lossy().to_string()
    } else {
        format!("{}/default", project_dir())
    };

    let mut exit = 0;
    for file in &files {
        let src = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("loft fix: cannot read {file}: {e}");
                exit = 1;
                continue;
            }
        };
        let diags = loft::lsp::diagnose(&src, file, &default_str);
        let (rewritten, report) = loft::fix_apply::apply_fixes(&src, file, &default_str, &diags);
        if report.is_empty() {
            continue; // Nothing to say. loft is BORING when there is no work.
        }
        println!("{file}");
        for r in &report {
            // Without `--apply` nothing is written, so nothing may SAY it was: a report
            // that claims an edit it did not make is the one output a reader cannot check.
            let mark = if r.written && apply {
                "applied"
            } else {
                r.verdict.tag()
            };
            println!("  {}:{}  {}  [{mark}]", file, r.line, r.title);
        }
        if apply && rewritten != src {
            if let Err(e) = std::fs::write(file, &rewritten) {
                eprintln!("loft fix: cannot write {file}: {e}");
                exit = 1;
                continue;
            }
            let n = report.iter().filter(|r| r.written).count();
            println!("  wrote {n} fix(es) to {file}");
        }
    }
    exit
}

/// `loft fmt [--check|--write] <file…>` — the parser-driven formatter, written in
/// loft (`tools/fmt/whole.loft`) and invoked via the `loft::host` call API.  Default
/// prints the formatted source; `--write` rewrites in place (reporting changes);
/// `--check` exits non-zero if any file is not already formatted (a CI gate).  `-`
/// reads stdin → stdout.  The formatter source is embedded in the binary.
fn run_fmt_command(args: &[String]) -> i32 {
    use loft::host::{Program, Value};
    const FMT_SRC: &str = include_str!("../tools/fmt/whole.loft");

    let mut check = false;
    let mut write = false;
    let mut files: Vec<String> = Vec::new();
    for a in args {
        match a.as_str() {
            "--check" => check = true,
            "--write" | "-w" => write = true,
            "-" => files.push("-".to_string()),
            s if s.starts_with('-') => {
                eprintln!("loft fmt: unknown option `{s}`");
                return 1;
            }
            s => files.push(s.to_string()),
        }
    }
    if files.is_empty() {
        eprintln!("loft fmt: usage: loft fmt [--check|--write] <file…>   (`-` = stdin→stdout)");
        return 1;
    }
    if check && write {
        eprintln!("loft fmt: --check and --write are mutually exclusive");
        return 1;
    }

    // Resolve the stdlib `default/` dir next to the binary (release layout), else the
    // source tree — same resolution as `run_layout_command`.
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    let default_dir = exe_dir.join("../default");
    let default_str = if default_dir.exists() {
        default_dir.to_string_lossy().to_string()
    } else {
        format!("{}/default", project_dir())
    };
    let mut prog = match Program::from_source_with_stdlib(FMT_SRC, &default_str) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("loft fmt: could not load formatter: {e}");
            return 1;
        }
    };

    let mut exit = 0;
    let mut unformatted: Vec<String> = Vec::new();
    for file in &files {
        let src = if file == "-" {
            use std::io::Read;
            let mut s = String::new();
            if std::io::stdin().read_to_string(&mut s).is_err() {
                eprintln!("loft fmt: cannot read stdin");
                return 1;
            }
            s
        } else {
            match std::fs::read_to_string(file) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("loft fmt: cannot read {file}: {e}");
                    exit = 1;
                    continue;
                }
            }
        };
        let formatted = match prog.call("format", &[Value::Text(src.clone())]) {
            Ok(v) => match v.into_text() {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("loft fmt: {file}: {e}");
                    exit = 1;
                    continue;
                }
            },
            Err(e) => {
                eprintln!("loft fmt: {file}: {e}");
                exit = 1;
                continue;
            }
        };
        if check {
            if formatted != src {
                unformatted.push(file.clone());
            }
        } else if write && file != "-" {
            if formatted != src {
                if let Err(e) = std::fs::write(file, &formatted) {
                    eprintln!("loft fmt: cannot write {file}: {e}");
                    exit = 1;
                    continue;
                }
                println!("formatted {file}");
            }
        } else {
            print!("{formatted}");
        }
    }
    if check && !unformatted.is_empty() {
        eprintln!("loft fmt: {} file(s) need formatting:", unformatted.len());
        for f in &unformatted {
            eprintln!("  {f}");
        }
        return 1;
    }
    exit
}

// ── agent-facing code-intelligence queries (@PLN63) ──────────────────────────
// One-shot CLI over the `loft::lsp` accessors — the same code intelligence the
// LSP server gives editors, reachable from the shell for scripts and agents.
// Human-readable by default; `--json` for structured output (mirrors `loft api`).

/// Resolve the stdlib `default/` dir (binary-relative, else the source tree —
/// the resolution `run_fmt_command` uses) AND enable the startup cache, so a
/// query warm-loads the precompiled stdlib `Data` (~10×) instead of cold-parsing.
fn lsp_default_dir() -> String {
    if std::env::var_os("LOFT_STDLIB_CACHE").is_none() {
        // SAFETY: single-threaded CLI startup, before any program runs.
        unsafe { std::env::set_var("LOFT_STDLIB_CACHE", "1") };
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(std::path::Path::to_path_buf))
        .unwrap_or_default();
    let default_dir = exe_dir.join("../default");
    let dir = if default_dir.exists() {
        default_dir
    } else {
        std::path::PathBuf::from(project_dir()).join("default")
    };
    // Canonicalize so recorded def paths are clean (no `..`, no `//`) — those
    // paths are shown to the user and pasted into `file:line` references.
    portable_path::plain_canonical(&dir)
        .to_string_lossy()
        .to_string()
}

/// JSON object from `(key, value)` pairs (the byte-offset slot is unused on emit).
fn jobj(entries: Vec<(&str, loft::json::Parsed)>) -> loft::json::Parsed {
    loft::json::Parsed::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), 0, v))
            .collect(),
    )
}

fn hover_json(h: &loft::lsp::Hover) -> loft::json::Parsed {
    use loft::json::Parsed;
    jobj(vec![
        ("name", Parsed::Str(h.name.clone())),
        ("signature", Parsed::Str(h.signature.clone())),
        (
            "doc",
            Parsed::Array(h.doc.iter().map(|l| Parsed::Str(l.clone())).collect()),
        ),
        ("file", Parsed::Str(h.def_file.clone())),
        ("line", Parsed::Int(i64::from(h.def_line))),
        ("col", Parsed::Int(i64::from(h.def_col))),
    ])
}

fn print_hover_human(h: &loft::lsp::Hover) {
    println!("{}", h.signature);
    for line in &h.doc {
        println!("    {line}");
    }
    println!("  \u{2192} {}:{}", h.def_file, h.def_line);
}

/// `loft symbols <file> [--json]` — the file's top-level definitions (outline).
fn run_symbols_command(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    let Some(file) = args.iter().find(|a| !a.starts_with('-')) else {
        eprintln!("loft symbols: usage: loft symbols <file.loft> [--json]");
        return 2;
    };
    let text = match std::fs::read_to_string(file) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("loft symbols: cannot read {file}: {e}");
            return 1;
        }
    };
    let dir = lsp_default_dir();
    let syms = loft::lsp::outline(&text, file, &dir);
    if json {
        use loft::json::Parsed;
        let arr = Parsed::Array(
            syms.iter()
                .map(|s| {
                    jobj(vec![
                        ("name", Parsed::Str(s.name.clone())),
                        ("kind", Parsed::Str(s.kind.to_string())),
                        ("line", Parsed::Int(i64::from(s.line))),
                        ("col", Parsed::Int(i64::from(s.col))),
                    ])
                })
                .collect(),
        );
        println!("{}", loft::json::to_json_string(&arr));
    } else {
        for s in &syms {
            println!("{:>5}:{:<3} {:<9} {}", s.line, s.col, s.kind, s.name);
        }
    }
    0
}

/// `loft def <name> [file] [--json]` — resolve a symbol by NAME to its
/// signature + doc + location: a free fn / type / const, PLUS every `Type.name`
/// method.  `file` optionally folds the buffer's own defs into the search.
fn run_def_command(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let Some(symbol) = positional.first() else {
        eprintln!("loft def: usage: loft def <name> [file.loft] [--json]");
        return 2;
    };
    let (text, name) = match positional.get(1) {
        Some(file) => match std::fs::read_to_string(file.as_str()) {
            Ok(t) => (t, (*file).clone()),
            Err(e) => {
                eprintln!("loft def: cannot read {file}: {e}");
                return 1;
            }
        },
        None => (String::new(), "query.loft".to_string()),
    };
    let dir = lsp_default_dir();
    let hits = loft::lsp::lookup(symbol, &text, &name, &dir);
    if json {
        let arr = loft::json::Parsed::Array(hits.iter().map(hover_json).collect());
        println!("{}", loft::json::to_json_string(&arr));
    } else if hits.is_empty() {
        eprintln!("loft def: '{symbol}' not found");
        return 1;
    } else {
        for (n, h) in hits.iter().enumerate() {
            if n > 0 {
                println!();
            }
            print_hover_human(h);
        }
    }
    0
}

/// `loft hover <file> <line> <col> [--json]` — the symbol under a cursor
/// (1-based line/col, matching editor gutters).
fn run_hover_command(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    let pos: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let (Some(file), Some(line_s), Some(col_s)) = (pos.first(), pos.get(1), pos.get(2)) else {
        eprintln!("loft hover: usage: loft hover <file.loft> <line> <col> [--json]  (1-based)");
        return 2;
    };
    let (Ok(line), Ok(col)) = (line_s.parse::<u32>(), col_s.parse::<u32>()) else {
        eprintln!("loft hover: line and col must be positive integers");
        return 2;
    };
    let text = match std::fs::read_to_string(file.as_str()) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("loft hover: cannot read {file}: {e}");
            return 1;
        }
    };
    let dir = lsp_default_dir();
    match loft::lsp::symbol_at(&text, file, &dir, line, col) {
        Some(h) if json => println!("{}", loft::json::to_json_string(&hover_json(&h))),
        Some(h) => print_hover_human(&h),
        None if json => println!("null"),
        None => {
            eprintln!("loft hover: no symbol at {file}:{line}:{col}");
            return 1;
        }
    }
    0
}

/// Walk up from the CWD to the nearest `index/` holding `tags.json`.
fn find_index_dir() -> Option<String> {
    let mut dir = std::env::current_dir().ok()?;
    loop {
        let cand = dir.join("index");
        if cand.join("tags.json").is_file() {
            return Some(cand.to_string_lossy().into_owned());
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn tag_json(info: &loft::lsp::TagInfo) -> loft::json::Parsed {
    use loft::json::Parsed;
    let opt = |o: &Option<String>| o.as_ref().map_or(Parsed::Null, |s| Parsed::Str(s.clone()));
    jobj(vec![
        ("tag", Parsed::Str(info.tag.clone())),
        ("kind", Parsed::Str(info.kind.to_string())),
        ("title", opt(&info.title)),
        ("summary", opt(&info.summary)),
        ("url", opt(&info.url)),
        ("references", Parsed::Int(info.references as i64)),
    ])
}

/// `loft refs <name> [root] [--json]` — every occurrence of an identifier across
/// the `.loft` files under `root` (default: CWD), via the workspace reverse index.
fn run_refs_command(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    let positional: Vec<&String> = args.iter().filter(|a| !a.starts_with('-')).collect();
    let Some(name) = positional.first() else {
        eprintln!("loft refs: usage: loft refs <name> [root] [--json]");
        return 2;
    };
    let root = positional.get(1).map_or_else(
        || {
            std::env::current_dir()
                .map_or_else(|_| ".".to_string(), |d| d.to_string_lossy().into_owned())
        },
        |s| (*s).clone(),
    );
    let wi = loft::lsp::WorkspaceIndex::build(&root);
    let refs = wi.references(name);
    if json {
        use loft::json::Parsed;
        let arr = Parsed::Array(
            refs.iter()
                .map(|r| {
                    jobj(vec![
                        ("file", Parsed::Str(r.file.clone())),
                        ("line", Parsed::Int(i64::from(r.line))),
                        ("col", Parsed::Int(i64::from(r.col))),
                    ])
                })
                .collect(),
        );
        println!("{}", loft::json::to_json_string(&arr));
    } else if refs.is_empty() {
        eprintln!("loft refs: no references to '{name}' under {root}");
        return 1;
    } else {
        for r in refs {
            println!("{}:{}:{}", r.file, r.line, r.col);
        }
    }
    0
}

/// `loft tag <@TAG> [--json]` — what the tracker index knows about a tag
/// (issue / feature / plan): title + summary + issue URL + reference count,
/// from `index/tags.json` + `index/features.json` (`make index`).
fn run_tag_command(args: &[String]) -> i32 {
    let json = args.iter().any(|a| a == "--json");
    let Some(tag) = args.iter().find(|a| !a.starts_with('-')) else {
        eprintln!("loft tag: usage: loft tag <@TAG> [--json]   (e.g. loft tag @F7)");
        return 2;
    };
    let Some(index_dir) = find_index_dir() else {
        eprintln!("loft tag: no index/tags.json found (run `make index` at the repo root)");
        return 1;
    };
    let Some(idx) = loft::lsp::TagIndex::load(&index_dir) else {
        eprintln!("loft tag: could not read {index_dir}/tags.json");
        return 1;
    };
    match idx.lookup(tag) {
        Some(info) if json => println!("{}", loft::json::to_json_string(&tag_json(&info))),
        Some(info) => println!("{}", loft::lsp::render_tag_markdown(&info)),
        None if json => println!("null"),
        None => {
            eprintln!("loft tag: '{tag}' — unknown or not indexed");
            return 1;
        }
    }
    0
}

#[allow(clippy::too_many_lines)]
fn main() {
    // @PLN119 arc A — this process is the worker holding one process-placed
    // library. Internal: spawned by `lib_placement::Worker::spawn`, never typed
    // by a user, so it is deliberately absent from `--help`. It takes over the
    // process and never returns.
    //
    // FIRST, before the execution-timeout watchdog below: a worker is idle
    // between calls by design, and the caller's `LOFT_TIMEOUT` is a bound on the
    // caller's work, not on how long a library is allowed to sit waiting to be
    // asked. Arming it here would kill a healthy worker mid-run.
    #[cfg(target_os = "linux")]
    if std::env::args().nth(1).is_some_and(|a| a == "--lib-worker") {
        let a: Vec<String> = std::env::args().skip(1).collect();
        let stdlib = a
            .iter()
            .position(|x| x == "--default")
            .and_then(|p| a.get(p + 1));
        match (a.get(1), a.get(2), stdlib) {
            (Some(w), Some(p), Some(s)) => loft::lib_placement::serve(
                std::path::Path::new(w),
                std::path::Path::new(p),
                std::path::Path::new(s),
            ),
            _ => {
                eprintln!(
                    "loft: --lib-worker is internal; usage: \
                     --lib-worker <wire> <pkg_dir> --default <stdlib_dir>"
                );
                std::process::exit(2);
            }
        }
    }
    // @PLN119 arc E — this process SERVES one library over a socket, so a
    // consumer elsewhere can declare `placement = "remote"` and reach it.
    //
    // Unlike `--lib-worker` this one is typed by a person: an operator starts it
    // where the library should run. It takes over the process and never returns,
    // and it is armed before the watchdog for the same reason a worker is —
    // sitting idle waiting to be asked is what it is FOR.
    #[cfg(target_os = "linux")]
    if std::env::args().nth(1).is_some_and(|a| a == "--lib-server") {
        let a: Vec<String> = std::env::args().skip(1).collect();
        let stdlib = a
            .iter()
            .position(|x| x == "--default")
            .and_then(|p| a.get(p + 1))
            .cloned()
            .unwrap_or_else(|| {
                let dir = std::env::current_exe()
                    .ok()
                    .and_then(|p| p.parent().map(std::path::Path::to_path_buf))
                    .unwrap_or_default()
                    .join("../default");
                if dir.exists() {
                    dir.to_string_lossy().into_owned()
                } else {
                    "default".to_string()
                }
            });
        match (a.get(1), a.get(2)) {
            (Some(addr), Some(pkg)) => loft::lib_placement::serve_remote(
                addr,
                std::path::Path::new(pkg),
                std::path::Path::new(&stdlib),
            ),
            _ => {
                eprintln!(
                    "loft: usage: --lib-server <host:port> <pkg_dir> [--default <stdlib_dir>]\n\
                     \n\
                     Serves ONE library's `pub fn` surface to consumers that declare\n\
                     `placement = \"remote\"`.  Point them at it with\n\
                     LOFT_REMOTE_<NAME>=<host:port>.\n\
                     \n\
                     The address is yours to choose and there is no default.  This is\n\
                     not an authenticated or encrypted channel and it is not a sandbox:\n\
                     it runs the library's functions for whoever connects, so bind it\n\
                     where only what should reach it can — 127.0.0.1 for a local test,\n\
                     a private network or a tunnel otherwise."
                );
                std::process::exit(2);
            }
        }
    }
    // Install SIGSEGV/SIGABRT/SIGBUS handler so crashes print the
    // last-executed opcode before the default handler fires.
    loft::crash_report::install("loft");
    // loft#665 piece 3 — render an internal panic as a loft diagnostic pointing at
    // the user's source, then fall through to the normal Rust report.
    loft::crash_report::install_panic_hook();
    // @PLAN49 T1+T3 — arm the execution-timeout watchdog from the env
    // (`LOFT_TIMEOUT=<secs>`) BEFORE we parse argv.  An explicit
    // `--timeout` later in argv re-arms (no-op — `arm` is idempotent)
    // but the env value is the floor.  MUST be `loft::timeout` (this
    // binary's module instance), not `loft::timeout` (the lib crate's
    // separate copy) — the binary runs its own `crate::` modules
    // (`loft::state::State` etc.), and the `checkpoint_*` call sites in
    // them resolve to `loft::timeout`, so the watchdog + breadcrumb must
    // share that same instance.  Arming `loft::timeout` set a different
    // set of statics the running code never reads.
    loft::timeout::arm(
        loft::timeout::env_timeout_secs(),
        loft::timeout::env_grace_secs(),
    );
    // Plan-07 phase 1 step 1.20 / phase 3 — chain a Rust panic hook
    // that surfaces the loft source position of the offending pc
    // before the default panic message.  Reads the per-thread snapshot
    // published by `State::execute_argv` via `crash_report`.
    //
    // It prints a position only when a recorded span COVERS the pc, and
    // otherwise says nothing.  The line renders exactly like a position the
    // reader can act on, so there is nothing to mark it a guess — and the span
    // table is sparse, so an inherited answer names whatever statement happened
    // to be wrapped last.  It sent a `#lock` write on line 8 to line 7, and,
    // with nothing preceding it in the user's file, to a stdlib file the program
    // never mentions (loft#1262).  A wrapped construct — a call, an arithmetic
    // fault site — still resolves, which is the case this exists for.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let (pc, _op, _fn_d_nr) = loft::crash_report::last_context();
        if pc != u32::MAX
            && let Some(pos) = loft::crash_report::source_loc_for_pc(pc)
        {
            eprintln!("  at {}:{}:{}", pos.file, pos.line, pos.pos);
        }
        prev_hook(info);
    }));
    let argv: Vec<String> = env::args_os()
        .skip(1)
        .map(|a| a.to_str().unwrap_or("").to_string())
        .collect();
    let mut i = 0;
    let mut file_name = String::new();
    let mut dir = project_dir();
    let mut project: Option<String> = None;
    let mut lib_dirs: Vec<String> = Vec::new();
    let mut log_conf: Option<String> = None;
    let mut production = false;
    let mut generate_log_config: Option<Option<String>> = None;
    let mut native_mode = true;
    // @PLN13 step 2 — `--script`: desugar a beginner script (loose top-level statements,
    // no `fn main`) into one run-once `fn main` before parsing. Opt-in for now.
    let mut script_mode = false;
    // LibCI: `loft test` / `--tests` default to the interpreter, but honour an
    // EXPLICIT `--native` (matching the `--help` docs for `--tests --native`).
    // Tracked separately because the `test`/`--tests` handlers force interpreter
    // unless native was explicitly requested (regardless of arg order).
    let mut native_requested = false;
    let mut native_release = false;
    // Plan-0.8.5 NDB.0 — `--native-debug` flag: pass `-Cdebuginfo=2`
    // to rustc, drop `-O` (unless `--native-release` is also set),
    // and preserve the generated `.rs` on disk so DWARF's `.debug_line`
    // table points at a real file the debugger can show.
    let mut native_debug = false;
    // @PLN98 P2 — `--lean`: strip the live/debug tier from the generated Rust
    // (no `live_flipped` entry checks, no `LOFT_LIVE_FNS`/`boot_stores`).  The
    // default keeps the live tier (non-breaking); this is opt-OUT.
    let mut lean = false;
    // @PLN98 P3.4 — opt IN to the browser live/debug tier.  A production `--html`
    // client ships WITHOUT it (default lean — no live-flip / breakpoint channel);
    // `--debug` or `--debug=<name>` includes the tier and bakes a debug NAME the
    // server uses to ADDRESS this client over the relay (`--debug` alone → "").
    let mut debug_name: Option<String> = None;
    let mut dump_only = false;
    // None  = flag not given
    // Some("") = flag given without explicit path → use .loft/ default
    // Some(path) = explicit output path
    let mut native_emit: Option<String> = None;
    let mut native_wasm: Option<String> = None;
    let mut native_android: Option<String> = None;
    // Plan-07 phase 2: --errors=compact|pretty CLI flag (overrides
    // LOFT_ERRORS env var).  None = use env-or-default (Pretty).
    let mut error_mode_arg: Option<String> = None;
    let mut html_out: Option<String> = None;
    // @PLN117 — `--threads` / `--no-threads`; `None` = decide from whether the
    // program actually uses `par`.
    let mut html_threads: Option<bool> = None;
    // loft#681 — the consumer supplies its OWN host for the emitted wasm, so the page
    // loft would write is never used and its shim's surface is not the relevant one.
    let mut html_host_provided = false;
    // loft#954 — keep the wasm `name` section, so a browser backtrace resolves.
    let mut html_names = false;
    let mut tests_dir: Option<String> = None;
    // loft#925 — whether `tests_dir` came from the `loft test` SUBCOMMAND, and
    // whether that subcommand was given a target of its own.  A target written
    // after a flag (`loft test --lib src tests/t1.loft`) is not a leading
    // positional, so it lands in `file_name` and the run falls back to the whole
    // `tests/` directory; these two say which case a leftover positional is.
    let mut test_subcommand = false;
    let mut test_target_given = false;
    // Plan-08 phase 01: --introspect mode collects per-section
    // selectors, output paths, and filters into one Options bundle.
    // The flag itself only toggles the mode; sub-flags accumulate
    // into `introspect_opts` and are flushed into a real
    // `introspect::Options` after argv parsing.
    let mut introspect_mode = false;
    // @PLN86 F12 — `loft sandbox-check <file>`: run the admission walk ONLY and print
    // Admitted / Rejected + diagnostics, NEVER execute.  The modder's "will this be
    // allowed?" loop; the policy comes from `loft.toml` like a normal run.
    let mut sandbox_check_mode = false;
    let mut introspect_sections: Vec<loft::introspect::Section> = Vec::new();
    let mut introspect_why: Option<String> = None;
    let mut introspect_bytecode_out: Option<String> = None;
    let mut introspect_rust_out: Option<String> = None;
    let mut introspect_slots_out: Option<String> = None;
    let mut introspect_types_out: Option<String> = None;
    let mut introspect_diff_against: Option<String> = None;
    let mut introspect_json = false;
    let mut introspect_trace = false;
    let mut introspect_fn_filter: Vec<String> = Vec::new();
    let mut introspect_all_fns = false;
    let mut native_lib_paths: Vec<String> = Vec::new();
    let mut no_warnings = false;
    let mut deny_warnings = false;
    let mut check_only = false;
    // Phase 6t Tier 4 — `--deps[=direct|=transitive]` walks the current
    // project's dep tree and runs `loft test` in each dep.  None = off,
    // Direct = only manifest.dependencies, Transitive = also deps-of-deps.
    let mut test_deps: Option<&'static str> = None;
    // @PLN77 T4/T5 — which lockfile drives registry-dep resolution, and which
    // packages to leave untested.  Both only mean anything under `--deps`.
    let mut deps_lock: Option<String> = None;
    let mut deps_skip: Vec<String> = Vec::new();
    let mut strict_deps = false;
    let mut user_args: Vec<String> = Vec::new();
    // loft#684 — an explicit `--` ends the CLI's own options: every token after it
    // is the script path (if still unknown) or a program argument, whatever it
    // spells.  The end-of-options marker is the convention a consumer reaches for
    // first, so it has to work even for a token the CLI would otherwise claim.
    let mut forward_all = false;

    while i < argv.len() {
        let a = argv[i].as_str();
        i += 1;
        // loft#684 — a program argument that happens to spell a subcommand
        // (`test`, `layout`, `build`, …) belongs to the program, not to the CLI.
        // Once the script path is known, a POSITIONAL token can only be a program
        // argument: every subcommand is the FIRST positional, never a later one.
        // Without this, `loft prog.loft <store> layout` printed the usage line for
        // `loft layout` and the program never ran — a failure that reads like a
        // broken script rather than a taken word.  Flags keep their existing
        // meaning (the `starts_with('-')` arm below forwards unrecognised ones);
        // after an explicit `--`, so does everything else.
        if forward_all || (!file_name.is_empty() && !a.starts_with('-')) {
            if file_name.is_empty() {
                file_name = a.to_string();
            } else {
                user_args.push(a.to_string());
            }
            continue;
        }
        if a == "--" {
            forward_all = true;
            continue;
        }
        if a == "--version" {
            println!("loft {}", env!("CARGO_PKG_VERSION"));
            return;
        } else if a == "--migrate-long" {
            // C54.B migration tool — rewrite `long` type references and
            // `l` literal suffixes to their post-C54.A equivalents.
            let dry_run = argv.get(i).is_some_and(|s| s == "--dry-run");
            if dry_run {
                i += 1;
            }
            let Some(target) = argv.get(i) else {
                eprintln!("usage: loft --migrate-long [--dry-run] <path-or-dir>");
                std::process::exit(1);
            };
            match loft::migrate_long::migrate_path(std::path::Path::new(target), dry_run) {
                Ok((scanned, modified)) => {
                    if dry_run {
                        println!(
                            "migrate-long (dry run): {scanned} file(s) scanned, \
                             {modified} would be rewritten"
                        );
                    } else {
                        println!(
                            "migrate-long: {scanned} file(s) scanned, \
                             {modified} rewritten"
                        );
                    }
                }
                Err(e) => {
                    eprintln!("migrate-long error: {e}");
                    std::process::exit(1);
                }
            }
            return;
        } else if a == "--path" {
            dir.clone_from(&argv[i]);
            i += 1;
        } else if a == "--project" {
            project = Some(argv[i].clone());
            i += 1;
        } else if a == "--lib" {
            lib_dirs.push(argv[i].clone());
            i += 1;
        } else if a == "--log-conf" {
            log_conf = Some(argv[i].clone());
            i += 1;
        } else if a == "--production" {
            production = true;
        } else if a == "--generate-log-config" {
            // Optional path: consume next arg only if it doesn't look like a flag or source file
            let path = if argv.get(i).is_some_and(|s| is_output_path(s)) {
                let p = argv[i].clone();
                i += 1;
                Some(p)
            } else {
                None
            };
            generate_log_config = Some(path);
        // @F48 — the loft CLI (run a program; --interpret / --native, --timeout, --help)
        } else if a == "--interpret" || a == "--bytecode" {
            native_mode = false;
        } else if a == "--script" {
            script_mode = true;
        } else if let Some(rest) = a.strip_prefix("--errors=") {
            // Plan-07 phase 2: --errors=compact|pretty selects the
            // diagnostic renderer.  Pretty is default; compact is
            // single-line for harnesses + CI.
            error_mode_arg = Some(rest.to_string());
        } else if a == "--errors" {
            // Two-arg form: `--errors compact`.
            error_mode_arg = argv.get(i).cloned();
            if error_mode_arg.is_some() {
                i += 1;
            }
        } else if a == "--dump" {
            native_mode = false;
            dump_only = true;
        } else if a == "sandbox-check" {
            // @PLN86 F12 — admission-only verdict; never executes (forced interpret
            // path, no codegen).  The program file follows as a positional arg.
            sandbox_check_mode = true;
            native_mode = false;
        } else if a == "--introspect" || a == "introspect" {
            // @PLN12 phase 01: introspection mode (flag or bare subcommand).
            // Default = emit bytecode + Rust + slots + types to stdout.
            // Sub-flags below narrow the section list, redirect per-section
            // output to files, and filter by function name.
            introspect_mode = true;
            native_mode = false;
        } else if a == "--show-bytecode" {
            introspect_sections.push(loft::introspect::Section::Bytecode);
        } else if a == "--bc-roundtrip" {
            introspect_sections.push(loft::introspect::Section::Roundtrip);
        } else if a == "--show-rust" {
            introspect_sections.push(loft::introspect::Section::Rust);
        } else if a == "--show-slots" {
            introspect_sections.push(loft::introspect::Section::Slots);
        } else if a == "--show-types" {
            introspect_sections.push(loft::introspect::Section::Types);
        } else if a == "--show-ownership" {
            introspect_sections.push(loft::introspect::Section::Ownership);
        } else if a == "--show-resolution" {
            introspect_sections.push(loft::introspect::Section::Resolution);
        } else if a == "--why" {
            // `--why <name>`: the resolution section, narrowed to one name.  Implies
            // the section, since asking the question is asking for it.
            introspect_sections.push(loft::introspect::Section::Resolution);
            // `argv[i]` is already the flag's VALUE here — the arg loop advanced past
            // the flag itself — so read before consuming, as `--path` does.
            introspect_why = argv.get(i).cloned();
            i += 1;
        } else if a == "--json" {
            // INSP.J — emit the introspection sections as one machine-readable
            // JSON object instead of the text dump (an editor / agent consumer).
            introspect_json = true;
        } else if a == "--bytecode-out" {
            introspect_bytecode_out = argv.get(i).cloned();
            i += 1;
        } else if a == "--rust-out" {
            introspect_rust_out = argv.get(i).cloned();
            i += 1;
        } else if a == "--slots-out" {
            introspect_slots_out = argv.get(i).cloned();
            i += 1;
        } else if a == "--types-out" {
            introspect_types_out = argv.get(i).cloned();
            i += 1;
        } else if a == "--diff" {
            introspect_diff_against = argv.get(i).cloned();
            i += 1;
        } else if a == "--trace" {
            introspect_trace = true;
        } else if a == "--fn" {
            if let Some(name) = argv.get(i) {
                introspect_fn_filter.push(name.clone());
                i += 1;
            }
        } else if a == "--all-fns" {
            introspect_all_fns = true;
        // @F53 — native-binary backend (--native → rustc)
        } else if a == "--native" {
            native_mode = true;
            native_requested = true;
        } else if a == "--native-release" {
            native_mode = true;
            native_requested = true;
            native_release = true;
        } else if a == "--native-debug" {
            // NDB.0 — emit DWARF; combine with --native-release if
            // optimised + debug-info is wanted.
            native_mode = true;
            native_requested = true;
            native_debug = true;
        } else if a == "--lean" {
            // @PLN98 P2 — opt OUT of the live/debug tier: the generated Rust
            // carries no live-dispatch machinery (smallest binary, no
            // live-flip / breakpoints).  Composes with any build target
            // (--native / --native-wasm / --html / --native-emit).
            lean = true;
        } else if a == "--debug" || a.starts_with("--debug=") {
            // @PLN98 P3.4 — opt IN to the browser debug tier + set the client's
            // debug name (`--debug=alice`; bare `--debug` → "").
            debug_name = Some(a.strip_prefix("--debug=").unwrap_or("").to_string());
        } else if a == "--dev-soft-halt" {
            // Plan-07 phase 4g.3 — demote dev-mode raises to
            // log-and-continue so a single run surfaces every
            // fault site.  Useful for porting / first-pass
            // scripts where the full pattern of breakage
            // matters more than fast loop-back on one site.
            //
            // SAFETY: set_var is unsafe in Rust 2024.  We set
            // the env var BEFORE any State is created (the
            // State::raise check reads via OnceLock that
            // captures the value on first call), so no
            // concurrent reads are in flight.
            unsafe {
                std::env::set_var("LOFT_DEV_SOFT_HALT", "1");
            }
        } else if a == "--report-copies" {
            // @PLN90 Step 5 — the user-facing copy report: every UNBOUND structure copy
            // (Avoidable / Forced) with its location, type and fix hint + a rollup. Off by
            // default; a perf lint, never an error (COPY_DIAGNOSTICS.md).
            //
            // SAFETY: as for --dev-soft-halt — set BEFORE any analysis runs; the gate reads it
            // via a OnceLock captured on first call, so no concurrent reads are in flight.
            unsafe {
                std::env::set_var("LOFT_REPORT_COPIES", "1");
            }
        } else if a == "--explain" {
            // @PLN131 — print the FIX line(s) under each diagnostic that carries them: what
            // to write instead, plus the concept it uses and where to read about it. Showing
            // only; nothing is applied.
            //
            // SAFETY: as for --report-copies — set BEFORE any analysis runs; the gate reads
            // it via a OnceLock captured on first call, so no concurrent reads are in flight.
            unsafe {
                std::env::set_var("LOFT_EXPLAIN", "1");
            }
        } else if a == "--native-emit" {
            // Optional path: consume next arg only if it looks like an output path
            native_emit = Some(if argv.get(i).is_some_and(|s| is_output_path(s)) {
                let p = argv[i].clone();
                i += 1;
                p
            } else {
                String::new() // sentinel: compute default from file_name later
            });
        } else if a == "--native-wasm" {
            // Optional path: consume next arg only if it looks like an output path
            native_wasm = Some(if argv.get(i).is_some_and(|s| is_output_path(s)) {
                let p = argv[i].clone();
                i += 1;
                p
            } else {
                String::new() // sentinel: compute default from file_name later
            });
        } else if a == "--native-android" {
            // @PLN-android B1 — cross-compile to an Android cdylib `.so` via the
            // NDK (aarch64-linux-android by default; ANDROID_NDK_HOME required).
            // Optional path: consume next arg only if it looks like an output path.
            native_android = Some(if argv.get(i).is_some_and(|s| is_output_path(s)) {
                let p = argv[i].clone();
                i += 1;
                p
            } else {
                String::new() // sentinel: compute default from file_name later
            });
        // @F54 — browser / WASM target (--html / --native-wasm)
        } else if a == "--html" {
            // single-file HTML export with compiled browser WASM.
            html_out = Some(if argv.get(i).is_some_and(|s| is_output_path(s)) {
                let p = argv[i].clone();
                i += 1;
                p
            } else {
                String::new()
            });
        // @PLN117 — override the automatic choice of whether the page carries
        // loft's browser thread pool.  By default a program that uses `par` gets
        // it and one that doesn't stays single-threaded.
        } else if a == "--threads" {
            html_threads = Some(true);
        } else if a == "--no-threads" {
            html_threads = Some(false);
        // loft#954 — a browser trap hands over a complete backtrace, and without the
        // wasm `name` section every frame in it is a bare index that resolves to
        // nothing.  Opt-in rather than default because the section is real bytes on a
        // page that is already large; the people who need it are debugging.
        } else if a == "--names" {
            html_names = true;
        } else if a == "--host-provided" || a == "--no-host-check" {
            // Extract-the-wasm workflows drive the module from their own JS (see
            // BROWSER_INTEROP's "loft owns the loop"), so an import loft's shim lacks is
            // not a defect to prevent — the shim is discarded with the page.
            html_host_provided = true;
        } else if a == "--tests" {
            // Optional directory/file: consume next non-flag arg.
            // Skip --native/--no-warnings/--deny-warnings that may appear between --tests and the path.
            let mut path = ".".to_string();
            while argv
                .get(i)
                .is_some_and(|s| s == "--native" || s == "--no-warnings" || s == "--deny-warnings")
            {
                if argv[i] == "--native" {
                    // LibCI: opt into native test compilation (matches --help).
                    native_requested = true;
                } else if argv[i] == "--no-warnings" {
                    no_warnings = true;
                } else if argv[i] == "--deny-warnings" {
                    deny_warnings = true;
                }
                i += 1;
            }
            if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                path.clone_from(&argv[i]);
                i += 1;
            }
            tests_dir = Some(path);
            // The test runner defaults to the interpreter; an explicit --native
            // (anywhere on the line) opts into native compilation per file.
            if !native_requested {
                native_mode = false;
            }
        } else if a == "--no-warnings" {
            no_warnings = true;
        } else if a == "--deps" {
            test_deps = Some("transitive");
        } else if a == "--deps=direct" {
            test_deps = Some("direct");
        } else if a == "--deps=transitive" {
            test_deps = Some("transitive");
        } else if let Some(v) = a.strip_prefix("--lock=") {
            // @PLN77 T4 — resolve registry deps through THIS lockfile, so a
            // candidate `loft.lock` can be tested before it is committed.
            if v.is_empty() {
                eprintln!("--lock= requires a path to a lockfile");
                std::process::exit(2);
            }
            // Read it HERE, not at the walk.  The walk happens after the host
            // project's whole suite, so a mistyped path would cost a full test
            // run before saying so — and it exits 2, a usage error, rather than
            // the 1 that means "tests failed".
            match loft::lockfile::read_lockfile(std::path::Path::new(v)) {
                Ok(Some(_)) => {}
                Ok(None) => {
                    eprintln!("--lock: no lockfile at {v}");
                    std::process::exit(2);
                }
                Err(e) => {
                    eprintln!("--lock: cannot read {v} — {e}");
                    std::process::exit(2);
                }
            }
            deps_lock = Some(v.to_string());
        } else if let Some(v) = a.strip_prefix("--skip=") {
            // @PLN77 T5 — leave these packages untested (known-broken on this
            // platform, or simply not this run's business).  Their own deps are
            // still walked; see `run_dep_tests`.
            deps_skip.extend(
                v.split(',')
                    .map(str::trim)
                    .filter(|n| !n.is_empty())
                    .map(str::to_string),
            );
            if deps_skip.is_empty() {
                eprintln!("--skip= requires at least one package name");
                std::process::exit(2);
            }
        } else if a == "--strict-deps" {
            // @PLN77 — hold dependencies to the same warning bar as the project
            // itself.  Off by default: lint debt inside a package you do not own
            // is not your build's failure.
            strict_deps = true;
        } else if a == "--deny-warnings" {
            // Lib CI gate: any Warning-level diagnostic on the run becomes
            // a non-zero exit, just like a parse error.  Used by extracted
            // library chunk CIs to prevent regression of clean libraries.
            // Defaults off so existing consumers are unaffected.
            // Env equivalent: LOFT_DENY_WARNINGS=1
            deny_warnings = true;
        } else if a == "--timeout" {
            // @PLAN49 T3 — `--timeout <secs>` arms the watchdog.  The
            // graceful T2 fault (when shipped) fires at `<secs>`; the
            // hard T1 kill at `<secs> + grace` (default 2s, overridable
            // via `LOFT_TIMEOUT_GRACE`).  `0` disables.
            let secs: u64 = argv.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                eprintln!("--timeout requires a non-negative integer (seconds)");
                std::process::exit(2);
            });
            i += 1;
            loft::timeout::arm(secs, loft::timeout::env_grace_secs());
        } else if a == "--check" {
            check_only = true;
        } else if a == "check" {
            // @PLN100 Slice 4 — a bare `loft check` in a project (loft.toml, no
            // `.loft` file arg) is the build+test GATE.  `loft check <file>` and
            // the `--check` flag keep the compile-check behaviour.
            let next_is_file = argv.get(i).is_some_and(|s| {
                !s.starts_with('-')
                    && std::path::Path::new(s)
                        .extension()
                        .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            });
            if next_is_file || !std::path::Path::new("loft.toml").exists() {
                check_only = true;
            } else {
                let mut requested: Vec<String> = Vec::new();
                let mut force = false;
                while let Some(arg) = argv.get(i) {
                    if arg == "--force" || arg == "--fresh" {
                        force = true;
                    } else if arg.starts_with('-') {
                        break;
                    } else {
                        requested.push(arg.clone());
                    }
                    i += 1;
                }
                let manifest = loft::manifest::read_manifest("loft.toml").unwrap_or_default();
                let entry = manifest.entry.clone().unwrap_or_else(|| {
                    let n = manifest.name.clone().unwrap_or_else(|| "main".to_string());
                    format!("src/{n}.loft")
                });
                let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
                let ok = loft::build_phase::check(&requested, &entry, &manifest, &cwd, force);
                std::process::exit(i32::from(!ok));
            }
        } else if a == "--help" || a == "-h" || a == "-?" {
            print_help();
            return;
        } else if a == "targets" {
            // loft#680 — answer "does this builtin exist on that target?" BEFORE a design
            // commits to it. The alternative was writing the program, building it for
            // `--html`, and reading a rustc error against generated Rust — which arrives
            // after the plan that assumed the builtin, not before it.
            print_target_surface(argv.get(i).map(String::as_str));
            return;
        } else if a == "repl" || a == "--repl" {
            // @PLN12 phase 04 — interactive `loft>` prompt.  Prompts + errors go
            // to stderr; evaluated results print to stdout (so a terminal sees
            // them and a pipe can capture them).
            start_repl();
        } else if a == "debug" {
            // @PLN16 M5a — `loft debug <file>:<line>` — interpreter-mode debugger on a
            // real source file: break at the line, drop into the interactive `(dbg)`
            // prompt (inspect / edit / step / undo).
            run_file_debugger();
        } else if a == "--fresh" {
            // @PLN12 REPL.S — recognised here only so a bare `loft --fresh`
            // reaches the REPL instead of "unknown option"; `start_repl` reads
            // the flag via an env scan and clears the saved session itself.
        } else if a == "build" {
            // @PLN100 Slice 2 — `loft build [target...] [entry.loft]`: build the
            // named targets (or the manifest's default-targets) for the project in
            // cwd, resolving each target's toolchain `requires` first.  Collect the
            // trailing positionals: a `.loft` file overrides the entry, anything
            // else is a target name.
            let mut requested: Vec<String> = Vec::new();
            let mut entry_override: Option<String> = None;
            let mut force = false;
            while let Some(arg) = argv.get(i) {
                if arg == "--force" || arg == "--fresh" {
                    // @PLN100 Slice 3 — rebuild every asset regardless of its
                    // fingerprint / TTL (a deterministic clean build; CI can pin it).
                    force = true;
                } else if arg.starts_with('-') {
                    break;
                } else if std::path::Path::new(arg)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
                {
                    entry_override = Some(arg.clone());
                } else {
                    requested.push(arg.clone());
                }
                i += 1;
            }
            let manifest = if std::path::Path::new("loft.toml").exists() {
                loft::manifest::read_manifest("loft.toml").unwrap_or_default()
            } else {
                loft::manifest::Manifest::default()
            };
            let entry = entry_override
                .or_else(|| manifest.entry.clone())
                .unwrap_or_else(|| {
                    let n = manifest.name.clone().unwrap_or_else(|| "main".to_string());
                    format!("src/{n}.loft")
                });
            if !std::path::Path::new(&entry).exists() {
                eprintln!(
                    "loft build: entry `{entry}` not found — run in a project dir (with a \
                     loft.toml declaring [package] entry / name), or pass a .loft file."
                );
                std::process::exit(1);
            }
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let ok = loft::build_phase::run(&requested, &entry, &manifest, &cwd, force);
            std::process::exit(i32::from(!ok));
        } else if a == "test" {
            // PKG.6: `loft test [target]` — run package tests.
            // Detects loft.toml in cwd, adds src/ to lib path, runs --tests tests/.
            let mut test_target = TESTS_DIR.to_string();
            test_subcommand = true;
            if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                test_target = resolve_test_target(&argv[i]);
                test_target_given = true;
                i += 1;
                // loft#916 — everything after the first target used to be dropped in
                // silence: `loft test good.loft alsogood.loft` ran the first, printed
                // `ok … 1 file`, and exited 0 even when the second file FAILED.  A
                // green reported for a file that did not run is the one failure mode a
                // test runner must not have, and the file count is the only place it
                // showed — which nobody re-reads when the point of naming two files was
                // that the whole run is slow.
                //
                // One target per run: the summary line is a single verdict over one
                // scope, and looping would print a partial one per file, which
                // misleads in a new way rather than fixing this one.  Only the
                // CONSECUTIVE leading positionals are examined, so a later flag's value
                // (`--lib <dir>`) is never mistaken for a second target.
                let extra: Vec<String> = argv[i..]
                    .iter()
                    .take_while(|s| !s.starts_with('-'))
                    .cloned()
                    .collect();
                if !extra.is_empty() {
                    eprintln!(
                        "loft test: one target per run, but {} were given ({}).\n\
                         Run them one at a time, or name a directory to run everything \
                         under it.",
                        extra.len() + 1,
                        std::iter::once(argv[i - 1].clone())
                            .chain(extra)
                            .map(|s| format!("`{s}`"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    );
                    std::process::exit(1);
                }
            }
            // Read loft.toml to find src/ directory, dependency paths, and native libs.
            let manifest_path = std::path::Path::new("loft.toml");
            if manifest_path.exists() {
                let manifest = loft::manifest::read_manifest("loft.toml").unwrap_or_default();
                let entry = manifest.entry.unwrap_or_else(|| "src".to_string());
                let src_dir = std::path::Path::new(&entry)
                    .parent()
                    .unwrap_or(std::path::Path::new("src"));
                let abs_src = std::env::current_dir()
                    .unwrap_or_default()
                    .join(src_dir)
                    .to_string_lossy()
                    .to_string();
                lib_dirs.push(abs_src);
                // Add parent directory so sibling packages (dependencies) are found.
                if !manifest.dependencies.is_empty() {
                    let parent = portable_path::plain_canonical(
                        &std::env::current_dir().unwrap_or_default().join(".."),
                    )
                    .to_string_lossy()
                    .to_string();
                    if !lib_dirs.contains(&parent) {
                        lib_dirs.push(parent);
                    }
                }
                // Register the package's own native lib for loading.
                // Dependency native libs are discovered when the parser
                // processes `use` statements via lib_path_manifest().
                if let Some(ref stem) = manifest.native {
                    let pkg_dir = std::env::current_dir()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let lib_file = loft::extensions::platform_lib_name(stem);
                    let prebuilt = format!("{pkg_dir}/native/{lib_file}");
                    if std::path::Path::new(&prebuilt).exists() {
                        native_lib_paths.push(prebuilt);
                    } else if let Some(built) = loft::extensions::auto_build_native(&pkg_dir, stem)
                    {
                        native_lib_paths.push(built);
                    }
                }
            } else if std::path::Path::new("src").is_dir() {
                let abs_src = std::env::current_dir()
                    .unwrap_or_default()
                    .join("src")
                    .to_string_lossy()
                    .to_string();
                lib_dirs.push(abs_src);
            }
            tests_dir = Some(test_target);
            // `loft test` defaults to the interpreter; an explicit `--native`
            // (e.g. `loft --native test draw`) compiles each test file to native
            // Rust and runs it — the LibCI native library gate uses this.
            if !native_requested {
                native_mode = false;
            }
        // @P229 G3, restored at the new location — pre-build every installed registry
        // package's native cdylib SEQUENTIALLY, so a parallel test runner does not have
        // many processes queueing on the one global build lock while holding its slots.
        // Not a user-facing verb: it is a CI warm-up, and it says what it did so a run
        // that pre-builds nothing is distinguishable from one that had nothing to do.
        } else if a == "--prebuild-natives" {
            #[cfg(feature = "registry")]
            {
                let (attempted, built) = loft::extensions::prebuild_installed_natives();
                println!("loft: pre-built {built} of {attempted} installed native package(s)");
            }
            #[cfg(not(feature = "registry"))]
            println!("loft: built without the registry feature — nothing to pre-build");
            return;
        // @F55 — package management (loft install, loft.toml, lockfile)
        } else if a == "install" {
            // Collect flags + positional in any order.
            #[cfg(feature = "registry")]
            let mut install_opts = loft::install::InstallOptions {
                allow_unsigned: true,
                refresh: false,
                offline: false,
                allow_prerelease: false,
                skip_lockfile: false,
                lock_path: None,
            };
            let mut positional: Vec<String> = Vec::new();
            while i < argv.len() {
                let a2 = argv[i].as_str();
                if a2 == "--refresh" {
                    #[cfg(feature = "registry")]
                    {
                        install_opts.refresh = true;
                    }
                    i += 1;
                } else if a2 == "--offline" {
                    #[cfg(feature = "registry")]
                    {
                        install_opts.offline = true;
                    }
                    i += 1;
                } else if a2 == "--prerelease" {
                    #[cfg(feature = "registry")]
                    {
                        install_opts.allow_prerelease = true;
                    }
                    i += 1;
                } else if a2 == "--allow-unsigned" {
                    #[cfg(feature = "registry")]
                    {
                        install_opts.allow_unsigned = true;
                    }
                    i += 1;
                } else if a2 == "--require-signature" {
                    #[cfg(feature = "registry")]
                    {
                        install_opts.allow_unsigned = false;
                    }
                    i += 1;
                } else if !a2.starts_with('-') {
                    positional.push(a2.to_string());
                    i += 1;
                } else {
                    break;
                }
            }
            // The first positional arg decides path-vs-registry mode.
            // Local-path install (`.`, `./`, `../`, `/`, contains `/`)
            // takes exactly one arg.  Registry install accepts N names.
            let first = positional.first().map(|s| s.as_str()).unwrap_or("");
            let is_local_path = first.starts_with('/')
                || first.starts_with("./")
                || first.starts_with("../")
                || first == "."
                || first.contains('/');
            if first.is_empty() {
                // loft#966 — bare `loft install` resolves the manifest's
                // `[dependencies]`, the npm/cargo reading of the verb and the one
                // `loft api` promises when it reports a dependency unresolved.  It used
                // to install the PROJECT, so the tool's only hint named the one command
                // that did not address the case it was printed for — and following it
                // left a copy in `~/.loft/lib/<name>` shadowing the registry (loft#667),
                // twice, from a command run for a different purpose.  Install-this-project
                // keeps its own spelling, `loft install .`, which always meant that.
                #[cfg(feature = "registry")]
                install_manifest_dependencies(&install_opts);
                #[cfg(not(feature = "registry"))]
                eprintln!(
                    "loft install: this build has no registry support; \
                     `loft install .` installs the package in this directory"
                );
            } else if is_local_path {
                install_package(&std::path::PathBuf::from(first));
            } else {
                // Registry install — multiple names allowed.
                #[cfg(feature = "registry")]
                {
                    install_from_registry_with_opts(&positional, &install_opts);
                    // PKG.STUB — refresh the in-project API stubs alongside
                    // the lockfile this install just wrote.
                    let cwd = std::env::current_dir().unwrap_or_default();
                    write_api_stubs(&cwd.join("loft.lock"), &cwd);
                }
                #[cfg(not(feature = "registry"))]
                {
                    for name in &positional {
                        install_from_registry(name);
                    }
                }
            }
            return;
        } else if a == "search" {
            // PKG.REG R8: client-side registry search.
            #[cfg(feature = "registry")]
            {
                let json = argv[i..].iter().any(|s| s == "--json");
                let query = argv[i..]
                    .iter()
                    .find(|s| !s.starts_with('-'))
                    .cloned()
                    .unwrap_or_default();
                search_registry(&query, json);
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft search: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "info" {
            // PKG.REG R8: per-package info (versions, latest, deps).
            #[cfg(feature = "registry")]
            {
                let Some(name) = argv.get(i) else {
                    eprintln!("loft info: package name required");
                    std::process::exit(1);
                };
                package_info(name);
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft info: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "api" {
            // PKG.STUB — agent-facing discovery: list reachable libraries, print
            // one library's public surface, or emit it as JSON (`--json`) for the
            // registry CI's `api` re-derive (S7-CI).
            #[cfg(feature = "registry")]
            {
                // First non-flag arg = the package dir (default cwd); robust to
                // flag order (`loft api --json <dir>` or `loft api <dir> --json`).
                let target = argv[i..].iter().find(|s| !s.starts_with('-')).cloned();
                if argv[i..].iter().any(|s| s == "--json") {
                    // The function-level surface as `[{ "sig":…, "doc":… }, …]` —
                    // the registry `validate.py` runs this on the cloned source and
                    // rejects a pasted `api` that disagrees (the no-drift gate).
                    let dir = target.map_or_else(
                        || std::env::current_dir().unwrap_or_default(),
                        std::path::PathBuf::from,
                    );
                    let items = loft::documentation::pkg_api_items(&dir);
                    println!("{}", loft::json::to_json_string(&api_items_json(&items)));
                } else if argv[i..].iter().any(|s| s == "--registry") {
                    let refresh = argv[i..].iter().any(|s| s == "--refresh");
                    api_registry_catalog(refresh);
                } else {
                    api_command(target.as_deref());
                }
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft api: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "bundle" {
            // @PLAN12 Phase 6.11 — offline bundle export/import.
            #[cfg(feature = "registry")]
            {
                let Some(sub) = argv.get(i) else {
                    eprintln!("loft bundle: subcommand required (export | import)");
                    std::process::exit(1);
                };
                i += 1;
                if sub == "export" {
                    let mut packages: Option<Vec<String>> = None;
                    let mut all = false;
                    let mut outdir: Option<String> = None;
                    while i < argv.len() {
                        let a2 = argv[i].as_str();
                        if a2 == "--all" {
                            all = true;
                            i += 1;
                        } else if a2 == "--packages" {
                            i += 1;
                            let Some(list) = argv.get(i) else {
                                eprintln!(
                                    "loft bundle export: --packages requires a comma-separated list"
                                );
                                std::process::exit(1);
                            };
                            packages = Some(
                                list.split(',')
                                    .filter(|s| !s.is_empty())
                                    .map(str::to_string)
                                    .collect(),
                            );
                            i += 1;
                        } else if !a2.starts_with('-') && outdir.is_none() {
                            outdir = Some(a2.to_string());
                            i += 1;
                        } else {
                            eprintln!("loft bundle export: unknown argument `{a2}`");
                            std::process::exit(1);
                        }
                    }
                    let Some(out) = outdir else {
                        eprintln!("loft bundle export: <outdir> required");
                        std::process::exit(1);
                    };
                    let code = bundle_export(&out, packages.as_deref(), all);
                    std::process::exit(code);
                } else if sub == "import" {
                    let Some(indir) = argv.get(i) else {
                        eprintln!("loft bundle import: <indir> required");
                        std::process::exit(1);
                    };
                    let code = bundle_import(indir);
                    std::process::exit(code);
                }
                eprintln!("loft bundle: unknown subcommand `{sub}`");
                std::process::exit(1);
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft bundle: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "update" {
            // @PLAN12 Phase 6.8 — refresh lockfile entries to latest
            // active version within each dep's loft.toml range.
            #[cfg(feature = "registry")]
            {
                let mut update_opts = UpdateOpts::default();
                let mut target: Option<String> = None;
                while i < argv.len() {
                    let a2 = argv[i].as_str();
                    if a2 == "--dry-run" {
                        update_opts.dry_run = true;
                        i += 1;
                    } else if a2 == "--check" {
                        update_opts.check_only = true;
                        i += 1;
                    } else if !a2.starts_with('-') && target.is_none() {
                        target = Some(a2.to_string());
                        i += 1;
                    } else {
                        eprintln!("loft update: unknown argument `{a2}`");
                        std::process::exit(1);
                    }
                }
                update_opts.target = target;
                let code = update_packages(&update_opts);
                // PKG.STUB — a real update rewrote the lockfile; refresh the
                // in-project API stubs with it.
                if code == 0 && !update_opts.dry_run && !update_opts.check_only {
                    let cwd = std::env::current_dir().unwrap_or_default();
                    write_api_stubs(&cwd.join("loft.lock"), &cwd);
                }
                std::process::exit(code);
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft update: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "yank" {
            // @PLAN12 Phase 6.7a — author-side yank workflow.
            // Emits the index.json + advisories.json edits ready
            // for the registry PR.
            #[cfg(feature = "registry")]
            {
                let mut target: Option<String> = None;
                let mut severity: Option<String> = None;
                let mut advisory: Option<String> = None;
                let mut summary: Option<String> = None;
                let mut affected: Option<String> = None;
                let mut fixed_in: Option<String> = None;
                while i < argv.len() {
                    let a2 = argv[i].as_str();
                    if a2 == "--severity" {
                        i += 1;
                        severity = argv.get(i).cloned();
                        i += 1;
                    } else if a2 == "--advisory" {
                        i += 1;
                        advisory = argv.get(i).cloned();
                        i += 1;
                    } else if a2 == "--summary" {
                        i += 1;
                        summary = argv.get(i).cloned();
                        i += 1;
                    } else if a2 == "--affected" {
                        i += 1;
                        affected = argv.get(i).cloned();
                        i += 1;
                    } else if a2 == "--fixed-in" {
                        i += 1;
                        fixed_in = argv.get(i).cloned();
                        i += 1;
                    } else if !a2.starts_with('-') && target.is_none() {
                        target = Some(a2.to_string());
                        i += 1;
                    } else {
                        eprintln!("loft yank: unknown argument `{a2}`");
                        std::process::exit(1);
                    }
                }
                let Some(target) = target else {
                    eprintln!("loft yank: <pkg>@<version> required");
                    eprintln!(
                        "  Usage: loft yank <pkg>@<ver> --severity <tier> --advisory <id> \\"
                    );
                    eprintln!(
                        "           --summary \"...\" --affected \">=X, <Y\" --fixed-in \"<ver>\""
                    );
                    std::process::exit(1);
                };
                let code = yank_package(
                    &target,
                    severity.as_deref(),
                    advisory.as_deref(),
                    summary.as_deref(),
                    affected.as_deref(),
                    fixed_in.as_deref(),
                );
                std::process::exit(code);
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft yank: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "new" {
            // @PLAN12 — `loft new <name>` scaffolds a fresh library
            // package: loft.toml + src/<name>.loft stub + tests/ +
            // canonical library-ci.yml (when --chunk is set).
            let mut native = false;
            let mut chunk = false;
            let mut name: Option<String> = None;
            while i < argv.len() {
                let a2 = argv[i].as_str();
                if a2 == "--native" {
                    native = true;
                    i += 1;
                } else if a2 == "--chunk" {
                    chunk = true;
                    i += 1;
                } else if !a2.starts_with('-') && name.is_none() {
                    name = Some(a2.to_string());
                    i += 1;
                } else {
                    eprintln!("loft new: unknown argument `{a2}`");
                    std::process::exit(1);
                }
            }
            let Some(name) = name else {
                eprintln!("loft new: library name required");
                eprintln!("  Usage: loft new <name> [--native] [--chunk]");
                std::process::exit(1);
            };
            let code = scaffold_library(&name, native, chunk);
            std::process::exit(code);
        } else if a == "compat" {
            // Library compatibility contract — `loft compat <api|test|check>`.
            // Self-contained + early-exit, like its api-surface sibling.  Registry-gated:
            // every sub-verb compares against a PUBLISHED release, which a build without the
            // registry feature cannot locate at all.
            #[cfg(feature = "registry")]
            {
                let rest: Vec<String> = argv[i..].to_vec();
                std::process::exit(run_compat_command(&rest));
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!(
                    "loft compat: this build has no registry support, so a published release \
                     cannot be fetched to compare against"
                );
                std::process::exit(2);
            }
        } else if a == "api-surface" {
            // @PLN102 C1 — `loft api-surface <file>` | `--diff <base> <new> [--json]`.
            // Self-contained + early-exit.
            let rest: Vec<String> = argv[i..].to_vec();
            std::process::exit(run_api_surface_command(&rest));
        } else if a == "layout" {
            // @PLN97 Phase F — `loft layout <accept|check> <file>`. Self-contained
            // + early-exit: never touches the normal build path (zero cost).
            let sub = argv.get(i).cloned().unwrap_or_default();
            i += 1;
            let Some(file) = argv.get(i).cloned() else {
                eprintln!("loft layout: usage: loft layout <accept|check> <file>");
                std::process::exit(1);
            };
            std::process::exit(run_layout_command(&sub, &file));
        } else if a == "fmt" {
            // Parser-driven formatter (loft-written, via the host-call API).
            std::process::exit(run_fmt_command(&argv[i..]));
        } else if a == "fix" {
            // @PLN131 steps 3–4 — verify each fix against the analysis, and write the
            // mechanical ones on `--apply`.
            std::process::exit(run_fix_command(&argv[i..]));
        } else if a == "symbols" {
            // @PLN63 — code-intelligence queries over the loft::lsp accessors.
            std::process::exit(run_symbols_command(&argv[i..]));
        } else if a == "def" {
            std::process::exit(run_def_command(&argv[i..]));
        } else if a == "hover" {
            std::process::exit(run_hover_command(&argv[i..]));
        } else if a == "tag" {
            std::process::exit(run_tag_command(&argv[i..]));
        } else if a == "refs" {
            std::process::exit(run_refs_command(&argv[i..]));
        } else if a == "publish" {
            // @PLAN12 Phase 6.16 — author-side publish helper.
            // Repackages locally (deterministic), verifies the
            // GitHub release exists with the expected tag + asset,
            // emits the index.json entry ready for the registry PR.
            #[cfg(feature = "registry")]
            {
                let mut dry_run = false;
                let mut pkg_path: Option<String> = None;
                while i < argv.len() {
                    let a2 = argv[i].as_str();
                    if a2 == "--dry-run" {
                        dry_run = true;
                        i += 1;
                    } else if !a2.starts_with('-') && pkg_path.is_none() {
                        pkg_path = Some(a2.to_string());
                        i += 1;
                    } else {
                        eprintln!("loft publish: unknown argument `{a2}`");
                        std::process::exit(1);
                    }
                }
                let dir = pkg_path
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
                let code = publish_package(&dir, dry_run);
                std::process::exit(code);
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft publish: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "audit" {
            // @PLAN12 Phase 6.7 — explicit deep scan: every cached
            // package vs the advisory feed.  Exit code reflects
            // worst severity.
            #[cfg(feature = "registry")]
            {
                let code = audit_installed();
                std::process::exit(code);
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft audit: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "self-update" {
            // @PLN78 step 3 — resolve + report only; step 4 adds the replacement.
            #[cfg(feature = "registry")]
            {
                let rest = &argv[i..];
                let has = |f: &str| rest.iter().any(|x| x == f);
                let from = rest
                    .iter()
                    .position(|x| x == "--from")
                    .and_then(|p| rest.get(p + 1))
                    .map(String::as_str);
                std::process::exit(self_update_cmd(&SelfUpdateArgs {
                    dry_run: has("--dry-run"),
                    refresh: has("--refresh"),
                    allow_unsigned: has("--allow-unsigned"),
                    from,
                    force: has("--force"),
                }));
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!(
                    "loft self-update: this binary was built without the `registry` feature."
                );
                std::process::exit(1);
            }
        } else if a == "verify-self" {
            // @PLN78 step 2 — read-only: hash the installation against the manifests
            // the release bundle ships, and say what that does and does not prove.
            std::process::exit(verify_self_cmd());
        } else if a == "list-installed" {
            // @PLAN12 Phase 6.6 — enumerate ~/.loft/registry/<pkg>-<ver>/
            // dirs, annotate with sha256 + size from the cached index.
            #[cfg(feature = "registry")]
            {
                list_installed();
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!(
                    "loft list-installed: this binary was built without the `registry` feature."
                );
                std::process::exit(1);
            }
        } else if a == "pin" {
            // @PLAN12 Phase 6.6 — write a sidecar lockfile next to
            // a script so subsequent runs use pinned versions
            // regardless of cwd or registry drift.
            #[cfg(feature = "registry")]
            {
                let Some(script) = argv.get(i) else {
                    eprintln!("loft pin: script path required");
                    std::process::exit(1);
                };
                pin_script(script);
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!("loft pin: this binary was built without the `registry` feature.");
                std::process::exit(1);
            }
        } else if a == "ship" {
            // C96 — the maintainer ship verb.  On a key-present machine (the local
            // trust-root file key exists) it packages + signs + pushes every own lib
            // newer than the index, autonomously; a key-absent machine can only defer
            // to a submission.  Wraps scripts/registry_maintain.sh (which chains the
            // CAS-retry sign+push in registry-sign.sh).
            std::process::exit(run_ship_command(&argv[i..]));
        } else if a == "registry" {
            handle_registry(&argv, &mut i);
            return;
        } else if a == "cache" {
            // loft#861 — the other side of the auto-native caches, which only grew.
            handle_cache(&argv, &mut i);
            return;
        } else if a == "generate" {
            // PKG.6a: `loft generate` — emit Rust stubs for #native declarations.
            let pkg_path = if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                std::path::PathBuf::from(&argv[i])
            } else {
                std::env::current_dir().unwrap_or_default()
            };
            generate_native_stubs(&pkg_path);
            return;
        } else if a == "package" {
            // PKG.REG R1 (PKG_REGISTRY.md): `loft package [path]` — build a
            // gzipped tarball + print SHA-256 + size + the registry-index
            // entry the publisher pastes into loft-lang/registry.
            // Feature-gated on `registry` because tar / flate2 / sha2
            // aren't worth carrying in a no-default-features build.
            //
            // `--tarball-only` builds the tarball and stops.  It exists because
            // packaging has a second, purely mechanical use: the
            // reproducible-build check re-packages every published library just
            // to compare bytes against its release, and must not care what any
            // of them declares.  Registering is the act that needs the
            // compatibility levels, so that is what the flag opts out of —
            // and it opts out of the index entry too, since the entry IS the
            // registration.
            #[cfg(feature = "registry")]
            {
                let tarball_only = argv[i..].iter().any(|s| s == "--tarball-only");
                let pkg_path = if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                    // `i += 1` would be dead since we `return` below, but
                    // the arg is consumed for clarity.
                    std::path::PathBuf::from(&argv[i])
                } else {
                    std::env::current_dir().unwrap_or_default()
                };
                match loft::package::package_create(&pkg_path, None) {
                    Ok(out) => {
                        let stdout = std::io::stdout();
                        let mut lock = stdout.lock();
                        if tarball_only {
                            drop(lock);
                            println!("{}", out.tarball.display());
                            return;
                        }
                        if out.levels.is_none() {
                            // ADVISORY here, fatal at the registry PR (`loft compat
                            // levels`).  Packaging is something a library does to itself —
                            // a local check, a byte comparison, an artifact for a release
                            // — and a library in its current form has to keep doing all of
                            // that unchanged.  Asking to enter the REGISTRY is the act that
                            // needs the declaration, because that is the point where other
                            // people start depending on the answer.
                            eprintln!(
                                "warning: `{}` declares no compatibility floor, so a registry \
                                 PR for it will be REJECTED.\n  Run `loft compat levels` for \
                                 the exact fields, or `loft compat floor` to measure what \
                                 they should say.\n  The tarball and the entry below are \
                                 still correct for every other use.",
                                out.name
                            );
                        }
                        if let Err(e) = loft::package::print_summary(&out, &mut lock) {
                            eprintln!("loft package: print summary failed: {e}");
                            std::process::exit(1);
                        }
                    }
                    Err(e) => {
                        eprintln!("loft package: {e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!(
                    "loft package: this binary was built without the `registry` feature; \
                     rebuild with default features."
                );
                std::process::exit(1);
            }
        } else if a == "build-native" {
            // @PLN21 Phase 4 producer — build a package's native cdylib for THIS
            // host and report the artifact + the loft-ffi fingerprint + the host
            // triple, so CI can publish it as a `prebuilt/<triple>/` registry
            // binary.  Runs NO program (a graphics lib needs no display) — just
            // the cdylib + its fp, the two things `loft install` then needs.
            #[cfg(feature = "registry")]
            {
                let pkg_path = if argv.get(i).is_some_and(|s| !s.starts_with('-')) {
                    std::path::PathBuf::from(&argv[i])
                } else {
                    std::env::current_dir().unwrap_or_default()
                };
                // Canonicalize: the auto-native branch resolves the library via
                // `use <name>` and filters its defs by `pkg_str` prefix, so the
                // path the parser opens and `pkg_str` must be one form.
                let pkg_path = portable_path::plain_canonical(&pkg_path);
                let manifest =
                    loft::manifest::read_manifest(&pkg_path.join("loft.toml").to_string_lossy());
                let pkg_str = pkg_path.to_string_lossy().to_string();
                // Machine-readable so a publish script can capture it.  The KEY
                // line differs by native model, because the two cdylibs have
                // different ABI contracts (see the auto-native branch below):
                //   • hand-written links loft-ffi's `#[repr(C)]` surface → valid for
                //     ANY loft on the same loft-ffi → `loft_ffi_fp`;
                //   • auto-compiled `extern crate loft` (statically embeds libloft,
                //     shares repr(Rust) `Stores`/`DbRef` by memory) → valid only for a
                //     byte-identical loft build → `loft_build_fp` + the rustc it used.
                let report = |cdylib: String, stem: String, keys: &[String]| {
                    println!("cdylib: {cdylib}");
                    println!("stem: {stem}");
                    println!("triple: {}", loft::cache::host_triple());
                    for k in keys {
                        println!("{k}");
                    }
                };
                if let Some(stem) = manifest.as_ref().and_then(|m| m.native.clone()) {
                    // Hand-written `native/` crate — links the loft-ffi C ABI, so the
                    // wide loft-ffi key is the correct compatibility gate.
                    let keys = [format!(
                        "loft_ffi_fp: {}",
                        loft::cache::loft_ffi_fingerprint()
                    )];
                    match loft::extensions::auto_build_native(&pkg_str, &stem) {
                        Some(cdylib) => report(cdylib, stem, &keys),
                        None => {
                            eprintln!(
                                "loft build-native: building `{stem}` failed (see the cargo error above)"
                            );
                            std::process::exit(1);
                        }
                    }
                } else if let Some(name) = manifest.and_then(|m| m.name) {
                    // @PLN21 Phase 4b — AUTO-COMPILED native (the default
                    // "libraries compile, scripts interpret" path): parse the
                    // library the `use`-resolution way, then build its auto-native
                    // cdylib (`loft_auto_<dir>`, exporting `loft_shared_<fn>`
                    // wrappers) FROM the loft source — so a pure-loft compute
                    // library also ships a toolchain-free prebuilt.
                    let mut p = parser::Parser::new();
                    // Resolve the library FROM the handed package path (a fresh
                    // checkout, not necessarily registry-installed): its parent dir
                    // lets `use <name>` resolve `<parent>/<name>` (and sibling deps)
                    // BEFORE the registry fallback.  Without it `use <name>` finds a
                    // same-named registry package and `library_export_set` (filtering
                    // by `pkg_str` prefix) sees none of its defs.
                    if let Some(parent) = pkg_path.parent() {
                        p.lib_dirs.push(parent.to_string_lossy().to_string());
                    }
                    let default_dir = format!("{}/default", project_dir());
                    let _ = p.parse_dir(&default_dir, true, false);
                    let tmp = std::env::temp_dir()
                        .join(format!("loft_build_native_{}.loft", std::process::id()));
                    let _ = std::fs::write(&tmp, format!("use {name};\n"));
                    p.parse(&tmp.to_string_lossy(), false);
                    let _ = std::fs::remove_file(&tmp);
                    // Slot/scope analysis the cdylib codegen depends on — the run
                    // path runs this before its auto-native build (main.rs), and
                    // without it codegen emits undeclared locals (e.g. `var_me`).
                    scopes::check(&mut p.data, &mut p.database);
                    let export = loft::native_lib::library_export_set(&p.data, &pkg_str);
                    if export.is_empty() {
                        eprintln!(
                            "loft build-native: `{name}` has no native-compilable public functions"
                        );
                        std::process::exit(1);
                    }
                    match loft::native_lib::cached_or_build_shared_cdylib(
                        &p.data,
                        &p.database,
                        &export,
                        &pkg_str,
                        std::slice::from_ref(&pkg_str),
                    ) {
                        Ok(Some(so)) => {
                            // This cdylib `extern crate loft`s — it statically embeds
                            // libloft and operates on the host's repr(Rust)
                            // `Stores`/`DbRef` by shared memory, so it is valid ONLY
                            // for a byte-identical loft build (the `loft_build_fp` rlib
                            // hash, which already folds in source + rustc).  Reporting
                            // `loft_ffi_fp` here would mislabel it as widely portable —
                            // a corruption-shaped trap for any consumer that gated on
                            // it.  rustc is named too, for human/diagnostic clarity.
                            report(
                                so.to_string_lossy().to_string(),
                                loft::native_lib::auto_cdylib_stem(&pkg_str),
                                &[
                                    format!(
                                        "loft_build_fp: {}",
                                        loft::cache::loft_build_fingerprint()
                                    ),
                                    format!(
                                        "rustc: {}",
                                        option_env!("LOFT_BUILD_RUSTC").unwrap_or("unknown")
                                    ),
                                ],
                            );
                        }
                        Ok(None) => {
                            eprintln!(
                                "loft build-native: `{name}` is being edited (dev-interpret); no cdylib produced"
                            );
                            std::process::exit(1);
                        }
                        Err(e) => {
                            eprintln!(
                                "loft build-native: auto-native build of `{name}` failed: {e}"
                            );
                            std::process::exit(1);
                        }
                    }
                } else {
                    eprintln!(
                        "loft build-native: {} is not a loft package (no [package] name)",
                        pkg_path.display()
                    );
                    std::process::exit(1);
                }
                return;
            }
            #[cfg(not(feature = "registry"))]
            {
                eprintln!(
                    "loft build-native: requires the `registry` feature; rebuild with default features."
                );
                std::process::exit(1);
            }
        } else if a == "doc" {
            // PKG.8: `loft doc [path | library] [-o <dir>]` — HTML docs for a package.
            //
            // loft#911 — the argument used to be a PATH only, but the command reads as
            // (and is used as) `loft doc <library>`.  A library name is not a directory,
            // so `loft doc graphics` took the default-manifest branch, created
            // `./graphics/doc/` out of nothing wherever the user happened to stand, found
            // no `src/`, and reported "0 API sections" for a package with 119 documented
            // `pub fn`s.  Two rules close that: a name that resolves to nothing produces
            // an ERROR and no directory, and an installed package's docs go to loft's own
            // doc cache rather than the CWD or the immutable registry copy.
            let mut target: Option<String> = None;
            let mut out_override: Option<std::path::PathBuf> = None;
            let mut j = i;
            while let Some(arg) = argv.get(j) {
                j += 1;
                if arg == "-o" || arg == "--out" {
                    match argv.get(j) {
                        Some(dir) => {
                            out_override = Some(std::path::PathBuf::from(dir));
                            j += 1;
                        }
                        None => {
                            eprintln!("loft doc: `{arg}` needs a directory");
                            std::process::exit(1);
                        }
                    }
                } else if !arg.starts_with('-') && target.is_none() {
                    target = Some(arg.clone());
                }
            }
            let (pkg_path, default_out) = match target {
                None => (std::env::current_dir().unwrap_or_default(), None),
                Some(t) => {
                    let as_path = std::path::PathBuf::from(&t);
                    // Resolving a NAME needs the installed-package index, which the
                    // `registry` feature owns — a build without it has no installed
                    // packages to search, so the name simply does not resolve and the
                    // error below is the honest answer.
                    #[cfg(feature = "registry")]
                    // `installed_packages` is sorted by (name, version), so the LAST
                    // match is the newest installed version of that name.
                    let installed = loft::registry_index::installed_packages()
                        .into_iter()
                        .rfind(|(n, _, _)| *n == t);
                    #[cfg(not(feature = "registry"))]
                    let installed: Option<(
                        String,
                        String,
                        std::path::PathBuf,
                    )> = None;
                    if as_path.is_dir() {
                        (as_path, None)
                    } else if let Some((name, version, dir)) = installed {
                        // An installed package is shared, immutable cache content: its
                        // docs belong beside it in loft's own tree, not inside it.
                        (dir, Some(doc_cache_dir().join(format!("{name}-{version}"))))
                    } else {
                        eprintln!(
                            "loft doc: `{t}` is neither a directory nor an installed package.\n\
                             Point it at a package directory, or install the library first \
                             (`loft install {t}`)."
                        );
                        std::process::exit(1);
                    }
                }
            };
            let out_dir = out_override.or(default_out);
            if let Err(e) = loft::documentation::generate_pkg_docs(&pkg_path, out_dir.as_deref()) {
                eprintln!("Error generating docs: {e}");
                std::process::exit(1);
            }
            return;
        } else if a.starts_with('-') {
            // once the script path has been seen, treat every later
            // token (including `--*` ones) as a script argument and forward
            // it to the script's `arguments()`. The loft CLI cannot ambiguate
            // its own options from script options after the script path is
            // known. `--` never reaches here: it is the end-of-options marker,
            // consumed at the top of the loop.
            if !file_name.is_empty() {
                user_args.push(a.to_string());
            } else {
                println!("unknown option: {a}");
                println!("usage: loft [options] <file>");
                println!("Try `loft --help` for more information.");
                std::process::exit(1);
            }
        } else if file_name.is_empty() {
            file_name = a.to_string();
        } else {
            user_args.push(a.to_string());
        }
    }
    // Resolve sentinel empty paths to .loft/ defaults now that file_name is known.
    if let Some(ref mut p) = native_wasm
        && p.is_empty()
        && !file_name.is_empty()
    {
        *p = default_artifact_path(&file_name, "wasm")
            .to_str()
            .unwrap_or("out.wasm")
            .to_string();
    }
    if let Some(ref mut p) = native_emit
        && p.is_empty()
        && !file_name.is_empty()
    {
        *p = default_artifact_path(&file_name, "rs")
            .to_str()
            .unwrap_or("out.rs")
            .to_string();
    }

    // Handle --generate-log-config before requiring an input file
    if let Some(path_opt) = generate_log_config {
        handle_generate_log_config(path_opt.as_deref());
        return;
    }

    // loft#925 — a target written AFTER a flag is still the target.
    //
    // `loft test`'s own parse takes only a LEADING positional, so
    // `loft test --lib src tests/t1.loft` left the target at its `tests/` default
    // and the path fell through to `file_name` — which the `--tests` dispatch below
    // never reads.  The whole suite ran, and reported `21 passed; 21 files` for a
    // run that had been asked for ONE.  That is loft#916's failure mode exactly (a
    // green over a scope nobody asked for), surviving in the ordering its fix did
    // not cover, and it is what stopped loft#925's reporter cutting a standalone
    // repro: every invocation they tried ran everything.
    //
    // A leftover positional is therefore adopted as the target when none was given,
    // and refused when one was — the same either/or the leading-positional check
    // makes, so the two orderings cannot disagree about what two targets mean.
    if test_subcommand && !file_name.is_empty() {
        if test_target_given {
            eprintln!(
                "loft test: one target per run, but two were given (`{}`, `{file_name}`).\n\
                 Run them one at a time, or name a directory to run everything under it.",
                tests_dir.as_deref().unwrap_or(TESTS_DIR)
            );
            std::process::exit(1);
        }
        tests_dir = Some(resolve_test_target(&file_name));
        file_name.clear();
    }

    // Handle --tests before requiring an input file
    if let Some(ref test_dir) = tests_dir {
        // loft#964 — refuse a compile-target flag the test runner does not implement,
        // rather than accepting it and running something else.
        //
        // `loft test --native-wasm` exited 0, reported success, and ran the INTERPRETER.
        // The banner did say "ran on the interpreter only", but it says that on every
        // interpreter run, so it reads as a suggestion for a run you did not ask for
        // rather than as notice that the flag you passed was dropped — a library author
        // could green-light the wasm column of the target matrix on a run that never
        // touched it.
        //
        // Refused as a group, not one flag at a time: this is the third flag found
        // silently dropped on the test path (#860 `LOFT_PROFILE`, #865), so what needs
        // to hold is *the test runner rejects what it cannot honour*, and a per-flag
        // patch would leave the next one to be discovered the same way.
        for (flag, set) in [
            ("--native-wasm", native_wasm.is_some()),
            ("--html", html_out.is_some()),
            ("--native-android", native_android.is_some()),
            ("--native-emit", native_emit.is_some()),
        ] {
            if set {
                eprintln!(
                    "loft test: `{flag}` is not supported by the test runner — it compiles \
                     a program, and a test run has no single program to compile.\n\
                     Run the suite on a backend it does have (`loft test` for the \
                     interpreter, `loft test --native` for native), or build the target \
                     from a program entry (`loft {flag} <program>.loft`)."
                );
                std::process::exit(2);
            }
        }
        // @PLAN49 T3 — default the timeout ON under `loft test` / `--tests`.
        // This is the auto-mode case the watchdog exists for: a hung test or a
        // looping compile in the suite can't be killed interactively, so a
        // generous deadline self-kills it with a breadcrumb.  `arm` is
        // idempotent, so an explicit positive `--timeout <secs>` or
        // `LOFT_TIMEOUT=<secs>` (armed earlier in `main`) overrides this
        // default.  300s is far longer than any single test's compile+run,
        // short enough to catch a true infinite loop.
        loft::timeout::arm(300, loft::timeout::env_grace_secs());
        // Env-var equivalent so external CI doesn't need to thread the flag
        // through `loft test` invocations buried in shell loops.
        let deny_warnings = deny_warnings
            || std::env::var("LOFT_DENY_WARNINGS")
                .map(|v| !v.is_empty() && v != "0")
                .unwrap_or(false);
        let exit_code = run_tests(
            &dir,
            test_dir,
            no_warnings,
            deny_warnings,
            &lib_dirs,
            project.as_deref(),
            native_mode,
            &native_lib_paths,
        );
        // Phase 6t Tier 4 — `loft test --deps` walks the current
        // project's transitive (or direct) dep tree and runs `loft test`
        // in each dep's directory.  Failures are reported per-dep; the
        // process exits non-zero if any dep failed OR if the host
        // project's own tests failed.
        // `--lock` / `--skip` only mean anything under `--deps`.  Passed without it
        // they would change nothing and look accepted, so say so — the run is still
        // valid, which is why this reports rather than refuses.
        if test_deps.is_none() && (deps_lock.is_some() || !deps_skip.is_empty() || strict_deps) {
            let flag = if deps_lock.is_some() {
                "--lock"
            } else if strict_deps {
                "--strict-deps"
            } else {
                "--skip"
            };
            eprintln!("  note: {flag} has no effect without --deps");
        }
        let final_code = if let Some(mode) = test_deps {
            let transitive = mode == "transitive";
            let dep_fail = run_dep_tests(
                transitive,
                native_mode,
                deps_lock.as_deref(),
                &deps_skip,
                strict_deps,
            );
            i32::from(exit_code != 0 || dep_fail != 0)
        } else {
            exit_code
        };
        std::process::exit(final_code);
    }

    if file_name.is_empty() {
        // @PLN12 phase 04 — no file and no subcommand: drop into the interactive
        // REPL (like `python` / `node` / `irb`).  Works for a terminal and for
        // piped input; the banner advertises `:help`.
        start_repl();
    }
    // Resolve the script path to absolute before potentially changing directory.
    let abs_file = portable_path::plain_canonical(std::path::Path::new(&file_name));
    let abs_file = abs_file.to_str().unwrap().to_string();
    // @P296-sibling (Windows-only): `canonicalize()` returns an
    // extended-length `\\?\D:\…` verbatim path, but library `use`
    // resolution (`lib_path` / `probe_*`) builds plain paths.  When the
    // entry file carries the verbatim prefix, a `lib::Name` reference in
    // it registers the module under a source derived from the verbatim
    // path while the same module loaded via `use` uses the plain form —
    // the two sources don't dedup → "Dual definition of <lib>" on
    // Windows (crystal_gold CI).  Strip the verbatim-disk prefix so every
    // path shares one representation (`portable_path::plain_canonical`, which already
    // shed the prefix above; this line is the documented reason it does).
    // --project: change working directory so file I/O is sandboxed to the project root.
    if let Some(ref proj) = project {
        if let Err(e) = env::set_current_dir(proj) {
            println!("Error: cannot change to project directory '{proj}': {e}");
            std::process::exit(1);
        }
        // Also expose the project's lib/ sub-directory for 'use' imports.
        lib_dirs.insert(
            0,
            std::path::Path::new(proj)
                .join("lib")
                .to_str()
                .unwrap()
                .to_string(),
        );
    }
    // Auto-detect loft.toml by walking up from the script file's directory.
    // This lets `loft lib/graphics/examples/01.loft` find the graphics package
    // without requiring the user to cd into the package directory first.
    if !abs_file.is_empty() {
        let script_dir = std::path::Path::new(&abs_file).parent();
        if let Some(mut search) = script_dir.map(std::path::Path::to_path_buf) {
            loop {
                let candidate = search.join("loft.toml");
                if candidate.exists() {
                    if let Some(manifest) =
                        manifest::read_manifest(candidate.to_str().unwrap_or("loft.toml"))
                    {
                        // Add the package's src/ directory to lib_dirs.
                        let entry = manifest.entry.unwrap_or_else(|| "src".to_string());
                        let src_dir = std::path::Path::new(&entry)
                            .parent()
                            .unwrap_or(std::path::Path::new("src"));
                        let abs_src = search.join(src_dir).to_string_lossy().to_string();
                        if !lib_dirs.contains(&abs_src) {
                            lib_dirs.push(abs_src);
                        }
                        // Add parent directory so sibling packages (deps) are found.
                        if !manifest.dependencies.is_empty() {
                            if let Some(parent) =
                                portable_path::try_plain_canonical(&search.join(".."))
                            {
                                let ps = parent.to_string_lossy().to_string();
                                if !lib_dirs.contains(&ps) {
                                    lib_dirs.push(ps);
                                }
                            }
                        }
                        // Register native lib path for auto-build/loading.
                        if let Some(ref stem) = manifest.native {
                            let pkg_dir = search.to_string_lossy().to_string();
                            if let Some(so_path) = extensions::auto_build_native(&pkg_dir, stem) {
                                native_lib_paths.push(so_path);
                            }
                        }
                    }
                    // Auto-add lib/ subdirectory for package imports.
                    let lib_dir = search.join("lib");
                    if lib_dir.is_dir() {
                        let ls = lib_dir.to_string_lossy().to_string();
                        if !lib_dirs.contains(&ls) {
                            lib_dirs.push(ls);
                        }
                    }
                    break;
                }
                if !search.pop() {
                    break;
                }
            }
        }
    }

    // Canonicalize library paths so relative --lib dirs resolve correctly
    // regardless of working directory changes during parsing.  Strip the
    // Windows verbatim prefix afterwards: everything downstream of
    // `lib_dirs` — `use` candidates, the package dirs recorded into
    // `pending_native_compile`, `def.position().file` prefix checks — must
    // share `abs_file`'s plain representation.  A verbatim entry here is
    // how the #460 entry-package skip (`entry_path.starts_with(pkg_dir)`)
    // silently missed on Windows: plain entry vs verbatim pkg_dir never
    // prefix-match, so the entry package auto-native-compiled after all.
    let lib_dirs: Vec<String> = lib_dirs
        .into_iter()
        .map(|d| portable_path::plain_canonical_str(&d))
        .collect();
    let mut p = parser::Parser::new();
    p.lib_dirs = lib_dirs;
    // @P363: join with a path separator (Path::join) instead of string
    // concat.  `project_dir()` returns a trailing-separator path, but
    // `--path <dir>` sets `dir` to the raw CLI argument with no trailing
    // separator — `dir + "default"` then yielded `<dir>default` and the
    // stdlib `default/` dir was never found.  A missing stdlib is a
    // recoverable CLI fault (wrong `--path`, not a corrupt install), so
    // emit a clean actionable diagnostic and exit non-zero rather than
    // unwrapping the NotFound into a panic.
    // Step 0 (startup-cache plan): env-gated parse timing (LOFT_TIMING).
    let t_parse_default = std::time::Instant::now();
    let default_dir = std::path::Path::new(&dir).join("default");
    let default_str = default_dir.to_string_lossy().to_string();
    // @PLN11 arc E / D2b / track 1 — the whole-program startup cache mmaps the
    // ENTIRE parsed program (stdlib + lazily-loaded libs + user file) on a
    // repeated unchanged run, skipping all parsing (~3–3.6× faster).  It is now
    // **default-on everywhere** (`cache::program_cache_enabled`): off only under
    // `LOFT_NO_CACHE`, the explicit slow path.  It used to switch itself off inside
    // Cargo and for any `target/` binary — which covered the whole test suite and every
    // compiler-development run — as a proxy for invalidation that was incomplete: the
    // program bundle folded in the binary's mtime and the STDLIB key did not.  Both do
    // now, so a rebuild invalidates on the fact itself (measured: `touch target/debug/loft`
    // makes the next run cold) and the proxy is gone.  The narrower stdlib cache
    // (`LOFT_STDLIB_CACHE`, D2b) caches `default/` only and engages just when the program
    // cache is off.
    // @PLN13 step 3 — AUTO-DETECT a beginner script (loose top-level statements, no
    // `fn main`) and desugar it to one run-once `fn main`, once, here; the parse below
    // uses this transformed source. `is_script` classifies every file the compiler
    // accepts (all-defs library / `fn main` program) as NOT a script, so this changes
    // nothing for existing programs — only a source loft rejects today becomes runnable.
    // `--script` remains an explicit request but is now redundant with auto-detect.
    // A desugared source (auto or `--script`) bypasses the whole-program cache, which is
    // keyed by the file on disk, not the transformed source.
    // T0.2 — keep the desugar's line map beside the generated source: the desugar
    // hoists defs and inserts lines, so a diagnostic carries GENERATED coordinates
    // until it is mapped back (which also restores the source snippet, since the
    // renderer then looks up a line the user's file actually has).
    let (script_desugared, script_line_map): (Option<String>, Option<Vec<u32>>) =
        if abs_file.is_empty() {
            (None, None)
        } else {
            match std::fs::read_to_string(&abs_file)
                .ok()
                .and_then(|src| loft::script::script_desugar_mapped(&src))
            {
                Some((out, map)) => (Some(out), Some(map)),
                None => (None, None),
            }
        };
    // `introspect` reports what the PARSER emits for this program, so it always parses: a
    // warm bundle carries no variable table, and the dump then rendered every variable as
    // `name(65535)` with `-` in the slot table — two runs of one binary read as two
    // compilers (measured 2026-09-05 on the released 2026.8.0, `LOFT_NO_CACHE=1` was the
    // only way to compare emissions).
    let program_cache_on = loft::cache::program_cache_enabled()
        && script_desugared.is_none()
        && !script_mode
        && !introspect_mode;
    p.track_sources = program_cache_on;
    // @PLN11 G2/M6 — on a warm hit with LOFT_CODEGEN_STORE, the cache is loaded
    // as a SKELETON (def table only) and the mmap'd bundle store is returned
    // here so codegen reads bodies straight from it (no read_data body rebuild).
    // @PLN86 — load the program's `[sandbox]` policy (loft.toml next to the file)
    // BEFORE the warm-load decision.  A whole-program warm load restores the IR
    // WITHOUT re-parsing, so the per-def designations (`def_sandbox`) would never
    // form and admission — AND the force-interpret guard — would be silently
    // skipped on every warm run; worse, the cache is keyed by program content, not
    // policy, so a TIGHTENED policy would be ignored.  A sandboxed program must
    // therefore always parse fresh: disable the warm-load when a policy is active.
    // The host owns this policy — a script cannot designate itself.
    if let Some(dir) = std::path::Path::new(&abs_file).parent()
        && let Ok(content) = std::fs::read_to_string(dir.join("loft.toml"))
    {
        p.set_sandbox_config(loft::sandbox::parse_sandbox_config(&content));
    }
    let mut warm_store: Option<(loft::database::Stores, loft::keys::DbRef)> = None;
    // #358 — a warm hit returns the def-table index where user definitions
    // start; the cold path derives it from the post-stdlib def count below.
    let warm_user_start = if program_cache_on && !p.sandbox_is_active() {
        loft::startup_cache::warm_load_program(&mut p, &abs_file, &mut warm_store)
    } else {
        None
    };
    let program_warm = warm_user_start.is_some();
    // A warm bundle REPLACES the parse.  When the user has armed a compiler
    // diagnostic, that silence is indistinguishable from "the code path never ran" —
    // it cost a full debugging session reading a stale parse while `eprintln`s in the
    // parser produced nothing.  So say it once, and name the way out.
    if program_warm && loft::cache::diagnostics_armed() {
        eprintln!(
            "loft: served a CACHED bundle for `{abs_file}` — the parser did not run, \
             so parser diagnostics stay silent (LOFT_NO_CACHE=1 re-parses)"
        );
    }
    if !program_warm {
        // When building a program cache, parse `default/` fresh so every stdlib
        // file lands in the program's drift manifest; otherwise use the D2b
        // stdlib cache.
        let stdlib_warm =
            !program_cache_on && loft::startup_cache::warm_load_stdlib(&mut p, &default_str);
        if !stdlib_warm {
            if let Err(e) = p.parse_dir(&default_str, true, false) {
                eprintln!(
                    "loft: cannot load standard library from `{}`: {e}",
                    default_dir.display()
                );
                eprintln!(
                    "  the `default/` library directory was not found under the \
                     compiler path.\n  Pass `--path <dir>` pointing at the directory \
                     that contains `default/`,\n  or run `loft` from an installed \
                     location where the stdlib is bundled."
                );
                std::process::exit(1);
            }
            if !program_cache_on {
                loft::startup_cache::save_stdlib_cache(&p, &default_str);
            }
        }
    }
    // #358 — on a warm load the def table already holds stdlib + user defs, so
    // `definitions()` would put `start_def` PAST the user fns and the no-`main`
    // test-fn fallback would silently execute nothing; use the persisted boundary.
    let start_def = warm_user_start.unwrap_or_else(|| p.data.definitions());
    if std::env::var("LOFT_TIMING").is_ok() {
        eprintln!(
            "LOFT_TIMING parse_default={:.2}ms ({start_def} defs)",
            t_parse_default.elapsed().as_secs_f64() * 1000.0
        );
    }
    // `--show-types --trace`: enable per-expression type recording
    // BEFORE parsing the user file (parse_dir on default/* already
    // ran without tracing — those are stdlib internals).
    if introspect_mode && introspect_trace {
        p.trace_types = true;
    }
    // @PLN11 arc E — a whole-program warm load already holds every definition;
    // skip parsing the user file (and its lib loads) entirely.  (The `[sandbox]`
    // policy was loaded above, before the warm-load gate, so designations form
    // during this parse — and a sandboxed program is never warm-loaded.)
    if !program_warm {
        // @PLN13 — parse the desugared script (auto-detected above), or for a normal
        // program parse the file unchanged.
        match &script_desugared {
            Some(src) => {
                p.parse_source(src, &abs_file, false);
            }
            None => {
                p.parse(&abs_file, false);
            }
        }
    }
    // T0.2 — put every diagnostic back into the user's line numbers before anything
    // reads or renders them.  Done here, right after the parse that produced them,
    // so no consumer downstream ever sees generated coordinates.
    if let Some(map) = script_line_map.as_deref() {
        p.diagnostics.remap_lines(&abs_file, map);
    }
    // loft#985 — the post-scope-check lint family lives in ONE place, so the program path
    // here and `loft test` run the same set; the error gate (loft#883) travels with it.
    loft::use_analysis::post_scope_lints(&p.data, &mut p.diagnostics, &abs_file);
    // @PLN24 arc B — the interpreter calls `#c` bindings for real now; what
    // remains gated is the ONE shape the contract does not cover.
    //
    // @PLN128 arc C — NOT gated on the backend any more. While this ran only
    // under `!native_mode`, `#c` was two languages: an over-ceiling binding
    // compiled and ran on `--native`, shipped, and failed for whoever
    // interpreted it — including `loft debug`, which IS the interpreter, so the
    // shapes you could not debug were exactly the ones you had no other way to
    // inspect. The ceiling moved to 32 (`MAX_C_ARITY`) so that unifying meant
    // raising the interpreter to meet rustc rather than narrowing what already
    // compiles.
    loft::use_analysis::c_binding_call_unsupported(&p.data, &mut p.diagnostics, &abs_file);
    // @PLN102 build step 2/3 — report-only link oracles (no-op unless LOFT_DUMP_LINK_SAFE/OBS).
    loft::use_analysis::dump_link_safety(&p.data);
    loft::use_analysis::dump_link_observability(&p.data);
    if !p.diagnostics.is_empty() {
        // @P282 fix: when `--no-warnings` is set, suppress
        // Warning-level diagnostics entirely so the program's
        // stdout stays free of warning preambles for piped
        // consumers (the loft-native scanner, viewer state
        // emission, anything machine-readable).  Errors still
        // print and still exit non-zero.
        let print_warnings = !no_warnings;
        let has_errors = p.diagnostics.level() >= Level::Error;
        if print_warnings || has_errors {
            let mode =
                loft::diagnostic_render::ErrorMode::from_cli_and_env(error_mode_arg.as_deref());
            match mode {
                loft::diagnostic_render::ErrorMode::Pretty => {
                    // @P282 — diagnostics (warnings + errors) go to STDERR,
                    // matching the rustc / clang convention.  This keeps the
                    // program's STDOUT free for piped consumers (the loft
                    // scanner, viewer state, any machine-readable output).
                    let loader = loft::diagnostic_render::FileSourceLoader::new();
                    if print_warnings {
                        let out = loft::diagnostic_render::render_pretty_all(
                            &p.diagnostics,
                            &loader,
                            loft::diagnostic_render::ColorMode::Auto,
                        );
                        eprint!("{out}");
                    } else {
                        // Errors-only: re-render entry-by-entry so we
                        // can skip Warning levels.  Mirrors render_pretty_all's
                        // shape minus the warning-cascade dedup (which is
                        // moot when no warnings are emitted).
                        for entry in p.diagnostics.entries() {
                            if entry.level >= Level::Error {
                                let s = loft::diagnostic_render::render_entry_pretty(
                                    entry,
                                    &loader,
                                    loft::diagnostic_render::ColorMode::Auto,
                                );
                                eprint!("{s}");
                                eprintln!();
                            }
                        }
                    }
                }
                loft::diagnostic_render::ErrorMode::Compact => {
                    for entry in p.diagnostics.entries() {
                        if entry.level == Level::Debug {
                            continue;
                        }
                        if !print_warnings && matches!(entry.level, Level::Warning | Level::Advice)
                        {
                            continue;
                        }
                        eprintln!("{}", entry.to_string_compact());
                    }
                }
            }
        }
        if p.diagnostics.level() >= Level::Error {
            std::process::exit(1);
        }
    }
    // @PLN86 F12 — `sandbox-check`: report the admission verdict and STOP, never
    // executing.  The whole point is a no-run "will this be allowed?" surface, so this
    // returns before any codegen/run path below.
    if sandbox_check_mode {
        let errors = p.sandbox_admission_errors();
        if errors.is_empty() {
            if p.has_sandboxed_defs() {
                println!("Admitted: no admission violations.");
                println!("{}", p.sandbox_complexity_report());
            } else {
                println!(
                    "Admitted: no sandboxed code is designated (check the [sandbox] \
                     policy in loft.toml)."
                );
            }
            std::process::exit(0);
        }
        eprintln!("Rejected: {} admission violation(s):", errors.len());
        for e in &errors {
            eprintln!("{e}");
        }
        std::process::exit(1);
    }
    // @PLN86 2.5 — admission: a program that designates sandboxed code is admitted
    // only if it is proven safe at LOAD (capabilities + totality + no-raw-write).
    // Reject with the actionable errors before it ever runs — the contract the modder
    // writes against; a clean compile is the guarantee.  This is BACKEND-AGNOSTIC: an
    // admitted script is total and fault-free on the interpreter AND on `--native`
    // (bounded loops, an acyclic call graph, partial ops total on both — div/mod-zero,
    // OOB, overflow all yield null natively too), so the host keeps its choice of
    // backend.  (A deployment that wants to forbid host-side `rustc` on mod-derived
    // input can opt in to interpret-only per profile; the cdylib-FFI surface is already
    // gated by `native_ffi`.)
    if p.has_sandboxed_defs() {
        let errors = p.sandbox_admission_errors();
        if !errors.is_empty() {
            eprintln!(
                "error: sandboxed code rejected — {} admission violation(s):",
                errors.len()
            );
            for e in &errors {
                eprintln!("{e}");
            }
            std::process::exit(1);
        }
    }
    scopes::check(&mut p.data, &mut p.database);
    // @PLN90 Step 5 — the user-facing copy report, emitted ONCE here (the whole program is now
    // loaded + checked) rather than per file-load. Gated on `--report-copies`; a no-op otherwise.
    loft::use_analysis::report_copies(&p.data);
    // @PLN130 F2 — views whose alias was taken away because their container is reshaped
    // while they are live.  The copy keeps the program correct; this keeps it honest.
    loft::copy_manifest::report_materialised_views();
    // @PLN11 Arc N / N3 (Step 2) — auto-compile `use`d libraries that opted in via
    // `[library] compile = "native"`, **build-before-mark**: build each library's
    // cdylib from the post-parse type schema FIRST, then mark its functions native
    // (so `byte_code` emits `OpStaticCall`) ONLY on success.  A build failure (or
    // `LOFT_FORCE_NATIVE_BUILD_FAIL`) leaves the library unmarked → it **silently
    // interprets** — byte-identical, no `exit`, no dispatch to an unbuilt symbol.
    // An auto-native program bypasses the program cache for now (artifact caching is
    // N1/Phase D).
    // `LOFT_NO_NATIVE_LIBS=1` forces ALL `use`d libraries to interpret — the parity
    // reference (a library run native ≡ run interpreted) and a manual escape hatch.
    let native_libs_off = std::env::var_os("LOFT_NO_NATIVE_LIBS").is_some();
    let force_build_fail = std::env::var_os("LOFT_FORCE_NATIVE_BUILD_FAIL").is_some();
    // Crawler / efficiency aid (`LOFT_REQUIRE_NATIVE=1`, the inverse of
    // `LOFT_NO_NATIVE_LIBS`): turn every native→interpreter fallback below into a
    // HARD ERROR that names the reason, so a performance run can never silently run
    // slow interpreted code.  Off by default — the normal warn-and-interpret
    // fallbacks are unchanged.  Enforced at two points (the only places native can
    // degrade to interpretation): the auto-native library loop just below, and the
    // main-program chokepoint past the `'native` block.
    let native_required = std::env::var("LOFT_REQUIRE_NATIVE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let mut pending_native = if native_libs_off {
        p.pending_native_compile.clear();
        Vec::new()
    } else {
        std::mem::take(&mut p.pending_native_compile)
    };
    // @PLN119 arc A — a library that declared `placement = "process"` runs in a
    // worker, so building it a cdylib here would compile code this process never
    // dispatches to. Drop it from the native candidates before that work starts;
    // its functions are marked for the placement route just below.
    let placed_libs: Vec<(String, String, loft::lib_placement::Placement)> =
        std::mem::take(&mut p.pending_placed_libs);
    pending_native.retain(|d| !placed_libs.iter().any(|(_, pkg, _)| pkg == d));
    let mut auto_native_libs: Vec<String> = Vec::new();
    let mut any_dev_interpret = false;
    // #460 — never auto-native-compile the package that OWNS the entry file: that
    // package is the *script* being run, not a `use`d library, and the model is
    // "libraries compile, scripts interpret".  Its export set is entry-point
    // dependent (only files reachable from THIS entry get parsed), so a cdylib
    // built for one entry (e.g. `selftest.loft` → exports `build_walls`) is wrong
    // for another (`equiptest.loft` → marks `player_new`): the freshness check
    // adopts the stale `.so`, then `mark_exports` marks a symbol it never built →
    // `OpStaticCall` to a missing bridge → the compile.rs panic stub.  Under
    // `--native` these functions compile into the whole-program binary anyway, so
    // excluding them from the cdylib loop costs nothing there.
    let entry_path = std::path::Path::new(&abs_file);
    for pkg_dir in &pending_native {
        if entry_path.starts_with(pkg_dir) {
            continue;
        }
        // #453 — for an `--html` build a `[wasm.bridge]` library builds its WASM
        // bridge (the `--html` branch below), not a native cdylib. Building the
        // native cdylib here is wasted work that FAILS for a browser-only bridge
        // lib: its `#native` symbols route through the bridge, so they have no
        // native implementation (P269). Skip it — the bridge build downstream uses
        // the routes that `register_native_manifest` registered. (Populated only
        // because the `[wasm.bridge]` registration there is now ungated by
        // `[native]`; the two halves of #453 are a pair.)
        if html_out.is_some()
            && p.data
                .wasm_bridge_packages
                .iter()
                .any(|(_, dir)| dir == pkg_dir)
        {
            continue;
        }
        // @PLN18 — `[native] in_binary = true`: the library's natives are
        // registered inside this binary (src/native.rs); a cdylib compile can
        // only fail (the symbols exist nowhere else).  Skip it silently.
        if manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
            .is_some_and(|m| m.native_in_binary)
        {
            continue;
        }
        let export = loft::native_lib::library_export_set(&p.data, pkg_dir);
        if export.is_empty() {
            continue;
        }
        let built = if force_build_fail {
            Err("LOFT_FORCE_NATIVE_BUILD_FAIL".to_string())
        } else {
            loft::native_lib::cached_or_build_shared_cdylib(
                &p.data,
                &p.database,
                &export,
                pkg_dir,
                &pending_native,
            )
        };
        match built {
            Ok(Some(so)) => {
                // loft#831 — the build succeeding does not mean this process can
                // dispatch through the artifact.  Load it and dlsym each bridge
                // BEFORE marking, so a symbol that will not resolve leaves its
                // function interpreting instead of compiling into an
                // `OpStaticCall` that panics at the first call.
                let probe = loft::native_lib::probe_and_mark_exports(&mut p.data, &export, &so);
                if probe.marked > 0 {
                    auto_native_libs.push(so.to_string_lossy().into_owned());
                }
                if !probe.complete() {
                    if native_required {
                        eprintln!(
                            "loft: LOFT_REQUIRE_NATIVE is set, but library '{pkg_dir}' built a \
                             cdylib this process cannot dispatch through ({}), so {} of its \
                             function(s) would run interpreted. Rebuild it with \
                             `make rebuild-native-cdylibs`, or unset LOFT_REQUIRE_NATIVE.",
                            if probe.not_loaded {
                                "it does not load"
                            } else {
                                "some bridge symbols are missing"
                            },
                            probe.unresolved.len(),
                        );
                        std::process::exit(1);
                    }
                    // One line for the library, not one per function: the program
                    // is CORRECT either way — this only costs speed.
                    let shown: Vec<&str> = probe
                        .unresolved
                        .iter()
                        .take(4)
                        .map(String::as_str)
                        .collect();
                    let more = probe.unresolved.len().saturating_sub(shown.len());
                    let more_txt = if more > 0 {
                        format!(", +{more} more")
                    } else {
                        String::new()
                    };
                    eprintln!(
                        "loft: library '{pkg_dir}' runs {n} function(s) interpreted this run — \
                         its cdylib built but {why} ({}{more_txt}). Results are unchanged, only \
                         slower; `make rebuild-native-cdylibs` restores native dispatch.",
                        shown.join(", "),
                        n = probe.unresolved.len(),
                        why = if probe.not_loaded {
                            "does not load"
                        } else {
                            "does not export their bridge symbols"
                        },
                    );
                }
            }
            Ok(None) => {
                // Dev-interpret-on-edit (Step 4): the library is being actively
                // edited, so it interprets this run — instant loop, no `rustc`, no
                // marking, no warning (this is the intended fast path while you iterate).
                // Do NOT cache the program: a warm load would replay this interpreted
                // image and the "rebuild once editing settles" check would never fire.
                if native_required {
                    eprintln!(
                        "loft: LOFT_REQUIRE_NATIVE is set, but library '{pkg_dir}' is being \
                         edited (dev-interpret) and would run interpreted, not native. \
                         Let its cdylib build (stop editing it), or unset LOFT_REQUIRE_NATIVE."
                    );
                    std::process::exit(1);
                }
                any_dev_interpret = true;
            }
            Err(e) => {
                // The fallback rule: interpreting a library instead of building it
                // native is graceful ONLY when native is impossible here — i.e. there
                // is no Rust toolchain.  When `rustc` IS installed, a failed native
                // build is a REAL failure: silently interpreting it would hand back a
                // partly-interpreted binary (or one whose `#native` functions panic at
                // runtime) while the caller asked for native.  So hard-fail when a
                // toolchain is present (and always under LOFT_REQUIRE_NATIVE); fall back
                // only when there is genuinely no toolchain to build with.
                if native_required || loft::native_lib::rustc_available() {
                    let why = if native_required {
                        "LOFT_REQUIRE_NATIVE is set"
                    } else {
                        "rustc is installed, so this is a real build failure, not a \
                         missing-toolchain fallback"
                    };
                    // Name the escape hatch that actually applies.  `--interpret`
                    // chooses the interpreter for the PROGRAM; a `use`d library
                    // still builds its cdylib, so advising it sent a blocked
                    // reader nowhere — the failing command already was
                    // `--interpret` (loft#815).  `LOFT_NO_NATIVE_LIBS=1` is the
                    // switch that makes every library interpret.
                    eprintln!(
                        "loft: library '{pkg_dir}' failed to build native ({e}).\n\
                         {why} — refusing to silently interpret it (that would hand back a \
                         partly-interpreted binary, or one whose #native functions panic \
                         when called).  Fix the library's native build, or set \
                         LOFT_NO_NATIVE_LIBS=1 to run every `use`d library interpreted on \
                         purpose (--interpret alone does not: it selects the interpreter \
                         for your program, while a library still builds its cdylib)."
                    );
                    std::process::exit(1);
                }
                eprintln!(
                    "loft: library '{pkg_dir}' has no native build and no Rust toolchain to \
                     build one ({e}); interpreting it.  Install rustc for a native build."
                );
            }
        }
    }
    // @PLN119 arc A — mark each process-placed library's routable functions
    // BEFORE `byte_code`, so their calls compile to `OpStaticCall` and get a
    // stub the worker dispatcher can take over. A function whose signature the
    // wire cannot carry yet is left unmarked and runs in-process, which is the
    // same silent, byte-identical fallback an uncompilable native library takes.
    //
    // Not under `--native`: that backend compiles the library's own body into
    // the whole-program binary, so its calls never reach a worker however they
    // were marked. Marking anyway would leave a dispatch symbol nothing routes
    // and start a worker process to sit idle for the run.
    #[cfg(target_os = "linux")]
    if !native_requested {
        for (_, pkg_dir, _) in &placed_libs {
            loft::lib_placement::dispatch::mark_exports(&mut p.data, pkg_dir);
        }
    }
    let has_auto_native = !auto_native_libs.is_empty();
    // @PLN11 G2 / M0 — equivalence harness.  With `LOFT_IR_CHECK` set, assert
    // the store-materialised IR is bit-for-bit identical to the native `Data`
    // before any subsystem is rewired to read from the store.  Opt-in (default
    // off): validates the store-mirror invariant on the *actual* program being
    // run — user code + lazily-loaded libs — not just the stdlib round-trip tests.
    if std::env::var_os("LOFT_IR_CHECK").is_some_and(|v| !v.is_empty()) {
        if let Err(diff) = loft::ir_read::ir_roundtrip_check(&p.data) {
            eprintln!("LOFT_IR_CHECK: store-backed IR diverges from native Data: {diff:?}");
            std::process::exit(1);
        }
        if std::env::var_os("LOFT_TIMING").is_some() {
            eprintln!(
                "LOFT_IR_CHECK: store round-trip == native ({} defs)",
                p.data.definitions()
            );
        }
    }
    // @PLN11 arc E — on a cold run with the program cache enabled, write the
    // whole-program bundle + drift manifest (post-`scopes::check`, so loaded
    // functions carry `done=true` and the baked free-ops).
    // Skip the program cache for auto-native programs: the warm-load path restores
    // the parsed `Data` without re-running manifest detection, so it would have the
    // `def.native` markings but no rebuilt cdylib to wire (Phase D persists this).
    // Also skip when a library took the dev-interpret-on-edit path (Step 4): caching
    // the interpreted image would pin it, so the "rebuild once editing settles" check
    // would never run on a warm load.
    if program_cache_on && !program_warm && !has_auto_native && !any_dev_interpret {
        loft::startup_cache::save_program(&p, &abs_file, start_def, &placed_libs);
    }
    // @PLAN28 debug/validation hook — when `LOFT_DUMP_SNAPSHOT=<path>` is set,
    // write the parsed `Data` as the startup-cache JSON snapshot and exit.
    // This is the manual validation path for the quick-loading work: dump the
    // post-parse image, then (later) load it back and diff.  The JSON is the
    // machine format (compact); format it externally if you need to eyeball.
    // No effect unless the env var is set.
    if let Ok(path) = std::env::var("LOFT_DUMP_SNAPSHOT") {
        let json = loft::ir_schema::data_to_json(&p.data);
        if let Err(e) = std::fs::write(&path, &json) {
            eprintln!("loft: failed to write snapshot to {path}: {e}");
            std::process::exit(1);
        }
        eprintln!(
            "loft: wrote startup-cache snapshot ({} bytes, {} defs) to {path}",
            json.len(),
            p.data.definitions()
        );
        std::process::exit(0);
    }
    if std::env::var_os("LOFT_DUMP_TYPES").is_some() {
        p.database.dump_types();
    }
    let mut state = State::new(p.database);
    // Set source_dir for the source_dir() built-in.
    state.database.source_dir = std::path::Path::new(&abs_file)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    // #255 / @PLN9: LOFT_PATHS overrides the path-resolution mode for this run
    // (`program` = re-home relative paths to the program's own directory;
    // `cwd` = the process cwd).  Unset → the parsed default / `#cwd` directive.
    if let Ok(mode) = std::env::var("LOFT_PATHS") {
        state.database.program_relative = mode.eq_ignore_ascii_case("program");
    }
    // store script-level arguments so arguments() returns only these.
    state.database.user_args.clone_from(&user_args);
    // @PLN11 G2/M6 — lower from the warm-loaded mmap store when present.
    // (The auto-native cdylibs were already built + their symbols marked above,
    // build-before-mark, so `byte_code` has emitted `OpStaticCall` only for the
    // libraries that compiled.)
    compile::byte_code_with_store(&mut state, &mut p.data, warm_store.as_ref());
    // @PLN119 arc A — a platform without the placement transport runs a placed
    // library in-process. By the plan's invariant that is the same PROGRAM, so
    // it is silent by default; but it is not the same ISOLATION, and a
    // deployment that asked for a worker to contain a crash should be able to
    // insist. `LOFT_REQUIRE_PLACEMENT=1` turns the quiet fallback into a refusal
    // — the same shape as `LOFT_REQUIRE_NATIVE` for native dispatch.
    //
    // Two things withdraw it, and a deployment that asked for a worker should
    // hear about either: a platform without the transport, and `--native`, whose
    // backend compiles the library into the whole-program binary so its calls
    // never leave the process.
    let no_placement_because = if placed_libs.is_empty() {
        None
    } else if cfg!(not(target_os = "linux")) {
        Some("out-of-process placement needs Linux")
    } else if native_requested {
        Some(
            "`--native` compiles a library's own body into the program binary, so its \
             calls do not cross a process boundary",
        )
    } else {
        None
    };
    if let Some(why) = no_placement_because
        && std::env::var("LOFT_REQUIRE_PLACEMENT").is_ok_and(|v| v == "1" || v == "true")
    {
        eprintln!(
            "loft: LOFT_REQUIRE_PLACEMENT is set, but {why}, so {} library/libraries that \
             declared `placement = \"process\"` would run in-process without isolation.",
            placed_libs.len()
        );
        std::process::exit(1);
    }
    // Start a worker for each process-placed library and point its marked
    // functions at it. After `byte_code`, because the stubs this replaces are
    // what `byte_code` registered — and only where marking happened, since a
    // worker with nothing routed to it is a process that idles for the run.
    #[cfg(target_os = "linux")]
    if !placed_libs.is_empty() && !native_requested {
        let stdlib = std::path::PathBuf::from(&default_str);
        // The directory the program will RUN in, which is not the one this
        // process is in yet: relative file access is anchored at `source_dir`
        // and the chdir for it happens much later (search `set_current_dir`).
        // A worker started now would inherit the INVOCATION directory instead,
        // and then every relative path a placed library touched would resolve
        // somewhere else than the same library in-process.
        let run_cwd = if state.database.program_relative && !state.database.source_dir.is_empty() {
            std::path::PathBuf::from(&state.database.source_dir)
        } else {
            std::path::PathBuf::new()
        };
        match loft::lib_placement::dispatch::install(
            &mut state,
            &p.data,
            &placed_libs,
            &stdlib,
            &run_cwd,
        ) {
            Ok(_) => {}
            // A placed library that will not start is fatal rather than a quiet
            // fall back to in-process: the declaration exists to get isolation,
            // and withdrawing it silently would leave no trace in any output.
            Err(why) => {
                eprintln!("loft: {why}");
                std::process::exit(1);
            }
        }
    }
    // load native extension shared libraries registered during parsing.
    // Also include any native libs discovered via loft.toml auto-detection.
    let mut all_native_libs = std::mem::take(&mut p.pending_native_libs);
    for nlp in &native_lib_paths {
        if !all_native_libs.contains(nlp) {
            all_native_libs.push(nlp.clone());
        }
    }
    all_native_libs.extend(auto_native_libs);
    extensions::load_all(&mut state, all_native_libs.clone());
    // PKG.5: wire auto-marshalled native functions from loaded cdylibs.
    extensions::wire_native_fns(&mut state, &p.data);
    // @PLN11 Arc N / N3 — wire the shared-store bridge dispatchers for the
    // auto-native libraries (the `loft_shared_*` symbols), a disjoint set from the
    // hand-written `#native` symbols `wire_native_fns` handles.
    extensions::wire_shared_native_fns(&mut state, &p.data);
    // loft#907: read back which Rust fn each library says implements its
    // `#native` symbols, so the native backend links the same one the
    // interpreter dispatches to.  Must run before any codegen below.
    extensions::resolve_native_impl_symbols(&mut p.data);

    // --check: parse + compile only, report errors and exit.
    // When combined with --native, fall through to the native pipeline
    // which will compile but not run the binary.
    if check_only && !native_mode && native_emit.is_none() {
        println!("ok");
        return;
    }

    // Android cross-compile pipeline: --native-android (@PLN106 B1+B2).
    // Emits the SAME target-agnostic Rust as --native / --native-wasm and hands
    // it to the Android descriptor (src/android.rs), which wraps it in a generated
    // NativeActivity crate (an `android_main` entry) and cross-builds a bionic
    // cdylib `.so` with cargo + the NDK toolchain.
    if let Some(ref android_out) = native_android {
        // Default artifact is a runnable, signed `.apk`; pass an explicit `*.so`
        // output to get just the NativeActivity library (needs only the NDK).
        let android_out = if android_out.is_empty() {
            default_artifact_path(&abs_file, "apk")
                .to_str()
                .unwrap_or("out.apk")
                .to_string()
        } else {
            android_out.clone()
        };
        let target = match android::AndroidTarget::detect() {
            Ok(t) => t,
            Err(msg) => {
                eprintln!("{msg}");
                std::process::exit(1);
            }
        };
        let end_def = p.data.definitions();
        // The generated `android_main` runs the program's `fn main`, so an Android
        // app needs one (unlike a `--native` test file, which can be entry-less).
        if p.data.def_nr("n_main") >= end_def {
            eprintln!(
                "loft: --native-android needs a `fn main` (it becomes the app's \
                 android_main entry); '{abs_file}' defines none."
            );
            std::process::exit(1);
        }
        // Per-process scratch so parallel --native-android runs never share the
        // generated source (one rustc reading another's program).
        let build_dir = platform::build_scratch_dir("android");
        let rs_path = build_dir.join("prog.rs");
        {
            let mut f = match std::fs::File::create(&rs_path) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!(
                        "loft: cannot write Android source to '{}': {e}",
                        rs_path.display()
                    );
                    std::process::exit(1);
                }
            };
            // @P379 — qualify native symbols for functions whose name collides
            // across libraries (no-op without a collision).
            p.data.namespace_colliding_native_fns();
            let mut out = generation::Output::new(&p.data, &state.database);
            // @PLN106 B3 — call native-package functions through the C-ABI marshalling
            // (loft_ffi types + `#[link_name]` decls) exactly as the host binary does;
            // the non-C-ABI extern-crate call path can't express their `loft_ffi`
            // signatures. Android still links the package as a unified rlib (an
            // `extern crate` prefix in src/android.rs force-links it), so the
            // `#[link_name]` decls resolve to the rlib's `#[no_mangle]` symbols.
            out.native_cabi = native_utils::native_cabi_enabled();
            // @PLN98 P2 — `--lean` strips the live/debug tier from the emitted Rust.
            // @PLN157 — and, separately, the frame NAMES; `out.lean` is the one
            // home for that second question (see `generation::Output::lean`).
            out.lean = lean;
            if lean {
                out.emit_live = false;
                out.lean_tier = true;
            }
            let main_nr = p.data.def_nr("n_main");
            let entry_defs: Vec<u32> = if main_nr < end_def {
                vec![main_nr]
            } else {
                (start_def..end_def).collect()
            };
            if let Err(e) = out.output_native_reachable(&mut f, start_def, end_def, &entry_defs) {
                eprintln!("loft: Android code generation failed: {e}");
                std::process::exit(1);
            }
        }
        let result = target.build(
            &rs_path,
            std::path::Path::new(&android_out),
            &p.data.native_packages,
        );
        if std::env::var("LOFT_KEEP_NATIVE_RS").is_err() {
            let _ = std::fs::remove_file(&rs_path);
        } else {
            eprintln!(
                "loft: Android source preserved at {} (LOFT_KEEP_NATIVE_RS)",
                rs_path.display()
            );
        }
        match result {
            Ok(()) => {
                let kind = if std::path::Path::new(&android_out)
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("apk"))
                {
                    "APK"
                } else {
                    "NativeActivity .so"
                };
                eprintln!("loft: wrote Android {kind} {android_out}");
                return;
            }
            Err(msg) => {
                eprintln!("{msg}");
                std::process::exit(1);
            }
        }
    }

    // WASM codegen pipeline: --native-wasm
    if let Some(ref wasm_out) = native_wasm {
        let wasm_out = if wasm_out.is_empty() {
            default_artifact_path(&abs_file, "wasm")
                .to_str()
                .unwrap_or("out.wasm")
                .to_string()
        } else {
            wasm_out.clone()
        };
        let wasm_out = &wasm_out;
        let end_def = p.data.definitions();
        // Per-process scratch so parallel `--native-wasm` runs never share the
        // generated source (a shared path let one rustc read another's program).
        let build_dir = platform::build_scratch_dir("wasm");
        let rs_path = build_dir.join("prog.rs");
        {
            let mut f = match std::fs::File::create(&rs_path) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!(
                        "loft: cannot write wasm source to '{}': {e}",
                        rs_path.display()
                    );
                    std::process::exit(1);
                }
            };
            // @P379 — qualify native symbols for functions whose name
            // collides across libraries (no-op without a collision; calls
            // resolve by d_nr so the renamed def stays consistent).
            p.data.namespace_colliding_native_fns();
            let mut out = generation::Output::new(&p.data, &state.database);
            // @PLN24 arc E — wasm32-wasip2.  It links a libc, so a `#c` binding
            // to a sysroot symbol used to build and then trap at the call; the
            // generator refuses the call instead.
            out.wasm_wasi = true;
            // @PLN98 P2 — `--lean` strips the live/debug tier from the emitted Rust.
            // @PLN157 — and, separately, the frame NAMES; `out.lean` is the one
            // home for that second question (see `generation::Output::lean`).
            out.lean = lean;
            if lean {
                out.emit_live = false;
                out.lean_tier = true;
            }
            let main_nr = p.data.def_nr("n_main");
            let entry_defs: Vec<u32> = if main_nr < end_def {
                vec![main_nr]
            } else {
                (start_def..end_def).collect()
            };
            if let Err(e) = out.output_native_reachable(&mut f, start_def, end_def, &entry_defs) {
                eprintln!("loft: wasm code generation failed: {e}");
                std::process::exit(1);
            }
        }
        let mut cmd = std::process::Command::new("rustc");
        cmd.arg("--edition=2024")
            .arg("--target")
            .arg("wasm32-wasip2")
            .arg("--crate-type")
            .arg("bin")
            .arg("-O")
            .arg("-o")
            .arg(wasm_out)
            .arg(&rs_path);
        // @PLN100 Slice 1 — build loft's own wasm runtime rlib on stale/missing and
        // locate it, instead of silently skipping the `--extern loft=…` (which used
        // to surface as an opaque "wasm compilation failed" when the rlib was absent).
        let wasm_deps_dir = if let Some(lib_dir) =
            native_utils::ensure_loft_runtime_rlib(native_utils::WasmRuntimeShape::Wasi)
        {
            cmd.args(loft::native_lib::loft_extern_args(
                &lib_dir.join("libloft.rlib"),
            ));
            let search = native_utils::dep_search_dirs(&lib_dir);
            for d in &search {
                cmd.arg("-L").arg(format!("dependency={}", d.display()));
            }
            search.first().cloned()
        } else {
            None
        };
        // PKG.5: add --extern flags for native packages (WASM target).
        native_utils::add_native_extern_flags(
            &mut cmd,
            &p.data,
            Some("wasm32-wasip2"),
            wasm_deps_dir.as_deref(),
        );
        let status = cmd.status();
        if std::env::var("LOFT_KEEP_NATIVE_RS").is_err() {
            let _ = std::fs::remove_file(&rs_path);
        } else {
            eprintln!(
                "loft: wasm source preserved at {} (LOFT_KEEP_NATIVE_RS)",
                rs_path.display()
            );
        }
        match status {
            Ok(s) if s.success() => {}
            Ok(_) => {
                eprintln!(
                    "loft: wasm compilation failed (try --native-emit to inspect the source)"
                );
                std::process::exit(1);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("loft: rustc not found; install the Rust toolchain to use --native-wasm");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("loft: failed to launch rustc: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    // --html — compile to browser WASM and assemble self-contained HTML.
    if let Some(ref html_path) = html_out {
        let html_path = if html_path.is_empty() {
            default_artifact_path(&abs_file, "html")
                .to_str()
                .unwrap_or("out.html")
                .to_string()
        } else {
            html_path.clone()
        };
        // @PLN146 F5 — the fonts this page has to carry, decided BEFORE the build:
        // a manifest that cannot produce a working page should not cost a wasm
        // compile first.  Two sources, because a main script's own package is never
        // resolved as a library — the entry program's nearest-ancestor `loft.toml`,
        // plus every `use`d package's declarations (collected by the parser).  This
        // refuses rather than reports: a family that drifts from the name the
        // program passes draws in a fallback with nothing on stderr, which is the
        // failure the declaration exists to remove.
        //
        // @PLN146 F4 reads the same two sources for `[[embed]]`: the files the page
        // carries in its own filesystem, so a program reads a pack with the same call
        // on the desktop and in a browser.
        let own_manifest = {
            let mut dir = std::path::Path::new(&abs_file)
                .parent()
                .map(std::path::Path::to_path_buf);
            let mut found = None;
            while let Some(d) = dir {
                let manifest = d.join("loft.toml");
                if manifest.exists() {
                    found = loft::manifest::read_manifest(&manifest.to_string_lossy());
                    break;
                }
                dir = d.parent().map(std::path::Path::to_path_buf);
            }
            found
        };
        let page_fonts = {
            let mut decls: Vec<loft::manifest::FontDecl> = own_manifest
                .as_ref()
                .map(|m| m.fonts.clone())
                .unwrap_or_default();
            for f in &p.data.declared_fonts {
                if !decls.contains(f) {
                    decls.push(f.clone());
                }
            }
            match loft::html_fonts::validate(&decls) {
                Ok(fonts) => fonts,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
        };
        let fonts_head = loft::html_fonts::head_html(&page_fonts);
        let fonts_await = loft::html_fonts::boot_await_js(&page_fonts);
        // @PLN146 F4 — the page's own filesystem, decided (and read) before the build
        // for F5's reason: a manifest naming a file that is not there should not cost
        // a wasm compile before it says so.
        let base_fs_js = {
            // The entry program's own declarations resolve against the PROGRAM, not
            // against its manifest.  `path` is the string the program passes, and loft
            // resolves what a program passes relative to the program file — so
            // `assets/game.pack` in `src/game.loft` is `src/assets/game.pack`, and
            // reading the manifest's own directory instead would embed a DIFFERENT
            // file under the key the program asks for.  Measured: with the source
            // rooted at the manifest, the desktop run answered `load=false` while the
            // page answered `load=true`, which is the divergence `[[embed]]` exists to
            // remove.  A library's declarations keep their own root — a library's file
            // is the library's to locate.
            let program_dir = std::path::Path::new(&abs_file)
                .parent()
                .map_or_else(String::new, |d| d.to_string_lossy().to_string());
            let mut decls: Vec<loft::manifest::EmbedDecl> = own_manifest
                .as_ref()
                .map(|m| m.embeds.clone())
                .unwrap_or_default();
            for d in &mut decls {
                d.root.clone_from(&program_dir);
            }
            for e in &p.data.declared_embeds {
                if !decls.contains(e) {
                    decls.push(e.clone());
                }
            }
            let files = loft::html_embed::validate(&decls).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            loft::html_embed::base_fs_js(&files).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            })
        };
        let end_def = p.data.definitions();
        // Per-process scratch dir for EVERY intermediate of this --html build
        // (generated .rs, the wasm output + its objects, bridge rlibs, wasm-opt
        // output).  One isolation point: routing all paths through `build_dir`
        // keeps concurrent `loft --html` runs (nextest, a parallel page build)
        // from racing on the old shared scratch/loft_html.{rs,wasm}.
        let build_dir = platform::build_scratch_dir("html");
        let rs_path = build_dir.join("prog.rs");
        // @PLN117 — does this program actually run anything in parallel?  Set
        // from the emitted reachable set below; it is what decides whether the
        // page carries loft's thread pool.
        let uses_par;
        {
            let mut f = match std::fs::File::create(&rs_path) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("loft: cannot write source to '{}': {e}", rs_path.display());
                    std::process::exit(1);
                }
            };
            // @P379 — qualify native symbols for functions whose name
            // collides across libraries (no-op without a collision; calls
            // resolve by d_nr so the renamed def stays consistent).
            p.data.namespace_colliding_native_fns();
            let mut out = generation::Output::new(&p.data, &state.database);
            out.wasm_browser = true;
            // @PLN98 P3.4 — a browser client is debug-OFF by default (a production
            // client should not ship a live-flip / breakpoint channel): the live
            // tier is opt-IN via `--debug[=name]`.  `--lean` also forces it off.
            // The debug name is baked so the client can announce itself to the
            // server, which then addresses debug frames to it over the relay.
            out.emit_live = debug_name.is_some() && !lean;
            // @PLN157 — frame NAMING is a separate question from the live tier, and
            // on this path they diverge: a production client is debug-OFF by default,
            // so `emit_live` is false without `--lean` ever being passed.  Keying the
            // nameless push off `!emit_live` stripped loft frame names from every
            // browser panic (`html_panic_names_itself_and_its_loft_frames`).
            out.lean = lean;
            out.debug_name.clone_from(&debug_name);
            // loft#954 — `--names` promises that a trap's frames resolve to loft
            // function names, which needs BOTH halves: the wasm name section (kept by
            // the wasm-opt flags below) and a function left to name.
            out.keep_fn_names = html_names;
            // Embed the program source so the debug client bootstraps the parked
            // interpreter from BYTES (no filesystem in a browser) — see P3.1.
            if out.emit_live {
                out.program_src = std::fs::read_to_string(&abs_file).ok();
            }
            let main_nr = p.data.def_nr("n_main");
            let entry_defs: Vec<u32> = if main_nr < end_def {
                vec![main_nr]
            } else {
                (start_def..end_def).collect()
            };
            if let Err(e) = out.output_native_reachable(&mut f, start_def, end_def, &entry_defs) {
                eprintln!("loft: html code generation failed: {e}");
                std::process::exit(1);
            }
            uses_par = out.uses_parallel();
        }
        // @PLN100 Slice 1 — build (on stale/missing) + locate loft's own wasm
        // runtime rlib in the ISOLATED `--html` shape dir (`target/loft/html/`), so
        // a wasm-bindgen `make wasm` build can't stomp it and no manual `make` step
        // is needed.  Computed once and reused for both the main link and each wasm
        // bridge crate (they must link the SAME loft copy).
        //
        // @PLN117 — a program that uses `par` gets the THREADED shape, whose
        // rlib is compiled together with an atomics std so `par` can run on Web
        // Workers.  `--threads` / `--no-threads` override the choice.  Threading
        // needs a nightly toolchain (only `-Z build-std` produces that std); when
        // it isn't there we say so and fall back to the single-threaded shape
        // rather than refusing to build a page at all — `par` then runs
        // sequentially, which is exactly what a non-isolated host does anyway.
        let want_threads = html_threads.unwrap_or(uses_par);
        let (html_runtime_dir, threaded) = {
            let shape = if want_threads {
                native_utils::WasmRuntimeShape::HtmlThreads
            } else {
                native_utils::WasmRuntimeShape::Html
            };
            match native_utils::ensure_loft_runtime_rlib(shape) {
                Some(dir) => (Some(dir), want_threads),
                None if want_threads => {
                    eprintln!(
                        "loft: --html: could not build the THREADED browser runtime — this page \
                         will run `par` sequentially on the main thread.\n{}",
                        native_utils::ATOMICS_STD_TOOLCHAIN_HINT
                    );
                    (
                        native_utils::ensure_loft_runtime_rlib(
                            native_utils::WasmRuntimeShape::Html,
                        ),
                        false,
                    )
                }
                None => (None, false),
            }
        };
        // The atomics std lives beside that rlib; this hands it to `rustc` as a
        // sysroot.  Missing it is a link error, never a quietly unthreaded page.
        let atomics_sysroot = threaded
            .then(|| {
                html_runtime_dir
                    .as_deref()
                    .and_then(native_utils::ensure_atomics_sysroot)
            })
            .flatten();
        // Compile to wasm32-unknown-unknown cdylib
        let wasm_path = build_dir.join("prog.wasm");
        // @PLN117 — an rlib only links with the rustc that built it, and the
        // threaded runtime + its atomics std come from nightly (only `-Z
        // build-std` can produce that std).  So the link runs on nightly too.
        let mut cmd = native_utils::wasm_rustc(atomics_sysroot.as_deref());
        cmd.arg("--edition=2024")
            .arg("--target")
            .arg("wasm32-unknown-unknown")
            .arg("--crate-type")
            .arg("cdylib")
            .arg("-O")
            .arg("-o")
            .arg(&wasm_path)
            .arg(&rs_path);
        if let Some(lib_dir) = html_runtime_dir.clone() {
            cmd.args(loft::native_lib::loft_extern_args(
                &lib_dir.join("libloft.rlib"),
            ));
            for d in native_utils::dep_search_dirs(&lib_dir) {
                cmd.arg("-L").arg(format!("dependency={}", d.display()));
            }
            // W1.1 env fix: libloft.rlib depends on wasm-bindgen, which pulls
            // in the proc-macro crate wasm_bindgen_macro.  Proc-macros are
            // always built for the host (never for wasm32), so rustc needs
            // the *host* deps directory on its search path in addition to
            // the target deps dir.  Without this, compilation fails with:
            //   error[E0463]: can't find crate for `wasm_bindgen_macro`
            // and subsequent errors cascade (every `use loft::...` fails,
            // so `cr_call_push` is reported unfound as a collateral).
            if let Some(host_lib_dir) = loft_lib_dir_for(None) {
                for d in native_utils::dep_search_dirs(&host_lib_dir) {
                    if d.is_dir() {
                        cmd.arg("-L").arg(format!("dependency={}", d.display()));
                    }
                }
            }
        }
        // lib_plan-29 W1c (2026-05-29): link each used library's wasm
        // bridge crate (declared via `[wasm.bridge]` in `loft.toml`).
        //
        // Build via rustc directly, not `cargo build`: cargo would
        // produce a SECOND copy of `loft` (with a different
        // StableCrateId than the top-level
        // `target/wasm32-unknown-unknown/release/libloft.rlib` the
        // standalone-binary `--extern loft=…` references), and rustc
        // would refuse with "expected DbRef, found DbRef" (two distinct
        // types from two builds of the same source).  By invoking
        // rustc with the SAME `--extern loft=…` + deps search path
        // the standalone build uses, the bridge rlib links against
        // exactly one copy of loft — eliminating the dup.
        let loft_wasm_lib_dir = html_runtime_dir;
        for (bridge_crate, pkg_dir) in &p.data.wasm_bridge_packages {
            let wasm_dir = std::path::PathBuf::from(pkg_dir).join("wasm");
            let bridge_src = wasm_dir.join("src/lib.rs");
            if !bridge_src.exists() {
                eprintln!(
                    "loft: --html: [wasm.bridge] declared `crate = \"{bridge_crate}\"` \
                     but {} is missing — skipping bridge link",
                    bridge_src.display()
                );
                continue;
            }
            let crate_ident = bridge_crate.replace('-', "_");
            let bridge_rlib = build_dir.join(format!("lib{crate_ident}.rlib"));
            // Build-extension (@PLN84 ZT-B): when the bridge crate declares Cargo
            // dependencies beyond `loft` (the vetted dalek/RustCrypto stack the SHARED
            // ed25519/x25519/aes modules use), build those deps for wasm32 to produce
            // their rlibs.  Each DIRECT dep is `--extern`'d on the bridge rustc compile
            // (the `use <crate>` extern-prelude entry — `-L` alone resolves only
            // TRANSITIVE deps), and the deps dir is `-L`'d on both the bridge compile
            // and the main wasm link (transitive symbols).  We consume ONLY the
            // dependency rlibs — the bridge itself is rustc-compiled below against the
            // SHARED prebuilt loft (no duplicate loft), and these deps are
            // loft-independent, so no "two copies of loft" (`expected DbRef, found
            // DbRef`) error arises.
            //
            // #446: we deliberately do NOT `cargo build` the bridge's own
            // `wasm/Cargo.toml`.  It carries a `loft = { path = "../../../loft" }` dep
            // that resolves only in a dev checkout — NOT for a registry-installed
            // package, where the relative path points at the nonexistent `~/.loft/loft`
            // and cargo aborts before building any dep.  That `loft` dep is pure
            // redundancy here: the bridge links the SHARED prebuilt loft via `--extern`
            // (below), never through this manifest.  So we synthesize a throwaway
            // deps-only crate whose `[dependencies]` are exactly the bridge's non-`loft`
            // deps and build THAT: cargo compiles every declared dep (even though the
            // empty lib uses none), producing the identical wasm32 rlibs without ever
            // resolving the manifest's `loft` path dep.
            let wasm_cargo = wasm_dir.join("Cargo.toml");
            // Every non-`loft` [dependencies] entry as (crate_ident, full TOML line).
            let nonloft_deps: Vec<(String, String)> = std::fs::read_to_string(&wasm_cargo)
                .map(|text| bridge_nonloft_deps(&text))
                .unwrap_or_default();
            let nonloft_idents: Vec<String> = nonloft_deps
                .iter()
                .map(|(ident, _)| ident.clone())
                .collect();
            let mut bridge_dep_search: Option<std::path::PathBuf> = None;
            let mut bridge_externs: Vec<(String, std::path::PathBuf)> = Vec::new();
            if !nonloft_deps.is_empty() {
                // Stage the deps-only crate under a per-bridge temp dir, then build it.
                let synth_dir = build_dir.join(format!("bridge_deps_{crate_ident}"));
                let synth_src = synth_dir.join("src");
                let manifest = synth_bridge_deps_manifest(&nonloft_deps);
                let staged = std::fs::create_dir_all(&synth_src).is_ok()
                    && std::fs::write(synth_dir.join("Cargo.toml"), &manifest).is_ok()
                    && std::fs::write(synth_src.join("lib.rs"), "").is_ok();
                if !staged {
                    eprintln!(
                        "loft: --html: failed to stage wasm-bridge dependency build for {bridge_crate}"
                    );
                    std::process::exit(1);
                }
                // loft#1238 — nothing here asks whether the deps are already built: the
                // staleness question is delegated wholesale to cargo, which is correct (it is
                // the tool that knows) but means the cost is invisible from loft's side.  Name
                // it in the report so a slow `--html` says which bridge is paying.
                let mut dep_build = std::process::Command::new("cargo");
                dep_build
                    .arg("build")
                    .arg("--release")
                    .arg("--target")
                    .arg("wasm32-unknown-unknown")
                    .arg("--manifest-path")
                    .arg(synth_dir.join("Cargo.toml"));
                let cargo_ok = loft::platform::timing_exec(
                    "cargo",
                    &format!("bridge deps ({crate_ident})"),
                    "no staleness check here — cargo decides what to rebuild",
                    || dep_build.status(),
                )
                .map(|s| s.success())
                .unwrap_or(false);
                if !cargo_ok {
                    eprintln!(
                        "loft: --html: failed to cargo-build wasm-bridge dependencies for {bridge_crate}"
                    );
                    std::process::exit(1);
                }
                let deps = synth_dir.join("target/wasm32-unknown-unknown/release/deps");
                if deps.is_dir() {
                    // Resolve each DIRECT dep to its `lib<ident>-<hash>.rlib` for `--extern`.
                    let files: Vec<String> = std::fs::read_dir(&deps)
                        .map(|rd| {
                            rd.flatten()
                                .map(|e| e.file_name().to_string_lossy().into_owned())
                                .collect()
                        })
                        .unwrap_or_default();
                    for ident in &nonloft_idents {
                        let prefix = format!("lib{ident}-");
                        if let Some(f) = files.iter().find(|f| {
                            f.starts_with(&prefix)
                                && std::path::Path::new(f.as_str())
                                    .extension()
                                    .is_some_and(|ext| ext.eq_ignore_ascii_case("rlib"))
                        }) {
                            bridge_externs.push((ident.clone(), deps.join(f)));
                        }
                    }
                    bridge_dep_search = Some(deps);
                }
            }
            // @PLN117 — a bridge links into the same wasm as loft's runtime, so it
            // gets the same compiler and the same std: an atomics rlib and a
            // non-atomics one do not link together.
            let mut build = native_utils::wasm_rustc(atomics_sysroot.as_deref());
            build
                .arg("--edition=2024")
                .arg("--target")
                .arg("wasm32-unknown-unknown")
                .arg("--crate-type")
                .arg("rlib")
                .arg("--crate-name")
                .arg(&crate_ident)
                .arg("-O")
                .arg("-o")
                .arg(&bridge_rlib)
                .arg(&bridge_src);
            if let Some(ref lib_dir) = loft_wasm_lib_dir {
                build.args(loft::native_lib::loft_extern_args(
                    &lib_dir.join("libloft.rlib"),
                ));
                for d in native_utils::dep_search_dirs(lib_dir) {
                    if d.is_dir() {
                        build.arg("-L").arg(format!("dependency={}", d.display()));
                    }
                }
                if let Some(host_lib_dir) = loft_lib_dir_for(None) {
                    for d in native_utils::dep_search_dirs(&host_lib_dir) {
                        if d.is_dir() {
                            build.arg("-L").arg(format!("dependency={}", d.display()));
                        }
                    }
                }
            }
            // Build-extension: the bridge crate's own Cargo deps (dalek/RustCrypto).
            // `--extern` each DIRECT dep (the `use <crate>` extern-prelude entry), then
            // `-L` the deps dir for their transitive deps.
            for (ident, rlib) in &bridge_externs {
                build
                    .arg("--extern")
                    .arg(format!("{ident}={}", rlib.display()));
            }
            if let Some(ref deps) = bridge_dep_search {
                build
                    .arg("-L")
                    .arg(format!("dependency={}", deps.display()));
            }
            let status = build.status();
            if !matches!(status, Ok(s) if s.success()) {
                eprintln!(
                    "loft: --html: failed to compile wasm bridge crate {} from {}",
                    bridge_crate,
                    bridge_src.display()
                );
                std::process::exit(1);
            }
            cmd.arg("--extern")
                .arg(format!("{crate_ident}={}", bridge_rlib.display()));
            // Build-extension: link the bridge crate's Cargo deps into the main wasm.
            if let Some(ref deps) = bridge_dep_search {
                cmd.arg("-L").arg(format!("dependency={}", deps.display()));
            }
        }
        let status = cmd.status();
        if std::env::var("LOFT_KEEP_NATIVE_RS").is_err() {
            let _ = std::fs::remove_file(&rs_path);
        } else {
            eprintln!(
                "loft: browser-wasm source preserved at {} (LOFT_KEEP_NATIVE_RS)",
                rs_path.display()
            );
        }
        match status {
            Ok(s) if s.success() => {}
            Ok(_) => {
                // loft#678 — rustc has just printed errors against `prog.rs`, a file the
                // consumer never wrote. Unattributed, that reads as a fault in their own
                // program and sends them auditing loft source that is not the cause. Say
                // whose code it is, and name the one shape that actually produces it: a
                // builtin whose implementation is absent on this target compiles
                // everywhere else and fails only here, as `no method named …` on a
                // runtime type (the working-set store loaders did exactly this until
                // they were bridged to the browser fetch).
                eprintln!(
                    "loft: browser WASM compilation failed.\n  \
                     The errors above are against loft-GENERATED Rust (`prog.rs`), not \
                     your .loft source — a location in it is not a location in your \
                     program.\n  \
                     If one reads `no method named …` on a loft runtime type, the \
                     builtin it names has no implementation on the browser target: the \
                     same program is expected to build on --native. Re-run with \
                     LOFT_KEEP_NATIVE_RS=1 to keep `prog.rs` and see the call in \
                     context, and please report the builtin — a builtin that --native \
                     accepts and --html cannot is a gap in loft, not in your code."
                );
                std::process::exit(1);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                eprintln!("loft: rustc not found; install the Rust toolchain to use --html");
                std::process::exit(1);
            }
            Err(e) => {
                eprintln!("loft: failed to launch rustc: {e}");
                std::process::exit(1);
            }
        }
        // wasm-opt: optimize size + enable asyncify for frame yield.
        // Asyncify lets loft_gl_swap_buffers suspend the WASM execution
        // so the browser can render the frame via requestAnimationFrame.
        let opt_path = build_dir.join("prog_opt.wasm");
        let mut wasm_opt = std::process::Command::new("wasm-opt");
        if threaded {
            // A threaded bundle uses atomics, shared memory and mutable globals;
            // without these wasm-opt rejects the module outright rather than
            // silently dropping anything.
            wasm_opt.args([
                "--enable-threads",
                "--enable-bulk-memory",
                "--enable-mutable-globals",
            ]);
        }
        // loft#954 — `--strip-debug` drops the `name` section along with the DWARF, so
        // every frame a browser prints for a trap is an index that resolves to nothing.
        // Under `--names` strip only the DWARF (which is the bulk: 1.5 MB against a
        // 100-byte name section on a toy module) and pass `-g`, because binaryen writes
        // the section out only when asked.  The names are the generated Rust symbols,
        // which carry the loft function name verbatim (`…_4prog21n_part_thumb_wire`).
        let (strip_flag, debuginfo_flags): (&str, &[&str]) = if html_names {
            ("--strip-dwarf", &["-g"])
        } else {
            ("--strip-debug", &[])
        };
        wasm_opt
            .args([
                // -O / -Oz plus --asyncify strips the host imports
                // (loft_gl.*, loft_io.*) entirely — wasm goes from 25
                // imports to 0 and every GL call runtime-panics as
                // "unreachable executed".  -O1 with the explicit
                // --asyncify pass keeps imports intact while still
                // producing a smaller, asyncify-ready bundle.
                "-O1",
                strip_flag,
                "--strip-producers",
                "--asyncify",
                // Asyncify suspend imports (comma-separated module.function):
                //   loft_gl.loft_gl_swap_buffers — the GL frame-yield (games).
                //   loft_web.ws_yield — the WebSocket frame-yield (@PLN84 ZT-C):
                //     a synchronous WS poll loop calls `web::yield_frame()` once
                //     per iteration; under --html that lowers to this import,
                //     which unwinds back to the JS event loop so
                //     `WebSocket.onmessage` can deliver, then resumes.  Naming
                //     it here even when no program yields on it is harmless (the
                //     import just isn't present in the wasm).
                //   loft_io.loft_host_http_get — the HTTP frame-yield (@PLN97):
                //     `store_load_url_trusted` calls this browser-only import,
                //     which unwinds to the event loop so `await fetch(url)` can
                //     complete, then resumes with the bytes — the synchronous
                //     loft API over an async fetch, without blocking the page.
                //   loft_io.loft_host_http_range — the same yield for ONE BYTE
                //     RANGE (loft#678): the working-set loaders
                //     (`store_load_key(s)` / `store_load_key_text` /
                //     `store_load_range`) fetch only the pages a lookup touches,
                //     so a phone can read a few map tiles out of a multi-GB block.
                //     It MUST be listed: a suspend import left out of this
                //     allowlist is not instrumented, so the unwind corrupts the
                //     stack instead of yielding.  Its companion
                //     `loft_host_http_range_total` is deliberately absent — it
                //     only reports what the completed fetch already learned.
                "--pass-arg=asyncify-imports@loft_gl.loft_gl_swap_buffers,loft_web.ws_yield,loft_io.loft_host_http_get,loft_io.loft_host_http_range",
            ])
            .args(debuginfo_flags)
            .arg("-o")
            .arg(&opt_path)
            .arg(&wasm_path);
        // loft#1238 — wasm-opt runs unconditionally: `--asyncify` is not an optimisation but
        // the pass that makes frame-yield work at all, so there is no staleness question and
        // nothing to skip.  Named in the report anyway, because a reader looking at a slow
        // `--html` needs to see the whole pipeline and not only the parts that can be cached.
        let final_wasm = if loft::platform::timing_exec(
            "wasm-opt",
            "prog_opt.wasm",
            "always runs — --asyncify is required for frame yield, not an optimisation",
            || wasm_opt.status(),
        )
        .is_ok_and(|s| s.success())
        {
            let _ = std::fs::remove_file(&wasm_path);
            opt_path
        } else {
            // @P337: a missing wasm-opt is NOT a cosmetic "larger output"
            // problem — without the `--asyncify` pass the bundle cannot
            // frame-yield, so the HTML driver runs loft_start() synchronously
            // and any render loop (`for _ in 0..N`) blocks the browser main
            // thread forever ("page times out").  Warn loudly so a hung
            // bundle is never shipped unknowingly.
            eprintln!(
                "WARNING: a required tool ('wasm-opt', from the 'binaryen' \
                 package) is not installed.\n  \
                 Without it this game page will FREEZE the browser tab \
                 (it locks up and never draws).\n  \
                 Install it and rebuild before publishing — e.g. `apt \
                 install binaryen`, or run `make doctor` for the command \
                 for your system."
            );
            wasm_path
        };
        // Assemble HTML
        let wasm_bytes = std::fs::read(&final_wasm).unwrap_or_default();
        let _ = std::fs::remove_file(&final_wasm);
        // loft#954 — `--names` is asked for precisely when a page has to be
        // debugged from its backtrace, so a build that silently produced no names
        // is the one failure that must not be quiet: the page looks identical and
        // its frames resolve to nothing, which is the state the flag exists to
        // leave.  Say so and name the tool, rather than let the next trap be
        // read as "the flag didn't help".
        if html_names && html_wasm_named_functions(&wasm_bytes).unwrap_or(0) == 0 {
            eprintln!(
                "loft: --names was requested but the emitted wasm carries no name \
                 section, so a browser backtrace will still show bare frame \
                 indices.\n  \
                 This needs a binaryen that honours `-g` (`wasm-opt --version`); \
                 without one, build without --names and bisect by hand."
            );
        }
        // @P350: self-validate the emitted wasm BEFORE writing the HTML so a
        // bare `loft --html` never ships the silently-broken "rlib stomp"
        // bundle.  `make wasm` (wasm-pack, feature=wasm) and `--html` write
        // the SAME target/wasm32-unknown-unknown/release/libloft.rlib with
        // incompatible feature sets; if --html links the wasm-bindgen variant
        // the wasm imports `__wbindgen_placeholder__` (35+), which the
        // embedded loft-gl-wasm.js glue (raw loft_gl/loft_io externs only)
        // can't provide → the page fails to instantiate.  A correct --html
        // bundle imports ONLY `loft_gl` + `loft_io`.  Same check as
        // tools/check_html_bundle.mjs, but inline so it guards the bare
        // command, not just `make game`.  (The asyncify/wasm-opt footgun is
        // handled by the loud warning above — it's conditional on whether the
        // program frame-yields, so it must not hard-abort compute-only bundles.)
        if let Err(bad_mods) = html_wasm_import_modules_ok(&wasm_bytes) {
            eprintln!(
                "loft: --html produced a BROKEN bundle — the wasm imports \
                 unexpected module(s) {bad_mods:?}.\n  \
                 The wasm32-unknown-unknown libloft.rlib was built with the \
                 `wasm` (wasm-bindgen) feature — most likely a prior `make \
                 wasm` stomped it (see WASM.md § The rlib-stomp hazard).\n  \
                 Rebuild the rlib in the --html shape, then re-run --html:\n    \
                 cargo build --release --target wasm32-unknown-unknown --lib \
                 --no-default-features --features random\n  \
                 (No HTML was written — a stomped bundle does not instantiate \
                 in the browser.)"
            );
            std::process::exit(1);
        }
        let wasm_b64 = crate::base64::encode(&wasm_bytes);
        let title = std::path::Path::new(&file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "Loft Program".to_string());
        // @P321(c) Phase 3a: auto-discover *.png siblings of the entry .loft
        // and embed each as a base64 string under `ctrl.assets[basename]`.
        // Phase 3b's JS preamble decodes them to RGB bytes before
        // `loft_start()` runs; the imaging bridge then looks up by basename.
        // Stays a no-op when no PNGs are adjacent (most --html programs).
        let assets_js = {
            let entry_dir = std::path::Path::new(&abs_file)
                .parent()
                .map(std::path::Path::to_path_buf)
                .unwrap_or_else(|| std::path::PathBuf::from("."));
            let mut entries: Vec<(String, String)> = Vec::new();
            if let Ok(rd) = std::fs::read_dir(&entry_dir) {
                let mut pngs: Vec<std::path::PathBuf> = rd
                    .filter_map(Result::ok)
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("png")))
                    .collect();
                pngs.sort();
                for p in pngs {
                    let Some(name) = p.file_name().and_then(|s| s.to_str()) else {
                        continue;
                    };
                    let Ok(bytes) = std::fs::read(&p) else {
                        continue;
                    };
                    entries.push((name.to_string(), crate::base64::encode(&bytes)));
                }
            }
            if entries.is_empty() {
                String::from("{}")
            } else {
                let mut s = String::from("{");
                for (i, (name, b64)) in entries.iter().enumerate() {
                    if i > 0 {
                        s.push(',');
                    }
                    // Asset names are filesystem basenames — restrict to
                    // safe chars; reject anything that could break out of
                    // the JS string literal.  PNG suffix already required.
                    let safe = name
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-');
                    if !safe {
                        continue;
                    }
                    s.push_str(&format!("\"{name}\":\"{b64}\""));
                }
                s.push('}');
                s
            }
        };
        // The asyncify async→sync bridge (AsyncifyCtrl), shared by the GL and the
        // headless templates.  gl_js references it, so it is emitted FIRST.
        let env_js = include_str!("../doc/loft-env.js");
        let asyncify_js =
            include_str!("../doc/loft-asyncify.js").replace("export { AsyncifyCtrl };", "");
        // loft#851 — the page's filesystem.  Emitted BEFORE gl_js, whose
        // `loft_io` block spreads `loftFSImports(getMem)` into its handlers, and
        // used directly by the minimal shell below.  Module `export` stripped
        // like the asyncify and deliver glue, so a page stays a single file.
        let fs_js = include_str!("../doc/loft-fs.js")
            .replace("export { LoftPageFS, loftFS, loftFSImports };", "");
        let gl_js = include_str!("../doc/loft-gl-wasm.js");
        // @PLN105 Phase 2/3 — the generic deliver reader, embedded so both page shells reconstruct a
        // JS value from a `deliver`/`expose` handle. Strip the trailing `export` (the file is a
        // module for the node harness; here it is inlined into a non-module `<script>`).
        let reader_js =
            include_str!("../doc/loft-deliver.js").replace("export { readLoftValue };", "");
        // @PLN117 — the browser thread pool's host half, and with it `loftInstantiate`:
        // the ONE way a loft page comes up, threaded or not.  Inlined into both shells
        // (module `export` stripped, as with the asyncify and deliver glue) so a page
        // stays a single file.
        let thread_js = include_str!("../doc/loft-thread.js").replace(
            "export { startLoftWorkers, loftInstantiate, loftTextDecoder, loftSharedMemory, loftMemoryImportLimits };",
            "",
        );
        // @lib_plan-29 W2: concatenate every used library's
        // `[wasm.bridge].host_js` file into the HTML preamble.  Each
        // file pushes a registration callback onto
        // `globalThis.LOFT_WASM_EXTENSIONS`; the dispatch loop below
        // applies each callback to the imports object after
        // `buildLoftImports` returns, so library-specific JS handlers
        // (e.g. `imaging_query`) become part of the wasm imports
        // without the compiler/tooling crate naming them.
        let host_js_extensions = {
            let mut s = String::new();
            for path in &p.data.wasm_bridge_host_js_files {
                match std::fs::read_to_string(path) {
                    Ok(content) => {
                        s.push_str("\n/* === lib_plan-29 W2: host.js from ");
                        s.push_str(path);
                        s.push_str(" === */\n");
                        s.push_str(&content);
                    }
                    Err(e) => {
                        eprintln!(
                            "loft: --html: cannot read [wasm.bridge].host_js file '{path}': {e}"
                        );
                        std::process::exit(1);
                    }
                }
            }
            s
        };
        // Pick the page shell by what the wasm actually imports — minimal by
        // default, bigger only when the program uses it.  A wasm that imports
        // only `loft_io` is pure text I/O, so it ships the tiny engine-less
        // shim (no WebGL2, no asyncify, no canvas).  `loft_gl` (graphics/audio)
        // or a `loft_<lib>` bridge means the program opted into the full engine
        // page.  Unparsable → full page (the shell that satisfies every import).
        // Threading is orthogonal to which shell a page needs — `loft_thread` and
        // the imported `env.memory` say nothing about graphics — so they are not
        // counted here.  Otherwise a compute-only program that uses `par` would
        // start shipping the full WebGL2 page.
        // loft#709: what a call that this target cannot serve does at RUNTIME.
        // Placed after the shim and every library bridge have had their say, so
        // it fills only names still free.
        // loft#1059 — what a page does when the module TRAPS.
        //
        // A trap is not a Rust panic, so the hook loft#950 installs never runs and
        // the page's only symptom is a thrown `RuntimeError`. Deep recursion is the
        // shape that reaches it: the engine's stack dies well before loft's own
        // frame cap can report, and one shell used to print the bare exception
        // while the other had no catch at all and lost it entirely.
        //
        // Nothing here calls back into the module for loft's own frames. A trap
        // leaves the shadow-stack pointer wherever it died — the epilogues that
        // would restore it never ran — so a second entry would run off the end of
        // the stack it just exhausted. The browser's OWN wasm backtrace rides on
        // the error and is the evidence to read; `--names` (loft#954) is what
        // makes its frame numbers resolve to loft function names.
        let trap_js = r#"
function loftReportTrap(e){
  const msg=(e&&e.message)?e.message:String(e);
  let out="\n[loft] "+msg;
  if(/call stack exhausted|Maximum call stack|stack overflow/i.test(msg)){
    out+="\n[loft] the stack ran out before loft's own frame cap could report it."
        +"\n[loft] That bound belongs to this wasm engine, not to loft — the same"
        +"\n[loft] program halts with a loft diagnostic on --interpret and --native.";
  }
  if(e&&e.stack)out+="\n"+e.stack;
  out+="\n[loft] rebuild with `loft --html --names` to resolve the frame numbers above.";
  try{console.error(out);}catch(_){}
  try{const o=document.getElementById('out');if(o)o.textContent+=out;}catch(_){}
}
"#;
        let stub_js = crate::native_utils::host_import_stub_js(&wasm_bytes);
        let minimal_page =
            crate::native_utils::html_wasm_import_modules(&wasm_bytes).is_some_and(|mods| {
                mods.iter()
                    .filter(|m| *m != "loft_thread" && *m != "env")
                    .all(|m| m == "loft_io")
            });
        // The host surface is decidable HERE: the program's imports are in `wasm_bytes`
        // and the page's JS is fully assembled above.  Without this the boundary was
        // invisible until the page loaded, and crossing it killed the whole page — a
        // `LinkError` naming an import INDEX, no canvas, no `println`, and nothing
        // pointing at the loft call responsible (loft#668).  Checked only for the full
        // engine page: the minimal shell is chosen because the program imports `loft_io`
        // alone, which that shell defines in full.
        if !minimal_page {
            let provided = format!("{gl_js}{fs_js}{host_js_extensions}{thread_js}");
            let missing = crate::native_utils::missing_host_imports(&wasm_bytes, &provided);
            if !missing.is_empty() {
                // loft#681 — the check assumes the page it is about to write is the one
                // that will run the module.  A consumer that extracts the wasm and drives
                // it from its own JS has already broken that assumption on purpose, and
                // for them a missing import in loft's shim says nothing about whether
                // their host provides it.  Report, but do not stand in the way.
                let verb = if missing.len() == 1 {
                    "a function"
                } else {
                    "functions"
                };
                let list = missing.join("\n    ");
                if html_host_provided {
                    eprintln!(
                        "loft: --html: {} not provided by loft's page shim:\n    {}\n  \
                         Building anyway (--host-provided): your host supplies the imports. \
                         The emitted wasm is unchanged — only this check is relaxed, so a \
                         name your host does NOT define still fails at instantiate.",
                        verb, list,
                    );
                } else {
                    // loft#709 — REPORT, do not refuse.  Whether a call can be
                    // served is a fact about this run on this target, not about
                    // whether the program is well-formed, so it belongs at
                    // runtime: the page carries a stub per unserviceable name
                    // (`host_import_stub_js`) that returns the declared zero and
                    // says so in the console.  Refusing made one source fork
                    // into two entry points differing only in which calls they
                    // may NAME, which is exactly what two renderers exist to
                    // avoid.  The diagnosis stays — it is the disposition that
                    // was wrong.
                    eprintln!(
                        "loft: --html: this program calls {verb} the browser host does not \
                         provide:\n    {list}\n  \
                         The browser shim implements a SUBSET of the native surface — a canvas \
                         cannot do everything a desktop window can.\n  \
                         The page still builds and runs: each of these returns its zero value \
                         (false / 0) and reports itself once in the browser console, so check \
                         the result as you would any other failure.\n  \
                         To serve one for real, add a handler to doc/loft-gl-wasm.js (or your \
                         library's [wasm.bridge] host_js)."
                    );
                }
            }
        }
        let html = if minimal_page {
            format!(
                r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{title}</title>
<style>body{{margin:0;font:14px/1.5 monospace;background:#111;color:#0f0}}pre{{margin:0;padding:1rem;white-space:pre-wrap;word-break:break-word}}</style>
</head><body><pre id="out"></pre>
<script>
{asyncify_js}
{env_js}
{reader_js}
{thread_js}
{fs_js}
{base_fs_js}// Minimal engine-less loft page: a small wasm + this tiny shim.  No WebGL2, no
// canvas — only `loft_io` (text out + the async `store_load_url_trusted` fetch).
// Asyncify IS driven here (via AsyncifyCtrl above) so a synchronous loft call can
// suspend for an async `fetch()` without freezing the page.  JS owns the page;
// loft is a callable module (loft_start builds fresh Stores each call, so JS can
// invoke it per request).  A program that uses graphics/audio/a frame loop gets
// the full engine page instead.
const wasmB64="{wasm_b64}";
const wasmBytes=Uint8Array.from(atob(wasmB64),c=>c.charCodeAt(0));
const out=document.getElementById('out');
const dec=loftTextDecoder();   // @PLN117: also reads a threaded page's SHARED memory
let mem;
// JS -> loft input is a QUEUE: seed it with globalThis.loftInput (a string)
// before this runs, push live messages any time with globalThis.loftPush(msg)
// (e.g. fetch() completions) — each host_input() call pops one message
// (len+copy pairs; the copy pops).  loft -> JS structured messages arrive at
// globalThis.loftOutput(msg) (host_output(); default: console.log) — the
// request/response pattern: loft host_output's a request, JS acts on it and
// loftPush'es the completion.
const enc=new TextEncoder();
const inQ=[];
if(globalThis.loftInput!=null)inQ.push(enc.encode(String(globalThis.loftInput)));
globalThis.loftPush=(m)=>{{inQ.push(enc.encode(String(m)));}};
// @PLN97: asyncify controller (Step 2 sets `.ac`) + the raw bytes of the last
// fetch, stashed between the unwind and rewind halves of loft_host_http_get.
const ctrl={{ac:null,httpBytes:null,httpTotal:-1}};
const imports={{loft_io:{{
  // loft#851 — the page's filesystem (loft-fs.js, inlined above).  `mem` is
  // re-read per call because growing the wasm heap detaches the old buffer.
  ...loftFSImports(()=>mem),
  loft_host_print:(ptr,len)=>{{out.textContent+=dec.decode(new Uint8Array(mem.buffer,ptr,len));}},
  // #620: the browser CLOCK bridge.  This target has no std clock, so without
  // these `now()`/`ticks()` returned a hardcoded 0 — every duration measured
  // 0ms silently.  `performance.now()` is monotonic and page-relative, which is
  // exactly `ticks()`'s contract.
  loft_host_time_now_ms:()=>Date.now(),
  loft_host_time_ticks_us:()=>performance.now()*1000,
  loft_host_input_len:()=>inQ.length?inQ[0].length:0,
  loft_host_input_copy:(ptr)=>{{const b=inQ.shift();if(b)new Uint8Array(mem.buffer,ptr,b.length).set(b);}},
  loft_host_output:(ptr,len)=>{{const m=dec.decode(new Uint8Array(mem.buffer,ptr,len));
    if(globalThis.loftOutput)globalThis.loftOutput(m);else console.log("[loft:out]",m);}},
  // @PLN97 store_load_url_trusted: async fetch() bridged to a SYNCHRONOUS loft
  // call via asyncify.  This suspend import is invoked TWICE per fetch:
  //  (1) NORMAL state — first call: start fetch(url), then ac.suspend() unwinds
  //      the whole wasm stack back to the JS event loop (return value ignored).
  //  (2) REWINDING state (===2) — after the fetch resolved and resume() replayed
  //      the stack to this yield: ac.suspend() stop_rewinds, and we RETURN the
  //      byte length (or 0xFFFFFFFF on error → net::fetch_bytes maps it to Err).
  // The bytes are copied out separately by loft_host_http_get_copy.  See
  // plans/97-layout-contract/WASM_STORE_LOAD_URL.md.
  loft_host_http_get:(ptr,len)=>{{
    if(ctrl.ac&&ctrl.ac.exports.asyncify_get_state()===2){{
      ctrl.ac.suspend();
      return ctrl.httpBytes?ctrl.httpBytes.length:0xFFFFFFFF;
    }}
    const url=dec.decode(new Uint8Array(mem.buffer,ptr,len));
    ctrl.httpBytes=null;
    fetch(url).then(async r=>{{ctrl.httpBytes=r.ok?new Uint8Array(await r.arrayBuffer()):null;ctrl.ac.resume('loft_start');}})
              .catch(()=>{{ctrl.httpBytes=null;ctrl.ac.resume('loft_start');}});
    if(ctrl.ac)ctrl.ac.suspend();
    return 0;
  }},
  loft_host_http_get_copy:(ptr)=>{{if(ctrl.httpBytes)new Uint8Array(mem.buffer,ptr,ctrl.httpBytes.length).set(ctrl.httpBytes);}},
  // loft#678 working-set loaders: the same two-phase asyncify bridge as
  // loft_host_http_get, but for ONE BYTE RANGE — `Range: bytes=off-(off+len-1)`.
  // The response also carries the resource's total size in `Content-Range:
  // bytes a-b/TOTAL`; stash it so loft_host_http_range_total can answer
  // PageProvider::size() without a second round trip.  `off`/`len` arrive as
  // plain JS numbers (the import declares f64 — exact below 2^53) so no BigInt
  // conversion is needed here or in the headless stubs.
  loft_host_http_range:(ptr,len,off,n)=>{{
    if(ctrl.ac&&ctrl.ac.exports.asyncify_get_state()===2){{
      ctrl.ac.suspend();
      return ctrl.httpBytes?ctrl.httpBytes.length:0xFFFFFFFF;
    }}
    const url=dec.decode(new Uint8Array(mem.buffer,ptr,len));
    ctrl.httpBytes=null;ctrl.httpTotal=-1;
    const last=off+n-1;
    fetch(url,{{headers:{{Range:`bytes=${{off}}-${{last}}`}}}}).then(async r=>{{
      // 206 = the body IS the range.  200 = the server ignored Range and sent the
      // whole file; slice out the window so the answer is right either way.
      const cr=r.headers.get('Content-Range');
      if(cr){{const t=cr.split('/').pop();ctrl.httpTotal=(t&&t!=='*')?Number(t):-1;}}
      else{{const cl=r.headers.get('Content-Length');ctrl.httpTotal=cl?Number(cl):-1;}}
      if(!r.ok){{ctrl.httpBytes=null;}}
      else{{const b=new Uint8Array(await r.arrayBuffer());
            ctrl.httpBytes=(r.status===206)?b:b.subarray(off,off+n);}}
      ctrl.ac.resume('loft_start');
    }}).catch(()=>{{ctrl.httpBytes=null;ctrl.ac.resume('loft_start');}});
    if(ctrl.ac)ctrl.ac.suspend();
    return 0;
  }},
  loft_host_http_range_total:()=>ctrl.httpTotal,
  // @PLN105 Phase 2 — deliver: reconstruct a live value from its raw linear-memory address + layout
  // descriptor (JSON) via the embedded readLoftValue (reader_js, inlined above), then hand the
  // finished value to globalThis.loftDeliver(tag, value, type_id). SYNCHRONOUS: read within this
  // call — the borrow ends on return.
  loft_host_deliver:(tag,store_base,rec,pos,type_id,dptr,dlen)=>{{
    const desc=JSON.parse(dec.decode(new Uint8Array(mem.buffer,dptr,dlen)));
    const value=readLoftValue(mem,store_base,desc,type_id,rec,pos);
    if(globalThis.loftDeliver)globalThis.loftDeliver(tag,value,type_id);
    else console.log("[loft:deliver]",tag,value);
  }},
  // @PLN105 Phase 3 — expose/release: a long-lived deliver. Stash a RE-READER closure by tag (loft
  // pins the store so its addresses stay valid across frames); a page calls
  // globalThis.loftExposed.get(String(tag))() each frame for a fresh value (re-derives the view —
  // memory.grow-safe). release drops the stash.
  loft_host_expose:(tag,store_base,rec,pos,type_id,dptr,dlen)=>{{
    const desc=JSON.parse(dec.decode(new Uint8Array(mem.buffer,dptr,dlen)));
    const reread=()=>readLoftValue(mem,store_base,desc,type_id,rec,pos);
    (globalThis.loftExposed||(globalThis.loftExposed=new Map())).set(String(tag),reread);
    if(globalThis.loftExpose)globalThis.loftExpose(tag,reread,type_id);
  }},
  loft_host_release:(tag)=>{{ if(globalThis.loftExposed)globalThis.loftExposed.delete(String(tag)); if(globalThis.loftRelease)globalThis.loftRelease(tag); }}
}}}};
{trap_js}
{stub_js}
// @PLN117 — one boot path: loftInstantiate threads the page when the wasm was
// built for it AND the host is cross-origin isolated, and otherwise brings it up
// exactly as before (par then runs sequentially, same results).
loftInstantiate(wasmBytes,imports).then(({{instance,memory}})=>{{
  mem=memory||instance.exports.memory;
  loftInstallEnv(instance, mem);
  // If the wasm was asyncify-instrumented (wasm-opt --asyncify present), drive it
  // through AsyncifyCtrl so store_load_url_trusted can suspend for an async
  // fetch().  Progress after the first suspend is EVENT-driven: each
  // loft_host_http_get .then() calls ctrl.ac.resume('loft_start') when its
  // response arrives (no render pump needed — a headless page has no rAF loop).
  if(instance.exports.asyncify_start_unwind){{
    ctrl.ac=new AsyncifyCtrl(instance);
    ctrl.ac.start('loft_start');
  }}else{{
    instance.exports.loft_start();
  }}
}}).catch(loftReportTrap);
</script></body></html>"#
            )
        } else {
            format!(
                r#"<!DOCTYPE html>
<html><head><meta charset="utf-8"><title>{title}</title>
<style>body{{margin:0;background:#000;display:flex;justify-content:center;align-items:center;height:100vh}}canvas{{display:block}}pre{{color:#0f0;font-size:14px}}</style>{fonts_head}
</head><body>
<canvas id="c" tabindex="0" style="display:none"></canvas>
<pre id="out"></pre>
<script>
{asyncify_js}
{env_js}
{reader_js}
{thread_js}
{fs_js}
{base_fs_js}{gl_js}
{host_js_extensions}
const wasmB64="{wasm_b64}";
const wasmBytes=Uint8Array.from(atob(wasmB64),c=>c.charCodeAt(0));
const canvas=document.getElementById('c');
const output=document.getElementById('out');
let mem;
// @P321(c) Phase 3a: raw base64 of *.png siblings of the entry .loft
// (auto-discovered).  Phase 3b decodes each to RGB bytes and replaces
// the slot with {{width, height, bytes}} before loft_start runs.
const ctrl={{ac:null,assets:{assets_js}}};
const imports=buildLoftImports(canvas,output,()=>mem,ctrl);
// @lib_plan-29 W2: apply each library's host.js-registered extension
// to the imports object (mutates `imports.loft_gl` in place).
for(const reg of (globalThis.LOFT_WASM_EXTENSIONS||[])){{
  try{{reg(imports,ctrl,()=>mem);}}catch(e){{console.error('loft host_js extension failed',e);}}
}}
{trap_js}
{stub_js}
// @PLN117 — one boot path: loftInstantiate threads the page when the wasm was
// built for it AND the host is cross-origin isolated, and otherwise brings it up
// exactly as before (par then runs sequentially, same results).
loftInstantiate(wasmBytes,imports).then(async ({{instance,memory}})=>{{
  mem=memory||instance.exports.memory;
  loftInstallEnv(instance, mem);
  // @P321(c) Phase 3b: decode base64 PNG assets to RGBA bytes before
  // loft_start so the wasm-side imaging bridge looks them up sync.
  ctrl.assets=await decodeLoftAssets(ctrl.assets);{fonts_await}
  if(instance.exports.asyncify_start_unwind){{
    const ac=new AsyncifyCtrl(instance);
    ctrl.ac=ac;
    ac.start('loft_start');
    if(ac.sleeping){{
      // Drive the asyncify resume loop.  A HIDDEN page (headless capture, a
      // backgrounded tab) throttles or fully pauses requestAnimationFrame, so
      // an rAF-only loop stalls at the first suspend (issue #450).  Pump via an
      // unthrottled MessageChannel while the page is hidden, and via rAF while
      // visible so a GL render loop stays vsync-aligned.  schedule() re-checks
      // visibility each tick, so a tab going hidden/visible adapts live.
      const mc=new MessageChannel();
      // A trap inside a RESUME lands here rather than on the boot promise, and
      // an uncaught one stops the pump silently — a frame loop that dies with a
      // blank page. Same reporter, so a trap reads the same wherever it fires.
      const pump=()=>{{
        try{{ if(ac.resume('loft_start'))schedule(); }}catch(e){{ loftReportTrap(e); }}
      }};
      mc.port1.onmessage=pump;
      const schedule=()=>{{
        if(document.hidden)mc.port2.postMessage(0);
        else requestAnimationFrame(pump);
      }};
      schedule();
    }}
  }}else{{
    instance.exports.loft_start();
  }}
}}).catch(loftReportTrap);
</script></body></html>"#
            )
        };
        if let Err(e) = std::fs::write(&html_path, &html) {
            eprintln!("loft: cannot write HTML to '{html_path}': {e}");
            std::process::exit(1);
        }
        let wasm_kb = wasm_bytes.len() / 1024;
        let html_kb = html.len() / 1024;
        let shell = if minimal_page {
            " · minimal engine-less shell"
        } else {
            " · full engine shell"
        };
        println!("wrote {html_path} ({html_kb} KB, WASM {wasm_kb} KB{shell})");
        // loft#1238 — the per-invocation breakdown, slowest first.  Silent unless LOFT_TIMING
        // is set AND something actually ran, so a build that reused every artifact says
        // nothing.
        loft::platform::timing_report("--html");
        return;
    }

    // Native codegen pipeline: --native or --native-emit.
    //
    // rustc availability is checked lazily, in the cache-miss compile branch
    // below (the NotFound arm of `cmd.output()`) — NOT up front.  A warm cache
    // hit runs the cached binary and never needs rustc, so an up-front
    // `rustc --version` probe would tax every run with a ~18 ms process spawn
    // for nothing.  The `'native` label lets the lazy check fall back to the
    // interpreter when rustc is genuinely absent on a cache miss.
    //
    // Each fallback `break 'native` below records WHY in `native_fallback_reason`
    // so the chokepoint past the block can report it (warning by default, hard
    // error under `LOFT_REQUIRE_NATIVE`).  `None` after the block means native
    // either ran (and `return`ed) or was never requested.
    let mut native_fallback_reason: Option<String> = None;
    'native: {
        if !(native_mode || native_emit.is_some()) {
            break 'native;
        }
        let end_def = p.data.definitions();
        let emit_path = match native_emit.as_deref() {
            // Default `loft <file>` writes to a per-process tmp file
            // so concurrent invocations (e.g. nextest's parallel test
            // execution) don't race on a single shared
            // `temp_dir/loft_native.rs`.  The PID suffix is a process-
            // local choice; user-visible artefacts pass through
            // `--native-emit <path>` which the user controls.
            None => platform::scratch_dir().join(format!("loft_native_{}.rs", std::process::id())),
            Some("") => default_artifact_path(&abs_file, "rs"),
            Some(p) => std::path::PathBuf::from(p),
        };
        {
            let mut f = match std::fs::File::create(&emit_path) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!(
                        "loft: cannot write native source to '{}': {e}",
                        emit_path.display()
                    );
                    std::process::exit(1);
                }
            };
            // @P379 — qualify native symbols for functions whose name
            // collides across libraries (no-op without a collision; calls
            // resolve by d_nr so the renamed def stays consistent).
            p.data.namespace_colliding_native_fns();
            let mut out = generation::Output::new(&p.data, &state.database);
            // @PLN98 P3.1 — embed the program's own source so a live build can
            // bootstrap the parked interpreter from BYTES (no `LOFT_LIVE_SRC` file)
            // — the browser/wasm delivery.  Best-effort: unreadable → fs fallback.
            out.program_src = std::fs::read_to_string(&abs_file).ok();
            // Host-native backend: link each `#native` package's cdylib by C-ABI
            // (`extern "C"` decls + `.so`), not its rlib — see NATIVE.md
            // § Resolution: separate the API id from the Rust part.  The shared
            // `native_cabi_enabled()` keeps codegen and the linker flags in sync
            // (off on Windows, which stays on the rlib path).
            out.native_cabi = native_utils::native_cabi_enabled();
            // @PLN98 P2 — `--lean` strips the live/debug tier from the emitted Rust.
            // @PLN157 — and, separately, the frame NAMES; `out.lean` is the one
            // home for that second question (see `generation::Output::lean`).
            // `--native-release` asks for both: a shipped binary is fully optimised,
            // and the named per-call frame push was the largest single cost the tier
            // carried (measured on the drawing pass, consumer lane: `hash` 4.7M → 1.2M
            // ns/op, `hair` −55 %, `wide_line` −32 %, the pixel rows −13–19 %).
            // `--native` keeps every tier — that is the semantics run, where frames and
            // the live channel are worth their cost.  The browser (`--html`) path is
            // separate and keeps its named frames: its panic hook's frame block is a
            // pinned contract.
            let shipped = lean || native_release;
            out.lean = shipped;
            if shipped {
                out.emit_live = false;
                out.lean_tier = true;
            }
            let result = if native_release {
                let main_nr = p.data.def_nr("n_main");
                let entry_defs: Vec<u32> = if main_nr < end_def {
                    vec![main_nr]
                } else {
                    (start_def..end_def).collect()
                };
                out.output_native_reachable(&mut f, start_def, end_def, &entry_defs)
            } else {
                out.output_native(&mut f, 0, end_def)
            };
            if let Err(e) = result {
                eprintln!("loft: native code generation failed: {e}");
                std::process::exit(1);
            }
            // For test-only files (no fn main()), generate a main() that calls
            // all zero-parameter user functions as test entry points.
            //
            // A file with neither — a LIBRARY, whose `pub fn`s all take arguments — gets no
            // `main` at all.  The rustc call below reads that off the generated crate and
            // compile-CHECKS it instead of linking a binary that cannot exist (loft#1171).
            let main_nr = p.data.def_nr("n_main");
            if main_nr >= end_def {
                use std::io::Write;
                let mut test_fns: Vec<(u32, String)> = Vec::new();
                for d_nr in start_def..end_def {
                    let def = p.data.def(d_nr);
                    if !matches!(def.def_type, loft::data::DefType::Function) {
                        continue;
                    }
                    if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
                        continue;
                    }
                    if portable_path::is_stdlib_source(&def.position.file) {
                        continue;
                    }
                    let has_user_params = def
                        .attributes
                        .iter()
                        .any(|a| !a.name.starts_with("__work_") && !a.name.starts_with("__ref_"));
                    if has_user_params {
                        continue;
                    }
                    test_fns.push((d_nr, def.name.clone()));
                }
                // loft#1010 — same rule as the CLI runner (`test_runner.rs`): a file that
                // names any `test_*` has said which functions are tests, so a helper with
                // no arguments is not one.  Kept in step with that site deliberately — a
                // generated entry point that runs a different SET than the interpreter is
                // a backend divergence the suite reads as a wrong answer.
                if test_fns.iter().any(|(_, n)| n.starts_with("n_test_")) {
                    test_fns.retain(|(_, n)| n.starts_with("n_test_"));
                }
                if !test_fns.is_empty() {
                    let _ = writeln!(f, "\nfn main() {{");
                    // P199 — wrap Stores in UnsafeCell so the native ABI
                    // can pass `&UnsafeCell<Stores>` instead of `&mut Stores`,
                    // eliminating E0499 in nested user-fn calls.  Each
                    // generated function derives its own short-lived
                    // `&mut Stores` from the cell at function entry.
                    let _ = writeln!(
                        f,
                        "    let cell = std::cell::UnsafeCell::new(Stores::new());"
                    );
                    let _ = writeln!(
                        f,
                        "    let stores: &mut Stores = unsafe {{ &mut *cell.get() }};"
                    );
                    let _ = writeln!(f, "    init(&cell);");
                    for (d_nr, name) in &test_fns {
                        let def = p.data.def(*d_nr);
                        let mut work_args = Vec::new();
                        for (i, attr) in def.attributes.iter().enumerate() {
                            if attr.name.starts_with("__work_") {
                                let wname = format!("_w_{i}");
                                let _ = writeln!(f, "    let mut {wname} = String::new();");
                                work_args.push(format!("&mut {wname}"));
                            } else if attr.name.starts_with("__ref_") {
                                let wname = format!("_r_{i}");
                                let _ = writeln!(
                                    f,
                                    "    let mut {wname} = stores.null_named(\"{wname}\");"
                                );
                                work_args.push(wname.clone());
                            }
                        }
                        if work_args.is_empty() {
                            let _ = writeln!(f, "    {name}(&cell);");
                        } else {
                            let _ = writeln!(f, "    {name}(&cell, {});", work_args.join(", "));
                        }
                    }
                    let _ = writeln!(f, "}}");
                }
            }
        }
        // @PLN130 — native generation is complete, so every copy IT wrote is on the
        // manifest.  Reported here (before the `--native-emit` return) so emitting the
        // source is checked exactly like compiling it.  A no-op unless
        // `LOFT_COPY_MANIFEST` is set.
        loft::copy_manifest::report(&p.data);
        if native_emit.is_some() {
            return; // --native-emit: just write the file, don't compile
        }
        // --native / --native-release: compile with rustc and run.
        // Cache compiled binaries in .loft/cache/ next to the source file,
        // keyed by a hash of the generated Rust source so recompilation is
        // skipped when the output hasn't changed.
        let source_bytes = std::fs::read(&emit_path).unwrap_or_default();
        let source_hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            source_bytes.hash(&mut h);
            // Include the release + debug flags in the hash so each
            // distinct rustc invocation produces a distinct cached
            // binary.  Without this, switching between `--native`
            // and `--native --native-debug` returns whichever build
            // ran first — debugger sees a stripped binary, or a
            // user accidentally runs an unoptimised build under
            // `--native-release` because the debug build was cached.
            native_release.hash(&mut h);
            native_debug.hash(&mut h);
            // Include modification times of native package rlibs and loft's
            // own rlib so the cache invalidates when dependencies are rebuilt.
            if let Some(lib_dir) = loft_lib_dir() {
                if let Ok(meta) = std::fs::metadata(lib_dir.join("libloft.rlib")) {
                    meta.modified().ok().hash(&mut h);
                }
            }
            for (_crate_name, pkg_dir) in &p.data.native_packages {
                let rlib_name = format!("lib{}.rlib", _crate_name.replace('-', "_"));
                let rlib_path = std::path::PathBuf::from(pkg_dir)
                    .join("native/target/release")
                    .join(&rlib_name);
                if let Ok(meta) = std::fs::metadata(&rlib_path) {
                    meta.modified().ok().hash(&mut h);
                }
            }
            format!("{:016x}", h.finish())
        };
        let cache_dir = std::path::Path::new(&abs_file)
            .parent()
            .unwrap_or(std::path::Path::new("."))
            .join(".loft")
            .join("cache");
        let source_stem = std::path::Path::new(&abs_file)
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let cached_binary = cache_dir.join(format!("{source_stem}-{source_hash}"));

        // P254 — cache-poisoning defense.  Bypass the cache entirely
        // when the user opts out via `LOFT_NATIVE_NO_CACHE=1` (matches
        // the behaviour the workaround in PROBLEMS.md described before
        // this fix landed; documented here so paranoid users have a
        // hard kill switch even if the safety helpers gain a future
        // bug).  We also reject the cache when the cached file fails
        // the safety helpers — symlinked cache, wrong owner uid,
        // group/other-readable, or SUID-set.  In every reject case we
        // recompile (rather than refusing to run) so a poisoned cache
        // doesn't deny the user service; it just costs them a
        // recompile.
        let no_cache = std::env::var("LOFT_NATIVE_NO_CACHE")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);
        let cache_usable = !no_cache
            && cached_binary.exists()
            && native_utils::cache_safe_to_execute(&cached_binary);
        if !no_cache && cached_binary.exists() && !cache_usable {
            eprintln!(
                "loft: rejecting suspicious cached binary at {} (P254 — wrong owner, world-writable, symlink, or SUID); recompiling",
                cached_binary.display()
            );
        }

        // loft#706 — has the runtime rlib already been rebuilt in this run?  Both heal
        // sites (the up-front version check and the post-compile retry) read it, so a
        // tree the rebuild cannot fix fails once instead of rebuilding per attempt.
        let mut runtime_rebuilt = false;
        // Use cached binary if it exists AND passes the safety check;
        // otherwise compile and cache.
        let binary = if cache_usable {
            cached_binary.clone()
        } else {
            // Up-front toolchain check (cache miss ⇒ about to compile).  A rustc
            // that differs from the one this loft + its rlib were built with
            // (the LOFT_BUILD_RUSTC stamp from build.rs) can't link the SVH-locked
            // rlib — the post-`rustup update` case.  For a DEFAULT native run,
            // detect it here and fall back to the interpreter WITHOUT the doomed
            // compile.  Cheap: one `rustc --version`, only on a cache miss (warm
            // hits ran the cached binary above) and only for the default path
            // (explicit `--native` proceeds and errors with the rebuild
            // diagnostic).  The lazy post-compile fallback still backstops
            // anything missed here (e.g. a matching rustc but a missing rlib).
            if let Some(reason) = loft::cache::rustc_mismatch() {
                // rustc changed since this loft was built, so the cached runtime
                // rlib is SVH-locked to the old rustc and can't be reused.  In a
                // source checkout, self-heal: rebuild the runtime with the user's
                // rustc and carry on natively — the fresh rlib is picked up by the
                // `loft_lib_dir()` resolution below.  From a bundle there is no
                // source to rebuild, so fall back (default) or error (`--native`).
                let healed = native_utils::loft_source_tree()
                    .is_some_and(|tree| native_utils::rebuild_runtime(&tree, reason));
                // Whether it healed or not, the rebuild has been attempted — the
                // post-compile heal (loft#706) must not run it a second time.
                runtime_rebuilt = true;
                if !healed && !native_requested {
                    // T0.1 — SILENT on the default path.  The user did not ask for
                    // native, and a downloaded release ships no native runtime by
                    // design, so "rebuild from source" is not an action they want:
                    // it reads as a defect report on a run that is about to succeed.
                    // The reason is still recorded below and surfaced to whoever DID
                    // ask (`--native`, or `LOFT_REQUIRE_NATIVE`, which hard-errors
                    // with it) — silence here costs no diagnosis.
                    native_fallback_reason = Some(format!(
                        "native compilation unavailable ({reason}); rebuild loft"
                    ));
                    let _ = std::fs::remove_file(&emit_path);
                    break 'native;
                }
                // healed → fresh rlib in place, fall through to the compile.
                // !healed && native_requested → fall through; the compile errors
                // below with the actionable `--native` message.
            }
            // Per-process tmp path — same rationale as the emit_path
            // above: avoids races between concurrent `loft <file>`
            // invocations.  The cached path (`cached_binary` above)
            // is content-addressed (source_hash) and thus safe to
            // share across processes; this fallback is only used
            // when the cache miss; it doesn't need to be content-
            // addressed but does need to be unique per process.
            let scratch = platform::scratch_dir();
            // Layer 2: refuse to start a compile that could overflow a
            // RAM-backed tmpfs and exhaust memory (reclaims loft's own stale
            // artefacts first).  Warn + continue rather than hard-fail; rustc
            // will surface a real ENOSPC if it genuinely can't write.
            if !platform::native_compile_space_ok(&scratch) {
                eprintln!(
                    "loft: warning — low space in {} (set LOFT_TMPFS_MIN_FREE_MB to tune)",
                    scratch.display()
                );
            }
            let binary = scratch.join(format!("loft_native_bin_{}", std::process::id()));
            // Does the crate that was just generated have an entry point?  Read it off the
            // artefact rather than re-deriving the decision that produced it: the file is
            // the fact, and one `fn main` in it is exactly what rustc will look for.
            let program_has_entry = std::fs::read_to_string(&emit_path)
                .map(|src| src.contains("\nfn main(") || src.starts_with("fn main("))
                .unwrap_or(true);
            let mut cmd = std::process::Command::new("rustc");
            cmd.env("TMPDIR", &scratch).arg("--edition=2024");
            if program_has_entry {
                cmd.arg("-o").arg(&binary);
            } else {
                // No `main`: there is no program to link.  Check the crate compiles and
                // stop — which is what a library asked to build as an executable can
                // honestly answer, and it keeps `loft build` / `loft check` on a
                // library-only package a clean pass instead of a raw rustc E0601.
                cmd.arg("--crate-type=lib")
                    .arg("--emit=metadata")
                    .arg("-o")
                    .arg(scratch.join(format!("loft_native_check_{}.rmeta", std::process::id())));
            }
            cmd.arg(&emit_path);
            if native_release {
                // A shipped binary: full optimisation.  Measured on the drawing pass
                // over `-O`: `hash` −15–20 %, the pixel rows −3–5 %; opt-level 3 alone
                // and `-C target-cpu=native` moved nothing, so neither is asked for on
                // its own.  Semantics runs (`--native`, the test runner) keep the
                // faster compile.
                cmd.args(["-C", "opt-level=3", "-C", "codegen-units=1"]);
            }
            // Layer 1: strip the linked binary (~36MB → ~1MB; the bulk is
            // debug info from libloft.rlib + std).  Skipped when the user
            // asked for debug info (--native-debug) or set
            // LOFT_NATIVE_KEEP_SYMBOLS=1.
            if !native_debug && platform::native_strip_symbols() {
                cmd.arg("-Cstrip=symbols");
            }
            // NDB.0 — when --native-debug is set, emit DWARF debug
            // info so stock GDB / LLDB can step through the native
            // binary.  Combines with --native-release: the user gets
            // an optimised build with debug info if both flags are
            // present.
            if native_debug {
                cmd.arg("-Cdebuginfo=2");
            }
            // P266 follow-up: each native package's rlib carries a copy
            // of `loft_register_v1` (synthesized by the `loft_ffi::loft_register!`
            // macro for the cdylib's dlopen registration path).  When two or
            // more native packages are pulled into the SAME --native binary
            // (e.g. the viewer pulls lib/web AND lib/server transitively),
            // ld errors with `duplicate symbol: loft_register_v1`.  The
            // binary never calls `loft_register_v1` (it inlines
            // `loft_<crate>::n_…` directly), so the duplicates are
            // functionally harmless.  Tell the linker to merge them
            // (keep the first definition, skip the rest).  This matches
            // the GNU ld / lld semantics for `-z muldefs`.
            //
            // macOS ld64 rejects `--allow-multiple-definition` as an
            // unknown option (the Apple linker has no equivalent
            // surface — duplicate symbols are either silently
            // permitted by symbol kind, or errors).  Skip the flag on
            // macOS; if a future cross-package binary hits a real
            // duplicate-symbol error on macOS, we'll narrow the fix
            // (e.g. weak-link the symbol or dedup the macro emission)
            // rather than re-add a flag the host linker doesn't
            // support.
            //
            // MSVC `link.exe` does not understand it either, and unlike ld64 it
            // does not fail — it prints `LNK4044: unrecognized option …; ignored`
            // once per occurrence, which on the `#c` shim path was three lines of
            // noise directly above the real error. Same reason as macOS: a flag
            // the host linker has no equivalent for is not passed to it.
            #[cfg(not(any(target_os = "macos", windows)))]
            cmd.arg("-Clink-arg=-Wl,--allow-multiple-definition");
            // Point rustc at loft's own runtime rlib and everything it links against,
            // answering the deps dir it found.  A closure rather than straight-line code
            // because the post-compile heal below rebuilds that rlib and must ask AGAIN:
            // the args are decided from what is on disk, and the whole point of the
            // rebuild is to change that (loft#855).
            let attach_loft_runtime = |cmd: &mut std::process::Command| {
                let lib_dir = loft_lib_dir()?;
                cmd.args(loft::native_lib::loft_extern_args(
                    &lib_dir.join("libloft.rlib"),
                ));
                // One `-L` per search dir: the classic layout yields exactly one
                // (`<profile>/deps`), the per-unit layout cargo nightly adopted on
                // 2026-07-29 yields one per crate.  See `dep_search_dirs`.
                let search = native_utils::dep_search_dirs(&lib_dir);
                for d in &search {
                    cmd.arg("-L").arg(format!("dependency={}", d.display()));
                }
                let deps = search
                    .first()
                    .cloned()
                    .unwrap_or_else(|| deps_dir_of(&lib_dir));
                // Pick the `loft_ffi` that `libloft` was built against, NOT the first
                // in dir order: with two copies in `deps/`, naming the wrong one puts
                // a second `loft_ffi` in the link → "colliding StableCrateId" (see
                // `native_lib::loft_ffi_for_libloft`).
                if let Some(ffi) =
                    loft::native_lib::loft_ffi_for_libloft(&lib_dir.join("libloft.rlib"), &deps)
                {
                    cmd.arg("--extern")
                        .arg(format!("loft_ffi={}", ffi.display()));
                }
                // Propagate `-L native=` for every build-script `OUT_DIR`
                // that bundles a native lib.  Windows-targets ships
                // `windows.0.48.5.lib` inside its OUT_DIR; without these
                // paths the link step fails with `LNK1181: cannot open
                // input file 'windows.0.48.5.lib'`.
                for out_dir in build_script_native_lib_dirs(&lib_dir) {
                    cmd.arg("-L").arg(format!("native={}", out_dir.display()));
                }
                Some(deps)
            };
            let mut native_deps_dir = attach_loft_runtime(&mut cmd);
            // PKG.4: add --extern flags for native packages.
            native_utils::add_native_extern_flags(
                &mut cmd,
                &p.data,
                None,
                native_deps_dir.as_deref(),
            );
            // @PLN24 arc D — and the C libraries `#c` bindings resolve against.
            // Host target only: wasm has no C ABI to link (arc E).
            native_utils::add_c_library_flags(&mut cmd, &p.data);
            // @PLN54 S6 — native-backend AddressSanitizer.  LOFT_NATIVE_ASAN=1
            // instruments the generated native binary with ASan so a codegen bug
            // that emits an out-of-bounds / use-after-free raw-pointer store access
            // (the class the in-process interpreter ASan job cannot see, because
            // `--native` runs as a separate uninstrumented binary) is caught, not
            // silent.  Needs nightly rustc (-Zsanitizer); the binary is per-PID so
            // there is no artifact cache to invalidate.  Opt-in, off by default.
            if std::env::var_os("LOFT_NATIVE_ASAN").is_some() {
                if std::env::var_os("RUSTUP_TOOLCHAIN").is_none() {
                    cmd.env("RUSTUP_TOOLCHAIN", "nightly");
                }
                cmd.arg("-Zsanitizer=address");
            }
            let output = cmd.output();
            let output = match output {
                Ok(o) => o,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    // No rustc on a cache miss → fall back to the interpreter
                    // rather than failing.  This is the moment the old up-front
                    // probe used to fire; doing it here means a warm cache hit
                    // never pays for a `rustc --version` spawn.
                    // T0.1 — only explain when native was explicitly asked for.
                    if native_requested {
                        eprintln!(
                            "loft: rustc not found — `--native` needs a Rust toolchain \
                             on PATH; running interpreted instead."
                        );
                    }
                    native_fallback_reason = Some("rustc not found".to_string());
                    let _ = std::fs::remove_file(&emit_path);
                    break 'native;
                }
                Err(e) => {
                    // T0.1 — as above: explain only on an explicit request.
                    if native_requested {
                        eprintln!(
                            "loft: rustc could not be launched ({e}) — `--native` needs a \
                             working Rust toolchain; running interpreted instead."
                        );
                    }
                    native_fallback_reason = Some(format!("rustc could not be launched ({e})"));
                    let _ = std::fs::remove_file(&emit_path);
                    break 'native;
                }
            };
            // loft#706 — heal on the COMPILE, not only on the up-front version check.
            //
            // That check (`rustc_mismatch`, above) compares the rustc that built the
            // loft BINARY with the current one.  What this compile LINKS is the
            // runtime RLIB, and one `cargo build` produces both — so the binary is a
            // fine proxy for the rlib right up until it isn't: a partially-restored
            // CI cache, an installed bundle beside a source checkout, a lib and a
            // binary built either side of a `rustup update`.  Then the check sees
            // nothing to do, and rustc is handed an rlib from another compiler:
            // `E0514`, with no rebuild attempted and no recovery, in exactly the
            // situation the auto-rebuild exists for.
            //
            // The compile is the reliable witness the version check is not, so heal
            // on it: rebuild the runtime once and retry.  Gated on a failure whose
            // error already names crate resolution, so a codegen bug never pays for a
            // rebuild, and on `runtime_rebuilt` so a tree the up-front heal already
            // rebuilt does not rebuild twice.  This path itself runs at most once —
            // it is straight-line, not a loop — so it does not set the flag.
            let mut output = output;
            if !output.status.success()
                && !runtime_rebuilt
                && native_utils::crate_resolution_failure(&String::from_utf8_lossy(&output.stderr))
                && let Some(tree) = native_utils::loft_source_tree()
                && native_utils::rebuild_runtime(
                    &tree,
                    "the runtime rlib this program links was built by a different rustc",
                )
            {
                // Retry against what the rebuild PRODUCED, not against what motivated it.
                // The `--extern loft=` / `-L dependency=` args above were chosen from the
                // rlib that was on disk when the command was built, so when there was no
                // rlib at all they were never added — and re-running the same command
                // after a successful rebuild re-ran a rustc that still named no crate.
                // The heal then reported its own success and the identical `E0463: can't
                // find crate for loft` in one breath, which is how it read as a broken
                // toolchain rather than a missed refresh (loft#855, `Suite under nightly`).
                //
                // Only the runtime args are re-asked. Package `--extern`s came from
                // `add_native_extern_flags` and re-running that would emit a second copy
                // of each; a tree with no runtime rlib has no built packages either, so
                // the case this recovers does not need them.
                if native_deps_dir.is_none() {
                    native_deps_dir = attach_loft_runtime(&mut cmd);
                }
                if let Ok(retry) = cmd.output() {
                    output = retry;
                }
            }
            let status = output.status;
            let stderr_utf8 = String::from_utf8_lossy(&output.stderr);
            // Classify a compile failure caused by the native TOOLCHAIN/cache, not
            // by loft codegen: a stale cached rlib after a `rustc`/`rustup update`
            // (E0514 "compiled by an incompatible version", E0460 "possibly newer
            // version" — the common case, rlibs are SVH-locked to one rustc), the
            // rand_core/cargo-cache staleness, an unresolvable loft/library crate
            // (E0463 — e.g. a distributed bundle ships no rlib), or an rmeta-without-
            // rlib dep (@P229 G3, an unbuilt package).
            let crate_resolution_failure = native_utils::crate_resolution_failure(&stderr_utf8);
            let rlib_format_failure =
                stderr_utf8.contains("required to be available in rlib format");
            // Turnkey fallback: a DEFAULT-native run (not an explicit `--native`)
            // that fails ONLY because the native toolchain isn't usable here —
            // loft's cached rlib is stale after a rustc update, or absent in a
            // distributed bundle — degrades to the interpreter so the program still
            // runs.  Keyed on the toolchain/crate failures above, never on arbitrary
            // compile errors, so a genuine codegen bug still surfaces loudly.
            // `--native` stays a hard error (explicit request needs the toolchain
            // set up).  The rustc-not-found (NotFound) arm above is the sibling case.
            if !status.success()
                && !native_requested
                && (crate_resolution_failure || rlib_format_failure)
            {
                // T0.1 — silent on the default path (see the rustc-mismatch arm above).
                native_fallback_reason = Some(
                    "native toolchain not usable here (cached build stale after a rustc \
                     update, or loft's runtime library unavailable); rebuild loft"
                        .to_string(),
                );
                let _ = std::fs::remove_file(&emit_path);
                break 'native;
            }
            // Relay rustc's own output to the user.
            let _ = std::io::Write::write_all(&mut std::io::stderr(), &output.stderr);
            let _ = std::io::Write::write_all(&mut std::io::stdout(), &output.stdout);
            if !status.success() {
                if crate_resolution_failure || rlib_format_failure {
                    // Print the rustc invocation + the deps directory listing
                    // so the diagnostic shows what was actually attempted.
                    // Surfaces the Windows latent issue + any future
                    // platform-specific dep-resolution gaps.
                    eprintln!("\nloft: rustc could not resolve a transitive crate dep.");
                    eprintln!("\nrustc invocation:");
                    let prog = cmd.get_program().to_string_lossy().to_string();
                    eprintln!("  {prog}");
                    for arg in cmd.get_args() {
                        eprintln!("    {}", arg.to_string_lossy());
                    }
                    if let Some(deps) = native_deps_dir.as_ref() {
                        eprintln!("\nDeps directory: {}", deps.display());
                        match std::fs::read_dir(deps) {
                            Ok(rd) => {
                                let mut entries: Vec<_> = rd
                                    .flatten()
                                    .filter_map(|e| {
                                        let n = e.file_name().to_string_lossy().to_string();
                                        let is_rlib = std::path::Path::new(&n)
                                            .extension()
                                            .is_some_and(|ext| ext.eq_ignore_ascii_case("rlib"));
                                        if n.contains("rand") || n.contains("loft") || is_rlib {
                                            Some(n)
                                        } else {
                                            None
                                        }
                                    })
                                    .collect();
                                entries.sort();
                                if entries.is_empty() {
                                    eprintln!("  (no rlibs found in deps directory)");
                                } else {
                                    for n in entries.iter().take(40) {
                                        eprintln!("  {n}");
                                    }
                                    if entries.len() > 40 {
                                        eprintln!("  ... ({} more)", entries.len() - 40);
                                    }
                                }
                            }
                            Err(e) => eprintln!("  (cannot read deps directory: {e})"),
                        }
                    } else {
                        eprintln!(
                            "\nNo deps directory was passed to rustc — `loft_lib_dir()` returned None."
                        );
                    }
                    eprintln!(
                        "\n--native needs loft's runtime rlib, built with your rustc.\n\n\
                         In a source checkout:  cargo build --release --lib --bin loft\n\
                         (prefix `cargo clean &&` after a rustc update).\n\
                         A downloaded release ships no native runtime — drop `--native` \
                         to run on the interpreter, or build loft from source.\n"
                    );
                } else {
                    eprintln!(
                        "loft: native compilation failed (codegen bug — try --native-emit to inspect the source)"
                    );
                }
                std::process::exit(1);
            }
            // Store in cache for next run.  P254 — also opt out
            // when LOFT_NATIVE_NO_CACHE=1 is set so paranoid users
            // can avoid leaving a cache file on disk at all.
            if !no_cache && std::fs::create_dir_all(&cache_dir).is_ok() {
                // P254 — tighten cache-dir mode to 0700 so a future
                // attacker can't drop files into our cache (or
                // remove ours from under us).  Repairs pre-existing
                // wider-mode cache directories left over from earlier
                // loft versions.  No-op on non-Unix.
                native_utils::tighten_cache_dir(&cache_dir);
                if !native_utils::cache_dir_safe(&cache_dir) {
                    // Couldn't tighten the directory — bail on the
                    // cache write so we don't write a binary the
                    // next run will reject anyway.  Common case:
                    // the cache dir lives on a network mount whose
                    // server enforces a different mode than we
                    // requested.
                    eprintln!(
                        "loft: cache directory {} has unsafe permissions and could not be tightened; skipping cache write (P254)",
                        cache_dir.display()
                    );
                } else {
                    // Publish ATOMICALLY.  A plain `fs::copy` onto `cached_binary`
                    // truncates it in place and then streams ~11 MB, and for that
                    // whole window another process testing the cache sees a file
                    // that EXISTS and passes `cache_safe_to_execute` — which reads
                    // symlink, owner and mode but never SIZE.  So a concurrent run
                    // execs a 0-byte-and-growing ELF and dies with no stdout and no
                    // stderr at all.  The mode check does not save it: once the
                    // entry exists at 0700, a later copy truncates it while the
                    // mode stays 0700.  Measured as a red `make ci` in two runs of
                    // three, on `alias_link_baseline::baseline_leak_clean_native` —
                    // whose two native cells compile the same source concurrently,
                    // so they race exactly when the entry is cold.
                    //
                    // Stage under a private per-process name in the SAME directory
                    // (so the rename cannot cross a filesystem), tighten it while it
                    // is still invisible under its final name, then rename.  POSIX
                    // rename is atomic and a process already exec'ing the old inode
                    // keeps it, so every reader sees one COMPLETE binary.  The
                    // leading `.` keeps a staged file out of the sweep's prefix.
                    native_utils::publish_cached_binary(
                        &binary,
                        &cached_binary,
                        &cache_dir,
                        &source_stem,
                    );
                }
            }
            binary
        };
        // NDB.0 — preserve the generated .rs on disk when
        // --native-debug is set so DWARF's `.debug_line` table points
        // at a real file the debugger can show.  Without this, GDB /
        // LLDB show `(no source)` even though debug info is present.
        // PLAN51 — also preserve when `LOFT_KEEP_NATIVE_RS=1` is set,
        // so probe runs that panic in the generated Rust leave the file
        // readable for post-mortem inspection.  Used by the Cluster V
        // (native-only) investigation in plans/finished/51-hidden-buffer-
        // aliasing/cluster-V-native-only.md.
        let keep_rs = std::env::var("LOFT_KEEP_NATIVE_RS").is_ok();
        if !native_debug && !keep_rs {
            let _ = std::fs::remove_file(&emit_path);
        } else {
            let reason = if native_debug {
                "--native-debug"
            } else {
                "LOFT_KEEP_NATIVE_RS"
            };
            eprintln!(
                "loft: source preserved at {} ({reason})",
                emit_path.display()
            );
        }

        if check_only {
            // --check --native: compile succeeded, report ok and exit.
            // @PLN18 08-S4 — the artifact path rides the ok line so the
            // background-rebuild host (live_dispatch) can find the build
            // without re-deriving the cache key (which hashes the GENERATED
            // source + rlib mtimes — only this pipeline can compute it).
            // Prefer the DURABLE content-addressed cache path over the
            // per-pid temp the miss branch built into (the consumer is the
            // S5 swap; a temp path is clobbered by the next same-pid run).
            let artifact = if !no_cache && cached_binary.exists() {
                &cached_binary
            } else {
                &binary
            };
            // Two audiences, one line.  The live host asks for the machine form by
            // setting LOFT_CHECK_ARTIFACT on the child it spawns; a person typing
            // `loft check prog.loft` gets the answer they asked for and nothing else
            // — the paths are an absolute source path they just typed and an internal
            // content-addressed cache entry they cannot act on.
            if std::env::var_os("LOFT_CHECK_ARTIFACT").is_some() {
                println!("ok {abs_file} {}", artifact.display());
            } else {
                println!("ok");
            }
            return;
        }
        // @PLN26 phase 4 — Windows has no RPATH, so a C-ABI-linked native-package
        // DLL must sit beside the binary that loads it; stage it there before the
        // spawn (no-op off Windows / on the rlib path).
        if let Some(dir) = binary.parent() {
            native_utils::stage_native_dlls(dir, &p.data);
        }
        // loft#865 — the loft-level profiler cannot follow the program here, and this
        // is the LAST point at which that is still certain: everything before it may
        // still `break 'native` and interpret after all, where the sampler does arm.
        //
        // Said out loud because the default backend is native, so this is the run a
        // user reaches for a profiler WITH — and an accepted-then-ignored variable
        // ends in a clean exit and an empty terminal, which is indistinguishable from
        // "the profiler ran and your program is not the problem". loft#860 fixed the
        // same hole for test runs; this is the branch next to it.
        announce_profiler_cannot_follow_native();
        // @PLN18 08-S2 — live-dispatch handoff: the spawned binary's bootstrap
        // re-parses the same sources, so hand it the resolved paths the driver
        // already knows.  Inert unless the binary runs under LOFT_LIVE_FLIP=1;
        // explicit user-set values win.
        let mut cmd = std::process::Command::new(&binary);
        cmd.args(&user_args);
        // @PLN26 follow-up — run the native binary with cwd = source_dir so its
        // raw `std::fs` anchors where its loft `file()` does (the binary bakes
        // `program_relative` + reads source_dir from LOFT_SOURCE_DIR).  Mirrors
        // the interpreter chdir above; gated on the same `program_relative`.
        if state.database.program_relative
            && let Some(dir) = std::path::Path::new(&abs_file).parent()
        {
            cmd.current_dir(dir);
        }
        // The artifact anchors relative paths at its OWN dir (the
        // standalone-bundle rule) — in driver mode that is the cache/tmp
        // dir, not the program's.  Hand the source anchor down so file I/O
        // matches the interpreter; an explicit user value wins.
        if std::env::var("LOFT_SOURCE_DIR").is_err()
            && let Some(dir) = std::path::Path::new(&abs_file).parent()
        {
            cmd.env("LOFT_SOURCE_DIR", dir);
        }
        if std::env::var("LOFT_LIVE_SRC").is_err() {
            cmd.env("LOFT_LIVE_SRC", &abs_file);
        }
        if std::env::var("LOFT_LIVE_STDLIB").is_err() {
            cmd.env("LOFT_LIVE_STDLIB", &default_str);
        }
        if std::env::var("LOFT_LIVE_LIBS").is_err() && !p.lib_dirs.is_empty() {
            cmd.env("LOFT_LIVE_LIBS", p.lib_dirs.join(":"));
        }
        // @PLN18 08-S4 — the background rebuild re-invokes THIS driver.
        if std::env::var("LOFT_LIVE_DRIVER").is_err()
            && let Ok(me) = std::env::current_exe()
        {
            cmd.env("LOFT_LIVE_DRIVER", me);
        }
        // A crate with no `main` was compile-CHECKED rather than linked, so no binary
        // exists to run.  Say what happened; reporting a missing file here would describe
        // the symptom of a decision made two steps earlier (loft#1171).
        if !binary.exists() && !cached_binary.exists() {
            eprintln!(
                "loft: `{abs_file}` defines no `main`, so there is nothing to run — it compiled cleanly."
            );
            std::process::exit(0);
        }
        let run_status = cmd.status().unwrap_or_else(|e| {
            eprintln!("loft: failed to run native binary: {e}");
            std::process::exit(1);
        });
        // Clean up temp binary (not the cached copy).
        if binary != cached_binary {
            let _ = std::fs::remove_file(&binary);
        }
        if !run_status.success() {
            native_utils::explain_windows_startup_failure(run_status, &binary, &p.data);
            std::process::exit(run_status.code().unwrap_or(1));
        }
        return;
    }

    // Reached only by the `'native` fallback above (rustc absent on a cache
    // miss).  A `--check --native` run wants a status, not execution: report
    // ok on a clean parse — the same answer the interpret-mode check gives —
    // rather than falling through and running the program.
    if check_only {
        println!("ok");
        return;
    }

    // Crawler / efficiency aid (`LOFT_REQUIRE_NATIVE`): reaching here with native ON
    // means a fallback above broke out of `'native` — native success `return`s at the
    // end of that block, and `--interpret`/`--bytecode`/the test paths set
    // `native_mode = false`, so `native_mode` still true here is exactly "native was
    // wanted but we're about to interpret".  Under the env var that is a hard error
    // (the per-site warning already named the reason for the default warn path).  The
    // `unwrap_or` is a catch-all so a future fallback that forgets to record a reason
    // still errors loudly rather than degrading silently.
    if native_required && native_mode {
        let reason = native_fallback_reason
            .as_deref()
            .unwrap_or("native execution did not occur (reason not recorded)");
        eprintln!(
            "loft: LOFT_REQUIRE_NATIVE is set, but native execution was unavailable \
             ({reason}); refusing to fall back to the interpreter. \
             Fix the toolchain (e.g. `cargo build --release --lib --bin loft`), \
             or unset LOFT_REQUIRE_NATIVE to allow interpreter fallback."
        );
        std::process::exit(1);
    }

    // Initialize the runtime logger
    // Resolved through the SHARED helper so the interpreter and a compiled `--native`
    // binary look in the same places (`.loft/log.conf`, then `log.conf`, beside the
    // program).  They diverging here is how native ended up with no logger at all.
    let conf_path = logger::Logger::resolve_config_path(log_conf.as_deref(), &abs_file);
    let mut lg = logger::Logger::from_config_file(&conf_path, &abs_file);
    if production {
        lg.config.production = true;
    }
    state.database.logger = Some(Arc::new(Mutex::new(lg)));

    let main_nr = p.data.def_nr("n_main");
    // Plan-08 phase 01: --introspect short-circuits everything.
    // Bypass execution; emit bytecode + Rust + slots and exit.
    if introspect_mode {
        let trace_lines = std::mem::take(&mut p.trace_types_lines);
        let opts = loft::introspect::Options {
            sections: introspect_sections.clone(),
            bytecode_out: introspect_bytecode_out.clone(),
            rust_out: introspect_rust_out.clone(),
            slots_out: introspect_slots_out.clone(),
            types_out: introspect_types_out.clone(),
            diff_against: introspect_diff_against.clone(),
            trace_lines,
            // The resolution CONTEXT this invocation assembled.  main.rs is the only
            // place that knows it, and printing it is what makes a dropped `--lib`
            // (@PLN120 E.1) visible without running the program.
            resolution_context: Some(
                loft::repl::ResolutionContext {
                    stdlib_dir: default_str.clone(),
                    lib_dirs: p.lib_dirs.clone(),
                }
                .describe(),
            ),
            why: introspect_why.clone(),
            json: introspect_json,
            fn_filter: introspect_fn_filter.clone(),
            all_fns: introspect_all_fns,
            lib_dirs: Vec::new(),
            install_dir: String::new(),
        };
        let end_def = p.data.definitions();
        if let Err(e) = loft::introspect::emit_all(&mut p.data, &mut state, end_def, &opts) {
            eprintln!("loft: introspect failed: {e}");
            std::process::exit(1);
        }
        return;
    }
    // @PLN26 follow-up — anchor native file I/O at source_dir: chdir so a native
    // crate's raw `std::fs` resolves a relative path the SAME way loft's
    // `resolve_path` (which joins source_dir) does.  Gated on `program_relative`
    // so a `#cwd` program keeps both at the cwd.  Done here — after parse +
    // native-lib resolution (those ran against the invocation cwd), before user
    // execution; no restore needed (the process exits after the run).  The
    // --native path uses `Command::current_dir` on the spawn instead.
    if state.database.program_relative && !state.database.source_dir.is_empty() {
        let _ = std::env::set_current_dir(&state.database.source_dir);
    }
    if main_nr == u32::MAX && !dump_only {
        // No main() — execute each zero-parameter user `test_*()` function
        // INDIVIDUALLY with a fresh `state.database` per call.  This
        // mirrors `src/test_runner.rs`'s per-fn isolation (line 997's
        // `clean_data.clone() + State::new(clean_db.clone())` pattern) and
        // is what prevents shared-State accumulation of leaked stores
        // from one test compounding into a SIGSEGV in a later test.
        //
        // History: the prior wrapper-synthesis approach (a synthetic
        // `fn main() { test_a(); test_b(); ... }`) ran every test in the
        // SAME `State`, so per-call store-lifetime leaks (the @P377
        // family — `map_get_hex(m: Map, …) -> Hex { return chunk.ck_hexes[idx]; }`
        // shape, deep-slice-borrows the dep-inference can't track) accumulated
        // until `cast_vector_from_text` SIGSEGV'd inside the next JSON cast
        // (@P382).  That synthesis also tripped a CONST_STORE re-lock by
        // calling `compile::byte_code` a second time (@P381, fixed by
        // `compile::byte_code_from`).  Both issues vanish with per-test
        // fresh-State isolation, which also matches the canonical
        // `loft --tests <file>` invocation.
        //
        // `--dump` skips this path — it wants the bytecode of the user
        // functions as parsed, not a synthetic execution.
        let mut test_names: Vec<String> = Vec::new();
        for d_nr in start_def..p.data.definitions() {
            let def = p.data.def(d_nr);
            // The no-`main` fallback runs every zero-param void user function
            // (the #358 contract — see `tests/arc_e_program_cache.rs`).  The one
            // exclusion is `#native` host imports: a used LIBRARY's zero-param
            // host imports (`gl_swap_buffers`, `gl_destroy_window`, …) have no
            // loft body, so running one via `execute_argv` hit a `def(u32::MAX)`
            // "Unknown definition" panic.  `def.native.is_empty()` filters them
            // out — the name is NOT gated (a bare `check_me()` must still run).
            if def.name.starts_with("n_")
                && !def.name.starts_with("n___lambda_")
                && matches!(def.def_type, data::DefType::Function)
                && def.native.is_empty()
                && def.attributes.is_empty()
                && matches!(def.returned, data::Type::Void)
                && !portable_path::is_stdlib_source(&def.position.file)
            {
                let name = def.name.strip_prefix("n_").unwrap_or(&def.name);
                test_names.push(name.to_string());
            }
        }
        if !test_names.is_empty() {
            // Per-test isolation pattern (mirrors `src/test_runner.rs:997`):
            // `Stores::clone` only clones the TYPE SCHEMA (allocations + runtime
            // state are reset to empty by design — see `src/database/mod.rs:412`),
            // so each test needs a full re-byte_code on its own freshly-cloned
            // Data + State.  Reset `max` on the clone to 0 because
            // `Stores::clone` preserves `max` from the source but clears
            // `free_bits` — `find_free_slot` would then return the stale `max`
            // value as the first allocation slot, and `State::new`'s initial
            // `db.database(1000)` would panic with `allocations[max]` OOB.
            let clean_data = p.data.clone();
            let clean_db = state.database.clone();
            for name in &test_names {
                let mut data_iter = clean_data.clone();
                let mut db_iter = clean_db.clone();
                db_iter.max = 0;
                let mut state_iter = State::new(db_iter);
                compile::byte_code(&mut state_iter, &mut data_iter);
                // Preserve native-extension wiring across test iterations.
                extensions::load_all(&mut state_iter, all_native_libs.clone());
                extensions::wire_native_fns(&mut state_iter, &data_iter);
                // #303 — the shared-store bridges too: a `loft_shared_*`-marked
                // fn dispatches via `OpStaticCall` in test bodies as well; without
                // this wire every such call hits the panicking stub.
                extensions::wire_shared_native_fns(&mut state_iter, &data_iter);
                state_iter.execute_argv(name, &data_iter, &[]);
            }
        }
    } else if dump_only {
        // --dump: compile to bytecode, dump to stderr, exit (no execution).
        // Respects LOFT_LOG for extra detail (e.g. LOFT_LOG=variables --dump).
        let config = if std::env::var("LOFT_LOG").is_ok() {
            log_config::LogConfig::from_env()
        } else {
            log_config::LogConfig::static_only()
        };
        let mut log = std::io::stderr();
        let _ = state.dump_bytecode(&mut log, &config, &mut p.data);
    } else if std::env::var("LOFT_LOG").is_ok() {
        let config = log_config::LogConfig::from_env();
        let mut log = std::io::stderr();
        if let Err(e) = state.execute_log(&mut log, "main", &config, &p.data) {
            eprintln!("Execution error: {e}");
            std::process::exit(1);
        }
    } else {
        // @PLN18 phase 02 — tier-0 live reload (opt-in): watch the program
        // file and hot-swap edited fns into the running State.  The shadow
        // session inherits the RESOLVED stdlib dir — a relative "default"
        // only exists when the cwd happens to be a loft checkout (#346).
        if std::env::var_os("LOFT_LIVE_RELOAD").is_some() {
            loft::live_reload::install(&abs_file, &default_str, &p.lib_dirs, &p.data);
        }
        // @PLN140 arc B/C — arm the loft-level sampler before the program starts, so
        // the interval the first sample credits is the program's, not start-up's.
        state.arm_profiler();
        state.execute_argv("main", &p.data, &user_args);
        // FY.3: native desktop frame loop — gl_swap_buffers sets frame_yield,
        // causing execute_argv to return. Resume until the program finishes.
        while state.database.frame_yield {
            state.resume();
        }
        // The program is over HERE in the frame-yield case — unwire the
        // parallel ctx at its program-scope owner (`resume` deliberately
        // does not: it also serves per-call re-entry, where the standing
        // ctx must survive).
        state.database.parallel_ctx = None;
    }
    // Plan-07 phase 4 — render typed runtime errors through the
    // phase-2 pretty renderer.  `panic("msg")`, failed `assert`, and
    // every fault-site opcode populate `state.database.runtime_error`;
    // pulling it out here avoids a borrow conflict with the renderer's
    // loader and keeps the existing `had_fatal` exit path intact.
    //
    // Skip `check_store_leaks` when a runtime error halted execution:
    // the abrupt halt skips scope-exit cleanup so owned vectors / texts
    // remain held, but those leaks are EXPECTED and the warning would
    // bury the error message users actually care about.  Keep the
    // leak check for clean exits where it still surfaces real bugs.
    let runtime_err = state.database.runtime_error.take();
    if runtime_err.is_none() {
        state.check_store_leaks();
    }
    // @PLN140 — the profiling reports. Outside the leak guard on purpose: a run that
    // ended in a fault still spent its time and its memory somewhere, and that is
    // often exactly what is being asked. Both are silent unless armed.
    state.report_alloc_sites(&p.data);
    state.report_profile(&p.data);
    // @PLN154 phase 0 — the stack-write census, on the same terms: it measured the run
    // that happened, fault or not.  Silent unless `LOFT_STACK_CENSUS` armed it.
    if loft::stack_census::enabled() {
        loft::stack_census::report(Some(&p.data));
    }
    // @PLN154 phase 1 — the stack shadow's verdict.  It prints on a CLEAN run too: silence
    // is the result this instrument exists to make trustworthy, and a detector that says
    // nothing when it found nothing reads exactly like one that never ran.
    if loft::stack_verify::enabled() {
        loft::stack_verify::report();
    }
    // loft#1088 — the network summary was BUILT and never printed: `LOFT_NET_PROFILE=1`
    // accumulated every event and nothing called `report`, so only `=trace` (which
    // prints per event) produced output at all.  Beside the other two because it is the
    // third instrument that reports on a RUNNING program.
    loft::net_profile::report();
    // @PLN119 arc A — say goodbye to each placed library's worker rather than
    // leaving the kernel to do it. `PR_SET_PDEATHSIG` is the backstop that
    // covers every `exit` path below and an outright kill; this is the graceful
    // one, and it runs after the leak check so a worker teardown can never be
    // what a leak report is describing.
    #[cfg(target_os = "linux")]
    loft::lib_placement::dispatch::shutdown();
    // @PLN130 F8 — LOFT_STRICT_STORES makes both store-lifetime faults fatal: a reference
    // that outlived its store, and a store nobody freed.  Reported at every site during the
    // run (so one run surfaces all of them), and turned into a non-zero exit here so a probe
    // can be a GATE rather than something someone has to read the output of.
    if loft::keys::strict_stores() {
        let n = loft::keys::strict_store_violations();
        if n > 0 {
            eprintln!("[strict-store] FAILED: {n} store-lifetime violation(s)");
            std::process::exit(1);
        }
    }
    // @PLN154 phase 5 — the same reason, one lever over: every finding is reported at its
    // own site during the run, and the count becomes a non-zero exit here so the shadow can
    // be a GATE rather than something someone has to read the output of.  A sweep that has
    // to grep stderr is a sweep that goes green when the format changes.
    if loft::stack_verify::enabled() {
        let n = loft::stack_verify::violations();
        if n > 0 {
            eprintln!("[stack-verify] FAILED: {n} stack-slot violation(s)");
            std::process::exit(1);
        }
    }
    if let Some(err) = runtime_err {
        // The typed-error block plus the call chain captured at raise time, through the
        // renderer the generated binary also uses (`RuntimeError::report_and_exit`).
        // Rendering it here in its own spelling is how `--native` and `--interpret`
        // came to report the same fault two different ways (loft#1056).
        eprint!("{}", err.render());
    }
    if state.database.run_failed() {
        std::process::exit(1);
    }
}

/// Where `loft test` looks for a package's tests, and the prefix it prints.
const TESTS_DIR: &str = "tests";

/// Resolve the `[target]` of `loft test [target]` to a `--tests` argument.
///
/// The target names a test FILE, optionally with a `::selector` suffix
/// (`::name`, `::{a,b}`), and every spelling of that file is accepted: bare
/// (`draw`), with the extension (`draw.loft`), and — loft#913 — **as printed**
/// (`tests/draw.loft`). `loft test` reports its files with the `tests/` prefix,
/// so pasting a failing line back is the obvious way to iterate on one file; it
/// used to be joined onto `tests/` a second time and rejected as
/// `tests/tests/draw.loft`. Since that doubled path can never exist, accepting
/// the prefix cannot change what any working invocation resolves to.
///
/// The `.loft` extension is supplied on the PATH half, not the whole argument —
/// `draw::test_foo` used to become `tests/draw::test_foo`, whose path half
/// (`tests/draw`) has no extension and does not exist, so the documented
/// selector form only worked when the caller also wrote `.loft`.
///
/// A target that IS a directory keeps its name (loft#925). `loft test` already
/// runs a whole directory — that is what the no-argument form does — but naming
/// one had `.loft` appended to it, so `loft test tests` asked for `tests.loft`
/// and `loft test tests/unit` for `tests/unit.loft`, neither of which exists.
/// The subset-of-a-suite invocation therefore looked unsupported, and the
/// consumer who tried to cut a standalone reproducer for the per-file library
/// recompile could not get one to run at all.
fn resolve_test_target(arg: &str) -> String {
    let (path, selector) = match arg.split_once("::") {
        Some((p, s)) => (p, Some(s)),
        None => (arg, None),
    };
    // A path already under the tests directory (or absolute, or reaching out of
    // the package with `..`) is used as given — only a bare test NAME is joined.
    // Read the leading COMPONENT rather than the leading text: `components()`
    // drops a `./` prefix and splits on the platform's separator, so this is
    // right on Windows without a backslash rewrite (which would corrupt a Unix
    // filename that legitimately contains one — `portable_path`'s gate).
    let as_path = std::path::Path::new(path);
    // `components()` keeps a leading `.` (it only drops interior ones), so skip
    // CurDir to see what the path really starts with.
    let first = as_path
        .components()
        .find(|c| !matches!(c, std::path::Component::CurDir));
    // `is_absolute()` is not the question — "is this a bare test NAME" is.  On Windows a
    // path is absolute only WITH a drive prefix, so `/abs/x.loft` came back false and
    // `loft test /abs/x.loft` looked in `tests//abs/x.loft` (loft#970 neighbours; the
    // nightly's Windows leg). `has_root` answers for the rooted form on both platforms,
    // and a bare `Prefix` covers the drive-relative `C:x.loft`, which is no more a name
    // than `/abs` is.
    let rooted = as_path.is_absolute()
        || as_path.has_root()
        || matches!(first, Some(std::path::Component::Prefix(_)))
        || matches!(first, Some(std::path::Component::ParentDir))
        || first.is_some_and(|c| c.as_os_str() == TESTS_DIR);
    let mut out = String::new();
    if !rooted {
        out.push_str(TESTS_DIR);
        out.push('/');
    }
    out.push_str(path);
    // A DIRECTORY names itself — `loft test tests/unit` runs that directory, the
    // same way the no-argument form runs `tests/`.  Checked on the JOINED path so
    // both `loft test unit` and `loft test tests/unit` see the same thing, and only
    // when there is no `::selector` (a selector names a function inside one FILE, so
    // a directory there is a mistake worth leaving to the existing report).
    let names_a_dir = selector.is_none() && std::path::Path::new(&out).is_dir();
    if !names_a_dir
        && !std::path::Path::new(path)
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
    {
        out.push_str(".loft");
    }
    if let Some(sel) = selector {
        out.push_str("::");
        out.push_str(sel);
    }
    out
}

#[cfg(test)]
mod tests {
    /// loft#1136 — `publish` must not print `"deps": {}` as an answer when the package's own
    /// source says otherwise.  A multi-package repo keeps its registry deps out of `loft.toml`
    /// deliberately, so an empty `[dependencies]` there means "not stated", and the entry is
    /// pasted verbatim into the index.
    #[test]
    fn source_uses_that_the_manifest_does_not_declare_are_reported() {
        let dir = std::env::temp_dir().join(format!("loft_1136_{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(&src).expect("mkdir");
        std::fs::write(
            src.join("hex_shape.loft"),
            "use hex_field::*;\nuse hex_grid;\nuse helper::thing;\nuse hex_shape;\n",
        )
        .expect("write entry");
        // A SIBLING module of this package, not a registry package: `use helper` names it.
        std::fs::write(src.join("helper.loft"), "fn h() -> integer { 1 }\n").expect("write mod");

        let declared = vec![("hex_grid".to_string(), ">=0.1".to_string())];
        let got = super::undeclared_source_deps(&dir, "hex_shape", &declared);
        assert_eq!(
            got,
            vec!["hex_field".to_string()],
            "only the use that is neither a sibling module, nor the package itself, nor \
             already declared"
        );

        // With nothing declared, BOTH registry uses are reported — the shape the issue filed.
        let got_none = super::undeclared_source_deps(&dir, "hex_shape", &[]);
        assert_eq!(
            got_none,
            vec!["hex_field".to_string(), "hex_grid".to_string()]
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The control: a package whose every `use` is a sibling module has nothing to report, so
    /// a genuinely dependency-free package still gets a clean `"deps": {}`.
    #[test]
    fn a_package_with_only_local_modules_reports_nothing() {
        let dir = std::env::temp_dir().join(format!("loft_1136b_{}", std::process::id()));
        let src = dir.join("src");
        std::fs::create_dir_all(&src).expect("mkdir");
        std::fs::write(src.join("solo.loft"), "use parts::*;\n").expect("write entry");
        std::fs::write(src.join("parts.loft"), "fn p() -> integer { 2 }\n").expect("write mod");
        assert!(super::undeclared_source_deps(&dir, "solo", &[]).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;

    /// loft#925 — a target that IS a directory names itself; only a test FILE
    /// gets `.loft` appended.  `loft test tests` used to ask for `tests.loft`, so
    /// running a subset of a suite looked unsupported even though the no-argument
    /// form runs a directory already.
    ///
    /// Anchored on real directories of THIS repo (`cargo test` runs at the repo
    /// root), so the cell exercises the filesystem probe rather than a mock of it —
    /// and on a name that is deliberately NOT a directory, which is what keeps the
    /// probe from swallowing the ordinary file case.
    #[test]
    fn a_directory_target_keeps_its_name() {
        assert_eq!(resolve_test_target("tests/scripts"), "tests/scripts");
        assert_eq!(resolve_test_target("tests/docs"), "tests/docs");
        // Not a directory → still a test file, extension supplied as before.
        assert_eq!(
            resolve_test_target("tests/no_such_dir"),
            "tests/no_such_dir.loft"
        );
        // A `::selector` names a function inside one FILE, so the directory probe
        // is skipped and the existing report handles the mistake.
        assert_eq!(
            resolve_test_target("tests/scripts::test_one"),
            "tests/scripts.loft::test_one"
        );
    }

    /// loft#913 — every spelling of the same test file resolves to the same
    /// `--tests` argument, INCLUDING the `tests/…` form `loft test` itself prints.
    /// The doubled path it used to produce (`tests/tests/good.loft`) can never
    /// exist, so these are all additions, not changes.
    #[test]
    fn a_test_target_resolves_the_same_however_it_is_spelled() {
        for spelling in ["good", "good.loft", "tests/good.loft", "tests/good"] {
            assert_eq!(
                resolve_test_target(spelling),
                "tests/good.loft",
                "spelling {spelling:?}"
            );
        }
        // A `./` prefix is recognised as already-rooted and passed through rather
        // than joined — the string keeps the prefix, which names the same file.
        assert_eq!(
            resolve_test_target("./tests/good.loft"),
            "./tests/good.loft"
        );
    }

    /// The `::selector` suffix survives, and the extension is supplied on the PATH
    /// half — `draw::test_foo` used to resolve to `tests/draw::test_foo`, whose path
    /// half has no extension and matches no file.
    #[test]
    fn a_selector_keeps_its_suffix_and_still_gets_the_extension() {
        assert_eq!(
            resolve_test_target("good::test_one"),
            "tests/good.loft::test_one"
        );
        assert_eq!(
            resolve_test_target("good.loft::test_one"),
            "tests/good.loft::test_one"
        );
        assert_eq!(
            resolve_test_target("tests/good.loft::test_one"),
            "tests/good.loft::test_one"
        );
        assert_eq!(resolve_test_target("good::{a,b}"), "tests/good.loft::{a,b}");
    }

    /// A path that reaches outside the package is used as given: joining it under
    /// `tests/` would silently look somewhere the caller did not name.
    #[test]
    fn a_path_outside_the_package_is_not_joined() {
        assert_eq!(
            resolve_test_target("../other/tests/x.loft"),
            "../other/tests/x.loft"
        );
        assert_eq!(resolve_test_target("/abs/x.loft"), "/abs/x.loft");
    }

    /// A test file whose own name starts with `tests` is a NAME, not a directory —
    /// only the `tests` component itself counts as already-rooted.
    #[test]
    fn a_name_beginning_with_tests_is_still_a_name() {
        assert_eq!(resolve_test_target("testsuite"), "tests/testsuite.loft");
    }

    // The crypto bridge manifest verbatim (loft-libs-core crypto/wasm/Cargo.toml,
    // as published in crypto 0.3.3): one `loft` path dep + the dalek/RustCrypto stack.
    const CRYPTO_BRIDGE_CARGO: &str = "\
[package]
name = \"crypto-wasm\"
version = \"0.1.0\"
edition = \"2024\"

[lib]
crate-type = [\"rlib\"]

[dependencies]
loft = { path = \"../../../loft\", default-features = false, features = [\"random\"] }
ed25519-dalek = { version = \"2.1\", default-features = false, features = [\"std\", \"fast\"] }
x25519-dalek  = { version = \"2.0\", default-features = false, features = [\"static_secrets\"] }
aes-gcm       = { version = \"0.10\", default-features = false, features = [\"aes\", \"alloc\"] }
";

    #[test]
    fn bridge_nonloft_deps_excludes_loft_and_keeps_the_rest() {
        let deps = bridge_nonloft_deps(CRYPTO_BRIDGE_CARGO);
        let idents: Vec<&str> = deps.iter().map(|(i, _)| i.as_str()).collect();
        // #446: `loft` must NOT appear — it is the redundant path dep that fails
        // to resolve for a registry-installed package.
        assert!(
            !idents.contains(&"loft"),
            "loft must be excluded, got {idents:?}"
        );
        // The hyphenated crate names are normalised to rlib idents.
        assert_eq!(idents, ["ed25519_dalek", "x25519_dalek", "aes_gcm"]);
        // The full dependency line is preserved verbatim (versions + features).
        assert!(deps[0].1.contains("version = \"2.1\""));
        assert!(deps[2].1.contains("features = [\"aes\", \"alloc\"]"));
    }

    #[test]
    fn synth_manifest_never_contains_loft_path_dep() {
        let deps = bridge_nonloft_deps(CRYPTO_BRIDGE_CARGO);
        let manifest = synth_bridge_deps_manifest(&deps);
        // #446 invariant: the synthesized deps-only manifest carries the non-loft
        // deps but NO `loft` line, so cargo never resolves `../../../loft`.
        assert!(
            !manifest.contains("loft = {"),
            "synth manifest leaked the loft dep:\n{manifest}"
        );
        assert!(!manifest.contains("../../../loft"));
        assert!(manifest.contains("ed25519-dalek = { version = \"2.1\""));
        assert!(manifest.contains("[lib]\ncrate-type = [\"rlib\"]"));
    }

    #[test]
    fn bridge_with_no_extra_deps_yields_empty() {
        // A bridge whose only dep is `loft` (e.g. the web WS bridge) must produce
        // zero non-loft deps, so the driver skips the build-extension entirely.
        let web = "[package]\nname = \"web-wasm\"\n\n[dependencies]\n\
                   loft = { path = \"../../../loft\" }\n";
        assert!(bridge_nonloft_deps(web).is_empty());
    }
}
