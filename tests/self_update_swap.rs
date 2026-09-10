// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// @PLN78 step 4 — replacing the RUNNING loft, on whatever platform CI is.
//
// This was written down as the one thing no test could cover: `apply_bundle` renames
// the target aside and copies in, because a running executable cannot be overwritten on
// Windows but can be renamed, and the unit tests only ever exercised that on ordinary
// files.  "Needs a published release and a Windows box" was wrong — what it needs is a
// loft that is running from the path being replaced, and a test can arrange that by
// running a COPY of the binary out of a temporary installation.
//
// So the Windows leg of the daily matrix now covers the swap for real, and the same
// test guards the Unix path (where unlinking a running binary is legal but leaves the
// process on the old inode) rather than being skipped there.

#![cfg(feature = "registry")]

use std::path::{Path, PathBuf};

fn exe_name() -> &'static str {
    if cfg!(windows) { "loft.exe" } else { "loft" }
}

fn write(path: &Path, body: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body).unwrap();
}

fn sha256_hex(bytes: &[u8]) -> String {
    loft::integrity::sha256_hex(bytes)
}

/// Write `SHA256SUMS` covering every file under `root` except itself — the same shape
/// `make-release.sh` produces, because `apply_bundle` refuses a bundle that fails it.
fn write_manifest(root: &Path) {
    let mut lines = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in std::fs::read_dir(&dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let rel = p
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            if rel == "SHA256SUMS" {
                continue;
            }
            lines.push(format!(
                "{}  {rel}",
                sha256_hex(&std::fs::read(&p).unwrap())
            ));
        }
    }
    lines.sort();
    std::fs::write(root.join("SHA256SUMS"), lines.join("\n") + "\n").unwrap();
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir()
        .join("loft-self-update-swap")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Replace the binary that is executing the replacement.
///
/// The staged `bin/loft` is deliberately NOT a working executable — the assertion is
/// about the bytes landing, and using real ones would make a silent no-op (the file
/// never replaced) indistinguishable from success.
#[test]
fn self_update_replaces_the_running_binary() {
    let base = scratch("running");
    let install = base.join("install");
    let staged = base.join("staged");

    // A minimal installation, laid out as a release bundle: <root>/bin + <root>/default.
    let running = install.join("bin").join(exe_name());
    std::fs::create_dir_all(running.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_loft"), &running).unwrap();
    write(&install.join("default").join("a.loft"), b"old\n");
    write_manifest(&install);

    // The bundle to install over it.
    const NEW_BINARY: &[u8] = b"REPLACED-BINARY-CONTENT\n";
    write(&staged.join("bin").join(exe_name()), NEW_BINARY);
    write(&staged.join("default").join("a.loft"), b"new\n");
    write_manifest(&staged);

    // Run the COPY, so the process replacing `bin/loft` is running FROM `bin/loft`.
    let out = std::process::Command::new(&running)
        .args(["self-update", "--from"])
        .arg(&staged)
        .output()
        .expect("run the installed loft");

    assert!(
        out.status.success(),
        "self-update --from failed\nstdout: {}\nstderr: {}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read(&running).unwrap(),
        NEW_BINARY,
        "the running binary must have been replaced, not skipped"
    );
    assert_eq!(
        std::fs::read(install.join("default").join("a.loft")).unwrap(),
        b"new\n",
        "the stdlib must move with the binary — a new bin/loft beside an old default/ \
         is the partial upgrade `verify-self` exists to catch"
    );
}

/// `--dry-run` must change nothing, including on the running binary.  Without this the
/// test above passes just as well for an implementation that always replaces.
#[test]
fn a_dry_run_leaves_the_running_binary_alone() {
    let base = scratch("dry");
    let install = base.join("install");
    let staged = base.join("staged");

    let running = install.join("bin").join(exe_name());
    std::fs::create_dir_all(running.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_loft"), &running).unwrap();
    write(&install.join("default").join("a.loft"), b"old\n");
    write_manifest(&install);
    let before = std::fs::read(&running).unwrap();

    write(&staged.join("bin").join(exe_name()), b"WOULD-REPLACE\n");
    write(&staged.join("default").join("a.loft"), b"new\n");
    write_manifest(&staged);

    let out = std::process::Command::new(&running)
        .args(["self-update", "--dry-run", "--from"])
        .arg(&staged)
        .output()
        .expect("run the installed loft");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read(&running).unwrap(),
        before,
        "--dry-run must not write"
    );
    assert_eq!(
        std::fs::read(install.join("default").join("a.loft")).unwrap(),
        b"old\n"
    );
}

/// A bundle that contradicts its own manifest is refused, and nothing moves — the
/// truncated-copy case, which is the failure that actually happens.
#[test]
fn a_bundle_that_fails_its_manifest_replaces_nothing() {
    let base = scratch("corrupt");
    let install = base.join("install");
    let staged = base.join("staged");

    let running = install.join("bin").join(exe_name());
    std::fs::create_dir_all(running.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_loft"), &running).unwrap();
    write(&install.join("default").join("a.loft"), b"old\n");
    write_manifest(&install);
    let before = std::fs::read(&running).unwrap();

    write(&staged.join("bin").join(exe_name()), b"NEW\n");
    write(&staged.join("default").join("a.loft"), b"new\n");
    write_manifest(&staged);
    // Corrupt AFTER the manifest is written, so the bundle disagrees with itself.
    write(&staged.join("default").join("a.loft"), b"tampered\n");

    let out = std::process::Command::new(&running)
        .args(["self-update", "--from"])
        .arg(&staged)
        .output()
        .expect("run the installed loft");
    assert!(
        !out.status.success(),
        "a contradicted manifest must be refused"
    );
    assert_eq!(
        std::fs::read(&running).unwrap(),
        before,
        "a refused bundle must leave the running binary untouched"
    );
}

/// `scripts/install.sh` installs the WHOLE bundle — the unit `apply_bundle` above
/// installs — and the installation it leaves passes `loft verify-self`.
///
/// The script is the documented `curl | sh` path.  It used to copy `bin/` and `default/`
/// only and then hand the manifest of the FULL bundle to `verify-self`, which counts a
/// missing file as a failure, so every installation it made ended in "the installation
/// does not verify" — and nothing ran the script to notice (2026.8.0 shipped that way).
/// A `file://` base URL keeps the run local: `curl` serves the fixture zip the way the
/// release CDN would.  Linux x86_64 only: the script derives the artifact name from
/// `uname`, and that is the one such host CI runs the tests on.
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn install_sh_installs_the_whole_bundle_and_it_verifies() {
    use std::io::Write as _;
    let version = env!("CARGO_PKG_VERSION");
    let name = format!("loft-{version}-x86_64-unknown-linux-musl");
    let root = scratch("install-sh");

    // A bundle laid out the way make-release.sh lays one out: the runtime and the files
    // a user reads, all under one manifest.
    let bundle = root.join(&name);
    write(
        &bundle.join("bin").join("loft"),
        &std::fs::read(env!("CARGO_BIN_EXE_loft")).unwrap(),
    );
    for e in std::fs::read_dir("default").unwrap().flatten() {
        if e.path().extension().is_some_and(|x| x == "loft") {
            write(
                &bundle.join("default").join(e.file_name()),
                &std::fs::read(e.path()).unwrap(),
            );
        }
    }
    write(&bundle.join("README.md"), b"# loft\n");
    write(
        &bundle.join("examples").join("hello.loft"),
        b"fn main() { println(\"hello\"); }\n",
    );
    write_manifest(&bundle);
    let manifest = std::fs::read_to_string(bundle.join("SHA256SUMS")).unwrap();
    let listed: Vec<&str> = manifest
        .lines()
        .map(|l| &l[l.find("  ").unwrap() + 2..])
        .collect();

    // Serve it as the releases page does: <base>/v<version>/<name>.zip plus its sidecar.
    let srv = root.join("srv");
    let dir = srv.join(format!("v{version}"));
    std::fs::create_dir_all(&dir).unwrap();
    let zip_path = dir.join(format!("{name}.zip"));
    {
        let mut w = zip::ZipWriter::new(std::fs::File::create(&zip_path).unwrap());
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .unix_permissions(0o755);
        for rel in listed.iter().copied().chain(std::iter::once("SHA256SUMS")) {
            w.start_file(format!("{name}/{rel}"), opts).unwrap();
            w.write_all(&std::fs::read(bundle.join(rel)).unwrap())
                .unwrap();
        }
        w.finish().unwrap();
    }
    write(
        &dir.join(format!("{name}.zip.sha256")),
        format!(
            "{}  {name}.zip\n",
            sha256_hex(&std::fs::read(&zip_path).unwrap())
        )
        .as_bytes(),
    );

    let prefix = root.join("prefix");
    let out = std::process::Command::new("sh")
        .arg("scripts/install.sh")
        .arg("--prefix")
        .arg(&prefix)
        .arg("--version")
        .arg(version)
        .env("LOFT_INSTALL_BASE", format!("file://{}", srv.display()))
        // No registry index under this home and no registry to fetch one from (a closed
        // loopback port: `InstallOptions::default()` is not offline), so the origin row is
        // informational and the verdict rests on the two manifest rows the bundle carries.
        .env("LOFT_HOME", root.join("home"))
        .env("LOFT_REGISTRY_URL", "http://127.0.0.1:9/")
        .output()
        .expect("run sh scripts/install.sh");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "install.sh failed\n--- stdout\n{stdout}\n--- stderr\n{stderr}"
    );
    // Every file the manifest lists is installed — the property that lets verify-self
    // hold the installation to the manifest at all.
    for rel in &listed {
        assert!(
            prefix.join(rel).is_file(),
            "install.sh did not install {rel}\n--- stdout\n{stdout}\n--- stderr\n{stderr}"
        );
    }
    assert!(
        prefix.join("SHA256SUMS").is_file(),
        "the manifest itself was not installed"
    );
    let confirm = format!("files: {} file(s) match", listed.len());
    assert!(
        stdout.contains(&confirm),
        "verify-self did not confirm the whole manifest (wanted `{confirm}`)\n--- stdout\n{stdout}\n--- stderr\n{stderr}"
    );
}

/// loft#1497 — an update that would not be the thing that RUNS is refused before anything
/// moves.
///
/// `self-update` writes the release layout (`bin/` beside `default/`) while the runtime's
/// resolver prefers `<prefix>/share/loft/` whenever that directory exists — which is exactly
/// what `make install` creates.  On a prefix holding both, the update landed a stdlib nothing
/// loads and the OLDER tree kept winning, silently: a 2026.9.0 binary reading a 2026.8.0
/// standard library, `verify-self` passing throughout, and `println("hello")` dying with
/// SIGSEGV in `OpFreeText`.  Reported from two machines; a third, built from source so binary
/// and stdlib were installed together, ran it fine — the control that says the split is by
/// install METHOD and not by release.
///
/// The refusal is BEFORE the write, not a rollback after it: the answer is knowable without
/// touching the disk, and the cure is the user's choice between two layouts rather than
/// something `self-update` can repair — a release bundle carries no `libloft.rlib`, and
/// `--native` links that from `share/loft/`, so writing the stdlib there would swap one
/// mismatch for a subtler one.
/// Falsified: with the pre-flight neutered (`if false && shadowed && !force`) this test
/// fails and the other six still pass, so it discriminates the fix and not the harness.
#[test]
fn an_update_that_would_not_be_the_stdlib_that_loads_is_refused() {
    let base = scratch("shadowed");
    let install = base.join("install");
    let staged = base.join("staged");

    let running = install.join("bin").join(exe_name());
    std::fs::create_dir_all(running.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_loft"), &running).unwrap();
    write(&install.join("default").join("a.loft"), b"old\n");
    // The shadow: what a previous `make install` left.  Its mere EXISTENCE re-points the
    // resolver, so the contents need not differ for the update to miss.
    write(
        &install
            .join("share")
            .join("loft")
            .join("default")
            .join("a.loft"),
        b"shadow\n",
    );
    write_manifest(&install);

    write(&staged.join("bin").join(exe_name()), b"WOULD-REPLACE\n");
    write(&staged.join("default").join("a.loft"), b"new\n");
    write_manifest(&staged);
    let before = std::fs::read(&running).unwrap();

    let out = std::process::Command::new(&running)
        .args(["self-update", "--from"])
        .arg(&staged)
        .output()
        .expect("run the installed loft");
    let err = String::from_utf8_lossy(&out.stderr);

    assert!(
        !out.status.success(),
        "a shadowed installation must refuse\nstdout: {}\nstderr: {err}",
        String::from_utf8_lossy(&out.stdout)
    );
    assert!(
        err.contains("would not run what this installs"),
        "the refusal must say WHY, naming both trees: {err}"
    );
    assert!(
        err.contains("share") && err.contains("loft#1497"),
        "the refusal must name the shadowing tree and the issue: {err}"
    );
    // Nothing moved — the whole point of refusing before the write.
    assert_eq!(
        std::fs::read(&running).unwrap(),
        before,
        "a refused update must not replace the binary"
    );
    assert_eq!(
        std::fs::read(install.join("default").join("a.loft")).unwrap(),
        b"old\n",
        "a refused update must not replace the stdlib either"
    );
    assert_eq!(
        std::fs::read(
            install
                .join("share")
                .join("loft")
                .join("default")
                .join("a.loft")
        )
        .unwrap(),
        b"shadow\n",
        "and must not touch the tree it declined to update"
    );
}

/// The CONTROL for the refusal above: without the shadowing tree the SAME bundle installs.
///
/// Without this, a `self-update` that had simply stopped working would pass the refusal test.
#[test]
fn the_same_update_installs_when_no_tree_shadows_it() {
    let base = scratch("unshadowed");
    let install = base.join("install");
    let staged = base.join("staged");

    let running = install.join("bin").join(exe_name());
    std::fs::create_dir_all(running.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_loft"), &running).unwrap();
    write(&install.join("default").join("a.loft"), b"old\n");
    write_manifest(&install);

    const NEW_BINARY: &[u8] = b"WOULD-REPLACE\n";
    write(&staged.join("bin").join(exe_name()), NEW_BINARY);
    write(&staged.join("default").join("a.loft"), b"new\n");
    write_manifest(&staged);

    let out = std::process::Command::new(&running)
        .args(["self-update", "--from"])
        .arg(&staged)
        .output()
        .expect("run the installed loft");
    assert!(
        out.status.success(),
        "an unshadowed installation must still update\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(std::fs::read(&running).unwrap(), NEW_BINARY);
    assert_eq!(
        std::fs::read(install.join("default").join("a.loft")).unwrap(),
        b"new\n"
    );
}

/// `--force` is the escape hatch, and it must still say what state it leaves behind.
///
/// The pre-existing forced-install line reads *"past a manifest this bundle does not match"*,
/// which is a different reason to force and would misdescribe this one.  What the user needs
/// after forcing here is the state they are now in — a new binary over an unchanged stdlib,
/// which is the crash they asked for.
/// Falsified against its OWN perturbation, which is a different one: neutering the pre-flight
/// leaves this test passing (forcing skips it either way), and neutering the `force && shadowed`
/// arm is what fails it.  Worth stating, because a guard that only fails on its sibling's
/// perturbation is measuring the sibling.
#[test]
fn forcing_past_a_shadow_installs_and_names_the_state_it_leaves() {
    let base = scratch("forced");
    let install = base.join("install");
    let staged = base.join("staged");

    let running = install.join("bin").join(exe_name());
    std::fs::create_dir_all(running.parent().unwrap()).unwrap();
    std::fs::copy(env!("CARGO_BIN_EXE_loft"), &running).unwrap();
    write(&install.join("default").join("a.loft"), b"old\n");
    write(
        &install
            .join("share")
            .join("loft")
            .join("default")
            .join("a.loft"),
        b"shadow\n",
    );
    write_manifest(&install);

    const NEW_BINARY: &[u8] = b"WOULD-REPLACE\n";
    write(&staged.join("bin").join(exe_name()), NEW_BINARY);
    write(&staged.join("default").join("a.loft"), b"new\n");
    write_manifest(&staged);

    let out = std::process::Command::new(&running)
        .args(["self-update", "--force", "--from"])
        .arg(&staged)
        .output()
        .expect("run the installed loft");
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "--force must proceed\nstderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        std::fs::read(&running).unwrap(),
        NEW_BINARY,
        "--force must actually install"
    );
    assert!(
        said.contains("SHADOWED"),
        "a forced install over a shadow must name what it left: {said}"
    );
    assert_eq!(
        std::fs::read(
            install
                .join("share")
                .join("loft")
                .join("default")
                .join("a.loft")
        )
        .unwrap(),
        b"shadow\n",
        "and the shadowing tree is still the one that loads"
    );
}
