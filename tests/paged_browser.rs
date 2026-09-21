// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
//! loft#678 — the working-set store loaders on the BROWSER target.
//!
//! `store_load_key` and friends were absent from `wasm32-unknown-unknown`: the whole
//! family sat behind the `remote-store` feature, whose `ureq` client cannot exist there,
//! so `loft --html` failed with `E0599: no method named load_key` pointing into generated
//! Rust. The capability now rides the same asyncify `fetch()` bridge
//! `store_load_url_trusted` uses, with `Range: bytes=a-b` GETs.
//!
//! Two things are asserted, and the second is the feature:
//!  1. the entry actually loads and the copied heap is sound (`store_verify`), and
//!  2. it read a small FRACTION of the file.
//!
//! Without (2) the test would pass over a silent fallback to a whole-file load — which is
//! precisely the outcome the consumer cannot afford (a phone paging a multi-GB block).

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn loft_bin() -> PathBuf {
    repo_root().join("target/release/loft")
}

fn which(cmd: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .is_ok_and(|o| o.status.success())
}

fn wasm32_installed() -> bool {
    Command::new("rustc")
        .args(["--print", "target-list"])
        .output()
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).contains("wasm32-unknown-unknown"))
        && repo_root()
            .join("target/wasm32-unknown-unknown/release/libloft.rlib")
            .exists()
}

/// Entries written to the fixture store (~4.5 MB). The size is load-bearing: a keyed
/// lookup costs a fixed handful of 64 KiB pages, so on a small image that constant IS
/// most of the file and a "read less than the whole thing" assertion passes for free.
const ENTRIES: usize = 40000;

/// The page budget a single keyed lookup may spend — the hash-root pages, the bucket it
/// probes, the entry, and its relocated text. Generous, but a CONSTANT: the design claim
/// is that cost depends on the lookup, not on how big the store is, so a whole-file
/// fallback (or a traversal that degrades with size) breaks this no matter the fixture.
const PAGE_BUDGET: u64 = 8 * 64 * 1024;

// @speed 1.0
#[test]
fn store_load_key_pages_over_the_browser_fetch_bridge() {
    if !which("node") {
        eprintln!("SKIP: node not installed");
        return;
    }
    if !wasm32_installed() {
        eprintln!("SKIP: wasm32-unknown-unknown target/rlib not built");
        return;
    }
    if !loft_bin().exists() {
        eprintln!("SKIP: target/release/loft not built");
        return;
    }

    let tmp = std::env::temp_dir().join("loft_paged_browser");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create test dir");
    let store = tmp.join("block.store");

    // 1. Write the fixture store natively.
    let writer = tmp.join("write.loft");
    std::fs::write(
        &writer,
        format!(
            "struct Tile {{ tkey: integer not null, name: text not null }}\n\
             fn main() {{\n\
             \x20 path = env_variable(\"LOFT_PAGED_WRITE_PATH\");\n\
             \x20 t: hash<Tile[tkey]> = [];\n\
             \x20 i = 0;\n\
             \x20 while i < {ENTRIES} {{ t += Tile {{ tkey: i, name: \"tile-{{i}}-padding-to-give-the-image-some-size\" }}; i += 1 }}\n\
             \x20 if !store_persist_bind(t, path) {{ println(\"FAIL write-bind\"); return }}\n\
             \x20 println(\"write ok len={{len(t)}}\");\n\
             }}\n"
        ),
    )
    .expect("write writer script");

    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&writer)
        .env("LOFT_PAGED_WRITE_PATH", &store)
        // Pin the hash seed, or this test measures a coin flip.  Bucket placement is
        // seed-dependent by design, the seed is drawn per store, and the number of
        // PAGES a lookup touches follows it — so the same fixture and the same lookup
        // cost a different number of pages on every run.  It sat far enough under the
        // budget not to show until @PLN135 arc H added the entry arena's directory hop,
        // and then it straddled: passing at seven pages, failing at ten, with nothing
        // changing between runs.  Widening the budget would only have made the coin
        // flip land the same way more often; pinning the seed removes the flip.
        .env("LOFT_HASH_SEED", "0x0123456789abcdef")
        .current_dir(repo_root())
        .output()
        .expect("invoke loft to write the store");
    let so = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        so.contains(&format!("write ok len={ENTRIES}")),
        "fixture store not written: {so} / {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let file_len = std::fs::metadata(&store).expect("store metadata").len();
    assert!(
        file_len > 4 * PAGE_BUDGET,
        "the fixture must be several times the page budget or the bounded-fetch \
         assertion below proves nothing; got {file_len} bytes"
    );

    // 2. Build the browser bundle. The URL is inert — the harness serves the bytes —
    //    but it must be `http://` so `PageSource` selects the range provider.
    let src = tmp.join("paged.loft");
    std::fs::write(
        &src,
        "struct Tile { tkey: integer not null, name: text not null }\n\
         fn main() {\n\
         \x20 tiles: hash<Tile[tkey]> = [];\n\
         \x20 ok = store_load_key(tiles, \"http://127.0.0.1:1/block.store\", 2718);\n\
         \x20 println(\"browser ok={ok} len={len(tiles)}\");\n\
         \x20 got = tiles[2718];\n\
         \x20 println(\"browser name={if got == null { \"null\" } else { got.name }}\");\n\
         \x20 println(\"browser verify={store_verify(tiles)}\");\n\
         }\n",
    )
    .expect("write browser script");

    let html = tmp.join("paged.html");
    let status = Command::new(loft_bin())
        .args([
            "--html",
            html.to_str().unwrap(),
            "--path",
            &format!("{}/", repo_root().display()),
        ])
        .arg(src.to_str().unwrap())
        .current_dir(repo_root())
        .status()
        .expect("invoke loft --html");
    assert!(
        status.success(),
        "loft --html must build a program that calls store_load_key (loft#678: it failed \
         with E0599 inside generated Rust)"
    );

    // 3. Extract the embedded wasm and run it against real ranges.
    let page = std::fs::read_to_string(&html).expect("read html");
    let marker = "const wasmB64=\"";
    let start = page.find(marker).expect("wasmB64 marker") + marker.len();
    let end = start + page[start..].find('"').expect("wasmB64 closing quote");
    let wasm = tmp.join("paged.wasm");
    std::fs::write(&wasm, loft::base64::decode(&page[start..end])).expect("write wasm");

    let run = Command::new("node")
        .arg(repo_root().join("tools/paged_range_host.mjs"))
        .arg(&wasm)
        .env("LOFT_PAGED_FILE", &store)
        .current_dir(repo_root())
        .output()
        .expect("invoke node harness");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    assert!(
        run.status.success(),
        "browser run failed\n  stdout:\n{stdout}\n  stderr:\n{stderr}"
    );

    // The entry arrived, intact, and the copied heap is structurally sound.
    assert!(
        stdout.contains("browser ok=true"),
        "the keyed entry must load in the browser\n  stdout:\n{stdout}\n  stderr:\n{stderr}"
    );
    assert!(
        stdout.contains("browser ok=true len=1"),
        "a working-set load must bring ONE entry, not the whole hash\n  stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("browser name=tile-2718-padding-to-give-the-image-some-size"),
        "the relocated text field must survive the paged copy\n  stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("browser verify=true"),
        "the paged copy must produce a structurally sound heap\n  stdout:\n{stdout}"
    );

    // …and it PAGED. This is the assertion the feature exists for.
    let paged = stdout
        .lines()
        .find(|l| l.starts_with("PAGED "))
        .unwrap_or_else(|| panic!("harness must report its fetch total\n{stdout}"));
    let field = |k: &str| -> u64 {
        paged
            .split_whitespace()
            .find_map(|f| f.strip_prefix(k))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("malformed harness line: {paged}"))
    };
    let fetched = field("bytes_fetched=");
    let total = field("file=");
    let requests = field("requests=");
    assert!(
        requests > 0,
        "the range bridge must actually be called — 0 requests means the loader never \
         reached it: {paged}"
    );
    assert_eq!(
        total, file_len,
        "the harness must be serving the fixture it was given: {paged}"
    );
    assert!(
        fetched <= PAGE_BUDGET,
        "a keyed lookup must cost a BOUNDED number of pages, independent of image size \
         — {fetched} bytes of a {total}-byte store exceeds the {PAGE_BUDGET}-byte budget, \
         which is what a silent whole-file fallback looks like: {paged}"
    );
}

/// loft#784 — the BATCHED loader over the same bridge.
///
/// `store_load_key` above never exercises this: one key resolves to one range at a time, and
/// `fetch_many` returns early on a single range. So when loft#782 gave the multi-range path
/// its own implementation — one `std::thread::spawn` per range — the browser gate stayed
/// green while the browser stopped working. A target with no threads never completed the
/// read, and it failed as a STALL: no error, no trap, a consumer's app sitting at
/// `hud="loading map…"` forever.
///
/// Hence the two shapes of this test. It uses `store_load_keys` with keys spread across the
/// image so the loader has genuinely independent ranges to issue, and it runs the browser
/// under a hard `timeout` — because the failure being guarded against is a hang, and a hang
/// in a test that waits forever takes the whole suite with it instead of reporting.
// @speed 0.9
#[test]
fn store_load_keys_batched_pages_over_the_browser_fetch_bridge() {
    if !which("node") {
        eprintln!("SKIP: node not installed");
        return;
    }
    if !which("timeout") {
        eprintln!("SKIP: coreutils `timeout` not installed — a hang could not be bounded");
        return;
    }
    if !wasm32_installed() {
        eprintln!("SKIP: wasm32-unknown-unknown target/rlib not built");
        return;
    }
    if !loft_bin().exists() {
        eprintln!("SKIP: target/release/loft not built");
        return;
    }

    let tmp = std::env::temp_dir().join("loft_paged_browser_batch");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create test dir");
    let store = tmp.join("block.store");

    // 1. The same fixture as the single-key gate: big enough that a bounded read is a real
    //    claim rather than an artefact of a small image.
    let writer = tmp.join("write.loft");
    std::fs::write(
        &writer,
        format!(
            "struct Tile {{ tkey: integer not null, name: text not null }}\n\
             fn main() {{\n\
             \x20 path = env_variable(\"LOFT_PAGED_WRITE_PATH\");\n\
             \x20 t: hash<Tile[tkey]> = [];\n\
             \x20 i = 0;\n\
             \x20 while i < {ENTRIES} {{ t += Tile {{ tkey: i, name: \"tile-{{i}}-padding-to-give-the-image-some-size\" }}; i += 1 }}\n\
             \x20 if !store_persist_bind(t, path) {{ println(\"FAIL write-bind\"); return }}\n\
             \x20 println(\"write ok len={{len(t)}}\");\n\
             }}\n"
        ),
    )
    .expect("write writer script");

    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&writer)
        .env("LOFT_PAGED_WRITE_PATH", &store)
        // Pin the hash seed, or this test measures a coin flip.  Bucket placement is
        // seed-dependent by design, the seed is drawn per store, and the number of
        // PAGES a lookup touches follows it — so the same fixture and the same lookup
        // cost a different number of pages on every run.  It sat far enough under the
        // budget not to show until @PLN135 arc H added the entry arena's directory hop,
        // and then it straddled: passing at seven pages, failing at ten, with nothing
        // changing between runs.  Widening the budget would only have made the coin
        // flip land the same way more often; pinning the seed removes the flip.
        .env("LOFT_HASH_SEED", "0x0123456789abcdef")
        .current_dir(repo_root())
        .output()
        .expect("invoke loft to write the store");
    let so = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        so.contains(&format!("write ok len={ENTRIES}")),
        "fixture store not written: {so} / {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let file_len = std::fs::metadata(&store).expect("store metadata").len();

    // 2. Six keys, spread across the key space so they cannot all share a page — the point
    //    is that the loader has several INDEPENDENT ranges to issue in one call.
    let src = tmp.join("paged_keys.loft");
    std::fs::write(
        &src,
        "struct Tile { tkey: integer not null, name: text not null }\n\
         fn main() {\n\
         \x20 tiles: hash<Tile[tkey]> = [];\n\
         \x20 keys = [137, 6011, 12907, 21001, 30011, 39002];\n\
         \x20 got = store_load_keys(tiles, \"http://127.0.0.1:1/block.store\", keys);\n\
         \x20 println(\"browser got={got} len={len(tiles)}\");\n\
         \x20 for k in keys { e = tiles[k]; println(\"browser k={k} name={if e == null { \"null\" } else { e.name }}\") }\n\
         \x20 println(\"browser verify={store_verify(tiles)}\");\n\
         }\n",
    )
    .expect("write browser script");

    let html = tmp.join("paged_keys.html");
    let status = Command::new(loft_bin())
        .args([
            "--html",
            html.to_str().unwrap(),
            "--path",
            &format!("{}/", repo_root().display()),
        ])
        .arg(src.to_str().unwrap())
        .current_dir(repo_root())
        .status()
        .expect("invoke loft --html");
    assert!(status.success(), "loft --html must build store_load_keys");

    let page = std::fs::read_to_string(&html).expect("read html");
    let marker = "const wasmB64=\"";
    let start = page.find(marker).expect("wasmB64 marker") + marker.len();
    let end = start + page[start..].find('"').expect("wasmB64 closing quote");
    let wasm = tmp.join("paged_keys.wasm");
    std::fs::write(&wasm, loft::base64::decode(&page[start..end])).expect("write wasm");

    // 3. Run it BOUNDED. `timeout` exits 124 when it has to kill the child, which is what
    //    the unconditional `thread::spawn` produced — and is why the assertion below names
    //    a stall specifically rather than reporting a generic non-zero exit.
    let run = Command::new("timeout")
        .arg("180")
        .arg("node")
        .arg(repo_root().join("tools/paged_range_host.mjs"))
        .arg(&wasm)
        .env("LOFT_PAGED_FILE", &store)
        .current_dir(repo_root())
        .output()
        .expect("invoke node harness under timeout");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    assert_ne!(
        run.status.code(),
        Some(124),
        "the batched browser read STALLED (killed at 180 s). A browser has no \
         `std::thread::spawn`, so a multi-range `fetch_many` never completes — loft#784.\n  \
         stdout:\n{stdout}\n  stderr:\n{stderr}"
    );
    assert!(
        run.status.success(),
        "browser run failed\n  stdout:\n{stdout}\n  stderr:\n{stderr}"
    );

    // Every key arrived, each carrying its own relocated text — a batch that silently
    // dropped ranges would still report a count, so the NAMES are what pins it.
    assert!(
        stdout.contains("browser got=6 len=6"),
        "all six keys must load in one batched call\n  stdout:\n{stdout}"
    );
    for k in [137, 6011, 12907, 21001, 30011, 39002] {
        assert!(
            stdout.contains(&format!(
                "browser k={k} name=tile-{k}-padding-to-give-the-image-some-size"
            )),
            "key {k} must arrive with its own text, not another key's\n  stdout:\n{stdout}"
        );
    }
    assert!(
        stdout.contains("browser verify=true"),
        "the batched paged copy must produce a structurally sound heap\n  stdout:\n{stdout}"
    );

    // …and it still PAGED. Six keys cost more than one, but nothing like the whole image.
    let paged = stdout
        .lines()
        .find(|l| l.starts_with("PAGED "))
        .unwrap_or_else(|| panic!("harness must report its fetch total\n{stdout}"));
    let field = |k: &str| -> u64 {
        paged
            .split_whitespace()
            .find_map(|f| f.strip_prefix(k))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("malformed harness line: {paged}"))
    };
    assert_eq!(
        field("file="),
        file_len,
        "the harness must be serving the fixture it was given: {paged}"
    );
    assert!(
        field("bytes_fetched=") <= 6 * PAGE_BUDGET,
        "six keyed lookups must stay a bounded multiple of one — a whole-file fallback \
         looks exactly like this assertion failing: {paged}"
    );
}

/// loft#678 sibling — `store_load_url` (the SHA-VERIFIED whole-image loader) was still
/// browser-unavailable after the paged family was bridged, for a reason that had nothing
/// to do with what it needs: its hash check lived in the `registry`-gated module, and a
/// gate is contagious. The trusted twin `store_load_url_trusted` had already been widened,
/// so the browser could fetch a whole store but could not PIN its hash — exactly the wrong
/// half to be missing on the target most likely to load third-party bytes.
///
/// Both directions are asserted in one run. Accepting a correct digest alone would pass on
/// a loader that verifies nothing at all, which is the failure that matters here.
#[test]
fn store_load_url_verifies_the_hash_in_the_browser() {
    if !which("node") {
        eprintln!("SKIP: node not installed");
        return;
    }
    if !wasm32_installed() {
        eprintln!("SKIP: wasm32-unknown-unknown target/rlib not built");
        return;
    }
    if !loft_bin().exists() {
        eprintln!("SKIP: target/release/loft not built");
        return;
    }

    let tmp = std::env::temp_dir().join("loft_verified_browser");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("create test dir");
    let store = tmp.join("v.store");

    let writer = tmp.join("write.loft");
    std::fs::write(
        &writer,
        "struct Rec { id: integer not null, val: integer not null }\n\
         fn main() {\n\
         \x20 path = env_variable(\"LOFT_VERIFIED_WRITE_PATH\");\n\
         \x20 h: hash<Rec[id]> = [];\n\
         \x20 h += Rec { id: 7, val: 700 };\n\
         \x20 h += Rec { id: 13, val: 1300 };\n\
         \x20 if !store_persist_bind(h, path) { println(\"FAIL write-bind\"); return }\n\
         \x20 println(\"write ok\");\n\
         }\n",
    )
    .expect("write writer script");
    let out = Command::new(loft_bin())
        .arg("--interpret")
        .arg(&writer)
        .env("LOFT_VERIFIED_WRITE_PATH", &store)
        .current_dir(repo_root())
        .output()
        .expect("invoke loft to write the store");
    assert!(
        String::from_utf8_lossy(&out.stdout).contains("write ok"),
        "fixture not written: {} / {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let bytes = std::fs::read(&store).expect("read fixture");
    let good = loft::integrity::sha256_hex(&bytes);
    let bad = "0".repeat(64);

    let stdout = run_url_program(
        &tmp,
        "verified",
        &format!(
            "struct Rec {{ id: integer not null, val: integer not null }}\n\
             fn main() {{\n\
             \x20 ok: hash<Rec[id]> = [];\n\
             \x20 a = store_load_url(ok, \"http://127.0.0.1:1/v.store\", \"{good}\");\n\
             \x20 println(\"good sha ok={{a}} len={{len(ok)}}\");\n\
             \x20 no: hash<Rec[id]> = [];\n\
             \x20 b = store_load_url(no, \"http://127.0.0.1:1/v.store\", \"{bad}\");\n\
             \x20 println(\"bad sha ok={{b}} len={{len(no)}}\");\n\
             }}\n"
        ),
        &store,
    );
    assert!(
        stdout.contains("good sha ok=true len=2"),
        "a matching digest must ADOPT the image in the browser\n  stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("bad sha ok=false len=0"),
        "a mismatched digest must REFUSE and adopt nothing — a loader that accepts \
         anything would still satisfy the match case above\n  stdout:\n{stdout}"
    );

    // loft#1562 — a program whose `Rec` grew a field is refused by BOTH whole-image loaders,
    // even with the image's own digest: the pin says the bytes are the ones published, and
    // the `.dschema` beside them says they were written with a layout this program does not
    // read.  The match half above now runs through that sidecar too, so the refusal here
    // is the layout and not a gate that refuses everything.
    let stdout = run_url_program(
        &tmp,
        "changed",
        &format!(
            "struct Rec {{ id: integer not null, extra: integer not null, val: integer not null }}\n\
             fn main() {{\n\
             \x20 a: hash<Rec[id]> = [];\n\
             \x20 x = store_load_url(a, \"http://127.0.0.1:1/v.store\", \"{good}\");\n\
             \x20 println(\"changed pinned ok={{x}} len={{len(a)}}\");\n\
             \x20 b: hash<Rec[id]> = [];\n\
             \x20 y = store_load_url_trusted(b, \"http://127.0.0.1:1/v.store\");\n\
             \x20 println(\"changed trusted ok={{y}} len={{len(b)}}\");\n\
             }}\n"
        ),
        &store,
    );
    assert!(
        stdout.contains("changed pinned ok=false len=0"),
        "a changed layout must be REFUSED in the browser even with a matching digest\n  \
         stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("changed trusted ok=false len=0"),
        "a changed layout must be REFUSED by the trusted loader in the browser\n  \
         stdout:\n{stdout}"
    );
}

/// Build `source` with `loft --html` and run it under `tools/paged_range_host.mjs` with
/// whole-file GETs enabled, serving `store` (and the `.dschema` beside it) for every URL.
/// Answers the program's stdout.
fn run_url_program(tmp: &Path, name: &str, source: &str, store: &Path) -> String {
    let src = tmp.join(format!("{name}.loft"));
    std::fs::write(&src, source).expect("write browser script");

    let html = tmp.join(format!("{name}.html"));
    let status = Command::new(loft_bin())
        .args([
            "--html",
            html.to_str().unwrap(),
            "--path",
            &format!("{}/", repo_root().display()),
        ])
        .arg(src.to_str().unwrap())
        .current_dir(repo_root())
        .status()
        .expect("invoke loft --html");
    assert!(
        status.success(),
        "loft --html must build a program that calls store_load_url — it failed with \
         E0599 on `load_url_verified` while only the trusted twin was bridged"
    );

    let page = std::fs::read_to_string(&html).expect("read html");
    let marker = "const wasmB64=\"";
    let start = page.find(marker).expect("wasmB64 marker") + marker.len();
    let end = start + page[start..].find('"').expect("wasmB64 closing quote");
    let wasm = tmp.join(format!("{name}.wasm"));
    std::fs::write(&wasm, loft::base64::decode(&page[start..end])).expect("write wasm");

    let run = Command::new("node")
        .arg(repo_root().join("tools/paged_range_host.mjs"))
        .arg(&wasm)
        .env("LOFT_PAGED_FILE", store)
        .env("LOFT_WHOLE_GET", "1")
        .current_dir(repo_root())
        .output()
        .expect("invoke node harness");
    let stdout = String::from_utf8_lossy(&run.stdout).into_owned();
    assert!(
        run.status.success(),
        "browser run failed\n  stdout:\n{stdout}\n  stderr:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );
    stdout
}
