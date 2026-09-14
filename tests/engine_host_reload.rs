// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN18 phase 02 — tier 0 live reload, end-to-end: editing a named fn of a
//! RUNNING kernel server swaps it live (fn-ref dispatch + patched call
//! sites), with world state preserved across the swap.  The matrix:
//!
//! 1. clean edit → new behavior, counter keeps counting (no restart);
//! 2. broken edit → the OLD body keeps serving (stale meaning beats a dead
//!    loop), the loop survives;
//! 3. the fix-up edit → swaps again (v1 → v2 of the temp-def chain);
//! 4. a signature change → rejected; the last good body keeps serving.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "common/mod.rs"]
mod common;

/// VM-aware deadline: CI runners are slow and CONTENDED (parallel test
/// binaries + native-build storms) — scale every wait there so timing
/// reflects the machine, not the meaning.
/// Disk-backed scratch for test fixtures.  `std::env::temp_dir()` is a
/// RAM-backed tmpfs on dev boxes (small quota, shared across sessions), and
/// loft's cache-next-to-source rule would put every `--native` fixture's
/// binary cache there too — the disk-quota stall class.  `target/` lives on
/// disk and is cleaned with the build tree.
fn test_tmp() -> std::path::PathBuf {
    let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/test-tmp");
    let _ = std::fs::create_dir_all(&dir);
    dir
}

fn vm_deadline(secs: u64) -> Instant {
    // Stretch the budget when the machine is shared — see `common::deadline_scale`.
    Instant::now() + Duration::from_secs(secs * common::deadline_scale())
}

const PORT_BASE: u16 = 18093;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/loft")
}

struct Guard(Option<Child>);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn ws_connect(port: u16) -> TcpStream {
    let deadline = vm_deadline(15);
    let stream = loop {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            break s;
        }
        assert!(Instant::now() < deadline, "server never listened");
        std::thread::sleep(Duration::from_millis(100));
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let req = "GET /ws HTTP/1.1\r\nHost: x\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
               Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n";
    (&stream).write_all(req.as_bytes()).unwrap();
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        (&stream).read_exact(&mut b).unwrap();
        head.push(b[0]);
    }
    assert!(String::from_utf8_lossy(&head).contains("101"));
    stream
}

fn ws_send(stream: &TcpStream, text: &str) {
    let mask = [9u8, 8, 7, 6];
    let bytes = text.as_bytes();
    let mut frame = vec![0x81u8, 0x80 | bytes.len() as u8];
    frame.extend_from_slice(&mask);
    frame.extend(bytes.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    let mut s = stream;
    s.write_all(&frame).unwrap();
}

fn ws_recv(stream: &TcpStream) -> String {
    let mut s = stream;
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).unwrap();
    let len = (hdr[1] & 0x7F) as usize;
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).unwrap();
    String::from_utf8(payload).unwrap()
}

/// Poll until the echo's prefix matches `want`; returns the counter suffix.
fn await_prefix(ws: &TcpStream, want: &str) -> i64 {
    let deadline = vm_deadline(10);
    loop {
        ws_send(ws, "ping");
        let r = ws_recv(ws);
        if let Some(rest) = r.strip_prefix(want) {
            return rest
                .split('#')
                .nth(1)
                .and_then(|n| n.parse().ok())
                .unwrap_or(-1);
        }
        assert!(
            Instant::now() < deadline,
            "echo never gained prefix {want:?} (last: {r:?})"
        );
        std::thread::sleep(Duration::from_millis(250));
    }
}

const BODY_A: &str = "\"A:{p}#{n}\"";

fn program(body: &str, sig: &str) -> String {
    let port = common::bind_port(PORT_BASE);
    format!(
        r#"
use engine_host;


fn reply({sig}) -> text {{
  {body}
}}

struct W {{ n: integer not null }}

fn main() {{
  w = W {{ n: 0 }};
  engine_host::run({port}, 50000,
    fn(ev: engine_host::Event) {{
      if ev.kind == 1 {{
        w.n = w.n + 1;
        engine_host::send(ev.cid, reply(ev.payload, w.n));
      }}
    }},
    fn() {{ }});
}}
"#
    )
}

// @speed 4.1
#[test]
fn live_reload_swaps_a_running_fn() {
    let port = common::bind_port(PORT_BASE);
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let dir = test_tmp().join(format!("eh_reload_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let prog = dir.join("srv.loft");
    let sig = "p: text, n: integer";
    std::fs::write(&prog, program(BODY_A, sig)).unwrap();

    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let child = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&prog)
        .current_dir(&root)
        .env("LOFT_LIVE_RELOAD", "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn server");
    let _guard = Guard(Some(child));

    let ws = ws_connect(port);

    // Baseline.
    let n0 = await_prefix(&ws, "A:ping");
    assert!(n0 >= 1);

    // 1. Clean edit → live swap, counter continuity (the world survived).
    std::fs::write(&prog, program("\"B:{p}#{n}\"", sig)).unwrap();
    let n1 = await_prefix(&ws, "B:ping");
    assert!(
        n1 > n0,
        "counter must continue across the swap ({n0} -> {n1})"
    );

    // 2. Broken edit → the old body keeps serving; the loop survives.
    std::fs::write(&prog, program("\"C:{p}#{n}\" +", sig)).unwrap();
    std::thread::sleep(Duration::from_millis(800));
    let n2 = await_prefix(&ws, "B:ping");
    assert!(
        n2 > n1,
        "still serving the last good body after a broken edit"
    );

    // 3. The fix-up → swaps to the corrected body.
    std::fs::write(&prog, program("\"C:{p}#{n}\"", sig)).unwrap();
    let n3 = await_prefix(&ws, "C:ping");
    assert!(n3 > n2);

    // 4. Signature change → rejected; the last good body keeps serving.
    std::fs::write(
        &prog,
        program("\"D:{p}#{n}\"", "p: text, n: integer, x: integer"),
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(800));
    let n4 = await_prefix(&ws, "C:ping");
    assert!(n4 > n3, "signature changes never half-apply");

    let _ = std::fs::remove_dir_all(&dir);
}

/// #346/#347 — the reload host must install from ANY cwd (the shadow session
/// inherits the resolved stdlib instead of opening a relative `default`), and
/// a program whose only diagnostics are WARNINGS must still get a watcher
/// (the shadow gate is errors-only, parity with the main session).
#[test]
fn reload_installs_from_foreign_cwd_with_warnings() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    // `v[i]` with no defensive check is the canonical standing warning.
    let prog_src = "fn main() {\n  v = [1, 2, 3];\n  i = 1;\n  if v[i] != null {\n    println(\"x\");\n  }\n}\n";
    let dir = std::env::temp_dir().join(format!("eh_reload_cwd_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    assert!(
        !dir.join("default").exists(),
        "probe cwd must not contain a stdlib dir"
    );
    let prog = dir.join("warny.loft");
    std::fs::write(&prog, prog_src).unwrap();
    let out = Command::new(loft_bin())
        .env("LOFT_LIVE_RELOAD", "1")
        .env("LOFT_OFFLINE", "1")
        .args(["--interpret"])
        .arg(&prog)
        .current_dir(&dir) // the foreign cwd: no default/, no lib/
        .output()
        .expect("run under reload");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("live-reload: watching"),
        "reload must install from a foreign cwd despite warnings, stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("reload disabled"),
        "warnings alone must never disable reload, stderr:\n{stderr}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// #350 + #351 — a live edit lands in a MODULE file (not the entry), and the
/// reloaded body resolves the same names its file could: a lib-qualified call
/// (`engine_host::clients()` — the shadow snippet parse keeps the session's
/// import scoping) and a cross-file user fn (the emitted call targets the
/// RUNNING state's code positions; the shadow itself never generates code).
// @speed 1.6
#[test]
fn live_reload_module_file_with_lib_and_cross_file_calls() {
    if !loft_bin().exists() {
        eprintln!("skipping: release loft not built");
        return;
    }
    let dir = std::env::temp_dir().join(format!("eh_reload_libs_{}", std::process::id()));
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    // The module resolves what ITS file imports — the library and a sibling module — and
    // nothing of the file that `use`s it: a fresh parse refuses a module calling its
    // importer's `double`, and a reload that accepted it was accepting a program the
    // language refuses (the shadow session used to parse the program under the prelude's
    // source id, where every name read as global — @PLN162 step 14).
    let module = src.join("viewmod.loft");
    std::fs::write(
        &module,
        "use engine_host;\nuse mathmod;\n\npub fn view_msg(n: integer) -> text {\n    \"view {n}\"\n}\n",
    )
    .unwrap();
    std::fs::write(
        src.join("mathmod.loft"),
        "pub fn double(n: integer) -> integer {\n    n * 2\n}\n",
    )
    .unwrap();
    let main = src.join("main.loft");
    std::fs::write(
        &main,
        r#"use viewmod;
use engine_host;

fn main() {
    frames = 0;
    engine_host::run_local(50000,
        fn(ev: engine_host::Event) {},
        fn() {
            frames += 1;
            println(view_msg(frames));
            if frames >= 600 {
                engine_host::client_stop();
            }
        });
}
"#,
    )
    .unwrap();
    let out_path = dir.join("stdout.txt");
    let out_file = std::fs::File::create(&out_path).unwrap();
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let child = Command::new(loft_bin())
        .arg("--interpret")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&main)
        .current_dir(&root)
        .env("LOFT_LIVE_RELOAD", "1")
        .env("LOFT_OFFLINE", "1")
        .stdout(Stdio::from(out_file))
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn local game");
    let _guard = Guard(Some(child));

    let await_marker = |marker: &str| -> bool {
        let deadline = vm_deadline(20);
        while Instant::now() < deadline {
            if std::fs::read_to_string(&out_path)
                .unwrap_or_default()
                .contains(marker)
            {
                return true;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        false
    };
    assert!(await_marker("view "), "baseline body must run first");

    // The MODULE edit: lib-qualified + cross-file calls in the new body.
    std::fs::write(
        &module,
        "use engine_host;\nuse mathmod;\n\npub fn view_msg(n: integer) -> text {\n    \"VIEW c={engine_host::clients()} d={double(n)}\"\n}\n",
    )
    .unwrap();
    assert!(
        await_marker("VIEW c=0 d="),
        "module edit must land live with lib + cross-file calls resolved (#350/#351); got:\n{}",
        std::fs::read_to_string(&out_path).unwrap_or_default()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// The port band reproduces POSIX `cksum` — the checksum `find_problems.sh` has always piped
/// the checkout path through.
///
/// The band is the one thing here that cannot be checked from inside a single checkout: a
/// shared band is invisible to both runs that share it, which is what made loft#1193 read as a
/// browser defect rather than as a port belonging to somebody else.  What CAN be checked is
/// that the formula is the same one, so moving it out of the shell script did not re-shuffle
/// which checkout sits where — and the expected values below come from `cksum` itself, not
/// from this implementation.
#[test]
fn the_port_band_reproduces_posix_cksum() {
    // $ printf '%s' <path> | cksum
    for (path, cksum) in [
        ("/home/jurjens/workspace/loft", 1_514_545_013_u64),
        ("/home/jurjens/workspace/loft2", 2_450_841_579),
        ("/home/jurjens/workspace/loft-8c", 85_609_831),
    ] {
        let expected = u16::try_from(cksum % 6 + 1).unwrap() * 2000;
        assert_eq!(
            common::port_band_of(path),
            expected,
            "band for `{path}` disagrees with `cksum` ({cksum})"
        );
        assert!(
            19322_u32 + u32::from(expected) < 32768,
            "band {expected} puts the top base port inside the ephemeral range"
        );
    }
}
