// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN119 arc E — `placement = "remote"`: the same gate, over a socket.
//
// The invariant does not change because the wire did:
//
//   A call to a library is indistinguishable — in type, effect,
//   ownership/lifetime, and error behaviour — from the same call in-process.
//   Where it runs is deployment policy, not source.
//
// So these tests are the parity gate with a third placement in it. The library
// and the consumer are byte-identical across all three runs; only the manifest
// line and an environment variable differ.
//
// What makes remote worth its own file rather than a row in
// `placement_parity.rs` is the setup: a server has to be started, reached, and
// stopped, and a test that leaked one would wedge a port for every later run.

#![cfg(target_os = "linux")]

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn scratch(name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = base.join("loft-placement-remote").join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn write_library(root: &Path, mode: &str, source: &str) {
    let pkg = root.join("libs").join("svc");
    std::fs::create_dir_all(pkg.join("src")).expect("create package");
    std::fs::write(
        pkg.join("loft.toml"),
        format!(
            "[package]\nname = \"svc\"\nversion = \"0.1.0\"\n\n\
             [library]\nplacement = \"{mode}\"\n"
        ),
    )
    .expect("write manifest");
    std::fs::write(pkg.join("src").join("svc.loft"), source).expect("write source");
}

struct Run {
    stdout: String,
    stderr: String,
    code: i32,
}

/// A server this test owns, killed when the test ends however it ends.
struct Server {
    child: Child,
    address: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    /// Start `loft --lib-server` on an ephemeral port and wait until it says it
    /// is listening.
    ///
    /// Port 0 and then READING the address back, rather than picking a number:
    /// a hard-coded port collides with whatever else is on the machine, and the
    /// failure is a confusing "connection refused" in an unrelated test.
    fn start(root: &Path) -> Server {
        let mut child = Command::new(env!("CARGO_BIN_EXE_loft"))
            .arg("--lib-server")
            .arg("127.0.0.1:0")
            .arg(root.join("libs").join("svc"))
            .arg("--default")
            .arg(workspace_root().join("default"))
            .current_dir(root)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start the library server");
        // The first line is `loft: serving <pkg> on <addr>`.
        let mut out = child.stdout.take().expect("server stdout");
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        while out.read(&mut byte).map(|n| n == 1).unwrap_or(false) {
            if byte[0] == b'\n' {
                break;
            }
            buf.push(byte[0]);
        }
        let line = String::from_utf8_lossy(&buf).into_owned();
        let address = line
            .rsplit_once(" on ")
            .map(|(_, a)| a.trim().to_string())
            .unwrap_or_else(|| panic!("the server did not announce an address: {line:?}"));
        Server { child, address }
    }
}

/// Run `consumer` with the library at `mode`. `address` is set only for remote.
fn run(root: &Path, consumer: &Path, address: Option<&str>) -> Run {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_loft"));
    cmd.arg("--interpret")
        .arg("--lib")
        .arg(root.join("libs"))
        .arg(consumer)
        .env("LOFT_TIMEOUT", "60")
        .env("LOFT_NO_NATIVE_LIBS", "1");
    if let Some(a) = address {
        cmd.env("LOFT_REMOTE_SVC", a);
    }
    let out = cmd.output().expect("failed to invoke loft");
    Run {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        code: out.status.code().unwrap_or(-1),
    }
}

/// The whole matrix, on all THREE placements.
///
/// The rows are arc B's: every scalar shape, text both ways, a struct, a `text`
/// field, a `vector<struct>`, an empty vector, a null, and — the one that needs
/// the arena to travel in BOTH directions — a callee writing to a compound
/// parameter.
///
/// Over a socket the copy-back is not a shared page becoming visible; it is the
/// argument arena's bytes coming home in the answer. A transport that sent the
/// arena only outward would pass every other row here and fail this one.
#[test]
fn a_remote_library_answers_exactly_what_a_local_one_does() {
    let library = "pub struct P { x: integer, label: text }\n\
                   pub fn add(a: integer, b: integer) -> integer { a + b }\n\
                   pub fn e_i8(v: i8) -> i8 { v }\n\
                   pub fn e_u16(v: u16) -> u16 { v }\n\
                   pub fn e_single(v: single) -> single { v }\n\
                   pub fn flag(b: boolean) -> boolean { !b }\n\
                   pub fn shout(s: text) -> text { \"<{s}>\" }\n\
                   pub fn sum_p(p: P) -> integer { p.x + len(p.label) }\n\
                   pub fn make_p(x: integer) -> P { P { x: x, label: \"m{x}\" } }\n\
                   pub fn make_v(n: integer) -> vector<P> {\n\
                   \x20   out: vector<P> = [];\n\
                   \x20   for i in 0..n { out += [P { x: i, label: \"e{i}\" }]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn total(v: vector<P>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for e in v { t += e.x; }\n\
                   \x20   t\n\
                   }\n\
                   pub fn bump(p: P) -> integer {\n\
                   \x20   p.x = p.x + 100;\n\
                   \x20   p.label = \"{p.label}!\";\n\
                   \x20   p.x\n\
                   }\n\
                   pub fn push(v: vector<P>) -> integer {\n\
                   \x20   v += [P { x: 99, label: \"new\" }];\n\
                   \x20   len(v)\n\
                   }\n\
                   pub fn maybe(p: P?) -> integer { if p == null { -1 } else { p.x } }\n";
    let consumer = "use svc::*;\n\
                    fn main() {\n\
                    \x20   println(\"scalars {add(2, 3)} {e_i8(-128)} {e_u16(65535)} \
                     {e_single(0.5 as single)} {flag(true)}\");\n\
                    \x20   println(\"text {shout(\"héllo ✓\")}\");\n\
                    \x20   p = P { x: 7, label: \"abc\" };\n\
                    \x20   println(\"struct {sum_p(p)}\");\n\
                    \x20   q = make_p(4);\n\
                    \x20   println(\"made {q.x} {q.label}\");\n\
                    \x20   v = make_v(5);\n\
                    \x20   println(\"vector {total(v)} {len(v)} {v[4].label}\");\n\
                    \x20   empty: vector<P> = [];\n\
                    \x20   println(\"empty {total(empty)} {len(empty)}\");\n\
                    \x20   absent: P? = null;\n\
                    \x20   println(\"null {maybe(q)} {maybe(absent)}\");\n\
                    \x20   println(\"write {bump(p)} sees {p.x} {p.label}\");\n\
                    \x20   println(\"append {push(v)} sees {len(v)} {v[5].label}\");\n\
                    \x20   acc = 0;\n\
                    \x20   for i in 0..50 { acc += total(v) + add(i, 1); }\n\
                    \x20   println(\"loop {acc}\");\n\
                    }\n";
    let root = scratch("matrix");
    let consumer_path = root.join("consumer.loft");
    std::fs::write(&consumer_path, consumer).expect("write consumer");

    write_library(&root, "inproc", library);
    let inproc = run(&root, &consumer_path, None);
    assert_eq!(
        inproc.code, 0,
        "the in-process run must succeed: {}",
        inproc.stderr
    );
    assert!(
        inproc.stdout.contains("write 107 sees 107 abc!"),
        "the in-process reference is not what this test assumes: {:?}",
        inproc.stdout
    );

    write_library(&root, "process", library);
    let placed = run(&root, &consumer_path, None);
    assert_eq!(
        inproc.stdout, placed.stdout,
        "process placement diverged\n--- inproc ---\n{}\n--- process ---\n{}",
        inproc.stdout, placed.stdout
    );

    write_library(&root, "remote", library);
    let server = Server::start(&root);
    let remote = run(&root, &consumer_path, Some(&server.address));
    assert_eq!(
        remote.code, 0,
        "the remote run failed: {}\n{}",
        remote.stderr, remote.stdout
    );
    assert_eq!(
        inproc.stdout, remote.stdout,
        "REMOTE placement diverged\n--- inproc ---\n{}\n--- remote ---\n{}",
        inproc.stdout, remote.stdout
    );
    assert_eq!(
        inproc.stderr, remote.stderr,
        "remote placement changed what was reported on stderr\n--- inproc ---\n{}\n--- remote ---\n{}",
        inproc.stderr, remote.stderr
    );
}

/// A value far larger than an arena's initial size, over a socket.
///
/// Locally, growth is a file resize the reader re-maps. Remotely there is no
/// mapping at all — the image simply gets bigger, and both ends have to accept
/// one they did not size for. The two failure modes this catches are a receiver
/// that kept its old capacity and a sender that shipped its capacity rather than
/// its live bytes.
#[test]
fn a_value_larger_than_the_arena_crosses_a_socket() {
    let library = "pub fn range_v(n: integer) -> vector<integer> {\n\
                   \x20   out: vector<integer> = [];\n\
                   \x20   for i in 0..n { out += [i]; }\n\
                   \x20   out\n\
                   }\n\
                   pub fn sum_v(v: vector<integer>) -> integer {\n\
                   \x20   t = 0;\n\
                   \x20   for e in v { t += e; }\n\
                   \x20   t\n\
                   }\n\
                   pub fn small(n: integer) -> integer { n + 1 }\n";
    let consumer = "use svc::*;\n\
                    fn main() {\n\
                    \x20   big = range_v(50000);\n\
                    \x20   println(\"big {len(big)} {big[49999]} {sum_v(big)}\");\n\
                    \x20   println(\"after {small(1)}\");\n\
                    \x20   acc = 0;\n\
                    \x20   for i in 0..20 { acc += sum_v(range_v(100)); }\n\
                    \x20   println(\"loop {acc}\");\n\
                    }\n";
    let root = scratch("grow");
    let consumer_path = root.join("consumer.loft");
    std::fs::write(&consumer_path, consumer).expect("write consumer");

    write_library(&root, "inproc", library);
    let inproc = run(&root, &consumer_path, None);
    assert_eq!(inproc.code, 0, "in-process run: {}", inproc.stderr);
    // Hand-computed: sum 0..50000 = 1249975000; 20 × sum(0..100) = 20 × 4950.
    assert!(
        inproc.stdout.contains("big 50000 49999 1249975000")
            && inproc.stdout.contains("loop 99000"),
        "the in-process reference is not what this test assumes: {:?}",
        inproc.stdout
    );

    write_library(&root, "remote", library);
    let server = Server::start(&root);
    let remote = run(&root, &consumer_path, Some(&server.address));
    assert_eq!(
        inproc.stdout, remote.stdout,
        "a large value did not survive the socket\n--- inproc ---\n{}\n--- remote ---\n{}",
        inproc.stdout, remote.stdout
    );
    // The call AFTER the big one matters as much: an arena that grew must not
    // leave the next small call reading a stale length.
    assert!(remote.stdout.contains("after 2"), "{:?}", remote.stdout);
}

/// A `remote` library with nowhere to be refuses, and says what to set.
///
/// Falling back to in-process would be the wrong kindness: a library declared
/// remote runs somewhere else on someone else's data, so running it HERE is a
/// different deployment, not a slower one.
#[test]
fn a_remote_library_with_no_address_refuses_and_says_which_variable() {
    let root = scratch("noaddr");
    let consumer_path = root.join("consumer.loft");
    std::fs::write(
        &consumer_path,
        "use svc::*;\nfn main() { println(\"v = {add(2, 3)}\"); }\n",
    )
    .expect("write consumer");
    write_library(
        &root,
        "remote",
        "pub fn add(a: integer, b: integer) -> integer { a + b }\n",
    );
    let out = run(&root, &consumer_path, None);
    assert!(
        !out.stdout.contains("v = 5"),
        "it ran the library anyway: {:?}",
        out.stdout
    );
    assert_ne!(out.code, 0, "a library with nowhere to run must refuse");
    assert!(
        out.stderr.contains("LOFT_REMOTE_SVC") && out.stderr.contains("svc"),
        "the refusal must name the library and the variable to set: {}",
        out.stderr
    );
}

/// A server that is not there, and one that stops answering mid-run.
///
/// The remote transport has a failure the local one does not: the far side is
/// someone else's process on someone else's machine, so "it died" is a guess
/// where "it stopped answering" is a fact — and the message says the second.
#[test]
fn a_server_that_stops_answering_is_an_error_not_a_hang() {
    let library = "pub fn slow(n: integer) -> integer {\n\
                   \x20   acc = 0;\n\
                   \x20   for i in 0..n { acc += i; }\n\
                   \x20   acc\n\
                   }\n\
                   pub fn ping(x: integer) -> integer { x + 1 }\n";
    let root = scratch("gone");
    let consumer_path = root.join("consumer.loft");
    std::fs::write(
        &consumer_path,
        "use svc::*;\n\
         fn main() {\n\
         \x20   println(\"before {ping(1)}\");\n\
         \x20   println(\"after {slow(4000000000)}\");\n\
         }\n",
    )
    .expect("write consumer");
    write_library(&root, "remote", library);

    // Nothing listening at all.
    let out = run(&root, &consumer_path, Some("127.0.0.1:1"));
    assert_ne!(out.code, 0, "an unreachable server must refuse");
    assert!(
        out.stderr.contains("svc") && out.stderr.contains("127.0.0.1:1"),
        "the refusal must name the library and the address: {}",
        out.stderr
    );

    // Listening, then killed with a call outstanding.
    let mut server = Server::start(&root);
    let address = server.address.clone();
    let consumer = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg("--lib")
        .arg(root.join("libs"))
        .arg(&consumer_path)
        .env("LOFT_TIMEOUT", "60")
        .env("LOFT_NO_NATIVE_LIBS", "1")
        .env("LOFT_REMOTE_SVC", &address)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the consumer");
    std::thread::sleep(std::time::Duration::from_millis(700));
    let _ = server.child.kill();
    let _ = server.child.wait();

    let out = consumer
        .wait_with_output()
        .expect("consumer did not finish");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("before 2"),
        "the consumer never reached the call: {stdout:?} / {stderr}"
    );
    assert_eq!(
        out.status.code(),
        Some(1),
        "a server that went away must end the program as a loft error, not a \
         signal ({:?}); stderr: {stderr}",
        out.status
    );
    assert!(
        stderr.contains("stopped answering") && stderr.contains("svc"),
        "the error must say what happened and to which library: {stderr}"
    );
}
