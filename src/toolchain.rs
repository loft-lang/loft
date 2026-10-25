// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: the toolchain on this machine

//! What this machine offers a native build: a working `rustc` ([`rustc_available`]), the
//! `libloft.rlib` a build links against ([`find_loft_rlib`]), and the reading of a failed
//! build's output that tells an environment failure (a full disk, a mismatched rlib) from a
//! code error ([`toolchain_failure_hint`]).  The driver asks it before choosing a backend, and
//! the Rust writer asks it to build.

/// Candidate `(rlib-lookup-dir, deps-dir)` pairs for a loft binary at `exe_dir`,
/// most authoritative first.  The dev/test layout looks beside the exe (`deps/`,
/// then the uplifted `<profile>/`); the INSTALLED layout (`<prefix>/bin/loft`)
/// keeps `libloft.rlib` + its deps under `<prefix>/share/loft/` — where `make
/// install` puts them, NOT next to the binary.  Without the share candidates the
/// installed loft can fingerprint its rlib (`cache::loft_rlib_path` has the same
/// fallback) but cannot LOCATE it to link a library's cdylib → "libloft.rlib not
/// found for this build".  Keep aligned with `cache::loft_rlib_path` (#304).
pub(crate) fn rlib_search_dirs(
    exe_dir: &std::path::Path,
) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
    // Dependency rlibs live in `<profile>/deps/`.  Real binary run: `exe_dir` is
    // `<profile>`, so `deps/` is its child.  Integration test: `exe_dir` already IS
    // `.../deps`.  Either way the returned link-search dir is that `deps/`.
    let dev_deps = if crate::file_access::file_name(exe_dir).is_some_and(|n| n == "deps") {
        exe_dir.to_path_buf()
    } else {
        exe_dir.join("deps")
    };
    let mut out = vec![
        (dev_deps.clone(), dev_deps.clone()),
        (exe_dir.to_path_buf(), dev_deps.clone()),
    ];
    if crate::file_access::file_name(exe_dir).is_some_and(|n| n == "bin")
        && let Some(prefix) = exe_dir.parent()
    {
        let share = prefix.join("share").join("loft");
        out.push((share.join("deps"), share.join("deps")));
        out.push((share.clone(), share.join("deps")));
    }
    out
}

/// @PLN11 Arc N / N3 — locate the running build's `libloft.rlib` + its sibling
/// `deps/` directory, for linking an auto-generated cdylib against the **same**
/// libloft this process links (so `Stores`/`DbRef`/`LibArg` are ABI-identical).
///
/// Works in both contexts the cdylib must build in: a real `cargo run --bin loft`
/// (an unhashed `target/<prof>/libloft.rlib`, or `deps/`) and an integration test
/// (a hashed `libloft-<hash>.rlib` in the test binary's `deps/`).  Returns the
/// chosen rlib path and the `deps/` dir to add to the link search path.
///
/// The deps-first preference must stay aligned with `cache::rlib_candidates`
/// (#304): the fingerprint that validates a built cdylib has to hash the same
/// rlib this function links, or a cdylib gets validated against one loft and
/// built against another.
#[must_use]
pub fn find_loft_rlib() -> Option<(std::path::PathBuf, std::path::PathBuf)> {
    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    // Find `libloft.rlib` (unhashed) or the newest hashed `libloft-<hash>.rlib`,
    // returning the matching `deps/` dir as the link-search path.
    for (dir, deps_dir) in &rlib_search_dirs(&exe_dir) {
        if !crate::file_access::is_dir(dir) {
            continue;
        }
        let exact = dir.join("libloft.rlib");
        if crate::file_access::exists(&exact) {
            return Some((exact, deps_dir.clone()));
        }
        let hashed = crate::file_access::read_dir(dir)
            .ok()
            .into_iter()
            .flatten()
            .filter(|e| {
                let n = e.file_name().unwrap_or_default();
                n.starts_with("libloft-") && has_rlib_ext(n)
            })
            .max_by_key(|e| {
                crate::file_access::symlink_metadata(e)
                    .and_then(|m| m.modified())
                    .ok()
            })
            .map(|e| e.os_spelling());
        if let Some(rlib) = hashed {
            return Some((rlib, deps_dir.clone()));
        }
    }
    None
}

/// Does filename `n` have a (case-insensitive) `.rlib` extension?
pub(crate) fn has_rlib_ext(n: &str) -> bool {
    crate::file_access::extension(n).is_some_and(|e| e.eq_ignore_ascii_case("rlib"))
}

/// Is a working `rustc` on `PATH`?  Cached for the process.
///
/// This is the line between a legitimate interpret-fallback and a real failure: with
/// NO toolchain, loft cannot build native at all, so interpreting a library is the
/// only option and is graceful.  With a toolchain PRESENT, a native build that fails
/// is a genuine error — silently interpreting it would hand back a partly-interpreted
/// binary (or one whose `#native` functions panic at runtime) while the user asked for
/// native.  Callers gate the fallback on this.
#[must_use]
pub fn rustc_available() -> bool {
    rustc_version().is_some()
}

/// The first line `rustc --version` answers on `PATH`, or `None` when there is no working
/// `rustc` — [`crate::cache::rustc_version`], the one place loft asks it.
#[must_use]
pub fn rustc_version() -> Option<&'static str> {
    crate::cache::rustc_version()
}

/// @C102 — what a developer is told when this machine has no `rustc`.  A program run says
/// nothing (it succeeds on the interpreter); the moments that belong to the developer say
/// this: `loft --version`, the `loft test` summary, and `loft new --native`'s next steps.
pub const NO_RUSTC: &str = "no rustc on PATH, so native builds are unavailable \
                            (install one from https://rustup.rs)";

/// Detect ENVIRONMENT failures that masquerade as compiler/linker errors.
///
/// A `rustc`/`cc`/`ld` invocation that dies from a full temp dir or OOM emits a
/// cryptic crash, not a clear diagnosis: a SIGBUS from `ld` writing to a full
/// tmpfs (the common case — the linker mmaps object files into `TMPDIR`, and a
/// write the tmpfs can no longer back faults with a Bus error) reads like a
/// linker bug or a stale-artifact problem, when it is really "disk is full".
/// Scan a failed invocation's captured stderr for those signatures and return a
/// one-line, actionable hint to surface ABOVE the raw output; return `None` for
/// a genuine compile error so the real diagnostics show through untouched.
#[must_use]
pub fn toolchain_failure_hint(stderr: &str) -> Option<String> {
    let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    let env_fault = |what: &str| {
        Some(format!(
            "NOTE: the toolchain {what} — this is an ENVIRONMENT failure, not a \
             code error. The build temp dir is almost certainly full (or out of \
             memory). Free space on TMPDIR ({tmp}), or set TMPDIR to a larger \
             filesystem, then retry."
        ))
    };
    if stderr.contains("No space left on device") || stderr.contains("ENOSPC") {
        return env_fault("ran out of disk space");
    }
    if stderr.contains("Bus error")
        || stderr.contains("signal: 7")
        || stderr.contains("signal 7")
        || stderr.contains("SIGBUS")
    {
        return env_fault("crashed with a Bus error (SIGBUS), typically a full temp dir");
    }
    if stderr.contains("Cannot allocate memory")
        || stderr.contains("signal: 9")
        || stderr.contains("SIGKILL")
    {
        return env_fault("was killed (out of memory)");
    }
    // The known `loft_ffi` duplicate-crate collision: two copies of loft-ffi reach the
    // same link carrying the same StableCrateId, which rustc refuses.  Name it
    // explicitly so a consumer does not read the raw rustc dump as a bug in their own
    // code or in the library — it is neither, and loft falls back to interpreting.
    // loft#693 — the generated code calls a loft runtime method the linked
    // `libloft.rlib` does not have, i.e. this BINARY and that RLIB are from different
    // builds.  Left raw, the message blames the LIBRARY whose cdylib was being built
    // ("library 'gridmesh-0.2.0' failed to build native", `no method named
    // dbref_borrow`) — the one thing that is certainly not at fault, and a consumer
    // reasonably reads it as the library being broken and starts bisecting versions.
    // Naming the rlib in use is the fastest way to see the mismatch.
    if stderr.contains("E0599") && stderr.contains("Stores") {
        let rlib = find_loft_rlib().map_or_else(
            || "none found".to_string(),
            |(p, _)| p.display().to_string(),
        );
        return Some(format!(
            "NOTE: the generated code calls a loft runtime method that the \
             `libloft.rlib` being linked does NOT have, so this loft binary and that \
             rlib come from different builds.  This is not a bug in the library being \
             built, nor in your program.  Re-install the pair as one unit — `make all \
             && make install` in the loft tree.  The rlib in use is: {rlib}"
        ));
    }
    if stderr.contains("StableCrateId") {
        return Some(
            "NOTE: this is the known loft_ffi duplicate-crate collision — two copies of \
             loft-ffi reach the same link with the same StableCrateId, which rustc \
             refuses.  It is a BUILD/TOOLCHAIN limitation, NOT a bug in your code or in \
             this library.  To clear it, rebuild the library's cdylib against the CURRENT \
             loft — `make rebuild-native-cdylibs` in the loft tree, or `cargo build \
             --release` in the library's native/ dir — then re-run.  If it persists after \
             a clean rebuild, it is the tracked StableCrateId limitation, not your build."
                .to_string(),
        );
    }
    None
}

#[cfg(test)]
mod toolchain_hint_tests {
    use super::toolchain_failure_hint;

    #[test]
    fn flags_sigbus_as_environment_failure() {
        // The real shape that misled diagnosis: ld dies with a Bus error when
        // TMPDIR's tmpfs is full.  Must be named an ENVIRONMENT failure.
        let stderr = "collect2: fatal error: ld terminated with signal 7 [Bus error], core dumped\n\
                      error: aborting due to 1 previous error";
        let hint = toolchain_failure_hint(stderr).expect("SIGBUS must produce a hint");
        assert!(hint.contains("ENVIRONMENT"), "hint: {hint}");
        assert!(
            hint.contains("TMPDIR"),
            "hint should point at TMPDIR: {hint}"
        );
    }

    #[test]
    fn flags_enospc() {
        assert!(toolchain_failure_hint("error: No space left on device (os error 28)").is_some());
    }

    #[test]
    fn genuine_compile_error_gets_no_hint() {
        // A real type error must pass through untouched so its diagnostics show.
        let stderr = "error[E0308]: mismatched types\n  expected `i32`, found `&str`";
        assert!(toolchain_failure_hint(stderr).is_none());
    }

    /// loft#693 — verbatim rustc output from the reported failure: the installed
    /// `libloft.rlib` predated the borrowed-capture helper the binary emits calls to.
    /// Raw, it names the LIBRARY whose cdylib was building, which is the one thing not
    /// at fault, so the hint must say the binary and the rlib disagree.
    #[test]
    fn flags_a_runtime_method_the_linked_rlib_lacks() {
        let stderr = "error[E0599]: no method named `dbref_borrow` found for mutable \
                      reference `&mut Stores` in the current scope\n \
                      429 |     let dbref_borrow_w = db.dbref_borrow();\n\
                      error: aborting due to 1 previous error";
        let hint = toolchain_failure_hint(stderr).expect("a missing runtime method must hint");
        assert!(
            hint.contains("libloft.rlib") && hint.contains("different builds"),
            "the hint must name the rlib/binary mismatch: {hint}"
        );
        assert!(
            hint.contains("make install"),
            "the hint must name the fix: {hint}"
        );
        assert!(
            hint.contains("not a bug in the library"),
            "the hint must clear the library being built: {hint}"
        );
    }

    /// The guard above keys on E0599 + `Stores`; an E0599 in a library's OWN code must
    /// still pass through, or a real user error gets blamed on the install.
    #[test]
    fn unrelated_missing_method_gets_no_hint() {
        let stderr = "error[E0599]: no method named `frobnicate` found for struct \
                      `MyOwnType` in the current scope";
        assert!(toolchain_failure_hint(stderr).is_none());
    }
}

#[cfg(test)]
mod rlib_search_tests {
    use super::rlib_search_dirs;
    use std::path::{Path, PathBuf};

    /// #398 follow-up — the INSTALLED layout (`<prefix>/bin/loft`) must search
    /// `<prefix>/share/loft/` for libloft.rlib, else a normal library's cdylib link
    /// fails "libloft.rlib not found for this build" (the bin/ dir holds no rlib).
    #[test]
    fn installed_bin_layout_searches_share_loft() {
        let dirs = rlib_search_dirs(Path::new("/usr/local/bin"));
        let share = PathBuf::from("/usr/local/share/loft");
        assert!(
            dirs.iter()
                .any(|(d, deps)| *d == share && *deps == share.join("deps")),
            "install layout must search <prefix>/share/loft with its deps/: {dirs:?}"
        );
        assert!(
            dirs.iter().any(|(d, _)| *d == share.join("deps")),
            "install layout must also search <prefix>/share/loft/deps: {dirs:?}"
        );
    }

    /// The dev/test layout (exe in `<profile>/`) keeps the existing behaviour:
    /// `deps/` first, then the uplifted `<profile>/`, and NO share/ candidate.
    #[test]
    fn dev_layout_unchanged_no_share() {
        let dirs = rlib_search_dirs(Path::new("target/release"));
        assert_eq!(dirs[0].0, PathBuf::from("target/release/deps"));
        assert_eq!(dirs[1].0, PathBuf::from("target/release"));
        assert!(
            !dirs
                .iter()
                .any(|(d, _)| crate::file_access::PathText::from_os(d)
                    .parts()
                    .windows(2)
                    .any(|w| w[0] == "share" && w[1] == "loft")),
            "a dev exe must not gain a share/ candidate: {dirs:?}"
        );
        // every dev candidate links the same deps/ search dir
        assert!(
            dirs.iter()
                .all(|(_, deps)| deps.as_path() == std::path::Path::new("target/release/deps"))
        );
    }

    /// Integration-test layout: exe already in `.../deps` → deps dir is itself.
    #[test]
    fn deps_dir_exe_uses_itself() {
        let dirs = rlib_search_dirs(Path::new("target/release/deps"));
        assert_eq!(dirs[0].0, PathBuf::from("target/release/deps"));
        assert_eq!(dirs[0].1, PathBuf::from("target/release/deps"));
    }
}
