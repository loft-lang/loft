// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader: the libraries this process has opened

//! The libraries this process has opened and the symbols they export: the opened-library
//! table, the registries a cdylib fills through `loft_register_v1` /
//! `loft_register_bridges_v1`, symbol lookup across them, and the declared C libraries
//! `resolve` opens on demand.  The lazy SQLite source in `database` loads and resolves
//! through it, so it sits in the core layer; wiring natives into a `State` stays in
//! `extensions` and the `#c` call trampolines in `c_call` (@PLN191).

#[cfg(feature = "native-extensions")]
use std::collections::HashMap;
#[cfg(feature = "native-extensions")]
use std::sync::Mutex;

/// One C library a package declared, as every consumer of the
/// declaration needs to see it.
///
/// A named struct rather than a tuple because four sites read it and each reads
/// a different field for a different purpose — the interpreter's load, the
/// `--native` link line, the `--native` emission, and the availability query.
/// A positional `bool` in a tuple is read wrong at one of them eventually, and
/// this plan's counted risk is that such a mistake is SILENT.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CLibrary {
    /// The soname as the dynamic linker knows it (`libpq.so.5`), or a path
    /// relative to `pkg_dir` for a library the package ships itself.
    pub name: String,
    /// The declaring package's directory. It travels with the name because a
    /// library may ship its own `.so` beside its `.loft`, so the name alone
    /// cannot be resolved.
    pub pkg_dir: String,
    /// `[c] optional-libs` rather than `[c] libs`: the package works without
    /// it, so it is NOT linked and NOT opened until a symbol needs it.
    ///
    /// The author's claim about their own library, never inferred from whether
    /// a `dlopen` happened to succeed — that would make one package required on
    /// one machine and optional on the next.
    pub optional: bool,
}

/// Wrapper for `*const ()` that is Send — function pointers from cdylibs are
/// valid for the process lifetime (the Library handle is leaked).
#[cfg(feature = "native-extensions")]
#[derive(Clone, Copy)]
pub(crate) struct FnPtr(pub(crate) *const ());
#[cfg(feature = "native-extensions")]
unsafe impl Send for FnPtr {}

/// Global registry of native function pointers loaded from cdylibs.
#[cfg(feature = "native-extensions")]
pub(crate) static NATIVE_REGISTRY: Mutex<Option<HashMap<String, FnPtr>>> = Mutex::new(None);

/// Plan-74: registry of generated marshal bridges (`loft_ffi::LoftBridgeFn`),
/// keyed by loft symbol.  Populated from a cdylib's `loft_register_bridges_v1`.
/// Every native call dispatches through its bridge (`dispatch_via_bridge`); a
/// symbol absent here is an un-migrated cdylib (a pre-bridge library version)
/// and panics at call time: there is no raw-pointer marshaller to fall back
/// to, because every native library ships bridges.
#[cfg(feature = "native-extensions")]
pub(crate) static BRIDGE_REGISTRY: Mutex<Option<HashMap<String, FnPtr>>> = Mutex::new(None);

/// Look up a generated bridge for `sym`, if the owning cdylib registered one.
#[cfg(feature = "native-extensions")]
pub(crate) fn get_bridge(sym: &str) -> Option<*const ()> {
    BRIDGE_REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .as_ref()
        .and_then(|m| m.get(sym))
        .map(|p| p.0)
}

/// The C-ABI registration callback type.
#[cfg(feature = "native-extensions")]
pub(crate) type RegisterFn =
    unsafe extern "C" fn(unsafe extern "C" fn(*const u8, usize, *const (), *mut ()), *mut ());

/// The registration callback: called once per symbol by the cdylib.
#[cfg(feature = "native-extensions")]
pub(crate) unsafe extern "C" fn collect(
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
/// `try_dlsym` to look up symbols from the cdylibs already loaded *and* report
/// whether the resolving library opted into registration — so the "unregistered
/// symbol" guard fires only for a library that chose the protocol, not for a
/// (legitimately) zero-registration cdylib loaded after one that did.
#[cfg(feature = "native-extensions")]
pub(crate) static LOADED_LIBS: Mutex<Vec<(libloading::Library, bool)>> = Mutex::new(Vec::new());

/// @PLN21 Phase 3 — turn a raw `dlopen`/`LoadLibrary` failure into an
/// actionable message.  The dynamic linker IS the authoritative validator — its
/// error text names exactly what is wrong — so we classify it rather than guess.
/// A missing RUNTIME system lib is terminal: building from source links the same
/// lib and fails identically (decision C3), so the user must install it, not
/// rebuild.
#[cfg(feature = "native-extensions")]
pub(crate) fn dlopen_diagnostic(path: &str, err: &str) -> String {
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

/// Open the library `name` names, with the loader's own error text.
///
/// `file_access::load_library`: a name with a separator is a file, a bare name asks the
/// dynamic linker's search path, and the error is the loader's own text (which
/// `dlopen_diagnostic` reads from its start).
#[cfg(feature = "native-extensions")]
pub(crate) unsafe fn open_library(name: &str) -> Result<libloading::Library, String> {
    // SAFETY: the caller's contract — loading runs the library's initialisers.
    unsafe { crate::file_access::load_library(name) }
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
/// (loft#739's neighbour; compare `load_c_library`).
pub(crate) fn load_one(path: &str) -> bool {
    use std::collections::HashSet;

    static LOAD_LOCK: Mutex<Option<HashSet<String>>> = Mutex::new(None);

    let canonical = crate::file_access::plain_canonical_str(path);

    let mut guard = LOAD_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let loaded = guard.get_or_insert_with(HashSet::new);
    if loaded.contains(&canonical) {
        return true;
    }

    let lib = match unsafe { open_library(path) } {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}", dlopen_diagnostic(path, &e));
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
pub(crate) fn try_dlsym(name: &str) -> Option<(*const (), bool)> {
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

/// `dlopen` a C library a package declared with `[c] libs`, and
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
    // @PLN197 — a native program bound this library at start, from where the binary found
    // it; the name alone (no package directory is written into the binary) would not.
    if crate::native_bind::is_bound_c(name) {
        return true;
    }
    let mut last_err = None;
    for cand in host_lib_variants(name) {
        let beside = std::path::Path::new(pkg_dir).join(&cand);
        // `load_one`, not `exists()`: a file that will not `dlopen` (a Linux
        // `.so` sitting in the tree on macOS) must fall through to the next
        // candidate rather than be reported as loaded.
        if crate::file_access::exists(&beside) && load_one(&beside.to_string_lossy()) {
            return true;
        }
        // Not a path we can see: hand the soname to the dynamic linker, which
        // knows the search path we do not.
        match unsafe { open_library(&cand) } {
            Ok(_) => {
                load_one(&cand);
                return true;
            }
            Err(e) => last_err = Some((cand, e)),
        }
    }
    // An OPTIONAL library that does not open is an ordinary
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
pub(crate) fn host_lib_variants(name: &str) -> Vec<String> {
    crate::platform::host_lib_variants(name)
}

/// The same lookup, for the `#c` caller: a C binding resolves
/// against the cdylibs loft has already loaded before it falls back to the
/// process. One resolver, so a symbol cannot mean two different things
/// depending on which caller asked for it.
#[cfg(feature = "native-extensions")]
#[must_use]
pub fn try_dlsym_pub(name: &str) -> Option<(*const (), bool)> {
    try_dlsym(name)
}

/// Every C library this program declared, each with the outcome of opening it:
/// `None` until it has been tried, then whether it opened.
///
/// Keeping the outcome (rather than draining the list) is what makes a MISSING
/// library cost one `dlopen` for the whole run instead of one per call — a
/// symbol that is genuinely absent is looked up on every iteration of whatever
/// loop calls it.
///
/// REQUIRED entries are in here too, and the miss path opens them like any
/// other. One rule for both backends is the point: the interpreter has already
/// opened them ([`register`]), where re-opening is a no-op that `load_one`
/// dedups by canonical path; a `--native` binary has opened nothing, and a
/// required library whose symbols are all resolved lazily may have lost its
/// `DT_NEEDED` entry to `--as-needed`. A flag consulted here would be a fourth
/// place to get the same fact wrong.
#[cfg(feature = "native-extensions")]
static DECLARED_LIBS: std::sync::Mutex<Vec<(CLibrary, Option<bool>)>> =
    std::sync::Mutex::new(Vec::new());

/// Soname → the `#c` symbols declared by the package that declared it. Read by
/// [`library_available`], which is symbol-granular because a library that is
/// PRESENT but of the wrong vintage exports a subset — a file-granular answer
/// would say yes where the call still faults.
#[cfg(feature = "native-extensions")]
static LIB_SYMBOLS: std::sync::Mutex<Vec<(String, Vec<String>)>> =
    std::sync::Mutex::new(Vec::new());

/// Record the C libraries the program declared, for `resolve` to open on
/// demand. Called by [`register`] with the whole list, so a re-registration
/// (the REPL, the test runner) replaces it rather than appending.
#[cfg(feature = "native-extensions")]
pub fn set_declared_libraries(libs: Vec<CLibrary>) {
    *DECLARED_LIBS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) =
        libs.into_iter().map(|l| (l, None)).collect();
}

/// Which declared libraries are EXPECTED to export `symbol`, when the source says so.
///
/// `library_symbol_table` attributes symbols per PACKAGE, and gives a multi-library
/// package an empty list rather than guessing — so this answers EMPTY exactly when the
/// source cannot attribute the symbol, and the caller must widen.
///
/// A package contributes one entry per artefact it owns — its `cc`-built shim AND its
/// declared soname — with the same symbol list on each, so more than one name can claim
/// a symbol. All of them are returned and the caller matches any; picking the first
/// selected the shim path, which no declared library is named after, and the narrowing
/// then never fired.
#[cfg(feature = "native-extensions")]
fn sonames_for_symbol(symbol: &str) -> Vec<String> {
    let guard = LIB_SYMBOLS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard
        .iter()
        .filter(|(_, syms)| syms.iter().any(|s| s == symbol))
        .map(|(lib, _)| lib.clone())
        .collect()
}

/// Record which `#c` symbols each declared library is expected to provide.
#[cfg(feature = "native-extensions")]
pub fn set_library_symbols(table: Vec<(String, Vec<String>)>) {
    *LIB_SYMBOLS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = table;
}

/// Open any declared library not yet tried. Returns whether this call opened
/// one — i.e. whether the set of resolvable symbols just grew, which is the
/// only reason for the caller to look again.
///
/// `only` narrows it to one library by name. A symbol MISS has no name to give —
/// a `#c` annotation never says which library it comes from, so the miss path
/// widens the search across all of them (one resolver, one meaning per
/// symbol). An availability QUESTION does have one, and answering it by opening
/// every optional library made `c_library_available("libsqlite3.so.0")` dlopen
/// libduckdb as a side effect — 70 MB mapped by a program that never mentions
/// duckdb, which is the cost the lazy open exists to avoid.
#[cfg(feature = "native-extensions")]
fn open_pending_optional_named(only: Option<&[String]>) -> bool {
    // The entries are cloned out and the guard dropped before any `dlopen`: an
    // optional library's initialisers run arbitrary C, and holding the lock
    // across that would serialise every parallel arm resolving a symbol (the
    // shape P245 fixed in `native_auto_dispatch`).
    let pending: Vec<CLibrary> = {
        let mut guard = DECLARED_LIBS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .iter_mut()
            .filter(|(lib, tried)| tried.is_none() && only.is_none_or(|ns| ns.contains(&lib.name)))
            .map(|(lib, _)| lib.clone())
            .collect()
    };
    if pending.is_empty() {
        return false;
    }
    let mut opened_any = false;
    for lib in pending {
        let ok = load_c_library(&lib.name, &lib.pkg_dir);
        opened_any |= ok;
        let mut guard = DECLARED_LIBS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        for (entry, tried) in guard.iter_mut() {
            if entry.name == lib.name && entry.pkg_dir == lib.pkg_dir {
                *tried = Some(ok);
            }
        }
    }
    opened_any
}

/// Resolve a C symbol for the running process.
///
/// Searches the cdylibs loft has already loaded, then the process itself —
/// which is what makes a libc binding work with nothing declared and nothing
/// installed. A declared REQUIRED library is already open by the time this runs
/// ([`register`]); an OPTIONAL one is opened here, on the miss, which is the
/// interpreter half of the lazy open.
///
/// One resolver, so a symbol cannot mean two different things depending on
/// which caller asked for it — the one-resolver rule, and the lazy path keeps it by
/// widening the search rather than adding a second search.
#[cfg(feature = "native-extensions")]
#[must_use]
pub fn resolve(symbol: &str) -> Option<*const ()> {
    if let Some(p) = resolve_in_loaded(symbol) {
        return Some(p);
    }
    // Miss: an optional library that has not been opened yet may export it.
    //
    // Try the library the SOURCE attributes this symbol to first. Widening to every
    // declared library still happens below when that does not answer — one resolver,
    // a symbol means one thing — but it is not the first
    // move, because it dlopens libraries the program never mentions. With four
    // one-library packages declared, resolving `sqlite3_open` mapped libduckdb's 70 MB
    // as a side effect, which is the cost `[c] optional-libs` exists to avoid.
    let attributed = sonames_for_symbol(symbol);
    if !attributed.is_empty()
        && open_pending_optional_named(Some(&attributed))
        && let Some(p) = resolve_in_loaded(symbol)
    {
        return Some(p);
    }
    if open_pending_optional_named(None) {
        return resolve_in_loaded(symbol);
    }
    None
}

/// [`resolve`], for a `--native` binary.
///
/// A compiled program never runs [`register`], so nothing has told it which
/// libraries may be opened on demand; the generated crate carries the list as a
/// static and hands it here on the first lazy call. After that this IS
/// [`resolve`] — the two backends share the resolver, which is the only way
/// they can agree about what a symbol means.
///
/// The list carries EVERY declared library, not just the optional ones: a
/// required library is normally a `DT_NEEDED` entry and already in the process,
/// but with its symbols resolved lazily there may be no undefined reference
/// left for the linker to keep it alive (`--as-needed`), and re-opening it by
/// name costs nothing when it is already there.
#[cfg(feature = "native-extensions")]
#[must_use]
pub fn resolve_native(
    symbol: &str,
    libs: &[(&str, &str)],
    syms: &[(&str, &[&str])],
) -> Option<*const ()> {
    register_native(libs, syms);
    resolve(symbol)
}

/// Hand a `--native` binary's baked-in C-library tables to the runtime.
///
/// Idempotent, and called from two places for one reason each: the generated
/// `main` calls it so [`library_available`] can answer in a program that never
/// makes a lazy call at all, and [`resolve_native`] calls it so a `#c` call
/// reaching the runtime BEFORE `main`'s prelude still resolves.
#[cfg(feature = "native-extensions")]
pub fn register_native(libs: &[(&str, &str)], syms: &[(&str, &[&str])]) {
    static INIT: std::sync::Once = std::sync::Once::new();
    INIT.call_once(|| {
        set_declared_libraries(
            libs.iter()
                .map(|(name, pkg_dir)| CLibrary {
                    name: (*name).to_string(),
                    pkg_dir: (*pkg_dir).to_string(),
                    // The flag decides eager loading and the link line, both of
                    // which are already settled by the time a binary exists.
                    // Nothing downstream of here reads it.
                    optional: true,
                })
                .collect(),
        );
        set_library_symbols(
            syms.iter()
                .map(|(lib, s)| {
                    (
                        (*lib).to_string(),
                        s.iter().map(|x| (*x).to_string()).collect(),
                    )
                })
                .collect(),
        );
    });
}

/// Is this C library usable RIGHT NOW?
///
/// True when the library opens **and** every `#c` symbol declared against it
/// resolves. Both halves are load-bearing: a library of the wrong vintage opens
/// and exports a subset, so a file-granular answer would say yes where the call
/// still faults — the version-skew hole that makes a naive query worse than no
/// query at all.
///
/// A library the program never declared answers false rather than probing the
/// dynamic linker for it. Asking about one is a program bug, and false is the
/// answer that keeps a caller on its fallback path instead of into a fault
/// (@C80: no runtime errors, ever).
/// [`library_available`], for generated Rust — the main binary AND every
/// auto-built package cdylib.
///
/// A cdylib links its own copy of loft, so the tables [`register`] filled in
/// the interpreter's process are NOT the tables a function compiled into a
/// package can see: measured, `duckdb_available()` answered false from inside
/// the package while the identical call from the program answered true. The
/// generated source carries the tables, so passing them is what makes one
/// question have one answer wherever it is asked from.
#[cfg(feature = "native-extensions")]
#[must_use]
pub fn library_available_native(
    name: &str,
    libs: &[(&str, &str)],
    syms: &[(&str, &[&str])],
) -> bool {
    register_native(libs, syms);
    library_available(name)
}

#[cfg(feature = "native-extensions")]
#[must_use]
pub fn library_available(name: &str) -> bool {
    open_pending_optional_named(Some(std::slice::from_ref(&name.to_string())));
    let opened = {
        let guard = DECLARED_LIBS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .iter()
            .filter(|(lib, _)| lib.name == name)
            .map(|(_, tried)| tried.unwrap_or(false))
            .fold(None, |acc: Option<bool>, ok| {
                Some(acc.unwrap_or(false) || ok)
            })
    };
    if opened != Some(true) {
        return false;
    }
    let symbols = {
        let guard = LIB_SYMBOLS
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        guard
            .iter()
            .find(|(lib, _)| lib == name)
            .map(|(_, s)| s.clone())
            .unwrap_or_default()
    };
    symbols.iter().all(|s| resolve(s).is_some())
}

/// What a `#c` call says when its symbol cannot be resolved.
///
/// One text for both backends: the interpreter panics with it from `dispatch`,
/// and the `--native` emission writes a call to this same function into the
/// generated crate. A message that differed between them would be a divergence
/// in the one place a user is already having a bad day.
#[cfg(feature = "native-extensions")]
#[must_use]
pub fn missing_symbol_message(symbol: &str) -> String {
    format!(
        "`#c` symbol '{symbol}' not found — it is not in this process and no loaded \
         library exports it. If it comes from an `[c] optional-libs` library, that \
         library is not installed: ask `c_library_available(\"<soname>\")` before \
         calling. Otherwise declare the library it comes from, or check the spelling"
    )
}

/// The search itself, over what is already open. Split out so the miss path can
/// re-run exactly the same search after widening it, rather than a similar one.
#[cfg(feature = "native-extensions")]
fn resolve_in_loaded(symbol: &str) -> Option<*const ()> {
    if let Some((p, _)) = try_dlsym_pub(symbol) {
        return Some(p);
    }
    // Then what the process already has open — the process handle on Unix, the
    // already-loaded CRT and base modules on Windows (`platform::symbol_in_process`).
    crate::platform::symbol_in_process(symbol)
}

/// @PLN174 F5 — set in a `LoftStoreCtx` word above the store number when the store was
/// MINTED for the bridge's vector return and holds nothing yet: the foreign callback
/// then adopts the bytes into that store instead of minting another (which would leave
/// the minted one orphaned).  The cdylib reads only the low 16 bits (`LoftStore::store_nr`)
/// and passes the word back untouched, so the bit is the host's to use.
pub const CTX_RETURN_STORE: usize = 1 << 16;

/// The foreign-vector callback's body, shared by the interpreter's dispatcher
/// (`CURRENT_STORES`) and the generated `--native` call (`native_call`, the same cell):
/// adopt `len` bytes at `ptr` — `(ptr, len, cap, release)` a cdylib's `Vec<u8>` — as a
/// read-only `vector<u8>`, into the return store when `ctx` says it was minted for this
/// answer, into a fresh store otherwise.  Answers the bare `FOREIGN_REC` for the
/// consumer to turn into the store's handle (`bridge_push_ref`), or the null ref when the
/// block is declined (a length past `u32`), which leaves it the cdylib's to free.
pub(crate) fn adopt_foreign_block(
    stores: &mut crate::database::Stores,
    ctx: usize,
    ptr: *mut u8,
    len: usize,
    cap: usize,
    elem_size: u32,
    release: unsafe extern "C" fn(*mut u8, usize, usize),
) -> loft_ffi::LoftRef {
    let null = loft_ffi::LoftRef {
        store_nr: 0,
        rec: 0,
        pos: 0,
    };
    let Ok(count) = u32::try_from(len / elem_size.max(1) as usize) else {
        return null;
    };
    if elem_size > 1 && !(ptr as usize).is_multiple_of(elem_size as usize) {
        return null;
    }
    let owner = crate::store::ForeignOwner::Extern {
        ptr,
        len,
        cap,
        release,
    };
    let db = if ctx & CTX_RETURN_STORE != 0 {
        stores.foreign_vector_in(ctx as u16, ptr, count, elem_size, owner)
    } else {
        stores.foreign_vector(ptr, count, elem_size, owner)
    };
    loft_ffi::LoftRef {
        store_nr: db.store_nr,
        rec: crate::store::FOREIGN_REC,
        pos: 8,
    }
}
