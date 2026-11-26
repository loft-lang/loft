// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I68 — Native generators: one written C unit cut into the units of a split build
//! @PLN229 B1 — one written unit cut into the units of a split build.
//!
//! The writer writes the unit it always writes, with each function's body marked with the file
//! it is declared in (`SPLIT_FN`) and the start of what no file owns marked (`SPLIT_SHARED`).
//! This cuts it: every unit starts with the runtime's hot part, then the head items its own code
//! names — the types, tables, constants and macros, followed through what each of those names —
//! and then the prototypes its own functions call, so a unit's text changes only with what it
//! uses.  One shared unit carries what no file owns (the dispatchers, the drop cascades, the
//! generated twins, the tables, `main`) and every file gets a unit with its own functions.
//!
//! A function defined in one unit and named from another (or from the head) needs external
//! linkage, so it loses its `static` / `inline` / `LOFT_HOT_INLINE`, at its definition and at
//! its prototype.  Every other function keeps its storage: the C compiler inlines it and drops
//! what nothing calls, as in the one-unit build.  The runtime keeps its own: every unit carries
//! the runtime's hot part (`split_runtime`; the cold part is one unit of its own, the split
//! build's exception to `@C169`), and its state is declared once through `LOFT_STATE` — the shared unit is
//! compiled with `LOFT_SPLIT_OWN`, every other with `LOFT_SPLIT_USE`.  A file-scope variable a
//! body moved out (a work-text slot) keeps its `static`: it belongs to its function's unit.

use super::{SPLIT_FN, SPLIT_FN_END, SPLIT_SHARED, SPLIT_UNIT};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fmt::Write as _;

/// The storage words a function the units define drops (`static`, `inline`, the hot-helper
/// macro, which is `static inline` with a hint).
const STORAGE: [&str; 3] = ["static ", "inline ", "LOFT_HOT_INLINE "];

/// `line` without the leading storage words of [`STORAGE`].
fn without_storage(line: &str) -> &str {
    let mut rest = line;
    loop {
        match STORAGE.iter().find(|w| rest.starts_with(**w)) {
            Some(w) => rest = &rest[w.len()..],
            None => return rest,
        }
    }
}

/// The name a column-0 function line declares or defines, when it is one: the identifier before
/// the first `(`, with nothing that assigns before it (a variable's initializer is no function).
fn function_name(line: &str) -> Option<&str> {
    if line.starts_with([' ', '\t', '#', '/', '}', '{']) || line.is_empty() {
        return None;
    }
    let paren = line.find('(')?;
    // `T (*name)(…)` is a variable holding a function, not a function
    if line[paren + 1..].starts_with('*') {
        return None;
    }
    let head = &line[..paren];
    if head.contains('=') || head.contains('"') {
        return None;
    }
    let name = head
        .trim_end()
        .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .next()?;
    (!name.is_empty() && !name.starts_with(|c: char| c.is_ascii_digit())).then_some(name)
}

/// Is `line` the head of a definition (`… f(…) {`) rather than a prototype (`… f(…);`)?
fn is_definition(line: &str) -> bool {
    line.trim_end().ends_with('{')
}

/// `text` with every column-0 line naming a function of `defined` written without storage: a
/// definition head everywhere, a prototype only where `prototypes` says so.
fn give_linkage(text: &str, defined: &HashSet<String>, prototypes: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.split_inclusive('\n') {
        let bare = without_storage(line);
        let rewrite = bare.len() != line.len()
            && function_name(bare).is_some_and(|n| defined.contains(n))
            && (is_definition(bare) || (prototypes && bare.trim_end().ends_with(';')));
        out.push_str(if rewrite { bare } else { line });
    }
    out
}

/// The runtime cut in two (@C169's split-build exception): every unit carries the HOT runtime —
/// its types, its macros and its inline-hinted functions whole — and one unit, compiled once,
/// carries the COLD functions, the plain `static` ones.  At the check tier gcc inlines a plain
/// `static` function only where it is called once, so a unit loses almost nothing by calling it
/// instead; the inline-hinted helpers (store access, references, the frame) stay where the
/// generated code can inline them.
///
/// Returns `(hot, cold, callbacks)`: `hot` is `runtime` with each cold function a prototype,
/// `cold` is `runtime` with each cold function defined with external linkage, and `callbacks`
/// the functions the runtime declares and the program defines (`loft_init`): the cold unit calls
/// them, so they are declared external in both halves and the program's definition is too.  A function head is one
/// column-0 line ending in `{` and its body ends at the next column-0 `}`; a definition the writer
/// spells otherwise keeps its `static` in every unit, which costs time, never correctness.
fn split_runtime(runtime: &str) -> (String, String, HashSet<String>) {
    let lines: Vec<&str> = runtime.split_inclusive('\n').collect();
    // A `static` function that is not inline-hinted.  Its return type may start with `const` or
    // `struct` (`static const loft_tdesc *loft_parse_types(loft_cx *cx);`, a callback): a table
    // or a variable is told apart by `function_name`, which finds no function in it.
    let plain = |line: &str| -> Option<String> {
        let rest = line.strip_prefix("static ")?;
        if rest.starts_with(|c: char| c.is_ascii_uppercase()) || rest.starts_with("inline ") {
            return None;
        }
        function_name(rest).map(str::to_string)
    };
    let mut cold_names: HashSet<String> = HashSet::new();
    let mut defined: HashSet<&str> = HashSet::new();
    for line in &lines {
        // a definition of any form: a body opened on its line (`{` at its end, or a whole body
        // on one line) — everything but a prototype
        if !line.trim_end().ends_with(';')
            && let Some(n) = function_name(without_storage(line))
        {
            defined.insert(n);
        }
        if is_definition(line)
            && let Some(n) = plain(line)
        {
            cold_names.insert(n);
        }
    }
    let callbacks: HashSet<String> = lines
        .iter()
        .filter(|l| !is_definition(l) && l.trim_end().ends_with(';'))
        .filter_map(|l| plain(l))
        .filter(|n| !defined.contains(n.as_str()))
        .collect();
    let mut hot = String::with_capacity(runtime.len());
    let mut cold = String::with_capacity(runtime.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let name = plain(line).filter(|n| cold_names.contains(n) || callbacks.contains(n));
        match name {
            Some(_) if is_definition(line) => {
                let bare = &line["static ".len()..];
                let head = bare.trim_end().trim_end_matches('{').trim_end();
                hot.push_str(head);
                hot.push_str(";\n");
                cold.push_str(bare);
                i += 1;
                while i < lines.len() {
                    cold.push_str(lines[i]);
                    let end = lines[i].starts_with('}');
                    i += 1;
                    if end {
                        break;
                    }
                }
                continue;
            }
            Some(_) if line.trim_end().ends_with(';') => {
                // its prototype: external, as its definition now is
                hot.push_str(&line["static ".len()..]);
                cold.push_str(&line["static ".len()..]);
            }
            _ => {
                hot.push_str(line);
                cold.push_str(line);
            }
        }
        i += 1;
    }
    (hot, cold, callbacks)
}

/// Every identifier `text` holds — a token of letters, digits and `_` not starting with a digit.
/// Strings and comments are not skipped: a name only they mention brings a prototype the unit does
/// not need, which costs nothing but its line.
fn identifiers(text: &str) -> HashSet<&str> {
    text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty() && !t.starts_with(|c: char| c.is_ascii_digit()))
        .collect()
}

/// One top-level item of the head after the runtime: a declaration, a definition, a macro.
struct Item {
    text: String,
    /// The names it declares; an item that declares none a unit can name is in every unit.
    defines: Vec<String>,
    /// In every unit: a preprocessor group or line other than a `#define`, or an item whose names
    /// could not be read.
    always: bool,
}

/// The change in brace depth over `line`, outside string and character literals and comments;
/// `comment` carries an open `/* … */` from one line to the next.
fn brace_delta(line: &str, comment: &mut bool) -> i32 {
    let b = line.as_bytes();
    let (mut d, mut i) = (0, 0);
    while i < b.len() {
        if *comment {
            if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                *comment = false;
                i += 1;
            }
        } else {
            match b[i] {
                b'/' if b.get(i + 1) == Some(&b'*') => {
                    *comment = true;
                    i += 1;
                }
                b'/' if b.get(i + 1) == Some(&b'/') => return d,
                q @ (b'"' | b'\'') => {
                    i += 1;
                    while i < b.len() && b[i] != q {
                        if b[i] == b'\\' {
                            i += 1;
                        }
                        i += 1;
                    }
                }
                b'{' => d += 1,
                b'}' => d -= 1,
                _ => {}
            }
        }
        i += 1;
    }
    d
}

/// Is the file-scope variable `decl` (its line after `static `) a constant, which every unit may
/// hold a copy of?  Only when the object itself is `const`: `const int n`, `const char *const p`,
/// or a table (`const char *names[]`, whose rows nothing assigns).  `const loft_tdesc *p` is a
/// MUTABLE pointer to constant data: assigned at start (`loft_init`) in one unit, it would stay
/// null in every other.
fn is_constant(decl: &str) -> bool {
    let head = decl.split(['=', ';']).next().unwrap_or_default();
    if head.contains('[') {
        return head.trim_start().starts_with("const ");
    }
    match head.rfind('*') {
        Some(star) => head[star + 1..].trim_start().starts_with("const "),
        None => head.split_whitespace().any(|w| w == "const"),
    }
}

/// The names declaration `text` declares: a function's name, a variable's, a type's (its
/// `typedef` name and its tag), a macro's, and every name inside an `enum`'s braces (its
/// constants — and possibly more, which only puts the item in more units).
fn declared_names(text: &str) -> Vec<String> {
    let first = text.lines().next().unwrap_or_default();
    if let Some(rest) = first.strip_prefix("#define ") {
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        return if name.is_empty() {
            Vec::new()
        } else {
            vec![name]
        };
    }
    let bare = without_storage(first);
    if let Some(n) = function_name(bare) {
        return vec![n.to_string()];
    }
    let ident = |t: &str| -> Option<String> {
        let n = t
            .trim_end()
            .rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
            .next()?;
        (!n.is_empty() && !n.starts_with(|c: char| c.is_ascii_digit())).then(|| n.to_string())
    };
    let mut names = Vec::new();
    // the tag of a `struct` / `union` / `enum`, with or without `typedef`
    let words: Vec<&str> = text.split_whitespace().take(4).collect();
    if let Some(k) = words
        .iter()
        .position(|w| matches!(*w, "struct" | "union" | "enum"))
        && let Some(tag) = words
            .get(k + 1)
            .and_then(|w| ident(w.trim_end_matches(['{', ';'])))
    {
        names.push(tag);
    }
    let open = text.find('{');
    let close = text.rfind('}');
    if let (Some(o), Some(c)) = (open, close)
        && text[..o].split_whitespace().any(|w| w == "enum")
    {
        names.extend(identifiers(&text[o + 1..c]).into_iter().map(str::to_string));
    }
    // the declarator: after a closing brace (`} Name;`), else before the first `=`, `[` or `;`
    let tail = match (open, close) {
        (Some(o), Some(c)) if !text[..o].contains('=') => &text[c + 1..],
        _ => text,
    };
    let decl = tail.split(['=', '[', ';']).next().unwrap_or_default();
    let decl = match decl.find("(*") {
        Some(p) => decl[p + 2..].split(')').next().unwrap_or_default(),
        None => decl,
    };
    if let Some(n) = ident(decl)
        && !matches!(n.as_str(), "struct" | "union" | "enum")
    {
        names.push(n);
    }
    names
}

/// `text` — the head after the runtime — cut into its top-level items, in order.  A comment or a
/// blank line goes with the item after it.
fn head_items(text: &str) -> Vec<Item> {
    let lines: Vec<&str> = text.split_inclusive('\n').collect();
    let mut items = Vec::new();
    let mut pending = String::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        let t = line.trim();
        if t.is_empty() || t.starts_with("//") || (t.starts_with("/*") && t.ends_with("*/")) {
            pending.push_str(line);
            i += 1;
            continue;
        }
        let mut item = std::mem::take(&mut pending);
        if line.starts_with('#') {
            let mut always = !line.starts_with("#define ");
            if line.starts_with("#if") {
                // a conditional group, whole: whatever it declares, every unit has it
                let mut nest = 0;
                while i < lines.len() {
                    let l = lines[i];
                    item.push_str(l);
                    if l.starts_with("#if") {
                        nest += 1;
                    } else if l.starts_with("#endif") {
                        nest -= 1;
                    }
                    i += 1;
                    if nest == 0 {
                        break;
                    }
                }
                always = true;
            } else {
                // a line, and its continuations
                while i < lines.len() {
                    item.push_str(lines[i]);
                    i += 1;
                    if !lines[i - 1].trim_end().ends_with('\\') {
                        break;
                    }
                }
            }
            let defines = if always {
                Vec::new()
            } else {
                declared_names(&item)
            };
            let always = always || defines.is_empty();
            items.push(Item {
                text: item,
                defines,
                always,
            });
            continue;
        }
        // a declaration: up to the `;` or the `}` that closes it at depth 0
        let (mut depth, mut comment) = (0, false);
        let start = item.len();
        while i < lines.len() {
            let l = lines[i];
            item.push_str(l);
            depth += brace_delta(l, &mut comment);
            i += 1;
            let end = l.trim_end();
            if depth <= 0 && !comment && (end.ends_with(';') || end.ends_with('}')) {
                break;
            }
        }
        let defines = declared_names(&item[start..]);
        let always = defines.is_empty();
        items.push(Item {
            text: item,
            defines,
            always,
        });
    }
    if !pending.is_empty() {
        items.push(Item {
            text: pending,
            defines: Vec::new(),
            always: true,
        });
    }
    items
}

/// The head a unit carries: every item it names, and every item those name, in the head's
/// order.  `names` are the identifiers of the unit's own text; `own` marks the unit that defines
/// the program's state, which carries every `LOFT_STATE` item whatever it names.  An item missed
/// here is a compile error in the unit, never a different program: C declares nothing implicitly.
fn unit_head(items: &[Item], names: &HashSet<&str>, own: bool) -> String {
    let mut want: HashSet<String> = names.iter().map(|n| (*n).to_string()).collect();
    let mut take = vec![false; items.len()];
    loop {
        let mut grew = false;
        for (k, item) in items.iter().enumerate() {
            let state = own && item.text.lines().any(|l| l.starts_with("LOFT_STATE "));
            if !take[k] && (item.always || state || item.defines.iter().any(|d| want.contains(d))) {
                take[k] = true;
                grew = true;
                want.extend(identifiers(&item.text).into_iter().map(str::to_string));
            }
        }
        if !grew {
            break;
        }
    }
    items
        .iter()
        .zip(&take)
        .filter(|(_, t)| **t)
        .map(|(i, _)| i.text.as_str())
        .collect()
}

/// `rest` — the unit past its `SPLIT_SHARED` mark — cut into what no file owns and each file's
/// functions, in the order the writer wrote them.
fn by_file(rest: &str) -> (String, BTreeMap<String, String>) {
    let mut shared = String::new();
    let mut files: BTreeMap<String, String> = BTreeMap::new();
    let mut cursor = rest;
    while let Some(open) = cursor.find(SPLIT_FN) {
        shared.push_str(&cursor[..open]);
        let after = &cursor[open + SPLIT_FN.len()..];
        let Some(name_end) = after.find("*/") else {
            break;
        };
        let file = after[..name_end].to_string();
        let body_start = &after[name_end + 2..];
        let Some(close) = body_start.find(SPLIT_FN_END) else {
            break;
        };
        files
            .entry(file)
            .or_default()
            .push_str(&body_start[..close]);
        cursor = &body_start[close + SPLIT_FN_END.len()..];
    }
    shared.push_str(cursor);
    (shared, files)
}

/// The unit each function of `bodies` is defined in, and the first line of its definition.
fn definers(bodies: &[&String]) -> (HashMap<String, usize>, HashMap<String, String>) {
    let mut definer: HashMap<String, usize> = HashMap::new();
    let mut heads: HashMap<String, String> = HashMap::new();
    for (u, text) in bodies.iter().enumerate() {
        for line in text.lines() {
            let bare = without_storage(line);
            if is_definition(bare)
                && let Some(n) = function_name(bare)
            {
                definer.insert(n.to_string(), u);
                heads
                    .entry(n.to_string())
                    .or_insert_with(|| line.to_string());
            }
        }
    }
    (definer, heads)
}

/// `kept` (the head past the runtime) without the definitions of `callbacks`, each replaced by its
/// prototype, and those definitions apart: the shared unit's.
fn take_callbacks(kept: &str, callbacks: &HashSet<String>) -> (String, String) {
    let mut called_back = String::new();
    let mut rest_of_head = String::with_capacity(kept.len());
    let mut lines = kept.split_inclusive('\n');
    while let Some(line) = lines.next() {
        let bare = without_storage(line);
        let named = function_name(bare).is_some_and(|n| callbacks.contains(n));
        // The head keeps the callback's prototype, unconditionally: the runtime's own may sit in
        // a conditional group this build leaves out (`loft_text_lower` under `LOFT_PATH_FOLD`),
        // where the one-unit build was declared by this definition.
        if named && !is_definition(bare) && !bare.trim_end().ends_with(';') {
            // a whole definition on its line
            if let Some(end) = bare.find(") {") {
                rest_of_head.push_str(&bare[..=end]);
                rest_of_head.push_str(";\n");
            }
            called_back.push_str(bare);
        } else if named && is_definition(bare) {
            rest_of_head.push_str(bare.trim_end().trim_end_matches('{').trim_end());
            rest_of_head.push_str(";\n");
            called_back.push_str(bare);
            for body in lines.by_ref() {
                called_back.push_str(body);
                if body.starts_with('}') {
                    break;
                }
            }
        } else {
            rest_of_head.push_str(line);
        }
    }
    (rest_of_head, called_back)
}

/// The mutable `static` variables of `hot` (the runtime every unit carries) and `kept` (the
/// head past it): each would be a copy per unit.
fn unshared_statics(hot: &str, kept: &str) -> Vec<String> {
    let mut unshared: Vec<String> = Vec::new();
    for line in hot.lines().chain(kept.lines()) {
        if let Some(rest) = line.strip_prefix("static ")
            && !rest.starts_with("inline ")
            && function_name(rest).is_none()
            && !is_constant(rest)
            && let Some(name) = rest
                .split(['=', '[', ';'])
                .next()
                .and_then(|d| d.trim_end().rsplit([' ', '*']).next())
        {
            unshared.push(name.to_string());
        }
    }
    unshared
}

/// The functions that need external linkage: those named outside the unit that defines them —
/// from another unit (`bodies`), from the head (`head_names`), or by the cold runtime
/// (`callbacks`).
fn exported_fns(
    bodies: &[&String],
    definer: &HashMap<String, usize>,
    head_names: &HashSet<&str>,
    callbacks: &HashSet<String>,
) -> HashSet<String> {
    let mut exported: HashSet<String> = HashSet::new();
    for (u, text) in bodies.iter().enumerate() {
        for n in identifiers(text) {
            if definer.get(n).is_some_and(|&home| home != u) {
                exported.insert(n.to_string());
            }
        }
    }
    exported.extend(
        definer
            .keys()
            .filter(|n| head_names.contains(n.as_str()) || callbacks.contains(n.as_str()))
            .cloned(),
    );
    exported
}

/// The written `unit` as the units of a split build, each starting with a
/// `SPLIT_UNIT <name>*/` line: the shared unit first, then one per file in a fixed order.
/// `runtime_end` closes the runtime in the head: nothing before it is rewritten.  A unit with no
/// `SPLIT_SHARED` mark is returned as it is.
pub(crate) fn into_units(unit: &str, runtime_end: &str) -> String {
    let Some(at) = unit.find(SPLIT_SHARED) else {
        return unit.to_string();
    };
    let head = &unit[..at];
    let rest = &unit[at + SPLIT_SHARED.len()..];
    let (shared, files) = by_file(rest);
    // The functions each unit defines, and which unit that is.
    let bodies: Vec<&String> = std::iter::once(&shared).chain(files.values()).collect();
    let (definer, heads) = definers(&bodies);
    let runtime_at = head.find(runtime_end).map_or(0, |a| a + runtime_end.len());
    let (hot, cold, callbacks) = split_runtime(&head[..runtime_at]);
    // The prototypes of the units' functions leave the head: each unit gets those its body names,
    // so a unit's text changes only with what it calls.  One the head's own code names (a table
    // naming a function) stays in the head, for every unit.
    let mut moved: Vec<(String, String)> = Vec::new();
    let mut kept = String::with_capacity(head.len() - runtime_at);
    for line in head[runtime_at..].split_inclusive('\n') {
        let bare = without_storage(line);
        let proto = !is_definition(bare)
            && bare.trim_end().ends_with(';')
            && function_name(bare).is_some_and(|n| definer.contains_key(n));
        if proto {
            moved.push((
                function_name(bare).unwrap_or_default().to_string(),
                line.to_string(),
            ));
        } else {
            kept.push_str(line);
        }
    }
    // A callback the head defines (`loft_init`) is called by the cold runtime: its definition is
    // the shared unit's, with external linkage, and every other unit gets its prototype.
    let (mut kept, called_back) = take_callbacks(&kept, &callbacks);
    // A variable the head declares `static` is a copy in every unit: one unit's write is not
    // another's read.  Program state is declared through `LOFT_STATE` (one definition, `extern`
    // elsewhere); a mutable `static` left in the head stops the build here, loudly, rather than
    // leaving each unit its own copy.
    let unshared = unshared_statics(&hot, &kept);
    if let Some(name) = unshared.first() {
        let _ = writeln!(
            kept,
            "#error \"a split build cannot share the static variable `{name}`: declare it through LOFT_STATE\""
        );
    }
    let head_names = identifiers(&kept);
    // A function needs external linkage only when something outside its own unit names it: the
    // head, or another unit.  Every other keeps its storage, so the C compiler still inlines it
    // and drops what nothing calls, as in the one-unit build.
    let exported = exported_fns(&bodies, &definer, &head_names, &callbacks);
    // A function another unit names that the head declares nowhere (a helper the writer defined
    // before its callers, as the function-value calls are) is declared from its definition.
    let declared: HashSet<String> = moved.iter().map(|(n, _)| n.clone()).collect();
    let mut synthesized: Vec<(String, String)> = exported
        .iter()
        .filter(|n| !declared.contains(*n))
        .filter_map(|n| {
            let def = heads.get(n)?;
            let proto = format!("{};\n", def.trim_end().trim_end_matches('{').trim_end());
            Some((n.clone(), proto))
        })
        .collect();
    synthesized.sort();
    moved.extend(synthesized);
    let mut stay = String::new();
    moved.retain(|(n, line)| {
        let named = head_names.contains(n.as_str());
        if named {
            stay.push_str(line);
        }
        !named
    });
    let moved: Vec<(String, String)> = moved
        .into_iter()
        .map(|(n, line)| {
            let line = give_linkage(&line, &exported, true);
            (n, line)
        })
        .collect();
    // Past the runtime, each unit carries only the head items it names (`unit_head`): a type, a
    // table or a constant changes the units that use it, not every unit.
    let items = head_items(&format!("{}{kept}", give_linkage(&stay, &exported, true)));
    let mut out = String::new();
    let mut put = |name: &str, body: &str, own: bool| {
        let body = give_linkage(body, &exported, false);
        let mut protos = String::new();
        let names = identifiers(&body);
        for (n, line) in &moved {
            if names.contains(n.as_str()) {
                protos.push_str(line);
            }
        }
        let mut wanted = names;
        wanted.extend(identifiers(&protos));
        out.push_str(SPLIT_UNIT);
        out.push_str(name);
        out.push_str("*/\n");
        out.push_str(&hot);
        out.push_str(&unit_head(&items, &wanted, own));
        out.push_str(&protos);
        out.push_str(&body);
    };
    put("shared", &format!("{called_back}{shared}"), true);
    for (i, body) in files.values().enumerate() {
        put(&format!("file{i}"), body, false);
    }
    // The cold runtime, alone: it names nothing the program wrote.
    out.push_str(SPLIT_UNIT);
    out.push_str("runtime*/\n");
    out.push_str(&cold);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_function_line_names_its_function_and_a_variable_none() {
        assert_eq!(function_name("int64_t n_f(int64_t v0_a) {"), Some("n_f"));
        assert_eq!(function_name("loft_text *n_g(void);"), Some("n_g"));
        assert_eq!(
            function_name("loft_text loft_wp_n_f_2 = { 0, 0, 0 };"),
            None
        );
        assert_eq!(function_name("    if (x) {"), None);
    }

    #[test]
    fn the_units_share_the_head_and_each_file_gets_its_functions() {
        let unit = format!(
            "RT\nEND\nstatic int64_t n_a(void);\nstatic inline int64_t n_b(void);\n{SPLIT_SHARED}int main(void) {{ return n_a(); }}\n{SPLIT_FN}one.loft*/\nstatic int64_t n_a(void) {{\n  return n_b();\n}}\n{SPLIT_FN_END}\n{SPLIT_FN}two.loft*/\nstatic inline int64_t n_b(void) {{\n  return 1;\n}}\n{SPLIT_FN_END}\n"
        );
        let units = into_units(&unit, "END\n");
        let parts: Vec<&str> = units.split(SPLIT_UNIT).filter(|p| !p.is_empty()).collect();
        assert_eq!(parts.len(), 4, "{units}");
        assert!(parts[3].starts_with("runtime*/"));
        assert!(parts[0].starts_with("shared*/") && parts[0].contains("int main(void)"));
        assert!(
            parts[1].contains("int64_t n_a(void) {")
                && !parts[1].contains("static int64_t n_a(void) {")
        );
        assert!(
            parts[2].contains("int64_t n_b(void) {")
                && !parts[2].contains("inline int64_t n_b(void) {")
        );
        // a prototype goes, without its storage, to the units whose functions name it: `n_b` to
        // the file calling it and the file defining it, not to the shared unit, which calls `n_a`
        assert!(
            parts[1].contains("\nint64_t n_b(void);") && parts[2].contains("\nint64_t n_b(void);")
        );
        assert!(!parts[0].contains("n_b(void);") && parts[0].contains("\nint64_t n_a(void);"));
        assert!(
            parts
                .iter()
                .all(|p| !p.contains("static inline int64_t n_b"))
        );
        // the runtime before the end mark is untouched
        assert!(parts.iter().all(|p| p.contains("RT\nEND\n")));
    }

    #[test]
    fn the_cold_runtime_is_one_unit_and_the_hot_part_stays_in_every_unit() {
        let rt = "static void loft_init(loft_cx *cx);\n\
                  static const loft_tdesc *loft_parse_types(loft_cx *cx);\n\
                  static const char *loft_name(int64_t a) {\n    return \"n(\";\n}\n\
                  static const char *const loft_names[] = { \"a(b\" };\n\
                  static struct loft_pair loft_pair_zero = { 0 };\n\
                  static inline int64_t loft_hot(int64_t a) { return a; }\n\
                  static int64_t loft_cold(int64_t a);\n\
                  static int64_t loft_cold(int64_t a) {\n    return loft_hot(a) + 1;\n}\n\
                  static const int loft_table[1] = { 0 };\n\
                  LOFT_HOT_INLINE int64_t loft_fast(void) {\n    return 2;\n}\n";
        let (hot, cold, callbacks) = split_runtime(rt);
        // a function the runtime declares and the program defines is external in both halves
        // — whatever its return type is spelled with (a `const` pointer)
        assert_eq!(
            callbacks,
            HashSet::from(["loft_init".to_string(), "loft_parse_types".to_string()])
        );
        assert!(
            hot.starts_with("void loft_init(loft_cx *cx);") && cold.starts_with("void loft_init(")
        );
        assert!(
            hot.contains("\nconst loft_tdesc *loft_parse_types(loft_cx *cx);\n")
                && cold.contains("\nconst loft_tdesc *loft_parse_types(loft_cx *cx);\n")
        );
        // a function returning a `const` pointer is cold like any other
        assert!(
            hot.contains("\nconst char *loft_name(int64_t a);\n"),
            "{hot}"
        );
        assert!(
            cold.contains("\nconst char *loft_name(int64_t a) {\n"),
            "{cold}"
        );
        // tables and variables keep their `static` in every unit
        for t in [
            "static const char *const loft_names[]",
            "static struct loft_pair loft_pair_zero",
        ] {
            assert!(hot.contains(t) && cold.contains(t), "{t}");
        }
        // the hot part keeps the inline helpers whole and declares the cold function
        assert!(hot.contains("static inline int64_t loft_hot(int64_t a) { return a; }"));
        assert!(hot.contains("LOFT_HOT_INLINE int64_t loft_fast(void) {\n    return 2;\n}"));
        assert!(hot.contains("\nint64_t loft_cold(int64_t a);\n"));
        assert!(!hot.contains("return loft_hot(a) + 1"), "{hot}");
        assert!(!hot.contains("static int64_t loft_cold"), "{hot}");
        // the cold unit defines it with external linkage, beside everything it calls
        assert!(
            cold.contains("\nint64_t loft_cold(int64_t a) {\n    return loft_hot(a) + 1;\n}\n")
        );
        assert!(cold.contains("static inline int64_t loft_hot"));
        assert!(!cold.contains("static int64_t loft_cold"), "{cold}");
        // a table is no function: every unit keeps its own
        assert!(
            hot.contains("static const int loft_table[1]")
                && cold.contains("static const int loft_table[1]")
        );
    }

    #[test]
    fn a_callback_the_head_defines_is_declared_in_every_unit() {
        // the runtime declares `loft_cb` only under a switch this build leaves out; the program's
        // head defines it, and a file's function calls it
        let unit = format!(
            "#ifdef LOFT_OFF\nstatic int64_t loft_cb(int64_t a);\n#endif\nEND\n\
             static int64_t loft_cb(int64_t a) {{ return a + 1; }}\nstatic int64_t n_a(void);\n\
             {SPLIT_SHARED}int main(void) {{ return (int) n_a(); }}\n\
             {SPLIT_FN}one.loft*/\nstatic int64_t n_a(void) {{\n  return loft_cb(2);\n}}\n{SPLIT_FN_END}\n"
        );
        let units = into_units(&unit, "END\n");
        let parts: Vec<&str> = units.split(SPLIT_UNIT).filter(|p| !p.is_empty()).collect();
        let one = parts
            .iter()
            .find(|p| p.contains("n_a(void) {"))
            .unwrap_or_else(|| {
                panic!(
                    "one.loft's unit: {:?}",
                    parts
                        .iter()
                        .map(|p| &p[..p.len().min(40)])
                        .collect::<Vec<_>>()
                )
            });
        // past the runtime (so outside its switched-off group), before the call
        let head = &one[one.find("END\n").expect("the runtime's end")..];
        let at = head
            .find("\nint64_t loft_cb(int64_t a);\n")
            .unwrap_or(usize::MAX);
        assert!(
            head.find("loft_cb(2)").is_some_and(|call| at < call),
            "{one}"
        );
        assert!(
            !one.contains("return a + 1"),
            "the definition is the shared unit's: {one}"
        );
        assert!(parts[0].contains("int64_t loft_cb(int64_t a) { return a + 1; }"));
    }

    #[test]
    fn a_pointer_to_constant_data_is_no_constant() {
        // shared by copy: the object is const, or a table
        for c in [
            "const int loft_n = 3;",
            "const uint32_t loft_cwords_C[] = { 1, 2 };",
            "const char *const loft_name = \"x\";",
            "const char *loft_kinds[] = { \"a\", \"b\" };",
        ] {
            assert!(is_constant(c), "{c}");
        }
        // state: one unit's write must be every unit's read
        for v in [
            "const loft_c_boundlib *loft_cbl = 0;",
            "const char *loft_anchor = \"\";",
            "const loft_lg_site *loft_lg_sites;",
            "uint32_t loft_count = 0;",
        ] {
            assert!(!is_constant(v), "{v}");
        }
    }

    #[test]
    fn a_head_item_declares_its_names() {
        let names = |t: &str| declared_names(t);
        assert_eq!(
            names("typedef struct lt_A {\n  int x;\n} lt_A;\n"),
            ["lt_A", "lt_A"]
        );
        assert_eq!(
            names("static const uint32_t loft_cwords_C[] = { 1, 2 };\n"),
            ["loft_cwords_C"]
        );
        assert_eq!(
            names("LOFT_STATE loft_ref loft_const_C;\n"),
            ["loft_const_C"]
        );
        assert_eq!(names("#define LOFT_N 3\n"), ["LOFT_N"]);
        assert_eq!(
            names("static void loft_show_a(loft_cx *cx) {\n}\n"),
            ["loft_show_a"]
        );
        let e = names("enum lt_E { LT_A = 1, LT_B };\n");
        assert!(
            e.contains(&"LT_A".to_string()) && e.contains(&"LT_B".to_string()),
            "{e:?}"
        );
    }

    #[test]
    fn a_unit_carries_the_head_items_it_names_and_what_they_name() {
        let head = "#define LOFT_K 2\n\
                    typedef struct lt_A { int x; } lt_A;\n\
                    /* B holds an A */\n\
                    typedef struct lt_B {\n  lt_A a;\n  char s[LOFT_K];\n} lt_B;\n\
                    typedef struct lt_C { int y; } lt_C;\n\
                    static const char loft_brace[] = \"}{\";\n\
                    LOFT_STATE loft_ref loft_const_C;\n\
                    #if defined(X)\nint loft_x;\n#endif\n";
        let items = head_items(head);
        assert_eq!(
            items.len(),
            7,
            "{:?}",
            items.iter().map(|i| &i.text).collect::<Vec<_>>()
        );
        let uses_b: HashSet<&str> = ["lt_B"].into_iter().collect();
        let got = unit_head(&items, &uses_b, false);
        assert!(
            got.contains("lt_A a;")
                && got.contains("typedef struct lt_A")
                && got.contains("#define LOFT_K"),
            "{got}"
        );
        assert!(
            got.contains("/* B holds an A */"),
            "a comment goes with its item: {got}"
        );
        assert!(
            !got.contains("lt_C") && !got.contains("loft_brace"),
            "{got}"
        );
        assert!(
            got.contains("#if defined(X)"),
            "a conditional group is in every unit: {got}"
        );
        let none: HashSet<&str> = HashSet::new();
        assert!(!unit_head(&items, &none, false).contains("loft_const_C"));
        assert!(
            unit_head(&items, &none, true).contains("LOFT_STATE loft_ref loft_const_C;"),
            "the unit that owns the state declares all of it"
        );
    }
}
