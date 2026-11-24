// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader: finding a package's native library

//! Finding a `use`d package's native library — the component loft ships that the package
//! names — and the facts around it (the target root, the missing-system-library diagnostic).
//! The parser asks it when it resolves a `use`, so it sits in the core layer; loading the
//! library and calling into it stay in `extensions` (@PLN191).  A package's own native
//! library, built by cargo or shipped prebuilt, is no longer loaded (@PLN225).

/// Compute the target dir for a package's
/// native cdylib/rlib build.
///
/// When `pkg_dir` is under `~/.loft/registry/` (i.e., a
/// `loft install`-extracted chunk), returns the redirected
/// `~/.loft/build-cache/<pkg>-<ver>/` path; otherwise returns
/// `<pkg_dir>/native/target` (the in-tree default).
///
/// Used by `native_utils::add_native_extern_flags` (to find the rlib at
/// link time).
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
        let registry_cache_canon = crate::file_access::try_plain_canonical(&registry_cache);
        let pkg_canon = crate::file_access::try_plain_canonical(std::path::Path::new(pkg_dir));
        let use_redirected = match (&registry_cache_canon, &pkg_canon) {
            (Some(rc), Some(pc)) => pc.starts_with(rc),
            _ => false,
        };
        if use_redirected {
            let stem_dir =
                crate::file_access::file_name(pkg_dir).unwrap_or_else(|| "native-pkg".to_string());
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

/// Set for a `loft --html` run: the program is built for the browser, which never loads a
/// package's HOST native library — the page answers the package's imports itself.  So the
/// parse registers no host cdylib and the driver builds no default-native one: either
/// is a full cargo build of code the build never links (`html_gl_imports` spent
/// minutes of a 327 s run there on a cold runner).  The program cache keys on `--html`
/// (`startup_cache::native_lib_context`), so a browser run's bundle is never warm-loaded by
/// a run that needs those libraries.
pub(crate) static BROWSER_TARGET: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Mark this run as a browser build (`loft --html`), before anything resolves a package.
pub fn set_browser_target() {
    BROWSER_TARGET.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Is this run a browser build?  Then no host native library is built or registered.
#[must_use]
pub fn browser_target() -> bool {
    BROWSER_TARGET.load(std::sync::atomic::Ordering::Relaxed)
}

/// Set for a run the C backend compiles with no way back to the interpreter (`--emit-c`, an
/// explicit `--native-c`): the C links a package's `[c] shim` and the components, never its
/// Rust cdylib, so building one is a full cargo build of code the run never loads — a
/// `use graphics` spent over 25 minutes in winit and glutin before a word of C.
pub(crate) static C_TARGET: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Mark this run as a C build, before anything resolves a package.
pub fn set_c_target() {
    C_TARGET.store(true, std::sync::atomic::Ordering::Relaxed);
}

/// Is this run a C build?  Then no host native library is built or registered.
#[must_use]
pub fn c_target() -> bool {
    C_TARGET.load(std::sync::atomic::Ordering::Relaxed)
}

/// The host native library a `use`d package registers — none for a browser build or a C
/// build, which never load one ([`browser_target`], [`c_target`]); otherwise
/// [`resolve_native_lib`].
#[must_use]
pub fn host_native_lib(pkg_dir: &str, stem: &str) -> Option<String> {
    if browser_target() || c_target() {
        return None;
    }
    resolve_native_lib(pkg_dir, stem)
}

/// Resolve a package's native registration (`stem`, the library its component stands for) to a
/// loadable library path: [`prebuilt_native_lib`], unless a declared runtime system library is
/// missing.  The ONE home for this resolution — the parser's two manifest paths and the warm
/// startup-cache load (#310) all derive from it, so a cached run resolves exactly like a cold
/// parse.
pub fn resolve_native_lib(pkg_dir: &str, stem: &str) -> Option<String> {
    // @PLN21 Phase 3 — a missing DECLARED runtime system lib is terminal: the library
    // links it, so it cannot load.  Check FIRST and emit an actionable hint.
    if let Some(lib) = first_missing_runtime_lib(pkg_dir) {
        eprintln!("{}", runtime_lib_missing_diagnostic(stem, &lib));
        return None;
    }
    prebuilt_native_lib(pkg_dir)
}

/// The component a package's natives are (`[native] component`), if they are one.
pub(crate) fn component_of(pkg_dir: &str) -> Option<String> {
    crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml")).and_then(|m| m.native_component)
}

/// Why a package's native part has nothing to load on this host, for a refusal: the component
/// loft ships that it names is not installed.  None for a package that names no component.
#[must_use]
pub fn unbuildable_native_reason(pkg_dir: &str) -> Option<String> {
    component_of(pkg_dir).map(|name| crate::components::missing(&name))
}

/// The native library a package's `#native` functions load from: the component loft ships
/// that the package names (@PLN205), never a build.  What [`resolve_native_lib`] answers and
/// all the C backend links.  A package's own native library is refused when its manifest is
/// read (`manifest::RUST_NATIVE_REFUSED`, @PLN225), so a component is the only one there is.
#[must_use]
pub fn prebuilt_native_lib(pkg_dir: &str) -> Option<String> {
    component_of(pkg_dir).and_then(|name| crate::components::path(&name))
}

/// The host OS as a display name, for runtime-lib diagnostics + platform-scoping.
/// Not feature-gated: `runtime_lib_missing_diagnostic` (and thus its call site in
/// the unconditionally-compiled `resolve_native_lib`) needs it even when
/// `native-extensions` is off (e.g. the WASM build).
pub(crate) fn host_os_name() -> &'static str {
    crate::platform::host_os_name()
}

#[cfg(not(feature = "native-extensions"))]
pub(crate) fn first_missing_runtime_lib(_pkg_dir: &str) -> Option<String> {
    None
}

/// Diagnostic for a host-applicable `[native] runtime-libs` entry that could not
/// be `dlopen`'d — the library is genuinely not installed.  Foreign-OS names are
/// skipped before this point (see [`first_missing_runtime_lib`]), so they never
/// reach here and never produce misleading "install it" advice.  Not
/// feature-gated: its call site in `resolve_native_lib` compiles unconditionally
/// (the `first_missing_runtime_lib` if-branch is dead but still type-checked when
/// `native-extensions` is off), so the helper must exist in every build.
pub(crate) fn runtime_lib_missing_diagnostic(stem: &str, lib: &str) -> String {
    format!(
        "loft: native library '{stem}' needs the system library '{lib}', which is not installed \
         on {host} ({triple}) — install it with your OS package manager (building from source \
         would link the same library and fail identically).",
        host = host_os_name(),
        triple = crate::cache::host_triple(),
    )
}

/// Resolve the platform-correct shared-library filename from a stem.
#[must_use]
pub fn platform_lib_name(stem: &str) -> String {
    crate::platform::cdylib_file_name(stem)
}

/// The OS a library NAME targets, inferred from its soname/extension form, or
/// `None` when it can't be told (extensionless / unrecognised).  Soname matching,
/// not file-extension parsing: `.so` is matched as a substring because versioned
/// sonames are `.so.N` (which `Path::extension` would read as "N"), and sonames
/// are lowercase by platform convention — so case-sensitive substring checks are
/// correct here.  The single source of truth for runtime-lib platform-scoping.
#[cfg(feature = "native-extensions")]
pub(crate) fn lib_name_target_os(lib: &str) -> Option<&'static str> {
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

/// @PLN21 Phase 3 — the first host-applicable `[native] runtime-libs` entry the
/// dynamic linker can't find, if any.  `dlopen` IS the authoritative presence
/// check: a declared lib that loads is present; one that fails is missing, so the
/// package's native library could not load.
///
/// A runtime-lib named for a DIFFERENT OS than the host (e.g. a Linux `libGL.so.1`
/// declared by a library used on macOS) is **skipped, not probed**: it can never
/// load here and is not a host requirement — the host build of the cdylib links
/// its own platform libraries.  Probing it would wrongly hard-fail the whole
/// library on every foreign platform.  Only host-applicable (or unclassifiable)
/// names gate resolution.  Empty `runtime-libs` → no check, no cost.
#[cfg(feature = "native-extensions")]
pub(crate) fn first_missing_runtime_lib(pkg_dir: &str) -> Option<String> {
    let host = host_os_name();
    crate::manifest::read_manifest(&format!("{pkg_dir}/loft.toml"))?
        .runtime_libs
        .into_iter()
        .filter(|lib| lib_name_target_os(lib).is_none_or(|os| os == host))
        // SAFETY: loading runs the library's initialisers; a declared runtime library is meant to load.
        .find(|lib| unsafe { crate::file_access::load_library(lib) }.is_err())
}
