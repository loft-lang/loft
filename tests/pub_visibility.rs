// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @C140 (@PLN187, @FR-F-Visible) — everything a file declares is private to it; `pub` gives consent.
//!
//! A type outside its file is INVISIBLE (not `pub`, named by no `pub` signature), NAME ONLY (not
//! `pub`, but named by a `pub` signature of its file: named, passed, stored, never built) or
//! `pub` (built too: a literal when every field is `pub`, an enum's variants).  A field is
//! private to its file; `pub` on it shows it.
//!
//! The naming half and the build refusal hold on every build.  The field, literal and variant
//! refusals are enforced under `LOFT_PUB_ENFORCE=1` until the published libraries carry their
//! `pub` (@PLN187 step 6) — each cell here runs both ways, so the switch-off answer (the program
//! as it ran before) is pinned beside the refusal.

use std::path::{Path, PathBuf};
use std::process::Command;

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
";

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_pub_vis_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(dir.join("lib")).expect("scratch");
    std::fs::write(dir.join("lib/units.loft"), UNITS).expect("units");
    dir
}

/// `(stdout, stderr, ok)` of `program` against `units`, with or without enforcement.
fn run(tag: &str, program: &str, enforce: bool) -> (String, String, bool) {
    let dir = scratch(tag);
    let src = dir.join("main.loft");
    std::fs::write(&src, program).expect("program");
    let mut c = Command::new(env!("CARGO_BIN_EXE_loft"));
    c.arg("--interpret")
        .arg("--lib")
        .arg(dir.join("lib"))
        .arg(&src)
        .env("LOFT_NO_CACHE", "1")
        .env("LOFT_TIMEOUT", "120");
    if enforce {
        c.env("LOFT_PUB_ENFORCE", "1");
    }
    let out = c.output().expect("run loft");
    let _ = std::fs::remove_dir_all(Path::new(&dir));
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
    for enforce in [false, true] {
        let (out, err, ok) = run(&format!("named{enforce}"), program, enforce);
        assert!(ok && out == "orc 101\n", "enforce={enforce}: {out}{err}");
    }
}

#[test]
fn a_name_only_type_is_not_built_outside_its_file() {
    let program = "use units;
fn main() { u = units::Unit { name: \"x\", hp: 1 }; println(\"{u.name}\"); }
";
    let (_, err, ok) = run("built", program, false);
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
    let (_, err, ok) = run("invisible", program, false);
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
    let (out, err, ok) = run("field-off", program, false);
    assert!(
        ok && out == "101\n",
        "switched off, the program runs as before: {out}{err}"
    );
    let (_, err, ok) = run("field-on", program, true);
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
    let (_, err, ok) = run("pattern-on", program, true);
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
    let (out, err, ok) = run("pubvariant", program, true);
    assert!(ok && out == "2.5 0.5\n", "{out}{err}");
}

#[test]
fn a_literal_with_a_private_field_is_refused() {
    let program = "use units;
fn main() { m = units::Mixed { a: 1 }; println(\"{m.a}\"); }
";
    let (out, err, ok) = run("mixed-off", program, false);
    assert!(ok && out == "1\n", "switched off: {out}{err}");
    let (_, err, ok) = run("mixed-on", program, true);
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
    let (out, err, ok) = run("mood-off", program, false);
    assert!(ok && out == "1\n", "switched off: {out}{err}");
    let (_, err, ok) = run("mood-on", program, true);
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
    for enforce in [false, true] {
        let (out, err, ok) = run(&format!("tuple{enforce}"), program, enforce);
        assert!(ok && out == "7 seven 7 seven 3 three\n", "enforce={enforce}: {out}{err}");
    }
    let hidden = "use units;
fn main() { h = units::hold(); println(\"{h.hidden.0}\"); }
";
    let (_, err, ok) = run("tuple-hidden", hidden, true);
    assert!(
        !ok && err.contains("field `hidden` of `Holder` is not `pub` in `units`"),
        "{err}"
    );
}
