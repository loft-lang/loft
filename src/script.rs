// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F48 — the loft CLI (run a program): the beginner-SCRIPT front of it — classify a
// no-`fn main` file and desugar it to one run-once `fn main` so `loft prog.loft` runs.

//! @PLN13 — beginner scripts: run a `.loft` file with loose top-level statements and
//! no `fn main`.
//!
//! A *script* is a `.loft` file with loose top-level statements (bare statements not
//! wrapped in a `fn`, and no explicit `fn main`). [`is_script`] CLASSIFIES a source
//! (step 1); [`script_desugar`] rewrites a script to one run-once `fn main` (step 2),
//! which `main.rs` parses instead of the file — auto-detected, no flag (step 3). Loose
//! statements may omit the trailing `;` at the script top level (step 4).
//!
//! The classification invariant that makes the desugar safe: **every file the current
//! compiler accepts classifies as NOT a script** — an all-defs library or a program
//! with `fn main`. So the desugar can only change source that is already rejected today,
//! and the whole existing corpus stays untouched. `is_script` is swept over
//! `default/*.loft` + `tests/**` in the test below to prove that.

// @PLN184 A1: not yet through `file_access` — this allow only goes (src/file_access/clippy_allow.baseline).
#![allow(clippy::disallowed_methods, clippy::disallowed_types)]
use crate::parser::Parser;

/// True when `src` is a beginner-style SCRIPT: it has ≥1 loose top-level statement
/// and defines no `fn main`. False for every all-defs or `fn main`-bearing file.
///
/// A file is NOT a script if any non-def item is a MALFORMED definition — a def with a
/// mistyped keyword (`funcion main() {…}`, `stru Foo {…}`) reads as a loose statement to
/// the split, but it is a broken program, not a script. Auto-detect must not desugar it
/// (that would bury the real "unknown keyword" error inside a synthesised `main`), so any
/// def-shaped item makes the whole file non-script and it parses — and errors — unchanged.
pub fn is_script(src: &str) -> bool {
    let items = split_top_level(src);
    if items.iter().any(|it| is_fn_main(it)) {
        return false;
    }
    let mut has_loose_stmt = false;
    for it in &items {
        if is_def_item(it) {
            continue;
        }
        if is_def_shaped(it) {
            return false; // a mistyped-keyword definition — not a script
        }
        has_loose_stmt = true;
    }
    has_loose_stmt
}

/// @PLN13 Step 2 — the script desugar. If `src` is a beginner script, return the
/// equivalent loft source: its top-level defs kept at top level, and its loose
/// statements collected into ONE `fn main()` that runs them once, in order, sharing
/// state. Returns `None` for a non-script (all-defs / has `fn main`), so the caller
/// leaves those untouched.
///
/// The top-level loose statements are split at newlines (step 4), so `;` is OPTIONAL
/// between them — each is terminated with `;` on emission. Statements NESTED inside a
/// block (`for i in … { a; b }`) still need their `;` for now; universal `;`-optional is
/// a later slice. (Line numbers shift in the desugared source — an error-position remap
/// is a later step.)
/// T0.2 — desugar a beginner script AND report where each generated line came
/// from, so a diagnostic can be shown in the USER's coordinates.
///
/// The desugar reorders items (defs hoisted) and inserts lines (the `fn main() {`
/// prologue, and a fresh-line `;` after every `;`-less statement), so generated
/// line N is not source line N — a 2-line script reports its second statement on
/// line 4.  That also silently loses the source snippet, because the renderer
/// looks up a line the user's file does not have: one cause, two symptoms.
///
/// Returns `(desugared_source, map)` where `map[i]` is the 1-based ORIGINAL line
/// that generated line `i + 1` came from.  A purely synthetic line (the prologue,
/// an inserted `;`, the closing brace) carries the nearest preceding original
/// line, so a diagnostic on one still points at the statement it belongs to.
pub fn script_desugar_mapped(src: &str) -> Option<(String, Vec<u32>)> {
    if !is_script(src) {
        return None;
    }
    // Original line of a slice OF `src`, from its byte offset — `split_top_level`
    // returns borrowed slices, so the offset is exact.
    let base = src.as_ptr() as usize;
    let line_of = |item: &str| -> u32 {
        let off = item.as_ptr() as usize - base;
        // `off` is the start of a token slice, so it is a char boundary.
        u32::try_from(src[..off].matches('\n').count() + 1).unwrap_or(1)
    };
    let (mut defs, mut body): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
    for it in split_top_level(src) {
        if is_def_item(it) {
            defs.push(it);
        } else {
            body.push(it);
        }
    }
    let mut out = String::new();
    let mut map: Vec<u32> = Vec::new();
    let mut last = 1u32;
    // Record `n` generated lines as coming from original line `orig`.
    let push = |out: &mut String, map: &mut Vec<u32>, text: &str, orig: u32| {
        out.push_str(text);
        // A pushed item may itself span several lines; every line it occupies maps
        // to a successive original line, since the item is a contiguous slice.
        let lines = text.matches('\n').count() + usize::from(!text.ends_with('\n'));
        for k in 0..lines.max(1) {
            map.push(orig + u32::try_from(k).unwrap_or(0));
        }
    };
    for d in defs.drain(..) {
        let orig = line_of(d);
        last = orig;
        push(&mut out, &mut map, d, orig);
        out.push('\n');
    }
    out.push_str("fn main() {\n");
    map.push(last); // synthetic prologue
    for st in body.drain(..) {
        let orig = line_of(st);
        last = orig;
        push(&mut out, &mut map, st, orig);
        if !st.trim_end().ends_with(';') {
            out.push_str("\n;");
            map.push(orig); // the inserted `;` belongs to its statement
        }
        out.push('\n');
    }
    out.push_str("}\n");
    map.push(last); // synthetic closing brace
    Some((out, map))
}

pub fn script_desugar(src: &str) -> Option<String> {
    script_desugar_mapped(src).map(|(out, _)| out)
}

/// Split `src` into its top-level items (source slices), skipping comments and string
/// contents. Each item is a def (with any leading `#` annotations) or a loose statement.
///
/// A DEF/const item is scanned to the `}` that closes its top-level body back to depth 0,
/// its depth-0 `;`, or EOF — with NO newline boundary, so a brace-less fn body
/// (`fn f() -> text\n  return x`) stays one item. A loose STATEMENT is scanned the same
/// way but ALSO ends at the first depth-0 newline where what has been scanned is a
/// complete statement — so a `;`-less script (`count = 0\nprint(count)`, @PLN13 step 4)
/// splits into one item per line, while a `;`-less multi-line expression (a trailing
/// operator, or an open bracket) is held together by [`Parser::statement_incomplete`].
pub fn split_top_level(src: &str) -> Vec<&str> {
    let b = src.as_bytes();
    let n = b.len();
    let mut items = Vec::new();
    // A leading `#!…` shebang line is not loft source (scripts and `fn main` files
    // alike may carry one) — skip it before the first item.
    let mut i = 0;
    if b.starts_with(b"#!") {
        while i < n && b[i] != b'\n' {
            i += 1;
        }
    }
    i = skip_trivia(b, i);
    while i < n {
        let start = i;
        // Decide def-vs-statement at the START (before the end is known) so a loose
        // statement gets the newline boundary while a def does not — this is what keeps
        // a brace-less def whole AND lets `;`-less loose statements separate correctly,
        // even a def sitting between two of them.
        i = scan_item_end(src, start, !item_at_is_def(src, start));
        if i <= start {
            break; // no-progress guard (should not happen)
        }
        let slice = src[start..i].trim();
        if !slice.is_empty() {
            items.push(slice);
        }
        i = skip_trivia(b, i);
    }
    items
}

/// Peek whether the top-level item beginning at `start` is a DEF or implicit top-level
/// constant (scanned to a brace/`;`, so a brace-less body survives) rather than a loose
/// statement (which gets the `;`-less newline boundary). Mirrors [`is_def_item`] but
/// decides BEFORE the item's end is known: it inspects only the leading keyword/name, so
/// passing the rest of the source is safe.
fn item_at_is_def(src: &str, start: usize) -> bool {
    let core = strip_leading_annotations(&src[start..]);
    core.is_empty() || Parser::starts_top_level_def(core) || is_top_level_const(core)
}

/// Whitespace, `//` line comments, and `/* */` block comments (the corpus uses all
/// three). Returns the offset of the next significant byte.
fn skip_trivia(b: &[u8], mut i: usize) -> usize {
    let n = b.len();
    loop {
        while i < n && b[i].is_ascii_whitespace() {
            i += 1;
        }
        if i + 1 < n && b[i] == b'/' && b[i + 1] == b'/' {
            while i < n && b[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if i + 1 < n && b[i] == b'/' && b[i + 1] == b'*' {
            i += 2;
            while i + 1 < n && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(n);
            continue;
        }
        return i;
    }
}

/// The offset just past the `"` string literal whose opening quote is at `open`.
///
/// A loft string is not flat. `{ … }` opens a format HOLE whose contents are CODE, and code
/// can contain another string — so `"got: {shout("a\"b")}"` is ONE literal. Stopping at the
/// first unescaped `"` ends it after `shout(`, and the rest of the line is then read as code
/// with a stray quote in it.
///
/// The cost of getting it wrong is not a bad boundary, it is a misclassified FILE:
/// `split_top_level` ended the item early, so the `fn main` that followed no longer STARTED
/// an item, `is_script` stopped seeing it, and an ordinary program was desugared as a
/// beginner script. The symptom was a lexer error — *"String not correctly terminated"* — on
/// a line the compiler accepts one function up, which is why it read as a lexer defect
/// (loft#1271). The lexer is fine: `Lexer::hole_closes_on_this_line` already keeps this
/// stack, and moving the same line above `fn main` compiles it.
///
/// The rule, mirroring that function: inside a string, `{` opens a hole (`{{` and `}}` are
/// literal braces) and the literal ends only at a `"` seen at hole depth 0; inside a hole,
/// `"` opens a nested string, `{`/`}` nest as code braces, and `'` / `` ` `` open literals
/// that cannot contain one.
fn scan_string_end(b: &[u8], open: usize) -> usize {
    let n = b.len();
    // `true` = inside a string literal, `false` = inside a `{…}` hole (code).
    let mut ctx = vec![true];
    let mut i = open + 1;
    while i < n {
        if b[i] == b'\\' {
            i += 2;
            continue;
        }
        if *ctx.last().unwrap_or(&true) {
            match b[i] {
                b'{' | b'}' if b.get(i + 1).copied() == Some(b[i]) => i += 2,
                b'{' => {
                    ctx.push(false);
                    i += 1;
                }
                b'"' => {
                    ctx.pop();
                    if ctx.is_empty() {
                        return i + 1;
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        } else {
            match b[i] {
                b'"' => {
                    ctx.push(true);
                    i += 1;
                }
                b'{' => {
                    ctx.push(false);
                    i += 1;
                }
                b'}' => {
                    ctx.pop();
                    i += 1;
                }
                // A char or raw-string literal inside a hole cannot itself hold a hole, so
                // it is skipped whole rather than pushed.
                b'\'' | b'`' => {
                    let close = b[i];
                    i += 1;
                    while i < n && b[i] != close {
                        i += if b[i] == b'\\' { 2 } else { 1 };
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
    }
    n
}

/// Scan one top-level item from `start`, tracking bracket depth and skipping string
/// contents (both `"…"` and `` `…` `` raw strings) and comments, and return the offset
/// just past its end: the first depth-0 `;`, the `}` that closes a top-level body back
/// to depth 0, or EOF.
///
/// When `stmt_boundary` is true (a loose statement, not a def), the scan ALSO ends at the
/// first depth-0 newline where `src[start..]` is already a complete statement — the
/// `;`-less script boundary (@PLN13 step 4). [`Parser::statement_incomplete`] holds a
/// multi-line expression together (a trailing binary operator, or an unclosed bracket),
/// so only a genuinely finished line ends the item.
///
/// When `stmt_boundary` is false (a def/const), there is deliberately no newline boundary,
/// so a brace-less fn body (`fn f() -> text\n  return x`) stays one item.
///
/// loft#736 — the depth-0 `}` does NOT end the item when an `else` follows it. `else` is
/// the one word that can never BEGIN a statement, so seeing it after the closing brace
/// means the statement continues; ending there split `if c { … } else { … }` into two
/// items and the second parsed as a bare `else` ("Expect token ;"). The same split hit
/// the expression form (`y = if c { 1 } else { 0 }`), which is why the `if` half alone
/// appeared to work. The loop re-checks after each arm, so `else if … else …` chains
/// stay whole.
fn scan_item_end(src: &str, start: usize, stmt_boundary: bool) -> usize {
    let b = src.as_bytes();
    let n = b.len();
    let mut i = start;
    let mut depth: i32 = 0;
    while i < n {
        match b[i] {
            b'"' => {
                i = scan_string_end(b, i);
                continue;
            }
            b'`' => {
                // a backtick raw string (loft's multi-line string — shader sources
                // in `graphics/render.loft` embed GLSL braces that must NOT count).
                i += 1;
                while i < n && b[i] != b'`' {
                    i += 1;
                }
                i += 1;
                continue;
            }
            b'/' if i + 1 < n && b[i + 1] == b'/' => {
                while i < n && b[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if i + 1 < n && b[i + 1] == b'*' => {
                i += 2;
                while i + 1 < n && !(b[i] == b'*' && b[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(n);
                continue;
            }
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' => depth -= 1,
            b'}' => {
                depth -= 1;
                if depth <= 0 && !continues_with_else(b, i + 1) {
                    return i + 1; // a top-level body closed
                }
            }
            b';' if depth == 0 => return i + 1,
            // a `;`-less loose statement ends at the first depth-0 newline where it is
            // already complete; a false guard (incomplete — trailing operator / open
            // bracket) falls through to `_` and the scan continues onto the next line.
            // loft#736 — a following `else` is the third way it is not finished, and
            // the only one visible AFTER the boundary rather than before it, so
            // `statement_incomplete` (which reads what precedes) cannot see it. This
            // is the `else`-on-its-own-line layout, comment in between included.
            b'\n'
                if stmt_boundary
                    && depth == 0
                    && !Parser::statement_incomplete(src[start..i].trim())
                    && !continues_with_else(b, i) =>
            {
                return i;
            }
            _ => {}
        }
        i += 1;
    }
    n
}

/// loft#736 — is the next significant word after offset `i` the keyword `else`?
///
/// Used at the depth-0 `}` boundary in [`scan_item_end`]: a statement can never START
/// with `else`, so an `else` there continues the item rather than opening a new one.
/// Trivia (whitespace and both comment forms) is skipped first, so a brace and its
/// `else` may sit on separate lines with a comment between. The word-boundary check
/// keeps an identifier that merely begins with those letters (`elsewhere`) from
/// matching — that one really is a new statement.
fn continues_with_else(b: &[u8], i: usize) -> bool {
    let at = skip_trivia(b, i);
    let end = at + 4;
    b[at..].starts_with(b"else")
        && b.get(end)
            .is_none_or(|c| !c.is_ascii_alphanumeric() && *c != b'_')
}

/// Skip leading `#annotation` / `#directive`s and the trivia (whitespace + comments)
/// around them, returning the def keyword underneath. Annotations take three forms:
/// bare (`#pure`, `#cwd`), a `"…"` payload (`#rust"impl"`), or a `(…)` payload
/// (`#impure(host_io)`); native stdlib fns carry a POST-fix `#rust"…"` that lands at
/// the head of the NEXT item (`#rust"prev" // comment  fn next() -> T;`), so the
/// comment between must be skipped too.
fn strip_leading_annotations(item: &str) -> &str {
    let b = item.as_bytes();
    let n = b.len();
    let mut i = skip_trivia(b, 0);
    while i < n && b[i] == b'#' {
        i += 1;
        while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
            i += 1;
        }
        while i < n && (b[i] == b' ' || b[i] == b'\t') {
            i += 1;
        }
        // An annotation's arguments, of which there may be MORE THAN ONE:
        // `#c "strlen" "size_t(const char*)"` takes two strings (@PLN24). Reading
        // only the first left the second looking like a loose statement, so a
        // library of `#c` declarations classified as a beginner script.
        loop {
            match b.get(i) {
                Some(b'"') => {
                    i += 1;
                    while i < n && b[i] != b'"' {
                        i += if b[i] == b'\\' { 2 } else { 1 };
                    }
                    i += 1;
                }
                Some(b'(') => {
                    let mut d = 0i32;
                    while i < n {
                        match b[i] {
                            b'(' => d += 1,
                            b')' => {
                                d -= 1;
                                i += 1;
                                if d == 0 {
                                    break;
                                }
                                continue;
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                }
                _ => break,
            }
            // Another argument only if a space separates it from the last one; a
            // newline ends the annotation.
            let mut j = i;
            while j < n && (b[j] == b' ' || b[j] == b'\t') {
                j += 1;
            }
            if j < n && (b[j] == b'"' || b[j] == b'(') {
                i = j;
            } else {
                break;
            }
        }
        i = skip_trivia(b, i);
    }
    item[i..].trim_start()
}

/// A top-level item is a DEF (a def keyword, possibly annotation-prefixed), a bare
/// directive (a lone `#cwd`), or an implicit top-level CONSTANT
/// (`UPPER = …` / `UPPER: T = …` — loft accepts these without a `const` keyword, which
/// is why a lowercase `x = 5` is instead a script statement). Anything else is loose.
fn is_def_item(item: &str) -> bool {
    let core = strip_leading_annotations(item);
    core.is_empty() || Parser::starts_top_level_def(core) || is_top_level_const(core)
}

/// `UPPER_IDENT = …` or `UPPER_IDENT: T = …` — loft's implicit uppercase-named
/// top-level constant. A LOWER-case leading name is NOT a const (it is a script
/// statement or a compile error), which is exactly the script-vs-not distinction.
fn is_top_level_const(core: &str) -> bool {
    let mut ci = core.char_indices();
    let Some((_, first)) = ci.next() else {
        return false;
    };
    if !first.is_ascii_uppercase() {
        return false;
    }
    let mut end = first.len_utf8();
    for (idx, c) in ci {
        if c.is_alphanumeric() || c == '_' {
            end = idx + c.len_utf8();
        } else {
            break;
        }
    }
    let after = core[end..].trim_start();
    after.starts_with('=') || after.starts_with(':')
}

/// True when the item has the SHAPE of a definition with a mistyped keyword: after
/// stripping annotations it begins with two identifier tokens — `<ident> <ident>` — where
/// the first is NOT a loft keyword (`funcion main`, `stru Foo`, `fnn f`). Every real
/// definition keyword is caught by [`is_def_item`] already, and no valid loose statement
/// begins with two bare identifiers: an assignment is `ident = …`, a call is `ident(…)`,
/// a method/index is `ident.` / `ident[`, and a keyword statement (`if x { … }`,
/// `for i in …`, `return x`, `while c { … }`, `match v { … }`) begins with a KEYWORD,
/// which the keyword guard below excludes. So this flags exactly the mistyped-keyword
/// definitions, and nothing a beginner would actually write as a statement.
fn is_def_shaped(item: &str) -> bool {
    let core = strip_leading_annotations(item);
    let b = core.as_bytes();
    let n = b.len();
    // first token must be an identifier start
    if n == 0 || !(b[0].is_ascii_alphabetic() || b[0] == b'_') {
        return false;
    }
    let mut i = 0;
    while i < n && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
        i += 1;
    }
    // a keyword-led item (`if`, `for`, `return`, …) is a real statement, not a def shape
    if crate::lexer::is_keyword(&core[..i]) {
        return false;
    }
    // require whitespace, then a second identifier start
    if i >= n || !b[i].is_ascii_whitespace() {
        return false;
    }
    while i < n && b[i].is_ascii_whitespace() {
        i += 1;
    }
    i < n && (b[i].is_ascii_alphabetic() || b[i] == b'_')
}

/// True when the item is `fn main` (after stripping annotations): `fn` then the name
/// `main` at a word boundary.
fn is_fn_main(item: &str) -> bool {
    let core = strip_leading_annotations(item);
    let Some(rest) = core.strip_prefix("fn") else {
        return false;
    };
    let rest = rest.trim_start();
    rest.strip_prefix("main")
        .is_some_and(|r| !r.starts_with(|c: char| c.is_alphanumeric() || c == '_'))
}

#[cfg(test)]
mod tests {
    use super::{is_script, scan_string_end, split_top_level};

    // ── injected-fault controls ──────────────────────────────────────────────
    #[test]
    fn loose_statements_are_a_script() {
        assert!(is_script("print(\"hi\")\n"));
        assert!(is_script("x = 5\nprint(\"x={x}\")\n"));
    }
    #[test]
    fn all_defs_is_not_a_script() {
        assert!(!is_script("fn helper() { print(\"hi\") }\n"));
        assert!(!is_script("struct P { x: integer }\nenum E { A, B }\n"));
    }
    #[test]
    fn operator_definitions_are_not_a_script() {
        // C132 (@PLN182) — `operator` begins a definition as `fn` does.  A file of definitions
        // with no `main` (a library, a `pub fn test()` corpus case) read as a script and was
        // desugared around its operators once `operator` became a keyword.
        assert!(!is_script(
            "struct M { v: integer }\noperator plus(self: M, o: M) -> M { M { v: self.v + o.v } }\n\
             pub fn test() { }\n"
        ));
        assert!(!is_script(
            "pub operator to_text(self: integer, spec: text) -> text { \"\" }\n"
        ));
    }
    #[test]
    fn explicit_main_is_not_a_script() {
        // even mixed with a loose-looking line, an explicit main opts out.
        assert!(!is_script("fn main() { print(\"hi\") }\n"));
    }
    #[test]
    fn annotated_and_commented_defs_are_not_scripts() {
        assert!(!is_script("#pure\nfn f() -> integer { 1 }\n"));
        assert!(!is_script("#cwd\nfn main() { print(\"hi\") }\n"));
        assert!(!is_script("// a comment\n/* block */\nfn f() {}\n"));
        // a def whose body/string contains statement-like text must not fool it.
        assert!(!is_script("fn f() { let s = \"x = 5\"; print(s) }\n"));
    }
    #[test]
    fn implicit_uppercase_constants_are_not_scripts() {
        // the hex_world / graphics corpus shape: top-level `UPPER = …` (+ typed, + arrays).
        assert!(!is_script("DX = 20.0;\nDY = -34.6;\n"));
        assert!(!is_script("STEP = [\n0, 4,\n2, 4,\n];\n"));
        assert!(!is_script(
            "MAX: integer = 10;\nfn f() -> integer { MAX }\n"
        ));
        // a LOWER-case top-level assignment IS a script statement (the distinction).
        assert!(is_script("total = 0\ntotal = total + 1\n"));
    }
    #[test]
    fn backtick_strings_and_braceless_bodies_are_not_scripts() {
        // graphics/render.loft: a const holding a backtick shader with GLSL braces.
        assert!(!is_script("SHADOW = `\nvoid main() {{ x = 1; }}\n`;\n"));
        // native_crate_pkg.loft: a brace-less fn body.
        assert!(!is_script("pub fn hi() -> text\n    return \"hi\"\n"));
    }
    /// loft#736 — an `else` after the depth-0 `}` CONTINUES the item.
    ///
    /// The split ended every item at the brace that closed it back to depth 0, so
    /// `else { … }` came out as a second item and parsed as a bare `else`.  Both
    /// boundaries had to learn it: the brace itself (same-line `else`) and the
    /// `;`-less newline rule (`else` on its own line) — `statement_incomplete` reads
    /// what PRECEDES the boundary and so can never see this continuation.
    #[test]
    fn else_continues_the_item() {
        // same line
        assert_eq!(
            split_top_level("if x > 2 { a() } else { b() }\n"),
            vec!["if x > 2 { a() } else { b() }"]
        );
        // own line, and across a comment
        assert_eq!(
            split_top_level("if x > 2 {\n  a()\n}\nelse {\n  b()\n}\n").len(),
            1
        );
        assert_eq!(
            split_top_level("if x > 2 { a() } // pick\nelse { b() }\n").len(),
            1
        );
        // an `else if` chain is still one item
        assert_eq!(
            split_top_level("if x > 2 {\n a()\n}\nelse if x > 0 {\n b()\n}\nelse {\n c()\n}\n")
                .len(),
            1
        );
        // the expression form — the same boundary broke `y = if … else …`.  The item
        // ends at the brace, so the trailing `;` is its own (empty) item, as it has
        // always been for a brace-terminated statement; what matters is that the
        // `else` arm stays with the `if`.
        assert_eq!(
            split_top_level("y = if x > 2 { 1 } else { 0 };\n")[0],
            "y = if x > 2 { 1 } else { 0 }"
        );
        // …and the boundary must still SPLIT where there is no `else`: two adjacent
        // `if`s stay two statements, and an identifier that merely starts with those
        // letters is not a continuation.
        assert_eq!(
            split_top_level("if x > 2 { a() }\nif x > 1 { b() }\n").len(),
            2
        );
        assert_eq!(
            split_top_level("if x > 2 { a() }\nelsewhere = 4\n").len(),
            2
        );
        // The invariant behind all of the above, stated directly: no item may BEGIN
        // with `else` — that is the shape that reached the parser as a bare `else`.
        for src in [
            "if x > 2 { a() } else { b() }\n",
            "if x > 2 {\n a()\n}\nelse {\n b()\n}\n",
            "if x > 2 { a() } // pick\nelse { b() }\n",
            "y = if x > 2 { 1 } else { 0 };\n",
        ] {
            assert!(
                !split_top_level(src).iter().any(|it| it.starts_with("else")),
                "an item began with `else`: {src:?} -> {:?}",
                split_top_level(src)
            );
        }
    }

    // ── the desugar (Step 2) ─────────────────────────────────────────────────
    #[test]
    fn desugar_none_for_non_scripts() {
        assert_eq!(super::script_desugar("fn main() { print(\"hi\") }\n"), None);
        assert_eq!(
            super::script_desugar("fn f() {}\nstruct S { x: integer }\n"),
            None
        );
    }
    #[test]
    /// T0.2 — the desugar's line map puts every generated line back on the source
    /// line it came from, so a diagnostic can be reported in the user's
    /// coordinates.  The prologue / inserted `;` / closing brace are synthetic and
    /// carry the nearest preceding original line.
    fn t02_line_map_tracks_the_source() {
        // 2 loose statements, no defs — the shape the review's repro uses.
        let (out, map) =
            super::script_desugar_mapped("name = \"world\"\nprintt(\"hi\")\n").unwrap();
        let g: Vec<&str> = out.lines().collect();
        assert_eq!(g[0], "fn main() {");
        assert_eq!(map[0], 1); // synthetic prologue -> first source line
        assert_eq!(g[1], "name = \"world\"");
        assert_eq!(map[1], 1);
        assert_eq!(g[2], ";");
        assert_eq!(map[2], 1); // the inserted `;` belongs to its statement
        assert_eq!(g[3], "printt(\"hi\")");
        assert_eq!(map[3], 2); // THE point: generated line 4 is source line 2
        // A hoisted def is emitted first but still maps to where it was written.
        let (out2, map2) =
            super::script_desugar_mapped("a = 1\nfn helper() { 1 }\nb = 2\n").unwrap();
        let g2: Vec<&str> = out2.lines().collect();
        assert_eq!(g2[0], "fn helper() { 1 }");
        assert_eq!(map2[0], 2, "the hoisted def maps back to its source line");
        let b_at = g2
            .iter()
            .position(|l| *l == "b = 2")
            .expect("b = 2 emitted");
        assert_eq!(map2[b_at], 3, "a statement after a hoisted def still maps");
    }

    #[test]
    fn desugar_all_loose_into_one_main() {
        let out = super::script_desugar("print(\"a\");\nprint(\"b\");\n").unwrap();
        assert_eq!(out, "fn main() {\nprint(\"a\");\nprint(\"b\");\n}\n");
    }
    #[test]
    fn desugar_hoists_defs_and_keeps_statement_order() {
        // a def BETWEEN two loose statements is hoisted to top level; the loose
        // statements keep their order in `main`.
        let out =
            super::script_desugar("print(\"a\");\nfn helper() { 1 }\nprint(\"b\");\n").unwrap();
        assert_eq!(
            out,
            "fn helper() { 1 }\nfn main() {\nprint(\"a\");\nprint(\"b\");\n}\n"
        );
    }

    #[test]
    fn mistyped_def_keyword_is_not_a_script() {
        // a typo'd def keyword reads as a loose statement to the split, but it is a
        // broken program — auto-detect must leave it for the compiler's real error,
        // not desugar it into a confusing one inside a synthesised `main` (#03 of the
        // error-message corpus: `funcion main() { … }`).
        assert!(!is_script("funcion main() {\n  print(\"x\");\n}\n"));
        assert!(!is_script("stru Point { x: integer }\n"));
        assert!(!is_script("fnn helper() { 1 }\n"));
        // but a keyword-led statement is still a real script (the guard must not over-reach)
        assert!(is_script("if true {\n  print(\"y\");\n}\n"));
        assert!(is_script("for i in 0..3 {\n  print(\"{i}\");\n}\n"));
        assert!(is_script("total = 0;\nreturn total;\n"));
    }

    // ── `;`-optional at the script top level (Step 4) ────────────────────────
    #[test]
    fn semicolon_less_statements_split_at_newlines() {
        // a `;`-less script splits into one loose item per line.
        let items = split_top_level("count = 0\nprint(count)\n");
        assert_eq!(items, vec!["count = 0", "print(count)"], "{items:?}");
        // a def sitting BETWEEN two `;`-less statements is still separated cleanly.
        let items = split_top_level("print(\"a\")\nfn helper() { 1 }\nprint(\"b\")\n");
        assert_eq!(
            items,
            vec!["print(\"a\")", "fn helper() { 1 }", "print(\"b\")"],
            "{items:?}"
        );
    }
    #[test]
    fn semicolon_less_multiline_expression_stays_one_item() {
        // a trailing binary operator continues the statement onto the next line, so the
        // newline is NOT a boundary (statement_incomplete holds it together).
        let items = split_top_level("x = 1 +\n2\nprint(x)\n");
        assert_eq!(items, vec!["x = 1 +\n2", "print(x)"], "{items:?}");
        // an unclosed bracket likewise continues across newlines.
        let items = split_top_level("v = [\n1,\n2,\n]\nprint(v)\n");
        assert_eq!(items, vec!["v = [\n1,\n2,\n]", "print(v)"], "{items:?}");
    }
    #[test]
    fn desugar_terminates_semicolon_less_body() {
        // each `;`-less statement is terminated with `;` on a fresh line (comment-safe).
        let out = super::script_desugar("count = 0\nprint(count)\n").unwrap();
        assert_eq!(
            out, "fn main() {\ncount = 0\n;\nprint(count)\n;\n}\n",
            "{out}"
        );
        // an already-`;`-terminated statement is left exactly as written (no extra `;`).
        let out = super::script_desugar("count = 0;\nprint(count);\n").unwrap();
        assert_eq!(out, "fn main() {\ncount = 0;\nprint(count);\n}\n", "{out}");
    }

    #[test]
    fn shebang_is_skipped() {
        // a shebang program keeps its `fn main` verdict; a shebang script is a script.
        assert!(!is_script(
            "#!/usr/bin/env loft\nfn main() { print(\"hi\") }\n"
        ));
        assert!(is_script("#!/usr/bin/env loft\nprint(\"hi\")\n"));
    }
    #[test]
    fn split_counts_top_level_items() {
        let items = split_top_level("fn a() {}\nfn b() {}\nstruct S { x: integer }\n");
        assert_eq!(items.len(), 3, "items: {items:?}");
    }

    /// A format hole holding a string with an ESCAPED QUOTE is one literal, and the item
    /// scan must not end inside it (loft#1271).
    ///
    /// `"got: {shout("a\"b")}"` is a single string: `{` opens a hole, the `"` inside it
    /// opens a NESTED string, and only the `"` at hole depth 0 closes the literal. Reading
    /// it flat ended the item after `shout(`, so the `fn main` that followed no longer
    /// STARTED an item, `is_script` stopped seeing it, and an ordinary program was desugared
    /// as a beginner script — surfacing as a LEXER error on a line the compiler accepts.
    ///
    /// Order is what made it look mysterious and is why both orders are here: with `fn main`
    /// FIRST the misplaced boundary falls after it, `is_fn_main` still matches, and the same
    /// bytes compile.
    #[test]
    fn an_escaped_quote_inside_a_hole_does_not_end_the_item() {
        let probe_first = "fn shout(v: text) -> text { \"{v}!\" }\n\
                           fn probe() { println(\"got: {shout(\"a\\\"b\")}\"); }\n\
                           fn main() { probe(); }\n";
        assert!(
            !is_script(probe_first),
            "a file whose `fn main` follows the escaped quote is not a script: {:?}",
            split_top_level(probe_first)
        );
        assert_eq!(
            split_top_level(probe_first).len(),
            3,
            "three defs, and the escaped quote splits none of them: {:?}",
            split_top_level(probe_first)
        );

        // The order that always worked — the control for the sentence above.
        let main_first = "fn shout(v: text) -> text { \"{v}!\" }\n\
                          fn main() { probe(); }\n\
                          fn probe() { println(\"got: {shout(\"a\\\"b\")}\"); }\n";
        assert!(
            !is_script(main_first),
            "control: main first is not a script"
        );
        assert_eq!(split_top_level(main_first).len(), 3, "control: three items");
    }

    /// The scanner's own rule, read directly rather than through `is_script`: `{{`/`}}` are
    /// literal braces, a hole nests, and a string inside a hole is skipped whole.
    #[test]
    fn scan_string_end_spans_holes_and_their_nested_strings() {
        let cases: &[&str] = &[
            r#""plain""#,
            // the loft#1271 shape: a hole holding a string that holds an escaped quote
            r#""got: {shout("a\"b")}""#,
            // `{{`/`}}` are literal braces and open no hole
            r#""{{literal}} {f("x")}""#,
            // a `}` INSIDE a nested string does not close the hole around it
            r#""{f("}")}""#,
            // a plain escape, with no hole anywhere
            r#""esc \" inside""#,
        ];
        for src in cases {
            let b = src.as_bytes();
            assert_eq!(
                scan_string_end(b, 0),
                b.len(),
                "the whole literal is one string: {src}"
            );
        }
        // Two literals side by side stop at the first one's own closing quote.
        let two = r#""a" + "b""#;
        assert_eq!(
            scan_string_end(two.as_bytes(), 0),
            3,
            "first literal is `\"a\"`"
        );
    }

    // ── the corpus sweep: EVERY file the compiler accepts must be NOT-a-script ──
    #[test]
    fn no_corpus_file_classifies_as_script() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut misclassified = Vec::new();
        for dir in ["default", "tests"] {
            let mut stack = vec![root.join(dir)];
            while let Some(d) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&d) else {
                    continue;
                };
                for e in entries.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        stack.push(p);
                    } else if p.extension().is_some_and(|x| x == "loft") {
                        // The invariant is about files the compiler ACCEPTS. Skip the
                        // deliberately-invalid fixtures: the error-message corpus and
                        // any `@EXPECT_ERROR` / `@EXPECT_FAIL` test.
                        if p.components().any(|c| c.as_os_str() == "error_messages") {
                            continue;
                        }
                        if let Ok(src) = std::fs::read_to_string(&p) {
                            // `@EXPECT_ERROR`/`@EXPECT_FAIL` = deliberately invalid;
                            // `@SCRIPT` = an intentional `--script`-only fixture (not
                            // accepted by the plain compiler, so outside the invariant).
                            if src.contains("@EXPECT_ERROR")
                                || src.contains("@EXPECT_FAIL")
                                || src.contains("@SCRIPT")
                            {
                                continue;
                            }
                            if is_script(&src) {
                                misclassified.push(p.display().to_string());
                            }
                        }
                    }
                }
            }
        }
        assert!(
            misclassified.is_empty(),
            "these accepted-by-the-compiler files wrongly classified as scripts:\n{}",
            misclassified.join("\n")
        );
    }
}
