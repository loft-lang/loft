// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN119 arc A — the attach handshake and one call crossing a process
// boundary.
//
// The plan's first green cell is "a no-op library (`fn ping() -> integer`)
// crossing the boundary". These tests drive that directly through
// `lib_placement::Worker`, so the transport is pinned independently of the
// interpreter wiring that routes a `use`d library's calls into it: if a later
// change breaks the wire, this fails and names the wire, rather than surfacing
// as a mysterious library call.
//
// The gate the plan states — identical output under `inproc` and `process` —
// lives with the language wiring; what is proven here is that a call CAN cross
// with its value, its type, and its error behaviour intact.

#![cfg(unix)]

use loft::file_access as fa;
use loft::host::Value;
use loft::lib_placement::Worker;
use std::path::{Path, PathBuf};

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = base.join("loft-placement").join(name);
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Write a one-file library package and return its root.
fn library(dir: &Path, name: &str, source: &str) -> PathBuf {
    let root = dir.join(name);
    fa::create_dir_all(root.join("src")).expect("create package");
    fa::write(
        root.join("loft.toml"),
        format!("[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[library]\nplacement = \"process\"\n"),
    )
    .expect("write manifest");
    fa::write(root.join("src").join(format!("{name}.loft")), source).expect("write source");
    root
}

/// Start a worker for `pkg`, with the real `loft` binary as the worker exe.
fn worker(name: &str, pkg: &Path) -> Worker {
    // SAFETY-ish: tests in one binary share the environment, but every test
    // here sets the same value, so the race is benign.
    unsafe { std::env::set_var("LOFT_WORKER_EXE", env!("CARGO_BIN_EXE_loft")) };
    Worker::spawn(name, pkg, &workspace_root().join("default"), Path::new(""))
        .expect("worker did not come up")
}

#[test]
fn ping_crosses_the_boundary() {
    let dir = scratch("ping");
    let pkg = library(
        &dir,
        "pinglib",
        "pub fn ping(x: integer) -> integer {\n    x + 1\n}\n",
    );
    let mut w = worker("pinglib", &pkg);
    assert_eq!(w.call("ping", &[Value::Int(41)]), Ok(Value::Int(42)));
    // Reusable: the worker holds the library across calls, which is what lets a
    // placed library keep state exactly as an in-process one does.
    assert_eq!(w.call("ping", &[Value::Int(-1)]), Ok(Value::Int(0)));
}

/// @PLN119 arc B — the layout gate's control.
///
/// A record graph in the call arena is read as the RECEIVING program's own type,
/// so the two sides must lay that type out identically. The dispatcher refuses
/// to place a function whose layout the worker reports differently — and a
/// comparison that cannot report a difference would pass forever, so this asks
/// the question the gate asks and requires a DIFFERENT answer from two libraries
/// that lay the same-named type out differently.
///
/// The pair is chosen so the two structs are the same size in bytes and differ
/// only in field ORDER. A gate that compared sizes, or that compared nothing and
/// returned a constant, passes on a field-swap and mis-reads every value.
#[test]
fn the_layout_gate_can_tell_two_programs_apart() {
    let dir = scratch("layout");
    let same = "pub struct P { a: integer, b: u8 }\n\
                pub fn take(p: P) -> integer { p.a + p.b }\n";
    let swapped = "pub struct P { b: u8, a: integer }\n\
                   pub fn take(p: P) -> integer { p.a + p.b }\n";
    let mut a = worker("layout_a", &library(&dir, "layout_a", same));
    let mut b = worker("layout_b", &library(&dir, "layout_b", same));
    let mut c = worker("layout_c", &library(&dir, "layout_c", swapped));

    let la = a.layout("take").expect("worker a reported a layout");
    let lb = b.layout("take").expect("worker b reported a layout");
    let lc = c.layout("take").expect("worker c reported a layout");

    assert_eq!(
        la, lb,
        "two workers holding the SAME library must agree, or the gate refuses \
         every placement it should allow"
    );
    assert_ne!(
        la, lc,
        "the gate saw no difference between `P {{ a, b }}` and `P {{ b, a }}` — \
         it is blind, and a placed call would read one program's bytes as the \
         other's type"
    );
    // A signature with nothing compound in it has nothing to disagree about, and
    // must not be made to look like it does.
    let mut s = worker(
        "layout_s",
        &library(
            &dir,
            "layout_s",
            "pub fn plain(x: integer) -> integer { x }\n",
        ),
    );
    assert_eq!(
        s.layout("plain").as_deref(),
        Ok(".,."),
        "a scalar signature's layout is its shape, not a hash"
    );
}

#[test]
fn every_scalar_shape_survives_the_crossing() {
    let dir = scratch("scalars");
    let pkg = library(
        &dir,
        "scalarlib",
        "pub fn echo_int(v: integer) -> integer { v }\n\
         pub fn echo_bool(v: boolean) -> boolean { v }\n\
         pub fn echo_single(v: single) -> single { v }\n\
         pub fn echo_text(v: text) -> text { v }\n\
         pub fn joined(a: text, b: text) -> text { \"{a}/{b}\" }\n",
    );
    let mut w = worker("scalarlib", &pkg);

    // Distinctive values, and the EDGES: a negative integer catches a narrow or
    // unsigned misread on the wire, which is exactly the class of bug the
    // `#native` ABI contract already had to learn the hard way.
    assert_eq!(
        w.call("echo_int", &[Value::Int(-9_007_199_254_740_993)]),
        Ok(Value::Int(-9_007_199_254_740_993))
    );
    assert_eq!(
        w.call("echo_bool", &[Value::Bool(true)]),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        w.call("echo_bool", &[Value::Bool(false)]),
        Ok(Value::Bool(false))
    );
    assert_eq!(
        w.call("echo_single", &[Value::Float(0.5)]),
        Ok(Value::Float(0.5))
    );

    // Text: empty, multi-byte, and longer than the 256-byte threshold where a
    // literal stops being inlined — the shapes @PLN119 Q1 proved are
    // store-resident, re-checked here on the CALL path rather than the file one.
    for probe in ["", "hello", "héllo — ünïcøde ✓", &"x".repeat(4096)] {
        assert_eq!(
            w.call("echo_text", &[Value::Text(probe.to_string())]),
            Ok(Value::Text(probe.to_string())),
            "text shape did not survive: {:?}",
            &probe[..probe.len().min(40)]
        );
    }
    assert_eq!(
        w.call(
            "joined",
            &[Value::Text("a".into()), Value::Text("b".into())]
        ),
        Ok(Value::Text("a/b".into()))
    );
}

#[test]
fn the_probe_can_fail() {
    // A test suite that only ever asserts agreement proves nothing about its own
    // sensitivity. Ask for a wrong answer and require the harness to say so.
    let dir = scratch("canary");
    let pkg = library(
        &dir,
        "canarylib",
        "pub fn ping(x: integer) -> integer { x + 1 }\n",
    );
    let mut w = worker("canarylib", &pkg);
    assert_ne!(
        w.call("ping", &[Value::Int(41)]),
        Ok(Value::Int(41)),
        "the wire returned the argument unchanged — it is not running the library"
    );
}

#[test]
fn a_fault_in_the_library_is_the_callers_error_not_a_dead_worker() {
    // The invariant names error behaviour explicitly: a placed library must fail
    // the way an in-process one does. A call that cannot be served comes back as
    // an error, and — the part that matters — the worker is still there
    // afterwards.
    let dir = scratch("fault");
    let pkg = library(
        &dir,
        "faultlib",
        "pub fn ping(x: integer) -> integer { x + 1 }\n",
    );
    let mut w = worker("faultlib", &pkg);

    let missing = w.call("no_such_function", &[Value::Int(1)]);
    assert!(
        missing.is_err(),
        "calling an absent fn should error, got {missing:?}"
    );

    let wrong_arity = w.call("ping", &[]);
    assert!(
        wrong_arity.is_err(),
        "wrong arity should error, got {wrong_arity:?}"
    );

    assert_eq!(
        w.call("ping", &[Value::Int(1)]),
        Ok(Value::Int(2)),
        "the worker must survive a rejected call"
    );
}

/// A worker that dies outright is an ERROR, not a hang (@PLN119 arc D).
///
/// This is the case the transport could not survive by construction: the caller
/// waits on a futex word in the shared mapping, and a dead worker will never
/// write it. `FUTEX_WAIT` does not fail when its counterpart disappears — it
/// waits forever — so the wait has to end by LOOKING at the child rather than by
/// being woken.
///
/// The test kills the worker with the caller mid-wait, which is why the call
/// runs on its own thread: if the fix regresses, this hangs, and the harness has
/// to be able to say so rather than hang with it.
#[test]
fn a_worker_killed_mid_call_is_an_error_not_a_hang() {
    let dir = scratch("death");
    let pkg = library(
        &dir,
        "deathlib",
        // Long enough that the worker is still busy when the kill lands, so the
        // caller is genuinely parked rather than racing it.
        "pub fn slow(n: integer) -> integer {\n\
         \x20   acc = 0;\n\
         \x20   for i in 0..n { acc += i; }\n\
         \x20   acc\n\
         }\n\
         pub fn ping(x: integer) -> integer { x + 1 }\n",
    );
    let mut w = worker("deathlib", &pkg);
    // Prove the worker is really serving before killing it, or a pass could
    // just be reporting a worker that never came up at all.
    assert_eq!(w.call("ping", &[Value::Int(1)]), Ok(Value::Int(2)));
    let pid = w.child_id();

    // `Worker` is Send but deliberately not Sync — the dispatcher serialises
    // calls — so the call MOVES to its own thread and the kill happens here.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let died = w.call("slow", &[Value::Int(4_000_000_000)]);
        // A gone worker must stay gone, and say so at once rather than wait again.
        let after = w.call("ping", &[Value::Int(1)]);
        let _ = tx.send((died, after));
    });

    std::thread::sleep(std::time::Duration::from_millis(400));
    // SIGKILL: the worker gets no chance to answer, which is the whole point.
    let killed = std::process::Command::new("kill")
        .arg("-KILL")
        .arg(pid.to_string())
        .status()
        .expect("run kill");
    assert!(killed.success(), "could not kill the worker (pid {pid})");

    let (died, after) = rx
        .recv_timeout(std::time::Duration::from_secs(20))
        .expect("the caller never returned — a dead worker hung the call");
    let msg = died.expect_err("a killed worker cannot have answered");
    assert!(
        msg.message.contains("deathlib"),
        "the failure must name the library: {msg}"
    );
    assert!(after.is_err(), "a gone worker must stay gone: {after:?}");
}

#[test]
fn a_library_that_cannot_load_is_reported_against_its_name() {
    let dir = scratch("badload");
    let pkg = library(&dir, "badlib", "pub fn broken( -> integer {\n");
    unsafe { std::env::set_var("LOFT_WORKER_EXE", env!("CARGO_BIN_EXE_loft")) };
    let started = Worker::spawn(
        "badlib",
        &pkg,
        &workspace_root().join("default"),
        Path::new(""),
    );
    match started {
        Ok(_) => panic!("a library that does not parse must not report a ready worker"),
        Err(e) => {
            let msg = e.to_string();
            assert!(
                msg.contains("badlib"),
                "the failure must name the library; got: {msg}"
            );
        }
    }
}
