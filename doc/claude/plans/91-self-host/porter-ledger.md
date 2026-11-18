# What aids and what baffles the porter — the tester's ledger (@PLN91)

Kept by the tester on the owner's request: each time a tool, a report or a habit of the testing
loop visibly HELPED the porting agent, or left it guessing, or let something land wrong that
the next tip had to repair, one row — with what would have prevented the repair.  The point is
the last column: a cause that shows twice becomes a change to the loop.

Read the patterns first; the rows are the evidence.

## Patterns, and what the loop does about each

| pattern | seen | prevention | state |
|---|---|---|---|
| A prediction names a program the gate never compares (`@EXPECT_ERROR`, or the Rust front end refuses it) | 3 | each tip's `noimage.txt` (the reason beside each program); `noimage.py <tip> NAME…` checks a prediction's list before a cycle is spent | built |
| A new port refuses, or drops, functions that agreed (found one tip later) | 2 | a try-gate: the porter names a commit, the tester answers drops and differs before it is promoted | offered |
| Main moves under the port (a Rust change, new suite programs) and the differ shows only after the rebase | 1 | `maincheck.sh`: when `origin/main` moves and the port does not hold it, the last tip is gated MERGED with main at main's suite pin, in a worktree of its own; one `MAINCHECK` line and a report | built; its full path runs first when main next moves |
| An instrument's ranking sends the porter to a non-problem | 1 | a new count is read on three examples by the tester before it is reported as a priority | habit |
| The watcher gates a commit in the middle of a multi-commit push | 1 | the watcher starts a cycle only for a tip that has stood for a minute | built |
| The tester's commits wait for a gap between cycles, or land mid-cycle | 3 | `sync.sh` (the cycle, its push and the baseline job merge what is on GitHub first) and `land.sh` (the tester's commits replayed onto the branch's tip from its own worktree); the real gate is asked to measure the commit the cycle GATED | built |
| The porter keeps a script of its own for something the loop has | 1 | each new tool is announced with a ready command for the porter's current tip | habit |

## The rows

`+` aided, `?` baffled or guessed, `!` went wrong first and was repaired.

| date | tip | | what happened | what would have prevented the repair |
|---|---|---|---|---|
| 11-18 | b9a917675 | ! | A new refusal ("an `assert_eq` at a narrow integer width") dropped four pinned `assert_eq` instances; removed again one tip later | a try-gate on the commit |
| 11-18 | 658595e8b | + | The definition-level ranking (`deflevel.py`) became "the right list to work from" | — |
| 11-18 | 2eb106879 | ? | "a struct field of this type" had no struct name and no line; the porter had to read programs | the refusal raised where the declaration is known (the porter added the kind and field later) |
| 11-18 | a7a74bb9a | ? | The watcher gated the middle commit of a three-commit push; an interim report for a tip nobody promoted | the debounce above |
| 11-18 | 45ef5df05 | ? | Two of eight predicted programs were `noimage`: never compared | `noimage.txt` |
| 11-18 | 3e824b4de | ! | The in-place `map` / `filter` port refused generic templates over `T` that agreed before: three function drops, repaired one tip later | a try-gate on the commit |
| 11-18 | c48e2a253 | ! | After the rebase onto main: one differ from a Rust change that came with main (a store `else` on an append) | gating the last tip against the new main before the rebase |
| 11-18 | 5c83f4c82 | + | The first-pass trace found a differ the corpus does not hold (`map` over a vector literal), with its cause on pass 1; fixed one tip later | — |
| 11-18 | 257f2cd69 | + | The C-Var differ: the porter's guess (a first-pass order) was confirmed and located in one step by `fn-end`; fixed in the next tip | — |
| 11-18 | f260caf9b | ? | The tester reported `expected=` as the largest first-pass group (3192 of 7012); it was a modelling difference, and the porter spent a look on it | the three-examples habit above |
| 11-18 | a03a1c25f | ? | The porter's own probe showed a differ the gate did not (H-Spent/2283): an `@EXPECT_ERROR` program, skipped by the gate | `noimage.txt`, and the reason beside each row |
| 11-18 | a03a1c25f | ? | The tester told the porter a total would not change with the comparison change; it did (1251 → 2341); corrected in the next message | the tester measures before stating what a change does to a count |
| 11-18 | f74d7e42b | + | `pass1_mint.py`'s rows: three causes found and fixed in one program (a body delivered twice, a work ref at every returned view, `_ret_root_N` pass 2's alone) | — |
| 11-18 | b22f9c998 | + | The first-pass variables showed Rust does nothing at a `return` inside a loop on pass 1 | — |
| 11-18 | 8a48b7b0d | + | `record-return.txt` (first pass of refused functions) chose the next batch: 48 functions with one cause | — |
| 11-18 | 8a48b7b0d | ? | Until `elab_trace.py` the porter diffed `fn-end` lines with a script of its own | the announce-with-a-command habit |
| 11-18 | — | ! | The tester's `cgen_tail.loft` did nothing inside the generator: a private struct `Nm` met `cgen_split.loft`'s (loft defect, fixed: a keyed collection is looked up by its element's registered name) | fixed at the cause |
| 11-18 | — | ! | The cycle script dropped the tester's four commits when the porter's branch was rebased; put back by hand | the carry step's check, to repair with the fetch-and-merge change |
| 11-18 | 727c7bc75 | ! | The tester landed a commit into the cycle's worktree 48 seconds into a cycle (harmless that time: no Rust in it) | the fetch-and-merge change |
