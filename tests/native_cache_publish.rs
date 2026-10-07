// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The native binary cache publishes ATOMICALLY.
//!
//! `--native` compiles into a per-process scratch dir and then publishes the result to a
//! shared, content-keyed path `<script dir>/.loft/cache/native-<hash>`. That publish is the
//! one step several concurrent runs of the SAME source contend on, and it used to be a plain
//! `fs::copy`: the destination is truncated in place and then streamed, while a concurrent
//! reader's `exists()` is true throughout and `cache_safe_to_execute` reads symlink, owner
//! and mode but never SIZE. The loser exec'd a 0-byte-and-growing ELF and died with no
//! stdout and no stderr — a red gate whose only evidence was an empty output block.
//!
//! Measured before the fix as `make ci` failing in two runs of three on
//! `alias_link_baseline::baseline_leak_clean_native`, whose two native cells compile one
//! source concurrently.
//!
//! ⚠ **This file is an end-to-end smoke test, NOT the guard for that defect.** It was
//! written to reproduce the race and does not: it PASSES on the pre-fix build (measured —
//! 18 concurrent cold-cache runs, and sccache makes each compile fast enough that the
//! processes barely overlap at the publish). A timing window this narrow cannot be pinned
//! by racing it. The guard that actually falsifies is deterministic and lives beside the
//! code: `native_utils::publish_cached_binary_tests` asserts the destination's INODE is
//! swapped rather than rewritten in place, which is the property — truncating the live
//! inode is exactly what lets a reader's file empty out underneath it. That one fails on
//! the pre-fix publish (same inode on both sides).
//!
//! What this file still earns its 1.4s for is the end-to-end shape the unit test cannot
//! see: concurrent `--native` runs of one source each exec a COMPLETE binary.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

const PROG: &str = "fn main() {\n  v = [1, 2, 3];\n  println(\"cache publish OK {v[1]}\");\n}\n";
const WANT: &str = "cache publish OK 2";

/// One round: wipe the cache beside the source, then start `n` runs at once.
/// Returns the failures as `(exit, stdout, stderr)` so an empty output block is visible.
fn race_round(dir: &std::path::Path, src: &std::path::Path, n: usize) -> Vec<String> {
    let _ = fa::remove_dir_all(dir.join(".loft"));
    let children: Vec<_> = (0..n)
        .map(|_| {
            loft::platform::process::harness_command(loft_bin())
                .arg("--native")
                .arg(src)
                .env("LOFT_TIMEOUT", "180")
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("spawn loft")
        })
        .collect();
    let mut bad = Vec::new();
    for child in children {
        let out = child.wait_with_output().expect("wait");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() || !stdout.contains(WANT) {
            bad.push(format!(
                "[{}] stdout={:?} stderr={:?}",
                out.status,
                stdout.trim(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
    }
    bad
}

/// Concurrent cold-cache builds of ONE source all produce a complete, runnable binary.
#[test]
fn concurrent_cold_cache_runs_all_succeed() {
    let dir = std::env::temp_dir().join(format!("loft_cache_race_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("mkdir");
    let src = dir.join("racer.loft");
    fa::write(&src, PROG).expect("write");

    let mut failures = Vec::new();
    for round in 0..3 {
        for f in race_round(&dir, &src, 6) {
            failures.push(format!("round {round}: {f}"));
        }
    }
    let _ = fa::remove_dir_all(&dir);

    assert!(
        failures.is_empty(),
        "concurrent --native runs of one source must each exec a COMPLETE cached binary; \
         an empty stdout+stderr with a non-zero status is the truncated-publish signature\n{}",
        failures.join("\n")
    );
}

/// The probe is not vacuous: the same harness reports a failure when one occurs.
/// A deliberately broken program must be caught by the very check the guard above uses.
#[test]
fn harness_can_fail() {
    let dir = std::env::temp_dir().join(format!("loft_cache_race_neg_{}", std::process::id()));
    fa::create_dir_all(&dir).expect("mkdir");
    let src = dir.join("racer.loft");
    fa::write(&src, "fn main() {\n  println(\"something else\");\n}\n").expect("write");
    let bad = race_round(&dir, &src, 2);
    let _ = fa::remove_dir_all(&dir);
    assert_eq!(
        bad.len(),
        2,
        "the harness must report a program that does not print {WANT}"
    );
}
