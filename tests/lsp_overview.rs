// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

//! @PLN183 P3 — "loft: what can I use here?": the language server writes the overview as a
//! Markdown site and asks the editor to open it (`window/showDocument`).  Driven over the real
//! JSON-RPC transport, each session with its own `LOFT_HOME`, so the site, the absent registry
//! index and the stamp are this test's alone.

use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use loft::json::{self, Parsed};

struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Session {
    fn start(home: &std::path::Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_loft-lsp"))
            .env("LOFT_HOME", home)
            // Resolution never reaches the network: a library the test needs is installed
            // under its own LOFT_HOME.
            .env("LOFT_OFFLINE", "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn loft-lsp");
        let stdin = child.stdin.take().unwrap();
        let stdout = BufReader::new(child.stdout.take().unwrap());
        Session {
            child,
            stdin,
            stdout,
        }
    }

    fn send_raw(&mut self, body: &str) {
        write!(self.stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
        self.stdin.flush().unwrap();
    }

    fn request(&mut self, id: i64, method: &str, params: &str) {
        self.send_raw(&format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":"{method}","params":{params}}}"#
        ));
    }

    fn notify(&mut self, method: &str, params: &str) {
        self.send_raw(&format!(
            r#"{{"jsonrpc":"2.0","method":"{method}","params":{params}}}"#
        ));
    }

    fn recv(&mut self) -> Parsed {
        let mut content_length = 0usize;
        loop {
            let mut line = String::new();
            let n = self.stdout.read_line(&mut line).unwrap();
            assert!(n > 0, "server closed stdout before replying");
            let header = line.trim_end_matches(['\r', '\n']);
            if header.is_empty() {
                break;
            }
            if let Some(v) = header.strip_prefix("Content-Length:") {
                content_length = v.trim().parse().unwrap();
            }
        }
        let mut buf = vec![0u8; content_length];
        self.stdout.read_exact(&mut buf).unwrap();
        json::parse(&String::from_utf8(buf).unwrap()).expect("reply is valid JSON")
    }

    fn open(&mut self, uri: &str, program: &str) {
        let text = json::to_json_string(&Parsed::Str(program.to_string()));
        self.notify(
            "textDocument/didOpen",
            &format!(r#"{{"textDocument":{{"uri":"{uri}","languageId":"loft","version":1,"text":{text}}}}}"#),
        );
        let _ = self.recv(); // publishDiagnostics
    }

    fn end(mut self) {
        self.request(99, "shutdown", "null");
        let _ = self.recv();
        self.notify("exit", "null");
        let _ = self.child.wait();
    }
}

fn field<'a>(v: &'a Parsed, key: &str) -> Option<&'a Parsed> {
    match v {
        Parsed::Object(e) => e.iter().find(|(k, _, _)| k == key).map(|(_, _, val)| val),
        _ => None,
    }
}

fn text_of(v: Option<&Parsed>) -> String {
    match v {
        Some(Parsed::Str(s)) => s.clone(),
        _ => String::new(),
    }
}

fn home(tag: &str) -> std::path::PathBuf {
    let d = std::env::temp_dir().join(format!("loft_overview_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

const PROGRAM: &str = "struct Money { cents: integer }\n\
                       pub operator plus(self: Money, other: Money) -> Money { Money { cents: self.cents + other.cents } }\n\
                       fn main() {\n  m = Money { cents: 5 };\n  n = m + m;\n  println(\"{n.cents}\");\n}\n";

/// Ask for the overview at (`line`, `ch`) and answer the page the server opens.
fn overview_at(s: &mut Session, id: i64, uri: &str, line: i64, ch: i64) -> String {
    s.request(
        id,
        "workspace/executeCommand",
        &format!(r#"{{"command":"loft.overview","arguments":["{uri}",{line},{ch}]}}"#),
    );
    let mut opened = String::new();
    let mut answered = false;
    while !(answered && !opened.is_empty()) {
        let msg = s.recv();
        if text_of(field(&msg, "method")) == "window/showDocument" {
            opened = text_of(field(field(&msg, "params").unwrap(), "uri"));
            // The editor's reply: it must be taken without an answer of its own.
            let back = match field(&msg, "id") {
                Some(Parsed::Int(n)) => *n,
                _ => panic!("showDocument carries an id: {msg:?}"),
            };
            s.send_raw(&format!(
                r#"{{"jsonrpc":"2.0","id":{back},"result":{{"success":true}}}}"#
            ));
        } else {
            assert!(
                matches!(field(&msg, "result"), Some(Parsed::Null)),
                "executeCommand answers null: {msg:?}"
            );
            answered = true;
        }
    }
    opened
}

fn path_of(uri: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(uri.strip_prefix("file://").unwrap_or(uri))
}

#[test]
fn the_server_offers_the_overview_and_opens_it() {
    let home = home("open");
    let mut s = Session::start(&home);
    s.request(1, "initialize", "{}");
    let init = s.recv();
    let caps = field(field(&init, "result").unwrap(), "capabilities").unwrap();
    let commands = format!("{:?}", field(caps, "executeCommandProvider"));
    assert!(
        commands.contains("loft.overview"),
        "advertises the command: {commands}"
    );
    assert!(
        format!("{:?}", field(caps, "codeActionProvider")).contains("source.loft.overview"),
        "advertises the action kind"
    );
    s.notify("initialized", "{}");
    let uri = "file:///overview/main.loft";
    s.open(uri, PROGRAM);

    // Offered at any position when nothing narrows the kinds; absent from a quick-fix request.
    let at = r#""range":{"start":{"line":2,"character":0},"end":{"line":2,"character":0}}"#;
    s.request(
        2,
        "textDocument/codeAction",
        &format!(r#"{{"textDocument":{{"uri":"{uri}"}},{at},"context":{{"diagnostics":[]}}}}"#),
    );
    let all = format!("{:?}", s.recv());
    assert!(
        all.contains("loft: what can I use here?") && all.contains("loft.overview"),
        "{all}"
    );
    s.request(3, "textDocument/codeAction", &format!(r#"{{"textDocument":{{"uri":"{uri}"}},{at},"context":{{"diagnostics":[],"only":["quickfix"]}}}}"#));
    assert!(
        !format!("{:?}", s.recv()).contains("loft.overview"),
        "not a quick-fix"
    );
    s.request(4, "textDocument/codeAction", &format!(r#"{{"textDocument":{{"uri":"{uri}"}},{at},"context":{{"diagnostics":[],"only":["source"]}}}}"#));
    assert!(
        format!("{:?}", s.recv()).contains("loft.overview"),
        "`source` admits it"
    );

    // Off any type name: the root page, written under LOFT_HOME.
    let root = overview_at(&mut s, 5, uri, 2, 0);
    assert!(root.ends_with("/index.md"), "{root}");
    let root_path = path_of(&root);
    assert!(root_path.starts_with(&home), "under LOFT_HOME: {root}");
    let index = std::fs::read_to_string(&root_path).unwrap();
    assert!(index.contains("[@F2 — "), "lists the features: {index}");
    assert!(
        index.contains("(libraries.md)") && index.contains("(inside.md)"),
        "{index}"
    );
    let libs = std::fs::read_to_string(root_path.with_file_name("libraries.md")).unwrap();
    assert!(
        libs.contains("no registry index on this machine yet"),
        "says what is missing: {libs}"
    );

    // On the type name `Money` (line 0, char 8): that type's own page.
    let ty = overview_at(&mut s, 6, uri, 0, 8);
    assert!(ty.ends_with("/type-Money.md"), "{ty}");
    let page = std::fs::read_to_string(path_of(&ty)).unwrap();
    assert!(page.contains("# What `Money` can do"), "{page}");
    assert!(
        page.contains("`+  +=` — `operator plus"),
        "names the definition behind `+`: {page}"
    );

    // Nothing the site depends on changed: a second request rewrites nothing.
    let before = std::fs::metadata(&root_path).unwrap().modified().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    let again = overview_at(&mut s, 7, uri, 2, 0);
    assert_eq!(again, root);
    assert_eq!(
        std::fs::metadata(&root_path).unwrap().modified().unwrap(),
        before,
        "rewritten"
    );

    // The editor's reply above was taken silently: the next answer is the shutdown's own.
    s.request(8, "shutdown", "null");
    let reply = s.recv();
    assert!(
        matches!(field(&reply, "id"), Some(Parsed::Int(8))),
        "{reply:?}"
    );
    s.notify("exit", "null");
    let _ = s.child.wait();
    let _ = std::fs::remove_dir_all(&home);
}

/// The whole-catalogue guard: every entry has exactly one page whose words are the entry's own
/// (the one renderer's Markdown), and is linked exactly once — from the root, or from the
/// maintainers' page for an `@I` entry.
#[test]
fn every_catalogue_entry_has_its_page_and_one_link() {
    let pages = loft::doc_site::feature_pages();
    let text = |name: &str| {
        pages
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.text.clone())
            .unwrap_or_else(|| panic!("no page {name}"))
    };
    let lists = format!("{}{}", text("index.md"), text("inside.md"));
    let entries = loft::doc_catalogue::entries();
    assert_eq!(
        pages.len(),
        entries.len() + 2,
        "one page per entry, plus the two lists"
    );
    for e in entries {
        let file = format!("({}.md)", e.tag);
        assert_eq!(lists.matches(&file).count(), 1, "@{} linked once", e.tag);
        let page = text(&format!("{}.md", e.tag));
        let body = loft::doc_render::blocks_markdown(&e.blocks());
        assert!(
            page.contains(&body),
            "@{}: the page carries the rendered entry",
            e.tag
        );
        assert!(
            page.contains(&e.page()),
            "@{}: names where the full text lives",
            e.tag
        );
    }
}

#[test]
fn a_session_with_nothing_open_still_ends_cleanly() {
    let home = home("end");
    let mut s = Session::start(&home);
    s.request(1, "initialize", "{}");
    let _ = s.recv();
    s.end();
    let _ = std::fs::remove_dir_all(&home);
}

/// Hover over the hover text of `(line, ch)` (0-based).
fn hover_at(s: &mut Session, id: i64, uri: &str, line: i64, ch: i64) -> String {
    s.request(
        id,
        "textDocument/hover",
        &format!(
            r#"{{"textDocument":{{"uri":"{uri}"}},"position":{{"line":{line},"character":{ch}}}}}"#
        ),
    );
    let reply = s.recv();
    field(field(&reply, "result").unwrap_or(&Parsed::Null), "contents")
        .map(|c| text_of(field(c, "value")))
        .unwrap_or_default()
}

/// A library installed under `home`'s registry cache: `demo` 1.2.0, one documented function,
/// a guide.
fn install_demo(home: &std::path::Path) {
    let dir = home.join(".loft").join("registry").join("demo-1.2.0");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("docs")).unwrap();
    std::fs::write(
        dir.join("loft.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.2.0\"\n\n[library]\nentry = \"src/demo.loft\"\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("demo.loft"),
        "/// Twice the value.\npub fn dbl(x: integer) -> integer { x * 2 }\n",
    )
    .unwrap();
    std::fs::write(dir.join("docs").join("01-start.loft"), "fn main() {}\n").unwrap();
}

#[test]
fn hover_names_the_construct_the_operator_and_the_library() {
    let home = home("hover");
    install_demo(&home);
    let mut s = Session::start(&home);
    s.request(1, "initialize", "{}");
    let _ = s.recv();
    s.notify("initialized", "{}");
    let uri = "file:///hover/main.loft";
    let program = "use demo;\n\
                   struct Money { cents: integer }\n\
                   pub operator plus(self: Money, other: Money) -> Money { Money { cents: self.cents + other.cents } }\n\
                   /// Whole units in an amount.\n\
                   fn whole(m: Money) -> integer { m.cents / 100 }\n\
                   fn main() {\n  m = Money { cents: 5 };\n  n = m + m;\n  a = demo::dbl(2) ?? 0;\n  v = [1, 2];\n  b = v[1];\n  println(\"{n.cents} {a} {b}\");\n}\n";
    s.open(uri, program);

    // `??`: the entry that declares it, nothing about the neighbouring name.
    let coalesce = hover_at(&mut s, 2, uri, 8, 19);
    assert!(coalesce.starts_with("**@F2 — "), "{coalesce}");
    assert!(
        coalesce.contains("https://github.com/loft-lang/features/issues/2"),
        "{coalesce}"
    );
    // `[` on a vector: the vector entry, decided by the operand's type.
    let index = hover_at(&mut s, 3, uri, 10, 7);
    assert!(index.starts_with("**@F6 — "), "{index}");
    // `+` between two `Money`: the definition behind it, then the operator entry.
    let plus = hover_at(&mut s, 4, uri, 7, 8);
    assert!(
        plus.contains("operator plus(self: Money, other: Money) -> Money"),
        "{plus}"
    );
    assert!(plus.contains("**@F37 — "), "{plus}");
    // A library's function: its own card, then the library's — the version in use, the guide.
    let lib = hover_at(&mut s, 5, uri, 8, 12);
    assert!(
        lib.contains("fn dbl(x: integer) -> integer") && lib.contains("Twice the value."),
        "{lib}"
    );
    assert!(lib.contains("**library `demo`** 1.2.0"), "{lib}");
    assert!(
        lib.contains("lib-demo-guide.html"),
        "a guide is installed: {lib}"
    );
    // The `use` line: the library's card.
    let use_line = hover_at(&mut s, 6, uri, 0, 5);
    assert!(
        use_line.starts_with("**library `demo`** 1.2.0"),
        "{use_line}"
    );

    // Completion items are documented when shown: a keyword by its entry, a name by its doc.
    s.request(
        7,
        "completionItem/resolve",
        &format!(r#"{{"label":"match","kind":14,"data":{{"uri":"{uri}"}}}}"#),
    );
    let kw = format!("{:?}", s.recv());
    assert!(kw.contains("@F29 — "), "{kw}");
    s.request(
        8,
        "completionItem/resolve",
        &format!(r#"{{"label":"whole","kind":3,"data":{{"uri":"{uri}"}}}}"#),
    );
    let name = format!("{:?}", s.recv());
    assert!(name.contains("Whole units in an amount."), "{name}");
    s.end();
    let _ = std::fs::remove_dir_all(&home);
}
