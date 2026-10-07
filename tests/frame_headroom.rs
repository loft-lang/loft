// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@FR-R-FrameHeadroom` — the interpreter's direct-path push tests no capacity, because every
//! frame enters through `State::push_frame`, which makes room for the function's whole height
//! once.  The one structural fact that rests on: a `CallFrame` reaches `call_stack` only
//! through `push_frame`.  A new entry path that pushed a frame itself would run the function's
//! operators without that room.  The values are guarded by
//! `tests/scripts/a-frame-never-pushes-past-the-room-its-entry-made.loft`.
use loft::file_access as fa;
use std::path::Path;

#[test]
fn a_frame_reaches_the_call_stack_only_through_push_frame() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut sites = Vec::new();
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("read src") {
            let path = entry.expect("entry").path();
            if fa::is_dir(&path) {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = fa::read_to_string(&path).expect("read file");
                for (n, line) in text.lines().enumerate() {
                    let code = line.trim_start();
                    if !code.starts_with("//") && code.contains("call_stack.push(") {
                        sites.push(format!("{}:{}", loft::file_access::portable(&path), n + 1));
                    }
                }
            }
        }
    }
    assert_eq!(
        sites.len(),
        1,
        "a frame is pushed outside `State::push_frame`, which makes the frame's room \
         (@FR-R-FrameHeadroom): {sites:?}"
    );
    assert!(sites[0].contains("state/mod.rs"), "{sites:?}");
}

/// The guard program under `LOFT_HEADROOM_VERIFY=1`: after every op, the stack top lies in
/// the running frame's room, and the room in the stack buffer.  The value cells alone cannot
/// see a missing room — the store grows by 7/3 when it grows, so a push past the room
/// usually lands in slack — so this is the run that fails when a height is recorded too low
/// or a frame enters without its room.
#[test]
fn the_guard_program_holds_every_frame_inside_its_room() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/scripts/a-frame-never-pushes-past-the-room-its-entry-made.loft");
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg(&file)
        .env("LOFT_HEADROOM_VERIFY", "1")
        .env("LOFT_TIMEOUT", "300")
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("spawn loft");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.trim_end().ends_with("ok"),
        "{stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
