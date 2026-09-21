# Performance analyses — where the work stands, and how to resume it

The portal (`doc/claude/PERF_PORTAL.md`, `make perf-portal`) says WHICH classes are slow.
The files here say WHY, price what could be done about it, and record what was built.
@PLN158; the rewrites' rules are `doc/claude/formal/rewrites.md`.

| file | class | state |
|---|---|---|
| `keyed.md` | keyed (3.5×) | L1–L7 BUILT; what is left is store-format and data-structure work, priced there |
| `records.md` | record-field, record-build | analysed and **R1–R7 BUILT** (§ Built has the table and what the building found) |
| `vector-build.md` | vector-build (7.5×, the worst class) + four levers beside it | **V1–V3 BUILT** (§ Built: push 0.47×, comprehension 1.70×, grid 2.81×, f32_build 1.49×) and **F1 BUILT** (record_update 4.62× → 1.60×; NOT chunk_lookup, whose loop hoists nothing); C1 priced, not built — the next work, ON ITS CORRECTED CONDITION ("no `CallRef` reachable" is not sufficient: `OpFreeRefOrHandUp`, in a capturing lambda's body, registers a store too — list every registrant as a blocker); T1 priced (−45 %) but RE-SIZED: it needs the "text variable is a `&str`" notion given one home first (six inline sites, three shipped bugs) |

## Where to resume (2026-09-21, branch `157-native-4x`)

**1. Read the gate's verdict first.**  The record-shapes arc (R1–R7) is pushed; the local
`make ci` cannot run on the measuring laptop (killed for memory during its compile), so the
gate is the GitHub one, run without a PR:

```bash
gh run list --workflow ci.yml --branch 157-native-4x --limit 3
gh run view <id> --json jobs --jq '.jobs[] | select(.conclusion=="failure") | .name'
gh run view <id> --log-failed | sed 's/\x1b\[[0-9;]*m//g' | grep -E "^\S.*\s+FAIL \["   # the test names
gh workflow run ci.yml --ref 157-native-4x -f os=ubuntu-latest                          # a fresh one
```

Run `35654730666` (head `d66652a4b`) was in progress, with no failed job, when this was
written; one doc-only commit and this handoff followed it.  The run before it was red for
reasons that are all fixed on the branch — read them as the things to check FIRST if a new
run is red:

- **pins moved by a LATER lever.**  A pin suite was last run before the lever that moved
  its counts (R4's mint windows on the R2/R3 cells' own appends, R5 on `v.nrm = v.pos`, R7
  on the enum cells, R1 on the two @PLN157 cells that pinned the old field-path boundary).
  Every VALUES test had passed throughout.  After any hoist-family change, re-run ALL of
  them, not the one being worked on:
  `for t in field_mint iteration_base nested_field mint_window copy_in_place leaving_free
  enum_record record_push group_push record_ptr mint_hoist element_first push_hoist
  vector_base twin_base scalar_hoist callee_inputs view_header emission_audit
  complete_write move_append retbuf_adopt loop_record loop_buffer literal_hoist; do cargo
  test --release --test $t 2>&1 | grep "^test result"; done` (seconds each once built).
- **`make optional-ratchet`** (`@FR-N-Shape`): a new function that names `Type` variants
  must peel the scrutinee (`.base()` / `.peel_link()`).  Advisory job, real finding.
- **a new `tests/*.rs` binary must be named by a subject** in `scripts/test_subjects.sh`
  (`doc_hygiene::every_test_binary_matches_a_subject`).
- **the browser bundle** (`doc/pkg`) is stale whenever `default/*.loft`, `src/compile.rs`,
  `src/native.rs`, `src/wasm.rs` or `src/engine_host.rs` change: `make wasm`, commit it.
- `browser_kernel_one_script_differential` failed ONCE on a first try and passed on retry
  in that run; treat a single first-try failure there as a flake, a repeated one as real.

**2. Then build `vector-build.md`, in its own order: V1–V3, F1, T1, C1.**  Everything in it
is hand-priced with the result hash unchanged; V1 alone is −76…78 % on three rows.  Build
each as the record-shapes levers were built — a clause of an existing rule, a switch, cells
(`tests/scripts/158-*.loft`), emission pins (`tests/*.rs`), a sabotage whose result is
written into the cell file as MEASURED.

**3. Not work** (priced negative or a design question; do not re-derive): a guarded plain
`sum` is slower; `split`'s collected pieces are a text-representation question; `keyed`'s
remainder is structural; `mesh_aabb`'s rest is the null-aware float compare.

## What this arc learned, that the next one should start from

- **Hand-price the emitted Rust BEFORE building.**  `bench/portal/hand_price.sh` compiles
  an edited emission with loft's own release flags.  R2's first build skipped this, took
  the address from the element's `DbRef`, and measured +46 % SLOWER; the hand-priced form
  (address from the INDEX) was −49 %.  Edit by script, one function, and keep the hash.
- **A cell that cannot fail proves nothing — run the sabotage, and believe a green one.**
  Three sabotages changed nothing on their first run: small cells never reallocate a store
  inside a window (R4 → cell `m4b`); a `?` discharge buffer is itself a whole-type write
  that evicts the same scalars (R5 → `c3b`); and one rule no program can make answer wrong,
  so it is falsified over synthetic IR (R6 → `hoist::free_order_tests`).
- **`LOFT_HOIST_VERIFY=1` has blind spots.**  It compares an ADDRESS or a HEADER with a fresh
  derivation: an offset summed wrongly re-reads the store at the same wrong offset (closed
  for nested fields: `vector::path_read_verify`), and an ABSENT answer has nothing to be
  compared with — there the interpreter is the falsifier.
- **Compute a cell's expected value; never write a plausible number.**  Every guessed
  expectation in this arc was wrong, and the interpreter was right each time.
- **An exclusion written for ONE rule's reason gets read by all of them.**
  `plain_record_type`'s refusal of enums is `(R-Scalar)`'s (type, offset) key problem; the
  push header and the record address never had it.  Ask whose reason a gate is.
- **A portal row that reads slower is a claim to A/B, not to explain.**  `fibonacci` +16 %
  and `replace` +23 % against the previous page were both between-session variation: the
  emission was byte-identical with every new switch off, and a build of the pre-arc commit
  measured the same in the same sitting (`git archive <commit> | tar -x -C <dir on DISK>`,
  `CARGO_TARGET_DIR=<dir>/target cargo build --release --lib --bin loft`, then
  `bench/stats.py --only N --loft <dir>/target/release/loft --lib-dir <dir>/target/release`).
- **The measuring laptop runs ONE heavy job at a time**, and `/tmp` is RAM: a build tree
  there is memory.  `scripts/find_problems.sh --changed` falls back to the long curated set
  while `loft-ffi/Cargo.lock` sits untracked in the tree; run named test binaries instead.

## Tools used here

`python3 bench/stats.py --only <lane>` (one lane as statistics; `--tsv` to keep a run) ·
`make perf-portal` (every lane, then the page) · `bench/portal/hand_price.sh` ·
`--native-release --native-emit out.rs` · the rewrites' own traces (`LOFT_TRACE_RECPTR`,
`LOFT_TRACE_HOIST_DECLINE`, `LOFT_TRACE_BASE`, `LOFT_TRACE_CHAIN`, `LOFT_TRACE_PUSH_FILL`) ·
`scripts/emission_audit.py <emitted.rs>` (one holder per path, live headers and bases).
