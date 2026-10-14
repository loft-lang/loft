// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN183 P4 — the construct under the cursor, named in the catalogue's vocabulary, and the
//! hover that shows its entry.  The vocabulary guard: every name the classifier answers is
//! declared by exactly one catalogue entry and is reached by a cell below; the catalogue keys
//! no hover reaches yet are pinned, so reaching one is a change this file records.

use loft::doc_construct::{CONSTRUCTS, construct_at};

/// The operand's type, from its NAME: what the language server reads off its resolution.
fn type_of_name(line: &str, col: u32) -> Option<String> {
    let chars: Vec<char> = line.chars().collect();
    let end = col as usize;
    let start = chars[..end]
        .iter()
        .rposition(|c| !(c.is_alphanumeric() || *c == '_'))
        .map_or(0, |i| i + 1);
    let name: String = chars[start..end].iter().collect();
    Some(
        match name.as_str() {
            "v" => "vector",
            "t" => "text",
            "h" => "hash",
            "ix" => "index",
            "so" => "sorted",
            "sp" => "spatial",
            "n" => "integer",
            _ => "Point",
        }
        .to_string(),
    )
}

/// (the line, the text the cursor sits on — its first occurrence, the construct).
const CELLS: &[(&str, &str, &str)] = &[
    ("  if a > 1 { b }", "if", "kw:if"),
    ("  } else {", "else", "kw:else"),
    ("  v[9] = 1 else { m += 1 };", "else", "kw:store-else"),
    ("  for i in v { }", "for", "kw:for"),
    ("  r = match a { 1 => 2, _ => 3 };", "match", "kw:match"),
    ("  if s is Circle { }", "is", "kw:is"),
    ("    break;", "break", "kw:break"),
    ("    continue;", "continue", "kw:continue"),
    ("    return 1;", "return", "kw:return"),
    ("    yield i;", "yield", "kw:yield"),
    ("    yield from inner();", "yield", "kw:yield-from"),
    ("use time;", "use", "kw:use"),
    ("pub fn area() -> integer { 1 }", "pub", "kw:pub"),
    ("  for x in v par(r = f(x), 4) { }", "par", "kw:par"),
    ("  x = null;", "null", "kw:null"),
    ("  for f in p#fields { }", "fields", "kw:#fields"),
    ("  sort(v, both);", "both", "kw:both"),
    ("  d#lock = true;", "lock", "attr:#lock"),
    ("#superseded(newer)", "superseded", "attr:#superseded"),
    ("  a = b ?? 0;", "??", "op:??"),
    ("  a = b ?? return;", "??", "op:??-return"),
    ("  a = b?;", "?", "op:?"),
    ("  a = b ** 2;", "**", "op:**"),
    ("  if a == b { }", "==", "op:=="),
    ("  if a <= b { }", "<=", "op:compare"),
    ("  c = a * b;", "*", "op:arith"),
    ("  n += 1;", "+=", "op:compound"),
    ("  if a && b { }", "&&", "op:logic"),
    ("  c = a ^ b;", "^", "op:bitwise"),
    ("  f = a as float;", "as", "op:as"),
    ("  for i in 0..3 { }", "..", "op:range"),
    ("fn grow(v: &vector<integer>) { }", "&", "param:&"),
    ("  r = &v;", "&", "bind:&"),
    ("  w = map(v, |x| x * 2);", "|", "lambda:short"),
    ("  f = fn(x: integer) -> integer { x };", "fn", "lambda:fn"),
    ("fn area(w: integer) -> integer { w }", "fn", "decl:fn"),
    ("struct Point { x: integer }", "struct", "decl:struct"),
    ("enum Shape { Circle, Square }", "enum", "decl:enum"),
    ("interface Shaped { }", "interface", "decl:interface"),
    ("type Id = integer;", "type", "decl:type-alias"),
    (
        "pub operator plus(self: Money, other: Money) -> Money { self }",
        "operator",
        "def:operator",
    ),
    (
        "pub operator plus(self: Money, other: Money) -> Money { self }",
        "plus",
        "def:operator-plus",
    ),
    (
        "pub operator compare(self: Day, o: Day) -> Ordering { Equal }",
        "compare",
        "def:operator-compare",
    ),
    (
        "pub operator to_text(self: Day) -> text { \"\" }",
        "to_text",
        "def:operator-to_text",
    ),
    (
        "pub operator minus(self: Day, o: Day) -> Day { self }",
        "minus",
        "def:operator-minus",
    ),
    (
        "pub operator times(self: Day, o: Day) -> Day { self }",
        "times",
        "def:operator-times",
    ),
    (
        "pub operator divided_by(self: Day, o: Day) -> Day { self }",
        "divided_by",
        "def:operator-divided_by",
    ),
    (
        "pub operator remainder(self: Day, o: Day) -> Day { self }",
        "remainder",
        "def:operator-remainder",
    ),
    (
        "pub operator negate(self: Day) -> Day { self }",
        "negate",
        "def:operator-negate",
    ),
    (
        "pub operator next(self: Walk) -> integer? { null }",
        "next",
        "def:operator-next",
    ),
    (
        "pub operator conversion(self: Day) -> integer { 0 }",
        "conversion",
        "def:operator-conversion",
    ),
    (
        "fn scale(self: Point, k: integer) -> Point { self }",
        "self",
        "param:self",
    ),
    ("fn f(x: const integer) { }", "const", "param:const"),
    ("  a = v[1];", "[", "index:vector"),
    ("  a = t[1];", "[", "index:text"),
    ("  a = h[1];", "[", "index:hash"),
    ("  a = ix[1];", "[", "index:index"),
    ("  a = so[1];", "[", "index:sorted"),
    ("  a = sp[1];", "]", "index:spatial"),
    ("  a = pt[1];", "[", "index:program-type"),
    ("  a = v[1..2];", "..", "slice:vector"),
    ("  a = t[1..2];", "[", "slice:text"),
    ("  v += [4];", "+=", "append:vector"),
    (
        "  r = match s { Circle | Square => 1, _ => 0 };",
        "|",
        "pattern:or",
    ),
    (
        "  r = match n { k if k > 1 => 1, _ => 0 };",
        "if",
        "pattern:guard",
    ),
    ("  println(\"total {n}\");", "{", "lit:interpolation"),
    ("  println(\"total {n:>5}\");", ":", "fmt:spec"),
    ("  f: fn(integer) -> integer = g;", "fn", "type:fn"),
    ("  x: integer? = null;", "?", "type:nullable"),
    ("  b: boolean = true;", "boolean", "type:boolean"),
    ("  c: character = 'a';", "character", "type:character"),
    ("  f: float = 1.0;", "float", "type:float"),
    ("  s: single = 1.0;", "single", "type:single"),
    ("  i: integer = 1;", "integer", "type:integer"),
    ("  a: u8 = 1;", "u8", "type:u8"),
    ("  a: u16 = 1;", "u16", "type:u16"),
    ("  a: u32 = 1;", "u32", "type:u32"),
    ("  a: i8 = 1;", "i8", "type:i8"),
    ("  a: i16 = 1;", "i16", "type:i16"),
    ("  a: i32 = 1;", "i32", "type:i32"),
    ("  w: vector<integer> = [];", "vector", "type:vector"),
    ("  m: hash<P[x]> = [];", "hash", "type:hash"),
    ("  m: index<P[x]> = [];", "index", "type:index"),
    ("  m: sorted<P[x]> = [];", "sorted", "type:sorted"),
    ("  m: spatial<P[x]> = [];", "spatial", "type:spatial"),
    (
        "fn walk() -> iterator<integer> { }",
        "iterator",
        "type:iterator",
    ),
    ("  f: File = file(\"a\");", "File", "type:File"),
    ("  a = arguments();", "arguments", "fn:arguments"),
    ("  assert(a, \"m\");", "assert", "fn:assert"),
    ("  deliver(x);", "deliver", "fn:deliver"),
    ("  expose(x);", "expose", "fn:expose"),
    ("  w = filter(v, |x| x > 1);", "filter", "fn:filter"),
    ("  j = json_parse(t);", "json_parse", "fn:json_parse"),
    ("  k = len(v);", "len", "fn:len"),
    ("  w = map(v, |x| x);", "map", "fn:map"),
    ("  t0 = now();", "now", "fn:now"),
    ("  panic(\"m\");", "panic", "fn:panic"),
    ("  s = reduce(v, 0, |a, b| a + b);", "reduce", "fn:reduce"),
    ("  for i in rev(v) { }", "rev", "fn:rev"),
    ("  k = size(t);", "size", "fn:size"),
    ("  k = sizeof(P);", "sizeof", "fn:sizeof"),
    ("  s = stack_trace();", "stack_trace", "fn:stack_trace"),
    ("  store_reclaim(p);", "store_reclaim", "fn:store_reclaim"),
    ("  store_release(p);", "store_release", "fn:store_release"),
    ("  k = ticks();", "ticks", "fn:ticks"),
    ("  j = to_json(p);", "to_json", "fn:to_json"),
];

/// The catalogue keys no hover reaches yet — each a construct the classifier cannot tell
/// from the line alone, or one with no source token.  Reaching one moves it to `CONSTRUCTS`
/// and a cell above.
const NOT_REACHED: &[&str] = &[
    "arg:named",
    "arith:overflow",
    "attr:#c",
    "bind:copy",
    "decl:associated-type",
    "decl:bound",
    "decl:generic-struct",
    "decl:interpolation-hook",
    "decl:struct-enum",
    "decl:test-fn",
    "decl:type-var",
    "decl:value-struct",
    "def:OpDrop",
    "dispatch:multiple",
    "dispatch:variant",
    "field:assert",
    "field:computed",
    "field:default",
    "field:limit",
    "field:not-null",
    "lit:backtick",
    "lit:comprehension",
    "lit:label",
    "lit:struct",
    "lit:tuple",
    "lit:variant",
    "lit:vector",
    "param:default",
    "pattern:sequence",
    "type:tuple",
];

/// The key prefixes no source token spells — a command, a library service, a file, a lint.
const NOT_IN_SOURCE: &[&str] = &["cli", "std", "file", "lint"];

#[test]
fn every_construct_is_owned_once_and_reached_by_a_cell() {
    for key in CONSTRUCTS {
        let owners = loft::doc_catalogue::entries()
            .iter()
            .filter(|e| e.keys.iter().any(|k| k == key))
            .count();
        assert_eq!(
            owners, 1,
            "`{key}` is declared by {owners} catalogue entries"
        );
        assert!(
            CELLS.iter().any(|(_, _, k)| k == key),
            "`{key}` has no cell below"
        );
    }
}

#[test]
fn every_catalogue_key_is_reached_or_pinned() {
    for e in loft::doc_catalogue::entries() {
        for key in &e.keys {
            let prefix = key.split(':').next().unwrap_or("");
            let reached = CONSTRUCTS.contains(&key.as_str());
            let pinned = NOT_REACHED.contains(&key.as_str());
            assert!(
                reached || pinned || NOT_IN_SOURCE.contains(&prefix),
                "@{}'s key `{key}` is neither reached by a hover nor pinned as not reached",
                e.tag
            );
            assert!(!(reached && pinned), "`{key}` is reached: unpin it");
        }
    }
}

#[test]
fn each_line_names_its_construct() {
    for (line, on, want) in CELLS {
        let col = line
            .find(on)
            .unwrap_or_else(|| panic!("`{on}` not in `{line}`"))
            + 1;
        let got = construct_at(line, 1, col as u32, &|c| type_of_name(line, c));
        assert_eq!(got, Some(*want), "`{line}` at `{on}`");
    }
}

#[test]
fn plain_names_and_type_parameters_name_no_construct() {
    for (line, on) in [
        ("  total = a;", "total"),
        ("  w: vector<integer> = [];", "<"),
        ("  x = 1;", "="),
    ] {
        let col = line.find(on).unwrap() + 1;
        assert_eq!(
            construct_at(line, 1, col as u32, &|c| type_of_name(line, c)),
            None,
            "`{line}` at `{on}`"
        );
    }
}
