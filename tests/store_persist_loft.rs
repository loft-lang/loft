// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @PLN203 C-gap: compaction and the trie/spatial rebuild of a persisted image differ on the C — its `--native` runs as `--native-rust` until then.

//! @PLAN38 — end-to-end exercise of store images through the compiled `loft`
//! binary: `store_persist_copy` writes one, `store_load` and the paged
//! loaders (`store_load_key*`, `store_load_range`, `store_load_prefix`, the
//! URL loaders) read it back, in a separate process and on both backends.
//!
//! The smoke pair is `tests/scripts/store_persist_smoke.loft`: a fresh-mode run
//! populates a hash and writes it, a reload-mode run loads it into an empty
//! hash and reads the keys back, and the two runs' outputs must agree.

#![cfg(feature = "mmap")]

use loft::file_access as fa;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::thread;

/// Serve `bytes` over a minimal HTTP/1.1 server that honours `Range: bytes=a-b`
/// with a `206 Partial Content` + `Content-Range` (200/whole-file otherwise) —
/// the remote store the `store_load_key` HTTP path (Phase 5) fetches from.
/// Returns the URL; the server thread is detached (ends with the test binary).
fn serve_ranges(store: Vec<u8>, sidecar: Option<Vec<u8>>) -> String {
    serve_ranges_tuned(store, sidecar, 0, None).0
}

/// loft#782 — the same server, with the two knobs a ROUND-TRIP claim needs.
///
/// `delay_ms` is injected per request, and every connection is handled on its own thread,
/// so a client that issues its ranges concurrently finishes in ~one delay while a serial
/// one pays `requests × delay`. Both matter: with the single-threaded server the socket
/// would serialise a concurrent client anyway, and the harness would report "no win" for a
/// fix that worked.
///
/// `counter` counts requests served, which is the DETERMINISTIC half — wall-clock is
/// suggestive and flaky, but round-trip depth is exact and is what the defect is about.
fn serve_ranges_tuned(
    store: Vec<u8>,
    sidecar: Option<Vec<u8>>,
    delay_ms: u64,
    counter: Option<std::sync::Arc<std::sync::atomic::AtomicUsize>>,
) -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().unwrap().port();
    let hits =
        counter.unwrap_or_else(|| std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)));
    let out = std::sync::Arc::clone(&hits);
    let store = std::sync::Arc::new(store);
    let sidecar = std::sync::Arc::new(sidecar);
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut s) = stream else { break };
            let store = std::sync::Arc::clone(&store);
            let sidecar = std::sync::Arc::clone(&sidecar);
            let hits = std::sync::Arc::clone(&hits);
            thread::spawn(move || {
                let (store, sidecar) = (&*store, &*sidecar);
                hits.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if delay_ms > 0 {
                    thread::sleep(std::time::Duration::from_millis(delay_ms));
                }
                let mut buf = [0u8; 2048];
                let n = s.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                // Path-aware (real servers 404 a missing sidecar): serve the layout
                // sidecar whole at `/store.dschema` (or 404 when absent), so the
                // @PLN97 3b.5 gate fetches a REAL sidecar, not the store bytes.
                let req_path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/");
                if req_path.ends_with(".dschema") {
                    match &sidecar {
                        Some(sc) => {
                            let hdr = format!(
                                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                sc.len()
                            );
                            let _ = s.write_all(hdr.as_bytes());
                            let _ = s.write_all(sc);
                        }
                        None => {
                            let _ = s.write_all(
                            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                        }
                    }
                    return; // the sidecar path is answered; this connection is done
                }
                let total = store.len();
                let range = req.lines().find_map(|l| {
                    l.trim()
                        .strip_prefix("Range: bytes=")
                        .or_else(|| l.trim().strip_prefix("range: bytes="))
                });
                if let Some(r) = range {
                    let (a, b) = r.split_once('-').unwrap_or(("0", ""));
                    let a: usize = a.trim().parse().unwrap_or(0);
                    let b: usize = b
                        .trim()
                        .parse()
                        .unwrap_or(total.saturating_sub(1))
                        .min(total.saturating_sub(1));
                    let body = if a <= b { &store[a..=b] } else { &store[0..0] };
                    let hdr = format!(
                        "HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {a}-{b}/{total}\r\n\
                     Content-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = s.write_all(hdr.as_bytes());
                    let _ = s.write_all(body);
                } else {
                    let hdr = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {total}\r\nConnection: close\r\n\r\n"
                    );
                    let _ = s.write_all(hdr.as_bytes());
                    let _ = s.write_all(store);
                }
            });
        }
    });
    (format!("http://127.0.0.1:{port}/store"), out)
}

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn smoke_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_persist_smoke.loft")
}

fn sorted_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_persist_sorted.loft")
}

/// Run a persist script in a chosen mode (fresh / reload) — the generic form
/// of `run_smoke`, parameterised by the script so the hash and sorted cases
/// share one driver.
fn run_mode(script: &Path, path: &Path, mode: &str) -> (String, i32) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(script)
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        eprintln!("{mode} stderr:\n{stderr}");
    }
    (stdout, out.status.code().unwrap_or(-1))
}

fn scratch(test_name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = base.join("loft-store-persist-loft").join(test_name);
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn run_smoke(path: &Path, mode: &str) -> (String, i32) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(smoke_script())
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let code = out.status.code().unwrap_or(-1);
    if !out.status.success() {
        eprintln!("smoke stderr:\n{stderr}");
    }
    (stdout, code)
}

/// Run an arbitrary persist script on a chosen backend with the scratch path.
fn run_script(script: &Path, backend: &str, path: &Path) -> (String, i32) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(script)
        .env("LOFT_PERSIST_TEST_PATH", path)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        eprintln!("{backend} stderr:\n{stderr}");
    }
    (stdout, out.status.code().unwrap_or(-1))
}

/// A paged working-set load must accept an ENUM field — one `kind:` was enough
/// to lose the range read of a whole collection.
///
/// `is_copyable_field` had an arm for a struct and for a struct-enum VARIANT and
/// none for the enum itself, so every `Parts::Enum` fell to the catch-all and the
/// load was refused — with a message blaming a `vector<text>` the type did not
/// have. The values are what is checked here, not just that the load was allowed:
/// a copy that is accepted and wrong is the worse failure of the two.
#[test]
fn paged_load_carries_enum_fields_both_backends() {
    let script = workspace_root().join("tests/scripts/store_paged_enum_field.loft");
    for backend in ["--interpret", "--native-rust"] {
        let dir = scratch(&format!("paged_enum_{}", backend.trim_start_matches('-')));
        let path = dir.join("recs.store");
        let (out, code) = run_script(&script, backend, &path);
        assert_eq!(code, 0, "{backend} exit: {out:?}");
        assert!(
            out.contains(
                "paged-enum: a:Bytes/0,0,0/40:1/11 b:Ogg/7,9,0/50:2/22 c:Wav/0,0,13/60:3/33"
            ),
            "{backend}: every variant, payload and byte must survive the paged copy \
             (the bug refused the load outright): {out:?}"
        );
    }
}

/// A `store_load` that never returns is worse than one that answers wrong, and
/// compaction-on-load produced exactly that: it rebuilt the image into a scratch
/// store and wrote the root block's header back as the word count it had ASKED
/// for, where `claim` had handed out the whole arena.  The word past the
/// shortened header became a block owning nothing, and `claim_scan` steps over a
/// block by adding its size to its position — so every later claim on that store
/// looped forever.
///
/// The shape is an asset pack's (@PLN146 F1): several keyed collections in one
/// container, holding vectors big enough that a later claim misses the free list
/// and reaches the linear scan.  Bounded here rather than left to the suite
/// watchdog, because the failure mode under test IS an unbounded loop.
#[test]
fn compaction_on_load_returns_both_backends() {
    let script = workspace_root().join("tests/scripts/store_load_compaction_scratch_header.loft");
    for backend in ["--interpret", "--native-rust"] {
        let dir = scratch(&format!(
            "compaction_scratch_header_{}",
            backend.trim_start_matches('-')
        ));
        let path = dir.join("pack.store");
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", &path)
            .env("LOFT_TIMEOUT", "120")
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert_eq!(
            out.status.code().unwrap_or(-1),
            0,
            "{backend} exit: {stdout:?} / {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            stdout.contains("compaction-load: loaded=true page=4096"),
            "{backend}: the pack must load back whole (the bug hung here): {stdout:?}"
        );
        assert!(
            stdout.contains("compaction-load: blob=3072 last=219"),
            "{backend}: every byte must survive the rebuild: {stdout:?}"
        );
    }
}

/// @C154 — an image is written from a store whose every byte was written: a write whose
/// rebuild lands in a store that is not DEFINED stops on `Store::raw_bytes` (measured: with
/// `rebuild_into_scratch` building into `Store::new_in_use`, this test and the sorted one fail
/// on both backends with that refusal).
#[test]
fn fresh_then_reload_round_trip() {
    let dir = scratch("fresh_then_reload_round_trip");
    let path = dir.join("world.store");

    // Pass 1 — path does not exist; the script populates a hash and
    // writes it to disk.
    assert!(!fa::exists(&path), "scratch should start clean");
    let (out1, code1) = run_smoke(&path, "fresh");
    assert_eq!(code1, 0, "fresh exit: stdout={out1:?}");
    assert!(
        out1.contains("fresh keys=7,13,42"),
        "fresh keys missing: {out1:?}"
    );
    assert!(fa::exists(&path), "the write should have created the file");
    let meta = fa::metadata(&path).unwrap();
    assert!(
        meta.len() >= 8192,
        "file should be padded to ≥1024 words: got {} bytes",
        meta.len()
    );

    // Pass 2 — same path, empty in-memory hash; the load should read the
    // on-disk contents.
    let (out2, code2) = run_smoke(&path, "reload");
    assert_eq!(code2, 0, "reload exit: stdout={out2:?}");
    assert!(
        out2.contains("reload keys=7,13,42"),
        "reload keys mismatch: {out2:?}"
    );
    // #523 — a KEY LOOKUP must succeed in this SEPARATE reload process, not
    // just iteration.  Pre-fix, the hash used a per-process random seed so the
    // reload process probed a different bucket and read null (v=1300 vanished)
    // while iteration still listed the key.  The per-hash seed now lives in the
    // bucket record, so any reader re-derives the same buckets.
    assert!(
        out2.contains("reload lookup h[13]=1300"),
        "reload cross-process lookup must find v=1300, not null (#523): {out2:?}"
    );
}

/// @PLN123 A0 — the high-water mark of a written image, re-opened from its
/// file.  Gating a shrink on `walk_complete` is only worth anything if a
/// healthy image reports it TRUE; were the image to leave a zero-header tail,
/// the gate would refuse forever while every test still passed.  A padded image
/// tiles its arena exactly — `build_padded_store_image` lays one free block over
/// the slack above the last record — and this pins that.
#[test]
fn a_written_image_reports_a_trustworthy_mark() {
    let dir = scratch("image_mark");
    let path = dir.join("world.store");
    let (out, code) = run_smoke(&path, "fresh");
    assert_eq!(code, 0, "fresh exit: stdout={out:?}");

    let store = loft::store::Store::open(path.to_str().expect("utf-8 path"));
    let u = store.usage();
    assert!(
        u.walk_complete,
        "an image's block chain must tile its file, or a shrink can never \
         trust its mark: claimed={} free={} mark={} capacity={}",
        u.claimed_words, u.free_words, u.live_end_words, u.capacity_words
    );
    assert!(
        u.claimed_words > 0,
        "the image holds the smoke script's records"
    );
    assert!(
        u.live_end_words <= u.capacity_words,
        "mark {} past capacity {}",
        u.live_end_words,
        u.capacity_words
    );
}

/// @PLN97 arc G Phase 0.5b — a `sorted<T[k]>` image must round-trip across
/// processes: a `sorted` is comparison-based (no per-process hash seed), so the
/// reload process must iterate in key order AND key-look-up correctly.
#[test]
fn sorted_fresh_then_reload_round_trip() {
    let dir = scratch("sorted_fresh_then_reload_round_trip");
    let path = dir.join("sorted.store");

    assert!(!fa::exists(&path), "scratch should start clean");
    let (out1, code1) = run_mode(&sorted_script(), &path, "fresh");
    assert_eq!(code1, 0, "fresh exit: stdout={out1:?}");
    assert!(
        out1.contains("fresh keys=7,13,42"),
        "fresh sorted keys must be ascending 7,13,42: {out1:?}"
    );
    assert!(fa::exists(&path), "the write should have created the file");

    // Reload in a SEPARATE process: both iteration and lookup must survive.
    let (out2, code2) = run_mode(&sorted_script(), &path, "reload");
    assert_eq!(code2, 0, "reload exit: stdout={out2:?}");
    assert!(
        out2.contains("reload keys=7,13,42"),
        "reload sorted keys mismatch: {out2:?}"
    );
    assert!(
        out2.contains("reload lookup s[13]=1300"),
        "reload cross-process sorted lookup must find v=1300, not null: {out2:?}"
    );
}

/// @PLN134 — persisting a `trie<T[k]>` lays its node array out for paging, and
/// moves no answer.
///
/// `store_persist_copy` renumbers every trie in the image van Emde Boas before it
/// writes it, because that image is what a paged reader fetches: node ids handed
/// out in INSERTION order cost one prefix query 27 pages of 64 KB to read ~330
/// bytes of nodes, and 2.8 after.  The pass rewrites ids and nothing else, so the
/// gate is that nothing observable moves — and what a renumbering could break is
/// exactly key order, exact lookup, and the prefix walk.
///
/// Both halves are asserted, and they are different claims: the `LOFT_LOADER_STATS`
/// line proves the pass RAN (without it a silently-skipped pass reads identically
/// to a working one), and the fresh/reload agreement proves it broke nothing.
/// Reload is a separate process, so the image alone carries the answers.
///
/// `kerx` is the cell that matters, as it is in the trie's own tests: it sorts
/// immediately after `kerkweg`, so a prefix walk that loses its stop condition
/// answers `kerx*` for `kerk`.
#[test]
fn trie_persist_lays_out_and_keeps_every_answer() {
    let script = workspace_root().join("tests/scripts/134-trie-paged-layout.loft");
    let dir = scratch("trie_persist_layout");
    let path = dir.join("words.store");
    const KERK: &str = "kerk=kerk,kerkdijk,kerkhof,kerklaan,kerkpad,kerkplein,\
                        kerksloot,kerkstraat,kerkweg";

    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(&script)
        .env("LOFT_PERSIST_TEST_PATH", &path)
        .env("LOFT_PERSIST_TEST_MODE", "fresh")
        .env("LOFT_LOADER_STATS", "1")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let fresh = String::from_utf8_lossy(&out.stdout).into_owned();
    let stats = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code().unwrap_or(-1), 0, "fresh exit: {fresh:?}");
    assert!(
        stats.contains("laid out 1 tree(s) for paging"),
        "the layout pass must run on the image being written: {stats:?}"
    );
    assert!(fresh.contains("fresh len=72"), "fresh: {fresh:?}");
    assert!(
        fresh.contains(&format!("fresh {KERK}")),
        "the prefix walk must stop before `kerx`: {fresh:?}"
    );
    assert!(fresh.contains("fresh lookup=3"), "fresh: {fresh:?}");
    assert!(fresh.contains("fresh absent=null"), "fresh: {fresh:?}");
    assert!(fa::exists(&path), "the write should have created the file");

    // A SEPARATE process, reading only the laid-out image.
    let (reload, code) = run_mode(&script, &path, "reload");
    assert_eq!(code, 0, "reload exit: {reload:?}");
    for line in fresh.lines().filter_map(|l| l.strip_prefix("fresh ")) {
        assert!(
            reload.contains(&format!("reload {line}")),
            "the image must answer exactly as the live trie did — missing {line:?} \
             in {reload:?}"
        );
    }
}

fn load_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_smoke.loft")
}

fn text_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_text.loft")
}

fn vec_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_vec.loft")
}

fn textkey_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_textkey.loft")
}

fn range_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_range.loft")
}

fn nested_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_nested.loft")
}

fn prefix_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_prefix.loft")
}

fn vecstruct_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_vecstruct.loft")
}

fn gate_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_layout_gate.loft")
}

fn gate_changed_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_layout_gate_changed.loft")
}

fn gate_refused_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_layout_gate_refused.loft")
}

fn vectext_refuse_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_load_vectext_refuse.loft")
}

/// `run_mode` fixed to `--interpret`, parameterised by backend so the heap
/// `store_load` path can be exercised on the native backend too.
fn run_mode_backend(backend: &str, script: &Path, path: &Path, mode: &str) -> (String, i32) {
    run_mode_backend_env(backend, script, path, mode, &[])
}

/// `run_mode_backend` with extra environment for the child.
fn run_mode_backend_env(
    backend: &str,
    script: &Path,
    path: &Path,
    mode: &str,
    env: &[(&str, &str)],
) -> (String, i32) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(script)
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .envs(env.iter().copied())
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if !out.status.success() {
        eprintln!("{backend} {mode} stderr:\n{stderr}");
    }
    (stdout, out.status.code().unwrap_or(-1))
}

/// @PLN97 arc G Phase 1 — `store_load` reads a written store IMAGE into a FRESH
/// hash and returns the same keys AND key-lookups, on BOTH the interpreter and
/// native.
#[test]
fn store_load_reads_persisted_image_both_backends() {
    let dir = scratch("store_load_phase1");
    let path = dir.join("load.store");

    // write the image once
    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(
        out_w.contains("write keys=7,13,42"),
        "write keys: {out_w:?}"
    );
    assert!(fa::exists(&path), "the write should create the file");

    // load it back via the heap path on both backends — same keys + lookup
    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_mode_backend(backend, &load_script(), &path, "load");
        assert_eq!(code, 0, "{backend} load exit: {out:?}");
        assert!(
            out.contains("load keys=7,13,42"),
            "{backend}: store_load must read all keys: {out:?}"
        );
        assert!(
            out.contains("load lookup h[13]=1300"),
            "{backend}: store_load key lookup must find v=1300: {out:?}"
        );
    }
}

/// A garbage / non-store file must make `store_load` return false (a clean
/// reject), never panic or misread past the signature check.
#[test]
fn store_load_rejects_garbage_file() {
    let dir = scratch("store_load_reject");
    let path = dir.join("garbage.store");
    fa::write(
        &path,
        b"this is not a loft store image, only junk bytes here!!",
    )
    .unwrap();

    let (out, code) = run_mode(&load_script(), &path, "load");
    assert_eq!(
        code, 0,
        "reject should be graceful (script prints FAIL, exit 0): {out:?}"
    );
    assert!(
        out.contains("FAIL load-returned-false"),
        "garbage file must reject cleanly (store_load=false): {out:?}"
    );
}

/// The writer and the reader of [`a_store_image_cut_short_is_refused_on_every_backend`]: three
/// hundred records and three hundred texts, so the image is many blocks long.
const CUT_TYPES: &str = "struct Cell { ce_key: text, ce_x: integer, ce_y: integer }
struct Box { cells: hash<Cell[ce_key]>, names: vector<text> }
";
const CUT_WRITE: &str = "fn main() {
  b = Box { cells: [], names: [] };
  for i in 0..300 {
    b.cells += [Cell { ce_key: \"cell-{i}\", ce_x: i * 3, ce_y: i * 7 }];
    b.names += [\"name number {i}\"];
  }
  assert(store_persist_copy(b, \"whole.store\"), \"write\");
}
";
const CUT_READ: &str = "fn read(name: text) {
  b = Box { cells: [], names: [] };
  ok = store_load(b, \"{name}.store\");
  if !ok { println(\"{name} load=false\"); return; }
  c = b.cells[\"cell-299\"];
  println(\"{name} load=true tail=[{b.names[len(b.names) - 1]}] last={c.ce_x},{c.ce_y}\");
}

fn main() {
  read(\"whole\");
  for k in 0..6 { read(\"cut{k}\"); }
}
";

/// loft#2200 — a whole-image load adopted a file cut short: the signature and a 16-byte floor
/// were all it checked, so a cut inside the last word read zero bytes as text (`load=true`,
/// `name numb` for `name number 299`), one inside a block read a byte the file never held
/// (`name num5`), and a deeper cut panicked on the first read.  `(S-Frame)` defines an image as
/// a chain that tiles the file, so each is now refused with its reason, on every backend.
///
/// The cuts are chosen off the image's own chain, so a layout change cannot move one: two
/// inside the last word, one a word into the last block, one a word into a block past the
/// middle, and two exactly ON a block boundary — the start of the last block and of that middle
/// one — where the shorter chain still tiles the file and only the length the image records
/// (bytes 4..8, `(S-Frame)`) tells it is not whole.
#[test]
fn a_store_image_cut_short_is_refused_on_every_backend() {
    let dir = scratch("store_load_cut_short");
    let write = dir.join("write.loft");
    fa::write(&write, format!("{CUT_TYPES}{CUT_WRITE}")).expect("write the writer");
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(&write)
        .output()
        .expect("run the writer");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let whole = fa::read(dir.join("whole.store")).expect("the image");
    assert_eq!(whole.len() % 8, 0, "an image is whole words");
    // The block chain: each block opens with its i32 size in words (S-Frame).
    let mut starts = Vec::new();
    let mut pos = 1usize;
    while pos * 8 < whole.len() {
        starts.push(pos);
        let w = &whole[pos * 8..pos * 8 + 4];
        pos += i32::from_ne_bytes([w[0], w[1], w[2], w[3]]).unsigned_abs() as usize;
    }
    let mid = starts
        .windows(2)
        .find(|w| w[0] * 8 >= whole.len() / 2 && w[1] - w[0] >= 2)
        .map(|w| w[0])
        .expect("a block of two words or more past the middle");
    let last = *starts.last().expect("a chain");
    assert!(
        whole.len() / 8 - last >= 2,
        "the last block is longer than a word"
    );
    assert_eq!(
        loft::store::image_recorded_words(&whole),
        Some(u32::try_from(whole.len() / 8).expect("a store's words fit u32")),
        "a written image records its own length"
    );
    let cuts = [
        whole.len() - 1,
        whole.len() - 7,
        whole.len() - 8,
        mid * 8 + 8,
        last * 8,
        mid * 8,
    ];
    let side = fa::read(dir.join("whole.store.dschema")).expect("the sidecar");
    for (k, &n) in cuts.iter().enumerate() {
        fa::write(dir.join(format!("cut{k}.store")), &whole[..n]).expect("write a cut");
        fa::write(dir.join(format!("cut{k}.store.dschema")), &side).expect("its sidecar");
    }
    let read = dir.join("read.loft");
    fa::write(&read, format!("{CUT_TYPES}{CUT_READ}")).expect("write the reader");
    let c_ok = loft::platform::process::harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    let backends: &[&str] = if c_ok {
        &["--interpret", "--native", "--native-c"]
    } else {
        &["--interpret", "--native"]
    };
    for backend in backends {
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(&read)
            .output()
            .expect("run the reader");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "{backend}: {stdout}\n{stderr}");
        assert_eq!(
            stdout.lines().collect::<Vec<_>>(),
            [
                "whole load=true tail=[name number 299] last=897,2093",
                "cut0 load=false",
                "cut1 load=false",
                "cut2 load=false",
                "cut3 load=false",
                "cut4 load=false",
                "cut5 load=false",
            ],
            "{backend}: {stderr}"
        );
        // Every cut changes the file's length, which the image records, so each says so.
        assert_eq!(
            stderr.matches("the image was cut short").count(),
            6,
            "{backend}: each cut says why: {stderr}"
        );
    }
}

/// @PLN97 arc G Phase 3a — `store_load_key` fetches ONLY the requested integer
/// key (the working set) from a persisted hash image, not the whole file. On
/// both backends: the loaded key is present with the right value, un-requested
/// keys are absent, and `len == 1` (proving the load is bounded, not a full
/// store_load). The read touches only the pages the lookup needs.
#[test]
fn store_load_key_loads_only_the_requested_key_both_backends() {
    let dir = scratch("store_load_key_phase3a");
    let path = dir.join("k.store");

    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write keys=7,13,42"), "write: {out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_mode_backend(backend, &load_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(
            out.contains("loadkey keys=13"),
            "{backend}: only key 13 must be present (bounded working set): {out:?}"
        );
        assert!(
            out.contains("loadkey lookup h[13]=1300"),
            "{backend}: the loaded key's value must be correct: {out:?}"
        );
        assert!(
            out.contains("loadkey len=1"),
            "{backend}: exactly one entry loaded, not the whole store: {out:?}"
        );
        assert!(
            out.contains("loadkey verify=true"),
            "{backend}: the loaded store must be structurally sound (store_verify): {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 3a (plural) — `store_load_keys` fetches a SUBSET of keys
/// ({7, 42}) in one call, returns the count found (2), and leaves un-requested
/// keys (13) absent. Both backends.
#[test]
fn store_load_keys_loads_the_requested_subset_both_backends() {
    let dir = scratch("store_load_keys_phase3a");
    let path = dir.join("ks.store");

    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_mode_backend(backend, &load_script(), &path, "loadkeys");
        assert_eq!(code, 0, "{backend} loadkeys exit: {out:?}");
        assert!(
            out.contains("loadkeys got=2"),
            "{backend}: both requested keys found: {out:?}"
        );
        assert!(
            out.contains("h7=700 h42=4200 h13=null"),
            "{backend}: subset values correct + 13 absent: {out:?}"
        );
        assert!(
            out.contains("loadkeys len=2"),
            "{backend}: exactly the subset loaded: {out:?}"
        );
    }
}

/// loft#730 — an image does not keep the slack its vectors grew to.
///
/// `Store::resize` leaves an in-place growth at 7/4 of the size that triggered
/// it and never gives it back, so an image of the live bytes carries it (2.07×
/// the content at this shape). `store_persist_copy` writes the collection
/// rebuilt, so the file must sit near the content.
///
/// Sizes are asserted with the digest, because a file that got smaller by LOSING
/// data satisfies every size assertion anyone would write.
#[test]
fn an_image_sheds_the_slack_its_vectors_grew_to() {
    let dir = scratch("store_compact_slack_730");
    let path = dir.join("s.store");
    let script = workspace_root().join("tests/scripts/store_compact_slack_730.loft");

    let field = |out: &str, key: &str| -> String {
        out.split_whitespace()
            .find_map(|t| t.strip_prefix(key).map(str::to_string))
            .unwrap_or_default()
    };

    for backend in ["--interpret", "--native-rust"] {
        let _ = fa::remove_file(&path);
        let _ = fa::remove_file(fa::with_extension(&path, "store.dschema"));
        // The fixture MANUFACTURES slack through in-place vector growth, which the exact
        // free tree gives it: the default lazy phase (`@FR-H-LazyFree`) writes the same data
        // into a file a third smaller (1.37× content against 2.07×), below the slack floor
        // this test needs to measure the shedding at all.  The shedding step is not pinned.
        let (w, cw) = run_mode_backend_env(
            backend,
            &script,
            &path,
            "write",
            &[("LOFT_NO_LAZY_FREE", "1")],
        );
        assert_eq!(cw, 0, "{backend} write exit: {w:?}");
        let (r, cr) = run_mode_backend(backend, &script, &path, "reload");
        assert_eq!(cr, 0, "{backend} reload exit: {r:?}");

        assert_eq!(
            field(&w, "digest="),
            field(&r, "digest="),
            "{backend}: the image must hold the data it was written from:\n  {w}\n  {r}"
        );
        let content: f64 = field(&w, "content=").parse().expect("content");
        let written: f64 = field(&w, "file=").parse().expect("written");
        assert!(
            written / content < 1.3,
            "{backend}: an image should be its content plus the format's overhead, \
             not the slack the vectors grew to: content={content} written={written}"
        );
    }
}

/// loft#729 — the loader reads a record's body in ONE fetch, and that must not
/// change a single loaded byte.
///
/// It used to read word by word: one `resolve` per 4 bytes, which on a real
/// 162 MB store made 1 073 065 requests for one 42-key viewport and 101 of every
/// other size. `LOFT_LOADER_WORDWISE` keeps that path so the two can be compared
/// in ONE binary — a build-to-build comparison would fold in every other change,
/// and the content is what must agree, so the content is what this reads.
#[test]
fn a_bulk_record_read_loads_exactly_what_the_word_at_a_time_read_did() {
    let dir = scratch("store_load_bulk_729");
    let path = dir.join("vs.store");

    let (out_w, code_w) = run_mode(&vecstruct_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (bulk, c1) = run_mode_backend(backend, &vecstruct_script(), &path, "loadkey");
        assert_eq!(c1, 0, "{backend} bulk exit: {bulk:?}");
        let wordwise = {
            let out = loft::platform::process::harness_command(loft_bin())
                .arg(backend)
                .arg(vecstruct_script())
                .env("LOFT_PERSIST_TEST_PATH", &path)
                .env("LOFT_PERSIST_TEST_MODE", "loadkey")
                .env("LOFT_LOADER_WORDWISE", "1")
                .current_dir(workspace_root())
                .output()
                .expect("failed to invoke loft binary");
            assert!(out.status.success(), "{backend} wordwise exit");
            String::from_utf8_lossy(&out.stdout).into_owned()
        };
        assert_eq!(
            bulk, wordwise,
            "{backend}: the bulk read must load exactly what the word-at-a-time read did"
        );
        // The control: an oracle that reports agreement without reading anything
        // would pass this too. These pin the values, so the comparison is between
        // two CORRECT reads rather than two empty ones.
        assert!(
            bulk.contains("vs e0=10,ten") && bulk.contains("vs e2=30,thirty"),
            "{backend}: relocated vector<struct> elements must read correctly: {bulk:?}"
        );
        assert!(
            bulk.contains("vs verify=true"),
            "{backend}: the loaded heap must be sound: {bulk:?}"
        );
    }
}

/// @PLN97 arc G Phase 3b.2 — a hash whose entry has a `text` field is now
/// partially loadable: `store_load_key` relocates the source string sub-record
/// into the local store and repoints the field. The loaded entry reads the right
/// string, un-requested keys are absent, and the result is structurally sound
/// (`store_verify`). Both backends.
#[test]
fn store_load_key_relocates_a_text_field_both_backends() {
    let dir = scratch("store_load_text");
    let path = dir.join("text.store");

    let (out_w, code_w) = run_mode(&text_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write ok"), "write: {out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_mode_backend(backend, &text_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(out.contains("text loadkey=true"), "{backend}: {out:?}");
        assert!(
            out.contains("text name13=thirteen-longer-string-spanning"),
            "{backend}: the relocated text must read correctly: {out:?}"
        );
        assert!(
            out.contains("text name7=ABSENT"),
            "{backend}: un-requested key must be absent: {out:?}"
        );
        assert!(out.contains("text len=1"), "{backend}: {out:?}");
        assert!(
            out.contains("text verify=true"),
            "{backend}: the relocated heap must be sound (no dangling text ptr): {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 3b.3 — a hash whose entry has a `vector<integer>` field is
/// now partially loadable: `store_load_key` relocates the vector's flat inner
/// record. The loaded entry reads the right elements and is structurally sound.
/// Both backends.
#[test]
fn store_load_key_relocates_a_vector_field_both_backends() {
    let dir = scratch("store_load_vec");
    let path = dir.join("vec.store");

    let (out_w, code_w) = run_mode(&vec_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write ok"), "write: {out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_mode_backend(backend, &vec_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(out.contains("vec loadkey=true"), "{backend}: {out:?}");
        assert!(
            out.contains("vec tags_len=3"),
            "{backend}: the relocated vector length must be right: {out:?}"
        );
        assert!(
            out.contains("vec tags=10,20,30"),
            "{backend}: the relocated vector elements must be right: {out:?}"
        );
        assert!(out.contains("vec len=1"), "{backend}: {out:?}");
        assert!(
            out.contains("vec verify=true"),
            "{backend}: the relocated heap must be sound: {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 3b.4 — a hash whose entry has an INLINE nested struct field
/// (`sub: Inner`, itself holding a text) is now partially loadable: `load_one`
/// relocates the nested heap fields at their NESTED offsets. The loaded entry
/// reads the nested scalar and text, and is structurally sound. Both backends.
#[test]
fn store_load_key_relocates_a_nested_struct_both_backends() {
    let dir = scratch("store_load_nested");
    let path = dir.join("nest.store");

    let (out_w, code_w) = run_mode(&nested_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write ok"), "write: {out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_mode_backend(backend, &nested_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(out.contains("nest loadkey=true"), "{backend}: {out:?}");
        assert!(
            out.contains("nest sub.a=42"),
            "{backend}: nested scalar must relocate: {out:?}"
        );
        assert!(
            out.contains("nest sub.b=deep-nested-string"),
            "{backend}: nested text must relocate at its nested offset: {out:?}"
        );
        assert!(out.contains("nest len=1"), "{backend}: {out:?}");
        assert!(
            out.contains("nest verify=true"),
            "{backend}: the relocated heap with a nested text must be sound: {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 3b.4b — a hash whose entry has a `vector<struct>` field
/// (elements carrying their own text) is now partially loadable: `load_one`
/// copies the inner vector record and relocates EACH element's heap fields. Every
/// element reads correctly and the result is sound. Both backends.
#[test]
fn store_load_key_relocates_a_vector_of_struct_both_backends() {
    let dir = scratch("store_load_vecstruct");
    let path = dir.join("vs.store");

    let (out_w, code_w) = run_mode(&vecstruct_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write ok"), "write: {out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_mode_backend(backend, &vecstruct_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(out.contains("vs loadkey=true"), "{backend}: {out:?}");
        assert!(out.contains("vs items_len=3"), "{backend}: {out:?}");
        assert!(
            out.contains("vs e0=10,ten"),
            "{backend}: element 0 (scalar + relocated text): {out:?}"
        );
        assert!(
            out.contains("vs e2=30,thirty"),
            "{backend}: element 2 (scalar + relocated text): {out:?}"
        );
        assert!(out.contains("vs len=1"), "{backend}: {out:?}");
        assert!(
            out.contains("vs verify=true"),
            "{backend}: the relocated vector<struct> heap must be sound: {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 3b (safe-refusal, ongoing) — a `vector<text>` field
/// (elements that are string pointers) is not handled (copying the inner record
/// leaves the element strings dangling), so it must still be REFUSED. The
/// refused-empty result is structurally sound. Both backends.
#[test]
fn store_load_key_refuses_a_vector_of_text_field_both_backends() {
    let dir = scratch("store_load_vectext_refuse");
    let path = dir.join("vt.store");

    let (out_w, code_w) = run_mode(&vectext_refuse_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write ok"), "write: {out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_mode_backend(backend, &vectext_refuse_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(
            out.contains("vt loadkey=false"),
            "{backend}: a vector<text> entry must be REFUSED: {out:?}"
        );
        assert!(out.contains("vt len=0"), "{backend}: {out:?}");
        assert!(
            out.contains("vt verify=true"),
            "{backend}: the refused-empty store must be sound: {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 5 — the REMOTE fetch. `store_load_key` over an `http://`
/// URL pulls only the ranges the lookup touches from a `Range`-capable server
/// (the #517 shape), producing the SAME bounded, structurally-sound working set
/// as the local-file path — the generic `PageProvider` seam leaves the
/// traversal and relocating copy unchanged. This is the "fetch" in the partial
/// store fetcher. Both backends.
#[test]
fn store_load_key_over_http_range() {
    let dir = scratch("store_load_http");
    let path = dir.join("world.store");

    // Write the image locally, then serve its bytes over HTTP Range.
    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    assert!(out_w.contains("write keys=7,13,42"), "{out_w:?}");
    // Serve the real `.dschema` beside the store so the 3b.5 layout gate does a
    // genuine remote Match (not a fall-through to the absent-sidecar path).
    let sidecar = fa::read(format!("{}.dschema", path.display())).ok();
    let url = serve_ranges(fa::read(&path).unwrap(), sidecar);

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_mode_backend(backend, &load_script(), Path::new(&url), "loadkey");
        assert_eq!(code, 0, "{backend} http loadkey exit: {out:?}");
        assert!(
            out.contains("loadkey keys=13"),
            "{backend}: remote fetch must load key 13: {out:?}"
        );
        assert!(
            out.contains("loadkey lookup h[13]=1300"),
            "{backend}: remote fetch must read the right value: {out:?}"
        );
        assert!(
            out.contains("loadkey len=1"),
            "{backend}: remote fetch is bounded to the working set: {out:?}"
        );
        assert!(
            out.contains("loadkey verify=true"),
            "{backend}: the working set fetched over http must be sound: {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 3b.6 — a TEXT-keyed hash (`hash<Place[name:text]>`) is
/// partially loadable by string key: `store_load_key_text` finds by hashing the
/// Content::Str and comparing the entry's text key over the reader. The loaded
/// entry reads correctly, un-requested keys are absent, and it's sound. Both
/// backends.
#[test]
fn store_load_key_text_both_backends() {
    let dir = scratch("store_load_textkey");
    let path = dir.join("places.store");

    let (out_w, code_w) = run_mode(&textkey_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    assert!(out_w.contains("write ok"), "{out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_mode_backend(backend, &textkey_script(), &path, "loadkey");
        assert_eq!(code, 0, "{backend} loadkey exit: {out:?}");
        assert!(out.contains("tk loadkey=true"), "{backend}: {out:?}");
        assert!(
            out.contains("tk berlin=3700000"),
            "{backend}: the text-keyed entry must load: {out:?}"
        );
        assert!(
            out.contains("tk amsterdam=-1"),
            "{backend}: un-requested key absent: {out:?}"
        );
        assert!(out.contains("tk len=1"), "{backend}: {out:?}");
        assert!(
            out.contains("tk verify=true"),
            "{backend}: text-keyed working set must be sound: {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 4 / 3b.7 — `store_load_range` over a `sorted<Tile[id]>`
/// fetches the [13,42] range (13,20,42) in key order, relocates each element's
/// text, leaves out-of-range keys absent, and produces a sound sorted collection.
/// Both backends. This is routing's tile-window fetch.
#[test]
fn store_load_range_over_sorted_both_backends() {
    let dir = scratch("store_load_range");
    let path = dir.join("tiles.store");

    let (out_w, code_w) = run_mode(&range_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    assert!(out_w.contains("write ok"), "{out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_mode_backend(backend, &range_script(), &path, "range");
        assert_eq!(code, 0, "{backend} range exit: {out:?}");
        assert!(out.contains("rng loaded=3"), "{backend}: {out:?}");
        assert!(
            out.contains("rng keys=13,20,42"),
            "{backend}: the range must load in key order: {out:?}"
        );
        assert!(
            out.contains("rng name20=twenty"),
            "{backend}: element text must relocate: {out:?}"
        );
        assert!(
            out.contains("rng g7=absent"),
            "{backend}: out-of-range key absent: {out:?}"
        );
        assert!(out.contains("rng len=3"), "{backend}: {out:?}");
        assert!(
            out.contains("rng verify=true"),
            "{backend}: the loaded sorted collection must be sound: {out:?}"
        );
    }
}

/// @PLN134 — a persisted TRIE read a page at a time: one text key by descent
/// (`store_load_key_text`) and a prefix run by a bounded walk
/// (`store_load_prefix`), on both backends.
///
/// The cells that carry weight are the ones a walk answering *something* would
/// still fail: `kerf` shares `ker` with the run and must be left OUT, `kerx` sorts
/// INSIDE the key range and must answer nothing (a seek lands on `lonneker`, so a
/// missing stop-check reports that instead), and the cap must take the first two
/// IN KEY ORDER. `store_verify` then says the collection the loader built is a
/// sound trie — a working-set copy that linked its records wrongly would still
/// print the right words.
#[test]
fn store_load_prefix_over_trie_both_backends() {
    let dir = scratch("store_load_prefix");
    let path = dir.join("words.store");

    let (out_w, code_w) = run_mode(&prefix_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    assert!(out_w.contains("write ok"), "{out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        // One key, by a root→leaf descent — and a miss that stays a miss.
        let (out, code) = run_mode_backend(backend, &prefix_script(), &path, "one");
        assert_eq!(code, 0, "{backend} one exit: {out:?}");
        assert!(out.contains("one ok=true"), "{backend}: {out:?}");
        assert!(
            out.contains("one hits=5"),
            "{backend}: the entry's own fields must arrive: {out:?}"
        );
        assert!(out.contains("one len=1"), "{backend}: {out:?}");
        assert!(
            out.contains("one miss=false") && out.contains("one miss_len=0"),
            "{backend}: `kerks` extends `kerk` and prefixes `kerkstraat`, and is \
             neither — a descent lands on SOME leaf, so this is where a missing key \
             comparison shows: {out:?}"
        );

        // The prefix run.
        let (out, code) = run_mode_backend(backend, &prefix_script(), &path, "prefix");
        assert_eq!(code, 0, "{backend} prefix exit: {out:?}");
        assert!(out.contains("pre loaded=4"), "{backend}: {out:?}");
        assert!(
            out.contains("pre keys=kerk,kerklaan,kerkstraat,kerkweg"),
            "{backend}: the prefix itself is included and the run is in key order: {out:?}"
        );
        assert!(
            out.contains("pre hits=9"),
            "{backend}: every element's fields relocate, not just the first: {out:?}"
        );
        assert!(
            out.contains("pre absent=true"),
            "{backend}: `kerf` shares `ker` but does not extend `kerk`: {out:?}"
        );
        assert!(
            out.contains("pre verify=true"),
            "{backend}: the trie the loader BUILT must be sound: {out:?}"
        );

        // The cap, and the three empty answers.
        let (out, code) = run_mode_backend(backend, &prefix_script(), &path, "cap");
        assert_eq!(code, 0, "{backend} cap exit: {out:?}");
        assert!(out.contains("cap loaded=2"), "{backend}: {out:?}");
        assert!(
            out.contains("cap keys=kerk,kerklaan"),
            "{backend}: the cap takes the FIRST n in key order: {out:?}"
        );
        assert!(
            out.contains("cap none=0"),
            "{backend}: `kerx` sorts inside the range and extends nothing: {out:?}"
        );
        assert!(out.contains("cap past=0"), "{backend}: {out:?}");
        assert!(
            out.contains("cap all=6"),
            "{backend}: the empty prefix is every record: {out:?}"
        );

        // loft#802's refusal narrows: a trie image can be PAGED now, so the bind is
        // accepted and a lookup that misses consults the source.
        let (out, code) = run_mode_backend(backend, &prefix_script(), &path, "bind");
        assert_eq!(code, 0, "{backend} bind exit: {out:?}");
        assert!(
            out.contains("bind ok=true"),
            "{backend}: a trie bound to a `.store` image is servable now: {out:?}"
        );
        assert!(
            out.contains("bind hits=3"),
            "{backend}: the lazy fetch must reach the record: {out:?}"
        );
        assert!(
            out.contains("bind quiet=true") && out.contains("bind faults=0"),
            "{backend}: a healthy binding says nothing: {out:?}"
        );
        assert!(
            out.contains("bind absent=true") && out.contains("still_quiet=true"),
            "{backend}: a genuine absence must not start speaking: {out:?}"
        );
    }
}

/// @PLN97 3b.5 — the layout-identity gate (local file). Persisting writes a
/// `.dschema` sidecar; a working-set load whose Tile layout MATCHES the
/// persisted one proceeds, but a load with an EXTRA field is REFUSED before
/// reading any foreign-layout bytes (leaving the collection empty, not
/// corrupt). Both backends. The sound gate behind trusting a partial store.
#[test]
fn layout_gate_rejects_changed_struct_both_backends() {
    let dir = scratch("layout_gate");
    let path = dir.join("tiles.store");

    let (out_w, code_w) = run_mode(&gate_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    assert!(out_w.contains("write ok"), "{out_w:?}");
    assert!(
        fa::exists(Path::new(&format!("{}.dschema", path.display()))),
        "persist must write a .dschema sidecar beside the store"
    );

    for backend in ["--interpret", "--native-rust"] {
        // MATCHING layout — the load proceeds.
        let (out, code) = run_mode_backend(backend, &gate_script(), &path, "load");
        assert_eq!(code, 0, "{backend} match exit: {out:?}");
        assert!(
            out.contains("gate ok=true"),
            "{backend}: a matching layout must load: {out:?}"
        );
        assert!(out.contains("gate name=forty-two"), "{backend}: {out:?}");
        assert!(out.contains("gate len=1"), "{backend}: {out:?}");

        // CHANGED layout (extra field) — the gate refuses.
        let (out, code) = run_mode_backend(backend, &gate_changed_script(), &path, "load");
        assert_eq!(code, 0, "{backend} mismatch exit: {out:?}");
        assert!(
            out.contains("changed ok=false"),
            "{backend}: a changed layout MUST be refused, not read raw: {out:?}"
        );
        assert!(
            out.contains("changed len=0"),
            "{backend}: a refused load leaves the collection empty: {out:?}"
        );
    }
}

/// loft#700 — the layout gate on the WHOLE-IMAGE loader, `store_load` (`@FR-L-Sound`:
/// reject or convert, never read raw).
///
/// A raw read through the slot's type took a struct that had GROWN a field at the new,
/// larger stride: `len()` returned wild values and iterating read arbitrary memory. The
/// grown Tile now CONVERTS (@PLN196): both records arrive with the added field at its
/// default.  A change the converter cannot carry (`name` from text to integer) is
/// refused and leaves the collection empty.
///
/// The matching half keeps the check honest: a gate that refused everything would pass
/// the refusal assertion on its own, and the converted VALUES are what a raw read at the
/// wrong stride cannot produce.
#[test]
fn store_load_converts_or_refuses_a_changed_layout_both_backends() {
    let dir = scratch("layout_gate_whole");
    let path = dir.join("tiles.store");

    let (out_w, code_w) = run_mode(&gate_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        // MATCHING layout — the whole-image load proceeds and yields the data.
        let (out, code) = run_mode_backend(backend, &gate_script(), &path, "whole");
        assert_eq!(code, 0, "{backend} match exit: {out:?}");
        assert!(
            out.contains("gate whole ok=true"),
            "{backend}: an unchanged layout must still load whole: {out:?}"
        );
        assert!(out.contains("gate whole len=2"), "{backend}: {out:?}");
        assert!(
            out.contains("gate whole name=forty-two"),
            "{backend}: {out:?}"
        );

        // GROWN layout (extra field, default 7) — converted, never read at the wrong stride.
        let (out, code) = run_mode_backend(backend, &gate_changed_script(), &path, "whole");
        assert_eq!(code, 0, "{backend} mismatch exit: {out:?}");
        assert!(
            out.contains("changed whole ok=true")
                && out.contains("changed whole len=2")
                && out.contains("changed whole tile=42/forty-two/7"),
            "{backend}: a grown layout converts, every record with its default: {out:?}"
        );

        // A change the converter cannot carry — refused, the collection empty.
        let (out, code) = run_mode_backend(backend, &gate_refused_script(), &path, "whole");
        assert_eq!(code, 0, "{backend} refused exit: {out:?}");
        assert!(
            out.contains("refused whole ok=false") && out.contains("refused whole len=0"),
            "{backend}: an unconvertible layout MUST be refused, not read raw: {out:?}"
        );
    }
}

/// `store_load_url` and `store_load_url_trusted` adopt a fetched image through the slot's
/// type, so they pass the layout gate too (`@FR-L-Sound`, loft#1562): a grown layout
/// converts (@PLN196), an unconvertible one is refused.  The sidecar is
/// `<url>.dschema`, read over the same transport as the image: beside the file for
/// `file://`, and from the server over HTTP.  Both loaders, both transports and both
/// backends are covered, each with its matching half, so the gate cannot pass by
/// refusing everything.
#[cfg(feature = "registry")]
#[test]
fn store_load_url_converts_or_refuses_a_changed_layout_both_backends() {
    let dir = scratch("layout_gate_url");
    let path = dir.join("tiles.store");

    let (out_w, code_w) = run_mode(&gate_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    let bytes = fa::read(&path).unwrap();
    let sidecar = fa::read(format!("{}.dschema", path.display())).unwrap();
    let sha = loft::integrity::sha256_hex(&bytes);
    let file_url = format!("file://{}", path.display());
    let http_url = serve_ranges(bytes, Some(sidecar));

    let run = |backend: &str, script: &Path, url: &str, pin: &str| -> String {
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(script)
            .env("LOFT_PERSIST_TEST_PATH", &path)
            .env("LOFT_PERSIST_TEST_MODE", "url")
            .env("LOFT_PERSIST_TEST_URL", url)
            .env("LOFT_PERSIST_TEST_SHA", pin)
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        assert!(
            out.status.success(),
            "{backend} {url} exit: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    };

    for backend in ["--interpret", "--native-rust"] {
        for url in [&file_url, &http_url] {
            for pin in ["trusted", sha.as_str()] {
                let out = run(backend, &gate_script(), url, pin);
                assert!(
                    out.contains("gate url ok=true")
                        && out.contains("gate url len=2")
                        && out.contains("gate url name=forty-two"),
                    "{backend} {url} pin={pin}: an unchanged layout must still load: {out:?}"
                );
                let out = run(backend, &gate_changed_script(), url, pin);
                assert!(
                    out.contains("changed url ok=true")
                        && out.contains("changed url len=2")
                        && out.contains("changed url tile=42/forty-two/7"),
                    "{backend} {url} pin={pin}: a grown layout converts: {out:?}"
                );
                let out = run(backend, &gate_refused_script(), url, pin);
                assert!(
                    out.contains("refused url ok=false") && out.contains("refused url len=0"),
                    "{backend} {url} pin={pin}: an unconvertible layout MUST be refused: {out:?}"
                );
            }
        }
    }
}

/// @PLN97 3b.5 — the layout-identity gate over HTTP: the remote loader fetches
/// `<url>.dschema` and rejects a mismatched layout, never range-reading foreign
/// bytes across the network (the safety gate for a REMOTE store read, #522).
/// A matching layout still loads. Both backends.
#[test]
fn layout_gate_over_http_rejects_changed_struct() {
    let dir = scratch("layout_gate_http");
    let path = dir.join("tiles.store");

    let (out_w, code_w) = run_mode(&gate_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");
    let sidecar = fa::read(format!("{}.dschema", path.display())).ok();
    assert!(sidecar.is_some(), "persist must write a .dschema sidecar");
    let url = serve_ranges(fa::read(&path).unwrap(), sidecar);

    for backend in ["--interpret", "--native-rust"] {
        // MATCHING layout over http — the remote gate does a real Match.
        let (out, code) = run_mode_backend(backend, &gate_script(), Path::new(&url), "load");
        assert_eq!(code, 0, "{backend} http match exit: {out:?}");
        assert!(
            out.contains("gate ok=true"),
            "{backend}: a matching remote layout must load: {out:?}"
        );

        // CHANGED layout over http — the remote gate refuses.
        let (out, code) =
            run_mode_backend(backend, &gate_changed_script(), Path::new(&url), "load");
        assert_eq!(code, 0, "{backend} http mismatch exit: {out:?}");
        assert!(
            out.contains("changed ok=false"),
            "{backend}: a changed layout MUST be refused over http: {out:?}"
        );
        assert!(out.contains("changed len=0"), "{backend}: {out:?}");
    }
}

/// @PLN97 arc G Phase 0 — `store_load_url` fetches a WHOLE persisted store IMAGE
/// from a URL and adopts it ONLY after its SHA-256 matches the caller-pinned
/// digest (the registry's fetch→verify→trust discipline bridged onto the store
/// loader).  A correct hash loads every key + is structurally sound; a WRONG hash
/// is REFUSED (returns false, adopts nothing — the collection stays empty).  Both
/// backends.  A `file://` URL is used so the test needs no network (the loader's
/// `http_get_bytes` treats `file://` the same as an HTTP GET).
#[cfg(feature = "registry")]
#[test]
fn store_load_url_verifies_sha_before_adopting_both_backends() {
    use sha2::{Digest, Sha256};
    use std::fmt::Write as _;

    let dir = scratch("store_load_url_phase0");
    let path = dir.join("world.store");

    // Write the image once, then compute its SHA-256 in the harness.
    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write keys=7,13,42"), "write: {out_w:?}");

    let bytes = fa::read(&path).unwrap();
    let sha = {
        let mut h = Sha256::new();
        h.update(&bytes);
        let mut s = String::with_capacity(64);
        for b in h.finalize() {
            let _ = write!(s, "{b:02x}");
        }
        s
    };
    let url = format!("file://{}", path.display());
    let wrong = "0".repeat(64);

    let run = |backend: &str, sha_arg: &str| -> (String, i32) {
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(load_script())
            .env("LOFT_PERSIST_TEST_PATH", &path)
            .env("LOFT_PERSIST_TEST_MODE", "loadurl")
            .env("LOFT_PERSIST_TEST_URL", &url)
            .env("LOFT_PERSIST_TEST_SHA", sha_arg)
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        if !out.status.success() {
            eprintln!(
                "{backend} stderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    };

    for backend in ["--interpret", "--native-rust"] {
        // Correct hash — the load succeeds, all keys present, heap sound.
        let (out, code) = run(backend, &sha);
        assert_eq!(code, 0, "{backend} verified load exit: {out:?}");
        assert!(
            out.contains("loadurl ok=true"),
            "{backend}: a SHA-matched image must load: {out:?}"
        );
        assert!(
            out.contains("loadurl keys=7,13,42"),
            "{backend}: all keys must be present after a verified load: {out:?}"
        );
        assert!(
            out.contains("loadurl lookup h[13]=1300"),
            "{backend}: key lookup must read the right value: {out:?}"
        );
        assert!(
            out.contains("loadurl verify=true"),
            "{backend}: the adopted store must be structurally sound: {out:?}"
        );

        // Wrong hash — REFUSED before adopting: nothing loaded.
        let (out, code) = run(backend, &wrong);
        assert_eq!(code, 0, "{backend} tampered load exit: {out:?}");
        assert!(
            out.contains("loadurl ok=false"),
            "{backend}: a SHA MISMATCH must refuse the load: {out:?}"
        );
        assert!(
            out.contains("loadurl lookup h[13]=null"),
            "{backend}: a refused load must adopt NOTHING (h stays empty): {out:?}"
        );
    }
}

/// @PLN97 arc G Phase 2 — `store_load_untrusted` reads a local store IMAGE that
/// may be untrusted and adopts it ONLY after `validate_structure` passes: a
/// well-formed image loads every key + is sound; a garbage file is rejected
/// (`false`) cleanly — never a hang or a misread. Both backends.
#[test]
fn store_load_untrusted_validates_before_adopting_both_backends() {
    let dir = scratch("store_load_untrusted");
    let path = dir.join("u.store");

    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write keys=7,13,42"), "write: {out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_mode_backend(backend, &load_script(), &path, "loaduntrusted");
        assert_eq!(code, 0, "{backend} loaduntrusted exit: {out:?}");
        assert!(
            out.contains("untrusted keys=7,13,42"),
            "{backend}: a valid image must load all keys: {out:?}"
        );
        assert!(
            out.contains("untrusted lookup h[13]=1300"),
            "{backend}: key lookup must be correct: {out:?}"
        );
        assert!(
            out.contains("untrusted verify=true"),
            "{backend}: the adopted store must be structurally sound: {out:?}"
        );
    }

    // A garbage file must reject cleanly (false), not hang or misread.
    let garbage = dir.join("garbage.store");
    fa::write(
        &garbage,
        b"not a loft store image, just junk bytes right here!!",
    )
    .unwrap();
    let (out, code) = run_mode(&load_script(), &garbage, "loaduntrusted");
    assert_eq!(code, 0, "garbage reject should be graceful: {out:?}");
    assert!(
        out.contains("FAIL untrusted-load-returned-false"),
        "a garbage file must reject cleanly (store_load_untrusted=false): {out:?}"
    );
}

/// @PLN97 arc G Phase 0 — `store_load_url_trusted` fetches a whole store IMAGE
/// over HTTP from a TRUSTED source (no SHA pin) and adopts it — the instant read,
/// still structurally validated. Both backends, over a real HTTP server.
#[test]
fn store_load_url_trusted_fetches_over_http_both_backends() {
    let dir = scratch("store_load_url_trusted");
    let path = dir.join("t.store");

    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("write keys=7,13,42"), "write: {out_w:?}");

    // Serve the store and its sidecar over HTTP (a plain GET → 200 whole file): an image is
    // read only beside its sidecar (`@FR-S-Gate`).
    let sidecar = fa::read(format!("{}.dschema", path.display())).ok();
    let url = serve_ranges(fa::read(&path).unwrap(), sidecar);

    for backend in ["--interpret", "--native-rust"] {
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(load_script())
            .env("LOFT_PERSIST_TEST_PATH", &path)
            .env("LOFT_PERSIST_TEST_MODE", "loadurltrusted")
            .env("LOFT_PERSIST_TEST_URL", &url)
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let so = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} exit: {so} / {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            so.contains("urltrusted keys=7,13,42"),
            "{backend}: a trusted HTTP fetch must load all keys: {so}"
        );
        assert!(
            so.contains("urltrusted lookup h[13]=1300"),
            "{backend}: key lookup must be correct: {so}"
        );
        assert!(
            so.contains("urltrusted verify=true"),
            "{backend}: the fetched store must be structurally sound: {so}"
        );
    }
}

#[test]
fn fresh_returns_true_and_file_appears() {
    let dir = scratch("fresh_returns_true_and_file_appears");
    let path = dir.join("only_fresh.store");

    let (out, code) = run_smoke(&path, "fresh");
    assert_eq!(code, 0, "{out:?}");
    assert!(!out.contains("FAIL"), "fresh should not fail; got: {out:?}");
    assert!(fa::exists(&path), "the write should create the file");
}

/// `store_load` of a path that does not exist answers `false` and writes nothing.
#[test]
fn reload_on_missing_file_refuses_and_writes_nothing() {
    let dir = scratch("reload_on_missing_file");
    let path = dir.join("never_existed.store");
    assert!(!fa::exists(&path));

    let (out, code) = run_smoke(&path, "reload");
    assert_eq!(code, 0, "{out:?}");
    assert!(
        out.contains("FAIL reload-load-returned-false"),
        "a load of a missing file must answer false: {out:?}"
    );
    assert!(!fa::exists(&path), "a load must not create the file");
}

fn handoff_residency_script() -> PathBuf {
    workspace_root().join("tests/scripts/store_handoff_residency.loft")
}

/// Assert every text-residency cell a reader task should have recovered from
/// the handoff file. `label` names the read path so a failure says which one.
fn assert_handoff_cells(out: &str, label: &str) {
    for want in [
        "h short=zeta-42 len=7",
        "h long=300 match=true",
        "h built=R-0#1#2#3#4#",
        "h utf8=héllo-π len=7",
        "h words=3 w0=kappa w1=lambda-9 w2=mu",
        "h nums=4 n3=44",
        "h narrow b=217 w=-31000",
        "h absent=null",
    ] {
        assert!(
            out.contains(want),
            "{label}: missing {want:?} in output: {out:?}"
        );
    }
    assert!(
        !out.contains("h ABSENT"),
        "{label}: entry not found: {out:?}"
    );
}

/// @PLN119 Q1 — a store file is a HANDOFF between tasks: text written by one
/// process is read back, intact, by a SEPARATE one. That is what makes the
/// store usable as a shared mmap arena, so the text shapes are the point —
/// short, over-256 (past the inline-constant threshold), runtime-built,
/// multi-byte, and `vector<text>` (whose elements are string pointers).
///
/// Contrast `store_load_vectext_refuse.loft`: the PAGED loader refuses
/// `vector<text>` because relocating an entry into a DIFFERENT store would
/// dangle each element pointer. The whole-store image is rebuilt with every
/// pointer rewritten into it, so the same field survives here — different
/// paths, both correct.
#[test]
fn handoff_text_residency_survives_a_separate_process() {
    let dir = scratch("handoff_text_residency");
    let path = dir.join("handoff.store");
    let script = handoff_residency_script();

    let (w_out, w_code) = run_mode(&script, &path, "write");
    assert_eq!(w_code, 0, "{w_out:?}");
    assert!(w_out.contains("write ok"), "write failed: {w_out:?}");
    assert!(
        fa::exists(&path),
        "the handoff file must exist after the write task"
    );

    // A FRESH process — only the file crosses.
    let (i_out, i_code) = run_mode(&script, &path, "readimage");
    assert_eq!(i_code, 0, "{i_out:?}");
    assert!(i_out.contains("readimage load=true"), "{i_out:?}");
    assert_handoff_cells(&i_out, "readimage");
}

#[test]
fn handoff_text_residency_reads_on_native_backend() {
    let dir = scratch("handoff_text_residency_native");
    let path = dir.join("handoff.store");
    let script = handoff_residency_script();

    let (w_out, w_code) = run_mode_backend("--interpret", &script, &path, "write");
    assert_eq!(w_code, 0, "{w_out:?}");
    assert!(w_out.contains("write ok"), "write failed: {w_out:?}");

    // Written by the interpreter, read by a natively-compiled process: the
    // image is portable across backends, not just across processes.
    let (n_out, n_code) = run_mode_backend("--native-rust", &script, &path, "readimage");
    assert_eq!(n_code, 0, "{n_out:?}");
    assert!(n_out.contains("readimage load=true"), "{n_out:?}");
    assert_handoff_cells(&n_out, "native readimage");
}

/// `run_mode` that also hands back stderr — the refusal channel the paged
/// loaders warn on.
fn run_mode_with_stderr(script: &Path, path: &Path, mode: &str) -> (String, String, i32) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(script)
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// #632 — a paged load that REFUSES must say so. `store_load_key` and friends
/// report failure as `false` / `0`, which is indistinguishable from "that key is
/// absent", so a silent refusal reads as missing data and hides an unsupported
/// shape. The layout gate already warns for exactly this reason; every other
/// refusal now routes through the same channel.
///
/// #632 part 2 — a HASH declared as a struct field is now paged-loadable. The
/// written image records the WRAPPER STRUCT as its type; the loader descends to the
/// keyed-collection field and loads it. The reported failure ("silently return
/// nothing for a collection declared as a struct field") is gone: the key loads,
/// the value is right, and the copied heap verifies.
#[test]
fn paged_load_accepts_a_hash_declared_as_a_struct_field() {
    let dir = scratch("paged_field_accept");
    let path = dir.join("field.store");
    let script = workspace_root().join("tests/scripts/store_load_field_refusal.loft");

    let (w_out, _, w_code) = run_mode_with_stderr(&script, &path, "write");
    assert_eq!(w_code, 0, "{w_out:?}");
    assert!(w_out.contains("write ok"), "write failed: {w_out:?}");

    let (out, err, code) = run_mode_with_stderr(&script, &path, "loadkey");
    assert_eq!(code, 0, "{out:?}\n{err:?}");
    assert!(
        out.contains("field loadkey=true len=1 n=7717 verify=true"),
        "hash-as-field must load the key with the right value + sound heap: {out:?}"
    );
    assert!(
        !err.contains("refusing"),
        "the hash field form must no longer refuse: {err:?}"
    );
}

/// #632 — a SORTED collection declared as a struct field still refuses (a
/// promoted/linked `ordered` field's elements sit behind an indirection the
/// positional range reader can't follow — @PLN97 arc G). What this pins is that
/// the refusal is AUDIBLE, never a silent `0` indistinguishable from "key absent".
#[test]
fn paged_refusal_on_sorted_field_is_audible() {
    let dir = scratch("paged_sorted_field_refusal");
    let path = dir.join("sfield.store");
    let script = workspace_root().join("tests/scripts/store_load_field_refusal.loft");

    let (w_out, _, w_code) = run_mode_with_stderr(&script, &path, "write_sorted");
    assert_eq!(w_code, 0, "{w_out:?}");
    assert!(w_out.contains("write ok"), "write failed: {w_out:?}");

    let (out, err, code) = run_mode_with_stderr(&script, &path, "loadrange_sorted");
    assert_eq!(code, 0, "{out:?}");
    assert!(out.contains("sorted loadrange=0 len=0"), "{out:?}");
    assert!(
        err.contains("refusing") && err.contains("not an absent key"),
        "a refusal must be reported, not silent; stderr was: {err:?}"
    );
    assert!(
        err.contains("SWrap") && err.contains("annotated local"),
        "the warning must name the wrapper type and the fix; stderr was: {err:?}"
    );
}

// ── loft#710 — the persisted file's size is a fact about its CONTENT ─────────

/// Build the size-probe store and return `(file bytes, digest line)`.
///
/// `seed` fixes `LOFT_HASH_SEED` when given, which is what makes a build
/// byte-reproducible.
fn persist_size(test: &str, n: u32, per: u32, mode: &str, seed: Option<&str>) -> (u64, String) {
    let dir = scratch(test);
    persist_size_at(&dir.join("size.store"), n, per, mode, seed)
}

/// [`persist_size`] against a path that already exists — the `reload` mode needs the
/// image the build modes left, and `scratch` wipes its directory.
fn persist_size_at(path: &Path, n: u32, per: u32, mode: &str, seed: Option<&str>) -> (u64, String) {
    let path = path.to_path_buf();
    let script = workspace_root().join("tests/scripts/store_persist_size_710.loft");
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg("--interpret")
        .arg(&script)
        .env("LOFT_PERSIST_TEST_PATH", &path)
        .env("LOFT_PERSIST_TEST_N", n.to_string())
        .env("LOFT_PERSIST_TEST_PER", per.to_string())
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .current_dir(workspace_root());
    if let Some(s) = seed {
        cmd.env("LOFT_HASH_SEED", s);
    } else {
        cmd.env_remove("LOFT_HASH_SEED");
    }
    let out = cmd.output().expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{mode} failed: {stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(stdout.contains("persist true"), "{mode}: {stdout:?}");
    let bytes = fa::metadata(&path).expect("store file").len();
    (bytes, stdout.trim().to_string())
}

/// [`persist_size`], answering the live digest of the image it wrote (`live_digest_line`).
fn persist_live_digest(test: &str, n: u32, per: u32, mode: &str, seed: Option<&str>) -> String {
    let path = scratch(test).join("size.store");
    let script = workspace_root().join("tests/scripts/store_persist_size_710.loft");
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg("--interpret")
        .arg(&script)
        .env("LOFT_PERSIST_TEST_PATH", &path)
        .env("LOFT_PERSIST_TEST_N", n.to_string())
        .env("LOFT_PERSIST_TEST_PER", per.to_string())
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .env("LOFT_STORE_LIVE_DIGEST", "1")
        .current_dir(workspace_root());
    match seed {
        Some(s) => cmd.env("LOFT_HASH_SEED", s),
        None => cmd.env_remove("LOFT_HASH_SEED"),
    };
    let out = cmd.output().expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success() && stdout.contains("persist true"),
        "{mode} failed: {stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    live_digest_line(&String::from_utf8_lossy(&out.stderr))
}

/// loft#710 — a persisted store's size must follow what it HOLDS.
///
/// It did not: the image was the arena's whole capacity, so the file carried
/// whatever the last 7/3 growth over-allocated.  Two consequences, both
/// measured here at the shapes the report used.
///
/// 1. The size did not move with the content.  125 records x 1000 coordinates
///    and 125 x 2312 — 2.3x the data — persisted to the SAME 3,816,152 bytes,
///    so a consumer sizing its hosting from file bytes was reading the
///    allocator, and a block that doubled because a category was added looked
///    exactly like one that doubled by allocation.
///
/// 2. At the reported shape, construction order alone decided the size: filling
///    each vector whole before inserting gave 1.84x what growing them
///    interleaved gave, for byte-identical data.  Interleaved is the shape that
///    matters — it is what a streaming generator does.
///
/// The live store still carries the INTERIOR free space each order leaves; the
/// image does not, because `store_persist_copy` writes the collection rebuilt.
// @speed 1.6
#[test]
fn persisted_size_tracks_content_not_construction() {
    let (small, _) = persist_size("size710_small", 125, 1000, "interleaved", None);
    let (large, _) = persist_size("size710_large", 125, 2312, "interleaved", None);
    let growth = large as f64 / small as f64;
    assert!(
        growth > 1.4,
        "2.3x the coordinates must show in the file (both were 3,816,152 bytes): \
         small={small} large={large} ratio={growth:.2}"
    );

    let whole_dir = scratch("size710_whole");
    let inter_dir = scratch("size710_interleaved");
    let whole_path = whole_dir.join("size.store");
    let inter_path = inter_dir.join("size.store");
    let (whole, d_whole) = persist_size_at(&whole_path, 125, 2312, "whole", None);
    let (inter, d_inter) = persist_size_at(&inter_path, 125, 2312, "interleaved", None);
    let digest = |line: &str| line.split("digest").nth(1).unwrap_or("").trim().to_string();
    assert_eq!(
        digest(&d_whole),
        digest(&d_inter),
        "the two orders must hold identical data, or the sizes below compare nothing:\n  {d_whole}\n  {d_inter}"
    );

    // Each image must read back as the data it was written from.
    let (_, r_whole) = persist_size_at(&whole_path, 125, 2312, "reload", None);
    let (_, r_inter) = persist_size_at(&inter_path, 125, 2312, "reload", None);
    assert_eq!(
        digest(&r_whole),
        digest(&d_whole),
        "the image must hold the data it was written from:\n  {d_whole}\n  {r_whole}"
    );
    assert_eq!(
        digest(&r_inter),
        digest(&d_inter),
        "the image must hold the data it was written from:\n  {d_inter}\n  {r_inter}"
    );
    let ratio = whole.max(inter) as f64 / whole.min(inter) as f64;
    assert!(
        ratio < 1.3,
        "construction order decided the size 1.84x at this shape; the image is the \
         collection rebuilt, so it must not: whole={whole} interleaved={inter} \
         ratio={ratio:.2}"
    );
    // And it must land on the CONTENT, not merely agree with itself: 125 records of
    // 2312 points, 8 bytes a point, is 2 312 000 bytes of coordinates.
    let content = 125.0 * 2312.0 * 8.0;
    assert!(
        whole.max(inter) as f64 / content < 1.3,
        "an image should be its content plus the format's overhead: \
         content={content} whole={whole} interleaved={inter}"
    );
}

/// One `count/sum/xor/sound` line from `store_digest_b0.loft`.
#[derive(PartialEq, Eq, Debug, Clone)]
struct B0 {
    count: i64,
    sum: i64,
    xor: i64,
    sound: bool,
}

/// Run the B0 oracle script and read back one tagged report line.
fn b0_run(
    backend: &str,
    path: &Path,
    order: &str,
    damage: &str,
    mode: &str,
    tag: &str,
    seed: Option<&str>,
) -> B0 {
    b0_run_full(backend, path, order, damage, mode, tag, seed).0
}

/// The digest of the bytes a persisted store holds IN ACTIVE USE, from the
/// `LOFT_STORE_LIVE_DIGEST=1` line its write prints (`Stores::live_digest`) — what two images
/// are compared by.  A saved image is the store's buffer, which also carries bytes no reader
/// reads (a vector's capacity past its length, a text's tail): a claim promises no value, so
/// those differ between runs that store the same data.
fn live_digest_line(stderr: &str) -> String {
    stderr
        .lines()
        .find_map(|l| l.strip_prefix("loft: live-digest "))
        .and_then(|rest| rest.split_whitespace().next())
        .unwrap_or_else(|| panic!("no live-digest line:\n{stderr}"))
        .to_string()
}

/// [`b0_run`], and the live digest of the image the run wrote (empty when it wrote none).
fn b0_run_full(
    backend: &str,
    path: &Path,
    order: &str,
    damage: &str,
    mode: &str,
    tag: &str,
    seed: Option<&str>,
) -> (B0, String) {
    let script = workspace_root().join("tests/scripts/store_digest_b0.loft");
    let mut cmd = loft::platform::process::harness_command(loft_bin());
    cmd.arg(backend)
        .arg(&script)
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .env("LOFT_STORE_LIVE_DIGEST", "1")
        .env("B0_ORDER", order)
        .env("B0_DAMAGE", damage)
        .env("B0_N", "200")
        .current_dir(workspace_root());
    match seed {
        Some(s) => cmd.env("LOFT_HASH_SEED", s),
        None => cmd.env_remove("LOFT_HASH_SEED"),
    };
    let out = cmd.output().expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "{backend} {order}/{damage}/{mode} failed: {stdout}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let line = stdout
        .lines()
        .find(|l| l.starts_with(tag))
        .unwrap_or_else(|| panic!("no `{tag}` line for {backend} {order}/{damage}:\n{stdout}"));
    let num = |key: &str| -> i64 {
        let mut it = line.split_whitespace();
        while let Some(t) = it.next() {
            if t == key {
                return it.next().and_then(|v| v.parse().ok()).unwrap_or_else(|| {
                    panic!("`{key}` is not a number in `{line}`");
                });
            }
        }
        panic!("no `{key}` in `{line}`");
    };
    let b0 = B0 {
        count: num("count"),
        sum: num("sum"),
        xor: num("xor"),
        sound: line.ends_with("sound true"),
    };
    let stderr = String::from_utf8_lossy(&out.stderr);
    let live = if stderr.contains("loft: live-digest ") {
        live_digest_line(&stderr)
    } else {
        String::new()
    };
    (b0, live)
}

/// @PLN123 B0 — the oracle arc B will be built against, calibrated BEFORE any
/// compaction code exists.
///
/// Invariant B is that a compacted image holds exactly the records reachable
/// from the root and reloads value-identical. No size assertion can check that:
/// a file that got smaller by losing data satisfies every one of them. So the
/// question here is not "is compaction correct" — there is no compaction yet —
/// but "would this digest KNOW". Two halves:
///
/// **It must see every loss.** Five injuries, one per way a rebuild could drop
/// something: a missing record, a changed scalar, a shortened text, a truncated
/// vector, a changed nested-struct field. Each must move the numbers. An oracle
/// that cannot fail is worse than none, because it reports success.
///
/// **It must be a function of the DATA, not the representation.** A rebuild
/// changes where everything sits, so anything the digest picks up from layout
/// would read as data loss — wrong in the direction that gets a fix applied to
/// working code. The five runs below hold identical records in five different
/// representations (three build orders × two bucket seeds), and the assertion
/// that the resulting FILES differ is what stops "same digest" from meaning
/// "nothing was perturbed".
///
/// **F9 is not a live hazard, and that took establishing.** The step assumed a
/// rebuild reorders iteration, so the digest was written order-independent
/// (sum/xor/count). Probing it with a deliberately order-DEPENDENT fold showed
/// no build order and no seed changes the order records come out in — loft
/// iterates a keyed collection through a key-sorted snapshot, so a rebuild
/// cannot reorder it. The order-independent form stays because it costs
/// nothing and keeps the oracle off that implementation detail; what could
/// not stay was the *assertion*, which no available lever can make fail.
///
/// The loss half earned its place immediately: the first draft truncated the
/// vector of a record whose vector was ALREADY one element, and the digest came
/// back identical. That reads exactly like a blind oracle and was a blind
/// injury — the failure mode this whole test exists to make visible.
// @speed 1.5
#[test]
fn store_digest_b0_oracle_sees_loss_not_layout() {
    for backend in ["--interpret", "--native-rust"] {
        let tag = backend.trim_start_matches('-');
        let dir = scratch(&format!("b0_{tag}"));
        let run = |file: &str, order: &str, damage: &str, mode: &str, want: &str, seed| {
            b0_run(backend, &dir.join(file), order, damage, mode, want, seed)
        };
        let run_live = |file: &str, order: &str, seed| {
            b0_run_full(
                backend,
                &dir.join(file),
                order,
                "none",
                "",
                "persisted",
                seed,
            )
        };

        // Same 200 records, five representations.  Every run pins the bucket
        // seed, because an unseeded hash draws a RANDOM one per process (the
        // P253 hash-DoS defense) — leave it unpinned and the images differ no
        // matter what else you vary, so the control below would pass while
        // attributing nothing.  Pinned, each row differs from the first by
        // exactly one named lever.
        let reps: [(&str, &str, &str, &str); 5] = [
            ("base.store", "forward", "7", "the reference representation"),
            ("again.store", "forward", "7", "a repeat of it"),
            (
                "reverse.store",
                "reverse",
                "7",
                "records inserted back to front",
            ),
            ("inter.store", "interleaved", "7", "evens then odds"),
            ("seed1.store", "forward", "1", "a different bucket seed"),
        ];
        // Each image by the bytes it holds in active use (`live_digest_line`): the gaps a
        // claim leaves unwritten differ from run to run and are no part of the data.
        let mut images: Vec<String> = Vec::new();
        let mut base: Option<B0> = None;
        for (file, order, seed, what) in reps {
            let (got, live) = run_live(file, order, Some(seed));
            assert!(got.sound, "{backend}: {what} must verify");
            assert!(
                got.count == 200 && got.sum != 0 && got.xor != 0,
                "{backend}: {what} produced an empty digest: {got:?}"
            );
            match &base {
                None => base = Some(got),
                Some(b) => assert_eq!(
                    &got, b,
                    "{backend}: {what} holds the same records — a digest that \
                     moved with the layout would report arc B's rebuild as data loss"
                ),
            }
            images.push(live);
        }
        let base = base.expect("five representations");

        // The controls for the claim above.  Negative: same order, same seed,
        // the same live bytes — so the harness is deterministic and the differences
        // below are attributable rather than noise.
        assert!(
            !images[0].is_empty(),
            "{backend}: the persist printed no live digest"
        );
        assert_eq!(
            images[0], images[1],
            "{backend}: two identical runs must store identical live bytes, or \
             nothing measured here can be attributed to anything"
        );
        // Positive: each lever really does reach the stored bytes, so the
        // invariance above is a statement about data surviving a CHANGED
        // representation — not about nothing having changed.
        for (i, lever) in [
            (2, "insertion order"),
            (3, "insertion order"),
            (4, "the bucket seed"),
        ] {
            assert_ne!(
                images[0], images[i],
                "{backend}: {lever} did not reach the stored live bytes, so the \
                 digest's invariance across it proves nothing"
            );
        }

        // Each injury must move the numbers — the proof the digest can fail.
        for damage in ["drop", "scalar", "text", "vector", "inner"] {
            let hurt = run("damaged.store", "forward", damage, "", "built", None);
            assert_ne!(
                hurt, base,
                "{backend}: a `{damage}` loss is invisible to the digest — an \
                 oracle that cannot fail reports success"
            );
            if damage == "drop" {
                assert_eq!(hurt.count, base.count - 1, "{backend}: a record vanished");
            }
        }

        // Round trip: the image written above; a FRESH process reads it
        // back, and deliberately UNSEEDED — it draws its own random bucket
        // seed and must still find every record, because the seed the writer
        // used travels in the image.  This is the comparison arc B's compacted
        // image has to pass.
        let reloaded = run("base.store", "forward", "none", "reload", "reload", None);
        assert_eq!(
            reloaded, base,
            "{backend}: the image must reload value-identical"
        );
        assert!(reloaded.sound, "{backend}: and with a sound heap graph");
    }
}

/// @PLN123 B1 — rebuild-and-swap, measured against B0's oracle.
///
/// The script rebuilds a fragmented collection at loft level, record by record,
/// and writes an image of each stage; `store_persist_copy` writes every image
/// rebuilt too.  What this pins:
///
/// - **The digest survives every stage.** A smaller file that lost data would
///   satisfy every size assertion here, so B0's oracle rides along.
/// - **An image carries no interior free space.** The fragmented collection's
///   image is the size of its rebuild's.
/// - **The rebuild is IDEMPOTENT.** A second and third rebuild land on the same
///   byte count: a compaction that grew the file a little every time would be
///   worse than none.
/// - **It reaches the from-scratch ceiling**, the image of the same records
///   built fresh.
/// - **Both backends agree byte for byte**, so the measurement is a property of
///   the store, not of one execution path.
#[test]
fn store_rebuild_b1_recovers_the_interior_and_is_idempotent() {
    let script = workspace_root().join("tests/scripts/store_rebuild_b1.loft");
    // tag, count, sum, xor, bytes — one row per stage the script reports.
    type Row = (String, i64, i64, i64, i64);
    let mut per_backend: Vec<Vec<Row>> = Vec::new();
    for backend in ["--interpret", "--native-rust"] {
        let dir = scratch(&format!("b1_{}", backend.trim_start_matches('-')));
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", dir.join("m"))
            .env("B1_SHAPE", "full")
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success(),
            "{backend} failed: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        // tag count sum xor bytes us
        let rows: Vec<Row> = stdout
            .lines()
            .filter(|l| l.contains(" count ") && l.contains(" bytes "))
            .map(|l| {
                let t: Vec<&str> = l.split_whitespace().collect();
                let num = |key: &str| -> i64 {
                    let i = t.iter().position(|x| *x == key).expect(key);
                    t[i + 1].parse().expect("number")
                };
                (
                    t[0].to_string(),
                    num("count"),
                    num("sum"),
                    num("xor"),
                    num("bytes"),
                )
            })
            .collect();
        let get = |tag: &str| {
            rows.iter()
                .find(|r| r.0 == tag)
                .unwrap_or_else(|| panic!("no `{tag}` row in:\n{stdout}"))
                .clone()
        };
        let frag = get("fragmented");
        let rebuilt = get("rebuilt");
        let again = get("rebuilt2");
        let third = get("rebuilt3");
        let fresh = get("fresh");

        // B0's oracle rides along: every stage holds the same records.
        for row in &rows {
            assert_eq!(
                (row.1, row.2, row.3),
                (frag.1, frag.2, frag.3),
                "{backend}: `{}` lost or changed data, so its size proves nothing:\n{stdout}",
                row.0
            );
        }

        // `store_persist_copy` writes every image rebuilt, so the fragmented
        // collection's image already has its interior back: it is the size a
        // loft-level rebuild's image is.
        assert_eq!(
            frag.4, rebuilt.4,
            "{backend}: the image of a fragmented collection must not carry its \
             interior free space:\n{stdout}"
        );

        // The property an automatic rebuild rests on.
        assert_eq!(
            (rebuilt.4, rebuilt.4),
            (again.4, third.4),
            "{backend}: rebuilding must reach a FIXED POINT — a compaction that \
             grows the file a little each time is worse than none:\n{stdout}"
        );

        // The ceiling: a rebuild lands on a from-scratch build's bytes.  The gap
        // @PLN123 B1 measured was the vector field — a copy into an empty
        // destination claimed `vector_append`'s eleven-element first block where a
        // fresh build claimed by length; `vector_add` now claims the length.
        // `LOFT_NO_EXACT_COPY_CLAIM=1` reopens it in the rebuild; this assertion
        // fails on it together with `LOFT_NO_APPEND_IN_PLACE=1` (alone, the fresh
        // build is written in its element through the same claim and lands on the
        // same flat block).
        assert_eq!(
            rebuilt.4, fresh.4,
            "{backend}: a rebuild must reach the from-scratch ceiling:\n{stdout}"
        );
        per_backend.push(rows);
    }
    assert_eq!(
        per_backend[0], per_backend[1],
        "the interpreter and --native must agree on every size and digest, or \
         this measures an execution path rather than the store"
    );
}

/// @PLN123 B2/B3 — a collection written as an image and loaded back, with
/// compaction at LOAD on (the default) and off (`LOFT_NO_COMPACT_ON_LOAD`).
///
/// `store_persist_copy` writes the collection rebuilt, so the image is already
/// dense and compaction at load has little to take; what must hold either way:
///
/// - **The data survives.** B0's oracle rides along — the digest over every
///   field must be unchanged, and `store_verify` must call the loaded heap graph
///   sound.
/// - **The root survives.** The collection variable is a `DbRef` at the root, so
///   the root is the one position a rebuild may not move; the script reads an
///   element through it immediately after the load, and keeps using the
///   collection after.
/// - **Refusal is audible and safe.** A record holding a `reference<T>` holds a
///   position, which the rebuild does not rewrite — the write answers `false`
///   and leaves no file, rather than an image whose links name other records.
#[test]
fn store_compact_b2_rebuilds_at_load_without_losing_anything() {
    let script = workspace_root().join("tests/scripts/store_compact_b2.loft");
    let run = |backend: &str, dir: &Path, shape: &str, compact: bool| -> String {
        let mut cmd = loft::platform::process::harness_command(loft_bin());
        cmd.arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", dir.join("c"))
            .env("B2_SHAPE", shape)
            .env("LOFT_LOADER_STATS", "1")
            .current_dir(workspace_root());
        if compact {
            cmd.env_remove("LOFT_NO_COMPACT_ON_LOAD"); // @PLN123 B3 — on by default
        } else {
            cmd.env("LOFT_NO_COMPACT_ON_LOAD", "1");
        }
        let out = cmd.output().expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success(),
            "{backend} {shape} compact={compact} failed: {stdout}\n{stderr}"
        );
        assert!(!stdout.contains("FAIL"), "{backend} {shape}: {stdout}");
        format!("{stdout}{stderr}")
    };
    let field = |out: &str, line: &str, key: &str| -> i64 {
        let l = out
            .lines()
            .find(|l| l.starts_with(line))
            .unwrap_or_else(|| panic!("no `{line}` line in:\n{out}"));
        let t: Vec<&str> = l.split_whitespace().collect();
        let i = t.iter().position(|x| *x == key).expect(key);
        t[i + 1].parse().expect("number")
    };

    for backend in ["--interpret", "--native-rust"] {
        let tag = backend.trim_start_matches('-');

        // With compaction at load on and off: the same data, a sound heap, a root
        // that still reads, and a store that still takes writes.
        for compact in [false, true] {
            let out = run(
                backend,
                &scratch(&format!("b2_{tag}_{}", u8::from(compact))),
                "rich",
                compact,
            );
            for key in ["count", "sum", "xor"] {
                assert_eq!(
                    field(&out, "source", key),
                    field(&out, "loaded", key),
                    "{backend} compact={compact}: `{key}` changed across the image — \
                     the load LOST data:\n{out}"
                );
            }
            assert!(
                out.lines()
                    .filter(|l| l.starts_with("loaded"))
                    .all(|l| l.ends_with("sound true")),
                "{backend} compact={compact}: the loaded heap graph must verify:\n{out}"
            );
            assert!(
                out.contains("root_ref label label-7") && out.contains("old label-13"),
                "{backend} compact={compact}: an element read through the root after \
                 the load, and the store still takes writes:\n{out}"
            );
        }

        // A `reference<T>` into the same store: the image is rebuilt by the converter,
        // which re-points each reference; the load reads every link back (hand-computed
        // in the script: 30 nodes, links summing to 406).
        let refs = run(backend, &scratch(&format!("b2ref_{tag}")), "refuse", true);
        assert!(
            refs.contains("refuse write true")
                && refs.contains("refuse count 30 links 406 sound true"),
            "{backend}: a collection holding references into its own store is written \
             and read back with every link:\n{refs}"
        );

        // A reference into a store outside the collection: the same decline.
        let foreign = run(backend, &scratch(&format!("b2for_{tag}")), "foreign", true);
        assert!(
            foreign.contains("foreign write false"),
            "{backend}: a collection holding a reference into another store must \
             not be written:\n{foreign}"
        );
    }
}

/// @PLN123 — compaction across COLLECTION KINDS.
///
/// Compaction at load is default-on and rebuilds through `copy_claims`, which
/// `type_is_compactable` waves through for every collection kind — only a
/// cross-store `DbRef` is refused, because this rebuild moves records within ONE
/// store and nothing would rewrite a foreign pointer at the far end. Every other
/// compaction test builds a `hash<Rec[id]>`, so
/// the rest of that surface shipped by default with no coverage, and the
/// array/ordered destination builder carries a documented history of exactly the
/// failure a digest catches (@P309: copied collections "read back as length 0").
///
/// Per kind: build, fragment, persist, `store_load` back. The count, the
/// order-independent digest and `store_verify` must all survive, and the loader
/// must reach the verdict this kind should reach.
///
/// **Two shapes could not be built at all, both pre-existing** (they reproduce
/// on the released 2026.7.2 binary, so neither is compaction's doing):
/// - fragmenting an index with `#remove` in a filtered loop SIGSEGVs the
///   interpreter and overflows the native stack when the record owns a `text`
///   (loft#718) — worked around here with key assignment, which is a different
///   path;
/// - the `ordered<T>` secondary shape needs a struct declaring BOTH a
///   `sorted<T[..]>` and an `index<T[..]>` field, and merely DECLARING that
///   hangs the interpreter and miscompiles on `--native` (loft#719). That cell
///   is not skipped for convenience — the shape itself does not work.
// @speed 1.6
#[test]
fn compaction_is_correct_for_every_collection_kind_it_accepts() {
    let script = workspace_root().join("tests/scripts/store_compact_kinds.loft");
    // kind, and the verdict the loader must reach for it.
    let kinds: [(&str, &str); 6] = [
        ("hash", "compacted"),
        ("sorted", "compacted"),
        ("index", "compacted"),
        ("nested", "compacted"),
        // @PLN134 — a trie became compactable when its records had a reason to
        // move: the rebuild places them in key order, which is what a paged
        // prefix query is cheap on.
        ("trie", "compacted"),
        // @PLN136 — and a spatial did, for the same reason one step over: the
        // rebuild places its records in MORTON order, which is what a paged
        // bounding-box query is cheap on. It was the last refusal in this list.
        ("spatial", "compacted"),
    ];
    let run = |backend: &str, dir: &Path, kind: &str, reload: bool| -> String {
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", dir.join("k.store"))
            .env("KIND", kind)
            .env("MODE", if reload { "reload" } else { "" })
            .env("LOFT_LOADER_STATS", "1")
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            out.status.success() && !stdout.contains("FAIL"),
            "{backend} kind={kind} reload={reload}: {stdout}\n{stderr}"
        );
        format!("{stdout}{stderr}")
    };
    let field = |out: &str, key: &str| -> String {
        let l = out
            .lines()
            .find(|l| l.starts_with("kind "))
            .unwrap_or_else(|| panic!("no report in:\n{out}"));
        let t: Vec<&str> = l.split_whitespace().collect();
        let i = t.iter().position(|x| *x == key).expect(key);
        t[i + 1].to_string()
    };

    for backend in ["--interpret", "--native-rust"] {
        for (kind, verdict) in kinds {
            let tag = backend.trim_start_matches('-');
            let dir = scratch(&format!("kinds_{kind}_{tag}"));
            let wrote = run(backend, &dir, kind, false);
            let read = run(backend, &dir, kind, true);

            for key in ["count", "sum"] {
                assert_eq!(
                    field(&wrote, key),
                    field(&read, key),
                    "{backend}/{kind}: `{key}` changed across the load — a \
                     rebuild that lost data would satisfy every size assertion \
                     anyone would write:\n{wrote}{read}"
                );
            }
            assert_ne!(field(&wrote, "sum"), "0", "{backend}/{kind}: empty digest");
            assert!(
                field(&wrote, "sound") == "true" && field(&read, "sound") == "true",
                "{backend}/{kind}: the heap graph must verify on both sides:\n{wrote}{read}"
            );
            assert!(
                read.contains(verdict),
                "{backend}/{kind}: expected the loader to report `{verdict}`, \
                 which is what makes this cell test anything:\n{read}"
            );
        }
    }
}

/// loft#710 — `LOFT_HASH_SEED` makes a persisted store byte-reproducible.
///
/// A hash's seed is drawn at random (the P253 hash-DoS defense) and stored in
/// the bucket record, where it decides the bucket ORDER — so rebuilding the
/// same data gave a different file every time, and a per-block checksum could
/// not tell "the data changed" from "it was rebuilt".  The unseeded pair is the
/// control: without it, two identical files would prove nothing about the seed.
#[test]
fn fixed_hash_seed_makes_a_persisted_store_reproducible() {
    // An image is compared by the bytes it holds in ACTIVE USE (`live_digest_line`): the gaps
    // a claim leaves unwritten (a vector's capacity past its length) are no part of the data
    // and differ between runs.  The file's LENGTH is layout, and stays compared.
    let read = |dir: &str, seed: Option<&str>| {
        let digest = persist_live_digest(dir, 120, 30, "interleaved", seed);
        let len = fa::metadata(scratch_existing(dir).join("size.store"))
            .expect("store file")
            .len();
        (len, digest)
    };
    let a = read("seed710_a", Some("12345"));
    let b = read("seed710_b", Some("12345"));
    assert_eq!(a.0, b.0, "same seed, same data: same length");
    assert!(
        a == b,
        "same seed, same data must give the same live bytes: {a:?} vs {b:?}"
    );

    let c = read("seed710_c", None);
    let d = read("seed710_d", None);
    assert!(
        c != d,
        "without LOFT_HASH_SEED the live bytes must still vary — otherwise this test proves nothing"
    );

    // loft#856 — the SPELLING is part of the contract, and only the decimal one was covered.
    // `0x…` parsed as nothing and fell through to the random default in silence, so a caller
    // who wrote a 64-bit seed the way 64-bit seeds are written got the behaviour they had
    // explicitly opted out of, with no way to tell. It was loft's own suite that paid:
    // `paged_browser.rs` pinned the seed in hex, with a comment explaining that the pin is
    // what stops the test measuring a coin flip, and the pin never took effect.
    //
    // Reproducible-in-hex is not enough to assert — a hex value that parsed to some OTHER
    // number would also be reproducible. So the hex and decimal spellings of one value must
    // give the SAME bytes, which only holds if both reached the same seed.
    let hex = read("seed710_hex", Some("0x0123456789abcdef"));
    let dec = read("seed710_dec", Some("81985529216486895"));
    assert!(
        hex == dec,
        "`0x0123456789abcdef` and its decimal spelling must be the same seed: {hex:?} vs {dec:?}"
    );
    assert!(
        hex != c,
        "the hex-seeded build must differ from an unseeded one — otherwise the seed did nothing"
    );
}

/// loft#727 — READING a keyed collection must cost its store nothing.
///
/// `for r in h` walks a key-sorted snapshot of rec-nrs claimed inside the
/// hash's own store, and the loop epilogue must release it there too, not only
/// when the snapshot has a store of its own (the read-only/`expose`d case): a
/// collection outlives the loops that read it, so a snapshot left behind grows
/// its store for as long as the program iterates.  The census counts records,
/// so it fails on the first leaked pass (pre-fix: 4,012 records → 4,092 over 40
/// passes).
// @speed 2.0
#[test]
fn reading_a_collection_leaks_nothing_into_its_store() {
    let script = workspace_root().join("tests/scripts/store_iter_scratch.loft");
    let run = |backend: &str, dir: &Path, mode: &str| -> String {
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", dir.join("s.store"))
            .env("LOFT_HASH_SEED", "727")
            .env("MODE", mode)
            .env("N", "2000")
            .env("PASSES", "40")
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(
            out.status.success() && !stdout.contains("FAIL"),
            "{backend} {mode}: {stdout}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        stdout
    };
    // The `store_memory()` row for the collection, from the census printed
    // after `marker`. Carries the record count and the largest free block, both
    // exact — the MB column is rounded and would hide two records.
    let rows = |stdout: &str, marker: &str| -> Vec<String> {
        let found: Vec<String> = stdout
            .lines()
            .skip_while(|l| !l.starts_with(marker))
            .take_while(|l| !l.starts_with("census") || l.starts_with(marker))
            .filter(|l| l.contains("type hash<") || l.contains("type spatial<"))
            // Drop the leading slot number: two runs need not land in the same
            // slot, and the slot is not the subject.
            .filter_map(|l| l.split_once("MB").map(|(_, rest)| rest.trim().to_string()))
            .collect();
        assert!(
            !found.is_empty(),
            "no census row after '{marker}' in:\n{stdout}"
        );
        found
    };

    for backend in ["--interpret", "--native-rust"] {
        let tag = backend.trim_start_matches('-');

        // In memory: 41 traversals must leave the census exactly where 1 did.
        let mem = run(backend, &scratch(&format!("iter_mem_{tag}")), "memory");
        assert_eq!(
            rows(&mem, "census one"),
            rows(&mem, "census many"),
            "{backend}: 40 more reads of the same collection changed its store\n{mem}"
        );

        // Every way OUT of such a loop, and both builders.  `break` leaves
        // through the epilogue, `return` skips it for the scope exit, a nested
        // loop holds two scratches at once, and a spatial collection builds
        // its scratch through a different pair of builders.  The release must
        // also survive being called twice on the same value, which is what the
        // emitted IR does — the epilogue's free and the scope exit's free are
        // adjacent, and the null-assignment meant to separate them is dropped.
        let exits = run(backend, &scratch(&format!("iter_exits_{tag}")), "exits");
        assert_eq!(
            rows(&exits, "census one"),
            rows(&exits, "census many"),
            "{backend}: 40 rounds of break / return / nested / spatial / range \
             changed a store\n{exits}"
        );
    }
}

/// `scratch` wipes the directory; this returns the same path without wiping, so
/// a caller can read back what a just-finished run wrote there.
fn scratch_existing(test_name: &str) -> PathBuf {
    let base = std::env::var_os("TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    base.join("loft-store-persist-loft").join(test_name)
}

/// @PLN129 arc A — a collection BOUND to a lazy source fetches on a MISS, and
/// fetches only what was asked for.
///
/// The load-bearing assertions are the resident COUNTS. Every value assertion
/// here would also pass under an eager whole-image load; `resident=1` then
/// `resident2=2` against a THREE-entry image is what proves the read was lazy.
/// The unbound control at the top is what proves the collection started empty,
/// without which the counts could be measuring nothing.
#[test]
fn lazy_bound_collection_fetches_only_the_touched_entry_both_backends() {
    let dir = scratch("lazy_bind_129");
    let path = dir.join("lz.store");
    let script = workspace_root().join("tests/scripts/129-lazy-bind.loft");

    let (out_w, code_w) = run_lazy("--interpret", &script, &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("seeded=3"), "write: {out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_lazy(backend, &script, &path, "read");
        assert_eq!(code, 0, "{backend} read exit: {out:?}");

        // The control: nothing is resident, and an unbound miss stays a miss.
        assert!(
            out.contains("unbound_miss=true unbound_len=0"),
            "{backend}: an UNBOUND collection must answer absent and hold nothing — \
             without this the counts below prove nothing: {out:?}"
        );
        assert!(out.contains("bound=true"), "{backend}: bind: {out:?}");

        // The fault itself, and the count that makes it lazy.
        assert!(
            out.contains("fetched=true name=grace resident=1"),
            "{backend}: a miss must fetch exactly ONE entry from a 3-entry image: {out:?}"
        );
        // One record per key, and a hit costs nothing.
        assert!(
            out.contains("same=true after_hit=1"),
            "{backend}: the same key twice is one record, and a hit fetches nothing: {out:?}"
        );
        assert!(
            out.contains("second=alan resident2=2"),
            "{backend}: a second fault adds exactly one: {out:?}"
        );
        // Absent stays absent, and does not grow the working set.
        assert!(
            out.contains("absent=true resident3=2"),
            "{backend}: a key absent from the SOURCE must not materialise anything: {out:?}"
        );
        assert!(
            out.contains("verify=true"),
            "{backend}: the partially-loaded heap must be structurally sound: {out:?}"
        );

        // @PLN129 arc C — the failure channel. A reachable source that simply
        // lacks the key must leave NO error, or "absent" and "unreachable" are
        // the same answer and the channel is useless.
        assert!(
            out.contains("err_after_absent=[]"),
            "{backend}: a genuine absence must leave the collection healthy: {out:?}"
        );
        assert!(out.contains("gone_bound=true"), "{backend}: bind: {out:?}");
        assert!(
            out.contains("gone_faults=1"),
            "{backend}: a failed fetch must be COUNTED: {out:?}"
        );
        // The shape that was silently wrong before stickiness: fail once, then
        // succeed. The collection is missing data, so it must NOT read healthy.
        assert!(
            out.contains("mixed_ok=true mixed_still_faulted=true"),
            "{backend}: a later success must NOT clear an earlier failure — that \
             traversal is missing data and 'healthy' would be a lie: {out:?}"
        );
        assert!(
            out.contains("cleared=true after_clear=0"),
            "{backend}: an explicit acknowledgement is what clears: {out:?}"
        );
        assert!(
            out.contains("clear_again=false"),
            "{backend}: clearing twice reports nothing left to clear: {out:?}"
        );
        // @PLN129 arc D — the control for the source pin: an UNCHANGED source
        // must fault nothing. Without this a pin that refused every fetch would
        // look correct.
        // @PLN129 arc E — evict-and-refault, and the safety that makes it
        // offerable: a held reference to an evicted record reads null.
        assert!(
            out.contains("ev_filled=grace ev_len=1"),
            "{backend}: the working set filled: {out:?}"
        );
        assert!(
            out.contains("ev_emptied=0"),
            "{backend}: `= []` must empty the working set: {out:?}"
        );
        assert!(
            out.contains("ev_held=grace"),
            "{backend}: a held reference must SURVIVE eviction with its value — \
             the deps system keeps a referenced record alive while the rest is \
             reclaimed, and that is what makes eviction safe to offer: {out:?}"
        );
        assert!(
            out.contains("ev_refaulted=alan ev_len2=1"),
            "{backend}: the BINDING must survive eviction so the next lookup \
             re-faults — otherwise emptying a collection silently unbinds it: {out:?}"
        );
        assert!(
            out.contains("stable=grace,alan stable_faults=0"),
            "{backend}: a stable source must serve a whole traversal with no \
             drift fault — otherwise the pin is refusing everything: {out:?}"
        );
        assert!(
            out.contains("gone_null=true gone_err_empty=false"),
            "{backend}: an UNREACHABLE source must answer null AND leave a reason — \
             the whole point is that these two nulls are told apart: {out:?}"
        );
    }
}

/// loft#802 — a lazy binding that cannot be served must say so IN-BAND, and a
/// binding that works must stay silent.
///
/// The reported failure was a `trie` bound to a `.store` image: `true` from the
/// bind, `null` from every lookup, `""` from `store_lazy_error` — whose
/// documented meaning is "reachable, genuinely no such key" — and `0` faults.
/// The refusal existed only on stderr, so the program had nothing but the null.
///
/// FOUR refusal cells, because a refusal is decided in two different places and
/// a fix for one leaves the other silent: the KIND is static and refuses at the
/// bind (`index`, `sorted`), while a foreign layout and a non-relocatable entry
/// are only knowable when a fetch runs. FOUR controls carry the test: the
/// REPORTED kind is served now (@PLN134) and must stay quiet, a working hash must
/// stay quiet (a fix that reported every null as a refusal would pass every
/// refusal cell), an unreachable source must keep its own more specific reason,
/// and a refused DIRECT load must not poison a later healthy binding.
#[test]
fn a_refused_lazy_binding_is_visible_to_the_program_both_backends() {
    let dir = scratch("lazy_refusal_802");
    let path = dir.join("rf");
    let script = workspace_root().join("tests/scripts/802-lazy-refusal-visible.loft");

    let (out_w, code_w) = run_lazy("--interpret", &script, &path, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(out_w.contains("seeded=2,1,1,1,1,2"), "write: {out_w:?}");

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_lazy(backend, &script, &path, "read");
        assert_eq!(code, 0, "{backend} read exit: {out:?}");

        // The reported shape, on a kind still unserved. It can never page, and
        // that is known with no I/O — so it fails at the call that is wrong, not
        // at a later lookup.
        assert!(
            out.contains("idx_bound=false"),
            "{backend}: a kind a paged image can never serve must be refused AT \
             THE BIND — `true` here is what made every later null look like an \
             absent key: {out:?}"
        );
        assert!(
            out.contains("idx_null=true idx_speaks=true"),
            "{backend}: the lookup still answers null (C80), and the channel that \
             exists to break that tie must carry the reason: {out:?}"
        );
        assert!(
            out.contains("idx_faults=1"),
            "{backend}: a binding that can serve NOTHING is maximally incomplete, \
             and `0` reads as healthy: {out:?}"
        );
        // The message is the deliverable here: the old one blamed the #632
        // wrapper-struct trap and told a plain local to declare itself as an
        // annotated local, which it already was.
        assert!(
            out.contains("idx_names_kind=true") && out.contains("idx_names_cure=true"),
            "{backend}: the refusal must name the KIND that was refused and the \
             call that does work: {out:?}"
        );
        // @PLN136 — the narrowing's second half. A `spatial` binds, FAULTS and
        // stays quiet. The fault count is the load-bearing part: a fetch routed by
        // the key's shape rather than the collection's kind would send a
        // two-coordinate lookup nowhere, and `spatial_v` alone cannot tell a fault
        // from a record that was already resident (none is).
        assert!(
            out.contains("spatial_bound=true") && out.contains("spatial_v=1020"),
            "{backend}: a spatial image is pageable since @PLN136, so the binding \
             must be accepted and the point must fault in: {out:?}"
        );
        assert!(
            out.contains("spatial_quiet=true") && out.contains("spatial_faults=0"),
            "{backend}: a binding that serves its lookups has nothing to report, \
             and a non-zero fault count would read as an incomplete one: {out:?}"
        );
        assert!(
            out.contains("spatial_absent=true") && out.contains("spatial_still_quiet=true"),
            "{backend}: a point the image does not hold is a genuine absence — the \
             refusal channel must not start speaking for it: {out:?}"
        );
        // The rule is the kind SET, not one kind — a fix keyed on one would leave
        // `sorted` exactly as silent.
        assert!(
            out.contains("sorted_bound=false") && out.contains("sorted_speaks=true"),
            "{backend}: `sorted` cannot page either, and must refuse the same \
             way: {out:?}"
        );

        // CONTROL 0 — @PLN134 narrowed the refused set, and the kind the report
        // was ABOUT is the one that moved. It must now bind, answer, and stay
        // quiet; keeping the old expectation here would pin a refusal that has
        // itself become the bug.
        assert!(
            out.contains("trie_bound=true") && out.contains("trie_hits=3"),
            "{backend}: a trie image is pageable now, so the bind is accepted and \
             the lookup reaches the record: {out:?}"
        );
        assert!(
            out.contains("trie_quiet=true trie_faults=0"),
            "{backend}: a served binding must not speak: {out:?}"
        );
        assert!(
            out.contains("trie_absent=true trie_still_quiet=true"),
            "{backend}: and a genuine absence in a SERVED trie is still an \
             absence: {out:?}"
        );

        // The refusals a bind CANNOT decide: the local is a `hash`, so the pair
        // is plausible, and only the fetch learns otherwise.
        assert!(
            out.contains("foreign_bound=true"),
            "{backend}: a hash local is a plausible binding — refusing it at the \
             bind would break the working case: {out:?}"
        );
        assert!(
            out.contains("foreign_null=true foreign_speaks=true")
                && out.contains("foreign_faults=1"),
            "{backend}: a foreign-layout image is refused by the layout gate, and \
             that refusal must reach the program too: {out:?}"
        );
        assert!(
            out.contains("big_bound=true") && out.contains("big_null=true big_speaks=true"),
            "{backend}: an entry the working-set copy cannot relocate is refused \
             at FETCH time, and was equally silent: {out:?}"
        );

        // CONTROL 1 — without this, "report every null as a refusal" passes
        // every assertion above and destroys the channel.
        assert!(
            out.contains("hash_hit=4200 hash_quiet=true"),
            "{backend}: a binding that WORKS must stay silent: {out:?}"
        );
        assert!(
            out.contains("hash_absent=true hash_still_quiet=true") && out.contains("hash_faults=0"),
            "{backend}: a genuine absence is still an absence — it must not start \
             reporting a reason: {out:?}"
        );

        // CONTROL 2 — the refusal must not PREEMPT the more specific answer. The
        // loader says only "it cannot be opened as a paged source"; the open
        // itself names the connection error, and the first fix lost it.
        assert!(
            out.contains("ur_null=true ur_speaks=true") && out.contains("ur_specific=true"),
            "{backend}: an unreachable source must keep its own reason, not the \
             loader's generic one: {out:?}"
        );

        // CONTROL 3 — the recorded refusal is per-fetch. A refused DIRECT
        // `store_load_key` must not leave a reason behind for an unrelated
        // collection bound afterwards.
        assert!(
            out.contains("direct_refused=false") && out.contains("clean_hit=4200 clean_quiet=true"),
            "{backend}: a refusal must not leak across collections: {out:?}"
        );

        // The sticky contract is untouched by all of this.
        assert!(
            out.contains("cleared=true after_clear=true"),
            "{backend}: an explicit acknowledgement still clears: {out:?}"
        );
    }
}

/// `run_mode_backend`'s sibling for the @PLN129 script, which reads its mode
/// from `LOFT_LAZY_MODE` so it cannot be confused with the persist scripts'.
fn run_lazy(backend: &str, script: &Path, path: &Path, mode: &str) -> (String, i32) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(script)
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_LAZY_MODE", mode)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        eprintln!(
            "{backend} {mode} stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    (stdout, out.status.code().unwrap_or(-1))
}

/// @PLN129 arc F — the gate: a real graph traversed lazily, with the fetch count
/// the design predicts.
///
/// Two assertions carry this test, and neither is about a value. **Identity**:
/// two persons at the same company must reach ONE company record, because
/// identity falls out of the collection rather than a side map — if that fails
/// the whole design is wrong. **Count**: `c=1` after the second hop is what
/// proves the hop HIT the working set; every value assertion here would pass
/// under an eager whole-image load, and only the counts would not.
#[test]
fn lazy_graph_traversal_fetches_only_what_it_touches_both_backends() {
    let dir = scratch("lazy_graph_129");
    let persons = dir.join("persons.store");
    let companies = dir.join("companies.store");
    let script = workspace_root().join("tests/scripts/129-lazy-graph.loft");

    let (out_w, code_w) = run_graph("--interpret", &script, &persons, &companies, "write");
    assert_eq!(code_w, 0, "write exit: {out_w:?}");
    assert!(
        out_w.contains("seeded companies=3 persons=4"),
        "seed: {out_w:?}"
    );

    for backend in ["--interpret", "--native-rust"] {
        let (out, code) = run_graph(backend, &script, &persons, &companies, "read");
        assert_eq!(code, 0, "{backend} read exit: {out:?}");

        // The control: both collections start empty, so the counts below mean
        // something.
        assert!(out.contains("start p=0 c=0"), "{backend}: {out:?}");
        assert!(
            out.contains("hop1 ada@Acme p=1 c=1"),
            "{backend}: one hop fetches one person and one company: {out:?}"
        );
        // The second person shares Acme: the company hop must HIT, so c stays 1.
        assert!(
            out.contains("hop2 grace@Acme p=2 c=1"),
            "{backend}: a second person at the SAME company must NOT re-fetch it \
             — c must stay 1: {out:?}"
        );
        assert!(
            out.contains("identity=true"),
            "{backend}: two paths to one company must give ONE record — identity \
             falls out of the collection, and if this fails the design is wrong: {out:?}"
        );
        assert!(
            out.contains("hop3 alan@Globex p=3 c=2"),
            "{backend}: a different company IS fetched: {out:?}"
        );
        // 3 persons + 2 companies for 3 hops — not 6, and edsger/Initech were
        // never asked for and must not be resident.
        assert!(
            out.contains("touched=5"),
            "{backend}: fetches must equal records TOUCHED, not reachable — a \
             lazy read that pulls the closure is an eager read with extra steps: {out:?}"
        );
        assert!(
            out.contains("sound=true,true"),
            "{backend}: both partially-loaded heaps must be structurally sound: {out:?}"
        );
    }
}

/// The @PLN129 graph script takes TWO scratch paths, one per collection —
/// per-collection binding is the point, so one path would not exercise it.
fn run_graph(
    backend: &str,
    script: &Path,
    persons: &Path,
    companies: &Path,
    mode: &str,
) -> (String, i32) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(script)
        .env("LOFT_GRAPH_PERSONS", persons)
        .env("LOFT_GRAPH_COMPANIES", companies)
        .env("LOFT_LAZY_MODE", mode)
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        eprintln!(
            "{backend} {mode} stderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    (stdout, out.status.code().unwrap_or(-1))
}

/// loft#783 — a pointer-bearing vector element relocates in BULK, not field-at-a-time.
///
/// `store_load_keys` bulk-read a `vector<Pt>` element block in one request, then fell off
/// that path the moment an element carried a POINTER: a `text` or a nested vector cost ~3
/// extra four-byte reads PER ELEMENT. Measured on a 100×250 `vector<Named>`, 25 000
/// elements drew **75 893** four-byte reads. In the consumer that filed it, one viewport
/// issued 804 313 of them — 175 ms of pure CPU with the bytes already in a prefetch buffer,
/// and ~1.3 s projected for a cold view. It was never bytes; it was call count.
///
/// Two of the three were avoidable and both are now gone: the field's POINTER word was
/// re-fetched over the reader although the caller had just flat-copied it into the local
/// record, and the sub-record's size and length words were fetched separately although
/// they are adjacent.
///
/// **The read counts are the assertion.** Every arm loaded the right values before the fix
/// too, so a value-only test says nothing about the defect — but the values are asserted
/// as well, because a relocation that dropped the strings would otherwise read as a win.
/// `LOFT_LOADER_WORDWISE=1` is the pre-fix route on the same binary: the control that
/// proves the count can move, so a green here is not a harness that measures nothing.
#[test]
fn a_pointer_bearing_element_relocates_in_bulk() {
    let dir = scratch("ptr_element_reads_783");
    let script = workspace_root().join("tests/scripts/783-ptr-element-reads.loft");
    let path = dir.join("p");

    let run = |mode: &str, wordwise: bool| -> String {
        let mut cmd = loft::platform::process::harness_command(loft_bin());
        cmd.arg("--interpret")
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", &path)
            .env("LOFT_LOADER_STATS", "1")
            .env("LOFT_TIMEOUT", "180")
            // Pin the hash seed, or this test measures the write's luck rather than the
            // loader. The store is written through a KEYED collection, its bucket seed is
            // drawn per hash, and the seed decides the bucket ORDER — so records land at
            // different offsets on every run and the ones that happen to be adjacent
            // coalesce into one read. Unpinned, the identical program cost 330..594
            // four-byte reads over 40 fresh writes (median 378), which put the old
            // `< 500` bound inside the spread: it failed on roughly the top decile and
            // read as a concurrency flake, because the full suite runs it often enough
            // to hit the tail. Pinned, it is 378 every time (loft#856 — and the hex
            // spelling used here is the one that used to be ignored in silence).
            .env("LOFT_HASH_SEED", "0x0123456789abcdef")
            .current_dir(workspace_root());
        if !mode.is_empty() {
            cmd.env("P783_MODE", mode);
        }
        if wordwise {
            cmd.env("LOFT_LOADER_WORDWISE", "1");
        }
        let out = cmd.output().expect("failed to invoke loft binary");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(
            out.status.success(),
            "mode={mode} wordwise={wordwise}: {all}"
        );
        assert!(!all.contains("FAIL"), "mode={mode}: {all}");
        all
    };

    /// The count in the 4-byte bucket of `reads=[(2, N, bytes), …]` — one entry per
    /// power-of-two size class, so bucket 2 is exactly the word-at-a-time traffic.
    fn four_byte_reads(out: &str) -> u64 {
        let at = out.find("reads=[").expect("loader stats must report reads");
        let rest = &out[at..];
        let open = rest.find("(2, ").map_or(usize::MAX, |i| i + 4);
        assert_ne!(open, usize::MAX, "no 4-byte bucket in: {rest}");
        let tail = &rest[open..];
        let end = tail.find(',').expect("a count then a comma");
        tail[..end].trim().parse().expect("a numeric read count")
    }
    /// The one result line, so the two routes can be compared value for value.
    fn result(out: &str) -> &str {
        out.lines()
            .find(|l| l.starts_with("loaded="))
            .expect("the script must report its load")
    }

    run("write", false);
    let fast = run("", false);
    let slow = run("", true);

    // Correctness FIRST: fewer reads that lose data is not a fix.
    assert_eq!(
        result(&fast),
        result(&slow),
        "the bulk route must load exactly what the word-at-a-time route does"
    );
    assert!(
        result(&fast).contains("elements=1000") && result(&fast).contains("chars=2800"),
        "every element and every string must survive the relocation: {}",
        result(&fast)
    );

    let (fast_reads, slow_reads) = (four_byte_reads(&fast), four_byte_reads(&slow));
    // The control: the pre-fix route really is word-at-a-time, so this test can fail.
    assert!(
        slow_reads > 1000,
        "the wordwise control must still be word-at-a-time ({slow_reads} reads) — if it is \
         not, this test is measuring nothing"
    );
    // Stated as a RATIO against the control as well as an absolute count. Both arms read the
    // SAME store in the same invocation, and the wordwise route is word-at-a-time whatever
    // the layout, so the ratio says "bulk, not field-at-a-time" without depending on how many
    // records happened to land adjacent — the property the test is actually about. Observed
    // 16x..29x across layouts; 8x leaves a wide margin and still fails hard on the defect,
    // where the two routes ARE the same route and the ratio is 1x.
    assert!(
        fast_reads * 8 < slow_reads,
        "a pointer-bearing element must relocate in bulk: {fast_reads} four-byte reads against \
         the word-at-a-time control's {slow_reads} — under 8x apart is the pre-fix route"
    );
    // The class boundary in absolute terms: 1000 elements, so one four-byte read EACH is 1000.
    // The defect cost ~3 each. Loose on purpose — it pins the CLASS, not today's count.
    assert!(
        fast_reads < 1000,
        "a pointer-bearing element must not cost a four-byte read each: {fast_reads} reads \
         for 1000 elements (wordwise control: {slow_reads})"
    );
}

/// loft#785 — a prefetched page must still be resident when the walk reads it.
///
/// loft#782's `warm()` fetches a level's pages together, at the LRU back. Once the warmed
/// set outgrew the 64-page cache it evicted the pages the walk was ABOUT to read, and the
/// walk fetched them again: 162 scattered keys over an 87 MB store drew 88.6 MB for a
/// 19.9 MB working set, and a real consumer's viewport went 26.9 MB → 81.4 MB. A prefetch
/// that does not fit is worse than no prefetch — demand paging at least has locality, which
/// is why the binary before batching did not amplify on the same cache.
///
/// **`bytes_fetched` against `distinct` is the assertion**: equal exactly when nothing was
/// fetched twice. The values are asserted first, because a loader that drops data also
/// fetches fewer bytes and would otherwise read as a win.
///
/// The `LOFT_PAGE_CACHE_BYTES` arm is the control. It pins a cache below the working set,
/// which must still amplify — someone who names a bound gets it, and re-fetching is the
/// cost of naming it. Without that arm a green here could mean the meter never moves.
#[test]
fn a_prefetched_page_is_resident_when_it_is_read() {
    let dir = scratch("page_cache_amplification_785");
    let script = workspace_root().join("tests/scripts/785-page-cache-amplification.loft");
    let path = dir.join("p");

    let run = |mode: &str, cap: Option<&str>| -> String {
        let mut cmd = loft::platform::process::harness_command(loft_bin());
        cmd.arg("--interpret")
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", &path)
            .env("LOFT_LOADER_STATS", "1")
            .env("LOFT_TIMEOUT", "180")
            .current_dir(workspace_root());
        if !mode.is_empty() {
            cmd.env("P785_MODE", mode);
        }
        if let Some(bytes) = cap {
            cmd.env("LOFT_PAGE_CACHE_BYTES", bytes);
        }
        let out = cmd.output().expect("failed to invoke loft binary");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "mode={mode} cap={cap:?}: {all}");
        assert!(!all.contains("FAIL"), "mode={mode}: {all}");
        all
    };

    /// `key=N` out of the loader-stats line.
    fn stat(out: &str, key: &str) -> u64 {
        let at = out
            .find(&format!("{key}="))
            .unwrap_or_else(|| panic!("loader stats must report `{key}`: {out}"))
            + key.len()
            + 1;
        let tail = &out[at..];
        let end = tail
            .find(|c: char| !c.is_ascii_digit())
            .unwrap_or(tail.len());
        tail[..end].parse().expect("a numeric stat")
    }
    fn result(out: &str) -> &str {
        out.lines()
            .find(|l| l.starts_with("loaded="))
            .expect("the script must report its load")
    }

    run("write", None);
    let grown = run("", None);
    let capped = run("", Some("4194304"));

    // Correctness FIRST, and identical under both policies: residency is a performance
    // choice, never a correctness one.
    assert_eq!(
        result(&grown),
        result(&capped),
        "how many pages stay resident must not change what a load produces"
    );
    assert!(
        result(&grown).contains("loaded=120") && result(&grown).contains("verify=true"),
        "every key must arrive and the copied heap must be sound: {}",
        result(&grown)
    );

    let (fetched, distinct) = (stat(&grown, "bytes_fetched"), stat(&grown, "distinct"));
    assert_eq!(
        fetched, distinct,
        "a page was fetched twice: {fetched} bytes fetched for a {distinct}-byte working \
         set. The prefetch evicted what it was about to read (loft#785)."
    );

    // The control — a cache pinned below the working set MUST still amplify, or this test
    // is measuring a meter that cannot move.
    let (cap_fetched, cap_distinct) = (stat(&capped, "bytes_fetched"), stat(&capped, "distinct"));
    assert!(
        cap_fetched > cap_distinct,
        "the capped control must re-fetch ({cap_fetched} vs {cap_distinct}) — if a 4 MB \
         cache holds this working set, the fixture no longer exercises the defect"
    );
}

/// loft#782 — the ROUND-TRIP DEPTH harness: `store_load_keys` issues its ranges serially.
///
/// `Stores::load_keys` is `for &kv in keys_vals { load_one(…) }`, and under it
/// `PageProvider::fetch` is synchronous and single-range. So a keyed load costs one
/// sequential round trip per page it discovers it needs, and over a real link the round
/// trip is the entire cost — the consumer that filed this measured a 1-byte range and a
/// 64 kB range at the SAME ~45 ms, with 764 serial reads making a cold viewport's 16–26 s
/// wait. It is depth, not bytes.
///
/// **This test is the gate, not the fix.** It exists so a batching change can be proved
/// rather than asserted, and it is written to measure the two things such a change moves:
///
/// - `requests` — how many round trips the load takes. Deterministic, and the real metric.
/// - wall time under injected per-request latency — the consequence, and the reason to care.
///
/// The server handles each connection on its own thread deliberately: with the
/// single-threaded one a concurrent client would be serialised by the socket, and this
/// harness would report "no improvement" for a fix that worked. That failure mode is worth
/// more care than the assertion itself.
///
/// What it pins today is the SHAPE: with N keys the load takes multiple round trips, and
/// wall time tracks `requests × latency`. When phase batching lands, the depth becomes
/// roughly constant in N and this test tightens to say so.
#[test]
fn store_load_keys_round_trip_depth_is_measurable() {
    let dir = scratch("load_keys_depth_782");
    let path = dir.join("world.store");
    let (out_w, code_w) = run_mode(&load_script(), &path, "write");
    assert_eq!(code_w, 0, "write: {out_w:?}");

    let sidecar = fa::read(format!("{}.dschema", path.display())).ok();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let delay_ms = 20;
    let (url, served) = serve_ranges_tuned(
        fa::read(&path).unwrap(),
        sidecar,
        delay_ms,
        Some(std::sync::Arc::clone(&hits)),
    );

    // Run the binary directly rather than through `run_mode_backend`: the loader stats —
    // and `depth=`, the number this test exists for — go to stderr, which that helper
    // discards.
    let started = std::time::Instant::now();
    let raw = loft::platform::process::harness_command(loft_bin())
        .arg("--interpret")
        .arg(load_script())
        .env("LOFT_PERSIST_TEST_PATH", &url)
        .env("LOFT_PERSIST_TEST_MODE", "loadkey")
        .env("LOFT_LOADER_STATS", "1")
        .env("LOFT_TIMEOUT", "180")
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    let elapsed = started.elapsed();
    let out = format!(
        "{}{}",
        String::from_utf8_lossy(&raw.stdout),
        String::from_utf8_lossy(&raw.stderr)
    );
    let code = raw.status.code().unwrap_or(-1);
    assert_eq!(code, 0, "http loadkey exit: {out:?}");
    assert!(
        out.contains("loadkey verify=true"),
        "the working set fetched over http must be sound: {out:?}"
    );

    let depth: u64 = out
        .lines()
        .find_map(|l| l.split("depth=").nth(1))
        .and_then(|t| t.split_whitespace().next())
        .and_then(|t| t.parse().ok())
        .expect("loader stats must report round-trip depth");
    let requests = served.load(std::sync::atomic::Ordering::Relaxed);
    // Report the measurement: this test's job is to make depth VISIBLE, so the numbers
    // belong in the output whether it passes or fails.
    eprintln!(
        "loft#782 depth harness: depth={depth} requests={requests} latency={delay_ms}ms \
         wall={elapsed:?}"
    );
    // DEPTH is the metric, not the range count. A batch of 27 issued together and 27 issued
    // one after another look identical to `requests`, and are the difference between 15 ms
    // and 400 ms — which is why the loader reports both and this asserts on the former.
    assert!(
        depth as usize <= requests,
        "round-trip depth {depth} exceeds the {requests} ranges served — impossible unless \
         the counter is wrong"
    );
    // The harness must actually be measuring something: no requests means the store came
    // from somewhere else and every number below is meaningless.
    assert!(
        requests > 0,
        "the load issued no HTTP requests — this harness is measuring nothing"
    );
    // Latency is REACHING the client: with requests served serially, the floor is
    // `requests × delay`. Generous (half) because a warm page costs nothing and the
    // request count includes the sidecar; the point is that depth is visible in the clock,
    // not a precise timing claim.
    let floor = std::time::Duration::from_millis(delay_ms * (requests as u64) / 2);
    assert!(
        elapsed >= floor,
        "wall time {elapsed:?} is below {floor:?} for {requests} requests at {delay_ms}ms — \
         the injected latency is not reaching the loader, so this harness cannot show a \
         batching win either"
    );
}

/// loft#790 — `store_verify` on a collection declared as a struct FIELD reports
/// what is true.
///
/// A store knows ONE type: the record allocated into it. For a field collection
/// that record is the WRAPPER, so the walk read the hash root as a `Firm` and
/// reported a corruption that was not there. The two calls are told apart by
/// `pos` — a ref to the record starts at the payload, a ref to a field carries
/// that field's byte offset — which is why the fix needs no guess about which
/// collection a wrapper meant, and why the two-keyed-field case is here.
///
/// Nothing in the script is lazy or paged: ordinary inserts, so a regression
/// cannot hide behind a loader.
#[test]
fn store_verify_reads_a_field_collection_as_itself_both_backends() {
    let script = workspace_root().join("tests/scripts/790-verify-field-collection.loft");
    for backend in ["--interpret", "--native-rust"] {
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(&script)
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        if !out.status.success() {
            eprintln!(
                "{backend} stderr:\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        assert!(out.status.success(), "{backend}: {stdout}");
        assert!(
            stdout.contains("field=true owner=true"),
            "{backend}: a sound field collection and its owner must both verify: {stdout}"
        );
        assert!(
            stdout.contains("top=true"),
            "{backend}: the top-level case was always right and must stay right: {stdout}"
        );
        assert!(
            stdout.contains("two=true,true,true"),
            "{backend}: two keyed fields in one wrapper resolve by OFFSET, so \
             neither is guessed: {stdout}"
        );
        // The symptom was a `[cr-check]` line naming an absurd bucket rec —
        // `1701667649` is `eman`, the wrapper's `name` text read as a pointer.
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            !stderr.contains("cr-check"),
            "{backend}: no structural complaint may be printed: {stderr}"
        );
    }
}

/// @PLN136 — a SPATIAL image, written laid out and read a BOX at a time.
///
/// The trie's twin, and deliberately a separate cell: the two kinds share the tree
/// and nothing above it. A prefix is one contiguous run of the key order and a box
/// over a Morton code is several disjoint ones, so what @PLN134 proved about
/// `store_load_prefix` says nothing about a viewport — and this is the query that
/// motivated flipping `type_is_compactable`'s `Radix` arm.
///
/// Bounded the same way, against the same trap: [`SPATIAL_AS_BUILT_REQUESTS`] is
/// what the box query fetched from an image of the records where they were
/// inserted, and the cell FAILS on an implementation that writes a faithful byte
/// copy and calls it laid out, which would answer every box correctly and fetch
/// exactly as much.
///
/// The containment half is the query script's: it checks every record it got back
/// is inside the viewport, because a walk that read the Morton interval and forgot
/// to filter returns MORE records, not fewer, and a count would not notice.
#[test]
fn a_spatial_survives_the_rebuild_and_comes_out_in_morton_order() {
    let script = workspace_root().join("tests/scripts/136-persist-copy-morton-order.loft");
    let query = workspace_root().join("tests/scripts/136-persist-copy-box-query.loft");

    for backend in ["--interpret", "--native-rust"] {
        let tag = backend.trim_start_matches('-');
        let dir = scratch(&format!("persist_copy_spatial_{tag}"));
        let bound = dir.join("bound.store");
        let copied = dir.join("copied.store");

        let run = |s: &Path, mode: &str, img: &Path| -> String {
            let out = loft::platform::process::harness_command(loft_bin())
                .arg(backend)
                .arg(s)
                .env("LOFT_PERSIST_TEST_PATH", &bound)
                .env("LOFT_PERSIST_TEST_COPY", &copied)
                .env("LOFT_PERSIST_TEST_MODE", mode)
                .env("LOFT_PERSIST_TEST_IMG", img)
                .env("LOFT_LOADER_STATS", "1")
                // 512-byte pages, as the trie's paged tests use: at 64 KB this
                // corpus is a handful of pages whole, so every layout reads the
                // same few and the cell could not tell a walk from a scan. The
                // effect under test is scatter, and scatter needs pages to
                // scatter ACROSS.
                .env("LOFT_PAGE_BYTES", "512")
                .current_dir(workspace_root())
                .output()
                .expect("failed to invoke loft binary");
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            assert!(
                out.status.success() && !stdout.contains("FAIL"),
                "{backend} mode={mode}: {stdout}\n{stderr}"
            );
            format!("{stdout}{stderr}")
        };

        let wrote = run(&script, "write", &bound);
        assert!(
            wrote.contains("held ok="),
            "{backend}: the reference held across the copy must still read its own \
             value — the rebuild goes into a scratch store, never the live one:\n{wrote}"
        );

        // The copy must be the same collection, read back in a separate process.
        let read = run(&script, "read", &copied);
        for key in ["all count=", "town=", "strip=", "gap="] {
            let of = |out: &str| -> String {
                out.lines()
                    .find(|l| l.contains(key))
                    .unwrap_or_else(|| panic!("no `{key}` in:\n{out}"))
                    .split_once(' ')
                    .map(|(_, r)| r.to_string())
                    .unwrap_or_default()
            };
            assert_eq!(
                of(&wrote),
                of(&read),
                "{backend}: `{key}` differs between the live collection and its \
                 copy — a rebuild that dropped or reordered records would still \
                 satisfy any assertion about SIZE:\n{wrote}{read}"
            );
        }

        // The layout half. `requests` is what the reader actually fetched.
        let requests = |out: &str| -> u32 {
            out.lines()
                .filter(|l| l.contains("store_load_box:"))
                .filter_map(|l| {
                    l.split_whitespace()
                        .find_map(|t| t.strip_prefix("requests="))
                        .and_then(|n| n.parse::<u32>().ok())
                })
                .sum()
        };
        let morton = requests(&run(&query, "query", &copied));
        assert!(
            morton > 0,
            "{backend}: the image must actually be queried — a run that fetched \
             nothing would pass the bound below vacuously"
        );
        assert!(
            morton * 2 <= SPATIAL_AS_BUILT_REQUESTS,
            "{backend}: a Morton-ordered image must fetch substantially less than \
             the image of the records as inserted ({SPATIAL_AS_BUILT_REQUESTS} \
             requests). The cell asserts only 2x so it survives a changed layout \
             without going quiet about a regression. morton={morton}"
        );
    }
}

/// What the trie prefix query of [`a_trie_survives_the_rebuild_and_comes_out_in_key_order`]
/// fetched, at 512-byte pages, from an image of the collection's live bytes — records
/// where they were inserted (laid out: 63).  No writer produces such an image now, so the
/// baseline is this measured constant; it moves only with the corpus or the page size.
const TRIE_AS_BUILT_REQUESTS: u32 = 208;

/// The same for the box query of
/// [`a_spatial_survives_the_rebuild_and_comes_out_in_morton_order`] (laid out: 42).
const SPATIAL_AS_BUILT_REQUESTS: u32 = 129;

/// @PLN134 — `store_persist_copy` writes an image a prefix query can PAGE, and
/// leaves every reference into the live collection valid.
///
/// The layout half cannot be asserted from inside the program: what a query
/// fetches is a property of the FILE, so this counts the requests a 20-record
/// prefix query makes against the image, and compares them with
/// [`TRIE_AS_BUILT_REQUESTS`], what the same query fetched from an image of the
/// records where they were inserted.  It FAILS on the implementation most likely
/// to be written by accident — one that writes a faithful byte copy and calls it
/// laid out, which would answer every key correctly and fetch exactly as much.
///
/// The reference half is the script's (`held ok=`): a rebuild that moved the
/// records the caller still holds would print a plausible number, so the script
/// checks the VALUE, not the absence of a crash.
#[test]
fn a_trie_survives_the_rebuild_and_comes_out_in_key_order() {
    let script = workspace_root().join("tests/scripts/134-persist-copy-key-order.loft");
    let query = workspace_root().join("tests/scripts/134-persist-copy-query.loft");

    for backend in ["--interpret", "--native-rust"] {
        let tag = backend.trim_start_matches('-');
        let dir = scratch(&format!("persist_copy_{tag}"));
        let bound = dir.join("bound.store");
        let copied = dir.join("copied.store");

        let run = |s: &Path, mode: &str, img: &Path| -> String {
            let out = loft::platform::process::harness_command(loft_bin())
                .arg(backend)
                .arg(s)
                .env("LOFT_PERSIST_TEST_PATH", &bound)
                .env("LOFT_PERSIST_TEST_COPY", &copied)
                .env("LOFT_PERSIST_TEST_MODE", mode)
                .env("LOFT_PERSIST_TEST_IMG", img)
                .env("LOFT_LOADER_STATS", "1")
                // 512-byte pages, as the trie's other paged tests use: at 64 KB
                // this corpus is ~5 pages whole, so every layout reads the same
                // handful and the cell could not tell a descent from a scan.
                // The effect under test is scatter, and scatter needs pages to
                // scatter ACROSS.
                .env("LOFT_PAGE_BYTES", "512")
                .current_dir(workspace_root())
                .output()
                .expect("failed to invoke loft binary");
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            assert!(
                out.status.success() && !stdout.contains("FAIL"),
                "{backend} mode={mode}: {stdout}\n{stderr}"
            );
            format!("{stdout}{stderr}")
        };

        let wrote = run(&script, "write", &bound);
        assert!(
            wrote.contains("held ok="),
            "{backend}: the reference held across the copy must still read its own \
             value — the rebuild goes into a scratch store, never the live one:\n{wrote}"
        );

        // The copy must be the same collection, read back in a separate process.
        let read = run(&script, "read", &copied);
        for key in ["count=", "aa=", "zusloot=", "arkerk=", "absent="] {
            let of = |out: &str| -> String {
                out.lines()
                    .find(|l| l.contains(key))
                    .unwrap_or_else(|| panic!("no `{key}` in:\n{out}"))
                    .split_once(' ')
                    .map(|(_, r)| r.to_string())
                    .unwrap_or_default()
            };
            assert_eq!(
                of(&wrote),
                of(&read),
                "{backend}: `{key}` differs between the live collection and its \
                 copy — a rebuild that dropped or reordered records would still \
                 satisfy any assertion about SIZE:\n{wrote}{read}"
            );
        }

        // The layout half. `requests` is what the reader actually fetched.
        let requests = |out: &str| -> u32 {
            out.lines()
                .filter(|l| l.contains("store_load_prefix:"))
                .filter_map(|l| {
                    l.split_whitespace()
                        .find_map(|t| t.strip_prefix("requests="))
                        .and_then(|n| n.parse::<u32>().ok())
                })
                .sum()
        };
        let key_order = requests(&run(&query, "query", &copied));
        assert!(
            key_order > 0,
            "{backend}: the image must actually be queried — a run that fetched \
             nothing would pass the bound below vacuously"
        );
        assert!(
            key_order * 2 <= TRIE_AS_BUILT_REQUESTS,
            "{backend}: a key-ordered image must fetch substantially less than the \
             image of the records as inserted ({TRIE_AS_BUILT_REQUESTS} requests). \
             The cell asserts only 2x so it survives a changed node layout without \
             going quiet about a regression. key-order={key_order}"
        );
    }
}

/// loft#1064 — `store_load_keys_text`: the text-keyed BATCH loader, and the reason
/// a batch form exists at all.
///
/// The paged loaders shipped a single form and a batch form, and the batch form was
/// there for a measured reason — one reader, one page cache, so a ring is one
/// traversal rather than N. Only the INTEGER-keyed side had it. Asset packs are
/// text-keyed by nature (`"page/mobs"`, `"hit.ogg"`), so the case that most wants the
/// batch form was the one that could not ask for it, and the workaround — loop
/// `store_load_key_text` — re-opened a reader per key and re-fetched the same
/// 64 KiB bucket-table page every time.
///
/// **The values are asserted first, and must be IDENTICAL between the two routes.**
/// A loader that drops entries also fetches fewer bytes, and would read as a win.
/// Only then does the cell compare cost: on this corpus the loop draws 5.37 MB in
/// 82 requests and the batch 256 KB in 4 — 20x — against a 447 KB file the loop
/// therefore downloads twelve times over. The assertion asks for 4x, so a smaller
/// corpus or a changed page size cannot turn a regression quiet.
#[test]
fn store_load_keys_text_batches_what_the_loop_repeats() {
    let dir = scratch("load_keys_text_1064");
    let script = workspace_root().join("tests/scripts/1064-load-keys-text.loft");
    let path = dir.join("pack");

    let run = |backend: &str, mode: &str| -> String {
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", &path)
            .env("P1064_MODE", mode)
            .env("LOFT_LOADER_STATS", "1")
            .env("LOFT_TIMEOUT", "240")
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "{backend} {mode}: {all}");
        assert!(!all.contains("FAIL"), "{backend} {mode}: {all}");
        all
    };

    /// Sum one loader-stats field over every line naming `call` — the loop emits one
    /// line per key, the batch one line in total, which is the difference being measured.
    fn stat_sum(out: &str, call: &str, field: &str) -> u64 {
        out.lines()
            .filter(|l| l.contains(call))
            .filter_map(|l| {
                l.split_whitespace()
                    .find_map(|t| t.strip_prefix(&format!("{field}=")))
                    .and_then(|n| n.parse::<u64>().ok())
            })
            .sum()
    }
    fn answer(out: &str) -> &str {
        out.lines()
            .find(|l| l.starts_with("loaded="))
            .expect("the script must report its load")
    }

    run("--interpret", "write");

    for backend in ["--interpret", "--native-rust"] {
        let batch = run(backend, "batch");
        let looped = run(backend, "loop");

        // Correctness first, and identical: 24 of the 25 requested keys are in the
        // pack, the 25th is absent and must be SKIPPED rather than counted or raised.
        assert_eq!(
            answer(&batch),
            answer(&looped),
            "{backend}: the batch form must produce exactly what the loop does"
        );
        assert_eq!(
            answer(&batch),
            "loaded=24 chars=1282 len=24 verify=true",
            "{backend}: 24 present keys load with their payloads, the absent key is \
             skipped, and the copied heap is sound: {batch}"
        );

        // The cost half — the whole point of the call.
        let (b_bytes, l_bytes) = (
            stat_sum(&batch, "store_load_keys_text:", "bytes_fetched"),
            stat_sum(&looped, "store_load_key_text:", "bytes_fetched"),
        );
        let (b_reqs, l_reqs) = (
            stat_sum(&batch, "store_load_keys_text:", "requests"),
            stat_sum(&looped, "store_load_key_text:", "requests"),
        );
        assert!(
            b_bytes > 0 && l_bytes > 0 && b_reqs > 0 && l_reqs > 0,
            "{backend}: both routes must actually have fetched something — a run that \
             read nothing would satisfy the ratio below vacuously \
             (batch {b_bytes}B/{b_reqs}req, loop {l_bytes}B/{l_reqs}req)"
        );
        assert!(
            b_bytes * 4 <= l_bytes,
            "{backend}: one reader must fetch substantially less than N readers. \
             Measured 262144 against 5373952 (20x); this asks 4x. \
             batch={b_bytes} loop={l_bytes}"
        );
        assert!(
            b_reqs * 4 <= l_reqs,
            "{backend}: one reader must make substantially fewer requests than N \
             readers. Measured 4 against 82 (20x); this asks 4x. \
             batch={b_reqs} loop={l_reqs}"
        );
    }
}

/// loft#1064 — the batch form serves a TRIE as well as a hash, the way the single
/// `store_load_key_text` already did.
///
/// A trie locates by DESCENT, so unlike a hash there are no addresses to batch: the
/// win is the shared page cache alone, and correctness is what this cell asserts.
/// A key the trie does not hold must stay absent — a descent lands on SOME leaf, so
/// a walk that skipped the key comparison would answer it.
#[test]
fn store_load_keys_text_serves_a_trie() {
    let dir = scratch("load_keys_text_trie_1064");
    let script = workspace_root().join("tests/scripts/1064-load-keys-text.loft");
    let path = dir.join("pack");

    for backend in ["--interpret", "--native-rust"] {
        let _ = fa::remove_file(dir.join("pack.trie"));
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", &path)
            .env("P1064_MODE", "trie")
            .env("LOFT_TIMEOUT", "240")
            .current_dir(workspace_root())
            .output()
            .expect("failed to invoke loft binary");
        let all = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "{backend}: {all}");
        assert!(
            all.contains("trie loaded=2 len=2 got=kerkweg:five lonneker:seven verify=true"),
            "{backend}: the two present keys load in key order, `kerks` stays absent, \
             and the trie the loader BUILT is sound: {all}"
        );
    }
}

/// `@FR-Col-Bucket` (@C146) — a hash table's bucket count is a power of two, and the home
/// bucket stays `digest mod count` on every table, so a store written under either sizing
/// reads and grows under both.  Each of the four runs writes 2000 entries, reads them back
/// and binds the file again to grow it to 4000; `LOFT_NO_POW2_BUCKETS=1` writes (or reads)
/// with the old sizes, a count that is not a power of two.  A reader that took the mask for
/// such a table would look in the wrong bucket and find fewer than it wrote.
#[test]
fn a_hash_written_under_either_bucket_sizing_reads_under_both() {
    let script =
        workspace_root().join("tests/scripts/a-hash-table-has-a-power-of-two-bucket-count.loft");
    for (old_write, old_read) in [(false, false), (false, true), (true, false), (true, true)] {
        let dir = scratch(&format!("pow2_buckets_{old_write}_{old_read}"));
        let path = dir.join("t.store");
        let run = |mode: &str, old: bool| -> String {
            let mut cmd = loft::platform::process::harness_command(loft_bin());
            cmd.arg("--interpret")
                .arg(&script)
                .env("LOFT_PERSIST_TEST_PATH", &path)
                .env("P_BUCKET_MODE", mode)
                .env("LOFT_TIMEOUT", "240")
                .env_remove("LOFT_NO_POW2_BUCKETS")
                .current_dir(workspace_root());
            if old {
                cmd.env("LOFT_NO_POW2_BUCKETS", "1");
            }
            let out = cmd.output().expect("failed to invoke loft binary");
            let all = String::from_utf8_lossy(&out.stdout).into_owned();
            assert!(
                out.status.success(),
                "{mode} old={old}: {all}{}",
                String::from_utf8_lossy(&out.stderr)
            );
            all.trim().to_string()
        };
        let cell = format!("written old={old_write}, read old={old_read}");
        assert_eq!(run("write", old_write), "wrote=2000", "{cell}");
        assert_eq!(
            run("read", old_read),
            "found=2000 sum=1999000 len=2000",
            "{cell}"
        );
        assert_eq!(
            run("grow", old_read),
            "found=4000 sum=7998000 len=4000",
            "{cell}"
        );
    }
}

/// The hash layouts a store can hold, each as the WRITER's policy that makes it:
/// `(Col-Tag)` off (plain 4-byte slots, the layout a store written before it carries),
/// `(Col-Inline)` off (tagged, entries in the arena), and neither off (inline where the
/// element type owns its entries, tagged where a group shares them).
const HASH_LAYOUTS: [(&str, &[(&str, &str)]); 3] = [
    ("plain", &[("LOFT_NO_HASH_TAG", "1")]),
    ("tagged", &[("LOFT_NO_HASH_INLINE", "1")]),
    ("inline", &[]),
];

/// One run of the layout script: stdout, stderr and the exit code.
fn run_layouts(
    backend: &str,
    path: &Path,
    mode: &str,
    env: &[(&str, &str)],
) -> (String, String, i32) {
    let out = loft::platform::process::harness_command(loft_bin())
        .arg(backend)
        .arg(
            workspace_root()
                .join("tests/scripts/a-store-holds-every-hash-layout-side-by-side.loft"),
        )
        .env("LOFT_PERSIST_TEST_PATH", path)
        .env("LOFT_PERSIST_TEST_MODE", mode)
        .env("LOFT_TIMEOUT", "300")
        .env_remove("LOFT_NO_HASH_TAG")
        .env_remove("LOFT_NO_HASH_INLINE")
        .envs(env.iter().copied())
        .current_dir(workspace_root())
        .output()
        .expect("failed to invoke loft binary");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

/// The four digest lines the script prints for the world as written, every value
/// hand-computed: 240 of 300 keys (sum 7 × 36000), 150 of 200 names (3 × 15000), 100 group
/// records (11 × 5050), 10 kept (385).
fn layouts_written(tag: &str) -> String {
    format!(
        "{tag} own=240/240/252000 named=150/150/45000 grp=100 by_k=100/100/55550 keep=10/385 late=0/0\n\
         {tag} own35=null own36=252 own37=259 own600=null\n\
         {tag} n6=18 n7=21 n8=null n399=null\n\
         {tag} by64=704 by250=null keep10=100 late50=null\n"
    )
}

/// The same after the update: 460 keys (7 × (36000 − 12000 + 135150)), 250 names
/// (3 × (15000 − 10000 + 59900)), 250 group records (11 × 31375), 50 late texts of 141 bytes.
fn layouts_updated(tag: &str) -> String {
    format!(
        "{tag} own=460/460/1114050 named=250/250/194700 grp=250 by_k=250/250/345125 keep=10/385 late=50/141\n\
         {tag} own35=null own36=null own37=259 own600=4200\n\
         {tag} n6=18 n7=null n8=null n399=1197\n\
         {tag} by64=704 by250=2750 keep10=100 late50=s50\n"
    )
}

/// @FR-Col-Layouts @C152 — every hash layout lives beside the others in one store, and
/// the interpreter and the native build each read and write all of them.  For every writer
/// layout against every updater layout, with the backends crossed: the world is written
/// under one, loaded, grown, shrunk and given a new table under the other, and read back by
/// the first backend under the third policy.  The update's trace names what the store holds
/// after it, so the mix is measured, not assumed.
#[test]
fn every_hash_layout_lives_beside_the_others_in_one_store() {
    for (first, second) in [
        ("--interpret", "--native-rust"),
        ("--native-rust", "--interpret"),
    ] {
        for (wn, we) in HASH_LAYOUTS {
            for (un, ue) in HASH_LAYOUTS {
                let cell = format!("{wn} written {first}, {un} update {second}");
                let dir = scratch(&format!("layouts_{wn}_{un}_{}", &first[2..]));
                let path = dir.join("world.store");
                let (out, err, code) = run_layouts(first, &path, "write", we);
                assert_eq!(
                    (code, out.as_str()),
                    (0, layouts_written("write").as_str()),
                    "{cell}: write\n{err}"
                );
                let mut env = ue.to_vec();
                env.push(("LOFT_TRACE_HASH_LAYOUT", "1"));
                let (out, err, code) = run_layouts(second, &path, "update", &env);
                let want = format!(
                    "{}{}",
                    layouts_written("loaded"),
                    layouts_updated("updated")
                );
                assert_eq!(
                    (code, out.as_str()),
                    (0, want.as_str()),
                    "{cell}: update\n{err}"
                );
                // The new table takes the updater's layout, whatever the store held before.
                let made = match un {
                    "plain" => "made plain at",
                    "tagged" => "made tagged at",
                    _ => "made inline at",
                };
                assert!(
                    err.contains(made),
                    "{cell}: the new table must be {un}:\n{err}"
                );
                // A plain table the updater grows comes back tagged unless it writes plain.
                if wn == "plain" && un != "plain" {
                    assert!(
                        err.contains("grown plain -> tagged")
                            && err.contains("grown plain borrowing -> tagged borrowing"),
                        "{cell}: the grown plain tables must be rebuilt tagged:\n{err}"
                    );
                }
                let third = HASH_LAYOUTS
                    .iter()
                    .find(|(n, _)| *n != wn && *n != un)
                    .map_or(&[][..], |l| l.1);
                let (out, err, code) = run_layouts(first, &path, "read", third);
                let want = format!("{}read verify=true\n", layouts_updated("read"));
                assert_eq!(
                    (code, out.as_str()),
                    (0, want.as_str()),
                    "{cell}: read\n{err}"
                );
            }
        }
    }
}

/// loft#2237 — an image a build wrote in the key order before `@FR-E-NullArg` reached keyed
/// collections (a null `float` / `single` key above every value, a null `text` key after `""`)
/// is CONVERTED by a whole-image load: its sidecar carries no `order=1`
/// (`placement::KEY_ORDER`), so its layout identity differs, and the store upgrade files every
/// entry again in this build's order.  The images in `tests/golden/key-order-before/` were
/// written once by a build of origin/main 23bc61e47 and are never written again; the expected
/// lines are hand-derived from `write.loft` (main itself reads them as `3 2 4 1`, `2 1 3`,
/// `3 1 2`, `3 2 1` and first=0).  The C converts them the same way (`loft_rekey_walk`: the one
/// layout change it converts, STORE_MODEL D30).
#[test]
fn an_image_in_the_old_key_order_is_converted() {
    let fixture = workspace_root().join("tests/golden/key-order-before");
    let dir = scratch("key_order_before");
    let names = [
        "sorted-text",
        "sorted-float-desc",
        "sorted-single",
        "index-text",
        "index-float",
        "nest",
    ];
    for name in names {
        for ext in [".store", ".store.dschema"] {
            let f = format!("{name}{ext}");
            fa::copy(fixture.join(&f), dir.join(&f)).expect("copy the fixture");
        }
    }
    fa::copy(fixture.join("read.loft"), dir.join("read.loft")).expect("copy the reader");
    fa::copy(fixture.join("read_nest.loft"), dir.join("read_nest.loft")).expect("copy the reader");
    let want = "sorted-text true 2 3 4 1 | 2 3 1\n\
                sorted-float-desc true 1 3 2 | 2 3\n\
                sorted-single true 2 3 1 | 2\n\
                index-text true 2 3 1 | 2 3 1\n\
                index-float true first=999 n=301 ordered=true | 999 150\n";
    let script = dir.join("read.loft");
    // `nest`: every path the conversion walks (a vector's records, a record's sorted and index,
    // a hash's records); keys "b", null, "", "a" hold 1..4, so this build's order reads 2 3 4 1
    // and the null and "" lookups 2 and 3.  Its own program: see read_nest.loft.
    let nest = dir.join("read_nest.loft");
    let want_nest = "nest true r0:2341/2341/23/23 r1:2341/2341/23/23 r2:2341/2341/23/23 \
                     top:2341/2 box0:2341/2 box1:2341/2\n";
    // `--native` here is the Rust-runtime build: CI points `--native` at the C backend
    // (`LOFT_NATIVE_BACKEND=c`), which converts nothing and is checked below on its own.
    for backend in ["--interpret", "--native"] {
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(&script)
            .env("LOFT_PERSIST_TEST_PATH", &dir)
            .env_remove("LOFT_NATIVE_BACKEND")
            .current_dir(workspace_root())
            .output()
            .expect("run the reader");
        assert!(
            out.status.success(),
            "{backend}: the reader ran:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            want,
            "{backend}: the old images, converted"
        );
        let out = loft::platform::process::harness_command(loft_bin())
            .arg(backend)
            .arg(&nest)
            .env("LOFT_PERSIST_TEST_PATH", &dir)
            .env_remove("LOFT_NATIVE_BACKEND")
            .current_dir(workspace_root())
            .output()
            .expect("run the nested reader");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            want_nest,
            "{backend}: the nested image, converted:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    // The C converts the same images (`loft_rekey_walk`, STORE_MODEL D30): a whole load of an
    // image that differs only by the key order files every entry again in this build's order.
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--native-c")
        .arg(&script)
        .env("LOFT_PERSIST_TEST_PATH", &dir)
        .current_dir(workspace_root())
        .output()
        .expect("run the C reader");
    assert!(
        out.status.success(),
        "C: the reader ran:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        want,
        "C: the old images, converted"
    );
    let out = loft::platform::process::harness_command(loft_bin())
        .arg("--native-c")
        .arg(&nest)
        .env("LOFT_PERSIST_TEST_PATH", &dir)
        .current_dir(workspace_root())
        .output()
        .expect("run the C nested reader");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        want_nest,
        "C: the nested image, converted:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}
