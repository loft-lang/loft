// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-H-ReadSurface` — concurrent `loft` processes share the cached stdlib and program images, and opening one
//! must not write it (`Store::open_read_surface`).  The mapping is shared between processes,
//! so a start that rebuilt the image's free-block tree in place handed another start, mid-
//! rebuild, a tree link to read as a record number: eight concurrent warm starts of a program
//! that only prints crashed eight times out of eight (`fl_rebuild` out of bounds, or a
//! segfault), and one start alone never did.  Each start here has its own script, so the
//! program cache misses and the shared stdlib image is what they meet.
use std::path::PathBuf;
use std::process::{Command, Stdio};

fn start(dir: &std::path::Path) -> std::process::Child {
    Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("--interpret")
        .arg(dir.join("p.loft"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .env("LOFT_TIMEOUT", "60")
        .env_remove("LOFT_NO_CACHE")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn loft")
}

#[test]
fn concurrent_warm_starts_leave_each_other_alone() {
    let base = std::env::temp_dir().join(format!("loft_warm_start_{}", std::process::id()));
    let dirs: Vec<PathBuf> = (0..8).map(|k| base.join(k.to_string())).collect();
    for d in &dirs {
        std::fs::create_dir_all(d).expect("mkdir");
        std::fs::write(
            d.join("p.loft"),
            "fn main() {\n  println(\"started\");\n}\n",
        )
        .expect("write the program");
    }
    // One start first, so the stdlib image is in the cache when the eight meet it.
    let warm = start(&dirs[0])
        .wait_with_output()
        .expect("the warming start");
    assert!(
        warm.status.success(),
        "{}",
        String::from_utf8_lossy(&warm.stderr)
    );
    for round in 0..3 {
        let children: Vec<_> = dirs.iter().map(|d| start(d)).collect();
        for (k, c) in children.into_iter().enumerate() {
            let out = c.wait_with_output().expect("wait");
            assert!(
                out.status.success() && String::from_utf8_lossy(&out.stdout) == "started\n",
                "round {round}, start {k}: {:?}\n{}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    let _ = std::fs::remove_dir_all(&base);
}
