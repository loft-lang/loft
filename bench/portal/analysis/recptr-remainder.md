# The record address's growth declines — counted, classified, priced, not built

An analysis of `(R-RecPtr)`'s most frequent refusal, "the remainder may grow a store", taken
2026-10-20 on x86-64 over the nineteen bench programs with the decline census
(`make decline-census`) and two hand-priced forms (`hand_price.sh`).  Its verdict is a
NEGATIVE one, recorded so it is not re-derived: the refusal count is the largest of any rule,
and lifting it moves no row that is over its bar.

## What the census counted, and what it counted wrongly

The first census read 431 views refused for growth.  293 of them had no fusable field read
or write at all — a view whose only use is a text write or a hand to a callee gains nothing
from an address — and the verdict had tested growth before use, so every one of those read
as a growth refusal.  The verdict now asks for a fusable use first (`view_extent_verdict_inner`);
the refusal that costs something is 180 sites, and `LOFT_TRACE_RECPTR=1` prints a `class`
line under each with the view's chain end, each mover's, the apart verdict, a buffer op when
one blocks, and `use-after-growth`.

| cause (126 unique sites) | count | what it is |
|---|---:|---|
| a frame-local call buffer's guarded mint, `if b == null { OpDatabase(b) } else { OpClear(b) }` | 34 | neither exempt (the exemption covers argument buffers) nor a mover: a precision gap |
| a push into the view's own vector | ~30 | true growth of the view's store |
| a text written into the view's own record (`OpSetText`), or a callee that may | ~40 | true growth: a record's text lives in its store |
| movers proven apart on one statement, blocked by another | 5 | — |
| `use-after-growth=no`: every address-served use precedes the first growth | 47 (9 distinct functions; 38 are two stdlib file methods) | the ORDERED clause would admit these: the keyed find-or-insert idiom (`found = h[k]; if found == null { h += [...] } else { found.n += 1 }`) is the user-side shape |

## What the two prices said

The ordered clause and the buffer-mint exemption were each priced by hand on the row they
would serve, the emitted Rust edited by script to the held-address form the rule would
emit, both binaries pinned to one core and run interleaved, hashes equal:

| row | ratio | sites lifted | current ns/op | hand form ns/op | verdict |
|---|---:|---|---:|---:|---|
| `17 panel_build` | 5.82× | `tools` (4 scalar writes a call) and `p` (1 scalar read) under the buffer mints | 5.32–5.58 M | 5.62–5.67 M | no gain |
| `16 composite_hash` | 4.18× | `found` in the update arm (1 read, 1 write a probe) under the ordered clause | 0.80–0.92 M | 0.79–0.88 M | within noise, ≤ 3 % |

A held address saves one store resolution per scalar access, one to two nanoseconds; these
routines spend their time in the keyed probe, the text building and the record construction
around those accesses.  The decline count pointed at the rule; the price says the count is
not the cost.

## What stands

- The reason order in the verdict (no fusable use first) and the `class` trace line are
  kept: the census reads true, and the trace answers "would the ordered clause admit this
  view" for any program without building the clause.
- Neither the ordered clause nor the buffer-mint exemption is built.  Both are sound as
  stated above; a routine that would pay them back is one with many scalar accesses to a
  found record per probe, which no measured row is.  Build them when such a row appears,
  against this page's method.
- The keyed class's cost is the probe itself (`OpGetRecord`, `OpGetHashLong` against a
  Rust `HashMap`): a runtime question, `keyed.md`'s subject, not a rewrite's.
