// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN63 D0 — protocol test harness for `loft-dap` (the DAP debug adapter).
//
// Drives the loft-dap binary over stdio with `Content-Length`-framed DAP JSON and
// asserts its replies + events, so the transport (D1) and every later step (D2-D6)
// is CI-tested WITHOUT a live editor.  The harness carries its own positive control
// (`unsupported_request_is_an_error_not_a_success`): it proves it can distinguish a
// FAILED response (`success:false`) from a success — so a green handshake test is
// meaningful, not vacuous (the @PLN16 "prove the harness can fail" rule).
//
// The engine path underneath is already proven end-to-end by `tests/rpc.rs`; these
// tests exercise only the DAP TRANSLATION (framing, envelope, drill-down synthesis).

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use loft::json::{self, Parsed};

/// A live loft-dap subprocess with framed read/write over its stdio.
struct Dap {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    seq: i64,
}

impl Dap {
    fn start() -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_loft-dap"))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn loft-dap");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Dap {
            child,
            stdin,
            stdout,
            seq: 0,
        }
    }

    /// Send a DAP request, returning the `seq` used (so a response can be matched by
    /// `request_seq`).
    fn request(&mut self, command: &str, arguments: &str) -> i64 {
        self.seq += 1;
        let seq = self.seq;
        let body = format!(
            r#"{{"seq":{seq},"type":"request","command":"{command}","arguments":{arguments}}}"#
        );
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
        seq
    }

    /// Read one framed message and parse it.
    fn recv(&mut self) -> Parsed {
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).unwrap();
            assert!(n > 0, "adapter closed stdout before replying");
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

    /// Read messages until a `response` for `request_seq` arrives (skipping events),
    /// then return it.
    fn recv_response(&mut self, request_seq: i64) -> Parsed {
        for _ in 0..64 {
            let m = self.recv();
            if field_str(&m, "type").as_deref() == Some("response")
                && field_i64(&m, "request_seq") == Some(request_seq)
            {
                return m;
            }
        }
        panic!("no response for request_seq {request_seq}");
    }

    /// Read messages until an `event` named `name` arrives (skipping others), then return
    /// its body.
    fn recv_event(&mut self, name: &str) -> Parsed {
        for _ in 0..64 {
            let m = self.recv();
            if field_str(&m, "type").as_deref() == Some("event")
                && field_str(&m, "event").as_deref() == Some(name)
            {
                return field(&m, "body").cloned().unwrap_or(Parsed::Null);
            }
        }
        panic!("no `{name}` event arrived");
    }

    /// The standard `initialize` → `initialized` handshake; returns the capabilities.
    fn handshake(&mut self) -> Parsed {
        let seq = self.request("initialize", "{}");
        let init = self.recv_response(seq);
        let caps = field(&init, "body").cloned().expect("initialize body");
        self.recv_event("initialized");
        caps
    }

    /// Launch a program written to a temp file; returns its path (kept alive for the run).
    fn launch(&mut self, tag: &str, src: &str, stop_on_entry: bool) -> std::path::PathBuf {
        let path = tmp_program(tag, src);
        let seq = self.request(
            "launch",
            &format!(
                r#"{{"program":{},"stopOnEntry":{stop_on_entry}}}"#,
                json::to_json_string(&Parsed::Str(path.to_string_lossy().into_owned()))
            ),
        );
        let resp = self.recv_response(seq);
        assert_eq!(
            field_bool(&resp, "success"),
            Some(true),
            "launch succeeds: {resp:?}"
        );
        path
    }

    fn configuration_done(&mut self) {
        let seq = self.request("configurationDone", "{}");
        let resp = self.recv_response(seq);
        assert_eq!(field_bool(&resp, "success"), Some(true), "configDone ok");
    }

    fn disconnect(&mut self) {
        let seq = self.request("disconnect", "{}");
        let _ = self.recv_response(seq);
        let _ = self.child.wait();
    }

    /// The current stop's `Locals` scope `variablesReference` (via stackTrace → scopes).
    fn current_locals_ref(&mut self) -> i64 {
        let st = self.request("stackTrace", r#"{"threadId":1}"#);
        let _ = self.recv_response(st);
        let sc = self.request("scopes", r#"{"frameId":1000}"#);
        let scopes = field_arr(field(&self.recv_response(sc), "body").unwrap(), "scopes")
            .expect("scopes")
            .clone();
        field_i64(&scopes[0], "variablesReference").expect("locals ref")
    }

    /// The `Locals` scope `variablesReference` for a specific stack frame.
    fn scope_ref(&mut self, frame_id: i64) -> i64 {
        let sc = self.request("scopes", &format!(r#"{{"frameId":{frame_id}}}"#));
        let scopes = field_arr(field(&self.recv_response(sc), "body").unwrap(), "scopes")
            .expect("scopes")
            .clone();
        field_i64(&scopes[0], "variablesReference").expect("scope ref")
    }

    /// The `stackFrames` array from a `stackTrace` request.
    fn stack_frames(&mut self) -> Vec<Parsed> {
        let st = self.request("stackTrace", r#"{"threadId":1}"#);
        field_arr(
            field(&self.recv_response(st), "body").unwrap(),
            "stackFrames",
        )
        .cloned()
        .unwrap_or_default()
    }

    /// The `variables` list for a reference.
    fn variables(&mut self, var_ref: i64) -> Vec<Parsed> {
        let seq = self.request(
            "variables",
            &format!(r#"{{"variablesReference":{var_ref}}}"#),
        );
        field_arr(
            field(&self.recv_response(seq), "body").unwrap(),
            "variables",
        )
        .cloned()
        .unwrap_or_default()
    }
}

/// Find a variable by name in a `variables` list (panics if absent).
fn var<'a>(vars: &'a [Parsed], name: &str) -> &'a Parsed {
    vars.iter()
        .find(|v| field_str(v, "name").as_deref() == Some(name))
        .unwrap_or_else(|| panic!("no variable `{name}` in {vars:?}"))
}

// ── json field helpers ───────────────────────────────────────────────────────────────
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
fn field_i64(v: &Parsed, key: &str) -> Option<i64> {
    field(v, key).and_then(Parsed::as_i64)
}
fn field_bool(v: &Parsed, key: &str) -> Option<bool> {
    match field(v, key) {
        Some(Parsed::Bool(b)) => Some(*b),
        _ => None,
    }
}
fn field_arr<'a>(v: &'a Parsed, key: &str) -> Option<&'a Vec<Parsed>> {
    match field(v, key) {
        Some(Parsed::Array(a)) => Some(a),
        _ => None,
    }
}

/// A unique temp `.loft` path keyed by tag + pid (so parallel tests don't collide).
fn tmp_program(tag: &str, src: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("loft_dap_{tag}_{}.loft", std::process::id()));
    std::fs::write(&p, src).expect("write temp program");
    p
}

// ── D1 — transport skeleton + handshake ──────────────────────────────────────────────
// initialize → capabilities response (correct request_seq) → `initialized` event, in
// that STRICT order.  The inline positive control proves the harness can see a FAILURE:
// an unsupported request comes back `success:false`, not a spurious success.
#[test]
fn handshake_advertises_capabilities_then_initialized() {
    let mut d = Dap::start();

    let seq = d.request("initialize", "{}");
    let init = d.recv_response(seq);
    assert_eq!(
        field_str(&init, "type").as_deref(),
        Some("response"),
        "a response envelope"
    );
    assert_eq!(
        field_i64(&init, "request_seq"),
        Some(seq),
        "echoes the request seq as request_seq"
    );
    assert_eq!(field_bool(&init, "success"), Some(true));
    let caps = field(&init, "body").expect("capabilities body");
    assert_eq!(
        field_bool(caps, "supportsConfigurationDoneRequest"),
        Some(true),
        "advertises configurationDone (the launch sequencing hinge): {caps:?}"
    );
    assert_eq!(
        field_bool(caps, "supportsConditionalBreakpoints"),
        Some(true),
        "advertises conditional breakpoints"
    );
    assert_eq!(
        field_bool(caps, "supportsStepBack"),
        Some(true),
        "advertises reverse stepping (RX): {caps:?}"
    );

    // The `initialized` event follows the response (never before it).
    let m = d.recv();
    assert_eq!(field_str(&m, "type").as_deref(), Some("event"));
    assert_eq!(field_str(&m, "event").as_deref(), Some("initialized"));

    // Positive control — the harness can tell failure from success.
    let bad = d.request("bogusRequest", "{}");
    let resp = d.recv_response(bad);
    assert_eq!(
        field_bool(&resp, "success"),
        Some(false),
        "an unsupported request is an ERROR response, not a success: {resp:?}"
    );

    d.disconnect();
}

// ── D2 — launch + run to termination + streamed output ───────────────────────────────
// A trivial program's `print` surfaces as an `output` event, then `terminated` (preceded
// by the synthesized `exited`).  The first real program under the adapter.
#[test]
fn launch_runs_to_termination_and_streams_output() {
    let mut d = Dap::start();
    d.handshake();
    let path = d.launch("run", "fn main() {\n  print(\"hello-dap\")\n}\n", false);

    d.configuration_done();
    let out = d.recv_event("output");
    assert!(
        field_str(&out, "output")
            .unwrap_or_default()
            .contains("hello-dap"),
        "the program's print is an output event: {out:?}"
    );
    assert_eq!(
        field_str(&out, "category").as_deref(),
        Some("stdout"),
        "program output rides the stdout category"
    );

    // `exited` (process model) precedes `terminated` (session ended).
    let exited = d.recv_event("exited");
    assert_eq!(field_i64(&exited, "exitCode"), Some(0));
    let _ = d.recv_event("terminated");

    d.disconnect();
    let _ = std::fs::remove_file(&path);
}

// ── D2 — stopOnEntry pauses before the first statement ───────────────────────────────
#[test]
fn stop_on_entry_pauses_at_entry() {
    let mut d = Dap::start();
    d.handshake();
    let path = d.launch(
        "entry",
        "fn main() {\n  a = 1;\n  print(\"a={a}\")\n}\n",
        true,
    );

    d.configuration_done();
    let stopped = d.recv_event("stopped");
    assert_eq!(
        field_str(&stopped, "reason").as_deref(),
        Some("entry"),
        "stopOnEntry yields a stop with reason `entry`: {stopped:?}"
    );
    assert_eq!(field_i64(&stopped, "threadId"), Some(1));

    d.disconnect();
    let _ = std::fs::remove_file(&path);
}

// ── D3 — breakpoints + stopped ───────────────────────────────────────────────────────
// A live line verifies and stops the run at the right function; a line with no breakable
// code comes back `verified:false` (a dead breakpoint the client hears about now).
#[test]
fn breakpoint_verifies_and_stops() {
    let mut d = Dap::start();
    d.handshake();
    // helper's body is line 2; main calls it.
    let src = "fn helper(n: integer) -> integer {\n  n * 2\n}\nfn main() {\n  a = helper(21);\n  print(\"a={a}\")\n}\n";
    let path = d.launch("bp", src, false);

    // Breakpoint on line 2 (live) + line 99 (dead).
    let file = json::to_json_string(&Parsed::Str(path.to_string_lossy().into_owned()));
    let seq = d.request(
        "setBreakpoints",
        &format!(r#"{{"source":{{"path":{file}}},"breakpoints":[{{"line":2}},{{"line":99}}]}}"#),
    );
    let resp = d.recv_response(seq);
    let bps = field_arr(field(&resp, "body").unwrap(), "breakpoints").expect("breakpoints array");
    assert_eq!(
        (
            field_bool(&bps[0], "verified"),
            field_bool(&bps[1], "verified")
        ),
        (Some(true), Some(false)),
        "line 2 verifies, line 99 does not: {bps:?}"
    );

    d.configuration_done();
    let stopped = d.recv_event("stopped");
    assert_eq!(
        field_str(&stopped, "reason").as_deref(),
        Some("breakpoint"),
        "stops for a breakpoint: {stopped:?}"
    );

    // The stop is inside `helper` — confirmed via the stackTrace frame.
    let st = d.request("stackTrace", r#"{"threadId":1}"#);
    let frames = field_arr(field(&d.recv_response(st), "body").unwrap(), "stackFrames")
        .expect("stackFrames")
        .clone();
    assert_eq!(
        field_str(&frames[0], "name").as_deref(),
        Some("helper"),
        "the frame is `helper`: {frames:?}"
    );
    assert_eq!(field_i64(&frames[0], "line"), Some(2), "parked on line 2");

    d.disconnect();
    let _ = std::fs::remove_file(&path);
}

// ── D4 — inspection drill-down + stale reference ─────────────────────────────────────
// At a stop, walk threads → stackTrace → scopes → variables and assert the locals-panel
// content; after a resume, the prior `variablesReference` returns an empty list.
#[test]
fn drilldown_reads_locals_and_invalidates_stale_reference() {
    let mut d = Dap::start();
    d.handshake();
    // `a` is assigned on line 2, so at the line-3 stop it is a live local.
    let src = "fn main() {\n  a = 21;\n  b = a + 1;\n  print(\"b={b}\")\n}\n";
    let path = d.launch("drill", src, false);

    let file = json::to_json_string(&Parsed::Str(path.to_string_lossy().into_owned()));
    let seq = d.request(
        "setBreakpoints",
        &format!(r#"{{"source":{{"path":{file}}},"breakpoints":[{{"line":3}}]}}"#),
    );
    let _ = d.recv_response(seq);

    d.configuration_done();
    let _ = d.recv_event("stopped");

    // threads → one synthetic main thread.
    let t = d.request("threads", "{}");
    let threads = field_arr(field(&d.recv_response(t), "body").unwrap(), "threads")
        .expect("threads")
        .clone();
    assert_eq!(field_i64(&threads[0], "id"), Some(1));

    // stackTrace → frameId; scopes → the Locals scope reference; variables → the locals.
    let st = d.request("stackTrace", r#"{"threadId":1}"#);
    let frames = field_arr(field(&d.recv_response(st), "body").unwrap(), "stackFrames")
        .expect("stackFrames")
        .clone();
    let frame_id = field_i64(&frames[0], "id").expect("frameId");

    let sc = d.request("scopes", &format!(r#"{{"frameId":{frame_id}}}"#));
    let scopes = field_arr(field(&d.recv_response(sc), "body").unwrap(), "scopes")
        .expect("scopes")
        .clone();
    assert_eq!(field_str(&scopes[0], "name").as_deref(), Some("Locals"));
    let locals_ref = field_i64(&scopes[0], "variablesReference").expect("locals ref");

    let vr = d.request(
        "variables",
        &format!(r#"{{"variablesReference":{locals_ref}}}"#),
    );
    let vars = field_arr(field(&d.recv_response(vr), "body").unwrap(), "variables")
        .expect("variables")
        .clone();
    let a = vars
        .iter()
        .find(|v| field_str(v, "name").as_deref() == Some("a"))
        .expect("local `a` is present");
    assert_eq!(
        field_str(a, "value").as_deref(),
        Some("21"),
        "local a == 21: {vars:?}"
    );
    assert_eq!(
        field_i64(a, "variablesReference"),
        Some(0),
        "a scalar local is a leaf (ref 0)"
    );

    // Resume, then read the OLD reference — it is stale → an empty list.
    let cont = d.request("continue", r#"{"threadId":1}"#);
    let _ = d.recv_response(cont);
    let _ = d.recv_event("terminated");
    let vr2 = d.request(
        "variables",
        &format!(r#"{{"variablesReference":{locals_ref}}}"#),
    );
    let vars2 = field_arr(field(&d.recv_response(vr2), "body").unwrap(), "variables")
        .expect("variables")
        .clone();
    assert!(
        vars2.is_empty(),
        "a stale reference after resume returns empty: {vars2:?}"
    );

    d.disconnect();
    let _ = std::fs::remove_file(&path);
}

// ── D5 — stepping + continue ─────────────────────────────────────────────────────────
// Step-over from one line lands on the next (reason `step`); continue then runs to
// termination.
#[test]
fn step_over_advances_then_continue_terminates() {
    let mut d = Dap::start();
    d.handshake();
    let src = "fn main() {\n  a = 1;\n  b = 2;\n  print(\"{a}{b}\")\n}\n";
    let path = d.launch("step", src, false);

    let file = json::to_json_string(&Parsed::Str(path.to_string_lossy().into_owned()));
    let seq = d.request(
        "setBreakpoints",
        &format!(r#"{{"source":{{"path":{file}}},"breakpoints":[{{"line":2}}]}}"#),
    );
    let _ = d.recv_response(seq);

    d.configuration_done();
    let first = d.recv_event("stopped");
    assert_eq!(field_str(&first, "reason").as_deref(), Some("breakpoint"));

    // Step over → the next line, reason `step`.
    let step = d.request("next", r#"{"threadId":1}"#);
    let _ = d.recv_response(step);
    let stepped = d.recv_event("stopped");
    assert_eq!(
        field_str(&stepped, "reason").as_deref(),
        Some("step"),
        "a step stop: {stepped:?}"
    );
    let st = d.request("stackTrace", r#"{"threadId":1}"#);
    let frames = field_arr(field(&d.recv_response(st), "body").unwrap(), "stackFrames")
        .expect("stackFrames")
        .clone();
    assert_eq!(
        field_i64(&frames[0], "line"),
        Some(3),
        "step-over advanced to line 3: {frames:?}"
    );

    // Continue → runs to termination.
    let cont = d.request("continue", r#"{"threadId":1}"#);
    let _ = d.recv_response(cont);
    let _ = d.recv_event("terminated");

    d.disconnect();
    let _ = std::fs::remove_file(&path);
}

// ── D6 — evaluate + setVariable ──────────────────────────────────────────────────────
// Evaluate an expression against the paused frame; set a variable and see the change in
// the next `variables` read.
#[test]
fn evaluate_and_set_variable_at_a_stop() {
    let mut d = Dap::start();
    d.handshake();
    let src = "fn main() {\n  a = 21;\n  b = a + 1;\n  print(\"b={b}\")\n}\n";
    let path = d.launch("eval", src, false);

    let file = json::to_json_string(&Parsed::Str(path.to_string_lossy().into_owned()));
    let seq = d.request(
        "setBreakpoints",
        &format!(r#"{{"source":{{"path":{file}}},"breakpoints":[{{"line":3}}]}}"#),
    );
    let _ = d.recv_response(seq);

    d.configuration_done();
    let _ = d.recv_event("stopped");

    // evaluate `a + 100` in the frame → 121.
    let ev = d.request("evaluate", r#"{"expression":"a + 100","context":"repl"}"#);
    let body = field(&d.recv_response(ev), "body")
        .cloned()
        .expect("evaluate body");
    assert_eq!(
        field_str(&body, "result").as_deref(),
        Some("121"),
        "evaluate a + 100 == 121: {body:?}"
    );

    // setVariable a = 5, then read variables → a is now 5.
    let sv = d.request(
        "setVariable",
        r#"{"variablesReference":2001,"name":"a","value":"5"}"#,
    );
    let sv_body = field(&d.recv_response(sv), "body")
        .cloned()
        .expect("setVariable body");
    assert_eq!(
        field_str(&sv_body, "value").as_deref(),
        Some("5"),
        "setVariable echoes the new value: {sv_body:?}"
    );

    // Fetch the current Locals reference and confirm the edit is reflected.
    let sc = d.request("scopes", r#"{"frameId":1000}"#);
    let scopes = field_arr(field(&d.recv_response(sc), "body").unwrap(), "scopes")
        .expect("scopes")
        .clone();
    let locals_ref = field_i64(&scopes[0], "variablesReference").unwrap();
    let vr = d.request(
        "variables",
        &format!(r#"{{"variablesReference":{locals_ref}}}"#),
    );
    let vars = field_arr(field(&d.recv_response(vr), "body").unwrap(), "variables")
        .expect("variables")
        .clone();
    let a = vars
        .iter()
        .find(|v| field_str(v, "name").as_deref() == Some("a"))
        .expect("local `a`");
    assert_eq!(
        field_str(a, "value").as_deref(),
        Some("5"),
        "the edited value is reflected in the next variables read: {vars:?}"
    );

    d.disconnect();
    let _ = std::fs::remove_file(&path);
}

// ── VE — structured variable expansion (DAP_ADVANCED.md § VE) ─────────────────────────
// A scalar local stays a leaf (ref 0); a struct / vector local carries an expansion
// handle whose children are exactly its fields / elements; a vector-of-structs expands
// two levels (VE2); a handle from a prior stop is invalidated on resume (VE3).
//
// The reliable path is a struct value shown BY NAME (`sq`), expanded through its evaluated
// JSON tree — its vector field + nested struct field drill down without depending on the
// frame's naming of bare heap locals (which shows a top-level `vector` under its `__vdb`
// backing — a `frame_field` fidelity limit, not VE; see DAP_ADVANCED.md § VE).
#[test]
fn variable_expansion_walks_structs_vectors_and_nesting() {
    let mut d = Dap::start();
    d.handshake();
    // At the line-6 stop, n + sq are live (total is the stop line, not yet set).
    let src = "struct Point { x: integer, y: integer }\n\
               struct Squad { members: vector<integer>, lead: Point }\n\
               fn main() {\n\
              \x20 n = 7;\n\
              \x20 sq = Squad { members: [10, 20, 30], lead: Point { x: 9, y: 2 } };\n\
              \x20 total = n + sq.lead.x;\n\
              \x20 print(\"total={total}\")\n\
               }\n";
    let path = d.launch("ve", src, false);

    let file = json::to_json_string(&Parsed::Str(path.to_string_lossy().into_owned()));
    let seq = d.request(
        "setBreakpoints",
        &format!(r#"{{"source":{{"path":{file}}},"breakpoints":[{{"line":6}}]}}"#),
    );
    let _ = d.recv_response(seq);
    d.configuration_done();
    let _ = d.recv_event("stopped");

    let lref = d.current_locals_ref();
    let locals = d.variables(lref);

    // VE0 — a scalar local is a leaf (ref 0), value from the flat frame.
    let n = var(&locals, "n");
    assert_eq!(
        field_i64(n, "variablesReference"),
        Some(0),
        "scalar `n` is a leaf"
    );
    assert_eq!(field_str(n, "value").as_deref(), Some("7"));

    // VE0 — the struct local carries a non-zero expansion handle.
    let sq_ref = field_i64(var(&locals, "sq"), "variablesReference").expect("sq ref");
    assert!(sq_ref != 0, "struct `sq` is expandable");

    // VE1 — expanding the struct yields exactly its fields; a struct/vector field is itself
    // expandable, a scalar field is a leaf.
    let fields = d.variables(sq_ref);
    assert_eq!(fields.len(), 2, "Squad has two fields: {fields:?}");
    let members_ref =
        field_i64(var(&fields, "members"), "variablesReference").expect("members ref");
    let lead_ref = field_i64(var(&fields, "lead"), "variablesReference").expect("lead ref");
    assert!(
        members_ref != 0 && lead_ref != 0,
        "both fields are expandable"
    );

    // VE1 — the vector field expands to its elements as `[i]`, in order.
    let elems = d.variables(members_ref);
    let elem_vals: Vec<Option<String>> = ["[0]", "[1]", "[2]"]
        .iter()
        .map(|i| field_str(var(&elems, i), "value"))
        .collect();
    assert_eq!(
        elem_vals,
        vec![Some("10".into()), Some("20".into()), Some("30".into())],
        "vector elements in order: {elems:?}"
    );
    assert_eq!(
        field_i64(var(&elems, "[0]"), "variablesReference"),
        Some(0),
        "a scalar element is a leaf"
    );

    // VE2 — two-level nesting through the CACHED tree (sq → lead → x/y), no re-eval (a
    // direct `eval sq.lead.x` isn't needed — the values come from sq's cached JSON).
    let lead = d.variables(lead_ref);
    assert_eq!(field_str(var(&lead, "x"), "value").as_deref(), Some("9"));
    assert_eq!(field_str(var(&lead, "y"), "value").as_deref(), Some("2"));

    // VE3 — after a resume the prior handle is stale → empty, never a wrong subtree.
    let cont = d.request("continue", r#"{"threadId":1}"#);
    let _ = d.recv_response(cont);
    let _ = d.recv_event("terminated");
    assert!(
        d.variables(sq_ref).is_empty(),
        "a stale expansion handle after resume returns empty"
    );

    d.disconnect();
    let _ = std::fs::remove_file(&path);
}

// ── SF — multi-frame stack trace (DAP_ADVANCED.md § SF) ──────────────────────────────
// A breakpoint three calls deep shows the whole runtime stack innermost-first (the
// synthetic `replmain_*` entry wrapper filtered out), each frame at its parked / call-site
// line; the top frame's locals expand (VE), and a CALLER frame's locals are readable.
#[test]
fn stack_trace_walks_all_frames_and_reads_caller_locals() {
    let mut d = Dap::start();
    d.handshake();
    let src = "fn inner(z: integer) -> integer {\n  w = z * 2;\n  w\n}\n\
               fn middle(y: integer) -> integer {\n  m = inner(y + 1);\n  m\n}\n\
               fn main() {\n  a = 10;\n  r = middle(a);\n  print(\"r={r}\")\n}\n";
    let path = d.launch("sf", src, false);

    // Break at line 2 (inner's body) — the stack is inner ← middle ← main.
    let file = json::to_json_string(&Parsed::Str(path.to_string_lossy().into_owned()));
    let seq = d.request(
        "setBreakpoints",
        &format!(r#"{{"source":{{"path":{file}}},"breakpoints":[{{"line":2}}]}}"#),
    );
    let _ = d.recv_response(seq);
    d.configuration_done();
    let _ = d.recv_event("stopped");

    let frames = d.stack_frames();
    // Three USER frames, innermost first; the `replmain_*` wrapper is dropped.
    let names: Vec<Option<String>> = frames.iter().map(|f| field_str(f, "name")).collect();
    assert_eq!(
        names,
        vec![
            Some("inner".into()),
            Some("middle".into()),
            Some("main".into())
        ],
        "innermost-first user frames, wrapper filtered: {frames:?}"
    );
    // SF4 — each frame at its line: the breakpoint line (top) and the call sites (callers).
    let lines: Vec<Option<i64>> = frames.iter().map(|f| field_i64(f, "line")).collect();
    assert_eq!(
        lines,
        vec![Some(2), Some(6), Some(11)],
        "breakpoint line + call-site lines: {frames:?}"
    );
    // Distinct frame ids so scopes/variables can target any frame.
    let ids: Vec<i64> = frames.iter().filter_map(|f| field_i64(f, "id")).collect();
    assert_eq!(ids.len(), 3);
    assert!(
        ids[0] != ids[1] && ids[1] != ids[2],
        "distinct frameIds: {ids:?}"
    );

    // Top frame (inner): its locals read, z == 11 (10 + 1).
    let top_ref = d.scope_ref(ids[0]);
    let top_vars = d.variables(top_ref);
    assert_eq!(
        field_str(var(&top_vars, "z"), "value").as_deref(),
        Some("11"),
        "top frame `inner` z == 11: {top_vars:?}"
    );

    // SF — a CALLER frame's locals are readable: `middle`'s y == 10.
    let mid_ref = d.scope_ref(ids[1]);
    let mid_vars = d.variables(mid_ref);
    assert_eq!(
        field_str(var(&mid_vars, "y"), "value").as_deref(),
        Some("10"),
        "caller frame `middle` y == 10: {mid_vars:?}"
    );

    d.disconnect();
    let _ = std::fs::remove_file(&path);
}

// ── DB — data breakpoints via watchpoints (DAP_ADVANCED.md § DB) ──────────────────────
// The adapter advertises data breakpoints; `dataBreakpointInfo` offers a watchable local
// as a `dataId` (and refuses an unwatchable container with `null`); `setDataBreakpoints`
// verifies it, and a later change to the local fires `stopped{reason:"data breakpoint"}`.
#[test]
fn data_breakpoint_on_local_fires_on_change() {
    let mut d = Dap::start();
    let caps = d.handshake();
    assert_eq!(
        field_bool(&caps, "supportsDataBreakpoints"),
        Some(true),
        "advertises data breakpoints: {caps:?}"
    );
    let src = "fn main() {\n  x = 1;\n  x = 2;\n  x = 3;\n  print(\"x={x}\")\n}\n";
    let path = d.launch("db", src, false);

    // Break at line 3 (x == 1 assigned).
    let file = json::to_json_string(&Parsed::Str(path.to_string_lossy().into_owned()));
    let seq = d.request(
        "setBreakpoints",
        &format!(r#"{{"source":{{"path":{file}}},"breakpoints":[{{"line":3}}]}}"#),
    );
    let _ = d.recv_response(seq);
    d.configuration_done();
    let _ = d.recv_event("stopped");

    let lref = d.current_locals_ref();

    // dataBreakpointInfo on the local `x` → a dataId + write access.
    let info = d.request(
        "dataBreakpointInfo",
        &format!(r#"{{"variablesReference":{lref},"name":"x"}}"#),
    );
    let info_body = field(&d.recv_response(info), "body")
        .cloned()
        .expect("info body");
    assert_eq!(
        field_str(&info_body, "dataId").as_deref(),
        Some("x"),
        "dataId is the watch expression: {info_body:?}"
    );

    // An unknown container → a null dataId (the client greys out the option).
    let bad = d.request(
        "dataBreakpointInfo",
        r#"{"variablesReference":999999,"name":"nope"}"#,
    );
    let bad_body = field(&d.recv_response(bad), "body")
        .cloned()
        .expect("info body");
    assert!(
        matches!(field(&bad_body, "dataId"), Some(Parsed::Null)),
        "an unwatchable container → null dataId: {bad_body:?}"
    );

    // setDataBreakpoints → verified.
    let set = d.request("setDataBreakpoints", r#"{"breakpoints":[{"dataId":"x"}]}"#);
    let set_body = field(&d.recv_response(set), "body")
        .cloned()
        .expect("set body");
    let bps = field_arr(&set_body, "breakpoints").expect("breakpoints");
    assert_eq!(
        field_bool(&bps[0], "verified"),
        Some(true),
        "the data breakpoint verifies: {set_body:?}"
    );

    // Continue → x changes → a stop with reason `data breakpoint`.
    let cont = d.request("continue", r#"{"threadId":1}"#);
    let _ = d.recv_response(cont);
    let stopped = d.recv_event("stopped");
    assert_eq!(
        field_str(&stopped, "reason").as_deref(),
        Some("data breakpoint"),
        "the change fires a data breakpoint: {stopped:?}"
    );

    d.disconnect();
    let _ = std::fs::remove_file(&path);
}

/// The top stack frame's current source line (via a fresh stackTrace).
fn top_line(d: &mut Dap) -> Option<i64> {
    d.stack_frames().first().and_then(|f| field_i64(f, "line"))
}

/// The current value of local `name` in the top frame (via scopes → variables).
fn local_val(d: &mut Dap, name: &str) -> Option<String> {
    let lref = d.current_locals_ref();
    d.variables(lref)
        .iter()
        .find(|v| field_str(v, "name").as_deref() == Some(name))
        .and_then(|v| field_str(v, "value"))
}

// ── RX5 — reverse stepping over DAP (DAP_ADVANCED.md § RX) ────────────────────────────
// stepBack reverses a forward step in the editor: after stepping over a mutating line, a
// stepBack returns the line AND the local to the exact prior stop.
#[test]
fn step_back_reverses_a_step_over_dap() {
    let mut d = Dap::start();
    let caps = d.handshake();
    assert_eq!(
        field_bool(&caps, "supportsStepBack"),
        Some(true),
        "advertises reverse stepping"
    );
    let src = "fn main() {\n  a = 1;\n  a = a + 1;\n  a = a + 2;\n  print(\"a={a}\")\n}\n";
    let path = d.launch("rxstep", src, false);
    let file = json::to_json_string(&Parsed::Str(path.to_string_lossy().into_owned()));
    let seq = d.request(
        "setBreakpoints",
        &format!(r#"{{"source":{{"path":{file}}},"breakpoints":[{{"line":3}}]}}"#),
    );
    let _ = d.recv_response(seq);
    d.configuration_done();
    let _ = d.recv_event("stopped");

    assert_eq!(top_line(&mut d), Some(3), "at the breakpoint line 3");
    assert_eq!(
        local_val(&mut d, "a").as_deref(),
        Some("1"),
        "a == 1 at the stop"
    );

    // Step forward → line 4, a == 2.
    let n = d.request("next", r#"{"threadId":1}"#);
    let _ = d.recv_response(n);
    let stepped = d.recv_event("stopped");
    assert_eq!(field_str(&stepped, "reason").as_deref(), Some("step"));
    assert_eq!(top_line(&mut d), Some(4), "advanced to line 4");
    assert_eq!(
        local_val(&mut d, "a").as_deref(),
        Some("2"),
        "a == 2 after the step"
    );

    // STEP BACK → line 3, a reverted to 1.
    let sb = d.request("stepBack", r#"{"threadId":1}"#);
    let _ = d.recv_response(sb);
    let back = d.recv_event("stopped");
    assert_eq!(
        field_str(&back, "reason").as_deref(),
        Some("step"),
        "stepBack is a step stop: {back:?}"
    );
    assert_eq!(top_line(&mut d), Some(3), "reverted to line 3");
    assert_eq!(
        local_val(&mut d, "a").as_deref(),
        Some("1"),
        "a reverted to its pre-step value 1"
    );

    d.disconnect();
    let _ = std::fs::remove_file(&path);
}

// reverseContinue reverses to the ring floor: from deep in the program it walks all retained
// checkpoints back to the earliest and reports one landing stop.
#[test]
fn reverse_continue_walks_back_to_the_floor() {
    let mut d = Dap::start();
    d.handshake();
    let src = "fn main() {\n  a = 1;\n  a = a + 1;\n  a = a + 2;\n  print(\"a={a}\")\n}\n";
    let path = d.launch("rxcont", src, false);
    let file = json::to_json_string(&Parsed::Str(path.to_string_lossy().into_owned()));
    let seq = d.request(
        "setBreakpoints",
        &format!(r#"{{"source":{{"path":{file}}},"breakpoints":[{{"line":3}}]}}"#),
    );
    let _ = d.recv_response(seq);
    d.configuration_done();
    let _ = d.recv_event("stopped"); // line 3, a == 1

    // Step forward twice → line 5, a == 4 (checkpoints for lines 3 and 4 accumulate).
    for _ in 0..2 {
        let n = d.request("next", r#"{"threadId":1}"#);
        let _ = d.recv_response(n);
        let _ = d.recv_event("stopped");
    }
    assert_eq!(top_line(&mut d), Some(5), "stepped to line 5");
    assert_eq!(
        local_val(&mut d, "a").as_deref(),
        Some("4"),
        "a == 4 at line 5"
    );

    // reverseContinue → all the way back to the earliest retained stop (line 3, a == 1).
    let rc = d.request("reverseContinue", r#"{"threadId":1}"#);
    let _ = d.recv_response(rc);
    let landed = d.recv_event("stopped");
    assert_eq!(field_str(&landed, "reason").as_deref(), Some("step"));
    assert_eq!(
        top_line(&mut d),
        Some(3),
        "reversed to the ring floor (line 3)"
    );
    assert_eq!(
        local_val(&mut d, "a").as_deref(),
        Some("1"),
        "a reversed all the way back to 1"
    );

    d.disconnect();
    let _ = std::fs::remove_file(&path);
}

// ── pause — an interrupt reaches a RUNNING program ───────────────────────────────────────
// `pause` arrives while `continue` runs, on a thread the adapter keeps for reading, so the
// run stops where it is with reason `pause`.  The response comes BEFORE the `stopped` event
// (DAP's order), the run then continues to its own end with the right answer (the interrupt
// was consumed, not left to fire again), and a `pause` sent while the program is already
// stopped is answered and changes nothing: the next `continue` runs to the end.
#[test]
fn pause_stops_a_running_program_and_continue_resumes_it() {
    const SRC: &str = "fn main() {\n  n = 0;\n  for i in 0..20000000 { n += i % 7; }\n  println(\"done {n}\");\n}\n";
    // sum(i % 7 for i in 0..20_000_000), computed outside loft.
    const DONE: &str = "done 59999997";

    // 1. A pause during the run.
    let mut d = Dap::start();
    d.handshake();
    let path = d.launch("pause_run", SRC, false);
    d.configuration_done();
    std::thread::sleep(std::time::Duration::from_millis(300));
    let pause = d.request("pause", r#"{"threadId":1}"#);
    let mut order = Vec::new();
    let mut stopped = None;
    for _ in 0..64 {
        let m = d.recv();
        match (
            field_str(&m, "type").as_deref(),
            field_str(&m, "event").as_deref(),
        ) {
            (Some("response"), _) if field_i64(&m, "request_seq") == Some(pause) => {
                assert_eq!(
                    field_bool(&m, "success"),
                    Some(true),
                    "pause is answered: {m:?}"
                );
                order.push("response");
            }
            (Some("event"), Some("stopped")) => {
                stopped = field(&m, "body").cloned();
                order.push("stopped");
            }
            (Some("event"), Some("terminated")) => {
                panic!("the run ended before the pause: {order:?}")
            }
            _ => {}
        }
        if order.len() == 2 {
            break;
        }
    }
    assert_eq!(
        order,
        ["response", "stopped"],
        "the response precedes the stopped event"
    );
    let stopped = stopped.expect("a stopped event");
    assert_eq!(
        field_str(&stopped, "reason").as_deref(),
        Some("pause"),
        "{stopped:?}"
    );

    // 2. Continue after the pause: the run resumes and ends with the right answer.
    let cont = d.request("continue", r#"{"threadId":1}"#);
    d.recv_response(cont);
    let mut out = String::new();
    for _ in 0..64 {
        let m = d.recv();
        match field_str(&m, "event").as_deref() {
            Some("output") => {
                out.push_str(
                    &field(&m, "body")
                        .and_then(|b| field_str(b, "output"))
                        .unwrap_or_default(),
                );
            }
            Some("stopped") => panic!("the consumed pause fired again: {m:?}"),
            Some("terminated") => break,
            _ => {}
        }
    }
    assert!(
        out.contains(DONE),
        "the resumed run finishes with its own answer: {out:?}"
    );
    d.disconnect();
    let _ = std::fs::remove_file(&path);

    // 3. A pause while already stopped changes nothing.
    let mut d = Dap::start();
    d.handshake();
    let path = d.launch("pause_stopped", SRC, true);
    d.configuration_done();
    let entry = d.recv_event("stopped");
    assert_eq!(field_str(&entry, "reason").as_deref(), Some("entry"));
    let pause = d.request("pause", r#"{"threadId":1}"#);
    assert_eq!(field_bool(&d.recv_response(pause), "success"), Some(true));
    let cont = d.request("continue", r#"{"threadId":1}"#);
    d.recv_response(cont);
    let mut out = String::new();
    for _ in 0..64 {
        let m = d.recv();
        match field_str(&m, "event").as_deref() {
            Some("output") => {
                out.push_str(
                    &field(&m, "body")
                        .and_then(|b| field_str(b, "output"))
                        .unwrap_or_default(),
                );
            }
            Some("stopped") => panic!("a pause sent while stopped fired on the next run: {m:?}"),
            Some("terminated") => break,
            _ => {}
        }
    }
    assert!(out.contains(DONE), "{out:?}");
    d.disconnect();
    let _ = std::fs::remove_file(&path);
}
