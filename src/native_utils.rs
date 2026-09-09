// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator

//! Native compilation utilities: rlib management, cache keys, artifact paths.

use std::env;
pub(crate) fn with_trailing_sep(p: &std::path::Path) -> String {
    let mut s = p.to_str().unwrap_or("").to_string();
    if !s.ends_with('/') && !s.ends_with('\\') {
        s.push(std::path::MAIN_SEPARATOR);
    }
    s
}

/// Return the directory that contains `libloft.rlib` for the given target triple.
/// Pass `None` for the native target, `Some("wasm32-wasip2")` for WASM.
/// Returns `None` when the rlib cannot be located.
pub(crate) fn loft_lib_dir_for(target: Option<&str>) -> Option<std::path::PathBuf> {
    loft_lib_dir_for_shaped(target, None)
}

/// As [`loft_lib_dir_for`], but keyed on a runtime SHAPE as well as the triple.
///
/// #619 — `Html` and `HtmlThreads` share the triple `wasm32-unknown-unknown`, so a
/// triple-only lookup answers a request for the THREADED runtime with the
/// single-threaded rlib.  That `Some` is silent: `--html --threads` then linked a
/// runtime with no threading in it, exited 0, printed nothing, and produced a page
/// annotated as threaded whose `par` ran sequentially on the main thread — the one
/// outcome the caller has a diagnostic for, on the path where it was unreachable.
/// A shape with its own `install_subdir` is looked up ONLY there, so a missing
/// threaded runtime returns `None` and that diagnostic fires.
pub(crate) fn loft_lib_dir_for_shaped(
    target: Option<&str>,
    shape_subdir: Option<&str>,
) -> Option<std::path::PathBuf> {
    let exe_dir = env::current_exe().ok()?.parent()?.to_path_buf();
    // Dev layout: <project>/target/release/loft  or  <project>/target/debug/loft
    // The wasm rlib lives at <project>/target/wasm32-wasip2/release/
    if let Some(triple) = target {
        // Walk up to find a sibling target/<triple>/release directory.
        let mut dir = exe_dir.clone();
        loop {
            // A shaped runtime lives under its own subdir in BOTH layouts, never
            // beside the default-shape rlib — see the #619 note above.
            let candidate = match shape_subdir {
                Some(sub) => dir
                    .join("target")
                    .join("loft")
                    .join(sub)
                    .join(triple)
                    .join("release"),
                None => dir.join("target").join(triple).join("release"),
            };
            if candidate.join("libloft.rlib").exists() {
                return Some(candidate);
            }
            // Installed layout: <prefix>/share/loft/[<shape>/]<triple>/
            if dir.file_name().is_some_and(|n| n == "bin") {
                let base = dir.parent()?.join("share").join("loft");
                let share = match shape_subdir {
                    Some(sub) => base.join(sub).join(triple),
                    None => base.join(triple),
                };
                if share.join("libloft.rlib").exists() {
                    return Some(share);
                }
            }
            if !dir.pop() {
                break;
            }
        }
        return None;
    }
    // Native: prefer `deps/` over the uplifted `<profile>/libloft.rlib` —
    // the deps copy is what every binary links; the uplifted copy is only
    // refreshed by an explicit `cargo build --lib` and goes stale-by-content
    // whenever another build universe rewrites deps (#304/#307: post-rebase it
    // lacked the new `loft::rpc` module while generated code referenced it,
    // failing every `--native` compile with E0433).  Keep the ordering aligned
    // with `cache::rlib_candidates` and `native_lib::find_loft_rlib`.
    let deps = exe_dir.join("deps");
    if deps.join("libloft.rlib").exists() {
        return Some(deps);
    }
    if exe_dir.join("libloft.rlib").exists() {
        return Some(exe_dir.clone());
    }
    // Installed as <prefix>/bin/loft — look in <prefix>/share/loft/.
    if exe_dir.file_name()? == "bin" {
        let share = exe_dir.parent()?.join("share").join("loft");
        if share.join("libloft.rlib").exists() {
            return Some(share);
        }
    }
    None
}

pub(crate) fn loft_lib_dir() -> Option<std::path::PathBuf> {
    loft_lib_dir_for(None)
}

/// Walk up from the running loft binary to the loft SOURCE tree — the first
/// ancestor directory whose `Cargo.toml` is the `loft` package's own manifest.
/// `Some` in a dev / source checkout (where the native runtime can be rebuilt
/// from source); `None` from an installed bundle (no source to build against).
pub(crate) fn loft_source_tree() -> Option<std::path::PathBuf> {
    let mut dir = env::current_exe().ok()?.parent()?.to_path_buf();
    loop {
        // loft's own manifest carries `name = "loft"`; a consumer crate that
        // merely depends on loft would not, so this never misfires on a user tree.
        if std::fs::read_to_string(dir.join("Cargo.toml"))
            .is_ok_and(|t| t.contains("name = \"loft\""))
        {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

/// loft#706 — does this rustc stderr say the compile failed because a crate it had to
/// LINK could not be resolved or was built by a different rustc, rather than because
/// loft's generated code is wrong?
///
/// The distinction is what makes healing safe: only a toolchain/cache failure may
/// trigger a runtime rebuild + retry, so a genuine codegen bug still surfaces loudly
/// instead of costing the user a minute of rebuild first.
///
/// * `E0514` — "compiled by an incompatible version of rustc" (rlibs are SVH-locked to
///   one rustc, so this is the post-`rustup update` case, and the case where the rlib
///   and the loft binary simply came from different builds);
/// * `E0460` — "possibly newer version of crate" (the same staleness, seen through a
///   transitive dep);
/// * `E0463` — "can't find crate" (a distributed bundle ships no rlib).
///
/// Both halves must match: the bare code is not enough, because `E0463` also fires for
/// a user's own missing `--lib` package, which no rebuild of loft's runtime can fix.
#[must_use]
pub(crate) fn crate_resolution_failure(stderr: &str) -> bool {
    (stderr.contains("E0460") || stderr.contains("E0463") || stderr.contains("E0514"))
        && (stderr.contains("rand_core")
            || stderr.contains("possibly newer version of crate")
            || stderr.contains("compiled by an incompatible version")
            || stderr.contains("can't find crate"))
}

/// Rebuild loft's native runtime rlib (and binary) from `tree` with the user's
/// current rustc, so `--native` keeps working after a `rustc` change instead of
/// falling back to the interpreter.  rlibs are SVH-locked to one rustc, so a
/// `rustup update` strands the cached rlib; this recompiles it against the live
/// rustc.  Prints why the wait happens and how long it took.  Returns whether the
/// rebuild succeeded.  Honours `LOFT_NO_AUTO_REBUILD` (skip → `false`) for CI /
/// users who prefer the interpreter fallback over an implicit `cargo` build.
pub(crate) fn rebuild_runtime(tree: &std::path::Path, reason: &str) -> bool {
    if env::var_os("LOFT_NO_AUTO_REBUILD").is_some() {
        return false;
    }
    eprintln!(
        "loft: native runtime out of date ({reason}).\n\
         Rebuilding it with your rustc — a one-time cost after a rustc change: loft \
         compiles its own runtime crate so your program's generated code can link \
         against it (typically under a minute; longer from a cold build).\n\
         Set LOFT_NO_AUTO_REBUILD=1 to skip this and run on the interpreter instead."
    );
    let start = std::time::Instant::now();
    let ran = std::process::Command::new("cargo")
        .args(["build", "--release", "--lib", "--bin", "loft"])
        .current_dir(tree)
        .status();
    match ran {
        Ok(s) if s.success() => {
            eprintln!(
                "loft: native runtime rebuilt in {:.0}s — continuing natively.",
                start.elapsed().as_secs_f64()
            );
            true
        }
        Ok(_) => {
            eprintln!("loft: native runtime rebuild failed (see cargo output above).");
            false
        }
        Err(e) => {
            eprintln!("loft: could not run cargo to rebuild the native runtime ({e}).");
            false
        }
    }
}

/// @PLN100 Slice 1 — a wasm "shape" loft's own runtime rlib (`libloft.rlib`) is
/// built in: a (triple × feature-set) pair.  `--html` and `--native-wasm` each
/// need loft's runtime compiled for their target with a SPECIFIC feature set, and
/// the `--html` shape MUST NOT share an output directory with the `make wasm`
/// (wasm-bindgen) shape — both are `wasm32-unknown-unknown` but with incompatible
/// features, so the last writer's `libloft.rlib` silently stomps the other (the
/// rlib-stomp, WASM.md § The rlib-stomp hazard; the inline guard is
/// [`html_wasm_import_modules_ok`]).  Giving the `--html` shape its own directory
/// under `target/loft/html/` removes the shared file, so the stomp cannot happen.
#[derive(Clone, Copy)]
pub(crate) enum WasmRuntimeShape {
    /// `loft --html`: `wasm32-unknown-unknown`, raw `loft_gl`/`loft_io` externs
    /// (`--features random`, no wasm-bindgen).  Isolated under `target/loft/html/`
    /// so the wasm-bindgen `make wasm` build (same triple) can't stomp it.
    Html,
    /// @PLN117 — `loft --html` for a program that uses `par`: the [`Html`](Self::Html)
    /// shape plus loft's browser threading (`wasm-native-threads`), which needs a
    /// std recompiled with wasm atomics.  Only cargo can produce that (`-Z
    /// build-std`, hence a nightly toolchain), and it lands beside the rlib in this
    /// shape's own directory, where the `--html` link picks it up as a sysroot.
    HtmlThreads,
    /// `loft --native-wasm`: `wasm32-wasip2`, `--features random`.  Its triple has
    /// only ONE shape, so it is not stomped and keeps the default `target/` output
    /// the install/test paths (`make install-artifacts`, the wasm_library_suite)
    /// already read.
    Wasi,
}

/// @PLN117 — the `-C` flags a threaded browser wasm must be built with, shared by
/// the runtime-rlib build and the final `--html` link (`make wasm-mt` passes the
/// same set).  This toolchain's rustc does NOT derive the memory and TLS flags
/// from `+atomics`, and dropping any one of them yields a bundle that builds but
/// whose workers die at runtime with *"Memory could not be cloned"*.
///
/// `--export=__stack_pointer` is loft's own addition (@PLN117 amendment A3): every
/// `WebAssembly.Instance` starts with the same stack pointer, so the worker
/// bootstrap must move each worker onto its own shadow stack before it runs any
/// wasm.  wasm-bindgen's thread transform did this internally; the raw path does
/// it from JS, which needs the global exported.
/// What to install when the threaded browser build cannot compile its std.
pub(crate) const ATOMICS_STD_TOOLCHAIN_HINT: &str = "  Browser threading rebuilds std with wasm atomics, which needs the nightly \
     toolchain and its sources:\n    rustup toolchain install nightly\n    rustup component \
     add rust-src --toolchain nightly\n  Or build without threads: loft --html --no-threads \
     <program>.loft";

pub(crate) const WASM_THREAD_FLAGS: &[&str] = &[
    "-Ctarget-feature=+atomics,+bulk-memory,+mutable-globals",
    "-Clink-arg=--shared-memory",
    "-Clink-arg=--max-memory=1073741824",
    "-Clink-arg=--import-memory",
    "-Clink-arg=--export=__heap_base",
    "-Clink-arg=--export=__wasm_init_tls",
    "-Clink-arg=--export=__tls_size",
    "-Clink-arg=--export=__tls_align",
    "-Clink-arg=--export=__tls_base",
    "-Clink-arg=--export=__stack_pointer",
];

impl WasmRuntimeShape {
    /// The rustc/rustup target triple this shape compiles for.
    pub(crate) fn triple(self) -> &'static str {
        match self {
            WasmRuntimeShape::Html | WasmRuntimeShape::HtmlThreads => "wasm32-unknown-unknown",
            WasmRuntimeShape::Wasi => "wasm32-wasip2",
        }
    }

    /// Short name for diagnostics / the isolated directory.
    pub(crate) fn name(self) -> &'static str {
        match self {
            WasmRuntimeShape::Html => "html",
            WasmRuntimeShape::HtmlThreads => "html-mt",
            WasmRuntimeShape::Wasi => "wasi",
        }
    }

    /// Does this shape need a std recompiled with wasm atomics (`-Z build-std`,
    /// nightly)?  Only the threaded browser shape does.
    pub(crate) fn needs_atomics_std(self) -> bool {
        matches!(self, WasmRuntimeShape::HtmlThreads)
    }

    /// The cargo `--features` this shape's runtime rlib is built with (always
    /// `--no-default-features`).  Both current shapes use `random` only; the
    /// wasm-bindgen (`wasm`) shape is a Makefile / `wasm-pack` concern never
    /// emitted by the loft binary, so it needs no arm here.
    fn features(self) -> &'static str {
        match self {
            WasmRuntimeShape::Html | WasmRuntimeShape::Wasi => "random",
            WasmRuntimeShape::HtmlThreads => "random wasm-native-threads",
        }
    }

    /// The isolated cargo `--target-dir` (relative to the loft source tree) this
    /// shape's rlib is built into — `Some` only when the shape shares its triple
    /// with another, feature-incompatible shape and so needs its own directory.
    /// `None` shapes use the default `target/` the install/test paths already read.
    fn isolated_target_subdir(self) -> Option<&'static str> {
        match self {
            WasmRuntimeShape::Html => Some("target/loft/html"),
            WasmRuntimeShape::HtmlThreads => Some("target/loft/html-mt"),
            WasmRuntimeShape::Wasi => None,
        }
    }

    /// #619 — the directory this shape's runtime is looked up under, RELATIVE to
    /// the triple, in an installed bundle (`<prefix>/share/loft/<subdir>/<triple>/`)
    /// and in a source tree (`target/loft/<subdir>/<triple>/release/`).
    ///
    /// `Html` and `HtmlThreads` share `wasm32-unknown-unknown`, so the triple alone
    /// cannot tell them apart: a triple-only lookup handed a request for the
    /// threaded runtime the single-threaded rlib, silently.  Only the shape that
    /// needs distinguishing carries a subdir; the default shapes keep the flat
    /// layout `make install` has always written, so nothing existing moves.
    fn install_subdir(self) -> Option<&'static str> {
        match self {
            WasmRuntimeShape::HtmlThreads => Some("html-mt"),
            WasmRuntimeShape::Html | WasmRuntimeShape::Wasi => None,
        }
    }
}

/// @PLN100 Slice 1 — ensure loft's own runtime rlib (`libloft.rlib`) for `shape`
/// exists and is up to date, building it on demand, and return the directory that
/// holds it (for `--extern loft=<dir>/libloft.rlib` + `-L dependency=<dir>/deps`).
///
/// This is the runtime-rlib analogue of [`crate::extensions::auto_build_native_target`]
/// (which does the same for a user PACKAGE's native crate): in a dev checkout it
/// auto-builds the rlib into a per-shape directory keyed on the running loft
/// build's content fingerprint, so `--html` / `--native-wasm` no longer need a
/// manual `make` step and never link a stale or feature-mismatched (`make
/// wasm`-stomped) rlib.  An installed loft (no source tree) has no cargo/source to
/// build with, so it falls back to the shipped per-triple rlib via
/// [`loft_lib_dir_for`].
///
/// Staleness is keyed on [`loft::cache::loft_build_fingerprint`] (the content
/// hash of the running loft's own rlib): it flips whenever loft's source changes
/// and loft is rebuilt, which is exactly when the wasm runtime rlib — compiled
/// from the same source — must be rebuilt.  Content-based, so it survives the
/// mtime resets a CI `target/` cache restore causes.
pub(crate) fn ensure_loft_runtime_rlib(shape: WasmRuntimeShape) -> Option<std::path::PathBuf> {
    let triple = shape.triple();
    let Some(tree) = loft_source_tree() else {
        // Installed bundle: no source to compile — use the shipped rlib as-is.
        // #619 — keyed on the SHAPE, not just the triple: `Html` and `HtmlThreads`
        // share `wasm32-unknown-unknown`, and answering a threaded request with the
        // single-threaded rlib is what produced a page annotated as threaded with no
        // threading in it.  `None` here reaches the caller's existing
        // "could not build the THREADED browser runtime" diagnostic, which then
        // falls back to the single-threaded shape with `threaded = false` — the
        // honest outcome, and the one the artifact then reflects.
        return loft_lib_dir_for_shaped(Some(triple), shape.install_subdir());
    };
    // The rlib lands under <target-dir>/<triple>/release/.  The Html shape gets an
    // isolated target-dir so the wasm-bindgen `make wasm` build (same triple)
    // can't stomp it; other shapes use the default `target/`.
    let target_dir = shape
        .isolated_target_subdir()
        .map_or_else(|| tree.join("target"), |sub| tree.join(sub));
    let profile_dir = target_dir.join(triple).join("release");
    let rlib = profile_dir.join("libloft.rlib");
    let fp = loft::cache::loft_build_fingerprint();
    let fresh = |dir: &std::path::Path| {
        dir.join("libloft.rlib").exists()
            && loft::cache::native_artifact_fingerprint_matches(dir, fp)
    };
    if fresh(&profile_dir) {
        crate::platform::timing_record("wasmrlib", shape.name(), true, None);
        return Some(profile_dir);
    }
    // loft#1238 — say WHY this build is about to run.  The two reasons are not the same
    // problem: a MISSING rlib is a first build on this checkout and costs what it costs, while
    // a STALE one is the fingerprint moving because loft itself was rebuilt — which happens on
    // every commit, so the first wasm/html run after any source change pays a full cargo build.
    // `loft_build_fingerprint` is a content hash of libloft.rlib / the loft executable, so this
    // is not tunable by the caller; it is the price of the correctness guarantee that a codegen
    // change reaches an already-built artifact.
    let why = if rlib.exists() {
        "loft's own build fingerprint moved (loft was rebuilt) — rlib is stale"
    } else {
        "no rlib for this shape yet — first build on this checkout"
    };
    if env::var_os("LOFT_NO_AUTO_REBUILD").is_some() {
        // Opt-out (CI / interpreter-preferring users): link whatever rlib is
        // present rather than trigger an implicit cargo build.
        return rlib.exists().then_some(profile_dir);
    }
    // Serialise cross-process builds on the SAME global lock the package
    // auto-build uses, so parallel `loft` invocations (e.g. the concurrent
    // html_wasm / html_asyncify test binaries) don't race cargo's shared registry
    // index/cache or each other's output dir.
    let _build_lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(std::env::temp_dir().join("loft-native-build.lock"))
        .ok();
    if let Some(f) = &_build_lock {
        // loft#1238 — the wait is timed separately from the build.  Under a parallel test
        // runner every wasm-shaped process reaches this lock at once after a loft rebuild, so
        // the wall-clock a single run reports can be almost entirely SOMEONE ELSE'S build; a
        // report that folded the two together would name cargo for time cargo did not spend.
        // loft#1238 — name the wait to the TIMEOUT as well as to the ledger: a process killed
        // here is queued behind another's cold build, and its kill message used to say
        // `phase=parse`.
        let waiting = format!(
            "the global native-build lock (for the {} wasm runtime rlib)",
            shape.name()
        );
        // loft#1238 — bounded by the run's remaining budget; see `timeout::lock_within_budget`.
        let mut gave_up = None;
        crate::platform::timing_exec(
            "lock",
            shape.name(),
            "waiting for the global build lock",
            || {
                if let Err(waited) = loft::timeout::lock_within_budget(f, &waiting) {
                    gave_up = Some(waited);
                }
            },
        );
        if let Some(waited) = gave_up {
            eprintln!(
                "loft: gave up waiting for {waiting} after {:.1}s — another process is \
                 building it and this run's timeout would expire first.\n  Build it once up \
                 front (`loft cache warm`), raise the timeout, or re-run when that build has \
                 finished.",
                waited.as_secs_f64()
            );
            return rlib.exists().then_some(profile_dir);
        }
    }
    if fresh(&profile_dir) {
        // A process we waited on just produced it.
        crate::platform::timing_record("wasmrlib", shape.name(), true, None);
        return Some(profile_dir);
    }
    eprintln!(
        "loft: building loft's wasm runtime rlib for {triple} (the '{}' shape) — a \
         one-time cost until loft's source changes (typically under a minute).  Set \
         LOFT_NO_AUTO_REBUILD=1 to skip and link the existing rlib instead.",
        shape.name()
    );
    // @PLN117 — the threaded shape needs a std compiled with wasm atomics, which
    // only `-Z build-std` produces, which only nightly accepts: run cargo through
    // `rustup run nightly`.  Every other shape runs the default toolchain's cargo.
    let mut cmd = if shape.needs_atomics_std() {
        let mut c = std::process::Command::new("rustup");
        c.args(["run", "nightly", "cargo"]);
        c
    } else {
        std::process::Command::new("cargo")
    };
    cmd.args([
        "build",
        "--release",
        "--target",
        triple,
        "--lib",
        "--no-default-features",
        "--features",
        shape.features(),
    ])
    .current_dir(&tree)
    // CLEAN flags: the host RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS loft was built with
    // are host-target-specific (e.g. target-cpu) and would break or mis-key the
    // wasm build — this rlib is target-defined, not host-flag-defined.
    .env_remove("RUSTFLAGS")
    .env_remove("CARGO_ENCODED_RUSTFLAGS")
    .stdout(std::process::Stdio::inherit())
    .stderr(std::process::Stdio::inherit());
    if shape.needs_atomics_std() {
        // std itself must carry the atomics/TLS ABI, so it is rebuilt from source
        // with the SAME flags as loft's rlib and the final link — mixing an
        // atomics rlib with a non-atomics std does not link at all (lld: "shared
        // memory is disallowed by std ... not compiled with 'atomics'").
        cmd.arg("-Zbuild-std=panic_abort,std")
            .env("RUSTFLAGS", WASM_THREAD_FLAGS.join(" "));
    }
    if let Some(sub) = shape.isolated_target_subdir() {
        cmd.arg("--target-dir").arg(tree.join(sub));
    }
    let subject = format!("libloft.rlib ({triple})");
    let status = crate::platform::timing_exec("cargo", &subject, why, || cmd.status());
    match status {
        Ok(s) if s.success() && rlib.exists() => {
            loft::cache::write_native_artifact_fingerprint(&profile_dir, fp);
            Some(profile_dir)
        }
        Ok(_) => {
            eprintln!(
                "loft: could not build loft's wasm runtime rlib for {triple} — install the \
                 target (`rustup target add {triple}`), or run with --interpret."
            );
            if shape.needs_atomics_std() {
                eprintln!("{}", ATOMICS_STD_TOOLCHAIN_HINT);
            }
            rlib.exists().then_some(profile_dir)
        }
        Err(e) => {
            eprintln!(
                "loft: cannot run cargo to build the wasm runtime rlib for {triple}: {e} — \
                 needs a Rust toolchain with the {triple} target."
            );
            if shape.needs_atomics_std() {
                eprintln!("{}", ATOMICS_STD_TOOLCHAIN_HINT);
            }
            rlib.exists().then_some(profile_dir)
        }
    }
}

/// @PLN117 — the `rustc` a `--html` wasm is compiled with.
///
/// Plain `rustc` normally; with an atomics sysroot (a threaded page) it is
/// nightly's, plus the flags that build shared-memory wasm.  Both are forced by
/// the same fact: that sysroot's rlibs were produced by nightly, and an rlib
/// links only with the compiler that built it.
pub(crate) fn wasm_rustc(atomics_sysroot: Option<&std::path::Path>) -> std::process::Command {
    let Some(sysroot) = atomics_sysroot else {
        return std::process::Command::new("rustc");
    };
    let mut cmd = std::process::Command::new("rustup");
    cmd.args(["run", "nightly", "rustc"]);
    cmd.arg("--sysroot").arg(sysroot);
    cmd.args(WASM_THREAD_FLAGS);
    cmd
}

/// @PLN117 — assemble the atomics-compiled sysroot the threaded `--html` link
/// needs, and return its root.
///
/// `-Z build-std` recompiles std with wasm atomics, but that is a *cargo* flag
/// and `--html` links with `rustc` directly (one loft copy, shared with the wasm
/// bridge crates — the reason it does not go through cargo).  The two meet here:
/// [`WasmRuntimeShape::HtmlThreads`]'s cargo build leaves the atomics std beside
/// loft's rlib, and copying those rlibs into the layout rustc expects turns them
/// into a `--sysroot` the direct link can use.  Without it the link fails outright
/// (*"--shared-memory is disallowed by std … not compiled with 'atomics'"*), so
/// the failure mode is loud, never a silently single-threaded page.
///
/// `profile_dir` is what [`ensure_loft_runtime_rlib`] returned for that shape.
/// Copies only what is missing or has changed size, so repeat builds are cheap.
pub(crate) fn ensure_atomics_sysroot(profile_dir: &std::path::Path) -> Option<std::path::PathBuf> {
    // Every layout's dep dirs, not `<profile>/deps` alone.  Under the per-unit layout
    // cargo adopted on 2026-07-29 that directory does not exist, `read_dir` returned
    // ENOENT, and the sysroot silently did not get assembled — which does NOT fail the
    // link outright as the doc comment above promises.  It drops `--html --threads` back
    // to the default rustc, and the nightly-built rlib then fails to link against it
    // with `E0514 ... compiled by an incompatible version of rustc`: an error naming the
    // wrong subject entirely, two steps from its cause.
    let deps = dep_search_dirs(profile_dir);
    // Candidate roots, in order.  #619: the derived one is the DEV layout
    // (`<target-dir>/<triple>/release` -> `<target-dir>/sysroot`) and stays
    // first so a repo build is unchanged.  In an INSTALLED layout the same
    // derivation lands on `<prefix>/share/loft`, which is root-owned — the
    // assembly failed there and `--html --threads` fell back to a
    // single-threaded page.  The sysroot is a pure CACHE (copies of rlibs that
    // are already installed), so a user-writable fallback is enough; nothing
    // has to be shipped in it.
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    if let Some(target_dir) = profile_dir.parent().and_then(std::path::Path::parent) {
        roots.push(target_dir.join("sysroot"));
    }
    roots.push(user_cache_sysroot_dir());
    let mut last_err = None;
    for sysroot in roots {
        match assemble_atomics_sysroot(&deps, &sysroot) {
            Ok(()) => return Some(sysroot),
            Err(e) => last_err = Some(e),
        }
    }
    if let Some(e) = last_err {
        eprintln!(
            "loft: could not assemble the wasm atomics sysroot ({e}) — \
             `--html --threads` cannot link a shared-memory bundle without it."
        );
    }
    None
}

/// `~/.loft/wasm-atomics-sysroot`, honouring `LOFT_HOME` like the rest of loft's
/// user state.  Falls back to the temp dir when there is no home at all, so an
/// unusual environment degrades to a rebuild-per-run rather than a failure.
fn user_cache_sysroot_dir() -> std::path::PathBuf {
    std::env::var_os("LOFT_HOME")
        .map(std::path::PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(std::env::temp_dir)
        .join(".loft")
        .join("wasm-atomics-sysroot")
}

/// Copy the atomics-compiled rlibs from `deps` into the `lib/rustlib/<triple>/lib`
/// layout `rustc --sysroot` expects, under `sysroot`.
///
/// Copies only what is missing or has changed size, so repeat builds are cheap.
fn assemble_atomics_sysroot(
    dep_dirs: &[std::path::PathBuf],
    sysroot: &std::path::Path,
) -> std::io::Result<()> {
    let lib_dir = sysroot
        .join("lib")
        .join("rustlib")
        .join("wasm32-unknown-unknown")
        .join("lib");
    std::fs::create_dir_all(&lib_dir)?;
    let mut read_any = false;
    for deps in dep_dirs {
        let Ok(entries) = std::fs::read_dir(deps) else {
            continue;
        };
        read_any = true;
        for entry in entries.flatten() {
            let src = entry.path();
            if src.extension().is_none_or(|e| e != "rlib") {
                continue;
            }
            // The `.rmeta` beside it comes too.  A toolchain that stops embedding full
            // metadata in the rlib leaves a STUB there, and rustc then refuses the
            // sysroot with "only metadata stub found for `rlib` dependency `std` —
            // please provide path to the corresponding .rmeta file with full metadata".
            // What follows is not a link error but a resolution one: the prelude import
            // cannot resolve, so `String`, `Ok`, `to_string`, `i32::from` and the `todo!`
            // macro all go missing at once and the failure reads as eighteen unrelated
            // codegen faults in the generated program.  `--native` learned this already
            // (loft#1030's rlib/rmeta pairing); the threaded browser sysroot had not.
            //
            // Paired by IDENTITY — the same `lib<name>-<hash>` stem, not merely a
            // neighbour in the directory — so a stale `.rmeta` from another build cannot
            // be adopted by a fresh rlib.
            copy_sibling_rmeta(&src, &lib_dir)?;
            let Some(name) = src.file_name() else {
                continue;
            };
            // loft's own rlibs stay out: the link names them with `--extern` and
            // `-L dependency=`, and a third copy on the sysroot search path is how
            // rustc ends up reporting multiple candidates for one crate.
            if name.to_string_lossy().starts_with("libloft") {
                continue;
            }
            let dst = lib_dir.join(name);
            let same = std::fs::metadata(&dst)
                .ok()
                .zip(entry.metadata().ok())
                .is_some_and(|(a, b)| a.len() == b.len());
            if !same {
                std::fs::copy(&src, &dst)?;
            }
        }
    }
    if !read_any {
        // Loud, and naming the first place looked in: an unassembled sysroot degrades
        // the link to the default rustc rather than failing there, so a silent Ok here
        // surfaces later as someone else's error.
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "no readable dependency directory for the atomics std (looked in {})",
                dep_dirs
                    .first()
                    .map_or_else(|| "nowhere".to_string(), |d| d.display().to_string())
            ),
        ));
    }
    Ok(())
}

/// Copy the `.rmeta` that belongs to `rlib` into `lib_dir`, if one is beside it.
///
/// Identity, not proximity: the metadata file must share the rlib's exact
/// `lib<name>-<hash>` stem.  Absent is fine — a toolchain that still embeds full
/// metadata in the rlib writes no separate `.rmeta`, and the sysroot is complete
/// without it.
fn copy_sibling_rmeta(rlib: &std::path::Path, lib_dir: &std::path::Path) -> std::io::Result<()> {
    let meta = rlib.with_extension("rmeta");
    let Some(name) = meta.file_name() else {
        return Ok(());
    };
    if !meta.is_file() {
        return Ok(());
    }
    let dst = lib_dir.join(name);
    let same = std::fs::metadata(&dst)
        .ok()
        .zip(std::fs::metadata(&meta).ok())
        .is_some_and(|(a, b)| a.len() == b.len());
    if !same {
        std::fs::copy(&meta, &dst)?;
    }
    Ok(())
}

/// The dependency search dir for a [`loft_lib_dir`] result: `lib_dir` itself
/// when it already IS `deps/` (the preferred deps-first resolution, #304/#307),
/// else `lib_dir/deps`.  Appending "deps" unconditionally yields an invalid
/// `…/deps/deps` path that rustc can't search — E0463 "can't find crate" for
/// every transitive dep of libloft (sha2, rand_core, …).
/// Every directory rustc must search to resolve loft's transitive rlibs.
///
/// Two cargo layouts, and a `--native` compile has to work under whichever the user's
/// toolchain produces:
///
/// * **classic** — one `<profile>/deps/` holding every rlib.  One search dir.
/// * **per-unit** (cargo 1.99.0-nightly 2026-07-29 and later) — each crate gets its own
///   `<profile>/build/<crate>/<hash>/out/`, and `deps/` does not exist at all.  ~107
///   search dirs for loft.
///
/// The per-unit layout broke `--native` outright: `-L dependency=<profile>/deps` named a
/// directory that was not there, and rustc reported `E0463: can't find crate for sha2
/// which loft depends on` — a message that points at a missing dependency rather than at
/// a moved one, which is why the nightly ASan gate read as a loft bug for a week.
///
/// Ordered so the classic layout stays a single `-L` (the common case is unchanged) and
/// the fallback engages only when `deps/` is genuinely absent.
pub(crate) fn dep_search_dirs(lib_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let classic = deps_dir_of(lib_dir);
    if classic.is_dir() {
        return vec![classic];
    }
    // Per-unit: <profile>/build/<crate>/<hash>/out/.  Only directories that actually
    // hold a linkable crate are worth passing — build-script `out` dirs sit in the same
    // tree and carry generated sources, not libraries.
    //
    // Both rlibs AND dylibs count: a proc-macro compiles to `.so`/`.dylib`/`.dll`, never
    // `.rlib`, so an rlib-only filter drops every derive crate.  Filtering on rlibs alone
    // moved the failure from `sha2` to `curve25519_dalek_derive` and no further — the
    // same error one crate later, which reads like the fix not working at all.
    let build_root = lib_dir.join("build");
    let mut dirs = Vec::new();
    if let Ok(crates) = std::fs::read_dir(&build_root) {
        for c in crates.flatten() {
            let Ok(hashes) = std::fs::read_dir(c.path()) else {
                continue;
            };
            for h in hashes.flatten() {
                let out = h.path().join("out");
                if std::fs::read_dir(&out).is_ok_and(|rd| {
                    rd.flatten().any(|f| {
                        std::path::Path::new(&f.file_name())
                            .extension()
                            .is_some_and(|e| {
                                ["rlib", "so", "dylib", "dll"]
                                    .iter()
                                    .any(|k| e.eq_ignore_ascii_case(k))
                            })
                    })
                }) {
                    dirs.push(out);
                }
            }
        }
    }
    if dirs.is_empty() {
        // Name the place it was expected, so the diagnostic stays actionable.
        return vec![classic];
    }
    dirs.sort();
    dirs
}

pub(crate) fn deps_dir_of(lib_dir: &std::path::Path) -> std::path::PathBuf {
    if lib_dir.file_name().is_some_and(|n| n == "deps") {
        lib_dir.to_path_buf()
    } else {
        lib_dir.join("deps")
    }
}

/// Parse every `target/<profile>/build/*/output` file and extract
/// `cargo:rustc-link-search=native=<path>` directives, returning the
/// list of directories that should be passed to rustc as
/// `-L native=<path>`.
///
/// Cargo passes these flags automatically when building the loft
/// binary itself, but when loft's `--native` mode invokes rustc
/// directly to compile generated user code we must replicate the
/// link environment by hand.  Without these search paths the
/// Windows link step fails with `LNK1181: cannot open input file
/// 'windows.0.48.5.lib'` because the `windows-targets` crate emits a
/// search path pointing into its registry source directory (not
/// `OUT_DIR`).
///
/// `lib_dir` is either `target/<profile>/` or `target/<profile>/deps/`
/// — both resolve to the same parent build root.
pub(crate) fn build_script_native_lib_dirs(lib_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let target_profile = if lib_dir.file_name().is_some_and(|n| n == "deps") {
        match lib_dir.parent() {
            Some(p) => p,
            None => return Vec::new(),
        }
    } else {
        lib_dir
    };
    let build_root = target_profile.join("build");
    let mut seen: std::collections::HashSet<std::path::PathBuf> = std::collections::HashSet::new();
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(&build_root) else {
        return out;
    };
    for entry in entries.flatten() {
        let output_path = entry.path().join("output");
        let Ok(text) = std::fs::read_to_string(&output_path) else {
            continue;
        };
        for line in text.lines() {
            // Strip optional `cargo::` (1.77+) or `cargo:` (legacy) prefix.
            let body = line
                .strip_prefix("cargo::")
                .or_else(|| line.strip_prefix("cargo:"))
                .unwrap_or(line);
            // Match `rustc-link-search=native=<path>` and the form
            // without an explicit kind (`rustc-link-search=<path>`,
            // which defaults to `all` and includes native).
            let Some(rest) = body.strip_prefix("rustc-link-search=") else {
                continue;
            };
            let path_str = rest.strip_prefix("native=").unwrap_or(rest);
            // Skip framework= / dependency= / crate= kinds — they're
            // unrelated to native lib search.
            if rest.starts_with("framework=")
                || rest.starts_with("dependency=")
                || rest.starts_with("crate=")
            {
                continue;
            }
            let p = std::path::PathBuf::from(path_str);
            if seen.insert(p.clone()) {
                out.push(p);
            }
        }
    }
    out
}

/// Ensure `libloft.rlib` is at least as fresh as the newest `src/*.rs` file.
/// If any source is newer, run `cargo build --lib` to rebuild it.
pub(crate) fn ensure_rlib_fresh() {
    // @P360: this is a dev convenience — rebuild loft's OWN runtime rlib when
    // loft's Rust sources change, for `cargo run`-style use inside the loft
    // repo.  It compares the RELATIVE `src/`/`default/` mtimes, so when loft
    // runs against an external project that merely *has* a `src/` (e.g. a
    // library package's `src/*.loft`), it wrongly fires `cargo build --lib`
    // from a dir with no `Cargo.toml`, printing a confusing
    // "could not find Cargo.toml" error.  Gate the whole thing on a
    // `Cargo.toml` in cwd: `cargo build --lib` needs one there anyway, so if
    // absent we are not in the loft source root — use the shipped rlib as-is.
    if !std::path::Path::new("Cargo.toml").exists() {
        return;
    }
    let Some(lib_dir) = loft_lib_dir() else {
        // No rlib found at all — try building from scratch.
        let _ = std::process::Command::new("cargo")
            .args(["build", "--lib"])
            .status();
        return;
    };
    let rlib = lib_dir.join("libloft.rlib");
    let Ok(rlib_mtime) = std::fs::metadata(&rlib).and_then(|m| m.modified()) else {
        return;
    };
    // Walk src/ for the newest .rs file.
    let newest_src = newest_mtime_in("src");
    // Also check default/*.loft — changes there affect codegen output.
    let newest_default = newest_mtime_in("default");
    let newest = newest_src.max(newest_default);
    if newest.is_some_and(|t| t > rlib_mtime) {
        eprintln!("loft: rebuilding libloft.rlib (source is newer)...");
        let _ = std::process::Command::new("cargo")
            .args(["build", "--lib"])
            .status();
    }
}

/// Return the newest modification time of any file under `dir` (recursive).
pub(crate) fn newest_mtime_in(dir: &str) -> Option<std::time::SystemTime> {
    fn walk(path: &std::path::Path, best: &mut Option<std::time::SystemTime>) {
        let Ok(entries) = std::fs::read_dir(path) else {
            return;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, best);
            } else if let Ok(m) = p.metadata().and_then(|m| m.modified()) {
                *best = Some(best.map_or(m, |b: std::time::SystemTime| b.max(m)));
            }
        }
    }
    let mut best = None;
    walk(std::path::Path::new(dir), &mut best);
    best
}

/// FNV-1a 64-bit hash for native binary cache keys.
pub(crate) fn fnv64(data: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for &b in data {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Build a cache key from generated Rust source and the rlib identity.
pub(crate) fn native_cache_key(
    rs_content: &[u8],
    lib_dir: Option<&std::path::Path>,
    data: Option<&crate::data::Data>,
) -> u64 {
    let mut key = fnv64(rs_content);
    if let Some(ld) = lib_dir {
        let rlib = ld.join("libloft.rlib");
        key ^= fnv64(rlib.to_string_lossy().as_bytes());
        // The main rlib MUST exist when `lib_dir` is Some (`loft_lib_dir` only
        // returns a dir that has it) — `expected: true` so a missing one (a
        // broken resolution) is loud, not a silent 0.
        fold_file_content(&mut key, &rlib, true);
    }
    // @P341: also fold each native PACKAGE rlib's path + content, so rebuilding
    // a library's `#native` crate (`lib/<pkg>/native/...`) invalidates the
    // cached test binary — which links those rlibs via `add_native_extern_flags`.
    // Without this, a cdylib fix is silently masked by a stale cached binary.
    if let Some(d) = data {
        for (crate_name, pkg_dir) in &d.native_packages {
            let rlib_name = format!("lib{}.rlib", crate_name.replace('-', "_"));
            let rlib = std::path::PathBuf::from(pkg_dir)
                .join("native")
                .join("target")
                .join("release")
                .join(&rlib_name);
            key ^= fnv64(rlib.to_string_lossy().as_bytes());
            // A package rlib may legitimately be not-yet-built → `expected:
            // false` (no warning; folds 0, the existing behaviour).
            fold_file_content(&mut key, &rlib, false);
        }
    }
    key
}

/// Fold a file's CONTENT hash into `key` (no-op if the file is missing).
///
/// BUILD2: keyed on bytes, not mtime, so the cache survives across CI runs.
/// `actions/cache` persists `target/` but every CI run reruns `cargo build
/// --release --lib`, which rewrites `libloft.rlib` with a fresh mtime even on a
/// no-op rebuild — an mtime fold then misses every time and recompiles every
/// native fixture from scratch.  rustc's rlib output is byte-deterministic for
/// unchanged sources (verified: a touch-and-rebuild yields an identical
/// sha256), so a content hash is stable across the no-op rebuild → warm-cache
/// hit, while still invalidating when the binary actually changes (different
/// bytes → different hash).  Results are memoised per path so a 14MB rlib is
/// hashed once per process, not once per fixture.
fn fold_file_content(key: &mut u64, path: &std::path::Path, expected: bool) {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    static CACHE: OnceLock<Mutex<HashMap<std::path::PathBuf, u64>>> = OnceLock::new();
    let map = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let hash = {
        let mut guard = map.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(&h) = guard.get(path) {
            h
        } else {
            let h = match std::fs::read(path) {
                Ok(b) => fnv64(&b),
                // A missing file folds to 0 (a not-yet-built PACKAGE rlib
                // legitimately contributes nothing).  But when the caller
                // declared it SHOULD exist (the main `libloft.rlib` this binary
                // links), missing means the cache key cannot reflect the loft
                // build it links → a stale binary may be reused.  Say so loudly;
                // the 0 is memoised, so this fires at most once per path.
                Err(_) => {
                    if expected {
                        eprintln!(
                            "loft: warning — fingerprint input {} is missing; native \
                             staleness detection is degraded (a stale binary may be reused). \
                             Rebuild loft, or run `loft cache prune --all`.",
                            path.display()
                        );
                    }
                    0
                }
            };
            guard.insert(path.to_path_buf(), h);
            h
        }
    };
    *key ^= hash;
}

/// Return true if `s` looks like an explicit output path rather than a flag or loft source file.
pub(crate) fn is_output_path(s: &str) -> bool {
    !s.starts_with('-')
        && !std::path::Path::new(s)
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("loft"))
}

/// Return (and create) the `.loft/` artifact directory beside `script_path`.
/// Falls back to the current directory's `.loft/` if the parent cannot be determined.
pub(crate) fn loft_artifact_dir(script_path: &str) -> std::path::PathBuf {
    let dir = std::path::Path::new(script_path)
        .parent()
        .unwrap_or(std::path::Path::new("."));
    let loft_dir = dir.join(".loft");
    let _ = std::fs::create_dir_all(&loft_dir);
    loft_dir
}

/// Return the default output path for a compiled artifact beside `script_path`.
/// `ext` is the file extension without leading dot (e.g. `"wasm"`, `"rs"`).
pub(crate) fn default_artifact_path(script_path: &str, ext: &str) -> std::path::PathBuf {
    let stem = std::path::Path::new(script_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("out");
    loft_artifact_dir(script_path).join(format!("{stem}.{ext}"))
}

/// @P350: parse the import section of a `loft --html` wasm and verify every
/// import module is one of the raw extern bridges the embedded HTML glue
/// provides (`loft_gl` / `loft_io`).  A wasm-bindgen-feature rlib — the
/// "rlib stomp" `make wasm` leaves at
/// `target/wasm32-unknown-unknown/release/libloft.rlib` — imports
/// `__wbindgen_placeholder__` (35+), which the glue cannot satisfy, so the
/// page never instantiates.  Returns `Ok(())` when every import module is
/// acceptable (a wasm with no imports included), or `Err(sorted distinct bad
/// module names)` otherwise.  Mirrors the import check in
/// `tools/check_html_bundle.mjs`, inline so it guards a bare `loft --html`
/// and not only the `make game` path.
///
/// The parser is deliberately conservative: on ANY shape it doesn't
/// understand (bad magic, truncated section, unknown import-descriptor kind)
/// it returns `Ok(())` rather than risk a false abort on a valid bundle — the
/// only path that errors is one where it successfully read import module
/// names and found one that is not `loft_gl`/`loft_io`.
pub(crate) fn html_wasm_import_modules_ok(wasm: &[u8]) -> Result<(), Vec<String>> {
    let Some(imports) = html_wasm_imports(wasm) else {
        return Ok(());
    };
    // loft's own host imports (`loft_io` / `loft_gl` / @PLN117's `loft_thread`) AND
    // library-bridge host imports (e.g. the crypto bridge's `loft_crypto.random_fill`)
    // all use a `loft_` prefix.  A wasm-bindgen stomp instead imports `__wbindgen_*`,
    // so a non-`loft_` module is the red flag this guards.  The one exception is the
    // shared MEMORY a threaded build imports as `env.memory` (kind 2) — still no
    // function may come from `env`.
    let bad: std::collections::BTreeSet<String> = imports
        .iter()
        .filter(|(m, _, kind, _)| !(m.starts_with("loft_") || (m == "env" && *kind == 2)))
        .map(|(m, _, _, _)| m.clone())
        .collect();
    if bad.is_empty() {
        Ok(())
    } else {
        Err(bad.into_iter().collect())
    }
}

/// Every `(module, field, descriptor-kind, result-valtype)` a wasm imports, or `None`
/// if it cannot be walked (bad magic, truncated section, unknown descriptor kind).
///
/// The result valtype is the wasm byte (`0x7F` i32, `0x7E` i64, `0x7D` f32, `0x7C`
/// f64) of a function import's single return value, and `None` for a function that
/// returns nothing or for a non-function import.  [`host_import_stub_js`] needs it:
/// a JS stub standing in for a missing host function has to hand back a value of the
/// declared type, and `undefined` where the module expects an i64 is a TypeError at
/// the call rather than the zero the caller can check.
///
/// The ONE import-section walk.  Four callers read it — the `--html` bundle guard
/// ([`html_wasm_import_modules_ok`]), the page-shell chooser
/// ([`html_wasm_import_modules`]), the host-surface check
/// ([`missing_host_imports`]), and the stub emitter — and they used to carry a copy
/// each of the LEB/name/limits decoding.
///
/// Deliberately conservative: on any shape it does not understand it returns `None`
/// rather than risk a false verdict on a valid bundle.  Callers treat `None` as "no
/// opinion", never as "nothing imported".
pub(crate) fn html_wasm_imports(
    wasm: &[u8],
) -> Option<std::collections::BTreeSet<(String, String, u8, Option<u8>)>> {
    fn read_uleb(b: &[u8], p: &mut usize) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = *b.get(*p)?;
            *p += 1;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }
    fn read_name<'a>(b: &'a [u8], p: &mut usize) -> Option<&'a [u8]> {
        let len = usize::try_from(read_uleb(b, p)?).ok()?;
        let s = b.get(*p..p.checked_add(len)?)?;
        *p += len;
        Some(s)
    }
    fn skip_limits(b: &[u8], p: &mut usize) -> Option<()> {
        let flag = *b.get(*p)?;
        *p += 1;
        read_uleb(b, p)?; // min
        if flag & 1 == 1 {
            read_uleb(b, p)?; // max
        }
        Some(())
    }
    // Header: magic "\0asm" + version.
    if wasm.len() < 8 || &wasm[0..4] != b"\0asm" {
        return None;
    }
    let mut p = 8;
    let mut out = std::collections::BTreeSet::new();
    // Result valtype per type index, filled from the type section.  Wasm orders
    // the known sections, so type (1) is already read when import (2) arrives.
    let mut results: Vec<Option<u8>> = Vec::new();
    while p < wasm.len() {
        let id = *wasm.get(p)?;
        p += 1;
        let size = usize::try_from(read_uleb(wasm, &mut p)?).ok()?;
        let section_start = p;
        let section_end = section_start
            .checked_add(size)
            .filter(|e| *e <= wasm.len())?;
        if id == 1 {
            // Type section: u32 count, then `count` functypes
            // (0x60, params vec, results vec).
            let mut tp = section_start;
            let count = read_uleb(wasm, &mut tp)?;
            for _ in 0..count {
                if *wasm.get(tp)? != 0x60 {
                    return None; // not a functype — no opinion
                }
                tp += 1;
                let params = usize::try_from(read_uleb(wasm, &mut tp)?).ok()?;
                tp = tp.checked_add(params)?;
                let n_res = usize::try_from(read_uleb(wasm, &mut tp)?).ok()?;
                results.push(if n_res == 0 {
                    None
                } else {
                    Some(*wasm.get(tp)?)
                });
                tp = tp.checked_add(n_res)?;
            }
        }
        if id == 2 {
            // Import section: u32 count, then `count` (module, field, desc).
            let mut ip = section_start;
            let count = read_uleb(wasm, &mut ip)?;
            for _ in 0..count {
                let module = read_name(wasm, &mut ip)?;
                let field = read_name(wasm, &mut ip)?;
                let kind = *wasm.get(ip)?;
                ip += 1;
                let mut result = None;
                match kind {
                    0 => {
                        let idx = usize::try_from(read_uleb(wasm, &mut ip)?).ok()?; // func: typeidx
                        result = results.get(idx).copied().flatten();
                    }
                    1 => {
                        ip += 1; // table: reftype byte, then limits
                        skip_limits(wasm, &mut ip)?;
                    }
                    2 => skip_limits(wasm, &mut ip)?, // mem: limits
                    3 => ip += 2,                     // global: valtype + mut byte
                    _ => return None,                 // unknown kind — no opinion
                }
                out.insert((
                    String::from_utf8_lossy(module).into_owned(),
                    String::from_utf8_lossy(field).into_owned(),
                    kind,
                    result,
                ));
            }
        }
        p = section_end;
    }
    Some(out)
}

/// loft#954 — how many of the module's own functions the `name` custom section
/// names, or `None` when there is no such section.
///
/// This is what makes `--names` checkable rather than hopeful.  The section is
/// what turns the `wasm-function[1073]` frames a browser prints for a trap into
/// function names, and it survives only if binaryen was asked for it (`-g`) and
/// not asked to drop it (`--strip-debug`).  A build that quietly emitted no
/// section would hand over a page whose backtrace is exactly as unreadable as
/// before, with nothing to say so.
///
/// Counts the whole function subsection, imports included — the caller wants to
/// know whether names are THERE, and the import names are present in any build
/// because linking needs them.  Conservative like its neighbours: any shape it
/// cannot walk reads as `None`.
pub(crate) fn html_wasm_named_functions(wasm: &[u8]) -> Option<usize> {
    fn read_uleb(b: &[u8], p: &mut usize) -> Option<u64> {
        let mut result: u64 = 0;
        let mut shift = 0u32;
        loop {
            let byte = *b.get(*p)?;
            *p += 1;
            result |= u64::from(byte & 0x7f) << shift;
            if byte & 0x80 == 0 {
                return Some(result);
            }
            shift += 7;
            if shift >= 64 {
                return None;
            }
        }
    }
    if wasm.len() < 8 || &wasm[0..4] != b"\0asm" {
        return None;
    }
    let mut p = 8;
    while p < wasm.len() {
        let id = *wasm.get(p)?;
        p += 1;
        let size = usize::try_from(read_uleb(wasm, &mut p)?).ok()?;
        let start = p;
        let end = start.checked_add(size).filter(|e| *e <= wasm.len())?;
        // Custom section (id 0): a name, then the payload.
        if id == 0 {
            let mut q = start;
            let len = usize::try_from(read_uleb(wasm, &mut q)?).ok()?;
            let name = wasm.get(q..q.checked_add(len)?)?;
            q += len;
            if name == b"name" {
                // Subsections, each `id : u8` + `size : uleb` + payload.  Id 1 is
                // the function subsection: a vec of (function index, name).
                while q < end {
                    let sub = *wasm.get(q)?;
                    q += 1;
                    let sub_size = usize::try_from(read_uleb(wasm, &mut q)?).ok()?;
                    let sub_end = q.checked_add(sub_size).filter(|e| *e <= end)?;
                    if sub == 1 {
                        let mut r = q;
                        return usize::try_from(read_uleb(wasm, &mut r)?).ok();
                    }
                    q = sub_end;
                }
                return Some(0);
            }
        }
        p = end;
    }
    None
}

/// JS that gives every `loft_*` host function this wasm imports a stand-in, for
/// the ones the page shim does not define.
///
/// A canvas cannot do everything a desktop window can, and some calls — a
/// screenshot to a `path`, say — may have no browser meaning at all.  That is
/// fine; what is not fine is finding out at BUILD time.  Whether a call can be
/// served is a fact about this run on this target, not about whether the program
/// is well-formed, and refusing the build forces one source to fork into two
/// that differ only in which calls they may NAME — which destroys the property
/// of running the same source on both renderers (loft#709).
///
/// So the page instantiates and the call answers at runtime: the declared zero
/// (`false` / `0` / `""`-shaped null pointer), which every correct caller
/// already handles, since a screenshot can fail for a dozen reasons besides the
/// target.  It says so once per name in the console — once, because a call in a
/// frame loop would otherwise bury the page in identical lines.
///
/// Emitted for EVERY `loft_*` function import and applied only where the name is
/// still free, so the shim and every library bridge keep precedence and a name
/// the build-time scan misjudged is covered anyway.
pub(crate) fn host_import_stub_js(wasm: &[u8]) -> String {
    let Some(imports) = html_wasm_imports(wasm) else {
        return String::new();
    };
    let mut entries = String::new();
    for (module, field, _, result) in imports
        .iter()
        .filter(|(m, _, k, _)| *k == 0 && m.starts_with("loft_"))
    {
        // The value the stub returns, as JS source: wasm coerces the JS result
        // to the declared type, and an i64 result MUST be a BigInt.
        let zero = match result {
            Some(0x7E) => "0n",
            Some(_) => "0",
            None => "undefined",
        };
        entries.push_str(&format!("[{module:?},{field:?},{zero}],"));
    }
    if entries.is_empty() {
        return String::new();
    }
    format!(
        "\n// loft#709: a call this target cannot serve answers at runtime instead of\n\
         // refusing the build.  Only fills names nothing else defined.\n\
         (function(){{const seen=new Set();for(const[m,f,z]of[{entries}]){{\n\
         \x20 const mod=(imports[m]||(imports[m]={{}}));\n\
         \x20 if(f in mod)continue;\n\
         \x20 mod[f]=function(){{ if(!seen.has(f)){{seen.add(f);\n\
         \x20   console.warn('loft: '+f+' is not available in the browser — returning '+String(z));}}\n\
         \x20   return z; }};\n\
         }}}})();\n"
    )
}

/// Host-import names the `--html` page will NOT provide, as `module.field` strings.
///
/// The browser shim implements a SUBSET of `lib/graphics`' native surface — reasonably,
/// since a canvas cannot do everything a desktop window can.  The defect was that the
/// boundary was invisible until the page loaded, and crossing it killed the WHOLE page:
/// `WebAssembly.instantiate` fails with `LinkError: Import #7 "loft_gl"
/// "loft_gl_window_width": function import requires a callable`, so there is no canvas,
/// no output, not even a `println` — and nothing names the loft call responsible
/// (loft#668).
///
/// Both sides are known at build time — the program's imports are in the wasm, and the
/// shim is a fixed file compiled into this binary — so the answer is decidable before the
/// page is ever written.
///
/// `provided_js` is the shim source; a host function is provided iff it appears there as a
/// definition. Reading the shim rather than a hand-kept list is deliberate: a second list
/// would drift from the shim exactly the way `loft install`'s whitelist drifted from
/// `loft package`. Only FUNCTION imports (kind 0) are judged — a threaded build's
/// `env.memory` is supplied by the loader, not the shim.
///
/// Three definition forms count, because JS has three and a library may use any:
/// `name(…) {` (method shorthand), `name:` (property), and `name =` (assignment, as in
/// `ns.ws_yield = function () {…}`). The assignment form was missing, and the omission was
/// not cosmetic — it REJECTED a program whose library bridge did provide the handler, with
/// a message telling the author to add the very handler they had already written
/// (loft#681, `web`'s `[wasm.bridge] host_js`).
///
/// The asymmetry of the two errors decides how strict to be. Failing to recognise a
/// definition BLOCKS a program that would have run; recognising one too eagerly lets a
/// genuinely missing import through to the `LinkError` this check exists to pre-empt —
/// the status quo before it existed. So when the two are in tension, prefer to admit: a
/// false accept costs the diagnostic, a false reject costs the build.
pub(crate) fn missing_host_imports(wasm: &[u8], provided_js: &str) -> Vec<String> {
    let Some(imports) = html_wasm_imports(wasm) else {
        return Vec::new();
    };
    let defined = |name: &str| {
        provided_js.match_indices(name).any(|(i, _)| {
            // The occurrence must be a whole identifier, not a suffix of a longer one.
            if provided_js[..i]
                .chars()
                .next_back()
                .is_some_and(|c| c.is_alphanumeric() || c == '_')
            {
                return false;
            }
            let rest = &provided_js[i + name.len()..];
            let mut after = rest.chars();
            match after.next() {
                // `name(...) {` — a method shorthand; `name:` — a property.
                Some('(' | ':') => true,
                // `name = …` / `name= …` — an assignment.  `==` / `===` is a COMPARISON,
                // not a definition, so it must not count.
                Some('=') => after.next() != Some('='),
                // `name  = …` / `name :` — allow horizontal space before the punctuation
                // only, so a bare mention in prose (`// ws_yield is the shim`) still does
                // not qualify.
                Some(' ' | '\t') => {
                    let t = rest.trim_start_matches([' ', '\t']);
                    t.starts_with(':') || (t.starts_with('=') && !t.starts_with("=="))
                }
                _ => false,
            }
        })
    };
    imports
        .iter()
        .filter(|(m, _, kind, _)| *kind == 0 && m.starts_with("loft_"))
        .filter(|(_, field, _, _)| !defined(field))
        .map(|(m, field, _, _)| format!("{m}.{field}"))
        .collect()
}

/// Return the distinct import-module names of a `loft --html` wasm, or `None`
/// if the module can't be walked (bad magic, truncated/unknown shape).
///
/// `--html` uses this to pick the page shell. A wasm that imports only
/// `loft_io` needs nothing but text I/O, so it gets the minimal engine-less
/// page (a tiny inline shim — no WebGL2, no asyncify, no canvas). Any other
/// module means the program opted into something bigger — `loft_gl` for
/// graphics/audio, or a `loft_<lib>` library bridge — which needs the full
/// engine page. On `None` the caller falls back to the full page, the shell
/// that satisfies every import.
pub(crate) fn html_wasm_import_modules(wasm: &[u8]) -> Option<std::collections::BTreeSet<String>> {
    Some(
        html_wasm_imports(wasm)?
            .into_iter()
            .map(|(m, _, _, _)| m)
            .collect(),
    )
}

pub(crate) fn project_dir() -> String {
    let Ok(prog) = env::current_exe() else {
        return String::new();
    };
    let Some(dir) = prog.parent() else {
        return String::new();
    };
    project_root_for(dir)
}

/// How far above the binary to look for the stdlib.  `target/<triple>/<profile>`
/// is the deepest cargo layout (3 hops); the extra headroom costs one `is_dir`
/// each and keeps a nested workspace target dir working.
const PROJECT_ROOT_SEARCH_DEPTH: usize = 6;

/// Resolve the project root — the directory holding the stdlib `default/` — from
/// the directory the running binary sits in.
///
/// Split out from [`project_dir`] so every layout can be tested without planting a
/// binary in each one.
fn project_root_for(dir: &std::path::Path) -> String {
    // Strip target/release or target/debug to get the project root.
    if (dir.ends_with("target/release") || dir.ends_with("target\\release"))
        && let Some(root) = dir.parent().and_then(|p| p.parent())
    {
        return with_trailing_sep(root);
    }
    if (dir.ends_with("target/debug") || dir.ends_with("target\\debug"))
        && let Some(root) = dir.parent().and_then(|p| p.parent())
    {
        return with_trailing_sep(root);
    }
    // Installed binary: binary is in <prefix>/bin/, stdlib in <prefix>/share/loft/.
    if dir.ends_with("bin")
        && let Some(prefix) = dir.parent()
    {
        let share_loft = prefix.join("share").join("loft");
        if share_loft.is_dir() {
            return with_trailing_sep(&share_loft);
        }
        return with_trailing_sep(prefix);
    }
    // Any other cargo layout.  `--target <triple>` nests the profile one level
    // deeper (`target/<triple>/release`), a custom profile renames it
    // (`target/<profile>`), and `CARGO_TARGET_DIR` renames `target` itself
    // (`target-da/release`).  Rather than enumerate them, walk up to the first
    // ancestor that actually HOLDS the stdlib — the question every caller is
    // really asking.  Without this a `--target` build resolves its own directory
    // as the project root, so `default/` is never found and every stdlib load
    // fails with "cannot load default library" (loft-lang/loft#638: the ASan
    // sweep is the one CI job that builds with `--target`).
    for ancestor in std::iter::successors(Some(dir), |d| d.parent()).take(PROJECT_ROOT_SEARCH_DEPTH)
    {
        if ancestor.join("default").is_dir() {
            return with_trailing_sep(ancestor);
        }
    }
    with_trailing_sep(dir)
}

/// P254 — cache-poisoning defense.  Returns true when a cached
/// native binary at `path` is safe to execute under the current
/// uid.  All platforms reject symlinks (a symlink lets an attacker
/// redirect execution to anything they can name).  On Unix we
/// additionally require:
///
/// - The file owner equals the current effective uid (root-owned
///   files are also rejected when running as a non-root user — a
///   common shared-machine attack drops a SUID binary owned by
///   root).
/// - No group/other permission bits set (mode `& 0o077 == 0`).
///
/// Failed checks return false WITHOUT a side effect; the caller
/// recompiles and overwrites the suspect cache file.  We do not
/// `eprintln!` from the helper so unit tests can assert the
/// boolean cleanly; the recompile path's `eprintln!` covers user
/// visibility.
#[must_use]
pub(crate) fn cache_safe_to_execute(path: &std::path::Path) -> bool {
    // symlink_metadata reports the link itself rather than the
    // target; if the target is owned by the current uid but the
    // link itself isn't, plain metadata() would say "safe" and
    // we'd execute attacker-pointed code.
    let lmd = match std::fs::symlink_metadata(path) {
        Ok(md) => md,
        Err(_) => return false,
    };
    if lmd.file_type().is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if lmd.uid() != unsafe { libc::geteuid() } {
            return false;
        }
        // Reject group/other permissions — only owner may read,
        // write, or execute.  An attacker with group access
        // could swap the file between our stat and exec; even
        // group-readable files leak compiled output that may
        // contain secrets.
        if lmd.mode() & 0o077 != 0 {
            return false;
        }
        // Reject SUID/SGID — cached binaries should never carry
        // privilege-escalation bits.
        if lmd.mode() & 0o6000 != 0 {
            return false;
        }
    }
    true
}

/// P254 — companion check for the cache directory itself.
/// Same shape as `cache_safe_to_execute` but applied to the
/// directory: symlink rejected; on Unix, owner-uid match and
/// no group/other bits required.  Used at cache-write time —
/// if the directory exists with weak permissions, we tighten
/// it via `tighten_cache_dir` before writing.
#[must_use]
pub(crate) fn cache_dir_safe(dir: &std::path::Path) -> bool {
    let lmd = match std::fs::symlink_metadata(dir) {
        Ok(md) => md,
        Err(_) => return false,
    };
    if lmd.file_type().is_symlink() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if lmd.uid() != unsafe { libc::geteuid() } {
            return false;
        }
        if lmd.mode() & 0o077 != 0 {
            return false;
        }
    }
    true
}

/// P254 — set the cache directory's mode to `0o700` on Unix.
/// No-op on non-Unix (NTFS / ReFS use ACLs that the parent
/// directory already restricts; the symlink check still applies).
/// Called both immediately after `create_dir_all` and before any
/// cache write to repair pre-existing cache directories left over
/// from earlier loft versions.
pub(crate) fn tighten_cache_dir(dir: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }
    let _ = dir;
}

/// P254 — set a freshly written cache binary's mode to `0o700`
/// on Unix (rwx for owner, nothing for group/other).  Called
/// after `std::fs::copy(&binary, &cached_binary)`.  No-op on
/// non-Unix.
pub(crate) fn tighten_cache_binary(path: &std::path::Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    let _ = path;
}

/// Publish a freshly built native binary to its shared, content-keyed cache path.
///
/// The publish is the one step that several concurrent runs of the SAME source contend
/// on, and it must be ATOMIC.  A plain `fs::copy` onto the destination truncates it in
/// place and then streams the whole binary, and for that entire window a concurrent
/// reader sees a path that EXISTS and passes [`cache_safe_to_execute`] — which checks
/// symlink, owner and mode but never SIZE.  The loser exec'd a 0-byte-and-growing ELF
/// and died with no stdout and no stderr, which is a red gate whose only evidence is an
/// empty output block.  Nor does the mode check save it: once the entry exists at 0700,
/// a later copy truncates it while the mode stays 0700.
///
/// So stage under a private per-process name in the SAME directory (a rename cannot
/// cross a filesystem), tighten the mode while the file is still invisible under its
/// final name, then rename.  POSIX rename is atomic, and a process already exec'ing the
/// previous inode keeps it — which is why the test for this is that the destination's
/// INODE changes: publishing in place is precisely the shape that can be truncated
/// under a reader.
pub(crate) fn publish_cached_binary(
    built: &std::path::Path,
    cached_binary: &std::path::Path,
    cache_dir: &std::path::Path,
    source_stem: &str,
) {
    let leaf = cached_binary
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    // The leading `.` keeps a staged file out of the sweep's `<stem>-` prefix.
    let staged = cache_dir.join(format!(".{leaf}.{}.tmp", std::process::id()));
    if std::fs::copy(built, &staged).is_ok() {
        // P254 — tighten BEFORE the rename, so the binary is never reachable
        // under its final name with a wider mode.
        tighten_cache_binary(&staged);
        if std::fs::rename(&staged, cached_binary).is_err() {
            let _ = std::fs::remove_file(&staged);
        }
    } else {
        let _ = std::fs::remove_file(&staged);
    }
    // Remove stale cached binaries for THIS source file only, AFTER the publish and
    // never the entry just written.  Sweeping first deleted the very binary a
    // concurrent run had already accepted as usable, leaving it to exec a path that
    // no longer existed.
    let prefix = format!("{source_stem}-");
    if let Ok(entries) = std::fs::read_dir(cache_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path != cached_binary && entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
}

/// Collect crate names → rlib paths from a deps directory
/// (e.g. `libfoo-<hash>.rlib` → `("foo", "/path/to/libfoo-<hash>.rlib")`).
pub(crate) fn rlibs_in_dir(
    dir: &std::path::Path,
) -> std::collections::HashMap<String, std::path::PathBuf> {
    let mut map = std::collections::HashMap::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("rlib"))
            {
                let fname = entry.file_name().to_string_lossy().to_string();
                if let Some(rest) = fname.strip_prefix("lib") {
                    if let Some(dash_pos) = rest.rfind('-') {
                        map.insert(rest[..dash_pos].to_string(), path);
                    }
                }
            }
        }
    }
    map
}

/// Whether the host-native backend links `#native` packages by C-ABI (their
/// cdylib `.so`) instead of as Rust rlibs — see NATIVE.md § Resolution: separate
/// the API id from the Rust part.  Both the codegen (`Output::native_cabi`) and
/// the linker flags below read this, so they always agree on a given host.
///
/// @PLN26 phase 4 — the C-ABI path (import-library linking + DLL staged beside
/// the binary on Windows, an RPATH'd `.so` elsewhere) is now the default on
/// EVERY host.  Windows was the last holdout; its arm was verified green in the
/// focused CI (`win-cdylib.yml` job `win-cdylib-cabi`, `LOFT_NATIVE_CABI=1`,
/// `native_crate_package_links_and_runs_via_cabi` PASS on `windows-latest`)
/// before this flip.  `LOFT_NATIVE_CABI=0` remains an escape hatch that forces
/// the legacy rlib link on any host should the C-ABI path regress; `=1` is now a
/// no-op (it already matches the default).
#[must_use]
pub(crate) fn native_cabi_enabled() -> bool {
    // The C-ABI path is the default on every host; only `LOFT_NATIVE_CABI=0`
    // opts back into the legacy rlib link (the escape hatch).
    !matches!(std::env::var("LOFT_NATIVE_CABI").ok().as_deref(), Some("0"))
}

/// PKG.4/PKG.5: add `--extern` flags to a rustc command for native package rlibs.
/// When `target` is `Some("wasm32-wasip2")`, looks for WASM rlibs in `prebuilt/wasm32-wasip2/`;
/// otherwise looks for native rlibs in `native/target/release/`.
///
/// Uses `-L dependency=` for the native package's deps so deep transitive deps
/// resolve. For any crate that also appears in loft's own deps, adds an explicit
/// `--extern name=<loft's copy>` so rustc uses a single copy, avoiding
/// StableCrateId collisions.
///
/// Shared by the standalone native compile (`main.rs`), the WASM compile, and
/// the native test runner (`test_runner.rs`) so all three link a package's
/// `#native` crate identically (LibCI native library gate).
/// @PLN24 arc D — link the C libraries the program declared with `[c] libs`, so
/// the `extern "C"` declarations the `#c` emission wrote resolve.
///
/// The interpreter `dlopen`s the same list; this is the compile-time half of
/// one fact. A library shipped beside its package links by PATH (`-L native=`
/// plus its stem), which also makes the built binary find it without an
/// install; a bare soname goes to the linker's own search path, matching what
/// the dynamic linker does at run time.
///
/// A binding to libc needs no entry and gets no flag: it is already linked.
pub(crate) fn add_c_library_flags(cmd: &mut std::process::Command, data: &crate::data::Data) {
    for lib in data.c_libraries.iter().filter(|c| !c.optional) {
        // @PLN24 arc G — an OPTIONAL library gets no flag at all. On the link
        // line it would be a BUILD dependency: measured, a declared-absent
        // soname fails `rust-lld: unable to find library -l:libfoo.so.9` on a
        // program that never calls into it. The emission resolves it at first
        // call instead (`output_c_direct_call`).
        let (name, dir) = (&lib.name, &lib.pkg_dir);
        // Resolve the declared name to what THIS host actually calls the
        // library, exactly as the interpreter's `dlopen` does — one home for
        // the spelling rules (`platform::host_lib_variants`), because a
        // declaration that loads under `--interpret` and fails to link under
        // `--native` is the divergence @PLN24 keeps refusing to ship.
        //
        // A package that ships its own library declares a PATH
        // (`../../liblc_types.so`), and on macOS its Makefile builds
        // `liblc_types.dylib`. Reading only the declared spelling, the
        // `beside.exists()` test below answered false, so no `-L` and no rpath
        // were emitted and `-l dylib=lc_types` went to the system search path,
        // where the library is not. Prefer the first variant that is really
        // there; fall back to the declared name so a bare soname (nothing
        // beside the package) behaves exactly as before.
        let name = &crate::platform::existing_lib_beside(
            std::path::Path::new(dir),
            name,
            crate::platform::host_lib_os(),
        )
        .unwrap_or_else(|| name.clone());
        let beside = std::path::Path::new(dir).join(name);
        // `libfoo.so.5` -> `foo`: strip the prefix the linker adds back and
        // every version suffix, because `-l` names the library, not the file.
        // Taken from the FILE NAME, never the declared string: a library that
        // ships beside its package is declared as a path (`../../libfoo.so`),
        // and `-l../../libfoo` is not a library name — the linker said so.
        let file = std::path::Path::new(name)
            .file_name()
            .map_or(name.as_str(), |f| f.to_str().unwrap_or(name));
        // `.dll` belongs in this chain beside `.so` / `.dylib`: `-l` names the
        // LIBRARY, not the file, and MSVC appends `.lib` to whatever it is given.
        // Leaving the extension on made `-l dylib=sqlite_shim_<hash>.dll` open
        // `sqlite_shim_<hash>.dll.lib` — the exact `LNK1181` the Windows leg died
        // on, for a shim whose import library is `sqlite_shim_<hash>.lib`.
        let stem = file
            .strip_prefix("lib")
            .unwrap_or(file)
            .split(".so")
            .next()
            .unwrap_or(file)
            .split(".dylib")
            .next()
            .unwrap_or(file)
            .split(".dll")
            .next()
            .unwrap_or(file)
            .to_string();
        if beside.exists()
            && let Some(parent) = beside.parent()
        {
            cmd.arg("-L").arg(format!("native={}", parent.display()));
            // The built binary has to find it at RUN time too, and a library
            // that ships beside its package is not on the system search path.
            //
            // Windows has no RPATH — MSVC `link.exe` does not understand
            // `-Wl,-rpath` and said so (`LNK4044: unrecognized option`, ignored),
            // so passing it was noise that hid the real error below it. The DLL is
            // found beside the `.exe` / on `PATH` instead, the same arrangement
            // `stage_native_dlls` already makes for a package cdylib (@PLN26 ph.4).
            if !cfg!(windows) {
                cmd.arg("-C")
                    .arg(format!("link-arg=-Wl,-rpath,{}", parent.display()));
            }
        }
        // A VERSIONED soname links by exact filename, not by stem.
        //
        // `-l dylib=mariadb` makes the linker look for `libmariadb.so`, which is
        // the `-dev` package's symlink — while the interpreter `dlopen`s
        // `libmariadb.so.3`, the runtime file, which is all a user has.  So one
        // declaration resolved to two different facts: the program ran under
        // `--interpret` and failed to LINK under `--native` with "unable to find
        // library -lmariadb", on a machine where the library is plainly present.
        // That is the backend divergence this plan keeps refusing to ship, and it
        // would have made every `#c` binding to a system library need dev headers
        // that `#c` exists to avoid.
        //
        // `-l:<file>` names the file itself, so both backends resolve exactly the
        // library the declaration named.  Passed as a link-arg because rustc's own
        // `-l` takes a library NAME and would reject the `:file` form.
        if file.contains(".so.") {
            cmd.arg("-C").arg(format!("link-arg=-l:{file}"));
        } else {
            cmd.arg("-l").arg(format!("dylib={stem}"));
        }
    }
}

pub(crate) fn add_native_extern_flags(
    cmd: &mut std::process::Command,
    data: &crate::data::Data,
    target: Option<&str>,
    loft_deps_dir: Option<&std::path::Path>,
) {
    let loft_rlibs = loft_deps_dir.map(rlibs_in_dir).unwrap_or_default();

    for (crate_name, pkg_dir) in &data.native_packages {
        // Host-native C-ABI link (NATIVE.md § Resolution: separate the API id
        // from the Rust part).  Link the package's cdylib `.so` by C-ABI instead
        // of its rlib via `--extern`: the `.so` seals the package's whole Rust
        // crate graph (its own `loft_ffi` copy included), so no `-L dependency=`
        // or per-crate pinning is needed and the shared-dep `StableCrateId`
        // collision class is gone by construction.  Native target only — wasm
        // cross-compiles the package to an rlib (the branch below).
        if target.is_none() && native_cabi_enabled() {
            // @PLN26 phase 0.4 — resolve the `.so` exactly as the interpreter does
            // (`resolve_native_lib`): a host-triple prebuilt (ABI-gated on
            // `loft_ffi_fingerprint`) wins over a source build, a missing declared
            // system lib is terminal, else auto-build (freshness keyed on the
            // loft-ffi ABI, so the `.so` survives loft rebuilds).  The shared
            // resolver keeps native-compile and interpret on the SAME `.so` and
            // adds the prebuilt + missing-syslib handling the hand-rolled path lacked.
            // @PLN26 phase 0.4b — the cdylib is named after the `[library] native`
            // stem, which can differ from the crate name; read it from the manifest
            // (the SAME stem the interpreter resolves with) and fall back to the
            // crate name only when absent.  This keeps native-compile and interpret
            // on one `.so` even when `[lib] name` ≠ `crate_name`.
            let stem = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
                .and_then(|m| m.native)
                .unwrap_or_else(|| crate_name.replace('-', "_"));
            let Some(so) = crate::extensions::resolve_native_lib(pkg_dir, &stem) else {
                // Unresolvable (missing system lib / build failed) — the resolver
                // already printed an actionable message.  Skip; the link then fails
                // loudly on the undefined symbol rather than silently mis-linking.
                continue;
            };
            let so_path = std::path::PathBuf::from(&so);
            if let Some(so_dir) = so_path.parent() {
                // `-l dylib=<name>` derived from the RESOLVED file (strip the `lib`
                // prefix + extension) so a prebuilt or non-`lib<stem>` cdylib links.
                let libname = so_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map_or(stem.as_str(), |s| s.strip_prefix("lib").unwrap_or(s));
                cmd.arg("-L").arg(format!("native={}", so_dir.display()));
                cmd.arg("-l").arg(format!("dylib={libname}"));
                if cfg!(windows) {
                    // @PLN26 phase 4 — Windows links a DLL through its IMPORT
                    // LIBRARY, and there is NO RPATH: the MSVC linker rejects
                    // `-Wl,-rpath`, and the loader finds the DLL beside the `.exe`
                    // / on `PATH` — so the DLL is staged beside the binary at run
                    // time (`stage_native_dlls`), the Windows form of the
                    // `$ORIGIN` rpath used below.
                    //
                    // Naming bridge: a Rust cdylib's import lib is `<stem>.dll.lib`,
                    // but `-l dylib=<stem>` makes MSVC link.exe open `<stem>.lib`
                    // (verified: `LNK1181: cannot open input file
                    // 'loft_native_scalar.lib'`).  Copy `<stem>.dll.lib` →
                    // `<stem>.lib` beside it so the `-l dylib=` above resolves —
                    // both are import libs for the same DLL, identical content.
                    let dll_lib = so_dir.join(format!("{libname}.dll.lib"));
                    let plain_lib = so_dir.join(format!("{libname}.lib"));
                    if dll_lib.exists() && !plain_lib.exists() {
                        let _ = std::fs::copy(&dll_lib, &plain_lib);
                    }
                    // Disallow-the-unverifiable-loudly: if NEITHER import-lib name
                    // is present the link would die on an opaque `LNK1181`, so name
                    // it rather than mis-link.
                    if !plain_lib.exists() && !dll_lib.exists() {
                        eprintln!(
                            "loft: native package `{crate_name}` cdylib at {} has no import \
                             library (`{libname}.dll.lib` / `{libname}.lib`) — Windows links a \
                             DLL through its import lib, not the DLL directly (@PLN26 phase 4).  \
                             Rebuild the package's cdylib with a toolchain that emits one.",
                            so_dir.display()
                        );
                    }
                } else {
                    // @PLN26 phase 0.1 — two RPATH entries: the build/prebuilt dir
                    // (run-from-build-tree: tests, dev) AND `$ORIGIN` (an installed
                    // binary that ships the `.so` beside it — `make install` copies
                    // it next to the binary).  `$ORIGIN` is passed literally; the
                    // dynamic loader expands it at run time.  Windows has no RPATH
                    // (the arm above); it stages the DLL beside the binary instead.
                    cmd.arg(format!("-Clink-arg=-Wl,-rpath,{}", so_dir.display()));
                    // `$ORIGIN` is the ELF spelling and Mach-O does not know it; the dyld
                    // form is `@loader_path`.  Both are emitted, because a linker ignores an
                    // rpath entry it cannot parse and the cost of the spare one is a string.
                    cmd.arg("-Clink-arg=-Wl,-rpath,$ORIGIN");
                    if cfg!(target_os = "macos") {
                        cmd.arg("-Clink-arg=-Wl,-rpath,@loader_path");
                    }
                }
            }
            continue;
        }
        // Look for the compiled rlib in the package's native crate output.
        let rlib_name = format!("lib{}.rlib", crate_name.replace('-', "_"));
        // P244-windows fix #2 (2026-05-12): use single-segment joins,
        // not `.join("native/target/release")` with embedded slashes.
        // When `pkg_dir` is a Windows extended-length path (`\\?\D:\…`),
        // a multi-segment join string with `/` separators inside the
        // verbatim namespace doesn't normalize and the resulting path
        // doesn't match real on-disk files.  Each `.join("X")` with a
        // single component is normalized correctly by `Path` semantics.
        let rlib_path = if let Some(tgt) = target {
            // WASM: check prebuilt first, then native/target/<target>/release/
            let prebuilt = std::path::PathBuf::from(pkg_dir)
                .join("prebuilt")
                .join(tgt)
                .join(&rlib_name);
            if prebuilt.exists() {
                prebuilt
            } else {
                std::path::PathBuf::from(pkg_dir)
                    .join("native")
                    .join("target")
                    .join(tgt)
                    .join("release")
                    .join(&rlib_name)
            }
        } else {
            // @PLAN12 Phase 6b — native target via the shared helper:
            // chunk-resident installs (~/.loft/registry/<pkg>-<ver>/)
            // get the redirected target at ~/.loft/build-cache/<pkg>-<ver>/;
            // monorepo lib/<pkg>/native/ keeps in-tree target/.  Must
            // match what `extensions::auto_build_native` writes so the
            // rlib is found at link time.
            crate::extensions::native_target_root(std::path::Path::new(pkg_dir))
                .join("release")
                .join(&rlib_name)
        };
        // @P359: on a clean checkout the package-under-test's rlib may not
        // exist yet at link time — parse-time `auto_build_native` only fires
        // for *dependency* packages (resolved via `use`), not for the package
        // being tested directly.  CI runs a single fresh `loft --native test`,
        // so without this the first (only) run links nothing and rustc errors
        // E0463 "can't find crate".  Build it on demand here.  Native target
        // only; WASM relies on prebuilt rlibs (no host cargo build).
        // @PLN11 Arc N / N0 — rebuild on a missing rlib OR a stale one (a package
        // rlib built by a DIFFERENT loft build, e.g. left by `make
        // rebuild-native-cdylibs` before loft's ABI changed).  Without the
        // fingerprint check the stale rlib is linked as-is → the "generated
        // rust-code error".  `auto_build_native` re-checks + re-stamps the sidecar.
        // #274 — key on the SAME `native_artifact_cache_key` that `auto_build_native`
        // stamps with (loft-ffi ABI + RUSTFLAGS), so this link-time gate agrees with
        // the build-time stamp: a flag-divergent rlib (whose shared `libloading` would
        // collide with loft's copy) reads as stale here and is rebuilt.
        let profile_dir = rlib_path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."));
        let stale = !loft::cache::native_artifact_fingerprint_matches(
            profile_dir,
            loft::cache::native_artifact_cache_key(),
        );
        if target.is_none() && (!rlib_path.exists() || stale) {
            let stem = crate_name.replace('-', "_");
            let _ = crate::extensions::auto_build_native(pkg_dir, &stem);
        }
        // @PLN26 phase 3 — a wasm target needs the package's rlib (wasm links statically;
        // the C-ABI `.so` path is host-only).  When neither a shipped `prebuilt/` rlib nor a
        // prior cross-build is present, CROSS-BUILD the package's native crate to the wasm
        // target on demand — `auto_build_native_target`, the wasm sibling of
        // `auto_build_native`, lands the rlib at the in-tree path `rlib_path` reads below.
        // Only if that can't produce the rlib (no toolchain/target, or the crate is not
        // wasm-clean) do we emit a clear signal instead of dying on a bare E0463.
        if let Some(tgt) = target {
            // Cross-build on demand UNLESS a `prebuilt/<t>/` rlib is shipped (trusted as
            // sent).  `auto_build_native_target` reuses an ABI-fresh in-tree rlib and
            // rebuilds a stale/missing one, so calling it every wasm compile is cheap (a
            // fingerprint read) and keeps a stale wasm rlib from being linked after a
            // loft-ffi ABI change — the wasm analogue of the host `stale` gate above.
            let prebuilt = std::path::PathBuf::from(pkg_dir)
                .join("prebuilt")
                .join(tgt)
                .join(&rlib_name);
            if !prebuilt.exists() {
                let stem = crate_name.replace('-', "_");
                crate::extensions::auto_build_native_target(pkg_dir, &stem, tgt);
            }
            if !rlib_path.exists() {
                eprintln!(
                    "loft: native package `{crate_name}` has no {tgt} build (no prebuilt, and \
                     cross-build unavailable or the crate is not wasm-clean) — ship a prebuilt \
                     rlib for the package, or run with --interpret."
                );
            }
        }
        if rlib_path.exists() {
            let extern_name = crate_name.replace('-', "_");
            cmd.arg("--extern")
                .arg(format!("{}={}", extern_name, rlib_path.display()));
            // Add the native crate's deps directory so transitive deps (GL, glutin, etc.)
            // resolve. Use `dependency` search scope so these crates are only found as
            // transitive deps of the native crate, not as direct deps.
            let deps_dir = rlib_path.parent().unwrap().join("deps");
            if deps_dir.is_dir() {
                cmd.arg("-L")
                    .arg(format!("dependency={}", deps_dir.display()));
                // Pin any crate that also exists in loft's deps to loft's copy,
                // preventing StableCrateId collisions from duplicate rlibs.
                if !loft_rlibs.is_empty() {
                    let pkg_crates = rlibs_in_dir(&deps_dir);
                    for (dep_name, loft_path) in &loft_rlibs {
                        if pkg_crates.contains_key(dep_name) {
                            cmd.arg("--extern").arg(format!(
                                "{}={}",
                                dep_name,
                                loft_path.display()
                            ));
                        }
                    }
                }
            }
            // @PLN26 phase 3 — a wasm cross-build keeps the package's PROC-MACRO deps
            // (e.g. `loft-ffi-macros`, used by its `#[loft_native]` bridge) as HOST
            // artifacts under `native/target/release/deps` (cargo builds proc-macros for
            // the host even with `--target`).  The wasm `<target>/release/deps` dir above
            // holds only the wasm rlibs, so `extern crate <pkg>` fails to resolve the
            // proc-macro (E0463) without this.  rustc filters by target, so the host
            // loft-ffi rlib that also lives here is ignored in favour of the wasm one.
            if target.is_some() {
                let host_deps = std::path::PathBuf::from(pkg_dir)
                    .join("native")
                    .join("target")
                    .join("release")
                    .join("deps");
                if host_deps.is_dir() {
                    cmd.arg("-L")
                        .arg(format!("dependency={}", host_deps.display()));
                }
            }
            // @P229 (G2): harvest build-script `rustc-link-search` dirs from
            // THIS native package's own target tree, not just the top-level
            // one.  A diamond-dep can pull a second `windows_x86_64_msvc`
            // version (e.g. graphics → winit/glutin pulls 0.52.6) built under
            // `<pkg>/native/target/release/build`, whose `windows.0.52.0.lib`
            // the top-level harvest at the call site never sees — so its
            // `/LIBPATH` is missing and the link fails `LNK1181: cannot open
            // input file 'windows.0.52.0.lib'`.  `rlib_path.parent()` is the
            // package's `<profile>` dir, exactly what the helper expects.
            if let Some(profile_dir) = rlib_path.parent() {
                for nd in build_script_native_lib_dirs(profile_dir) {
                    cmd.arg("-L").arg(format!("native={}", nd.display()));
                }
            }
        }
    }
}

/// Say something useful when a Windows binary dies before `main`.
///
/// `STATUS_DLL_NOT_FOUND` (`0xC000_0135`) is the loader refusing to start a
/// process whose imports it cannot resolve. It happens BEFORE any user code, so
/// the program writes nothing at all: no stdout, no stderr, just an exit code
/// most people have never seen. That silence is the worst part of the failure,
/// and it is entirely avoidable — loft knows which libraries it staged and where.
///
/// Reports what IS beside the binary rather than guessing which import is
/// missing: naming the missing DLL needs the import table, and the useful
/// question for an author is almost always "did my library get here at all".
/// A `[c]` library resolved from the system (a bare soname) is deliberately not
/// listed — it was never loft's to place.
///
/// No-op off Windows and for any other exit code.
pub(crate) fn explain_windows_startup_failure(
    status: std::process::ExitStatus,
    binary: &std::path::Path,
    data: &crate::data::Data,
) {
    const STATUS_DLL_NOT_FOUND: i32 = 0xC000_0135_u32 as i32;
    if !cfg!(windows) || status.code() != Some(STATUS_DLL_NOT_FOUND) {
        return;
    }
    eprintln!(
        "loft: the program could not start — Windows could not find a DLL it \
         imports (STATUS_DLL_NOT_FOUND, 0x{STATUS_DLL_NOT_FOUND:08X}). This \
         happens before any of your code runs, which is why nothing was printed."
    );
    if let Some(dir) = binary.parent() {
        let mut staged: Vec<String> = std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| n.to_ascii_lowercase().ends_with(".dll"))
            .collect();
        staged.sort();
        if staged.is_empty() {
            eprintln!("loft:   no DLLs are staged beside `{}`", dir.display());
        } else {
            eprintln!(
                "loft:   staged beside `{}`: {}",
                dir.display(),
                staged.join(", ")
            );
        }
    }
    for lib in &data.c_libraries {
        let d = std::path::Path::new(&lib.pkg_dir);
        if let Some(found) =
            crate::platform::existing_lib_beside(d, &lib.name, crate::platform::host_lib_os())
        {
            eprintln!("loft:   `[c]` library shipped by the package: {found}");
        }
    }
    eprintln!(
        "loft:   Windows has no RPATH: a DLL is found beside the executable or \
         on PATH. A MinGW-built library may also need its own runtime \
         (libgcc_s_seh-1.dll, libwinpthread-1.dll) on PATH."
    );
}

/// @PLN24 — the `[c]` half of [`stage_native_dlls`]: a C library that ships
/// BESIDE its package has the same run-time problem as a native-package cdylib,
/// and had none of the answer.
///
/// The link succeeds (the import library is right there), and then the binary
/// dies before `main` with `STATUS_DLL_NOT_FOUND` — writing nothing at all, so
/// the failure presents as an empty stdout and an empty stderr. Windows has no
/// RPATH to fall back on, which is exactly why `add_c_library_flags` stopped
/// emitting one there.
///
/// Only libraries that are really PRESENT beside the package are copied:
/// `existing_lib_beside` answers `None` for a bare soname like
/// `libsqlite3.so.0`, and that is the correct answer — a system library is the
/// system's to find, and copying one next to the binary would be wrong.
/// Best-effort, like its sibling: a failed copy leaves the loader's normal
/// search to do what it can.
fn stage_c_library_dlls(exe_dir: &std::path::Path, data: &crate::data::Data) {
    for lib in &data.c_libraries {
        let dir = std::path::Path::new(&lib.pkg_dir);
        let Some(found) =
            crate::platform::existing_lib_beside(dir, &lib.name, crate::platform::host_lib_os())
        else {
            continue; // a bare soname — the system's to resolve, not ours to copy
        };
        let src = dir.join(&found);
        if let Some(name) = src.file_name() {
            let dest = exe_dir.join(name);
            if dest != src {
                let _ = std::fs::copy(&src, &dest);
            }
        }
    }
}

/// @PLN26 phase 4 — stage every native-package DLL beside a just-built Windows
/// binary so it loads at run time.  Windows has no RPATH, so the loader finds a
/// linked DLL beside the `.exe` / on `PATH`; copying it next to the binary is the
/// Windows form of the `$ORIGIN` rpath the ELF link embeds (`add_native_extern_flags`).
/// No-op off Windows and when the C-ABI path is disabled (the rlib path links the
/// package statically, so there is nothing to stage).  Resolves the DLL with the
/// same `resolve_native_lib` the link used, so run-time and link-time agree on the
/// file.  Best-effort: a failed copy leaves the loader's normal search to find it.
pub(crate) fn stage_native_dlls(exe_dir: &std::path::Path, data: &crate::data::Data) {
    if !cfg!(windows) {
        return;
    }
    stage_c_library_dlls(exe_dir, data);
    if !native_cabi_enabled() {
        return;
    }
    for (crate_name, pkg_dir) in &data.native_packages {
        let stem = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
            .and_then(|m| m.native)
            .unwrap_or_else(|| crate_name.replace('-', "_"));
        if let Some(so) = crate::extensions::resolve_native_lib(pkg_dir, &stem) {
            let so_path = std::path::PathBuf::from(&so);
            if let Some(name) = so_path.file_name() {
                let dest = exe_dir.join(name);
                if dest != so_path {
                    let _ = std::fs::copy(&so_path, &dest);
                }
            }
        }
    }
}

/// The stdlib must be findable from EVERY cargo layout, not just
/// `target/{release,debug}`.  A `--target <triple>` build nests the profile directory
/// one level deeper, so the project root cannot be derived by assuming a fixed depth
/// above the binary.
///
/// ⚠ Getting this wrong fails far from its cause: `default/` is simply not found, every
/// stdlib load dies with "cannot load default library", and the only job that builds with
/// `--target` is the nightly ASan sweep — so it surfaces there, as an unrelated
/// sandbox-admission test. (loft#638)
#[cfg(test)]
mod project_root_layouts {
    use super::*;

    fn chrono_ish_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    }

    /// A throwaway project root that HOLDS a `default/` dir, plus the binary dir
    /// under it named by `rel`.  Returns (root, binary dir).
    fn layout(tag: &str, rel: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "loft_root_{tag}_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("default")).unwrap();
        let bin_dir = root.join(rel);
        std::fs::create_dir_all(&bin_dir).unwrap();
        (root, bin_dir)
    }

    #[test]
    fn plain_release_and_debug_layouts_resolve_to_the_root() {
        for rel in ["target/release", "target/debug"] {
            let (root, bin_dir) = layout("plain", rel);
            assert_eq!(
                project_root_for(&bin_dir),
                with_trailing_sep(&root),
                "{rel} must resolve to the project root"
            );
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// The regression: `cargo --target <triple>` (how the ASan gate instruments
    /// the whole build) puts the binary one level deeper.
    #[test]
    fn explicit_target_triple_layout_resolves_to_the_root() {
        for rel in [
            "target/x86_64-unknown-linux-gnu/release",
            "target/aarch64-apple-darwin/release",
            "target/x86_64-unknown-linux-gnu/debug",
        ] {
            let (root, bin_dir) = layout("triple", rel);
            assert_eq!(
                project_root_for(&bin_dir),
                with_trailing_sep(&root),
                "{rel} must resolve to the project root, not the binary's own dir"
            );
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// `CARGO_TARGET_DIR=target-da` (the debug-assertions calibration build) and
    /// custom cargo profiles rename the dirs; the stdlib must still be found.
    #[test]
    fn renamed_target_dir_and_custom_profile_resolve_to_the_root() {
        for rel in ["target-da/release", "target/ci", "target-da/x86_64-foo/ci"] {
            let (root, bin_dir) = layout("renamed", rel);
            assert_eq!(
                project_root_for(&bin_dir),
                with_trailing_sep(&root),
                "{rel} must resolve to the project root"
            );
            let _ = std::fs::remove_dir_all(&root);
        }
    }

    /// An unpacked release dir ships the binary NEXT TO `default/` — the walk must
    /// stop there rather than climbing past it.
    #[test]
    fn binary_beside_the_stdlib_resolves_to_its_own_dir() {
        let (root, _) = layout("beside", "unused");
        assert_eq!(project_root_for(&root), with_trailing_sep(&root));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// No stdlib anywhere above: unchanged fallback to the binary's own dir (the
    /// caller then reports a missing stdlib, which is the honest answer).
    #[test]
    fn no_stdlib_above_falls_back_to_the_binary_dir() {
        let dir = std::env::temp_dir().join(format!(
            "loft_root_none_{}_{}/deep/deeper",
            std::process::id(),
            chrono_ish_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(project_root_for(&dir), with_trailing_sep(&dir));
        let _ = std::fs::remove_dir_all(dir.parent().unwrap().parent().unwrap());
    }
}

#[cfg(test)]
mod p254_cache_safety {
    use super::*;

    #[test]
    fn nonexistent_cache_is_unsafe() {
        let p = std::env::temp_dir().join("loft_p254_does_not_exist_xyz_12345");
        let _ = std::fs::remove_file(&p);
        assert!(!cache_safe_to_execute(&p));
    }

    #[test]
    fn freshly_written_owner_only_cache_is_safe() {
        let p = std::env::temp_dir().join(format!(
            "loft_p254_safe_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, b"#!/bin/sh\necho hi\n").unwrap();
        tighten_cache_binary(&p);
        assert!(cache_safe_to_execute(&p), "owner-only file should be safe");
        let _ = std::fs::remove_file(&p);
    }

    #[cfg(unix)]
    #[test]
    fn group_writable_cache_is_unsafe() {
        use std::os::unix::fs::PermissionsExt;
        let p = std::env::temp_dir().join(format!(
            "loft_p254_groupwrite_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, b"x").unwrap();
        // 0o766 has group write and other rwx — attacker-modifiable.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o766)).unwrap();
        assert!(!cache_safe_to_execute(&p));
        let _ = std::fs::remove_file(&p);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_cache_is_unsafe() {
        let target = std::env::temp_dir().join(format!(
            "loft_p254_symtarget_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let link = std::env::temp_dir().join(format!(
            "loft_p254_symlink_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = std::fs::remove_file(&target);
        let _ = std::fs::remove_file(&link);
        std::fs::write(&target, b"x").unwrap();
        tighten_cache_binary(&target);
        std::os::unix::fs::symlink(&target, &link).unwrap();
        // Even though the symlink TARGET would pass, the link
        // itself routes through `symlink_metadata` and is rejected.
        assert!(!cache_safe_to_execute(&link));
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_file(&target);
    }

    #[cfg(unix)]
    #[test]
    fn suid_cache_is_unsafe() {
        use std::os::unix::fs::PermissionsExt;
        let p = std::env::temp_dir().join(format!(
            "loft_p254_suid_{}_{}",
            std::process::id(),
            chrono_ish_nanos()
        ));
        let _ = std::fs::remove_file(&p);
        std::fs::write(&p, b"x").unwrap();
        // 0o4700 — owner rwx + setuid bit.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o4700)).unwrap();
        assert!(!cache_safe_to_execute(&p));
        let _ = std::fs::remove_file(&p);
    }

    /// Lightweight nanosecond suffix to keep test temp paths
    /// from colliding when `cargo test` runs the cases in
    /// parallel without `--test-threads=1`.
    fn chrono_ish_nanos() -> u128 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos())
    }
}

#[cfg(test)]
mod p350_html_wasm_import_check {
    use super::*;

    /// Build a minimal valid wasm (header + one import section) whose single
    /// import has the given module name and is a function import (kind 0).
    /// A minimal wasm importing exactly `module.field` as a function (kind 0).
    fn wasm_with_import(module: &str, field: &str) -> Vec<u8> {
        let mut entry = Vec::new();
        entry.push(u8::try_from(module.len()).expect("short module"));
        entry.extend_from_slice(module.as_bytes());
        entry.push(u8::try_from(field.len()).expect("short field"));
        entry.extend_from_slice(field.as_bytes());
        entry.push(0); // kind: func
        entry.push(0); // typeidx
        let mut body = vec![1u8]; // one import
        body.extend_from_slice(&entry);
        let mut wasm = b"\0asm\x01\x00\x00\x00".to_vec();
        wasm.push(2); // import section
        wasm.push(u8::try_from(body.len()).expect("small section"));
        wasm.extend_from_slice(&body);
        wasm
    }

    fn wasm_with_import_module(module: &str) -> Vec<u8> {
        // import entry: <ulen module><module><ulen field><field><kind=0><typeidx=0>
        let field = "f";
        let mut entry = Vec::new();
        entry.push(module.len() as u8);
        entry.extend_from_slice(module.as_bytes());
        entry.push(field.len() as u8);
        entry.extend_from_slice(field.as_bytes());
        entry.push(0x00); // kind: func
        entry.push(0x00); // typeidx 0
        // import section body: <count=1><entry>
        let mut body = vec![0x01u8];
        body.extend_from_slice(&entry);
        // section: <id=2><usize len><body>
        let mut wasm = b"\0asm".to_vec();
        wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]); // version 1
        wasm.push(0x02); // import section id
        wasm.push(body.len() as u8); // section size (small enough for one byte uleb)
        wasm.extend_from_slice(&body);
        wasm
    }

    #[test]
    fn accepts_loft_gl_and_loft_io() {
        assert!(html_wasm_import_modules_ok(&wasm_with_import_module("loft_gl")).is_ok());
        assert!(html_wasm_import_modules_ok(&wasm_with_import_module("loft_io")).is_ok());
    }

    #[test]
    fn rejects_wbindgen_stomp() {
        let bad = html_wasm_import_modules_ok(&wasm_with_import_module("__wbindgen_placeholder__"))
            .expect_err("wbindgen import module must be rejected");
        assert_eq!(bad, vec!["__wbindgen_placeholder__".to_string()]);
    }

    #[test]
    fn no_imports_is_ok() {
        // Bare header, no sections — a wasm with zero imports is acceptable.
        let mut wasm = b"\0asm".to_vec();
        wasm.extend_from_slice(&[0x01, 0x00, 0x00, 0x00]);
        assert!(html_wasm_import_modules_ok(&wasm).is_ok());
    }

    #[test]
    fn malformed_does_not_false_abort() {
        // Bad magic / truncated input must be conservatively accepted (Ok),
        // never a false rejection of a bundle this parser can't read.
        assert!(html_wasm_import_modules_ok(b"not a wasm").is_ok());
        assert!(html_wasm_import_modules_ok(&[]).is_ok());
        assert!(html_wasm_import_modules_ok(b"\0asm\x01\x00\x00\x00\x02\xff").is_ok());
    }

    /// loft#668 — a host function the page will not provide is decidable at BUILD time,
    /// so it must be reported by name instead of becoming a `LinkError` at instantiate
    /// that names an import index and kills the whole page.
    #[test]
    fn missing_host_import_is_named() {
        let wasm = wasm_with_import("loft_gl", "loft_gl_window_width");
        // Absent from the shim → reported, module-qualified.
        assert_eq!(
            missing_host_imports(&wasm, "loft_gl_use_shader(p) {}"),
            vec!["loft_gl.loft_gl_window_width".to_string()],
        );
        // Present as a method, and as a property — both are definitions.
        assert!(missing_host_imports(&wasm, "loft_gl_window_width() { return 1; }").is_empty());
        assert!(missing_host_imports(&wasm, "loft_gl_window_width: () => 1,").is_empty());
        // A LONGER name that merely CONTAINS the import must not count as providing it —
        // otherwise `loft_gl_window_width_ex` would mask the real gap.
        assert_eq!(
            missing_host_imports(&wasm, "x_loft_gl_window_width() {}").len(),
            1,
            "a longer identifier ending in the name does not define it"
        );
        // The shipped shim really does provide it (this is what the fix added).
        assert!(
            missing_host_imports(&wasm, include_str!("../doc/loft-gl-wasm.js")).is_empty(),
            "doc/loft-gl-wasm.js must define loft_gl_window_width"
        );
        // Unwalkable input yields no opinion, never a false build failure.
        assert!(missing_host_imports(b"not a wasm", "").is_empty());
    }

    /// loft#669 — every handle table in the browser shim hands out 1-BASED handles, so
    /// `0` means failure and nothing else, as `lib/graphics` documents and as the native
    /// backend behaves.  Pinned by reading the shim: a table that goes back to returning
    /// its array index would make the FIRST shader/VAO/texture on a page indistinguishable
    /// from a failure.
    #[test]
    fn browser_gl_handles_are_one_based() {
        let js = include_str!("../doc/loft-gl-wasm.js");
        assert!(
            !js.contains("programs.length; programs.push")
                && !js.contains("vaos.length; vaos.push")
                && !js.contains("textures.length; textures.push"),
            "a handle table returned its 0-based array index"
        );
        // `hold` returns Array.prototype.push's value — the new LENGTH, i.e. index + 1.
        assert!(
            js.contains("const hold = (arr, obj) => arr.push(obj)"),
            "handles must be minted through `hold` (1-based)"
        );
        assert!(
            js.contains("const slot = (arr, h) => (h > 0 && h <= arr.length ? arr[h - 1] : null)"),
            "handles must be resolved through `slot` (rejects 0)"
        );
    }
}

#[cfg(test)]
mod atomics_sysroot_tests {
    use super::*;

    /// Build a fake shape dir `<root>/<triple>/release` with one dep rlib in it,
    /// mirroring what `ensure_loft_runtime_rlib` hands `ensure_atomics_sysroot`.
    fn fake_profile_dir(root: &std::path::Path) -> std::path::PathBuf {
        let profile = root.join("wasm32-unknown-unknown").join("release");
        let deps = profile.join("deps");
        std::fs::create_dir_all(&deps).expect("deps");
        std::fs::write(deps.join("libcore-abc.rlib"), b"rlib").expect("dep rlib");
        // loft's own rlib must be EXCLUDED from the sysroot (duplicate-candidate
        // errors at link time), so seed one to prove the filter still runs.
        std::fs::write(deps.join("libloft-def.rlib"), b"rlib").expect("loft rlib");
        profile
    }

    /// The dev layout is unchanged: the sysroot lands beside the target dir.
    #[test]
    fn assembles_next_to_the_target_dir_when_writable() {
        let root = std::env::temp_dir().join(format!("loft_sysroot_dev_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let profile = fake_profile_dir(&root);
        let got = ensure_atomics_sysroot(&profile).expect("sysroot assembled");
        assert_eq!(got, root.join("sysroot"), "dev layout must not move");
        let lib = got
            .join("lib/rustlib/wasm32-unknown-unknown/lib")
            .join("libcore-abc.rlib");
        assert!(lib.exists(), "dep rlib copied into the sysroot");
        assert!(
            !got.join("lib/rustlib/wasm32-unknown-unknown/lib/libloft-def.rlib")
                .exists(),
            "loft's own rlib must stay out of the sysroot"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// #619 — the installed layout is root-owned, so the derived location cannot
    /// be created.  The assembly must fall back to the user cache instead of
    /// giving up, which is what silently produced a single-threaded page.
    #[test]
    #[cfg(unix)]
    fn falls_back_to_the_user_cache_when_the_target_dir_is_read_only() {
        use std::os::unix::fs::PermissionsExt;
        let base = std::env::temp_dir().join(format!("loft_sysroot_ro_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let root = base.join("share").join("loft");
        let profile = fake_profile_dir(&root);
        let home = base.join("home");
        std::fs::create_dir_all(&home).expect("home");

        // Make the derived sysroot parent unwritable — the installed-layout shape.
        let mut perms = std::fs::metadata(&root).expect("meta").permissions();
        perms.set_mode(0o555);
        std::fs::set_permissions(&root, perms).expect("chmod ro");

        // SAFETY: single-threaded test process; restored below.
        unsafe { std::env::set_var("LOFT_HOME", &home) };
        let got = ensure_atomics_sysroot(&profile);
        unsafe { std::env::remove_var("LOFT_HOME") };

        let mut perms = std::fs::metadata(&root).expect("meta").permissions();
        perms.set_mode(0o755);
        let _ = std::fs::set_permissions(&root, perms);

        let got = got.expect("must fall back rather than return None");
        assert!(
            got.starts_with(&home),
            "expected the user cache under {home:?}, got {got:?}"
        );
        assert!(
            got.join("lib/rustlib/wasm32-unknown-unknown/lib/libcore-abc.rlib")
                .exists(),
            "dep rlib copied into the fallback sysroot"
        );
        let _ = std::fs::remove_dir_all(&base);
    }
}

#[cfg(test)]
mod host_import_tests {
    use super::missing_host_imports;

    /// A minimal wasm importing exactly one function: `loft_web.ws_yield`.
    /// Hand-built so the test does not need a toolchain: magic + version, a type
    /// section with one `() -> ()`, and an import section naming the function.
    fn wasm_importing(module: &str, field: &str) -> Vec<u8> {
        let mut w = vec![0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00];
        // type section: one type, `() -> ()`
        w.extend_from_slice(&[0x01, 0x04, 0x01, 0x60, 0x00, 0x00]);
        // import section: one entry, kind 0 (function), type index 0
        let mut body = vec![0x01];
        body.push(u8::try_from(module.len()).unwrap());
        body.extend_from_slice(module.as_bytes());
        body.push(u8::try_from(field.len()).unwrap());
        body.extend_from_slice(field.as_bytes());
        body.extend_from_slice(&[0x00, 0x00]);
        w.push(0x02);
        w.push(u8::try_from(body.len()).unwrap());
        w.extend_from_slice(&body);
        w
    }

    /// loft#681 — `ns.ws_yield = function () {…}` is how a library's
    /// `[wasm.bridge] host_js` installs a handler, and it was invisible to the check.
    /// The program was refused with a message telling its author to add the handler
    /// they had already written, and there was no way to overrule it.
    #[test]
    fn assignment_form_counts_as_provided() {
        let wasm = wasm_importing("loft_web", "ws_yield");
        let js = "  ns.ws_yield = function () { return 0; };\n";
        assert!(
            missing_host_imports(&wasm, js).is_empty(),
            "an assigned handler is provided"
        );
    }

    /// The two forms that already worked must keep working.
    #[test]
    fn method_and_property_forms_still_count() {
        let wasm = wasm_importing("loft_web", "ws_yield");
        assert!(
            missing_host_imports(&wasm, "  ws_yield() { return 0; },\n").is_empty(),
            "method shorthand"
        );
        assert!(
            missing_host_imports(&wasm, "  ws_yield: () => 0,\n").is_empty(),
            "property"
        );
        assert!(
            missing_host_imports(&wasm, "  ws_yield : () => 0,\n").is_empty(),
            "property with a space before the colon is still a colon form"
        );
    }

    /// The check must still be able to FAIL, or widening it would have quietly
    /// disabled loft#668's whole point. A prose mention is not a definition, and
    /// neither is a comparison.
    #[test]
    fn a_mention_or_comparison_is_not_a_definition() {
        let wasm = wasm_importing("loft_web", "ws_yield");
        for js in [
            "// ws_yield is the asyncify suspend shim\n",
            "if (name === ws_yield) { }\n",
            "  other_thing = function () {};\n",
            "",
        ] {
            assert_eq!(
                missing_host_imports(&wasm, js),
                vec!["loft_web.ws_yield".to_string()],
                "must report missing for: {js:?}"
            );
        }
    }

    /// A longer identifier that merely ENDS with the name is a different function.
    #[test]
    fn a_longer_identifier_is_not_the_same_name() {
        let wasm = wasm_importing("loft_web", "ws_yield");
        assert_eq!(
            missing_host_imports(&wasm, "  do_ws_yield = function () {};\n"),
            vec!["loft_web.ws_yield".to_string()],
            "`do_ws_yield` does not define `ws_yield`"
        );
    }
}

#[cfg(test)]
mod crate_resolution_tests {
    use super::crate_resolution_failure;

    // ── loft#706 — what may trigger a runtime rebuild + retry ────────────────────
    //
    // The classifier is the safety half of the heal: only a toolchain/cache failure
    // may cost the user a rebuild.  A codegen bug must fall straight through, or every
    // broken program pays a minute before seeing its error.

    /// The shape that motivated the issue: the rlib was built by a different rustc
    /// than the one compiling against it.  A rebuild is exactly the cure.
    #[test]
    fn an_incompatible_rlib_is_a_crate_resolution_failure() {
        let stderr = "error[E0514]: found crate `loft` compiled by an incompatible \
                      version of rustc\n = help: please recompile that crate";
        assert!(crate_resolution_failure(stderr));
    }

    /// The same staleness seen through a transitive dep.
    #[test]
    fn a_newer_transitive_crate_is_a_crate_resolution_failure() {
        let stderr = "error[E0460]: found possibly newer version of crate `rand_core`";
        assert!(crate_resolution_failure(stderr));
    }

    /// A bundle that ships no rlib.
    #[test]
    fn a_missing_crate_is_a_crate_resolution_failure() {
        let stderr = "error[E0463]: can't find crate for `loft`";
        assert!(crate_resolution_failure(stderr));
    }

    /// A genuine codegen bug must NOT heal — this is the guard that keeps a rebuild
    /// off the path of every broken program.
    #[test]
    fn a_codegen_error_is_not_a_crate_resolution_failure() {
        for stderr in [
            "error[E0308]: mismatched types\n expected `DbRef`, found `()`",
            "error[E0425]: cannot find value `var_x` in this scope",
            "error[E0506]: cannot assign to `var___work_c1` because it is borrowed",
            "error: aborting due to 1 previous error",
        ] {
            assert!(
                !crate_resolution_failure(stderr),
                "must not rebuild for: {stderr:?}"
            );
        }
    }

    /// BOTH halves are required.  A bare code with no matching explanation is not
    /// enough: `E0463` also fires for a user's own missing `--lib` package, which no
    /// rebuild of loft's runtime can fix.
    #[test]
    fn a_bare_error_code_without_its_explanation_does_not_heal() {
        assert!(!crate_resolution_failure(
            "error[E0463]: something else entirely"
        ));
        assert!(!crate_resolution_failure(
            "compiled by an incompatible version"
        ));
    }
}

#[cfg(test)]
mod dep_search_dirs_tests {
    use super::dep_search_dirs;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join("loft-dep-search").join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn touch(p: &std::path::Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    /// The classic layout must keep yielding exactly ONE search dir — the per-unit
    /// fallback is a fallback, not a replacement, and a change here would multiply
    /// every `--native` compile's `-L` count on the platform that works today.
    #[test]
    fn the_classic_single_deps_layout_yields_one_dir() {
        let root = scratch("classic");
        touch(&root.join("deps").join("libsha2-abc.rlib"));
        assert_eq!(dep_search_dirs(&root), vec![root.join("deps")]);
    }

    /// cargo 1.99.0-nightly (2026-07-29) gives each crate its own
    /// `build/<crate>/<hash>/out/`, and no `deps/` at all.
    #[test]
    fn the_per_unit_layout_yields_one_dir_per_crate() {
        let root = scratch("per-unit");
        touch(&root.join("build/sha2/h1/out/libsha2-h1.rlib"));
        touch(&root.join("build/serde/h2/out/libserde-h2.rlib"));
        // A proc-macro ships a dylib, never an rlib.  Excluding these is what made an
        // rlib-only filter fail one crate later on `curve25519_dalek_derive` —
        // indistinguishable, from the error alone, from the fix not working.
        touch(&root.join("build/derive/h3/out/libderive-h3.so"));
        // A build-script output dir carries generated sources, not libraries: passing
        // it would be noise, and there are ~100 of them.
        touch(&root.join("build/somelib/h4/out/generated.rs"));

        let dirs = dep_search_dirs(&root);
        assert_eq!(dirs.len(), 3, "{dirs:?}");
        for c in ["sha2", "serde", "derive"] {
            assert!(
                dirs.iter().any(|d| d.to_string_lossy().contains(c)),
                "{c} missing from {dirs:?}"
            );
        }
        assert!(
            !dirs.iter().any(|d| d.to_string_lossy().contains("somelib")),
            "a source-only out dir must not be searched: {dirs:?}"
        );
    }

    /// The atomics sysroot is assembled from the SAME dep dirs, and its failure mode is
    /// what made the browser gate unreadable: an unassembled sysroot does not fail the
    /// link, it drops `--html --threads` to the default rustc, which then cannot link
    /// the nightly-built rlib and reports `E0514` — an error naming the wrong subject,
    /// two steps from its cause.  So "no readable dep dir" must stay LOUD.
    #[test]
    fn the_sysroot_assembles_from_every_dep_dir_and_fails_loudly_with_none() {
        let root = scratch("sysroot");
        let out = root.join("out");

        // Per-unit layout: two crates, two directories.
        let a = root.join("build/core/h1/out");
        let b = root.join("build/alloc/h2/out");
        touch(&a.join("libcore-h1.rlib"));
        touch(&b.join("liballoc-h2.rlib"));
        // loft's own rlib is deliberately excluded — a third copy on the sysroot search
        // path is how rustc ends up reporting multiple candidates for one crate.
        touch(&b.join("libloft.rlib"));

        super::assemble_atomics_sysroot(&[a, b], &out).expect("assembles from both dirs");
        let lib = out.join("lib/rustlib/wasm32-unknown-unknown/lib");
        assert!(lib.join("libcore-h1.rlib").is_file());
        assert!(lib.join("liballoc-h2.rlib").is_file());
        assert!(
            !lib.join("libloft.rlib").exists(),
            "loft's own rlib must not reach the sysroot"
        );

        // Nothing readable: an error, never a silent Ok.
        let err =
            super::assemble_atomics_sysroot(&[root.join("does-not-exist")], &root.join("out2"))
                .expect_err("an unassembled sysroot must not report success");
        assert_eq!(err.kind(), std::io::ErrorKind::NotFound, "{err}");
    }

    /// Neither layout present: name the classic path anyway, so the failure diagnostic
    /// tells the user where loft looked instead of printing nothing.
    #[test]
    fn an_empty_tree_still_names_the_expected_place() {
        let root = scratch("empty");
        assert_eq!(dep_search_dirs(&root), vec![root.join("deps")]);
    }
}

#[cfg(test)]
mod publish_cached_binary_tests {
    use super::publish_cached_binary;

    fn scratch(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join("loft-publish-cache").join(name);
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The pre-fix publish, kept as the POSITIVE CONTROL.  Without it the test below
    /// asserts a property nothing was ever measured to violate, and a guard that cannot
    /// fail is not a guard.
    ///
    /// `cfg(unix)` with its one caller: the control exists only for the inode reading, so
    /// on Windows it would be a function nobody calls and `-D warnings` fails the build on
    /// `dead_code`.
    #[cfg(unix)]
    fn publish_by_copy(built: &std::path::Path, cached: &std::path::Path) {
        let _ = std::fs::copy(built, cached);
    }

    /// A publish REPLACES the destination rather than rewriting it in place.
    ///
    /// The inode is the deterministic reading of that: `fs::copy` opens the destination
    /// with truncate and streams into the SAME inode, so a concurrent reader that has
    /// already accepted the path watches its bytes vanish and then regrow.  A rename
    /// swaps in a different inode, and a process still exec'ing the old one keeps a
    /// complete file.  This is the property; the timing window is only its symptom.
    ///
    /// UNIX only, because the READING is: `MetadataExt::ino` does not exist on Windows,
    /// and an unguarded `use std::os::unix::fs::MetadataExt` at the module head broke the
    /// Windows build outright (`cannot find unix in os`, then four `no method named ino`)
    /// — a test's import, taking down `cargo test` for the whole target.  The property is
    /// not unix-only and Windows spells its identity `MetadataExt::file_index`; that arm is
    /// deliberately NOT written here, because it cannot be measured from this box and an
    /// assertion nobody has watched fail is the thing this test's own control exists to
    /// refuse.  The sweep test below stays on every platform.
    #[cfg(unix)]
    #[test]
    fn a_publish_swaps_the_inode_instead_of_truncating_in_place() {
        use std::os::unix::fs::MetadataExt;
        let dir = scratch("inode");
        let a = dir.join("built-a");
        let b = dir.join("built-b");
        std::fs::write(&a, vec![0xAAu8; 4096]).unwrap();
        std::fs::write(&b, vec![0xBBu8; 8192]).unwrap();
        let dest = dir.join("prog-deadbeef");

        publish_cached_binary(&a, &dest, &dir, "prog");
        let first = std::fs::metadata(&dest).unwrap().ino();
        publish_cached_binary(&b, &dest, &dir, "prog");
        let second = std::fs::metadata(&dest).unwrap().ino();

        assert_ne!(
            first, second,
            "a publish must swap a new inode in, never rewrite the live one"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), vec![0xBBu8; 8192]);

        // The control: the copy this replaced keeps the inode, so the same assertion
        // fails on it — the probe discriminates.
        let dest2 = dir.join("ctl-deadbeef");
        publish_by_copy(&a, &dest2);
        let c1 = std::fs::metadata(&dest2).unwrap().ino();
        publish_by_copy(&b, &dest2);
        let c2 = std::fs::metadata(&dest2).unwrap().ino();
        assert_eq!(
            c1, c2,
            "control: a plain copy rewrites the live inode in place"
        );
    }

    /// The publish leaves no staging file behind, and the sweep spares the entry it
    /// just wrote — sweeping first deleted the binary a concurrent run had accepted.
    #[test]
    fn the_sweep_takes_other_hashes_and_spares_the_published_entry() {
        let dir = scratch("sweep");
        let built = dir.join("built");
        std::fs::write(&built, vec![0xCCu8; 2048]).unwrap();
        let stale = dir.join("prog-0000000000000000");
        std::fs::write(&stale, b"stale").unwrap();
        let other = dir.join("elsewhere-1111111111111111");
        std::fs::write(&other, b"other source").unwrap();
        let dest = dir.join("prog-ffffffffffffffff");

        publish_cached_binary(&built, &dest, &dir, "prog");

        assert!(
            dest.exists(),
            "the entry just published must survive its own sweep"
        );
        assert_eq!(std::fs::read(&dest).unwrap(), vec![0xCCu8; 2048]);
        assert!(!stale.exists(), "a stale hash for this source is swept");
        assert!(
            other.exists(),
            "another source's entry is not this sweep's business"
        );
        let staging: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|n| {
                std::path::Path::new(n)
                    .extension()
                    .is_some_and(|e| e == "tmp")
            })
            .collect();
        assert!(
            staging.is_empty(),
            "no staging file may be left behind: {staging:?}"
        );
    }
}
