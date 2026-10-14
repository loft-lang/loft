// @PLN109 — golden regression guard for `crate::json::parse` / `parse_with` over a
// liberal corpus of real consumer shapes + edge cases, as a `Debug`-string golden
// (`tests/json_corpus.golden`). Born as the Phase-0 executable spec for the
// lexer-driven rewrite (Phase 2 reproduced every snapshot byte-for-byte); it now
// pins the @PLN109 end state, incl. the integer-shaped `Parsed::Int` values (H5).
// Kept as a permanent guard rather than deleted — a cheap, exhaustive lock on the
// number/escape/structure/error surface. Re-bless only for an intended change:
//   bless: LOFT_BLESS_JSON_CORPUS=1 cargo test --test json_corpus
//
//   run:   cargo test --test json_corpus
//   bless: LOFT_BLESS_JSON_CORPUS=1 cargo test --test json_corpus
//
// The corpus is deliberately liberal (design-protocol: missing a shape is the
// worst failure; redundant variants cost nothing). It pins numbers (H5 + every
// exponent form), string escapes (\uXXXX + surrogate pairs + astral + \/),
// structure/whitespace, the true/false/null literals, error byte-offsets (the
// diagnostic-parity risk), the `Dialect::Lenient` bare-key/ident/constructor
// paths, and one extracted REAL consumer input (the registry index sample).

use loft::file_access as fa;
use loft::json::{Dialect, parse, parse_with};
use std::fmt::Write as _;

/// (label, dialect, input) — dialect selects `parse` (Strict) vs `parse_with`.
const CORPUS: &[(&str, Dialect, &str)] = &[
    // ── numbers: integer-shaped (these FLIP to Parsed::Int in Phase 2) ──────
    ("num/zero", Dialect::Strict, "0"),
    ("num/neg-zero", Dialect::Strict, "-0"),
    ("num/small", Dialect::Strict, "42"),
    ("num/neg", Dialect::Strict, "-42"),
    ("num/i32-max", Dialect::Strict, "2147483647"),
    ("num/i32-max+1", Dialect::Strict, "2147483648"),
    ("num/2^53", Dialect::Strict, "9007199254740992"),
    ("num/2^53+1-H5", Dialect::Strict, "9007199254740993"),
    ("num/i64-max", Dialect::Strict, "9223372036854775807"),
    // > i64 — by decision these stay float (documented ceiling)
    ("num/i64-max+1", Dialect::Strict, "9223372036854775808"),
    ("num/u64-max", Dialect::Strict, "18446744073709551615"),
    // ── numbers: fractional / exponent (these STAY Parsed::Number) ──────────
    ("num/frac", Dialect::Strict, "3.14"),
    ("num/neg-frac", Dialect::Strict, "-3.14"),
    ("num/one-point-zero", Dialect::Strict, "1.0"),
    ("num/frac-zero", Dialect::Strict, "0.0"),
    ("num/exp-lower", Dialect::Strict, "1e3"),
    ("num/exp-upper", Dialect::Strict, "1E3"),
    ("num/exp-plus", Dialect::Strict, "1e+3"),
    ("num/exp-minus", Dialect::Strict, "1e-3"),
    ("num/exp-upper-plus", Dialect::Strict, "1E+5"),
    ("num/frac-exp", Dialect::Strict, "1.5e3"),
    ("num/frac-exp-neg", Dialect::Strict, "2.5e-2"),
    // ── string escapes (the highest correctness risk of the rewrite) ────────
    ("str/plain", Dialect::Strict, "\"hello\""),
    ("str/empty", Dialect::Strict, "\"\""),
    ("str/escaped-quote", Dialect::Strict, "\"a\\\"b\""),
    ("str/escaped-backslash", Dialect::Strict, "\"a\\\\b\""),
    ("str/tab", Dialect::Strict, "\"t\\tb\""),
    ("str/newline", Dialect::Strict, "\"n\\nb\""),
    ("str/carriage", Dialect::Strict, "\"c\\rb\""),
    ("str/json-slash", Dialect::Strict, "\"a\\/b\""),
    ("str/u-ascii", Dialect::Strict, "\"\\u0041\""),
    ("str/u-latin", Dialect::Strict, "\"\\u00e9\""),
    ("str/u-bmp", Dialect::Strict, "\"\\u2764\""),
    ("str/surrogate-pair", Dialect::Strict, "\"\\uD83D\\uDE00\""),
    ("str/astral-clef", Dialect::Strict, "\"\\uD834\\uDD1E\""),
    ("str/braces-literal", Dialect::Strict, "\"a{b}c\""),
    // ── structure / whitespace ──────────────────────────────────────────────
    ("struct/empty-array", Dialect::Strict, "[]"),
    ("struct/empty-object", Dialect::Strict, "{}"),
    ("struct/flat-array", Dialect::Strict, "[1,2,3]"),
    ("struct/nested-array", Dialect::Strict, "[[1],[2,[3]]]"),
    ("struct/flat-object", Dialect::Strict, "{\"a\":1}"),
    (
        "struct/deep-object",
        Dialect::Strict,
        "{\"a\":{\"b\":{\"c\":1}}}",
    ),
    (
        "struct/mixed",
        Dialect::Strict,
        "{\"arr\":[1,2],\"obj\":{\"x\":true}}",
    ),
    (
        "struct/whitespace",
        Dialect::Strict,
        "  {  \"a\" : 1 ,  \"b\" : [ 2 , 3 ] }  ",
    ),
    (
        "struct/array-of-objects",
        Dialect::Strict,
        "[{\"v\":10},{\"v\":20}]",
    ),
    // ── literals ────────────────────────────────────────────────────────────
    ("lit/true", Dialect::Strict, "true"),
    ("lit/false", Dialect::Strict, "false"),
    ("lit/null", Dialect::Strict, "null"),
    // ── errors: pin the byte_offset / message (diagnostic parity) ───────────
    ("err/empty-input", Dialect::Strict, ""),
    ("err/unterminated-object", Dialect::Strict, "{"),
    ("err/unterminated-array", Dialect::Strict, "[1,2"),
    ("err/missing-value", Dialect::Strict, "{\"a\":}"),
    ("err/missing-colon", Dialect::Strict, "{\"a\" 1}"),
    ("err/trailing-comma-array", Dialect::Strict, "[1,2,]"),
    ("err/trailing-comma-object", Dialect::Strict, "{\"a\":1,}"),
    ("err/bad-literal", Dialect::Strict, "nul"),
    ("err/unterminated-string", Dialect::Strict, "\"abc"),
    ("err/bare-key-strict", Dialect::Strict, "{a:1}"),
    ("err/bad-escape", Dialect::Strict, "\"a\\qb\""),
    ("err/lone-high-surrogate", Dialect::Strict, "\"\\uD83D\""),
    // ── Dialect::Lenient — bare keys / ident values / constructors ──────────
    ("lenient/bare-key", Dialect::Lenient, "{a: 1}"),
    ("lenient/bare-ident-value", Dialect::Lenient, "{a: b}"),
    ("lenient/mixed-keys", Dialect::Lenient, "{a: 1, \"b\": 2}"),
    (
        "lenient/bare-literals",
        Dialect::Lenient,
        "{x: true, y: null, z: false}",
    ),
    ("lenient/constructor", Dialect::Lenient, "Point{x: 1, y: 2}"),
    (
        "lenient/constructor-array",
        Dialect::Lenient,
        "[Red{v: 1}, Blue{v: 2}]",
    ),
    // ── real extracted consumer input: the registry index sample ────────────
    (
        "real/registry-index",
        Dialect::Strict,
        r#"{
            "schema_version": 1,
            "updated": "2026-05-24T08:00:00Z",
            "packages": {
                "crypto": {
                    "description": "SHA-256 etc.",
                    "categories": ["crypto"],
                    "yanked": ["0.1.0"],
                    "versions": {
                        "0.1.1": {
                            "url": "https://example.com/crypto-0.1.1.tar.gz",
                            "sha256": "def",
                            "size": 110,
                            "loft": ">=0.8",
                            "deps": {"hash": ">=0.1"},
                            "prerelease": true
                        }
                    }
                }
            }
        }"#,
    ),
    // ── real extracted consumer input: an RPC request line ──────────────────
    (
        "real/rpc-request",
        Dialect::Strict,
        r#"{"method":"eval","params":["greet()"],"id":7,"verified":true}"#,
    ),
];

/// Render one corpus entry to a stable, human-readable snapshot block.
fn snapshot_entry(label: &str, dialect: Dialect, input: &str) -> String {
    let parsed = match dialect {
        Dialect::Strict => parse(input),
        d => parse_with(input, d),
    };
    let mut s = String::new();
    writeln!(s, "=== {label} [{dialect:?}] ===").unwrap();
    writeln!(s, "input: {input:?}").unwrap();
    writeln!(s, "{parsed:#?}").unwrap();
    s
}

fn render_all() -> String {
    let mut out = String::new();
    out.push_str(
        "# @PLN109 Phase 0 golden — CURRENT crate::json output (auto-generated; bless to update)\n\n",
    );
    for (label, dialect, input) in CORPUS {
        out.push_str(&snapshot_entry(label, *dialect, input));
        out.push('\n');
    }
    out
}

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/json_corpus.golden");

#[test]
fn corpus_matches_golden() {
    let actual = render_all();

    let blessing = std::env::var("LOFT_BLESS_JSON_CORPUS").is_ok();
    let existing = fa::read_to_string(GOLDEN).ok();

    if blessing || existing.is_none() {
        fa::write(GOLDEN, &actual).expect("write golden");
        eprintln!(
            "json_corpus: blessed {} entries -> tests/json_corpus.golden",
            CORPUS.len()
        );
        return;
    }

    let expected = existing.unwrap();
    assert_eq!(
        actual, expected,
        "\n\njson_corpus drift vs tests/json_corpus.golden.\n\
         If this is Phase 2's intended integer-preservation (Number -> Int on \
         integer-shaped values), hand-verify each diff is EXACTLY that, then \
         re-bless: LOFT_BLESS_JSON_CORPUS=1 cargo test --test json_corpus\n"
    );
}

/// Guard: the harness must be able to FAIL (a vacuous golden is worthless).
/// Perturbing any input changes its snapshot, so a stale golden mismatches.
#[test]
fn harness_is_not_vacuous() {
    let a = snapshot_entry("probe", Dialect::Strict, "1");
    let b = snapshot_entry("probe", Dialect::Strict, "2");
    assert_ne!(a, b, "snapshot must vary with input");
}
