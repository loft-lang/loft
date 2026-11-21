// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C
//! @PLN217 — the wide ledger: every place a 32-bit unit (`--width 32`) holds or computes 64
//! bits, by the function that does it and the reason.
//!
//! A 32-bit build is meant to do 64-bit work only where the source asks for it.  The compiler
//! knows where it wrote any: a value held as `int64_t` or `double` has a loft type, and a
//! 32-bit value meets a 64-bit one only through a conversion the emitter writes (`loft_w64`,
//! `loft_n32`, a cast to `double`).  The ledger lists both, so "is this 64-bit work asked for?"
//! is read off the unit, not guessed from the linked image.
//!
//! | class | is | verdict |
//! |---|---|---|
//! | `declared` | a variable or result whose type is wide by its declaration: `i64`, `u32`, `f64` | asked for |
//! | `temp` | a variable the compiler made (`__dn4_1`) whose type is wide | to be explained by a `declared` value of the same function |
//! | `undeclared` | a variable or result held in 64 bits whose type is the plain `integer` nobody declared wide | a compiler defect |
//! | `runtime` | a value converted where a runtime function takes or answers it | a runtime function not at the build's width yet |
//! | `op` | a value converted at a 64-bit operator (`loft_mul`, `LOFT_PLAIN_NEG`) | one operand is a `declared` value; else the compiler computed a 32-bit value in 64 bits |
//! | `call` | a value converted where one of the program's functions takes or answers it | that function is 64-bit: an `i64` parameter, a `#width 64` file |
//! | `value` | a value converted in an expression of the function itself | one of its `declared` values |
//!
//! `LOFT_WIDE_LEDGER=<file>` writes it for the unit being emitted, one line an entry:
//! `function`, `class`, `what`, `count`, tab-separated, sorted.  Nothing is written for a
//! 64-bit build.

use crate::data::{Data, Type};
use std::collections::BTreeMap;
use std::fmt::Write as _;

/// One line of the ledger.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Entry {
    pub function: String,
    pub class: &'static str,
    pub what: String,
}

/// The conversions the emitter writes where two widths meet.
const CROSSINGS: [&str; 3] = ["loft_w64(", "loft_n32(", "(double) "];

/// Is a value of `tp` held in 64 bits, and by whose choice?  `Some(true)`: its declaration
/// asks for them (`i64`, `u32`, `f64`).  `Some(false)`: it is the plain `integer` no
/// declaration made wide — in a body under the regime, where a written `integer` is an `i32`,
/// only the compiler makes one.  `None`: it is not held in 64 bits.
fn wide_by_declaration(tp: &Type) -> Option<bool> {
    match tp.base() {
        Type::Integer(spec) if !spec.is_width32() => {
            Some(*spec != crate::narrow::IntegerSpec::signed32())
        }
        Type::Float => Some(true),
        _ => None,
    }
}

/// The variables and the result of each function of `fns` whose body is under the regime and
/// which are held in 64 bits: `declared` or `undeclared`.
pub(crate) fn held(data: &Data, fns: &[u32]) -> BTreeMap<Entry, u32> {
    let mut out = BTreeMap::new();
    for &d in fns {
        if !data.body_width32(d) {
            continue;
        }
        let def = data.def(d);
        let mut note = |what: String, tp: &Type| {
            if let Some(declared) = wide_by_declaration(tp) {
                // A name the compiler made (`__dn4_1`, `_walk_at_6`, `x#index`) is no
                // declaration of the program's.
                let made = what.starts_with('_') || what.contains('#');
                let entry = Entry {
                    function: def.name().to_string(),
                    class: match (declared, made) {
                        (false, _) => "undeclared",
                        (true, true) => "temp",
                        (true, false) => "declared",
                    },
                    what,
                };
                *out.entry(entry).or_insert(0) += 1;
            }
        };
        note("the result".to_string(), def.returned());
        let vars = def.variables();
        for v in 0..vars.count() {
            note(vars.name(v).to_string(), vars.tp(v));
        }
    }
    out
}

/// The identifier a call's `(` at `open` follows, when it is one: `""` for a parenthesis that
/// only groups, a cast or a control keyword.
fn callee_before(text: &[u8], open: usize) -> &str {
    let mut end = open;
    while end > 0 && text[end - 1] == b' ' {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && (text[start - 1].is_ascii_alphanumeric() || text[start - 1] == b'_') {
        start -= 1;
    }
    let name = std::str::from_utf8(&text[start..end]).unwrap_or("");
    if matches!(
        name,
        "if" | "while" | "for" | "switch" | "return" | "sizeof"
    ) {
        ""
    } else {
        name
    }
}

/// The call a conversion at `at` is an argument of — or whose answer it converts, for a
/// conversion that wraps a call: the nearest enclosing call that is not itself a conversion,
/// `""` when the statement has none.  String literals are skipped: a path or a message in an
/// argument may hold a parenthesis.
fn enclosing_call(text: &[u8], at: usize) -> &str {
    let mut depth = 0usize;
    let mut i = at;
    while i > 0 {
        i -= 1;
        match text[i] {
            b'"' => {
                // Back to the quote that opens this literal.
                while i > 0 {
                    i -= 1;
                    if text[i] == b'"' && (i == 0 || text[i - 1] != b'\\') {
                        break;
                    }
                }
            }
            b')' => depth += 1,
            b'(' if depth > 0 => depth -= 1,
            b'(' => {
                let name = callee_before(text, i);
                if !name.is_empty() && !CROSSINGS.iter().any(|c| c.starts_with(name)) {
                    return name;
                }
            }
            b';' | b'{' | b'}' if depth == 0 => return "",
            _ => {}
        }
    }
    ""
}

/// The call a conversion at `at` wraps, when what it converts is a call's answer:
/// `loft_n32(loft_vec_len(v))` converts what `loft_vec_len` answers.
fn wrapped_call(text: &[u8], after: usize) -> &str {
    let mut end = after;
    while end < text.len() && (text[end].is_ascii_alphanumeric() || text[end] == b'_') {
        end += 1;
    }
    if end > after && end < text.len() && text[end] == b'(' {
        std::str::from_utf8(&text[after..end]).unwrap_or("")
    } else {
        ""
    }
}

/// Is `callee` one of the runtime's 64-bit OPERATORS — the checked `loft_add`, the wrapping
/// `LOFT_PLAIN_ADD` — which compute what the program wrote and take no loft `integer` as a
/// length or an index does?  Its `_at` form reports a fault at a source position.
fn is_wide_operator(callee: &str) -> bool {
    if callee.starts_with("LOFT_PLAIN_") {
        return true;
    }
    let name = callee.strip_suffix("_at").unwrap_or(callee);
    matches!(
        name,
        "loft_add"
            | "loft_sub"
            | "loft_mul"
            | "loft_div"
            | "loft_rem"
            | "loft_neg"
            | "loft_abs"
            | "loft_and"
            | "loft_or"
            | "loft_xor"
            | "loft_shl"
            | "loft_shr"
            | "loft_min"
            | "loft_max"
    )
}

fn class_of(callee: &str) -> &'static str {
    // A 32-bit helper (`loft_add32`, `loft_rem32_at`, `loft_pa_int4_32`) handed a narrowed
    // value computes in 32 bits: the 64 bits are the operand's own.
    if callee.is_empty()
        || callee.ends_with("32")
        || callee.contains("32_")
        || callee.starts_with("INT")
        || callee.starts_with("UINT")
    {
        "value"
    } else if is_wide_operator(callee) {
        "op"
    } else if callee.starts_with("loft_") || callee.starts_with("LOFT_") {
        "runtime"
    } else {
        // The program's own: a function (`n_`, `t_`), a function value's body (`f_`), a
        // worker (`__par_`).
        "call"
    }
}

/// The name of the C function a definition line opens: `static int32_t n_walk__fr(loft_ref v) {`.
fn definition_name(line: &str) -> Option<&str> {
    if line.starts_with([' ', '\t', '#', '/', '}']) || !line.ends_with('{') {
        return None;
    }
    let open = line.find('(')?;
    let head = &line[..open];
    let start = head
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map_or(0, |i| i + 1);
    (start < open).then(|| &head[start..])
}

/// Every conversion between a 32-bit and a 64-bit value in `program`, the unit's text after
/// its runtime: by the C function it stands in and what it converts for — the runtime
/// function or the program's function whose parameter or answer it meets, or the function's
/// own expression.
pub(crate) fn crossings(program: &str) -> BTreeMap<Entry, u32> {
    let mut out = BTreeMap::new();
    let mut function = "";
    let mut body = String::new();
    let flush = |function: &str, body: &str, out: &mut BTreeMap<Entry, u32>| {
        let text = body.as_bytes();
        for conv in CROSSINGS {
            for (at, _) in body.match_indices(conv) {
                let wrapped = wrapped_call(text, at + conv.len());
                let callee = if wrapped.is_empty() || wrapped.starts_with("loft_w64") {
                    enclosing_call(text, at)
                } else {
                    wrapped
                };
                // The runtime's own 32-bit helpers are no crossing's reason.
                let callee = if CROSSINGS.iter().any(|c| c.starts_with(callee)) {
                    ""
                } else {
                    callee
                };
                let entry = Entry {
                    function: function.to_string(),
                    class: class_of(callee),
                    what: if class_of(callee) == "value" {
                        conv.trim_end_matches(['(', ' ']).to_string()
                    } else {
                        callee.to_string()
                    },
                };
                *out.entry(entry).or_insert(0) += 1;
            }
        }
    };
    for line in program.lines() {
        if let Some(name) = definition_name(line) {
            flush(function, &body, &mut out);
            function = name;
            body.clear();
            continue;
        }
        if line == "}" {
            flush(function, &body, &mut out);
            function = "";
            body.clear();
            continue;
        }
        if !function.is_empty() {
            body.push_str(line);
            body.push('\n');
        }
    }
    flush(function, &body, &mut out);
    out
}

/// The ledger as its file: one line an entry, sorted.
pub(crate) fn render(entries: &BTreeMap<Entry, u32>) -> String {
    let mut text = String::new();
    for (e, n) in entries {
        let _ = writeln!(text, "{}\t{}\t{}\t{n}", e.function, e.class, e.what);
    }
    text
}

/// Write the ledger of a 32-bit unit where `LOFT_WIDE_LEDGER` names a file.  `program` is the
/// unit's text after its runtime; `fns` the functions it holds.
pub(crate) fn write(data: &Data, fns: &[u32], program: &str) {
    if !crate::keys::width32_all() {
        return;
    }
    let Some(path) = std::env::var_os("LOFT_WIDE_LEDGER").filter(|p| !p.is_empty()) else {
        return;
    };
    let mut entries = held(data, fns);
    entries.extend(crossings(program));
    let _ = crate::file_access::write(std::path::Path::new(&path), render(&entries));
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROGRAM: &str = "\
static const char *const names[] = { \"a (b\" };

static int32_t n_show__fr(loft_ref v0_v, int64_t v1_key) {
    int32_t t1;
    t1 = loft_n32(loft_vec_len(v0_v));
    loft_format_long(&w, loft_w64(t1), 10, \"a ( in a text\", 2);
    loft_format_float(&w, (double) v2_f, 0);
    t2 = n_wide(loft_w64(t1));
    if (v1_key < loft_w64(t1)) {
        t3 = loft_add32(t1, loft_n32(v1_key));
    }
    return t1;
}

static void n_plain(int32_t v0_a) {
    v1_b = loft_add32(v0_a, INT32_C(1));
}
";

    fn find(l: &BTreeMap<Entry, u32>, function: &str, class: &str, what: &str) -> u32 {
        l.iter()
            .find(|(e, _)| e.function == function && e.class == class && e.what == what)
            .map_or(0, |(_, n)| *n)
    }

    /// A conversion is charged to the call it serves: the runtime function whose answer it
    /// narrows or whose parameter it widens for, the program's function it calls, or — in a
    /// compare, or as the operand of a 32-bit helper — the function's own expression.  A
    /// parenthesis inside a string literal opens no call, and a function with no conversion
    /// has no line.
    #[test]
    fn a_conversion_is_charged_to_the_call_it_serves() {
        let l = crossings(PROGRAM);
        assert_eq!(find(&l, "n_show__fr", "runtime", "loft_vec_len"), 1);
        assert_eq!(find(&l, "n_show__fr", "runtime", "loft_format_long"), 1);
        assert_eq!(find(&l, "n_show__fr", "runtime", "loft_format_float"), 1);
        assert_eq!(find(&l, "n_show__fr", "call", "n_wide"), 1);
        assert_eq!(find(&l, "n_show__fr", "value", "loft_w64"), 1);
        assert_eq!(find(&l, "n_show__fr", "value", "loft_n32"), 1);
        assert_eq!(l.values().sum::<u32>(), 6);
        assert!(l.keys().all(|e| e.function != "n_plain"));
    }

    #[test]
    fn a_wide_type_is_told_from_the_plain_integer() {
        use crate::narrow::IntegerSpec;
        assert_eq!(
            wide_by_declaration(&Type::Integer(IntegerSpec::signed32())),
            Some(false),
            "the plain integer nobody declared wide"
        );
        assert_eq!(
            wide_by_declaration(&Type::Integer(IntegerSpec::i32())),
            None,
            "an i32 is not held in 64 bits"
        );
        assert_eq!(wide_by_declaration(&Type::Float), Some(true), "an f64");
        assert_eq!(wide_by_declaration(&Type::Single), None);
    }
}
