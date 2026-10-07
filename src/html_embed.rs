// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F54 — Browser / WASM target (--html / --native-wasm)

//! @PLN146 F4 — turn `[[embed]]` declarations into the page's own filesystem, so a
//! `--html` page carries the pack it reads.
//!
//! The store loader already reads a page's tree (`store::image_bytes` falls back to
//! the `loft_host_fs_*` bridge that `doc/loft-fs.js` serves `globalThis.loftBaseFS`
//! through). What was missing is the other half: nothing PUT anything there, so a
//! page could only carry a pack if someone spliced the bytes in by hand.
//!
//! One invariant carries the module:
//!
//! > A file declared `[[embed]] path = "P"` is readable inside the page by exactly
//! > the call that reads it on the desktop — same function, same string `"P"`.
//!
//! Which is why [`validate`] is strict about `path` and relaxed about `source`.
//! `path` is *the program's* path: the page carries the file under `/` + `path`,
//! because that is what `loft-fs.js` `resolve()` makes of `"P"` under the default
//! cwd `/`. An absolute or `..`-bearing spelling is refused rather than carried —
//! carried faithfully, it would sit under a key only a program passing that same
//! build-box path could ask for. `source` is merely where the bytes are on the build
//! box, so it may be anything that exists.
//!
//! **Both are resolved against [`EmbedDecl::root`], and for the entry program's own
//! manifest that root is the PROGRAM's directory, not the manifest's.** loft resolves
//! a path a program passes relative to the program file, so `assets/game.pack` in
//! `src/game.loft` is `src/assets/game.pack`; rooting at the manifest would put a
//! different file in the page under the key the program asks for. Measured before the
//! fix: the desktop run answered `load=false` while its own page answered `load=true`.
//! A library's declarations keep the library's own root — a library's file is the
//! library's to locate, and no desktop-parity claim is made for it.
//!
//! The failure this removes is silent in F5's way: a page that carries the file
//! under a key nothing asks for answers `store_load` → `false`, the game draws no
//! art, and nothing on stderr says why.
//!
//! This is the exception `plans/146-content-delivery/ASSETS.md` names, not the
//! pipeline: assets travel as a store on a dumb file server, read by HTTP range
//! (F2/F3). Embedding is for the bytes a page needs before its first fetch, and for
//! a gallery page that has to be a single self-contained file.

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use crate::manifest::EmbedDecl;
use std::path::{Path, PathBuf};

/// One validated declaration: where the page carries the file, and where its bytes
/// come from on the build box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageFile {
    /// The key in `globalThis.loftBaseFS` — `/` + the declared `path`, which is what
    /// the page resolves the program's own relative string to.
    pub page_path: String,
    /// The file on the build box, already resolved against the declaring manifest.
    pub source: PathBuf,
}

/// Reject a value that cannot be embedded in the page's JavaScript. A page path
/// lands inside a quoted JS string inside a `<script>`, so a quote or a `<` would
/// end the construct it sits in.
fn embeddable(value: &str) -> Result<(), String> {
    if let Some(bad) = value
        .chars()
        .find(|c| matches!(c, '"' | '\'' | '<' | '>' | '\\' | '\n' | '\r'))
    {
        return Err(format!(
            "loft.toml: [[embed]] `path = \"{value}\"` contains `{bad}`, which cannot \
             appear in the page's JavaScript"
        ));
    }
    Ok(())
}

/// The page key for a declared `path`, or the reason it cannot have one.
///
/// The rule is one sentence: the declared path must be **exactly** what the program
/// passes, so the page can carry the file under exactly that name. Everything
/// refused here is a spelling that resolves to something other than itself —
/// `./a/b`, `a/../b`, a leading `/` — which would leave the program asking for one
/// key while the page holds another.
fn page_key(path: &str) -> Result<String, String> {
    let path = path.trim();
    if path.is_empty() {
        return Err(
            "loft.toml: an [[embed]] declares no `path` — the path is what the program \
             passes to read the file, so it cannot be left out"
                .to_string(),
        );
    }
    if path.starts_with('/') || path.contains('\\') || path.chars().nth(1) == Some(':') {
        return Err(format!(
            "loft.toml: [[embed]] `path = \"{path}\"` is not a relative path.\n  \
             `path` is what the PROGRAM passes, and a program that names a build-box \
             path runs on this box only.\n  \
             Name the path the program uses (`assets/game.pack`) and point `source` at \
             the build-box file."
        ));
    }
    if path
        .split('/')
        .any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Err(format!(
            "loft.toml: [[embed]] `path = \"{path}\"` is not in normal form.\n  \
             The page carries the file under this exact name, so a spelling that \
             resolves to something else would leave the program asking for a key the \
             page does not hold.\n  \
             Write it the way the program writes it, with no `.`, `..` or empty \
             segments."
        ));
    }
    embeddable(path)?;
    Ok(format!("/{path}"))
}

/// Check the `[[embed]]` declarations and answer the files the page must carry.
///
/// Refuses **before** the wasm compile, the way F5 does: a manifest that cannot
/// produce a working page should not cost a wasm build first.
///
/// # Errors
/// One message per problem found, joined by newlines — a build-stopping diagnostic.
pub fn validate(decls: &[EmbedDecl]) -> Result<Vec<PageFile>, String> {
    let mut out: Vec<PageFile> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    for d in decls {
        let declared = d.path.clone().unwrap_or_default();
        let page_path = match page_key(&declared) {
            Ok(k) => k,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        // `source` defaults to `path` — the common case is a file that ships where
        // the program reads it, and repeating the name would only be a second thing
        // to keep in step.
        let rel = d.source.clone().unwrap_or_else(|| declared.clone());
        let source = if Path::new(&rel).is_absolute() || d.root.is_empty() {
            PathBuf::from(&rel)
        } else {
            Path::new(&d.root).join(&rel)
        };
        if !source.is_file() {
            errors.push(format!(
                "loft.toml: [[embed]] `{declared}` has no file at `{}`.\n  \
                 The page carries the bytes themselves, so they have to exist when it \
                 is built.\n  \
                 Build the file first (a `[[build.asset]]` step is the usual home for \
                 that), or fix `source`.",
                source.display()
            ));
            continue;
        }
        let entry = PageFile { page_path, source };
        match out.iter().find(|p| p.page_path == entry.page_path) {
            // Declared twice identically — an app and a library it uses may name the
            // same file, and agreeing about it is not an error.
            Some(prev) if *prev == entry => {}
            Some(prev) => errors.push(format!(
                "loft.toml: [[embed]] `{declared}` is declared twice from different \
                 sources (`{}` and `{}`) — the page can carry one file under one \
                 name only",
                prev.source.display(),
                entry.source.display()
            )),
            None => out.push(entry),
        }
    }
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors.join("\n"))
    }
}

/// The statement that seeds the page's filesystem, run before anything reads it.
///
/// Empty when nothing is declared, so a page that embeds nothing is byte-identical
/// to one built before this existed. It ADDS to `globalThis.loftBaseFS` rather than
/// replacing it: a page may hand-seed its own tree, and a library's `host_js` may
/// too, and the declaration is not entitled to throw either away.
///
/// # Errors
/// The file that could not be read, and why. Between [`validate`] and here a source
/// can still vanish, and a page silently missing an asset is the failure this whole
/// module exists to remove.
pub fn base_fs_js(files: &[PageFile]) -> Result<String, String> {
    if files.is_empty() {
        return Ok(String::new());
    }
    use std::fmt::Write as _;
    let mut entries = String::new();
    for f in files {
        let bytes = std::fs::read(&f.source).map_err(|e| {
            format!(
                "loft: cannot embed '{}' into the page: {e}",
                f.source.display()
            )
        })?;
        if !entries.is_empty() {
            entries.push(',');
        }
        let b64 = crate::base64::encode(&bytes);
        let _ = write!(
            entries,
            "\n\"{}\":Uint8Array.from(atob(\"{b64}\"),c=>c.charCodeAt(0))",
            f.page_path
        );
    }
    Ok(format!(
        "// @PLN146 F4 — the files this page carries in its own filesystem, declared\n\
         // as [[embed]] in loft.toml.  `loft-fs.js` resolves a program's relative path\n\
         // against `/`, so a key here is the exact string the program reads by.\n\
         globalThis.loftBaseFS=Object.assign(globalThis.loftBaseFS||{{}},{{{entries}}});\n"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decl(path: &str, source: Option<&str>, root: &str) -> EmbedDecl {
        EmbedDecl {
            path: Some(path.to_string()),
            source: source.map(str::to_string),
            root: root.to_string(),
        }
    }

    /// A temp dir holding one file, so the existence check has something to find.
    fn fixture(name: &str, body: &[u8]) -> (PathBuf, String) {
        let dir = std::env::temp_dir().join(format!("loft_embed_unit_{name}"));
        let _ = std::fs::create_dir_all(dir.join("assets"));
        let rel = format!("assets/{name}.pack");
        std::fs::write(dir.join(&rel), body).expect("write fixture");
        (dir.clone(), rel)
    }

    #[test]
    fn a_relative_path_becomes_the_key_the_program_asks_for() {
        let (root, rel) = fixture("plain", b"hello");
        let files = validate(&[decl(&rel, None, &root.to_string_lossy())]).expect("valid");
        assert_eq!(files.len(), 1);
        // The whole invariant: `store_load(q, "assets/plain.pack")` resolves to this.
        assert_eq!(files[0].page_path, "/assets/plain.pack");
        assert_eq!(files[0].source, root.join(&rel));
    }

    #[test]
    fn a_path_that_is_not_the_programs_path_is_refused() {
        let (root, rel) = fixture("drift", b"x");
        let r = root.to_string_lossy().to_string();
        // Absolute: carried faithfully, under a key only this box could ask for.
        let abs = root.join(&rel).to_string_lossy().to_string();
        assert!(validate(&[decl(&abs, None, &r)]).is_err());
        // Non-normal spellings resolve to something other than themselves.
        for p in ["./assets/drift.pack", "a/../assets/drift.pack", "assets//x"] {
            assert!(validate(&[decl(p, None, &r)]).is_err(), "{p} was accepted");
        }
        // Nothing declared at all.
        assert!(validate(&[EmbedDecl::default()]).is_err());
        // A name that would break out of the page's script.
        assert!(validate(&[decl("a\"></script>.pack", None, &r)]).is_err());
        // A Windows spelling is refused as a PATH problem, not as a JavaScript one —
        // `\\` trips both rules, and only one of them names what is actually wrong.
        for p in ["C:\\assets\\drift.pack", "assets\\drift.pack"] {
            let e = validate(&[decl(p, None, &r)]).expect_err("must refuse");
            assert!(e.contains("is not a relative path"), "{e}");
        }
        // The control: the same file, spelled the way the program spells it.
        assert!(validate(&[decl(&rel, None, &r)]).is_ok());
    }

    #[test]
    fn a_source_that_is_not_there_stops_the_build() {
        let (root, rel) = fixture("absent", b"x");
        let r = root.to_string_lossy().to_string();
        let err = validate(&[decl("assets/nope.pack", None, &r)]).expect_err("must refuse");
        assert!(err.contains("has no file at"), "{err}");
        // `source` redirects where the bytes come from without moving the page key.
        let files = validate(&[decl("assets/renamed.pack", Some(&rel), &r)]).expect("valid");
        assert_eq!(files[0].page_path, "/assets/renamed.pack");
        assert_eq!(files[0].source, root.join(&rel));
    }

    #[test]
    fn one_page_path_cannot_come_from_two_files() {
        let (root, rel) = fixture("dup", b"x");
        let r = root.to_string_lossy().to_string();
        let other = "assets/dup2.pack";
        std::fs::write(root.join(other), b"y").expect("write second");
        // Two declarations that AGREE are one file — an app and a library it uses may
        // both name the pack they read.
        let twice = vec![decl(&rel, None, &r), decl(&rel, None, &r)];
        assert_eq!(validate(&twice).expect("agreeing duplicates").len(), 1);
        // Disagreeing ones are a conflict.
        let clash = vec![decl(&rel, None, &r), decl(&rel, Some(other), &r)];
        assert!(validate(&clash).is_err());
    }

    #[test]
    fn a_librarys_source_is_relative_to_the_library() {
        let (root, rel) = fixture("lib", b"librarybytes");
        // Same declaration, two roots: the resolved source follows the declaring
        // manifest, which is the whole reason `root` is carried on the decl.
        let mine = validate(&[decl(&rel, None, &root.to_string_lossy())]).expect("valid");
        assert_eq!(mine[0].source, root.join(&rel));
        let elsewhere = validate(&[decl(&rel, None, "/nowhere")]);
        assert!(elsewhere.is_err(), "a root with no such file must refuse");
    }

    #[test]
    fn nothing_declared_emits_nothing() {
        // C5: a page that embeds nothing must be what it was before this existed.
        assert_eq!(validate(&[]).expect("no decls").len(), 0);
        assert_eq!(base_fs_js(&[]).expect("no files"), "");
    }

    #[test]
    fn the_emitted_seed_carries_the_bytes_under_the_page_key() {
        let (root, rel) = fixture("emit", b"PACKBYTES");
        let files = validate(&[decl(&rel, None, &root.to_string_lossy())]).expect("valid");
        let js = base_fs_js(&files).expect("read the fixture");
        assert!(js.contains("\"/assets/emit.pack\":"), "{js}");
        assert!(js.contains(&crate::base64::encode(b"PACKBYTES")), "{js}");
        // It adds to whatever the page already had rather than replacing it.
        assert!(js.contains("globalThis.loftBaseFS||{}"), "{js}");
    }
}
