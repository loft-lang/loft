// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The files a built program carries with it, found from what names them: an ESP-IDF app's
//! packed assets (`--native-esp`, esp_app.rs) and a page's base filesystem (`--native-c
//! --html`, when the program declares no `[[embed]]`).  One rule for both, so the device and the
//! page carry the same files (TANMATSU.md § A loft program as an app).

use loft::file_access as fa;
use std::path::{Path, PathBuf};

/// Every file under `dir`, as its path relative to `root` with `/` between names, sorted so
/// the packed table is the same on every run.
pub(crate) fn files_under(root: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fa::read_dir(dir) else {
        return;
    };
    for e in entries {
        let p = e.os_spelling();
        let name = fa::file_name(&p).unwrap_or_default();
        if name.starts_with('.') {
            continue;
        }
        if fa::is_dir(&p) {
            files_under(root, &p, out);
        } else if let Ok(rel) = p.strip_prefix(root) {
            let rel: Vec<String> = rel
                .components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect();
            out.push((rel.join("/"), p));
        }
    }
    out.sort();
}

/// `*` in a pattern matches any run of characters inside one name (never a `/`).
pub(crate) fn glob_match(pattern: &[u8], path: &[u8]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((b'*', rest)) => (0..=path.len())
            .take_while(|&k| k == 0 || path[k - 1] != b'/')
            .any(|k| glob_match(rest, &path[k..])),
        Some((c, rest)) => path
            .split_first()
            .is_some_and(|(p, tail)| p == c && glob_match(rest, tail)),
    }
}

/// The files `assets` names, relative to `program_dir` with `/` between names: a file, a
/// directory (every file under it), or a pattern with `*` (every file it matches); an entry
/// starting with `!` takes what it matches out again.  An entry that matches nothing is refused,
/// so a typing error does not ship an app without its files — except the default `assets`.
///
/// # Errors
/// An entry that names nothing beside the program.
pub(crate) fn asset_files(
    program_dir: &Path,
    assets: &[String],
) -> Result<Vec<(String, PathBuf)>, String> {
    let mut files = Vec::new();
    let mut every = Vec::new();
    files_under(program_dir, program_dir, &mut every);
    let mut drop = Vec::new();
    for a in assets {
        let a = a.trim_start_matches("./").replace('\\', "/");
        if let Some(out) = a.strip_prefix('!') {
            drop.push(out.to_string());
            continue;
        }
        let p = program_dir.join(&a);
        let before = files.len();
        if a.contains('*') {
            files.extend(
                every
                    .iter()
                    .filter(|(rel, _)| glob_match(a.as_bytes(), rel.as_bytes()))
                    .cloned(),
            );
        } else if fa::is_dir(&p) {
            files_under(program_dir, &p, &mut files);
        } else if fa::is_file(&p) {
            files.push((a.clone(), p));
        }
        if files.len() == before && a != "assets" {
            return Err(format!("--assets: `{a}` names no file beside the program"));
        }
    }
    files.retain(|(rel, _)| {
        !drop.iter().any(|d| {
            glob_match(d.as_bytes(), rel.as_bytes())
                || rel.starts_with(&format!("{}/", d.trim_end_matches('/')))
        })
    });
    files.sort();
    files.dedup();
    Ok(files)
}

/// `loft_assets.c`: each file `assets` names (a directory: every file under it) as a byte
/// array, and the table `loft_esp.h` serves them from by their path relative to the program.
/// Answers the C and the packed byte count.
///
/// # Errors
/// A named asset that does not exist, or one that cannot be read.
/// Every string literal of a C text, unescaped as far as a path needs (`\\"`, `\\\\`).
pub(crate) fn c_literals(c: &str) -> Vec<String> {
    let mut out = Vec::new();
    let b = c.as_bytes();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i = c[i + 2..].find("*/").map_or(b.len(), |e| i + 2 + e + 2);
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                i = c[i..].find('\n').map_or(b.len(), |e| i + e);
            }
            b'\'' => {
                // a character literal: skip it, so a `'"'` opens no string
                i += 1;
                while i < b.len() && b[i] != b'\'' {
                    i += if b[i] == b'\\' { 2 } else { 1 };
                }
                i += 1;
            }
            b'"' => {
                let mut lit = Vec::new();
                // A text constant of the program is `loft_lit("…", n)`; a fault's location
                // (`"game.loft", 120`) is not, and names a source file, never one opened.
                let constant = c[..i].ends_with("loft_lit(");
                i += 1;
                while i < b.len() && b[i] != b'"' {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        lit.push(b[i + 1]);
                        i += 2;
                    } else {
                        lit.push(b[i]);
                        i += 1;
                    }
                }
                i += 1;
                let after = &c[i.min(c.len())..];
                let location = !constant
                    && after
                        .strip_prefix(", ")
                        .is_some_and(|r| r.starts_with(|ch: char| ch.is_ascii_digit()));
                if !location {
                    out.push(String::from_utf8_lossy(&lit).into_owned());
                }
            }
            _ => i += 1,
        }
    }
    out
}

/// Why a file goes into the app: what named it.
pub(crate) type Reason = String;

/// The files beside the program that the program can open, found from what names them, never
/// from a list:
///
/// - its path written in the program (a text constant of the unit), or in a file already
///   packed (theme.json naming a sheet), with nothing but a `/` or a quote before it;
/// - a directory and an extension the program writes as constants of their own, which is how
///   a path built at run time leaves them in the C (`"{dir}/levels/{name}.json"` is the
///   constants `/levels/` and `.json`): every file directly in that directory with that
///   extension.
///
/// A file packed for the second reason is read for paths too.  What nothing names stays out.
pub(crate) fn named_assets(program_dir: &Path, c: &str) -> Vec<(String, PathBuf, Reason)> {
    let literals = c_literals(c);
    let dirs: Vec<String> = literals
        .iter()
        .filter(|l| l.ends_with('/') && l.len() > 1 && !l.contains(' '))
        .map(|l| l.trim_start_matches("./").trim_matches('/').to_string())
        .filter(|d| !d.is_empty())
        .collect();
    let exts: Vec<&str> = literals
        .iter()
        .map(String::as_str)
        .filter(|l| {
            l.len() > 1
                && l.len() <= 9
                && l.starts_with('.')
                && l[1..].bytes().all(|b| b.is_ascii_alphanumeric())
        })
        .collect();
    let mut every = Vec::new();
    files_under(program_dir, program_dir, &mut every);
    // A constant of the program names a file by its path relative to the program, or by that
    // path's tail after a directory built at run time (`"{dir}/theme.json"` leaves `/theme.json`).
    // An absolute path (a library's file on the build machine) reaches no packed file on the
    // device, where only relative paths fall through to the assets.
    let program_names = |rel: &str| {
        literals.iter().any(|l| {
            let l = l.strip_prefix("./").unwrap_or(l);
            l == rel || l.strip_prefix('/').is_some_and(|tail| tail == rel)
        })
    };
    // The packed texts a path can be named in, each with its file.
    let mut texts: Vec<(String, String)> = Vec::new();
    let mut chosen: Vec<(String, PathBuf, Reason)> = Vec::new();
    let names = |text: &str, rel: &str| {
        text.match_indices(rel).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + rel.len()..].chars().next();
            matches!(before, None | Some('/' | '"' | '\'' | '\n'))
                && matches!(after, None | Some('"' | '\'' | '\n'))
        })
    };
    let mut read = 0;
    loop {
        let mut grew = false;
        for (rel, path) in &every {
            if chosen.iter().any(|(r, _, _)| r == rel) {
                continue;
            }
            let reason = program_names(rel)
                .then(|| "named in the program".to_string())
                .or_else(|| {
                    texts[read.min(texts.len())..]
                        .iter()
                        .chain(texts[..read.min(texts.len())].iter())
                        .find(|(t, _)| names(t, rel))
                        .map(|(_, from)| format!("named in {from}"))
                })
                .or_else(|| {
                    let (dir, name) = rel.rsplit_once('/').unwrap_or(("", rel.as_str()));
                    let ext = name.rfind('.').map(|k| &name[k..])?;
                    (dirs.iter().any(|d| d == dir) && exts.contains(&ext))
                        .then(|| format!("in `{dir}/`, a `{ext}` the program builds a path to"))
                });
            if let Some(reason) = reason {
                chosen.push((rel.clone(), path.clone(), reason));
                grew = true;
            }
        }
        read = texts.len();
        // A packed text file can name more files: read each one once.
        for (rel, path, _) in &chosen {
            if texts.iter().any(|(_, from)| from == rel) {
                continue;
            }
            if let Ok(t) = fa::read_to_string(path) {
                if t.len() <= 1 << 20 {
                    texts.push((t, rel.clone()));
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    chosen.sort();
    chosen
}
