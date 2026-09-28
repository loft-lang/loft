// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! Plan-12 extraction-hygiene gate — and @C78's checkable half: the engine ships mechanism,
//! and a library's code (its world model, its natives) never moves into it.
//!
//! The loft compiler crate (everything under `src/`) must contain
//! **zero** library code — only language core, runtime, codegen, and
//! stdlib symbols.  Libraries (`lib/crypto`, `lib/imaging`, `lib/web`,
//! `lib/random`, …) live in their own per-package native crates
//! (`lib/<X>/native/src/lib.rs`) and are loaded at runtime via the
//! package format.  See
//! [`doc/claude/lib_plans/12-library-extraction/README.md`](../doc/claude/lib_plans/12-library-extraction/README.md)
//! for the stdlib-vs-library boundary table.
//!
//! Two gates here:
//!
//! 1. **Symbol drain** — `forbidden_library_symbols_absent_from_src`:
//!    walks `src/**/*.rs`, fails if any known library `n_*` symbol
//!    name appears.  The list MUST stay in sync with what each
//!    library's `native/` crate exports.
//!
//! 2. **Dep drain** — `forbidden_library_deps_absent_from_main_cargo`:
//!    reads the main crate's `Cargo.toml`, fails if it lists
//!    library-only dependencies (`png`, `ureq`, `rustls`, `fontdue`)
//!    once those libraries' native crates are the sole owners.  This
//!    gate is **scoped**: currently it only enforces deps that have
//!    actually been drained.  Add a row when a library's drain
//!    completes; never add a row for an undrained library (the test
//!    would be wrong, not the code).
//!
//! These gates are CHEAP — pure file reads, no compilation.  Run on
//! every CI cycle.  When a future @P321-style attempt tries to add
//! `n_load_png` to `src/codegen_runtime.rs`, this test fails before
//! the PR can merge.

use std::fs;
use std::path::{Path, PathBuf};

/// Manual fallback list for library `n_*` symbols that need to be
/// forbidden in `src/**/*.rs` but DON'T yet have a `[native.functions]`
/// entry in their library's `loft.toml`.
///
/// @PLAN12 phase 2 (2026-05-24): the primary source of truth moved from
/// this hand-maintained const to `lib/<X>/loft.toml::[native.functions]`.
/// `forbidden_library_symbols()` below walks every `lib/*/loft.toml`,
/// reads its `[native.functions]` table, and appends each `n_*` value to
/// the effective forbidden list.  This const stays empty by default and
/// is reserved for symbols whose owning library's loft.toml can't yet
/// declare them (e.g. a Tier-B library being prepared for drain whose
/// metadata isn't ready).  When you need to add a manual row, prefer
/// populating the library's loft.toml instead.
///
/// Stdlib symbols (`n_panic`, `n_assert`, `n_log_*`, `n_json_*`,
/// `n_parallel_*`, `n_now`, `n_ticks`, …) are deliberately NOT forbidden
/// — they stay in the compiler crate by design.  See the
/// stdlib-vs-library table in the plan README.
const FORBIDDEN_LIBRARY_SYMBOLS_MANUAL: &[(&str, &str)] = &[
    // @PLAN12 phase 3.5 (2026-05-24) — extracted to the
    // `loft-lang/loft-libs-core` chunk repo.  Their
    // `loft.toml::[native.functions]` (or `#native` annotations in
    // source) are no longer scanned by `forbidden_library_symbols()`
    // (which walks `lib/*` only), so symbols are pinned here to keep
    // the hygiene gate detecting re-introduction in `src/**`.
    //
    // crypto (5): n_hmac_sha256_raw was removed 2026-05-30 alongside
    // loft-libs-core's `jwt_sign` cleanup (its sole consumer).
    ("n_sha256", "loft-libs-core/crypto/native"),
    ("n_hmac_sha256", "loft-libs-core/crypto/native"),
    ("n_base64_encode", "loft-libs-core/crypto/native"),
    ("n_base64_decode", "loft-libs-core/crypto/native"),
    ("n_base64url_encode", "loft-libs-core/crypto/native"),
    // random (3) — drained in @PLAN12 phase 3.5a (2026-05-24).
    // The drain became possible after the `loft::native_call`
    // helpers shipped (LoftStore-forwarding codegen for
    // store-allocating cdylib returns).  Random is now the
    // canonical example pattern for libraries with
    // `n_*_indices`-shaped functions.
    ("n_rand", "loft-libs-core/random/native"),
    ("n_rand_seed", "loft-libs-core/random/native"),
    ("n_rand_indices", "loft-libs-core/random/native"),
    // web (19) — drained in @PLAN12 Phase 6b (2026-05-31).
    // Stage B removed `lib/web/` from the monorepo; web is now
    // resolved exclusively through the loft package registry
    // (`loft install web` → `~/.loft/registry/web-<ver>/`).
    // Symbols pinned here because `forbidden_library_symbols()`'s
    // dynamic scan walks `lib/*` only.
    ("n_http_do", "loft-libs-net/web/native"),
    ("n_http_body", "loft-libs-net/web/native"),
    ("n_ws_connect", "loft-libs-net/web/native"),
    ("n_ws_client_send", "loft-libs-net/web/native"),
    ("n_ws_client_send_binary", "loft-libs-net/web/native"),
    ("n_ws_client_recv", "loft-libs-net/web/native"),
    ("n_ws_client_message", "loft-libs-net/web/native"),
    ("n_ws_client_opcode", "loft-libs-net/web/native"),
    ("n_ws_client_close", "loft-libs-net/web/native"),
    ("n_sleep_ms", "loft-libs-net/web/native"),
    ("n_pack_reset", "loft-libs-net/web/native"),
    ("n_pack_u8", "loft-libs-net/web/native"),
    ("n_pack_u16_le", "loft-libs-net/web/native"),
    ("n_pack_u32_le", "loft-libs-net/web/native"),
    ("n_pack_take", "loft-libs-net/web/native"),
    ("n_byte_at", "loft-libs-net/web/native"),
    ("n_ws_group_clear", "loft-libs-net/web/native"),
    ("n_ws_group_add", "loft-libs-net/web/native"),
    ("n_ws_group_poll", "loft-libs-net/web/native"),
    // graphics (56) — drained in @PLAN12 Phase 5b (2026-05-31).
    // Stage B removed `lib/graphics/` from the monorepo; graphics is now
    // resolved exclusively through the loft package registry
    // (`loft install graphics` → `~/.loft/registry/graphics-<ver>/`).
    // OpenGL bindings + canvas-image PNG save + sprite/text/audio.
    ("n_gl_create_window", "loft-libs-graphics/graphics/native"),
    (
        "n_gl_create_fullscreen_window",
        "loft-libs-graphics/graphics/native",
    ),
    ("n_gl_set_fullscreen", "loft-libs-graphics/graphics/native"),
    ("n_gl_window_width", "loft-libs-graphics/graphics/native"),
    ("n_gl_window_height", "loft-libs-graphics/graphics/native"),
    ("n_gl_mouse_wheel", "loft-libs-graphics/graphics/native"),
    ("n_gl_poll_events", "loft-libs-graphics/graphics/native"),
    ("n_gl_swap_buffers", "loft-libs-graphics/graphics/native"),
    ("n_gl_clear", "loft-libs-graphics/graphics/native"),
    ("n_gl_destroy_window", "loft-libs-graphics/graphics/native"),
    ("n_gl_create_shader", "loft-libs-graphics/graphics/native"),
    ("n_gl_use_shader", "loft-libs-graphics/graphics/native"),
    ("n_gl_draw", "loft-libs-graphics/graphics/native"),
    ("n_gl_bind_texture", "loft-libs-graphics/graphics/native"),
    ("n_gl_delete_texture", "loft-libs-graphics/graphics/native"),
    ("n_gl_load_font", "loft-libs-graphics/graphics/native"),
    ("n_text_height", "loft-libs-graphics/graphics/native"),
    ("n_gl_font_ascent", "loft-libs-graphics/graphics/native"),
    (
        "n_rasterize_text_into",
        "loft-libs-graphics/graphics/native",
    ),
    ("n_gl_measure_text", "loft-libs-graphics/graphics/native"),
    ("n_gl_upload_vertices", "loft-libs-graphics/graphics/native"),
    ("n_gl_set_mat4", "loft-libs-graphics/graphics/native"),
    (
        "n_gl_set_uniform_float",
        "loft-libs-graphics/graphics/native",
    ),
    ("n_gl_set_uniform_int", "loft-libs-graphics/graphics/native"),
    (
        "n_gl_set_uniform_vec3",
        "loft-libs-graphics/graphics/native",
    ),
    ("n_gl_enable", "loft-libs-graphics/graphics/native"),
    ("n_gl_disable", "loft-libs-graphics/graphics/native"),
    ("n_gl_blend_func", "loft-libs-graphics/graphics/native"),
    ("n_gl_cull_face", "loft-libs-graphics/graphics/native"),
    ("n_gl_depth_mask", "loft-libs-graphics/graphics/native"),
    ("n_gl_viewport", "loft-libs-graphics/graphics/native"),
    (
        "n_gl_create_framebuffer",
        "loft-libs-graphics/graphics/native",
    ),
    (
        "n_gl_bind_framebuffer",
        "loft-libs-graphics/graphics/native",
    ),
    (
        "n_gl_framebuffer_texture",
        "loft-libs-graphics/graphics/native",
    ),
    (
        "n_gl_create_depth_texture",
        "loft-libs-graphics/graphics/native",
    ),
    (
        "n_gl_create_color_texture",
        "loft-libs-graphics/graphics/native",
    ),
    (
        "n_gl_draw_fullscreen_quad",
        "loft-libs-graphics/graphics/native",
    ),
    ("n_gl_key_pressed", "loft-libs-graphics/graphics/native"),
    ("n_gl_mouse_x", "loft-libs-graphics/graphics/native"),
    ("n_gl_mouse_y", "loft-libs-graphics/graphics/native"),
    ("n_gl_mouse_button", "loft-libs-graphics/graphics/native"),
    ("n_gl_draw_elements", "loft-libs-graphics/graphics/native"),
    ("n_gl_draw_mode", "loft-libs-graphics/graphics/native"),
    ("n_gl_delete_shader", "loft-libs-graphics/graphics/native"),
    ("n_gl_delete_vao", "loft-libs-graphics/graphics/native"),
    (
        "n_gl_delete_framebuffer",
        "loft-libs-graphics/graphics/native",
    ),
    ("n_gl_line_width", "loft-libs-graphics/graphics/native"),
    ("n_gl_point_size", "loft-libs-graphics/graphics/native"),
    ("n_gl_load_texture", "loft-libs-graphics/graphics/native"),
    ("n_gl_upload_canvas", "loft-libs-graphics/graphics/native"),
    ("n_save_png", "loft-libs-graphics/graphics/native"),
    ("n_gl_screenshot", "loft-libs-graphics/graphics/native"),
    ("n_audio_load", "loft-libs-graphics/graphics/native"),
    ("n_audio_play", "loft-libs-graphics/graphics/native"),
    ("n_audio_stop", "loft-libs-graphics/graphics/native"),
    ("n_audio_set_volume", "loft-libs-graphics/graphics/native"),
    ("n_audio_play_raw", "loft-libs-graphics/graphics/native"),
    // imaging (2) — drained in @PLAN12 Phase 5b (2026-05-31).
    // Note `n_save_png` exists in both graphics + imaging cdylibs
    // (graphics' save_png writes a Canvas; imaging's writes an Image)
    // — different ABIs scoped to their respective cdylib.
    ("n_load_png", "loft-libs-graphics/imaging/native"),
    // n_save_png already listed above under graphics.
];

/// Read every `lib/*/loft.toml` and walk its `[native.functions]` table
/// (if present), appending each `(value, "lib/<name>/native")` pair to
/// the result.  Augmented by the manual fallback list above for any
/// symbol that doesn't yet live in a manifest.
///
/// Phase 2: this is the metadata-driven path.  Adding a new
/// `[native.functions]` row in a library's `loft.toml` automatically
/// extends the hygiene gate's coverage.
fn forbidden_library_symbols() -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = FORBIDDEN_LIBRARY_SYMBOLS_MANUAL
        .iter()
        .map(|(s, o)| ((*s).to_string(), (*o).to_string()))
        .collect();
    let lib_dir = workspace_root().join("lib");
    let Ok(entries) = fs::read_dir(&lib_dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Skip `.loft_test_tmp_*` artifact-isolation dirs (run_lib_test_in_temp_cwd)
        // — transient symlinked package mirrors, not real packages.
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let manifest_path = path.join("loft.toml");
        let Ok(content) = fs::read_to_string(&manifest_path) else {
            continue;
        };
        // @PLN18 — `[native] in_binary = true`: the library's natives are
        // DELIBERATELY inside the compiler crate (registered in
        // `src/native.rs`); its symbols are sanctioned in `src/**`, so the
        // package is excluded from the forbidden scan.  The same mark makes
        // the auto-native driver skip the doomed cdylib compile (main.rs).
        if content
            .lines()
            .any(|l| l.trim().starts_with("in_binary") && l.contains("true"))
        {
            continue;
        }
        let pkg_name = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let owner = format!("lib/{pkg_name}/native");
        let mut in_section = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with('[') && trimmed.ends_with(']') {
                in_section = trimmed == "[native.functions]";
                continue;
            }
            if !in_section {
                continue;
            }
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some((_, value)) = trimmed.split_once('=') {
                let sym = value.trim().trim_matches('"').to_string();
                if !sym.is_empty() && !out.iter().any(|(s, _)| *s == sym) {
                    out.push((sym, owner.clone()));
                }
            }
        }
        // Also derive symbols from the co-located `#native "n_*"`
        // annotations in the package's `.loft` source.  This is the
        // single, co-located source of truth: a library can declare its
        // native bindings purely via annotations (no separate
        // `[native.functions]` manifest table) and still be guarded here.
        let mut loft_files = Vec::new();
        collect_loft_files(&path, &mut loft_files);
        for lf in &loft_files {
            let Ok(src) = fs::read_to_string(lf) else {
                continue;
            };
            // Track the most recent `fn <name>` so a BARE `#native`
            // (symbol defaulted to `n_<name>`) is guarded as well as an
            // explicit `#native "n_custom"` override.
            let mut last_fn: Option<String> = None;
            for line in src.lines() {
                let t = line.trim();
                let decl = t.strip_prefix("pub ").unwrap_or(t);
                if let Some(after) = decl.strip_prefix("fn ") {
                    let name: String = after
                        .chars()
                        .take_while(|c| c.is_alphanumeric() || *c == '_')
                        .collect();
                    if !name.is_empty() {
                        last_fn = Some(name);
                    }
                } else if let Some(rest) = t.strip_prefix("#native") {
                    let rest = rest.trim();
                    let sym = if rest.is_empty() {
                        last_fn.as_ref().map(|n| format!("n_{n}"))
                    } else {
                        Some(rest.trim_matches('"').to_string())
                    };
                    if let Some(sym) = sym
                        && sym.starts_with("n_")
                        && !out.iter().any(|(s, _)| *s == sym)
                    {
                        out.push((sym, owner.clone()));
                    }
                }
            }
        }
    }
    out
}

/// Cargo dependencies that belong to a library's `native/` crate, not
/// the main loft crate.  Currently empty — the `png` / `ureq` / etc.
/// deps still live in `Cargo.toml` because Tier-B libraries
/// (graphics/imaging/random/server) are still linked as workspace
/// members and the cdylib loader hasn't shipped yet.  Add a row when
/// a dep moves out for real.
const FORBIDDEN_MAIN_CRATE_DEPS: &[&str] = &[
    // Add rows when a library's drain removes its dep from the main
    // crate.  Until then this gate is intentionally lenient — the
    // symbol gate above catches the more important regression
    // (compiler-crate Rust code referencing library symbols).
];

fn collect_rs_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

/// Recursively collect every `*.loft` file under `dir` (skipping any
/// `native/` subtree — those hold Rust, not loft source).
fn collect_loft_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "native") {
                continue;
            }
            collect_loft_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "loft") {
            out.push(path);
        }
    }
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Precompute, for one file's lines, the set of line indices that live
/// inside (or directly above) a `#[cfg(...wasm32...)]` gate.  A line is
/// gated when:
///
/// 1. It IS a `#[cfg(...wasm32...)]` attribute.
/// 2. The previous non-empty non-comment line is one (covers per-row
///    attribute on a const-slice entry or `fn …` definition).
/// 3. It lives inside a top-level brace block (`fn`, `const ... = &[`,
///    `mod`, `impl`) whose opening line was preceded by a wasm32 cfg
///    attribute.
///
/// Heuristic — not a full Rust parser — but matches every wasm32-only
/// shape currently in the loft codebase (per-fn cfg on WASM bridge
/// helpers; per-array cfg on `WEB_FUNCTIONS_WASM`).  False positives
/// (skipping non-wasm lines) only hide forbidden symbols; false
/// negatives just surface them sooner.
fn wasm32_cfg_gated_lines(lines: &[&str]) -> Vec<bool> {
    let mut gated = vec![false; lines.len()];
    // Walk forward, tracking whether the most recent non-empty
    // non-comment line was a `#[cfg(...wasm32...)]` attribute.  When
    // we see a brace-opening line, decide whether the resulting block
    // is gated by checking the pending attribute.  Maintain a stack
    // of (depth_at_open, gated_flag) so nested blocks inherit.
    let mut stack: Vec<(i32, bool)> = vec![(0, false)];
    let mut depth: i32 = 0;
    let mut pending_wasm_attr = false;
    for (i, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim();
        let is_attr_line = is_wasm32_cfg_attr(trimmed);
        // Gate the line itself if it IS the attribute or the
        // enclosing block is gated.
        if is_attr_line {
            gated[i] = true;
            pending_wasm_attr = true;
            continue;
        }
        let inside_gated_block = stack.last().is_some_and(|&(_, g)| g);
        // Determine effective gating BEFORE we update brace depth so
        // open-brace lines are themselves marked gated when the
        // pending attribute applied to them.
        let gated_now = inside_gated_block || (pending_wasm_attr && !trimmed.is_empty());
        if gated_now {
            gated[i] = true;
        }
        // Strip comments / strings minimally before counting to avoid
        // mis-counting delimiters inside `//` or `"…"`.  Cheap
        // approximation: drop `//` and everything after.  Track `{`,
        // `[`, AND `(` together — a const-slice declaration opens with
        // `&[` not `{`, and we still want to mark its elements as gated.
        let code = trimmed.split_once("//").map_or(trimmed, |(c, _)| c);
        let opens = (code.matches('{').count()
            + code.matches('[').count()
            + code.matches('(').count()) as i32;
        let closes = (code.matches('}').count()
            + code.matches(']').count()
            + code.matches(')').count()) as i32;
        let net = opens - closes;
        if opens > 0 {
            // Opening at least one block on this line — push gating
            // state derived from pending_wasm_attr OR inherited.
            let block_gated = inside_gated_block || pending_wasm_attr;
            for _ in 0..opens {
                stack.push((depth, block_gated));
                depth += 1;
            }
        }
        if closes > 0 {
            for _ in 0..closes {
                depth = depth.saturating_sub(1);
                if stack.len() > 1 {
                    stack.pop();
                }
            }
        }
        // `pending_wasm_attr` clears once we've seen any non-empty
        // non-comment, non-cfg-attr line OR after the opening brace
        // of the gated block has been pushed.
        if !trimmed.is_empty() && !trimmed.starts_with("#[") {
            pending_wasm_attr = false;
        }
        // Also clear if pending was consumed by opening a block.
        if net > 0 {
            pending_wasm_attr = false;
        }
    }
    gated
}

fn is_wasm32_cfg_attr(line: &str) -> bool {
    let trimmed = line.trim_start();
    if !(trimmed.starts_with("#[cfg(") || trimmed.starts_with("#![cfg(")) {
        return false;
    }
    trimmed.contains("wasm32") || trimmed.contains("target_arch = \"wasm32\"")
}

#[test]
fn forbidden_library_symbols_absent_from_src() {
    let root = workspace_root();
    let src_dir = root.join("src");
    let mut files = Vec::new();
    collect_rs_files(&src_dir, &mut files);
    assert!(!files.is_empty(), "no .rs files found under src/");

    // Phase 2 (@PLAN12, 2026-05-24): build the forbidden list from
    // `lib/*/loft.toml::[native.functions]` plus the manual fallback
    // const.  Loft.toml is now the source of truth.
    let forbidden = forbidden_library_symbols();
    assert!(
        !forbidden.is_empty(),
        "no forbidden library symbols loaded — \
         `lib/*/loft.toml::[native.functions]` should contain at least \
         the crypto + web entries from phases 1a / 1b.  \
         If the libraries' manifests changed, re-populate them."
    );

    let mut violations: Vec<String> = Vec::new();
    for path in &files {
        let content = match fs::read_to_string(path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let rel = path
            .strip_prefix(&root)
            .map(|p| p.display().to_string())
            .unwrap_or_else(|_| path.display().to_string());
        // Walk by line so we can skip `//`-comment occurrences (a
        // valid description of the drain location can mention the
        // symbol name — see the trailing comment in `src/native.rs`'s
        // NATIVE_TABLE).  Only CODE occurrences are violations.
        let lines: Vec<&str> = content.lines().collect();
        let gated = wasm32_cfg_gated_lines(&lines);
        for (line_idx, line) in lines.iter().enumerate() {
            // Strip after `//`.  Doesn't handle `/* ... */` block
            // comments correctly, but those aren't used in this
            // codebase for symbol-name references.
            let code_part = line.split_once("//").map_or(*line, |(c, _)| c);
            // Phase 1b (@PLAN12, 2026-05-24): skip lines living inside
            // a `#[cfg(...wasm32...)]` gated block.  Those library
            // `n_*` references stay in the compiler crate because WASM
            // has no dlopen and must register symbols statically; the
            // regular native path uses the cdylib in `lib/<X>/native/`.
            if gated[line_idx] {
                continue;
            }
            for (sym, owner) in &forbidden {
                for (idx, _) in code_part.match_indices(sym.as_str()) {
                    let prev_ok = idx == 0
                        || !code_part.as_bytes()[idx - 1].is_ascii_alphanumeric()
                            && code_part.as_bytes()[idx - 1] != b'_';
                    let end = idx + sym.len();
                    let next_ok = end >= code_part.len()
                        || (!code_part.as_bytes()[end].is_ascii_alphanumeric()
                            && code_part.as_bytes()[end] != b'_');
                    if !prev_ok || !next_ok {
                        continue;
                    }
                    violations.push(format!(
                        "{rel}:{}: `{sym}` (belongs in {owner})",
                        line_idx + 1
                    ));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "\n@PLAN12 extraction-hygiene gate failed — \
         library `n_*` symbol(s) found in the compiler crate (src/):\n  {}\n\n\
         These symbols belong in their library's `native/src/lib.rs` (cdylib).  \
         If you intend to add a NEW library symbol, put it in the library's \
         native crate and declare it in `lib/<X>/loft.toml::[native.functions]` \
         (phase 2 source of truth).  If a symbol is genuinely stdlib (every \
         loft program needs it), do NOT add it to `[native.functions]` — \
         stdlib stays in the compiler crate by design.  See the \
         stdlib-vs-library table in `doc/claude/lib_plans/\
         12-library-extraction/README.md`.\n",
        violations.join("\n  ")
    );
}

#[test]
fn forbidden_library_deps_absent_from_main_cargo() {
    let root = workspace_root();
    let cargo = fs::read_to_string(root.join("Cargo.toml")).expect("read Cargo.toml");
    // Match a top-level dep line like `png = "0.17"` or `png = {..}` —
    // anchored at start-of-line so feature-gated lines `dep:png` are
    // ignored (the `[features]` section is its own thing).
    let mut violations: Vec<String> = Vec::new();
    for dep in FORBIDDEN_MAIN_CRATE_DEPS {
        let needle_eq = format!("\n{dep} ");
        let needle_eq2 = format!("\n{dep}=");
        if cargo.contains(&needle_eq) || cargo.contains(&needle_eq2) {
            violations.push((*dep).to_string());
        }
    }
    assert!(
        violations.is_empty(),
        "@PLAN12 extraction-hygiene gate failed — main `Cargo.toml` lists \
         library-only dependencies that should live in `lib/<X>/native/Cargo.toml`: {:?}",
        violations
    );
}

/// Phase 2 audit: assert the manifest-driven path actually loads the
/// known-drained symbols.  Catches regressions where a `loft.toml` is
/// accidentally emptied or the `[native.functions]` section header is
/// renamed/typo'd (the symbol-absent test would silently pass with an
/// empty forbidden list — `forbidden_library_symbols_absent_from_src`'s
/// assert above guards against the latter shape; this test gives a
/// per-symbol audit trail).
#[test]
fn manifest_native_functions_cover_drained_libraries() {
    let forbidden = forbidden_library_symbols();
    // Phase 1a: crypto.  5 symbols (n_hmac_sha256_raw retired 2026-05-30).
    let crypto_expected: &[&str] = &[
        "n_sha256",
        "n_hmac_sha256",
        "n_base64_encode",
        "n_base64_decode",
        "n_base64url_encode",
    ];
    for sym in crypto_expected {
        assert!(
            forbidden.iter().any(|(s, _)| s == sym),
            "@PLAN12 phase 3.5 — crypto symbol `{sym}` missing from \
             forbidden list.  After Phase 3.5 dry-run, crypto was \
             extracted to `../loft-crypto/`; symbols are pinned via \
             `FORBIDDEN_LIBRARY_SYMBOLS_MANUAL` in this file.  Restore \
             the entry there."
        );
    }
    // Phase 1b: web.  19 symbols.
    let web_expected: &[&str] = &[
        "n_http_do",
        "n_http_body",
        "n_ws_connect",
        "n_ws_client_send",
        "n_ws_client_send_binary",
        "n_ws_client_recv",
        "n_ws_client_message",
        "n_ws_client_opcode",
        "n_ws_client_close",
        "n_sleep_ms",
        "n_pack_reset",
        "n_pack_u8",
        "n_pack_u16_le",
        "n_pack_u32_le",
        "n_pack_take",
        "n_byte_at",
        "n_ws_group_clear",
        "n_ws_group_add",
        "n_ws_group_poll",
    ];
    for sym in web_expected {
        assert!(
            forbidden.iter().any(|(s, _)| s == sym),
            "@PLAN12 — web symbol `{sym}` missing from forbidden list.  \
             web declares its native bindings via co-located `#native` \
             annotations in `lib/web/src/web.loft` (bare → `n_<fn>`, \
             explicit string → override); restore the annotation."
        );
    }
}

/// @PLAN12 — the clean-libraries gate.  Every in-monorepo `lib/<X>/native/`
/// cdylib must follow the drift-proof binding pattern (see
/// `doc/claude/lib_plans/12-library-extraction/REFERENCE.md`
/// § Clean libraries):
///   (1) NO `[native.functions]` manifest table — bindings live in the
///       co-located `#native` annotations next to each loft signature.
///   (2) the `loft_register!` list is GENERATED (the cdylib `include!`s
///       `loft_register_gen.rs` from `build.rs`), never hand-maintained.
///
/// EXCEPTIONS — libraries whose cdylib must register symbols BEYOND their
/// `#native`-bound public API (so the `#native`-scan can't produce the full
/// list) keep a hand-maintained `loft_register!`.  Each must justify why.
const CLEAN_REGISTER_EXCEPTIONS: &[(&str, &str)] = &[
    // graphics' cdylib exports internal `n_*` symbols (n_save_png,
    // n_rasterize_text_into, n_gl_upload_canvas/_vertices, n_gl_set_mat4,
    // n_audio_play_raw) that the GL / vector-arg paths invoke but no
    // `#native` annotation binds — they must be registered yet aren't
    // derivable from the `.loft` annotations.  Keeps its hand-written list.
    (
        "graphics",
        "cdylib registers internal n_* symbols beyond the #native public API",
    ),
];

#[test]
fn native_libraries_follow_clean_binding_pattern() {
    let lib_dir = workspace_root().join("lib");
    let mut violations: Vec<String> = Vec::new();
    let Ok(entries) = fs::read_dir(&lib_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        // Skip `.loft_test_tmp_*` artifact-isolation dirs (run_lib_test_in_temp_cwd).
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let native_src = path.join("native").join("src");
        if !native_src.is_dir() {
            continue;
        }
        let pkg = path
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        // (1) no [native.functions] manifest table.
        if let Ok(toml) = fs::read_to_string(path.join("loft.toml"))
            && toml.lines().any(|l| l.trim() == "[native.functions]")
        {
            violations.push(format!(
                "lib/{pkg}: loft.toml still has a `[native.functions]` table — \
                 declare bindings via co-located `#native` annotations instead"
            ));
        }
        // (2) register list generated, not hand-written.
        let mut rs = Vec::new();
        collect_rs_files(&native_src, &mut rs);
        let mut has_include = false;
        let mut hand_register: Option<String> = None;
        for f in &rs {
            let s = fs::read_to_string(f).unwrap_or_default();
            if s.contains("loft_register_gen.rs") {
                has_include = true;
            }
            // A literal `loft_register! {` macro invocation in the source
            // (the generated file lives in OUT_DIR, not scanned here).
            if s.contains("loft_register!") {
                hand_register = Some(f.display().to_string());
            }
        }
        let excepted = CLEAN_REGISTER_EXCEPTIONS.iter().any(|(p, _)| *p == pkg);
        if let Some(file) = hand_register
            && !has_include
            && !excepted
        {
            violations.push(format!(
                "lib/{pkg}: `{file}` hand-maintains `loft_register!` — generate \
                 it from the `#native` annotations via `build.rs` \
                 (`loft_ffi_build::generate_register_from_loft(\"../src\")` + \
                 `include!(\".../loft_register_gen.rs\")`)"
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "native-binding cleanliness violations (see REFERENCE.md § Clean libraries):\n{}",
        violations.join("\n")
    );
}
