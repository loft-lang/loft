<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# APART_VALUES-history.md — the record of [APART_VALUES.md](APART_VALUES.md)

## How the prize was priced (moved on 2026-11-24)

The third row's function was hand-written in the emitted Rust of the rustc writer (retired with
`@C170`) as a stack array `[i64; 4]` with the same overflow-checked additions: 29 ns → under
1 ns, the sum unchanged (`hand_price.sh`, three alternated rounds).  The instances were first
built in that writer, where a distinct Rust type was the backstop of requirement 2 (a store
routine handed an apart value was a `rustc` type error) and a bare Rust `Vec` was the
representation instance 3 measured against.
