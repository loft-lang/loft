<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN179 — reasons to reach for another tool (driven to zero)

One row per reason a port MET to prefer Python or bash over loft — recorded when met, never
guessed, struck only when the port that met it re-measures green on that axis
([README § The bar](README.md)).  `OPEN` is the plan's headline number.

**This table is the hand-kept SEED.**  The register's home is the tracker: each row
becomes an issue carrying an `axis:` label (LABELS.md § axis), and from then on
`make script-reasons` renders this file from those issues and nothing here is edited by
hand.  A row below that has no issue number is one not yet filed.

**OPEN: 9**

| # | met by | axis | the reason | the fix it needs | status |
|---|---|---|---|---|---|
| 1 | `scripts/script_census` (strand 1) | performance | start-up of a `hello`, INSTALLED binary, idle box, 2026-09-30, 5 runs: `--interpret` 15–22 ms (Python 15–19, bash 5) — at the bar; the DEFAULT path (native, warm cache) 32–37 ms — 2× Python.  The from-source binary's 78–84 ms was rule 4 (program cache off), not the language | where the interpreter's ~15 ms goes before the first statement — `LOFT_TIMING=1` on the installed binary: `parse_default` 5–10 ms for 748 stdlib defs on a WARM run, scopes 3, lints 2, codegen 3 — against the 7 ms a whole `cat \| sha256sum` pipeline takes; the warm-native path's ~30 ms on top of that; and the `--interpret` pin in a script's `#!` line ([MODES.md](MODES.md)) | open — the stdlib load on a warm run is the item |
| 2 | `scripts/script_census` (strand 1) | clarity | tokenising a line needs a character loop: `split` takes one separator and `find` answers bytes against `len`'s characters | a regex-shaped split in the stdlib, or the `regex` library measured for it | open |
| 3 | `scripts/script_census` (strand 1) | behaviour | the interpreter segfaults at `OpFreeText` on `argv_call(self.result[n - 1] ?? "")` inside a struct method that also appends to `self.result`, once enough heap is behind it; `--native` runs it; four other spellings run; no extraction reproduces it yet | [loft#1773](https://github.com/loft-lang/loft/issues/1773) — the census binds the element to a local first, marked to fold back | open |
| 4 | `fuzz/seed_program_source` (strand 3, the first port) | clarity | no `copy(from, to)`: a file is copied as `write_bytes(dst, read_bytes(src) ?? [])`, two calls and a null discharge for what `cp` says in one word | a `copy` builtin beside `move`, answering `FileResult` | open |
| 5 | `fuzz/seed_program_source` (strand 3) | clarity | no recursive listing: `files()` answers one level, so `find … -name '*.loft'` is a hand-written walk — the tree already carries one in `tools/indexer/src/scan.loft` (`walk_plan_dirs`), one in each of the two gallery scripts, one in the census and one in this port, none symlink-aware before `is_symlink` existed | a `files(dir, recursive)` or a `find(dir, suffix)` in the stdlib, symlink-aware, replacing the five copies | open |
| 6 | `scripts/wasm_bundle_stamp.sh` (strand 3, not yet ported) | environment | `use crypto;` cannot auto-install behind this box's TLS-intercepting proxy — loft's HTTP client trusts only its bundled roots and ignores `SSL_CERT_FILE`, where `curl` on the same box succeeds | honour `SSL_CERT_FILE` / the system root store in the registry fetch.  Interim that works: `git clone` (which the proxy allows) `loft-libs-core` and copy the package under `~/.loft/lib/<name>`; its native crate builds on first use and both backends resolve it | open — the fetch itself |
| 7 | `fuzz/seed_program_source` (strand 3) | clarity | the refusal of `{n:0>5}` names the cure (`{n:05}`) but points at the line ABOVE the format string | the diagnostic's span | open |
| 8 | `scripts/wasm_bundle_stamp` (strand 3) | clarity | no path pattern: bash's `default/*.loft` is a `list_dir` + an `ends_with` filter + a path join, four lines for one word | a glob, or a `files(dir, pattern)` in the stdlib | open |
| 9 | `scripts/wasm_bundle_stamp` (strand 3) | behaviour | no `exit(code)`: `set -e`'s "stop with status 1 on the first unreadable file" is an `assert`, whose stderr and status are loft's, not the script's | `exit(code)` (README § Strand 1) | open |

