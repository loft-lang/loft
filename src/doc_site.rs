// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN183 P3 — the overview, as a small Markdown site an editor opens.
//!
//! "What can I use here?" has three answers: every language feature, every library, and what
//! one type can do.  The language server writes them as linked Markdown pages under
//! `~/.loft/doc/overview-<loft version>/` and asks the editor to open the root
//! (`window/showDocument`), so every client shows the overview with no code of its own: a
//! Markdown file is something every editor already opens, and a link is how it drills down.
//!
//! The pages are built from the same sources as the web pages and the REPL — the embedded
//! feature catalogue, the registry index already on this machine, the program's own parse —
//! through the one renderer (`doc_render`), so the three surfaces say the same thing.  Building
//! them is pure (a list of pages); writing them is a separate step that skips when nothing they
//! depend on changed, so the site is never rebuilt while the user types.

use crate::data::DefType;
use crate::doc_catalogue::{self, Entry, GROUPS, MAINTAINERS_GROUP};
use crate::doc_render;
use crate::parser::Parser;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

/// One page of the site: its file name and its Markdown.
pub struct Page {
    pub name: String,
    pub text: String,
}

/// The root page every invocation opens.
pub const ROOT: &str = "index.md";

/// The first sentence of `s` — a summary line short enough for a list.
fn first_sentence(s: &str) -> String {
    let flat = s.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut = flat.find(". ").map_or(flat.len(), |i| i + 1);
    let line = &flat[..cut];
    if line.chars().count() > 160 {
        let short: String = line.chars().take(159).collect();
        format!("{short}…")
    } else {
        line.to_string()
    }
}

fn entry_file(e: &Entry) -> String {
    format!("{}.md", e.tag)
}

fn entry_line(e: &Entry) -> String {
    format!(
        "- [@{} — {}]({}) — {}",
        e.tag,
        e.title,
        entry_file(e),
        first_sentence(&e.summary())
    )
}

/// One catalogue entry's page: its title, its body through the Markdown back-end, and where
/// its full text lives.
#[must_use]
pub fn entry_page(e: &Entry) -> Page {
    let mut text = format!(
        "[← what you can use](index.md)\n\n# @{} — {}\n\n",
        e.tag, e.title
    );
    text.push_str(&doc_render::blocks_markdown(&e.blocks()));
    let _ = write!(text, "\n\nFull text: <{}>\n", e.page());
    Page {
        name: entry_file(e),
        text,
    }
}

/// The feature half of the site: the root page's feature groups, the maintainers' page, and
/// one page per catalogue entry.  The root ends with the libraries link; the libraries pages
/// come from [`library_pages`].
#[must_use]
pub fn feature_pages() -> Vec<Page> {
    let mut root = format!(
        "# What you can use in loft {}\n\nEvery language feature by subject, then every library.  \
         Follow a link for the whole entry and its example.\n",
        crate::manifest::LOFT_RUNNING_VERSION
    );
    let mut inside = "[← what you can use](index.md)\n\n# Inside loft (maintainers)\n\n\
                      How loft itself is built — the entries a program does not use.\n\n"
        .to_string();
    for (key, title) in GROUPS {
        let entries: Vec<&Entry> = doc_catalogue::in_group(key).collect();
        if entries.is_empty() {
            continue;
        }
        if *key == MAINTAINERS_GROUP {
            for e in &entries {
                let _ = writeln!(inside, "{}", entry_line(e));
            }
            continue;
        }
        let _ = write!(root, "\n## {title}\n\n");
        for e in &entries {
            let _ = writeln!(root, "{}", entry_line(e));
        }
    }
    let inside_count = doc_catalogue::in_group(MAINTAINERS_GROUP).count();
    let _ = write!(
        root,
        "\n## Libraries\n\n- [Every library you can use](libraries.md)\n\n\
         ## Inside loft\n\n- [How loft itself is built](inside.md) — {inside_count} entries for \
         maintainers\n"
    );
    let mut pages = vec![
        Page {
            name: ROOT.to_string(),
            text: root,
        },
        Page {
            name: "inside.md".to_string(),
            text: inside,
        },
    ];
    pages.extend(doc_catalogue::entries().iter().map(entry_page));
    pages
}

/// The type a public item takes as `self`, read off its signature (`fn now(self: Clock) …`),
/// or `None` for a free function or a type.
#[cfg(feature = "registry")]
fn self_type(sig: &str) -> Option<String> {
    let rest = sig.split_once("(self: ")?.1;
    let end = rest.find([',', ')']).unwrap_or(rest.len());
    let ty = rest[..end].trim().trim_start_matches('&');
    (!ty.is_empty()).then(|| ty.to_string())
}

/// The library half of the site: `libraries.md` (every package in the registry index on this
/// machine, grouped by its first category, with its newest version, the version this project
/// locks and the one installed here) and one page per library listing its public surface,
/// grouped by the type each item takes as `self`, then the free items.  `locked` is the
/// project's `loft.lock` (`(name, version)`), `installed` the packages under the registry cache.
#[cfg(feature = "registry")]
#[must_use]
pub fn library_pages(
    index: Result<&crate::registry_index::RegistryIndex, &str>,
    locked: &[(String, String)],
    installed: &[(String, String)],
) -> Vec<Page> {
    use crate::registry_index::find_best_version;
    let mut list = "[← what you can use](index.md)\n\n# Libraries\n\n".to_string();
    let index = match index {
        Ok(i) => i,
        Err(why) => {
            let _ = writeln!(list, "{why}");
            return vec![Page {
                name: "libraries.md".to_string(),
                text: list,
            }];
        }
    };
    let mut pages = Vec::new();
    let mut groups: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for (name, pkg) in &index.packages {
        let Some(newest) = find_best_version(pkg, "*", false) else {
            continue;
        };
        let lock = locked
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str());
        let here = installed
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
            .next_back();
        let mut state = format!("newest {}", newest.semver);
        if let Some(v) = lock {
            let _ = write!(state, " · this project uses {v}");
        }
        match here {
            Some(v) => {
                let _ = write!(state, " · installed {v}");
            }
            None => {
                let _ = write!(state, " · `loft install {name}`");
            }
        }
        let desc = pkg.description.as_deref().unwrap_or("");
        let cat = pkg
            .categories
            .first()
            .cloned()
            .unwrap_or_else(|| "other".to_string());
        groups
            .entry(cat)
            .or_default()
            .push(format!("- [{name}](lib-{name}.md) — {desc} ({state})"));
        // The surface of the version this project uses, when it locks one the index knows;
        // else the newest.
        let shown = lock
            .and_then(|v| pkg.versions.values().find(|ver| ver.semver == v))
            .unwrap_or(newest);
        pages.push(library_page(name, desc, shown));
    }
    for (cat, lines) in &groups {
        let _ = write!(list, "## {cat}\n\n");
        for l in lines {
            let _ = writeln!(list, "{l}");
        }
        list.push('\n');
    }
    pages.insert(
        0,
        Page {
            name: "libraries.md".to_string(),
            text: list,
        },
    );
    pages
}

/// One library's page: the public surface of `shown`, grouped by the type each item takes as
/// `self`, then the free items — or, when the index records no surface for that version, where
/// to read it instead.
#[cfg(feature = "registry")]
fn library_page(name: &str, desc: &str, shown: &crate::registry_index::Version) -> Page {
    let mut page = format!(
        "[← libraries](libraries.md)\n\n# {name} {}\n\n{desc}\n\n",
        shown.semver
    );
    if shown.api.is_empty() {
        let _ = writeln!(
            page,
            "The registry does not record the public surface of {name} {} — `loft api {name}` \
             reads it from the installed package.",
            shown.semver
        );
    } else {
        let mut by_type: std::collections::BTreeMap<String, Vec<String>> =
            std::collections::BTreeMap::new();
        let mut free = Vec::new();
        for item in &shown.api {
            let md = doc_render::item_markdown(&doc_render::Item {
                sig: &item.sig,
                doc: &item.doc,
            });
            match self_type(&item.sig) {
                Some(t) => by_type.entry(t).or_default().push(md),
                None => free.push(md),
            }
        }
        for (t, items) in &by_type {
            let _ = write!(page, "## {t}\n\n");
            for md in items {
                let _ = write!(page, "{md}\n\n");
            }
        }
        if !free.is_empty() {
            page.push_str("## Types and functions\n\n");
            for md in &free {
                let _ = write!(page, "{md}\n\n");
            }
        }
    }
    Page {
        name: format!("lib-{name}.md"),
        text: page,
    }
}

/// @PLN183 P4/P5 — a library's card for a hover: which library, the version this program
/// uses, what it is, and where its guide and API pages are.  `dir` is the installed copy the
/// program resolved (`~/.loft/registry/<name>-<version>/`); its `docs/` says whether a guide
/// exists, and the registry index on this machine gives the description when it is here.
#[cfg(feature = "registry")]
#[must_use]
pub fn library_card(dir: &Path) -> Option<String> {
    let base = dir.file_name()?.to_str()?;
    let (name, version) = base.rsplit_once('-')?;
    if !version.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        return None;
    }
    let site = crate::documentation::SITE_BASE;
    let desc = crate::install::cached_index()
        .ok()
        .and_then(|i| i.packages.get(name).and_then(|p| p.description.clone()))
        .unwrap_or_default();
    let has_guide = std::fs::read_dir(dir.join("docs")).is_ok_and(|mut d| {
        d.any(|e| e.is_ok_and(|e| e.path().extension().is_some_and(|x| x == "loft")))
    });
    let mut card = format!("**library `{name}`** {version}");
    if !desc.is_empty() {
        let _ = write!(card, " — {desc}");
    }
    let _ = write!(card, "\n\n[API]({site}lib-{name}-api.html)");
    if has_guide {
        let _ = write!(card, " · [guide]({site}lib-{name}-guide.html)");
    }
    Some(card)
}

/// The version of library `name` the `loft.lock` nearest above `project` pins, if any.
#[must_use]
pub fn locked_version(name: &str, project: &Path) -> Option<String> {
    let lock = project
        .ancestors()
        .map(|d| d.join("loft.lock"))
        .find(|l| l.is_file())?;
    crate::lockfile::read_lockfile(&lock)
        .ok()
        .flatten()?
        .packages
        .into_iter()
        .find(|p| p.name == name)
        .map(|p| p.version)
}

/// The installed copy of library `name` a program in `project` uses: the version its
/// `loft.lock` pins when that copy is installed, else the newest installed copy.
#[cfg(feature = "registry")]
#[must_use]
pub fn installed_library(name: &str, project: Option<&Path>) -> Option<PathBuf> {
    let installed: Vec<(String, String, PathBuf)> = crate::registry_index::installed_packages()
        .into_iter()
        .filter(|(n, _, _)| n == name)
        .collect();
    let locked = project.and_then(|p| locked_version(name, p));
    if let Some(v) = locked
        && let Some((_, _, dir)) = installed.iter().find(|(_, iv, _)| *iv == v)
    {
        return Some(dir.clone());
    }
    installed
        .into_iter()
        .max_by(|a, b| crate::registry_index::compare_semver(&a.1, &b.1))
        .map(|(_, _, dir)| dir)
}

/// What one type can be written with — @PLN182's operator forms with the definition behind
/// each, the `[ ]` forms, the interfaces it meets.  The REPL's `:ops` and the editor's type page
/// both render this, so the two cannot disagree.
pub struct TypeCaps {
    pub name: String,
    /// `(the spelling, the definition behind it)`.
    pub operators: Vec<(String, String)>,
    pub index_forms: String,
    pub meets: Vec<String>,
}

/// The capabilities of type `ty` in `parser`'s program, or `None` when no such type is in scope.
#[must_use]
pub fn type_capabilities(parser: &Parser, ty: &str) -> Option<TypeCaps> {
    let data = &parser.data;
    let ty_nr = data.def_nr(ty);
    if ty.is_empty() || ty_nr == u32::MAX {
        return None;
    }
    let prefix = format!("t_{}{ty}_", ty.len());
    let mut operators: Vec<(String, String)> = Vec::new();
    for d in 0..data.definitions() {
        let def = data.def(d);
        if !def.operator_form || def.def_type != DefType::Function {
            continue;
        }
        let Some(rest) = def.name.strip_prefix(&prefix) else {
            continue;
        };
        let form = rest.split('#').next().unwrap_or(rest);
        let symbol = match form {
            "compare" => "<  <=  >  >=".to_string(),
            "plus" => "+  +=".to_string(),
            "minus" => "-  -=".to_string(),
            "times" => "*  *=".to_string(),
            "divided_by" => "/  /=".to_string(),
            "remainder" => "%  %=".to_string(),
            "negate" => "-x".to_string(),
            "next" => "for e in x".to_string(),
            "to_text" => "\"{x}\"".to_string(),
            f => f
                .strip_prefix("to_")
                .map_or_else(|| f.to_string(), |t| format!("x as {t}")),
        };
        let sig = format!(
            "operator {form}{}",
            crate::api_surface::signature_of(data, d, "fn")
        );
        if !operators.iter().any(|(_, s)| *s == sig) {
            operators.push((symbol, sig));
        }
    }
    let index_forms = match ty {
        "vector" => "v[i] an element · v[a..b] a slice",
        "text" => "s[i] one character · s[a..b] a slice (byte offsets)",
        "hash" | "sorted" | "index" | "spatial" | "trie" => "c[key] the record with that key",
        _ if matches!(data.def(ty_nr).def_type, DefType::Struct | DefType::Enum)
            && !data.def(ty_nr).is_stdlib() =>
        {
            "none — a type of its own reads an element through a named method (@F114)"
        }
        _ => "none",
    }
    .to_string();
    let mut meets = Vec::new();
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type == DefType::Interface
            && !def.name.starts_with("__")
            && parser.satisfaction_failures(d, ty_nr).is_empty()
        {
            meets.push(def.name.clone());
        }
    }
    meets.sort();
    meets.dedup();
    Some(TypeCaps {
        name: ty.to_string(),
        operators,
        index_forms,
        meets,
    })
}

const EQUALITY: &str = "compare by value — a record field by field; no type redefines them";

/// The REPL's `:ops` text.
#[must_use]
pub fn caps_text(c: &TypeCaps) -> String {
    let mut out = format!("{}\n", c.name);
    let _ = writeln!(out, "  operators");
    for (symbol, sig) in &c.operators {
        let _ = writeln!(out, "    {symbol:<14} {sig}");
    }
    let _ = writeln!(out, "    {:<14} {EQUALITY}", "==  !=");
    let _ = writeln!(out, "  [ ]\n    {}", c.index_forms);
    let _ = writeln!(
        out,
        "  meets\n    {}",
        if c.meets.is_empty() {
            "no interface".to_string()
        } else {
            c.meets.join(", ")
        }
    );
    out
}

/// The editor's type page: the same facts as [`caps_text`], as Markdown.
#[must_use]
pub fn type_page(c: &TypeCaps) -> Page {
    let mut text = format!(
        "[← what you can use](index.md)\n\n# What `{}` can do\n\n## Operators\n\n",
        c.name
    );
    for (symbol, sig) in &c.operators {
        let _ = writeln!(text, "- `{symbol}` — `{sig}`");
    }
    let _ = write!(
        text,
        "- `==  !=` — {EQUALITY}\n\n## [ ]\n\n{}\n\n## Meets\n\n{}\n",
        c.index_forms,
        if c.meets.is_empty() {
            "no interface".to_string()
        } else {
            c.meets
                .iter()
                .map(|m| format!("`{m}`"))
                .collect::<Vec<_>>()
                .join(", ")
        }
    );
    Page {
        name: format!("type-{}.md", c.name),
        text,
    }
}

/// Where the site lives: `~/.loft/doc/overview-<loft version>/`, honouring `LOFT_HOME` like
/// the rest of loft's per-user files.
#[must_use]
pub fn site_dir() -> PathBuf {
    std::env::var_os("LOFT_HOME")
        .map(PathBuf::from)
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".loft")
        .join("doc")
        .join(format!(
            "overview-{}",
            crate::manifest::LOFT_RUNNING_VERSION
        ))
}

/// Write `pages` into `dir` unless its `.stamp` already says `stamp` — the site depends only
/// on what `stamp` names (the loft version, the project's lock, the registry index), so an
/// unchanged stamp means every page is already what it would be.  Answers whether it wrote.
///
/// # Errors
/// A page or the stamp could not be written.
pub fn write_site(dir: &Path, pages: &[Page], stamp: &str) -> std::io::Result<bool> {
    let stamp_path = dir.join(".stamp");
    if dir.join(ROOT).is_file() && std::fs::read_to_string(&stamp_path).is_ok_and(|s| s == stamp) {
        return Ok(false);
    }
    std::fs::create_dir_all(dir)?;
    for p in pages {
        std::fs::write(dir.join(&p.name), &p.text)?;
    }
    std::fs::write(stamp_path, stamp)?;
    Ok(true)
}
