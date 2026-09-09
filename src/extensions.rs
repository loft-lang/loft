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

/// Load all pending native extension libraries.
#[cfg(feature = "native-extensions")]
use std::collections::HashMap;
// Every registry this guards is itself `native-extensions`-only, so the import follows the
// same gate — without it a `--no-default-features` build warns on an unused import, which is
// noise in the one gate whose whole job is to be read.
#[cfg(feature = "native-extensions")]
use std::sync::Mutex;

/// Wrapper for `*const ()` that is Send — function pointers from cdylibs are
/// valid for the process lifetime (the Library handle is leaked).
#[cfg(feature = "native-extensions")]
#[derive(Clone, Copy)]
struct FnPtr(*const ());
#[cfg(feature = "native-extensions")]
unsafe impl Send for FnPtr {}

/// Global registry of native function pointers loaded from cdylibs.
#[cfg(feature = "native-extensions")]
static NATIVE_REGISTRY: Mutex<Option<HashMap<String, FnPtr>>> = Mutex::new(None);

/// Plan-74: registry of generated marshal bridges (`loft_ffi::LoftBridgeFn`),
/// keyed by loft symbol.  Populated from a cdylib's `loft_register_bridges_v1`.
/// Every native call dispatches through its bridge (`dispatch_via_bridge`); a
/// symbol absent here is an un-migrated cdylib (a pre-bridge library version)
/// and panics at call time — the legacy raw-ptr marshaller it used to fall back
/// to has been removed now that every native library ships bridges.
#[cfg(feature = "native-extensions")]
static BRIDGE_REGISTRY: Mutex<Option<HashMap<String, FnPtr>>> = Mutex::new(None);

/// Look up a generated bridge for `sym`, if the owning cdylib registered one.
#[cfg(feature = "native-extensions")]
fn get_bridge(sym: &str) -> Option<*const ()> {
    BRIDGE_REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|m| m.get(sym))
        .map(|p| p.0)
}

/// The C-ABI registration callback type.
#[cfg(feature = "native-extensions")]
type RegisterFn =
    unsafe extern "C" fn(unsafe extern "C" fn(*const u8, usize, *const (), *mut ()), *mut ());

/// The registration callback: called once per symbol by the cdylib.
#[cfg(feature = "native-extensions")]
unsafe extern "C" fn collect(
    name_ptr: *const u8,
    name_len: usize,
    fn_ptr: *const (),
    ctx: *mut (),
) {
    let collected = unsafe { &mut *ctx.cast::<Vec<(String, *const ())>>() };
    let name = std::str::from_utf8(unsafe { std::slice::from_raw_parts(name_ptr, name_len) })
        .unwrap_or("<invalid>");
    collected.push((name.to_string(), fn_ptr));
}

#[cfg(feature = "native-extensions")]
pub fn load_all(_state: &mut crate::state::State, paths: Vec<String>) {
    for path in paths {
        load_one(&path);
    }
}

#[cfg(not(feature = "native-extensions"))]
pub fn load_all(_state: &mut crate::state::State, _paths: Vec<String>) {}

/// Load ONE cdylib, ahead of the rest, and report whether it is loaded.
///
/// Same call `load_all` makes, exposed so a caller can prove an artifact before
/// committing to dispatch through it (loft#831): the artifact existing, being
/// fresh and declaring the right layout are all proxies, and each of them can be
/// true of a library this process cannot `dlopen` — built against a different
/// `libloft.rlib`, or replaced by a concurrent build between the check and the
/// load.  Loading is the only question whose answer is the one that matters.
///
/// It is also what makes the answer STAY true: the handle is kept for the
/// process, so an artifact another process prunes or rebuilds afterwards cannot
/// change what this one dispatches to.  Idempotent — `load_all` calling it again
/// for the same path is a no-op.
#[cfg(feature = "native-extensions")]
pub fn load_cdylib(path: &str) -> bool {
    load_one(path)
}

#[cfg(not(feature = "native-extensions"))]
pub fn load_cdylib(_path: &str) -> bool {
    false
}

/// Whether `sym` resolves in a cdylib this process has loaded — the exact
/// question `wire_shared_native_fns` asks later, asked early enough to still
/// have a choice (loft#831).
#[cfg(feature = "native-extensions")]
#[must_use]
pub fn bridge_symbol_resolves(sym: &str) -> bool {
    try_dlsym(sym).is_some()
}

#[cfg(not(feature = "native-extensions"))]
#[must_use]
pub fn bridge_symbol_resolves(_sym: &str) -> bool {
    false
}

/// Loaded libraries kept alive for the process lifetime, each paired with
/// whether it exported `loft_register_v1` (the registration protocol).  Used by
/// `try_dlsym` to look up symbols from previously loaded cdylibs *and* report
/// whether the resolving library opted into registration — so the "unregistered
/// symbol" guard fires only for a library that chose the protocol, not for a
/// (legitimately) zero-registration cdylib loaded after one that did.
#[cfg(feature = "native-extensions")]
static LOADED_LIBS: Mutex<Vec<(libloading::Library, bool)>> = Mutex::new(Vec::new());

/// @PLN21 Phase 3 — turn a raw `dlopen`/`LoadLibrary` failure into an
/// actionable message.  The dynamic linker IS the authoritative validator — its
/// error text names exactly what is wrong — so we classify it rather than guess.
/// A missing RUNTIME system lib is terminal: building from source links the same
/// lib and fails identically (decision C3), so the user must install it, not
/// rebuild.
#[cfg(feature = "native-extensions")]
fn dlopen_diagnostic(path: &str, err: &str) -> String {
    let lower = err.to_ascii_lowercase();
    // Missing shared-object dependency: linux "cannot open shared object file";
    // macOS "image not found"; windows "specified module could not be found".
    if lower.contains("cannot open shared object file")
        || lower.contains("image not found")
        || lower.contains("specified module could not be found")
    {
        // linux names the missing lib before the first ':'
        // ("libasound.so.2: cannot open shared object file …").
        let named = err.split(':').next().unwrap_or("").trim();
        let lib = if named.is_empty() || named == path {
            "a system library"
        } else {
            named
        };
        return format!(
            "loft: native library '{path}' needs {lib} at runtime, but it is not installed — \
             this is a SYSTEM library; install it with your OS package manager (building from \
             source would link the same library and fail identically)."
        );
    }
    if lower.contains("glibc_") && lower.contains("not found") {
        return format!(
            "loft: native library '{path}' was built against a newer glibc than this system \
             provides ({err}) — update the system, or build the library from source for this host."
        );
    }
    if lower.contains("undefined symbol") {
        return format!(
            "loft: native library '{path}' has an ABI mismatch ({err}) — built against a \
             different loft-ffi; it will be rebuilt from source."
        );
    }
    format!("loft: cannot load native extension '{path}': {err}")
}

/// Load a single native extension shared library.
///
/// If the library exports `loft_register_v1`, calls it to collect all symbols.
/// Otherwise, the library is kept loaded and individual symbols will be
/// resolved on demand via `try_dlsym` during `wire_native_fns`.
#[cfg(feature = "native-extensions")]
/// Returns whether the library is loaded when this returns — false only when
/// `dlopen` itself refused it.
///
/// Existing on disk is not the same fact as loading: an ELF `.so` left in a
/// tree by a Linux build is a file macOS cannot map, and a caller that reads
/// "the path is there" as "the symbols are there" reports success and then
/// fails at the first `#c` call, naming the symbol rather than the library
/// (loft#739's neighbour — see `load_c_library`, which used to do exactly this).
fn load_one(path: &str) -> bool {
    use libloading::Library;
    use std::collections::HashSet;

    static LOAD_LOCK: Mutex<Option<HashSet<String>>> = Mutex::new(None);

    let canonical = crate::portable_path::plain_canonical_str(path);

    let mut guard = LOAD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let loaded = guard.get_or_insert_with(HashSet::new);
    if loaded.contains(&canonical) {
        return true;
    }

    let lib = match unsafe { Library::new(path) } {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}", dlopen_diagnostic(path, &e.to_string()));
            return false;
        }
    };

    // Try the registration protocol first.  Records whether this library opted
    // into it — `try_dlsym` reports the flag so the unregistered-symbol guard
    // only fires for a library that chose the protocol (issue #119), not for a
    // zero-registration cdylib that legitimately relies on dlsym.
    let mut uses_v1 = false;
    if let Ok(register_sym) = unsafe { lib.get::<RegisterFn>(b"loft_register_v1\0") } {
        uses_v1 = true;
        let register_sym = *register_sym;
        let mut collected: Vec<(String, *const ())> = Vec::new();
        unsafe {
            register_sym(collect, std::ptr::addr_of_mut!(collected).cast::<()>());
        }
        let mut reg_guard = NATIVE_REGISTRY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let registry = reg_guard.get_or_insert_with(HashMap::new);
        for (name, ptr) in collected {
            registry.insert(name, FnPtr(ptr));
        }
    }
    // Plan-25: collect generated marshal bridges, if the cdylib exports them.
    // Symbols with a bridge dispatch through `dispatch_via_bridge`; the rest
    // keep using the legacy raw-ptr arms (a library may export both during the
    // transition, or only `loft_register_v1`).
    if let Ok(reg_sym) = unsafe { lib.get::<RegisterFn>(b"loft_register_bridges_v1\0") } {
        let reg_sym = *reg_sym;
        let mut collected: Vec<(String, *const ())> = Vec::new();
        unsafe {
            reg_sym(collect, std::ptr::addr_of_mut!(collected).cast::<()>());
        }
        let mut bridge_guard = BRIDGE_REGISTRY
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let registry = bridge_guard.get_or_insert_with(HashMap::new);
        for (name, ptr) in collected {
            registry.insert(name, FnPtr(ptr));
        }
    }
    // Either way, keep the library loaded for potential dlsym lookups.
    loaded.insert(canonical);
    LOADED_LIBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push((lib, uses_v1));
    true
}

/// Try to resolve a symbol by name from any loaded cdylib.  Returns the symbol
/// pointer paired with whether the resolving library opted into
/// `loft_register_v1`.  Called by `wire_native_fns` as a fallback when the symbol
/// wasn't provided via the registry. This enables zero-registration cdylibs:
/// just export `#[unsafe(no_mangle)] pub extern "C" fn n_my_func(...)`.
#[cfg(feature = "native-extensions")]
fn try_dlsym(name: &str) -> Option<(*const (), bool)> {
    let libs = LOADED_LIBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut sym_name = name.to_string();
    sym_name.push('\0');
    for (lib, uses_v1) in libs.iter() {
        if let Ok(sym) = unsafe { lib.get::<*const ()>(sym_name.as_bytes()) } {
            return Some((*sym, *uses_v1));
        }
    }
    None
}

/// @PLN24 arc D — `dlopen` a C library a package declared with `[c] libs`, and
/// KEEP it loaded so its symbols resolve.
///
/// Distinct from the `[native] runtime-libs` probe, which opens a library only
/// to ask whether it exists and drops the handle. Here the handle is the point:
/// a `#c` symbol is looked up through it. Loading is idempotent (`load_one`
/// keys on the canonical path), so a library named by several packages opens
/// once.
///
/// A relative name resolves against the declaring package's directory first, so
/// a library can ship its own `.so`; otherwise it goes to the dynamic linker by
/// soname, exactly as `runtime-libs` does. Returns whether it loaded — a
/// missing library is the caller's to report, with the binding that needed it.
///
/// The declared spelling is tried first and unchanged, then the same library
/// under the HOST's naming ([`host_lib_variants`]). A manifest carries one
/// string, and every `[c] libs` in this repo and the registry spells it the
/// Linux way (`libmariadb.so.3`), so without the fallback the whole of @PLN24 is
/// Linux-only: the fixture's `../../liblc_types.so` is built as
/// `liblc_types.dylib` by its own Makefile on macOS, so the declaration pointed
/// at a file that could not exist and every `#c` symbol in it went unresolved.
#[cfg(feature = "native-extensions")]
pub fn load_c_library(name: &str, pkg_dir: &str) -> bool {
    let mut last_err = None;
    for cand in host_lib_variants(name) {
        let beside = std::path::Path::new(pkg_dir).join(&cand);
        // `load_one`, not `exists()`: a file that will not `dlopen` (a Linux
        // `.so` sitting in the tree on macOS) must fall through to the next
        // candidate rather than be reported as loaded.
        if beside.exists() && load_one(&beside.to_string_lossy()) {
            return true;
        }
        // Not a path we can see: hand the soname to the dynamic linker, which
        // knows the search path we do not.
        match unsafe { libloading::Library::new(&cand) } {
            Ok(_) => {
                load_one(&cand);
                return true;
            }
            Err(e) => last_err = Some((cand, e)),
        }
    }
    // @PLN24 arc G — an OPTIONAL library that does not open is an ordinary
    // answer (`c_library_available` says false and the program takes its
    // fallback), so this must not print by default. But "not installed" and
    // "installed and unloadable" are very different problems with the same
    // symptom, and only the linker's own text tells them apart.
    if std::env::var_os("LOFT_C_DEBUG").is_some()
        && let Some((cand, e)) = last_err
    {
        eprintln!("loft: `[c]` library '{name}' did not open (last tried '{cand}'): {e}");
    }
    false
}

/// The declared library name, then the same library spelled for THIS host.
///
/// A `[c] libs` entry is one string in a manifest, so it cannot say `.so` on
/// Linux and `.dylib` on macOS. Rather than invent a per-platform manifest key,
/// translate at load time: the declared spelling is authoritative and always
/// tried first, and these are what to try when the host does not use it.
///
/// The versioned forms matter as much as the bare one — a real declaration is
/// `libmariadb.so.3`, whose macOS twin is `libmariadb.3.dylib` (the soversion
/// moves BEFORE the extension) and whose Windows twin drops both the `lib`
/// prefix and the version. Returns just the declared name on Linux, where the
/// spelling already is the host's.
#[cfg(feature = "native-extensions")]
fn host_lib_variants(name: &str) -> Vec<String> {
    crate::platform::host_lib_variants(name)
}

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
        std::fs::create_dir_all(&dir).expect("probe dir");
        let bogus = dir.join("libnotanelf.so");
        std::fs::write(&bogus, b"this is not a shared object").expect("write probe");
        assert!(
            !super::load_c_library("libnotanelf.so", &dir.to_string_lossy()),
            "a path that exists but will not dlopen must not count as loaded"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// @PLN24 arc B — the same lookup, for the `#c` caller: a C binding resolves
/// against the cdylibs loft has already loaded before it falls back to the
/// process. One resolver, so a symbol cannot mean two different things
/// depending on which caller asked for it.
#[cfg(feature = "native-extensions")]
#[must_use]
pub fn try_dlsym_pub(name: &str) -> Option<(*const (), bool)> {
    try_dlsym(name)
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
}

/// Compact native signature: parameter types + return type.
#[cfg(feature = "native-extensions")]
#[derive(Clone, Debug)]
struct NativeSig {
    params: Vec<ArgT>,
    ret: Option<ArgT>,
}

/// Side table: library index → (native symbol name, signature).
/// Populated by `wire_native_fns`, read by the generic dispatcher.
#[cfg(feature = "native-extensions")]
static NATIVE_SIGS: Mutex<Option<HashMap<u16, (String, NativeSig)>>> = Mutex::new(None);

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
static SHARED_SIGS: Mutex<Option<HashMap<u16, (FnPtr, SharedSig)>>> = Mutex::new(None);

/// loft#715 — library slot → the bridge symbol it dispatches to.  Populated once
/// at wiring; read only when a fault is raised while a bridge call is on the
/// stack, so a bridge fault stops reading like a program fault.
#[cfg(feature = "native-extensions")]
static SHARED_LABELS: Mutex<Option<HashMap<u16, String>>> = Mutex::new(None);

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
        _ => return None,
    })
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
                pops.push(marshal_arg_t(&attr.typedef)?);
                forward.push(true);
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
            if data.wasm_bridge_routes.contains_key(sym) {
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
        if get_bridge(sym).is_none() {
            bridgeless.push(sym.clone());
            continue;
        }

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

        // Store the signature for runtime dispatch.
        sig_table.insert(lib_idx, (sym.clone(), sig));

        // Replace the stub with the generic auto-marshal dispatcher.
        state.replace_static_fn(sym, native_auto_dispatch);
    }

    if !unresolved.is_empty() {
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
/// aborts; the matching panic stub still fires if the function is actually called,
/// but the operator has already seen which library to rebuild.
#[cfg(feature = "native-extensions")]
fn report_unresolved_natives(data: &crate::data::Data, unresolved: &[String]) {
    use std::collections::BTreeMap;
    let mut by_crate: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for sym in unresolved {
        let krate = data
            .native_symbol_crates
            .get(sym)
            .map_or("<unknown library>", String::as_str);
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
             unavailable and will panic if called ({shown}{more_txt}). Its cdylib is missing or \
             stale (commonly built against a different libloft.rlib / loft-ffi). Rebuild it with \
             `make rebuild-native-cdylibs` (in the loft tree) or `cargo build --release` in the \
             library's native/ dir, then re-run.",
            n = syms.len(),
        );
    }
}

#[cfg(not(feature = "native-extensions"))]
pub fn wire_native_fns(_state: &mut crate::state::State, _data: &crate::data::Data) {}

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

/// Ask the dynamic loader what the function at `ptr` is called.
///
/// Only an EXACT hit counts: `dladdr` reports the nearest preceding symbol when
/// the address falls inside one, and a near miss would hand codegen a `#[link_name]`
/// for a neighbouring function. Requiring `dli_saddr == ptr` turns that into a
/// `None` (leave the symbol alone), which is the pre-loft#907 behaviour.
#[cfg(all(feature = "native-extensions", unix))]
fn exported_name_at(ptr: *const ()) -> Option<String> {
    let mut info: libc::Dl_info = unsafe { std::mem::zeroed() };
    if unsafe { libc::dladdr(ptr.cast(), &raw mut info) } == 0 {
        return None;
    }
    if info.dli_sname.is_null() || !std::ptr::eq(info.dli_saddr.cast_const().cast::<()>(), ptr) {
        return None;
    }
    unsafe { std::ffi::CStr::from_ptr(info.dli_sname) }
        .to_str()
        .ok()
        .map(str::to_string)
}

/// The Windows half of [`exported_name_at`] (loft#972).
///
/// There is no `dladdr` here, so the module's own PE export table answers instead:
/// `GetModuleHandleExW(FROM_ADDRESS)` names the module the pointer lives in — and an
/// `HMODULE` **is** that module's mapped base — then the export directory is walked for
/// the export whose address equals the pointer.
///
/// Without it `--native` and `--interpret` called DIFFERENT functions on Windows: no
/// remap was ever recorded, so codegen linked the `#native` string literally, which is
/// the name a library that remaps (published `graphics`, for `save_png`) does NOT
/// implement.
///
/// The loader is asked rather than the registration because the bridge's own identifier
/// never crosses the ABI — `loft_register_bridges!` passes `(loft symbol, fn pointer)`,
/// and the name is a macro token. Fixing it there instead would mean an ABI addition and
/// a republish of every native library before any of them stopped mis-linking.
///
/// `UNCHANGED_REFCOUNT`: this only reads the module, so it must not pin it loaded.
#[cfg(all(feature = "native-extensions", windows))]
fn exported_name_at(ptr: *const ()) -> Option<String> {
    const FROM_ADDRESS: u32 = 0x0000_0004;
    const UNCHANGED_REFCOUNT: u32 = 0x0000_0002;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetModuleHandleExW(
            flags: u32,
            module_name: *const u16,
            module: *mut *mut core::ffi::c_void,
        ) -> i32;
    }

    let mut handle: *mut core::ffi::c_void = std::ptr::null_mut();
    if unsafe {
        GetModuleHandleExW(
            FROM_ADDRESS | UNCHANGED_REFCOUNT,
            ptr.cast::<u16>(),
            &raw mut handle,
        )
    } == 0
    {
        return None;
    }
    let base = handle.cast::<u8>();
    if base.is_null() {
        return None;
    }
    // Every read below is an offset from the mapped base, and each is bounded by the
    // header field that precedes it — a module whose headers do not parse yields `None`
    // rather than a guess, which is the same answer the pre-loft#907 path gave.
    let rd32 = |off: usize| -> u32 { unsafe { base.add(off).cast::<u32>().read_unaligned() } };
    let rd16 = |off: usize| -> u16 { unsafe { base.add(off).cast::<u16>().read_unaligned() } };
    if rd16(0) != 0x5A4D {
        return None; // not `MZ` — not a PE image
    }
    let pe = rd32(0x3C) as usize;
    if rd32(pe) != 0x0000_4550 {
        return None; // not the PE signature `P`,`E`,NUL,NUL
    }
    // The export directory's RVA sits at a different offset in PE32 vs PE32+, and the
    // magic in the optional header is what tells them apart.
    let opt = pe + 24;
    let export_rva = match rd16(opt) {
        0x20B => rd32(opt + 112) as usize, // PE32+
        0x10B => rd32(opt + 96) as usize,  // PE32
        _ => return None,
    };
    let export_size = match rd16(opt) {
        0x20B => rd32(opt + 116) as usize,
        _ => rd32(opt + 100) as usize,
    };
    if export_rva == 0 {
        return None; // the module exports nothing
    }
    let names = rd32(export_rva + 32) as usize;
    let name_count = rd32(export_rva + 24) as usize;
    let functions = rd32(export_rva + 28) as usize;
    let ordinals = rd32(export_rva + 36) as usize;
    for i in 0..name_count {
        let ordinal = rd16(ordinals + i * 2) as usize;
        let func_rva = rd32(functions + ordinal * 4) as usize;
        // An RVA inside the export directory is a FORWARDER string, not code — it names
        // another module's export and has no address here.
        if func_rva >= export_rva && func_rva < export_rva + export_size {
            continue;
        }
        if !std::ptr::eq(unsafe { base.add(func_rva) }.cast::<()>().cast_const(), ptr) {
            continue;
        }
        // Exact hit. Mirrors the `dli_saddr == ptr` requirement on unix: a near miss
        // would hand codegen a `#[link_name]` for a neighbouring function.
        let name_ptr = unsafe { base.add(rd32(names + i * 4) as usize) };
        return unsafe { std::ffi::CStr::from_ptr(name_ptr.cast()) }
            .to_str()
            .ok()
            .map(str::to_string);
    }
    None
}

#[cfg(all(feature = "native-extensions", not(unix), not(windows)))]
fn exported_name_at(_ptr: *const ()) -> Option<String> {
    None
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
        let Some((ptr, _uses_v1)) = try_dlsym(sym) else {
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
            table.insert(*lib_idx, (FnPtr(*ptr), sig.clone()));
        }
    }
    // loft#715 — remember which symbol each slot dispatches to, so a fault raised
    // while the bridge is on the stack can NAME it.  Written once at wiring, read
    // only on the panic path.
    {
        let mut guard = SHARED_LABELS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let table = guard.get_or_insert_with(HashMap::new);
        for (sym, lib_idx, ..) in &wired {
            table.insert(*lib_idx, sym.clone());
        }
    }
    for (sym, ..) in &wired {
        state.replace_static_fn(sym, shared_store_dispatch);
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
    use crate::keys::DbRef;
    use crate::native_lib::LibArg;

    crate::state::SHARED_DISPATCH_HITS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let lib_idx = CURRENT_LIB_IDX.with(std::cell::Cell::get);
    // loft#715 — mark that a bridge call is on the stack.  A fault raised from
    // here is a BRIDGE fault, and it used to read exactly like a program fault:
    // `Cannot add to none-structure '<type>'` names a type, never the library
    // that supplied the index, and the abort takes the whole process with it.
    // One `Cell` write per call, cleared on the way out.
    IN_SHARED_BRIDGE.with(|c| c.set(lib_idx.wrapping_add(1)));
    let (bridge_ptr, sig) = {
        let guard = SHARED_SIGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let table = guard.as_ref().expect("SHARED_SIGS not initialized");
        match table.get(&lib_idx) {
            Some((p, s)) => (p.0, s.clone()),
            None => panic!("no shared signature for lib_idx {lib_idx}"),
        }
    };

    // Pop ONE stack slot per attribute (the call site pushes every attribute,
    // including hidden dests and the text work buffer) — in reverse (stack is
    // LIFO), then restore declaration order.
    let mut popped: Vec<LibArg> = Vec::with_capacity(sig.pops.len());
    for &t in sig.pops.iter().rev() {
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
        };
        popped.push(slot);
    }
    popped.reverse();
    // Forward only the marshalled slots (compacted order — the bridge reads
    // `a[0..n]` skipping work buffers and hidden dests it satisfies locally).
    let args: Vec<LibArg> = popped
        .iter()
        .zip(&sig.forward)
        .filter(|(_, f)| **f)
        .map(|(a, _)| *a)
        .collect();

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
    let stores_ptr: *mut crate::database::Stores = stores;
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

    // @PLN118 arc F — non-perturbing attribution of the shared-return orphan.
    // For a struct/vector return the caller pre-allocates a retbuf and forwards
    // it as `popped[hidden_dest]`; the bridge writes into it and returns it.  If
    // the callee IGNORES the retbuf and returns a fresh store (e.g. a struct
    // literal that allocates a fresh record), the returned ref differs from the
    // forwarded retbuf and the retbuf is orphaned — one leaked store per call.
    // Gated, zero cost off.
    if std::env::var_os("LOFT_TRACE_SHARED_RET").is_some()
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
                // buffer (e.g. a constant-text body) — there is no caller cell
                // to carry the bytes; degrade to an empty `Str` and flag in dev.
                debug_assert!(
                    ret.text_ptr.is_null(),
                    "auto-native text return reached the dispatcher without any \
                     dest cell — @PLN10 dest-passing coverage gap"
                );
                stores.put(stack, crate::keys::Str::new(""));
            }
        }
    }
    IN_SHARED_BRIDGE.with(|c| c.set(0));
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
/// The `--remap-path-prefix` flags for the machine doing the building.
///
/// Mirrors `scripts/repro-flags.sh`, which does the same for the release build.
/// Both exist so a compiled artifact records `/cargo` and `/rustc` instead of
/// whoever's home directory produced it — the difference between a release
/// anyone can rebuild and one that only matches on the maintainer's laptop.
// Not gated on `native-extensions`: its only caller, `auto_build_native`, is
// not either, so gating it broke `--no-default-features` (E0425). Nothing in
// the body needs the feature — it reads `rustc --print sysroot` and two env
// vars.
fn local_remap_flags() -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    // The toolchain sysroot first: it lives inside the rustup home and carries
    // the toolchain's own directory NAME, so a build on `stable` and one pinned
    // to an exact version would otherwise still differ.
    if let Ok(o) = std::process::Command::new(std::env::var("RUSTC").as_deref().unwrap_or("rustc"))
        .arg("--print")
        .arg("sysroot")
        .output()
        && o.status.success()
    {
        let root = String::from_utf8_lossy(&o.stdout).trim().to_string();
        if !root.is_empty() {
            let _ = write!(out, "--remap-path-prefix={root}=/rustc ");
        }
    }
    let cargo_home = std::env::var("CARGO_HOME")
        .ok()
        .or_else(|| std::env::var("HOME").ok().map(|h| format!("{h}/.cargo")));
    if let Some(c) = cargo_home {
        let _ = write!(out, "--remap-path-prefix={c}=/cargo");
    }
    out
}

#[cfg(feature = "native-extensions")]
fn native_auto_dispatch(stores: &mut crate::database::Stores, stack: &mut crate::keys::DbRef) {
    // Read the current library index from the thread-local.
    let lib_idx = CURRENT_LIB_IDX.with(std::cell::Cell::get);

    // P245: clone the (sym, sig) entry out of NATIVE_SIGS and DROP the
    // mutex guard before invoking the native function.  The native fn
    // may block (e.g. `n_tcp_accept` waits in `listener.accept()`) —
    // holding the guard across the call serialises every parallel arm
    // that calls into native code, manifesting as a hang on the
    // sibling worker.  The Mutex is now used only for the table
    // lookup itself, never for the call.
    let (sym, sig) = {
        let guard = NATIVE_SIGS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let sig_table = guard.as_ref().expect("NATIVE_SIGS not initialized");
        match sig_table.get(&lib_idx) {
            Some((s, sg)) => (s.clone(), sg.clone()),
            None => panic!("no signature for lib_idx {lib_idx}"),
        }
    };

    // Plan-74: dispatch goes through the generated `#[loft_native]` bridge the
    // owning cdylib registered.  The legacy ~98-arm raw-ptr marshaller is gone —
    // every native library now ships a `loft_register_bridges_v1` table.  A
    // symbol with no bridge is an un-migrated cdylib (a pre-bridge library
    // version); rebuild/republish it against the bridge-capable `loft-ffi`.
    let bridge_ptr = get_bridge(&sym).unwrap_or_else(|| {
        panic!(
            "native symbol '{sym}' has no marshal bridge — its cdylib predates \
             the generated `#[loft_native]` bridge (loft-ffi `loft_register_bridges_v1`); \
             rebuild the library against the bridge-capable loft-ffi"
        );
    });
    dispatch_via_bridge(stores, stack, bridge_ptr, &sig);
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

    // Build args at full width (pop in reverse — LIFO — then restore order).
    let mut args: Vec<LoftValue> = Vec::with_capacity(sig.params.len());
    for &t in sig.params.iter().rev() {
        let v = match t {
            // No pre-narrowing: pass the whole i64 cell; the bridge casts to
            // the impl's real width (i32/u16/…) per its Rust signature.
            ArgT::I32 | ArgT::I64 => LoftValue::int(stores.get::<i64>(stack)),
            ArgT::F32 | ArgT::F64 => LoftValue::float(stores.get::<f64>(stack)),
            ArgT::Bool => LoftValue::boolean(stores.get::<bool>(stack)),
            ArgT::Text => {
                let s = stores.get::<Str>(stack);
                LoftValue::text(FfiStr {
                    ptr: s.str().as_ptr(),
                    len: s.str().len(),
                })
            }
            ArgT::Ref => {
                let r = stores.get::<crate::keys::DbRef>(stack);
                LoftValue::reference(FfiRef {
                    store_nr: r.store_nr,
                    rec: r.rec,
                    pos: r.pos,
                })
            }
            ArgT::Vec => {
                // Same indirect-vector deref as the legacy marshal.
                let r = stores.get::<crate::keys::DbRef>(stack);
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
        };
        args.push(v);
    }
    args.reverse();

    // The store the bridge allocates in: the first ref arg's store (so ref/
    // vector params and an allocating return resolve there), else — for a
    // ref/vector RETURN with no ref arg to derive from — a fresh heap store via
    // `stores.null()`, exactly as `dispatch_call`'s ref-return arms do.  The
    // old `unwrap_or(0)` fallback put an owned return vector in store 0 (the
    // stack store), so freeing it on scope exit tripped the #306 guard (a
    // stack-record ref treated as an owned heap store) — e.g. `rand_indices`
    // and imaging's PNG loaders, which return a vector with no ref argument.
    let ref_arg_store = args
        .iter()
        .find_map(|v| (v.tag == LoftTag::Ref).then(|| v.as_ref().store_nr));
    let store_nr = match ref_arg_store {
        Some(s) => s,
        None if matches!(sig.ret, Some(ArgT::Ref | ArgT::Vec)) => stores.null().store_nr,
        None => 0,
    };
    let ls = make_loft_store(stores, store_nr);

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
    let header = stores.claim(&base, 1);
    stores.store_mut(&base).set_u32_raw(header.rec, 4, r.rec);
    let dbref = crate::keys::DbRef {
        store_nr: r.store_nr,
        rec: header.rec,
        pos: 4,
    };
    stores.put(stack, dbref);
}

// Thread-local: current library index being dispatched.
// Set by `State::static_call()` before invoking the Call function.
std::thread_local! {
    static CURRENT_LIB_IDX: std::cell::Cell<u16> = const { std::cell::Cell::new(0) };
    /// loft#715 — `lib_idx + 1` while a shared-bridge call is on this thread's
    /// stack, 0 otherwise.  Read by [`current_shared_bridge`] on the fault path.
    static IN_SHARED_BRIDGE: std::cell::Cell<u16> = const { std::cell::Cell::new(0) };
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

/// Set the current library index for auto-dispatch. Called from `State::static_call()`.
/// The library index of the static call in flight — the only way a
/// `Call = fn(&mut Stores, &mut DbRef)` handler learns WHICH binding it is
/// serving. Read by the `#c` dispatcher (@PLN24 arc B) for the same reason the
/// native auto-dispatcher reads it.
#[must_use]
pub fn current_lib_idx() -> u16 {
    CURRENT_LIB_IDX.with(std::cell::Cell::get)
}

pub fn set_current_lib_idx(idx: u16) {
    CURRENT_LIB_IDX.with(|c| c.set(idx));
}

/// loft#715 — the bridge symbol currently being dispatched on this thread, if any.
///
/// A fault raised while a shared-library bridge is on the stack is a BRIDGE
/// fault, but it surfaced with the same words as a program fault: `Cannot add to
/// none-structure '<type>'` names the type the index resolved to and nothing
/// about who supplied the index — and because it is a non-unwinding panic the
/// process aborts, so the surrounding harness reports only that the program
/// never started.  Appending this turns "a type is wrong somewhere" into "this
/// library's bridge passed something this loft cannot use".
#[must_use]
pub fn current_shared_bridge() -> Option<String> {
    #[cfg(feature = "native-extensions")]
    {
        let marked = IN_SHARED_BRIDGE.with(std::cell::Cell::get);
        if marked == 0 {
            return None;
        }
        let lib_idx = marked - 1;
        let guard = SHARED_LABELS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Some(guard.as_ref().and_then(|t| t.get(&lib_idx)).map_or_else(
            || format!("shared-library bridge (slot {lib_idx})"),
            |sym| format!("shared-library bridge `{sym}`"),
        ))
    }
    #[cfg(not(feature = "native-extensions"))]
    None
}

/// Build a LoftStore handle from the store that a LoftRef points to.
/// Includes allocation callbacks so native code can create records and vectors.
#[cfg(feature = "native-extensions")]
fn make_loft_store(stores: &crate::database::Stores, store_nr: u16) -> LoftStore {
    let store = stores.store(&crate::keys::DbRef {
        store_nr,
        rec: 0,
        pos: 0,
    });
    LoftStore {
        ptr: store.base_ptr(),
        size: store.capacity_words(),
        ctx: LoftStoreCtx {
            _opaque: store_nr as usize as *mut (),
        },
        claim_fn: Some(ffi_claim),
        reload_fn: Some(ffi_reload),
        resize_fn: Some(ffi_resize),
    }
}

// ── Auto-build ──────────────────────────────────────────────────────────

/// Auto-build a package's native crate if the shared library OR
/// rlib is missing.
///
/// Both artifacts are produced by the same `cargo build` invocation
/// (every `lib/*/native/Cargo.toml` declares
/// `crate-type = ["cdylib", "rlib"]`).  The cdylib is loaded by the
/// runtime extension loader; the rlib is consumed by `--native`
/// codegen via `--extern <crate>=<path>` (see
/// `src/main.rs::add_native_extern_flags`).
///
/// Pre-2026-05-12 this function checked ONLY the cdylib before
/// returning early, which let a parallel test see "cdylib exists"
/// → skip the build → then fail later when `add_native_extern_flags`
/// looked for the rlib that hadn't been written yet.  On Windows
/// (where file-system flush ordering between cdylib and rlib differs
/// from ext4/APFS, and file-locks held by another concurrent cargo
/// invocation block reads even when the file is on disk) this race
/// surfaced as `error[E0463]: can't find crate for <name>` in
/// `tests/codegen_emitter.rs::p244_text_native_wrapper_compiles_under_native`
/// and any other test that ran in parallel with a fresh build.
///
/// Fix: also require the rlib to exist before returning early.  If
/// either artifact is missing, run cargo (which is itself
/// fcntl-locked per target dir, so concurrent invocations serialize
/// and only one actually rebuilds).
/// @PLAN12 Phase 6b — compute the target dir for a package's
/// native cdylib/rlib build.
///
/// When `pkg_dir` is under `~/.loft/registry/` (i.e., a
/// `loft install`-extracted chunk), returns the redirected
/// `~/.loft/build-cache/<pkg>-<ver>/` path; otherwise returns
/// `<pkg_dir>/native/target` (the in-tree default).
///
/// Used by [`auto_build_native`] (to set `CARGO_TARGET_DIR` and
/// to read the freshly-built artifact) AND by
/// `native_utils::add_native_extern_flags` (to find the rlib at
/// link time).  Single source of truth so the cdylib/rlib are
/// always at the same root.
#[must_use]
pub fn native_target_root(pkg_dir: &std::path::Path) -> std::path::PathBuf {
    // Without the registry feature there is no `~/.loft/registry/` to
    // redirect away from — every install is in-tree, so the package
    // dir's own `native/target/` is the target root.
    #[cfg(not(feature = "registry"))]
    {
        pkg_dir.join("native").join("target")
    }
    #[cfg(feature = "registry")]
    {
        let registry_cache = crate::registry_index::cache_dir();
        let registry_cache_canon = crate::portable_path::try_plain_canonical(&registry_cache);
        let pkg_canon = crate::portable_path::try_plain_canonical(std::path::Path::new(pkg_dir));
        let use_redirected = match (&registry_cache_canon, &pkg_canon) {
            (Some(rc), Some(pc)) => pc.starts_with(rc),
            _ => false,
        };
        if use_redirected {
            let stem_dir = pkg_dir.file_name().map_or_else(
                || "native-pkg".to_string(),
                |s| s.to_string_lossy().into_owned(),
            );
            registry_cache
                .parent()
                .map_or_else(
                    || std::path::PathBuf::from("."),
                    std::path::Path::to_path_buf,
                )
                .join("build-cache")
                .join(stem_dir)
        } else {
            pkg_dir.join("native").join("target")
        }
    }
}

/// Resolve a `[library] native = "<stem>"` registration to a loadable cdylib
/// path: a pre-built `<pkg_dir>/native/<libname>` wins, else build (or reuse a
/// fingerprint-fresh build of) the package's native crate via
/// [`auto_build_native`].  The ONE home for this resolution — the parser's two
/// manifest paths and the warm startup-cache load (#310) all derive from it,
/// so a cached run re-checks cdylib freshness exactly like a cold parse.
pub fn resolve_native_lib(pkg_dir: &str, stem: &str) -> Option<String> {
    // @PLN21 Phase 3 — a missing DECLARED runtime system lib is terminal:
    // neither a prebuilt nor a source build can load it (both link the same lib,
    // decision C3).  Check FIRST and emit an actionable hint, rather than loading
    // a doomed prebuilt or spending ~90s on a build that cannot load.
    if let Some(lib) = first_missing_runtime_lib(pkg_dir) {
        eprintln!("{}", runtime_lib_missing_diagnostic(stem, &lib));
        return None;
    }
    let filename = platform_lib_name(stem);
    // @PLN21 Phase 1 — a precompiled cdylib shipped for THIS host triple wins
    // over a source build (the "no rustc to use a library" path).  A cdylib
    // links loft-ffi (the C-ABI), not libloft, so it is valid for any loft on
    // the same loft-ffi version — gated on the `.loft-build-fp` sidecar matching
    // `loft_ffi_fingerprint()`, so a binary built against a different loft-ffi
    // is skipped (never mis-loaded), falling through to a source build.
    let triple_dir = format!("{pkg_dir}/prebuilt/{}", crate::cache::host_triple());
    let triple_lib = format!("{triple_dir}/{filename}");
    if std::path::Path::new(&triple_lib).exists()
        && crate::cache::native_artifact_fingerprint_matches(
            std::path::Path::new(&triple_dir),
            crate::cache::loft_ffi_fingerprint(),
        )
    {
        crate::platform::timing_record("prebuilt", stem, true, None);
        return Some(triple_lib);
    }
    // Legacy platform-agnostic prebuilt (existence-only; kept for back-compat).
    let prebuilt = format!("{pkg_dir}/native/{filename}");
    if std::path::Path::new(&prebuilt).exists() {
        return Some(prebuilt);
    }
    auto_build_native(pkg_dir, stem)
}

/// The OS a library NAME targets, inferred from its soname/extension form, or
/// `None` when it can't be told (extensionless / unrecognised).  Soname matching,
/// not file-extension parsing: `.so` is matched as a substring because versioned
/// sonames are `.so.N` (which `Path::extension` would read as "N"), and sonames
/// are lowercase by platform convention — so case-sensitive substring checks are
/// correct here.  The single source of truth for runtime-lib platform-scoping.
#[cfg(feature = "native-extensions")]
fn lib_name_target_os(lib: &str) -> Option<&'static str> {
    #[allow(clippy::case_sensitive_file_extension_comparisons)]
    if lib.contains(".so") {
        Some("Linux")
    } else if lib.ends_with(".dylib") {
        Some("macOS")
    } else if lib.ends_with(".dll") {
        Some("Windows")
    } else {
        None
    }
}

/// The host OS as a display name, for runtime-lib diagnostics + platform-scoping.
/// Not feature-gated: `runtime_lib_missing_diagnostic` (and thus its call site in
/// the unconditionally-compiled `resolve_native_lib`) needs it even when
/// `native-extensions` is off (e.g. the WASM build).
fn host_os_name() -> &'static str {
    if cfg!(target_os = "linux") {
        "Linux"
    } else if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "this OS"
    }
}

/// @PLN21 Phase 3 — the first host-applicable `[native] runtime-libs` entry the
/// dynamic linker can't find, if any.  `dlopen` IS the authoritative presence
/// check: a declared lib that loads is present; one that fails is missing, so the
/// package's cdylib (prebuilt OR freshly built) could not load.
///
/// A runtime-lib named for a DIFFERENT OS than the host (e.g. a Linux `libGL.so.1`
/// declared by a library used on macOS) is **skipped, not probed**: it can never
/// load here and is not a host requirement — the host build of the cdylib links
/// its own platform libraries.  Probing it would wrongly hard-fail the whole
/// library on every foreign platform.  Only host-applicable (or unclassifiable)
/// names gate resolution.  Empty `runtime-libs` → no check, no cost.
#[cfg(feature = "native-extensions")]
fn first_missing_runtime_lib(pkg_dir: &str) -> Option<String> {
    let host = host_os_name();
    crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))?
        .runtime_libs
        .into_iter()
        .filter(|lib| lib_name_target_os(lib).is_none_or(|os| os == host))
        .find(|lib| unsafe { libloading::Library::new(lib) }.is_err())
}

#[cfg(not(feature = "native-extensions"))]
fn first_missing_runtime_lib(_pkg_dir: &str) -> Option<String> {
    None
}

/// Diagnostic for a host-applicable `[native] runtime-libs` entry that could not
/// be `dlopen`'d — the library is genuinely not installed.  Foreign-OS names are
/// skipped before this point (see [`first_missing_runtime_lib`]), so they never
/// reach here and never produce misleading "install it" advice.  Not
/// feature-gated: its call site in `resolve_native_lib` compiles unconditionally
/// (the `first_missing_runtime_lib` if-branch is dead but still type-checked when
/// `native-extensions` is off), so the helper must exist in every build.
fn runtime_lib_missing_diagnostic(stem: &str, lib: &str) -> String {
    format!(
        "loft: native library '{stem}' needs the system library '{lib}', which is not installed \
         on {host} ({triple}) — install it with your OS package manager (building from source \
         would link the same library and fail identically).",
        host = host_os_name(),
        triple = crate::cache::host_triple(),
    )
}

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
    let Ok(entries) = std::fs::read_dir(crate::registry_index::cache_dir()) else {
        return (0, 0);
    };
    let mut dirs: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("native").join("Cargo.toml").exists())
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
        let name = dir
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
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

/// The link flags that make a built cdylib RELOCATABLE — empty on every platform but macOS.
///
/// A Mach-O dylib records its own path (`LC_ID_DYLIB`) and a program that links it copies THAT
/// path in, so the loader follows the build-time location and nothing else.  Cargo's default is
/// the absolute output path, `…/target/release/deps/lib<stem>.dylib` — and this cdylib is
/// CACHED and reused from a different directory than the one it was built in, so the recorded
/// path names a directory that no longer exists.  On the nightly's macOS leg that is
/// `dyld: Library not loaded: …/.loft_test_tmp_<pid>_0/native/target/release/deps/…`, on a
/// cache HIT, after a MISS built it under a previous run's temporary directory.
///
/// ELF does not have the problem: a `.so` records only its SONAME (the bare file name) and the
/// consumer's `-rpath` resolves it, which is why the same cache is fine on Linux and why this
/// is macOS-only rather than a cache bug.  `@rpath/<file>` makes Mach-O behave the same way,
/// and the consumer already emits both the absolute `-rpath` of the resolved library and
/// `$ORIGIN` / `@loader_path` (`native_utils::add_native_extern_flags`).
fn relocatable_dylib_flags(lib_name: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("-Clink-arg=-Wl,-install_name,@rpath/{lib_name}")
    } else {
        String::new()
    }
}

pub fn auto_build_native(pkg_dir: &str, stem: &str) -> Option<String> {
    use std::path::PathBuf;
    // P244-windows fix #2 (2026-05-12): use PathBuf::join, not
    // `format!("{pkg_dir}/...")`.  When `pkg_dir` arrives as a
    // canonicalized Windows extended-length path (e.g.
    // `\\?\D:\a\loft\loft\lib\server`), concatenating with `/native/...`
    // produces a malformed path: the `\\?\` verbatim prefix bypasses
    // Rust's path normalization, so the mixed-separator suffix
    // doesn't resolve and `Path::exists()` returns false even when
    // the file is on disk.  PathBuf::join handles each component
    // through proper Path semantics on every platform.
    let pkg = PathBuf::from(pkg_dir);
    let cargo_toml = pkg.join("native").join("Cargo.toml");
    if !cargo_toml.exists() {
        return None;
    }
    let lib_name = platform_lib_name(stem);
    let rlib_name = format!("lib{stem}.rlib");

    // @PLAN12 Phase 6b — target root via the shared helper (redirects
    // chunk-resident installs to ~/.loft/build-cache/<pkg>-<ver>/;
    // keeps in-tree target/ for the monorepo's lib/<pkg>/native/).
    let target_root = native_target_root(&pkg);
    let in_tree_target = pkg.join("native").join("target");
    let use_redirected_target = target_root != in_tree_target;

    // Existing-cache check: look in the redirected target first
    // (where future builds land), then the legacy in-tree target/
    // (so existing builds from older loft binaries are still
    // reused).
    let mut search_roots: Vec<PathBuf> = Vec::new();
    search_roots.push(target_root.clone());
    if use_redirected_target {
        search_roots.push(in_tree_target.clone());
    }
    // Reuse a cached cdylib only when it was built against the same loft-ffi ABI, the
    // same RUSTFLAGS and the same loft version.  This crate is HAND-WRITTEN: it LINKS
    // loft-ffi (the C-ABI), never libloft.rlib (verified: zero loft undefined symbols),
    // and depends on no loft codegen — so the loft build is deliberately NOT in the key
    // (`native_artifact_cache_key`, @PLN159 phase C; the generated `loft_auto_*` cdylib
    // is the one a codegen change reaches, and it carries the rlib hash in its name).
    // Profile-independent, so debug/release/test in a job SHARE the cache; it flips on
    // a real loft-ffi change, a flag change (#274 — `-g` ↔ not, the shared-dep SVH fix)
    // or a version bump, and the crate's own sources are checked by mtime (loft#965).
    let fp = crate::cache::native_artifact_cache_key();
    let find_existing = || {
        for root in &search_roots {
            for profile in ["release", "debug"] {
                let dir = root.join(profile);
                let lib = dir.join(&lib_name);
                let rlib = dir.join(&rlib_name);
                if lib.exists()
                    && rlib.exists()
                    && crate::cache::native_artifact_fingerprint_matches(&dir, fp)
                    // loft#965 — and the crate's OWN Rust sources are not newer.  The
                    // fingerprint answers "same loft-ffi ABI, RUSTFLAGS and codegen
                    // version", which are all questions about the environment; it says
                    // nothing about the hand-written code being compiled.  So editing a
                    // `#native` package's `native/src/lib.rs` and running `loft test`
                    // loaded the PREVIOUS build and passed — a false GREEN, and one only
                    // a local author sees, because a clean CI checkout has no artifact
                    // to reuse.
                    && !native_crate_newer_than(pkg_dir, &lib)
                {
                    return Some(lib.to_string_lossy().to_string());
                }
            }
        }
        None
    };
    if let Some(p) = find_existing() {
        crate::platform::timing_record("cdylib", stem, true, None);
        return Some(p);
    }

    // Visibility (building on loft2's loud-fallback commit): distinguish a COLD
    // miss (nothing cached) from a STALE REJECTION — the cdylib + rlib EXIST but
    // their stamped fingerprint != THIS build's fp, so they are rebuilt.  Across
    // CI runs that is the dominant cost: every loft source commit flips
    // `loft_build_fingerprint` (the libloft.rlib content hash), rejecting cdylibs
    // that actually only depend on the stable published loft-ffi ABI.  Log the
    // stamped-vs-current fp (so two runs reveal whether the rlib hash flipped on
    // an IDENTICAL commit — a second, non-reproducible-build instability — or
    // only across commits) and record a `cdylibstale` event the CI timing step
    // sums into wasted-rebuild seconds.  Pure diagnostics — no gate change yet.
    'scan: for root in &search_roots {
        for profile in ["release", "debug"] {
            let dir = root.join(profile);
            if dir.join(&lib_name).exists() && dir.join(&rlib_name).exists() {
                let stamped = crate::cache::native_artifact_stamped_fp(&dir)
                    .map_or_else(|| "none".to_string(), |s| s.to_string());
                eprintln!(
                    "loft: note — cdylib {stem} rebuilt: cached artifact rejected \
                     (stamped loft-ffi fp={stamped} != current fp={fp}) — loft-ffi's source \
                     changed since it was built. (Keyed on loft-ffi, not libloft.rlib, so a \
                     plain interpreter change no longer triggers this.)"
                );
                // Encode stamped|cur into the ledger name so the fp values
                // survive on a PASSING run (nextest hides the eprintln above);
                // the CI step strips the suffix for the per-package join.
                let tagged = format!("{stem}|stamped={stamped}|cur={fp}");
                crate::platform::timing_record("cdylibstale", &tagged, false, None);
                break 'scan;
            }
        }
    }

    // @P388: serialise on-demand native builds ACROSS PROCESSES.  Parallel test
    // binaries (nextest runs one process per test) and parallel end-user `loft`
    // invocations otherwise each run an unlocked `cargo build` that re-resolves
    // the dependency tree concurrently, racing cargo's shared registry index +
    // package cache → transient "required to be available in rlib format" errors
    // and non-deterministic version picks.  A single cross-process advisory file
    // lock serialises the builds.  (The CI pre-build already serialises CI; this
    // covers the latent end-user-parallelism half of @P388.)  The `--locked`
    // route is unavailable: the repo deliberately gitignores every `Cargo.lock`,
    // and these crates.io-sourced native deps would drift against the actively-
    // versioned loft-ffi crates.  Best-effort: if the lock can't be opened/taken
    // we proceed unserialised — no worse than before.  `File::lock` blocks until
    // acquired and releases when `_build_lock` drops at function exit (so a crash
    // mid-build can't strand the lock).
    let _build_lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(std::env::temp_dir().join("loft-native-build.lock"))
        .ok();
    if let Some(f) = &_build_lock {
        // Reveal cross-process contention on this ONE global lock (the prime
        // suspect for the CI 600s multiplayer hang): record `lockwait` BEFORE
        // blocking (so it lands in the ledger even if this process is killed
        // while still waiting), then `lockheld` with the wait duration once
        // acquired.  A `lockwait` with no matching `lockheld` for a package =
        // a process stuck behind another's long cold build.
        let lock_t = std::time::Instant::now();
        crate::platform::timing_record("lockwait", stem, false, None);
        // loft#1238 — and tell the TIMEOUT what we are waiting for.  The ledger already
        // distinguished "still waiting" (a `lockwait` with no `lockheld`) from "waited and
        // built", but only to a reader who has the ledger AND knows that convention; the kill
        // message said `phase=parse`, which is true and useless.  Now the process that dies here
        // says so in the line that reports its death.
        let waiting = format!("the global native-build lock (for {stem})");
        // loft#1238 — bounded by the run's remaining budget.  Waiting past the deadline cannot
        // succeed: the process is killed mid-queue, builds nothing, stamps nothing, and the next
        // attempt repeats it.  Failing here instead is the same outcome said out loud.
        if let Err(waited) = crate::timeout::lock_within_budget(f, &waiting) {
            crate::platform::timing_record("lockgaveup", stem, false, Some(waited.as_secs_f64()));
            if crate::timeout::first_giveup_report(stem) {
                eprintln!(
                    "loft: TRANSIENT — gave up waiting for {waiting} after {:.1}s.\n  \
                     Another process is building a native library and this run's timeout \
                     would expire before the queue cleared, so it stopped rather than be \
                     killed mid-wait.\n  Nothing here is broken or stale in the way the \
                     next message will suggest: `{stem}`'s cdylib simply had not been \
                     rebuilt yet. Build it once up front (`loft cache warm`), raise the \
                     timeout, or re-run once the other build has finished.",
                    waited.as_secs_f64()
                );
            }
            return None;
        }
        crate::platform::timing_record(
            "lockheld",
            stem,
            false,
            Some(lock_t.elapsed().as_secs_f64()),
        );
    }
    // Re-check under the lock: a process we waited on may have just produced the
    // artifact, in which case we must NOT rebuild.
    if let Some(p) = find_existing() {
        return Some(p);
    }

    // NO rustc-version guard here, deliberately: a package's native crate
    // depends on loft-ffi (the C-ABI contract) — never the SVH-locked loft
    // rlib — so ANY rustc builds it correctly (cargo itself rebuilds when
    // the resolved toolchain flips between invocation cwds).  The guard
    // belongs only to builds that link the rlib (`build_shared_cdylib`,
    // the driver's program-native path).

    // An artifact EXISTS here but its fingerprint names another loft build —
    // cargo cannot be trusted to rebuild it, and the post-build stamp would
    // launder it as fresh (see `cache::clear_stale_native_target`).  Only
    // the REDIRECTED root (`~/.loft/build-cache/…`) is cleared: it is
    // private to this resolution path (every consumer arrives through the
    // build lock above), whereas an in-tree `native/target` is shared with
    // direct-path consumers — the tests/lib fixture loaders read the `.so`
    // path without the lock, so a wipe there races them (suite-caught) —
    // and in-tree crates remain the documented dev workflow
    // (`make rebuild-native-cdylibs` / the stub-panic rebuild hint).
    if use_redirected_target {
        crate::cache::clear_stale_native_target(&target_root, &lib_name, &rlib_name, fp);
    }

    // Build.  When redirecting, pass `CARGO_TARGET_DIR` so cargo writes outside
    // the install dir.  Factored into a closure so a `--locked` failure can
    // retry without it (below) — the two invocations differ only in that flag.
    let make_cmd = |locked: bool| {
        let mut cmd = std::process::Command::new("cargo");
        cmd.args(["build", "--release", "--manifest-path"])
            .arg(&cargo_toml)
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit());
        if locked {
            cmd.arg("--locked");
        }
        // #274 — build the package crate with the SAME RUSTFLAGS loft's own
        // rlibs used (captured at loft build time), so a shared transitive dep
        // like `libloading` gets a matching SVH.  Otherwise loft's `-g` copy and
        // the package's plain copy share a `StableCrateId` but differ in SVH and
        // rustc aborts at the generated program's link step.  Force it
        // (overriding any ambient value) and clear `CARGO_ENCODED_RUSTFLAGS` so
        // cargo doesn't see both forms at once.
        // The baked flags carry no `--remap-path-prefix` (build.rs strips them:
        // they name the RELEASE machine's paths).  Recompute them for THIS
        // machine so loft's rlibs and the package crate agree on `/cargo` and
        // `/rustc`, which is what #274's SVH match actually needs — and which
        // also keeps the consumer's cdylib free of their own home directory.
        let flags = format!(
            "{} {} {}",
            env!("LOFT_BUILD_RUSTFLAGS"),
            local_remap_flags(),
            relocatable_dylib_flags(&lib_name)
        );
        cmd.env("RUSTFLAGS", flags.trim())
            .env_remove("CARGO_ENCODED_RUSTFLAGS");
        if use_redirected_target {
            if let Some(parent) = target_root.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            cmd.env("CARGO_TARGET_DIR", &target_root);
        }
        cmd
    };
    let built_path = target_root.join("release").join(&lib_name);
    let build_start = std::time::Instant::now();
    // @PLN21 Phase 6 — prefer `--locked` when the package SHIPS a
    // `native/Cargo.lock`: cargo then uses the pinned resolution (reproducible —
    // two machines produce the same cdylib bytes).  But a shipped lock can be
    // platform-INCOMPLETE: one generated on Linux lacks a crate's `cfg(windows)`
    // deps (e.g. `windows-targets`), so `cargo build --locked` on Windows must
    // update the lock and refuses ("cannot update the lock file … because
    // --locked was passed").  A FALLBACK source build must never hard-fail on
    // that (Goal F), so on a locked failure we retry WITHOUT --locked, warning
    // it is non-reproducible — reproducibility is best-effort here; the
    // submit-time gate is where a complete lock is enforced.
    let has_lock = cargo_toml.with_file_name("Cargo.lock").exists();
    let mut status = make_cmd(has_lock).status();
    if has_lock && matches!(&status, Ok(s) if !s.success()) {
        eprintln!(
            "loft: '--locked' build of '{stem}' failed — its Cargo.lock can't be \
             satisfied as-is on this host (often a platform-specific dep missing \
             from the shipped lock); retrying without --locked (non-reproducible)."
        );
        status = make_cmd(false).status();
    }
    crate::platform::timing_record(
        "cdylib",
        stem,
        false,
        Some(build_start.elapsed().as_secs_f64()),
    );
    match status {
        Ok(s) if s.success() => {
            // @PLN11 Arc N / N0 — stamp the build fingerprint on ANY successful
            // cargo build: the rlib is produced even for rlib-only packages whose
            // cdylib `built_path` is absent, so a later loft change still
            // invalidates it (see `find_existing` / `add_native_extern_flags`).
            crate::cache::write_native_artifact_fingerprint(&target_root.join("release"), fp);
            built_path
                .exists()
                .then(|| built_path.to_string_lossy().to_string())
        }
        // @PLN21 Phase 3 — the build RAN but failed (cargo's error is on the
        // inherited stderr).  The usual cause is a missing build dependency, so
        // name the package's declared `build-deps` rather than leaving the user
        // to parse a raw rustc/linker/pkg-config error.
        Ok(_) => {
            eprintln!(
                "loft: building native library '{stem}' from source failed (cargo error above).{}",
                build_deps_hint(pkg_dir)
            );
            None
        }
        // cargo itself could not start — typically no Rust toolchain installed.
        Err(e) => {
            eprintln!(
                "loft: cannot build native library '{stem}': {e} — building a library with no \
                 prebuilt for this host needs a Rust toolchain (rustc + cargo)."
            );
            None
        }
    }
}

/// @PLN26 phase 3 — cross-build a `#native` package's crate to a wasm `target`
/// (`wasm32-wasip2` for `--native-wasm`, `wasm32-unknown-unknown` for `--html`) so the
/// wasm linker can consume its **rlib**, the way [`auto_build_native`] produces the host
/// cdylib.  The rlib lands at the IN-TREE `<pkg>/native/target/<target>/release/lib<stem>.rlib`
/// — exactly the path `native_utils::add_native_extern_flags` reads for a wasm target.
///
/// Returns whether that rlib is present afterwards: reused when it is already stamped with
/// the current loft-ffi ABI key, else freshly cross-built.  Best-effort — a missing
/// toolchain/target, or a crate that is not wasm-clean, returns `false` (the caller then
/// emits the "no wasm build" notice rather than dying on a bare `E0463`).
/// Whether anything in `<pkg_dir>/native/` is newer than `artifact` — the crate's own
/// hand-written Rust, which no other freshness check looks at (loft#965).
///
/// The sibling walk for GENERATED code (`native_lib.rs::source_newer_than`) covers
/// `*.loft` and `loft.toml` and explicitly SKIPS the `native` directory, and the artifact
/// fingerprint answers a question about the environment (loft-ffi ABI, RUSTFLAGS, codegen
/// version) rather than about the sources.  So between them a `#native` package's own
/// `src/lib.rs` was the one input nothing watched.
///
/// `target/` is skipped: it is the build's own output, and it is always newer.
/// Unreadable metadata answers "newer", so the failure mode is a rebuild, never a reuse
/// of something stale.
fn native_crate_newer_than(pkg_dir: &str, artifact: &std::path::Path) -> bool {
    let Ok(art) = artifact.metadata().and_then(|m| m.modified()) else {
        return true;
    };
    let mut stack = vec![std::path::PathBuf::from(pkg_dir).join("native")];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in entries.flatten() {
            if e.file_name() == "target" {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            if ft.is_dir() {
                stack.push(e.path());
                continue;
            }
            if e.metadata()
                .and_then(|m| m.modified())
                .is_ok_and(|mt| mt > art)
            {
                return true;
            }
        }
    }
    false
}

pub fn auto_build_native_target(pkg_dir: &str, stem: &str, target: &str) -> bool {
    use std::path::PathBuf;
    let pkg = PathBuf::from(pkg_dir);
    let cargo_toml = pkg.join("native").join("Cargo.toml");
    if !cargo_toml.exists() {
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
        out.join(&rlib_name).exists()
            && crate::cache::native_artifact_fingerprint_matches(out, fp)
            // loft#965 — same hole, same cure, on the cross-target path.
            && !native_crate_newer_than(pkg_dir, &out.join(&rlib_name))
    };
    if fresh(&out_dir) {
        return true;
    }
    // Serialise cross-process builds on the SAME global lock the host path uses, so
    // parallel `loft` invocations don't race cargo's shared registry index/cache.
    let _build_lock = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(false)
        .open(std::env::temp_dir().join("loft-native-build.lock"))
        .ok();
    if let Some(f) = &_build_lock {
        let _ = f.lock();
    }
    if fresh(&out_dir) {
        // A process we waited on just produced it.
        return true;
    }
    let mut cmd = std::process::Command::new("cargo");
    cmd.args(["build", "--release", "--target", target, "--manifest-path"])
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
            rlib.exists()
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

/// @PLN21 Phase 3 — a trailing hint naming the package's declared `[native]
/// build-deps` (the system dev packages its cdylib needs to compile), for a
/// failed source build.  Empty when none are declared.
fn build_deps_hint(pkg_dir: &str) -> String {
    let deps = crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))
        .map(|m| m.build_deps)
        .unwrap_or_default();
    if deps.is_empty() {
        String::new()
    } else {
        format!(
            " This package needs these dev packages to build: {}.",
            deps.join(", ")
        )
    }
}

/// Resolve the platform-correct shared-library filename from a stem.
#[must_use]
pub fn platform_lib_name(stem: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("lib{stem}.dylib")
    } else if cfg!(windows) {
        format!("{stem}.dll")
    } else {
        format!("lib{stem}.so")
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

    /// Construct a `loft_ffi::LoftStore` handle pointing at the
    /// store identified by `store_nr`.  Callbacks read from the
    /// `CURRENT_STORES` thread-local — call `enter()` before this
    /// to set it up.
    #[must_use]
    pub fn build_store(stores: &Stores, store_nr: u16) -> loft_ffi::LoftStore {
        let store = stores.store(&crate::keys::DbRef {
            store_nr,
            rec: 0,
            pos: 0,
        });
        loft_ffi::LoftStore {
            ptr: store.base_ptr(),
            size: store.capacity_words(),
            ctx: loft_ffi::LoftStoreCtx {
                _opaque: store_nr as usize as *mut (),
            },
            claim_fn: Some(ffi_claim_pub),
            reload_fn: Some(ffi_reload_pub),
            resize_fn: Some(ffi_resize_pub),
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
    use super::{
        build_deps_hint, dlopen_diagnostic, first_missing_runtime_lib,
        runtime_lib_missing_diagnostic,
    };

    fn temp_pkg(name: &str, toml: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("loft_p21_{name}_{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("loft.toml"), toml).unwrap();
        dir
    }

    // A library-name that targets THIS host (so it's probed) and one that targets
    // another OS (so it's skipped) — derived from the runner's OS so the runtime-
    // lib tests hold on every CI leg (ubuntu/macos/windows), not just Linux.
    fn host_and_foreign_lib_names() -> (&'static str, &'static str) {
        if cfg!(target_os = "macos") {
            ("libnot-real-skip.dylib", "libnot-real-skip.so.7")
        } else if cfg!(target_os = "windows") {
            ("not-real-skip.dll", "libnot-real-skip.so.7")
        } else {
            ("libnot-real-skip.so.7", "libnot-real-skip.dylib")
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
        assert!(build_deps_hint(temp_pkg("nobd", "[native]\n").to_str().unwrap()).is_empty());
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
        std::fs::create_dir_all(&sub).expect("probe dir");
        // Only the macOS artefact exists, as on a Mac that just ran `make`.
        std::fs::write(dir.join("liblc_types.dylib"), b"x").expect("write probe");

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
        std::fs::write(dir.join("liblc_types.so"), b"x").expect("write probe");
        for os in [LibOs::Linux, LibOs::Macos, LibOs::Windows] {
            assert_eq!(
                existing_lib_beside(&sub, "../liblc_types.so", os).as_deref(),
                Some("../liblc_types.so"),
                "the declared spelling is authoritative when it is there"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[cfg(test)]
mod native_freshness_tests {
    use super::native_crate_newer_than;
    use std::io::Write;

    /// Build `<root>/native/src/lib.rs` plus an artifact, and return both paths.
    fn fixture(tag: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("loft_965_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("native/src")).expect("mkdir");
        std::fs::create_dir_all(root.join("native/target/release")).expect("mkdir target");
        let src = root.join("native/src/lib.rs");
        std::fs::File::create(&src)
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
        std::fs::write(&art, b"artifact").expect("write artifact");
        art
    }

    /// The reuse case: sources older than the artifact, so the cached cdylib stands.
    #[test]
    fn an_untouched_crate_is_not_newer() {
        let (root, _src) = fixture("fresh");
        let art = artifact_after(&root, "libx.so");
        assert!(!native_crate_newer_than(&root.to_string_lossy(), &art));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// loft#965 itself: the crate's own Rust edited after the artifact was built.  This
    /// is what `loft test` used to answer "reuse" to, running the previous build against
    /// code no longer in the tree.
    #[test]
    fn an_edited_crate_source_is_newer() {
        let (root, src) = fixture("edited");
        let art = artifact_after(&root, "libx.so");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::File::create(&src)
            .and_then(|mut f| f.write_all(b"// edited\n"))
            .expect("re-write source");
        assert!(native_crate_newer_than(&root.to_string_lossy(), &art));
        let _ = std::fs::remove_dir_all(&root);
    }

    /// ⚠ `native/target/` is the build's OWN output and is always newer than the
    /// artifact's siblings.  Counting it would report every crate as stale forever —
    /// a rebuild on every run, which is the failure mode a careless fix has.
    #[test]
    fn the_crates_own_target_dir_does_not_count() {
        let (root, _src) = fixture("target");
        let art = artifact_after(&root, "libx.so");
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(root.join("native/target/release/stamp"), b"x").expect("write stamp");
        assert!(
            !native_crate_newer_than(&root.to_string_lossy(), &art),
            "output under native/target must not make the crate look stale"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    /// A package with no native crate at all answers "not newer" rather than walking
    /// nothing and defaulting to a rebuild.
    #[test]
    fn a_package_without_a_native_crate_is_not_newer() {
        let root = std::env::temp_dir().join(format!("loft_965_none_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).expect("mkdir");
        let art = artifact_after(&root, "libx.so");
        assert!(!native_crate_newer_than(&root.to_string_lossy(), &art));
        let _ = std::fs::remove_dir_all(&root);
    }
}
