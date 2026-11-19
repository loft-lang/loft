# What aids and what baffles the porter — the tester's ledger (@PLN91)

Kept by the tester on the owner's request: each time a tool, a report or a habit of the testing
loop visibly HELPED the porting agent, or left it guessing, or let something land wrong that
the next tip had to repair, one row — with what would have prevented the repair.  The point is
the last column: a cause that shows twice becomes a change to the loop.

Read the patterns first; the rows are the evidence.

## What the testing side is for

The measure of everything below is the porting agent's free working time — its session context
and its clock, both kept for porting.  A port of this kind cannot be split over two writers: the
rules interact (a variable's mint order, a shared helper, what a refusal covers), and two
sessions writing the elaborator would each break what the other stands on.  What CAN run beside
it is everything that only reads the port: running the checks, reading results, reading the
Rust front end, preparing the next topic.  So the testing side never edits the port's logic,
and answers three questions for the porter:

| question | answered by | rule |
|---|---|---|
| is this working? | the try-gate's status, and the cycle report after a promotion | automatic in both cases (clean, or issues); the porter posts and goes on, and waits for nothing |
| what is wrong with it? | the tester's analysis, when a try or a cycle shows issues | the check is automatic, the ANALYSIS is not — and since either session could do it, the tester does |
| what to do next? | one topic, already analysed, with two one-line alternates | the porter takes the first by habit, so the order is the tester's responsibility; the alternates carry real numbers and the true reason each ranks lower, so the reasoning can be seen and challenged |

How an analysis is written:

- It ends at WHAT TO CODE — the Rust decision the port does not mirror, where in the port it
  belongs, the cases it must cover and those it must leave alone — and never at code.  Writing
  it is the easy part and the porter's, who holds the surrounding code in context.
- It states facts about Rust and about the tip that was measured, not about the porter's
  working copy.
- Only conclusions that are certain are sent.  An open case is probed until it is settled, not
  listed as open; how the conclusion was reached stays out of the message.
- A count, and an explanation of a count, is measured before it is sent, and read on three
  examples first.
- The porter's own handoff is read before an order is proposed: it holds the cost side.

The standing order: stay far enough AHEAD with prepared subjects that the porter never waits
for one; FIX a defect that blocks the port or that the port would mirror; FILE every other
defect met on the way, fixed or not.

**A defect is not ported (owner: "make sure you do not reimplement bugs … as long as loft
gets better for it you can take months").**  The number of agreeing programs is not the goal,
and no cycle is short enough to justify copying a fault.  So a form the port would mirror is
JUDGED before it is carried: a rule of the language, or a defect or an accident of the Rust
front end.  Each prepared page says which, per form.  A defect is fixed in Rust first and the
port follows the fixed form; until then the port refuses the shape by name.  An accident that
changes nothing a program can observe (@C168: internal numbers) may be carried.  The tester
never advises "carry it" for a form it has not judged.

Preventing a problem is worth more than finding it sooner.  Three routines:

- **A prepared topic comes with its probe already run both ways.**  On the Rust front end, on
  both backends, before the porter sees it: a probe that fails there has found a defect the
  port would otherwise mirror faithfully.  And on the port's current build: the message says
  which cases fail today and at which line, so each costs one build and no search.
- **A Rust change in an area the port mirrors is announced before it lands**, with the
  functions whose report changes and the new form, so the port follows in the same cycle.
- **A program left one refusal from agreeing is looked at ahead of the porter.**  A try or a
  cycle that moves a program there names what the porter reaches for next; that refusal is
  prepared — analysis and probe — before it is asked for.

## Patterns, and what the loop does about each

| pattern | seen | prevention | state |
|---|---|---|---|
| A prediction names a program the gate never compares (`@EXPECT_ERROR`, or the Rust front end refuses it) | 3 | each tip's `noimage.txt` (the reason beside each program); `noimage.py <tip> NAME…` checks a prediction's list before a cycle is spent | built |
| A new port refuses, drops or crashes on what agreed or was refused (found one tip later) | 4 | a try-gate: the porter names a commit, the tester answers drops and differs before it is promoted — for any change that adds or widens a REFUSAL, not only a shared rule; and `dropped.py` keeps a drop listed past the next baseline | the try-gate needs no branch: a bundle in a comment on loft#2560 (`scripts/try_gate.sh`); `dropped.py` built |
| Main moves under the port (a Rust change, new suite programs) and the differ shows only after the rebase | 1 | `maincheck.sh`: when `origin/main` moves and the port does not hold it, the last tip is gated MERGED with main at main's suite pin, in a worktree of its own; one `MAINCHECK` line and a report | built; its full path runs first when main next moves |
| An instrument's ranking, or a count of the tester's, sends the porter to a non-problem | 4 | a new count is read on three examples before it is reported, and a count's EXPLANATION is measured before it is sent | habit, broken once since it was written |
| The watcher gates a commit in the middle of a multi-commit push | 1 | the watcher starts a cycle only for a tip that has stood for a minute | built |
| The tester's commits wait for a gap between cycles, or land mid-cycle | 3 | `sync.sh` (the cycle, its push and the baseline job merge what is on GitHub first) and `land.sh` (the tester's commits replayed onto the branch's tip from its own worktree); the real gate is asked to measure the commit the cycle GATED | built |
| A Rust decision the port must mirror has no trace line, and is learned by probe builds | 1 | a `route` line at the decision, verdict and names included, added when the porter names the selector | habit: ask each handoff which decision cost probes |
| The tester proposes an order without the porter's cost side | 1 | the porter's handoff is read first | habit |
| An analysis carries open cases, or a line composed and not copied | 2 | probe until settled; copy the trace's line | habit |
| A tool hands the porter raw output to wait for and read | 1 | every porter-facing tool posts and returns; the answer is the tester's analysis in a message | rule, from the owner |
| The porter keeps a script of its own for something the loop has | 1 | each new tool is announced with a ready command for the porter's current tip | habit |
| An instrument reports a pass it never ran (a judgment whose default arm never ran; a memory figure from a binary that never started) | 2 | every probe prints the program's own output beside each number and a path marker for the arm it judges, so a probe that did not run cannot read as a pass | adopted |

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
| 11-18 | 0320e32d1 | + | The porter's own ranking of what helped: (1) the first-pass `fn-end var` rows per function — three mint slips found in one cycle, every delivery since decided from them; (2) "which functions of a family already agree on pass 1"; (3) the verdict line with the predicted names checked | — |
| 11-18 | 0320e32d1 | + | `noimage.txt`: the porter saw before predicting that the one program its probe lifted carries `@EXPECT_ERROR` | — |
| 11-18 | 0320e32d1 | ? | Rust's return selector (`classify_reference_delivery` → `ref_return`) had no instrument: about six probe builds to learn its verdict per return site | a `route` line for the selector's verdict (added: `classify_reference_delivery=<verdict>`, a rename's work refs by name) |
| 11-18 | f74d7e42b | ! | The porter's `view_tail` ran on bodies ending in a `return` statement (a spare `__ref_N`); its own corpus listing could not see it, the program being refused elsewhere — `pass1_mint.py` did | the first pass read for refused programs too (built) |
| 11-18 | eb02bf3c4 | + | The selector's `route` line: "the line I was missing"; predictions now checked against the noimage rule before they are sent | — |
| 11-18 | dda79c069 | ? | `fn-end … ret=ref(Own)["__retbuf"]` read as "the port misses a bind" for 47 functions; the dep is an ATTRIBUTE number printed through the variable of that number, so a rename and a bind printed alike | the result's deps spelled as attributes (added: `attrs=<n>:<attribute name>`) |
| 11-18 | 3757d9dce | ! | The reasoned refusal "a field append whose item reads the field: beside a value built in place" caught `own.hs += [H { r: own.items[0] }]`, which the port built correctly before: one function drop (and a schema section beside it) | a try-gate; the porter judged the change not shared, so none was asked — a new REFUSAL is a shared rule too |
| 11-18 | 3757d9dce | ? | The tester sent four counts tallied over both sides' lines as if they were the Rust side's; withdrawn in the next message | the three-examples habit, applied to the tester's own tallies |
| 11-18 | 476ca4edd | + | The porter on the two drops: its own pre-promotion check is per PROGRAM, so a function that agreed inside a program that stays refused is invisible to it; FUNCTION DROP is the only instrument that sees it | the try-gate for any change that ADDS or widens a refusal (the porter's own conclusion) |
| 11-18 | a44780b2e | ? | A drop is listed only until the next baseline is written without it: the two would have left the report unrepaired | `dropped.py`: a definition that agreed once stays listed per tip (`dropped.txt`, and on the REPORT line) until it agrees again |
| 11-18 | 006ab959f | ! | The repair of the two drops read `wa[2]!` one line before the arity test: the elaborator panicked on a program it had refused (one differ); fixed in the next tip | a try-gate — the third repair on the field-append refusal in three tips |
| 11-18 | 353f3f752 | ? | The try-gate cannot be used: it needs a branch of the porter's own for the commit under test, and a branch is made only on the owner's explicit ask (the push of `tuxedo-pln204-try` was stopped) | the owner naming one branch for try commits; put to the owner, undecided |
| 11-18 | 353f3f752 | + | The 17 / 30 split of the `fn-end ret` rows (binds against renames), read on examples before it was counted; and the "parted at" rows for the vector `??` the porter had tried and backed out: 28 functions, the statement named | — |
| 11-18 | — | + | The try-gate without a branch: `scripts/try_gate.sh` posts the change (committed or not) as a bundle in a comment on the closed, locked mailbox issue loft#2560; the tester's poller gates it and answers there.  Replaying three real commits: it reproduces the two drops and the panic of today, each before a promotion | — |
| 11-18 | — | ! | The tester's try-gate, on its first end-to-end run: the bundle's ref name was not the one the gate fetched ("nothing to fetch"), and a join in a locale's sort order listed 19 definitions as dropped that were not | an end-to-end test before the porter is told (done: both found and fixed in it) |
| 11-18 | 62b87dbe7 | + | The `_vec_N` / `__ncc_N` "parted at" rows were "the answer": the default literal's buffer minted first on pass 1, and the vector `??` over a nested field (backed out earlier) went in; 26 of the 28 functions fixed at that statement, no first-pass drop from a shared change | — |
| 11-18 | 62b87dbe7 | ? | The porter will not run `try_gate.sh` unasked: it publishes a comment under the owner's account, which only the owner can grant; the tester's own instruction from the owner does not carry to another session | the owner's word to the porter's session (put to the owner; the porter's handoff records it) |
| 11-18 | 62b87dbe7 | ? | The tester explained a count with "about half", unmeasured, and corrected it one message later (22 of the 28 were refused functions, outside the headline) — the fourth withdrawn figure of the day | no explanation of a count is sent before the command that measures it has run |
| 11-18 | — | ? | The try-gate as first built waited and printed the gate's raw output: the porter running the gate at one remove, and still reading it | the owner's rule: a hand-off pays only if the tester ANALYSES and the porter works on meanwhile — `try_gate.sh` returns at once, the issue gets a receipt, the analysis is a message |
| 11-18 | f5a9ca6a4 | + | The first real try (3d6a82573): handed over, analysed by the tester, promoted with its outcome already known — the owner gave the porter the word to use it | — |
| 11-18 | f5a9ca6a4 | ? | The porter's wanted shape for a try's answer: (1) one line, clean or not; (2) each definition that stopped agreeing, with the refusal text and source line that caught it; (3) each differ with the first differing line of both sides — nothing else unless one is non-empty.  The first analysis was longer than that | the three-part answer, from the next try |
| 11-18 | f5a9ca6a4 | ? | The route coverage line ("3 of 31") is in every cycle report and has not moved a decision | left out of the report unless it changes |
| 11-18 | — | + | The owner's split for a try: the check is automatic; a CLEAN result is acknowledged automatically, one with issues goes back only with the tester's analysis of what is wrong | built: `try.sh` classifies (checked on four tries already run: two clean, the panic, the two drops), the poller answers a clean one and holds the other |
| 11-18 | — | ? | The tester held a try's "issues" answer back until its analysis was written: minutes in which the porter cannot know whether to build on the change | the owner's measure — the porter's free working time: the status is automatic in both cases, the analysis only adds to it, and nothing is waited for |
| 11-18 | — | + | The hand-off used as meant: the porter posted a try, ran its own corpus check beside it and went on to the next topic | — |
| 11-18 | — | + | The owner's reason for the split, stated: the check is automatic; the ANALYSIS of what comes up can be done by either session, so the tester does it — the porter keeps both its session context and its time for porting | an analysis goes as far as WHAT to code — the Rust decision the port does not mirror, where it belongs in the port, the cases it must cover and those it must leave alone — and not to the code, which is the easy part and the porter's |
| 11-18 | b8ad5ed2c | + | The first "next topic" sent analysed to what-to-code (the staging clause of an in-place literal): "exact" — both named programs agreed on the first build; "saves me a probe-and-read round per item; keep sending it" | — |
| 11-18 | f44e5f731 | ? | That topic was the wrong FIRST pick: the porter's own handoff already listed larger items with their Rust side worked out, and the tester had not read it before ranking | the porter's handoff is read before an order is proposed |
| 11-18 | f44e5f731 | ? | The tester's first table for the record-tail item showed 13 functions with no Rust decision line: a function's TAIL routes print with the lexer on the next item's first token (`<next line>:3`), so they had been given to the wrong function.  Caught against Rust's own return trace before it was sent | a tail's routes are attributed up to the next top-level item's first token; a table is checked against a second instrument before it is sent |
| 11-18 | f44e5f731 | ? | The tester spelled Rust's first-pass result line wrongly in an analysis (`["__ref_K"]` for `["__retbuf"] attrs=N:__ref_K`); corrected in the next message | a line the porter will compare against is copied from the trace, not composed |
| 11-18 | f44e5f731 | ? | An analysis ended with five cases "not probed"; two of them, once probed, contradicted what the Rust comments suggested (a default reading `$` and building a value is not refused; an empty-vector default in a generic struct is not hoisted) | probe until settled, then send the conclusions only |
| 11-18 | b8ad5ed2c | + | Four tries, all clean, each acknowledged without the tester; the porter ran its own corpus check beside each and went on to the next topic | — |
| 11-18 | b8ad5ed2c | ? | The porter still ran its own corpus listing before each promotion: the try's answer gave a COUNT of newly agreeing programs, not their names, nor what still blocks the programs a change was meant to lift | the answer lists what the change moved (`try_moves.py`): the programs that newly agree, and per program that lost a refusal, what is left; a lifted program that differs comes back analysed |
| 11-18 | 5db484adb | + | The probe per topic: `record-tail-default.loft` agreed whole after "one build per remaining case instead of a corpus hunt"; the porter's corpus listing is gone, replaced by "what it moved" | — |
| 11-18 | 5db484adb | + | A probe run on the Rust front end before it was sent found a silent wrong answer there (loft#2561: a `-> S?` result read out of a held store is an empty record where its source is absent) — which the port would have mirrored | the first routine above |
| 11-18 | 075b8a290 | ! | A try was answered against an older tip (its own base's cycle had not finished), reporting the base's gains as the change's; and a hand-run try shared the worktree with the mailbox's | a try waits until its base is gated and names the tip it compared against; tries take a lock |

| 11-19 | fb394a832 | ! | The tester's judgment of 11-18 ("the null vector default is usable", eight cases, both backends) was vacuous: every case discharged `src.split(',')?`, and `split` of a null text answers an EMPTY vector, so the default arm never ran.  Probing a record receiver found that default silently wrong on main (loft#2567: every append lost; bound once per loop pass, it accumulates) | a judgment cell first proves that its path RUNS: the subject really absent (a function that answers `null`), the default's own value asserted |
| 11-19 | fb394a832 | ? | The porter's refusal ("a call's nullable vector") followed the one-line shape the analysis named (`<call>?`) and also caught six shapes that agreed: the boundary was a receiver with deps, not a call | an analysis gives a shape as its measured BOUNDARY (what is in and what is out), never as one example |
| 11-19 | ddbe4db03 | + | The boundary sent as a table, before the Rust fix: the porter narrowed by syntax to exactly it and promoted clean on the next try | — |
| 11-19 | 93e7a5201 | + | The try's one differ (a nested literal's text element typed with its format buffer's dep) was judged a Rust ACCIDENT before the port mirrored it: the type depended on which item came first.  Fixed in Rust (d693191cf); the porter dropped the try that had copied the flat case | — |
| 11-19 | d20efc493 | ? | The porter held a local stack on a question built from a "free-call twin" whose Rust form, dumped on the current build, matched the field-chain form exactly: the difference to judge was not in Rust | an analysis question names the program behind every comparison; the tester dumps both twins on the current Rust before judging |
| 11-19 | 2cbc247dc | + | The pick "a `&` argument that is a link of another type", sent measured on a port build with the one-line change (2 agree, lib/process exposed with its Rust form judged): the try lifted exactly the three predicted programs, the porter carrying the held `__place_N` form the analysis named | — |
| 11-19 | 6327f9206 | ? | The tip's one differ was in the tester's own new guard (loft#2567's `handed_back`): a guard written for Rust is also a corpus program the port meets, and nobody had gated it on the port before it landed | a new tests/scripts guard is run through the port's gate (`elab_gate.py --show` with the last tip's tools) before it lands, and its differ, if any, sent with the landing |
| 11-19 | 9cb9f0c37 | ! | The tester's C memory figures for the keyed replace and the struct-enum default were an artifact: `--out` makes a directory, so the timed command was `time` failing to run it ("944 KB" every time).  The conclusions held once remeasured (1.8 MB flat, 1x to 100x), and the correction was sent.  The second vacuous instrument in one day, after the null-vector judgment whose default arm never ran | every number a probe reports is printed beside the program's own output, so a probe that did not run cannot read as a pass; promoted to the Patterns table as "an instrument proves it ran" |
| 11-19 | 9cb9f0c37 | ? | The tester prepared the struct-enum `?` default while the porter's own try of it was already in the mailbox | the mailbox's open tries and the porter's queue are read before a subject is chosen |
| 11-19 | 28c911e5f | ! | The tester's analysis said a struct-enum `?` over an element ALWAYS reads through the nullable getter, generalised from literal-index probes; the porter's one-function-per-row table showed a variable index taking the read-through-tag `if` form.  Ruled: the form follows the element read's TYPE (a variable index types it `Sh?`), both forms correct out of range on three backends.  The porter withdrew the try rather than port the wrong rule | an analysis that states a form for a CLASS varies the class's axis (here the index: literal, local, parameter, loop variable) before it says "always" |
| 11-19 | a0688644c | + | The porter held a shape refused and asked for a judgment ("the discharge is evaluated TWICE"), naming the check to run; measured, it was a silent-wrong Rust defect on main (loft#2571: a record append to a field at a computed place ran the index twice per item), fixed before the port mirrored it.  The tester's own ruling on the same shape an hour earlier had looked at the absent case and not at the evaluation count | a ruling on a WRITE through a computed place counts how often the place's addressing runs (an index that counts its calls), beside the value |
| 11-19 | 4b17a78b7 | ! | The tester's analysis of 1450d said the `(Col-Insert-Absent)` test is "chosen by the local's type"; the porter, reading Rust's sites, found two arms (`absent_append_mint` tests `OpRefIsNull`; `vector_db`'s guarded mint `OpVectorIsNull` where the local names no store yet) and ported the split | a form stated as a rule names the Rust function that emits it, read there, not inferred from one dump |
| 11-19 | 5912b6842 | + | Preparing a subject (lib/lexer.loft's `for pt in self.tokens[l].possible ?? []`) found a silent-wrong Rust defect on main before the port met it (loft#2573: a sorted/index walk re-evaluated a call source at every step); the subject went to the porter in Rust's corrected form | — |
| 11-19 | e43620208 | ! | loft#2573's fix was announced as "11 programs move, all refused by the port, so no differ"; the next cycle (its oracle is loft30-gate with the tip merged) dropped a PINNED function, `2484-…::n_s_ret`, which agreed inside a refused program | a Rust fix's corpus check compares the FUNCTIONS that agree (the latest `fn_verdicts.tsv` `agree` rows of each moved program, def by def), not program verdicts, and names each drop to the porter BEFORE it lands |
| 11-19 | a6c6e201d | ! | Two tester analyses were stated wider than their probes: the vector-field literal's head quirks (read off two functions, both parameter roots) and c268's cause 2 ("the tail block keeps the then arm's type", true only of parameter-view merges). Each cost a try round; the third cause (c268's cause 3) was a porter heuristic in place of Rust's `rebuild_call_deps` | a form or rule handed over names the axes it was measured on (root kind, read/no read, arm kind) and is probed across each before it is sent; where Rust has a named rule, the analysis names the function, never a heuristic that fits the failing cases |
| 11-19 | ca4065330 | ! | The tester reported a gate-E "blind spot" (a nested-tuple text write absent from the defs dump) and retracted it minutes later: its own `grep -v __work` noise filter had removed exactly the statements that name a work text | a "missing statement" claim is checked on the UNFILTERED dump before it is reported; a noise filter is for reading, never for judging absence |
