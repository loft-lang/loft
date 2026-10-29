// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! @PLN91 strand 1b — the loft-written interpreter (`selfhost/`), run through its own
//! `loft test` against a program image this harness emits with `loft --emit-image`.
//!
//! The image is the IR the Rust front end writes; the package reads it as the records
//! `selfhost/src/ir.loft` declares.  The harness hands the image's path to the loft tests in
//! `LOFT_SELFHOST_IMAGE` (and a file that is no image in `LOFT_SELFHOST_NOT_AN_IMAGE`), and an
//! image of every kernel in `selfhost/kernels/` in the directory `LOFT_SELFHOST_KERNELS`.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The selfhost package's own tests, on images this harness emits: S0.3's loader (a program
/// with `fn twice` and `fn main`, and a file that is no image) and every kernel of
/// `selfhost/kernels/`.
#[test]
fn the_selfhost_tests_pass_on_an_emitted_image() {
    let tmp = std::env::temp_dir().join(format!("loft_selfhost_{}", std::process::id()));
    let _ = fa::remove_dir_all(&tmp);
    fa::create_dir_all(&tmp).expect("scratch");
    let program = tmp.join("prog.loft");
    fa::write(
        &program,
        "fn twice(n: integer) -> integer { n * 2 }\nfn main() { println(\"{twice(21)}\"); }\n",
    )
    .expect("program");
    let image = tmp.join("prog.store");
    let not_an_image = tmp.join("not.store");
    fa::write(&not_an_image, b"not a store").expect("not an image");

    let emitted = loft::platform::process::harness_command(loft())
        .arg("--emit-image")
        .arg(&image)
        .arg(&program)
        .env("LOFT_NO_CACHE", "1")
        .output()
        .expect("run loft --emit-image");
    assert!(
        emitted.status.success(),
        "emit-image: {}",
        String::from_utf8_lossy(&emitted.stderr)
    );

    let kernels = tmp.join("kernels");
    fa::create_dir_all(&kernels).expect("kernel images");
    for entry in fa::read_dir(root().join("selfhost/kernels")).expect("selfhost/kernels") {
        let source = entry.os_spelling();
        // Only the kernels: a run beside them leaves a `.loft/` cache directory there.
        if fa::extension(&source).as_deref() != Some("loft") {
            continue;
        }
        let Some(stem) = fa::file_stem(&source) else {
            continue;
        };
        let emitted = loft::platform::process::harness_command(loft())
            .arg("--emit-image")
            .arg(kernels.join(format!("{stem}.store")))
            .arg(&source)
            .env("LOFT_NO_CACHE", "1")
            .output()
            .expect("run loft --emit-image");
        assert!(
            emitted.status.success(),
            "emit-image {stem}: {}",
            String::from_utf8_lossy(&emitted.stderr)
        );
        // The C generator's type table for the same program: the oracle the loft-derived
        // table is compared with (@PLN91 S3, `selfhost/tests/types.loft`).
        let c = tmp.join(format!("{stem}.c"));
        // `LOFT_NO_COMPLETE_WRITE` (values unchanged, (R-Switch)) makes the C generator fill
        // every appended record from its default image, so each element type's image is
        // there for P6 to compare.
        let emitted_c = loft::platform::process::harness_command(loft())
            .arg("--emit-c")
            .arg(&c)
            .arg(&source)
            .env("LOFT_NO_CACHE", "1")
            .env("LOFT_NO_COMPLETE_WRITE", "1")
            .output()
            .expect("run loft --emit-c");
        assert!(
            emitted_c.status.success(),
            "emit-c {stem}: {}",
            String::from_utf8_lossy(&emitted_c.stderr)
        );
        let emitted_source = fa::read_to_string(&c).expect("the emitted C");
        let table = c_type_table(&emitted_source);
        fa::write(kernels.join(format!("{stem}.ctypes")), table).expect("the C type table");
        // And its default images (P6): what `OpNewRecord` fills an element with.
        fa::write(
            kernels.join(format!("{stem}.cdflt")),
            c_default_images(&emitted_source),
        )
        .expect("the C default images");
    }

    let tested = loft::platform::process::harness_command(loft())
        .arg("test")
        .current_dir(root().join("selfhost"))
        .env("LOFT_SELFHOST_IMAGE", &image)
        .env("LOFT_SELFHOST_KERNELS", &kernels)
        .env("LOFT_SELFHOST_NOT_AN_IMAGE", &not_an_image)
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("run loft test");
    let _ = fa::remove_dir_all(&tmp);
    let out = String::from_utf8_lossy(&tested.stdout);
    let err = String::from_utf8_lossy(&tested.stderr);
    assert!(
        tested.status.success() && out.contains("passed") && !out.contains("FAIL"),
        "selfhost tests:\n{out}\n{err}"
    );
}

/// The default images in an emitted C program (`static const uint8_t loft_dflt{tp}[n] = { … };`),
/// one line per type: `tp`, then its bytes — the form `selfhost/tests/types.loft` compares with
/// `types::default_image`.
fn c_default_images(c: &str) -> String {
    let mut rows = Vec::new();
    for line in c.lines() {
        let Some(rest) = line.strip_prefix("static const uint8_t loft_dflt") else {
            continue;
        };
        let Some((tp, rest)) = rest.split_once('[') else {
            continue;
        };
        let Some((_, list)) = rest.split_once("= {") else {
            continue;
        };
        let bytes: Vec<&str> = list
            .trim_end_matches(';')
            .trim_end_matches('}')
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .collect();
        rows.push(format!("{tp} {}", bytes.join(" ")));
    }
    rows.join("\n")
}

/// The rows of the `loft_types` table in an emitted C program, one line per type in the form
/// `selfhost/src/types.loft`'s `render` writes: `tp kind size elem n owns_heap linked`, then each
/// field as `pos,type,view,hidden,group`, then a struct enum's variants as `variants=a,b,…`.
/// Keys are left out: the loft side does not derive them yet.
fn c_type_table(c: &str) -> String {
    const KINDS: [&str; 12] = [
        "SCALAR", "TEXT", "STRUCT", "VECTOR", "ENUM", "CHILD", "HASH", "INDEX", "ORDERED", "RADIX",
        "TRIE", "DBREF",
    ];
    let numbers = |text: &str| -> Vec<String> {
        text.split(['{', '}', ','])
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .collect()
    };
    let mut fields: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut variants: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut rows = Vec::new();
    for line in c.lines() {
        if let Some(rest) = line.strip_prefix("static const uint16_t ")
            && let Some((name, list)) = rest.split_once("[] = ")
            && name.starts_with("loft_variants")
        {
            variants.insert(name.to_string(), numbers(list.trim_end_matches(';')));
            continue;
        }
        if let Some(rest) = line.strip_prefix("static const loft_fld ")
            && let Some((name, list)) = rest.split_once("[] = ")
        {
            let n = numbers(list.trim_end_matches(';'));
            let per: Vec<String> = n.chunks(5).map(|f| f.join(",")).collect();
            fields.insert(name.to_string(), per);
            continue;
        }
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix('[')
            && let Some((tp, body)) = rest.split_once("] = {")
            && body.contains("LOFT_T_")
        {
            let parts = numbers(body.trim_end_matches(','));
            let kind = parts[0].trim_start_matches("LOFT_T_");
            let kind = KINDS.iter().position(|k| *k == kind).expect("a known kind");
            let mut out = format!(
                "{tp} {kind} {} {} {} {} {}",
                parts[1], parts[2], parts[3], parts[6], parts[9]
            );
            if let Some(list) = fields.get(&parts[4]) {
                let n: usize = parts[3].parse().unwrap_or(0);
                for f in list.iter().take(n) {
                    out.push(' ');
                    out.push_str(f);
                }
            }
            if let Some(list) = variants.get(&parts[5])
                && !list.is_empty()
            {
                out.push_str(" variants=");
                out.push_str(&list.join(","));
            }
            rows.push(out);
        }
    }
    rows.join("\n")
}
