// Expose a BUILD_ID to the compiler so the bytecode cache can detect
// same-version rebuilds (e.g. a parser fix without a version bump).
//
// Three sources, in order, and the FIRST one is what makes a release
// reproducible: an explicit `LOFT_BUILD_ID`, then the git HEAD commit, then a
// timestamp.
//
// The timestamp is why a release used to fail `repro-verify.sh` on every target.
// A release is cut from a git checkout, so it baked a commit hash; the verifier
// rebuilds from the release's SOURCE ARCHIVE, which has no `.git`, so it fell
// through to seconds-since-epoch — a different string, of a different LENGTH,
// changing on every run. The binary could not match, and the mismatch looked
// like a source problem rather than a stamp the build invented.
//
// The fallback stays a timestamp rather than a fixed word: BUILD_ID exists so
// the bytecode cache can tell two same-version builds apart, and a constant
// would make two different non-git builds share a cache entry. The override is
// the seam a reproducible build needs; the timestamp is still right for a
// casual one.

fn main() {
    let id = std::env::var("LOFT_BUILD_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .or_else(|| {
            std::process::Command::new("git")
                .args(["rev-parse", "--short", "HEAD"])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
                .filter(|v| !v.is_empty())
        })
        .unwrap_or_else(|| {
            // Fallback: seconds since epoch — changes on every build.
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .unwrap_or_default()
        });
    println!("cargo:rustc-env=LOFT_BUILD_ID={id}");
    // Without this, a second verification run reuses the first one's baked id.
    println!("cargo:rerun-if-env-changed=LOFT_BUILD_ID");

    // Record the effective RUSTFLAGS this loft build used so the package
    // native-crate build (`extensions::auto_build_native`) can compile shared
    // transitive deps (e.g. `libloading`) with the SAME flags (#274).  A
    // debuginfo divergence — loft built with `-g` (the Makefile does), the
    // package without — gives the shared dep a different SVH; rustc then finds
    // two copies with one `StableCrateId` and aborts at the generated program's
    // link step.  `CARGO_ENCODED_RUSTFLAGS` is set by cargo for the build script
    // and captures flags from env AND `.cargo/config`; its `\x1f` separators
    // decode to a space-joined `RUSTFLAGS` string.
    let rustflags = std::env::var("CARGO_ENCODED_RUSTFLAGS")
        .map(|enc| {
            enc.split('\u{1f}')
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .or_else(|_| std::env::var("RUSTFLAGS"))
        .unwrap_or_default();
    // Drop `--remap-path-prefix` entries before baking (repro-flags.sh adds them
    // to make a release reproducible).  Each names an ABSOLUTE path on the BUILD
    // machine, so baking them would (a) put the maintainer's home directory in
    // every released binary — the 193 embedded paths that made a release
    // unverifiable in the first place — and (b) be useless to a consumer, whose
    // cargo home is somewhere else entirely, so the remap would not fire and
    // their copy of a shared dep would embed real paths while loft's embeds the
    // mapped ones.  That is the #274 SVH mismatch, arrived at from the other
    // side.  `extensions::local_remap_flags` recomputes them for whatever machine
    // is doing the building, which makes both halves agree on `/cargo` and
    // `/rustc` rather than on nothing.
    let rustflags = rustflags
        .split_whitespace()
        .filter(|f| !f.starts_with("--remap-path-prefix="))
        .collect::<Vec<_>>()
        .join(" ");
    println!("cargo:rustc-env=LOFT_BUILD_RUSTFLAGS={}", rustflags.trim());

    // Stamp the rustc version this loft (and its rlib) was built with, so the
    // native path can detect — up front, before a doomed compile — that the
    // toolchain changed under it.  rlibs are SVH-locked to one rustc, so after a
    // `rustup update` the cached rlib no longer links with the new rustc; loft
    // compares the live `rustc --version` against this stamp and, for a default
    // native run, falls back to the interpreter instead of attempting (and
    // failing) the compile.  Uses cargo's `RUSTC` (the exact rustc in use).
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let rustc_version = std::process::Command::new(&rustc)
        .arg("--version")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    println!("cargo:rustc-env=LOFT_BUILD_RUSTC={rustc_version}");

    // Fingerprint loft-ffi's SOURCE — the stable cache key for REGISTRY cdylibs
    // (`extensions::auto_build_native`).  A registry cdylib links loft-ffi (the
    // C-ABI), NEVER libloft.rlib — verified: the cdylib has zero loft undefined
    // symbols, only system NEEDED libs.  So its validity depends on loft-ffi,
    // not the interpreter.  Hashing loft-ffi's SOURCE (not a compiled artifact)
    // gives a key identical across debug/release/test profiles, so the shared
    // ~/.loft/build-cache is reused across every loft build in a job instead of
    // cross-invalidating on the libloft.rlib hash (which differs per profile);
    // it still flips on any real loft-ffi change (ABI or impl, even
    // un-version-bumped).  FNV-1a/64 over the sorted `loft-ffi/src/*.rs` bytes.
    let mut ffi_fp: u64 = 0xcbf2_9ce4_8422_2325;
    let mut ffi_files: Vec<std::path::PathBuf> = std::fs::read_dir("loft-ffi/src")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "rs"))
        .collect();
    ffi_files.sort();
    for f in &ffi_files {
        if let Ok(bytes) = std::fs::read(f) {
            for b in &bytes {
                ffi_fp ^= u64::from(*b);
                ffi_fp = ffi_fp.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
    }
    println!("cargo:rustc-env=LOFT_FFI_FINGERPRINT={ffi_fp}");

    // @PLN21 Phase 1 — the target triple this loft was built for (e.g.
    // `x86_64-unknown-linux-gnu`).  cargo sets `TARGET` for build scripts; it is
    // the *authoritative* host triple — `std::env::consts` only yields
    // `<arch>-<os>-<family>` (`x86_64-linux-unix`), which loses the libc/abi a
    // prebuilt cdylib's portability hinges on (gnu vs musl).  Names the
    // `prebuilt/<triple>/` dir loft loads a precompiled cdylib from.
    println!(
        "cargo:rustc-env=LOFT_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap_or_default()
    );

    // @PLN117 — one name for "this build's `par` runs on the page's GLOBAL rayon
    // pool" (`parallel.rs::with_pool`): the wasm-bindgen browser bundle, or
    // loft's own browser threading on a wasm target.  A native build that merely
    // has the threading feature switched on — `cargo clippy --all-features` does
    // — keeps the private native pool, because there is no page to install one.
    println!("cargo:rustc-check-cfg=cfg(browser_pool)");
    let target = std::env::var("TARGET").unwrap_or_default();
    let wasm_target = target.starts_with("wasm32");
    let wasm_bindgen_build = std::env::var_os("CARGO_FEATURE_WASM").is_some();
    let loft_browser_threads = std::env::var_os("CARGO_FEATURE_WASM_NATIVE_THREADS").is_some();
    if wasm_bindgen_build || (loft_browser_threads && wasm_target) {
        println!("cargo:rustc-cfg=browser_pool");
    }

    // loft#678 — one name for "the working-set store loaders (`store_load_key`,
    // `store_load_keys`, `store_load_key_text`, `store_load_range`) exist in this
    // build".  Two builds qualify and they get their bytes differently: a native
    // build with `remote-store` reads ranges over `ureq`, and the BROWSER
    // (`--html`) reads them through the asyncify `fetch()` host import — the same
    // split `net::fetch_bytes` already carries for `store_load_url_trusted`.
    //
    // It is a cfg rather than a feature because the browser rlib is built
    // `--no-default-features --features random` (WASM.md pins that command, and
    // the --html bundle check keys off it), so the capability cannot be selected
    // there by a feature flag without changing a documented build line.  Naming it
    // once also keeps the gate honest: it appeared in 25 `#[cfg]`s, and a
    // hand-written `any(...)` at each is how a call site drifts out of step with
    // its callee and breaks only the wasm compile gate.
    // BOTH browser bundles qualify, and the distinction between them is the
    // TRANSPORT, never the capability: `--html` reads ranges through the asyncify
    // `fetch()` host import, and the wasm-bindgen gallery reads them through
    // `loftHost.fs_*` — the same split `host_fs` below already names.  Excluding
    // the wasm-bindgen build left `store_load_key_text` out of the gallery bundle
    // while the stdlib still declared it, so `assets::prefetch` reached a panic
    // stub: a library that resolves, compiles and then aborts mid-frame.
    println!("cargo:rustc-check-cfg=cfg(paged_store)");
    let browser_wasm = wasm_target && !target.contains("wasi") && !wasm_bindgen_build;
    let browser_bindgen = wasm_target && !target.contains("wasi") && wasm_bindgen_build;
    if std::env::var_os("CARGO_FEATURE_REMOTE_STORE").is_some() || browser_wasm || browser_bindgen {
        println!("cargo:rustc-cfg=paged_store");
    }

    // loft#851 — one name for "this build reaches the filesystem through a JS
    // host rather than through `std::fs`".  Two builds qualify and they differ
    // only in HOW they call out: the wasm-bindgen bundle reaches
    // `globalThis.loftHost.fs_*` through `js_sys`, and `--html`
    // (wasm32-unknown-unknown, no wasm-bindgen) reaches the same contract
    // through raw `loft_io` imports.  `src/wasm.rs` owns that choice; every
    // call site asks only this one question.
    //
    // wasip2 is deliberately excluded.  `--native-wasm` has a REAL filesystem
    // through WASI preopens, so routing it to a page's host would replace a
    // working `std::fs` with a bridge no wasip2 host defines — the same
    // over-broad `target_arch = "wasm32"` that @P334 already had to narrow in
    // `src/state/io.rs` and `src/database/io.rs`.
    println!("cargo:rustc-check-cfg=cfg(host_fs)");
    if wasm_bindgen_build || browser_wasm {
        println!("cargo:rustc-cfg=host_fs");
    }

    // Re-run when anything that identifies this build changes: git HEAD / refs
    // (committed state), build.rs itself, the compiler source (`src/`) and stdlib
    // (`default/`), and loft-ffi's source.  Without src/ + default/, build.rs never
    // recomputes on an uncommitted WIP edit, so a source-content build id goes
    // stale across WIP rebuilds and the bytecode/stdlib cache mis-reads a store
    // laid out by a different binary (the cross-checkout / WIP-rebuild corruption:
    // `d_nr=u32::MAX`).  loft-ffi/src keeps LOFT_FFI_FINGERPRINT accurate.  git
    // HEAD alone is blind to WIP; these make the rerun fire on the edits a content
    // hash must reflect.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=default");
    println!("cargo:rerun-if-changed=loft-ffi/src");
    // …and when the build flags change, so LOFT_BUILD_RUSTFLAGS stays accurate.
    println!("cargo:rerun-if-env-changed=RUSTFLAGS");
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
    // …and when rustc itself changes, so LOFT_BUILD_RUSTC stays accurate.
    println!("cargo:rerun-if-env-changed=RUSTC");
}
