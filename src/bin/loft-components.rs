// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
// @I74 — CDylib extension loader: the prebuilt parts loft ships beside its binary

//! `loft-components` — build the components loft ships beside its binary (@PLN205): the
//! prebuilt libraries behind a package's `[native] component = "<name>"`, such as `libloft_net`
//! (the web library's HTTP, TLS and WebSocket natives).  `loft` never builds one itself.
//!
//! ```sh
//! loft-components [--out DIR] [NAME…]
//! ```
//!
//! Without `--out` the libraries land in `<this binary>/../components/`, where a `loft` built
//! beside it looks (`target/debug/loft` → `target/components/`); `make install` passes
//! `<prefix>/lib/loft/components`.  Without names, every component is built.  A component that
//! is already current is kept.  Prints nothing on success.

fn main() {
    let mut out: Option<std::path::PathBuf> = None;
    let mut names: Vec<String> = Vec::new();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--out" => out = args.next().map(std::path::PathBuf::from),
            "-h" | "--help" => {
                println!(
                    "usage: loft-components [--out DIR] [NAME…]\n\
                     Builds loft's components (known: {}) into DIR, by default\n\
                     <this binary>/../components.",
                    loft::c::component::NAMES.join(", ")
                );
                return;
            }
            _ => names.push(a),
        }
    }
    let Some(out) = out.or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|e| e.parent()?.parent().map(|p| p.join("components")))
    }) else {
        eprintln!("loft-components: cannot tell where this binary is; pass --out DIR");
        std::process::exit(2);
    };
    if names.is_empty() {
        names = loft::c::component::NAMES
            .iter()
            .map(|n| (*n).to_string())
            .collect();
    }
    let mut failed = false;
    for name in &names {
        if let Err(why) = loft::c::component::build(name, &out) {
            eprintln!("loft-components: {why}");
            failed = true;
        }
    }
    if failed {
        std::process::exit(1);
    }
}
