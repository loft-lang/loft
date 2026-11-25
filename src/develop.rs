// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I77 — Registry / manifest / lockfile resolution: a library switched to an editable copy and back

//! `loft develop` — `@C173`: a library the project uses can always be debugged and fixed.
//!
//! A pinned library loads only from its pin, so a same-named file that merely lies in a
//! directory never replaces it.  Developing one is therefore an explicit act, and this
//! command is that act in one step: it puts an editable copy of the library where the project
//! can see it and declares it as a `{ path = … }` dependency, which outranks the pin.  The
//! declaration keeps the version it replaced and says what it was, so `--done` puts the
//! project back exactly as it found it.
//!
//! ```text
//! loft develop <name>                a copy of the version in use, in lib/<name>
//! loft develop <name> --path <dir>   your own checkout of it (a clone of its repository)
//! loft develop --done <name>         back to the declaration it replaced; the copy stays
//! loft develop                       the libraries this project develops
//! ```
//!
//! For one run, `--lib <dir>` does the same without touching the project.

use crate::file_access as fa;
use std::path::{Path, PathBuf};

/// The comment that marks a line `loft develop` wrote, followed by what the line held before:
/// a quoted value, or `absent` when the project did not declare the library.
const MARK: &str = "# loft develop: was ";

/// Run `loft develop` with the arguments after the subcommand; answers the exit code.
#[must_use]
pub fn run(args: &[String]) -> i32 {
    let manifest = PathBuf::from("loft.toml");
    let Ok(text) = fa::read_to_string(&manifest) else {
        eprintln!(
            "loft develop: run it in a package's directory (one with a loft.toml); for a single \
             script, `--lib <dir>` runs a copy of a library for that run"
        );
        return 1;
    };
    let result = match args {
        [] => {
            for (name, path, was) in developed(&text) {
                println!("{name}  {path}  (was {was})");
            }
            return 0;
        }
        [done, name] if done == "--done" => done_with(&text, name),
        [name] => start(&text, name, None),
        [name, flag, dir] if flag == "--path" => start(&text, name, Some(dir)),
        _ => Err(
            "usage: loft develop <name> [--path <dir>] | loft develop --done <name> | loft develop"
                .to_string(),
        ),
    };
    match result {
        Ok((new_text, said)) => {
            if let Err(e) = fa::write(&manifest, new_text) {
                eprintln!("loft develop: cannot write loft.toml: {e}");
                return 1;
            }
            println!("{said}");
            0
        }
        Err(e) => {
            eprintln!("loft develop: {e}");
            1
        }
    }
}

/// Develop `name`: from `dir` when given, else from a copy of the version in use.  Answers the
/// new manifest text and the line to print.
fn start(text: &str, name: &str, dir: Option<&String>) -> Result<(String, String), String> {
    let current = dependency(text, name);
    if let Some(dep) = &current {
        if dep.comment.contains(MARK) {
            return Err(format!(
                "`{name}` is already being developed; `loft develop --done {name}` ends that"
            ));
        }
        if crate::manifest::extract_path_dep(&dep.value).is_some() {
            return Err(format!(
                "`{name}` is already a path dependency ({}); edit it there",
                dep.value
            ));
        }
    }
    let version = version_in_use(name, current.as_ref().map(|d| d.value.as_str()))?;
    let path = match dir {
        Some(d) => {
            let own = crate::manifest::read_manifest(&format!("{d}/loft.toml"))
                .and_then(|m| m.name)
                .ok_or_else(|| format!("{d} is not a loft package (no loft.toml with a name)"))?;
            if own != name {
                return Err(format!("{d} is the package `{own}`, not `{name}`"));
            }
            d.clone()
        }
        None => copy_in_use(name, &version)?,
    };
    let was = current
        .as_ref()
        .map_or_else(|| "absent".to_string(), |d| d.value.clone());
    let req = current
        .as_ref()
        .and_then(|d| {
            crate::manifest::extract_version_req(&d.value).map(|v| v.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| version.clone());
    let line = format!("{name} = {{ path = \"{path}\", version = \"{req}\" }}  {MARK}{was}");
    let new_text = set_line(text, name, &line);
    Ok((
        new_text,
        format!(
            "{name}: developed in {path} (it outranks the pinned {version}); `loft develop --done \
             {name}` returns to it"
        ),
    ))
}

/// End developing `name`: the line `loft develop` wrote goes back to what it replaced.
fn done_with(text: &str, name: &str) -> Result<(String, String), String> {
    let dep = dependency(text, name)
        .filter(|d| d.comment.contains(MARK))
        .ok_or_else(|| format!("`{name}` is not being developed"))?;
    let was = dep.comment.split_once(MARK).map_or("", |(_, w)| w.trim());
    let path = crate::manifest::extract_path_dep(&dep.value)
        .unwrap_or("")
        .to_string();
    let new_text = if was == "absent" {
        remove_line(text, dep.line)
    } else {
        set_line(text, name, &format!("{name} = {was}"))
    };
    Ok((
        new_text,
        format!("{name}: back to {was}; {path} is left as it is"),
    ))
}

/// The libraries this project develops: `(name, path, what the line held before)`.
fn developed(text: &str) -> Vec<(String, String, String)> {
    dependency_lines(text)
        .into_iter()
        .filter_map(|d| {
            let (_, was) = d.comment.split_once(MARK)?;
            let path = crate::manifest::extract_path_dep(&d.value)?.to_string();
            Some((d.name, path, was.trim().to_string()))
        })
        .collect()
}

/// The version of `name` the project uses: the lock's pin, else the one installed copy that
/// meets the declared requirement.
fn version_in_use(name: &str, declared: Option<&str>) -> Result<String, String> {
    if let Ok(Some(lock)) = crate::lockfile::read_lockfile(Path::new("loft.lock"))
        && let Some(p) = lock.packages.iter().find(|p| p.name == name)
    {
        return Ok(p.version.clone());
    }
    let wanted = declared.and_then(crate::manifest::extract_version_req);
    Err(match wanted {
        Some(req) => format!(
            "`{name}` ({req}) is not in loft.lock; run `loft install` first, or name a checkout \
             with `--path <dir>`"
        ),
        None => format!(
            "this project does not use `{name}`; name a checkout with `loft develop {name} --path \
             <dir>`"
        ),
    })
}

/// Copy the installed `name` `version` into `lib/<name>`, answering that path.
#[cfg(feature = "registry")]
fn copy_in_use(name: &str, version: &str) -> Result<String, String> {
    let from = crate::registry_index::extract_dir(name, version);
    if !fa::is_dir(&from) {
        return Err(format!(
            "`{name}` {version} is not installed here; run `loft install` first"
        ));
    }
    let to = format!("lib/{name}");
    if fa::exists(&to) {
        return Err(format!(
            "{to} exists already; `loft develop {name} --path {to}` uses it as it is"
        ));
    }
    crate::package_layout::copy_package_tree(&from, Path::new(&to))
        .map_err(|e| format!("cannot copy {} to {to}: {e}", from.display()))?;
    Ok(to)
}

#[cfg(not(feature = "registry"))]
fn copy_in_use(name: &str, _version: &str) -> Result<String, String> {
    Err(format!(
        "this loft was built without the registry; name a checkout of `{name}` with `--path <dir>`"
    ))
}

/// One `name = value  # comment` line of the `[dependencies]` table.
struct DepLine {
    line: usize,
    name: String,
    value: String,
    comment: String,
}

/// The `[dependencies]` lines of a manifest, in order.
fn dependency_lines(text: &str) -> Vec<DepLine> {
    let mut out = Vec::new();
    let mut inside = false;
    for (i, raw) in text.lines().enumerate() {
        let t = raw.trim();
        if t.starts_with('[') {
            inside = t == "[dependencies]";
            continue;
        }
        if !inside || t.is_empty() || t.starts_with('#') {
            continue;
        }
        let Some((key, rest)) = t.split_once('=') else {
            continue;
        };
        let (value, comment) = split_comment(rest.trim());
        out.push(DepLine {
            line: i,
            name: key.trim().trim_matches('"').to_string(),
            value: value.to_string(),
            comment: comment.to_string(),
        });
    }
    out
}

fn dependency(text: &str, name: &str) -> Option<DepLine> {
    dependency_lines(text).into_iter().find(|d| d.name == name)
}

/// A value and its trailing comment: the first `#` outside a quoted string.
fn split_comment(s: &str) -> (&str, &str) {
    let mut quoted = false;
    for (i, c) in s.char_indices() {
        match c {
            '"' => quoted = !quoted,
            '#' if !quoted => return (s[..i].trim_end(), &s[i..]),
            _ => {}
        }
    }
    (s, "")
}

/// `text` with `name`'s `[dependencies]` line replaced by `line`, or `line` added to the table
/// (and the table to the manifest) when there is none.
fn set_line(text: &str, name: &str, line: &str) -> String {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    if let Some(dep) = dependency(text, name) {
        lines[dep.line] = line.to_string();
    } else if let Some(at) = lines.iter().position(|l| l.trim() == "[dependencies]") {
        lines.insert(at + 1, line.to_string());
    } else {
        if lines.last().is_some_and(|l| !l.trim().is_empty()) {
            lines.push(String::new());
        }
        lines.push("[dependencies]".to_string());
        lines.push(line.to_string());
    }
    lines.join("\n") + "\n"
}

fn remove_line(text: &str, line: usize) -> String {
    let lines: Vec<&str> = text
        .lines()
        .enumerate()
        .filter_map(|(i, l)| (i != line).then_some(l))
        .collect();
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    const DECLARED: &str =
        "[package]\nname = \"app\"\n\n[dependencies]\nwidgets = \"0.3\"\ncrypto = \"0.5.0\"\n";

    /// `@C173` — developing a declared library rewrites only its line, keeps its version and
    /// says what it was; ending it restores the manifest byte for byte.
    #[test]
    fn a_declared_library_is_developed_and_restored_exactly() {
        let line = "crypto = { path = \"lib/crypto\", version = \"0.5.0\" }  # loft develop: was \"0.5.0\"";
        let dev = set_line(DECLARED, "crypto", line);
        assert!(
            dev.contains(line) && dev.contains("widgets = \"0.3\""),
            "{dev}"
        );
        assert_eq!(
            developed(&dev),
            vec![(
                "crypto".to_string(),
                "lib/crypto".to_string(),
                "\"0.5.0\"".to_string()
            )]
        );
        let (back, said) = done_with(&dev, "crypto").expect("it is developed");
        assert_eq!(back, DECLARED);
        assert!(said.contains("back to \"0.5.0\""), "{said}");
    }

    /// A library the project only uses through another (pinned, not declared) is added, and
    /// ending it removes the line again.
    #[test]
    fn an_undeclared_library_is_added_and_removed() {
        let line =
            "random = { path = \"lib/random\", version = \"0.5.3\" }  # loft develop: was absent";
        let dev = set_line(DECLARED, "random", line);
        assert!(dev.contains(line), "{dev}");
        let (back, _) = done_with(&dev, "random").expect("it is developed");
        assert_eq!(back, DECLARED);
        let bare = set_line("[package]\nname = \"app\"\n", "random", line);
        assert!(
            bare.ends_with(&format!("[dependencies]\n{line}\n")),
            "{bare}"
        );
    }

    #[test]
    fn a_line_loft_develop_did_not_write_is_not_ended() {
        assert!(done_with(DECLARED, "crypto").is_err());
        assert_eq!(developed(DECLARED), Vec::new());
    }

    #[test]
    fn a_comment_starts_outside_quotes_only() {
        assert_eq!(split_comment("\"a#b\"  # c"), ("\"a#b\"", "# c"));
        assert_eq!(split_comment("\"0.1\""), ("\"0.1\"", ""));
    }
}
