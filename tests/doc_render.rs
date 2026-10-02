// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN183 P1 — the one documentation renderer and the embedded feature catalogue.
//!
//! The renderer's rules (what a paragraph is, which line is bookkeeping, what an unclosed
//! backtick means) are pinned here once for all three back-ends; the catalogue tests pin that
//! what the REPL and the language server will read is the catalogue the issues define — every
//! entry grouped, every key owned once, every example the one the example tests run.

use loft::doc_catalogue::{self, GROUPS, MAINTAINERS_GROUP};
use loft::doc_render::{self, Block, Item, Span};
use std::collections::HashMap;

#[test]
fn a_doc_comment_is_paragraphs_without_its_citations() {
    let doc = "Opens the file and\nanswers its text.\n\nExample: @FTR-007 — a citation\n\nNull when it is missing.";
    assert_eq!(
        doc_render::paragraphs(doc),
        vec![
            "Opens the file and answers its text.",
            "Null when it is missing."
        ]
    );
}

#[test]
fn an_unclosed_backtick_stays_in_the_prose() {
    assert_eq!(
        doc_render::spans("calls `f(x)` then `g"),
        vec![
            Span::Text("calls "),
            Span::Code("f(x)"),
            Span::Text(" then `g")
        ]
    );
    assert_eq!(
        doc_render::inline_html("a < `b<c>`"),
        "a &lt; <code>b&lt;c&gt;</code>"
    );
}

#[test]
fn an_item_renders_the_same_facts_in_all_three_back_ends() {
    let item = Item {
        sig: "pub fn first(v: vector<integer>) -> integer?",
        doc: "The first element of `v`,\nor null when `v` is empty.",
    };
    let html = doc_render::item_html(&item, &HashMap::<String, String>::new());
    assert!(
        html.starts_with("<div class=\"item\">\n<pre><code>"),
        "{html}"
    );
    assert!(
        html.ends_with("<p>The first element of <code>v</code>, or null when <code>v</code> is empty.</p>\n</div>\n"),
        "{html}"
    );
    assert_eq!(
        doc_render::item_markdown(&item),
        "```loft\npub fn first(v: vector<integer>) -> integer?\n```\n\nThe first element of `v`, or null when `v` is empty.\n"
    );
    assert_eq!(
        doc_render::item_text(&item, 40),
        "pub fn first(v: vector<integer>) -> integer?\n  The first element of `v`, or null when\n  `v` is empty.\n"
    );
}

#[test]
fn a_body_is_blocks_and_its_keys_comment_is_not_text() {
    let body = "## What it is\n\nA `??` picks\nthe first present value.\n\n```loft\nx = a ?? 0;\n```\n\n- one\n- two\n  continued\n\n<!-- keys: op:?? -->\n";
    assert_eq!(
        doc_render::blocks(body),
        vec![
            Block::Heading(2, "What it is".into()),
            Block::Para("A `??` picks the first present value.".into()),
            Block::Code("loft".into(), "x = a ?? 0;".into()),
            Block::List(vec!["one".into(), "two continued".into()]),
        ]
    );
}

#[test]
fn the_embedded_catalogue_is_the_committed_mirror() {
    let mirror = std::fs::read_to_string("index/features.json").expect("index/features.json");
    let parsed = doc_catalogue::parse(&mirror);
    let embedded = doc_catalogue::entries();
    assert!(
        !embedded.is_empty(),
        "the embedded catalogue parsed to nothing"
    );
    assert_eq!(
        embedded
            .iter()
            .map(|e| (&e.tag, &e.title, &e.group, &e.keys, &e.body))
            .collect::<Vec<_>>(),
        parsed
            .iter()
            .map(|e| (&e.tag, &e.title, &e.group, &e.keys, &e.body))
            .collect::<Vec<_>>(),
        "the build embeds a different catalogue than index/features.json — rebuild"
    );
}

#[test]
fn every_entry_is_grouped_and_every_reader_entry_says_what_it_is() {
    let known: Vec<&str> = GROUPS.iter().map(|(k, _)| *k).collect();
    let mut bad = Vec::new();
    for e in doc_catalogue::entries() {
        if !known.contains(&e.group.as_str()) {
            bad.push(format!(
                "{}: group `{}` is not one of the overview's",
                e.tag, e.group
            ));
        }
        if e.group != MAINTAINERS_GROUP && e.keys.is_empty() {
            bad.push(format!(
                "{}: no keys, so nothing a reader points at reaches it",
                e.tag
            ));
        }
        if e.summary().is_empty() {
            bad.push(format!("{}: no summary paragraph", e.tag));
        }
    }
    let mut owner: HashMap<&str, &str> = HashMap::new();
    for e in doc_catalogue::entries() {
        for k in &e.keys {
            if let Some(prev) = owner.insert(k, &e.tag) {
                bad.push(format!("key `{k}` owned by {prev} and {}", e.tag));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert_eq!(
        doc_catalogue::by_key("op:??").map(|e| e.tag.as_str()),
        Some("F2")
    );
    assert_eq!(
        doc_catalogue::by_tag("@F124").map(|e| e.group.as_str()),
        Some("operators")
    );
}

/// The example the REPL will run is the one the example tests already run: the first `loft`
/// fence, byte for byte the program `tools/features/gen.loft` wrote to
/// `tests/docs/features/<tag>.loft`.
#[test]
fn an_entrys_example_is_the_program_its_example_test_runs() {
    let mut checked = 0;
    let mut bad = Vec::new();
    for e in doc_catalogue::entries() {
        let path = format!("tests/docs/features/{}.loft", e.tag);
        let Ok(file) = std::fs::read_to_string(&path) else {
            continue;
        };
        // The generator writes a three-line header, then the fence's text and a newline.
        let program: String = file.lines().skip(3).map(|l| format!("{l}\n")).collect();
        match e.example() {
            Some(ex) if format!("{}\n", ex.trim_end()) == program => checked += 1,
            Some(_) => bad.push(format!("{}: example differs from {path}", e.tag)),
            None => bad.push(format!(
                "{}: {path} exists but the entry has no loft fence",
                e.tag
            )),
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert!(
        checked > 70,
        "only {checked} examples compared — is tests/docs/features/ there?"
    );
}

/// The visible text of `html`: tags removed, the four entities read back, whitespace collapsed.
fn visible(html: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    collapse(
        &out.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&amp;", "&"),
    )
}

fn collapse(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The HTML and the terminal text show a reader the SAME words: markup may change (a span
/// becomes `<code>`, a signature is highlighted and linked) and the text may not.  Measured
/// over every `///`-documented declaration of the stdlib — the corpus the reference pages
/// render — with the backticks the terminal keeps discounted.  A renderer change that alters
/// what one surface says, and not the other, goes red here.
#[test]
fn every_back_end_shows_the_same_words() {
    let mut items: Vec<(String, String)> = Vec::new();
    let mut files: Vec<_> = std::fs::read_dir("default")
        .expect("default/")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "loft"))
        .collect();
    files.sort();
    for f in files {
        let src = std::fs::read_to_string(&f).expect("a stdlib file");
        let mut doc: Vec<&str> = Vec::new();
        for line in src.lines() {
            if let Some(d) = line.trim_start().strip_prefix("///") {
                doc.push(d.strip_prefix(' ').unwrap_or(d));
            } else {
                if !doc.is_empty() && line.starts_with("pub ") {
                    items.push((
                        line.trim_end().trim_end_matches('{').trim_end().to_string(),
                        doc.join("\n"),
                    ));
                }
                doc.clear();
            }
        }
    }
    assert!(
        items.len() > 200,
        "only {} documented items found in default/ (255 when written)",
        items.len()
    );
    let mut bad = Vec::new();
    for (sig, doc) in &items {
        let item = Item { sig, doc };
        let html = visible(&doc_render::item_html(
            &item,
            &HashMap::<String, String>::new(),
        ));
        let text = collapse(&doc_render::item_text(&item, 10_000).replace('`', ""));
        if html.replace('`', "") != text {
            bad.push(format!("{sig}\n  html: {html}\n  text: {text}"));
        }
    }
    assert!(
        bad.is_empty(),
        "{} item(s) differ:\n{}",
        bad.len(),
        bad.join("\n")
    );
}
