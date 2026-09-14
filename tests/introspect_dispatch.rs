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
//!
//! And the slim-artifact property (step 12): because every static site is a direct call, an
//! overload nothing calls is unreachable, and the SHIPPED lane (`--native-release`, which emits
//! only reachable functions) leaves it out — the one property whose failure is invisible in
//! behaviour, so it is asserted on the emitted source.  The semantics lane (`--native`) keeps
//! every tier by design (NATIVE.md § Optimisation tiers), which is the control that lets the
//! assertion fail: it emits all four.

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

const WITH_UNCALLED: &str = "\
enum Entity { Fireball { n: integer }, IceWall { n: integer }, Crate { n: integer }, Slime { n: integer } }
fn hit(f: Fireball, w: IceWall) -> integer { f.n * 10 + w.n }
fn hit(f: Fireball, c: Crate) -> integer { f.n * 100 + c.n }
fn hit(s: Slime, c: Crate) -> integer { s.n * 1000 + c.n }
fn hit(a: Entity, b: Entity) -> integer { a.n * 0 + b.n * 0 }
fn main() {
  f = Fireball { n: 1 };
  w = IceWall { n: 2 };
  c = Crate { n: 3 };
  println(\"{hit(f, w)} {hit(f, c)}\");
}
";

/// The user-function headers the given lane emits for `WITH_UNCALLED`.
fn emitted_hit_overloads(lane: &str) -> Vec<String> {
    let dir = std::env::temp_dir().join(format!("loft_dce_dispatch_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let src = dir.join("with_uncalled.loft");
    let out_rs = dir.join(format!(
        "with_uncalled_{}.rs",
        lane.trim_start_matches("--")
    ));
    std::fs::write(&src, WITH_UNCALLED).expect("write source");
    let out = Command::new(loft_binary())
        .args([
            lane,
            "--native-emit",
            out_rs.to_str().unwrap(),
            src.to_str().unwrap(),
        ])
        .env("LOFT_TIMEOUT", "240")
        .current_dir(project_root())
        .output()
        .expect("failed to spawn the loft binary");
    assert!(
        out.status.success(),
        "{lane} --native-emit failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let emitted = std::fs::read_to_string(&out_rs).expect("read the emitted source");
    let _ = std::fs::remove_file(&src);
    let _ = std::fs::remove_file(&out_rs);
    emitted
        .lines()
        .filter(|l| l.starts_with("fn f_") && l.contains("_hit("))
        .map(|l| l["fn ".len()..l.find('(').unwrap()].to_string())
        .collect()
}

#[test]
fn an_uncalled_overload_is_absent_from_the_shipped_artifact() {
    let shipped = emitted_hit_overloads("--native-release");
    assert!(
        shipped.iter().any(|f| f.ends_with("Fireball_IceWall_hit"))
            && shipped.iter().any(|f| f.ends_with("Fireball_Crate_hit")),
        "the two called overloads must be emitted in the shipped lane: {shipped:?}"
    );
    assert!(
        !shipped.iter().any(|f| f.ends_with("Slime_Crate_hit"))
            && !shipped.iter().any(|f| f.ends_with("Entity_Entity_hit")),
        "an overload nothing calls, the total fallback included, must be absent from the shipped lane: {shipped:?}"
    );
    // The control: the semantics lane keeps every tier, so it emits all four — which is what
    // makes the absence above an assertion the release lane can fail.
    let semantics = emitted_hit_overloads("--native");
    assert_eq!(
        semantics.len(),
        4,
        "the semantics lane must emit every overload (the control for the shipped lane): {semantics:?}"
    );
}
