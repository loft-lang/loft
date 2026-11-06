// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! A call to a function declared LATER is never handed its caller's return buffer.
//!
//! The work-ref names (`__ref_N`) are claimed in a different order on the two parser passes: a
//! callee declared later has no known result on pass 1, so its call mints a buffer on pass 2
//! only.  Pass 1 promotes the return site's work-ref to the return-buffer ARGUMENT, and the
//! shifted pass-2 call claimed that name with a request of the same type — the call's answer
//! landed in the caller's record, and the local holding it freed the caller's store.  A work-ref
//! that resolves to an argument is reused only by the site that minted it
//! (`Function::argument_of_another_site`).  The values are asserted on both backends by
//! `tests/scripts/a-later-declared-call-is-never-handed-the-return-buffer.loft`; this pins the IR.
use std::path::{Path, PathBuf};

const GUARD: &str = "tests/scripts/a-later-declared-call-is-never-handed-the-return-buffer.loft";

#[test]
fn a_later_declared_call_gets_a_buffer_of_its_own() {
    let guard = Path::new(env!("CARGO_MANIFEST_DIR")).join(GUARD);
    let out = loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("introspect")
        .arg(&guard)
        .env("LOFT_TIMEOUT", "240")
        .output()
        .expect("spawn loft");
    assert!(
        out.status.success(),
        "introspect failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let head = text
        .lines()
        .find(|l| l.starts_with("fn n_f(") && l.contains("{#block"))
        .expect("f's IR");
    // The return buffer is f's last parameter: `…, <buf>:Ex) -> Ex["<buf>"]`.
    let buf = head
        .split_once(" -> Ex[\"")
        .and_then(|(_, r)| r.split_once('"'))
        .map(|(b, _)| b.to_owned())
        .expect("f answers through a return buffer");
    let call = text
        .lines()
        .find(|l| l.contains("= n_condition("))
        .expect("the call to condition");
    assert!(
        !call.contains(&format!("{buf}(0))")),
        "the call to `condition` is handed f's return buffer `{buf}`:\n{call}"
    );
}
