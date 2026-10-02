// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN183 P2 — the REPL's documentation commands.
//!
//! The guard the plan names: every catalogue entry is reachable from `:features`, and `:doc`
//! on it shows the same words as its page.  Beside it, the lookups (`:doc` on a construct, a
//! function, a library), `:libs` and `:api` over a fixed index so the answer does not depend
//! on this machine's cache, `:ops` on a built-in and on a program's type, and running an
//! example in a scratch session.

use loft::doc_catalogue::{self, GROUPS, MAINTAINERS_GROUP};
use loft::repl::ReplSession;
use loft::repl_doc;
use std::io::Write as _;
use std::process::{Command, Stdio};

/// The words a reader sees, Markdown's punctuation and the terminal's decoration removed:
/// backticks, emphasis and heading marks, list bullets, fences, and the underline the terminal
/// draws under a heading.
fn words(s: &str) -> Vec<String> {
    s.split_whitespace()
        .filter(|w| !w.starts_with("```"))
        .map(|w| {
            w.trim_matches(|c| matches!(c, '`' | '*' | '#'))
                .replace('`', "")
        })
        .filter(|w| !w.is_empty() && !w.chars().all(|c| c == '-'))
        .collect()
}

/// A page's body as published in `doc/features/<tag>.md`: the generator's header removed, and
/// the `<!-- keys -->` comment, which is data and not text.
fn page_body(tag: &str) -> Option<String> {
    let page = std::fs::read_to_string(format!("doc/features/{tag}.md")).ok()?;
    let title_line = page.find("\n# ")?;
    let after_title = page[title_line + 1..].split_once('\n')?.1;
    Some(
        after_title
            .lines()
            .filter(|l| !l.trim_start().starts_with("<!--"))
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

#[test]
fn every_catalogue_entry_is_reachable_from_features() {
    let overview = format!(
        "{}\n{}",
        repl_doc::features("", repl_doc::WIDTH),
        repl_doc::features(MAINTAINERS_GROUP, repl_doc::WIDTH)
    );
    let missing: Vec<&str> = doc_catalogue::entries()
        .iter()
        .filter(|e| !overview.contains(&format!("@{} ", e.tag)))
        .map(|e| e.tag.as_str())
        .collect();
    assert!(
        missing.is_empty(),
        "not reachable from :features: {missing:?}"
    );
    for (_, title) in GROUPS {
        assert!(overview.contains(title), "group `{title}` is not shown");
    }
    // The maintainers' group stays closed until asked for.
    assert!(!repl_doc::features("", repl_doc::WIDTH).contains("@I57 "));
}

#[test]
fn doc_on_an_entry_shows_the_words_of_its_page() {
    let mut bad = Vec::new();
    let mut checked = 0;
    for e in doc_catalogue::entries() {
        let Some(body) = page_body(&e.tag) else {
            bad.push(format!("{}: no page doc/features/{}.md", e.tag, e.tag));
            continue;
        };
        let shown = repl_doc::entry_text(e, repl_doc::WIDTH);
        // The entry's own text, between its title line and the `Full text:` footer.
        let start = shown.find("\n\n").map_or(0, |i| i + 2);
        let end = shown.rfind("\nFull text:").unwrap_or(shown.len());
        let (got, want) = (words(&shown[start..end]), words(&body));
        if got != want {
            let at = got
                .iter()
                .zip(&want)
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(want.len()));
            bad.push(format!(
                "{}: differs at word {at}: shown {:?} / page {:?}",
                e.tag,
                got.get(at..(at + 6).min(got.len())),
                want.get(at..(at + 6).min(want.len()))
            ));
        }
        checked += 1;
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
    assert!(checked > 100, "only {checked} entries compared");
}

#[test]
fn a_construct_resolves_to_the_entry_that_owns_it() {
    for e in doc_catalogue::entries() {
        for k in &e.keys {
            let found: Vec<&str> = repl_doc::find_entries(k)
                .iter()
                .map(|f| f.tag.as_str())
                .collect();
            assert_eq!(found, vec![e.tag.as_str()], "key `{k}`");
        }
    }
    for (spelling, tag) in [
        ("??", "F2"),
        ("match", "F29"),
        ("@F124", "F124"),
        ("F125", "F125"),
    ] {
        let found: Vec<&str> = repl_doc::find_entries(spelling)
            .iter()
            .map(|f| f.tag.as_str())
            .collect();
        assert!(
            found.contains(&tag),
            "`{spelling}` → {found:?}, expected {tag}"
        );
    }
    let slices = repl_doc::features("slice", repl_doc::WIDTH);
    assert!(
        slices.contains("@F6 ") && slices.contains("@F125 "),
        "{slices}"
    );
}

#[cfg(feature = "registry")]
fn fixture_index() -> loft::registry_index::RegistryIndex {
    let json = r#"{"schema_version":1,"updated":"2026-10-02","packages":{
      "geom":{"description":"Points and shapes.","categories":["graphics"],"versions":{
        "0.2.0":{"url":"u","sha256":"0","size":1,"loft":">=2026.1.0","published":"2026-09-01","api":[
          {"sig":"pub fn area(r: Rect) -> float","doc":"The area of `r`."},
          {"sig":"pub fn width(r: Rect) -> float","doc":"How wide `r` is."}]}}},
      "old":{"description":"Before the api field.","categories":[],"versions":{
        "0.1.0":{"url":"u","sha256":"0","size":1,"loft":">=2026.1.0","published":"2026-01-01"}}}}}"#;
    loft::registry_index::parse_index(json).expect("fixture index")
}

#[cfg(feature = "registry")]
#[test]
fn libs_and_api_read_the_index_on_this_machine() {
    let idx = fixture_index();
    let libs = repl_doc::libs(
        Ok(&idx),
        &[("geom".into(), "0.1.0".into())],
        repl_doc::WIDTH,
    );
    assert!(
        libs.contains("graphics\n  geom") && libs.contains("installed 0.1.0"),
        "{libs}"
    );
    assert!(libs.contains("other\n  old"), "{libs}");
    let api = repl_doc::api(Ok(&idx), "geom", "area", repl_doc::WIDTH);
    assert!(
        api.starts_with("geom 0.2.0 — 2 public item(s), 1 with `area`"),
        "{api}"
    );
    assert!(
        api.contains("pub fn area(r: Rect) -> float\n  The area of `r`."),
        "{api}"
    );
    assert!(!api.contains("width"), "{api}");
    assert!(repl_doc::api(Ok(&idx), "old", "", repl_doc::WIDTH).contains("does not record"));
    assert!(repl_doc::api(Ok(&idx), "nope", "", repl_doc::WIDTH).contains("no library `nope`"));
    let offline = repl_doc::libs(
        Err("no registry index on this machine yet"),
        &[],
        repl_doc::WIDTH,
    );
    assert_eq!(offline, "no registry index on this machine yet\n");
}

#[test]
fn ops_and_doc_answer_from_the_session() {
    let mut s = ReplSession::new("default").expect("load stdlib");
    let int = s.ops_text("integer");
    assert!(
        int.contains("operator compare(self: integer, other: integer) -> Ordering"),
        "{int}"
    );
    assert!(int.contains("Numeric") && int.contains("Ordered"), "{int}");
    assert!(int.contains("[ ]\n    none\n"), "{int}");
    let _ = s.eval("struct Money { cents: integer }");
    let _ = s.eval("operator plus(self: Money, other: Money) -> Money { Money { cents: self.cents + other.cents } }");
    let money = s.ops_text("Money");
    assert!(
        money.contains("+  +=          operator plus(self: Money, other: Money) -> Money"),
        "{money}"
    );
    assert!(
        money.contains("Addable") && !money.contains("Ordered"),
        "{money}"
    );
    assert!(money.contains("named method (@F114)"), "{money}");
    let split = s.doc_text("split", "default");
    assert!(
        split.contains("fn text.split(self: text, separator: character) -> vector<text>"),
        "{split}"
    );
    assert!(s.doc_text("??", "default").starts_with("@F2 — "));
    assert!(
        s.doc_text("no_such_thing_here", "default")
            .starts_with("nothing named")
    );
}

#[test]
fn an_example_runs_in_a_scratch_session() {
    let mut child = Command::new(env!("CARGO_BIN_EXE_loft"))
        .arg("repl")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn loft repl");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(b"x = 7;\n:doc @F124 run\nx + 1\n")
        .expect("write");
    let out = child.wait_with_output().expect("wait");
    let stdout = String::from_utf8_lossy(&out.stdout);
    // The example's own output, then the user's session untouched by its definitions.
    assert!(stdout.contains("15.50 9.50 9.00 -3.00"), "{stdout}");
    assert!(
        stdout.contains("8"),
        "the session's `x` survives the scratch run: {stdout}"
    );
}
