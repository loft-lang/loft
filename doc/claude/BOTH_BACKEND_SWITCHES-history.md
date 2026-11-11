<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# BOTH_BACKEND_SWITCHES-history.md — past measurements of the both-backend switches

The working doc is [BOTH_BACKEND_SWITCHES.md](BOTH_BACKEND_SWITCHES.md).

## `(R-Place)` — the drawing bench's parse row

Measured 2026-09-16 on the drawing bench's parse row: one store fewer per line and the copy
gone, and a WASH on every counter (`perf stat`: instructions, cycles and cache misses equal
within noise) because the record relocated there carries no heap on that scene.

## Store confinement across sibling blocks — before and after

**Store confinement across sibling blocks (default-ON since 2026-08-21, both backends):** a
local reassigned across sibling `if`/`else if`/`match` arms used to keep EVERY arm's store
alive to scope exit, so the watermark grew with the number of reassignment SITES rather than
with how many of them run — a 16-site function peaked at 20 stores whichever single arm was
taken. `recover_backer` confines each block's store to its block: a flat **5** at 2, 4, 8 and
16 sites.

## Exit-vector placement — the cbor decoder's bench

measured on the cbor decoder's bench: decode 34.7 ms → 26.2 ms per op, and stores per
`check_request` 20 → 7.

## `(R-Place)`'s loop clause — the cbor decoder's bench

Measured on the cbor decoder's bench (`--ship`, the tuxedo box): decode 42.6 ms → 34.7 ms per op.

## Return-field placement — pluginabi's `check_request`

Measured on pluginabi's `check_request` driver (`--ship`, the tuxedo box): 0.34 s → 0.33 s per
40 rounds; the copy census of `return p.a` 1 → 0 per call.

## Placed call answers — the drawing bench's parse row

Measured on the drawing bench's parse row: the consumer spells this nowhere — the emission is
byte-identical under the switch — so the gain is structural.

## The free footer — `fronds`

Measured: `fronds` −7.7 % (the § V-u arena's churn), and the class it retires is the
`coalesce_free` cliff PERFORMANCE.md § V-j P2 first measured at 29.5 % of a shared-arena row.

## Lazy free — pluginabi's `check_request`

pluginabi's `check_request` −6 %: the free-tree walks were a quarter of the row.

## `(R-LiteralWalk)` — mesh3d's `mesh_to_floats`

mesh3d's `mesh_to_floats`, whose per-triangle `[t.a, t.b, t.c]` was a buffer reset, three appends
and three element reads through the store before the rewrite.

## The join buffer witness — the consumer's `smooth`

the consumer's `smooth` −26 %.

## `(R-Rebind)`'s own-buffer clause — zttext's insert loop

zttext insert loop 5.01 → 5.95 ms per op with the switch.

## `(R-VecCopy)` — zttext's `insert_text`

zttext `insert_text` 25.4 → 5.95 ms per op.

## `(R-FormatAppend)` — html and zttext

html `escape_html` 10.3× → 2.6× of Rust, zttext `materialise` 24× → 6.9×, `flow_layout_full`
107× → 15×.

## `(R-SliceCopy)`'s byte clause — cbor `encode_bytes`

cbor `encode_bytes` 94.7× → 43.1×; 2.9 → 0.4 ns a byte on the probe.

## The field fill with the block repeat — `lock_curved`

together with the block repeat `lock_curved` 4.2× → 2.2×.

## Slices (`LOFT_NO_SLICE_COPY`, `LOFT_NO_FOREIGN_VIEW`)

Both default-ON since 2026-09-28 (@PLN174 F4a, F4b).  The block copy: forty 200 000-element
slices 0.13 → 0.03 s.  @PLN174 F5 extended `LOFT_NO_FOREIGN_VIEW` to a `#native` bridge's
whole vector answer.

## A value branch of calls (`LOFT_NO_JOIN_BUFFER_WITNESS`)

Before the pairing, the branch paired nothing and every iteration minted and freed a store;
with it, the consumer's `smooth` ran 26 % faster.

## `(R-Place)` in a loop

Measured on the cbor decoder's bench: decode 34.7 ms → 26.2 ms per op, and stores per
`check_request` 20 → 7.
