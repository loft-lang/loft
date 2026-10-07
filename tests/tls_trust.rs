// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![cfg(feature = "registry")]

//! loft trusts the certificates the machine trusts (`src/tls.rs`): the bundled Mozilla roots
//! PLUS the platform store, which `SSL_CERT_FILE` / `SSL_CERT_DIR` override exactly as they do
//! for curl and git.  Behind a TLS-inspecting proxy the proxy's CA is installed there, and a
//! client trusting the bundled roots alone refuses every connection with `UnknownIssuer`.
//!
//! Hermetic: a local HTTPS server signed by a test CA (`tests/fixtures/tls/`, the CA's own key
//! discarded), and `loft search` pointed at it through `LOFT_REGISTRY_URL`.  The SERVER is the
//! witness: it records the request line of every connection whose handshake completed, so the
//! verdict does not depend on what loft makes of the body.  Each case is its own `loft`
//! process, because the trust store is read once per process.
//!
//! `LOFT_TLS_TEST_BINARY` runs the cases against another `loft` — a build from before the
//! shared TLS config refuses the CA even with `SSL_CERT_FILE` set, which is this test's control.

use loft::file_access as fa;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/tls/");

fn server_config() -> Arc<rustls::ServerConfig> {
    use rustls::pki_types::pem::PemObject;
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let certs = CertificateDer::pem_file_iter(format!("{FIXTURES}server.pem"))
        .expect("server.pem is readable")
        .map(|c| c.expect("server.pem parses"))
        .collect();
    let key = PrivateKeyDer::from_pem_file(format!("{FIXTURES}server.key")).expect("server.key");
    Arc::new(
        rustls::ServerConfig::builder_with_provider(
            rustls::crypto::ring::default_provider().into(),
        )
        .with_safe_default_protocol_versions()
        .expect("ring supports the default versions")
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .expect("the fixture certificate and key match"),
    )
}

/// Serve HTTPS on a free local port; every request line that arrives over a completed
/// handshake is recorded.  A client that rejects the certificate never sends one.
fn start_server() -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind a local port");
    let port = listener.local_addr().expect("local address").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let cfg = server_config();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(tcp) = stream else { continue };
            let (cfg, log) = (Arc::clone(&cfg), Arc::clone(&log));
            std::thread::spawn(move || {
                let Ok(conn) = rustls::ServerConnection::new(cfg) else {
                    return;
                };
                let mut tls = rustls::StreamOwned::new(conn, tcp);
                let mut buf = [0u8; 4096];
                let Ok(n) = tls.read(&mut buf) else {
                    return; // the handshake failed: no request reached the server
                };
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                if let Some(line) = request.lines().next() {
                    log.lock().expect("log").push(line.to_string());
                }
                let body = "{}";
                let _ = write!(
                    tls,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });
    (port, seen)
}

/// `loft search` against the local server, with `SSL_CERT_FILE` set to `ca` or unset; answers
/// the request lines the server saw and loft's stderr.
fn search(ca: Option<&str>) -> (Vec<String>, String) {
    let (port, seen) = start_server();
    let home = std::env::temp_dir().join(format!("loft_tls_trust_{}_{port}", std::process::id()));
    fa::create_dir_all(&home).expect("a scratch home");
    let bin = std::env::var("LOFT_TLS_TEST_BINARY")
        .unwrap_or_else(|_| env!("CARGO_BIN_EXE_loft").to_string());
    let mut cmd = loft::platform::process::harness_command(bin);
    cmd.args(["search", "anything"])
        // `LOFT_HOME` too: on Windows `$HOME` is not where loft looks (registry_index::cache_dir,
        // @P332), so without it `search` read the real profile's cached index and never
        // connected — the server saw nothing and loft said nothing.
        .env("HOME", &home)
        .env("LOFT_HOME", &home)
        .env("XDG_CACHE_HOME", home.join("cache"))
        .env(
            "LOFT_REGISTRY_URL",
            format!("https://127.0.0.1:{port}/index.json"),
        )
        .env_remove("SSL_CERT_FILE")
        .env_remove("SSL_CERT_DIR");
    if let Some(ca) = ca {
        cmd.env("SSL_CERT_FILE", ca);
    }
    let out = cmd.output().expect("loft runs");
    let _ = fa::remove_dir_all(&home);
    let seen = seen.lock().expect("log").clone();
    (seen, String::from_utf8_lossy(&out.stderr).to_string())
}

#[test]
fn a_ca_the_machine_does_not_trust_is_refused() {
    let (seen, stderr) = search(None);
    assert!(
        seen.is_empty(),
        "no request may cross an untrusted handshake, but the server saw {seen:?}"
    );
    assert!(
        stderr.contains("UnknownIssuer") || stderr.contains("certificate"),
        "the refusal names the certificate: {stderr}"
    );
}

#[test]
fn a_ca_in_ssl_cert_file_is_trusted() {
    let ca = format!("{FIXTURES}ca.pem");
    let (seen, stderr) = search(Some(&ca));
    assert!(
        seen.iter().any(|l| l == "GET /index.json HTTP/1.1"),
        "with the CA in SSL_CERT_FILE the index request completes the handshake; the server saw \
         {seen:?}, loft said: {stderr}"
    );
}
