// Copyright (c) 2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I79 — Documentation generator
//
// Generate standard library HTML pages from the documented default/*.loft files.
// Run with: cargo run --bin gendoc

use loft::documentation::typst_escape;
use loft::documentation::{
    StdlibSection, TopicSource, build_nav, gather_topic_info, generate_docs, get_topic_sources,
    is_catalogue_anchor, is_example_tag, page_html, render_topic_body, render_topic_typst,
    without_example_citations,
};
use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs;

// ---  Data model  ---

#[derive(Debug)]
enum Entry {
    /// A named section header (// --- Name ---).
    Section(String),
    /// A public item or section description.
    /// sig is empty for section-description items.
    Item { sig: String, doc: Vec<String> },
}

struct SectionFull {
    id: String,
    name: String,
    items: Vec<(String, Vec<String>)>, // (signature, doc_lines)
}

// ---  Entry point  ---

fn main() -> std::io::Result<()> {
    let version = env!("CARGO_PKG_VERSION");
    // Every `default/*.loft`, in the order the interpreter loads them — which is the
    // numeric prefix, so sorting the directory gives it. A hard-coded list of three
    // names read as a decision and was a sample: `04_stacktrace`, `05_coroutine`,
    // `06_json` and `07_reflect` joined `default/` afterwards and never joined the list,
    // so the whole JSON and reflection surface was missing from the published Standard
    // Library while the JSON chapter and @F42 documented it.
    let mut files: Vec<std::path::PathBuf> = fs::read_dir("default")
        .expect("default/ is unreadable")
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "loft"))
        .collect();
    files.sort();
    assert!(
        files.len() >= 3,
        "default/ holds {} .loft files — the stdlib surface cannot be that small",
        files.len()
    );

    let mut entries: Vec<Entry> = Vec::new();
    for path in &files {
        match fs::read_to_string(path) {
            Ok(content) => {
                let stem = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                parse_loft(&content, &mut entries, &section_name_for(&stem));
            }
            Err(e) => eprintln!("Cannot read {}: {e}", path.display()),
        }
    }

    let sections: Vec<SectionFull> = build_sections(&entries)
        .into_iter()
        .filter(|s| s.items.iter().any(|(sig, _)| !sig.is_empty()))
        .collect();
    let link_map = build_link_map(&sections);

    let stdlib_info: Vec<StdlibSection> = sections
        .iter()
        .map(|s| StdlibSection {
            id: s.id.clone(),
            name: s.name.clone(),
            description: stdlib_description(&s.id).to_string(),
        })
        .collect();

    generate_docs(&stdlib_info, &link_map, version)?;

    let topic_info = gather_topic_info();

    for section in &sections {
        generate_stdlib_section(section, &stdlib_info, &topic_info)?;
    }

    generate_stdlib_toc(&sections, &stdlib_info, &topic_info)?;
    let registry = load_registry_index()?;
    generate_search_index(&sections, &stdlib_info, &registry)?;

    let topic_sources = get_topic_sources();
    generate_print_page(&topic_sources, &sections, &stdlib_info, &link_map, version)?;
    generate_typst(&topic_sources, &sections, version)?;

    let libraries = generate_libraries_page(&registry, &stdlib_info, &topic_info, &link_map)?;

    let sitemap_pages = loft::documentation::generate_sitemap()?;
    println!("Generated doc/sitemap.xml ({sitemap_pages} pages) + doc/robots.txt");
    println!("Generated {} stdlib section pages", sections.len());
    println!("Generated doc/libraries.html ({libraries} registry packages)");
    Ok(())
}

// ---  Parser  ---

/// Does the next DECLARATION after this point start with `pub`?
///
/// Answers who owns the doc block that opens a section: a `pub` item coming up claims it
/// (`// --- Text ---` then `split`'s doc then `pub fn split`), and anything else means the
/// block describes the section itself (`// --- Text ---` then the blurb then the private
/// `fn OpVarText`). Comments, blank lines and `#` attributes are skipped on the way.
fn next_declaration_is_public(rest: &[&str]) -> bool {
    for line in rest {
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") || t.starts_with('#') {
            continue;
        }
        return t.starts_with("pub ");
    }
    false
}

/// A readable section name for a stdlib file that names none itself: `06_json` -> `Json`.
/// Deliberately plain — it is a placeholder the stderr note asks you to replace with a
/// `// --- Name ---` marker in the file, not a naming scheme to rely on.
fn section_name_for(stem: &str) -> String {
    let bare = stem.split_once('_').map_or(stem, |(_, rest)| rest);
    let mut c = bare.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => stem.to_string(),
    }
}

/// Read one `default/*.loft` into section and item entries.
///
/// `fallback_section` opens the file when it declares something before naming a section of
/// its own. Without it a file's items join whichever section the PREVIOUS file left open,
/// which is silent and wrong: `06_json.loft` and `07_reflect.loft` have no
/// `// --- Name ---` marker, so the whole JSON and reflection surface landed under
/// "Environment" the moment they were read at all.
fn parse_loft(content: &str, entries: &mut Vec<Entry>, fallback_section: &str) {
    let lines: Vec<&str> = content.lines().collect();
    let mut i = 0;
    let mut doc: Vec<String> = Vec::new();
    let mut after_section = false;
    let mut named_a_section = false;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        if let Some(name) = parse_section(trimmed) {
            entries.push(Entry::Section(name));
            named_a_section = true;
            doc.clear();
            after_section = true;
            i += 1;
            continue;
        }

        // A catalogue anchor is metadata for `feature_hygiene.sh`, not prose.  Skipped
        // like a `#` attribute — it must not JOIN the doc block and must not break one
        // either, since a real doc comment can sit on both sides of it.
        if is_catalogue_anchor(trimmed) {
            i += 1;
            continue;
        }

        if trimmed.starts_with("//") {
            let text = trimmed.trim_start_matches('/').trim().to_string();
            doc.push(text);
            i += 1;
            continue;
        }

        if trimmed.starts_with("pub ") {
            if !named_a_section {
                eprintln!(
                    "gendoc: {fallback_section} declares a public item before any \
                     `// --- Section ---` marker; filing it under \"{fallback_section}\". \
                     Add a marker at the top of the file to name the section yourself."
                );
                entries.push(Entry::Section(fallback_section.to_string()));
                named_a_section = true;
            }
            let (sig, consumed) = collect_sig(&lines[i..]);
            entries.push(Entry::Item {
                sig,
                doc: std::mem::take(&mut doc),
            });
            after_section = false;
            i += consumed;
            continue;
        }

        // A BLANK line does not orphan a doc block. The same authoring shape is written
        // both ways across `default/` — `// --- min / max / clamp ---` puts its doc
        // against `pub fn min`, `// --- Text ---` in `02_files.loft` leaves a blank line
        // before `pub fn split` — and only the second lost its documentation, along with
        // `sin`, `sqrt`, `floor`, `round` and 36 others whose published entry was a bare
        // signature. The blank is formatting; the doc belongs to whatever declares next.
        //
        // The one thing a blank still settles is who owns the block that OPENS a section:
        // it is the section's own description unless a `pub` declaration is coming to
        // claim it, so look ahead for that rather than guessing from the layout.
        if trimmed.is_empty() {
            if after_section && !doc.is_empty() && !next_declaration_is_public(&lines[i..]) {
                entries.push(Entry::Item {
                    sig: String::new(),
                    doc: std::mem::take(&mut doc),
                });
                after_section = false;
            }
            i += 1;
            continue;
        }

        // #rust attribute lines do not break doc accumulation.
        if !trimmed.starts_with('#') {
            if after_section && !doc.is_empty() {
                entries.push(Entry::Item {
                    sig: String::new(),
                    doc: std::mem::take(&mut doc),
                });
                // Reached only by a private declaration or a non-`pub` line: a `pub` item
                // would have taken the doc, and the blank-line arm above already offered it
                // to one. So this is a block nobody can claim, which is what a section
                // description is.
            } else {
                doc.clear();
            }
            after_section = false;
        }

        i += 1;
    }
}

fn parse_section(trimmed: &str) -> Option<String> {
    if !trimmed.starts_with("// ---") {
        return None;
    }
    let inner = trimmed
        .trim_start_matches('/')
        .trim()
        .trim_matches('-')
        .trim();
    if inner.is_empty() {
        None
    } else {
        Some(inner.to_string())
    }
}

/// Use this rather than `collect_block` directly; it selects between
/// signature-only and full-body capture based on the declaration kind.
fn collect_sig(lines: &[&str]) -> (String, usize) {
    let first = lines[0].trim();
    // An interface's body IS its contract — the operators or methods a type must define to
    // satisfy it — so, like a struct's fields, it is shown whole rather than cut at `{`.
    if first.starts_with("pub struct")
        || first.starts_with("pub enum")
        || first.starts_with("pub interface")
    {
        return collect_block(lines);
    }
    (strip_body(first), 1)
}

fn collect_block(lines: &[&str]) -> (String, usize) {
    let mut result: Vec<String> = Vec::new();
    let mut depth = 0i32;
    for (idx, &line) in lines.iter().enumerate() {
        for ch in line.trim().chars() {
            match ch {
                '{' => depth += 1,
                '}' => depth -= 1,
                _ => {}
            }
        }
        result.push(strip_offset_comment(line));
        if depth == 0 && idx > 0 {
            return (result.join("\n"), idx + 1);
        }
    }
    (result.join("\n"), lines.len())
}

fn strip_offset_comment(s: &str) -> String {
    if let Some(pos) = s.rfind("//")
        && s[pos + 2..].trim().parse::<i64>().is_ok()
    {
        return s[..pos].trim_end().to_string();
    }
    s.trim_end().to_string()
}

fn strip_body(sig: &str) -> String {
    let mut depth = 0i32;
    let mut result = String::new();
    for ch in sig.chars() {
        match ch {
            '(' => {
                depth += 1;
                result.push(ch);
            }
            ')' => {
                depth -= 1;
                result.push(ch);
            }
            '{' | ';' if depth == 0 => break,
            _ => result.push(ch),
        }
    }
    result.trim_end().to_string()
}

fn stdlib_description(id: &str) -> &'static str {
    match id {
        "types" => "Primitive type conversions and null checks",
        "math" => "Arithmetic, rounding, and trigonometry",
        "text" => "String manipulation and formatting",
        "collections" => "Vector, sorted, index, and hash operations",
        "output-and-diagnostics" => "Print, assert, and diagnostic functions",
        "parallel" => "Parallel for-loop and worker functions",
        "file-system" => "File, directory, and path operations",
        "environment" => "Command-line arguments and program state",
        _ => "",
    }
}

// ---  Section grouping  ---

/// Build this before generating HTML or the link map; it groups the flat entry
/// list into the structure both renderers need, merging sections that share a
/// name across multiple source files.
fn build_sections(entries: &[Entry]) -> Vec<SectionFull> {
    let mut sections: Vec<SectionFull> = Vec::new();
    let mut section_idx: Option<usize> = None;

    for entry in entries {
        match entry {
            Entry::Section(name) => {
                if let Some(idx) = sections.iter().position(|s| &s.name == name) {
                    section_idx = Some(idx);
                } else {
                    sections.push(SectionFull {
                        id: section_id(name),
                        name: name.clone(),
                        items: Vec::new(),
                    });
                    section_idx = Some(sections.len() - 1);
                }
            }
            Entry::Item { sig, doc } => {
                if let Some(idx) = section_idx {
                    sections[idx].items.push((sig.clone(), doc.clone()));
                }
            }
        }
    }
    sections
}

// ---  Link map  ---

/// Build this before calling `generate_docs` and `generate_stdlib_section`;
/// the map lets the syntax highlighter inject stdlib links without a second
/// pass over the generated HTML.
fn build_link_map(sections: &[SectionFull]) -> HashMap<String, String> {
    let mut map: HashMap<String, String> = HashMap::new();

    for section in sections {
        let url = format!("stdlib-{}.html", section.id);
        for (sig, _) in &section.items {
            if sig.is_empty() {
                continue;
            }
            if let Some(name) = sig_name(sig) {
                map.entry(name).or_insert_with(|| url.clone());
            }
        }
    }

    // vector and sorted are user-visible type aliases not captured by pub declarations.
    map.entry("vector".into())
        .or_insert_with(|| "stdlib-collections.html".into());
    map.entry("sorted".into())
        .or_insert_with(|| "stdlib-collections.html".into());

    map
}

fn sig_name(sig: &str) -> Option<String> {
    let trimmed = sig.trim();
    let rest = trimmed
        .strip_prefix("pub fn ")
        .or_else(|| trimmed.strip_prefix("fn "))
        .or_else(|| trimmed.strip_prefix("pub type "))
        .or_else(|| trimmed.strip_prefix("pub struct "))
        .or_else(|| trimmed.strip_prefix("pub enum "))
        .or_else(|| trimmed.strip_prefix("pub interface "))
        .or_else(|| trimmed.strip_prefix("pub "))?;
    let name: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

// ---  HTML rendering  ---

fn section_id(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A doc comment's lines as display paragraphs, blank-line separated.
///
/// Worked-example citations are dropped first: they are bookkeeping addressed to
/// whoever maintains the examples, and this is the one path the stdlib section pages,
/// the print sheet and the PDF all render through.
fn group_paragraphs(lines: &[String]) -> Vec<String> {
    let lines = without_example_citations(lines);
    let mut result = Vec::new();
    let mut current = String::new();
    for line in &lines {
        if line.is_empty() {
            if !current.is_empty() {
                result.push(current.trim().to_string());
                current = String::new();
            }
        } else {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(line);
        }
    }
    if !current.is_empty() {
        result.push(current.trim().to_string());
    }
    result
}

/// Call once per section after building the link map; generates a self-contained
/// page with full nav so readers can navigate to any other section or topic.
fn generate_stdlib_section(
    section: &SectionFull,
    stdlib_info: &[StdlibSection],
    topic_info: &[(String, String)],
) -> std::io::Result<()> {
    let stem = format!("stdlib-{}", section.id);
    let nav = build_nav(topic_info, stdlib_info, &stem);
    let mut body = String::new();
    for (sig, doc_lines) in &section.items {
        let paras = group_paragraphs(doc_lines);
        if sig.is_empty() {
            body.push_str("<div class=\"section-desc\">");
            for p in &paras {
                body.push_str(&format!("<p>{}</p>", esc(p)));
            }
            body.push_str("</div>\n");
        } else {
            body.push_str("<div class=\"item\">\n");
            body.push_str(&format!("<pre><code>{}</code></pre>\n", esc(sig)));
            for p in &paras {
                body.push_str(&format!("<p>{}</p>\n", esc(p)));
            }
            body.push_str("</div>\n");
        }
    }
    // `stdlib_info` carries the hand-written one-line description for this
    // section; fall back to the name when a section has none.
    let desc = stdlib_info.iter().find(|s| s.id == section.id).map_or_else(
        || format!("{} — the Loft standard library.", section.name),
        |s| s.description.clone(),
    );
    let slug = format!("stdlib-{}", section.id);
    let meta = loft::documentation::PageMeta {
        slug: &slug,
        description: &desc,
    };
    let html = page_html(&section.name, &nav, &section.name, &body, &meta);
    fs::create_dir_all("doc")?;
    fs::write(format!("doc/{stem}.html"), html)?;
    println!("Generated doc/{stem}.html");
    Ok(())
}

/// Generate after all section pages exist so the item counts are accurate and
/// readers get a complete overview when landing on the stdlib index.
/// The **Libraries** page — every package in the registry, grouped by category.
///
/// A reader's first question about a distribution is *what is there*, and until this page
/// existed the published site answered it for two of the 42 packages, in an install line.
/// The registry index is the one place that knows the answer, so the page renders that and
/// nothing else: no per-library file to maintain, and it cannot fall behind the registry it
/// is generated from ([USER_DOCS.md](../doc/claude/USER_DOCS.md) Tier 0).
///
/// It is the web twin of `loft api --registry`, over the same `load_index` — the CLI and
/// the page agree because they read one source, not because someone keeps them in step.
///
/// A package appears under each of its categories, the way the maintainer catalogue lists
/// it: a reader arrives looking for *graphics*, not for a name they do not yet know.
/// The registry index the library pages and the search index are all rendered from.
///
/// Offline-tolerant on purpose: `load_index` falls back to the cached index, so a doc build
/// does not need the network — only that the box has talked to the registry once.
///
/// A failure here STOPS the build rather than skipping the library pages. The nav links them
/// from every other page, and a site published without them is the exact hole they exist to
/// close, so a missing registry must not turn into 100-odd links to a 404.
fn load_registry_index() -> std::io::Result<loft::registry_index::RegistryIndex> {
    let opts = loft::install::InstallOptions {
        allow_unsigned: true,
        refresh: false,
        offline: false,
        allow_prerelease: false,
        skip_lockfile: true,
        lock_path: None,
    };
    loft::install::load_index(&opts).map_err(|e| {
        eprintln!(
            "gendoc: cannot build the library pages — the registry index is unreachable \
             and no cached copy is available: {e}\n\
             Run `loft api --registry` once to populate the cache, then re-run gendoc."
        );
        std::io::Error::other("registry index unavailable")
    })
}

fn generate_libraries_page<S: std::hash::BuildHasher>(
    index: &loft::registry_index::RegistryIndex,
    stdlib_sections: &[StdlibSection],
    topic_info: &[(String, String)],
    link_map: &HashMap<String, String, S>,
) -> std::io::Result<usize> {
    // Category → the packages carrying it.  A package with no category still has to be
    // reachable, so it lands under "Other" rather than vanishing from the one page whose
    // job is completeness.
    let mut by_category: std::collections::BTreeMap<&str, Vec<&loft::registry_index::Package>> =
        std::collections::BTreeMap::new();
    for pkg in index.packages.values() {
        if pkg.categories.is_empty() {
            by_category.entry("other").or_default().push(pkg);
        }
        for cat in &pkg.categories {
            by_category.entry(cat.as_str()).or_default().push(pkg);
        }
    }

    let mut body = String::new();
    body.push_str(
        "<p>Every library in the loft registry. Each one is written in loft itself, so its \
         source reads like your own code. Install one with <code>loft install &lt;name&gt;</code>, \
         then <code>use &lt;name&gt;;</code> in your program.</p>\n",
    );
    let _ = writeln!(
        body,
        "<p class=\"lib-count\">{} packages{}</p>",
        index.packages.len(),
        if by_category.len() > 1 {
            format!(" in {} categories", by_category.len())
        } else {
            String::new()
        }
    );
    body.push_str(
        "<p>The same catalogue on the command line: <code>loft api --registry</code>. \
         One library's public surface: <code>loft api &lt;name&gt;</code>.</p>\n",
    );

    for (cat, packages) in &by_category {
        let _ = writeln!(
            body,
            "<h2 id=\"{}\">{}</h2>",
            esc(cat),
            esc(&category_title(cat))
        );
        body.push_str("<table class=\"lib-table\">\n");
        body.push_str("<tr><th>Library</th><th>Version</th><th>What it gives you</th></tr>\n");
        for pkg in packages {
            let latest = loft::registry_index::find_best_version(pkg, "*", false)
                .map_or_else(|| "\u{2014}".to_string(), |v| esc(&v.semver));
            let name = esc(&pkg.name);
            let described = pkg.description.as_deref().unwrap_or("");
            // The name links to the library's own page, not straight out to its
            // repository: a reader deciding whether to depend on something wants the card
            // first, and the repository link is on it.
            let link = format!("<a href=\"lib-{name}.html\"><code>{name}</code></a>");
            let _ = writeln!(
                body,
                "<tr><td>{link}</td><td>{latest}</td><td>{}</td></tr>",
                esc(described)
            );
        }
        body.push_str("</table>\n");
    }

    let nav = build_nav(topic_info, stdlib_sections, "libraries");
    let meta = loft::documentation::PageMeta {
        slug: "libraries",
        description: "Every library in the loft registry \u{2014} graphics and 3D, an HTTP \
             server and client, cryptography, text engines and geometry, each written in loft \
             and installed with `loft install`.",
    };
    let html = page_html("Libraries", &nav, "Libraries", &body, &meta);
    fs::write("doc/libraries.html", html)?;

    generate_library_cards(index, stdlib_sections, topic_info)?;
    let (rendered, unrecorded) =
        generate_library_api_pages(index, stdlib_sections, topic_info, link_map)?;
    println!(
        "Generated {rendered} library API references ({unrecorded} package(s) predate the \
         registry's `api` field and say so)"
    );
    let (guides, no_guide, guide_uncached) =
        generate_library_guides(index, stdlib_sections, topic_info, link_map)?;
    println!(
        "Generated {guides} library guides ({no_guide} package(s) ship none yet, \
         {guide_uncached} not in the local registry cache and say so)"
    );
    let (src_pages, uncached) =
        generate_library_source_pages(index, stdlib_sections, topic_info, link_map)?;
    println!(
        "Generated {src_pages} library source browsers ({uncached} package(s) not in the \
         local registry cache and say so)"
    );
    Ok(index.packages.len())
}

/// One page per library — the **card**: the facts a reader needs before deciding to depend
/// on something, which is a different question from *what is there* (the catalogue) and from
/// *how do I start* (the guide).
///
/// The categories are the tag system and they answer *where does this belong*. Nothing
/// answered *should I depend on this*, even though the registry already declares almost all
/// of it — the version and its date, the loft floor, the compatibility floors, the size, the
/// dependencies, the auto-use triggers and the size of the public surface.
///
/// **The card is a renderer, not a dataset.** No field here is written by hand anywhere, so
/// there is nothing for a maintainer to keep in step and nothing that can drift from the
/// package it describes ([USER_DOCS.md](../doc/claude/USER_DOCS.md) Tier 0).
fn generate_library_cards(
    index: &loft::registry_index::RegistryIndex,
    stdlib_sections: &[StdlibSection],
    topic_info: &[(String, String)],
) -> std::io::Result<()> {
    // "What uses this" — the one field on the card that no package declares about itself.
    // Inverting every package's `deps` is the whole derivation, and it is the field that
    // tells a reader whether they are the first person to try something.
    let mut used_by: std::collections::BTreeMap<&str, Vec<&str>> =
        std::collections::BTreeMap::new();
    for (name, pkg) in &index.packages {
        let Some(v) = loft::registry_index::find_best_version(pkg, "*", false) else {
            continue;
        };
        for dep in v.deps.keys() {
            used_by.entry(dep.as_str()).or_default().push(name.as_str());
        }
    }

    for (name, pkg) in &index.packages {
        let Some(v) = loft::registry_index::find_best_version(pkg, "*", false) else {
            continue;
        };
        let mut body = String::new();

        if let Some(d) = pkg.description.as_deref().filter(|d| !d.is_empty()) {
            let _ = writeln!(body, "<p class=\"lib-lede\">{}</p>", esc(d));
        }
        let _ = writeln!(
            body,
            "<pre><code>loft install {}\n</code></pre>\n<pre><code>use {};\n</code></pre>",
            esc(name),
            esc(name)
        );

        body.push_str("<table class=\"lib-table lib-card\">\n");
        let mut row = |k: &str, val: &str| {
            let _ = writeln!(body, "<tr><th>{k}</th><td>{val}</td></tr>");
        };

        // The guide comes first: a reader who has decided to look at a library wants the
        // introduction before the facts about it. A package that ships none says so and
        // names the cure, because "no link" and "no guide" look identical.
        row(
            "Guide",
            &match guide_source(name, &v.semver) {
                GuideSource::Ships(_) => format!(
                    "<a href=\"lib-{0}-guide.html\">getting started with {0}</a>",
                    esc(name)
                ),
                GuideSource::None => {
                    "none yet \u{2014} the API reference and the source below are what there is"
                        .to_string()
                }
                GuideSource::Uncached => format!(
                    "not known \u{2014} <code>{}</code> is not in this build's registry cache",
                    esc(name)
                ),
            },
        );
        row(
            "Version",
            &format!(
                "{} \u{2014} published {}",
                esc(&v.semver),
                esc(&short_date(&v.published))
            ),
        );
        if !pkg.categories.is_empty() {
            let cats = pkg
                .categories
                .iter()
                .map(|c| {
                    format!(
                        "<a href=\"libraries.html#{}\">{}</a>",
                        esc(c),
                        esc(&category_title(c))
                    )
                })
                .collect::<Vec<_>>()
                .join(" \u{b7} ");
            row("Categories", &cats);
        }
        row("Requires", &format!("loft <code>{}</code>", esc(&v.loft)));
        row(
            "Compatibility",
            &compat_cell(
                &v.semver,
                v.api_compatible_with.as_deref(),
                v.data_compatible_with.as_deref(),
            ),
        );
        row("Download", &human_size(v.size));
        if v.api.is_empty() {
            row(
                "Public API",
                &format!(
                    "<a href=\"lib-{0}-api.html\">not recorded for this version</a> \u{2014} \
                     <code>loft api {0}</code> reads it from the package itself",
                    esc(name)
                ),
            );
        } else {
            row(
                "Public API",
                &format!(
                    "<a href=\"lib-{}-api.html\">{} items, with their documentation</a>",
                    esc(name),
                    v.api.len()
                ),
            );
        }
        row(
            "Dependencies",
            &package_links(
                &v.deps.keys().map(String::as_str).collect::<Vec<_>>(),
                index,
                "none",
            ),
        );
        row(
            "Used by",
            &package_links(
                used_by.get(name.as_str()).map(Vec::as_slice).unwrap_or(&[]),
                index,
                "no other registry package \u{2014} you would be the first",
            ),
        );
        if !v.triggers.is_empty() {
            // A trigger is why a method resolves with no `use` line, which reads as magic
            // until you know the package behind it.
            let names = v
                .triggers
                .iter()
                .map(|t| {
                    let (m, recv) = t.split_once(':').unwrap_or((t.as_str(), ""));
                    if recv.is_empty() {
                        format!("<code>{}</code>", esc(m))
                    } else {
                        format!("<code>{}.{}()</code>", esc(recv), esc(m))
                    }
                })
                .collect::<Vec<_>>()
                .join(" \u{b7} ");
            row(
                "Loads itself for",
                &format!("{names} \u{2014} these resolve with no <code>use</code> line"),
            );
        }
        if !pkg.yanked.is_empty() {
            let ys = pkg
                .yanked
                .iter()
                .map(|y| format!("<code>{}</code>", esc(y)))
                .collect::<Vec<_>>()
                .join(", ");
            row(
                "Yanked versions",
                &format!("{ys} \u{2014} do not pin these"),
            );
        }
        row(
            "Source",
            &format!(
                "<a href=\"lib-{0}-src.html\">read it here</a> \u{2014} the libraries are \
                 written in loft",
                esc(name)
            ),
        );
        if let Some(url) = pkg.homepage.as_deref().filter(|u| !u.is_empty()) {
            row("Repository", &format!("<a href=\"{0}\">{0}</a>", esc(url)));
        }
        body.push_str("</table>\n");

        body.push_str(
            "<p class=\"lib-legend\">A <em>drop-in</em> floor is the oldest release of this \
             package the current one still replaces without a source change. \
             <em>Not declared</em> means exactly that \u{2014} the package has made no promise, \
             which is the honest state for one published before the compatibility contract \
             existed.</p>\n",
        );
        body.push_str("<p><a href=\"libraries.html\">\u{2190} All libraries</a></p>\n");

        let nav = build_nav(topic_info, stdlib_sections, "libraries");
        let desc = pkg
            .description
            .as_deref()
            .filter(|d| !d.is_empty())
            .map_or_else(
                || format!("The {name} library for loft \u{2014} install it with `loft install {name}`."),
                std::string::ToString::to_string,
            );
        let slug = format!("lib-{name}");
        let meta = loft::documentation::PageMeta {
            slug: &slug,
            description: &desc,
        };
        let html = page_html(name, &nav, name, &body, &meta);
        fs::write(format!("doc/lib-{name}.html"), html)?;
    }
    Ok(())
}

/// What the two compatibility floors mean, in the reader's terms.
///
/// The floor is *the oldest release this version is still a drop-in for* — so a floor EQUAL
/// to the version says the opposite of what it looks like: nothing older is compatible,
/// because the break happened here. A version that declares no floor has promised nothing,
/// and that absence is load-bearing rather than an omission to paper over: 18 of the 42
/// packages predate the contract.
fn compat_cell(version: &str, api: Option<&str>, data: Option<&str>) -> String {
    let one = |kind: &str, floor: Option<&str>| -> String {
        match floor {
            None => format!("{kind}: not declared"),
            // Say what the floor literally declares, not what it implies. A floor equal to
            // the version means either "the break happened here" or "this is a first
            // release" — the index cannot tell those apart, and only one of them makes
            // the word "breaks" true.
            Some(f) if f == version => format!("{kind}: no earlier release is a drop-in"),
            Some(f) => format!("{kind}: drop-in for {} and later", esc(f)),
        }
    };
    format!("{} \u{b7} {}", one("API", api), one("stored data", data))
}

/// A dependency / dependent list, each name linked to its own card. `empty` is what the cell
/// says when the list is empty — an empty cell reads as *unknown*, and here it is a fact.
fn package_links(
    names: &[&str],
    index: &loft::registry_index::RegistryIndex,
    empty: &str,
) -> String {
    if names.is_empty() {
        return empty.to_string();
    }
    names
        .iter()
        .map(|n| {
            if index.packages.contains_key(*n) {
                format!("<a href=\"lib-{0}.html\"><code>{0}</code></a>", esc(n))
            } else {
                format!("<code>{}</code>", esc(n))
            }
        })
        .collect::<Vec<_>>()
        .join(" \u{b7} ")
}

/// `2026-08-21T10:48:14Z` \u{2192} `2026-08-21`. A reader wants the day, not the second.
fn short_date(stamp: &str) -> String {
    stamp.split('T').next().unwrap_or(stamp).to_string()
}

/// One API reference page per library — Tier 2, *what is the exact signature?*
///
/// Rendered from the registry index's own `api` array: every `pub` signature with its doc
/// comment, re-derived from source by registry CI at publish time. So this needs no package
/// installed and no clone, and it cannot drift from the version it describes — the two facts
/// that make the reference the tier which already worked.
///
/// Returns `(rendered, unrecorded)`. **A version published before the `api` field existed
/// carries none**, and that page says so and points at the two routes that do work, rather
/// than rendering an empty list that reads as "this library has no public functions". The
/// gap closes itself: the field is filled at a package's next release.
fn generate_library_api_pages<S: std::hash::BuildHasher>(
    index: &loft::registry_index::RegistryIndex,
    stdlib_sections: &[StdlibSection],
    topic_info: &[(String, String)],
    link_map: &HashMap<String, String, S>,
) -> std::io::Result<(usize, usize)> {
    let mut rendered = 0usize;
    let mut unrecorded = 0usize;
    for (name, pkg) in &index.packages {
        let Some(v) = loft::registry_index::find_best_version(pkg, "*", false) else {
            continue;
        };
        let mut body = String::new();
        let _ = writeln!(
            body,
            "<p><a href=\"lib-{0}.html\">\u{2190} {0}</a> \u{b7} version {1}</p>",
            esc(name),
            esc(&v.semver)
        );

        if v.api.is_empty() {
            unrecorded += 1;
            let _ = writeln!(
                body,
                "<p>The registry does not record a public surface for <code>{0}</code> {1}. \
                 The field was added after this version was published, and it is filled from \
                 source at a package's next release \u{2014} so this page fills itself in \
                 when <code>{0}</code> ships again.</p>\n\
                 <p>Both of these work today:</p>\n\
                 <pre><code>loft api {0}\n</code></pre>\n\
                 <p>prints the surface with its documentation, read from the package itself; \
                 and the source is the other route.</p>",
                esc(name),
                esc(&v.semver)
            );
            if let Some(url) = pkg.homepage.as_deref().filter(|u| !u.is_empty()) {
                let _ = writeln!(body, "<p><a href=\"{0}\">{0}</a></p>", esc(url));
            }
        } else {
            rendered += 1;
            let _ = writeln!(
                body,
                "<p class=\"lib-legend\">{} public items, in source order. Generated from the \
                 registry index, which re-derives them from the package source at publish \
                 time \u{2014} so a signature here is the one that version actually ships.</p>",
                v.api.len()
            );
            for item in &v.api {
                // The same wrapper the stdlib section pages use for a documented entry —
                // one structure for "a signature and its doc", so a style applied later
                // reaches both rather than only the half that was written second.
                body.push_str("<div class=\"item\">\n");
                let _ = writeln!(
                    body,
                    "<pre><code>{}</code></pre>",
                    loft::documentation::highlight_loft(&item.sig, link_map)
                );
                body.push_str(&doc_paragraphs(&item.doc));
                body.push_str("</div>\n");
            }
        }

        let nav = build_nav(topic_info, stdlib_sections, "libraries");
        let desc = format!(
            "The public API of the loft library {name} \u{2014} every function, struct and enum \
             it exports, with its documentation."
        );
        let slug = format!("lib-{name}-api");
        let meta = loft::documentation::PageMeta {
            slug: &slug,
            description: &desc,
        };
        let title = format!("{name} API");
        let html = page_html(&title, &nav, &title, &body, &meta);
        fs::write(format!("doc/lib-{name}-api.html"), html)?;
    }
    Ok((rendered, unrecorded))
}

/// One guide page per library that ships one — Tier 1, *how do I start?*
///
/// The guide is a `.loft` file in the package's `docs/`, in the same `@NAME` / `@TITLE`
/// topic format the language pages use, so it is a RUNNING PROGRAM whose prose is its
/// comments: an example that stops working stops compiling, in the library's own CI, which
/// is the property that makes a guide worth trusting.
///
/// Rendered with `render_topic_body` — the same renderer the language topics go through, so
/// a library guide and a language page are the same artefact pointed at a different file.
///
/// Returns `(rendered, without, uncached)`. A library with no guide gets no page and its
/// card says so rather than linking at an empty one; writing `docs/01-getting-started.loft`
/// in the package is the whole of what it takes to appear here.
fn generate_library_guides<S: std::hash::BuildHasher>(
    index: &loft::registry_index::RegistryIndex,
    stdlib_sections: &[StdlibSection],
    topic_info: &[(String, String)],
    link_map: &HashMap<String, String, S>,
) -> std::io::Result<(usize, usize, usize)> {
    let mut rendered = 0usize;
    let mut without = 0usize;
    let mut uncached = 0usize;
    for (name, pkg) in &index.packages {
        let Some(v) = loft::registry_index::find_best_version(pkg, "*", false) else {
            continue;
        };
        let guides = match guide_source(name, &v.semver) {
            GuideSource::Ships(files) => files,
            GuideSource::None => {
                without += 1;
                continue;
            }
            GuideSource::Uncached => {
                uncached += 1;
                continue;
            }
        };
        rendered += 1;

        let mut body = String::new();
        let _ = writeln!(
            body,
            "<p><a href=\"lib-{0}.html\">\u{2190} {0}</a> \u{b7} <a href=\"lib-{0}-api.html\">API reference</a> \u{b7} <a href=\"lib-{0}-src.html\">source</a></p>",
            esc(name)
        );
        for g in &guides {
            let Ok(source) = fs::read_to_string(g) else {
                continue;
            };
            if guides.len() > 1
                && let Some(t) = topic_title(&source)
            {
                let _ = writeln!(body, "<h2>{}</h2>", esc(&t));
            }
            body.push_str(&render_topic_body(&source, link_map));
        }

        let nav = build_nav(topic_info, stdlib_sections, "libraries");
        let desc = format!(
            "Getting started with the loft library {name} \u{2014} a runnable guide that is \
             itself a loft program, so every example in it compiles."
        );
        let slug = format!("lib-{name}-guide");
        let meta = loft::documentation::PageMeta {
            slug: &slug,
            description: &desc,
        };
        let title = format!("{name} guide");
        let html = page_html(&title, &nav, &title, &body, &meta);
        fs::write(format!("doc/lib-{name}-guide.html"), html)?;
    }
    Ok((rendered, without, uncached))
}

/// What this build box can truthfully say about a package's guide.
///
/// The three answers are not two: *ships none* is a fact about the PACKAGE, *not cached* is
/// a fact about the BOX, and a page that prints the second as the first tells a reader that
/// `graphics` has no guide when it has one.  The release runner is exactly that box — it
/// checks out, generates and publishes with an empty `~/.loft/registry` unless the workflow
/// fills it — so the distinction is the published one, not a corner case.
enum GuideSource {
    /// The package is extracted here and ships these guide files (never empty, sorted).
    Ships(Vec<std::path::PathBuf>),
    /// The package is extracted here and ships no `docs/*.loft`.
    None,
    /// The package is not in this box's registry cache, so nothing about it is known.
    Uncached,
}

/// The one place `docs/*.loft` is looked for, so the card and the guide page cannot disagree
/// about whether there is a page to link at.
fn guide_source(name: &str, semver: &str) -> GuideSource {
    let dir = loft::registry_index::extract_dir(name, semver);
    if !dir.is_dir() {
        return GuideSource::Uncached;
    }
    let mut files: Vec<std::path::PathBuf> = Vec::new();
    collect_loft_files(&dir.join("docs"), &mut files);
    files.sort();
    if files.is_empty() {
        GuideSource::None
    } else {
        GuideSource::Ships(files)
    }
}

/// The `@TITLE:` a topic file declares, used only to head one guide among several.
fn topic_title(source: &str) -> Option<String> {
    source.lines().find_map(|l| {
        l.trim_start()
            .strip_prefix("// @TITLE:")
            .map(|t| t.trim().to_string())
    })
}

/// One `.loft` source file of a package, ready to render.
struct SourceFile {
    /// Package-relative path, e.g. `src/regex.loft`.
    rel: String,
    /// `rel` reduced to an HTML id prefix, so every line gets a stable anchor.
    slug: String,
    text: String,
}

/// Where a name or a worked-example tag lives: which file, which line.
type Site = (String, usize);

/// One page per library — Tier 3, *how does it actually work?*
///
/// The libraries are written IN loft, so this is at the same time the largest body of
/// idiomatic loft in existence and the thing a reader who has finished the tutorial most
/// wants. Until now it existed only on GitHub.
///
/// Rendered from the package in the registry cache — the same source `loft api` reads — so
/// it is the code that version actually ships. Returns `(rendered, uncached)`.
///
/// **Two signals, labelled apart.** A *worked example* is a `// Example: @AAA-###` citation
/// an author chose as the thing to read first (@PLN141); *call sites* are every line that
/// names the item, found mechanically. Showing only the first would make an untagged
/// function look unused, which is false — the convention deliberately refuses a retroactive
/// sweep, so most functions carry no citation and never will.
fn generate_library_source_pages<S: std::hash::BuildHasher>(
    index: &loft::registry_index::RegistryIndex,
    stdlib_sections: &[StdlibSection],
    topic_info: &[(String, String)],
    link_map: &HashMap<String, String, S>,
) -> std::io::Result<(usize, usize)> {
    let mut rendered = 0usize;
    let mut uncached = 0usize;
    for (name, pkg) in &index.packages {
        let Some(v) = loft::registry_index::find_best_version(pkg, "*", false) else {
            continue;
        };
        let dir = loft::registry_index::extract_dir(name, &v.semver);
        let files = collect_sources(&dir);

        let mut body = String::new();
        let _ = writeln!(
            body,
            "<p><a href=\"lib-{0}.html\">\u{2190} {0}</a> \u{b7} <a href=\"lib-{0}-api.html\">API reference</a> \u{b7} version {1}</p>",
            esc(name),
            esc(&v.semver)
        );

        if files.is_empty() {
            // The package is not in this box's registry cache, so there is no source to
            // show.  Say which of the two it is rather than rendering an empty page: an
            // absent library and a library with no code look identical otherwise.
            uncached += 1;
            let _ = writeln!(
                body,
                "<p>The source for <code>{0}</code> {1} is not on this build box, so this page \
                 could not be rendered from it. <code>loft install {0}</code> fetches the \
                 package, and the repository is the other route.</p>",
                esc(name),
                esc(&v.semver)
            );
            if let Some(url) = pkg.homepage.as_deref().filter(|u| !u.is_empty()) {
                let _ = writeln!(body, "<p><a href=\"{0}\">{0}</a></p>", esc(url));
            }
        } else {
            rendered += 1;
            let total: usize = files.iter().map(|f| f.text.lines().count()).sum();
            let _ = writeln!(
                body,
                "<p class=\"lib-legend\">{} file(s), {} lines. This is the source the \
                 published version ships, and it is loft \u{2014} the library is written in \
                 the same language you call it from.</p>",
                files.len(),
                total
            );

            let tags = tag_definitions(&files);
            body.push_str(&public_item_table(&files, &tags, link_map));

            for f in &files {
                let _ = writeln!(body, "<h2 id=\"{}\">{}</h2>", f.slug, esc(&f.rel));
                body.push_str(&numbered_source(f, link_map));
            }
        }

        let nav = build_nav(topic_info, stdlib_sections, "libraries");
        let desc = format!(
            "The loft source of the {name} library \u{2014} every file it ships, \
                     highlighted, with each public item linked to its definition, its worked \
                     example and its call sites."
        );
        let slug = format!("lib-{name}-src");
        let meta = loft::documentation::PageMeta {
            slug: &slug,
            description: &desc,
        };
        let title = format!("{name} source");
        let html = page_html(&title, &nav, &title, &body, &meta);
        fs::write(format!("doc/lib-{name}-src.html"), html)?;
    }
    Ok((rendered, uncached))
}

/// Every `.loft` file a package ships, `src/` before `tests/`.
///
/// The tests are included deliberately: they are where the worked examples live, so leaving
/// them out would make every `// Example:` citation point at a page that does not exist.
/// They are also the best-explained calls in the package.
fn collect_sources(dir: &std::path::Path) -> Vec<SourceFile> {
    let mut out: Vec<SourceFile> = Vec::new();
    for sub in ["src", "tests"] {
        let mut found: Vec<std::path::PathBuf> = Vec::new();
        collect_loft_files(&dir.join(sub), &mut found);
        found.sort();
        for path in found {
            let Ok(text) = fs::read_to_string(&path) else {
                continue;
            };
            let rel = loft::portable_path::portable(path.strip_prefix(dir).unwrap_or(&path));
            let slug: String = rel
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                .collect();
            out.push(SourceFile { rel, slug, text });
        }
    }
    out
}

fn collect_loft_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_loft_files(&p, out);
        } else if p.extension().is_some_and(|x| x == "loft") {
            out.push(p);
        }
    }
}

/// Where each worked-example tag is DEFINED.
///
/// The convention (@PLN141, LIBRARY_AUTHORING § 2a): a comment block above a `fn` defines the
/// first tag it names — unless the block carries an `// Example:` line, which makes the whole
/// block a citation that defines nothing. Both halves matter; keying on the tag alone would
/// make every citation define the tag it cites.
fn tag_definitions(files: &[SourceFile]) -> std::collections::BTreeMap<String, Site> {
    let mut defs: std::collections::BTreeMap<String, Site> = std::collections::BTreeMap::new();
    for f in files {
        for (start, block) in comment_blocks(&f.text) {
            if block.contains("Example:") {
                continue;
            }
            if let Some(tag) = first_tag(&block) {
                defs.entry(tag).or_insert((f.slug.clone(), start));
            }
        }
    }
    defs
}

/// Runs of `//` lines, each with the 1-based line number it starts at.
fn comment_blocks(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    let mut start = 0usize;
    for (i, line) in text.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            if cur.is_empty() {
                start = i + 1;
            }
            cur.push(line);
        } else if !cur.is_empty() {
            out.push((start, cur.join("\n")));
            cur.clear();
        }
    }
    if !cur.is_empty() {
        out.push((start, cur.join("\n")));
    }
    out
}

/// The first `@AAA-###` in a comment block, if any. Three uppercase letters, a hyphen and
/// three digits — the shape that cannot collide with loft's `@P` / `@PLN` / `@F` families.
fn first_tag(block: &str) -> Option<String> {
    let chars: Vec<char> = block.chars().collect();
    for i in 0..chars.len() {
        if chars[i] != '@' {
            continue;
        }
        let w: String = chars[i + 1..].iter().take(7).collect();
        if is_example_tag(&w) {
            return Some(w);
        }
    }
    None
}

/// The navigation that makes a few thousand lines usable: one row per `pub` item, with where
/// it is defined, the worked example that teaches it, and the lines that call it.
fn public_item_table<S: std::hash::BuildHasher>(
    files: &[SourceFile],
    tags: &std::collections::BTreeMap<String, Site>,
    link_map: &HashMap<String, String, S>,
) -> String {
    let mut items: Vec<(String, String, Site, Option<String>)> = Vec::new();
    for f in files {
        let lines: Vec<&str> = f.text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if let Some((kind, nm)) = public_item(line) {
                items.push((
                    kind.to_string(),
                    nm,
                    (f.slug.clone(), i + 1),
                    cited_example(&lines, i),
                ));
            }
        }
    }
    if items.is_empty() {
        return String::new();
    }
    let _ = link_map;

    let mut out = String::new();
    out.push_str("<h2 id=\"items\">Public items</h2>\n");
    out.push_str(
        "<p class=\"lib-legend\">A <em>worked example</em> is a call site the author chose as \
         the thing to read first. <em>Call sites</em> are every line naming the item, found \
         mechanically over the source with comments excluded \u{2014} so an item with no \
         worked example is not an item nobody uses.</p>\n",
    );
    out.push_str("<table class=\"lib-table\">\n");
    out.push_str(
        "<tr><th>Item</th><th>Defined</th><th>Worked example</th><th>Call sites</th></tr>\n",
    );
    for (kind, nm, (slug, line), tag) in &items {
        let example = match tag.as_ref().and_then(|t| tags.get(t).map(|s| (t, s))) {
            Some((t, (ts, tl))) => format!("<a href=\"#{ts}-L{tl}\">{}</a>", esc(t)),
            // A citation whose tag is defined in ANOTHER repo — an application's source, which
            // this package does not ship.  Naming it is still worth more than a blank: the tag
            // is greppable, which is the whole point of the convention.
            None => tag
                .as_ref()
                .map_or_else(|| "\u{2014}".to_string(), |t| esc(t)),
        };
        let sites = call_sites(files, nm, &(slug.clone(), *line));
        let shown = sites
            .iter()
            .take(6)
            .map(|(s, l)| format!("<a href=\"#{s}-L{l}\">{l}</a>"))
            .collect::<Vec<_>>()
            .join(" ");
        let cell = if sites.is_empty() {
            "none in this package".to_string()
        } else if sites.len() > 6 {
            format!("{shown} \u{2026} ({} total)", sites.len())
        } else {
            shown
        };
        let _ = writeln!(
            out,
            "<tr><td><code>{}</code> <span class=\"search-kind\">{}</span></td>\
             <td><a href=\"#{slug}-L{line}\">{line}</a></td><td>{example}</td><td>{cell}</td></tr>",
            esc(nm),
            esc(kind)
        );
    }
    out.push_str("</table>\n");
    out
}

/// `pub fn name(` \u{2192} `("fn", "name")`. Only the declaration forms a reader navigates by.
fn public_item(line: &str) -> Option<(&'static str, String)> {
    let t = line.trim_start();
    let rest = t.strip_prefix("pub ")?;
    let (kind, after) = ["fn", "struct", "enum", "const", "type"]
        .iter()
        .find_map(|k| rest.strip_prefix(k).map(|a| (*k, a)))?;
    let after = after.strip_prefix(' ')?;
    let nm: String = after
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if nm.is_empty() {
        return None;
    }
    let kind = match kind {
        "fn" => "fn",
        "struct" => "struct",
        "enum" => "enum",
        "const" => "const",
        _ => "type",
    };
    Some((kind, nm))
}

/// The `// Example: @AAA-###` in the comment block directly above line `idx`, if there is one.
fn cited_example(lines: &[&str], idx: usize) -> Option<String> {
    let mut block: Vec<&str> = Vec::new();
    let mut i = idx;
    while i > 0 {
        let prev = lines[i - 1].trim_start();
        if prev.starts_with("//") {
            block.push(prev);
            i -= 1;
        } else {
            break;
        }
    }
    let joined = block.join("\n");
    if !joined.contains("Example:") {
        return None;
    }
    first_tag(&joined)
}

/// Every line naming `nm`, excluding its own declaration and excluding comment lines.
///
/// A text scan, not a resolver: it matches the identifier on word boundaries, so it finds
/// uses the citation convention was never going to cover. Comments are excluded because a
/// name discussed in prose is not a call, and their inclusion is what would turn this from a
/// useful signal into noise.
fn call_sites(files: &[SourceFile], nm: &str, def: &Site) -> Vec<Site> {
    let mut out = Vec::new();
    for f in files {
        for (i, line) in f.text.lines().enumerate() {
            let ln = i + 1;
            if f.slug == def.0 && ln == def.1 {
                continue;
            }
            if line.trim_start().starts_with("//") {
                continue;
            }
            if names_identifier(line, nm) {
                out.push((f.slug.clone(), ln));
            }
        }
    }
    out
}

/// Does `line` contain `nm` as a whole identifier?
fn names_identifier(line: &str, nm: &str) -> bool {
    let bytes = line.as_bytes();
    let n = nm.as_bytes();
    let ident = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    let mut from = 0;
    while let Some(rel) = line[from..].find(nm) {
        let at = from + rel;
        let before_ok = at == 0 || !ident(bytes[at - 1]);
        let end = at + n.len();
        let after_ok = end >= bytes.len() || !ident(bytes[end]);
        if before_ok && after_ok {
            return true;
        }
        from = at + 1;
    }
    false
}

/// One file as highlighted, line-anchored source.
///
/// `highlight_loft` closes every span at the end of each line, so its output splits on
/// newlines safely and each line can carry its own anchor.
fn numbered_source<S: std::hash::BuildHasher>(
    f: &SourceFile,
    link_map: &HashMap<String, String, S>,
) -> String {
    let highlighted = loft::documentation::highlight_loft(&f.text, link_map);
    let mut out = String::from("<pre class=\"src\"><code>");
    for (i, line) in highlighted.lines().enumerate() {
        // The line NUMBER is a CSS counter, not markup. Written out per line it was ~60
        // bytes of boilerplate each, which on the largest library is half a megabyte of
        // page weight carrying no information the stylesheet cannot derive.
        let _ = writeln!(out, "<span id=\"{}-L{}\">{line}</span>", f.slug, i + 1);
    }
    out.push_str("</code></pre>\n");
    out
}

/// A doc comment from the registry, as HTML paragraphs.
///
/// The stored form is plain text: paragraphs separated by a blank line, wrapped with hard
/// newlines inside a paragraph, and backtick spans for code. Rejoining the wrapped lines
/// matters — kept as-is the text renders with the author's terminal width baked in, which
/// is not a line break they chose for a browser.
fn doc_paragraphs(doc: &str) -> String {
    // Citations are dropped by LINE before the split, because a citation is appended to
    // the prose paragraph above it rather than given one of its own.
    let doc = without_example_citations(&doc.lines().collect::<Vec<_>>()).join("\n");
    let mut out = String::new();
    for para in doc.split("\n\n") {
        let joined = para
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        if joined.is_empty() {
            continue;
        }
        let _ = writeln!(out, "<p>{}</p>", inline_code(&joined));
    }
    out
}

/// Escape one paragraph, turning `` `spans` `` into `<code>`.
///
/// An unclosed backtick is left as a literal character rather than swallowing the rest of
/// the paragraph into a code span — a doc comment is prose someone typed, and the failure
/// mode of guessing is that the sentence disappears.
fn inline_code(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(open) = rest.find('`') {
        let (before, after) = rest.split_at(open);
        out.push_str(&esc(before));
        match after[1..].find('`') {
            Some(close) => {
                let _ = write!(out, "<code>{}</code>", esc(&after[1..=close]));
                rest = &after[close + 2..];
            }
            None => {
                out.push_str(&esc(after));
                return out;
            }
        }
    }
    out.push_str(&esc(rest));
    out
}

/// A download size a person can judge. Packages here run from 11 kB to 200 kB, so kB is the
/// working unit and MB only appears if one ever grows into it.
fn human_size(bytes: u64) -> String {
    if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else {
        format!("{} kB", bytes.div_ceil(1024))
    }
}

/// The heading a category slug is shown as.  The registry's slugs are terse because they
/// are also search keys (`asset-format`, `cli`); a heading is read by a person.
///
/// An unknown slug renders as itself rather than being dropped, so a category added to the
/// registry appears on the page the same day, unstyled but present.
fn category_title(slug: &str) -> String {
    match slug {
        "animation" => "Animation".to_string(),
        "asset-format" => "Asset formats".to_string(),
        "cli" => "Command line".to_string(),
        "crypto" => "Cryptography".to_string(),
        "encoding" => "Encoding".to_string(),
        "game" => "Games".to_string(),
        "geometry" => "Geometry".to_string(),
        "graphics" => "Graphics and 3D".to_string(),
        "math" => "Mathematics".to_string(),
        "net" => "Networking".to_string(),
        "plugins" => "Plugins".to_string(),
        "random" => "Randomness".to_string(),
        "text" => "Text".to_string(),
        "time" => "Time".to_string(),
        "world" => "World building".to_string(),
        "other" => "Other".to_string(),
        s => s.to_string(),
    }
}

fn generate_stdlib_toc(
    sections: &[SectionFull],
    stdlib_info: &[StdlibSection],
    topic_info: &[(String, String)],
) -> std::io::Result<()> {
    let nav = build_nav(topic_info, stdlib_info, "stdlib");
    let mut body = String::new();
    body.push_str("<div class=\"grid\">\n");
    for section in sections {
        let count = section.items.iter().filter(|(s, _)| !s.is_empty()).count();
        body.push_str(&format!(
            "  <a class=\"card\" href=\"stdlib-{id}.html\"><h2>{name}</h2><p>{count} items</p></a>\n",
            id = section.id,
            name = esc(&section.name),
        ));
    }
    body.push_str("</div>\n");
    let toc_meta = loft::documentation::PageMeta {
        slug: "stdlib",
        description: "The Loft standard library — every built-in function and type, by section.",
    };
    let html = page_html(
        "Standard Library",
        &nav,
        "Loft Standard Library",
        &body,
        &toc_meta,
    );
    fs::write("doc/stdlib.html", &html)?;
    println!("Generated doc/stdlib.html");
    Ok(())
}

/// Generate last so all section URLs are stable before they are written into
/// the index; the index is consumed at page load so stale URLs cause silent
/// broken links.
fn generate_search_index(
    sections: &[SectionFull],
    stdlib_info: &[StdlibSection],
    registry: &loft::registry_index::RegistryIndex,
) -> std::io::Result<()> {
    let mut entries: Vec<String> = Vec::new();

    for sec in stdlib_info {
        entries.push(format!(
            "{{name:{:?},kind:\"section\",url:\"stdlib-{}.html\"}}",
            sec.name, sec.id
        ));
    }

    for section in sections {
        let url = format!("stdlib-{}.html", section.id);
        for (sig, _) in &section.items {
            if sig.is_empty() {
                continue;
            }
            if let Some(name) = sig_name(sig) {
                let kind = sig_kind(sig);
                entries.push(format!("{{name:{:?},kind:{:?},url:{:?}}}", name, kind, url));
            }
        }
    }

    // The registry's public surfaces, so the site's search covers the DISTRIBUTION and not
    // only the bundled stdlib.  A reference nobody can find is half-published, and the same
    // `api` array the reference pages render already carries every name.
    //
    // Qualified `pkg::name`, for two reasons: a bare `render` would collide across packages
    // with nothing in the result to tell them apart, and the match is a substring test — so
    // the qualifier disambiguates the result without narrowing what finds it.
    for (pkg_name, pkg) in &registry.packages {
        let Some(v) = loft::registry_index::find_best_version(pkg, "*", false) else {
            continue;
        };
        let url = format!("lib-{pkg_name}-api.html");
        entries.push(format!(
            "{{name:{:?},kind:\"library\",url:\"lib-{}.html\"}}",
            pkg_name, pkg_name
        ));
        // A guide is the answer to *how do I start?*, and it is the tier a reader is most
        // likely to go looking for by name.  Only a package that actually ships one is
        // listed — the search box must not offer a page the build did not write.
        if matches!(guide_source(pkg_name, &v.semver), GuideSource::Ships(_)) {
            entries.push(format!(
                "{{name:{:?},kind:\"guide\",url:\"lib-{}-guide.html\"}}",
                format!("{pkg_name} guide"),
                pkg_name
            ));
        }
        for item in &v.api {
            if let Some(name) = sig_name(&item.sig) {
                let kind = sig_kind(&item.sig);
                entries.push(format!(
                    "{{name:{:?},kind:{:?},url:{:?}}}",
                    format!("{pkg_name}::{name}"),
                    kind,
                    url
                ));
            }
        }
    }

    let js = format!("const SEARCH_INDEX=[\n{}\n];\n", entries.join(",\n"));
    fs::write("doc/search-index.js", js)?;
    println!("Generated doc/search-index.js ({} entries)", entries.len());
    Ok(())
}

fn sig_kind(sig: &str) -> &'static str {
    let trimmed = sig.trim();
    if trimmed.starts_with("pub fn ") || trimmed.starts_with("fn ") {
        "fn"
    } else if trimmed.starts_with("pub type ") {
        "type"
    } else if trimmed.starts_with("pub struct ") {
        "struct"
    } else if trimmed.starts_with("pub enum ") {
        "enum"
    } else if trimmed.starts_with("pub interface ") || trimmed.starts_with("interface ") {
        "interface"
    } else {
        "const"
    }
}

/// Strip all HTML tags from `s`, returning only the text content.
fn html_strip_tags(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

/// Decode common HTML entities.
fn html_decode(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&mdash;", "\u{2014}")
        .replace("&ndash;", "\u{2013}")
}

/// Return the substring strictly between the first `open` and the next `close`.
fn between<'a>(s: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = s.find(open)? + open.len();
    let end = s[start..].find(close)? + start;
    Some(&s[start..end])
}

/// Render `doc/00-vs-rust.html` as Typst markup for inclusion in the PDF.
/// The HTML is the single source of truth; this function extracts headings,
/// Loft/Rust code blocks, and verdict paragraphs directly from it.
fn render_vs_rust_typst(html: &str) -> String {
    let mut out = String::new();

    // Intro paragraph (between <article> and first <h2>)
    if let Some(after_article) = html
        .find("<article>")
        .map(|p| &html[p + "<article>".len()..])
        && let Some(intro_raw) = between(after_article, "<p>", "</p>")
    {
        let intro = html_decode(&html_strip_tags(intro_raw));
        out.push_str(&typst_escape(intro.trim()));
        out.push_str("\n\n");
    }

    // Walk through each <h2 ...> section
    let mut pos = 0;
    while let Some(rel) = html[pos..].find("<h2 ") {
        let h2_start = pos + rel;

        // Heading text: content between the first > and </h2>
        let after_h2_tag = h2_start + html[h2_start..].find('>').unwrap_or(0) + 1;
        let h2_close = html[after_h2_tag..].find("</h2>").unwrap_or(0);
        let heading_raw = &html[after_h2_tag..after_h2_tag + h2_close];
        let heading = html_decode(&html_strip_tags(heading_raw));
        // Strip leading "N. " prefix — Typst adds its own numbering via #set heading(numbering: …)
        let heading_text = heading.trim();
        let heading_text = heading_text
            .find(". ")
            .filter(|&dot| heading_text[..dot].chars().all(|c| c.is_ascii_digit()))
            .map(|dot| &heading_text[dot + 2..])
            .unwrap_or(heading_text);
        out.push_str(&format!("=== {}\n\n", typst_escape(heading_text)));

        let section_start = after_h2_tag + h2_close + "</h2>".len();

        // Section ends at the next <h2 or </article>
        let section_end = html[section_start..]
            .find("<h2 ")
            .or_else(|| html[section_start..].find("</article>"))
            .map(|p| section_start + p)
            .unwrap_or(html.len());
        let section = &html[section_start..section_end];

        // Loft code block (plain <pre><code>…</code></pre>)
        if let Some(loft_raw) = between(section, "<pre><code>", "</code></pre>") {
            let loft_code = html_decode(&html_strip_tags(loft_raw));
            out.push_str(&format!("```rust\n{}\n```\n\n", loft_code.trim_end()));
        }

        // Rust code block (<pre class="rust-pre"><code>…</code></pre>)
        if let Some(rust_raw) = between(section, "<pre class=\"rust-pre\"><code>", "</code></pre>")
        {
            let rust_code = html_decode(rust_raw);
            out.push_str(&format!("```rust\n{}\n```\n\n", rust_code.trim_end()));
        }

        // Verdict: upside and downside as separate paragraphs
        if let Some(up_raw) = between(section, "<p class=\"up\">", "</p>") {
            let up = html_decode(&html_strip_tags(up_raw));
            out.push_str(&typst_escape(up.trim()));
            out.push_str("\n\n");
        }
        if let Some(down_raw) = between(section, "<p class=\"down\">", "</p>") {
            let down = html_decode(&html_strip_tags(down_raw));
            out.push_str(&typst_escape(down.trim()));
            out.push_str("\n\n");
        }

        pos = section_end;
    }
    out
}

/// Render `doc/00-vs-python.html` as Typst markup for inclusion in the PDF.
fn render_vs_python_typst(html: &str) -> String {
    let mut out = String::new();

    // Intro paragraph (between <article> and first <h2>)
    if let Some(after_article) = html
        .find("<article>")
        .map(|p| &html[p + "<article>".len()..])
        && let Some(intro_raw) = between(after_article, "<p>", "</p>")
    {
        let intro = html_decode(&html_strip_tags(intro_raw));
        out.push_str(&typst_escape(intro.trim()));
        out.push_str("\n\n");
    }

    // Walk through each <h2 ...> section
    let mut pos = 0;
    while let Some(rel) = html[pos..].find("<h2 ") {
        let h2_start = pos + rel;

        // Heading text: content between the first > and </h2>
        let after_h2_tag = h2_start + html[h2_start..].find('>').unwrap_or(0) + 1;
        let h2_close = html[after_h2_tag..].find("</h2>").unwrap_or(0);
        let heading_raw = &html[after_h2_tag..after_h2_tag + h2_close];
        let heading = html_decode(&html_strip_tags(heading_raw));
        // Strip leading "N. " prefix — Typst adds its own numbering via #set heading(numbering: …)
        let heading_text = heading.trim();
        let heading_text = heading_text
            .find(". ")
            .filter(|&dot| heading_text[..dot].chars().all(|c| c.is_ascii_digit()))
            .map(|dot| &heading_text[dot + 2..])
            .unwrap_or(heading_text);
        out.push_str(&format!("=== {}\n\n", typst_escape(heading_text)));

        let section_start = after_h2_tag + h2_close + "</h2>".len();

        // Section ends at the next <h2 or </article>
        let section_end = html[section_start..]
            .find("<h2 ")
            .or_else(|| html[section_start..].find("</article>"))
            .map(|p| section_start + p)
            .unwrap_or(html.len());
        let section = &html[section_start..section_end];

        // Loft code block (plain <pre><code>…</code></pre>)
        if let Some(loft_raw) = between(section, "<pre><code>", "</code></pre>") {
            let loft_code = html_decode(&html_strip_tags(loft_raw));
            out.push_str(&format!("```rust\n{}\n```\n\n", loft_code.trim_end()));
        }

        // Python code block (<pre class="python-pre"><code>…</code></pre>)
        if let Some(python_raw) =
            between(section, "<pre class=\"python-pre\"><code>", "</code></pre>")
        {
            let python_code = html_decode(python_raw);
            out.push_str(&format!("```python\n{}\n```\n\n", python_code.trim_end()));
        }

        // Verdict: upside and downside as separate paragraphs
        if let Some(up_raw) = between(section, "<p class=\"up\">", "</p>") {
            let up = html_decode(&html_strip_tags(up_raw));
            out.push_str(&typst_escape(up.trim()));
            out.push_str("\n\n");
        }
        if let Some(down_raw) = between(section, "<p class=\"down\">", "</p>") {
            let down = html_decode(&html_strip_tags(down_raw));
            out.push_str(&typst_escape(down.trim()));
            out.push_str("\n\n");
        }

        pos = section_end;
    }
    out
}

// ---  Print page  ---

/// Generate a single-file HTML page containing all documentation for offline
/// reading and PDF printing. The page links back to individual online pages
/// and includes a clickable table of contents.
fn generate_print_page(
    topic_sources: &[TopicSource],
    sections: &[SectionFull],
    stdlib_info: &[StdlibSection],
    link_map: &HashMap<String, String>,
    version: &str,
) -> std::io::Result<()> {
    let mut toc = String::from("<nav class=\"print-toc\">\n<h2>Contents</h2>\n<ul>\n");
    toc.push_str(
        "<li><a href=\"00-vs-rust.html\">vs Rust — Key Differences</a> <span class=\"toc-note\">(online only)</span></li>\n",
    );
    toc.push_str(
        "<li><a href=\"00-vs-python.html\">vs Python — Key Differences</a> <span class=\"toc-note\">(online only)</span></li>\n",
    );
    toc.push_str(
        "<li><a href=\"#00-performance\">Performance</a> — <span class=\"toc-desc\">Benchmark results across interpreter, native, wasm, and Rust</span></li>\n",
    );
    for ts in topic_sources {
        toc.push_str(&format!(
            "<li><a href=\"#{fn}\">{name}</a> — <span class=\"toc-desc\">{title}</span></li>\n",
            fn = ts.filename,
            name = esc(&ts.name),
            title = esc(&ts.title),
        ));
    }
    toc.push_str("<li class=\"toc-section\"><a href=\"#stdlib\">Standard Library</a></li>\n");
    for sec in stdlib_info {
        toc.push_str(&format!(
            "<li class=\"toc-indent\"><a href=\"#stdlib-{id}\">{name}</a></li>\n",
            id = sec.id,
            name = esc(&sec.name),
        ));
    }
    toc.push_str("</ul>\n</nav>\n");

    let mut content = toc;

    for ts in topic_sources {
        content.push_str(&format!(
            "<section class=\"print-section\" id=\"{fn}\">\n<h1>{name}</h1>\n",
            fn = ts.filename,
            name = esc(&ts.name),
        ));
        content.push_str(&render_topic_body(&ts.source, link_map));
        content.push_str("</section>\n");
    }

    // Embed the hand-maintained performance page as a print section.
    let perf_html = fs::read_to_string("doc/00-performance.html").unwrap_or_default();
    let article_body = if let (Some(start), Some(end)) =
        (perf_html.find("<article>"), perf_html.find("</article>"))
    {
        &perf_html[start + "<article>".len()..end]
    } else {
        ""
    };
    content.push_str(
        "<section class=\"print-section\" id=\"00-performance\">\n<h1>Performance</h1>\n",
    );
    content.push_str(article_body);
    content.push_str("</section>\n");

    content
        .push_str("<section class=\"print-section\" id=\"stdlib\">\n<h1>Standard Library</h1>\n");
    for section in sections {
        content.push_str(&format!(
            "<h2 id=\"stdlib-{id}\">{name}</h2>\n",
            id = section.id,
            name = esc(&section.name),
        ));
        for (sig, doc_lines) in &section.items {
            let paras = group_paragraphs(doc_lines);
            if sig.is_empty() {
                for p in &paras {
                    content.push_str(&format!("<p class=\"section-desc\">{}</p>\n", esc(p)));
                }
            } else {
                content.push_str("<div class=\"item\">\n");
                content.push_str(&format!("<pre><code>{}</code></pre>\n", esc(sig)));
                for p in &paras {
                    content.push_str(&format!("<p>{}</p>\n", esc(p)));
                }
                content.push_str("</div>\n");
            }
        }
    }
    content.push_str("</section>\n");

    let count = topic_sources.len() + sections.len();
    let html = format!(
        "<!DOCTYPE html>\n\
<html lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\">\n\
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
  <title>Loft Language — Complete Reference</title>\n\
  <link rel=\"stylesheet\" href=\"style.css\">\n\
</head>\n\
<body class=\"print-page\">\n\
  <header class=\"print-header\">\n\
    <h1>Loft Language Reference</h1>\n\
    <p class=\"tagline\">Complete language and standard library documentation</p>\n\
    <p class=\"version\">v{version}</p>\n\
    <p><a href=\"index.html\">&#8592; Back to online documentation</a> \
       &nbsp;|&nbsp; <a href=\"00-vs-rust.html\">vs Rust comparison</a></p>\n\
  </header>\n\
{content}\
</body>\n\
</html>\n",
        content = content,
    );
    fs::write("doc/print.html", &html)?;
    println!("Generated doc/print.html ({count} sections)");
    Ok(())
}

// ---  Typst generation  ---

/// Render inline HTML content as Typst markup.
/// Handles `<code>`, `<strong>`, badge spans; strips all other tags.
fn render_inline_typst(s: &str) -> String {
    // Replace badge spans before general tag stripping
    let s = s.replace(
        "<span class=\"badge planned\">planned</span>",
        "_(planned)_",
    );
    let mut out = String::new();
    let mut pos = 0;
    while pos < s.len() {
        let rest = &s[pos..];
        if rest.starts_with("<code>") {
            let after = pos + "<code>".len();
            if let Some(end) = s[after..].find("</code>") {
                let text = html_decode(&html_strip_tags(&s[after..after + end]));
                out.push('`');
                out.push_str(&text.replace('`', "'"));
                out.push('`');
                pos = after + end + "</code>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with("<strong>") {
            let after = pos + "<strong>".len();
            if let Some(end) = s[after..].find("</strong>") {
                let text = html_decode(&html_strip_tags(&s[after..after + end]));
                out.push('*');
                out.push_str(&typst_escape(&text));
                out.push('*');
                pos = after + end + "</strong>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                pos += end + 1;
            } else {
                pos += 1;
            }
        } else {
            let next_tag = rest.find('<').unwrap_or(rest.len());
            let text = html_decode(&rest[..next_tag]);
            out.push_str(&typst_escape(&text));
            pos += next_tag;
        }
    }
    out
}

/// Render an HTML article page (install.html, roadmap.html) as Typst markup.
/// Handles h2, h3, p, pre/code, ul/li; strips navigation and script elements.
fn render_article_html_typst(html: &str) -> String {
    let article = match between(html, "<article>", "</article>") {
        Some(s) => s,
        None => return String::new(),
    };
    let mut out = String::new();
    let mut pos = 0;
    while pos < article.len() {
        let rest = &article[pos..];
        if rest.starts_with("<h2 ") || rest.starts_with("<h2>") {
            let end_tag = rest.find('>').unwrap_or(0);
            let after_tag = end_tag + 1;
            if let Some(close) = rest[after_tag..].find("</h2>") {
                let heading = html_decode(&html_strip_tags(&rest[after_tag..after_tag + close]));
                out.push_str(&format!("=== {}\n\n", typst_escape(heading.trim())));
                pos += after_tag + close + "</h2>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with("<h3 ") || rest.starts_with("<h3>") {
            let end_tag = rest.find('>').unwrap_or(0);
            let after_tag = end_tag + 1;
            if let Some(close) = rest[after_tag..].find("</h3>") {
                let heading = html_decode(&html_strip_tags(&rest[after_tag..after_tag + close]));
                out.push_str(&format!("==== {}\n\n", typst_escape(heading.trim())));
                pos += after_tag + close + "</h3>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with("<p>") || rest.starts_with("<p ") {
            let end_tag = rest.find('>').unwrap_or(2);
            let after_tag = end_tag + 1;
            if let Some(close) = rest[after_tag..].find("</p>") {
                let text = render_inline_typst(&rest[after_tag..after_tag + close]);
                if !text.trim().is_empty() {
                    out.push_str(text.trim());
                    out.push_str("\n\n");
                }
                pos += after_tag + close + "</p>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with("<pre>") || rest.starts_with("<pre ") {
            let end_tag = rest.find('>').unwrap_or(4);
            let after_pre = end_tag + 1;
            if let Some(close) = rest[after_pre..].find("</pre>") {
                let content = &rest[after_pre..after_pre + close];
                let code_text = if let Some(code) = between(content, "<code>", "</code>") {
                    html_decode(&html_strip_tags(code))
                } else {
                    html_decode(&html_strip_tags(content))
                };
                out.push_str(&format!("```\n{}\n```\n\n", code_text.trim_end()));
                pos += after_pre + close + "</pre>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with("<ul>") {
            if let Some(close) = rest.find("</ul>") {
                let ul_content = &rest["<ul>".len()..close];
                let mut li_pos = 0;
                while li_pos < ul_content.len() {
                    let li_rest = &ul_content[li_pos..];
                    if li_rest.starts_with("<li>") || li_rest.starts_with("<li ") {
                        let end_tag = li_rest.find('>').unwrap_or(3);
                        let after_tag = end_tag + 1;
                        if let Some(li_close) = li_rest[after_tag..].find("</li>") {
                            let item =
                                render_inline_typst(&li_rest[after_tag..after_tag + li_close]);
                            out.push_str(&format!("- {}\n", item.trim()));
                            li_pos += after_tag + li_close + "</li>".len();
                        } else {
                            li_pos += 1;
                        }
                    } else {
                        li_pos += 1;
                    }
                }
                out.push('\n');
                pos += close + "</ul>".len();
            } else {
                pos += 1;
            }
        } else if rest.starts_with('<') {
            if let Some(end) = rest.find('>') {
                pos += end + 1;
            } else {
                pos += rest.chars().next().map_or(1, char::len_utf8);
            }
        } else {
            pos += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}

/// Generate a Typst source file for the full language reference.
/// Compile with: typst compile doc/loft-reference.typ doc/loft-reference.pdf
fn generate_typst(
    topic_sources: &[TopicSource],
    sections: &[SectionFull],
    version: &str,
) -> std::io::Result<()> {
    let mut out = String::new();

    // Document preamble
    out.push_str("// Loft Language Reference — generated by `cargo run --bin gendoc`\n");
    out.push_str("// Compile with: typst compile loft-reference.typ loft-reference.pdf\n\n");
    out.push_str(&format!(
        "#set document(title: \"Loft Language Reference\", keywords: (\"v{version}\"))\n"
    ));
    out.push_str("#set page(paper: \"a4\", margin: (x: 2.5cm, y: 3cm))\n");
    out.push_str("#set text(font: (\"Libertinus Serif\", \"Liberation Serif\", \"DejaVu Serif\"), size: 11pt)\n");
    out.push_str("#set par(justify: true, leading: 0.75em)\n");
    out.push_str("#set heading(numbering: \"1.1.\")\n");
    out.push('\n');
    out.push_str("#show raw.where(block: true): it => block(\n");
    out.push_str("  fill: luma(245),\n");
    out.push_str("  inset: (x: 10pt, y: 8pt),\n");
    out.push_str("  radius: 4pt,\n");
    out.push_str("  width: 100%,\n");
    out.push_str("  it,\n");
    out.push_str(")\n\n");
    out.push_str("#show heading.where(level: 1): it => {\n");
    out.push_str("  pagebreak(weak: true)\n");
    out.push_str("  v(0.5em)\n");
    out.push_str("  it\n");
    out.push_str("}\n\n");

    // Title page
    out.push_str("= Loft Language Reference\n\n");
    out.push_str("#v(1em)\n\n");
    out.push_str("_A statically-typed scripting language with null safety and built-in parallel execution._\n\n");
    out.push_str(&format!(
        "#text(size: 10pt, fill: luma(100))[Version {version}]\n\n"
    ));
    out.push_str(
        "Every code example in this document is an executable part of the test suite.\n\n",
    );
    out.push_str("#outline(depth: 3)\n\n");
    out.push_str("#pagebreak()\n\n");

    // Getting Started — generated from install.html
    if let Ok(install_html) = std::fs::read_to_string("doc/install.html") {
        out.push_str("= Getting Started\n\n");
        out.push_str(&render_article_html_typst(&install_html));
        out.push('\n');
    }

    // vs Rust section — generated directly from the static HTML (single source of truth)
    if let Ok(vs_rust_html) = std::fs::read_to_string("doc/00-vs-rust.html") {
        out.push_str("= vs Rust\n\n");
        out.push_str(&render_vs_rust_typst(&vs_rust_html));
        out.push('\n');
    }

    // vs Python section — generated directly from the static HTML (single source of truth)
    if let Ok(vs_python_html) = std::fs::read_to_string("doc/00-vs-python.html") {
        out.push_str("= vs Python\n\n");
        out.push_str(&render_vs_python_typst(&vs_python_html));
        out.push('\n');
    }

    // Language topic sections
    for ts in topic_sources {
        out.push_str(&format!("= {}\n\n", typst_escape(&ts.name)));
        out.push_str(&render_topic_typst(&ts.source));
        out.push('\n');
    }

    // Standard library
    out.push_str("= Standard Library\n\n");
    for section in sections {
        out.push_str(&format!("== {}\n\n", typst_escape(&section.name)));
        for (sig, doc_lines) in &section.items {
            let paras = group_paragraphs(doc_lines);
            if sig.is_empty() {
                for p in &paras {
                    if !p.is_empty() {
                        out.push_str(&typst_escape(p));
                        out.push_str("\n\n");
                    }
                }
            } else {
                // Code signature block
                out.push_str(&format!("```rust\n{}\n```\n\n", sig));
                for p in &paras {
                    if !p.is_empty() {
                        out.push_str(&typst_escape(p));
                        out.push('\n');
                    }
                }
                if paras.iter().any(|p| !p.is_empty()) {
                    out.push('\n');
                }
            }
        }
    }

    // Roadmap appendix — generated from roadmap.html
    if let Ok(roadmap_html) = std::fs::read_to_string("doc/roadmap.html") {
        out.push_str("= Roadmap\n\n");
        out.push_str(&render_article_html_typst(&roadmap_html));
        out.push('\n');
    }

    fs::write("doc/loft-reference.typ", &out)?;
    println!("Generated doc/loft-reference.typ");
    Ok(())
}

// ── Interface declarations in the stdlib pages ───────────────────────────────

#[cfg(test)]
mod tests {
    use super::{collect_sig, sig_kind, sig_name};

    /// An interface is its own kind in the search index, never mislabelled `"const"`.
    #[test]
    fn sig_kind_interface_returns_interface() {
        assert_eq!(sig_kind("pub interface Ordered { }"), "interface");
        assert_eq!(sig_kind("interface Foo {}"), "interface");
    }

    /// An interface is found by its NAME: without its own prefix the name read was the
    /// keyword, and every stdlib interface was listed as `interface`.
    #[test]
    fn sig_name_of_an_interface_is_its_name() {
        assert_eq!(
            sig_name("pub interface Ordered {").as_deref(),
            Some("Ordered")
        );
    }

    /// An interface's signature carries its body — the operators a type must define.
    #[test]
    fn collect_sig_keeps_an_interface_body() {
        let src = [
            "pub interface Ordered {",
            "  op < (self: Self, other: Self) -> boolean",
            "}",
            "",
            "fn after() {}",
        ];
        let (sig, consumed) = collect_sig(&src);
        assert_eq!(consumed, 3);
        assert!(
            sig.contains("op < (self: Self, other: Self) -> boolean"),
            "{sig}"
        );
    }
}
