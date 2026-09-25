// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
#![warn(clippy::pedantic)]
#![allow(
    // Numeric casts: pervasive in the interpreter's hot paths; every
    // stack push/pop goes through an i32/u16/usize conversion and
    // annotating each one kills readability without adding safety.
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_possible_wrap,
    // Style preferences we deliberately keep:
    clippy::match_same_arms,
    clippy::used_underscore_binding,
    clippy::doc_markdown,
    clippy::items_after_statements,
    clippy::implicit_hasher,
    clippy::let_underscore_untyped,
    clippy::must_use_candidate,
    clippy::manual_let_else,
    clippy::too_many_lines,
    clippy::type_complexity,
    // Re-emerges in src/fill.rs every time regen_fill_rs runs; the
    // template format is easier to keep stable than a generator fix.
    clippy::semicolon_if_nothing_returned
)]

// HTML export: when loft's own lib is compiled for
// `wasm32-unknown-unknown` without the full `wasm` feature (the target
// used by `loft --html`), the `print` opcode's `#rust` template calls
// `loft_host_print` — a function the browser host is expected to
// provide via the `loft_io` WASM import module.  Declare it here so
// `src/fill.rs` (auto-generated) can reference it unqualified.
// `not(target_os = "wasi")` keeps this browser-only: wasip2 has working
// std stdout, so its `print` branch uses `print!` (mirrors the @P334 FS
// fix in `src/state/io.rs`); native and the full `wasm` feature each take
// their own branch — so this import is declared only where it is used.
#[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
#[link(wasm_import_module = "loft_io")]
unsafe extern "C" {
    pub(crate) safe fn loft_host_print(ptr: *const u8, len: usize);
    // Input mirror of `loft_host_print`: `host_input()` pulls the bytes the JS
    // host set before `loft_start`.  Two calls because loft must size the buffer
    // before the host fills it — `len` then `copy` (the pattern `web`'s
    // `ws_msg_len`/`ws_msg_copy` bridge already proves).  Used by
    // `Stores::host_input_native`'s browser branch (`src/database/format.rs`).
    pub(crate) safe fn loft_host_input_len() -> usize;
    pub(crate) safe fn loft_host_input_copy(ptr: *mut u8);
    // Output mirror of `host_input` for STRUCTURED host messages (requests to
    // the JS shell — e.g. "fetch this URL"), distinct from user-facing
    // `loft_host_print`.  Used by `Stores::host_output_native`'s browser
    // branch; the page routes it to `globalThis.loftOutput(msg)`.
    pub(crate) safe fn loft_host_output(ptr: *const u8, len: usize);
    // Synchronous-from-loft binary HTTP GET (the browser arm of `net::fetch_bytes`,
    // used by `store_load_url*`).  A browser `fetch()` is async, but this import
    // is in the wasm-opt `--asyncify` allowlist (`asyncify-imports@…loft_io.loft_host_http_get`),
    // so calling it UNWINDS the whole wasm stack back to the JS event loop; the
    // shell's `AsyncifyCtrl` runs `await fetch(url).arrayBuffer()`, then REWINDS
    // with the result — so `store_load_url_trusted(r,url) -> boolean` keeps its
    // synchronous signature and never blocks the page (the same async→sync bridge
    // `loft_web.ws_yield` proves).  Returns the byte length, or `usize::MAX` on a
    // network / non-2xx error.  Then `loft_host_http_get_copy` fills the buffer —
    // the `len`-then-`copy` shape `loft_host_input_len`/`copy` already proves.
    pub(crate) safe fn loft_host_http_get(url_ptr: *const u8, url_len: usize) -> usize;
    pub(crate) safe fn loft_host_http_get_copy(ptr: *mut u8);
    // loft#678 — the RANGE arm of the same bridge, behind the working-set store
    // loaders (`store_load_key(s)` / `store_load_key_text` / `store_load_range`):
    // one `Range: bytes=off-(off+len-1)` GET instead of a whole-file one, so a
    // phone reads the few pages a lookup touches out of a multi-GB hosted block.
    // Suspends exactly like `loft_host_http_get` (it is in the same asyncify
    // allowlist) and returns the byte count fetched, or `usize::MAX` on a network
    // / non-2xx error.  The bytes are then copied out by `loft_host_http_get_copy`
    // — the two share the shell's one response stash, which is safe because a
    // suspend bridges a SYNCHRONOUS loft call: the next request cannot begin until
    // this one has rewound and copied.
    //
    // `off`/`len` cross as `f64` deliberately.  A JS number represents every
    // integer below 2^53 exactly — eight petabytes, so no store offset can lose
    // precision — while `u64` would arrive in JS as a `BigInt` and each of the six
    // headless harnesses that stub these imports would have to return one too; a
    // plain `0` there traps at the call rather than at link, which is the kind of
    // failure that shows up as a wrong answer in one target only.
    pub(crate) safe fn loft_host_http_range(
        url_ptr: *const u8,
        url_len: usize,
        off: f64,
        len: f64,
    ) -> usize;
    // The total size of the resource the LAST `loft_host_http_range` call read,
    // parsed from its `Content-Range: bytes a-b/TOTAL` (or `-1` when the server
    // did not say).  SYNCHRONOUS — it reports what the completed fetch already
    // learned, so it needs no suspend and is not in the asyncify allowlist.  This
    // is how the browser provider answers `PageProvider::size()` without a second
    // round trip: the `bytes=0-0` probe that opens the source carries the total.
    pub(crate) safe fn loft_host_http_range_total() -> f64;
    // #620 — the browser CLOCK bridge.  `--native-wasm` (wasip2) reads a real
    // clock through `std`, but this target has no `std` clock at all, so
    // `now()`/`ticks()` used to return a hardcoded 0: a frame timer read the
    // same instant forever and every duration measured 0ms, silently.
    // `f64` because both JS sources are doubles (`Date.now()` is ms since the
    // epoch, well past i32; `performance.now()` is a fractional ms) — crossing
    // as f64 and narrowing on the loft side keeps the full range.
    // `performance.now()` is monotonic and page-relative, which is exactly
    // `ticks()`'s contract, so no start-time subtraction is needed here.
    pub(crate) safe fn loft_host_time_now_ms() -> f64;
    pub(crate) safe fn loft_host_time_ticks_us() -> f64;
    // @PLN105 Phase 2 — `deliver(tag, value)`: hand the JS host a self-describing handle to a
    // live loft value with no serialization.  `store_base` is the value's Store buffer base in
    // wasm linear memory (`Store.ptr`); `(rec, pos)` is its `DbRef` within that store.  The reader
    // addresses ANY node as `store_base + rec*8 + pos` (`Store::checked_offset`), so it can FOLLOW
    // child records (a vector field holds a child record-index; strings are interned records at
    // `id*8+8`) — which a pre-computed root address alone could not do.  `desc_ptr/desc_len` is
    // the layout descriptor as JSON (`LayoutDesc::to_json`, memoized host-side by `type_id`).
    // SYNCHRONOUS — unlike `loft_host_http_get` it is NOT in the asyncify allowlist; the borrow
    // ends when this returns, so JS must finish reading (or copy out) before then (§5 borrow).
    pub(crate) safe fn loft_host_deliver(
        tag: i64,
        store_base: usize,
        rec: u32,
        pos: u32,
        type_id: u32,
        desc_ptr: *const u8,
        desc_len: usize,
    );
    // @PLN105 Phase 3 — `expose`: a LONG-LIVED `deliver` (same handle), stashed by `tag` and
    // re-read across frames; the value's store is pinned loft-side so the addresses stay stable.
    // `loft_host_release(tag)` tells the host to drop the stash when loft unpins.
    pub(crate) safe fn loft_host_expose(
        tag: i64,
        store_base: usize,
        rec: u32,
        pos: u32,
        type_id: u32,
        desc_ptr: *const u8,
        desc_len: usize,
    );
    pub(crate) safe fn loft_host_release(tag: i64);
    // loft#851 — the page's FILESYSTEM.  `--html` used to bind none at all: every
    // file call took an inert branch and answered "absent", so an editor in a page
    // could draw but could not save.  These are the raw-import twins of the
    // `globalThis.loftHost.fs_*` bridges the wasm-bindgen host already defines
    // (`tests/wasm/host.mjs`), which `--html` cannot reuse — those need
    // wasm-bindgen, and this target refuses a page importing anything beyond
    // `loft_gl` and `loft_io`.  `src/wasm.rs` is the one place that picks between
    // the two transports; everything above it calls `host_fs_*` and never learns
    // which one it got.
    //
    // Reads answer a LENGTH and then fill a buffer, the `len`-then-`copy` shape
    // `loft_host_input_len`/`copy` already proves — a raw wasm import cannot
    // return a string.  `usize::MAX` means absent, which is distinct from a
    // length of 0 (an empty file that really exists).  All six readers share ONE
    // host-side stash drained by `loft_host_fs_copy`, and that is safe for the
    // same reason the HTTP bridge's single stash is: each of these is a
    // SYNCHRONOUS loft call, so the next read cannot begin before this one has
    // copied.  Unlike the HTTP bridge none of them suspends, so they are not in
    // the asyncify allowlist.
    pub(crate) safe fn loft_host_fs_read_text(path_ptr: *const u8, path_len: usize) -> usize;
    pub(crate) safe fn loft_host_fs_read_binary(path_ptr: *const u8, path_len: usize) -> usize;
    pub(crate) safe fn loft_host_fs_read_bytes(
        path_ptr: *const u8,
        path_len: usize,
        want: usize,
    ) -> usize;
    // Directory names, `\n`-joined.  A newline cannot occur in a name the host
    // stores, and joining keeps this to one import instead of a count-then-
    // index-then-copy triple.
    pub(crate) safe fn loft_host_fs_list_dir(path_ptr: *const u8, path_len: usize) -> usize;
    pub(crate) safe fn loft_host_fs_cwd() -> usize;
    pub(crate) safe fn loft_host_fs_user_dir() -> usize;
    pub(crate) safe fn loft_host_fs_program_dir() -> usize;
    pub(crate) safe fn loft_host_fs_copy(ptr: *mut u8);
    // Writes and metadata answer directly.  An `i32` result is 0 on success and
    // an errno-shaped code otherwise, matching what `fs_classify` returns on
    // native so a `FileResult` reads the same on every target.
    pub(crate) safe fn loft_host_fs_write_text(
        path_ptr: *const u8,
        path_len: usize,
        data_ptr: *const u8,
        data_len: usize,
    ) -> i32;
    pub(crate) safe fn loft_host_fs_write_binary(
        path_ptr: *const u8,
        path_len: usize,
        data_ptr: *const u8,
        data_len: usize,
    ) -> i32;
    pub(crate) safe fn loft_host_fs_write_bytes(
        path_ptr: *const u8,
        path_len: usize,
        data_ptr: *const u8,
        data_len: usize,
    ) -> i32;
    pub(crate) safe fn loft_host_fs_delete(path_ptr: *const u8, path_len: usize) -> i32;
    pub(crate) safe fn loft_host_fs_move(
        from_ptr: *const u8,
        from_len: usize,
        to_ptr: *const u8,
        to_len: usize,
    ) -> i32;
    pub(crate) safe fn loft_host_fs_mkdir(path_ptr: *const u8, path_len: usize) -> i32;
    pub(crate) safe fn loft_host_fs_mkdir_all(path_ptr: *const u8, path_len: usize) -> i32;
    pub(crate) safe fn loft_host_fs_exists(path_ptr: *const u8, path_len: usize) -> i32;
    pub(crate) safe fn loft_host_fs_is_dir(path_ptr: *const u8, path_len: usize) -> i32;
    pub(crate) safe fn loft_host_fs_is_file(path_ptr: *const u8, path_len: usize) -> i32;
    // No `truncate` import: resizing is `read_binary` → slice → `write_binary`,
    // which the wasm-bindgen path already composes the same way.  Every import
    // here is one more function a page (and each headless stub) must define, so
    // a resize that costs one extra round trip is worth not having a 22nd name.
    //
    // Sizes and cursor positions cross as `f64` for the same reason the HTTP
    // range bridge's offsets do: a JS number is exact to 2^53 (eight petabytes),
    // while a `u64`/`i64` would arrive as a `BigInt` every headless stub would
    // then have to return too — and a plain `0` there traps at the call rather
    // than at link, which is the kind of failure that shows up in one target only.
    // `-1` from `file_size` means absent.
    pub(crate) safe fn loft_host_fs_file_size(path_ptr: *const u8, path_len: usize) -> f64;
    pub(crate) safe fn loft_host_fs_seek(path_ptr: *const u8, path_len: usize, pos: f64);
    pub(crate) safe fn loft_host_fs_get_cursor(path_ptr: *const u8, path_len: usize) -> f64;
}

/// `eprintln!` that a browser page can actually be read from (loft#950).
///
/// On `wasm32-unknown-unknown` std's stderr is the `unsupported` backend: a write succeeds
/// and goes nowhere, and std owns it, so there is nothing to intercept.  A `--html` page
/// therefore ran every diagnostic in this crate into a sink — the switch was armed, the
/// work was done, and the finding was discarded.  That is worse than an unarmed switch,
/// because it reads as the instrument having nothing to say.
///
/// The page already binds `loft_io.loft_host_print`; a panic has reached the console
/// through it since loft#950's first instrument.  This routes the ordinary diagnostics the
/// same way, so `LOFT_STRICT_STORES=1` in a page prints what it prints on a desktop.
///
/// Hosted targets — native, `wasm32-wasip2`, and the `wasm` feature's own bridge — keep
/// `eprintln!` exactly, so nothing about their output moves.
/// Read an environment switch ONCE per process.  `env_once!(expr)` evaluates `expr` (a
/// `bool` over `std::env::var*`) on first use and answers the cached value after — the same
/// `OnceLock` every hand-written `*_enabled()` helper carries, as one spelling.
///
/// The front end read its `LOFT_*` switches on every token and every node: over a compile
/// of the 12 826-line front-end corpus `getenv` ran 178 732 times, each a `strncmp` walk over
/// the whole environment, for 3 % of all instructions (callgrind, @PLN166 B4).  A switch is
/// a fact of the invocation, so a process-wide read is the same answer every time; the one
/// thing this must not wrap is a variable the process itself SETS mid-run (`set_var`), and
/// none of the front end's switches is.  `env_once!(@value T, expr)` caches a non-`bool`.
#[macro_export]
macro_rules! env_once {
    (@value $t:ty, $e:expr) => {{
        static ONCE: ::std::sync::OnceLock<$t> = ::std::sync::OnceLock::new();
        ONCE.get_or_init(|| $e)
    }};
    ($e:expr) => {{
        static ONCE: ::std::sync::OnceLock<bool> = ::std::sync::OnceLock::new();
        *ONCE.get_or_init(|| $e)
    }};
}

#[macro_export]
macro_rules! loft_eprintln {
    ($($arg:tt)*) => {{
        #[cfg(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm")))]
        {
            $crate::live_dispatch::wasm_host_log(&format!("{}\n", format_args!($($arg)*)));
        }
        // Through `host_eprint` rather than `eprintln!`: a diagnostic whose write cannot
        // land must not panic, because a panic whose own message cannot be printed is what
        // turns `prog 2>&1 | head` into a SIGABRT and a crash report (loft#1289).
        #[cfg(not(all(target_arch = "wasm32", not(target_os = "wasi"), not(feature = "wasm"))))]
        {
            $crate::codegen_runtime::host_eprint(&format!("{}\n", format_args!($($arg)*)));
        }
    }};
}

#[macro_use]
pub mod diagnostics;
pub mod api_diff;
pub mod api_surface;
pub mod arena;
pub mod base64;
pub mod build_phase;
pub mod c_call;
pub mod c_shim;
pub mod c_signature;
pub mod cache;
pub mod cache_gc;
mod calc;
pub mod copy_manifest;
pub mod crash_report;
pub mod data;
pub mod data_store;
pub mod database;
pub mod debugger;
pub mod diagnostic_render;
pub mod ffi_deliver;
pub mod fix_apply;
pub mod fxhash;
pub mod hash;
pub mod ir_node;
pub mod ir_read;
pub mod ir_schema;
pub mod ir_schema_gen;
pub mod ir_store;
pub mod json;
pub mod keys;
pub mod lease;
mod lexer;
pub mod lsp;
pub mod narrow;
pub mod native;
pub mod net_profile;
pub mod null_census;
pub mod spent;
pub mod stack_census;
pub mod stack_verify;
// `net::fetch_bytes` (behind `store_load_url*`) exists exactly where `load_url`
// does: a native `registry` build, or the browser (`--html`) target.
pub mod host;
/// @PLN146 F4 — `[[embed]]` declarations become the page's own filesystem, so a
/// browser page carries the pack it reads.
pub mod html_embed;
/// @PLN146 F5/F6 — `[[font]]` declarations become the `<head>` a browser page needs
/// and the await that holds `loft_start` until the fonts have arrived.
pub mod html_fonts;
// `registry` and `remote-store` each pull `ureq` (the native client); the browser
// arm needs no dependency at all.  `remote-store` is listed because the paged
// loaders' range GETs live here too (loft#678) and that feature can be selected
// without `registry`.
/// @PLN119 — where a library RUNS (in-process or a worker process).  Distinct
/// from [`placement`], which is where a keyed collection puts an entry on disk.
pub mod lib_placement;
// Compiled on EVERY target.  A build with no HTTP transport gets the module's
// "unavailable" arm, which its header has always promised — that answer used to be
// spelled by not compiling the module at all, and the difference is not academic:
// `paged_reader` names `crate::net` unconditionally, so a build that has the paged
// loaders but no transport could not compile rather than simply refusing a URL.
pub mod byte_copy;
pub mod compact;
pub mod const_fn;
pub(crate) mod net;
pub mod ownership_cfg;
#[cfg(paged_store)]
pub mod paged_reader;
pub mod place_result;
pub mod placement;
pub mod portable_path;
pub mod rebind_place;
pub mod resolution;
pub mod resolution_scope;
pub mod scopes;
pub mod siphash;
pub mod use_analysis;
mod variables;
pub mod vector;

pub mod trace;

pub mod codegen_runtime;
pub mod generation;
pub mod ops;
#[cfg(test)]
mod page_metrics;
pub mod parser;
#[cfg(feature = "png")]
mod png_store;
pub mod profiler;
mod radix_db;
mod radix_tree;
mod spatial;
pub mod store;
pub mod store_budget;
pub mod tree;
pub mod trie_db;
mod typedef;

pub mod const_eval;
pub mod coroutine_layout;
pub mod create;
pub mod fill;
pub mod parallel;
pub mod platform;
pub mod state;

pub mod compile;
pub mod engine_host;
pub mod extensions;
pub mod git_query;
pub mod live_dispatch;
#[cfg(not(target_arch = "wasm32"))]
pub mod live_reload;
pub mod repl;
pub mod rpc;
pub mod script;
pub mod serve;
pub mod startup_cache;
pub mod wasm_debug;
// @PLAN12 phase 3.5a (2026-05-24) — re-export `extensions::native_call`
// at the crate root so generated native code can write
// `use loft::native_call;` without coupling to the extensions module.
// Present in every build, including one without `native-extensions`: a
// `wasm32-wasip2` binary links its `[native] crate` statically and still needs
// the store handle (loft#967).
pub use extensions::native_call;
// @PLN53 F1/F2 — raw-source fuzz oracle + keyed-container generator; available
// under cargo-fuzz (the `fuzzing` feature) and under `cargo test`.
#[cfg(any(test, feature = "fuzzing"))]
pub mod fuzz_keyed;
#[cfg(any(test, feature = "fuzzing"))]
pub mod fuzz_oracle;
#[cfg(feature = "registry")]
pub mod install;
pub mod integrity;
pub mod introspect;
pub mod libscan;
pub mod lockfile;
pub mod log_config;
pub mod logger;
pub mod manifest;
pub mod native_gate;
pub mod native_lib;
#[cfg(feature = "registry")]
pub mod package;
pub mod package_layout;
pub mod registry;
#[cfg(feature = "registry")]
pub mod registry_advisories;
#[cfg(feature = "registry")]
pub mod registry_index;
pub mod registry_keys;
#[cfg(feature = "registry")]
pub mod registry_signing;
pub mod runtime_error;
/// @PLN86 — sandbox policy model (capability-group allow-lists + the loft.toml parser).
pub mod sandbox;
/// @PLN97 Phase D — the schema-description sidecar (a store's self-describing layout identity).
pub mod schema_sidecar;
#[cfg(feature = "registry")]
pub mod self_update;
mod stack;
pub mod timeout;
pub mod triggers;
pub mod verify_self;

pub mod documentation;
pub mod migrate_long;
pub mod stdlib_sources;

// `host_fs`, not `feature = "wasm"`: `--html` reaches the same host bridges
// over raw `loft_io` imports, and this module is where the two transports are
// told apart (loft#851).  Its wasm-bindgen half stays behind the feature.
#[cfg(host_fs)]
pub mod wasm;
pub mod wasm_assets;
pub mod wasm_gl;
// @PLN117 — the browser thread pool.  Browser-only by nature (it exists because
// wasm has no `thread::spawn`), so it is not compiled for the host: type-checking
// it is the wasm build's job, via `make check-wasm-threads`.
#[cfg(all(feature = "wasm-native-threads", target_arch = "wasm32"))]
pub mod wasm_threads;
