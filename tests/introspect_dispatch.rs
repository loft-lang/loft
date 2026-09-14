// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `Disp-Closed` (@PLN162 step 11): a call site whose argument types are all statically
//! concrete selects at parse time and lowers to a DIRECT call of the selected definition — no
//! runtime table, no dispatcher.  The proof is the one the plan asked for: the dispatched
//! program's `main` is byte-identical, in IR and bytecode, to its hand-monomorphised twin
//! (unique names instead of an overload set) once the callee names are normalised.  Measured
//! before it was pinned: the only differences over the whole `loft introspect` dump were the
//! callee names, the def numbers shifted by the dispatcher's own def, and the live-reload table
//! (`LOFT_LIVE_FNS`), which lists neither `f_`-keyed overload — a step-14 finding.

use std::path::PathBuf;
use std::process::Command;

fn loft_binary() -> &'static str {
    env!("CARGO_BIN_EXE_loft")
}

fn project_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

const DISPATCHED: &str = "\
enum Entity { Fireball { n: integer }, IceWall { n: integer }, Crate { n: integer } }
fn hit(f: Fireball, w: IceWall) -> integer { f.n * 10 + w.n }
fn hit(f: Fireball, c: Crate) -> integer { f.n * 100 + c.n }
fn hit(a: Entity, b: Entity) -> integer { a.n * 0 + b.n * 0 }
fn main() {
  f = Fireball { n: 1 };
  w = IceWall { n: 2 };
  c = Crate { n: 3 };
  println(\"{hit(f, w)} {hit(f, c)}\");
}
";

const MONOMORPHISED: &str = "\
enum Entity { Fireball { n: integer }, IceWall { n: integer }, Crate { n: integer } }
fn hit_fw(f: Fireball, w: IceWall) -> integer { f.n * 10 + w.n }
fn hit_fc(f: Fireball, c: Crate) -> integer { f.n * 100 + c.n }
fn hit_ee(a: Entity, b: Entity) -> integer { a.n * 0 + b.n * 0 }
fn main() {
  f = Fireball { n: 1 };
  w = IceWall { n: 2 };
  c = Crate { n: 3 };
  println(\"{hit_fw(f, w)} {hit_fc(f, c)}\");
}
";

fn introspect(name: &str, source: &str) -> String {
    let dir = std::env::temp_dir().join(format!("loft_introspect_dispatch_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let src = dir.join(format!("{name}.loft"));
    std::fs::write(&src, source).expect("write source");
    let out = Command::new(loft_binary())
        .args(["--introspect", src.to_str().unwrap()])
        .env("LOFT_NO_CACHE", "1")
        .current_dir(project_root())
        .output()
        .expect("failed to spawn the loft binary");
    let _ = std::fs::remove_file(&src);
    assert!(
        out.status.success(),
        "introspect failed for {name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// The bytecode listing of `n_main`, from its header to the blank line that ends it.
fn main_bytecode(dump: &str) -> String {
    let start = dump
        .find(":n_main()\n")
        .expect("no bytecode header for n_main");
    let body = &dump[start + ":n_main()\n".len()..];
    let end = body.find("\n\n").unwrap_or(body.len());
    body[..end].to_string()
}

/// Names and per-program numbers that legitimately differ between the twins.
fn normalise(listing: &str) -> String {
    let mut s = listing.to_string();
    for (from, to) in [
        ("f_16Fireball#IceWall_hit", "HIT_FW"),
        ("f_14Fireball#Crate_hit", "HIT_FC"),
        ("n_hit_fw", "HIT_FW"),
        ("n_hit_fc", "HIT_FC"),
    ] {
        s = s.replace(from, to);
    }
    // `Call(d_nr=748, …)` — the dispatcher's own def shifts every number after it.
    let mut out = String::with_capacity(s.len());
    let mut rest = s.as_str();
    while let Some(i) = rest.find("d_nr=") {
        out.push_str(&rest[..i + 5]);
        rest = &rest[i + 5..];
        let n = rest
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(rest.len());
        out.push('N');
        rest = &rest[n..];
    }
    out.push_str(rest);
    out
}

#[test]
fn a_static_dispatched_call_lowers_to_a_direct_call() {
    let dispatched = introspect("dispatched", DISPATCHED);
    let mono = introspect("monomorphised", MONOMORPHISED);
    let bc = main_bytecode(&dispatched);
    assert!(
        bc.contains("fn=f_16Fireball#IceWall_hit") && bc.contains("fn=f_14Fireball#Crate_hit"),
        "the two statically-concrete sites must each be a plain `Call` of the selected overload:\n{bc}"
    );
    assert!(
        !dispatched.contains("t_6Entity_hit"),
        "no enum-level dispatcher may be synthesised for a free overload set that carries its own fallback:\n{dispatched}"
    );
    assert_eq!(
        normalise(&bc),
        normalise(&main_bytecode(&mono)),
        "the dispatched `main` must be byte-identical to its hand-monomorphised twin once the callee names are normalised"
    );
}
