// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN179 strand 1 — `is_symlink(path)` names the LINK, where `is_dir` / `is_file`
//! answer for its target.  The gap was measured by the first loft script of that
//! plan: a directory walk had nothing to ask and followed every link, listing the
//! scripts under the tracked `docs -> doc` link twice.  Both backends, one cell,
//! and the expected line is written down so two wrong-but-identical answers cannot
//! pass.

mod common;

use common::cross_mode::run_cross_mode_expect;

#[cfg(unix)]
#[test]
fn is_symlink_names_the_link_not_its_target() {
    let root = std::env::temp_dir().join(format!("loft_symlink_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("d")).expect("create dir");
    std::fs::write(root.join("f"), "x").expect("create file");
    std::os::unix::fs::symlink(root.join("d"), root.join("l")).expect("create link");
    let p = |name: &str| root.join(name).display().to_string();
    let body = format!(
        r#"
    fn test() {{
        link = is_symlink("{l}");
        dir = is_symlink("{d}");
        plain = is_symlink("{f}");
        through = is_dir("{l}");
        missing = is_symlink("{m}");
        print("link={{link}} dir={{dir}} file={{plain}} is_dir_through_link={{through}} missing={{missing}}\n");
    }}
    "#,
        l = p("l"),
        d = p("d"),
        f = p("f"),
        m = p("missing"),
    );
    run_cross_mode_expect(
        "is_symlink",
        &body,
        "link=true dir=false file=false is_dir_through_link=true missing=false\n",
    );
    let _ = std::fs::remove_dir_all(&root);
}
