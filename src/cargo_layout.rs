// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: where cargo put a build

//! Where a cargo build put a crate's outputs, under either build-dir layout: the classic one
//! (one `target/<profile>/deps/`) and cargo 1.100's per-unit one (each crate in
//! `target/<profile>/build/<crate>/<hash>/out/`, no `deps/`).  The dependency search dirs, the
//! build-script units, the profile a binary belongs to, and which rustc wrote an rlib — one home
//! for each question, low in the layering so the toolchain, the cache and the platform code ask
//! it as the native writer does.

/// What a crate-metadata header says about the compile that wrote it: the crate's metadata
/// hash (format 11 on; `None` before it) and the rustc version string.  Read from the `rust`
/// blob rustc writes at the head of every `lib.rmeta` and `.rmeta`: the magic and a format
/// version, the offsets, from format 11 the 16-byte hash, then the version as a
/// length-prefixed string.
#[derive(PartialEq, Eq, Debug)]
pub(crate) struct MetaHeader {
    pub(crate) hash: Option<[u8; 16]>,
    pub(crate) rustc: String,
}

pub(crate) fn meta_header_of_blob(blob: &[u8]) -> Option<MetaHeader> {
    const MAGIC: &[u8] = b"rust\0\0\0";
    let at = blob.windows(MAGIC.len()).position(|w| w == MAGIC)?;
    let format = *blob.get(at + 7)?;
    // Format 10 puts the version string right after the root offset; 11 adds a second
    // offset and the hash before it.
    let len_at = at + if format >= 11 { 40 } else { 16 };
    let hash = if format >= 11 {
        Some(blob.get(at + 24..at + 40)?.try_into().ok()?)
    } else {
        None
    };
    let len = usize::from(*blob.get(len_at)?);
    let rustc = std::str::from_utf8(blob.get(len_at + 1..len_at + 1 + len)?).ok()?;
    rustc.starts_with("rustc ").then(|| MetaHeader {
        hash,
        rustc: rustc.to_string(),
    })
}

/// The metadata header of an `.rmeta` file: its first few KiB, never the whole file (the full
/// metadata of loft's own crate is tens of MB).
pub(crate) fn rmeta_meta_header(rmeta: &std::path::Path) -> Option<MetaHeader> {
    use std::io::Read;
    let mut head = Vec::with_capacity(4096);
    crate::file_access::open(rmeta)
        .ok()?
        .take(4096)
        .read_to_end(&mut head)
        .ok()?;
    meta_header_of_blob(&head)
}

/// The metadata header of an rlib's `lib.rmeta` member.  The archive is walked by SEEKING past
/// each member — a stable rlib opens with a symbol table of several MB, and the rlib itself is
/// tens of MB — and only the member's first few KiB are read.
pub(crate) fn rlib_meta_header(rlib: &std::path::Path) -> Option<MetaHeader> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = crate::file_access::open(rlib).ok()?;
    let mut magic = [0u8; 8];
    f.read_exact(&mut magic).ok()?;
    if &magic != b"!<arch>\n" {
        return None;
    }
    let mut header = [0u8; 60];
    while f.read_exact(&mut header).is_ok() {
        let name = std::str::from_utf8(&header[0..16]).ok()?.trim_end();
        let size: u64 = std::str::from_utf8(&header[48..58])
            .ok()?
            .trim_end()
            .parse()
            .ok()?;
        if name.strip_suffix('/').unwrap_or(name) == "lib.rmeta" {
            let mut head = Vec::with_capacity(4096);
            f.take(size.min(4096)).read_to_end(&mut head).ok()?;
            return meta_header_of_blob(&head);
        }
        f.seek(SeekFrom::Current(i64::try_from(size + size % 2).ok()?))
            .ok()?;
    }
    None
}

/// Was this rlib written by a rustc OTHER than the one that will compile against it?  `false`
/// whenever either side is unknown, so an unreadable header keeps the old choice.
pub(crate) fn rlib_from_other_rustc(rlib: &std::path::Path) -> bool {
    match (rlib_meta_header(rlib), crate::cache::rustc_version()) {
        (Some(h), Some(current)) => h.rustc != current,
        _ => false,
    }
}

/// Is `<profile>/deps/` debris: the profile's uplifted `libloft.rlib` is a per-unit output (a
/// hard link into `build/<crate>/<hash>/out/`), so the per-unit layout built it and a `deps/`
/// beside it was left by an older cargo or toolchain (loft#2033)?
#[must_use]
pub fn classic_deps_is_debris(profile_dir: &std::path::Path) -> bool {
    let rlib = profile_dir.join("libloft.rlib");
    crate::file_access::exists(&rlib)
        && per_unit_dep_dirs(profile_dir)
            .iter()
            .any(|d| crate::file_access::same_file_identity(d.join("libloft.rlib"), &rlib))
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
/// * **per-unit** (cargo 1.99 nightlies and later) — each crate gets its own
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
#[must_use]
pub fn dependency_dirs(lib_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let classic = deps_dir_of(lib_dir);
    let per_unit = per_unit_dep_dirs(lib_dir);
    // A `deps/` beside a per-unit tree is debris when the rlib being linked IS a per-unit
    // output (cargo uplifts it as a hard link): a cache restored from an older toolchain put
    // it there, and searching it resolved loft's dependencies against that toolchain's
    // rlibs (loft#2033).  Otherwise `deps/` is the layout, as before.
    let rlib = lib_dir.join("libloft.rlib");
    let linked_is_per_unit = per_unit
        .iter()
        .any(|d| crate::file_access::same_file_identity(d.join("libloft.rlib"), &rlib));
    if crate::file_access::is_dir(&classic) && !linked_is_per_unit {
        return vec![classic];
    }
    if per_unit.is_empty() {
        // Name the place it was expected, so the diagnostic stays actionable.
        return vec![classic];
    }
    per_unit
}

/// The per-unit layout's search dirs: every `<profile>/build/<crate>/<hash>/out/` that holds a
/// linkable crate, sorted.  Empty under the classic layout.
pub(crate) fn per_unit_dep_dirs(lib_dir: &std::path::Path) -> Vec<std::path::PathBuf> {
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
    if let Ok(crates) = crate::file_access::read_dir(&build_root) {
        for c in crates {
            let Ok(hashes) = crate::file_access::read_dir(&c) else {
                continue;
            };
            for h in hashes {
                let out = h.os_spelling().join("out");
                if crate::file_access::read_dir(&out).is_ok_and(|rd| {
                    rd.iter().any(|f| {
                        crate::file_access::extension(f).is_some_and(|e| {
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
    dirs.sort();
    dirs
}

#[must_use]
pub fn deps_dir_of(lib_dir: &std::path::Path) -> std::path::PathBuf {
    if crate::file_access::file_name(lib_dir).is_some_and(|n| n == "deps") {
        lib_dir.to_path_buf()
    } else {
        lib_dir.join("deps")
    }
}

/// Every build-script unit under `<profile>/build/`: its `out/` dir and the file holding the
/// script's `cargo:` directives.  Classic layout: `build/<crate>-<hash>/out` and
/// `build/<crate>-<hash>/output`; cargo's per-unit layout (1.100 stable):
/// `build/<crate>/<hash>/out` and `build/<crate>/<hash>/run/stdout`.  A unit with neither file
/// is answered with its `out/` alone (`None`), as a library unit's.
#[must_use]
pub fn build_units(
    profile: &std::path::Path,
) -> Vec<(std::path::PathBuf, Option<std::path::PathBuf>)> {
    let mut out = Vec::new();
    let Ok(entries) = crate::file_access::read_dir(profile.join("build")) else {
        return out;
    };
    for entry in entries {
        let dir = entry.os_spelling();
        let classic = dir.join("output");
        if crate::file_access::exists(&classic) || crate::file_access::is_dir(dir.join("out")) {
            out.push((
                dir.join("out"),
                crate::file_access::exists(&classic).then_some(classic),
            ));
            continue;
        }
        let Ok(units) = crate::file_access::read_dir(&dir) else {
            continue;
        };
        for unit in units {
            let u = unit.os_spelling();
            let stdout = u.join("run").join("stdout");
            out.push((
                u.join("out"),
                crate::file_access::exists(&stdout).then_some(stdout),
            ));
        }
    }
    out
}

/// The build profile directory a binary at `exe_dir` belongs to: `exe_dir` itself, except under
/// cargo's per-unit build-dir layout (cargo 1.100 stable), where a test or example binary runs
/// from `<profile>/build/<crate>/<hash>/out/` — three levels below the profile that holds the
/// runtime (`refresh_runtime_uplift` puts `libloft.rlib` there) and its dependency dirs.
#[must_use]
pub fn runtime_profile_dir(exe_dir: &std::path::Path) -> std::path::PathBuf {
    let name = |p: &std::path::Path| crate::file_access::file_name(p).unwrap_or_default();
    if name(exe_dir) == "out"
        && let Some(unit) = exe_dir.parent()
        && let Some(krate) = unit.parent()
        && let Some(build) = krate.parent()
        && name(build) == "build"
        && let Some(profile) = build.parent()
    {
        return profile.to_path_buf();
    }
    exe_dir.to_path_buf()
}

/// [`runtime_profile_dir`] of the running binary.
#[must_use]
pub fn current_runtime_profile_dir() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(runtime_profile_dir(exe.parent()?))
}

/// Make `<profile>/libloft.rlib` the runtime the last build produced, as a cargo uplift does.
///
/// Every lookup links the rlib at `<profile>/libloft.rlib` (and its `.rmeta` beside it), and
/// derives the search path from that directory.  A cargo on the per-unit layout writes the
/// rlib only to `<profile>/build/loft/<unit>/out/` and does not uplift it, so that path is
/// either missing (a fresh tree) or a file an older build left there (a restored cache): a
/// compile against it fails with `E0514` when another rustc wrote it, or with `E0425` on the
/// runtime functions it lacks when the same rustc wrote it from older source.  Cargo's own
/// rule is the one kept: the uplift is the NEWEST unit.  When the uplift is missing, or a
/// unit newer than it is not the same file, that unit and its `.rmeta` are linked into place
/// through a temporary name and a rename, so a concurrent reader sees the old file or the new
/// one.  The rustc is not compared: the shape that builds with nightly (the threaded browser
/// runtime) is compiled against by nightly while `PATH` answers stable, so "the live rustc"
/// is not a question this lookup can ask.  A tree cargo still uplifts in has no units, and an
/// uplift that IS the newest unit (a hard link) is left as it is.
pub fn refresh_runtime_uplift(profile: &std::path::Path) {
    let uplifted = profile.join("libloft.rlib");
    let Ok(units) = crate::file_access::read_dir(profile.join("build").join("loft")) else {
        return;
    };
    let modified = |p: &std::path::Path| {
        crate::file_access::metadata(p)
            .and_then(|m| m.modified())
            .ok()
    };
    let newest = units
        .iter()
        .map(|u| u.os_spelling().join("out").join("libloft.rlib"))
        .filter(|r| crate::file_access::exists(r))
        .max_by_key(|r| modified(r));
    let Some(unit_rlib) = newest else {
        return;
    };
    if crate::file_access::exists(&uplifted)
        && (same_file(&unit_rlib, &uplifted) || modified(&unit_rlib) <= modified(&uplifted))
    {
        return;
    }
    let unit_rmeta = crate::file_access::with_extension(&unit_rlib, "rmeta");
    let sibling_rmeta = profile.join("libloft.rmeta");
    if crate::file_access::exists(&unit_rmeta) {
        let _ = place(&unit_rmeta, &sibling_rmeta);
    } else {
        // A leftover `.rmeta` beside a self-contained rlib is a second candidate for one crate.
        let _ = crate::file_access::remove_file(&sibling_rmeta);
    }
    let _ = place(&unit_rlib, &uplifted);
}

/// `from` at `to`, by hard link where the filesystem allows one and a copy where not, through
/// a name of this process's own so the rename is the only step a reader can observe.
fn place(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    let ext = crate::file_access::extension(to).unwrap_or_default();
    let tmp = crate::file_access::with_extension(to, &format!("{ext}.tmp{}", std::process::id()));
    let _ = crate::file_access::remove_file(&tmp);
    if crate::file_access::hard_link(from, &tmp).is_err() {
        crate::file_access::copy(from, &tmp)?;
    }
    crate::file_access::rename(&tmp, to)
}

/// Are these two paths the SAME file?  Hard-link identity first (which is what an uplift
/// produces, and is free), then content — a copy is as good as a link for this question,
/// and answering `false` only costs the caller the metadata it was looking for.
pub fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    let (Ok(ma), Ok(mb)) = (
        crate::file_access::metadata(a),
        crate::file_access::metadata(b),
    ) else {
        return false;
    };
    if let (Some(ia), Some(ib)) = (
        crate::platform::file_identity(&ma),
        crate::platform::file_identity(&mb),
    ) && ia == ib
    {
        return true;
    }
    if ma.len() != mb.len() {
        return false;
    }
    match (crate::file_access::read(a), crate::file_access::read(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}
