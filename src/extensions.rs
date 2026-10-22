// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader

//! Native extension loader.
//!
//! Package native crates (cdylib) export `loft_register_v1`, a C-ABI function
//! that registers all native symbols with the interpreter via a callback.
//! Only C primitives cross the boundary — no Rust types are shared.
//!
//! See `EXTERNAL_LIBS.md` for the full design.

use crate::platform::process::{Program, Spawn};
// The opened-library table and symbol lookup live in `c_libs` (core); these are the paths
// callers name here, and what the wiring below reads.
use crate::c_libs::adopt_foreign_block;
pub use crate::c_libs::{CTX_RETURN_STORE, bridge_symbol_resolves, load_cdylib};
#[cfg(feature = "native-extensions")]
use crate::c_libs::{FnPtr, NATIVE_REGISTRY, get_bridge, load_one, try_dlsym};
#[cfg(feature = "native-extensions")]
pub use crate::c_libs::{load_c_library, try_dlsym_pub};
// Finding or building a package's native library lives in `native_build` (core),
// because the parser resolves it on `use`; these are the paths callers name here.
pub use crate::native_build::{
    auto_build_native, browser_target, host_native_lib, native_target_root, platform_lib_name,
    resolve_native_lib, set_browser_target,
};
use crate::native_build::{build_deps_hint, native_crate_newer_than};
/// Load all pending native extension libraries.
#[cfg(feature = "native-extensions")]
use std::collections::HashMap;
// Every registry this guards is itself `native-extensions`-only, so the import follows the
// same gate — without it a `--no-default-features` build warns on an unused import, which is
// noise in the one gate whose whole job is to be read.
#[cfg(feature = "native-extensions")]
use std::sync::Mutex;

#[cfg(feature = "native-extensions")]
pub fn load_all(_state: &mut crate::state::State, paths: Vec<String>) {
    for path in paths {
        load_one(&path);
    }
}

#[cfg(not(feature = "native-extensions"))]
pub fn load_all(_state: &mut crate::state::State, _paths: Vec<String>) {}

#[cfg(all(test, feature = "native-extensions"))]
mod c_lib_naming_tests {
    use crate::platform::{LibOs, lib_variants};

    /// The declared spelling is authoritative: it is always first, on every
    /// host, so a translation can only ever ADD a fallback.
    #[test]
    fn the_declared_name_is_always_tried_first() {
        for os in [LibOs::Linux, LibOs::Macos, LibOs::Windows] {
            assert_eq!(lib_variants("libmariadb.so.3", os)[0], "libmariadb.so.3");
        }
        assert_eq!(
            lib_variants("libmariadb.so.3", LibOs::Linux).len(),
            1,
            "Linux needs no fallback — the declared spelling is already its own"
        );
    }

    /// The soversion moves BEFORE the extension on macOS: `libmariadb.so.3` is
    /// `libmariadb.3.dylib`, not `libmariadb.so.3.dylib`.
    #[test]
    fn a_versioned_soname_becomes_the_macos_spelling() {
        assert_eq!(
            lib_variants("libmariadb.so.3", LibOs::Macos),
            ["libmariadb.so.3", "libmariadb.3.dylib", "libmariadb.dylib"],
            "versioned first, then the unversioned fallback"
        );
        assert_eq!(
            lib_variants("libsqlite3.so.0", LibOs::Macos)[1],
            "libsqlite3.0.dylib"
        );
    }

    /// The fixture's own shape — a RELATIVE path, which must stay relative to
    /// the same directory (this is what the macOS daily CI caught).
    #[test]
    fn a_relative_path_keeps_its_directory() {
        assert_eq!(
            lib_variants("../../liblc_types.so", LibOs::Macos),
            ["../../liblc_types.so", "../../liblc_types.dylib"]
        );
        assert!(
            lib_variants("../../liblc_types.so", LibOs::Windows)
                .contains(&"../../lc_types.dll".to_string())
        );
    }

    /// A DLL carries neither the `lib` prefix nor the soversion, but a
    /// MinGW-built one keeps the prefix — so both spellings are tried.
    #[test]
    fn a_windows_name_drops_the_prefix_and_the_version() {
        assert_eq!(
            lib_variants("libmariadb.so.3", LibOs::Windows),
            ["libmariadb.so.3", "mariadb.dll", "libmariadb.dll"]
        );
    }

    /// A name that is not a Linux spelling is passed through untouched —
    /// translating `sqlite3.dll` or a bare soname would invent a library.
    #[test]
    fn a_non_linux_spelling_is_left_alone() {
        for n in ["sqlite3.dll", "libfoo.dylib", "foo"] {
            assert_eq!(lib_variants(n, LibOs::Macos), [n.to_string()], "{n}");
            assert_eq!(lib_variants(n, LibOs::Windows), [n.to_string()], "{n}");
        }
    }

    /// A library file that EXISTS but cannot be mapped answers false.
    ///
    /// This is what makes the host-spelling fallback reachable at all. The
    /// candidate loop used to accept a path on `exists()` alone, so on macOS a
    /// `liblc_types.so` left behind by a Linux build — the fixture's `.so` is
    /// git-ignored, but any dev tree that ran `make` on Linux has one — would
    /// be taken as loaded and the `.dylib` beside it never tried. The symptom
    /// then names the missing SYMBOL, pointing at the binding rather than at
    /// the library that never opened.
    #[test]
    fn a_file_that_cannot_be_mapped_is_not_reported_as_loaded() {
        let dir = std::env::temp_dir().join(format!("loft_cload_{}", std::process::id()));
        crate::file_access::create_dir_all(&dir).expect("probe dir");
        let bogus = dir.join("libnotanelf.so");
        crate::file_access::write(&bogus, b"this is not a shared object").expect("write probe");
        assert!(
            !super::load_c_library("libnotanelf.so", &dir.to_string_lossy()),
            "a path that exists but will not dlopen must not count as loaded"
        );
        let _ = crate::file_access::remove_dir_all(&dir);
    }
}

// ── Auto-marshal: wire cdylib functions via type-driven dispatch ────────

/// Argument type tag for auto-marshalling.
#[cfg(feature = "native-extensions")]
#[derive(Clone, Copy, Debug, PartialEq)]
enum ArgT {
    I32,
    I64,
    F32,
    F64,
    Bool,
    Text,
    Ref, // DbRef — struct reference (rec/pos point to the struct)
    Vec, // DbRef — vector reference (indirect: dereference rec/pos to get data record)
    /// @PLN199 C1 — a function value: the interpreter's 20-byte slot (`d_nr` as `i64`, then
    /// the closure `DbRef`), carried in one `LibArg` as `scalar` + `dbref`.
    FnRef,
}

/// Mirror of `loft_ffi::LoftStoreCtx` — `#[repr(C)]` so layout matches.
#[cfg(feature = "native-extensions")]
#[repr(C)]
#[derive(Clone, Copy)]
struct LoftStoreCtx {
    _opaque: *mut (),
}

/// Mirror of `loft_ffi::LoftStore` — `#[repr(C)]` so layout matches.
#[cfg(feature = "native-extensions")]
#[repr(C)]
#[derive(Clone, Copy)]
struct LoftStore {
    ptr: *mut u8,
    size: u32,
    ctx: LoftStoreCtx,
    claim_fn: Option<unsafe extern "C" fn(LoftStoreCtx, u32) -> u32>,
    reload_fn: Option<unsafe extern "C" fn(LoftStoreCtx, *mut *mut u8, *mut u32)>,
    resize_fn: Option<unsafe extern "C" fn(LoftStoreCtx, u32, u32) -> u32>,
    foreign_fn: Option<
        unsafe extern "C" fn(
            LoftStoreCtx,
            *mut u8,
            usize,
            usize,
            u32,
            unsafe extern "C" fn(*mut u8, usize, usize),
        ) -> loft_ffi::LoftRef,
    >,
}

/// Compact native signature: parameter types + return type.
#[cfg(feature = "native-extensions")]
#[derive(Clone, Debug)]
struct NativeSig {
    params: Vec<ArgT>,
    ret: Option<ArgT>,
}

/// Side table: library index → (the generated bridge, signature), resolved ONCE by
/// `wire_native_fns`.  The dispatcher reads it through [`cached_sig`], never per call.
#[cfg(feature = "native-extensions")]
type NativeSlot = (FnPtr, std::sync::Arc<NativeSig>);
#[cfg(feature = "native-extensions")]
static NATIVE_SIGS: Mutex<Option<HashMap<u16, NativeSlot>>> = Mutex::new(None);

/// Bumped by every wiring pass that writes `NATIVE_SIGS` or `SHARED_SIGS`, so a thread's
/// copy of a slot ([`cached_sig`]) is refetched after a rewire.
#[cfg(feature = "native-extensions")]
static SIGS_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A thread's copy of a wired-native table, indexed by library index: the generation it was
/// filled under, and the slots it has met.
#[cfg(feature = "native-extensions")]
struct SigCache<V> {
    generation: u64,
    slots: Vec<Option<V>>,
}

#[cfg(feature = "native-extensions")]
thread_local! {
    static NATIVE_SIG_CACHE: std::cell::RefCell<SigCache<NativeSlot>> =
        const { std::cell::RefCell::new(SigCache { generation: u64::MAX, slots: Vec::new() }) };
    static SHARED_SIG_CACHE: std::cell::RefCell<SigCache<SharedSlot>> =
        const { std::cell::RefCell::new(SigCache { generation: u64::MAX, slots: Vec::new() }) };
}

/// The wired slot for `lib_idx`: from this thread's copy, or — on a thread's first call to
/// it, or after a rewire — from the shared table under its lock.  A native call reads its
/// slot here once per call, so the per-call cost is an index, not a lock, a hash of the
/// symbol and a clone of its signature (the lock is never held across the call itself,
/// P245: a blocking native would serialise every `par` worker behind it).
#[cfg(feature = "native-extensions")]
fn cached_sig<V: Clone>(
    cache: &'static std::thread::LocalKey<std::cell::RefCell<SigCache<V>>>,
    table: &Mutex<Option<HashMap<u16, V>>>,
    lib_idx: u16,
) -> Option<V> {
    let generation = SIGS_GENERATION.load(std::sync::atomic::Ordering::Acquire);
    cache.with(|c| {
        let mut c = c.borrow_mut();
        if c.generation != generation {
            c.slots.clear();
            c.generation = generation;
        }
        let at = usize::from(lib_idx);
        if let Some(Some(v)) = c.slots.get(at) {
            return Some(v.clone());
        }
        let v = table
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .as_ref()?
            .get(&lib_idx)?
            .clone();
        if c.slots.len() <= at {
            c.slots.resize(at + 1, None);
        }
        c.slots[at] = Some(v.clone());
        Some(v)
    })
}

/// #303 — wire-time signature of a shared-store bridge function, derived from
/// the ONE marshallability judgment (`native_gate::classify_bridge_attr`).
/// The interpreter call site pushes EVERY attribute, so `pops` has one entry
/// per attribute (declaration order) and drives the dispatcher's stack pops;
/// the generated bridge reads only the marshalled subset (compacted order), so
/// `forward` marks which popped slots become `LibArg`s.  `text_workbuf` is the
/// `pops` index of the `text_return` work buffer: in a non-dest call
/// (expression context) the dispatcher reuses that caller-cleared cell as
/// `bridge_text_dest` and returns a `Str` over it.
#[cfg(feature = "native-extensions")]
#[derive(Clone)]
struct SharedSig {
    pops: Vec<ArgT>,
    forward: Vec<bool>,
    text_workbuf: Option<usize>,
    /// The `pops` index of the hidden `ref_return` destination (the retbuf the
    /// caller pre-allocates and forwards).  Used by the `LOFT_TRACE_SHARED_RET`
    /// attribution instrument to check the retbuf-vs-return orphan (@PLN118 arc F).
    hidden_dest: Option<usize>,
    ret: Option<ArgT>,
}

/// @PLN11 Arc N — side table for the **shared-store** bridge
/// (`native_lib::generate_shared_cdylib_lib_rs`): library index → (bridge fn ptr,
/// signature).  Populated by `wire_shared_native_fns`, read by
/// `shared_store_dispatch`.  Separate from `NATIVE_SIGS` because the ABI differs
/// (a `*mut Stores` + `LibArg` bridge, not the `LoftStore`/raw-ptr marshalling).
#[cfg(feature = "native-extensions")]
type SharedSlot = (FnPtr, std::sync::Arc<SharedSig>);
#[cfg(feature = "native-extensions")]
static SHARED_SIGS: Mutex<Option<HashMap<u16, SharedSlot>>> = Mutex::new(None);

/// Compute the argument type list and return type from a definition's signature.
/// Returns `None` if the signature contains types that can't be auto-marshalled
/// (e.g. struct references, vectors).
#[cfg(feature = "native-extensions")]
fn compute_sig(data: &crate::data::Data, d_nr: u32) -> Option<NativeSig> {
    use crate::data::Type;
    let def = data.def(d_nr);
    let mut params = Vec::new();
    for attr in &def.attributes {
        // Marshal classification is layout-based, and `Optional(τ)` shares τ's
        // sentinel layout (@PLN25) — classify the peeled type, here and for the
        // return below.  Without the peel, a `#native` fn declaring `integer?`
        // silently classified as unmarshallable and was never wired (the call
        // then hit the stale-cdylib panic stub).
        let t = match attr.typedef.base() {
            // @P370: a plain loft `integer` is 64-bit (8-byte slot) — it must
            // marshal as I64.  Only an EXPLICIT narrow integer (`u8/i8/u16/i16/
            // i32`, which carry `forced_size`) or a `Character` (4-byte
            // codepoint) is ≤4 bytes → I32.  Auto-converting `integer` to i32
            // truncated i64 values (e.g. an `i64::MIN` null sentinel → 0) and
            // diverged from `--native` (which uses the lib's real i64 ABI).
            Type::Integer(s) if s.forced_size.is_none() => ArgT::I64,
            Type::Integer(_) | Type::Character => ArgT::I32,
            Type::Float => ArgT::F64,
            Type::Single => ArgT::F32,
            Type::Boolean => ArgT::Bool,
            Type::Text(_) => ArgT::Text,
            Type::Enum(_, false, _) => ArgT::I32, // simple enum tag
            Type::Reference(_, _)
            | Type::Enum(_, true, _)
            | Type::Sorted(_, _, _)
            | Type::Index(_, _, _)
            | Type::Hash(_, _, _)
            | Type::Radix(_, _, _)
            | Type::Trie(_, _, _) => ArgT::Ref,
            Type::Vector(_, _) => ArgT::Vec,
            _ => return None,
        };
        params.push(t);
    }
    let ret = match def.returned.base() {
        Type::Void | Type::Null => None,
        // @P370: plain loft `integer` is 64-bit → I64; only explicit narrow
        // ints (`forced_size`) and `Character` are ≤4 bytes → I32.
        Type::Integer(s) if s.forced_size.is_none() => Some(ArgT::I64),
        Type::Integer(_) | Type::Character => Some(ArgT::I32),
        Type::Float => Some(ArgT::F64),
        Type::Single => Some(ArgT::F32),
        Type::Boolean => Some(ArgT::Bool),
        Type::Text(_) => Some(ArgT::Text),
        Type::Enum(_, false, _) => Some(ArgT::I32),
        Type::Reference(_, _)
        | Type::Enum(_, true, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _)
        | Type::Trie(_, _, _) => Some(ArgT::Ref),
        Type::Vector(_, _) => Some(ArgT::Vec),
        _ => return None,
    };
    Some(NativeSig { params, ret })
}

/// The `ArgT` mapping for one marshalled loft type (shared-bridge param or
/// return).  `None` = not marshallable.
#[cfg(feature = "native-extensions")]
fn marshal_arg_t(t: &crate::data::Type) -> Option<ArgT> {
    use crate::data::Type;
    // `Optional(τ)` rides τ's sentinel layout (@PLN25) — marshal as the base type.
    Some(match t.base() {
        // @P370: plain loft `integer` is 64-bit → I64; only explicit narrow
        // ints (`forced_size`) and `Character` are ≤4 bytes → I32.
        Type::Integer(s) if s.forced_size.is_none() => ArgT::I64,
        Type::Integer(_) | Type::Character => ArgT::I32,
        Type::Float => ArgT::F64,
        Type::Single => ArgT::F32,
        Type::Boolean => ArgT::Bool,
        Type::Text(_) => ArgT::Text,
        Type::Enum(_, false, _) => ArgT::I32,
        Type::Reference(_, _)
        | Type::Enum(_, true, _)
        | Type::Sorted(_, _, _)
        | Type::Index(_, _, _)
        | Type::Hash(_, _, _)
        | Type::Radix(_, _, _)
        | Type::Trie(_, _, _) => ArgT::Ref,
        Type::Vector(_, _) => ArgT::Vec,
        // @PLN199 A4 — a `&T` argument is the `DbRef` of the slot it names.
        Type::RefVar(_) => ArgT::Ref,
        // @PLN199 D1 — a generator crosses as its handle in the host's coroutine table.
        Type::Iterator(_, _) => ArgT::Ref,
        // @PLN199 C1 — a function value.
        Type::Function(..) => ArgT::FnRef,
        _ => return None,
    })
}

/// The leaves of a tuple type, depth first in element order; any other type is its own leaf.
#[cfg(feature = "native-extensions")]
fn tuple_leaves(t: &crate::data::Type) -> Vec<&crate::data::Type> {
    if let crate::data::Type::Tuple(elems) = t.base() {
        elems.iter().flat_map(tuple_leaves).collect()
    } else {
        vec![t]
    }
}

/// #303 — build a [`SharedSig`] from a definition, via the ONE marshallability
/// judgment (`native_gate::classify_bridge_attr`).  Returns `None` exactly when
/// the gate would not have marked the function — so a marked function always
/// wires (the divergence that left panicking stubs behind emitted dispatches).
#[cfg(feature = "native-extensions")]
fn compute_shared_sig(data: &crate::data::Data, d_nr: u32) -> Option<SharedSig> {
    use crate::data::Type;
    use crate::native_gate::{BridgeAttrKind, classify_bridge_attr};
    let def = data.def(d_nr);
    let ret_text = matches!(def.returned().base(), Type::Text(_));
    let mut pops = Vec::new();
    let mut forward = Vec::new();
    let mut text_workbuf = None;
    let mut hidden_dest = None;
    for attr in &def.attributes {
        match classify_bridge_attr(attr, ret_text)? {
            BridgeAttrKind::Marshal => {
                // @PLN199 A3 — a tuple argument is its elements, contiguous on the stack in
                // element order: one pop per leaf, which the reverse pop below reads in place.
                for leaf in tuple_leaves(&attr.typedef) {
                    pops.push(marshal_arg_t(leaf)?);
                    forward.push(true);
                }
            }
            BridgeAttrKind::WorkText => {
                // The call site pushes the work buffer's `DbRef` (a CreateStack
                // cell) — popped like any ref, never forwarded.
                if text_workbuf.is_none() {
                    text_workbuf = Some(pops.len());
                }
                pops.push(ArgT::Ref);
                forward.push(false);
            }
            BridgeAttrKind::HiddenDest => {
                // The call site pushes the caller-allocated destination record —
                // forward it so the bridge writes the result THERE (the record
                // the caller's frame owns and frees).  A bridge-local allocation
                // instead orphaned the caller's copy: one leaked store per
                // vector-returning call (#311).  The wrapper still allocates as
                // a fallback when no slot arrives (a no-body `#native` decl
                // caller has no hidden attrs) or the incoming ref is null.
                if hidden_dest.is_none() {
                    hidden_dest = Some(pops.len());
                }
                pops.push(ArgT::Vec);
                forward.push(true);
            }
        }
    }
    let ret = match def.returned().base() {
        Type::Void | Type::Null => None,
        t => Some(marshal_arg_t(t)?),
    };
    Some(SharedSig {
        pops,
        forward,
        text_workbuf,
        hidden_dest,
        ret,
    })
}

/// After `load_all()` has populated `NATIVE_REGISTRY`, iterate all `#native`
/// definitions and replace the panic stubs with auto-marshalled wrappers.
///
/// For symbols not found in the registry (i.e. the cdylib didn't use
/// `loft_register_v1`), falls back to direct `dlsym` lookup — enabling
/// zero-registration cdylibs that just export `extern "C" fn n_*()`.
///
/// Functions already registered by `native::init()` are skipped — their
/// stubs were never created by `register_native_stubs`.
///
/// # Panics
/// Panics if a symbol is found via dlsym but the library used `loft_register_v1`
/// (indicating a registration bug).
#[cfg(feature = "native-extensions")]
pub fn wire_native_fns(state: &mut crate::state::State, data: &crate::data::Data) {
    wire_native_fns_for(state, data, true);
}

/// [`wire_native_fns`], saying whether an unloaded native library is worth REPORTING.
/// It is not for a browser build (`--html`): the page provides its own imports (the
/// GL and web host functions) and the host's cdylibs play no part in what ships, so
/// "did not load" there names nothing the author can act on (loft#1830's stderr).
///
/// # Panics
/// As [`wire_native_fns`]: if a symbol is found via dlsym but the library used
/// `loft_register_v1` (a registration bug).
#[cfg(feature = "native-extensions")]
pub fn wire_native_fns_for(
    state: &mut crate::state::State,
    data: &crate::data::Data,
    report_unresolved: bool,
) {
    // THIS program's stub set (see `State::native_stub_symbols`) — cloned because
    // `state` is mutated below while wiring.  Never a process-global: a global was
    // overwritten by whichever compile ran last, so in a process compiling more than
    // one program the wiring consulted a SIBLING's set and skipped its own symbols.
    let stub_syms = state.native_stub_symbols.clone();

    // Phase 1: resolve any missing symbols via dlsym.
    {
        let reg_guard = NATIVE_REGISTRY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);

        let mut to_resolve: Vec<String> = Vec::new();
        for d_nr in 0..data.definitions() {
            let def = data.def(d_nr);
            if def.native.is_empty() {
                continue;
            }
            let sym = &def.native;
            // @PLN11 Arc N: shared-store bridges are wired by
            // `wire_shared_native_fns` (a different ABI) — skip them here.
            if sym.starts_with("loft_shared_") {
                continue;
            }
            if !stub_syms.contains(sym) {
                continue;
            }
            let found = reg_guard.as_ref().is_some_and(|r| r.contains_key(sym));
            if !found {
                to_resolve.push(sym.clone());
            }
        }
        drop(reg_guard);

        // Resolve via dlsym (no locks held).  The guard is now keyed on the
        // *resolving library's* own `uses_v1` flag (returned by `try_dlsym`), not
        // a global "registry non-empty" proxy — so a zero-registration cdylib
        // loaded after one that used the protocol no longer false-positives, while
        // issue #119 (a v1 library's unregistered-but-dlsym-found symbol) still
        // panics.
        for sym in to_resolve {
            if let Some((ptr, lib_uses_v1)) = try_dlsym(&sym) {
                // The resolving library used loft_register_v1 but didn't register
                // this symbol — this is a registration bug.
                assert!(
                    !lib_uses_v1,
                    "native symbol '{sym}' was not registered via loft_register_v1 \
                     but was found via dlsym. This is a registration bug — \
                     add reg!(b\"{sym}\", <fn>) to loft_register_v1.",
                );
                let mut rg = NATIVE_REGISTRY
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                rg.get_or_insert_with(HashMap::new).insert(sym, FnPtr(ptr));
            }
        }
    }

    // Phase 2: wire auto-marshalled dispatchers.
    let guard = NATIVE_REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    // A `None` registry means NO native cdylib loaded at all — the worst case, not a
    // skip case: if a library's cdylib failed to load (missing / stale / rebuild
    // failed) and it was the only one, every one of its stubs is unresolved.  Treat
    // None as an empty registry so the loop still runs and reports them, instead of
    // returning early and leaving the failure silent until a mid-run panic.
    let empty_registry = HashMap::new();
    let registry = guard.as_ref().unwrap_or(&empty_registry);

    let mut sigs = NATIVE_SIGS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let sig_table = sigs.get_or_insert_with(HashMap::new);

    // Stub symbols whose cdylib never provided them — collected so the failure is
    // reported LOUDLY at load (below), not left to surface as a generic panic at
    // first call deep in execution.
    let mut unresolved: Vec<String> = Vec::new();

    // Symbols the cdylib DOES export but registered no `#[loft_native]` bridge for
    // (loft#886). A different failure from `unresolved` with a different fix, so it
    // gets its own list and its own message.
    let mut bridgeless: Vec<String> = Vec::new();

    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        if def.native.is_empty() {
            continue;
        }
        let sym = &def.native;

        // @PLN11 Arc N: shared-store bridges use a different dispatcher — skip.
        if sym.starts_with("loft_shared_") {
            continue;
        }
        // @PLN119 arc A: a process-placed library's symbol is served by a worker
        // over the placement wire, not by a cdylib in this process. It has
        // already been wired by `lib_placement::dispatch::install`, so looking
        // for it here would find nothing and report a missing cdylib that was
        // never supposed to exist.
        if sym.starts_with("loft_placed_") {
            continue;
        }

        // Only replace stubs — skip hand-written glue from native::init().
        if !stub_syms.contains(sym) {
            continue;
        }

        if !registry.contains_key(sym) {
            // #453 — a `[wasm.bridge].routes` `#native` symbol is implemented by
            // the bridge crate (the `--html` target), not a native cdylib. Its
            // absence from the cdylib registry is by design, not a missing/stale
            // build, so it is NOT an unresolved native — reporting it (and telling
            // the user to "rebuild the cdylib") is wrong. (Run in the interpreter
            // it is genuinely unavailable, but that is a "use --html" matter.)
            if data.wasm_bridge_routes.contains_key(sym) || dest_served(state, data, d_nr) {
                continue;
            }
            // Neither the registry nor `try_dlsym` (phase 1) found it: the owning
            // library's cdylib is missing / stale / failed to rebuild.  The panic
            // stub stays in place; record the symbol so load-time reporting names it.
            unresolved.push(sym.clone());
            continue;
        }

        // loft#886 — the cdylib loaded and exports the symbol, but registered no
        // marshal bridge for it, so the call has nothing to dispatch through. That
        // is a REGISTRATION gap in the library, not a stale build, and until now it
        // was discovered only when the function was called: a library can ship, pass
        // its own suite, and have a function that is dead for every consumer who
        // exercises a path the library's tests do not. Report it at load beside the
        // missing-cdylib report, naming the function, so the author sees it on the
        // first run rather than in a consumer's bug report.
        let Some(bridge) = get_bridge(sym) else {
            bridgeless.push(sym.clone());
            continue;
        };

        // Only wire if we can auto-marshal the signature.
        let sig = match compute_sig(data, d_nr) {
            Some(s) => s,
            None => continue,
        };

        // Get the library index for this symbol.
        let lib_idx = match state.library_names.get(sym) {
            Some(&idx) => idx,
            None => continue,
        };

        // Store the bridge and signature for runtime dispatch.
        sig_table.insert(lib_idx, (FnPtr(bridge), std::sync::Arc::new(sig)));

        // Replace the stub with the generic auto-marshal dispatcher.
        state.replace_static_fn(sym, native_auto_dispatch);
    }
    SIGS_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);

    if report_unresolved && !unresolved.is_empty() {
        report_unresolved_natives(data, &unresolved);
    }
    if !bridgeless.is_empty() {
        report_bridgeless_natives(data, &bridgeless);
    }
}

/// loft#886 — loud load-time diagnostic for `#native` symbols the cdylib exports but
/// never registered a marshal bridge for.
///
/// Separate from [`report_unresolved_natives`] because the fix is different: the
/// library is not stale, its registration is incomplete. The usual cause is a
/// hand-maintained `loft_register_bridges!` list that drifted from the `#native`
/// declarations — which is silent by construction, since the two lists are written in
/// different files and nothing compares them. `loft-ffi-build`'s
/// `generate_register_from_loft_with_bridges` derives BOTH from the `#native`
/// annotations and cannot drift; that is the fix to name.
///
/// Non-fatal, for the same reason the sibling is: a declared but never-called native
/// must still let the program run. The call-time panic in `native_auto_dispatch` is
/// still there for anyone who ignores this.
#[cfg(feature = "native-extensions")]
fn report_bridgeless_natives(data: &crate::data::Data, bridgeless: &[String]) {
    use std::collections::BTreeMap;
    let mut by_crate: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for sym in bridgeless {
        let krate = data
            .native_symbol_crates
            .get(sym)
            .map_or("<unknown library>", String::as_str);
        by_crate.entry(krate).or_default().push(sym.as_str());
    }
    for (krate, mut syms) in by_crate {
        syms.sort_unstable();
        eprintln!("{}", bridgeless_message(krate, &syms));
    }
}

/// The text [`report_bridgeless_natives`] prints for one library. Split out so the
/// message — the entire user-facing product of this diagnostic — is assertable without
/// a cdylib whose registration is deliberately broken.
#[cfg(feature = "native-extensions")]
fn bridgeless_message(krate: &str, syms: &[&str]) -> String {
    let shown = syms.iter().take(6).copied().collect::<Vec<_>>().join(", ");
    let more = syms.len().saturating_sub(6);
    let more_txt = if more > 0 {
        format!(", +{more} more")
    } else {
        String::new()
    };
    format!(
        "loft: native library '{krate}' registered no marshal bridge for {n} of its \
         #native function(s) ({shown}{more_txt}) — the cdylib loaded and exports the \
         symbol(s), so this is an incomplete registration, not a stale build. Calling one \
         panics. If its `loft_register_bridges!` list is hand-written, replace it with \
         `loft_ffi_build::generate_register_from_loft_with_bridges(\"../src\")` in the \
         native crate's build.rs, which derives the list from the `#native` annotations \
         and cannot drift.",
        n = syms.len(),
    )
}

#[cfg(all(test, feature = "native-extensions"))]
mod bridgeless_report_tests {
    use super::bridgeless_message;

    /// The report must name the LIBRARY to fix and every function that is dead in it —
    /// a count alone sends the author looking, which is the cost loft#886 paid.
    ///
    /// The sample symbols are invented, not borrowed from a real library: the
    /// extraction-hygiene gate (`tests/extraction_hygiene.rs`) forbids a library's `n_*`
    /// symbol from appearing anywhere in `src/`, and a test fixture is no exception.
    #[test]
    fn the_message_names_the_library_and_each_dead_function() {
        let m = bridgeless_message("samplelib", &["n_sample_alpha", "n_sample_beta"]);
        assert!(m.contains("'samplelib'"), "{m}");
        assert!(m.contains("n_sample_alpha"), "{m}");
        assert!(m.contains("n_sample_beta"), "{m}");
        assert!(m.contains("2 of its"), "{m}");
        // It must not read as the sibling "stale build" diagnostic: the fix is a
        // registration, and telling the author to rebuild sends them nowhere.
        assert!(!m.contains("rebuild the native libraries"), "{m}");
        assert!(
            m.contains("generate_register_from_loft_with_bridges"),
            "{m}"
        );
    }

    /// A long list is truncated, but the COUNT still tells the author how many are
    /// dead — a silently shortened list reads as "these seven", not "seven of twelve".
    #[test]
    fn a_long_list_is_truncated_but_the_count_is_not() {
        let syms: Vec<String> = (0..12).map(|i| format!("n_f{i}")).collect();
        let refs: Vec<&str> = syms.iter().map(String::as_str).collect();
        let m = bridgeless_message("big", &refs);
        assert!(m.contains("12 of its"), "{m}");
        assert!(m.contains("+6 more"), "{m}");
        assert!(!m.contains("n_f7"), "only the first six are listed: {m}");
    }
}

/// Loud load-time diagnostic for `#native` symbols whose cdylib never loaded.
/// Grouped by the owning crate (via `native_symbol_crates`) so the message names
/// the library to rebuild, not just orphan symbols.  Non-fatal — a declared but
/// never-called native must still let the program run, so this warns rather than
/// aborts; one the program CALLS is refused before it starts
/// ([`reachable_unimplemented_natives`], @C67).
#[cfg(feature = "native-extensions")]
fn report_unresolved_natives(data: &crate::data::Data, unresolved: &[String]) {
    use std::collections::BTreeMap;
    // A package with no native part at all never registered one, so the symbol→crate map
    // cannot name it; the library that DECLARED the function can (`qualified_type_name`'s
    // most-qualified rule), and naming it is the difference between a cure and a guess.
    let declared_in = |sym: &str| -> Option<String> {
        let d = (0..data.definitions()).find(|&d| data.def(d).native == sym)?;
        let q = data.qualified_type_name(d);
        let (lib, _) = q.rsplit_once("::")?;
        (!lib.starts_with("src")).then(|| lib.to_string())
    };
    let mut by_crate: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for sym in unresolved {
        let krate = data
            .native_symbol_crates
            .get(sym)
            .cloned()
            .or_else(|| declared_in(sym))
            .unwrap_or_else(|| "<unknown library>".to_string());
        by_crate.entry(krate).or_default().push(sym.as_str());
    }
    for (krate, mut syms) in by_crate {
        syms.sort_unstable();
        let shown = syms.iter().take(6).copied().collect::<Vec<_>>().join(", ");
        let more = syms.len().saturating_sub(6);
        let more_txt = if more > 0 {
            format!(", +{more} more")
        } else {
            String::new()
        };
        eprintln!(
            "loft: native library '{krate}' did not load — {n} of its #native function(s) are \
             unavailable ({shown}{more_txt}), and a program that calls one does not start. Its cdylib is missing or \
             stale (commonly built against a different libloft.rlib / loft-ffi). Rebuild it with \
             `make rebuild-native-cdylibs` (in the loft tree) or `cargo build --release` in the \
             library's native/ dir, then re-run.",
            n = syms.len(),
        );
    }
}

/// Is definition `d` a text producer the interpreter calls through its registered `_dest`
/// twin (`is_text_dest_native`)?  Its own symbol then stays a stub that no call reaches, so
/// it is neither missing nor worth a warning — the engine-host kernel natives live inside the
/// loft binary this way and were reported as a library that "did not load".
#[cfg(feature = "native-extensions")]
fn dest_served(state: &crate::state::State, data: &crate::data::Data, d: u32) -> bool {
    let def = data.def(d);
    crate::state::codegen::is_text_dest_native(def.name())
        && state
            .library_names
            .contains_key(&format!("{}_dest", def.native))
}

/// @C67 — the `#native` symbols `entry` can reach that nothing implements: no cdylib
/// registered them, and no wasm bridge serves them.  Asked after [`wire_native_fns`], by a
/// caller about to RUN `entry`, so a missing implementation is refused before the program
/// starts rather than panicking when the call is reached.  A merely DECLARED native is not
/// here — it stays the load-time warning [`report_unresolved_natives`] prints.  Reachability
/// is the one `--native` asks for its own refusal (P269), so the two backends agree on which
/// programs a missing native stops.
#[cfg(feature = "native-extensions")]
#[must_use]
pub fn reachable_unimplemented_natives(
    state: &crate::state::State,
    data: &crate::data::Data,
    entry: u32,
) -> Vec<String> {
    let guard = NATIVE_REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut out: Vec<String> = crate::generation::reachable_functions(data, &[entry])
        .into_iter()
        .filter(|&d| {
            let def = data.def(d);
            let sym = &def.native;
            !sym.is_empty()
                && !sym.starts_with("loft_shared_")
                && !sym.starts_with("loft_placed_")
                && state.native_stub_symbols.contains(sym)
                && !guard.as_ref().is_some_and(|r| r.contains_key(sym))
                && !data.wasm_bridge_routes.contains_key(sym)
                && !dest_served(state, data, d)
        })
        .map(|d| data.def(d).native.clone())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(not(feature = "native-extensions"))]
#[must_use]
pub fn reachable_unimplemented_natives(
    _state: &crate::state::State,
    _data: &crate::data::Data,
    _entry: u32,
) -> Vec<String> {
    Vec::new()
}

#[cfg(not(feature = "native-extensions"))]
pub fn wire_native_fns(_state: &mut crate::state::State, _data: &crate::data::Data) {}

#[cfg(not(feature = "native-extensions"))]
pub fn wire_native_fns_for(
    _state: &mut crate::state::State,
    _data: &crate::data::Data,
    _report_unresolved: bool,
) {
}

/// The suffix `#[loft_native]` appends when it generates a fn's marshal bridge.
/// The bridge for Rust fn `X` is `X__loft_bridge`, always adjacent to `X`, so a
/// bridge's exported name names its implementation.
#[cfg(feature = "native-extensions")]
const BRIDGE_SUFFIX: &str = "__loft_bridge";

/// Given the loft symbol a cdylib registered a bridge under, and that bridge's
/// own exported name, answer the Rust fn that implements the symbol — but only
/// when it is NOT the symbol itself.
///
/// `loft_register_bridges! { "S" => X__loft_bridge }` is a free mapping: nothing
/// requires `X == S`. When they agree (the clean binding `loft-ffi-build`
/// generates) the C symbol named `S` IS loft's entry point and there is nothing
/// to redirect. When they differ, `S` names some *other* export — for the
/// library that motivated loft#907, the same call in a raw `(ptr, count)` shape
/// instead of loft's `(LoftStore, LoftRef)` one — and linking `S` marshals the
/// call into the wrong function.
///
/// Split out from the pointer plumbing so the rule is testable without a cdylib.
#[cfg(feature = "native-extensions")]
fn impl_symbol_for(sym: &str, bridge_name: &str) -> Option<String> {
    let implemented_by = bridge_name.strip_suffix(BRIDGE_SUFFIX)?;
    (implemented_by != sym).then(|| implemented_by.to_string())
}

/// Ask the dynamic loader what the function at `ptr` is called — an EXACT hit only
/// (`platform::exported_symbol_at`), so a near miss leaves the symbol alone, which is the
/// pre-loft#907 behaviour.
#[cfg(feature = "native-extensions")]
fn exported_name_at(ptr: *const ()) -> Option<String> {
    crate::platform::exported_symbol_at(ptr)
}

/// loft#907 — record, for each `#native` symbol whose library implements it
/// under a DIFFERENT Rust name, what that name is, so `--native` links the same
/// function `--interpret` dispatches to.
///
/// The interpreter's resolution is the authoritative one: it reads the library's
/// own `loft_register_bridges_v1` table. Native codegen has no such table — it
/// puts the `#native` string straight into a `#[link_name]` — so the two backends
/// silently disagreed wherever a library remapped a symbol. Reading the answer off
/// the loaded cdylib gives both backends ONE source for it.
///
/// Call after [`load_all`]; a symbol whose cdylib is absent, un-loadable or
/// pre-bridge is simply left unmapped and codegen keeps linking it literally.
#[cfg(feature = "native-extensions")]
pub fn resolve_native_impl_symbols(data: &mut crate::data::Data) {
    let mut found: Vec<(String, String)> = Vec::new();
    for d_nr in 0..data.definitions() {
        let sym = data.def(d_nr).native();
        // The same gate native codegen uses to decide a symbol is a package
        // C-ABI native: anything else (a stem/dlopen native, a shared-store or
        // placed bridge) is not linked by name and must not be redirected.
        if sym.is_empty()
            || !data.native_symbol_crates.contains_key(sym)
            || data.native_impl_symbols.contains_key(sym)
        {
            continue;
        }
        let Some(bridge) = get_bridge(sym) else {
            continue;
        };
        if let Some(name) = exported_name_at(bridge)
            && let Some(implemented_by) = impl_symbol_for(sym, &name)
        {
            found.push((sym.to_string(), implemented_by));
        }
    }
    for (sym, implemented_by) in found {
        data.native_impl_symbols.insert(sym, implemented_by);
    }
}

#[cfg(not(feature = "native-extensions"))]
pub fn resolve_native_impl_symbols(_data: &mut crate::data::Data) {}

#[cfg(all(test, feature = "native-extensions"))]
mod impl_symbol_tests {
    use super::impl_symbol_for;

    // The symbols below are invented, not borrowed from a real library: the
    // extraction-hygiene gate (`tests/extraction_hygiene.rs`) forbids a library's
    // `n_*` symbol from appearing anywhere in `src/`, and a test is no exception.

    /// The clean binding — the library registered `S`'s bridge under `S`'s own
    /// name — must produce NO redirection, or every native call would be
    /// rewritten to link a name codegen already had right.
    #[test]
    fn a_bridge_named_after_its_own_symbol_redirects_nothing() {
        assert_eq!(
            impl_symbol_for("n_sample_alpha", "n_sample_alpha__loft_bridge"),
            None
        );
    }

    /// The loft#907 shape: the symbol the loft source declares is implemented by
    /// a differently-named Rust fn, and THAT is what `--native` must link.
    #[test]
    fn a_remapped_bridge_names_the_function_to_link() {
        assert_eq!(
            impl_symbol_for("samplelib_alpha", "n_sample_alpha__loft_bridge").as_deref(),
            Some("n_sample_alpha")
        );
    }

    /// A name that is not a generated bridge says nothing about which function
    /// implements the symbol — `dladdr` landing on some unrelated export must not
    /// be read as a remap.
    #[test]
    fn a_name_that_is_not_a_bridge_yields_nothing() {
        assert_eq!(impl_symbol_for("samplelib_alpha", "n_sample_alpha"), None);
        assert_eq!(impl_symbol_for("samplelib_alpha", "samplelib_alpha"), None);
    }
}

/// @PLN11 Arc N — wire the **shared-store** bridge dispatchers.  For every
/// `#native "loft_shared_…"` definition (the marker for an auto-generated
/// shared-store bridge), resolve the bridge symbol from the loaded cdylibs via
/// dlsym, record `(bridge_ptr, signature)` in `SHARED_SIGS`, and replace the stub
/// `OpStaticCall` target with `shared_store_dispatch`.  Call this *after*
/// `load_all` (and, if also using legacy `#native`, after `wire_native_fns` —
/// they handle disjoint symbol sets, `wire_native_fns` skips `loft_shared_…`).
#[cfg(feature = "native-extensions")]
pub fn wire_shared_native_fns(state: &mut crate::state::State, data: &crate::data::Data) {
    // @PLN199 C0 — every library cdylib receives the host's table before any of its bridges
    // can be called.  Idempotent: the cdylib keeps the first table it is handed.
    type InitFn = unsafe extern "C" fn(*const crate::native_lib::HostTable) -> u32;
    for init in crate::c_libs::dlsym_in_each("loft_shared_init") {
        let init: InitFn = unsafe { std::mem::transmute(init) };
        let accepted = unsafe { init(&raw const crate::native_lib::HOST_TABLE) };
        debug_assert_eq!(accepted, 1, "a library cdylib refused this host's table");
    }
    // Phase 1: collect resolvable shared bridges (no lock held).
    let mut wired: Vec<(String, u16, *const (), SharedSig)> = Vec::new();
    for d_nr in 0..data.definitions() {
        let def = data.def(d_nr);
        let sym = &def.native;
        if !sym.starts_with("loft_shared_") {
            continue;
        }
        // #303 — a `loft_shared_*`-marked def means codegen already emitted
        // `OpStaticCall` dispatches for it.  A wiring failure here leaves the
        // panicking stub behind those dispatches, so every skip must be LOUD:
        // the program will panic at the first call with a message that doesn't
        // name the function.
        let unwired = |reason: &str| {
            eprintln!(
                "loft: auto-native fn `{}` ({sym}) is marked for cdylib dispatch but \
                 could not be wired ({reason}) — calling it will panic",
                def.original_name(),
            );
        };
        // @PLN181 — the compiled standard library is in this binary, not in a cdylib.
        let found = crate::compiled_stdlib::bridge(sym)
            .map(|p| (p, false))
            .or_else(|| try_dlsym(sym));
        let Some((ptr, _uses_v1)) = found else {
            unwired("bridge symbol not found in any loaded cdylib");
            continue;
        };
        let Some(sig) = compute_shared_sig(data, d_nr) else {
            unwired("signature not bridge-marshallable (gate/wire divergence)");
            continue;
        };
        let Some(&lib_idx) = state.library_names.get(sym) else {
            unwired("no stub slot registered for the symbol");
            continue;
        };
        wired.push((sym.clone(), lib_idx, ptr, sig));
    }

    // Phase 2: record signatures, then replace the stub dispatch targets.
    {
        let mut guard = SHARED_SIGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let table = guard.get_or_insert_with(HashMap::new);
        for (_, lib_idx, ptr, sig) in &wired {
            table.insert(*lib_idx, (FnPtr(*ptr), std::sync::Arc::new(sig.clone())));
        }
        SIGS_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
    }
    // loft#715 — remember which symbol each slot dispatches to, so a fault raised
    // while the bridge is on the stack can NAME it.  Written once at wiring, read
    // only on the panic path.
    crate::call_frames::label_shared_bridges(
        wired
            .iter()
            .map(|(sym, lib_idx, ..)| (*lib_idx, sym.clone())),
    );
    for (sym, ..) in &wired {
        state.replace_static_fn(sym, shared_store_dispatch);
    }
    // The destination-passing slot of each text bridge: the same bridge and signature.
    let dests: Vec<(u16, String, FnPtr, std::sync::Arc<SharedSig>)> = wired
        .iter()
        .filter(|(_, _, _, sig)| matches!(sig.ret, Some(ArgT::Text)))
        .filter_map(|(sym, _, ptr, sig)| {
            let dest = shared_dest_symbol(sym);
            let idx = *state.library_names.get(&dest)?;
            Some((idx, dest, FnPtr(*ptr), std::sync::Arc::new(sig.clone())))
        })
        .collect();
    if !dests.is_empty() {
        {
            let mut guard = SHARED_SIGS
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let table = guard.get_or_insert_with(HashMap::new);
            for (idx, _, ptr, sig) in &dests {
                table.insert(*idx, (*ptr, std::sync::Arc::clone(sig)));
            }
            SIGS_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
        }
        crate::call_frames::label_shared_bridges(
            dests.iter().map(|(idx, dest, ..)| (*idx, dest.clone())),
        );
        for (_, dest, ..) in &dests {
            state.replace_static_fn(dest, shared_store_dispatch_dest);
        }
    }
}

#[cfg(not(feature = "native-extensions"))]
pub fn wire_shared_native_fns(_state: &mut crate::state::State, _data: &crate::data::Data) {}

/// @PLN11 Arc N — the shared-store bridge dispatcher.  Invoked via `OpStaticCall`
/// for a function whose `#native` symbol is an auto-generated `loft_shared_…`
/// bridge.  Reads the call's args off the interpreter stack into `LibArg` slots
/// (scalars by value; `vector`/`reference` as the **raw** stack `DbRef`, no
/// deref — the `--native` body expects the same indirect form), passes the
/// caller's `*mut Stores` directly (zero-marshalling shared store), calls the
/// bridge, and writes the return back onto the stack.
#[cfg(feature = "native-extensions")]
fn shared_store_dispatch(stores: &mut crate::database::Stores, stack: &mut crate::keys::DbRef) {
    shared_store_dispatch_raw(stores, stack);
}

/// The raw entry of `call` when it is a shared-bridge dispatcher.  `State::invoke_native`
/// asks, to enter it with the interpreter reachable for an upcall (@PLN199 C2).
pub type SharedRaw = fn(*mut crate::database::Stores, &mut crate::keys::DbRef);

#[cfg(feature = "native-extensions")]
#[must_use]
pub fn shared_dispatch_raw(call: crate::database::Call) -> Option<SharedRaw> {
    if std::ptr::fn_addr_eq(call, shared_store_dispatch as crate::database::Call) {
        Some(shared_store_dispatch_raw)
    } else if std::ptr::fn_addr_eq(call, shared_store_dispatch_dest as crate::database::Call) {
        Some(shared_store_dispatch_dest_raw)
    } else {
        None
    }
}

#[cfg(not(feature = "native-extensions"))]
#[must_use]
pub fn shared_dispatch_raw(_call: crate::database::Call) -> Option<SharedRaw> {
    None
}

/// The library slot of shared bridge `sym`'s destination-passing call (`<sym>#dest`): a
/// text call whose result goes to a caller cell pops that cell as one more slot, instead of
/// handing it over in a native call of its own (`n_set_bridge_dest`).
#[must_use]
pub fn shared_dest_symbol(sym: &str) -> String {
    format!("{sym}#dest")
}

/// [`shared_store_dispatch`] for a destination-passing text call: the destination `DbRef`
/// is the top slot, above the arguments.
#[cfg(feature = "native-extensions")]
fn shared_store_dispatch_dest(
    stores: &mut crate::database::Stores,
    stack: &mut crate::keys::DbRef,
) {
    shared_store_dispatch_dest_raw(stores, stack);
}

#[cfg(feature = "native-extensions")]
fn shared_store_dispatch_dest_raw(
    stores_raw: *mut crate::database::Stores,
    stack: &mut crate::keys::DbRef,
) {
    {
        let stores: &mut crate::database::Stores = unsafe { &mut *stores_raw };
        let dest = stores.get::<crate::keys::DbRef>(stack);
        stores.bridge_text_dest = Some(dest);
    }
    shared_store_dispatch_raw(stores_raw, stack);
}

std::thread_local! {
    /// @PLN199 C2 — the interpreter running the shared-bridge call on this thread, for a
    /// compiled library's call back into it (`HostTable::call_ref`).  Null outside one.
    static HOST_STATE: std::cell::Cell<*mut crate::state::State> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// Record `state` as the interpreter a bridge call on this thread may call back into, and
/// answer the one it replaces, which [`leave_host_state`] restores — so a nested bridge call
/// (a callback that calls the library again) leaves the outer one reachable.
pub fn enter_host_state(state: *mut crate::state::State) -> *mut crate::state::State {
    HOST_STATE.with(|c| c.replace(state))
}

/// Restore the interpreter [`enter_host_state`] replaced.
pub fn leave_host_state(prev: *mut crate::state::State) {
    HOST_STATE.with(|c| c.set(prev));
}

/// The interpreter running the current bridge call on this thread, or null.
pub(crate) fn host_state() -> *mut crate::state::State {
    HOST_STATE.with(std::cell::Cell::get)
}

/// [`shared_store_dispatch`] through a raw store pointer, the entry `State::invoke_native`
/// takes (@PLN199 C2).  No `&mut Stores` stays alive across the bridge call: the bridge may
/// call back into the interpreter, which reaches the same `Stores` through its `State`.  The
/// pops reborrow before the call and the pushes after it.
#[cfg(feature = "native-extensions")]
#[expect(clippy::too_many_lines, reason = "inherited")]
fn shared_store_dispatch_raw(
    stores_raw: *mut crate::database::Stores,
    stack: &mut crate::keys::DbRef,
) {
    use crate::keys::DbRef;
    use crate::native_lib::LibArg;
    let stores: &mut crate::database::Stores = unsafe { &mut *stores_raw };

    crate::state::SHARED_DISPATCH_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let lib_idx = crate::call_frames::current_lib_idx();
    // loft#715 — mark that a bridge call is on the stack.  A fault raised from
    // here is a BRIDGE fault, and it used to read exactly like a program fault:
    // `Cannot add to none-structure '<type>'` names a type, never the library
    // that supplied the index, and the abort takes the whole process with it.
    // One `Cell` write per call, cleared on the way out.
    let outer_bridge = crate::call_frames::enter_shared_bridge(lib_idx);
    let Some((FnPtr(bridge_ptr), sig)) = cached_sig(&SHARED_SIG_CACHE, &SHARED_SIGS, lib_idx)
    else {
        panic!("no shared signature for lib_idx {lib_idx}")
    };

    // Pop ONE stack slot per attribute (the call site pushes every attribute,
    // including hidden dests and the text work buffer) — in reverse (stack is
    // LIFO), each into its declaration-order slot.  On the stack for any signature
    // of up to `INLINE_ARGS` attributes: two `Vec`s per call were a third of a bare
    // crossing's cost (@PLN199 perf).
    let n_pops = sig.pops.len();
    let mut popped_inline = [LibArg::ZERO; INLINE_ARGS];
    let mut popped_spill: Vec<LibArg> = Vec::new();
    let popped: &mut [LibArg] = if n_pops <= INLINE_ARGS {
        &mut popped_inline[..n_pops]
    } else {
        popped_spill.resize(n_pops, LibArg::ZERO);
        &mut popped_spill
    };
    for (at, &t) in sig.pops.iter().enumerate().rev() {
        let slot = match t {
            ArgT::I32 | ArgT::I64 => LibArg {
                scalar: stores.get::<i64>(stack),
                ..LibArg::ZERO
            },
            ArgT::F64 => LibArg {
                scalar: (stores.get::<f64>(stack)).to_bits() as i64,
                ..LibArg::ZERO
            },
            ArgT::F32 => LibArg {
                scalar: i64::from((stores.get::<f64>(stack) as f32).to_bits()),
                ..LibArg::ZERO
            },
            ArgT::Bool => LibArg {
                scalar: i64::from(stores.get::<bool>(stack)),
                ..LibArg::ZERO
            },
            // The raw stack DbRef, passed UNCHANGED (the --native body consumes
            // the indirect-header form, not the dereferenced direct record).
            ArgT::Ref | ArgT::Vec => LibArg {
                dbref: stores.get::<DbRef>(stack),
                ..LibArg::ZERO
            },
            // Text arg → `&str` for the body: the store-backed bytes (borrowed for
            // the call's duration, valid because the store is shared and live).
            ArgT::Text => {
                let s = stores.get::<crate::keys::Str>(stack);
                LibArg {
                    text_ptr: s.ptr,
                    text_len: s.len as usize,
                    ..LibArg::ZERO
                }
            }
            ArgT::FnRef => {
                let (d_nr, closure) = read_fn_ref(stores.get::<[u8; FN_REF_SIZE]>(stack));
                LibArg {
                    scalar: d_nr,
                    dbref: closure,
                    ..LibArg::ZERO
                }
            }
        };
        popped[at] = slot;
    }
    // Forward only the marshalled slots (compacted order — the bridge reads
    // `a[0..n]` skipping work buffers and hidden dests it satisfies locally).
    let mut args_inline = [LibArg::ZERO; INLINE_ARGS];
    let mut args_spill: Vec<LibArg> = Vec::new();
    let mut n_args = 0;
    for (a, f) in popped.iter().zip(&sig.forward) {
        if !*f {
            continue;
        }
        if n_pops <= INLINE_ARGS {
            args_inline[n_args] = *a;
        } else {
            args_spill.push(*a);
        }
        n_args += 1;
    }
    let args: &[LibArg] = if n_pops <= INLINE_ARGS {
        &args_inline[..n_args]
    } else {
        &args_spill
    };

    // @PLN10 — dest-mode detection: the interpreter caller routes a text-returning
    // auto-native call through `gen_cdylib_text_dest_call`, which set a per-call
    // `bridge_text_dest` before this dispatch.  The bridge consumes it (writes the
    // result into that caller-owned record) and signals dest-mode by leaving `ret`
    // text null — so we must push NOTHING here, matching the codegen's dest-mode
    // stack accounting.  Captured before the call because the bridge `take()`s it.
    let text_dest_mode = stores.bridge_text_dest.is_some();
    // #303 — expression-context text return: no external dest was stashed, so
    // reuse the call's own work buffer (a caller-cleared CreateStack cell whose
    // DbRef the call site pushed) as the destination; after the call a `Str`
    // over its content is pushed as the result, mirroring the interpreted ABI.
    let self_dest: Option<DbRef> = if !text_dest_mode && matches!(sig.ret, Some(ArgT::Text)) {
        sig.text_workbuf.map(|i| {
            let d = popped[i].dbref;
            stores.bridge_text_dest = Some(d);
            d
        })
    } else {
        None
    };

    let mut ret = LibArg::ZERO;
    let stores_ptr: *mut crate::database::Stores = stores_raw;
    // SAFETY: `bridge_ptr` is a `loft_shared_…` export of an auto-generated cdylib
    // that links this exact `LibArg` / `Stores` / `DbRef` from libloft, so the ABI
    // matches.  `stores_ptr` is borrowed from the live `&mut Stores`; the bridge
    // uses it (and only it) for the duration of the call.
    let bridge: unsafe extern "C" fn(
        *mut crate::database::Stores,
        *const LibArg,
        usize,
        *mut LibArg,
    ) = unsafe { std::mem::transmute(bridge_ptr) };
    unsafe {
        bridge(
            stores_ptr,
            args.as_ptr(),
            args.len(),
            std::ptr::from_mut(&mut ret),
        )
    };
    // Reborrowed after the call: the bridge may have called back into the interpreter.
    let stores: &mut crate::database::Stores = unsafe { &mut *stores_raw };

    // @PLN118 arc F — non-perturbing attribution of the shared-return orphan.
    // For a struct/vector return the caller pre-allocates a retbuf and forwards
    // it as `popped[hidden_dest]`; the bridge writes into it and returns it.  If
    // the callee IGNORES the retbuf and returns a fresh store (e.g. a struct
    // literal that allocates a fresh record), the returned ref differs from the
    // forwarded retbuf and the retbuf is orphaned — one leaked store per call.
    // Gated.
    // Asked once: a `getenv` per bridge call was 1.5 % of an interpreted script whose
    // compiled standard library calls cross here 350,000 times.
    static TRACE_SHARED_RET: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *TRACE_SHARED_RET.get_or_init(|| std::env::var_os("LOFT_TRACE_SHARED_RET").is_some())
        && let Some(hd) = sig.hidden_dest
        && matches!(sig.ret, Some(ArgT::Ref | ArgT::Vec))
    {
        let rb = popped[hd].dbref;
        let rf = ret.dbref;
        let rb_free = (rb.store_nr as usize) < stores.allocations.len()
            && stores.allocations[rb.store_nr as usize].free;
        eprintln!(
            "[shared-ret] lib_idx={lib_idx} retbuf=({},{},{}) return=({},{},{}) \
             same={} retbuf_freed={}",
            rb.store_nr,
            rb.rec,
            rb.pos,
            rf.store_nr,
            rf.rec,
            rf.pos,
            rb.store_nr == rf.store_nr && rb.rec == rf.rec && rb.pos == rf.pos,
            rb_free,
        );
    }

    match sig.ret {
        None => {}
        Some(ArgT::I32 | ArgT::I64) => stores.put::<i64>(stack, ret.scalar),
        Some(ArgT::F64) => stores.put::<f64>(stack, f64::from_bits(ret.scalar as u64)),
        Some(ArgT::F32) => stores.put::<f64>(stack, f64::from(f32::from_bits(ret.scalar as u32))),
        Some(ArgT::Bool) => stores.put(stack, ret.scalar != 0),
        Some(ArgT::Ref | ArgT::Vec) => stores.put::<DbRef>(stack, ret.dbref),
        Some(ArgT::FnRef) => stores.put(stack, write_fn_ref(ret.scalar, ret.dbref)),
        // @PLN10 — dest-passing: in dest-mode the bridge wrote the result into the
        // caller-owned `bridge_text_dest` record and left `ret` text null; the value
        // lives in its destination, so push NOTHING (matches the dest-mode codegen).
        Some(ArgT::Text) if text_dest_mode => {}
        // #303 — expression context: the bridge wrote the result into the
        // self-stashed work buffer; push a `Str` over its content (the cell
        // outlives the expression — it is the call's own CreateStack cell),
        // matching the +16-byte result the call-site codegen accounts.
        Some(ArgT::Text) => {
            if let Some(d) = self_dest {
                let s: &String = stores.store(&d).addr::<String>(d.rec, d.pos);
                let result = crate::keys::Str {
                    ptr: s.as_ptr(),
                    len: s.len() as u32,
                };
                stores.put(stack, result);
            } else {
                // A text return with neither an external dest nor a work
                // buffer — there is no caller cell to carry the bytes.  The parser
                // hands every value-position call a temporary
                // (`Parser::crosses_text_bridge`), so reaching here is a coverage gap;
                // degrade to an empty `Str` and flag in dev.
                debug_assert!(
                    ret.text_ptr.is_null(),
                    "auto-native text return reached the dispatcher without any \
                     dest cell — @PLN10 dest-passing coverage gap"
                );
                stores.put(stack, crate::keys::Str::new(""));
            }
        }
    }
    crate::call_frames::leave_shared_bridge(outer_bridge);
}

/// The attribute count a shared-bridge call marshals without allocating
/// ([`shared_store_dispatch_raw`]); a wider signature spills to the heap.
#[cfg(feature = "native-extensions")]
const INLINE_ARGS: usize = 16;

/// The interpreter's function-value slot: an 8-byte `d_nr`, then the closure `DbRef`
/// (`OpVarFnRef`, `data::element_stack_size_in`).
const FN_REF_SIZE: usize = 20;

const _: () = assert!(FN_REF_SIZE == 8 + std::mem::size_of::<crate::keys::DbRef>());

/// The `(d_nr, closure)` a function-value slot holds.
#[cfg(feature = "native-extensions")]
fn read_fn_ref(raw: [u8; FN_REF_SIZE]) -> (i64, crate::keys::DbRef) {
    let d_nr = i64::from_ne_bytes(raw[0..8].try_into().unwrap_or_default());
    let closure =
        unsafe { std::ptr::read_unaligned(raw[8..].as_ptr().cast::<crate::keys::DbRef>()) };
    (d_nr, closure)
}

/// The function-value slot for `(d_nr, closure)`.
#[cfg(feature = "native-extensions")]
fn write_fn_ref(d_nr: i64, closure: crate::keys::DbRef) -> [u8; FN_REF_SIZE] {
    let mut raw = [0u8; FN_REF_SIZE];
    raw[0..8].copy_from_slice(&d_nr.to_ne_bytes());
    unsafe {
        std::ptr::write_unaligned(raw[8..].as_mut_ptr().cast::<crate::keys::DbRef>(), closure)
    };
    raw
}

/// Generic auto-marshal dispatcher. Called via `OpStaticCall` for all
/// auto-wired native functions. Reads signature from `NATIVE_SIGS` using
/// the library index stored in the bytecode (passed via the stack frame).
///
/// Since `Call = fn(&mut Stores, &mut DbRef)` doesn't receive the library
/// index, we use CURRENT_LIB_IDX (set by a patched static_call).
///
/// Actually — `State::static_call()` doesn't pass the library index to
/// the Call function. We need a different mechanism.
///
/// Solution: use a thread-local that `static_call` sets before invoking.
#[cfg(feature = "native-extensions")]
fn native_auto_dispatch(stores: &mut crate::database::Stores, stack: &mut crate::keys::DbRef) {
    // Read the current library index from the thread-local.
    let lib_idx = crate::call_frames::current_lib_idx();

    // Plan-74: dispatch goes through the generated `#[loft_native]` bridge the owning
    // cdylib registered, resolved at wiring — a symbol without one is reported there
    // (loft#886) and never wired, so its stub stays in place.
    let Some((FnPtr(bridge_ptr), sig)) = cached_sig(&NATIVE_SIG_CACHE, &NATIVE_SIGS, lib_idx)
    else {
        panic!("no signature for lib_idx {lib_idx}")
    };
    dispatch_via_bridge(stores, stack, bridge_ptr, &sig);
}

/// A native call's argument slots: on the stack up to [`ArgBuf::INLINE`], on the heap past it,
/// so the common call allocates nothing.
#[cfg(feature = "native-extensions")]
enum ArgBuf<T: Copy> {
    Inline([T; ArgBuf::<()>::INLINE], usize),
    Heap(Vec<T>),
}

#[cfg(feature = "native-extensions")]
impl<T: Copy> ArgBuf<T> {
    const INLINE: usize = 8;

    fn new(len: usize, fill: T) -> Self {
        if len <= Self::INLINE {
            Self::Inline([fill; ArgBuf::<()>::INLINE], len)
        } else {
            Self::Heap(vec![fill; len])
        }
    }

    fn as_mut(&mut self) -> &mut [T] {
        match self {
            Self::Inline(a, len) => &mut a[..*len],
            Self::Heap(v) => v,
        }
    }
}

#[cfg(all(test, feature = "native-extensions"))]
mod dispatch_cache_tests {
    use super::{ArgBuf, SIGS_GENERATION, SigCache, cached_sig};
    use std::collections::HashMap;
    use std::sync::Mutex;

    static TABLE: Mutex<Option<HashMap<u16, u32>>> = Mutex::new(None);
    thread_local! {
        static CACHE: std::cell::RefCell<SigCache<u32>> =
            const { std::cell::RefCell::new(SigCache { generation: u64::MAX, slots: Vec::new() }) };
    }

    fn wire(lib_idx: u16, v: u32) {
        TABLE
            .lock()
            .unwrap()
            .get_or_insert_with(HashMap::new)
            .insert(lib_idx, v);
        SIGS_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Release);
    }

    /// A rewire of a slot this thread already called through must be seen on the next call:
    /// two programs run one after another in one thread can give the same library index to
    /// different natives, and a stale copy would call the first program's bridge.
    #[test]
    fn a_rewired_slot_is_never_served_from_the_stale_copy() {
        wire(300, 11);
        assert_eq!(cached_sig(&CACHE, &TABLE, 300), Some(11));
        assert_eq!(cached_sig(&CACHE, &TABLE, 300), Some(11));
        wire(300, 22);
        assert_eq!(cached_sig(&CACHE, &TABLE, 300), Some(22));
        assert_eq!(cached_sig(&CACHE, &TABLE, 301), None);
    }

    /// Past the inline slots the buffer moves to the heap and keeps every argument.
    #[test]
    fn a_long_argument_list_keeps_every_slot() {
        for len in [0, 1, ArgBuf::<u8>::INLINE, ArgBuf::<u8>::INLINE + 1, 20] {
            let mut b = ArgBuf::new(len, 0u32);
            for (i, s) in b.as_mut().iter_mut().enumerate() {
                *s = u32::try_from(i).unwrap() * 3 + 1;
            }
            let got: Vec<u32> = b.as_mut().to_vec();
            let want: Vec<u32> = (0..len)
                .map(|i| u32::try_from(i).unwrap() * 3 + 1)
                .collect();
            assert_eq!(got, want, "len {len}");
        }
    }
}

/// Plan-25 F3: dispatch a native call through its generated `LoftBridgeFn`.
///
/// Marshals the stack args into `[LoftValue]` at FULL width (the bridge casts
/// each to the impl's real param type), builds the `LoftStore` from the first
/// ref arg's store (or store 0), calls the bridge, and writes the tagged
/// return back to the stack.  Replaces the ~98-arm `dispatch_call` for any
/// symbol whose cdylib was built with `#[loft_native]`.
#[cfg(feature = "native-extensions")]
fn dispatch_via_bridge(
    stores: &mut crate::database::Stores,
    stack: &mut crate::keys::DbRef,
    bridge_ptr: *const (),
    sig: &NativeSig,
) {
    use crate::keys::Str;
    use loft_ffi::{LoftRef as FfiRef, LoftStr as FfiStr, LoftTag, LoftValue};

    // Pop the raw arguments (in reverse — LIFO — then restore order), then set the
    // heap-typed ones up as ONE bridge call (`Stores::bridge_args`, @PLN174 F5): the store
    // the bridge is handed is the first argument's, a foreign vector is copied into it,
    // and a vector or record answer with nothing to pin takes a store minted for it.
    #[derive(Clone, Copy)]
    enum Raw {
        Int(i64),
        Float(f64),
        Bool(bool),
        Text(Str),
        Heap(crate::keys::DbRef),
    }
    let mut raw = ArgBuf::new(sig.params.len(), Raw::Int(0));
    for (at, &t) in sig.params.iter().enumerate().rev() {
        raw.as_mut()[at] = match t {
            ArgT::I32 | ArgT::I64 => Raw::Int(stores.get::<i64>(stack)),
            ArgT::F32 | ArgT::F64 => Raw::Float(stores.get::<f64>(stack)),
            ArgT::Bool => Raw::Bool(stores.get::<bool>(stack)),
            ArgT::Text => Raw::Text(stores.get::<Str>(stack)),
            ArgT::Ref | ArgT::Vec => Raw::Heap(stores.get::<crate::keys::DbRef>(stack)),
            ArgT::FnRef => unreachable!("a native library's signature carries no function value"),
        };
    }
    let raw = raw.as_mut();
    let returns_ref = matches!(sig.ret, Some(ArgT::Ref | ArgT::Vec));
    let heap: Vec<(crate::keys::DbRef, bool)> = sig
        .params
        .iter()
        .zip(raw.iter())
        .filter_map(|(t, r)| match r {
            Raw::Heap(db) => Some((*db, *t == ArgT::Vec)),
            _ => None,
        })
        .collect();
    // A call with no store argument and no store answer — every scalar/text native — has
    // nothing for `bridge_args` to pin, copy or mint, nor for `bridge_args_done` to undo.
    let call = (!heap.is_empty() || returns_ref).then(|| stores.bridge_args(&heap, returns_ref));
    let (store_nr, minted) = call.as_ref().map_or((0, false), |c| (c.store_nr, c.minted));
    let mut handed = call.as_ref().map_or([].iter(), |c| c.handed.iter());
    let mut args = ArgBuf::new(sig.params.len(), LoftValue::VOID);
    for (at, (t, r)) in sig.params.iter().zip(raw.iter()).enumerate() {
        args.as_mut()[at] = match (t, r) {
            // No pre-narrowing: pass the whole i64 cell; the bridge casts to
            // the impl's real width (i32/u16/…) per its Rust signature.
            (_, Raw::Int(i)) => LoftValue::int(*i),
            (_, Raw::Float(f)) => LoftValue::float(*f),
            (_, Raw::Bool(b)) => LoftValue::boolean(*b),
            (_, Raw::Text(s)) => LoftValue::text(FfiStr {
                ptr: s.str().as_ptr(),
                len: s.str().len(),
            }),
            (ArgT::Vec, Raw::Heap(_)) => {
                // Same indirect-vector deref as the legacy marshal.
                let r = handed.next().copied().unwrap_or(crate::keys::DbRef::NULL);
                let rec = if r.rec == 0 || r.pos == 0 {
                    0
                } else {
                    stores.store(&r).get_u32_raw(r.rec, r.pos)
                };
                LoftValue::reference(FfiRef {
                    store_nr: r.store_nr,
                    rec,
                    pos: 0,
                })
            }
            (_, Raw::Heap(_)) => {
                let r = handed.next().copied().unwrap_or(crate::keys::DbRef::NULL);
                LoftValue::reference(FfiRef {
                    store_nr: r.store_nr,
                    rec: r.rec,
                    pos: r.pos,
                })
            }
        };
    }
    let args = args.as_mut();
    let ls = make_loft_store(stores, store_nr, minted);

    // CURRENT_STORES must be live for the bridge's ffi_claim/resize callbacks.
    struct StoresGuard;
    impl Drop for StoresGuard {
        fn drop(&mut self) {
            CURRENT_STORES.with(|c| c.set(std::ptr::null_mut()));
        }
    }
    let stores_ptr: *mut crate::database::Stores = stores;
    CURRENT_STORES.with(|c| c.set(stores_ptr));
    let _guard = StoresGuard;

    let mut ret = LoftValue::VOID;
    // SAFETY: `bridge_ptr` is a `loft_ffi::LoftBridgeFn` registered by the
    // cdylib; the local `LoftStore` mirror is `#[repr(C)]`-identical to
    // `loft_ffi::LoftStore`, so the ABI matches.
    let bridge: unsafe extern "C" fn(LoftStore, *const LoftValue, usize, *mut LoftValue) =
        unsafe { std::mem::transmute(bridge_ptr) };
    unsafe { bridge(ls, args.as_ptr(), args.len(), std::ptr::from_mut(&mut ret)) };

    // The copies the foreign vector arguments took, and a store minted for a scalar answer.
    if let Some(call) = &call {
        stores.bridge_args_done(call, returns_ref);
    }

    // Write the tagged return to the stack (mirrors dispatch_call's returns).
    match ret.tag {
        LoftTag::Void => {}
        // The bridge already widened i32 returns (i32::MIN → i64::MIN).
        LoftTag::I64 => stores.put::<i64>(stack, ret.as_i64()),
        LoftTag::Bool => stores.put(stack, ret.as_bool()),
        LoftTag::F64 => stores.put(stack, ret.as_f64()),
        LoftTag::Text => bridge_push_str(stores, stack, ret.as_text()),
        LoftTag::Ref => bridge_push_ref(stores, stack, ret.as_ref()),
    }
}

/// Copy a returned `loft_ffi::LoftStr` into the stack (mirror of
/// `dispatch_call`'s nested `push_loft_str`, for the bridge path).
#[cfg(feature = "native-extensions")]
fn bridge_push_str(
    stores: &mut crate::database::Stores,
    stack: &mut crate::keys::DbRef,
    s: loft_ffi::LoftStr,
) {
    bridge_text_result(stores, stack, s.ptr, s.len);
}

/// Materialise a foreign `LoftStr` text return into the interpreter.
///
/// @PLN10 N2b — when `stores.bridge_text_dest` is set (by `n_set_bridge_dest`,
/// emitted immediately before this cdylib call by `gen_cdylib_text_dest_call`),
/// write the bytes into that caller-owned work-buffer record and push NOTHING —
/// the record IS the result (read by the chokepoint's `Var(w)`), so the
/// never-cleared `stores.scratch` is bypassed.  Otherwise fall back to the legacy
/// scratch-backed `Str` (the value-position case the chokepoint hasn't wrapped).
#[cfg(feature = "native-extensions")]
fn bridge_text_result(
    stores: &mut crate::database::Stores,
    stack: &mut crate::keys::DbRef,
    ptr: *const u8,
    len: usize,
) {
    use crate::keys::Str;
    let text: &str = if !ptr.is_null() && len > 0 {
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) }
    } else {
        ""
    };
    if let Some(dest) = stores.bridge_text_dest.take() {
        if !text.is_empty() {
            stores
                .store_mut(&dest)
                .addr_mut::<String>(dest.rec, dest.pos)
                .push_str(text);
        }
        return;
    }
    // @PLN10 D/G2 — no dest set ⇒ this cdylib text call was NOT routed through
    // `n_set_bridge_dest` (an uncovered value position).  Dest-passing covers
    // every position in the corpus (whole-suite `=panic` == 0), so this is dead;
    // degrade gracefully to an empty `Str` rather than re-introduce
    // `stores.scratch` (the field is being retired), and flag the coverage gap
    // loudly in dev builds.
    debug_assert!(
        false,
        "cdylib text return reached the bridge without a dest (uncovered value \
         position) — @PLN10 N2b coverage gap"
    );
    stores.put(stack, Str::new(""));
}

/// Wrap a returned `loft_ffi::LoftRef` (direct vector record) in the indirect
/// header layout the interpreter expects, and push it (mirror of
/// `dispatch_call`'s nested `push_loft_ref`, for the bridge path).
#[cfg(feature = "native-extensions")]
fn bridge_push_ref(
    stores: &mut crate::database::Stores,
    stack: &mut crate::keys::DbRef,
    r: loft_ffi::LoftRef,
) {
    let base = crate::keys::DbRef {
        store_nr: r.store_nr,
        rec: 0,
        pos: 0,
    };
    // @PLN174 F5 — a foreign answer already has its handle (the slot `foreign_vector`
    // claimed), and its store is locked, so no header is claimed here.
    if crate::store::Store::is_foreign_rec(r.rec)
        && let Some((rec, pos)) = stores.store(&base).foreign_handle()
    {
        stores.put(
            stack,
            crate::keys::DbRef {
                store_nr: r.store_nr,
                rec,
                pos,
            },
        );
        return;
    }
    let header = stores.claim(&base, 1);
    stores.store_mut(&base).set_u32_raw(header.rec, 4, r.rec);
    let dbref = crate::keys::DbRef {
        store_nr: r.store_nr,
        rec: header.rec,
        pos: 4,
    };
    stores.put(stack, dbref);
}

// ── Store callback infrastructure for FFI allocation ─────────────────────

// Thread-local raw pointer to the interpreter's Stores during a native call.
// Set before calling a native function, cleared after it returns.
//
// Ungated on purpose, unlike the `ffi_*` callbacks below that read it: it is also
// the backing store for [`native_call`], which generated code links on a target
// that has no `dlopen` and therefore no `native-extensions` (loft#967).
std::thread_local! {
    pub(crate) static CURRENT_STORES: std::cell::Cell<*mut crate::database::Stores> =
        const { std::cell::Cell::new(std::ptr::null_mut()) };
}

/// C-ABI callback: allocate `words` 8-byte words in the store identified by ctx.
/// Returns the new record number. May reallocate the store buffer.
/// Returns 0 if the allocation panics (caught to prevent UB at the C-ABI boundary).
#[cfg(feature = "native-extensions")]
unsafe extern "C" fn ffi_claim(ctx: LoftStoreCtx, words: u32) -> u32 {
    let store_nr = ctx._opaque as usize as u16;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        CURRENT_STORES.with(|c| {
            let stores = unsafe { &mut *c.get() };
            let store = &mut stores.allocations[store_nr as usize];
            store.claim(words)
        })
    }))
    .unwrap_or(0)
}

/// C-ABI callback: resize record `rec` to `words` 8-byte words.
/// Returns the (possibly new) record number. May reallocate the store buffer.
/// Returns `rec` unchanged if the resize panics.
#[cfg(feature = "native-extensions")]
unsafe extern "C" fn ffi_resize(ctx: LoftStoreCtx, rec: u32, words: u32) -> u32 {
    let store_nr = ctx._opaque as usize as u16;
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        CURRENT_STORES.with(|c| {
            let stores = unsafe { &mut *c.get() };
            let store = &mut stores.allocations[store_nr as usize];
            store.resize(rec, words)
        })
    }))
    .unwrap_or(rec)
}

/// C-ABI callback: refresh ptr and size after a potential reallocation.
/// No-op if the reload panics (ptr/size remain unchanged).
#[cfg(feature = "native-extensions")]
unsafe extern "C" fn ffi_reload(ctx: LoftStoreCtx, out_ptr: *mut *mut u8, out_size: *mut u32) {
    let store_nr = ctx._opaque as usize as u16;
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        CURRENT_STORES.with(|c| {
            let stores = unsafe { &*c.get() };
            let store = &stores.allocations[store_nr as usize];
            unsafe {
                *out_ptr = store.base_ptr();
                *out_size = store.capacity_words();
            }
        });
    }));
}

pub use crate::call_frames::{current_lib_idx, set_current_lib_idx};

pub use crate::call_frames::current_shared_bridge;

/// C-ABI callback: adopt a cdylib's byte block as a read-only `vector<u8>` (@PLN174 F5,
/// [`adopt_foreign_block`]).  The null ref if the adoption panics: the block stays the
/// cdylib's, which then copies it.
#[cfg(feature = "native-extensions")]
unsafe extern "C" fn ffi_foreign(
    ctx: LoftStoreCtx,
    ptr: *mut u8,
    len: usize,
    cap: usize,
    elem_size: u32,
    release: unsafe extern "C" fn(*mut u8, usize, usize),
) -> loft_ffi::LoftRef {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        CURRENT_STORES.with(|c| {
            let stores = unsafe { &mut *c.get() };
            adopt_foreign_block(
                stores,
                ctx._opaque as usize,
                ptr,
                len,
                cap,
                elem_size,
                release,
            )
        })
    }))
    .unwrap_or(loft_ffi::LoftRef {
        store_nr: 0,
        rec: 0,
        pos: 0,
    })
}

/// Build a LoftStore handle from the store that a LoftRef points to.
/// Includes allocation callbacks so native code can create records and vectors.
/// `minted`: the store was made for the bridge's vector return and is empty
/// ([`CTX_RETURN_STORE`]).
#[cfg(feature = "native-extensions")]
fn make_loft_store(stores: &crate::database::Stores, store_nr: u16, minted: bool) -> LoftStore {
    let store = stores.store(&crate::keys::DbRef {
        store_nr,
        rec: 0,
        pos: 0,
    });
    let ctx = store_nr as usize | if minted { CTX_RETURN_STORE } else { 0 };
    LoftStore {
        ptr: store.base_ptr(),
        size: store.capacity_words(),
        ctx: LoftStoreCtx {
            _opaque: ctx as *mut (),
        },
        claim_fn: Some(ffi_claim),
        reload_fn: Some(ffi_reload),
        resize_fn: Some(ffi_resize),
        foreign_fn: Some(ffi_foreign),
    }
}

// ── Auto-build ──────────────────────────────────────────────────────────

/// Build the native cdylib of every INSTALLED registry package, one at a time.
///
/// @P229 G3, restored at the new location.  A test that `use`s a native-backed package
/// calls [`auto_build_native`], which takes ONE global exclusive lock
/// (`loft-native-build.lock`) around its `cargo build`.  Under a parallel test runner many
/// of those fire at once: each blocked process still occupies a runner slot, so the suite
/// loses concurrency it is being charged for, and the builds churn the shared cargo dir
/// while they are at it.
///
/// CI used to prevent that by pre-building `lib/<pkg>/native/` sequentially before the
/// suite.  Those libraries moved to their own repos (@PLAN12 5b) and the step became a
/// stub, so the rebuilds went back to happening DURING the suite — measured on
/// `tuxedo-work-2026-08-28`: 7 lock waits totalling 62.5s, worst 26.9s, with five cdylibs
/// rebuilding mid-run because a fresh loft binary stamps them all stale.
///
/// This is that step at the new location.  It calls the same [`resolve_native_lib`] the
/// tests reach, so there is no second spelling of where a cdylib lands or when it is
/// stale — the reason a shell loop cannot do this job: `native_target_root`'s redirect and
/// the `[library] native` stem would both have to be re-derived.
///
/// Best-effort by design. A package that fails to build here is not fatal: the test that
/// needs it will try again and report properly. Returns `(attempted, built)`.
#[cfg(feature = "registry")]
pub fn prebuild_installed_natives() -> (usize, usize) {
    let Ok(entries) = crate::file_access::read_dir(crate::registry_index::cache_dir()) else {
        return (0, 0);
    };
    let mut dirs: Vec<std::path::PathBuf> = entries
        .iter()
        .map(crate::file_access::PathText::os_spelling)
        .filter(|p| crate::file_access::exists(p.join("native").join("Cargo.toml")))
        .collect();
    // Deterministic order, so two runs of the same tree do the same work in the same
    // sequence and a slow package is identifiable from the log rather than from timing.
    dirs.sort();
    let (mut attempted, mut built) = (0, 0);
    for dir in dirs {
        // The stem comes from the package's own `[library] native = "…"`, read through
        // the one manifest reader — the same value the parser resolves at `use` time.
        let Some(stem) = crate::manifest::read_manifest(&dir.join("loft.toml").to_string_lossy())
            .and_then(|m| m.native)
        else {
            continue;
        };
        attempted += 1;
        let pkg = dir.to_string_lossy().into_owned();
        // Name each package as it is reached.  A warm cache makes `resolve_native_lib` an
        // early return, so this is nearly silent; a run that takes minutes is one where the
        // loft binary changed and every stamp went stale, and then this log is the only
        // thing that says which package is being waited on.
        let name = crate::file_access::file_name(&dir).unwrap_or_default();
        let t = std::time::Instant::now();
        if resolve_native_lib(&pkg, &stem).is_some() {
            built += 1;
            let secs = t.elapsed().as_secs_f64();
            if secs > 1.0 {
                println!("  prebuilt {name} ({secs:.1}s)");
            }
        } else {
            println!("  warn: {name} did not resolve — the test that needs it will retry");
        }
    }
    (attempted, built)
}

pub fn auto_build_native_target(pkg_dir: &str, stem: &str, target: &str) -> bool {
    use std::path::PathBuf;
    let pkg = PathBuf::from(pkg_dir);
    let cargo_toml = pkg.join("native").join("Cargo.toml");
    if !crate::file_access::exists(&cargo_toml) {
        return false;
    }
    let rlib_name = format!("lib{stem}.rlib");
    // A `cargo build --target <t>` writes under `<dir>/<t>/release/`; the wasm consume
    // path reads the IN-TREE `native/target` (not the redirected host root), so build
    // there to keep the produced rlib and the linked rlib the same file.
    let out_dir = pkg
        .join("native")
        .join("target")
        .join(target)
        .join("release");
    let rlib = out_dir.join(&rlib_name);
    let fp = crate::cache::native_artifact_cache_key();
    let fresh = |out: &std::path::Path| {
        crate::file_access::exists(out.join(&rlib_name))
            && crate::cache::native_artifact_fingerprint_matches(out, fp)
            // loft#965 — same hole, same cure, on the cross-target path.
            && !native_crate_newer_than(pkg_dir, &out.join(&rlib_name))
    };
    if fresh(&out_dir) {
        return true;
    }
    // Serialise cross-process builds on the SAME global lock the host path uses, so
    // parallel `loft` invocations don't race cargo's shared registry index/cache.
    let _build_lock = crate::file_access::open_with(
        std::env::temp_dir().join("loft-native-build.lock"),
        std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false),
    )
    .ok();
    if let Some(f) = &_build_lock {
        let _ = f.lock();
    }
    if fresh(&out_dir) {
        // A process we waited on just produced it.
        return true;
    }
    let mut cmd = Spawn::new(Program::search("cargo"))
        .args(["build", "--release", "--target", target, "--manifest-path"])
        .arg(&cargo_toml)
        // Build with CLEAN flags: the host `RUSTFLAGS`/`CARGO_ENCODED_RUSTFLAGS` loft was
        // built with are host-target-specific and would either break the wasm build or
        // mis-key it.  A `#native` crate links loft-ffi (the source-stable C-ABI), not
        // loft's rlib, so no shared-SVH flag matching is needed for the wasm leg.
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .stdout(std::process::Stdio::inherit())
        .stderr(std::process::Stdio::inherit());
    match cmd.status() {
        Ok(s) if s.success() => {
            // Stamp the same loft-ffi ABI key so a later loft-ffi change re-builds it.
            crate::cache::write_native_artifact_fingerprint(&out_dir, fp);
            crate::file_access::exists(&rlib)
        }
        Ok(_) => {
            eprintln!(
                "loft: cross-building native library '{stem}' for {target} failed (cargo error \
                 above) — its native crate is likely not wasm-clean.{}",
                build_deps_hint(pkg_dir)
            );
            false
        }
        Err(e) => {
            eprintln!(
                "loft: cannot cross-build native library '{stem}' for {target}: {e} — needs a \
                 Rust toolchain with the {target} target (`rustup target add {target}`)."
            );
            false
        }
    }
}

/// Public API for generated native code that needs to call a cdylib
/// function with a `LoftStore` handle.
///
/// @PLAN12 phase 3.5a (2026-05-24) — added so the `--native` codegen
/// can drive store-allocating cdylib calls (e.g. `n_rand_indices`,
/// `n_load_png`).  The interpreter's `dispatch_via_bridge` does the same
/// thing inline (`make_loft_store` + `CURRENT_STORES` setup); this module
/// exposes the same machinery as a `pub` API constructing
/// `loft_ffi::LoftStore` directly (the interpreter still uses the local
/// `#[repr(C)]` `LoftStore` mirror for its internal dispatch).
///
/// Usage from generated code:
/// ```ignore
/// let _guard = loft::native_call::enter(stores);
/// let ls = loft::native_call::build_store(stores, store_nr);
/// let r = unsafe { external_crate::n_some_fn(ls, arg1, arg2) };
/// // _guard's Drop clears CURRENT_STORES.
/// ```
///
/// Not gated on `native-extensions`, though it sits among the loader's private
/// helpers: that feature buys `dlopen`, and nothing here opens anything — it
/// builds a `LoftStore` over a store loft already owns.  The generated code that
/// calls it links the native crate STATICALLY on `wasm32-wasip2`, where `dlopen`
/// does not exist, so gating it on the loader made `--native-wasm` refuse every
/// program using a `[native] crate` package (loft#967).
#[allow(dead_code)] // Consumed by generated native code (separate compilation unit),
// not by the loft binary itself.  Clippy can't see those callers.
pub mod native_call {
    use crate::database::Stores;

    /// C-ABI callback equivalent to `super::ffi_claim` but typed
    /// against `loft_ffi::LoftStoreCtx` so the resulting
    /// `loft_ffi::LoftStore` can be passed to a generated-code
    /// callsite without `transmute`.
    unsafe extern "C" fn ffi_claim_pub(ctx: loft_ffi::LoftStoreCtx, words: u32) -> u32 {
        let store_nr = ctx._opaque as usize as u16;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::CURRENT_STORES.with(|c| {
                let stores = unsafe { &mut *c.get() };
                let store = &mut stores.allocations[store_nr as usize];
                store.claim(words)
            })
        }))
        .unwrap_or(0)
    }

    unsafe extern "C" fn ffi_resize_pub(ctx: loft_ffi::LoftStoreCtx, rec: u32, words: u32) -> u32 {
        let store_nr = ctx._opaque as usize as u16;
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::CURRENT_STORES.with(|c| {
                let stores = unsafe { &mut *c.get() };
                let store = &mut stores.allocations[store_nr as usize];
                store.resize(rec, words)
            })
        }))
        .unwrap_or(rec)
    }

    unsafe extern "C" fn ffi_reload_pub(
        ctx: loft_ffi::LoftStoreCtx,
        out_ptr: *mut *mut u8,
        out_size: *mut u32,
    ) {
        let store_nr = ctx._opaque as usize as u16;
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::CURRENT_STORES.with(|c| {
                let stores = unsafe { &*c.get() };
                let store = &stores.allocations[store_nr as usize];
                unsafe {
                    *out_ptr = store.base_ptr();
                    *out_size = store.capacity_words();
                }
            });
        }));
    }

    /// The foreign-vector callback for generated code (@PLN174 F5): the same body as
    /// the interpreter's `ffi_foreign`, over the `CURRENT_STORES` cell `enter` set.
    unsafe extern "C" fn ffi_foreign_pub(
        ctx: loft_ffi::LoftStoreCtx,
        ptr: *mut u8,
        len: usize,
        cap: usize,
        elem_size: u32,
        release: unsafe extern "C" fn(*mut u8, usize, usize),
    ) -> loft_ffi::LoftRef {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            super::CURRENT_STORES.with(|c| {
                let stores = unsafe { &mut *c.get() };
                super::adopt_foreign_block(
                    stores,
                    ctx._opaque as usize,
                    ptr,
                    len,
                    cap,
                    elem_size,
                    release,
                )
            })
        }))
        .unwrap_or(loft_ffi::LoftRef {
            store_nr: 0,
            rec: 0,
            pos: 0,
        })
    }

    /// Construct a `loft_ffi::LoftStore` handle pointing at the
    /// store identified by `store_nr`.  Callbacks read from the
    /// `CURRENT_STORES` thread-local — call `enter()` before this
    /// to set it up.
    #[must_use]
    pub fn build_store(stores: &Stores, store_nr: u16) -> loft_ffi::LoftStore {
        build_store_ctx(stores, store_nr, store_nr as usize)
    }

    /// [`build_store`] for the store the generated call MINTED for the bridge's vector
    /// return (@PLN174 F5): the context word carries [`super::CTX_RETURN_STORE`], so a
    /// foreign answer is adopted into that store rather than into a second one.
    #[must_use]
    pub fn build_store_for_return(stores: &Stores, store_nr: u16) -> loft_ffi::LoftStore {
        build_store_ctx(
            stores,
            store_nr,
            store_nr as usize | super::CTX_RETURN_STORE,
        )
    }

    /// @PLN174 F5 — ONE generated cdylib call's heap-typed arguments (`Stores::bridge_args`,
    /// the same set-up the interpreter's dispatcher does): begun before the call with the
    /// arguments in declaration order, asked for what to hand the bridge and for the store
    /// handle, and ended by its drop — after the answer is computed, since a local drops
    /// after the tail expression — which frees the copies and refuses a write through one.
    pub struct BridgeGuard<'a> {
        cell: &'a std::cell::UnsafeCell<Stores>,
        args: Option<crate::database::BridgeArgs>,
        returns_ref: bool,
    }

    impl<'a> BridgeGuard<'a> {
        #[must_use]
        pub fn begin(
            cell: &'a std::cell::UnsafeCell<Stores>,
            stores: &mut Stores,
            args: &[(crate::keys::DbRef, bool)],
            returns_ref: bool,
        ) -> Self {
            Self {
                cell,
                args: Some(stores.bridge_args(args, returns_ref)),
                returns_ref,
            }
        }

        /// What the bridge is handed for the `i`th heap-typed argument.
        #[must_use]
        pub fn handed(&self, i: usize) -> crate::keys::DbRef {
            self.args
                .as_ref()
                .and_then(|a| a.handed.get(i).copied())
                .unwrap_or(crate::keys::DbRef::NULL)
        }

        /// The store handle for the call: the pinned store, or the one minted for it,
        /// announced as the return store ([`super::CTX_RETURN_STORE`]).
        #[must_use]
        pub fn store(&self, stores: &Stores) -> loft_ffi::LoftStore {
            let (nr, minted) = self
                .args
                .as_ref()
                .map_or((0, false), |a| (a.store_nr, a.minted));
            if minted {
                build_store_for_return(stores, nr)
            } else {
                build_store(stores, nr)
            }
        }
    }

    impl Drop for BridgeGuard<'_> {
        fn drop(&mut self) {
            if let Some(args) = self.args.take() {
                // SAFETY: generated code holds no other live borrow of the stores across a
                // local's drop (the same cell discipline `FnRefBufGuard` relies on).
                let stores = unsafe { &mut *self.cell.get() };
                stores.bridge_args_done(&args, self.returns_ref);
            }
        }
    }

    fn build_store_ctx(stores: &Stores, store_nr: u16, ctx: usize) -> loft_ffi::LoftStore {
        let store = stores.store(&crate::keys::DbRef {
            store_nr,
            rec: 0,
            pos: 0,
        });
        loft_ffi::LoftStore {
            ptr: store.base_ptr(),
            size: store.capacity_words(),
            ctx: loft_ffi::LoftStoreCtx {
                _opaque: ctx as *mut (),
            },
            claim_fn: Some(ffi_claim_pub),
            reload_fn: Some(ffi_reload_pub),
            resize_fn: Some(ffi_resize_pub),
            foreign_fn: Some(ffi_foreign_pub),
        }
    }

    /// RAII guard: sets `CURRENT_STORES` on entry, clears on drop.
    /// One per cdylib call; nested calls are NOT supported (would
    /// stomp the thread-local).  Native codegen emits one per
    /// call-site, scoped via `let _guard = native_call::enter(stores);`.
    pub struct StoresGuard {
        _priv: (),
    }

    /// Set the thread-local current stores pointer for the
    /// duration of the returned guard.  Generated code:
    /// `let _guard = native_call::enter(stores);` immediately
    /// before a cdylib call.
    pub fn enter(stores: &mut Stores) -> StoresGuard {
        let ptr = std::ptr::from_mut::<Stores>(stores);
        super::CURRENT_STORES.with(|c| c.set(ptr));
        StoresGuard { _priv: () }
    }

    impl Drop for StoresGuard {
        fn drop(&mut self) {
            super::CURRENT_STORES.with(|c| c.set(std::ptr::null_mut()));
        }
    }
}

// @PLN21 Phase 3 — the dlopen-failure classifier turns raw linker errors into
// actionable guidance.  Pure string-in/string-out, so unit-testable.
#[cfg(all(test, feature = "native-extensions"))]
mod dlopen_diag_tests {
    use super::build_deps_hint;
    use crate::c_libs::dlopen_diagnostic;
    use crate::native_build::{first_missing_runtime_lib, runtime_lib_missing_diagnostic};

    fn temp_pkg(name: &str, toml: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("loft_p21_{name}_{}", std::process::id()));
        let _ = crate::file_access::create_dir_all(&dir);
        crate::file_access::write(dir.join("loft.toml"), toml).unwrap();
        dir
    }

    // A library-name that targets THIS host (so it's probed) and one that targets
    // another OS (so it's skipped) — derived from the runner's OS so the runtime-
    // lib tests hold on every CI leg (ubuntu/macos/windows), not just Linux.
    fn host_and_foreign_lib_names() -> (&'static str, &'static str) {
        match crate::platform::host_lib_os() {
            crate::platform::LibOs::Macos => ("libnot-real-skip.dylib", "libnot-real-skip.so.7"),
            crate::platform::LibOs::Windows => ("not-real-skip.dll", "libnot-real-skip.so.7"),
            crate::platform::LibOs::Linux => ("libnot-real-skip.so.7", "libnot-real-skip.dylib"),
        }
    }

    #[test]
    fn missing_declared_runtime_lib_is_detected() {
        // A host-applicable name that isn't installed must be detected.
        let (host_missing, _) = host_and_foreign_lib_names();
        let dir = temp_pkg(
            "rtlib",
            &format!("[native]\ncrate = \"x\"\nruntime-libs = \"{host_missing}\"\n"),
        );
        assert_eq!(
            first_missing_runtime_lib(dir.to_str().unwrap()).as_deref(),
            Some(host_missing)
        );
        // none declared → no check, no false positive.
        let dir2 = temp_pkg("nort", "[native]\ncrate = \"x\"\n");
        assert!(first_missing_runtime_lib(dir2.to_str().unwrap()).is_none());
    }

    #[test]
    fn first_missing_runtime_lib_skips_foreign_platform_names() {
        // The macOS-on-`libGL.so.1` case: a foreign-OS name can never load here and
        // is NOT a host requirement, so it is skipped — not treated as missing
        // (which would wrongly hard-fail the whole library).
        let (host_missing, foreign) = host_and_foreign_lib_names();
        let f = temp_pkg(
            "foreignrt",
            &format!("[native]\ncrate = \"x\"\nruntime-libs = \"{foreign}\"\n"),
        );
        assert!(first_missing_runtime_lib(f.to_str().unwrap()).is_none());
        // foreign (skipped) + a missing host name (probed) → the host one is still
        // detected.
        let mixed = temp_pkg(
            "mixrt",
            &format!("[native]\ncrate = \"x\"\nruntime-libs = \"{foreign}, {host_missing}\"\n"),
        );
        assert_eq!(
            first_missing_runtime_lib(mixed.to_str().unwrap()).as_deref(),
            Some(host_missing)
        );
    }

    #[test]
    fn runtime_lib_diag_keeps_install_advice_for_host_targeted_name() {
        // A `.so` on Linux is the right format — genuinely not installed.
        let m = runtime_lib_missing_diagnostic("audio", "libasound.so.2");
        assert!(
            m.contains("needs the system library 'libasound.so.2'"),
            "{m}"
        );
        assert!(m.contains("install it with your OS package manager"), "{m}");
    }

    #[test]
    fn build_deps_hint_names_declared_deps() {
        let dir = temp_pkg(
            "bdeps",
            "[native]\ncrate = \"x\"\nbuild-deps = \"libgl-dev, libasound2-dev\"\n",
        );
        let h = build_deps_hint(dir.to_str().unwrap());
        assert!(
            h.contains("libgl-dev") && h.contains("libasound2-dev"),
            "{h}"
        );
        let unexpected = build_deps_hint(temp_pkg("nobd", "[native]\n").to_str().unwrap());
        assert!(unexpected.is_empty(), "{unexpected:?}");
    }

    #[test]
    fn missing_system_lib_names_it_and_says_install() {
        let m = dlopen_diagnostic(
            "/x/libgraphics.so",
            "libasound.so.2: cannot open shared object file: No such file or directory",
        );
        assert!(m.contains("libasound.so.2"), "names the missing lib: {m}");
        assert!(
            m.contains("SYSTEM library") && m.contains("install"),
            "actionable, not a raw linker error: {m}"
        );
    }

    #[test]
    fn glibc_too_old_is_flagged() {
        let m = dlopen_diagnostic("/x/lib.so", "/x/lib.so: version `GLIBC_2.38' not found");
        assert!(m.to_ascii_lowercase().contains("glibc"), "{m}");
    }

    #[test]
    fn undefined_symbol_is_abi_mismatch() {
        let m = dlopen_diagnostic("/x/lib.so", "undefined symbol: n_foo");
        assert!(m.contains("ABI mismatch") || m.contains("loft-ffi"), "{m}");
    }
}

#[cfg(all(test, feature = "native-extensions"))]
mod c_lib_beside_tests {
    use crate::platform::{LibOs, existing_lib_beside};

    /// A package that ships its own library declares a PATH, and its Makefile
    /// builds the HOST's spelling — `liblc_types.dylib` on macOS for a
    /// `../../liblc_types.so` declaration. Both the loader and the link line
    /// have to find it; the link line did not, so macOS emitted no `-L` and no
    /// rpath and sent `-l dylib=lc_types` to the system search path.
    ///
    /// Run from any host by passing the convention in — the point of taking
    /// `LibOs` as an argument. Left implicit this could only ever be checked on
    /// a Mac, which is exactly how it stayed broken.
    #[test]
    fn the_host_spelling_beside_a_package_is_found() {
        let dir = std::env::temp_dir().join(format!("loft_beside_{}", std::process::id()));
        let sub = dir.join("pkg");
        crate::file_access::create_dir_all(&sub).expect("probe dir");
        // Only the macOS artefact exists, as on a Mac that just ran `make`.
        crate::file_access::write(dir.join("liblc_types.dylib"), b"x").expect("write probe");

        assert_eq!(
            existing_lib_beside(&sub, "../liblc_types.so", LibOs::Macos).as_deref(),
            Some("../liblc_types.dylib"),
            "macOS must fall back to the .dylib the Makefile actually built"
        );
        assert_eq!(
            existing_lib_beside(&sub, "../liblc_types.so", LibOs::Linux),
            None,
            "Linux must NOT invent one — the declared .so is genuinely absent"
        );

        // With the declared spelling present, it wins on every host: the
        // fallback may only ever ADD a candidate.
        crate::file_access::write(dir.join("liblc_types.so"), b"x").expect("write probe");
        for os in [LibOs::Linux, LibOs::Macos, LibOs::Windows] {
            assert_eq!(
                existing_lib_beside(&sub, "../liblc_types.so", os).as_deref(),
                Some("../liblc_types.so"),
                "the declared spelling is authoritative when it is there"
            );
        }
        let _ = crate::file_access::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod native_freshness_tests {
    use super::native_crate_newer_than;
    use std::io::Write;

    /// Build `<root>/native/src/lib.rs` plus an artifact, and return both paths.
    fn fixture(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("loft_965_{tag}_{}", std::process::id()));
        let _ = crate::file_access::remove_dir_all(&root);
        crate::file_access::create_dir_all(root.join("native/src")).expect("mkdir");
        crate::file_access::create_dir_all(root.join("native/target/release"))
            .expect("mkdir target");
        let src = root.join("native/src/lib.rs");
        crate::file_access::create(&src)
            .and_then(|mut f| f.write_all(b"// crate source\n"))
            .expect("write source");
        (root, src)
    }

    /// Write the artifact AFTER the sources, which is what a real build leaves behind.
    fn artifact_after(root: &std::path::Path, name: &str) -> std::path::PathBuf {
        // A coarse filesystem timestamp would make "written after" indistinguishable
        // from "written at the same moment", and the comparison is `>`.  Sleep past the
        // granularity rather than assert on a tie whose direction is the platform's
        // choice.
        std::thread::sleep(std::time::Duration::from_millis(20));
        let art = root.join(name);
        crate::file_access::write(&art, b"artifact").expect("write artifact");
        art
    }

    /// The reuse case: sources older than the artifact, so the cached cdylib stands.
    #[test]
    fn an_untouched_crate_is_not_newer() {
        let (root, _src) = fixture("fresh");
        let art = artifact_after(&root, "libx.so");
        assert!(!native_crate_newer_than(&root.to_string_lossy(), &art));
        let _ = crate::file_access::remove_dir_all(&root);
    }

    /// loft#965 itself: the crate's own Rust edited after the artifact was built.  This
    /// is what `loft test` used to answer "reuse" to, running the previous build against
    /// code no longer in the tree.
    #[test]
    fn an_edited_crate_source_is_newer() {
        let (root, src) = fixture("edited");
        let art = artifact_after(&root, "libx.so");
        std::thread::sleep(std::time::Duration::from_millis(20));
        crate::file_access::create(&src)
            .and_then(|mut f| f.write_all(b"// edited\n"))
            .expect("re-write source");
        assert!(native_crate_newer_than(&root.to_string_lossy(), &art));
        let _ = crate::file_access::remove_dir_all(&root);
    }

    /// ⚠ `native/target/` is the build's OWN output and is always newer than the
    /// artifact's siblings.  Counting it would report every crate as stale forever —
    /// a rebuild on every run, which is the failure mode a careless fix has.
    #[test]
    fn the_crates_own_target_dir_does_not_count() {
        let (root, _src) = fixture("target");
        let art = artifact_after(&root, "libx.so");
        std::thread::sleep(std::time::Duration::from_millis(20));
        crate::file_access::write(root.join("native/target/release/stamp"), b"x")
            .expect("write stamp");
        assert!(
            !native_crate_newer_than(&root.to_string_lossy(), &art),
            "output under native/target must not make the crate look stale"
        );
        let _ = crate::file_access::remove_dir_all(&root);
    }

    /// A package with no native crate at all answers "not newer" rather than walking
    /// nothing and defaulting to a rebuild.
    #[test]
    fn a_package_without_a_native_crate_is_not_newer() {
        let root = std::env::temp_dir().join(format!("loft_965_none_{}", std::process::id()));
        let _ = crate::file_access::remove_dir_all(&root);
        crate::file_access::create_dir_all(&root).expect("mkdir");
        let art = artifact_after(&root, "libx.so");
        assert!(!native_crate_newer_than(&root.to_string_lossy(), &art));
        let _ = crate::file_access::remove_dir_all(&root);
    }
}
