// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN174 F2 — a FOREIGN store at the program level: `file_map(path)` serves a file's
//! bytes through the store contract with no copy, every read op answers what the copying
//! `read_bytes` answers (the cell file, on both backends and under the hoist verifier, the
//! strict-store and the poison switches), and a WRITE into the mapped bytes halts the
//! program with the author-facing runtime error whose advice is to copy first.  The halt is
//! `report_and_exit`, which leaves the process, so it is asserted here on the spawned
//! binary rather than by an `@EXPECT_FAIL` cell (which tolerates a panic, not an exit).
use std::path::PathBuf;
use std::process::Command;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

const READS: &str = "tests/scripts/174-foreign-file.loft";

const WRITE_REFUSED: &str = "\
fn wr_path() -> text { return \"loft_174_refused.tmp\"; }
fn main() {
  assert(write_bytes(wr_path(), [1, 2, 3, 4]), \"write\");
  m = file_map(wr_path()) ?? [];
  assert(len(m) == 4 && m[1] == 2, \"mapped {len(m)}\");
  m[1] = 9;
  println(\"reached: the write landed ({m[1]})\");
}
";

/// Run `src` (a file path, or a program written to a scratch dir) on `backend` with
/// `envs`, from a scratch directory of its own so the files the program writes land
/// nowhere shared.  Answers `(exit ok, stdout + stderr)`.
fn run(tag: &str, backend: &str, src: &str, inline: bool, envs: &[(&str, &str)]) -> (bool, String) {
    let root = std::env::temp_dir().join(format!("loft_174_{tag}_{}", std::process::id()));
    std::fs::create_dir_all(&root).expect("scratch dir");
    let path = if inline {
        let p = root.join("prog.loft");
        std::fs::write(&p, src).expect("write program");
        p
    } else {
        std::env::current_dir().expect("cwd").join(src)
    };
    let mut cmd = Command::new(loft_bin());
    cmd.arg(backend)
        .arg(&path)
        .current_dir(&root)
        .env("LOFT_TIMEOUT", "180")
        .env("RUST_BACKTRACE", "0");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("run loft");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&root);
    (out.status.success(), text)
}

#[test]
fn every_read_of_a_mapped_file_answers_the_copying_path_on_both_backends() {
    for backend in ["--interpret", "--native"] {
        for (tag, envs) in [
            ("plain", vec![]),
            ("verify", vec![("LOFT_HOIST_VERIFY", "1")]),
            ("strict", vec![("LOFT_STRICT_STORES", "1")]),
            ("poison", vec![("LOFT_POISON", "1")]),
            ("leak", vec![("LOFT_STORES", "warn")]),
        ] {
            let (ok, out) = run(tag, backend, READS, false, &envs);
            assert!(
                ok && out.contains("foreign file ok"),
                "{backend} {tag}: the reads cell must pass:\n{out}"
            );
            assert!(
                !out.to_lowercase().contains("leak"),
                "{backend} {tag}: the mapping must be released with its handle:\n{out}"
            );
        }
    }
}

#[test]
fn a_write_into_a_mapped_file_halts_with_the_copy_first_advice_on_both_backends() {
    for backend in ["--interpret", "--native"] {
        let (ok, out) = run("refused", backend, WRITE_REFUSED, true, &[]);
        assert!(!ok, "{backend}: the write must halt the program:\n{out}");
        assert!(
            out.contains("write to bytes the program does not own")
                && out.contains("copy them first"),
            "{backend}: the halt must be the foreign store's own advice:\n{out}"
        );
        assert!(
            !out.contains("reached: the write landed"),
            "{backend}: the write must not land:\n{out}"
        );
        assert!(
            !out.contains("out of bounds"),
            "{backend}: a refused write is not a corrupt reference:\n{out}"
        );
    }
}
