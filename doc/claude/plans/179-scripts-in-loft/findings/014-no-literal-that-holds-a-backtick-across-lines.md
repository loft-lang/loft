# A multi-line literal cannot contain a backtick, and a `"…"` literal cannot span lines
axis: clarity
met-by: tests/dump_ignored_tests (strand 3) — the eleven-line header the original holds in one triple-quoted string
status: open
fix: an escape for a backtick inside a backtick literal, or a `"…"` literal that may span lines — either lets a block of text be written as one literal whatever it contains
ref: 
probe: 014.probe.loft
expect: refused
checked: 6a26d8ca9
holds: yes

The baseline's header quotes code in backticks (`#[ignore = "..."]`, `make release-checklist`) across eleven lines. Python holds it in one `"""…"""`; loft has two literal forms and neither fits: the backtick literal spans lines but cannot contain a backtick, and the double-quoted literal takes `\"` but is refused at the first line break. The port prints eleven `print("…\n")` calls instead, each with its quotes escaped — the one place the twin's clarity verdict went against loft on a string rather than on a missing helper.
