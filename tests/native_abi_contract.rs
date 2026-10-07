// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A `#native` declaration and its Rust `extern "C"` export are ONE contract.
//!
//! loft does not read the Rust signature.  It emits the extern from the loft
//! DECLARATION — `fn f(x: integer) -> integer` becomes
//! `unsafe extern "C" { fn n_f(x: i64) -> i64; }` (doc/claude/PACKAGES.md § native type
//! mapping) — and calls that.  So a narrower Rust signature is not a harmless narrowing,
//! it is an ABI mismatch: on x86-64 SysV a function returning `i32` leaves the upper half
//! of `rax` undefined, and loft reads whatever is there.
//!
//! The failure mode is the worst kind.  It is invisible on `--interpret` (a different
//! dispatch path), invisible for small positive values on `--native`, and wrong only for
//! negatives — which is exactly where sentinels live.  Found in the wild in
//! `loft-libs-net`: `server::listen` answered -1 on a failed bind, which arrived in loft
//! as 4294967295, so the `handle >= 0` check passed and the server reported itself up
//! while accepting nothing.  `web::ws_connect` had the identical defect against a live
//! `if id < 0 { return null }`.
//!
//! Two guards here, because neither alone is enough:
//!   * [`native_declarations_and_exports_agree`] is STATIC and covers every fixture at
//!     once, including shapes no test calls.
//!   * the behavioural half lives in `tests/lib/native_scalar_pkg` — `native_sentinel()`
//!     returns -1 and its `.loft` test asserts the round trip on both backends.  That is
//!     what proves the rule is about real values and not just about text matching.

use loft::file_access as fa;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The Rust types a loft type is allowed to appear as on the C boundary.
/// `boolean` admits `u8` as well as `bool`: both are one byte and Rust's `bool` is
/// guaranteed 0/1, so the two agree in the register.
fn allowed_rust_types(loft_ty: &str) -> Option<&'static [&'static str]> {
    match loft_ty {
        "integer" => Some(&["i64"]),
        "float" => Some(&["f64"]),
        "single" => Some(&["f32"]),
        "boolean" => Some(&["bool", "u8"]),
        // Everything else (text, vector, reference, …) expands to a multi-argument
        // convention this check deliberately does not model — see the doc comment on
        // `check_crate` for why that is safe.
        _ => None,
    }
}

/// `#native` declarations in a package's `.loft` sources: symbol → (param loft types,
/// return loft type).  A bare `#native` means the symbol is `n_<fn name>`.
fn loft_native_decls(pkg: &Path) -> HashMap<String, (Vec<String>, Option<String>)> {
    let mut out = HashMap::new();
    for file in walk(pkg, "loft") {
        let text = fa::read_to_string(&file).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            if !line.trim_start().starts_with("#native") {
                continue;
            }
            // The declaration is the nearest preceding `fn …;` line.
            let Some((decl, name)) = (i.saturating_sub(5)..i)
                .rev()
                .find_map(|j| parse_fn_decl(lines[j]).map(|d| (d, j)))
            else {
                continue;
            };
            let _ = name;
            let rest = line.trim().trim_start_matches("#native").trim();
            let symbol = rest
                .strip_prefix('"')
                .and_then(|r| r.strip_suffix('"'))
                .map_or_else(|| format!("n_{}", decl.0), str::to_string);
            out.insert(symbol, (decl.1, decl.2));
        }
    }
    out
}

/// `fn name(a: T, b: U) -> R;` → (name, [T, U], Some(R)).  Returns None for anything
/// that is not a bare declaration (a definition with a body, a comment, …).
fn parse_fn_decl(line: &str) -> Option<(String, Vec<String>, Option<String>)> {
    let t = line.trim().trim_start_matches("pub ").trim();
    let t = t.strip_prefix("fn ")?;
    let t = t.strip_suffix(';')?;
    let open = t.find('(')?;
    let close = t.rfind(')')?;
    let name = t[..open].trim().to_string();
    let params = t[open + 1..close]
        .split(',')
        .filter_map(|p| p.split_once(':').map(|(_, ty)| ty.trim().to_string()))
        .collect();
    let ret = t[close + 1..]
        .trim()
        .strip_prefix("->")
        .map(|r| r.trim().to_string());
    Some((name, params, ret))
}

/// `extern "C"` exports in the Rust files the crate actually COMPILES — `src/lib.rs`
/// plus the modules it declares.  Scanning the whole tree would pick up stale scaffolds
/// (`loft generate` leaves a `generated.rs` that is never `mod`-declared), and reporting
/// a signature that is not in the built cdylib is a false positive.
fn rust_exports(crate_dir: &Path) -> HashMap<String, (Vec<String>, Option<String>)> {
    let mut out = HashMap::new();
    let lib = crate_dir.join("src/lib.rs");
    if !fa::exists(&lib) {
        return out;
    }
    let mut files = vec![lib.clone()];
    let text = fa::read_to_string(&lib).unwrap_or_default();
    for line in text.lines() {
        let t = line.trim().trim_start_matches("pub ").trim();
        if let Some(rest) = t.strip_prefix("mod ")
            && let Some(name) = rest.strip_suffix(';')
        {
            let cand = crate_dir.join(format!("src/{}.rs", name.trim()));
            if fa::exists(&cand) {
                files.push(cand);
            }
        }
    }
    for f in files {
        let src = fa::read_to_string(&f).unwrap_or_default();
        // Signatures may span lines; join on the opening brace.
        let flat = src.replace('\n', " ");
        let mut rest = flat.as_str();
        while let Some(pos) = rest.find("extern \"C\" fn ") {
            let after = &rest[pos + "extern \"C\" fn ".len()..];
            let Some(open) = after.find('(') else { break };
            let name = after[..open].trim().to_string();
            let Some(close) = after.find(')') else { break };
            let Some(brace) = after[close..].find('{') else {
                break;
            };
            let params: Vec<String> = after[open + 1..close]
                .split(',')
                .filter_map(|p| p.split_once(':').map(|(_, ty)| ty.trim().to_string()))
                .collect();
            let tail = after[close + 1..close + brace].trim();
            let ret = tail
                .strip_prefix("->")
                .map(|r| r.trim().trim_end().to_string());
            // Only C-ABI EXPORTS (`pub`), not local helpers.
            if rest[..pos].trim_end().ends_with("pub")
                || rest[..pos].trim_end().ends_with("pub unsafe")
            {
                out.insert(name, (params, ret));
            }
            rest = &after[close..];
        }
    }
    out
}

fn walk(root: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if fa::is_dir(&p) {
                if name != "target" && name != ".loft" && name != ".git" {
                    stack.push(p);
                }
            } else if p.extension().is_some_and(|x| x == ext) {
                out.push(p);
            }
        }
    }
    out
}

/// Compare one package's declarations against its crate's exports, returning a
/// human-readable line per disagreement.
///
/// Only the SCALAR types in `allowed_rust_types` are judged.  `text` / `vector` /
/// reference args expand to multi-argument conventions (`ptr, len`, `ptr, count`,
/// `LoftStore` handles), so the arities legitimately differ and comparing them
/// positionally would be noise.  Skipping them costs nothing here: the width hazard
/// this test exists for is an integer one.
fn check_crate(pkg: &Path, crate_dir: &Path) -> Vec<String> {
    let decls = loft_native_decls(pkg);
    let exports = rust_exports(crate_dir);
    let mut problems = Vec::new();
    for (symbol, (lparams, lret)) in &decls {
        let Some((rparams, rret)) = exports.get(symbol) else {
            continue; // provided elsewhere (built-in, another crate) — not this test's business
        };
        if let Some(lt) = lret
            && let Some(allowed) = allowed_rust_types(lt)
        {
            match rret {
                Some(rt) if allowed.contains(&rt.as_str()) => {}
                Some(rt) => problems.push(format!(
                    "  {symbol}: loft declares `-> {lt}` (so loft emits `-> {}`), Rust returns `-> {rt}`",
                    allowed[0]
                )),
                None => problems.push(format!(
                    "  {symbol}: loft declares `-> {lt}`, Rust returns nothing"
                )),
            }
        }
        // Params only when the arities line up — see the doc comment.
        if lparams.len() == rparams.len() {
            for (lt, rt) in lparams.iter().zip(rparams) {
                if let Some(allowed) = allowed_rust_types(lt)
                    && !allowed.contains(&rt.as_str())
                {
                    problems.push(format!(
                        "  {symbol}: loft declares a `{lt}` arg (so loft passes `{}`), Rust takes `{rt}`",
                        allowed[0]
                    ));
                }
            }
        }
    }
    problems
}

/// Every `#native` declaration in this repo's fixture packages must match its Rust
/// export.  Fixtures are the only native crates loft owns; real libraries live in their
/// own repos, and the same rule is checked there.
#[test]
fn native_declarations_and_exports_agree() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut checked = 0;
    let mut problems = Vec::new();
    for base in ["tests/fixtures/libs", "tests/lib"] {
        let Ok(rd) = std::fs::read_dir(root.join(base)) else {
            continue;
        };
        for e in rd.flatten() {
            let pkg = e.path();
            let crate_dir = pkg.join("native");
            if !fa::exists(crate_dir.join("src/lib.rs")) {
                continue;
            }
            checked += 1;
            problems.extend(check_crate(&pkg, &crate_dir));
        }
    }
    // Guard against the check silently covering nothing — a moved fixture directory
    // would otherwise turn this test green by finding no crates at all.
    assert!(
        checked >= 3,
        "expected at least 3 native fixture crates, found {checked} — the fixture layout moved \
         and this test is no longer checking anything"
    );
    assert!(
        problems.is_empty(),
        "`#native` declaration / Rust export mismatches ({} in {checked} crates).\n\
         loft emits the extern from the DECLARATION and calls it directly, so a narrower \
         Rust signature reads undefined register bits — silently, and only on --native, \
         and only for values the narrow type cannot carry (negatives above all):\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// The check must be able to SEE a mismatch — a parser that quietly matches nothing
/// would make the test above vacuously green.  Feeds it a crate whose export is
/// deliberately narrow and requires that it complains.
#[test]
fn the_check_detects_a_deliberate_mismatch() {
    let dir = std::env::temp_dir().join(format!("loft_abi_probe_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(dir.join("src")).expect("mkdir src");
    fa::create_dir_all(dir.join("native/src")).expect("mkdir native/src");
    fa::write(
        dir.join("src/probe.loft"),
        "pub fn probe_value() -> integer;\n#native\n",
    )
    .expect("write loft");
    fa::write(
        dir.join("native/src/lib.rs"),
        "#[no_mangle]\npub extern \"C\" fn n_probe_value() -> i32 { -1 }\n",
    )
    .expect("write rust");

    let problems = check_crate(&dir, &dir.join("native"));
    let _ = fa::remove_dir_all(&dir);

    assert!(
        problems.iter().any(|p| p.contains("n_probe_value")),
        "the ABI check did not flag a deliberately narrow export — it is not actually \
         parsing, so `native_declarations_and_exports_agree` proves nothing. Got: {problems:?}"
    );
}
