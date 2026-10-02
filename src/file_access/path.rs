// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

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

impl Flavor {
    /// The flavor of the platform the compiler runs on.
    pub const HOST: Flavor = if cfg!(windows) {
        Flavor::Windows
    } else {
        Flavor::Unix
    };

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
}

impl PathText {
    /// Parse `text` under `flavor`'s rules (see the module docs).
    #[must_use]
    pub fn parse(text: &str, flavor: Flavor) -> PathText {
        let mut rest = text;
        let mut prefix = String::new();
        if flavor == Flavor::Windows {
            // The verbatim forms `canonicalize` answers are their plain twins.
            let unc_verbatim = rest
                .strip_prefix(r"\\?\UNC\")
                .or_else(|| rest.strip_prefix("//?/UNC/"));
            let verbatim = rest.strip_prefix(r"\\?\").or_else(|| rest.strip_prefix("//?/"));
            let unc_body = if let Some(body) = unc_verbatim {
                Some(body)
            } else if let Some(body) = verbatim {
                rest = body;
                None
            } else {
                let mut cs = rest.chars();
                match (cs.next(), cs.next()) {
                    (Some(a), Some(b)) if flavor.is_separator(a) && flavor.is_separator(b) => {
                        Some(&rest[2..])
                    }
                    _ => None,
                }
            };
            if let Some(body) = unc_body {
                // `\\srv\share\…`: the server and share are the prefix, and the path is
                // rooted under them.
                let mut it = body.splitn(3, |c| flavor.is_separator(c));
                let server = it.next().unwrap_or("");
                let share = it.next().unwrap_or("");
                prefix = format!("//{server}/{share}");
                let tail = it.next().unwrap_or("");
                return PathText::from_parts(flavor, prefix, true, tail);
            }
            let b = rest.as_bytes();
            if b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':' {
                prefix = format!("{}:", char::from(b[0]).to_ascii_uppercase());
                rest = &rest[2..];
            }
        }
        let rooted = rest.chars().next().is_some_and(|c| flavor.is_separator(c));
        PathText::from_parts(flavor, prefix, rooted, rest)
    }

    /// [`PathText::parse`] under the host's rules.
    #[must_use]
    pub fn host(text: &str) -> PathText {
        PathText::parse(text, Flavor::HOST)
    }

    fn from_parts(flavor: Flavor, prefix: String, rooted: bool, rest: &str) -> PathText {
        let mut parts: Vec<String> = Vec::new();
        for part in rest.split(|c| flavor.is_separator(c)) {
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
        }
    }

    #[must_use]
    pub fn flavor(&self) -> Flavor {
        self.flavor
    }

    /// The path starts at a root (`/a`, `C:\a`, `\\srv\share\a`) rather than at a
    /// current directory.
    #[must_use]
    pub fn is_absolute(&self) -> bool {
        self.rooted
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
        Some(p)
    }

    /// `self` followed by `rel`; an absolute `rel` replaces `self`, as joining does on
    /// every platform.
    #[must_use]
    pub fn join(&self, rel: &str) -> PathText {
        let other = PathText::parse(rel, self.flavor);
        if other.rooted || !other.prefix.is_empty() {
            return other;
        }
        let mut text = self.native();
        text.push(self.flavor.separator());
        text.push_str(&other.native());
        PathText::parse(&text, self.flavor)
    }

    /// Is `self` `dir` or inside it — by components, so `pkg` does not claim `pkg2/x`, a
    /// trailing separator means nothing, and on Windows `/` and `\` agree.
    #[must_use]
    pub fn starts_with(&self, dir: &PathText) -> bool {
        self.flavor == dir.flavor
            && self.rooted == dir.rooted
            && self.flavor.name_eq(&self.prefix, &dir.prefix)
            && dir.parts.len() <= self.parts.len()
            && dir
                .parts
                .iter()
                .zip(&self.parts)
                .all(|(a, b)| self.flavor.name_eq(a, b))
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
        if out.is_empty() {
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

/// Is `file` inside a directory named `default` — the shipped standard library?
///
/// A stdlib position's `file` is whatever path the loader used: `default/01_code.loft` in
/// a test, `<install>/share/loft/default/…` from an installed binary, and on Windows any
/// mix of separators (`D:\a\loft/default\01_code.loft` when a `/` joined a Windows
/// directory, which a text pattern missed and the stdlib's own operators were then judged
/// a user's — loft#1859).  A user directory literally named `default` is misread as the
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
        assert_eq!(a, w(r"d:\A\LOFT\loft\Default\01_CODE.loft"), "case and drive case");
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
        assert_eq!(w(r"\\?\UNC\srv\share\a.loft").native(), r"\\srv\share\a.loft");
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
        assert!(!u("/a/b").starts_with(&u("a")), "rooted and relative differ");
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
    fn join_appends_or_replaces() {
        assert_eq!(w(r"D:\a\loft\loft/").join("default").native(), r"D:\a\loft\loft\default");
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

    #[test]
    fn the_stdlib_is_recognised_however_it_was_loaded() {
        assert!(is_stdlib_source("default/01_code.loft", Unix));
        assert!(is_stdlib_source("/usr/local/share/loft/default/01_code.loft", Unix));
        assert!(is_stdlib_source(r"C:\loft\default\01_code.loft", Windows));
        assert!(is_stdlib_source(r"D:\a\loft\loft/default\01_code.loft", Windows));
        assert!(is_stdlib_source(r"D:/a/loft/loft\default/01_code.loft", Windows));
        assert!(is_stdlib_source(r"default\01_code.loft", Windows));
        assert!(!is_stdlib_source(r"default\01_code.loft", Unix), "one Unix file name");
        assert!(!is_stdlib_source("src/main.loft", Unix));
        assert!(!is_stdlib_source("defaults/x.loft", Unix));
        assert!(!is_stdlib_source("src/default", Unix), "a FILE named default");
        assert!(!is_stdlib_source("", Unix));
    }
}
