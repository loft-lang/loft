// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: the interpreter adopting a compiled library artifact

//! The interpreter's side of a compiled library artifact: whether one on disk was built for
//! this program (`artifact_matches_layout`), routing a library's calls to the bodies it
//! carries ([`mark_exports`], [`probe_and_mark_exports`]), and the [`HOST_TABLE`] through
//! which the artifact reaches this interpreter (its coroutines and function values).
//! Building an artifact is the Rust writer's (`native_lib`); the key both use is
//! `opt::native_key`.

use crate::data::Data;
use crate::database::Stores;
#[cfg(feature = "native-extensions")]
use crate::host_abi::LAYOUT_FP_SYMBOL;
use crate::host_abi::{HOST_ABI, HostTable, LibArg};
use crate::keys::DbRef;
use crate::opt::native_key::{
    artifact_sources, auto_cdylib_stem, contributing_fingerprint, fn_numbering_fingerprint,
    library_export_set, mix_fp, platform_cdylib_name, source_content_hash, source_newer_than,
    type_layout_fingerprint,
};
use std::collections::HashSet;

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
    let dups = crate::opt::names::duplicate_fn_names(data);
    for &d in export {
        // The disambiguated ident (#305) — must equal the cdylib wrapper's
        // exported symbol, or `wire_shared_native_fns`' dlsym misses it.
        let sym = format!(
            "loft_shared_{}",
            crate::opt::names::disambiguated_fn_ident(&dups, data.def(d))
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
    /// @PLN199 G0b-2 — the key of a type or function the artifact names and this program
    /// does not have: it is refused, and none of its exports is marked.
    pub refused: Option<String>,
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
    stores: &Stores,
    export: &HashSet<u32>,
    so: &std::path::Path,
) -> BridgeProbe {
    let mut probe = BridgeProbe::default();
    let all = |data: &Data| -> Vec<String> {
        let mut v: Vec<String> = export
            .iter()
            .map(|&d| data.def(d).original_name().clone())
            .collect();
        v.sort_unstable();
        v
    };
    if !crate::c_libs::load_cdylib(&so.to_string_lossy()) {
        // `load_cdylib` has already printed the classified dlopen diagnostic.
        probe.not_loaded = true;
        probe.unresolved = all(data);
        return probe;
    }
    // @PLN199 G0b-2 — an artifact names its types and functions by key; one that names a key
    // this program lacks is never marked, so its functions interpret rather than reach a
    // bridge that cannot run.
    let ids = crate::lib_ids::HostIds::new(data, stores);
    if init_one(so, (&raw const ids).cast()) == Some(0) {
        probe.refused = Some(ids.take_missing().unwrap_or_default());
        probe.unresolved = all(data);
        return probe;
    }
    let dups = crate::opt::names::duplicate_fn_names(data);
    let mut resolvable: HashSet<u32> = HashSet::new();
    for &d in export {
        let sym = format!(
            "loft_shared_{}",
            crate::opt::names::disambiguated_fn_ident(&dups, data.def(d))
        );
        if crate::c_libs::bridge_symbol_resolves(&sym) {
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

/// @PLN199 G0b-2 — hand the library cdylib at `path` this host's table and `ids` (its
/// `loft_shared_init`), and answer what it says: 1 accepted, 0 refused, `None` when it
/// cannot be opened or has no such entry.  Opens it by path — a handle on an image already
/// loaded, which `c_libs::load_cdylib` keeps.
#[cfg(feature = "native-extensions")]
pub(crate) fn init_one(path: &std::path::Path, ids: *const std::ffi::c_void) -> Option<u32> {
    type InitFn = unsafe extern "C" fn(*const HostTable, *const std::ffi::c_void) -> u32;
    let lib = unsafe { crate::c_libs::open_library(&path.to_string_lossy()) }.ok()?;
    let init: libloading::Symbol<InitFn> = unsafe { lib.get(b"loft_shared_init\0") }.ok()?;
    Some(unsafe { init(&raw const HOST_TABLE, ids) })
}

#[cfg(not(feature = "native-extensions"))]
pub(crate) fn init_one(_path: &std::path::Path, _ids: *const std::ffi::c_void) -> Option<u32> {
    None
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
/// so a nested lambda (`__lambda_<fn>_<k>`) is also `pub_visible` — but it is a *fn-ref*
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

/// The table this process hands to every library cdylib it loads.
pub static HOST_TABLE: HostTable = HostTable {
    abi: HOST_ABI,
    coroutine_adopt: host_coroutine_adopt,
    coroutines: HOST_COROUTINES,
    call_ref: host_call_ref,
    type_id: crate::lib_ids::host_type_id,
    fn_id: crate::lib_ids::host_fn_id,
};

/// @PLN199 D2 / D3 — the host's coroutine table as a library cdylib sees it: the runtime's
/// own (`CoroutineHost::HERE`) for a compiled generator, and the interpreter running this
/// thread's bridge call for one of the interpreter's (`COROUTINE_STORE`), which a library is
/// handed only through such a call.
pub(crate) const HOST_COROUTINES: crate::codegen_runtime::CoroutineHost =
    crate::codegen_runtime::CoroutineHost {
        alloc_held: crate::codegen_runtime::CoroutineHost::HERE.alloc_held,
        alloc: crate::codegen_runtime::CoroutineHost::HERE.alloc,
        retain: host_coroutine_retain,
        free: host_coroutine_free,
        next_i64: host_coroutine_next_i64,
        next_into: crate::codegen_runtime::CoroutineHost::HERE.next_into,
        next_dbref: host_coroutine_next_dbref,
        next_text: host_coroutine_next_text,
        exhausted: host_coroutine_exhausted,
    };

/// The interpreter an interpreted generator handle belongs to, or `None` for a compiled one.
pub(crate) fn interpreter_for(gen_ref: DbRef) -> Option<&'static mut crate::state::State> {
    if gen_ref.store_nr != crate::database::COROUTINE_STORE {
        return None;
    }
    let state = crate::extensions::host_state();
    assert!(
        !state.is_null(),
        "an interpreted generator was advanced from compiled code outside a call from the \
         interpreter"
    );
    Some(unsafe { &mut *state })
}

pub(crate) fn host_coroutine_next_i64(gen_ref: DbRef, stores: &mut Stores) -> i64 {
    let Some(st) = interpreter_for(gen_ref) else {
        return (crate::codegen_runtime::CoroutineHost::HERE.next_i64)(gen_ref, stores);
    };
    // A one-byte yield is a boolean's storage byte; anything else an `i64` image.
    let yielded = st.advance_for_native(gen_ref);
    let v = yielded.as_slice();
    match v.len() {
        0 => crate::codegen_runtime::COROUTINE_EXHAUSTED,
        1 => i64::from(v[0]),
        _ => i64::from_ne_bytes(v[..8].try_into().unwrap_or_default()),
    }
}

pub(crate) fn host_coroutine_next_dbref(gen_ref: DbRef, stores: &mut Stores) -> DbRef {
    let Some(st) = interpreter_for(gen_ref) else {
        return (crate::codegen_runtime::CoroutineHost::HERE.next_dbref)(gen_ref, stores);
    };
    let yielded = st.advance_for_native(gen_ref);
    let v = yielded.as_slice();
    if v.len() < std::mem::size_of::<DbRef>() {
        DbRef::NULL
    } else {
        unsafe { std::ptr::read_unaligned(v.as_ptr().cast::<DbRef>()) }
    }
}

pub(crate) fn host_coroutine_next_text(gen_ref: DbRef, stores: &mut Stores) -> String {
    let Some(st) = interpreter_for(gen_ref) else {
        return (crate::codegen_runtime::CoroutineHost::HERE.next_text)(gen_ref, stores);
    };
    // The interpreter yields a `Str` into its own memory; the compiled consumer owns a copy.
    let yielded = st.advance_for_native(gen_ref);
    let v = yielded.as_slice();
    if v.len() < std::mem::size_of::<crate::keys::Str>() {
        return crate::store::STRING_NULL.to_string();
    }
    let s = unsafe { std::ptr::read_unaligned(v.as_ptr().cast::<crate::keys::Str>()) };
    s.str().to_string()
}

pub(crate) fn host_coroutine_exhausted(gen_ref: DbRef) -> bool {
    match interpreter_for(gen_ref) {
        Some(st) => st.coroutine_exhausted(&gen_ref),
        None => (crate::codegen_runtime::CoroutineHost::HERE.exhausted)(gen_ref),
    }
}

pub(crate) fn host_coroutine_retain(gen_ref: DbRef) -> DbRef {
    match interpreter_for(gen_ref) {
        Some(st) => st.coroutine_retain(gen_ref),
        None => (crate::codegen_runtime::CoroutineHost::HERE.retain)(gen_ref),
    }
}

pub(crate) fn host_coroutine_free(gen_ref: DbRef, stores: &mut Stores) {
    match interpreter_for(gen_ref) {
        Some(st) => st.free_coroutine(&gen_ref),
        None => (crate::codegen_runtime::CoroutineHost::HERE.free)(gen_ref, stores),
    }
}

/// [`HostTable::call_ref`] — the interpreter is the one `State::invoke_native` recorded for
/// this thread's bridge call, or, on a compiled library's own worker thread, one made over
/// that worker's stores (`parallel::with_worker_state`).
pub(crate) unsafe extern "C" fn host_call_ref(
    stores: *mut Stores,
    d_nr: u32,
    closure: *const LibArg,
    args: *const LibArg,
    n: usize,
    ret: *mut LibArg,
) {
    let args = unsafe { std::slice::from_raw_parts(args, n) };
    let closure = unsafe { (*closure).dbref };
    let state = crate::extensions::host_state();
    let result = if state.is_null() {
        // A compiled library's own `par` worker: no interpreter runs on this thread, so one
        // is made over the worker's stores for the call.
        crate::parallel::with_worker_state(unsafe { &mut *stores }, |st| {
            st.call_back(d_nr, closure, args)
        })
    } else {
        unsafe { (*state).call_back(d_nr, closure, args) }
    };
    unsafe { *ret = result };
}

/// [`HostTable::coroutine_adopt`] — the generator's frame takes holds on its argument stores
/// in the host's table (`@FR-G-Arg-Rebind`), exactly as a `--native` caller's
/// `alloc_coroutine_held` does in its own.  The box was allocated in the cdylib by the same
/// system allocator this process frees with.
pub(crate) unsafe extern "C" fn host_coroutine_adopt(
    stores: *mut Stores,
    coro: *mut std::ffi::c_void,
    ret: *mut LibArg,
) {
    let coro: Box<Box<dyn crate::codegen_runtime::LoftCoroutine>> =
        unsafe { Box::from_raw(coro.cast()) };
    let cell = unsafe { &*(stores.cast::<std::cell::UnsafeCell<Stores>>()) };
    let handle = crate::codegen_runtime::alloc_coroutine_held(cell, *coro);
    unsafe { (*ret).dbref = handle };
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
pub(crate) fn layout_fp_of_relocated(so: &std::path::Path) -> LayoutProbe {
    unsafe {
        let Ok(lib) = crate::file_access::load_library(so) else {
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
pub(crate) fn layout_fp_of_relocated(_so: &std::path::Path) -> LayoutProbe {
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
pub(crate) enum LayoutProbe {
    /// Could not be opened — torn, half-written, or deleted between the existence
    /// check and the load.  Never adopt; rebuilding is always available.
    Unopenable,
    /// Opened, and names no layout: a hand-written cdylib, generated against no
    /// type table and with nothing to declare.
    Undeclared,
    /// Opened, and names the layout it was generated for.
    Declares(u64),
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
/// The value compared is the key the filename carries (`cached_or_build_shared_cdylib`),
/// so a file that reached the name another way — copied, or built for another program by a
/// literal build — is refused for what it is, not for what its name says.
///
/// The question is always asked through [`layout_fp_off_path`], never at `so`
/// itself, because a caller that hears "no" REBUILDS at that very path — see
/// there for what a load of the real path costs on macOS.  There is deliberately
/// no second entry point that probes in place: which variant a call site picked
/// used to be the difference between a correct answer and a silently stale one
/// (loft#999).
pub(crate) fn artifact_matches_layout(so: &std::path::Path, layout_fp: u64) -> bool {
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
pub(crate) fn layout_fp_off_path(so: &std::path::Path) -> LayoutProbe {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let (Some(dir), Some(name)) = (so.parent(), crate::file_access::file_name(so)) else {
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
    if let Err(e) = crate::file_access::create_dir_all(&probe_dir) {
        report_unprobeable(so, &format!("create {e}"));
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
    let _ = crate::file_access::remove_dir_all(&probe_dir);
    fp
}

/// Put the artifact at `so` under the throwaway name `probe`: a hard link, or a
/// byte copy where linking is not available (a filesystem without links, or a
/// `probe` that somehow lands on another device).
///
/// `LOFT_FORCE_PROBE_RELOCATE_FAIL` makes both legs fail, which is how the tests
/// reach the answer this used to get wrong on macOS alone.
pub(crate) fn relocate_for_probe(
    so: &std::path::Path,
    probe: &std::path::Path,
) -> Result<(), String> {
    if std::env::var_os("LOFT_FORCE_PROBE_RELOCATE_FAIL").is_some() {
        return Err("LOFT_FORCE_PROBE_RELOCATE_FAIL".to_string());
    }
    if crate::file_access::hard_link(so, probe).is_ok() {
        return Ok(());
    }
    crate::file_access::copy(so, probe)
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
pub(crate) fn report_unprobeable(so: &std::path::Path, why: &str) {
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

/// What a run does with a library's compiled artifact.
pub enum Adopt {
    /// An artifact built for this program and this loft: dispatch to it.
    Fresh(std::path::PathBuf),
    /// The library is being edited: interpret it this run, and build nothing.
    Interpret,
    /// No artifact this program can adopt: build one under this key.
    Build(ArtifactKey),
}

/// Where a library's artifact is to be built and under which name.  It holds the package's
/// build lock, so the build that follows is serialised with every other process's.
pub struct ArtifactKey {
    /// The package's `native-auto/` directory.
    pub out_dir: std::path::PathBuf,
    /// The artifact's file stem, which carries `fp`.
    pub stem: String,
    /// The fingerprint the artifact is built for and records.
    pub fp: u64,
    /// The build lock, released when the key drops.
    _lock: crate::file_access::File,
}

/// The adopting half of `native_artifact::cached_or_build_shared_cdylib`: the artifact's key, whether one on
/// disk serves this run, and — under the build lock — whether to build one or interpret.
///
/// # Errors
/// When the package's `native-auto/` directory or its build lock cannot be made.
pub fn adopt_shared_cdylib(
    data: &Data,
    stores: &Stores,
    export_set: &HashSet<u32>,
    pkg_dir: &str,
    pending: &[String],
) -> Result<Adopt, String> {
    let out_dir = std::path::Path::new(pkg_dir).join("native-auto");
    let contributing = &artifact_sources(data, pkg_dir, pending, export_set)[..];
    // #461 — type-table INDICES shift with which libraries are loaded (an `i32` write is
    // `db_tp=64` standalone, `67` once `hex_grid` is parsed), and a cdylib that hardcoded
    // them, reused by a consumer whose table differs, read the WRONG type and corrupted
    // (the moros GLB header wrote 8-byte fields for `as i32` → version 0).  A literal build
    // still folds the caller's layout into its key for that reason.
    let layout_fp = type_layout_fingerprint(stores);
    // @PLN199 G0c — an artifact that names its ids by key (`lib_ids`) runs in any program
    // that has those keys, so neither the program's type table nor its function numbering is
    // part of what it is built for: the loft that built it and the sources it carries are.
    // A literal build (`LOFT_NO_LIB_IDS=1`) still serves one program, and keeps both.
    let program_fp = if std::env::var_os("LOFT_NO_LIB_IDS").is_some() {
        mix_fp(layout_fp, fn_numbering_fingerprint(data, export_set))
    } else {
        0
    };
    // And WHICH packages were compiled in.  The artifact carries the code of every library
    // it reaches, a dependency's included, so two runs that resolved `use probepkg` to
    // different versions build different libraries — with the SAME layout when the versions
    // declare the same types.  Keyed on layout alone, the second run adopted the first
    // one's artifact and ran the other version's code without a word (a registry package's
    // `native-auto/` is shared by every consumer on the box).  `contributing` is the set
    // `source_newer_than` already reads as this artifact's sources, so one set answers both
    // "is it current" and "is it this build"; a version is a different directory.
    // …and WHICH loft executable: the artifact is this binary's generated code, called through
    // this binary's store layout, so a different binary must rebuild rather than adopt it —
    // the rlib's hash alone stays the same when an installed `loft` is upgraded without it
    // (loft#1776).  A release bundle's binary names its build by itself: the release builds
    // its artifacts beside an rlib that ships nowhere, and a user's copy has none to hash.
    let loft_fp = if crate::cache::release_bundle() {
        crate::cache::loft_exe_identity()
    } else {
        mix_fp(
            crate::cache::loft_build_fingerprint(),
            crate::cache::loft_exe_identity(),
        )
    };
    let fp = mix_fp(
        mix_fp(loft_fp, program_fp),
        mix_fp(
            contributing_fingerprint(contributing),
            crate::cache::native_release_flags_fingerprint(),
        ),
    );
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
    // The probe OPENS the artifact, so whatever it links must be loadable first.
    crate::opt::native_key::load_linked_packages(data, export_set);
    if crate::file_access::exists(&so)
        && !source_newer_than(contributing, &so)
        && artifact_matches_layout(&so, fp)
    {
        return Ok(Adopt::Fresh(so));
    }

    // Concurrent `loft` processes (parallel tests are the common case)
    // routinely load the same library at once; an unserialized double-build
    // rewrites the generated `.rs` under a running rustc and tears the `.so`
    // (mixed-object "dangerous relocation" link errors, "file too short" on
    // load).  Take an exclusive advisory lock for the check+build, then
    // RE-CHECK freshness: the waiter adopts the winner's artifact instead of
    // rebuilding over it.  Released when `lock` drops, on every return path.
    crate::file_access::create_dir_all(&out_dir).map_err(|e| format!("create {e}"))?;
    let lock_path = out_dir.join(".build.lock");
    let lock = crate::file_access::create(&lock_path).map_err(|e| format!("create {e}"))?;
    lock.lock()
        .map_err(|e| format!("lock {}: {e}", lock_path.display()))?;
    if crate::file_access::exists(&so)
        && !source_newer_than(contributing, &so)
        && artifact_matches_layout(&so, fp)
    {
        return Ok(Adopt::Fresh(so));
    }
    let build = |out_dir: std::path::PathBuf| {
        Ok(Adopt::Build(ArtifactKey {
            out_dir,
            stem: stem.clone(),
            fp,
            _lock: lock,
        }))
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
    if !crate::file_access::exists(&so) || !artifact_matches_layout(&so, fp) {
        crate::cache::write_run_source_hash(&out_dir, source_content_hash(contributing));
        return build(out_dir);
    }

    // 3. Stale artifact (the library was edited) → dev-interpret-on-edit.
    let cur = source_content_hash(contributing);
    let stable = crate::cache::read_run_source_hash(&out_dir) == Some(cur);
    crate::cache::write_run_source_hash(&out_dir, cur);
    if stable {
        build(out_dir) // editing settled → rebuild, native again
    } else {
        Ok(Adopt::Interpret) // still being edited → interpret this run, no rustc
    }
}
