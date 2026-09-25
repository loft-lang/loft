// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I83 — Dev server (--serve / --rpc)

//! @PLN16 M5d phase 2 — the **`--rpc` debug server**: an NDJSON request/response + event
//! stream over the [wire protocol](../../doc/claude/plans/16-debugger/PROTOCOL.md) that
//! drives the debug engine non-interactively.  One JSON object per line: the client
//! sends `{ "id", "req", … }` requests, the server replies `{ "id", "ok", … }` and emits
//! `{ "event", … }` events (`stopped` / `output` / `terminated`).
//!
//! Requests are parsed with loft's **inbuilt JSON parser** ([`crate::json::parse`]); a
//! value shown to the client (an `eval` result) is serialised with loft's **inbuilt JSON
//! serializer** (a struct/enum via its `.to_json()` method, scalars as raw JSON literals —
//! [`ReplSession::debug_eval_json`]).  The server is a thin driver over [`ReplSession`] —
//! every request is one engine method (the protocol invariant: no UI-only, no agent-only
//! semantics).
//!
//! The debugged program's `print` output is captured (the thread-local sink below) and
//! streamed as `output` events, so it never corrupts the protocol stream on stdout.

use crate::json::{ParseError, Parsed};
use crate::repl::ReplSession;
use std::cell::RefCell;
use std::io::{BufRead, Write};

thread_local! {
    /// While `Some`, the loft `print` builtin appends here instead of writing stdout —
    /// so the program's output can be drained into `output` events.  `None` (the default)
    /// is one branch on the print path.
    static OUTPUT_SINK: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// Called by the `print` builtin (`fill.rs`): route the program's output to the capture
/// sink when one is active (returns `true`), else let the caller write stdout (`false`).
#[must_use]
pub fn print_or_capture(text: &str) -> bool {
    OUTPUT_SINK.with(|s| {
        let mut b = s.borrow_mut();
        if let Some(buf) = b.as_mut() {
            buf.push_str(text);
            true
        } else {
            false
        }
    })
}

pub(crate) fn capture_begin() {
    OUTPUT_SINK.with(|s| *s.borrow_mut() = Some(String::new()));
}
pub(crate) fn capture_end() {
    OUTPUT_SINK.with(|s| *s.borrow_mut() = None);
}
/// Take the captured program output since the last drain (empty if none / not capturing).
fn capture_take() -> String {
    OUTPUT_SINK.with(|s| {
        s.borrow_mut()
            .as_mut()
            .map(std::mem::take)
            .unwrap_or_default()
    })
}

/// An in-process debug session driven one wire-protocol line at a time — the seam both
/// [`run_rpc`] (NDJSON on stdio) and the `loft-dap` DAP adapter drive the engine through,
/// with no child interpreter process and no port.  Construction mirrors `run_rpc`'s
/// preamble: a stepping-enabled [`ReplSession`] over `stdlib_dir` + `lib_dirs`, with the
/// program's `print` output routed to the capture sink (drained into `output` events);
/// `Drop` ends the capture.
///
/// `loft-dap` links the `loft` rlib as a separate binary — the same shape as `loft-lsp` —
/// and translates DAP JSON to/from the [`handle`] chokepoint via [`drive`](Self::drive),
/// so it inherits the engine coverage of `tests/rpc.rs` for free and adds only the
/// protocol translation.
pub struct DebugDriver {
    session: ReplSession,
}

impl DebugDriver {
    /// Build a stepping-enabled debug session and arm output capture.
    ///
    /// # Errors
    /// Returns an I/O error if the stdlib cannot be loaded.
    pub fn new(stdlib_dir: &str, lib_dirs: &[String]) -> std::io::Result<Self> {
        let mut session = ReplSession::new_with_libs(stdlib_dir, lib_dirs)?;
        session.debug_stepping(true);
        capture_begin();
        Ok(Self { session })
    }

    /// Handle one NDJSON request line: the messages to emit (response first, then any
    /// events) and whether the client asked to disconnect.  The one dispatch chokepoint.
    pub fn drive(&mut self, line: &str) -> (Vec<String>, bool) {
        handle(&mut self.session, line)
    }

    /// Set a **function-scoped** breakpoint at `func`'s entry — the engine capability the
    /// file:line `setBreakpoints` verb does not surface — so the DAP adapter can honour
    /// `stopOnEntry` by pausing at the program's first statement.
    pub fn set_function_breakpoint(&mut self, func: &str) {
        self.session.add_breakpoint(func);
    }
}

impl Drop for DebugDriver {
    fn drop(&mut self) {
        capture_end();
    }
}

/// Run the file-run debugger over the wire protocol: read NDJSON requests from `input`,
/// write NDJSON responses + events to `out`, until EOF or a `disconnect` request.
///
/// # Errors
/// Returns an I/O error from the input / output streams.
pub fn run_rpc<R: BufRead, W: Write>(
    stdlib_dir: &str,
    lib_dirs: &[String],
    input: R,
    out: &mut W,
) -> std::io::Result<()> {
    let mut driver = DebugDriver::new(stdlib_dir, lib_dirs)?;
    // Silence the raw panic handler — a fault in the debugged program is caught and
    // reported as `terminated`, not printed.
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    for line in input.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let (messages, disconnect) = driver.drive(&line);
        for m in messages {
            writeln!(out, "{m}")?;
        }
        out.flush()?;
        if disconnect {
            break;
        }
    }
    std::panic::set_hook(prev_hook);
    Ok(())
}

// ── request handling ─────────────────────────────────────────────────────────

/// Handle one request line; returns the NDJSON messages to emit (response first, then
/// any events) and whether the client asked to disconnect.
pub(crate) fn handle(session: &mut ReplSession, line: &str) -> (Vec<String>, bool) {
    let parsed = match crate::json::parse(line) {
        Ok(p) => p,
        Err(e) => return (vec![resp_err(0, &parse_err(&e))], false),
    };
    let id = num(&parsed, "id").unwrap_or(0);
    let req = text(&parsed, "req").unwrap_or("");
    let mut out = Vec::new();
    match req {
        "launch" => match session.load_program(text(&parsed, "file").unwrap_or("")) {
            Ok(Ok(())) => out.push(resp_ok(id, "")),
            Ok(Err(diags)) => out.push(resp_err(id, &diags_msg(&diags))),
            Err(e) => out.push(resp_err(id, &format!("cannot read file: {e}"))),
        },
        "replEval" => {
            // @PLN16 M5e — the REPL panel's context-aware eval.  A top-level expression
            // prints its value to the capture sink, so drain anything pending first, then
            // drain again after to capture *this* eval's output (the value / any prints),
            // keeping REPL output distinct from a program `run`'s streamed output.
            let _ = capture_take();
            let outcome = session.repl_eval(text(&parsed, "input").unwrap_or(""), true);
            let printed = capture_take();
            let mut extra = format!("\"context\":\"{}\"", outcome.context);
            if outcome.more {
                extra.push_str(",\"more\":true");
            }
            if let Some(v) = &outcome.value {
                let _ = std::fmt::Write::write_fmt(&mut extra, format_args!(",\"value\":{v}"));
            }
            if !printed.is_empty() {
                let _ = std::fmt::Write::write_fmt(
                    &mut extra,
                    format_args!(",\"output\":{}", esc(&printed)),
                );
            }
            out.push(resp_ok(id, &extra));
            if !outcome.diagnostics.is_empty() {
                out.push(diagnostics_event("<repl>", &outcome.diagnostics));
            }
        }
        "writeFile" => {
            // @PLN16 M5e slice 3 — the editor's save, sandboxed to the one served file.
            match session.write_file(
                text(&parsed, "path").unwrap_or(""),
                text(&parsed, "content").unwrap_or(""),
            ) {
                Ok(()) => out.push(resp_ok(id, "")),
                Err(e) => out.push(resp_err(id, &e)),
            }
        }
        "compile" => {
            // @PLN16 M5e slice 2 — the compiler-console feed: check the file (no run, no
            // load) and emit its diagnostics (errors + warnings) as a structured event.
            let file = text(&parsed, "file").unwrap_or("");
            match session.compile(file) {
                Ok(diags) => {
                    out.push(resp_ok(id, ""));
                    out.push(diagnostics_event(file, &diags));
                }
                Err(e) => out.push(resp_err(id, &format!("cannot read file: {e}"))),
            }
        }
        "runTests" => {
            // @PLN16 M5e slice 5 — run the file's tests, one `testResult` event each, then a
            // `testSummary`.  Drain the capture sink around the run so the tests' own `print`
            // output doesn't leak into the program console.
            let file = text(&parsed, "file").unwrap_or("");
            let _ = capture_take();
            match session.run_file_tests(file) {
                Ok(results) => {
                    let _ = capture_take();
                    out.push(resp_ok(id, ""));
                    let (mut pass_count, mut fail_count) = (0u32, 0u32);
                    for t in &results {
                        if t.passed {
                            pass_count += 1;
                        } else {
                            fail_count += 1;
                        }
                        out.push(test_result_event(t));
                    }
                    out.push(event(
                        "testSummary",
                        &format!("\"passed\":{pass_count},\"failed\":{fail_count}"),
                    ));
                }
                Err(e) => out.push(resp_err(id, &format!("cannot read file: {e}"))),
            }
        }
        "runSuite" => {
            // @PLN16 M5e slice 5 — the PACKAGE suite (`loft test` semantics): walk up from
            // `file` to the nearest loft.toml, run every tests/*.loft.  Each result event
            // carries its `file` so the panel can group; the summary adds the file count.
            let file = text(&parsed, "file").unwrap_or("");
            let _ = capture_take();
            match session.run_suite(file) {
                Ok(by_file) => {
                    let _ = capture_take();
                    out.push(resp_ok(id, ""));
                    let (mut pass_count, mut fail_count) = (0u32, 0u32);
                    let files = by_file.len();
                    for (fname, results) in &by_file {
                        for t in results {
                            if t.passed {
                                pass_count += 1;
                            } else {
                                fail_count += 1;
                            }
                            out.push(test_result_event_in(t, fname));
                        }
                    }
                    out.push(event(
                        "testSummary",
                        &format!(
                            "\"passed\":{pass_count},\"failed\":{fail_count},\"files\":{files}"
                        ),
                    ));
                }
                Err(e) => out.push(resp_err(id, &e.to_string())),
            }
        }
        "launchGame" => {
            // @PLN16 M5e slice 6 — launch the file as a real `loft` child process (its frame
            // loop / future native window must not block this loop); output arrives via
            // `gameStatus` polls.
            match session.launch_game(text(&parsed, "file").unwrap_or("")) {
                Ok(()) => out.push(resp_ok(id, "")),
                Err(e) => out.push(resp_err(id, &e)),
            }
        }
        "gameStatus" => match session.game_status() {
            Some((running, chunk, exit)) => {
                let mut extra = format!("\"running\":{running}");
                if !chunk.is_empty() {
                    let _ = std::fmt::Write::write_fmt(
                        &mut extra,
                        format_args!(",\"output\":{}", esc(&chunk)),
                    );
                }
                if let Some(code) = exit {
                    let _ =
                        std::fmt::Write::write_fmt(&mut extra, format_args!(",\"exit\":{code}"));
                }
                out.push(resp_ok(id, &extra));
            }
            None => out.push(resp_ok(id, "\"running\":false")),
        },
        "stopGame" => match session.stop_game() {
            Some(chunk) => {
                let extra = if chunk.is_empty() {
                    String::new()
                } else {
                    format!("\"output\":{}", esc(&chunk))
                };
                out.push(resp_ok(id, &extra));
            }
            None => out.push(resp_err(id, "no game is running")),
        },
        "setBreakpoints" => {
            let bps = set_breakpoints(session, &parsed);
            out.push(resp_ok(id, &bps));
        }
        "setWatch" => {
            let ok = session.add_watchpoint(text(&parsed, "expr").unwrap_or(""));
            out.push(if ok {
                resp_ok(id, "")
            } else {
                resp_err(id, "not a watchable scalar region")
            });
        }
        "clearWatch" => {
            session.clear_watchpoints();
            out.push(resp_ok(id, ""));
        }
        "run" => {
            out.push(resp_ok(id, ""));
            let entry = text(&parsed, "entry").unwrap_or("main");
            let halted = session.eval_observe(&format!("{entry}()"));
            report(session, "breakpoint", halted, &mut out);
        }
        "continue" => {
            out.push(resp_ok(id, ""));
            let halted = session.debug_continue();
            report(session, "breakpoint", halted, &mut out);
        }
        "stepIn" | "stepOver" | "stepOut" => {
            out.push(resp_ok(id, ""));
            let mode = match req {
                "stepIn" => crate::debugger::StepMode::Into,
                "stepOut" => crate::debugger::StepMode::Out,
                _ => crate::debugger::StepMode::Over,
            };
            let halted = session.debug_step(mode);
            report(session, "step", halted, &mut out);
        }
        "setReverse" => {
            // @PLN63 RX4 — arm reverse stepping so forward steps checkpoint (the client sends
            // this when it will offer `stepBack`); default `on:true`.
            session.set_reverse(bp_bool(&parsed, "on").unwrap_or(true));
            out.push(resp_ok(id, ""));
        }
        "stepBack" => {
            // @PLN63 RX4 — reverse one step (distinct from the edit-scoped `undo`/`redo`).
            // Reverse never terminates the program: we stay paused whether it moved or hit
            // the ring floor, so report the current frame as a `step` stop.  `moved` says
            // whether a checkpoint was restored (false = the ring floor) — the driver uses it
            // to end a `reverseContinue`.
            let moved = session.debug_step_back();
            out.push(resp_ok(id, &format!("\"moved\":{moved}")));
            report(session, "step", session.is_debugging(), &mut out);
        }
        "eval" => {
            let v = session.debug_eval_json(text(&parsed, "expr").unwrap_or(""));
            out.push(resp_ok(
                id,
                &format!("\"value\":{}", v.as_deref().unwrap_or("null")),
            ));
        }
        "setValue" => {
            let ok = session.debug_set(
                text(&parsed, "target").unwrap_or(""),
                text(&parsed, "value").unwrap_or(""),
            );
            out.push(if ok {
                resp_ok(id, &frame_field(session))
            } else {
                resp_err(id, "edit rejected")
            });
        }
        "undo" => out.push(step_resp(id, session.debug_undo(), session)),
        "redo" => out.push(step_resp(id, session.debug_redo(), session)),
        "stackTrace" => out.push(resp_ok(id, &stack_field(session))),
        "disconnect" => {
            out.push(resp_ok(id, ""));
            return (out, true);
        }
        other => out.push(resp_err(id, &format!("unknown request: {other}"))),
    }
    (out, false)
}

/// `setBreakpoints { file, breakpoints: [{ line, condition?, log?, stop? }] }` — replace
/// the session's breakpoints with the request's (v1: replaces all; single-file debug).
/// Returns the response extra: `"breakpoints":[{line, verified}, …]` where `verified`
/// says the line carries breakable code in the loaded program — `false` means this
/// breakpoint can never fire (wrong file or no code on that line), so the client
/// hears about it now instead of waiting forever.
fn set_breakpoints(session: &mut ReplSession, parsed: &Parsed) -> String {
    session.clear_breakpoints();
    let file = text(parsed, "file").unwrap_or("");
    let Some(Parsed::Array(bps)) = field(parsed, "breakpoints") else {
        return "\"breakpoints\":[]".to_string();
    };
    let breakable = session.breakable_lines_in_file(file);
    let mut items: Vec<String> = Vec::new();
    for bp in bps {
        let Some(line) = num(bp, "line") else {
            continue;
        };
        let condition = text(bp, "condition").map(str::to_string);
        let stop = bp_bool(bp, "stop").unwrap_or(true);
        let actions: Vec<String> = match field(bp, "log") {
            Some(Parsed::Array(a)) => a.iter().filter_map(as_str).map(str::to_string).collect(),
            Some(Parsed::Str(s)) => vec![s.clone()],
            _ => Vec::new(),
        };
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        session.add_file_breakpoint_rich(file, line as u32, condition, actions, stop);
        #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
        let verified = breakable.contains(&(line as u32));
        items.push(format!("{{\"line\":{line},\"verified\":{verified}}}"));
    }
    format!("\"breakpoints\":[{}]", items.join(","))
}

/// After a run/continue/step, emit the program output + tracepoints + the stop verdict.
fn report(session: &mut ReplSession, step_reason: &str, paused: bool, out: &mut Vec<String>) {
    let captured = capture_take();
    for chunk in captured.split_inclusive('\n') {
        let line = chunk.strip_suffix('\n').unwrap_or(chunk);
        if !line.is_empty() {
            out.push(event(
                "output",
                &format!("\"category\":\"stdout\",\"text\":{}", esc(line)),
            ));
        }
    }
    for t in session.take_trace_output() {
        out.push(event(
            "output",
            &format!("\"category\":\"trace\",\"text\":{}", esc(&t)),
        ));
    }
    if paused {
        let watch = session.take_watch_hit();
        let interrupted =
            crate::debugger::STOPPED_BY_INTERRUPT.swap(false, std::sync::atomic::Ordering::Relaxed);
        let reason = if watch.is_some() {
            "watch"
        } else if interrupted {
            "pause"
        } else {
            step_reason
        };
        let mut body = format!("\"reason\":\"{reason}\",{}", frame_field(session));
        if let Some(h) = watch {
            use std::fmt::Write as _;
            let _ = write!(
                body,
                ",\"watch\":{{\"label\":{},\"old\":{},\"new\":{}}}",
                esc(&h.label),
                esc(&h.old),
                esc(&h.new)
            );
        }
        out.push(event("stopped", &body));
    } else {
        out.push(event("terminated", ""));
    }
}

/// A `frame` field for the current pause (`"frame": { function, locals: [...] }`), or
/// `"frame":null` when not paused.  Locals carry their own-format rendering as `value`.
fn frame_field(session: &ReplSession) -> String {
    match session.paused_frame() {
        Some(f) => {
            let locals: Vec<String> = f
                .locals
                .iter()
                .map(|(n, v)| format!("{{\"name\":{},\"value\":{}}}", esc(n), esc(v)))
                .collect();
            // The source line the pause is parked on (moves as you step) — drives the
            // editor's current-line marker.  Absent when the line is unknown.
            let line = session
                .paused_line()
                .map_or(String::new(), |l| format!(",\"line\":{l}"));
            format!(
                "\"frame\":{{\"function\":{},\"locals\":[{}]{line}}}",
                esc(&f.function),
                locals.join(",")
            )
        }
        None => "\"frame\":null".to_string(),
    }
}

/// A `stackTrace` body: the legacy single `frame` (the top frame, kept additively for
/// compatibility) plus a `frames` array — one `{function, line, locals}` per runtime call
/// frame, innermost first (@PLN63 SF, the multi-frame stack).
fn stack_field(session: &ReplSession) -> String {
    let frames: Vec<String> = session
        .paused_stack()
        .iter()
        .map(|f| {
            let locals: Vec<String> = f
                .locals
                .iter()
                .map(|(n, v)| format!("{{\"name\":{},\"value\":{}}}", esc(n), esc(v)))
                .collect();
            format!(
                "{{\"function\":{},\"line\":{},\"locals\":[{}]}}",
                esc(&f.function),
                f.line,
                locals.join(",")
            )
        })
        .collect();
    format!("{},\"frames\":[{}]", frame_field(session), frames.join(","))
}

/// An `ok` response carrying the refreshed frame (for `undo`/`redo`).
fn step_resp(id: i64, ok: bool, session: &ReplSession) -> String {
    if ok {
        resp_ok(id, &frame_field(session))
    } else {
        resp_err(id, "nothing to do")
    }
}

// ── tiny JSON helpers ────────────────────────────────────────────────────────

/// A field of a JSON object, or `None` if `p` isn't an object / lacks the key.
fn field<'a>(p: &'a Parsed, key: &str) -> Option<&'a Parsed> {
    match p {
        Parsed::Object(fields) => fields.iter().find(|(k, _, _)| k == key).map(|(_, _, v)| v),
        _ => None,
    }
}
fn as_str(p: &Parsed) -> Option<&str> {
    match p {
        Parsed::Str(s) => Some(s),
        _ => None,
    }
}
fn text<'a>(p: &'a Parsed, key: &str) -> Option<&'a str> {
    field(p, key).and_then(as_str)
}
fn num(p: &Parsed, key: &str) -> Option<i64> {
    // @PLN109 — an integer field now arrives as `Parsed::Int` (exact); `as_i64`
    // also accepts a `Number` (truncated) for backward compatibility.
    field(p, key).and_then(Parsed::as_i64)
}
fn bp_bool(p: &Parsed, key: &str) -> Option<bool> {
    match field(p, key) {
        Some(Parsed::Bool(b)) => Some(*b),
        _ => None,
    }
}

/// A JSON string literal (quoted + escaped) — the one home for protocol string escaping.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = std::fmt::Write::write_fmt(&mut out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn resp_ok(id: i64, extra: &str) -> String {
    if extra.is_empty() {
        format!("{{\"id\":{id},\"ok\":true}}")
    } else {
        format!("{{\"id\":{id},\"ok\":true,{extra}}}")
    }
}
fn resp_err(id: i64, msg: &str) -> String {
    format!("{{\"id\":{id},\"ok\":false,\"error\":{}}}", esc(msg))
}
fn event(name: &str, body: &str) -> String {
    if body.is_empty() {
        format!("{{\"event\":\"{name}\"}}")
    } else {
        format!("{{\"event\":\"{name}\",{body}}}")
    }
}

/// @PLN16 M5e slice 2 — the compiler console's feed: a `diagnostics` event listing the
/// file's diagnostics (errors + warnings), each structured (line / col / level / message)
/// so the editor can mark them and the console can list them.  Emitted even when the list
/// is empty, so a re-compile clears stale markers.
fn diagnostics_event(file: &str, diags: &[crate::diagnostics::DiagEntry]) -> String {
    let items = diags
        .iter()
        .map(|d| {
            format!(
                "{{\"line\":{},\"col\":{},\"level\":{},\"message\":{}}}",
                d.line,
                d.col,
                esc(level_str(d.level)),
                esc(&d.message)
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    event(
        "diagnostics",
        &format!("\"file\":{},\"items\":[{items}]", esc(file)),
    )
}

/// @PLN16 M5e slice 5 — one test's result as a structured event (name / passed / line, plus
/// the failure message when it failed), so the browser's test panel can list it and a
/// failing row can jump to its line.
fn test_result_event(t: &crate::repl::TestRun) -> String {
    test_result_event_in(t, "")
}

/// [`test_result_event`] carrying the test's **file** — the suite runner's form, so the
/// panel can group results per test file (empty `file` = single-file run, field omitted).
fn test_result_event_in(t: &crate::repl::TestRun, file: &str) -> String {
    let msg = t
        .message
        .as_deref()
        .map_or(String::new(), |m| format!(",\"message\":{}", esc(m)));
    let in_file = if file.is_empty() {
        String::new()
    } else {
        format!(",\"file\":{}", esc(file))
    };
    event(
        "testResult",
        &format!(
            "\"name\":{},\"passed\":{},\"line\":{}{in_file}{msg}",
            esc(&t.name),
            t.passed,
            t.line
        ),
    )
}

/// The protocol spelling of a diagnostic [`Level`](crate::diagnostics::Level).
fn level_str(level: crate::diagnostics::Level) -> &'static str {
    use crate::diagnostics::Level;
    match level {
        Level::Debug => "debug",
        Level::Advice => "advice",
        Level::Warning => "warning",
        Level::Error => "error",
        Level::Fatal => "fatal",
    }
}

fn diags_msg(diags: &[crate::diagnostics::DiagEntry]) -> String {
    diags
        .iter()
        .map(crate::diagnostics::DiagEntry::to_string_compact)
        .collect::<Vec<_>>()
        .join("; ")
}
fn parse_err(e: &ParseError) -> String {
    format!("bad json: {e:?}")
}
