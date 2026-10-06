// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I91 — Editor tooling: language server (LSP), debug adapter (DAP), resolution index
//
// loft-lsp — the loft Language Server (LSP over JSON-RPC / stdio).
//
// @PLN63 S1 transport + S3 diagnostics.  The read-dispatch loop handles the
// `initialize` / `shutdown` / `exit` lifecycle (S1) and the document lifecycle
// (`didOpen` / `didChange` / `didClose`); on each edit it re-parses the buffer
// with a fresh stdlib-loaded parser (`loft::lsp::diagnose`) and pushes
// `textDocument/publishDiagnostics` so the editor shows squiggles live.  The
// later providers (outline / hover / definition, S4-S6) hang off this same loop.
//
// The compiler coupling lives in the library (`loft::lsp`), so this binary owns
// only the wire protocol + deployment concerns (locating the stdlib).
//
// The JSON on the wire is loft's OWN parser/serializer (`loft::json`), not an
// external crate — the same "own your dependencies" rule as the rest of the tree.
//
// Protocol channel discipline: stdout carries ONLY framed JSON-RPC; anything
// else (logging) must go to stderr, or the transport corrupts.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};

use loft::diagnostics::{DiagEntry, Level};
use loft::json::{self, Parsed};

/// @PLN131 — where a diagnostic code's documentation lives, as the `codeDescription` link
/// an editor renders on the code itself.
///
/// The GitHub blob URL rather than the docs site: `loft-lang.org/loft/` publishes
/// `doc/*.html` (the generated stdlib reference) and has no diagnostics page, so pointing
/// there would open onto something real but unrelated — which is the weaker half of a door
/// onto nothing.
///
/// It targets `main`, and `DIAGNOSTICS.md` is not on `main` until this work merges — so the
/// link 404s from a branch build and resolves from a release. That is the right coupling
/// rather than a hazard: a user reaches this binary through a release cut from `main`, which
/// is the same commit that carries the file. `the_code_links_to_an_anchor_that_exists` pins
/// the `#the-codes` anchor against the local copy, which is the half that can rot silently.
///
/// One URL for every code — the table is in-page searchable, which is what a
/// self-describing SLUG is for.
const DIAGNOSTIC_DOC_URL: &str =
    "https://github.com/loft-lang/loft/blob/main/doc/claude/DIAGNOSTICS.md#the-codes";

const SERVER_NAME: &str = "loft-lsp";
const SERVER_VERSION: &str = loft::manifest::LOFT_RUNNING_VERSION;

fn main() {
    // Opt this server into the stdlib startup cache: the parse accessors then
    // warm-load the precompiled stdlib `Data` bundle (~12× faster than
    // re-parsing `default/` on every edit) instead of cold-parsing each request.
    // Only default it on — an explicit override (the var set to empty = off) is
    // respected.  SAFETY: set before any thread is spawned (single-threaded
    // startup), the one point Rust 2024 requires for `set_var`.
    if std::env::var_os("LOFT_STDLIB_CACHE").is_none() {
        unsafe { std::env::set_var("LOFT_STDLIB_CACHE", "1") };
    }

    let mut stdin = io::stdin().lock();
    let stdout = io::stdout();
    let mut shutdown_requested = false;
    // Resolve the stdlib once; re-parsing the buffer per edit reloads it, but
    // the DIRECTORY never moves during a session.
    let stdlib_dir = resolve_stdlib_dir();
    // uri -> current full text.  Sets up S4-S6 (outline/hover/definition need
    // the live buffer); S3 also parses straight from it on each edit.
    let mut documents: HashMap<String, String> = HashMap::new();
    // The source formatter, compiled LAZILY on the first formatting request
    // (compiling it loads the stdlib + the formatter program — heavy — and many
    // sessions never format).  `formatter_tried` distinguishes "not yet built"
    // from "built and failed" so a failed build isn't retried every request.
    let mut formatter: Option<loft::lsp::Formatter> = None;
    let mut formatter_tried = false;
    // The workspace root (from `initialize`) and the tracker index under it —
    // `index/tags.json` + `features.json` (`make index`).  Lazy + cached; absent
    // outside the loft repo, so the tag features are simply inert there.
    let mut workspace_root: Option<String> = None;
    let mut tag_index: Option<loft::lsp::TagIndex> = None;
    // The mtime of `index/tags.json` the cached `tag_index` was loaded from — used
    // to reload it after a mid-session `make index` (None = never loaded / absent).
    let mut tag_index_mtime: Option<std::time::SystemTime> = None;
    // The workspace reverse index (identifier occurrences across the tree),
    // built lazily on the first find-references request; open buffers are
    // overlaid at query time so unsaved edits are reflected.
    let mut workspace_index: Option<loft::lsp::WorkspaceIndex> = None;
    let mut workspace_index_tried = false;
    // Ids of the requests the SERVER sends (`window/showDocument`); the client's replies
    // carry them back and are ignored.
    let mut server_request_id: i64 = 0;

    while let Some(body) = read_message(&mut stdin) {
        // A frame that isn't valid JSON is skipped, not fatal — a robust server
        // keeps the session alive rather than dying on one bad message.
        let Ok(msg) = json::parse(&body) else {
            continue;
        };

        let method = obj_str(&msg, "method").unwrap_or_default();
        // Presence of `id` distinguishes a REQUEST (needs a reply) from a
        // NOTIFICATION (must not be replied to).
        let id = obj_get(&msg, "id").cloned();

        match (method.as_str(), id) {
            ("initialize", Some(id)) => {
                workspace_root = initialize_root(&msg);
                send(&stdout, &response(id, initialize_result()));
            }
            ("initialized", None) => {} // notification — no reply
            ("textDocument/didOpen", None) => {
                if let Some((uri, text)) = did_open_params(&msg) {
                    let mut diags = diagnose_text(&text, &uri, &stdlib_dir);
                    ensure_tag_index(&workspace_root, &mut tag_index, &mut tag_index_mtime);
                    diags.extend(tag_diagnostics(&text, tag_index.as_ref())); // T3
                    documents.insert(uri.clone(), text);
                    send(&stdout, &publish_diagnostics(&uri, diags));
                }
            }
            ("textDocument/didChange", None) => {
                if let Some((uri, text)) = did_change_params(&msg) {
                    let mut diags = diagnose_text(&text, &uri, &stdlib_dir);
                    ensure_tag_index(&workspace_root, &mut tag_index, &mut tag_index_mtime);
                    diags.extend(tag_diagnostics(&text, tag_index.as_ref())); // T3
                    documents.insert(uri.clone(), text);
                    send(&stdout, &publish_diagnostics(&uri, diags));
                }
            }
            ("textDocument/didClose", None) => {
                if let Some(uri) = did_close_uri(&msg) {
                    documents.remove(&uri);
                    // Clear the editor's squiggles for a closed file: publish an
                    // empty list (LSP has no separate "clear" message).
                    send(&stdout, &publish_diagnostics(&uri, Vec::new()));
                }
            }
            ("textDocument/didSave", None) => {
                // The saved file's on-disk content changed, so the cached workspace
                // reverse index is stale.  Drop it: the next find-references rebuilds
                // from disk.  (Open buffers are overlaid at query time, so unsaved
                // edits already count — this catches a saved-then-closed file, whose
                // overlay is gone but whose new disk content the index must reflect.)
                workspace_index = None;
                workspace_index_tried = false;
            }
            ("textDocument/documentSymbol", Some(id)) => {
                // Outline the CURRENTLY-OPEN buffer (never re-read from disk — the
                // editor's copy is the source of truth).  Unknown doc → empty list.
                let symbols = text_document_uri(&msg)
                    .and_then(|uri| documents.get(&uri))
                    .map(|text| document_symbols(text, &stdlib_dir))
                    .unwrap_or_default();
                send(&stdout, &response(id, Parsed::Array(symbols)));
            }
            ("textDocument/codeAction", Some(id)) => {
                // Step B: turn each diagnostic that carries a structured
                // `suggestion` into a "Change to `X`" quick-fix (a WorkspaceEdit).
                let actions = code_actions(&msg, &documents, &stdlib_dir);
                send(&stdout, &response(id, Parsed::Array(actions)));
            }
            ("textDocument/semanticTokens/full", Some(id)) => {
                // Step D: classify the buffer's identifier tokens, delta-encoded.
                let data = text_document_uri(&msg)
                    .and_then(|uri| documents.get(&uri))
                    .map(|text| {
                        encode_semantic_tokens(&loft::lsp::semantic_tokens(
                            text,
                            "buf.loft",
                            &stdlib_dir,
                        ))
                    })
                    .unwrap_or_default();
                send(
                    &stdout,
                    &response(id, obj(vec![("data", Parsed::Array(data))])),
                );
            }
            ("textDocument/inlayHint", Some(id)) => {
                // S7 (@PLN115): inferred-type hints at assignment-local declarations,
                // positioned via the resolution index (the substrate E was blocked on).
                let hints = text_document_uri(&msg)
                    .and_then(|uri| documents.get(&uri))
                    .map(|text| {
                        loft::lsp::inlay_hints(text, "buf.loft", &stdlib_dir)
                            .iter()
                            .map(inlay_hint_json)
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                send(&stdout, &response(id, Parsed::Array(hints)));
            }
            ("textDocument/completion", Some(id)) => {
                // T4: a partial `@`-tag → tracker-tag candidates from the index;
                // else step C — members after `expr.`, or in-scope names + keywords.
                // Clone the buffer out so the `documents` borrow drops before the
                // mutable `tag_index` access.
                let at = text_document_position(&msg).and_then(|(uri, line, ch)| {
                    Some((documents.get(&uri)?.clone(), line, ch, uri))
                });
                let items = match at {
                    Some((text, line, ch, uri)) => {
                        if let Some(prefix) =
                            loft::lsp::tag_completion_prefix(&text, line + 1, ch + 1)
                        {
                            ensure_tag_index(&workspace_root, &mut tag_index, &mut tag_index_mtime);
                            tag_index
                                .as_ref()
                                .map(|idx| {
                                    idx.complete(&prefix).iter().map(completion_item).collect()
                                })
                                .unwrap_or_default()
                        } else {
                            loft::lsp::complete(&text, "buf.loft", &stdlib_dir, line + 1, ch + 1)
                                .iter()
                                .map(|c| completion_item_in(c, &uri))
                                .collect()
                        }
                    }
                    None => Vec::new(),
                };
                send(&stdout, &response(id, Parsed::Array(items)));
            }
            ("completionItem/resolve", Some(id)) => {
                let item = resolve_completion(&msg, &documents, &stdlib_dir);
                send(&stdout, &response(id, item));
            }
            ("textDocument/formatting", Some(id)) => {
                // Run the same formatter the `loft fmt` CLI uses on the open buffer;
                // reply with ONE whole-document edit (or none when already tidy).
                let edits =
                    match text_document_uri(&msg).and_then(|uri| documents.get(&uri).cloned()) {
                        Some(text) => {
                            if !formatter_tried {
                                formatter_tried = true;
                                formatter = loft::lsp::Formatter::new(&stdlib_dir);
                            }
                            match formatter.as_mut().and_then(|f| f.format(&text)) {
                                // A no-op edit is noise; only send one when it changes the text.
                                Some(formatted) if formatted != text => {
                                    vec![full_document_edit(&text, &formatted)]
                                }
                                _ => Vec::new(),
                            }
                        }
                        None => Vec::new(),
                    };
                send(&stdout, &response(id, Parsed::Array(edits)));
            }
            ("textDocument/hover", Some(id)) => {
                // Clone the buffer out so the `documents` borrow is dropped before
                // the mutable `tag_index` access below.
                let at = text_document_position(&msg).and_then(|(uri, line, ch)| {
                    Some((documents.get(&uri)?.clone(), line, ch, uri_to_path(&uri)))
                });
                let hover = match at {
                    Some((text, line, ch, file)) => {
                        // T1: a tracker tag under the cursor wins over symbol hover.
                        let tag_hit = loft::lsp::tags_in(&text)
                            .into_iter()
                            .find(|(_, l, s, e)| *l == line + 1 && *s <= ch && ch < *e);
                        let tag_hover = tag_hit.and_then(|(tag, l, s, e)| {
                            ensure_tag_index(&workspace_root, &mut tag_index, &mut tag_index_mtime);
                            let info = tag_index.as_ref()?.lookup(&tag)?;
                            let contents = markup(&loft::lsp::render_tag_markdown(&info));
                            Some(obj(vec![
                                ("contents", contents),
                                ("range", range_of(l - 1, s, e)),
                            ]))
                        });
                        tag_hover
                            .or_else(|| hover_with_construct(&text, &stdlib_dir, line, ch, &file))
                            .unwrap_or(Parsed::Null)
                    }
                    None => Parsed::Null,
                };
                send(&stdout, &response(id, hover));
            }
            ("textDocument/references", Some(id)) => {
                // Clone the buffer out first so the `documents` borrow is dropped.
                let req = text_document_position(&msg).and_then(|(uri, line, ch)| {
                    let text = documents.get(&uri)?.clone();
                    Some((uri, text, line, ch))
                });
                let locations = match req {
                    Some((uri, text, line, ch)) => {
                        let file = uri_to_path(&uri);
                        // S4 (@PLN115): an assignment-local resolves to its EXACT binding
                        // occurrences via the resolution index — a same-named field access
                        // (`p.x` vs local `x`) is excluded, which the name-scan can't do.
                        // Params / loop binders / globals return None → the F-v1 path.
                        if let Some(refs) = loft::lsp::local_binding_refs(
                            &text,
                            &stdlib_dir,
                            &file,
                            line + 1,
                            ch + 1,
                        ) {
                            let len = loft::lsp::identifier_at(&text, line + 1, ch + 1)
                                .map_or(0, |n| n.chars().count() as u32);
                            refs.iter().map(|r| location_of(r, len)).collect()
                        } else if let Some(refs) = loft::lsp::method_refs(
                            &text,
                            line + 1,
                            ch + 1,
                            workspace_root.as_deref().unwrap_or(""),
                            &open_overlays(&documents),
                            &stdlib_dir,
                        ) {
                            // @PLN115: a METHOD resolves to its exact receiver-typed
                            // occurrences across the workspace (`text.len`, not every
                            // `len`) — keyed on the stable mangled method name.
                            let len = loft::lsp::identifier_at(&text, line + 1, ch + 1)
                                .map_or(0, |n| n.chars().count() as u32);
                            refs.iter().map(|r| location_of(r, len)).collect()
                        } else {
                            match loft::lsp::identifier_at(&text, line + 1, ch + 1) {
                                Some(name) => {
                                    ensure_workspace_index(
                                        &workspace_root,
                                        &mut workspace_index,
                                        &mut workspace_index_tried,
                                    );
                                    // F: a LOCAL scopes to its function; a global stays workspace-wide.
                                    let scope = loft::lsp::reference_scope(
                                        &text,
                                        &name,
                                        &stdlib_dir,
                                        line + 1,
                                    );
                                    references_of(
                                        &name,
                                        workspace_index.as_ref(),
                                        &documents,
                                        &scope,
                                        &file,
                                    )
                                }
                                None => Vec::new(),
                            }
                        }
                    }
                    None => Vec::new(),
                };
                send(&stdout, &response(id, Parsed::Array(locations)));
            }
            ("textDocument/prepareRename", Some(id)) => {
                // The identifier's range + placeholder IF renamable (not stdlib),
                // else null so the editor won't offer a rename box.
                let result = text_document_position(&msg)
                    .and_then(|(uri, line, ch)| {
                        let text = documents.get(&uri)?;
                        let (name, s, e) =
                            loft::lsp::prepare_rename(text, line + 1, ch + 1, &stdlib_dir)?;
                        Some(obj(vec![
                            ("range", range_of(line, s, e)),
                            ("placeholder", Parsed::Str(name)),
                        ]))
                    })
                    .unwrap_or(Parsed::Null);
                send(&stdout, &response(id, result));
            }
            ("textDocument/rename", Some(id)) => {
                ensure_workspace_index(
                    &workspace_root,
                    &mut workspace_index,
                    &mut workspace_index_tried,
                );
                match rename_edit(&msg, &documents, workspace_index.as_ref(), &stdlib_dir) {
                    Ok(edit) => send(&stdout, &response(id, edit)),
                    // A refused/invalid rename → a JSON-RPC error the editor surfaces.
                    Err(reason) => send(&stdout, &error_response(id, -32803, &reason)),
                }
            }
            ("textDocument/documentLink", Some(id)) => {
                // T2: every tracker tag with an issue URL becomes a clickable link.
                let links = text_document_uri(&msg)
                    .and_then(|uri| documents.get(&uri).cloned())
                    .map(|text| {
                        ensure_tag_index(&workspace_root, &mut tag_index, &mut tag_index_mtime);
                        document_links(&text, tag_index.as_ref())
                    })
                    .unwrap_or_default();
                send(&stdout, &response(id, Parsed::Array(links)));
            }
            ("textDocument/definition", Some(id)) => {
                let location = text_document_position(&msg)
                    .and_then(|(uri, line, ch)| {
                        let text = documents.get(&uri)?;
                        // @PLN115: index-first (resolves locals / methods / fields
                        // by position), then the name-based fallback.
                        let h = loft::lsp::resolve_at(text, &stdlib_dir, line + 1, ch + 1)
                            .or_else(|| {
                                loft::lsp::symbol_at(
                                    text,
                                    "buf.loft",
                                    &stdlib_dir,
                                    line + 1,
                                    ch + 1,
                                )
                            })?;
                        Some(definition_location(&uri, &h, &stdlib_dir))
                    })
                    .unwrap_or(Parsed::Null); // unresolved → null
                send(&stdout, &response(id, location));
            }
            ("workspace/executeCommand", Some(id)) => {
                // @PLN183 P3 — `loft.overview`: write the overview site and ask the editor to
                // open it (a type's own page when the action was invoked on a type name).
                match execute_overview(&msg, &documents, &stdlib_dir) {
                    Some(Ok(page)) => {
                        send(&stdout, &response(id, Parsed::Null));
                        server_request_id += 1;
                        send(&stdout, &show_document(server_request_id, &page));
                    }
                    Some(Err(why)) => send(&stdout, &error_response(id, -32603, &why)),
                    None => send(&stdout, &error_response(id, -32602, "unknown command")),
                }
            }
            ("shutdown", Some(id)) => {
                shutdown_requested = true;
                send(&stdout, &response(id, Parsed::Null));
            }
            ("exit", _) => {
                // LSP: exit 0 iff `shutdown` came first, else 1.
                std::process::exit(i32::from(!shutdown_requested));
            }
            ("", Some(_)) => {
                // A REPLY from the client to a request this server sent (`window/showDocument`,
                // @PLN183): it has an `id` and no `method`, and it must not be answered — an
                // error response to a response is a protocol violation.
            }
            (_, Some(id)) => {
                // Unknown REQUEST → JSON-RPC MethodNotFound; a request must always
                // get exactly one reply, even if we don't handle it yet.
                send(&stdout, &error_response(id, -32601, "method not found"));
            }
            (_, None) => {} // unknown notification — ignore
        }
    }
    // stdin closed without `exit` — treat as a clean end of session.
}

// ── diagnostics (S3) ─────────────────────────────────────────────────────────
/// Parse `text` and map its Warning+ diagnostics to LSP `Diagnostic` objects.
/// `uri` is only a label for the parser's internal filename; the wire position
/// comes from the buffer, and the URI on the notification is set by the caller.
fn diagnose_text(text: &str, uri: &str, stdlib_dir: &str) -> Vec<Parsed> {
    let diags = loft::lsp::diagnose(text, uri, stdlib_dir);
    diags
        .entries()
        .iter()
        // `>= Advice`, not `>= Warning`: an editor is exactly where advice belongs — a
        // faint Hint the author can act on or ignore.  Filtering it out would make the
        // tier invisible in the one place it is most useful, and the deprecation steer
        // (its main occupant) would silently stop being offered.  Debug stays excluded.
        .filter(|e| e.level >= Level::Advice)
        .map(|e| lsp_diagnostic(e, text, uri))
        .collect()
}

/// One loft `DiagEntry` -> one LSP `Diagnostic`.  loft positions are 1-based
/// (line 1, col 1 = first char); LSP positions are 0-based, so both drop by one.
/// loft records a single point, so the range underlines the identifier at that
/// point (its extent read from `text`) — a visible squiggle under the token,
/// not a zero-width caret.
fn lsp_diagnostic(e: &DiagEntry, text: &str, uri: &str) -> Parsed {
    let line0 = e.line.saturating_sub(1);
    let col0 = e.col.saturating_sub(1);
    let end_col = col0 + token_len_at(text, line0, col0);
    let range = range_of(line0, col0, end_col);
    let mut fields = vec![
        ("range", range),
        ("severity", Parsed::Int(lsp_severity(e.level))),
        ("source", Parsed::Str("loft".into())),
        ("message", Parsed::Str(e.message.clone())),
    ];
    if let Some(code) = e.code {
        fields.push(("code", Parsed::Str(code.into())));
        // @PLN131 — the DOOR, as a link the editor renders on the code itself (LSP 3.16
        // `codeDescription`). The concept handle was CLI text until now; here it is one
        // click. Every code has a row in that table, and `every_pinned_code_is_documented`
        // is what keeps that true, so this cannot become the dead door the plan refuses.
        fields.push((
            "codeDescription",
            obj(vec![("href", Parsed::Str(DIAGNOSTIC_DOC_URL.into()))]),
        ));
    }
    // @PLN131 — every fix as `relatedInformation`, which is where an editor shows detail
    // that is not itself a problem.
    //
    // This is the half the code-action path CANNOT carry: a quick-fix needs an `edit`, and
    // 57 of 62 fixes have none — they name a rewrite the compiler cannot place. Without
    // this the editor shows a message that (since the prose trim) deliberately no longer
    // says what to write, and the cure lives only in the CLI. The condition rides along for
    // the same reason it does everywhere else: it is what a reader affirms.
    if !e.fixes.is_empty() {
        let related: Vec<Parsed> = e
            .fixes
            .iter()
            .map(|f| {
                let mut msg = format!("fix: {}", f.title);
                if let Some(c) = &f.condition {
                    let _ = write!(msg, " — only if {c}");
                }
                let _ = write!(msg, "  [{} · {}]", f.concept, f.concept_ref);
                // @PLN183 P4 — the concept's catalogue entry, as an address the editor links.
                if let Some(entry) = loft::doc_catalogue::by_tag(f.concept_ref) {
                    let _ = write!(msg, " {}", entry.page());
                }
                obj(vec![
                    (
                        "location",
                        obj(vec![
                            ("uri", Parsed::Str(uri.to_string())),
                            ("range", range_of(line0, col0, end_col)),
                        ]),
                    ),
                    ("message", Parsed::Str(msg)),
                ])
            })
            .collect();
        fields.push(("relatedInformation", Parsed::Array(related)));
    }
    // Round-trip the structured suggestion on the diagnostic's `data` — the
    // editor echoes it back in a `codeAction` request, so the quick-fix needs no
    // re-parse (step A→B).
    // @PLN131 step 4 — the same round-trip for FIXES. Each carries its own span, so a
    // quick-fix edits where the rewrite belongs rather than where the squiggle is: the two
    // differ (a cast is reported past the statement terminator and inserts its `?` at the
    // type's end), and using the diagnostic's range would write to the wrong place.
    //
    // The `condition` rides along so the editor can show what a click AFFIRMS. That is the
    // whole of the interactive tier rule: a conditional fix is clickable — the click is the
    // affirmation — provided the condition is in the line the author reads.
    let fixes: Vec<Parsed> = e
        .fixes
        .iter()
        .filter_map(|f| {
            let ed = f.edit.as_ref()?;
            let (l0, c0) = (ed.line.saturating_sub(1), ed.col.saturating_sub(1));
            let mut row = vec![
                ("title", Parsed::Str(f.title.clone())),
                ("range", range_of(l0, c0, c0 + ed.len)),
                ("newText", Parsed::Str(ed.text.clone())),
                (
                    "mechanical",
                    Parsed::Bool(f.kind == loft::diagnostics::FixKind::Mechanical),
                ),
                ("concept", Parsed::Str(f.concept.into())),
            ];
            if let Some(c) = &f.condition {
                row.push(("condition", Parsed::Str(c.clone())));
            }
            Some(obj(row))
        })
        .collect();
    if e.suggestion.is_some() || !fixes.is_empty() {
        let mut data = Vec::new();
        if let Some(suggestion) = &e.suggestion {
            data.push(("suggestion", Parsed::Str(suggestion.clone())));
        }
        if !fixes.is_empty() {
            data.push(("fixes", Parsed::Array(fixes)));
        }
        fields.push(("data", obj(data)));
    }
    obj(fields)
}

/// LSP DiagnosticSeverity: Error 1, Warning 2, Information 3, Hint 4.
fn lsp_severity(level: Level) -> i64 {
    match level {
        Level::Fatal | Level::Error => 1,
        Level::Warning => 2,
        Level::Debug => 3,
        // Hint, not Information: advice reports correct code, so an editor should
        // render it as a suggestion (a faint underline) rather than a problem.
        Level::Advice => 4,
    }
}

/// The character length of the identifier starting at (`line0`, `col0`) in
/// `text` — so the squiggle underlines the whole token.  Non-identifier or
/// past-end positions get length 1 (a single-character caret, never zero-width).
fn token_len_at(text: &str, line0: u32, col0: u32) -> u32 {
    let Some(line) = text.lines().nth(line0 as usize) else {
        return 1;
    };
    let chars: Vec<char> = line.chars().collect();
    let start = col0 as usize;
    if start >= chars.len() {
        return 1;
    }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    if !is_word(chars[start]) {
        return 1;
    }
    let len = chars[start..].iter().take_while(|&&c| is_word(c)).count();
    len.max(1) as u32
}

fn position(line: u32, character: u32) -> Parsed {
    obj(vec![
        ("line", Parsed::Int(i64::from(line))),
        ("character", Parsed::Int(i64::from(character))),
    ])
}

/// An LSP `Range` on one line from `start_char` to `end_char` (all 0-based).
fn range_of(line: u32, start_char: u32, end_char: u32) -> Parsed {
    obj(vec![
        ("start", position(line, start_char)),
        ("end", position(line, end_char)),
    ])
}

/// An LSP `Range` spanning lines — the whole-document form a full replacement needs.
fn range_of2(sl: u32, sc: u32, el: u32, ec: u32) -> Parsed {
    obj(vec![("start", position(sl, sc)), ("end", position(el, ec))])
}

/// An LSP `InlayHint` JSON object (kind 1 = Type) for a resolved hint.  The `line`
/// and `col` are 1-based; LSP positions are 0-based.
fn inlay_hint_json(h: &loft::lsp::InlayHint) -> Parsed {
    obj(vec![
        (
            "position",
            position(h.line.saturating_sub(1), h.col.saturating_sub(1)),
        ),
        ("label", Parsed::Str(h.label.clone())),
        ("kind", Parsed::Int(1)),
    ])
}

/// Delta-encode classified tokens into the LSP semantic-tokens flat int array:
/// five ints per token `[Δline, Δcol, len, tokenType, tokenModifiers]`, each
/// position relative to the previous token.  Tokens must arrive sorted.
fn encode_semantic_tokens(tokens: &[loft::lsp::SemanticToken]) -> Vec<Parsed> {
    let mut out = Vec::with_capacity(tokens.len() * 5);
    let (mut prev_line, mut prev_col) = (0u32, 0u32);
    for t in tokens {
        let line0 = t.line.saturating_sub(1);
        let col0 = t.col.saturating_sub(1);
        let delta_line = line0.saturating_sub(prev_line);
        let delta_col = if delta_line == 0 {
            col0.saturating_sub(prev_col)
        } else {
            col0
        };
        out.push(Parsed::Int(i64::from(delta_line)));
        out.push(Parsed::Int(i64::from(delta_col)));
        out.push(Parsed::Int(i64::from(t.len)));
        out.push(Parsed::Int(i64::from(t.kind)));
        out.push(Parsed::Int(0)); // no modifiers
        prev_line = line0;
        prev_col = col0;
    }
    out
}

/// One `loft::lsp::Completion` → an LSP `CompletionItem`.
fn completion_item(c: &loft::lsp::Completion) -> Parsed {
    obj(vec![
        ("label", Parsed::Str(c.label.clone())),
        ("kind", Parsed::Int(i64::from(c.kind))),
        ("detail", Parsed::Str(c.detail.clone())),
    ])
}

/// The same item with the document it came from, so `completionItem/resolve` can answer its
/// documentation (@PLN183 P4) without the list paying for every item's lookup.
fn completion_item_in(c: &loft::lsp::Completion, uri: &str) -> Parsed {
    let mut item = completion_item(c);
    if let Parsed::Object(entries) = &mut item {
        entries.push((
            "data".to_string(),
            0,
            obj(vec![("uri", Parsed::Str(uri.to_string()))]),
        ));
    }
    item
}

/// `completionItem/resolve`: the item back, with its `documentation` — a keyword's catalogue
/// entry, or a name's signature and doc (`lsp::completion_documentation`).
fn resolve_completion(
    msg: &Parsed,
    documents: &HashMap<String, String>,
    stdlib_dir: &str,
) -> Parsed {
    let Some(item) = obj_get(msg, "params").cloned() else {
        return Parsed::Null;
    };
    let label = obj_str(&item, "label").unwrap_or_default();
    let keyword = obj_get(&item, "kind").and_then(Parsed::as_i64) == Some(14);
    let text = obj_get(&item, "data")
        .and_then(|d| obj_str(d, "uri"))
        .and_then(|u| documents.get(&u).cloned())
        .unwrap_or_default();
    let doc = loft::lsp::completion_documentation(&text, stdlib_dir, &label, keyword);
    match (item, doc) {
        (Parsed::Object(mut entries), Some(doc)) => {
            entries.push(("documentation".to_string(), 0, markup(&doc)));
            Parsed::Object(entries)
        }
        (item, _) => item,
    }
}

// ── codeAction (quick-fixes from diagnostic suggestions, step B) ─────────────
/// For each diagnostic the editor sends back that carries a `data.suggestion`
/// (round-tripped from step A), a `CodeAction` quick-fix whose `WorkspaceEdit`
/// replaces the diagnostic's range with the suggestion.  loft already COMPUTED
/// the fix ("did you mean 'X'") — this just applies it.
fn code_actions(
    msg: &Parsed,
    documents: &HashMap<String, String>,
    stdlib_dir: &str,
) -> Vec<Parsed> {
    let Some(params) = obj_get(msg, "params") else {
        return Vec::new();
    };
    let Some(uri) = obj_get(params, "textDocument").and_then(|d| obj_str(d, "uri")) else {
        return Vec::new();
    };
    let mut actions: Vec<Parsed> = Vec::new();
    // Step B — quick-fixes from each diagnostic that carries a structured suggestion.
    if let Some(Parsed::Array(diags)) =
        obj_get(params, "context").and_then(|c| obj_get(c, "diagnostics"))
    {
        // The legacy `data.suggestion` quickfix, ONLY where the diagnostic carries no
        // structured fix. Every `suggest_last` site now also emits a `Fix` (@PLN131's
        // did-you-mean arc), and offering both put the same rename in the list twice under
        // two different titles. `fixes` wins because it is the one that carries a tier, a
        // concept and a door — `suggestion` is a replacement token and nothing else.
        actions.extend(
            diags
                .iter()
                .filter(|d| {
                    !matches!(
                        obj_get(d, "data").and_then(|x| obj_get(x, "fixes")),
                        Some(Parsed::Array(f)) if !f.is_empty()
                    )
                })
                .filter_map(|d| quickfix_from_diagnostic(d, &uri)),
        );
        // @PLN131 step 4 — one quick-fix per spelled fix, both tiers.
        actions.extend(diags.iter().flat_map(|d| fix_actions(d, &uri)));
    }
    // @PLN131 — one `source.fixAll` over everything mechanical AND verified in the buffer.
    if let Some(text) = documents.get(&uri)
        && let Some(action) = fix_all_action(&uri, text, stdlib_dir)
    {
        actions.push(action);
    }
    // E4 (extract-function) — on a MULTI-LINE selection (a real statement selection,
    // never a bare cursor or a single-line diagnostic range, so a quick-fix request is
    // unaffected), offer `refactor.extract` with the WorkspaceEdit the data-flow
    // engine computes.  Absent when the selection does not map to whole statements.
    if let Some(range) = obj_get(params, "range").filter(|r| range_is_multiline(r))
        && let Some(text) = documents.get(&uri)
        && let Some(action) = extract_function_action(&uri, range, text, stdlib_dir)
    {
        actions.push(action);
    }
    // @PLN183 P3 — "what can I use here?", at any position: a command, not an edit, so the
    // client runs it through `workspace/executeCommand` and the server opens the overview.
    if wants_kind(params, OVERVIEW_KIND) {
        let start = obj_get(params, "range").and_then(|r| obj_get(r, "start"));
        let at = |k: &str| {
            start
                .and_then(|p| obj_get(p, k))
                .and_then(Parsed::as_i64)
                .unwrap_or(0)
        };
        actions.push(obj(vec![
            ("title", Parsed::Str(OVERVIEW_TITLE.into())),
            ("kind", Parsed::Str(OVERVIEW_KIND.into())),
            (
                "command",
                obj(vec![
                    ("title", Parsed::Str(OVERVIEW_TITLE.into())),
                    ("command", Parsed::Str(OVERVIEW_COMMAND.into())),
                    (
                        "arguments",
                        Parsed::Array(vec![
                            Parsed::Str(uri.clone()),
                            Parsed::Int(at("line")),
                            Parsed::Int(at("character")),
                        ]),
                    ),
                ]),
            ),
        ]));
    }
    actions
}

const OVERVIEW_KIND: &str = "source.loft.overview";
const OVERVIEW_COMMAND: &str = "loft.overview";
const OVERVIEW_TITLE: &str = "loft: what can I use here?";

/// Whether a code-action request admits `kind`: no `context.only`, or an entry of it that is
/// `kind` or a dotted prefix of it (`source` admits `source.loft.overview`).
fn wants_kind(params: &Parsed, kind: &str) -> bool {
    match obj_get(params, "context").and_then(|c| obj_get(c, "only")) {
        Some(Parsed::Array(only)) => only.iter().any(|k| match k {
            Parsed::Str(k) => kind == k || kind.starts_with(&format!("{k}.")),
            _ => false,
        }),
        _ => true,
    }
}

/// `workspace/executeCommand loft.overview [uri, line, character]`: write the overview site
/// and answer the page to open — the type's own page when the word at the position names a
/// type of the buffer's program, else the root.  `None` for any other command.
fn execute_overview(
    msg: &Parsed,
    documents: &HashMap<String, String>,
    stdlib_dir: &str,
) -> Option<Result<PathBuf, String>> {
    let params = obj_get(msg, "params")?;
    if obj_str(params, "command").as_deref() != Some(OVERVIEW_COMMAND) {
        return None;
    }
    let args = match obj_get(params, "arguments") {
        Some(Parsed::Array(a)) => a.clone(),
        _ => Vec::new(),
    };
    let uri = args.first().and_then(|a| match a {
        Parsed::Str(s) => Some(s.clone()),
        _ => None,
    });
    let num = |i: usize| {
        args.get(i)
            .and_then(Parsed::as_i64)
            .and_then(|n| u32::try_from(n).ok())
    };
    let dir = loft::doc_site::site_dir();
    let project = uri.as_deref().map(uri_to_path);
    let (pages, stamp) = overview_pages(project.as_deref());
    if let Err(e) = loft::doc_site::write_site(&dir, &pages, &stamp) {
        return Some(Err(format!(
            "the overview could not be written to {}: {e}",
            dir.display()
        )));
    }
    if let (Some(uri), Some(line), Some(ch)) = (uri, num(1), num(2))
        && let Some(text) = documents.get(&uri)
        && let Some(caps) =
            loft::lsp::type_capabilities_at(text, "buf.loft", stdlib_dir, line + 1, ch + 1)
    {
        let page = loft::doc_site::type_page(&caps);
        let path = dir.join(&page.name);
        return Some(
            loft::file_access::write(&loft::file_access::PathText::from_os(&path), &page.text)
                .map(|()| path)
                .map_err(|e| e.to_string()),
        );
    }
    Some(Ok(dir.join(loft::doc_site::ROOT)))
}

/// Every page of the overview and the stamp that says what they depend on: the loft version
/// (the embedded catalogue is fixed per build), the `loft.lock` of the project `file` sits in,
/// and the registry index on this machine.
fn overview_pages(file: Option<&str>) -> (Vec<loft::doc_site::Page>, String) {
    let mut pages = loft::doc_site::feature_pages();
    let lock_path = file.and_then(|f| {
        Path::new(f)
            .ancestors()
            .skip(1)
            .map(|d| d.join("loft.lock"))
            .find(|p| loft::file_access::is_file(&loft::file_access::PathText::from_os(p)))
    });
    let lock_text = lock_path
        .as_ref()
        .and_then(|p| {
            loft::file_access::read_to_string(&loft::file_access::PathText::from_os(p)).ok()
        })
        .unwrap_or_default();
    let locked: Vec<(String, String)> = loft::lockfile::parse(&lock_text)
        .map(|l| {
            l.packages
                .into_iter()
                .map(|p| (p.name, p.version))
                .collect()
        })
        .unwrap_or_default();
    #[cfg(feature = "registry")]
    let index_stamp = {
        let (idx_path, _, _) = loft::registry_index::index_paths();
        let index_stamp =
            loft::file_access::metadata(&loft::file_access::PathText::from_os(&idx_path))
                .map(|m| format!("{} {:?}", m.len(), m.modified().ok()))
                .unwrap_or_default();
        let installed: Vec<(String, String)> = loft::registry_index::installed_packages()
            .into_iter()
            .map(|(n, v, _)| (n, v))
            .collect();
        let index = loft::install::cached_index();
        pages.extend(loft::doc_site::library_pages(
            index.as_ref().map_err(String::as_str),
            &locked,
            &installed,
        ));
        format!("{index_stamp}\n{installed:?}")
    };
    // Built without the registry, the libraries page says so rather than going missing.
    #[cfg(not(feature = "registry"))]
    let index_stamp = {
        let _ = &locked;
        pages.push(loft::doc_site::Page {
            name: "libraries.md".to_string(),
            text: "[← what you can use](index.md)\n\n# Libraries\n\nThis loft was built \
                   without the package registry, so it lists no libraries.\n"
                .to_string(),
        });
        String::new()
    };
    let stamp = format!(
        "{}\n{}\n{lock_text}\n{index_stamp}",
        loft::manifest::LOFT_RUNNING_VERSION,
        lock_path
            .map(|p| p.display().to_string())
            .unwrap_or_default()
    );
    (pages, stamp)
}

/// The `window/showDocument` request (LSP 3.16) that opens `page` in the editor.
fn show_document(id: i64, page: &Path) -> Parsed {
    obj(vec![
        ("jsonrpc", Parsed::Str("2.0".into())),
        ("id", Parsed::Int(id)),
        ("method", Parsed::Str("window/showDocument".into())),
        (
            "params",
            obj(vec![
                ("uri", Parsed::Str(file_uri(page))),
                ("takeFocus", Parsed::Bool(true)),
            ]),
        ),
    ])
}

/// One "Change to `X`" quick-fix from a diagnostic carrying `data.suggestion`.
fn quickfix_from_diagnostic(d: &Parsed, uri: &str) -> Option<Parsed> {
    let suggestion = obj_str(obj_get(d, "data")?, "suggestion")?;
    let range = obj_get(d, "range")?.clone();
    let edit = obj(vec![
        ("range", range),
        ("newText", Parsed::Str(suggestion.clone())),
    ]);
    // `{ uri: [TextEdit] }` — uri is a runtime String, so build the object directly.
    let changes = Parsed::Object(vec![(uri.to_string(), 0, Parsed::Array(vec![edit]))]);
    Some(obj(vec![
        ("title", Parsed::Str(format!("Change to `{suggestion}`"))),
        ("kind", Parsed::Str("quickfix".into())),
        ("diagnostics", Parsed::Array(vec![d.clone()])),
        ("isPreferred", Parsed::Bool(true)),
        ("edit", obj(vec![("changes", changes)])),
    ]))
}

/// @PLN131 step 4 — the quick-fixes a diagnostic's `data.fixes` carries.
///
/// Both tiers are offered, which is the plan's rule and not an oversight: a conditional fix
/// is one click for a veteran, because the click IS the affirmation. What makes that safe is
/// that the condition is in the title they read before clicking, so the thing being affirmed
/// cannot be missed. `isPreferred` is reserved for mechanical fixes — an editor may apply a
/// preferred action from a "fix all" gesture, and nobody would be reading a condition then.
fn fix_actions(d: &Parsed, uri: &str) -> Vec<Parsed> {
    let Some(Parsed::Array(fixes)) = obj_get(d, "data").and_then(|x| obj_get(x, "fixes")) else {
        return Vec::new();
    };
    fixes
        .iter()
        .filter_map(|f| {
            let title = obj_str(f, "title")?;
            let new_text = obj_str(f, "newText")?;
            let range = obj_get(f, "range")?.clone();
            let mechanical = matches!(obj_get(f, "mechanical"), Some(Parsed::Bool(true)));
            // The condition belongs in the TITLE, not beside it: a code-action list shows
            // titles, so a condition anywhere else is a condition the clicker never saw.
            let label = match obj_str(f, "condition") {
                Some(c) if !mechanical => format!("{title} — only if {c}"),
                _ => title,
            };
            let edit = obj(vec![("range", range), ("newText", Parsed::Str(new_text))]);
            let changes = Parsed::Object(vec![(uri.to_string(), 0, Parsed::Array(vec![edit]))]);
            Some(obj(vec![
                ("title", Parsed::Str(label)),
                ("kind", Parsed::Str("quickfix".into())),
                ("diagnostics", Parsed::Array(vec![d.clone()])),
                ("isPreferred", Parsed::Bool(mechanical)),
                ("edit", obj(vec![("changes", changes)])),
            ]))
        })
        .collect()
}

/// @PLN131 — the `source.fixAll` action: every mechanical, verified fix in one edit.
///
/// It delegates to `fix_apply::apply_fixes`, the same function behind `loft fix --apply`, so
/// the editor and the CLI cannot drift — an editor that applied a different set would be a
/// second implementation of "which fixes are safe", which is the one thing this must not be.
/// All three gates come with it: mechanical only, spells a placeable edit, and VERIFIES —
/// each candidate is applied to an in-memory copy and the analysis re-run before it counts.
///
/// The edit replaces the WHOLE document, as a formatter's does. Verification already
/// re-parsed the buffer, so the rewritten text is a known-good artefact; emitting it whole
/// is what makes editor output byte-identical to the CLI's, rather than merely equivalent.
///
/// Cost is real — one re-parse per candidate fix — and acceptable here for a reason it would
/// not be on `didChange`: fix-all is a deliberate gesture, not something that runs per
/// keystroke. `None` when nothing would change, so the action never appears as a no-op.
fn fix_all_action(uri: &str, text: &str, stdlib_dir: &str) -> Option<Parsed> {
    let diags = loft::lsp::diagnose(text, "buf.loft", stdlib_dir);
    let (rewritten, report) = loft::fix_apply::apply_fixes(text, "buf.loft", stdlib_dir, &diags);
    if rewritten == *text {
        return None;
    }
    let n = report.iter().filter(|r| r.written).count();
    let (end_l, end_c) = doc_end(text);
    let edit = obj(vec![
        ("range", range_of2(0, 0, end_l, end_c)),
        ("newText", Parsed::Str(rewritten)),
    ]);
    let changes = Parsed::Object(vec![(uri.to_string(), 0, Parsed::Array(vec![edit]))]);
    Some(obj(vec![
        ("title", Parsed::Str(format!("Apply {n} verified fix(es)"))),
        ("kind", Parsed::Str("source.fixAll".into())),
        ("edit", obj(vec![("changes", changes)])),
    ]))
}

/// True when a `Range` spans more than one line — the E0 discriminator for a real
/// statement selection (a bare cursor and a single-line diagnostic range are not).
fn range_is_multiline(range: &Parsed) -> bool {
    let line = |end: &str| {
        obj_get(range, end)
            .and_then(|p| obj_get(p, "line"))
            .and_then(Parsed::as_i64)
    };
    match (line("start"), line("end")) {
        (Some(s), Some(e)) => s != e,
        _ => false,
    }
}

/// E4 — the `refactor.extract` action: a `WorkspaceEdit` that inserts the new
/// function after the buffer and replaces the selected statement lines with the call
/// (both ranges in ORIGINAL-document coordinates, per LSP).  `None` when the selection
/// does not map to whole statements (`loft::lsp::extract_function`).
fn extract_function_action(
    uri: &str,
    range: &Parsed,
    text: &str,
    stdlib_dir: &str,
) -> Option<Parsed> {
    let (start_line, end_line) = range_lines(range)?;
    let e = loft::lsp::extract_function(text, "buf.loft", stdlib_dir, start_line, end_line)?;
    // Insert the new function at the buffer end (a zero-width edit there).
    let (end_l, end_c) = doc_end(text);
    let insert = obj(vec![
        (
            "range",
            obj(vec![
                ("start", position(end_l, end_c)),
                ("end", position(end_l, end_c)),
            ]),
        ),
        ("newText", Parsed::Str(format!("\n{}\n", e.new_function))),
    ]);
    // Replace the selected lines (0-based) with the call.
    let last0 = e.end_line - 1;
    let last_len = text
        .lines()
        .nth(last0 as usize)
        .map_or(0, |l| l.chars().count() as u32);
    let replace = obj(vec![
        (
            "range",
            obj(vec![
                ("start", position(e.start_line - 1, 0)),
                ("end", position(last0, last_len)),
            ]),
        ),
        ("newText", Parsed::Str(e.call.clone())),
    ]);
    let changes = Parsed::Object(vec![(
        uri.to_string(),
        0,
        Parsed::Array(vec![replace, insert]),
    )]);
    Some(obj(vec![
        ("title", Parsed::Str("Extract to function".into())),
        ("kind", Parsed::Str("refactor.extract".into())),
        ("edit", obj(vec![("changes", changes)])),
    ]))
}

/// The 1-based INCLUSIVE line range of an LSP selection `Range`.  An end at column 0
/// on a later line (a whole-line selection) stops at the previous line.
fn range_lines(range: &Parsed) -> Option<(u32, u32)> {
    let coord = |end: &str, axis: &str| {
        obj_get(range, end)
            .and_then(|p| obj_get(p, axis))
            .and_then(Parsed::as_i64)
    };
    let sl = coord("start", "line")?;
    let el = coord("end", "line")?;
    let ec = coord("end", "character")?;
    let end_line = if ec == 0 && el > sl { el } else { el + 1 };
    Some((u32::try_from(sl + 1).ok()?, u32::try_from(end_line).ok()?))
}

fn publish_diagnostics(uri: &str, diagnostics: Vec<Parsed>) -> Parsed {
    let params = obj(vec![
        ("uri", Parsed::Str(uri.into())),
        ("diagnostics", Parsed::Array(diagnostics)),
    ]);
    notification("textDocument/publishDiagnostics", params)
}

// ── outline / document symbols (S4) ──────────────────────────────────────────
/// Outline the buffer `text`: map each top-level `loft::lsp::Symbol` to an LSP
/// `DocumentSymbol`.  The parser labels the source with a fixed name (it only
/// tags the internal filename; positions come from the buffer itself).
fn document_symbols(text: &str, stdlib_dir: &str) -> Vec<Parsed> {
    loft::lsp::outline(text, "buf.loft", stdlib_dir)
        .iter()
        .map(|s| lsp_document_symbol(s, text))
        .collect()
}

/// One `Symbol` -> one LSP `DocumentSymbol`.  `range` and `selectionRange` both
/// point at the NAME on its declaration line: the parser records a def's
/// position at the body start (past the name), so — to make the Outline jump to
/// the name — the name is located in the source line (`name_range`), with the
/// recorded position as a fallback.
fn lsp_document_symbol(sym: &loft::lsp::Symbol, text: &str) -> Parsed {
    let range = name_range(text, sym);
    obj(vec![
        ("name", Parsed::Str(sym.name.clone())),
        ("kind", Parsed::Int(symbol_kind(sym.kind))),
        ("range", range.clone()),
        ("selectionRange", range),
    ])
}

/// LSP `SymbolKind` for a `classify` label.  Unknown labels fall back to
/// Function (12) rather than dropping the symbol.
fn symbol_kind(kind: &str) -> i64 {
    match kind {
        "struct" => 23,
        "enum" => 10,
        "method" => 6,
        "constant" => 14,
        "interface" => 11,
        "operator" => 25,
        "typedef" => 5, // no dedicated typedef kind; Class is the conventional fallback
        _ => 12,        // "fn" and anything unrecognized
    }
}

/// A `Range` covering the symbol's NAME on its declaration line.  The display
/// name may be a method `Type.method`; the source token is the last dotted
/// segment, so search for that.  Falls back to the parser's recorded position
/// (never zero-width) when the name isn't found on the line.
fn name_range(text: &str, sym: &loft::lsp::Symbol) -> Parsed {
    let line0 = sym.line.saturating_sub(1);
    let needle = sym.name.rsplit('.').next().unwrap_or(&sym.name);
    if let Some(line) = text.lines().nth(line0 as usize)
        && let Some(byte_idx) = line.find(needle)
    {
        let start = line[..byte_idx].chars().count() as u32;
        let end = start + needle.chars().count() as u32;
        return range_of(line0, start, end);
    }
    let col0 = sym.col.saturating_sub(1);
    range_of(line0, col0, col0 + 1)
}

/// `params.textDocument.uri` — shared by documentSymbol + formatting.
fn text_document_uri(msg: &Parsed) -> Option<String> {
    obj_str(obj_get(obj_get(msg, "params")?, "textDocument")?, "uri")
}

// ── formatting ───────────────────────────────────────────────────────────────
/// A single LSP `TextEdit` replacing the WHOLE document with `formatted`.  The
/// range spans from the start to the true end of `original` (past the last
/// character), so it covers the buffer regardless of a trailing newline.
fn full_document_edit(original: &str, formatted: &str) -> Parsed {
    let (end_line, end_char) = doc_end(original);
    let range = obj(vec![
        ("start", position(0, 0)),
        ("end", position(end_line, end_char)),
    ]);
    obj(vec![
        ("range", range),
        ("newText", Parsed::Str(formatted.into())),
    ])
}

/// The 0-based position just past the last character of `text` (its end).
fn doc_end(text: &str) -> (u32, u32) {
    let mut line = 0u32;
    let mut col = 0u32;
    for ch in text.chars() {
        if ch == '\n' {
            line += 1;
            col = 0;
        } else {
            col += 1;
        }
    }
    (line, col)
}

/// The hover at a position: the symbol there (`hover_result`), and the language construct
/// there from the feature catalogue (@PLN183 P4) — the construct alone where no symbol answers
/// (`??`, `match`, `v[1]`), under the symbol's own card for a type keyword (`integer`, `hash`).
fn hover_with_construct(
    text: &str,
    stdlib_dir: &str,
    line0: u32,
    char0: u32,
    file: &str,
) -> Option<Parsed> {
    // @PLN183 P5 — on a `use` line, the library's card: the version this project uses, what it
    // is, and where its guide and API pages are.
    if let Some(card) = use_line_card(text, line0, char0, file) {
        return Some(obj(vec![("contents", markup(&card))]));
    }
    let construct = loft::lsp::construct_hover(text, stdlib_dir, line0 + 1, char0 + 1);
    // On punctuation (`[`, `??`, `..`) only the construct answers: the symbol resolution there
    // reaches the neighbouring name, which is not what the cursor is on.
    let on_word = text
        .lines()
        .nth(line0 as usize)
        .and_then(|l| l.chars().nth(char0 as usize))
        .is_some_and(|c| c.is_alphanumeric() || c == '_');
    let symbol = if construct.is_some() && !on_word {
        None
    } else {
        loft::lsp::resolve_at(text, stdlib_dir, line0 + 1, char0 + 1)
            .or_else(|| loft::lsp::symbol_at(text, "buf.loft", stdlib_dir, line0 + 1, char0 + 1))
    };
    let body = match (symbol, construct) {
        (Some(h), Some(c)) => format!("{}\n\n---\n\n{c}", hover_markdown(&h)),
        // @PLN183 P4 — a library's item adds the library's card beneath its own.
        (Some(h), None) => match library_of(&h) {
            Some(card) => format!("{}\n\n---\n\n{card}", hover_markdown(&h)),
            None => hover_markdown(&h),
        },
        (None, Some(c)) => c,
        (None, None) => return None,
    };
    Some(obj(vec![("contents", markup(&body))]))
}

/// The card of the library a resolved definition comes from, when it lives in an installed
/// copy (`…/registry/<name>-<version>/…`).
fn library_of(h: &loft::lsp::Hover) -> Option<String> {
    #[cfg(feature = "registry")]
    {
        let dir = Path::new(&h.def_file)
            .ancestors()
            .find(|a| a.parent().is_some_and(|p| p.ends_with("registry")))?;
        loft::doc_site::library_card(dir)
    }
    #[cfg(not(feature = "registry"))]
    {
        let _ = h;
        None
    }
}

/// On `use <library>` (the cursor on the name), the card of the installed copy this project
/// resolves.
fn use_line_card(text: &str, line0: u32, char0: u32, file: &str) -> Option<String> {
    let line = text.lines().nth(line0 as usize)?;
    let rest = line.trim_start().strip_prefix("use ")?;
    let name: String = rest
        .trim_start()
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    let at = line.find(&name)?;
    if (char0 as usize) < at || char0 as usize >= at + name.len() {
        return None;
    }
    #[cfg(feature = "registry")]
    {
        loft::doc_site::library_card(&loft::doc_site::installed_library(
            &name,
            Some(Path::new(file)),
        )?)
    }
    #[cfg(not(feature = "registry"))]
    {
        let _ = file;
        None
    }
}

/// The hover body: the signature in a `loft` code fence, then the `///` doc.
fn hover_markdown(h: &loft::lsp::Hover) -> String {
    // @PLN183 — the one renderer's Markdown back-end, so a hover says what the web page and the
    // REPL say (a worked-example citation is bookkeeping, not prose).
    let doc = h.doc.join("\n");
    loft::doc_render::item_markdown(&loft::doc_render::Item {
        sig: &h.signature,
        doc: &doc,
    })
    .trim_end()
    .to_string()
}

/// An LSP `MarkupContent` (markdown).
fn markup(value: &str) -> Parsed {
    obj(vec![
        ("kind", Parsed::Str("markdown".into())),
        ("value", Parsed::Str(value.into())),
    ])
}

/// An LSP `TextDocumentPositionParams`: `params.textDocument.uri` +
/// `params.position.{line, character}` (0-based).  Shared by hover + definition.
fn text_document_position(msg: &Parsed) -> Option<(String, u32, u32)> {
    let params = obj_get(msg, "params")?;
    let uri = obj_str(obj_get(params, "textDocument")?, "uri")?;
    let pos = obj_get(params, "position")?;
    let line = u32::try_from(obj_get(pos, "line")?.as_i64()?).ok()?;
    let ch = u32::try_from(obj_get(pos, "character")?.as_i64()?).ok()?;
    Some((uri, line, ch))
}

// ── go-to-definition (S6) ────────────────────────────────────────────────────
/// An LSP `Location` for the resolved definition.  A LOCAL def lives in the open
/// document, so its uri is the request's; a stdlib / library def lives in a file
/// on disk, so its uri is a `file://` path there.  The range underlines the name.
fn definition_location(request_uri: &str, h: &loft::lsp::Hover, stdlib_dir: &str) -> Parsed {
    let uri = if h.def_file == "buf.loft" {
        request_uri.to_string()
    } else {
        stdlib_file_uri(stdlib_dir, &h.def_file)
    };
    let line0 = h.def_line.saturating_sub(1);
    let col0 = h.def_col.saturating_sub(1);
    let name_len = h.name.rsplit('.').next().unwrap_or(&h.name).chars().count() as u32;
    obj(vec![
        ("uri", Parsed::Str(uri)),
        ("range", range_of(line0, col0, col0 + name_len)),
    ])
}

/// `file://` URI for a stdlib/library source file, `pos.file` being repo-root
/// relative and the stdlib root the parent of `stdlib_dir` (`…/default`).
fn stdlib_file_uri(stdlib_dir: &str, rel_file: &str) -> String {
    let path = Path::new(stdlib_dir)
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join(rel_file);
    let abs = loft::file_access::plain_canonical(&path);
    file_uri(&abs)
}

/// `file://` URI for an absolute path, in the shape editors actually accept.
///
/// Windows needs three things the naive `format!("file://{}", p.display())` gets
/// wrong: the extended-length `\\?\` prefix stripped (see [`loft::lsp::plain_path`]),
/// separators as `/`, and a THIRD slash before the drive letter — `file:///D:/a.loft`,
/// not `file://D:\a.loft`.  A POSIX path already starts with `/`, so it takes the
/// same branch and is unchanged.
fn file_uri(abs: &Path) -> String {
    loft::lsp::path_to_uri(abs)
}

// ── tracker-tag integration (T1 hover / T2 documentLink) ─────────────────────
/// The workspace root path from `initialize` params — `rootUri` (a `file://`
/// uri) or the legacy `rootPath`.  Used to locate `<root>/index/`.
fn initialize_root(msg: &Parsed) -> Option<String> {
    let params = obj_get(msg, "params")?;
    if let Some(uri) = obj_str(params, "rootUri") {
        return Some(uri_to_path(&uri));
    }
    obj_str(params, "rootPath")
}

// ── find-references (workspace reverse index) ────────────────────────────────
/// Build the workspace reverse index once (from `workspace_root`), caching it.
fn ensure_workspace_index(
    root: &Option<String>,
    index: &mut Option<loft::lsp::WorkspaceIndex>,
    tried: &mut bool,
) {
    if *tried {
        return;
    }
    *tried = true;
    if let Some(r) = root {
        *index = Some(loft::lsp::WorkspaceIndex::build(r));
    }
}

/// LSP `Location[]` for every reference to `name`, with open buffers overlaid so
/// unsaved edits count, then narrowed by `scope` (a local → its function only).
/// Empty when there is no workspace index.
fn references_of(
    name: &str,
    index: Option<&loft::lsp::WorkspaceIndex>,
    documents: &HashMap<String, String>,
    scope: &loft::lsp::RefScope,
    current_file: &str,
) -> Vec<Parsed> {
    let Some(wi) = index else {
        return Vec::new();
    };
    let overlays: Vec<(String, String)> = documents
        .iter()
        .map(|(uri, text)| (uri_to_path(uri), text.clone()))
        .collect();
    let len = name.chars().count() as u32;
    let all = wi.references_overlaid(name, &overlays);
    loft::lsp::scoped_refs(all, scope, current_file)
        .iter()
        .map(|r| location_of(r, len))
        .collect()
}

/// An LSP `Location` JSON object for a reference `r`, highlighting `len` chars.
fn location_of(r: &loft::lsp::Reference, len: u32) -> Parsed {
    let line0 = r.line.saturating_sub(1);
    let col0 = r.col.saturating_sub(1);
    obj(vec![
        ("uri", Parsed::Str(file_uri(Path::new(&r.file)))),
        ("range", range_of(line0, col0, col0 + len)),
    ])
}

/// A `file://` document uri → its filesystem path (platform-agnostic; see
/// [`loft::lsp::uri_to_path`]).
fn uri_to_path(uri: &str) -> String {
    loft::lsp::uri_to_path(uri)
}

/// The open documents as `(path, content)` overlays — the unsaved editor view that
/// replaces the on-disk copy during a workspace query.
fn open_overlays(documents: &HashMap<String, String>) -> Vec<(String, String)> {
    documents
        .iter()
        .map(|(uri, text)| (uri_to_path(uri), text.clone()))
        .collect()
}

/// Plan a rename and return an LSP `WorkspaceEdit`, or an error message (the
/// server sends it as a JSON-RPC error, which the editor shows to the user).
fn rename_edit(
    msg: &Parsed,
    documents: &HashMap<String, String>,
    index: Option<&loft::lsp::WorkspaceIndex>,
    stdlib_dir: &str,
) -> Result<Parsed, String> {
    let (uri, line, ch) = text_document_position(msg).ok_or("bad rename params")?;
    let new_name = obj_get(msg, "params")
        .and_then(|p| obj_str(p, "newName"))
        .ok_or("missing newName")?;
    let text = documents.get(&uri).ok_or("document is not open")?;
    let old = loft::lsp::identifier_at(text, line + 1, ch + 1)
        .ok_or("the cursor is not on an identifier")?;
    // S4 (@PLN115): an assignment-local renames its EXACT binding occurrences (a
    // same-named field / method / global excluded) — no workspace index needed, as
    // a local lives in this buffer alone.  Params / loop binders / globals fall
    // through to the workspace name-based plan below.
    let file = uri_to_path(&uri);
    if let Some(refs) = loft::lsp::local_binding_refs(text, stdlib_dir, &file, line + 1, ch + 1) {
        if !loft::lsp::is_valid_identifier(&new_name) {
            return Err(format!("`{new_name}` is not a valid identifier"));
        }
        if new_name == old {
            return Ok(Parsed::Null);
        }
        return Ok(build_workspace_edit(
            &refs,
            old.chars().count() as u32,
            &new_name,
        ));
    }
    let wi = index.ok_or("no workspace to rename across")?;
    let overlays: Vec<(String, String)> = documents
        .iter()
        .map(|(u, t)| (uri_to_path(u), t.clone()))
        .collect();
    let refs = loft::lsp::plan_rename(&old, &new_name, wi, &overlays, stdlib_dir)?;
    // F: a LOCAL rename touches only its own function; a global stays workspace-wide.
    let scope = loft::lsp::reference_scope(text, &old, stdlib_dir, line + 1);
    let refs = loft::lsp::scoped_refs(refs, &scope, &file);
    if refs.is_empty() {
        return Err(format!("no references to `{old}` in scope"));
    }
    Ok(build_workspace_edit(
        &refs,
        old.chars().count() as u32,
        &new_name,
    ))
}

/// An LSP `WorkspaceEdit` — a `{uri: [TextEdit]}` map — replacing each reference's
/// old-name range with `new`.
fn build_workspace_edit(refs: &[loft::lsp::Reference], old_len: u32, new: &str) -> Parsed {
    let mut by_uri: Vec<(String, Vec<Parsed>)> = Vec::new();
    for r in refs {
        let uri = file_uri(Path::new(&r.file));
        let col0 = r.col.saturating_sub(1);
        let edit = obj(vec![
            (
                "range",
                range_of(r.line.saturating_sub(1), col0, col0 + old_len),
            ),
            ("newText", Parsed::Str(new.to_string())),
        ]);
        if let Some((_, edits)) = by_uri.iter_mut().find(|(u, _)| u == &uri) {
            edits.push(edit);
        } else {
            by_uri.push((uri, vec![edit]));
        }
    }
    let changes = Parsed::Object(
        by_uri
            .into_iter()
            .map(|(u, e)| (u, 0, Parsed::Array(e)))
            .collect(),
    );
    obj(vec![("changes", changes)])
}

/// Ensure the cached tracker index (from `<root>/index/`) is current: (re)load it
/// whenever `index/tags.json`'s mtime differs from `mtime` — the first request, and
/// again after a mid-session `make index` regenerates the index.  A cheap `stat`
/// per call; the parse only reruns on a real change.  `mtime` = None means never
/// loaded OR the file is absent (outside the loft repo), in which case the index
/// stays `None` and no work repeats until the file appears.
fn ensure_tag_index(
    root: &Option<String>,
    index: &mut Option<loft::lsp::TagIndex>,
    mtime: &mut Option<std::time::SystemTime>,
) {
    let Some(r) = root else { return };
    let current = std::fs::metadata(format!("{r}/index/tags.json"))
        .and_then(|m| m.modified())
        .ok();
    if current == *mtime {
        return; // unchanged (including still-absent) — keep the cache
    }
    *mtime = current;
    *index = current
        .is_some()
        .then(|| loft::lsp::TagIndex::load(&format!("{r}/index")))
        .flatten();
}

/// T3 — Warnings for buffer tags the scanner flagged as broken (a `@P`/`@PLAN`
/// reference to no valid issue/plan, per the index's `broken` array).  Empty when
/// the index is absent.  Folded into the diagnostics push on open/change.
fn tag_diagnostics(text: &str, index: Option<&loft::lsp::TagIndex>) -> Vec<Parsed> {
    let Some(idx) = index else {
        return Vec::new();
    };
    loft::lsp::tags_in(text)
        .into_iter()
        .filter(|(tag, ..)| idx.is_broken(tag))
        .map(|(tag, line, s, e)| {
            obj(vec![
                ("range", range_of(line - 1, s, e)),
                ("severity", Parsed::Int(2)), // Warning
                ("source", Parsed::Str("loft-tag".into())),
                (
                    "message",
                    Parsed::Str(format!(
                        "broken tracker reference {tag} — no matching issue/plan (per the index)"
                    )),
                ),
            ])
        })
        .collect()
}

/// Every tracker tag with an issue URL, as an LSP `DocumentLink`.  Empty when the
/// index is absent (outside the loft repo) — tags then carry no links.
fn document_links(text: &str, index: Option<&loft::lsp::TagIndex>) -> Vec<Parsed> {
    let Some(idx) = index else {
        return Vec::new();
    };
    loft::lsp::tags_in(text)
        .into_iter()
        .filter_map(|(tag, line, s, e)| {
            let url = idx.lookup(&tag)?.url?;
            Some(obj(vec![
                ("range", range_of(line - 1, s, e)),
                ("target", Parsed::Str(url)),
            ]))
        })
        .collect()
}

// ── document-lifecycle param extraction ──────────────────────────────────────
/// `didOpen`: `params.textDocument.{uri, text}`.
fn did_open_params(msg: &Parsed) -> Option<(String, String)> {
    let doc = obj_get(obj_get(msg, "params")?, "textDocument")?;
    Some((obj_str(doc, "uri")?, obj_str(doc, "text")?))
}

/// `didChange` under full-document sync: `params.textDocument.uri` +
/// `params.contentChanges[last].text` (full sync sends whole-file replacements;
/// take the last change, which carries the final text).
fn did_change_params(msg: &Parsed) -> Option<(String, String)> {
    let params = obj_get(msg, "params")?;
    let uri = obj_str(obj_get(params, "textDocument")?, "uri")?;
    let changes = match obj_get(params, "contentChanges")? {
        Parsed::Array(v) => v,
        _ => return None,
    };
    let text = obj_str(changes.last()?, "text")?;
    Some((uri, text))
}

/// `didClose`: `params.textDocument.uri`.
fn did_close_uri(msg: &Parsed) -> Option<String> {
    obj_str(obj_get(obj_get(msg, "params")?, "textDocument")?, "uri")
}

// ── stdlib resolution (deployment) ───────────────────────────────────────────
/// Locate the stdlib `default/` directory the way the `loft` CLI does: relative
/// to this binary for a release/installed layout, else the source tree.  Checked
/// most-specific first; falls back to a CWD-relative `default` (repo-root runs).
fn resolve_stdlib_dir() -> String {
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .unwrap_or_default();
    let candidates = [
        exe_dir.join("../../default"), // dev: target/{release,debug}/loft-lsp -> repo/default
        exe_dir.join("../share/loft/default"), // installed: <prefix>/bin -> <prefix>/share/loft
        exe_dir.join("../default"),    // release layout with default beside the binary dir
    ];
    for c in candidates {
        if c.is_dir() {
            return c.to_string_lossy().into_owned();
        }
    }
    "default".to_string()
}

// ── framing ─────────────────────────────────────────────────────────────────
// Read one `Content-Length: N\r\n <headers> \r\n<N bytes>` message; the JSON
// body, or `None` at EOF.
fn read_message(stdin: &mut impl BufRead) -> Option<String> {
    let mut content_length: Option<usize> = None;
    loop {
        let mut line = String::new();
        if stdin.read_line(&mut line).ok()? == 0 {
            return None; // EOF
        }
        let header = line.trim_end_matches(['\r', '\n']);
        if header.is_empty() {
            break; // blank line ends the header block
        }
        if let Some(v) = header.strip_prefix("Content-Length:") {
            content_length = v.trim().parse().ok();
        }
        // Content-Type and any other header is ignored.
    }
    let mut buf = vec![0u8; content_length?];
    stdin.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

fn send(stdout: &io::Stdout, msg: &Parsed) {
    let body = json::to_json_string(msg);
    let mut out = stdout.lock();
    // Content-Length is the BYTE length of the UTF-8 body.
    let _ = write!(out, "Content-Length: {}\r\n\r\n{body}", body.len());
    let _ = out.flush();
}

// ── json helpers over loft::json::Parsed ─────────────────────────────────────
fn obj_get<'a>(v: &'a Parsed, key: &str) -> Option<&'a Parsed> {
    match v {
        Parsed::Object(entries) => entries
            .iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, _, val)| val),
        _ => None,
    }
}

fn obj_str(v: &Parsed, key: &str) -> Option<String> {
    match obj_get(v, key) {
        Some(Parsed::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

/// Build a JSON object from `(key, value)` pairs.  The `usize` in each entry is
/// the source byte offset `loft::json` records for diagnostics; on the emit path
/// it is unused, so `0` is fine.
fn obj(entries: Vec<(&str, Parsed)>) -> Parsed {
    Parsed::Object(
        entries
            .into_iter()
            .map(|(k, v)| (k.to_string(), 0, v))
            .collect(),
    )
}

// ── message builders ─────────────────────────────────────────────────────────
fn response(id: Parsed, result: Parsed) -> Parsed {
    obj(vec![
        ("jsonrpc", Parsed::Str("2.0".into())),
        ("id", id),
        ("result", result),
    ])
}

fn error_response(id: Parsed, code: i64, message: &str) -> Parsed {
    let err = obj(vec![
        ("code", Parsed::Int(code)),
        ("message", Parsed::Str(message.into())),
    ]);
    obj(vec![
        ("jsonrpc", Parsed::Str("2.0".into())),
        ("id", id),
        ("error", err),
    ])
}

/// A JSON-RPC notification (no `id` — the server pushes it, the client never
/// replies).  `publishDiagnostics` is the S3 use.
fn notification(method: &str, params: Parsed) -> Parsed {
    obj(vec![
        ("jsonrpc", Parsed::Str("2.0".into())),
        ("method", Parsed::Str(method.into())),
        ("params", params),
    ])
}

fn initialize_result() -> Parsed {
    // `textDocumentSync` as an object: `openClose` so the client sends
    // didOpen/didClose, `change: 1` for full-document sync on each edit — the S3
    // diagnostics contract — and `save` so the client notifies on save (the signal
    // to invalidate the workspace reverse index).  Later providers (hover,
    // definition, …) add their own capability flags as they land.
    let sync = obj(vec![
        ("openClose", Parsed::Bool(true)),
        ("change", Parsed::Int(1)),
        ("save", Parsed::Bool(true)),
    ]);
    let capabilities = obj(vec![
        ("textDocumentSync", sync),
        ("documentSymbolProvider", Parsed::Bool(true)),
        ("hoverProvider", Parsed::Bool(true)),
        ("definitionProvider", Parsed::Bool(true)),
        ("referencesProvider", Parsed::Bool(true)),
        (
            // Advertise the kinds the server produces: quick-fixes (step B) and the
            // extract-function refactoring (E0+, EXTRACT.md).
            "codeActionProvider",
            obj(vec![(
                "codeActionKinds",
                Parsed::Array(vec![
                    Parsed::Str("quickfix".into()),
                    Parsed::Str("refactor.extract".into()),
                    // @PLN131 — "fix all" over the MECHANICAL, VERIFIED fixes. Editors bind
                    // this to fix-on-save, so it runs unattended: exactly the lane a
                    // conditional fix is barred from.
                    Parsed::Str("source.fixAll".into()),
                    // @PLN183 P3 — the overview ("loft: what can I use here?").
                    Parsed::Str(OVERVIEW_KIND.into()),
                ]),
            )]),
        ),
        (
            "executeCommandProvider",
            obj(vec![(
                "commands",
                Parsed::Array(vec![Parsed::Str(OVERVIEW_COMMAND.into())]),
            )]),
        ),
        ("inlayHintProvider", Parsed::Bool(true)),
        (
            "completionProvider",
            obj(vec![
                (
                    // `.` → member completion; `@` → tracker-tag completion (T4).
                    "triggerCharacters",
                    Parsed::Array(vec![Parsed::Str(".".into()), Parsed::Str("@".into())]),
                ),
                // @PLN183 P4 — an item's documentation is asked for when it is shown.
                ("resolveProvider", Parsed::Bool(true)),
            ]),
        ),
        (
            "semanticTokensProvider",
            obj(vec![
                (
                    "legend",
                    obj(vec![
                        (
                            "tokenTypes",
                            Parsed::Array(
                                loft::lsp::semantic_token_types()
                                    .iter()
                                    .map(|t| Parsed::Str((*t).into()))
                                    .collect(),
                            ),
                        ),
                        ("tokenModifiers", Parsed::Array(Vec::new())),
                    ]),
                ),
                ("full", Parsed::Bool(true)),
            ]),
        ),
        (
            "renameProvider",
            obj(vec![("prepareProvider", Parsed::Bool(true))]),
        ),
        ("documentFormattingProvider", Parsed::Bool(true)),
        (
            "documentLinkProvider",
            obj(vec![("resolveProvider", Parsed::Bool(false))]),
        ),
    ]);
    let server_info = obj(vec![
        ("name", Parsed::Str(SERVER_NAME.into())),
        ("version", Parsed::Str(SERVER_VERSION.into())),
    ]);
    obj(vec![
        ("capabilities", capabilities),
        ("serverInfo", server_info),
    ])
}
