// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN16 M5e slice 1 — drive the `--serve` browser server end-to-end over a real socket:
//! HTTP shell, then the WebSocket handshake + the debug protocol (launch → run → output →
//! terminated).  This is the browser's path, exercised without a browser.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

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

fn tmp_program(tag: &str, src: &str) -> std::path::PathBuf {
    // Tag-keyed: the two tests run in parallel in one process, so a pid-only name would
    // collide and one's cleanup would delete the other's file mid-`launch`.
    let p = test_tmp().join(format!("loft_serve_{tag}_{}.loft", std::process::id()));
    std::fs::write(&p, src).expect("write temp program");
    p
}

/// A path as a JSON-string-safe value: Windows separators (`C:\Users\…`) are JSON escape
/// characters and made every request unparseable on the Windows runner (`invalid escape \U`).
fn json_path(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "\\\\")
}

/// Spawn the server on a test port in a background thread (it loops forever; the thread
/// leaks, which is fine — nextest runs one process per test).  Returns once the port is
/// accepting.
fn start_server(port: u16, file: String) {
    std::thread::spawn(move || {
        let _ = loft::serve::run_serve("default", &[], port, &file);
    });
    let deadline = vm_deadline(10);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server did not start listening on {port}");
}

/// Open a connection and perform the WebSocket upgrade handshake; returns the live stream.
fn ws_connect(port: u16) -> BufReader<TcpStream> {
    let stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    let req = "GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
               Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
               Sec-WebSocket-Version: 13\r\n\r\n";
    (&stream).write_all(req.as_bytes()).unwrap();
    let mut reader = BufReader::new(stream);
    // Read the response head; the server replies 101 with the computed accept key.
    let mut status = String::new();
    reader.read_line(&mut status).unwrap();
    assert!(
        status.contains("101"),
        "expected 101 switching protocols, got: {status}"
    );
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line.trim_end().is_empty() {
            break;
        }
    }
    reader
}

/// Send a client text frame (masked, as the spec requires for client → server).
fn ws_send(stream: &TcpStream, text: &str) {
    let mask = [0x12u8, 0x34, 0x56, 0x78];
    let bytes = text.as_bytes();
    let mut frame = vec![0x81u8]; // FIN + text
    if bytes.len() <= 125 {
        frame.push(0x80 | bytes.len() as u8); // masked, 7-bit len
    } else {
        assert!(
            bytes.len() <= 0xFFFF,
            "test payloads fit the 16-bit length form"
        );
        frame.push(0x80 | 126);
        frame.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    }
    frame.extend_from_slice(&mask);
    frame.extend(bytes.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
    let mut s = stream;
    s.write_all(&frame).unwrap();
}

/// Read one server text frame (unmasked), returning its payload as a string.
fn ws_recv(reader: &mut BufReader<TcpStream>) -> String {
    let mut head = [0u8; 2];
    reader.read_exact(&mut head).unwrap();
    let len = match head[1] & 0x7F {
        126 => {
            let mut b = [0u8; 2];
            reader.read_exact(&mut b).unwrap();
            u16::from_be_bytes(b) as usize
        }
        n => n as usize,
    };
    let mut payload = vec![0u8; len];
    reader.read_exact(&mut payload).unwrap();
    String::from_utf8(payload).unwrap()
}

/// Drain frames until one satisfies `pred` (or a bounded count), collecting them.
fn recv_until(
    reader: &mut BufReader<TcpStream>,
    max: usize,
    pred: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut msgs = Vec::new();
    for _ in 0..max {
        let m = ws_recv(reader);
        let stop = pred(&m);
        msgs.push(m);
        if stop {
            break;
        }
    }
    msgs
}

#[test]
fn serve_http_shell_has_game_debug_strip() {
    // @PLN18 08-S7 editor support — the shell ships the game-debug strip
    // (fn-entry breakpoints, resume, rebuild, swap) and the control-channel
    // client that dials the game port directly.
    let path = tmp_program("gshell", "fn main() { print(\"x\") }\n");
    let port = 18794;
    start_server(port, path.to_string_lossy().into_owned());
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    use std::io::Write as _;
    write!(
        stream,
        "GET / HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut body = String::new();
    use std::io::Read as _;
    stream.read_to_string(&mut body).unwrap();
    for needle in [
        "id=\"gdbg\"",
        "id=\"gbp\"",
        "id=\"gresume\"",
        "id=\"grebuild\"",
        "id=\"gswap\"",
        "D!:bp ",
        "D!:swap auto",
        "D!:quit",
        "listening on ws:",
    ] {
        assert!(body.contains(needle), "shell must ship {needle:?}");
    }
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_http_shell_embeds_file_and_run_button() {
    let path = tmp_program("shell", "fn main() { print(\"x\") }\n");
    let port = 18781;
    start_server(port, path.to_string_lossy().into_owned());
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    (&stream)
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut body = String::new();
    let mut s = stream;
    let _ = s.read_to_string(&mut body);
    assert!(body.contains("id=\"run\""), "shell has a Run button");
    assert!(body.contains("ws://"), "shell opens a WebSocket");
    assert!(
        body.contains(&path.to_string_lossy().into_owned()),
        "shell embeds the file path for launch"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_ws_launch_run_streams_output() {
    let path = tmp_program(
        "run",
        "fn greet(n: integer) -> integer { n * 2 }\n\
         fn main() {\n  a = greet(21);\n  print(\"hi a={a}\")\n}\n",
    );
    let file = path.to_string_lossy().into_owned();
    let jfile = json_path(&path);
    let port = 18782;
    start_server(port, file.clone());
    let mut ws = ws_connect(port);

    // launch the file → {ok:true}
    ws_send(
        ws.get_ref(),
        &format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{jfile}\"}}"),
    );
    let launch = ws_recv(&mut ws);
    assert!(
        launch.contains("\"id\":1,\"ok\":true"),
        "launch ok: {launch}"
    );

    // run → the program's print streams as an `output` event, then `terminated`
    ws_send(ws.get_ref(), "{\"id\":2,\"req\":\"run\"}");
    let msgs = recv_until(&mut ws, 8, |m| m.contains("\"event\":\"terminated\""));
    let all = msgs.join("\n");
    assert!(
        all.contains("\"category\":\"stdout\",\"text\":\"hi a=42\""),
        "program output streamed over the websocket: {all}"
    );
    assert!(
        all.contains("\"event\":\"terminated\""),
        "run terminates: {all}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_ws_compile_streams_diagnostics() {
    // @PLN16 M5e slice 2 — compile over the websocket → a structured `diagnostics` event.
    let path = tmp_program("diag", "fn main() {\n  X = 5;\n  print(\"{X}\")\n}\n");
    let file = path.to_string_lossy().into_owned();
    let jfile = json_path(&path);
    let port = 18783;
    start_server(port, file.clone());
    let mut ws = ws_connect(port);
    ws_send(
        ws.get_ref(),
        &format!("{{\"id\":1,\"req\":\"compile\",\"file\":\"{jfile}\"}}"),
    );
    let msgs = recv_until(&mut ws, 4, |m| m.contains("\"event\":\"diagnostics\""));
    let all = msgs.join("\n");
    // The tier is asserted, not just the presence: UPPER_CASE is advice (correct code,
    // a naming preference), so a client gating on `"level":"warning"` must not see it.
    assert!(
        all.contains("\"event\":\"diagnostics\"") && all.contains("\"level\":\"advice\""),
        "an advice diagnostic streamed over the websocket: {all}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_ws_repl_eval_top_level() {
    // @PLN16 M5e — the REPL panel's context-aware eval over the websocket: a top-level
    // expression prints its value, a definition persists + is callable, an error returns
    // a <repl> diagnostics event.
    let path = tmp_program("repl", "fn main() { print(\"x\") }\n");
    let file = path.to_string_lossy().into_owned();
    let port = 18784;
    start_server(port, file);
    let mut ws = ws_connect(port);
    // an expression → value printed
    ws_send(
        ws.get_ref(),
        "{\"id\":1,\"req\":\"replEval\",\"input\":\"2 + 3\"}",
    );
    let r1 = ws_recv(&mut ws);
    assert!(
        r1.contains("\"context\":\"top\"") && r1.contains("\"output\":\"5\\n\""),
        "2+3 → 5: {r1}"
    );
    // define then call
    ws_send(
        ws.get_ref(),
        "{\"id\":2,\"req\":\"replEval\",\"input\":\"fn dbl(n: integer) -> integer { n * 2 }\"}",
    );
    let _ = ws_recv(&mut ws);
    ws_send(
        ws.get_ref(),
        "{\"id\":3,\"req\":\"replEval\",\"input\":\"dbl(21)\"}",
    );
    let r3 = ws_recv(&mut ws);
    assert!(r3.contains("\"output\":\"42\\n\""), "dbl(21) → 42: {r3}");
    // incomplete → more
    ws_send(
        ws.get_ref(),
        "{\"id\":4,\"req\":\"replEval\",\"input\":\"fn open() -> integer {\"}",
    );
    let r4 = ws_recv(&mut ws);
    assert!(r4.contains("\"more\":true"), "incomplete → more: {r4}");
    // error → a <repl> diagnostics event
    ws_send(
        ws.get_ref(),
        "{\"id\":5,\"req\":\"replEval\",\"input\":\"nope + 1\"}",
    );
    let msgs = recv_until(&mut ws, 3, |m| m.contains("diagnostics"));
    assert!(
        msgs.join("\n").contains("\"file\":\"<repl>\"")
            && msgs.join("\n").contains("\"level\":\"error\""),
        "error → <repl> diagnostics: {:?}",
        msgs
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_ws_writefile_saves_and_sandboxes() {
    // @PLN16 M5e slice 3 — the editor's save: writeFile overwrites the served file, but a
    // path outside it (the sandbox) is refused.
    let path = tmp_program("write", "fn main() { print(\"old\") }\n");
    let file = path.to_string_lossy().into_owned();
    let jfile = json_path(&path);
    let port = 18785;
    start_server(port, file.clone());
    let mut ws = ws_connect(port);
    // save new content to the served file → ok, and the file on disk changes
    ws_send(
        ws.get_ref(),
        &format!(
            "{{\"id\":1,\"req\":\"writeFile\",\"file_unused\":0,\"path\":\"{jfile}\",\"content\":\"fn main() {{ print(\\\"new\\\") }}\\n\"}}"
        ),
    );
    let r1 = ws_recv(&mut ws);
    assert!(r1.contains("\"id\":1,\"ok\":true"), "writeFile ok: {r1}");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "fn main() { print(\"new\") }\n",
        "file saved"
    );
    // a path outside the sandbox is refused
    ws_send(
        ws.get_ref(),
        "{\"id\":2,\"req\":\"writeFile\",\"path\":\"/etc/hosts\",\"content\":\"x\"}",
    );
    let r2 = ws_recv(&mut ws);
    assert!(
        r2.contains("\"ok\":false"),
        "out-of-sandbox write refused: {r2}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_http_shell_has_editable_source() {
    // @PLN16 M5e slice 3 — the shell's source pane is now an editable textarea (with a Save
    // button), not a read-only listing; the source is embedded in the textarea body.
    let path = tmp_program("editor", "fn main() { print(\"hello\") }\n");
    let port = 18786;
    start_server(port, path.to_string_lossy().into_owned());
    let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    (&stream)
        .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .unwrap();
    let mut body = String::new();
    let mut s = stream;
    let _ = s.read_to_string(&mut body);
    assert!(
        body.contains("id=\"src\""),
        "shell has an editable textarea"
    );
    assert!(body.contains("id=\"save\""), "shell has a Save button");
    assert!(
        body.contains("print(\"hello\")"),
        "source embedded in the editor"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_ws_writefile_then_relaunch_runs_new_code() {
    // @PLN16 M5e slice 3 — the edit→save→reload→run loop the editor's Run button drives:
    // `run` re-executes the *launched* program, so after a `writeFile` the editor must
    // re-`launch` for the new code to take effect.  This proves that server-side contract.
    let path = tmp_program("reload", "fn main() { print(\"old\") }\n");
    let file = path.to_string_lossy().into_owned();
    let jfile = json_path(&path);
    let port = 18787;
    start_server(port, file.clone());
    let mut ws = ws_connect(port);
    // save new code to the served file
    ws_send(
        ws.get_ref(),
        &format!(
            "{{\"id\":1,\"req\":\"writeFile\",\"path\":\"{jfile}\",\"content\":\"fn main() {{ print(\\\"NEW\\\") }}\\n\"}}"
        ),
    );
    assert!(ws_recv(&mut ws).contains("\"ok\":true"), "writeFile ok");
    // re-launch (reads the new disk content), then run executes it
    ws_send(
        ws.get_ref(),
        &format!("{{\"id\":2,\"req\":\"launch\",\"file\":\"{jfile}\"}}"),
    );
    assert!(
        ws_recv(&mut ws).contains("\"id\":2,\"ok\":true"),
        "relaunch ok"
    );
    ws_send(ws.get_ref(), "{\"id\":3,\"req\":\"run\"}");
    let all = recv_until(&mut ws, 8, |m| m.contains("\"event\":\"terminated\"")).join("\n");
    assert!(
        all.contains("\"text\":\"NEW\""),
        "run executes the saved code: {all}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_ws_breakpoint_stops_with_line_and_locals() {
    // @PLN16 M5e slice 4 — the debugger flow the browser UI drives: set a breakpoint, run,
    // and the `stopped` event carries the function, the current LINE (`paused_line` — the
    // editor's current-line marker), and the frame locals (the variables panel).  Continue
    // then runs to completion.
    let path = tmp_program(
        "dbg",
        "fn dbl(n: integer) -> integer {\n  r = n * 2;\n  r\n}\nfn main() {\n  x = dbl(21);\n  print(\"x={x}\")\n}\n",
    );
    let file = path.to_string_lossy().into_owned();
    let jfile = json_path(&path);
    let port = 18788;
    start_server(port, file.clone());
    let mut ws = ws_connect(port);
    ws_send(
        ws.get_ref(),
        &format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{jfile}\"}}"),
    );
    assert!(ws_recv(&mut ws).contains("\"ok\":true"), "launch ok");
    ws_send(
        ws.get_ref(),
        &format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{jfile}\",\"breakpoints\":[{{\"line\":2}}]}}"
        ),
    );
    assert!(
        ws_recv(&mut ws).contains("\"ok\":true"),
        "setBreakpoints ok"
    );
    ws_send(ws.get_ref(), "{\"id\":3,\"req\":\"run\"}");
    let stopped = recv_until(&mut ws, 6, |m| m.contains("\"event\":\"stopped\"")).join("\n");
    assert!(
        stopped.contains("\"function\":\"dbl\""),
        "stopped in dbl: {stopped}"
    );
    assert!(
        stopped.contains("\"line\":2"),
        "stopped event carries the current line: {stopped}"
    );
    assert!(
        stopped.contains("\"name\":\"n\"") && stopped.contains("\"value\":\"21\""),
        "frame exposes local n=21 for the variables panel: {stopped}"
    );
    // continue → runs to completion (the program's print, then terminated)
    ws_send(ws.get_ref(), "{\"id\":4,\"req\":\"continue\"}");
    let done = recv_until(&mut ws, 6, |m| m.contains("\"event\":\"terminated\"")).join("\n");
    assert!(
        done.contains("\"event\":\"terminated\""),
        "continue runs to completion: {done}"
    );
    let _ = std::fs::remove_file(&path);
}

// @PLN98 — the FULL debugger cycle THROUGH A SERVER SETUP: `loft debug --serve`
// (the game/serve host an agent is close to) drives launch → breakpoint → EVAL →
// resume over the WebSocket the client holds — the same channel the browser relay
// uses.  The eval exercises the P1b live-frame path: a keyed-collection (`hash`)
// local read live over the socket (was `null` before P1b), a scalar, and the
// literal `2+2` (the untouched text path).  Then continue runs to completion with
// the correct output.  This is the server-relay debug acceptance, minus a browser.
#[test]
fn serve_ws_debug_cycle_eval_and_resume_through_server() {
    let path = tmp_program(
        "dbgcycle",
        "struct HRec { name: text, v: integer }\n\
         fn main() {\n\
        \x20 h: hash<HRec[name]> = [];\n\
        \x20 h += [HRec{name: \"a\", v: 7}];\n\
        \x20 x = 5;\n\
        \x20 y = h[\"a\"].v + x;\n\
        \x20 print(\"y={y}\")\n\
         }\n",
    );
    let jfile = json_path(&path);
    let port = 18795;
    start_server(port, path.to_string_lossy().into_owned());
    let mut ws = ws_connect(port);

    ws_send(
        ws.get_ref(),
        &format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{jfile}\"}}"),
    );
    assert!(ws_recv(&mut ws).contains("\"ok\":true"), "launch ok");
    // Line 6 (`y = h["a"].v + x;`) references h (so it is live at the pause) and
    // has a `+` (a breakable offset).
    ws_send(
        ws.get_ref(),
        &format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{jfile}\",\"breakpoints\":[{{\"line\":6}}]}}"
        ),
    );
    assert!(
        ws_recv(&mut ws).contains("\"ok\":true"),
        "setBreakpoints ok"
    );
    ws_send(ws.get_ref(), "{\"id\":3,\"req\":\"run\"}");
    let stopped = recv_until(&mut ws, 6, |m| m.contains("\"event\":\"stopped\"")).join("\n");
    assert!(
        stopped.contains("\"function\":\"main\""),
        "stopped in main: {stopped}"
    );

    // EVAL over the socket — the live-frame path the server relays to the client.
    // The keyed-collection read (`h["a"].v`) is the P1b capability (was null).
    ws_send(
        ws.get_ref(),
        "{\"id\":4,\"req\":\"eval\",\"expr\":\"2 + 2\"}",
    );
    assert!(
        ws_recv(&mut ws).contains("\"id\":4,\"ok\":true,\"value\":4"),
        "2+2 == 4 over the server (text path)"
    );
    ws_send(
        ws.get_ref(),
        "{\"id\":5,\"req\":\"eval\",\"expr\":\"h[\\\"a\\\"].v\"}",
    );
    assert!(
        ws_recv(&mut ws).contains("\"id\":5,\"ok\":true,\"value\":7"),
        "h[\"a\"].v == 7 over the server (P1b keyed live-frame eval)"
    );
    ws_send(
        ws.get_ref(),
        "{\"id\":6,\"req\":\"eval\",\"expr\":\"h[\\\"a\\\"].v + x\"}",
    );
    assert!(
        ws_recv(&mut ws).contains("\"id\":6,\"ok\":true,\"value\":12"),
        "h[\"a\"].v + x == 12 over the server (keyed + scalar)"
    );

    // RESUME — the paused frame survived the evals; the run completes correctly.
    ws_send(ws.get_ref(), "{\"id\":7,\"req\":\"continue\"}");
    let done = recv_until(&mut ws, 8, |m| m.contains("\"event\":\"terminated\"")).join("\n");
    assert!(
        done.contains("\"category\":\"stdout\",\"text\":\"y=12\""),
        "continue runs to completion with the right output: {done}"
    );
    assert!(
        done.contains("\"event\":\"terminated\""),
        "terminated: {done}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_ws_run_tests_reports_pass_and_fail() {
    // @PLN16 M5e slice 5 — `runTests` runs the file's zero-parameter test functions, one
    // `testResult {name, passed, line, message?}` each, then a `testSummary {passed, failed}`.
    // A failing test carries its assertion message and the others still run (isolation).
    let path = tmp_program(
        "tests",
        "fn test_pass() {\n  assert(1 + 1 == 2, \"math\");\n}\nfn test_fail() {\n  assert(1 == 2, \"deliberately fails\");\n}\n",
    );
    let file = path.to_string_lossy().into_owned();
    let jfile = json_path(&path);
    let port = 18789;
    start_server(port, file.clone());
    let mut ws = ws_connect(port);
    ws_send(
        ws.get_ref(),
        &format!("{{\"id\":1,\"req\":\"runTests\",\"file\":\"{jfile}\"}}"),
    );
    let all = recv_until(&mut ws, 6, |m| m.contains("\"event\":\"testSummary\"")).join("\n");
    assert!(
        all.contains("\"name\":\"test_pass\",\"passed\":true"),
        "test_pass passes: {all}"
    );
    assert!(
        all.contains("\"name\":\"test_fail\",\"passed\":false")
            && all.contains("deliberately fails"),
        "test_fail fails with its message: {all}"
    );
    assert!(
        all.contains("\"event\":\"testSummary\",\"passed\":1,\"failed\":1"),
        "summary counts 1 pass / 1 fail: {all}"
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_ws_run_suite_runs_package_tests() {
    // @PLN16 M5e slice 5 — `runSuite` is the PACKAGE-aware runner (`loft test` semantics):
    // walk up from the served file to the nearest loft.toml, put the manifest's src/ on the
    // import path, and run every tests/*.loft.  Each testResult carries its `file`; the
    // summary carries the file count.  A start point with no loft.toml upward is refused.
    let root = test_tmp().join(format!("loft_suitepkg_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::write(
        root.join("loft.toml"),
        "[package]\nname = \"suitepkg\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/suitepkg.loft\"\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src/suitepkg.loft"),
        "pub fn triple(n: integer) -> integer {\n  n * 3\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("tests/a_math.loft"),
        "use suitepkg::*;\nfn test_triple() {\n  assert(triple(7) == 21, \"triple\");\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("tests/b_more.loft"),
        "use suitepkg::*;\nfn test_wrong() {\n  assert(triple(1) == 999, \"deliberately fails\");\n}\n",
    )
    .unwrap();
    let entry = root
        .join("src/suitepkg.loft")
        .to_string_lossy()
        .into_owned();
    let jentry = json_path(&root.join("src/suitepkg.loft"));
    let port = 18790;
    start_server(port, entry.clone());
    let mut ws = ws_connect(port);
    ws_send(
        ws.get_ref(),
        &format!("{{\"id\":1,\"req\":\"runSuite\",\"file\":\"{jentry}\"}}"),
    );
    let all = recv_until(&mut ws, 8, |m| m.contains("\"event\":\"testSummary\"")).join("\n");
    assert!(
        all.contains(
            "\"name\":\"test_triple\",\"passed\":true,\"line\":2,\"file\":\"a_math.loft\""
        ),
        "a_math's test passes with its file tag: {all}"
    );
    assert!(
        all.contains("\"file\":\"b_more.loft\"") && all.contains("deliberately fails"),
        "b_more's failure carries file + message: {all}"
    );
    assert!(
        all.contains("\"event\":\"testSummary\",\"passed\":1,\"failed\":1,\"files\":2"),
        "summary counts across both files: {all}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Variant of [`start_server`] with library import paths (a kernel game
/// needs `use engine_host`).
fn start_server_libs(port: u16, file: String, libs: Vec<String>) {
    std::thread::spawn(move || {
        let _ = loft::serve::run_serve("default", &libs, port, &file);
    });
    let deadline = vm_deadline(10);
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("server did not start listening on {port}");
}

#[test]
fn serve_game_debug_control_end_to_end() {
    // @PLN18 08-S7 editor support — the EXACT flow the editor shell drives:
    // launch a kernel game through the serve RPC (launch_game now defaults
    // LOFT_DEBUG_CONTROL=1 + LOFT_LIVE_FLIP=1), scrape the game port from the
    // streamed output (the page does the same), dial the game's D!: control
    // channel DIRECTLY, set a fn-entry breakpoint, observe the hit with
    // bindings while a game client's reply is held, resume, and stop the game
    // over the channel (D!:quit — the editor's stop for a swapped game).
    let loft_bin = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/loft");
    if !loft_bin.exists() {
        eprintln!("skipping: {} not built", loft_bin.display());
        return;
    }
    // SAFETY: same contract as the slice-6 test — a constant, valid path.
    unsafe { std::env::set_var("LOFT_BIN", &loft_bin) };
    let game_port = 18114u16;
    let path = tmp_program(
        "gamedbg",
        &format!(
            "use engine_host;\n\
             struct W {{ events: integer not null, ticks: integer not null }}\n\
             fn bump_events(w: W) -> integer {{\n  w.events = w.events + 1;\n  w.events\n}}\n\
             fn main() {{\n  w = W {{ events: 0, ticks: 0 }};\n  resumed = engine_host::swap_world(w);\n  \
             engine_host::run({game_port}, 10000,\n    fn(ev: engine_host::Event) {{\n      \
             if ev.kind != 1 {{ return; }}\n      n = bump_events(w);\n      \
             engine_host::send(ev.cid, \"got:{{ev.payload}}#{{n}}\");\n    }},\n    \
             fn() {{ w.ticks = w.ticks + 1; }});\n}}\n"
        ),
    );
    let file = path.to_string_lossy().into_owned();
    let jfile = json_path(&path);
    let port = 18793;
    let lib = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("lib")
        .to_string_lossy()
        .into_owned();
    start_server_libs(port, file, vec![lib]);
    let mut ws = ws_connect(port);
    ws_send(
        ws.get_ref(),
        &format!("{{\"id\":1,\"req\":\"launchGame\",\"file\":\"{jfile}\"}}"),
    );
    assert!(ws_recv(&mut ws).contains("\"ok\":true"), "launch ok");
    // Scrape the port announcement from the streamed output (the page's move).
    let mut announced = false;
    for i in 0..100 {
        std::thread::sleep(Duration::from_millis(100));
        ws_send(
            ws.get_ref(),
            &format!("{{\"id\":{},\"req\":\"gameStatus\"}}", 10 + i),
        );
        let r = ws_recv(&mut ws);
        if r.contains(&format!("listening on ws://0.0.0.0:{game_port}/")) {
            announced = true;
            break;
        }
        assert!(
            r.contains("\"running\":true"),
            "the game must stay up until it announces: {r}"
        );
    }
    assert!(announced, "the kernel game never announced its port");

    // The control channel + a game client — the editor's two sockets.
    let mut ctl = ws_connect(game_port);
    let mut game = ws_connect(game_port);
    ws_send(ctl.get_ref(), "D!:bp bump_events");
    assert_eq!(ws_recv(&mut ctl), "D:ok bp bump_events");
    ws_send(game.get_ref(), "a");
    let hit = ws_recv(&mut ctl);
    assert!(
        hit.starts_with("D:hit bump_events") && hit.contains("w="),
        "hit with bindings: {hit}"
    );
    ws_send(ctl.get_ref(), "D!:resume");
    assert_eq!(ws_recv(&mut ctl), "D:resumed");
    assert_eq!(ws_recv(&mut game), "got:a#1");

    // Stop over the channel (the editor's post-swap stop path).
    ws_send(ctl.get_ref(), "D!:quit");
    assert_eq!(ws_recv(&mut ctl), "D:quitting");
    // A DEADLINE, not a trip count — the sibling of the wait below and the same defect
    // (loft#1668): fifty polls at 100 ms is five seconds of wall clock, and wall clock is what
    // a loaded box takes away.  Found by a sweep after the other loop was cured, which is the
    // reason to sweep: one file held two of them and only one had gone red yet.
    let mut stopped = false;
    let started = Instant::now();
    let deadline = vm_deadline(20);
    let mut polls = 0;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        ws_send(
            ws.get_ref(),
            &format!("{{\"id\":{},\"req\":\"gameStatus\"}}", 200 + polls),
        );
        polls += 1;
        if ws_recv(&mut ws).contains("\"running\":false") {
            stopped = true;
            break;
        }
    }
    assert!(
        stopped,
        "the game must exit on D!:quit — {polls} polls in {:?}",
        started.elapsed()
    );
    let _ = std::fs::remove_file(&path);
}

#[test]
fn serve_ws_game_launch_streams_and_stops() {
    // @PLN16 M5e slice 6 — the game-process layer: `launchGame` spawns the file as a real
    // `loft` child process, `gameStatus` polls drain its output and report exit, `stopGame`
    // kills a running one.  The child binary comes from LOFT_BIN (here: the built loft —
    // in production the serve process IS the loft binary, so current_exe is used).
    let loft_bin = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/release/loft");
    if !loft_bin.exists() {
        eprintln!("skipping: {} not built", loft_bin.display());
        return;
    }
    // SAFETY: process-global env write; no other serve test reads LOFT_BIN, and the value
    // is constant for the whole test process, so a concurrent reader still sees a valid path.
    unsafe { std::env::set_var("LOFT_BIN", &loft_bin) };
    let path = tmp_program(
        "game",
        "fn main() {\n  for i in 0..3 {\n    print(\"frame {i}\");\n  }\n}\n",
    );
    let file = path.to_string_lossy().into_owned();
    let jfile = json_path(&path);
    let port = 18791;
    start_server(port, file.clone());
    let mut ws = ws_connect(port);
    ws_send(
        ws.get_ref(),
        &format!("{{\"id\":1,\"req\":\"launchGame\",\"file\":\"{jfile}\"}}"),
    );
    assert!(ws_recv(&mut ws).contains("\"ok\":true"), "launch ok");
    // Poll until the game exits, on a DEADLINE rather than a trip count (loft#1668).  Forty
    // polls at 100 ms is four seconds of wall clock, which is a budget that SHRINKS exactly
    // when the box is loaded — so the test went red under a gate while the server answered
    // every one of its forty polls correctly and the game simply had not finished.  A trip
    // count inverts the property a CI wait needs; `vm_deadline` is the idiom this file already
    // uses for that, and it stretches ×3 on a shared machine (`common::deadline_scale`).
    let mut all = String::new();
    let mut exited = false;
    let started = Instant::now();
    let deadline = vm_deadline(20);
    let mut polls = 0;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        ws_send(
            ws.get_ref(),
            &format!("{{\"id\":{},\"req\":\"gameStatus\"}}", 10 + polls),
        );
        polls += 1;
        let r = ws_recv(&mut ws);
        all.push_str(&r);
        if r.contains("\"running\":false") {
            exited = true;
            break;
        }
    }
    // Say what was actually waited, so a future red is triage rather than a guess.
    assert!(
        exited,
        "game ran to completion after {polls} polls in {:?}: {all}",
        started.elapsed()
    );
    assert!(
        all.contains("frame 0") && all.contains("frame 2"),
        "game output streamed through gameStatus: {all}"
    );
    assert!(all.contains("\"exit\":0"), "exit code reported: {all}");

    // An infinite-loop game is killed by stopGame.
    let loop_path = tmp_program(
        "gameloop",
        "fn main() {\n  i = 0;\n  while true {\n    i = i + 1;\n  }\n}\n",
    );
    let loop_file = json_path(&loop_path);
    ws_send(
        ws.get_ref(),
        &format!("{{\"id\":2,\"req\":\"launchGame\",\"file\":\"{loop_file}\"}}"),
    );
    assert!(ws_recv(&mut ws).contains("\"ok\":true"), "loop launch ok");
    ws_send(ws.get_ref(), "{\"id\":3,\"req\":\"stopGame\"}");
    assert!(ws_recv(&mut ws).contains("\"ok\":true"), "stop ok");
    ws_send(ws.get_ref(), "{\"id\":4,\"req\":\"gameStatus\"}");
    assert!(
        ws_recv(&mut ws).contains("\"running\":false"),
        "stopped game reports not running"
    );
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(&loop_path);
}
