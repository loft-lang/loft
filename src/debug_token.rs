// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I83 — Dev server (--serve / --rpc): who may drive a debug session

//! `@C172` — a debug session is driven only by whoever holds its run's TOKEN.
//!
//! A debug channel evaluates code and edits frames with the user's own rights, and it listens
//! on a loopback port that every process on the machine can reach, whatever account runs it.
//! So each process that opens one draws a token once, from the operating system's random
//! source, and answers only a connection whose opening request names it:
//! `ws://127.0.0.1:<port>/<path>?t=<token>` (`http://…/?t=<token>` for the `--serve` page).  A
//! browser sets the URL of a WebSocket but none of its headers, so the token rides the URL.
//!
//! The process says its token on its standard output, on the line that already gives its
//! port, which only the program that started it reads.  A process that starts another one to
//! debug (`--serve` launching a game, a game swapping in its next build) hands its own token on
//! in `LOFT_DEBUG_TOKEN`, so one session keeps one token.  A channel over standard input and
//! output (`--rpc`, the DAP adapter) needs none: only its parent holds the pipe.
//!
//! The C backend's engine keeps the same rules (`loft_eh_debug_token` in `src/c/loft_engine.h`).

use std::sync::OnceLock;

/// The bytes of randomness a token carries: 128 bits, written as 32 hexadecimal digits.
const TOKEN_BYTES: usize = 16;

/// This run's token, or `None` when none can be had: then no debug connection is answered.
///
/// `LOFT_DEBUG_TOKEN` when it holds at least 32 hexadecimal digits and nothing else (handed on
/// by the process that started this one), else 128 bits drawn from the operating system.  A
/// host whose random source fails gets no token, never a guessable one.
#[must_use]
pub fn token() -> Option<&'static str> {
    static TOKEN: OnceLock<Option<String>> = OnceLock::new();
    TOKEN
        .get_or_init(|| {
            if let Ok(given) = std::env::var("LOFT_DEBUG_TOKEN")
                && well_formed(&given)
            {
                return Some(given.to_ascii_lowercase());
            }
            drawn()
        })
        .as_deref()
}

/// A token handed on is at least 128 bits of hexadecimal digits, and only that.
fn well_formed(t: &str) -> bool {
    t.len() >= 2 * TOKEN_BYTES && t.len() <= 128 && t.bytes().all(|b| b.is_ascii_hexdigit())
}

#[cfg(not(target_arch = "wasm32"))]
fn drawn() -> Option<String> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::getrandom(&mut bytes).ok()?;
    Some(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// A wasm build opens no debug listener, so it draws nothing.
#[cfg(target_arch = "wasm32")]
fn drawn() -> Option<String> {
    None
}

/// Whether the request target of an opening request (`/ws?t=…`, `/?t=…`) names this run's
/// token.  Compared in a time that does not depend on how much of it matches, so the answer
/// says nothing about the token's digits.
#[must_use]
pub fn admits(target: &str) -> bool {
    let Some(want) = token() else {
        return false;
    };
    let Some(query) = target.split_once('?').map(|(_, q)| q) else {
        return false;
    };
    query
        .split('&')
        .filter_map(|kv| kv.strip_prefix("t="))
        .any(|got| same(got.as_bytes(), want.as_bytes()))
}

/// The request target named on the first line of an HTTP request head (`GET /ws?t=… HTTP/1.1`).
#[must_use]
pub fn request_target(head: &str) -> &str {
    head.lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .unwrap_or("")
}

/// The `?t=<token>` a client's URL carries, or the empty text when there is no token.
#[must_use]
pub fn query() -> String {
    token().map_or_else(String::new, |t| format!("?t={t}"))
}

fn same(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `@C172` — only the token itself admits: a missing query, a missing `t`, a prefix, one
    /// digit off and a longer value are each refused.
    #[test]
    fn only_the_runs_token_admits() {
        let t = token().expect("the host has a random source");
        assert_eq!(t.len(), 2 * TOKEN_BYTES);
        assert!(admits(&format!("/ws?t={t}")));
        assert!(admits(&format!("/?x=1&t={t}")));
        let mut off = t.to_string();
        let last = off.pop().expect("a digit");
        off.push(if last == '0' { '1' } else { '0' });
        for target in [
            "/ws".to_string(),
            "/ws?".to_string(),
            "/ws?u=1".to_string(),
            format!("/ws?t={}", &t[..31]),
            format!("/ws?t={off}"),
            format!("/ws?t={t}0"),
        ] {
            assert!(!admits(&target), "{target} must be refused");
        }
    }

    #[test]
    fn a_handed_on_token_is_hex_of_at_least_128_bits() {
        assert!(well_formed(&"a1".repeat(16)));
        assert!(!well_formed(&"a1".repeat(15)));
        assert!(!well_formed(&format!("{}g", "a1".repeat(16))));
        assert!(!well_formed(""));
    }

    #[test]
    fn the_request_target_is_the_first_lines_second_word() {
        assert_eq!(
            request_target("GET /ws?t=ab HTTP/1.1\r\nHost: x\r\n"),
            "/ws?t=ab"
        );
        assert_eq!(request_target(""), "");
    }
}
