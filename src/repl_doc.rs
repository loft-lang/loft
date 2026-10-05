// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN183 P2 — the REPL's documentation commands, as text.
//!
//! The OVERVIEW first: `:features` lists every language feature by group, `:libs` every library
//! reachable here, `:api <lib>` what one offers.  Each answer is built from the same sources
//! the web pages are (the embedded feature catalogue, the registry index already on this
//! machine) through the one renderer (`doc_render`), so the terminal and the page say the
//! same thing.  Nothing here reaches the network.

use crate::doc_catalogue::{self, Entry, GROUPS, MAINTAINERS_GROUP};
use crate::doc_render;
#[cfg(feature = "registry")]
use crate::registry_index::{RegistryIndex, find_best_version};
use std::fmt::Write as _;

/// The column the REPL wraps its documentation to.
pub const WIDTH: usize = 96;

fn clip(s: &str, width: usize) -> String {
    if s.chars().count() <= width {
        s.to_string()
    } else {
        let cut: String = s.chars().take(width.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}

/// Does `e` answer `query` — its tag, a group, a key, or a word of its title or summary?
fn matches(e: &Entry, query: &str) -> bool {
    let q = query.to_lowercase();
    let q = q.strip_prefix('@').unwrap_or(&q);
    e.tag.to_lowercase() == q
        || e.group == q
        || e.keys
            .iter()
            .any(|k| k == q || k.split_once(':').is_some_and(|(_, s)| s == q))
        || e.title.to_lowercase().contains(q)
        || e.summary().to_lowercase().contains(q)
}

/// `:features [query]` — the language, grouped.  With no query, every group a programmer
/// uses; with a group's key (`:features inside`), that group; otherwise the entries whose
/// tag, key, title or summary answers the query, in their groups.
#[must_use]
pub fn features(query: &str, width: usize) -> String {
    let query = query.trim();
    let whole_group = GROUPS.iter().any(|(k, _)| *k == query);
    let mut out = String::new();
    let mut shown = 0;
    for (key, title) in GROUPS {
        let open = if query.is_empty() {
            *key != MAINTAINERS_GROUP
        } else {
            !whole_group || *key == query
        };
        if !open {
            continue;
        }
        let entries: Vec<&Entry> = doc_catalogue::in_group(key)
            .filter(|e| query.is_empty() || whole_group || matches(e, query))
            .collect();
        if entries.is_empty() {
            continue;
        }
        let _ = writeln!(out, "{title}");
        for e in entries {
            let tag = format!("@{}", e.tag);
            let _ = writeln!(
                out,
                "  {tag:<6} {}",
                clip(&e.title, width.saturating_sub(9))
            );
            shown += 1;
        }
        out.push('\n');
    }
    if shown == 0 {
        let _ = writeln!(
            out,
            "nothing in the catalogue answers `{query}` — `:features` lists it all"
        );
    } else if query.is_empty() {
        let _ = writeln!(
            out,
            "`:doc @F2` shows one entry · `:features <word>` searches · `:features {MAINTAINERS_GROUP}` is how loft itself is built"
        );
    }
    out
}

/// The entries a `:doc` query names: a tag (`@F2`), a construct key (`op:??`), or a key's
/// own spelling (`??`, `match`) — every entry that answers, so an ambiguous spelling
/// (`index` names a type and a `[]` form) shows each.
#[must_use]
pub fn find_entries(query: &str) -> Vec<&'static Entry> {
    let q = query.trim();
    if let Some(e) = doc_catalogue::by_tag(q) {
        return vec![e];
    }
    doc_catalogue::entries()
        .iter()
        .filter(|e| {
            e.keys
                .iter()
                .any(|k| k == q || k.split_once(':').is_some_and(|(_, s)| s == q))
        })
        .collect()
}

/// One catalogue entry, as the terminal shows it: the title, the body, where the full
/// text lives, and how to run the example.
#[must_use]
pub fn entry_text(e: &Entry, width: usize) -> String {
    let mut out = format!("@{} — {}\n\n", e.tag, e.title);
    out.push_str(&doc_render::blocks_text(&e.blocks(), width));
    let _ = write!(out, "\nFull text: {}\n", e.page());
    if e.example().is_some() {
        let _ = writeln!(
            out,
            "`:doc @{} run` runs the example in a scratch session.",
            e.tag
        );
    }
    out
}

/// `:libs` — every library reachable from here: the registry's packages grouped by their
/// first category, each with its newest version, the version installed on this machine
/// (`installed`: `(name, version)`), and its description.
#[cfg(feature = "registry")]
#[must_use]
pub fn libs(
    index: Result<&RegistryIndex, &str>,
    installed: &[(String, String)],
    width: usize,
) -> String {
    let index = match index {
        Ok(i) => i,
        Err(why) => return format!("{why}\n"),
    };
    let mut groups: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();
    for (name, pkg) in &index.packages {
        let Some(v) = find_best_version(pkg, "*", false) else {
            continue;
        };
        let here: Vec<&str> = installed
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, ver)| ver.as_str())
            .collect();
        let state = match here.last() {
            Some(ver) if *ver == v.semver => "installed".to_string(),
            Some(ver) => format!("installed {ver}"),
            None => String::new(),
        };
        let desc = pkg.description.as_deref().unwrap_or("");
        let line = format!("  {name:<16} {:<8} {state:<16} {desc}", v.semver);
        let cat = pkg
            .categories
            .first()
            .cloned()
            .unwrap_or_else(|| "other".to_string());
        groups.entry(cat).or_default().push(clip(&line, width));
    }
    let mut out = String::new();
    for (cat, lines) in &groups {
        let _ = writeln!(out, "{cat}");
        for l in lines {
            let _ = writeln!(out, "{}", l.trim_end());
        }
        out.push('\n');
    }
    let _ = writeln!(
        out,
        "{} libraries · `:api <library>` lists what one offers · `loft install <library>` fetches one",
        index.packages.len()
    );
    out
}

/// `:api <library> [filter]` — what a library offers: every public item of its newest
/// version, as the registry index records it, its signature and doc; with a filter, only
/// the items whose signature contains it.
#[cfg(feature = "registry")]
#[must_use]
pub fn api(index: Result<&RegistryIndex, &str>, lib: &str, filter: &str, width: usize) -> String {
    let index = match index {
        Ok(i) => i,
        Err(why) => return format!("{why}\n"),
    };
    let Some(pkg) = index.packages.get(lib) else {
        return format!("no library `{lib}` in the registry — `:libs` lists them\n");
    };
    let Some(v) = find_best_version(pkg, "*", false) else {
        return format!("`{lib}` has no published version\n");
    };
    if v.api.is_empty() {
        return format!(
            "the registry does not record the public surface of {lib} {} — `loft api {lib}` reads \
             it from the installed package\n",
            v.semver
        );
    }
    let items: Vec<_> = v.api.iter().filter(|i| i.sig.contains(filter)).collect();
    let mut out = format!("{lib} {} — {} public item(s)", v.semver, v.api.len());
    if !filter.is_empty() {
        let _ = write!(out, ", {} with `{filter}`", items.len());
    }
    out.push_str("\n\n");
    for i in items {
        out.push_str(&doc_render::item_text(
            &doc_render::Item {
                sig: &i.sig,
                doc: &i.doc,
            },
            width,
        ));
        out.push('\n');
    }
    out
}
