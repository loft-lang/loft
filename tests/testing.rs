// Copyright (c) 2022-2025 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

#![allow(dead_code)]

//! Testing framework
use loft::data::Context;
use loft::scopes;
extern crate loft;
#[path = "common/mod.rs"]
mod common;
use loft::compile::byte_code;
use loft::compile::show_code;
use loft::data::Data;
use loft::generation::Output;
use loft::log_config::LogConfig;
use std::fs::File;
use std::io::Write;

/// Normalise standard-library source locations in a diagnostic line: any
/// `*.loft:LINE:COL` collapses to `*.loft`, so tests do NOT hard-code (and break
/// on) `default/*.loft` line numbers when the stdlib shifts — e.g. when a
/// catalogue tag is inserted anywhere in a stdlib file.  User code is referenced
/// by test name, not a `.loft` path, so only stdlib references are affected.
/// Remove a leading `Level[code]:` tag, collapsing it to `Level:` (E1). Only a
/// `[...]` sitting between a known level word and the first `:` is stripped, so
/// brackets elsewhere in a message are untouched.
fn strip_diag_code(s: &str) -> String {
    // The shared stripper, not a local copy: this one omitted `Advice`, so a coded advice
    // kept its tag and no `@EXPECT_WARNING` prose could ever match it (@PLN131).
    loft::diagnostics::strip_compact_code(s)
}

fn normalize_loft_loc(s: &str) -> String {
    // @PLN102 arc-E E1 — a diagnostic may carry a stable `[code]` tag rendered
    // between the level and the colon (`Error[shift-amount-out-of-range]: …`).
    // The code is orthogonal to the (improvable) prose these assertions pin, so
    // collapse a leading `Level[...]:` back to `Level:` — expected strings never
    // carry a tag, so this only touches the found lines and keeps existing
    // `.error("<prose>")` assertions matching a now-coded diagnostic.
    let stripped = strip_diag_code(s);
    let mut out = String::with_capacity(stripped.len());
    let mut rest = stripped.as_str();
    while let Some(pos) = rest.find(".loft:") {
        out.push_str(&rest[..pos]);
        out.push_str(".loft");
        let tail = &rest[pos + ".loft:".len()..];
        let line_len = tail.chars().take_while(|c| c.is_ascii_digit()).count();
        let after_line = &tail[line_len..];
        if line_len > 0 && after_line.starts_with(':') {
            let col = &after_line[1..];
            let col_len = col.chars().take_while(|c| c.is_ascii_digit()).count();
            if col_len > 0 {
                rest = &col[col_len..]; // dropped ":LINE:COL"
                continue;
            }
        }
        out.push(':'); // a ".loft:" not followed by LINE:COL — keep it verbatim
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// Evaluate the given code.
/// When a result is given, there should be a present @test routine returning this result.
/// When a type is also given, this result should be of that type.
/// Defining an error will not expect a result but will validate that this specific error is thrown.
/// Defining warnings will just validate the given warnings and not expect a change of flow.
#[macro_export]
macro_rules! code {
    ($code:expr) => {
        testing::testing_code($code, stdext::function_name!())
    };
}

/// Directly evaluate a given expression.
/// This is shorthand for a test routine returning this expression.
#[macro_export]
macro_rules! expr {
    ($code:expr) => {
        testing::testing_expr($code, stdext::function_name!())
    };
}

use common::cached_default;
use loft::data::{IntegerSpec, Type, Value};
use loft::diagnostics::Level;
use loft::parser::Parser;
use loft::state::State;
use std::collections::BTreeSet;
use std::collections::HashMap;

// The test data for one test.
// Many parts can remain empty for each given test.
pub struct Test {
    name: String,
    file: String,
    expr: String,
    code: String,
    warnings: Vec<String>,
    advice: Vec<String>,
    errors: Vec<String>,
    fatal: Vec<String>,
    sizes: HashMap<String, u32>,
    result: Value,
    tp: Type,
    /// Compact slot-mapping spec checked after byte_code (debug builds only).
    /// Format: space-separated tokens of `name(scope)=slot`, e.g. `"_t(4L)=0 b(4L)=4"`.
    /// Scope suffix "L" means the scope is a loop scope; no suffix means a regular scope.
    expected_slots: Option<String>,
}

impl Test {
    /// Expect the parsing of the test to end in this error.
    /// Can be given multiple times to expect more than one error.
    pub fn error(&mut self, text: &str) -> &mut Test {
        if self.result != Value::Null {
            panic!("Cannot combine result with errors");
        }
        self.errors.push(text.to_string());
        self
    }

    pub fn fatal(&mut self, text: &str) -> &mut Test {
        if self.result != Value::Null {
            panic!("Cannot combine result with fatal");
        }
        self.fatal.push(text.to_string());
        self
    }

    /// Expect this warning during parsing.
    /// This will not change if it results in an error or a normal result.
    pub fn warning(&mut self, text: &str) -> &mut Test {
        self.warnings.push(text.to_string());
        self
    }

    /// Expect this ADVICE during parsing — a diagnostic on code that is correct as
    /// written (a deprecation, a cost, a preferred spelling).  Distinct from
    /// [`Test::warning`] because only the Warning tier gates a library's CI under
    /// `--deny-warnings`; asserting the tier here is what keeps that split honest.
    pub fn advice(&mut self, text: &str) -> &mut Test {
        self.advice.push(text.to_string());
        self
    }

    /// Shorthand expressions for a test routine that returns a result.
    pub fn expr(&mut self, value: &str) -> &mut Test {
        self.expr = value.to_string();
        self
    }

    /// Assert the stack-slot layout of `n_test` variables after codegen.
    ///
    /// `spec` is the multi-line visual layout the harness renders after
    /// `byte_code`, using `name(scope)+size=slot [first_def..last_use]`
    /// tokens with depth bars for nested scopes.  Calling `.slots("")`
    /// triggers the harness to panic with the computed layout so the
    /// spec can be copy-pasted back — this is the intended workflow for
    /// adding a new fixture.
    ///
    /// Locks the aligned (V2) slot layout so it will not drift silently;
    /// fires in every test profile (the rendering cost is negligible).  See
    /// `doc/claude/plans/finished/04-slot-assignment-redesign/`.
    pub fn slots(&mut self, spec: &str) -> &mut Test {
        self.expected_slots = Some(spec.to_string());
        self
    }

    /// Plan-04 Phase 2d: assert the slot layout satisfies invariants
    /// I1–I6 from
    /// `doc/claude/plans/finished/04-slot-assignment-redesign/SPEC.md § 5a`,
    /// without locking any specific numeric layout.
    ///
    /// Current implementation is a marker: `validate_slots` already
    /// runs at the end of codegen (`src/state/codegen.rs:156`) and
    /// panics with a distinct `[I1]` … `[I6]` prefix on any
    /// violation, so a V1 regression that breaks an invariant fails
    /// this test by construction.  Phase 2e will additionally run
    /// `assign_slots_v2` under `LOFT_SLOT_V2=validate` and invoke
    /// the same validator on its output.
    pub fn invariants_pass(&mut self) -> &mut Test {
        // No-op marker — actual checking happens in codegen's
        // validate_slots call.  Documenting intent at the fixture
        // site makes the test's purpose unambiguous.
        self
    }

    /// The expected result value. Cannot be combined with expected errors.
    pub fn result(&mut self, value: Value) -> &mut Test {
        if !self.errors.is_empty() {
            panic!("Cannot combine result with errors");
        }
        if matches!(value, Value::Boolean(_)) {
            self.tp = Type::Boolean;
        }
        self.result = value;
        self
    }

    /// In some cases the result type will different from its internal type.
    /// This is the case for Type::Boolean or Type::Enum types that return Value::Int(_) values.
    /// Also Value::None results can happen in combination with most other types.
    pub fn tp(&mut self, tp: Type) -> &mut Test {
        self.tp = tp;
        self
    }

    fn test(&self) -> String {
        let mut res = match &self.result {
            Value::Long(v) => v.to_string(),
            Value::Int(v) => v.to_string(),
            Value::Enum(v, _) => v.to_string(),
            Value::Boolean(v) if *v => "true".to_string(),
            Value::Boolean(_) => "false".to_string(),
            Value::Text(v) => replace_tokens(v),
            Value::Float(v) => v.to_string(),
            Value::Single(v) => v.to_string(),
            Value::Null if matches!(self.tp, Type::Text(_) | Type::Integer(_)) => {
                "null".to_string()
            }
            Value::Null if !matches!(self.tp, Type::Text(_)) => {
                return format!("pub fn test() {{\n    {};\n}}", self.expr);
            }
            _ => panic!("test {:?}", self.result),
        };
        let mut message = res.clone();
        if matches!(self.result, Value::Text(_)) {
            message = "\\\"".to_string() + &res + "\\\"";
            res = "\"".to_string() + &res + "\"";
        }
        format!(
            "pub fn test() {{\n    test_value = {{{}}};\n    assert(\n        test_value == {res},\n        \"Test failed {{test_value}} != {message}\"\n    );\n}}",
            self.expr
        )
    }

    fn output_code(
        &mut self,
        data: &mut Data,
        types: usize,
        code: &mut String,
        state: &mut State,
        config: &LogConfig,
    ) -> File {
        let _ = std::fs::create_dir_all("tests/dumps");
        let mut w = File::create(format!("tests/dumps/{}_{}.txt", self.file, self.name)).unwrap();
        writeln!(w, "{code}").unwrap();
        let to = state.database.types.len();
        for tp in types..to {
            writeln!(w, "Type {tp}:{}", state.database.show_type(tp as u16, true)).unwrap();
        }
        show_code(&mut w, state, data, config).unwrap();
        w
    }
}

fn replace_tokens(res: &str) -> String {
    // P233: escape `\` FIRST.  When the resulting text gets embedded
    // back into a loft string literal, loft's lexer interprets `\`
    // as an escape introducer (e.g. `\"` → `"`, `\\` → `\`), so a
    // lone `\` in the original `res` would otherwise let the lexer
    // re-interpret subsequent characters and either corrupt the
    // value or — for shapes with `\\"` after the later `"` → `\"`
    // step — close the string literal mid-scan and trip a hang in
    // the lexer's recovery path.  Doubling backslashes first turns
    // each original `\` into `\\` (which the lexer reads back as
    // exactly one `\`), making the round-trip lossless.
    //
    // Order matters: this MUST run before the `\n` and `"` steps
    // below, because both add fresh `\` characters that should NOT
    // themselves be re-escaped (those backslashes are part of the
    // canonical escape sequences `\n` and `\"`).
    res.replace('\\', "\\\\")
        .replace('{', "{{")
        .replace('}', "}}")
        .replace('\n', "\\n")
        .replace('"', "\\\"")
}

impl Drop for Test {
    // The actual evaluation of the test happens when the Test object is dropped.
    // So there is no need for an 'activate' method call.
    #[allow(unused_variables)]
    fn drop(&mut self) {
        let mut p = Parser::new();
        let (data, db) = cached_default();
        p.data = data;
        p.database = db;
        let types = p.database.types.len();
        let start = p.data.definitions();
        let mut code = self.code.clone();
        if !self.expr.is_empty() {
            if !code.is_empty() {
                code += "\n\n";
            }
            code += &self.test();
        }
        // emit-repro: dump the assembled test source to /tmp/loft-repro/
        // so a failing test can be replayed standalone via
        //   target/release/loft /tmp/loft-repro/<test>.loft
        // Emitted BEFORE parse/execute so a panic still leaves the file.
        #[cfg(feature = "emit-repro")]
        {
            let _ = std::fs::create_dir_all("/tmp/loft-repro");
            let path = format!("/tmp/loft-repro/{}.loft", self.name);
            let header = format!(
                "// Auto-generated reproducer for tests/{}.rs::{}\n\
                 // Source: Test::drop writes this under `--features emit-repro`.\n\
                 // Run:    target/release/loft {path}\n\n",
                self.file, self.name,
            );
            // Tests expect a `pub fn test()` entry point; `loft` runs `fn main`.
            // Append a thin `main` that calls `test` so the file is directly
            // runnable.  Skip when the body is a parse-error fixture (no
            // runnable shape) or already defines `fn main`.
            let needs_main = !code.contains("fn main(") && !code.contains("fn main ");
            let tail = if needs_main {
                "\n\nfn main() {\n    test();\n}\n"
            } else {
                "\n"
            };
            let _ = std::fs::write(&path, header + &code + tail);
        }
        p.parse_str(&code, &self.name, false);
        for (d, s) in &self.sizes {
            let size = p.database.size(p.data.def(p.data.def_nr(d)).known_type);
            assert_eq!(u32::from(size), *s, "Size of {}", *d);
        }
        // Validate that we found the correct warnings and errors. Halt when
        // differences are found.  Running this BEFORE scopes::check matches
        // `src/main.rs` order and prevents malformed-IR scope-analysis
        // panics from masking the real parse-time diagnostic (P140 class).
        self.assert_diagnostics(&p);
        // Do not run scope analysis / codegen when parsing did not succeed.
        if p.diagnostics.level() >= Level::Error {
            return;
        }
        scopes::check(&mut p.data, &mut p.database);
        // P132: also generate per-test native code in release builds so the
        // n2..n10/o7 codegen-inspection tests can read tests/generated/<name>.rs
        // when the suite is run via `cargo test --release` (the new default
        // since make ci switched to release for ~1800x speedup).
        self.generate_code(&p, start).unwrap();
        // generate_code (fill.rs) and generate_lib (text.rs) are now done
        // via dedicated staleness-check tests to avoid file-write races
        // during parallel test execution.  Per-test native codegen output
        // still goes to tests/generated/<test>.rs below.
        let mut state = State::new(p.database);
        let codegen_panic = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            byte_code(&mut state, &mut p.data);
        }))
        .err();
        if let Some(spec) = self.expected_slots.as_ref() {
            let test_nr = p.data.def_nr("n_test");
            let f = &p.data.def(test_nr).variables;
            // Build the full calculated layout, sorted by slot, for diff output.
            let mut all: Vec<(u16, u16, u16)> = (0..f.next_var())
                .filter(|&v| f.stack(v) != u16::MAX)
                .map(|v| (f.stack(v), f.scope(v), v))
                .collect();
            all.sort_by_key(|&(slot, _, _)| slot);
            // Scope ranks: sorted unique non-arg scope numbers → depth 0, 1, 2, ...
            let scope_ranks: std::collections::HashMap<u16, usize> = {
                let unique: std::collections::BTreeSet<u16> = all
                    .iter()
                    .filter(|&&(_, _, v)| !f.is_argument(v))
                    .map(|&(_, scope, _)| scope)
                    .filter(|&s| s != u16::MAX)
                    .collect();
                unique
                    .into_iter()
                    .enumerate()
                    .map(|(i, s)| (s, i))
                    .collect()
            };
            // Build the calculated visual: a scope-header line on first entry into each scope,
            // followed by variable lines with depth bars but no per-line scope label.
            let mut seen_scopes: std::collections::HashSet<u16> = std::collections::HashSet::new();
            let mut lines: Vec<String> = Vec::new();
            let mut full_tokens: Vec<String> = Vec::new();
            for &(slot, scope, v) in &all {
                let is_arg = f.is_argument(v);
                let ctx = if is_arg {
                    Context::Argument
                } else {
                    Context::Variable
                };
                let sz = f.size(v, &ctx);
                let scope_str = if is_arg {
                    "arg".to_string()
                } else if scope == u16::MAX {
                    "-".to_string()
                } else if f.is_loop_scope(scope) {
                    format!("{scope}L")
                } else {
                    scope.to_string()
                };
                let origin: &str = if is_arg || scope == u16::MAX {
                    ""
                } else {
                    f.scope_origin(scope)
                };
                let rank = if is_arg {
                    0
                } else {
                    scope_ranks.get(&scope).copied().unwrap_or(0)
                };
                let bars = "│ ".repeat(rank);
                let scope_key = if is_arg { u16::MAX } else { scope };
                if seen_scopes.insert(scope_key) {
                    let seq_range = if !is_arg && scope != u16::MAX {
                        if let Some((s, e)) = f.loop_seq_range(scope) {
                            format!(" [seq {s}..{e}]")
                        } else {
                            String::new()
                        }
                    } else {
                        String::new()
                    };
                    let header = if origin.is_empty() {
                        format!("  {bars}{scope_str}{seq_range}")
                    } else {
                        format!("  {bars}{origin}:{scope_str}{seq_range}")
                    };
                    lines.push(header);
                }
                let interval = if is_arg {
                    String::new()
                } else {
                    let fd = f.first_def(v);
                    let lu = f.last_use(v);
                    if fd == u32::MAX {
                        " [never]".to_string()
                    } else {
                        format!(" [{fd}..{lu}]")
                    }
                };
                lines.push(format!("  {bars}{}+{sz}={slot}{interval}", f.name(v)));
                full_tokens.push(format!("{}({scope_str})+{sz}={slot}", f.name(v)));
            }
            let calculated = lines.join("\n");
            // Compact single-line spec for copy-pasting slot values.
            let spec_line = full_tokens.join("  ");
            if spec.is_empty() {
                panic!(
                    "slots not asserted; calculated:\n{calculated}\n\n  .slots(\"{spec_line}\")"
                );
            }
            if spec.trim() != calculated.trim() {
                panic!(
                    "slots mismatch:\n  asserted:\n{}\n  calculated:\n{calculated}\n\n  .slots(\"{spec_line}\")",
                    spec.lines()
                        .map(|l| format!("    {l}"))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
            }
        }
        let config = LogConfig::from_env();
        let log_active = std::env::var("LOFT_LOG").is_ok();
        #[cfg(debug_assertions)]
        let log_active = true;
        if log_active {
            let mut w = self.output_code(&mut p.data, types, &mut code, &mut state, &config);
            if let Some(panic_payload) = codegen_panic {
                // Dump was written — now re-panic with the original message.
                std::panic::resume_unwind(panic_payload);
            }
            state.execute_log(&mut w, "test", &config, &p.data).unwrap();
        } else if let Some(panic_payload) = codegen_panic {
            std::panic::resume_unwind(panic_payload);
        } else {
            state.execute("test", &p.data);
        }
        // Plan-07 phase 4 — typed runtime errors halt execution
        // gracefully via `database.runtime_error`; the in-process test
        // harness re-raises them as a Rust panic so `#[should_panic]`
        // fixtures (failed assert / `panic("…")` builtin / future
        // `RuntimeError` kinds) keep firing.  The binary path
        // (`src/main.rs`) renders pretty + exits 1 instead.
        if let Some(err) = state.database.runtime_error.take() {
            panic!("{}", err.message);
        }
    }
}

impl Test {
    fn generate_code(&self, p: &Parser, start: u32) -> std::io::Result<()> {
        std::fs::create_dir_all("tests/generated")?;
        let w = &mut File::create("tests/generated/default.rs")?;
        let mut o = Output::new(&p.data, &p.database);
        o.output_native(w, 0, start)?;
        // Write code output when the result is tested, not only for errors or warnings.
        if self.result != Value::Null || !self.tp.is_unknown() {
            let w = &mut File::create(format!("tests/generated/{}_{}.rs", self.file, self.name))?;
            let def_nr = p.data.definitions();
            // Find the entry function n_test and emit only reachable functions.
            let test_fn = p.data.def_nr("n_test");
            if test_fn != u32::MAX {
                o.output_native_reachable(w, start, def_nr, &[test_fn])?;
            } else {
                o.output_native(w, start, def_nr)?;
            }
            writeln!(w, "#[test]\nfn code_{}() {{", self.name)?;
            writeln!(w, "    let mut stores = Stores::new();")?;
            writeln!(w, "    init(&mut stores);")?;
            writeln!(w, "    n_test(&mut stores);")?;
            writeln!(w, "}}")?;
        }
        Ok(())
    }

    fn assert_diagnostics(&self, p: &Parser) {
        let mut expected = BTreeSet::new();
        for w in &self.warnings {
            expected.insert(normalize_loft_loc(&format!("Warning: {w}")));
        }
        for w in &self.advice {
            expected.insert(normalize_loft_loc(&format!("Advice: {w}")));
        }
        for w in &self.errors {
            expected.insert(normalize_loft_loc(&format!("Error: {w}")));
        }
        for w in &self.fatal {
            expected.insert(normalize_loft_loc(&format!("Fatal: {w}")));
        }
        let mut found = "".to_string();
        for l in &p.diagnostics.lines() {
            if l.starts_with("Debug: ") {
                continue; // Debug-level diagnostics are not surfaced in tests
            }
            // Stdlib source locations are location-agnostic in assertions
            // (@PLN92): match the normalized form used to build `expected`.
            let l = normalize_loft_loc(l);
            // @PLN102 arc-E test-hygiene (2026-07-19) — the tolerated-warning filter is
            // GONE: the `code!` harness now asserts EXACTLY what loft emits, with NO
            // silent absorption. A fixture that emits a warning must `.warning(..)`-assert
            // it (or not emit it). The retired families (÷/%/`v[i]`/`s[i]`/`not null`-hint
            // "may produce null", the N-Store nudge — all DN1-dead) and the corrected ones
            // (redundant-`&` dropped, `not null` deleted) are logged in
            // test-hygiene-buckets.md; end-to-end warning coverage lives in
            // `tests/runtime_warnings.rs`.
            if expected.contains(&l) {
                expected.remove(&l);
            } else {
                if !found.is_empty() {
                    found += "|";
                }
                found += &l;
            }
        }
        let mut was = "".to_string();
        for e in expected {
            if !was.is_empty() {
                was += "|";
            }
            was += &e;
        }
        if !found.is_empty() || !was.is_empty() {
            panic!("Found '{found}' Expected '{was}'");
        }
    }

    // Try to decipher the correct return type from value() and tp() data.
    fn return_type(&self) -> &str {
        let tp = if self.tp.is_unknown() {
            if let Value::Int(_) = self.result {
                Type::Integer(IntegerSpec {
                    min: i32::MIN,
                    max: i64::from(i32::MAX),
                    not_null: false,
                    forced_size: None,
                })
            } else if let Value::Long(_) = self.result {
                loft::data::I64.clone()
            } else if let Value::Text(_) = self.result {
                Type::Text(loft::data::Deps::none())
            } else if let Value::Float(_) = self.result {
                Type::Float
            } else if let Value::Null = self.result {
                return "";
            } else {
                Type::Unknown(0)
            }
        } else {
            self.tp.clone()
        };
        if let Type::Integer(_) = tp {
            "integer"
        } else if let Type::Text(_) = tp {
            "text"
        } else if let Type::Boolean = tp {
            "boolean"
        } else if let Type::Float = tp {
            "float"
        } else {
            panic!("Unknown type {tp:?}");
        }
    }
}

fn short(name: &str) -> String {
    let s: Vec<&str> = name.split("::").collect();
    s[s.len() - 1].to_string()
}

fn front(name: &str) -> String {
    let s: Vec<&str> = name.split("::").collect();
    s[s.len() - 2].to_string()
}

pub fn testing_code(code: &str, test: &str) -> Test {
    Test {
        name: short(test),
        file: front(test),
        expr: "".to_string(),
        code: code.to_string(),
        warnings: vec![],
        advice: vec![],
        errors: vec![],
        fatal: vec![],
        result: Value::Null,
        tp: Type::Unknown(0),
        sizes: HashMap::new(),
        expected_slots: None,
    }
}

pub fn testing_expr(expr: &str, test: &str) -> Test {
    Test {
        name: short(test),
        file: front(test),
        expr: expr.to_string(),
        code: "".to_string(),
        warnings: vec![],
        advice: vec![],
        errors: vec![],
        fatal: vec![],
        result: Value::Null,
        tp: Type::Unknown(0),
        sizes: HashMap::new(),
        expected_slots: None,
    }
}

#[cfg(test)]
mod replace_tokens_tests {
    //! P233 regression coverage: `replace_tokens` must produce a
    //! string that, after being embedded in a loft string literal
    //! and parsed by loft's lexer, recovers the original input
    //! byte-for-byte.  Without this property the test harness
    //! corrupts JSON-escape shapes and (worse) can hang loft's
    //! lexer on `\\"` patterns that close strings prematurely.
    use super::replace_tokens;

    /// Apply loft's string-literal escape interpretation to `src`.
    /// Mirrors the lexer's escape table for the characters
    /// `replace_tokens` produces: `{{`/`}}` → `{`/`}`, `\"` → `"`,
    /// `\\` → `\`, `\n` → real newline.
    fn loft_lex_string_literal(src: &str) -> String {
        let mut out = String::with_capacity(src.len());
        let bytes = src.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            let c = bytes[i];
            if c == b'\\' && i + 1 < bytes.len() {
                match bytes[i + 1] {
                    b'\\' => out.push('\\'),
                    b'"' => out.push('"'),
                    b'n' => out.push('\n'),
                    b't' => out.push('\t'),
                    other => {
                        out.push('\\');
                        out.push(other as char);
                    }
                }
                i += 2;
                continue;
            }
            if c == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                out.push('{');
                i += 2;
                continue;
            }
            if c == b'}' && i + 1 < bytes.len() && bytes[i + 1] == b'}' {
                out.push('}');
                i += 2;
                continue;
            }
            out.push(c as char);
            i += 1;
        }
        out
    }

    fn roundtrip(s: &str) {
        let encoded = replace_tokens(s);
        let decoded = loft_lex_string_literal(&encoded);
        assert_eq!(
            decoded, s,
            "replace_tokens round-trip lost data\n  input:   {s:?}\n  encoded: {encoded:?}\n  decoded: {decoded:?}"
        );
    }

    #[test]
    fn ascii_passthrough() {
        roundtrip("hello world");
    }

    #[test]
    fn quotes_round_trip() {
        roundtrip(r#"she said "hi""#);
    }

    #[test]
    fn double_backslash_round_trips() {
        // The shape that hung loft's lexer before P233.
        roundtrip(r#"a \\ b"#);
    }

    #[test]
    fn backslash_quote_round_trips() {
        // Literal `\"` (2 chars) — must come back as 2 chars,
        // not be re-interpreted by the lexer as an escaped quote.
        roundtrip(r#"a \" b"#);
    }

    #[test]
    fn json_escaped_text_round_trips() {
        // The exact shape used as `Value::Text` expected in
        // `tests/issues.rs::q3b_struct_to_json_string_escapes_*`.
        roundtrip(r#"{"msg":"she said \"hi\" \\ done"}"#);
        roundtrip(r#"{"msg":"line1\nline2\ttab"}"#);
    }

    #[test]
    fn real_newlines_become_escaped() {
        roundtrip("line1\nline2");
    }

    #[test]
    fn real_tabs_round_trip() {
        // Note: `replace_tokens` doesn't escape real tab → `\t`,
        // but loft's lexer accepts a literal tab in a string
        // literal as itself.
        roundtrip("a\tb");
    }

    #[test]
    fn curly_braces_passthrough() {
        roundtrip("{key: value}");
    }
}
