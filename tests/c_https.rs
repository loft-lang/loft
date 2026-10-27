// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![cfg(feature = "registry")]

//! A fetch over a URL from a compiled program answers as it does interpreted (@PLN203 C5): the C
//! runtime's TLS client (`src/c/tls/loft_tls.c`, BearSSL) under its HTTP client — `store_load_url*`'s
//! whole-image load, `engine_host::http_fetch`, and the paged loaders' `Range` reads of an
//! `http(s)://` image (`HttpRangeProvider`'s port) — trusts what `src/tls.rs` trusts and refuses
//! what it refuses, and a paged load reads only the pages it needs.
//!
//! Hermetic, as `tls_trust.rs` is: a local server — HTTPS signed by the test CA of
//! `tests/fixtures/tls/` (its certificate names `127.0.0.1` and `localhost`), the CA trusted
//! through `SSL_CERT_FILE`, or plain HTTP — that answers `Range` requests or, as some hosts do,
//! ignores them.  The SERVER is the witness: it records the request line of every connection
//! whose handshake completed, so "refused" means no request crossed, not that the program printed
//! a failure, and it counts the bytes it sent, so "paged" means the image did not cross whole.
//! Each case is its own process, because the trust store is read once per process.

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

/// How the test server answers.
#[derive(Clone, Copy)]
struct Serve {
    tls: bool,
    /// Answer `Range: bytes=a-b` with a 206; when false the whole file comes back, as from a
    /// host that ignores the header.
    ranges: bool,
}

const HTTPS: Serve = Serve {
    tls: true,
    ranges: true,
};

/// What the server saw: the request line (with its `Range`, if any) of every request that
/// arrived over a completed handshake, and the body bytes it sent per path.
#[derive(Clone, Default)]
struct Seen {
    lines: Arc<Mutex<Vec<String>>>,
    sent: Arc<Mutex<Vec<(String, usize)>>>,
}

impl Seen {
    fn lines(&self) -> Vec<String> {
        self.lines.lock().expect("log").clone()
    }
    fn sent_for(&self, path: &str) -> usize {
        let sent = self.sent.lock().expect("log");
        sent.iter().filter(|(p, _)| p == path).map(|(_, n)| n).sum()
    }
}

/// Serve the files of `dir` over HTTPS on `bind` (`/probe` answers `hello-tls`).
fn start_server(bind: &str, dir: PathBuf) -> (u16, Arc<Mutex<Vec<String>>>) {
    let (port, seen) = start_serving(bind, dir, HTTPS);
    (port, seen.lines)
}

/// Serve the files of `dir` on `bind` as `how` says.
fn start_serving(bind: &str, dir: PathBuf, how: Serve) -> (u16, Seen) {
    let listener = TcpListener::bind(format!("{bind}:0")).expect("bind a local port");
    let port = listener.local_addr().expect("local address").port();
    let seen = Seen::default();
    let log = seen.clone();
    let cfg = server_config();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(tcp) = stream else { continue };
            let (cfg, log, dir) = (Arc::clone(&cfg), log.clone(), dir.clone());
            std::thread::spawn(move || {
                if how.tls {
                    let Ok(conn) = rustls::ServerConnection::new(cfg) else {
                        return;
                    };
                    let mut tls = rustls::StreamOwned::new(conn, tcp);
                    answer(&mut tls, &dir, how, &log);
                    tls.conn.send_close_notify();
                    let _ = tls.flush();
                } else {
                    let mut tcp = tcp;
                    answer(&mut tcp, &dir, how, &log);
                    let _ = tcp.flush();
                }
            });
        }
    });
    (port, seen)
}

/// One request on `s`: the file it names, a `Range` of it when asked and `how` honours it.
fn answer<S: Read + Write>(s: &mut S, dir: &Path, how: Serve, log: &Seen) {
    let mut head = Vec::new();
    let mut buf = [0u8; 4096];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") {
        match s.read(&mut buf) {
            Ok(0) | Err(_) => return, // the handshake failed, or no request came
            Ok(n) => head.extend_from_slice(&buf[..n]),
        }
    }
    let request = String::from_utf8_lossy(&head).to_string();
    let line = request.lines().next().unwrap_or_default().to_string();
    let range = request.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        if !k.trim().eq_ignore_ascii_case("range") {
            return None;
        }
        let (a, b) = v.trim().strip_prefix("bytes=")?.split_once('-')?;
        Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?))
    });
    log.lines.lock().expect("log").push(match range {
        Some((a, b)) => format!("{line} bytes={a}-{b}"),
        None => line.clone(),
    });
    let path = line.split(' ').nth(1).unwrap_or("/").to_string();
    let file: Option<Vec<u8>> = if path == "/probe" {
        Some(b"hello-tls".to_vec())
    } else {
        fa::read(dir.join(path.trim_start_matches('/'))).ok()
    };
    let Some(file) = file else {
        let _ = s
            .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    };
    let total = file.len();
    let (status, extra, body) = match range {
        Some((a, b)) if how.ranges && a < total => {
            let end = b.min(total - 1);
            (
                "206 Partial Content",
                format!("Content-Range: bytes {a}-{end}/{total}\r\n"),
                &file[a..=end],
            )
        }
        Some(_) if how.ranges => (
            "416 Range Not Satisfiable",
            format!("Content-Range: bytes */{total}\r\n"),
            &file[0..0],
        ),
        _ => ("200 OK", String::new(), &file[..]),
    };
    let wrote = s
        .write_all(
            format!(
                "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .as_bytes(),
        )
        .and_then(|()| s.write_all(body));
    if wrote.is_ok() {
        log.sent.lock().expect("log").push((path, body.len()));
    }
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
        let fetch = out
            .lines()
            .find(|l| l.starts_with("FETCH"))
            .unwrap_or_default();
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

/// The paged loaders over a URL.  The program writes two images when `LOFT_PG_BASE` is empty —
/// a hash of 100 000 records (`id = 7 i`, `val = 3 i + 1`, `name = "n<i>"`) and a sorted one of
/// the same records — and otherwise reads them back by key, by several keys, by a key range and
/// through a lazy binding, printing what it found.
const PAGED: &str = r#"
struct Rec {
  id: integer not null,
  val: integer not null,
  name: text,
}

fn main() {
  base = env_variable("LOFT_PG_BASE") ?? "";
  if base == "" {
    h: hash<Rec[id]> = [];
    s: sorted<Rec[id]> = [];
    for i in 0..100000 {
      h += Rec { id: 7 * i, val: 3 * i + 1, name: "n{i}" };
      s += Rec { id: 7 * i, val: 3 * i + 1, name: "n{i}" };
    }
    dir = env_variable("LOFT_PG_DIR") ?? "";
    a = store_persist_copy(h, "{dir}/big.store");
    b = store_persist_copy(s, "{dir}/sorted.store");
    println("WROTE {a} {b}");
    return
  }
  url = "{base}/big.store";
  one: hash<Rec[id]> = [];
  hit = store_load_key(one, url, 8638);
  miss = store_load_key(one, url, 5);
  r = one[8638];
  shown = if r == null { "null" } else { "{r.val} {r.name}" };
  println("KEY hit={hit} miss={miss} rec={shown} len={len(one)}");
  many: hash<Rec[id]> = [];
  n = store_load_keys(many, url, [0, 70, 699993, 700000]);
  last = many[699993];
  lv = if last == null { "null" } else { "{last.val} {last.name}" };
  println("KEYS n={n} last={lv}");
  ordered: sorted<Rec[id]> = [];
  got = store_load_range(ordered, "{base}/sorted.store", 700, 770);
  sum = 0;
  for e in ordered { sum += e.val; }
  println("RANGE n={got} sum={sum}");
  lazy: hash<Rec[id]> = [];
  bound = store_bind_lazy(lazy, url);
  l = lazy[5439];
  ls = if l == null { "null" } else { "{l.val} {l.name}" };
  println("LAZY bound={bound} rec={ls} error={store_lazy_error(lazy)}");
}
"#;

/// The answers a reachable, well-formed pair of images gives, computed by hand: key 8638 is
/// record 1234 (`val` 3703), 5 is no multiple of 7; of the four keys 700000 is past the last
/// (699993, record 99999, `val` 299998); ids 700‥770 are records 100‥110, whose `val`s sum to
/// 3 · 1155 + 11 = 3476; key 5439 is record 777 (`val` 2332).
const PAGED_ANSWERS: [&str; 4] = [
    "KEY hit=true miss=false rec=3703 n1234 len=1",
    "KEYS n=3 last=299998 n99999",
    "RANGE n=11 sum=3476",
    "LAZY bound=true rec=2332 n777 error=",
];

const IMAGES: [&str; 4] = [
    "big.store",
    "big.store.dschema",
    "sorted.store",
    "sorted.store.dschema",
];

/// The two images and their sidecars copied into `dir`: written once per process (interpreted;
/// `store_persist_copy` writes each sidecar), because a case may change its copy.
fn paged_images(dir: &Path) {
    static WRITTEN: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    let from = WRITTEN.get_or_init(|| {
        let at = scratch("pg_images");
        let (out, err) = run_paged("--interpret", &at, &[("LOFT_PG_DIR", path_text(&at))], None);
        assert!(
            out.contains("WROTE true true"),
            "the images are written: {out}{err}"
        );
        at
    });
    for f in IMAGES {
        fa::copy(from.join(f), dir.join(f)).expect("copy an image");
    }
}

fn path_text(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

fn run_paged(
    backend: &str,
    dir: &Path,
    env: &[(&str, String)],
    ca: Option<&str>,
) -> (String, String) {
    let file = dir.join("paged.loft");
    fa::write(&file, PAGED).expect("write the program");
    let mut cmd = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"));
    cmd.arg(backend)
        .arg("--no-warnings")
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

/// Every paged loader reads an image over plain HTTP, over HTTPS, and from a host that ignores
/// `Range`, answering what it answers from a file; where ranges are served, the image never
/// crosses whole and every request for it is a range.
#[test]
fn a_paged_load_over_a_url_reads_by_range_on_both_backends() {
    let ca = format!("{FIXTURES}ca.pem");
    let ways = [
        (
            "http",
            Serve {
                tls: false,
                ranges: true,
            },
        ),
        ("https", HTTPS),
        (
            "http",
            Serve {
                tls: false,
                ranges: false,
            },
        ),
    ];
    for (scheme, how) in ways {
        for backend in BACKENDS {
            let tag = format!("pg_{scheme}_{}{}", how.ranges, &backend[1..]);
            let dir = scratch(&tag);
            paged_images(&dir);
            let size = fa::metadata(dir.join("big.store"))
                .expect("the image")
                .len() as usize;
            let (port, seen) = start_serving("127.0.0.1", dir.clone(), how);
            let env = [("LOFT_PG_BASE", format!("{scheme}://127.0.0.1:{port}"))];
            let (out, err) = run_paged(backend, &dir, &env, how.tls.then_some(ca.as_str()));
            for want in PAGED_ANSWERS {
                assert!(
                    out.lines().any(|l| l == want),
                    "{tag}: wants `{want}`\nstdout: {out}\nstderr: {err}\nserver saw: {:?}",
                    seen.lines()
                );
            }
            if how.ranges {
                let image: Vec<String> = seen
                    .lines()
                    .into_iter()
                    .filter(|l| l.starts_with("GET /big.store "))
                    .collect();
                assert!(
                    !image.is_empty() && image.iter().all(|l| l.contains(" bytes=")),
                    "{tag}: every request for the image is a range: {image:?}"
                );
                let sent = seen.sent_for("/big.store");
                assert!(
                    sent * 4 < size,
                    "{tag}: a paged load reads a few pages, not the image: {sent} of {size} bytes"
                );
            }
            let _ = fa::remove_dir_all(&dir);
        }
    }
}

/// What a paged load over a URL refuses, and how it says so: a sidecar that could not be
/// FETCHED (an untrusted certificate) is not an image without one; a host's 404 for the sidecar
/// is; and a reachable URL whose bytes are no store image is refused at its signature, which a
/// lazy binding names.
#[test]
fn a_paged_load_over_a_url_refuses_what_it_cannot_trust_on_both_backends() {
    for backend in BACKENDS {
        let dir = scratch(&format!("pg_refuse{}", &backend[1..]));
        paged_images(&dir);
        // An untrusted CA: nothing crosses, and the refusal names the fetch.
        let (port, seen) = start_serving("127.0.0.1", dir.clone(), HTTPS);
        let env = [("LOFT_PG_BASE", format!("https://127.0.0.1:{port}"))];
        let (out, err) = run_paged(backend, &dir, &env, None);
        assert!(
            seen.lines().is_empty(),
            "{backend}: nothing crosses an untrusted handshake: {:?}",
            seen.lines()
        );
        assert!(
            out.contains("KEY hit=false miss=false rec=null len=0")
                && out.contains("RANGE n=0 sum=0"),
            "{backend}: an untrusted host loads nothing\nstdout: {out}\nstderr: {err}"
        );
        assert!(
            err.contains("could not be fetched") && !err.contains("no layout sidecar"),
            "{backend}: the refusal names the failed fetch, not a missing sidecar: {err}"
        );
        // The sidecar is gone (404): the image records no layout, so it is not read.
        let plain = Serve {
            tls: false,
            ranges: true,
        };
        fa::remove_file(dir.join("big.store.dschema")).expect("remove the sidecar");
        let (port, _seen) = start_serving("127.0.0.1", dir.clone(), plain);
        let env = [("LOFT_PG_BASE", format!("http://127.0.0.1:{port}"))];
        let (out, err) = run_paged(backend, &dir, &env, None);
        assert!(
            out.contains("KEY hit=false") && err.contains("no layout sidecar"),
            "{backend}: an image without a sidecar is refused as such\nstdout: {out}\nstderr: {err}"
        );
        // A sidecar beside bytes that are no store image: the open refuses it at the signature.
        paged_images(&dir);
        fa::write(dir.join("big.store"), vec![b'x'; 200_000]).expect("overwrite the image");
        let (port, _seen) = start_serving("127.0.0.1", dir.clone(), plain);
        let env = [("LOFT_PG_BASE", format!("http://127.0.0.1:{port}"))];
        let (out, err) = run_paged(backend, &dir, &env, None);
        assert!(
            out.contains("KEY hit=false") && err.contains("it cannot be opened as a paged source"),
            "{backend}: bytes that are no image are refused\nstdout: {out}\nstderr: {err}"
        );
        let lazy = out
            .lines()
            .find(|l| l.starts_with("LAZY"))
            .unwrap_or_default();
        assert!(
            lazy.contains("does not begin with the store signature"),
            "{backend}: the lazy binding names the signature: {lazy}\nstderr: {err}"
        );
        let _ = fa::remove_dir_all(&dir);
    }
}
