// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The disk janitor removes from a working tree only what its rules allow.
//!
//! `scripts/sweep_ignored.sh` deletes files, unattended, in every checkout on a development
//! box: what git ignores, once nothing wrote it for days.  What it must never take is
//! exactly what looks like it — an untracked file git does not ignore (new work), a tracked
//! file inside an ignored directory, a path the keep-list names, a worktree with uncommitted
//! work or with commits no ref holds, a worktree a live process has locked.
//!
//! The script carries one cell per rule (`--self-test`: a scratch repository it builds,
//! sweeps and inspects); this runs them, so a change to the sweep is checked by the gate and
//! not by the first checkout it runs in.

/// Every rule of the sweep holds on a scratch repository: what must go is gone, what must
/// stay is there, a dry run removes nothing, and a checkout with no keep-list is not swept.
#[test]
fn the_ignored_sweep_removes_only_what_its_rules_allow() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    // The interpreter is named (`platform::bash_program`): a bare `bash` on Windows is the
    // WSL launcher, which runs nothing.
    let out = loft::platform::process::harness_command(loft::platform::bash_program())
        .arg(root.join("scripts/sweep_ignored.sh"))
        .arg("--self-test")
        .current_dir(root)
        .output()
        .expect("run scripts/sweep_ignored.sh --self-test");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success() && stdout.contains("self-test ok"),
        "scripts/sweep_ignored.sh --self-test failed:\n{stdout}{stderr}"
    );
}
