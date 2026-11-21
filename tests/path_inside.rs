// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `@FR-Path-Inside` — a path from elsewhere stays inside its root.
//!
//! loft opens whatever path it is given, so a path a program did not write itself — a
//! request's, an archive member's — is held against the directory it was meant for before it
//! is used.  `path_inside(root, path)` answers where the path lies under the root, or null when it
//! would lie outside.
//!
//! Varied: the path — plain, with `.`, with a `..` that stays inside, with one that leaves,
//! absolute, with a drive, and each leaving form spelled with `\`, which loft reads as a
//! separator on every platform.  Held fixed: the root `public`, then three other roots for
//! the join.
//!
//! Values hand-computed, by walking each path left to right from the root:
//!
//! | path | answer |
//! |---|---|
//! | `images/logo.png` | `public/images/logo.png` |
//! | `a/../b` | `public/b` (down one, up one, down one) |
//! | `./a/./b/` | `public/a/b` |
//! | empty | `public` |
//! | `a/b/../../c` | `public/c` |
//! | `../secret`, `a/../../secret`, `a/b/../../../c` | null (above the root) |
//! | `a/../../a/b` | null (above the root, whatever follows) |
//! | `/etc/passwd` | null (absolute) |
//! | `C:/windows` | null (a drive) |
//! | `..\secret`, `a\..\..\secret` | null (`\` separates too) |
//!
//! The program asserts each and prints `path_inside: 16 held`; it fails on a build without
//! `path_inside` (the name does not resolve) and on one whose walk misses a row.

use loft::file_access as fa;

const PROGRAM: &str = r#"fn at(root: text, path: text) -> text {
  found = path_inside(root, path);
  if found == null { "null" } else { "{found}" }
}

fn holds(root: text, path: text, want: text) {
  got = at(root, path);
  assert(got == want, "path_inside(\"{root}\", \"{path}\") is {got}, not {want}");
}

fn main() {
  holds("public", "images/logo.png", "public/images/logo.png");
  holds("public", "a/../b", "public/b");
  holds("public", "./a/./b/", "public/a/b");
  holds("public", "", "public");
  holds("public", "a/b/../../c", "public/c");
  holds("public", "../secret", "null");
  holds("public", "a/../../secret", "null");
  holds("public", "a/b/../../../c", "null");
  holds("public", "a/../../a/b", "null");
  holds("public", "/etc/passwd", "null");
  holds("public", "C:/windows", "null");
  holds("public", "..\\secret", "null");
  holds("public", "a\\..\\..\\secret", "null");
  // The join: a root with its own trailing separator, the file system's root, no root.
  holds("public/", "x", "public/x");
  holds("/", "x/y", "/x/y");
  holds("", "x", "x");
  println("path_inside: 16 held");
}
"#;

#[test]
fn a_path_from_elsewhere_stays_inside_its_root() {
    let dir = std::env::temp_dir().join(format!("loft_path_inside_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch");
    fa::write(dir.join("inside.loft"), PROGRAM).expect("program");
    let mut backends = vec!["--interpret"];
    if loft::platform::c_runtime_posix_host() {
        backends.push("--native-c");
    }
    for backend in backends {
        let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
            .args([backend, "--no-warnings", "inside.loft"])
            .current_dir(&dir)
            .env("LOFT_TIMEOUT", "300")
            .output()
            .expect("loft runs");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim_end(),
            "path_inside: 16 held",
            "{backend}\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let _ = fa::remove_dir_all(&dir);
}
