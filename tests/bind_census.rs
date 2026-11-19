// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! The lowering census as a guard (@PLN220, STORE_RECORDS.md § The lowering census): the
//! interpreter's bytecode writer and the C writer name the same store effect for every
//! assignment of `tests/scripts/a-discharge-block-hands-over-the-local-it-bound.loft` —
//! `scripts/bind_census.py --check` over that one program.  The whole suite is the nightly
//! `Lowering census` job; this is the cell that held the last disagreement, kept on every PR.
//!
//! @falsified-at: none — the check is new with its guard; the second test IS the receipt:
//! with `LOFT_DISCHARGE_BLOCK_JOIN=1` the interpreter joins a discharge block again and the
//! check must fail naming the pair.

fn census(switch: Option<&str>) -> Option<(bool, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let dir = std::env::temp_dir().join(format!(
        "loft-bind-census-{}-{}",
        std::process::id(),
        u8::from(switch.is_some())
    ));
    loft::file_access::create_dir_all(&dir).ok()?;
    // A copy: the census runs the program from its own directory, and a run there writes
    // caches beside it.
    let cell = dir.join("cell.loft");
    loft::file_access::copy(
        &root.join("tests/scripts/a-discharge-block-hands-over-the-local-it-bound.loft"),
        &cell,
    )
    .ok()?;
    let cells = dir.join("cells.txt");
    loft::file_access::write(&cells, cell.display().to_string()).ok()?;
    let mut cmd = loft::platform::process::harness_command("python3");
    cmd.arg(root.join("scripts/bind_census.py"))
        .arg("--loft")
        .arg(env!("CARGO_BIN_EXE_loft"))
        .arg("--cells")
        .arg(&cells)
        .args(["--writer", "c", "--check"])
        .env_remove("LOFT_DISCHARGE_BLOCK_JOIN");
    if let Some(s) = switch {
        cmd.env(s, "1");
    }
    // No python3 on this host: the nightly job is the census then.
    let out = cmd.output().ok()?;
    let _ = loft::file_access::remove_dir_all(&dir);
    Some((
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned() + &String::from_utf8_lossy(&out.stderr),
    ))
}

#[test]
fn both_writers_name_one_store_effect_for_a_discharge_block() {
    let Some((ok, text)) = census(None) else {
        eprintln!("skip: python3 unavailable");
        return;
    };
    assert!(ok, "the census check passes:\n{text}");
    assert!(
        text.contains("paired assignments") && !text.starts_with("0 paired"),
        "the census paired assignments (a check over nothing cannot fail):\n{text}"
    );
}

#[test]
fn the_census_check_fails_when_the_interpreter_joins_the_block_again() {
    let Some((ok, text)) = census(Some("LOFT_DISCHARGE_BLOCK_JOIN")) else {
        eprintln!("skip: python3 unavailable");
        return;
    };
    assert!(!ok, "the check must fail under the switch:\n{text}");
    assert!(
        text.contains("DIFFERS: re R1/b2=pre is lowered as displacing-rebind"),
        "the failure names the pair:\n{text}"
    );
}
