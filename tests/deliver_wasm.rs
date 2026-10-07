// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN105 Phase 2c/2d — the deliver WHOLE-SLICE falsifier: a loft value delivered from a
// `--html`-built wasm is reconstructed IN JS, driven only by the layout descriptor, and equals
// the value the program delivered. This closes the parity gate interpret == native == --html:
// `deliver_parity.rs` pins interpret == native (the loopback bytes); this pins that the generic
// JS reader (doc/loft-deliver.js, the twin of `read_via_descriptor`) reproduces the same value in
// the browser target.
//
// Self-skips when the wasm toolchain is unavailable (node / wasm32 target / release binary) —
// same policy as tests/html_wasm.rs.

use loft::file_access as fa;
use std::path::PathBuf;

fn which(cmd: &str) -> bool {
    loft::platform::process::harness_command("sh")
        .arg("-c")
        .arg(format!("command -v {cmd}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn wasm32_installed() -> bool {
    loft::platform::process::harness_command("rustup")
        .args(["target", "list", "--installed"])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("wasm32-unknown-unknown"))
        .unwrap_or(false)
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Build `source` via `loft --html`, extract the embedded wasm, run it through the deliver node
/// harness, and return its stdout — or `None` if the toolchain self-skips.
fn run_deliver(name: &str, source: &str) -> Option<String> {
    run_deliver_env(name, source, &[])
}

/// [`run_deliver`], but sets extra environment variables on the node harness process (e.g.
/// `LOFT_DELIVER_GROW=1` to arm the memory.grow-safety cell).
fn run_deliver_env(name: &str, source: &str, env: &[(&str, &str)]) -> Option<String> {
    run_harness(name, source, "tools/deliver_repro.mjs", env)
}

/// [`run_deliver`], but through the asyncify-driven CROSS-FRAME harness
/// (`tools/deliver_crossframe.mjs`) — exposes a value, yields via a stubbed fetch, and reads the
/// exposed value back on the JS side AFTER the yield.
fn run_crossframe(name: &str, source: &str) -> Option<String> {
    run_harness(name, source, "tools/deliver_crossframe.mjs", &[])
}

/// [`run_deliver`], but through the ZERO-COPY perf harness (`tools/deliver_perf.mjs`) — measures
/// that an inline scalar vector delivers O(1) in n (a view aliasing wasm memory, not a copy).
fn run_perf(name: &str, source: &str) -> Option<String> {
    run_harness(name, source, "tools/deliver_perf.mjs", &[])
}

/// Build `source` via `loft --html`, extract the embedded wasm, run it through the given node
/// harness (`harness_rel`, repo-relative) with extra `env`, and return its stdout — or `None` if
/// the toolchain self-skips.
fn run_harness(
    name: &str,
    source: &str,
    harness_rel: &str,
    env: &[(&str, &str)],
) -> Option<String> {
    if !which("node") {
        eprintln!("SKIP: node not installed");
        return None;
    }
    if !wasm32_installed() {
        eprintln!("SKIP: wasm32-unknown-unknown target not installed");
        return None;
    }
    let loft_bin = repo_root().join("target/release/loft");
    if !fa::exists(&loft_bin) {
        eprintln!("SKIP: target/release/loft not built (run `cargo build --release`)");
        return None;
    }

    let tmp = std::env::temp_dir().join(format!("loft_deliver_{name}"));
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).expect("create per-test dir");
    let src = tmp.join(format!("{name}.loft"));
    let html = tmp.join(format!("{name}.html"));
    let wasm = tmp.join(format!("{name}.wasm"));
    fa::write(&src, source).expect("write source");

    let status = loft::platform::process::harness_command(&loft_bin)
        .args([
            "--html",
            html.to_str().unwrap(),
            "--path",
            &format!("{}/", repo_root().display()),
        ])
        .arg(src.to_str().unwrap())
        .status()
        .expect("invoke loft --html");
    assert!(status.success(), "loft --html failed for {name}");

    // Extract the embedded wasm (`const wasmB64="…"`), exactly as tests/html_wasm.rs does.
    let page = fa::read_to_string(&html).expect("read html");
    let marker = "const wasmB64=\"";
    let start = page.find(marker).expect("wasmB64 marker") + marker.len();
    let end = start + page[start..].find('"').expect("wasmB64 closing quote");
    fa::write(&wasm, loft::base64::decode(&page[start..end])).expect("write wasm");

    let harness = repo_root().join(harness_rel);
    assert!(fa::exists(&harness), "{harness_rel} missing");
    let out = loft::platform::process::harness_command("node")
        .arg(&harness)
        .arg(&wasm)
        .envs(env.iter().copied())
        .output()
        .expect("invoke node deliver harness");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "deliver harness failed for {name}\nstdout:{stdout}\nstderr:{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Some(stdout)
}

#[test]
fn deliver_reconstructs_nested_value_in_js() {
    // Exercises every node kind in the serializable subset the reader mirrors: text (interned),
    // a nested record, an i64 scalar, an f64 scalar, a scalar vector (the fast lane), and a bool.
    let src = r#"
struct Inner { a: integer, b: float }
struct Outer { name: text, inner: Inner, nums: vector<integer>, ok: boolean }
fn main() {
  o = Outer { name: "hi", inner: Inner { a: 7, b: 1.5 }, nums: [10, 20, 30], ok: true };
  deliver(1, o);
}
"#;
    let Some(stdout) = run_deliver("nested", src) else {
        return; // toolchain self-skip
    };
    // The JS reader must reconstruct the exact value the program delivered. Pin the tag + value,
    // NOT the internal type-id (which shifts with the stdlib).
    let want_value =
        "\"value\":{\"name\":\"hi\",\"inner\":{\"a\":7,\"b\":1.5},\"nums\":[10,20,30],\"ok\":true}";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value) && stdout.contains("\"tag\":1"),
        "reconstructed value mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_flattens_top_level_hash_in_js() {
    // @PLN105 Phase 3 slice — a top-level keyed `hash<T>` has no byte layout a reader can walk, so
    // deliver PRE-FLATTENS it (materialise to a scratch array of element records, the `for x in h`
    // path) and hands JS a synthetic `Array(elem)` descriptor. JS reconstructs the elements via the
    // existing array reader, KEY-SORTED, touching no hash-layout knowledge.
    let src = r#"
struct Item { ik: integer, name: text }
fn main() {
  h: hash<Item[ik]> = [];
  h[30] = Item { ik: 30, name: "thirty" };
  h[10] = Item { ik: 10, name: "ten" };
  h[20] = Item { ik: 20, name: "twenty" };
  deliver(1, h);
}
"#;
    let Some(stdout) = run_deliver("hash", src) else {
        return; // toolchain self-skip
    };
    // Inserted out of order (30,10,20); reconstructed KEY-SORTED (10,20,30) == the interpreter's
    // `for x in h` order (both use build_hash_sorted_vec).
    let want_value = "\"value\":[{\"ik\":10,\"name\":\"ten\"},{\"ik\":20,\"name\":\"twenty\"},{\"ik\":30,\"name\":\"thirty\"}]";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "flattened hash mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_flattens_nested_hash_field_in_js() {
    // @PLN105 Phase 3 — a struct with a keyed FIELD: the scalar/text fields are read in place, and
    // the `hash` field is flattened to a key-sorted array (a FlatArray descriptor node carrying the
    // materialised data record) — so JS reconstructs `{…scalars…, field: [elements]}` with no
    // hash-layout knowledge.
    let src = r#"
struct Item { ik: integer, name: text }
struct Bag { label: text, items: hash<Item[ik]>, count: integer }
fn main() {
  b: Bag = Bag { label: "mybag", items: [], count: 3 };
  b.items[20] = Item { ik: 20, name: "twenty" };
  b.items[10] = Item { ik: 10, name: "ten" };
  deliver(1, b);
}
"#;
    let Some(stdout) = run_deliver("nesthash", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":{\"label\":\"mybag\",\"items\":[{\"ik\":10,\"name\":\"ten\"},{\"ik\":20,\"name\":\"twenty\"}],\"count\":3}";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "nested-hash-field mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_flattens_top_level_spatial_in_js() {
    // @PLN105 Phase 3 — a `spatial`/radix collection: pre-flattened via build_radix_sorted_vec (the
    // same tree-walk `for x in xs` uses), so JS reconstructs the elements in NATURAL (Morton)
    // order, identical to the interpreter's iteration.
    let src = r#"
struct Pt { x: integer, y: integer }
fn main() {
  xs: spatial<Pt[x, y]> = [];
  xs += [Pt{x: 3, y: 3}];
  xs += [Pt{x: 1, y: 2}];
  xs += [Pt{x: 0, y: 0}];
  deliver(1, xs);
}
"#;
    let Some(stdout) = run_deliver("spatial", src) else {
        return; // toolchain self-skip
    };
    // Morton/Z-order (== the interpreter's `for m in xs`): (0,0), (1,2), (3,3).
    let want_value = "\"value\":[{\"x\":0,\"y\":0},{\"x\":1,\"y\":2},{\"x\":3,\"y\":3}]";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "flattened spatial mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_reads_top_level_sorted_in_js() {
    // @PLN105 Phase 3 — a `sorted<T>` is stored as an INLINE vector kept in key order
    // (`sorted_finish` inserts each `+=` in sorted position), so it needs NO materialisation: it is
    // reclassified to an in-place `Vector` node and read where it lives. Inserted {30,10,20} →
    // key-sorted [10,20,30] == the interpreter's `for it in s`.
    let src = r#"
struct Item { k: integer, name: text }
fn main() {
  s: sorted<Item[k]> = [];
  s += [Item{k: 30, name: "thirty"}];
  s += [Item{k: 10, name: "ten"}];
  s += [Item{k: 20, name: "twenty"}];
  deliver(1, s);
}
"#;
    let Some(stdout) = run_deliver("sorted", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":[{\"k\":10,\"name\":\"ten\"},{\"k\":20,\"name\":\"twenty\"},{\"k\":30,\"name\":\"thirty\"}]";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "sorted mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_reads_nested_sorted_field_in_js() {
    // @PLN105 Phase 3 — a struct with a `sorted` field: scalars in place, the sorted field
    // reclassified to a `Vector` node (in-place, key-ordered).
    let src = r#"
struct Item { k: integer, name: text }
struct Cat { title: text, items: sorted<Item[k]>, n: integer }
fn main() {
  c: Cat = Cat { title: "cat", items: [], n: 5 };
  c.items += [Item{k: 30, name: "thirty"}];
  c.items += [Item{k: 10, name: "ten"}];
  deliver(1, c);
}
"#;
    let Some(stdout) = run_deliver("nestsorted", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":{\"title\":\"cat\",\"items\":[{\"k\":10,\"name\":\"ten\"},{\"k\":30,\"name\":\"thirty\"}],\"n\":5}";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "nested-sorted-field mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_flattens_top_level_index_in_js() {
    // @PLN105 Phase 3 — an `index<T>` (red-black tree): pre-flattened via build_index_sorted_vec (an
    // in-order first→next walk), so JS reconstructs the elements in ASCENDING KEY order == the
    // interpreter's `for it in ix`. The node's #left/#right/#color tree bookkeeping is SKIPPED (a
    // `#`-prefixed synthetic field), so only the user fields appear.
    let src = r#"
struct IRec { ik: integer, name: text }
fn main() {
  ix: index<IRec[ik]> = [];
  ix += [IRec{ik: 30, name: "thirty"}];
  ix += [IRec{ik: 10, name: "ten"}];
  ix += [IRec{ik: 20, name: "twenty"}];
  deliver(1, ix);
}
"#;
    let Some(stdout) = run_deliver("index", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":[{\"ik\":10,\"name\":\"ten\"},{\"ik\":20,\"name\":\"twenty\"},{\"ik\":30,\"name\":\"thirty\"}]";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "flattened index mismatch (leaked tree fields? wrong order?)\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_flattens_nested_index_field_in_js() {
    // @PLN105 Phase 3 — a struct with an `index` field: scalars in place, the index field
    // materialised to a key-ordered array (FlatArray), tree bookkeeping skipped.
    let src = r#"
struct IRec { ik: integer, name: text }
struct Db { label: text, idx: index<IRec[ik]>, n: integer }
fn main() {
  d: Db = Db { label: "db", idx: [], n: 7 };
  d.idx += [IRec{ik: 20, name: "twenty"}];
  d.idx += [IRec{ik: 10, name: "ten"}];
  deliver(1, d);
}
"#;
    let Some(stdout) = run_deliver("nestindex", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":{\"label\":\"db\",\"idx\":[{\"ik\":10,\"name\":\"ten\"},{\"ik\":20,\"name\":\"twenty\"}],\"n\":7}";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "nested-index-field mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_flattens_keyed_in_substruct_in_js() {
    // @PLN105 Phase 3 — DEEP nesting: a keyed collection inside a SUB-struct (not a direct field of
    // the root). `flatten_at` recurses through inline record fields — `Outer.inner.items` is at
    // `(rec, pos + inner.pos + items.pos)`, single-instance and addressable — materialising it and
    // rewiring both the Inner and Outer descriptor nodes.
    let src = r#"
struct Item { ik: integer, name: text }
struct Inner { count: integer, items: hash<Item[ik]> }
struct Outer { label: text, inner: Inner, n: integer }
fn main() {
  o: Outer = Outer { label: "top", inner: Inner { count: 2, items: [] }, n: 9 };
  o.inner.items[20] = Item { ik: 20, name: "twenty" };
  o.inner.items[10] = Item { ik: 10, name: "ten" };
  deliver(1, o);
}
"#;
    let Some(stdout) = run_deliver("deep", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":{\"label\":\"top\",\"inner\":{\"count\":2,\"items\":[{\"ik\":10,\"name\":\"ten\"},{\"ik\":20,\"name\":\"twenty\"}]},\"n\":9}";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "deep-nested keyed field mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_flattens_keyed_in_vector_elements_in_js() {
    // @PLN105 Phase 3 — MULTI-INSTANCE: a `vector<Bag>` where EACH element has its OWN keyed
    // collection at a different record. `collect_keyed` descends into vector elements and
    // materialises every hash instance into the `flat` redirect map keyed by its `(rec, pos)`; the
    // single (type-shared) FlatArray node reads each element's own data record via that map.
    let src = r#"
struct Item { ik: integer, name: text }
struct Bag { tag: integer, items: hash<Item[ik]> }
fn main() {
  bags: vector<Bag> = [];
  b0: Bag = Bag { tag: 0, items: [] };
  b0.items[10] = Item { ik: 10, name: "ten" };
  bags += [b0];
  b1: Bag = Bag { tag: 1, items: [] };
  b1.items[21] = Item { ik: 21, name: "twentyone" };
  b1.items[20] = Item { ik: 20, name: "twenty" };
  bags += [b1];
  deliver(1, bags);
}
"#;
    let Some(stdout) = run_deliver("multi", src) else {
        return; // toolchain self-skip
    };
    // Element 0 has one item; element 1 has two (key-sorted) — each from its own materialised hash.
    let want_value = "\"value\":[{\"tag\":0,\"items\":[{\"ik\":10,\"name\":\"ten\"}]},{\"tag\":1,\"items\":[{\"ik\":20,\"name\":\"twenty\"},{\"ik\":21,\"name\":\"twentyone\"}]}]";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "multi-instance (keyed in vector elements) mismatch\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_expose_and_release_a_value_in_js() {
    // @PLN105 Phase 3 — expose(tag,value): a LONG-LIVED deliver. Same handle as deliver (here a
    // materialised hash), but the store is PINNED (read-only lock) so its addresses stay valid
    // across frames until release(tag,value) unpins it. This single-shot harness reads the exposed
    // value once (in the expose call) + confirms release fires; the full cross-frame path needs the
    // asyncify game-loop harness. The interpreter run also proves lock→…→unlock does not crash.
    let src = r#"
struct Item { ik: integer, name: text }
fn main() {
  h: hash<Item[ik]> = [];
  h[20] = Item { ik: 20, name: "twenty" };
  h[10] = Item { ik: 10, name: "ten" };
  expose(1, h);
  release(1, h);
}
"#;
    let Some(stdout) = run_deliver("expose", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":[{\"ik\":10,\"name\":\"ten\"},{\"ik\":20,\"name\":\"twenty\"}]";
    assert!(
        stdout.contains("EXPOSE ") && stdout.contains(want_value) && stdout.contains("RELEASE 1"),
        "expose/release mismatch\n  want EXPOSE with {want_value} + a RELEASE 1\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_survives_memory_grow_after_expose() {
    // @PLN105 §5 memory.grow-safety cell — the exposed re-reader must survive a `memory.grow` that
    // DETACHES the wasm ArrayBuffer. Armed by LOFT_DELIVER_GROW=1: after the initial expose read,
    // the harness grows linear memory by a page (detaching the old buffer) and re-reads the SAME
    // exposed value. readLoftValue re-derives `mem.buffer` per call, so the re-read must NOT throw
    // and must reproduce the value byte-for-byte (`reread_matches`). Anti-vacuity: a DataView
    // captured before the grow MUST throw afterwards (`stale_detached`) — otherwise a passing
    // re-read would be vacuous (the buffer never actually changed).
    let src = r#"
struct Item { ik: integer, name: text }
fn main() {
  h: hash<Item[ik]> = [];
  h[20] = Item { ik: 20, name: "twenty" };
  h[10] = Item { ik: 10, name: "ten" };
  expose(1, h);
  release(1, h);
}
"#;
    let Some(stdout) = run_deliver_env("grow", src, &[("LOFT_DELIVER_GROW", "1")]) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":[{\"ik\":10,\"name\":\"ten\"},{\"ik\":20,\"name\":\"twenty\"}]";
    assert!(
        stdout.contains("GROW-SAFE ")
            && stdout.contains("\"grew\":true")
            && stdout.contains("\"stale_detached\":true") // the detach really happened (anti-vacuity)
            && stdout.contains("\"reread_matches\":true") // reader re-derived its view over the new buffer
            && stdout.contains(want_value),
        "memory.grow-safety cell failed\n  want GROW-SAFE with grew+stale_detached+reread_matches all true and {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_reconstructs_narrow_ints_in_js() {
    // @PLN105 corpus — narrow-int fields (Parts::Byte/Short/ShortRaw/Int). These do NOT store the
    // value directly: a narrow int is packed as `(value - start)` (byte/shortraw), or `(value -
    // start + 1)` with 0=null (nullable short), and the descriptor carries `start` (the range
    // minimum). The reader must re-add `start` and read shorts UNSIGNED — a raw `getInt16`/`getUint8`
    // gives the packed representation, not the value. Distinctive per-field values pin every arm:
    //   u8=200 (start 0) · i8=-5 (start -128) · u16=40000 (start 0) · i16=-1000 (start -32768) ·
    //   i32=100000.
    let src = r#"
struct Narrow { u: u8, sb: i8, us: u16, ss: i16, i: i32 }
fn main() {
  n = Narrow { u: 200, sb: -5, us: 40000, ss: -1000, i: 100000 };
  deliver(1, n);
}
"#;
    let Some(stdout) = run_deliver("narrow", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":{\"u\":200,\"sb\":-5,\"us\":40000,\"ss\":-1000,\"i\":100000}";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "narrow-int reconstruction mismatch (start offset / short signedness not applied?)\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_reconstructs_value_enum_in_js() {
    // @PLN105 corpus — a plain (value) enum field (Parts::Enum/Choices). The discriminant is stored
    // 1-BASED (Text=1, Number=2, FileName=3; 0 unused, 255=null), and the reader resolves the byte
    // to the variant NAME via `variants[disc - 1]` — an off-by-one (indexing `variants[disc]`)
    // silently returns the WRONG variant. Setting `Format.Number` (disc 2) must read back "Number".
    let src = r#"
enum Format { Text, Number, FileName }
struct Rec { name: text, fmt: Format, n: integer }
fn main() {
  r = Rec { name: "hi", fmt: Format.Number, n: 42 };
  deliver(1, r);
}
"#;
    let Some(stdout) = run_deliver("venum", src) else {
        return; // toolchain self-skip
    };
    let want_value = "\"value\":{\"name\":\"hi\",\"fmt\":\"Number\",\"n\":42}";
    assert!(
        stdout.contains("DELIVER ") && stdout.contains(want_value),
        "value-enum reconstruction mismatch (1-based disc / off-by-one variant?)\n  want the value: {want_value}\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_reads_by_ref_array_synthetic_in_js() {
    // @PLN105 corpus — the by-ref `array` arm (Parts::Array) of readLoftValue. No loft SOURCE
    // construct emits a by-ref Array today (every IR vector is inline Parts::Vector — see
    // src/data_store.rs:128), so this arm can't be reached through a `deliver` program. Guard it
    // directly with a SYNTHETIC descriptor + hand-laid memory (tools/reader_array_unit.mjs): a
    // 3-element `array<integer>` must reconstruct to [100, 200, 300]. (Its per-element loop matches
    // the `flatarray` arm the keyed-collection tests already exercise; this pins the field→vRec→
    // element-index path unique to `array`.)
    if !which("node") {
        eprintln!("SKIP: node not installed");
        return;
    }
    let unit = repo_root().join("tools/reader_array_unit.mjs");
    assert!(fa::exists(&unit), "tools/reader_array_unit.mjs missing");
    let out = loft::platform::process::harness_command("node")
        .arg(&unit)
        .output()
        .expect("invoke node array-unit");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success() && stdout.contains("ARRAY-UNIT OK 100,200,300"),
        "by-ref array arm mismatch\n  stdout:{stdout}\n  stderr:{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn deliver_expose_survives_cross_frame_yield_in_js() {
    // @PLN105 §5 — the FULL cross-frame expose test. `deliver_expose_and_release_a_value_in_js`
    // reads the exposed value DURING the expose host call (Stores unambiguously live); this proves
    // the real use case — a page reads the value on a LATER frame. The program exposes a hash, then
    // runs on to `store_load_url_trusted` (a fetch), which asyncify-UNWINDS the whole wasm stack
    // back to JS. The cross-frame harness (asyncify-driven) reconstructs the exposed value at that
    // yield — after `expose` + real execution + the unwind — and only then resumes loft (which
    // releases). A correct key-sorted read there proves `lock_store` keeps the store pinned across
    // the yield and that the reader re-derives its view (the fetch may have grown memory).
    let src = r#"
struct Item { ik: integer, name: text }
fn main() {
  h: hash<Item[ik]> = [];
  h[20] = Item { ik: 20, name: "twenty" };
  h[10] = Item { ik: 10, name: "ten" };
  expose(1, h);
  world: hash<Item[ik]> = [];
  ok = store_load_url_trusted(world, "http://x/y");
  release(1, h);
}
"#;
    let Some(stdout) = run_crossframe("xframe", src) else {
        return; // toolchain self-skip
    };
    if stdout.contains("SKIP: wasm lacks asyncify") {
        // wasm-opt/binaryen absent at build time → no --asyncify pass → no yield to exercise.
        eprintln!("SKIP: --html bundle has no asyncify instrumentation (wasm-opt not installed)");
        return;
    }
    // CROSSFRAME (read AFTER the yield) must carry the key-sorted value, and it must be bracketed by
    // EXPOSE (before the yield) and RELEASE 1 (after the resume) — i.e. the read really is between.
    let want_value = "\"value\":[{\"ik\":10,\"name\":\"ten\"},{\"ik\":20,\"name\":\"twenty\"}]";
    assert!(
        stdout.contains("EXPOSE ")
            && stdout.contains("CROSSFRAME ")
            && stdout.contains(want_value)
            && stdout.contains("RELEASE 1"),
        "cross-frame expose read mismatch\n  want EXPOSE + CROSSFRAME with {want_value} + RELEASE 1\n  got stdout:\n{stdout}"
    );
}

#[test]
fn deliver_scalar_vector_is_zero_copy_o1_in_js() {
    // @PLN105 probe #3 — the ZERO-COPY perf falsifier (the reason the feature exists: routing's
    // ~230k-feature `view` serialises to text and JS `parseFloat`s millions of coord strings; this
    // must instead be O(1)). An inline `vector<scalar>` must reach JS O(1) in the element count `n`,
    // not O(n)/copying, on BOTH sides — proven structurally (airtight), not by absolute wall-clock:
    //  * LOFT-SIDE O(1): the delivered top node is a plain `vector` (NOT a materialised `flatarray`).
    //    An inline vector is handed over by reference; only keyed collections are pre-flattened.
    //  * JS-SIDE O(1): the value reconstructs to a typed-array VIEW aliasing wasm memory
    //    (`shared=true` — buffer identity). A copy cannot alias; a view over an existing buffer
    //    touches no element, so it is O(1) by construction.
    // Delivering 64k AND 1M shows view cost is FLAT (independent of n); the harness's `o1` flag is a
    // same-machine ratio of view-construction vs an O(n) scan of the SAME data (~1000x margin,
    // min-based — robust, never a bare time threshold). Keyed collections are O(n) by the Phase 3
    // pre-flatten design; this covers the inline fast-lane path the perf claim is about.
    let src = r#"
fn fill(n: integer) -> vector<single> {
  v: vector<single> = [];
  i = 0;
  while i < n { v += [i as single]; i += 1; }
  v
}
fn main() {
  deliver(9, fill(2048));      // JIT warmup — tag 9 ignored below
  deliver(1, fill(65536));     // small
  deliver(2, fill(1048576));   // large
}
"#;
    let Some(stdout) = run_perf("perf", src) else {
        return; // toolchain self-skip
    };
    // Both sizes: a plain `vector` node (loft-side O(1)) reconstructs to a zero-copy VIEW
    // (`shared=true`) with exact endpoint values and `o1=true` (view O(1) vs O(n) scan). The prefix
    // is deterministic; the trailing `(viewMs=… )` timing is reported but not asserted.
    let small = "PERF tag=1 n=65536 kind=vector shared=true first=0 last=65535 o1=true";
    let large = "PERF tag=2 n=1048576 kind=vector shared=true first=0 last=1048575 o1=true";
    assert!(
        stdout.contains(small) && stdout.contains(large),
        "zero-copy O(1) falsifier failed (delivered a copy? materialised the vector? o1 regressed?)\n  want both:\n    {small}\n    {large}\n  got stdout:\n{stdout}"
    );
}
