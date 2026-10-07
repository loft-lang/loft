// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 S0 — protocol test harness for `loft-lsp`.
//
// Drives the loft-lsp binary over stdio with `Content-Length`-framed JSON-RPC
// and asserts its replies, so the transport (S1) and every later feature step is
// CI-tested WITHOUT a live editor.  The harness carries its own positive control:
// `unknown_request_is_an_error_not_a_result` proves it can distinguish a FAILURE
// reply (a JSON-RPC error) from a success — so a green handshake test is
// meaningful, not vacuous.

use loft::file_access as fa;
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use loft::json::{self, Parsed};

/// A live loft-lsp subprocess with framed read/write over its stdio.
struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Session {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_loft-lsp"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn loft-lsp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Session {
            child,
            stdin,
            stdout,
        }
    }

    fn send_raw(&mut self, body: &str) {
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, id: i64, method: &str, params: &str) {
        self.send_raw(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#
        ));
    }

    fn notify(&mut self, method: &str, params: &str) {
        self.send_raw(&format!(
            r#"{{"jsonrpc":"2.0","method":"{method}","params":{params}}}"#
        ));
    }

    /// Read one framed reply and parse it.
    fn recv(&mut self) -> Parsed {
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).unwrap();
            assert!(n > 0, "server closed stdout before replying");
            let header = line.trim_end_matches(['\r', '\n']);
            if header.is_empty() {
                break;
            }
            if let Some(v) = header.strip_prefix("Content-Length:") {
                content_length = v.trim().parse().unwrap();
            }
        }
        let mut buf = vec![0u8; content_length];
        self.stdout.read_exact(&mut buf).unwrap();
        json::parse(&String::from_utf8(buf).unwrap()).expect("reply is valid JSON")
    }
}

fn field<'a>(v: &'a Parsed, key: &str) -> Option<&'a Parsed> {
    match v {
        Parsed::Object(e) => e.iter().find(|(k, _, _)| k == key).map(|(_, _, val)| val),
        _ => None,
    }
}

fn field_str(v: &Parsed, key: &str) -> Option<String> {
    match field(v, key) {
        Some(Parsed::Str(s)) => Some(s.clone()),
        _ => None,
    }
}

fn field_arr<'a>(v: &'a Parsed, key: &str) -> Option<&'a Vec<Parsed>> {
    match field(v, key) {
        Some(Parsed::Array(a)) => Some(a),
        _ => None,
    }
}

/// Build a `didOpen`/`didChange`-shaped params string, with `program` correctly
/// JSON-escaped via loft's own serializer (so embedded `\n`/`"` survive).
fn open_params(uri: &str, program: &str) -> String {
    let text = json::to_json_string(&Parsed::Str(program.to_string()));
    format!(r#"{{"textDocument":{{"uri":"{uri}","languageId":"loft","version":1,"text":{text}}}}}"#)
}

fn change_params(uri: &str, program: &str) -> String {
    let text = json::to_json_string(&Parsed::Str(program.to_string()));
    format!(
        r#"{{"textDocument":{{"uri":"{uri}","version":2}},"contentChanges":[{{"text":{text}}}]}}"#
    )
}

// S3 — the diagnostics push path.  Open a buffer with an error, assert the
// server pushes `publishDiagnostics` with the fault at the right RANGE; then
// edit it clean and assert the squiggle clears (an empty diagnostics list).
#[test]
fn diagnostics_publish_on_open_then_clear_on_fix() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let _ = s.recv();
    s.notify("initialized", "{}");

    let uri = "file:///buf.loft";
    // `nope` is undefined; it sits on line 2 (1-based), col 3 — so LSP 0-based
    // range start is line 1, character 2, and the token is 4 chars wide.
    s.notify(
        "textDocument/didOpen",
        &open_params(uri, "fn main() {\n  nope(3)\n}\n"),
    );
    let note = s.recv();
    assert_eq!(
        field_str(&note, "method").as_deref(),
        Some("textDocument/publishDiagnostics"),
        "server pushes a publishDiagnostics notification on open"
    );
    let params = field(&note, "params").expect("notification carries params");
    assert_eq!(
        field_str(params, "uri").as_deref(),
        Some(uri),
        "echoes the document uri"
    );
    let diags = field_arr(params, "diagnostics").expect("diagnostics is an array");
    assert_eq!(diags.len(), 1, "exactly one error, got {diags:?}");

    let d = &diags[0];
    assert_eq!(
        field(d, "severity").and_then(Parsed::as_i64),
        Some(1),
        "an Error maps to LSP severity 1"
    );
    assert!(
        field_str(d, "message").unwrap_or_default().contains("nope"),
        "message names the offending symbol: {d:?}"
    );
    let start = field(field(d, "range").unwrap(), "start").unwrap();
    assert_eq!(
        (
            field(start, "line").and_then(Parsed::as_i64),
            field(start, "character").and_then(Parsed::as_i64)
        ),
        (Some(1), Some(2)),
        "0-based range start sits on `nope` (line 1, char 2): {d:?}"
    );
    let end = field(field(d, "range").unwrap(), "end").unwrap();
    assert_eq!(
        field(end, "character").and_then(Parsed::as_i64),
        Some(6),
        "range underlines the whole 4-char token `nope`: {d:?}"
    );

    // Edit the buffer to a valid program — the squiggle must clear.
    s.notify(
        "textDocument/didChange",
        &change_params(uri, "fn main() {\n  print(\"hi\")\n}\n"),
    );
    let cleared = s.recv();
    let params = field(&cleared, "params").expect("clear carries params");
    assert!(
        field_arr(params, "diagnostics").is_some_and(Vec::is_empty),
        "a valid buffer publishes an empty diagnostics list (clears squiggles): {cleared:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// S4 — the outline path.  Open a buffer, request `documentSymbol`, assert the
// server replies with the top-level defs (kind + name in source order) and a
// selectionRange that lands on the NAME.
#[test]
fn document_symbol_lists_the_outline() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    assert!(
        matches!(
            field(caps, "documentSymbolProvider"),
            Some(Parsed::Bool(true))
        ),
        "advertises documentSymbolProvider: {init:?}"
    );
    s.notify("initialized", "{}");

    let uri = "file:///o.loft";
    let prog = "struct Point {\n  x: integer,\n}\nfn main() {\n  print(\"hi\")\n}\n";
    s.notify("textDocument/didOpen", &open_params(uri, prog));
    let _ = s.recv(); // consume the publishDiagnostics push from didOpen

    s.request(
        2,
        "textDocument/documentSymbol",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}}}}"#),
    );
    let reply = s.recv();
    assert_eq!(field(&reply, "id").and_then(Parsed::as_i64), Some(2));
    let syms = match field(&reply, "result") {
        Some(Parsed::Array(a)) => a,
        other => panic!("documentSymbol result is an array, got {other:?}"),
    };
    let got: Vec<(i64, String)> = syms
        .iter()
        .map(|x| {
            (
                field(x, "kind").and_then(Parsed::as_i64).unwrap(),
                field_str(x, "name").unwrap(),
            )
        })
        .collect();
    // SymbolKind: Struct 23, Function 12 — in source order.
    assert_eq!(
        got,
        vec![(23, "Point".to_string()), (12, "main".to_string())],
        "outline lists `struct Point` then `fn main`: {got:?}"
    );

    // The struct's selectionRange underlines the NAME `Point` (line 0, chars 7..12),
    // not the parser's body-start position.
    let sel = field(&syms[0], "selectionRange").unwrap();
    let start = field(sel, "start").unwrap();
    let end = field(sel, "end").unwrap();
    assert_eq!(
        (
            field(start, "line").and_then(Parsed::as_i64),
            field(start, "character").and_then(Parsed::as_i64),
            field(end, "character").and_then(Parsed::as_i64),
        ),
        (Some(0), Some(7), Some(12)),
        "selectionRange covers `Point`: {:?}",
        syms[0]
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// S5 — the hover path.  Open a buffer, hover a call site, assert the reply
// carries the resolved definition's signature + `///` doc as markdown; hover a
// blank spot and assert a null result.
#[test]
fn hover_shows_signature_and_doc() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    assert!(
        matches!(field(caps, "hoverProvider"), Some(Parsed::Bool(true))),
        "advertises hoverProvider: {init:?}"
    );
    s.notify("initialized", "{}");

    let uri = "file:///h.loft";
    let prog = "/// Area of a rectangle.\nfn area(w: integer, h: integer) -> integer {\n  w * h\n}\nfn main() {\n  print(area(2, 3))\n}\n";
    s.notify("textDocument/didOpen", &open_params(uri, prog));
    let _ = s.recv(); // consume the publishDiagnostics push

    // Hover `area` inside the call on 0-based line 5, char 8 — resolves to the def.
    s.request(
        2,
        "textDocument/hover",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":5,"character":8}}}}"#),
    );
    let reply = s.recv();
    let contents = field(
        field(&reply, "result").expect("hover returns a result"),
        "contents",
    )
    .expect("hover has MarkupContent");
    assert_eq!(field_str(contents, "kind").as_deref(), Some("markdown"));
    let value = field_str(contents, "value").unwrap_or_default();
    assert!(
        value.contains("fn area(w: integer, h: integer) -> integer"),
        "shows the signature: {value}"
    );
    assert!(
        value.contains("Area of a rectangle."),
        "shows the /// doc: {value}"
    );

    // Hover a blank spot (line 2, char 0 — indentation) → a null hover result.
    s.request(
        3,
        "textDocument/hover",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":2,"character":0}}}}"#),
    );
    let none = s.recv();
    assert!(
        matches!(field(&none, "result"), Some(Parsed::Null)),
        "no symbol under the cursor → null: {none:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// S6 — go-to-definition.  A LOCAL symbol jumps within the open document to its
// name; a STDLIB symbol jumps into the stdlib source file (a `file://` uri); a
// blank spot yields null.
#[test]
fn go_to_definition_jumps_to_local_and_stdlib_defs() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    assert!(
        matches!(field(caps, "definitionProvider"), Some(Parsed::Bool(true))),
        "advertises definitionProvider: {init:?}"
    );
    s.notify("initialized", "{}");

    let uri = "file:///d.loft";
    let prog = "fn area(w: integer, h: integer) -> integer {\n  w * h\n}\nfn main() {\n  print(area(2, 3))\n}\n";
    s.notify("textDocument/didOpen", &open_params(uri, prog));
    let _ = s.recv(); // consume the publishDiagnostics push

    // `area` in the call (0-based line 4, char 8) → the local definition, at its
    // NAME (line 0, chars 3..7), in the SAME document.
    s.request(
        2,
        "textDocument/definition",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":4,"character":8}}}}"#),
    );
    let loc = field(&s.recv(), "result")
        .cloned()
        .expect("definition returns a Location");
    assert_eq!(
        field_str(&loc, "uri").as_deref(),
        Some(uri),
        "a local def stays in the open document: {loc:?}"
    );
    let start = field(field(&loc, "range").unwrap(), "start").unwrap();
    assert_eq!(
        (
            field(start, "line").and_then(Parsed::as_i64),
            field(start, "character").and_then(Parsed::as_i64),
        ),
        (Some(0), Some(3)),
        "jumps to `area` (line 0, char 3): {loc:?}"
    );

    // `print` (0-based line 4, char 4) → the stdlib definition in a `file://`
    // source under `default/`.
    s.request(
        3,
        "textDocument/definition",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":4,"character":4}}}}"#),
    );
    let loc = field(&s.recv(), "result")
        .cloned()
        .expect("stdlib Location");
    let target = field_str(&loc, "uri").unwrap_or_default();
    assert!(
        target.starts_with("file://") && target.contains("default/"),
        "a stdlib def jumps into its source file: {target}"
    );

    // A blank spot → null.
    s.request(
        4,
        "textDocument/definition",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":1,"character":0}}}}"#),
    );
    let none = s.recv();
    assert!(
        matches!(field(&none, "result"), Some(Parsed::Null)),
        "no symbol under the cursor → null: {none:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// Formatting — a full-document `TextEdit` from the same formatter the `loft fmt`
// CLI uses; an already-tidy buffer yields no edits.
#[test]
fn formatting_returns_a_whole_document_edit_and_noops_when_tidy() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    assert!(
        matches!(
            field(caps, "documentFormattingProvider"),
            Some(Parsed::Bool(true))
        ),
        "advertises documentFormattingProvider: {init:?}"
    );
    s.notify("initialized", "{}");

    let uri = "file:///f.loft";
    let opts = r#"{"tabSize":2,"insertSpaces":true}"#;
    s.notify(
        "textDocument/didOpen",
        &open_params(uri, "fn main(){\n  x=1+2\n}\n"),
    );
    let _ = s.recv(); // consume the publishDiagnostics push

    // An unformatted buffer → exactly one whole-document edit with tidy text.
    s.request(
        2,
        "textDocument/formatting",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"options":{opts}}}"#),
    );
    let reply = s.recv();
    let edits = match field(&reply, "result") {
        Some(Parsed::Array(a)) => a,
        other => panic!("formatting result is an array, got {other:?}"),
    };
    assert_eq!(edits.len(), 1, "one whole-document edit: {edits:?}");
    let e = &edits[0];
    let new_text = field_str(e, "newText").unwrap_or_default();
    assert!(
        new_text.contains("fn main() {"),
        "tidies spacing: {new_text}"
    );
    assert!(
        new_text.contains("x = 1 + 2"),
        "tidies operators: {new_text}"
    );
    let start = field(field(e, "range").unwrap(), "start").unwrap();
    assert_eq!(
        (
            field(start, "line").and_then(Parsed::as_i64),
            field(start, "character").and_then(Parsed::as_i64),
        ),
        (Some(0), Some(0)),
        "the edit range starts at the document start: {e:?}"
    );

    // Replace the buffer with the ALREADY-tidy text → no edits.
    s.notify(
        "textDocument/didChange",
        &change_params(uri, "fn main() {\n  x = 1 + 2\n}\n"),
    );
    let _ = s.recv(); // diagnostics push
    s.request(
        3,
        "textDocument/formatting",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"options":{opts}}}"#),
    );
    let tidy = s.recv();
    assert!(
        matches!(field(&tidy, "result"), Some(Parsed::Array(a)) if a.is_empty()),
        "an already-tidy buffer yields no edits: {tidy:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// T1/T2 — the tracker-tag integration.  A synthetic workspace with an index;
// hover over a tag shows the feature, and the tag becomes a document link.
#[test]
fn tag_hover_and_document_link() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("tagws");
    let idx = root.join("index");
    fa::create_dir_all(&idx).unwrap();
    fa::write(
        idx.join("tags.json"),
        r#"{"@F1":[{"file":"a.md","line":1,"context":"@F1"}]}"#,
    )
    .unwrap();
    fa::write(
        idx.join("features.json"),
        r#"[{"number":1,"title":"Keyed collections","kind":"feature","body":"Look up records by key.\n"}]"#,
    )
    .unwrap();

    let mut s = Session::start();
    let root_uri = loft::lsp::path_to_uri(&root);
    s.request(1, "initialize", &format!(r#"{{"rootUri":"{root_uri}"}}"#));
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    assert!(
        field(caps, "documentLinkProvider").is_some(),
        "advertises documentLinkProvider: {init:?}"
    );
    s.notify("initialized", "{}");

    let uri = "file:///t.loft";
    // `// @F1 tracked` — `@F1` at chars 3..6.
    let prog = "// @F1 tracked\nfn main() {\n  print(\"hi\")\n}\n";
    s.notify("textDocument/didOpen", &open_params(uri, prog));
    let _ = s.recv(); // consume the publishDiagnostics push

    // Hover on `@F1` (0-based line 0, char 4) → the feature, not a symbol.
    s.request(
        2,
        "textDocument/hover",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":0,"character":4}}}}"#),
    );
    let h = s.recv();
    let contents = field(field(&h, "result").expect("hover result"), "contents").unwrap();
    let val = field_str(contents, "value").unwrap_or_default();
    assert!(
        val.contains("@F1") && val.contains("Keyed collections"),
        "tag hover shows the feature: {val}"
    );

    // documentLink → one link for `@F1` at its range, targeting the issue.
    s.request(
        3,
        "textDocument/documentLink",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}}}}"#),
    );
    let l = s.recv();
    let links = match field(&l, "result") {
        Some(Parsed::Array(a)) => a,
        other => panic!("documentLink result is an array, got {other:?}"),
    };
    assert_eq!(links.len(), 1, "one tag link: {links:?}");
    assert_eq!(
        field_str(&links[0], "target").as_deref(),
        Some("https://github.com/loft-lang/features/issues/1"),
        "links to the features issue: {links:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// T3 — broken-tag diagnostics.  A synthetic index marks `@P99` broken; opening a <!--noindex-->
// buffer that references it publishes a Warning at the tag; a NON-broken tag does
// not.
#[test]
fn broken_tag_publishes_a_warning() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("t3ws");
    let idx = root.join("index");
    fa::create_dir_all(&idx).unwrap();
    fa::write(
        idx.join("tags.json"),
        r#"{"@P99":[{"file":"z","line":1,"context":"@P99 <!--noindex-->"}],"broken":[{"tag":"@P99","refs":["z:1"]}]}"#,
    )
    .unwrap();
    fa::write(idx.join("features.json"), "[]").unwrap();

    let mut s = Session::start();
    s.request(
        1,
        "initialize",
        &format!(r#"{{"rootUri":"{}"}}"#, loft::lsp::path_to_uri(&root)),
    );
    let _ = s.recv();
    s.notify("initialized", "{}");

    let uri = "file:///a.loft";
    // `@P99` is broken; `@P42` is unknown to the index → NOT flagged (no false positive). <!--noindex-->
    let prog = "// tracks @P99 and @P42\nfn main() {\n  print(\"hi\")\n}\n"; // <!--noindex-->

    s.notify("textDocument/didOpen", &open_params(uri, prog));
    let note = s.recv();
    let diags =
        field_arr(field(&note, "params").unwrap(), "diagnostics").expect("diagnostics array");
    let tag_warnings: Vec<&Parsed> = diags
        .iter()
        .filter(|d| field_str(d, "source").as_deref() == Some("loft-tag"))
        .collect();
    assert_eq!(
        tag_warnings.len(),
        1,
        "exactly one broken-tag warning: {diags:?}"
    );
    let w = tag_warnings[0];
    assert_eq!(
        field(w, "severity").and_then(Parsed::as_i64),
        Some(2),
        "Warning"
    );
    assert!(
        field_str(w, "message").unwrap_or_default().contains("@P99"), // <!--noindex-->
        "names the broken tag: {w:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// find-references — a workspace with two files; references to a symbol span both,
// and the open file's live buffer is overlaid.
#[test]
fn find_references_spans_the_workspace() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("refws2");
    fa::create_dir_all(&root).unwrap();
    fa::write(
        root.join("a.loft"),
        "fn area(w: integer) -> integer {\n  w * w\n}\n",
    )
    .unwrap();
    fa::write(root.join("b.loft"), "fn main() {\n  print(area(3))\n}\n").unwrap();

    let mut s = Session::start();
    let caps = {
        s.request(
            1,
            "initialize",
            &format!(r#"{{"rootUri":"{}"}}"#, loft::lsp::path_to_uri(&root)),
        );
        let init = s.recv();
        field(field(&init, "result").unwrap(), "capabilities")
            .unwrap()
            .clone()
    };
    assert!(
        matches!(field(&caps, "referencesProvider"), Some(Parsed::Bool(true))),
        "advertises referencesProvider"
    );
    s.notify("initialized", "{}");

    let a_uri = loft::lsp::path_to_uri(&root.join("a.loft"));
    let a_text = "fn area(w: integer) -> integer {\n  w * w\n}\n";
    s.notify("textDocument/didOpen", &open_params(&a_uri, a_text));
    let _ = s.recv(); // publishDiagnostics

    // References to `area` — cursor on its definition (0-based line 0, char 3).
    s.request(
        2,
        "textDocument/references",
        &format!(
            r#"{{"textDocument":{{"uri":"{a_uri}"}},"position":{{"line":0,"character":3}},"context":{{"includeDeclaration":true}}}}"#
        ),
    );
    let reply = s.recv();
    let locs = match field(&reply, "result") {
        Some(Parsed::Array(a)) => a,
        other => panic!("references result is an array, got {other:?}"),
    };
    let files: Vec<String> = locs
        .iter()
        .filter_map(|l| field_str(l, "uri"))
        .map(|u| u.rsplit('/').next().unwrap_or("").to_string())
        .collect();
    assert!(
        files.iter().any(|f| f == "a.loft") && files.iter().any(|f| f == "b.loft"),
        "references span both files: {files:?}"
    );
    assert_eq!(locs.len(), 2, "the def + the one call: {locs:?}");

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// didSave invalidates the cached workspace reverse index, so a file added to disk
// after the first find-references is picked up on the next query.
#[test]
fn did_save_refreshes_the_workspace_index() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("savews");
    // Clean slate — the test ADDS c.loft mid-run, so a leftover copy from a prior
    // run would poison the first (pre-c.loft) reference count.
    let _ = fa::remove_dir_all(&root);
    fa::create_dir_all(&root).unwrap();
    fa::write(
        root.join("a.loft"),
        "fn area(w: integer) -> integer {\n  w * w\n}\n",
    )
    .unwrap();
    fa::write(root.join("b.loft"), "fn main() {\n  print(area(3))\n}\n").unwrap();

    let mut s = Session::start();
    s.request(
        1,
        "initialize",
        &format!(r#"{{"rootUri":"{}"}}"#, loft::lsp::path_to_uri(&root)),
    );
    let init = s.recv();
    let sync = field(
        field(field(&init, "result").unwrap(), "capabilities").unwrap(),
        "textDocumentSync",
    )
    .unwrap();
    assert!(
        matches!(field(sync, "save"), Some(Parsed::Bool(true))),
        "advertises save so the client notifies didSave: {init:?}"
    );
    s.notify("initialized", "{}");

    let a_uri = loft::lsp::path_to_uri(&root.join("a.loft"));
    let a_text = "fn area(w: integer) -> integer {\n  w * w\n}\n";
    s.notify("textDocument/didOpen", &open_params(&a_uri, a_text));
    let _ = s.recv();

    let refs_query = format!(
        r#"{{"textDocument":{{"uri":"{a_uri}"}},"position":{{"line":0,"character":3}},"context":{{"includeDeclaration":true}}}}"#
    );
    let count_refs = |s: &mut Session, id: i64| -> usize {
        s.request(id, "textDocument/references", &refs_query);
        match field(&s.recv(), "result") {
            Some(Parsed::Array(a)) => a.len(),
            other => panic!("references result is an array, got {other:?}"),
        }
    };

    // First query builds the index over a.loft + b.loft → the def + one call.
    assert_eq!(count_refs(&mut s, 2), 2, "def in a.loft + call in b.loft");

    // A NEW file appears on disk; without invalidation the cached index misses it.
    fa::write(root.join("c.loft"), "fn other() {\n  print(area(4))\n}\n").unwrap();
    assert_eq!(
        count_refs(&mut s, 3),
        2,
        "stale cache does not yet see c.loft"
    );

    // didSave drops the cache → the next query rebuilds and finds c.loft's call.
    s.notify(
        "textDocument/didSave",
        &format!(r#"{{"textDocument":{{"uri":"{a_uri}"}}}}"#),
    );
    assert_eq!(
        count_refs(&mut s, 4),
        3,
        "after didSave the rebuild sees c.loft"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// The tag index reloads when index/tags.json changes mid-session (a `make index`):
// a tag absent at startup resolves once it is added to the index.
#[test]
fn tag_index_reloads_when_the_index_changes() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("tagws-mtime");
    let idx = root.join("index");
    fa::create_dir_all(&idx).unwrap();
    // Start with an index that does NOT know @P321. <!--noindex-->
    fa::write(
        idx.join("tags.json"),
        r#"{"@F1":[{"file":"a.md","line":1,"context":"@F1"}]}"#,
    )
    .unwrap();
    fa::write(
        idx.join("features.json"),
        r#"[{"number":1,"title":"Keyed collections","kind":"feature","body":"x\n"}]"#,
    )
    .unwrap();

    let mut s = Session::start();
    s.request(
        1,
        "initialize",
        &format!(r#"{{"rootUri":"{}"}}"#, loft::lsp::path_to_uri(&root)),
    );
    let _ = s.recv();
    s.notify("initialized", "{}");

    let uri = "file:///t.loft";
    let prog = "// @P321 here\nfn main() {\n}\n"; // <!--noindex-->
    s.notify("textDocument/didOpen", &open_params(uri, prog));
    let _ = s.recv();

    let hover =
        format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":0,"character":5}}}}"#);

    // @P321 is not in the index yet → hover resolves nothing. <!--noindex-->
    s.request(2, "textDocument/hover", &hover);
    assert!(
        matches!(field(&s.recv(), "result"), Some(Parsed::Null)),
        "unknown tag hovers to null before the index knows it"
    );

    // Regenerate the index (a mid-session `make index`); sleep first so the mtime
    // is guaranteed to differ from the loaded one.
    std::thread::sleep(std::time::Duration::from_millis(10));
    fa::write(
        idx.join("tags.json"),
        r#"{"@F1":[{"file":"a.md","line":1,"context":"@F1"}],"@P321":[{"file":"P.md","line":3,"context":"@P321 a bug"}]}"#, // <!--noindex-->
    )
    .unwrap();

    // The next hover re-stats tags.json, sees the new mtime, reloads → @P321 resolves.
    s.request(3, "textDocument/hover", &hover);
    let h = s.recv();
    let val = field(field(&h, "result").expect("hover result"), "contents")
        .and_then(|c| field_str(c, "value"))
        .unwrap_or_default();
    assert!(
        val.contains("@P321"), // <!--noindex-->
        "after the index reload the tag resolves: {val}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// @PLN115 — method find-references spans the workspace, keyed on the receiver TYPE:
// every `text.len` call across files, and NOT a same-spelled `vector.len`.
#[test]
fn method_references_span_the_workspace_by_receiver_type() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("methrefws");
    fa::create_dir_all(&root).unwrap();
    // a.loft: a text.len call AND a vector.len call (same spelling, other type).
    let a_text =
        "fn main() {\n  s = \"hi\";\n  x = s.len();\n  v = [1, 2, 3];\n  w = v.len();\n}\n";
    fa::write(root.join("a.loft"), a_text).unwrap();
    // b.loft: another text.len call, in a different file.
    fa::write(
        root.join("b.loft"),
        "fn other() {\n  t = \"yo\";\n  y = t.len();\n}\n",
    )
    .unwrap();

    let mut s = Session::start();
    s.request(
        1,
        "initialize",
        &format!(r#"{{"rootUri":"{}"}}"#, loft::lsp::path_to_uri(&root)),
    );
    let _ = s.recv();
    s.notify("initialized", "{}");

    let a_uri = loft::lsp::path_to_uri(&root.join("a.loft"));
    s.notify("textDocument/didOpen", &open_params(&a_uri, a_text));
    let _ = s.recv();

    // Cursor on a.loft's `s.len()` method name (0-based line 2, char 8).
    s.request(
        2,
        "textDocument/references",
        &format!(
            r#"{{"textDocument":{{"uri":"{a_uri}"}},"position":{{"line":2,"character":8}},"context":{{"includeDeclaration":true}}}}"#
        ),
    );
    let reply = s.recv();
    let locs = match field(&reply, "result") {
        Some(Parsed::Array(a)) => a,
        other => panic!("references result is an array, got {other:?}"),
    };
    let files: Vec<String> = locs
        .iter()
        .filter_map(|l| field_str(l, "uri"))
        .map(|u| u.rsplit('/').next().unwrap_or("").to_string())
        .collect();
    // The two text.len calls (a.loft L3 + b.loft L3) — NOT a.loft's vector.len (L5).
    assert_eq!(locs.len(), 2, "exactly the two text.len calls: {files:?}");
    assert!(
        files.iter().any(|f| f == "a.loft") && files.iter().any(|f| f == "b.loft"),
        "text.len references span both files: {files:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// rename — prepareRename returns the identifier's span; rename produces a
// cross-file WorkspaceEdit; an invalid new name is refused with an error.
#[test]
fn rename_produces_a_cross_file_workspace_edit() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("renws3");
    fa::create_dir_all(&root).unwrap();
    fa::write(
        root.join("a.loft"),
        "fn area(w: integer) -> integer {\n  w * w\n}\n",
    )
    .unwrap();
    fa::write(root.join("b.loft"), "fn main() {\n  print(area(3))\n}\n").unwrap();

    let mut s = Session::start();
    s.request(
        1,
        "initialize",
        &format!(r#"{{"rootUri":"{}"}}"#, loft::lsp::path_to_uri(&root)),
    );
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    assert!(
        field(caps, "renameProvider").is_some(),
        "advertises renameProvider"
    );
    s.notify("initialized", "{}");

    let a_uri = loft::lsp::path_to_uri(&root.join("a.loft"));
    s.notify(
        "textDocument/didOpen",
        &open_params(&a_uri, "fn area(w: integer) -> integer {\n  w * w\n}\n"),
    );
    let _ = s.recv();

    // prepareRename on `area` → its span (0-based cols 3..7) + placeholder.
    s.request(
        2,
        "textDocument/prepareRename",
        &format!(r#"{{"textDocument":{{"uri":"{a_uri}"}},"position":{{"line":0,"character":3}}}}"#),
    );
    let prep = s.recv();
    let pr = field(&prep, "result").expect("prepareRename result");
    assert_eq!(field_str(pr, "placeholder").as_deref(), Some("area"));

    // rename area → zone → a WorkspaceEdit touching BOTH files.
    s.request(
        3,
        "textDocument/rename",
        &format!(r#"{{"textDocument":{{"uri":"{a_uri}"}},"position":{{"line":0,"character":3}},"newName":"zone"}}"#),
    );
    let ren = s.recv();
    let changes = field(field(&ren, "result").expect("rename result"), "changes")
        .expect("WorkspaceEdit has changes");
    let files: Vec<String> = match changes {
        Parsed::Object(e) => e
            .iter()
            .map(|(uri, _, _)| uri.rsplit('/').next().unwrap_or("").to_string())
            .collect(),
        other => panic!("changes is an object, got {other:?}"),
    };
    assert!(
        files.iter().any(|f| f == "a.loft") && files.iter().any(|f| f == "b.loft"),
        "the edit spans both files: {files:?}"
    );

    // An invalid new name is refused with a JSON-RPC error.
    s.request(
        4,
        "textDocument/rename",
        &format!(r#"{{"textDocument":{{"uri":"{a_uri}"}},"position":{{"line":0,"character":3}},"newName":"fn"}}"#),
    );
    let bad = s.recv();
    assert!(
        field(&bad, "error").is_some() && field(&bad, "result").is_none(),
        "an invalid rename returns an error, not a result: {bad:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// codeAction (A→B) — an unknown call publishes a diagnostic carrying a
// structured suggestion; echoing it back in a codeAction yields a "Change to
// `X`" quick-fix whose edit replaces the token.
#[test]
fn code_action_offers_the_did_you_mean_fix() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    assert!(
        field(caps, "codeActionProvider").is_some(),
        "advertises codeActionProvider (an object with codeActionKinds)"
    );
    s.notify("initialized", "{}");

    let uri = "file:///a.loft";
    s.notify(
        "textDocument/didOpen",
        &open_params(uri, "fn main() {\n  nope(3)\n}\n"),
    );
    let note = s.recv();
    let diags = field_arr(field(&note, "params").unwrap(), "diagnostics").unwrap();
    let diag = &diags[0];
    // Step A: the published diagnostic carries the structured suggestion.
    assert_eq!(
        field(diag, "data")
            .and_then(|d| field_str(d, "suggestion"))
            .as_deref(),
        Some("move"),
        "the diagnostic round-trips a structured suggestion: {diag:?}"
    );

    // Step B: echo the diagnostic back in a codeAction → a quick-fix edit.
    let range = field(diag, "range").unwrap();
    s.request(
        2,
        "textDocument/codeAction",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri}"}},"range":{},"context":{{"diagnostics":[{}]}}}}"#,
            json::to_json_string(range),
            json::to_json_string(diag),
        ),
    );
    let reply = s.recv();
    let actions = match field(&reply, "result") {
        Some(Parsed::Array(a)) => a,
        other => panic!("codeAction result is an array, got {other:?}"),
    };
    // ONE quick-fix, not two. Every did-you-mean site now emits a structured `Fix` as well
    // as the legacy `data.suggestion`, and offering both listed the same rename twice under
    // different titles — so the code-action path prefers `fixes` and skips the legacy one
    // where a fix exists (@PLN131).
    // Counted among the QUICK-FIXES: the list also carries the overview action (@PLN183 P3),
    // which is offered at every position by design.
    let fixes: Vec<&Parsed> = actions
        .iter()
        .filter(|a| field_str(a, "kind").as_deref() == Some("quickfix"))
        .collect();
    assert_eq!(fixes.len(), 1, "one quick-fix: {actions:?}");
    let a = fixes[0];
    assert!(
        field_str(a, "title").unwrap_or_default().contains("move"),
        "title names the fix: {a:?}"
    );
    let changes = field(field(a, "edit").unwrap(), "changes").unwrap();
    let new_text = match changes {
        Parsed::Object(e) => e
            .iter()
            .find_map(|(_, _, v)| match v {
                Parsed::Array(edits) => edits.first().and_then(|ed| field_str(ed, "newText")),
                _ => None,
            })
            .unwrap_or_default(),
        _ => String::new(),
    };
    assert_eq!(
        new_text, "move",
        "the edit replaces the token with `move`: {a:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// codeAction step-B follow-up — the @PLN102 arc-C #superseded steer offers a
// "Change to `Y`" quick-fix that replaces the call name with the successor.
#[test]
fn code_action_offers_the_superseded_steer_fix() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let _ = s.recv();
    s.notify("initialized", "{}");

    let uri = "file:///s.loft";
    // `#superseded` marks the PRECEDING def (old_add); main's call to it steers to new_add.
    let prog = "fn old_add(a: integer, b: integer) -> integer { new_add(a, b) }\n\
                #superseded \"new_add\"\n\
                fn new_add(a: integer, b: integer) -> integer { a + b }\n\
                fn main() -> integer { old_add(1, 2) }\n";
    s.notify("textDocument/didOpen", &open_params(uri, prog));
    let note = s.recv();
    let diags = field_arr(field(&note, "params").unwrap(), "diagnostics").unwrap();
    let diag = diags
        .iter()
        .find(|d| {
            field_str(d, "message")
                .unwrap_or_default()
                .contains("superseded")
        })
        .expect("the steer warning is published");
    // The published diagnostic round-trips the successor as a structured suggestion.
    assert_eq!(
        field(diag, "data")
            .and_then(|d| field_str(d, "suggestion"))
            .as_deref(),
        Some("new_add"),
        "steer diagnostic carries the successor: {diag:?}"
    );

    // Echo it back in a codeAction → a quick-fix that rewrites the call name.
    let range = field(diag, "range").unwrap();
    s.request(
        2,
        "textDocument/codeAction",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri}"}},"range":{},"context":{{"diagnostics":[{}]}}}}"#,
            json::to_json_string(range),
            json::to_json_string(diag),
        ),
    );
    let reply = s.recv();
    let actions = match field(&reply, "result") {
        Some(Parsed::Array(a)) => a,
        other => panic!("codeAction result is an array, got {other:?}"),
    };
    let a = actions.first().expect("one quick-fix");
    assert!(
        field_str(a, "title")
            .unwrap_or_default()
            .contains("new_add"),
        "title names the successor: {a:?}"
    );
    let changes = field(field(a, "edit").unwrap(), "changes").unwrap();
    let new_text = match changes {
        Parsed::Object(e) => e
            .iter()
            .find_map(|(_, _, v)| match v {
                Parsed::Array(edits) => edits.first().and_then(|ed| field_str(ed, "newText")),
                _ => None,
            })
            .unwrap_or_default(),
        _ => String::new(),
    };
    assert_eq!(
        new_text, "new_add",
        "the edit replaces the call name with `new_add`: {a:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// E0 (extract-function) — `refactor.extract` is advertised and offered on a
// MULTI-LINE selection, absent on a bare cursor (EXTRACT.md E0).
#[test]
fn extract_function_offered_on_a_multiline_selection() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    let kinds = field(
        field(caps, "codeActionProvider").unwrap(),
        "codeActionKinds",
    );
    assert!(
        matches!(kinds, Some(Parsed::Array(a)) if a.iter().any(|k| matches!(k, Parsed::Str(s) if s == "refactor.extract"))),
        "advertises the refactor.extract kind: {init:?}"
    );
    s.notify("initialized", "{}");

    let uri = "file:///x.loft";
    // 0-based lines 1-2 are `y = x + 1;` and `z = y * 2;`.
    let prog = "fn f(x: integer) -> integer {\n  y = x + 1;\n  z = y * 2;\n  return z;\n}\n";
    s.notify("textDocument/didOpen", &open_params(uri, prog));
    let _ = s.recv();

    let code_action = |s: &mut Session, id: i64, range: &str| -> Vec<Parsed> {
        s.request(
            id,
            "textDocument/codeAction",
            &format!(
                r#"{{"textDocument":{{"uri":"{uri}"}},"range":{range},"context":{{"diagnostics":[]}}}}"#
            ),
        );
        match field(&s.recv(), "result") {
            Some(Parsed::Array(a)) => a.clone(),
            other => panic!("codeAction result is an array, got {other:?}"),
        }
    };

    // A MULTI-LINE selection of the two assignments → an extract action carrying the
    // real WorkspaceEdit (E4): the new function + the call.
    let actions = code_action(
        &mut s,
        2,
        r#"{"start":{"line":1,"character":2},"end":{"line":2,"character":12}}"#,
    );
    let ext = actions
        .iter()
        .find(|a| field_str(a, "kind").as_deref() == Some("refactor.extract"))
        .expect("an extract action is offered");
    let changes = field(field(ext, "edit").unwrap(), "changes").unwrap();
    let new_texts: Vec<String> = match changes {
        Parsed::Object(e) => e
            .iter()
            .flat_map(|(_, _, v)| match v {
                Parsed::Array(edits) => edits
                    .iter()
                    .filter_map(|ed| field_str(ed, "newText"))
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect(),
        _ => Vec::new(),
    };
    let joined = new_texts.join("\n");
    assert!(
        joined.contains("fn extracted(x: integer) -> integer"),
        "the new function is in the edit: {joined}"
    );
    assert!(
        joined.contains("z = extracted(x)"),
        "the call replaces the selection: {joined}"
    );

    // A bare cursor (zero-width, single line) → no extract action.
    let cursor = code_action(
        &mut s,
        3,
        r#"{"start":{"line":1,"character":2},"end":{"line":1,"character":2}}"#,
    );
    assert!(
        !cursor
            .iter()
            .any(|a| field_str(a, "kind").as_deref() == Some("refactor.extract")),
        "no extract on a bare cursor"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// completion (step C) — `expr.` offers the type's members over the wire.
#[test]
fn completion_offers_members_after_a_dot() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    assert!(
        field(caps, "completionProvider").is_some(),
        "advertises completionProvider"
    );
    s.notify("initialized", "{}");

    let uri = "file:///c.loft";
    let prog = "struct Point {\n  x: integer,\n}\nfn main() {\n  p = Point { x: 1 };\n  p.\n}\n";
    s.notify("textDocument/didOpen", &open_params(uri, prog));
    let _ = s.recv();

    // `p.` on 0-based line 5, char 4 (after the dot).
    s.request(
        2,
        "textDocument/completion",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":5,"character":4}}}}"#),
    );
    let reply = s.recv();
    let items = match field(&reply, "result") {
        Some(Parsed::Array(a)) => a,
        other => panic!("completion result is an array, got {other:?}"),
    };
    let labels: Vec<String> = items.iter().filter_map(|i| field_str(i, "label")).collect();
    assert!(
        labels.iter().any(|l| l == "x"),
        "the struct's field is offered: {labels:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// T4 — completion of a partial `@`-tag offers tracker tags from the workspace index.
#[test]
fn tag_completion_offers_tracker_tags() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("tagws-complete");
    let idx = root.join("index");
    fa::create_dir_all(&idx).unwrap();
    fa::write(
        idx.join("tags.json"),
        r#"{"@F1":[{"file":"a.md","line":1,"context":"@F1"}]}"#,
    )
    .unwrap();
    fa::write(
        idx.join("features.json"),
        r#"[{"number":1,"title":"Keyed collections","kind":"feature","body":"Look up records by key.\n"}]"#,
    )
    .unwrap();

    let mut s = Session::start();
    let root_uri = loft::lsp::path_to_uri(&root);
    s.request(1, "initialize", &format!(r#"{{"rootUri":"{root_uri}"}}"#));
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    let triggers = field(
        field(caps, "completionProvider").unwrap(),
        "triggerCharacters",
    );
    assert!(
        matches!(triggers, Some(Parsed::Array(a)) if a.iter().any(|c| matches!(c, Parsed::Str(s) if s == "@"))),
        "advertises `@` as a completion trigger: {init:?}"
    );
    s.notify("initialized", "{}");

    let uri = "file:///t.loft";
    // A comment line ending in the partial tag `@F` (chars 0..5 → cursor at char 5).
    let prog = "// @F\nfn main() {\n}\n";
    s.notify("textDocument/didOpen", &open_params(uri, prog));
    let _ = s.recv();

    s.request(
        2,
        "textDocument/completion",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":0,"character":5}}}}"#),
    );
    let reply = s.recv();
    let items = match field(&reply, "result") {
        Some(Parsed::Array(a)) => a,
        other => panic!("completion result is an array, got {other:?}"),
    };
    let labels: Vec<String> = items.iter().filter_map(|i| field_str(i, "label")).collect();
    assert!(
        labels.iter().any(|l| l == "@F1"),
        "the `@F1` tracker tag is offered: {labels:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// semanticTokens (step D) — the full-document token stream is advertised and
// returns a valid delta-encoded int array.
#[test]
fn semantic_tokens_full_returns_encoded_tokens() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    assert!(
        field(caps, "semanticTokensProvider").is_some(),
        "advertises semanticTokensProvider"
    );
    s.notify("initialized", "{}");

    let uri = "file:///s.loft";
    s.notify(
        "textDocument/didOpen",
        &open_params(
            uri,
            "struct Point {\n  x: integer,\n}\nfn main() {\n  print(\"hi\")\n}\n",
        ),
    );
    let _ = s.recv();

    s.request(
        2,
        "textDocument/semanticTokens/full",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}}}}"#),
    );
    let reply = s.recv();
    let data = field_arr(field(&reply, "result").unwrap(), "data").expect("a data array");
    assert!(!data.is_empty(), "a non-empty token stream");
    assert_eq!(data.len() % 5, 0, "five ints per token, got {}", data.len());

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// @PLN115 integration — the resolution index drives go-to-definition + hover for
// LOCALS and METHODS, which name-based lookup could not resolve at all.
#[test]
fn definition_and_hover_resolve_locals_and_methods_via_index() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let _ = s.recv();
    s.notify("initialized", "{}");

    let uri = "file:///idx.loft";
    // L1 count decl · L2 count USE · L4 greeting.len() method call.
    let prog = "fn main() {\n  count = 5;\n  total = count + 1;\n  \
                greeting = \"hi\";\n  size = greeting.len();\n}\n";
    s.notify("textDocument/didOpen", &open_params(uri, prog));
    let _ = s.recv();

    // Go-to-def on the LOCAL `count` USE (0-based line 2, char 10) → its declaration
    // (line 1, char 2) in the same buffer.  Impossible before the index.
    s.request(
        2,
        "textDocument/definition",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":2,"character":10}}}}"#),
    );
    let loc = field(&s.recv(), "result").cloned().expect("local Location");
    assert_eq!(
        field_str(&loc, "uri").as_deref(),
        Some(uri),
        "stays in buffer"
    );
    let start = field(field(&loc, "range").unwrap(), "start").unwrap();
    assert_eq!(
        (
            field(start, "line").and_then(Parsed::as_i64),
            field(start, "character").and_then(Parsed::as_i64),
        ),
        (Some(1), Some(2)),
        "jumps to the `count` declaration: {loc:?}"
    );

    // Go-to-def on the METHOD `.len` (0-based line 4, char 18) → into the stdlib.
    s.request(
        3,
        "textDocument/definition",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":4,"character":18}}}}"#),
    );
    let loc = field(&s.recv(), "result")
        .cloned()
        .expect("method Location");
    let target = field_str(&loc, "uri").unwrap_or_default();
    assert!(
        target.starts_with("file://") && target.contains("default/"),
        "method jumps into its stdlib source: {target}"
    );

    // Hover on the LOCAL `total` (0-based line 2, char 2) → its inferred type.
    s.request(
        4,
        "textDocument/hover",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":2,"character":2}}}}"#),
    );
    let hv = s.recv();
    let contents = field(field(&hv, "result").expect("hover result"), "contents").unwrap();
    let value = field_str(contents, "value").unwrap_or_default();
    assert!(
        value.contains("total: integer"),
        "hover shows the local's type: {value}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// S7 (@PLN115) — inferred-type inlay hints at assignment-local declarations (the
// feature E, unblocked by the resolution index).
#[test]
fn inlay_hints_annotate_local_declaration_types() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    assert!(
        field(caps, "inlayHintProvider").is_some(),
        "advertises inlayHintProvider"
    );
    s.notify("initialized", "{}");

    let uri = "file:///h.loft";
    s.notify(
        "textDocument/didOpen",
        &open_params(uri, "fn main() {\n  n = 5;\n  greeting = \"hi\";\n}\n"),
    );
    let _ = s.recv();

    s.request(
        2,
        "textDocument/inlayHint",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri}"}},"range":{{"start":{{"line":0,"character":0}},"end":{{"line":4,"character":0}}}}}}"#
        ),
    );
    let reply = s.recv();
    let hints = match field(&reply, "result").expect("inlayHint result") {
        Parsed::Array(a) => a.clone(),
        other => panic!("expected an array, got {other:?}"),
    };
    let labels: Vec<String> = hints.iter().filter_map(|h| field_str(h, "label")).collect();
    assert!(
        labels.iter().any(|l| l == ": integer"),
        "n : integer: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == ": text"),
        "greeting : text: {labels:?}"
    );
    // Each hint carries a 0-based position (kind 1 = Type).
    let first = &hints[0];
    assert!(
        field(field(first, "position").unwrap(), "line").is_some(),
        "hint carries a position: {first:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// F (v1) — renaming a LOCAL touches only its own function, not a same-named
// local in another function.
#[test]
fn rename_a_local_scopes_to_its_function() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fscopews");
    fa::create_dir_all(&root).unwrap();
    // fn a (lines 0..3) has three `x`; fn b (lines 4..6) has its own `x`.
    let prog = "fn a() {\n  x = 1\n  x = x + 1\n}\nfn b() {\n  x = 9\n}\n";
    fa::write(root.join("m.loft"), prog).unwrap();

    let mut s = Session::start();
    s.request(
        1,
        "initialize",
        &format!(r#"{{"rootUri":"{}"}}"#, loft::lsp::path_to_uri(&root)),
    );
    let _ = s.recv();
    s.notify("initialized", "{}");
    let uri = loft::lsp::path_to_uri(&root.join("m.loft"));
    s.notify("textDocument/didOpen", &open_params(&uri, prog));
    let _ = s.recv();

    // Rename `a`'s `x` (0-based line 1, char 2).
    s.request(
        2,
        "textDocument/rename",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":1,"character":2}},"newName":"total"}}"#),
    );
    let reply = s.recv();
    let changes = field(field(&reply, "result").expect("rename result"), "changes").unwrap();
    let lines: Vec<i64> = match changes {
        Parsed::Object(e) => e
            .iter()
            .flat_map(|(_, _, v)| match v {
                Parsed::Array(edits) => edits
                    .iter()
                    .filter_map(|ed| {
                        field(field(ed, "range")?, "start")
                            .and_then(|st| field(st, "line"))
                            .and_then(Parsed::as_i64)
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect(),
        _ => Vec::new(),
    };
    assert!(!lines.is_empty(), "some edits: {reply:?}");
    assert!(
        lines.iter().all(|&l| l < 4),
        "only fn a's block (lines <4) is edited, not fn b's `x` on line 5: {lines:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// @PLN115 tail — renaming a PARAMETER edits its signature declaration + body uses
// and, keyed on the binding identity, excludes a same-named FIELD (`q.w`).
#[test]
fn rename_a_parameter_edits_the_signature_and_excludes_a_field() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("paramrename");
    let _ = fa::remove_dir_all(&root);
    fa::create_dir_all(&root).unwrap();
    // `w` is a param AND the name of a field of Q; `q.w` must NOT be renamed.
    let prog =
        "struct Q { w: integer }\nfn f(w: integer, q: Q) -> integer {\n  return w + q.w\n}\n";
    fa::write(root.join("m.loft"), prog).unwrap();

    let mut s = Session::start();
    s.request(
        1,
        "initialize",
        &format!(r#"{{"rootUri":"{}"}}"#, loft::lsp::path_to_uri(&root)),
    );
    let _ = s.recv();
    s.notify("initialized", "{}");
    let uri = loft::lsp::path_to_uri(&root.join("m.loft"));
    s.notify("textDocument/didOpen", &open_params(&uri, prog));
    let _ = s.recv();

    // Rename the param `w` — cursor on the body use (0-based line 2, char 9).
    s.request(
        2,
        "textDocument/rename",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":2,"character":9}},"newName":"width"}}"#),
    );
    let reply = s.recv();
    let changes = field(field(&reply, "result").expect("rename result"), "changes").unwrap();
    let edits: Vec<(i64, i64)> = match changes {
        Parsed::Object(e) => e
            .iter()
            .flat_map(|(_, _, v)| match v {
                Parsed::Array(a) => a
                    .iter()
                    .filter_map(|ed| {
                        let st = field(field(ed, "range")?, "start")?;
                        Some((
                            field(st, "line").and_then(Parsed::as_i64)?,
                            field(st, "character").and_then(Parsed::as_i64)?,
                        ))
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect(),
        _ => Vec::new(),
    };
    // The signature `w` (line 1, char 5) + the body use (line 2, char 9) = 2 edits.
    assert_eq!(edits.len(), 2, "signature decl + one body use: {edits:?}");
    assert!(
        edits.contains(&(1, 5)),
        "edits the signature declaration so the rename is complete: {edits:?}"
    );
    // The field `q.w`'s `w` sits at line 2 char 13; it must NOT be edited.
    assert!(
        !edits.iter().any(|(l, c)| *l == 2 && *c > 10),
        "the same-named field q.w is excluded: {edits:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

// S4 (@PLN115) — renaming an assignment-local resolves by binding identity, so a
// same-named FIELD access (`p.x`) is EXCLUDED — the F-v1 name-scan could not do this.
#[test]
fn rename_a_local_excludes_a_same_named_field() {
    let root = std::path::PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("fieldexclws");
    fa::create_dir_all(&root).unwrap();
    // Local `x` (decl L3, read in `{x}` L4) alongside field `p.x` (also L4, spelled `x`).
    let prog = "struct P { x: integer }\nfn h(p: P) {\n  x = 5;\n  print(\"{p.x} {x}\");\n}\n";
    fa::write(root.join("m.loft"), prog).unwrap();

    let mut s = Session::start();
    s.request(
        1,
        "initialize",
        &format!(r#"{{"rootUri":"{}"}}"#, loft::lsp::path_to_uri(&root)),
    );
    let _ = s.recv();
    s.notify("initialized", "{}");
    let uri = loft::lsp::path_to_uri(&root.join("m.loft"));
    s.notify("textDocument/didOpen", &open_params(&uri, prog));
    let _ = s.recv();

    // Rename the local `x` at its declaration (0-based line 2, char 2) → `y`.
    s.request(
        2,
        "textDocument/rename",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":2,"character":2}},"newName":"y"}}"#),
    );
    let reply = s.recv();
    let changes = field(field(&reply, "result").expect("rename result"), "changes").unwrap();
    let starts: Vec<(i64, i64)> = match changes {
        Parsed::Object(e) => e
            .iter()
            .flat_map(|(_, _, v)| match v {
                Parsed::Array(edits) => edits
                    .iter()
                    .filter_map(|ed| {
                        let st = field(field(ed, "range")?, "start")?;
                        Some((
                            field(st, "line").and_then(Parsed::as_i64)?,
                            field(st, "character").and_then(Parsed::as_i64)?,
                        ))
                    })
                    .collect::<Vec<_>>(),
                _ => Vec::new(),
            })
            .collect(),
        _ => Vec::new(),
    };
    // Exactly the two local occurrences (decl + the `{x}` read); the field `p.x`'s
    // `x` (0-based line 3, char 12) must NOT be edited.
    assert_eq!(
        starts.len(),
        2,
        "only the local's 2 occurrences: {starts:?}"
    );
    assert!(
        starts.contains(&(2, 2)),
        "the declaration is renamed: {starts:?}"
    );
    assert!(
        !starts.contains(&(3, 12)),
        "the field p.x is NOT renamed: {starts:?}"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}

#[test]
fn initialize_handshake_and_clean_shutdown() {
    let mut s = Session::start();

    s.request(1, "initialize", "{}");
    let reply = s.recv();
    assert_eq!(
        field(&reply, "id").and_then(Parsed::as_i64),
        Some(1),
        "id must echo the request"
    );
    let result = field(&reply, "result").expect("initialize returns a result, not an error");
    assert!(
        field(result, "capabilities").is_some(),
        "advertises capabilities"
    );
    let info = field(result, "serverInfo").expect("advertises serverInfo");
    assert_eq!(field_str(info, "name").as_deref(), Some("loft-lsp"));

    s.notify("initialized", "{}"); // notification — must draw no reply

    s.request(2, "shutdown", "null");
    let sd = s.recv();
    assert_eq!(field(&sd, "id").and_then(Parsed::as_i64), Some(2));
    assert!(
        matches!(field(&sd, "result"), Some(Parsed::Null)),
        "shutdown result is null"
    );

    s.notify("exit", "null");
    let status = s.child.wait().expect("wait for loft-lsp");
    assert_eq!(status.code(), Some(0), "exit after shutdown must be code 0");
}

// POSITIVE CONTROL — the harness must be able to SEE a failure path, or a green
// handshake proves nothing.  An unhandled request must come back as a JSON-RPC
// error (never a bogus success), and the harness must read it as such.
#[test]
fn unknown_request_is_an_error_not_a_result() {
    let mut s = Session::start();
    s.request(1, "initialize", "{}");
    let _ = s.recv();

    s.request(7, "textDocument/signatureHelp", "{}"); // deliberately unimplemented
    let reply = s.recv();
    assert_eq!(field(&reply, "id").and_then(Parsed::as_i64), Some(7));
    assert!(
        field(&reply, "result").is_none(),
        "an unhandled request must NOT return a result"
    );
    let err = field(&reply, "error").expect("an unhandled request returns an error object");
    assert_eq!(
        field(err, "code").and_then(Parsed::as_i64),
        Some(-32601),
        "MethodNotFound"
    );

    s.notify("exit", "null");
    let _ = s.child.wait();
}
