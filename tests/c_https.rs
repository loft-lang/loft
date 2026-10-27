// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![cfg(feature = "registry")]

//! HTTPS from a compiled program answers as it does interpreted (@PLN203 C5): the C runtime's TLS
//! client (`src/c/tls/loft_tls.c`, BearSSL) under both of its HTTP clients — `store_load_url*`'s
//! whole-image load and `engine_host::http_fetch` — trusts what `src/tls.rs` trusts and refuses
//! what it refuses.
//!
//! Hermetic, as `tls_trust.rs` is: a local HTTPS server signed by the test CA of
//! `tests/fixtures/tls/` (its certificate names `127.0.0.1` and `localhost`), the CA trusted
//! through `SSL_CERT_FILE`.  The SERVER is the witness for a refusal: it records the request line
//! of every connection whose handshake completed, so "refused" means no request crossed, not
//! that the program printed a failure.  Each case is its own process, because the trust store is
//! read once per process.

use loft::file_access as fa;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
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

/// Serve the files of `dir` over HTTPS on `bind` (`/probe` answers `hello-tls`); every request
/// line that arrives over a completed handshake is recorded.
fn start_server(bind: &str, dir: PathBuf) -> (u16, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind(format!("{bind}:0")).expect("bind a local port");
    let port = listener.local_addr().expect("local address").port();
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = Arc::clone(&seen);
    let cfg = server_config();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(tcp) = stream else { continue };
            let (cfg, log, dir) = (Arc::clone(&cfg), Arc::clone(&log), dir.clone());
            std::thread::spawn(move || {
                let Ok(conn) = rustls::ServerConnection::new(cfg) else {
                    return;
                };
                let mut tls = rustls::StreamOwned::new(conn, tcp);
                let mut head = Vec::new();
                let mut buf = [0u8; 4096];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match tls.read(&mut buf) {
                        Ok(0) | Err(_) => return, // the handshake failed, or no request came
                        Ok(n) => head.extend_from_slice(&buf[..n]),
                    }
                }
                let request = String::from_utf8_lossy(&head).to_string();
                let line = request.lines().next().unwrap_or_default().to_string();
                log.lock().expect("log").push(line.clone());
                let path = line.split(' ').nth(1).unwrap_or("/");
                let body: Option<Vec<u8>> = if path == "/probe" {
                    Some(b"hello-tls".to_vec())
                } else {
                    fa::read(dir.join(path.trim_start_matches('/'))).ok()
                };
                let _ = match body {
                    Some(b) => tls
                        .write_all(
                            format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                b.len()
                            )
                            .as_bytes(),
                        )
                        .and_then(|()| tls.write_all(&b)),
                    None => tls.write_all(
                        b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                    ),
                };
                tls.conn.send_close_notify();
                let _ = tls.flush();
            });
        }
    });
    (port, seen)
}

const PROGRAM: &str = r#"use engine_host;

struct Rec {
  id: integer not null,
  val: integer not null,
}

struct W { issued: integer not null }

fn main() {
  base = env_variable("LOFT_HTTPS_BASE") ?? "";
  h: hash<Rec[id]> = [];
  if base == "" {
    h += Rec { id: 13, val: 1300 };
    h += Rec { id: 7, val: 700 };
    ok = store_persist_copy(h, env_variable("LOFT_HTTPS_STORE") ?? "");
    println("WROTE {ok}");
    return
  }
  ok = store_load_url_trusted(h, "{base}/world.store");
  found = h[13];
  shown = if found == null { "null" } else { "{found.val}" };
  println("LOAD ok={ok} h13={shown}");
  w = W { issued: 0 };
  engine_host::run((env_variable("LOFT_HTTPS_WS") ?? "") as integer ?? 0, 5,
    fn(ev: engine_host::Event) {
      if ev.kind == 3 {
        println("FETCH status={ev.status} body={ev.payload}");
        engine_host::stop();
      }
    },
    fn() {
      if w.issued == 0 {
        w.issued = engine_host::http_fetch("GET", "{base}/probe", "", "");
      }
    });
}
"#;

fn free_port() -> u16 {
    let l = TcpListener::bind("127.0.0.1:0").expect("a free port");
    l.local_addr().expect("local address").port()
}

fn scratch(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("loft_c_https_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&d);
    fa::create_dir_all(&d).expect("a scratch directory");
    d
}

/// Run the program on `backend` with `env`; answers stdout and stderr.
fn run(backend: &str, dir: &Path, env: &[(&str, String)], ca: Option<&str>) -> (String, String) {
    let file = dir.join("https.loft");
    fa::write(&file, PROGRAM).expect("write the program");
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("lib");
    let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
    cmd.arg(backend)
        .arg("--no-warnings")
        .arg("--lib")
        .arg(&lib)
        .arg(&file)
        .current_dir(dir)
        .env("LOFT_TIMEOUT", "120")
        .env_remove("SSL_CERT_FILE")
        .env_remove("SSL_CERT_DIR");
    for (k, v) in env {
        cmd.env(k, v);
    }
    if let Some(ca) = ca {
        cmd.env("SSL_CERT_FILE", ca);
    }
    let out = cmd.output().expect("loft runs");
    (
        String::from_utf8_lossy(&out.stdout).to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

/// The image the server serves, written by the program itself (`store_persist_copy` writes the
/// `.dschema` sidecar beside it).
fn world(dir: &Path) {
    let store = dir.join("world.store");
    let (out, err) = run(
        "--interpret",
        dir,
        &[("LOFT_HTTPS_STORE", store.to_string_lossy().to_string())],
        None,
    );
    assert!(
        out.contains("WROTE true"),
        "the image is written: {out}{err}"
    );
}

const BACKENDS: [&str; 2] = ["--interpret", "--native-c"];

/// A trusted CA, the host named both ways the certificate names it: the image loads and the
/// fetch answers, on both backends.  `localhost` is checked by BearSSL's DNS-name match,
/// `127.0.0.1` by the runtime's iPAddress check.
#[test]
fn https_through_a_trusted_ca_answers_on_both_backends() {
    let ca = format!("{FIXTURES}ca.pem");
    for host in ["127.0.0.1", "localhost"] {
        for backend in BACKENDS {
            let dir = scratch(&format!("ok_{host}{}", &backend[1..]));
            world(&dir);
            let (port, seen) = start_server("127.0.0.1", dir.clone());
            let env = [
                ("LOFT_HTTPS_BASE", format!("https://{host}:{port}")),
                ("LOFT_HTTPS_WS", free_port().to_string()),
            ];
            let (out, err) = run(backend, &dir, &env, Some(&ca));
            assert!(
                out.contains("LOAD ok=true h13=1300"),
                "{backend} {host}: the image loads over https\nstdout: {out}\nstderr: {err}\nserver saw: {:?}",
                seen.lock().expect("log")
            );
            assert!(
                out.contains("FETCH status=200 body=hello-tls"),
                "{backend} {host}: http_fetch answers over https\nstdout: {out}\nstderr: {err}"
            );
            let _ = fa::remove_dir_all(&dir);
        }
    }
}

/// A CA the machine does not trust: no request crosses the handshake, the load answers false
/// naming the certificate, and the fetch completes with a negative status.
#[test]
fn https_through_an_untrusted_ca_is_refused_on_both_backends() {
    for backend in BACKENDS {
        let dir = scratch(&format!("untrusted{}", &backend[1..]));
        world(&dir);
        let (port, seen) = start_server("127.0.0.1", dir.clone());
        let env = [
            ("LOFT_HTTPS_BASE", format!("https://127.0.0.1:{port}")),
            ("LOFT_HTTPS_WS", free_port().to_string()),
        ];
        let (out, err) = run(backend, &dir, &env, None);
        let seen = seen.lock().expect("log").clone();
        assert!(
            seen.is_empty(),
            "{backend}: no request may cross an untrusted handshake, but the server saw {seen:?}"
        );
        assert!(
            out.contains("LOAD ok=false") && out.contains("FETCH status=-1"),
            "{backend}: both clients refuse\nstdout: {out}\nstderr: {err}"
        );
        // The load says its sidecar could not be FETCHED — never that there is none, which
        // would send the reader looking for a missing file instead of at the certificate.
        assert!(
            err.contains("could not be fetched") && !err.contains("no layout sidecar"),
            "{backend}: the load names the failed fetch, not a missing sidecar: {err}"
        );
        let fetch = out.lines().find(|l| l.starts_with("FETCH")).unwrap_or_default();
        assert!(
            fetch.contains("UnknownIssuer") || fetch.contains("certificate"),
            "{backend}: the refusal names the certificate: {fetch}\nstderr: {err}"
        );
        let _ = fa::remove_dir_all(&dir);
    }
}

/// A trusted certificate that does not name the address connected to: `127.0.0.2` reaches the
/// same server (Linux routes all of 127/8 to the loopback), whose certificate names `127.0.0.1`
/// only.  The C refuses it at its iPAddress check, which BearSSL alone does not make.
#[cfg(target_os = "linux")]
#[test]
fn https_to_an_address_the_certificate_does_not_name_is_refused() {
    let ca = format!("{FIXTURES}ca.pem");
    for backend in BACKENDS {
        let dir = scratch(&format!("name{}", &backend[1..]));
        world(&dir);
        let (port, seen) = start_server("0.0.0.0", dir.clone());
        let env = [
            ("LOFT_HTTPS_BASE", format!("https://127.0.0.2:{port}")),
            ("LOFT_HTTPS_WS", free_port().to_string()),
        ];
        let (out, err) = run(backend, &dir, &env, Some(&ca));
        let seen = seen.lock().expect("log").clone();
        assert!(
            seen.is_empty(),
            "{backend}: no request may cross a handshake for the wrong name, but the server saw {seen:?}"
        );
        assert!(
            out.contains("LOAD ok=false") && out.contains("FETCH status=-1"),
            "{backend}: both clients refuse\nstdout: {out}\nstderr: {err}"
        );
        let _ = fa::remove_dir_all(&dir);
    }
}
