// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader: where a native program finds its libraries
//! Where a native program finds its libraries (@PLN195).
//!
//! A dynamic native program keeps each native package's Rust as a separate library file
//! (`.so` / `.dylib` / `.dll`).  It does not ask the OS loader to bind them before `main`:
//! the generated code calls each library function through a slot, and [`bind`] fills the
//! slots at start from a search order loft owns, the same on every OS:
//!
//! 1. `LOFT_LIB_PATH`, a list of directories;
//! 2. the binary's own directory — a `dist/` directory written by `loft build`;
//! 3. the per-user root `~/.loft/lib/` (`LOFT_HOME` moves it);
//! 4. the system root, `/usr/local/lib/loft/` (`%ProgramData%\loft\lib\` on Windows).
//!
//! The command line has no part in it: a program's arguments are its own, and the same on
//! every backend — an interpreted run cannot drop an argument a compiled one reserved.  The
//! variable is never stripped where macOS strips `DYLD_LIBRARY_PATH` (a start through
//! `/bin/sh` or `/usr/bin/env`, as a `#!` script does).
//!
//! In each directory a library is looked for in its versioned subdirectory
//! `<package>-<version>/` first.  Directories 1 and 2 also hold it flat, beside the binary:
//! that is the layout `loft build` writes, where the library came with the binary.  The two
//! shared roots (3, 4) hold only versioned subdirectories, so programs built against two
//! versions of one library each load their own.
//!
//! The libraries are the native packages' own (a dynamic link), and the C libraries packages
//! ship — a `[c] shim` or a file beside the package — that are not linked into the binary
//! (@PLN197).  A row with no file is a C library that IS linked in; binding it only marks it
//! present for `c_library_available`.
//!
//! Each symbol is looked up in ITS library's handle, never in a process-wide namespace, so
//! two packages exporting one name cannot bind to each other's function; the @PLN26 rule
//! that two packages may not DECLARE one symbol is still refused at codegen.
//!
//! A library that is found nowhere is a loft error naming the package, the version wanted,
//! every directory searched and any other version of it that was there.  No directory of
//! the building machine is written into the binary: the driver that runs a program it just
//! built hands it the build's library directories through `LOFT_LIB_PATH`.

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicPtr;

/// Where each package's library was loaded from, for [`unbound`] to name.
static LOADED_FROM: std::sync::Mutex<Vec<(&'static str, PathBuf)>> =
    std::sync::Mutex::new(Vec::new());

/// One function a program calls in a library: the exported symbol and the slot the
/// generated code calls through.  The slot starts at a stub that binds on first use.
pub struct Symbol {
    pub name: &'static str,
    pub slot: &'static AtomicPtr<()>,
}

/// One native library a program links: the package and version it was built against, the
/// file's name on this OS, and the functions the program calls in it.  `c_name` is set for a
/// C library a package ships (@PLN197): the name it goes by in the program's `__C_LIBS`
/// table, so the `#c` resolver and `c_library_available` see it as loaded.  Empty for a
/// package's own Rust library.
pub struct Library {
    pub package: &'static str,
    pub version: &'static str,
    pub file: &'static str,
    pub c_name: &'static str,
    pub symbols: &'static [Symbol],
}

/// The `__C_LIBS` names of the C libraries [`bind`] loaded.
static BOUND_C: std::sync::Mutex<Vec<&'static str>> = std::sync::Mutex::new(Vec::new());

/// @PLN197 — did [`bind`] load the C library the program's `__C_LIBS` names `name`?  Then
/// it is open, its symbols are in `c_libs`' loaded set, and the `#c` resolver must not go
/// looking for it by name — which would fail, since its directory is not in the binary.
#[must_use]
pub fn is_bound_c(name: &str) -> bool {
    BOUND_C
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .contains(&name)
}

/// Fill every slot of `libs` once, from the search order above.  A library found nowhere
/// ends the program with exit code 1 and the message [`missing_library`] gives.  A symbol
/// the library does not export leaves its slot at the first-use stub, which reports it by
/// name when (and only when) it is called — [`unbound`].
pub fn bind(libs: &'static [Library]) {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let dirs = search_dirs();
        for lib in libs {
            if let Err(message) = bind_one(lib, &dirs) {
                eprintln!("{message}");
                std::process::exit(1);
            }
        }
    });
}

/// The first-use stub of a slot found its symbol still unbound after [`bind`]: the library
/// loaded but does not export it.  Names the symbol and the library and exits.
pub fn unbound(symbol: &str, libs: &'static [Library]) -> ! {
    let lib = libs
        .iter()
        .find(|l| l.symbols.iter().any(|s| s.name == symbol));
    match lib {
        Some(lib) => {
            let from = LOADED_FROM
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .find(|(p, _)| *p == lib.package)
                .map(|(_, path)| format!(" ({})", path.display()))
                .unwrap_or_default();
            eprintln!(
                "loft: the native library of package `{}` {} does not export `{symbol}` — the \
                 file loaded{from} is not the version this program was built against",
                lib.package, lib.version
            );
        }
        None => eprintln!("loft: native function `{symbol}` is not bound to any library"),
    }
    std::process::exit(1);
}

/// The directories searched, in order, each with whether it may hold a library flat (beside
/// the binary) or only in a `<package>-<version>/` subdirectory.
pub(crate) fn search_dirs() -> Vec<(PathBuf, bool)> {
    let mut dirs = Vec::new();
    if let Some(list) = std::env::var_os("LOFT_LIB_PATH") {
        for d in std::env::split_paths(&list) {
            if !d.as_os_str().is_empty() {
                dirs.push((d, true));
            }
        }
    }
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|e| crate::file_access::try_plain_canonical(e.as_path()))
        .and_then(|e| e.parent().map(Path::to_path_buf))
    {
        dirs.push((dir, true));
    }
    if let Some(home) = std::env::var_os("LOFT_HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
    {
        dirs.push((home.join(".loft").join("lib"), false));
    }
    dirs.push((crate::platform::system_library_root(), false));
    dirs
}

/// Every place `lib` may be in `dirs`, in search order.
#[cfg(feature = "native-extensions")]
fn candidates(lib: &Library, dirs: &[(PathBuf, bool)]) -> Vec<PathBuf> {
    let versioned = format!("{}-{}", lib.package, lib.version);
    let mut out = Vec::new();
    for (dir, flat) in dirs {
        out.push(dir.join(&versioned).join(lib.file));
        if *flat {
            out.push(dir.join(lib.file));
        }
    }
    out
}

#[cfg(feature = "native-extensions")]
fn bind_one(lib: &'static Library, dirs: &[(PathBuf, bool)]) -> Result<(), String> {
    // A row with no file is a C library linked INTO the binary (@PLN197): nothing to load,
    // only its name to mark present.
    if lib.file.is_empty() {
        BOUND_C
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(lib.c_name);
        return Ok(());
    }
    let mut refused = Vec::new();
    for path in candidates(lib, dirs) {
        if !crate::file_access::is_file(path.as_path()) {
            continue;
        }
        // SAFETY: loading a library runs its initialisers; this is the library the program
        // was built to call, found where the program says to look.
        let handle = match unsafe { crate::c_libs::open_library(&path.to_string_lossy()) } {
            Ok(h) => h,
            Err(e) => {
                refused.push(crate::c_libs::dlopen_diagnostic(
                    &path.to_string_lossy(),
                    &e,
                ));
                continue;
            }
        };
        for sym in lib.symbols {
            let mut name = sym.name.as_bytes().to_vec();
            name.push(0);
            // SAFETY: the pointer is only stored; the generated caller transmutes it to the
            // signature the package declared, the same contract an `extern "C"` block makes.
            if let Ok(p) = unsafe { handle.get::<*const ()>(&name) } {
                sym.slot
                    .store((*p).cast_mut(), std::sync::atomic::Ordering::Release);
            }
        }
        // Kept for the process: the slots point into it.
        std::mem::forget(handle);
        // A C library joins `c_libs`' loaded set too, so a `#c` symbol of its package that
        // is resolved by name (a lazy one, or the slot fallback) finds it there.
        if !lib.c_name.is_empty() && crate::c_libs::load_one(&path.to_string_lossy()) {
            BOUND_C
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(lib.c_name);
        }
        LOADED_FROM
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push((lib.package, path));
        return Ok(());
    }
    Err(missing_library(lib, dirs, &refused))
}

#[cfg(not(feature = "native-extensions"))]
fn bind_one(lib: &'static Library, dirs: &[(PathBuf, bool)]) -> Result<(), String> {
    Err(missing_library(lib, dirs, &[]))
}

/// The message for a library found nowhere: the package, the version wanted, every
/// directory searched, any other version of the package those directories hold, and why a
/// file that was there would not load.
fn missing_library(lib: &Library, dirs: &[(PathBuf, bool)], refused: &[String]) -> String {
    use std::fmt::Write as _;
    let mut m = format!(
        "loft: cannot find the native library of package `{}` {} ({})\n  searched:",
        lib.package, lib.version, lib.file
    );
    for (dir, _) in dirs {
        let _ = write!(m, "\n    {}", dir.display());
    }
    let other: Vec<String> = dirs
        .iter()
        .filter_map(|(dir, _)| crate::file_access::read_dir(dir.as_path()).ok())
        .flatten()
        .map(|p| p.native())
        .filter_map(|p| {
            let name = crate::file_access::file_name(p.as_str())?;
            let ver = name.strip_prefix(&format!("{}-", lib.package))?;
            (ver != lib.version && ver.starts_with(|c: char| c.is_ascii_digit())).then(|| p.clone())
        })
        .collect();
    if !other.is_empty() {
        let _ = write!(m, "\n  other versions found (not used):");
        for p in &other {
            let _ = write!(m, "\n    {p}");
        }
    }
    for r in refused {
        let _ = write!(m, "\n  {r}");
    }
    let _ = write!(
        m,
        "\n  fix: put `{}` beside the program, in `{}-{}/` under one of these directories, or \
         name its directory with LOFT_LIB_PATH",
        lib.file, lib.package, lib.version
    );
    m
}
