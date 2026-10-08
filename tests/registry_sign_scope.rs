// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @C96 — shipping is keyed on trust-root presence: the scope cells keep a key holder's
// signature from re-pointing a published version, and the routing cells at the end keep
// `loft ship` autonomous only where the key is.

//! `registry-sign.sh`'s scope check must tell a version's METADATA correction from a
//! re-point of a version's BYTES.
//!
//! `--expect P@V` matches a CHANGED version as well as a new one — which is what makes a
//! per-version correction (`deps`, `api`) signable at all, and is the route
//! `registry_maintain.sh` directs an operator to when a publish carries no `deps`
//! ("Add them to `[dependencies]` or to the previous entry's `deps` first").  But the same
//! latitude read a tarball swap on an ALREADY-PUBLISHED version exactly like a `deps` fix:
//! the identical scope line, the identical "nothing else added, removed or altered", and
//! the download check confirms only that the new sha matches the new url — never that
//! either still matches what consumers already resolved.  A lock file names the version,
//! not the bytes, so that write re-points something already installed, and
//! REGISTRY_SUBMIT.md's "Published releases are immutable" had no enforcement anywhere.
//!
//! Measured on `hex_terrain` 0.1.0, whose published entry carries `deps: {}` while its
//! source does `use hex_grid` — so the correction is a real one, and the neighbouring
//! dangerous write is one field away from it.
//!
//! The cells run the review block lifted out of the SCRIPT rather than a restatement of
//! its rules, the way `registry_publish_categories.rs` lifts the publish fold: an edit
//! that changes the rule changes what runs here too.  Both directions are asserted —
//! the correction must still pass and an ordinary new-version publish must still pass,
//! because a refusal that also blocks the everyday route would simply be turned off.

use loft::file_access as fa;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Lift the review block out of `registry-sign.sh` — the body of its `python3 - <<'PY'`
/// heredoc, which is where every scope decision is made.
fn review_source() -> String {
    let script = fa::read_to_string(repo_root().join("scripts/registry-sign.sh"))
        .expect("registry-sign.sh readable");
    let open = script
        .find("python3 - \"$PREV\" \"$INDEX\" <<'PY'\n")
        .expect("the review heredoc opener — did registry-sign.sh change shape?");
    let body = &script[open..];
    let body = &body[body.find('\n').expect("end of the opener line") + 1..];
    let end = body.find("\nPY\n").expect("the review heredoc terminator");
    body[..end].to_string()
}

/// One index, as JSON text.  `deps` and the byte-identity triple are what the cells move.
fn index_with(versions: &str) -> String {
    format!(
        r#"{{"schema_version": 1, "updated": "2026-09-10T00:00:00Z", "packages": {{
             "hex_terrain": {{"description": "terrain over a hex grid",
                              "homepage": "https://example.invalid",
                              "categories": ["game"], "yanked": [],
                              "versions": {{{versions}}}}}}}}}"#
    )
}

/// A version entry.  The url deliberately does NOT match the github-release regex, so the
/// review block makes no `gh` call and the cells stay offline and fast.
fn version(ver: &str, sha: &str, size: u32, deps: &str) -> String {
    format!(
        r#""{ver}": {{"url": "https://example.invalid/hex_terrain-{ver}.tar.gz",
                     "sha256": "{sha}", "size": {size}, "loft": ">=0.8",
                     "subpath": "hex_terrain", "deps": {deps},
                     "published": "2026-06-14T14:42:33Z"}}"#
    )
}

struct Verdict {
    ok: bool,
    out: String,
}

/// Run the lifted review block over `prev` → `cur` with the given bound forms.
fn review(tag: &str, prev: &str, cur: &str, expect: &str, expect_meta: &str) -> Verdict {
    let dir = std::env::temp_dir().join(format!("loft_signscope_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("temp dir");
    let script = dir.join("review.py");
    fa::write(&script, review_source()).expect("write review");
    let p = dir.join("prev.json");
    let c = dir.join("cur.json");
    fa::write(&p, prev).expect("write prev");
    fa::write(&c, cur).expect("write cur");
    let out = loft::platform::process::harness_command("python3")
        .arg(&script)
        .arg(&p)
        .arg(&c)
        .env("DOWNLOAD", "0")
        .env("NOTES", "0")
        .env("EXPECT", expect)
        .env("EXPECT_YANK", "")
        .env("EXPECT_META", expect_meta)
        .output()
        .expect("python3 runs the review block");
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    text.push_str(&String::from_utf8_lossy(&out.stderr));
    let _ = fa::remove_dir_all(&dir);
    Verdict {
        ok: out.status.success(),
        out: text,
    }
}

const SHA_A: &str = "98669010662c92b5d6a26a1d6829e96458fa79c2ed57929568d87f3b78c7249c";
const SHA_B: &str = "0ac6d756c694584954ff9b9d6215ec9738008bd16713b77a01b91ada95f39f49";

#[test]
fn correcting_a_published_versions_deps_is_signable_and_says_what_moved() {
    let prev = index_with(&version("0.1.0", SHA_A, 14087, "{}"));
    let cur = index_with(&version("0.1.0", SHA_A, 14087, r#"{"hex_grid": ">=0.1"}"#));
    let v = review("deps", &prev, &cur, "hex_terrain@0.1.0", "");
    assert!(
        v.ok,
        "a deps correction must be signable under --expect:\n{}",
        v.out
    );
    // The reviewer is the trust root, so the delta has to be VISIBLE, not merely allowed:
    // the block otherwise prints the post-state only, which reads the same either way.
    assert!(
        v.out.contains("WAS PUBLISHED") && v.out.contains("fields moved: deps"),
        "the review must name the field that moved:\n{}",
        v.out
    );
    assert!(
        v.out.contains("was {}") && v.out.contains("'hex_grid': '>=0.1'"),
        "the review must show was -> now for the corrected field:\n{}",
        v.out
    );
}

#[test]
fn repointing_a_published_versions_bytes_is_refused() {
    let prev = index_with(&version("0.1.0", SHA_A, 14087, "{}"));
    // Internally consistent — a different tarball with its own honest sha and size, which
    // is what makes the download check unable to see it.
    let cur = index_with(&version("0.1.0", SHA_B, 14113, "{}"));
    let v = review("repoint", &prev, &cur, "hex_terrain@0.1.0", "");
    assert!(
        !v.ok,
        "moving a published version's bytes must NOT sign — that re-points an install:\n{}",
        v.out
    );
    assert!(
        v.out.contains("IMMUTABLE") && v.out.contains("hex_terrain@0.1.0"),
        "the refusal must name the rule and the version:\n{}",
        v.out
    );
    assert!(
        v.out.contains("sha256"),
        "and which fields moved:\n{}",
        v.out
    );
}

#[test]
fn an_ordinary_new_version_publish_is_untouched() {
    // The control that keeps the refusal honest: it must be blind to a NEW version, or the
    // everyday publish path breaks and the check gets switched off rather than fixed.
    let prev = index_with(&version("0.1.0", SHA_A, 14087, "{}"));
    let cur = index_with(&format!(
        "{}, {}",
        version("0.1.0", SHA_A, 14087, "{}"),
        version("0.1.1", SHA_B, 14113, "{}")
    ));
    let v = review("newver", &prev, &cur, "hex_terrain@0.1.1", "");
    assert!(
        v.ok,
        "an ordinary new-version publish must still sign:\n{}",
        v.out
    );
    assert!(v.out.contains("[NEW]"), "and read as NEW:\n{}", v.out);
    assert!(
        !v.out.contains("WAS PUBLISHED"),
        "a new version was never published, so the immutability report must stay quiet:\n{}",
        v.out
    );
}

#[test]
fn expect_meta_on_a_version_field_names_the_form_that_works() {
    // The navigational half.  `--expect-meta` is package-level, so a version's `deps`
    // lands in its overreach refusal — and that refusal used to end the trail, which read
    // as "the signer has no bound form for this" when one exists a flag away.
    let prev = index_with(&version("0.1.0", SHA_A, 14087, "{}"));
    let cur = index_with(&version("0.1.0", SHA_A, 14087, r#"{"hex_grid": ">=0.1"}"#));
    let v = review("route", &prev, &cur, "", "hex_terrain");
    assert!(
        !v.ok,
        "--expect-meta must still refuse a per-version field:\n{}",
        v.out
    );
    assert!(
        v.out.contains("--expect <pkg>@<ver>"),
        "the refusal must name the form that DOES describe this write:\n{}",
        v.out
    );
}

/// One `loft ship` with `HOME` a private tree that does or does not hold the trust-root key, and
/// a stub in place of `registry_maintain.sh` (`LOFT_SHIP_SCRIPT`) that reports what it was handed:
/// its arguments and the signer it was told to use.  Nothing is packaged, signed or pushed.
#[cfg(unix)]
fn ship_with_key(tag: &str, key: bool) -> (String, String) {
    let home = std::env::temp_dir().join(format!("loft-ship-route-{tag}-{}", std::process::id()));
    let _ = fa::remove_dir_all(&home);
    fa::create_dir_all(home.join(".loft/trust-root")).expect("mkdir");
    if key {
        fa::write(home.join(".loft/trust-root/registry-signing-key.bin"), "k").expect("key");
    }
    let stub = home.join("ship-stub.sh");
    fa::write(
        &stub,
        "echo \"ARGS=$*\"\necho \"SIGNER=${LOFT_REGISTRY_SIGNER:-unset}\"\n",
    )
    .expect("stub");
    let out = loft::platform::process::harness_command(PathBuf::from(env!("CARGO_BIN_EXE_loft")))
        .arg("ship")
        .env("HOME", &home)
        .env("LOFT_SHIP_SCRIPT", &stub)
        .env_remove("LOFT_REGISTRY_SIGNER")
        .output()
        .expect("spawn loft ship");
    let _ = fa::remove_dir_all(&home);
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// @C96 — a key-present machine ships autonomously: `--yes`, and the local file key as signer.
#[cfg(unix)]
#[test]
fn a_key_present_machine_ships_autonomously_with_the_file_signer() {
    let (out, err) = ship_with_key("present", true);
    assert!(
        out.contains("--yes") && !out.contains("--dry-run") && out.contains("SIGNER=file"),
        "a key-present `loft ship` runs the routine with `--yes` and the file signer:\n{out}{err}"
    );
    assert!(
        !err.contains("cannot sign"),
        "nothing to defer on a key-present machine:\n{err}"
    );
}

/// @C96 — a key-absent machine defers: it says it cannot sign, and runs the routine in review
/// mode (`--dry-run`) with no signer chosen for it.
#[cfg(unix)]
#[test]
fn a_key_absent_machine_says_so_and_runs_in_review_mode() {
    let (out, err) = ship_with_key("absent", false);
    assert!(
        out.contains("--dry-run") && !out.contains("--yes") && out.contains("SIGNER=unset"),
        "a key-absent `loft ship` runs the routine in review mode, unsigned:\n{out}{err}"
    );
    assert!(
        err.contains("this machine cannot sign"),
        "a key-absent machine says why it is not shipping:\n{err}"
    );
}
