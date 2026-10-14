// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN183 P1 — the feature catalogue, inside the build.
//!
//! `index/features.json` is the generated mirror of the loft-lang/features issues (`make
//! features-fetch`): every language feature and infrastructure entry, its overview group, the
//! construct keys it documents, and its Markdown body.  Embedded here, the REPL and the
//! language server answer from it with no network and no checkout of this repository — the
//! catalogue ships with the loft that implements it, as the stdlib does.

use crate::json::Parsed;
use std::sync::OnceLock;

/// One catalogue entry.
pub struct Entry {
    /// `F2`, `I57` — the tag without its `@`.
    pub tag: String,
    pub number: i64,
    pub title: String,
    /// `feature` or `infra`.
    pub kind: String,
    /// The overview group: one key of [`GROUPS`].
    pub group: String,
    /// The construct names this entry documents — one owner per key (`make features-check`).
    pub keys: Vec<String>,
    /// The issue body, Markdown.
    pub body: String,
}

/// The overview's groups, in the order the overview shows them: `(key, title)`.  A group's key
/// is the `group:<key>` label its entries carry on the catalogue issues.
pub const GROUPS: &[(&str, &str)] = &[
    ("values", "Values and types"),
    ("absence", "Absence and arithmetic safety"),
    ("operators", "Operators"),
    ("collections", "Collections"),
    ("control", "Control flow and patterns"),
    ("functions", "Functions, methods and generics"),
    ("text", "Text"),
    ("data", "Files, stores and data"),
    ("concurrency", "Concurrency"),
    ("services", "Standard services"),
    ("modules", "Modules, libraries and packages"),
    ("tools", "Tools: run, test, debug, build"),
    ("inside", "Inside loft (maintainers)"),
];

/// The group a reader's overview does not open by default: how loft itself is built.
pub const MAINTAINERS_GROUP: &str = "inside";

const SOURCE: &str = include_str!("../index/features.json");

/// Every entry, in issue order — parsed once, on first use.
#[must_use]
pub fn entries() -> &'static [Entry] {
    static ENTRIES: OnceLock<Vec<Entry>> = OnceLock::new();
    ENTRIES.get_or_init(|| parse(SOURCE))
}

fn field<'a>(v: &'a Parsed, key: &str) -> Option<&'a Parsed> {
    match v {
        Parsed::Object(e) => e.iter().find(|(k, _, _)| k == key).map(|(_, _, val)| val),
        _ => None,
    }
}

fn text(v: &Parsed, key: &str) -> String {
    match field(v, key) {
        Some(Parsed::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

/// The entries of a `features.json` text; an unreadable one yields none rather than a panic,
/// so a broken mirror shows as an empty overview, which the catalogue test fails on.
#[must_use]
pub fn parse(source: &str) -> Vec<Entry> {
    let Ok(Parsed::Array(items)) = crate::json::parse(source) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|it| {
            let number = field(it, "number").and_then(Parsed::as_i64).unwrap_or(0);
            let kind = text(it, "kind");
            let keys = match field(it, "keys") {
                Some(Parsed::Array(k)) => k
                    .iter()
                    .filter_map(|x| match x {
                        Parsed::Str(s) => Some(s.clone()),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            Entry {
                tag: format!("{}{number}", if kind == "feature" { "F" } else { "I" }),
                number,
                title: text(it, "title"),
                kind,
                group: text(it, "group"),
                keys,
                body: text(it, "body"),
            }
        })
        .collect()
}

/// The entry with this tag (`F2`, `@F2`).
#[must_use]
pub fn by_tag(tag: &str) -> Option<&'static Entry> {
    let tag = tag.strip_prefix('@').unwrap_or(tag);
    entries().iter().find(|e| e.tag == tag)
}

/// The one entry that documents construct `key` (`op:??`, `kw:match`).
#[must_use]
pub fn by_key(key: &str) -> Option<&'static Entry> {
    entries().iter().find(|e| e.keys.iter().any(|k| k == key))
}

/// The entries of one group, in issue order.
pub fn in_group(group: &str) -> impl Iterator<Item = &'static Entry> {
    let group = group.to_string();
    entries().iter().filter(move |e| e.group == group)
}

impl Entry {
    /// The body's blocks.
    #[must_use]
    pub fn blocks(&self) -> Vec<crate::doc_render::Block> {
        crate::doc_render::blocks(&self.body)
    }

    /// The one-paragraph summary: the first paragraph under `What it is` (a feature) or
    /// `What it does` (an infrastructure entry), or else the body's first paragraph.
    #[must_use]
    pub fn summary(&self) -> String {
        use crate::doc_render::Block;
        let blocks = self.blocks();
        let mut after_heading = false;
        for b in &blocks {
            match b {
                Block::Heading(_, t) => after_heading = t == "What it is" || t == "What it does",
                Block::Para(p) if after_heading => return p.clone(),
                _ => {}
            }
        }
        blocks
            .iter()
            .find_map(|b| match b {
                Block::Para(p) => Some(p.clone()),
                _ => None,
            })
            .unwrap_or_default()
    }

    /// The runnable example: the first `loft` fence — the one `tools/features/gen.loft` turns
    /// into `tests/docs/features/<tag>.loft`, so it is known to run.
    #[must_use]
    pub fn example(&self) -> Option<String> {
        self.blocks().into_iter().find_map(|b| match b {
            crate::doc_render::Block::Code(lang, code) if lang == "loft" => Some(code),
            _ => None,
        })
    }

    /// Where the entry's full text lives: its issue, the catalogue's source of truth.
    #[must_use]
    pub fn page(&self) -> String {
        format!(
            "https://github.com/loft-lang/features/issues/{}",
            self.number
        )
    }
}
