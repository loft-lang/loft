// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I58 — Parser (two-pass recursive descent to IR): a part of src/parser/mod.rs

use super::Parser;
use crate::sandbox::parse_sandbox_config;

/// @PLN86 step 1.2 — a host designation (`fn:<name>`) tags exactly the named
/// function with its profile; other functions stay unrestricted; and the tag
/// comes from the host policy, never the source (only `fn:scripted` is
/// designated, yet `host` in the same file is untagged).
#[test]
fn fn_designation_tags_only_the_designated_function() {
    let mut p = Parser::new();
    p.set_sandbox_config(parse_sandbox_config(
        "[sandbox]\nmod-script = [\"fn:scripted\"]\n[profile.mod-script]\nallow = [\"math#read\"]\n",
    ));
    let dir = std::env::temp_dir();
    let path = dir.join(format!("plan86_designation_{}.loft", std::process::id()));
    crate::file_access::write(&path, "fn scripted() { }\nfn host() { }\n").unwrap();
    p.parse(path.to_str().unwrap(), false);
    let _ = crate::file_access::remove_file(&path);
    assert!(
        p.diagnostics.level() < crate::diagnostics::Level::Error,
        "unexpected parse errors: {:?}",
        p.diagnostics.lines()
    );
    let scripted = p.data.def_nr("n_scripted");
    let host = p.data.def_nr("n_host");
    assert_ne!(scripted, u32::MAX, "scripted fn registered");
    assert_ne!(host, u32::MAX, "host fn registered");
    assert_eq!(p.def_sandbox_profile(scripted), Some("mod-script"));
    assert_eq!(p.def_sandbox_profile(host), None);
}
