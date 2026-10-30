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
1. **never written below the root — already proven, by the parser.**  `Decoded.value`,
   `CborEntry.key` and `CborEntry.value` are value-const fields (`(Const-Value)`,
   `@FR-Const-Value`): every write reached through them, at any depth (`t.v[i] = …`,
   `t.r.x = …`), is a compile error.  A type whose tree is not value-const below the root is
   not admitted (T1–T2), so the twins carry no write check of their own;
2. the SITE proof — every use of the root reads: `use_analysis::read_only_uses`, the
   `@FR-R-Const` walk, with the producers marked (§ The sites).  It also refuses the one write
   `const` still allows, the whole-value rebind `d.value = other`: a setter's root is a reach;
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
  where the parameter is `const`, or the callee never writes it — the one-call-deep fact
  `(R-Const)` and `(R-CopyView)` already read (`R-Callee`) — AND the callee is itself a READER:
  its own body, walked with that parameter as the root, passes the same test;
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
8. the rebind `const` allows — `d = decode(f); d.value = CInt { value: 7 }; read d.value` —
   declines (a setter root to the walk) and reads 7;
9. a reader that writes its plain parameter (`fn touch(m: CborValue) { … m = … }`) declines
   the hand-on, and its caller's tree is store-built, same values.

Falsified by plants: a twin that keeps the arena across sites (6 grows), a text view that skips
the UTF-8 check (2 reads the raw bytes), a site proof that ignores a write (5 reads the new byte).

## Steps

| step | what | measures against |
|---|---|---|
| T0 | cells 1–9 on the store form, hand-computed, both backends | — |
| T1 | `opt/transient.rs`: the producer candidates (a result type value-const below its root), the root sites from `read_only_uses(…, marked, true)` with the dead-`.value` check, the forwarder half of the fixpoint; `c/body/transient.rs` `__tr0` (the dead tree); switch `LOFT_NO_TRANSIENT=1`, trace `LOFT_TRACE_TRANSIENT=1` naming each site admitted and the position that declined | `check_request` 0.54, cbor `decode` |
| T2 | the hand-on extension (a `const` or never-written parameter of a READER); the source proof; the representation: `lt_<T>`, the arena in `loft_rt.h`, `__tr` twins of producers, forwarders and readers | `check_request` ≈ 0.09 |
| T3 | materialisation at a storing use: `req_state_b64`, `request` (`bytes_to_base64(value)`) | those rows |

Each step: the C sweep (`scripts/c_sweep.py`), the C test binaries, gcc and clang A/B over all 36
libraries (`ab_all.sh`), the cells falsified.

## Not in this design

- A twin for a producer whose result one site keeps and another reads: both forms are emitted,
  each site calls its own.
- Native: shelved.  The interpreter: unchanged, the oracle.
- Changing the store (C125): the store is untouched; the transient tree is a short-lived object
  outside it, as clarified by the owner on 2026-10-30.
