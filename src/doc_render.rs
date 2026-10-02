// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN183 P1 — one renderer for documentation, three back-ends.
//!
//! The web pages, the REPL and the language server show the same facts: a documented item (a
//! signature and its doc comment) and a catalogue entry (a feature's Markdown body).  Each is
//! read ONCE into a small model here and written by three back-ends — plain text for the
//! terminal, Markdown for a hover or the IDE overview, HTML for `gendoc` — so the three
//! surfaces cannot drift apart.  The rules a reader can see (what a paragraph is, that a
//! worked-example citation is bookkeeping and not prose, that an unclosed backtick stays a
//! backtick) live here once.

use std::collections::HashMap;
use std::fmt::Write as _;

/// A documented item: its signature and its doc comment, as written.
pub struct Item<'a> {
    pub sig: &'a str,
    pub doc: &'a str,
}

/// One run of a paragraph: prose, or a `` `code` `` span.
#[derive(Debug, PartialEq, Eq)]
pub enum Span<'a> {
    Text(&'a str),
    Code(&'a str),
}

/// A doc comment as display paragraphs: worked-example citations dropped (they address the
/// maintainer of the examples, not the reader), blank-line separated, each paragraph's lines
/// joined with one space.
#[must_use]
pub fn paragraphs(doc: &str) -> Vec<String> {
    let lines: Vec<&str> = doc.lines().collect();
    let doc = crate::documentation::without_example_citations(&lines).join("\n");
    doc.split("\n\n")
        .map(|para| {
            para.lines()
                .map(str::trim_end)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|p| !p.is_empty())
        .collect()
}

/// A paragraph split into prose and `` `code` `` spans.  An unclosed backtick stays a literal
/// character in the prose rather than swallowing the rest of the paragraph: a doc comment is
/// prose someone typed, and the failure mode of guessing is that the sentence disappears.
#[must_use]
pub fn spans(text: &str) -> Vec<Span<'_>> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let (before, after) = rest.split_at(open);
        match after[1..].find('`') {
            Some(close) => {
                if !before.is_empty() {
                    out.push(Span::Text(before));
                }
                out.push(Span::Code(&after[1..=close]));
                rest = &after[close + 2..];
            }
            None => break,
        }
    }
    if !rest.is_empty() {
        out.push(Span::Text(rest));
    }
    out
}

/// Escape text for HTML content and attribute values.
#[must_use]
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// One paragraph as HTML: escaped prose, `` `spans` `` as `<code>`.
#[must_use]
pub fn inline_html(text: &str) -> String {
    let mut out = String::new();
    for span in spans(text) {
        match span {
            Span::Text(t) => out.push_str(&esc(t)),
            Span::Code(c) => {
                let _ = write!(out, "<code>{}</code>", esc(c));
            }
        }
    }
    out
}

/// A doc comment as HTML paragraphs, one `<p>` per line of output.
#[must_use]
pub fn paragraphs_html(doc: &str) -> String {
    let mut out = String::new();
    for p in paragraphs(doc) {
        let _ = writeln!(out, "<p>{}</p>", inline_html(&p));
    }
    out
}

/// An item as HTML: the highlighted signature, its links resolved through `link_map`, then
/// its paragraphs — the `<div class="item">` block every reference page uses.
#[must_use]
pub fn item_html<S: std::hash::BuildHasher>(
    item: &Item,
    link_map: &HashMap<String, String, S>,
) -> String {
    let mut out = String::from("<div class=\"item\">\n");
    let _ = writeln!(
        out,
        "<pre><code>{}</code></pre>",
        crate::documentation::highlight_loft(item.sig, link_map)
    );
    out.push_str(&paragraphs_html(item.doc));
    out.push_str("</div>\n");
    out
}

/// An item as Markdown: the signature in a `loft` fence, then its paragraphs.  Spans pass
/// through as Markdown's own `` `code` ``.
#[must_use]
pub fn item_markdown(item: &Item) -> String {
    let mut out = format!("```loft\n{}\n```\n", item.sig.trim_end());
    for p in paragraphs(item.doc) {
        let _ = write!(out, "\n{p}\n");
    }
    out
}

/// An item as terminal text: the signature, then each paragraph wrapped to `width` and indented
/// two columns.  Spans keep their backticks, which read as code in a terminal.
#[must_use]
pub fn item_text(item: &Item, width: usize) -> String {
    let mut out = format!("{}\n", item.sig.trim_end());
    for p in paragraphs(item.doc) {
        out.push_str(&wrap(&p, width.saturating_sub(2).max(20), "  "));
    }
    out
}

/// `text` wrapped at word boundaries to `width`, each line prefixed with `indent`.  A word
/// longer than the width stands on a line of its own rather than being split.
#[must_use]
pub fn wrap(text: &str, width: usize, indent: &str) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > width {
            let _ = writeln!(out, "{indent}{line}");
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        let _ = writeln!(out, "{indent}{line}");
    }
    out
}

/// One block of a catalogue entry's Markdown body.
#[derive(Debug, PartialEq, Eq)]
pub enum Block {
    /// `## Title` — the level is the count of `#`.
    Heading(usize, String),
    /// A prose paragraph, its lines joined with one space.
    Para(String),
    /// A fenced block: its language (`loft`, or empty) and its lines.
    Code(String, String),
    /// A `- ` list: its items, continuation lines joined.
    List(Vec<String>),
    /// Lines passed through as written — a table, a quote.
    Raw(String),
}

/// A catalogue body's blocks.  HTML comments (the `<!-- keys: … -->` line) are dropped: they
/// are data for the tools, not text for a reader.
#[must_use]
pub fn blocks(body: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut lines = body.lines().peekable();
    let mut para: Vec<&str> = Vec::new();
    let flush = |para: &mut Vec<&str>, out: &mut Vec<Block>| {
        if !para.is_empty() {
            out.push(Block::Para(para.join(" ")));
            para.clear();
        }
    };
    while let Some(line) = lines.next() {
        let t = line.trim_end();
        if t.trim().is_empty() {
            flush(&mut para, &mut out);
            continue;
        }
        if t.trim_start().starts_with("<!--") {
            flush(&mut para, &mut out);
            if !t.contains("-->") {
                for l in lines.by_ref() {
                    if l.contains("-->") {
                        break;
                    }
                }
            }
            continue;
        }
        if let Some(rest) = t.strip_prefix("```") {
            flush(&mut para, &mut out);
            let mut code = Vec::new();
            for l in lines.by_ref() {
                if l.trim_start().starts_with("```") {
                    break;
                }
                code.push(l);
            }
            out.push(Block::Code(rest.trim().to_string(), code.join("\n")));
            continue;
        }
        let hashes = t.chars().take_while(|c| *c == '#').count();
        if hashes > 0 && t[hashes..].starts_with(' ') {
            flush(&mut para, &mut out);
            out.push(Block::Heading(hashes, t[hashes..].trim().to_string()));
            continue;
        }
        if t.starts_with("- ") || t.starts_with("* ") {
            flush(&mut para, &mut out);
            let mut items = vec![t[2..].to_string()];
            while let Some(next) = lines.peek() {
                let n = next.trim_end();
                if n.starts_with("- ") || n.starts_with("* ") {
                    items.push(n[2..].to_string());
                } else if n.starts_with("  ")
                    && !n.trim().is_empty()
                    && let Some(last) = items.last_mut()
                {
                    last.push(' ');
                    last.push_str(n.trim());
                } else {
                    break;
                }
                lines.next();
            }
            out.push(Block::List(items));
            continue;
        }
        if t.starts_with('|') || t.starts_with('>') {
            flush(&mut para, &mut out);
            let mut raw = vec![t.to_string()];
            while let Some(next) = lines.peek() {
                let n = next.trim_end();
                if n.starts_with('|') || n.starts_with('>') {
                    raw.push(n.to_string());
                    lines.next();
                } else {
                    break;
                }
            }
            out.push(Block::Raw(raw.join("\n")));
            continue;
        }
        para.push(t.trim());
    }
    flush(&mut para, &mut out);
    out
}

/// Blocks as Markdown — the body as a hover or the IDE overview shows it.
#[must_use]
pub fn blocks_markdown(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        if !out.is_empty() {
            out.push('\n');
        }
        match b {
            Block::Heading(n, t) => {
                let _ = writeln!(out, "{} {t}", "#".repeat(*n));
            }
            Block::Para(p) => {
                let _ = writeln!(out, "{p}");
            }
            Block::Code(lang, code) => {
                let _ = writeln!(out, "```{lang}\n{code}\n```");
            }
            Block::List(items) => {
                for i in items {
                    let _ = writeln!(out, "- {i}");
                }
            }
            Block::Raw(r) => {
                let _ = writeln!(out, "{r}");
            }
        }
    }
    out
}

/// Blocks as terminal text: headings underlined, prose wrapped to `width`, code indented four
/// columns, list items bulleted.
#[must_use]
pub fn blocks_text(blocks: &[Block], width: usize) -> String {
    let mut out = String::new();
    for b in blocks {
        if !out.is_empty() {
            out.push('\n');
        }
        match b {
            Block::Heading(_, t) => {
                let _ = writeln!(out, "{t}\n{}", "-".repeat(t.chars().count()));
            }
            Block::Para(p) => out.push_str(&wrap(p, width, "")),
            Block::Code(_, code) => {
                for l in code.lines() {
                    let _ = writeln!(out, "    {l}");
                }
            }
            Block::List(items) => {
                for i in items {
                    let w = wrap(i, width.saturating_sub(2).max(20), "  ");
                    out.push_str("- ");
                    out.push_str(w.trim_start());
                }
            }
            Block::Raw(r) => {
                let _ = writeln!(out, "{r}");
            }
        }
    }
    out
}

/// Blocks as HTML.  Tables and quotes are passed through escaped inside a `<pre>`: the
/// catalogue bodies use them rarely, and an exact table renderer is not owed until a page
/// renders one.
#[must_use]
pub fn blocks_html<S: std::hash::BuildHasher>(
    blocks: &[Block],
    link_map: &HashMap<String, String, S>,
) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(n, t) => {
                let level = (*n).clamp(2, 6);
                let _ = writeln!(out, "<h{level}>{}</h{level}>", inline_html(t));
            }
            Block::Para(p) => {
                let _ = writeln!(out, "<p>{}</p>", inline_html(p));
            }
            Block::Code(lang, code) if lang == "loft" => {
                let _ = writeln!(
                    out,
                    "<pre><code>{}</code></pre>",
                    crate::documentation::highlight_loft(code, link_map)
                );
            }
            Block::Code(_, code) => {
                let _ = writeln!(out, "<pre><code>{}</code></pre>", esc(code));
            }
            Block::List(items) => {
                out.push_str("<ul>\n");
                for i in items {
                    let _ = writeln!(out, "<li>{}</li>", inline_html(i));
                }
                out.push_str("</ul>\n");
            }
            Block::Raw(r) => {
                let _ = writeln!(out, "<pre>{}</pre>", esc(r));
            }
        }
    }
    out
}
