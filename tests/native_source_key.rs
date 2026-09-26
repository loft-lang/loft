// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN166 B3 — the source-keyed native fast path: a native run of an unchanged program
//! execs its cached binary without parsing, and NOTHING that would change the binary is
//! ever served from that key.  A wrong key here serves a stale binary in silence, so every
//! cell asserts the program's answer, not only the marker: the value printed is the fact,
//! `LOFT_TIMING`'s `native_source_key=` line says which path answered.
//!
//! Each cell runs `target/debug/loft --native` with the program cache forced on
//! (`LOFT_PROGRAM_CACHE=1`, a dev build has it off) in a private cache root.

use std::path::{Path, PathBuf};
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}
fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// A private root per cell: the script, its `.loft/cache` and the program cache all live
/// under it, so cells never share a sidecar.
fn fresh_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("loft_nsk_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("root");
    root
}

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Run {
    /// Which cache layer answered: `hit` / `miss` / `off` from the fast path's own line.
    fn source_key(&self) -> &str {
        self.marker("native_source_key=")
    }
    /// The generated-Rust-keyed binary cache's verdict (absent on a source-key hit, which
    /// never reaches it).
    fn binary_cache(&self) -> &str {
        self.marker("native_binary_cache=")
    }
    fn marker(&self, key: &str) -> &str {
        let Some(at) = self.stderr.find(key) else {
            return "(absent)";
        };
        let rest = &self.stderr[at + key.len()..];
        &rest[..rest.find([' ', '\n']).unwrap_or(rest.len())]
    }
}

fn run_with(root: &Path, script: &Path, args: &[&str], env: &[(&str, &str)]) -> Run {
    let mut cmd = Command::new(loft_bin());
    cmd.args(args).arg(script).current_dir(workspace_root());
    // A switch inherited from the harness's own environment would decline the fast path
    // in every cell and read as a red.  Strip them all, then set the cell's own.
    for (k, _) in std::env::vars_os() {
        if k.to_string_lossy().starts_with("LOFT_") {
            cmd.env_remove(&k);
        }
    }
    cmd.env("LOFT_PROGRAM_CACHE", "1")
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("LOFT_TIMING", "1")
        .env("LOFT_TIMEOUT", "300");
    for (k, v) in env {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("failed to invoke the loft binary");
    Run {
        ok: out.status.success(),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    }
}

fn run(root: &Path, script: &Path) -> Run {
    run_with(root, script, &["--native"], &[])
}

/// Two runs: the first compiles and publishes, the second must be the source-key hit.
fn warm_to_a_hit(root: &Path, script: &Path, want: &str) {
    let cold = run(root, script);
    assert!(
        cold.ok && cold.stdout.contains(want),
        "cold run: {}{}",
        cold.stdout,
        cold.stderr
    );
    assert_eq!(
        cold.source_key(),
        "miss",
        "a first run has no sidecar: {}",
        cold.stderr
    );
    let warm = run(root, script);
    assert!(
        warm.ok && warm.stdout.contains(want),
        "warm run: {}{}",
        warm.stdout,
        warm.stderr
    );
    assert_eq!(
        warm.source_key(),
        "hit",
        "the second run of an unchanged program must be served by its source key: {}",
        warm.stderr
    );
}

fn sidecar_of(root: &Path) -> PathBuf {
    let dir = root.join("cache").join("loft");
    let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("program cache dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("native"))
        .collect();
    assert_eq!(
        found.len(),
        1,
        "one sidecar under {}: {found:?}",
        dir.display()
    );
    found.remove(0)
}

fn cached_binary_of(root: &Path) -> PathBuf {
    let dir = root.join(".loft").join("cache");
    let mut found: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("binary cache dir")
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("prog-"))
        })
        .collect();
    assert_eq!(
        found.len(),
        1,
        "one cached binary under {}: {found:?}",
        dir.display()
    );
    found.remove(0)
}

const PROG_30: &str = "fn main() {\n  v = [5, 10, 15];\n  println(\"sum={v[0]+v[1]+v[2]}\");\n}\n";
const PROG_100: &str = "fn main() {\n  v = [5, 10, 85];\n  println(\"sum={v[0]+v[1]+v[2]}\");\n}\n";

/// Cells 1, 3, 9, 10, 11: an unchanged program is served from its source key; a comment-only
/// edit misses the key but still skips rustc; a sidecar whose binary is gone, whose text is
/// garbage, or whose fingerprint was altered is a miss and never a crash — and each of those
/// heals into a hit on the run after.
#[test]
fn an_unchanged_program_is_served_from_its_source_key_and_every_damaged_sidecar_misses() {
    let root = fresh_root("unchanged");
    let script = root.join("prog.loft");
    std::fs::write(&script, PROG_30).expect("script");
    warm_to_a_hit(&root, &script, "sum=30");

    // Cell 3 — a comment-only edit: the sources moved, so the source key misses and the
    // run re-keys.  (Whether the binary cache then hits is the live tier's question, not
    // this key's: a `--native` build embeds the program's source, so it recompiles.)
    std::fs::write(&script, format!("{PROG_30}// a comment\n")).expect("edit");
    let r = run(&root, &script);
    assert!(
        r.ok && r.stdout.contains("sum=30"),
        "{}{}",
        r.stdout,
        r.stderr
    );
    assert_eq!(
        r.source_key(),
        "miss",
        "an edited source is a new key: {}",
        r.stderr
    );
    let r = run(&root, &script);
    assert_eq!(
        r.source_key(),
        "hit",
        "re-keyed after the edit: {}",
        r.stderr
    );

    // Cell 9 — the binary the sidecar names is gone.
    std::fs::remove_file(cached_binary_of(&root)).expect("remove binary");
    let r = run(&root, &script);
    assert!(
        r.ok && r.stdout.contains("sum=30"),
        "{}{}",
        r.stdout,
        r.stderr
    );
    assert_eq!(
        r.source_key(),
        "miss",
        "a missing binary is a miss: {}",
        r.stderr
    );
    assert_eq!(r.binary_cache(), "miss", "…and a recompile: {}", r.stderr);
    let r = run(&root, &script);
    assert_eq!(
        r.source_key(),
        "hit",
        "republished and re-keyed: {}",
        r.stderr
    );

    // Cell 10 — garbage, then a truncated sidecar.
    let sidecar = sidecar_of(&root);
    let good = std::fs::read_to_string(&sidecar).expect("sidecar");
    assert_eq!(good.lines().count(), 3, "sig / fp / bin: {good}");
    std::fs::write(&sidecar, "not a sidecar\n").expect("garbage");
    let r = run(&root, &script);
    assert!(
        r.ok && r.stdout.contains("sum=30"),
        "{}{}",
        r.stdout,
        r.stderr
    );
    assert_eq!(r.source_key(), "miss", "garbage is a miss: {}", r.stderr);
    let r = run(&root, &script);
    assert_eq!(
        r.source_key(),
        "hit",
        "rewritten after garbage: {}",
        r.stderr
    );
    let two_lines: String = good.lines().take(2).map(|l| format!("{l}\n")).collect();
    std::fs::write(&sidecar, two_lines).expect("truncate");
    let r = run(&root, &script);
    assert_eq!(
        r.source_key(),
        "miss",
        "a truncated sidecar is a miss: {}",
        r.stderr
    );

    // Cell 11 — the fingerprint line altered by hand: another flag set's binary.
    let r = run(&root, &script);
    assert_eq!(r.source_key(), "hit", "{}", r.stderr);
    let good = std::fs::read_to_string(&sidecar).expect("sidecar");
    let altered = good.replace("\nfp ", "\nfp 0");
    assert_ne!(altered, good);
    std::fs::write(&sidecar, altered).expect("alter");
    let r = run(&root, &script);
    assert_eq!(
        r.source_key(),
        "miss",
        "a foreign fingerprint is a miss: {}",
        r.stderr
    );
    assert!(
        r.ok && r.stdout.contains("sum=30"),
        "{}{}",
        r.stdout,
        r.stderr
    );

    let _ = std::fs::remove_dir_all(&root);
}

/// Cell 2 — THE cell: an edit that changes the program is never answered by the old binary.
#[test]
fn an_edit_that_changes_the_program_is_never_served_stale() {
    let root = fresh_root("edit");
    let script = root.join("prog.loft");
    std::fs::write(&script, PROG_30).expect("script");
    warm_to_a_hit(&root, &script, "sum=30");
    std::fs::write(&script, PROG_100).expect("edit");
    let r = run(&root, &script);
    assert!(r.ok, "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("sum=100"),
        "the edited program's answer, never the cached binary's: {}{}",
        r.stdout,
        r.stderr
    );
    assert_eq!(r.source_key(), "miss", "{}", r.stderr);
    assert_eq!(
        r.binary_cache(),
        "miss",
        "new Rust, new binary: {}",
        r.stderr
    );
    let r = run(&root, &script);
    assert!(
        r.stdout.contains("sum=100") && r.source_key() == "hit",
        "{}{}",
        r.stdout,
        r.stderr
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Cell 12 — a source-key hit says what the cold run said: the parse did not run, so its
/// diagnostics are replayed from the manifest, through the same renderer.
#[test]
fn a_source_key_hit_renders_the_cold_runs_diagnostics() {
    let root = fresh_root("diag");
    let script = root.join("prog.loft");
    std::fs::write(
        &script,
        "struct DfPlayer { name: text, health: integer }\nfn main() {\n  p = DfPlayer { name: \"Bob\" };\n  println(\"{p.name}\");\n}\n",
    )
    .expect("script");
    let cold = run(&root, &script);
    assert!(
        cold.ok && cold.stdout.contains("Bob"),
        "{}{}",
        cold.stdout,
        cold.stderr
    );
    assert!(
        cold.stderr.contains("omitted-field-zero"),
        "the cold run must produce the advice this compares: {}",
        cold.stderr
    );
    let warm = run(&root, &script);
    assert_eq!(warm.source_key(), "hit", "{}", warm.stderr);
    let strip = |s: &str| -> String {
        s.lines()
            .filter(|l| !l.starts_with("LOFT_TIMING"))
            .map(|l| format!("{l}\n"))
            .collect()
    };
    assert_eq!(
        strip(&cold.stderr),
        strip(&warm.stderr),
        "a source-key hit must render what the cold run rendered"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Cell 5 — a flag that changes the binary is a different key: `--native-release` after
/// `--native` misses (and, since the binary cache keeps one entry per program, so does the
/// `--native` run after it), and each answers correctly.
#[test]
fn a_flag_that_changes_the_binary_is_a_different_key() {
    let root = fresh_root("flag");
    let script = root.join("prog.loft");
    std::fs::write(&script, PROG_30).expect("script");
    warm_to_a_hit(&root, &script, "sum=30");
    let r = run_with(&root, &script, &["--native-release"], &[]);
    assert!(
        r.ok && r.stdout.contains("sum=30"),
        "{}{}",
        r.stdout,
        r.stderr
    );
    assert_eq!(
        r.source_key(),
        "miss",
        "a release build is another binary: {}",
        r.stderr
    );
    assert_eq!(r.binary_cache(), "miss", "{}", r.stderr);
    let r = run_with(&root, &script, &["--native-release"], &[]);
    assert_eq!(
        r.source_key(),
        "hit",
        "the release key, re-run: {}",
        r.stderr
    );
    let r = run(&root, &script);
    assert!(
        r.ok && r.stdout.contains("sum=30"),
        "{}{}",
        r.stdout,
        r.stderr
    );
    assert_eq!(
        r.source_key(),
        "miss",
        "back to the semantics build: {}",
        r.stderr
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Cells 7, 8 — a `LOFT_*` switch outside the inert list declines the path (a codegen
/// switch changes the Rust), and the P254 kill switch turns every native cache off.
#[test]
fn a_switch_in_the_environment_declines_the_fast_path() {
    let root = fresh_root("env");
    let script = root.join("prog.loft");
    std::fs::write(&script, PROG_30).expect("script");
    warm_to_a_hit(&root, &script, "sum=30");
    let r = run_with(
        &root,
        &script,
        &["--native"],
        &[("LOFT_NO_VECTOR_HOIST", "1")],
    );
    assert!(
        r.ok && r.stdout.contains("sum=30"),
        "{}{}",
        r.stdout,
        r.stderr
    );
    assert_eq!(
        r.source_key(),
        "off",
        "a codegen switch declines the path: {}",
        r.stderr
    );
    let r = run_with(
        &root,
        &script,
        &["--native"],
        &[("LOFT_NATIVE_NO_CACHE", "1")],
    );
    assert!(
        r.ok && r.stdout.contains("sum=30"),
        "{}{}",
        r.stdout,
        r.stderr
    );
    assert_eq!(
        r.source_key(),
        "off",
        "the kill switch declines it: {}",
        r.stderr
    );
    assert_eq!(
        r.binary_cache(),
        "miss",
        "…and the binary cache: {}",
        r.stderr
    );
    // An inert variable does not.
    let r = run_with(
        &root,
        &script,
        &["--native"],
        &[("LOFT_MEMORY_LIMIT", "512M")],
    );
    assert_eq!(
        r.source_key(),
        "hit",
        "a run-time bound is inert: {}",
        r.stderr
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Cell 4 — an edited stdlib is never served from the source key.  A scratch copy of
/// `default/` (via `--path`) carries one probe function; the key is warmed, the probe
/// changed, and the next run must answer with the new value.
#[test]
fn an_edited_stdlib_is_never_served_from_the_source_key() {
    let root = fresh_root("stdlib");
    let dflt = root.join("default");
    std::fs::create_dir_all(&dflt).expect("scratch default/");
    for e in std::fs::read_dir(workspace_root().join("default")).expect("default/") {
        let p = e.expect("entry").path();
        if p.extension().and_then(|x| x.to_str()) == Some("loft") {
            std::fs::copy(&p, dflt.join(p.file_name().expect("name"))).expect("copy");
        }
    }
    let probe = dflt.join("99_b3_probe.loft");
    std::fs::write(&probe, "pub fn b3probe() -> integer { 1 }\n").expect("probe");
    let script = root.join("prog.loft");
    std::fs::write(&script, "fn main() { println(\"v={b3probe()}\"); }\n").expect("script");
    let path = root.to_string_lossy().into_owned();
    let args = ["--path", path.as_str(), "--native"];
    let cold = run_with(&root, &script, &args, &[]);
    assert!(
        cold.ok && cold.stdout.contains("v=1"),
        "{}{}",
        cold.stdout,
        cold.stderr
    );
    let warm = run_with(&root, &script, &args, &[]);
    assert_eq!(warm.source_key(), "hit", "{}", warm.stderr);
    std::fs::write(&probe, "pub fn b3probe() -> integer { 2 }\n").expect("edit");
    let r = run_with(&root, &script, &args, &[]);
    assert!(r.ok, "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("v=2"),
        "an edited stdlib must be re-read, never the cached binary: {}{}",
        r.stdout,
        r.stderr
    );
    assert_eq!(r.source_key(), "miss", "{}", r.stderr);
    let _ = std::fs::remove_dir_all(&root);
}

/// Cell 6 — an edited `--lib` dependency is never served from the source key.
#[test]
fn an_edited_library_is_never_served_from_the_source_key() {
    let root = fresh_root("lib");
    let lib = root.join("lib");
    std::fs::create_dir_all(&lib).expect("lib dir");
    let dep = lib.join("b3dep.loft");
    std::fs::write(&dep, "pub fn dep_value() -> integer { 28 }\n").expect("dep");
    let script = root.join("prog.loft");
    std::fs::write(
        &script,
        "use b3dep;\nfn main() { println(\"d={dep_value()}\"); }\n",
    )
    .expect("script");
    let libs = lib.to_string_lossy().into_owned();
    let args = ["--lib", libs.as_str(), "--native"];
    let cold = run_with(&root, &script, &args, &[]);
    assert!(
        cold.ok && cold.stdout.contains("d=28"),
        "{}{}",
        cold.stdout,
        cold.stderr
    );
    let warm = run_with(&root, &script, &args, &[]);
    assert_eq!(warm.source_key(), "hit", "{}", warm.stderr);
    std::fs::write(&dep, "pub fn dep_value() -> integer { 777 }\n").expect("edit");
    let r = run_with(&root, &script, &args, &[]);
    assert!(r.ok, "{}{}", r.stdout, r.stderr);
    assert!(
        r.stdout.contains("d=777"),
        "an edited library must be re-read, never the cached binary: {}{}",
        r.stdout,
        r.stderr
    );
    assert_eq!(r.source_key(), "miss", "{}", r.stderr);
    let _ = std::fs::remove_dir_all(&root);
}
