// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! A log record names the source file the way the config can address it (loft#1264).
//!
//! LOGGER.md has always documented the record's source field as "relative to project root"
//! and shown `src/compute.loft:142`.  What was written was the absolute path the program
//! was invoked by, and that had two consequences — one cosmetic, one a feature that could
//! not work at all.
//!
//! `Logger::effective_level` matches a `[levels]` key ending in `/` with
//! `loft_file.starts_with(pattern)`.  Against an absolute path no such key can ever match,
//! so the path-prefix override that `loft --generate-log-config` documents in its own
//! generated comments had NO working spelling.  The basename form worked, which is what
//! made this survivable and invisible.
//!
//! The cosmetic half is not nothing either: a log shipped from a build machine carried
//! `/home/<user>/…` on every record, long enough to bury the message.
//!
//! The fix relativises where a path ENTERS the logger — `Logger::log` — so the override
//! match, the rate-limit key and the written record cannot disagree about what a file is
//! called.  Diagnostics are deliberately untouched: they are read by the author on the
//! machine that produced them, where the absolute path is the useful form.
//!
//! `a_prefix_that_does_not_match_stays_silent` is the row that keeps the rest honest —
//! without it, a change that made every prefix key match would satisfy them all.

use loft::file_access as fa;
use std::path::Path;

/// Build a program at `<root>/sub/app.loft` with `log.conf` beside it, run it, and return
/// the log file's contents.  `manifest` decides whether the tree is a project.
fn run(tag: &str, manifest: bool, levels: Option<&str>) -> String {
    let root = std::env::temp_dir().join(format!("loft_1264_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    fa::create_dir_all(root.join("sub")).expect("mkdir");
    if manifest {
        fa::write(
            root.join("loft.toml"),
            "[package]\nname = \"p1264\"\nversion = \"0.1.0\"\n",
        )
        .expect("manifest");
    }
    fa::write(
        root.join("sub/app.loft"),
        "fn main() {\n  log_info(\"MARKER\");\n}\n",
    )
    .expect("prog");
    // `level = error` with an override is how a per-file key is scored: the record appears
    // only if the override raised this file's level.  With no override the global level is
    // `info`, which is the control for "the record is written at all".
    let conf = match levels {
        Some(key) => format!("[log]\nfile = log.txt\nlevel = error\n\n[levels]\n{key} = info\n"),
        None => "[log]\nfile = log.txt\nlevel = info\n".to_string(),
    };
    fa::write(root.join("sub/log.conf"), conf).expect("conf");
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret", root.join("sub/app.loft").to_str().unwrap()])
        .env("LOFT_TIMEOUT", "120")
        .current_dir(&root)
        .output()
        .expect("spawn loft");
    let log = fa::read_to_string(root.join("sub/log.txt")).unwrap_or_default();
    assert!(
        out.status.success(),
        "the probe program must run: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = fa::remove_dir_all(&root);
    log
}

/// The source field is project-relative, and carries no absolute path.
#[test]
fn a_record_in_a_project_names_the_file_relative_to_the_root() {
    let log = run("proj", true, None);
    assert!(
        log.contains("sub/app.loft:2"),
        "the record must name the file relative to the project root:\n{log}"
    );
    assert!(
        !log.contains(&format!("{}", std::env::temp_dir().display())),
        "no absolute path may reach a log record — it is shipped off this machine:\n{log}"
    );
}

/// The feature that had no working spelling: a `[levels]` key ending in `/`.
#[test]
fn a_path_prefix_override_fires_in_a_project() {
    for key in ["\"sub/\"", "sub/"] {
        let log = run("prefix", true, Some(key));
        assert!(
            log.contains("MARKER"),
            "the [levels] prefix key {key} must raise this file's level; it matched \
             nothing:\n{log}"
        );
    }
}

/// The negative control.  A prefix naming a different directory must NOT fire — otherwise
/// "the override works" is satisfied by an override that matches everything.
#[test]
fn a_prefix_that_does_not_match_stays_silent() {
    let log = run("noprefix", true, Some("\"other/\""));
    assert!(
        !log.contains("MARKER"),
        "a [levels] prefix naming another directory must not raise this file's \
         level:\n{log}"
    );
}

/// The basename form is the spelling that always worked, and must keep working.
#[test]
fn a_basename_override_still_fires() {
    for key in ["\"app.loft\"", "app.loft"] {
        let log = run("base", true, Some(key));
        assert!(
            log.contains("MARKER"),
            "the [levels] basename key {key} must still raise this file's level:\n{log}"
        );
    }
}

/// A bare script has no project root, and "relative to project root" has no answer for it.
/// Its own directory is the base, which still keeps the developer's home out of the log.
#[test]
fn a_bare_script_is_relative_to_its_own_directory() {
    let log = run("bare", false, None);
    assert!(
        log.contains("app.loft:2"),
        "a bare script's record must still name its file:\n{log}"
    );
    let tmp = format!("{}", std::env::temp_dir().display());
    assert!(
        !log.contains(&tmp),
        "a bare script's record must not carry an absolute path either:\n{log}"
    );
}

/// Diagnostics are a different surface with a different reader, and are deliberately left
/// alone: the author is on the machine that produced them, where the full path is what
/// makes an error message clickable.  This row exists so a future tidy-up that relativises
/// diagnostics too has to be a deliberate decision rather than a side effect.
#[test]
fn a_diagnostic_keeps_its_full_path() {
    let root = std::env::temp_dir().join(format!("loft_1264_diag_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    fa::create_dir_all(&root).expect("mkdir");
    let prog = root.join("bad.loft");
    fa::write(&prog, "fn main() {\n  panic(\"boom\");\n}\n").expect("prog");
    let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
        .args(["--interpret", prog.to_str().unwrap()])
        .env("LOFT_TIMEOUT", "120")
        .output()
        .expect("spawn loft");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let full = Path::new(&prog).to_string_lossy().into_owned();
    assert!(
        text.contains(&full) || text.contains("bad.loft:2"),
        "a diagnostic must still point at the file the author can open:\n{text}"
    );
    let _ = fa::remove_dir_all(&root);
}
