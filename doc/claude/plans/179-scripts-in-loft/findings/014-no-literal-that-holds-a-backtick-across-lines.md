# A multi-line literal cannot contain a backtick, and a `"…"` literal cannot span lines
axis: clarity
met-by: tests/dump_ignored_tests (strand 3) — the eleven-line header the original holds in one triple-quoted string
status: fixed
fix: an escape for a backtick inside a backtick literal, or a `"…"` literal that may span lines — either lets a block of text be written as one literal whatever it contains
ref: macos-scripting (2026-10-08) — `` \` `` in the lexer's one escape handler, so both scanners of a backtick literal and the `"…"` form take it; guarded by tests/scripts/014-a-backtick-literal-holds-a-backtick.loft (one line, across lines after a hole, the other quote form)
probe: 014.probe.loft
expect: refused
checked: 647c14b83
holds: no

The baseline's header quotes code in backticks (`#[ignore = "..."]`, `make release-checklist`) across eleven lines. Python holds it in one `"""…"""`; loft has two literal forms and neither fits: the backtick literal spans lines but cannot contain a backtick, and the double-quoted literal takes `\"` but is refused at the first line break. The port prints eleven `print("…\n")` calls instead, each with its quotes escaped — the one place the twin's clarity verdict went against loft on a string rather than on a missing helper.
