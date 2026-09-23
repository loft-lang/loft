// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN167 A2 — every DEBUGGER entry that touches a LINKED narrow local's frame slot.
//!
//! Such a local holds its type's FIELD encoding in the low bytes of its 8-byte slot
//! (@PLN167 decision 1, `@FR-B-Ref-Uniform`), so a reader must decode it and a writer must
//! encode it.  These drive the real `loft debug` CLI over a pipe, because the entries are
//! reached by pausing a run and a `.loft` file cannot pause itself; the userland half —
//! `stack_trace()` — is `tests/scripts/167-a-frame-reader-decodes-a-linked-narrow-local.loft`.
//!
//! Measured before the fix, and each is a DIFFERENT site:
//!
//! | entry | site | reported |
//! |---|---|---|
//! | `:vars` | `State::render_frame_local` | `b = 127` for `-1`, `d = 50` for `1050`, `c = 255` for null |
//! | `name = <lit>` | `State::set_frame_literal` | typing `-5` resumed the run with `123` |
//! | `:watch` | `State::render_scalar_bytes` + the region's width | `-1 -> 5 -> -7` read `127 -> 133 -> 121` |
//!
//! `a: u8` and `e: u16` were right throughout, their bias being zero — which is why every
//! case below uses `i8`, `limit(1000, 1100)` or `u8?` as well, and keeps a `u8` beside them
//! as the cell that cannot fail.
//!
//! Expression evaluation at the pause (`State::eval_frame_reenter`) was MEASURED and is
//! correct without a change — it re-enters the frame and reads through the ordinary ops — so
//! it is pinned here rather than fixed, to keep a later change from quietly breaking it.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

/// Every local here is named by a `&`, so every one holds its field encoding; `h` is the
/// unlinked control and keeps the full-width slot.
const PROG: &str = r#"type Lim = integer limit(1000, 1100);
fn main() {
  a: u8 = 250;          p = &a;
  b: i8 = -1;           q = &b;
  c: u8? = null;        r = &c;
  d: Lim = 1050;        s = &d;
  h: u8 = 9;
  n = 0;
  n = n + 1;
  println("{p} {q} {r == null} {s} {h} {n}");
}
"#;

/// Write `PROG` to a scratch file and run `loft debug <file>:<line>` with `cmds` on stdin.
fn debug_session(dir: &std::path::Path, line: u32, cmds: &str) -> String {
    let prog = dir.join("frame_readers.loft");
    std::fs::write(&prog, PROG).expect("write the probe");
    let mut child = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("debug")
        .arg(format!("{}:{line}", prog.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn loft debug");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(cmds.as_bytes())
        .expect("write commands");
    let out = child.wait_with_output().expect("wait");
    String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr)
}

/// A per-test scratch directory under the runner's `TMPDIR`, named for the test so two
/// running at once cannot write each other's probe.
fn scratch(who: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("loft_frame_readers_{who}"));
    std::fs::create_dir_all(&dir).expect("scratch dir");
    dir
}

/// `:vars` renders a linked narrow local's VALUE, not the code its slot stores.
#[test]
fn vars_decodes_every_linked_narrow_kind() {
    let dir = scratch("vars");
    let out = debug_session(&dir, 8, ":vars\n:quit\n");
    for (name, want) in [
        ("a", "a = 250"),  // bias 0 — right before the fix too, the cell that cannot fail
        ("b", "b = -1"),   // was 127, the store byte `enc_byte(-1, -128)`
        ("c", "c = null"), // was 255, the absence CODE read as a value
        ("d", "d = 1050"), // was 50, the bias not applied
        ("h", "h = 9"),    // NOT linked: the full-width control
    ] {
        assert!(
            out.contains(want),
            "{name}: expected `{want}` in the paused frame\n{out}"
        );
    }
}

/// An edit writes the local's own encoding, so the resumed run sees what was typed.
#[test]
fn an_edit_of_a_linked_narrow_local_lands_as_typed() {
    let dir = scratch("edit");
    let out = debug_session(&dir, 8, "b = -5\nd = 1099\n:vars\n:continue\n");
    assert!(out.contains("b = -5"), "the edit is displayed\n{out}");
    assert!(out.contains("d = 1099"), "the bias is applied\n{out}");
    // The program prints `{p} {q} {r == null} {s} {h} {n}` — the run must see the edits.
    assert!(
        out.contains("250 -5 true 1099 9 1"),
        "the resumed run sees what was typed\n{out}"
    );
}

/// A watch on a linked narrow local reports the values, and its region is the encoding's
/// width — eight bytes would both render the code and fire on the neighbouring slot.
#[test]
fn a_watch_on_a_linked_narrow_local_reports_values() {
    let dir = scratch("watch");
    let prog = dir.join("watch.loft");
    std::fs::write(
        &prog,
        "fn main() {\n  b: i8 = -1; q = &b;\n  n = 0;\n  q = 5;\n  n = n + 1;\n  q = -7;\n  println(\"{b} {n}\");\n}\n",
    )
    .expect("write");
    let mut child = Command::new(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("debug")
        .arg(format!("{}:3", prog.display()))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b":watch b\n:continue\n:continue\n:continue\n:quit\n")
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    // The debugger's banner and its watch reports go to STDERR; the program's own output to
    // stdout.  Both are needed, and taking only one is how this cell first read as "the run
    // finished and nothing fired".
    let text =
        String::from_utf8_lossy(&out.stdout).to_string() + &String::from_utf8_lossy(&out.stderr);
    assert!(
        text.contains("b changed -1 → 5"),
        "the first write, as values (was `127 → 133`)\n{text}"
    );
    assert!(
        text.contains("b changed 5 → -7"),
        "the second write (was `133 → 121`)\n{text}"
    );
}

/// Expression evaluation at the pause was already right; this pins it.
#[test]
fn an_expression_over_a_linked_narrow_local_is_unmoved() {
    let dir = scratch("eval");
    let out = debug_session(&dir, 8, "b + 0\nd * 2\n:quit\n");
    assert!(out.contains("-1"), "`b + 0` answers the value\n{out}");
    assert!(out.contains("2100"), "`d * 2` answers the value\n{out}");
}
