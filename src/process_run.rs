// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

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
use crate::database::Stores;
use crate::keys::{DbRef, Str};
use std::io::{Read, Write};
use std::process::{Command, Stdio};

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

fn drain(mut pipe: impl Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = pipe.read_to_end(&mut out);
        out
    })
}

/// Run `argv` to completion with `input` on its stdin, collecting both streams.
pub(crate) fn run_collect(argv: &[String], input: &[u8]) -> Finished {
    let Some((program, rest)) = argv.split_first() else {
        return Finished {
            code: -1,
            stdout: Vec::new(),
            stderr: b"an empty command: there is no program to run".to_vec(),
        };
    };
    let spawned = Command::new(program)
        .args(rest)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(e) => {
            return Finished {
                code: -1,
                stdout: Vec::new(),
                stderr: format!("{program}: {e}").into_bytes(),
            };
        }
    };
    // Both readers exist before a byte is written, so a child that answers while it is still
    // being fed never blocks on a full pipe.
    let out = child.stdout.take().map(drain);
    let err = child.stderr.take().map(drain);
    let feed = child.stdin.take().map(|mut pipe| {
        let input = input.to_vec();
        // A child that exits without reading its input closes the pipe: that is its choice,
        // not a failure of the run, so the write's error is not reported.
        std::thread::spawn(move || {
            let _ = pipe.write_all(&input);
        })
    });
    let status = child.wait();
    if let Some(f) = feed {
        let _ = f.join();
    }
    let stdout = out.and_then(|h| h.join().ok()).unwrap_or_default();
    let mut stderr = err.and_then(|h| h.join().ok()).unwrap_or_default();
    let code = match status {
        Ok(s) => s.code().map_or_else(|| signal_code(s), i64::from),
        Err(e) => {
            stderr.extend_from_slice(format!("{program}: {e}").as_bytes());
            -1
        }
    };
    Finished {
        code,
        stdout,
        stderr,
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

/// The whole run, independent of which backend asked — so the interpreter and the compiled
/// call cannot answer differently.
fn answer(argv: &str, input: &str) -> (i64, String, String) {
    let Some(words) = decode_argv(argv) else {
        return (
            -1,
            String::new(),
            "process: a malformed argv reached the binary — the library and the binary disagree"
                .to_string(),
        );
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
    use super::*;

    #[test]
    fn an_argv_decodes_word_for_word_or_not_at_all() {
        assert_eq!(
            decode_argv("3:git3:log0:4:a:b "),
            Some(vec!["git".into(), "log".into(), String::new(), "a:b ".into()])
        );
        assert_eq!(decode_argv(""), Some(vec![]));
        assert_eq!(decode_argv("3:gi"), None, "a word shorter than its length");
        assert_eq!(decode_argv("x:git"), None, "a length that is not a number");
        assert_eq!(decode_argv("3git"), None, "no separator");
    }
}
