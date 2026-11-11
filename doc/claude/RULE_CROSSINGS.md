# Rule crossings — the ledger of rule pairs already verified together

A formal rule guarded alone and its neighbour guarded alone say nothing about the two
together, and most defects the rule-led walk finds sit in such a crossing
([STABILITY_METHOD.md § Where two rules meet](STABILITY_METHOD.md)).  The walk's queue,
`python3 scripts/rule_tags.py pairs [--unstated]`, is computed from the rules and the suite, so
every agent that runs it is handed the same pairs.  This ledger is what stops a pair that was
already built, hand-computed and found to hold from being built again.

The rows live in [RULE_CROSSINGS-history.md](RULE_CROSSINGS-history.md): each one is a dated
record of a crossing that was done, which is history by
[DOC_CONTRACT.md](DOC_CONTRACT.md)'s rule, and the table there is read by the tools.

## What the ledger does

- `rule_tags.py pairs` and `pairs --unstated` leave out every pair whose row says `holds` or
  `fixed loft#N`, and print how many they left out.  `--include-verified` lists them as well.
- A row whose verdict is `open loft#N` is listed in the ledger and NOT skipped: the pair still
  owes work.
- `python3 scripts/crossings.py check` fails on a row that names an undefined rule or an
  unknown verdict; `crossings.py list` prints the rows.
- `scripts/matrix_axes.py` measures composition-AXIS values over the corpus files, not rule
  pairs.  A crossing reaches it through the suite cell the row names, which joins the corpus.

## What earns a row

A row is a claim that the pair was EXERCISED, so it carries what a reader needs to trust or
redo it:

1. **The pair** — two `@FR-` tags, the rule the cells were written against first.
2. **The verdict** — `holds` (every cell answered what the rules say), `fixed loft#N` (a
   defect was found and fixed, with a cell), `open loft#N` (found, not fixed yet), or
   `disjoint — <why>`: no program can make both rules act on one site, and the verdict names
   the decline in the code that keeps them apart.  A `disjoint` row's **How** gives the probes
   that reached each rule and the point where the other declined, and its **Guard** is the
   cell that would fail if that decline moved; `crossings.py check` refuses one without a why.
3. **Validated** — the date and the `main` sha the probes ran on.
4. **How** — the cells, the backends (the interpreter and `--native-c` at least,
   CLAUDE.md § Debugging policy), and the channel each cell reads: value, length, leak.
   `scripts/probe-matrix <dir> --backend both` is the harness; every cell has a hand-computed
   `@EXPECT` and the directory a `@CONTROL` cell that must fail.
5. **Axes crossed** — the values the cells moved, in `matrix_axes.py`'s words where one
   applies (element type, source position, statement context, nullability).  This is the
   part that tells the next walker what was NOT tried: a pair verified only at scalar
   elements is not verified at records.
6. **Guard** — the suite cell that keeps the crossing true.  A crossing no loft program can
   observe (allocator layout in a file only the runtime opens, an image written by another
   release) is guarded by the Rust test that does, named with its path and test function.  A passing probe that is not
   graduated into a cell is remembered only by this row, and the row cannot fail when the
   code regresses: graduate the probes, then write the row.

A row is never a reason to skip a crossing the row does not cover.  Where a later walk
crosses the same pair along a new axis, it extends the row's **Axes crossed** and **Guard**
cells instead of adding a second row.

## When a row stops counting

- A fix that changes a site enforcing either rule (`rule_tags.py sites @FR-<rule>`) and
  breaks the row's guard has broken the crossing: the suite says so, and the row's verdict
  goes back to `open loft#N` until it holds again.
- A rule that is renamed or retired fails `crossings.py check`; rename the row with it.
