// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The lowering census as a guard (@PLN220, STORE_RECORDS.md § The lowering census): the
//! interpreter's bytecode writer and the C writer name the same store effect for every
//! assignment of `tests/scripts/a-discharge-block-hands-over-the-local-it-bound.loft` —
//! `scripts/bind_census.py --check` over that one program.  The whole suite is the nightly
//! `Lowering census` job; this is the cell that held the last disagreement, kept on every PR.
//!
//! @falsified-at: none — the check is new with its guard; the second test IS the receipt:
//! with `LOFT_DISCHARGE_BLOCK_JOIN=1` the interpreter joins a discharge block again and the
//! check must fail naming the pair.

fn census(switch: Option<&str>) -> Option<(bool, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!(
        "loft-bind-census-{}-{}",
        std::process::id(),
        u8::from(switch.is_some())
    ));
    loft::file_access::create_dir_all(&dir).ok()?;
    // A copy: the census runs the program from its own directory, and a run there writes
    // caches beside it.
    let cell = dir.join("cell.loft");
    loft::file_access::copy(
        root.join("tests/scripts/a-discharge-block-hands-over-the-local-it-bound.loft"),
        &cell,
    )
    .ok()?;
    let cells = dir.join("cells.txt");
    loft::file_access::write(&cells, cell.display().to_string()).ok()?;
    let mut cmd = loft::platform::process::harness_command("python3");
    cmd.arg(root.join("scripts/bind_census.py"))
        .arg("--loft")
        .arg(env!("CARGO_BIN_EXE_loft"))
        .arg("--cells")
        .arg(&cells)
        .args(["--writer", "c", "--check"])
        .env_remove("LOFT_DISCHARGE_BLOCK_JOIN");
    if let Some(s) = switch {
        cmd.env(s, "1");
    }
    // No python3 on this host: the nightly job is the census then.
    let out = cmd.output().ok()?;
    let _ = loft::file_access::remove_dir_all(&dir);
    Some((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
    ))
}

#[test]
fn both_writers_name_one_store_effect_for_a_discharge_block() {
    let Some((ok, text)) = census(None) else {
        eprintln!("skip: python3 unavailable");
        return;
    };
    assert!(ok, "the census check passes:\n{text}");
    assert!(
        text.contains("paired assignments") && !text.starts_with("0 paired"),
        "the census paired assignments (a check over nothing cannot fail):\n{text}"
    );
}

#[test]
fn the_census_check_fails_when_the_interpreter_joins_the_block_again() {
    let Some((ok, text)) = census(Some("LOFT_DISCHARGE_BLOCK_JOIN")) else {
        eprintln!("skip: python3 unavailable");
        return;
    };
    assert!(!ok, "the check must fail under the switch:\n{text}");
    assert!(
        text.contains("DIFFERS: re R1/b2=pre is lowered as displacing-rebind"),
        "the failure names the pair:\n{text}"
    );
}

/// `@FR-O-NoDiverge` / `@FR-O-Buffer` — a promoted return-buffer local rebound to a choice
/// (`if` or `match`) between a call and a parameter, then rebound to the parameter and in a loop,
/// agrees across `bind_lowering`, the interpreter, and the C backend without freeing or mutating
/// the caller's record or the caller's entry buffer (`#2695`).
///
/// @falsified-at: d63f9a324 — `--native` freed the caller's record on the `if` cell (double free /
/// poisoned read under `LOFT_STRICT_STORES=1 LOFT_POISON=1`); both backends corrupted/freed the
/// caller's record on the `match` cell (`20 20` on `--interpret`, UAF on `--native`).
#[test]
fn promoted_return_buffer_choice_rebind_agrees_across_backends() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!("loft-2695-guard-{}", std::process::id()));
    let _ = loft::file_access::remove_dir_all(&dir);
    loft::file_access::create_dir_all(&dir).expect("scratch dir");
    let prog = dir.join("guard_2695.loft");
    let src = "\
struct P { v: integer }
fn w(a: P) -> P { if a.v % 2 == 1 { return P { v: a.v + 100 }; } a }
fn f_if(p: P, k: integer) -> P {
  if k < 0 { return P { v: 55 }; }
  x = if k == 0 { w(p) } else { p };
  x = p;
  for i in 0..3 {
    if i == k { return x; }
    x = P { v: i + 20 };
  }
  x
}
fn f_match(p: P, k: integer) -> P {
  if k < 0 { return P { v: 55 }; }
  x = match k { 0 => w(p), _ => p };
  x = p;
  for i in 0..3 {
    if i == k { return x; }
    x = P { v: i + 20 };
  }
  x
}
fn main() {
  q = P { v: 4 };
  qo = P { v: 5 };
  t_if = 0;
  t_match = 0;
  for k in -1..4 {
    t_if += f_if(q, k).v;
    t_match += f_match(q, k).v;
  }
  r_odd = f_if(qo, 0).v + f_match(qo, 1).v;
  println(\"{t_if} {t_match} {r_odd} {q.v} {qo.v}\");
  assert(t_if == 55 + 4 + 20 + 21 + 22 && q.v == 4, \"if {t_if} {q.v}\");
  assert(t_match == 55 + 4 + 20 + 21 + 22 && qo.v == 5, \"match {t_match} {qo.v}\");
  assert(r_odd == 5 + 20, \"odd {r_odd}\");
}
";
    loft::file_access::write(&prog, src).expect("write guard");
    let loft_bin = env!("CARGO_BIN_EXE_loft");
    for backend in ["--interpret", "--native-c"] {
        let out = loft::platform::process::harness_command(loft_bin)
            .args([backend, "--no-warnings"])
            .arg(&prog)
            .env("LOFT_STRICT_STORES", "1")
            .env("LOFT_POISON", "1")
            .env("LOFT_NATIVE_LEAK_CHECK", "1")
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_TIMEOUT", "60")
            .output()
            .expect("run loft");
        assert!(
            out.status.success() && out.stderr.is_empty(),
            "{backend} failed (status={:?}):\nstdout: {}\nstderr: {}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "122 122 25 4 5\n",
            "{backend} output"
        );
    }
    let cells = dir.join("cells.txt");
    loft::file_access::write(&cells, prog.display().to_string()).expect("write cells");
    if let Ok(out) = loft::platform::process::harness_command("python3")
        .arg(root.join("scripts/bind_census.py"))
        .arg("--loft")
        .arg(loft_bin)
        .arg("--cells")
        .arg(&cells)
        .args(["--writer", "c", "--check"])
        .output()
    {
        let text = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "bind_census disagreed:\n{text}");
    }
    let _ = loft::file_access::remove_dir_all(&dir);
}
