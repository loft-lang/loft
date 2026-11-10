// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `(R-CSplit)` (@PLN212): a function body over the size budget is written as a chain of
//! `LOFT_NOINLINE` part functions, so no function the C compiler optimises is large enough
//! for its optimiser time to grow out of bounds (MSVC's grows about quadratically above 400
//! runtime-call sites).
//!
//! The shape:
//! - every local declared at the body's top level, and every parameter, becomes a member of
//!   one frame struct, `struct <fn>__frame`, which the function owns; its declarations become
//!   assignments in place;
//! - consecutive top-level statements are lifted into `<fn>__part<k>(struct <fn>__frame *)`,
//!   called in order, each holding at most the budget of runtime-call sites where the
//!   statements allow;
//! - the function's static slots move to file scope, prefixed with the function's name.
//!
//! What stays in the function itself: a statement that returns, and `LOFT_HOLD` (its slot
//! array must live as long as the frame, since the hold list keeps its address).  A `goto`
//! and its label always land in the same part.  A body this pass cannot read for certain
//! (a `setjmp`, a preprocessor conditional, a block comment over lines, a label it cannot
//! place) is written unsplit.
//!
//! It runs in every semantics build (`--native-c`, `loft test`, `--emit-c`, the C sweep) and
//! never in a shipped one: a `--ship` unit and an ESP app keep every function whole
//! ([`super::functions_whole`]; NATIVE.md § Optimisation tiers), so a shipped artefact's C is
//! the C it was before the split existed.  `LOFT_C_SPLIT=<n>` sets the budget (`1` the
//! default, `0` off); `LOFT_NO_C_SPLIT=1` turns it off over any setting; `LOFT_TRACE_C_SPLIT=1`
//! names each function over the budget, its parts and its frame size, or why it stayed whole.
//!
//! By default the parts are compiled by MSVC alone: the function is written both ways, the
//! parts under `#if defined(_MSC_VER)` and the whole body under `#else` ([`everywhere`]).  The
//! statements are the same in both, so no answer can differ.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// Runtime-call sites per function above which `cl /O1`'s time stops being linear (@PLN212 P0).
const DEFAULT_BUDGET: usize = 300;

/// The budget the split runs under, or `None` when it is off: a shipped unit, or a switch.
pub(crate) fn budget() -> Option<usize> {
    if super::functions_whole()
        || crate::env_once!(std::env::var("LOFT_NO_C_SPLIT").is_ok_and(|v| v != "0"))
    {
        return None;
    }
    *crate::env_once!(@value Option<usize>, {
        match std::env::var("LOFT_C_SPLIT").ok().map(|v| v.trim().parse::<usize>()) {
            None | Some(Ok(1) | Err(_)) => Some(DEFAULT_BUDGET),
            Some(Ok(0)) => None,
            Some(Ok(n)) => Some(n),
        }
    })
}

/// Whether the parts are written for every C compiler: only where a budget was asked for
/// (`LOFT_C_SPLIT=<n>`, `n > 1`, the split cells and a bisect).  By default the parts are written
/// for MSVC alone, under `#if defined(_MSC_VER)`, with the whole function in the `#else` arm:
/// gcc and clang stay linear on a large function, and a split costs a hot function its
/// registers — a frame struct, its reset and two indirect calls per call (cbor `decode` 12x).
pub(crate) fn everywhere() -> bool {
    *crate::env_once!(@value bool, {
        std::env::var("LOFT_C_SPLIT")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .is_some_and(|n| n > 1)
    })
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_C_SPLIT").is_ok_and(|v| v != "0"))
}

/// `line` with the contents of its string and character literals and its comments blanked,
/// the same length, so a position in one is a position in the other.  `None` when a block
/// comment runs past the line: a body this pass cannot read line by line.
fn strip(line: &str) -> Option<String> {
    let b = line.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            q @ (b'"' | b'\'') => {
                out.push(q);
                i += 1;
                while i < b.len() && b[i] != q {
                    if b[i] == b'\\' && i + 1 < b.len() {
                        out.extend_from_slice(b"  ");
                        i += 2;
                    } else {
                        out.push(b' ');
                        i += 1;
                    }
                }
                if i < b.len() {
                    out.push(q);
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'/') => {
                out.resize(b.len(), b' ');
                break;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let end = line[i + 2..].find("*/")? + i + 4;
                out.resize(out.len() + (end - i), b' ');
                i = end;
            }
            c => {
                out.push(c);
                i += 1;
            }
        }
    }
    // Only ASCII was replaced by ASCII, so the bytes stay UTF-8.
    Some(String::from_utf8(out).unwrap_or_default())
}

fn ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// Every identifier of a stripped text, with its byte range; number literals are skipped
/// whole, so `0x1F` and `INT64_C`'s `1` name nothing.
fn idents(s: &str) -> Vec<(usize, usize)> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_digit() {
            while i < b.len() && (ident_char(b[i]) || b[i] == b'.') {
                i += 1;
            }
        } else if ident_start(b[i]) {
            let s0 = i;
            while i < b.len() && ident_char(b[i]) {
                i += 1;
            }
            out.push((s0, i));
        } else {
            i += 1;
        }
    }
    out
}

/// The runtime-call sites of a stripped text: `loft_<name>(`, the unit P0 measured.
fn call_sites(s: &str) -> usize {
    let b = s.as_bytes();
    idents(s)
        .into_iter()
        .filter(|&(a, e)| {
            s[a..e].starts_with("loft_")
                && b[e..].iter().find(|c| !c.is_ascii_whitespace()) == Some(&b'(')
        })
        .count()
}

/// `orig` with every identifier `names` maps replaced, except one written after `.` or `->`
/// (a member of something else) and one inside a literal or comment.
fn rewrite(orig: &str, bare: &str, names: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(orig.len() + 16);
    let mut last = 0;
    for (a, e) in idents(bare) {
        let Some(to) = names.get(&bare[a..e]) else {
            continue;
        };
        let before = bare[..a].trim_end();
        if before.ends_with('.') || before.ends_with("->") {
            continue;
        }
        out.push_str(&orig[last..a]);
        out.push_str(to);
        last = e;
    }
    out.push_str(&orig[last..]);
    out
}

/// A top-level statement: its lines, original and stripped.
struct Stmt {
    orig: Vec<String>,
    bare: Vec<String>,
}

impl Stmt {
    fn bare_text(&self) -> String {
        self.bare.join("\n")
    }
}

/// The body's top-level statements by brace and parenthesis depth.  A line that opens no
/// statement of its own (`#line`, a statement's continuation) joins the one it belongs to; an
/// `else` joins the `if` before it.
fn statements(lines: &[&str]) -> Option<Vec<Stmt>> {
    let mut out: Vec<Stmt> = Vec::new();
    let mut cur: Option<Stmt> = None;
    let mut depth: i64 = 0;
    for &l in lines {
        let bare = strip(l)?;
        let t = bare.trim();
        if t.starts_with('#') && !t.starts_with("#line") {
            return None;
        }
        if cur.is_none() && depth == 0 && t.starts_with("else") {
            cur = out.pop();
        }
        let s = cur.get_or_insert_with(|| Stmt {
            orig: Vec::new(),
            bare: Vec::new(),
        });
        for c in bare.bytes() {
            match c {
                b'{' | b'(' => depth += 1,
                b'}' | b')' => depth -= 1,
                _ => {}
            }
        }
        s.orig.push(l.to_string());
        s.bare.push(bare.clone());
        if depth < 0 {
            return None;
        }
        if depth == 0 && !t.starts_with('#') && (t.ends_with(';') || t.ends_with('}')) {
            out.extend(cur.take());
        }
    }
    out.extend(cur.take());
    Some(out)
}

/// One declarator of a top-level declaration.
struct Decl {
    /// The member's spelling in the frame struct, without its `;`.
    member: String,
    name: String,
    /// The assignment that replaces it in place, before identifiers are rewritten.
    assign: Option<String>,
}

const NOT_TYPES: &[&str] = &[
    "return", "goto", "if", "else", "for", "while", "do", "switch", "case", "default", "break",
    "continue", "sizeof", "static",
];

/// Split `s` at every top-level occurrence of `sep` (outside brackets of any kind).
fn split_top(s: &str, sep: u8) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut depth = 0i64;
    let mut start = 0;
    for (i, c) in s.bytes().enumerate() {
        match c {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth -= 1,
            c if c == sep && depth == 0 => {
                out.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push((start, s.len()));
    out
}

/// The first top-level `=` of `s` that assigns: not part of `==`, `<=`, `>=` or `!=`.
fn first_assign(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut depth = 0i64;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'{' | b'(' | b'[' => depth += 1,
            b'}' | b')' | b']' => depth -= 1,
            b'=' if depth == 0
                && b.get(i + 1) != Some(&b'=')
                && !matches!(
                    i.checked_sub(1).map(|j| b[j]),
                    Some(b'=' | b'<' | b'>' | b'!')
                ) =>
            {
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

/// `TYPE *NAME[N]` (the part of a declarator before its `=`): the type words, the stars, the
/// name and the array suffix.  `None` for anything that is not that shape.
fn declarator(bare: &str, orig: &str) -> Option<(String, String, String, String)> {
    let t = bare.trim_end();
    let (head, arr) = match t.find('[') {
        Some(k) => (&t[..k], orig[k..t.len()].trim().to_string()),
        None => (t, String::new()),
    };
    if !arr.is_empty() && !arr.ends_with(']') {
        return None;
    }
    let ids = idents(head);
    let &(na, ne) = ids.last()?;
    if head[ne..].trim() != "" {
        return None;
    }
    let before = &head[..na];
    let stars: String = before
        .trim_end()
        .chars()
        .rev()
        .take_while(|c| *c == '*' || c.is_whitespace())
        .collect();
    let stars: String = stars.chars().filter(|c| *c == '*').collect();
    let words = before
        .trim_end()
        .trim_end_matches(|c: char| c == '*' || c.is_whitespace());
    // The type words alone: identifiers and spaces, nothing else.
    if !words
        .bytes()
        .all(|c| ident_char(c) || c.is_ascii_whitespace())
    {
        return None;
    }
    Some((
        words.trim().to_string(),
        stars,
        head[na..ne].to_string(),
        arr,
    ))
}

/// `LOFT_HH_NONE`: an upper-case identifier, by the C convention an object-like macro, whose
/// expansion the split cannot read.
fn is_macro_name(init: &str) -> bool {
    let b = init.as_bytes();
    !b.is_empty()
        && !b[0].is_ascii_digit()
        && b.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || *c == b'_')
        && b.iter().any(u8::is_ascii_uppercase)
}

/// The declarators of a top-level declaration statement, or `None` when it is no declaration.
/// One line may hold several (`int a = 0; int64_t b = 0, c = 0;`).
fn declaration(s: &Stmt) -> Option<Vec<Decl>> {
    let bare = s.bare_text();
    let orig = s.orig.join("\n");
    if bare.trim_start().starts_with('#') {
        return None;
    }
    let mut out = Vec::new();
    let segs: Vec<(usize, usize)> = split_top(&bare, b';')
        .into_iter()
        .filter(|&(a, e)| !bare[a..e].trim().is_empty())
        .collect();
    if segs.is_empty() {
        return None;
    }
    for (a, e) in segs {
        let (sb, so) = (&bare[a..e], &orig[a..e]);
        let parts = split_top(sb, b',');
        let mut ty = String::new();
        for (k, &(pa, pe)) in parts.iter().enumerate() {
            let (pb, po) = (&sb[pa..pe], &so[pa..pe]);
            let (lhs_b, lhs_o, init) = match first_assign(pb) {
                None => (pb, po, None),
                Some(k) => (&pb[..k], &po[..k], Some(po[k + 1..].trim().to_string())),
            };
            let (words, stars, name, arr) = declarator(lhs_b, lhs_o)?;
            if k == 0 {
                let w: Vec<&str> = words.split_whitespace().collect();
                if w.is_empty() || w.iter().any(|x| NOT_TYPES.contains(x)) {
                    return None;
                }
                // A `const` value becomes an assigned member, so it loses its qualifier.
                ty = if stars.is_empty() && w[0] == "const" {
                    w[1..].join(" ")
                } else {
                    w.join(" ")
                };
                if ty.is_empty() {
                    return None;
                }
            } else if !words.is_empty() {
                return None;
            }
            let member = format!("{ty} {stars}{name}{arr}");
            let assign = init.map(|init| {
                if !arr.is_empty() {
                    format!("{{ {ty} {stars}loft_split_tmp{arr} = {init}; memcpy({name}, loft_split_tmp, sizeof loft_split_tmp); }}")
                } else if init.starts_with('{') {
                    format!("{name} = ({ty} {stars}){init};")
                } else if is_macro_name(&init) {
                    // A macro may expand to a brace list (`LOFT_HH_NONE`), which only an
                    // initializer accepts: a temporary keeps it one, whatever it expands to.
                    format!("{{ {ty} {stars}loft_split_tmp = {init}; {name} = loft_split_tmp; }}")
                } else {
                    format!("{name} = {init};")
                }
            });
            out.push(Decl {
                member,
                name,
                assign,
            });
        }
    }
    Some(out)
}

/// `true` when `s` must stay in the function itself: it holds the frame's texts (`LOFT_HOLD`
/// keeps its slot array's address on the hold list), or it leaves the frame another way.
fn pinned(bare: &str) -> bool {
    idents(bare).into_iter().any(|(a, e)| {
        matches!(
            &bare[a..e],
            "LOFT_HOLD" | "loft_hd" | "longjmp" | "va_start" | "alloca"
        )
    })
}

/// `s` with each `return` made a part's: the value goes to the frame's `loft_split_ret` and
/// the part answers 1, which its caller turns into the function's own return.  `None` when a
/// `return` does not end on its own line, which this pass does not rewrite (the statement
/// then stays in the function).  The second value says whether `s` returns at all.
fn lift_returns(s: &Stmt, void: bool) -> Option<(Stmt, bool)> {
    let mut out = Stmt {
        orig: Vec::new(),
        bare: Vec::new(),
    };
    let mut any = false;
    for (o, b) in s.orig.iter().zip(&s.bare) {
        let mut o = o.clone();
        let rets: Vec<(usize, usize)> = idents(b)
            .into_iter()
            .filter(|&(a, e)| &b[a..e] == "return")
            .collect();
        // Right to left, so each position still names the original text.
        for &(a, e) in rets.iter().rev() {
            any = true;
            let mut depth = 0i64;
            let semi = b[e..].bytes().enumerate().find_map(|(k, c)| {
                match c {
                    b'(' | b'[' | b'{' => depth += 1,
                    b')' | b']' | b'}' => depth -= 1,
                    b';' if depth == 0 => return Some(e + k),
                    _ => {}
                }
                None
            })?;
            let expr = o[e..semi].trim().to_string();
            let with = match (expr.is_empty(), void) {
                (true, _) => "return 1;".to_string(),
                (false, true) => format!("{{ (void) ({expr}); return 1; }}"),
                (false, false) => format!("{{ loft_split_ret = ({expr}); return 1; }}"),
            };
            o.replace_range(a..=semi, &with);
        }
        let bare = strip(&o)?;
        out.orig.push(o);
        out.bare.push(bare);
    }
    Some((out, any))
}

fn label_of(bare_line: &str) -> Option<&str> {
    let t = bare_line.trim();
    let e = t.bytes().take_while(|c| ident_char(*c)).count();
    if e == 0 || !ident_start(t.as_bytes()[0]) {
        return None;
    }
    let rest = &t[e..];
    (rest.starts_with(':') && !rest.starts_with("::") && !matches!(&t[..e], "default" | "case"))
        .then_some(&t[..e])
}

/// The written text of `<storage> <ret> <name>(<params>) { <body> }` split into parts, or
/// `None` when it stays as it is: under the budget, or a body this pass does not split.
pub(crate) fn split(
    storage: &str,
    ret: &str,
    name: &str,
    params: &str,
    body: &str,
) -> Option<String> {
    split_at(budget()?, storage, ret, name, params, body)
}

fn split_at(
    budget: usize,
    storage: &str,
    ret: &str,
    name: &str,
    params: &str,
    body: &str,
) -> Option<String> {
    let lines: Vec<&str> = body
        .strip_suffix('\n')
        .unwrap_or(body)
        .split('\n')
        .collect();
    let stmts = statements(&lines)?;
    let cost: Vec<usize> = stmts.iter().map(|s| call_sites(&s.bare_text())).collect();
    let total: usize = cost.iter().sum();
    if total <= budget {
        return None;
    }
    let refuse = |why: &str| {
        if trace() {
            eprintln!("[c-split] {name}: {total} sites, kept whole: {why}");
        }
        None
    };
    // A `/*loft_hot:D*/` marker resolves later to `static` or `static inline`; the function
    // keeps it, and its parts are out of line either way.
    if storage != "static" && !storage.starts_with("static /*loft_hot:") {
        return refuse("an inline function");
    }
    if params.contains('(') || params.contains("...") {
        return refuse("a function-pointer or variadic parameter");
    }
    if stmts.iter().any(|s| {
        idents(&s.bare_text()).into_iter().any(|(a, e)| {
            let b = s.bare_text();
            matches!(&b[a..e], "setjmp" | "_setjmp" | "sigsetjmp")
        })
    }) {
        return refuse("a setjmp");
    }
    // The members: every parameter, every top-level local (name, declaration).  The static
    // slots move out.
    let mut members: Vec<(String, String)> = Vec::new();
    let mut param_names: HashSet<String> = HashSet::new();
    let mut names: HashMap<String, String> = HashMap::new();
    let mut statics: Vec<String> = Vec::new();
    let mut moved: Vec<Stmt> = Vec::new();
    let mut moved_cost: Vec<usize> = Vec::new();
    if params.trim() != "void" && !params.trim().is_empty() {
        for p in params.split(',') {
            let p = p.trim();
            let Some((a, e)) = idents(p).last().copied() else {
                return refuse("a parameter without a name");
            };
            members.push((p[a..e].to_string(), p.to_string()));
            param_names.insert(p[a..e].to_string());
            names.insert(p[a..e].to_string(), format!("loft_fr->{}", &p[a..e]));
        }
    }
    let mut slot_renames: Vec<(String, String)> = Vec::new();
    for (s, c) in stmts.into_iter().zip(cost) {
        let bare = s.bare_text();
        let t = bare.trim();
        if let Some(rest) = t.strip_prefix("LOFT_REF_IN(") {
            let v = rest.trim_end_matches(';').trim_end_matches(')').trim();
            if !v.bytes().all(ident_char) {
                return refuse("an unreadable LOFT_REF_IN");
            }
            members.push((v.to_string(), format!("loft_ref {v}")));
            names.insert(v.to_string(), format!("loft_fr->{v}"));
            moved.push(Stmt {
                orig: vec![format!("    {v} = {v}__in;")],
                bare: vec![format!("    {v} = {v}__in;")],
            });
            moved_cost.push(0);
            continue;
        }
        if let Some(decl) = t.strip_prefix("static ") {
            // A static slot keeps its one instance: at file scope, named for this function.
            let Some(ds) = declaration(&Stmt {
                orig: vec![decl.to_string()],
                bare: vec![decl.to_string()],
            }) else {
                return refuse("an unreadable static");
            };
            for d in &ds {
                slot_renames.push((d.name.clone(), format!("{name}__{}", d.name)));
            }
            statics.push(
                s.orig
                    .iter()
                    .map(|l| l.trim())
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            continue;
        }
        if let Some(ds) = declaration(&s) {
            let mut assigns = Vec::new();
            for d in ds {
                members.push((d.name.clone(), d.member));
                names.insert(d.name.clone(), format!("loft_fr->{}", d.name));
                assigns.extend(d.assign);
            }
            if !assigns.is_empty() {
                let l = format!("    {}", assigns.join(" "));
                let b = strip(&l)?;
                moved.push(Stmt {
                    orig: vec![l],
                    bare: vec![b],
                });
                moved_cost.push(c);
            }
            continue;
        }
        moved.push(s);
        moved_cost.push(c);
    }
    for (from, to) in slot_renames {
        names.insert(from, to);
    }
    names.insert(
        "loft_split_ret".to_string(),
        "loft_fr->loft_split_ret".to_string(),
    );
    let stmts = moved;
    let cost = moved_cost;
    // A statement that returns moves with its `return` rewritten; one this pass cannot
    // rewrite stays in the function.
    let void = ret.trim() == "void";
    let lifted: Vec<Option<(Stmt, bool)>> = stmts.iter().map(|s| lift_returns(s, void)).collect();
    // Labels and gotos: a goto and its label land in one unit.
    let mut label_at: HashMap<String, usize> = HashMap::new();
    for (k, s) in stmts.iter().enumerate() {
        for b in &s.bare {
            if let Some(l) = label_of(b) {
                label_at.insert(l.to_string(), k);
            }
        }
    }
    let n = stmts.len();
    let mut joined_to = vec![0usize; n]; // the furthest statement `k`'s unit must reach
    for (k, s) in stmts.iter().enumerate() {
        joined_to[k] = joined_to[k].max(k);
        let bare = s.bare_text();
        let ids = idents(&bare);
        for w in ids.windows(2) {
            if &bare[w[0].0..w[0].1] == "goto" {
                let target = &bare[w[1].0..w[1].1];
                let Some(&t) = label_at.get(target) else {
                    return refuse("a goto to a label it cannot place");
                };
                let (lo, hi) = (k.min(t), k.max(t));
                joined_to[lo] = joined_to[lo].max(hi);
            }
        }
    }
    // Units: maximal runs that no goto crosses.
    let mut units: Vec<(usize, usize, usize, bool)> = Vec::new(); // first, last, cost, movable
    let mut k = 0;
    while k < n {
        let mut end = joined_to[k];
        let mut j = k;
        while j <= end {
            end = end.max(joined_to[j]);
            j += 1;
        }
        let c = cost[k..=end].iter().sum();
        let movable = (k..=end).all(|j| !pinned(&stmts[j].bare_text()) && lifted[j].is_some());
        units.push((k, end, c, movable));
        k = end + 1;
    }
    // Greedy runs of movable units under the budget.
    let mut parts: Vec<Vec<usize>> = Vec::new(); // unit indices
    let mut cur: Vec<usize> = Vec::new();
    let mut cur_cost = 0;
    for (u, &(_, _, c, movable)) in units.iter().enumerate() {
        if !movable {
            parts.extend((!cur.is_empty()).then(|| std::mem::take(&mut cur)));
            cur_cost = 0;
            continue;
        }
        if !cur.is_empty() && cur_cost + c > budget {
            parts.push(std::mem::take(&mut cur));
            cur_cost = 0;
        }
        cur.push(u);
        cur_cost += c;
    }
    parts.extend((!cur.is_empty()).then_some(cur));
    // A small run stays in the function while the function stays under the budget.
    let unit_cost = |p: &Vec<usize>| p.iter().map(|&u| units[u].2).sum::<usize>();
    let mut parent_cost: usize = units.iter().filter(|u| !u.3).map(|u| u.2).sum();
    parts.retain(|p| {
        let c = unit_cost(p);
        if c < budget / 4 && parent_cost + c <= budget {
            parent_cost += c;
            false
        } else {
            true
        }
    });
    // One part holding nearly everything moves the size, it does not cut it.
    if parts.is_empty() || (parts.len() == 1 && parent_cost < budget / 4) {
        return refuse("no statement boundary to split at");
    }
    let mut in_part: Vec<Option<usize>> = vec![None; n];
    for (pi, p) in parts.iter().enumerate() {
        for &u in p {
            for slot in &mut in_part[units[u].0..=units[u].1] {
                *slot = Some(pi);
            }
        }
    }
    // A local only one part reads or writes stays that part's own: through the frame, the
    // compiler keeps every access in memory, and a function of thousands of temporaries
    // compiled 38 times slower with all of them there.  Every other local, and every
    // parameter, is a frame member.
    let mut used: HashMap<&str, HashSet<Option<usize>>> = HashMap::new();
    let member_names: HashSet<&str> = members.iter().map(|(n, _)| n.as_str()).collect();
    for (k, s) in stmts.iter().enumerate() {
        let bare = s.bare_text();
        for (a, e) in idents(&bare) {
            if let Some(&m) = member_names.get(&bare[a..e]) {
                used.entry(m).or_default().insert(in_part[k]);
            }
        }
    }
    // A local whose address is taken stays in the frame too: a link holding it may be read by
    // a later part, after its own part's frame is gone.
    let mut addr_taken: HashSet<&str> = HashSet::new();
    for s in &stmts {
        let bare = s.bare_text();
        for (a, e) in idents(&bare) {
            let before = bare[..a].trim_end();
            if before.ends_with('&')
                && !before.ends_with("&&")
                && let Some(&m) = member_names.get(&bare[a..e])
            {
                addr_taken.insert(m);
            }
        }
    }
    let own_part = |m: &str| -> Option<usize> {
        if param_names.contains(m) || addr_taken.contains(m) {
            return None;
        }
        let u = used.get(m)?;
        match u.iter().next() {
            Some(Some(pi)) if u.len() == 1 => Some(*pi),
            _ => None,
        }
    };
    let mut part_locals: Vec<Vec<&str>> = vec![Vec::new(); parts.len()];
    let mut frame_members: Vec<&str> = Vec::new();
    for (m, decl) in &members {
        match own_part(m) {
            Some(pi) => part_locals[pi].push(decl),
            None => frame_members.push(decl),
        }
    }
    let part_names: Vec<HashMap<String, String>> = (0..parts.len())
        .map(|pi| {
            let mut n = names.clone();
            for (m, _) in &members {
                if own_part(m) == Some(pi) {
                    n.remove(m);
                }
            }
            n
        })
        .collect();
    let emit = |out: &mut String, s: &Stmt, names: &HashMap<String, String>| {
        for (o, b) in s.orig.iter().zip(&s.bare) {
            let _ = writeln!(out, "{}", rewrite(o, b, names));
        }
    };
    let frame = format!("{name}__frame");
    let mut out = String::new();
    for st in &statics {
        let b = strip(st)?;
        let _ = writeln!(out, "{}", rewrite(st, &b, &names));
    }
    let _ = writeln!(out, "struct {frame} {{");
    for m in &frame_members {
        let _ = writeln!(out, "    {m};");
    }
    let returns: Vec<bool> = parts
        .iter()
        .map(|p| {
            p.iter().any(|&u| {
                (units[u].0..=units[u].1).any(|j| lifted[j].as_ref().is_some_and(|l| l.1))
            })
        })
        .collect();
    if !void && returns.iter().any(|r| *r) {
        let _ = writeln!(out, "    {ret} loft_split_ret;");
    }
    let _ = writeln!(out, "    char loft_unused;\n}};");
    for (pi, p) in parts.iter().enumerate() {
        let kind = if returns[pi] { "int" } else { "void" };
        let _ = writeln!(
            out,
            "static LOFT_NOINLINE {kind} {name}__part{pi}(void *loft_frv) {{\n    struct {frame} *const loft_fr = (struct {frame} *) loft_frv;"
        );
        for d in &part_locals[pi] {
            let _ = writeln!(out, "    {d};");
        }
        for &u in p {
            for j in units[u].0..=units[u].1 {
                let s = lifted[j].as_ref().map_or(&stmts[j], |l| &l.0);
                emit(&mut out, s, &part_names[pi]);
            }
        }
        if returns[pi] {
            let _ = writeln!(out, "    return 0;");
        }
        let _ = writeln!(out, "}}");
    }
    let _ = writeln!(out, "{storage} {ret} {name}({params}) {{");
    let _ = writeln!(
        out,
        "    struct {frame} loft_frs;\n    struct {frame} *const loft_fr = &loft_frs;"
    );
    let _ = writeln!(out, "    memset(&loft_frs, 0, sizeof loft_frs);");
    if params.trim() != "void" && !params.trim().is_empty() {
        for p in params.split(',') {
            let p = p.trim();
            if let Some((a, e)) = idents(p).last().copied() {
                let _ = writeln!(out, "    loft_fr->{0} = {0};", &p[a..e]);
            }
        }
    }
    let mut called: HashSet<usize> = HashSet::new();
    for (k, s) in stmts.iter().enumerate() {
        match in_part[k] {
            Some(pi) => {
                if called.insert(pi) {
                    let _ = match (returns[pi], void) {
                        (false, _) => {
                            writeln!(out, "    LOFT_PART_CALL({name}__part{pi}, loft_fr);")
                        }
                        (true, true) => writeln!(
                            out,
                            "    if (LOFT_PART_RET({name}__part{pi}, loft_fr)) return;"
                        ),
                        (true, false) => writeln!(
                            out,
                            "    if (LOFT_PART_RET({name}__part{pi}, loft_fr)) return loft_fr->loft_split_ret;"
                        ),
                    };
                }
            }
            None => emit(&mut out, s, &names),
        }
    }
    // A function whose last statement went into a part ends in that part's call; the call
    // returns when the function does, and a compiler that warns of a path without a `return`
    // (MSVC's C4715, an error under `/WX`) is answered with the value the part left.
    if !void && n > 0 && in_part[n - 1].is_some() {
        let _ = writeln!(out, "    return loft_fr->loft_split_ret;");
    }
    let _ = writeln!(out, "}}");
    if trace() {
        let costs: Vec<String> = parts.iter().map(|p| unit_cost(p).to_string()).collect();
        eprintln!(
            "[c-split] {name}: {total} sites into {} parts ({}), {parent_cost} left in the function, {} frame members, {} part locals",
            parts.len(),
            costs.join(", "),
            frame_members.len(),
            members.len() - frame_members.len()
        );
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::split_at;
    use std::fmt::Write as _;

    /// Three statements of three runtime calls each, between `pre` and `post`.
    fn body(pre: &str, post: &str) -> String {
        let mut b = String::from("    int64_t v0_a = 0;\n    loft_text v1_t = { 0, 0, 0 };\n");
        b.push_str(pre);
        for k in 0..3 {
            let _ = writeln!(b, "    v0_a = loft_add(loft_add(v0_a, {k}), loft_one());");
        }
        b.push_str(post);
        b
    }

    /// A statement of three runtime calls, so a part of its own, reading `what`.
    fn read(what: &str) -> String {
        format!("    v0_a = loft_a(loft_b(loft_c({what})));\n")
    }

    fn split(b: &str) -> Option<String> {
        split_at(3, "static", "void", "n_f__fr", "int64_t v9_p", b)
    }

    #[test]
    fn under_the_budget_nothing_changes() {
        assert!(split_at(100, "static", "void", "n_f__fr", "void", &body("", "")).is_none());
    }

    #[test]
    fn parts_share_one_frame() {
        let out = split(&body("", "")).expect("split");
        let frame = "struct n_f__fr__frame {\n    int64_t v9_p;\n    int64_t v0_a;\n    char loft_unused;\n";
        assert!(out.contains(frame), "{out}");
        let part2 = "n_f__fr__part2(void *loft_frv) {\n    \
                     struct n_f__fr__frame *const loft_fr = (struct n_f__fr__frame *) loft_frv;\n    \
                     loft_fr->v0_a = loft_add(loft_add(loft_fr->v0_a, 2), loft_one());";
        assert!(out.contains(part2), "{out}");
    }

    /// A local one part alone uses is declared in that part, never in the frame.
    #[test]
    fn a_local_of_one_part_stays_in_it() {
        let out = split(&body("", "")).expect("split");
        let part0 = "n_f__fr__part0(void *loft_frv) {\n    struct n_f__fr__frame *const loft_fr = (struct n_f__fr__frame *) loft_frv;\n    loft_text v1_t;\n";
        assert!(out.contains(part0), "{out}");
        assert!(
            out.contains("\n    v1_t = (loft_text ){ 0, 0, 0 };\n"),
            "{out}"
        );
    }

    /// The hold list keeps the address of `LOFT_HOLD`'s slot array, so the array must live in
    /// the function's own frame, never in a part that has returned.
    #[test]
    fn the_hold_list_stays_in_the_function() {
        let out = split(&body("    LOFT_HOLD(1, &v1_t);\n", "")).expect("split");
        let f = out.find("static void n_f__fr(").expect("the function");
        assert!(
            out[f..].contains("    LOFT_HOLD(1, &loft_fr->v1_t);"),
            "{out}"
        );
        assert!(!out[..f].contains("LOFT_HOLD"), "{out}");
    }

    /// A part that returns answers 1 and leaves its value in the frame; the function returns
    /// it at once.
    #[test]
    fn a_return_moves_with_its_part() {
        let ret = "    if (loft_c(v0_a)) return loft_d(v0_a, v9_p);\n";
        let out = split_at(
            3,
            "static",
            "int64_t",
            "n_f__fr",
            "int64_t v9_p",
            &body(ret, "    return v0_a;\n"),
        )
        .expect("split");
        assert!(out.contains("    int64_t loft_split_ret;\n"), "{out}");
        let lifted = "if (loft_c(loft_fr->v0_a)) { loft_fr->loft_split_ret = \
                      (loft_d(loft_fr->v0_a, loft_fr->v9_p)); return 1; }";
        assert!(out.contains(lifted), "{out}");
        assert!(
            out.contains(
                "if (LOFT_PART_RET(n_f__fr__part0, loft_fr)) return loft_fr->loft_split_ret;"
            ),
            "{out}"
        );
        assert!(out.contains("    return 0;\n}"), "{out}");
        // The tail `return` went into the last part, so the function ends returning its value.
        assert!(
            out.ends_with(
                "return loft_fr->loft_split_ret;\n    return loft_fr->loft_split_ret;\n}\n"
            ),
            "{out}"
        );
    }

    /// A local whose address is taken stays in the frame even when one part alone names it:
    /// a link holding the address may be read after that part has returned.
    #[test]
    fn an_address_taken_local_stays_in_the_frame() {
        let link = "    loft_text v3_t = { 0, 0, 0 };\n    loft_text *v4_l = &v3_t;\n";
        let out = split(&body(link, &read("v4_l->len"))).expect("split");
        assert!(out.contains("struct n_f__fr__frame {\n    int64_t v9_p;\n    int64_t v0_a;\n    loft_text v3_t;\n"), "{out}");
    }

    #[test]
    fn a_goto_and_its_label_share_a_part() {
        let b = body(
            "    if (loft_c(v0_a)) {\n        goto out_1;\n    }\n",
            "    out_1:;\n",
        );
        let out = split(&b).expect("split");
        let g = out.find("goto out_1").expect("the goto");
        let l = out.find("out_1:;").expect("the label");
        assert!(g < l && !out[g..l].contains("\n}\n"), "{out}");
    }

    #[test]
    fn a_body_it_cannot_read_stays_whole() {
        let jump = "    if (setjmp(loft_jb)) {\n        v0_a = loft_one();\n    }\n";
        assert!(split(&body(jump, "")).is_none());
        assert!(split(&body("#ifdef LOFT_X\n    v0_a = loft_one();\n#endif\n", "")).is_none());
        let lost = "    if (loft_c(v0_a)) {\n        goto nowhere;\n    }\n";
        assert!(split(&body(lost, "")).is_none());
        let inline = split_at(3, "static inline", "void", "n_f__fr", "void", &body("", ""));
        assert!(inline.is_none());
    }

    #[test]
    fn a_static_slot_moves_out_under_the_functions_name() {
        let slot = "    static LOFT_TLS loft_text loft_wp7_1 = { 0, 0, 0 };\n    \
                    loft_text v2_w = loft_wpark_take(&loft_wp7_1);\n";
        let out = split(&body(slot, &read("v2_w.len"))).expect("split");
        let moved = "static LOFT_TLS loft_text n_f__fr__loft_wp7_1 = { 0, 0, 0 };\n";
        assert!(out.starts_with(moved), "{out}");
        assert!(
            out.contains("loft_fr->v2_w = loft_wpark_take(&n_f__fr__loft_wp7_1);"),
            "{out}"
        );
    }

    #[test]
    fn a_macro_initializer_stays_an_initializer() {
        // `LOFT_HH_NONE` expands to a brace list: `x = { … };` is no C, so the moved
        // declaration initializes a temporary and assigns that.
        let decl = "    loft_hh hh6 = LOFT_HH_NONE;\n";
        let out = split(&body(decl, &read("hh6.rec"))).expect("split");
        assert!(
            out.contains("{ loft_hh loft_split_tmp = LOFT_HH_NONE; loft_fr->hh6 = loft_split_tmp; }"),
            "{out}"
        );
        assert!(!out.contains("loft_fr->hh6 = LOFT_HH_NONE;"), "{out}");
    }

    #[test]
    fn several_declarators_on_a_line_each_join_the_frame() {
        let decl = "    int vg = 0; int64_t vg_f = 0, vg_l = 0;\n";
        let out = split(&body(decl, &read("vg + vg_f + vg_l"))).expect("split");
        assert!(
            out.contains("    int vg;\n    int64_t vg_f;\n    int64_t vg_l;\n"),
            "{out}"
        );
        assert!(
            out.contains("loft_fr->vg = 0; loft_fr->vg_f = 0; loft_fr->vg_l = 0;"),
            "{out}"
        );
    }
}
