// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 — the library-side support surface for `loft-lsp` (the Language
// Server binary in `src/bin/loft-lsp.rs`).  The binary owns the wire protocol;
// this module owns the loft-compiler calls, so the compiler coupling lives in
// the library and stays testable without spawning a server (`tests/lsp_*`).
//
// Feature providers land here step by step: S3 diagnostics (this file), then
// S4 outline / S5 hover / S6 go-to-definition reuse the same fresh-parse.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::data::{Data, MAIN_SOURCE, Type};
use crate::diagnostics::Diagnostics;
use crate::host::{Program, Value};
use crate::json::Parsed;
use crate::lexer::Position;
use crate::parser::Parser;

/// Load the stdlib into a fresh parser: warm-load the precompiled `Data` bundle
/// when it's available (`startup_cache` — the same bundle the `loft` CLI keeps —
/// ~12× faster than re-parsing `default/`), else cold-parse and save the bundle
/// so the next call is warm.  Both paths are gated on `LOFT_STDLIB_CACHE`; with
/// it unset (the test harness) this degrades to a plain cold parse.  A fresh
/// parser per call is still mandatory — loft registers every definition *per
/// source*, so reusing a warm parser re-registers and conflicts (`"Cannot
/// redefine 'main'"`); the cache short-circuits the stdlib *work*, not the rule.
fn load_stdlib(p: &mut Parser, stdlib_dir: &str) {
    // The stdlib prelude (STD_SOURCE) must be registered before the user buffer,
    // or every stdlib symbol reads as undefined — both paths do that first.
    if !crate::startup_cache::warm_load_stdlib(p, stdlib_dir) {
        let _ = p.parse_dir(stdlib_dir, true, false);
        crate::startup_cache::save_stdlib_cache(p, stdlib_dir);
    }
}

/// The one parse every LSP accessor shares: a FRESH parser (the per-source rule
/// above), the stdlib loaded, then the user buffer parsed — with @PLN115
/// occurrence recording ON, so navigation can resolve an identifier by its
/// binding IDENTITY (which local / which method) rather than by name.
///
/// Recording is enabled *after* `load_stdlib`, so only the user buffer's
/// occurrences are captured: the warm-cache path doesn't re-parse the stdlib, and
/// the cold fallback (`parse_dir`) shouldn't pay to record thousands of stdlib
/// locals no editor query asks about.  The CLI / compiler parse never routes
/// through here, so its gate stays off and its IR is byte-identical (the @PLN115
/// S2 gate).  `p.resolutions()` on the returned parser is the query surface.
fn parse_lsp_buffer(text: &str, name: &str, stdlib_dir: &str) -> Parser {
    let mut p = Parser::new();
    load_stdlib(&mut p, stdlib_dir);
    p.set_record_resolutions(true);
    p.parse_source(text, name, false);
    p
}

/// Parse `text` as a standalone loft source — with the stdlib in `stdlib_dir`
/// loaded first — and return its diagnostics (positioned, coded; @I75).  The
/// caller resolves `stdlib_dir` (a deployment concern the binary owns, exactly
/// as the `loft` CLI does), so this stays a pure function of its inputs.
pub fn diagnose(text: &str, name: &str, stdlib_dir: &str) -> Diagnostics {
    diagnose_reach(text, name, stdlib_dir).0
}

/// [`diagnose`], plus whether the parse reached its SECOND pass.
///
/// The flag is what makes two parses comparable. Pass 1 stopping on an error means every
/// pass-2 diagnostic is missing, so a later parse that gets further reports things the
/// earlier one could not have — new to the comparison, but not new to the program
/// (@PLN131's masking rule).
#[must_use]
pub fn diagnose_reach(text: &str, name: &str, stdlib_dir: &str) -> (Diagnostics, bool) {
    let mut p = parse_lsp_buffer(text, name, stdlib_dir);
    let reached = p.reached_second_pass();
    (std::mem::take(&mut p.diagnostics), reached)
}

/// @PLN115 S3 — the parse-time resolution index for a buffer: every identifier
/// occurrence the parser recorded, resolved to its binding IDENTITY.  Because
/// recording is enabled only for the user-buffer parse (see [`parse_lsp_buffer`]),
/// every occurrence here is in `text`, positioned in `text`'s coordinates.
///
/// This is the position-precise substrate that replaces @PLN63's lexical,
/// name-based navigation: S4 (references/rename of a LOCAL) filters these by the
/// binding under the cursor; S7 (inlayHint) reads each `Local`'s binding to type
/// it.  Currently the parser records LOCALS (S2); S5/S6 add globals/fields/methods.
#[must_use]
pub fn resolutions(text: &str, name: &str, stdlib_dir: &str) -> Vec<crate::resolution::Occurrence> {
    parse_lsp_buffer(text, name, stdlib_dir)
        .resolutions()
        .to_vec()
}

/// One top-level definition in a buffer, for the editor Outline / breadcrumb.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// User-facing name, internal encodings stripped: `main`, `text.len`, `Point`.
    pub name: String,
    /// Display kind: `fn` `method` `operator` `struct` `enum` `typedef` `constant`
    /// `interface` — the `api_surface::classify` label set.
    pub kind: &'static str,
    /// 1-based source line of the definition.
    pub line: u32,
    /// 1-based source column of the definition (`Position::pos`).
    pub col: u32,
}

/// The top-level definitions the buffer `text` declares (the user source,
/// `MAIN_SOURCE`) — name, kind, and position — ordered by source position.
/// Drives `textDocument/documentSymbol`.
///
/// Fresh parse per call, the same rule as [`diagnose`].  Only the user buffer's
/// own definitions are returned: stdlib defs live at `STD_SOURCE`, and
/// compiler-`synthetic` defs (e.g. `__nullable<S>`) are excluded — an outline
/// shows what the user wrote, not what the compiler manufactured.  Kind and the
/// decoded name come from the shared [`crate::api_surface::classify`], so an
/// enum VARIANT (part of its enum's shape) is folded out, not listed top-level.
pub fn outline(text: &str, name: &str, stdlib_dir: &str) -> Vec<Symbol> {
    let p = parse_lsp_buffer(text, name, stdlib_dir);
    let data = &p.data;
    let mut symbols: Vec<Symbol> = (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            def.source == MAIN_SOURCE && def.synthetic.is_none()
        })
        .filter_map(|d| {
            let (kind, name) = crate::api_surface::classify(data, d)?;
            let pos = &data.def(d).position;
            Some(Symbol {
                name,
                kind,
                line: pos.line,
                col: pos.pos,
            })
        })
        .collect();
    symbols.sort_by_key(|s| (s.line, s.col));
    symbols
}

/// Hover information for the symbol under a cursor: its signature, its `///` doc,
/// and where it is defined (the location also drives S6 go-to-definition).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hover {
    /// The resolved user-facing name (`area`, `Point`, `print`).
    pub name: String,
    /// A one-line signature: `fn area(p: Point) -> integer`, `struct Point { … }`,
    /// `const PI: float`.  The clean user-facing type spelling (`type_name_str`).
    pub signature: String,
    /// The `///` doc block immediately above the declaration, in reading order —
    /// empty when the definition has none (or its source can't be read).
    pub doc: Vec<String>,
    /// The resolved definition's source file (as the parser recorded it —
    /// the buffer's parse-name for a local def, else a repo-root-relative path).
    pub def_file: String,
    /// 1-based line of the definition's NAME.
    pub def_line: u32,
    /// 1-based column of the definition's NAME (located in the source, so a jump
    /// lands on the name — not the parser's body-start position).
    pub def_col: u32,
}

/// Resolve the identifier under the cursor at (`line`, `col`) — both 1-based,
/// loft-native (the binary converts from LSP's 0-based) — to its definition and
/// return a [`Hover`]: signature + `///` doc + the definition's location.
///
/// Name-based resolution: the identifier is looked up as a free function
/// (`n_<word>`) then as a type / struct / enum / typedef / constant (`<word>`),
/// each falling back to the stdlib source — so hovering `print` resolves.
/// Methods (`t_<LEN><Type>_…`, which need the receiver type) and local variables
/// are NOT resolved here; that needs the position-aware index (a later step).
/// Fresh parse per call, the same rule as [`diagnose`].
#[must_use]
pub fn symbol_at(text: &str, name: &str, stdlib_dir: &str, line: u32, col: u32) -> Option<Hover> {
    let word = word_at(text, line, col)?;
    let p = parse_lsp_buffer(text, name, stdlib_dir);
    let data = &p.data;
    // A free function first, then a type-like def; `def_nr` checks the user source
    // then falls back to the stdlib (STD_SOURCE), so both user and stdlib resolve.
    let mut d = data.def_nr(&format!("n_{word}"));
    if d == u32::MAX {
        d = data.def_nr(&word);
    }
    if d == u32::MAX {
        return None;
    }
    hover_of_def(data, d, text, name, stdlib_dir)
}

/// Resolve a symbol BY NAME (not by cursor) to every definition that name spells:
/// a free function (`n_<symbol>`), a type / struct / enum / typedef / constant
/// (`<symbol>`), AND every method `Type.<symbol>` — so `lookup("len", …)` returns
/// a free `len` plus `text.len`, `vector.len`, … each with its own signature +
/// doc + location.  The agent-facing counterpart to [`symbol_at`]: you know the
/// NAME, not a `(line, col)`.  Empty when nothing spells it.  Fresh parse per
/// call (same rule as [`diagnose`]); `text` may be empty to search the stdlib alone.
#[must_use]
pub fn lookup(symbol: &str, text: &str, name: &str, stdlib_dir: &str) -> Vec<Hover> {
    let p = parse_lsp_buffer(text, name, stdlib_dir);
    let data = &p.data;
    let mut out: Vec<Hover> = Vec::new();
    let mut seen: Vec<u32> = Vec::new();
    let take = |d: u32, out: &mut Vec<Hover>, seen: &mut Vec<u32>| {
        if d == u32::MAX || seen.contains(&d) {
            return;
        }
        let def = data.def(d);
        // Skip machinery: compiler-`synthetic` defs, and the `Dynamic`
        // multi-receiver DISPATCHER (bare `len` → `fn len(text: fn, character:
        // fn, …) -> unknown`) — a degenerate umbrella signature.  The concrete
        // `text.len` / `vector.len` (each a `Function`) carry the real info and
        // come through the method scan below.
        if def.synthetic.is_some() || matches!(def.def_type(), crate::data::DefType::Dynamic) {
            return;
        }
        seen.push(d);
        if let Some(h) = hover_of_def(data, d, text, name, stdlib_dir) {
            out.push(h);
        }
    };
    // Direct: a free function, then a type-like def.
    take(data.def_nr(&format!("n_{symbol}")), &mut out, &mut seen);
    take(data.def_nr(symbol), &mut out, &mut seen);
    // Methods: `t_<LEN><Type>_<symbol>` decode to `Type.<symbol>` — scan and match
    // the trailing segment so `lookup("len")` also surfaces `text.len` etc.
    for d in 0..data.definitions() {
        if let Some((kind, cname)) = crate::api_surface::classify(data, d)
            && kind == "method"
            && cname.rsplit('.').next() == Some(symbol)
        {
            take(d, &mut out, &mut seen);
        }
    }
    out
}

/// Build a [`Hover`] for a resolved definition `d`: signature (via
/// `api_surface::signature_of`) + the `///` doc and name-precise location read
/// from the def's own source (the buffer for a local def, the file on disk for
/// stdlib/library ones).  Shared by [`symbol_at`] and [`lookup`].
fn hover_of_def(data: &Data, d: u32, text: &str, name: &str, stdlib_dir: &str) -> Option<Hover> {
    let (kind, cname) = crate::api_surface::classify(data, d)?;
    let pos = data.def(d).position.clone();
    // Read the definition's own source ONCE — for the `///` doc AND to locate the
    // name (the parser records `pos` at the body start, past the name).
    let src = read_def_source(text, name, stdlib_dir, &pos);
    let doc = src
        .as_deref()
        .map_or_else(Vec::new, |s| doc_block_above(s, pos.line));
    let def_col = src
        .as_deref()
        .and_then(|s| name_col_on_line(s, pos.line, &cname))
        .unwrap_or(pos.pos);
    Some(Hover {
        signature: render_signature(data, d, kind, &cname),
        name: cname,
        doc,
        def_file: collapse_slashes(&pos.file),
        def_line: pos.line,
        def_col,
    })
}

/// @PLN115 — resolve the identifier under a 1-based `(line, col)` cursor via the
/// PARSE-TIME resolution index, returning a [`Hover`] (signature + location).
///
/// Position-precise, unlike the name-based [`symbol_at`]: it resolves LOCALS and
/// METHODS (which `symbol_at` documents it cannot) and distinguishes a local `x`
/// from a field `x`, and `text.len` from `vector.len` — because the parser recorded
/// exactly what each occurrence binds to.  `Global`/`Method` reuse [`hover_of_def`]
/// (the target's real signature + `///` doc); a `Local` synthesizes `name: type`
/// at its declaration; a `Field` shows `Type.field: type` at the containing struct.
/// `None` when the cursor is not on a recorded occurrence (e.g. sitting on a
/// definition's OWN name) — the caller then falls back to `symbol_at`.
#[must_use]
pub fn resolve_at(text: &str, stdlib_dir: &str, line: u32, col: u32) -> Option<Hover> {
    let p = parse_lsp_buffer(text, "buf.loft", stdlib_dir);
    let occ = p
        .resolutions()
        .iter()
        .find(|o| o.line == line && col >= o.col && col <= o.col + u32::from(o.len))?;
    match occ.res {
        crate::resolution::Resolution::Global(d)
        | crate::resolution::Resolution::Method { method_def: d, .. } => {
            hover_of_def(&p.data, d, text, "buf.loft", stdlib_dir)
        }
        crate::resolution::Resolution::Local { fn_def, var_nr } => {
            if fn_def >= p.data.definitions() {
                return None;
            }
            let vars = p.data.def(fn_def).variables();
            let vname = vars.name(var_nr).to_string();
            let sig = format!("{vname}: {}", p.data.type_name_str(vars.tp(var_nr)));
            // The declaration is the binding's earliest occurrence in the buffer.
            let decl = p
                .resolutions()
                .iter()
                .filter(|o| {
                    matches!(o.res, crate::resolution::Resolution::Local { fn_def: f, var_nr: v }
                        if f == fn_def && v == var_nr)
                })
                .min_by(|a, b| a.line.cmp(&b.line).then(a.col.cmp(&b.col)))?;
            Some(Hover {
                name: vname,
                signature: sig,
                doc: Vec::new(),
                def_file: "buf.loft".to_string(),
                def_line: decl.line,
                def_col: decl.col,
            })
        }
        crate::resolution::Resolution::Field { type_def, attr } => {
            if type_def >= p.data.definitions() {
                return None;
            }
            let attr = attr as usize;
            let fname = p.data.attr_name(type_def, attr);
            let tname = p.data.def(type_def).name().to_string();
            let sig = format!(
                "{tname}.{fname}: {}",
                p.data.type_name_str(&p.data.attr_type(type_def, attr))
            );
            let pos = p.data.def(type_def).position.clone();
            Some(Hover {
                name: fname,
                signature: sig,
                doc: Vec::new(),
                def_file: collapse_slashes(&pos.file),
                def_line: pos.line,
                def_col: pos.pos,
            })
        }
        crate::resolution::Resolution::Unresolved => None,
    }
}

/// Collapse repeated `/` in a path.  The stdlib startup-cache can bake a `//`
/// into a def's recorded file (a trailing-separator base dir joined with a
/// name), and a doubled slash in a `file:line` reference reads as unpolished.
fn collapse_slashes(p: &str) -> String {
    let mut out = String::with_capacity(p.len());
    let mut prev_slash = false;
    for c in p.chars() {
        if c == '/' && prev_slash {
            continue;
        }
        prev_slash = c == '/';
        out.push(c);
    }
    out
}

/// `<keyword> <name><body>` — the signature body from `api_surface::signature_of`
/// joined with the right spacing (fn/const bodies open with `(`/`:`, no space; the
/// rest open with `{`/`=`, one space).
fn render_signature(data: &Data, d: u32, kind: &str, name: &str) -> String {
    let body = crate::api_surface::signature_of(data, d, kind);
    let kw = match kind {
        "fn" | "method" => "fn",
        "struct" => "struct",
        "enum" => "enum",
        "typedef" => "type",
        "constant" => "const",
        "interface" => "interface",
        other => other, // "operator" and any future label render as-is
    };
    match kind {
        "fn" | "method" | "operator" | "constant" => format!("{kw} {name}{body}"),
        _ => format!("{kw} {name} {body}"),
    }
}

/// The definition's own source text: the open buffer when `pos` points into it,
/// else the file on disk (stdlib / imported libs).  `pos.file` is repo-root
/// relative (e.g. `default/01_code.loft`); the stdlib root is the parent of
/// `stdlib_dir` (`…/default`).  `None` when the source can't be read.
fn read_def_source(buf: &str, buf_name: &str, stdlib_dir: &str, pos: &Position) -> Option<String> {
    if pos.file == buf_name {
        return Some(buf.to_string());
    }
    let root = Path::new(stdlib_dir)
        .parent()
        .map_or_else(|| Path::new("").to_path_buf(), Path::to_path_buf);
    std::fs::read_to_string(root.join(&pos.file)).ok()
}

/// The contiguous comment block directly above the declaration on `decl_line`
/// (1-based), in reading order.  Empty when there is none.
///
/// **Both `///` and `//` count.**  `///` is the Rust-shaped spelling and reads as a doc
/// comment to anyone arriving from there, but the loft sources overwhelmingly use plain
/// `//` — measured across the distribution, 936 `pub fn` are documented that way against 54
/// with `///`.  Recognising only `///` therefore reported almost the whole public surface as
/// undocumented: the library review counted 334 `pub fn` "carrying no doc comment" when the
/// project's own baseline in [API_SURFACE.md] says 44 of the stdlib's user-facing functions
/// have none — and 44 is exactly the count of `pub fn` with NO comment above them at all.
/// The sources were right and the reader was wrong.
///
/// A SECTION MARKER (`// ---  JSON  ---`) is not documentation and stops the block: it names
/// the group below it, so folding it into the first function's doc would attach a heading to
/// one arbitrary member.  That is also the one shape a plain-`//` reading could plausibly get
/// wrong, which is why it is excluded here rather than left to chance.
fn doc_block_above(src: &str, decl_line: u32) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let mut doc: Vec<String> = Vec::new();
    // The line above the declaration is index `decl_line - 2`.
    let mut i = decl_line as isize - 2;
    // A BLANK line between the comment and the declaration does not end the block.  The
    // `///` convention forbids one, but loft sources routinely leave a blank there
    // (`default/01_code.loft`'s `exp` is written marker / comment / blank / `pub fn`), and
    // breaking on it silently dropped those docs the same way `//` did.  Only blanks are
    // skipped: code, a `}` or an attribute line still ends the block, so a comment belonging
    // to the PREVIOUS declaration can never be pulled onto this one.
    while i >= 0 && lines[i as usize].trim().is_empty() {
        i -= 1;
    }
    while i >= 0 {
        let trimmed = lines[i as usize].trim_start();
        if is_section_marker(trimmed) {
            break;
        }
        let Some(rest) = trimmed
            .strip_prefix("///")
            .or_else(|| trimmed.strip_prefix("//"))
        else {
            break;
        };
        doc.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
        i -= 1;
    }
    doc.reverse();
    // A block that is only blank comment lines is not documentation, and would otherwise
    // count as present — the exact failure this reader exists to stop, one level in.
    while doc.first().is_some_and(|l| l.trim().is_empty()) {
        doc.remove(0);
    }
    while doc.last().is_some_and(|l| l.trim().is_empty()) {
        doc.pop();
    }
    doc
}

/// Is this comment line a section HEADING rather than prose?
///
/// Two spellings are in use and both must be caught, because a heading swept into the block
/// becomes the first sentence of one arbitrary member's documentation:
///
/// ```text
/// // --- Collections ---
/// // ── Palette ──────────────────────────────
/// ```
///
/// The second is why this is a rule rather than a literal test: catching only `---` let
/// `palette_r`'s doc begin *"── Palette ───…"*.  So the shape is **a comment that opens with
/// a run of two or more RULE characters** — hyphen or box-drawing — which no prose sentence
/// does and every heading here does.
fn is_section_marker(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("//") else {
        return false;
    };
    let rest = rest.trim_start();
    let rule = |c: char| matches!(c, '-' | '─' | '=' | '━' | '~' | '*' | '#');
    rest.chars().take_while(|c| rule(*c)).count() >= 2
}

/// The 1-based column of `name` on `decl_line` (1-based) in `src`, so a jump
/// lands on the name rather than the parser's body-start position.  A method
/// name (`Type.method`) is searched by its last segment.  `None` if not found
/// on that line (e.g. a multi-line signature) — the caller keeps `pos.pos`.
fn name_col_on_line(src: &str, decl_line: u32, name: &str) -> Option<u32> {
    let needle = name.rsplit('.').next().unwrap_or(name);
    let line = src.lines().nth(decl_line.saturating_sub(1) as usize)?;
    let byte_idx = line.find(needle)?;
    Some(line[..byte_idx].chars().count() as u32 + 1)
}

/// The identifier under a 1-based (`line`, `col`) cursor — the token that
/// find-references / rename resolve.  Public wrapper over `identifier_span_at`.
#[must_use]
pub fn identifier_at(text: &str, line: u32, col: u32) -> Option<String> {
    identifier_span_at(text, line, col).map(|(name, ..)| name)
}

/// The identifier under a 1-based (`line`, `col`) cursor as `(name, start_col,
/// end_col)` — the cols 0-based — or `None` if not on a word.  Right-edge
/// anchoring: a cursor just past a token still selects it.  `prepareRename` uses
/// the span for the highlight range.
#[must_use]
pub fn identifier_span_at(text: &str, line: u32, col: u32) -> Option<(String, u32, u32)> {
    let line_str = text.lines().nth(line.saturating_sub(1) as usize)?;
    let chars: Vec<char> = line_str.chars().collect();
    if chars.is_empty() {
        return None;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut anchor = (col.saturating_sub(1) as usize).min(chars.len() - 1);
    if !is_word(chars[anchor]) {
        if anchor == 0 || !is_word(chars[anchor - 1]) {
            return None;
        }
        anchor -= 1;
    }
    let mut start = anchor;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = anchor + 1;
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    Some((chars[start..end].iter().collect(), start as u32, end as u32))
}

fn word_at(text: &str, line: u32, col: u32) -> Option<String> {
    identifier_at(text, line, col)
}

/// The loft source formatter (`tools/fmt/whole.loft`) compiled to a runnable
/// program — the SAME formatter the `loft fmt` CLI runs, so the LSP and the CLI
/// produce identical output.  Compiling loads the stdlib + the formatter program
/// and is heavy (~hundreds of ms), so build ONE and reuse it across requests.
pub struct Formatter {
    prog: Program,
}

impl Formatter {
    /// Compile the formatter with the stdlib in `stdlib_dir`.  `None` if it can't
    /// be built (e.g. the stdlib can't be found or loaded).
    #[must_use]
    pub fn new(stdlib_dir: &str) -> Option<Formatter> {
        // Relative to THIS file (`src/lsp.rs`); the same `include_str!` target the
        // CLI's `run_fmt_command` uses, so there is one formatter source of truth.
        const FMT_SRC: &str = include_str!("../tools/fmt/whole.loft");
        Program::from_source_with_stdlib(FMT_SRC, stdlib_dir)
            .ok()
            .map(|prog| Formatter { prog })
    }

    /// Format `text`, or `None` if the formatter itself errors (never on a
    /// no-op — a well-formatted buffer returns itself unchanged).
    pub fn format(&mut self, text: &str) -> Option<String> {
        self.prog
            .call("format", &[Value::Text(text.to_string())])
            .ok()?
            .into_text()
            .ok()
    }
}

// ── tracker-tag integration (@PLN63 T1/T2) ───────────────────────────────────
// Surface loft's own `@`-tracker system (issues / features / plans) inline,
// reading the ALREADY-generated `index/tags.json` + `index/features.json`
// (`make index`).  Lights up in the loft repo; inert elsewhere (files absent).

/// One tracker tag's information, gathered from the generated index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagInfo {
    /// The tag as written, e.g. `@F7`.
    pub tag: String,
    /// Display kind: `feature` `infra` `problem` `plan` `github` `plan-legacy` `tag`.
    pub kind: &'static str,
    /// The feature/infra title (`@F`/`@I` only), from `features.json`.
    pub title: Option<String>,
    /// A short description — the feature body's first paragraph, else the first
    /// indexed reference's source line.
    pub summary: Option<String>,
    /// The canonical issue URL (`@GH`/`@PLN`/`@F`/`@I`).
    pub url: Option<String>,
    /// How many references the index has for this tag.
    pub references: usize,
}

/// The generated tracker index (`index/tags.json` + `index/features.json`),
/// parsed once and queried per tag.  `load` returns `None` outside the loft repo
/// (the files are absent) — the caller treats that as "no tag info", not an error.
pub struct TagIndex {
    tags: Parsed,
    features: Parsed,
    /// Tags the scanner deemed BROKEN (its `broken` array — `@P`/`@PLAN` that
    /// resolve to no valid PID/plan).  The scanner is the authority (it reads
    /// PROBLEMS.md + the plan dirs + the freeze-banner map); we consume its
    /// verdict rather than re-deriving it.
    broken: Vec<String>,
}

impl TagIndex {
    /// Parse `<index_dir>/tags.json` (required) + `features.json` (optional).
    #[must_use]
    pub fn load(index_dir: &str) -> Option<TagIndex> {
        let tags_txt = std::fs::read_to_string(Path::new(index_dir).join("tags.json")).ok()?;
        let tags = crate::json::parse(&tags_txt).ok()?;
        let features = std::fs::read_to_string(Path::new(index_dir).join("features.json"))
            .ok()
            .and_then(|s| crate::json::parse(&s).ok())
            .unwrap_or(Parsed::Array(Vec::new()));
        // The `broken` array is `[{"tag":"@P999","refs":[…]}, …]`. <!--noindex-->

        let broken = match pj_get(&tags, "broken") {
            Some(Parsed::Array(items)) => items.iter().filter_map(|it| pj_str(it, "tag")).collect(),
            _ => Vec::new(),
        };
        Some(TagIndex {
            tags,
            features,
            broken,
        })
    }

    /// Whether the scanner flagged `tag` as broken — a `@P`/`@PLAN` reference to
    /// no valid issue/plan.  From the index's `broken` array, so it inherits ALL
    /// the scanner's validation (PROBLEMS.md, plan dirs, the freeze-banner map)
    /// with no re-derivation.  NOTE: a freshly-typed broken tag not yet indexed
    /// won't show until the next `make index` — the index is the source of truth.
    #[must_use]
    pub fn is_broken(&self, tag: &str) -> bool {
        self.broken.iter().any(|t| t == tag)
    }

    /// Everything the index knows about `tag`, or `None` if it is neither
    /// referenced nor a well-formed issue tag (a probable typo → T3 territory).
    #[must_use]
    pub fn lookup(&self, tag: &str) -> Option<TagInfo> {
        let (fam, num) = split_tag(tag)?;
        let refs = pj_get(&self.tags, tag);
        let references = match refs {
            Some(Parsed::Array(a)) => a.len(),
            _ => 0,
        };
        // Deterministic issue URLs per tag family (the repos in CLAUDE.md).
        let (kind, url) = match fam {
            "GH" => (
                "github",
                Some(format!("https://github.com/loft-lang/loft/issues/{num}")),
            ),
            "PLN" => (
                "plan",
                Some(format!("https://github.com/loft-lang/plans/issues/{num}")),
            ),
            "F" => (
                "feature",
                Some(format!(
                    "https://github.com/loft-lang/features/issues/{num}"
                )),
            ),
            "I" => (
                "infra",
                Some(format!(
                    "https://github.com/loft-lang/features/issues/{num}"
                )),
            ),
            "P" => ("problem", None),
            "PLAN" => ("plan-legacy", None),
            _ => ("tag", None),
        };
        let (title, summary) = if fam == "F" || fam == "I" {
            match self.feature(num) {
                Some(f) => (
                    pj_str(f, "title"),
                    pj_str(f, "body").map(|b| first_para(&b)),
                ),
                None => (None, first_ref_context(refs)),
            }
        } else {
            (None, first_ref_context(refs))
        };
        // A local tag (`@P`/`@PLAN`) with no references and no title is unknown.
        if references == 0 && title.is_none() && url.is_none() {
            return None;
        }
        Some(TagInfo {
            tag: tag.to_string(),
            kind,
            title,
            summary,
            url,
            references,
        })
    }

    fn feature(&self, num: i64) -> Option<&Parsed> {
        match &self.features {
            Parsed::Array(items) => items.iter().find(|it| pj_i64(it, "number") == Some(num)),
            _ => None,
        }
    }

    /// T4 — completion candidates for a partial tracker tag (`@`, `@P`, `@PLN6`):
    /// every known tag whose text starts with `prefix`, as `Completion`s (kind
    /// Reference).  Known tags are the referenced tags in `tags.json` (never a
    /// broken-only one — those live in the `broken` array, not as keys) plus the
    /// `@F<n>` / `@I<n>` feature / infra tags synthesized from `features.json`.
    /// The detail is the feature title, else the tag's kind label.  Capped at 200.
    #[must_use]
    pub fn complete(&self, prefix: &str) -> Vec<Completion> {
        let mut tags: Vec<String> = Vec::new();
        // Referenced tags from tags.json (skip the `broken` meta key).
        if let Parsed::Object(entries) = &self.tags {
            for (k, _, _) in entries {
                if k != "broken" && k.starts_with('@') {
                    tags.push(k.clone());
                }
            }
        }
        // Feature / infra tags synthesized from features.json (`kind` → family).
        if let Parsed::Array(items) = &self.features {
            for it in items {
                if let (Some(num), Some(kind)) = (pj_i64(it, "number"), pj_str(it, "kind")) {
                    let fam = match kind.as_str() {
                        "feature" => "F",
                        "infra" => "I",
                        _ => continue,
                    };
                    tags.push(format!("@{fam}{num}"));
                }
            }
        }
        tags.sort();
        tags.dedup();
        let mut out: Vec<Completion> = tags
            .into_iter()
            .filter(|t| t.starts_with(prefix))
            .filter_map(|t| {
                let info = self.lookup(&t)?;
                Some(Completion {
                    kind: 18, // Reference
                    detail: info.title.unwrap_or_else(|| info.kind.to_string()),
                    label: t,
                })
            })
            .collect();
        out.truncate(200);
        out
    }
}

/// A [`TagInfo`] as a markdown block — the LSP hover body and `loft tag` output.
#[must_use]
pub fn render_tag_markdown(info: &TagInfo) -> String {
    use std::fmt::Write;
    let mut s = format!("**{}** · {}", info.tag, info.kind);
    if let Some(url) = &info.url {
        let _ = write!(s, "  ·  [issue]({url})");
    }
    if let Some(title) = &info.title {
        let _ = write!(s, "\n\n**{title}**");
    }
    if let Some(summary) = &info.summary {
        let _ = write!(s, "\n\n{summary}");
    }
    let plural = if info.references == 1 {
        "reference"
    } else {
        "references"
    };
    let _ = write!(s, "\n\n_{} {plural} indexed_", info.references);
    s
}

/// Split `@F7` → `("F", 7)`; `None` if not a well-formed `@<UPPER>+<digits>` tag.
fn split_tag(tag: &str) -> Option<(&str, i64)> {
    let body = tag.strip_prefix('@')?;
    let split = body.find(|c: char| c.is_ascii_digit())?;
    let (fam, rest) = body.split_at(split);
    if fam.is_empty() || !fam.chars().all(|c| c.is_ascii_uppercase()) {
        return None;
    }
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let num = digits.parse::<i64>().ok()?;
    Some((fam, num))
}

/// The first real paragraph of a feature body — skip headings / code fences.
fn first_para(body: &str) -> String {
    for para in body.split("\n\n") {
        let t = para.trim();
        if !t.is_empty() && !t.starts_with('#') && !t.starts_with("```") {
            return t.replace('\n', " ");
        }
    }
    String::new()
}

fn first_ref_context(refs: Option<&Parsed>) -> Option<String> {
    match refs {
        Some(Parsed::Array(a)) => a.first().and_then(|r| pj_str(r, "context")).map(|c| {
            let t = c.trim();
            t.chars().take(200).collect()
        }),
        _ => None,
    }
}

/// Tracker tags in one line as `(tag, start_col, end_col)` — 0-based char cols.
/// A tag is `@` + uppercase letters + digits, with an optional single lowercase
/// suffix (`@P259a`).  Deep segment forms (`@PLAN22-2d`) resolve to their base.
fn tags_in_line(line: &str) -> Vec<(String, usize, usize)> {
    let chars: Vec<char> = line.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '@' {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i + 1;
        let alpha0 = j;
        while j < chars.len() && chars[j].is_ascii_uppercase() {
            j += 1;
        }
        let digit0 = j;
        while j < chars.len() && chars[j].is_ascii_digit() {
            j += 1;
        }
        if j > digit0 && digit0 > alpha0 {
            // A single trailing lowercase (e.g. `@P259a`) when word-bounded.
            if j < chars.len()
                && chars[j].is_ascii_lowercase()
                && (j + 1 >= chars.len() || !chars[j + 1].is_ascii_alphanumeric())
            {
                j += 1;
            }
            out.push((chars[start..j].iter().collect(), start, j));
            i = j;
        } else {
            i += 1;
        }
    }
    out
}

/// The tracker tag under a 1-based (`line`, `col`) cursor, or `None`.
#[must_use]
pub fn tag_at(text: &str, line: u32, col: u32) -> Option<String> {
    let line_str = text.lines().nth(line.saturating_sub(1) as usize)?;
    let c = col.saturating_sub(1) as usize;
    tags_in_line(line_str)
        .into_iter()
        .find(|(_, s, e)| *s <= c && c < *e)
        .map(|(t, _, _)| t)
}

/// T4 — the partial tracker tag being typed at a 1-based (`line`, `col`) cursor:
/// an `@` followed by the uppercase-letter + digit run up to the cursor (`@`,
/// `@P`, `@PLN6`), or `None` when the cursor is not in a tag.  `@` never starts a
/// loft identifier, so a `Some` here means the completion handler should offer
/// tracker-tag candidates ([`TagIndex::complete`]) instead of symbols.
#[must_use]
pub fn tag_completion_prefix(text: &str, line: u32, col: u32) -> Option<String> {
    let line_str = text.lines().nth(line.saturating_sub(1) as usize)?;
    let chars: Vec<char> = line_str.chars().collect();
    let cursor = (col.saturating_sub(1) as usize).min(chars.len());
    let mut i = cursor;
    while i > 0 && (chars[i - 1].is_ascii_uppercase() || chars[i - 1].is_ascii_digit()) {
        i -= 1;
    }
    (i > 0 && chars[i - 1] == '@').then(|| chars[i - 1..cursor].iter().collect())
}

/// Every tag occurrence in `text` as `(tag, line, start_col, end_col)` — 1-based
/// line, 0-based cols — for `documentLink`.
#[must_use]
pub fn tags_in(text: &str) -> Vec<(String, u32, u32, u32)> {
    let mut out = Vec::new();
    for (i, line) in text.lines().enumerate() {
        for (tag, s, e) in tags_in_line(line) {
            out.push((tag, i as u32 + 1, s as u32, e as u32));
        }
    }
    out
}

// ── Parsed accessors (over loft::json) ───────────────────────────────────────
fn pj_get<'a>(v: &'a Parsed, key: &str) -> Option<&'a Parsed> {
    match v {
        Parsed::Object(e) => e.iter().find(|(k, _, _)| k == key).map(|(_, _, val)| val),
        _ => None,
    }
}

fn pj_str(v: &Parsed, key: &str) -> Option<String> {
    match pj_get(v, key) {
        Some(Parsed::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

fn pj_i64(v: &Parsed, key: &str) -> Option<i64> {
    pj_get(v, key).and_then(Parsed::as_i64)
}

// ── workspace reverse index (@PLN63 — find-references / rename) ───────────────
// A workspace-wide reverse index: every identifier's occurrences across the
// `.loft` files under a root, keyed by name.  Built by LEXING each file (loft's
// own lexer), so comments / strings / keywords are excluded and `x.len` yields a
// `len` identifier — a precise-but-unscoped finder (name-keyed, not type-resolved:
// `references("len")` returns every `len` token across all types).

/// One occurrence of an identifier in the workspace — a use site or a definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    /// Absolute file path (canonicalized).
    pub file: String,
    /// 1-based line.
    pub line: u32,
    /// 1-based column.
    pub col: u32,
}

/// A reverse index of identifier occurrences across a workspace's `.loft` files.
pub struct WorkspaceIndex {
    by_name: HashMap<String, Vec<Reference>>,
}

impl WorkspaceIndex {
    /// Build by lexing every `.loft` file under `root` (skipping build / VCS /
    /// dependency dirs).  Reflects the SAVED files; the caller overlays open,
    /// edited buffers at query time (`identifier_refs`).
    #[must_use]
    pub fn build(root: &str) -> WorkspaceIndex {
        let mut by_name: HashMap<String, Vec<Reference>> = HashMap::new();
        for path in loft_files(Path::new(root)) {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let file = canonical(&path);
            for (name, r) in scan_identifiers(&text, &file) {
                by_name.entry(name).or_default().push(r);
            }
        }
        WorkspaceIndex { by_name }
    }

    /// Every recorded occurrence of `name` (the SAVED-file view).
    #[must_use]
    pub fn references(&self, name: &str) -> &[Reference] {
        self.by_name.get(name).map_or(&[][..], Vec::as_slice)
    }

    /// References to `name` with open buffers overlaid: for each open document,
    /// its live refs replace its stale disk refs (matched by canonical path), so
    /// find-references reflects unsaved edits.  `overlays` is `(file_path, text)`
    /// per open document.  Sorted by (file, line, col).
    #[must_use]
    pub fn references_overlaid(&self, name: &str, overlays: &[(String, String)]) -> Vec<Reference> {
        let open: Vec<(String, &str)> = overlays
            .iter()
            .map(|(p, t)| (canonical(Path::new(p)), t.as_str()))
            .collect();
        let mut refs: Vec<Reference> = self
            .references(name)
            .iter()
            .filter(|r| !open.iter().any(|(p, _)| *p == r.file))
            .cloned()
            .collect();
        for (path, text) in &open {
            refs.extend(identifier_refs(text, path, name));
        }
        refs.sort_by(|a, b| {
            a.file
                .cmp(&b.file)
                .then(a.line.cmp(&b.line))
                .then(a.col.cmp(&b.col))
        });
        refs
    }
}

/// Every occurrence of the identifier `name` in `text`, tagged with `file`.  A
/// fresh lex — the overlay for an open, edited buffer (its live refs replace the
/// stale disk ones).
#[must_use]
pub fn identifier_refs(text: &str, file: &str, name: &str) -> Vec<Reference> {
    scan_identifiers(text, file)
        .into_iter()
        .filter(|(n, _)| n == name)
        .map(|(_, r)| r)
        .collect()
}

/// Lex `text` and collect every non-keyword identifier as `(name, Reference)`.
fn scan_identifiers(text: &str, file: &str) -> Vec<(String, Reference)> {
    use crate::lexer::{LexItem, Lexer};
    let mut lexer = Lexer::default();
    lexer.parse_string(text, file);
    let mut out = Vec::new();
    // `restart()` (inside `parse_string`) already advanced to the first token,
    // so peek FIRST, then `cont()`.
    loop {
        let r = lexer.peek();
        match r.has {
            LexItem::None => break,
            LexItem::Identifier(name) if !crate::lexer::is_keyword(&name) => {
                out.push((
                    name,
                    Reference {
                        file: file.to_string(),
                        line: r.position.line,
                        col: r.position.pos,
                    },
                ));
            }
            _ => {}
        }
        lexer.cont();
    }
    out
}

/// Canonical absolute path string (falls back to the lossy path on error).
fn canonical(p: &Path) -> String {
    crate::portable_path::plain_canonical(p)
        .to_string_lossy()
        .into_owned()
}

/// A path as a consumer expects to see it, with Windows' extended-length prefix
/// removed.
///
/// `std::fs::canonicalize` returns a VERBATIM path on Windows — `\\?\D:\src\a.loft`,
/// or `\\?\UNC\server\share\…` for a network path.  That prefix is an OS detail for
/// bypassing the legacy MAX_PATH limit, and it leaks: it reached the `file` field of
/// every `loft def --json` result and every LSP `file://` URI, where editors do not
/// accept it.  Strip it so a path means the same thing to a consumer on every platform.
/// Non-Windows paths pass through untouched.
pub fn plain_path(p: &Path) -> String {
    crate::portable_path::strip_verbatim(&p.to_string_lossy())
}

/// `true` when `s` begins with a Windows drive-letter prefix (`C:`).
fn drive_prefixed(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// The `file://` URI for a filesystem path, in the shape editors accept on EVERY
/// platform — the counterpart of [`uri_to_path`].
///
/// Windows `D:\a\b` → `file:///D:/a/b`; POSIX `/a/b` → `file:///a/b`. It strips the
/// `\\?\` verbatim prefix (via [`plain_path`]), renders separators as `/`, and adds
/// the third slash before a drive letter.
///
/// The naive `format!("file://{}", p.display())` gets all three wrong on Windows —
/// and its **backslashes are invalid JSON string escapes** (`\U`, `\r`, `\t`), so
/// embedding such a URI in an LSP message silently corrupts it: a strict parser
/// rejects the whole message. That is not a cosmetic bug — it is why an editor (or
/// a test) that sent a Windows `rootUri` this way got no `initialize` reply and hung.
/// Every path→URI in the LSP surface MUST go through here.
#[must_use]
pub fn path_to_uri(p: &Path) -> String {
    let plain = crate::portable_path::for_uri(std::path::Path::new(&plain_path(p)));
    if plain.starts_with('/') {
        format!("file://{plain}")
    } else {
        format!("file:///{plain}")
    }
}

/// The filesystem path a `file://` URI denotes — the inverse of [`path_to_uri`] on
/// every platform.
///
/// `file:///D:/a/b` → `D:\a\b` on Windows (drops the spurious leading slash before
/// the drive letter and uses native `\` separators); `file:///a/b` → `/a/b` on
/// POSIX. The result matches the form [`WorkspaceIndex`] stores ([`plain_path`] of a
/// canonicalized path), so a URI and a walked path compare equal for the same file.
/// A non-`file://` string passes through unchanged.
#[must_use]
pub fn uri_to_path(uri: &str) -> String {
    // Only a `file://` URI names a filesystem path; anything else passes through
    // UNCHANGED — never rewrite its separators (a Windows `/`→`\` sweep across the
    // whole string turned `stdin://x` into `stdin:\x`).
    let Some(rest) = uri.strip_prefix("file://") else {
        return uri.to_string();
    };
    // `file:///D:/…` leaves `/D:/…`; the leading slash before a drive is spurious.
    let rest = match rest.strip_prefix('/') {
        Some(after) if drive_prefixed(after) => after,
        _ => rest,
    };
    if cfg!(windows) {
        rest.replace('/', "\\")
    } else {
        rest.to_string()
    }
}

/// Every `.loft` file under `root`, skipping build / VCS / dependency dirs.
fn loft_files(root: &Path) -> Vec<PathBuf> {
    const SKIP: &[&str] = &[
        "target",
        ".git",
        ".loft",
        "node_modules",
        ".claude",
        ".cache",
    ];
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if path.is_dir() {
                if !SKIP.contains(&name.as_ref()) && !name.starts_with('.') {
                    stack.push(path);
                }
            } else if name.ends_with(".loft") {
                out.push(path);
            }
        }
    }
    out
}

// ── rename (on top of the reverse index) ─────────────────────────────────────

/// Whether `name` is a syntactically valid loft identifier (and not a keyword) —
/// the guard on a rename's NEW name.
#[must_use]
pub fn is_valid_identifier(name: &str) -> bool {
    let mut cs = name.chars();
    if !matches!(cs.next(), Some(c) if c.is_ascii_alphabetic() || c == '_') {
        return false;
    }
    name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && !crate::lexer::is_keyword(name)
}

/// A stdlib source path (under a `default/` dir) — files a rename must never edit.
fn is_stdlib_path(file: &str) -> bool {
    file.contains("/default/") || file.ends_with("/default")
}

/// For `prepareRename`: the identifier's span at the cursor IF it is renamable —
/// on an identifier that is NOT a standard-library symbol.  `None` otherwise, so
/// the editor won't offer to rename a stdlib name.
#[must_use]
pub fn prepare_rename(
    text: &str,
    line: u32,
    col: u32,
    stdlib_dir: &str,
) -> Option<(String, u32, u32)> {
    let span = identifier_span_at(text, line, col)?;
    if !lookup(&span.0, "", "query.loft", stdlib_dir).is_empty() {
        return None; // a stdlib symbol — not renamable
    }
    Some(span)
}

/// Plan a workspace-wide rename of `old` → `new`: the reference locations to
/// rewrite, or an error message explaining why the rename is refused.
///
/// Guards (SAFE by default): the new name must be a valid identifier; the old
/// name must not resolve to a **standard-library** symbol (`print`, `len`, …);
/// and no reference may live in a stdlib file — renaming those would corrupt the
/// shared library.  NOTE: lexical / name-keyed (like find-references), so it best
/// fits GLOBAL user symbols whose name is unique across the workspace; a local
/// variable or a name reused across scopes is renamed everywhere it appears.
///
/// # Errors
/// Returns a human-readable message when the rename is refused: the new name is
/// not a valid identifier, `old` is a stdlib symbol or is referenced by stdlib
/// files, or there are no references to `old`.
pub fn plan_rename(
    old: &str,
    new: &str,
    index: &WorkspaceIndex,
    overlays: &[(String, String)],
    stdlib_dir: &str,
) -> Result<Vec<Reference>, String> {
    if !is_valid_identifier(new) {
        return Err(format!("`{new}` is not a valid identifier"));
    }
    if new == old {
        return Ok(Vec::new());
    }
    if !lookup(old, "", "query.loft", stdlib_dir).is_empty() {
        return Err(format!(
            "`{old}` is a standard-library symbol — refusing to rename it"
        ));
    }
    let refs = index.references_overlaid(old, overlays);
    if refs.iter().any(|r| is_stdlib_path(&r.file)) {
        return Err(format!(
            "`{old}` is used by the standard library — refusing to rename it"
        ));
    }
    if refs.is_empty() {
        return Err(format!("no references to `{old}` in the workspace"));
    }
    Ok(refs)
}

/// True when the identifier occurrence `o` is the target of a plain `=` assignment
/// — for an assignment-local, its DECLARATION (loft creates the local at its first
/// write).  The next non-space char after the name is `=` and not `==`.  Used to
/// confirm a binding's declaration is captured before trusting the index for rename.
fn occurrence_is_assignment(text: &str, line_no: u32, col: u32, len: u16) -> bool {
    let Some(line) = text.lines().nth(line_no.saturating_sub(1) as usize) else {
        return false;
    };
    let chars: Vec<char> = line.chars().collect();
    let mut i = (col.saturating_sub(1) + u32::from(len)) as usize; // just past the name
    while chars.get(i) == Some(&' ') || chars.get(i) == Some(&'\t') {
        i += 1;
    }
    chars.get(i) == Some(&'=') && chars.get(i + 1) != Some(&'=')
}

/// @PLN115 S4 — the exact occurrences of the LOCAL binding under a 1-based
/// `(line, col)` cursor, resolved by binding IDENTITY from the parse-time
/// resolution index — or `None` when the precise path is not SOUND for this
/// binding, telling the caller to fall back to the F-v1 name-scan.
///
/// Precise because it keys on `(fn_def, var_nr)`, so a same-spelled FIELD access
/// (`p.x` vs local `x`), method, or global is excluded — the name-scan can't do
/// that.  loft is flat-scoped per function, so a binding's occurrences are all in
/// the current buffer; the result needs no workspace index.
///
/// SOUNDNESS: a rename must edit the binding's DECLARATION, so the precise path is
/// taken only when the recorded occurrence set INCLUDES it.  That holds for an
/// assignment-local (its earliest occurrence is a declaring `name =` write) and —
/// via the @PLN115 tail — for a PARAMETER (its signature name is recorded with the
/// `declaration` flag).  A binder the parser does not yet record a declaration for
/// (a `for i` / lambda binder) still returns `None`, so the caller falls back to
/// the F-v1 name-scan.
#[must_use]
pub fn local_binding_refs(
    text: &str,
    stdlib_dir: &str,
    file: &str,
    line: u32,
    col: u32,
) -> Option<Vec<Reference>> {
    let p = parse_lsp_buffer(text, "buf.loft", stdlib_dir);
    let occ = p.resolutions();
    // The occurrence whose 1-based span [col, col+len] covers the cursor (a cursor
    // one past the last char still selects it, mirroring `identifier_span_at`).
    let target = occ
        .iter()
        .find(|o| o.line == line && col >= o.col && col <= o.col + u32::from(o.len))?;
    let crate::resolution::Resolution::Local { fn_def, var_nr } = target.res else {
        return None;
    };
    let mut mine: Vec<&crate::resolution::Occurrence> = occ
        .iter()
        .filter(|o| {
            matches!(o.res, crate::resolution::Resolution::Local { fn_def: f, var_nr: v }
                if f == fn_def && v == var_nr)
        })
        .collect();
    mine.sort_by(|a, b| a.line.cmp(&b.line).then(a.col.cmp(&b.col)));
    // SOUNDNESS: the precise path is complete only when the binding's DECLARATION is
    // in the set, so a rename edits it too.  That holds when the index recorded the
    // declaration (a parameter signature or `for` / lambda binder — `declaration`),
    // or the earliest occurrence is a declaring `name =` write (an assignment-local).
    // A binder whose declaration the parser does not yet record → incomplete set →
    // `None`, and the caller uses the F-v1 name-scan.
    let first = mine.first()?;
    let has_declaration = mine.iter().any(|o| o.declaration)
        || occurrence_is_assignment(text, first.line, first.col, first.len);
    if !has_declaration {
        return None;
    }
    Some(
        mine.iter()
            .map(|o| Reference {
                file: file.to_string(),
                line: o.line,
                col: o.col,
            })
            .collect(),
    )
}

/// @PLN115 — every workspace occurrence of the METHOD under a 1-based `(line, col)`
/// cursor, resolved through the index so a same-named method of another receiver
/// type (`vector.len` vs `text.len`) is EXCLUDED — which the name-scan cannot do.
///
/// The cross-file key is the method's mangled def NAME (`t_<len><Type>_<method>`),
/// which encodes (receiver type, method) and is stable across separate parses (a
/// per-parse def_nr is not).  Every workspace `.loft` file is parsed with the index
/// (open buffers overlaid by path); a matching `Method` occurrence becomes a
/// `Reference`.  `None` when the cursor is not on a method (the caller falls back).
///
/// NOTE: each file is parsed with only the stdlib + itself, so a stdlib method
/// resolves everywhere and a user method resolves where its type is visible; a
/// cross-file user method whose definition the calling file does not import is not
/// found (under-match, never over-match).  Parsing every file makes this an
/// on-demand cost (find-references), not a per-keystroke one.
#[must_use]
pub fn method_refs(
    text: &str,
    line: u32,
    col: u32,
    root: &str,
    overlays: &[(String, String)],
    stdlib_dir: &str,
) -> Option<Vec<Reference>> {
    // 1. Resolve the method under the cursor → its stable mangled name.
    let p = parse_lsp_buffer(text, "buf.loft", stdlib_dir);
    let target = p
        .resolutions()
        .iter()
        .find(|o| o.line == line && col >= o.col && col <= o.col + u32::from(o.len))?;
    let crate::resolution::Resolution::Method { method_def, .. } = target.res else {
        return None;
    };
    if method_def >= p.data.definitions() {
        return None;
    }
    let key = p.data.def(method_def).name().to_string();

    // 2. Gather every workspace file's content, overlaying open buffers by path.
    let open: Vec<(String, &str)> = overlays
        .iter()
        .map(|(pth, t)| (canonical(Path::new(pth)), t.as_str()))
        .collect();
    let mut files: Vec<(String, String)> = Vec::new();
    for path in loft_files(Path::new(root)) {
        let cpath = canonical(&path);
        if let Some((_, t)) = open.iter().find(|(op, _)| *op == cpath) {
            files.push((cpath, (*t).to_string()));
        } else if let Ok(t) = std::fs::read_to_string(&path) {
            files.push((cpath, t));
        }
    }
    // Include an open buffer that is not on disk (an unsaved / new file).
    for (cpath, t) in &open {
        if !files.iter().any(|(fp, _)| fp == cpath) {
            files.push((cpath.clone(), (*t).to_string()));
        }
    }

    // 3. Collect the matching Method occurrences across all files.
    let mut refs = Vec::new();
    for (cpath, content) in &files {
        let fp = parse_lsp_buffer(content, "buf.loft", stdlib_dir);
        for o in fp.resolutions() {
            if let crate::resolution::Resolution::Method { method_def: md, .. } = o.res
                && md < fp.data.definitions()
                && fp.data.def(md).name() == key
            {
                refs.push(Reference {
                    file: cpath.clone(),
                    line: o.line,
                    col: o.col,
                });
            }
        }
    }
    refs.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.line.cmp(&b.line))
            .then(a.col.cmp(&b.col))
    });
    Some(refs)
}

/// One inlay hint — an inferred-type annotation shown inline after a local
/// variable's declaration.  `(line, col)` are 1-based; `col` is the position AFTER
/// the name, where `: <type>` renders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlayHint {
    pub line: u32,
    pub col: u32,
    pub label: String,
}

/// @PLN115 S7 — inferred-type inlay hints for a buffer's ASSIGNMENT-locals.  This is
/// the feature @PLN63 E could not ship: it needs each local binding's DECLARATION
/// position, and `Variable.source` was unreliable in the fresh-parse path (only the
/// first local per fn carried one).  The resolution index makes the position exact —
/// a binding's declaration is its earliest occurrence when that is a `name =` write.
///
/// The type is `def(fn_def).variables().tp(var_nr)`, rendered by `type_name_str`.
/// Parameters (explicit types in the signature) and loop / comprehension binders
/// (declared outside `parse_var`, so their earliest recorded occurrence is a use, not
/// a `name =` write) are skipped, as are unresolved types.  Reads `: <type>`.
#[must_use]
pub fn inlay_hints(text: &str, name: &str, stdlib_dir: &str) -> Vec<InlayHint> {
    let p = parse_lsp_buffer(text, name, stdlib_dir);
    let mut locals: Vec<&crate::resolution::Occurrence> = p
        .resolutions()
        .iter()
        .filter(|o| matches!(o.res, crate::resolution::Resolution::Local { .. }))
        .collect();
    locals.sort_by(|a, b| a.line.cmp(&b.line).then(a.col.cmp(&b.col)));
    let mut seen: Vec<(u32, u16)> = Vec::new();
    let mut hints = Vec::new();
    for o in locals {
        let crate::resolution::Resolution::Local { fn_def, var_nr } = o.res else {
            continue;
        };
        if seen.contains(&(fn_def, var_nr)) {
            continue; // only the declaration (earliest occurrence) gets a hint
        }
        seen.push((fn_def, var_nr));
        if fn_def >= p.data.definitions() {
            continue;
        }
        let vars = p.data.def(fn_def).variables();
        // Skip parameters (explicit types) and loop / comprehension binders (their
        // earliest recorded occurrence is a use, not the `name =` declaration).
        if vars.is_argument(var_nr) || !occurrence_is_assignment(text, o.line, o.col, o.len) {
            continue;
        }
        let tp = vars.tp(var_nr);
        if matches!(tp, Type::Unknown(_) | Type::Never | Type::Null | Type::Void) {
            continue;
        }
        let label = p.data.type_name_str(tp);
        if label.is_empty() {
            continue;
        }
        hints.push(InlayHint {
            line: o.line,
            col: o.col + u32::from(o.len),
            label: format!(": {label}"),
        });
    }
    hints
}

// ── completion (step C) ──────────────────────────────────────────────────────
// Reuses what loft already computes: `Data.definitions` + `classify` for the
// candidate names/kinds, the per-fn variable tables for `expr.` member types,
// and `lexer::keywords()`.  No new analysis — enumerate + filter.

/// One completion candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Completion {
    /// The text to insert.
    pub label: String,
    /// LSP `CompletionItemKind` (Function 3, Method 2, Variable 6, Struct 22,
    /// Enum 13, Field 5, EnumMember 20, Constant 21, Interface 8, Class 7,
    /// Keyword 14).
    pub kind: u32,
    /// A short detail (kind label or a member's type).
    pub detail: String,
}

/// Completions at a 1-based (`line`, `col`) cursor: after `expr.` → the members
/// (fields / variants / methods) of the receiver's type; otherwise the enclosing
/// function's in-scope LOCALS + PARAMETERS, then the in-scope GLOBAL names (fn /
/// struct / enum / typedef / constant / interface, methods excluded), then
/// keywords — ranked by the typed prefix (a 4+-char prefix also surfaces
/// typo'd near-misses).  Fresh parse per call.
///
/// Scope: member completion resolves the receiver as a TYPE name, or as a
/// variable via the enclosing function's table (@PLN115 scope-precise); the
/// in-scope locals come from that same enclosing function.
#[must_use]
pub fn complete(text: &str, name: &str, stdlib_dir: &str, line: u32, col: u32) -> Vec<Completion> {
    let line_text = text
        .lines()
        .nth(line.saturating_sub(1) as usize)
        .unwrap_or("");
    let (prefix, receiver) = completion_context(line_text, col.saturating_sub(1) as usize);
    let p = parse_lsp_buffer(text, name, stdlib_dir);
    let data = &p.data;
    match receiver {
        Some(recv) => member_completions(data, &recv, line),
        None => identifier_completions(data, &prefix, line),
    }
}

/// The typed prefix at the cursor, and the receiver identifier if a `.` precedes it.
fn completion_context(line: &str, col0: usize) -> (String, Option<String>) {
    let chars: Vec<char> = line.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let end = col0.min(chars.len());
    let mut start = end;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let prefix: String = chars[start..end].iter().collect();
    if start > 0 && chars[start - 1] == '.' {
        let mut rs = start - 1;
        while rs > 0 && is_word(chars[rs - 1]) {
            rs -= 1;
        }
        let receiver: String = chars[rs..start - 1].iter().collect();
        if !receiver.is_empty() {
            return (prefix, Some(receiver));
        }
    }
    (prefix, None)
}

/// In-scope identifier completions at `cursor_line`: the enclosing function's
/// LOCALS + PARAMETERS first (the names you reach for when filling a call
/// argument — @PLN115 scope-precise, and invisible to a global scan), then the
/// in-scope GLOBAL defs, then keywords.
///
/// Matching ranks a case-exact prefix above a case-insensitive one; a 4+-char
/// prefix additionally surfaces near-misses within edit distance 2 (a typo'd
/// `prnt` still offers `print`), ranked strictly after every prefix match so the
/// exact ones lead.  An empty prefix keeps the enclosing locals + the user's own
/// globals + keywords, never the whole stdlib.
fn identifier_completions(data: &Data, prefix: &str, cursor_line: u32) -> Vec<Completion> {
    let lower = prefix.to_lowercase();
    // Match rank for a candidate label — lower is better, `None` excludes it.  An
    // empty prefix keeps everything (rank 0); short prefixes never fuzzy-match (too
    // noisy, cf. `suggest_similar_capped`); a 4+-char prefix falls back to a
    // Levenshtein-≤2 match ranked strictly after all prefix matches.
    let rank = |label: &str| -> Option<u32> {
        if prefix.is_empty() {
            return Some(0);
        }
        if label.starts_with(prefix) {
            return Some(0); // exact-case prefix
        }
        let ll = label.to_lowercase();
        if ll.starts_with(&lower) {
            return Some(1); // case-insensitive prefix
        }
        if lower.chars().count() < 4 {
            return None;
        }
        let d = crate::diagnostics::levenshtein(&lower, &ll);
        (d <= 2).then_some(10 + d as u32)
    };

    // (rank, source priority, item) — locals (0) sort above globals (1) above
    // keywords (2) at an equal rank, so the most-relevant names lead.
    let mut scored: Vec<(u32, u8, Completion)> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    // The enclosing function's locals + parameters (loft is flat-scoped per fn, so
    // every one of its variables is in scope at the cursor).
    if let Some(f) = enclosing_fn(data, cursor_line) {
        let vars = data.def(f).variables();
        for v in 0..vars.count() {
            let vname = vars.name(v);
            // Skip compiler temporaries (`__acc`, …) and unnamed slots.
            if vname.is_empty() || vname == "??" || vname.starts_with("__") {
                continue;
            }
            let Some(r) = rank(vname) else { continue };
            if seen.iter().any(|s| s == vname) {
                continue;
            }
            seen.push(vname.to_string());
            scored.push((
                r,
                0,
                Completion {
                    label: vname.to_string(),
                    kind: 6, // Variable
                    detail: data.type_name_str(vars.tp(v)),
                },
            ));
        }
    }

    // Global defs (fn / struct / enum / typedef / constant / interface).
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.synthetic.is_some() {
            continue;
        }
        let Some((kind, cname)) = crate::api_surface::classify(data, d) else {
            continue;
        };
        if kind == "method" || kind == "operator" {
            continue; // not typed as a bare identifier
        }
        // An empty prefix would dump the whole stdlib — restrict it to the user's
        // own defs (a real prefix may still reach the stdlib by prefix or fuzzy).
        if prefix.is_empty() && def.source != MAIN_SOURCE {
            continue;
        }
        let Some(r) = rank(&cname) else { continue };
        if seen.contains(&cname) {
            continue;
        }
        seen.push(cname.clone());
        scored.push((
            r,
            1,
            Completion {
                kind: completion_kind(kind),
                detail: kind.to_string(),
                label: cname,
            },
        ));
    }

    // Keywords.
    for kw in crate::lexer::keywords() {
        if let Some(r) = rank(kw) {
            scored.push((
                r,
                2,
                Completion {
                    label: (*kw).to_string(),
                    kind: 14, // Keyword
                    detail: "keyword".to_string(),
                },
            ));
        }
    }

    scored.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then(a.1.cmp(&b.1))
            .then_with(|| a.2.label.cmp(&b.2.label))
    });
    let mut out: Vec<Completion> = scored.into_iter().map(|(_, _, c)| c).collect();
    out.truncate(200);
    out
}

fn member_completions(data: &Data, receiver: &str, cursor_line: u32) -> Vec<Completion> {
    let Some(type_nr) = resolve_receiver_type(data, receiver, cursor_line) else {
        return Vec::new();
    };
    let type_name = data.def(type_nr).name().to_string();
    let is_enum = matches!(data.def(type_nr).def_type, crate::data::DefType::Enum);
    let mut out: Vec<Completion> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    // Methods FIRST (`Type.method`) — so a name that is both a method and a
    // struct virtual-field shows once, as a method.
    let dotted = format!("{type_name}.");
    for d in 0..data.definitions() {
        if let Some((kind, cname)) = crate::api_surface::classify(data, d)
            && kind == "method"
            && let Some(method) = cname.strip_prefix(&dotted)
            && !seen.iter().any(|s| s == method)
        {
            seen.push(method.to_string());
            out.push(Completion {
                label: method.to_string(),
                kind: 2, // Method
                detail: cname.clone(),
            });
        }
    }
    if is_enum {
        // An enum's `attributes()` ARE its variants; enumerate the EnumValue defs.
        for v in 0..data.definitions() {
            let vd = data.def(v);
            if vd.parent == type_nr
                && matches!(vd.def_type, crate::data::DefType::EnumValue)
                && !seen.contains(&vd.name)
            {
                seen.push(vd.name.clone());
                out.push(Completion {
                    label: vd.name.clone(),
                    kind: 20, // EnumMember
                    detail: "variant".to_string(),
                });
            }
        }
    } else {
        // A struct's fields — skip hidden, the synthetic `enum` tag, and names
        // already surfaced as methods (virtual fields).
        for a in data.def(type_nr).attributes() {
            if a.hidden || a.name == "enum" || seen.contains(&a.name) {
                continue;
            }
            seen.push(a.name.clone());
            out.push(Completion {
                label: a.name.clone(),
                kind: 5, // Field
                detail: data.type_name_str(&a.typedef),
            });
        }
    }
    out
}

/// Resolve a receiver identifier to a struct/enum def_nr: a TYPE name directly,
/// else a variable's type — resolved SCOPE-PRECISELY in the function enclosing the
/// cursor (@PLN115), falling back to any function's table.
fn resolve_receiver_type(data: &Data, receiver: &str, cursor_line: u32) -> Option<u32> {
    let is_type = |d: u32| {
        d != u32::MAX
            && matches!(
                data.def(d).def_type,
                crate::data::DefType::Struct | crate::data::DefType::Enum
            )
    };
    let named = data.def_nr(receiver);
    if is_type(named) {
        return Some(named);
    }
    let var_type_in = |f: u32| {
        let vars = data.def(f).variables();
        if vars.name_exists(receiver) {
            let tn = data.type_def_nr(vars.var_type(vars.var(receiver)));
            if is_type(tn) {
                return Some(tn);
            }
        }
        None
    };
    // The ENCLOSING function first — a top-level fn whose body contains the cursor
    // is the one with the greatest declaration line at or before it — so a receiver
    // `x` resolves to THIS function's `x`, not a same-named `x` in another.
    if let Some(f) = enclosing_fn(data, cursor_line)
        && let Some(tn) = var_type_in(f)
    {
        return Some(tn);
    }
    // Fallback: any user function with a var of that name (an out-of-scope guess is
    // better than no members when the enclosing-fn heuristic misses).
    for f in 0..data.definitions() {
        let def = data.def(f);
        if def.source == MAIN_SOURCE
            && matches!(def.def_type, crate::data::DefType::Function)
            && let Some(tn) = var_type_in(f)
        {
            return Some(tn);
        }
    }
    None
}

/// The top-level user function enclosing `cursor_line` — the one with the greatest
/// declaration line at or before it (functions don't nest at top level, so the last
/// one opened before the cursor owns it).
fn enclosing_fn(data: &Data, cursor_line: u32) -> Option<u32> {
    (0..data.definitions())
        .filter(|&d| {
            let def = data.def(d);
            def.source == MAIN_SOURCE
                && matches!(def.def_type, crate::data::DefType::Function)
                && def.position.line <= cursor_line
        })
        .max_by_key(|&d| data.def(d).position.line)
}

// ── extract-function (E1+, EXTRACT.md) ───────────────────────────────────────
// A function body is a `Value::Block` whose `operators` interleave `Value::Line(n)`
// markers before each statement; a nested block (`for`/`if` body) carries its OWN
// `Line` markers INSIDE the nested `Block`, so those lines are not top-level markers
// — a selection inside one maps to no top-level statement and is refused.

/// @PLN63 E1 — resolve a 1-based line selection `[start_line, end_line]` to the
/// enclosing top-level function and the contiguous TOP-LEVEL statement operators it
/// covers: `(fn_def, first_op, last_op)`, i.e. `body.operators[first_op..=last_op]`
/// (including the `Line` markers) are the selected statements — the slice the E2/E3
/// data-flow then analyses.
///
/// `None` when the selection does not map to WHOLE top-level statements: no
/// enclosing function, a non-block body (e.g. a `#rust` fn), an empty/reversed
/// range, a start that is not a statement boundary (a `Line` marker at
/// `start_line`), or a range whose lines are inside a nested block.
#[must_use]
pub fn extract_range(
    text: &str,
    name: &str,
    stdlib_dir: &str,
    start_line: u32,
    end_line: u32,
) -> Option<(u32, usize, usize)> {
    if end_line < start_line {
        return None;
    }
    let p = parse_lsp_buffer(text, name, stdlib_dir);
    let data = &p.data;
    let f = enclosing_fn(data, start_line)?;
    let crate::data::Value::Block(b) = data.def(f).code().unspan() else {
        return None;
    };
    let (first_op, last_op) = statement_slice(b, start_line, end_line)?;
    Some((f, first_op, last_op))
}

/// The top-level operator index range `[first_op, last_op]` in `b` covering the
/// statements on lines `[start_line, end_line]`, via the `Value::Line` markers.
/// `None` when the start is not a top-level statement boundary or nothing maps.
fn statement_slice(
    b: &crate::data::Block,
    start_line: u32,
    end_line: u32,
) -> Option<(usize, usize)> {
    // Top-level source-line markers, in order: (line, operator index).
    let marks: Vec<(u32, usize)> = b
        .operators
        .iter()
        .enumerate()
        .filter_map(|(i, op)| match op {
            crate::data::Value::Line(n) => Some((*n, i)),
            _ => None,
        })
        .collect();
    // The selection must START at a top-level statement boundary (a `Line == start_line`).
    let first = marks.iter().position(|(l, _)| *l == start_line)?;
    // The last top-level statement whose line is within the selection.
    let last = marks.iter().rposition(|(l, _)| *l <= end_line)?;
    if last < first {
        return None;
    }
    // Include everything up to the operator before the NEXT top-level statement (or
    // the block end) — the last selected statement's whole body.
    let last_op = marks
        .get(last + 1)
        .map_or(b.operators.len(), |&(_, i)| i)
        .saturating_sub(1);
    Some((marks[first].1, last_op))
}

/// The extract-function signature of a slice: `(inputs, outputs)` var_nrs.
///
/// - **inputs** = the upward-exposed uses ∩ LIVE-IN (a parameter of the function, or
///   a var written BEFORE the slice).  The live-in filter is load-bearing: it
///   excludes a `for`/lambda BINDER (read in the slice but declared there, so it must
///   not become a parameter — a param colliding with the binder is invalid) and any
///   var first produced inside the slice.
/// - **outputs** = writes-in-slice ∩ live-out (read in the tail).  A var in both is
///   in-out.
fn slice_signature(
    data: &Data,
    f: u32,
    b: &crate::data::Block,
    first_op: usize,
    last_op: usize,
) -> (Vec<u16>, Vec<u16>) {
    let vars = data.def(f).variables();
    let slice = &b.operators[first_op..=last_op];
    // Live-in: written before the slice, or a parameter.
    let mut written_before: Vec<u16> = Vec::new();
    for op in &b.operators[..first_op] {
        collect_writes(op, &mut written_before);
    }
    let live_in = |v: u16| vars.is_argument(v) || written_before.contains(&v);
    let inputs: Vec<u16> = slice_inputs(slice)
        .into_iter()
        .filter(|&v| live_in(v))
        .collect();
    // Outputs: written in the slice AND read in the tail.
    let mut writes: Vec<u16> = Vec::new();
    for op in slice {
        collect_writes(op, &mut writes);
    }
    let mut tail_reads: Vec<u16> = Vec::new();
    for op in &b.operators[last_op + 1..] {
        collect_reads(op, &mut tail_reads);
    }
    let outputs: Vec<u16> = writes
        .into_iter()
        .filter(|v| tail_reads.contains(v))
        .collect();
    (inputs, outputs)
}

/// @PLN63 E2 — the INPUT variables (→ parameters) for extracting a statement slice:
/// the upward-exposed uses ∩ live-in (see [`slice_signature`]), in first-use order.
/// `None` when the selection does not map (see [`extract_range`]).
#[must_use]
pub fn extract_inputs(
    text: &str,
    name: &str,
    stdlib_dir: &str,
    start_line: u32,
    end_line: u32,
) -> Option<Vec<String>> {
    if end_line < start_line {
        return None;
    }
    let p = parse_lsp_buffer(text, name, stdlib_dir);
    let data = &p.data;
    let f = enclosing_fn(data, start_line)?;
    let crate::data::Value::Block(b) = data.def(f).code().unspan() else {
        return None;
    };
    let (first_op, last_op) = statement_slice(b, start_line, end_line)?;
    let (inputs, _outputs) = slice_signature(data, f, b, first_op, last_op);
    let vars = data.def(f).variables();
    Some(inputs.iter().map(|&v| vars.name(v).to_string()).collect())
}

/// The upward-exposed uses (input var_nrs) of a statement slice, in first-use order.
fn slice_inputs(ops: &[crate::data::Value]) -> Vec<u16> {
    let mut written: Vec<u16> = Vec::new();
    let mut inputs: Vec<u16> = Vec::new();
    for op in ops {
        scan_input(op, &mut written, &mut inputs, true);
    }
    inputs
}

/// Record a READ of `v`: an input iff not yet definitely written and not already noted.
fn note_read(v: u16, written: &[u16], inputs: &mut Vec<u16>) {
    if !written.contains(&v) && !inputs.contains(&v) {
        inputs.push(v);
    }
}

/// Ordered upward-exposed-use walk.  `definite` is false inside a conditional branch
/// or loop body (writes there are not guaranteed, so they do not mask a later read).
fn scan_input(
    node: &crate::data::Value,
    written: &mut Vec<u16>,
    inputs: &mut Vec<u16>,
    definite: bool,
) {
    use crate::data::Value;
    match node.unspan() {
        // Var-READ fields `for_each_child` does not surface — handle explicitly.
        Value::Var(v) => note_read(*v, written, inputs),
        Value::TupleGet(v, _) => note_read(*v, written, inputs),
        Value::CallRef(v, args) => {
            note_read(*v, written, inputs);
            for a in args {
                scan_input(a, written, inputs, definite);
            }
        }
        Value::TuplePut(v, _, inner) => {
            note_read(*v, written, inputs);
            scan_input(inner, written, inputs, definite);
        }
        // A write: read the RHS FIRST (so `x += 1` reads `x` before the write), then
        // mark `x` written — only when the write is unconditional.
        Value::Set(v, rhs) => {
            scan_input(rhs, written, inputs, definite);
            if definite && !written.contains(v) {
                written.push(*v);
            }
        }
        Value::If(c, t, e) => {
            scan_input(c, written, inputs, definite);
            scan_input(t, written, inputs, false);
            scan_input(e, written, inputs, false);
        }
        Value::Loop(b) => {
            for o in &b.operators {
                scan_input(o, written, inputs, false);
            }
        }
        // Every other variant: recurse into child expressions in order (covers Call,
        // Block, Return, Iter, Tuple, … safely — none carry a bare var-read field).
        other => other.for_each_child(&mut |c| scan_input(c, written, inputs, definite)),
    }
}

/// @PLN63 E3 — the OUTPUT variables (→ return values) for extracting a statement
/// slice: vars WRITTEN in the slice (on any path) that are LIVE-OUT — read in the
/// function tail after the slice — in write order.  A var that is both an input and
/// an output is an IN-OUT (a parameter the function also returns).  `None` when the
/// selection does not map (see [`extract_range`]).
///
/// Conservatism flips from E2: outputs are a SUPERSET (all writes ∩ any tail read),
/// so a var whose new value the caller might need is always returned — never a stale
/// value left behind (a missed output would silently drop a modification).
#[must_use]
pub fn extract_outputs(
    text: &str,
    name: &str,
    stdlib_dir: &str,
    start_line: u32,
    end_line: u32,
) -> Option<Vec<String>> {
    if end_line < start_line {
        return None;
    }
    let p = parse_lsp_buffer(text, name, stdlib_dir);
    let data = &p.data;
    let f = enclosing_fn(data, start_line)?;
    let crate::data::Value::Block(b) = data.def(f).code().unspan() else {
        return None;
    };
    let (first_op, last_op) = statement_slice(b, start_line, end_line)?;
    let (_inputs, outputs) = slice_signature(data, f, b, first_op, last_op);
    let vars = data.def(f).variables();
    Some(outputs.iter().map(|&v| vars.name(v).to_string()).collect())
}

/// Every var WRITTEN anywhere in `node` (any path), in first-write order — a `Set`
/// or `TuplePut` target.  `for_each_child` covers the rest safely.
fn collect_writes(node: &crate::data::Value, out: &mut Vec<u16>) {
    use crate::data::Value;
    match node.unspan() {
        Value::Set(v, rhs) => {
            if !out.contains(v) {
                out.push(*v);
            }
            collect_writes(rhs, out);
        }
        Value::TuplePut(v, _, inner) => {
            if !out.contains(v) {
                out.push(*v);
            }
            collect_writes(inner, out);
        }
        other => other.for_each_child(&mut |c| collect_writes(c, out)),
    }
}

/// Every var READ anywhere in `node` (any path) — the var-read variants
/// `for_each_child` does not surface, plus recursion for the rest.
fn collect_reads(node: &crate::data::Value, out: &mut Vec<u16>) {
    use crate::data::Value;
    match node.unspan() {
        Value::Var(v) | Value::TupleGet(v, _) => {
            if !out.contains(v) {
                out.push(*v);
            }
        }
        Value::CallRef(v, args) => {
            if !out.contains(v) {
                out.push(*v);
            }
            for a in args {
                collect_reads(a, out);
            }
        }
        Value::TuplePut(v, _, inner) => {
            if !out.contains(v) {
                out.push(*v);
            }
            collect_reads(inner, out);
        }
        other => other.for_each_child(&mut |c| collect_reads(c, out)),
    }
}

/// @PLN63 E6 — whether `node` contains control flow that would ESCAPE the extracted
/// slice: a `return` / `yield` (always leaves the function), or a `break` / `continue`
/// whose target loop is not itself inside the slice (`loop_depth` counts loops entered
/// WITHIN the slice; at depth 0 a break/continue targets an outer loop).
fn control_flow_escapes(node: &crate::data::Value, loop_depth: u32) -> bool {
    use crate::data::Value;
    match node.unspan() {
        Value::Return(_) | Value::Yield(_) => true,
        Value::Break(_) | Value::Continue(_) => loop_depth == 0,
        Value::Loop(b) => b
            .operators
            .iter()
            .any(|o| control_flow_escapes(o, loop_depth + 1)),
        other => {
            let mut escapes = false;
            other.for_each_child(&mut |c| {
                escapes = escapes || control_flow_escapes(c, loop_depth);
            });
            escapes
        }
    }
}

/// @PLN63 E4 — the concrete edit for extracting a statement slice: the new function
/// to insert and the call that replaces the selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtractEdit {
    /// The full source of the new function (`fn extracted(…) -> … { … }`) — inserted
    /// after the buffer end.  The name is a placeholder the user renames.
    pub new_function: String,
    /// The replacement for the selected statement lines, indented at the call site
    /// (`(a, b) = extracted(x, y);`).
    pub call: String,
    /// 1-based inclusive line range of the selection the `call` replaces.
    pub start_line: u32,
    pub end_line: u32,
}

/// @PLN63 E4 — synthesise the extract-function edit for `[start_line, end_line]`:
/// combines the E1 slice, E2 inputs (→ parameters), and E3 outputs (→ return
/// values, a tuple for ≥2) with the selected SOURCE TEXT into a new function and its
/// call.  Variable names are preserved, so the selected body resolves unchanged
/// (inputs are same-named params, outputs are locals or in-out params).  `None` when
/// the selection does not map (see [`extract_range`]).
#[must_use]
pub fn extract_function(
    text: &str,
    name: &str,
    stdlib_dir: &str,
    start_line: u32,
    end_line: u32,
) -> Option<ExtractEdit> {
    if end_line < start_line {
        return None;
    }
    let p = parse_lsp_buffer(text, name, stdlib_dir);
    let data = &p.data;
    let f = enclosing_fn(data, start_line)?;
    let crate::data::Value::Block(b) = data.def(f).code().unspan() else {
        return None;
    };
    let (first_op, last_op) = statement_slice(b, start_line, end_line)?;
    // E6 — REFUSE control flow that escapes the slice: a `return` / `yield` (always
    // leaves the function), or a `break` / `continue` targeting a loop OUTSIDE the
    // slice.  Extracting it would change control flow (the `return` would leave the
    // NEW function, not the caller), so offer nothing rather than a broken edit.
    if b.operators[first_op..=last_op]
        .iter()
        .any(|op| control_flow_escapes(op, 0))
    {
        return None;
    }
    // Inputs (E2, live-in-filtered) and outputs (E3) from this one parse.
    let (inputs, outputs) = slice_signature(data, f, b, first_op, last_op);
    let vars = data.def(f).variables();
    // E5 — REFUSE a `&`-reference input/output.  A borrow's aliasing can't be safely
    // re-expressed as a by-value parameter/return (the extracted call might copy where
    // the original aliased), so rather than emit a subtly-wrong edit we offer nothing.
    // Owned heap values (`vector`/`struct`/`text` by value) ARE safe: loft copies them
    // into the parameter and delivers an in-out via the return.
    if inputs
        .iter()
        .chain(outputs.iter())
        .any(|&v| matches!(vars.tp(v), crate::data::Type::RefVar(_)))
    {
        return None;
    }
    let ty = |v: u16| data.type_name_str(vars.tp(v));
    let params: Vec<String> = inputs
        .iter()
        .map(|&v| format!("{}: {}", vars.written_name(v), ty(v)))
        .collect();
    // The selected lines are copied verbatim, so they spell each variable as the author wrote
    // it (`f`, never the second loop's `f#1`); the signature and the call must match them.
    let out_names: Vec<String> = outputs
        .iter()
        .map(|&v| vars.written_name(v).to_string())
        .collect();
    let out_types: Vec<String> = outputs.iter().map(|&v| ty(v)).collect();
    let args: Vec<String> = inputs
        .iter()
        .map(|&v| vars.written_name(v).to_string())
        .collect();

    // The selected source lines (verbatim) + the call-site indentation.
    let lines: Vec<&str> = text.lines().collect();
    let (lo, hi) = ((start_line - 1) as usize, (end_line - 1) as usize);
    if hi >= lines.len() {
        return None;
    }
    let body = lines[lo..=hi].join("\n");
    let indent: String = lines[lo]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();

    let ret_type = match out_types.len() {
        0 => String::new(),
        1 => format!(" -> {}", out_types[0]),
        _ => format!(" -> ({})", out_types.join(", ")),
    };
    let ret_stmt = match out_names.len() {
        0 => String::new(),
        1 => format!("\n{indent}return {};", out_names[0]),
        _ => format!("\n{indent}return ({});", out_names.join(", ")),
    };
    let new_function = format!(
        "fn extracted({}){ret_type} {{\n{body}{ret_stmt}\n}}",
        params.join(", ")
    );
    let call = match out_names.len() {
        0 => format!("{indent}extracted({});", args.join(", ")),
        1 => format!("{indent}{} = extracted({});", out_names[0], args.join(", ")),
        _ => format!(
            "{indent}({}) = extracted({});",
            out_names.join(", "),
            args.join(", ")
        ),
    };
    Some(ExtractEdit {
        new_function,
        call,
        start_line,
        end_line,
    })
}

/// `classify` label → LSP `CompletionItemKind`.
fn completion_kind(kind: &str) -> u32 {
    match kind {
        "struct" => 22,
        "enum" => 13,
        "method" => 2,
        "constant" => 21,
        "interface" => 8,
        "typedef" => 7,
        "operator" => 24,
        _ => 3, // fn
    }
}

// ── semantic tokens (step D) ─────────────────────────────────────────────────
// Lex the buffer and classify each identifier by its resolved def kind (reusing
// the same name-lookup as references/completion) + keywords.  A lexical first
// cut: global fns / types / structs / enums / consts + keywords are typed; locals
// and methods (need the receiver type) fall through to the editor's grammar until
// step F.

/// One classified token — absolute 1-based `line`/`col`, char `len`, and a `kind`
/// index into [`semantic_token_types`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SemanticToken {
    pub line: u32,
    pub col: u32,
    pub len: u32,
    pub kind: u32,
}

/// The token-type legend the LSP `semanticTokensProvider` declares; a token's
/// `kind` indexes into it.  (0 keyword, 1 function, 2 method, 3 struct, 4 enum,
/// 5 type, 6 variable, 7 interface, 8 property.)
#[must_use]
pub fn semantic_token_types() -> &'static [&'static str] {
    &[
        "keyword",
        "function",
        "method",
        "struct",
        "enum",
        "type",
        "variable",
        "interface",
        "property",
    ]
}

/// Classify every identifier token in `text` by its resolved kind.  Fresh parse
/// per call (same rule as [`diagnose`]).
///
/// @PLN115: each token is first matched against the parse-time resolution index by
/// position — so LOCALS (variable), METHODS (method, keyed on the receiver type),
/// and FIELDS (property) are classified, which the name-based classifier cannot do —
/// then falls back to the name lookup for globals / types / keywords the index does
/// not record (a type in a declaration, a keyword).
#[must_use]
pub fn semantic_tokens(text: &str, name: &str, stdlib_dir: &str) -> Vec<SemanticToken> {
    use crate::lexer::{LexItem, Lexer};
    let p = parse_lsp_buffer(text, name, stdlib_dir);
    let data = &p.data;
    // A `(line, col)` → legend-kind map from the index (same position convention as
    // the lexer, so a token's start matches its occurrence exactly).
    let mut resolved: HashMap<(u32, u32), u32> = HashMap::new();
    for o in p.resolutions() {
        let kind = match o.res {
            crate::resolution::Resolution::Local { .. } => 6, // variable
            crate::resolution::Resolution::Method { .. } => 2, // method
            crate::resolution::Resolution::Field { .. } => 8, // property
            crate::resolution::Resolution::Global(_) => 1,    // function
            crate::resolution::Resolution::Unresolved => continue,
        };
        resolved.insert((o.line, o.col), kind);
    }
    let mut lexer = Lexer::default();
    lexer.parse_string(text, name);
    let mut out = Vec::new();
    loop {
        let r = lexer.peek();
        match r.has {
            LexItem::None => break,
            LexItem::Identifier(id) => {
                let kind = resolved
                    .get(&(r.position.line, r.position.pos))
                    .copied()
                    .or_else(|| token_kind(data, &id));
                if let Some(kind) = kind {
                    out.push(SemanticToken {
                        line: r.position.line,
                        col: r.position.pos,
                        len: id.chars().count() as u32,
                        kind,
                    });
                }
            }
            _ => {}
        }
        lexer.cont();
    }
    out.sort_by_key(|t| (t.line, t.col));
    out
}

/// The legend index for an identifier `name`, or `None` when it isn't a token we
/// classify (a local / parameter / method-by-bare-name — the grammar handles it).
fn token_kind(data: &Data, name: &str) -> Option<u32> {
    if crate::lexer::is_keyword(name) {
        return Some(0); // keyword
    }
    if data.def_nr(&format!("n_{name}")) != u32::MAX {
        return Some(1); // function
    }
    let d = data.def_nr(name);
    if d == u32::MAX {
        return None;
    }
    match data.def(d).def_type {
        crate::data::DefType::Struct => Some(3),
        crate::data::DefType::Enum => Some(4),
        crate::data::DefType::Type => Some(5),
        crate::data::DefType::Constant => Some(6),
        crate::data::DefType::Interface => Some(7),
        _ => None,
    }
}

// ── scope-aware resolution (step F v1) ───────────────────────────────────────
// Distinguish a GLOBAL symbol (a top-level def / method — workspace-wide) from a
// LOCAL (a variable / parameter — scoped to its enclosing block).  Sharpens
// find-references and rename: a local is renamed only within its own function,
// not every same-named local elsewhere.  A source-scan foundation (no parser
// instrumentation); the type/scope work that later features build on.

/// Where an identifier's references live.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RefScope {
    /// A top-level definition or method — references are workspace-wide.
    Global,
    /// A local / parameter — references are confined to the enclosing top-level
    /// `{ … }` block in the SAME file (1-based inclusive line range).
    Local { start_line: u32, end_line: u32 },
}

/// The scope of the identifier `name` sitting on `line`: GLOBAL if it resolves to
/// a top-level def or a method name, else LOCAL (the enclosing top-level block).
/// Fresh parse per call (same rule as [`diagnose`]).
#[must_use]
pub fn reference_scope(text: &str, name: &str, stdlib_dir: &str, line: u32) -> RefScope {
    let p = parse_lsp_buffer(text, name, stdlib_dir);
    if is_global_symbol(&p.data, name) {
        return RefScope::Global;
    }
    // A local: confine to the enclosing block; if the cursor is not inside one
    // (e.g. a top-level position), fall back to global (nothing to over-narrow).
    match enclosing_block(text, line) {
        Some((start_line, end_line)) => RefScope::Local {
            start_line,
            end_line,
        },
        None => RefScope::Global,
    }
}

/// Filter workspace references by a [`RefScope`]: a global keeps them all; a local
/// keeps only those in `current_file` within its line range.
#[must_use]
pub fn scoped_refs(all: Vec<Reference>, scope: &RefScope, current_file: &str) -> Vec<Reference> {
    match scope {
        RefScope::Global => all,
        RefScope::Local {
            start_line,
            end_line,
        } => {
            let here = canonical(Path::new(current_file));
            all.into_iter()
                .filter(|r| r.file == here && r.line >= *start_line && r.line <= *end_line)
                .collect()
        }
    }
}

/// A name is a GLOBAL symbol if it is a free function (`n_<name>`), a type-like
/// def (`<name>`), or a method (`Type.<name>`, cross-file).  Anything else the
/// buffer names is a local / parameter.
fn is_global_symbol(data: &Data, name: &str) -> bool {
    if data.def_nr(&format!("n_{name}")) != u32::MAX {
        return true;
    }
    let d = data.def_nr(name);
    if d != u32::MAX
        && matches!(
            data.def(d).def_type,
            crate::data::DefType::Struct
                | crate::data::DefType::Enum
                | crate::data::DefType::Type
                | crate::data::DefType::Constant
                | crate::data::DefType::Interface
                | crate::data::DefType::Function
        )
    {
        return true;
    }
    // A method name — the trailing segment of some `Type.method`.
    let dotted = format!(".{name}");
    (0..data.definitions()).any(|d| {
        crate::api_surface::classify(data, d)
            .is_some_and(|(kind, cname)| kind == "method" && cname.ends_with(&dotted))
    })
}

/// The top-level `{ … }` block (1-based inclusive line range) whose body contains
/// `line`, found by lexing (so braces in strings / comments don't miscount).
fn enclosing_block(text: &str, line: u32) -> Option<(u32, u32)> {
    use crate::lexer::{LexItem, Lexer};
    let mut lexer = Lexer::default();
    lexer.parse_string(text, "buf.loft");
    let mut depth = 0i32;
    let mut block_start = 0u32;
    let mut best = None;
    loop {
        let r = lexer.peek();
        match &r.has {
            LexItem::None => break,
            LexItem::Token(t) if t == "{" => {
                if depth == 0 {
                    block_start = r.position.line;
                }
                depth += 1;
            }
            LexItem::Token(t) if t == "}" => {
                depth -= 1;
                if depth == 0 && line >= block_start && line <= r.position.line {
                    best = Some((block_start, r.position.line));
                }
            }
            _ => {}
        }
        lexer.cont();
    }
    best
}

#[cfg(test)]
mod uri_path_tests {
    use super::{path_to_uri, uri_to_path};
    use std::path::Path;

    #[test]
    fn path_to_uri_is_editor_shaped_and_never_has_backslashes() {
        // A Windows drive path → the triple-slash, forward-slash form editors accept.
        let u = path_to_uri(Path::new(r"C:\Users\x\a.loft"));
        assert_eq!(u, "file:///C:/Users/x/a.loft");
        // The whole point: a `file://` URI must NEVER carry backslashes — they are
        // invalid JSON escapes (`\U`), which silently corrupts any LSP message the
        // URI is embedded in — the cause of the Windows-nightly lsp_transport hang.
        assert!(!u.contains('\\'), "backslash in URI: {u}");
        // A POSIX absolute path already starts with `/`, so the branches meet at
        // three slashes.
        assert_eq!(
            path_to_uri(Path::new("/home/x/a.loft")),
            "file:///home/x/a.loft"
        );
        assert!(!path_to_uri(Path::new("/home/x/a.loft")).contains('\\'));
    }

    #[test]
    fn uri_to_path_inverts_path_to_uri_on_this_platform() {
        // Round-trips with native separators, whichever platform runs the test.
        #[cfg(windows)]
        {
            assert_eq!(uri_to_path("file:///C:/a/b"), r"C:\a\b");
            assert_eq!(uri_to_path(&path_to_uri(Path::new(r"C:\a\b"))), r"C:\a\b");
        }
        #[cfg(not(windows))]
        {
            assert_eq!(uri_to_path("file:///a/b"), "/a/b");
            assert_eq!(uri_to_path(&path_to_uri(Path::new("/a/b"))), "/a/b");
        }
        // A non-file:// string passes through.
        assert_eq!(uri_to_path("stdin://x"), "stdin://x");
    }
}
