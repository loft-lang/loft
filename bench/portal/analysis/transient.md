# The transient tree — a value read and dropped is built outside the store

`(R-Transient)` in `doc/claude/formal/rewrites.md` is the rule; this file is its design: what the
hand price measured, the one invariant, the representation, the code points, the cells, and the
steps.  C only.  The interpreter keeps the store form and is the values oracle; native is not
built (shelved, @PLN203).  NOT BUILT: this is the paper half, for the owner's sign-off.

## The shape it is for

pluginabi's `check_request`, the worst C ship row (≈ 8× its twin after `(R-SliceArg)`):

```loft
pub fn check_request(frame: vector<u8>) -> text {
  if !pa_decode_ok(frame) { return ERR_MALFORMED; }   // decode(frame).ok — the tree is never read
  op = pa_text(pa_decode(frame), "op");               // the tree is read for one key
  if !valid_op(op) { return ERR_UNKNOWN_OP; }
  return "";
}
```

Each decode is cbor's `read_value`: a `Decoded { value: CborValue, next, ok }` whose `value` is a
tree of store records (a claim per node, a store for a map's entries, a text per key).  Neither
use stores the tree.  The twin's `Cbor<'a>` borrows the frame and allocates one `Vec` per map.

## The price (2026-10-30, by hand, in the emitted C, same hashes)

The emitted C of `pluginabi/bench/bench.loft` at `a906a782f`, `check_request` alone, `ab.py`, 5–7
alternating rounds, pinned.  The hand code is `lt_read`: `read_value` line for line, scalars in a
C struct, texts and bytes as views of the frame (a text view is validated once and reads as the
empty text when it is not UTF-8, as `text_from_bytes` answers), one child array per array or map.

| form | gcc | clang |
|---|---|---|
| store tree (today) — 1 053 ns a check, twin 124 | 1.000 | 1.000 |
| T1 only: the `.ok` decode builds no tree at all (validation) | **0.537** | — |
| the `.ok` decode as a transient tree (`malloc` per array), the key decode unchanged | 0.590 | — |
| T2: both decodes transient, `malloc` per array | **0.109** | **0.129** |
| T2 with the arrays in a bump arena the site owns | **0.087** (≈ 87 ns, 0.7× the twin) | — |

The arena beat `malloc` by 21 %: a decode's arrays are claimed in recursion order and die
together, which is exactly a stack.  That settles the representation question the rule left open.

Hand code: `target-exp/ab/pricing/transient.c` (local); `b.c` / `d.c` / `e.c` are the patched units.

### Re-priced after `(R-PureReuse)` reached the decoders (2026-10-31)

Building S3 showed `(R-PureReuse)` never fired on cbor: `effect_free` declined `read_value`
because its write list (`pure_reuse::write_target`) did not name `OpSliceView`, the operator a
slice VALUE lowers to — a write into the slice's own fresh record, exactly as `OpSliceVector`
(listed) is.  With it named, `check_request`'s `pa_decode_ok(frame)` and `pa_decode(frame)` are
two projections of one `decode(frame)`, computed once: **0.584× on gcc**, same hash, from one
operator name.  What that leaves for this rule: `check_request` now decodes ONCE, into the store,
and reads one key — T2's case (the hand price had both decodes transient at 0.109×, so one decode
transient is expected well under that).  T1's dead tree no longer has a site in `check_request`;
it keeps cbor's `decode` row (`do_d.next` only) and stays the scaffolding T2 is built on.

## The one invariant

**A transient value is never written after it is built, it dies before the arena mark its ROOT
SITE took is restored, and no source one of its views reads is written while it lives.**

Everything else follows from it.  Since nothing writes the tree, a copy and a view of it cannot
be told apart: a struct copy (shallow, children by pointer) is a correct `(B-Copy)`, and
`pa_get` answering `e.value` hands back a pointer into its argument's tree.  Since the tree dies
with the mark, nothing needs releasing node by node: no claim, no free, nothing for the leak
report.  Since the frame is not written, a text view reads the bytes `text_from_bytes` would
have copied.

Re-assertion sites (where the invariant must be checked, and the only ones):
1. **never written below the root — proven by the site walk, NOT by the declarations.**  cbor
   declares `pub const value: CborValue`: `const` before the NAME is `(Const-Bind)`, which
   freezes the slot and leaves the contents writable (`t.v += …`, `t.v[i] = x`).  Only
   `value: const T` (`(Const-Value)`) forbids writes through it, and cbor does not use it.  So
   the proof is the walk's: a setter at any depth of projection under the root is a reach, and
   a reach declines the site.  (Found writing the cells, 2026-10-31: the earlier text here
   claimed the parser proved it, and a candidate test on value-const would have excluded cbor);
2. the SITE proof — every use of the root reads: `use_analysis::read_only_uses`, the
   `@FR-R-Const` walk, with the producers marked (§ The sites) — this is also where site 1 is
   asserted;
3. the hand-on proof — a root handed to a parameter or returned through a forwarder (§ The
   sites), the two positions `read_only_uses` refuses today;
4. the source proof (T2 only) — no write of a viewed source between the root's bind and its
   last read (`opt::body_writes`, the question `(O-ViewField)` already asks); `read_value`'s
   `bytes` is a plain parameter, so `const` does not answer it;
5. the mark — restored at the root's scope exit and at every exit that passes it.

## The representation

| loft | transient C |
|---|---|
| a record or a struct-enum reached from the result type | `lt_<T>`: a tag byte (enum) and the union of the variants' fields |
| `integer` / `boolean` / `float` field | the C scalar |
| `text` | `loft_str` view (into a viewed source, or into the arena) |
| `vector<u8>` (any scalar element) | `{ const uint8_t *p; uint32_t n; }` view |
| `vector<R>` of records | `{ lt_R *p; uint32_t n, cap; }`, `p` in the arena |
| a record field of record type | inline `lt_R` (a field cannot be shared) |

The arena: one per thread (`loft_ta`), a 64 KiB static block with a chain of `malloc`'d spill
blocks; `loft_ta_mark()` / `loft_ta_back(m)` at the root site, the spills freed on the way back.
A vector of records that grows past its capacity re-claims at the arena top (its old block is
dead space until the mark is restored — a decode knows its counts up front, so the CBOR rows
never grow).

## The sites

A ROOT SITE is a call to a twinnable producer whose result is only ever read.  The test is not
new: it is `use_analysis::read_only_uses(body, n_vars, data, marked, through = true)`, the walk
behind `@FR-R-Const`, with `marked` answering the producers.  Its `calls` answer is exactly the
root sites: a marked call at the root of a scalar getter or of a projection chain that ends in
one (`decode(f).next`), or the value of the single `Set` of a local that is itself read-only
(`d = decode(bytes); return d.ok;` — `pa_decode_ok`).  `through` adds a block's tail in a read
position and the alias `w = v` (how `for x in c` reads `c`).  Its fallback is the one this rule
needs: a position the walk does not name reaches the value, so the set only SHRINKS on a shape
it does not understand.  The scalar getters it lists (`OpGetEnum`, `OpGetText`,
`OpLengthVector`, …) are the reads a `match` on the variant, a payload binding, a `len` and a
text operand lower to.

What the walk refuses today, and this design adds, as extensions behind its `through`-style
switches so the plain form's answers never move:

- **a hand-on to a parameter** (`pa_text(pa_decode(frame), "op")`, `pa_get(m, key)`): admitted
  where the callee is a READER of that parameter — its own body, walked with the parameter as
  the root, passes the same test.  That subsumes "the callee never writes it": a write is a
  reach to the walk.  The parser's `callee_param_writes` (`src/parser/closures.rs:460`) is NOT
  reused — its recursion placeholder answers "not written" for a cycle, the wrong direction for
  a proof; the fixpoint below starts every candidate IN and only removes;
- **a return through a FORWARDER** (`decode` returns `d`, `pa_decode` returns `decode(…).value`):
  the result is admitted when every call site of the forwarder is itself a root site or a
  forwarder's.

Those two make the sets a fixpoint over the call graph, the one new analysis
(`opt/transient.rs`), run as `close_fnref_groups` runs: start with every candidate in, take a
function out when one of its uses fails, re-check its callers, repeat — it only removes, so it
ends.  The sets: the PRODUCERS (`read_value`, recursively — its own recursive calls are root
sites inside its body), the FORWARDERS (`decode`, `pa_decode`) and the READERS (`pa_get`,
`pa_text`).

A use that stores the value — a field set, an append, a keyed insert, a return that escapes
the unit, a capture, a `yield`, a `parallel` arm, a native callee that takes a store reference
(`bytes_to_base64(value)`) — is a reach to the walk, so it declines the site in T1 and T2 and is
T3's materialisation.

**T1's extra condition**: the site reads no field of the tree at all — no getter or projection
of `Decoded.value` (field 0) is among the root's reads.  Then the tree is DEAD and the twin
builds none of it.

## The twins

`f__tr` is `f`'s IR emitted under the transient mapping, by a second arm in the C body writer
(`src/c/body/transient.rs`), with an ALLOW-list of operations, as `opt::hoist` gates: an op on a
transient value that the arm does not map declines the twin, never a wrong value.  The mapping
for `read_value`, op by op:

| IR (as emitted today) | `__tr` form |
|---|---|
| `OpDatabase` / `loft_database_typed` of the result or a node | an `lt_<T>` local, zeroed |
| `OpSetEnum` / `loft_set_byte` of the tag | `.tag = k` |
| `loft_set_int` / `loft_set_byte` of a scalar field | `.f = v` |
| `text_from_bytes(bytes[a..b])` into a text field | `loft_str` view, empty when not UTF-8 |
| `bytes[a..b]` into a `vector<u8>` field | `{ p + a, b - a }` |
| `items += [sub.value]` (push header, element-first build) | `lt_vec_push(&items, sub.value)` in the arena |
| `loft_copy_record` of a node into a field | struct assignment |
| `loft_free_*` / `loft_remove_claims` of a transient | nothing |
| a call to a producer | its `__tr` twin |

**T1, the dead tree:** where the site reads no field of the tree at all (`pa_decode_ok`, cbor's
`decode` row reads `.next`), the twin is `f__tr0`: every op the table maps is elided instead,
and so is every local that only feeds them (`items`, `entries`, `kv`, `vv`).  It is the same
writer arm with the mapping "to nothing", so T1 builds the analysis, the twin emission and the
allow-list, and T2 adds the representation.

## Soundness, case by case

1. **Values.**  The twin computes the same scalars on every path; the tree it builds holds the
   same variants, payloads and order as the store tree, since every write is mapped one for one
   and nothing else writes it.  A key that is not UTF-8 reads as `""` in both.
2. **Ownership.**  The store form's tree had one owner, the site's local; the transient tree
   has none and needs none — it dies at the mark, which is the local's scope exit.
3. **A view's source.**  Proved unwritten while the root lives (site 3).  `frame` is a
   parameter of `check_request` that nothing there writes.
4. **Recursion and depth.**  `MAX_DEPTH` bounds the recursion as it does today; the arena
   spills instead of failing, so a deep or wide frame costs `malloc`s, not a different answer.
5. **Faults.**  An arena spill that cannot be allocated halts as a failed store claim does.

## Cells (written before the build, hand-computed)

`tests/scripts/a-value-read-and-dropped-is-built-outside-the-store.loft`, both backends:

1. `decode(f).ok` / `.next` on a valid map, a truncated frame, a trailing byte, a non-canonical
   head, depth 256 and 257 (T1);
2. one key read: present, absent, a key that is not UTF-8 (`""`), a duplicate key (the first);
3. a reader that walks the tree recursively (`fnv_value`) — values AND count of nodes;
4. a site that STORES the result (`keep += [decode(f).value]`) — must stay store-built, same values;
5. a source written while the tree lives (`f[0] = 0` between decode and read) — must decline;
6. a site inside a loop of 10 000 — the store watermark flat, nothing for the leak report;
7. a forwarder chain (`pa_text(pa_decode(f), k)`) and a site in a `par` arm (declines);
8. a site that rebinds its root — `t = pa_decode(f); read t; t = CInt { value: 7 }; read t` —
   declines (two binds) and reads 7 (`d.value = …` itself is a compile error under
   `(Const-Bind)`, so it is no cell);
9. a reader that writes its plain parameter (`fn touch(m: CborValue) { … m = … }`) declines
   the hand-on, and its caller's tree is store-built, same values.

Falsified by plants: a twin that keeps the arena across sites (6 grows), a text view that skips
the UTF-8 check (2 reads the raw bytes), a site proof that ignores a write (5 reads the new byte).

## Where it lives

Decisions over the IR, emission in the C — the split `(R-ValueRecord)` (`opt::value_records` →
`VrFn`, `src/c/body/value.rs`) and `(R-AppendTwin)` (`opt::append_twin::eligible` →
`FnWriter::append_twin`) already use.  The IR gains no type and no op: a transient value is a
representation the C chooses, not a second kind of object the interpreter runs (@C125, @C122),
and the interpreter stays the independent oracle the cells compare against.

| `src/opt/transient.rs` — the plan, over the IR | `src/c/body/transient.rs` — the emission |
|---|---|
| the sets: producers, forwarders, readers (one removing fixpoint) | an `lt_<T>` struct per admitted type |
| the root sites (`use_analysis::read_only_uses`) | the `__tr0` / `__tr` signatures |
| per twin: each local's form — struct, view, arena vector, dead | each op in its mapped form; an unmapped op declines the twin |
| T1: the dead nodes, the statements that only feed `.value` | those nodes skipped |
| each decline with its reason (`LOFT_TRACE_TRANSIENT=1`) | the arena mark at the root site, its reset at the exits |

## Steps

Each step is one commit, verified before the next starts, and each leaves the program's output
unchanged — only S5, S10 and S12 change what runs, each behind `LOFT_NO_TRANSIENT=1`.  Every step
that touches `src/` runs fmt and all-features clippy before the push (two red CI runs on
2026-10-30 were those alone), then `--changed`, the C test binaries (`tests/c_emit.rs` and the
`push_*` / `locked` / `foreign` binaries), and the C sweep `scripts/c_sweep.py`.  A step marked
**byte-identical** proves the emitted C of the sweep corpus and of the 36 library benches equal
before and after (`--emit-c`, `cmp`) — the loft-codegen skill's refactor gate.

### T0 — the cells, on today's code

**S0.** `tests/scripts/a-value-read-and-dropped-is-built-outside-the-store.loft`: cells 1–9, each
value hand-computed in a comment, value AND length AND leak asserted; both backends
(`--interpret`, `--native-c`).  Proves the harness can fail: one cell's expectation flipped by hand
reads red, then restored.  No `src/` change.

### T1 — the dead tree

**S1. Free a twin bit, and close the overlap found while reading.**  `src/c/mod.rs:287` gives
`DS_BIT` bit 57, and `NN_MASK` (`:294`, 26 bits from 32) covers bits 32–57: a call proving its
26th parameter non-sentinel (`nn_twin_bits`, `src/c/body.rs:5734`, admits `args.len() <= 26` at
`:5693`, and sets bit `i` for parameter `i` under `.take(26)` at `:5775`) also sets `DS_BIT`
and is written as a destination twin.  Narrow `NN_MASK` to 24 bits (32–55), the call-site guard
to `args.len() <= 24` and the `.take` to 24, and add `TR_BIT = 1 << 56` beside `AP_BIT`
(`:282`), with its suffix in the twin name (`:1221`–`:1240`) and in `function`'s `places` mask
(`:2489`).  **Byte-identical** except a call to a function of 25 or 26 parameters (the sweep
names any; expected none).  Cell: a 26-parameter function whose last parameter is proven
non-sentinel at one call site, called beside another that passes null — reads the right value
(today it reads through a destination twin).

**S2. The switch, the trace, the empty plan.**  `transient_enabled()` in `src/keys.rs`, beside
`copy_view_enabled` (`:1745`), reading `LOFT_NO_TRANSIENT`.  `src/opt/transient.rs` registered in
`src/opt/mod.rs`, with `pub struct Plan` (empty sets) and `pub fn plan(data, stores) -> Plan`;
`Cx::transient()` in `src/c/mod.rs`, cached the way `Cx::value_records` is (`:403`), counting
`rewrite_census::fired("R-Transient", …)`.  **Byte-identical.**

**S3. The candidates and the root sites — computed, printed, not used.**  A PRODUCER candidate:
a loft-defined function returning a record, with no `yield`, capture or `par` — its fields'
`const` does not matter (site 1).  The
root sites: `use_analysis::read_only_uses(body, n_vars, data, marked, true)` (`src/use_analysis.rs:4123`)
with `marked` = the candidates; its `calls` are the sites.  The FORWARDER half of the fixpoint:
a candidate whose result is returned (`decode` returns `d`) stays in only when every call site of
it is a site or another forwarder's; repeat until nothing leaves, as `close_fnref_groups`
(`src/opt/value_records/sites.rs:24`) does.  Each site's DEAD verdict: no getter or projection
of the result's heap field among the root's reads.  `LOFT_TRACE_TRANSIENT=1` prints every site,
forwarder and decline with its reason.  **Byte-identical.**  Written down before running, the
expected trace on pluginabi + cbor: producer `read_value`; forwarder `decode`; dead sites
`pa_decode_ok`'s `decode`, the cbor bench's two `decode_op` calls; `pa_decode` declined (it
returns a field — a T2 forwarder), `decode_hash` declined (`fnv_value(…, dh_a.value)` hands it
on — T2's hand-on).

**S4. The dead slice — computed, printed, not used.**  For each producer with a dead site, the
statements of its body that only feed the result's heap field: start from the result field's
`Set`s, walk back through locals whose every use is such a statement (`items`, `entries`, `kv`,
`vv`, the `sub.value` reads, the `CText`/`CBytes`/`CInt`/`CBool` builds, the `text_from_bytes`
calls and their slices), and through the producer's own recursive calls (their result is dead
there too, so they call `__tr0`).  A local with any other use (`sub.ok`, `sub.next`) is live.
The trace lists the dead locals per producer; expected for `read_value`: `items`, `entries`,
`kv`, `vv` and the build of every `value:` field.  **Byte-identical.**

**S5. The `__tr0` twin, emitted and called.**  In `FnWriter::call` (`src/c/body.rs:5550`), beside
`append_call` (`:3319`, set at `:5681`): a call that is a dead site, or a producer's recursive
call inside a `__tr0`, sets `TR_BIT`.  `function` (`src/c/mod.rs`, the builder chain at `:2678`)
passes `.transient_dead(&plan)` to the body writer; `stmt` (`src/c/body.rs:3341`) and `expr`
(`:5081`) skip a dead node, and the twin returns the result's remaining scalars as a `loft_tup`
the way an admitted `(R-ValueRecord)` function does (`src/c/body/value.rs`), so it takes no
return buffer.  Measured: `check_request` gcc and clang (priced 0.537 on gcc), cbor `decode`;
`ab_all.sh` over the 36 libraries both compilers.  Cells 1, 4, 6, 8 green both backends; the
`LOFT_NO_TRANSIENT=1` run byte-identical to S4.  Falsified: a plant that also elides a live local
(`sub.next`) reads the wrong `next` in cell 1.

### T2 — the tree outside the store

**S6. The arena, in the runtime, unused.**  `loft_ta_mark` / `loft_ta_alloc` / `loft_ta_back` in
`src/c/loft_rt.h` beside the slice carriers (`loft_slice_take`, `:1793`): one thread-local 64 KiB
block, a chain of `malloc`'d spills freed on the way back.  Nothing calls it; a static function
the compiler drops.  **Byte-identical program code**; a C unit cell in `tests/c_emit.rs` for mark,
nested mark, spill and back.

**S7. The struct types, declared, unused.**  `lt_<T>` per admitted record and enum (§ The
representation), emitted by `c/body/transient.rs` into the unit's declarations as `Cx::vr_ctype`
(`src/c/mod.rs:462`) shapes a value tuple.  Only typedefs change.

**S8. The producer twin `__tr`, for sites that read the tree in their own body.**  The mapping
table of § The twins, with texts and bytes COPIED into the arena — no view yet, so no source proof
is needed and this step cannot read a byte the program later changed.  An op the table does not
map declines the twin (trace says which op).  Root site: `loft_ta_mark` at the bind, `loft_ta_back`
at the root's scope exit and on every exit that passes it.  Cells 1–3, 6; falsified by a site
that skips its `loft_ta_back` (cell 6's watermark grows).

**S9. The hand-on.**  A third extension of `read_only_uses`, beside `through`, so its plain answers
never move: a marked value at an argument position is a read when the callee is a READER of that
parameter — the callee's body walked with the parameter as the root (memoised per (function,
parameter), every candidate IN at the start, removing).  Readers' twins take `const lt_<T> *`.  A
forwarder that projects (`pa_decode` returns `decode(…).value`) returns that field's struct.
`check_request` now runs both decodes transient.  Cells 2, 3, 7, 9.

**S10. The views.**  A text or bytes field built from a slice of a parameter becomes a view of it
where `opt::body_writes` proves the source unwritten from the root's bind to its last read at the
SITE (the question `(O-ViewField)` asks); otherwise S8's arena copy stays.  A text view is checked
as UTF-8 once, empty when it is not.  Cell 5 (a written source keeps the copy); falsified by a
view that skips the UTF-8 check (cell 2 reads raw bytes).  Measured: `check_request` priced 0.087
gcc.

**S11. Re-measure.**  `ab_all.sh` both compilers; a fresh `bench/stats.py --lanes rust,cship`
table for the rows this reaches (pluginabi, cbor); placement-only regressions recorded, not fixed
(owner).

### T3 — materialisation

**S12.** A use that stores the value builds it into the store at that use, from the transient
tree (`(R-Place)`, `(R-InPlaceLiteral)`), instead of declining the site: `req_state_b64` and
`request` (`bytes_to_base64(value)`).  Designed after S11's table.

## Not in this design

- A twin for a producer whose result one site keeps and another reads: both forms are emitted,
  each site calls its own.
- Native: shelved.  The interpreter: unchanged, the oracle.
- Changing the store (C125): the store is untouched; the transient tree is a short-lived object
  outside it, as clarified by the owner on 2026-10-30.
