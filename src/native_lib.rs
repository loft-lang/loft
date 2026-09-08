// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader / generator

//! @PLN11 Arc N / N2 — auto-generate a native cdylib from a library's functions.
//!
//! In the native-library execution model (C71) a stable library compiles to a
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
//! **shares the caller's real `Stores` by pointer** (zero-marshalling, C71's
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
use std::collections::{BTreeSet, HashSet};

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
    let decl_dups = crate::generation::duplicate_fn_names(data);
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
            crate::generation::disambiguated_fn_ident(&decl_dups, def)
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
    let Ok(bytes) = std::fs::read(rlib) else {
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
    let sibling = rlib.with_extension("rmeta");
    if sibling.exists() {
        return Some(sibling);
    }
    let dir = rlib.parent()?;
    let mut search: Vec<std::path::PathBuf> = vec![dir.join("deps")];
    if let Ok(units) = std::fs::read_dir(dir.join("build").join("loft")) {
        search.extend(units.flatten().map(|u| u.path().join("out")));
    }
    for candidate_dir in search {
        let Ok(entries) = std::fs::read_dir(&candidate_dir) else {
            continue;
        };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_none_or(|x| x != "rlib") {
                continue;
            }
            let is_loft = path
                .file_stem()
                .is_some_and(|st| st == "libloft" || st.to_string_lossy().starts_with("libloft-"));
            if !is_loft || !same_file(&path, rlib) {
                continue;
            }
            let paired = path.with_extension("rmeta");
            if paired.exists() {
                return Some(paired);
            }
        }
    }
    None
}

/// Are these two paths the SAME file?  Hard-link identity first (which is what an uplift
/// produces, and is free), then content — a copy is as good as a link for this question,
/// and answering `false` only costs the caller the metadata it was looking for.
fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    let (Ok(ma), Ok(mb)) = (a.metadata(), b.metadata()) else {
        return false;
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        if ma.dev() == mb.dev() && ma.ino() == mb.ino() {
            return true;
        }
    }
    if ma.len() != mb.len() {
        return false;
    }
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => false,
    }
}

/// @PLN11 Arc N / N3 — mark a library's functions for native dispatch.  Of the
/// `candidates` (a library's functions — e.g. all functions from a `use`d
/// library's source), the **shared-store-dispatchable** subset has its
/// `def.native` set to the bridge symbol `loft_shared_<name>`.  Returns that
/// subset (the cdylib export set).
///
/// This is the hook that turns a *normal* library function (a body, no
/// hand-written `#native`) into a native-dispatched one: once `def.native` is set,
/// `byte_code` routes every call to it through `OpStaticCall` (codegen.rs), the
/// stub is registered by `register_native_stubs`, and `wire_shared_native_fns`
/// wires the real bridge after the cdylib loads.  **Call before `byte_code`.**
pub fn mark_native_exports(data: &mut Data, candidates: &HashSet<u32>) -> HashSet<u32> {
    let exportable: HashSet<u32> = crate::native_gate::shared_store_dispatchable(data)
        .intersection(candidates)
        .copied()
        .collect();
    mark_exports(data, &exportable);
    exportable
}

/// @PLN11 Arc N / N3 (Step 2) — set `def.native = "loft_shared_<name>"` on each
/// function in `export` (which `byte_code` then routes through `OpStaticCall`).
/// Split out from [`mark_native_exports`] so the **build-before-mark** flow can
/// mark *only after* the cdylib compiles — a build failure simply never marks, so
/// the library interprets (Step 2's invariant: a library that can't compile native
/// silently interprets, no `exit`, no `OpStaticCall` to an unbuilt symbol).
pub fn mark_exports(data: &mut Data, export: &HashSet<u32>) {
    let dups = crate::generation::duplicate_fn_names(data);
    for &d in export {
        // The disambiguated ident (#305) — must equal the cdylib wrapper's
        // exported symbol, or `wire_shared_native_fns`' dlsym misses it.
        let sym = format!(
            "loft_shared_{}",
            crate::generation::disambiguated_fn_ident(&dups, data.def(d))
        );
        data.def_mut(d).native = sym;
    }
}

/// What proving a built cdylib against the process turned up.
#[derive(Debug, Default, Clone)]
pub struct BridgeProbe {
    /// Functions whose bridge symbol resolved — the ones actually marked.
    pub marked: usize,
    /// Original names of the functions left interpreting, sorted.  Empty when
    /// every export resolved.
    pub unresolved: Vec<String>,
    /// True when the artifact itself could not be `dlopen`ed, so none of its
    /// exports resolve.
    pub not_loaded: bool,
}

impl BridgeProbe {
    /// True when the whole export set is dispatching natively — nothing fell
    /// back to the interpreter.
    #[must_use]
    pub fn complete(&self) -> bool {
        !self.not_loaded && self.unresolved.is_empty()
    }
}

/// Load the cdylib at `so`, and mark for native dispatch exactly those functions
/// of `export` whose bridge symbol it really exports.
///
/// [`mark_exports`] marks the whole set on the strength of the BUILD having
/// succeeded.  That is a proxy, and it fails in both directions (loft#831): a
/// cdylib can build cleanly and still not load — linked against a different
/// `libloft.rlib`, missing a system library, or replaced by a concurrent build
/// between the freshness check and the load.  `byte_code` has by then emitted
/// `OpStaticCall` to a symbol that will never be wired, and the panic stub in
/// `compile.rs` takes the program down at the first call — even though the loft
/// body it was compiled from is sitting right there in the same process.
///
/// So ask the question whose answer is the one that matters, at the moment the
/// answer is still actionable: load the artifact and `dlsym` each bridge.  A
/// symbol that does not resolve leaves its function unmarked, which is precisely
/// the documented fallback for a library that cannot compile native — it
/// interprets, byte-identically.  Loading here also PINS the image for the
/// process, so pruning or rebuilding by a concurrent `loft` cannot invalidate
/// the decision after it is made.
pub fn probe_and_mark_exports(
    data: &mut Data,
    export: &HashSet<u32>,
    so: &std::path::Path,
) -> BridgeProbe {
    let mut probe = BridgeProbe::default();
    if !crate::extensions::load_cdylib(&so.to_string_lossy()) {
        // `load_cdylib` has already printed the classified dlopen diagnostic.
        probe.not_loaded = true;
        probe.unresolved = export
            .iter()
            .map(|&d| data.def(d).original_name().clone())
            .collect();
        probe.unresolved.sort_unstable();
        return probe;
    }
    let dups = crate::generation::duplicate_fn_names(data);
    let mut resolvable: HashSet<u32> = HashSet::new();
    for &d in export {
        let sym = format!(
            "loft_shared_{}",
            crate::generation::disambiguated_fn_ident(&dups, data.def(d))
        );
        if crate::extensions::bridge_symbol_resolves(&sym) {
            resolvable.insert(d);
        } else {
            probe.unresolved.push(data.def(d).original_name().clone());
        }
    }
    probe.unresolved.sort_unstable();
    probe.marked = resolvable.len();
    mark_exports(data, &resolvable);
    probe
}

/// @PLN11 Arc N / N3 (Step 2) — the cdylib **export set** for the library at
/// `pkg_dir`, computed **without marking** (`&Data`, not `&mut`): the library's
/// top-level, user-named, `pub` functions (the dispatch-target invariant — see
/// [`mark_library_native`]) intersected with the shared-store-dispatchable gate.
/// The build-before-mark flow builds the cdylib from this set, then calls
/// [`mark_exports`] only on success.
#[must_use]
pub fn library_export_set(data: &Data, pkg_dir: &str) -> HashSet<u32> {
    let candidates: HashSet<u32> = (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            matches!(def.def_type(), DefType::Function)
                && def.pub_visible
                && !is_synthetic_name(def.name())
                && def.position().file.starts_with(pkg_dir)
        })
        .collect();
    crate::native_gate::shared_store_dispatchable(data)
        .intersection(&candidates)
        .copied()
        .collect()
}

/// @PLN11 Arc N / N3 — mark a `use`d library's **public API** functions native.
///
/// **Invariant:** a dispatch target is a function the consuming script can directly
/// *name and `Call`* — a top-level, user-named, `pub` function owned by the package
/// at `pkg_dir` (by `def.position().file` prefix — the same ownership guard the
/// manifest path uses).  [`mark_native_exports`] then marks the
/// shared-store-dispatchable subset.
///
/// The candidate filter must exclude **synthetic** functions, not just lambdas: a
/// `pub fn`'s parse sprays `pub_visible` over every def it creates (`parser/mod.rs`),
/// so a nested lambda (`__lambda_N`) is also `pub_visible` — but it is a *fn-ref*
/// target the script cannot name, never a direct-`Call` dispatch target.  The
/// `__`-prefix is the codebase's synthetic-name convention (same marker
/// `native_gate` uses for synthetic params), so excluding it covers the whole class
/// of compiler-generated functions, not the lambda instance.  (Private helpers are
/// already excluded by `pub_visible`; they ride into the cdylib as reachable deps.)
pub fn mark_library_native(data: &mut Data, pkg_dir: &str) -> HashSet<u32> {
    let export = library_export_set(data, pkg_dir);
    mark_exports(data, &export);
    export
}

/// Is `stored_name` (an `n_<name>` definition name) a compiler-generated
/// (synthetic) function — i.e. its user-facing name starts with `__` (lambdas
/// `__lambda_N`, and any future synthetic kind)?  Such functions are never a
/// script-callable public API, so they are not auto-native dispatch targets.
fn is_synthetic_name(stored_name: &str) -> bool {
    stored_name
        .strip_prefix("n_")
        .unwrap_or(stored_name)
        .starts_with("__")
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

/// @PLN11 Arc N — a uniform 16/24-byte argument/return slot for the
/// **shared-store** native-library bridge ([`generate_shared_cdylib_lib_rs`]).
///
/// The bridge knows each slot's type from the function signature, so no tag is
/// needed: a scalar reads/writes `.scalar` (an `i64`, or float bits, or `0/1` for
/// bool), a `vector`/`reference` reads/writes `.dbref`.  `#[repr(C)]` so the
/// interpreter and the generated cdylib — **both linking this exact type from
/// libloft** — agree on layout with no marshalling.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct LibArg {
    /// Scalar payload: an `i64`, `f64::to_bits() as i64`, `f32::to_bits() as i64`,
    /// or a boolean as `0/1`.  Junk for non-scalar slots.
    pub scalar: i64,
    /// Reference payload: the raw stack `DbRef` for a `vector`/`reference` slot
    /// (passed through unchanged — the `--native` body expects the same indirect
    /// form the interpreter holds).  Junk for non-ref slots.
    pub dbref: crate::keys::DbRef,
    /// Text payload pointer: for a `text` arg, the UTF-8 bytes (borrowed from the
    /// caller's store for the call's duration — `--native` takes `&str`).  Null
    /// for non-text slots.
    pub text_ptr: *const u8,
    /// Text payload length (bytes), paired with `text_ptr`.
    pub text_len: usize,
}

impl LibArg {
    /// All-zero slot — the spread base so each `LibArg` literal sets only the one
    /// field its type uses (`LibArg { scalar: x, ..LibArg::ZERO }`, etc.).
    pub const ZERO: LibArg = LibArg {
        scalar: 0,
        dbref: crate::keys::DbRef {
            store_nr: 0,
            rec: 0,
            pos: 0,
        },
        text_ptr: std::ptr::null(),
        text_len: 0,
    };
}

/// Build the shared `--native` program (header + `init` + only the functions
/// reachable from `entry` + their deps) for `data`.  Both cdylib generators
/// append their export wrappers to this.
fn emit_program(data: &Data, stores: &Stores, entry: &[u32]) -> String {
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut out = Output::new(data, stores);
        // @PLN26 phase 2 — a cdylib that calls a `[native] crate` package emits
        // that package's fns as C-ABI `extern "C"` decls naming `loft_ffi::Loft*`,
        // its `.so` linked (sealing the package's whole Rust crate graph), exactly
        // as the executable native path does.  The SAME `native_cabi_enabled()`
        // gate drives the matching link flags in `build_shared_cdylib`, so codegen
        // and link never disagree.  No-op when the library uses no native package.
        out.native_cabi = native_cabi_link_enabled();
        // @PLN157 — a library cdylib is a SHIPPED binary: the lean tier, exactly as a
        // `--native-release` program.  Its functions run under the consumer's every
        // call, and the named per-call frame push was that lane's largest cost.
        out.emit_live = false;
        out.lean_tier = true;
        // Only the exported functions + their transitive deps (header + init + the
        // reachable subset) — exactly what a `--native` binary emits from `n_main`,
        // so unreachable operator stubs never surface.
        let _ = out.output_native_library(&mut buf, 0, data.definitions(), entry);
    }
    String::from_utf8(buf).unwrap_or_default()
}

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
    let mut src = emit_program(data, stores, &entry);
    let dups = crate::generation::duplicate_fn_names(data);
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
    let entry: Vec<u32> = export_set.iter().copied().collect();
    let mut src = emit_program(data, stores, &entry);
    let dups = crate::generation::duplicate_fn_names(data);
    for &d in export_set {
        src.push('\n');
        src.push_str(&shared_bridge_wrapper(data, &dups, d));
    }
    src.push('\n');
    src.push_str(&layout_fp_export(type_layout_fingerprint(stores)));
    src
}

/// The name of the symbol a generated cdylib exports to declare the type layout
/// it was built against.  Read by [`layout_fp_of_relocated`] before the artifact
/// is adopted.
pub(crate) const LAYOUT_FP_SYMBOL: &str = "loft_type_layout_fp_v1";

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
    let inner = crate::generation::disambiguated_fn_ident(dups, def); // e.g. "n_double"
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
fn shared_bridge_wrapper(data: &Data, dups: &HashSet<String>, d_nr: u32) -> String {
    use std::fmt::Write as _;
    let def = data.def(d_nr);
    // Same identifier the emitted program uses for this fn (#305).
    let inner = crate::generation::disambiguated_fn_ident(dups, def); // e.g. "n_vec_sum"

    let mut body = String::new();
    let mut fwd = String::new();
    let mut slot = 0usize; // next public-arg LibArg slot
    // @PLN118 arc F — hidden dests the bridge allocated as a FALLBACK (caller
    // forwarded a null/empty ref).  If the inner fn ignores its retbuf and returns
    // a fresh store (a struct-literal return does), that fallback record is orphaned
    // — one leaked store per call.  Freed after the call when the return differs.
    let mut fresh_dests: Vec<String> = Vec::new();
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
                let tname = hidden_dest_type_name(data, &a.typedef).unwrap_or_else(|| {
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
                let _ = writeln!(body, "    if {var}.rec == 0 && {var}.pos == 0 {{");
                let _ = writeln!(
                    body,
                    "        let _tid{slot} = unsafe {{ (&*cell.get()) }}.name({tname:?});"
                );
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
                fresh_dests.push(var.clone());
                slot += 1;
                let _ = write!(fwd, ", {var}");
            }
            crate::native_gate::BridgeAttrKind::WorkText => {
                // text_return work buffer (`&mut String`) — own a LOCAL String, pass
                // `&mut`.  The returned `Str` points into it; `bridge_write_ret` copies
                // the bytes into the caller-owned `bridge_text_dest` record (@PLN10
                // dest-passing) before this frame drops.
                let _ = writeln!(body, "    let mut {var}: String = String::new();");
                let _ = write!(fwd, ", &mut {var}");
            }
            crate::native_gate::BridgeAttrKind::Marshal => {
                let ty = rust_type(&a.typedef, &Context::Argument);
                let read = bridge_read(&a.typedef, &format!("a[{slot}]"));
                let _ = writeln!(body, "    let {var}: {ty} = {read};");
                slot += 1;
                let _ = write!(fwd, ", {var}");
            }
        }
    }

    let call = format!("{inner}(cell{fwd})");
    let ret_stmt = bridge_write_ret(def.returned(), &call, returns_owned_string(def));

    // @PLN118 arc F — after writing the return, free every bridge-allocated
    // FALLBACK dest the callee did not return (`(*ret).dbref` differs).  A hidden
    // dest only exists for an aggregate (dbref) return, so `(*ret).dbref` holds the
    // result here.  When the inner fn wrote into and returned the dest, the
    // identity matches and it is kept (the caller adopts + frees it); when the inner
    // fn ignored the retbuf and allocated its own store, the fallback is orphaned —
    // free it so it does not leak (one store per call across the interp↔cdylib
    // boundary; native whole-program has no bridge and is unaffected).
    let mut free_orphans = String::new();
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
         {free_orphans}\
         }}\n",
    )
}

/// The schema type id for a hidden `ref_return` destination of loft type `t`,
/// for the `OpDatabase(cell, ref, <id>)` allocation.  Vectors resolve via the
/// `main_vector<elm>` schema name (the same key `--native`'s `output_alloc_heap`
/// uses).  Other aggregates (struct `reference`, data-`enum`) need their own
/// type id and are not yet handled (the gate excludes them).
/// The SHARED-STORE type NAME for a hidden destination attr — resolved at
/// BRIDGE RUNTIME via `Stores::name` in the caller's store, because the
/// library's compile-time type IDS live in a different id space than the
/// caller's (@PLAN59: a lib-side constant id produced `claim(size=0)` /
/// "Incomplete record" aborts once struct dests became universal).
pub(crate) fn hidden_dest_type_name(data: &Data, t: &Type) -> Option<String> {
    match t.base() {
        Type::Vector(elm, _) => Some(format!("main_vector<{}>", elm.name(data))),
        Type::Reference(td, _) | Type::Enum(td, true, _) => Some(data.def(*td).name().to_string()),
        _ => None,
    }
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
fn bridge_write_ret(t: &Type, expr: &str, inner_owned: bool) -> String {
    // `Optional(τ)` rides τ's sentinel layout (@PLN25) — write as the base type.
    match t.base() {
        Type::Void | Type::Null => format!("let _ = {expr};"),
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
                 if let Some(__d) = __st.bridge_text_dest.take() {{\n    \
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

/// Is `t` a `text_return` work buffer — a `&mut String` the inner fn appends the
/// result into (loft IR type `RefVar(Text)`)?  The bridge owns a local `String`
/// for each and passes `&mut`.  Shared with `native_gate` (the gate admits exactly
/// these as the one allowed `__`-prefixed param, and only for a text-returning fn).
pub(crate) fn is_text_work_buffer(t: &Type) -> bool {
    matches!(t, Type::RefVar(inner) if matches!(**inner, Type::Text(_)))
}

/// Candidate `(rlib-lookup-dir, deps-dir)` pairs for a loft binary at `exe_dir`,
/// most authoritative first.  The dev/test layout looks beside the exe (`deps/`,
/// then the uplifted `<profile>/`); the INSTALLED layout (`<prefix>/bin/loft`)
/// keeps `libloft.rlib` + its deps under `<prefix>/share/loft/` — where `make
/// install` puts them, NOT next to the binary.  Without the share candidates the
/// installed loft can fingerprint its rlib (`cache::loft_rlib_path` has the same
/// fallback) but cannot LOCATE it to link a library's cdylib → "libloft.rlib not
/// found for this build".  Keep aligned with `cache::loft_rlib_path` (#304).
fn rlib_search_dirs(exe_dir: &std::path::Path) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
    // Dependency rlibs live in `<profile>/deps/`.  Real binary run: `exe_dir` is
    // `<profile>`, so `deps/` is its child.  Integration test: `exe_dir` already IS
    // `.../deps`.  Either way the returned link-search dir is that `deps/`.
    let dev_deps = if exe_dir.file_name().is_some_and(|n| n == "deps") {
        exe_dir.to_path_buf()
    } else {
        exe_dir.join("deps")
    };
    let mut out = vec![
        (dev_deps.clone(), dev_deps.clone()),
        (exe_dir.to_path_buf(), dev_deps.clone()),
    ];
    if exe_dir.file_name().is_some_and(|n| n == "bin")
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
        if !dir.is_dir() {
            continue;
        }
        let exact = dir.join("libloft.rlib");
        if exact.exists() {
            return Some((exact, deps_dir.clone()));
        }
        let hashed = std::fs::read_dir(dir)
            .ok()
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                let n = e.file_name().to_string_lossy().into_owned();
                n.starts_with("libloft-") && has_rlib_ext(&n)
            })
            .max_by_key(|e| e.metadata().and_then(|m| m.modified()).ok())
            .map(|e| e.path());
        if let Some(rlib) = hashed {
            return Some((rlib, deps_dir.clone()));
        }
    }
    None
}

/// Does filename `n` have a (case-insensitive) `.rlib` extension?
fn has_rlib_ext(n: &str) -> bool {
    std::path::Path::new(n)
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("rlib"))
}

/// `--extern name=path` flags for the optional feature-dep rlibs (random/png/…) in
/// `deps` that the generated stdlib code may reference — every `libX-<hash>.rlib`
/// except `libloft*` (which is passed explicitly as `--extern loft=`).
fn extra_externs(deps: &std::path::Path) -> Vec<(String, std::path::PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(deps) else {
        return out;
    };
    for e in entries.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if !n.starts_with("lib") || !has_rlib_ext(&n) || n.starts_with("libloft") {
            continue;
        }
        if let Some(stem) = n
            .strip_prefix("lib")
            .and_then(|s| s.rsplit_once('-'))
            .map(|x| x.0)
        {
            out.push((stem.to_string(), e.path()));
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
    let anchor = libloft.metadata().and_then(|m| m.modified()).ok();
    let mut candidates: Vec<(std::path::PathBuf, std::time::Duration)> = Vec::new();
    for e in std::fs::read_dir(deps).ok()?.flatten() {
        let n = e.file_name().to_string_lossy().into_owned();
        if !n.starts_with("libloft_ffi-") || !has_rlib_ext(&n) {
            continue;
        }
        // Symmetric gap to `libloft`'s mtime (clock direction ignored); a missing
        // mtime on either side sorts the candidate last via `Duration::MAX`.
        let gap = match (anchor, e.metadata().and_then(|m| m.modified()).ok()) {
            (Some(a), Some(m)) => m
                .duration_since(a)
                .or_else(|_| a.duration_since(m))
                .unwrap_or_default(),
            _ => std::time::Duration::MAX,
        };
        candidates.push((e.path(), gap));
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
    let Some(stem) = cand.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    let dir = std::env::temp_dir().join(format!("loft_ffi_probe_{}_{stem}", std::process::id()));
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let src = dir.join("probe.rs");
    let ok = std::fs::write(&src, "extern crate loft;\nextern crate loft_ffi;\n").is_ok()
        && std::process::Command::new("rustc")
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
            .output()
            .is_ok_and(|o| o.status.success());
    let _ = std::fs::remove_dir_all(&dir);
    ok
}

/// On Windows MSVC, the build-script output dirs holding native import libraries
/// (e.g. `windows.0.48.5.lib` from `windows-sys`) must be passed to a hand-driven
/// `rustc` as `-L` paths — cargo adds them via `cargo:rustc-link-search` but we
/// don't, so the cdylib link fails `LNK1181: cannot open input file …`.  Mirrors
/// the `--native` test runner's `find_native_lib_dirs`.  Empty (a no-op) off Windows.
#[cfg(not(windows))]
fn native_lib_search_dirs(_rlib: &std::path::Path) -> Vec<std::path::PathBuf> {
    Vec::new()
}

#[cfg(windows)]
fn native_lib_search_dirs(rlib: &std::path::Path) -> Vec<std::path::PathBuf> {
    // `rlib` is `target/<profile>/libloft.rlib` or `target/<profile>/deps/libloft-*.rlib`;
    // walk up to the profile dir, then scan `build/<crate>-<hash>/`.
    let Some(profile_dir) = rlib.parent().and_then(|p| {
        if p.file_name().is_some_and(|n| n == "deps") {
            p.parent()
        } else {
            Some(p)
        }
    }) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(profile_dir.join("build")) else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let build_entry = entry.path();
        // `out/` and its immediate subdirs (some crates emit into `out/<target>/`).
        let out = build_entry.join("out");
        if out.is_dir() {
            dirs.push(out.clone());
            if let Ok(subs) = std::fs::read_dir(&out) {
                dirs.extend(
                    subs.filter_map(|e| e.ok())
                        .map(|e| e.path())
                        .filter(|p| p.is_dir()),
                );
            }
        }
        // `cargo:rustc-link-search` directives cached in `build/<crate>-<hash>/output`
        // (e.g. `windows_x86_64_msvc` ships its `.lib` inside the registry package).
        if let Ok(content) = std::fs::read_to_string(build_entry.join("output")) {
            for line in content.lines() {
                if let Some(p) = line
                    .strip_prefix("cargo:rustc-link-search=native=")
                    .or_else(|| line.strip_prefix("cargo:rustc-link-search="))
                {
                    let p = std::path::PathBuf::from(p);
                    if p.is_dir() && !dirs.contains(&p) {
                        dirs.push(p);
                    }
                }
            }
        }
    }
    dirs
}

/// Platform cdylib filename for `stem` (`lib<stem>.so` / `.dylib` / `<stem>.dll`).
#[must_use]
pub fn platform_cdylib_name(stem: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{stem}.dll")
    } else if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else {
        format!("lib{stem}.so")
    }
}

/// @PLN26 — whether a cdylib links `[native] crate` packages by C-ABI (their
/// `.so`) rather than as Rust rlibs.  The library-crate twin of
/// `native_utils::native_cabi_enabled` (the binary-crate copy the executable path
/// reads): both read the ONE env gate, so the cdylib's codegen + link agree with
/// the executable's on a given host.  `LOFT_NATIVE_CABI=0` forces the legacy rlib
/// link (the escape hatch); the C-ABI path is the default on every host.
fn native_cabi_link_enabled() -> bool {
    !matches!(std::env::var("LOFT_NATIVE_CABI").ok().as_deref(), Some("0"))
}

/// @PLN26 phase 2 — the rustc link flags for ONE native package's resolved cdylib
/// `.so`: `-L native=<dir>` + `-l dylib=<name>` + an RPATH (the build/prebuilt
/// dir and `$ORIGIN` for an installed binary shipping the `.so` beside it).  On
/// Windows a DLL links through its import library and there is no RPATH, so this
/// bridges `<stem>.dll.lib` → `<stem>.lib` and the loader finds the staged DLL
/// beside the binary instead.
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
    // The cdylib is named after the `[library] native` stem, which can differ
    // from the crate name; read it from the manifest (the SAME stem the
    // interpreter resolves with), falling back to the crate name when absent.
    let stem = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
        .and_then(|m| m.native)
        .unwrap_or_else(|| crate_name.replace('-', "_"));
    let Some(so) = crate::extensions::resolve_native_lib(pkg_dir, &stem) else {
        return Vec::new();
    };
    let so_path = std::path::PathBuf::from(&so);
    let Some(so_dir) = so_path.parent() else {
        return Vec::new();
    };
    // `-l dylib=<name>` derived from the RESOLVED file (strip `lib` prefix +
    // extension) so a prebuilt or non-`lib<stem>` cdylib links.
    let libname = so_path
        .file_stem()
        .and_then(|s| s.to_str())
        .map_or(stem.as_str(), |s| s.strip_prefix("lib").unwrap_or(s));
    let mut args = vec![
        "-L".to_string(),
        format!("native={}", so_dir.display()),
        "-l".to_string(),
        format!("dylib={libname}"),
    ];
    if cfg!(windows) {
        // Naming bridge: a Rust cdylib's import lib is `<stem>.dll.lib`, but
        // `-l dylib=<stem>` makes MSVC link.exe open `<stem>.lib`.  Copy
        // `<stem>.dll.lib` → `<stem>.lib` beside it (identical content) so the
        // `-l dylib=` resolves.
        let dll_lib = so_dir.join(format!("{libname}.dll.lib"));
        let plain_lib = so_dir.join(format!("{libname}.lib"));
        if dll_lib.exists() && !plain_lib.exists() {
            let _ = std::fs::copy(&dll_lib, &plain_lib);
        }
        // Disallow-the-unverifiable-loudly: with NEITHER import-lib name present
        // the link dies on an opaque `LNK1181`, so name it rather than mis-link.
        if !plain_lib.exists() && !dll_lib.exists() {
            eprintln!(
                "loft: native package `{crate_name}` cdylib at {} has no import \
                 library (`{libname}.dll.lib` / `{libname}.lib`) — Windows links a \
                 DLL through its import lib (@PLN26 phase 4); rebuild the package's \
                 cdylib with a toolchain that emits one.",
                so_dir.display()
            );
        }
    } else {
        // Two RPATH entries: the build/prebuilt dir (run-from-build-tree) AND
        // `$ORIGIN` (an installed binary shipping the `.so` beside it).  `$ORIGIN`
        // is literal; the dynamic loader expands it at run time.
        args.push(format!("-Clink-arg=-Wl,-rpath,{}", so_dir.display()));
        args.push("-Clink-arg=-Wl,-rpath,$ORIGIN".to_string());
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
    let (rlib, deps) = find_loft_rlib().ok_or(
        "libloft.rlib not found for this build — run `cargo build --release --lib` \
         (or `make check-rlib` to check before a gate); a `cargo build --bin loft` \
         refreshes the binary but not the library rlib the native path links",
    )?;
    let src = generate_shared_cdylib_lib_rs(data, stores, export_set);
    std::fs::create_dir_all(out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let rs = out_dir.join(format!("{stem}.rs"));
    std::fs::write(&rs, &src).map_err(|e| format!("write {}: {e}", rs.display()))?;
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
        // Full optimisation, as `--native-release` compiles a program (see its
        // rustc arguments in `main.rs`): a library cdylib is a shipped binary.
        "-C".to_string(),
        "opt-level=3".to_string(),
        "-C".to_string(),
        "codegen-units=1".to_string(),
        "--crate-type".to_string(),
        "cdylib".to_string(),
        "-o".to_string(),
        tmp_so.display().to_string(),
        rs.display().to_string(),
    ];
    args.extend(crate::native_lib::loft_extern_args(&rlib));
    args.extend(["-L".to_string(), deps.display().to_string()]);
    // macOS records the `-o` path inside the Mach-O as its INSTALL NAME, and the
    // rename below moves the file without touching it — so this cdylib claimed to
    // live at `<stem>.building`, in an absolute build directory. Harmless only for
    // as long as every consumer `dlopen`s it BY PATH (a path load ignores the
    // recorded name); it breaks the moment one is resolved through `@rpath` or the
    // package is moved. Same flag, same reason, as the `cc`-built shim.
    let final_name = so
        .file_name()
        .map_or_else(|| stem.to_string(), |f| f.to_string_lossy().into_owned());
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
        for (crate_name, pkg_dir) in &data.native_packages {
            args.extend(native_pkg_cabi_link_args(crate_name, pkg_dir));
        }
    }
    // Windows MSVC: add the build-script `-L` dirs holding native import libs
    // (`windows.0.48.5.lib` etc.) or the link fails LNK1181.  No-op off Windows.
    for dir in native_lib_search_dirs(&rlib) {
        args.push("-L".to_string());
        args.push(dir.display().to_string());
    }
    // @PLN54 S9 — mixed-boundary (C71) AddressSanitizer.  When the interpreter
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
    std::fs::write(&argfile, contents).map_err(|e| format!("write {}: {e}", argfile.display()))?;
    let mut rustc = std::process::Command::new("rustc");
    rustc.arg(format!("@{}", argfile.display()));
    // @PLN54 S9 — the ASan cdylib (above) needs nightly rustc for `-Zsanitizer`.
    if std::env::var_os("LOFT_NATIVE_ASAN").is_some()
        && std::env::var_os("RUSTUP_TOOLCHAIN").is_none()
    {
        rustc.env("RUSTUP_TOOLCHAIN", "nightly");
    }
    let output = rustc
        .output()
        .map_err(|e| format!("launch rustc: {e} (is the Rust toolchain installed?)"))?;
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
    std::fs::rename(&tmp_so, &so)
        .map_err(|e| format!("install {} -> {}: {e}", tmp_so.display(), so.display()))?;
    Ok(so)
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
    use std::sync::OnceLock;
    static AVAIL: OnceLock<bool> = OnceLock::new();
    *AVAIL.get_or_init(|| {
        std::process::Command::new("rustc")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

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

/// Derive the auto-native cdylib crate stem from a package directory.
///
/// The stem becomes the cdylib's crate name (rustc derives the crate name from the
/// source-file stem when no `--crate-name` is given), so every character must be
/// valid in a Rust identifier.  A registry dir carries a dotted version
/// (`glb-0.1.0`): rustc maps `-`→`_` itself but rejects the surviving `.`, so map
/// **every** non-`[A-Za-z0-9_]` char to `_` — enforcing the full invalid-character
/// class, not just `-`/`.`.  The constant `loft_auto_` prefix guarantees the leading
/// character is alphabetic (a crate name may not start with a digit).  (#294)
#[must_use]
pub fn auto_cdylib_stem(pkg_dir: &str) -> String {
    let raw = std::path::Path::new(pkg_dir)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("lib");
    format!(
        "loft_auto_{}",
        raw.replace(|c: char| !(c.is_ascii_alphanumeric() || c == '_'), "_")
    )
}

/// #461 — combine two fingerprints into one stable, order-sensitive digest.
fn mix_fp(a: u64, b: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    a.hash(&mut h);
    b.hash(&mut h);
    h.finish()
}

/// #461 — a digest of the type-table LAYOUT a cdylib would be built against:
/// the index → (name, size, align) mapping, in order.  Two contexts with the
/// same digest assign every type the same index, so a cdylib's hardcoded
/// `db_tp` indices stay valid when reused across them; a different digest means
/// the indices would resolve to the wrong type and the cdylib must be rebuilt.
/// The type layout an already-built artifact declares, or `None` when it does not
/// say (a hand-written `#native` cdylib, or one generated before loft#717).
///
/// Opening the library to ask is safe in a way that USING it is not: the symbol is
/// a constant-returning `extern "C" fn() -> u64`, so nothing resolves a type index
/// or touches a store. An artifact that cannot be opened answers `None` and is
/// treated as unusable by the caller, which rebuilds — the same outcome dlopen
/// failure would have produced later, minus the corruption in between.
///
/// `None` for an artifact with no symbol deliberately does NOT reject it: a
/// hand-written cdylib is not generated against a type table and has nothing to
/// declare. Only a generated artifact is held to the check, and every generated
/// artifact now carries the symbol.
///
/// **Give this a RELOCATED path, never an artifact's own** — that is the whole of
/// the name.  Loading a path this process may republish later is what loft#777 and
/// loft#999 both were; [`layout_fp_off_path`] is the only caller and exists to
/// guarantee it.
#[cfg(feature = "native-extensions")]
fn layout_fp_of_relocated(so: &std::path::Path) -> LayoutProbe {
    unsafe {
        let Ok(lib) = libloading::Library::new(so) else {
            return LayoutProbe::Unopenable;
        };
        let sym = format!("{LAYOUT_FP_SYMBOL}\0");
        match lib.get::<unsafe extern "C" fn() -> u64>(sym.as_bytes()) {
            Ok(f) => LayoutProbe::Declares(f()),
            Err(_) => LayoutProbe::Undeclared,
        }
    }
}

#[cfg(not(feature = "native-extensions"))]
fn layout_fp_of_relocated(_so: &std::path::Path) -> LayoutProbe {
    // No loader in this build, so nothing can be asked — the same answer a
    // hand-written cdylib gives, which is what the caller already tolerates.
    LayoutProbe::Undeclared
}

/// What asking an artifact for its layout produced.
///
/// Three outcomes, because two of them used to be one.  `layout_fp_of_relocated`
/// answered `None` both when the artifact declared nothing (a hand-written
/// cdylib, which is fine to adopt) and when it could not be OPENED at all — and
/// the adopter read that as "fine to adopt", then handed the caller a path whose
/// `dlopen` would fail.  Harmless while nothing removed artifacts; `prune_artifacts`
/// does, and the fresh fast path adopts without taking the build lock, so the two
/// must be told apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// A build without the loader has nothing that can OPEN an artifact, so its
// `layout_fp_of_relocated` answers `Undeclared` unconditionally and these two are
// never constructed there.  They are still matched by every reader, so the
// variants stay — the outcome set is the type's contract, not a per-build fact.
#[cfg_attr(not(feature = "native-extensions"), allow(dead_code))]
enum LayoutProbe {
    /// Could not be opened — torn, half-written, or deleted between the existence
    /// check and the load.  Never adopt; rebuilding is always available.
    Unopenable,
    /// Opened, and names no layout: a hand-written cdylib, generated against no
    /// type table and with nothing to declare.
    Undeclared,
    /// Opened, and names the layout it was generated for.
    Declares(u64),
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
    let ext = if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut built: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == ext))
        .filter(|e| artifact_stem(&e.path()).is_some_and(|s| s.starts_with(family)))
        .filter_map(|e| {
            let m = e.metadata().ok()?.modified().ok()?;
            Some((m, e.path()))
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
        let stem = stem.to_string();
        let _ = std::fs::remove_file(&so);
        let _ = std::fs::remove_file(dir.join(format!("{stem}.rs")));
        let _ = std::fs::remove_file(dir.join(format!("{stem}.args")));
    }
}

/// `libfoo_<fp>.so` / `foo_<fp>.dll` → the `foo_<fp>` the generated sources are
/// named after.  Only the platform prefix differs, and stripping it is what lets
/// one loop delete a library and its generated companions together — and what
/// lets the sweep tell one family of artifacts from another.
fn artifact_stem(so: &std::path::Path) -> Option<&str> {
    let name = so.file_stem().and_then(|s| s.to_str())?;
    Some(name.strip_prefix("lib").unwrap_or(name))
}

/// May the artifact at `so` be adopted by a context whose type layout is
/// `layout_fp`?
///
/// True when it declares `layout_fp`, or declares nothing at all (a hand-written
/// cdylib — see [`layout_fp_of_relocated`]).  False when it declares a DIFFERENT
/// layout, which is the case that used to corrupt, and false when it cannot be
/// asked at all — an artifact this process cannot load is not one it can adopt,
/// and saying so here is what lets `prune_artifacts` delete concurrently.
///
/// Note this compares the RAW `type_layout_fingerprint`, not the mixed key the
/// filename carries: an artifact built by a different loft BUILD cannot link
/// against this one's rlib at all, so the layout is the part worth asserting here.
///
/// The question is always asked through [`layout_fp_off_path`], never at `so`
/// itself, because a caller that hears "no" REBUILDS at that very path — see
/// there for what a load of the real path costs on macOS.  There is deliberately
/// no second entry point that probes in place: which variant a call site picked
/// used to be the difference between a correct answer and a silently stale one
/// (loft#999).
fn artifact_matches_layout(so: &std::path::Path, layout_fp: u64) -> bool {
    match layout_fp_off_path(so) {
        LayoutProbe::Undeclared => true,
        LayoutProbe::Declares(found) => found == layout_fp,
        LayoutProbe::Unopenable => false,
    }
}

/// Read an artifact's layout fingerprint WITHOUT handing its own path to
/// `dlopen`: link (or copy) it to a throwaway name, ask that, and leave the real
/// path untouched.
///
/// The path matters because a caller that hears "this artifact does not match"
/// rebuilds at exactly that path, and macOS dyld caches a loaded image BY PATH
/// for the life of the process while its `dlclose` is a no-op.  Probing the old
/// artifact in place and then publishing a fresh one at the same path made the
/// later load hand back the STALE image dyld had already cached: the settling run
/// executed pre-edit code while writing the correct file for next time
/// (loft#777's macOS tail).  Linux keys `dlopen` on `(dev, inode)` and loads the
/// new file, which is why only macOS breaks.
///
/// Dropping the probe instead is what this replaces, and it cost the loft#717
/// guard: the justification was that a rebuild always follows, and it does not —
/// branch 3's edit-loop arm answers `Ok(None)` and leaves the artifact in place,
/// so a foreign-layout cdylib stopped being rebuilt
/// (`n3_use_native::a_foreign_context_artifact_is_rejected_not_adopted`).
///
/// **A relocation that fails answers `Unopenable`, never a probe in place.** The
/// in-place fallback is what loft#999 was: one unlucky `copy` — a full disk, an
/// exhausted file-descriptor limit, a concurrent prune — silently restored the
/// pre-fix behaviour, so `n3_parity::a_dependency_edit_invalidates_its_dependents_cdylib`
/// failed on macOS twice in a fortnight and passed the other eleven times.
/// `Unopenable` means "do not adopt", so the caller rebuilds: a wasted `rustc`
/// where the old file was in fact fine, against a wrong answer that no later run
/// clears.  Rebuilding is always available; being right is not optional.
///
/// A HARD LINK first, and a byte copy only if that fails, is what keeps the
/// honest answer cheap: the link is O(1) and consumes no space, so the conditions
/// that made the copy fail no longer decide whether the artifact is adopted, and
/// the probe is cheap enough for the fresh fast path to use the same one.  A link
/// shares the artifact's inode, which is safe on both keying schemes — the
/// rebuild publishes a NEW inode by rename, so nothing can match it, and where
/// the caller ADOPTS instead the bytes are identical anyway.
fn layout_fp_off_path(so: &std::path::Path) -> LayoutProbe {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let (Some(dir), Some(name)) = (so.parent(), so.file_name()) else {
        return LayoutProbe::Unopenable;
    };
    // A SUBDIRECTORY rather than a sibling file, and the relocated artifact keeps
    // its own file NAME inside it.  `native-auto/` is enumerated by extension
    // elsewhere — the parity tests count `so`/`dylib`/`dll` to assert how many
    // artifacts a context built — so a probe beside the real one would be counted
    // as a second artifact.  A dot-prefixed directory has no such extension, and
    // the name inside stays the one `LoadLibrary` wants on Windows.
    let probe_dir = dir.join(format!(
        ".layout-probe-{}-{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    if let Err(e) = std::fs::create_dir_all(&probe_dir) {
        report_unprobeable(so, &format!("create {}: {e}", probe_dir.display()));
        return LayoutProbe::Unopenable;
    }
    let probe = probe_dir.join(name);
    let fp = match relocate_for_probe(so, &probe) {
        Ok(()) => layout_fp_of_relocated(&probe),
        Err(e) => {
            report_unprobeable(so, &e);
            LayoutProbe::Unopenable
        }
    };
    // The probe has been read; the process may keep the image mapped (dlclose is a
    // no-op on macOS), and unlinking a mapped file is fine on both unixes.
    let _ = std::fs::remove_dir_all(&probe_dir);
    fp
}

/// Put the artifact at `so` under the throwaway name `probe`: a hard link, or a
/// byte copy where linking is not available (a filesystem without links, or a
/// `probe` that somehow lands on another device).
///
/// `LOFT_FORCE_PROBE_RELOCATE_FAIL` makes both legs fail, which is how the tests
/// reach the answer this used to get wrong on macOS alone.
fn relocate_for_probe(so: &std::path::Path, probe: &std::path::Path) -> Result<(), String> {
    if std::env::var_os("LOFT_FORCE_PROBE_RELOCATE_FAIL").is_some() {
        return Err("LOFT_FORCE_PROBE_RELOCATE_FAIL".to_string());
    }
    if std::fs::hard_link(so, probe).is_ok() {
        return Ok(());
    }
    std::fs::copy(so, probe)
        .map(|_| ())
        .map_err(|e| format!("link and copy both failed: {e}"))
}

/// Say once that an artifact could not be moved off its path to be asked about
/// its type layout, so it was rebuilt rather than adopted.
///
/// Once per process, because the condition is a property of the machine (space,
/// descriptors, permissions) rather than of one library: repeating it per library
/// per run would bury the one line that matters.  It is worth saying at all
/// because the alternative is what loft#999 was — an unexplained rebuild is
/// noticeable and recoverable, an unexplained STALE ANSWER is neither.
fn report_unprobeable(so: &std::path::Path, why: &str) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static SAID: AtomicBool = AtomicBool::new(false);
    if SAID.swap(true, Ordering::Relaxed) {
        return;
    }
    eprintln!(
        "loft: cannot copy {} aside to read its type layout ({why}); rebuilding it \
         instead of reusing it.  Check free space and the open-file limit if builds \
         stay slow.",
        so.display()
    );
}

fn type_layout_fingerprint(stores: &Stores) -> u64 {
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

/// @PLN11 Arc N / N3 (Step 4 — **dev-interpret-on-edit**) — decide how the library
/// at `pkg_dir` should run this invocation, and build its cdylib only when warranted.
///
/// Returns:
/// - `Ok(Some(so))` — dispatch native against the cdylib at `so` (fresh cached, or
///   just built);
/// - `Ok(None)` — **interpret this run** (the library is being actively edited);
/// - `Err(_)` — a build was attempted and failed (the caller interprets + warns).
///
/// The policy reconciles "a library is always native" with "no `rustc` per save":
/// 1. **Fresh artifact** (`.so` exists, fingerprint matches, source not newer) →
///    native, no work.
/// 2. **No artifact yet, or `loft` itself changed** (fingerprint mismatch) → **build
///    eagerly** → native.  First use of a library, and a deployed/stable dep, pay the
///    one-time build and run native from the start (this is also why the parity tests,
///    which wipe `native-auto/` then run once, still dispatch native).
/// 3. **Stale artifact** (an `.so` exists but the source is newer — the library was
///    edited) → consult the last-run source hash:
///    - *changed since the previous run* → the edit loop is live → **interpret this
///      run** (`Ok(None)`); record the new hash.  No `rustc`.
///    - *unchanged since the previous run* → editing has settled → **rebuild** → native.
///
/// So an edit-run-edit-run loop interprets each time (no `rustc`); the first run after
/// you stop editing rebuilds and the library is native again.  (Polish: do that
/// rebuild in the background so even the settling run never blocks — see the plan's
/// Step 4 option 3.)
///
/// # Errors
/// Propagates a build failure from [`build_shared_cdylib`].
pub fn cached_or_build_shared_cdylib(
    data: &Data,
    stores: &Stores,
    export_set: &HashSet<u32>,
    pkg_dir: &str,
    contributing: &[String],
) -> Result<Option<std::path::PathBuf>, String> {
    let out_dir = std::path::Path::new(pkg_dir).join("native-auto");
    // #461 — the generated cdylib hardcodes type-table INDICES (e.g. `OpWriteFile`'s
    // `db_tp`), but those indices SHIFT with which libraries are loaded (an `i32`
    // write is `db_tp=64` standalone, `67` once `hex_grid` is parsed).  At runtime
    // the cdylib resolves the index against the caller's SHARED `Stores` type table,
    // so a cdylib built in one consumer's context, then reused (cached per-library)
    // by another consumer whose table differs, reads the WRONG type and corrupts
    // (the moros GLB header wrote 8-byte fields for `as i32` → version 0).  Fold the
    // caller's type-table layout into the freshness key so a context mismatch
    // rebuilds instead of silently linking an index-incompatible cdylib.
    let layout_fp = type_layout_fingerprint(stores);
    let fp = mix_fp(crate::cache::loft_build_fingerprint(), layout_fp);
    // loft#715 — and put that key in the artifact's NAME, so two contexts can
    // never name the same file.  The fingerprint alone was not enough: the fast
    // path below reads `so.exists()` and the sidecar WITHOUT the build lock, so a
    // process whose context still matched the stamped fp could take the path
    // while another process, mid-build for a DIFFERENT table, renamed its own
    // artifact over it.  The publish is atomic, so the reader never saw a torn
    // file — it saw a COMPLETE library built for someone else's type indices, and
    // a garbage index reaching `Stores::add` aborts the process with "Cannot add
    // to none-structure" (a non-unwinding panic: the whole run dies).  It needed
    // two overlapping processes to show, which is why it read as 1 start in 12.
    //
    // Content-addressing removes the class rather than narrowing the window: a
    // context only ever opens its own file, lock or no lock.
    let stem = format!("{}_{fp:016x}", auto_cdylib_stem(pkg_dir));
    let so = out_dir.join(platform_cdylib_name(&stem));

    // 1. Fresh artifact → native, no hashing.  The FILENAME carries `fp`, so its
    // existence is the fingerprint match — the shared `.loft-build-fp` sidecar is
    // per-DIRECTORY and two contexts would otherwise invalidate each other's
    // stamp on every run, rebuilding forever (loft#715).  The sidecar is still
    // written below for the readers that report it.
    // loft#717 — and ASK the artifact what layout it was built for, rather than
    // inferring it from the name.  Content-addressing (#715) makes a collision
    // unreachable by construction, but "by construction" is an argument, not a
    // check: it holds only while the fingerprint keeps covering every layout
    // difference and while nothing else can put a file at this path.  When it
    // does not hold the failure is silent corruption, so the cheap verification
    // is worth more than the argument.  A mismatch falls through to REBUILD,
    // which is the same thing a name miss already does — and REBUILD is why this
    // probe, too, must not load `so` itself (loft#999).
    if so.exists()
        && !source_newer_than(contributing, &so)
        && artifact_matches_layout(&so, layout_fp)
    {
        return Ok(Some(so));
    }

    // Concurrent `loft` processes (parallel tests are the common case)
    // routinely load the same library at once; an unserialized double-build
    // rewrites the generated `.rs` under a running rustc and tears the `.so`
    // (mixed-object "dangerous relocation" link errors, "file too short" on
    // load).  Take an exclusive advisory lock for the check+build, then
    // RE-CHECK freshness: the waiter adopts the winner's artifact instead of
    // rebuilding over it.  Released when `lock` drops, on every return path.
    std::fs::create_dir_all(&out_dir).map_err(|e| format!("create {}: {e}", out_dir.display()))?;
    let lock_path = out_dir.join(".build.lock");
    let lock = std::fs::File::create(&lock_path)
        .map_err(|e| format!("create {}: {e}", lock_path.display()))?;
    lock.lock()
        .map_err(|e| format!("lock {}: {e}", lock_path.display()))?;
    if so.exists()
        && !source_newer_than(contributing, &so)
        && artifact_matches_layout(&so, layout_fp)
    {
        return Ok(Some(so));
    }

    let build = |out_dir: &std::path::Path| -> Result<Option<std::path::PathBuf>, String> {
        let built = build_shared_cdylib(data, stores, export_set, out_dir, &stem)?;
        crate::cache::write_native_artifact_fingerprint(out_dir, fp);
        // Bound the directory, while the build lock is still held.  A new artifact
        // appears per consumer type-layout (#715) and nothing collected the old
        // ones — 25 GB across this repo's fixtures, 532 files in one package.
        // Scoped to THIS package's auto-native family: a `[c] shim` cdylib shares
        // the directory and is never rebuilt, so an unscoped sweep eats it.
        prune_artifacts(out_dir, &auto_cdylib_stem(pkg_dir));
        Ok(Some(built))
    };

    // 2. No artifact yet, or `loft` changed → build eagerly (native from the start).
    // An artifact built for a DIFFERENT type layout counts as "no artifact"
    // (loft#717): it can never be adopted, and letting it reach the edit-loop
    // branch below would leave a foreign artifact sitting at this name — branch 3
    // does NOT always rebuild, its edit-loop arm answers `Ok(None)` and leaves the
    // file alone.
    //
    // The probe never touches this path — see [`layout_fp_off_path`].  This branch
    // is about to REPLACE the file it is asking about, and on macOS dyld caches a
    // loaded image by PATH for the process, so probing here and rebuilding at the
    // same path made the later load return the stale image (loft#777, and loft#999
    // for the one-unlucky-`copy` route back into it).
    if !so.exists() || !artifact_matches_layout(&so, layout_fp) {
        crate::cache::write_run_source_hash(&out_dir, source_content_hash(contributing));
        return build(&out_dir);
    }

    // 3. Stale artifact (the library was edited) → dev-interpret-on-edit.
    let cur = source_content_hash(contributing);
    let stable = crate::cache::read_run_source_hash(&out_dir) == Some(cur);
    crate::cache::write_run_source_hash(&out_dir, cur);
    if stable {
        build(&out_dir) // editing settled → rebuild, native again
    } else {
        Ok(None) // still being edited → interpret this run, no rustc
    }
}

/// @PLN11 Arc N / N3 (Step 4) — a content hash of every `.loft` / `loft.toml` source
/// under `pkg_dirs` (build/artifact dirs skipped, files visited in sorted order for a
/// stable digest).  Used to detect "did this library's source change since the last
/// run?" — content-based, not mtime, so it is deterministic (testable) and a no-op
/// touch doesn't trigger a rebuild.
///
/// Spans every package that CONTRIBUTES to the artifact, not just the one that owns
/// it, for the reason given on `source_newer_than` (loft#777).
fn source_content_hash(pkg_dirs: &[String]) -> u64 {
    use sha2::{Digest, Sha256};
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    let mut stack: Vec<std::path::PathBuf> =
        pkg_dirs.iter().map(std::path::PathBuf::from).collect();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name();
            if name == "native-auto" || name == "native" || name == "target" {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                stack.push(p);
                continue;
            }
            let is_src = p
                .extension()
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
        if let Ok(bytes) = std::fs::read(f) {
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
fn source_newer_than(pkg_dirs: &[String], artifact: &std::path::Path) -> bool {
    let Ok(art_mtime) = artifact.metadata().and_then(|m| m.modified()) else {
        return true;
    };
    let mut newest: Option<std::time::SystemTime> = None;
    let mut stack: Vec<std::path::PathBuf> =
        pkg_dirs.iter().map(std::path::PathBuf::from).collect();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            let name = e.file_name();
            if name == "native-auto" || name == "native" || name == "target" {
                continue; // build/artifact dirs
            }
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                stack.push(p);
                continue;
            }
            let is_src = p
                .extension()
                .is_some_and(|x| x.eq_ignore_ascii_case("loft"))
                || name == "loft.toml";
            if is_src
                && let Ok(mt) = e.metadata().and_then(|m| m.modified())
                && newest.is_none_or(|n| mt > n)
            {
                newest = Some(mt);
            }
        }
    }
    newest.is_some_and(|src| src > art_mtime)
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
                .any(|(d, _)| d.to_string_lossy().contains("share/loft")),
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

#[cfg(test)]
mod rmeta_pairing_tests {
    use super::loft_rmeta_beside;

    /// A cargo-shaped layout: the pair in `deps/`, the rlib UPLIFTED beside it as a hard
    /// link.  That link is what makes the pairing findable without guessing.
    fn layout(root: &std::path::Path, uplift_links_to_hash: bool) -> std::path::PathBuf {
        let deps = root.join("deps");
        std::fs::create_dir_all(&deps).unwrap();
        let hashed = deps.join("libloft-1111111111111111.rlib");
        std::fs::write(&hashed, b"rlib-bytes").unwrap();
        std::fs::write(
            deps.join("libloft-1111111111111111.rmeta"),
            b"the-paired-one",
        )
        .unwrap();
        let uplifted = root.join("libloft.rlib");
        if uplift_links_to_hash {
            std::fs::hard_link(&hashed, &uplifted).unwrap();
        } else {
            std::fs::write(&uplifted, b"a-different-rlib-entirely").unwrap();
        }
        uplifted
    }

    #[test]
    fn the_rmeta_paired_with_our_rlib_is_the_one_chosen() {
        let root = std::env::temp_dir().join(format!("loft_pair_ok_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let rlib = layout(&root, true);
        let found = loft_rmeta_beside(&rlib).expect("the paired rmeta");
        assert_eq!(std::fs::read(&found).unwrap(), b"the-paired-one");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The reported failure: debris from an older build, NEWER than the pair, sitting in
    /// the same directory.  Choosing by recency picks it; choosing by identity does not.
    #[test]
    fn newer_unpaired_debris_is_not_chosen() {
        let root = std::env::temp_dir().join(format!("loft_pair_debris_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let rlib = layout(&root, true);
        // Debris: an rmeta with no rlib of its own, written last so it is the newest file.
        std::fs::write(
            root.join("deps").join("libloft-9999999999999999.rmeta"),
            b"stale-debris",
        )
        .unwrap();
        let found = loft_rmeta_beside(&rlib).expect("still the paired rmeta");
        assert_eq!(
            std::fs::read(&found).unwrap(),
            b"the-paired-one",
            "recency must not decide this"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// When nothing can be shown to belong to the rlib, answer None: one `--extern`, and
    /// the honest "only metadata stub found" if the rlib really was split.  Passing an
    /// unpaired rmeta instead is what produced `E0464: multiple candidates`, an error that
    /// names the crate and never the staleness.
    #[test]
    fn an_unpaired_rmeta_alone_answers_none() {
        let root = std::env::temp_dir().join(format!("loft_pair_none_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let rlib = layout(&root, false); // uplifted rlib is NOT our deps rlib
        assert!(
            loft_rmeta_beside(&rlib).is_none(),
            "an rmeta that belongs to a different rlib must not be offered"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// The layout where nothing was uplifted: the rmeta sits directly beside the rlib.
    #[test]
    fn a_sibling_rmeta_is_taken_directly() {
        let root = std::env::temp_dir().join(format!("loft_pair_sib_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let rlib = root.join("libloft.rlib");
        std::fs::write(&rlib, b"rlib").unwrap();
        std::fs::write(root.join("libloft.rmeta"), b"sibling").unwrap();
        let found = loft_rmeta_beside(&rlib).expect("the sibling");
        assert_eq!(std::fs::read(&found).unwrap(), b"sibling");
        let _ = std::fs::remove_dir_all(&root);
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
