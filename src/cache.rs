// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I86 — Startup cache & embedded stdlib

//! Startup cache — skip re-parsing the `default/` standard library on
//! every run.
//!
//! Measured (native `--release`, see
//! `doc/claude/plans/82-const-store/STARTUP_CACHE_PLAN.md`):
//! parsing `default/` is ~90 % of cold-start (~15 ms) while bytecode
//! generation is ~0.5 ms.  So the win is to cache the parser's output
//! (`Data`) keyed on the stdlib content, restore it, and re-run the
//! cheap codegen fresh — rather than serialise bytecode/stores.
//!
//! This module currently provides the **cache key** — the correctness
//! foundation.  The retired Phase D cache (removed in the integer-i64
//! migration) keyed only on the user source and therefore served stale
//! bytecode after a `default/*.loft` edit; the key here folds in the
//! stdlib content, the loft version, the build id, and the active
//! feature set so no such staleness is possible.
//!
//! ## No serde
//!
//! The `Data`/bytecode snapshot is serialised by hand (length-prefixed
//! little-endian, the same approach as the retired Phase D
//! `src/cache.rs`), **not** via `serde`.  serde-derive cannot express
//! the IR cleanly — `&'static str` fields (`Block.name`,
//! `Definition.synthetic`) make the derive inject a `'de: 'static`
//! bound that poisons the whole recursive `Value`/`Type`/`Data` graph,
//! and the `OnceLock` index field is non-derivable.  Hand-rolled
//! encoding sidesteps all of it and lets us skip the rebuildable
//! `HashMap` indices entirely.  See [CODE.md](../doc/claude/CODE.md)
//! § Dependencies — serde is a forbidden dependency project-wide
//! (native builds).

use sha2::{Digest, Sha256};

/// Format-version byte.  Bump whenever the on-disk snapshot layout
/// changes so old caches are rejected rather than misread.
///
/// 2 — @PLN127 arc D grew `DbField` by a `nullable` byte (stride 28 → 29). The
/// stdlib key does NOT fold in the binary's mtime the way a program bundle does
/// (`BUILD_ID` is the git HEAD hash, unchanged across uncommitted edits), so a
/// cache written at the old stride was read at the new one and panicked in
/// `ir_read` on a shifted discriminant. A layout change is exactly what this
/// byte is for.
///
/// 3 — `NdBlock` / `NdLoop` hold their sub-record BY REFERENCE (a
/// box-of-one vector) instead of inlining it, which moved every offset in a
/// `Node` and shrank its stride from 48 to 28.
///
/// 4 — the `Data` root carries the two import tables (`imports`, `use_names`)
/// a warm load replays into `def_names`, which grew the root from 16 to
/// 20 bytes (loft#1359).
///
/// 6 — `Variable` carries `view_elided`, the eleventh codegen-read field
/// (stride 37 → 38): a warm load at the old stride emitted the copy the
/// elision removes (@PLN157 § V-g).
///
/// 7 — `Variable` carries `lazy_buffer` (stride 38 → 39): a warm load at the old stride
/// would mint a buffer at entry AND leave its guarded mint inert (@PLN164 A0).
///
/// 8 — `Variable` carries `deferred_first_bind` (stride 39 → 40): a warm load at the old
/// stride would copy at a bind the scope pass paired for an adopt (@PLN164 B1b).
///
/// 9 — `DefType::TypeTemplate` (code 11), a generic struct or enum (@PLN165 D2): an older
/// reader panics on the code, so an image carrying one must not be read by it.
///
/// 10 — `Definition` carries `type_params`, `instance_of` and `instance_args` (stride
/// 167 → 183, @PLN165 D2/D3): an older image lays the record out differently.
///
/// 11 — `Definition` carries `builtin` (stride 183 → 184, @PLN165 arc E).
///
/// 12 — `Variable` carries `linked_narrow` and `store_text_link` (@PLN167): a warm load
/// without `linked_narrow` read a linked narrow local's field encoding as its value.
///
/// 13 — `Attribute` carries `work_buffer` (`@FR-R-WorkBuffer`): a warm load without it
/// reads a promoted local's hidden parameter as the function's return buffer.
///
/// 14 — `Data` carries `type_var_bounds` (@PLN166 B1): a parse continuing a loaded stdlib
/// without them mints a second placeholder for a `<T>` the stdlib already has.
///
/// 15 — `Variable` carries `user_named` (stride 42 → 43, loft#1834): a warm load without it
/// reads a user's `_`-prefixed parameter as a compiler temporary, and loses F-ParamRebind.
///
/// 16 — `Variable` carries `user_appended` and `copy_bound` (stride 43 → 45, loft#1840): a
/// warm load without them reads a copy the program appended to as unwritten, and loses its
/// `lost-write`.
///
/// 17 — `Definition` carries `operator` (stride 184 → 185, @PLN182): a warm stdlib without it
/// reads an `operator` definition as a plain method, which no operator form reaches.
///
/// 18 — `Variable` carries `buffer_witnessed`, `rebind_orig` and `scope` (stride 45 → 62), and
/// the image is the CLOSED program (loft#1858): a warm load without `rebind_orig` freed the
/// store a caller handed a rebound parameter.
const CACHE_FORMAT_VERSION: u8 = 18;

/// Loft crate version — a release bump invalidates every cache.
const LOFT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Build id from `build.rs` (git short HEAD, or epoch-seconds fallback).
/// Invalidates the cache when the interpreter is rebuilt without a
/// version bump — e.g. a parser fix that changes `Data` output.
const BUILD_ID: &str = env!("LOFT_BUILD_ID");

/// The set of compile-time features that change parsed `Data`, the
/// bytecode, or the native registry — and therefore must be part of the
/// cache key.  A cache produced by a `threading` build must not be read
/// by a non-`threading` build.
///
/// Encoded as a stable, comma-joined string of the features that are
/// *enabled* in this build.  Order is fixed (not dependent on cfg
/// evaluation order) so the key is deterministic.
#[must_use]
pub fn feature_signature() -> String {
    let mut feats: Vec<&str> = Vec::new();
    if cfg!(feature = "threading") {
        feats.push("threading");
    }
    if cfg!(feature = "wasm") {
        feats.push("wasm");
    }
    if cfg!(feature = "mmap") {
        feats.push("mmap");
    }
    if cfg!(feature = "png") {
        feats.push("png");
    }
    if cfg!(feature = "native-extensions") {
        feats.push("native-extensions");
    }
    if cfg!(feature = "random") {
        feats.push("random");
    }
    feats.join(",")
}

/// @PLN11 G2/M6 — a stable string identifying THIS binary build, for the
/// whole-program cache **manifest**.  Mirrors the version inputs of
/// [`stdlib_cache_key`] (format / version / rebuild-id / target / features) so a
/// binary upgrade invalidates a stale program bundle: a bundle written by one
/// build must never be loaded by another (its baked store layout / codegen may
/// differ, which `Store::is_store_file`'s fixed magic does NOT catch).
///
/// Also folds in the running executable's modification time ([`binary_signature_tag`])
/// so an **uncommitted** compiler rebuild invalidates bundles too — [`BUILD_ID`]
/// is the git HEAD hash, which does not change across uncommitted edits, leaving
/// a parser/scopes fix under development at risk of a stale warm-load (see the
/// plan's "Debugging-iteration cost + dev-safety caveat").
#[must_use]
pub fn build_signature() -> String {
    format!(
        "v{CACHE_FORMAT_VERSION}|{LOFT_VERSION}|{BUILD_ID}|{}|{}|{}",
        target_triple(),
        feature_signature(),
        binary_signature_tag(),
    )
}

/// A tag for the *running binary's own build*, folded into [`build_signature`].
///
/// The executable's modification time changes on every rebuild (cargo rewrites
/// the binary), so mixing it in makes any rebuild — committed or not —
/// invalidate program bundles, closing the gap [`BUILD_ID`] (git HEAD) leaves
/// open for uncommitted dev builds.  Best-effort: returns `""` when the exe path
/// or its mtime is unavailable, so the signature gracefully falls back to the
/// [`BUILD_ID`]-only behaviour rather than panicking.
#[must_use]
fn binary_signature_tag() -> String {
    let Ok(exe) = std::env::current_exe() else {
        return String::new();
    };
    let Ok(meta) = std::fs::metadata(&exe) else {
        return String::new();
    };
    let Ok(mtime) = meta.modified() else {
        return String::new();
    };
    match mtime.duration_since(std::time::UNIX_EPOCH) {
        Ok(d) => format!("{}.{}", d.as_secs(), d.subsec_nanos()),
        Err(_) => String::new(),
    }
}

/// Which `loft` EXECUTABLE this is: its build id and the binary's own modification time
/// ([`binary_signature_tag`]).  An auto-native library artifact holds code THIS binary's
/// generator emitted and is called through THIS binary's store layout, so it must never be
/// reused by another one — not even by a binary that finds the same `libloft.rlib`.  Keyed
/// on the rlib alone, an installed `loft` upgraded without its rlib reused the previous
/// binary's artifact and corrupted the store on the first call (loft#1776).
#[must_use]
pub fn loft_exe_identity() -> u64 {
    use std::hash::{Hash, Hasher};
    use std::sync::OnceLock;
    static ID: OnceLock<u64> = OnceLock::new();
    *ID.get_or_init(|| {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        BUILD_ID.hash(&mut h);
        binary_signature_tag().hash(&mut h);
        h.finish()
    })
}

/// @PLN11 G2 / track 1 — whether the whole-program startup cache is active for
/// this run: **on**, for an installed `loft` and a development build alike, unless
/// `LOFT_NO_CACHE` is set — see [`cache_decision`] for the policy.
#[must_use]
pub fn program_cache_enabled() -> bool {
    cache_decision(std::env::var_os("LOFT_NO_CACHE").is_some_and(|v| !v.is_empty()))
}

/// Whether the user has armed a compiler diagnostic for this run.
///
/// When they have, a layer that can serve a STALE answer must say so: an instrument
/// pointed at the parser reads nothing at all if the parse was replaced by a cached
/// bundle, and the silence is indistinguishable from "the code path never ran".
#[must_use]
pub fn diagnostics_armed() -> bool {
    ["LOFT_LOG", "LOFT_IR"]
        .iter()
        .any(|v| std::env::var_os(v).is_some_and(|s| !s.is_empty()))
}

/// The cache-enable policy (@C133): on, unless `LOFT_NO_CACHE` turns it off.
///
/// A development build (`cargo run`, `cargo test`, a binary under `target/`) is cached like
/// an installed one, which is safe on the facts: both cache keys fold in
/// [`binary_signature_tag`], so a rebuilt compiler never reads a bundle an earlier build
/// wrote, and a warm load reproduces the cold run's behaviour — the placed-library workers
/// ride the manifest's `plib` list, and each diagnostic's fixes ride its line
/// (loft#1129).  What stays is the case no key can see: a compiler instrumented WITHOUT a
/// rebuild (an armed `LOFT_LOG` / `LOFT_IR`), where a warm run says so and names
/// `LOFT_NO_CACHE=1` (`main.rs`), and a test that runs the binary to measure a cold compile,
/// which sets it.
#[must_use]
fn cache_decision(no_cache: bool) -> bool {
    !no_cache
}

/// Compute the stdlib cache key: a SHA-256 over every input that can
/// change the compiled standard library.
///
/// Inputs, each length-prefixed so no concatenation ambiguity exists:
/// - [`CACHE_FORMAT_VERSION`] — on-disk layout
/// - [`LOFT_VERSION`] — release version
/// - [`BUILD_ID`] — same-version rebuild discriminator
/// - the target triple — no cross-arch cache reuse
/// - the active [`feature_signature`]
/// - the concatenated `default/*.loft` source bytes (the
///   retirement-bug fix: a stdlib edit changes the key)
///
/// `stdlib_sources` is a slice of `(name, content)` pairs.  The caller
/// passes them in a stable order (the loader collects them sorted);
/// names are included so a rename also invalidates.
#[must_use]
pub fn stdlib_cache_key(stdlib_sources: &[(String, String)]) -> [u8; 32] {
    let mut h = Sha256::new();
    // Each field is fed as (u64 little-endian length, bytes) so two
    // different field boundaries can never hash identically.
    let mut put = |bytes: &[u8]| {
        h.update((bytes.len() as u64).to_le_bytes());
        h.update(bytes);
    };
    put(&[CACHE_FORMAT_VERSION]);
    put(LOFT_VERSION.as_bytes());
    put(BUILD_ID.as_bytes());
    // The running binary's own build, exactly as [`build_signature`] folds it into a PROGRAM
    // bundle.  `BUILD_ID` is the git HEAD hash and does not move across uncommitted edits, so
    // without this a stdlib snapshot written by one compiler is warm-loaded by the next — which
    // is not hypothetical: `CACHE_FORMAT_VERSION`'s own history records a `DbField` stride
    // change (28 → 29) read back at the wrong stride and panicking in `ir_read`, caught only
    // because someone remembered to bump the format byte by hand.  The two caches answer ONE
    // question — *may a bundle written by another build be read by this one?* — so they fold in
    // the same facts; the format byte goes back to being a second line of defence.
    put(binary_signature_tag().as_bytes());
    put(target_triple().as_bytes());
    put(feature_signature().as_bytes());
    put(semantic_env_signature().as_bytes());
    // Number of stdlib files, then each (name, content).
    h.update((stdlib_sources.len() as u64).to_le_bytes());
    for (name, content) in stdlib_sources {
        let mut field = |bytes: &[u8]| {
            h.update((bytes.len() as u64).to_le_bytes());
            h.update(bytes);
        };
        field(name.as_bytes());
        field(content.as_bytes());
    }
    h.finalize().into()
}

/// The `LOFT_*` variables verified to change neither the parsed `Data` nor the emitted code:
/// watchdog and memory bounds, timing, the cache's own knobs, the diagnostic renderer, the
/// scratch dir and the source anchor a native binary reads at run time.  An ALLOW-list,
/// because the `LOFT_*` surface is large and scattered — every lowering and every `--native`
/// rewrite has a `LOFT_NO_*` switch, read where it acts — so a list of the variables that DO
/// matter drifts the moment someone adds one (`LOFT_NO_WORK_BUFFER` was missing from the one
/// kept here, and a bundle written under it was warm-loaded by the next default run, which
/// then emitted the unpromoted program; loft#1697).  Anything not named here counts, which
/// costs a warm start and never a wrong answer.
pub const INERT_ENV: &[&str] = &[
    "LOFT_TIMING",
    "LOFT_TIMING_LEDGER",
    "LOFT_TIMEOUT",
    "LOFT_TIMEOUT_GRACE",
    "LOFT_MEMORY_LIMIT",
    "LOFT_PROGRAM_CACHE",
    "LOFT_STDLIB_CACHE",
    "LOFT_CACHE_MAX_MB",
    "LOFT_CACHE_TTL_HOURS",
    "LOFT_ERRORS",
    "LOFT_TMPDIR",
    "LOFT_TMPFS_MIN_FREE_MB",
    "LOFT_SOURCE_DIR",
];

/// Whether `name` is an environment variable that may change what a run parses or emits:
/// any `LOFT_*` outside [`INERT_ENV`].
#[must_use]
pub fn env_counts(name: &str) -> bool {
    name.starts_with("LOFT_") && !INERT_ENV.contains(&name)
}

/// Every counting `LOFT_*` variable with its value, sorted by name: two runs that differ in
/// any of them must never share a cached parse.  A switch-ON sweep otherwise poisons a later
/// switch-OFF run of the same file (observed: the @PLN85 54-cell over-free map read 0/54
/// switch-OFF right after a switch-ON run — the real count was 6/54).
#[must_use]
fn semantic_env_signature() -> String {
    let mut vars: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| {
            let k = k.to_string_lossy().into_owned();
            env_counts(&k).then(|| (k, v.to_string_lossy().into_owned()))
        })
        .collect();
    vars.sort_unstable();
    let mut sig = String::new();
    for (k, v) in vars {
        sig.push_str(&k);
        sig.push('=');
        sig.push_str(&v);
        sig.push(';');
    }
    sig
}

/// The target triple this binary was built for, e.g.
/// `x86_64-unknown-linux-gnu`.  A cache must never be shared across
/// architectures.  Assembled from the standard `cfg` values rather than
/// a build-script env var so it works in every build configuration.
#[must_use]
fn target_triple() -> String {
    format!(
        "{}-{}-{}",
        std::env::consts::ARCH,
        std::env::consts::OS,
        std::env::consts::FAMILY,
    )
}

/// @PLN21 Phase 1 — the FULL target triple this loft binary was built for
/// (e.g. `x86_64-unknown-linux-gnu`), stamped by `build.rs` from cargo's
/// `TARGET`.  Authoritative for selecting a `prebuilt/<triple>/` cdylib: unlike
/// [`target_triple`] (the `env::consts` `<arch>-<os>-<family>` form) it captures
/// the libc/abi a prebuilt's portability depends on (gnu vs musl).  Falls back
/// to the `env::consts` form only if the stamp is somehow empty.
#[must_use]
pub fn host_triple() -> String {
    option_env!("LOFT_BUILD_TARGET")
        .filter(|t| !t.is_empty())
        .map_or_else(target_triple, str::to_string)
}

/// Collect the `default/` stdlib sources as `(filename, content)` pairs,
/// sorted by filename for a deterministic cache key.  Reads every `*.loft`
/// file directly under `default_dir` (non-recursive — the stdlib is flat).
///
/// Returns an empty vec on any read error; the caller treats that as
/// "cannot cache" and falls back to a cold parse.
#[must_use]
pub fn collect_stdlib_sources(default_dir: &str) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(default_dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, String)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("loft") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let Ok(content) = std::fs::read_to_string(&path) else {
            return Vec::new();
        };
        out.push((name, content));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The on-disk path for the stdlib bundle keyed by `key` (@PLN11 D2b — the
/// store-format `.store` bundle written by `ir_store::save_bundle`).
///
/// Uses `$XDG_CACHE_HOME/loft/` (or `$HOME/.cache/loft/`), falling back to
/// the system temp dir if neither is set.  The filename embeds the full
/// 64-hex key so distinct builds / feature-sets / stdlib-content never
/// collide.
#[must_use]
pub fn stdlib_cache_path(key: &[u8; 32]) -> std::path::PathBuf {
    cache_base_dir().join(format!("stdlib-{}.store", hex32(key)))
}

/// The loft cache directory: `$XDG_CACHE_HOME/loft/` (or `$HOME/.cache/loft/`),
/// falling back to the system temp dir.
///
/// Also the value the stdlib `cache_dir()` returns (`Stores::os_cache_dir_native`).
#[must_use]
pub fn cache_base_dir() -> std::path::PathBuf {
    std::env::var_os("XDG_CACHE_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
        .join("loft")
}

/// Lower-case 64-hex of a 32-byte key.
#[must_use]
fn hex32(key: &[u8; 32]) -> String {
    let mut hex = String::with_capacity(64);
    for b in key {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    hex
}

/// @PLN11 arc E — SHA-256 of a file's bytes, or `None` if unreadable.  Used to
/// hash every parsed source for the whole-program bundle's drift manifest.
#[must_use]
pub fn file_hash(path: &str) -> Option<[u8; 32]> {
    let bytes = std::fs::read(path).ok()?;
    let mut h = Sha256::new();
    h.update(&bytes);
    Some(h.finalize().into())
}

/// @PLN11 Arc N / N0 — locate `libloft.rlib` for THIS build (dev `target/<prof>/`,
/// its `deps/`, or an installed `<prefix>/share/loft/`).
fn loft_rlib_path() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    if let Some(found) = rlib_candidates(exe_dir).into_iter().find(|c| c.exists()) {
        return Some(found);
    }
    if exe_dir.file_name().is_some_and(|n| n == "bin") {
        let share = exe_dir
            .parent()?
            .join("share")
            .join("loft")
            .join("libloft.rlib");
        if share.exists() {
            return Some(share);
        }
    }
    None
}

/// Candidate `libloft.rlib` locations for a loft binary at `exe_dir`, most
/// authoritative first.
///
/// `deps/libloft.rlib` MUST come before the uplifted `<profile>/libloft.rlib`
/// (#304): the deps copy is what every binary links and what
/// `native_lib::find_loft_rlib` hands to cdylib builds, while the uplifted copy
/// is only refreshed by an explicit `cargo build --lib`.  `cargo test` rebuilds
/// the lib (dev-dep-unified features) into `deps/` and re-uplifts the *binary*
/// but NOT the rlib, so after any `src/` change the uplifted copy describes a
/// loft that no longer exists.  Fingerprinting that stale file made
/// [`loft_build_fingerprint`] validate native cdylibs against a different rlib
/// than they are built against — the full-`make test` viewer crash.  The
/// uplifted path stays as a fallback for trees without `deps/` (e.g. a
/// hand-copied binary + rlib pair).
fn rlib_candidates(exe_dir: &std::path::Path) -> [std::path::PathBuf; 2] {
    [
        exe_dir.join("deps").join("libloft.rlib"),
        exe_dir.join("libloft.rlib"),
    ]
}

/// @PLN11 Arc N / N0 — the canonical fingerprint of THIS loft build, for
/// native-artifact validity (C71 "build fingerprint").  A native artifact links
/// `libloft.rlib`, so it is valid only against the loft build whose rlib it links:
/// this is the rlib's **content** hash (sha256 → `u64`), memoised per process.  A
/// loft codegen/runtime change, a rustc bump, or a cross-target build all rewrite
/// the rlib bytes (cargo rebuilds it), so all flip the fingerprint.  This is the
/// single seam the native-artifact cache — and, eventually, the library
/// validation layer — consults; do NOT key on mtime or git-`BUILD_ID` (both miss
/// an *uncommitted* dev rebuild, which is exactly when the stale-link bites).
///
/// Falls back to the loft **binary**'s content hash when the rlib can't be located
/// (a `--bin`-built dev loft, whose only rlib is the *hashed* `deps/libloft-<hash>.rlib`
/// — not the bare `libloft.rlib` [`loft_rlib_path`] looks for).  The binary is rebuilt
/// together with the rlib, so its hash flips on the same changes: a valid staleness
/// signal.  This matters because the old `0`-when-absent path made
/// [`native_artifact_fingerprint_matches`] **match anything**, silently reusing a stale
/// cdylib → the `native function not loaded` stub (the M1c viewer failure).  Only reaches
/// `0` if neither rlib nor exe can be hashed; a source-less prebuilt install never hits
/// this gate (it loads its committed `.so` via the prebuilt path), so the fallback can't
/// force a doomed rebuild there.
#[must_use]
pub fn loft_build_fingerprint() -> u64 {
    use std::sync::OnceLock;
    static FP: OnceLock<u64> = OnceLock::new();
    *FP.get_or_init(|| {
        warn_if_uplifted_rlib_stale();
        let fp = loft_rlib_path()
            .or_else(|| std::env::current_exe().ok())
            .and_then(|p| file_hash(&p.to_string_lossy()))
            .map_or(0, |h| {
                u64::from_le_bytes(h[..8].try_into().unwrap_or([0; 8]))
            });
        if fp == 0 {
            // The `fp == 0` path makes `native_artifact_fingerprint_matches`
            // "match anything" — i.e. silently reuse whatever artifact is on
            // disk regardless of the loft build that produced it.  That is the
            // stale-link footgun; never let it pass unannounced.
            eprintln!(
                "loft: warning — cannot fingerprint this loft build (no readable \
                 libloft.rlib or executable); native artifacts cannot be staleness-checked \
                 and are reused as-is. If results look stale, run `loft cache prune --all`."
            );
        }
        fp
    })
}

/// Loud staleness signal for the dev `target/<profile>/` layout: the uplifted
/// bare `libloft.rlib` is refreshed only by an explicit `cargo build --lib`, so
/// after a `--bin` rebuild it lags the `deps/libloft.rlib` loft actually links
/// (deps-first, #304).  The fingerprint paths route around it, but the bare copy
/// is a trap for anything that reads it directly (a hand-rolled `rustc --extern`,
/// a script, future code).  Warn once when the bare copy is older than deps — a
/// cheap, definitive signal that the visible-but-unused rlib is stale.  Both
/// absent (installed / prebuilt loft) → no bare-vs-deps split, nothing to warn.
fn warn_if_uplifted_rlib_stale() {
    let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(std::path::Path::to_path_buf))
    else {
        return;
    };
    let deps = exe_dir.join("deps").join("libloft.rlib");
    let bare = exe_dir.join("libloft.rlib");
    let (Ok(deps_t), Ok(bare_t)) = (
        std::fs::metadata(&deps).and_then(|m| m.modified()),
        std::fs::metadata(&bare).and_then(|m| m.modified()),
    ) else {
        return;
    };
    if bare_t < deps_t {
        eprintln!(
            "loft: warning — {} is STALE (older than deps/libloft.rlib, which loft links). \
             loft routes around it (deps-first), but tools reading the bare path get an \
             outdated rlib. Run `cargo build --lib` to refresh it, or delete it.",
            bare.display()
        );
    }
}

/// Fingerprint for REGISTRY-cdylib cache validity (`extensions::auto_build_native`).
///
/// Distinct from [`loft_build_fingerprint`] (the `libloft.rlib` content hash):
/// a registry cdylib links **loft-ffi** (the C-ABI), never `libloft.rlib` —
/// verified by inspection (the cdylib has zero loft undefined symbols; its only
/// NEEDED libs are system ones).  So its staleness depends on loft-ffi, not the
/// interpreter.  This returns the build-time hash of loft-ffi's **source**
/// (stamped by `build.rs` into `LOFT_FFI_FINGERPRINT`): identical across
/// debug/release/test profiles, so the shared `~/.loft/build-cache` is reused
/// across every loft build in a CI job instead of cross-invalidating on the
/// per-profile rlib hash — yet it still flips on any real loft-ffi change (ABI
/// or impl, even an un-version-bumped dev edit).  Falls back to
/// [`loft_build_fingerprint`] (the old rlib-hash gate) when the stamp is absent
/// or 0, so it is never *less* safe than before.
#[must_use]
pub fn loft_ffi_fingerprint() -> u64 {
    option_env!("LOFT_FFI_FINGERPRINT")
        .and_then(|s| s.parse::<u64>().ok())
        .filter(|&h| h != 0)
        .unwrap_or_else(loft_build_fingerprint)
}

/// #274 — fingerprint of the RUSTFLAGS this loft build used (`LOFT_BUILD_RUSTFLAGS`,
/// stamped by `build.rs`).  A package's native rlib bundles shared transitive deps
/// (e.g. `libloading`) whose SVH must match loft's own copy at the consumer link, and
/// that SVH depends on RUSTFLAGS — which `loft_ffi_fingerprint` does NOT capture.  So
/// a graphics rlib built by a `-g` loft, reused by a no-`-g` loft (loft-ffi source
/// unchanged), links a `-g` `libloading` against loft's no-`-g` copy: same
/// `StableCrateId`, different SVH, "colliding StableCrateId" at link.  Folding this
/// into the native-artifact cache key invalidates on a flag change while preserving
/// the cross-profile sharing the loft-ffi key was chosen for (same flags → shared).
/// `DefaultHasher` is seeded with fixed keys, so the value is stable across runs.
#[must_use]
pub fn rustflags_fingerprint() -> u64 {
    rustflags_fp_of(env!("LOFT_BUILD_RUSTFLAGS"))
}

/// Pure core of [`rustflags_fingerprint`] (takes the flag string so it is testable).
fn rustflags_fp_of(flags: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    flags.hash(&mut h);
    h.finish()
}

/// #274 — the native-artifact cache key: `loft_ffi_fingerprint` (the published C-ABI
/// the cdylib links) combined with [`rustflags_fingerprint`] (the flags that fix the
/// rlib's shared-dep SVH), folded with `LOFT_VERSION`.  `.max(1)` keeps it out of the
/// `fp == 0` match-anything sentinel in [`native_artifact_fingerprint_matches`].
///
/// What is deliberately NOT in it: the loft build itself — neither `BUILD_ID` nor a
/// content hash.  This key stamps a package's HAND-WRITTEN `native/` crate, whose inputs
/// are its own sources (`native_crate_newer_than`, loft#965), loft-ffi and RUSTFLAGS: no
/// crate in the tree or in the registry depends on the `loft` crate, and loft's codegen
/// writes nothing into them (every `native/Cargo.toml` names loft-ffi, loft-ffi-macros
/// and loft-ffi-build only — measured 2026-09-08).  The artifact a codegen change DOES
/// invalidate is the generated `loft_auto_*` cdylib, and that one carries
/// [`loft_build_fingerprint`] — the rlib's content hash — in its file NAME
/// (`cached_or_build_shared_cdylib`, loft#715), so it moves on every rebuild, committed
/// or not.  Folding `BUILD_ID` (the git HEAD) here, #433's fix from before loft#715,
/// bought nothing that name does not, and cost a rebuild of every package cdylib on
/// every commit: each CI run's `cdylibstale` events, a cold `cache warm` after every
/// local commit, and a stamp ping-pong between checkouts sharing `~/.loft/build-cache`.
/// Nor could a HEAD-keyed stamp see an uncommitted edit, so had a crate depended on
/// loft it would have answered green on a stale one.  @PLN159 phase C.
#[must_use]
pub fn native_artifact_cache_key() -> u64 {
    // An empty recipe leaves the key as it was, so a platform with nothing extra is not
    // made to rebuild every package for a change that is not its own.
    let rustflags_and_recipe = if NATIVE_LINK_RECIPE.is_empty() {
        rustflags_fingerprint()
    } else {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        rustflags_fingerprint().hash(&mut h);
        NATIVE_LINK_RECIPE.hash(&mut h);
        h.finish()
    };
    native_artifact_cache_key_of(
        combine_native_cache_key(loft_ffi_fingerprint(), rustflags_and_recipe),
        LOFT_VERSION,
    )
}

/// The platform part of how a package cdylib is linked, beyond the baked RUSTFLAGS —
/// one home, read by the build (`extensions::relocatable_dylib_flags`) and folded into
/// [`native_artifact_cache_key`], so a change to it rebuilds every cached cdylib instead
/// of reusing one linked the old way.  On macOS the linker drops the debug symbols itself
/// and the post-link `strip` is off: that strip left a TLS package's string table
/// misaligned, and the linker then refused it (`mis-aligned LINKEDIT string pool`).
pub const NATIVE_LINK_RECIPE: &str = if cfg!(target_os = "macos") {
    "-Clink-arg=-Wl,-S"
} else {
    ""
};

/// Pure core of [`native_artifact_cache_key`] (testable without the build-time env):
/// fold `LOFT_VERSION` into the ABI/RUSTFLAGS key — a release is the one floor under
/// which every package cdylib is rebuilt once.
fn native_artifact_cache_key_of(abi_rustflags_key: u64, version: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    abi_rustflags_key.hash(&mut h);
    version.hash(&mut h);
    h.finish().max(1)
}

/// Pure core of [`native_artifact_cache_key`] (testable without the build-time env).
fn combine_native_cache_key(ffi_fp: u64, rustflags_fp: u64) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    ffi_fp.hash(&mut h);
    rustflags_fp.hash(&mut h);
    h.finish().max(1)
}

/// @PLN11 Arc N / N0 — path of a native artifact's build-fingerprint sidecar.
fn fp_sidecar(profile_dir: &std::path::Path) -> std::path::PathBuf {
    profile_dir.join(".loft-build-fp")
}

/// @PLN11 Arc N / N0 — true iff `profile_dir` carries a build-fingerprint sidecar
/// equal to `fp`.  A missing / stale / unreadable sidecar returns `false`, so the
/// caller rebuilds — this is what makes a loft change (new fingerprint) invalidate
/// an already-built package rlib / cdylib instead of silently linking the stale
/// one (the `make rebuild-native-cdylibs` hazard).  `fp == 0` (no rlib to
/// fingerprint) matches anything → existence-only fallback, no spurious churn.
#[must_use]
pub fn native_artifact_fingerprint_matches(profile_dir: &std::path::Path, fp: u64) -> bool {
    if fp == 0 {
        return true;
    }
    std::fs::read_to_string(fp_sidecar(profile_dir))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .is_some_and(|stored| stored == fp)
}

/// @PLN11 Arc N / N0 — stamp `profile_dir` with `fp` after building an artifact
/// there (best-effort; no-op when `fp == 0`).
pub fn write_native_artifact_fingerprint(profile_dir: &std::path::Path, fp: u64) {
    if fp != 0 {
        let _ = std::fs::write(fp_sidecar(profile_dir), fp.to_string());
    }
}

/// How many loft builds one wasm runtime rlib stays stamped for.
const RUNTIME_FPS_KEPT: usize = 4;

/// Is the wasm runtime rlib in `profile_dir` current for the loft build `fp`?
///
/// The runtime rlib is compiled from loft's SOURCES and links nothing of the running
/// binary, so every loft build of one source tree can use it — a release and a debug
/// binary alike.  Its sidecar therefore holds a SET of fingerprints, not one: stamped
/// with one, the release and the debug binary each read the other's stamp as stale, and
/// every alternation took the global build lock for a cargo no-op — behind other tests'
/// native builds, minutes per call on a CI runner.  A package cdylib links the host rlib
/// and keeps the single stamp ([`native_artifact_fingerprint_matches`]).
#[must_use]
pub fn runtime_fingerprint_matches(profile_dir: &std::path::Path, fp: u64) -> bool {
    fp == 0
        || std::fs::read_to_string(fp_sidecar(profile_dir))
            .is_ok_and(|s| s.split_whitespace().any(|t| t.parse::<u64>() == Ok(fp)))
}

/// Stamp the wasm runtime rlib in `profile_dir` as current for `fp` after cargo ran.
///
/// `rebuilt` says whether cargo actually changed the rlib.  A rebuild describes a new
/// source tree, so the set restarts at `fp` alone; a no-op confirms the rlib already
/// matches the tree, so `fp` joins the builds it serves (the most recent
/// [`RUNTIME_FPS_KEPT`]).
pub fn stamp_runtime_fingerprint(profile_dir: &std::path::Path, fp: u64, rebuilt: bool) {
    if fp == 0 {
        return;
    }
    let mut fps: Vec<u64> = if rebuilt {
        Vec::new()
    } else {
        std::fs::read_to_string(fp_sidecar(profile_dir))
            .map(|s| {
                s.split_whitespace()
                    .filter_map(|t| t.parse().ok())
                    .collect()
            })
            .unwrap_or_default()
    };
    fps.retain(|&f| f != fp);
    fps.push(fp);
    let keep = fps.len().saturating_sub(RUNTIME_FPS_KEPT);
    let text: Vec<String> = fps[keep..].iter().map(u64::to_string).collect();
    let _ = std::fs::write(fp_sidecar(profile_dir), text.join(" "));
}

/// The fingerprint a native artifact dir was stamped with, if any — the
/// visibility companion to [`native_artifact_fingerprint_matches`] (which only
/// returns a bool).  Extends the loud-fallback theme: when a cached cdylib is
/// rejected and rebuilt, the caller can SHOW the stamped-vs-current fp so the
/// cause is legible (a flipped `libloft.rlib` hash vs an absent sidecar) — the
/// data we need before deciding whether to re-key the cdylib gate on the stable
/// published `loft-ffi` version instead of the rlib hash.
#[must_use]
pub fn native_artifact_stamped_fp(profile_dir: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(fp_sidecar(profile_dir))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
}

/// The reason [`rustc_mismatch`] gives when there is no `rustc` at all — a missing toolchain,
/// which no runtime rebuild can cure, as opposed to a changed one.
pub const RUSTC_NOT_FOUND: &str = "rustc not found";

/// The rustc this loft was BUILT with (the `LOFT_BUILD_RUSTC` stamp from
/// build.rs) vs the one a plain `rustc` resolves RIGHT NOW — which depends
/// on PATH and rustup's cwd-sensitive toolchain overrides, so it can
/// differ between two invocations of the same loft binary (the repo's
/// `rust-toolchain.toml` pin vs the rustup default in `/tmp`).  Every
/// build that links the SVH-locked `libloft.rlib` is DOOMED under a
/// mismatch (rustc E0514, "compiled by an incompatible version") — so
/// build paths ask this BEFORE spawning, and skip to the interpreter with
/// a one-line reason instead of spewing a compiler error and leaving a
/// half-written target.  `None` = proceed (matching, or no stamp to
/// compare); `Some(reason)` = mismatch / no usable rustc.  Memoised: the
/// answer cannot change within a process.
#[must_use]
pub fn rustc_mismatch() -> Option<&'static str> {
    use std::sync::OnceLock;
    static REASON: OnceLock<Option<String>> = OnceLock::new();
    REASON
        .get_or_init(|| {
            let stamp = option_env!("LOFT_BUILD_RUSTC").unwrap_or("");
            if stamp.is_empty() {
                return None;
            }
            let live = std::process::Command::new("rustc")
                .arg("--version")
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
            match live {
                Some(v) if v == stamp => None,
                Some(v) => Some(format!(
                    "rustc changed since this loft was built ({stamp} → {v})"
                )),
                None => Some(RUSTC_NOT_FOUND.to_string()),
            }
        })
        .as_deref()
}

/// @PLN11 Arc N / N0 — clear a native build target whose existing artifact
/// was produced by ANOTHER loft build (fingerprint mismatch), so the
/// follow-up `cargo build` cannot no-op over it.  cargo is structurally
/// blind to a loft upgrade here: the package crate's sources and the
/// captured RUSTFLAGS can be byte-identical across loft builds while the
/// generated ABI changed — and the post-build stamp would then launder the
/// stale artifact with the NEW fingerprint (probed 2026-06-12 on
/// graphics-0.1.0: a real loft rebuild left the cdylib at the old ABI —
/// the store-65535 sentinel-deref class).  Returns whether it cleared.
/// Clears ONLY on a present-and-mismatched sidecar (proof of another loft
/// build); an absent sidecar is unknown provenance — commonly a legitimate
/// hand-built artifact — and keeps the build-and-stamp path.  `fp == 0`
/// (nothing to fingerprint) never clears, matching
/// [`native_artifact_fingerprint_matches`]' existence-only fallback.
pub fn clear_stale_native_target(
    target_root: &std::path::Path,
    lib_name: &str,
    rlib_name: &str,
    fp: u64,
) -> bool {
    if fp == 0 {
        return false;
    }
    let release_dir = target_root.join("release");
    let artifact_present =
        release_dir.join(lib_name).exists() || release_dir.join(rlib_name).exists();
    // Clear only on a PRESENT-and-mismatched sidecar — proof the artifact
    // was stamped by another loft build.  An ABSENT sidecar is unknown
    // provenance and commonly a legitimate HAND-BUILT artifact (`cargo
    // build` in the library's `native/` dir — the documented workflow; the
    // `tests/lib` fixture cdylibs are exactly this) — deleting those breaks
    // the dev loop, so they keep the pre-existing build-and-stamp path.
    let stamped = std::fs::read_to_string(release_dir.join(".loft-build-fp"))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok());
    if artifact_present && stamped.is_some_and(|s| s != fp) {
        let _ = std::fs::remove_dir_all(target_root);
        return true;
    }
    false
}

/// @PLN11 Arc N / N3 (Step 4) — path of a library's *last-run source-hash* sidecar.
/// Dev-interpret-on-edit compares this run's source hash against the one recorded
/// here to decide "still being edited (changed since last run → interpret)" vs
/// "stable (unchanged → build the cdylib)".
fn run_hash_sidecar(profile_dir: &std::path::Path) -> std::path::PathBuf {
    profile_dir.join(".loft-run-hash")
}

/// Read the source hash recorded on the previous run (None if absent/unreadable).
#[must_use]
pub fn read_run_source_hash(profile_dir: &std::path::Path) -> Option<u64> {
    std::fs::read_to_string(run_hash_sidecar(profile_dir))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
}

/// Record `hash` as this run's library source hash (creates `profile_dir` if
/// needed, since a library that has only ever interpreted has no artifact dir yet).
/// Best-effort.
pub fn write_run_source_hash(profile_dir: &std::path::Path, hash: u64) {
    let _ = std::fs::create_dir_all(profile_dir);
    let _ = std::fs::write(run_hash_sidecar(profile_dir), hash.to_string());
}

/// @PLN11 arc E — the `(bundle, manifest)` paths for the whole-program cache of
/// the script at `script_abspath`, resolved against the `--lib` search path
/// `lib_dirs`.  The manifest (every parsed source + its content hash) detects drift
/// in any input — stdlib, lazily-loaded libs, or the script.
///
/// **The search path is part of the key, not just of what it resolved to** (loft#930).
/// One script run against two library trees is two programs, and keying on the script
/// alone made the second silently reuse the first's build.  Nothing downstream could
/// catch it: the manifest re-validates the files the FIRST run resolved, and those are
/// still unchanged, so the drift check passes and hands back the wrong library's code.
/// It looked responsive — an in-place edit of the bound library did rebuild — while
/// ignoring the flag that selects WHICH library is edited, which is what made it hard
/// to notice.  A consumer's A/B harness compared an arm against itself and read the
/// byte-identical output as the strongest possible pass.  The environment is part of it
/// for the same reason: a lowering switch parses another program (`env_counts`, loft#1697).
#[must_use]
pub fn program_cache_paths(
    script_abspath: &str,
    lib_dirs: &[String],
) -> (std::path::PathBuf, std::path::PathBuf) {
    let mut h = Sha256::new();
    h.update(script_abspath.as_bytes());
    // Length-prefixed so `["ab", "c"]` and `["a", "bc"]` cannot hash alike, and in
    // ORDER, because the search path is ordered: the same dirs listed differently can
    // resolve a name to a different file.
    for dir in lib_dirs {
        h.update(u32::try_from(dir.len()).unwrap_or(u32::MAX).to_le_bytes());
        h.update(dir.as_bytes());
    }
    // A slot per environment: a run under a lowering switch parses a different program, and
    // sharing one slot would make every run after a switch run cold (the stdlib key, which
    // the manifest pins, folds in the same signature — so a shared slot would miss, not lie).
    let env = semantic_env_signature();
    h.update(u32::try_from(env.len()).unwrap_or(u32::MAX).to_le_bytes());
    h.update(env.as_bytes());
    let key: [u8; 32] = h.finalize().into();
    let base = cache_base_dir();
    let stem = format!("program-{}", hex32(&key));
    (
        base.join(format!("{stem}.store")),
        base.join(format!("{stem}.manifest")),
    )
}

/// @PLN166 B3 — the sidecar beside a program's manifest that names the NATIVE binary
/// built from the program the manifest validates (`program-<key>.native`).  It lives
/// with the manifest because its validity is the manifest's: a native run that finds
/// both current execs the binary without parsing, and one evicts with the other.
#[must_use]
pub fn native_sidecar_path(manifest: &std::path::Path) -> std::path::PathBuf {
    manifest.with_extension("native")
}

/// @PLN11 G2 / track 1 — default budget (MiB) for the program-cache directory
/// before eviction kicks in.  ~512 MiB ≈ 70 bundles at the measured ~7 MiB each;
/// overridable via `LOFT_CACHE_MAX_MB`.
const DEFAULT_CACHE_MAX_MB: u64 = 512;

/// The program-cache size budget in bytes (`LOFT_CACHE_MAX_MB` × 1 MiB, default
/// [`DEFAULT_CACHE_MAX_MB`]).  A malformed value falls back to the default.
#[must_use]
fn program_cache_budget_bytes() -> u64 {
    std::env::var("LOFT_CACHE_MAX_MB")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_CACHE_MAX_MB)
        .saturating_mul(1024 * 1024)
}

/// @PLN11 Arc N / N1 — the idle-TTL after which an *unused* cached artifact is
/// evicted (`LOFT_CACHE_TTL_HOURS`, default 24 h).  A re-use ([`touch_now`]) or a
/// re-save refreshes the entry's mtime, so an actively-used "library set" persists
/// indefinitely while a one-off ages out — "store common sets longer, clear when
/// idle 24 h" (C71).  A malformed value falls back to the default.
#[must_use]
fn cache_ttl() -> std::time::Duration {
    let hours = std::env::var("LOFT_CACHE_TTL_HOURS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(24);
    std::time::Duration::from_secs(hours.saturating_mul(3600))
}

/// @PLN11 Arc N / N1 — mark a cached file as used *now* (touch-on-use): bumps its
/// modification time so the idle-TTL GC keeps it.  Best-effort (no-op if the file
/// is missing / unopenable).
///
/// The handle is opened for WRITE only on Windows, where `SetFileTime` needs it.  On Unix
/// `futimens` works on a read-only descriptor, and the files touched here are cached
/// executables and loaded cdylibs: a write handle on a Mach-O another process is running
/// is what macOS's code-signing checks act on (a cached native test binary was killed by
/// a signal on its first run after this took a write handle everywhere).
pub fn touch_now(path: &std::path::Path) {
    let mut open = std::fs::OpenOptions::new();
    open.read(true);
    if cfg!(windows) {
        open.write(true);
    }
    if let Ok(f) = open.open(path) {
        let _ = f.set_modified(std::time::SystemTime::now());
    }
}

/// @PLN11 G2 / track 1 + Arc N / N1 — bound cache growth.  With the cache
/// default-on each distinct script gets its own `program-<hash>.store` bundle
/// (~7 MiB) and nothing removes them.  Eviction is **idle-TTL primary**
/// ([`cache_ttl`]) — drop any bundle not used within the TTL — with the size-cap
/// ([`program_cache_budget_bytes`]) as a runaway backstop.  Whole (`.store` +
/// `.manifest` + `.native`) sets go together.  Best-effort.
pub fn prune_program_cache() {
    prune_dir(&cache_base_dir(), program_cache_budget_bytes(), cache_ttl());
}

/// The eviction core, factored out of [`prune_program_cache`] so it is testable
/// against a temp dir with an explicit budget + TTL (no env, no global cache dir).
/// Only `program-*.store` files (and their sibling `.manifest` / `.native`) are considered;
/// any other cache file (e.g. the stdlib bundle) is left untouched.  Phase 1 drops
/// every bundle idle longer than `ttl` (the primary policy); phase 2 is the
/// oldest-first size-cap backstop on what remains.
fn prune_dir(base: &std::path::Path, budget_bytes: u64, ttl: std::time::Duration) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    struct Bundle {
        store: std::path::PathBuf,
        mtime: std::time::SystemTime,
        size: u64,
    }
    let mut bundles: Vec<Bundle> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let is_program_store = path.extension().and_then(|x| x.to_str()) == Some("store")
            && path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("program-"));
        if !is_program_store {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        bundles.push(Bundle {
            store: path,
            mtime: meta.modified().unwrap_or(std::time::UNIX_EPOCH),
            size: meta.len(),
        });
    }
    let now = std::time::SystemTime::now();
    let remove_pair = |store: &std::path::Path| {
        let _ = std::fs::remove_file(store);
        let manifest = store.with_extension("manifest");
        let _ = std::fs::remove_file(native_sidecar_path(&manifest));
        let _ = std::fs::remove_file(manifest);
    };
    // Phase 1 — idle-TTL: drop any bundle not used within `ttl`.
    bundles.retain(|b| {
        if now.duration_since(b.mtime).unwrap_or_default() > ttl {
            remove_pair(&b.store);
            false
        } else {
            true
        }
    });
    // Phase 2 — size-cap backstop on the survivors.
    let mut total: u64 = bundles.iter().map(|b| b.size).sum();
    if total <= budget_bytes {
        return;
    }
    bundles.sort_by_key(|b| b.mtime); // oldest first
    for b in &bundles {
        if total <= budget_bytes {
            break;
        }
        remove_pair(&b.store);
        total = total.saturating_sub(b.size);
    }
}

#[cfg(test)]
mod tests {
    /// loft#930 — the `--lib` search path is part of the program-cache key.  Keying on
    /// the script path alone made one script run against two library trees reuse the
    /// FIRST tree's build, and the drift manifest could not catch it: it re-validates the
    /// files that run resolved, which are still unchanged.  A consumer's A/B harness
    /// compared an arm against itself and read the identical output as a pass.
    #[test]
    fn the_lib_search_path_is_part_of_the_program_cache_key() {
        let script = "/abs/path/prog.loft";
        let (bundle1, manifest1) = super::program_cache_paths(script, &["/abs/lib1".to_string()]);
        let (bundle2, manifest2) = super::program_cache_paths(script, &["/abs/lib2".to_string()]);
        assert_ne!(
            bundle1, bundle2,
            "a different --lib must key a different bundle"
        );
        assert_ne!(manifest1, manifest2);

        // Same script, same path → the same slot, or nothing would ever hit warm.
        let (again, _) = super::program_cache_paths(script, &["/abs/lib1".to_string()]);
        assert_eq!(bundle1, again, "an unchanged --lib must reuse its bundle");

        // ORDER matters: the search path is ordered, so the same dirs listed differently
        // can resolve a name to a different file.
        let ab = super::program_cache_paths(script, &["/a".to_string(), "/b".to_string()]);
        let ba = super::program_cache_paths(script, &["/b".to_string(), "/a".to_string()]);
        assert_ne!(ab.0, ba.0, "search-path order must key distinctly");

        // Length-prefixed, so a split cannot be forged by concatenation.
        let split = super::program_cache_paths(script, &["/ab".to_string(), "/c".to_string()]);
        let joined = super::program_cache_paths(script, &["/ab/c".to_string()]);
        assert_ne!(
            split.0, joined.0,
            "concatenation must not collide with a split"
        );

        // And no lib dirs at all is its own key, not any of the above.
        let none = super::program_cache_paths(script, &[]);
        assert_ne!(none.0, bundle1);
    }

    use super::*;

    fn sample() -> Vec<(String, String)> {
        vec![
            ("01_code.loft".to_string(), "fn a() {}".to_string()),
            ("02_files.loft".to_string(), "fn b() {}".to_string()),
        ]
    }

    #[test]
    fn key_is_deterministic() {
        // Identical inputs → identical key, across repeated calls.
        let a = stdlib_cache_key(&sample());
        let b = stdlib_cache_key(&sample());
        assert_eq!(a, b, "same inputs must yield the same key");
    }

    #[test]
    fn key_changes_on_stdlib_content() {
        // The retirement bug: a stdlib edit MUST change the key.
        let base = stdlib_cache_key(&sample());
        let mut edited = sample();
        edited[0].1.push_str(" // tweak");
        assert_ne!(
            base,
            stdlib_cache_key(&edited),
            "editing default/*.loft content must invalidate the cache"
        );
    }

    #[test]
    fn key_changes_on_stdlib_name() {
        let base = stdlib_cache_key(&sample());
        let mut renamed = sample();
        renamed[0].0 = "01_core.loft".to_string();
        assert_ne!(
            base,
            stdlib_cache_key(&renamed),
            "renaming a stdlib file must invalidate the cache"
        );
    }

    #[test]
    fn key_changes_on_file_count() {
        let base = stdlib_cache_key(&sample());
        let mut more = sample();
        more.push(("03_text.loft".to_string(), "fn c() {}".to_string()));
        assert_ne!(
            base,
            stdlib_cache_key(&more),
            "adding a stdlib file must invalidate the cache"
        );
    }

    #[test]
    fn key_is_order_sensitive() {
        // Reordering files is a different stdlib (load order matters for
        // parse); the key must reflect it.
        let base = stdlib_cache_key(&sample());
        let mut swapped = sample();
        swapped.swap(0, 1);
        assert_ne!(
            base,
            stdlib_cache_key(&swapped),
            "stdlib file order is part of the key"
        );
    }

    #[test]
    fn boundary_shift_changes_key() {
        // Moving a byte across the name/content boundary must not
        // collide (length-prefixing guarantees this).
        let a = stdlib_cache_key(&[("ab".to_string(), "c".to_string())]);
        let b = stdlib_cache_key(&[("a".to_string(), "bc".to_string())]);
        assert_ne!(a, b, "field boundaries must not be ambiguous");
    }

    #[test]
    fn feature_signature_is_stable() {
        // Two calls in the same build must agree (deterministic order).
        assert_eq!(feature_signature(), feature_signature());
        // The default native build enables threading; sanity-check the
        // signature is non-empty there so the key actually varies by
        // build config.
        #[cfg(feature = "threading")]
        assert!(feature_signature().contains("threading"));
    }

    #[test]
    fn collect_stdlib_sources_reads_default_dir_sorted() {
        // The real default/ dir relative to the crate root.
        let srcs = collect_stdlib_sources("default");
        assert!(
            srcs.len() >= 3,
            "expected the stdlib .loft files, got {}",
            srcs.len()
        );
        assert!(
            srcs.iter()
                .all(|(n, _)| std::path::Path::new(n).extension() == Some("loft".as_ref()))
        );
        // Sorted by filename → deterministic key.
        let mut sorted = srcs.clone();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(srcs, sorted, "sources must be returned sorted by filename");
        // 01_code.loft is always present and non-empty.
        assert!(
            srcs.iter()
                .any(|(n, c)| n == "01_code.loft" && !c.is_empty())
        );
    }

    // #274 — the native-artifact cache key MUST change when the RUSTFLAGS that fix a
    // shared transitive dep's SVH change; otherwise a package rlib built by a `-g`
    // loft is reused by a no-`-g` loft and its bundled `libloading` collides with
    // loft's copy at the consumer link ("colliding StableCrateId").  Guards the pure
    // cores so a regression is caught without an end-to-end native build.
    #[test]
    fn native_cache_key_is_rustflags_sensitive() {
        // Different flags → different rustflags fingerprint.
        assert_ne!(
            rustflags_fp_of("-g"),
            rustflags_fp_of(""),
            "a flag change must change the rustflags fingerprint"
        );
        assert_eq!(
            rustflags_fp_of("-g"),
            rustflags_fp_of("-g"),
            "the fingerprint must be deterministic"
        );
        // …and that propagates through the combined cache key.
        let ffi = 0xABCD_1234_5678_9ABC;
        assert_ne!(
            combine_native_cache_key(ffi, rustflags_fp_of("-g")),
            combine_native_cache_key(ffi, rustflags_fp_of("")),
            "the cache key must change when RUSTFLAGS change (same loft-ffi)"
        );
        // A loft-ffi change still changes the key (the original dimension is kept).
        assert_ne!(
            combine_native_cache_key(ffi, rustflags_fp_of("-g")),
            combine_native_cache_key(ffi ^ 1, rustflags_fp_of("-g")),
            "the cache key must still track the loft-ffi fingerprint"
        );
        // Never 0 — that is the `native_artifact_fingerprint_matches` match-anything
        // sentinel; the key tripping it would silently reuse any stale rlib.
        assert_ne!(
            combine_native_cache_key(0, 0),
            0,
            "key must avoid the 0 sentinel"
        );
        assert_ne!(native_artifact_cache_key(), 0);
    }

    #[test]
    fn native_cache_key_is_independent_of_the_loft_build() {
        // @PLN159 phase C — the key stamps hand-written package crates, which do not
        // depend on the loft crate; the generated `loft_auto_*` cdylib is content-
        // addressed by its NAME (loft#715).  So the loft build — committed or not — must
        // NOT move this key: when #433 folded BUILD_ID in, every commit rebuilt every
        // package cdylib for nothing.  The N0 framework's
        // `stale_fingerprint_forces_rebuild_and_restamp` still proves the sidecar
        // MECHANISM (a clobbered sidecar → rebuild); this pins the KEY's inputs.
        let abi = combine_native_cache_key(0xABCD_1234, 0x5678_9ABC);
        let base = native_artifact_cache_key_of(abi, "1.0.0");
        assert_eq!(
            base,
            native_artifact_cache_key_of(abi, "1.0.0"),
            "the key is a pure function of ABI, RUSTFLAGS and version"
        );
        assert_ne!(
            base,
            native_artifact_cache_key_of(abi, "1.1.0"),
            "a LOFT_VERSION bump must move the key"
        );
        // The ABI/RUSTFLAGS dimension still matters after folding the version in.
        assert_ne!(
            base,
            native_artifact_cache_key_of(abi ^ 1, "1.0.0"),
            "an FFI-ABI / RUSTFLAGS change must still move the key"
        );
        // The key proper is never the 0 match-anything sentinel.
        assert_ne!(native_artifact_cache_key_of(0, ""), 0);
    }

    #[test]
    fn collect_stdlib_sources_missing_dir_is_empty() {
        let unexpected = collect_stdlib_sources("nonexistent-dir-xyz");
        assert!(unexpected.is_empty(), "{unexpected:?}");
    }

    #[test]
    fn native_artifact_fingerprint_sidecar_gate() {
        let dir = std::env::temp_dir().join(format!("loft_fpgate_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // No sidecar yet → a non-zero fp must NOT match (forces a rebuild).
        assert!(!native_artifact_fingerprint_matches(&dir, 0xABCD));
        // Stamp it: the same fp matches, a different (stale) one does not.
        write_native_artifact_fingerprint(&dir, 0xABCD);
        assert!(native_artifact_fingerprint_matches(&dir, 0xABCD));
        assert!(
            !native_artifact_fingerprint_matches(&dir, 0x1234),
            "a stale loft fingerprint must not match → forces a rebuild"
        );
        // fp == 0 (can't fingerprint) is the existence-only fallback → matches.
        assert!(native_artifact_fingerprint_matches(&dir, 0));
        // Writing fp == 0 is a no-op — it must not clobber the real stamp.
        write_native_artifact_fingerprint(&dir, 0);
        assert!(
            native_artifact_fingerprint_matches(&dir, 0xABCD),
            "a 0-fp write must not overwrite the existing stamp"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rlib_candidates_prefer_deps_over_uplifted() {
        // #304 — the fingerprint must hash `deps/libloft.rlib` (the copy every
        // binary links and `native_lib::find_loft_rlib` builds cdylibs against),
        // not the uplifted `<profile>/libloft.rlib`: `cargo test` rewrites the
        // deps copy and the binary but leaves the uplifted copy stale, so the
        // old uplifted-first order validated cdylibs against a dead loft build.
        let exe_dir = std::path::Path::new("target/release");
        let [first, second] = rlib_candidates(exe_dir);
        assert_eq!(first, exe_dir.join("deps").join("libloft.rlib"));
        assert_eq!(second, exe_dir.join("libloft.rlib"));
    }

    /// The wasm runtime rlib serves every loft build of one source tree: a release and a
    /// debug binary alternating keep both stamps (no lock, no cargo per call), a real
    /// rebuild forgets every build that does not match it, and the set stays bounded.
    #[test]
    fn a_runtime_rlib_is_stamped_for_every_build_it_serves() {
        let dir = std::env::temp_dir().join(format!("loft_runtime_fp_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // The single-number sidecar every older loft wrote still reads.
        write_native_artifact_fingerprint(&dir, 11);
        assert!(runtime_fingerprint_matches(&dir, 11));
        assert!(!runtime_fingerprint_matches(&dir, 22));
        // A cargo no-op for build 22: both builds are current.
        stamp_runtime_fingerprint(&dir, 22, false);
        assert!(runtime_fingerprint_matches(&dir, 11) && runtime_fingerprint_matches(&dir, 22));
        // A real rebuild for build 33: only 33.
        stamp_runtime_fingerprint(&dir, 33, true);
        assert!(runtime_fingerprint_matches(&dir, 33));
        assert!(!runtime_fingerprint_matches(&dir, 11) && !runtime_fingerprint_matches(&dir, 22));
        // Bounded: the oldest falls out after RUNTIME_FPS_KEPT.
        for fp in 40..40 + RUNTIME_FPS_KEPT as u64 {
            stamp_runtime_fingerprint(&dir, fp, false);
        }
        assert!(!runtime_fingerprint_matches(&dir, 33));
        assert!(runtime_fingerprint_matches(
            &dir,
            40 + RUNTIME_FPS_KEPT as u64 - 1
        ));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn loft_build_fingerprint_is_deterministic() {
        // Memoised + content-hashed → stable within a process (the value depends
        // on whether the rlib is present in this test layout; assert determinism).
        assert_eq!(loft_build_fingerprint(), loft_build_fingerprint());
    }

    #[test]
    fn loft_build_fingerprint_is_never_spuriously_zero() {
        // M1c: a real fingerprint (never `0`) is what keeps the staleness gate
        // *enabled* — `fp == 0` makes `native_artifact_fingerprint_matches` match
        // anything, silently reusing a stale cdylib (the viewer `native function not
        // loaded` stub).  With no rlib beside the exe it falls back to the binary's
        // hash, so any runnable loft (exe always exists) yields a usable fingerprint.
        assert_ne!(
            loft_build_fingerprint(),
            0,
            "fingerprint must fall back to the loft binary, never degrade to 0"
        );
    }

    #[test]
    fn cache_decision_precedence() {
        // The kill switch is the one setting: on without it, off with it — a development
        // build included, which the policy treats like any other.
        assert!(cache_decision(false));
        assert!(!cache_decision(true));
    }

    #[test]
    fn clear_stale_native_target_clears_only_mismatched_artifacts() {
        let root = std::env::temp_dir().join(format!("loft_stale_clear_{}", std::process::id()));
        let release = root.join("release");
        let setup = |sidecar: Option<&str>| {
            let _ = std::fs::remove_dir_all(&root);
            std::fs::create_dir_all(&release).unwrap();
            std::fs::write(release.join("libprobe.so"), b"so").unwrap();
            if let Some(s) = sidecar {
                std::fs::write(release.join(".loft-build-fp"), s).unwrap();
            }
        };
        // 1. Mismatched sidecar → cleared (the laundering hazard: cargo can
        //    no-op over a stale artifact, then the stamp would bless it).
        setup(Some("999"));
        assert!(clear_stale_native_target(
            &root,
            "libprobe.so",
            "libprobe.rlib",
            7
        ));
        assert!(!release.exists(), "stale target must be removed");
        // 2. Matching sidecar → kept.
        setup(Some("7"));
        assert!(!clear_stale_native_target(
            &root,
            "libprobe.so",
            "libprobe.rlib",
            7
        ));
        assert!(release.join("libprobe.so").exists(), "fresh artifact kept");
        // 3. Missing sidecar = unknown provenance, commonly a legitimate
        //    HAND-BUILT artifact (the documented `cargo build` workflow;
        //    the tests/lib fixture cdylibs) → NOT cleared, keeps the
        //    pre-existing build-and-stamp path.
        setup(None);
        assert!(!clear_stale_native_target(
            &root,
            "libprobe.so",
            "libprobe.rlib",
            7
        ));
        assert!(
            release.join("libprobe.so").exists(),
            "hand-built artifact kept"
        );
        // 4. No artifact at all → nothing to clear (first build).
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&release).unwrap();
        assert!(!clear_stale_native_target(
            &root,
            "libprobe.so",
            "libprobe.rlib",
            7
        ));
        // 5. fp == 0 (nothing to fingerprint) → existence-only fallback, never clears.
        setup(Some("999"));
        assert!(!clear_stale_native_target(
            &root,
            "libprobe.so",
            "libprobe.rlib",
            0
        ));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn build_signature_is_deterministic_and_carries_version() {
        let a = build_signature();
        assert_eq!(a, build_signature(), "same build → same signature");
        assert!(a.contains(LOFT_VERSION), "signature pins the crate version");
        // Five '|'-separated fields (format|version|build-id|target|features|binary).
        assert_eq!(a.matches('|').count(), 5, "signature shape: {a}");
    }

    #[test]
    fn prune_dir_evicts_oldest_over_budget() {
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("loft_prune_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Three 10-byte bundles (+ manifests); set distinct mtimes oldest→newest.
        let mk = |name: &str, age_secs: u64| {
            let store = dir.join(format!("{name}.store"));
            std::fs::write(&store, b"0123456789").unwrap();
            std::fs::write(dir.join(format!("{name}.manifest")), b"m").unwrap();
            let when = std::time::SystemTime::now() - Duration::from_secs(age_secs);
            // Open for WRITE before set_modified: Windows `SetFileTime` needs write
            // access (Unix `futimens` works on a read-only fd), matching `touch_now`.
            std::fs::OpenOptions::new()
                .write(true)
                .open(&store)
                .unwrap()
                .set_modified(when)
                .unwrap();
        };
        mk("program-aaa", 300); // oldest
        mk("program-bbb", 200);
        mk("program-ccc", 100); // newest
        // A non-program file must be left alone.
        std::fs::write(dir.join("stdlib-zzz.store"), b"keepme").unwrap();

        // Budget 25 bytes: 3×10 = 30 > 25 → evict the single oldest (→ 20 ≤ 25).
        // TTL of a year so the (recent-mtime) bundles never trip the idle pass —
        // this exercises the size-cap backstop in isolation.
        prune_dir(&dir, 25, Duration::from_hours(24 * 365));
        assert!(
            !dir.join("program-aaa.store").exists(),
            "oldest store evicted"
        );
        assert!(
            !dir.join("program-aaa.manifest").exists(),
            "oldest manifest evicted with it"
        );
        assert!(dir.join("program-bbb.store").exists(), "newer bundle kept");
        assert!(dir.join("program-ccc.store").exists(), "newest bundle kept");
        assert!(
            dir.join("stdlib-zzz.store").exists(),
            "non-program cache file untouched"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_dir_idle_ttl_evicts_unused() {
        use std::time::Duration;
        let dir = std::env::temp_dir().join(format!("loft_idle_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mk = |name: &str, age_secs: u64| {
            let store = dir.join(format!("{name}.store"));
            std::fs::write(&store, b"x").unwrap();
            std::fs::write(dir.join(format!("{name}.manifest")), b"m").unwrap();
            let when = std::time::SystemTime::now() - Duration::from_secs(age_secs);
            // Open for WRITE before set_modified: Windows `SetFileTime` needs write
            // access (Unix `futimens` works on a read-only fd), matching `touch_now`.
            std::fs::OpenOptions::new()
                .write(true)
                .open(&store)
                .unwrap()
                .set_modified(when)
                .unwrap();
        };
        mk("program-fresh", 60); // used a minute ago
        mk("program-stale", 2 * 86400); // idle two days
        // Huge budget so the size-cap never fires → only the idle-TTL pass acts.
        // TTL = 1 day: the 2-day-idle bundle is evicted, the fresh one kept.
        prune_dir(&dir, u64::MAX, Duration::from_hours(24));
        assert!(
            dir.join("program-fresh.store").exists(),
            "a recently-used bundle is kept regardless of age"
        );
        assert!(
            !dir.join("program-stale.store").exists(),
            "a bundle idle longer than the TTL is evicted"
        );
        assert!(
            !dir.join("program-stale.manifest").exists(),
            "the idle bundle's manifest is evicted with it"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_path_is_deterministic_and_key_specific() {
        let k1 = stdlib_cache_key(&[("x.loft".into(), "a".into())]);
        let k2 = stdlib_cache_key(&[("x.loft".into(), "b".into())]);
        let p1 = stdlib_cache_path(&k1);
        assert_eq!(p1, stdlib_cache_path(&k1), "same key → same path");
        assert_ne!(p1, stdlib_cache_path(&k2), "different key → different path");
        let name = p1.file_name().unwrap().to_string_lossy();
        assert!(name.starts_with("stdlib-") && name.ends_with(".store"));
        assert!(
            name.len() == "stdlib-".len() + 64 + ".store".len(),
            "filename embeds 64-hex key"
        );
    }
}
