<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN179 — reasons to reach for another tool (driven to zero)

One row per reason a port MET to prefer Python or bash over loft — recorded when met, never
guessed, struck only when the port that met it re-measures green on that axis
([README § The bar](README.md)).  `OPEN` is the plan's headline number.

**OPEN: 9**

| # | met by | axis | the reason | the fix it needs | status |
|---|---|---|---|---|---|
| 1 | `scripts/script_census` (strand 1) | performance | start-up of a `hello`, INSTALLED binary, idle box, 2026-09-30, 5 runs: `--interpret` 15–22 ms (Python 15–19, bash 5) — at the bar; the DEFAULT path (native, warm cache) 32–37 ms — 2× Python.  The from-source binary's 78–84 ms was rule 4 (program cache off), not the language | the warm-native start-up: where the ~30 ms goes (`parse_default` is 5 ms of it), and the `--interpret` pin in a script's `#!` line ([MODES.md](MODES.md)) so a short script meets the bar today | open — narrowed to the native path |
| 2 | `scripts/script_census` (strand 1) | clarity | tokenising a line needs a character loop: `split` takes one separator and `find` answers bytes against `len`'s characters | a regex-shaped split in the stdlib, or the `regex` library measured for it | open |
| 3 | `scripts/script_census` (strand 1) | behaviour | the interpreter segfaults at `OpFreeText` on `argv_call(self.result[n - 1] ?? "")` inside a struct method that also appends to `self.result`, once enough heap is behind it; `--native` runs it; four other spellings run; no extraction reproduces it yet | [loft#1773](https://github.com/loft-lang/loft/issues/1773) — the census binds the element to a local first, marked to fold back | open |
| 4 | `fuzz/seed_program_source` (strand 3, the first port) | clarity | no `copy(from, to)`: a file is copied as `write_bytes(dst, read_bytes(src) ?? [])`, two calls and a null discharge for what `cp` says in one word | a `copy` builtin beside `move`, answering `FileResult` | open |
| 5 | `fuzz/seed_program_source` (strand 3) | clarity | no recursive listing: `find … -name '*.loft'` is a hand-written `walk` over `files()` in every script that needs one (the census carries the same function) | a `files(dir, recursive)` or a `find(dir, suffix)` in the stdlib, symlink-aware | open |
| 6 | `scripts/wasm_bundle_stamp.sh` (strand 3, not yet ported) | environment | `use crypto;` cannot auto-install behind this box's TLS-intercepting proxy — loft's HTTP client trusts only its bundled roots and ignores `SSL_CERT_FILE`, where `curl` on the same box succeeds | honour `SSL_CERT_FILE` / the system root store in the registry fetch.  Interim that works: `git clone` (which the proxy allows) `loft-libs-core` and copy the package under `~/.loft/lib/<name>`; its native crate builds on first use and both backends resolve it | open — the fetch itself |
| 7 | `fuzz/seed_program_source` (strand 3) | clarity | the refusal of `{n:0>5}` names the cure (`{n:05}`) but points at the line ABOVE the format string | the diagnostic's span | open |
| 8 | `scripts/wasm_bundle_stamp` (strand 3) | clarity | no path pattern: bash's `default/*.loft` is a `list_dir` + an `ends_with` filter + a path join, four lines for one word | a glob, or a `files(dir, pattern)` in the stdlib | open |
| 9 | `scripts/wasm_bundle_stamp` (strand 3) | behaviour | no `exit(code)`: `set -e`'s "stop with status 1 on the first unreadable file" is an `assert`, whose stderr and status are loft's, not the script's | `exit(code)` (README § Strand 1) | open |

