// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The web library's natives, loft's `net` component (`[native] component = "net"`;
//! `src/c/components/net.c` over `src/c/loft_web.h`, built by `loft-components`), answer the same
//! on every backend: the interpreter calls its bridges, the Rust writer and the C backend its
//! typed symbols.  The program
//! reaches them through `tests/fixtures/web_natives`, their declarations word for word, against a
//! local server whose answers fix every expected line in advance: a body carrying a NUL and a
//! 0xff byte, a header, a request body and header echoed, a 404 with its body, a refused port, a
//! HEAD size; a WebSocket echo of a text and a binary frame, a ping the client must answer, a
//! round-robin group poll and a close.  The SERVER witnesses what the program cannot print: the
//! pong and the close frame.

use loft::file_access as fa;
use std::path::{Path, PathBuf};

#[path = "common/web_server.rs"]
mod web_server;

const PROGRAM: &str = r#"use web_natives;

fn main() {
  base = env_variable("WN_BASE") ?? "";
  ws = env_variable("WN_WS") ?? "";
  st = web_natives::http_do("GET", "{base}/bin", "", "");
  b = web_natives::http_body();
  println("BIN {st} {b.len()} {web_natives::byte_at(0, b)} {web_natives::byte_at(1, b)} {web_natives::byte_at(2, b)} {web_natives::byte_at(3, b)} {web_natives::byte_at(4, b)}");
  h = web_natives::http_headers().to_lowercase();
  println("HDR {h.contains("x-test: yes")}");
  st = web_natives::http_do("POST", "{base}/echo", "hello", "X-Mine: v1\n");
  println("POST {st} {web_natives::http_body()}");
  st = web_natives::http_do("GET", "{base}/missing", "", "");
  println("MISSING {st} {web_natives::http_body()}");
  st = web_natives::http_do("GET", "http://127.0.0.1:1/x", "", "");
  println("REFUSED {st}");
  println("SIZE {web_natives::http_head_size("{base}/sized")}");
  web_natives::pack_reset();
  web_natives::pack_u8(0);
  web_natives::pack_u8(255);
  web_natives::pack_u16_le(258);
  web_natives::pack_u32_le(4294967295);
  p = web_natives::pack_take();
  println("PACK {p.len()} {web_natives::byte_at(0, p)} {web_natives::byte_at(1, p)} {web_natives::byte_at(2, p)} {web_natives::byte_at(3, p)} {web_natives::byte_at(7, p)}");
  c = web_natives::ws_connect("{ws}/ws");
  web_natives::ws_client_send(c, "hi");
  got = "";
  for i in 0..400 {
    if web_natives::ws_client_recv(c) {
      got = "{web_natives::ws_client_message()} {web_natives::ws_client_opcode()}";
      break;
    }
    web_natives::sleep_ms(5);
  }
  println("WS {got}");
  web_natives::ws_client_send_binary(c, p);
  got = "";
  for i in 0..400 {
    if web_natives::ws_client_recv(c) {
      m = web_natives::ws_client_message();
      got = "{web_natives::ws_client_opcode()} {m.len()} {web_natives::byte_at(1, m)} {web_natives::byte_at(2, m)}";
      break;
    }
    web_natives::sleep_ms(5);
  }
  println("WSBIN {got}");
  d = web_natives::ws_connect("{ws}/ws");
  web_natives::ws_client_send(d, "g");
  web_natives::ws_group_clear();
  web_natives::ws_group_add(c);
  web_natives::ws_group_add(d);
  fired = -1;
  for i in 0..400 {
    fired = web_natives::ws_group_poll();
    if fired >= 0 { break; }
    web_natives::sleep_ms(5);
  }
  println("GROUP {fired == d} {web_natives::ws_client_message()}");
  println("BAD {web_natives::ws_connect("http://not-a-ws")}");
  web_natives::frame_yield();
  web_natives::ws_client_close(c);
  web_natives::ws_client_close(d);
  web_natives::sleep_ms(50);
  println("DONE");
}
"#;

/// The lines the program prints, fixed by the server's answers.
const WANT: [&str; 12] = [
    "BIN 200 5 65 0 255 66 10",
    "HDR true",
    "POST 200 POST hello v1",
    "MISSING 404 not here",
    "REFUSED 0",
    "SIZE 1234",
    "PACK 8 0 255 2 1 255",
    "WS echo:hi 1",
    "WSBIN 2 8 255 2",
    "GROUP true echo:g",
    "BAD -1",
    "DONE",
];

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("loft_web_natives_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&d);
    fa::create_dir_all(&d).expect("a scratch directory");
    d
}

/// Build the component where a `loft` beside `loft-components` finds it (`<binary>/../components`),
/// as `make components` does; the guard then exercises that default lookup, not an override.
fn build_component() {
    let loft = Path::new(env!("CARGO_BIN_EXE_loft"));
    let out = loft
        .parent()
        .and_then(Path::parent)
        .expect("the binary's directory")
        .join("components");
    let built = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft-components"))
        .arg("--out")
        .arg(&out)
        .arg("net")
        .output()
        .expect("loft-components runs");
    assert!(
        built.status.success(),
        "loft-components net:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
}

fn run(backend: &str, dir: &Path, port: u16) -> (String, String) {
    let file = dir.join("web.loft");
    fa::write(&file, PROGRAM).expect("write the program");
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg(backend)
        .arg("--no-warnings")
        .arg("--lib")
        .arg(&fixtures)
        .arg(&file)
        .current_dir(dir)
        .env_remove("LOFT_COMPONENTS")
        .env("LOFT_TIMEOUT", "120")
        .env("WN_BASE", format!("http://127.0.0.1:{port}"))
        .env("WN_WS", format!("ws://127.0.0.1:{port}"))
        .output()
        .expect("loft runs");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// The backends the component is called from.
fn backends() -> [&'static str; 3] {
    ["--interpret", "--native-rust", "--native-c"]
}

#[test]
fn the_web_natives_answer_the_same_on_every_backend() {
    if !loft::platform::c_runtime_posix_host() {
        // The component needs the C runtime's POSIX host layer (@PLN203 C8 is Windows').
        return;
    }
    build_component();
    for backend in backends() {
        let dir = scratch(&backend[2..]);
        let (port, seen) = web_server::start();
        let (out, err) = run(backend, &dir, port);
        for want in WANT {
            assert!(
                out.lines().any(|l| l == want),
                "{backend}: wants `{want}`\nstdout:\n{out}\nstderr:\n{err}"
            );
        }
        let seen = seen.lock().expect("seen");
        assert_eq!(
            seen.pongs, 1,
            "{backend}: the client answers the server's ping once"
        );
        assert_eq!(
            seen.closes, 2,
            "{backend}: each closed session sends its close frame"
        );
        let _ = fa::remove_dir_all(&dir);
    }
}
