// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! The ssh library's natives, loft's `net` component (`[native] component = "net"`;
//! `src/c/loft_ssh.h` over `src/c/tls/loft_ssh.c` in `src/c/components/net.c`), keep the
//! promises they make a program, the same on every backend: the interpreter calls their bridges,
//! the Rust writer and the C backend their typed symbols.  The program
//! (`tests/fixtures/ssh_natives/interact.loft`) is the CLIENT, reaching the natives through
//! `tests/fixtures/ssh_natives` (their declarations word for word); the server is
//! `tests/fixtures/ssh_natives/test_sshd.c`, a small SSH server built here against the same TLS
//! archive, and every line the program must print is fixed in advance:
//!
//! - a refused password answers false, and a shell without a login is refused;
//! - the pty's size reaches the server, and so does a resize;
//! - bytes both ways: the 256 byte values unchanged, 3 000 000 bytes out past the client's
//!   2 MiB window (its reads give the window back), 200 000 bytes in through the server's 8 KiB
//!   window in 1 KiB packets (the send splits and waits);
//! - `recv` answers "" when nothing arrived, never waiting;
//! - the shell's exit closes the session, and every call on a closed or unknown handle does
//!   nothing; a port nobody serves answers -1.
//!
//! This checks the INTERACTIONS.  The protocol itself is checked against OpenSSH: the server
//! here is built on the client's own packet layer, so the two agree by construction.  The
//! OpenSSH check runs on a Linux runner, where an account with a password can be made:
//! `gh workflow run linux-probe.yml -f run=…` with `tests/fixtures/ssh_natives/session.loft`
//! (C_BACKEND.md § The SSH client).

use loft::file_access as fa;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

const WANT: &[&str] = &[
    "BADPW true false false",
    "CONNECT true true",
    "LOGIN true",
    "SHELL true true",
    "START READY vt100 80x24",
    "RESIZE SIZE 132x43",
    "ECHO hello there",
    "BYTES 257 32640 true -1",
    "BIG 3000000 true",
    "COUNT 200000 16860460",
    "IDLE [] true",
    "EXIT false",
    "CLOSED [] false false false",
    "UNKNOWN [] false false",
    "REFUSED -1",
];

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Build the component where a `loft` beside `loft-components` finds it (`<binary>/../components`),
/// as `make components` does.
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

/// The test server, built once per test process into `dir`; `None` (with the reason printed)
/// where it cannot be built here.
fn build_server(dir: &Path) -> Option<PathBuf> {
    let cc = loft::platform::c_unit_compiler();
    if loft::platform::c_compiler_is_msvc(&cc) {
        eprintln!(
            "SKIP: the test SSH server is built with a gcc-style compiler, and `{cc}` links units here"
        );
        return None;
    }
    let archive = match loft::c::component::tls_archive() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("SKIP: the TLS archive did not build: {e}");
            return None;
        }
    };
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/c/tls");
    let exe = dir.join(if cfg!(windows) {
        "test_sshd.exe"
    } else {
        "test_sshd"
    });
    let out = Command::new(&cc)
        .arg("-std=c99")
        .arg("-O1")
        .arg("-I")
        .arg(&src)
        .arg("-I")
        .arg(src.join("bearssl"))
        .arg(fixtures().join("ssh_natives/test_sshd.c"))
        .arg(&archive)
        .args(loft::platform::c_system_link_libs())
        .arg("-o")
        .arg(&exe)
        .output()
        .expect("the C compiler runs");
    assert!(
        out.status.success(),
        "test_sshd.c did not build:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(exe)
}

fn run(backend: &str) {
    // The component needs the C runtime's sockets (POSIX or Winsock, @PLN203 C8).
    if !loft::platform::c_runtime_has_sockets() {
        return;
    }
    build_component();
    let dir = std::env::temp_dir().join(format!(
        "loft_ssh_natives_{}_{}",
        &backend[2..],
        std::process::id()
    ));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("a scratch directory");
    let Some(server) = build_server(&dir) else {
        return;
    };
    // Two connections: the refused password's and the session's; then it exits, and the port
    // it served is the one nobody does.
    let mut srv = Command::new(&server)
        .arg("2")
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("the test server starts");
    let mut first = String::new();
    BufReader::new(srv.stdout.take().expect("the server's stdout"))
        .read_line(&mut first)
        .expect("the server's port line");
    let port = first
        .trim()
        .strip_prefix("PORT ")
        .unwrap_or_else(|| panic!("the server printed {first:?}"))
        .to_string();
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .arg(backend)
        .arg("--no-warnings")
        .arg("--lib")
        .arg(fixtures())
        .arg(fixtures().join("ssh_natives/interact.loft"))
        .current_dir(&dir)
        .env_remove("LOFT_COMPONENTS")
        .env("LOFT_TIMEOUT", "240")
        .env("LOFT_SSH_TRACE", "1")
        .env("SSH_PORT", &port)
        .output()
        .expect("loft runs");
    let _ = srv.kill();
    let _ = srv.wait();
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines, WANT, "{backend}\nstderr:\n{stderr}");
    let _ = fa::remove_dir_all(&dir);
}

#[test]
fn the_ssh_natives_keep_their_promises_on_the_interpreter() {
    run("--interpret");
}

#[test]
fn the_ssh_natives_keep_their_promises_on_the_rust_writer() {
    run("--native-rust");
}

#[test]
fn the_ssh_natives_keep_their_promises_on_the_c() {
    run("--native-c");
}
