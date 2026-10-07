// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader: the ANSI-C shim `#c` libraries compile (@PLN24 arc D).

//! @PLN24 arc D — build the ANSI-C shim a `#c` library ships.
//!
//! The plan's trade is that loft-core stays a generic linking tool and every
//! signature the fixed trampolines cannot express — a `double` argument, a
//! struct by value, varargs, an out-parameter, a caller-frees `char *` return —
//! is wrapped by a few lines of C the library ships. That trade only holds if
//! loft can BUILD those lines: a shim loft cannot compile is a claim, not an
//! escape hatch, and the author's alternative is the rustc toolchain the whole
//! plan exists to avoid.
//!
//! **A built shim IS a `[c] libs` entry.** It is compiled to a shared library
//! and then registered exactly like a declared one, so the interpreter's
//! `dlopen`, the `--native` link line, and the symbol resolver all keep one
//! code path. Nothing downstream can tell a shim from any other C library,
//! which is what stops the shim becoming a second mechanism to keep in sync —
//! the failure this plan counts (`N × silence`) at every re-assertion site.
//!
//! `cc`, never rustc. That is the whole point.

/// Where a built shim lands, beside the package that declared it.
///
/// The same directory the auto-built Rust cdylibs use, so one `.gitignore` and
/// one "delete this to force a rebuild" answer covers both.
const OUT_DIR: &str = "native-auto";

/// Build (or reuse) the shim for `pkg_dir` from `sources`, returning the shared
/// library's path.
///
/// The artifact is **content-addressed**: its name carries a hash of the shim
/// sources, so editing a shim produces a different file rather than racing to
/// overwrite the one another process may be reading — the same reasoning that
/// made the Rust cdylibs content-addressed (loft#715), and it costs nothing to
/// apply here. An existing artifact with the right name IS the freshness check.
///
/// # Errors
/// Returns a message naming the cure when a source is unreadable, when `cc` is
/// absent, or when the compile fails (carrying `cc`'s own diagnostics — the
/// author needs the C error, not our summary of it).
pub fn build(pkg_dir: &str, sources: &[String]) -> Result<std::path::PathBuf, String> {
    if sources.is_empty() {
        return Err("no shim sources".to_string());
    }
    let dir = std::path::Path::new(pkg_dir);
    use sha2::{Digest, Sha256};
    let mut paths = Vec::with_capacity(sources.len());
    let mut hasher = Sha256::new();
    for s in sources {
        let p = dir.join(s);
        let bytes = crate::file_access::read(&p).map_err(|e| {
            format!(
                "names `{s}`, which cannot be read ({e}) — the path resolves against \
                 the package directory"
            )
        })?;
        hasher.update(s.as_bytes());
        hasher.update(&bytes);
        paths.push(p);
    }
    // The compiler identity belongs in the key: the same source built by a
    // different `cc` is a different artifact, and reusing one across a toolchain
    // change is how a stale ABI survives an upgrade.
    hasher.update(cc_identity().as_bytes());

    let out_dir = dir.join(OUT_DIR);
    let digest = hasher.finalize();
    let key = u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8]));
    let stem = format!("{}_shim_{key:016x}", stem_of(pkg_dir));
    let so = out_dir.join(crate::native_lib::platform_cdylib_name(&stem));
    if crate::file_access::exists(&so) {
        return Ok(so);
    }
    crate::file_access::create_dir_all(&out_dir).map_err(|e| format!("cannot create {e}"))?;

    // Build in a unique temporary DIRECTORY and rename over: the publish is
    // atomic, so a concurrent reader sees either no file or a complete one,
    // never a partial library it would `dlopen`.
    //
    // The artifacts already carry their FINAL names in there, because a linker
    // records the name it was given. Building `<stem>.<pid>.tmp` and renaming
    // afterwards moves the file and leaves that recorded name behind, so every
    // consumer went looking for a temporary that no longer existed and Windows
    // refused to start the process with `STATUS_DLL_NOT_FOUND` — naming nothing,
    // because a missing import names no name. Measured on the runner: the
    // program imported `lcshim_shim_<key>.8496.tmp` while
    // `lcshim_shim_<key>.dll` sat beside it (loft#…, WINDOWS.md).
    //
    // A temp DIRECTORY rather than a temp FILENAME is what buys both: the
    // recorded name follows the BASENAME (verified — a staging path does not
    // leak into it), and the rename still lands in the same directory, so it is
    // still atomic. `-Wl,--soname` does NOT fix this: PE ignores it, verified
    // side by side with this shape.
    let final_name = crate::file_access::file_name(&so).unwrap_or_else(|| stem.clone());
    let stage = out_dir.join(format!(".stage.{}", std::process::id()));
    crate::file_access::create_dir_all(&stage).map_err(|e| format!("cannot create {e}"))?;
    let tmp = stage.join(&final_name);
    let mut cmd = std::process::Command::new(cc_program());
    cmd.arg("-O2").arg("-fPIC").arg("-shared");
    // A shim must not drag its COMPILER's runtime along. A MinGW `cc` links
    // `libgcc_s_seh-1.dll` (and `libwinpthread-1.dll`) by default, so the shim
    // loads only where MinGW's `bin` is on PATH — which the build machine has and
    // a consumer does not. Measured: the shim was staged correctly beside the
    // binary and Windows still refused to start the process
    // (`STATUS_DLL_NOT_FOUND`), because the missing import was the runtime and
    // not the shim.
    //
    // Static-linking it is the fix rather than staging two more DLLs: the shims
    // are three-line trampolines, the runtime they actually use is a rounding
    // error, and a self-contained artifact is one fewer thing to ship. Harmless
    // where it does not apply — a non-MinGW `cc` has nothing to statically link.
    cmd.args(crate::platform::shim_cc_runtime_args());
    // `-Wall -Wextra` deliberately absent: a warning in the AUTHOR's C is theirs
    // to see when they compile it, not a reason for a consumer's build to look
    // broken. Errors still fail the build below.
    cmd.arg("-o").arg(&tmp);
    // macOS bakes the `-o` PATH into the library as its INSTALL NAME — the whole
    // path, not just the basename — so the staging directory would leak into it
    // even though the name is now right. Pin it explicitly to where the library
    // ends up. (Windows needs no counterpart: its recorded name follows the
    // basename, which the staging shape above already makes correct.)
    for arg in crate::platform::install_name_args(&final_name, crate::platform::host_lib_os()) {
        cmd.arg(arg);
    }
    // Windows links against an IMPORT LIBRARY, not the DLL, and `cc -shared`
    // writes one only when asked. Built in the staging directory beside the
    // DLL's own and renamed with it below, so the pair is published together — a
    // consumer that found the `.dll` and not yet the `.lib` would fail exactly
    // as if the flag had never been added.
    //
    // This is the file that carried the bug: the import library records the DLL
    // name from the linker's `-o`, and every binary linking it copies that
    // string in as the thing to ask the loader for.
    let implib = crate::platform::shim_implib_name(&final_name, crate::platform::host_lib_os())
        .map(|name| (stage.join(&name), out_dir.join(name)));
    if let Some((tmp_lib, _)) = &implib {
        for arg in crate::platform::shim_implib_args(
            &tmp_lib.to_string_lossy(),
            crate::platform::host_lib_os(),
        ) {
            cmd.arg(arg);
        }
    }
    for p in &paths {
        cmd.arg(p);
    }
    let out = cmd.output().map_err(|e| {
        format!(
            "`[c] shim` needs a C compiler and `{}` could not be run ({e}). Install one \
             (`cc`/`gcc`/`clang`), or set CC to the one to use",
            cc_program()
        )
    })?;
    if !out.status.success() {
        let _ = crate::file_access::remove_dir_all(&stage);
        // The caller already names the package; repeating it here only pushed
        // cc's own diagnostics further down the line, and those are what the
        // author needs to read first.
        return Err(format!(
            "did not compile.\n{}",
            String::from_utf8_lossy(&out.stderr).trim_end()
        ));
    }
    // The import library first: the DLL's rename is what makes the shim visible
    // to the cache check at the top, so anything that must accompany it has to
    // already be in place when that happens.
    if let Some((tmp_lib, final_lib)) = &implib {
        crate::file_access::rename(tmp_lib, final_lib).map_err(|e| {
            let _ = crate::file_access::remove_dir_all(&stage);
            format!(
                "the shim built but its import library `{}` could not be published: {e}",
                final_lib.display()
            )
        })?;
    }
    // A rename onto an existing file is fine — same content-addressed name means
    // the same bytes, so whoever wins publishes an identical library.
    let published = crate::file_access::rename(&tmp, &so)
        .map_err(|e| format!("cannot publish the shim to `{}`: {e}", so.display()));
    // The staging directory goes whatever happened: it is named for this process,
    // so nothing else will ever reuse it, and a leftover would accumulate one
    // empty directory per build.
    let _ = crate::file_access::remove_dir_all(&stage);
    published?;
    Ok(so)
}

/// The C compiler to drive: `CC` when set, else `cc`.
fn cc_program() -> String {
    std::env::var("CC").unwrap_or_else(|_| "cc".to_string())
}

/// A short identity for the compiler, folded into the artifact key. Its own
/// version banner, or the program name when it will not report one.
fn cc_identity() -> String {
    std::process::Command::new(cc_program())
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(cc_program, |o| {
            String::from_utf8_lossy(&o.stdout)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string()
        })
}

/// A filesystem-safe stem for the package directory, so the artifact names the
/// library it belongs to rather than a bare hash.
fn stem_of(pkg_dir: &str) -> String {
    let raw = crate::file_access::file_name(pkg_dir).unwrap_or_else(|| "shim".to_string());
    let cleaned: String = raw
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    if cleaned.is_empty() {
        "shim".to_string()
    } else {
        cleaned
    }
}
