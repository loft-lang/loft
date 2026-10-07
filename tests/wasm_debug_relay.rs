// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN98 P3.4 — the FULL browser debug relay, end-to-end and headless: an AGENT
//! debugs a browser wasm CLIENT THROUGH a running game server, over the
//! WebSockets both hold. The client is a real `loft --html --debug` wasm run in
//! Node with a JS driver bridging the server socket to its `host_input` /
//! host-output; the server is a real `engine_host` kernel with the per-name debug
//! relay; the agent addresses the client by name. Verifies the whole chain:
//! `D!:@alice:bp` → server forward → client pause → client reply → server relay →
//! agent, for bp / run / eval / resume.
//!
//! Skips cleanly when node / the wasm32 target / the release binary are missing.

// @PLN184 C2 exemption candidate: `killpg` of the `--native` server grandchild (and `sh -c command -v`) has no Windows equivalent; Windows substitute: none
#![cfg(unix)]

use loft::file_access as fa;
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

#[path = "common/mod.rs"]
mod common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn have(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn wasm32_installed() -> bool {
    Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .is_some_and(|s| s.lines().any(|l| l == "wasm32-unknown-unknown"))
}

/// Kill the whole PROCESS GROUP on drop. A default (`--native`) server run spawns
/// the compiled binary as a GRANDCHILD of the loft driver; killing only the child
/// orphans the real server, which keeps LISTENing. With SO_REUSEPORT the orphans
/// co-bind the port and the kernel spreads connections across them, so a later run's
/// client and agent land on DIFFERENT servers and the relay silently fails ("relayed
/// alice" to a dead socket, or "no debug client alice"). Same fix as
/// `engine_host_kernel.rs`.
struct Guard(Option<Child>);
impl Drop for Guard {
    fn drop(&mut self) {
        if let Some(mut c) = self.0.take() {
            unsafe {
                libc::killpg(c.id() as i32, libc::SIGKILL);
            }
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

// @speed 6.0
#[test]
fn agent_debugs_a_browser_wasm_client_through_the_server_relay() {
    if !have("node") || !wasm32_installed() {
        eprintln!("SKIP: node or wasm32-unknown-unknown target missing");
        return;
    }
    let root = repo_root();
    let loft = root.join("target/release/loft");
    if !fa::exists(&loft) {
        eprintln!("SKIP: target/release/loft not built");
        return;
    }
    let e2e = root.join("tools/wasm_debug_e2e.mjs");
    if !fa::exists(&e2e) {
        eprintln!("SKIP: tools/wasm_debug_e2e.mjs missing");
        return;
    }
    let tmp = std::env::temp_dir();
    let port: u16 = common::bind_port(19322);

    // 1. Build the browser CLIENT: `--html --debug` with a breakpointable fn.
    let client_src = tmp.join("p98_relay_client.loft");
    fa::write(
        &client_src,
        "fn compute(n: integer) -> integer {\n  m = n + 2;\n  m\n}\nfn main() { compute(40); }\n",
    )
    .unwrap();
    let client_html = tmp.join("p98_relay_client.html");
    let built = Command::new(&loft)
        .args(["--html", client_html.to_str().unwrap(), "--path"])
        .arg(format!("{}/", root.display()))
        .args(["--debug=alice"])
        .arg(client_src.to_str().unwrap())
        .status()
        .expect("run loft --html --debug");
    assert!(built.success(), "client --html --debug build failed");

    // 2. Spawn the SERVER: an engine_host kernel with the debug relay on.
    let server_src = tmp.join("p98_relay_server.loft");
    fa::write(
        &server_src,
        format!(
            "use engine_host;\nstruct W {{ ticks: integer not null }}\nfn main() {{\n  \
             w = W {{ ticks: 0 }};\n  engine_host::run({port}, 100000,\n    \
             fn(ev: engine_host::Event) {{ if ev.kind != 1 {{ return; }} }},\n    \
             fn() {{ w.ticks = w.ticks + 1; }});\n}}\n"
        ),
    )
    .unwrap();
    let server = Command::new(&loft)
        .env("LOFT_OFFLINE", "1")
        .env("LOFT_DEBUG_CONTROL", "1")
        .arg("--no-warnings")
        .arg("--lib")
        .arg(root.join("lib"))
        .arg(&server_src)
        .current_dir(&root)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0) // own group, so the Guard's killpg reaps the --native grandchild
        .spawn()
        .expect("spawn server");
    let _guard = Guard(Some(server));
    // Wait for the server to accept.
    let deadline = Instant::now() + Duration::from_secs(15);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline, "server never started on {port}");
        std::thread::sleep(Duration::from_millis(100));
    }

    // 3. Run the AGENT+CLIENT node driver against the relay.
    let out = Command::new("node")
        .arg(&e2e)
        .arg(&client_html)
        .arg(port.to_string())
        .output()
        .expect("run node e2e driver");
    let all = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    // The agent must have received the CLIENT's replies, relayed by the server.
    for needle in [
        "D:ok bp compute", // breakpoint set on the browser client
        "D:hit compute",   // the client PAUSED at it
        "n=40",            // with the live frame local
        "D:eval n=40",     // read a live frame local over the paused client
        "D:eval n + 2=42", // FULL-EXPRESSION eval over the paused client
        "D:terminated",    // resume ran to completion
    ] {
        assert!(
            all.contains(needle),
            "agent did not receive {needle:?} over the relay.\n{all}"
        );
    }
    let _ = fa::remove_file(&client_src);
    let _ = fa::remove_file(&server_src);
}
