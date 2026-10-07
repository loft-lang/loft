// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `host_input()` — the program-input primitive (mirror of `print`).  Reads all
//! input as one text: stdin on `--interpret` / `--native` (and WASI), the JS
//! host on `--html`.  This guards the interpret == native byte-parity and the
//! empty-input case.  The `--html` leg (JS `globalThis.loftInput` → the same
//! bytes) is proven with the Node harness in `doc/claude/WEB_APPS.md`; it is not
//! run here because it needs the wasm toolchain + Node.

use loft::file_access as fa;
use std::io::Write;
use std::process::Stdio;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

static SEQ: AtomicU32 = AtomicU32::new(0);

const FIXTURE: &str = r#"fn main() {
  input = host_input();
  println("echo=[{input}] len={input.len()}");
}
"#;

/// Run the fixture on `backend`, feeding `stdin`, return captured stdout.  Each
/// call gets its own temp dir so `--native`'s `.loft/` build artifacts and
/// parallel test threads never collide.
///
/// On anything other than a clean exit with output, the returned string is a
/// DIAGNOSTIC rather than the empty string.  This used to discard stderr and return
/// `""`, so when the `--native` leg failed under full-suite load the assertion read
/// `left: "echo=[café] len=4\n", right: ""` — a symptom with its cause thrown away,
/// leaving nothing to act on but "it flaked".  A failing build or a killed child now
/// says so, in the panic message, at the moment it happens.
fn run(backend: &str, stdin: &str) -> String {
    let id = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("loft_host_input_{}_{id}", std::process::id()));
    fa::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("hi.loft");
    fa::write(&path, FIXTURE).expect("write fixture");
    let mut child = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .args([backend, path.to_str().unwrap()])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn loft");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(stdin.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait");
    let _ = fa::remove_dir_all(&dir);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() && !stdout.is_empty() {
        return stdout;
    }
    // Carry the cause into the assertion instead of returning a bare "".
    let stderr = String::from_utf8_lossy(&out.stderr);
    format!(
        "<{backend} produced no usable output: status={:?}, stdout={stdout:?}, stderr={}>",
        out.status,
        if stderr.trim().is_empty() {
            "(empty)".to_string()
        } else {
            format!("{:?}", stderr.trim())
        }
    )
}

/// The whole point: the same source reading `host_input()` produces byte-for-byte
/// identical output on the interpreter and the native backend.
#[test]
fn host_input_interpret_equals_native() {
    let interp = run("--interpret", "hello loft");
    let native = run("--native", "hello loft");
    assert_eq!(
        interp, "echo=[hello loft] len=10\n",
        "interpret: {interp:?}"
    );
    assert_eq!(interp, native, "interpret vs native diverged");
}

/// No input → an empty (not null) text, length 0 — on both backends.
#[test]
fn host_input_empty() {
    assert_eq!(run("--interpret", ""), "echo=[] len=0\n");
    assert_eq!(run("--native", ""), "echo=[] len=0\n");
}

/// `host_output(msg)` — the outbound mirror.  On native/WASI it writes one
/// line per message to STDERR (the machine channel, scriptable by the
/// invoking process), never to stdout — and byte-identically on both
/// backends.  (The `--html` leg — globalThis.loftOutput — rides the same
/// Node/browser harness as the input leg.)
#[test]
fn host_output_goes_to_stderr_on_both_backends() {
    let fixture = r#"fn main() {
  host_output("req 1 GET http://x/");
  host_output("req 2 GET http://y/");
  println("done");
}
"#;
    for backend in ["--interpret", "--native"] {
        let id = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir =
            std::env::temp_dir().join(format!("loft_host_output_{}_{id}", std::process::id()));
        fa::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("ho.loft");
        fa::write(&path, fixture).expect("write fixture");
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .args([backend, path.to_str().unwrap()])
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .expect("run loft");
        let _ = fa::remove_dir_all(&dir);
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            stdout,
            "done
",
            "{backend} stdout: {stdout:?}"
        );
        assert!(
            stderr.contains(
                "req 1 GET http://x/
"
            ) && stderr.contains(
                "req 2 GET http://y/
"
            ),
            "{backend} stderr: {stderr:?}"
        );
    }
}

/// Run `source` with stdin held OPEN by a pipe this test keeps its end of, so
/// the child cannot reach EOF, and return its stdout.
///
/// This is the shape the plain `run` harness cannot express: it writes stdin and
/// drops it, which closes the pipe and lets even a blocking drain finish.  An
/// absent or slow host does neither, and that is where an unbounded read waits
/// forever (loft#891).
///
/// `answers` plays the host: each `(request, reply)` pair writes `reply` onto
/// the pipe when the program `host_output`s `request`.  Driving it off the
/// program's own requests rather than off a sleep is what makes the ORDER of
/// reads and writes a fact instead of a race — a fixed delay feeds `--native`
/// while rustc is still compiling, so every chunk is already queued before
/// `main` runs and a test about splitting reads never splits one.
///
/// A child still running after `grace` is killed and reported as a HANG rather
/// than left to wedge the suite — a timeout is a result here, not an accident.
fn run_with_open_stdin(
    backend: &str,
    source: &str,
    answers: &[(&str, &[u8])],
    grace: Duration,
) -> String {
    let id = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("loft_host_wait_{}_{id}", std::process::id()));
    fa::create_dir_all(&dir).expect("mkdir");
    let path = dir.join("hw.loft");
    fa::write(&path, source).expect("write fixture");
    let mut child = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .args([backend, path.to_str().unwrap()])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn loft");
    // Held for the whole run: dropping it is what would signal EOF.
    let pipe = child.stdin.take().expect("stdin");
    let requests = child.stderr.take().expect("stderr");
    let script: Vec<(String, Vec<u8>)> = answers
        .iter()
        .map(|(q, a)| ((*q).to_string(), (*a).to_vec()))
        .collect();
    let host = std::thread::spawn(move || {
        let mut pipe = pipe;
        for line in std::io::BufRead::lines(std::io::BufReader::new(requests)) {
            let Ok(line) = line else { break };
            for (request, reply) in &script {
                if line.trim() == request {
                    let _ = pipe.write_all(reply);
                    let _ = pipe.flush();
                }
            }
        }
        // Returned rather than dropped here: closing the pipe would signal the
        // EOF whose absence is the whole point of this harness.
        pipe
    });
    let deadline = std::time::Instant::now() + grace;
    let status = loop {
        match child.try_wait().expect("try_wait") {
            Some(s) => break Some(s),
            None if std::time::Instant::now() >= deadline => break None,
            None => std::thread::sleep(Duration::from_millis(20)),
        }
    };
    if status.is_none() {
        let _ = child.kill();
    }
    let out = child.wait_with_output().expect("wait");
    drop(host.join());
    let _ = fa::remove_dir_all(&dir);
    match status {
        Some(_) => String::from_utf8_lossy(&out.stdout).into_owned(),
        None => format!(
            "<{backend} HUNG with stdin open; partial stdout={:?}>",
            String::from_utf8_lossy(&out.stdout)
        ),
    }
}

/// A bounded `host_input(ms)` ANSWERS when nothing is listening, instead of
/// waiting for a stream that will never end.
///
/// This is loft#891's case exactly: a program asks its environment an optional
/// question, and the answer it most needs is "nobody is there".  The default
/// form cannot give it — waiting for EOF on a pipe no one closes is a hang — so
/// the guard is that the process *terminates* with an empty read, on both
/// backends.
#[test]
fn bounded_host_input_answers_with_no_host() {
    let source = r#"fn main() {
  host_output("MODE?");
  m = host_input(200);
  println("mode=[{m}] len={m.len()}");
}
"#;
    for backend in ["--interpret", "--native"] {
        let out = run_with_open_stdin(backend, source, &[], Duration::from_secs(20));
        assert_eq!(out, "mode=[] len=0\n", "{backend}: {out:?}");
    }
}

/// The other half: a bounded read still SEES input from a host that stays
/// connected.
///
/// An unbounded read cannot answer here either — the bytes have arrived, but
/// waiting for EOF means waiting for the host to hang up, so a live
/// request/response exchange never gets its reply.  Returning "" would be the
/// worse failure of the two (a false "nobody is listening"), which is why this
/// runs beside the test above rather than on its own.
#[test]
fn bounded_host_input_reads_a_host_that_stays_connected() {
    let source = r#"fn main() {
  host_output("MODE?");
  m = host_input(3000);
  println("mode=[{m}] len={m.len()}");
}
"#;
    for backend in ["--interpret", "--native"] {
        let out = run_with_open_stdin(
            backend,
            source,
            &[("MODE?", b"SERVER")],
            Duration::from_secs(20),
        );
        assert_eq!(out, "mode=[SERVER] len=6\n", "{backend}: {out:?}");
    }
}

/// A multi-byte character split across two host writes arrives WHOLE.
///
/// A timed read can land mid-character, and handing those bytes over as they
/// come would turn one `é` into replacement characters on both sides of the
/// split.  The first read takes the complete prefix only; the held-back byte
/// must not then end the next wait instantly either, or that read reports empty
/// while the character is one byte away.
#[test]
fn bounded_host_input_never_splits_a_character() {
    // Each read is announced first, so the second chunk cannot be written until
    // the first read has already returned — the tear is guaranteed, not hoped for.
    let source = r#"fn main() {
  host_output("ONE?");
  a = host_input(3000);
  host_output("TWO?");
  b = host_input(3000);
  println("a=[{a}] b=[{b}] sizes={a.size()}/{b.size()}");
}
"#;
    for backend in ["--interpret", "--native"] {
        // "caf" + the two bytes of 'é', torn apart between the writes.
        let out = run_with_open_stdin(
            backend,
            source,
            &[("ONE?", b"caf\xc3"), ("TWO?", b"\xa9")],
            Duration::from_secs(20),
        );
        assert_eq!(out, "a=[caf] b=[é] sizes=3/2\n", "{backend}: {out:?}");
    }
}

/// Bytes pass through verbatim, including a UTF-8 multi-byte char (len is bytes).
#[test]
fn host_input_utf8_passthrough() {
    // "café" = 4 characters over 5 UTF-8 bytes (é is 2).  The engine moves opaque
    // bytes; loft decodes them and `len` counts characters (@PLN110).
    let interp = run("--interpret", "café");
    assert_eq!(interp, "echo=[café] len=4\n", "interpret: {interp:?}");
    assert_eq!(
        interp,
        run("--native", "café"),
        "interpret vs native diverged"
    );
}
