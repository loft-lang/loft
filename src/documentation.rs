// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I79 — Documentation generator

use std::collections::HashMap;
use std::fmt::Write;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;

/// A stdlib section visible in the nav and the search index.
pub struct StdlibSection {
    pub id: String,          // URL-safe slug, e.g. "output-and-diagnostics"
    pub name: String,        // Human-readable label, e.g. "Output and diagnostics"
    pub description: String, // One-line description shown on the index page card
}

struct Topic {
    file: PathBuf,
    filename: String, // Stem without extension, e.g. "05-float"
    name: String,     // Short display name from @NAME header
    title: String,    // Descriptive title from @TITLE header
}

#[must_use]
fn gather_topics() -> Vec<Topic> {
    let mut result: Vec<Topic> = Vec::new();
    // Sorted by name: every entry shares the directory.
    let entries = crate::file_access::read_dir("tests/docs").unwrap();
    entries
        .iter()
        .map(crate::file_access::PathText::os_spelling)
        .filter(|p| {
            crate::file_access::extension(p).is_some_and(|e| e.eq_ignore_ascii_case("loft"))
        })
        .for_each(|file| {
            let file_handle = crate::file_access::open(&file).expect("failed to open file");
            let reader = BufReader::new(file_handle);
            let filename = crate::file_access::file_stem(&file).unwrap();
            let mut name = String::new();
            let mut title = String::new();
            reader.lines().for_each(|line_result| {
                if let Ok(line) = line_result {
                    if let Some(s) = line.strip_prefix("// @NAME: ") {
                        name = s.to_string();
                    }
                    if let Some(s) = line.strip_prefix("// @TITLE: ") {
                        title = s.to_string();
                    }
                }
            });
            result.push(Topic {
                file,
                filename,
                name,
                title,
            });
        });
    result
}

/// Call this after building the stdlib sections and link map so that language
/// pages are generated with stdlib links already inlined and their nav matches
/// the stdlib pages generated separately.
/// # Errors
/// When the `doc/` directory is unwritable.
pub fn generate_docs<S: std::hash::BuildHasher>(
    stdlib_sections: &[StdlibSection],
    link_map: &HashMap<String, String, S>,
    version: &str,
) -> std::io::Result<()> {
    let topics = gather_topics();
    write_index(&topics, stdlib_sections, version)?;
    let nav_info = topic_nav_info(&topics);
    for entry in &topics {
        if entry.filename.starts_with("00-") {
            continue;
        }
        let stem = crate::file_access::file_stem(&entry.file).unwrap_or_default();
        if let Ok(source) = crate::file_access::read_to_string(&entry.file) {
            let html = render_doc_page(
                &source,
                &entry.name,
                &entry.title,
                &stem,
                &nav_info,
                stdlib_sections,
                link_map,
            );
            crate::file_access::write(format!("doc/{stem}.html"), html)?;
        }
    }
    Ok(())
}

fn flush_intro_para(result: &mut String, para_buf: &mut String) {
    if !para_buf.is_empty() {
        writeln!(result, "<p>{}</p>", para_buf.trim()).expect("");
        para_buf.clear();
    }
}

fn index_intro(topic: &Topic) -> std::io::Result<String> {
    let mut result = String::new();
    let file = crate::file_access::open(&topic.file).expect("failed to open file");
    let source = BufReader::new(file);
    let mut in_header = true;
    let mut in_list = false;
    let mut para_buf = String::new();
    for line_result in source.lines() {
        let line = line_result?;
        let trimmed = line.trim();
        if is_topic_directive(trimmed) || (in_header && skip_header(trimmed)) {
            continue;
        }
        in_header = false;
        if !trimmed.starts_with("//") {
            // Blank line or non-comment = paragraph / list break
            if in_list {
                writeln!(result, "</ul>").expect("");
                in_list = false;
            } else {
                flush_intro_para(&mut result, &mut para_buf);
            }
            continue;
        }
        let text = trimmed.strip_prefix("//").unwrap_or("").trim().to_string();
        // Stop at the first section heading — the index page shows only the
        // brief introductory paragraphs, not the full topic content.
        if text.starts_with("## ") || text.starts_with("### ") {
            break;
        }
        if let Some(n) = text.strip_prefix("- ") {
            flush_intro_para(&mut result, &mut para_buf);
            if !in_list {
                write!(result, "<ul>").expect("");
                in_list = true;
            }
            writeln!(result, "<li>{n}</li>").expect("");
        } else if text.is_empty() {
            // `//` alone on a line also breaks the paragraph
            if in_list {
                writeln!(result, "</ul>").expect("");
                in_list = false;
            } else {
                flush_intro_para(&mut result, &mut para_buf);
            }
        } else {
            if in_list {
                writeln!(result, "</ul>").expect("");
                in_list = false;
            }
            if !para_buf.is_empty() {
                para_buf.push(' ');
            }
            para_buf.push_str(&text);
        }
    }
    if in_list {
        writeln!(result, "</ul>").expect("");
    } else {
        flush_intro_para(&mut result, &mut para_buf);
    }
    Ok(result)
}

/// Is this line a topic DIRECTIVE — one `gather_topics` consumes to name the page?
///
/// It scans the whole file for these, not just the header, so a directive is metadata
/// wherever it sits. Rendering one as prose would print the page's own title into its
/// body, so both renderers drop it regardless of position.
fn is_topic_directive(trimmed: &str) -> bool {
    trimmed.starts_with("// @NAME: ") || trimmed.starts_with("// @TITLE: ")
}

/// Does `s` open with a worked-example tag — three uppercase letters, a hyphen, three
/// digits (`STD-001`, `LEX-002`)? `s` is the text FOLLOWING an `@`.
///
/// The shape is chosen so it cannot collide with loft's tracker families: `@P259`,
/// `@PLN3` and `@F7` all put a digit or a differently-sized letter run where this wants
/// exactly three letters then a hyphen.
///
/// One home for the shape, because two readers must agree on it: the example index
/// FINDS a tag, and the topic renderer must NOT print one. A renderer that disagreed
/// about what a tag looks like would publish the ones it failed to recognise.
#[must_use]
pub fn is_example_tag(s: &str) -> bool {
    let w: Vec<char> = s.chars().take(7).collect();
    w.len() == 7
        && w[0..3].iter().all(char::is_ascii_uppercase)
        && w[3] == '-'
        && w[4..7].iter().all(char::is_ascii_digit)
}

/// Does this line OPEN a worked-example tag definition (`// @AAA-### — what it shows`)?
///
/// A tag definition marks the call site that teaches a function (@PLN141). It is
/// bookkeeping for `check_doc_drift.sh examples`, not prose a reader of the page asked
/// for, so the topic renderers drop it along with the rest of its comment block.
fn opens_example_tag(trimmed: &str) -> bool {
    trimmed
        .strip_prefix("//")
        .map(str::trim_start)
        .and_then(|t| t.strip_prefix('@'))
        .is_some_and(is_example_tag)
}

/// Is this comment line a feature-catalogue ANCHOR (`@F40 — file & directory I/O
/// (catalogue anchor, @PLN92)`)?
///
/// An anchor ties a stdlib file to its `@F` entry in the feature catalogue.
/// `scripts/feature_hygiene.sh` reads it out of `default/` as the CODE anchor — the
/// answer to *where does this feature live* — so it has to stay in the source. It is
/// bookkeeping for that script and for nothing else, so it must not reach the page: an
/// anchor sitting above a `pub` item became that item's published description, and
/// `store_bind_lazy` opened with `@F108 — Lazy store binding (catalogue anchor, @PLN92)`.
///
/// Keyed on BOTH halves — an opening `@F<n>` and the `catalogue anchor` marker — so a
/// sentence that merely mentions a feature id stays prose.
#[must_use]
pub fn is_catalogue_anchor(trimmed: &str) -> bool {
    let Some(text) = trimmed.strip_prefix("//").map(str::trim_start) else {
        return false;
    };
    let opens_with_feature = text
        .strip_prefix("@F")
        .is_some_and(|r| r.starts_with(|c: char| c.is_ascii_digit()));
    opens_with_feature && text.contains("catalogue anchor")
}

/// Does this doc line OPEN a worked-example citation (`Example: @AAA-### — what it shows`)?
///
/// A citation points a maintainer at the call site that teaches this item; it is
/// bookkeeping for `check_doc_drift.sh examples`, the sibling of the `@AAA-###`
/// DEFINITION [`opens_example_tag`] drops. The reader of a published page never asked
/// for it, and CLAUDE.md § User-facing output keeps tracker tags out of what a command
/// prints — a rendered citation is a bare tag with no link and no explanation.
///
/// Accepts the line with or without its `//`, because the two shapes are both real: the
/// stdlib and package extractors hand on comment text already stripped, while a caller
/// reading source lines has not stripped it. A predicate that silently matched only one
/// of them is how the second renderer would start publishing what the first hides.
#[must_use]
pub fn opens_example_citation(line: &str) -> bool {
    let text = line.trim_start();
    let text = text.strip_prefix("//").map_or(text, str::trim_start);
    text.strip_prefix("Example:")
        .map(str::trim_start)
        .and_then(|t| t.strip_prefix('@'))
        .is_some_and(is_example_tag)
}

/// A doc comment's lines with every worked-example citation removed.
///
/// One home for "a citation is not prose", because the renderers that must agree on it
/// are three pages apart: the stdlib sections, the print sheet and the PDF all reach it
/// through `group_paragraphs`, and the library API pages through `doc_paragraphs`. A
/// renderer that disagreed would publish the citations the others hide.
///
/// A citation is a BLOCK, not a line: 51 of the 377 in the shipped distribution wrap
/// onto continuation lines, so dropping the opener alone would leave the tail behind as
/// a sentence fragment — a worse page than the one this fixes. The block runs from its
/// opener to the next blank line, the next citation, or the end of the comment.
///
/// The terminating blank is KEPT: it separates the citation's paragraph from whatever
/// follows, and swallowing it would weld two paragraphs of real prose together.
///
/// Only a line that OPENS with the citation is bookkeeping. A tag written INSIDE a
/// sentence is the author's prose and stays — `graphics::rgba` explains a hex literal
/// "(a hand-written hex literal — @GFX-001)", and `hex_shape` cites `(@HXS-002)` in the
/// middle of a measurement. A predicate that matched any tag would cut both sentences in
/// half.
#[must_use]
pub fn without_example_citations<S: AsRef<str>>(lines: &[S]) -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut in_citation = false;
    for line in lines {
        let text = line.as_ref();
        if opens_example_citation(text) {
            in_citation = true;
            continue;
        }
        if in_citation {
            if !text.trim().is_empty() {
                continue;
            }
            in_citation = false;
        }
        out.push(text.to_string());
    }
    out
}

/// Is this line part of the header block a topic opens with, rather than its prose?
///
/// The block is attribution, provenance and directives. Provenance matters because a
/// generated topic opens with a "DO NOT EDIT" note addressed to whoever maintains the
/// generator: the reader of the published page is not that person, and the note would
/// otherwise be the first paragraph they meet.
fn skip_header(trimmed: &str) -> bool {
    trimmed.starts_with("// Copyright")
        || trimmed.starts_with("// SPDX")
        || trimmed.starts_with("// GENERATED")
        || is_topic_directive(trimmed)
        || trimmed.is_empty()
}

#[expect(clippy::too_many_lines, reason = "inherited")]
fn write_index(
    topics: &[Topic],
    stdlib_sections: &[StdlibSection],
    version: &str,
) -> std::io::Result<()> {
    let mut lang_cards = String::from(
        "      <a class=\"card card-featured\" href=\"00-vs-rust.html\">\
<h2>vs Rust</h2><p>Key differences for developers coming from Rust.</p></a>\n\
      <a class=\"card card-featured\" href=\"00-vs-python.html\">\
<h2>vs Python</h2><p>Key differences for developers coming from Python.</p></a>\n\
      <a class=\"card card-featured\" href=\"00-performance.html\">\
<h2>Performance</h2><p>Benchmark results across interpreter, native, wasm, and Rust.</p></a>\n",
    );
    for topic in topics {
        if !topic.filename.starts_with("00-") {
            let _ = writeln!(
                lang_cards,
                "      <a class=\"card\" href=\"{}.html\"><h2>{}</h2><p>{}</p></a>",
                topic.filename, topic.name, topic.title
            );
        }
    }
    let lib_cards: String = stdlib_sections
        .iter()
        .fold(String::new(), |mut output, sec| {
            if sec.description.is_empty() {
                let _ = writeln!(
                    output,
                    "      <a class=\"card\" href=\"stdlib-{}.html\"><h2>{}</h2></a>",
                    sec.id, sec.name
                );
            } else {
                let _ = writeln!(
                    output,
                    "      <a class=\"card\" href=\"stdlib-{}.html\"><h2>{}</h2><p>{}</p></a>",
                    sec.id, sec.name, sec.description
                );
            }
            output
        });
    let start_cards = concat!(
        "      <a class=\"card card-featured\" href=\"install.html\">",
        "<h2>Install</h2>",
        "<p>Get loft running and write your first Loft program in minutes.</p></a>\n",
        "      <a class=\"card\" href=\"roadmap.html\">",
        "<h2>Roadmap</h2>",
        "<p>Planned features for version 1.0 and beyond, with syntax previews.</p></a>\n",
        "      <a class=\"card card-featured\" href=\"libraries.html\">",
        "<h2>Libraries</h2>",
        "<p>Every package in the registry \u{2014} graphics and 3D, an HTTP server and client, ",
        "cryptography, text engines, geometry. Each written in loft.</p></a>\n",
    );
    let title = topics[0].title.clone();
    let intro = index_intro(&topics[0])?;
    // T2.1 — the landing page is the one a search result and a shared link show
    // most, and it had no description or card tags at all.  Its title is the
    // tagline itself (not "<page> — <tagline>"), so the meta block is built
    // directly rather than through `head_meta`'s page-title shape.
    let index_desc = "Loft is a statically typed language for small browser-playable games: write it, \
         share a link, anyone plays. Four execution modes, records with indexes in the \
         language, and a complete arcade game in one readable file.";
    // schema.org JSON-LD (#635-adjacent SEO): a `WebSite` + a free
    // `SoftwareApplication`, so a search engine can render a richer result card
    // and recognise loft as a downloadable developer tool.  `index_desc` carries
    // no quotes/braces, so it embeds in the JSON string as-is.  Version rides the
    // same source as the hero, so it never goes stale.
    let index_jsonld = format!(
        "  <script type=\"application/ld+json\">\n\
{{\"@context\":\"https://schema.org\",\"@graph\":[\
{{\"@type\":\"WebSite\",\"name\":\"Loft\",\"url\":\"{SITE_BASE}\",\"description\":\"{index_desc}\"}},\
{{\"@type\":\"SoftwareApplication\",\"name\":\"Loft\",\"applicationCategory\":\"DeveloperApplication\",\
\"operatingSystem\":\"Linux, macOS, Windows, WebAssembly\",\"url\":\"{SITE_BASE}\",\
\"downloadUrl\":\"{SITE_BASE}install.html\",\"softwareVersion\":\"{version}\",\
\"image\":\"{SITE_OG_IMAGE}\",\"description\":\"{index_desc}\",\
\"offers\":{{\"@type\":\"Offer\",\"price\":\"0\",\"priceCurrency\":\"USD\"}},\
\"author\":{{\"@type\":\"Organization\",\"name\":\"loft-lang\",\"url\":\"https://github.com/loft-lang\"}}}}\
]}}\n\
  </script>\n"
    );
    let index_meta = format!(
        "  <meta name=\"description\" content=\"{index_desc}\">\n\
  <link rel=\"canonical\" href=\"{SITE_BASE}\">\n\
  <meta property=\"og:title\" content=\"{SITE_TITLE_INDEX}\">\n\
  <meta property=\"og:description\" content=\"{index_desc}\">\n\
  <meta property=\"og:type\" content=\"website\">\n\
  <meta property=\"og:url\" content=\"{SITE_BASE}\">\n\
  <meta property=\"og:image\" content=\"{SITE_OG_IMAGE}\">\n\
  <meta property=\"og:site_name\" content=\"Loft\">\n\
  <meta name=\"twitter:card\" content=\"summary_large_image\">\n\
  <meta name=\"twitter:title\" content=\"{SITE_TITLE_INDEX}\">\n\
  <meta name=\"twitter:description\" content=\"{index_desc}\">\n\
  <meta name=\"twitter:image\" content=\"{SITE_OG_IMAGE}\">\n\
{index_jsonld}"
    );
    let html = format!(
        "<!DOCTYPE html>\n\
<html lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\">\n\
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
  <title>{SITE_TITLE_INDEX}</title>\n\
{index_meta}\
  <link rel=\"stylesheet\" href=\"style.css\">\n\
</head>\n\
<body>\n\
  <header class=\"index-header\">\n\
    <div class=\"index-hero\">\n\
      <h1>Loft</h1>\n\
      <p class=\"tagline\">Build small games and interactive things \u{2014} share a link, anyone plays.</p>\n\
      <p class=\"subtagline\">{title}</p>\n\
      <p class=\"version\">v{version}</p>\n\
      <div class=\"hero-ctas\">\n\
        <a class=\"hero-btn hero-btn-primary\" href=\"playground.html\">Try it in the browser</a>\n\
        <a class=\"hero-btn\" href=\"gallery.html\">See the gallery</a>\n\
        <a class=\"hero-btn\" href=\"install.html\">Install</a>\n\
      </div>\n\
    </div>\n\
    <div class=\"search-wrap index-search\">\n\
      <input id=\"search\" type=\"search\" placeholder=\"Search docs\u{2026}\" autocomplete=\"off\">\n\
      <div class=\"search-results\" id=\"search-results\" hidden></div>\n\
    </div>\n\
  </header>\n\
  <section class=\"showcase\">\n\
    <a class=\"showcase-tile showcase-hero\" href=\"brick-buster.html\">\n\
      <img src=\"images/hero-brick-buster.png\" alt=\"Brick Buster \u{2014} a complete loft game\" loading=\"lazy\">\n\
      <div class=\"showcase-caption\">\n\
        <span class=\"showcase-tag\">Built with loft</span>\n\
        <h2>Brick Buster</h2>\n\
        <p>A complete arcade game \u{2014} hand-designed levels, cel-shaded sprites, heart lives, round ball with a velocity-directional squash, rising balloon bombs, fireball after-images, chiptune music. Written in loft, runs in your browser.</p>\n\
      </div>\n\
    </a>\n\
    <div class=\"showcase-side\">\n\
      <a class=\"showcase-tile showcase-sub\" href=\"playground.html\">\n\
        <div class=\"showcase-caption\">\n\
          <span class=\"showcase-tag\">No install</span>\n\
          <h3>Live playground</h3>\n\
          <p>Type a few lines of loft code. Press run. See output. That is the whole tutorial.</p>\n\
        </div>\n\
      </a>\n\
      <a class=\"showcase-tile showcase-sub\" href=\"gallery.html\">\n\
        <div class=\"showcase-caption\">\n\
          <span class=\"showcase-tag\">WebGL</span>\n\
          <h3>Graphics gallery</h3>\n\
          <p>Demos written in loft, running live in the browser \u{2014} including Brick Buster, a complete arcade game.</p>\n\
        </div>\n\
      </a>\n\
    </div>\n\
  </section>\n\
  <section class=\"intro\">\n\
{intro}\
  </section>\n\
  <section class=\"topics\">\n\
    <h2 class=\"topics-heading\">Getting Started</h2>\n\
    <div class=\"grid\">\n\
{start_cards}\
    </div>\n\
  </section>\n\
  <section class=\"topics\">\n\
    <h2 class=\"topics-heading\">Language</h2>\n\
    <div class=\"grid\">\n\
{lang_cards}\
    </div>\n\
  </section>\n\
  <section class=\"topics\">\n\
    <h2 class=\"topics-heading\">Standard Library</h2>\n\
    <div class=\"grid\">\n\
{lib_cards}\
    </div>\n\
  </section>\n\
  <script src=\"search-index.js\"></script>\n\
  <script src=\"search.js\"></script>\n\
</body>\n\
</html>\n"
    );
    crate::file_access::write("doc/index.html", html)
}

fn html_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

fn to_anchor_id(text: &str) -> String {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// The byte offset — relative to `i + 1` — of the delimiter closing the `'…'` or
/// `` `…` `` span that `c` opens at `i`, or `None` when this delimiter opens no span.
///
/// A backtick means one thing, so the next backtick closes it.  An apostrophe means three:
/// a code delimiter, a possessive and a contraction.  Reading every apostrophe as a
/// delimiter let the possessive in *"a product's name"* open a span that the real opener of
/// `'product.price'` then closed, rendering three sentences of the Structs page as code.
///
/// So a quote opens a span only when it does not follow a word character, and closes one
/// only when it is not followed by one.  That leaves `'don't'` a single span, and leaves
/// `L'Ecuyer's` — where neither quote qualifies — as the prose it is.
fn close_span(s: &str, i: usize, c: char) -> Option<usize> {
    let rest = &s[i + 1..];
    if c == '`' {
        return rest.find(c).filter(|&e| e > 0);
    }
    if s[..i]
        .chars()
        .next_back()
        .is_some_and(char::is_alphanumeric)
    {
        return None;
    }
    let mut from = 0;
    while let Some(e) = rest[from..].find(c) {
        let end = from + e;
        if end > 0
            && !rest[end + 1..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric)
        {
            return Some(end);
        }
        from = end + 1;
    }
    None
}

/// Convert inline markdown in already-escaped HTML text:
/// `**bold**` → `<strong>bold</strong>`, `'code'` → `<code>code</code>`.
fn inline_format(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        if c == '*'
            && s[i + 1..].starts_with('*')
            && let Some(end) = s[i + 2..].find("**")
        {
            out.push_str("<strong>");
            out.push_str(&s[i + 2..i + 2 + end]);
            out.push_str("</strong>");
            // Skip past the closing **
            let skip_to = i + 2 + end + 2;
            while chars.peek().is_some_and(|(j, _)| *j < skip_to) {
                chars.next();
            }
        } else if (c == '\'' || c == '`')
            && let Some(end) = close_span(s, i, c)
        {
            out.push_str("<code>");
            out.push_str(&s[i + 1..i + 1 + end]);
            out.push_str("</code>");
            let skip_to = i + 1 + end + 1;
            while chars.peek().is_some_and(|(j, _)| *j < skip_to) {
                chars.next();
            }
        } else {
            out.push(c);
        }
    }
    out
}

fn flush_para(para: &mut Vec<String>, body: &mut String) {
    if !para.is_empty() {
        let text = para.join(" ");
        body.push_str("<p>");
        body.push_str(&inline_format(&html_esc(&text)));
        body.push_str("</p>\n");
        para.clear();
    }
}

fn flush_list(in_list: &mut bool, body: &mut String) {
    if *in_list {
        body.push_str("</ul>\n");
        *in_list = false;
    }
}

fn flush_indented(block: &mut Vec<String>, body: &mut String) {
    if block.is_empty() {
        return;
    }
    body.push_str("<pre><code>");
    for line in block.iter() {
        body.push_str(&html_esc(line));
        body.push('\n');
    }
    body.push_str("</code></pre>\n");
    block.clear();
}

fn flush_list_item(item: &mut Vec<String>, body: &mut String) {
    if item.is_empty() {
        return;
    }
    let text = item.join(" ");
    let _ = writeln!(body, "<li>{}</li>", inline_format(&html_esc(&text)));
    item.clear();
}

/// Render prose lines into HTML, supporting `## Heading`, `### Sub-heading`,
/// `- list item` (with 2-space continuation), indented code blocks (2+ leading
/// spaces outside lists), and regular paragraph text.
fn render_prose_lines(lines: &[String], body: &mut String) {
    let mut para: Vec<String> = Vec::new();
    let mut in_list = false;
    let mut list_item: Vec<String> = Vec::new();
    let mut indented: Vec<String> = Vec::new();
    for line in lines {
        if let Some(heading) = line.strip_prefix("## ") {
            flush_indented(&mut indented, body);
            flush_list_item(&mut list_item, body);
            flush_list(&mut in_list, body);
            flush_para(&mut para, body);
            let id = to_anchor_id(heading);
            let _ = writeln!(
                body,
                "<h2 id=\"{id}\">{}</h2>",
                inline_format(&html_esc(heading))
            );
        } else if let Some(heading) = line.strip_prefix("### ") {
            flush_indented(&mut indented, body);
            flush_list_item(&mut list_item, body);
            flush_list(&mut in_list, body);
            flush_para(&mut para, body);
            let id = to_anchor_id(heading);
            let _ = writeln!(
                body,
                "<h3 id=\"{id}\">{}</h3>",
                inline_format(&html_esc(heading))
            );
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            flush_indented(&mut indented, body);
            flush_list_item(&mut list_item, body);
            flush_para(&mut para, body);
            if !in_list {
                body.push_str("<ul>\n");
                in_list = true;
            }
            list_item.push(item.to_string());
        } else if let Some(rest) = line.strip_prefix("  ") {
            if in_list {
                // Continuation of the current list item.
                list_item.push(rest.trim_start().to_string());
            } else if !para.is_empty() && indented.is_empty() {
                // Continuation of the current paragraph.
                para.push(rest.trim_start().to_string());
            } else {
                // Indented code block (only after an empty line / heading).
                flush_para(&mut para, body);
                indented.push(rest.to_string());
            }
        } else if line.is_empty() {
            flush_indented(&mut indented, body);
            flush_list_item(&mut list_item, body);
            flush_list(&mut in_list, body);
            flush_para(&mut para, body);
        } else {
            flush_indented(&mut indented, body);
            flush_list_item(&mut list_item, body);
            flush_list(&mut in_list, body);
            para.push(line.clone());
        }
    }
    flush_indented(&mut indented, body);
    flush_list_item(&mut list_item, body);
    flush_list(&mut in_list, body);
    flush_para(&mut para, body);
}

/// Escape one run of plain text so Typst renders it literally.
///
/// Every character here opens something in Typst markup, and an unclosed one is a
/// COMPILE error rather than a rendering blemish: a bare `_` in `log_*` reads as an
/// emphasis delimiter that never closes, and the whole document fails to build.
///
/// The set must stay complete for that reason. It lives here as the ONE home — a second
/// copy in the generator drifted from this one by exactly the `_` line, which is how a
/// generated reference stopped compiling while every gate stayed green (nothing in CI runs
/// `typst`; `tests/typst_compiles.rs` now does when it is installed).
pub fn typst_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('#', "\\#")
        .replace('@', "\\@")
        .replace('$', "\\$")
        .replace('[', "\\[")
        .replace(']', "\\]")
        .replace('<', "\\<")
        .replace('>', "\\>")
        .replace('*', "\\*")
        .replace('_', "\\_")
}

fn code_to_typst(code: &str) -> String {
    // Use "rust" lang tag as approximation — Typst doesn't know "loft" natively
    format!("```rust\n{code}\n```\n\n")
}

fn flush_typst_indented(block: &mut Vec<String>, result: &mut String) {
    if block.is_empty() {
        return;
    }
    result.push_str("```\n");
    for line in block.iter() {
        result.push_str(line);
        result.push('\n');
    }
    result.push_str("```\n\n");
    block.clear();
}

fn prose_to_typst(lines: &[String]) -> String {
    let mut result = String::new();
    let mut para: Vec<String> = Vec::new();
    let mut in_list = false;
    let mut indented: Vec<String> = Vec::new();
    for line in lines {
        if let Some(heading) = line.strip_prefix("## ") {
            flush_typst_indented(&mut indented, &mut result);
            if in_list {
                result.push('\n');
                in_list = false;
            }
            if !para.is_empty() {
                result.push_str(&para.join(" "));
                result.push_str("\n\n");
                para.clear();
            }
            let _ = write!(result, "=== {}\n\n", typst_escape(heading));
        } else if let Some(heading) = line.strip_prefix("### ") {
            flush_typst_indented(&mut indented, &mut result);
            if in_list {
                result.push('\n');
                in_list = false;
            }
            if !para.is_empty() {
                result.push_str(&para.join(" "));
                result.push_str("\n\n");
                para.clear();
            }
            let _ = write!(result, "==== {}\n\n", typst_escape(heading));
        } else if let Some(item) = line.strip_prefix("- ").or_else(|| line.strip_prefix("* ")) {
            flush_typst_indented(&mut indented, &mut result);
            if !para.is_empty() {
                result.push_str(&para.join(" "));
                result.push_str("\n\n");
                para.clear();
            }
            in_list = true;
            let _ = writeln!(result, "- {}", typst_escape(item));
        } else if let Some(rest) = line.strip_prefix("  ") {
            if in_list {
                result.push('\n');
                in_list = false;
            }
            if !para.is_empty() {
                result.push_str(&para.join(" "));
                result.push_str("\n\n");
                para.clear();
            }
            indented.push(rest.to_string());
        } else if line.is_empty() {
            flush_typst_indented(&mut indented, &mut result);
            if in_list {
                result.push('\n');
                in_list = false;
            }
            if !para.is_empty() {
                result.push_str(&para.join(" "));
                result.push_str("\n\n");
                para.clear();
            }
        } else {
            flush_typst_indented(&mut indented, &mut result);
            if in_list {
                result.push('\n');
                in_list = false;
            }
            para.push(typst_escape(line));
        }
    }
    flush_typst_indented(&mut indented, &mut result);
    if in_list {
        result.push('\n');
    }
    if !para.is_empty() {
        result.push_str(&para.join(" "));
        result.push_str("\n\n");
    }
    result
}

// ─── Section parser ───────────────────────────────────────────────────────────

enum DocSection {
    /// Consecutive `//` comment lines with the `// ` prefix stripped.
    Prose(Vec<String>),
    /// Consecutive non-comment source lines kept verbatim.
    Code(Vec<String>),
}

fn parse_sections(source: &str) -> Vec<DocSection> {
    let mut sections: Vec<DocSection> = Vec::new();
    let mut prose: Vec<String> = Vec::new();
    let mut code: Vec<String> = Vec::new();
    let mut in_header = true;
    // Inside a worked-example tag definition's comment block, which is dropped whole.
    let mut in_example_tag = false;

    for line in source.lines() {
        let trimmed = line.trim();
        if is_topic_directive(trimmed) || (in_header && skip_header(trimmed)) {
            continue;
        }
        in_header = false;

        // Drop a worked-example tag definition and the rest of its comment block.  The
        // WHOLE block goes, not just the tagged line: the description wraps, so dropping
        // one line leaves its continuation as an orphan sentence that reads worse than
        // the tag did.  The block ends at the first non-comment line, which then falls
        // through to be handled normally.
        if in_example_tag {
            if trimmed.starts_with("//") {
                continue;
            }
            in_example_tag = false;
        }
        if opens_example_tag(trimmed) {
            in_example_tag = true;
            continue;
        }

        if trimmed.starts_with("//") {
            if !code.is_empty() {
                sections.push(DocSection::Code(std::mem::take(&mut code)));
            }
            let after_slashes = trimmed.strip_prefix("//").unwrap_or("");
            // Strip at most one leading space to preserve indentation.
            let text = after_slashes
                .strip_prefix(' ')
                .unwrap_or(after_slashes)
                .to_string();
            prose.push(text);
        } else if trimmed.is_empty() {
            if !prose.is_empty() {
                sections.push(DocSection::Prose(std::mem::take(&mut prose)));
            }
            if !code.is_empty() {
                sections.push(DocSection::Code(std::mem::take(&mut code)));
            }
        } else {
            if !prose.is_empty() {
                sections.push(DocSection::Prose(std::mem::take(&mut prose)));
            }
            code.push(line.to_string());
        }
    }
    if !prose.is_empty() {
        sections.push(DocSection::Prose(prose));
    }
    if !code.is_empty() {
        sections.push(DocSection::Code(code));
    }
    sections
}

// ─── Syntax highlighter ───────────────────────────────────────────────────────

const KW: &[&str] = &[
    "fn", "operator", "if", "else", "for", "in", "return", "break", "continue", "struct", "enum",
    "pub", "use", "type", "as", "not", "null", "true", "false", "and", "or", "limit", "default",
    "virtual",
];
const TY: &[&str] = &[
    "integer",
    "text",
    "boolean",
    "float",
    "single",
    "character",
    "vector",
    "sorted",
    "index",
    "hash",
    "reference",
    "u8",
    "u16",
    "u32",
    "i8",
    "i16",
    "i32",
    "i64",
];
const BI: &[&str] = &[
    "assert", "panic", "len", "round", "ceil", "floor", "abs", "sin", "cos", "log", "rev", "file",
    "min", "max", "sqrt", "typeof", "typedef",
];

fn scan_quoted(chars: &[char], i: usize, n: usize, delim: char) -> usize {
    let mut j = i + 1;
    while j < n && chars[j] != delim {
        j += 1;
    }
    if j < n { j + 1 } else { j }
}

fn scan_number(chars: &[char], i: usize, n: usize) -> usize {
    let mut j = i;
    if chars[i] == '0' && i + 1 < n {
        match chars[i + 1] {
            'x' | 'X' => {
                j += 2;
                while j < n && chars[j].is_ascii_hexdigit() {
                    j += 1;
                }
            }
            'b' | 'B' => {
                j += 2;
                while j < n && (chars[j] == '0' || chars[j] == '1') {
                    j += 1;
                }
            }
            'o' | 'O' => {
                j += 2;
                while j < n && chars[j].is_ascii_digit() {
                    j += 1;
                }
            }
            _ => {
                while j < n && (chars[j].is_ascii_digit() || chars[j] == '.' || chars[j] == '_') {
                    j += 1;
                }
            }
        }
    } else {
        while j < n && (chars[j].is_ascii_digit() || chars[j] == '.' || chars[j] == '_') {
            j += 1;
        }
    }
    if j < n && (chars[j] == 'l' || chars[j] == 'f') {
        j += 1;
    }
    j
}

fn word_class(word: &str, is_call: bool) -> &'static str {
    if KW.contains(&word) {
        "kw"
    } else if TY.contains(&word) {
        "ty"
    } else if BI.contains(&word) {
        "bi"
    } else if word.starts_with(|c: char| c.is_uppercase()) {
        "en"
    } else if is_call {
        "fn-call"
    } else {
        ""
    }
}

fn emit_span<S: std::hash::BuildHasher>(
    out: &mut String,
    cls: &str,
    word: &str,
    link_map: &HashMap<String, String, S>,
) {
    out.push_str("<span class=\"");
    out.push_str(cls);
    out.push_str("\">");
    if let Some(url) = link_map.get(word) {
        out.push_str("<a href=\"");
        out.push_str(url);
        out.push_str("\">");
        out.push_str(&html_esc(word));
        out.push_str("</a>");
    } else {
        out.push_str(&html_esc(word));
    }
    out.push_str("</span>");
}

/// Render loft source as highlighted HTML, emitting the classes
/// [`DOC.md` § Syntax highlighting classes](../doc/claude/DOC.md) documents.
///
/// Use this instead of raw HTML concatenation for any block of loft. An identifier that
/// appears in `link_map` is wrapped in an `<a href>` as it is highlighted, so cross-linking
/// needs no second pass over the output — and pointing a bigger map at it is how a library's
/// signatures come to link at the types they mention.
pub fn highlight_loft<S: std::hash::BuildHasher>(
    code: &str,
    link_map: &HashMap<String, String, S>,
) -> String {
    let mut out = String::with_capacity(code.len() * 2);

    for line in code.lines() {
        let chars: Vec<char> = line.chars().collect();
        let char_count = chars.len();
        let mut pos = 0;

        while pos < char_count {
            if pos + 1 < char_count && chars[pos] == '/' && chars[pos + 1] == '/' {
                let rest: String = chars[pos..].iter().collect();
                out.push_str("<span class=\"cm\">");
                out.push_str(&html_esc(&rest));
                out.push_str("</span>");
                pos = char_count;
                continue;
            }

            if chars[pos] == '"' {
                let end = scan_quoted(&chars, pos, char_count, '"');
                let token: String = chars[pos..end].iter().collect();
                out.push_str("<span class=\"st\">");
                out.push_str(&html_esc(&token));
                out.push_str("</span>");
                pos = end;
                continue;
            }

            if chars[pos] == '\'' {
                let end = scan_quoted(&chars, pos, char_count, '\'');
                let token: String = chars[pos..end].iter().collect();
                out.push_str("<span class=\"ch\">");
                out.push_str(&html_esc(&token));
                out.push_str("</span>");
                pos = end;
                continue;
            }

            if chars[pos].is_ascii_digit() {
                let end = scan_number(&chars, pos, char_count);
                let token: String = chars[pos..end].iter().collect();
                out.push_str("<span class=\"nm\">");
                out.push_str(&html_esc(&token));
                out.push_str("</span>");
                pos = end;
                continue;
            }

            if chars[pos].is_alphabetic() || chars[pos] == '_' {
                let mut end = pos;
                while end < char_count && (chars[end].is_alphanumeric() || chars[end] == '_') {
                    end += 1;
                }
                let word: String = chars[pos..end].iter().collect();
                let mut peek = end;
                while peek < char_count && chars[peek] == ' ' {
                    peek += 1;
                }
                let cls = word_class(&word, peek < char_count && chars[peek] == '(');
                if cls.is_empty() {
                    out.push_str(&html_esc(&word));
                } else {
                    emit_span(&mut out, cls, &word, link_map);
                }
                pos = end;
                continue;
            }

            out.push_str(&html_esc(&chars[pos].to_string()));
            pos += 1;
        }
        out.push('\n');
    }

    if out.ends_with('\n') {
        out.pop();
    }
    out
}

// ─── Nav builder ──────────────────────────────────────────────────────────────

/// Shared transformation from a loaded topic list to (filename, name) pairs.
/// Separates the disk-reading concern in `gather_topic_info` from the filtering
/// logic used by `render_doc_page` on its already-loaded topic slice.
fn topic_nav_info(topics: &[Topic]) -> Vec<(String, String)> {
    topics
        .iter()
        .filter(|t| !t.filename.starts_with("00-"))
        .map(|t| (t.filename.clone(), t.name.clone()))
        .collect()
}

/// Use when building stdlib pages outside of `generate_docs`, where the full
/// `Topic` list is not already in scope.
#[must_use]
pub fn gather_topic_info() -> Vec<(String, String)> {
    topic_nav_info(&gather_topics())
}

/// Use to get consistent nav HTML for any page — language topic or stdlib
/// section — so that switching page types does not break the nav structure.
/// `active` is the filename stem of the current page.
#[must_use]
pub fn build_nav(
    topic_info: &[(String, String)],
    stdlib_sections: &[StdlibSection],
    active: &str,
) -> String {
    let mut parts: Vec<String> = Vec::new();

    parts.push("<a href=\"index.html\">Home</a>".to_string());

    // Hand-maintained utility pages before the Language section.
    if active == "install" {
        parts.push("<span class=\"cur\">Install</span>".to_string());
    } else {
        parts.push("<a href=\"install.html\">Install</a>".to_string());
    }
    if active == "roadmap" {
        parts.push("<span class=\"cur\">Roadmap</span>".to_string());
    } else {
        parts.push("<a href=\"roadmap.html\">Roadmap</a>".to_string());
    }
    if active == "report" {
        parts.push("<span class=\"cur\">Report a problem</span>".to_string());
    } else {
        parts.push("<a href=\"report.html\">Report a problem</a>".to_string());
    }
    // The registry catalogue. It sits with Install and Roadmap rather than under
    // "Library:" because that section is the bundled STDLIB, and a reader looking for
    // `graphics` is asking a different question from one looking for `len`.
    if active == "libraries" {
        parts.push("<span class=\"cur\">Libraries</span>".to_string());
    } else {
        parts.push("<a href=\"libraries.html\">Libraries</a>".to_string());
    }

    parts.push("<span class=\"nav-sep\">Language:</span>".to_string());

    // vs-Rust and vs-Python are hand-maintained pages with no corresponding .loft file.
    if active == "00-vs-rust" {
        parts.push("<span class=\"cur\">vs Rust</span>".to_string());
    } else {
        parts.push("<a href=\"00-vs-rust.html\">vs Rust</a>".to_string());
    }
    if active == "00-vs-python" {
        parts.push("<span class=\"cur\">vs Python</span>".to_string());
    } else {
        parts.push("<a href=\"00-vs-python.html\">vs Python</a>".to_string());
    }
    if active == "00-performance" {
        parts.push("<span class=\"cur\">Performance</span>".to_string());
    } else {
        parts.push("<a href=\"00-performance.html\">Performance</a>".to_string());
    }

    for (filename, name) in topic_info {
        if filename == active {
            parts.push(format!("<span class=\"cur\">{name}</span>"));
        } else {
            parts.push(format!("<a href=\"{filename}.html\">{name}</a>"));
        }
    }

    if !stdlib_sections.is_empty() {
        parts.push("<span class=\"nav-sep\">Library:</span>".to_string());
        for sec in stdlib_sections {
            let stem = format!("stdlib-{}", sec.id);
            if stem == active {
                parts.push(format!("<span class=\"cur\">{}</span>", sec.name));
            } else {
                parts.push(format!(
                    "<a href=\"stdlib-{}.html\">{}</a>",
                    sec.id, sec.name
                ));
            }
        }
    }

    let links = parts.join(" · ");
    format!(
        "<div class=\"nav-links\">{links}</div>\
<div class=\"search-wrap\">\
<input type=\"search\" id=\"search\" placeholder=\"Search functions, types…\" autocomplete=\"off\">\
<div id=\"search-results\" class=\"search-results\" hidden></div>\
</div>"
    )
}

// ─── HTML page renderer ───────────────────────────────────────────────────────

/// T2.1 — where the published site lives.  Absolute URLs are REQUIRED for
/// OpenGraph: a crawler resolving a share card has no page context, so a
/// relative `og:image` silently yields no preview.
pub const SITE_BASE: &str = "https://loft-lang.org/loft/";

/// The pitch that rides after every page title.  Search results and link
/// previews show the title alone, so the page name comes FIRST and the pitch
/// second — "Structs — Loft, a programming language for small browser games".
/// "programming language" (not just "language") is the phrase people actually
/// search, and "loft" alone is far too ambiguous to rank for — so every page
/// title carries the full term.
pub const SITE_TAGLINE: &str = "Loft, a programming language for small browser games";

/// The landing page's own title.  Sub-pages read "<page> \u{2014} Loft, a programming
/// language for small browser games"; on the index that shape stutters
/// ("Loft \u{2014} Loft, a programming language\u{2026}"), so the index says it once.
pub const SITE_TITLE_INDEX: &str = "Loft \u{2014} a programming language for small browser games";

/// The default share image (the Brick Buster hero).  OpenGraph does not
/// animate, so this stays a PNG even once an animated hero exists.
pub const SITE_OG_IMAGE: &str = "https://loft-lang.org/loft/images/hero-brick-buster.png";

/// T2.2 — emit `sitemap.xml` and `robots.txt` for the generated site.
///
/// Written after every page exists, by listing `doc/*.html` — deriving the list
/// from the directory rather than a hand-kept table means a new page cannot be
/// silently missing from the sitemap.
///
/// # Errors
/// Returns the I/O error if either file cannot be written.
pub fn generate_sitemap() -> std::io::Result<usize> {
    let mut pages: Vec<String> = crate::file_access::read_dir("doc")?
        .iter()
        .filter_map(|p| {
            if crate::file_access::extension(p).is_some_and(|x| x == "html") {
                p.file_name().map(str::to_string)
            } else {
                None
            }
        })
        // `print.html` is the one-page printable rendering of pages already in
        // the sitemap — indexing it would offer searchers a duplicate of the
        // whole site under one URL.
        .filter(|n| n != "print.html")
        .collect();
    pages.sort();

    let mut xml = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for page in &pages {
        // `index.html` is served at the directory root; list the canonical form.
        let loc = if page == "index.html" {
            SITE_BASE.to_string()
        } else {
            format!("{SITE_BASE}{page}")
        };
        let _ = writeln!(xml, "  <url><loc>{loc}</loc></url>");
    }
    xml.push_str("</urlset>\n");
    crate::file_access::write("doc/sitemap.xml", xml)?;

    crate::file_access::write(
        "doc/robots.txt",
        format!("User-agent: *\nAllow: /\nSitemap: {SITE_BASE}sitemap.xml\n"),
    )?;
    Ok(pages.len())
}

/// T2.1 — the per-page facts the `<head>` needs beyond the title.
pub struct PageMeta<'a> {
    /// File stem of this page (`"05-float"`), used to build the canonical URL.
    pub slug: &'a str,
    /// One sentence describing the page, for `meta description` / `og:description`.
    /// This is what a search result and a shared link actually show.
    pub description: &'a str,
}

/// Escape a string for use inside a double-quoted HTML attribute.
fn attr(v: &str) -> String {
    v.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// T2.1 — the discoverability block: description, OpenGraph, Twitter card and a
/// canonical URL.  Generated pages had none of these, so a search engine had no
/// summary to show and a shared link rendered as a bare URL.
fn head_meta(title: &str, meta: &PageMeta) -> String {
    let desc = attr(meta.description);
    let full_title = attr(&format!("{title} \u{2014} {SITE_TAGLINE}"));
    let url = format!("{SITE_BASE}{}.html", meta.slug);
    format!(
        "  <meta name=\"description\" content=\"{desc}\">\n\
  <link rel=\"canonical\" href=\"{url}\">\n\
  <meta property=\"og:title\" content=\"{full_title}\">\n\
  <meta property=\"og:description\" content=\"{desc}\">\n\
  <meta property=\"og:type\" content=\"article\">\n\
  <meta property=\"og:url\" content=\"{url}\">\n\
  <meta property=\"og:image\" content=\"{SITE_OG_IMAGE}\">\n\
  <meta property=\"og:site_name\" content=\"Loft\">\n\
  <meta name=\"twitter:card\" content=\"summary_large_image\">\n\
  <meta name=\"twitter:title\" content=\"{full_title}\">\n\
  <meta name=\"twitter:description\" content=\"{desc}\">\n\
  <meta name=\"twitter:image\" content=\"{SITE_OG_IMAGE}\">\n"
    )
}

/// Use to get consistent page structure for both language topic pages and stdlib
/// section pages; avoids duplicating the HTML boilerplate in two places.
#[must_use]
pub fn page_html(title: &str, nav: &str, h1: &str, body: &str, meta: &PageMeta) -> String {
    let meta_tags = head_meta(title, meta);
    format!(
        "<!DOCTYPE html>\n\
<html lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\">\n\
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
  <title>{title} \u{2014} {SITE_TAGLINE}</title>\n\
{meta_tags}\
  <link rel=\"stylesheet\" href=\"style.css\">\n\
  <style>.playground-link{{display:inline-block;margin:0.3em 0 0.8em;padding:4px 12px;\
background:#2563eb;color:#fff;border-radius:4px;text-decoration:none;font-size:0.85em}}\
.playground-link:hover{{filter:brightness(1.15)}}\
@media print{{.playground-link{{display:none}}}}</style>\n\
</head>\n\
<body>\n\
  <nav>{nav}</nav>\n\
  <h1>{h1}</h1>\n\
  <article>\n{body}\n  </article>\n\
  <script src=\"search-index.js\"></script>\n\
  <script src=\"search.js\"></script>\n\
  <script>\n\
(function(){{\n\
  var m={{\"Keywords\":\"keywords\",\"Texts\":\"texts\",\"Integers\":\"integers\",\
\"Boolean\":\"boolean\",\"Float\":\"float\",\"Functions\":\"functions\",\
\"Vector\":\"vector\",\"Structs\":\"structs\",\"Enums\":\"enums\",\
\"Sorted\":\"sorted\",\"Index\":\"index\",\"Hash\":\"hash\",\"File\":\"file\",\
\"Lexer\":\"lexer\",\"Parser\":\"parser\",\"Libraries\":\"libraries\",\
\"Store Locks\":\"store_locks\",\"Parallel execution\":\"parallel_execution\",\
\"Logging\":\"logging\",\"Time\":\"time\",\"Safety\":\"safety\",\"JSON\":\"json\",\
\"Generics\":\"generics\",\"Closures\":\"closures\",\"Coroutines\":\"coroutines\",\
\"Tuples\":\"tuples\",\"Match\":\"match\",\"Formatting\":\"formatting\"}};\n\
  var h=document.querySelector('h1');\n\
  if(!h)return;\n\
  var key=m[h.textContent];\n\
  if(!key)return;\n\
  var a=document.createElement('a');\n\
  a.className='playground-link';\n\
  a.href='playground.html?example='+key;\n\
  a.textContent='\\u25B6 Try in Playground';\n\
  h.parentNode.insertBefore(a,h.nextSibling);\n\
}})();\n\
  </script>\n\
</body>\n\
</html>\n"
    )
}

/// Render the body content of a topic page (prose + code, no nav or HTML frame).
/// Supports `// ## Section`, `// ### Sub-section`, and `// - list item` in prose.
#[must_use]
pub fn render_topic_body<S: std::hash::BuildHasher>(
    source: &str,
    link_map: &HashMap<String, String, S>,
) -> String {
    let mut body = String::new();
    for section in parse_sections(source) {
        match section {
            DocSection::Prose(lines) => render_prose_lines(&lines, &mut body),
            DocSection::Code(lines) => {
                let highlighted = highlight_loft(&lines.join("\n"), link_map);
                body.push_str("<pre><code>");
                body.push_str(&highlighted);
                body.push_str("</code></pre>\n");
            }
        }
    }
    body
}

/// Render a topic page's content as Typst markup (no document header; use within a `=` section).
/// `## heading` maps to `===`, `### heading` to `====`.
#[must_use]
pub fn render_topic_typst(source: &str) -> String {
    let mut out = String::new();
    for section in parse_sections(source) {
        match section {
            DocSection::Prose(lines) => out.push_str(&prose_to_typst(&lines)),
            DocSection::Code(lines) => out.push_str(&code_to_typst(&lines.join("\n"))),
        }
    }
    out
}

/// A topic source file's metadata and content, ready for print/typst rendering.
pub struct TopicSource {
    pub filename: String,
    pub name: String,
    pub title: String,
    pub source: String,
}

/// Collect all non-`00-` topic source files for use in print/typst generation.
#[must_use]
pub fn get_topic_sources() -> Vec<TopicSource> {
    gather_topics()
        .into_iter()
        .filter(|t| !t.filename.starts_with("00-"))
        .filter_map(|t| {
            crate::file_access::read_to_string(&t.file)
                .ok()
                .map(|source| TopicSource {
                    filename: t.filename,
                    name: t.name,
                    title: t.title,
                    source,
                })
        })
        .collect()
}

/// @PLN149 step 8 — the Run / REPL / Debug panel appended to an executed topic page.
///
/// The page is a loft program; this is what lets a reader DRIVE it — run it, stop it on a
/// line, and evaluate expressions against the frame it stopped in, in their own browser.
/// The behaviour is `doc/loft-panel.js` over the two wasm entries (`debug_start`,
/// `debug_command`); this is the markup it binds to, plus the page's source for it to run.
///
/// The panel ships `hidden` and the script reveals it, so a reader without JavaScript or
/// without the wasm bundle sees the page exactly as before rather than a dead widget.
///
/// The source rides in a `<pre hidden>` and not a `<script>`: a script element's content is
/// raw text, where an escaped `&lt;` stays four characters in `textContent`, and the panel
/// would then compile something that is not the program on the page.
fn panel_html(source: &str) -> String {
    format!(
        "<section id=\"loft-panel\" class=\"loft-panel\" hidden>\n\
         <h2>Run it yourself</h2>\n\
         <p class=\"lp-note\">This page is a loft program. It runs here, in your browser \u{2014} \
         press Run, then ask it something.</p>\n\
         <div class=\"lp-bar\">\
         <button id=\"lp-run\" disabled>\u{25B6} Run</button>\
         <button id=\"lp-step\" disabled>Step</button>\
         <button id=\"lp-resume\" disabled>Resume</button>\
         <span id=\"lp-status\" class=\"lp-status\">loading\u{2026}</span>\
         </div>\n\
         <pre id=\"lp-output\" class=\"lp-output\"></pre>\n\
         <div id=\"lp-frame\" class=\"lp-frame\" hidden></div>\n\
         <div id=\"lp-callables\" class=\"lp-callables\" hidden></div>\n\
         <div class=\"lp-prompt\"><span class=\"lp-caret\">&gt;</span>\
         <input id=\"lp-input\" type=\"text\" autocomplete=\"off\" spellcheck=\"false\" disabled \
         placeholder=\"an expression \u{2014} it is evaluated where the program stopped\"></div>\n\
         <div id=\"lp-log\" class=\"lp-log\"></div>\n\
         <details class=\"lp-lines\"><summary>Stop on a line</summary>\
         <ol id=\"lp-src\" class=\"lp-src\"></ol></details>\n\
         </section>\n\
         <pre id=\"lp-source\" hidden>{}</pre>\n\
         <style>{PANEL_CSS}</style>\n\
         <script type=\"module\" src=\"loft-panel.js\"></script>\n",
        html_esc(source)
    )
}

/// The panel's styling, beside the markup it styles rather than in the shared stylesheet:
/// every rule here is scoped to `.loft-panel` and exists only where the panel does.
const PANEL_CSS: &str = "\
.loft-panel{margin:2em 0;padding:1em;border:1px solid #d0d7de;border-radius:6px;background:#f6f8fa}\
.loft-panel h2{margin-top:0}\
.lp-note{color:#57606a;font-size:0.9em;margin:0 0 0.8em}\
.lp-bar{display:flex;gap:8px;align-items:center;flex-wrap:wrap;margin-bottom:0.8em}\
.loft-panel button{background:#2563eb;color:#fff;border:0;border-radius:4px;padding:5px 14px;\
font-size:0.9em;font-weight:600;cursor:pointer}\
.loft-panel button:disabled{opacity:0.4;cursor:default}\
.lp-status{font-size:0.85em;color:#57606a}\
.lp-status.lp-ok{color:#1a7f37}.lp-status.lp-err{color:#cf222e}.lp-status.lp-info{color:#57606a}\
.lp-output{background:#fff;border:1px solid #d0d7de;border-radius:4px;padding:8px;\
min-height:2.2em;max-height:16em;overflow:auto;white-space:pre-wrap;margin:0 0 0.8em}\
.lp-frame{background:#fff;border:1px solid #d0d7de;border-radius:4px;padding:8px;margin-bottom:0.8em}\
.lp-frame-head{font-size:0.85em;color:#57606a;margin-bottom:6px}\
.lp-locals{display:flex;gap:6px;flex-wrap:wrap}\
.lp-local,.lp-callable{background:#eaeef2;border-radius:3px;padding:2px 6px;font-size:0.85em;cursor:pointer}\
.lp-local:hover,.lp-callable:hover{background:#d0d7de}\
.lp-callables{display:flex;gap:6px;flex-wrap:wrap;align-items:center;margin-bottom:0.8em}\
.lp-prompt{display:flex;gap:6px;align-items:center}\
.lp-caret{color:#2563eb;font-weight:700}\
.lp-prompt input{flex:1;padding:6px 8px;border:1px solid #d0d7de;border-radius:4px;\
font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:0.9em}\
.lp-log{margin-top:0.6em;display:flex;flex-direction:column;gap:4px;max-height:14em;overflow:auto}\
.lp-row{display:flex;gap:8px;align-items:baseline;font-size:0.9em}\
.lp-q{color:#2563eb;flex:0 0 auto}.lp-a{color:#1f2328;word-break:break-word}\
.lp-row.lp-warn .lp-a{color:#9a6700}\
.lp-lines{margin-top:1em}\
.lp-lines summary{cursor:pointer;font-size:0.9em;color:#57606a}\
.lp-src{margin:0.6em 0 0;padding-left:3.5em;background:#fff;border:1px solid #d0d7de;\
border-radius:4px;max-height:20em;overflow:auto}\
.lp-line{font-family:ui-monospace,SFMono-Regular,Menlo,monospace;font-size:0.82em;\
white-space:pre;cursor:pointer;padding:0 4px}\
.lp-line:hover{background:#eaeef2}\
.lp-line.lp-break{background:#ffebe9;outline:1px solid #cf222e}\
@media print{.loft-panel{display:none}}";

fn render_doc_page<S: std::hash::BuildHasher>(
    source: &str,
    name: &str,
    description: &str,
    active: &str,
    topic_info: &[(String, String)],
    stdlib_sections: &[StdlibSection],
    link_map: &HashMap<String, String, S>,
) -> String {
    let nav = build_nav(topic_info, stdlib_sections, active);
    let mut body = render_topic_body(source, link_map);
    body.push_str(&panel_html(source));
    // The topic's `@TITLE` is already a hand-written one-liner — exactly what a
    // search result should show — so it is the description rather than a scrape
    // of the first paragraph.
    let meta = PageMeta {
        slug: active,
        description,
    };
    page_html(name, &nav, name, &body, &meta)
}

// ─── Package documentation generation ────────────────────────────────────────

/// Build a navigation bar for a package's documentation pages.
/// Contains: package name home link, topic pages, API section links.
fn build_pkg_nav(
    pkg_name: &str,
    topic_info: &[(String, String)],
    api_sections: &[String],
    active: &str,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    if active == "index" {
        parts.push(format!("<span class=\"cur\">{pkg_name}</span>"));
    } else {
        parts.push(format!("<a href=\"index.html\">{pkg_name}</a>"));
    }

    if !topic_info.is_empty() {
        parts.push("<span class=\"nav-sep\">Guides:</span>".to_string());
        for (filename, name) in topic_info {
            if filename == active {
                parts.push(format!("<span class=\"cur\">{name}</span>"));
            } else {
                parts.push(format!("<a href=\"{filename}.html\">{name}</a>"));
            }
        }
    }

    if !api_sections.is_empty() {
        parts.push("<span class=\"nav-sep\">API:</span>".to_string());
        for section_name in api_sections {
            let id = slugify(section_name);
            let stem = format!("api-{id}");
            if stem == active {
                parts.push(format!("<span class=\"cur\">{section_name}</span>"));
            } else {
                parts.push(format!("<a href=\"api-{id}.html\">{section_name}</a>"));
            }
        }
    }

    parts.join(" · ")
}

/// Convert a section name to a URL-safe slug.
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Parsed API section from a package source file.
struct PkgApiSection {
    name: String,
    items: Vec<PkgApiItem>,
}

/// One `pub` item as a reader needs it: the WHOLE declaration head, its doc, and — for a
/// type — every member a program has to name to build or match one.
struct PkgApiItem {
    /// The declaration on one line, however many lines it spans in the source.  A
    /// struct or enum carries its members inline (`pub struct Rect { rx: float, … }`),
    /// in declaration order: this is the search corpus and the registry's `api` field.
    sig: String,
    /// The `//` lines directly above the item; for a one-line item with none, its
    /// trailing ` // …` comment (the way a constant is usually documented).
    doc: Vec<String>,
    /// The declaration head without the member block (`pub struct Rect`), for the
    /// readers that print members one per line.
    head: String,
    /// Each field or variant with its comment, in declaration order.
    members: Vec<(String, String)>,
}

/// A character walk over `lines` from `(line, col)` that knows strings, character
/// literals and `//` comments, so a brace or comma inside one is not structure.
struct SrcCursor<'a> {
    lines: &'a [&'a str],
    line: usize,
    chars: Vec<char>,
    col: usize,
}

enum SrcTok {
    Char(char),
    /// A `//` comment running to the end of the line, text without the marker.
    Comment(String),
    Newline,
    End,
}

impl<'a> SrcCursor<'a> {
    fn new(lines: &'a [&'a str], line: usize) -> Self {
        let chars = lines
            .get(line)
            .map_or_else(Vec::new, |l| l.chars().collect());
        SrcCursor {
            lines,
            line,
            chars,
            col: 0,
        }
    }

    /// The next token.  A quote comes back as a `Char`; the caller reads the literal it
    /// opens with [`Self::literal`], so nothing inside one is taken for structure.
    fn next(&mut self) -> SrcTok {
        if self.col >= self.chars.len() {
            if self.line + 1 >= self.lines.len() {
                self.line = self.lines.len();
                return SrcTok::End;
            }
            self.line += 1;
            self.chars = self.lines[self.line].chars().collect();
            self.col = 0;
            return SrcTok::Newline;
        }
        let c = self.chars[self.col];
        if c == '/' && self.chars.get(self.col + 1) == Some(&'/') {
            let text: String = self.chars[self.col..].iter().collect();
            self.col = self.chars.len();
            return SrcTok::Comment(text.trim_start_matches('/').trim().to_string());
        }
        self.col += 1;
        SrcTok::Char(c)
    }

    /// Consume a literal opened by `q` (already read), returning its text with quotes.
    fn literal(&mut self, q: char) -> String {
        let mut out = String::from(q);
        while self.col < self.chars.len() {
            let c = self.chars[self.col];
            self.col += 1;
            out.push(c);
            if c == '\\' && self.col < self.chars.len() {
                out.push(self.chars[self.col]);
                self.col += 1;
            } else if c == q {
                break;
            }
        }
        out
    }
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The members of the body `cur` has just entered (past its `{`): each at brace depth 1,
/// split at a comma — or, for an interface's methods (`split_at_eol`), at a `;` or a line
/// end — outside any nested bracket.  A comment on the line a member ends on is that
/// member's; comment lines before a member are its own.  Leaves `cur` on the closing `}`.
fn read_members(cur: &mut SrcCursor<'_>, split_at_eol: bool) -> Vec<(String, String)> {
    let mut members: Vec<(String, String)> = Vec::new();
    let mut depth = 0i32;
    let mut member = String::new();
    let mut leading: Vec<String> = Vec::new();
    let mut last_line = usize::MAX; // the line the previous member ended on
    let finish =
        |member: &mut String, leading: &mut Vec<String>, members: &mut Vec<(String, String)>| {
            let m = squash(member.trim().trim_start_matches("pub ").trim());
            if !m.is_empty() {
                members.push((m, leading.join(" ")));
            }
            member.clear();
            leading.clear();
        };
    loop {
        match cur.next() {
            SrcTok::Char(c @ ('"' | '\'')) => member.push_str(&cur.literal(c)),
            SrcTok::Char(c) => match c {
                '(' | '[' | '{' => {
                    depth += 1;
                    member.push(c);
                }
                '}' if depth == 0 => {
                    finish(&mut member, &mut leading, &mut members);
                    break;
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                    member.push(c);
                }
                ',' | ';' if depth == 0 => {
                    finish(&mut member, &mut leading, &mut members);
                    last_line = cur.line;
                }
                _ => member.push(c),
            },
            SrcTok::Comment(t) => {
                if member.trim().is_empty() && cur.line == last_line {
                    if let Some(prev) = members.last_mut()
                        && prev.1.is_empty()
                    {
                        prev.1 = t;
                    }
                } else {
                    leading.push(t);
                }
            }
            SrcTok::Newline => {
                if split_at_eol && depth == 0 && !member.trim().is_empty() {
                    finish(&mut member, &mut leading, &mut members);
                    last_line = cur.line - 1;
                } else {
                    member.push(' ');
                }
            }
            SrcTok::End => break,
        }
    }
    members
}

/// Read the `pub` item that starts on `lines[at]`: its whole head, its trailing comment,
/// and a struct's, enum's or interface's members.  Returns the item and the index of the
/// first line after what it consumed (a function body is left to the caller's walk, as
/// before: its lines hold no `pub` item and clear any pending doc).
fn read_pub_item(lines: &[&str], at: usize) -> (PkgApiItem, usize) {
    // `pub value struct` is a struct too.
    let kind = lines[at]
        .split_whitespace()
        .skip(1)
        .find(|w| *w != "value")
        .unwrap_or("");
    let has_members = matches!(kind, "struct" | "enum" | "interface");
    let mut cur = SrcCursor::new(lines, at);
    let mut head = String::new();
    let mut depth = 0i32; // ( and [ — a line break inside them continues the head
    let mut trailing = String::new();
    let mut body = false;
    loop {
        match cur.next() {
            SrcTok::Char(c @ ('"' | '\'')) => head.push_str(&cur.literal(c)),
            SrcTok::Char(c) => {
                match c {
                    '(' | '[' => depth += 1,
                    ')' | ']' => depth -= 1,
                    '{' if depth <= 0 => {
                        body = true;
                        break;
                    }
                    ';' if depth <= 0 => {
                        // The rest of the line may carry the item's comment.
                        while let SrcTok::Char(_) = cur.next() {}
                        if let Some(rest) = lines.get(cur.line)
                            && cur.line == at
                            && let Some(i) = rest.find("//")
                        {
                            trailing = rest[i..].trim_start_matches('/').trim().to_string();
                        }
                        break;
                    }
                    _ => {}
                }
                head.push(c);
            }
            SrcTok::Comment(t) => {
                if cur.line == at && trailing.is_empty() {
                    trailing = t;
                }
            }
            SrcTok::Newline => {
                // A head continues past a line break inside parentheses, after a
                // dangling `,` / `->` / `=`, or when the next line opens with `->`, `)`
                // or `{` (a return type, a closing paren, a brace on its own line).
                let h = head.trim_end();
                let next = lines.get(cur.line).map_or("", |l| l.trim());
                let continues = depth > 0
                    || h.ends_with(',')
                    || h.ends_with("->")
                    || h.ends_with('=')
                    || next.starts_with("->")
                    || next.starts_with(')')
                    || (has_members && next.starts_with('{'));
                if !continues {
                    break;
                }
                head.push(' ');
            }
            SrcTok::End => break,
        }
    }
    let head = squash(head.trim());
    let mut members: Vec<(String, String)> = Vec::new();
    let mut next_line = if body {
        cur.line + 1
    } else {
        cur.line.max(at + 1)
    };
    if body && has_members {
        members = read_members(&mut cur, kind == "interface");
        next_line = cur.line + 1;
    }
    let sig = if members.is_empty() {
        head.clone()
    } else {
        let inner: Vec<&str> = members.iter().map(|(m, _)| m.as_str()).collect();
        format!("{head} {{ {} }}", inner.join(", "))
    };
    let doc = if trailing.is_empty() {
        Vec::new()
    } else {
        vec![trailing]
    };
    (
        PkgApiItem {
            sig,
            doc,
            head,
            members,
        },
        next_line,
    )
}

/// Parse `pub` items and `// --- Section ---` headers from a source file.
fn parse_pkg_api(content: &str) -> Vec<PkgApiSection> {
    let lines: Vec<&str> = content.lines().collect();
    let mut sections = Vec::new();
    let mut current_name = "General".to_string();
    let mut items: Vec<PkgApiItem> = Vec::new();
    let mut doc: Vec<String> = Vec::new();

    let mut i = 0;
    while i < lines.len() {
        let trimmed = lines[i].trim();
        // Section header
        if trimmed.starts_with("// ---") && trimmed.ends_with("---") {
            let inner = trimmed
                .trim_start_matches('/')
                .trim()
                .trim_matches('-')
                .trim();
            if !inner.is_empty() {
                if !items.is_empty() {
                    sections.push(PkgApiSection {
                        name: current_name.clone(),
                        items: std::mem::take(&mut items),
                    });
                }
                current_name = inner.to_string();
                doc.clear();
                i += 1;
                continue;
            }
        }
        // Comment → accumulate doc
        if trimmed.starts_with("//") {
            let text = trimmed.trim_start_matches('/').trim().to_string();
            doc.push(text);
            i += 1;
            continue;
        }
        // Public item
        if trimmed.starts_with("pub ") {
            let (mut item, next) = read_pub_item(&lines, i);
            // The lines above are the doc; a trailing comment speaks only for an item
            // that has none (`pub const N = 3; // the count`).
            if !doc.is_empty() {
                item.doc = std::mem::take(&mut doc);
            }
            items.push(item);
            i = next;
            continue;
        }
        // Other lines: clear doc accumulation (unless #rust annotation)
        if !trimmed.starts_with('#') {
            doc.clear();
        }
        i += 1;
    }
    if !items.is_empty() {
        sections.push(PkgApiSection {
            name: current_name,
            items,
        });
    }
    sections
}

/// Flatten [`parse_pkg_api`] over a source file's `content` into the registry's
/// function-level surface: one [`ApiItem`](crate::registry_index::ApiItem) per
/// `pub` FUNCTION or TYPE (`fn` / `struct` / `enum` / `typedef` / `interface`) —
/// its signature plus its FULL documentation paragraph (every comment line above
/// the item, newline-joined), the keyword corpus search matches against.  This is
/// the single extractor both feeds run: publish-time over a library's source (→
/// the index `api` field) and search-time over the binary's embedded
/// `default/*.loft` (→ the stdlib surface) — so stdlib and library hits are
/// identical in shape.
///
/// `pub const` and other value bindings are excluded: the question search answers
/// is "what can I CALL or USE", and a constant's inline-comment tail makes a noisy
/// signature.  Registry-gated: its only callers (`loft search`, `loft publish`)
/// are, and the `ApiItem` it returns lives in the registry-gated `registry_index`.
#[cfg(feature = "registry")]
#[must_use]
pub fn extract_api_items(content: &str) -> Vec<crate::registry_index::ApiItem> {
    parse_pkg_api(content)
        .into_iter()
        .flat_map(|s| s.items)
        .filter(|item| {
            let sig = &item.sig;
            // `pub type` is how a type alias is written; `pub value struct` is a struct
            // (`time::DateTime`) — both were missing from search.
            sig.starts_with("pub fn ")
                // @PLN182 — an `operator` definition is a method under its own keyword.
                || sig.starts_with("pub operator ")
                || sig.starts_with("pub struct ")
                || sig.starts_with("pub value struct ")
                || sig.starts_with("pub enum ")
                || sig.starts_with("pub type ")
                || sig.starts_with("pub typedef ")
                || sig.starts_with("pub interface ")
        })
        .map(|item| {
            // The FULL paragraph (every `//` line above the item, newline-joined)
            // is the keyword corpus; the search result displays only its first
            // line as a summary.
            let doc = item.doc.join("\n").trim().to_string();
            crate::registry_index::ApiItem { sig: item.sig, doc }
        })
        .collect()
}

/// The function-level API surface of a whole package — [`extract_api_items`] over
/// ALL of its `src/*.loft` (the flat sibling of [`render_pkg_api_text`]).  Files
/// are read in SORTED order so the derived surface is deterministic and
/// CI-reproducible.  Used by `loft publish` (→ the index `api` field) and by
/// `loft api --json` (→ the registry CI re-derive that keeps the field honest,
/// S7-CI — a pasted `api` that disagrees with the source is rejected).
#[cfg(feature = "registry")]
#[must_use]
pub fn pkg_api_items(pkg_dir: &std::path::Path) -> Vec<crate::registry_index::ApiItem> {
    let mut items = Vec::new();
    if let Ok(rd) = crate::file_access::read_dir(pkg_dir.join("src")) {
        let mut srcs: Vec<std::path::PathBuf> = rd
            .iter()
            .map(crate::file_access::PathText::os_spelling)
            .filter(|p| crate::file_access::extension(p).is_some_and(|x| x == "loft"))
            .collect();
        srcs.sort();
        for f in srcs {
            if let Ok(src) = crate::file_access::read_to_string(&f) {
                items.extend(extract_api_items(&src));
            }
        }
    }
    items
}

/// An item as a reader sees it: the one-line signature, or for a type the head with
/// one member per line and each member's comment beside it.
fn item_block(item: &PkgApiItem) -> String {
    if item.members.is_empty() {
        return item.sig.clone();
    }
    let mut out = format!("{} {{\n", item.head);
    for (m, c) in &item.members {
        if c.is_empty() {
            let _ = writeln!(out, "  {m},");
        } else {
            let _ = writeln!(out, "  {m},  // {c}");
        }
    }
    out.push('}');
    out
}

/// Render an API section page as HTML body content.
fn render_api_section_body(section: &PkgApiSection) -> String {
    let mut body = String::new();
    for item in &section.items {
        body.push_str("<div class=\"item\">\n");
        let shown = item_block(item);
        if !shown.is_empty() {
            writeln!(body, "<pre><code>{}</code></pre>", html_escape(&shown)).expect("");
        }
        // @PLN183 — the shared paragraph renderer: escaped, `spans` as code, citations
        // dropped.  The lines were joined into one raw `<p>`, so a doc saying `vector<T>`
        // reached the browser as an unknown `<T>` tag and the reader saw "vector".
        body.push_str(&crate::doc_render::paragraphs_html(&item.doc.join("\n")));
        body.push_str("</div>\n");
    }
    body
}

/// Escape HTML special characters in a string.
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Plain-text public API surface of a package — the agent-readable sibling of
/// [`generate_pkg_docs`]: the same `pub`-signature + doc-comment extraction,
/// emitted as one greppable text instead of HTML.  This is what `loft api`
/// prints and what the generated `.loft/api/<name>.api` stubs contain.
///
/// # Errors
///
/// Propagates directory-read errors on `src/`; a package without `src/*.loft`
/// files yields a header-only surface, not an error.
pub fn render_pkg_api_text(pkg_dir: &std::path::Path) -> std::io::Result<String> {
    use std::fmt::Write as _;

    let manifest_path = pkg_dir.join("loft.toml");
    let manifest = if crate::file_access::exists(&manifest_path) {
        crate::manifest::read_manifest(&manifest_path.to_string_lossy()).unwrap_or_default()
    } else {
        crate::manifest::Manifest::default()
    };
    let pkg_name = manifest
        .name
        .unwrap_or_else(|| crate::file_access::file_name(pkg_dir).unwrap_or_default());
    let version = manifest.version.unwrap_or_else(|| "0.0.0".to_string());

    let mut out = String::new();
    let _ = writeln!(out, "# {pkg_name} {version} — public API surface");
    let _ = writeln!(out, "# generated from {}", pkg_dir.display());
    let _ = writeln!(out, "# use it with: use {pkg_name}::*;");

    let src_dir = pkg_dir.join("src");
    let mut files: Vec<std::path::PathBuf> = match crate::file_access::read_dir(&src_dir) {
        Ok(read) => read
            .iter()
            .map(crate::file_access::PathText::os_spelling)
            .filter(|p| crate::file_access::extension(p).as_deref() == Some("loft"))
            .collect(),
        Err(_) => Vec::new(),
    };
    files.sort();
    for file in files {
        let content = crate::file_access::read_to_string(&file)?;
        let sections = parse_pkg_api(&content);
        if sections.iter().all(|s| s.items.is_empty()) {
            continue;
        }
        let fname = crate::file_access::file_name(&file).unwrap_or_default();
        let _ = writeln!(out, "\n## src/{fname}");
        for section in sections {
            if section.items.is_empty() {
                continue;
            }
            if section.name != "General" {
                let _ = writeln!(out, "\n// --- {} ---", section.name);
            }
            for item in &section.items {
                let _ = writeln!(out);
                for line in &item.doc {
                    let _ = writeln!(out, "// {line}");
                }
                let _ = writeln!(out, "{}", item_block(item));
            }
        }
    }
    Ok(out)
}

/// Generate documentation for a package directory.
///
/// Expects the standard package layout:
/// - `loft.toml` — package manifest (name, version)
/// - `src/*.loft` — source files (scanned for `pub` API docs)
/// - `docs/*.loft` — topic/guide pages (optional)
///
/// Generates HTML files in `doc/` (created if missing):
/// - `doc/index.html` — package overview
/// - `doc/<topic>.html` — guide pages from `docs/*.loft`
/// - `doc/api-<section>.html` — API reference from `src/*.loft`
///
/// # Errors
/// Returns `Err` when files cannot be read or written.
#[expect(clippy::too_many_lines, reason = "inherited")]
pub fn generate_pkg_docs(
    pkg_dir: &std::path::Path,
    out_override: Option<&std::path::Path>,
) -> std::io::Result<()> {
    let manifest_path = pkg_dir.join("loft.toml");
    let manifest = if crate::file_access::exists(&manifest_path) {
        crate::manifest::read_manifest(&manifest_path.to_string_lossy()).unwrap_or_default()
    } else {
        crate::manifest::Manifest::default()
    };
    let pkg_name = manifest
        .name
        .unwrap_or_else(|| crate::file_access::file_name(pkg_dir).unwrap_or_default());
    let version = manifest.version.unwrap_or_else(|| "0.0.0".to_string());

    // Create the output directory.  A package the caller pointed at gets `doc/`
    // beside its source; an installed library is given a destination of its own
    // (loft#911) because its source tree is shared, immutable cache content.
    let out_dir = out_override.map_or_else(|| pkg_dir.join("doc"), std::path::Path::to_path_buf);
    crate::file_access::create_dir_all(&out_dir)?;

    // Copy style.css from the main doc directory if it exists.
    let main_style = std::path::Path::new("doc/style.css");
    let pkg_style = out_dir.join("style.css");
    if crate::file_access::exists(main_style) && !crate::file_access::exists(&pkg_style) {
        crate::file_access::copy(main_style, &pkg_style)?;
    }

    // Gather topic pages from docs/*.loft
    let docs_dir = pkg_dir.join("docs");
    let topics = if crate::file_access::is_dir(&docs_dir) {
        gather_pkg_topics(&docs_dir)
    } else {
        Vec::new()
    };
    let topic_info: Vec<(String, String)> = topics
        .iter()
        .map(|t| (t.filename.clone(), t.name.clone()))
        .collect();

    // Parse API sections from src/*.loft
    let src_dir = pkg_dir.join("src");
    let mut all_api_sections = Vec::new();
    if crate::file_access::is_dir(&src_dir) {
        // Sorted by name: every entry shares the directory.
        let src_files: Vec<_> = crate::file_access::read_dir(&src_dir)?
            .into_iter()
            .filter(|e| {
                crate::file_access::extension(e).is_some_and(|ext| ext.eq_ignore_ascii_case("loft"))
            })
            .collect();
        for entry in src_files {
            if let Ok(content) = crate::file_access::read_to_string(&entry) {
                all_api_sections.extend(parse_pkg_api(&content));
            }
        }
    }
    let section_names: Vec<String> = all_api_sections.iter().map(|s| s.name.clone()).collect();

    // Generate index page.
    let nav = build_pkg_nav(&pkg_name, &topic_info, &section_names, "index");
    let mut index_body = String::new();
    writeln!(index_body, "<p><strong>{pkg_name}</strong> v{version}</p>").expect("");
    if !topic_info.is_empty() {
        index_body.push_str("<h2>Guides</h2>\n<ul>\n");
        for (filename, name) in &topic_info {
            writeln!(
                index_body,
                "<li><a href=\"{filename}.html\">{name}</a></li>"
            )
            .expect("");
        }
        index_body.push_str("</ul>\n");
    }
    if !section_names.is_empty() {
        index_body.push_str("<h2>API Reference</h2>\n<ul>\n");
        for name in &section_names {
            let id = slugify(name);
            writeln!(index_body, "<li><a href=\"api-{id}.html\">{name}</a></li>").expect("");
        }
        index_body.push_str("</ul>\n");
    }
    let index_meta = PageMeta {
        slug: &pkg_name,
        description: &format!("The {pkg_name} package for Loft — API reference and guides."),
    };
    let index_html = page_html(&pkg_name, &nav, &pkg_name, &index_body, &index_meta);
    crate::file_access::write(out_dir.join("index.html"), index_html)?;

    // Generate topic pages.
    let link_map: HashMap<String, String> = HashMap::new();
    for topic in &topics {
        let stem = &topic.filename;
        let nav = build_pkg_nav(&pkg_name, &topic_info, &section_names, stem);
        if let Ok(source) = crate::file_access::read_to_string(&topic.file) {
            let body = render_topic_body(&source, &link_map);
            let topic_meta = PageMeta {
                slug: &topic.filename,
                description: &topic.title,
            };
            let html = page_html(&topic.name, &nav, &topic.name, &body, &topic_meta);
            crate::file_access::write(out_dir.join(format!("{stem}.html")), html)?;
        }
    }

    // Generate API section pages.
    for section in &all_api_sections {
        let id = slugify(&section.name);
        let stem = format!("api-{id}");
        let nav = build_pkg_nav(&pkg_name, &topic_info, &section_names, &stem);
        let body = render_api_section_body(section);
        let sec_desc = format!(
            "{} — API reference in the {pkg_name} package.",
            section.name
        );
        let sec_meta = PageMeta {
            slug: &stem,
            description: &sec_desc,
        };
        let html = page_html(&section.name, &nav, &section.name, &body, &sec_meta);
        crate::file_access::write(out_dir.join(format!("{stem}.html")), html)?;
    }

    let topic_count = topics.len();
    let api_count = all_api_sections.len();
    // The ABSOLUTE path: a relative `graphics/doc` reads like part of the project you
    // are standing in, which is how stray doc trees ended up committed (loft#911).
    let shown = crate::file_access::plain_canonical(&out_dir)
        .display()
        .to_string();
    println!(
        "Generated docs for {pkg_name}: {topic_count} guide(s), {api_count} API section(s) → {shown}"
    );
    Ok(())
}

/// Gather topic files from a package's docs/ directory.
fn gather_pkg_topics(docs_dir: &std::path::Path) -> Vec<Topic> {
    let mut result = Vec::new();
    // Sorted by name: every entry shares the directory.
    let Ok(entries) = crate::file_access::read_dir(docs_dir) else {
        return result;
    };
    for entry in entries {
        let path = entry.os_spelling();
        if !crate::file_access::extension(&path).is_some_and(|e| e.eq_ignore_ascii_case("loft")) {
            continue;
        }
        let filename = crate::file_access::file_stem(&path).unwrap_or_default();
        let mut name = filename.clone();
        let mut title = String::new();
        if let Ok(file) = crate::file_access::open(&path) {
            let reader = BufReader::new(file);
            for line in reader.lines().map_while(Result::ok) {
                if let Some(s) = line.strip_prefix("// @NAME: ") {
                    name = s.to_string();
                }
                if let Some(s) = line.strip_prefix("// @TITLE: ") {
                    title = s.to_string();
                }
            }
        }
        result.push(Topic {
            file: path,
            filename,
            name,
            title,
        });
    }
    result
}

#[cfg(all(test, feature = "registry"))]
mod tests {
    use super::*;

    // An apostrophe is a possessive and a contraction as well as a code delimiter, and
    // reading every one as a delimiter published three sentences of the Structs page as
    // code.  These are the shapes the topic corpus actually contains.
    #[test]
    fn inline_format_reads_an_apostrophe_as_prose_unless_it_delimits() {
        // After a space, before punctuation: a code span, as it always was.
        assert_eq!(
            inline_format("write 'product.price' here"),
            "write <code>product.price</code> here"
        );
        // The possessive cannot open a span, so the intended opener later in the sentence
        // is still an opener.  (tests/docs/08-struct.loft; doc/08-struct.html shipped this
        // paragraph with three sentences monospaced and a stray quote after them.)
        assert_eq!(
            inline_format("a product's name, using 'product.price'."),
            "a product's name, using <code>product.price</code>."
        );
        // Neither quote qualifies here: one follows `L`, the other precedes `s`.
        assert_eq!(
            inline_format("L'Ecuyer's combined LCG"),
            "L'Ecuyer's combined LCG"
        );
        // A contraction inside a span does not close it early.
        assert_eq!(
            inline_format("say 'don't' now"),
            "say <code>don't</code> now"
        );
        // A backtick has one meaning, so the next backtick closes it.
        assert_eq!(
            inline_format("use `len(v)` here"),
            "use <code>len(v)</code> here"
        );
    }

    #[test]
    fn extract_api_items_pulls_pub_sig_and_full_doc() {
        // The `//` lines directly above a `pub fn` are its doc; the signature is
        // the declaration with the body stripped; non-`pub` items are excluded.
        let src = "\
// A greeting helper.
// Detail on a second line.
pub fn hello(name: text) -> text { name }

fn private_helper() {}
";
        let items = extract_api_items(src);
        assert_eq!(items.len(), 1, "only the pub fn is surfaced");
        assert_eq!(items[0].sig, "pub fn hello(name: text) -> text");
        // S10: the FULL paragraph (both lines), not just the first — the corpus a
        // keyword search matches against.
        assert_eq!(items[0].doc, "A greeting helper.\nDetail on a second line.");
    }

    #[test]
    fn extract_api_items_gathers_types_and_excludes_consts() {
        let src = "\
// An axis-aligned box.
pub struct Rect { x: integer, y: integer }

// Mathematical tau.
pub const TAU = 6.28;

// A drawable shape.
pub enum Shape { Circle, Square }
";
        let sigs: Vec<String> = extract_api_items(src).into_iter().map(|i| i.sig).collect();
        let valued: Vec<String> =
            extract_api_items("pub value struct Ms { ms: integer }\npub type Id = integer;\n")
                .into_iter()
                .map(|i| i.sig)
                .collect();
        assert_eq!(
            valued,
            [
                "pub value struct Ms { ms: integer }",
                "pub type Id = integer"
            ],
            "a value struct and a type alias are part of the usable surface"
        );
        // Types carry their members: a program has to name a field to build one, and a
        // head alone (`pub struct Rect`) sent a reader guessing `x` for `rx`.
        assert!(
            sigs.contains(&"pub struct Rect { x: integer, y: integer }".to_string()),
            "struct gathered with its fields, got {sigs:?}"
        );
        assert!(
            sigs.contains(&"pub enum Shape { Circle, Square }".to_string()),
            "enum gathered with its variants, got {sigs:?}"
        );
        // A `pub const` value is NOT part of the callable/usable surface.
        assert!(
            !sigs.iter().any(|s| s.contains("const")),
            "const excluded, got {sigs:?}"
        );
    }

    /// A signature that wraps is read whole — its later parameters and its return type
    /// were dropped at the first line break, in 103 of 1663 published signatures.
    #[test]
    fn a_wrapped_signature_is_read_whole() {
        let src = "\
// Overlap of two boxes given as raw numbers.
pub fn aabb(ax: float, ay: float,
            bx: float, by: float)
    -> boolean {
  ax < bx
}

// A later item still parses.
pub fn after() -> integer { 1 }
";
        let items = extract_api_items(src);
        let sigs: Vec<&str> = items.iter().map(|i| i.sig.as_str()).collect();
        assert_eq!(
            sigs,
            [
                "pub fn aabb(ax: float, ay: float, bx: float, by: float) -> boolean",
                "pub fn after() -> integer"
            ]
        );
        assert_eq!(items[1].doc, "A later item still parses.");
    }

    /// Fields and variants reach `loft api` one per line with their comments, in
    /// declaration order; a brace or comma inside a string default is not structure,
    /// and a struct-shaped variant stays one member.
    #[test]
    fn members_keep_their_order_comments_and_nesting() {
        let src = "\
// A labelled box.
pub struct Box {
  // the label shown
  label: text = \"a, {b}\",
  w: float,   // width
  h: float    // height
}

// A shape.
pub enum Shape {
  Dot,
  Circle { r: float },
  Rect { w: float, h: float }
}
";
        let sections = parse_pkg_api(src);
        let items = &sections[0].items;
        assert_eq!(
            items[0].members,
            [
                (
                    "label: text = \"a, {b}\"".to_string(),
                    "the label shown".to_string()
                ),
                ("w: float".to_string(), "width".to_string()),
                ("h: float".to_string(), "height".to_string()),
            ]
        );
        assert_eq!(
            items[1].sig,
            "pub enum Shape { Dot, Circle { r: float }, Rect { w: float, h: float } }"
        );
        assert_eq!(
            item_block(&items[0]),
            "pub struct Box {\n  label: text = \"a, {b}\",  // the label shown\n  w: float,  // width\n  h: float,  // height\n}"
        );
    }

    /// A constant is usually documented by a comment at the end of its own line; that
    /// comment is its doc when no line above speaks for it, and never when one does.
    #[test]
    fn a_trailing_comment_documents_an_item_with_no_doc_above() {
        let src = "\
pub const FIT_OK = 0;
pub const FIT_NO_FORM = 1;   // the form is not admissible
// Documented above.
pub const FIT_BAD = 2;  // not this
";
        let sections = parse_pkg_api(src);
        let docs: Vec<Vec<String>> = sections[0].items.iter().map(|i| i.doc.clone()).collect();
        assert_eq!(
            docs,
            [
                Vec::<String>::new(),
                vec!["the form is not admissible".to_string()],
                vec!["Documented above.".to_string()],
            ]
        );
        assert_eq!(sections[0].items[1].sig, "pub const FIT_NO_FORM = 1");
    }

    /// A worked-example tag DEFINITION is bookkeeping, and a topic page must not print it.
    ///
    /// `@FTR-037` reached the published Sorted Collections page as a bare paragraph
    /// beginning "@FTR-037 —". The whole comment block has to go: the description wraps,
    /// so dropping only the tagged line leaves its continuation as an orphan sentence.
    #[test]
    fn a_worked_example_tag_block_is_not_rendered_as_prose() {
        let src = "\
// Prose the reader asked for.

// @FTR-037 — a `sorted` collection keeps its order as records arrive, descending on a
// `-key`, and answers a lookup by that key
fn main() {
  // ## A heading
  // Prose after the tag.
}
";
        let body = render_topic_body(src, &HashMap::<String, String>::new());
        assert!(
            !body.contains("FTR-037"),
            "the tag must not reach the page: {body}"
        );
        assert!(
            !body.contains("answers a lookup by that key"),
            "the tag's continuation line must go with it: {body}"
        );
        // The control: prose on BOTH sides of the block still renders, so the test is
        // measuring suppression of the block and not suppression of everything.
        assert!(
            body.contains("Prose the reader asked for."),
            "prose before the tag survives: {body}"
        );
        assert!(
            body.contains("Prose after the tag."),
            "prose after the tag survives: {body}"
        );
        assert!(
            body.contains("A heading"),
            "a heading after the tag survives: {body}"
        );
    }

    /// A citation is a BLOCK: the opener and the lines it wraps onto both go, and the
    /// blank that ends it stays so the paragraphs either side do not weld together.
    #[test]
    fn a_worked_example_citation_is_dropped_with_its_continuation_lines() {
        let doc = vec![
            "The topmost node under this screen point, or -1.",
            "Example: @STG-006 — picking samples alpha, so a click falls",
            "through a hole.",
            "",
            "Walks the draw order BACKWARDS, so the node drawn last is tested first.",
        ];
        let kept = without_example_citations(&doc);
        assert_eq!(
            kept,
            vec![
                "The topmost node under this screen point, or -1.".to_string(),
                String::new(),
                "Walks the draw order BACKWARDS, so the node drawn last is tested first."
                    .to_string(),
            ],
            "opener and continuation go, the separating blank stays"
        );
    }

    /// The control the block rule needs: a tag written INSIDE a sentence is the author's
    /// prose, not bookkeeping, so only a line that OPENS with the citation is dropped.
    /// A predicate that matched any tag would cut `hex_shape`'s measurement in half.
    #[test]
    fn a_tag_inside_a_sentence_is_prose_and_survives() {
        let doc = vec!["half of `D` sits 1.1021 degrees off its nominal (@HXS-002), so the"];
        assert_eq!(
            without_example_citations(&doc),
            vec![doc[0].to_string()],
            "a mid-sentence tag is the author's prose"
        );
        assert!(!opens_example_citation(doc[0]));
    }

    /// Consecutive citations are one run, and a citation that ends the comment needs no
    /// terminator — the two shapes that make up most of the distribution's 377.
    #[test]
    fn consecutive_citations_all_go_and_a_trailing_one_needs_no_blank() {
        let doc = vec![
            "Seed an independent stream.",
            "Example: @RND-001 — the reason to prefer this over `rand_seed`.",
            "Example: @RND-003 — equal seeds replay exactly.",
        ];
        assert_eq!(
            without_example_citations(&doc),
            vec!["Seed an independent stream.".to_string()]
        );
    }

    /// Both input shapes reach the predicate: the extractors hand on stripped comment
    /// text, a caller reading source lines has not stripped the `//`.
    #[test]
    fn the_citation_predicate_accepts_a_line_with_or_without_its_comment_marker() {
        assert!(opens_example_citation("Example: @STD-012"));
        assert!(opens_example_citation(
            "// Example: @STD-012 — trailing prose"
        ));
        assert!(opens_example_citation("  //   Example:  @STD-012"));
        assert!(
            !opens_example_citation("Example: see rand_seed"),
            "prose that merely opens with the word is not a citation"
        );
        assert!(
            !opens_example_citation("Example: @PLN3"),
            "a plan tag is not the worked-example shape"
        );
    }

    /// The tag shape is deliberately narrow so it cannot swallow loft's tracker families.
    #[test]
    fn the_example_tag_shape_excludes_the_tracker_families() {
        assert!(
            is_example_tag("STD-001"),
            "three letters, hyphen, three digits"
        );
        assert!(is_example_tag("FTR-037 — trailing prose is fine"));
        assert!(!is_example_tag("PLN3"), "@PLN3 is a plan, not an example");
        assert!(!is_example_tag("P259"), "@P259 is a P-issue");
        assert!(!is_example_tag("F7"), "@F7 is a feature");
        assert!(!is_example_tag("FR-B-Copy"), "@FR- is a formal rule");
        assert!(!is_example_tag("STD-01"), "two digits is not the shape");
        assert!(!is_example_tag("ST-001"), "two letters is not the shape");
    }
}
