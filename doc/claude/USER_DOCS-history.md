<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# USER_DOCS-history.md — what the user-documentation design described and no longer holds

The working doc is [USER_DOCS.md](USER_DOCS.md).

## The `--html --debug` client and its server relay

Until 2026-11-24 a page built with `loft --html --debug[=name]` carried the Rust writer's live
tier: its compiled `main` dispatched into a parked interpreter, and the page answered `D!:`
debug frames over its host input.  `tests/wasm_debug_relay.rs` drove the whole chain headless —
an agent, through an `engine_host` server, into that browser client — for `bp`, `run`, `eval`
and `resume`, and USER_DOCS cited it as the evidence for browser breakpoints and for the REPL
panel's protocol.

The tier retired with the rustc writer (@PLN225 R4).  A page built through the C has no debug
channel, so `--html --debug` is refused at the build (`html_wasm::html_debug_is_refused`).  The
doc-page panel was never that tier: it runs the program in the interpreter's wasm build, and
`tests/wasm_entry.rs` is its evidence.
