// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//! `@PLN206 P6` — a library's loft code never resolves to a rustc-built binary: it runs as a
//! hosted C unit, or interpreted where no C compiler is found.  A stub `cargo` and `rustc` first
//! on `PATH` (and named by `CARGO` / `RUSTC`) log every call; the cells must leave the log empty
//! and answer right.  A library that declares a native library of its own in Rust
//! (`[library] native`, `[native] crate`) is refused on every backend (@PLN225), with no call.
//!
//! @falsified-at: none — the route is removed by this change, so the receipt is the tree before
//!   it (2026-11-23): `LOFT_LIB_BACKEND=rust`, a run with no C compiler and a library declaring
//!   a `[native] crate` each called the stub `rustc` (the generated rustc artifact).
//!   CHANNEL: the stub's log.  WITNESS: "rustc was asked to build a library".
//!   The refusal's name, the tree before this change: WITNESS "its library, its library".

// approved exemption: the stub `cargo` and `rustc` are shell scripts, which Windows does not run by bare name; Windows substitute: the route they watch is removed from the source, which every platform builds, and `registry_e2e`'s consumer runs a library compiled with no rustc on Windows CI
#![cfg(unix)]

use loft::file_access as fa;
use loft::platform::process::harness_command;
use std::path::{Path, PathBuf};

fn have_cc() -> bool {
    harness_command("cc")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A scratch tree: the stubs in `bin/`, their log, the packages under `lib/`.
struct Scratch {
    root: PathBuf,
}

impl Scratch {
    fn new(tag: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("loft-rustc-free-{tag}-{}", std::process::id()));
        let _ = fa::remove_dir_all(&root);
        let bin = root.join("bin");
        fa::create_dir_all(&bin).expect("bin");
        for tool in ["cargo", "rustc"] {
            let stub = bin.join(tool);
            fa::write(
                &stub,
                format!(
                    "#!/bin/sh\necho \"{tool} $*\" >> '{}'\nexit 1\n",
                    root.join("calls.log").display()
                ),
            )
            .expect("stub");
            let _ = harness_command("chmod").arg("+x").arg(&stub).status();
        }
        Scratch { root }
    }

    fn package(&self, name: &str, manifest_extra: &str, src: &str) {
        let pkg = self.root.join("lib").join(name);
        fa::create_dir_all(pkg.join("src")).expect("package dir");
        fa::write(
            pkg.join("loft.toml"),
            format!(
                "[package]\nname = \"{name}\"\nversion = \"0.1.0\"\n\n[library]\nentry = \"src/{name}.loft\"\n{manifest_extra}"
            ),
        )
        .expect("manifest");
        fa::write(pkg.join(format!("src/{name}.loft")), src).expect("source");
    }

    fn program(&self, src: &str) -> PathBuf {
        let p = self.root.join("prog.loft");
        fa::write(&p, src).expect("program");
        p
    }

    /// Run `prog` with `args` and `env`, the stubs first on `PATH`: (status ok, stdout, stderr).
    fn run(&self, args: &[&str], prog: &Path, env: &[(&str, &str)]) -> (bool, String, String) {
        let bin = self.root.join("bin");
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let mut cmd = harness_command(env!("CARGO_BIN_EXE_loft"));
        cmd.args(args)
            .arg("--lib")
            .arg(self.root.join("lib"))
            .arg(prog)
            .env("PATH", path)
            .env("CARGO", bin.join("cargo"))
            .env("RUSTC", bin.join("rustc"))
            .env("LOFT_NO_CACHE", "1")
            .env_remove("LOFT_LIB_BACKEND")
            .env_remove("LOFT_NO_RUSTC");
        for (k, v) in env {
            cmd.env(k, v);
        }
        let out = cmd.output().expect("run loft");
        (
            out.status.success(),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn calls(&self) -> String {
        fa::read_to_string(self.root.join("calls.log")).unwrap_or_default()
    }

    /// Did the run build a C unit for the package: its `.c` and the shared library compiled
    /// from it (the `.c` is written before the compiler is asked, so it alone proves nothing).
    fn built_c_unit(&self, name: &str) -> bool {
        let entries =
            fa::read_dir(self.root.join("lib").join(name).join("native-auto")).unwrap_or_default();
        let has = |ext: &str| {
            entries
                .iter()
                .any(|e| fa::has_extension(e.os_spelling(), ext))
        };
        has("c") && has(loft::platform::dll_extension())
    }

    fn clear(&self, name: &str) {
        let _ = fa::remove_dir_all(self.root.join("lib").join(name).join("native-auto"));
        let _ = fa::remove_file(self.root.join("calls.log"));
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fa::remove_dir_all(&self.root);
    }
}

const TALLY: &str =
    "pub fn tally(n: integer) -> integer {\n  t = 0;\n  for i in 0..n { t += i % 5; }\n  t\n}\n";
/// By hand: `i % 5` over `0..1000` is 200 rounds of 0 + 1 + 2 + 3 + 4 = 2000.
const USE_TALLY: &str = "use counts;\nfn main() {\n  println(\"{counts::tally(1000)}\");\n}\n";

#[test]
fn a_librarys_loft_code_never_resolves_to_a_rustc_built_artifact() {
    if !have_cc() {
        eprintln!("skipped: no cc on PATH");
        return;
    }
    let s = Scratch::new("cells");
    s.package("counts", "", TALLY);
    let prog = s.program(USE_TALLY);
    // a. The default: a C unit, no rustc.
    let (ok, out, err) = s.run(&["--interpret"], &prog, &[]);
    assert!(ok && out == "2000\n", "a: {out}{err}");
    assert!(
        s.built_c_unit("counts"),
        "a: the library runs as a C unit: {err}"
    );
    assert_eq!(s.calls(), "", "a: rustc was asked to build a library");
    // b. The old opt-out names no backend any more: still the C unit, no rustc.
    s.clear("counts");
    let (ok, out, err) = s.run(&["--interpret"], &prog, &[("LOFT_LIB_BACKEND", "rust")]);
    assert!(ok && out == "2000\n", "b: {out}{err}");
    assert_eq!(s.calls(), "", "b: rustc was asked to build a library");
    // c. No C compiler and no unit: interpreted, no rustc.
    s.clear("counts");
    let (ok, out, err) = s.run(
        &["--interpret"],
        &prog,
        &[("LOFT_CC", "/nonexistent/loft-test-cc")],
    );
    assert!(ok && out == "2000\n", "c: {out}{err}");
    assert!(!s.built_c_unit("counts"), "c: nothing was compiled");
    assert_eq!(s.calls(), "", "c: rustc was asked to build a library");
    // d. A library that also declares a Rust crate for its natives is refused (@PLN225), and
    //    nothing is built for it.
    s.package("crated", "\n[native]\ncrate = \"crated-native\"\n", TALLY);
    let prog = s.program("use crated;\nfn main() {\n  println(\"{crated::tally(1000)}\");\n}\n");
    let (ok, out, err) = s.run(&["--interpret"], &prog, &[]);
    assert!(
        !ok && err.contains(loft::manifest::RUST_NATIVE_REFUSED),
        "d: the Rust crate is refused: {out}{err}"
    );
    assert!(!s.built_c_unit("crated"), "d: nothing was compiled");
    assert_eq!(s.calls(), "", "d: rustc was asked to build a library");
}

/// A library whose native part is a hand-written Rust cdylib is refused on every backend at
/// its manifest, naming the cure (@PLN225), and nothing calls cargo or rustc.
#[test]
fn a_rust_only_native_part_is_refused_by_its_librarys_name() {
    let s = Scratch::new("refuse");
    s.package(
        "rusty",
        "native = \"rusty_native\"\n",
        "pub fn add_one(x: integer) -> integer;\n#native \"rusty_add_one\"\n\npub fn plus_two(x: integer) -> integer {\n  add_one(add_one(x))\n}\n",
    );
    let prog = s.program("use rusty;\nfn main() {\n  println(\"{rusty::plus_two(40)}\");\n}\n");
    for backend in ["--interpret", "--native"] {
        let (ok, _, err) = s.run(&[backend], &prog, &[("LOFT_TIMEOUT", "120")]);
        assert!(!ok, "{backend}: a Rust-only native part is refused");
        assert!(
            err.contains(loft::manifest::RUST_NATIVE_REFUSED) && err.contains("rusty/loft.toml"),
            "{backend}: the refusal names the library's manifest and the cure: {err}"
        );
        assert_eq!(s.calls(), "", "{backend}: cargo or rustc was called");
    }
}
