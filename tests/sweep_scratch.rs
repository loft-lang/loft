// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The scratch sweep removes only what its rules allow, and stops where its caller bounds it.
//!
//! `scripts/sweep_scratch.sh` runs before every gate (through `scripts/disk_headroom.sh`) and
//! after every agent turn, in temp directories other processes are writing.  A running
//! process's `--native` artefacts must survive it — on every OS, which the `/proc` test it
//! once made did not hold on macOS — and a caller's bound (`--deadline`, `--enough-gb`) must
//! stop it, because an unbounded sweep spent a gate's whole 20-minute budget before its first
//! test (RUN_BOUNDS.md § Scratch hygiene).

use loft::file_access as fa;
use std::path::{Path, PathBuf};

/// A `bash -c` script, run to its end.  The fixture uses bash for what the script under test
/// will see the same way on every host: a process's id and an entry's age.
fn bash(script: &str, args: &[&Path]) -> String {
    let out = loft::platform::process::harness_command(loft::platform::bash_program())
        .arg("-c")
        .arg(script)
        .arg("bash")
        .args(args)
        .output()
        .expect("run bash");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Can the sweep tell a live process from a dead one here?  It reads the process list from
/// `ps -A -o pid=`; where that lists nothing (Git Bash's `ps` has no `-o`), no pid is known
/// dead and every pid's artefacts stay.
fn lists_processes() -> bool {
    let me = std::process::id().to_string();
    bash("ps -A -o pid= 2>/dev/null || true", &[])
        .split_whitespace()
        .any(|p| p == me)
}

/// A temp directory with: a live and a dead process's two native artefacts, `aged` loft
/// entries a week old, one fresh loft entry and one old entry that is not loft's.
fn fixture(name: &str, aged: usize) -> (PathBuf, u32, u32) {
    let dir = std::env::temp_dir().join(format!("sweep_scratch_{name}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).unwrap();
    let live = std::process::id();
    let mut child = loft::platform::process::harness_command(loft::platform::bash_program())
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let dead = child.id();
    child.wait().unwrap();
    for pid in [live, dead] {
        fa::write(dir.join(format!("loft_native_bin_{pid}")), "").unwrap();
        fa::write(dir.join(format!("loft_native_{pid}.rs")), "").unwrap();
    }
    for i in 0..aged {
        fa::create_dir_all(dir.join(format!("loft_old_{i}")).join("inner")).unwrap();
    }
    fa::create_dir_all(dir.join("loft_fresh")).unwrap();
    fa::create_dir_all(dir.join("other_old")).unwrap();
    // A week old, a directory included: `touch -t` with the stamp from GNU `date -d` (Linux,
    // Git Bash) or BSD `date -v` (macOS), the entries found by `find` in the directory, so no
    // command line carries hundreds of paths (Windows refuses one past 32 KiB).
    bash(
        "t=$(date -d '7 days ago' +%Y%m%d%H%M 2>/dev/null || date -v-7d +%Y%m%d%H%M) && \
         cd \"$1\" && find . -maxdepth 1 \\( -name 'loft_old_*' -o -name other_old \\) \
         -exec touch -t \"$t\" {} +",
        &[&dir],
    );
    (dir, live, dead)
}

fn sweep(args: &[&str], dir: &Path) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let out = loft::platform::process::harness_command(loft::platform::bash_program())
        .arg(root.join("scripts/sweep_scratch.sh"))
        .args(args)
        .arg(dir)
        .output()
        .expect("run scripts/sweep_scratch.sh");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn names(dir: &Path) -> Vec<String> {
    let mut v: Vec<String> = fa::read_dir(dir)
        .unwrap()
        .iter()
        .map(|e| e.file_name().unwrap().to_string())
        .collect();
    v.sort();
    v
}

/// Dead pids' artefacts and aged loft entries go — more than one removal batch of them; a live
/// process's artefacts, a fresh entry and another program's entry stay.
#[test]
fn the_scratch_sweep_keeps_a_live_process_and_fresh_work() {
    let (dir, live, dead) = fixture("rules", 600);
    sweep(&[], &dir);
    let mut want = vec![
        "loft_fresh".to_string(),
        format!("loft_native_{live}.rs"),
        format!("loft_native_bin_{live}"),
        "other_old".to_string(),
    ];
    if !lists_processes() {
        want.push(format!("loft_native_{dead}.rs"));
        want.push(format!("loft_native_bin_{dead}"));
    }
    want.sort();
    assert_eq!(names(&dir), want);
    fa::remove_dir_all(&dir).unwrap();
}

/// A deadline already passed, or the room already there, stops the sweep before it removes
/// anything: the caller's bound is the sweep's bound.
#[test]
fn the_scratch_sweep_stops_at_its_callers_bound() {
    for (name, args) in [
        ("deadline", vec!["--deadline", "1"]),
        ("enough", vec!["--enough-gb", "1", "--on", "."]),
    ] {
        let (dir, _, _) = fixture(name, 300);
        let before = names(&dir).len();
        sweep(&args, &dir);
        assert_eq!(
            names(&dir).len(),
            before,
            "{name}: the sweep went past its bound"
        );
        fa::remove_dir_all(&dir).unwrap();
    }
}
