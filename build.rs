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
    // @PLN184 — Windows gives a program's main thread 1 MiB of stack where Linux gives 8 MiB,
    // so a parse or a recursion that runs on Linux overflowed on Windows (the frame-headroom
    // guard did).  Every loft binary gets the Linux amount on Windows too.
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.contains("windows-msvc") {
        println!("cargo:rustc-link-arg-bins=/STACK:{}", 8 << 20);
    } else if target.contains("windows-gnu") {
        println!("cargo:rustc-link-arg-bins=-Wl,--stack,{}", 8 << 20);
    }
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

    // loft#1859 — WHICH compiler this is, as the content it was built from: every file under
    // `src/`, `default/` and `loft-ffi/src`, and the manifest and lock that pick its
    // dependencies.  The startup caches key on it.  They keyed on the running executable's
    // modification time instead, and every test binary is its own executable: one rebuild
    // wrote one stdlib image per test binary (31 548 images, 227 GB on one box), none of them
    // ever read by another.  Every binary of one build shares this stamp, and a WIP edit still
    // moves it.  A content hash, not a timestamp, so a release rebuilt from its source
    // archive bakes the same bytes (`scripts/repro-roundtrip.sh`).
    println!("cargo:rustc-env=LOFT_BUILD_STAMP={:016x}", source_stamp());

    // The version this loft answers to — what a package's `loft = ">=…"` floor is checked
    // against, and what `loft --version` prints.  A RELEASE is its `Cargo.toml` version.  Any
    // other build — a daily, a development build — is `<last release's YYYY.M>.<yyyymmdd of
    // HEAD's commit, UTC>`: it orders after the release it builds on and after any point
    // release of it, and before the next monthly, so a library that needs what landed on `main`
    // can name the day instead of waiting for the next release (`2026.10.20261005`).
    let effective = effective_version();
    println!("cargo:rustc-env=LOFT_EFFECTIVE_VERSION={effective}");
    println!("cargo:rerun-if-env-changed=LOFT_VERSION");

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
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");
    // …and when the build flags change, so LOFT_BUILD_RUSTFLAGS stays accurate.
    println!("cargo:rerun-if-env-changed=RUSTFLAGS");
    println!("cargo:rerun-if-env-changed=CARGO_ENCODED_RUSTFLAGS");
    // …and when rustc itself changes, so LOFT_BUILD_RUSTC stays accurate.
    println!("cargo:rerun-if-env-changed=RUSTC");
    warn_stale_compiled_stdlib();
}

/// @PLN181 — say so, on every build, when the compiled standard library was compiled from a
/// different `default/*.loft` than the one this binary will load.  The binary then declines
/// the whole compiled library at start-up and interprets those functions: correct, silently
/// 5-23x slower on text routines, and nothing else reports it (`scripts/compiled_stdlib_fresh.py`
/// has the why and the other three places that ask).  The hash is `compiled_stdlib::source_hash`
/// re-derived: SHA-256 over each file's name and bytes in name order, each part prefixed by its
/// length as 8 little-endian bytes, the first 8 bytes of the digest.
fn warn_stale_compiled_stdlib() {
    use sha2::{Digest, Sha256};
    let Ok(generated) = std::fs::read_to_string("src/compiled_stdlib_gen.rs") else {
        return;
    };
    let recorded = generated
        .split("SOURCE_HASH: u64 = ")
        .nth(1)
        .and_then(|rest| rest.split(';').next())
        .and_then(|n| n.trim().parse::<u64>().ok());
    let Ok(entries) = std::fs::read_dir("default") else {
        return;
    };
    let mut files: Vec<(String, Vec<u8>)> = entries
        .flatten()
        .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("loft"))
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_string();
            Some((name, std::fs::read(e.path()).ok()?))
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut h = Sha256::new();
    for (name, content) in &files {
        for part in [name.as_bytes(), content.as_slice()] {
            h.update((part.len() as u64).to_le_bytes());
            h.update(part);
        }
    }
    let d = h.finalize();
    let current = u64::from_le_bytes([d[0], d[1], d[2], d[3], d[4], d[5], d[6], d[7]]);
    if recorded != Some(current) {
        println!(
            "cargo:warning=the compiled standard library is STALE (src/compiled_stdlib_gen.rs was \
             compiled from a different default/*.loft): this binary interprets every compiled \
             stdlib function, 5-23x slower on text routines.  Run: make compiled-stdlib"
        );
    }
}

/// FNV-1a/64 over every file under the compiler's source roots, in sorted order of their
/// relative paths, each path hashed before its bytes so a rename moves the stamp too.
fn source_stamp() -> u64 {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out);
            } else {
                out.push(p);
            }
        }
    }
    let mut files = vec![
        std::path::PathBuf::from("Cargo.toml"),
        std::path::PathBuf::from("Cargo.lock"),
    ];
    for root in ["src", "default", "loft-ffi/src"] {
        walk(std::path::Path::new(root), &mut files);
    }
    files.sort();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut put = |bytes: &[u8]| {
        for b in bytes {
            h ^= u64::from(*b);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    };
    for f in &files {
        put(f.to_string_lossy().replace('\\', "/").as_bytes());
        if let Ok(bytes) = std::fs::read(f) {
            put(&(bytes.len() as u64).to_le_bytes());
            put(&bytes);
        }
    }
    h
}

/// See the `LOFT_EFFECTIVE_VERSION` comment in `main`.  `LOFT_VERSION` overrides it outright.  A
/// tree with no git of its OWN — a release's source archive, which `repro-verify.sh` rebuilds —
/// is the release it was cut from, so it answers the `Cargo.toml` version and reproduces the
/// tagged build byte for byte.  So does a checkout whose HEAD carries the release's tag.
fn effective_version() -> String {
    let release = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    if let Ok(v) = std::env::var("LOFT_VERSION") {
        let v = v.trim().to_string();
        if !v.is_empty() {
            return v;
        }
    }
    let git = |args: &[&str]| -> Option<String> {
        std::process::Command::new("git")
            .args(args)
            .env("TZ", "UTC")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    // Git of its OWN: a source archive unpacked inside some other repository must not read
    // that repository's history.
    let own = std::env::var("CARGO_MANIFEST_DIR").ok().and_then(|d| {
        let top = git(&["rev-parse", "--show-toplevel"])?;
        let same = std::fs::canonicalize(&top).ok()? == std::fs::canonicalize(&d).ok()?;
        same.then_some(())
    });
    if own.is_none() {
        return release;
    }
    let tagged = git(&["tag", "--points-at", "HEAD"]).unwrap_or_default();
    if tagged.lines().any(|t| t.trim() == format!("v{release}")) {
        return release;
    }
    let Some(day) = git(&["log", "-1", "--format=%cd", "--date=format-local:%Y%m%d"])
        .filter(|d| d.len() == 8 && d.bytes().all(|b| b.is_ascii_digit()))
    else {
        return release;
    };
    // The release this build is ON: the newest `v*` tag HEAD descends from.  `Cargo.toml` may
    // already name the NEXT release, and a daily stamped with that would outrank the release
    // it precedes.  A shallow clone cannot walk the ancestry, so it takes the highest release
    // tag it has (`fetch-tags: true` in the workflows that build loft); with no tag at all it
    // falls back to `Cargo.toml`.
    let base = git(&["describe", "--tags", "--abbrev=0", "--match", "v[0-9]*"])
        .or_else(|| {
            git(&["tag", "-l", "v[0-9]*", "--sort=-v:refname"])
                .and_then(|l| l.lines().next().map(str::to_string))
        })
        .and_then(|t| t.strip_prefix('v').map(str::to_string))
        .unwrap_or_else(|| release.clone());
    let mut parts = base.split('.');
    match (parts.next(), parts.next()) {
        (Some(y), Some(m)) => format!("{y}.{m}.{day}"),
        _ => release,
    }
}
