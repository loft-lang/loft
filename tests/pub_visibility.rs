// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @C140 (@PLN187, @FR-F-Visible) — everything a file declares is private to it; `pub` gives consent.
//!
//! A type outside its file is INVISIBLE (not `pub`, named by no `pub` signature), NAME ONLY (not
//! `pub`, but named by a `pub` signature of its file: named, passed, stored, never built) or
//! `pub` (built too: a literal when every field is `pub`, an enum's variants).  A field is
//! private to its file; `pub` on it shows it.
//!
//! Every refusal holds on every build: naming, building, a field, a literal, a variant and an
//! abstract alias's representation.

use std::path::{Path, PathBuf};

const UNITS: &str = "\
struct Seed { v: integer }
struct Unit { pub name: text, hp: integer }
pub struct Color { pub r: float, pub g: float }
pub struct Mixed { pub a: integer, b: integer }
pub enum Shape { Circle { pub r: float }, Blob { seed: integer } }
enum Mood { Calm, Angry }
pub fn spawn(name: text) -> Unit { s = Seed { v: 1 }; Unit { name: name, hp: 100 + s.v } }
pub fn hp_of(u: Unit) -> integer { u.hp }
pub fn mood(angry: boolean) -> Mood { if angry { Angry } else { Calm } }
pub struct Holder { pub pair: (u8, text), hidden: (integer, integer) }
pub fn pair() -> (u8, text) { (7, \"seven\") }
pub fn hold() -> Holder { Holder { pair: (3, \"three\"), hidden: (1, 2) } }
type Handle = integer;
type Data = (integer, text, boolean);
pub type Open = (integer, text);
pub fn open(path: text) -> Handle { len(path) }
pub fn close(h: Handle, d: Data, plain: integer) -> integer { h + d.0 + plain }
pub fn data(x: integer) -> Data { (x, \"x\", true) }
pub fn opened() -> Open { (1, \"o\") }
pub fn many() -> vector<Handle> { [1, 2] }
pub fn total(hs: vector<Handle>) -> integer { s = 0; for h in hs { s += h; } s }
pub type Pair = (Handle, integer);
pub struct Item { pub id: Handle, pub tag: text }
type Flag = boolean;
pub fn both(path: text) -> (Handle, integer) { (len(path), 10) }
pub fn pair_of(path: text) -> Pair { (len(path), 20) }
pub fn grid() -> vector<vector<Handle>> { [[1, 2], [3]] }
pub fn named() -> vector<(Handle, text)> { [(4, \"four\"), (5, \"five\")] }
pub fn items() -> hash<Item[id]> { [Item { id: 7, tag: \"seven\" }] }
pub fn id_of(h: Handle) -> integer { h }
pub fn add(h: Handle, extra: integer) -> integer { h + extra }
pub fn flag() -> Flag { true }
";

/// Generic types at each level: an instance (`E<Mine>`, `N<integer>`) is visible as its
/// TEMPLATE is, wherever it was minted — a `pub` fn's body, or the caller's file.
const GENERICS: &str = "\
pub enum E<T> { Has { pub v: T }, Nothing }
pub struct S<T> { pub a: T, pub b: integer }
pub struct Q<T> { pub a: T, hidden: integer }
struct N<T> { pub a: T }
enum M<T> { Mx { pub v: T }, My }
struct Secret { pub s: integer }
pub struct Box<T> { pub held: T }
pub fn mk_n(x: integer) -> N<integer> { N { a: x } }
pub fn mk_m(x: integer) -> M<integer> { Mx { v: x } }
pub fn mk_q(x: integer) -> Q<integer> { Q { a: x, hidden: 5 } }
pub fn boxed() -> Box<Secret> { Box { held: Secret { s: 11 } } }
pub fn inside() -> integer {
  e: E<integer> = Has { v: 3 }; s = S { a: 1, b: 2 }; q = Q { a: 4, hidden: 6 }; n = N { a: 7 };
  m: M<integer> = Mx { v: 8 };
  match e { Has { v } => v, Nothing => 0 } + s.a + s.b + q.a + q.hidden + n.a + match m { Mx { v } => v, My => 0 }
}
";

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_pub_vis_{tag}_{}", std::process::id()));
    loft::file_access::create_dir_all(dir.join("lib")).expect("scratch");
    loft::file_access::write(dir.join("lib/units.loft"), UNITS).expect("units");
    loft::file_access::write(dir.join("lib/gl.loft"), GENERICS).expect("gl");
    dir
}

/// `(stdout, stderr, ok)` of `program` against `units`.
fn run(tag: &str, program: &str) -> (String, String, bool) {
    let dir = scratch(tag);
    let src = dir.join("main.loft");
    loft::file_access::write(&src, program).expect("program");
    let mut c = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
    c.arg("--interpret")
        .arg("--lib")
        .arg(dir.join("lib"))
        .arg(&src)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "120");
    let out = c.output().expect("run loft");
    let _ = loft::file_access::remove_dir_all(Path::new(&dir));
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.success(),
    )
}

#[test]
fn a_name_only_type_is_named_passed_and_read_through_its_pub_field() {
    let program = "use units;
fn show(u: units::Unit) -> text { \"{u.name}\" }
fn main() { u = units::spawn(\"orc\"); println(\"{show(u)} {units::hp_of(u)}\"); }
";
    let (out, err, ok) = run("named", program);
    assert!(ok && out == "orc 101\n", "{out}{err}");
}

#[test]
fn a_name_only_type_is_not_built_outside_its_file() {
    let program = "use units;
fn main() { u = units::Unit { name: \"x\", hp: 1 }; println(\"{u.name}\"); }
";
    let (_, err, ok) = run("built", program);
    assert!(
        !ok && err
            .contains("`Unit` is not `pub` in `units`, so it cannot be built outside that file"),
        "{err}"
    );
}

#[test]
fn an_invisible_type_is_not_named() {
    let program = "use units;
fn f(s: units::Seed) -> integer { 1 }
fn main() { println(\"{f}\"); }
";
    let (_, err, ok) = run("invisible", program);
    assert!(
        !ok && err.contains("`Seed` is not `pub` in `units`"),
        "{err}"
    );
}

#[test]
fn a_private_field_is_refused_outside_its_file() {
    let program = "use units;
fn main() { u = units::spawn(\"orc\"); println(\"{u.hp}\"); }
";
    let (_, err, ok) = run("field-on", program);
    assert!(
        !ok && err.contains("field `hp` of `Unit` is not `pub` in `units`")
            && err.contains("`units` declares it `pub hp`"),
        "{err}"
    );
}

#[test]
fn a_private_variant_field_is_refused_in_a_pattern() {
    let program = "use units;
fn main() {
  s = units::Blob { seed: 7 };
  v = match s { units::Blob { seed } => seed, _ => 0 };
  println(\"{v}\");
}
";
    let (_, err, ok) = run("pattern-on", program);
    assert!(
        !ok && err.contains("field `seed` of `Blob` is not `pub` in `units`"),
        "{err}"
    );
}

#[test]
fn a_pub_variant_field_reads_outside_its_file() {
    let program = "use units;
fn main() {
  s = units::Circle { r: 2.5 };
  v = match s { units::Circle { r } => r, _ => 0.0 };
  c = units::Color { r: 1.0, g: 0.5 };
  println(\"{v} {c.g}\");
}
";
    let (out, err, ok) = run("pubvariant", program);
    assert!(ok && out == "2.5 0.5\n", "{out}{err}");
}

#[test]
fn a_literal_with_a_private_field_is_refused() {
    let program = "use units;
fn main() { m = units::Mixed { a: 1 }; println(\"{m.a}\"); }
";
    let (_, err, ok) = run("mixed-on", program);
    assert!(
        !ok && err.contains("`Mixed` cannot be built outside `units`: its field `b` is not `pub`"),
        "{err}"
    );
}

#[test]
fn a_variant_of_a_name_only_enum_is_refused() {
    let program = "use units;
fn main() { m = units::mood(true); v = match m { Angry => 1, _ => 0 }; println(\"{v}\"); }
";
    let (_, err, ok) = run("mood-on", program);
    assert!(
        !ok && err.contains("`Angry` is a variant of `Mood`, which is not `pub` in `units`"),
        "{err}"
    );
}

#[test]
fn a_tuples_members_are_visible_wherever_the_tuple_is() {
    // A tuple is structural: no file declares it, so its members are never private fields —
    // returned by a `pub fn`, held in a `pub` field, read by index and destructured, a narrow
    // member included.  A PRIVATE field that holds a tuple is still refused, at the field.
    let program = "use units;
fn main() {
  p = units::pair();
  (a, b) = units::pair();
  q = units::hold().pair;
  println(\"{p.0} {p.1} {a} {b} {q.0} {q.1}\");
}
";
    let (out, err, ok) = run("tuple", program);
    assert!(ok && out == "7 seven 7 seven 3 three\n", "{out}{err}");
    let hidden = "use units;
fn main() { h = units::hold(); println(\"{h.hidden.0}\"); }
";
    let (_, err, ok) = run("tuple-hidden", hidden);
    assert!(
        !ok && err.contains("field `hidden` of `Holder` is not `pub` in `units`"),
        "{err}"
    );
}

#[test]
fn an_abstract_alias_is_bound_passed_back_stored_compared_and_printed() {
    // A non-`pub` alias named by a `pub` signature is ABSTRACT outside its file: held in a
    // variable, a parameter, a local declared with it and a field of the program's own
    // struct, handed back to the parameter declared with it, compared with `==`, printed.
    // Inside `units`, `close` reads both representations (`h + d.0`): there it is transparent.
    let program = "use units;
struct Mine { h: units::Handle, tag: integer }
fn keep(v: units::Handle) -> units::Handle { v }
fn main() {
  h = keep(units::open(\"abc\"));
  h2: units::Handle = units::open(\"xy\");
  m = Mine { h: h2, tag: 1 };
  d = units::data(2);
  same = h == units::open(\"def\");
  println(\"{h} {h2} {m.h} {same} {units::close(m.h, d, 3)}\");
}
";
    let (out, err, ok) = run("abstract", program);
    assert!(ok && out == "3 2 2 true 7\n", "{out}{err}");
}

#[test]
fn an_abstract_alias_refuses_what_reads_its_representation() {
    // Each cell reads the representation of a `Handle` or a `Data` outside `units` — an
    // operator, a member, destructuring, a parameter or a place of the underlying type, a
    // value built from a plain one, a result declared plain.  Each is refused naming the alias
    // and its library.
    let cells = [
        (
            "arith",
            "n = h + 1; println(\"{n}\");",
            "`+` reads its representation",
        ),
        ("member", "println(\"{d.0}\");", "reading a member"),
        (
            "destr",
            "(i, t, b) = d; println(\"{i}{t}{b}\");",
            "destructuring it",
        ),
        (
            "plain-param",
            "println(\"{abs(h)}\");",
            "parameter `self` of `abs` takes its underlying type",
        ),
        (
            "build",
            "println(\"{units::close(7, d, 3)}\");",
            "parameter `h` of `close` takes a `Handle`",
        ),
        (
            "annotated",
            "n: integer = h; println(\"{n}\");",
            "this place takes its underlying type",
        ),
        (
            "rebind",
            "h = 5; println(\"{h}\");",
            "this place takes a `Handle`",
        ),
    ];
    for (tag, body, why) in cells {
        let program = format!(
            "use units;\nfn main() {{\n  h = units::open(\"abc\");\n  d = units::data(2);\n  {body}\n}}\n"
        );
        let (_, err, ok) = run(&format!("reveal-{tag}-on"), &program);
        assert!(
            !ok && err.contains("is abstract outside `units`") && err.contains(why),
            "{tag}: {err}"
        );
    }
    let returned = "use units;
fn reveal() -> integer { units::open(\"abc\") }
fn main() { println(\"{reveal()}\"); }
";
    let (_, err, ok) = run("reveal-result", returned);
    assert!(
        !ok && err.contains("`Handle` is abstract outside `units`: the result of `reveal`"),
        "{err}"
    );
}

#[test]
fn a_pub_alias_stays_transparent() {
    // `pub type Open` is the substitution aliases always were: its members read anywhere.
    let program = "use units;
fn main() { o = units::opened(); (n, s) = o; println(\"{o.0 + 1} {o.1} {n} {s}\"); }
";
    let (out, err, ok) = run("pubalias", program);
    assert!(ok && out == "2 o 1 o\n", "{out}{err}");
}

#[test]
fn a_vector_of_an_abstract_alias_keeps_its_elements_abstract() {
    // `vector<Handle>` outside `units`: its elements read as `Handle`s (an index, a loop
    // variable, a parameter declared `units::Handle`), and the container operations that
    // never look at an element are open — `+=` of one alias, `[]`, `len`, `insert`,
    // `reverse`, in both spellings.  Hand-computed: [1,2] += [3] → [1,2,3]; w is a copy;
    // insert 1 at 0 → [1,1,2,3]; reversed → [3,2,1,1]; one element equals open("abc").
    let program = "use units;
fn first(v: vector<units::Handle>) -> units::Handle { v[0] }
fn main() {
  v = units::many();
  v += [units::open(\"abc\")];
  w: vector<units::Handle> = [];
  w += v;
  insert(v, 0, units::open(\"q\"));
  v.reverse();
  n = 0;
  for x in v { if x == units::open(\"abc\") { n += 1; } }
  println(\"{len(v)} {v.len()} {first(v)} {units::total(v)} {units::total(w)} {n}\");
}
";
    let (out, err, ok) = run("abstract-vector", program);
    assert!(ok && out == "4 4 3 7 6 1\n", "{out}{err}");
    // What reads an element as `integer`, or builds a `vector<Handle>` from plain values.
    let cells = [
        (
            "element",
            "println(\"{v[0] + 1}\");",
            "`+` reads its representation",
        ),
        (
            "loop",
            "for x in v { println(\"{x + 1}\"); }",
            "`+` reads its representation",
        ),
        (
            "sum",
            "println(\"{sum(v)}\");",
            "parameter `v` of `sum` takes its underlying type",
        ),
        (
            "sort",
            "sort(v); println(\"{v}\");",
            "parameter `self` of `sort`",
        ),
        (
            "sort-method",
            "v.sort(); println(\"{v}\");",
            "parameter `self` of `sort`",
        ),
        (
            "plain-element",
            "v += [5]; println(\"{v}\");",
            "`+=` reads its representation",
        ),
        (
            "plain-vector",
            "w: vector<integer> = v; println(\"{w}\");",
            "this place takes its underlying type",
        ),
        (
            "build",
            "println(\"{units::total([1, 2])}\");",
            "takes a `vector<Handle>`",
        ),
    ];
    for (tag, body, why) in cells {
        let program = format!("use units;\nfn main() {{\n  v = units::many();\n  {body}\n}}\n");
        let (_, err, ok) = run(&format!("vreveal-{tag}-on"), &program);
        assert!(
            !ok && err.contains("is abstract outside `units`") && err.contains(why),
            "{tag}: {err}"
        );
    }
}

#[test]
fn an_abstract_alias_is_followed_through_tuples_branches_patterns_and_lambdas() {
    // A `Handle` keeps its abstraction wherever a value carries it: a tuple member (a
    // returned tuple, a transparent `pub type Pair`, a tuple literal, destructuring, a
    // tuple pattern's binder), `vector<vector<Handle>>` and `vector<(Handle, text)>`, the
    // value of an `if`, a `match` and a block, a comprehension, a lambda's parameter and
    // result through a generic (`map`), a named argument and a keyed lookup's key.  Each
    // `Handle` reaches `id_of`, which takes only a `Handle`.  Hand-computed: both("abc") =
    // (3, 10); both("xy").0 = 2; pair("four") = (4, 20); grid()[0] = [1, 2]; named()[1].0 = 5.
    let program = "use units;
fn main() {
  (h, n) = units::both(\"abc\");
  t = units::both(\"xy\");
  a = t.0;
  p = units::pair_of(\"four\");
  b = p.0;
  q = p.1 + 1;
  row = units::grid()[0];
  c = row[1];
  e = units::named()[1].0;
  tg = units::named()[0].1;
  it = units::items()[units::open(\"1234567\")];
  x = if n > 5 { h } else { a };
  y = match n { 10 => b, _ => c };
  z = { a };
  m = [for v in row { v }];
  w = match t { (hh, 10) => hh, _ => h };
  mapped = map(row, |v| { v });
  lit = (h, 1);
  row.insert(0, h);
  via_method = row.map(|v| { v });
  print(\"{units::id_of(h)} {n} {units::id_of(a)} {units::id_of(b)} {q} {units::id_of(c)} \");
  print(\"{units::id_of(e)} {tg} {it.tag} {units::id_of(x)} {units::id_of(y)} {units::id_of(z)} \");
  print(\"{units::id_of(m[0])} {units::id_of(w)} {units::add(extra: 1, h: h)} \");
  println(\"{units::id_of(mapped[1])} {units::id_of(lit.0)} {units::id_of(via_method[0])}\");
}
";
    let (out, err, ok) = run("abstract-nested", program);
    assert!(
        ok && out == "3 10 2 4 21 2 5 four seven 3 4 2 1 2 4 2 3 3\n",
        "{out}{err}"
    );
    // Each cell reads a `Handle` (or a `Flag`) somewhere a value carries it, or builds one.
    let cells = [
        ("tuple-member", "println(\"{t.0 + 1}\");", "`+` reads"),
        (
            "destructure",
            "(a, b) = t; println(\"{a * 2}{b}\");",
            "`*` reads",
        ),
        (
            "pub-pair",
            "p = units::pair_of(\"four\"); println(\"{p.0 + 1}\");",
            "`+` reads",
        ),
        (
            "grid",
            "println(\"{units::grid()[0][1] + 1}\");",
            "`+` reads",
        ),
        (
            "grid-loop",
            "for r in units::grid() { for v in r { println(\"{v + 1}\"); } }",
            "`+` reads",
        ),
        (
            "named-tuple",
            "println(\"{units::named()[0].0 + 1}\");",
            "`+` reads",
        ),
        (
            "tuple-literal",
            "x = (h, 1); println(\"{x.0 + 1}\");",
            "`+` reads",
        ),
        (
            "tuple-build",
            "x: (units::Handle, integer) = (5, 1); println(\"{x.1}\");",
            "takes a `(Handle, _)`",
        ),
        (
            "if-mix",
            "x = if t.1 > 5 { h } else { 7 }; println(\"{x}\");",
            "another branch of this value",
        ),
        (
            "if-value",
            "x = if t.1 > 5 { h } else { h }; println(\"{x + 1}\");",
            "`+` reads",
        ),
        (
            "if-flag",
            "if units::flag() { println(\"yes\"); }",
            "branching on it",
        ),
        (
            "match-subject",
            "v = match h { 3 => 1, _ => 0 }; println(\"{v}\");",
            "matching on it",
        ),
        (
            "match-literal",
            "v = match t { (2, k) => k, _ => 0 }; println(\"{v}\");",
            "matching a pattern",
        ),
        (
            "match-binder",
            "v = match t { (hh, k) => hh + k }; println(\"{v}\");",
            "`+` reads",
        ),
        ("block", "x = { h }; println(\"{x + 1}\");", "`+` reads"),
        (
            "comprehension",
            "m = [for v in units::grid()[0] { v }]; println(\"{m[0] + 1}\");",
            "`+` reads",
        ),
        (
            "comprehension-build",
            "m = [for v in units::grid()[0] { 5 }]; println(\"{units::id_of(m[0])}\");",
            "takes a `Handle`",
        ),
        (
            "lambda-param",
            "d = map(units::grid()[0], |v| { v + 1 }); println(\"{d}\");",
            "`+` reads",
        ),
        (
            "lambda-result",
            "d = map(units::grid()[0], |v| { v }); println(\"{d[0] + 1}\");",
            "`+` reads",
        ),
        (
            "lambda-declared",
            "f = fn(x: units::Handle) -> integer { x + 1 }; println(\"{f(h)}\");",
            "`+` reads",
        ),
        (
            "named-build",
            "println(\"{units::add(h: 7, extra: 1)}\");",
            "parameter `h` of `add` takes a `Handle`",
        ),
        (
            "method-lambda",
            "d = units::grid()[0].map(|v| { v + 1 }); println(\"{d}\");",
            "`+` reads",
        ),
        (
            "method-lambda-result",
            "r = units::grid()[0]; d = r.map(|v| { v }); println(\"{d[0] + 1}\");",
            "`+` reads",
        ),
        (
            "method-insert-build",
            "r = units::grid()[0]; r.insert(0, 5); println(\"{r}\");",
            "parameter `elem` of `insert` takes a `Handle`",
        ),
        (
            "key-build",
            "println(\"{units::items()[7].tag}\");",
            "this key takes a `Handle`",
        ),
        (
            "key-field",
            "for it in units::items() { println(\"{it.id + 1}\"); }",
            "`+` reads",
        ),
    ];
    for (tag, body, why) in cells {
        let program = format!(
            "use units;\nfn main() {{\n  h = units::open(\"abc\");\n  t = units::both(\"xy\");\n  {body}\n}}\n"
        );
        let (_, err, ok) = run(&format!("nreveal-{tag}-on"), &program);
        assert!(
            !ok && err.contains("is abstract outside `units`") && err.contains(why),
            "{tag}: {err}"
        );
    }
}

#[test]
fn a_generic_instance_is_visible_as_its_template_is() {
    let allowed = [
        (
            "variant-user-arg",
            "use gl::*;\nstruct Mine { n: integer }\nfn main() { c: E<Mine> = Has { v: Mine { n: 7 } }; a = match c { Has { v } => v.n, Nothing => 0 }; println(\"{a}\"); }\n",
            "7\n",
        ),
        (
            "variant-qualified",
            "use gl;\nfn main() { e: gl::E<integer> = gl::Has { v: 41 }; r = match e { gl::Has { v } => v, gl::Nothing => 0 }; println(\"{r}\"); }\n",
            "41\n",
        ),
        (
            "literal-all-pub",
            "use gl;\nfn main() { s = gl::S { a: 40, b: 2 }; println(\"{s.a + s.b}\"); }\n",
            "42\n",
        ),
        (
            "name-only-named",
            "use gl;\nfn show(n: gl::N<integer>) -> integer { n.a }\nfn peek(s: gl::Secret) -> integer { s.s }\nfn main() { println(\"{show(gl::mk_n(4))} {peek(gl::boxed().held)}\"); }\n",
            "4 11\n",
        ),
        (
            "inside",
            "use gl;\nfn main() { println(\"{gl::inside()} {gl::mk_q(2).a}\"); }\n",
            "31 2\n",
        ),
    ];
    for (tag, program, want) in allowed {
        let (out, err, ok) = run(&format!("gen-{tag}"), program);
        assert!(ok && out == want, "{tag}: {out}{err}");
    }
    let refused = [
        (
            "literal-private-field",
            "use gl;\nfn main() { q = gl::Q { a: 1, hidden: 2 }; println(\"{q.a}\"); }\n",
            "`Q<integer>` cannot be built outside `gl`: its field `hidden` is not `pub`",
        ),
        (
            "field-private",
            "use gl;\nfn main() { q = gl::mk_q(1); println(\"{q.hidden}\"); }\n",
            "field `hidden` of `Q<integer>` is not `pub` in `gl`",
        ),
        (
            "name-only-variant",
            "use gl::*;\nfn main() { m = mk_m(5); r = match m { Mx { v } => v, _ => 0 }; println(\"{r}\"); }\n",
            "`Mx` is a variant of `M`, which is not `pub` in `gl`",
        ),
        (
            "name-only-built",
            "use gl::*;\nfn main() { n = N { a: 3 }; println(\"{n.a}\"); }\n",
            "`N` is not `pub` in `gl`, so it cannot be built outside that file",
        ),
        (
            "name-only-arg-built",
            "use gl;\nfn main() { s = gl::Secret { s: 1 }; println(\"{s.s}\"); }\n",
            "`Secret` is not `pub` in `gl`, so it cannot be built outside that file",
        ),
    ];
    for (tag, program, why) in refused {
        let (_, err, ok) = run(&format!("gen-{tag}"), program);
        assert!(!ok && err.contains(why), "{tag}: {err}");
    }
}
