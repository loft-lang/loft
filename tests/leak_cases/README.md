<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Cross-backend leak cases

Each `*.loft` file here is a self-contained leak case: it defines
`fn test()` (no `main` — the harness appends one) using only built-ins
and its own struct/enum declarations, so it needs no library on the
load path.

The harness in [`tests/leak_cases.rs`](../leak_cases.rs) runs **every**
file through both the interpreter (`--interpret`) and the native
Rust-codegen backend (`--native`, with `LOFT_NATIVE_LEAK_CHECK=1` so the
generated binary runs the same exit-time store-leak report the
interpreter prints unconditionally).  The folder a file lives in is its
**expected** leak behaviour:

| Folder | interp | native | meaning |
|---|---|---|---|
| `clean/` | no leak | no leak | the contract — a regression on either backend fails the suite |
| `known_<id>_both/` | LEAK | LEAK | open bug; the file asserts the leak still reproduces.  When fixed, move the file to `clean/` |
| `known_<id>_native/` | no leak | LEAK | open bug, native-only; move to `clean/` when fixed |

There are currently **no open-bug folders** — all known leaks are fixed
and their cases live in `clean/`: @P297 / @P298 (2026-05-21), the three
`i490_*` cases ([#490]) and `i491_file_ctor_receiver.loft` ([#491], both
2026-07-03).  Recreate a `known_<id>_both/` or `known_<id>_native/` folder
when a new leak is found that can't be fixed immediately — the folder
names are matched by suffix in `folder_expect` (`tests/leak_cases.rs`), so
no harness change is needed.

**Why this exists:** before @P297, the `tests/leak.rs` suite ran only on
the interpreter — native had no exit-time leak check at all, so
native-only leaks (like @P298) went undetected.  This directory is the
permanent both-backend layer: drop a `.loft` file in `clean/` and it is
guarded on native too, for free.

Run the native half (heavy — `rustc` per file, so `#[ignore]`d):

```bash
cargo test --release --test leak_cases -- --ignored
```

The interpreter half runs in the default `cargo test` path.

[@P297]: ../../doc/claude/PROBLEMS.md
[@P298]: ../../doc/claude/PROBLEMS.md
[#490]: https://github.com/loft-lang/loft/issues/490
[#491]: https://github.com/loft-lang/loft/issues/491
