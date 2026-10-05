// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! The compiler's ONE way to reach the file system.
//!
//! **The rule:** compiler code (`src/`) never opens, lists, creates, removes or inspects a
//! file or directory from a string or a bare `Path` — it builds a [`PathText`] and calls
//! the operation here.  `guard` counts every direct access outside this module against a
//! baseline that may only shrink, so a new one fails the build and the old ones migrate.
//!
//! Why: loft programs reach files through the runtime, which is platform-neutral; the
//! COMPILER's own file handling kept attracting Windows defects, each a decision taken on
//! path text — a mixed `D:\a\loft/default\x` missing a `"/default/"` pattern, a verbatim
//! `\\?\D:\…` that never equals its plain twin, a `dir/` that is not `dir`, `pkg` claiming
//! `pkg2/x`, an error that says "No such file or directory" without saying which.  Here a
//! path is parsed once under an explicit flavor ([`path`]), so every Windows rule is a unit
//! test on any host, and every operation's error names its path.

pub mod path;

pub use path::{Flavor, PathText};

use std::io;
use std::path::{Path, PathBuf};

impl PathText {
    /// A host path the OS handed over (`current_exe`, a directory listing, a home dir).
    /// Lossy for a name that is not UTF-8, which the compiler's sources never are.
    #[must_use]
    pub fn from_os(path: &Path) -> PathText {
        PathText::host(&path.to_string_lossy())
    }

    /// The spelling the OS is handed.  Only a HOST path reaches the OS: a path parsed
    /// under the other flavor (a test's Windows path on Linux) is not a file here.
    fn os(&self) -> io::Result<PathBuf> {
        if self.flavor() == Flavor::HOST {
            Ok(PathBuf::from(self.native()))
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{}: not a path on this platform", self.portable()),
            ))
        }
    }
}

/// Name the path in an error: a bare `io::Error` says what failed, never where.
fn named<T>(path: &PathText, r: io::Result<T>) -> io::Result<T> {
    r.map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.portable())))
}

fn run<T>(path: &PathText, op: impl FnOnce(&Path) -> io::Result<T>) -> io::Result<T> {
    let os = path.os()?;
    named(path, op(&os))
}

/// The file's text.
///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn read_to_string(path: &PathText) -> io::Result<String> {
    run(path, |p| std::fs::read_to_string(p))
}

/// The file's bytes.
///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn read(path: &PathText) -> io::Result<Vec<u8>> {
    run(path, |p| std::fs::read(p))
}

/// Replace the file's content (creating it).
///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn write(path: &PathText, contents: impl AsRef<[u8]>) -> io::Result<()> {
    run(path, |p| std::fs::write(p, contents))
}

/// Create the directory and every missing parent.
///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn create_dir_all(path: &PathText) -> io::Result<()> {
    run(path, |p| std::fs::create_dir_all(p))
}

///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn remove_file(path: &PathText) -> io::Result<()> {
    run(path, |p| std::fs::remove_file(p))
}

/// Remove the directory and everything in it.
///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn remove_dir_all(path: &PathText) -> io::Result<()> {
    run(path, |p| std::fs::remove_dir_all(p))
}

///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn copy(from: &PathText, to: &PathText) -> io::Result<u64> {
    let dest = to.os()?;
    run(from, |p| std::fs::copy(p, &dest))
}

///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn rename(from: &PathText, to: &PathText) -> io::Result<()> {
    let dest = to.os()?;
    run(from, |p| std::fs::rename(p, &dest))
}

/// Open the file for reading.
///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn open(path: &PathText) -> io::Result<std::fs::File> {
    run(path, |p| std::fs::File::open(p))
}

/// Create (or truncate) the file for writing.
///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn create(path: &PathText) -> io::Result<std::fs::File> {
    run(path, |p| std::fs::File::create(p))
}

/// Open an existing file for reading AND writing, neither creating nor truncating it.
///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn open_read_write(path: &PathText) -> io::Result<std::fs::File> {
    run(path, |p| {
        std::fs::OpenOptions::new().read(true).write(true).open(p)
    })
}

/// Open the file with `options` (append, create-new, …).
///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn open_with(path: &PathText, options: &std::fs::OpenOptions) -> io::Result<std::fs::File> {
    run(path, |p| options.open(p))
}

///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn metadata(path: &PathText) -> io::Result<std::fs::Metadata> {
    run(path, |p| std::fs::metadata(p))
}

/// The entries of a directory, SORTED: listing order is the file system's (hash order on
/// one, name order on another), and nothing the compiler derives from a listing may
/// depend on which machine produced it.
///
/// # Errors
/// The OS's error, naming the path; `InvalidInput` for a path of the other flavor.
pub fn read_dir(path: &PathText) -> io::Result<Vec<PathText>> {
    run(path, |p| {
        let mut out = Vec::new();
        for entry in std::fs::read_dir(p)? {
            out.push(PathText::from_os(&entry?.path()));
        }
        out.sort_by_key(PathText::portable);
        Ok(out)
    })
}

#[must_use]
pub fn exists(path: &PathText) -> bool {
    path.os().is_ok_and(|p| p.exists())
}

#[must_use]
pub fn is_file(path: &PathText) -> bool {
    path.os().is_ok_and(|p| p.is_file())
}

#[must_use]
pub fn is_dir(path: &PathText) -> bool {
    path.os().is_ok_and(|p| p.is_dir())
}

/// The resolved path, in the plain spelling every other path in the process uses (never
/// Windows's verbatim `\\?\D:\…`) — `None` when it does not exist.
#[must_use]
pub fn canonical(path: &PathText) -> Option<PathText> {
    let os = path.os().ok()?;
    std::fs::canonicalize(os)
        .ok()
        .map(|abs| PathText::from_os(&abs))
}

/// Do two spellings name the same file on disk?  `false` when either does not exist.
#[must_use]
pub fn same_file(a: &PathText, b: &PathText) -> bool {
    match (canonical(a), canonical(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// Does `path` on disk live inside `dir` on disk?  `false` when either does not exist.
#[must_use]
pub fn is_under_canonical(path: &PathText, dir: &PathText) -> bool {
    match (canonical(path), canonical(dir)) {
        (Some(p), Some(d)) => p.starts_with(&d),
        _ => false,
    }
}

/// A path a loft PROGRAM handed the runtime, as the host spelling to open — or the refusal
/// to log (`@FR-Path-Refuse`).  [`PathText::program`] is the rule; this renders it.  An empty
/// path is passed through: there is no name to judge, and it names nothing.
///
/// # Errors
/// The refused name and why.
pub fn program_path(raw: &str) -> Result<String, String> {
    if raw.is_empty() {
        return Ok(String::new());
    }
    PathText::program(raw).map(|p| p.native())
}

/// `@FR-Path-Refuse` — the one log line for a refused path, once per path per process: one
/// program operation resolves its path more than once (`file(p).write(..)` does twice), and a
/// refusal said three times reads as three problems.
pub fn log_refusal_once(raw: &str, why: &str) {
    static SAID: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
        std::sync::Mutex::new(None);
    let first = SAID.lock().map_or(true, |mut said| {
        said.get_or_insert_with(Default::default)
            .insert(raw.to_string())
    });
    if first {
        crate::loft_eprintln!("loft: the path `{raw}` is refused: {why}");
    }
}

/// Open a resolved path for reading; a refused one (`None`) opens with [`path_refused`].
///
/// # Errors
/// The OS's error, or the refusal.
pub fn open_resolved(path: Option<&str>) -> std::io::Result<std::fs::File> {
    path.ok_or_else(path_refused).and_then(std::fs::File::open)
}

/// `@FR-Path-Refuse` — the error a refused path opens with, so an open site's existing error
/// branch answers for it; the refusal itself was logged by `Stores::resolve_path`.
#[must_use]
pub fn path_refused() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidInput,
        "the path is refused (formal/paths.md)",
    )
}

/// `@FR-Path-Case` — a name means exactly its spelling.  `Err` when a name on `full`'s way
/// matches an entry of its directory only when case is ignored: on a case-folding file
/// system (Windows, macOS) the OS would answer for that other entry, and on Linux the two
/// would be two files where Windows holds one.
///
/// Cost: on Linux one `stat` when the path exists, one directory read when it does not; on
/// a case-folding system one `canonicalize` (which answers the on-disk spelling).
///
/// # Errors
/// The clash, naming both spellings.
pub fn case_clash(full: &str) -> Result<(), String> {
    let want = PathText::host(full);
    // The longest prefix that exists, and the first name below it that does not.
    let mut existing = want.clone();
    let mut missing: Option<String> = None;
    while !exists(&existing) {
        match existing.parent() {
            Some(up) => {
                missing = existing.file_name().map(str::to_string);
                existing = up;
            }
            None => return Ok(()),
        }
    }
    let folding = cfg!(any(windows, target_os = "macos"));
    if folding && let Some(on_disk) = canonical(&existing) {
        // Compare the trailing names: a pair equal without case but not with it is a clash.
        for (asked, real) in existing
            .parts()
            .iter()
            .rev()
            .zip(on_disk.parts().iter().rev())
        {
            if asked != real && asked.to_lowercase() == real.to_lowercase() {
                return Err(format!("`{asked}` is spelled `{real}` on disk"));
            }
        }
    }
    if let Some(name) = missing
        && let Ok(entries) = read_dir(&existing)
    {
        let lower = name.to_lowercase();
        if let Some(other) = entries
            .iter()
            .filter_map(PathText::file_name)
            .find(|e| *e != name && e.to_lowercase() == lower)
        {
            return Err(format!("`{name}` would be a second spelling of `{other}`"));
        }
    }
    Ok(())
}

/// `@FR-Path-Sep` — a host path in the form loft GIVES a program: `/` only and no trailing
/// separator (`C:/Users/x/Temp`, `/tmp`), so a program joining `"{dir}/{name}"` builds one
/// spelling on every platform.  Empty stays empty (no directory to give).
#[must_use]
pub fn given(path: &str) -> String {
    if path.is_empty() {
        String::new()
    } else {
        PathText::host(path).portable()
    }
}

/// Is a source `file` in the shipped standard library — any directory named `default`
/// on its way?  See [`path::is_stdlib_source`].
#[must_use]
pub fn is_stdlib_source(file: &str) -> bool {
    path::is_stdlib_source(file, Flavor::HOST)
}

/// Is `file` `dir` or inside it, by components (`pkg` does not claim `pkg2/x.loft`).
#[must_use]
pub fn is_under(file: &str, dir: &str) -> bool {
    PathText::host(file).starts_with(&PathText::host(dir))
}

// The name operations the compiler asks of a host path, shaped like `std::path`'s so a call
// site keeps its types, and answered by `PathText` under the host's flavor — so each answer
// is the one the Windows rules give when the compiler runs there, and those rules are unit
// tests on every host.  Paths here are host paths the OS or the user gave; a loft PROGRAM's
// paths go through `PathText::program` instead.

/// The last name (`std`: `Path::file_name`).
#[must_use]
pub fn file_name(path: &Path) -> Option<String> {
    PathText::from_os(path).file_name().map(str::to_string)
}

/// The last name without its extension (`std`: `Path::file_stem`).
#[must_use]
pub fn file_stem(path: &Path) -> Option<String> {
    PathText::from_os(path).file_stem().map(str::to_string)
}

/// The last name's extension, without its dot (`std`: `Path::extension`).
#[must_use]
pub fn extension(path: &Path) -> Option<String> {
    PathText::from_os(path).extension().map(str::to_string)
}

/// Does the last name carry extension `ext` — compared under the flavor's case rule, so
/// `x.DLL` is a `dll` on Windows?
#[must_use]
pub fn has_extension(path: &Path, ext: &str) -> bool {
    let p = PathText::from_os(path);
    p.extension().is_some_and(|e| match p.flavor() {
        Flavor::Unix => e == ext,
        Flavor::Windows => e.eq_ignore_ascii_case(ext),
    })
}

/// The path without its last name (`std`: `Path::parent`), `None` at a root.
#[must_use]
pub fn parent(path: &Path) -> Option<PathBuf> {
    PathText::from_os(path)
        .parent()
        .map(|p| PathBuf::from(p.native_or_empty()))
}

/// `path` below `base` as a relative path (`std`: `Path::strip_prefix`), `None` when it
/// is not inside — by component, under the flavor's case rule.
#[must_use]
pub fn relative(path: &Path, base: &Path) -> Option<PathBuf> {
    PathText::from_os(path)
        .relative_to(&PathText::from_os(base))
        .map(|p| PathBuf::from(p.native_or_empty()))
}

/// The path with its extension replaced (`std`: `Path::with_extension`).
#[must_use]
pub fn with_extension(path: &Path, ext: &str) -> PathBuf {
    PathBuf::from(PathText::from_os(path).with_extension(ext).native())
}

/// Does the path start at a root (`std`: `Path::is_absolute`)?
#[must_use]
pub fn is_absolute(path: &Path) -> bool {
    PathText::from_os(path).is_absolute()
}

/// Do two spellings of a host path name the same place — by components, a trailing
/// separator meaning nothing and, on Windows, `/` and `\` and case agreeing?  Text only: the
/// file system is not asked (see [`same_file`] for that).
#[must_use]
pub fn same_path(a: &str, b: &str) -> bool {
    PathText::host(a) == PathText::host(b)
}

/// The last name of a host path — `server` for `C:\libs\server` and for `/libs/server/` —
/// or the text itself when it has none.
#[must_use]
pub fn name_of(path: &str) -> String {
    PathText::host(path)
        .file_name()
        .map_or_else(|| path.to_string(), str::to_string)
}

/// Render a host path with `/` separators, and otherwise exactly as given — a display
/// spelling, not a parse: `./x` stays `./x`.  On Unix a `\` is a filename character and
/// is left alone; only the host's own separator is replaced.
#[must_use]
pub fn portable(path: &Path) -> String {
    portable_str(&path.to_string_lossy())
}

/// [`portable`] for a path already held as text.
#[must_use]
pub fn portable_str(path: &str) -> String {
    if std::path::MAIN_SEPARATOR == '/' {
        path.to_string()
    } else {
        path.replace(std::path::MAIN_SEPARATOR, "/")
    }
}

/// Render `path` for a format in which a backslash is ILLEGAL — a `file://` URI, where
/// `\U` is an invalid JSON escape that corrupts the LSP message carrying it (#639).  The
/// input may come from an editor on another platform, so every backslash goes wherever
/// this runs: the format wins over an exotic Unix file name.  Text otherwise untouched —
/// an editor matches the URI it sent.
#[must_use]
pub fn for_uri(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Shed a Windows verbatim prefix from text (`\\?\D:\…` → `D:\…`,
/// `\\?\UNC\srv\share\…` → `\\srv\share\…`); anything else is answered unchanged.
#[must_use]
pub fn strip_verbatim(text: &str) -> String {
    if let Some(unc) = text.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{unc}")
    } else if let Some(rest) = text.strip_prefix(r"\\?\")
        && rest.as_bytes().get(1) == Some(&b':')
    {
        rest.to_string()
    } else {
        text.to_string()
    }
}

/// [`canonical`] for a host `Path`, answered as given when it does not resolve.
#[must_use]
pub fn plain_canonical(path: &Path) -> PathBuf {
    try_plain_canonical(path).unwrap_or_else(|| path.to_path_buf())
}

/// [`canonical`] for a host `Path`; `None` when it does not exist.
#[must_use]
pub fn try_plain_canonical(path: &Path) -> Option<PathBuf> {
    canonical(&PathText::from_os(path)).map(|p| PathBuf::from(p.native()))
}

/// [`plain_canonical`] for a path held as text.
#[must_use]
pub fn plain_canonical_str(path: &str) -> String {
    plain_canonical(Path::new(path))
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod guard;

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathText {
        let dir = PathText::from_os(&std::env::temp_dir())
            .join(&format!("loft_file_access_{tag}_{}", std::process::id()));
        let _ = remove_dir_all(&dir);
        create_dir_all(&dir).expect("scratch dir");
        dir
    }

    #[test]
    fn operations_round_trip_on_the_host() {
        let dir = scratch("ops");
        let f = dir.join("a.txt");
        write(&f, "x").unwrap();
        assert_eq!(read_to_string(&f).unwrap(), "x");
        assert!(exists(&f) && is_file(&f) && !is_dir(&f) && is_dir(&dir));
        let g = dir.join("b.txt");
        copy(&f, &g).unwrap();
        rename(&g, &dir.join("c.txt")).unwrap();
        let names: Vec<String> = read_dir(&dir)
            .unwrap()
            .iter()
            .filter_map(|p| p.file_name().map(str::to_string))
            .collect();
        assert_eq!(names, ["a.txt", "c.txt"], "a listing is sorted");
        remove_dir_all(&dir).unwrap();
        assert!(!exists(&dir));
    }

    #[test]
    fn an_error_names_its_path() {
        let missing = scratch("err").join("no-such-file.loft");
        let e = read_to_string(&missing).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
        assert!(e.to_string().contains("no-such-file.loft"), "{e}");
    }

    #[test]
    fn a_foreign_flavor_never_reaches_the_os() {
        let other = if Flavor::HOST == Flavor::Unix {
            Flavor::Windows
        } else {
            Flavor::Unix
        };
        let p = PathText::parse("C:/x/y.loft", other);
        assert_eq!(read(&p).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert!(!exists(&p));
    }

    #[test]
    fn canonical_is_plain_and_answers_none_for_what_does_not_exist() {
        let dir = scratch("canon");
        let c = canonical(&dir).expect("exists");
        assert!(c.is_absolute());
        assert!(!c.native().starts_with(r"\\?\"), "{}", c.native());
        assert!(same_file(&dir, &c));
        assert_eq!(canonical(&dir.join("missing")), None);
        assert!(is_under_canonical(&dir, &dir));
    }

    #[test]
    fn a_program_path_reads_both_separators_and_refuses_what_no_platform_takes() {
        let host = |t: &str| PathText::host(t).native();
        assert_eq!(program_path(r"data\x.txt").unwrap(), host("data/x.txt"));
        assert_eq!(program_path("/tmp/a/../b.txt").unwrap(), host("/tmp/b.txt"));
        assert!(program_path("./x").is_ok());
        // An absolute path is the host's: a drive is a Windows host's, and `:` refuses it
        // as a name on Unix.
        assert_eq!(
            program_path(r"C:\data\x.txt").is_ok(),
            Flavor::HOST == Flavor::Windows
        );
        for bad in [
            "a:b.txt",
            "q?.txt",
            "x*",
            "a<b",
            "p|q",
            "say\"hi\"",
            "trail.",
            "space ",
            "aux.txt",
            "CON",
            "nul.tar.gz",
            "com1",
            "LPT9.log",
            "d/C:x",
            "tab\there",
            "x/C:/y",
        ] {
            assert!(program_path(bad).is_err(), "{bad} must be refused");
        }
        for good in [
            "com0",
            "console.txt",
            "auxiliary",
            "a.b.c",
            ".hidden",
            "..",
            "a/./b",
            "",
        ] {
            assert!(program_path(good).is_ok(), "{good} must be accepted");
        }
    }

    #[test]
    fn a_name_that_differs_only_in_case_clashes() {
        let dir = scratch("case");
        write(&dir.join("b.txt"), "b").unwrap();
        create_dir_all(&dir.join("sub")).unwrap();
        let at = |rel: &str| dir.join(rel).native();
        assert!(case_clash(&at("b.txt")).is_ok(), "the exact spelling");
        assert!(case_clash(&at("c.txt")).is_ok(), "a new name");
        assert!(
            case_clash(&at("B.txt")).is_err(),
            "a second spelling of b.txt"
        );
        assert!(
            case_clash(&at("SUB/x.txt")).is_err(),
            "a directory's second spelling"
        );
        assert!(
            case_clash(&at("sub/new/deeper.txt")).is_ok(),
            "missing below an exact name"
        );
        remove_dir_all(&dir).unwrap();
    }

    /// The answers the compiler's package and library sites now take from here instead of
    /// text `starts_with` / `==` / `rsplit('/')`.
    #[test]
    fn package_questions_are_answered_by_component() {
        assert!(is_under("/libs/server/src/a.loft", "/libs/server"));
        assert!(is_under("/libs/server/src/a.loft", "/libs/server/"));
        assert!(
            !is_under("/libs/server2/src/a.loft", "/libs/server"),
            "pkg does not claim pkg2"
        );
        assert!(same_path("/libs/server/", "/libs/server"));
        assert!(same_path("/libs/./server", "/libs/server"));
        assert!(!same_path("/libs/server", "/libs/server2"));
        assert_eq!(name_of("/libs/server/"), "server");
        assert_eq!(name_of("server"), "server");
        assert_eq!(name_of(""), "");
        // The Windows spellings, through the same component model.
        let w = |t: &str| PathText::parse(t, Flavor::Windows);
        assert!(w(r"C:\libs\server\src\a.loft").starts_with(&w("c:/LIBS/server")));
        assert_eq!(w(r"C:\libs\server\").file_name(), Some("server"));
    }

    #[test]
    fn uri_rendering_is_not_platform_rendering() {
        assert_eq!(for_uri(Path::new(r"C:\a\b.loft")), "C:/a/b.loft");
    }

    #[test]
    fn a_verbatim_prefix_is_shed_and_nothing_else_is_touched() {
        assert_eq!(strip_verbatim(r"\\?\C:\work\a.loft"), r"C:\work\a.loft");
        assert_eq!(
            strip_verbatim(r"\\?\UNC\srv\share\a.loft"),
            r"\\srv\share\a.loft"
        );
        assert_eq!(strip_verbatim("/home/u/a.loft"), "/home/u/a.loft");
    }
}
