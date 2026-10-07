// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN119 arc F, second half — the engine-host wire.
//
// The engine host holds sockets, a client table and an event queue. Running it
// out of process is the plan's most demanding consumer, and it is demanding for
// a reason the earlier arcs never touched: the library is not a function you
// call and forget, it is a SERVICE with state that outlives every call, driven
// in a loop.
//
// What is proven here is that a real client, over a real websocket, sees the
// same conversation whether the kernel is in this process or in a worker — with
// the consumer's source byte-identical across both runs.

// @PLN184 C2 approved exemption (owner, 2026-10-07): library placement (`lib_placement::wire`, an mmap wire, is `cfg(unix)` in src) has no Windows equivalent yet; Windows substitute: none
#![cfg(unix)]

use loft::file_access as fa;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Disk-backed scratch: `std::env::temp_dir()` is a small shared tmpfs on dev
/// boxes, and loft caches next to the source.
fn scratch(name: &str) -> PathBuf {
    let dir = workspace_root()
        .join("target/test-tmp/engine-host-placed")
        .join(name);
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create scratch dir");
    dir
}

/// A port nobody else in this run is using, taken by binding and releasing.
///
/// Asking the OS beats picking a number: a hard-coded port collides with
/// whatever else is on the machine, and the failure surfaces as a confusing
/// timeout in an unrelated test.
fn free_port() -> u16 {
    let l = std::net::TcpListener::bind("127.0.0.1:0").expect("bind an ephemeral port");
    l.local_addr().expect("read it back").port()
}

/// Kills the consumer if this test leaves by any path but the happy one — a
/// leaked one keeps its port and its worker, and the next run then fails for a
/// reason that has nothing to do with the code.
struct Guard(Option<Child>);

impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Copy `lib/engine_host` into `dir`, with `placement` set.
///
/// The real source, not a paraphrase of it — the point of the test is that the
/// shipped library works this way. Only the manifest differs, which is the only
/// thing the plan claims decides where it runs.
fn place_engine_host(dir: &Path, placement: &str) -> PathBuf {
    let real = workspace_root().join("lib").join("engine_host");
    let pkg = dir.join("libs").join("engine_host");
    fa::create_dir_all(pkg.join("src")).expect("create package");
    fa::copy(
        real.join("src").join("engine_host.loft"),
        pkg.join("src").join("engine_host.loft"),
    )
    .expect("copy the library source");
    let manifest = fa::read_to_string(real.join("loft.toml")).expect("read manifest");
    let manifest = manifest.replace(
        "[native]",
        &format!("placement = \"{placement}\"\n\n[native]"),
    );
    fa::write(pkg.join("loft.toml"), manifest).expect("write manifest");
    dir.join("libs")
}

fn ws_connect(port: u16, deadline: Instant) -> TcpStream {
    let stream = loop {
        if let Ok(s) = TcpStream::connect(("127.0.0.1", port)) {
            break s;
        }
        assert!(
            Instant::now() < deadline,
            "the kernel never listened on {port}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .expect("set a read timeout");
    let req = "GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
               Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
               Sec-WebSocket-Version: 13\r\n\r\n";
    (&stream)
        .write_all(req.as_bytes())
        .expect("send the upgrade");
    let mut head = Vec::new();
    let mut b = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        (&stream).read_exact(&mut b).expect("read the 101 head");
        head.push(b[0]);
    }
    let head = String::from_utf8_lossy(&head);
    assert!(head.contains("101"), "upgrade accepted: {head}");
    stream
}

fn ws_send(stream: &TcpStream, text: &str) {
    let mask = [0x21u8, 0x43, 0x65, 0x87];
    let bytes = text.as_bytes();
    assert!(bytes.len() <= 125, "the probe frames are short by design");
    let mut frame = vec![0x81u8, 0x80 | bytes.len() as u8];
    frame.extend_from_slice(&mask);
    frame.extend(bytes.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    (&mut { stream })
        .write_all(&frame)
        .expect("send a websocket frame");
}

fn ws_recv(stream: &TcpStream) -> String {
    let mut s = stream;
    let mut hdr = [0u8; 2];
    s.read_exact(&mut hdr).expect("read a frame header");
    let len = match hdr[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            s.read_exact(&mut b).expect("read a 16-bit length");
            u16::from_be_bytes(b) as usize
        }
        n => n as usize,
    };
    let mut payload = vec![0u8; len];
    s.read_exact(&mut payload).expect("read a frame payload");
    String::from_utf8(payload).expect("a text frame")
}

/// The consumer: the kernel loop with the `while` on THIS side.
///
/// `engine_host::run` takes closures, and closures do not cross a process
/// boundary — so the loop is inverted. That is not a workaround; it is what the
/// `turn()` surface exists for, and it reads about the same.
///
/// loft#937 — the loop stops on the CONJUNCTION of "`bye` arrived" and "a tick
/// came due", not on `bye` alone. The transcript this consumer prints is compared
/// between placements verbatim, so every fact in it has to be one both placements
/// are REQUIRED to agree on. Stopping the moment `bye` landed made
/// `ticks-positive` a sample of how fast the exchange happened to run instead: on
/// an idle machine both crossed a 5 ms tick boundary, and under parallel suite
/// load one could finish the whole three-message exchange inside a single
/// interval and report `false` against its sibling's `true`. Waiting for the tick
/// costs at most one interval and turns the sample into the guarantee the
/// assertion already assumed it was.
///
/// `TURN_BUDGET` is the backstop for the other direction: a kernel whose tick
/// never comes due must fail the assertion, not hang the suite until
/// `LOFT_TIMEOUT`. It is deliberately far above the ~5 ms wait — a turn is a
/// pump plus a drain, so even at a microsecond each the budget outlasts the
/// interval by orders of magnitude, and reaching it means the tick is genuinely
/// broken.
fn consumer_source(port: u16) -> String {
    format!(
        "use engine_host::*;\n\
         fn main() {{\n\
         \x20   if !listen({port}, 5000) {{ println(\"listen-failed\"); return; }}\n\
         \x20   println(\"ready\");\n\
         \x20   tick_count = 0;\n\
         \x20   msg_count = 0;\n\
         \x20   saw_bye = false;\n\
         \x20   turns = 0;\n\
         \x20   while alive() {{\n\
         \x20       t = turn(64);\n\
         \x20       if !t.running {{ break; }}\n\
         \x20       for ev in t.events {{\n\
         \x20           if ev.kind == 0 {{ println(\"joined clients={{clients()}}\"); }}\n\
         \x20           if ev.kind == 1 {{\n\
         \x20               msg_count += 1;\n\
         \x20               println(\"got {{ev.payload}}\");\n\
         \x20               broadcast(\"echo:{{ev.payload}}#{{msg_count}}\");\n\
         \x20               if ev.payload == \"bye\" {{ saw_bye = true; }}\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if t.tick {{ tick_count += 1; }}\n\
         \x20       turns += 1;\n\
         \x20       if saw_bye && (tick_count > 0 || turns > 2000000) {{ stop(); }}\n\
         \x20   }}\n\
         \x20   println(\"closed ticks-positive={{tick_count > 0}} seen={{msg_count}}\");\n\
         }}\n"
    )
}

/// One scenario, run with the kernel wherever `placement` says.
///
/// Returns the client's transcript and the consumer's own output — both, because
/// they answer different questions: the transcript says the SOCKETS behaved the
/// same, and the output says the loop did.
fn drive(name: &str, placement: &str) -> (Vec<String>, String) {
    let dir = scratch(name);
    let port = free_port();
    let libs = place_engine_host(&dir, placement);
    let consumer = dir.join("consumer.loft");
    fa::write(&consumer, consumer_source(port)).expect("write consumer");

    let child = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("--interpret")
        .arg("--lib")
        .arg(&libs)
        .arg(&consumer)
        .env("LOFT_TIMEOUT", "120")
        .env("LOFT_NO_NATIVE_LIBS", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the consumer");
    let mut guard = Guard(Some(child));

    let deadline = Instant::now() + Duration::from_secs(30);
    let ws = ws_connect(port, deadline);
    let mut transcript = Vec::new();
    for msg in ["hello", "again", "bye"] {
        ws_send(&ws, msg);
        transcript.push(ws_recv(&ws));
    }
    drop(ws);

    let out = guard
        .0
        .take()
        .expect("the consumer was started")
        .wait_with_output()
        .expect("the consumer did not finish");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "the consumer failed under {placement}: {}\n{stdout}",
        String::from_utf8_lossy(&out.stderr)
    );
    (transcript, stdout)
}

/// The engine host, in this process and in a worker, seen from a real client.
///
/// The rows this exercises that nothing before it did:
///
/// * the library is a SERVICE — `listen` binds a socket in the worker, and every
///   later `turn()` reads a queue that the worker has been filling between
///   calls. A placement that did not hold state across calls would answer an
///   empty first turn and nothing after.
/// * `turn()` answers a `Turn { boolean, boolean, vector<Event> }` — a struct
///   carrying a vector of structs with a text field each, which is the deepest
///   shape arc B's arena carries, produced by a real library rather than a probe.
/// * `broadcast` and `clients` are called from inside the loop, so the crossing
///   goes both ways within one frame.
#[test]
fn the_engine_host_serves_the_same_client_from_either_placement() {
    let (inproc_transcript, inproc_out) = drive("inproc", "inproc");
    assert_eq!(
        inproc_transcript,
        vec![
            "echo:hello#1".to_string(),
            "echo:again#2".to_string(),
            "echo:bye#3".to_string()
        ],
        "the in-process reference is not what this test assumes: {inproc_out}"
    );
    assert!(
        inproc_out.contains("joined clients=1") && inproc_out.contains("seen=3"),
        "the in-process reference did not see the client: {inproc_out}"
    );

    let (placed_transcript, placed_out) = drive("process", "process");
    assert_eq!(
        inproc_transcript, placed_transcript,
        "the client saw a different conversation when the kernel was placed\n\
         --- inproc ---\n{inproc_transcript:?}\n--- process ---\n{placed_transcript:?}"
    );
    assert_eq!(
        inproc_out, placed_out,
        "the consumer behaved differently when the kernel was placed\n\
         --- inproc ---\n{inproc_out}\n--- process ---\n{placed_out}"
    );
}
