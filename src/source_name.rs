// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native generators: the name a source file is given in what a build writes

//! The name a source file is given in what a build writes and in what a program prints when
//! it faults: relative to the file's ROOT, never to where the checkout happens to be, so the
//! same program builds to the same bytes in any directory (@PLN229, reproducible builds).
//!
//! - a standard-library file is `default/<file>`;
//! - a file beneath the main file's directory is named from that directory;
//! - any other file — a library found elsewhere — from its package's parent directory (the
//!   directory above the nearest `loft.toml`), and by its file name when it has none;
//! - a name that is already relative is kept: a built program's names arrive named.
//!
//! The name is for PRINTING.  A source line shown under a fault is read from the file itself:
//! [`locate`] finds it again from a name.

use std::path::{Path, PathBuf};

/// The two roots a program's files are named from: the main file's directory and the
/// standard library's directory.  Either may be empty when unknown.
#[derive(Clone, Default, Debug, PartialEq, Eq)]
pub struct Roots {
    pub main_dir: String,
    pub std_dir: String,
}

impl Roots {
    /// The roots of a program whose main file is `main_file` and whose standard library was
    /// read from the directory holding `std_file`.
    #[must_use]
    pub fn of(main_file: &str, std_file: &str) -> Self {
        let dir = |f: &str| {
            Path::new(f)
                .parent()
                .map_or_else(String::new, |p| p.to_string_lossy().into_owned())
        };
        Self {
            main_dir: dir(main_file),
            std_dir: dir(std_file),
        }
    }
}

/// The roots of the program this process runs, once its driver has named them; a fault the
/// interpreter renders is named from them ([`crate::runtime_error::RuntimeError::render`]).
static PROGRAM_ROOTS: std::sync::RwLock<Option<Roots>> = std::sync::RwLock::new(None);

/// Name the roots of the program this process runs.
pub fn set_program_roots(roots: Roots) {
    *PROGRAM_ROOTS
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(roots);
}

/// The roots [`set_program_roots`] named; `None` before it did, when a fault keeps the file as
/// its position holds it.
#[must_use]
pub fn program_roots() -> Option<Roots> {
    PROGRAM_ROOTS
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// The roots a unit or a generated program names its sources from: `hosted_root` for a hosted
/// unit (its package's parent), else the directory of the program's `main`; and the directory
/// the standard library was read from.  Either is empty when the definitions hold none.
#[must_use]
pub fn roots_of(data: &crate::data::Data, hosted_root: Option<String>) -> Roots {
    let file_of = |d: u32| {
        (d < data.definitions())
            .then(|| data.def(d).position.file.to_string())
            .filter(|f| !f.is_empty())
    };
    let std_file = (0..data.definitions())
        .find(|&d| data.def(d).source == crate::data::STD_SOURCE && file_of(d).is_some())
        .and_then(file_of)
        .unwrap_or_default();
    let mut roots = Roots::of("", &std_file);
    roots.main_dir = match hosted_root {
        Some(root) => root,
        None => Roots::of(&file_of(data.def_nr("n_main")).unwrap_or_default(), "").main_dir,
    };
    roots
}

/// `rest` when `file` lies beneath directory `dir`.  Both are compared in their `/` spelling: on
/// Windows a package directory arrives with the host's separator and a source position may hold
/// either, so a bare prefix test would miss and name the machine's path.
fn beneath<'a>(file: &'a str, dir: &str) -> Option<&'a str> {
    let dir = crate::file_access::portable_str(dir);
    let dir = dir.trim_end_matches('/');
    if dir.is_empty() {
        return None;
    }
    file.strip_prefix(dir)?.strip_prefix('/')
}

/// The name `file` is printed and written under (the module documentation's rule).
#[must_use]
pub fn name(file: &str, roots: &Roots) -> String {
    if !Path::new(file).is_absolute() {
        return file.to_string();
    }
    let portable = crate::file_access::portable_str(file);
    let file = portable.as_str();
    if let Some(rest) = beneath(file, &roots.std_dir) {
        return format!("default/{rest}");
    }
    if let Some(rest) = beneath(file, &roots.main_dir) {
        return rest.to_string();
    }
    let path = Path::new(file);
    let package_parent = path
        .ancestors()
        .skip(1)
        .find(|d| crate::file_access::exists(d.join("loft.toml")))
        .and_then(Path::parent);
    if let Some(parent) = package_parent
        && let Ok(rest) = path.strip_prefix(parent)
    {
        return rest.to_string_lossy().into_owned();
    }
    crate::file_access::file_name(path).unwrap_or_else(|| file.to_string())
}

/// The file a printed `name` stands for, to show its source line: the name itself when it can
/// be read as given (an absolute path, or one relative to the working directory), else beneath
/// `LOFT_SOURCE_ROOT`, else beneath the roots.  `None` when none of them holds it.
#[must_use]
pub fn locate(name: &str, roots: &Roots) -> Option<PathBuf> {
    let given = PathBuf::from(name);
    if crate::file_access::exists(&given) {
        return Some(given);
    }
    if given.is_absolute() {
        return None;
    }
    let mut bases: Vec<PathBuf> = Vec::new();
    if let Ok(root) = std::env::var("LOFT_SOURCE_ROOT")
        && !root.is_empty()
    {
        bases.push(PathBuf::from(root));
    }
    if let Some(rest) = name.strip_prefix("default/")
        && !roots.std_dir.is_empty()
    {
        let p = Path::new(&roots.std_dir).join(rest);
        if crate::file_access::exists(&p) {
            return Some(p);
        }
    }
    if !roots.main_dir.is_empty() {
        bases.push(PathBuf::from(&roots.main_dir));
    }
    bases
        .into_iter()
        .map(|b| b.join(name))
        .find(|p| crate::file_access::exists(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Roots {
        Roots {
            main_dir: "/work/app".into(),
            std_dir: "/opt/loft/default".into(),
        }
    }

    #[test]
    fn a_standard_library_file_is_named_from_default() {
        assert_eq!(
            name("/opt/loft/default/02_files.loft", &roots()),
            "default/02_files.loft"
        );
    }

    #[test]
    fn a_program_file_is_named_from_the_main_directory() {
        assert_eq!(name("/work/app/main.loft", &roots()), "main.loft");
        assert_eq!(
            name("/work/app/lib/shapes.loft", &roots()),
            "lib/shapes.loft"
        );
    }

    #[test]
    fn a_relative_name_is_kept() {
        assert_eq!(name("lib/shapes.loft", &roots()), "lib/shapes.loft");
    }

    #[test]
    fn a_file_outside_every_root_without_a_package_is_its_file_name() {
        assert_eq!(name("/elsewhere/odd/x.loft", &roots()), "x.loft");
    }

    #[test]
    fn a_directory_prefix_is_not_a_root() {
        // `/work/application` is not beneath `/work/app`.
        assert_eq!(name("/work/application/m.loft", &roots()), "m.loft");
    }
}
