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

use crate::platform::process::{Program, Spawn};

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
    let (stem, paths, out_dir) = identity(pkg_dir, sources)?;
    let so = out_dir.join(crate::platform::cdylib_file_name(&stem));
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
    let mut cmd = Spawn::new(Program::os(cc_program()));
    cmd.push_arg("-O2").push_arg("-fPIC").push_arg("-shared");
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
    cmd.push_args(crate::platform::shim_cc_runtime_args());
    // `-Wall -Wextra` deliberately absent: a warning in the AUTHOR's C is theirs
    // to see when they compile it, not a reason for a consumer's build to look
    // broken. Errors still fail the build below.
    cmd.push_arg("-o").push_arg(&tmp);
    // macOS bakes the `-o` PATH into the library as its INSTALL NAME — the whole
    // path, not just the basename — so the staging directory would leak into it
    // even though the name is now right. Pin it explicitly to where the library
    // ends up. (Windows needs no counterpart: its recorded name follows the
    // basename, which the staging shape above already makes correct.)
    for arg in crate::platform::install_name_args(&final_name, crate::platform::host_lib_os()) {
        cmd.push_arg(arg);
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
            cmd.push_arg(arg);
        }
    }
    for p in &paths {
        cmd.push_arg(p);
    }
    let out = cmd.run(b"").map_err(|e| {
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

/// The shim's artefact identity: its stem (`<package>_shim_<key>`, the key a hash of the
/// sources and the compiler), its source paths, and the directory its artefacts land in.
/// One home for it, so the shared library and the static archive of the same sources can
/// never be keyed apart.
fn identity(
    pkg_dir: &str,
    sources: &[String],
) -> Result<(String, Vec<std::path::PathBuf>, std::path::PathBuf), String> {
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
    let digest = hasher.finalize();
    let key = u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8]));
    Ok((
        format!("{}_shim_{key:016x}", stem_of(pkg_dir)),
        paths,
        dir.join(OUT_DIR),
    ))
}

/// @PLN197 — the shim as a STATIC archive, beside the shared library of the same sources,
/// for a program that links its libraries into the binary (`--static`, `#link static`).
/// Built on demand: the interpreter and a dynamic program never need it.  Compiled by the
/// host's shim archiver (`platform::shim_archiver`) — `cc` + `ar`, or MSVC `cl` + `lib` on
/// Windows, whose program link cannot take the MinGW objects of the shared shim.  `Ok(None)`
/// where this host has no archiver; the static link then keeps the shim beside the binary.
///
/// # Errors
/// As [`build`]: an unreadable source, no compiler, a compile that fails, or no archiver.
pub fn archive(pkg_dir: &str, sources: &[String]) -> Result<Option<std::path::PathBuf>, String> {
    let Some(tools) = crate::platform::shim_archiver() else {
        return Ok(None);
    };
    archive_with(tools, pkg_dir, sources).map(Some)
}

/// @PLN203 — the shim's static archive for a unit the C backend writes, which `cc` links: built
/// by `cc` + `ar` on every host.  On Windows the host archiver of [`archive`] is MSVC's, whose
/// `.lib` MinGW's `ld` refuses ("corrupt .drectve"), so the C unit cannot share it.
///
/// # Errors
/// As [`archive`].
pub fn cc_archive(pkg_dir: &str, sources: &[String]) -> Result<std::path::PathBuf, String> {
    archive_with(&crate::platform::ShimArchiver::Unix, pkg_dir, sources)
}

fn archive_with(
    tools: &crate::platform::ShimArchiver,
    pkg_dir: &str,
    sources: &[String],
) -> Result<std::path::PathBuf, String> {
    let (stem, paths, out_dir) = identity(pkg_dir, sources)?;
    let name = tools.file_name(&stem);
    let archive = out_dir.join(&name);
    if crate::file_access::exists(&archive) {
        return Ok(archive);
    }
    crate::file_access::create_dir_all(&out_dir).map_err(|e| format!("cannot create {e}"))?;
    // Staged and renamed like the shared library, for the same reason: a concurrent
    // static link sees no archive or a complete one.
    let stage = out_dir.join(format!(".stage-a.{}", std::process::id()));
    crate::file_access::create_dir_all(&stage).map_err(|e| format!("cannot create {e}"))?;
    let fail = |msg: String| {
        let _ = crate::file_access::remove_dir_all(&stage);
        Err(msg)
    };
    // `cl` writes its diagnostics to stdout, `cc` and `ar` to stderr: the author needs
    // them whichever it was.
    let said = |o: &crate::platform::process::Ran| {
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        )
        .trim_end()
        .to_string()
    };
    let cc = cc_program();
    let mut objects = Vec::with_capacity(paths.len());
    for (i, p) in paths.iter().enumerate() {
        let obj = stage.join(format!("{i}.o"));
        match tools.compile(&cc, p, &obj).run(b"") {
            Ok(o) if o.status.success() => objects.push(obj),
            Ok(o) => return fail(format!("did not compile.\n{}", said(&o))),
            Err(e) => {
                return fail(format!(
                    "`[c] shim` needs a C compiler and `{}` could not be run ({e})",
                    tools.compiler(&cc)
                ));
            }
        }
    }
    let tmp = stage.join(&name);
    match tools.archive(&tmp, &objects).run(b"") {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            return fail(format!(
                "`{}` could not archive the shim.\n{}",
                tools.archiver(),
                said(&o)
            ));
        }
        Err(e) => {
            return fail(format!(
                "a static link of a `[c] shim` needs `{}` ({e})",
                tools.archiver()
            ));
        }
    }
    let published = crate::file_access::rename(&tmp, &archive).map_err(|e| {
        format!(
            "cannot publish the shim archive to `{}`: {e}",
            archive.display()
        )
    });
    let _ = crate::file_access::remove_dir_all(&stage);
    published.map(|()| archive)
}

/// @PLN197 — the static archive of a C library a package ships, which a static native link
/// puts into the binary: the package's `[c] shim`, archived on demand ([`archive`]), or a
/// `lib<stem>.a` the package ships beside the shared library.  `None` when there is none, or
/// where this host has no archive of that kind (`platform::shim_archiver`,
/// `platform::c_archive_file_name`).
#[must_use]
pub fn static_archive_for(
    lib: &crate::data::CLibrary,
    path: &std::path::Path,
) -> Option<std::path::PathBuf> {
    if let Some(m) = crate::manifest::read_manifest(&format!("{}/loft.toml", lib.pkg_dir))
        && !m.c_shim.is_empty()
        && build(&lib.pkg_dir, &m.c_shim).ok().as_deref() == Some(path)
    {
        return match archive(&lib.pkg_dir, &m.c_shim) {
            Ok(a) => a,
            Err(why) => {
                eprintln!("loft: `[c] shim` for `{}`: {why}", lib.pkg_dir);
                None
            }
        };
    }
    let file = crate::file_access::file_name(path)?;
    let archive = path
        .parent()?
        .join(crate::platform::c_archive_file_name(&library_stem(&file))?);
    crate::file_access::exists(&archive).then_some(archive)
}

/// @PLN197 — does a static link put the C libraries the package at `pkg_dir` ships INTO the
/// binary?  Only when every one of them has a static archive ([`static_archive_for`]).  A
/// `#c` symbol is attributed to its package, never to one of its libraries, so a package
/// whose libraries are split — one inside the binary, one bound at start — would leave its
/// symbols nowhere a lookup by name can reach: a symbol linked into a binary is not in its
/// dynamic symbol table.  Such a package keeps all of them bound instead.
#[must_use]
pub fn archived_package(data: &crate::data::Data, pkg_dir: &str) -> bool {
    let shipped: Vec<_> = data
        .shipped_c_libraries()
        .into_iter()
        .filter(|(c, _)| c.pkg_dir == pkg_dir)
        .collect();
    !shipped.is_empty()
        && shipped
            .iter()
            .all(|(c, path)| static_archive_for(c, path).is_some())
}

/// @PLN197 — which `#c` symbols the C libraries a package ships DEFINE, asked of their
/// shared form on the build machine.  A package that also declares an optional library has
/// every `#c` symbol resolved by name at run time (`Data::c_symbol_is_lazy`), and under a
/// static link that cannot reach its archived shim: nothing references the archive's
/// objects at link time, so the linker leaves them out.  The symbols the shipped libraries
/// define are declared `extern "C"` instead, which links them; the rest stay lazy, for the
/// optional library.  The shared library and the archive are built from the same sources,
/// so they define the same symbols.  The libraries are loaded once per package.
#[derive(Default)]
pub struct ShippedExports {
    #[cfg(feature = "native-extensions")]
    loaded: std::collections::HashMap<String, Vec<libloading::Library>>,
}

impl ShippedExports {
    /// Does a C library the package at `pkg_dir` ships define `symbol`?
    #[cfg(feature = "native-extensions")]
    pub fn defines(&mut self, data: &crate::data::Data, pkg_dir: &str, symbol: &str) -> bool {
        let libs = self.loaded.entry(pkg_dir.to_string()).or_insert_with(|| {
            data.shipped_c_libraries()
                .into_iter()
                .filter(|(c, _)| c.pkg_dir == pkg_dir)
                // SAFETY: the package's own library, the one `--interpret` loads to run it.
                .filter_map(|(_, path)| unsafe { crate::file_access::load_library(&path) }.ok())
                .collect()
        });
        libs.iter().any(|l| {
            // SAFETY: only the address is asked for; nothing is called through it.
            unsafe { l.get::<*const ()>(symbol.as_bytes()) }.is_ok()
        })
    }

    /// Without a loader nothing can be attributed: every symbol stays lazy.
    #[cfg(not(feature = "native-extensions"))]
    pub fn defines(&mut self, _data: &crate::data::Data, _pkg_dir: &str, _symbol: &str) -> bool {
        false
    }
}

/// `libfoo.so.5` / `libfoo.dylib` / `foo.dll` / `libfoo.a` -> `foo`: the library's name as
/// `-l` takes it — the file's name without the `lib` the linker adds back and without any
/// extension or version.
#[must_use]
pub fn library_stem(file: &str) -> String {
    let base = file.strip_prefix("lib").unwrap_or(file);
    [".so", ".dylib", ".dll", ".a"]
        .iter()
        .find_map(|ext| base.split_once(ext).map(|(stem, _)| stem))
        .unwrap_or(base)
        .to_string()
}

/// The C compiler to drive: `CC` when set, else `cc`.
fn cc_program() -> String {
    std::env::var("CC").unwrap_or_else(|_| "cc".to_string())
}

/// A short identity for the compiler, folded into the artifact key. Its own
/// version banner, or the program name when it will not report one.
fn cc_identity() -> String {
    // Asked once per process: every artefact lookup keys on it, and a static link asks for
    // the shim and its archive several times.
    static IDENTITY: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    IDENTITY.get_or_init(cc_identity_uncached).clone()
}

fn cc_identity_uncached() -> String {
    Spawn::new(Program::os(cc_program()))
        .arg("--version")
        .run(b"")
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

/// The shim route — the shim as wasm32 OBJECTS for a wasm target
/// (`--native-wasm`, `--html`): each source compiled by a C cross-compiler
/// (`LOFT_WASM_CC`, else `clang`) freestanding against the headers of `src/c/wasm/`, plus
/// loft's own freestanding C library (`loft_wasm_libc.c`) built the same way, so one set of
/// objects links into the WASI module beside Rust's libc and into the browser module where
/// there is none.  A `#c` call then reaches the shim's symbol directly, which is what makes
/// a `[c] shim` library a wasm library too; a declared `[c] libs` entry still has no route.
/// Cached under `native-auto/`, keyed as [`build`] keys the shared library plus the
/// cross-compiler's identity.
///
/// # Errors
/// An unreadable source, no cross-compiler, or a compile that fails — a shim that opens a
/// file or needs a function this library does not carry is one such failure, and the right
/// one: the browser has no files.
pub fn wasm_objects(pkg_dir: &str, sources: &[String]) -> Result<Vec<std::path::PathBuf>, String> {
    let (stem, paths, out_dir) = identity(pkg_dir, sources)?;
    let cc = wasm_cc_program();
    let key = crate::cache::stable_hash(&[
        wasm_cc_identity(&cc).as_bytes(),
        WASM_LIBC.as_bytes(),
        WASM_STRING_H.as_bytes(),
        WASM_STDLIB_H.as_bytes(),
        WASM_STDIO_H.as_bytes(),
    ]);
    let dir = out_dir.join(format!("{stem}.wasm32-{key:016x}"));
    let mut objects: Vec<std::path::PathBuf> = (0..paths.len())
        .map(|i| dir.join(format!("{i}.o")))
        .collect();
    objects.push(dir.join("loft_wasm_libc.o"));
    if objects.iter().all(crate::file_access::exists) {
        return Ok(objects);
    }
    // Staged and renamed whole, as the shared library is: a concurrent build sees no
    // directory or a complete one.
    let stage = out_dir.join(format!(".stage-w.{}", std::process::id()));
    let include = stage.join("include");
    crate::file_access::create_dir_all(&include).map_err(|e| format!("cannot create {e}"))?;
    let fail = |msg: String| {
        let _ = crate::file_access::remove_dir_all(&stage);
        Err(msg)
    };
    for (name, text) in [
        ("string.h", WASM_STRING_H),
        ("stdlib.h", WASM_STDLIB_H),
        ("stdio.h", WASM_STDIO_H),
        ("loft_wasm_libc.c", WASM_LIBC),
    ] {
        if let Err(e) = crate::file_access::write(include.join(name), text) {
            return fail(format!("cannot write {name}: {e}"));
        }
    }
    let mut sources_to_build: Vec<(std::path::PathBuf, std::path::PathBuf)> = paths
        .iter()
        .enumerate()
        .map(|(i, p)| (p.clone(), stage.join(format!("{i}.o"))))
        .collect();
    sources_to_build.push((
        include.join("loft_wasm_libc.c"),
        stage.join("loft_wasm_libc.o"),
    ));
    for (src, obj) in &sources_to_build {
        let run = Spawn::new(Program::os(&cc))
            .arg("--target=wasm32-unknown-unknown")
            .arg("-ffreestanding")
            .arg("-nostdlib")
            .arg("-O2")
            .arg("-fvisibility=default")
            .arg("-I")
            .arg(&include)
            .arg("-c")
            .arg(src)
            .arg("-o")
            .arg(obj)
            .run(b"");
        match run {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                return fail(format!(
                    "`{}` does not compile for wasm32.\n{}",
                    src.display(),
                    String::from_utf8_lossy(&o.stderr).trim_end()
                ));
            }
            Err(e) => {
                return fail(format!(
                    "a `[c] shim` on a wasm target needs a C compiler that targets wasm32 \
                     (`clang`; LOFT_WASM_CC names another) and `{cc}` could not be run ({e})"
                ));
            }
        }
    }
    let _ = crate::file_access::remove_dir_all(&include);
    if !crate::file_access::exists(&dir) && crate::file_access::rename(&stage, &dir).is_err() {
        // Another build published the same directory first; its objects are the same.
        let _ = crate::file_access::remove_dir_all(&stage);
    }
    if objects.iter().all(crate::file_access::exists) {
        Ok(objects)
    } else {
        Err(format!(
            "the wasm objects were not published to {}",
            dir.display()
        ))
    }
}

/// The freestanding C library and headers a shim sees on a wasm target (`src/c/wasm/`).
const WASM_LIBC: &str = include_str!("c/wasm/loft_wasm_libc.c");
const WASM_STRING_H: &str = include_str!("c/wasm/string.h");
const WASM_STDLIB_H: &str = include_str!("c/wasm/stdlib.h");
const WASM_STDIO_H: &str = include_str!("c/wasm/stdio.h");

fn wasm_cc_program() -> String {
    std::env::var("LOFT_WASM_CC").unwrap_or_else(|_| "clang".to_string())
}

fn wasm_cc_identity(cc: &str) -> String {
    Spawn::new(Program::os(cc))
        .arg("--version")
        .run(b"")
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(
            || cc.to_string(),
            |o| {
                String::from_utf8_lossy(&o.stdout)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string()
            },
        )
}
