// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN183 P4 — which language construct is under the cursor, named in the catalogue's
//! vocabulary (`op:??`, `kw:match`, `index:hash`, `slice:text`, …).
//!
//! A hover on `??` should show the `??` entry, and a plain search of the catalogue cannot find
//! it: every entry USES the constructs it does not document (`??` appears in 13 entries' code).
//! So the construct is named from the source, and the entry that DECLARES it as a key answers
//! (`doc_catalogue::by_key`, one owner per key).  `[ ]` and a slice are named by the operand's
//! type, never by their text — `v[1]` and `h[1]` are different constructs.
//!
//! The answer is a key of [`CONSTRUCTS`], the closed list of names this classifier produces;
//! `tests/lsp_construct.rs` holds every one to exactly one catalogue entry.

/// Every construct name [`construct_at`] can answer.  Each must be declared by exactly one
/// catalogue entry (its `<!-- keys: … -->` line); the test pins that, and the catalogue keys
/// no hover reaches yet.
pub const CONSTRUCTS: &[&str] = &[
    "kw:if",
    "kw:else",
    "kw:store-else",
    "kw:for",
    "kw:match",
    "kw:is",
    "kw:break",
    "kw:continue",
    "kw:return",
    "kw:yield",
    "kw:yield-from",
    "kw:use",
    "kw:pub",
    "kw:par",
    "kw:null",
    "kw:#fields",
    "attr:#lock",
    "op:??",
    "op:??-return",
    "op:?",
    "op:**",
    "op:==",
    "op:compare",
    "op:arith",
    "op:compound",
    "op:logic",
    "op:bitwise",
    "op:as",
    "op:range",
    "param:&",
    "bind:&",
    "lambda:short",
    "lambda:fn",
    "decl:fn",
    "decl:struct",
    "decl:enum",
    "decl:interface",
    "decl:type-alias",
    "def:operator",
    "index:vector",
    "index:text",
    "index:hash",
    "index:index",
    "index:sorted",
    "index:spatial",
    "index:program-type",
    "slice:vector",
    "slice:text",
    "type:boolean",
    "type:character",
    "type:float",
    "type:single",
    "type:integer",
    "type:u8",
    "type:u16",
    "type:u32",
    "type:i8",
    "type:i16",
    "type:i32",
    "type:vector",
    "type:hash",
    "type:index",
    "type:sorted",
    "type:spatial",
    "type:iterator",
    "type:File",
    "kw:both",
    "type:fn",
    "type:nullable",
    "param:self",
    "param:const",
    "pattern:or",
    "pattern:guard",
    "lit:interpolation",
    "fmt:spec",
    "attr:#superseded",
    "append:vector",
    "fn:arguments",
    "fn:assert",
    "fn:deliver",
    "fn:expose",
    "fn:filter",
    "fn:json_parse",
    "fn:len",
    "fn:map",
    "fn:now",
    "fn:panic",
    "fn:reduce",
    "fn:rev",
    "fn:size",
    "fn:sizeof",
    "fn:stack_trace",
    "fn:store_reclaim",
    "fn:store_release",
    "fn:ticks",
    "fn:to_json",
    "def:operator-compare",
    "def:operator-conversion",
    "def:operator-divided_by",
    "def:operator-minus",
    "def:operator-negate",
    "def:operator-next",
    "def:operator-plus",
    "def:operator-remainder",
    "def:operator-times",
    "def:operator-to_text",
];

/// The stdlib functions the catalogue documents by name: a call to one names its entry.
const DOCUMENTED_FNS: &[(&str, &str)] = &[
    ("arguments", "fn:arguments"),
    ("assert", "fn:assert"),
    ("deliver", "fn:deliver"),
    ("expose", "fn:expose"),
    ("filter", "fn:filter"),
    ("json_parse", "fn:json_parse"),
    ("len", "fn:len"),
    ("map", "fn:map"),
    ("now", "fn:now"),
    ("panic", "fn:panic"),
    ("reduce", "fn:reduce"),
    ("rev", "fn:rev"),
    ("size", "fn:size"),
    ("sizeof", "fn:sizeof"),
    ("stack_trace", "fn:stack_trace"),
    ("store_reclaim", "fn:store_reclaim"),
    ("store_release", "fn:store_release"),
    ("ticks", "fn:ticks"),
    ("to_json", "fn:to_json"),
];

/// The `operator` forms the catalogue documents: `operator plus` names its entry.
const OPERATOR_FORMS: &[(&str, &str)] = &[
    ("compare", "def:operator-compare"),
    ("conversion", "def:operator-conversion"),
    ("divided_by", "def:operator-divided_by"),
    ("minus", "def:operator-minus"),
    ("negate", "def:operator-negate"),
    ("next", "def:operator-next"),
    ("plus", "def:operator-plus"),
    ("remainder", "def:operator-remainder"),
    ("times", "def:operator-times"),
    ("to_text", "def:operator-to_text"),
];

/// Whether index `i` sits inside a string literal on this line (an odd number of unescaped
/// `"` before it).
fn in_string(chars: &[char], i: usize) -> bool {
    let mut inside = false;
    let mut k = 0;
    while k < i.min(chars.len()) {
        match chars[k] {
            '\\' => k += 1,
            '"' => inside = !inside,
            _ => {}
        }
        k += 1;
    }
    inside
}

/// Whether index `i` sits inside an interpolation `{ … }` of a string literal.
fn in_interpolation(chars: &[char], i: usize) -> bool {
    in_string(chars, i) && {
        let open = chars[..i].iter().rposition(|c| *c == '{');
        let close = chars[..i].iter().rposition(|c| *c == '}');
        open.is_some_and(|o| close.is_none_or(|c| c < o))
    }
}

/// One line of `text`, 1-based, as chars.
fn line_chars(text: &str, line: u32) -> Vec<char> {
    text.lines()
        .nth(line.saturating_sub(1) as usize)
        .unwrap_or("")
        .chars()
        .collect()
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The previous non-blank char before index `i` on the line, if any.
fn prev_char(chars: &[char], i: usize) -> Option<char> {
    chars[..i]
        .iter()
        .rev()
        .copied()
        .find(|c| !c.is_whitespace())
}

/// The next word after index `i` on the line.
fn next_word(chars: &[char], i: usize) -> String {
    chars[i.min(chars.len())..]
        .iter()
        .skip_while(|c| !is_word(**c))
        .take_while(|c| is_word(**c))
        .collect()
}

/// The construct a word names, by itself and its neighbours on the line.
fn word_construct(chars: &[char], start: usize, end: usize) -> Option<&'static str> {
    let word: String = chars[start..end].iter().collect();
    let first_on_line = chars[..start]
        .iter()
        .collect::<String>()
        .split_whitespace()
        .all(|w| w == "pub");
    let prev_word: String = {
        let head: String = chars[..start].iter().collect();
        head.split_whitespace().last().unwrap_or("").to_string()
    };
    let in_arm = chars.iter().collect::<String>().contains("=>");
    let next = chars[end..].iter().copied().find(|c| !c.is_whitespace());
    if prev_word == "operator"
        && let Some((_, key)) = OPERATOR_FORMS.iter().find(|(f, _)| *f == word)
    {
        return Some(key);
    }
    if next == Some('(')
        && let Some((_, key)) = DOCUMENTED_FNS.iter().find(|(f, _)| *f == word)
    {
        return Some(key);
    }
    Some(match word.as_str() {
        // A guard: an `if` before the arm's `=>`.
        "if" if in_arm && !chars[..start].iter().collect::<String>().contains("=>") => {
            "pattern:guard"
        }
        "if" => "kw:if",
        "both" => "kw:both",
        "self" if next == Some(':') => "param:self",
        "const" => "param:const",
        "superseded" => "attr:#superseded",
        // `fn(` after a `:` or `->` is a function TYPE, not a lambda.
        "fn" if next == Some('(')
            && matches!(prev_char(chars, start), Some(':' | '>' | '<' | ',')) =>
        {
            "type:fn"
        }
        // An `else` after a block closes an `if`; after anything else it is a store's arm.
        "else" => {
            if prev_char(chars, start) == Some('}') {
                "kw:else"
            } else {
                "kw:store-else"
            }
        }
        "for" => "kw:for",
        "match" => "kw:match",
        "is" => "kw:is",
        "break" => "kw:break",
        "continue" => "kw:continue",
        "return" => "kw:return",
        "yield" if next_word(chars, end) == "from" => "kw:yield-from",
        "yield" => "kw:yield",
        "use" => "kw:use",
        "pub" => "kw:pub",
        "par" => "kw:par",
        "null" => "kw:null",
        "as" => "op:as",
        "fn" if first_on_line => "decl:fn",
        "fn" => "lambda:fn",
        "struct" => "decl:struct",
        "enum" => "decl:enum",
        "interface" => "decl:interface",
        "type" if first_on_line => "decl:type-alias",
        "operator" => "def:operator",
        "boolean" => "type:boolean",
        "character" => "type:character",
        "float" => "type:float",
        "single" => "type:single",
        "integer" => "type:integer",
        "u8" => "type:u8",
        "u16" => "type:u16",
        "u32" => "type:u32",
        "i8" => "type:i8",
        "i16" => "type:i16",
        "i32" => "type:i32",
        "vector" => "type:vector",
        "hash" => "type:hash",
        "index" => "type:index",
        "sorted" => "type:sorted",
        "spatial" => "type:spatial",
        "iterator" => "type:iterator",
        "File" => "type:File",
        _ => return None,
    })
}

/// The construct a keyword names on its own, with no line around it — a completion item's
/// (`else` reads as the `if`'s, `fn` as a declaration).
#[must_use]
pub fn keyword_construct(word: &str) -> Option<&'static str> {
    let chars: Vec<char> = word.chars().collect();
    word_construct(&chars, 0, chars.len())
}

const OPERATOR_CHARS: &str = "?=!<>+-*/%&|^.";

/// The construct an operator names.  `|x|` is a lambda, `&` after `:` a reference parameter
/// and after `=` a reference binding; `..` is a range, or a slice inside `[ ]` (decided by the
/// caller, which knows the operand's type).
fn operator_construct(chars: &[char], start: usize, end: usize) -> Option<&'static str> {
    let op: String = chars[start..end].iter().collect();
    let before = prev_char(chars, start);
    Some(match op.as_str() {
        "??" if matches!(
            next_word(chars, end).as_str(),
            "return" | "break" | "continue"
        ) =>
        {
            "op:??-return"
        }
        "??" => "op:??",
        "?" => "op:?",
        "**" => "op:**",
        "==" | "!=" => "op:==",
        "<" | "<=" | ">" | ">=" => "op:compare",
        "+" | "-" | "*" | "/" | "%" => "op:arith",
        "+=" | "-=" | "*=" | "/=" | "%=" => "op:compound",
        "&&" | "||" | "!" => "op:logic",
        "&" if before == Some(':') => "param:&",
        "&" if before == Some('=') => "bind:&",
        "|" if matches!(before, Some('(' | ',' | '=')) || before.is_none() => "lambda:short",
        "&" | "|" | "^" | "<<" | ">>" => "op:bitwise",
        ".." | "..=" => "op:range",
        _ => return None,
    })
}

/// The `[` that opens the bracket the cursor at `i` sits in (or on), if any, on this line.
fn open_bracket(chars: &[char], i: usize) -> Option<usize> {
    // On a `]`, the bracket it closes: start the walk just inside it.
    let from = if chars.get(i) == Some(&']') {
        i.checked_sub(1)?
    } else {
        i
    };
    let mut depth = 0i32;
    for j in (0..=from.min(chars.len().checked_sub(1)?)).rev() {
        match chars[j] {
            ']' => depth += 1,
            '[' if depth == 0 => return Some(j),
            '[' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// What the cursor at `line:col` (1-based) is on: a construct key of [`CONSTRUCTS`], and the
/// columns (1-based, end exclusive) of the operand whose TYPE decides it, when one does — the
/// identifier before a `[`.  `type_of` answers that operand's type name (`vector`, `text`,
/// `hash`, a program type), read from the hover's own resolution.
#[must_use]
pub fn construct_at(
    text: &str,
    line: u32,
    col: u32,
    type_of: &dyn Fn(u32) -> Option<String>,
) -> Option<&'static str> {
    let chars = line_chars(text, line);
    let i = (col as usize).checked_sub(1)?;
    let c = *chars.get(i)?;
    if c == '#' || is_word(c) {
        let back = |k: usize| chars[..k].iter().rev().take_while(|c| is_word(**c)).count();
        let start = if c == '#' { i + 1 } else { i - back(i) };
        let end = start
            + chars[start.min(chars.len())..]
                .iter()
                .take_while(|c| is_word(**c))
                .count();
        // `#lock`, `#fields`: the attribute after a `#`.
        if start > 0 && chars[start - 1] == '#' {
            let word: String = chars[start..end].iter().collect();
            return match word.as_str() {
                "lock" => Some("attr:#lock"),
                "fields" => Some("kw:#fields"),
                "superseded" => Some("attr:#superseded"),
                _ => None,
            };
        }
        return if is_word(c) {
            word_construct(&chars, start, end)
        } else {
            None
        };
    }
    if c == '[' || c == ']' || ((c == '.') && open_bracket(&chars, i).is_some()) {
        let open = open_bracket(&chars, i).or((c == '[').then_some(i))?;
        let close = (open + 1..chars.len())
            .find(|&k| chars[k] == ']')
            .unwrap_or(chars.len());
        let inside: String = chars[open + 1..close].iter().collect();
        // The operand: the identifier right before the `[`.
        let op_end = open;
        let ty = (op_end > 0 && is_word(chars[op_end - 1]))
            .then(|| type_of(u32::try_from(op_end).unwrap_or(0)))
            .flatten()?;
        let slice = inside.contains("..");
        return Some(match (ty.as_str(), slice) {
            ("vector", true) => "slice:vector",
            ("text", true) => "slice:text",
            ("vector", false) => "index:vector",
            ("text", false) => "index:text",
            ("hash", _) => "index:hash",
            ("index", _) => "index:index",
            ("sorted", _) => "index:sorted",
            ("spatial", _) => "index:spatial",
            _ => "index:program-type",
        });
    }
    // Inside a string: `{` / `}` open and close an interpolation, a `:` inside one starts its
    // format spec.
    if in_string(&chars, i) {
        return match c {
            '{' | '}' => Some("lit:interpolation"),
            ':' if in_interpolation(&chars, i) => Some("fmt:spec"),
            _ => None,
        };
    }
    if OPERATOR_CHARS.contains(c) {
        return operator_at(&chars, i, type_of);
    }
    None
}

/// The construct an operator at index `i` of the line names: the operator's own entry, or —
/// read from its neighbours — a type parameter (`vector<integer>`: none), the nullable type
/// (`integer?` after a `:`), a match arm's alternatives (`A | B =>`), an append (`v += [x]`
/// on a vector, from the operand's type).
fn operator_at(
    chars: &[char],
    i: usize,
    type_of: &dyn Fn(u32) -> Option<String>,
) -> Option<&'static str> {
    let start = i - chars[..i]
        .iter()
        .rev()
        .take_while(|c| OPERATOR_CHARS.contains(**c))
        .count();
    let end = i + chars[i..]
        .iter()
        .take_while(|c| OPERATOR_CHARS.contains(**c))
        .count();
    // `<` and `>` hugging words on both sides are a type's parameters, not a comparison.
    let op: String = chars[start..end].iter().collect();
    if matches!(op.as_str(), "<" | ">")
        && start > 0
        && is_word(chars[start - 1])
        && chars
            .get(end)
            .is_some_and(|c| is_word(*c) || *c == '>' || *c == ']')
    {
        return None;
    }
    // `integer?` after a `:` — the nullable TYPE, not the `?` operator.
    if op == "?" && start > 0 && is_word(chars[start - 1]) {
        let w = start
            - chars[..start]
                .iter()
                .rev()
                .take_while(|c| is_word(**c))
                .count();
        if prev_char(chars, w) == Some(':') {
            return Some("type:nullable");
        }
    }
    // `A | B =>`: alternatives of a match arm.
    if op == "|" && chars.iter().collect::<String>().contains("=>") && start > 0 {
        let arrow = chars.iter().collect::<String>().find("=>").unwrap_or(0);
        if start < arrow {
            return Some("pattern:or");
        }
    }
    // `v += [x]` on a vector: an append, decided by the operand's type.
    if op == "+=" {
        let left = chars[..start].iter().rposition(|c| !c.is_whitespace());
        if left.is_some_and(|l| is_word(chars[l]))
            && type_of(u32::try_from(left.unwrap_or(0) + 1).unwrap_or(0)).as_deref()
                == Some("vector")
        {
            return Some("append:vector");
        }
    }
    operator_construct(chars, start, end)
}
