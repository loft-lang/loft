// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F42 — JSON (json_parse / JsonValue / to_json)
//
// JSON parser used by the `json_parse` native function in
// `src/native.rs`.  Walks UTF-8 text once and returns a `Parsed`
// value that the caller materialises into a loft `JsonValue`
// struct-enum record.
//
// Step 4 scope: full RFC 8259 — null, true, false, number, string
// (incl. standard escapes), array, object.  The `Parsed` tree is
// fully recursive here; `native::n_json_parse` flattens it into
// the arena-indexed loft JsonValue form at materialisation time.
//
// Q1 (this commit): parse failures carry a JSON Pointer path
// (RFC 6901) plus the byte offset.  Line:column and the
// surrounding context snippet are computed by `format_error`
// at error-formatting time, not per token, so the success path
// pays nothing.

/// Intermediate tree produced by [`parse`].  The loft-level
/// `JsonValue` variants are built from these values inside
/// `native::n_json_parse` so this module stays free of database
/// concerns.
///
/// The `Ident` variant is produced only by `Dialect::Lenient`
/// for a bare identifier in value position (e.g. `Daily` in
/// `{category: Daily}`, where Daily is a loft enum tag).  The
/// distinction is preserved so the walker can dispatch
/// strictly: text fields accept `Str` only, enum fields accept
/// either `Str` or `Ident`.  `Dialect::Strict` never emits
/// `Ident`.
#[derive(Debug, Clone)]
pub enum Parsed {
    Null,
    Bool(bool),
    Number(f64),
    /// @PLN109 — an integer-shaped JSON number (no `.`, no exponent) that fits
    /// `i64`.  Preserves the exact integer through deserialize (fixes @PLN102 H5:
    /// values > 2⁵³ used to round through `f64`).  A number with a fraction or
    /// exponent, or one that overflows `i64`, stays [`Number`](Self::Number).
    Int(i64),
    Str(String),
    Ident(String),
    Array(Vec<Parsed>),
    /// Object entries carry the byte offset of the key within the
    /// original input — used by the schema walker to produce
    /// `"line N:M path:X"` diagnostics on shape mismatches without
    /// re-scanning the source.  Tuple shape: `(name, key_byte_offset, value)`.
    Object(Vec<(String, usize, Parsed)>),
    /// A type-tagged constructor `Tag { … }` (Lenient only), kept **distinct**
    /// from `Object` so a struct type-tag (`Point{…}`) or enum-struct variant
    /// (`Red{…}`) is unambiguous against a plain object with a field named like
    /// the tag — the property that lets new dumps carry type tags while old
    /// (un-tagged) dumps still read as fields.  Fields: `(tag, tag_byte_offset,
    /// body)`, where `body` is the `{ … }` [`Parsed::Object`].
    Constructor(String, usize, Box<Parsed>),
}

impl Parsed {
    /// The integer value of a numeric leaf: an exact `i64` from [`Int`](Self::Int)
    /// (H5-preserved), or a truncated [`Number`](Self::Number) (backward-compatible
    /// with the pre-@PLN109 float path).  `None` for a non-numeric value.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Parsed::Int(n) => Some(*n),
            Parsed::Number(n) => Some(*n as i64),
            _ => None,
        }
    }

    /// The float value of a numeric leaf: [`Number`](Self::Number) as-is, or an
    /// [`Int`](Self::Int) widened to `f64`.  `None` for a non-numeric value.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Parsed::Number(n) => Some(*n),
            Parsed::Int(n) => Some(*n as f64),
            _ => None,
        }
    }
}

/// Input dialect selector.
///
/// * `Strict` — RFC 8259 JSON.  Object keys must be quoted
///   strings, no extensions.  This is what `json_parse(text)`
///   uses and is the public surface for user-supplied JSON.
/// * `Lenient` — accepts the same grammar as `Strict` *plus*
///   loft's bare-identifier object keys (`{val: 7}`) that the
///   legacy `vector<T>.parse(text)` path has supported since
///   day one.  This keeps loft-authored data literals compiling
///   through the unified parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Dialect {
    #[default]
    Strict,
    Lenient,
}

/// Structured parse error.  `path` is an RFC 6901 JSON Pointer
/// to the location in the input where parsing gave up — `""`
/// means "at the root", `/users/3/age` means "third element of
/// the `users` array's `age` field".  `byte_offset` is the
/// absolute byte position; line:column + context snippet are
/// derived by [`format_error`] at error-formatting time so the
/// success path pays nothing.
#[derive(Debug, Clone)]
pub struct ParseError {
    pub message: String,
    pub byte_offset: usize,
    pub path: String,
}

/// Parse the entire `input` as a JSON value in strict RFC 8259
/// mode.  Equivalent to `parse_with(input, Dialect::Strict)`.
///
/// Leading and trailing whitespace is allowed.  Characters after
/// the value (other than whitespace) are a syntax error — strict
/// RFC 8259, not a forgiving tokeniser.
///
/// # Errors
/// Returns a [`ParseError`] when the input is not valid JSON.
/// The `path` field localises the failure inside the document
/// (RFC 6901 JSON Pointer); the `byte_offset` field locates it
/// inside the raw text.
pub fn parse(input: &str) -> Result<Parsed, ParseError> {
    parse_with(input, Dialect::Strict)
}

/// Parse the entire `input` as a JSON value using the given
/// [`Dialect`].  See [`Dialect`] for the differences between
/// `Strict` (RFC 8259) and `Lenient` (loft data literals).
///
/// # Errors
/// Returns a [`ParseError`] when the input is not valid in the
/// chosen dialect.
pub fn parse_with(input: &str, dialect: Dialect) -> Result<Parsed, ParseError> {
    // @PLN109 Phase 2 — driven by loft's own JSON-mode lexer (`parse_lexer`),
    // proven byte-identical to the retired byte-scanner over the Phase-0 corpus.
    parse_lexer(input, dialect)
}

/// Render the path stack as an RFC 6901 JSON Pointer.  Empty
/// stack → `""` (root).  Each segment is escaped: `~` → `~0`,
/// `/` → `~1`.
fn render_path(stack: &[String]) -> String {
    if stack.is_empty() {
        return String::new();
    }
    let mut out = String::with_capacity(stack.iter().map(|s| s.len() + 1).sum());
    for seg in stack {
        out.push('/');
        for ch in seg.chars() {
            match ch {
                '~' => out.push_str("~0"),
                '/' => out.push_str("~1"),
                _ => out.push(ch),
            }
        }
    }
    out
}

/// Convert a byte offset into 1-based (line, column).  Line
/// counts `\n`; column counts bytes since the last newline + 1.
/// Out-of-range offsets clamp to the input length.
#[must_use]
pub fn line_col_of(input: &str, byte_offset: usize) -> (usize, usize) {
    let bytes = input.as_bytes();
    let cap = byte_offset.min(bytes.len());
    let mut line = 1usize;
    let mut col_start = 0usize;
    for (i, b) in bytes[..cap].iter().enumerate() {
        if *b == b'\n' {
            line += 1;
            col_start = i + 1;
        }
    }
    (line, cap - col_start + 1)
}

/// Format a [`ParseError`] into a human-readable diagnostic with
/// path, line:column, message, and a context snippet (N lines
/// before the error, the error line with a caret, M lines after).
#[must_use]
pub fn format_error(input: &str, err: &ParseError, before: usize, after: usize) -> String {
    let (line, col) = line_col_of(input, err.byte_offset);
    let path_disp = if err.path.is_empty() {
        "(root)"
    } else {
        err.path.as_str()
    };
    let snippet = context_snippet(input, line, col, before, after);
    format!(
        "parse error at line {line} col {col} (byte {byte}):\n  path: {path_disp}\n  {msg}\n{snippet}",
        byte = err.byte_offset,
        msg = err.message,
    )
}

/// Build a `before`/error/`after` line snippet around `(line,
/// col)`.  The error line is followed by a caret `^` placed
/// under `col` (1-based).
fn context_snippet(input: &str, line: usize, col: usize, before: usize, after: usize) -> String {
    let lines: Vec<&str> = input.lines().collect();
    let lo = line.saturating_sub(before + 1);
    let hi = (line + after).min(lines.len());
    let width = hi.to_string().len();
    let mut out = String::new();
    use std::fmt::Write;
    for (idx, content) in lines.iter().enumerate().take(hi).skip(lo) {
        let n = idx + 1;
        let _ = writeln!(out, "    {n:>width$} \u{2502} {content}");
        if n == line {
            // caret line — width-wide gutter, vertical-bar, then
            // (col-1) spaces, then ^
            let spaces = " ".repeat(col.saturating_sub(1));
            let _ = writeln!(out, "    {pad:>width$} \u{2502} {spaces}^", pad = "");
        }
    }
    out
}

/// Length of the UTF-8 codepoint whose first byte is `b`.  Returns 1 for
/// ASCII or any malformed lead byte (continuation / 0xF8+); returns 2/3/4
/// for the 2-byte / 3-byte / 4-byte forms.  Used by `parse_string` to
/// slurp a whole codepoint at once instead of byte-by-byte (P264).
fn utf8_lead_len(b: u8) -> usize {
    match b {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        0xF0..=0xF7 => 4,
        _ => 1, // continuation byte or invalid; fall through to from_utf8 error
    }
}

/// Read exactly four hex digits of a `\uXXXX` escape starting at `pos`
/// (the index of the first hex digit, i.e. just past the `u`) and return
/// their value.  Errors if fewer than four bytes remain or any is not
fn skip_ws(bytes: &[u8], mut i: usize) -> usize {
    while i < bytes.len() {
        match bytes[i] {
            b' ' | b'\t' | b'\n' | b'\r' => i += 1,
            _ => break,
        }
    }
    i
}

// ============================================================================
// @PLN109 Phase 2 — lexer-driven parser (byte-identical replacement).
//
// Drives loft's own lexer (`LexConfig::json()`) for structure, strings (the
// JSON-escape decoder — the one genuinely shared piece) and identifiers.
// Numbers are read from the raw lexeme (loft's number lexer overflows above
// i64::MAX and drops the raw text), and string *errors* are detected by a
// syntactic scan (`json_string_validate`) that reproduces the old scanner's
// messages/offsets without re-decoding.  Byte offsets (Option 1) are recovered
// from the lexer's (line, char-col) `Position` via `ByteMapper`.  Phase 2 keeps
// integers as `Number(f64)` (byte-identical to the old scanner, H5 rounding
// included); the `Parsed::Int` flip is Phase 3.
// ============================================================================

use crate::lexer::{LexConfig, LexItem, Lexer, Position};

/// Maps the lexer's 1-based `(line, char-column)` back to the absolute byte
/// offset the old byte-scanner reported (Option 1 — keep the byte-offset model).
///
/// ⚠⚠ THIS IS A HOT PATH, AND THE OBVIOUS IMPLEMENTATION IS QUADRATIC.  `byte()` is
/// called once per token, and the naive form walks `col - 1` chars from the START of the
/// line every time.  On a multi-line file that is short; on a **single-line** file — which
/// a serialiser produces by default, including our own `data_to_json` — `col` IS the
/// offset, so the walk is O(n) per token and O(n²) over the parse.  Measured 2026-08-21:
/// the 1 MB single-line stdlib snapshot took **124 s** to parse, ~8 KB/s.
///
/// A huge single-line document is a normal input, not an edge case, so both routes below
/// are O(1)-amortised rather than only the tidy one:
///
///   * **ASCII line** — a char column IS a byte column, so the answer is arithmetic.
///     `line_ascii` is computed in the same pass that finds the line starts, so it costs
///     nothing extra.
///   * **Non-ASCII line** — a forward CURSOR.  Positions arrive in lexing order, so the
///     walk resumes from the previous answer instead of restarting; total work across the
///     parse is O(n), not O(n²).  A backwards or foreign request falls back to the walk
///     from the line start, which is still correct, just not amortised.
struct ByteMapper {
    /// Byte offset of the start of each 1-based line (`line_starts[0]` = 0).
    line_starts: Vec<usize>,
    /// Per line: does it contain only ASCII?  Then char column == byte column.
    line_ascii: Vec<bool>,
    /// Memo for the non-ASCII path: the last `(line_index, chars_consumed, byte_offset)`
    /// answered, so an in-order request continues from there.
    cursor: std::cell::Cell<(usize, usize, usize)>,
}

impl ByteMapper {
    fn new(input: &str) -> Self {
        let mut line_starts = vec![0usize];
        let mut line_ascii = Vec::new();
        let mut ascii = true;
        for (i, b) in input.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
                line_ascii.push(ascii);
                ascii = true;
            } else if !b.is_ascii() {
                ascii = false;
            }
        }
        line_ascii.push(ascii); // the final line, which has no terminating newline
        ByteMapper {
            line_starts,
            line_ascii,
            cursor: std::cell::Cell::new((usize::MAX, 0, 0)),
        }
    }

    /// Absolute byte offset of `(line, col)`.  `col` is a 1-based char column, so
    /// the byte offset walks `col - 1` chars from the line start summing UTF-8
    /// widths — exact for non-ASCII lines.
    fn byte(&self, input: &str, line: u32, col: u32) -> usize {
        let li = (line.max(1) as usize) - 1;
        let start = self.line_starts.get(li).copied().unwrap_or(input.len());
        if start >= input.len() {
            return input.len();
        }
        let want = col.saturating_sub(1) as usize; // chars into the line

        // ASCII line: a char column IS a byte column.  No walk at all.
        if self.line_ascii.get(li).copied().unwrap_or(false) {
            return (start + want).min(input.len());
        }

        // Non-ASCII: resume from the cursor when this request is at or after it on the
        // SAME line, which is what in-order lexing produces.  Otherwise restart.
        let (c_li, c_chars, c_off) = self.cursor.get();
        let (mut off, mut done) = if c_li == li && c_chars <= want {
            (c_off, c_chars)
        } else {
            (start, 0)
        };
        for ch in input[off..].chars() {
            if done >= want {
                break;
            }
            off += ch.len_utf8();
            done += 1;
        }
        self.cursor.set((li, done, off));
        off.min(input.len())
    }
}

/// Scan a bare identifier in value position (`Dialect::Lenient`) from `start`,
/// returning `(name, end)`.  Consumes `[A-Za-z0-9_]` and `.`-joined segments — a
/// *qualified* enum tag like `Color.Red` (a `.` is taken only when followed by
/// another identifier segment, so a trailing dot or `tag.0` is left for the
/// caller).  Mirrors the retired `parse_bare_identifier_value`.
fn scan_bare_ident(bytes: &[u8], start: usize) -> (String, usize) {
    let mut j = start + 1;
    loop {
        while j < bytes.len() && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_') {
            j += 1;
        }
        if j + 1 < bytes.len()
            && bytes[j] == b'.'
            && (bytes[j + 1].is_ascii_alphabetic() || bytes[j + 1] == b'_')
        {
            j += 1; // consume the '.', loop to take the next segment
        } else {
            break;
        }
    }
    // Safe: every accepted byte is ASCII.
    let name = std::str::from_utf8(&bytes[start..j])
        .expect("ASCII identifier slice")
        .to_string();
    (name, j)
}

/// Old-scanner string error detection + terminator scan, WITHOUT decoding (the
/// decode is the lexer's `CString`).  `start` is the opening-quote offset.
/// Returns the byte offset just past the closing `"` on success, or the first
/// error exactly as the old byte-scanner reported it.
fn json_string_validate(bytes: &[u8], start: usize) -> Result<usize, (String, usize)> {
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => return Ok(i + 1),
            b'\\' => {
                if i + 1 >= bytes.len() {
                    return Err(("unterminated escape".to_string(), i));
                }
                i += 1;
                match bytes[i] {
                    b'"' | b'\\' | b'/' | b'b' | b'f' | b'n' | b'r' | b't' => i += 1,
                    b'u' => {
                        if i + 4 >= bytes.len() {
                            return Err(("truncated \\uXXXX escape".to_string(), i + 1));
                        }
                        if !bytes[i + 1..i + 5].iter().all(u8::is_ascii_hexdigit) {
                            return Err(("invalid hex in \\uXXXX escape".to_string(), i + 1));
                        }
                        i += 5;
                    }
                    other => return Err((format!("invalid escape \\{}", other as char), i)),
                }
            }
            c if c < 0x20 => return Err((format!("raw control byte {c:#x} in string"), i)),
            _ => {
                let n = utf8_lead_len(bytes[i]);
                i = (i + n).min(bytes.len());
            }
        }
    }
    Err(("unterminated string".to_string(), start))
}

struct JParser<'a> {
    lx: Lexer,
    input: &'a str,
    bytes: &'a [u8],
    map: ByteMapper,
    dialect: Dialect,
    path: Vec<String>,
    /// Byte offset just past the value most recently parsed, in the RAW input.
    /// loft's number lexer over-consumes JSON-invalid forms (`007`, `0x1f`,
    /// `1_0`) as a single token, so the lexer's next-token position would hide
    /// the trailing bytes the old scanner rejected.  Structural / trailing-byte
    /// checks read `skip_ws(value_end)` instead, matching the old scanner exactly.
    value_end: usize,
}

impl<'a> JParser<'a> {
    fn new(input: &'a str, dialect: Dialect) -> Self {
        JParser {
            lx: Lexer::from_str_with(input, "json", LexConfig::json()),
            input,
            bytes: input.as_bytes(),
            map: ByteMapper::new(input),
            dialect,
            path: Vec::new(),
            value_end: 0,
        }
    }

    /// The next non-whitespace byte after the value just parsed — the raw
    /// position a delimiter (`,` / `]` / `}` / EOF) must appear at.
    fn after_value(&self) -> usize {
        skip_ws(self.bytes, self.value_end)
    }

    /// Byte offset of the lexer's current token (or input end at EOF).  Exact for
    /// the non-number tokens that never over-consume.
    fn peek_pos(&self) -> usize {
        let r = self.lx.peek();
        if r.has == LexItem::None {
            self.bytes.len()
        } else {
            self.byte_of(&r.position)
        }
    }

    fn byte_of(&self, p: &Position) -> usize {
        self.map.byte(self.input, p.line, p.pos)
    }

    /// Advance the lexer until its next token starts at or after `end` (or EOF).
    /// Used after a raw number lexeme is consumed, to re-sync the token stream.
    fn advance_past(&mut self, end: usize) {
        loop {
            let r = self.lx.peek();
            if r.has == LexItem::None || self.byte_of(&r.position) >= end {
                break;
            }
            self.lx.cont();
        }
    }

    /// Parse a JSON value.  `at` is the byte offset where the value is expected
    /// (skip-whitespace'd) — used only to attribute a lexer `None` to a failed
    /// string (the old scanner's `"unterminated string"` etc.) vs genuine EOF.
    fn parse_value(&mut self, at: usize) -> Result<Parsed, (String, usize)> {
        let r = self.lx.peek();
        match &r.has {
            LexItem::None => {
                // The lexer produced no token where a value is expected.  Attribute
                // it to the raw byte at `at`: loft's lexer fails to tokenise some
                // valid-position JSON (a string it can't terminate; a number like
                // `1.` with a trailing dot).  Scan the raw input so the error (or
                // value) matches the old byte-scanner; otherwise it is real EOF.
                if at < self.bytes.len() {
                    match self.bytes[at] {
                        b'"' => {
                            return Err(json_string_validate(self.bytes, at)
                                .expect_err("lexer string failure must be an error"));
                        }
                        b'-' | b'0'..=b'9' => {
                            let (value, end) = self.scan_number(at)?;
                            self.value_end = end;
                            self.advance_past(end);
                            return Ok(value);
                        }
                        _ => {}
                    }
                }
                Err(("unexpected end of input".to_string(), self.bytes.len()))
            }
            LexItem::Token(t) if t == "-" => self.parse_number(),
            LexItem::Integer(..) | LexItem::Long(_) | LexItem::Float(..) | LexItem::Single(_) => {
                self.parse_number()
            }
            LexItem::Token(t) if t == "[" => self.parse_array(),
            LexItem::Token(t) if t == "{" => self.parse_object(),
            LexItem::CString(_) => {
                let s = self.take_string()?;
                Ok(Parsed::Str(s))
            }
            LexItem::Identifier(name) => {
                let name = name.clone();
                let pos = r.position.clone();
                self.parse_identifier_value(&name, &pos)
            }
            _ => {
                // An unexpected structural token (`}`, `]`, `,`, `:`) in value
                // position — the old scanner's "unexpected byte 0xNN at offset N".
                let off = self.byte_of(&r.position);
                let b = self.bytes.get(off).copied().unwrap_or(0);
                Err((format!("unexpected byte {b:#x} at offset {off}"), off))
            }
        }
    }

    /// Consume the current `CString` token, validating it against the old
    /// scanner's error semantics first (the lexer decodes leniently).
    fn take_string(&mut self) -> Result<String, (String, usize)> {
        // A `CString` token's position is the first *content* char; the opening
        // quote is one byte before it.
        let quote = self.byte_of(&self.lx.peek().position).saturating_sub(1);
        self.value_end = json_string_validate(self.bytes, quote)?;
        let LexItem::CString(s) = &self.lx.peek().has else {
            unreachable!("take_string called off a CString");
        };
        let s = s.clone();
        self.lx.cont();
        Ok(s)
    }

    fn parse_number(&mut self) -> Result<Parsed, (String, usize)> {
        let start = self.byte_of(&self.lx.peek().position);
        let (value, end) = self.scan_number(start)?;
        self.value_end = end;
        self.advance_past(end);
        Ok(value)
    }

    /// Scan a JSON number lexeme from the raw input (mirrors the old
    /// `parse_number`): optional `-`, integer part, fraction, exponent.  @PLN109
    /// Phase 3: an integer-shaped lexeme (no `.`, no exponent) that fits `i64`
    /// becomes [`Parsed::Int`] (H5-exact); a fractional/exponent number, or one
    /// that overflows `i64`, becomes [`Parsed::Number`] (`f64`).
    fn scan_number(&self, start: usize) -> Result<(Parsed, usize), (String, usize)> {
        let bytes = self.bytes;
        let mut i = start;
        if i < bytes.len() && bytes[i] == b'-' {
            i += 1;
        }
        if i >= bytes.len() || !bytes[i].is_ascii_digit() {
            return Err(("expected digit in number".to_string(), i));
        }
        if bytes[i] == b'0' {
            i += 1;
        } else {
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
        let mut fractional = false;
        if i < bytes.len() && bytes[i] == b'.' {
            fractional = true;
            i += 1;
            if i >= bytes.len() || !bytes[i].is_ascii_digit() {
                return Err(("expected digit after `.`".to_string(), i));
            }
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
        if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
            fractional = true;
            i += 1;
            if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                i += 1;
            }
            if i >= bytes.len() || !bytes[i].is_ascii_digit() {
                return Err(("expected digit in exponent".to_string(), i));
            }
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
        }
        let slice = std::str::from_utf8(&bytes[start..i])
            .map_err(|_| ("non-ASCII in number".to_string(), start))?;
        // Integer-shaped and i64-fitting → exact Int (H5); else f64.
        if !fractional && let Ok(n) = slice.parse::<i64>() {
            return Ok((Parsed::Int(n), i));
        }
        let n: f64 = slice
            .parse()
            .map_err(|_| (format!("invalid number `{slice}`"), start))?;
        Ok((Parsed::Number(n), i))
    }

    /// An identifier in value position: `null`/`true`/`false` in either dialect,
    /// or (Lenient only) a bare identifier / `Tag{…}` constructor.  In Strict, a
    /// non-literal identifier is the old scanner's "unexpected byte" / bad-literal.
    fn parse_identifier_value(
        &mut self,
        name: &str,
        pos: &Position,
    ) -> Result<Parsed, (String, usize)> {
        let off = self.byte_of(pos);
        if self.dialect == Dialect::Lenient {
            // Raw-scan the FULL bare identifier — including `.`-joined segments for a
            // qualified enum tag (`Category.Daily`) — since the lexer tokenises the
            // dots separately.  Then re-sync the token stream past the whole thing.
            let _ = name;
            let (full, end) = scan_bare_ident(self.bytes, off);
            self.value_end = end;
            self.advance_past(end);
            let value = match full.as_str() {
                "null" => Parsed::Null,
                "true" => Parsed::Bool(true),
                "false" => Parsed::Bool(false),
                _ => Parsed::Ident(full),
            };
            // `Tag{…}` — a type-tagged constructor (kept distinct from Object).
            if let Parsed::Ident(tag) = &value
                && self.lx.peek().has == LexItem::Token("{".to_string())
            {
                let obj = self.parse_object()?; // sets value_end past `}`
                return Ok(Parsed::Constructor(tag.clone(), off, Box::new(obj)));
            }
            return Ok(value);
        }
        // Strict: only n/t/f open a literal; anything else is an unexpected byte.
        let b = self.bytes.get(off).copied().unwrap_or(0);
        match b {
            b'n' | b't' | b'f' => {
                let (word, val) = match b {
                    b'n' => ("null", Parsed::Null),
                    b't' => ("true", Parsed::Bool(true)),
                    _ => ("false", Parsed::Bool(false)),
                };
                if name == word {
                    self.lx.cont();
                    self.value_end = self.peek_pos();
                    Ok(val)
                } else {
                    Err((format!("expected `{word}` at offset {off}"), off))
                }
            }
            _ => Err((format!("unexpected byte {b:#x} at offset {off}"), off)),
        }
    }

    fn parse_array(&mut self) -> Result<Parsed, (String, usize)> {
        let start = self.byte_of(&self.lx.peek().position); // the `[`
        self.lx.cont();
        let mut items: Vec<Parsed> = Vec::new();
        if self.lx.peek().has == LexItem::Token("]".to_string()) {
            self.lx.cont();
            self.value_end = self.peek_pos();
            return Ok(Parsed::Array(items));
        }
        // First element starts just past the `[`; each subsequent one past its `,`.
        let mut at = skip_ws(self.bytes, start + 1);
        let mut idx = 0usize;
        loop {
            self.path.push(idx.to_string());
            let v = self.parse_value(at)?;
            self.path.pop();
            items.push(v);
            // The delimiter must appear at the RAW position after the value; the
            // lexer's peek could be past it if the value was an over-consumed
            // JSON-invalid number.  For a valid value the two coincide, so the
            // lexer stays synced to consume the delimiter.
            let d = self.after_value();
            match self.bytes.get(d).copied() {
                None => return Err(("unterminated array".to_string(), start)),
                Some(b',') => {
                    self.lx.cont();
                    at = skip_ws(self.bytes, d + 1);
                    idx += 1;
                }
                Some(b']') => {
                    self.lx.cont();
                    self.value_end = self.peek_pos();
                    return Ok(Parsed::Array(items));
                }
                Some(b) => {
                    return Err((format!("expected `,` or `]` in array, got {b:#x}"), d));
                }
            }
        }
    }

    fn parse_object(&mut self) -> Result<Parsed, (String, usize)> {
        let start = self.byte_of(&self.lx.peek().position); // the `{`
        self.lx.cont();
        let mut fields: Vec<(String, usize, Parsed)> = Vec::new();
        if self.lx.peek().has == LexItem::Token("}".to_string()) {
            self.lx.cont();
            self.value_end = self.peek_pos();
            return Ok(Parsed::Object(fields));
        }
        loop {
            let (name, key_at) = self.parse_object_key(start)?;
            let colon = self.lx.peek();
            if colon.has != LexItem::Token(":".to_string()) {
                let off = if colon.has == LexItem::None {
                    self.bytes.len()
                } else {
                    self.byte_of(&colon.position)
                };
                return Err(("expected `:` after object key".to_string(), off));
            }
            let colon = self.byte_of(&colon.position);
            self.lx.cont(); // consume `:`
            let at = skip_ws(self.bytes, colon + 1);
            self.path.push(name.clone());
            let v = self.parse_value(at)?;
            self.path.pop();
            fields.push((name, key_at, v));
            // Delimiter at the RAW position after the value (see `parse_array`).
            let d = self.after_value();
            match self.bytes.get(d).copied() {
                None => return Err(("unterminated object".to_string(), start)),
                Some(b',') => self.lx.cont(),
                Some(b'}') => {
                    self.lx.cont();
                    self.value_end = self.peek_pos();
                    return Ok(Parsed::Object(fields));
                }
                Some(b) => {
                    return Err((format!("expected `,` or `}}` in object, got {b:#x}"), d));
                }
            }
        }
    }

    /// A quoted string key (either dialect) or a bare identifier key (Lenient).
    /// Returns `(name, key_byte_offset)` where the offset is the opening quote /
    /// identifier start, matching the old scanner's `Parsed::Object` key offset.
    fn parse_object_key(&mut self, obj_start: usize) -> Result<(String, usize), (String, usize)> {
        let r = self.lx.peek();
        match &r.has {
            LexItem::None => Err(("expected object key".to_string(), self.bytes.len())),
            LexItem::CString(_) => {
                let quote = self.byte_of(&r.position).saturating_sub(1);
                let name = self.take_string()?;
                Ok((name, quote))
            }
            LexItem::Identifier(name) if self.dialect == Dialect::Lenient => {
                let name = name.clone();
                let off = self.byte_of(&r.position);
                self.lx.cont();
                Ok((name, off))
            }
            _ => {
                let _ = obj_start;
                let off = self.byte_of(&r.position);
                Err(("expected string key in object".to_string(), off))
            }
        }
    }
}

/// @PLN109 Phase 2 — parse `input` by driving loft's JSON-mode lexer.  Same
/// `Parsed` tree + errors as the old byte-scanner (byte-identical; integers stay
/// `Number(f64)` until Phase 3).
fn parse_lexer(input: &str, dialect: Dialect) -> Result<Parsed, ParseError> {
    // RFC 8259 allows ignoring a leading UTF-8 BOM.  The lexer would merge the BOM
    // char with the following token, so strip it before lexing; reported byte
    // offsets are then relative to the post-BOM body (a BOM-prefixed document is
    // rare and its offsets are not a pinned contract — the common bom == 0 path is
    // byte-exact).
    let body = if input.as_bytes().starts_with(&[0xEF, 0xBB, 0xBF]) {
        &input[3..]
    } else {
        input
    };
    let mut p = JParser::new(body, dialect);
    let at = skip_ws(p.bytes, 0);
    let value = match p.parse_value(at) {
        Ok(v) => v,
        Err((message, byte_offset)) => {
            return Err(ParseError {
                message,
                byte_offset,
                path: render_path(&p.path),
            });
        }
    };
    // Trailing check against the RAW position after the value: the lexer may have
    // over-consumed a JSON-invalid number's suffix, which the old scanner rejected.
    let end = p.after_value();
    if end != p.bytes.len() {
        return Err(ParseError {
            message: format!("unexpected trailing byte at offset {end}"),
            byte_offset: end,
            path: render_path(&p.path),
        });
    }
    Ok(value)
}

/// Serialise a [`Parsed`] tree back to compact JSON text — the inverse of
/// [`parse`].  CLI commands that emit `--json` (e.g. `loft search --json`) build
/// a `Parsed` value and render it here instead of hand-assembling JSON strings.
/// Object key order is preserved and strings are escaped per RFC 8259.  A
/// `Constructor` type tag (lenient input only) serialises as its bare body
/// object, since JSON has no tag syntax.
#[must_use]
pub fn to_json_string(value: &Parsed) -> String {
    let mut out = String::new();
    write_json(&mut out, value);
    out
}

fn write_json(out: &mut String, value: &Parsed) {
    use std::fmt::Write as _;
    match value {
        Parsed::Null => out.push_str("null"),
        Parsed::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Parsed::Int(n) => {
            let _ = write!(out, "{n}");
        }
        Parsed::Number(n) => {
            // Whole, in-range values render without a trailing `.0` so a round
            // trip through `parse` reproduces the same text.  2^53 is the
            // largest integer an f64 represents exactly.
            if n.is_finite() && n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
                let _ = write!(out, "{}", *n as i64);
            } else {
                let _ = write!(out, "{n}");
            }
        }
        Parsed::Str(s) | Parsed::Ident(s) => write_json_string(out, s),
        Parsed::Array(items) => {
            out.push('[');
            for (idx, item) in items.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                write_json(out, item);
            }
            out.push(']');
        }
        Parsed::Object(entries) => {
            out.push('{');
            for (idx, (key, _offset, val)) in entries.iter().enumerate() {
                if idx > 0 {
                    out.push(',');
                }
                write_json_string(out, key);
                out.push(':');
                write_json(out, val);
            }
            out.push('}');
        }
        Parsed::Constructor(_tag, _offset, body) => write_json(out, body),
    }
}

fn write_json_string(out: &mut String, s: &str) {
    use std::fmt::Write as _;
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod bytemapper_tests {
    use super::{Dialect, parse, parse_with};

    // ⚠ `byte()` maps a CHAR column back to a BYTE offset, and it has two routes: an
    // arithmetic fast path for an all-ASCII line, and a resumable walk otherwise.  The
    // golden corpus covers non-ASCII as VALUES but never reports an error offset AFTER a
    // multi-byte char on the same line — which is precisely where the fast path would be
    // wrong if it were applied to a non-ASCII line.
    //
    // Asserted as a PROPERTY, not a pinned number, so it does not encode this parser's
    // choice of which token to blame: wherever it points, the offset must be a real char
    // boundary and must land on the text it means.  Under an ASCII-only assumption the
    // offset comes out 2 bytes short here (the width of `é`) and lands mid-token.
    #[test]
    fn an_error_offset_after_a_multibyte_char_is_a_byte_offset() {
        let input = "{\"k\":\"é\",\"bad\" 1}";
        let err = parse(input).expect_err("missing colon must fail");
        assert!(
            input.is_char_boundary(err.byte_offset),
            "offset {} is not a char boundary in {input:?}",
            err.byte_offset
        );
        // ⚠ NOT `starts_with('1') || starts_with(' ')`.  That first spelling passed under
        // a deliberately broken build (the control), because a 2-byte undershoot still
        // landed on the space — a guard that cannot fail for the defect it names.  The
        // offset must be exactly the reported token.
        assert!(
            input[err.byte_offset..].starts_with('1'),
            "offset {} points at {:?}, not at the `1` the error is about",
            err.byte_offset,
            &input[err.byte_offset..]
        );
    }

    // The same, with the multi-byte char in a KEY and several of them, so the walk has to
    // accumulate more than one width before the reported position.
    #[test]
    fn several_multibyte_chars_before_the_error_offset() {
        let input = "{\"é😀𝄞\":1,\"b\" 2}";
        let err = parse(input).expect_err("missing colon must fail");
        assert!(input.is_char_boundary(err.byte_offset));
        assert!(
            input[err.byte_offset..].starts_with('2'),
            "offset {} points at {:?}, not at the `2` the error is about",
            err.byte_offset,
            &input[err.byte_offset..]
        );
    }

    // ⚠⚠ THE PERFORMANCE GUARD, and it guards a CLASS rather than a number.
    //
    // `byte()` used to walk from the line start on every call.  On a single-line document
    // — what a serialiser emits by default, ours included — the char column IS the offset,
    // so that is O(n) per token and O(n²) over the parse: the 1 MB single-line stdlib
    // snapshot took **124 s**, about 8 KB/s.  It is now ~120 ms.
    //
    // The bound is deliberately loose (10 s for ~1 MB).  It is not a benchmark and must not
    // flake on a loaded machine; it only has to separate "linear" from "quadratic", and the
    // gap between the two is three orders of magnitude.
    #[test]
    fn a_huge_single_line_document_parses_in_linear_time() {
        let mut src = String::from("[");
        for i in 0..40_000 {
            if i > 0 {
                src.push(',');
            }
            src.push_str("{\"k\":123,\"s\":\"abcdefgh\"}");
        }
        src.push(']');
        assert!(src.len() > 900_000, "fixture too small: {}", src.len());
        assert_eq!(src.matches('\n').count(), 0, "fixture must be ONE line");

        let t = std::time::Instant::now();
        let parsed = parse_with(&src, Dialect::Strict).expect("huge single-line parse");
        let took = t.elapsed();
        drop(parsed);
        assert!(
            took.as_secs() < 10,
            "parsing {} bytes on one line took {took:?} — the quadratic byte-mapper is back",
            src.len()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The serializer is correct iff the existing parser reads its output back
    /// to the same text.  Compare *strings*, not trees: `Parsed::Object` carries
    /// byte offsets that differ between a built tree and a parsed one.
    #[test]
    fn to_json_string_round_trips_through_parse() {
        let sample = Parsed::Array(vec![Parsed::Object(vec![
            ("name".to_string(), 0, Parsed::Str("regex".to_string())),
            ("version".to_string(), 0, Parsed::Str("0.2.0".to_string())),
            (
                "description".to_string(),
                0,
                Parsed::Str("Regular \"expr\"\nwith\ttabs".to_string()),
            ),
            (
                "categories".to_string(),
                0,
                Parsed::Array(vec![Parsed::Str("text".to_string())]),
            ),
            ("auto_use".to_string(), 0, Parsed::Bool(true)),
            ("count".to_string(), 0, Parsed::Number(42.0)),
            ("homepage".to_string(), 0, Parsed::Null),
        ])]);
        let s = to_json_string(&sample);
        assert_eq!(to_json_string(&parse(&s).unwrap()), s);
    }

    #[test]
    fn primitives() {
        assert!(matches!(parse("null").unwrap(), Parsed::Null));
        assert!(matches!(parse("true").unwrap(), Parsed::Bool(true)));
        assert!(matches!(parse("false").unwrap(), Parsed::Bool(false)));
    }

    #[test]
    fn numbers() {
        // @PLN109 — integer-shaped numbers are `Parsed::Int`; fractional/exponent
        // numbers stay `Parsed::Number`.
        assert!(matches!(parse("0").unwrap(), Parsed::Int(0)));
        assert!(matches!(parse("42").unwrap(), Parsed::Int(42)));
        assert!(
            matches!(parse("-17.5").unwrap(), Parsed::Number(v) if (v - (-17.5)).abs() < f64::EPSILON)
        );
        assert!(
            matches!(parse("1.5e3").unwrap(), Parsed::Number(v) if (v - 1500.0).abs() < f64::EPSILON)
        );
    }

    #[test]
    fn strings() {
        let got = parse(r#""hello""#).unwrap();
        assert!(matches!(got, Parsed::Str(ref s) if s == "hello"));
        let got = parse(r#""\"quote\"""#).unwrap();
        assert!(matches!(got, Parsed::Str(ref s) if s == "\"quote\""));
        let got = parse(r#""line\nfeed""#).unwrap();
        assert!(matches!(got, Parsed::Str(ref s) if s == "line\nfeed"));
    }

    /// P264 — multi-byte UTF-8 sequences in JSON string payloads must
    /// round-trip byte-for-byte.  The pre-fix loop pushed each byte as
    /// its own char, widening 0xE2/0x86/0x92 (the bytes for `→`) to
    /// three separate codepoints (U+00E2, U+0086, U+0092), each
    /// re-encoded as 2-byte UTF-8 → 3 input bytes became 6 output
    /// bytes, displaying as `âââ`.
    #[test]
    fn p264_multibyte_utf8_passthrough() {
        // 3-byte codepoint: U+2192 RIGHTWARDS ARROW
        let got = parse(r#""before → after""#).unwrap();
        let Parsed::Str(s) = got else {
            panic!("expected Str")
        };
        assert_eq!(s, "before → after");
        assert_eq!(s.len(), 16); // 7 + 3 + 6 = 16 UTF-8 bytes
        // 4-byte codepoint: U+1F600 GRINNING FACE
        let got = parse(r#""smile 😀""#).unwrap();
        let Parsed::Str(s) = got else {
            panic!("expected Str")
        };
        assert_eq!(s, "smile 😀");
        assert_eq!(s.len(), 10); // 6 + 4 = 10 UTF-8 bytes
        // 2-byte codepoint: U+00E9 LATIN SMALL LETTER E WITH ACUTE
        let got = parse(r#""café""#).unwrap();
        let Parsed::Str(s) = got else {
            panic!("expected Str")
        };
        assert_eq!(s, "café");
        assert_eq!(s.len(), 5); // 3 + 2 = 5 UTF-8 bytes
        // Mix of all three widths in one string
        let got = parse(r#""→ é 😀""#).unwrap();
        let Parsed::Str(s) = got else {
            panic!("expected Str")
        };
        assert_eq!(s, "→ é 😀");
    }

    /// P285 — a non-BMP character escaped as a UTF-16 surrogate pair
    /// (`\uHI\uLO`, what Python's default `json.dumps` emits) must combine
    /// into one scalar.  The pre-fix code decoded each half independently;
    /// a surrogate code unit is not a scalar, so each became U+FFFD and
    /// `😀` came back as `��`.
    #[test]
    fn p285_surrogate_pairs() {
        // U+1F600 GRINNING FACE as a surrogate pair.
        let Parsed::Str(s) = parse(r#""😀""#).unwrap() else {
            panic!("expected Str")
        };
        assert_eq!(s, "😀");
        assert_eq!(s.len(), 4);
        // Mixed BMP escape + surrogate pair + ASCII, pair not at the end.
        let Parsed::Str(s) = parse(r#""a→b😀c""#).unwrap() else {
            panic!("expected Str")
        };
        assert_eq!(s, "a→b😀c");
        // Lone high surrogate → one U+FFFD.
        let Parsed::Str(s) = parse(r#""\uD83D""#).unwrap() else {
            panic!("expected Str")
        };
        assert_eq!(s, "\u{fffd}");
        // Lone low surrogate → one U+FFFD.
        let Parsed::Str(s) = parse(r#""\uDE00""#).unwrap() else {
            panic!("expected Str")
        };
        assert_eq!(s, "\u{fffd}");
        // High surrogate followed by a non-low `\u` escape: the high half is
        // U+FFFD, the second escape is still decoded normally ('A').
        let Parsed::Str(s) = parse(r#""\uD83DA""#).unwrap() else {
            panic!("expected Str")
        };
        assert_eq!(s, "\u{fffd}A");
    }

    /// P285 — a single leading UTF-8 BOM (`EF BB BF`) is ignored at parse
    /// entry (RFC 8259 permits this).  A BOM-prefixed document used to fail
    /// to parse.
    #[test]
    fn p285_leading_bom_skipped() {
        let bom = "\u{feff}";
        assert!(matches!(
            parse(&format!("{bom}null")).unwrap(),
            Parsed::Null
        ));
        let Parsed::Object(fields) = parse(&format!("{bom}{{\"a\": 1}}")).unwrap() else {
            panic!("expected object");
        };
        assert_eq!(fields[0].0, "a");
        // A BOM only counts at offset 0 — a `﻿` escape inside a string
        // is a normal ZERO WIDTH NO-BREAK SPACE scalar, untouched.
        let Parsed::Str(s) = parse(r#""﻿""#).unwrap() else {
            panic!("expected Str")
        };
        assert_eq!(s, "\u{feff}");
    }

    #[test]
    fn whitespace_tolerated() {
        assert!(matches!(parse("  null  ").unwrap(), Parsed::Null));
    }

    #[test]
    fn arrays() {
        assert!(matches!(parse("[]").unwrap(), Parsed::Array(ref v) if v.is_empty()));
        let got = parse("[1, 2, 3]").unwrap();
        let Parsed::Array(v) = got else {
            panic!("expected array");
        };
        assert_eq!(v.len(), 3);
        assert!(matches!(v[0], Parsed::Int(1)));
        let nested = parse("[[1], [2, 3]]").unwrap();
        let Parsed::Array(outer) = nested else {
            panic!("expected array");
        };
        assert_eq!(outer.len(), 2);
    }

    #[test]
    fn objects() {
        assert!(matches!(parse("{}").unwrap(), Parsed::Object(ref v) if v.is_empty()));
        let got = parse(r#"{"a": 1, "b": "hi"}"#).unwrap();
        let Parsed::Object(fields) = got else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 2);
        assert_eq!(fields[0].0, "a");
        assert!(matches!(fields[0].2, Parsed::Int(1)));
        assert_eq!(fields[1].0, "b");
        assert!(matches!(fields[1].2, Parsed::Str(ref s) if s == "hi"));
    }

    #[test]
    fn nested_mixed() {
        let got = parse(r#"{"items": [1, {"x": true}], "n": null}"#).unwrap();
        let Parsed::Object(fields) = got else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 2);
        let Parsed::Array(items) = &fields[0].2 else {
            panic!("expected array");
        };
        assert_eq!(items.len(), 2);
        let Parsed::Object(inner) = &items[1] else {
            panic!("expected inner object");
        };
        assert_eq!(inner[0].0, "x");
        assert!(matches!(inner[0].2, Parsed::Bool(true)));
    }

    // ── Q1: structured errors with path / line:col / snippet ────────

    #[test]
    fn err_root_failure_has_empty_path() {
        let err = parse("xyz").unwrap_err();
        assert_eq!(err.path, "");
        assert_eq!(err.byte_offset, 0);
    }

    #[test]
    fn err_inside_array_carries_index_path() {
        let err = parse("[1, 2, 1.]").unwrap_err();
        assert_eq!(err.path, "/2");
    }

    #[test]
    fn err_inside_object_carries_field_path() {
        let err = parse(r#"{"a": 1, "b": 1.}"#).unwrap_err();
        assert_eq!(err.path, "/b");
    }

    #[test]
    fn err_nested_path_is_full_pointer() {
        let err = parse(r#"{"users": [{"name": "x"}, {"name": 1.}]}"#).unwrap_err();
        assert_eq!(err.path, "/users/1/name");
    }

    #[test]
    fn err_path_escapes_slash_and_tilde() {
        // Field "a/b~c" → "/a~1b~0c" per RFC 6901.
        let err = parse(r#"{"a/b~c": 1.}"#).unwrap_err();
        assert_eq!(err.path, "/a~1b~0c");
    }

    #[test]
    fn line_col_basic() {
        assert_eq!(line_col_of("abc", 0), (1, 1));
        assert_eq!(line_col_of("abc", 2), (1, 3));
        assert_eq!(line_col_of("a\nbc", 2), (2, 1));
        assert_eq!(line_col_of("a\nbc", 3), (2, 2));
        assert_eq!(line_col_of("a\nb\nc", 4), (3, 1));
    }

    #[test]
    fn format_error_includes_path_line_col_and_caret() {
        let raw = "{\n  \"x\": 1.\n}";
        let err = parse(raw).unwrap_err();
        let formatted = format_error(raw, &err, 1, 1);
        // Diagnostic mentions path, line, col, message, and a caret.
        assert!(formatted.contains("/x"), "missing path: {formatted}");
        assert!(
            formatted.contains("line 2"),
            "missing line number: {formatted}"
        );
        assert!(formatted.contains('^'), "missing caret: {formatted}");
        assert!(
            formatted.contains("expected digit after `.`"),
            "missing message: {formatted}"
        );
    }

    #[test]
    fn format_error_root_path_renders_as_root_label() {
        let formatted = format_error("xyz", &parse("xyz").unwrap_err(), 0, 0);
        assert!(
            formatted.contains("(root)"),
            "root path label missing: {formatted}"
        );
    }

    #[test]
    fn malformed_collections() {
        assert!(parse("[").is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse("{").is_err());
        assert!(parse(r#"{"a"}"#).is_err());
        assert!(parse(r#"{"a": 1,}"#).is_err());
        assert!(parse(r"{a: 1}").is_err());
    }

    #[test]
    fn malformed_returns_err() {
        assert!(parse("").is_err());
        assert!(parse("nu").is_err());
        assert!(parse("1.").is_err());
        assert!(parse(r#""no-close"#).is_err());
        assert!(parse("null trailing").is_err());
    }

    // ── Dialect::Lenient accepts loft bare-identifier keys ──────

    #[test]
    fn parse_with_strict_rejects_bare_key() {
        assert!(parse_with(r"{a: 1}", Dialect::Strict).is_err());
        assert!(parse_with(r"{x_1: null}", Dialect::Strict).is_err());
    }

    #[test]
    fn parse_with_lenient_accepts_bare_key() {
        let Parsed::Object(fields) = parse_with(r"{val: 7}", Dialect::Lenient).unwrap() else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "val");
        assert!(matches!(fields[0].2, Parsed::Int(7)));
    }

    #[test]
    fn parse_with_lenient_allows_mixed_quoted_and_bare() {
        let Parsed::Object(fields) =
            parse_with(r#"{a: 1, "b": 2, c_2: 3}"#, Dialect::Lenient).unwrap()
        else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 3);
        assert_eq!(fields[0].0, "a");
        assert_eq!(fields[1].0, "b");
        assert_eq!(fields[2].0, "c_2");
    }

    #[test]
    fn parse_with_lenient_rejects_non_identifier_keys() {
        // Numeric object keys are not accepted even under Lenient —
        // only `[A-Za-z_][A-Za-z0-9_]*` identifiers or quoted
        // strings qualify as keys.  Bare-identifier *values* are
        // accepted separately (see `parse_with_lenient_accepts_bare_ident_value`).
        assert!(parse_with(r"{1: 2}", Dialect::Lenient).is_err());
        assert!(parse_with(r"{-foo: 1}", Dialect::Lenient).is_err());
    }

    #[test]
    fn parse_default_is_strict() {
        // Default Dialect is Strict — behaviour identical to bare `parse`.
        assert_eq!(Dialect::default(), Dialect::Strict);
    }

    // ── Dialect::Lenient also accepts bare identifier values ──

    #[test]
    fn parse_with_lenient_accepts_bare_ident_value() {
        // `Daily` here represents a loft enum tag in value position.
        let Parsed::Object(fields) = parse_with(r"{category: Daily}", Dialect::Lenient).unwrap()
        else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 1);
        assert_eq!(fields[0].0, "category");
        assert!(
            matches!(&fields[0].2, Parsed::Ident(s) if s == "Daily"),
            "expected Ident(\"Daily\"), got {:?}",
            fields[0].2
        );
    }

    #[test]
    fn parse_with_lenient_recognises_true_false_null_as_bare() {
        let Parsed::Object(fields) =
            parse_with(r"{a: true, b: false, c: null}", Dialect::Lenient).unwrap()
        else {
            panic!("expected object");
        };
        assert!(matches!(fields[0].2, Parsed::Bool(true)));
        assert!(matches!(fields[1].2, Parsed::Bool(false)));
        assert!(matches!(fields[2].2, Parsed::Null));
    }

    #[test]
    fn parse_with_strict_still_rejects_bare_ident_value() {
        assert!(parse_with(r"{category: Daily}", Dialect::Strict).is_err());
        assert!(parse_with(r"{x: hello}", Dialect::Strict).is_err());
    }

    #[test]
    fn parse_with_lenient_top_level_bare_ident() {
        // Not only in object values — a bare identifier is a valid
        // top-level loft literal (e.g. a single enum tag stored as
        // the whole record).
        let parsed = parse_with("Hourly", Dialect::Lenient).unwrap();
        assert!(
            matches!(&parsed, Parsed::Ident(s) if s == "Hourly"),
            "expected Ident(\"Hourly\"), got {parsed:?}",
        );
    }

    #[test]
    fn parse_with_lenient_bare_ident_in_array() {
        let Parsed::Array(items) =
            parse_with(r"[Daily, Weekly, Hourly]", Dialect::Lenient).unwrap()
        else {
            panic!("expected array");
        };
        assert_eq!(items.len(), 3);
        assert!(matches!(&items[0], Parsed::Ident(s) if s == "Daily"));
        assert!(matches!(&items[1], Parsed::Ident(s) if s == "Weekly"));
        assert!(matches!(&items[2], Parsed::Ident(s) if s == "Hourly"));
    }

    #[test]
    fn parse_with_lenient_mixed_example_from_data_structures_test() {
        // This input comes from tests/data_structures.rs::record —
        // the legacy parser's canonical round-trip shape.
        let input = r#"{ name: "Hello World!", category: Hourly, size: 12345, percentage: 0.15 }"#;
        let Parsed::Object(fields) = parse_with(input, Dialect::Lenient).unwrap() else {
            panic!("expected object");
        };
        assert_eq!(fields.len(), 4);
        assert!(matches!(&fields[0].2, Parsed::Str(s) if s == "Hello World!"));
        assert!(matches!(&fields[1].2, Parsed::Ident(s) if s == "Hourly"));
        assert!(matches!(fields[2].2, Parsed::Int(12345)));
        assert!(matches!(fields[3].2, Parsed::Number(n) if (n - 0.15).abs() < 1e-9));
    }
}
