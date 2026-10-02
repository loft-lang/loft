// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I78 — Live-reload dispatch: the browser debug CLIENT (@PLN98 P3.4), the
// live/debug tier's browser-side sibling to live_dispatch.rs.

//! The interactive debug client a browser (`--html --debug`) build runs: the
//! program executes INTERPRETED over a parked [`State`], breakpoints PAUSE it
//! cooperatively (P3.2 — `execute_argv` / `debug_step` return to the JS event
//! loop, never block), and `D!:` control frames arrive over the `host_input`
//! channel (relayed from the server the client holds a WebSocket to), with `D:`
//! replies going back over the host output.  It reuses the `State`'s own debug
//! methods directly — NOT the native `debug_cmd_dispatch` / TCP machinery — so
//! the whole path is wasm-safe (no threads, no sockets, no process spawn).
//!
//! Split for testability: [`apply`] is a pure `(session, frame) -> replies`
//! step (unit-tested natively); [`pump`] wires it to the wasm host I/O.

use std::cell::RefCell;

use crate::debugger::StepMode;
use crate::state::State;

/// The parked interpreter + its owning parser, plus the run's lifecycle flags.
struct Session {
    /// Owns the `Data` the `State` executes against (its address must stay put;
    /// boxed, held here for the session's lifetime).
    parser: Box<crate::parser::Parser>,
    state: Box<State>,
    /// `execute_argv` has been entered (later resumes use `debug_step`).
    started: bool,
    /// `main` ran to completion — further `resume` is a no-op.
    done: bool,
    /// Names each synthetic eval fn (`__eval_<n>`) uniquely across evals.
    counter: u32,
    /// The def-source the PROGRAM parsed under.
    ///
    /// A definition is keyed by `(name, source)`, and `Parser::parse_str` resets the scope to
    /// the stdlib's on each pass — so an `__eval_N` compiled that way cannot see the program's
    /// own functions, and `stats(1, 9)` answered *"Unknown function stats"*.  That is where
    /// every non-scalar row of loft#1187 actually stopped, before any question about how the
    /// value crosses the frame.  [`Parser::parse_snippet`] parses under this scope instead;
    /// it exists for the same failure one tier over (a live-reload snippet losing its library
    /// scope, #350).
    program_source: u16,
    /// The first definition index belonging to the PROGRAM rather than the stdlib.
    /// The stdlib is parsed first, so everything from here on is the reader's own code —
    /// which is what `fns` has to list and what the panel offers to call.
    user_from: u32,
}

thread_local! {
    static SESSION: RefCell<Option<Session>> = const { RefCell::new(None) };
}

/// Parse the embedded stdlib + `program_src` into an interpreter session ready to
/// debug (compiled, not yet run).  Returns `false` if either does not parse
/// clean.  Replaces any previous session.
#[must_use]
pub fn start(program_src: &str) -> bool {
    start_reporting(program_src).is_ok()
}

/// [`start`], with the diagnostics on failure instead of a bare `false`.
///
/// A page that offers a REPL over the code in front of the reader has to say WHY the code
/// did not compile — "it didn't work" is not something a reader can act on. The bool form
/// stays for the callers that only branch on it.
///
/// # Errors
/// The parse diagnostics, rendered, when the stdlib or the program does not compile.
pub fn start_reporting(program_src: &str) -> Result<(), String> {
    let mut p = Box::new(crate::parser::Parser::new());
    // Parsed against again after compiling: no whole-program signature rewrite.
    p.data.open_world = true;
    for (name, content) in crate::stdlib_sources::STDLIB_SOURCES {
        if !p.parse_source(content, name, true) {
            return Err(format!(
                "the bundled standard library ({name}) did not compile"
            ));
        }
    }
    let user_from = p.data.definitions();
    p.parse_source(program_src, "program.loft", false);
    if p.diagnostics.level() >= crate::diagnostics::Level::Error {
        return Err(p.diagnostics.to_string());
    }
    let program_source = p.data.source;
    crate::scopes::check(&mut p.data, &mut p.database);
    let mut state = Box::new(State::new(p.database.clone()));
    crate::compile::byte_code(&mut state, &mut p.data);
    // Stepping mode so a registered breakpoint SUSPENDS (returns) instead of
    // record-and-continue — the cooperative pause.
    state.enable_stepping();
    SESSION.with(|s| {
        *s.borrow_mut() = Some(Session {
            parser: p,
            state,
            started: false,
            done: false,
            counter: 0,
            program_source,
            user_from,
        });
    });
    Ok(())
}

/// Break on the LAST line of `main`, so a run ends paused instead of unwound.
///
/// `eval` reads its answer off a live frame, so a prompt against a program that has
/// finished has nothing to evaluate against — the reader would type `fib(10)` into a
/// session with no stack.  Pausing on main's last line leaves the frame standing, with
/// every local main assigned in it, which is the state a REPL wants and the state the
/// program is in one instruction before it disappears.
///
/// Walks the user file's breakable lines from the bottom and takes the first that lands
/// inside `main` — the scoping matters, because a bare line number matches that line in
/// every function.  `false` when there is no `main` or it has no breakable line.
fn break_at_end_of_main(sess: &mut Session) -> bool {
    let data = &sess.parser.data;
    let d = data.def_nr("n_main");
    if d >= data.definitions() {
        return false;
    }
    let mut lines = sess.state.breakable_lines_in_file("program.loft", data);
    lines.reverse();
    let data = &sess.parser.data;
    lines
        .into_iter()
        .any(|line| sess.state.set_breakpoint_fn_line(d, line, data).is_some())
}

/// Apply one `D!:`-stripped control command to `sess`, returning the `D:` replies
/// (0+).  The command grammar mirrors the TCP debug channel's:
/// - `bp <fn>` — break at the entry of `<fn>` (`D:ok`/`D:err`).
/// - `bp <line>` — break at a line of `program.loft`; `bp end` at main's last line.
/// - `run` / `resume` — start (or continue) the run to the next breakpoint or
///   completion; `D:hit <fn> <locals>` when it pauses, `D:terminated` when done.
/// - `step` — resume one source line.
/// - `eval <expr>` — evaluate `<expr>` over the paused frame (`D:eval <expr>=<v>`).
/// - `fns` — the program's own callable functions, with signatures.
fn apply(sess: &mut Session, cmd: &str) -> Vec<String> {
    let (verb, arg) = cmd
        .split_once(' ')
        .map_or((cmd, ""), |(a, b)| (a, b.trim()));
    // Disjoint borrows: `state` and `parser` are separate fields.
    let data = &sess.parser.data;
    match verb {
        "bp" if !arg.is_empty() => {
            let ok = match arg.parse::<u32>() {
                Ok(line) => sess
                    .state
                    .set_breakpoint_file_line("program.loft", line, data)
                    .is_some(),
                Err(_) if arg == "end" => break_at_end_of_main(sess),
                Err(_) => sess.state.set_breakpoint_fn_start(arg, data).is_some(),
            };
            vec![format!("D:{} bp {arg}", if ok { "ok" } else { "err" })]
        }
        // The program's own callable functions, so a REPL panel can say what there is to
        // call instead of leaving the reader to scroll up and guess.  Drawn from the
        // definitions the PROGRAM added (everything after the stdlib), rendered through
        // `api_surface::signature_of` — the same spelling `loft api` and the LSP hover use.
        "fns" => {
            let mut out = Vec::new();
            for d in sess.user_from..data.definitions() {
                if let Some(("fn", name)) = crate::api_surface::classify(data, d)
                    && !name.starts_with("__")
                    && name != "main"
                {
                    let sig = crate::api_surface::signature_of(data, d, "fn");
                    out.push(format!("{name}{sig}"));
                }
            }
            vec![format!("D:fns {}", out.join("|"))]
        }
        "run" | "resume" | "step" => {
            if sess.done {
                return vec!["D:terminated".to_string()];
            }
            let mode = if verb == "step" {
                StepMode::Into
            } else {
                StepMode::Continue
            };
            if sess.started {
                let paused = sess.state.debug_step(mode, data);
                if !paused {
                    sess.done = true;
                }
            } else {
                sess.started = true;
                // The entry run: a registered breakpoint suspends (stepping mode).
                sess.state.execute_argv("main", data, &[]);
                if !sess.state.is_paused() {
                    sess.done = true;
                }
            }
            if let Some(hit) = sess.state.paused_frame() {
                // `user_locals` and not `locals`: the frame also holds the compiler's own
                // scratch (`__work_N` format buffers, the `#`-infixed loop machinery), and
                // a reader looking at a panel of their own variables should not have to
                // know which of them they wrote.  Display-only — `eval <name>` still
                // resolves a temp for anyone who wants one.
                let locals = hit
                    .user_locals()
                    .iter()
                    .map(|(n, v)| format!("{n}={v}"))
                    .collect::<Vec<_>>()
                    .join("|");
                vec![format!("D:hit {} {locals}", hit.function)]
            } else {
                vec!["D:terminated".to_string()]
            }
        }
        "eval" if !arg.is_empty() => {
            vec![format!("D:eval {arg}={}", eval_expr(sess, arg))]
        }
        _ => vec![format!("D:err unknown command {cmd:?}")],
    }
}

/// @PLN98 P3.4 — full-expression eval over the paused frame.  Binds every
/// referenced live local as a typed arg of a synthetic fn and evaluates it via
/// [`State::eval_frame_reenter`](crate::state::State::eval_frame_reenter), reading
/// each local where it lives.  Handles arbitrary expressions (`2 + 2`, `n + 2`,
/// `h["a"].v`, `len(v)`, `s.field`).  A bare heap ident is read live in place; every value
/// shape answers, text and vector included, because a non-scalar result is boxed in a
/// generated one-field record and serialised inside the callee (loft#1187).
/// `<unavailable>` is left for an expression that cannot be BOUND OR COMPILED at all — a
/// parse error, a name not in scope at the frame — and is never a wrong value.
fn eval_expr(sess: &mut Session, expr: &str) -> String {
    let expr = expr.trim();
    if is_bare_ident(expr) {
        if let Some(v) = sess.state.eval_frame_heap(expr, false, &sess.parser.data) {
            return v;
        }
        if let Some(v) = sess.state.paused_frame().and_then(|f| {
            f.locals
                .iter()
                .find(|(n, _)| n == expr)
                .map(|(_, v)| v.clone())
        }) {
            return v;
        }
    }
    eval_via_reenter(sess, expr).unwrap_or_else(|| "<unavailable>".to_string())
}

fn is_bare_ident(s: &str) -> bool {
    s.chars()
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// The identifiers appearing in `expr` (over-approximate — string contents are
/// included, but only actual frame-local names are ever bound, so it's harmless).
fn idents(expr: &str) -> std::collections::HashSet<String> {
    let mut out = std::collections::HashSet::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, out: &mut std::collections::HashSet<String>| {
        if cur
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        {
            out.insert(std::mem::take(cur));
        } else {
            cur.clear();
        }
    };
    for c in expr.chars() {
        if c.is_alphanumeric() || c == '_' {
            cur.push(c);
        } else if !cur.is_empty() {
            flush(&mut cur, &mut out);
        }
    }
    if !cur.is_empty() {
        flush(&mut cur, &mut out);
    }
    out
}

/// Bind the referenced live locals as args, infer the result type, and evaluate
/// `expr` over the paused frame.
///
/// A heap result cannot ride the frame base, so it is serialised in-fn and returned as text
/// (as [`State::eval_frame_reenter`] documents).  Everything that is not a scalar goes through
/// ONE shape — a generated one-field record, serialised with `.to_json()`:
///
/// ```text
///   struct __EvalBox_7 { v: <inferred type> }
///   fn __eval_7(…) -> text { __EvalBox_7 { v: (expr) }.to_json() }
/// ```
///
/// The direct spelling `(expr).to_json()` works only for a STRUCT — `.to_json()` is a parser
/// intercept for struct instances, so a `text` or a `vector` failed to compile and the client
/// answered `<unavailable>` (loft#1187).  Boxing puts every non-scalar back on the one path
/// that is known good: the record is serialised inside the callee and what crosses the frame
/// boundary is a call-returned-owned text, which is the Text arm's stated precondition.  A
/// text returned DIRECTLY is a work buffer freed at teardown, and reading it there is the
/// store corruption that issue's second route measured.
///
/// The box's type is registered in the PARSER's schema when the eval compiles, and the paused
/// `State`'s was cloned before the session started — [`Stores::adopt_new_types`] carries it
/// across, which is that issue's fourth route.
fn eval_via_reenter(sess: &mut Session, expr: &str) -> Option<String> {
    let refs = idents(expr);
    let mut binds: Vec<(String, String)> = Vec::new();
    {
        let frame = sess.state.paused_frame()?;
        for (name, _) in &frame.locals {
            if refs.contains(name)
                && let Some(ty) = sess.state.frame_local_arg_type(name, &sess.parser.data)
            {
                binds.push((name.clone(), ty));
            }
        }
    }
    let arg_names: Vec<String> = binds.iter().map(|(n, _)| n.clone()).collect();
    let sig = binds
        .iter()
        .map(|(n, t)| format!("{n}: {t}"))
        .collect::<Vec<_>>()
        .join(", ");
    let ret = infer_ret(sess, &sig, expr)?;
    if is_scalar_type(&ret) {
        return build_run(sess, &sig, expr, &ret, "", &arg_names);
    }
    let box_name = format!("__EvalBox_{}", sess.counter + 1);
    let boxed = build_run(
        sess,
        &sig,
        &format!("{box_name} {{ v: ({expr}) }}.to_json()"),
        "text",
        &format!("struct {box_name} {{ v: {ret} }}\n"),
        &arg_names,
    )?;
    unbox(&boxed)
}

/// The value out of `{"v":<value>}`, verbatim.
///
/// Verbatim rather than re-rendered: the box is a serialisation detail of how the value
/// crossed the frame boundary, and what the reader asked for is the value.  A `text` comes
/// back quoted and a `vector` bracketed, which is what `loft debug` shows for the same
/// expressions.
fn unbox(json: &str) -> Option<String> {
    let inner = json.trim().strip_prefix("{\"v\":")?.strip_suffix('}')?;
    Some(inner.to_string())
}

/// Compile `fn _(sig) {{ __t = (expr); }}` and read `__t`'s (base) type.  Rolled
/// back — a throwaway probe.  `None` if it doesn't type-check.
fn infer_ret(sess: &mut Session, sig: &str, expr: &str) -> Option<String> {
    let name = format!("__evalinfer_{}", sess.counter + 1);
    let src = format!("fn {name}({sig}) {{\n  __t = ({expr});\n}}\n");
    let pre_defs = sess.parser.data.definitions();
    let pre_diag = sess.parser.diagnostics.entries().len();
    sess.parser
        .parse_snippet(&src, "<debug>", sess.program_source);
    let failed = sess.parser.diagnostics.entries()[pre_diag..]
        .iter()
        .any(|e| e.level >= crate::diagnostics::Level::Error);
    let result = if failed {
        None
    } else {
        let d = sess.parser.data.def_nr(&format!("n_{name}"));
        (d != u32::MAX)
            .then(|| {
                let def = sess.parser.data.def(d);
                let vars = &def.variables;
                (0..vars.count())
                    .find(|&i| vars.name(i) == "__t")
                    .map(|i| vars.tp(i).show(&sess.parser.data, vars))
            })
            .flatten()
    };
    sess.parser.data.rollback_to(pre_defs);
    result.map(|s| base_type(&s).to_string())
}

/// Compile `fn _(sig) -> ret_ty {{ (expr) }}` and evaluate it over the paused frame
/// (the args are `arg_names`, the live frame locals).  The def is kept — its
/// bytecode is appended into the paused State by `eval_frame_reenter`.
fn build_run(
    sess: &mut Session,
    sig: &str,
    expr: &str,
    ret_ty: &str,
    prelude: &str,
    arg_names: &[String],
) -> Option<String> {
    let name = format!("__eval_{}", sess.counter + 1);
    let src = format!("{prelude}fn {name}({sig}) -> {ret_ty} {{\n  ({expr})\n}}\n");
    let pre_diag = sess.parser.diagnostics.entries().len();
    sess.parser
        .parse_snippet(&src, "<debug>", sess.program_source);
    let failed = sess.parser.diagnostics.entries()[pre_diag..]
        .iter()
        .any(|e| e.level >= crate::diagnostics::Level::Error);
    if failed {
        return None;
    }
    sess.counter += 1;
    crate::scopes::check(&mut sess.parser.data, &mut sess.parser.database);
    let d = sess.parser.data.def_nr(&format!("n_{name}"));
    if d == u32::MAX {
        return None;
    }
    let ret_type = sess.parser.data.def(d).returned.clone();
    // A type the eval just declared exists only in the PARSER's schema; the paused `State`
    // holds a clone taken before the session started, and a record of an id it cannot resolve
    // faults inside `database::types` rather than answering (loft#1187).  Ids are positional
    // and both schemas grew from that one clone, so this is an append.
    if !sess.state.database.adopt_new_types(&sess.parser.database) {
        return None;
    }
    sess.state
        .eval_frame_reenter(&mut sess.parser.data, d, arg_names, &ret_type, false)
}

fn base_type(show: &str) -> &str {
    let base = show.split('[').next().unwrap_or(show);
    base.strip_prefix("ref(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(base)
}

fn is_scalar_type(t: &str) -> bool {
    matches!(
        t,
        "integer" | "float" | "single" | "boolean" | "character" | "byte"
    ) || t.starts_with("integer(")
}

/// The DIRECT entry, for a caller that already holds the session in its own process — the
/// doc site's panel, which calls straight into the wasm module and has no relay to speak
/// `D!:` frames through, and the native tests, which drive the same grammar [`pump`] does.
/// `pump` is the same step wired to the host I/O channel instead.
///
/// `D:err no session` when [`start`] has not run, so a caller that lost its session gets an
/// answer rather than silence.
#[must_use]
pub fn command(cmd: &str) -> Vec<String> {
    SESSION.with(|s| {
        let mut g = s.borrow_mut();
        match g.as_mut() {
            Some(sess) => apply(sess, cmd),
            None => vec!["D:err no session".to_string()],
        }
    })
}

// ── @PLN149 step 8 — the entries a doc page calls ───────────────────────────
//
// `crate::wasm::compile_and_run` runs a program and hands back its output; these two let a
// PAGE drive the same program — set a breakpoint, pause, read the frame, evaluate an
// expression against it, resume.  Until now this module was reachable only from a
// `--html --debug` build with the source BAKED IN and a WebSocket relay feeding it `D!:`
// frames.  A doc page has neither: it holds the source the reader is editing and calls
// straight in.  The command grammar is the same one [`pump`] applies, so the page and the
// relay drive one implementation rather than two that agree until they do not.

/// Start a debug session over `source`, replacing any previous one.
///
/// Returns JSON `{"ok":true}`, or `{"ok":false,"error":"…"}` carrying the diagnostics — a
/// page that offers to run the code in front of the reader has to say why it will not.
///
/// A bare script (top-level statements, no `fn main`) is desugared exactly as
/// `compile_and_run` desugars it, so the two entries accept the same inputs.
#[cfg_attr(feature = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
#[must_use]
pub fn debug_start(source: &str) -> String {
    let desugared = crate::script::script_desugar(source);
    let src = desugared.as_deref().unwrap_or(source);
    // The capture buffer is per-thread and shared with `compile_and_run`; a stale tail from
    // an earlier run would otherwise surface as output of this session's first `run`.
    let _ = take_output();
    let value = match start_reporting(src) {
        Ok(()) => json_object(&[("ok", crate::json::Parsed::Bool(true))]),
        Err(e) => json_object(&[
            ("ok", crate::json::Parsed::Bool(false)),
            ("error", crate::json::Parsed::Str(e)),
        ]),
    };
    crate::json::to_json_string(&value)
}

/// Apply one debug command to the live session.
///
/// Returns JSON `{"replies":[…],"output":"…"}` — the `D:` replies the command produced and
/// whatever the program printed while it ran.  The two travel together because a `run` or a
/// `resume` produces both, and a page fetching them separately could paint a pause before
/// the output that led to it.
///
/// The grammar is [`command`]'s: `bp <fn>` / `bp <line>`, `run`, `resume`, `step`,
/// `eval <expr>`, `fns`.
#[cfg_attr(feature = "wasm", wasm_bindgen::prelude::wasm_bindgen)]
#[must_use]
pub fn debug_command(cmd: &str) -> String {
    use crate::json::Parsed;
    let replies = command(cmd);
    let value = json_object(&[
        (
            "replies",
            Parsed::Array(replies.into_iter().map(Parsed::Str).collect()),
        ),
        ("output", Parsed::Str(take_output())),
    ]);
    crate::json::to_json_string(&value)
}

/// A JSON object from `(key, value)` pairs, rendered through the one serialiser this repo
/// has (`json::to_json_string`, RFC 8259) rather than a fourth hand-rolled escaper.
fn json_object(fields: &[(&str, crate::json::Parsed)]) -> crate::json::Parsed {
    crate::json::Parsed::Object(
        fields
            .iter()
            .map(|(k, v)| ((*k).to_string(), 0, v.clone()))
            .collect(),
    )
}

/// Whatever the program printed since the last call.
///
/// Only a browser build captures print — `crate::wasm::output_push` is where the print op
/// routes under the `wasm` feature.  Natively the output goes to stdout as it always does,
/// so this is empty and the JSON field is present-but-empty rather than absent: one shape
/// for the page to read whichever build served it.
fn take_output() -> String {
    #[cfg(feature = "wasm")]
    {
        crate::wasm::output_take()
    }
    #[cfg(not(feature = "wasm"))]
    {
        String::new()
    }
}

/// The pump the JS driver calls (per animation frame): drain every pending
/// `host_input` frame, apply the `D!:`-tagged debug ones to the session (routing
/// non-`D!:` messages back as program input is the browser driver's job), and
/// emit each `D:` reply over the host output for the server relay.  No-op when no
/// session is active.
pub fn pump() {
    SESSION.with(|s| {
        let mut guard = s.borrow_mut();
        let Some(sess) = guard.as_mut() else {
            return;
        };
        loop {
            // 0 = take what is queued and return; this pump must never block the
            // page's one thread, which is what would deliver the next frame.
            let frame = sess.state.database.host_input_native(0);
            if frame.is_empty() {
                break;
            }
            let Some(cmd) = frame.trim().strip_prefix("D!:") else {
                continue; // program input — not ours (the JS side re-queues it)
            };
            for reply in apply(sess, cmd.trim()) {
                crate::live_dispatch::wasm_host_log(&format!("{reply}\n"));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{SESSION, apply, start};

    // Drive the full debug cycle over the session with the SAME command grammar
    // the browser relay feeds: bp -> run (pause) -> eval a live local -> resume
    // (terminate). Uses `apply` directly (host I/O is the wasm-only wiring).
    #[test]
    fn debug_session_bp_run_eval_resume_cycle() {
        assert!(start(
            "fn compute(n: integer) -> integer {\n  m = n + 2;\n  m\n}\n\
             fn main() { compute(40); }\n"
        ));
        SESSION.with(|s| {
            let mut g = s.borrow_mut();
            let sess = g.as_mut().expect("session");
            assert_eq!(apply(sess, "bp compute"), vec!["D:ok bp compute"]);
            // run -> pauses at compute's entry; the arg n=40 is live.
            let hit = apply(sess, "run");
            assert!(
                hit[0].starts_with("D:hit compute") && hit[0].contains("n=40"),
                "paused in compute with n=40: {hit:?}"
            );
            // eval the live frame local + a full EXPRESSION over it (the P3.4
            // full-eval: binds `n` as an arg of a synthetic fn, reenters).
            assert_eq!(apply(sess, "eval n"), vec!["D:eval n=40"]);
            assert_eq!(apply(sess, "eval n + 2"), vec!["D:eval n + 2=42"]);
            assert_eq!(apply(sess, "eval n * n"), vec!["D:eval n * n=1600"]);
            assert_eq!(apply(sess, "eval 2 + 3"), vec!["D:eval 2 + 3=5"]);
            // resume -> runs to completion.
            assert_eq!(apply(sess, "resume"), vec!["D:terminated"]);
        });
    }

    /// A text- or vector-valued expression evaluates, and answers the value (loft#1187).
    ///
    /// Those two used to answer `<unavailable>`, and it was the SAFE answer: `(expr).to_json()`
    /// does not compile for either — `.to_json()` is a parser intercept for struct instances —
    /// and each of the three ways round it corrupted the store, because a text returned
    /// directly rides a work buffer freed at the callee's teardown.  Boxing the value in a
    /// generated one-field record puts every non-scalar back on the STRUCT path, which was
    /// working throughout.
    ///
    /// The scalar and struct rows are here as the controls: this rewrote how every non-scalar
    /// eval is compiled, so the shape that already worked has to still work, and the shape that
    /// never went near the box has to be untouched.
    // One expression that does not evaluate must not end the session.  It did: a failed
    // `build_run` left its half-parsed definition in `data` AND did not advance the
    // counter, so the next eval built the same name over the wreck and every eval after
    // the first failure answered `<unavailable>` — including ones that had just worked.
    //
    // The failing input is an unknown FUNCTION, because that is what `<unavailable>` means
    // now that every value shape evaluates (loft#1187); any non-evaluating expression
    // exercises the same path.
    #[test]
    fn a_failed_eval_does_not_end_the_session() {
        assert!(start(
            "fn fib(n: integer) -> integer { if n < 2 { return n; } fib(n-1) + fib(n-2) }\n\
             fn compute(n: integer) -> integer {\n  m = n + 2;\n  m\n}\n\
             fn main() { compute(40); }\n"
        ));
        let s = SESSION.with(|s| {
            let mut g = s.borrow_mut();
            let sess = g.as_mut().expect("session");
            let _ = apply(sess, "bp compute");
            let _ = apply(sess, "run");
            // Works before.
            let before = apply(sess, "eval fib(10)");
            // Does not evaluate — and must not be fatal.  The expression has to be one
            // that genuinely fails to COMPILE: this was `"a" + "b"`, chosen when a
            // text-valued eval could not answer at all, and loft#1187 made that shape
            // succeed — so the test would have gone on passing while measuring nothing.
            let bad = apply(sess, "eval nosuchfunction(1)");
            // Still works after, which is the whole assertion.
            let after = apply(sess, "eval fib(10)");
            let other = apply(sess, "eval n + 2");
            (before, bad, after, other)
        });
        assert_eq!(s.0, vec!["D:eval fib(10)=55"]);
        assert_eq!(s.1, vec!["D:eval nosuchfunction(1)=<unavailable>"]);
        assert_eq!(
            s.2,
            vec!["D:eval fib(10)=55"],
            "a failed eval poisoned the session"
        );
        assert_eq!(s.3, vec!["D:eval n + 2=42"]);
    }

    // The program's OWN functions are callable from `eval`, which is the whole point of a
    // REPL over a paused program: a reader types `fib(10)`, not `n + 2`.  They were not.
    // `eval` compiles its synthetic fn through `parse_str`, which resolves under
    // `STD_SOURCE`, while the program had been registered under its own source — so a
    // stdlib call worked and a call to the program answered `<unavailable>`.
    //
    // Both sides are asserted together on purpose: a guard on the program call alone would
    // also pass on a build where nothing resolves at all.
    #[test]
    fn eval_calls_the_programs_own_functions_and_the_stdlib() {
        assert!(start(
            "fn fib(n: integer) -> integer { if n < 2 { return n; } fib(n-1) + fib(n-2) }\n\
             fn compute(n: integer) -> integer {\n  m = n + 2;\n  m\n}\n\
             fn main() { compute(40); }\n"
        ));
        SESSION.with(|s| {
            let mut g = s.borrow_mut();
            let sess = g.as_mut().expect("session");
            assert_eq!(apply(sess, "bp compute"), vec!["D:ok bp compute"]);
            assert!(apply(sess, "run")[0].starts_with("D:hit compute"));
            // A user fn on a literal.  fib(10) is 55, computed by hand.
            assert_eq!(apply(sess, "eval fib(10)"), vec!["D:eval fib(10)=55"]);
            // A user fn on the paused frame's own live local: compute's `n` is 40.
            assert_eq!(
                apply(sess, "eval fib(n - 30)"),
                vec!["D:eval fib(n - 30)=55"]
            );
            // The stdlib side of the split, which never broke and must not start.
            assert_eq!(
                apply(sess, "eval len(\"abc\")"),
                vec!["D:eval len(\"abc\")=3"]
            );
        });
    }

    #[test]
    fn a_text_or_vector_valued_eval_answers_its_value() {
        assert!(start(
            "fn shout(s: text) -> text { s.to_uppercase() }\n             fn evens(n: integer) -> vector<integer> { [for i in 0..n { i * 2 }] }\n             struct Stats { lo: integer, hi: integer }\n             fn stats(a: integer, b: integer) -> Stats { Stats { lo: a, hi: b } }\n             fn compute(n: integer, t: text) -> integer {\n  m = n + 2;\n  m + len(t) - 3\n}\n             fn main() { compute(40, \"abc\"); }\n"
        ));
        SESSION.with(|s| {
            let mut g = s.borrow_mut();
            let sess = g.as_mut().expect("session");
            assert_eq!(apply(sess, "bp compute"), vec!["D:ok bp compute"]);
            let hit = apply(sess, "run");
            assert!(hit[0].starts_with("D:hit compute"), "paused: {hit:?}");
            for (expr, want) in [
                ("n + 2", "42"),
                // A bare `text` local, read off the frame rather than re-entered: the
                // other half of the same question, and the one `eval_expr`'s own doc
                // comment used to name as unavailable.
                ("t", r#""abc""#),
                ("stats(1, 9)", r#"{"lo":1,"hi":9}"#),
                ("shout(\"hi\")", "\"HI\""),
                ("\"a\" + \"b\"", "\"ab\""),
                ("evens(4)", "[0,2,4,6]"),
            ] {
                assert_eq!(
                    apply(sess, &format!("eval {expr}")),
                    vec![format!("D:eval {expr}={want}")],
                    "evaluating `{expr}` over the paused frame"
                );
            }
            assert_eq!(apply(sess, "resume"), vec!["D:terminated"]);
        });
    }
}
