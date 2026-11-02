<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# COPY_DIAGNOSTICS — history

## Open items closed 2026-11-01 (loft#2342)

- **The later-use search did not follow paths.**  A use in an arm exclusive with the copy's,
  and a read after the variable is reassigned — across a loop's back edge included — counted
  as "still used after this point", so the advice fired where a move is legal and its second
  fix named a line that does not use the value.  Each copy row now carries its node and the
  lint asks the lease liveness pass, which follows paths.  The pass also learned the
  `OpDistinctStore` release guard as scope scaffolding (for the operand it releases).
- **Copies no costlier than a move were reported** — a plain enum value or a small record with
  no heap field (`NdVar { n }`), placed twice; over half of a large program's output.  Ruled:
  a copy whose type has no heap part anywhere beneath it is silent when it lands in an
  existing structure (the slot is inline, a move writes the same bytes); copied into a whole
  variable's fresh store it still allocates and keeps its notice.  This reversed loft#1190's
  "a plain `Recorded` keeps its notice".
- **Fix lines that did not hold at the site.**  "Build the value in place" was offered
  unconditionally and "drop the later use" named any later use.  Now the notice fires only
  with a rewrite usable at its site — the later use moved above the copy, or a dead one
  dropped — and says nothing for a necessary copy.  The self-hosted elaborator went from 89
  notices to 0: 52 flat, 21 with no path reading the source again, 16 necessary.
