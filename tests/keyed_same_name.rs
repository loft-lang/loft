// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Two libraries that each declare a struct of one NAME, with other fields, each key their own
//! collections: a `hash<Nm[k]>`, a `sorted<Nm[n]>` and an `index<Nm[n]>`, as a local and as a
//! struct field, answer the library's own records whichever library is loaded first.
//!
//! The schema registers the second library's struct source-qualified, and its collections
//! with it; `StoreTypeOf::keyed_type` looked a collection up by the element's DEFINITION name
//! and so found the first library's, whose key layout is another struct's: every hash lookup
//! of the second library missed, on both backends, with no diagnostic.
//!
//! @falsified-at: 4338b7a7a — by hand: `keyed_type` naming the element by its definition again.
//!   CHANNEL: value (the printed line against the hand-computed one), on both backends.
//!   ARMED: nothing to arm.
//!   WITNESS: loaded second, `rb` answers `-1 110 77 …` and `-1 55 66` (its hash reads miss);
//!   loaded second, `ra` answers `miss s10 i7 …` and `miss f5 f6`.
//!   HOLDS: each library alone answers its line on both trees.

const RA: &str = r#"#package
struct Nm { k: text, n: integer, v: text }
struct Box { h: hash<Nm[k]>, s: sorted<Nm[n]>, i: index<Nm[n]> }
pub fn a_local() -> text {
  h: hash<Nm[k]> = [Nm { k: "x", n: 1, v: "vx" }, Nm { k: "y", n: 2, v: "vy" }];
  s: sorted<Nm[n]> = [Nm { k: "p", n: 30, v: "s30" }, Nm { k: "q", n: 10, v: "s10" }];
  i: index<Nm[n]> = [Nm { k: "p", n: 7, v: "i7" }, Nm { k: "q", n: 3, v: "i3" }];
  o = "";
  for e in s { o += e.v; }
  for e in i { o += e.v; }
  "{h["y"].v ?? "miss"} {s[10].v ?? "miss"} {i[7].v ?? "miss"} {len(h)}{len(s)}{len(i)} {o}"
}
pub fn a_field() -> text {
  b = Box { h: [Nm { k: "x", n: 1, v: "fx" }], s: [Nm { k: "p", n: 5, v: "f5" }], i: [Nm { k: "q", n: 6, v: "f6" }] };
  "{b.h["x"].v ?? "miss"} {b.s[5].v ?? "miss"} {b.i[6].v ?? "miss"}"
}
"#;

const RB: &str = r#"#package
struct Nm { n: integer, k: text, v: integer }
struct Box { h: hash<Nm[k]>, s: sorted<Nm[n]>, i: index<Nm[n]> }
pub fn b_local() -> text {
  h: hash<Nm[k]> = [Nm { n: 1, k: "x", v: 100 }, Nm { n: 2, k: "y", v: 200 }];
  s: sorted<Nm[n]> = [Nm { n: 30, k: "p", v: 330 }, Nm { n: 10, k: "q", v: 110 }];
  i: index<Nm[n]> = [Nm { n: 7, k: "p", v: 77 }, Nm { n: 3, k: "q", v: 33 }];
  t = 0;
  for e in s { t = t * 1000 + e.v; }
  for e in i { t = t * 100 + e.v; }
  "{h["y"].v ?? -1} {s[10].v ?? -1} {i[7].v ?? -1} {len(h)}{len(s)}{len(i)} {t}"
}
pub fn b_field() -> text {
  b = Box { h: [Nm { n: 1, k: "x", v: 11 }], s: [Nm { n: 5, k: "p", v: 55 }], i: [Nm { n: 6, k: "q", v: 66 }] };
  "{b.h["x"].v ?? -1} {b.s[5].v ?? -1} {b.i[6].v ?? -1}"
}
"#;

/// What `ra` answers: `h["y"]`, `s[10]`, `i[7]`, the three lengths, the two walks in key order;
/// then the same reads through a struct's fields.
const A: &str = "vy s10 i7 222 s10s30i3i7 | fx f5 f6";
/// What `rb` answers: 200, 110, 77, the lengths, and the walks folded into one number —
/// `110`, then `110 * 1000 + 330`, then `* 100 + 33`, then `* 100 + 77`.
const B: &str = "200 110 77 222 1103303377 | 11 55 66";

const CALL: &str = "fn main() { println(\"{ra::a_local()} | {ra::a_field()} | {rb::b_local()} | {rb::b_field()}\"); }\n";

fn loft_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn have_cc() -> bool {
    loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn each_library_keys_its_own_struct_of_a_shared_name() {
    let dir = std::env::temp_dir().join(format!("loft-keyed-same-name-{}", std::process::id()));
    let lib = dir.join("lib");
    loft::file_access::create_dir_all(&lib).unwrap();
    loft::file_access::write(lib.join("ra.loft"), RA).unwrap();
    loft::file_access::write(lib.join("rb.loft"), RB).unwrap();
    for (name, uses) in [("ab", "use ra;\nuse rb;\n"), ("ba", "use rb;\nuse ra;\n")] {
        let prog = dir.join(format!("{name}.loft"));
        loft::file_access::write(&prog, format!("{uses}{CALL}")).unwrap();
        for mode in ["--interpret", "--native-c"] {
            if mode == "--native-c" && !have_cc() {
                continue;
            }
            let o = loft::platform::process::harness_command(loft_bin())
                .args([mode, "--no-warnings", "--lib"])
                .arg(&lib)
                .arg(&prog)
                .env("LOFT_NATIVE_LEAK_CHECK", "1")
                .output()
                .expect("loft runs");
            let err = String::from_utf8_lossy(&o.stderr);
            assert_eq!(
                String::from_utf8_lossy(&o.stdout),
                format!("{A} | {B}\n"),
                "{name} {mode}: {err}"
            );
            assert!(
                o.status.success() && !err.contains("stores not freed"),
                "{name} {mode}: {err}"
            );
        }
    }
}
