// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `json_parse` refuses a document nested deeper than 512 arrays or objects, on both backends.
//!
//! The parser goes one call deeper per level, so the input decides how deep: two million `[`
//! ended the process with a stack overflow.  A program builds `n` opening and `n` closing
//! brackets and parses them: 3 and 512 levels parse with no error; 513 and 200,000 answer the
//! error `nested more than 512 deep`, and the program goes on to print its line.

use loft::file_access as fa;

const PROGRAM: &str = r#"fn main() {
  n = ((env_variable("N") ?? "3") as integer?) ?? 3;
  s = "";
  for _ in 0..n { s += "["; }
  for _ in 0..n { s += "]"; }
  json_parse(s);
  println("depth {n} refused {json_errors().contains("nested more than 512 deep")} clean {json_errors() == ""}");
}
"#;

#[test]
fn a_document_nested_too_deep_is_refused_not_a_crash() {
    let dir = std::env::temp_dir().join(format!("loft_json_nesting_{}", std::process::id()));
    let _ = fa::remove_dir_all(&dir);
    fa::create_dir_all(&dir).expect("scratch");
    fa::write(dir.join("deep.loft"), PROGRAM).expect("program");
    let mut backends = vec!["--interpret"];
    if loft::platform::c_runtime_posix_host() {
        backends.push("--native-c");
    }
    for backend in backends {
        for (n, want) in [
            ("3", "depth 3 refused false clean true"),
            ("512", "depth 512 refused false clean true"),
            ("513", "depth 513 refused true clean false"),
            ("200000", "depth 200000 refused true clean false"),
        ] {
            let out = loft::platform::process::harness_command(env!("CARGO_BIN_EXE_loft"))
                .args([backend, "--no-warnings", "deep.loft"])
                .current_dir(&dir)
                .env("N", n)
                .env("LOFT_TIMEOUT", "300")
                .output()
                .expect("loft runs");
            assert_eq!(
                String::from_utf8_lossy(&out.stdout).trim_end(),
                want,
                "{backend} N={n}\n{}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
    let _ = fa::remove_dir_all(&dir);
}
