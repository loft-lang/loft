// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! #444 — a `use`d library's `[wasm.bridge]` state must survive a warm
//! whole-program cache load.
//!
//! `wasm_bridge_routes` / `wasm_bridge_packages` / `wasm_bridge_host_js_files`
//! are populated only at parse (from each dependency manifest) and are NOT part
//! of the IR bundle the cache serialises.  A warm load skips parsing, so before
//! the fix the route table came back EMPTY — and `--html` codegen then emitted a
//! host-import `extern` for an already-routed `#native`, colliding (`E0428`)
//! with the library's public wrapper of the same name.  The bug was a coin-flip:
//! a COLD run (cache miss) re-parses and succeeds; a WARM run (cache hit) fails.
//!
//! This drives `save_program` → `warm_load_program` directly (no wasm toolchain)
//! and asserts the three fields round-trip — the cold-equal invariant the
//! `--html` extern-skip decision depends on.  It runs in its own test binary, so
//! the `XDG_CACHE_HOME` override is free of in-process env races.
//!
//! @PLN119 — the second test is the same claim for a different parse-time-only fact: an
//! out-of-process library's PLACEMENT.  `mark_exports` writes its marks into `Data`, so the
//! bundle carries them; the list `main` starts a worker from is built by the parse and does
//! not.  A warm load therefore left the marked calls pointing at `compile.rs`'s
//! "native function not loaded" stub — a placed library that works on its first run and panics
//! on its second (loft#1129).  Both facts ride one bundle and are asserted in the single test
//! this binary holds — the `XDG_CACHE_HOME` override is what makes a second test here a race.

use loft::database::Stores;
use loft::keys::DbRef;
use loft::parser::Parser;

#[test]
fn wasm_bridge_state_survives_warm_program_cache() {
    let pid = std::process::id();
    let tmp = std::env::temp_dir();
    let cache_dir = tmp.join(format!("loft_p444_cache_{pid}"));
    let _ = std::fs::remove_dir_all(&cache_dir);
    // SAFETY: this is the only test in this binary, so no other thread is
    // reading the environment concurrently.
    unsafe { std::env::set_var("XDG_CACHE_HOME", &cache_dir) };
    // A whole-program bundle is keyed on the script path and validated by every
    // parsed source's content hash, so the warm load needs a real, unchanged
    // source file on disk.
    let script = tmp.join(format!("loft_p444_{pid}.loft"));
    std::fs::write(&script, "fn main() { println(sha256(\"hi\")); }\n").expect("write script");
    let script_abs = std::fs::canonicalize(&script)
        .expect("canonicalize")
        .to_string_lossy()
        .into_owned();

    // ── cold: a parse populated the bridge state; persist it (save_program). ──
    let mut cold = Parser::new();
    cold.parsed_sources.push(script_abs.clone());
    cold.data.wasm_bridge_routes.insert(
        "n_sha256".to_string(),
        ("crypto-wasm".to_string(), "crypto_sha256".to_string()),
    );
    cold.data.wasm_bridge_routes.insert(
        "n_hmac_sha256".to_string(),
        ("crypto-wasm".to_string(), "crypto_hmac_sha256".to_string()),
    );
    cold.data.wasm_bridge_packages.push((
        "crypto-wasm".to_string(),
        "/home/me/.loft/registry/crypto-0.3.3".to_string(),
    ));
    cold.data
        .wasm_bridge_host_js_files
        .push("/home/me/.loft/registry/crypto-0.3.3/wasm/host.js".to_string());
    // @PLN119 — the placement registrations ride the same manifest, and are taken as an
    // ARGUMENT rather than read off the parser because `main` consumes the list before the
    // bundle is written.  A directory with SPACES on purpose: the line is
    // `plib <name> <spelling> <pkg_dir>` and only the first two tokens are space-free, so the
    // reader has to take the remainder verbatim.
    let placed = vec![
        (
            "svc".to_string(),
            "/home/me/lib dirs/svc".to_string(),
            loft::lib_placement::Placement::Remote,
        ),
        (
            "worker".to_string(),
            "/home/me/.loft/registry/worker-1.0.0".to_string(),
            loft::lib_placement::Placement::Process,
        ),
    ];
    loft::startup_cache::save_program(
        &cold,
        &script_abs,
        &cold.lib_dirs,
        "default",
        cold.data.definitions(),
        &placed,
        &loft::startup_cache::AutoNative {
            ctx: &loft::startup_cache::native_lib_context(false),
            libs: &[],
        },
    );

    // ── warm: a fresh parser loads the bundle and skips parsing entirely. ──
    let mut warm = Parser::new();
    let mut store: Option<(Stores, DbRef)> = None;
    let hit = loft::startup_cache::warm_load_program(
        &mut warm,
        &script_abs,
        "default",
        &loft::startup_cache::native_lib_context(false),
        &mut store,
    );
    assert!(
        hit.is_some(),
        "warm load must hit the just-written bundle (same binary, unchanged source)"
    );

    // The route table is what the `--html` extern-skip keys on: empty ⇒ the
    // host-import `extern` leaks ⇒ E0428.  All three fields must come back.
    assert_eq!(
        warm.data.wasm_bridge_routes.get("n_sha256"),
        Some(&("crypto-wasm".to_string(), "crypto_sha256".to_string())),
        "n_sha256 route lost across warm load (#444): the extern would leak and collide"
    );
    assert_eq!(
        warm.data.wasm_bridge_routes.get("n_hmac_sha256"),
        Some(&("crypto-wasm".to_string(), "crypto_hmac_sha256".to_string())),
        "n_hmac_sha256 route lost across warm load (#444)"
    );
    assert_eq!(
        warm.data.wasm_bridge_packages,
        vec![(
            "crypto-wasm".to_string(),
            "/home/me/.loft/registry/crypto-0.3.3".to_string()
        )],
        "bridge package (drives the bridge-crate link) lost across warm load (#444)"
    );
    assert_eq!(
        warm.data.wasm_bridge_host_js_files,
        vec!["/home/me/.loft/registry/crypto-0.3.3/wasm/host.js".to_string()],
        "host_js preamble file lost across warm load (#444)"
    );

    // @PLN119 — and the placement registrations, whose loss is worse than a lost route because
    // the MARKS survive: `mark_exports` writes them into `Data`, so a warm run has functions
    // marked for a worker that was never started and every one of them resolves to
    // `compile.rs`'s "native function not loaded" stub.
    assert_eq!(
        warm.pending_placed_libs, placed,
        "the placement registrations must come back, or a placed library works on its first \
         run and panics on its second"
    );

    let _ = std::fs::remove_file(&script);
    let _ = std::fs::remove_dir_all(&cache_dir);
}
