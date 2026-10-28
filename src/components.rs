// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader: the prebuilt parts loft ships beside its binary

//! Components: the parts of loft that ship as prebuilt libraries beside the binary rather than
//! inside it (@PLN205) — native code a loft program calls, such as `libloft_net` (HTTP, TLS,
//! WebSockets).  A package names one with `[native] component = "<name>"`; its `#native`
//! functions are then the component's `n_*` symbols, behind the loft-ffi C ABI
//! (`loft-ffi/include/loft_ffi.h`), resolved on all three backends as a package's own
//! `prebuilt/<triple>/` library is (`native_build::prebuilt_native_lib`).
//!
//! A component is never built when it is used: the release builds it for each platform, and a
//! development tree builds it with `make components`.  Where one is looked for, in order:
//!
//! 1. `LOFT_COMPONENTS`, a list of directories;
//! 2. `<binary>/../lib/loft/components/` — an installed prefix (`bin/loft`, `lib/loft/`);
//! 3. `<binary>/../components/` — a development build (`target/debug/loft`, `target/components/`);
//! 4. `components/` in each directory a compiled program searches for its libraries
//!    (`native_bind::search_dirs`): `LOFT_LIB_PATH`, the binary's own directory, `~/.loft/lib/`,
//!    the system root.
//!
//! A file is taken only when the loft-ffi fingerprint beside it (`.loft-build-fp`) is this
//! loft's, as for a package's prebuilt library.

use std::path::PathBuf;

/// The library file of component `name` on this OS: `libloft_<name>.so` and its kin.
#[must_use]
pub fn file_name(name: &str) -> String {
    crate::native_build::platform_lib_name(&stem(name))
}

/// The stem a component's library is named by: `loft_<name>`.
#[must_use]
pub fn stem(name: &str) -> String {
    format!("loft_{name}")
}

/// The directories searched for a component, in order.
#[must_use]
pub fn dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(list) = std::env::var_os("LOFT_COMPONENTS") {
        out.extend(std::env::split_paths(&list).filter(|d| !d.as_os_str().is_empty()));
    }
    if let Some(prefix) = std::env::current_exe()
        .ok()
        .and_then(|e| crate::file_access::try_plain_canonical(e.as_path()))
        .and_then(|e| {
            e.parent()
                .and_then(std::path::Path::parent)
                .map(std::path::Path::to_path_buf)
        })
    {
        out.push(prefix.join("lib").join("loft").join("components"));
        out.push(prefix.join("components"));
    }
    out.extend(
        crate::native_bind::search_dirs()
            .into_iter()
            .map(|(d, _)| d.join("components")),
    );
    out
}

/// The library of component `name` for this loft, or `None` when no searched directory holds
/// one built against this loft-ffi.
#[must_use]
pub fn path(name: &str) -> Option<String> {
    let file = file_name(name);
    dirs().into_iter().find_map(|d| {
        let lib = d.join(&file);
        (crate::file_access::exists(&lib)
            && crate::cache::native_artifact_fingerprint_matches(
                &d,
                crate::cache::loft_ffi_fingerprint(),
            ))
        .then(|| lib.to_string_lossy().into_owned())
    })
}

/// Why component `name` is missing, for a refusal: what was looked for and where.
#[must_use]
pub fn missing(name: &str) -> String {
    let dirs: Vec<String> = dirs()
        .iter()
        .map(|d| d.to_string_lossy().into_owned())
        .collect();
    format!(
        "loft's component `{name}` ({}) is not installed: looked in {}",
        file_name(name),
        dirs.join(", ")
    )
}
