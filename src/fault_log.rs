// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `LOFT_FAULT_LOG=<file>` — every store fault the runtime detects, as DATA: one JSON object
//! per line, appended, beside the human message the reporter prints anyway.
//!
//! A sweep, a test or a falsify run that asks "did this run fault, and how?" reads the kinds
//! from here instead of matching the reporters' prose, which each consumer used to do with a
//! pattern of its own — and a message that changed, or a fault reached through a different
//! reporter, then read as a clean run.  The C runtime writes the same lines (`loft_log.h`'s
//! `loft_fault_log`), so the three backends answer in one format.
//!
//! Each line: `{"kind": …, "backend": …, …}` with `kind` one of
//!
//! | kind | reported by |
//! |---|---|
//! | `use-after-free` | `LOFT_STRICT_STORES` (an access), the strict child-record check, `LOFT_UAF` |
//! | `leak` | the exit report and the per-test leak check (`LOFT_LEAK_CHECK`) |
//! | `re-free` | `LOFT_REFREE_FAILS`, the `LOFT_DOUBLE_FREE` census |
//! | `wrong-free` | a refused free (#306 the stack store, #405 out of range) |
//! | `corrupt-reference` | a store access out of the store's bounds |
//!
//! and further fields per kind (`store`, `freed`, `access`, `var`, `count`, `detail`; a
//! `corrupt-reference` on the interpreter carries the `pc` and `site` of the read).  A
//! consumer reads `kind` and treats the rest as detail; a reporter may add fields.

use std::io::Write;
use std::sync::{Mutex, OnceLock};

fn sink() -> Option<&'static Mutex<crate::file_access::File>> {
    static SINK: OnceLock<Option<Mutex<crate::file_access::File>>> = OnceLock::new();
    SINK.get_or_init(|| {
        let path = std::env::var("LOFT_FAULT_LOG")
            .ok()
            .filter(|p| !p.is_empty())?;
        crate::file_access::open_append(path.as_str())
            .ok()
            .map(Mutex::new)
    })
    .as_ref()
}

/// Is the fault log armed?  A reporter that has to compute a field asks first.
#[must_use]
pub fn armed() -> bool {
    sink().is_some()
}

/// The backend a fault is reported from: the generated Rust sets `native` at its start.
static BACKEND: OnceLock<&'static str> = OnceLock::new();

/// Name the backend this process reports for (`native`); the interpreter is the default.
pub fn set_backend(name: &'static str) {
    let _ = BACKEND.set(name);
}

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = std::fmt::Write::write_fmt(out, format_args!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Append one fault: `kind` and the `fields` as strings.  Does nothing unless armed.
pub fn record(kind: &str, fields: &[(&str, String)]) {
    let Some(sink) = sink() else {
        return;
    };
    let mut line = String::from("{\"kind\":");
    json_str(&mut line, kind);
    line.push_str(",\"backend\":");
    json_str(&mut line, BACKEND.get().copied().unwrap_or("interpret"));
    for (k, v) in fields {
        line.push(',');
        json_str(&mut line, k);
        line.push(':');
        json_str(&mut line, v);
    }
    line.push_str("}\n");
    if let Ok(mut f) = sink.lock() {
        let _ = f.write_all(line.as_bytes());
        let _ = f.flush();
    }
}

/// A `LOFT_UAF` report: a frame variable still reading a store that was just freed.
pub fn uaf_gen(store: u16, var: &str) {
    record(
        "use-after-free",
        &[
            ("store", store.to_string()),
            ("var", var.to_string()),
            ("detail", "LOFT_UAF".to_string()),
        ],
    );
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_field_is_escaped_as_json() {
        let mut out = String::new();
        super::json_str(&mut out, "C:\\dir\\\"f\".loft\nx\u{1}");
        // A Windows path's backslashes, a quote, a newline and a control character: each
        // escaped, so a consumer's JSON parser reads the field back as written.
        assert_eq!(out, "\"C:\\\\dir\\\\\\\"f\\\".loft\\nx\\u0001\"");
    }
}
