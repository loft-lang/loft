// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C

//! `@FR-R-CTail`: the releases a function runs before a `return` are written once.
//!
//! Every exit of a function releases what the function still owns: the scope pass's frees,
//! then the writer's own epilogue (its texts, its parked work buffers).  Written at each
//! `return`, that list makes a body grow as exits × owned locals instead of with its source:
//! the loft-written elaborator's `assignment` held 99 returns of about 650 release lines each.
//!
//! This pass reads a finished body and keeps ONE copy of each release tail that two exits
//! share.  An exit's tail is the unbroken run of release statements straight before its
//! `return`.  Where the last `MIN_SHARED` or more statements of a tail are, text for text,
//! the end of a tail already written, the exit keeps the statements of its own that differ
//! and jumps into the written one:
//!
//! ```c
//!     loft_free_var(&v9_own);          /* this exit's own */
//!     loft_rv = v2_a;
//!     goto loft_rt3;                   /* the rest, written at an earlier exit */
//! ```
//!
//! The statements each exit runs, and their order, are the ones it ran before: the pass
//! changes where they are written, never which run.  So it holds in every tier and needs no
//! analysis of what a release does.
//!
//! What it relies on, each checked per exit, and an exit that fails one is written as it was:
//! - a tail statement names only what the function declares at its top level — a name a
//!   nested block declares is another object in another block;
//! - the statement before the tail is complete (it ends `;`, `{` or `}`), so no brace-less
//!   `if` or `else` governs the tail's first statement;
//! - the value returned is a plain name no release in the tail writes.  It is read into the
//!   function's one result slot `loft_rv` where the exit read it — at its own place in the
//!   tail when the tail assigns it, before the tail when it was assigned earlier.
//!
//! A body with a preprocessor line is left whole.  `LOFT_NO_C_TAIL=1` turns the pass off (the
//! bisect step); `LOFT_TRACE_C_TAIL=1` names each function it shortened and by how much.
//! `LOFT_PROBE_C_TAIL=1` writes the instrument instead ([`probe`]): which store variables an
//! exit leaves unreleased while they hold a store.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

/// The shortest shared end worth a jump: below it a small function keeps straight-line exits.
const MIN_SHARED: usize = 8;

/// A tail shorter than this is never read: it cannot share `MIN_SHARED` statements.
const MIN_TAIL: usize = MIN_SHARED - 1;

fn off() -> bool {
    crate::env_once!(std::env::var("LOFT_NO_C_TAIL").is_ok_and(|v| v != "0"))
}

fn trace() -> bool {
    crate::env_once!(std::env::var("LOFT_TRACE_C_TAIL").is_ok_and(|v| v != "0"))
}

fn ident_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn is_ident(s: &str) -> bool {
    !s.is_empty() && !s.as_bytes()[0].is_ascii_digit() && s.bytes().all(ident_char)
}

/// The identifiers of `s`, outside string and character literals.
fn idents(s: &str) -> Vec<&str> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' || c == b'\'' {
            i += 1;
            while i < b.len() && b[i] != c {
                i += if b[i] == b'\\' { 2 } else { 1 };
            }
            i += 1;
        } else if ident_char(c) {
            let from = i;
            while i < b.len() && ident_char(b[i]) {
                i += 1;
            }
            if !b[from].is_ascii_digit() {
                out.push(&s[from..i]);
            }
        } else {
            i += 1;
        }
    }
    out
}

/// The release statements a tail is made of: one call, one line, no block of its own.
const RELEASES: [&str; 4] = [
    "loft_free_var(&",
    "loft_free_var_distinct(&",
    "loft_text_free(&",
    "loft_wpark_put(&",
];

fn is_release(t: &str) -> bool {
    t.ends_with(");") && !t.contains('{') && RELEASES.iter().any(|r| t.starts_with(r))
}

/// The name a nested declaration line declares: `<type words> <name>` then `=`, `;` or `[`.
/// A statement starts with ONE identifier before its operator (`t5 = …`, `f(…)`), a
/// declaration with two or more; the keywords that also do are named.
fn nested_declared(t: &str) -> Option<&str> {
    let end = t.find(['=', ';', '[', '(']).unwrap_or(t.len());
    if t[end..].starts_with('(') {
        return None;
    }
    let words: Vec<&str> = t[..end]
        .split(|c: char| c.is_whitespace() || c == '*')
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() < 2 || !words.iter().all(|w| is_ident(w)) {
        return None;
    }
    if matches!(words[0], "return" | "goto" | "else" | "case" | "do") {
        return None;
    }
    words.last().copied()
}

/// What `return x;` returns: `Some("")` for a bare `return;`, `None` for any other line.
fn returned(t: &str) -> Option<&str> {
    if t == "return;" {
        return Some("");
    }
    let x = t.strip_prefix("return ")?.strip_suffix(';')?;
    is_ident(x).then_some(x)
}

/// One exit: the lines `[from, ret]` of the body, and its statements in the shared form.
struct Exit {
    from: usize,
    ret: usize,
    /// The statements from the first of the tail to the `return`, result slot named.
    stmts: Vec<String>,
}

/// The exit ending at the `return` on line `ret`, when it may share its tail.
fn exit_at(lines: &[&str], ret: usize, nested: &HashSet<&str>) -> Option<Exit> {
    let x = returned(lines[ret].trim())?;
    let assign = format!("{x} = ");
    let mut from = ret;
    let mut assigned = false;
    while from > 0 {
        let t = lines[from - 1].trim();
        // The tail's own read of the result: `<x> = <name>;`, once.
        let reads = !x.is_empty()
            && !assigned
            && t.strip_prefix(assign.as_str())
                .and_then(|r| r.strip_suffix(';'))
                .is_some_and(is_ident);
        if !reads && !is_release(t) {
            break;
        }
        assigned |= reads;
        from -= 1;
    }
    if ret - from < MIN_TAIL {
        return None;
    }
    // The statement before the tail is complete: nothing brace-less governs the tail.
    if from > 0 {
        let before = lines[from - 1].trim();
        if !(before.ends_with(';') || before.ends_with('{') || before.ends_with('}')) {
            return None;
        }
    }
    let writes = format!("(&{x}");
    let mut stmts = Vec::with_capacity(ret - from + 2);
    for l in &lines[from..ret] {
        let t = l.trim();
        if idents(t).iter().any(|i| nested.contains(i)) {
            return None;
        }
        if let Some(value) = t.strip_prefix(assign.as_str()).filter(|_| !x.is_empty()) {
            stmts.push(format!("loft_rv = {value}"));
        } else {
            // A release that writes the returned name: the exit stays as written.
            if !x.is_empty()
                && t.find(writes.as_str()).is_some_and(|p| {
                    !t.as_bytes()
                        .get(p + writes.len())
                        .copied()
                        .is_some_and(ident_char)
                })
            {
                return None;
            }
            stmts.push(t.to_string());
        }
    }
    if x.is_empty() {
        stmts.push("return;".to_string());
    } else {
        if !assigned {
            // Assigned before the tail: read where the tail starts, which no release changes.
            stmts.insert(0, format!("loft_rv = {x};"));
        }
        stmts.push("return loft_rv;".to_string());
    }
    Some(Exit { from, ret, stmts })
}

/// `body` with each shared release tail written once, or `None` when nothing is shared.
/// `ret` is the C type the function returns (unused for a `void` one).
pub(crate) fn merge(name: &str, ret: &str, body: &str) -> Option<String> {
    if off() {
        return None;
    }
    if crate::env_once!(std::env::var("LOFT_PROBE_C_TAIL").is_ok()) {
        return probe(name, body);
    }
    let lines: Vec<&str> = body.lines().collect();
    if lines.iter().any(|l| l.trim_start().starts_with('#')) {
        return None;
    }
    // Names a nested block declares: the top level is one indentation step in.
    let nested: HashSet<&str> = lines
        .iter()
        .filter(|l| l.starts_with("     "))
        .filter_map(|l| nested_declared(l.trim()))
        .collect();
    // The written tails, as a trie from the `return` backwards: (parent, statement) -> node,
    // and per node the output line holding its statement.
    let mut nodes: HashMap<(usize, String), usize> = HashMap::new();
    let mut at: Vec<usize> = vec![usize::MAX];
    let mut labels: HashMap<usize, usize> = HashMap::new();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut next = 0;
    let mut saved = 0;
    for ret_line in 0..lines.len() {
        if returned(lines[ret_line].trim()).is_none() {
            continue;
        }
        let Some(exit) = exit_at(&lines, ret_line, &nested) else {
            continue;
        };
        if exit.from < next {
            continue;
        }
        out.extend(lines[next..exit.from].iter().map(|l| (*l).to_string()));
        next = exit.ret + 1;
        let indent = &lines[exit.ret][..lines[exit.ret].len() - lines[exit.ret].trim_start().len()];
        // The longest end of this tail already written.
        let mut node = 0;
        let mut shared = 0;
        for s in exit.stmts.iter().rev() {
            match nodes.get(&(node, s.clone())) {
                Some(&n) => {
                    node = n;
                    shared += 1;
                }
                None => break,
            }
        }
        if shared < MIN_SHARED {
            node = 0;
            shared = 0;
        }
        let own = exit.stmts.len() - shared;
        // Its own statements, each a new node under the shared end, innermost last.
        let first = out.len();
        for s in &exit.stmts[..own] {
            out.push(format!("{indent}{s}"));
        }
        let mut parent = node;
        for (k, s) in exit.stmts[..own].iter().enumerate().rev() {
            let id = at.len();
            at.push(first + k);
            // A statement this exit repeats keeps the node of its first writing.
            parent = *nodes.entry((parent, s.clone())).or_insert(id);
        }
        if shared > 0 {
            let n = labels.len();
            let label = *labels.entry(at[node]).or_insert(n);
            out.push(format!("{indent}goto loft_rt{label};"));
            saved += shared - 1;
        }
    }
    if labels.is_empty() {
        return None;
    }
    out.extend(lines[next..].iter().map(|l| (*l).to_string()));
    let mut text = String::with_capacity(body.len());
    if out.iter().any(|l| l.contains("loft_rv")) {
        let _ = writeln!(text, "    {ret} loft_rv;");
    }
    for (k, l) in out.iter().enumerate() {
        if let Some(label) = labels.get(&k) {
            let _ = writeln!(text, "loft_rt{label}:");
        }
        text.push_str(l);
        text.push('\n');
    }
    if trace() {
        eprintln!(
            "[c-tail]\t{name}\t{} -> {} lines, {} shared end(s), {saved} statement(s) not repeated",
            lines.len(),
            out.len() + labels.len(),
            labels.len()
        );
    }
    Some(text)
}

/// The store variable a release statement frees (`loft_free_var(&x)`, `…_distinct(&x, w)`).
fn freed_ref(t: &str) -> Option<&str> {
    let r = t
        .strip_prefix("loft_free_var(&")
        .or_else(|| t.strip_prefix("loft_free_var_distinct(&"))?;
    let end = r.find([',', ')'])?;
    is_ident(&r[..end]).then_some(&r[..end])
}

/// `LOFT_PROBE_C_TAIL=1` — the instrument for "an exit that does not release a store variable
/// finds it unset": before each exit's tail, every store variable some OTHER exit of the
/// function releases and this one does not is tested, and one that holds a store is reported
/// on stderr as `[tail-held]\t<fn>\t<var>\t<exit>\t<live>` (`0`: the store was freed already).  The body is otherwise as written.
fn probe(name: &str, body: &str) -> Option<String> {
    let lines: Vec<&str> = body.lines().collect();
    let mut exits: Vec<(usize, usize)> = Vec::new();
    for (ret, l) in lines.iter().enumerate() {
        if returned(l.trim()).is_none() {
            continue;
        }
        let mut from = ret;
        while from > 0 {
            let t = lines[from - 1].trim();
            let reads = t
                .split_once(" = ")
                .is_some_and(|(a, b)| is_ident(a) && b.strip_suffix(';').is_some_and(is_ident));
            if !reads && !is_release(t) {
                break;
            }
            from -= 1;
        }
        let complete = from == 0 || {
            let b = lines[from - 1].trim();
            b.ends_with(';') || b.ends_with('{') || b.ends_with('}')
        };
        if complete {
            exits.push((from, ret));
        }
    }
    let nested: HashSet<&str> = lines
        .iter()
        .filter(|l| l.starts_with("     "))
        .filter_map(|l| nested_declared(l.trim()))
        .collect();
    let mut all: Vec<&str> = Vec::new();
    for &(from, ret) in &exits {
        for l in &lines[from..ret] {
            if let Some(v) = freed_ref(l.trim())
                && !all.contains(&v)
                && !nested.contains(v)
            {
                all.push(v);
            }
        }
    }
    if exits.len() < 2 || all.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(body.len() * 2);
    let mut next = 0;
    for (k, &(from, ret)) in exits.iter().enumerate() {
        for l in &lines[next..from] {
            out.push_str(l);
            out.push('\n');
        }
        next = from;
        let here: Vec<&str> = lines[from..ret]
            .iter()
            .filter_map(|l| freed_ref(l.trim()))
            .collect();
        for v in all.iter().filter(|v| !here.contains(v)) {
            let _ = writeln!(
                out,
                "    if ({v}.store_nr != 0xFFFF) fprintf(stderr, \"[tail-held]\\t{name}\\t{v}\\t{k}\\t%d\\n\", (int) ({v}.store_nr < loft_nstores ? (int) loft_stores[{v}.store_nr].live : -1));"
            );
        }
    }
    for l in &lines[next..] {
        out.push_str(l);
        out.push('\n');
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tail(n: usize) -> String {
        let mut t = String::new();
        for k in 0..n {
            let _ = writeln!(t, "        loft_free_var(&v{k}_x);");
        }
        t
    }

    #[test]
    fn a_shared_tail_is_written_once() {
        let body = format!(
            "    if (a) {{\n{t}        t1 = v9_r;\n        loft_text_free(&t7);\n        return t1;\n    }}\n    if (b) {{\n        loft_free_var(&v99_own);\n{t}        t2 = v8_r;\n        loft_text_free(&t7);\n        return t2;\n    }}\n{t}    t3 = v9_r;\n    loft_text_free(&t7);\n    return t3;\n",
            t = tail(10)
        );
        let m = merge("f", "loft_ref", &body).expect("merged");
        // The first exit is written whole and the third shares all of it; the second returns
        // another value, read mid-tail, so it shares too little and is whole as well.
        assert_eq!(m.matches("loft_free_var(&v0_x);").count(), 2, "{m}");
        assert_eq!(m.matches("loft_text_free(&t7);").count(), 2, "{m}");
        assert_eq!(m.matches("return loft_rv;").count(), 2, "{m}");
        assert!(m.contains("loft_rv = v8_r;") && m.contains("loft_free_var(&v99_own);"));
        assert!(m.starts_with("    loft_ref loft_rv;\n"));
    }

    #[test]
    fn a_tail_under_a_braceless_if_or_naming_a_nested_local_stays() {
        let t = tail(10);
        let braceless = format!("{t}    return;\n    if (a)\n{t}    return;\n");
        let m = merge("f", "void", &braceless);
        assert!(m.is_none(), "{m:?}");
        let nested = format!(
            "    {{\n        loft_ref v0_x = q;\n{t}        return;\n    }}\n{t}    return;\n"
        );
        assert!(merge("f", "void", &nested).is_none());
    }

    #[test]
    fn a_release_of_the_returned_name_keeps_the_exit() {
        let t = tail(10);
        let body = format!("{t}    return v3_x;\n{t}    return v3_x;\n");
        assert!(merge("f", "loft_ref", &body).is_none());
        let other = format!("{t}    return v77_r;\n{t}    return v77_r;\n");
        let m = merge("f", "loft_ref", &other).expect("merged");
        // The second exit shares all of the first, its read of the result included.
        assert_eq!(m.matches("loft_rv = v77_r;").count(), 1, "{m}");
        assert_eq!(m.matches("goto loft_rt0;").count(), 1, "{m}");
        assert_eq!(m.matches("loft_free_var(&v0_x);").count(), 1, "{m}");
    }
}
