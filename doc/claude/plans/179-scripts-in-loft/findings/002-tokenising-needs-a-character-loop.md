# Tokenising a line needs a character loop
axis: clarity
met-by: scripts/script_census (strand 1)
status: fixed
fix: a regex-shaped split in the stdlib, or the `regex` library measured for it — the library: `split_on(pattern)` is the split, `capture_all(pattern)` the words of a line, each one call
ref: measured 2026-10-15 over every Python and bash file of the tree (287 files, 40491 lines): the census tokenizer written over `capture_all` — one pattern yields the words, the literals, `$(` and single punctuation, the quote / `$(`-depth / argv state runs over the TOKENS — agrees with the character loop on every line (the three that differed were the loop's own escape reading, `\\"` taken for an escaped quote, fixed in the census the same day) and regenerates WORKLIST.md unchanged.  It is no shorter (61 non-comment lines against 59: the state IS the tokenizer, the character accumulation was a third of the loop) and slower (2312 ms against 1022 interpreted, 1615 against 907 native, the per-line `capture_all` records), so the census keeps its loop; what the finding named — a line's words without a character loop — is one call now
probe: 002.probe.loft
expect: refused
checked: 52bab212
holds: no

tokenising a line needs a character loop: `split` takes one separator and `find` answers bytes against `len`'s characters
