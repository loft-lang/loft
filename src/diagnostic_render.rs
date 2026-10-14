// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I75 — Diagnostics collector

//! Plan-07 phase 2 — pretty diagnostic renderer.
//!
//! Two output formats live side-by-side:
//!
//! - **Compact**: `Error: msg at file:line:col` — one line per entry.
//!   Used by `tests/issues.rs`, `tests/dumps/*`, `LOFT_LOG=...` traces.
//!   The harness machinery greps stderr in single-line shape.
//!
//! - **Pretty**: rustc / clang style, multi-line with caret + optional
//!   note.  Used by `cargo run --bin loft` and the user-facing CLI.
//!
//! `LOFT_ERRORS=compact|pretty` and `--errors=compact|pretty` switch
//! between the two; pretty is the default for interactive runs.
//!
//! Plan-07 phase 4 will wire runtime errors through the same
//! renderer; phase 5 adds suggestions; phase 6 adds typed-mismatch
//! detail.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::Mutex;

use crate::diagnostics::{DiagEntry, Diagnostics, Level};

/// Read-once cache of source-file lines, keyed by absolute path.  Used
/// by the renderer to fetch the offending source line for the caret.
pub trait SourceLoader {
    /// Return the 1-based source line for the given file, or `None` if
    /// the file can't be read or the line number is out of range.
    fn line(&self, file: &str, line_1based: u32) -> Option<String>;
}

/// File-system-backed `SourceLoader`.  Reads each file once on first
/// access; caches the per-line vector in an internal `Mutex<HashMap>`.
/// Marginal memory: ~50 KB per loft file kept resident.  At ~3 files
/// loaded in practice (user file + 2-3 default/*.loft) the total is
/// well under 1 MB.
#[derive(Default)]
pub struct FileSourceLoader {
    cache: Mutex<HashMap<String, Option<Vec<String>>>>,
}

impl FileSourceLoader {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl SourceLoader for FileSourceLoader {
    fn line(&self, file: &str, line_1based: u32) -> Option<String> {
        if line_1based == 0 || file.is_empty() {
            return None;
        }
        let mut cache = self.cache.lock().ok()?;
        let entry = cache.entry(file.to_string()).or_insert_with(|| {
            crate::file_access::read_to_string(file)
                .ok()
                .map(|text| text.lines().map(str::to_string).collect())
        });
        let lines = entry.as_ref()?;
        lines.get((line_1based - 1) as usize).cloned()
    }
}

/// In-memory `SourceLoader` for unit tests — pre-populates the cache
/// with `(file, vec[line_strings])` pairs.
#[cfg(test)]
#[derive(Default)]
pub struct InMemoryLoader {
    files: HashMap<String, Vec<String>>,
}

#[cfg(test)]
impl InMemoryLoader {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&mut self, file: &str, content: &str) {
        self.files.insert(
            file.to_string(),
            content.lines().map(str::to_string).collect(),
        );
    }
}

#[cfg(test)]
impl SourceLoader for InMemoryLoader {
    fn line(&self, file: &str, line_1based: u32) -> Option<String> {
        if line_1based == 0 {
            return None;
        }
        self.files
            .get(file)?
            .get((line_1based - 1) as usize)
            .cloned()
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorMode {
    /// Always emit ANSI escape sequences.
    Always,
    /// Never emit ANSI escapes.  Default for non-TTY output (tests, pipes).
    Off,
    /// Emit ANSI escapes only when stderr is a terminal.
    Auto,
}

impl ColorMode {
    /// Resolve `Auto` to `Always` or `Off` based on whether stderr is a
    /// terminal.
    fn resolved(self) -> bool {
        match self {
            ColorMode::Always => true,
            ColorMode::Off => false,
            ColorMode::Auto => {
                use std::io::IsTerminal;
                std::io::stderr().is_terminal()
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ErrorMode {
    /// Single-line per entry: `Error: msg at file:line:col`.
    Compact,
    /// Multi-line rustc-style with source + caret.
    Pretty,
}

impl ErrorMode {
    /// Resolve from CLI flag and env var.  CLI wins; env var is
    /// fallback; default is `Pretty`.
    pub fn from_cli_and_env(cli_arg: Option<&str>) -> Self {
        if let Some(s) = cli_arg {
            return Self::parse_str(s).unwrap_or(ErrorMode::Pretty);
        }
        if let Ok(env) = std::env::var("LOFT_ERRORS") {
            return Self::parse_str(&env).unwrap_or(ErrorMode::Pretty);
        }
        ErrorMode::Pretty
    }

    fn parse_str(s: &str) -> Option<Self> {
        match s.trim() {
            "compact" => Some(ErrorMode::Compact),
            "pretty" => Some(ErrorMode::Pretty),
            _ => None,
        }
    }
}

const ANSI_RED: &str = "\x1b[31m";
const ANSI_YELLOW: &str = "\x1b[33m";
const ANSI_BLUE: &str = "\x1b[34m";
const ANSI_BOLD: &str = "\x1b[1m";
const ANSI_RESET: &str = "\x1b[0m";

fn level_label(level: Level) -> &'static str {
    match level {
        Level::Debug => "note",
        Level::Advice => "advice",
        Level::Warning => "warning",
        Level::Error => "error",
        Level::Fatal => "fatal",
    }
}

fn level_color(level: Level) -> &'static str {
    match level {
        Level::Debug | Level::Advice => ANSI_BLUE,
        Level::Warning => ANSI_YELLOW,
        Level::Error | Level::Fatal => ANSI_RED,
    }
}

/// Number of decimal digits in `n` (used to size the gutter).
fn digit_width(n: u32) -> usize {
    let mut n = n.max(1);
    let mut w = 0;
    while n > 0 {
        n /= 10;
        w += 1;
    }
    w
}

/// Display column for byte position `byte_col_0based` inside `line`.
/// Tabs render as 4 spaces.  Multi-byte UTF-8 characters count as one
/// column (display width approximation; good enough for the caret —
/// full unicode-width handling deferred to phase 4 if needed).
fn display_column(line: &str, byte_col_0based: usize) -> usize {
    let mut col = 0;
    for (i, ch) in line.char_indices() {
        if i >= byte_col_0based {
            break;
        }
        col += if ch == '\t' { 4 } else { 1 };
    }
    col
}

/// Render the full source line with tabs expanded to 4 spaces.
fn render_source_line(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    for ch in line.chars() {
        if ch == '\t' {
            out.push_str("    ");
        } else {
            out.push(ch);
        }
    }
    out
}

/// Render one diagnostic entry in pretty mode.
///
/// Format (no colour):
/// ```text
/// error: <message>
///   --> <file>:<line>:<col>
///    |
/// NN | <source line>
///    |     <padding>^
/// ```
///
/// `(file, line, col)` are taken from the entry; if any is empty/0,
/// the location block is omitted.  If the source loader returns None,
/// the source-line block is omitted but the `-->` line still renders
/// so the error stays clickable.
#[must_use]
pub fn render_entry_pretty(
    entry: &DiagEntry,
    loader: &dyn SourceLoader,
    color: ColorMode,
) -> String {
    let use_color = color.resolved();
    let label = level_label(entry.level);
    let color_open = if use_color {
        level_color(entry.level)
    } else {
        ""
    };
    let bold_open = if use_color { ANSI_BOLD } else { "" };
    let reset = if use_color { ANSI_RESET } else { "" };

    let mut out = String::with_capacity(128);
    // Header: `error[code]: message` (with optional bold + colour).
    // @PLN102 arc-E E1 — the `[code]` names the frozen-identity diagnostic;
    // omitted when the site carries no code yet.
    let code_tag = entry.code.map_or(String::new(), |c| format!("[{c}]"));
    let _ = writeln!(
        out,
        "{color_open}{bold_open}{label}{code_tag}{reset}{bold_open}: {}{reset}",
        entry.message
    );
    if entry.file.is_empty() || entry.line == 0 {
        return out;
    }
    // Location: `  --> file:line:col`
    let _ = writeln!(out, "  --> {}:{}:{}", entry.file, entry.line, entry.col);
    // Source line + caret, only if loader returns a line.
    let Some(line_text) = loader.line(&entry.file, entry.line) else {
        return out;
    };
    let line_num_str = entry.line.to_string();
    let gutter = digit_width(entry.line);
    let pad = " ".repeat(gutter);
    let _ = writeln!(out, "{pad} |");
    let _ = writeln!(out, "{line_num_str} | {}", render_source_line(&line_text));
    let col_byte_0based = entry.col.saturating_sub(1) as usize;
    let display_col = display_column(&line_text, col_byte_0based);
    let caret_pad = " ".repeat(display_col);
    let _ = writeln!(out, "{pad} | {caret_pad}{color_open}^{reset}");
    render_fixes(&mut out, entry, bold_open, reset);
    out
}

/// @PLN131 — append the fix lines under a rendered diagnostic, when `--explain` asked for
/// them and the site carries any.
///
/// The layout puts the CONDITION in its own column rather than inside a sentence, because a
/// click has to affirm it: the reader must be able to see the thing being affirmed, not
/// extract it from a clause. That is also what gives the CLI and an LSP code action one
/// shared shape — `title` on the lightbulb, `condition` in the confirm step.
///
/// ```text
///   fix  build the value in place                                        [move · @F106]
///   fix  drop the later use of `src`   needs: `src` is used again at …   [move · @F106]
/// ```
fn render_fixes(out: &mut String, entry: &DiagEntry, bold_open: &str, reset: &str) {
    if entry.fixes.is_empty() || !crate::keys::explain_enabled() {
        return;
    }
    for fix in &entry.fixes {
        // Both columns when both exist. A conditional fix that also spells an edit is the
        // one shape where dropping either is unsafe: the edit is what gets applied and the
        // condition is what the click affirms, so showing only the edit would let a reader
        // apply a rewrite whose precondition they never saw.
        // An edit renders as the TEXT it writes, not as its span: the span is for an
        // applier, and a reader wants to see what would appear in their source. An
        // insertion (`len == 0`) says so, because a bare `?` on its own reads as noise.
        let mut detail = fix.edit.as_ref().map_or_else(String::new, |e| {
            if e.len == 0 {
                format!("insert `{}`", e.text)
            } else {
                format!("write `{}`", e.text)
            }
        });
        if let Some(c) = &fix.condition {
            if !detail.is_empty() {
                detail.push_str("   ");
            }
            let _ = write!(detail, "needs: {c}");
        }
        // The concept is a door, not a lecture: the noun plus where to read about it, and
        // nothing that defines it here.
        let door = format!("[{} · {}]", fix.concept, fix.concept_ref);
        if detail.is_empty() {
            let _ = writeln!(out, "  {bold_open}fix{reset}  {}   {door}", fix.title);
        } else {
            let _ = writeln!(
                out,
                "  {bold_open}fix{reset}  {}   {detail}   {door}",
                fix.title
            );
        }
    }
}

/// Render an entire `Diagnostics` value as pretty output.
///
/// Multi-entry layout:
/// 1. Entries appear in their original order (preserves causality).
/// 2. One blank line between entries.
/// 3. After all entries, a summary line:
///    `error: aborting due to N previous error(s)` (rustc style)
///    when at least one error-level entry was rendered.
///
/// Cascade dedup: a `Warning` at the exact same `(file, line, col)` as
/// a preceding `Error` is suppressed (the error already covered the
/// location).
#[must_use]
pub fn render_pretty_all(
    diags: &Diagnostics,
    loader: &dyn SourceLoader,
    color: ColorMode,
) -> String {
    let entries = diags.entries();
    if entries.is_empty() {
        return String::new();
    }
    // Cascade dedup pass: collect indices to render.
    let mut keep: Vec<bool> = vec![true; entries.len()];
    for i in 0..entries.len() {
        if !matches!(entries[i].level, Level::Warning | Level::Advice) {
            continue;
        }
        for j in 0..i {
            if !keep[j] {
                continue;
            }
            let prev = &entries[j];
            let cur = &entries[i];
            if prev.level >= Level::Error
                && prev.file == cur.file
                && prev.line == cur.line
                && prev.col == cur.col
            {
                keep[i] = false;
                break;
            }
        }
    }

    let mut out = String::new();
    let mut error_count = 0u32;
    let mut fixable = 0u32;
    let mut first = true;
    for (i, entry) in entries.iter().enumerate() {
        if entry.level == Level::Debug {
            continue;
        }
        if !keep[i] {
            continue;
        }
        if !first {
            out.push('\n');
        }
        first = false;
        out.push_str(&render_entry_pretty(entry, loader, color));
        if entry.level >= Level::Error {
            error_count += 1;
        }
        if !entry.fixes.is_empty() {
            fixable += 1;
        }
    }

    // @PLN131 — ONCE per run, never per diagnostic.
    //
    // The messages hand "what to write instead" to the fix lines, which are opt-in — so
    // without this a reader who does not already know about `--explain` is simply told less
    // than before. A per-diagnostic pointer would pay for that with a doubled line count on
    // a file with fifty copy notices, which is the noise the opt-in exists to avoid. One
    // line naming the count is the whole cost.
    if fixable > 0 && !crate::keys::explain_enabled() {
        let bold_open = if color.resolved() { ANSI_BOLD } else { "" };
        let reset = if color.resolved() { ANSI_RESET } else { "" };
        let plural = if fixable == 1 { "" } else { "s" };
        out.push('\n');
        let _ = writeln!(
            out,
            "{bold_open}note{reset}: {fixable} diagnostic{plural} above suggest{} what to write \
             instead — re-run with `--explain`",
            if fixable == 1 { "s" } else { "" }
        );
    }

    if error_count > 0 {
        let use_color = color.resolved();
        let color_open = if use_color { ANSI_RED } else { "" };
        let bold_open = if use_color { ANSI_BOLD } else { "" };
        let reset = if use_color { ANSI_RESET } else { "" };
        let plural = if error_count == 1 { "error" } else { "errors" };
        out.push('\n');
        let _ = writeln!(
            out,
            "{color_open}{bold_open}error{reset}{bold_open}: aborting due to {error_count} previous {plural}{reset}"
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(level: Level, msg: &str, file: &str, line: u32, col: u32) -> DiagEntry {
        DiagEntry {
            level,
            message: msg.to_string(),
            file: file.to_string(),
            line,
            col,
            code: None,
            suggestion: None,
            fixes: Vec::new(),
        }
    }

    #[test]
    fn pretty_renders_single_error_with_caret() {
        let mut loader = InMemoryLoader::new();
        loader.add("foo.loft", "fn main() {\n  let x = (1 + 2;\n}\n");
        let e = entry(Level::Error, "Expect token )", "foo.loft", 2, 14);
        let out = render_entry_pretty(&e, &loader, ColorMode::Off);
        let expected = "\
error: Expect token )
  --> foo.loft:2:14
  |
2 |   let x = (1 + 2;
  |              ^
";
        assert_eq!(out, expected);
    }

    #[test]
    fn pretty_handles_tabs_in_source_line() {
        let mut loader = InMemoryLoader::new();
        // Tab-indented `f`: `\t\tfoo` — caret at col 3 (the `f`)
        loader.add("t.loft", "\t\tfoo\n");
        let e = entry(Level::Error, "msg", "t.loft", 1, 3);
        let out = render_entry_pretty(&e, &loader, ColorMode::Off);
        // Tabs render as 4 spaces in source line; caret column = 8
        // (two tabs × 4 spaces; byte col 2 is the `f` start)
        assert!(
            out.contains("        foo"),
            "tabs should expand to 4 spaces in source, got:\n{out}"
        );
        // Caret at display column 8 (under the `f`)
        assert!(
            out.contains("|         ^"),
            "caret should land at display column 8, got:\n{out}"
        );
    }

    #[test]
    fn pretty_handles_multibyte_utf8_column() {
        let mut loader = InMemoryLoader::new();
        // 'ä' is 2 bytes in UTF-8; the caret at byte position 8 (after
        // "let x = ") should land at display col 8 (under `ä`).
        loader.add("u.loft", "let x = ä;\n");
        let e = entry(Level::Error, "msg", "u.loft", 1, 9);
        let out = render_entry_pretty(&e, &loader, ColorMode::Off);
        // display column for byte 8 is 8 (each ASCII char + ä each
        // count as 1).
        assert!(
            out.contains("|         ^"),
            "caret should land at display column 8 (under `ä`), got:\n{out}"
        );
    }

    #[test]
    fn pretty_omits_source_when_loader_returns_none() {
        let loader = InMemoryLoader::new();
        let e = entry(Level::Error, "msg", "missing.loft", 5, 1);
        let out = render_entry_pretty(&e, &loader, ColorMode::Off);
        // Should still have header + location, but no source/caret block
        assert!(out.contains("error: msg"));
        assert!(out.contains("--> missing.loft:5:1"));
        assert!(!out.contains('|'));
    }

    #[test]
    fn pretty_omits_location_when_file_empty() {
        let loader = InMemoryLoader::new();
        let e = entry(Level::Error, "no file", "", 0, 0);
        let out = render_entry_pretty(&e, &loader, ColorMode::Off);
        assert_eq!(out, "error: no file\n");
    }

    #[test]
    fn pretty_color_mode_emits_ansi_escapes() {
        let mut loader = InMemoryLoader::new();
        loader.add("c.loft", "x = 1;\n");
        let e = entry(Level::Error, "msg", "c.loft", 1, 1);
        let out = render_entry_pretty(&e, &loader, ColorMode::Always);
        assert!(out.contains("\x1b["), "should contain ANSI escapes");
        // Off mode produces no escapes.
        let out_off = render_entry_pretty(&e, &loader, ColorMode::Off);
        assert!(!out_off.contains("\x1b["), "Off mode should suppress ANSI");
    }

    #[test]
    fn cascade_dedup_drops_warning_at_same_position_as_preceding_error() {
        let loader = InMemoryLoader::new();
        let mut diags = Diagnostics::new();
        diags.add_at(Level::Error, "real bug", "f.loft", 5, 10);
        diags.add_at(Level::Warning, "noise", "f.loft", 5, 10);
        let out = render_pretty_all(&diags, &loader, ColorMode::Off);
        assert!(out.contains("real bug"));
        assert!(!out.contains("noise"));
    }

    #[test]
    fn summary_line_pluralises_correctly() {
        let loader = InMemoryLoader::new();
        let mut diags = Diagnostics::new();
        diags.add_at(Level::Error, "one", "f.loft", 1, 1);
        let out_one = render_pretty_all(&diags, &loader, ColorMode::Off);
        assert!(out_one.contains("aborting due to 1 previous error"));
        assert!(!out_one.contains("errors"));

        diags.add_at(Level::Error, "two", "f.loft", 2, 1);
        let out_two = render_pretty_all(&diags, &loader, ColorMode::Off);
        assert!(out_two.contains("aborting due to 2 previous errors"));
    }

    #[test]
    fn debug_entries_dropped_in_pretty_render() {
        let loader = InMemoryLoader::new();
        let mut diags = Diagnostics::new();
        diags.add_at(Level::Debug, "noise", "f.loft", 1, 1);
        diags.add_at(Level::Error, "real", "f.loft", 2, 1);
        let out = render_pretty_all(&diags, &loader, ColorMode::Off);
        assert!(!out.contains("noise"));
        assert!(out.contains("real"));
    }

    #[test]
    fn error_mode_parses_env_and_cli() {
        // CLI wins
        assert_eq!(
            ErrorMode::from_cli_and_env(Some("compact")),
            ErrorMode::Compact
        );
        assert_eq!(
            ErrorMode::from_cli_and_env(Some("pretty")),
            ErrorMode::Pretty
        );
        // Bogus CLI falls through to default
        assert_eq!(
            ErrorMode::from_cli_and_env(Some("garbage")),
            ErrorMode::Pretty
        );
    }
}
