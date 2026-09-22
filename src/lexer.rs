// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I57 — Lexer (source to LexItems)

//! Change a text into symbols to use in the parser.
//! It is possible to link to the current position in the lexer (link) and return to it (revert)
//! when the parser has to try a certain path and might dismiss this later.

use crate::diagnostics::{Diagnostics, Fix, FixKind, Level, diagnostic_format};
use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt::{Debug, Display, Formatter};
use std::fs::File;
use std::io::{BufRead, BufReader, Result as IoResult};
use std::iter::Peekable;
use std::rc::Rc;
use std::vec::IntoIter;

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    /// Expect code with spaces, line ends and remarks removed.
    Code,
    // @F36 — string formatting / format specifiers (+ for-expressions)
    /// Expect formatting expressions, when encountering a closing bracket continue with a string.
    Formatting,
}

/// An item parsed by the lexer.
#[derive(Debug, Clone, PartialEq)]
pub enum LexItem {
    /// This routine cannot directly parse negative number, because - is reported as a token.
    /// Second token is if the number started with a 0. Only needed for string formatting.
    Integer(u32, bool),
    Long(u64),
    /// A float literal and whether its spelling began with a `0` — the leading zero
    /// that a `{f:08.2}` format spec means as "pad with zeros", exactly as [`Self::Integer`]
    /// carries it for `{n:08}`.  The parsed `f64` cannot answer it: `08.2` and `8.2` are the
    /// same number.
    Float(f64, bool),
    Single(f32),
    /// Can be both a keyword and one or more position tokens.
    Token(String),
    /// A still unknown identifier.
    Identifier(String),
    /// A constant string: was presented as "content" with possibly escaped tokens inside.
    CString(String),
    Character(u32),
    /// The end of the content is reached.
    None,
}

#[derive(Clone, PartialEq)]
pub struct Position {
    /// The file name where this construct is found.
    pub file: String,
    /// The line where this result was found.
    pub line: u32,
    /// The position on the line where this result was found.
    pub pos: u32,
}

impl Position {
    fn format(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        fmt.write_str(&format!("{}:{}:{}", self.file, self.line, self.pos))
    }
}

/// `LOFT_TRACE_LEX=1` — narrate the lexer's POSITION bookkeeping to stderr:
/// each recorded identifier position, each `to()` seek, each `revert`, and each
/// memory replay.
///
/// The reporting cursor is a shared, long-lived thing: any warning pass may seek
/// it backwards to point at an earlier site, and `to()` moves only that cursor —
/// the tokenizer keeps counting lines from wherever it was left.  A seek that is
/// not put back therefore corrupts the line of every LATER diagnostic, and the
/// symptom shows up far from the cause, in an unrelated message (#625).
///
/// Run it over both passes and diff them: the pass that records a token at the
/// wrong line names the seek that preceded it.
pub(crate) fn lex_trace(args: std::fmt::Arguments<'_>) {
    if std::env::var_os("LOFT_TRACE_LEX").is_some() {
        eprintln!("[lex] {args}");
    }
}

impl Debug for Position {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        self.format(fmt)
    }
}

impl Display for Position {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        self.format(fmt)
    }
}

/// The lexer can be iterated to gain a string of results.
#[derive(Debug, Clone, PartialEq)]
pub struct LexResult {
    pub has: LexItem,
    pub position: Position,
}

impl LexResult {
    fn new(it: LexItem, position: Position) -> LexResult {
        LexResult { has: it, position }
    }
}

/// A lexer that can remember a state via a link and then optionally return to that state.
///
/// It defaults to reading all found data into Text elements but has a list of TOKENS and
/// KEYWORDS that are parsed when a line starts with a token.
// The bool fields are independent lexer modes (interpolation, JSON strings, format-expr /
// backtick state), not a state enum — each toggles a distinct behaviour.
#[allow(clippy::struct_excessive_bools)]
pub struct Lexer {
    lines: Box<dyn Iterator<Item = IoResult<String>>>,
    /// In-memory sources by name (`parse_string` registers them): `switch`
    /// serves these before trying the filesystem, so a `use` that halts and
    /// later RESUMES the current file by name works for virtual sources
    /// (REPL snippets, probes, live-reload's "<live-reload>") — names like
    /// `<probe>` are not openable paths on any platform.
    virtual_files: std::collections::HashMap<String, String>,
    iter: Peekable<IntoIter<char>>,
    peek: LexResult,
    /// Keep the scanned items in memory when a Link is created to return when reverted to this link.
    memory: Vec<LexResult>,
    /// The string-interpolation state each token in `memory` was scanned in, index for
    /// index — see [`ScanState`].  A replay restores it with the token.
    memory_state: Vec<ScanState>,
    /// Keep track of the number of currently in use links
    links: Rc<RefCell<u32>>,
    /// Keep track of where we are in the current memory structure
    link: usize,
    position: Position,
    /// End of the token BEFORE the current one — where the source the parser has
    /// already consumed stops.  A diagnostic about a construct the parser has finished
    /// belongs here rather than at the scan cursor, which has run on to the next token;
    /// see [`report_pos`](Self::report_pos).
    prev_end: Position,
    /// Where [`to`](Self::to) moved the reporting position away FROM, until the next
    /// token is scanned.
    ///
    /// `to()` moves the reporting line/pos but not the read cursor, so a seek that is
    /// never undone offsets every position derived from the lexer for the rest of the
    /// file — the caret, the `file:line` of a runtime span, the line the compiler
    /// injects into `assert`.  Restoring it at the next freshly scanned token bounds a
    /// missing restore to the one diagnostic it was made for.
    seek_return: Option<(u32, u32)>,
    tokens: HashSet<String>,
    keywords: HashSet<String>,
    /// The comment marker (from it to end-of-line is skipped); loft `//`.  See
    /// [`LexConfig`].
    comment: String,
    /// Whether `"…"` literals interpret `{…}` as interpolation (loft) or as literal
    /// braces (configs).  See [`LexConfig::interpolate_strings`].
    interpolate_strings: bool,
    /// Whether `"…"` literals accept JSON string escapes (`\/`, `\uXXXX` + surrogate
    /// pairs).  See [`LexConfig::json_strings`].
    json_strings: bool,
    /// @PLN109 — set by [`json_unicode_escape`](Self::json_unicode_escape) when an
    /// unpaired `\uXXXX` high surrogate had to over-read past its 4th hex digit (to
    /// look for a low surrogate that was not there), leaving the char iterator one
    /// position PAST the escape rather than AT its last char.  [`string`](Self::string)
    /// honours it by skipping its shared post-escape advance, so the following byte
    /// (e.g. the closing `"`) is not swallowed.
    json_over_read: bool,
    /// Should we expect code with whitespaces here?
    mode: Mode,
    /// True while the lexer is inside a `{...}` format expression of a string literal.
    /// Allows `"` (and `\"`) to open a nested string literal instead of closing the outer one.
    in_format_expr: bool,
    /// Which string each OPEN format expression belongs to, innermost last —
    /// what `}` must resume. One entry per open hole rather than a flag,
    /// because a hole can be opened from inside a string literal that is itself
    /// inside a hole (`"{"{y}"}"`). A flag can answer "resume which?" at depth
    /// one and can only be wrong deeper, silently: loft#767 emitted the inner
    /// string's own `{y}` verbatim and the program printed a plausible wrong
    /// value.
    open_strings: Vec<StrKind>,
    /// How many leading spaces each OPEN backtick literal drops from its lines,
    /// innermost last — `None` until its first content line settles it.
    ///
    /// A stack rather than a field because a backtick literal can be written inside
    /// another one's interpolation hole, and one entry cannot answer for two literals.
    /// It has to be lexer state at all because a literal with a hole is scanned in
    /// SEGMENTS: [`backtick_string`](Lexer::backtick_string) hands the text before the
    /// hole to the parser and returns, and
    /// [`backtick_string_resume`](Lexer::backtick_string_resume) picks the literal up
    /// again with nothing of its own to remember.
    backtick_strip: Vec<Option<usize>>,
    diagnostics: Diagnostics,
}

/// The lexer's string-interpolation state as it stood after a token was scanned: what a
/// `"…{` opened, and what the `}` that closes it must resume.
///
/// It is lexer state and not token content, so the replay buffer has to carry it beside the
/// tokens.  A literal the parser reads a SECOND time (after a `revert`) is handed its tokens
/// back from the buffer while these fields still describe wherever the live scan had got to,
/// and the parser reads them to decide whether a string has a hole: every interpolation in
/// a re-read literal was refused (`Expect token ]`).
#[derive(Clone)]
struct ScanState {
    mode: Mode,
    in_format_expr: bool,
    open_strings: Vec<StrKind>,
    backtick_strip: Vec<Option<usize>>,
}

/// The string a `}` returns to when it closes a format expression.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum StrKind {
    /// `"…"` at statement level.
    Plain,
    /// `` `…` `` — multi-line, resumed by `backtick_string_resume`. The flag is
    /// true when the literal itself sits inside a format expression, which is
    /// what decides whether closing it returns to code or to that expression.
    Backtick(bool),
    /// A string literal written INSIDE a format expression. The flag is true
    /// when it was opened with `\"`, which is then also what closes it.
    Nested(bool),
}

impl Debug for Lexer {
    fn fmt(&self, fmt: &mut Formatter<'_>) -> Result<(), std::fmt::Error> {
        fmt.write_str(&format!("{:?}", self.position))
    }
}

static LINE: String = String::new();

static TOKENS: &[&str] = &[
    ":", "::", ".", "..", ",", "{", "}", "(", ")", "[", "]", ";", "!", "!=", "+", "+=", "-", "-=",
    "*", "**", "*=", "/", "/=", "%", "%=", "=", "==", "<", "<=", ">", ">=", "&", "&&", "|", "||",
    "->", "=>", "^", "<<", ">>", "$", "//", "#", "?", "??", "@", "~",
];

static KEYWORDS: &[&str] = &[
    "as",
    "if",
    "in",
    "else",
    "for",
    "while",
    "continue",
    "break",
    "return",
    "yield",
    "true",
    "false",
    "null",
    "struct",
    "fn",
    "type",
    "enum",
    "interface",
    "pub",
    "and",
    "or",
    "use",
    "match",
    "sizeof",
    "debug_assert",
    "assert",
    "panic",
    "interface",
    "is",
];

/// True when `word` is one of loft's reserved keywords (the `KEYWORDS` table above).
/// Used by the @PLN13 script detector to tell a real loose statement that begins with
/// a keyword (`if x { … }`, `for i in …`, `return x`) from a MALFORMED definition whose
/// keyword was mistyped (`funcion main()`), which must not be treated as a script.
/// loft's reserved keywords — the completion provider offers them alongside
/// in-scope names.
#[must_use]
pub fn keywords() -> &'static [&'static str] {
    KEYWORDS
}

#[must_use]
pub fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}

/// The lexicon a [`Lexer`] tokenises with: its multi-/single-character
/// tokens/operators, its keywords, the comment marker that runs to end-of-line,
/// and whether string literals interpret `{…}` as interpolation.  `Default` is
/// loft's own lexicon; a consumer can supply another so the SAME lexer tokenises a
/// different surface syntax (e.g. [`LexConfig::config`] for `loft.toml`, which uses
/// `#` comments and treats `{ }` in strings as literal) — so there is no second
/// lexer in the codebase.
#[derive(Clone)]
pub struct LexConfig {
    /// Tokens/operators recognised — single- AND multi-character (both are matched
    /// against this set).  MUST contain `comment`.
    pub tokens: HashSet<String>,
    /// Bare identifiers promoted to keywords.
    pub keywords: HashSet<String>,
    /// The comment marker: from it to end-of-line is skipped (loft `//`, TOML `#`).
    pub comment: String,
    /// When true (loft), a lone `{` in a `"…"` literal opens a `{expr}` format slot;
    /// when false (configs), `{` / `}` are literal string content.
    pub interpolate_strings: bool,
    /// When true (@PLN109 JSON mode), `"…"` literals additionally accept JSON's
    /// string escapes — `\/` and `\uXXXX` (four hex, no braces) with surrogate-pair
    /// combining — on top of loft's own escape set (loft is a lenient superset, not
    /// a JSON validator).  Off for all normal loft / config lexing, so loft source
    /// string semantics are unchanged.
    pub json_strings: bool,
}

impl Default for LexConfig {
    /// loft's own lexicon: the `TOKENS` + `KEYWORDS` tables, `//` comments, and
    /// `{…}` string interpolation.
    fn default() -> Self {
        LexConfig {
            tokens: TOKENS.iter().map(|s| (*s).to_string()).collect(),
            keywords: KEYWORDS.iter().map(|s| (*s).to_string()).collect(),
            comment: "//".to_string(),
            interpolate_strings: true,
            json_strings: false,
        }
    }
}

impl LexConfig {
    /// A lexicon for a config surface (e.g. `loft.toml`): the given `tokens` and
    /// `comment` marker, no keywords, and NO string interpolation (`{ }` are
    /// literal).  `comment` is added to `tokens` automatically so it lexes.
    #[must_use]
    pub fn config(tokens: &[&str], comment: &str) -> Self {
        let mut tokens: HashSet<String> = tokens.iter().map(|s| (*s).to_string()).collect();
        tokens.insert(comment.to_string());
        LexConfig {
            tokens,
            keywords: HashSet::new(),
            comment: comment.to_string(),
            interpolate_strings: false,
            json_strings: false,
        }
    }

    /// A lexicon for JSON (@PLN109): the structural tokens `{ } [ ] : ,` plus the
    /// number sign `-` (loft's lexer reports `-` as its own token; the JSON parser
    /// combines it with the following number, since a JSON number can be negative).
    /// No keywords (`true`/`false`/`null` lex as identifiers — the JSON parser
    /// dispatches on them), no comment, no `{…}` interpolation, and JSON string
    /// escapes enabled.
    #[must_use]
    pub fn json() -> Self {
        // `.` is a token so a `Dialect::Lenient` qualified enum tag (`Category.Daily`)
        // tokenises as `Ident "." Ident` rather than choking the lexer; the JSON
        // parser raw-scans the full dotted identifier and skips past these tokens.
        // (Fractional numbers like `1.5` are still read as a single number token
        // before the `.` token rule applies.)
        let tokens: HashSet<String> = ["{", "}", "[", "]", ":", ",", "-", "."]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        LexConfig {
            tokens,
            keywords: HashSet::new(),
            comment: String::new(),
            interpolate_strings: false,
            json_strings: true,
        }
    }
}

#[derive(Debug)]
pub struct Link {
    links: Rc<RefCell<u32>>,
    pos: usize,
}

impl Drop for Link {
    fn drop(&mut self) {
        *self.links.borrow_mut() -= 1;
    }
}

fn hex_parse(val: &str) -> Option<u64> {
    let mut res: u64 = 0;
    for ch in val.chars() {
        if ch.is_ascii_digit() {
            res = res * 16 + ch as u64 - '0' as u64;
        } else if ch.is_ascii_hexdigit() {
            res = res * 16 + 10 + ch.to_ascii_lowercase() as u64 - 'a' as u64;
        } else {
            return None;
        }
    }
    Some(res)
}

fn bin_parse(val: &str) -> Option<u64> {
    let mut res: u64 = 0;
    for ch in val.chars() {
        if ('0'..='1').contains(&ch) {
            res = res * 2 + ch as u64 - '0' as u64;
        } else {
            return None;
        }
    }
    Some(res)
}

fn oct_parse(val: &str) -> Option<u64> {
    let mut res: u64 = 0;
    for ch in val.chars() {
        if ('0'..='7').contains(&ch) {
            res = res * 8 + ch as u64 - '0' as u64;
        } else {
            return None;
        }
    }
    Some(res)
}

impl Default for Lexer {
    fn default() -> Self {
        let cfg = LexConfig::default();
        Lexer {
            virtual_files: std::collections::HashMap::new(),
            prev_end: Position {
                file: String::new(),
                line: 0,
                pos: 0,
            },
            lines: Box::new(Vec::new().into_iter()),
            peek: LexResult {
                has: LexItem::None,
                position: Position {
                    file: String::new(),
                    line: 0,
                    pos: 0,
                },
            },
            position: Position {
                file: String::new(),
                line: 0,
                pos: 0,
            },
            memory: Vec::new(),
            memory_state: Vec::new(),
            link: 0,
            links: Rc::new(RefCell::new(0)),
            seek_return: None,
            iter: LINE.chars().collect::<Vec<_>>().into_iter().peekable(),
            tokens: cfg.tokens,
            keywords: cfg.keywords,
            comment: cfg.comment,
            interpolate_strings: cfg.interpolate_strings,
            json_strings: cfg.json_strings,
            json_over_read: false,
            mode: Mode::Code,
            in_format_expr: false,
            open_strings: Vec::new(),
            backtick_strip: Vec::new(),
            diagnostics: Diagnostics::new(),
        }
    }
}

/// The closest existing sibling of `path` — a `.loft` file in the same directory
/// whose name is within the shared edit-distance cap.  Used to turn a mistyped
/// path into a suggestion rather than a dead end.
fn suggest_sibling_file(path: &str) -> Option<String> {
    let p = std::path::Path::new(path);
    let want = p.file_name()?.to_str()?;
    let dir = if p.parent()?.as_os_str().is_empty() {
        std::path::Path::new(".")
    } else {
        p.parent()?
    };
    let names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(std::result::Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "loft"))
        .filter_map(|e| e.file_name().to_str().map(String::from))
        .collect();
    let refs: Vec<&str> = names.iter().map(String::as_str).collect();
    crate::diagnostics::suggest_similar_capped(want, &refs).map(String::from)
}

impl Lexer {
    /// Construct a lexer over `lines` using `config` as its lexicon (tokens,
    /// keywords, comment marker, string-interpolation).  Pass
    /// `LexConfig::default()` for loft.
    fn new_with(
        lines: impl Iterator<Item = IoResult<String>> + 'static,
        filename: &str,
        config: LexConfig,
    ) -> Lexer {
        Lexer {
            virtual_files: std::collections::HashMap::new(),
            prev_end: Position {
                file: filename.to_string(),
                line: 0,
                pos: 0,
            },
            lines: Box::new(lines),
            peek: LexResult {
                has: LexItem::None,
                position: Position {
                    file: filename.to_string(),
                    line: 0,
                    pos: 0,
                },
            },
            position: Position {
                file: filename.to_string(),
                line: 0,
                pos: 0,
            },
            memory: Vec::new(),
            memory_state: Vec::new(),
            link: 0,
            links: Rc::new(RefCell::new(0)),
            seek_return: None,
            iter: LINE.chars().collect::<Vec<_>>().into_iter().peekable(),
            tokens: config.tokens,
            keywords: config.keywords,
            comment: config.comment,
            interpolate_strings: config.interpolate_strings,
            json_strings: config.json_strings,
            json_over_read: false,
            mode: Mode::Code,
            in_format_expr: false,
            open_strings: Vec::new(),
            backtick_strip: Vec::new(),
            diagnostics: Diagnostics::new(),
        }
    }

    /// Point the REPORTING position at `scope` so the next diagnostic carries it.
    ///
    /// The read cursor does not move: this is for a warning pass that walks a finished
    /// body and wants its caret on the declaration it is complaining about.  The seek
    /// lasts until the next token is scanned from the source, which then restores the
    /// real cursor — see [`seek_return`](Self#structfield.seek_return).  A pass that
    /// emits several diagnostics and then keeps parsing should still restore explicitly
    /// (`let p = lexer.at(); … lexer.to(p);`), because a position READ back with
    /// [`at`](Self::at) before the next token still sees the seek.
    pub fn to(&mut self, scope: (u32, u32)) {
        lex_trace(format_args!(
            "to() seek {}:{} -> {}:{}",
            self.position.line, self.position.pos, scope.0, scope.1
        ));
        if self.seek_return.is_none() {
            self.seek_return = Some((self.position.line, self.position.pos));
        }
        self.position.line = scope.0;
        self.position.pos = scope.1;
    }

    /// Undo a [`to`](Self::to) seek NOW, restoring the read cursor's own position and
    /// clearing the pending restore.
    ///
    /// For a pass that seeks around ONE call and then keeps raising diagnostics of its
    /// own.  Seeking BACK with a second `to` does not do this: the pending
    /// `seek_return` survives, and [`report_pos`](Self::report_pos) treats a live seek as
    /// a deliberate choice and stops attributing to the consumed source — so every later
    /// diagnostic in that pass silently reverts to the scan cursor.  Measured: seeking
    /// around the block-tail conversion this way sent `Not all code paths return a value`
    /// back onto the FOLLOWING function.
    pub fn end_seek(&mut self) {
        if let Some((line, pos)) = self.seek_return.take() {
            lex_trace(format_args!(
                "end_seek restore {}:{} -> {line}:{pos}",
                self.position.line, self.position.pos
            ));
            self.position.line = line;
            self.position.pos = pos;
        }
    }

    #[allow(clippy::too_many_lines)] // large lexer dispatch — splitting would obscure control flow
    fn next(&mut self) -> Option<LexResult> {
        if self.link < self.memory.len() {
            let n = self.memory[self.link].clone();
            if let Some(st) = self.memory_state.get(self.link).cloned() {
                self.restore_scan_state(st);
            }
            self.link += 1;
            lex_trace(format_args!(
                "replay {:?} @ {}:{} (cursor stays {}:{})",
                n.has, n.position.line, n.position.pos, self.position.line, self.position.pos
            ));
            return Some(n);
        }
        // Scanning fresh source: the read cursor is authoritative again, so a
        // reporting seek that was never undone ends here rather than shifting every
        // later position in the file (loft#625's mechanism, at a second site).
        if let Some((line, pos)) = self.seek_return.take() {
            lex_trace(format_args!(
                "seek_return restore {}:{} -> {line}:{pos}",
                self.position.line, self.position.pos
            ));
            self.position.line = line;
            self.position.pos = pos;
        }
        if self.mode != Mode::Formatting {
            loop {
                if let Some(&c) = self.iter.peek() {
                    if c != ' ' && c != '\t' {
                        break;
                    }
                    self.next_char();
                } else if let Some(line_result) = self.lines.next() {
                    match line_result {
                        Ok(ln) => {
                            if self.position.line == 0 && ln.starts_with("#!/") {
                                continue;
                            }
                            self.iter = ln.chars().collect::<Vec<_>>().into_iter().peekable();
                            self.position.line += 1;
                            self.position.pos = 1;
                        }
                        Err(e) => {
                            self.position.line += 1;
                            self.err(
                                Level::Fatal,
                                &format!(
                                    "Cannot read line {} — is the file valid UTF-8? ({})",
                                    self.position.line, e
                                ),
                            );
                            break;
                        }
                    }
                } else {
                    break;
                }
            }
        }
        let pos = self.position.clone();
        if let Some(&c) = self.iter.peek() {
            Some(match c {
                '0'..='9' => self.number(),
                '"' => {
                    self.next_char();
                    if self.in_format_expr {
                        self.string_nested(false, false)
                    } else {
                        self.string()
                    }
                }
                '`' => {
                    self.next_char();
                    let nested = self.in_format_expr;
                    // A fresh literal: its dedent is not settled until its first content
                    // line.  `close_backtick` pops it.
                    self.backtick_strip.push(None);
                    self.backtick_string(nested)
                }
                '\'' => {
                    self.next_char();
                    self.char()
                }
                ' ' | '\t' => {
                    self.next_char();
                    LexResult::new(LexItem::Token(" ".to_string()), pos)
                }
                _ => {
                    let single = String::from(c);
                    if self.tokens.contains(&single) {
                        self.next_char();
                        if let Some(&d) = self.iter.peek() {
                            let double = format!("{c}{d}");
                            if self.tokens.contains(&double) {
                                self.next_char();
                                LexResult::new(LexItem::Token(double), pos)
                            } else if self.mode == Mode::Formatting && single == "}" {
                                self.resume_string()
                            } else {
                                LexResult::new(LexItem::Token(single), pos)
                            }
                        } else {
                            LexResult::new(LexItem::Token(single), pos)
                        }
                    } else if c == '\\' && self.in_format_expr {
                        // `\"` inside a format expression opens a nested string literal.
                        self.next_char(); // consume '\'
                        if let Some(&nc) = self.iter.peek() {
                            if nc == '"' {
                                self.next_char(); // consume '"'
                                self.string_nested(true, false)
                            } else {
                                self.err(
                                    Level::Error,
                                    &format!("Escape '\\{nc}' is not allowed inside a {{...}} format expression — \
                                     use {{{{ to get a literal '{{' that won't start an expression"),
                                );
                                Lexer::none()
                            }
                        } else {
                            self.err(Level::Error, "Unexpected end of input after '\\'");
                            Lexer::none()
                        }
                    } else {
                        let ident = self.get_identifier();
                        if ident.is_empty() {
                            // An unrecognized character: not a known token, not a
                            // number/string/identifier start (e.g. a stray '\' in
                            // code).  `get_identifier` consumed nothing, so we must
                            // consume the offending char here — without this the
                            // lexer re-reads the same position forever, hanging the
                            // parser instead of reporting a clean error.
                            self.err(Level::Error, &format!("Unexpected character {c:?}"));
                            self.next_char();
                            Lexer::none()
                        } else if self.keywords.contains(&ident) {
                            LexResult::new(LexItem::Token(ident), pos)
                        } else {
                            LexResult::new(LexItem::Identifier(ident), pos)
                        }
                    }
                }
            })
        } else if let Some(line_result) = self.lines.next() {
            match line_result {
                Ok(ln) => {
                    self.iter = ln.chars().collect::<Vec<_>>().into_iter().peekable();
                    self.position.line += 1;
                    self.position.pos = 1;
                    Some(LexResult::new(LexItem::None, self.position.clone()))
                }
                Err(e) => {
                    self.position.line += 1;
                    self.err(
                        Level::Fatal,
                        &format!(
                            "Cannot read line {} — is the file valid UTF-8? ({})",
                            self.position.line, e
                        ),
                    );
                    None
                }
            }
        } else {
            None
        }
    }

    pub fn pos(&self) -> &Position {
        &self.position
    }

    /// Start position of the current lookahead token.  `pos()` returns the
    /// scan cursor (the *end* of the peeked token); a diagnostic that should
    /// point at the token the parser is about to consume needs its start.
    pub fn peek_pos(&self) -> &Position {
        &self.peek.position
    }

    pub fn at(&self) -> (u32, u32) {
        (self.position.line, self.position.pos)
    }

    /// Where a diagnostic raised right now should point.
    ///
    /// [`position`](Self#structfield.position) is the scan cursor — the END of the token
    /// the parser is currently holding.  Most diagnostics are about source already
    /// CONSUMED: a write to a `const` parameter, a nullable flowing into a non-null slot,
    /// a capture inside a parallel arm.  Those checks can only run once the construct is
    /// complete, and by then the lexer has moved on to the next token.  While that token
    /// is on the same line both answers agree, which is why the cursor looked right; when
    /// it is on a LATER line the cursor attributes the diagnostic to a line the construct
    /// does not occupy.  Measured: `a = 42` followed by `}` on its own line reported the
    /// const write AT the `}`, and blank lines between them carried the caret further —
    /// three lines from the code it names, with a different statement under it.
    ///
    /// So the report goes to the end of the consumed source whenever the current token
    /// has crossed a line boundary.  A site that really is about the token the parser is
    /// HOLDING — `'struct' definitions must be at file scope`, raised while looking at the
    /// keyword — says so explicitly with [`specific`](Self::specific) or
    /// [`pos_diagnostic`](Self::pos_diagnostic); that is a claim only the site can make,
    /// and it is the same shape as the 48 sites already reaching for
    /// [`peek_pos`](Self::peek_pos).
    ///
    /// A deliberate [`to`](Self::to) seek outranks both: that position was chosen.
    fn report_pos(&self) -> (u32, u32) {
        if self.seek_return.is_none()
            && self.prev_end.line > 0
            && self.prev_end.file == self.position.file
            && self.peek.position.line > self.prev_end.line
        {
            (self.prev_end.line, self.prev_end.pos)
        } else {
            (self.position.line, self.position.pos)
        }
    }

    #[track_caller]
    pub fn diagnostic(&mut self, level: Level, message: &str) {
        crate::diagnostics::audit_site(std::panic::Location::caller());
        let (line, pos) = self.report_pos();
        self.diagnostics
            .add_at(level, message, &self.position.file, line, pos);
    }

    /// Attach a machine-readable `suggestion` (a replacement token) to the
    /// diagnostic just emitted — call right after a "did you mean 'X'?" so a
    /// tool (`codeAction`) can apply `X` without parsing the prose.
    pub fn suggest_last(&mut self, suggestion: &str) {
        self.diagnostics.suggest_last(suggestion);
    }

    /// @PLN131 — attach "what to write instead" to the diagnostic just emitted.  Call it
    /// immediately after the `diagnostic!` that raised the problem, so the fix and its
    /// diagnostic stay one unit: a suggestion that has drifted from its diagnostic is
    /// misinformation.
    pub fn fix_last(&mut self, fix: Fix) {
        self.diagnostics.fix_last(fix);
    }

    /// The index of the entry a following [`Self::fix_last`] attaches to — for a fix whose
    /// edit is only spellable once more source has been parsed (loft#1003).
    #[must_use]
    pub fn last_diagnostic_index(&self) -> Option<usize> {
        self.diagnostics.last_index()
    }

    /// Attach the edit to an earlier fix, named by the index
    /// [`Self::last_diagnostic_index`] answered.
    pub fn set_fix_edit(&mut self, entry: usize, fix_at: usize, edit: crate::diagnostics::Edit) {
        self.diagnostics.set_fix_edit(entry, fix_at, edit);
    }

    /// Emit a diagnostic carrying a stable `code` (kebab-case kind slug).
    /// @PLN102 arc-E E1 — the code is the frozen identity; prose is free.
    #[track_caller]
    pub fn diagnostic_coded(&mut self, level: Level, code: &'static str, message: &str) {
        crate::diagnostics::audit_site(std::panic::Location::caller());
        let (line, pos) = self.report_pos();
        self.diagnostics
            .add_at_coded(level, Some(code), message, &self.position.file, line, pos);
    }

    #[track_caller]
    pub fn specific(&mut self, result: &LexResult, level: Level, message: &str) {
        crate::diagnostics::audit_site(std::panic::Location::caller());
        self.diagnostics.add_at(
            level,
            message,
            &self.position.file,
            result.position.line,
            result.position.pos,
        );
    }

    #[track_caller]
    pub fn pos_diagnostic(&mut self, level: Level, pos: &Position, message: &str) {
        crate::diagnostics::audit_site(std::panic::Location::caller());
        self.diagnostics
            .add_at(level, message, &pos.file, pos.line, pos.pos);
    }

    /// Like [`pos_diagnostic`], but carrying a stable `code` — the explicit-position twin of
    /// [`diagnostic_coded`](Lexer::diagnostic_coded).
    #[track_caller]
    pub fn pos_diagnostic_coded(
        &mut self,
        level: Level,
        pos: &Position,
        code: &'static str,
        message: &str,
    ) {
        crate::diagnostics::audit_site(std::panic::Location::caller());
        self.diagnostics
            .add_at_coded(level, Some(code), message, &pos.file, pos.line, pos.pos);
    }

    pub fn diagnostics(&self) -> &Diagnostics {
        &self.diagnostics
    }

    /// See [`Diagnostics::rewind`].
    pub fn rewind_diagnostics(&mut self, mark: (usize, Level)) {
        self.diagnostics.rewind(mark);
    }

    /// loft#1260 — tell this lexer's diagnostics who the compilation's lints are addressed
    /// to.  See [`Diagnostics::reaches_author`].
    pub fn set_lint_scope_for(&mut self, entry: &str) {
        self.diagnostics.set_lint_scope_for(entry);
    }

    pub fn mode(&self) -> Mode {
        self.mode.clone()
    }

    /// Resume the string whose `{` opened the format expression a `}` just
    /// closed. One home for that decision: it is asked from two places, and
    /// answering it differently in either is how a nested string stopped being
    /// resumed at all (loft#767).
    ///
    /// An empty stack means the `}` closed a hole nothing recorded — a plain
    /// string is the pre-existing behaviour and the safe answer.
    fn resume_string(&mut self) -> LexResult {
        self.in_format_expr = false;
        match self.open_strings.pop() {
            Some(StrKind::Backtick(nested)) => self.backtick_string_resume(nested),
            Some(StrKind::Nested(escaped_delim)) => self.string_nested(escaped_delim, true),
            _ => self.string(),
        }
    }

    /// Does the string literal just returned as a `CString` have a `{…}` of its
    /// own still open?
    ///
    /// `mode` cannot answer this. The lexer runs one token ahead, so a nested
    /// literal is scanned — and its hole opened — BEFORE the enclosing
    /// `parse_string` starts; that loop then sets `Mode::Code` to read its own
    /// expression, and by the time the parser reaches the nested literal the
    /// mode records where the LEXER is, not what this string is. Reading it
    /// anyway is what made `"{"{y}"}"` come out as `{y}` (loft#767).
    #[must_use]
    pub fn nested_hole_open(&self) -> bool {
        matches!(
            self.open_strings.last(),
            Some(StrKind::Nested(_) | StrKind::Backtick(true))
        )
    }

    fn scan_state(&self) -> ScanState {
        ScanState {
            mode: self.mode.clone(),
            in_format_expr: self.in_format_expr,
            open_strings: self.open_strings.clone(),
            backtick_strip: self.backtick_strip.clone(),
        }
    }

    fn restore_scan_state(&mut self, st: ScanState) {
        self.mode = st.mode;
        self.in_format_expr = st.in_format_expr;
        self.open_strings = st.open_strings;
        self.backtick_strip = st.backtick_strip;
    }

    pub fn set_mode(&mut self, mode: Mode) {
        if mode == Mode::Formatting && self.peek_token("}") {
            self.mode = mode;
            self.peek = self.resume_string();
            // The rest of the string is read straight from the source, not through `cont()`,
            // so the replay buffer still holds the `}` it replaces.  A revert past this point
            // replayed that `}`, and resuming again read from wherever the live scan had got
            // to: a literal parsed a second time refused every interpolation inside it
            // (`Expect token ]`).  The resumed string takes the `}`'s slot, so a replay hands
            // back what was read.
            if self.count_links() > 0 && self.link > 0 && self.link <= self.memory.len() {
                self.memory[self.link - 1] = self.peek.clone();
                self.memory_state[self.link - 1] = self.scan_state();
            }
        } else {
            self.mode = mode;
        }
    }

    #[allow(dead_code)]
    pub fn whitespace(&mut self) {
        while self.peek_token(" ") || self.peek_token("\t") {
            self.cont();
        }
    }

    fn none() -> LexResult {
        LexResult {
            has: LexItem::None,
            position: Position {
                file: String::new(),
                line: 0,
                pos: 0,
            },
        }
    }

    /// Parse a character constant for the lexer.
    fn char(&mut self) -> LexResult {
        let pos = self.position.clone();
        let mut res = String::new();
        while let Some(&c) = self.iter.peek() {
            if c == '\'' {
                self.next_char();
                let mut chars = res.chars();
                return LexResult::new(
                    LexItem::Character(if let Some(ch) = chars.next() {
                        if chars.next().is_some() {
                            self.err(Level::Error, "Expected only one character in constant");
                        }
                        ch as u32
                    } else {
                        self.err(Level::Error, "Expected a character in constant");
                        0
                    }),
                    pos,
                );
            }
            if c == '\\' {
                self.next_char();
                if !self.escape_seq(&mut res) {
                    break;
                }
            } else if c == '\n' {
                break;
            } else {
                res.push(c);
            }
            self.next_char();
        }
        self.err(Level::Fatal, "Character not correctly terminated");
        Lexer::none()
    }

    fn escape_seq(&mut self, res: &mut String) -> bool {
        // Convention: escape_seq processes ONE designator char (peeked
        // by self.iter.peek()).  The caller's outer loop advances past
        // that char via self.next_char() AFTER escape_seq returns.  So
        // multi-char escapes (\xNN, \u{NNNN}) advance past every char
        // EXCEPT the last one (which the caller will skip).
        if let Some(&c) = self.iter.peek() {
            match c {
                '"' | '\'' | '\\' => res.push(c),
                // @PLN109 JSON: `\/` is a JSON escape for `/`; loft rejects it.
                '/' if self.json_strings => res.push('/'),
                't' => res.push('\t'),
                'r' => res.push('\r'),
                'n' | '\n' => res.push('\n'),
                '0' => res.push('\0'),
                'x' => {
                    // \xNN — two hex digits, ASCII range only (0x00-0x7F).
                    // Higher codepoints must use \u{NNNN} (a single
                    // \xFF byte isn't valid UTF-8 on its own).
                    self.next_char(); // consume 'x', now at first hex digit
                    let h1 = self.iter.peek().copied();
                    let h2 = h1.and_then(|_| {
                        self.next_char(); // consume first hex; now at second
                        self.iter.peek().copied()
                    });
                    // Don't advance past second hex — caller does.
                    if let (Some(d1), Some(d2)) = (h1, h2) {
                        if let (Some(v1), Some(v2)) = (d1.to_digit(16), d2.to_digit(16)) {
                            let byte = (v1 << 4) | v2;
                            if byte < 0x80 {
                                res.push(char::from(byte as u8));
                            } else {
                                self.err(
                                    Level::Error,
                                    "\\xNN escape only supports ASCII (00-7F); use \\u{NN} for higher codepoints",
                                );
                                res.push('?');
                            }
                        } else {
                            self.err(Level::Error, "\\xNN escape requires two hex digits");
                            res.push('?');
                        }
                    } else {
                        self.err(Level::Error, "\\xNN escape requires two hex digits");
                        res.push('?');
                    }
                }
                'u' => {
                    // \u{NNNN} — Unicode codepoint, 1-6 hex digits, must
                    // be a valid Unicode scalar value (excludes surrogates).
                    self.next_char(); // consume 'u', now at '{' (loft) or first hex (JSON)
                    if self.iter.peek() != Some(&'{') {
                        // @PLN109 JSON mode: `\uXXXX` (four hex, no braces) with
                        // surrogate-pair combining.  loft's own form needs `\u{…}`.
                        if self.json_strings {
                            self.json_unicode_escape(res);
                            return true;
                        }
                        self.err(Level::Error, "\\u escape requires \\u{NNNN} form");
                        res.push('?');
                        return true;
                    }
                    self.next_char(); // consume '{', now at first hex digit
                    let mut hex = String::new();
                    while let Some(&ch) = self.iter.peek() {
                        if ch.is_ascii_hexdigit() {
                            hex.push(ch);
                            self.next_char();
                        } else {
                            break;
                        }
                    }
                    // Iterator now at '}' (or some other char if malformed).
                    // Don't advance past '}' — caller does.
                    if self.iter.peek() != Some(&'}') {
                        self.err(Level::Error, "\\u{NNNN} escape requires closing brace");
                        res.push('?');
                        return true;
                    }
                    if hex.is_empty() || hex.len() > 6 {
                        self.err(Level::Error, "\\u{NNNN} escape requires 1-6 hex digits");
                        res.push('?');
                        return true;
                    }
                    if let Ok(cp) = u32::from_str_radix(&hex, 16) {
                        if let Some(ch) = char::from_u32(cp) {
                            res.push(ch);
                        } else {
                            self.err(
                                Level::Error,
                                "\\u{NNNN} escape is not a valid Unicode codepoint",
                            );
                            res.push('?');
                        }
                    } else {
                        self.err(Level::Error, "\\u{NNNN} escape has invalid hex");
                        res.push('?');
                    }
                }
                _ => {
                    self.err(Level::Error, "Unknown escape sequence");
                    res.push('?');
                }
            }
            true
        } else {
            false
        }
    }

    /// @PLN109 — decode a JSON `\uXXXX` escape (four hex digits, no braces),
    /// already positioned past the `u`, with surrogate-pair combining.  Follows
    /// the [`escape_seq`](Self::escape_seq) convention: on return the iterator's
    /// peek is at the LAST consumed hex digit (the caller's outer loop skips it).
    /// Pushes the decoded scalar, or `\u{FFFD}` + a diagnostic for a malformed or
    /// unpaired-surrogate escape.
    fn json_unicode_escape(&mut self, res: &mut String) {
        let Some(cp) = self.read_hex4() else {
            self.err(Level::Error, "\\uXXXX escape requires four hex digits");
            res.push('\u{FFFD}');
            return;
        };
        // Not a surrogate — a direct Unicode scalar value.
        if !(0xD800..=0xDFFF).contains(&cp) {
            if let Some(ch) = char::from_u32(cp) {
                res.push(ch);
            } else {
                self.err(Level::Error, "\\uXXXX escape is not a valid codepoint");
                res.push('\u{FFFD}');
            }
            return;
        }
        // A low surrogate cannot lead a pair.
        if cp >= 0xDC00 {
            self.err(Level::Error, "\\uXXXX unpaired low surrogate");
            res.push('\u{FFFD}');
            return;
        }
        // High surrogate: require a following `\uXXXX` low surrogate.  Peek is at
        // this high surrogate's 4th hex; consume it, then match `\ u X X X X`.
        self.next_char(); // consume the high surrogate's 4th hex digit
        let saw_backslash = self.iter.peek() == Some(&'\\');
        let paired = saw_backslash && {
            self.next_char(); // consume '\'
            self.iter.peek() == Some(&'u')
        };
        if !paired {
            self.err(Level::Error, "\\uXXXX unpaired high surrogate");
            res.push('\u{FFFD}');
            // We over-read: peek is now PAST the escape (at the char after the 4th
            // hex, or after a stray `\`), not AT its last char.  Signal `string()` to
            // skip its post-escape advance so the next byte (e.g. the closing `"`,
            // matching the old scanner's `Ok(Str("\u{FFFD}"))`) is not swallowed.
            self.json_over_read = true;
            return;
        }
        self.next_char(); // consume 'u', now at the low surrogate's first hex
        match self.read_hex4() {
            Some(low) if (0xDC00..=0xDFFF).contains(&low) => {
                let c = 0x1_0000 + ((cp - 0xD800) << 10) + (low - 0xDC00);
                res.push(char::from_u32(c).unwrap_or('\u{FFFD}'));
            }
            Some(low) => {
                // High surrogate followed by a `\uXXXX` that is NOT a low
                // surrogate: the high is lone (U+FFFD), and the second escape
                // decodes as its OWN codepoint — a BMP scalar (`A` → `A`),
                // or U+FFFD if it too is a surrogate.  Matches the old byte-scanner,
                // which left the second `\uXXXX` for the next pass rather than
                // swallowing it.
                self.err(
                    Level::Error,
                    "\\uXXXX high surrogate not followed by a low surrogate",
                );
                res.push('\u{FFFD}');
                if (0xD800..=0xDFFF).contains(&low) {
                    res.push('\u{FFFD}');
                } else {
                    res.push(char::from_u32(low).unwrap_or('\u{FFFD}'));
                }
            }
            None => {
                self.err(
                    Level::Error,
                    "\\uXXXX high surrogate not followed by a low surrogate",
                );
                res.push('\u{FFFD}');
            }
        }
    }

    /// Read exactly four hex digits, consuming the first three and leaving the
    /// iterator's peek at the fourth (the [`escape_seq`](Self::escape_seq)
    /// last-char convention).  Returns `None` if four hex digits do not follow.
    fn read_hex4(&mut self) -> Option<u32> {
        let mut val: u32 = 0;
        for i in 0..4 {
            let d = self.iter.peek().and_then(|c| c.to_digit(16))?;
            val = (val << 4) | d;
            if i < 3 {
                self.next_char(); // leave peek at the 4th digit
            }
        }
        Some(val)
    }

    /// Parse a string for the lexer.
    fn string(&mut self) -> LexResult {
        let pos = self.position.clone();
        let mut res = String::new();
        while let Some(&c) = self.iter.peek() {
            if c == '"' {
                self.mode = Mode::Code;
                self.next_char();
                return LexResult::new(LexItem::CString(res), pos);
            }
            if c == '\\' {
                self.next_char();
                if !self.escape_seq(&mut res) {
                    break;
                }
                // @PLN109: an unpaired `\uXXXX` high surrogate over-read past its 4th
                // hex; the char iterator is already at the next char, so skip the
                // shared advance below (which would swallow it — e.g. the closing `"`).
                if self.json_over_read {
                    self.json_over_read = false;
                    continue;
                }
            } else if c == '\n' {
                break;
            } else if c == '{' && self.interpolate_strings {
                self.next_char();
                if let Some('{') = self.iter.peek() {
                    res.push(c);
                } else if !self.hole_closes_on_this_line() {
                    // Say it here, where the `{` is, and carry on as though the author
                    // had written `{{` — which is the fix offered.  Opening a hole that
                    // cannot close hands the rest of the line to the wrong scanner, and
                    // that is what turned one mistake into six diagnostics (loft#989).
                    self.unclosed_hole();
                    res.push('{');
                    continue;
                } else {
                    self.mode = Mode::Formatting;
                    self.in_format_expr = true;
                    self.open_strings.push(StrKind::Plain);
                    return LexResult::new(LexItem::CString(res), pos);
                }
            } else if c == '}' && self.interpolate_strings {
                self.next_char();
                if let Some('}') = self.iter.peek() {
                    res.push(c);
                } else {
                    self.unescaped_brace();
                }
            } else {
                // With interpolation off (configs), `{` / `}` fall here as literal
                // string content.
                res.push(c);
            }
            self.next_char();
        }
        self.err(Level::Fatal, "String not correctly terminated");
        Lexer::none()
    }

    /// Scan a string literal that appears as an expression inside a `{...}` format slot.
    ///
    /// When `escaped_delim` is false (opened by bare `"`), the string closes on
    /// a bare `"` and `\"` is a normal escape producing a literal quote.  This
    /// is the path used by `.loft` source files: `"text {"inner \"quoted\""}"`.
    ///
    /// When `escaped_delim` is true (opened by `\"`), the string closes on `\"`
    /// as well as bare `"`.  This preserves backward compatibility with Rust
    /// test macros where the source already has the outer quotes escaped:
    /// `"text {\"inner\"}"`.
    /// Finish a nested string literal.
    ///
    /// `Mode::Code` because the STRING is complete: its own `parse_string` loops
    /// while the mode is `Formatting`, so leaving it there would make it hunt
    /// for a hole that belongs to the string outside it. The parser puts the
    /// mode back to `Formatting` when it is done with the expression, and that
    /// is what lets the outer `}` resume the outer string.
    ///
    /// `in_format_expr` stays TRUE: the format expression this literal sits in
    /// is still open, so a following `"` opens another nested string rather than
    /// closing the outer one. On the resumed path a `}` has just cleared it, and
    /// that is exactly the case this restores.
    /// Finish a backtick literal. Same rule as [`close_nested`], because a
    /// `` `…` `` written inside a `{…}` is a nested literal in exactly the way a
    /// `"…"` is: the enclosing format expression is still open, so the mode must
    /// keep describing THAT string and a following `"` or `` ` `` must still open
    /// a nested literal rather than close something. Resetting to `Code`
    /// unconditionally is what made a backtick unusable inside an interpolation
    /// at all — even `` "{`abc`}" ``, with no hole of its own (loft#767).
    fn close_backtick(
        &mut self,
        res: String,
        pos: Position,
        nested: bool,
        resumed: bool,
    ) -> LexResult {
        self.backtick_strip.pop();
        if nested {
            return self.close_nested(res, pos, resumed);
        }
        self.mode = Mode::Code;
        LexResult::new(LexItem::CString(res), pos)
    }

    fn close_nested(&mut self, res: String, pos: Position, resumed: bool) -> LexResult {
        if resumed {
            self.mode = Mode::Code;
        }
        self.in_format_expr = true;
        LexResult::new(LexItem::CString(res), pos)
    }

    fn string_nested(&mut self, escaped_delim: bool, resumed: bool) -> LexResult {
        let pos = self.position.clone();
        let mut res = String::new();
        while let Some(&c) = self.iter.peek() {
            if c == '"' {
                // Bare " always closes the nested string literal.
                self.next_char();
                return self.close_nested(res, pos, resumed);
            }
            if c == '\\' {
                self.next_char(); // consume '\'
                if escaped_delim && let Some(&'"') = self.iter.peek() {
                    // Opened by \" → \" also closes.
                    self.next_char();
                    return self.close_nested(res, pos, resumed);
                }
                // Normal escape sequence (including \" when !escaped_delim).
                if !self.escape_seq(&mut res) {
                    break;
                }
            } else if c == '\n' {
                break;
            } else if c == '{' && self.interpolate_strings {
                // A nested string interpolates like any other. Without this its
                // `{…}` was copied out as text, so `"{"{y}"}"` printed `{y}` —
                // no error, no warning, a plausible wrong VALUE that survived
                // being consumed (loft#767).
                self.next_char();
                if let Some('{') = self.iter.peek() {
                    res.push(c);
                } else if !self.hole_closes_on_this_line() {
                    self.unclosed_hole();
                    res.push('{');
                    continue;
                } else {
                    self.mode = Mode::Formatting;
                    self.in_format_expr = true;
                    self.open_strings.push(StrKind::Nested(escaped_delim));
                    return LexResult::new(LexItem::CString(res), pos);
                }
            } else if c == '}' && self.interpolate_strings {
                self.next_char();
                if let Some('}') = self.iter.peek() {
                    res.push(c);
                } else {
                    // Reached only with no hole open in THIS string — the outer
                    // `}` is consumed by the token path, never here.
                    self.unescaped_brace();
                }
            } else {
                res.push(c);
            }
            self.next_char();
        }
        self.err(
            Level::Fatal,
            "Nested string literal not correctly terminated",
        );
        Lexer::none()
    }

    /// Advance to the next source line inside a multi-line backtick string.
    /// Returns false at end-of-file.
    fn advance_line(&mut self) -> bool {
        if let Some(line_result) = self.lines.next() {
            match line_result {
                Ok(ln) => {
                    self.iter = ln.chars().collect::<Vec<_>>().into_iter().peekable();
                    self.position.line += 1;
                    self.position.pos = 1;
                    true
                }
                Err(_) => false,
            }
        } else {
            false
        }
    }

    /// Consume the leading spaces of the line just entered inside a backtick literal,
    /// and answer what survives its dedent.
    ///
    /// **The dedent rule is the FIRST CONTENT LINE's indentation.**  That many leading
    /// spaces come off every line; a line indented LESS than the base loses all of its
    /// own and comes out flush, which is the only answer that cannot leave it further
    /// right than a sibling that was indented past it.  A tab-indented line is untouched
    /// — a tab is not a space, so there is nothing to count.
    ///
    /// It reads off the first content line because that is the only anchor a literal
    /// with an interpolation has.  The rule used to be the CLOSING backtick's column,
    /// which the scanner only learns when it gets there — and a hole makes it hand the
    /// text so far to the parser and return long before that, so every segment of a
    /// holed literal was built with no dedent at all.  The feature worked for a block
    /// with no values in it and silently stopped for a template, which is the shape it
    /// exists for (loft#990).  Reading the first content line instead is knowable
    /// before any hole can occur, so holed and unholed literals answer alike.
    ///
    /// A BLANK line settles nothing — a template may open with one, and taking its zero
    /// indentation as the anchor would switch the dedent off for the whole block.  Nor
    /// does the CLOSING line: it is dropped when it holds only whitespace, so it is not
    /// content either.  The line the opening backtick sits on cannot be an anchor at
    /// all: it starts wherever that backtick ended, so its indentation is the
    /// statement's, not the block's.
    fn backtick_line_start(&mut self) -> String {
        let mut spaces = 0usize;
        while let Some(&' ') = self.iter.peek() {
            spaces += 1;
            self.next_char();
        }
        // Peek decides what this line IS, and every case is settled right here: a
        // special character or ordinary text means content, the closing backtick means
        // the last line, and end-of-line means blank.
        let settles = !matches!(self.iter.peek(), None | Some('`'));
        let strip = match self.backtick_strip.last_mut() {
            Some(slot) => {
                if settles && slot.is_none() {
                    *slot = Some(spaces);
                }
                slot.unwrap_or(0)
            }
            None => 0,
        };
        " ".repeat(spaces.saturating_sub(strip))
    }

    /// Drop a final line that holds only whitespace, and the newline before it.
    ///
    /// The closing backtick's own line is layout, not content — `` `\n  a\n  ` `` is
    /// one line of text.  [`backtick_string`](Lexer::backtick_string) has always said so
    /// for a literal it scans in one piece; a RESUMED segment needs it said again, which
    /// is why a holed block kept a trailing run of spaces nothing asked for (loft#990).
    fn drop_trailing_blank_line(text: &str) -> &str {
        match text.rfind('\n') {
            Some(nl) if text[nl + 1..].chars().all(|c| c == ' ' || c == '\t') => &text[..nl],
            _ => text,
        }
    }

    /// Scan a backtick string literal: `` `...` ``.
    ///
    /// Multi-line, supports `{expr}` interpolation and `{{`/`}}` escaping.
    /// Bare `"` is literal (no escaping needed).  `\` escapes work as usual.
    /// Closes on the next `` ` ``.
    ///
    /// **Indent stripping:** the FIRST CONTENT LINE's indentation defines the base;
    /// that many leading spaces are removed from every line of the content.  The
    /// first line (on the same line as the opening `` ` ``) and the last line (on the
    /// same line as the closing `` ` ``) are trimmed if they contain only whitespace.
    /// [`backtick_line_start`](Lexer::backtick_line_start) applies it, one line at a
    /// time as the line is entered — which is what lets a literal with an
    /// interpolation in it be dedented at all (loft#990).
    fn backtick_string(&mut self, nested: bool) -> LexResult {
        let pos = self.position.clone();
        let mut lines: Vec<String> = Vec::new();
        let mut cur = String::new();

        loop {
            match self.iter.peek() {
                Some(&'`') => {
                    self.next_char();
                    lines.push(cur);
                    // Each line arrived already dedented — `backtick_line_start` took
                    // its share off as the line was entered.  What is left here is the
                    // two layout lines: the opening backtick's and the closing one's,
                    // each dropped when it holds only whitespace.
                    let mut result = String::new();
                    for (i, line) in lines.iter().enumerate() {
                        if i == 0 {
                            // First line: content after opening backtick on same line.
                            if !line.trim().is_empty() {
                                result += line;
                            }
                            continue;
                        }
                        // Last line before closing backtick: skip if whitespace-only.
                        if i == lines.len() - 1 && line.trim().is_empty() {
                            continue;
                        }
                        if !result.is_empty() {
                            result.push('\n');
                        }
                        result += line;
                    }
                    return self.close_backtick(result, pos, nested, false);
                }
                Some(&'{') => {
                    self.next_char();
                    if let Some('{') = self.iter.peek() {
                        cur.push('{');
                    } else if !self.hole_closes_on_this_line() {
                        self.unclosed_hole();
                        cur.push('{');
                        continue;
                    } else {
                        // Enter format interpolation — return what we have so far.
                        lines.push(std::mem::take(&mut cur));
                        let mut result = String::new();
                        for (i, line) in lines.iter().enumerate() {
                            if i == 0 && line.trim().is_empty() {
                                continue;
                            }
                            if !result.is_empty() {
                                result.push('\n');
                            }
                            result += line;
                        }
                        self.mode = Mode::Formatting;
                        self.in_format_expr = true;
                        self.open_strings.push(StrKind::Backtick(nested));
                        return LexResult::new(LexItem::CString(result), pos);
                    }
                }
                Some(&'}') => {
                    self.next_char();
                    if let Some('}') = self.iter.peek() {
                        cur.push('}');
                    } else {
                        self.unescaped_brace();
                    }
                }
                Some(&'\\') => {
                    self.next_char();
                    if !self.escape_seq(&mut cur) {
                        self.backtick_strip.pop();
                        self.err(Level::Fatal, "Backtick string not correctly terminated");
                        return Lexer::none();
                    }
                }
                Some(&c) => {
                    cur.push(c);
                }
                None => {
                    // End of line — advance to next line.
                    lines.push(std::mem::take(&mut cur));
                    if !self.advance_line() {
                        self.backtick_strip.pop();
                        self.err(Level::Fatal, "Backtick string not correctly terminated");
                        return Lexer::none();
                    }
                    // The dedent comes off here, at the line start, where the leading
                    // spaces are — not at the close, which a hole never reaches.
                    cur = self.backtick_line_start();
                    while let Some(&c) = self.iter.peek() {
                        if c == '`' || c == '{' || c == '}' || c == '\\' {
                            break;
                        }
                        cur.push(c);
                        self.next_char();
                    }
                    continue; // don't call next_char — we're positioned at the special char
                }
            }
            self.next_char();
        }
    }

    /// Resume a backtick string after a `}` closes a format expression.
    /// Called from the `}` token handler when the backtick string owns the
    /// format context.
    fn backtick_string_resume(&mut self, nested: bool) -> LexResult {
        let pos = self.position.clone();
        let mut cur = String::new();
        loop {
            match self.iter.peek() {
                Some(&'`') => {
                    self.next_char();
                    // The closing backtick's own line is layout: drop it when it holds
                    // only whitespace, exactly as the unresumed scanner does.
                    let cur = Self::drop_trailing_blank_line(&cur).to_string();
                    return self.close_backtick(cur, pos, nested, true);
                }
                Some(&'{') => {
                    self.next_char();
                    if let Some('{') = self.iter.peek() {
                        cur.push('{');
                    } else if !self.hole_closes_on_this_line() {
                        self.unclosed_hole();
                        cur.push('{');
                        continue;
                    } else {
                        self.mode = Mode::Formatting;
                        self.in_format_expr = true;
                        self.open_strings.push(StrKind::Backtick(nested));
                        return LexResult::new(LexItem::CString(cur), pos);
                    }
                }
                Some(&'}') => {
                    self.next_char();
                    if let Some('}') = self.iter.peek() {
                        cur.push('}');
                    } else {
                        self.unescaped_brace();
                    }
                }
                Some(&'\\') => {
                    self.next_char();
                    if !self.escape_seq(&mut cur) {
                        self.backtick_strip.pop();
                        self.err(Level::Fatal, "Backtick string not correctly terminated");
                        return Lexer::none();
                    }
                }
                Some(&c) => {
                    cur.push(c);
                }
                None => {
                    cur.push('\n');
                    if !self.advance_line() {
                        self.backtick_strip.pop();
                        self.err(Level::Fatal, "Backtick string not correctly terminated");
                        return Lexer::none();
                    }
                    // Same dedent, same place: a segment after a hole is still made of
                    // the literal's own lines.
                    cur += &self.backtick_line_start();
                    continue;
                }
            }
            self.next_char();
        }
    }

    fn next_char(&mut self) {
        self.iter.next();
        self.position.pos += 1;
    }

    fn get_identifier(&mut self) -> String {
        let mut string = String::new();
        while let Some(&ident) = self.iter.peek() {
            if ident.is_ascii_lowercase()
                || ident.is_ascii_uppercase()
                || ident.is_ascii_digit()
                || ident == '_'
            {
                string.push(ident);
                self.next_char();
            } else {
                break;
            }
        }
        string
    }

    /// Scan a run of digits and return the digit string (with any `_`
    /// separators stripped) together with the size of each digit group the
    /// separators carve out.  L11: a `_` is accepted only *between two
    /// digits*; a misplaced `_` (trailing, doubled, or next to a radix
    /// prefix / `.` / `e`) is a hard error.  A leading `_` never reaches
    /// here — `_1` lexes as an identifier.  The group sizes let `number()`
    /// lint non-thousands grouping on the decimal integer part.
    fn get_number(&mut self) -> (String, Vec<usize>) {
        let mut number = String::new();
        let mut hex = false;
        let mut groups: Vec<usize> = Vec::new();
        let mut group_start = 0usize; // number.len() at the last separator
        let mut prev_was_digit = false; // was the last CONSUMED char a (hex)digit?
        while let Some(&c) = self.iter.peek() {
            if c.is_ascii_digit() {
                number.push(c);
                self.next_char();
                prev_was_digit = true;
            } else if matches!(c, 'x' | 'b' | 'o') && !hex && number == "0" {
                // A base marker belongs to the literal only directly after a leading `0`
                // — `0x1f`, `0b1010`, `0o17`.  Elsewhere the letter is the next token, and
                // the three must agree about that: while `b` and `o` were taken anywhere
                // in a number, `{10:6b}` scanned `6b`, failed to parse it and reported
                // "Problem parsing number", so a width could be written before `x` but
                // never before `b` or `o`.
                hex = c == 'x';
                number.push(c);
                self.next_char();
                prev_was_digit = false;
            } else if hex && (('a'..='f').contains(&c) || ('A'..='F').contains(&c)) {
                number.push(c);
                self.next_char();
                prev_was_digit = true;
            } else if c == '_' {
                // Digit separator (L11): valid only between two digits.
                self.next_char();
                let next_is_digit = self
                    .iter
                    .peek()
                    .is_some_and(|&n| n.is_ascii_digit() || (hex && n.is_ascii_hexdigit()));
                if prev_was_digit && next_is_digit {
                    groups.push(number.len() - group_start);
                    group_start = number.len();
                } else {
                    self.err(
                        Level::Error,
                        "Misplaced '_' in number literal — separators go between digits",
                    );
                    // Swallow the rest of a `_` run so `1__0` is one error, not one per `_`.
                    while self.iter.peek() == Some(&'_') {
                        self.next_char();
                    }
                }
                prev_was_digit = false;
            } else {
                break;
            }
        }
        groups.push(number.len() - group_start);
        (number, groups)
    }

    /// Parse a number for the lexer.
    fn number(&mut self) -> LexResult {
        let pos = self.position.clone();
        let (mut val, int_groups) = self.get_number();
        // L11 thousands-grouping lint: warn (but still accept) when decimal `_`
        // separators don't carve standard 3-digit groups — the leftmost group
        // may be 1-3 digits, every group after it must be exactly 3.  Skipped for
        // hex/bin/oct (those group by 4/8, not 3) and for un-separated numbers.
        if int_groups.len() > 1
            && !val.starts_with("0x")
            && !val.starts_with("0b")
            && !val.starts_with("0o")
            && (int_groups[0] > 3 || int_groups[1..].iter().any(|&g| g != 3))
        {
            self.err_coded(
                Level::Warning,
                "digit-separator-grouping",
                "Digit separators '_' are not on thousands boundaries (expected groups of 3)",
            );
            self.fix_last(crate::diagnostics::Fix {
                kind: crate::diagnostics::FixKind::Conditional,
                title: "regroup the separators in threes".to_string(),
                condition: Some(
                    "you meant thousands — a different grouping may be deliberate".to_string(),
                ),
                edit: None,
                concept: "numeric literals",
                concept_ref: "@F3",
            });
        }
        let mut f = false;
        // P195: when the previous emitted token was a `.` (field
        // access), the current number is a tuple/struct field index —
        // never a float.  `n.v.0.0` must lex as `n`, `.`, `v`, `.`,
        // `0`, `.`, `0` instead of `n`, `.`, `v`, `.`, `0.0`.
        let prev_was_field_dot = self.peek.has == LexItem::Token(".".to_string());
        if let Some('.') = self.iter.peek() {
            self.next_char();
            if let Some('.') = self.iter.peek() {
                self.next_char();
                self.link = self.memory.len();
                self.memory_state.push(self.scan_state());
                self.memory.push(LexResult::new(
                    LexItem::Token("..".to_string()),
                    pos.clone(),
                ));
                // Through `ret_number`, NOT a bare `u32` parse (loft#1559).  This
                // short-circuit is taken when a number is followed by `..`, and parsing it
                // as `u32` skipped the width split every other integer literal goes
                // through: `3000000000..` became `Integer(3000000000u32)`, which the
                // parser narrowed to `Value::Int(-1294967296)`, so `for i in 3000000000..N`
                // started 2^32 below where it says and ran unbounded, while a NEGATIVE
                // start flipped sign and ran zero times.  Both silent, on both backends.
                return if let Ok(r) = val.parse::<u64>() {
                    self.ret_number(r, pos, val.starts_with('0'))
                } else {
                    self.err(
                        Level::Error,
                        "Integer literal out of range (exceeds i64::MAX)",
                    );
                    self.ret_number(0, pos, false)
                };
            }
            if prev_was_field_dot {
                // P195 + P234: when the previous emitted token was a `.`
                // (field access), the current number is a tuple/struct
                // field index — NEVER a float fragment.  This covers two
                // shapes:
                //   - `n.v.0.0` → `n`, `.`, `v`, `.`, `0`, `.`, `0`
                //     (P195 — nested tuple index access).
                //   - `r.0.x`   → `r`, `.`, `0`, `.`, `x`
                //     (P234 — struct field access through a tuple
                //     element).  Without this branch, `0.x` was
                //     greedily consumed as a malformed float literal
                //     and rejected with "Problem parsing float".
                // Either way: emit the leading integer, queue the `.`
                // so the next `cont()` returns it as a separator, and
                // the following token (digit, identifier, or whatever)
                // re-lexes fresh.
                self.link = self.memory.len();
                self.memory_state.push(self.scan_state());
                self.memory.push(LexResult::new(
                    LexItem::Token(".".to_string()),
                    self.position.clone(),
                ));
                // Through `ret_number` for the same reason as the `..` site above: one
                // home for what an integer literal's token is.  A tuple index this large is
                // out of range for every tuple, and the three projection sites read it with
                // `has_long`, which matches both widths — so it reaches the out-of-range
                // refusal instead of being silently wrapped into a small index.
                return if let Ok(r) = val.parse::<u64>() {
                    self.ret_number(r, pos, val.starts_with('0'))
                } else {
                    self.err(
                        Level::Error,
                        "Integer literal out of range (exceeds i64::MAX)",
                    );
                    self.ret_number(0, pos, false)
                };
            }
            val.push('.');
            f = true;
            let (part, _) = self.get_number();
            if part.is_empty() {
                self.err(Level::Error, "Problem parsing float");
                return Lexer::none();
            }
            val += &part;
        }
        // Exponent: accept both `e` and `E`, and a `+` or `-` sign — the full
        // JSON/IEEE form (`1E5`, `1e+5`).  Harmless for normal loft (no `<digit>E`
        // literal exists today; hex `0x1E` is consumed in get_number before here).
        // The `val` string is normalised to lowercase `e` for the f64/f32 parse.
        if let Some('e' | 'E') = self.iter.peek() {
            f = true;
            val.push('e');
            self.next_char();
            if let Some(sign @ ('-' | '+')) = self.iter.peek().copied() {
                self.next_char();
                val.push(sign);
            }
            let (exp, _) = self.get_number();
            if exp.is_empty() {
                self.err(Level::Error, "Problem parsing float");
                return Lexer::none();
            }
            val += &exp;
        }
        if f {
            // loft#1146 — `f` is the ONLY suffix a decimal literal takes (LOFT.md § Literals),
            // so any other letter glued to one is a suffix the author guessed.  Left to the
            // parser it became "Expect token ;", which names nothing: the `s` was lexed as a
            // separate identifier and the reader was told a semicolon was missing.  Say what
            // the suffix set is, here, where the two characters are still adjacent.
            if let Some(&c) = self.iter.peek()
                && c.is_ascii_alphabetic()
                && c != 'f'
            {
                let mut bad = String::new();
                while let Some(&n) = self.iter.peek() {
                    if n.is_ascii_alphanumeric() || n == '_' {
                        bad.push(n);
                        self.next_char();
                    } else {
                        break;
                    }
                }
                // The run is CONSUMED, so the reader gets one diagnostic rather than this
                // one plus the `Expect token ;` the leftover identifier used to earn.
                self.err(
                    Level::Error,
                    &format!(
                        "`{bad}` is not a numeric suffix; write `f` for a `single` literal (`1.5f`) — a bare decimal literal is a `float`"
                    ),
                );
            }
            if let Some('f') = self.iter.peek() {
                self.next_char();
                if let Ok(r) = val.parse::<f32>() {
                    LexResult::new(LexItem::Single(r), pos)
                } else {
                    self.err(Level::Error, "Problem parsing single float");
                    LexResult::new(LexItem::Single(0.0), pos)
                }
            } else if let Ok(r) = val.parse::<f64>() {
                LexResult::new(LexItem::Float(r, val.starts_with('0')), pos)
            } else {
                self.err(Level::Error, "Problem parsing float");
                LexResult::new(LexItem::Float(0.0, val.starts_with('0')), pos)
            }
        } else if let Some(short) = val.strip_prefix("0x") {
            let res = if let Some(r) = hex_parse(short) {
                r
            } else {
                self.err(Level::Error, "Problem parsing hex number");
                0
            };
            self.ret_number(res, pos, false)
        } else if let Some(short) = val.strip_prefix("0b") {
            let res = if let Some(r) = bin_parse(short) {
                r
            } else {
                self.err(Level::Error, "Problem parsing binary number");
                0
            };
            self.ret_number(res, pos, false)
        } else if let Some(short) = val.strip_prefix("0o") {
            let res = if let Some(r) = oct_parse(short) {
                r
            } else {
                self.err(Level::Error, "Problem parsing octal number");
                0
            };
            self.ret_number(res, pos, false)
        } else if let Ok(r) = val.parse::<u64>() {
            self.ret_number(r, pos, val.starts_with('0'))
        } else {
            self.err(Level::Error, "Problem parsing number");
            self.ret_number(0, pos, false)
        }
    }

    fn ret_number(&mut self, r: u64, p: Position, start_zero: bool) -> LexResult {
        // Post-2c round 10c: `integer` is 8 bytes.  Values > i64::MAX
        // (impossible to represent in i64) are rejected; values up to
        // i32::MAX are emitted as LexItem::Integer, larger values as
        // LexItem::Long so the parser carries the full i64 payload
        // through to `Value::Long`.  Both land in a wide Type::Integer
        // at the parser layer — the distinction is only about how
        // many bytes of bytecode the literal consumes.
        let i32_max = u64::from(i32::MAX as u32);
        let i64_max = i64::MAX as u64;
        if r > i64_max {
            self.err(
                Level::Error,
                "Integer literal out of range (exceeds i64::MAX)",
            );
            LexResult::new(LexItem::Integer(0, start_zero), p)
        } else if r > i32_max {
            LexResult::new(LexItem::Long(r), p)
        } else {
            LexResult::new(LexItem::Integer(r as u32, start_zero), p)
        }
    }

    pub fn parse_string(&mut self, string: &str, filename: &str) {
        self.virtual_files
            .insert(filename.to_string(), string.to_string());
        let mut v = Vec::new();
        for l in string.split('\n') {
            v.push(Ok(String::from(l)));
        }
        self.lines = Box::new(v.into_iter());
        self.restart(filename);
    }

    /// The text of an in-memory source registered by [`Self::parse_string`], or `None` for a
    /// file that lives on disk.
    #[must_use]
    pub fn virtual_source(&self, filename: &str) -> Option<&str> {
        self.virtual_files.get(filename).map(String::as_str)
    }

    pub fn switch(&mut self, filename: &str) {
        // An in-memory source registered by `parse_string` re-serves from
        // memory — its name is not an openable path.
        if let Some(content) = self.virtual_files.get(filename) {
            let v: Vec<IoResult<String>> =
                content.split('\n').map(|l| Ok(String::from(l))).collect();
            self.lines = Box::new(v.into_iter());
            self.restart(filename);
            return;
        }
        // try VIRT_FS first (WASM has no real filesystem for library files).
        #[cfg(feature = "wasm")]
        if let Some(content) = crate::wasm::virt_fs_get(filename) {
            self.lines = Box::new(
                content
                    .lines()
                    .map(|l| Ok(l.to_string()))
                    .collect::<Vec<_>>()
                    .into_iter(),
            );
            self.restart(filename);
            return;
        }
        let Ok(fp) = File::open(filename) else {
            // Mistyping the path is one of the commonest FIRST things anyone does
            // (`loft examples/helo.loft`), so answer it the way a mistyped
            // function or type is answered: name the file and offer the nearest
            // sibling.  The old text — `Unknown file:<path>`, no space, no
            // suggestion — made a one-character slip look like a broken install.
            let msg = suggest_sibling_file(filename).map_or_else(
                || format!("no such file: {filename}"),
                |s| format!("no such file: {filename} — did you mean '{s}'?"),
            );
            self.diagnostics.add(Level::Fatal, &msg);
            return;
        };
        self.lines = Box::new(BufReader::new(fp).lines());
        self.restart(filename);
    }

    fn restart(&mut self, filename: &str) {
        self.position = Position {
            file: filename.to_string(),
            line: 0,
            pos: 0,
        };
        // A pending reporting seek belongs to the file being left: restoring it after
        // the switch would stamp the OLD file's line onto the new one's first token.
        self.seek_return = None;
        // Likewise the consumed-source position: `report_pos` also checks the file, so
        // this is the second of two independent guards rather than the only one.
        self.prev_end = self.position.clone();
        self.peek = LexResult {
            has: LexItem::None,
            position: self.position.clone(),
        };
        self.memory.clear();
        self.memory_state.clear();
        self.link = 0;
        self.links = Rc::new(RefCell::new(0));
        self.iter = LINE.chars().collect::<Vec<_>>().into_iter().peekable();
        self.mode = Mode::Code;
        self.cont();
    }

    fn err(&mut self, level: Level, error: &str) {
        diagnostic!(self, level, "{error}");
    }

    /// Like [`err`], but carries a stable diagnostic `code` (@PLN102 arc-E E1).
    fn err_coded(&mut self, level: Level, code: &'static str, error: &str) {
        diagnostic!(self, level, code = code, "{error}");
    }

    /// A literal `}` where a format string expects a hole to close.
    ///
    /// The four string scanners (plain, nested, and the two backtick forms) all reach this
    /// same conclusion, and @PLN131 gives it a fix — so it gets ONE home rather than four
    /// copies that can drift apart in either half.  The rewrite is fully determined by the
    /// code: a literal brace is spelled `}}` and nothing else, which is what makes it the
    /// one fix here that is `Mechanical` with a real `edit`.
    fn unescaped_brace(&mut self) {
        self.err_coded(
            Level::Error,
            "format-unescaped-brace",
            "a literal `}` in a format string — `}` closes an interpolation hole, and none is open",
        );
        // The scanners all consume the `}` before reporting, so the diagnostic sits ONE
        // column past it and the brace to replace is at `col - 1`. Placeable because that
        // offset is a property of this code path rather than of the input — which is what
        // makes this the one fix an applier can run unattended today (@PLN131 step 4).
        let (line, col) = (self.position.line, self.position.pos);
        self.fix_last(Fix {
            kind: FixKind::Mechanical,
            title: "double the brace".to_string(),
            condition: None,
            edit: Some(crate::diagnostics::Edit {
                line,
                col: col.saturating_sub(1).max(1),
                len: 1,
                text: "}}".to_string(),
            }),
            concept: "interpolation",
            concept_ref: "@F35",
        });
    }

    /// A literal `{` where a format string expects a hole to open.
    ///
    /// The mirror of [`unescaped_brace`](Self::unescaped_brace), and it earns the same
    /// shape: one home for four scanners, one code, one mechanical fix.  Until it
    /// existed the two halves of the same mistake got very different answers — `}` a
    /// coded error naming `}}`, `{` a bare "Formatter error" raised four diagnostics
    /// later by a parser that could not know a `{` started it (loft#989).
    fn unclosed_hole(&mut self) {
        // The scanners consume the brace before reporting, so the cursor sits ONE column
        // past the `{`.  Point the caret AT it instead of one past: the whole message is
        // about that character, and a reader following the caret to the space beside it
        // has to guess which of the two the compiler meant.
        let mut at = self.position.clone();
        at.pos = at.pos.saturating_sub(1).max(1);
        diagnostic_at!(
            self,
            &at,
            Level::Error,
            code = "format-unclosed-hole",
            "a literal `{{` in a format string — `{{` opens an interpolation hole, and nothing closes it"
        );
        self.fix_last(Fix {
            kind: FixKind::Mechanical,
            title: "double the brace".to_string(),
            condition: None,
            edit: Some(crate::diagnostics::Edit {
                line: at.line,
                col: at.pos,
                len: 1,
                text: "{{".to_string(),
            }),
            concept: "interpolation",
            concept_ref: "@F35",
        });
    }

    /// Can the hole opened by the `{` just consumed still close on this line?
    ///
    /// A hole holds CODE, and the code scanner stops at the end of a line — so a hole
    /// that does not close on the line it opened never closes at all (`"a {` + newline
    /// is an error today, in a backtick literal as much as a quoted one).  That bound
    /// is what turns a same-line scan into a DECISION rather than a guess, and it is
    /// the whole reason the caller may speak: without it the missing `}` surfaces
    /// several diagnostics later, at a position that says nothing about the `{`.
    ///
    /// Answers `true` — stay silent — for the one thing it does not model, a `//`
    /// comment.  The direction is the point.  A wrong `false` would refuse a legal
    /// program; a wrong `true` only leaves the pre-loft#989 behaviour in place.
    fn hole_closes_on_this_line(&self) -> bool {
        /// What the scan is inside.  A hole nested in a string literal pushes `Code`
        /// again, which is why this is a stack and not a flag.
        #[derive(Clone, Copy, PartialEq)]
        enum Ctx {
            Code,
            Str,
            Backtick,
            Char,
        }
        // The `{` is already consumed, so the hole itself is the first frame.
        let mut stack = vec![Ctx::Code];
        let mut it = self.iter.clone();
        while let Some(c) = it.next() {
            // An escape hides whatever follows it.  That is also what makes the
            // Rust-macro spelling `{\"inner\"}` scan clean: both delimiters are
            // hidden, so the hole sees only the identifier between them.
            if c == '\\' {
                it.next();
                continue;
            }
            match stack.last().copied().unwrap_or(Ctx::Code) {
                Ctx::Code => match c {
                    // The rest of the line is a comment, so nothing on it can close the
                    // hole — but that is a shape nobody writes, and reporting it would
                    // spend the risk budget on a case that does not occur.
                    '/' if it.peek() == Some(&'/') => return true,
                    '{' => stack.push(Ctx::Code),
                    '}' => {
                        stack.pop();
                        if stack.is_empty() {
                            return true;
                        }
                    }
                    '"' => stack.push(Ctx::Str),
                    // A `` ` `` inside a hole can only OPEN a literal: code has no bare
                    // closing backtick.  If that literal runs past the end of the line
                    // the hole cannot close on it either, and running past the line is
                    // exactly what the enclosing literal's own terminator looks like
                    // from in here — both readings are an unclosed hole.
                    '`' => stack.push(Ctx::Backtick),
                    '\'' => stack.push(Ctx::Char),
                    _ => {}
                },
                Ctx::Str | Ctx::Backtick => match c {
                    '"' if stack.last() == Some(&Ctx::Str) => {
                        stack.pop();
                    }
                    '`' if stack.last() == Some(&Ctx::Backtick) => {
                        stack.pop();
                    }
                    '{' | '}' if it.peek() == Some(&c) => {
                        it.next();
                    }
                    '{' => stack.push(Ctx::Code),
                    _ => {}
                },
                Ctx::Char => {
                    if c == '\'' {
                        stack.pop();
                    }
                }
            }
        }
        false
    }

    /// Debug feature to check the amount of currently in use links
    pub fn count_links(&self) -> u32 {
        *self.links.borrow()
    }

    /// Return the currently found lexer element.
    pub fn peek(&self) -> LexResult {
        self.peek.clone()
    }

    pub fn peek_token(&self, token: &str) -> bool {
        self.peek.has == LexItem::Token(token.to_string())
    }

    fn end(&mut self) {
        self.peek = LexResult {
            has: LexItem::None,
            position: self.position.clone(),
        }
    }

    /// Continue the lexer to the next step.
    pub fn cont(&mut self) {
        // Are we at the live edge (scanning fresh) rather than replaying the memory
        // buffer?  Capture it BEFORE `next()`, because `next()` bumps `self.link` when
        // it replays — so testing `link == memory.len()` afterwards cannot tell a
        // freshly-scanned token (append it to the buffer) apart from a replay of the
        // LAST buffered token (already in the buffer).  Conflating them re-appended a
        // duplicate, so two look-aheads that start at the same position desynced the
        // real parse (guard `link_revert_repeatable_same_region`).
        let at_edge = self.link == self.memory.len();
        // Where the source the parser has consumed stops, captured before the cursor
        // runs on to the next token — see `report_pos`.
        self.prev_end = self.position.clone();
        let Some(n) = self.next() else {
            self.end();
            return;
        };
        let mut res = n;
        while res.has == LexItem::Token(self.comment.clone()) {
            while self.iter.peek().is_some() {
                self.iter.next();
            }
            let Some(n) = self.next() else {
                self.end();
                return;
            };
            res = n;
        }
        // Remember a token freshly scanned at the edge.  A replay (`at_edge` was false)
        // is already in the buffer and must not be appended twice — two look-aheads
        // starting at the same position desynced the real parse when it was
        // (`link_revert_repeatable_same_region`).
        if at_edge && self.link == self.memory.len() {
            if self.count_links() > 0 {
                self.memory.push(res.clone());
                self.memory_state.push(self.scan_state());
                self.link += 1;
            } else {
                self.memory.clear();
                self.memory_state.clear();
                self.link = 0;
            }
        } else if at_edge && self.link < self.memory.len() && self.count_links() > 0 {
            // A mid-scan QUEUE: reading `1..` or `n.v.0.0`, the number lexer emits two
            // tokens from one scan — it pushes the follow-up (`..` / `.`) into the buffer
            // and returns the NUMBER as the live token.  So the number is the one token
            // the buffer does not hold, and a revert to any point before it replays the
            // follow-up alone: `i + 1..` came back as `i`, `+`, `..`, and the `+` was left
            // with one operand (loft#1164).
            //
            // Insert it BEFORE the queued token and step over it, which leaves the live
            // sequence unchanged — the next `cont()` still replays the follow-up — and
            // makes the buffer say what was actually read.
            self.memory.insert(self.link, res.clone());
            self.memory_state.insert(self.link, self.scan_state());
            self.link += 1;
        }
        self.peek = res;
    }

    /// Create a link to the current lexer position, it can be used to revert to
    /// this position later.
    pub fn link(&mut self) -> Link {
        let cur: u32 = *self.links.borrow();
        self.links.replace(cur + 1);
        if self.memory.is_empty() {
            self.memory.push(self.peek.clone());
            self.memory_state.push(self.scan_state());
            self.link += 1;
        }
        Link {
            links: Rc::clone(&self.links),
            pos: self.link - 1,
        }
    }

    /// Reset to a previously made link position in the source.
    pub fn revert(&mut self, link: Link) {
        lex_trace(format_args!(
            "revert link {} -> {} (cursor {}:{})",
            self.link, link.pos, self.position.line, self.position.pos
        ));
        self.link = link.pos;
        drop(link);
        self.cont();
    }

    pub fn token(&mut self, token: &'static str) -> bool {
        if self.has_token(token) {
            true
        } else {
            diagnostic!(self, Level::Error, "Expect token {token}");
            false
        }
    }

    /// L1: skip tokens until one of `targets` is reached (or end-of-input).
    /// Nested `{...}`, `(...)`, `[...]` are matched and skipped as units so
    /// that a target inside a nested group does not incorrectly terminate
    /// recovery.  The target token itself is NOT consumed — the caller may
    /// decide whether to consume it or treat it as the recovered boundary.
    ///
    /// Use after a failed `token()` call in contexts where the parser would
    /// otherwise produce a cascade of confusing diagnostics.  Returns `true`
    /// if a target was found, `false` on EOF.
    ///
    /// Example: after `Expect token )` in a function-argument list, call
    /// `recover_to(&[")", "{", ";"])` to jump to the nearest plausible
    /// resynchronisation point.
    pub fn recover_to(&mut self, targets: &[&str]) -> bool {
        let mut depth: i32 = 0;
        let mut bc_throttle: u32 = 0;
        loop {
            // @PLAN49 T1 breadcrumb — refresh every 256 iterations so a
            // hang in this recovery loop tells T1 *where* we got stuck
            // when it hard-kills.  Throttled so the mutex-try in
            // `checkpoint_parse` isn't on every token.
            bc_throttle = bc_throttle.wrapping_add(1);
            if bc_throttle.is_multiple_of(256) {
                crate::timeout::checkpoint_parse(&self.peek.position.file, self.peek.position.line);
            }
            if matches!(self.peek.has, LexItem::None) {
                return false;
            }
            if depth == 0 {
                for t in targets {
                    if self.peek_token(t) {
                        return true;
                    }
                }
            }
            if self.peek_token("{") || self.peek_token("(") || self.peek_token("[") {
                depth += 1;
            } else if self.peek_token("}") || self.peek_token(")") || self.peek_token("]") {
                if depth == 0 {
                    // An unmatched closer at the outer level is also a
                    // valid resynchronisation point — do not consume it.
                    return false;
                }
                depth -= 1;
            }
            self.cont();
        }
    }

    /// Is the parenthesised group just opened a TUPLE literal — does a top-level `,`
    /// reach the caller before the group's own `)`?
    ///
    /// The caller needs the answer BEFORE it parses member 0, which is the only member
    /// parsed before a `,` has proved anything: a tuple MEMBER is not the enclosing
    /// assignment's whole value, so it must not adopt that assignment's destination
    /// variable as its build accumulator the way a parenthesised expression legitimately
    /// does.
    ///
    /// One thing stops the walk short of the closer, and answers `false`: a depth-0 `;`, or
    /// end of input — inside parentheses that means the source is malformed, and stopping
    /// keeps the scan STATEMENT-LOCAL.  That bound is not only about speed:
    /// [`revert`](Self::revert) restores the token STREAM and not the reporting cursor (a
    /// replayed token deliberately leaves `position` where the scan reached), so however far
    /// this walks is how far a diagnostic raised inside the replayed region has its caret
    /// pushed.  Unbounded, an unclosed `(` moved its caret from the offending line to the end
    /// of the function.
    ///
    /// A string literal WITHOUT an interpolation hole is crossed like any other token: a comma
    /// or bracket inside it is part of the string's one token.  Stopping at every string, as
    /// the walk did, answered `false` for a group whose member 0 held a string ANYWHERE —
    /// `(["a"], 1)`, a struct literal with a text field — and member 0 then adopted the
    /// assignment's destination as its build accumulator, which typed the local as the member
    /// and refused a legal program.
    ///
    /// A string that OPENS a hole still stops it, answering `false`.  The hole's closing `}`
    /// resumes the string only once the parser has read the hole's expression and switched
    /// the lexer back ([`set_mode`](Self::set_mode)), and a spec after a `:` is read in
    /// `Formatting` mode, where whitespace is a token; walked without the parser, that `}` is
    /// an ordinary token, the rest of the string is scanned as code, and the tokens a revert
    /// then replays are the mis-scanned ones.  Neither answer is free there: `false` refuses
    /// `t = (["{x}"], 1)` (the destination is typed as the member), and `true` refuses
    /// `v = (["{x}"] + w)`, a parenthesised concatenation that needs the destination as its
    /// accumulator.  `false` is the answer this walk always gave, so the vector literal in
    /// member 0 holding an interpolated string is the shape left open (loft#1590).
    ///
    /// Deliberately not [`recover_to`](Self::recover_to), whose depth walk this otherwise
    /// mirrors: recovery may cross anything, and this stops at the statement's end.
    pub fn peek_tuple_literal(&mut self) -> bool {
        let saved = self.link();
        let mut depth: i32 = 0;
        let mut found = false;
        loop {
            if matches!(self.peek.has, LexItem::None) {
                break;
            }
            // `in_format_expr` is the scanner state of the token in hand, fresh or replayed:
            // a string that just opened a hole.
            if matches!(self.peek.has, LexItem::CString(_)) && self.in_format_expr {
                break;
            }
            if depth == 0 {
                if self.peek_token(",") {
                    found = true;
                    break;
                }
                if self.peek_token(";") {
                    break;
                }
            }
            if self.peek_token("(") || self.peek_token("[") || self.peek_token("{") {
                depth += 1;
            } else if self.peek_token(")") || self.peek_token("]") || self.peek_token("}") {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            self.cont();
        }
        self.revert(saved);
        found
    }

    /// Shorthand test if the current element is a specific token and skip it if found.
    pub fn has_token(&mut self, token: &'static str) -> bool {
        if self.peek_token(token) {
            self.cont();
            true
        } else {
            false
        }
    }

    /// Like `has_token(">")` but also splits `>>` into `>` + `>` so that
    /// nested generic types like `vector<vector<T>>` parse correctly.
    pub fn has_closing_angle(&mut self) -> bool {
        if self.peek_token(">") {
            self.cont();
            true
        } else if let LexItem::Token(ref t) = self.peek.has
            && t.starts_with('>')
            && t.len() > 1
        {
            let rest = t[1..].to_string();
            self.peek.has = LexItem::Token(rest);
            true
        } else {
            false
        }
    }

    /// Expect a closing `>` for generic types.  Like `token(">")` but handles
    /// `>>` splitting for nested generics.
    pub fn closing_angle(&mut self) -> bool {
        if self.has_closing_angle() {
            true
        } else {
            diagnostic!(self, Level::Error, "Expect token >");
            false
        }
    }

    /// Shorthand test if the current element is a specific local keyword, so not one of the reserved
    pub fn has_keyword(&mut self, keyword: &'static str) -> bool {
        if self.peek.has == LexItem::Identifier(keyword.to_string()) {
            self.cont();
            true
        } else {
            false
        }
    }

    /// Shorthand test if the current element is a number and skip it if found.
    pub fn has_integer(&mut self) -> Option<u32> {
        if let LexItem::Integer(n, _) = self.peek().has {
            self.cont();
            Some(n)
        } else {
            None
        }
    }

    /// Shorthand test if the current element is a number and skip it if found.
    pub fn has_long(&mut self) -> Option<u64> {
        if let LexItem::Long(n) = self.peek().has {
            self.cont();
            Some(n)
        } else if let LexItem::Integer(n, _zero) = self.peek().has {
            self.cont();
            Some(u64::from(n))
        } else {
            None
        }
    }

    pub fn has_char(&mut self) -> Option<u32> {
        if let LexItem::Character(c) = self.peek().has {
            self.cont();
            Some(c)
        } else {
            None
        }
    }

    /// Shorthand test if the current element is a constant string and skip it if found.
    pub fn has_cstring(&mut self) -> Option<String> {
        if let LexItem::CString(n) = self.peek().has {
            self.cont();
            Some(n)
        } else {
            None
        }
    }

    /// Shorthand test if the current element is a float and skip it if found.
    pub fn has_float(&mut self) -> Option<f64> {
        if let LexItem::Float(n, _) = self.peek().has {
            self.cont();
            Some(n)
        } else {
            None
        }
    }

    /// Shorthand test if the current element is a float and skip it if found.
    pub fn has_single(&mut self) -> Option<f32> {
        if let LexItem::Single(n) = self.peek().has {
            self.cont();
            Some(n)
        } else {
            None
        }
    }

    // @F17 — named-argument detection (two-token lookahead)
    /// Peek two tokens ahead to detect `identifier :` (named argument syntax).
    /// Returns `Some(name)` if the pattern matches, without consuming any tokens.
    /// Returns `None` if the current token is not an identifier followed by `:`.
    pub fn peek_named_arg(&mut self) -> Option<String> {
        if let LexItem::Identifier(ref name) = self.peek.has {
            let name = name.clone();
            let saved = self.link();
            self.cont(); // consume identifier
            let is_colon = self.peek_token(":");
            let is_double_colon = self.peek_token("::");
            self.revert(saved); // restore to before identifier
            if is_colon && !is_double_colon {
                return Some(name);
            }
        }
        None
    }

    /// Shorthand test if the current element is an identifier and skip it if found.
    pub fn has_identifier(&mut self) -> Option<String> {
        if let LexItem::Identifier(n) = self.peek().has {
            self.cont();
            Some(n)
        } else {
            None
        }
    }

    /// Consume an identifier and return it together with its start position.
    /// Diagnostics about the named entity (an unknown type / variable) must
    /// point here; after consumption the cursor drifts to the next token.
    pub fn has_identifier_pos(&mut self) -> Option<(String, Position)> {
        if let LexItem::Identifier(n) = self.peek().has {
            let pos = self.peek.position.clone();
            lex_trace(format_args!(
                "idpos {n:?} @ {}:{} (cursor {}:{})",
                pos.line, pos.pos, self.position.line, self.position.pos
            ));
            self.cont();
            Some((n, pos))
        } else {
            None
        }
    }

    /// Create a lexer from a static string
    #[allow(unused)]
    pub fn from_str(s: &str, filename: &str) -> Lexer {
        Self::from_str_with(s, filename, LexConfig::default())
    }

    /// Like [`from_str`](Self::from_str) but with an explicit lexicon, so the SAME
    /// lexer tokenises a non-loft surface syntax (e.g. `loft.toml`).  The lexicon
    /// is set BEFORE the first token is primed, so it applies from the first char.
    pub fn from_str_with(s: &str, filename: &str, config: LexConfig) -> Lexer {
        let mut v = Vec::new();
        for l in s.split('\n') {
            v.push(Ok(String::from(l)));
        }
        let mut res = Lexer::new_with(v.into_iter(), filename, config);
        res.cont();
        res
    }
}

#[cfg(test)]
mod test {
    fn test_id(lexer: &Lexer, id: &str) {
        assert_eq!(lexer.peek().has, LexItem::Identifier(String::from(id)));
    }

    fn links(lexer: &Lexer, nr: u32) {
        assert_eq!(lexer.count_links(), nr);
    }

    fn array(lexer: &mut Lexer) -> Vec<LexItem> {
        let mut rest = Vec::new();
        rest.push(lexer.peek().has);
        while let Some(res) = lexer.next() {
            rest.push(res.has);
        }
        rest
    }

    use super::*;
    fn validate(s: &'static str, data: &[LexItem]) {
        let res = array(&mut Lexer::from_str(s, "validate"));
        assert_eq!(res, data);
    }

    /// Lex a single `"…"` literal in JSON mode ([`LexConfig::json`]) and return the
    /// decoded [`LexItem::CString`] content.
    #[cfg(test)]
    fn json_str(s: &str) -> String {
        let l = Lexer::from_str_with(s, "json", LexConfig::json());
        match l.peek().has {
            LexItem::CString(c) => c,
            other => panic!("expected CString, got {other:?}"),
        }
    }

    /// Lex a single `"…"` literal in normal loft mode and return the decoded string.
    #[cfg(test)]
    fn loft_str(s: &str) -> String {
        let l = Lexer::from_str(s, "loft");
        match l.peek().has {
            LexItem::CString(c) => c,
            other => panic!("expected CString, got {other:?}"),
        }
    }

    /// The first [`LexItem`] of `s` lexed in JSON mode.
    #[cfg(test)]
    fn json_first(s: &str) -> LexItem {
        Lexer::from_str_with(s, "json", LexConfig::json())
            .peek()
            .has
    }

    /// @PLN109 Phase 1c — loft's lexer already distinguishes integer-shaped from
    /// fractional numbers (the basis for Phase 2's `Parsed::Int` vs
    /// `Parsed::Number` mapping).  Load-bearing for H5: a 16-digit integer must
    /// reach the parser as an exact `Long(u64)`, NOT rounded through f64.
    #[test]
    fn json_number_classification() {
        assert_eq!(json_first("42"), LexItem::Integer(42, false));
        // H5: 2^53 + 1 preserved exactly as Long — the whole point of the arc.
        assert_eq!(
            json_first("9007199254740993"),
            LexItem::Long(9_007_199_254_740_993)
        );
        assert!(matches!(json_first("3.14"), LexItem::Float(..)));
        // Exponent-bearing numbers are Float (1b): `1e3` / `1E5` are not i64.
        assert!(matches!(json_first("1e3"), LexItem::Float(..)));
        assert!(matches!(json_first("1E5"), LexItem::Float(..)));
    }

    /// @PLN109 Phase 1a — JSON string escapes decode in JSON mode: `\/` and
    /// `\uXXXX` (four hex, no braces) with surrogate-pair combining.  The `\\uXXXX`
    /// in each Rust literal is the two chars backslash-u reaching the lexer (the
    /// JSON source text); the expected side is the decoded scalar.
    #[test]
    fn json_string_escapes() {
        assert_eq!(json_str("\"a\\/b\""), "a/b"); // \/ -> /
        assert_eq!(json_str("\"\\u0041\""), "A"); // A -> A
        assert_eq!(json_str("\"\\u00e9\""), "é"); // é -> é
        assert_eq!(json_str("\"\\u2764\""), "❤"); // ❤ -> ❤ (BMP)
        assert_eq!(json_str("\"\\uD83D\\uDE00\""), "😀"); // surrogate pair -> U+1F600
        assert_eq!(json_str("\"\\uD834\\uDD1E\""), "𝄞"); // astral pair -> U+1D11E
        assert_eq!(json_str("\"a\\u0041b\\/c\""), "aAb/c"); // mixed in one string
        // loft's own escapes still work in JSON mode (lenient superset).
        assert_eq!(json_str("\"x\\ty\""), "x\ty");
    }

    /// @PLN109 Phase 1a — normal loft string lexing is UNCHANGED: `\u{…}` braces
    /// form still decodes, `\t` still works, and the JSON-only escapes are NOT
    /// silently accepted (they still route to loft's error path, not `/` / a char).
    #[test]
    fn loft_string_escapes_unchanged() {
        assert_eq!(loft_str("\"\\u{41}\""), "A"); // braces form still works
        assert_eq!(loft_str("\"a\\tb\""), "a\tb"); // \t
        // `\/` and `\uXXXX` are NOT decoded in loft mode — bare `\u` and unknown
        // `\/` route to the error path (emit `?`), so they do NOT decode.
        assert_ne!(loft_str("\"a\\/b\""), "a/b");
        assert_ne!(loft_str("\"\\u0041\""), "A");
    }

    #[cfg(test)]
    fn error(s: &'static str, err: &'static str) {
        let mut l = Lexer::from_str(s, "error");
        l.cont();
        assert_eq!(format!("{:?}", l.diagnostics), err.to_string());
    }

    #[cfg(test)]
    fn tokens(s: &'static str, t: &'static [&'static str]) {
        let mut data: Vec<LexItem> = Vec::new();
        for s in t {
            if s.chars().next().unwrap().is_ascii_digit() {
                if let Ok(res) = s.parse::<u32>() {
                    data.push(LexItem::Integer(res, false));
                } else {
                    panic!("Cannot parse {s}");
                }
            } else if KEYWORDS.contains(s) || TOKENS.contains(s) {
                data.push(LexItem::Token((*s).to_string()));
            } else {
                data.push(LexItem::Identifier((*s).to_string()));
            }
        }
        assert_eq!(array(&mut Lexer::from_str(s, "tokens")), data);
    }

    #[test]
    fn stray_unrecognized_char_errors_and_advances() {
        // Regression #434 — an unrecognized character (a stray '\' in code) must
        // produce an error AND advance past it.  The old code emitted an empty
        // Identifier("") without consuming the char, so every cont() re-read the
        // same position forever — an infinite parse loop / hang, not a clean error.
        let mut l = Lexer::from_str("5 \\ 2", "stray");
        // Drain with a hard cap well above the ~4 real tokens: if the lexer fails to
        // advance, the stream never reaches `None` and only the cap stops it — that
        // overrun IS the bug this guards against.
        let mut steps = 0;
        while l.peek().has != LexItem::None && steps < 50 {
            l.cont();
            steps += 1;
        }
        assert!(
            steps < 50,
            "lexer did not advance past a stray '\\' — it hangs"
        );
        assert!(
            format!("{:?}", l.diagnostics).contains("Unexpected character"),
            "a stray '\\' must report an 'Unexpected character' error"
        );
    }

    #[test]
    fn test_lexer() {
        validate("1234", &[LexItem::Integer(1234, false)]);
        validate("0xaf", &[LexItem::Integer(0xaf, false)]);
        validate("1e2", &[LexItem::Float(100.0, false)]);
        validate(
            "1..4",
            &[
                LexItem::Integer(1, false),
                LexItem::Token("..".to_string()),
                LexItem::Integer(4, false),
            ],
        );
        tokens("=1+2", &["=", "1", "+", "2"]);
        tokens("=if 1 in a", &["=", "if", "1", "in", "a"]);
    }

    /// Drive the lexer through `cont()` (the API the parser uses) so
    /// `self.peek` stays current for context-dependent rules like
    /// P195's "previous token was `.`".  The plain `array()` helper
    /// calls `next()` directly and leaves `peek` stale, which is fine
    /// for whitespace-insensitive tokens but not for context-aware
    /// ones.
    fn cont_array(lexer: &mut Lexer) -> Vec<LexItem> {
        let mut rest = Vec::new();
        while !matches!(lexer.peek().has, LexItem::None) {
            rest.push(lexer.peek().has);
            lexer.cont();
        }
        rest
    }

    fn validate_cont(s: &'static str, data: &[LexItem]) {
        let mut lex = Lexer::from_str(s, "validate_cont");
        let res = cont_array(&mut lex);
        assert_eq!(res, data);
    }

    /// P195: when the previous token is `.` (field access), a digit
    /// followed by `.<digit>` must lex as integer + `.` + integer
    /// (chained tuple-index access), not as a single float literal.
    #[test]
    fn p195_chained_tuple_index_does_not_glue_into_float() {
        // n.v.0.0 — the inner `0.0` is two tuple indices, not a float.
        validate_cont(
            "n.v.0.0",
            &[
                LexItem::Identifier("n".to_string()),
                LexItem::Token(".".to_string()),
                LexItem::Identifier("v".to_string()),
                LexItem::Token(".".to_string()),
                LexItem::Integer(0, true),
                LexItem::Token(".".to_string()),
                LexItem::Integer(0, true),
            ],
        );
        // Stand-alone float still works at expression position.
        validate_cont("0.0", &[LexItem::Float(0.0, true)]);
        validate_cont(
            "x = 0.0",
            &[
                LexItem::Identifier("x".to_string()),
                LexItem::Token("=".to_string()),
                LexItem::Float(0.0, true),
            ],
        );
        // Mixed: float at expression position, integer after `.`.
        validate_cont(
            "1.5 + p.0",
            &[
                LexItem::Float(1.5, false),
                LexItem::Token("+".to_string()),
                LexItem::Identifier("p".to_string()),
                LexItem::Token(".".to_string()),
                LexItem::Integer(0, true),
            ],
        );
        // Non-leading-zero too: `t.1.2.3`.
        validate_cont(
            "t.1.2.3",
            &[
                LexItem::Identifier("t".to_string()),
                LexItem::Token(".".to_string()),
                LexItem::Integer(1, false),
                LexItem::Token(".".to_string()),
                LexItem::Integer(2, false),
                LexItem::Token(".".to_string()),
                LexItem::Integer(3, false),
            ],
        );
        // Range still wins over field access: `0..5` is range, not
        // tuple index `.5`.
        validate_cont(
            "0..5",
            &[
                LexItem::Integer(0, true),
                LexItem::Token("..".to_string()),
                LexItem::Integer(5, false),
            ],
        );
    }

    /// P234: when the previous token is `.`, an integer followed by
    /// `.<ident>` must lex as integer + `.` + ident (struct field
    /// access through a tuple element), not as a malformed float.
    /// Pre-fix the lexer raised "Problem parsing float" because `0.x`
    /// matched the float-fraction grammar then failed at the
    /// non-digit `x`.  P195 already handled the digit-after case
    /// (`0.0`); P234 extends it to the identifier-after case (`0.x`).
    #[test]
    fn p234_tuple_index_then_field_does_not_glue_into_float() {
        // r.0.x — `r` ident, `.` field, `0` tuple index, `.` field, `x` ident.
        validate_cont(
            "r.0.x",
            &[
                LexItem::Identifier("r".to_string()),
                LexItem::Token(".".to_string()),
                LexItem::Integer(0, true),
                LexItem::Token(".".to_string()),
                LexItem::Identifier("x".to_string()),
            ],
        );
        // p.1.field — non-leading-zero too.
        validate_cont(
            "p.1.field",
            &[
                LexItem::Identifier("p".to_string()),
                LexItem::Token(".".to_string()),
                LexItem::Integer(1, false),
                LexItem::Token(".".to_string()),
                LexItem::Identifier("field".to_string()),
            ],
        );
    }

    #[test]
    fn lexer_errors() {
        error("123.a", "[\"Error: Problem parsing float at error:1:5\"]");
        error("12. ", "[\"Error: Problem parsing float at error:1:4\"]");
        error("1.12ea", "[\"Error: Problem parsing float at error:1:6\"]");
        error(
            "123456789012345678901",
            "[\"Error: Problem parsing number at error:1:22\"]",
        );
        error(
            "\"1\\a2\"",
            "[\"Error: Unknown escape sequence at error:1:4\"]",
        );
        error(
            "\"\\",
            "[\"Fatal: String not correctly terminated at error:1:3\"]",
        );
        error(
            "\"1\\t2",
            "[\"Fatal: String not correctly terminated at error:1:6\"]",
        );
        error(
            "\"12\nss",
            "[\"Fatal: String not correctly terminated at error:1:4\"]",
        );
    }

    #[test]
    fn test_links() {
        let mut lex = Lexer::from_str("{num:1 + a*(2.0e2+= b )", "test_links");
        assert_eq!(lex.count_links(), 0);
        assert_eq!(lex.peek().has, LexItem::Token(String::from("{")));
        {
            lex.cont();
            test_id(&lex, "num");
            let l1 = lex.link();
            links(&lex, 1);
            test_id(&lex, "num");
            lex.cont();
            assert!(lex.has_token(":"));
            assert_eq!(lex.peek().has, LexItem::Integer(1, false));
            links(&lex, 1);
            lex.revert(l1);
            test_id(&lex, "num");
            links(&lex, 0);
        }
        links(&lex, 0);
        test_id(&lex, "num");
        lex.cont();
        links(&lex, 0);
        assert_eq!(lex.peek().has, LexItem::Token(":".to_string()));
        lex.mode = Mode::Code;
        assert!(lex.has_token(":"));
        if let Some(n) = lex.has_integer() {
            assert_eq!(n, 1);
        } else {
            panic!("Expected a number")
        }
        assert!(lex.has_token("+"));
        if let Some(n) = lex.has_identifier() {
            assert_eq!(n, "a");
        } else {
            panic!("Expected an identifier")
        }
        assert!(lex.has_token("*"));
        assert!(lex.has_token("("));
        if let LexResult {
            has: LexItem::Float(f, _),
            ..
        } = lex.peek()
        {
            assert!(f64::abs(f - 200.0) < 0.00001);
        } else {
            panic!("Expected a float")
        }
        lex.cont();
        assert!(lex.has_token("+="));
    }

    #[test]
    fn link_revert_replays_a_queued_number_split() {
        // A number that ends at a `..` or a field `.` emits TWO tokens from one scan: the
        // lexer returns the NUMBER and queues the follow-up in the replay buffer.  Both
        // must survive a revert, or an expression re-read after a look-ahead loses its
        // operand — `(s[i + 1..])` came back as `i`, `+`, `..` and left the `+` with one
        // side (loft#1164).
        let mut lex = Lexer::from_str("i + 1..4", "queued_split");
        assert_eq!(lex.peek().has, LexItem::Identifier("i".into()));

        // Look ahead over the whole expression, the way a tuple-literal classifier does.
        let l = lex.link();
        for _ in 0..5 {
            lex.cont();
        }
        lex.revert(l);

        // The replay must be what was READ, number included.
        assert_eq!(lex.peek().has, LexItem::Identifier("i".into()));
        lex.cont();
        assert_eq!(lex.peek().has, LexItem::Token("+".into()));
        lex.cont();
        assert_eq!(lex.peek().has, LexItem::Integer(1, false));
        lex.cont();
        assert_eq!(lex.peek().has, LexItem::Token("..".into()));
        lex.cont();
        assert_eq!(lex.peek().has, LexItem::Integer(4, false));
    }

    #[test]
    fn link_revert_repeatable_same_region() {
        // @PLN35 — two SEQUENTIAL link/revert look-aheads that start at the SAME
        // position (the shape `peek_named_arg` + a second classifier peek make) must
        // leave the stream intact.  The replay buffer must NOT duplicate the last
        // token replayed by the second peek: `cont()` decided whether to remember a
        // token from `link == memory.len()` AFTER `next()`, but `next()` bumps `link`
        // when it replays, so replaying the last buffered token fired the same branch
        // and re-appended a copy — the real parse then read that token twice.
        let mut lex = Lexer::from_str("a b c d", "link_repeat");
        assert_eq!(lex.peek().has, LexItem::Identifier("a".into()));

        // Peek 1: buffer through `b`, then revert to `a`.
        let l1 = lex.link();
        lex.cont();
        assert_eq!(lex.peek().has, LexItem::Identifier("b".into()));
        lex.revert(l1);
        assert_eq!(lex.peek().has, LexItem::Identifier("a".into()));

        // Peek 2: SAME start — replay `b` (the last buffered token), then revert.
        let l2 = lex.link();
        lex.cont();
        assert_eq!(lex.peek().has, LexItem::Identifier("b".into()));
        lex.revert(l2);
        assert_eq!(lex.peek().has, LexItem::Identifier("a".into()));
        assert_eq!(lex.count_links(), 0);

        // The real parse must now read a, b, c, d — not a, b, b, … (the corruption).
        for id in ["a", "b", "c", "d"] {
            assert_eq!(lex.peek().has, LexItem::Identifier(id.into()));
            lex.cont();
        }
        assert_eq!(lex.peek().has, LexItem::None);
    }

    #[test]
    fn link_revert_nested_links() {
        // Multiple links open at once: an inner link taken while an outer link is
        // still live, reverted inner-then-outer, must restore cleanly and keep the
        // full stream intact.
        let mut lex = Lexer::from_str("a b c d e", "link_nested");
        let outer = lex.link(); // at `a`
        lex.cont(); // b
        let inner = lex.link(); // at `b`
        lex.cont(); // c
        assert_eq!(lex.peek().has, LexItem::Identifier("c".into()));
        assert_eq!(lex.count_links(), 2);
        lex.revert(inner);
        assert_eq!(lex.peek().has, LexItem::Identifier("b".into()));
        assert_eq!(lex.count_links(), 1);
        lex.revert(outer);
        assert_eq!(lex.peek().has, LexItem::Identifier("a".into()));
        assert_eq!(lex.count_links(), 0);
        for id in ["a", "b", "c", "d", "e"] {
            assert_eq!(lex.peek().has, LexItem::Identifier(id.into()));
            lex.cont();
        }
        assert_eq!(lex.peek().has, LexItem::None);
    }

    #[test]
    fn test_formats() {
        validate(
            "\"ab{{cd}}ef\"",
            &[LexItem::CString("ab{cd}ef".to_string())],
        );
        validate(
            "\"ab{c:d}ef\"",
            &[
                LexItem::CString("ab".to_string()),
                LexItem::Identifier("c".to_string()),
                LexItem::Token(":".to_string()),
                LexItem::Identifier("d".to_string()),
                LexItem::CString("ef".to_string()),
            ],
        );
    }
}
