// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native Rust generator: a second compiled target, C
//! The written body of one C function, without the names it declares and never uses.
//!
//! The writer declares every local of the loft function and a label for every loop, then writes
//! the statements; a rewrite that removes the last use of a local, or a loop no `continue`
//! leaves early, leaves the declaration or the label behind, which the C compiler reports
//! (`-Wunused-variable`, `-Wunused-label`, …).  A warning can mean a wrong program, so the
//! written C carries none (`scripts/c_lint_census.py`).  This pass reads the body the writer
//! produced and drops only what nothing names: a label no `goto` names; a top-level declaration
//! whose name appears nowhere else and whose initializer has no effect; a local that is only
//! ever assigned, whose assignments keep their right-hand side as `(void) (…);`.  A parameter
//! the body never names is marked `(void)`.  Nothing it removes is read, so the program does
//! what it did.

use std::collections::HashMap;

/// Every identifier in `body` with how often it occurs, string and character literals and
/// comments skipped.
fn identifier_counts(body: &str) -> HashMap<&str, usize> {
    let b = body.as_bytes();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'"' || c == b'\'' {
            let q = c;
            i += 1;
            while i < b.len() && b[i] != q {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            i += 1;
        } else if c == b'/' && b.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == b'*' && b[i + 1] == b'/') {
                i += 1;
            }
            i += 2;
        } else if c == b'/' && b.get(i + 1) == Some(&b'/') {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            *counts.entry(&body[s..i]).or_insert(0) += 1;
        } else if c.is_ascii_digit() {
            // A number, its suffix and exponent included, is no identifier.
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.') {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    counts
}

fn is_ident(s: &str) -> bool {
    let mut c = s.chars();
    c.next()
        .is_some_and(|f| f.is_ascii_alphabetic() || f == '_')
        && c.all(|x| x.is_ascii_alphanumeric() || x == '_')
}

/// A label alone on its line (`cont_3:;`, or `loft_gen_end:` before its statement): its name.
fn label_of(line: &str) -> Option<&str> {
    let t = line.trim();
    let name = t.strip_suffix(":;").or_else(|| t.strip_suffix(':'))?;
    (is_ident(name) && name != "default").then_some(name)
}

/// Is `t` a C type the writer declares a local as?
fn local_type(t: &str) -> bool {
    matches!(
        t,
        "int64_t"
            | "int32_t"
            | "int16_t"
            | "int8_t"
            | "uint64_t"
            | "uint32_t"
            | "uint16_t"
            | "uint8_t"
            | "double"
            | "float"
            | "int"
            | "size_t"
    ) || t.starts_with("loft_")
        || t.starts_with("lt_")
}

/// A declaration alone on its line (`T name = init;` or `T name;`, at any indent): the name,
/// the initializer, and whether it is at the function's top level (four spaces).
fn declaration_at(line: &str) -> Option<(&str, Option<&str>, bool)> {
    let rest = line.trim_start_matches(' ');
    let top = line.len() - rest.len() == 4;
    if rest.is_empty() || !rest.ends_with(';') || line.len() == rest.len() {
        return None;
    }
    let rest = &rest[..rest.len() - 1];
    let (head, init) = match rest.split_once(" = ") {
        Some((h, i)) => (h, Some(i)),
        None => (rest, None),
    };
    // `T name`, `T *name` or `T * name`: the pointer belongs to the declarator.
    let (ty, name) = head.rsplit_once(' ')?;
    let name = name.trim_start_matches('*');
    let ty = ty.trim_end_matches(['*', ' ']);
    (local_type(ty) && is_ident(name)).then_some((name, init, top))
}

/// A top-level declaration: [`declaration_at`] at four spaces.
fn declaration(line: &str) -> Option<(&str, Option<&str>)> {
    declaration_at(line).filter(|d| d.2).map(|(n, i, _)| (n, i))
}

/// Does evaluating `init` do nothing but produce a value?  Literals, names, braces, and the
/// writer's constant spellings; any other call may have an effect.
fn pure(init: &str) -> bool {
    let mut s = init.to_string();
    for keep in [
        "loft_ref_null()",
        "(double) NAN",
        "(float) NAN",
        "INT64_C(",
        "UINT64_C(",
        "INT32_C(",
    ] {
        s = s.replace(keep, " ");
    }
    !s.contains('(') && !s.contains('"') && !s.contains('\'')
}

/// An assignment statement alone on its line (`    name = rhs;`, or to a member of it,
/// `name.f0 = rhs;`): the name, the indent and the right-hand side.
fn assignment(line: &str) -> Option<(&str, &str, &str)> {
    let body = line.trim_start();
    let indent = &line[..line.len() - body.len()];
    let body = body.strip_suffix(';')?;
    let (target, rhs) = body.split_once(" = ")?;
    let name = target.split('.').next()?;
    let members_ok = target.split('.').skip(1).all(is_ident);
    (is_ident(name) && members_ok && !rhs.contains(" = ")).then_some((name, indent, rhs))
}

/// `body` (the lines between the function's braces) without the labels, locals and
/// assignments nothing reads, and with every parameter of `params` (`"T name"` each) the body
/// never names marked `(void)`.  Repeated until nothing changes: a local dropped may have been
/// the last reader of another.
pub(crate) fn tidy(params: &[String], body: &str) -> String {
    let mut now = body.to_string();
    for _ in 0..8 {
        let next = tidy_once(params, &now);
        if next == now {
            break;
        }
        now = next;
    }
    now
}

fn tidy_once(params: &[String], body: &str) -> String {
    let counts = identifier_counts(body);
    let n = |s: &str| counts.get(s).copied().unwrap_or(0);
    let lines: Vec<&str> = body.split('\n').collect();
    // The locals written but never read: every occurrence beyond the declaration is the target
    // of a whole-line assignment whose right-hand side does not name it.
    let mut assigned: HashMap<&str, usize> = HashMap::new();
    for l in &lines {
        if let Some((name, _, rhs)) = assignment(l)
            && !identifier_counts(rhs).contains_key(name)
        {
            *assigned.entry(name).or_insert(0) += 1;
        }
    }
    let mut set_only: Vec<&str> = Vec::new();
    // A parameter the body only ever assigns: its value is never read.
    for p in params {
        if let Some((_, name)) = p.rsplit_once(' ') {
            let name = name.trim_start_matches('*');
            if is_ident(name) && n(name) > 0 && assigned.get(name).copied().unwrap_or(0) == n(name)
            {
                set_only.push(name);
            }
        }
    }
    for l in &lines {
        if let Some((name, init)) = declaration(l)
            && init.is_none_or(pure)
            && !l.contains("volatile")
            && n(name) > 1
            && assigned.get(name).copied().unwrap_or(0) + 1 == n(name)
        {
            set_only.push(name);
        }
    }
    let mut out = String::with_capacity(body.len());
    for p in params {
        if let Some((_, name)) = p.rsplit_once(' ') {
            let name = name.trim_start_matches('*');
            if is_ident(name) && n(name) == 0 && !body.contains(&format!("(void) {name};")) {
                out.push_str("    (void) ");
                out.push_str(name);
                out.push_str(";\n");
            }
        }
    }
    for (i, l) in lines.iter().enumerate() {
        let last = i + 1 == lines.len();
        // A local the whole body names once is unused wherever it is declared; one only ever
        // assigned is dropped at the top level alone, where its name is the function's.
        let keep = if let Some(label) = label_of(l) {
            n(label) > 1
        } else if let Some((name, init, top)) = declaration_at(l) {
            !(init.is_none_or(pure)
                && !l.contains("volatile")
                && (n(name) == 1 || (top && set_only.contains(&name))))
        } else {
            true
        };
        if !keep {
            continue;
        }
        if let Some((name, indent, rhs)) = assignment(l)
            && set_only.contains(&name)
        {
            // Nothing reads the local: the value is not kept, its computation still runs.
            if !pure(rhs) {
                out.push_str(indent);
                out.push_str("(void) (");
                out.push_str(rhs);
                out.push_str(");");
                if !last {
                    out.push('\n');
                }
            }
            continue;
        }
        out.push_str(l);
        if !last {
            out.push('\n');
        }
    }
    out
}

/// The written part of a unit (everything after the runtime) with every function body tidied
/// ([`tidy`]) — the program's functions, their twins and dispatchers, generator resumes and the
/// render functions alike.  A function is a line at column 0 that ends in `) {` and the next
/// line that is `}` alone; anything else (a table, a type) is passed over as it is.
pub(crate) fn tidy_unit(written: &str) -> String {
    let lines: Vec<&str> = written.split('\n').collect();
    let mut out = String::with_capacity(written.len());
    let mut i = 0;
    let push = |out: &mut String, l: &str, more: bool| {
        out.push_str(l);
        if more {
            out.push('\n');
        }
    };
    while i < lines.len() {
        let l = lines[i];
        let signature = !l.starts_with(char::is_whitespace)
            && l.ends_with(") {")
            && l.contains('(')
            && !l.starts_with('#');
        if signature && let Some(end) = (i + 1..lines.len()).find(|&k| lines[k] == "}") {
            let open = l.find('(').unwrap_or(0);

            let close = l.rfind(')').unwrap_or(l.len());
            let params: Vec<String> = l[open + 1..close]
                .split(", ")
                .filter(|p| *p != "void" && !p.is_empty() && !p.contains('('))
                .map(str::to_string)
                .collect();
            let body = lines[i + 1..end].join("\n");
            push(&mut out, l, true);
            let tidied = tidy(&params, &format!("{body}\n"));
            out.push_str(&tidied);
            push(&mut out, "}", end + 1 < lines.len());
            i = end + 1;
            continue;
        }
        push(&mut out, l, i + 1 < lines.len());
        i += 1;
    }
    out
}

/// The name a column-0 line defines or declares: a function (`static T name(…) {` or its
/// prototype `…);`), or a one-line table (`static const T name[…] = …;`).
fn defined_name(l: &str) -> Option<(&str, bool)> {
    if !l.starts_with("static ") {
        return None;
    }
    if let Some(open) = l.find('(')
        && (l.ends_with(") {") || l.ends_with(");"))
    {
        let name = l[..open].rsplit([' ', '*']).next()?;
        return is_ident(name).then_some((name, l.ends_with(") {")));
    }
    if l.starts_with("static const ") && l.ends_with(';') {
        let name = l.split_once('[')?.0.rsplit(' ').next()?;
        return is_ident(name).then_some((name, false));
    }
    None
}

/// The finished program (`main` included) without the written functions and one-line tables
/// nothing names: a twin the writer declined, a default function no constructor calls, a
/// default image or key table no code reads.  Only the part after `runtime_end` is touched;
/// the whole text is what names them.  Repeated until nothing changes, since a function dropped
/// may have been its callee's last caller.
pub(crate) fn drop_unnamed(program: &str, runtime_end: &str) -> String {
    let Some(at) = program.find(runtime_end).map(|a| a + runtime_end.len()) else {
        return program.to_string();
    };
    let (runtime, mut written) = (program[..at].to_string(), program[at..].to_string());
    for _ in 0..8 {
        let all = format!("{runtime}{written}");
        let counts = identifier_counts(&all);
        let lines: Vec<&str> = written.split('\n').collect();
        // How many lines of the program define or declare each name: those are no use of it.
        let mut decls: HashMap<&str, usize> = HashMap::new();
        for l in all.split('\n') {
            if let Some((name, _)) = defined_name(l) {
                *decls.entry(name).or_insert(0) += 1;
            }
        }
        let unused = |name: &str| {
            name != "main"
                && counts.get(name).copied().unwrap_or(0) <= decls.get(name).copied().unwrap_or(0)
        };
        let mut out = String::with_capacity(written.len());
        let mut changed = false;
        let mut i = 0;
        while i < lines.len() {
            let l = lines[i];
            let next = |out: &mut String, l: &str, i: usize| {
                out.push_str(l);
                if i + 1 < lines.len() {
                    out.push('\n');
                }
            };
            match defined_name(l) {
                Some((name, true)) if unused(name) => {
                    changed = true;
                    let end = (i + 1..lines.len()).find(|&k| lines[k] == "}").unwrap_or(i);
                    i = end + 1;
                    // The blank line the function was written after goes with it.
                    if out.ends_with("\n\n") {
                        out.pop();
                    }
                }
                Some((name, false)) if unused(name) => {
                    changed = true;
                    i += 1;
                }
                _ => {
                    next(&mut out, l, i);
                    i += 1;
                }
            }
        }
        written = out;
        if !changed {
            break;
        }
    }
    format!("{runtime}{written}")
}

#[cfg(test)]
mod tests {
    use super::tidy;

    #[test]
    fn an_unused_label_and_local_go_and_a_used_one_stays() {
        let body = "    loft_ref v1___retbuf = { 0xFFFF, 0, 0, 0 };\n    int64_t v0_a = LOFT_NULL;\n    \
                    for (;;) {\n        v0_a = 3;\n        if (v0_a > 2) goto brk_1;\n    cont_1:;\n    }\n    \
                    brk_1:;\n    return v0_a;\n";
        let t = tidy(&[], body);
        assert!(!t.contains("v1___retbuf"), "{t}");
        assert!(!t.contains("cont_1"), "{t}");
        assert!(t.contains("brk_1:;") && t.contains("int64_t v0_a"), "{t}");
    }

    #[test]
    fn a_write_only_local_keeps_its_effect() {
        let body = "    int64_t v2__discard_1 = LOFT_NULL;\n    v2__discard_1 = n_f(3);\n    v2__discard_1 = 4;\n";
        let t = tidy(&[], body);
        assert_eq!(t, "    (void) (n_f(3));\n");
    }

    #[test]
    fn an_initializer_with_an_effect_is_kept() {
        let body = "    loft_text v1_s = loft_text_from(n_g());\n    return 0;\n";
        assert_eq!(tidy(&[], body), body);
    }

    #[test]
    fn a_name_in_a_string_or_comment_is_no_use() {
        let body = "    int64_t v0_x = 0;\n    /* v0_x */ n_p(\"v0_x\");\n";
        assert_eq!(tidy(&[], body), "    /* v0_x */ n_p(\"v0_x\");\n");
    }

    #[test]
    fn a_function_and_a_table_nothing_names_are_dropped() {
        let p = "RT\n/*end*/\nstatic const uint8_t loft_dflt4[2] = { 0, 0 };\n\
                 static int n_a(void);\nstatic int n_b(void);\n\
                 static int n_a(void) {\n    return n_b();\n}\nstatic int n_b(void) {\n    return 1;\n}\n\
                 static int n_c(void) {\n    return 2;\n}\nint main(void) {\n    return n_c();\n}\n";
        let t = super::drop_unnamed(p, "/*end*/\n");
        assert!(
            !t.contains("loft_dflt4") && !t.contains("n_a") && !t.contains("n_b"),
            "{t}"
        );
        assert!(
            t.contains("static int n_c(void) {") && t.contains("int main"),
            "{t}"
        );
    }

    #[test]
    fn a_parameter_only_assigned_keeps_its_effect() {
        let t = tidy(
            &["int64_t v0_b".into()],
            "    v0_b = n_f(1);\n    return 0;\n",
        );
        assert_eq!(t, "    (void) v0_b;\n    (void) (n_f(1));\n    return 0;\n");
    }

    #[test]
    fn a_pointer_local_nothing_reads_goes() {
        let body = "    lt_103 * v2___ref_p2_1 = 0;\n    uint8_t *loft_pa0_0 = 0;\n    v2___ref_p2_1 = 0;\n    return 1;\n";
        assert_eq!(tidy(&[], body), "    return 1;\n");
    }

    #[test]
    fn an_unused_parameter_is_marked() {
        let t = tidy(
            &["loft_ref v0_t".into(), "loft_vh hi0".into()],
            "    return v0_t;\n",
        );
        assert_eq!(t, "    (void) hi0;\n    return v0_t;\n");
    }
}
