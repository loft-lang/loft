// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @F49 — REPL (interactive sessions)

//! @PLN12 phase 03 — interactive REPL session.
//!
//! Holds a parser with the stdlib loaded plus the accumulated session
//! statements.  Each [`ReplSession::eval`] appends the input to one shared
//! function scope and runs it, so a variable bound in one input is visible to
//! the next (`x = 1` then `x + 1` sees `x`).
//!
//! **Scope / strategy note.**  This re-evaluates the accumulated body in a
//! single shared scope each input.  As long as each binding's right-hand side
//! is deterministic and side-effect-free — any value type — that is
//! behaviourally correct: re-running yields the same values.  A statement with
//! *side effects* (I/O)
//! would re-run on each later input; eliminating that is the incremental,
//! stack-resident model (compile only the new statement, keep the variable
//! region on the stack across `reset_for_repl`) planned in
//! `plans/12-repl-and-introspection/03-state-reset-and-append.md`.  That
//! refinement sits behind this same `ReplSession` API.

use crate::compile;
use crate::data::{DefType, Type};
#[cfg(not(target_arch = "wasm32"))]
use crate::database::Parts;
use crate::diagnostics::{DiagEntry, Level};
use crate::introspect::{Options, Section};
use crate::parser::Parser;
use crate::state::State;
#[cfg(not(target_arch = "wasm32"))]
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, Write};
use std::panic::AssertUnwindSafe;
use std::path::Path;

/// @PLN14 arc A — initial word size of the session store.  It grows on demand
/// like any store; this only sets how much room the first few bindings get
/// without a realloc.
const SESSION_STORE_WORDS: u32 = 1024;

/// @PLN14 arc F — resume-image header: a magic so a foreign file is refused, and
/// a format version so an older image is refused rather than misread.
const SESSION_IMAGE_MAGIC: &[u8; 8] = b"LOFTSES1";
const SESSION_IMAGE_VERSION: u32 = 1;

/// @PLN14 arc F — the outcome of loading a resume image.  Every non-`Loaded`
/// variant means the session was left **untouched**, so the caller falls back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageLoad {
    /// The image was accepted; the session now holds its store-resident values.
    Loaded,
    /// No image file at that path — a first run.
    Missing,
    /// Not a session image, a different format version, truncated, or a store
    /// arena `Store::from_bytes` refused as structurally invalid.
    Malformed,
    /// A valid image from a build whose STORAGE LAYOUT differs (a changed struct,
    /// a different loft build, another endianness), or one referencing a type this
    /// session has not defined.  The values cannot be read as they were written,
    /// so they are refused rather than misread.
    SchemaMismatch,
}

/// Encode a [`SessionShape`] as two bytes for the resume image.
fn shape_tags(shape: SessionShape) -> (u8, u8) {
    match shape {
        SessionShape::Heap => (0, 0),
        SessionShape::TextInVector => (1, 0),
        SessionShape::Scalar(k) => (
            2,
            match k {
                ScalarKind::Integer => 0,
                ScalarKind::Float => 1,
                ScalarKind::Single => 2,
                ScalarKind::Boolean => 3,
                ScalarKind::Character => 4,
                ScalarKind::Text => 5,
                ScalarKind::SimpleEnum => 6,
            },
        ),
    }
}

/// Inverse of [`shape_tags`]; `None` on an unknown tag (a malformed image).
fn shape_from_tags(tag: u8, kind: u8) -> Option<SessionShape> {
    Some(match tag {
        0 => SessionShape::Heap,
        1 => SessionShape::TextInVector,
        2 => SessionShape::Scalar(match kind {
            0 => ScalarKind::Integer,
            1 => ScalarKind::Float,
            2 => ScalarKind::Single,
            3 => ScalarKind::Boolean,
            4 => ScalarKind::Character,
            5 => ScalarKind::Text,
            6 => ScalarKind::SimpleEnum,
            _ => return None,
        }),
        _ => return None,
    })
}

/// @PLN14 arc E — whether a fresh session starts with store-backed observing on.
///
/// **Default ON** since Step 8: observing a store-resident binding reads the
/// session store instead of replaying the accumulated body. `LOFT_NO_STORE_OBSERVE`
/// opts out, matching the repo's default-on `LOFT_NO_*` family.
///
/// Safe to default because what a session PRINTS is byte-identical either way —
/// `a_real_repl_session_prints_identically_with_the_flip` runs the real binary
/// over one script with the flag both ways and diffs stdout. The store serves
/// both renderings (display and own-format) precisely so this flip is invisible.
fn store_observe_default() -> bool {
    std::env::var("LOFT_NO_STORE_OBSERVE").is_err()
}

/// Path to the auto-resume session file (`~/.loft_session`), or `None` if the
/// home directory can't be located.  Shared by the interactive driver (which
/// replays it on launch and appends new state-changing inputs to it) and the
/// `--fresh` flag in `main` (which clears it).
pub fn session_file_path() -> Option<std::path::PathBuf> {
    dirs::home_dir().map(|h| h.join(".loft_session"))
}

thread_local! {
    /// Set while THIS thread is running program code whose panics the REPL catches
    /// and reports itself.  Read by the shared hook installed by [`silence_panics`].
    static PANICS_SILENCED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Suppress Rust's raw `thread panicked at …` output **on this thread** until the
/// returned guard drops.
///
/// The REPL catches a program's runtime panic and reports it cleanly, so the raw
/// backtrace is noise.  Silencing it by swapping the panic hook is wrong, though:
/// `std::panic::set_hook` is PROCESS-global, so in any multi-threaded host — a test
/// binary, an embedder running a session on a worker — the swap silences unrelated
/// threads for the duration, and two threads swapping at once can leave a silencing
/// hook installed permanently (each restores what it took, which may be the other's).
/// A swallowed assertion message is indistinguishable from a test that failed for no
/// reason.
///
/// So the hook is installed ONCE and consults a thread-local instead: the thread that
/// asked for silence gets it, every other thread keeps the default handler. The guard
/// restores the previous value on drop, so an unwinding panic cannot leave this thread
/// permanently silenced either — which the old swap could, since it never restored on
/// the panic path.
struct PanicSilence(bool);

impl Drop for PanicSilence {
    fn drop(&mut self) {
        PANICS_SILENCED.with(|s| s.set(self.0));
    }
}

fn silence_panics() -> PanicSilence {
    static INSTALL: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INSTALL.get_or_init(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            if !PANICS_SILENCED.with(std::cell::Cell::get) {
                previous(info);
            }
        }));
    });
    PanicSilence(PANICS_SILENCED.with(|s| s.replace(true)))
}

/// Run the interactive `loft>` REPL.
///
/// Reads inputs (one statement per line, multi-line accumulated on
/// [`Eval::NeedMore`]) and writes the prompt, messages, and parse errors to
/// `chrome` (stderr in the CLI).  Evaluated **results** are printed by the
/// program itself to process stdout, so a terminal sees them and a piped caller
/// can capture them.  Returns when input reaches EOF or the user types `:quit`.
///
/// When the process's stdin is an interactive terminal, input is read through a
/// line editor (arrow-key history, in-line editing) and `input` is ignored.
/// Otherwise — a pipe, a file, a test harness, or a wasm build — input is read
/// plainly from `input`, so captured/piped output is byte-for-byte unchanged.
///
/// A runtime panic (failed `assert`, overflow) is caught — execution runs on a
/// throwaway clone of the database, so the session survives; the loop reports it
/// and continues.
///
/// # Errors
/// Returns an I/O error from loading the stdlib or writing to `chrome`.
pub fn run_repl<R: BufRead, W: Write>(
    ctx: &ResolutionContext,
    input: R,
    chrome: &mut W,
) -> std::io::Result<()> {
    let mut session = ReplSession::open(ctx)?;
    // @PLN16 G1 — the REPL is the interactive debugger surface: a breakpoint hit
    // suspends into the paused sub-mode (inspect / edit / step), rather than the
    // record-and-continue mode programmatic callers use.
    session.debug_stepping(true);
    writeln!(chrome, "loft REPL — :help for commands, :quit to exit")?;
    // A runtime error inside `eval` is caught below and reported cleanly, so the
    // user should not also see Rust's raw "thread panicked at …" backtrace.
    // Thread-scoped, not a global hook swap — see `silence_panics`.
    let _silence = silence_panics();
    run_loop(ctx, &mut session, input, chrome)
}

/// @PLN16 M5a — the **file-run debugger** (`loft debug prog.loft:42`).  Loads `file`
/// (parsed under its real path, so breakpoints address it by `file:line`), breaks at
/// `file:line`, auto-runs `main()` to the breakpoint, then drops into the interactive
/// `(dbg)` prompt — the same paused engine as the REPL, reading the rest from `input`.
///
/// Reports cleanly and returns without entering the loop when `file` can't be
/// read/parsed, has no `main()`, or `line` carries no breakable code (hinting the
/// breakable lines).
///
/// # Errors
/// Returns an I/O error from the input/output streams.
/// `lib_dirs` are the `--lib` import paths, so the debugged file can `use` a library
/// exactly as running it does.  Passing them is not optional: without them a program
/// whose `use` resolves through `--lib` fails to load and the debugger reports
/// `Library '<x>' not found` on a file that runs fine — @PLN120 E1, reported by moros
/// as "the debugger does not work on real programs".  The `--rpc` and `--serve` paths
/// always did this; this one did not, which is why the fault was interactive-only.
pub fn run_file_debug<R: BufRead, W: Write>(
    stdlib_dir: &str,
    lib_dirs: &[String],
    file: &str,
    line: u32,
    input: R,
    chrome: &mut W,
) -> std::io::Result<()> {
    let ctx = ResolutionContext {
        stdlib_dir: stdlib_dir.to_string(),
        lib_dirs: lib_dirs.to_vec(),
    };
    let mut session = ReplSession::open(&ctx)?;
    match session.load_program(file) {
        Ok(Ok(())) => {}
        Ok(Err(diags)) => {
            for d in diags {
                writeln!(chrome, "{}", d.to_string_compact())?;
            }
            return Ok(());
        }
        Err(e) => {
            writeln!(chrome, "cannot read {file}: {e}")?;
            return Ok(());
        }
    }
    if !session.defines_function("main") {
        writeln!(chrome, "{file} has no `main()` to run under the debugger")?;
        return Ok(());
    }
    // Validate the line is breakable *before* arming, else hint the breakable lines.
    let breakable = session.breakable_lines_in_file(file);
    if !breakable.contains(&line) {
        if breakable.is_empty() {
            writeln!(chrome, "no breakable code found in {file}")?;
        } else {
            writeln!(
                chrome,
                "no breakable code at {file}:{line}. Breakable lines: {}",
                breakable
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )?;
        }
        return Ok(());
    }
    session.debug_stepping(true);
    session.add_file_breakpoint(file, line);
    // A resumed-run panic is caught + reported below, so the raw handler is noise.
    // Thread-scoped, not a global hook swap — see `silence_panics`.
    let _silence = silence_panics();
    let _ = writeln!(
        chrome,
        "loft debugger — break at {file}:{line}.  :help for commands, :continue to run, :quit to exit"
    );
    // Auto-run the entry to the breakpoint, then hand off to the interactive loop.
    let mut pending = String::new();
    match process_line("main()", &mut session, &mut pending, &ctx, None, chrome) {
        Ok(_) => {
            if !session.is_debugging() {
                let _ = writeln!(
                    chrome,
                    "program finished without stopping at {file}:{line} (was the line reached?)"
                );
            }
            run_loop(&ctx, &mut session, input, chrome)
        }
        Err(e) => Err(e),
    }
}

/// Pick the input driver: the interactive line editor when stdin is a terminal,
/// the plain reader otherwise.  wasm has no line editor, so it always reads
/// plainly.
#[cfg(not(target_arch = "wasm32"))]
fn run_loop<R: BufRead, W: Write>(
    ctx: &ResolutionContext,
    session: &mut ReplSession,
    input: R,
    chrome: &mut W,
) -> std::io::Result<()> {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        run_interactive(ctx, session, chrome)
    } else {
        run_piped(ctx, session, input, chrome)
    }
}

/// wasm32 has no terminal, so there is only the piped path.  Kept in step with the
/// host `run_loop` above: both take the [`ResolutionContext`], and a signature change
/// here does not surface in a host `cargo clippy` — only the wasm32 build compiles this
/// arm, which is what `make check-wasm-threads` and the suite's wasm32 rlib step cover.
#[cfg(target_arch = "wasm32")]
fn run_loop<R: BufRead, W: Write>(
    ctx: &ResolutionContext,
    session: &mut ReplSession,
    input: R,
    chrome: &mut W,
) -> std::io::Result<()> {
    run_piped(ctx, session, input, chrome)
}

/// The continuation-aware prompt: the `(dbg)` prompt while suspended at a
/// breakpoint (@PLN16 G1), the dotted prompt while a multi-line statement is still
/// open, the primary prompt otherwise.
fn prompt(pending: &str, debugging: bool) -> &'static str {
    if !pending.is_empty() {
        "..... > "
    } else if debugging {
        "(dbg) "
    } else {
        "loft> "
    }
}

/// Read input plainly from `input`, one line at a time.  Used for pipes, files,
/// test harnesses, and wasm — anywhere there is no interactive terminal — so the
/// captured output stays stable.
fn run_piped<R: BufRead, W: Write>(
    ctx: &ResolutionContext,
    session: &mut ReplSession,
    mut input: R,
    chrome: &mut W,
) -> std::io::Result<()> {
    let mut pending = String::new();
    let mut line = String::new();
    loop {
        write!(chrome, "{}", prompt(&pending, session.is_debugging()))?;
        chrome.flush()?;
        line.clear();
        if input.read_line(&mut line)? == 0 {
            break; // EOF
        }
        // No session path: a piped/file/test run never persists or resumes.
        if process_line(line.trim_end(), session, &mut pending, ctx, None, chrome)? {
            break; // :quit
        }
    }
    Ok(())
}

/// True when `s` is a plain identifier (`[A-Za-z_][A-Za-z0-9_]*`).  Keeps
/// synthetic definition names (`main_vector<…>`, `__tuple<…>`) out of the
/// completion list.
#[cfg(not(target_arch = "wasm32"))]
fn is_plain_ident(s: &str) -> bool {
    let mut cs = s.chars();
    cs.next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_alphanumeric() || c == '_')
}

/// The set of maximal identifier tokens (`[A-Za-z_][A-Za-z0-9_]*`) in `expr` — the names a debug
/// eval expression could bind to a frame local (@PLN98 P1). Over-inclusive is harmless (an extra
/// name matches no local); it must not MISS a referenced name (that would leave it unbound → a clean
/// eval failure, never a wrong value). Field/method names after `.` are included too — they just do
/// not match a local. Does not exclude keywords; a keyword never names a frame local.
fn expr_idents(expr: &str) -> std::collections::HashSet<&str> {
    let mut out = std::collections::HashSet::new();
    let b = expr.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_alphabetic() || b[i] == b'_' {
            let s = i;
            while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.insert(&expr[s..i]);
        } else {
            i += 1;
        }
    }
    out
}

/// True when `method` is a member a user can call as `recv.method(...)`: a plain
/// identifier that is neither a compiler-internal (`__…`) nor an operator
/// overload — operator methods register as `Op<Name>` (`Op` + an uppercase
/// letter) and are invoked through the operator, not by name.
#[cfg(not(target_arch = "wasm32"))]
fn is_callable_method(method: &str) -> bool {
    is_plain_ident(method)
        && !method.starts_with("__")
        && !(method.len() > 2
            && method.starts_with("Op")
            && method.as_bytes()[2].is_ascii_uppercase())
}

/// The methods defined on the type named `type_name`, each rendered with a
/// trailing `(` so completion shows it is callable (and the cursor lands inside
/// the call).  Methods register under the length-prefixed name
/// `t_<len><type_name>_<method>` — the same `Type::name` the receiver resolves
/// to — so one `strip_prefix` recovers exactly this type's methods, for structs,
/// text, vectors, and every base type alike.
#[cfg(not(target_arch = "wasm32"))]
fn methods_for_type(data: &crate::data::Data, type_name: &str) -> Vec<String> {
    let prefix = format!("t_{}{type_name}_", type_name.len());
    let mut out = Vec::new();
    for d in 0..data.definitions() {
        let def = data.def(d);
        if def.def_type == DefType::Function
            && let Some(method) = def.name.strip_prefix(&prefix)
            && is_callable_method(method)
        {
            out.push(format!("{method}("));
        }
    }
    out
}

/// The `:`-commands Tab completion offers when the line starts with `:`.
#[cfg(not(target_arch = "wasm32"))]
const REPL_COMMANDS: [&str; 10] = [
    "quit", "help", "reset", "fns", "vars", "type", "break", "bytecode", "rust", "slots",
];

/// The completion model the live completer matches against, rebuilt by the
/// interactive loop after each input.  `names` are the bare identifiers
/// (globals + session variables); `members` maps a dotted receiver — a
/// struct-typed variable to its field names, or an enum *type* name to its
/// variant names — to the candidates valid after `receiver.`.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
pub struct CompletionModel {
    /// Bare-identifier candidates: globals (user + stdlib fns, type names) and
    /// the variables bound this session.  Sorted + deduped.
    pub names: Vec<String>,
    /// Dotted-access candidates keyed by receiver token: a struct variable → its
    /// field names; an enum type name → its variant names.  Each list sorted.
    pub members: HashMap<String, Vec<String>>,
}

/// The trailing identifier of `s` (`"1 + p"` → `"p"`), or `None` when `s` does
/// not end in an identifier character (`"arr[0]"` → `None`).  Reads the receiver
/// token immediately before a `.`.
#[cfg(not(target_arch = "wasm32"))]
fn trailing_ident(s: &str) -> Option<&str> {
    let start = s
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map_or(s.len(), |(i, _)| i);
    let id = &s[start..];
    (!id.is_empty()).then_some(id)
}

/// Pure completion logic, shared by the live completer and its unit tests.
/// Given the `model` and the cursor at byte `pos` in `line`, returns the offset
/// where the replaced word starts and the matching candidates.  Three contexts,
/// in priority order:
///
/// - a leading `:word` → [`REPL_COMMANDS`];
/// - `receiver.partial` (the cursor sits past a `.`) → **only** that receiver's
///   `members`, never the global list — an unresolved receiver (`foo.`,
///   `arr[0].`) yields nothing rather than leaking globals after the dot;
/// - otherwise a bare identifier prefix → `model.names`.  An empty *bare* prefix
///   yields nothing (a stray Tab never dumps the whole list), but an empty
///   prefix right after `receiver.` lists all of that receiver's members.
#[cfg(not(target_arch = "wasm32"))]
fn complete_word(model: &CompletionModel, line: &str, pos: usize) -> (usize, Vec<String>) {
    let head = &line[..pos];
    // `:command` — only while the cursor is still inside the leading `:word`.
    if let Some(after) = head.strip_prefix(':')
        && !after.contains(char::is_whitespace)
    {
        let out = REPL_COMMANDS
            .iter()
            .filter(|c| c.starts_with(after))
            .map(|c| (*c).to_string())
            .collect();
        return (1, out);
    }
    // Identifier under the cursor: walk back over identifier characters.
    let start = head
        .char_indices()
        .rev()
        .take_while(|(_, c)| c.is_alphanumeric() || *c == '_')
        .last()
        .map_or(pos, |(i, _)| i);
    let prefix = &head[start..];
    // Dotted member access: once the cursor is past a `.`, only this receiver's
    // members are valid candidates — never the global identifier list.
    if let Some(stem) = head[..start].strip_suffix('.') {
        let out = trailing_ident(stem)
            .and_then(|recv| model.members.get(recv))
            .map(|ms| {
                ms.iter()
                    .filter(|m| m.starts_with(prefix))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        return (start, out);
    }
    if prefix.is_empty() {
        return (start, Vec::new());
    }
    let out = model
        .names
        .iter()
        .filter(|n| n.starts_with(prefix))
        .cloned()
        .collect();
    (start, out)
}

/// rustyline glue: completes loft identifiers and `:`-commands from the live
/// session.  `names` is refreshed by the interactive loop after each input.
/// Hinting, highlighting, and validation use rustyline's defaults.
#[cfg(not(target_arch = "wasm32"))]
struct ReplHelper {
    model: CompletionModel,
}

#[cfg(not(target_arch = "wasm32"))]
impl rustyline::completion::Completer for ReplHelper {
    type Candidate = String;
    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<String>)> {
        Ok(complete_word(&self.model, line, pos))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl rustyline::hint::Hinter for ReplHelper {
    type Hint = String;
}
#[cfg(not(target_arch = "wasm32"))]
impl rustyline::highlight::Highlighter for ReplHelper {}
#[cfg(not(target_arch = "wasm32"))]
impl rustyline::validate::Validator for ReplHelper {}
#[cfg(not(target_arch = "wasm32"))]
impl rustyline::Helper for ReplHelper {}

/// Read input through a line editor (arrow-key history, in-line editing, and Tab
/// completion of session identifiers + `:`-commands).  The editor owns the
/// prompt and reads the terminal directly, so the plain `input` reader is not
/// used here.  History persists to `~/.loft_history` across sessions
/// (best-effort — a failure to load or save is ignored).  Ctrl-C cancels the
/// statement in progress; Ctrl-D at an empty prompt quits.
#[cfg(not(target_arch = "wasm32"))]
fn run_interactive<W: Write>(
    ctx: &ResolutionContext,
    session: &mut ReplSession,
    chrome: &mut W,
) -> std::io::Result<()> {
    use rustyline::error::ReadlineError;
    // `List` shows all candidates on an ambiguous Tab rather than cycling.
    let config = rustyline::Config::builder()
        .completion_type(rustyline::CompletionType::List)
        .build();
    let mut rl: rustyline::Editor<ReplHelper, rustyline::history::DefaultHistory> =
        match rustyline::Editor::with_config(config) {
            Ok(rl) => rl,
            // No usable terminal after all — fall back to the plain reader.
            Err(_) => return run_piped(ctx, session, std::io::stdin().lock(), chrome),
        };
    rl.set_helper(Some(ReplHelper {
        model: CompletionModel::default(),
    }));
    let history = dirs::home_dir().map(|h| h.join(".loft_history"));
    if let Some(path) = &history {
        let _ = rl.load_history(path);
    }
    // Auto-resume: replay the previous session, then append new state-changing
    // inputs to the same file.  Interactive-only — `run_piped` (pipes, files,
    // tests, wasm) never touches it, so captured output stays deterministic.
    let session_path = session_file_path();
    if let Some(path) = &session_path {
        let stats = session.resume_from(path);
        if stats.restored > 0 {
            let skipped = if stats.skipped > 0 {
                format!(" ({} skipped)", stats.skipped)
            } else {
                String::new()
            };
            writeln!(
                chrome,
                "restored {} statement(s) from last session{skipped}",
                stats.restored
            )?;
        }
        let _ = session.enable_persistence(path);
    }
    // Seed Tab completion with whatever resume restored, then keep it current.
    if let Some(h) = rl.helper_mut() {
        h.model = session.completion_model();
    }
    let mut pending = String::new();
    loop {
        match rl.readline(prompt(&pending, session.is_debugging())) {
            Ok(line) => {
                let _ = rl.add_history_entry(line.as_str());
                let quit = process_line(
                    line.trim_end(),
                    session,
                    &mut pending,
                    ctx,
                    session_path.as_deref(),
                    chrome,
                )?;
                // A new def or binding changes the candidate set — refresh it.
                if let Some(h) = rl.helper_mut() {
                    h.model = session.completion_model();
                }
                if quit {
                    break; // :quit
                }
            }
            // Ctrl-C drops the statement in progress and returns to a fresh prompt.
            Err(ReadlineError::Interrupted) => pending.clear(),
            // Ctrl-D (or any read error) ends the session.
            Err(ReadlineError::Eof) => break,
            Err(_) => break,
        }
    }
    if let Some(path) = &history {
        let _ = rl.save_history(path);
    }
    Ok(())
}

/// Process one input line against the session: dispatch a `:`-command, or feed
/// the line into the accumulating statement and evaluate it when complete.
/// Returns `Ok(true)` when the user asked to quit (`:quit`).  Shared by both
/// input drivers so interactive and piped sessions behave identically.
fn process_line<W: Write>(
    trimmed: &str,
    session: &mut ReplSession,
    pending: &mut String,
    ctx: &ResolutionContext,
    session_path: Option<&Path>,
    chrome: &mut W,
) -> std::io::Result<bool> {
    // @PLN16 G1 — while suspended at a breakpoint, inputs drive the paused
    // sub-mode (step verbs / value edits / frame eval), not a fresh evaluation.
    // A debug op runs user code (a resumed program, a frame expression); catch a
    // runtime panic so it abandons the debug session rather than killing the REPL,
    // mirroring the eval path below.
    if pending.is_empty() && session.is_debugging() {
        let outcome =
            std::panic::catch_unwind(AssertUnwindSafe(|| handle_paused(trimmed, session, chrome)));
        let res = match outcome {
            Ok(res) => res,
            Err(payload) => {
                session.abort_debug();
                // @PLN120 E3a — SAY WHAT HAPPENED.  This used to print a fixed
                // string and drop the payload, so every distinct cause looked
                // identical and the one failure the debugger exists to explain —
                // its own — was the one it could not.  It also left the user at a
                // `loft>` prompt where `:continue` answers "unknown command",
                // which reads as a typo rather than as "the session is over".
                writeln!(
                    chrome,
                    "runtime error in the paused run: {}\n  \
                     the debug session ended (the REPL session is preserved) — \
                     step/continue no longer apply; re-run `loft debug <file>:<line>` \
                     to start a new one",
                    panic_message(&payload)
                )?;
                return Ok(false);
            }
        };
        return res;
    }
    // `:`-commands are only recognised at the start of a fresh statement.
    if pending.is_empty() && trimmed.starts_with(':') {
        let mut words = trimmed[1..].split_whitespace();
        let cmd = words.next().unwrap_or("");
        let filter: Vec<String> = words.map(str::to_string).collect();
        match cmd {
            "quit" | "q" => return Ok(true),
            "help" | "h" => writeln!(
                chrome,
                "commands: :quit  :help  :reset  :fns  :vars  :type <expr>  \
                 :break <fn>[:<line>] [if <cond>]  :trace <fn> <expr>,…  \
                 :bytecode [fn]  :rust [fn]  :slots [fn]"
            )?,
            "reset" => {
                // Re-open with the SAME resolution inputs.  Rebuilding from
                // `stdlib_dir` alone silently un-libbed a session that was working —
                // the @PLN120 E.1 fault turned inward, and latent because the only
                // lib-bearing REPL route (`loft repl --lib`) was broken too.
                *session = ReplSession::open(ctx)?;
                // Clearing state clears the persisted session too, so the next
                // launch starts clean; keep persisting to the now-empty file.
                if let Some(path) = session_path {
                    ReplSession::clear_session(path);
                    let _ = session.enable_persistence(path);
                }
                writeln!(chrome, "session reset.")?;
            }
            "bytecode" => session.introspect(Section::Bytecode, filter),
            "rust" => session.introspect(Section::Rust, filter),
            "slots" => session.introspect(Section::Slots, filter),
            "ownership" => session.introspect(Section::Ownership, filter),
            "fns" => session.list_fns(),
            "vars" => match session.show_vars() {
                Ok(true) => {}
                Ok(false) => writeln!(chrome, "no variables bound yet")?,
                Err(diags) => {
                    for d in diags {
                        writeln!(chrome, "{}", d.to_string_compact())?;
                    }
                }
            },
            "type" => {
                let expr = filter.join(" ");
                match session.infer_type(&expr) {
                    Some(t) => println!("{t}"),
                    None => writeln!(chrome, "could not infer the type of `{expr}`")?,
                }
            }
            "break" => match filter.first().map(String::as_str) {
                None if session.breakpoints().is_empty() => {
                    writeln!(
                        chrome,
                        "no breakpoints (`:break <fn>` / `<fn>:<line>` to add)"
                    )?;
                }
                None => writeln!(chrome, "breakpoints: {}", session.breakpoints().join(", "))?,
                Some("clear") => {
                    session.clear_breakpoints();
                    writeln!(chrome, "breakpoints cleared")?;
                }
                Some(_) => {
                    let spec = filter.join(" ");
                    if spec.parse::<u32>().is_ok() {
                        // A bare line isn't unique (same line number in every
                        // function; the REPL has no real file).  Steer to the
                        // function-scoped forms.
                        writeln!(
                            chrome,
                            "a bare line isn't unique — use `:break <fn>` or `<fn>:<line>`"
                        )?;
                    } else {
                        session.add_breakpoint(&spec);
                        writeln!(chrome, "breakpoint set: {spec}")?;
                    }
                }
            },
            // @PLN16 rich-bp — `:trace <fn>[:<line>] <expr>, <expr>` sets a tracepoint:
            // on each hit it logs the expressions and the run continues (no pause).
            "trace" => {
                let spec = filter.join(" ");
                session.add_tracepoint(&spec);
                writeln!(chrome, "tracepoint set: {spec}")?;
            }
            other => writeln!(chrome, "unknown command: :{other}  (:help)")?,
        }
        return Ok(false);
    }
    pending.push_str(trimmed);
    pending.push('\n');
    let src = pending.clone();
    // Catch a runtime panic so a bad input never kills the REPL.  AssertUnwindSafe
    // is sound here: a panic corrupts only the per-eval database clone, never
    // `session`'s own state.
    match std::panic::catch_unwind(AssertUnwindSafe(|| session.eval(&src))) {
        Ok(Eval::Ran) => {
            pending.clear();
            print_trace(session, chrome)?; // @PLN16 rich-bp — tracepoints fired this run
        }
        Ok(Eval::Paused) => {
            // The run hit a breakpoint and suspended — show the frame and enter
            // the paused sub-mode; the next inputs are routed to `handle_paused`.
            pending.clear();
            print_trace(session, chrome)?;
            print_pause(session, chrome)?;
        }
        Ok(Eval::NeedMore) => {} // keep accumulating; continuation prompt next
        Ok(Eval::Error(diags)) => {
            for d in diags {
                writeln!(chrome, "{}", d.to_string_compact())?;
            }
            pending.clear();
        }
        Err(payload) => {
            // Carry the cause, do not just name the category — the same fix @PLN120
            // E3a made for the debug-abandon path.  A fixed string here collapses
            // every distinct fault into one line, and it hid a compiler-invariant
            // assert (the E.4 rollback guard) whose whole purpose is to explain itself.
            writeln!(
                chrome,
                "runtime error: {} (session preserved; :reset to clear state)",
                panic_message(&payload)
            )?;
            pending.clear();
        }
    }
    Ok(false)
}

/// The text of a caught panic.  `catch_unwind` hands back a `Box<dyn Any>` whose
/// payload is a `&'static str` for `panic!("literal")` and a `String` for a
/// formatted one; anything else has no readable text.  Discarding it — which is
/// what the debugger's abandon path used to do — collapses every distinct runtime
/// fault into one message (@PLN120 E3a).
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        (*s).to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "a panic with no message".to_string()
    }
}

/// @PLN16 G1 — handle one input while **suspended** at a breakpoint.  Step verbs
/// resume execution (`:step`/`:s` into, `:next`/`:n` over, `:finish`/`:o` out,
/// `:continue`/`:c` to the next breakpoint or the end); `name = <expr>` edits the
/// live frame (scalar / text / enum / `pt.x` / `v[i]` / whole struct-or-vector);
/// `:undo`/`:u` and `:redo`/`:r` walk this suspension's edit history; any other
/// expression is **evaluated against the frame** (`n * 2`, `pt.x`); `:vars` re-shows
/// the frame; `:quit`/`:q` leaves the REPL.  Returns `Ok(true)` only to quit.  Verbs
/// work with or without the leading colon, so a paused user can type `step` or `:step`.
fn handle_paused<W: Write>(
    trimmed: &str,
    session: &mut ReplSession,
    chrome: &mut W,
) -> std::io::Result<bool> {
    use crate::debugger::StepMode;
    let t = trimmed.trim();
    let cmd = t.strip_prefix(':').unwrap_or(t);
    // A colon-less verb is a convenience, but the verb names collide with ordinary
    // loft locals — `s`, `n`, `c`, `r`, `o`, `u`, `q` are as common as names get,
    // and `step` / `next` / `vars` are plausible too.  Typing `step` to see the
    // local `step` silently STEPPED the program instead: the wrong action, and one
    // that moves the frame you were reading.  When the paused frame actually has a
    // local by that name, the name wins; `:step` always means the verb.
    if !t.starts_with(':')
        && session
            .paused_frame()
            .is_some_and(|f| f.locals.iter().any(|(n, _)| n == cmd))
        && let Some(v) = session.debug_eval(cmd)
    {
        println!("{v}");
        return Ok(false);
    }
    // @PLN16 M3 — `:watch <expr>` sets a watchpoint (a scalar `pt.x` / `v[i]`); `:watch`
    // lists; `:watch clear` clears.  Handled before the match because it takes an arg.
    if cmd == "watch" || cmd.starts_with("watch ") {
        let arg = cmd.strip_prefix("watch").unwrap_or("").trim();
        if arg.is_empty() {
            let ws = session.watchpoints();
            if ws.is_empty() {
                writeln!(
                    chrome,
                    "no watchpoints (`:watch pt.x` / `:watch v[0]` to add)"
                )?;
            } else {
                writeln!(chrome, "watchpoints: {}", ws.join(", "))?;
            }
        } else if arg == "clear" {
            session.clear_watchpoints();
            writeln!(chrome, "watchpoints cleared")?;
        } else if session.add_watchpoint(arg) {
            writeln!(
                chrome,
                "watching {arg} — :continue and the run stops when it changes"
            )?;
        } else {
            writeln!(
                chrome,
                "can't watch `{arg}` — watch a scalar struct field (`pt.x`) or vector element (`v[i]`)"
            )?;
        }
        return Ok(false);
    }
    match cmd {
        "quit" | "q" => return Ok(true),
        "step" | "s" => step_and_report(session, StepMode::Into, chrome)?,
        "next" | "n" => step_and_report(session, StepMode::Over, chrome)?,
        "finish" | "o" => step_and_report(session, StepMode::Out, chrome)?,
        "continue" | "c" => step_and_report(session, StepMode::Continue, chrome)?,
        "vars" => print_pause(session, chrome)?,
        "vars all" => print_pause_filtered(session, chrome, true)?,
        "undo" | "u" => {
            if session.debug_undo() {
                print_pause(session, chrome)?;
            } else if let Some((label, why)) = session.dropped_undo_here().first() {
                // @PLN120 F — an empty stack because an edit was DROPPED is a different
                // answer from "you made no edits", and saying the generic one here would
                // re-lose the edit the drop notice just explained.
                writeln!(
                    chrome,
                    "nothing to undo here — the edit to `{label}` is no longer \
                     undoable ({why})"
                )?;
            } else {
                // @PLN120 F.4 — ":undo" reads as time-travel, so an empty stack after
                // plain stepping looked like a broken feature (it is correct: stepping
                // makes no edits).  Name the boundary and point at the tool that does
                // move the program backwards.
                writeln!(
                    chrome,
                    "no edits to undo at this pause — `:undo` reverts edits YOU made \
                     (`name = <expr>`), so plain stepping leaves nothing to undo; to move \
                     the program backwards, use reverse-stepping on the RPC surface \
                     (`stepBack`, depth via LOFT_REVERSE_DEPTH)"
                )?;
            }
        }
        "redo" | "r" => {
            if session.debug_redo() {
                print_pause(session, chrome)?;
            } else {
                writeln!(chrome, "nothing to redo")?;
            }
        }
        "help" | "h" => writeln!(
            chrome,
            "paused: :step(:s) into  :next(:n) over  :finish(:o) out  :continue(:c)  \
             :vars  :undo(:u) an edit  :redo(:r)  :watch <expr>  |  `name = <expr>` edits a local \
             (scalar / text / enum / `pt.x` / `v[i]` / whole struct/vector)  |  any \
             expression is evaluated at the frame  |  :quit"
        )?,
        _ => match parse_assign(t) {
            // `name = <expr>` writes the live frame (picked up on resume); the RHS
            // is evaluated against the frame, so an expression works too.
            Some((name, rhs)) if session.debug_set(name, rhs) => {
                print_pause(session, chrome)?;
            }
            // @PLN120 A — a local the frame does not HOLD is refused, and the
            // refusal names the reason rather than implying a typo.
            Some((name, _)) => match session.unheld_local_reason(name) {
                Some(why) => writeln!(chrome, "can't set it: {why}")?,
                None => writeln!(
                    chrome,
                    "couldn't set `{name}` — unknown local, or a value whose type \
                     doesn't match the local"
                )?,
            },
            // Anything else is an expression read against the frame's live values;
            // the value prints to stdout like a normal REPL result.
            None => match session.debug_eval(t) {
                Some(v) => println!("{v}"),
                None => match session.unheld_local_reason(t) {
                    Some(why) => writeln!(chrome, "{why}")?,
                    None => writeln!(
                        chrome,
                        "couldn't evaluate `{t}` at the frame \
                         (:step/:next/:finish/:continue, `name = <expr>` to edit, :help)"
                    )?,
                },
            },
        },
    }
    Ok(false)
}

/// Print the current paused frame (function + its in-scope variables), or nothing
/// when no longer paused.
fn print_pause<W: Write>(session: &ReplSession, chrome: &mut W) -> std::io::Result<()> {
    print_pause_filtered(session, chrome, false)
}

/// `print_pause`, with `all` to include the compiler's scratch variables
/// (`:vars all`).  @PLN120 D1 — the default frame is the user's own locals; the
/// temps are still there for compiler work, one word away.
fn print_pause_filtered<W: Write>(
    session: &ReplSession,
    chrome: &mut W,
    all: bool,
) -> std::io::Result<()> {
    if let Some(f) = session.paused_frame() {
        let vars: Vec<String> = if all {
            f.locals.iter().map(|(n, v)| format!("{n} = {v}")).collect()
        } else {
            f.user_locals()
                .into_iter()
                .map(|(n, v)| format!("{n} = {v}"))
                .collect()
        };
        let hidden = f.locals.len() - vars.len();
        let note = if hidden > 0 && !all {
            format!("   (+{hidden} compiler temp(s) — `:vars all`)")
        } else {
            String::new()
        };
        writeln!(
            chrome,
            "⏸ paused in {} | {}{note}",
            f.function,
            vars.join(", ")
        )?;
    }
    Ok(())
}

/// Resume per `mode` and report: the new frame if it paused again, else that the
/// run finished and the sub-mode is left.
fn step_and_report<W: Write>(
    session: &mut ReplSession,
    mode: crate::debugger::StepMode,
    chrome: &mut W,
) -> std::io::Result<()> {
    let paused = session.debug_step(mode);
    print_trace(session, chrome)?;
    // @PLN16 M3 — a watchpoint that fired during this resume is what stopped us; name it.
    if let Some(hit) = session.take_watch_hit() {
        writeln!(
            chrome,
            "⏯ watchpoint: {} changed {} → {}",
            hit.label, hit.old, hit.new
        )?;
    }
    if paused {
        print_pause(session, chrome)?;
    } else {
        writeln!(chrome, "▶ resumed — run finished")?;
    }
    Ok(())
}

/// @PLN16 rich-bp — print the tracepoint emissions from the most recent resume, one
/// `⤳ <label> | k = v …` line per hit-batch (a tracepoint logs and the run continues).
fn print_trace<W: Write>(session: &mut ReplSession, chrome: &mut W) -> std::io::Result<()> {
    let lines = session.take_trace_output();
    if !lines.is_empty() {
        writeln!(chrome, "⤳ trace | {}", lines.join(", "))?;
    }
    Ok(())
}

/// Parse a `name = <expr>` edit typed at the paused prompt into `(name, rhs)`: a
/// single plain identifier on the left, `=`, a non-empty right side.  `None` for a
/// non-assignment — a read expression, a comparison (`n == 5`, whose `rhs` would
/// start with `=`), or a compound assign (`n += 1`, whose `lhs` isn't a bare
/// identifier) — so those route to frame evaluation instead.  The RHS is left
/// unparsed: [`ReplSession::debug_set`] evaluates it against the frame.
fn parse_assign(s: &str) -> Option<(&str, &str)> {
    let (lhs, rhs) = s.split_once('=')?;
    let name = lhs.trim();
    // A bare local (`n`), a struct-field path (`pt.x`), or a vector element (`v[1]`):
    // `.`, `[`, `]` are allowed so the paused prompt can route field / element edits;
    // `debug_set` splits on them.  A leading digit / `.` / `[` is never a valid local.
    if name.is_empty()
        || !name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '[' | ']'))
        || name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_digit() || matches!(c, '.' | '['))
    {
        return None;
    }
    let rhs = rhs.trim();
    // A leading `=` means the operator was `==` (a comparison), not an assignment.
    if rhs.is_empty() || rhs.starts_with('=') {
        return None;
    }
    Some((name, rhs))
}

/// Reduce `Type::show`'s debug form to the loft-source type name: drop the
/// `[...]` dep-tracking list, then unwrap a `ref(...)` reference wrapper
/// (`vector<integer>["__vdb_1"]` → `vector<integer>`; `ref(P)` → `P`).  Shared by
/// the value-snapshot capture (its cap-fn return type + `show_loft` schema
/// lookup) and Tab completion's struct-field resolution.
/// @PLN98 P1b — whether a base type name is an INLINE scalar (rides the call
/// frame base, so [`State::eval_frame_reenter`] can read it straight back), as
/// opposed to a heap value (struct / vector / collection — destination-passed,
/// serialised via `.to_json()` instead).  `character` counts (an inline `u32`).
fn is_scalar_type_name(t: &str) -> bool {
    matches!(
        t,
        "integer" | "float" | "single" | "boolean" | "character" | "byte"
    ) || t.starts_with("integer(")
}

fn base_type_name(show: &str) -> String {
    // A trailing `?` is part of the SOURCE name, and `Type::show` puts both of its debug
    // decorations INSIDE it — the dep list (`text["at"]?`) and the `ref(…)` wrapper
    // (`ref(P)?`).  So the `?` has to come off first and go back on afterwards, rather
    // than be reduced in place: splitting at `[` silently DROPPED it (a `text?` then took
    // the non-null capture wrapper, reaching the raw `Str`-off-the-stack read @P293
    // forbids), and the `ref(…)` unwrap failed outright against it (`ref(P)?` went to the
    // parser as a type it cannot spell, so a nullable struct never captured at all).
    // Both read as "the debugger declines a nullable"; only `integer?`, which carries
    // neither decoration, came through — loft#1459.
    let (body, opt) = show.strip_suffix('?').map_or((show, ""), |b| (b, "?"));
    let base = body.split('[').next().unwrap_or(body);
    let base = base
        .strip_prefix("ref(")
        .and_then(|s| s.strip_suffix(')'))
        .unwrap_or(base);
    format!("{base}{opt}")
}

/// Render an `f64` as a loft `float` literal — always with a decimal point so a
/// whole number like `3` re-parses as `float`, not `integer`.  The f64-typed
/// adapter over the shared round-trip rule
/// ([`crate::state::loft_float_literal`]); the frame renderer uses the same rule.
fn float_literal(v: f64) -> String {
    crate::state::loft_float_literal(&v.to_string())
}

/// Render `raw` as a quoted, escaped loft `text` literal — the form the parser
/// re-reads.  Used by the REPL.X value-snapshot to store a captured text binding
/// as `name = "…"`.  Delegates to the shared
/// [`state::loft_text_literal`](crate::state::loft_text_literal) so the snapshot
/// and the breakpoint frame renderer escape identically.
fn escape_loft_text(raw: &str) -> String {
    crate::state::loft_text_literal(raw)
}

/// Render the return value (read off `state`'s stack top) of type `ret_ty` as an
/// own-format loft literal — the per-type half of [`ReplSession::capture_binding`].
/// `name` is the type's loft-source name (for the `show_loft` / enum schema
/// lookup).  Covers **every** value type:
///
/// - **inline** values read at their width and rendered directly: `integer`
///   (64-bit), `single`, `float`, `boolean`, `character`, `text`, and a simple
///   enum (an inline 1-based discriminant byte → `Enum.Variant`);
/// - **`DbRef`-backed heap** values (struct, vector, struct-enum variant): the
///   return is a 12-byte `DbRef` on the stack top → [`show_loft`] renders the
///   own-format literal.
///
/// `None` only on an unresolved type name (a fallback to source, never reached in
/// practice for a value the session just produced).
///
/// `captured` receives the raw value the stack read consumed — the one chance to
/// keep it, since `get_stack` pops.  @PLN14 gives it a home in the session store:
/// arc B materializes a [`Captured::Heap`] ref, arc C boxes a
/// [`Captured::Scalar`].  Raw, never the rendered literal: the store-resident
/// value must be exact (a float round-tripped through its decimal form is not).
fn render_capture(
    state: &mut State,
    ret_ty: &Type,
    name: &str,
    json: bool,
    captured: &mut Option<Captured>,
) -> Option<String> {
    match ret_ty {
        // @FR-L-Null — absence is a SENTINEL inside the value's own bytes, so each arm
        // below reads its type exactly as it always did and only the ANSWER differs.
        // `formatting.md` (F-Render) is the contract: **a null of any type renders as
        // the literal word `null`** — so a renderer that prints the sentinel's numeric
        // form (`-9223372036854775808` for a null `integer`, `true` for the 0xFF
        // boolean) is claiming a value the language says is absent.  The panel's twin,
        // `State::render_frame_local`, has answered this way since loft#1459 site 1.
        Type::Integer(_) => {
            let n = *state.get_stack::<i64>();
            *captured = Some(Captured::Scalar(ScalarValue::Integer(n)));
            Some(if n == i64::MIN {
                "null".to_string()
            } else {
                n.to_string()
            })
        }
        Type::Float => {
            let v = *state.get_stack::<f64>();
            *captured = Some(Captured::Scalar(ScalarValue::Float(v)));
            Some(if v.is_nan() {
                "null".to_string()
            } else {
                float_literal(v)
            })
        }
        // own-format `2f` isn't valid JSON; drop the suffix for `json`.
        Type::Single if json => {
            let v = *state.get_stack::<f32>();
            *captured = Some(Captured::Scalar(ScalarValue::Single(v)));
            Some(if v.is_nan() {
                "null".to_string()
            } else {
                v.to_string()
            })
        }
        Type::Single => {
            let v = *state.get_stack::<f32>();
            *captured = Some(Captured::Scalar(ScalarValue::Single(v)));
            Some(if v.is_nan() {
                "null".to_string()
            } else {
                format!("{v}f")
            })
        }
        // @PLN17 C73's three-state boolean: 0 and 1 are the values and every other
        // byte is the absence, which is the same split `Stores::is_null` makes (`> 1`)
        // rather than the narrower `== 255` — one home for the two to agree on.
        Type::Boolean => {
            let raw = *state.get_stack::<u8>();
            *captured = Some(Captured::Scalar(ScalarValue::Boolean(raw == 1)));
            Some(
                match raw {
                    0 => "false",
                    1 => "true",
                    _ => "null",
                }
                .to_string(),
            )
        }
        // own-format `'c'` isn't valid JSON; emit a JSON string for `json`.
        // Codepoint 0 is `character`'s reserved absence (`formal/types.md`, loft#1014),
        // not a renderable character — `char::from_u32` would happily answer `'\0'`.
        Type::Character => {
            let raw = *state.get_stack::<u32>();
            *captured = Some(Captured::Scalar(ScalarValue::Character(raw)));
            if raw == 0 {
                return Some("null".to_string());
            }
            let c = char::from_u32(raw)?;
            Some(if json {
                format!("\"{c}\"")
            } else {
                format!("'{c}'")
            })
        }
        // Text nullity is CONTENT-based (`Store::text_is_null`): the `STRING_NULL` NUL
        // byte IS the absence, and quoting it renders a literal that looks like a
        // one-character string — worse than declining, because it claims a value.
        Type::Text(_) => {
            let s = state.get_stack::<crate::keys::Str>().str().to_string();
            if s == crate::state::STRING_NULL {
                *captured = Some(Captured::Scalar(ScalarValue::Text(s)));
                return Some("null".to_string());
            }
            let lit = escape_loft_text(&s);
            *captured = Some(Captured::Scalar(ScalarValue::Text(s)));
            Some(lit)
        }
        // Heap value backed by a `DbRef`: struct, vector, struct-enum variant.  The
        // return is a 12-byte `DbRef` on the stack top; `json` selects the inbuilt
        // serializer — `show_json` (RFC 8259, `{"x":3}`, the generic value→JSON walk
        // that backs `T.to_json()`) for the wire protocol, `show_loft` (own-format
        // `P{a:7,b:9}`) for the REPL.  (Not `json_to_text`: that one serializes loft's
        // `JValue` JSON-AST type, so an arbitrary struct misreads its first field as a
        // discriminant.)  Both need the schema index `tp` from `Stores::name`.
        Type::Reference(_, _) | Type::Vector(_, _) | Type::Enum(_, true, _) => {
            let tp = state.database.name(name);
            if tp == u16::MAX {
                return None;
            }
            let db = *state.get_stack::<crate::keys::DbRef>();
            *captured = Some(Captured::Heap(db));
            let mut out = String::new();
            if json {
                state.database.show_json(&mut out, &db, tp, false);
            } else {
                state.database.show_loft(&mut out, &db, tp);
            }
            Some(out)
        }
        // Simple enum: an inline 1-based discriminant byte → `Enum.Variant`.
        Type::Enum(_, false, _) => {
            let tp = state.database.name(name);
            if tp == u16::MAX {
                return None;
            }
            let disc = *state.get_stack::<u8>();
            *captured = Some(Captured::Scalar(ScalarValue::SimpleEnum(disc)));
            if crate::database::Stores::enum_is_null(disc) {
                Some("null".to_string())
            } else if json {
                Some(format!("\"{name}.{}\"", state.database.enum_val(tp, disc)))
            } else {
                Some(format!("{name}.{}", state.database.enum_val(tp, disc)))
            }
        }
        // A `τ?` shares `τ`'s runtime layout — no wrapper, no moved offset (`Type::Optional`
        // in data.rs) — so the value on the stack is READ by the base arm above and only
        // needs its sentinel recognised, which every arm now does.  Without this arm the
        // catch-all below claimed the whole nullable family and answered `None`, which is
        // indistinguishable from "this expression does not evaluate": `eval` printed
        // "couldn't evaluate `ni`" for a local the variables panel one site over printed as
        // `null` (loft#1459).  `Optional(Vector)` never got here — its `?` peels for
        // delivery — which is why the hole read as an `integer?` edge rather than as the
        // whole family.
        //
        // `name` is the loft SOURCE type name, and it is the base type's schema that the
        // heap arms look up: `Stores::name("P?")` is `u16::MAX`, so the `?` has to come off
        // the name as well as off the type.
        Type::Optional(inner) => render_capture(
            state,
            inner,
            name.strip_suffix('?').unwrap_or(name),
            json,
            captured,
        ),
        _ => None,
    }
}

/// A mark to rewind a **speculative parse** to — everything such a parse adds
/// to the session, so it is undone as ONE unit (#618).
///
/// A REPL parse writes to two places: it appends `Data` definitions, and it
/// registers the schema types those definitions need in `Stores`.  Rewinding
/// only the definitions leaves the schema holding types whose defs are gone;
/// the next parse then re-creates the same def, sees its name already taken,
/// and registers a *source-qualified* duplicate — until the qualified name
/// repeats too and `Stores::structure` aborts with "Double structure type".
/// (Reachable via any type whose wrapper is not pre-registered by the stdlib,
/// e.g. `vector<integer(-2147483647, 4294967295)>` from `v = [9000000000, 0]`.)
///
/// Taking and restoring both marks together is the whole point of the type:
/// there are a dozen speculative-parse sites, and a bare `rollback_to(defs)`
/// at any one of them re-opens the bug.  Take with
/// [`ReplSession::savepoint`], restore with [`ReplSession::rewind`].
#[derive(Clone, Copy)]
struct Savepoint {
    defs: u32,
    types: u16,
    /// Debug-only oracle: the schema summary at the mark.  [`ReplSession::rewind`]
    /// asserts the restored schema matches it exactly, so a rewind that fails to
    /// undo a registration is caught at its own call site rather than as a
    /// "Double structure type" abort in some later, unrelated parse.
    #[cfg(debug_assertions)]
    schema: (u16, u32, u64),
}

/// @PLN14 — the raw value a capture read off the stack, on its way to a home in
/// the session store.  Split the way the store itself splits: a heap value is
/// already a record (arc B copies it), a scalar is not (arc C boxes it).
enum Captured {
    /// A struct / vector / struct-enum: the record's root ref.
    Heap(crate::keys::DbRef),
    /// An inline value with no heap home of its own yet.
    Scalar(ScalarValue),
}

/// @PLN14 arc C — a scalar in the exact form it left the stack in.  Kept raw
/// rather than as its literal so boxing is lossless (`float_literal` is a
/// display form; round-tripping through it is not the identity).
enum ScalarValue {
    Integer(i64),
    Float(f64),
    Single(f32),
    Boolean(bool),
    /// The raw `u32` scalar value, already known to be a valid `char`.
    Character(u32),
    Text(String),
    /// A simple (payload-free) enum's 1-based discriminant; `0` is null.
    SimpleEnum(u8),
}

/// @PLN14 arc C — how a boxed scalar is read back out of the session store.
/// Mirrors [`ScalarValue`] without the payload: the bytes live in the store, this
/// only says how to interpret them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScalarKind {
    Integer,
    Float,
    Single,
    Boolean,
    Character,
    Text,
    SimpleEnum,
}

impl ScalarValue {
    fn kind(&self) -> ScalarKind {
        match self {
            ScalarValue::Integer(_) => ScalarKind::Integer,
            ScalarValue::Float(_) => ScalarKind::Float,
            ScalarValue::Single(_) => ScalarKind::Single,
            ScalarValue::Boolean(_) => ScalarKind::Boolean,
            ScalarValue::Character(_) => ScalarKind::Character,
            ScalarValue::Text(_) => ScalarKind::Text,
            ScalarValue::SimpleEnum(_) => ScalarKind::SimpleEnum,
        }
    }
}

/// Outcome of trying to value-snapshot a binding's RHS (REPL.X capture).
enum Capture {
    /// The value was captured — store `name = <this literal>`.
    Done(String),
    /// Not capturable (un-inferable type, cap-fn parse failure, unresolved type
    /// name) and the RHS did **not** fault — fall back to storing the RHS as
    /// source.
    Skip,
    /// The RHS **faulted** while being run to snapshot it — surface the error at
    /// the binding and store nothing, so the effect ran once and the session is
    /// not poisoned by a re-running source binding.
    Failed(Vec<DiagEntry>),
}

/// Outcome of evaluating one REPL input.
#[derive(Debug)]
pub enum Eval {
    /// The statement parsed and ran; the session advanced.
    Ran,
    /// Input ends mid-construct (open bracket, unterminated string, trailing
    /// operator).  The caller should read another line and re-call with the
    /// concatenated input.  The session is unchanged.
    NeedMore,
    /// Parse error; the session is left exactly as it was before the call.
    Error(Vec<DiagEntry>),
    /// @PLN16 G1 — the run hit a breakpoint and **suspended** (interactive
    /// stepping is on).  The session now holds the live frame: inspect it with
    /// [`ReplSession::paused_frame`], edit a value with
    /// [`ReplSession::debug_set`], and resume with [`ReplSession::debug_step`] /
    /// [`ReplSession::debug_continue`].  The observing statement finishes (and
    /// prints its value) when the run is resumed to completion.
    Paused,
}

/// @PLN16 M5e — outcome of a **context-aware** REPL evaluation ([`ReplSession::repl_eval`]),
/// for the browser REPL panel.
#[derive(Debug)]
pub struct ReplOutcome {
    /// Which env the input ran against: `"frame"` (paused at a breakpoint — the debugger)
    /// or `"top"` (the session top level — the normal REPL).
    pub context: &'static str,
    /// The input is incomplete (`NeedMore`) — the multi-line continuation signal; the
    /// caller keeps the buffer and re-submits when there's more.
    pub more: bool,
    /// A rendered result, when the eval hands one back directly (frame eval).  A top-level
    /// expression instead *prints* its value to the output sink, which the transport drains
    /// into the REPL pane — so `value` is `None` there.
    pub value: Option<String>,
    /// Diagnostics (errors), empty on success.  Positions are the engine's **as reported**:
    /// the REPL is the consumer that drives these toward correctness, so they are surfaced
    /// raw here rather than silently massaged.
    pub diagnostics: Vec<DiagEntry>,
}

/// @PLN16 M5e slice 5 — one test function's outcome, for the browser's test panel.
pub struct TestRun {
    /// The user function name (no `n_` prefix).
    pub name: String,
    /// Ran without a fault.
    pub passed: bool,
    /// The failure message (a typed runtime fault or a panic), `None` when it passed.
    pub message: Option<String>,
    /// The function's definition line, so a failure row can jump to it.
    pub line: u32,
}

/// @PLN16 M5e slice 6 — pump a child pipe into `buf` on a detached thread, line by line.
/// Keeps the game's stdout/stderr flowing (an undrained pipe fills and stalls the child at
/// its next print); the thread ends when the pipe closes (game exit / kill).
fn drain_pipe<R: std::io::Read + Send + 'static>(
    pipe: Option<R>,
    buf: &std::sync::Arc<std::sync::Mutex<String>>,
) {
    let Some(p) = pipe else { return };
    let buf = std::sync::Arc::clone(buf);
    std::thread::spawn(move || {
        let reader = std::io::BufReader::new(p);
        for line in reader.lines() {
            let Ok(line) = line else { break };
            let mut g = buf.lock().unwrap();
            g.push_str(&line);
            g.push('\n');
        }
    });
}

/// Extract a readable message from a caught panic payload.
fn panic_text(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|s| (*s).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "panicked".to_string())
}

/// How a [`ReplSession::resume_from`] replay went.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResumeStats {
    /// Saved entries that replayed cleanly.
    pub restored: usize,
    /// Entries skipped because they no longer parse/run — a stale or corrupt
    /// line never bricks the resume.
    pub skipped: usize,
}

/// @PLN16 rich-bp — where a breakpoint sits: a function-scoped spec (`"foo"` body
/// start, `"foo:3"` line 3 of foo) or a `file:line` (the file-run debugger).
#[derive(Clone, PartialEq)]
enum BpLocation {
    Function(String),
    File(String, u32),
}

/// @PLN16 rich-bp — a breakpoint with optional **condition** (break only when an
/// expression over the frame holds — reuses E) and **tracepoint actions** (expressions
/// emitted on each hit).  `stop` = pause (a breakpoint) vs continue (a tracepoint:
/// emit the actions and run on).  This is the unit the wire protocol's `setBreakpoints`
/// carries; the prompt sets it via `:break <loc> [if <cond>]` / `:trace <loc> <exprs>`.
#[derive(Clone, PartialEq)]
struct BreakSpec {
    location: BpLocation,
    condition: Option<String>,
    actions: Vec<String>,
    stop: bool,
}

impl BreakSpec {
    /// Render the spec the way the user typed it, for the `:break` list.
    fn describe(&self) -> String {
        let mut s = match &self.location {
            BpLocation::Function(l) => l.clone(),
            BpLocation::File(f, n) => format!("{f}:{n}"),
        };
        if let Some(c) = &self.condition {
            s.push_str(" if ");
            s.push_str(c);
        }
        if !self.stop {
            s.push_str(" { ");
            s.push_str(&self.actions.join(", "));
            s.push_str(" }");
        }
        s
    }
}

/// @PLN16 rich-bp — split a `:break` spec into its location and optional `if <cond>`
/// (e.g. `foo:3 if c.n < 0` → `("foo:3", Some("c.n < 0"))`).
fn parse_break_spec(spec: &str) -> (String, Option<String>) {
    if let Some(idx) = spec.find(" if ") {
        let cond = spec[idx + 4..].trim();
        if !cond.is_empty() {
            return (spec[..idx].trim().to_string(), Some(cond.to_string()));
        }
    }
    (spec.trim().to_string(), None)
}

/// @PLN16 rich-bp — the per-offset metadata `resolve_pause` consults after a hit:
/// rebuilt by `apply_breakpoints` keyed on the resolved bytecode offset.
#[derive(Clone, Default)]
struct BpMeta {
    condition: Option<String>,
    actions: Vec<String>,
    stop: bool,
}

/// A live REPL session: stdlib + the statements entered so far.
// The flags below (`replaying`, `stepping`, `reverse_armed`, `store_observe`) are
// INDEPENDENT session modes, not the states of one machine: a session can be
// replaying a saved file while stepping, with reverse armed, reading values from
// the store.  Folding them into an enum would have to enumerate the product, so
// the lint's suggested refactor is the worse shape here.
/// Everything that decides which names a source can see — one value, so a session
/// cannot be opened with a subset of it.
///
/// The parallel-parameter shape this replaces produced the same defect twice: a
/// `--lib` wired into three entry points and forgotten in the fourth (@PLN120 E.1),
/// and a `:reset` that rebuilt from `stdlib_dir` alone and dropped the libraries.
/// Adding a field here is a compile error at every construction site, which is the
/// property being bought.
#[derive(Clone, Debug, Default)]
pub struct ResolutionContext {
    /// The `default/` standard library directory.
    pub stdlib_dir: String,
    /// `--lib` import paths, searched for a `use`d library.
    pub lib_dirs: Vec<String>,
}

impl ResolutionContext {
    /// The context for a stdlib-only session (no `--lib` paths).
    #[must_use]
    pub fn stdlib_only(stdlib_dir: &str) -> Self {
        Self {
            stdlib_dir: stdlib_dir.to_string(),
            lib_dirs: Vec::new(),
        }
    }

    /// Render for `--show-resolution`'s `context:` line — an empty `lib_dirs` under a
    /// `--lib` invocation is @PLN120 E.1, visible without running the program.
    #[must_use]
    pub fn describe(&self) -> String {
        format!("stdlib={:?}  lib_dirs={:?}", self.stdlib_dir, self.lib_dirs)
    }
}

#[allow(clippy::struct_excessive_bools)]
pub struct ReplSession {
    pub(crate) parser: Parser,
    /// @PLN120 follow-up — the resolution inputs this session was opened with, kept so
    /// a re-open (`:reset`) restores them instead of silently degrading to stdlib-only.
    context: ResolutionContext,
    /// The standard-library directory this session was built from, kept so the test runner
    /// can spin up a **fresh** parser (stdlib + the file under test) — the clean single-parse
    /// the CLI runner uses, which the persistent session's accumulated parser state can't
    /// reproduce for a bare-function call.
    stdlib_dir: String,
    /// Accumulated statements, each terminated with `;`, replayed in one shared
    /// function scope so earlier bindings stay visible.
    body: String,
    /// Monotonic counter naming each generation's synthetic entry fn.
    counter: u32,
    /// Append handle to the session file when persistence is on (the
    /// interactive driver enables it; piped/test sessions leave it `None`, so
    /// they neither read nor write any session file).
    record: Option<File>,
    /// True only while [`resume_from`](Self::resume_from) is feeding saved
    /// inputs back in — suppresses re-recording them to the file.
    replaying: bool,
    /// @PLN16 — every breakpoint + tracepoint (function-scoped and `file:line`), each
    /// with its optional condition / tracepoint actions.  Re-applied to the fresh
    /// `State` of every observing run by [`apply_breakpoints`](Self::apply_breakpoints).
    breakpoints: Vec<BreakSpec>,
    /// @PLN16 rich-bp — resolved bytecode-offset → metadata for the current run, built
    /// by `apply_breakpoints` and read by `resolve_pause` when a hit lands.
    bp_meta: std::collections::HashMap<u32, BpMeta>,
    /// @PLN16 rich-bp — tracepoint emissions from the most recent resume, drained by the
    /// driver (printed at the prompt, an `output` event over the wire protocol).
    trace_output: Vec<String>,
    /// @PLN120 B — breakpoint offsets whose condition could not be evaluated and has
    /// already been reported.  The complaint is worth making once; on a hot line,
    /// once per hit would bury the frame the user was sent to look at.
    cond_unevaluable: std::collections::HashSet<u32>,
    /// Frames captured at breakpoints during the most recent observing run
    /// (record-and-continue mode — when `stepping` is off).
    last_hits: Vec<crate::debugger::BreakHit>,
    /// @PLN16 G1 — **interactive stepping**: when on, an observing run that
    /// reaches a breakpoint *suspends* into the paused sub-mode (held in `paused`)
    /// instead of recording all hits and continuing.  The interactive driver turns
    /// it on; programmatic/piped callers that want the full hit list leave it off.
    stepping: bool,
    /// @PLN16 G1 — a run suspended at a breakpoint, held across REPL inputs so the
    /// user can inspect the frame, edit a value, and step.  `None` unless paused.
    /// Boxed because `State` is large and the paused case is rare.
    paused: Option<Box<State>>,
    /// @PLN63 RX — whether reverse stepping is armed for this session.  Applied to the
    /// paused `State` before each step (so its ring fills), so a later `step_back` can
    /// reverse it.  Off by default (a normal debug session pays no snapshot cost).
    reverse_armed: bool,
    /// @PLN16 M5e slice 3 — the editor's **write sandbox**: the canonical path of the one
    /// file the browser may save back to (the `--serve` target).  `None` (e.g. `--rpc`)
    /// rejects every `writeFile`.  Single-file for now; a workspace-root form lands with
    /// multi-file editing.
    workspace_file: Option<std::path::PathBuf>,
    /// @PLN16 M5e slice 6 — the running **game child process** (`launchGame`), if any.
    /// A game is a real `loft` run in its own process — its frame loop (and, once the
    /// graphics library lands, its native window) must not block the serve loop.
    game: Option<GameProc>,
    /// @PLN14 arc A — **the session store**: one detached `Store` holding a
    /// materialized copy of every heap-backed binding's value.  It is adopted into
    /// each eval's throwaway `State` just long enough to materialize into, then
    /// taken back out, so it outlives the run.  One `Store` suffices because a
    /// materialized value is self-contained in it (see @PLN14 Step 0/1 findings).
    session_store: Option<crate::store::Store>,
    /// @PLN14 arc A — **the binding environment**: name → where that binding's value
    /// lives in [`session_store`](Self::session_store).
    ///
    /// **Write-only for now (Step 2).**  The replay model stays the source of truth;
    /// this shadow is written on every bind and read only by
    /// [`env_value`](Self::env_value), which is the differential oracle Step 4's
    /// frame-seed will be checked against.
    env: std::collections::HashMap<String, SessionValue>,
    /// @PLN14 arc E — **the flip**: when on, observing a store-resident binding
    /// reads the session store instead of replaying the accumulated body.  ON by
    /// default since Step 8; `LOFT_NO_STORE_OBSERVE` opts out, and
    /// [`set_store_observe`](Self::set_store_observe) overrides it for a test.
    store_observe: bool,
    /// @PLN14 arc B — the value the most recent capture materialized, waiting to be
    /// filed under the bound name by [`eval`](Self::eval).  `capture_typed` cannot
    /// name it (only the caller knows the binding name), so it parks it here.
    pending_materialized: Option<SessionValue>,
}

/// @PLN14 arc A — one binding's home in the session store.
///
/// Deliberately holds **no `store_nr`**: the session store is adopted at whatever
/// slot is free in each eval's `State`, so the slot is not a stable identity.  A
/// materialized value's interior references are slot-independent (in-store record
/// ids), which is what makes this safe — pinned by
/// `a_session_store_survives_re_adoption_at_a_different_slot`.
#[derive(Debug, Clone)]
struct SessionValue {
    /// The loft-source type name, for the `show_loft` / enum schema lookup on
    /// read-back.
    type_name: String,
    rec: u32,
    pos: u32,
    /// How to read the bytes back: a record the schema walks, or a boxed scalar.
    shape: SessionShape,
}

/// @PLN14 — how a session-store entry is interpreted on read-back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionShape {
    /// A record the schema can walk — rendered by `show_loft` with `type_name`'s
    /// type id (arc B).
    Heap,
    /// A scalar boxed into a 1-field record (arc C).
    Scalar(ScalarKind),
    /// A `text` binding stored as the single-element `vector<text>` that
    /// `capture_binding` builds to dodge @P293.  The characters ARE store-resident
    /// (the vector copied them in); only the wrapper has to be undone on read.
    TextInVector,
}

/// @PLN16 M5e slice 6 — a launched game: the child process + its drained output.
/// stdout/stderr are pumped by detached threads into one shared buffer (a pipe left
/// undrained would fill and stall the game at its next print).
struct GameProc {
    child: std::process::Child,
    output: std::sync::Arc<std::sync::Mutex<String>>,
}

/// @PLN14 arc D — one binding's frame-seed result, as the differential Step 4 is
/// gated on.
///
/// `replayed` is what the slot held *before* seeding — the value the body replay
/// put there, i.e. the model still known to be correct.  `seeded` is what it holds
/// *after* the store-resident value was written in.  Step 4's whole safety
/// argument is that these two are equal for every binding: the new path is checked
/// against the old one rather than trusted.
#[derive(Debug, Clone)]
pub struct SeedReport {
    pub name: String,
    pub replayed: String,
    pub seeded: String,
}

impl ReplSession {
    /// Open a session from its full [`ResolutionContext`] — the constructor to use.
    ///
    /// Every input that decides which names a source can see travels as one value, so
    /// a session cannot be built with a subset of them.  That shape is the fix for a
    /// defect this repo hit twice: `--lib` was threaded into three entry points and
    /// forgotten in the fourth (@PLN120 E.1, reported by a consumer as *"the debugger
    /// does not work on real programs"*), and `:reset` rebuilt a live session from
    /// `stdlib_dir` alone, silently dropping the libraries it had.  With one value a
    /// new resolution input is a compile error at every site instead of a silent
    /// degradation at one.
    ///
    /// # Errors
    /// Returns the I/O error if the stdlib directory cannot be read.
    pub fn open(ctx: &ResolutionContext) -> std::io::Result<Self> {
        let mut session = Self::new(&ctx.stdlib_dir)?;
        session.parser.lib_dirs.clone_from(&ctx.lib_dirs);
        session.context = ctx.clone();
        Ok(session)
    }

    /// The context this session was opened with — so a re-open (`:reset`) restores the
    /// same resolution inputs rather than re-deriving them from whatever is in scope.
    #[must_use]
    pub fn context(&self) -> &ResolutionContext {
        &self.context
    }

    /// Start a session with the standard library loaded from `stdlib_dir`
    /// (e.g. `"default"`, or an absolute path to a release bundle's `default/`).
    /// Prefer [`open`](Self::open), which carries the `--lib` paths too.
    ///
    /// # Errors
    /// Returns the I/O error if the stdlib directory cannot be read.
    pub fn new(stdlib_dir: &str) -> std::io::Result<Self> {
        let mut parser = Parser::new();
        parser.parse_dir(stdlib_dir, true, false)?;
        Ok(Self {
            parser,
            stdlib_dir: stdlib_dir.to_string(),
            context: ResolutionContext {
                stdlib_dir: stdlib_dir.to_string(),
                lib_dirs: Vec::new(),
            },
            body: String::new(),
            counter: 0,
            record: None,
            replaying: false,
            breakpoints: Vec::new(),
            bp_meta: std::collections::HashMap::new(),
            cond_unevaluable: std::collections::HashSet::new(),
            trace_output: Vec::new(),
            last_hits: Vec::new(),
            stepping: false,
            paused: None,
            workspace_file: None,
            game: None,
            session_store: None,
            store_observe: store_observe_default(),
            env: std::collections::HashMap::new(),
            pending_materialized: None,
            reverse_armed: false,
        })
    }

    /// Like [`new`](Self::new) but with explicit `--lib` import search paths, so a program
    /// loaded into this session can `use` **libraries**, not just the stdlib.  The browser
    /// IDE (`--serve`) and the `--rpc` server pass the dirs collected from the command line,
    /// giving the IDE the same `use`-resolution the normal run path has — without it the
    /// session is stdlib-only and a library (or a project that depends on one) cannot load.
    ///
    /// # Errors
    /// Returns the I/O error if the standard library cannot be read.
    pub fn new_with_libs(stdlib_dir: &str, lib_dirs: &[String]) -> std::io::Result<Self> {
        Self::open(&ResolutionContext {
            stdlib_dir: stdlib_dir.to_string(),
            lib_dirs: lib_dirs.to_vec(),
        })
    }

    /// Load and wire the native cdylibs of every `#native` package the session has
    /// parsed.  **Call this after `byte_code` on any State that will RUN user
    /// code** — `byte_code` registers native STUBS, and a stub that is never wired
    /// panics with *"native function not loaded"* the moment it is called.
    ///
    /// @PLN120 E3b: the CLI run path and `loft test` did this; the REPL's own
    /// execute path did not, so the debugger died on the first call that crossed
    /// into native code — `use web; sleep_ms(5)` — while the same file ran fine
    /// under `--interpret`. Importing the package was harmless; calling the part
    /// of it that is native was not, which is why it looked like a package problem.
    fn wire_natives(&self, state: &mut State) {
        let pending = self.parser.pending_native_libs.clone();
        crate::extensions::load_all(state, pending);
        crate::extensions::wire_native_fns(state, &self.parser.data);
    }

    /// Build a session over an existing `parser` already loaded with a program's
    /// definitions — used by the @PLN16 debugger to evaluate at a paused frame with
    /// the program's types + functions in scope.  The accumulated body starts
    /// empty; persistence is off.
    #[must_use]
    pub fn from_parser(parser: Parser) -> Self {
        Self {
            parser,
            // `from_parser` builds a debugger-eval session over an existing program, not a
            // test runner; the stdlib dir is unknown here, so the conventional `"default"`
            // is the fallback (run_file_tests is driven from `new`/`new_with_libs` sessions).
            stdlib_dir: "default".to_string(),
            // Same fallback as `stdlib_dir` above: this session wraps an ALREADY-parsed
            // program, so its `lib_dirs` live on the parser it was handed rather than
            // being re-derived here.
            context: ResolutionContext::stdlib_only("default"),
            body: String::new(),
            counter: 0,
            record: None,
            replaying: false,
            breakpoints: Vec::new(),
            bp_meta: std::collections::HashMap::new(),
            cond_unevaluable: std::collections::HashSet::new(),
            trace_output: Vec::new(),
            last_hits: Vec::new(),
            stepping: false,
            paused: None,
            workspace_file: None,
            game: None,
            session_store: None,
            store_observe: store_observe_default(),
            env: std::collections::HashMap::new(),
            pending_materialized: None,
            reverse_armed: false,
        }
    }

    /// Seed this session with a paused frame's variables (a @PLN16
    /// [`BreakHit`](crate::debugger::BreakHit)): each captured
    /// `(name, own-format literal)` becomes a binding `name = <literal>`, so
    /// expressions evaluated afterwards run against the frame's live values — the
    /// **REPL-at-frame** bridge.  It reuses the literal each value already renders
    /// to, so it needs no store-resident environment (exact value-for-value seeding
    /// with frame mutation is the @PLN14 upgrade).  Returns the number of variables
    /// bound; a value whose type isn't in scope (seeding a struct on a stdlib-only
    /// session) is skipped, not fatal.
    pub fn seed_frame(&mut self, hit: &crate::debugger::BreakHit) -> usize {
        let mut bound = 0;
        for (name, literal) in hit.held_locals() {
            if matches!(self.eval(&format!("{name} = {literal}")), Eval::Ran) {
                bound += 1;
            }
        }
        bound
    }

    /// Evaluate a boolean `condition` against a captured frame — the @PLN16 E
    /// **conditional / test breakpoint** primitive.  Seeds the frame (D1) then
    /// `assert`s the condition: returns `true` iff it holds, so a caller keeps only
    /// the hits where it does ("break when `i > 3`") or where an invariant is
    /// violated ("break when `!(balance >= 0)`").  The session is mutated
    /// (re-seeded each call — a rebind, safe for one breakpoint's same-shaped
    /// frames), so build it with [`from_parser`](Self::from_parser) so heap values'
    /// types are in scope.  This filters *recorded* hits post-run; skipping the
    /// capture in-loop (so a non-matching breakpoint never even pauses) would need
    /// the condition pre-compiled against the frame — a later refinement.
    pub fn frame_holds(&mut self, hit: &crate::debugger::BreakHit, condition: &str) -> bool {
        self.seed_frame(hit);
        matches!(
            self.eval(&format!("assert({condition}, \"cond\")")),
            Eval::Ran
        )
    }

    /// The current value of `expr` in this session, rendered as an own-format loft
    /// literal (`"99"`, `"Point{x:3,y:4}"`) — or `None` if it doesn't evaluate.
    /// Reuses the value-snapshot capture, so it covers every type the REPL renders.
    /// The @PLN16 debugger uses it to read a value the user edited at a breakpoint
    /// (`n = 99`) before writing it back into the live frame.
    pub fn value_of(&mut self, expr: &str) -> Option<String> {
        self.value_of_fmt(expr, false)
    }

    /// Like [`value_of`](Self::value_of) but `json` selects how a value is rendered:
    /// the wire protocol shows data as JSON, the REPL shows own-format (`P{a:7}`).
    ///
    /// For `json` a struct / struct-enum is serialised through loft's inbuilt
    /// `.to_json()` (a *text* result — `{"x":9}`), tried first via [`capture_json`].
    /// That is deliberate, not just for the JSON shape: returning a bare heap value
    /// from the synthetic capture fn faults on a cloned paused state (the fn-return
    /// deep-copy targets a store the clone never allocated), whereas a text result
    /// copies safely.  `.to_json()` parse-fails for scalars / text / bare vectors, so
    /// those fall through to [`capture_binding`] — scalars render as raw JSON there,
    /// a bare vector has no `.to_json()` and yields `None` (eval its elements instead).
    fn value_of_fmt(&mut self, expr: &str, json: bool) -> Option<String> {
        // @PLN14 arc E — the own-format read (`value_of`) is answered from the
        // session store too.  `json` keeps the existing path: the store read
        // renders own-format, and the wire protocol wants JSON.
        if !json
            && self.store_observe
            && let Some(name) = self.store_resident_name(expr)
            && let Some(v) = self.env_value(&name)
        {
            return Some(v);
        }
        if json && let Some(j) = self.capture_json(expr) {
            return Some(j);
        }
        match self.capture_binding(expr, json) {
            Capture::Done(lit) => Some(lit),
            Capture::Skip | Capture::Failed(_) => None,
        }
    }

    /// Serialise `expr` to JSON via loft's inbuilt `.to_json()` method, captured as
    /// **raw** text (no surrounding quotes — the result already *is* JSON).  Returns
    /// `None` when the type has no `.to_json()` (scalars, text, bare vectors) or the
    /// run faults — the caller then falls back to the own-renderer path.  See
    /// [`value_of_fmt`] for why text-not-heap-value matters on a paused clone.
    fn capture_json(&mut self, expr: &str) -> Option<String> {
        let next = self.counter + 1;
        let name = format!("replmain_{next}");
        let src = format!(
            "fn {name}() -> text {{\n{}({expr}).to_json()\n}}\n",
            self.body
        );
        let sp = self.savepoint();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        let failed = self.parser.diagnostics.entries()[pre_diag..]
            .iter()
            .any(|e| e.level >= Level::Error);
        if failed {
            self.rewind(sp); // type has no `.to_json()`
            return None;
        }
        self.counter = next;
        crate::scopes::check(&mut self.parser.data, &mut self.parser.database);
        let mut state = State::new(self.parser.database.clone());
        compile::byte_code(&mut state, &mut self.parser.data);
        state.execute_argv(&name, &self.parser.data, &[]);
        let out = if state.database.runtime_error.take().is_some() {
            None
        } else {
            Some(state.get_stack::<crate::keys::Str>().str().to_string())
        };
        self.rewind(sp); // discard the throwaway cap gen
        out
    }

    /// Add a breakpoint (the `:break` command).  **Function-scoped** forms only —
    /// `foo` (the body start of function `foo`) or `foo:3` (line 3 of `foo`) —
    /// because that is the only form unique in the REPL (every input parses under
    /// the synthetic file `"<repl>"` with line numbers restarting at 1, so a bare
    /// line is not unique; `file:line` is for a file-run debugger).  Re-applied to
    /// the fresh `State` of every later observing run.
    pub fn add_breakpoint(&mut self, spec: &str) {
        let (loc, condition) = parse_break_spec(spec.trim());
        if loc.is_empty() {
            return;
        }
        self.push_breakpoint(BreakSpec {
            location: BpLocation::Function(loc),
            condition,
            actions: Vec::new(),
            stop: true,
        });
    }

    /// @PLN16 rich-bp — set a **tracepoint** (`:trace <loc> <expr>, <expr>`): on each
    /// hit, evaluate the comma-separated expressions, emit them, and **continue** (no
    /// pause) — a non-interactive log of values at a point.  `<loc>` is function-scoped
    /// like `:break`.
    pub fn add_tracepoint(&mut self, spec: &str) {
        let Some((loc, rest)) = spec.trim().split_once(char::is_whitespace) else {
            return; // need both a location and at least one expression
        };
        let actions: Vec<String> = rest
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if loc.is_empty() || actions.is_empty() {
            return;
        }
        self.push_breakpoint(BreakSpec {
            location: BpLocation::Function(loc.to_string()),
            condition: None,
            actions,
            stop: false,
        });
    }

    /// Append a breakpoint spec, skipping an exact duplicate.
    fn push_breakpoint(&mut self, bp: BreakSpec) {
        if !self.breakpoints.contains(&bp) {
            self.breakpoints.push(bp);
        }
    }

    /// @PLN16 M5a — load a program's definitions from `src` (parsed under `filename`,
    /// so the file-run debugger can address it by `file:line`) into this session, on
    /// top of the stdlib already loaded.  On a parse error the session is rolled back
    /// and the diagnostics returned.
    ///
    /// # Errors
    /// Returns the error-level diagnostics if `src` does not parse.
    pub fn load_program_str(&mut self, src: &str, filename: &str) -> Result<(), Vec<DiagEntry>> {
        let sp = self.savepoint();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(src, filename, false);
        let produced: Vec<DiagEntry> = self.parser.diagnostics.entries()[pre_diag..].to_vec();
        if produced.iter().any(|e| e.level >= Level::Error) {
            self.rewind(sp);
            return Err(produced);
        }
        Ok(())
    }

    /// @PLN16 M5a — read a `.loft` file and load its definitions
    /// ([`load_program_str`](Self::load_program_str)).  The outer `Result` is the file
    /// read; the inner is the parse — split so the caller can word each failure.
    ///
    /// # Errors
    /// Returns the I/O error if `path` cannot be read.
    pub fn load_program(&mut self, path: &str) -> std::io::Result<Result<(), Vec<DiagEntry>>> {
        let src = std::fs::read_to_string(path)?;
        // Reset to a pristine stdlib (+ the session's `--lib` dirs) parser before loading, so
        // a **re-launch is idempotent**: `load_program_str` is additive, and re-parsing a
        // `use`-program over an already-loaded one re-loads its libraries → "Cannot redefine".
        // Both callers (the file-run debugger and the rpc/serve `launch`) want a *fresh* whole-
        // program load, not an append — the REPL's incremental path is `eval`, not this.
        let lib_dirs = std::mem::take(&mut self.parser.lib_dirs);
        let mut parser = Parser::new();
        parser.lib_dirs = lib_dirs;
        parser.parse_dir(&self.stdlib_dir, true, false)?;
        self.parser = parser;
        Ok(self.load_program_str(&src, path))
    }

    /// @PLN16 M5e slice 2 — **check** `path` and return ALL its diagnostics (errors AND
    /// warnings), leaving the session unchanged.  Unlike [`load_program`](Self::load_program)
    /// — which keeps the program on success and surfaces only *error*-level diagnostics —
    /// this always rolls back: a pure compiler-console feed, callable repeatedly (each edit,
    /// before/after a `launch`) without duplicating definitions.
    ///
    /// # Errors
    /// Returns the I/O error if `path` cannot be read.
    pub fn compile(&mut self, path: &str) -> std::io::Result<Vec<DiagEntry>> {
        let src = std::fs::read_to_string(path)?;
        let sp = self.savepoint();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, path, false);
        let produced = self.parser.diagnostics.entries()[pre_diag..].to_vec();
        self.rewind(sp);
        Ok(produced)
    }

    /// @PLN16 M5e slice 5 — run every test in `path` and return a structured outcome per
    /// test, for the browser's test panel.  A "test" is a zero-parameter user function
    /// defined in `path` itself — the same discovery `loft --tests` uses (stdlib, library,
    /// generator, and lambda functions are skipped).  Each runs in a **fresh** `State` (a
    /// clone of the parsed program), so one test can't pollute the next, and a panic or a
    /// typed runtime fault in one test is caught and reported, not fatal to the rest.  This
    /// reuses the runner's execution primitives (`State::execute_argv` + `had_fatal`) without
    /// its CLI-only annotation machinery (`@EXPECT_FAIL` etc. are a suite-file concern).
    /// Parsed fresh each call (so a just-saved edit is what runs), in a clean parser, so the
    /// session's accumulated parser state never interferes.
    ///
    /// # Errors
    /// Returns the I/O error if `path` cannot be read.
    pub fn run_file_tests(&mut self, path: &str) -> std::io::Result<Vec<TestRun>> {
        self.run_file_tests_with(path, &[])
    }

    /// [`run_file_tests`](Self::run_file_tests) with extra `use`-import dirs appended after
    /// the session's own — how [`run_suite`](Self::run_suite) injects the package's `src/`
    /// (+ sibling-deps parent) for each test file without mutating the session.
    fn run_file_tests_with(
        &mut self,
        path: &str,
        extra_lib_dirs: &[String],
    ) -> std::io::Result<Vec<TestRun>> {
        // A **fresh** parser — stdlib + the file + the session's `--lib` dirs — is the clean
        // single-parse the CLI runner uses; the persistent session's accumulated parser state
        // does not run a bare test function (every native call faults "Unknown definition").
        // Parse the file **by path** (`parse`, not `parse_str`): that sets up the source dir +
        // `use` context a bare-function call needs.  Read it first for a clean io error.
        let _ = std::fs::read_to_string(path)?;
        let abs = crate::portable_path::plain_canonical_str(path);
        let mut parser = Parser::new();
        parser.lib_dirs.clone_from(&self.parser.lib_dirs);
        for d in extra_lib_dirs {
            if !parser.lib_dirs.contains(d) {
                parser.lib_dirs.push(d.clone());
            }
        }
        parser.parse_dir(&self.stdlib_dir, true, false)?;
        let pre_diag = parser.diagnostics.entries().len();
        let parse_outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            parser.parse(&abs, false);
        }));
        if parse_outcome.is_err() {
            return Ok(vec![TestRun {
                name: "(compile)".to_string(),
                passed: false,
                message: Some("parse panicked".to_string()),
                line: 1,
            }]);
        }
        let errors: Vec<DiagEntry> = parser.diagnostics.entries()[pre_diag..]
            .iter()
            .filter(|d| {
                matches!(
                    d.level,
                    crate::diagnostics::Level::Error | crate::diagnostics::Level::Fatal
                )
            })
            .cloned()
            .collect();
        if !errors.is_empty() {
            // Won't compile → one synthetic failure naming the error(s).
            let line = errors.first().map_or(1, |d| d.line);
            let message = errors
                .iter()
                .map(crate::diagnostics::DiagEntry::to_string_compact)
                .collect::<Vec<_>>()
                .join("; ");
            return Ok(vec![TestRun {
                name: "(compile)".to_string(),
                passed: false,
                message: Some(message),
                line,
            }]);
        }
        crate::scopes::check(&mut parser.data, &mut parser.database);
        // Discover the file's zero-parameter user test functions (defs whose `position.file`
        // is the canonical path we parsed).
        let in_file = |pf: &str| pf == abs;
        let mut targets: Vec<(String, u32)> = Vec::new();
        for d_nr in 0..parser.data.definitions() {
            let def = parser.data.def(d_nr);
            if !matches!(def.def_type, DefType::Function) {
                continue;
            }
            if !def.name.starts_with("n_") || def.name.starts_with("n___lambda_") {
                continue;
            }
            if crate::portable_path::is_stdlib_source(&def.position.file)
                || !in_file(&def.position.file)
            {
                continue;
            }
            // Generators (return iterator<T>) and parameterised functions are not tests.
            if matches!(def.returned, Type::Iterator(_, _)) || !def.attributes.is_empty() {
                continue;
            }
            let name = def.name.strip_prefix("n_").unwrap_or(&def.name).to_string();
            targets.push((name, def.position.line));
        }
        // Run each in a **fresh State** (a clean heap / stores, so one test can't pollute the
        // next), wiring the native functions after bytecode — the CLI runner's exact sequence.
        let pending_native = parser.pending_native_libs.clone();
        let mut out = Vec::with_capacity(targets.len());
        for (name, line) in targets {
            let mut data = parser.data.clone();
            let mut state = State::new(parser.database.clone());
            compile::byte_code(&mut state, &mut data);
            crate::extensions::load_all(&mut state, pending_native.clone());
            crate::extensions::wire_native_fns(&mut state, &data);
            // `execute_argv` prepends `n_` itself — pass the bare user name (`name`), not an
            // already-prefixed one, or it looks up `n_n_<name>` → "Unknown definition".
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                state.execute_argv(&name, &data, &[]);
                let fault = state.database.had_fatal;
                let msg = state
                    .database
                    .runtime_error
                    .as_ref()
                    .map(|e| e.message.clone())
                    .unwrap_or_default();
                (fault, msg)
            }));
            let (passed, message) = match result {
                Ok((false, _)) => (true, None),
                Ok((true, m)) => (
                    false,
                    Some(if m.is_empty() {
                        "runtime error".to_string()
                    } else {
                        m
                    }),
                ),
                Err(payload) => (false, Some(panic_text(&*payload))),
            };
            out.push(TestRun {
                name,
                passed,
                message,
                line,
            });
        }
        Ok(out)
    }

    /// @PLN16 M5e slice 5 — run the **package suite**: the `loft test` semantics, in-session.
    /// Walks up from `start` (the served file) to the nearest `loft.toml`, mirrors the CLI's
    /// package setup — the manifest `entry`'s `src/` dir joins the import path, plus the
    /// package's **parent** dir when it has dependencies (so sibling packages resolve) — and
    /// runs every `tests/*.loft` through the per-file runner.  Returns `(file name, results)`
    /// per test file.  Deliberately **package-aware, not a directory sweep**: a suite is
    /// defined by its manifest, so a missing `loft.toml` is an error naming what's missing,
    /// never a guess.
    ///
    /// # Errors
    /// `NotFound` when no `loft.toml` exists upward from `start`, or the package has no
    /// `tests/` directory; other I/O errors from reading the test files.
    pub fn run_suite(&mut self, start: &str) -> std::io::Result<Vec<(String, Vec<TestRun>)>> {
        use std::io::{Error, ErrorKind};
        // Find the package root: the nearest ancestor of `start` holding a loft.toml.
        let abs = crate::portable_path::plain_canonical(std::path::Path::new(start));
        let mut root = if abs.is_dir() {
            Some(abs.as_path())
        } else {
            abs.parent()
        };
        while let Some(dir) = root {
            if dir.join("loft.toml").exists() {
                break;
            }
            root = dir.parent();
        }
        let Some(root) = root else {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("no loft.toml found upward from {start} — runSuite needs a package"),
            ));
        };
        // The CLI's package setup (`loft test`): manifest entry → src/ on the import path;
        // the parent dir too when dependencies exist (sibling packages).
        let manifest = crate::manifest::read_manifest(&root.join("loft.toml").to_string_lossy())
            .unwrap_or_default();
        let entry = manifest.entry.unwrap_or_else(|| "src".to_string());
        let src_dir = std::path::Path::new(&entry)
            .parent()
            .map_or_else(|| "src".to_string(), |p| p.to_string_lossy().into_owned());
        let mut lib_dirs = vec![root.join(&src_dir).to_string_lossy().into_owned()];
        if !manifest.dependencies.is_empty()
            && let Some(parent) = root.parent()
        {
            lib_dirs.push(parent.to_string_lossy().into_owned());
        }
        // Every tests/*.loft, in name order (stable output for the panel).
        let tests_dir = root.join("tests");
        if !tests_dir.is_dir() {
            return Err(Error::new(
                ErrorKind::NotFound,
                format!("package {} has no tests/ directory", root.display()),
            ));
        }
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&tests_dir)?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("loft"))
            })
            .collect();
        files.sort();
        let mut out = Vec::with_capacity(files.len());
        for f in files {
            let name = f.file_name().map_or_else(
                || f.to_string_lossy().into_owned(),
                |n| n.to_string_lossy().into_owned(),
            );
            let results = self.run_file_tests_with(&f.to_string_lossy(), &lib_dirs)?;
            out.push((name, results));
        }
        Ok(out)
    }

    /// @PLN16 M5e slice 3 — set the **write sandbox**: the one file the editor may save
    /// back to (the `--serve` target), stored canonical.  An unreadable path leaves the
    /// sandbox `None`, so every [`write_file`](Self::write_file) is then refused.
    pub fn set_workspace_file(&mut self, path: &str) {
        self.workspace_file = crate::portable_path::try_plain_canonical(std::path::Path::new(path));
    }

    /// @PLN16 M5e slice 3 — save the editor's `content` to `path`, **only** if `path`
    /// canonicalises to the sandboxed workspace file ([`set_workspace_file`]).  Any other
    /// path — or no sandbox set — is refused, so the browser can never write outside the
    /// one file it opened.
    ///
    /// # Errors
    /// `Err` with a message when no sandbox is set, the target is outside it, or the write
    /// fails.
    pub fn write_file(&self, path: &str, content: &str) -> Result<(), String> {
        let Some(allowed) = &self.workspace_file else {
            return Err("no writable workspace (server has no file sandbox)".to_string());
        };
        // Canonicalise the request against the sandbox: the file exists (we are overwriting
        // it), so a path that resolves anywhere else — `..`, a symlink, an absolute escape —
        // fails this equality and is refused.
        match crate::portable_path::try_plain_canonical(std::path::Path::new(path)) {
            Some(p) if &p == allowed => {
                std::fs::write(allowed, content).map_err(|e| format!("write failed: {e}"))
            }
            _ => Err("path is outside the editable file".to_string()),
        }
    }

    /// @PLN16 M5e slice 6 — launch `file` as a **game**: a real `loft` run in its own child
    /// process (with the session's `--lib` dirs), so its frame loop — and, once the graphics
    /// library lands on this branch, its native window — never blocks the serve loop.
    /// stdout/stderr are drained into a buffer the poll-based [`game_status`](Self::game_status)
    /// hands back in chunks.  The binary is this process's own executable (`loft debug --serve`
    /// IS the loft binary); `LOFT_BIN` overrides it for tests, whose `current_exe` is the test
    /// harness.  One game at a time: launching over a live game is refused (stop it first).
    ///
    /// # Errors
    /// A message when a game is already running or the spawn fails.
    pub fn launch_game(&mut self, file: &str) -> Result<(), String> {
        if let Some(g) = &mut self.game
            && matches!(g.child.try_wait(), Ok(None))
        {
            return Err("a game is already running — stop it first".to_string());
        }
        let bin = std::env::var_os("LOFT_BIN").map_or_else(
            || std::env::current_exe().map_err(|e| format!("cannot locate loft binary: {e}")),
            |b| Ok(std::path::PathBuf::from(b)),
        )?;
        let mut cmd = std::process::Command::new(bin);
        cmd.arg(file);
        for d in &self.parser.lib_dirs {
            cmd.arg("--lib").arg(d);
        }
        // @PLN18 02 (the 6b wire-up): an IDE-launched game is live-editable
        // by default — the child's file watcher reacts to every IDE save and
        // hot-swaps the edited fn (tier 0); its `live-reload:` stderr lines
        // are the structured feedback.  LOFT_LIVE_RELOAD=0 opts out.
        if std::env::var_os("LOFT_LIVE_RELOAD").is_none() {
            cmd.env("LOFT_LIVE_RELOAD", "1");
        }
        // @PLN18 08-S7 editor support — an IDE-launched game is DEBUGGABLE by
        // default: the D!: control channel answers on the game's port
        // (loopback-only) and a compiled game keeps the parked interpreter
        // for breakpoint flips.  Opt out with =0 (the LIVE_RELOAD pattern).
        if std::env::var_os("LOFT_DEBUG_CONTROL").is_none() {
            cmd.env("LOFT_DEBUG_CONTROL", "1");
        }
        if std::env::var_os("LOFT_LIVE_FLIP").is_none() {
            cmd.env("LOFT_LIVE_FLIP", "1");
        }
        cmd.stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        // A `--native` game serves from a GRANDCHILD of this child (the S1
        // process-model finding) — own group so stop_game can reach it all.
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("cannot launch game: {e}"))?;
        let output = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        drain_pipe(child.stdout.take(), &output);
        drain_pipe(child.stderr.take(), &output);
        self.game = Some(GameProc { child, output });
        Ok(())
    }

    /// The running game's state: `(still running, output drained since the last call,
    /// exit code if it ended)`.  `None` when no game was launched.  When the game has
    /// ended the slot is cleared, so the next [`launch_game`](Self::launch_game) is free.
    pub fn game_status(&mut self) -> Option<(bool, String, Option<i32>)> {
        let g = self.game.as_mut()?;
        let chunk = std::mem::take(
            &mut *g
                .output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        match g.child.try_wait() {
            Ok(None) => Some((true, chunk, None)),
            Ok(Some(status)) => {
                self.game = None;
                Some((false, chunk, status.code()))
            }
            Err(_) => {
                self.game = None;
                Some((false, chunk, None))
            }
        }
    }

    /// Stop the running game (kill the child this session spawned — never any other
    /// process).  Returns the output drained since the last poll, or `None` when no game
    /// is running.
    pub fn stop_game(&mut self) -> Option<String> {
        let mut g = self.game.take()?;
        // Stop the whole TREE this session created, not just the handle it holds: a
        // `--native` game's real server is a GRANDCHILD (driver → compiled binary), so
        // reaping the child alone leaves the server running and holding its port.
        //
        // Unix reaches the tree through the process group the launch put the child in.
        // Windows has no process group, so it walks the tree by parent link instead —
        // and the ordering is the whole of it: `taskkill /T` needs the child ALIVE to
        // walk from, so it must run BEFORE the kill below, not after.  Measured on
        // `windows-latest`: the grandchild survives a bare `child.kill()` and still
        // holds its port, and `taskkill /T /F` terminates it ("the process with PID N
        // (child process of PID M) has been terminated") — see WINDOWS.md § Known gaps.
        #[cfg(unix)]
        unsafe {
            libc::killpg(g.child.id() as i32, libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/T", "/F", "/PID", &g.child.id().to_string()])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
        let _ = g.child.kill();
        let _ = g.child.wait();
        Some(std::mem::take(
            &mut *g
                .output
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        ))
    }

    /// @PLN16 M5a — set a `file:line` breakpoint for the file-run debugger.  Stored and
    /// re-applied to each observing run's fresh `State` via
    /// `State::set_breakpoint_file_line`.
    pub fn add_file_breakpoint(&mut self, file: &str, line: u32) {
        self.push_breakpoint(BreakSpec {
            location: BpLocation::File(file.to_string(), line),
            condition: None,
            actions: Vec::new(),
            stop: true,
        });
    }

    /// @PLN16 phase 2 — the wire protocol's `setBreakpoints` unit: a `file:line`
    /// breakpoint with an optional condition (break only when it holds) and tracepoint
    /// actions (`stop:false` → log the expressions and continue).
    pub fn add_file_breakpoint_rich(
        &mut self,
        file: &str,
        line: u32,
        condition: Option<String>,
        actions: Vec<String>,
        stop: bool,
    ) {
        self.push_breakpoint(BreakSpec {
            location: BpLocation::File(file.to_string(), line),
            condition,
            actions,
            stop,
        });
    }

    /// @PLN16 phase 2 — run `input` as an observing statement (the RPC `run` request),
    /// returning whether it suspended at a breakpoint (vs ran to completion).
    pub fn eval_observe(&mut self, input: &str) -> bool {
        let _ = self.eval(input);
        self.is_debugging()
    }

    /// @PLN16 M5a — the breakable source lines in `file` of the loaded program (compiles
    /// it), for the file-run debugger's "no breakable op on line N — try one of these"
    /// hint.
    #[must_use]
    pub fn breakable_lines_in_file(&mut self, file: &str) -> Vec<u32> {
        crate::scopes::check(&mut self.parser.data, &mut self.parser.database);
        let mut state = State::new(self.parser.database.clone());
        compile::byte_code(&mut state, &mut self.parser.data);
        state.breakable_lines_in_file(file, &self.parser.data)
    }

    /// @PLN16 M5a — whether the session defines a function `name` (user or stdlib).
    #[must_use]
    pub fn defines_function(&self, name: &str) -> bool {
        self.parser.data.def_nr(&format!("n_{name}")) < self.parser.data.definitions()
    }

    /// The breakpoint + tracepoint specs set this session, rendered for display.
    #[must_use]
    pub fn breakpoints(&self) -> Vec<String> {
        self.breakpoints.iter().map(BreakSpec::describe).collect()
    }

    /// Remove all breakpoints + tracepoints (function-scoped and `file:line`).
    pub fn clear_breakpoints(&mut self) {
        self.breakpoints.clear();
        self.bp_meta.clear();
    }

    /// Frames captured at breakpoints during the most recent observing run.
    #[must_use]
    pub fn last_hits(&self) -> &[crate::debugger::BreakHit] {
        &self.last_hits
    }

    /// @PLN16 G1 — turn **interactive stepping** on or off.  When on, an observing
    /// run that reaches a breakpoint *suspends* into the paused sub-mode (rather
    /// than recording every hit and continuing): inspect with
    /// [`paused_frame`](Self::paused_frame), edit with [`debug_set`](Self::debug_set),
    /// resume with [`debug_step`](Self::debug_step) / [`debug_continue`](Self::debug_continue).
    /// The interactive REPL driver enables it; programmatic callers that want the
    /// full hit list (e.g. a conditional-breakpoint sweep via
    /// [`frame_holds`](Self::frame_holds)) leave it off.
    pub fn debug_stepping(&mut self, on: bool) {
        self.stepping = on;
    }

    /// Whether a run is currently **suspended** at a breakpoint (the paused
    /// sub-mode is active).
    #[must_use]
    pub fn is_debugging(&self) -> bool {
        self.paused.is_some()
    }

    /// The frame at the current suspension, or `None` if not paused.
    #[must_use]
    pub fn paused_frame(&self) -> Option<&crate::debugger::BreakHit> {
        self.paused.as_deref().and_then(State::paused_frame)
    }

    /// The full runtime call stack at the current suspension (innermost frame first), each
    /// frame carrying its function, source line, and live locals — the multi-frame
    /// `stackTrace` source (@PLN63 SF).  Empty when not paused.
    #[must_use]
    pub fn paused_stack(&self) -> Vec<crate::debugger::BreakHit> {
        self.paused
            .as_deref()
            .map(|s| s.break_stack(&self.parser.data))
            .unwrap_or_default()
    }

    /// The source line the current suspension is stopped on, or `None` if not paused —
    /// drives the browser debugger's current-line marker (it moves as you step).
    #[must_use]
    pub fn paused_line(&self) -> Option<u32> {
        self.paused.as_deref().and_then(State::paused_line)
    }

    /// Edit scalar local `name` in the **live** paused frame to the value of `rhs`
    /// (the user types `n = 99`, `f = 2.0`, `b = !b` at the paused prompt), then
    /// refresh the frame view.  `rhs` is **evaluated against the frame** first (so
    /// it may be any expression — `n + 1`, `!b` — not just a literal), then written
    /// type-directed by the local's declared type.  Returns `false` when not paused,
    /// `rhs` doesn't evaluate, `name` isn't a local, the value's type doesn't match
    /// the local, or the local is text / heap (those need the store-resident
    /// write-back, not yet built).  The edit is picked up when the run resumes — the
    /// @PLN16 F edit-and-continue, driven from the REPL.
    pub fn debug_set(&mut self, name: &str, rhs: &str) -> bool {
        let Some(lit) = self.debug_eval(rhs) else {
            return false;
        };
        if self.paused.is_none() {
            return false;
        }
        // @PLN16 M2 — arm the per-edit undo journal; the frame-write sites record their
        // before/after bytes into it, and a successful edit commits it to the undo stack.
        if let Some(state) = self.paused.as_deref_mut() {
            state.begin_edit_journal();
        }
        // A `[index]` LHS is a vector element edit (`v[1]`); a dotted LHS is a
        // struct-field path edit (`pt.x`, `pt.inner.x` — nested structs are inlined, so
        // the path resolves by summed offsets); a bare name is a whole-local edit
        // (inline in place, or — for a heap value — a freshly-built value grafted into
        // the live stores).  Each branch takes its own short borrow of the paused
        // state so the heap path can re-enter `self` to build the value.
        // `paused` is `Some` (guarded above), so each branch's `let Some` binds — the
        // `else` is only there to keep the borrow short (a fresh borrow per branch so
        // the heap path can re-enter `self` to build the value).
        let ok = if let Some(open) = name.find('[') {
            // `v[i]` — bare-local vector element only for now; a dotted base
            // (`s.items[0]`) is the nested-path-plus-index case, not yet handled.
            let base = name[..open].trim();
            let idx = name[open + 1..]
                .strip_suffix(']')
                .and_then(|s| s.trim().parse::<i64>().ok());
            let Some(state) = self.paused.as_deref_mut() else {
                return false;
            };
            match idx {
                Some(i) if !base.contains('.') => {
                    state.set_frame_element(base, i, &lit, &self.parser.data)
                }
                _ => false,
            }
        } else if name.contains('.') {
            let mut segs = name.split('.').map(str::trim);
            let base = segs.next().unwrap_or("");
            let path: Vec<&str> = segs.collect();
            let Some(state) = self.paused.as_deref_mut() else {
                return false;
            };
            state.set_frame_path(base, &path, &lit, &self.parser.data)
        } else {
            // Inline scalar / text / simple-enum edits in place; a heap local
            // (struct / vector / struct-enum) needs the value **materialised** in the
            // live store and the slot repointed at it (@PLN16 M1a).
            let inplace = match self.paused.as_deref_mut() {
                Some(state) => state.set_frame_literal(name, &lit, &self.parser.data),
                None => return false,
            };
            if inplace {
                true
            } else if let Some(ty) = self.frame_heap_type(name) {
                match self.materialize_heap_value(&ty, &lit) {
                    Some(root) => match self.paused.as_deref_mut() {
                        Some(state) => state.set_frame_dbref(name, root, &self.parser.data),
                        None => false,
                    },
                    None => false,
                }
            } else {
                false
            }
        };
        if let Some(state) = self.paused.as_deref_mut() {
            if ok {
                // Push the recorded edit onto the undo stack; refresh the frame view.
                // `name` labels the entry, so a drop notice at a later pause can say
                // WHICH edit is no longer undoable.
                state.commit_edit_journal(name, &self.parser.data);
                state.refresh_paused_frame(&self.parser.data);
            } else {
                state.discard_edit_journal();
            }
        }
        ok
    }

    /// @PLN16 M2 — undo the last interactive edit at this suspension (reverts its
    /// journal) and refresh the frame view.  `false` when there is nothing to undo or
    /// no pause.  The undone edit is then available via [`debug_redo`](Self::debug_redo).
    pub fn debug_undo(&mut self) -> bool {
        let Some(state) = self.paused.as_deref_mut() else {
            return false;
        };
        let ok = state.debug_undo();
        if ok {
            state.refresh_paused_frame(&self.parser.data);
        }
        ok
    }

    /// @PLN16 M2 — redo the last undone edit (re-applies its journal) and refresh the
    /// frame view.  `false` when there is nothing to redo or no pause.
    pub fn debug_redo(&mut self) -> bool {
        let Some(state) = self.paused.as_deref_mut() else {
            return false;
        };
        let ok = state.debug_redo();
        if ok {
            state.refresh_paused_frame(&self.parser.data);
        }
        ok
    }

    /// @PLN16 M3 — set a **watchpoint** on the scalar heap region named by `expr`
    /// (`pt.x`, `v[i]`) at the current pause: a resumed run pauses when a later write
    /// changes it.  `false` when not paused or the expression isn't a watchable region.
    pub fn add_watchpoint(&mut self, expr: &str) -> bool {
        self.paused
            .as_deref_mut()
            .is_some_and(|s| s.add_watchpoint(expr, &self.parser.data))
    }

    /// @PLN16 M3 — the active watchpoint labels (`:watch` list).
    #[must_use]
    pub fn watchpoints(&self) -> Vec<String> {
        self.paused
            .as_deref()
            .map_or_else(Vec::new, State::watchpoint_labels)
    }

    /// @PLN16 M3 — remove all watchpoints.
    pub fn clear_watchpoints(&mut self) {
        if let Some(s) = self.paused.as_deref_mut() {
            s.clear_watchpoints();
        }
    }

    /// @PLN16 M3 — the watchpoint that fired during the most recent resume (label +
    /// old → new), taken so the driver reports it once.
    #[must_use]
    pub fn take_watch_hit(&mut self) -> Option<crate::debugger::WatchHit> {
        self.paused.as_deref_mut().and_then(State::take_watch_hit)
    }

    /// @PLN16 M1a — the loft-source type name of a **heap** frame local at the current
    /// pause (struct / vector / struct-enum / collection — a `DbRef` slot), or `None`
    /// when not paused or the local is inline / unknown.  Routes the whole-value edit.
    fn frame_heap_type(&self, name: &str) -> Option<String> {
        self.paused
            .as_deref()?
            .frame_heap_type(name, &self.parser.data)
    }

    /// @PLN16 M1a — build the self-contained own-format heap literal `lit` (loft type
    /// `ty`) into the live paused stores and return its root `DbRef`.  The constructor
    /// runs on a **throwaway build `State`** whose store high-water is raised above the
    /// live stores' ([`Stores::raise_floor`](crate::database::Stores::raise_floor)), so
    /// every value-store it claims sits on a slot FREE in the live store; those stores
    /// are then grafted into the paused State with **no `DbRef` remap** (slots
    /// coincide — [`Stores::adopt_value_stores`](crate::database::Stores::adopt_value_stores)).
    /// The suspended frame is never touched (a separate State, a separate stack).
    /// `None` when not paused, the wrapper doesn't compile, or the constructor faults.
    fn materialize_heap_value(&mut self, ty: &str, lit: &str) -> Option<crate::keys::DbRef> {
        let live_top = self.paused.as_deref()?.database.high_water();
        let next = self.counter + 1;
        let name = format!("replmain_{next}");
        let src = format!("fn {name}() -> {ty} {{\n{lit}\n}}\n");
        let sp = self.savepoint();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        let failed = self.parser.diagnostics.entries()[pre_diag..]
            .iter()
            .any(|e| e.level >= Level::Error);
        if failed {
            self.rewind(sp);
            return None;
        }
        self.counter = next;
        crate::scopes::check(&mut self.parser.data, &mut self.parser.database);
        let mut build = State::new(self.parser.database.clone());
        compile::byte_code(&mut build, &mut self.parser.data);
        // Force the value above BOTH stores' high-water → its slots are free in live.
        let floor = live_top.max(build.database.high_water());
        build.database.raise_floor(floor);
        // #629 follow-up — this wrapper RETURNS a value and `render_capture`
        // reads it off the stack after the run, so claim the hidden return
        // buffer: the entry teardown must not free what we are about to read.
        build.keep_entry_return();
        build.execute_argv(&name, &self.parser.data, &[]);
        let root = if build.database.runtime_error.take().is_some() {
            None
        } else {
            // The root `DbRef` is the function's return on the build stack top; its
            // store_nr is in `[floor, build.max)`, valid in live after the graft.
            let db = *build.get_stack::<crate::keys::DbRef>();
            match self.paused.as_deref_mut() {
                Some(live) => {
                    live.database.adopt_value_stores(&mut build.database, floor);
                    Some(db)
                }
                None => None,
            }
        };
        self.rewind(sp); // discard the throwaway cap gen
        root
    }

    /// Resume the suspended run, stopping per `mode` (the step verbs —
    /// [`StepMode`](crate::debugger::StepMode)).  Returns `true` if it paused again
    /// (the new frame is in [`paused_frame`](Self::paused_frame)), `false` if the
    /// run finished — in which case the paused sub-mode is left
    /// ([`is_debugging`](Self::is_debugging) becomes `false`).
    pub fn debug_step(&mut self, mode: crate::debugger::StepMode) -> bool {
        self.trace_output.clear();
        if self.paused.is_none() {
            return false;
        }
        if !self.resume_continue_with(mode) {
            return false;
        }
        // @PLN16 rich-bp — honour the condition / tracepoint of whatever we stopped at.
        self.resolve_pause();
        // @PLN120 F — an edit that stopped being undoable says so, once, on the same
        // channel as arc B's diagnostics (so the interactive prompt prints it AND the
        // RPC surface emits it as an `output` event, from one place).  A correct edit
        // disappearing wordlessly is the failure this arc closes.
        let dropped = self
            .paused
            .as_deref()
            .map(State::dropped_undo)
            .unwrap_or_default();
        for (label, why) in dropped {
            self.trace_output.push(format!(
                "the edit to `{label}` is no longer undoable — {why}"
            ));
        }
        self.is_debugging()
    }

    /// Raw resume by `mode` (the engine `debug_step`, no condition/tracepoint
    /// resolution — that is [`resolve_pause`](Self::resolve_pause)'s job).  Returns
    /// whether still paused.
    fn resume_continue_with(&mut self, mode: crate::debugger::StepMode) -> bool {
        let armed = self.reverse_armed;
        let Some(state) = self.paused.as_deref_mut() else {
            return false;
        };
        // @PLN63 RX — keep the running frame's reverse ring armed per the session pref, so a
        // forward step checkpoints its pre-step state (a no-op cost when reverse is off).
        state.set_reverse(armed);
        let still = state.debug_step(mode, &self.parser.data);
        if !still {
            self.paused = None;
        }
        still
    }

    /// @PLN63 RX — arm (or disarm) reverse stepping for this session.  Stored as a pref and
    /// applied to the live paused frame + every subsequent step, so [`debug_step_back`] can
    /// reverse them.  Off by default (a normal debug session pays no snapshot cost).
    pub fn set_reverse(&mut self, on: bool) {
        self.reverse_armed = on;
        if let Some(s) = self.paused.as_deref_mut() {
            s.set_reverse(on);
        }
    }

    /// @PLN63 RX — step **backward** one step: restore the most recent checkpoint (heap +
    /// registers) so the paused frame returns to the state before the last forward step.
    /// Returns `false` at the ring floor (nothing earlier retained); the frame stays paused
    /// and unchanged either way.
    pub fn debug_step_back(&mut self) -> bool {
        self.trace_output.clear();
        match self.paused.as_deref_mut() {
            Some(s) => s.step_back(&self.parser.data),
            None => false,
        }
    }

    /// @PLN16 rich-bp — after a pause, honour the breakpoint's facets: a conditional
    /// break whose condition is false auto-resumes; a tracepoint evaluates its actions
    /// (collected into `trace_output`) and auto-resumes; loop until a real stop or the
    /// run finishes.  Leaves `self.paused` reflecting the outcome.
    fn resolve_pause(&mut self) {
        loop {
            let off = match self.paused.as_deref() {
                Some(s) => s.paused_at_breakpoint(),
                None => return, // run finished
            };
            let Some(off) = off else {
                return; // a step / watch pause is a real stop
            };
            let Some(meta) = self.bp_meta.get(&off).cloned() else {
                return; // a plain breakpoint with no metadata: stop
            };
            // Conditional break — THREE outcomes, not two (@PLN120 B).  `debug_eval`
            // returns `None` when the condition cannot be evaluated at this frame (a
            // typo, or a name that is not in scope here).  Treating that as "false"
            // is what made a breakpoint report `verified: true` at setBreakpoints and
            // then never fire, with nothing said: the client was promised a
            // breakpoint that could not exist.  Say so — once, because a hot line
            // would bury the frame — and STOP, so a user who typo'd a condition
            // lands at the line instead of reading a warning they may never see.
            if let Some(cond) = &meta.condition {
                let cond = cond.clone();
                match self.debug_eval(&cond).as_deref() {
                    Some("true") => {}
                    Some(_) => {
                        if !self.resume_continue_with(crate::debugger::StepMode::Continue) {
                            return;
                        }
                        continue;
                    }
                    None => {
                        if self.cond_unevaluable.insert(off) {
                            self.trace_output.push(format!(
                                "breakpoint condition `{cond}` cannot be evaluated here \
                                 — stopping anyway so it is not silently never hit; \
                                 check the names are in scope at this line (`:vars`)"
                            ));
                        }
                    }
                }
            }
            // Tracepoint (stop = false): emit the actions and continue.
            if !meta.stop {
                for a in &meta.actions {
                    let v = self.debug_eval(a).unwrap_or_else(|| "?".to_string());
                    self.trace_output.push(format!("{a} = {v}"));
                }
                if !self.resume_continue_with(crate::debugger::StepMode::Continue) {
                    return;
                }
                continue;
            }
            return; // real stop (condition held / no condition, stop = true)
        }
    }

    /// @PLN120 F — the undo entries the current pause dropped, `(label, reason)`.
    #[must_use]
    pub fn dropped_undo_here(&self) -> Vec<(String, String)> {
        self.paused
            .as_deref()
            .map(crate::state::State::dropped_undo)
            .unwrap_or_default()
    }

    /// @PLN16 rich-bp — drain the tracepoint emissions from the most recent resume (the
    /// driver prints them; the wire protocol sends them as `output` events).
    #[must_use]
    pub fn take_trace_output(&mut self) -> Vec<String> {
        std::mem::take(&mut self.trace_output)
    }

    /// Continue to the next breakpoint or the end of the run (the `:continue`
    /// verb) — [`debug_step`](Self::debug_step) with
    /// [`StepMode::Continue`](crate::debugger::StepMode::Continue).
    pub fn debug_continue(&mut self) -> bool {
        self.debug_step(crate::debugger::StepMode::Continue)
    }

    /// Evaluate `expr` against the **current paused frame** and render its value as
    /// an own-format loft literal (`"15"`, `"Point{x:3,y:4}"`), or `None` if it
    /// doesn't evaluate / there is no pause.  This is the REPL-at-frame: typing an
    /// expression at the `(dbg)` prompt reads the frame's live variables (`n * 2`,
    /// `pt.x * pt.y`).  It reuses the D1 bridge — the frame's captured variables
    /// render to literals, so binding them as the evaluation body and running
    /// [`value_of`](Self::value_of) covers **every** type the frame holds.
    ///
    /// Read-only: it runs on a throwaway `State` (the held paused state is
    /// untouched), and any value the user has edited with
    /// [`debug_set`](Self::debug_set) is already reflected in the frame's literals.
    /// The session body is swapped to the frame bindings only for this call and
    /// always restored — even if the evaluation unwinds.
    pub fn debug_eval(&mut self, expr: &str) -> Option<String> {
        self.debug_eval_fmt(expr, false)
    }

    /// @PLN120 A — why a bare local name that IS in the paused frame still cannot be
    /// read or written: the frame does not hold its value.  `None` when the name is
    /// not a frame local, or is one the frame holds (so the failure has another
    /// cause).  A `<reused by …>` local is readable one line earlier, and saying so
    /// is the difference between a diagnosis and "couldn't evaluate".
    #[must_use]
    pub fn unheld_local_reason(&self, expr: &str) -> Option<String> {
        let name = expr.trim();
        if name.is_empty()
            || !name
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '#')
        {
            return None;
        }
        let state = self
            .paused
            .as_deref()?
            .frame_local_state(name, &self.parser.data)?;
        match state {
            // Held: the failure has another cause, so the caller's generic message is
            // the right one.
            crate::state::LocalState::Held => None,
            // @PLN120 A follow-up — a name that IS a local of this function but is not
            // in scope on this line used to get the same "couldn't evaluate" as a typo,
            // which is what made the consumer read a working debugger as broken (their
            // case: the loop iterator, read while stopped on the `for` line).
            crate::state::LocalState::OutOfScope => Some(format!(
                "`{name}` is a local of this function but is not in scope at this line \
                 — break inside the block that declares it"
            )),
            crate::state::LocalState::Unset => Some(format!(
                "`{name}` is in scope but has no value yet at this line — \
                 its first assignment has not run"
            )),
            crate::state::LocalState::Reused(by) => Some(format!(
                "`{name}` is in scope but the frame no longer holds it — \
                 its stack slot was reused by `{by}`; break earlier in the \
                 function to read it"
            )),
        }
    }

    /// @PLN16 phase 2 — evaluate `expr` against the paused frame and render the result
    /// as **JSON** — the form the wire protocol's `eval` reply carries.  A bare heap
    /// local is read live and serialised with `show_json` (**D2**); a computed
    /// struct/enum goes through `.to_json()`; scalars are raw JSON literals.  `None`
    /// when not paused or it doesn't evaluate.
    pub fn debug_eval_json(&mut self, expr: &str) -> Option<String> {
        self.debug_eval_fmt(expr, true)
    }

    /// @PLN16 M5e — the REPL panel's **context-aware** evaluation. When paused at a
    /// breakpoint it evaluates against the **live frame** (D1/D2, `debug_eval`); otherwise
    /// against the **session top level** (the normal `eval` — defines, bindings, statements,
    /// expressions). Incomplete input returns `more` (the multi-line continuation, which is
    /// **non-mutating** — the session is unchanged, so the buffer stays freely editable). A
    /// top-level expression's value is printed to the output sink (the transport drains it
    /// into the REPL pane); a frame value comes back in [`ReplOutcome::value`]. Runs the
    /// input **exactly once** (it routes through `eval`'s own def/binding/observe paths).
    pub fn repl_eval(&mut self, input: &str, json: bool) -> ReplOutcome {
        if self.is_debugging() {
            return ReplOutcome {
                context: "frame",
                more: false,
                value: self.debug_eval_fmt(input, json),
                diagnostics: Vec::new(),
            };
        }
        if Parser::statement_incomplete(input) {
            return ReplOutcome {
                context: "top",
                more: true,
                value: None,
                diagnostics: Vec::new(),
            };
        }
        match self.eval(input) {
            Eval::Ran | Eval::Paused => ReplOutcome {
                context: "top",
                more: false,
                value: None,
                diagnostics: Vec::new(),
            },
            Eval::NeedMore => ReplOutcome {
                context: "top",
                more: true,
                value: None,
                diagnostics: Vec::new(),
            },
            Eval::Error(diagnostics) => ReplOutcome {
                context: "top",
                more: false,
                value: None,
                diagnostics,
            },
        }
    }

    /// @PLN98 P1b — the invariant-honouring live-frame eval.  When `expr`
    /// references a keyed-collection (`hash`/`sorted`/`index`) paused-frame local
    /// — which the reconstruct path can't text-seed (it renders non-reparseable)
    /// — bind that local as a typed argument of a synthetic eval fn and pass its
    /// **live `DbRef`** into a `reenter_ret` over the paused State
    /// ([`State::eval_frame_reenter`]), reading the collection where it lives.
    /// Other referenced locals (scalars / vectors / structs reparse fine) stay in
    /// the seed prefix.  Returns `None` when no keyed local is referenced — so the
    /// caller's text-seed path still handles every previously-working expression
    /// (`2 + 2`, a struct via `.to_json()`, a bare heap read) unchanged.
    fn eval_frame_expr(&mut self, expr: &str, json: bool) -> Option<String> {
        let idents = expr_idents(expr);
        // Split the referenced live locals into keyed collections (the live-arg
        // path) and everything else (seed prefix), in the paused frame's
        // declaration order — a stable signature/push order.
        let (keyed, seed) = {
            let state = self.paused.as_deref()?;
            let frame = state.paused_frame()?;
            let mut keyed: Vec<(String, String)> = Vec::new();
            let mut seed = String::new();
            for (name, lit) in frame.held_locals() {
                if !idents.contains(name.as_str()) {
                    continue;
                }
                if let Some(ty) = state.frame_keyed_type_source(name, &self.parser.data) {
                    keyed.push((name.clone(), ty));
                } else {
                    seed.push_str(name);
                    seed.push_str(" = ");
                    match state.eval_frame_heap(name, false, &self.parser.data) {
                        Some(full) => seed.push_str(&full),
                        None => seed.push_str(lit),
                    }
                    seed.push_str(";\n");
                }
            }
            (keyed, seed)
        };
        if keyed.is_empty() {
            return None; // no keyed local — the text-seed path handles it
        }
        let arg_names: Vec<String> = keyed.iter().map(|(n, _)| n.clone()).collect();
        let sig = keyed
            .iter()
            .map(|(n, t)| format!("{n}: {t}"))
            .collect::<Vec<_>>()
            .join(", ");
        // Pass 1 — infer the result type with the keyed args + seed in scope.
        // The raw shown type, because the `?` cannot survive `base_type_name`: that helper
        // drops the DEP LIST by splitting on `[`, and a `τ?` prints its `?` AFTER the list
        // (`ref(HRec)["h"]?`), so everything past the bracket goes with it and a nullable
        // reads as its NON-NULL twin.  Ask the raw form for nullability and the helper for
        // the base name — two facts, two reads.
        let shown = self.infer_frame_type(&sig, &seed, expr)?;
        let nullable = shown.ends_with('?');
        let ret = base_type_name(&shown);
        // A **scalar** result rides the frame base and is read straight back.
        if is_scalar_type_name(&ret) && !nullable {
            return self.eval_frame_build_run(&sig, &seed, expr, &ret, &arg_names, json);
        }
        // A **NULLABLE scalar** cannot: the wrapper would declare `-> integer` and be handed
        // a `τ?`.  It has no `.to_json()` to route through either — that method is heap-only
        // (`integer.to_json` is *Unknown field*) — so it binds and interpolates, which is
        // what the frame RENDERER does one site over for the same values (loft#1459).
        //
        // Measured, because the whole question is whether the result is still JSON on the
        // path whose caller reads it as JSON: `integer` → `42`, `float` → `1.5`, `single` →
        // `2.5`, `boolean` → `true` are all valid JSON tokens bare.  `character` → `x` is
        // NOT, and quoting it here would need JSON escaping loft cannot spell for a `'"'` or
        // a `'\\'`.  So a character is evaluable on the INTERACTIVE path and still declines
        // under `--rpc`, which is the honest split: every caller gets what it can encode,
        // and none gets something that only looks encoded.
        // …asked of the BARE name.  `base_type_name` deliberately re-attaches the trailing
        // `?` (it is part of the source name), and `is_scalar_type_name` lists the non-null
        // spellings — so `integer?` matched neither the scalar branch above nor this one, and
        // every nullable scalar fell through to the heap branch below, failed to compile
        // `.to_json()` on an integer, and rendered the text-seed path's graceful `null`.  A
        // debugger answering "absent" about a value it can see, which is the failure loft#1459
        // opened this branch to end: the branch was right and unreachable.
        let ret_bare = ret.strip_suffix('?').unwrap_or(ret.as_str()).to_string();
        if nullable && is_scalar_type_name(&ret_bare) && !(json && ret_bare == "character") {
            // TWO re-entries, both on the plain-scalar path above, because that is the only
            // shape measured to survive a frame teardown.  The single-wrapper form this branch
            // used to carry — bind `__ev`, then yield `"null"` or `"{__ev?}"` — returns a text
            // INTERPOLATED in the wrapper's own frame, and unlike the heap branch's
            // `.to_json()` that is not a call-returned-owned text: it is the raw `Str`-off-the
            // -stack read @P293 forbids, and it SIGSEGVs on re-entry.  The branch was
            // unreachable when it was written (`base_type_name` re-attaches the `?`, which
            // `is_scalar_type_name` rejects), so nothing had ever run it.
            //
            // Ask presence first, then the value: a `boolean` and then the bare scalar, each
            // the shape the first branch already proves safe.  The cost is that `expr` is
            // evaluated twice, so this is confined to a CALL-FREE expression — a projection or
            // a subscript, which is what a paused frame is asked about.  Anything with a call
            // keeps the old graceful decline rather than running a side effect twice.
            if !expr.contains('(') {
                let absent = self.eval_frame_build_run(
                    &sig,
                    &seed,
                    &format!("({expr}) == null"),
                    "boolean",
                    &arg_names,
                    json,
                )?;
                if absent.trim() == "true" {
                    return Some("null".to_string());
                }
                return self.eval_frame_build_run(
                    &sig,
                    &seed,
                    &format!("({expr})?"),
                    &ret_bare,
                    &arg_names,
                    json,
                );
            }
        }
        if is_scalar_type_name(&ret) {
            return self.eval_frame_build_run(&sig, &seed, expr, &ret, &arg_names, json);
        }
        // A **NULLABLE** result binds first and branches.  `@FR-Col-Lookup` gives every
        // keyed point lookup a `?` (loft#1450), so `h["a"]` at a paused frame — the exact
        // expression this whole path was built for — is now `HRec?`, and `.to_json()` is
        // not a method on a `τ?`.  The wrapper stopped compiling, fell to `None`, and the
        // text-seed path rendered its graceful `null` for a key that is PRESENT: a
        // debugger answering "absent" about a record it can see.
        //
        // Discharging with `?` alone would compile and be WORSE — a miss would render as
        // the element type's zero record, which is a wrong answer wearing the right shape,
        // and a debugger is the one place that must not invent a value.  So bind once and
        // ask: absent reads `null`, present reads its record.  Binding also keeps `expr`
        // evaluated exactly once, which matters when it is a call.
        if nullable {
            return self.eval_frame_build_run(
                &sig,
                &format!("{seed}__ev = ({expr});\n"),
                "if __ev == null { \"null\" } else { __ev?.to_json() }",
                "text",
                &arg_names,
                json,
            );
        }
        // A **heap** result (struct / vector / struct-enum) can't be returned via
        // `reenter_ret` — it is destination-passed, so the frame base still holds
        // the first arg.  Serialise it in-fn with `.to_json()` — a
        // call-returned-owned text that survives the frame teardown (@P293-safe) —
        // and return the raw JSON.  A type with no `.to_json()` (a bare `text`
        // field, an odd result) fails the compile → `None` → the text-seed path
        // renders it (its previous graceful `null` for a keyed reference).
        self.eval_frame_build_run(
            &sig,
            &seed,
            &format!("({expr}).to_json()"),
            "text",
            &arg_names,
            json,
        )
    }

    /// @PLN98 P1b — infer the loft type of `expr` evaluated with the keyed-arg
    /// signature `sig` and seed prefix `seed` in scope (a paused-frame-aware
    /// [`infer_type`](Self::infer_type)): compile `fn _(sig) { seed __t = (expr); }`
    /// and read `__t`'s type.  The throwaway def is rolled back.  `None` when the
    /// wrapper doesn't type-check.
    fn infer_frame_type(&mut self, sig: &str, seed: &str, expr: &str) -> Option<String> {
        let name = format!("replmain_{}", self.counter + 1);
        let src = format!("fn {name}({sig}) {{\n{seed}__t = ({expr});\n}}\n");
        let sp = self.savepoint();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        let failed = self.parser.diagnostics.entries()[pre_diag..]
            .iter()
            .any(|e| e.level >= Level::Error);
        let result = if failed {
            None
        } else {
            let d = self.parser.data.def_nr(&format!("n_{name}"));
            if d == u32::MAX {
                None
            } else {
                let def = self.parser.data.def(d);
                let vars = &def.variables;
                (0..vars.count())
                    .find(|&i| vars.name(i) == "__t")
                    .map(|i| vars.tp(i).show(&self.parser.data, vars))
            }
        };
        self.rewind(sp);
        result
    }

    /// @PLN98 P1b — build `fn _(sig) -> ret_ty { seed (expr) }`, compile it, and
    /// evaluate it over the paused frame via [`State::eval_frame_reenter`], the
    /// keyed args (`arg_names`) bound to the paused frame's live `DbRef`s.  The
    /// synthetic def is **kept** (not rolled back): its bytecode was appended into
    /// the paused State, and a rollback would desync `fn_positions` from `data`
    /// (it stays an unused throwaway).  `None` when the wrapper fails to compile
    /// or the return type is unsupported.
    fn eval_frame_build_run(
        &mut self,
        sig: &str,
        seed: &str,
        expr: &str,
        ret_ty: &str,
        arg_names: &[String],
        json: bool,
    ) -> Option<String> {
        let next = self.counter + 1;
        let name = format!("replmain_{next}");
        let src = format!("fn {name}({sig}) -> {ret_ty} {{\n{seed}({expr})\n}}\n");
        let sp = self.savepoint();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        let failed = self.parser.diagnostics.entries()[pre_diag..]
            .iter()
            .any(|e| e.level >= Level::Error);
        if failed {
            self.rewind(sp);
            return None;
        }
        self.counter = next;
        crate::scopes::check(&mut self.parser.data, &mut self.parser.database);
        let d = self.parser.data.def_nr(&format!("n_{name}"));
        if d == u32::MAX {
            return None;
        }
        let ret_type = self.parser.data.def(d).returned.clone();
        let state = self.paused.as_deref_mut()?;
        state.eval_frame_reenter(&mut self.parser.data, d, arg_names, &ret_type, json)
    }

    /// Run one frame-eval attempt, catching a panic so the session survives — and
    /// **reporting** the panic's message instead of discarding it.
    ///
    /// The two eval attempts used to be `catch_unwind(…).unwrap_or(None)`, so a compiler
    /// invariant tripping inside them degraded to a bare "couldn't evaluate": exactly
    /// the swallowed-payload defect @PLN120 E.3 fixed for the abandon path, still live
    /// here.  It hid a real one — the @PLN120 E.4 rollback guard fires inside this call,
    /// and its message never reached the surface.  The report rides `trace_output`, so
    /// the interactive prompt prints it and the RPC surface emits it as an `output`
    /// event, from one place.
    fn eval_or_report_panic<F>(&mut self, attempt: F) -> Option<String>
    where
        F: FnOnce(&mut Self) -> Option<String>,
    {
        match std::panic::catch_unwind(AssertUnwindSafe(|| attempt(self))) {
            Ok(v) => v,
            Err(payload) => {
                let why = panic_message(&payload);
                self.trace_output
                    .push(format!("evaluating at the frame failed: {why}"));
                None
            }
        }
    }

    fn debug_eval_fmt(&mut self, expr: &str, json: bool) -> Option<String> {
        // @PLN16 D2 — a bare local that holds a heap value (struct / vector / collection)
        // is read **live, in place**: render its actual `DbRef` from the paused store
        // rather than reconstructing it from a rendered literal on a clone. That is both
        // correct (a bare `vector` faults when returned from the reconstruct-eval capture
        // fn) and *trustworthy* — it shows what is in the store, not a copy of it, which
        // matters when the value being inspected is the suspect in a store-lifetime bug.
        // A non-bare expression (a path, an index, any computed form) falls through to the
        // reconstruct path below, which handles scalars + struct-via-`.to_json()`.
        let trimmed = expr.trim();
        let is_bare_ident = trimmed
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            && trimmed.chars().all(|c| c.is_alphanumeric() || c == '_');
        if is_bare_ident
            && let Some(state) = self.paused.as_deref()
            && let Some(v) = state.eval_frame_heap(trimmed, json, &self.parser.data)
        {
            return Some(v);
        }
        // @PLN98 P1b — an expression that REFERENCES a keyed-collection local
        // (`hash`/`sorted`/`index`) can't be text-seeded (those render
        // non-reparseable), so evaluate it live over the paused frame instead of
        // through the reconstruct clone.  `None` when no keyed local is
        // referenced → the text-seed path below still handles everything else.
        // Guarded by the same `catch_unwind` as the reconstruct path so a codegen
        // fault in the live-frame compile can't abandon the debug session.
        let live = self.eval_or_report_panic(|s| s.eval_frame_expr(expr, json));
        if let Some(v) = live {
            return Some(v);
        }
        let prefix = {
            let state = self.paused.as_deref()?;
            let frame = state.paused_frame()?;
            // @PLN98 P1 — seed ONLY the locals the expression actually NAMES. Seeding every frame
            // local (as before) let ONE local whose captured literal is not loft source poison the
            // WHOLE reconstruct parse — the F1 bug: `2 + 2` returned null merely because the frame
            // HELD a vector, because that vector's compiler backing `__vdb_1` renders the un-reparseable
            // `main_vector<integer>{vector:[1,2,3]}`. Restricting to referenced identifiers means an
            // unrelated heap/keyed local can never break an expression that does not use it.
            let idents = expr_idents(expr);
            let mut p = String::new();
            for (name, lit) in frame.held_locals() {
                if !idents.contains(name.as_str()) {
                    continue;
                }
                p.push_str(name);
                // A `null` literal carries NO TYPE, so `ni = null;` does not compile and
                // the whole reconstruct fails — the expression degrades to "couldn't
                // evaluate" even when the question was only *what is `ni`* (loft#1459).
                // Annotate the seed so the absence has a type to be absent OF.  Only for
                // the null literal: every other seed already carries its type in its own
                // syntax, and annotating those would turn a rendering difference into a
                // compile error.
                if lit == "null"
                    && let Some(ty) = state.frame_local_source_type(name, &self.parser.data)
                {
                    p.push_str(": ");
                    p.push_str(&ty);
                }
                p.push_str(" = ");
                // Seed a HEAP local from the LIVE store, UNBOUNDED (the render path-A
                // `eval_frame_heap` trusts for a bare ident) rather than the captured BOUNDED display
                // literal (`[1,…,8,...]`), whose truncation tokens do not reparse for a large vector.
                // Scalars (heap read → None) fall back to the captured literal. A REFERENCED value
                // whose unbounded render is still not reparseable (keyed collections) stays the
                // live-frame eval follow-up (P1b) — but it no longer poisons unrelated expressions.
                match state.eval_frame_heap(name, false, &self.parser.data) {
                    Some(full) => p.push_str(&full),
                    None => p.push_str(lit),
                }
                p.push_str(";\n");
            }
            p
        };
        let saved = std::mem::replace(&mut self.body, prefix);
        let result = self.eval_or_report_panic(|s| s.value_of_fmt(expr, json));
        self.body = saved; // restore the real session body, panic or not
        result
    }

    /// Abandon a paused debug session, dropping the held state — used to recover the
    /// REPL after a debug operation panics.  The breakpoints stay set.
    pub fn abort_debug(&mut self) {
        self.paused = None;
    }

    /// Resolve + set the session's breakpoint specs on a freshly-compiled `state`
    /// (called by `compile_generation` before an observing run).  Specs are
    /// **function-scoped** — `<fn>:<line>` or `<fn>` (body start) — because that is
    /// the only form unique in the REPL: every input parses under the synthetic
    /// file `"<repl>"` with line numbers restarting at 1, so a bare or file:line
    /// number is not unique here (file:line is for a file-run debugger).  An
    /// unresolvable spec (unknown fn / no such line) is skipped this run, not error.
    fn apply_breakpoints(&mut self, state: &mut State) {
        let mut metas: Vec<(u32, BpMeta)> = Vec::new();
        for spec in &self.breakpoints {
            let offset = match &spec.location {
                BpLocation::Function(loc) => {
                    if let Some((name, line)) = loc.split_once(':') {
                        line.trim().parse::<u32>().ok().and_then(|l| {
                            let d = self.parser.data.def_nr(&format!("n_{}", name.trim()));
                            state.set_breakpoint_fn_line(d, l, &self.parser.data)
                        })
                    } else {
                        state.set_breakpoint_fn_start(loc, &self.parser.data)
                    }
                }
                BpLocation::File(file, line) => {
                    state.set_breakpoint_file_line(file, *line, &self.parser.data)
                }
            };
            // @PLN16 rich-bp — record offset → (condition, actions, stop) so the pause
            // resolver can honour a conditional break / tracepoint at this offset.
            if let Some(off) = offset {
                metas.push((
                    off,
                    BpMeta {
                        condition: spec.condition.clone(),
                        actions: spec.actions.clone(),
                        stop: spec.stop,
                    },
                ));
            }
        }
        self.bp_meta = metas.into_iter().collect();
    }

    /// Turn on session persistence: every later state-changing input appends to
    /// `path` (created if absent).  The interactive driver calls this after
    /// [`resume_from`](Self::resume_from); piped/test sessions never do, so they
    /// stay file-free and their output stays deterministic.
    ///
    /// # Errors
    /// Returns the I/O error if `path` cannot be opened for appending.
    pub fn enable_persistence(&mut self, path: &Path) -> std::io::Result<()> {
        self.record = Some(
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?,
        );
        Ok(())
    }

    /// Append one state-changing input to the session file, when persistence is
    /// on and we are not mid-resume.  Entries are NUL-separated: NUL never
    /// occurs in loft source, so a multi-line statement survives verbatim and
    /// splits back out unambiguously.  Best-effort — a write failure is dropped
    /// rather than allowed to break the live session.
    fn record_input(&mut self, input: &str) {
        if self.replaying {
            return;
        }
        if let Some(f) = self.record.as_mut() {
            let _ = f.write_all(input.as_bytes());
            let _ = f.write_all(b"\0");
            let _ = f.flush();
        }
    }

    /// Replay a saved session from `path` to rebuild state, before persistence
    /// is enabled.  Each NUL-separated entry is fed back through
    /// [`eval`](Self::eval) with recording suppressed; an entry that no longer
    /// parses (or panics) is skipped, never aborting the resume.  A missing file
    /// is an empty session.  Returns how many entries were restored vs skipped.
    pub fn resume_from(&mut self, path: &Path) -> ResumeStats {
        let mut stats = ResumeStats::default();
        let Ok(bytes) = std::fs::read(path) else {
            return stats; // no prior session
        };
        let text = String::from_utf8_lossy(&bytes);
        self.replaying = true;
        for entry in text.split('\0') {
            if entry.trim().is_empty() {
                continue;
            }
            // Same panic isolation as the live loop: a poison entry must not
            // abort the resume.  Resume replays only defs/bindings (neither
            // executes), so a panic here would be a parser fault, not runtime.
            match std::panic::catch_unwind(AssertUnwindSafe(|| self.eval(entry))) {
                Ok(Eval::Ran) => stats.restored += 1,
                _ => stats.skipped += 1, // Error / NeedMore / panic
            }
        }
        self.replaying = false;
        stats
    }

    /// Discard the saved session at `path` (the `:reset` command and the
    /// `--fresh` flag) so the next launch starts clean.  Best-effort.
    pub fn clear_session(path: &Path) {
        let _ = std::fs::remove_file(path);
    }

    /// Evaluate one input line/statement against the session.
    ///
    /// Returns [`Eval::NeedMore`] for incomplete input, [`Eval::Error`] (session
    /// unchanged) on a parse error, or [`Eval::Ran`] when the statement parsed
    /// and executed.
    ///
    /// A binding (`x = 1`) is recorded but not executed now — an unused
    /// binding's slot is elided by the allocator, so even compiling it would
    /// panic; its value is realised when a later input observes it.  Any other
    /// input is an *observing* statement: it is wrapped as `println("{<input>}")`
    /// so a bare expression's value is shown in loft's native rendering, with a
    /// fall back to running it plain when it is a void statement (`assert`,
    /// `print`) or otherwise can't be string-interpolated.
    ///
    /// # Panics
    /// Propagates a runtime panic from the executed program (e.g. a failed
    /// `assert`, arithmetic overflow, or the interpreter's infinite-loop guard).
    /// Because execution runs on a throwaway clone of the database, the session
    /// itself survives such a panic when the caller wraps this in `catch_unwind`.
    pub fn eval(&mut self, input: &str) -> Eval {
        if Parser::statement_incomplete(input) {
            return Eval::NeedMore;
        }
        if Parser::starts_top_level_def(input) {
            // A definition (struct/enum/fn/type/…): parse it as a top-level def
            // and keep it — it persists in `data` (parse_str appends, never
            // wipes prior defs) and is callable from later inputs.  Nothing to
            // print or execute.
            let sp = self.savepoint();
            let pre_diag = self.parser.diagnostics.entries().len();
            self.parser.parse_str(input, "<repl>", false);
            let produced: Vec<DiagEntry> = self.parser.diagnostics.entries()[pre_diag..].to_vec();
            if produced.iter().any(|e| e.level >= Level::Error) {
                self.rewind(sp);
                return Eval::Error(produced);
            }
            self.record_input(input); // a def changes session state — persist it
            return Eval::Ran;
        }
        if let Some(var) = Self::binding_name(input) {
            // REPL.X value-snapshot — run the RHS once, capture the value, and
            // store the binding as a literal (`name = 42`) so re-running `body`
            // on every later observe does NOT repeat a side effect.
            let rhs = input.split_once('=').map_or("", |(_, r)| r.trim());
            if !rhs.is_empty() {
                match self.capture_binding(rhs, false) {
                    Capture::Done(lit) => {
                        let snap = format!("{var} = {lit}");
                        let bound = format!("{}{snap};\n", self.body);
                        if self.compile_generation(&bound, false, false).is_ok() {
                            self.body = bound;
                            // @PLN14 arc A — file the shadow env entry under the
                            // bound name.  A re-bind (`n = n + 1`) replaces it; the
                            // old record is orphaned in the session store until
                            // arc G collects it.
                            if let Some(v) = self.pending_materialized.take() {
                                // arc G — a re-bind orphans the old record; release
                                // it so a long session does not grow per re-bind.
                                if let Some(old) = self.env.remove(&var) {
                                    self.free_session_value(&old);
                                }
                                self.env.insert(var.clone(), v);
                            }
                            self.record_input(&snap); // persist the snapshot, not the RHS
                            return Eval::Ran;
                        }
                        // (rare) the rendered literal didn't recompile — fall through.
                    }
                    // The RHS faulted while we snapshotted it: surface the error at
                    // the binding and store NOTHING — the effect ran once and no
                    // re-running source binding can poison later observes.
                    Capture::Failed(diags) => return Eval::Error(self.map_input_lines(diags)),
                    Capture::Skip => {} // not capturable — fall back to source
                }
            }
            // Fall back: record the binding as source (validate only).  It is
            // recompiled, in use, when a later input observes it.
            let bound = format!("{}{};\n", self.body, input);
            return match self.compile_generation(&bound, false, false) {
                Ok(()) => {
                    self.body = bound;
                    self.record_input(input);
                    Eval::Ran
                }
                Err(diags) => Eval::Error(self.map_input_lines(diags)),
            };
        }
        // Observing: show the value.  Bind it to a temp first, then print the
        // temp — `__replval = <input>; println("{__replval}")`.  Binding first
        // (rather than interpolating `<input>` into the format string directly)
        // means a text result echoes too (no nested-quote breakage), and works
        // uniformly for scalars, text, and structs.  If the input is a void
        // statement (`assert`, `print`) the temp binds to void and this fails to
        // compile — fall back to running the input plain so its side effects
        // still happen.
        // @PLN14 arc E — THE FLIP: a bare name with a store-resident value is
        // answered from the session store.  No generation is compiled and the
        // accumulated body does not re-run, so a side effect in any earlier
        // binding cannot repeat here and the observe cost stops growing with the
        // session.  Not gated on `stepping`: answering from the store runs no
        // code, so there is no breakpoint for stepping to catch — and the
        // interactive driver turns stepping ON by default, which would otherwise
        // disable the flip in exactly the session it is meant for.  Only an
        // active pause is excluded (those inputs belong to the paused sub-mode).
        if self.store_observe
            && self.paused.is_none()
            && let Some(name) = self.store_resident_name(input)
            && let Some(shown) = self.env_display(&name)
        {
            println!("{shown}");
            return Eval::Ran;
        }
        let shown = format!(
            "{}__replval = {input};\nprintln(\"{{__replval}}\");\n",
            self.body
        );
        if self.compile_generation(&shown, true, true).is_ok() {
            return if self.is_debugging() {
                Eval::Paused
            } else {
                Eval::Ran
            };
        }
        let plain = format!("{}{};\n", self.body, input);
        match self.compile_generation(&plain, true, true) {
            Ok(()) if self.is_debugging() => Eval::Paused,
            Ok(()) => Eval::Ran,
            Err(diags) => Eval::Error(self.map_input_lines(diags)),
        }
    }

    /// @PLN16 M5e — map diagnostics from a **wrapped** REPL generation back to the user's
    /// INPUT lines.  An expression / binding is evaluated inside `fn replmain_N() { <body>
    /// <input> }` (a synthetic header line + the replayed session `body` prepended), so the
    /// engine's diagnostic line is offset by that prefix; subtract it so an error points at
    /// the line the user actually typed.  Only the wrapped paths call this — a top-level
    /// **definition** is parsed directly (no wrapper), so its lines are already input-relative.
    /// (The *column* and a def-body line can still sit on the next token — a separate parser
    /// position issue, not this offset.)  `<repl>`-only and clamped, so an underflow leaves
    /// the line at 1 rather than wrapping.
    ///
    /// A diagnostic at or below `prefix` is about the wrapper header or the **replayed
    /// session body** — machinery the user did not type this turn — and the clamp above
    /// would park it on their line 1 as if they had.  That produced the FU.3 cascade: one
    /// typo answered with `Unknown function prnt` *and* `Variable x is never read` about
    /// an earlier, correct binding, at a column inside the failing line.  So a prelude
    /// **non-error is dropped**: the success path shows no warnings at all (this is the
    /// only route by which any warning reaches the user), so surfacing the replay's
    /// warnings only on failure was noise, and false noise at that — `x` *was* read, by
    /// the input that failed to compile.  A prelude **error is kept**, imprecise line and
    /// all: swallowing it would leave the REPL rejecting input while saying nothing, and a
    /// bad position beats silence.
    fn map_input_lines(&self, diags: Vec<DiagEntry>) -> Vec<DiagEntry> {
        let prefix = 1 + self.body.matches('\n').count() as u32;
        diags
            .into_iter()
            .filter(|d| d.file != "<repl>" || d.line > prefix || d.level >= Level::Error)
            .map(|mut d| {
                if d.file == "<repl>" {
                    d.line = d.line.saturating_sub(prefix).max(1);
                }
                d
            })
            .collect()
    }

    /// Parse one generation fn `fn replmain_N() { <gen_body> }`; when `execute`,
    /// run it.  On a parse error rolls `data` back and returns the diagnostics.
    /// When not executing (a binding), the generation's def is rolled back too —
    /// it lives on only as source in `body`.
    fn compile_generation(
        &mut self,
        gen_body: &str,
        execute: bool,
        debug: bool,
    ) -> Result<(), Vec<DiagEntry>> {
        let next = self.counter + 1;
        let name = format!("replmain_{next}");
        let src = format!("fn {name}() {{\n{gen_body}}}\n");
        let sp = self.savepoint();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        // Only this call's diagnostics — `Diagnostics::level` is monotonic.
        let produced: Vec<DiagEntry> = self.parser.diagnostics.entries()[pre_diag..].to_vec();
        if produced.iter().any(|e| e.level >= Level::Error) {
            // The lexer clears its diagnostics per parse_str, so this error does
            // not leak into the next input — the session stays usable after a typo.
            self.rewind(sp);
            return Err(produced);
        }
        self.counter = next;
        if execute {
            // scopes::check assigns slots + does lifetime analysis (the real
            // pipeline runs it between parse and byte_code; without it locals get
            // no slot and execution underflows).  A fresh State per input
            // sidesteps the @P381 CONST_STORE re-lock and isolates a runtime
            // panic to the throwaway clone.
            crate::scopes::check(&mut self.parser.data, &mut self.parser.database);
            let mut state = State::new(self.parser.database.clone());
            compile::byte_code(&mut state, &mut self.parser.data);
            self.wire_natives(&mut state);
            // @PLN16 G1 — apply the session's breakpoints to this run.  In stepping
            // mode a hit *suspends* execution; otherwise it records-and-continues.
            // Only on a real observing run (`debug`), not the value-render re-runs
            // (`:vars`, snapshot validation).
            if debug && !self.breakpoints.is_empty() {
                self.apply_breakpoints(&mut state);
                if self.stepping {
                    state.enable_stepping();
                }
            }
            self.trace_output.clear();
            state.execute_argv(&name, &self.parser.data, &[]);
            if debug && state.is_paused() {
                // Suspended at a breakpoint (interactive stepping): hold the live
                // state so the caller can inspect / edit / step it.  The gen def
                // stays in `data` — its bytecode is what the held state runs — and
                // the observing wrapper's `println` fires when the run is resumed
                // to completion.  Return early; the run is not yet finished.
                self.paused = Some(Box::new(state));
                // @PLN16 rich-bp — a conditional break may not really stop here, and a
                // tracepoint emits + continues; resolve before handing back to the caller
                // (which checks `is_debugging()` to decide Paused vs Ran).
                self.resolve_pause();
                return Ok(());
            }
            if debug {
                self.last_hits = state.debug_hits().to_vec();
                for hit in &self.last_hits {
                    let vars: Vec<String> = hit
                        .locals
                        .iter()
                        .map(|(n, v)| format!("{n} = {v}"))
                        .collect();
                    println!("⏸ break in {} | {}", hit.function, vars.join(", "));
                }
            }
            // A failed `assert`, `panic(…)`, or a fault-site opcode is captured
            // here (execution stops cleanly, no Rust panic) rather than thrown —
            // surface it as an error instead of silently swallowing it.  Roll the
            // throwaway gen back so the failed line leaves no def behind.
            if let Some(err) = state.database.runtime_error.take() {
                self.rewind(sp);
                return Err(vec![err.to_diag_entry()]);
            }
        } else {
            self.rewind(sp);
        }
        Ok(())
    }

    /// REPL.X value-snapshot — run a binding's RHS **once**, capture its value,
    /// and render it as an own-format loft literal so the binding can be stored
    /// as `name = <literal>` (side-effect-free on every later re-run of `body`).
    /// Returns a [`Capture`]: `Done(literal)` to store the snapshot, `Failed` if
    /// the RHS faulted (surface the error, store nothing), or `Skip` if it is not
    /// capturable (fall back to storing the RHS as source).
    ///
    /// Mechanism ([`capture_typed`](Self::capture_typed)): build
    /// `fn replmain_N() -> <T> { <body> <rhs> }` so the RHS is the trailing return
    /// expression, run it on a throwaway `State`, and read the value off the **stack
    /// top** — where `execute_at` reads its own return, so no new execution entry
    /// point.  Dispatch is on the value's exact [`Type`] (in [`render_capture`]) and
    /// covers **every** type: inline scalars / simple-enum rendered directly, and
    /// `DbRef`-backed heap values (struct, vector, struct-enum) rendered by
    /// `show_loft` on the returned `DbRef`.
    ///
    /// **Text is the exception (@P293):** reading a text return off the entry stack
    /// double-frees when the value borrows a local `String` the fn frees on teardown,
    /// so a text RHS is captured via a store-resident single-element `vector<text>`
    /// wrap (the heap path) and unwrapped — never returned bare.
    ///
    /// A binding whose RHS isn't a simple `<name> = <expr>`, or whose value's type
    /// name doesn't resolve, falls back to storing the RHS as source (re-run).
    fn capture_binding(&mut self, rhs: &str, json: bool) -> Capture {
        // `Type::show` is a debug form: it appends a dep-tracking list
        // (`vector<integer>["__vdb_1"]`) and wraps a struct as `ref(P)`.  The
        // cap-fn return type and the `show_loft` schema lookup both need the
        // loft-SOURCE name — so reduce to the base type name.
        let Some(ty_show) = self.infer_type(rhs) else {
            return Capture::Skip;
        };
        let ty = base_type_name(&ty_show);
        // @P293 — a **text** value can't be captured by returning it from a synthetic
        // entry fn and reading the `Str` off the stack: if the value borrows a local
        // `String` the fn frees on teardown (a bare var read, a `+` concat, an
        // interpolation, a `text[self]` borrow), the read sees freed memory and the
        // buffer double-frees.  A *call*-returned-owned or const-backed text happens
        // to survive, but the borrowed cases abort the process.  Wrap text in a
        // single-element vector instead: building the vector **copies** the bytes into
        // store-resident memory that outlives the teardown (a `DbRef`, captured by the
        // working heap path), then unwrap the `["…"]` back to the bare text literal.
        // The explicit `vector<text>` element type coerces a borrowed/work text
        // (`text["__work_N"]`, e.g. a concat) to a plain owned element.
        // The hazard @P293 names is the raw TEXT read, and a `text?` is read exactly like
        // a `text` (`Optional(τ)` shares τ's layout) — so the wrapper has to cover both
        // spellings or the nullable one reaches the very `Str`-off-the-stack read the
        // wrapper exists to prevent.  It was unreachable only while `render_capture`
        // declined every nullable outright (loft#1459).  The wrapper keeps the `?` on the
        // ELEMENT type: `vector<text>` would take the null through (N-Store)'s warning and
        // emit a diagnostic into the session for a value the user only asked to look at.
        if ty.strip_suffix('?').unwrap_or(&ty) == "text" {
            // A `text?` needs the same wrapper and a nullable ELEMENT, so the absence
            // survives the round trip (`vector<text>` would take it through (N-Store) and
            // emit a warning into the session for a value the user only asked to look at).
            // The vector is bound to a local first and the local returned: a `vector<τ?>`
            // LITERAL in return position is refused, while the same literal checked against
            // a declared local type is accepted (loft#1476).
            let (ret, body) = if ty.ends_with('?') {
                (
                    "vector<text?>",
                    format!("__cap_text: vector<text?> = [({rhs})];\n__cap_text"),
                )
            } else {
                ("vector<text>", format!("[({rhs})]"))
            };
            let out = match self.capture_typed(&body, ret, json) {
                Capture::Done(lit) => lit
                    .strip_prefix('[')
                    .and_then(|s| s.strip_suffix(']'))
                    .map_or(Capture::Skip, |inner| Capture::Done(inner.to_string())),
                other => other,
            };
            // @PLN14 arc C — the wrapper above materialized a `vector<text>`, but
            // the BINDING is a `text`.  Left as-is the env would report `["hi"]`
            // for `t = "hi"`, so retag it: the characters are already store-
            // resident inside that vector, and the read side unwraps the single
            // element back to a bare text literal.  (Capturing the text directly
            // instead is exactly what @P293 forbids — a borrowed text read off the
            // stack aborts the process.)
            if let Some(v) = self.pending_materialized.as_mut() {
                v.shape = SessionShape::TextInVector;
            }
            return out;
        }
        self.capture_typed(rhs, &ty, json)
    }

    /// Run `fn replmain_N() -> <ty> { <body> <rhs> }` once on a throwaway `State`
    /// and render its return value as an own-format literal — the execute-and-read
    /// half of [`capture_binding`].  `ty` is the loft-source return type, used both
    /// for the fn signature and (via [`render_capture`]) for the `show_loft` schema
    /// lookup.  `Skip` if the wrapper doesn't parse/compile, `Failed` if the RHS
    /// faulted (surface it — do not fall back to a re-running source binding), `Done`
    /// with the literal otherwise.
    fn capture_typed(&mut self, rhs: &str, ty: &str, json: bool) -> Capture {
        let next = self.counter + 1;
        let name = format!("replmain_{next}");
        let src = format!("fn {name}() -> {ty} {{\n{}{rhs}\n}}\n", self.body);
        let sp = self.savepoint();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        let failed = self.parser.diagnostics.entries()[pre_diag..]
            .iter()
            .any(|e| e.level >= Level::Error);
        if failed {
            self.rewind(sp);
            return Capture::Skip;
        }
        self.counter = next;
        // The value's exact type drives the stack read (the inferred type
        // *string* above is only for the fn signature + the schema lookup).
        let cap_d = self.parser.data.def_nr(&format!("n_{name}"));
        let ret_ty = self.parser.data.def(cap_d).returned.clone();
        // `ty` is the SOURCE spelling and it wrote the fn signature; the schema lookup
        // downstream asks a different question and must not be handed the same string.
        // `Type::name` is the schema spelling of the type the parser actually RESOLVED
        // (loft#1449's `name`-vs-`source_name` split), so a `?` the source wrote — or one
        // resolution dropped — cannot reach `Stores::name` as part of a key: `vector<text?>`
        // is not a registered type and the capture answered `None`, which the debugger
        // cannot tell from "this does not evaluate" (loft#1459).
        let schema_name = ret_ty.name(&self.parser.data);
        crate::scopes::check(&mut self.parser.data, &mut self.parser.database);
        let mut state = State::new(self.parser.database.clone());
        compile::byte_code(&mut state, &mut self.parser.data);
        state.keep_entry_return();
        state.execute_argv(&name, &self.parser.data, &[]);
        // The RHS just ran (its side effect happened once).  A fault here is a
        // real binding error — surface it, don't fall back to source (which would
        // re-run the fault on every later observe and poison the session).
        if let Some(err) = state.database.runtime_error.take() {
            self.rewind(sp);
            return Capture::Failed(vec![err.to_diag_entry()]);
        }
        let mut captured = None;
        let lit = render_capture(&mut state, &ret_ty, &schema_name, json, &mut captured);
        // @PLN14 arcs B + C — SHADOW WRITE: give the value its own home in the
        // session store, whatever its shape.  Nothing reads it yet (the replay
        // literal below is still the source of truth), so this cannot change
        // behaviour; it is the differential oracle Step 4's frame-seed gets
        // checked against.
        self.pending_materialized = None;
        match captured {
            Some(Captured::Heap(root)) => self.materialize_into_session(&mut state, root, ty),
            Some(Captured::Scalar(v)) => self.box_scalar_into_session(&mut state, &v, ty),
            None => {}
        }
        self.rewind(sp); // discard the throwaway cap gen
        lit.map_or(Capture::Skip, Capture::Done)
    }

    /// Discard a throwaway capture generation: both the definitions and the
    /// schema types its parse added (#618).  Always undo the two together —
    /// rolling back only the definitions strands schema names with no defs
    /// behind them, and the next capture needing the same name aborts.
    /// Mark the session state before a speculative parse, to be restored with
    /// [`rewind`](Self::rewind).  See [`Savepoint`] for why both halves travel
    /// together.
    fn savepoint(&self) -> Savepoint {
        Savepoint {
            defs: self.parser.data.definitions(),
            types: self.parser.database.types_len(),
            #[cfg(debug_assertions)]
            schema: self.parser.database.schema_fingerprint(),
        }
    }

    /// Discard everything parsed since `sp` — the definitions and the schema
    /// types registered for them.
    fn rewind(&mut self, sp: Savepoint) {
        if std::env::var_os("LOFT_TRACE_SCHEMA").is_some() {
            eprintln!(
                "[schema] rewind defs {}->{} types {}->{}",
                self.parser.data.definitions(),
                sp.defs,
                self.parser.database.types_len(),
                sp.types,
            );
        }
        self.parser.data.rollback_to(sp.defs);
        self.parser.database.rollback_types_to(sp.types);
        #[cfg(debug_assertions)]
        debug_assert_eq!(
            self.parser.database.schema_fingerprint(),
            sp.schema,
            "a rewound speculative parse must leave the schema exactly as it found it"
        );
    }

    /// @PLN14 arc B — copy `root` into the session store and park the resulting
    /// location in [`pending_materialized`](Self::pending_materialized) for
    /// [`eval`](Self::eval) to file under the bound name.
    ///
    /// The session store is adopted into this run's `State` only for the copy, then
    /// taken straight back out, so it survives the throwaway `State`.  The slot it
    /// lands at is not recorded — it differs run to run and a materialized value's
    /// interior references do not depend on it.
    fn materialize_into_session(
        &mut self,
        state: &mut State,
        root: crate::keys::DbRef,
        type_name: &str,
    ) {
        let tp = state.database.name(type_name);
        if tp == u16::MAX {
            return;
        }
        let slot = self.session_slot(state);
        let copy = state.database.materialize(&root, tp, slot);
        self.session_store = Some(state.database.take_store(slot));
        if copy.rec != 0 {
            self.pending_materialized = Some(SessionValue {
                type_name: type_name.to_string(),
                rec: copy.rec,
                pos: copy.pos,
                shape: SessionShape::Heap,
            });
        }
    }

    /// @PLN14 arc C — **scalars at rest**: box `v` into a 1-field record in the
    /// session store, so a scalar binding has a store-resident home exactly like a
    /// heap one.  That uniformity is the point: Step 4's frame-seed reads every
    /// prior name from the store along ONE path rather than branching on whether
    /// the value happened to be inline.
    ///
    /// The bytes are written raw (never via the display literal), so the value
    /// read back is bit-identical — the reason Q2 keeps own-format out of the
    /// session path.
    fn box_scalar_into_session(&mut self, state: &mut State, v: &ScalarValue, type_name: &str) {
        let slot = self.session_slot(state);
        let store = &mut state.database.allocations[slot as usize];
        // One header word + one payload word; `pos = 8` is the payload, matching
        // the record shape `Stores::claim` hands out.
        let rec = store.claim(2);
        match v {
            ScalarValue::Integer(n) => {
                store.set_int(rec, 8, *n);
            }
            ScalarValue::Float(f) => {
                store.set_float(rec, 8, *f);
            }
            ScalarValue::Single(f) => {
                store.set_single(rec, 8, *f);
            }
            ScalarValue::Boolean(b) => {
                store.set_int(rec, 8, i64::from(*b));
            }
            ScalarValue::Character(c) => {
                store.set_u32_raw(rec, 8, *c);
            }
            ScalarValue::SimpleEnum(d) => {
                store.set_int(rec, 8, i64::from(*d));
            }
            ScalarValue::Text(s) => {
                // The text's BYTES are copied into the session store (`set_str`
                // claims a record for them there), so the entry owns its
                // characters rather than pointing at the run's memory — the
                // bare-`Str` raw-pointer hazard cannot follow the value here.
                let idx = store.set_str(s);
                store.set_u32_raw(rec, 8, idx);
            }
        }
        self.session_store = Some(state.database.take_store(slot));
        self.pending_materialized = Some(SessionValue {
            type_name: type_name.to_string(),
            rec,
            pos: 8,
            shape: SessionShape::Scalar(v.kind()),
        });
    }

    /// @PLN14 arc G — release a session-store entry whose name has been re-bound.
    ///
    /// `n = n + 1` in a REPL replaces the env entry, and without this the old
    /// record stays claimed forever — a long session's store would grow with
    /// every re-bind. The record is owned SOLELY by the env (nothing else holds a
    /// ref into the session store), so freeing it on replace is safe.
    ///
    /// Nested heap the value owns (sub-records, in-store text) is released by
    /// `remove_claims` first, then the record itself; a boxed scalar owns nothing
    /// except a `Text`'s separate `set_str` record.
    fn free_session_value(&mut self, entry: &SessionValue) {
        let Some(store) = self.session_store.take() else {
            return;
        };
        let mut state = State::new(self.parser.database.clone());
        let slot = state.database.adopt_store(store);
        let db = crate::keys::DbRef {
            store_nr: slot,
            rec: entry.rec,
            pos: entry.pos,
        };
        match entry.shape {
            SessionShape::Heap | SessionShape::TextInVector => {
                let tp = state.database.name(&entry.type_name);
                if tp != u16::MAX {
                    state.database.remove_claims(&db, tp);
                }
            }
            SessionShape::Scalar(ScalarKind::Text) => {
                // The characters live in their own `set_str` record; the field
                // holds its index.
                let idx = state.database.store(&db).get_u32_raw(db.rec, db.pos);
                if idx != 0 {
                    state.database.store_mut(&db).delete(idx);
                }
            }
            SessionShape::Scalar(_) => {} // inline bytes, nothing nested
        }
        state.database.store_mut(&db).delete(db.rec);
        self.session_store = Some(state.database.take_store(slot));
    }

    /// @PLN14 arc A — the session store's slot in `state`, adopting the carried
    /// store or creating one on the session's first binding.  The caller must take
    /// it back out (`take_store`) before `state` is dropped.
    fn session_slot(&mut self, state: &mut State) -> u16 {
        match self.session_store.take() {
            Some(store) => state.database.adopt_store(store),
            None => state.database.database(SESSION_STORE_WORDS).store_nr,
        }
    }

    /// @PLN14 arc A — read a binding back **from the session store** (not from the
    /// replayed body), rendered own-format.  `None` when the name has no
    /// store-resident value (an unbound name, or a bind the snapshot path
    /// declined).
    ///
    /// This is the shadow's read side: Steps 2–3 use it purely as the differential
    /// oracle (`env value == replayed value`); Step 5 is where observing switches
    /// over to it and the body replay goes away.
    pub fn env_value(&mut self, name: &str) -> Option<String> {
        let entry = self.env.get(name)?.clone();
        let store = self.session_store.take()?;
        let mut state = State::new(self.parser.database.clone());
        let slot = state.database.adopt_store(store);
        let db = crate::keys::DbRef {
            store_nr: slot,
            rec: entry.rec,
            pos: entry.pos,
        };
        let tp = state.database.name(&entry.type_name);
        let out = Self::render_session_value(&mut state, &entry, &db, tp);
        self.session_store = Some(state.database.take_store(slot));
        out
    }

    /// Render one session-store entry in the own-format the replay literal uses —
    /// the two must agree character for character, since that equality is what the
    /// shadow is checked on.
    fn render_session_value(
        state: &mut State,
        entry: &SessionValue,
        db: &crate::keys::DbRef,
        tp: u16,
    ) -> Option<String> {
        let store = state.database.store(db);
        match entry.shape {
            SessionShape::Scalar(ScalarKind::Integer) => Some(store.get_int(db.rec, 8).to_string()),
            SessionShape::Scalar(ScalarKind::Float) => {
                Some(float_literal(store.get_float(db.rec, 8)))
            }
            SessionShape::Scalar(ScalarKind::Single) => {
                Some(format!("{}f", store.get_single(db.rec, 8)))
            }
            SessionShape::Scalar(ScalarKind::Boolean) => Some(
                if store.get_int(db.rec, 8) != 0 {
                    "true"
                } else {
                    "false"
                }
                .to_string(),
            ),
            SessionShape::Scalar(ScalarKind::Character) => {
                char::from_u32(store.get_u32_raw(db.rec, 8)).map(|c| format!("'{c}'"))
            }
            SessionShape::Scalar(ScalarKind::Text) => {
                let idx = store.get_u32_raw(db.rec, 8);
                Some(escape_loft_text(store.get_str(idx)))
            }
            SessionShape::Scalar(ScalarKind::SimpleEnum) => {
                let disc = u8::try_from(store.get_int(db.rec, 8)).unwrap_or(0);
                if crate::database::Stores::enum_is_null(disc) {
                    Some("null".to_string())
                } else if tp == u16::MAX {
                    None
                } else {
                    Some(format!(
                        "{}.{}",
                        entry.type_name,
                        state.database.enum_val(tp, disc)
                    ))
                }
            }
            SessionShape::Heap | SessionShape::TextInVector => {
                if tp == u16::MAX {
                    return None;
                }
                let mut s = String::new();
                state.database.show_loft(&mut s, db, tp);
                if entry.shape == SessionShape::TextInVector {
                    // Stored as the single-element `vector<text>` the @P293
                    // work-around builds; unwrap it back to the bare text literal,
                    // exactly as `capture_binding` unwraps the rendered one.
                    return s
                        .strip_prefix('[')
                        .and_then(|x| x.strip_suffix(']'))
                        .map(str::to_string);
                }
                Some(s)
            }
        }
    }

    /// @PLN14 arc D — **the frame-seed**: load prior names from the session store
    /// into their slots in the currently paused frame, and report the
    /// before/after differential per binding.
    ///
    /// This is the step where a prior-name reference *can* read from the store
    /// instead of from a replayed literal. It is deliberately NOT wired into the
    /// normal eval path yet: Step 4 proves the mechanism against the still-running
    /// replay, and Step 5 is where observing switches over and the replay goes
    /// away. Nothing calls this during an ordinary session, so it cannot change
    /// behaviour.
    ///
    /// Only locals that are BOTH in the env and live in the frame are seeded;
    /// anything else is left alone. Returns one entry per seeded binding, or an
    /// empty vector when not paused.
    ///
    /// # Panics
    /// Panics if the session store cannot be returned to the session (an
    /// unreachable adopt/take mismatch).
    pub fn seed_paused_frame(&mut self) -> Vec<SeedReport> {
        let Some(mut state) = self.paused.take() else {
            return Vec::new();
        };
        let Some(store) = self.session_store.take() else {
            self.paused = Some(state);
            return Vec::new();
        };
        let slot = state.database.adopt_store(store);
        let data = self.parser.data.clone();
        // What the REPLAY put in the slots, read through the same renderer the
        // debugger's variables panel uses.
        state.refresh_paused_frame(&data);
        let before = Self::render_frame(&state, &data);

        let mut seeded_names = Vec::new();
        // Two names can resolve to the SAME slot: the compiler coalesces the stack
        // slots of locals whose live ranges do not overlap (an assigned-but-never-
        // read local shares with the next one).  Seeding both would silently
        // clobber the first — so seed a slot once and skip the rest.  The skipped
        // name is simply absent from the report, which makes the situation visible
        // to the caller instead of producing a quietly wrong value.
        let mut written: std::collections::HashSet<(u32, u32)> = std::collections::HashSet::new();
        for (name, entry) in self.env.clone() {
            let Some((dest, tp, _is_arg)) = state.frame_slot_addr(&name, &data) else {
                continue; // not a local of this frame
            };
            if !written.insert((dest.rec, dest.pos)) {
                continue; // slot already seeded by another (coalesced) binding
            }
            let src = crate::keys::DbRef {
                store_nr: slot,
                rec: entry.rec,
                pos: entry.pos,
            };
            if Self::seed_one_slot(&mut state, &entry, &src, &dest, &tp) {
                seeded_names.push(name);
            }
        }

        state.refresh_paused_frame(&data);
        let after = Self::render_frame(&state, &data);
        self.session_store = Some(state.database.take_store(slot));
        self.paused = Some(state);

        seeded_names.sort();
        seeded_names
            .into_iter()
            .map(|name| {
                let pick = |m: &std::collections::HashMap<String, String>| {
                    m.get(&name)
                        .cloned()
                        .unwrap_or_else(|| "<unreadable>".into())
                };
                SeedReport {
                    replayed: pick(&before),
                    seeded: pick(&after),
                    name,
                }
            })
            .collect()
    }

    /// The paused frame's locals as a name → rendered-value map.
    ///
    /// A **heap** local is rendered through `eval_frame_heap`, which dereferences
    /// its `DbRef`; the captured frame's own rendering shows the slot's raw words
    /// for those (`P{x:3,y:12884901900}` — a `DbRef` read as fields), which would
    /// make the seed differential compare garbage to garbage.  Same precedence
    /// `frame_value_of` uses.
    fn render_frame(
        state: &State,
        data: &crate::data::Data,
    ) -> std::collections::HashMap<String, String> {
        let mut out: std::collections::HashMap<String, String> = state
            .paused_frame()
            .map(|h| h.locals.iter().cloned().collect())
            .unwrap_or_default();
        let names: Vec<String> = out.keys().cloned().collect();
        for name in names {
            if let Some(heap) = state.eval_frame_heap(&name, false, data) {
                out.insert(name, heap);
            }
        }
        out
    }

    /// Write one session-store value into the frame slot `dest` holds.
    ///
    /// A scalar is written raw (bit-exact, never through its literal). A heap
    /// value is **materialized out of the session store into the run's own heap**
    /// first — the same [`Stores::materialize`](crate::database::Stores::materialize)
    /// chokepoint the bind side uses, run in the other direction — so the frame
    /// gets its own copy and the session's master is never aliased into a slot the
    /// running statement could mutate.
    fn seed_one_slot(
        state: &mut State,
        entry: &SessionValue,
        src: &crate::keys::DbRef,
        dest: &crate::keys::DbRef,
        tp: &Type,
    ) -> bool {
        match entry.shape {
            SessionShape::Heap => {
                let type_id = state.database.name(&entry.type_name);
                if type_id == u16::MAX || !matches!(tp, Type::Reference(_, _) | Type::Vector(_, _))
                {
                    return false;
                }
                // A fresh store in the RUN's heap owns the frame's copy.
                let home = state.database.database(SESSION_STORE_WORDS).store_nr;
                let copy = state.database.materialize(src, type_id, home);
                *state
                    .database
                    .store_mut(dest)
                    .addr_mut::<crate::keys::DbRef>(dest.rec, dest.pos) = copy;
                true
            }
            SessionShape::Scalar(kind) => Self::seed_scalar_slot(state, src, dest, kind),
            // The wrapper is an implementation detail of the @P293 work-around, not
            // a shape a frame slot ever has; seeding it would need the unwrapped
            // text, which arc C stores inside the vector.  Left to Step 5.
            SessionShape::TextInVector => false,
        }
    }

    /// Raw slot write for a boxed scalar — bit-exact, no literal round-trip.
    fn seed_scalar_slot(
        state: &mut State,
        src: &crate::keys::DbRef,
        dest: &crate::keys::DbRef,
        kind: ScalarKind,
    ) -> bool {
        let store = state.database.store(src);
        match kind {
            ScalarKind::Integer => {
                let v = store.get_int(src.rec, src.pos);
                *state
                    .database
                    .store_mut(dest)
                    .addr_mut::<i64>(dest.rec, dest.pos) = v;
            }
            ScalarKind::Float => {
                let v = store.get_float(src.rec, src.pos);
                *state
                    .database
                    .store_mut(dest)
                    .addr_mut::<f64>(dest.rec, dest.pos) = v;
            }
            ScalarKind::Single => {
                let v = store.get_single(src.rec, src.pos);
                *state
                    .database
                    .store_mut(dest)
                    .addr_mut::<f32>(dest.rec, dest.pos) = v;
            }
            ScalarKind::Boolean => {
                let v = u8::from(store.get_int(src.rec, src.pos) != 0);
                *state
                    .database
                    .store_mut(dest)
                    .addr_mut::<u8>(dest.rec, dest.pos) = v;
            }
            ScalarKind::Character => {
                let v = store.get_u32_raw(src.rec, src.pos);
                *state
                    .database
                    .store_mut(dest)
                    .addr_mut::<u32>(dest.rec, dest.pos) = v;
            }
            ScalarKind::SimpleEnum => {
                let v = u8::try_from(store.get_int(src.rec, src.pos)).unwrap_or(0);
                *state
                    .database
                    .store_mut(dest)
                    .addr_mut::<u8>(dest.rec, dest.pos) = v;
            }
            // Text needs the owned-`String` / borrowed-`Str` distinction the edit
            // path makes (`set_frame_literal`); not part of Step 4's slice.
            ScalarKind::Text => return false,
        }
        true
    }

    /// @PLN14 arc E — turn store-backed observing on or off for this session.
    pub fn set_store_observe(&mut self, on: bool) {
        self.store_observe = on;
    }

    /// @PLN14 arc E — whether observing currently reads the session store.
    #[must_use]
    pub fn store_observe(&self) -> bool {
        self.store_observe
    }

    /// How many generations this session has compiled — i.e. how many times the
    /// accumulated body has been replayed.
    ///
    /// This is the instrument Step 5 is measured with: the flip's whole point is
    /// that observing a store-resident binding no longer advances this counter,
    /// which is a direct proof that the body did not re-run (rather than an
    /// indirect one via timing).
    #[must_use]
    pub fn generations(&self) -> u32 {
        self.counter
    }

    /// `input` as the name of a store-resident binding, if that is all it is.
    /// A bare identifier only — anything else is a real expression and still goes
    /// through a generation.
    fn store_resident_name(&self, input: &str) -> Option<String> {
        let name = input.trim();
        if name.is_empty() || !self.env.contains_key(name) {
            return None;
        }
        let mut cs = name.chars();
        let first = cs.next()?;
        if !(first.is_ascii_alphabetic() || first == '_')
            || !cs.all(|c| c.is_alphanumeric() || c == '_')
        {
            return None;
        }
        Some(name.to_string())
    }

    /// @PLN14 arc E — a binding's value read from the session store and rendered
    /// the way loft **displays** it (`hi`, `{x:7,y:9}`, `3`), which is what the
    /// echo and `:vars` print — as opposed to [`env_value`](Self::env_value)'s
    /// own-format literal (`"hi"`, `P{x:7,y:9}`, `3.0`), which is what `value_of`
    /// returns.  The two renderings genuinely differ, so the flip needs both or it
    /// would change what a session prints.
    pub fn env_display(&mut self, name: &str) -> Option<String> {
        let entry = self.env.get(name)?.clone();
        let store = self.session_store.take()?;
        let mut state = State::new(self.parser.database.clone());
        let slot = state.database.adopt_store(store);
        let db = crate::keys::DbRef {
            store_nr: slot,
            rec: entry.rec,
            pos: entry.pos,
        };
        let tp = state.database.name(&entry.type_name);
        let out = Self::render_session_display(&mut state, &entry, &db, tp);
        self.session_store = Some(state.database.take_store(slot));
        out
    }

    /// The display form of one session-store entry (see [`env_display`]).
    fn render_session_display(
        state: &mut State,
        entry: &SessionValue,
        db: &crate::keys::DbRef,
        tp: u16,
    ) -> Option<String> {
        let store = state.database.store(db);
        match entry.shape {
            SessionShape::Scalar(ScalarKind::Integer) => Some(store.get_int(db.rec, 8).to_string()),
            // Display drops the own-format decimal point: `3.0` shows as `3`.
            SessionShape::Scalar(ScalarKind::Float) => Some(store.get_float(db.rec, 8).to_string()),
            SessionShape::Scalar(ScalarKind::Single) => {
                Some(store.get_single(db.rec, 8).to_string())
            }
            SessionShape::Scalar(ScalarKind::Boolean) => Some(
                if store.get_int(db.rec, 8) != 0 {
                    "true"
                } else {
                    "false"
                }
                .to_string(),
            ),
            // Display is the bare character / bare text — no quotes.
            SessionShape::Scalar(ScalarKind::Character) => {
                char::from_u32(store.get_u32_raw(db.rec, 8)).map(|c| c.to_string())
            }
            SessionShape::Scalar(ScalarKind::Text) => {
                Some(store.get_str(store.get_u32_raw(db.rec, 8)).to_string())
            }
            // Display is the variant alone, without the enum's name.
            SessionShape::Scalar(ScalarKind::SimpleEnum) => {
                let disc = u8::try_from(store.get_int(db.rec, 8)).unwrap_or(0);
                if crate::database::Stores::enum_is_null(disc) {
                    Some("null".to_string())
                } else if tp == u16::MAX {
                    None
                } else {
                    Some(state.database.enum_val(tp, disc).to_string())
                }
            }
            // A `text` binding is physically a 1-element `vector<text>`, and the
            // native renderer QUOTES a vector's text elements (`["hi"]`), so
            // unwrapping it yields `"hi"` where the session displays `hi`.
            // Recovering the bare characters means either storing the raw text on
            // this path or reading the element through a vector accessor — neither
            // is Step 5's slice, so DECLINE and let the replay answer.  The
            // own-format read (`env_value`) is unaffected: quoted is correct there.
            SessionShape::TextInVector => None,
            SessionShape::Heap => {
                if tp == u16::MAX {
                    return None;
                }
                let mut out = String::new();
                state.database.show(&mut out, db, tp, false);
                Some(out)
            }
        }
    }

    /// @PLN14 arc F — the layout key this session's stored values depend on.
    ///
    /// `layout_algo_hash` folds the record sizes, field byte positions, narrow-int
    /// encodings, collection strides AND the host endianness of every type the env
    /// references.  If any of those move — a different loft build, a changed
    /// struct, a different-endian machine — the key changes and the image is
    /// refused.  `None` when a referenced type no longer resolves, which is itself
    /// a reason to refuse.
    fn env_layout_key(&self) -> Option<u64> {
        let mut roots: Vec<u16> = Vec::new();
        for entry in self.env.values() {
            let tp = self.parser.database.name(&entry.type_name);
            if tp == u16::MAX {
                // A scalar's type name is not a schema type; it carries no layout.
                if matches!(entry.shape, SessionShape::Scalar(_)) {
                    continue;
                }
                return None;
            }
            roots.push(tp);
        }
        roots.sort_unstable();
        roots.dedup();
        Some(self.parser.database.layout_algo_hash(&roots))
    }

    /// @PLN14 arc F — write the session's store-resident values to `path` as a
    /// resume image: a header, the layout key, the binding environment, and the
    /// session store's raw arena.
    ///
    /// Returns `false` (writing nothing) when there is nothing to save.
    ///
    /// # Errors
    /// Returns the I/O error if the image cannot be written.
    pub fn save_session_image(&self, path: &Path) -> std::io::Result<bool> {
        let (Some(store), Some(key)) = (self.session_store.as_ref(), self.env_layout_key()) else {
            return Ok(false);
        };
        if self.env.is_empty() {
            return Ok(false);
        }
        let mut out: Vec<u8> = Vec::new();
        out.extend_from_slice(SESSION_IMAGE_MAGIC);
        out.extend_from_slice(&SESSION_IMAGE_VERSION.to_le_bytes());
        out.extend_from_slice(&key.to_le_bytes());
        let mut names: Vec<&String> = self.env.keys().collect();
        names.sort(); // deterministic image bytes
        out.extend_from_slice(&(names.len() as u32).to_le_bytes());
        for name in names {
            let e = &self.env[name];
            for text in [name.as_str(), e.type_name.as_str()] {
                out.extend_from_slice(&(text.len() as u32).to_le_bytes());
                out.extend_from_slice(text.as_bytes());
            }
            out.extend_from_slice(&e.rec.to_le_bytes());
            out.extend_from_slice(&e.pos.to_le_bytes());
            let (tag, kind) = shape_tags(e.shape);
            out.push(tag);
            out.push(kind);
        }
        let bytes = store.raw_bytes();
        out.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
        out.extend_from_slice(bytes);
        std::fs::write(path, &out)?;
        Ok(true)
    }

    /// @PLN14 arc F — restore a [`save_session_image`](Self::save_session_image).
    ///
    /// **Fail-closed by construction.** Every way the image can be wrong — absent,
    /// truncated, wrong magic or version, a layout key that does not match this
    /// build's schema, or a store arena `Store::from_bytes` rejects — returns a
    /// [`ImageLoad`] refusal and leaves the session **untouched**, so the caller
    /// falls back to a fresh session (or to the shipped text-replay resume). The
    /// image is never partially applied and a mismatch never miscomputes.
    ///
    /// The caller must have replayed the session's **type definitions** first: the
    /// layout key is computed against the current schema, so an image referencing
    /// a struct this session has not defined is correctly refused.
    pub fn load_session_image(&mut self, path: &Path) -> ImageLoad {
        let Ok(bytes) = std::fs::read(path) else {
            return ImageLoad::Missing;
        };
        let Some((env, store_bytes, key)) = Self::decode_session_image(&bytes) else {
            return ImageLoad::Malformed;
        };
        // Build the candidate env FIRST so the layout key is computed over exactly
        // the types the image references, then compare against this build.
        let saved_env = std::mem::replace(&mut self.env, env);
        let matches = self.env_layout_key() == Some(key);
        if !matches {
            self.env = saved_env; // untouched — nothing was applied
            return ImageLoad::SchemaMismatch;
        }
        let Some(store) = crate::store::Store::from_bytes(&store_bytes) else {
            self.env = saved_env;
            return ImageLoad::Malformed;
        };
        self.session_store = Some(store);
        ImageLoad::Loaded
    }

    /// Parse an image into `(env, store bytes, layout key)`; `None` on anything
    /// malformed.  Pure decoding — it applies nothing.
    fn decode_session_image(
        bytes: &[u8],
    ) -> Option<(
        std::collections::HashMap<String, SessionValue>,
        Vec<u8>,
        u64,
    )> {
        let mut at = 0usize;
        let take = |at: &mut usize, n: usize| -> Option<&[u8]> {
            let end = at.checked_add(n)?;
            let out = bytes.get(*at..end)?;
            *at = end;
            Some(out)
        };
        if take(&mut at, SESSION_IMAGE_MAGIC.len())? != SESSION_IMAGE_MAGIC {
            return None;
        }
        let version = u32::from_le_bytes(take(&mut at, 4)?.try_into().ok()?);
        if version != SESSION_IMAGE_VERSION {
            return None;
        }
        let key = u64::from_le_bytes(take(&mut at, 8)?.try_into().ok()?);
        let count = u32::from_le_bytes(take(&mut at, 4)?.try_into().ok()?) as usize;
        let mut env = std::collections::HashMap::with_capacity(count);
        for _ in 0..count {
            let mut text = || -> Option<String> {
                let n = u32::from_le_bytes(take(&mut at, 4)?.try_into().ok()?) as usize;
                String::from_utf8(take(&mut at, n)?.to_vec()).ok()
            };
            let name = text()?;
            let type_name = text()?;
            let rec = u32::from_le_bytes(take(&mut at, 4)?.try_into().ok()?);
            let pos = u32::from_le_bytes(take(&mut at, 4)?.try_into().ok()?);
            let tag = *take(&mut at, 1)?.first()?;
            let kind = *take(&mut at, 1)?.first()?;
            let shape = shape_from_tags(tag, kind)?;
            env.insert(
                name,
                SessionValue {
                    type_name,
                    rec,
                    pos,
                    shape,
                },
            );
        }
        let len = u32::from_le_bytes(take(&mut at, 4)?.try_into().ok()?) as usize;
        let store_bytes = take(&mut at, len)?.to_vec();
        Some((env, store_bytes, key))
    }

    /// @PLN14 arc H — evaluate one line and **return** its rendered value instead
    /// of printing it: the in-process eval API for an embedder or a GUI
    /// (@PLN12's absorbed REPL.T tail).
    ///
    /// `Ok(Some(text))` for an expression that produced a value, `Ok(None)` for a
    /// statement that did not (a binding, a definition, a `print`), and `Err` with
    /// the diagnostics on a parse or runtime error. The session advances exactly
    /// as it would through [`eval`](Self::eval) — this is the same evaluation, with
    /// the value handed back rather than written to stdout.
    ///
    /// Nearly free at this point: the value is already store-resident and the
    /// renderers (`show_loft` / `render_capture`) already exist, so a bare name
    /// is answered from the session store without replaying the body at all.
    ///
    /// # Errors
    /// Returns the diagnostics when the input does not parse or the run faults.
    pub fn eval_value(&mut self, input: &str) -> Result<Option<String>, Vec<DiagEntry>> {
        if Parser::statement_incomplete(input) {
            return Ok(None);
        }
        // A bare name with a store-resident value: answer from the store, no
        // generation compiled (the arc-E flip, used here regardless of the
        // observe flag — this API has no stdout to keep byte-identical).
        if self.paused.is_none()
            && let Some(name) = self.store_resident_name(input)
            && let Some(v) = self.env_value(&name)
        {
            return Ok(Some(v));
        }
        // A binding / definition advances the session but yields no value.
        if Parser::starts_top_level_def(input) || Self::binding_name(input).is_some() {
            return match self.eval(input) {
                Eval::Error(diags) => Err(diags),
                _ => Ok(None),
            };
        }
        // Otherwise it is an expression: render it.
        Ok(self.value_of(input))
    }

    /// @PLN14 arc G — how many records the session store currently holds (`0`
    /// before the first binding).  Exposed so the re-bind growth guard can assert
    /// that orphaned records are actually released; the arena's byte size is
    /// pre-allocated and therefore says nothing.
    #[must_use]
    pub fn session_store_records(&self) -> usize {
        self.session_store
            .as_ref()
            .map_or(0, crate::store::Store::claims_count)
    }

    /// @PLN14 arc A — the names that currently have a store-resident value.
    #[must_use]
    pub fn env_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.env.keys().cloned().collect();
        names.sort();
        names
    }

    /// If `input` is a simple binding `<name> = <expr>` (not `==`/`+=`/…),
    /// return the bound name.  The caller echoes it as a trailing read so the
    /// variable keeps a stack slot in this and every later generation (an
    /// unused binding would otherwise have its slot elided).
    fn binding_name(input: &str) -> Option<String> {
        let t = input.trim_start();
        let name: String = t
            .chars()
            .take_while(|c| c.is_alphanumeric() || *c == '_')
            .collect();
        let first = name.chars().next()?;
        if !(first.is_ascii_alphabetic() || first == '_') {
            return None;
        }
        let rest = t[name.len()..].trim_start();
        let mut cs = rest.chars();
        // A single `=` (assignment), not `==` (comparison).
        if cs.next() == Some('=') && cs.next() != Some('=') {
            Some(name)
        } else {
            None
        }
    }

    /// Compile the current session and emit one introspection `section`
    /// (bytecode / Rust / slots / types) to stdout, optionally restricted to
    /// named functions — the engine behind the REPL's `:bytecode` / `:rust` /
    /// `:slots` commands (reuses phase 01's `introspect`).
    pub fn introspect(&mut self, section: Section, fn_filter: Vec<String>) {
        crate::scopes::check(&mut self.parser.data, &mut self.parser.database);
        let mut state = State::new(self.parser.database.clone());
        compile::byte_code(&mut state, &mut self.parser.data);
        let end_def = self.parser.data.definitions();
        let opts = Options {
            sections: vec![section],
            fn_filter,
            ..Options::new()
        };
        let _ = crate::introspect::emit_all(&mut self.parser.data, &mut state, end_def, &opts);
    }

    /// Print the user-defined functions entered this session (name + return
    /// type), to stdout — the `:fns` command.  Synthetic generation wrappers
    /// (`repl_*` / `replmain_*`) and stdlib functions are excluded.
    pub fn list_fns(&self) {
        let data = &self.parser.data;
        for d in 0..data.definitions() {
            let def = data.def(d);
            if def.def_type != DefType::Function
                || !def.name.starts_with("n_")
                || def.name.starts_with("n_repl")
                || def.position.file != "<repl>"
            {
                continue;
            }
            let user = def.name.strip_prefix("n_").unwrap_or(&def.name);
            let ret = def.returned.show(data, &def.variables);
            println!("{user} -> {ret}");
        }
    }

    /// The identifiers Tab completion offers (the `:`-commands are added by the
    /// completer itself): every global function (user + stdlib, operators
    /// excluded), every struct/enum/base-type name, and the variables bound this
    /// session.  Synthetic and operator definitions are filtered out by
    /// [`is_plain_ident`].  Sorted + deduped; recomputed by the interactive loop
    /// after each input.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn completion_names(&self) -> Vec<String> {
        let data = &self.parser.data;
        let mut names: Vec<String> = Vec::new();
        for d in 0..data.definitions() {
            let def = data.def(d);
            match def.def_type {
                DefType::Function => {
                    // Operators and methods (`t_…`) aren't called by bare name.
                    if def.is_operator() {
                        continue;
                    }
                    if let Some(n) = def.name.strip_prefix("n_")
                        && !n.starts_with("repl")
                        && is_plain_ident(n)
                    {
                        names.push(n.to_string());
                    }
                }
                DefType::Struct | DefType::Enum | DefType::Type if is_plain_ident(&def.name) => {
                    names.push(def.name.clone());
                }
                _ => {}
            }
        }
        names.extend(self.bound_var_names());
        names.sort();
        names.dedup();
        names
    }

    /// The full completion model for the current session: the bare-identifier
    /// [`completion_names`](Self::completion_names) plus dotted-access `members`.
    /// A receiver's members are what may follow `receiver.`:
    ///
    /// - a **variable** → its type's methods (each rendered `method(` so the
    ///   completion shows it is callable and leaves the cursor inside the call),
    ///   plus, if the type is a struct, its field names (bare — a field names a
    ///   value, not a call);
    /// - an **enum type** name → its variant names (`Color.Red`, bare).
    ///
    /// Rebuilt by the interactive loop after each input.  `&mut` because
    /// resolving each variable's type runs a throwaway, rolled-back
    /// type-inference probe (no execution).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn completion_model(&mut self) -> CompletionModel {
        let names = self.completion_names();
        // Resolve variable types first (a `&mut` probe) before borrowing the
        // schema immutably below.
        let var_types = self.infer_var_types();
        let mut members: HashMap<String, Vec<String>> = HashMap::new();
        let data = &self.parser.data;
        let db = &self.parser.database;
        // Enum *type* names → their variant names (for `Color.Red` qualified
        // access).  Struct *type* names get no members — a struct is built with
        // `Type{…}`, not dotted — so only enums are added here.
        for d in 0..data.definitions() {
            let def = data.def(d);
            if def.def_type == DefType::Enum && is_plain_ident(&def.name) {
                let tp = db.name(&def.name);
                if tp != u16::MAX
                    && let Parts::Enum(variants) = &db.types[tp as usize].parts
                {
                    let mut vs: Vec<String> = variants.iter().map(|(_, n)| n.clone()).collect();
                    vs.sort();
                    members.insert(def.name.clone(), vs);
                }
            }
        }
        // *Variables* → their type's methods (`method(`) plus, for a struct, its
        // fields (bare).  The type name comes from `Type::name` — the SAME name
        // methods register under (`t_<len><name>_<method>`), so the lookup
        // matches for structs, text, vectors, and every base type uniformly.
        for (var, ty) in &var_types {
            let tname = ty.name(data);
            let mut ms: Vec<String> = methods_for_type(data, &tname);
            let tp = db.name(&tname);
            if tp != u16::MAX
                && let Parts::Struct(fields) = &db.types[tp as usize].parts
            {
                ms.extend(fields.iter().map(|f| f.name.clone()));
            }
            if !ms.is_empty() {
                ms.sort();
                ms.dedup();
                members.insert(var.clone(), ms);
            }
        }
        CompletionModel { names, members }
    }

    /// Infer the static type of every bound variable in a *single* probe: build
    /// `fn replmain_N() { <body> __cmpl0 = v0; … }`, read each temp's inferred
    /// type from the function's variable table, and roll the probe back — one
    /// parse for all variables.  Returns `var → Type` (the type indices it holds
    /// point at definitions that predate, and so survive, the rollback); a
    /// variable whose type can't be read is omitted.  Compile-time only —
    /// nothing executes, so a side-effecting binding does not run here.
    #[cfg(not(target_arch = "wasm32"))]
    fn infer_var_types(&mut self) -> HashMap<String, Type> {
        use std::fmt::Write;
        let vars = self.bound_var_names();
        let mut out = HashMap::new();
        if vars.is_empty() {
            return out;
        }
        let name = format!("replmain_{}", self.counter + 1);
        let mut probe = self.body.clone();
        for (i, v) in vars.iter().enumerate() {
            let _ = writeln!(probe, "__cmpl{i} = {v};");
        }
        let src = format!("fn {name}() {{\n{probe}}}\n");
        let sp = self.savepoint();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        let failed = self.parser.diagnostics.entries()[pre_diag..]
            .iter()
            .any(|e| e.level >= Level::Error);
        if !failed {
            let d = self.parser.data.def_nr(&format!("n_{name}"));
            if d != u32::MAX {
                let def = self.parser.data.def(d);
                let v = &def.variables;
                for i in 0..v.count() {
                    if let Some(idx) = v
                        .name(i)
                        .strip_prefix("__cmpl")
                        .and_then(|s| s.parse::<usize>().ok())
                        && let Some(src_var) = vars.get(idx)
                    {
                        out.insert(src_var.clone(), v.tp(i).clone());
                    }
                }
            }
        }
        self.rewind(sp);
        out
    }

    /// The variables bound this session, each once in first-seen order (a rebind
    /// keeps the original position).  Derived from the binding lines accumulated
    /// in `body`.  Feeds both `:vars` and Tab completion.
    fn bound_var_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for line in self.body.lines() {
            if let Some(name) = Self::binding_name(line)
                && !names.contains(&name)
            {
                names.push(name);
            }
        }
        names
    }

    /// Print each session variable and its current value (`name = value`, in
    /// loft's native rendering) to stdout — the `:vars` command.  Returns
    /// `Ok(false)` when nothing is bound (the caller reports it) or `Ok(true)`
    /// once values are printed.  Realising the values re-runs the accumulated
    /// body, so a side effect in a binding's RHS repeats here too (REPL.X).
    ///
    /// # Errors
    /// Returns the diagnostics if realising a value raises a parse or runtime
    /// error.
    pub fn show_vars(&mut self) -> Result<bool, Vec<DiagEntry>> {
        use std::fmt::Write;
        let names = self.bound_var_names();
        if names.is_empty() {
            return Ok(false);
        }
        // @PLN14 arc E — answer from the session store when EVERY bound name has a
        // value there.  All-or-nothing on purpose: a name the snapshot path
        // declined has no store entry, and printing a partial list from one source
        // and the rest from another would be worse than replaying.
        if self.store_observe {
            let mut lines = Vec::with_capacity(names.len());
            for n in &names {
                let Some(v) = self.env_display(n) else {
                    lines.clear();
                    break;
                };
                lines.push(format!("{n} = {v}"));
            }
            if lines.len() == names.len() {
                for line in lines {
                    println!("{line}");
                }
                return Ok(true);
            }
        }
        // Append one `println("name = {name}")` per variable after the body, so
        // a single run renders every current value through the same path a bare
        // expression uses.
        let mut gen_src = self.body.clone();
        for n in &names {
            let _ = writeln!(gen_src, "println(\"{n} = {{{n}}}\");");
        }
        self.compile_generation(&gen_src, true, false)?;
        Ok(true)
    }

    /// Infer the static type of `expr` against the current session, without
    /// running it — the `:type` command.  Returns the rendered type, or `None`
    /// if it doesn't type-check.  Compile-time only: it binds `expr` to a temp
    /// in a throwaway generation, reads the temp's inferred type from the
    /// function's variable table, and rolls the probe back — no execution.
    pub fn infer_type(&mut self, expr: &str) -> Option<String> {
        let name = format!("replmain_{}", self.counter + 1);
        let probe = format!("{}__t = {expr};\n", self.body);
        let src = format!("fn {name}() {{\n{probe}}}\n");
        let sp = self.savepoint();
        let pre_diag = self.parser.diagnostics.entries().len();
        self.parser.parse_str(&src, "<repl>", false);
        let failed = self.parser.diagnostics.entries()[pre_diag..]
            .iter()
            .any(|e| e.level >= Level::Error);
        let result = if failed {
            None
        } else {
            let d = self.parser.data.def_nr(&format!("n_{name}"));
            if d == u32::MAX {
                None
            } else {
                let def = self.parser.data.def(d);
                let vars = &def.variables;
                let mut found = None;
                for i in 0..vars.count() {
                    if vars.name(i) == "__t" {
                        found = Some(vars.tp(i).show(&self.parser.data, vars));
                        break;
                    }
                }
                found
            }
        };
        self.rewind(sp); // discard the probe def
        result
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod completion_tests {
    use super::{CompletionModel, complete_word};
    use std::collections::HashMap;

    /// A model mirroring a session with globals `Point dbl print println x`; a
    /// struct variable `p` with a method `scale` and fields `{x, y}`; a text
    /// variable `s` with methods `{length, starts_with}`; and an enum type
    /// `Color{Red, Green, Blue}`.  `names` is pre-sorted, as `completion_names`
    /// returns it; each `members` list is sorted with methods rendered `name(`,
    /// as `completion_model` builds them.
    fn model() -> CompletionModel {
        let names = ["Point", "dbl", "print", "println", "x"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let members = HashMap::from([
            (
                "p".to_string(),
                vec!["scale(".to_string(), "x".to_string(), "y".to_string()],
            ),
            (
                "s".to_string(),
                vec!["length(".to_string(), "starts_with(".to_string()],
            ),
            (
                "Color".to_string(),
                vec!["Blue".to_string(), "Green".to_string(), "Red".to_string()],
            ),
        ]);
        CompletionModel { names, members }
    }

    #[test]
    fn completes_identifier_prefix() {
        let (start, out) = complete_word(&model(), "pr", 2);
        assert_eq!(start, 0);
        assert_eq!(out, vec!["print".to_string(), "println".to_string()]);
    }

    /// Only the word under the cursor is replaced, not the whole line.
    #[test]
    fn completes_word_mid_line() {
        let line = "1 + db";
        let (start, out) = complete_word(&model(), line, line.len());
        assert_eq!(start, 4); // "db" begins at byte 4
        assert_eq!(out, vec!["dbl".to_string()]);
    }

    #[test]
    fn completes_colon_command() {
        let (start, out) = complete_word(&model(), ":by", 3);
        assert_eq!(start, 1); // replace after the ':'
        assert_eq!(out, vec!["bytecode".to_string()]);
    }

    /// A `:command` with an argument falls through to identifier completion of
    /// the argument (e.g. `:rust db` completes a fn name).
    #[test]
    fn colon_command_arg_completes_identifier() {
        let line = ":rust db";
        let (start, out) = complete_word(&model(), line, line.len());
        assert_eq!(start, 6);
        assert_eq!(out, vec!["dbl".to_string()]);
    }

    /// A stray Tab with no prefix offers nothing (rather than every name).
    #[test]
    fn empty_prefix_yields_nothing() {
        let (_start, out) = complete_word(&model(), "1 + ", 4);
        assert!(out.is_empty());
    }

    // ── REPL.C member completion ─────────────────────────────────────────────

    /// `p.` with an empty prefix lists all of the struct variable's members —
    /// its methods (rendered `name(`) and its fields, together and sorted.
    #[test]
    fn dot_lists_all_struct_members() {
        let (start, out) = complete_word(&model(), "p.", 2);
        assert_eq!(start, 2); // replace after the dot
        assert_eq!(
            out,
            vec!["scale(".to_string(), "x".to_string(), "y".to_string()]
        );
    }

    /// A field prefix after the dot filters to the matching field.
    #[test]
    fn dot_filters_struct_fields_by_prefix() {
        let (start, out) = complete_word(&model(), "p.x", 3);
        assert_eq!(start, 2);
        assert_eq!(out, vec!["x".to_string()]);
    }

    /// A method completes with its trailing `(` so the user sees it is callable.
    #[test]
    fn dot_completes_method_with_paren() {
        let (start, out) = complete_word(&model(), "s.st", 4);
        assert_eq!(start, 2);
        assert_eq!(out, vec!["starts_with(".to_string()]);
    }

    /// A non-struct receiver (here a `text` variable) still completes its
    /// methods after the dot — `.` is never a dead end for a typed value.
    #[test]
    fn dot_lists_methods_for_non_struct() {
        let (start, out) = complete_word(&model(), "s.", 2);
        assert_eq!(start, 2);
        assert_eq!(out, vec!["length(".to_string(), "starts_with(".to_string()]);
    }

    /// An enum *type* name completes its variants after the dot.
    #[test]
    fn dot_completes_enum_variants() {
        let (start, out) = complete_word(&model(), "Color.G", 7);
        assert_eq!(start, 6);
        assert_eq!(out, vec!["Green".to_string()]);
    }

    /// Member completion works mid-line, replacing only the partial field.
    #[test]
    fn dot_completes_mid_line() {
        let line = "1 + p.x";
        let (start, out) = complete_word(&model(), line, line.len());
        assert_eq!(start, 6); // the `x` after `p.`
        assert_eq!(out, vec!["x".to_string()]);
    }

    /// A non-matching field prefix yields nothing — not a fall-through to globals.
    #[test]
    fn dot_unknown_field_yields_nothing() {
        let (_start, out) = complete_word(&model(), "p.zzz", 5);
        assert!(out.is_empty());
    }

    /// An unknown / non-struct receiver offers nothing, and never leaks the
    /// global list after a dot (`x` is a global but must not appear here).
    #[test]
    fn dot_unknown_receiver_yields_nothing() {
        let (_start, out) = complete_word(&model(), "foo.x", 5);
        assert!(out.is_empty(), "globals must not leak after a dot: {out:?}");
    }

    /// A non-identifier receiver (index / call result) needs live inference — the
    /// documented residual — so it yields nothing rather than guessing.
    #[test]
    fn dot_non_identifier_receiver_yields_nothing() {
        let (_start, out) = complete_word(&model(), "arr[0].x", 8);
        assert!(out.is_empty());
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod paused_prompt_tests {
    use super::{Eval, ReplSession, handle_paused, panic_message};

    /// @PLN120 E3a — the abandon path must SURFACE the panic payload.
    ///
    /// It used to print a fixed category line and drop the `Box<dyn Any>`, so every
    /// distinct runtime fault read identically and the debugger could not explain
    /// its own failure. Recovering the text is what named E3b's cause ("native
    /// function not loaded: its library's native cdylib is missing or stale") on
    /// the first run after this landed.
    #[test]
    fn a_caught_panic_keeps_its_message() {
        let from_literal =
            std::panic::catch_unwind(|| panic!("a literal cause")).expect_err("must unwind");
        assert_eq!(panic_message(&from_literal), "a literal cause");

        let n = 7;
        let from_format =
            std::panic::catch_unwind(|| panic!("a formatted cause: {n}")).expect_err("must unwind");
        assert_eq!(panic_message(&from_format), "a formatted cause: 7");

        // A payload with no readable text still says something, never nothing.
        let from_value =
            std::panic::catch_unwind(|| std::panic::panic_any(42u8)).expect_err("must unwind");
        assert!(!panic_message(&from_value).is_empty());
    }

    /// @PLN120 E3 — a bare verb must not shadow a live local of the same name.
    ///
    /// The paused prompt accepts verbs with or without the leading colon, and the
    /// verb set (`s` `n` `c` `r` `o` `u` `q`, `step`, `next`, `vars`, …) collides
    /// with the commonest loft local names there are.  Typing `n` to read the local
    /// `n` used to run `:next` — and typing `c` ran `:continue`, resuming the
    /// program and ending the session instead of printing a value.  A live local
    /// wins; the colon form is always the verb.
    #[test]
    fn a_bare_word_naming_a_live_local_is_read_not_run() {
        let mut s = ReplSession::new("default").expect("stdlib");
        assert!(matches!(
            s.eval("fn calc(n: integer) -> integer {\n  n * 10\n}"),
            Eval::Ran
        ));
        s.debug_stepping(true);
        s.add_breakpoint("calc");
        assert!(matches!(
            s.eval("assert(calc(5) == 50, \"runs\")"),
            Eval::Paused
        ));
        let at = s.paused_frame().expect("suspended in calc").line;
        assert!(
            s.paused_frame()
                .unwrap()
                .locals
                .iter()
                .any(|(n, _)| n == "n"),
            "the local `n` must be in the frame for this test to mean anything"
        );

        let mut out = Vec::new();
        assert!(!handle_paused("n", &mut s, &mut out).expect("io"));
        let frame = s
            .paused_frame()
            .expect("still suspended — bare `n` must not have stepped");
        assert_eq!(
            frame.line, at,
            "bare `n` ran the :next verb instead of reading the local"
        );

        // The colon form keeps meaning the verb even though `n` is a local.
        assert!(!handle_paused(":next", &mut s, &mut out).expect("io"));
        assert!(
            s.paused_frame().is_none_or(|f| f.line != at),
            ":next must still step"
        );
    }

    /// The control: a bare verb with NO matching local still runs the verb — the
    /// convenience the colon-less form exists for is preserved.
    #[test]
    fn a_bare_verb_with_no_such_local_still_runs() {
        let mut s = ReplSession::new("default").expect("stdlib");
        assert!(matches!(
            s.eval("fn calc(v: integer) -> integer {\n  v * 10\n}"),
            Eval::Ran
        ));
        s.debug_stepping(true);
        s.add_breakpoint("calc");
        assert!(matches!(
            s.eval("assert(calc(5) == 50, \"runs\")"),
            Eval::Paused
        ));
        let at = s.paused_frame().expect("suspended in calc").line;
        assert!(
            !s.paused_frame()
                .unwrap()
                .locals
                .iter()
                .any(|(n, _)| n == "n"),
            "no local named `n` here — that is the point of the control"
        );

        let mut out = Vec::new();
        assert!(!handle_paused("n", &mut s, &mut out).expect("io"));
        assert!(
            s.paused_frame().is_none_or(|f| f.line != at),
            "bare `n` must still step when nothing shadows it"
        );
    }
}
