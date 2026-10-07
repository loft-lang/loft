// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I73 — Native function registry (the natives behind lib/process)

// @PLN179 strand 4b — the natives behind `lib/process`.
//! Running a program for a loft script, under the rule that **a stream is never left without
//! a reader**: the moment the child exists, one thread drains each of its output pipes and a
//! third feeds its input, so no pipe can fill while another is waited on — the deadlock a
//! hand-written `Popen(...).stdout.read()` walks into when the child talks on stderr.  It is
//! enforced here, once, in the binary, and no loft surface above it can undo it.
//!
//! The argv arrives as ONE text, each word length-prefixed (`3:git3:log`), because a word may
//! hold any byte a separator could be; [`decode_argv`] refuses a malformed one rather than
//! guessing.  The words were composed by `Command` (`lib/process`), whose holes can never
//! become syntax — nothing here splits or quotes.
//!
//! The library's other operations reach the binary through the same native: a text that
//! starts with `@<op>:` (never a valid argv, which starts with a length) names the operation,
//! and its words follow, encoded as an argv's are — see [`control`].  A started program
//! lives in a handle table here, as a [`Running`] of `platform::process`, which owns its
//! tree: a stop, a timeout and this program's end all reach everything it started.
use crate::database::Stores;
use crate::file_access::PathText;
use crate::keys::{DbRef, Str};
use crate::platform::process::{Program, Running, Spawn, host_spelling};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// The words of a length-prefixed argv, or `None` when it is not one.
fn decode_argv(enc: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut rest = enc;
    while !rest.is_empty() {
        let colon = rest.find(':')?;
        let n: usize = rest[..colon].parse().ok()?;
        let body = rest.get(colon + 1..colon + 1 + n)?;
        words.push(body.to_string());
        rest = &rest[colon + 1 + n..];
    }
    Some(words)
}

/// A finished run: the exit code, and both streams as they arrived.  The code is the
/// program's own, `128 + n` for a child ended by signal `n` (the shell's convention), and
/// `-1` when the program could not be started at all — `stderr` then says why.
pub(crate) struct Finished {
    pub code: i64,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// Run `argv` to completion with `input` on its stdin, collecting both streams.
pub(crate) fn run_collect(argv: &[String], input: &[u8]) -> Finished {
    run_until(argv, input, None)
}

/// [`run_collect`], stopped with its tree at `limit`: the code is then -2.
fn run_until(argv: &[String], input: &[u8], limit: Option<Duration>) -> Finished {
    let Some((program, rest)) = argv.split_first() else {
        return Finished {
            code: -1,
            stdout: Vec::new(),
            stderr: b"an empty command: there is no program to run".to_vec(),
        };
    };
    let spawn = Spawn::new(Program::search(program)).args(rest);
    let ran = match limit {
        Some(limit) => spawn.run_for(input, limit),
        None => spawn.run(input),
    };
    match ran {
        Ok(ran) => Finished {
            code: if ran.timed_out {
                -2
            } else {
                exit_code(ran.status)
            },
            stdout: ran.stdout,
            stderr: ran.stderr,
        },
        Err(e) => Finished {
            code: -1,
            stdout: Vec::new(),
            stderr: format!("{program}: {e}").into_bytes(),
        },
    }
}

/// The code a loft script reads for `status`: the program's own, or `128 + n` for signal `n`.
fn exit_code(status: std::process::ExitStatus) -> i64 {
    status
        .code()
        .map_or_else(|| signal_code(status), |c| own_code(i64::from(c)))
}

/// The program's own exit code, in the one form every platform can give it.  A Windows child
/// built on MSYS or Cygwin (`sh`, the Git for Windows tools) that a signal ended exits with
/// `n << 8`, a value no Unix exit code can take (those are 0..=255), so it reads as the
/// `128 + n` the same program answers on Unix: `sh -c 'kill -TERM $$'` is 143 on both, not
/// 3840 on one (formal/paths.md: one program answers the same on every platform).
fn own_code(c: i64) -> i64 {
    own_code_on(c, cfg!(windows))
}

fn own_code_on(c: i64, windows: bool) -> i64 {
    if windows && c.trailing_zeros() >= 8 && (1..=64).contains(&(c >> 8)) {
        128 + (c >> 8)
    } else {
        c
    }
}

#[cfg(unix)]
fn signal_code(s: std::process::ExitStatus) -> i64 {
    use std::os::unix::process::ExitStatusExt;
    s.signal().map_or(-1, |n| 128 + i64::from(n))
}

#[cfg(not(unix))]
fn signal_code(_: std::process::ExitStatus) -> i64 {
    -1
}

/// The programs `start()` left running, by the handle the library holds.
static STARTED: Mutex<Option<HashMap<i64, Running>>> = Mutex::new(None);
static NEXT_HANDLE: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(1);

fn started<T>(f: impl FnOnce(&mut HashMap<i64, Running>) -> T) -> T {
    let mut g = STARTED
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    f(g.get_or_insert_with(HashMap::new))
}

fn number(w: Option<&String>) -> Option<i64> {
    w?.parse().ok()
}

/// One of the library's other operations, `@<op>:<words>`; `None` for an argv.  Each
/// answers the integer the library reads, and `out`/`err` texts:
/// - `start` argv: a handle (> 0), or -1 and why;
/// - `run` ms argv: a run to completion stopped at `ms` (-2 when it was), streams collected;
/// - `wait` handle ms: 1 when it ended (`out` its code), 0 while it runs, -1 for no handle;
/// - `alive` handle: 1 or 0;
/// - `stop` handle: 1 and `out` its code, 0 for no handle;
/// - `path` text: 0 and `out` the host's spelling, or -1 and why the path is refused.
fn control(enc: &str, input: &str) -> Option<(i64, String, String)> {
    let (op, rest) = enc.strip_prefix('@')?.split_once(':')?;
    let Some(words) = decode_argv(rest) else {
        return Some((-1, String::new(), MALFORMED.to_string()));
    };
    let none = || (0, String::new(), String::new());
    Some(match op {
        "start" => start(&words),
        "run" => match words.split_first() {
            Some((ms, argv)) => {
                let limit = ms.parse().unwrap_or(0);
                let f = run_until(argv, input.as_bytes(), Some(Duration::from_millis(limit)));
                (
                    f.code,
                    String::from_utf8_lossy(&f.stdout).into_owned(),
                    String::from_utf8_lossy(&f.stderr).into_owned(),
                )
            }
            None => (-1, String::new(), MALFORMED.to_string()),
        },
        "wait" => {
            let (Some(h), Some(ms)) = (number(words.first()), number(words.get(1))) else {
                return Some((-1, String::new(), MALFORMED.to_string()));
            };
            // Out of the table while it is waited on, so another caller is not held up.
            let Some(mut p) = started(|t| t.remove(&h)) else {
                return Some((-1, String::new(), String::new()));
            };
            if p.wait(Duration::from_millis(u64::try_from(ms).unwrap_or(0))) {
                let code = p.finish().map_or(-1, exit_code);
                (1, code.to_string(), String::new())
            } else {
                started(|t| t.insert(h, p));
                none()
            }
        }
        "alive" => {
            let alive = number(words.first())
                .is_some_and(|h| started(|t| t.get_mut(&h).is_some_and(Running::alive)));
            (i64::from(alive), String::new(), String::new())
        }
        "stop" => match number(words.first()).and_then(|h| started(|t| t.remove(&h))) {
            Some(mut p) => {
                let code = p.stop_tree().map_or(-1, exit_code);
                (1, code.to_string(), String::new())
            }
            None => none(),
        },
        "path" => {
            let raw = words.first().map_or("", String::as_str);
            match PathText::program(raw).and_then(|p| host_spelling(&p).map_err(|e| e.to_string()))
            {
                Ok(os) => (0, os.to_string_lossy().into_owned(), String::new()),
                Err(why) => (-1, String::new(), format!("path `{raw}`: {why}")),
            }
        }
        _ => (-1, String::new(), MALFORMED.to_string()),
    })
}

const MALFORMED: &str =
    "process: a malformed argv reached the binary — the library and the binary disagree";

/// `start()`: the program runs beside this one, its input empty and its output this
/// program's own.
fn start(argv: &[String]) -> (i64, String, String) {
    let Some((program, rest)) = argv.split_first() else {
        return (
            -1,
            String::new(),
            "an empty command: there is no program to run".to_string(),
        );
    };
    match Spawn::new(Program::search(program))
        .args(rest)
        .stdin(std::process::Stdio::null())
        .start()
    {
        Ok(p) => {
            let h = NEXT_HANDLE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            started(|t| t.insert(h, p));
            (h, String::new(), String::new())
        }
        Err(e) => (-1, String::new(), format!("{program}: {e}")),
    }
}

/// The whole run, independent of which backend asked — so the interpreter and the compiled
/// call cannot answer differently.
fn answer(argv: &str, input: &str) -> (i64, String, String) {
    if let Some(r) = control(argv, input) {
        return r;
    }
    let Some(words) = decode_argv(argv) else {
        return (-1, String::new(), MALFORMED.to_string());
    };
    let f = run_collect(&words, input.as_bytes());
    (
        f.code,
        String::from_utf8_lossy(&f.stdout).into_owned(),
        String::from_utf8_lossy(&f.stderr).into_owned(),
    )
}

/// `process_run(argv, input, out, err) -> integer` — run to completion, both streams
/// collected.  Arguments pop in reverse; `out` and `err` are `&text` destinations.
pub fn n_process_run(stores: &mut Stores, stack: &mut DbRef) {
    let err = stores.get::<DbRef>(stack);
    let out = stores.get::<DbRef>(stack);
    let input = stores.get::<Str>(stack);
    let argv = stores.get::<Str>(stack);
    let (code, o, e) = answer(argv.str(), input.str());
    *stores.store_mut(&out).addr_mut::<String>(out.rec, out.pos) = o;
    *stores.store_mut(&err).addr_mut::<String>(err.rec, err.pos) = e;
    stores.put(stack, code);
}

/// The compiled backend's twin: `--native` resolves the runtime function by loft def name
/// through `CODEGEN_RUNTIME_FNS`, a `&text` out-parameter arriving as `&mut String`.
pub mod typed {
    use crate::database::Stores;
    use std::cell::UnsafeCell;

    /// See [`super::n_process_run`].
    pub fn n_process_run(
        _cell: &UnsafeCell<Stores>,
        argv: &str,
        input: &str,
        out: &mut String,
        err: &mut String,
    ) -> i64 {
        let (code, o, e) = super::answer(argv, input);
        *out = o;
        *err = e;
        code
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_signal_ended_msys_child_reads_as_128_plus_n_on_windows() {
        use super::own_code_on;
        assert_eq!(own_code_on(3840, true), 143, "SIGTERM: 15 << 8");
        assert_eq!(own_code_on(2304, true), 137, "SIGKILL: 9 << 8");
        assert_eq!(
            own_code_on(3, true),
            3,
            "an ordinary exit code is the program's own"
        );
        assert_eq!(own_code_on(0, true), 0);
        assert_eq!(own_code_on(65 << 8, true), 65 << 8, "no signal 65");
        assert_eq!(
            own_code_on(3840, false),
            3840,
            "Unix codes are never read this way"
        );
    }

    use super::*;

    #[test]
    fn an_argv_decodes_word_for_word_or_not_at_all() {
        assert_eq!(
            decode_argv("3:git3:log0:4:a:b "),
            Some(vec![
                "git".into(),
                "log".into(),
                String::new(),
                "a:b ".into()
            ])
        );
        assert_eq!(decode_argv(""), Some(vec![]));
        assert_eq!(decode_argv("3:gi"), None, "a word shorter than its length");
        assert_eq!(decode_argv("x:git"), None, "a length that is not a number");
        assert_eq!(decode_argv("3git"), None, "no separator");
    }
}
