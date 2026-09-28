// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// A native compile sweeps the scratch directory of every artefact whose process is dead,
// and leaves the test runner's per-file binary cache alone (`platform::
// reclaim_dead_native_scratch`, called from `native_compile_space_ok` on every compile).
// A run that ends normally removes its own binary; a run killed from outside cannot, and
// with nothing else ever looking at the directory those accumulated one per killed process
// — sixteen thousand of them, 151 GB, on one box (RUN_BOUNDS.md § Scratch hygiene).

use std::process::Command;

fn rustc_available() -> bool {
    Command::new("rustc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

#[test]
fn a_native_compile_sweeps_dead_process_artefacts_and_keeps_the_test_cache() {
    if !rustc_available() {
        eprintln!("SKIP — rustc not on PATH");
        return;
    }
    let scratch = std::env::temp_dir().join(format!("loft_scratch_hygiene_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&scratch);
    std::fs::create_dir_all(&scratch).unwrap();
    // u32::MAX-1 is no real pid (Linux pid_max caps far below): provably dead.
    let dead_bin = scratch.join("loft_native_bin_4294967294");
    let dead_rs = scratch.join("loft_native_4294967294.rs");
    // The test runner's cache is named by STEM, not pid: it must survive a compile with room.
    let cache_bin = scratch.join("loft_test_native_some_file_bin");
    for f in [&dead_bin, &dead_rs, &cache_bin] {
        std::fs::write(f, b"planted").unwrap();
    }
    // Unique source per run: the binary cache is content-addressed, and only a cache MISS
    // compiles — which is where the sweep runs.
    let prog = scratch.join("hello.loft");
    std::fs::write(
        &prog,
        format!("fn main() {{ println(\"hi {}\"); }}\n", std::process::id()),
    )
    .unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--native")
        .arg(&prog)
        // Both spellings: `make ci` exports `LOFT_TMPDIR` beside `TMPDIR`, and the runtime
        // reads its scratch through `platform::scratch_dir`, which prefers the former.
        .env("TMPDIR", &scratch)
        .env("LOFT_TMPDIR", &scratch)
        .output()
        .expect("run loft");
    assert!(
        out.status.success(),
        "the native run must succeed: stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        !dead_bin.exists() && !dead_rs.exists(),
        "dead-process artefacts are swept by the compile"
    );
    assert!(
        cache_bin.exists(),
        "the test runner's per-file cache survives a compile with room"
    );
    let own: Vec<String> = std::fs::read_dir(&scratch)
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| n.starts_with("loft_native_"))
        .collect();
    assert!(
        own.is_empty(),
        "a run that ends normally leaves no artefact of its own, found {own:?}"
    );
    let _ = std::fs::remove_dir_all(&scratch);
}
