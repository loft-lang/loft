// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader / generator

//! @PLN11 Arc N / N2 — auto-generate a native cdylib from a library's functions.
//!
//! In the native-library execution model (@C71) a stable library compiles to a
//! cdylib and the interpreter dispatches calls to it (the stdlib `#native` path,
//! `OpStaticCall` → dlsym).  This module generates that cdylib's `lib.rs`.  Two
//! dispatch ABIs, by what crosses the boundary:
//!
//! **Scalar slice** (`generate_cdylib_lib_rs`, `scalar_dispatchable`): params +
//! return are scalars, so no store reference crosses.  The export wrapper
//! `loft_n_<name>(scalars) -> ret` stands up a **per-call** `UnsafeCell<Stores>` +
//! `init()` and forwards — safe because a scalar-return body cannot leak a store
//! ref out (any internal allocation is contained and dropped with the cell).  This
//! is ABI-identical to a hand-written scalar `#native` symbol, so it reuses the
//! existing dispatch wholesale.
//!
//! **Store-touching slice** (`generate_shared_cdylib_lib_rs`,
//! `shared_store_dispatchable`): a non-scalar value (`vector`/`reference`) crosses
//! the boundary.  Because an auto-generated cdylib **links libloft**, its `Stores`
//! / `DbRef` are the *same Rust types* as the interpreter's — so the bridge
//! **shares the caller's real `Stores` by pointer** (zero-marshalling, @C71's
//! promise) rather than going through the `LoftStore` FFI handle (that handle
//! exists for hand-written cdylibs that don't link `Stores`).  The bridge wrapper
//! `loft_shared_n_<name>(stores: *mut Stores, args, n, ret)` casts `*mut Stores` →
//! `&UnsafeCell<Stores>` (`UnsafeCell` is `repr(transparent)`) and forwards — **no
//! per-call cell, no `init`, no marshalling** (the caller's store is live; `DbRef`
//! args are already valid in it).  Args/return are passed through the uniform
//! [`LibArg`] slot.

use crate::data::{Context, Data, DefType, Type};
use crate::database::Stores;
use crate::generation::{Output, returns_owned_string, rust_type};
pub(crate) use crate::native_gate::is_text_work_buffer;
use crate::opt::native_key::{linked_packages, native_pkg_lib};
use crate::platform::process::{Program, Spawn};
use std::collections::{BTreeSet, HashMap, HashSet};

/// @PLN11 Arc N / N2 (lean interface) — generate the loft-source **interface** a
/// script adopts to call a native library: the public type definitions the
/// exported functions reference (transitively) plus, per exported function, a
/// `#native "loft_shared_…"` forward declaration.
///
/// This is the lean half of "an interpreted script calling a compiled library":
/// the script parses only this interface (type layouts + signatures + dispatch
/// symbols), **never the library bodies**, and gets the library's types defined
/// in the library's own order — so type ids align by construction rather than by
/// the caller redefining the types identically.  (A binary schema load — the D2a
/// cache — is the robust successor that also covers non-public ordering; this
/// source form covers the common case where the public types are the only ones.)
#[must_use]
pub fn generate_interface(data: &Data, export_set: &HashSet<u32>) -> String {
    use std::fmt::Write as _;
    // Referenced struct/enum defs, transitively, kept in definition order (BTreeSet
    // on `d_nr`) so the script registers them in the library's order.
    let mut types: BTreeSet<u32> = BTreeSet::new();
    for &d in export_set {
        let def = data.def(d);
        for a in def
            .attributes()
            .iter()
            .filter(|a| !a.hidden && !is_text_work_buffer(&a.typedef))
        {
            collect_type_defs(data, &a.typedef, &mut types);
        }
        collect_type_defs(data, def.returned(), &mut types);
    }

    let mut src = String::new();
    for &t in &types {
        emit_type_def(data, t, &mut src);
        src.push('\n');
    }
    let mut fns: Vec<u32> = export_set.iter().copied().collect();
    fns.sort_unstable();
    let decl_dups = crate::opt::names::duplicate_fn_names(data);
    for d in fns {
        let def = data.def(d);
        let name = def.name().strip_prefix("n_").unwrap_or(def.name());
        let params: Vec<String> = def
            .attributes()
            .iter()
            .filter(|a| !a.hidden && !is_text_work_buffer(&a.typedef))
            .map(|a| format!("{}: {}", a.name, a.typedef.name(data)))
            .collect();
        let ret = def.returned();
        let ret_clause = if matches!(ret, Type::Void | Type::Null) {
            String::new()
        } else {
            format!(" -> {} not null", ret.name(data))
        };
        let _ = writeln!(src, "pub fn {name}({}){ret_clause};", params.join(", "));
        let _ = writeln!(
            src,
            "#native \"loft_shared_{}\"",
            crate::opt::names::disambiguated_fn_ident(&decl_dups, def)
        );
    }
    src
}

/// The `--extern` arguments that name loft's runtime rlib to a generated crate's rustc.
///
/// TWO of them when the rlib carries no embedded metadata.  A rustc/cargo that builds an
/// rlib with `-Zembed-metadata=no` — the default on nightly since 2026-08 — leaves the
/// full metadata in a SIBLING `.rmeta` and puts only a stub in the `.rlib`, so compiling
/// against the rlib alone fails with *"only metadata stub found for `rlib` dependency
/// `loft`, please provide path to the corresponding .rmeta file with full metadata"* and
/// every `--native` build stops.  Passing both is what cargo itself does for such a
/// dependency; the sibling is only added when it EXISTS, so a toolchain that still embeds
/// metadata (stable today) passes exactly the one argument it always did.
///
/// The ONE home for that decision — seven sites build this argument, and a toolchain
/// change that reaches only some of them is a `--native` that works from one entry point
/// and not another.
#[must_use]
pub fn loft_extern_args(rlib: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    // Only when the rlib REALLY lacks its metadata.  Asking "does a `.rmeta` sit nearby?"
    // instead is wrong on any tree with history: a leftover `.rmeta` from an older build
    // is debris cargo does not own and does not clean, and naming it beside a
    // self-contained rlib gives rustc two candidates for one crate —
    // `error[E0464]: multiple candidates for `rlib` dependency `loft``, which names the
    // crate and never the staleness.  That failure is asymmetric in the worst way: CI is a
    // fresh checkout and stays green while every established working tree breaks.
    // (Reported by a peer session hitting it on `tuxedo-pln145`; reproduced here.)
    if rlib_metadata_is_stub(rlib)
        && let Some(rmeta) = loft_rmeta_beside(rlib)
    {
        out.push("--extern".to_string());
        out.push(format!("loft={}", rmeta.display()));
    }
    out.push("--extern".to_string());
    out.push(format!("loft={}", rlib.display()));
    out
}

/// The floor under which `lib.rmeta` inside an rlib cannot be real metadata.
///
/// A `-Zembed-metadata=no` stub is an ELF wrapper with an empty payload — 688 bytes for
/// loft's own rlib — while the genuine blob is the whole crate interface (11 MB for the
/// same rlib).  Four orders of magnitude apart, and this predicate is only ever asked of
/// `libloft.rlib`, so the floor does not have to hold for small crates in general.
const RMETA_STUB_FLOOR: u64 = 4096;

/// True when this rlib carries only a metadata STUB, so the full metadata lives in a
/// separate `.rmeta` that the compiler must be given as well.
///
/// Reads the archive rather than the file system: mtimes are the signal this codebase
/// already rejected for staleness ([`crate::cache::loft_build_fingerprint`] is
/// content-based precisely because a CI cache restore resets them), and a stale `.rmeta`
/// beside a fresh rlib is exactly the case that must NOT be mistaken for a split pair.
/// Unreadable or unparseable answers `false` — the pre-existing behaviour, one `--extern`.
fn rlib_metadata_is_stub(rlib: &std::path::Path) -> bool {
    let Ok(bytes) = crate::file_access::read(rlib) else {
        return false;
    };
    metadata_member_size(&bytes).is_some_and(|n| n < RMETA_STUB_FLOOR)
}

/// The size of an `ar` archive's `lib.rmeta` member, or `None` when there is no such
/// member (or the file is not an archive this can read).
///
/// The `ar` format is a magic line then a chain of 60-byte headers, each naming a member
/// and its size in ASCII; members are padded to an even offset.  Only the name and size
/// fields are read, and `lib.rmeta-link` — a different member that also starts with
/// `lib.rmeta` — is excluded by matching the name exactly.
fn metadata_member_size(bytes: &[u8]) -> Option<u64> {
    const MAGIC: &[u8] = b"!<arch>\n";
    const HEADER: usize = 60;
    if !bytes.starts_with(MAGIC) {
        return None;
    }
    let mut at = MAGIC.len();
    while at + HEADER <= bytes.len() {
        let header = &bytes[at..at + HEADER];
        let name = std::str::from_utf8(&header[0..16]).ok()?.trim_end();
        // GNU `ar` terminates a short name with `/`.
        let name = name.strip_suffix('/').unwrap_or(name);
        let size: u64 = std::str::from_utf8(&header[48..58])
            .ok()?
            .trim_end()
            .parse()
            .ok()?;
        if name == "lib.rmeta" {
            return Some(size);
        }
        at += HEADER + usize::try_from(size).ok()?;
        // Members start on an even offset.
        at += at % 2;
    }
    None
}

/// The `.rmeta` holding the full metadata for THIS `libloft.rlib`, when the toolchain
/// split them.  `None` when nothing can be shown to belong to it.
///
/// Identity, not proximity.  A stale `.rmeta` from an older build is debris cargo neither
/// owns nor cleans, and picking "the newest one nearby" can choose it — which hands rustc
/// two candidates for one crate (`error[E0464]`) or, worse, metadata that does not
/// describe the rlib being linked.  The pairing is exact instead: cargo writes the pair
/// `libloft-<hash>.rlib` + `libloft-<hash>.rmeta` in one unit and then UPLIFTS the rlib —
/// as a hard link — so the rmeta we want is the one beside the file our rlib IS.
///
/// A sibling `libloft.rmeta` next to the rlib is taken first (the layout where nothing was
/// uplifted); otherwise the search is for the file that IS ours, and only the `.rmeta`
/// beside that.  Nothing unpaired is accepted — passing an unpaired one is how this went
/// wrong.
fn loft_rmeta_beside(rlib: &std::path::Path) -> Option<std::path::PathBuf> {
    let sibling = crate::file_access::with_extension(rlib, "rmeta");
    if crate::file_access::exists(&sibling) {
        return Some(sibling);
    }
    let dir = rlib.parent()?;
    let mut search: Vec<std::path::PathBuf> = vec![dir.join("deps")];
    if let Ok(units) = crate::file_access::read_dir(dir.join("build").join("loft")) {
        search.extend(units.iter().map(|u| u.os_spelling().join("out")));
    }
    for candidate_dir in search {
        let Ok(entries) = crate::file_access::read_dir(&candidate_dir) else {
            continue;
        };
        for e in entries {
            let path = e.os_spelling();
            if !crate::file_access::has_extension(&path, "rlib") {
                continue;
            }
            let is_loft = crate::file_access::file_stem(&path)
                .is_some_and(|st| st == "libloft" || st.starts_with("libloft-"));
            if !is_loft || !same_file(&path, rlib) {
                continue;
            }
            let paired = crate::file_access::with_extension(&path, "rmeta");
            if crate::file_access::exists(&paired) {
                return Some(paired);
            }
        }
    }
    rmeta_by_metadata_hash(rlib)
}

/// The `.rmeta` whose metadata header carries the same hash as this stub's — the layout
/// where cargo writes the rlib UNHASHED (a `cdylib` + `rlib` crate) beside a HASHED
/// `libloft-<hash>.rmeta`, so neither the sibling name nor an uplift links the two
/// (loft#2033).  The hash is the crate's own, written into the stub and the full metadata by
/// the same compile, so this is identity too: a leftover `.rmeta` from another build carries
/// another hash and is never chosen.
fn rmeta_by_metadata_hash(rlib: &std::path::Path) -> Option<std::path::PathBuf> {
    let want = rlib_meta_header(rlib)?.hash?;
    let dir = rlib.parent()?;
    crate::file_access::read_dir(dir)
        .ok()?
        .into_iter()
        .map(|e| e.os_spelling())
        .filter(|p| {
            crate::file_access::has_extension(p, "rmeta")
                && crate::file_access::file_stem(p)
                    .is_some_and(|st| st == "libloft" || st.starts_with("libloft-"))
        })
        .find(|p| rmeta_meta_header(p).is_some_and(|h| h.hash == Some(want)))
}

/// What a crate-metadata header says about the compile that wrote it: the crate's metadata
/// hash (format 11 on; `None` before it) and the rustc version string.  Read from the `rust`
/// blob rustc writes at the head of every `lib.rmeta` and `.rmeta`: the magic and a format
/// version, the offsets, from format 11 the 16-byte hash, then the version as a
/// length-prefixed string.
#[derive(PartialEq, Eq, Debug)]
struct MetaHeader {
    hash: Option<[u8; 16]>,
    rustc: String,
}

fn meta_header_of_blob(blob: &[u8]) -> Option<MetaHeader> {
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
fn rmeta_meta_header(rmeta: &std::path::Path) -> Option<MetaHeader> {
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
fn rlib_meta_header(rlib: &std::path::Path) -> Option<MetaHeader> {
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
fn rlib_from_other_rustc(rlib: &std::path::Path) -> bool {
    match (rlib_meta_header(rlib), rustc_version()) {
        (Some(h), Some(current)) => h.rustc != current,
        _ => false,
    }
}

/// Which of loft's runtime rlibs a build links: the `deps/` copy, then the uplifted one —
/// UNLESS the `deps/` copy was written by another rustc and the uplifted one by this one.
///
/// `deps/` is preferred because a plain `cargo test` rewrites it and leaves the uplift stale
/// (#304).  But a cargo whose layout no longer writes `deps/` at all (`build/<pkg>/<unit>/out/`)
/// leaves a `deps/libloft.rlib` that only a restored cache put there, and preferring it would
/// link an older toolchain's runtime: a rebuild writes the fresh rlib beside it and the next
/// compile would take the stale one again (loft#2033).  The rustc that wrote
/// each candidate is read from its metadata, so the choice moves only on that evidence.  Asked by
/// `native_utils::loft_lib_dir_for`, the lookup every `--native` compile links through.
pub fn preferred_runtime_rlib(
    deps: &std::path::Path,
    uplifted: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let has_deps = crate::file_access::exists(deps);
    let has_uplifted = crate::file_access::exists(uplifted);
    if has_deps
        && !(has_uplifted && rlib_from_other_rustc(deps) && !rlib_from_other_rustc(uplifted))
    {
        return Some(deps.to_path_buf());
    }
    has_uplifted.then(|| uplifted.to_path_buf())
}

/// Make `<profile>/libloft.rlib` the runtime the live rustc built, as a cargo uplift does.
///
/// Every lookup links the rlib at `<profile>/libloft.rlib` (and its `.rmeta` beside it), and
/// derives the search path from that directory.  A cargo on the per-unit layout writes the
/// rlib only to `<profile>/build/loft/<unit>/out/` and does not uplift it, so that path is
/// either missing (a fresh tree) or a file an older toolchain left there (a restored cache),
/// and every compile against it fails with `E0514` (or, when the same rustc wrote it from older
/// source, with `E0425` on the runtime functions it lacks).  When the uplifted rlib is missing,
/// was written by another rustc, or is older than a per-unit one this rustc wrote, the newest
/// such unit is linked into place — the rlib and its `.rmeta` — through a temporary name and
/// a rename, so a concurrent reader sees the old file or the new one.  Otherwise nothing is
/// touched: a tree cargo still uplifts in is left exactly as cargo left it.
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
    let fresh = units
        .iter()
        .map(|u| u.os_spelling().join("out").join("libloft.rlib"))
        .filter(|r| crate::file_access::exists(r) && rlib_meta_header(r).is_some())
        .filter(|r| !rlib_from_other_rustc(r))
        .max_by_key(|r| modified(r));
    let Some(unit_rlib) = fresh else {
        return;
    };
    // The uplift stands when this rustc wrote it and no unit is newer: either it IS that unit
    // (cargo's hard link, or this function's) or it is the newer build.  An uplift this same
    // rustc wrote from OLDER source is as stale as one from another rustc — the generated
    // code then names runtime functions it lacks (`E0425`) instead of failing on the version.
    if crate::file_access::exists(&uplifted)
        && !rlib_from_other_rustc(&uplifted)
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

pub use crate::opt::native_key::{auto_cdylib_stem, library_export_set, platform_cdylib_name};
pub(crate) use crate::opt::native_key::{
    hidden_dest_type_name, library_candidates, type_layout_fingerprint,
};

/// `LOFT_TRACE_NATIVE_GATE=1` — how the library at `pkg_dir` splits: how many of the
/// functions a script can call run compiled, and for each that stays interpreted, the rule
/// that kept it there (`native_gate::explain_interpreted`).  A library's build reports only
/// failure, so without this a function left out ran slower in silence.
pub fn trace_partition(data: &Data, pkg_dir: &str) {
    let mut names: Vec<(String, Option<String>)> = library_candidates(data, pkg_dir)
        .into_iter()
        .map(|d| {
            (
                data.def(d).name().to_string(),
                crate::native_gate::explain_interpreted(data, d),
            )
        })
        .collect();
    names.sort();
    let compiled = names.iter().filter(|(_, why)| why.is_none()).count();
    let pkg = crate::file_access::file_name(pkg_dir).unwrap_or_else(|| pkg_dir.to_string());
    eprintln!(
        "[native-gate] {pkg}: {compiled} of {} callable functions compiled",
        names.len()
    );
    for (name, why) in names {
        if let Some(why) = why {
            eprintln!("[native-gate]   `{name}` interpreted — {why}");
        }
    }
}

/// Add to `types` (transitively, in definition order) the struct/enum defs that
/// loft type `t` references.
fn collect_type_defs(data: &Data, t: &Type, types: &mut BTreeSet<u32>) {
    // The struct/enum `def_nr` this type references, if any.  Reference / Enum /
    // Sorted / Index / Hash / Radix all carry a leading element-struct `def_nr`;
    // a Vector recurses into its element type; everything else is a leaf.
    let d = match t {
        Type::Reference(d, _)
        | Type::Enum(d, _, _)
        | Type::Sorted(d, _, _)
        | Type::Index(d, _, _)
        | Type::Hash(d, _, _)
        | Type::Radix(d, _, _)
        | Type::Trie(d, _, _) => *d,
        Type::Vector(elm, _) => {
            collect_type_defs(data, elm, types);
            return;
        }
        _ => return,
    };
    if !types.insert(d) {
        return; // already collected (also breaks cycles)
    }
    let def = data.def(d);
    for a in def.attributes() {
        collect_type_defs(data, &a.typedef, types);
    }
    // enum variants carry their own fields (DefType::EnumValue children)
    for v in data.children_of(d) {
        for a in data.def(v).attributes() {
            collect_type_defs(data, &a.typedef, types);
        }
    }
}

/// Emit the loft-source definition of struct/enum `d_nr` into `src`.
fn emit_type_def(data: &Data, d_nr: u32, src: &mut String) {
    use std::fmt::Write as _;
    let def = data.def(d_nr);
    match def.def_type() {
        DefType::Struct => {
            let _ = writeln!(src, "struct {} {{", def.name());
            for a in def.attributes() {
                let _ = writeln!(src, "    {}: {},", a.name, a.typedef.name(data));
            }
            src.push_str("}\n");
        }
        DefType::Enum => {
            let variants: Vec<u32> = data.children_of(d_nr).collect();
            let parts: Vec<String> = variants
                .iter()
                .map(|&v| {
                    let vd = data.def(v);
                    let vname = vd.name().rsplit('.').next().unwrap_or(vd.name());
                    // Each variant carries an auto-added `enum` discriminant field
                    // (definitions.rs) — skip it; emit only the user-declared fields.
                    let fields: Vec<String> = vd
                        .attributes()
                        .iter()
                        .filter(|a| a.name != "enum")
                        .map(|a| format!("{}: {}", a.name, a.typedef.name(data)))
                        .collect();
                    if fields.is_empty() {
                        vname.to_string()
                    } else {
                        format!("{vname} {{ {} }}", fields.join(", "))
                    }
                })
                .collect();
            let _ = writeln!(src, "enum {} {{ {} }}", def.name(), parts.join(", "));
        }
        _ => {}
    }
}

pub(crate) use crate::host_abi::LAYOUT_FP_SYMBOL;
pub use crate::host_abi::{HOST_ABI, HostTable, IdTable, LibArg};

/// The part of every library cdylib that receives the [`HostTable`]: `loft_shared_init`
/// fills the artifact's id tables from it ([`id_tables_src`]), stores it, and `__host()`
/// reads it.  Appended by [`generate_shared_cdylib_lib_rs`]; the compiled standard library,
/// which is built INTO the host, swaps it for [`IN_HOST_TABLE_SRC`].
pub(crate) const HOST_INIT_SRC: &str = "
static __HOST: std::sync::OnceLock<&'static loft::native_lib::HostTable> = std::sync::OnceLock::new();

#[unsafe(no_mangle)]
pub extern \"C\" fn loft_shared_init(table: *const loft::native_lib::HostTable, ids: *const std::ffi::c_void) -> u32 {
    let table: &'static loft::native_lib::HostTable = unsafe { &*table };
    if table.abi != loft::native_lib::HOST_ABI || !__fill_ids(table, ids) {
        return 0;
    }
    let _ = __HOST.set(table);
    loft::codegen_runtime::forward_coroutines(table.coroutines);
    1
}

#[allow(dead_code)]
fn __host() -> &'static loft::native_lib::HostTable {
    __HOST.get().expect(\"a library cdylib is called only after its host ran loft_shared_init\")
}

/// The host's coroutine table, under the runtime's names: what the generator calls in a
/// library cdylib, so a generator handle means one table on both sides.
#[allow(dead_code)]
mod __coro {
    use loft::database::Stores;
    use loft::keys::DbRef;
    pub fn alloc_coroutine_held(cell: &std::cell::UnsafeCell<Stores>, coro: Box<dyn loft::codegen_runtime::LoftCoroutine>) -> DbRef { (super::__host().coroutines.alloc_held)(cell, coro) }
    pub fn coroutine_retain(g: DbRef) -> DbRef { (super::__host().coroutines.retain)(g) }
    pub fn coroutine_next_i64(g: DbRef, s: &mut Stores) -> i64 { (super::__host().coroutines.next_i64)(g, s) }
    pub fn coroutine_next_into(g: DbRef, s: &mut Stores, d: &mut [i64]) -> bool { (super::__host().coroutines.next_into)(g, s, d) }
    pub fn coroutine_next_dbref(g: DbRef, s: &mut Stores) -> DbRef { (super::__host().coroutines.next_dbref)(g, s) }
    pub fn coroutine_next_text(g: DbRef, s: &mut Stores) -> String { (super::__host().coroutines.next_text)(g, s) }
    pub fn coroutine_is_exhausted(g: DbRef) -> bool { (super::__host().coroutines.exhausted)(g) }
}
";

/// [`HOST_INIT_SRC`] for code linked into the host itself: its table is the host's own.
pub(crate) const IN_HOST_TABLE_SRC: &str = "
#[allow(dead_code)]
fn __host() -> &'static loft::native_lib::HostTable {
    &loft::native_adopt::HOST_TABLE
}

#[allow(dead_code, unused_imports)]
mod __coro {
    pub use crate::codegen_runtime::{
        alloc_coroutine_held, coroutine_is_exhausted, coroutine_next_dbref, coroutine_next_i64,
        coroutine_next_into, coroutine_next_text, coroutine_retain,
    };
}
";

/// Build the shared `--native` program (header + `init` + only the functions
/// reachable from `entry` + their deps) for `data`.  Both cdylib generators
/// append their export wrappers to this.
///
/// With `own_ids`, the code reads the ids of the types and functions it names from tables the
/// host fills at load (`Output::shared_ids`), and those ids come back beside the source.
fn emit_program(
    data: &Data,
    stores: &Stores,
    entry: &[u32],
    own_ids: bool,
) -> (String, Option<crate::generation::SharedIds>) {
    let _census = crate::rewrite_census::Pause::new();
    let mut buf: Vec<u8> = Vec::new();
    let ids = {
        let mut out = Output::new(data, stores);
        if own_ids {
            out.shared_ids = Some(std::cell::RefCell::default());
        }
        // @PLN26 phase 2 — a cdylib that calls a `[native] crate` package emits
        // that package's fns as C-ABI `extern "C"` decls naming `loft_ffi::Loft*`,
        // its `.so` linked (sealing the package's whole Rust crate graph), exactly
        // as the executable native path does.  The SAME `native_cabi_enabled()`
        // gate drives the matching link flags in `build_shared_cdylib`, so codegen
        // and link never disagree.  No-op when the library uses no native package.
        out.native_cabi = native_cabi_link_enabled();
        out.library_unit = true;
        if own_ids {
            // @PLN199 G1 — only the packages this artifact links can collide in its link.
            let set: HashSet<u32> = entry.iter().copied().collect();
            out.native_collisions = linked_collisions(data, &linked_packages(data, &set));
        }
        // @PLN157 — a library cdylib is a SHIPPED binary: the lean tier, exactly as a
        // `--native-release` program.  Its functions run under the consumer's every
        // call, and the named per-call frame push was that lane's largest cost.
        out.emit_live = false;
        out.lean_tier = true;
        // The frame-naming question has its own home (`Output::lean`, kept apart from the
        // tier so a browser client can be debug-off and still name its frames): a shipped
        // library names none.
        out.lean = true;
        // Only the exported functions + their transitive deps (header + init + the
        // reachable subset) — exactly what a `--native` binary emits from `n_main`,
        // so unreachable operator stubs never surface.
        let _ = out.output_native_library(&mut buf, 0, data.definitions(), entry);
        out.shared_ids.take().map(std::cell::RefCell::into_inner)
    };
    (String::from_utf8(buf).unwrap_or_default(), ids)
}

/// The `#native` symbols two of `linked` packages declare: what [`linked_packages`]' link
/// cannot tell apart.
fn linked_collisions(data: &Data, linked: &[(String, String)]) -> HashSet<String> {
    let mut by_sym: HashMap<&str, HashSet<usize>> = HashMap::new();
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.native().is_empty() || *def.code() != crate::data::Value::Null {
            continue;
        }
        if let Some(at) = linked
            .iter()
            .position(|(_, dir)| crate::file_access::is_under(def.position().file.as_str(), dir))
        {
            by_sym.entry(def.native()).or_default().insert(at);
        }
    }
    by_sym
        .into_iter()
        .filter(|(_, pkgs)| pkgs.len() > 1)
        .map(|(s, _)| s.to_string())
        .collect()
}

/// @PLN199 G0b-2 — the code a library artifact carries for `entry` and what it reaches, with
/// its id tables: what `tests/lib_ids.rs` generates in two programs that number the same
/// functions differently and requires to be the same text (outside `init`, the program's own
/// type table).
#[doc(hidden)]
#[must_use]
pub fn library_code_rs(data: &Data, stores: &Stores, entry: &[u32]) -> String {
    let (mut src, ids) = emit_program(data, stores, entry, true);
    src.push_str(&id_tables_src(data, stores, &ids.unwrap_or_default()));
    src
}

/// @PLN199 G0b-2 — the id tables of a library artifact: the key of each type and function its
/// code reads by position (`lib_ids`), the slots `loft_shared_init` fills with the loading
/// program's numbers, and the readers.  The slots are written once, at load, before any bridge
/// runs, and only read after, so a read is a plain load (`loft::native_lib::IdTable`), which
/// the optimiser may merge with another read of the same slot — what lets a fn-value dispatch
/// that made its value from `__fid(k)` fold its `__fid(k)` arm as it folds a literal one.  A
/// key the program lacks refuses the artifact (`__fill_ids` answers false).
fn id_tables_src(data: &Data, stores: &Stores, ids: &crate::generation::SharedIds) -> String {
    use std::fmt::Write as _;
    let names = crate::lib_ids::TypeNames::new(data, stores);
    let type_keys: Vec<String> = ids.types.iter().map(|&t| names.key(t)).collect();
    let fn_keys: Vec<String> = ids
        .fns
        .iter()
        .map(|&d| crate::lib_ids::fn_key(data, d))
        .collect();
    let list = |keys: &[String]| {
        let mut s = String::new();
        for k in keys {
            let _ = writeln!(s, "    {k:?},");
        }
        s
    };
    let (nt, nf) = (type_keys.len(), fn_keys.len());
    format!(
        "
static __TID: loft::native_lib::IdTable<u16, {nt}> = loft::native_lib::IdTable::new(0);
const __TID_KEYS: [&str; {nt}] = [
{}];
static __FID: loft::native_lib::IdTable<u32, {nf}> = loft::native_lib::IdTable::new(0);
const __FID_KEYS: [&str; {nf}] = [
{}];

#[allow(dead_code)]
#[inline(always)]
fn __tid(k: usize) -> u16 {{
    __TID.get(k)
}}

#[allow(dead_code)]
#[inline(always)]
fn __fid(k: usize) -> u32 {{
    __FID.get(k)
}}

/// The program's number of function value number `f`: a function of this artifact's is
/// numbered by its table position here (`0x8000_0000 | k`).
#[allow(dead_code)]
#[inline(always)]
fn __to_host(f: u32) -> u32 {{
    if f & 0x8000_0000 != 0 && f != u32::MAX {{ __FID.get((f & 0x7fff_ffff) as usize) }} else {{ f }}
}}

/// This artifact's number of function value number `f`, as the program numbered it: its own
/// for a function it carries, the program's for any other.
#[allow(dead_code)]
#[inline(always)]
fn __norm(f: u32) -> u32 {{
    if f & 0x8000_0000 != 0 {{
        return f;
    }}
    let local = __FLOCAL.get(0);
    let at = f as usize;
    if at < local.len() && local[at] != 0 {{ local[at] }} else {{ f }}
}}

/// The program's function numbers to this artifact's: `0x8000_0000 | k` at the program's
/// number of function `k`, 0 elsewhere.
static __FLOCAL: loft::native_lib::IdTable<&'static [u32], 1> = loft::native_lib::IdTable::new(&[]);

fn __fill_ids(table: &loft::native_lib::HostTable, ids: *const std::ffi::c_void) -> bool {{
    for (k, key) in __TID_KEYS.iter().enumerate() {{
        let id = unsafe {{ (table.type_id)(ids, key.as_ptr(), key.len()) }};
        if id == u16::MAX {{
            return false;
        }}
        unsafe {{ __TID.set(k, id) }};
    }}
    for (k, key) in __FID_KEYS.iter().enumerate() {{
        let id = unsafe {{ (table.fn_id)(ids, key.as_ptr(), key.len()) }};
        if id == u32::MAX {{
            return false;
        }}
        unsafe {{ __FID.set(k, id) }};
    }}
    let mut local = vec![0_u32; (0..{nf}).map(|k| __FID.get(k) as usize + 1).max().unwrap_or(0)];
    for k in 0..{nf} {{
        local[__FID.get(k) as usize] = 0x8000_0000 | k as u32;
    }}
    unsafe {{ __FLOCAL.set(0, Box::leak(local.into_boxed_slice())) }};
    true
}}
",
        list(&type_keys),
        list(&fn_keys)
    )
}

// SAFETY: written only by `loft_shared_init`, which the host runs once per artifact before
// handing out any of the artifact's functions, on any thread.
unsafe impl<T: Copy, const N: usize> Sync for IdTable<T, N> {}

/// Generate the cdylib `lib.rs` source for `data`'s **scalar** export set: the
/// `--native` program for the functions in `export_set` **and their transitive
/// dependencies**, plus a `#[no_mangle] pub extern "C"` export wrapper for each
/// `export_set` function (the symbols the interpreter will dispatch to).
///
/// `export_set` is the **library's own public, scalar-dispatchable functions** —
/// the cdylib's exported API, not the whole stdlib.  It serves as both the
/// reachability roots (so only those functions + their deps are emitted) and the
/// set to wrap.  The caller computes it as `library_pub_fns ∩ scalar_dispatchable`
/// (operator definitions and stdlib internals are deps, inlined/emitted as needed,
/// never wrapped).
#[must_use]
pub fn generate_cdylib_lib_rs(data: &Data, stores: &Stores, export_set: &HashSet<u32>) -> String {
    let entry: Vec<u32> = export_set.iter().copied().collect();
    let (mut src, _) = emit_program(data, stores, &entry, false);
    let dups = crate::opt::names::duplicate_fn_names(data);
    for &d in export_set {
        src.push('\n');
        src.push_str(&export_wrapper(data, &dups, d));
    }
    src
}

/// Generate the cdylib `lib.rs` source for `data`'s **shared-store** export set:
/// the `--native` program for `export_set` + deps, plus a uniform shared-store
/// bridge ([`LibArg`] ABI) per exported function.  Use this when a non-scalar
/// value (`vector`/`reference`) crosses the boundary; `export_set` is computed as
/// `library_pub_fns ∩ shared_store_dispatchable`.
#[must_use]
pub fn generate_shared_cdylib_lib_rs(
    data: &Data,
    stores: &Stores,
    export_set: &HashSet<u32>,
) -> String {
    shared_cdylib_source(
        data,
        stores,
        export_set,
        false,
        type_layout_fingerprint(stores),
    )
}

/// [`generate_shared_cdylib_lib_rs`].  `in_host`: the code writes this program's ids as
/// literals and carries no id tables — the compiled standard library, built into the host
/// against the type prefix every program starts with.  `LOFT_NO_LIB_IDS=1` writes a library
/// artifact's ids as literals too, with empty tables: the artifact then serves only the
/// program it was generated in (the A/B and the falsifier of @PLN199 G0b-2).
///
/// `artifact_fp` is the key the artifact declares itself built for (`LAYOUT_FP_SYMBOL`).
pub(crate) fn shared_cdylib_source(
    data: &Data,
    stores: &Stores,
    export_set: &HashSet<u32>,
    in_host: bool,
    artifact_fp: u64,
) -> String {
    let own_ids = !in_host && std::env::var_os("LOFT_NO_LIB_IDS").is_none();
    // Sorted: the artifact's source is then the same bytes for the same set, which a
    // generated file under a drift guard needs (@PLN181), and a `HashSet`'s order is not.
    let mut entry: Vec<u32> = export_set.iter().copied().collect();
    entry.sort_unstable();
    let (mut src, mut ids) = emit_program(data, stores, &entry, own_ids);
    src.push_str(HOST_INIT_SRC);
    let dups = crate::opt::names::duplicate_fn_names(data);
    for &d in &entry {
        src.push('\n');
        src.push_str(&shared_bridge_wrapper(
            data,
            stores,
            &dups,
            d,
            !in_host,
            ids.as_mut(),
        ));
    }
    if !in_host {
        src.push_str(&id_tables_src(data, stores, &ids.unwrap_or_default()));
    }
    src.push('\n');
    src.push_str(&layout_fp_export(artifact_fp));
    src
}

/// The `#[no_mangle] pub extern "C"` export that makes an artifact **name its own
/// type layout** (loft#717).
///
/// The generated cdylib hardcodes type-table INDICES and field OFFSETS, so it is
/// valid only against the exact table it was generated from.  Until now the only
/// thing tying an artifact to that table was its FILENAME — a naming convention,
/// not a check.  Anything that puts the wrong file at the right name (a stale
/// artifact copied in, a package shipping a prebuilt one, a filename collision, a
/// fingerprint that stops covering some layout difference) is therefore not
/// caught at all: the bridges register, the indices resolve against someone
/// else's table, and reads land at the wrong offsets.  That is silent memory
/// corruption whose crash surfaces arbitrarily far from the cause — a SIGSEGV
/// with nothing to attribute it to.
///
/// Carrying the fingerprint INSIDE the artifact makes the tie inseparable: a
/// rename cannot change it and a copy takes it along, so the loader can verify
/// rather than trust.
fn layout_fp_export(fp: u64) -> String {
    format!(
        "#[unsafe(no_mangle)]\n\
         pub extern \"C\" fn {LAYOUT_FP_SYMBOL}() -> u64 {{ {fp}u64 }}\n"
    )
}

/// The `#[no_mangle] pub extern "C"` **scalar** export wrapper for
/// scalar-dispatchable function `d_nr` (inner `--native` fn = its name, e.g.
/// `n_double`).  The export symbol is `loft_<name>` (distinct from the inner).
fn export_wrapper(data: &Data, dups: &HashSet<String>, d_nr: u32) -> String {
    let def = data.def(d_nr);
    // The emitted program disambiguates colliding fn names (#305) — the
    // wrapper must call (and be named after) the SAME identifier.
    let inner = crate::opt::names::disambiguated_fn_ident(dups, def); // e.g. "n_double"
    let ret = rust_type(def.returned(), &Context::Result);
    // Positional params (`a0`, `a1`, …) so wrapper arg names never clash with
    // anything; types match the inner function's argument-context signature.
    let params: Vec<(String, String)> = def
        .attributes()
        .iter()
        .filter(|a| !a.name.starts_with("__"))
        .enumerate()
        .map(|(i, a)| (format!("a{i}"), rust_type(&a.typedef, &Context::Argument)))
        .collect();
    let sig: Vec<String> = params.iter().map(|(n, t)| format!("{n}: {t}")).collect();
    let mut fwd = String::new();
    for (n, _) in &params {
        fwd.push_str(", ");
        fwd.push_str(n);
    }
    format!(
        "#[unsafe(no_mangle)]\n\
         pub extern \"C\" fn loft_{inner}({}) -> {ret} {{\n    \
         let cell = std::cell::UnsafeCell::new(Stores::new());\n    \
         init(&cell);\n    \
         {inner}(&cell{fwd})\n\
         }}\n",
        sig.join(", "),
    )
}

/// The `#[no_mangle] pub extern "C"` **shared-store** bridge for
/// shared-store-dispatchable function `d_nr`.  The export symbol is
/// `loft_shared_<name>`.  It casts the caller's `*mut Stores` to
/// `&UnsafeCell<Stores>` (no per-call cell — the caller's store is live), then,
/// in the inner fn's attribute order:
/// - a **visible** parameter is read from the next [`LibArg`] slot by its type;
/// - a **hidden** destination parameter (`Attribute::hidden`, appended by
///   `ref_return` for a non-scalar return) is **allocated here** in the shared
///   store (`null_named` + `OpDatabase(<type_id>)`), exactly as a `--native`
///   caller would — so the caller-side dispatcher only ever passes the public args.
///
/// Finally it forwards to the inner `--native` fn and writes the return.
#[expect(clippy::too_many_lines, reason = "inherited")]
///
/// `own_ids`: the artifact numbers its own functions (`Output::fn_lit_as`), so a function
/// value it answers takes the program's number on the way out (`__to_host`).  With `ids`, a
/// destination the bridge allocates takes its type from the artifact's table (`__tid`).
fn shared_bridge_wrapper(
    data: &Data,
    stores: &Stores,
    dups: &HashSet<String>,
    d_nr: u32,
    own_ids: bool,
    mut ids: Option<&mut crate::generation::SharedIds>,
) -> String {
    // @PLN157 § V-aa (`@FR-R-ValueRecord`) — an admitted function returns its record's
    // fields in REGISTERS, so this wrapper stops forwarding the hidden dest and writes
    // the tuple into it instead.  The dest is the ABI's own return record: the caller's
    // when it supplied one, a fresh store when it did not, and `(*ret).dbref` names it
    // either way — so the C boundary keeps its record contract while the loft-to-loft
    // call inside the cdylib takes the value path.
    let value_rec = crate::generation::hoist::value_records(data, stores, true);
    let value_fields = value_rec
        .fn_fields(d_nr)
        .map(<[(i64, &'static str)]>::to_vec);
    // `@FR-R-ValueRecord`'s nullable clause — a `-> S?` tuple ends in its presence word: an
    // ABSENT one writes nothing and answers the null record, which leaves the fresh dest to
    // the orphan free below like any record the callee did not return.
    let value_presence = value_rec
        .fn_layout(d_nr)
        .and_then(crate::generation::hoist::ValueTuple::presence);
    use std::fmt::Write as _;
    let def = data.def(d_nr);
    // Same identifier the emitted program uses for this fn (#305).
    let inner = crate::opt::names::disambiguated_fn_ident(dups, def); // e.g. "n_vec_sum"

    let mut body = String::new();
    let mut fwd = String::new();
    // Statements after the call: a `&T` argument's local, back into the caller's slot.
    let mut write_backs = String::new();
    let mut slot = 0usize; // next public-arg LibArg slot
    // @PLN118 arc F — hidden dests the bridge allocated as a FALLBACK (caller
    // forwarded a null/empty ref).  If the inner fn ignores its retbuf and returns
    // a fresh store (a struct-literal return does), that fallback record is orphaned
    // — one leaked store per call.  Freed after the call when the return differs.
    let mut value_dest: Option<String> = None;
    let mut fresh_dests: Vec<String> = Vec::new();
    // `@FR-R-WorkBuffer` — hidden WORK buffers the bridge allocated: never a result, so a
    // fresh one is released after the call without asking what the callee returned.
    let mut work_bufs: Vec<String> = Vec::new();
    let ret_text = matches!(def.returned().base(), Type::Text(_));
    for (i, a) in def.attributes().iter().enumerate() {
        let var = format!("p{i}");
        // #303 — the ONE marshallability judgment (shared with the gate and the
        // wire-time signature builder).  The gate guarantees `Some` for every
        // attribute of a marked function; a `None` here means the judgment
        // drifted between marking and generation — fail loudly, never emit a
        // bridge whose ABI the dispatcher would disagree with.
        let kind = crate::native_gate::classify_bridge_attr(a, ret_text).unwrap_or_else(|| {
            panic!(
                "shared bridge for {}: attribute '{}' is not bridge-classifiable \
                 but the function was marked dispatchable (gate/generator divergence)",
                def.name(),
                a.name,
            )
        });
        match kind {
            crate::native_gate::BridgeAttrKind::HiddenDest => {
                // ref_return destination.  A body-bearing caller pre-allocates
                // one and the dispatcher forwards its slot (#311) — write the
                // result into THAT record (the caller's frame owns and frees
                // it; a bridge-local allocation orphaned it, one leaked store
                // per call).  Fallbacks allocate, mirroring a `--native` caller
                // (`null_named` + `OpDatabase(<type_id>)`): a no-body `#native`
                // decl caller forwards no slot (`{slot} >= n`), and a null
                // incoming ref means no usable record arrived.
                let tname = hidden_dest_type_name(data, stores, &a.typedef).unwrap_or_else(|| {
                    panic!(
                        "shared bridge for {}: hidden dest '{}' has no shared-store type name",
                        def.name(),
                        a.name
                    )
                });
                let _ = writeln!(
                    body,
                    "    let mut {var}: DbRef = if {slot} < n {{ a[{slot}].dbref }} else {{ DbRef {{ store_nr: 0, rec: 0, pos: 0 }} }};"
                );
                let _ = writeln!(body, "    let mut {var}_fresh = false;");
                // "No usable record arrived" is a different test per destination kind.  A
                // RECORD destination names a record or nothing: `rec == 0` is absence
                // whatever `pos` says (@FR-L-Null).  A fn-ref call hands the callee a store
                // with no record in it — `stores.null()`, `pos` 8 — and testing `pos == 0`
                // as well let that through: the result was written into record 0 and the
                // caller read null (loft#1663).  A VECTOR destination keeps the pair,
                // because a real store with `rec == 0` there is a valid empty vector.
                let absent = if matches!(a.typedef.base(), Type::Vector(_, _)) {
                    format!("{var}.rec == 0 && {var}.pos == 0")
                } else {
                    format!("{var}.rec == 0")
                };
                let _ = writeln!(body, "    if {absent} {{");
                // @PLN199 G0b-2 — by key in a library artifact: a registered NAME is the
                // first definition's to register it, which a program's own type can be.
                let tp = stores.name(&tname);
                match ids
                    .as_deref_mut()
                    .filter(|_| tp != u16::MAX)
                    .map(|t| t.type_slot(tp))
                {
                    Some(k) => {
                        let _ = writeln!(body, "        let _tid{slot} = __tid({k});");
                    }
                    None => {
                        let _ = writeln!(
                            body,
                            "        let _tid{slot} = unsafe {{ (&*cell.get()) }}.name({tname:?});"
                        );
                    }
                }
                let _ = writeln!(
                    body,
                    "        assert!(_tid{slot} != u16::MAX, \"shared bridge: type {tname} not registered in the caller store\");"
                );
                let _ = writeln!(
                    body,
                    "        {var} = unsafe {{ (&mut *cell.get()).null_named(\"__shared_dest\") }};"
                );
                let _ = writeln!(
                    body,
                    "        {var} = OpDatabase(cell, {var}, i32::from(_tid{slot}));"
                );
                let _ = writeln!(body, "        {var}_fresh = true;");
                let _ = writeln!(body, "    }}");
                if a.work_buffer {
                    work_bufs.push(var.clone());
                } else {
                    fresh_dests.push(var.clone());
                }
                slot += 1;
                if a.work_buffer || value_fields.is_none() {
                    let _ = write!(fwd, ", {var}");
                } else {
                    value_dest = Some(var.clone());
                }
            }
            crate::native_gate::BridgeAttrKind::WorkText => {
                // text_return work buffer (`&mut String`) — own a LOCAL String, pass
                // `&mut`.  The returned `Str` points into it; `bridge_write_ret` copies
                // the bytes into the caller-owned `bridge_text_dest` record (@PLN10
                // dest-passing) before this frame drops.
                let _ = writeln!(body, "    let mut {var}: String = String::new();");
                let _ = write!(fwd, ", &mut {var}");
            }
            // @PLN199 — `&T` to a record, vector, keyed collection or data enum: the slot the
            // caller's `DbRef` names holds that value's `DbRef`.  It is copied into a local,
            // `--native`'s `*mut DbRef` parameter writes through or rebinds the local, and the
            // local goes back to the slot after the call — re-resolved then, because a call
            // back into the interpreter may grow its stack, and the slot with it.
            crate::native_gate::BridgeAttrKind::Marshal if matches!(a.typedef.base(), Type::RefVar(inner) if crate::data::is_dbref(inner.base())) =>
            {
                let at = slot;
                slot += 1;
                let _ = writeln!(body, "    let {var}_at: DbRef = a[{at}].dbref;");
                let _ = writeln!(
                    body,
                    "    let mut {var}_v: DbRef = if {var}_at.rec == 0 {{ DbRef::NULL }} else {{ \
                     *unsafe {{ &*cell.get() }}.store(&{var}_at).addr::<DbRef>({var}_at.rec, {var}_at.pos) }};"
                );
                let _ = write!(fwd, ", &mut {var}_v");
                let _ = writeln!(
                    write_backs,
                    "    if {var}_at.rec != 0 {{ *unsafe {{ &mut *cell.get() }}.store_mut(&{var}_at).addr_mut::<DbRef>({var}_at.rec, {var}_at.pos) = {var}_v; }}"
                );
            }
            crate::native_gate::BridgeAttrKind::Marshal => {
                let ty = rust_type(&a.typedef, &Context::Argument);
                // @PLN199 A3 — a tuple reads one slot per leaf (`extensions::tuple_leaves`).
                let read = bridge_read_value(&a.typedef, &mut slot);
                let _ = writeln!(body, "    let {var}: {ty} = {read};");
                // `(R-ValueLocal)` — a TUPLE PARAMETER: the C boundary hands a record, the
                // inner fn takes its fields (`hoist::layout_reads`, the live arm's spelling)
                // and, for a nullable layout, whether the record is there.
                if let Some(layout) = value_rec
                    .param_type(d_nr, i)
                    .and_then(|tp| value_rec.types.get(&tp))
                {
                    let reads = crate::generation::hoist::layout_reads(
                        layout,
                        &var,
                        "unsafe { &*cell.get() }",
                    );
                    let _ = write!(fwd, ", {reads}");
                } else {
                    let _ = write!(fwd, ", {var}");
                }
            }
        }
    }

    let call = format!("{inner}(cell{fwd})");
    // @PLN199 C2 — a text return's destination is taken at ENTRY, before the inner fn runs:
    // the inner fn may call back into the interpreter, whose own bridge calls set and take
    // the same `Stores::bridge_text_dest`, and taken afterwards it would read theirs.
    if ret_text {
        let _ = writeln!(
            body,
            "    let __text_dest = unsafe {{ (&mut *cell.get()).bridge_text_dest.take() }};"
        );
    }
    let ret_stmt = if let (Some(fields), Some(dest)) = (&value_fields, &value_dest) {
        let mut w = format!("    let __vt = {call};\n    let __vd = {dest};\n");
        let present = value_presence.map_or_else(|| "true".to_string(), |at| format!("__vt.{at}"));
        let _ = writeln!(
            w,
            "    if {present} {{ unsafe {{ let __s = &mut *cell.get();"
        );
        for (i, (off, rt)) in fields.iter().enumerate() {
            let setter = match *rt {
                "f64" => format!("set_float(__vd.rec, __vd.pos + {off}u32, __vt.{i})"),
                "f32" => format!("set_single(__vd.rec, __vd.pos + {off}u32, __vt.{i})"),
                // `set_byte` takes the value as `i32`, not as the `u8` a boolean's
                // STORAGE form suggests (@PLN17) — `u8::from` here compiled to
                // "expected `i32`, found `u8`" and broke the whole cdylib, which is the
                // only reason a library with one boolean-field record could not build.
                "bool" => {
                    format!("set_byte(__vd.rec, __vd.pos + {off}u32, 0, i32::from(__vt.{i}))")
                }
                _ => format!("set_int(__vd.rec, __vd.pos + {off}u32, __vt.{i})"),
            };
            let _ = writeln!(w, "        __s.store_mut(&__vd).{setter};");
        }
        let _ = write!(
            w,
            "    }} }}\n    unsafe {{ (*ret).dbref = if {present} {{ __vd }} else {{ DbRef::NULL }}; }}"
        );
        w
    } else {
        bridge_write_ret(def.returned(), &call, returns_owned_string(def), own_ids)
    };

    // @PLN118 arc F — after writing the return, free every bridge-allocated
    // FALLBACK dest the callee did not return (`(*ret).dbref` differs).  A hidden
    // dest only exists for an aggregate (dbref) return, so `(*ret).dbref` holds the
    // result here.  When the inner fn wrote into and returned the dest, the
    // identity matches and it is kept (the caller adopts + frees it); when the inner
    // fn ignored the retbuf and allocated its own store, the fallback is orphaned —
    // free it so it does not leak (one store per call across the interp↔cdylib
    // boundary; native whole-program has no bridge and is unaffected).
    let mut free_orphans = String::new();
    for var in &work_bufs {
        let _ = writeln!(
            free_orphans,
            "    if {var}_fresh {{ unsafe {{ (&mut *cell.get()).free_named(&{var}, \"__shared_work_buffer\"); }} }}",
        );
    }
    for var in &fresh_dests {
        let _ = write!(
            free_orphans,
            "    if {var}_fresh && !loft::keys::bridge_orphan_free_disabled() {{\n    \
                 let __r = unsafe {{ (*ret).dbref }};\n    \
                 if !(__r.store_nr == {var}.store_nr && __r.rec == {var}.rec && __r.pos == {var}.pos) {{\n    \
                     unsafe {{ (&mut *cell.get()).free_named(&{var}, \"__shared_dest_orphan\"); }}\n    \
                 }}\n    \
             }}\n",
        );
    }

    format!(
        "#[unsafe(no_mangle)]\n\
         pub extern \"C\" fn loft_shared_{inner}(\n    \
         stores: *mut Stores,\n    \
         args: *const loft::native_lib::LibArg,\n    \
         n: usize,\n    \
         ret: *mut loft::native_lib::LibArg,\n\
         ) {{\n    \
         let cell = unsafe {{ &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) }};\n    \
         let a = unsafe {{ std::slice::from_raw_parts(args, n) }};\n    \
         let _ = ({slot}, a);\n\
         {body}    \
         {ret_stmt}\n\
         {write_backs}\
         {free_orphans}\
         }}\n",
    )
}

/// The leaves of the returned tuple `t`, each with its field path in the Rust tuple the inner
/// fn answers (`.1.0`), in element order.
fn returned_tuple_leaves<'t>(t: &'t Type, path: String, out: &mut Vec<(String, &'t Type)>) {
    if let Type::Tuple(elems) = t.base() {
        for (i, e) in elems.iter().enumerate() {
            returned_tuple_leaves(e, format!("{path}.{i}"), out);
        }
    } else {
        out.push((path, t));
    }
}

/// Rust expression reading an argument of loft type `t` starting at `LibArg` slot `*slot`,
/// advancing it past the slots read: one for a plain value, one per leaf for a tuple, which
/// is rebuilt as the Rust tuple the inner fn takes (`(i64, (f64, u8))`).
fn bridge_read_value(t: &Type, slot: &mut usize) -> String {
    if let Type::Tuple(elems) = t.base() {
        let parts: Vec<String> = elems.iter().map(|e| bridge_read_value(e, slot)).collect();
        // A one-element Rust tuple needs its trailing comma.
        return if parts.len() == 1 {
            format!("({},)", parts[0])
        } else {
            format!("({})", parts.join(", "))
        };
    }
    let read = bridge_read(t, &format!("a[{slot}]"));
    *slot += 1;
    read
}

/// Rust expression reading an argument of loft type `t` out of `LibArg` slot
/// `slot` (e.g. `"a[0]"`), at the inner fn's argument-context type.
fn bridge_read(t: &Type, slot: &str) -> String {
    // `Optional(τ)` rides τ's sentinel layout (@PLN25) — read as the base type.
    match t.base() {
        Type::Integer(s) if s.forced_size.is_none() => format!("{slot}.scalar"),
        Type::Integer(_) => format!("{slot}.scalar as {}", rust_type(t, &Context::Argument)),
        Type::Character => format!("{slot}.scalar as i32"),
        // A loft bool is a `u8` (0/1) in the inner fn's signature, so the read must
        // be a `u8`, not a bare Rust `bool`: `let p: u8 = a[0].scalar != 0` is a type
        // error (#433 — surfaced once the loft_ffi collision stopped masking it).
        // `!= 0` normalises any non-zero scalar to the canonical 1.
        Type::Boolean => format!("(({slot}.scalar != 0) as u8)"),
        Type::Float => format!("f64::from_bits({slot}.scalar as u64)"),
        Type::Single => format!("f32::from_bits({slot}.scalar as u32)"),
        // Text arg → `&str` borrowed from the slot's (store-backed) bytes.
        Type::Text(_) => format!(
            "unsafe {{ std::str::from_utf8_unchecked(std::slice::from_raw_parts({slot}.text_ptr, {slot}.text_len)) }}"
        ),
        // Plain (tag-only) enum → a `u8` tag riding in the scalar slot.
        Type::Enum(_, false, _) => format!("{slot}.scalar as u8"),
        Type::Vector(_, _)
        | Type::Reference(_, _)
        | Type::Enum(_, true, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _)
        | Type::Trie(_, _, _) => format!("{slot}.dbref"),
        // @PLN199 D3 — a generator handle in the host's table, or the interpreter's.
        Type::Iterator(_, _) => format!("{slot}.dbref"),
        // @PLN199 C1 — a function value: the `(d_nr, closure)` pair `--native` passes.
        Type::Function(..) => format!("({slot}.scalar as u32, {slot}.dbref)"),
        // @PLN199 A4 — `&T` to a scalar: a pointer into the slot the caller's `DbRef` names,
        // null for a null reference, as `--native`'s `*mut T` parameter takes it.
        Type::RefVar(inner) if crate::native_gate::is_ref_scalar(inner) => {
            let rt = match inner.base() {
                Type::Float => "f64",
                Type::Boolean => "u8",
                _ => "i64",
            };
            format!(
                "{{ let __d = {slot}.dbref; if __d.rec == 0 {{ std::ptr::null_mut() }} else {{ \
                 std::ptr::from_mut(unsafe {{ (&mut *cell.get()).store_mut(&__d).addr_mut::<{rt}>(__d.rec, __d.pos) }}) }} }}"
            )
        }
        // Not bridge-able — the gate excludes these, so this is unreachable for a
        // shared-store-dispatchable function; emit a clearly-wrong token so a gate
        // bug surfaces as a compile error rather than silent corruption.
        _ => "compile_error!(\"unsupported shared-store arg type\")".to_string(),
    }
}

/// Rust statement writing the inner-fn result `expr` (of loft return type `t`)
/// into the `ret` [`LibArg`] slot.  `inner_owned` is [`returns_owned_string`] for
/// the inner fn: text producers split into those returning an owned `String`
/// (nwb / FFI-direct / curated) and those returning a buffer-backed `Str`, and the
/// bridge must borrow `&str` from the right one (`String` has no `.str()`).
fn bridge_write_ret(t: &Type, expr: &str, inner_owned: bool, own_ids: bool) -> String {
    // `Optional(τ)` rides τ's sentinel layout (@PLN25) — write as the base type.
    match t.base() {
        Type::Void | Type::Null => format!("let _ = {expr};"),
        // @PLN199 — a tuple of scalars (`native_gate::is_returned_tuple_leaf`): the inner fn
        // answers a Rust tuple, and each leaf goes to its own `ret[i]`, in element order —
        // the slots the dispatcher pushes back onto the interpreter's stack one by one.
        Type::Tuple(_) => {
            use std::fmt::Write as _;
            let mut leaves = Vec::new();
            returned_tuple_leaves(t, String::new(), &mut leaves);
            let mut out = format!("let __t = ({expr});\n    unsafe {{\n");
            for (i, (path, leaf)) in leaves.iter().enumerate() {
                let value = if matches!(leaf.base(), Type::Float) {
                    format!("__t{path}.to_bits() as i64")
                } else {
                    format!("(__t{path}) as i64")
                };
                let _ = writeln!(out, "        (*ret.add({i})).scalar = {value};");
            }
            out.push_str("    }");
            out
        }
        // Plain (tag-only) enum returns a `u8` tag → widen into the scalar slot.
        Type::Integer(_) | Type::Character | Type::Boolean | Type::Enum(_, false, _) => {
            format!("unsafe {{ (*ret).scalar = ({expr}) as i64; }}")
        }
        Type::Float => format!("unsafe {{ (*ret).scalar = ({expr}).to_bits() as i64; }}"),
        Type::Single => format!("unsafe {{ (*ret).scalar = ({expr}).to_bits() as i64; }}"),
        Type::Vector(_, _)
        | Type::Reference(_, _)
        | Type::Enum(_, true, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _)
        | Type::Trie(_, _, _) => format!("unsafe {{ (*ret).dbref = ({expr}); }}"),
        // @PLN199 C1 — a function value, as the `(d_nr, closure)` pair.
        Type::Function(..) => format!(
            "let __f = ({expr});\n    unsafe {{ (*ret).scalar = i64::from({}); (*ret).dbref = __f.1; }}",
            if own_ids { "__to_host(__f.0)" } else { "__f.0" }
        ),
        // @PLN199 D1 — a generator: the inner fn answers its state machine, which the host
        // registers in ITS coroutine table (the cdylib's is a copy of its own), writing the
        // handle into `ret`.
        Type::Iterator(_, _) => format!(
            "let __b: Box<Box<dyn loft::codegen_runtime::LoftCoroutine>> = Box::new({expr});\n    \
             unsafe {{ (__host().coroutine_adopt)(stores, Box::into_raw(__b).cast(), ret) }};"
        ),
        // Text return: the inner fn returns a `Str` pointing into a local work
        // `String` (about to drop).  @PLN10 — destination-passing, not `scratch`:
        // the interpreter caller routes this call through `gen_cdylib_text_dest_call`
        // (`is_cdylib_text_call` ⇒ true for an auto-native text fn), which stashed a
        // per-call `bridge_text_dest` on the shared store.  Write the bytes into that
        // caller-owned record (line-lifetime, distinct per call — no re-entrancy,
        // no never-cleared global buffer) and signal dest-mode by leaving `ret` text
        // null so the dispatcher pushes nothing (mirrors the cdylib `bridge_text_result`).
        Type::Text(_) => {
            // Borrow `&str` from the inner result: an owned `String` (nwb /
            // FFI-direct) vs a buffer-backed `Str` (which has `.str()`).
            let borrow = if inner_owned {
                "let __t: &str = __r.as_str();"
            } else {
                "let __t = __r.str();"
            };
            format!(
                "let __r = ({expr});\n    \
                 {borrow}\n    \
                 let __st: &mut Stores = unsafe {{ &mut *cell.get() }};\n    \
                 if let Some(__d) = __text_dest {{\n    \
                     if !__t.is_empty() {{\n    \
                         __st.store_mut(&__d).addr_mut::<String>(__d.rec, __d.pos).push_str(__t);\n    \
                     }}\n    \
                 }}\n    \
                 unsafe {{ (*ret).text_ptr = std::ptr::null(); (*ret).text_len = 0; }}"
            )
        }
        _ => "compile_error!(\"unsupported shared-store return type\");".to_string(),
    }
}

pub(crate) use crate::toolchain::has_rlib_ext;
pub use crate::toolchain::{
    NO_RUSTC, find_loft_rlib, rustc_available, rustc_version, toolchain_failure_hint,
};

/// `--extern name=path` flags for the optional feature-dep rlibs (random/png/…) in
/// `deps` that the generated stdlib code may reference — every `libX-<hash>.rlib`
/// except `libloft*` (which is passed explicitly as `--extern loft=`).
fn extra_externs(deps: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = crate::file_access::read_dir(deps) else {
        return out;
    };
    for e in entries {
        let n = e.file_name().unwrap_or_default();
        if !n.starts_with("lib") || !has_rlib_ext(n) || n.starts_with("libloft") {
            continue;
        }
        if let Some(stem) = n
            .strip_prefix("lib")
            .and_then(|s| s.rsplit_once('-'))
            .map(|x| x.0)
        {
            out.push((stem.to_string(), e.os_spelling()));
        }
    }
    out
}

/// Pick the `loft_ffi` rlib in `deps` that `libloft` was built against.
///
/// loft's `deps/` can hold several `libloft_ffi-<hash>.rlib` with the same
/// StableCrateId but different SVH — e.g. one from `cargo build` (the loft binary)
/// and one from `cargo test` (the suite).  `libloft` links exactly ONE of them; if
/// the cdylib's `--extern loft_ffi=` names a DIFFERENT copy, the link carries two
/// `loft_ffi` and rustc aborts with "found crates (`loft_ffi` and `loft_ffi`) with
/// colliding StableCrateId values" — the zero-trust native blocker, and the same
/// failure behind p171/p310/imaging.
///
/// The filename hash (`-<hash>` in `libloft_ffi-<hash>.rlib`) is the crate's
/// `-Cextra-filename`, NOT its SVH, and rmeta records the dependency by SVH — so we
/// cannot string-match libloft's dep to a filename.  We resolve it by VERIFICATION
/// instead: order the candidates by mtime-closeness to `libloft` (a crate and the dep
/// it links usually share a build time — a good first guess), then confirm the guess
/// with a throwaway `rustc` probe.  A trivial crate that names both `loft` and
/// `loft_ffi` links cleanly ONLY when the pinned copy's SVH matches the one `libloft`
/// records; a mismatched copy makes rustc pull `libloft`'s real dep from `-L` too and
/// reproduces the "colliding StableCrateId" abort.
///
/// This is precise and rustc-release-proof: after a toolchain bump cargo rebuilds
/// `loft_ffi` (new SVH) and leaves the OLD rlib in `deps/` (cargo never GCs it) — the
/// exact recurring trigger — but the stale copy now fails the probe, so the fresh copy
/// wins with no manual `cargo clean` between rustc updates.  The probe runs ONLY when
/// two-or-more copies coexist; the common single-copy case takes the fast path.
pub fn loft_ffi_for_libloft(
    libloft: &std::path::Path,
    deps: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let anchor = crate::file_access::metadata(libloft)
        .and_then(|m| m.modified())
        .ok();
    let mut candidates: Vec<(std::path::PathBuf, std::time::Duration)> = Vec::new();
    for e in crate::file_access::read_dir(deps).ok()? {
        let n = e.file_name().unwrap_or_default();
        if !n.starts_with("libloft_ffi-") || !has_rlib_ext(n) {
            continue;
        }
        // Symmetric gap to `libloft`'s mtime (clock direction ignored); a missing
        // mtime on either side sorts the candidate last via `Duration::MAX`.
        let modified = crate::file_access::symlink_metadata(&e)
            .and_then(|m| m.modified())
            .ok();
        let gap = match (anchor, modified) {
            (Some(a), Some(m)) => m
                .duration_since(a)
                .or_else(|_| a.duration_since(m))
                .unwrap_or_default(),
            _ => std::time::Duration::MAX,
        };
        candidates.push((e.os_spelling(), gap));
    }
    // Closest-mtime first: the best guess, and the order the probe walks.
    candidates.sort_by_key(|(_, gap)| *gap);
    match candidates.len() {
        0 => None,
        // Single copy — unambiguous, no probe.
        1 => Some(candidates.remove(0).0),
        // Two-or-more copies — verify by SVH, walking closest-mtime first.
        _ => {
            for (cand, _) in &candidates {
                if loft_ffi_candidate_links(libloft, cand, deps) {
                    return Some(cand.clone());
                }
            }
            // No copy verified (no `rustc`, or a probe env fault that fails all
            // equally) — fall back to the mtime-closest so the pin is still emitted
            // rather than dropped (dropping it re-opens the collision).
            Some(candidates.remove(0).0)
        }
    }
}

/// True iff a throwaway crate naming both `loft` and `loft_ffi` compiles with
/// `--extern loft_ffi=cand` — i.e. `cand`'s SVH is the one `libloft` records for its
/// `loft_ffi` dependency.  A mismatched `cand` forces rustc to load a SECOND
/// `loft_ffi` (libloft's real dep, resolved via `-L dependency`), which aborts with
/// "colliding StableCrateId".  Metadata-only (`--emit=metadata`, no codegen), so it
/// is cheap; used only to disambiguate two-or-more coexisting copies.
fn loft_ffi_candidate_links(
    libloft: &std::path::Path,
    cand: &std::path::Path,
    deps: &std::path::Path,
) -> bool {
    let Some(stem) = crate::file_access::file_stem(cand) else {
        return false;
    };
    let dir = std::env::temp_dir().join(format!("loft_ffi_probe_{}_{stem}", std::process::id()));
    if crate::file_access::create_dir_all(&dir).is_err() {
        return false;
    }
    let src = dir.join("probe.rs");
    let ok = crate::file_access::write(&src, "extern crate loft;\nextern crate loft_ffi;\n")
        .is_ok()
        && Spawn::new(Program::search("rustc"))
            .arg("--edition=2024")
            .arg("--crate-type")
            .arg("rlib")
            .arg("--emit=metadata")
            .arg("-o")
            .arg(dir.join("probe.rmeta"))
            .args(crate::native_lib::loft_extern_args(libloft))
            .arg("--extern")
            .arg(format!("loft_ffi={}", cand.display()))
            .arg("-L")
            .arg(format!("dependency={}", deps.display()))
            .arg(&src)
            .run(b"")
            .is_ok_and(|o| o.status.success());
    let _ = crate::file_access::remove_dir_all(&dir);
    ok
}

/// On Windows MSVC, the build-script output dirs holding native import libraries
/// (e.g. `windows.0.48.5.lib` from `windows-sys`) must be passed to a hand-driven
/// `rustc` as `-L` paths — cargo adds them via `cargo:rustc-link-search` but we
/// don't, so the cdylib link fails `LNK1181: cannot open input file …`.  Mirrors
/// the `--native` test runner's `find_native_lib_dirs`.  Empty (a no-op) off Windows
/// (`platform::import_lib_search_dirs`).
fn native_lib_search_dirs(rlib: &std::path::Path) -> Vec<std::path::PathBuf> {
    crate::platform::import_lib_search_dirs(rlib)
}

/// @PLN26 — whether a cdylib links `[native] crate` packages by C-ABI (their
/// `.so`) rather than as Rust rlibs.  The library-crate twin of
/// `native_utils::native_cabi_enabled` (the binary-crate copy the executable path
/// reads): both read the ONE env gate, so the cdylib's codegen + link agree with
/// the executable's on a given host.  `LOFT_NATIVE_CABI=0` forces the legacy rlib
/// link (the escape hatch); the C-ABI path is the default on every host.
fn native_cabi_link_enabled() -> bool {
    crate::platform::native_cabi_link()
}

/// @PLN26 phase 2 — the rustc link flags for ONE native package's resolved cdylib
/// `.so`: `-L native=<dir>` + `-l dylib=<name>` + an RPATH (the build/prebuilt
/// dir and `$ORIGIN` for an installed binary shipping the `.so` beside it).  On
/// Windows a DLL links through its import library and there is no RPATH, so this
/// bridges `<stem>.dll.lib` → `<stem>.lib`; the host loads the package's DLL before the
/// artifact ([`load_linked_packages`]), so the artifact's import resolves to it.
///
/// Returns empty when the `.so` can't be resolved ([`crate::extensions::resolve_native_lib`]
/// already printed why) — the link then fails loudly on the undefined symbol
/// rather than silently mis-linking.  The `.so` seals the package's whole Rust
/// crate graph (its own `loft_ffi` + `loft_register_v1` included), so no
/// `-L dependency=` / per-crate pinning is needed and two packages no longer
/// collide on `loft_register_v1`.
///
/// Mirrors the C-ABI branch of `native_utils::add_native_extern_flags` (the
/// executable path); kept separate because that helper lives in the binary crate
/// and this builder lives in the library crate.
fn native_pkg_cabi_link_args(crate_name: &str, pkg_dir: &str) -> Vec<String> {
    let Some((stem, so_path)) = native_pkg_lib(crate_name, pkg_dir) else {
        return Vec::new();
    };
    let Some(so_dir) = so_path.parent() else {
        return Vec::new();
    };
    // `-l dylib=<name>` derived from the RESOLVED file (strip `lib` prefix +
    // extension) so a prebuilt or non-`lib<stem>` cdylib links.
    let file_stem = crate::file_access::file_stem(&so_path);
    let libname = file_stem
        .as_deref()
        .map_or(stem.as_str(), |s| s.strip_prefix("lib").unwrap_or(s));
    let mut args = vec![
        "-L".to_string(),
        format!("native={}", so_dir.display()),
        "-l".to_string(),
        format!("dylib={libname}"),
    ];
    match crate::platform::bridge_import_lib(so_dir, libname) {
        // Disallow-the-unverifiable-loudly: with NEITHER import-lib name present
        // the link dies on an opaque `LNK1181`, so name it rather than mis-link.
        crate::platform::ImportLib::Missing => eprintln!(
            "loft: native package `{crate_name}` cdylib at {} has no import \
             library (`{libname}.dll.lib` / `{libname}.lib`) — Windows links a \
             DLL through its import lib; rebuild the package's \
             cdylib with a toolchain that emits one.",
            so_dir.display()
        ),
        crate::platform::ImportLib::Ready => {}
        // A host that links the shared library directly: two RPATH entries, the
        // build/prebuilt dir (run-from-build-tree) AND `$ORIGIN` (an installed binary
        // shipping the `.so` beside it).  `$ORIGIN` is literal; the dynamic loader
        // expands it at run time.
        crate::platform::ImportLib::NotUsed => {
            args.push(format!("-Clink-arg=-Wl,-rpath,{}", so_dir.display()));
            args.push("-Clink-arg=-Wl,-rpath,$ORIGIN".to_string());
        }
    }
    args
}

/// @PLN11 Arc N / N3 — generate **and compile** the shared-store cdylib for
/// `export_set` into `out_dir`, returning the built cdylib path.  This is the
/// production build step `use <lib>` runs after `byte_code`: it locates the
/// running build's `libloft.rlib` ([`find_loft_rlib`]), writes `lib.rs`
/// ([`generate_shared_cdylib_lib_rs`]), and invokes `rustc` with the `--native`
/// flag set (cdylib, edition 2024, `--extern loft=` + feature-dep externs).
///
/// # Errors
/// Returns a message if the rlib can't be found, the source can't be written,
/// `rustc` can't be launched, or compilation fails (the message includes the
/// `rustc` stderr tail and the kept `lib.rs` path for inspection).
pub fn build_shared_cdylib(
    data: &Data,
    stores: &Stores,
    export_set: &HashSet<u32>,
    out_dir: &std::path::Path,
    stem: &str,
) -> Result<std::path::PathBuf, String> {
    build_keyed_cdylib(
        data,
        stores,
        export_set,
        out_dir,
        stem,
        type_layout_fingerprint(stores),
    )
}

/// [`build_shared_cdylib`], declaring itself built for `artifact_fp` (the key
/// [`artifact_matches_layout`] asks it for).
#[expect(clippy::too_many_lines, reason = "inherited")]
fn build_keyed_cdylib(
    data: &Data,
    stores: &Stores,
    export_set: &HashSet<u32>,
    out_dir: &std::path::Path,
    stem: &str,
    artifact_fp: u64,
) -> Result<std::path::PathBuf, String> {
    // Same pre-check as `auto_build_native` / the driver's native path: a
    // live rustc differing from loft's build rustc cannot link the
    // SVH-locked rlib below (E0514) — fail with the reason instead of a
    // compiler spew (callers fall back to interpreting the library).
    if let Some(reason) = crate::cache::rustc_mismatch() {
        return Err(format!("{reason} — rebuild loft to restore native"));
    }
    // Name the cure, not just the absence.  This fires when `libloft.rlib` has not
    // been built for the running binary's `target/` — the ordinary way to reach it
    // is a run of `cargo build --bin loft` (or `--tests`), which refreshes the
    // binary and leaves the library rlib behind.  Left bare, the message costs a
    // full gate: it surfaces as several unrelated-looking native tests failing ~9
    // minutes in, each naming a file that is present when you go and look.
    // The per-unit layout's rlib, put where the lookup reads it (`refresh_runtime_uplift`).
    if let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(std::path::Path::to_path_buf))
    {
        for (dir, _) in crate::toolchain::rlib_search_dirs(&exe_dir) {
            refresh_runtime_uplift(&dir);
        }
    }
    let (rlib, deps) = find_loft_rlib().ok_or(
        "libloft.rlib not found for this build — run `cargo build --release --lib` \
         (or `make check-rlib` to check before a gate); a `cargo build --bin loft` \
         refreshes the binary but not the library rlib the native path links",
    )?;
    let src = shared_cdylib_source(data, stores, export_set, false, artifact_fp);
    crate::file_access::create_dir_all(out_dir).map_err(|e| format!("create {e}"))?;
    let rs = out_dir.join(format!("{stem}.rs"));
    crate::file_access::write(&rs, &src).map_err(|e| format!("write {e}"))?;
    let so = out_dir.join(platform_cdylib_name(stem));
    // Compile to a temp name, rename into place on success: a concurrent
    // reader (fast path of `cached_or_build_shared_cdylib`) then always sees
    // either the old complete `.so` or the new complete one, never a torn
    // file mid-write ("file too short" on dlopen).
    let tmp_so = out_dir.join(format!("{stem}.building"));

    // Pass rustc args via an `@argfile`: the `--extern <crate>=<path>` list + `-L`
    // search paths routinely exceed Windows' ~32 KB `CreateProcessW` command-line
    // limit (os error 206: "The filename or extension is too long"), and the
    // argfile form is cross-platform — the same fix the `--native` test runner
    // already uses (PLAN49).
    let mut args: Vec<String> = vec![
        "--edition=2024".to_string(),
        "-C".to_string(),
        "debuginfo=0".to_string(),
        "--crate-type".to_string(),
        "cdylib".to_string(),
        "-o".to_string(),
        tmp_so.display().to_string(),
        rs.display().to_string(),
    ];
    // Full optimisation, as `--native-release` compiles a program: a library cdylib is a
    // shipped binary (`src/native_release_args.txt`).
    args.extend(crate::cache::release_rustc_args());
    args.extend(crate::native_lib::loft_extern_args(&rlib));
    args.extend(["-L".to_string(), deps.display().to_string()]);
    // macOS records the `-o` path inside the Mach-O as its INSTALL NAME, and the
    // rename below moves the file without touching it — so this cdylib claimed to
    // live at `<stem>.building`, in an absolute build directory. Harmless only for
    // as long as every consumer `dlopen`s it BY PATH (a path load ignores the
    // recorded name); it breaks the moment one is resolved through `@rpath` or the
    // package is moved. Same flag, same reason, as the `cc`-built shim.
    let final_name = crate::file_access::file_name(&so).unwrap_or_else(|| stem.to_string());
    for flag in crate::platform::install_name_args(&final_name, crate::platform::host_lib_os()) {
        args.push("-C".to_string());
        args.push(format!("link-arg={flag}"));
    }
    for (name, path) in extra_externs(&deps) {
        args.push("--extern".to_string());
        args.push(format!("{name}={}", path.display()));
    }
    // @PLN26 phase 2 — a shared-store library cdylib that USES a `[native] crate`
    // package links that package by C-ABI, exactly as the executable native path
    // does: its `.so` (sealing the package's whole Rust crate graph — its own
    // `loft_ffi` + `loft_register_v1` included) via `-L native`/`-l dylib`/RPATH,
    // and its fns as `extern "C"` decls naming `loft_ffi::Loft*` (codegen gated on
    // the SAME `native_cabi_link_enabled()` consulted by `emit_program`, so codegen
    // and link can't disagree).  The sealed `.so` is why two native packages no
    // longer collide on `loft_register_v1` — the old 2-package limit is lifted.
    if !data.native_packages.is_empty() {
        // The legacy rlib link (LOFT_NATIVE_CABI=0) CAN'T link a native package
        // into a cdylib: pulling the package rlib in brings a SECOND `loft_ffi`
        // rlib (duplicate `loft_register_v1`, two `StableCrateId`s) — unlinkable.
        // Refuse THAT combo loudly and fall back to interpret (codegen emitted
        // `extern crate <pkg>` for it, which has no matching `--extern` anyway).
        if !native_cabi_link_enabled() {
            let names: Vec<&str> = data
                .native_packages
                .iter()
                .map(|(c, _)| c.as_str())
                .collect();
            eprintln!(
                "loft: a shared-store cdylib using `[native] crate` package(s) {names:?} \
                 needs the C-ABI link path, but LOFT_NATIVE_CABI=0 forces the legacy rlib \
                 link (which cannot take two `loft_ffi` rlibs into one cdylib); interpreting \
                 it instead.  Unset LOFT_NATIVE_CABI to use the C-ABI path."
            );
            return Err(
                "shared-store cdylib + native package needs the C-ABI path (LOFT_NATIVE_CABI=0 set)"
                    .to_string(),
            );
        }
        // The C-ABI decls name `loft_ffi::LoftStore`/`LoftRef`/`LoftStr`; loft's
        // own `loft_ffi` rlib must be on the command (the package `.so` is C-ABI,
        // so this is the only `loft_ffi` the cdylib's Rust code names).  Mirrors
        // the standalone native compile in main.rs.
        // Pick the `loft_ffi` `libloft` was built against, NOT the first one in dir
        // order: with two copies present, naming the wrong one puts a second
        // `loft_ffi` in the link → "colliding StableCrateId" (see `loft_ffi_for_libloft`).
        if let Some(ffi) = loft_ffi_for_libloft(&rlib, &deps) {
            args.push("--extern".to_string());
            args.push(format!("loft_ffi={}", ffi.display()));
        }
        // Link each native package's resolved cdylib `.so` (the same `-L native`
        // / `-l dylib` / RPATH the executable path emits) so the cdylib's
        // `extern "C"` calls resolve at link time and the `.so` loads at run time.
        for (crate_name, pkg_dir) in &linked_packages(data, export_set) {
            args.extend(native_pkg_cabi_link_args(crate_name, pkg_dir));
        }
    }
    // Windows MSVC: add the build-script `-L` dirs holding native import libs
    // (`windows.0.48.5.lib` etc.) or the link fails LNK1181.  No-op off Windows.
    for dir in native_lib_search_dirs(&rlib) {
        args.push("-L".to_string());
        args.push(dir.display().to_string());
    }
    // @PLN54 S9 — mixed-boundary (@C71) AddressSanitizer.  When the interpreter
    // HOST is ASan-instrumented (LOFT_NATIVE_ASAN=1), instrument the auto-built
    // cdylib too, so an out-of-bounds / use-after-free on the `*mut Stores` the
    // host shares with this cdylib BY RAW POINTER is caught on BOTH sides of the
    // boundary — the one cross-boundary surface no in-process sanitizer sees
    // (ASan sees only the host's own accesses; Miri cannot `dlopen` a cdylib).
    // Needs nightly rustc (set on the Command below).  Opt-in, off by default.
    if std::env::var_os("LOFT_NATIVE_ASAN").is_some() {
        args.push("-Zsanitizer=address".to_string());
    }
    // One arg per line; quote any containing whitespace (rustc's argfile parser is
    // whitespace-separated, newline-separated is a strict subset).
    let argfile = out_dir.join(format!("{stem}.args"));
    let contents = args
        .iter()
        .map(|s| {
            if s.contains(char::is_whitespace) {
                format!("\"{}\"", s.replace('"', "\\\""))
            } else {
                s.clone()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    crate::file_access::write(&argfile, contents).map_err(|e| format!("write {e}"))?;
    let mut rustc = Spawn::new(Program::search("rustc")).arg(format!("@{}", argfile.display()));
    // @PLN54 S9 — the ASan cdylib (above) needs nightly rustc for `-Zsanitizer`.
    if std::env::var_os("LOFT_NATIVE_ASAN").is_some()
        && std::env::var_os("RUSTUP_TOOLCHAIN").is_none()
    {
        rustc.push_env("RUSTUP_TOOLCHAIN", "nightly");
    }
    crate::timeout::blocked_on("rustc compiling a library");
    let output = rustc.run(b"");
    crate::timeout::unblocked();
    let output =
        output.map_err(|e| format!("launch rustc: {e} (is the Rust toolchain installed?)"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let hint = toolchain_failure_hint(&stderr)
            .map(|h| format!("{h}\n\n"))
            .unwrap_or_default();
        let tail: Vec<&str> = stderr.lines().rev().take(30).collect();
        let tail: String = tail.into_iter().rev().collect::<Vec<_>>().join("\n");
        return Err(format!(
            "{hint}cdylib compile failed (source kept at {}):\n{tail}",
            rs.display()
        ));
    }
    crate::file_access::rename(&tmp_so, &so)
        .map_err(|e| format!("install as {}: {e}", so.display()))?;
    Ok(so)
}

/// How many built artifacts a package's `native-auto/` keeps.
///
/// The name carries the caller's type-layout fingerprint (#715), so a NEW file
/// appears whenever a consumer's type table differs — and nothing ever removed the
/// old ones.  Measured in this repo: `tests/lib/typeshift/native-auto` held 532
/// artifacts, 9.1 GB, at ~28 MB per suite run, because that fixture's whole job is
/// to shift type layouts.  Across the tree it was 25 GB.
///
/// Generous on purpose.  A pruned artifact that is wanted again is REBUILT, so the
/// cost of keeping too few is `rustc` latency and the cost of keeping too many is
/// disk; 8 covers a package used from several consumer contexts in one session
/// while still bounding the directory.
const KEEP_ARTIFACTS: usize = 8;

/// Drop all but the [`KEEP_ARTIFACTS`] most recent artifacts in `dir`, each with
/// the generated `.rs` / `.args` it was built from.
///
/// Called after a successful build, while the build lock is still held.  Ordering
/// is by mtime, so the survivors are the most recently BUILT — including the one
/// that just finished.
///
/// `family` is the artifact-name prefix this sweep OWNS
/// ([`auto_cdylib_stem`]) — and passing it is load-bearing, not tidiness.  The
/// sweep used to take every `.so` in the directory, and `native-auto/` is not
/// exclusively ours: a package with a `[c] shim` builds its shim cdylib
/// (`<pkg>_shim_<key>.so`) right there.  That library is content-keyed and built
/// ONCE, so it is always the OLDEST file in the directory — the first thing an
/// age-ordered sweep deletes the moment the directory saturates.  Deleting it
/// does not cost a rebuild, it removes the only definition of the package's `#c`
/// symbols, and the next run dies with *"`#c` symbol 'x' not found — check the
/// spelling"*, naming neither the library nor the sweep that ate it.  That is the
/// residual half of loft#831: parallel runs are what saturate the directory,
/// because each distinct type-layout context adds an artifact of its own.
///
/// A concurrent process can be adopting one of these without holding the lock (the
/// fresh fast path takes none).  That is why [`artifact_layout_probe`] refuses to
/// adopt an artifact it cannot OPEN: the loser of that race rebuilds instead of
/// dlopening a file that just vanished.
fn prune_artifacts(dir: &std::path::Path, family: &str) {
    let ext = crate::platform::dll_extension();
    let Ok(entries) = crate::file_access::read_dir(dir) else {
        return;
    };
    let mut built: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .into_iter()
        .filter(|e| crate::file_access::extension(e).is_some_and(|x| x == ext))
        .filter(|e| artifact_stem(e).is_some_and(|s| s.starts_with(family)))
        .filter_map(|e| {
            let m = crate::file_access::symlink_metadata(&e)
                .ok()?
                .modified()
                .ok()?;
            Some((m, e.os_spelling()))
        })
        .collect();
    if built.len() <= KEEP_ARTIFACTS {
        return;
    }
    // Newest first; everything past the keep window goes.
    built.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    for (_, so) in built.drain(KEEP_ARTIFACTS..) {
        let Some(stem) = artifact_stem(&so) else {
            continue;
        };
        let _ = crate::file_access::remove_file(&so);
        let _ = crate::file_access::remove_file(dir.join(format!("{stem}.rs")));
        let _ = crate::file_access::remove_file(dir.join(format!("{stem}.args")));
    }
}

/// `libfoo_<fp>.so` / `foo_<fp>.dll` → the `foo_<fp>` the generated sources are
/// named after.  Only the platform prefix differs, and stripping it is what lets
/// one loop delete a library and its generated companions together — and what
/// lets the sweep tell one family of artifacts from another.
fn artifact_stem(so: impl crate::file_access::HostPath) -> Option<String> {
    let name = crate::file_access::file_stem(so)?;
    Some(
        name.strip_prefix("lib")
            .map_or_else(|| name.clone(), str::to_string),
    )
}

/// The building half of `native_artifact::cached_or_build_shared_cdylib`: build the artifact `stem` for `fp`
/// into `out_dir`, record `fp` beside it, and prune the package's older artifacts.  Called
/// while the caller holds the package's build lock.
///
/// # Errors
/// Propagates a build failure from `build_keyed_cdylib`.
pub fn build_shared_artifact(
    data: &Data,
    stores: &Stores,
    export_set: &HashSet<u32>,
    pkg_dir: &str,
    out_dir: &std::path::Path,
    stem: &str,
    fp: u64,
) -> Result<std::path::PathBuf, String> {
    let built = build_keyed_cdylib(data, stores, export_set, out_dir, stem, fp)?;
    crate::cache::write_native_artifact_fingerprint(out_dir, fp);
    // Bound the directory, while the build lock is still held.  A new artifact
    // appears per consumer type-layout (#715) and nothing collected the old
    // ones — 25 GB across this repo's fixtures, 532 files in one package.
    // Scoped to THIS package's auto-native family: a `[c] shim` cdylib shares
    // the directory and is never rebuilt, so an unscoped sweep eats it.
    prune_artifacts(out_dir, &auto_cdylib_stem(pkg_dir));
    Ok(built)
}

#[cfg(test)]
mod bridge_read_tests {
    use super::bridge_read;
    use crate::data::Type;

    #[test]
    fn bool_arg_reads_as_u8_not_bare_bool() {
        // #433 — a loft bool param is a `u8` in the inner fn's signature, so the
        // LibArg read must yield a `u8`, not a bare Rust `bool`.  `let p: u8 =
        // a[0].scalar != 0` is an E0308; every other scalar arm casts `.scalar` to
        // the param's Rust type, and Boolean must too.  (Surfaced in the zero-trust
        // telemetry cdylib once the loft_ffi collision stopped masking it.)
        assert_eq!(
            bridge_read(&Type::Boolean, "a[0]"),
            "((a[0].scalar != 0) as u8)"
        );
    }
}

#[cfg(test)]
mod rmeta_pairing_tests {
    use super::loft_rmeta_beside;

    /// A cargo-shaped layout: the pair in `deps/`, the rlib UPLIFTED beside it as a hard
    /// link.  That link is what makes the pairing findable without guessing.
    fn layout(root: &std::path::Path, uplift_links_to_hash: bool) -> std::path::PathBuf {
        let deps = root.join("deps");
        crate::file_access::create_dir_all(&deps).unwrap();
        let hashed = deps.join("libloft-1111111111111111.rlib");
        crate::file_access::write(&hashed, b"rlib-bytes").unwrap();
        crate::file_access::write(
            deps.join("libloft-1111111111111111.rmeta"),
            b"the-paired-one",
        )
        .unwrap();
        let uplifted = root.join("libloft.rlib");
        if uplift_links_to_hash {
            crate::file_access::hard_link(&hashed, &uplifted).unwrap();
        } else {
            crate::file_access::write(&uplifted, b"a-different-rlib-entirely").unwrap();
        }
        uplifted
    }

    #[test]
    fn the_rmeta_paired_with_our_rlib_is_the_one_chosen() {
        let root = std::env::temp_dir().join(format!("loft_pair_ok_{}", std::process::id()));
        let _ = crate::file_access::remove_dir_all(&root);
        crate::file_access::create_dir_all(&root).unwrap();
        let rlib = layout(&root, true);
        let found = loft_rmeta_beside(&rlib).expect("the paired rmeta");
        assert_eq!(crate::file_access::read(&found).unwrap(), b"the-paired-one");
        let _ = crate::file_access::remove_dir_all(&root);
    }

    /// The reported failure: debris from an older build, NEWER than the pair, sitting in
    /// the same directory.  Choosing by recency picks it; choosing by identity does not.
    #[test]
    fn newer_unpaired_debris_is_not_chosen() {
        let root = std::env::temp_dir().join(format!("loft_pair_debris_{}", std::process::id()));
        let _ = crate::file_access::remove_dir_all(&root);
        crate::file_access::create_dir_all(&root).unwrap();
        let rlib = layout(&root, true);
        // Debris: an rmeta with no rlib of its own, written last so it is the newest file.
        crate::file_access::write(
            root.join("deps").join("libloft-9999999999999999.rmeta"),
            b"stale-debris",
        )
        .unwrap();
        let found = loft_rmeta_beside(&rlib).expect("still the paired rmeta");
        assert_eq!(
            crate::file_access::read(&found).unwrap(),
            b"the-paired-one",
            "recency must not decide this"
        );
        let _ = crate::file_access::remove_dir_all(&root);
    }

    /// When nothing can be shown to belong to the rlib, answer None: one `--extern`, and
    /// the honest "only metadata stub found" if the rlib really was split.  Passing an
    /// unpaired rmeta instead is what produced `E0464: multiple candidates`, an error that
    /// names the crate and never the staleness.
    #[test]
    fn an_unpaired_rmeta_alone_answers_none() {
        let root = std::env::temp_dir().join(format!("loft_pair_none_{}", std::process::id()));
        let _ = crate::file_access::remove_dir_all(&root);
        crate::file_access::create_dir_all(&root).unwrap();
        let rlib = layout(&root, false); // uplifted rlib is NOT our deps rlib
        assert!(
            loft_rmeta_beside(&rlib).is_none(),
            "an rmeta that belongs to a different rlib must not be offered"
        );
        let _ = crate::file_access::remove_dir_all(&root);
    }

    /// The layout where nothing was uplifted: the rmeta sits directly beside the rlib.
    #[test]
    fn a_sibling_rmeta_is_taken_directly() {
        let root = std::env::temp_dir().join(format!("loft_pair_sib_{}", std::process::id()));
        let _ = crate::file_access::remove_dir_all(&root);
        crate::file_access::create_dir_all(&root).unwrap();
        let rlib = root.join("libloft.rlib");
        crate::file_access::write(&rlib, b"rlib").unwrap();
        crate::file_access::write(root.join("libloft.rmeta"), b"sibling").unwrap();
        let found = loft_rmeta_beside(&rlib).expect("the sibling");
        assert_eq!(crate::file_access::read(&found).unwrap(), b"sibling");
        let _ = crate::file_access::remove_dir_all(&root);
    }
}

#[cfg(test)]
mod rmeta_identity_tests {
    use super::{MetaHeader, meta_header_of_blob, rlib_meta_header, rmeta_by_metadata_hash};

    /// A metadata blob in the shape rustc writes: magic, format version, two offsets, the
    /// 16-byte hash, then the rustc version as a length-prefixed string.
    fn blob(hash: u8, rustc: &str) -> Vec<u8> {
        let mut b = b"rust\0\0\0\x0b".to_vec();
        b.extend_from_slice(&[0u8; 16]);
        b.extend_from_slice(&[hash; 16]);
        b.push(u8::try_from(rustc.len()).unwrap());
        b.extend_from_slice(rustc.as_bytes());
        b.extend_from_slice(b"\xc1\x00\x04loft");
        b
    }

    /// An rlib whose first member is a large symbol table, as a stable rlib's is, then the
    /// `lib.rmeta` member wrapping `meta` behind an ELF-ish prefix.
    fn rlib(meta: &[u8]) -> Vec<u8> {
        let mut out = b"!<arch>\n".to_vec();
        let mut member = |name: &str, body: &[u8]| {
            let mut header = vec![b' '; 60];
            header[..name.len()].copy_from_slice(name.as_bytes());
            let s = body.len().to_string();
            header[48..48 + s.len()].copy_from_slice(s.as_bytes());
            header[58] = b'`';
            header[59] = b'\n';
            out.extend_from_slice(&header);
            out.extend_from_slice(body);
            if body.len() % 2 == 1 {
                out.push(b'\n');
            }
        };
        member("/", &vec![7u8; 9001]);
        let mut wrapped = b"\x7fELF-prefix".to_vec();
        wrapped.extend_from_slice(meta);
        member("lib.rmeta/", &wrapped);
        out
    }

    fn scratch(tag: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("loft_rmeta_id_{tag}_{}", std::process::id()));
        let _ = crate::file_access::remove_dir_all(&d);
        crate::file_access::create_dir_all(&d).unwrap();
        d
    }

    /// A per-unit layout profile: `<profile>/build/loft/<unit>/out/libloft.rlib` written by
    /// `rustc`, with an `.rmeta` beside it when `rmeta` is set.  Answers the unit's rlib.
    fn unit(profile: &std::path::Path, name: &str, rustc: &str, rmeta: bool) -> std::path::PathBuf {
        let out = profile.join("build").join("loft").join(name).join("out");
        crate::file_access::create_dir_all(&out).unwrap();
        let r = out.join("libloft.rlib");
        crate::file_access::write(&r, rlib(&blob(0x31, rustc))).unwrap();
        if rmeta {
            crate::file_access::write(out.join("libloft.rmeta"), blob(0x31, rustc)).unwrap();
        }
        r
    }

    fn rustc_of(p: &std::path::Path) -> String {
        rlib_meta_header(p).expect("an rlib header").rustc
    }

    const OTHER: &str = "rustc 0.0.1-nightly (deadbeef 2000-01-01)";

    /// A cargo on the per-unit layout writes the fresh rlib only under `build/loft/<unit>/out/`:
    /// a stale uplift a restored cache left is replaced by the unit this rustc wrote, with its
    /// `.rmeta`, and a missing uplift is created.
    #[test]
    fn a_stale_or_missing_uplift_takes_the_unit_this_rustc_wrote() {
        let Some(live) = crate::toolchain::rustc_version() else {
            return;
        };
        let p = scratch("uplift_stale");
        crate::file_access::write(p.join("libloft.rlib"), rlib(&blob(0x11, OTHER))).unwrap();
        crate::file_access::write(p.join("libloft.rmeta"), blob(0x11, OTHER)).unwrap();
        unit(&p, "aaaa", OTHER, true);
        let fresh = unit(&p, "bbbb", live, true);
        super::refresh_runtime_uplift(&p);
        assert_eq!(rustc_of(&p.join("libloft.rlib")), live);
        assert!(super::same_file(&p.join("libloft.rlib"), &fresh));
        assert!(super::same_file(
            &p.join("libloft.rmeta"),
            &crate::file_access::with_extension(&fresh, "rmeta")
        ));

        let q = scratch("uplift_missing");
        unit(&q, "cccc", live, false);
        super::refresh_runtime_uplift(&q);
        assert_eq!(rustc_of(&q.join("libloft.rlib")), live);
        assert!(!crate::file_access::exists(q.join("libloft.rmeta")));
        let _ = crate::file_access::remove_dir_all(&p);
        let _ = crate::file_access::remove_dir_all(&q);
    }

    /// Nothing is touched where cargo's own uplift is current, nor where no unit was written by
    /// this rustc; and a leftover `.rmeta` is dropped when the unit placed has none, so it is
    /// never a second candidate beside a self-contained rlib.
    #[test]
    fn an_uplift_is_left_alone_unless_a_fresh_unit_replaces_it() {
        let Some(live) = crate::toolchain::rustc_version() else {
            return;
        };
        // This rustc's uplift, NEWER than every unit: the newer build stands.
        let p = scratch("uplift_current");
        let older = unit(&p, "dddd", live, true);
        let current = rlib(&blob(0x22, live));
        crate::file_access::write(p.join("libloft.rlib"), &current).unwrap();
        let now = std::time::SystemTime::now();
        crate::file_access::set_modified(&older, now - std::time::Duration::from_secs(60)).unwrap();
        crate::file_access::set_modified(p.join("libloft.rlib"), now).unwrap();
        super::refresh_runtime_uplift(&p);
        assert_eq!(
            crate::file_access::read(p.join("libloft.rlib")).unwrap(),
            current
        );
        assert!(!crate::file_access::exists(p.join("libloft.rmeta")));

        // This rustc's uplift from OLDER source than a unit it also wrote (`E0425` in a run):
        // the newer unit replaces it.
        let o = scratch("uplift_same_rustc_older");
        let stale = rlib(&blob(0x33, live));
        crate::file_access::write(o.join("libloft.rlib"), &stale).unwrap();
        let newer = unit(&o, "gggg", live, true);
        crate::file_access::set_modified(
            o.join("libloft.rlib"),
            now - std::time::Duration::from_secs(60),
        )
        .unwrap();
        crate::file_access::set_modified(&newer, now).unwrap();
        super::refresh_runtime_uplift(&o);
        assert!(super::same_file(&o.join("libloft.rlib"), &newer));
        // …and placed, it is that unit: a second refresh changes nothing.
        super::refresh_runtime_uplift(&o);
        assert!(super::same_file(&o.join("libloft.rlib"), &newer));
        let _ = crate::file_access::remove_dir_all(&o);

        let q = scratch("uplift_none_fresh");
        crate::file_access::write(q.join("libloft.rlib"), rlib(&blob(0x11, OTHER))).unwrap();
        unit(&q, "eeee", OTHER, true);
        super::refresh_runtime_uplift(&q);
        assert_eq!(rustc_of(&q.join("libloft.rlib")), OTHER);

        let r = scratch("uplift_drop_rmeta");
        crate::file_access::write(r.join("libloft.rlib"), rlib(&blob(0x11, OTHER))).unwrap();
        crate::file_access::write(r.join("libloft.rmeta"), blob(0x11, OTHER)).unwrap();
        unit(&r, "ffff", live, false);
        super::refresh_runtime_uplift(&r);
        assert_eq!(rustc_of(&r.join("libloft.rlib")), live);
        assert!(!crate::file_access::exists(r.join("libloft.rmeta")));
        for d in [p, q, r] {
            let _ = crate::file_access::remove_dir_all(&d);
        }
    }

    #[test]
    fn the_header_names_the_hash_and_the_rustc() {
        let got = meta_header_of_blob(&blob(0x35, "rustc 1.101.0-nightly (4ebd25825 2026-10-15)"));
        assert_eq!(
            got,
            Some(MetaHeader {
                hash: Some([0x35; 16]),
                rustc: "rustc 1.101.0-nightly (4ebd25825 2026-10-15)".to_string(),
            })
        );
    }

    /// Format 10 (rustc 1.99) carries no hash and puts the version right after the root
    /// offset — the layout of the stable rlib measured beside the nightly's format 11.
    #[test]
    fn a_format_10_header_names_the_rustc_without_a_hash() {
        let mut b = b"rust\0\0\0\x0a".to_vec();
        b.extend_from_slice(&[0xf4, 0xf4, 0x5f, 0x01, 0, 0, 0, 0]);
        let v = "rustc 1.99.0 (b940084d7 2026-09-28)";
        b.push(u8::try_from(v.len()).unwrap());
        b.extend_from_slice(v.as_bytes());
        b.extend_from_slice(b"\xc1\x02");
        assert_eq!(
            meta_header_of_blob(&b),
            Some(MetaHeader {
                hash: None,
                rustc: v.to_string(),
            })
        );
    }

    #[test]
    fn the_member_is_found_past_a_symbol_table() {
        let d = scratch("walk");
        let p = d.join("libloft.rlib");
        crate::file_access::write(&p, rlib(&blob(0x11, "rustc 1.99.0 (b940084d7 2026-09-28)")))
            .unwrap();
        let h = rlib_meta_header(&p).expect("header read past the symbol table");
        assert_eq!(h.hash, Some([0x11; 16]));
        assert_eq!(h.rustc, "rustc 1.99.0 (b940084d7 2026-09-28)");
        let _ = crate::file_access::remove_dir_all(&d);
    }

    /// loft#2033 — an unhashed stub beside two hashed `.rmeta` files: the one carrying the
    /// stub's hash is chosen, the leftover from another build never is.
    #[test]
    fn a_hashed_rmeta_pairs_with_the_unhashed_stub_by_identity() {
        let d = scratch("pair");
        let stub = d.join("libloft.rlib");
        crate::file_access::write(
            &stub,
            rlib(&blob(0x42, "rustc 1.101.0-nightly (x 2026-10-15)")),
        )
        .unwrap();
        crate::file_access::write(
            d.join("libloft-aaaa.rmeta"),
            blob(0x24, "rustc 1.101.0-nightly (x 2026-10-15)"),
        )
        .unwrap();
        crate::file_access::write(
            d.join("libloft-bbbb.rmeta"),
            blob(0x42, "rustc 1.101.0-nightly (x 2026-10-15)"),
        )
        .unwrap();
        assert_eq!(
            rmeta_by_metadata_hash(&stub),
            Some(d.join("libloft-bbbb.rmeta"))
        );
        crate::file_access::remove_file(d.join("libloft-bbbb.rmeta")).unwrap();
        assert_eq!(
            rmeta_by_metadata_hash(&stub),
            None,
            "an unpaired rmeta is never taken"
        );
        let _ = crate::file_access::remove_dir_all(&d);
    }
}

#[cfg(test)]
mod rmeta_stub_tests {
    use super::{RMETA_STUB_FLOOR, metadata_member_size};

    /// Build a minimal `ar` archive holding the named members, so the parser is tested
    /// against the format rather than against whatever rlib happens to sit in `target/`.
    fn archive(members: &[(&str, usize)]) -> Vec<u8> {
        let mut out = b"!<arch>\n".to_vec();
        for (name, size) in members {
            let mut header = vec![b' '; 60];
            let named = format!("{name}/");
            header[..named.len()].copy_from_slice(named.as_bytes());
            let s = size.to_string();
            header[48..48 + s.len()].copy_from_slice(s.as_bytes());
            header[58] = b'`';
            header[59] = b'\n';
            out.extend_from_slice(&header);
            out.extend(std::iter::repeat_n(b'x', *size));
            if size % 2 == 1 {
                out.push(b'\n');
            }
        }
        out
    }

    #[test]
    fn a_full_metadata_member_is_not_a_stub() {
        let a = archive(&[("lib.rmeta", 11_220_440), ("lib.rmeta-link", 1256)]);
        let n = metadata_member_size(&a).expect("member found");
        assert!(n >= RMETA_STUB_FLOOR, "{n} should read as real metadata");
    }

    /// The shape a `-Zembed-metadata=no` rlib has: the member exists and is an empty ELF
    /// wrapper.  688 bytes is what loft's own rlib measured under the nightly that
    /// introduced it.
    #[test]
    fn an_empty_wrapper_reads_as_a_stub() {
        let a = archive(&[("lib.rmeta", 688), ("lib.rmeta-link", 1256)]);
        let n = metadata_member_size(&a).expect("member found");
        assert!(n < RMETA_STUB_FLOOR, "{n} should read as a stub");
    }

    /// `lib.rmeta-link` starts with the same nine characters and is a DIFFERENT member;
    /// matching on a prefix would read its size and call every rlib a stub.
    #[test]
    fn the_link_member_is_not_mistaken_for_the_metadata() {
        let a = archive(&[("lib.rmeta-link", 1256), ("lib.rmeta", 11_220_440)]);
        assert_eq!(metadata_member_size(&a), Some(11_220_440));
    }

    #[test]
    fn a_file_that_is_not_an_archive_answers_none() {
        assert_eq!(metadata_member_size(b"not an archive at all"), None);
        assert_eq!(metadata_member_size(&[]), None);
    }

    /// An odd-sized member is padded to an even offset; miss that and every member after
    /// it is read from the wrong place.
    #[test]
    fn an_odd_sized_member_does_not_desync_the_walk() {
        let a = archive(&[("first.o", 7), ("lib.rmeta", 4096)]);
        assert_eq!(metadata_member_size(&a), Some(4096));
    }
}
