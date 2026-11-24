// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F54 — the browser entry points

//! The browser ENTRY POINTS: `compile_and_run`, the frame-yielding game session and the
//! pipeline they share — parse, compile, run, report.  They sit in the top layer because
//! they drive the whole compiler; the host bridge they print and read files through stays
//! in `wasm`, which the stores and the runtime call (@PLN191).  wasm-bindgen exports by
//! function name, so moving them changes nothing a page imports.

#[allow(clippy::wildcard_imports)]
use crate::wasm::*;

// ── W1.9  compile_and_run() — WASM/native entry point ────────────────────────

/// Embedded default standard library files (compiled into the WASM binary).
/// The single source set lives in [`crate::stdlib_sources`] — shared with
/// `loft search`'s stdlib API feed so both read the same embedded `default/*`.
const DEFAULT_FILES: &[(&str, &str)] = crate::stdlib_sources::STDLIB_SOURCES;

/// Library files embedded in the WASM build so `use <name>;` resolves in the
/// browser sandbox without a native cdylib.
const BUNDLED_LIB_FILES: &[(&str, &str)] = &[
    // graphics + shapes are sourced from
    // fixture clones of loft-libs-graphics (see scripts/sync-fixtures.sh
    // PINNED_REFS); monorepo lib/{graphics,shapes}/ are gone.
    (
        "graphics.loft",
        include_str!("../tests/fixtures/libs/graphics/src/graphics.loft"),
    ),
    (
        "math.loft",
        include_str!("../tests/fixtures/libs/graphics/src/math.loft"),
    ),
    (
        "mesh.loft",
        include_str!("../tests/fixtures/libs/graphics/src/mesh.loft"),
    ),
    (
        "scene.loft",
        include_str!("../tests/fixtures/libs/graphics/src/scene.loft"),
    ),
    (
        "render.loft",
        include_str!("../tests/fixtures/libs/graphics/src/render.loft"),
    ),
    (
        "glb.loft",
        include_str!("../tests/fixtures/libs/graphics/src/glb.loft"),
    ),
    (
        "shapes.loft",
        include_str!("../tests/fixtures/libs/shapes/src/shapes.loft"),
    ),
    // TTT v3.5 — `use web` resolves in the browser via this baked-in
    // source.  The native fns it declares (`n_ws_*`, `n_http_*`,
    // `n_sleep_ms`, `n_pack_*`, `n_byte_at`) are registered in
    // `src/native.rs` under `cfg(all(target_arch="wasm32",
    // feature="wasm"))` and route through `host_*` JS bridges in
    // this file.
    // web is sourced from the fixture clone
    // of loft-libs-net (see scripts/sync-fixtures.sh PINNED_REFS).
    (
        "web.loft",
        include_str!("../tests/fixtures/libs/web/src/web.loft"),
    ),
    // The gallery showpiece reads its sprite atlas out of a content pack, so
    // `assets` has to resolve here or `25-brick-buster` does not compile in the
    // browser at all — which is how the gallery shipped loading cleanly and
    // drawing nothing (loft#1545).  Sourced from the fixture clone of
    // loft-libs-assets (see scripts/sync-fixtures.sh PINNED_REFS); its one
    // dependency, `shapes`, is already bundled above.
    //
    // ⚠ Bundling a library makes `use <id>;` RESOLVE, which is worth only as much
    // as the natives it then calls.  `assets` needs the paged loaders
    // (`store_load_key_text`) and reads its pack through the host filesystem, so
    // `paged_store` covers this build (build.rs) and `LocalFileProvider` has a
    // host arm (src/paged_reader.rs).  Without both, this row would trade a loud
    // "library not found" for a panic stub mid-frame.
    (
        "assets.loft",
        include_str!("../tests/fixtures/libs/assets/src/assets.loft"),
    ),
    // The browser kernel's loft surface: the SAME lib
    // source the native kernel uses (the script is the contract).
    (
        "engine_host.loft",
        include_str!("../lib/engine_host/src/engine_host.loft"),
    ),
];

/// Run a loft program supplied as a JSON array of `{name, content}` file objects.
///
/// Returns a JSON string: `{"output": "...", "diagnostics": [...], "success": true|false}`.
///
/// The default standard library files are embedded in the binary; user files
/// are taken from `files_json`.  Any `use <id>;` statement is resolved against
/// files whose name matches `<id>.loft` in the supplied file list.
///
/// # Errors
/// Returns a JSON error object if `files_json` cannot be parsed.
///
/// When compiled with `--features wasm` and exported via `wasm-bindgen`, this
/// function is callable from JavaScript as:
/// ```js
/// const result = JSON.parse(loft.compile_and_run(JSON.stringify([
///   {name: 'main.loft', content: 'fn main() { println("hi") }'}
/// ])));
/// ```
#[cfg_attr(feature = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
pub fn compile_and_run(files_json: &str) -> String {
    #[cfg(feature = "wasm")]
    console_error_panic_hook::set_once();
    // Parse the JSON input.
    let files = match parse_files_json(files_json) {
        Ok(f) => f,
        Err(e) => {
            return format!(
                "{{\"output\":\"\",\"diagnostics\":{},\"success\":false}}",
                json_str(&e)
            );
        }
    };

    // The page's files: the defaults, the graphics library and the user's.
    let mut all_files: Vec<(String, String)> = DEFAULT_FILES
        .iter()
        .chain(BUNDLED_LIB_FILES.iter())
        .map(|(n, c)| (n.to_string(), (*c).to_string()))
        .collect();
    for (name, content) in &files {
        // @PLN13 step 6 — auto-detect a beginner script (loose top-level statements, no
        // `fn main`) and desugar it to one run-once `fn main`, exactly as the CLI run
        // path does, so the browser playground runs scripts too. A non-script (a library
        // or a `fn main` program) desugars to `None` and is pushed unchanged.
        let desugared = crate::script::script_desugar(content);
        let content = desugared.as_deref().unwrap_or(content);
        all_files.push((name.clone(), content.to_string()));
    }
    // Build and run.
    let (diag, had_error, asserts, output) = run_pipeline(&all_files);

    // Build asserts JSON array.
    let asserts_json = if asserts.is_empty() {
        "[]".to_string()
    } else {
        let items: Vec<String> = asserts
            .iter()
            .map(|(pass, msg, file, line)| {
                format!(
                    "{{\"pass\":{pass},\"message\":{},\"file\":{},\"line\":{line}}}",
                    json_str(msg),
                    json_str(file),
                )
            })
            .collect();
        format!("[{}]", items.join(","))
    };

    format!(
        "{{\"output\":{},\"diagnostics\":{},\"asserts\":{asserts_json},\"success\":{}}}",
        json_str(&output),
        json_str(&diag),
        !had_error,
    )
}

// ── FY.2–FY.3  Game session with frame yield ────────────────────────────────

/// Persistent game session that survives across frame yields.
/// The `State` is BOXED: `execute_argv` stores raw pointers into the `State` itself (its
/// bytecode and library, in the parallel context), and the session is moved twice after
/// that — into this struct, then into its [`Game`].  A box moves, its contents do not —
/// the pattern `live_dispatch::bootstrap_core` already follows.  The definition table needs
/// no such care: the `State` co-owns it.
pub(crate) struct GameSession {
    state: Box<crate::interp::state::State>,
}

/// `f` over a game's session slot.
fn with_game<R>(
    slot: &mut Option<GameSession>,
    f: impl FnOnce(&mut Option<GameSession>) -> R,
) -> R {
    f(slot)
}

/// `@PLN226` — a page's game: the program it started and runs frame by frame, and the world
/// snapshot staged for the next program it starts (@PLN18 08-S6).  The page holds the handle and
/// passes it to every call (`@FR-C-Boundary`): two games on one page never meet, and nothing is
/// kept per thread.
///
/// ```js
/// const game = new Game();
/// let r = JSON.parse(game.start(JSON.stringify([{ name: 'main.loft', content: src }])));
/// while (r.running) { await nextFrame(); r = JSON.parse(game.resume_frame()); }
/// ```
#[cfg_attr(feature = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
#[derive(Default)]
pub struct Game {
    session: Option<GameSession>,
    stage: Option<String>,
}

#[cfg_attr(feature = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
impl Game {
    /// A game with nothing started.
    #[cfg_attr(feature = "wasm", wasm_bindgen::prelude::wasm_bindgen(constructor))]
    #[must_use]
    pub fn new() -> Game {
        Game::default()
    }

    /// Start the program in `files_json` (`[{name, content}]`), ending this game's previous
    /// one: parse, compile, execute until the first frame yield.  JSON
    /// `{"ok":true,"running":true}`, `{"ok":true,"running":false,"output":"…"}` when it ran to
    /// its end, or `{"ok":false,"error":"…"}`.
    pub fn start(&mut self, files_json: &str) -> String {
        let stage = self.stage.take();
        start_on(&mut self.session, stage, files_json)
    }

    /// Run the next frame.  JSON `{"running":true}` (with `"output"` when the frame printed),
    /// `{"running":false,"output":"…"}` when the program finished, or
    /// `{"running":false,"error":"…"}` when it crashed.
    pub fn resume_frame(&mut self) -> String {
        resume_on(&mut self.session)
    }

    /// End the running program and close its window.
    pub fn stop(&mut self) {
        dispose(&mut self.session);
    }

    /// Stage `snapshot` (`swap_export` of another game) for the next program this game starts:
    /// its `swap_world` restores it — the browser's form of the native `LOFT_RESUME`.
    pub fn stage_swap(&mut self, snapshot: &str) {
        self.stage = Some(snapshot.to_string());
    }

    /// The running program's world, for another game to take over (`stage_swap`).
    pub fn swap_export(&mut self) -> String {
        export_on(&mut self.session)
    }

    /// May the program in `files_json` take over this game's world?
    pub fn swap_check(&mut self, files_json: &str) -> String {
        check_on(&mut self.session, files_json)
    }
}

/// End a game's running program and close its window.
fn dispose(slot: &mut Option<GameSession>) {
    if slot.is_some() {
        #[cfg(feature = "wasm")]
        {
            let args = js_sys::Array::new();
            host_call_raw("gl_destroy_window", &args);
        }
        *slot = None;
    }
}

/// The program's own file among the page's: the first, by name, that is neither the stdlib
/// nor a bundled library.
fn main_file(all_files: &[(String, String)]) -> Option<String> {
    let libs: std::collections::HashSet<&str> = BUNDLED_LIB_FILES.iter().map(|(n, _)| *n).collect();
    all_files
        .iter()
        .map(|(n, _)| n)
        .filter(|k| !k.starts_with("default/") && !libs.contains(k.as_str()))
        .min()
        .cloned()
}

/// The page's files as the run reads them (`Stores::page_files`).
fn page_files(
    all_files: &[(String, String)],
) -> std::sync::Arc<std::collections::HashMap<String, String>> {
    std::sync::Arc::new(all_files.iter().cloned().collect())
}

/// [`Game::start`] on a game's session `slot`, handing the program the `stage`d world.
fn start_on(slot: &mut Option<GameSession>, stage: Option<String>, files_json: &str) -> String {
    #[cfg(feature = "wasm")]
    console_error_panic_hook::set_once();
    dispose(slot);

    let files = match parse_files_json(files_json) {
        Ok(f) => f,
        Err(e) => return format!("{{\"ok\":false,\"error\":{}}}", json_str(&e)),
    };

    let mut all_files: Vec<(String, String)> = DEFAULT_FILES
        .iter()
        .chain(BUNDLED_LIB_FILES.iter())
        .map(|(n, c)| (n.to_string(), (*c).to_string()))
        .collect();
    for (name, content) in &files {
        // @PLN13 step 6 — auto-detect a beginner script (loose top-level statements, no
        // `fn main`) and desugar it to one run-once `fn main`, exactly as the CLI run
        // path does, so the browser playground runs scripts too. A non-script (a library
        // or a `fn main` program) desugars to `None` and is pushed unchanged.
        let desugared = crate::script::script_desugar(content);
        let content = desugared.as_deref().unwrap_or(content);
        all_files.push((name.clone(), content.to_string()));
    }
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use crate::diagnostics::Level;
        use crate::interp::compile::byte_code;
        use crate::interp::state::State;
        use crate::parser::Parser;
        use crate::scopes;

        let mut p = Parser::new();
        p.lexer.add_page_files(&all_files);
        for (name, _) in DEFAULT_FILES {
            p.parse(name, true);
            if p.diagnostics.level() >= Level::Error {
                return Err(p.diagnostics.to_string());
            }
        }
        let Some(main_name) = main_file(&all_files) else {
            return Err("no user file found".to_string());
        };
        p.parse(&main_name, false);
        if p.diagnostics.level() >= Level::Error {
            return Err(p.diagnostics.to_string());
        }
        scopes::check(&mut p.data, &mut p.database);
        if p.diagnostics.level() >= Level::Error {
            return Err(p.diagnostics.to_string());
        }
        // Boxed BEFORE any pointer is taken, so the pointers `execute_argv` keeps survive the
        // session being moved into its game (see `GameSession`).
        let mut state = Box::new(State::new(p.database));
        // The page captures what the program prints, and the program reads the page's files
        // while its first frame runs.
        state.database.output = Some(String::new());
        state.database.page_files = Some(page_files(&all_files));
        crate::engine_host::stage_swap(&mut state.database, stage);
        let mut data = p.data;
        byte_code(&mut state, &mut data);
        crate::interp::wasm_gl::register_wgl_natives(&mut state);
        // The State co-owns the table, so the session may move freely.
        state.execute_argv("main", std::sync::Arc::new(data), &[]);
        state.database.page_files = None;
        // execute_argv returns either because the program finished or because
        // frame_yield was set.  Store the session for resume_frame.
        Ok(GameSession { state })
    }));

    match result {
        Ok(Ok(mut session)) => {
            let yielded = session.state.database.frame_yield;
            if yielded {
                *slot = Some(session);
                "{\"ok\":true,\"running\":true}".to_string()
            } else {
                let out = take_output(&mut session);
                format!(
                    "{{\"ok\":true,\"running\":false,\"output\":{}}}",
                    json_str(&out)
                )
            }
        }
        Ok(Err(diag)) => {
            format!("{{\"ok\":false,\"error\":{}}}", json_str(&diag))
        }
        Err(_panic) => {
            *slot = None;
            "{\"ok\":false,\"error\":\"internal panic\"}".to_string()
        }
    }
}

/// [`Game::resume_frame`] on a game's session `game`.
fn resume_on(game: &mut Option<GameSession>) -> String {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        with_game(game, |slot| {
            let Some(session) = slot.as_mut() else {
                return "{\"running\":false}".to_string();
            };
            let still_running = session.state.resume();
            if still_running {
                // Carry whatever the program printed DURING this frame.  A running
                // session answering the bare `{"running":true}` would hold everything a
                // long-lived program said until it stopped — and a game does not stop,
                // so a program reporting that it could not load its assets would say so
                // into a buffer nobody reads, the same silence as a missing host method
                // one layer up.
                //
                // `take_output` CLEARS as it reads, so this is the only copy: the page
                // appends it rather than replacing, or draining here would lose
                // everything printed before the final frame.
                let out = take_output(session);
                if out.is_empty() {
                    "{\"running\":true}".to_string()
                } else {
                    format!("{{\"running\":true,\"output\":{}}}", json_str(&out))
                }
            } else {
                let out = take_output(session);
                *slot = None;
                format!("{{\"running\":false,\"output\":{}}}", json_str(&out))
            }
        })
    }));
    match result {
        Ok(json) => json,
        Err(_panic) => {
            *game = None;
            #[cfg(feature = "wasm")]
            {
                let args = js_sys::Array::new();
                host_call_raw("gl_destroy_window", &args);
            }
            "{\"running\":false,\"error\":\"internal panic\"}".to_string()
        }
    }
}

/// Parse `[{name: string, content: string}]` JSON into a Vec of pairs.
fn parse_files_json(json: &str) -> Result<Vec<(String, String)>, String> {
    let json = json.trim();
    if !json.starts_with('[') {
        return Err("expected JSON array".to_string());
    }
    // Minimal hand-rolled parser sufficient for well-formed wasm-bridge input.
    // Avoids pulling in a full JSON library.
    let mut result = Vec::new();
    let mut i = 1usize; // skip '['
    let bytes = json.as_bytes();
    let len = bytes.len();
    while i < len {
        // Skip whitespace and commas.
        while i < len && matches!(bytes[i], b' ' | b',' | b'\n' | b'\r' | b'\t') {
            i += 1;
        }
        if i >= len || bytes[i] == b']' {
            break;
        }
        if bytes[i] != b'{' {
            return Err(format!("unexpected char at {i}"));
        }
        i += 1; // consume '{'
        let name = extract_json_field(json, &mut i, "name")?;
        let content = extract_json_field(json, &mut i, "content")?;
        result.push((name, content));
        // Advance past any remaining fields to the closing '}'.
        while i < len && bytes[i] != b'}' {
            i += 1;
        }
        if i < len {
            i += 1; // consume '}'
        }
    }
    Ok(result)
}

/// Extract a `"key": "value"` pair from a JSON object string starting at `*pos`.
/// Advances `*pos` to just past the closing `"` of the value.
fn extract_json_field(json: &str, pos: &mut usize, key: &str) -> Result<String, String> {
    let key_pat = format!("\"{key}\"");
    if let Some(k) = json[*pos..].find(&key_pat) {
        let after_key = *pos + k + key_pat.len();
        if let Some(colon) = json[after_key..].find(':') {
            let after_colon = after_key + colon + 1;
            let (value, end) = extract_json_string(json, after_colon)?;
            *pos = end;
            return Ok(value);
        }
    }
    Err(format!("field '{key}' not found"))
}

/// Extract a JSON string value starting near `start`.
/// Returns `(unescaped_content, byte_position_after_closing_quote)`.
fn extract_json_string(json: &str, start: usize) -> Result<(String, usize), String> {
    let slice = &json[start..];
    let trimmed = slice.trim_start();
    let offset = start + (slice.len() - trimmed.len()); // absolute position of opening '"'
    if !trimmed.starts_with('"') {
        return Err("expected string".to_string());
    }
    let inner = &trimmed[1..]; // skip opening '"'
    let mut out = String::new();
    let mut chars = inner.char_indices();
    while let Some((byte_off, c)) = chars.next() {
        match c {
            '"' => {
                // byte_off is relative to inner; +1 for opening '"', +1 past closing '"'
                return Ok((out, offset + 1 + byte_off + 1));
            }
            '\\' => match chars.next() {
                Some((_, 'n')) => out.push('\n'),
                Some((_, 't')) => out.push('\t'),
                Some((_, 'r')) => out.push('\r'),
                Some((_, '"')) => out.push('"'),
                Some((_, '\\')) => out.push('\\'),
                Some((_, '/')) => out.push('/'),
                Some((_, c)) => out.push(c),
                None => return Err("unterminated escape".to_string()),
            },
            c => out.push(c),
        }
    }
    Err("unterminated string".to_string())
}

/// Minimally escape a string for JSON output.
fn json_str(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Execute the full pipeline over the page's files, `all_files`: the diagnostics, whether the
/// run failed, its asserts and what it printed.
/// Returns `(diagnostic_string, had_error)`.  Warnings produce a non-empty
/// diagnostic string but `had_error = false`; errors set `had_error = true`.
/// Assert result: (passed, message, file, line).
type AssertResult = (bool, String, String, u32);

fn run_pipeline(all_files: &[(String, String)]) -> (String, bool, Vec<AssertResult>, String) {
    use crate::diagnostics::Level;
    use crate::interp::compile::byte_code;
    use crate::interp::state::State;
    use crate::parser::Parser;
    use crate::scopes;

    let mut p = Parser::new();
    p.lexer.add_page_files(all_files);
    for (name, _) in DEFAULT_FILES {
        p.parse(name, true);
        let lvl = p.diagnostics.level();
        if lvl == Level::Error || lvl == Level::Fatal {
            return (p.diagnostics.to_string(), true, Vec::new(), String::new());
        }
    }
    let Some(main_name) = main_file(all_files) else {
        return (
            "no user file found".to_string(),
            true,
            Vec::new(),
            String::new(),
        );
    };
    p.parse(&main_name, false);
    let lvl = p.diagnostics.level();
    if lvl == Level::Error || lvl == Level::Fatal {
        return (p.diagnostics.to_string(), true, Vec::new(), String::new());
    }
    scopes::check(&mut p.data, &mut p.database);
    let lvl = p.diagnostics.level();
    if lvl == Level::Error || lvl == Level::Fatal {
        return (p.diagnostics.to_string(), true, Vec::new(), String::new());
    }
    let diag = p.diagnostics.to_string();
    let mut state = State::new(p.database);
    state.database.output = Some(String::new());
    state.database.page_files = Some(page_files(all_files));
    byte_code(&mut state, &mut p.data);
    // GL6.1–GL6.3: register WebGL bridge functions for graphics library.
    crate::interp::wasm_gl::register_wgl_natives(&mut state);
    // Enable assert reporting for the playground.
    state.database.report_asserts = true;
    state.execute_argv("main", &p.data, &[]);
    let asserts = std::mem::take(&mut state.database.assert_results);
    let had_fatal = state.database.had_fatal;
    let mut output = state.database.output.take().unwrap_or_default();
    #[cfg(feature = "wasm")]
    output.push_str(&std::mem::take(&mut *state.database.par_trace.borrow_mut()));
    (diag, had_fatal, asserts, output)
}

/// What the game printed since the last call, taken out of its capture.
fn take_output(session: &mut GameSession) -> String {
    let mut out = session
        .state
        .database
        .output
        .as_mut()
        .map(std::mem::take)
        .unwrap_or_default();
    #[cfg(feature = "wasm")]
    out.push_str(&std::mem::take(
        &mut *session.state.database.par_trace.borrow_mut(),
    ));
    out
}

// ── @PLN117 — the thread pool ───────────────────────────────────────────────
//
// A threaded gallery bundle gets its pool from `crate::wasm_threads`, the same
// runtime the raw `loft --html` bundle links: rayon over Web Workers the page
// spawns.  Its entry points are plain wasm exports (`loft_pool_new`,
// `loft_pool_build`, `loft_rayon_start_worker`), driven from `doc/loft-thread.js`
// — nothing here to re-export.  Until the page starts them, `par` takes rayon's
// single-threaded fallback and returns the same values, only sequentially.

/// @PLN117 step 0 — arm the parallel-worker tracer from JS (the browser has no
/// env vars).  With it on, a `par` reports `distinct_workers=N` into the
/// program output, so the harness can prove dispatch really crossed >=2 Web
/// Worker threads.
#[cfg(feature = "wasm-threads")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn set_par_trace_workers(on: bool) {
    crate::par_pool::set_par_trace(on);
}

// @PLN117 step 4 — the hand-rolled `worker_entry` stub (a never-implemented
// W1.18-2 no-op) and its JS glue (`tests/wasm/{worker,parallel}.mjs`,
// `harness.mjs::initThreaded`) were retired.  Browser threading now rides the
// wasm-bindgen-rayon pool (the same rayon scheduler as native) — ONE scheduler,
// not two half-built ones.  See `set_par_trace_workers` / `init_thread_pool`
// above and `doc/claude/plans/117-browser-multithreading/`.

/// Export the registered world of the PARKED (frame-yielded) run; "" when no run is parked
/// or no world was registered.
///
/// @PLN196 step 8 in the browser — a world that is the root of its own store goes over as
/// that store's image with its layout sidecar (`crate::wasm::SWAP_IMAGE_MARKER`), which the
/// next game's `swap_world` reads through the converter: keyed collections and references
/// survive, and a changed struct converts.  A world sharing its store goes over as JSON, as
/// before.
fn export_on(slot: &mut Option<GameSession>) -> String {
    with_game(slot, |gs| {
        gs.as_mut().map_or_else(String::new, |session| {
            let Some((root, kt)) = crate::engine_host::world_root(&session.state.database) else {
                return String::new();
            };
            let db = &mut session.state.database;
            if let Some((sidecar, image)) = db.world_image(&root, kt) {
                return format!(
                    "{SWAP_IMAGE_MARKER}\n{}\n{}",
                    crate::base64::encode(sidecar.as_bytes()),
                    crate::base64::encode(&image)
                );
            }
            let mut json = String::new();
            db.show_json(&mut json, &root, kt, false);
            json
        })
    })
}

/// @PLN196 step 8 — may the program in `files_json` take over the PARKED run's world?  The
/// browser's form of a live rebuild's compile-time check (`LOFT_CHECK_WORLD`): the page asks
/// it BEFORE `Game::start`, which disposes the running instance, so a refused change
/// leaves the running build serving.  The program is parsed and scope-checked, not run, and
/// the parked run is not touched.
///
/// Answers JSON: `{"ok":true,"changes":[…]}` (the conversion the swap will do, empty when
/// the layout is the same or no world is registered), `{"ok":false,"error":"…"}` for a
/// program that does not compile or a world it cannot take over.
fn check_on(slot: &mut Option<GameSession>, files_json: &str) -> String {
    let world = with_game(slot, |gs| {
        gs.as_mut().and_then(|session| {
            let (root, kt) = crate::engine_host::world_root(&session.state.database)?;
            session
                .state
                .database
                .world_image(&root, kt)
                .map(|(sidecar, _)| sidecar)
        })
    });
    let files = match parse_files_json(files_json) {
        Ok(f) => f,
        Err(e) => return format!("{{\"ok\":false,\"error\":{}}}", json_str(&e)),
    };
    let mut all_files: Vec<(String, String)> = DEFAULT_FILES
        .iter()
        .chain(BUNDLED_LIB_FILES.iter())
        .map(|(n, c)| (n.to_string(), (*c).to_string()))
        .collect();
    for (name, content) in &files {
        let desugared = crate::script::script_desugar(content);
        all_files.push((name.clone(), desugared.unwrap_or_else(|| content.clone())));
    }
    let verdict = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        use crate::diagnostics::Level;
        let mut p = crate::parser::Parser::new();
        p.lexer.add_page_files(&all_files);
        for (name, _) in DEFAULT_FILES {
            p.parse(name, true);
        }
        let Some(main_name) = main_file(&all_files) else {
            return Err("no user file found".to_string());
        };
        p.parse(&main_name, false);
        if p.diagnostics.level() < Level::Error {
            crate::scopes::check(&mut p.data, &mut p.database);
        }
        if p.diagnostics.level() >= Level::Error {
            return Err(p.diagnostics.to_string());
        }
        let Some(sidecar) = world else {
            return Ok(Vec::new());
        };
        let routines = crate::parser::Parser::upgrade_routines(&p.data);
        match p.database.world_upgrade_plan(&sidecar, &routines)? {
            crate::database::WorldPlan::Same => Ok(Vec::new()),
            crate::database::WorldPlan::Converts(changes) => Ok(changes),
        }
    }));
    match verdict {
        Ok(Ok(changes)) => {
            let list: Vec<String> = changes.iter().map(|c| json_str(c)).collect();
            format!("{{\"ok\":true,\"changes\":[{}]}}", list.join(","))
        }
        Ok(Err(why)) => format!("{{\"ok\":false,\"error\":{}}}", json_str(&why)),
        Err(_) => "{\"ok\":false,\"error\":\"internal panic\"}".to_string(),
    }
}
