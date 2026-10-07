// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! `loft doc` writes where it says, and only when it resolved something (loft#911).
//!
//! The command reads as `loft doc <library>` and is used that way, but its argument
//! was a PATH only.  A library name is not a directory, so `loft doc graphics` fell
//! through to the empty-manifest branch: it CREATED `./graphics/doc/` in whatever
//! directory the user happened to be standing in, found no `src/` to read, and
//! reported "0 API sections" for a package with 119 documented `pub fn`s.  The
//! printed path was relative, so `graphics/` looked like part of the project — one
//! such tree was swept into an unrelated repository by a later `git add -A`.
//!
//! Three rules close it, and each has a test here: a name that resolves to nothing
//! is an ERROR that creates nothing; an installed library's docs go to loft's own
//! doc cache instead of the CWD; and the reported path is absolute.

use loft::file_access as fa;
use std::path::PathBuf;

fn loft_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_loft"))
}

fn tmp_root(tag: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("loft_911_{tag}_{}", std::process::id()));
    let _ = fa::remove_dir_all(&root);
    fa::create_dir_all(&root).expect("mkdir root");
    root
}

/// An unresolvable name must not leave a directory behind.  This is the whole
/// mechanism of the reported litter: the old code took a name it could not resolve,
/// treated it as a relative path, and `create_dir_all`'d it into existence.
#[test]
fn an_unresolvable_name_creates_nothing_and_fails() {
    let root = tmp_root("noresolve");
    let out = loft::platform::process::harness_command(loft_bin())
        .current_dir(&root)
        .args(["doc", "definitely_not_a_package_zzz"])
        .output()
        .expect("run loft doc");
    assert!(
        !out.status.success(),
        "an unresolvable name must fail, not succeed quietly"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("neither a directory nor an installed package"),
        "the refusal must say what it looked for; got:\n{stderr}"
    );
    assert!(
        !fa::exists(root.join("definitely_not_a_package_zzz")),
        "no directory may be created for a name that resolved to nothing"
    );
    let left: Vec<_> = fa::read_dir(&root)
        .expect("read root")
        .into_iter()
        .map(|e| e.os_name().unwrap_or_default())
        .collect();
    assert!(
        left.is_empty(),
        "the working directory must be untouched, found: {left:?}"
    );
    let _ = fa::remove_dir_all(&root);
}

/// A real package directory still documents in place, and the API sections are
/// extracted from its `src/*.loft` — the half the reporter never saw, because the
/// name never resolved to a package with a `src/` at all.
#[test]
fn a_package_directory_documents_its_own_api() {
    let root = tmp_root("pkgdir");
    let pkg = root.join("mylib");
    fa::create_dir_all(pkg.join("src")).expect("mkdir pkg");
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"mylib\"\nversion = \"0.2.0\"\n",
    )
    .expect("write manifest");
    fa::write(
        pkg.join("src/mylib.loft"),
        "// Add two numbers together and answer the sum.\n\
         pub fn add_two(a: integer, b: integer) -> integer { a + b }\n",
    )
    .expect("write src");

    let out = loft::platform::process::harness_command(loft_bin())
        .current_dir(&root)
        .arg("doc")
        .arg(&pkg)
        .output()
        .expect("run loft doc");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "loft doc failed: {stdout}");
    assert!(
        !stdout.contains("0 API section"),
        "a documented `pub fn` must produce an API section; got:\n{stdout}"
    );
    // The reported path is absolute, so it cannot be mistaken for a project subdir.  It is
    // compared in the ONE spelling the CLI prints (`file_access::plain_canonical`): on a
    // Windows runner `temp_dir()` is the 8.3 short name (`RUNNER~1`) and `canonicalize()` the
    // verbatim `\\?\C:\…`, and the printed plain long form matched neither.
    let printed_dir = loft::file_access::plain_canonical(&pkg.join("doc"))
        .to_string_lossy()
        .to_string();
    assert!(
        stdout.contains(&printed_dir),
        "the absolute output path must be printed ({printed_dir}); got:\n{stdout}"
    );
    let index = fa::read_to_string(pkg.join("doc/index.html")).expect("index.html");
    assert!(
        index.contains("API Reference"),
        "the index must link the API it extracted"
    );
    let api: String = fa::read_dir(pkg.join("doc"))
        .expect("read doc")
        .into_iter()
        .filter(|e| e.file_name().is_some_and(|n| n.starts_with("api-")))
        .filter_map(|e| fa::read_to_string(&e).ok())
        .collect();
    assert!(
        api.contains("add_two"),
        "the extracted API must carry the function's signature"
    );
    let _ = fa::remove_dir_all(&root);
}

/// A doc comment is TEXT on the API page: its `<` and `&` are shown, not read as markup, and a
/// `` `span` `` is code.  The page joined the comment's lines into one raw `<p>`, so
/// `vector<T>` reached the browser as an unknown `<T>` tag and the reader saw "vector".
#[test]
fn a_doc_comment_is_shown_as_text_on_the_api_page() {
    let root = tmp_root("pkgesc");
    let pkg = root.join("esclib");
    fa::create_dir_all(pkg.join("src")).expect("mkdir pkg");
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"esclib\"\nversion = \"0.1.0\"\n",
    )
    .expect("write manifest");
    fa::write(
        pkg.join("src/esclib.loft"),
        "// Answers a vector<T> & a <b>bold</b> claim, via `first(v)`.\n\
         pub fn first_of(x: integer) -> integer { x }\n",
    )
    .expect("write src");
    let out = loft::platform::process::harness_command(loft_bin())
        .current_dir(&root)
        .arg("doc")
        .arg(&pkg)
        .output()
        .expect("run loft doc");
    assert!(out.status.success(), "loft doc failed");
    let api: String = fa::read_dir(pkg.join("doc"))
        .expect("read doc")
        .into_iter()
        .filter(|e| e.file_name().is_some_and(|n| n.starts_with("api-")))
        .filter_map(|e| fa::read_to_string(&e).ok())
        .collect();
    assert!(
        api.contains(
            "vector&lt;T&gt; &amp; a &lt;b&gt;bold&lt;/b&gt; claim, via <code>first(v)</code>"
        ),
        "the doc must be escaped text with its span as code; got:\n{api}"
    );
    let _ = fa::remove_dir_all(&root);
}

/// `-o <dir>` puts the output exactly where it is told — the escape hatch for the
/// case where neither "beside the source" nor the doc cache is what is wanted.
#[test]
fn out_flag_redirects_the_output() {
    let root = tmp_root("outflag");
    let pkg = root.join("mylib");
    fa::create_dir_all(pkg.join("src")).expect("mkdir pkg");
    fa::write(
        pkg.join("loft.toml"),
        "[package]\nname = \"mylib\"\nversion = \"0.2.0\"\n",
    )
    .expect("write manifest");
    fa::write(
        pkg.join("src/mylib.loft"),
        "// Answer a constant.\npub fn one() -> integer { 1 }\n",
    )
    .expect("write src");
    let elsewhere = root.join("elsewhere");

    let out = loft::platform::process::harness_command(loft_bin())
        .current_dir(&root)
        .arg("doc")
        .arg(&pkg)
        .arg("-o")
        .arg(&elsewhere)
        .output()
        .expect("run loft doc");
    assert!(
        out.status.success(),
        "loft doc -o failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        fa::exists(elsewhere.join("index.html")),
        "-o must place the pages in the named directory"
    );
    assert!(
        !fa::exists(pkg.join("doc")),
        "-o must not also write beside the source"
    );
    let _ = fa::remove_dir_all(&root);
}
