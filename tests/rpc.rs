// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN16 M5d phase 2 — the `--rpc` debug server, driven over an in-memory pipe.
//!
//! Sends NDJSON requests through `rpc::run_rpc` and asserts the NDJSON responses +
//! events: launch → setBreakpoints → run (→ `stopped` in the right frame) → eval
//! (JSON value) → continue (→ program `output` + `terminated`).  This is the surface
//! an agent / CI drives the debugger through.

use std::io::Cursor;

/// A unique temp path for a `.loft` program, keyed by tag + pid.
fn tmp_program(tag: &str, src: &str) -> std::path::PathBuf {
    let p = std::env::temp_dir().join(format!("loft_rpc_{tag}_{}.loft", std::process::id()));
    std::fs::write(&p, src).expect("write temp program");
    p
}

/// A path as a JSON-string-safe value: Windows separators (`C:\Users\…`) are JSON escape
/// characters and made every request unparseable on the Windows runner (`invalid escape \U`).
fn json_path(p: &std::path::Path) -> String {
    p.to_string_lossy().replace('\\', "\\\\")
}

/// Run the RPC server over `requests` (joined by newlines) and return the output text.
fn drive(requests: &[String]) -> String {
    let input = Cursor::new(requests.join("\n").into_bytes());
    let mut out: Vec<u8> = Vec::new();
    loft::rpc::run_rpc("default", &[], input, &mut out).expect("rpc run");
    String::from_utf8(out).expect("utf8")
}

#[test]
fn rpc_launch_break_eval_continue() {
    // helper's body is line 2; main calls it then prints.
    let path = tmp_program(
        "basic",
        "fn helper(n: integer) -> integer {\n  n * 2\n}\nfn main() {\n  a = helper(21);\n  print(\"a={a}\")\n}\n",
    );
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":2}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        "{\"id\":4,\"req\":\"eval\",\"expr\":\"n\"}".to_string(),
        "{\"id\":5,\"req\":\"continue\"}".to_string(),
        "{\"id\":6,\"req\":\"disconnect\"}".to_string(),
    ]);

    // Every output line is a JSON object.
    for line in out.lines() {
        assert!(
            line.starts_with('{') && line.ends_with('}'),
            "not JSON: {line}"
        );
    }
    assert!(out.contains("\"id\":1,\"ok\":true"), "launch ok: {out}");
    // run → stopped at the breakpoint, inside `helper`.
    assert!(out.contains("\"event\":\"stopped\""), "a stop event: {out}");
    assert!(
        out.contains("\"function\":\"helper\""),
        "stopped in helper: {out}"
    );
    // eval `n` against the frame → the JSON value 21 (helper's argument).
    assert!(
        out.contains("\"id\":4,\"ok\":true,\"value\":21"),
        "eval n == 21: {out}"
    );
    // continue → the program prints, captured as an `output` event, then terminates.
    assert!(
        out.contains("\"category\":\"stdout\",\"text\":\"a=42\""),
        "program output captured: {out}"
    );
    assert!(
        out.contains("\"event\":\"terminated\""),
        "terminated: {out}"
    );

    let _ = std::fs::remove_file(&path);
}

// @PLN98 P1 — `eval`/`setValue` must work in a frame that HOLDS a heap (vector) local. Before the
// fix, a vector local's compiler backing (`__vdb_N`, rendered `main_vector<…>{…}`, not loft source)
// was seeded into the reconstruct-eval prefix and poisoned the WHOLE parse, so EVERY expression —
// even `2 + 2`, which uses no local — returned `value:null`, and `setValue` was "edit rejected".
// The fix seeds only the locals the expression names, heap ones rendered live+unbounded.
#[test]
fn rpc_eval_and_set_in_a_vector_local_frame() {
    let path = tmp_program(
        "veclocal",
        "fn main() {\n  v: vector<integer> = [10, 20, 30];\n  x = 5;\n  print(\"len={len(v)} x={x}\")\n}\n",
    );
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":4}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        // A literal that references NO local — used to return null merely because the frame held `v`.
        "{\"id\":4,\"req\":\"eval\",\"expr\":\"2 + 2\"}".to_string(),
        // Expressions that DO reference the vector local — live read, unbounded.
        "{\"id\":5,\"req\":\"eval\",\"expr\":\"len(v)\"}".to_string(),
        "{\"id\":6,\"req\":\"eval\",\"expr\":\"v[1] + x\"}".to_string(),
        // setValue was "edit rejected" in a vector frame; now it edits the live run.
        "{\"id\":7,\"req\":\"setValue\",\"target\":\"x\",\"value\":\"42\"}".to_string(),
        "{\"id\":8,\"req\":\"continue\"}".to_string(),
        "{\"id\":9,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(
        out.contains("\"id\":4,\"ok\":true,\"value\":4"),
        "eval 2+2 == 4: {out}"
    );
    assert!(
        out.contains("\"id\":5,\"ok\":true,\"value\":3"),
        "eval len(v) == 3: {out}"
    );
    assert!(
        out.contains("\"id\":6,\"ok\":true,\"value\":25"),
        "eval v[1]+x == 25: {out}"
    );
    assert!(
        out.contains("\"id\":7,\"ok\":true"),
        "setValue x=42 accepted: {out}"
    );
    assert!(
        out.contains("\"category\":\"stdout\",\"text\":\"len=3 x=42\""),
        "continue prints the edited x: {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// @PLN98 P1b — the true live-frame eval: an expression that REFERENCES a keyed-collection
// (`hash`) local. The reconstruct/text-seed path can't seed a hash (it renders as the
// non-reparseable `<…>` catch-all), so P1 returned a graceful `null` here. The fix binds the
// hash as a typed argument of a synthetic eval fn and passes its live `DbRef` into a
// `reenter_ret` over the paused frame — reading the collection where it lives. The literal
// `2 + 2` (no local) must still go through the untouched text path (no regression).
#[test]
fn rpc_eval_in_a_keyed_collection_frame() {
    let path = tmp_program(
        "hashlocal",
        "struct HRec { name: text, v: integer }\n\
         fn main() {\n\
        \x20 h: hash<HRec[name]> = [];\n\
        \x20 h += [HRec{name: \"a\", v: 7}];\n\
        \x20 h += [HRec{name: \"b\", v: 9}];\n\
        \x20 x = 5;\n\
        \x20 r = h[\"a\"].v;\n\
        \x20 print(\"r={r} x={x}\")\n\
         }\n",
    );
    let file = json_path(&path);
    // In the JSON request, a `"` inside the expr is escaped `\"`; from Rust source that
    // is `\\\"`. Line 7 (`r = h["a"].v;`) is where h (2 entries) and x (5) are both live.
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":7}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        // A literal referencing NO local — must still work through the text path.
        "{\"id\":4,\"req\":\"eval\",\"expr\":\"2 + 2\"}".to_string(),
        // Keyed-local expressions — live-arg path (was `null` before P1b).
        "{\"id\":5,\"req\":\"eval\",\"expr\":\"h[\\\"a\\\"].v\"}".to_string(),
        "{\"id\":6,\"req\":\"eval\",\"expr\":\"h[\\\"b\\\"].v + x\"}".to_string(),
        "{\"id\":7,\"req\":\"eval\",\"expr\":\"len(h)\"}".to_string(),
        // The whole element struct → JSON object via the live DbRef → to_json.
        "{\"id\":8,\"req\":\"eval\",\"expr\":\"h[\\\"a\\\"]\"}".to_string(),
        // …and the MISS, which is the half a discharge would get wrong.  `@FR-Col-Lookup`
        // makes a keyed point lookup `τ?`, so the whole-element eval has to distinguish
        // "absent" from "present": discharging with `?` compiles and renders the element
        // type's ZERO RECORD for a key that is not there — a debugger inventing a value,
        // which is worse than the refusal it replaces.  Without this cell that cure passes.
        "{\"id\":11,\"req\":\"eval\",\"expr\":\"h[\\\"zz\\\"]\"}".to_string(),
        "{\"id\":9,\"req\":\"continue\"}".to_string(),
        "{\"id\":10,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(
        out.contains("\"id\":4,\"ok\":true,\"value\":4"),
        "eval 2+2 == 4 (text path unaffected): {out}"
    );
    assert!(
        out.contains("\"id\":5,\"ok\":true,\"value\":7"),
        "eval h[\"a\"].v == 7 (live keyed read): {out}"
    );
    assert!(
        out.contains("\"id\":6,\"ok\":true,\"value\":14"),
        "eval h[\"b\"].v + x == 14 (keyed + scalar): {out}"
    );
    assert!(
        out.contains("\"id\":7,\"ok\":true,\"value\":2"),
        "eval len(h) == 2: {out}"
    );
    assert!(
        out.contains("\"id\":8,\"ok\":true,\"value\":{\"name\":\"a\",\"v\":7}"),
        "eval h[\"a\"] == the element struct: {out}"
    );
    assert!(
        out.contains("\"id\":11,\"ok\":true,\"value\":null"),
        "eval h[\"zz\"] == null (an ABSENT key is absent, not a zero record): {out}"
    );
    assert!(
        out.contains("\"category\":\"stdout\",\"text\":\"r=7 x=5\""),
        "continue prints correctly (paused frame intact after eval): {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// A conditional breakpoint whose condition reads a struct field: break only on the
// matching call, then eval a scalar field.
#[test]
fn rpc_conditional_breakpoint_struct_field() {
    let path = tmp_program(
        "proba",
        "struct Point { x: integer, y: integer }\n\
         fn use_pt(p: Point) -> integer {\n  p.x + p.y\n}\n\
         fn main() {\n  use_pt(Point { x: 1, y: 2 });\n  use_pt(Point { x: 9, y: 2 })\n}\n",
    );
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":3,\"condition\":\"p.x == 9\"}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        "{\"id\":4,\"req\":\"eval\",\"expr\":\"p.x\"}".to_string(),
        "{\"id\":5,\"req\":\"continue\"}".to_string(),
        "{\"id\":6,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(
        out.contains("\"event\":\"stopped\""),
        "stopped on the matching call: {out}"
    );
    assert!(
        out.contains("\"id\":4,\"ok\":true,\"value\":9"),
        "eval p.x == 9: {out}"
    );
    assert!(
        out.contains("\"event\":\"terminated\""),
        "terminated: {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// Plain (unconditional) break, then eval the whole struct — returned as a JSON object
// via loft's inbuilt `.to_json()`.
#[test]
fn rpc_eval_struct_as_json() {
    let path = tmp_program(
        "probb",
        "struct Point { x: integer, y: integer }\n\
         fn use_pt(p: Point) -> integer {\n  p.x + p.y\n}\n\
         fn main() {\n  use_pt(Point { x: 9, y: 2 })\n}\n",
    );
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":3}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        "{\"id\":4,\"req\":\"eval\",\"expr\":\"p\"}".to_string(),
        "{\"id\":5,\"req\":\"continue\"}".to_string(),
        "{\"id\":6,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(out.contains("\"event\":\"stopped\""), "stopped: {out}");
    // Eval of a bare struct → JSON object via the D2 live-frame read (show_json on
    // the live DbRef).
    assert!(
        out.contains("\"value\":{\"x\":9,\"y\":2}"),
        "eval p as JSON: {out}"
    );
    assert!(
        out.contains("\"event\":\"terminated\""),
        "terminated: {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// @PLN16 D2 — eval of a bare *vector* local (the case the reconstruct-eval path
// faulted on, returning null): the live-frame read renders it straight from the store,
// as a real JSON array — including a vector of structs.
#[test]
fn rpc_eval_bare_vector_live() {
    let path = tmp_program(
        "vec",
        "struct Mob { hp: integer }\n\
         fn build() -> integer {\n\
        \x20 nums = [10, 20, 30];\n\
        \x20 mobs = [Mob { hp: 5 }, Mob { hp: 9 }];\n\
        \x20 total = nums[0] + mobs[0].hp;\n\
        \x20 total\n\
         }\n\
         fn main() {\n  build()\n}\n",
    );
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        // line 5 is `total = nums[0] + mobs[0].hp;` — both locals are live (read here).
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":5}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        "{\"id\":4,\"req\":\"eval\",\"expr\":\"nums\"}".to_string(),
        "{\"id\":5,\"req\":\"eval\",\"expr\":\"mobs\"}".to_string(),
        "{\"id\":6,\"req\":\"continue\"}".to_string(),
        "{\"id\":7,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(out.contains("\"event\":\"stopped\""), "stopped: {out}");
    // The previously-failing case: a bare vector → a real JSON array, not null.
    assert!(
        out.contains("\"id\":4,\"ok\":true,\"value\":[10,20,30]"),
        "eval nums as a JSON array: {out}"
    );
    assert!(
        out.contains("\"id\":5,\"ok\":true,\"value\":[{\"hp\":5},{\"hp\":9}]"),
        "eval mobs (vector of structs) as JSON: {out}"
    );
    assert!(
        out.contains("\"event\":\"terminated\""),
        "terminated: {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// @PLN63 RX4 — `stepBack` REVERSES a forward step over the RPC (distinct from the edit-scoped
// `undo`, which was a no-op after a step).  Arm reverse, run to a breakpoint, step over a
// mutating line, then step back — the frame returns to the exact prior stop (line + value).
#[test]
fn rpc_step_back_reverses_a_step() {
    // `main` (the default `run` entry); each line reads `a`, so none is a dead store elided
    // by codegen — every line steps.
    let path = tmp_program(
        "rxback",
        "fn main() {\n  a = 1;\n  a = a + 1;\n  a = a + 2;\n  print(\"a={a}\")\n}\n",
    );
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":3}}]}}"
        ),
        "{\"id\":3,\"req\":\"setReverse\",\"on\":true}".to_string(),
        "{\"id\":4,\"req\":\"run\"}".to_string(), // stop at line 3, a == 1
        "{\"id\":5,\"req\":\"stepOver\"}".to_string(), // → line 4, a == 2
        "{\"id\":6,\"req\":\"stepBack\"}".to_string(), // → line 3, a == 1 again
        "{\"id\":7,\"req\":\"disconnect\"}".to_string(),
    ]);
    let stops: Vec<&str> = out
        .lines()
        .filter(|l| l.contains("\"event\":\"stopped\""))
        .collect();
    // The forward step advanced to line 4 with a == 2.
    assert!(
        stops
            .iter()
            .any(|s| s.contains("\"line\":4") && s.contains("\"value\":\"2\"")),
        "stepOver advanced to line 4, a == 2: {out}"
    );
    // The LAST stop is stepBack's: back at line 3 with a reverted to 1, reason `step`.
    let last = stops.last().expect("a stopped event");
    assert!(
        last.contains("\"reason\":\"step\"") && last.contains("\"line\":3"),
        "stepBack reverted to line 3 as a step stop: {last}"
    );
    assert!(
        last.contains("\"name\":\"a\",\"value\":\"1\""),
        "stepBack reverted a to its pre-step value 1: {last}"
    );
    // Reverse never terminates the program.
    assert!(
        !out.contains("\"event\":\"terminated\""),
        "no termination: {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// @PLN63 SF — `stackTrace` returns the FULL runtime call stack (a `frames` array),
// innermost first, each frame with its function + call-site line + locals, alongside the
// legacy single `frame` (additive).  A breakpoint three calls deep surfaces all three
// user frames.
#[test]
fn rpc_stack_trace_returns_full_call_stack() {
    let path = tmp_program(
        "sfstack",
        "fn inner(z: integer) -> integer {\n  z * 2\n}\n\
         fn middle(y: integer) -> integer {\n  inner(y + 1)\n}\n\
         fn main() {\n  r = middle(10);\n  print(\"r={r}\")\n}\n",
    );
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":2}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        "{\"id\":4,\"req\":\"stackTrace\"}".to_string(),
        "{\"id\":5,\"req\":\"disconnect\"}".to_string(),
    ]);
    // The legacy single frame (top) is still present (additive compatibility).
    assert!(
        out.contains("\"frame\":{\"function\":\"inner\""),
        "legacy single frame retained: {out}"
    );
    // The new multi-frame array: inner (top, line 2) → middle (call site line 5) → main.
    let stack_line = out
        .lines()
        .find(|l| l.contains("\"id\":4"))
        .expect("stackTrace response");
    assert!(
        stack_line.contains("\"frames\":[{\"function\":\"inner\",\"line\":2"),
        "top frame inner at line 2: {stack_line}"
    );
    assert!(
        stack_line.contains("{\"function\":\"middle\",\"line\":5"),
        "caller middle at its call site line 5: {stack_line}"
    );
    assert!(
        stack_line.contains("{\"function\":\"main\","),
        "caller main is on the stack: {stack_line}"
    );
    let _ = std::fs::remove_file(&path);
}

// @PLN63 DB — a `setWatch` on a BARE SCALAR LOCAL (a stack slot, previously "not a
// watchable scalar region") is accepted, and a later mutation fires `stopped{reason:
// "watch"}` with the old → new value.
#[test]
fn rpc_watch_stack_local_fires_on_change() {
    let path = tmp_program(
        "dblocal",
        "fn main() {\n  x = 1;\n  x = 2;\n  x = 3;\n  print(\"x={x}\")\n}\n",
    );
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":3}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        "{\"id\":4,\"req\":\"setWatch\",\"expr\":\"x\"}".to_string(),
        "{\"id\":5,\"req\":\"continue\"}".to_string(),
        "{\"id\":6,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(
        out.contains("\"id\":4,\"ok\":true"),
        "a bare local is now watchable: {out}"
    );
    assert!(
        out.contains("\"event\":\"stopped\",\"reason\":\"watch\""),
        "the watch fires on the change: {out}"
    );
    assert!(
        out.contains("\"watch\":{\"label\":\"x\",\"old\":\"1\",\"new\":\"2\"}"),
        "old → new value reported: {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// @PLN63 DB1 — a stack-local watch is bound to its frame: once that frame RETURNS the slot
// is dead, so the watch is dropped and never fires a spurious hit on the reused slot.
#[test]
fn rpc_watch_stack_local_drops_on_frame_exit() {
    let path = tmp_program(
        "dbexit",
        "fn helper() -> integer {\n  h = 5;\n  h + 0\n}\n\
         fn main() {\n  a = helper();\n  b = a + 10;\n  print(\"a={a} b={b}\")\n}\n",
    );
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":3}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        "{\"id\":4,\"req\":\"setWatch\",\"expr\":\"h\"}".to_string(),
        "{\"id\":5,\"req\":\"continue\"}".to_string(),
        "{\"id\":6,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(
        out.contains("\"id\":4,\"ok\":true"),
        "watch set in helper: {out}"
    );
    // helper returns → the watch drops → the run completes with no watch stop.
    assert!(
        !out.contains("\"reason\":\"watch\""),
        "no spurious hit after the frame returns: {out}"
    );
    assert!(
        out.contains("\"event\":\"terminated\""),
        "runs to termination: {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// `setBreakpoints` answers with a per-breakpoint `verified` flag: `true` for a line
// carrying breakable code, `false` for a line that can never fire (no code on it, or
// a file the program doesn't use) — so a client sees a dead breakpoint immediately
// instead of waiting on a stop that never comes.
#[test]
fn rpc_set_breakpoints_reports_verified() {
    let path = tmp_program("verif", "fn main() {\n  a = 1;\n  print(\"a={a}\")\n}\n");
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":2}},{{\"line\":99}}]}}"
        ),
        "{\"id\":3,\"req\":\"setBreakpoints\",\"file\":\"/no/such/dir/other.loft\",\"breakpoints\":[{\"line\":2}]}".to_string(),
        "{\"id\":4,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(
        out.contains(
            "\"id\":2,\"ok\":true,\"breakpoints\":[{\"line\":2,\"verified\":true},{\"line\":99,\"verified\":false}]"
        ),
        "live line verified, dead line not: {out}"
    );
    assert!(
        out.contains("\"id\":3,\"ok\":true,\"breakpoints\":[{\"line\":2,\"verified\":false}]"),
        "unknown file never verifies: {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// A tracepoint's `log` accepts a single expression as a plain string (sugar for a
// one-element array): `stop:false` + `log:"a"` streams `output{category:"trace"}`
// lines without pausing.
#[test]
fn rpc_tracepoint_log_accepts_plain_string() {
    let path = tmp_program(
        "tracestr",
        "fn main() {\n  a = 10;\n  for n in 0..3 {\n    a += n;\n    print(\"n={n}\")\n  }\n}\n",
    );
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"launch\",\"file\":\"{file}\"}}"),
        format!(
            "{{\"id\":2,\"req\":\"setBreakpoints\",\"file\":\"{file}\",\"breakpoints\":[{{\"line\":5,\"log\":\"n\",\"stop\":false}}]}}"
        ),
        "{\"id\":3,\"req\":\"run\"}".to_string(),
        "{\"id\":4,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(
        out.contains("\"category\":\"trace\",\"text\":\"n = 0\""),
        "first trace line: {out}"
    );
    assert!(
        out.contains("\"category\":\"trace\",\"text\":\"n = 2\""),
        "last trace line: {out}"
    );
    // The run never pauses: no stopped event, normal termination.
    assert!(!out.contains("\"event\":\"stopped\""), "no pause: {out}");
    assert!(
        out.contains("\"event\":\"terminated\""),
        "terminated: {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// @PLN16 M5e slice 2 — `compile` checks a file (no run, no load) and emits a structured
// `diagnostics` event with errors AND warnings (the compiler-console feed).
#[test]
fn rpc_compile_emits_structured_diagnostics() {
    // `X = 5` is clean but warns (UPPER_CASE reserved for constants) — proves warnings
    // surface here even though `launch`/run treat the program as runnable.
    let path = tmp_program("warn", "fn main() {\n  X = 5;\n  print(\"{X}\")\n}\n");
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"compile\",\"file\":\"{file}\"}}"),
        "{\"id\":2,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(out.contains("\"id\":1,\"ok\":true"), "compile ok: {out}");
    assert!(
        out.contains("\"event\":\"diagnostics\""),
        "a diagnostics event: {out}"
    );
    // `level` is part of the RPC contract, so the tier is asserted, not just the
    // presence: UPPER_CASE is ADVICE (correct code, a naming preference), and a client
    // that gates on `"level":"warning"` must not see it.
    assert!(
        out.contains("\"line\":2,\"col\":6,\"level\":\"advice\""),
        "structured advice at line 2:6: {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// A file with an error compiles to an `error`-level diagnostic (and is not loaded).
#[test]
fn rpc_compile_reports_errors() {
    // A call to an undefined function is an error the two-pass parser catches.
    let path = tmp_program("err", "fn main() {\n  no_such_function()\n}\n");
    let file = json_path(&path);
    let out = drive(&[
        format!("{{\"id\":1,\"req\":\"compile\",\"file\":\"{file}\"}}"),
        "{\"id\":2,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(
        out.contains("\"event\":\"diagnostics\""),
        "a diagnostics event: {out}"
    );
    assert!(
        out.contains("\"level\":\"error\""),
        "an error-level diagnostic: {out}"
    );
    let _ = std::fs::remove_file(&path);
}

// @PLN16 M5e — bug 1: a REPL expression error must point at the user's INPUT line, not the
// synthetic `fn replmain_N(){…}` wrapper line. `nosuchvar + 1` is a 1-line input, so the
// error is on line 1 (before the fix the wrapper offset reported line 2).
#[test]
fn rpc_repl_eval_error_line_is_input_relative() {
    let out = drive(&[
        "{\"id\":1,\"req\":\"replEval\",\"input\":\"nosuchvar + 1\"}".to_string(),
        "{\"id\":2,\"req\":\"disconnect\"}".to_string(),
    ]);
    assert!(
        out.contains("\"file\":\"<repl>\""),
        "a <repl> diagnostics event: {out}"
    );
    assert!(
        out.contains("\"line\":1,"),
        "error on the input's line 1, not the wrapper line: {out}"
    );
}
