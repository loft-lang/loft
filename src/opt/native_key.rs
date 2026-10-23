// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: the key a compiled library artifact is built and found under

//! The KEY of a compiled library artifact: which functions it carries, the file name it is
//! built under, and the fingerprints that decide whether an artifact on disk was built for
//! this program and this loft.  The Rust writer computes it to build the artifact and the
//! interpreter computes it to adopt one, so both ask here (`@C153`).

use crate::data::{Data, DefType, Type};
use crate::database::Stores;
use std::collections::HashSet;

/// @PLN11 Arc N / N3 (Step 2) — the cdylib **export set** for the library at
/// `pkg_dir`, computed **without marking** (`&Data`, not `&mut`): the library's
/// top-level, user-named, `pub` functions (the dispatch-target invariant — see
/// `native_adopt::mark_library_native`) intersected with the shared-store-dispatchable gate.
/// The build-before-mark flow builds the cdylib from this set, then calls
/// `native_adopt::mark_exports` only on success.
#[must_use]
pub fn library_export_set(data: &Data, pkg_dir: &str) -> HashSet<u32> {
    let candidates = library_candidates(data, pkg_dir);
    crate::native_gate::shared_store_dispatchable(data)
        .intersection(&candidates)
        .copied()
        .collect()
}

/// The functions of the library at `pkg_dir` a script can name and call — the candidates
/// [`library_export_set`] narrows to those that compile.
///
/// @PLN199 E1 — a function already bound to a native symbol is not one: a body-less
/// `#native` declaration walks clean through the gate (its body is `Null`), and exporting a
/// `loft_shared_` bridge for it would replace the binding with a bridge to a function that
/// has no loft body — the @PLN24 hazard, for `#native` instead of `#c`.  A symbol that IS
/// a `loft_shared_` bridge was set by an earlier marking of this same set.  A `#c`
/// binding is left out for the same reason: its C symbol is its implementation, so it is
/// neither compiled nor interpreted, and the trace does not list it as either.
pub(crate) fn library_candidates(data: &Data, pkg_dir: &str) -> HashSet<u32> {
    (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            matches!(def.def_type(), DefType::Function)
                && def.pub_visible
                && !is_synthetic_name(def.name())
                && (def.native().is_empty() || def.native().starts_with("loft_shared_"))
                && def.c_sig.is_empty()
                && crate::file_access::is_under(def.position().file.as_str(), pkg_dir)
        })
        .collect()
}

/// Is `stored_name` (an `n_<name>` definition name) a compiler-generated
/// (synthetic) function — i.e. its user-facing name starts with `__` (lambdas
/// `__lambda_<fn>_<k>`, and any future synthetic kind)?  Such functions are never a
/// script-callable public API, so they are not auto-native dispatch targets.
pub(crate) fn is_synthetic_name(stored_name: &str) -> bool {
    stored_name
        .strip_prefix("n_")
        .unwrap_or(stored_name)
        .starts_with("__")
}

/// The SHARED-STORE type NAME for a hidden destination attr of loft type `t` — resolved at
/// BRIDGE RUNTIME via `Stores::name` in the caller's store, because the library's
/// compile-time type IDS live in a different id space than the caller's (@PLAN59: a
/// lib-side constant id produced `claim(size=0)` / "Incomplete record" aborts once struct
/// dests became universal).  Vectors resolve via the `main_vector<elm>` schema name (the
/// same key `--native`'s `output_alloc_heap` uses).
///
/// A record or data-enum answers the name its row was REGISTERED under in `stores`, not its
/// definition's name.  The two differ when two definitions share a name: the first to
/// register keeps it and the second is qualified (`input::InputState` beside a program's own
/// `InputState`, `typedef::fill_database`).  Asked by the bare name, the caller's store
/// answered the OTHER type, and the bridge minted a record of that type's size for this
/// one's fields — a 6-boolean record for a 56-byte `InputState`, whose writes then ran into
/// the recycled store's leftover bytes (loft#1706).  The cdylib is keyed on the caller's
/// type-table layout (#461), so the row registered here is the row the caller has.
pub(crate) fn hidden_dest_type_name(data: &Data, stores: &Stores, t: &Type) -> Option<String> {
    match t.base() {
        Type::Vector(elm, _) => Some(format!("main_vector<{}>", elm.name(data))),
        Type::Reference(td, _) | Type::Enum(td, true, _) => {
            let def = data.def(*td);
            Some(
                stores
                    .types
                    .get(usize::from(def.known_type))
                    .map_or_else(|| def.name().to_string(), |row| row.name.clone()),
            )
        }
        _ => None,
    }
}

/// Platform cdylib filename for `stem` (`lib<stem>.so` / `.dylib` / `<stem>.dll`).
#[must_use]
pub fn platform_cdylib_name(stem: &str) -> String {
    crate::platform::cdylib_file_name(stem)
}

/// Derive the auto-native cdylib crate stem from a package directory.
///
/// The stem becomes the cdylib's crate name (rustc derives the crate name from the
/// source-file stem when no `--crate-name` is given), so every character must be
/// valid in a Rust identifier.  A registry dir carries a dotted version
/// (`glb-0.1.0`): rustc maps `-`→`_` itself but rejects the surviving `.`, so map
/// **every** non-`[A-Za-z0-9_]` char to `_` — enforcing the full invalid-character
/// class, not just `-`/`.`.  The constant `loft_auto_` prefix guarantees the leading
/// character is alphabetic (a crate name may not start with a digit).  (#294)
///
/// @PLN199 G2b — the package's NAME and VERSION from its `loft.toml` (`glb` 0.1.0 →
/// `loft_auto_glb_0_1_0`), the directory's name only without one: a registry install and the
/// checkout CI builds a prebuilt from lie in differently named directories, and both must
/// name the artifact alike.
#[must_use]
pub fn auto_cdylib_stem(pkg_dir: &str) -> String {
    let raw = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
        .and_then(|m| {
            let name = m.name?;
            Some(match m.version {
                Some(v) => format!("{name}_{v}"),
                None => name,
            })
        })
        .or_else(|| crate::file_access::file_name(pkg_dir))
        .unwrap_or_else(|| "lib".to_string());
    format!(
        "loft_auto_{}",
        raw.replace(|c: char| !(c.is_ascii_alphanumeric() || c == '_'), "_")
    )
}

/// #461 — combine two fingerprints into one stable, order-sensitive digest.
pub(crate) fn mix_fp(a: u64, b: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    a.hash(&mut h);
    b.hash(&mut h);
    h.finish()
}

/// A digest of the package directories compiled into an auto-native artifact, in sorted
/// order so the order they were resolved in does not name a second artifact.
/// @PLN199 G0c — the libraries among `pending` whose code the artifact for `pkg_dir` can
/// carry: `pkg_dir` itself, what it imports, transitively, and the home of every function
/// its exports reach (a qualified `use dep;` binds no import).  The program around it may
/// load others (`pending` is every library it compiles), and those are no part of what this
/// artifact is — keyed on them, a second program that loads one more library rebuilt it.
pub(crate) fn artifact_sources(
    data: &Data,
    pkg_dir: &str,
    pending: &[String],
    export_set: &HashSet<u32>,
) -> Vec<String> {
    let under = |file: &str, dir: &str| crate::file_access::is_under(file, dir);
    let roots: Vec<u32> = export_set.iter().copied().collect();
    let mut sources: HashSet<u16> = (0..data.definitions())
        .map(|d| data.def(d))
        .filter(|def| under(def.position().file.as_str(), pkg_dir))
        .map(|def| def.source)
        .chain(
            crate::opt::names::reachable_functions(data, &roots)
                .into_iter()
                .map(|d| data.def(d).source),
        )
        // The program's source is no library's: an instance the program made of one of the
        // library's generics lies in the library's file, and the program imports everything.
        .filter(|&s| s != crate::data::MAIN_SOURCE)
        .collect();
    loop {
        let before = sources.len();
        for imp in data.applied_imports() {
            if sources.contains(&imp.into_source) {
                sources.insert(imp.lib_source);
            }
        }
        if sources.len() == before {
            break;
        }
    }
    let mut out: Vec<String> = pending
        .iter()
        .filter(|dir| {
            dir.as_str() == pkg_dir
                || (0..data.definitions()).any(|d| {
                    let def = data.def(d);
                    // A stdlib definition can lie in a library's file — the wrapper type
                    // `main_vector<T>` the library's `vector<T>` minted — and is no code of it.
                    def.source != crate::data::STD_SOURCE
                        && sources.contains(&def.source)
                        && under(def.position().file.as_str(), dir)
                })
        })
        .cloned()
        .collect();
    if !out.iter().any(|d| d == pkg_dir) {
        out.push(pkg_dir.to_string());
    }
    out
}

/// @PLN199 G1b — which packages, by name and version (`loft.toml`), not by where they lie:
/// the same library at two places is the same artifact.  A registry version is immutable; an
/// edited local package is caught by the staleness check beside this key.  A directory with
/// no manifest is named by its last component.
pub(crate) fn contributing_fingerprint(contributing: &[String]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut ids: Vec<(String, String)> = contributing
        .iter()
        .map(|dir| {
            let m = crate::manifest::read_manifest(&format!("{dir}/loft.toml"));
            let name = m
                .as_ref()
                .and_then(|m| m.name.clone())
                .or_else(|| crate::file_access::file_name(dir))
                .unwrap_or_default();
            (name, m.and_then(|m| m.version).unwrap_or_default())
        })
        .collect();
    ids.sort_unstable();
    ids.dedup();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ids.hash(&mut h);
    crate::cache::host_triple().hash(&mut h);
    h.finish()
}

pub(crate) fn type_layout_fingerprint(stores: &Stores) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    stores.types.len().hash(&mut h);
    for t in &stores.types {
        t.name.hash(&mut h);
        t.size_bytes().hash(&mut h);
        t.align_bytes().hash(&mut h);
    }
    h.finish()
}

/// @PLN199 C1 — the numbers the artifact's code gives the functions it can reach.  Generated
/// code writes a function value's `d_nr` as a literal (a fn-ref dispatch arm, a `FnRef` it
/// makes), and the interpreter hands it values of ITS numbering, so an artifact serves only
/// a program that numbers those functions the same way.  A library's functions are numbered
/// before the script's, so editing the script does not change this.
pub(crate) fn fn_numbering_fingerprint(data: &Data, export_set: &HashSet<u32>) -> u64 {
    use std::hash::{Hash, Hasher};
    let roots: Vec<u32> = export_set.iter().copied().collect();
    let mut reach: Vec<u32> = crate::opt::names::reachable_functions(data, &roots)
        .into_iter()
        .collect();
    reach.sort_unstable();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for d in reach {
        d.hash(&mut h);
        data.def(d).name().hash(&mut h);
    }
    h.finish()
}

/// @PLN11 Arc N / N3 (Step 4) — a content hash of every `.loft` / `loft.toml` source
/// under `pkg_dirs` (build/artifact dirs skipped, files visited in sorted order for a
/// stable digest).  Used to detect "did this library's source change since the last
/// run?" — content-based, not mtime, so it is deterministic (testable) and a no-op
/// touch doesn't trigger a rebuild.
///
/// Spans every package that CONTRIBUTES to the artifact, not just the one that owns
/// it, for the reason given on `source_newer_than` (loft#777).
pub(crate) fn source_content_hash(pkg_dirs: &[String]) -> u64 {
    use sha2::{Digest, Sha256};
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> =
        pkg_dirs.iter().map(std::path::PathBuf::from).collect();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = crate::file_access::read_dir(&dir) else {
            continue;
        };
        for e in entries {
            let name = e.file_name().unwrap_or_default();
            if name == "native-auto" || name == "native" || name == "target" {
                continue;
            }
            let Ok(ft) = crate::file_access::symlink_metadata(&e).map(|m| m.file_type()) else {
                continue;
            };
            let p = e.os_spelling();
            if ft.is_dir() {
                stack.push(p);
                continue;
            }
            let is_src = crate::file_access::extension(&p)
                .is_some_and(|x| x.eq_ignore_ascii_case("loft"))
                || name == "loft.toml";
            if is_src {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut h = Sha256::new();
    for f in &files {
        // The path relative to its OWN package keeps the digest stable across
        // machines while still reacting to renames; content reacts to edits.
        let rel = pkg_dirs
            .iter()
            .find_map(|d| f.strip_prefix(d).ok())
            .unwrap_or(f);
        h.update(rel.to_string_lossy().as_bytes());
        if let Ok(bytes) = crate::file_access::read(f) {
            h.update(&bytes);
        }
    }
    let digest: [u8; 32] = h.finalize().into();
    u64::from_le_bytes(digest[..8].try_into().unwrap_or([0; 8]))
}

/// Is any `.loft` / `loft.toml` source under `pkg_dirs` newer than the artifact at
/// `artifact`?  (Build/artifact dirs are skipped.)  A missing/unreadable artifact
/// mtime counts as stale.
///
/// `pkg_dirs` is every package that CONTRIBUTES code to the artifact, not just the
/// one that owns it.  A cdylib carries its dependencies inlined — `emit_program`
/// emits the export set *and its transitive deps*, so `hex_editor`'s cdylib holds a
/// full copy of `hex_part::part_name_ok` — and it EXPORTS that copy under the same
/// `loft_shared_<name>` symbol the dependency's own cdylib exports.  Whichever
/// library loads first wins the lookup.
///
/// Asking only about the owning package therefore reported a dependent as fresh
/// after its DEPENDENCY was edited: the edited library rebuilt correctly and the
/// dependent kept serving its stale inlined copy, permanently — nothing about the
/// dependent's own sources ever changes again (loft#777).  It read as a
/// consumer-size effect only because you need a second library in the graph, loaded
/// first, before anything can shadow the fresh one; the small consumer that loaded
/// the edited library directly was always right.
///
/// The cost of the wider question is one `stat` walk over the loaded packages —
/// under a millisecond for a ten-package tree, against a `rustc` invocation. It does
/// mean a dependency edit makes every dependent stale, which is the honest answer:
/// the dependent really does contain the edited code.  `dev-interpret-on-edit` still
/// keeps `rustc` out of the loop until editing settles.
pub(crate) fn source_newer_than(pkg_dirs: &[String], artifact: &std::path::Path) -> bool {
    let Ok(art_mtime) = crate::file_access::metadata(artifact).and_then(|m| m.modified()) else {
        return true;
    };
    let mut newest: Option<std::time::SystemTime> = None;
    let mut stack: Vec<std::path::PathBuf> =
        pkg_dirs.iter().map(std::path::PathBuf::from).collect();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = crate::file_access::read_dir(&dir) else {
            continue;
        };
        for e in entries {
            let name = e.file_name().unwrap_or_default();
            if name == "native-auto" || name == "native" || name == "target" {
                continue; // build/artifact dirs
            }
            let Ok(ft) = crate::file_access::symlink_metadata(&e).map(|m| m.file_type()) else {
                continue;
            };
            let p = e.os_spelling();
            if ft.is_dir() {
                stack.push(p);
                continue;
            }
            let is_src = crate::file_access::extension(&p)
                .is_some_and(|x| x.eq_ignore_ascii_case("loft"))
                || name == "loft.toml";
            if is_src
                && let Ok(mt) = crate::file_access::symlink_metadata(&e).and_then(|m| m.modified())
                && newest.is_none_or(|n| mt > n)
            {
                newest = Some(mt);
            }
        }
    }
    newest.is_some_and(|src| src > art_mtime)
}

/// The fingerprint of the first `n` types of `stores`, or `None` when it has fewer.  The same
/// facts `native_lib`'s whole-table fingerprint hashes (name, size, alignment), over a prefix.
pub(crate) fn prefix_fingerprint(stores: &Stores, n: usize) -> Option<u64> {
    if stores.types.len() < n {
        return None;
    }
    let mut facts: Vec<Vec<u8>> = Vec::with_capacity(n + 1);
    facts.push((n as u64).to_le_bytes().to_vec());
    for t in &stores.types[..n] {
        let mut f = t.name.as_bytes().to_vec();
        f.extend_from_slice(&t.size_bytes().to_le_bytes());
        f.push(t.align_bytes());
        facts.push(f);
    }
    let parts: Vec<&[u8]> = facts.iter().map(Vec::as_slice).collect();
    Some(crate::cache::stable_hash(&parts))
}

/// @PLN199 G1 — the `[native]` packages a library artifact links: those that declare a
/// `#native` definition its exports reach.  The program around it may load others; linked
/// too, their symbols shared the artifact's flat C-ABI namespace, and one exported by two of
/// them refused a library that calls only its own.
pub(crate) fn linked_packages(data: &Data, export_set: &HashSet<u32>) -> Vec<(String, String)> {
    let roots: Vec<u32> = export_set.iter().copied().collect();
    let reach = crate::opt::names::reachable_functions(data, &roots);
    data.native_packages
        .iter()
        .filter(|(_, dir)| {
            reach.iter().any(|&d| {
                let def = data.def(d);
                !def.native().is_empty()
                    && *def.code() == crate::data::Value::Null
                    && crate::file_access::is_under(def.position().file.as_str(), dir)
            })
        })
        .cloned()
        .collect()
}

/// The `[library] native` stem of the package at `pkg_dir` and its resolved cdylib — the file
/// a library artifact that calls the package links (`native_lib::native_pkg_cabi_link_args`) and needs
/// loaded first where there is no RPATH ([`load_linked_packages`]).  The stem can differ from
/// the crate name; it is read from the manifest (the SAME stem the interpreter resolves with),
/// falling back to the crate name.  `None` when the cdylib cannot be resolved
/// ([`crate::extensions::resolve_native_lib`] already printed why).
pub(crate) fn native_pkg_lib(crate_name: &str, pkg_dir: &str) -> Option<(String, std::path::PathBuf)> {
    let stem = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
        .and_then(|m| m.native)
        .unwrap_or_else(|| crate_name.replace('-', "_"));
    let so = crate::native_build::resolve_native_lib(pkg_dir, &stem)?;
    Some((stem, std::path::PathBuf::from(so)))
}

/// Load, into this process, the cdylib of every `[native] crate` package the artifact for
/// `export_set` links, before anything opens that artifact.  Windows has no RPATH: the
/// artifact names `<package>.dll` as an import, and the loader resolves an import from the
/// modules already loaded or from its search path (the `.exe`'s directory, the system
/// directories, `PATH`) — never from the package's build directory, where the DLL is.  A
/// package loaded first is resolved by its module name.  Without it the artifact did not
/// load, its layout probe answered "unopenable", and every run rebuilt it and interpreted
/// the library anyway (loft#2147).  The host loads these packages for their own functions
/// regardless; this only moves that load ahead of the artifact's.
#[cfg(feature = "native-extensions")]
pub(crate) fn load_linked_packages(data: &Data, export_set: &HashSet<u32>) {
    if !crate::platform::loads_library_dependencies_first()
        || data.native_packages.is_empty()
        || !crate::platform::native_cabi_link()
    {
        return;
    }
    for (crate_name, pkg_dir) in &linked_packages(data, export_set) {
        if let Some((_, so)) = native_pkg_lib(crate_name, pkg_dir) {
            crate::c_libs::load_one(&so.to_string_lossy());
        }
    }
}

#[cfg(not(feature = "native-extensions"))]
pub(crate) fn load_linked_packages(_data: &Data, _export_set: &HashSet<u32>) {}
