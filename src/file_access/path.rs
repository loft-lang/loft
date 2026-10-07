// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I90 — Shared utilities & data structures

//! Path TEXT, parsed under an explicit [`Flavor`].
//!
//! The compiler handles paths as text all the time — a source position's `file`, a
//! `--path` argument, a cache key, a report line — and every Windows defect it has had
//! came from deciding something on that text directly: `contains("/default/")` missing
//! `D:\a\loft/default\x.loft`, a verbatim `\\?\D:\…` never equal to its plain twin, a
//! `dir/` that was not `dir`, `pkg` claiming `pkg2/x`.  Here a path is parsed ONCE into
//! components, and comparisons, prefixes and renderings work on those.
//!
//! The flavor is a parameter rather than the host's `std::path` so that the Windows
//! rules are tested on every host: `std::path` on Linux treats `\` as a filename
//! character, so a test of Windows behaviour written against it proves nothing there.
//! [`Flavor::HOST`] is what production code passes.
//!
//! Rules, per flavor:
//! - **Unix**: only `/` separates; a `\` is an ordinary filename character.
//! - **Windows**: `/` and `\` both separate; a drive (`C:`) or UNC share (`\\srv\share`)
//!   is the prefix; the verbatim forms `canonicalize` answers (`\\?\C:\…`,
//!   `\\?\UNC\srv\share\…`) parse as their plain twins; names compare ignoring ASCII case.
//! - **Both**: empty and `.` components are dropped, a trailing separator means nothing,
//!   and `..` folds lexically into the component before it (not above a root).

/// Which platform's rules a path text is read under.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flavor {
    Unix,
    Windows,
}

thread_local! {
    /// A test's [`with_program_host`]; `None` defers to the switch.
    static PROGRAM_HOST: std::cell::Cell<Option<Flavor>> = const { std::cell::Cell::new(None) };
}

/// Run `body` with this thread's programs on `flavor`, as `LOFT_POISON_HOST` would put them —
/// how a test asks the emulated host on any host without a process of its own.
pub fn with_program_host<T>(flavor: Flavor, body: impl FnOnce() -> T) -> T {
    struct Restore(Option<Flavor>);
    impl Drop for Restore {
        fn drop(&mut self) {
            PROGRAM_HOST.with(|h| h.set(self.0));
        }
    }
    let _restore = Restore(PROGRAM_HOST.with(|h| h.replace(Some(flavor))));
    body()
}

/// The drive the emulated Windows host mounts the real file system on: `/a/b` is `L:\a\b`.
pub const EMULATED_DRIVE: &str = "L:";

impl Flavor {
    /// The flavor of the platform the compiler runs on.
    pub const HOST: Flavor = if cfg!(windows) {
        Flavor::Windows
    } else {
        Flavor::Unix
    };

    /// The platform a loft PROGRAM's paths are read under: [`Flavor::HOST`], or Windows under
    /// `LOFT_POISON_HOST=windows` (@PLN184 Track W).  The compiler's own paths stay on
    /// [`Flavor::HOST`]: they still reach the disk through `std` at sites that do not go
    /// through `file_access`, and a Windows spelling there names nothing on Linux.
    #[must_use]
    pub fn program_host() -> Flavor {
        PROGRAM_HOST
            .with(std::cell::Cell::get)
            .or_else(crate::keys::poison_host)
            .unwrap_or(Flavor::HOST)
    }

    /// Does a program run on an emulated platform — Windows' rules over a Unix file system?
    #[must_use]
    pub fn emulating() -> bool {
        Flavor::program_host() != Flavor::HOST
    }

    #[must_use]
    pub fn is_separator(self, c: char) -> bool {
        c == '/' || (self == Flavor::Windows && c == '\\')
    }

    /// The separator a native rendering uses.
    #[must_use]
    pub fn separator(self) -> char {
        match self {
            Flavor::Unix => '/',
            Flavor::Windows => '\\',
        }
    }

    fn name_eq(self, a: &str, b: &str) -> bool {
        match self {
            Flavor::Unix => a == b,
            Flavor::Windows => a.eq_ignore_ascii_case(b),
        }
    }
}

/// A path text parsed into components.  Build it with [`PathText::parse`] (or
/// [`PathText::host`]); compare it with `==`, [`PathText::starts_with`] and
/// [`PathText::has_component`]; render it with [`PathText::portable`] or
/// [`PathText::native`].
#[derive(Clone, Debug)]
pub struct PathText {
    flavor: Flavor,
    /// `C:` (drive letter upper-cased) or `//srv/share`; empty for none.
    prefix: String,
    /// A separator follows the prefix: the path starts at a root.
    rooted: bool,
    parts: Vec<String>,
    /// `@FR-Path-Utf8` — the OS's own spelling of a name its text cannot hold (bytes that
    /// are not UTF-8, an unpaired UTF-16 half), beside that name's lossy text in `parts`.
    /// Empty when every name is text, which is nearly always; otherwise one entry per part.
    raw: Vec<Option<std::ffi::OsString>>,
    /// The path names nothing: the empty text, or what `std` answers for the parent of a
    /// lone relative name.  It renders as `""`, and no operation reaches the file system with
    /// it — where `.` (also no parts) is the current directory.
    empty: bool,
}

impl PathText {
    /// Parse `text` under `flavor`'s rules (see the module docs).
    #[must_use]
    pub fn parse(text: &str, flavor: Flavor) -> PathText {
        PathText::parse_with(text, flavor, false)
    }

    /// A path a loft PROGRAM wrote, judged by the portable contract of `formal/paths.md` and
    /// parsed by the same code as every other path: `\` separates on every host
    /// (`@FR-Path-Sep`), and each name must be one every platform can hold
    /// (`@FR-Path-Name`).  The answer is a path on the program's host
    /// ([`Flavor::program_host`]), so it reaches the OS through `file_access`.  A drive prefix is
    /// a Windows host's; on Unix `C:` is a name, and `:` refuses it — an absolute path names
    /// a place on this host and was never portable.
    ///
    /// # Errors
    /// The refused name and why.
    pub fn program(raw: &str) -> Result<PathText, String> {
        PathText::program_in(raw, Flavor::program_host())
    }

    /// [`PathText::program`] under `flavor`'s rules — the form the tests ask on every host.
    ///
    /// # Errors
    /// The refused name and why.
    pub fn program_in(raw: &str, flavor: Flavor) -> Result<PathText, String> {
        let p = PathText::parse_with(raw, flavor, true);
        // A drive without a root (`a:b.txt`, `C:x`) is Windows's drive-RELATIVE form: it
        // names a place by that drive's hidden current directory.  `:` is allowed only for a
        // drive leading an absolute path.
        if !p.prefix.is_empty() && !p.prefix.starts_with("//") && !p.rooted {
            return Err(format!(
                "`{raw}` is relative to drive {}'s current directory, which no other platform has",
                p.prefix
            ));
        }
        for name in &p.parts {
            if let Some(why) = name_refusal(name) {
                return Err(why);
            }
        }
        Ok(p)
    }

    fn parse_with(text: &str, flavor: Flavor, backslash: bool) -> PathText {
        if text.is_empty() {
            return PathText {
                flavor,
                prefix: String::new(),
                rooted: false,
                parts: Vec::new(),
                raw: Vec::new(),
                empty: true,
            };
        }
        let is_sep = |c: char| flavor.is_separator(c) || (backslash && c == '\\');
        let mut rest = text;
        let mut prefix = String::new();
        if flavor == Flavor::Windows {
            // The verbatim forms `canonicalize` answers are their plain twins.
            let unc_verbatim = rest
                .strip_prefix(r"\\?\UNC\")
                .or_else(|| rest.strip_prefix("//?/UNC/"));
            let verbatim = rest
                .strip_prefix(r"\\?\")
                .or_else(|| rest.strip_prefix("//?/"));
            let unc_body = if let Some(body) = unc_verbatim {
                Some(body)
            } else if let Some(body) = verbatim {
                rest = body;
                None
            } else {
                let mut cs = rest.chars();
                match (cs.next(), cs.next()) {
                    (Some(a), Some(b)) if is_sep(a) && is_sep(b) => Some(&rest[2..]),
                    _ => None,
                }
            };
            if let Some(body) = unc_body {
                // `\\srv\share\…`: the server and share are the prefix, and the path is
                // rooted under them.
                let mut it = body.splitn(3, is_sep);
                let server = it.next().unwrap_or("");
                let share = it.next().unwrap_or("");
                prefix = format!("//{server}/{share}");
                let tail = it.next().unwrap_or("");
                return PathText::from_parts(flavor, prefix, true, tail, &is_sep);
            }
            let b = rest.as_bytes();
            if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
                prefix = format!("{}:", char::from(b[0]).to_ascii_uppercase());
                rest = &rest[2..];
            }
        }
        let rooted = rest.chars().next().is_some_and(is_sep);
        PathText::from_parts(flavor, prefix, rooted, rest, &is_sep)
    }

    /// [`PathText::parse`] under the host's rules.
    #[must_use]
    pub fn host(text: &str) -> PathText {
        PathText::parse(text, Flavor::HOST)
    }

    /// A host path as a PROGRAM sees it: itself, or under the emulated host the same place
    /// in Windows' spelling (`/a/b` is `L:\a\b`, a relative path keeps its names; a path
    /// already in that spelling gains the current drive when it has none).
    #[must_use]
    pub fn for_program(&self) -> PathText {
        if !Flavor::emulating() {
            return self.clone();
        }
        if self.flavor != Flavor::HOST {
            // Already the program's spelling; a root without a drive is the current drive's.
            let mut p = self.clone();
            if p.rooted && p.prefix.is_empty() {
                p.prefix = EMULATED_DRIVE.to_string();
            }
            return p;
        }
        PathText {
            flavor: Flavor::Windows,
            prefix: if self.rooted {
                EMULATED_DRIVE.to_string()
            } else {
                String::new()
            },
            rooted: self.rooted,
            parts: self.parts.clone(),
            raw: self.raw.clone(),
            empty: self.empty,
        }
    }

    /// The host path an emulated-Windows path names — the inverse of
    /// [`PathText::for_program`].  A rooted path without a drive is on the current drive,
    /// which is [`EMULATED_DRIVE`].
    ///
    /// # Errors
    /// Another drive or a UNC share: the emulated host has neither.
    pub fn from_emulated(&self) -> Result<PathText, String> {
        if self.prefix.is_empty() || self.prefix == EMULATED_DRIVE {
            Ok(PathText {
                flavor: Flavor::HOST,
                prefix: String::new(),
                rooted: self.rooted,
                parts: self.parts.clone(),
                raw: self.raw.clone(),
                empty: self.empty,
            })
        } else {
            Err(format!(
                "{}: the emulated Windows host has only drive {EMULATED_DRIVE}",
                self.portable()
            ))
        }
    }

    fn from_parts(
        flavor: Flavor,
        prefix: String,
        rooted: bool,
        rest: &str,
        is_sep: &dyn Fn(char) -> bool,
    ) -> PathText {
        let mut parts: Vec<String> = Vec::new();
        for part in rest.split(is_sep) {
            match part {
                "" | "." => {}
                ".." => {
                    if parts.last().is_some_and(|p| p != "..") {
                        parts.pop();
                    } else if !rooted {
                        parts.push("..".to_string());
                    }
                }
                name => parts.push(name.to_string()),
            }
        }
        PathText {
            flavor,
            prefix,
            rooted,
            parts,
            raw: Vec::new(),
            empty: false,
        }
    }

    /// Does the path name nothing (see the `empty` field)?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.empty
    }

    #[must_use]
    pub fn flavor(&self) -> Flavor {
        self.flavor
    }

    /// The path names one place whatever the current directory is: `/a` on Unix, `C:\a` or
    /// `\\srv\share\a` on Windows.  A Windows `\a` is NOT absolute — it is the root of the
    /// CURRENT drive — as `std` also answers.
    #[must_use]
    pub fn is_absolute(&self) -> bool {
        match self.flavor {
            Flavor::Unix => self.rooted,
            Flavor::Windows => self.rooted && !self.prefix.is_empty(),
        }
    }

    /// The components after the prefix and root.
    #[must_use]
    pub fn parts(&self) -> &[String] {
        &self.parts
    }

    #[must_use]
    pub fn file_name(&self) -> Option<&str> {
        self.parts.last().map(String::as_str).filter(|p| *p != "..")
    }

    /// The extension of the last component, without its dot; `None` for `.hidden`.
    #[must_use]
    pub fn extension(&self) -> Option<&str> {
        let name = self.file_name()?;
        let dot = name.rfind('.')?;
        (dot > 0).then(|| &name[dot + 1..])
    }

    /// The path without its last component.
    #[must_use]
    pub fn parent(&self) -> Option<PathText> {
        if self.parts.is_empty() {
            return None;
        }
        let mut p = self.clone();
        p.parts.pop();
        p.raw.pop();
        p.empty = p.prefix.is_empty() && !p.rooted && p.parts.is_empty();
        Some(p)
    }

    /// The last name without its extension (`b` for `a/b.tar.gz` is `b.tar`, as `std` does).
    #[must_use]
    pub fn file_stem(&self) -> Option<&str> {
        let name = self.file_name()?;
        match name.rfind('.') {
            Some(dot) if dot > 0 => Some(&name[..dot]),
            _ => Some(name),
        }
    }

    /// The path with its last name's extension replaced by `ext` (removed when `ext` is
    /// empty); unchanged when there is no last name.
    #[must_use]
    pub fn with_extension(&self, ext: &str) -> PathText {
        let mut p = self.clone();
        if let Some(stem) = self.file_stem().map(str::to_string)
            && let Some(last) = p.parts.last_mut()
        {
            *last = if ext.is_empty() {
                stem
            } else {
                format!("{stem}.{ext}")
            };
            if let Some(Some(raw)) = p.raw.last_mut() {
                *raw = std::path::Path::new(raw)
                    .with_extension(ext)
                    .into_os_string();
            }
        }
        p
    }

    /// `self` below `base`, as a relative path — `None` when `self` is not inside `base`
    /// (by component, under the flavor's case rule).
    #[must_use]
    pub fn relative_to(&self, base: &PathText) -> Option<PathText> {
        if !self.starts_with(base) {
            return None;
        }
        Some(PathText {
            flavor: self.flavor,
            prefix: String::new(),
            rooted: false,
            parts: self.parts[base.parts.len()..].to_vec(),
            raw: self.raw.get(base.parts.len()..).unwrap_or(&[]).to_vec(),
            empty: self.parts.len() == base.parts.len(),
        })
    }

    /// The native spelling as `std` would hand it over: an empty relative path is `""`
    /// (where [`PathText::native`] renders `.`), so `parent("a")` stays `""`.
    #[must_use]
    pub fn native_or_empty(&self) -> String {
        if self.prefix.is_empty() && !self.rooted && self.parts.is_empty() {
            String::new()
        } else {
            self.native()
        }
    }

    /// `self` followed by `rel`; an absolute `rel` replaces `self`, as joining does on
    /// every platform.
    #[must_use]
    pub fn join(&self, rel: &str) -> PathText {
        let other = PathText::parse(rel, self.flavor);
        if other.rooted || !other.prefix.is_empty() {
            return other;
        }
        // Appended part by part, as `from_parts` folds them, so a name kept in the OS's own
        // spelling (`raw`) survives the join.
        let mut p = self.clone();
        p.empty = p.empty && other.empty;
        for part in other.parts {
            if part == ".." && p.parts.last().is_some_and(|l| l != "..") {
                p.parts.pop();
                p.raw.pop();
            } else if part != ".." || !p.rooted {
                p.parts.push(part);
                if !p.raw.is_empty() {
                    p.raw.push(None);
                }
            }
        }
        p
    }

    /// Is `self` `dir` or inside it — by components, so `pkg` does not claim `pkg2/x`, a
    /// trailing separator means nothing, and on Windows `/` and `\` agree.
    #[must_use]
    pub fn starts_with(&self, dir: &PathText) -> bool {
        self.flavor == dir.flavor
            && self.rooted == dir.rooted
            && self.flavor.name_eq(&self.prefix, &dir.prefix)
            && dir.parts.len() <= self.parts.len()
            && (0..dir.parts.len()).all(|i| self.same_name(i, dir, i))
    }

    /// Is part `i` the same name as `other`'s part `j`?  A name kept in the OS's own spelling
    /// equals only that spelling: two names that are not UTF-8 can share one lossy text.
    fn same_name(&self, i: usize, other: &PathText, j: usize) -> bool {
        match (self.raw_at(i), other.raw_at(j)) {
            (None, None) => self.flavor.name_eq(&self.parts[i], &other.parts[j]),
            (a, b) => a == b,
        }
    }

    pub(crate) fn raw_at(&self, i: usize) -> Option<&std::ffi::OsString> {
        self.raw.get(i).and_then(Option::as_ref)
    }

    /// The last name as the OS spells it — its own bytes for a name that is not text — to
    /// join under another directory without losing it.
    #[must_use]
    pub fn os_name(&self) -> Option<std::ffi::OsString> {
        match self.raw.last() {
            Some(Some(raw)) => Some(raw.clone()),
            _ => self.file_name().map(std::ffi::OsString::from),
        }
    }

    /// `@FR-Path-Utf8` — is the last name one loft text cannot spell (shown with U+FFFD)?
    #[must_use]
    pub fn last_is_unspellable(&self) -> bool {
        self.raw.last().is_some_and(Option::is_some)
    }

    /// The spelling handed to the OS: [`PathText::native`], except that a name kept in the
    /// OS's own spelling is handed over as it was received.
    #[must_use]
    pub fn os_spelling(&self) -> std::path::PathBuf {
        if self.raw.iter().all(Option::is_none) {
            return std::path::PathBuf::from(self.native());
        }
        let head = PathText {
            parts: Vec::new(),
            raw: Vec::new(),
            ..self.clone()
        };
        let mut out = if head.prefix.is_empty() && !head.rooted {
            std::path::PathBuf::new()
        } else {
            std::path::PathBuf::from(head.native())
        };
        for (i, part) in self.parts.iter().enumerate() {
            match self.raw_at(i) {
                Some(raw) => out.push(raw),
                None => out.push(part),
            }
        }
        out
    }

    /// A path the OS handed over, keeping each name its text cannot hold in the OS's own
    /// spelling.  `lossy` is its text parsed under `flavor`; `os` is the same path.
    pub(crate) fn keeping_os_names(mut lossy: PathText, os: &std::path::Path) -> PathText {
        if os.to_str().is_some() {
            return lossy;
        }
        let mut parts = Vec::new();
        let mut raw = Vec::new();
        for c in os.components() {
            match c {
                std::path::Component::Normal(n) => {
                    parts.push(n.to_string_lossy().into_owned());
                    raw.push(n.to_str().is_none().then(|| n.to_os_string()));
                }
                std::path::Component::ParentDir => {
                    if parts.last().is_some_and(|l: &String| l != "..") {
                        parts.pop();
                        raw.pop();
                    } else if !lossy.rooted {
                        parts.push("..".to_string());
                        raw.push(None);
                    }
                }
                _ => {}
            }
        }
        lossy.parts = parts;
        lossy.raw = raw;
        lossy
    }

    /// Does the path END with these components (`["database", "mod.rs"]`)?  The way to
    /// ask what a displayed path names without spelling a separator.
    #[must_use]
    pub fn ends_with(&self, tail: &[&str]) -> bool {
        tail.len() <= self.parts.len()
            && tail
                .iter()
                .rev()
                .zip(self.parts.iter().rev())
                .all(|(a, b)| self.flavor.name_eq(a, b))
    }

    /// Is one of the DIRECTORY components `name` (the last component, the file, is not
    /// asked)?
    #[must_use]
    pub fn has_component(&self, name: &str) -> bool {
        let dirs = self.parts.len().saturating_sub(1);
        self.parts[..dirs]
            .iter()
            .any(|p| self.flavor.name_eq(p, name))
    }

    /// The path with `/` separators — for text a reader copies, a key, a comparison
    /// written in a test.  `C:/a/b`, `//srv/share/a`, `/a/b`, `a/b`; `.` for nothing.
    #[must_use]
    pub fn portable(&self) -> String {
        self.render('/')
    }

    /// The path with the flavor's own separator — the spelling to hand the OS.
    #[must_use]
    pub fn native(&self) -> String {
        self.render(self.flavor.separator())
    }

    fn render(&self, sep: char) -> String {
        let mut out = if sep == '/' {
            self.prefix.clone()
        } else {
            self.prefix.replace('/', &sep.to_string())
        };
        // A UNC prefix carries its own root; every other rooted path starts with one.
        if self.rooted && !self.prefix.starts_with("//") {
            out.push(sep);
        }
        for (i, part) in self.parts.iter().enumerate() {
            if i > 0 || (self.prefix.starts_with("//") && self.rooted) {
                out.push(sep);
            }
            out.push_str(part);
        }
        if out.is_empty() && !self.empty {
            out.push('.');
        }
        out
    }
}

impl PartialEq for PathText {
    fn eq(&self, other: &PathText) -> bool {
        self.parts.len() == other.parts.len() && self.starts_with(other)
    }
}

/// `@FR-Path-Name` — why `name` is a name some platform cannot hold, or `None`.  The union of
/// what Windows refuses: a control character or one of `< > : " \\ | ? *`, a device name
/// (`CON PRN AUX NUL COM1–9 LPT1–9`, any case, with or without an extension), a name ending
/// in `.` or a space.  `.` and `..` are steps of the walk, not names.
#[must_use]
pub fn name_refusal(name: &str) -> Option<String> {
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    if let Some(c) = name
        .chars()
        .find(|c| c.is_control() || "<>:\"\\|?*".contains(*c))
    {
        return Some(format!(
            "the name `{name}` holds {c:?}, which a file name cannot hold on every platform"
        ));
    }
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let numbered = stem.len() == 4
        && (stem.starts_with("COM") || stem.starts_with("LPT"))
        && matches!(stem.as_bytes()[3], b'1'..=b'9');
    if matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || numbered {
        return Some(format!(
            "the name `{name}` is a device name on Windows, so no platform takes it"
        ));
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return Some(format!(
            "the name `{name}` ends in a dot or a space, which Windows drops"
        ));
    }
    None
}

/// Is `file` inside a directory named `default` — the shipped standard library?
///
/// A stdlib position's `file` is whatever path the loader used: `default/01_code.loft` in
/// a test, `<install>/share/loft/default/…` from an installed binary, and on Windows any
/// mix of separators (`D:\a\loft/default\01_code.loft` when a `/` joined a Windows
/// directory, which a text pattern missed and the stdlib's own operators were then judged
/// a user's — loft#1860).  A user directory literally named `default` is misread as the
/// stdlib; that is the price of a path-shaped answer.
#[must_use]
pub fn is_stdlib_source(file: &str, flavor: Flavor) -> bool {
    PathText::parse(file, flavor).has_component("default")
}

#[cfg(test)]
mod tests {
    use super::Flavor::{Unix, Windows};
    use super::*;

    fn w(t: &str) -> PathText {
        PathText::parse(t, Windows)
    }
    fn u(t: &str) -> PathText {
        PathText::parse(t, Unix)
    }

    /// F1 — a Windows directory joined with `/`: every mix of separators is one path.
    #[test]
    fn windows_separators_mix_freely() {
        let a = w(r"D:\a\loft\loft/default\01_code.loft");
        assert_eq!(a.portable(), "D:/a/loft/loft/default/01_code.loft");
        assert_eq!(a.native(), r"D:\a\loft\loft\default\01_code.loft");
        assert_eq!(a, w("D:/a/loft/loft/default/01_code.loft"));
        assert_eq!(
            a,
            w(r"d:\A\LOFT\loft\Default\01_CODE.loft"),
            "case and drive case"
        );
        assert!(a.is_absolute());
    }

    /// The correction `portable_path` carried: on Unix a backslash is a filename
    /// character, so it may never be read as a separator there.
    #[test]
    fn a_unix_backslash_is_part_of_the_name() {
        let p = u(r"dir/weird\name.loft");
        assert_eq!(p.parts(), ["dir", r"weird\name.loft"]);
        assert_eq!(p.portable(), r"dir/weird\name.loft");
        assert_ne!(p, u("dir/weird/name.loft"));
        assert_ne!(u("A/b"), u("a/b"), "Unix names are case-sensitive");
    }

    /// F3 — what `canonicalize` answers on Windows equals the plain spelling.
    #[test]
    fn verbatim_paths_are_their_plain_twins() {
        assert_eq!(w(r"\\?\C:\work\a.loft"), w(r"C:\work\a.loft"));
        assert_eq!(w(r"\\?\C:\work\a.loft").native(), r"C:\work\a.loft");
        assert_eq!(w(r"\\?\UNC\srv\share\a.loft"), w(r"\\srv\share\a.loft"));
        assert_eq!(
            w(r"\\?\UNC\srv\share\a.loft").native(),
            r"\\srv\share\a.loft"
        );
        assert_eq!(w(r"\\srv\share\a.loft").portable(), "//srv/share/a.loft");
    }

    /// F5, F6 — containment is by component, and a trailing separator means nothing.
    #[test]
    fn containment_is_by_component() {
        assert!(u("pkg/src/a.loft").starts_with(&u("pkg")));
        assert!(u("pkg/src/a.loft").starts_with(&u("pkg/src/")));
        assert!(!u("pkg2/src/a.loft").starts_with(&u("pkg")));
        assert!(!u("pkg").starts_with(&u("pkg/src")));
        assert!(w(r"D:\a\loft\default\x.loft").starts_with(&w("D:/a/loft/")));
        assert!(!w(r"D:\a\x.loft").starts_with(&w(r"C:\a")), "another drive");
        assert!(
            !u("/a/b").starts_with(&u("a")),
            "rooted and relative differ"
        );
        assert_eq!(u("dir/"), u("dir"));
        assert_eq!(w(r"D:\a\loft\loft/"), w(r"D:\a\loft\loft"));
    }

    #[test]
    fn dot_components_fold_lexically() {
        assert_eq!(u("a/./b/../c").portable(), "a/c");
        assert_eq!(u("../x").portable(), "../x");
        assert_eq!(u("/../x").portable(), "/x", "nothing is above the root");
        assert_eq!(u("a/..").portable(), ".");
        assert_eq!(w(r"C:\a\..\b").portable(), "C:/b");
    }

    #[test]
    fn stems_extensions_and_relative_paths() {
        assert_eq!(u("a/b.tar.gz").file_stem(), Some("b.tar"));
        assert_eq!(u("a/.hidden").file_stem(), Some(".hidden"));
        assert_eq!(u("a/b").file_stem(), Some("b"));
        assert_eq!(u("a/b.loft").with_extension("rs").portable(), "a/b.rs");
        assert_eq!(u("a/b.loft").with_extension("").portable(), "a/b");
        assert_eq!(
            w(r"C:\x\y.LOFT").with_extension("store").native(),
            r"C:\x\y.store"
        );
        let rel = w(r"C:\Pkg\src\a.loft").relative_to(&w("c:/pkg")).unwrap();
        assert_eq!(rel.portable(), "src/a.loft");
        assert!(rel.parts() == ["src", "a.loft"] && !rel.is_absolute());
        assert!(u("/x/pkg2/a").relative_to(&u("/x/pkg")).is_none());
        assert_eq!(u("a").parent().unwrap().native_or_empty(), "");
        assert!(w(r"C:\a").is_absolute() && w(r"\\srv\share\a").is_absolute());
        assert!(!w(r"\a").is_absolute(), "the current drive's root");
        assert!(u("/a").is_absolute() && !u("a").is_absolute());
        assert_eq!(u("/").parent().map(|p| p.portable()), None);
    }

    #[test]
    fn join_appends_or_replaces() {
        assert_eq!(
            w(r"D:\a\loft\loft/").join("default").native(),
            r"D:\a\loft\loft\default"
        );
        assert_eq!(u("/x").join("y/z.loft").portable(), "/x/y/z.loft");
        assert_eq!(u("/x").join("/abs").portable(), "/abs");
        assert_eq!(w(r"C:\x").join(r"D:\y").portable(), "D:/y");
    }

    #[test]
    fn names_and_parents() {
        let p = w(r"C:\a\b.loft");
        assert_eq!(p.file_name(), Some("b.loft"));
        assert_eq!(p.extension(), Some("loft"));
        assert_eq!(p.parent().map(|d| d.portable()), Some("C:/a".to_string()));
        assert_eq!(u(".hidden").extension(), None);
        assert!(p.ends_with(&["a", "b.loft"]));
        assert!(w(r"D:\a\loft\src\database\mod.rs").ends_with(&["database", "mod.rs"]));
        assert!(!u("src/database/mod.rs").ends_with(&["base", "mod.rs"]));
    }

    /// `@FR-Path-Name`'s one `:` — a drive leading an ABSOLUTE path — on both flavors.
    #[test]
    fn a_program_path_allows_a_drive_only_before_a_root() {
        let ok = |t: &str, f| PathText::program_in(t, f).is_ok();
        assert!(ok("C:/x", Windows) && ok(r"C:\x\y.loft", Windows) && !ok("C:", Windows));
        assert!(!ok("a:b.txt", Windows), "drive-relative");
        assert!(!ok("C:x", Windows), "drive-relative");
        assert!(
            !ok("x/C:/y", Windows),
            "a drive in the middle is a name with `:`"
        );
        assert!(
            !ok("a:b.txt", Unix) && !ok("C:/x", Unix),
            "on Unix `C:` is a name"
        );
        assert!(ok(r"data\x.txt", Unix) && ok("data/x.txt", Windows));
        assert_eq!(
            PathText::program_in(r"data\x.txt", Unix)
                .unwrap()
                .portable(),
            "data/x.txt"
        );
    }

    #[test]
    fn the_stdlib_is_recognised_however_it_was_loaded() {
        assert!(is_stdlib_source("default/01_code.loft", Unix));
        assert!(is_stdlib_source(
            "/usr/local/share/loft/default/01_code.loft",
            Unix
        ));
        assert!(is_stdlib_source(r"C:\loft\default\01_code.loft", Windows));
        assert!(is_stdlib_source(
            r"D:\a\loft\loft/default\01_code.loft",
            Windows
        ));
        assert!(is_stdlib_source(
            r"D:/a/loft/loft\default/01_code.loft",
            Windows
        ));
        assert!(is_stdlib_source(r"default\01_code.loft", Windows));
        assert!(
            !is_stdlib_source(r"default\01_code.loft", Unix),
            "one Unix file name"
        );
        assert!(!is_stdlib_source("src/main.loft", Unix));
        assert!(!is_stdlib_source("defaults/x.loft", Unix));
        assert!(
            !is_stdlib_source("src/default", Unix),
            "a FILE named default"
        );
        assert!(!is_stdlib_source("", Unix));
    }
}
