
// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later

# The C backend

`src/c/` writes a loft program as C99 that `cc` builds into a standalone executable.  It is the
second compiled target beside the Rust writer (`src/generation/`), and it exists to prove that
the compiler's lowering is backend-neutral (@PLN200 B5): it reads the same IR, the same
analyses (`use_analysis`, `scopes`) and the same store layouts as the bytecode writer, and it
edits none of them.  A program the backend does not carry yet is refused by name; it never
writes C that answers differently from the interpreter.

## Running it

```bash
loft --emit-c out.c prog.loft      # write the C; exit 2 with "does not carry this program
                                   #   yet: <fn>: <why>" for a construct not carried
cc -std=c99 out.c -o prog -lm && ./prog
python3 scripts/c_sweep.py         # every corpus program: C against the interpreter
python3 scripts/c_sweep.py --asan  # the same, built with AddressSanitizer + UBSan
python3 scripts/c_sweep.py --poison  # the store poison modes, against the interpreter's
python3 scripts/c_sweep.py --verbose FILE...   # both outputs of each differing program
python3 scripts/c_sweep.py --mode default      # one macro mode instead of all three
python3 scripts/c_sweep.py --all               # every refusal reason, not the top 25
```

`c_sweep.py` reads `target/release/loft` (or `LOFT_BIN`) for every program, so a rebuild while
it runs mixes two compilers into one result: build first, or point `LOFT_BIN` at a copy.  It
builds every carried program in the three macro modes (below) and puts it in one bucket: `match`, `differ`, `cc-error`, `sanitizer` (`--asan`) or `refused`, and ranks the
refusals by reason — that ranking is the work list.
`differ`, `cc-error` and `sanitizer` are defects of the backend and must stay at 0; a change
that grows `match` is measured by the sweep, by `--asan` and by `--poison` before it is
committed.  A sanitizer cannot see inside a store — a store is one allocation — so a read of a
freed store, of a record a growth moved, or of a claim nothing wrote is valid memory to it; the
poison modes are the store's own falsifiers, ported from `src/store.rs`: `-DLOFT_POISON` (a
freed store reads `0xDEADBEEF`, a growth moves the buffer and scribbles the one it leaves) and
`-DLOFT_POISON_CLAIM` (a claim and a grown tail read `0xDEADBEEF`, and `loft_claim` stops
zeroing), each against the interpreter under `LOFT_POISON=1` / `LOFT_POISON_CLAIM=1`.  A program
without `main` is its tests: the C runs every test the way `loft --tests` picks them, and the
two exit statuses are compared.  A printed `store_memory()` report is compared without its
lines, which name the interpreter's own stores (its value stack, its constants) and bytecode
positions; the programs that READ the report's counts — a store census before and after a
thousand rounds — are the sweep's leak check, since a release the C skips leaves a record
behind and nothing else fails.  `tests/c_emit.rs` is the cargo guard: a scalar program checked
against hand-computed values on both the interpreter and the C, a keyed program (hash, sorted,
index, a linked group) checked the same way, a refusal naming its construct, and the overflow
macros against Rust's checked arithmetic in every mode — and the memory model's replays: the
allocator, the hash, the index and the trees each run one seeded sequence through the Rust
module and the C header and compare every answer and every byte of the store.

## The parts

| file | holds |
|---|---|
| `mod.rs` | `Cx` (one per unit): the C shape of each loft type, the tuple structs, the copy table, the enum name tables, the record default images, the render functions; `emit_unit`, `emit_program`, the test `main` |
| `body.rs` | `FnWriter`: one function's body — statements, expressions, the copy rules, the operators |
| `show.rs` | the `{v}` rendering of a record or vector: one `loft_show{tp}` per type |
| `fnval.rs` | function values: the dispatcher of each signature, one arm per function used as a value |
| `body/keyed.rs` | the keyed collections: filing a record, a lookup by key, a walk over a key range, a removal, a whole-collection write |
| `body/generator.rs` | generators: the frame, the creator, the resume and the release of a function returning `iterator<T>` |
| `body/par.rs` | `par`: the worker over the rows of a `for … par(…)` loop, its results in a buffer the loop reads |
| `body/misc.rs` | the operators no family claims: `abs`, `**`, vector reverse and sort, a text's bytes |
| `body/files.rs` | files: a `File`'s description and text, a value written or read in its binary form, the filesystem operations |
| `files.rs` | the binary form of each record and vector type a program writes or reads: one `loft_ser{tp}` / `loft_de{tp}` pair |
| `loft_cc.h` | the macro layer — the ONLY place the C names a compiler — pasted first into every unit |
| `loft_store.h` | `src/store.rs`' allocator, ported: the free tree, parked lists, lazy frees, tombstones |
| `loft_rt.h` | the C runtime, pasted after it |
| `loft_keys.h` | key parts, their compare, and the per-table SipHash-1-3 digest of `src/siphash.rs` |
| `loft_hash.h` | `src/hash.rs` and `src/arena.rs`, ported, and the `Stores` routines over a hash |
| `loft_btree.h` | `src/btree.rs`, ported: the index, and the `Stores` routines over it |
| `loft_radix.h` | `src/radix_tree.rs`'s layout and key bits: the constants, a key's composed bits, the Morton helpers |
| `loft_radix_walk.h` | the tree's walks (descend, seek, step, the box and prefix walks), pasted twice: over a store (`loft_rx_*`) and over a paged image (`loft_prx_*`), as the Rust walks are generic over `TreeNodes` |
| `loft_radix_db.h` | `radix_db.rs` and `trie_db.rs`, ported: the spatial and trie collections over a store |
| `loft_image.h` | store images: `store_persist_copy` and `store_load` (`Stores::persist_copy`, `load_path`) |
| `loft_paged.h` | `src/paged_reader.rs`, ported: an image read a page at a time, every read past its end 0 |
| `loft_paged_load.h` | the working-set loaders (`store_load_key`, `_keys`, `_key_text`, `_keys_text`, `_prefix`, `_box`, `_range`) and their relocating copy |
| `loft_keyed.h` | sorted and ordered (`src/vector.rs`), the walks, renders and keyed writes |
| `loft_json.h` | `src/json.rs`' `JParser` and the lexer's JSON mode, ported; the JsonValue tree in a store and its natives (`LOFT_JSON`, `LOFT_JV_*`) |
| `loft_parse.h` | `T.parse(text)` and `T.parse(JsonValue)`: the walks into a type, read from the `loft_ptypes` table `parse.rs` writes, and the per-store `#errors` (`LOFT_PARSE`) |
| `loft_io.h` | files and the filesystem, pasted last |

## The macro modes

`loft_cc.h` gives the emitted C three compile-time modes, and a mode changes speed, never a
value: `-DLOFT_STRICT` is pure C99 (clean under `-std=c99 -pedantic -Werror`, every macro its
C99 fallback); the default uses gcc / clang / MSVC extensions where it detects them;
`-DLOFT_VERIFY` checks every proof the generator relied on again and aborts naming the site.
Its sites are of three kinds: SAFE HINTS (`LOFT_HOT_INLINE`, `LOFT_LIKELY`, the overflow tests
`LOFT_ADD_OVF` / `LOFT_SUB_OVF` / `LOFT_MUL_OVF`), PROOF SITES (`LOFT_PROVED`, the
`LOFT_PLAIN_*` operators, emitted only where a rewrite proved the fact) and SHIP SITES
(`LOFT_SHIP_*`, @PLN201's dropped checks, wrapping through `uint64_t` because a C signed
overflow is undefined).  The runtime and the generator write these macros and never a raw
builtin or attribute.

The OS beyond C99 — a monotonic clock, a path's kind and size, directories, truncate, fsync — is
POSIX where `loft_cc.h` finds the platform to be one (`LOFT_POSIX`).  `-DLOFT_NO_POSIX` builds
against ISO C99 alone: the clock is `clock()`, a file is what `fopen` opens, a directory is
never seen, a listing is empty and the directory operations answer `Other`.

## The model

**Shapes.**  Every loft value has one C shape (`CType`): `int64_t` for every integer width
(null `LOFT_NULL`, a narrow range is enforced by `OpRangeDefault` and by the field setters),
`uint8_t` for a boolean (255 null) and a plain enum (255 null), `double` / `float` (NaN null),
`uint32_t` for a character (0 null), `loft_text` for an owned text and `loft_str` for a view,
`loft_ref` for a reference into a store, and a numbered tuple struct.  Every local and every
temporary is declared at the top of its function.

**Texts.**  A text local is an owned `loft_text`; a text parameter is a borrowed `loft_str`.
An owned temporary an expression makes is freed when its statement ends — and statements
written inside an expression (a compound, a result buffer) free only their own temporaries,
because the enclosing expression's are still to be read.  The epilogue frees every text the
function holds.  The null text is the one byte `"\0"`, as in the interpreter.

**Stores.**  `loft_rt.h` ports the store model the IR addresses: a store is an array of 8-byte
words, a record a run of words whose first word is its claimed size, a reference names a store,
a record and a byte position, and `rec == 0` is null.  Vectors keep their claim at byte 0, their
length at byte 4 and their elements from byte 8, with the interpreter's growth rule.  The memory
model is loft's ([@PLN200 B5](plans/200-backend-structure.md), owner): the allocator is
`src/store.rs`' (`loft_store.h`), and the routines above it that decide what is claimed and
deleted — `vector_add`, `clear_vector_release`, `remove_claims`, `Stores::clear` — are ported
with it.  The default bytes of a new record are captured by
running `Stores::set_default_value` on a scratch copy of the stores.

**Copies.**  Where the bytecode writer copies a record, the C copies it, decided by the same
questions: a whole-value bind (`record_copy_source`), a call result that is not fresh
(`call_copy`, with the argument stores protected as `protectable_ref_args` protects them), an
element bind.  A deep copy walks a table of the program's types (`loft_types`).

**`&` parameters and locals.**  A link is a pointer to the variable it names — the caller's, for
a parameter; the one `b = &a` names (`OpCreateStack(a)`), for a local — and every read and write
of it goes through the pointer.  A link into a store place (a field, a vector element) is not
carried yet.  A text result's hidden buffer is a link the caller makes for the
call.  A rebind of a `&` record releases the record it displaces.

**Held vectors.**  Before a loop, each vector `opt::header::hoistable_vectors` admits — a loop
that writes no store but in place, so no record moves and no length changes — is held in a
`loft_vh` (store, record, length, element base), and the loop's element reads and in-place
writes go through it.  Each held access carries `LOFT_PROVED(loft_vh_same(…))`, so
`LOFT_VERIFY` catches a held vector that changed.  `LOFT_NO_VECTOR_HOIST=1` holds none and
`LOFT_NO_WRITE_HOIST=1` none in a loop that writes, as for the Rust writer.

**Keyed collections.**  A persisted image carries a collection's layout, so the layouts are
loft's.  A hash is `src/hash.rs` ported (`loft_hash.h`): the seeded table, inline and arena
entries, a linked type's borrowed records, and its `Stores` routines — the one-probe insert and
replace, `remove_owned`, `reserve_hash`, the copy and the release.  An index is `src/btree.rs`
ported (`loft_btree.h`): the node arena, the key parts and text prefixes, the walk's
record cursors.  A sorted is a vector kept in key order (`sorted_new`, `sorted_finish`), and an
`ordered` and an `array` a vector of record numbers (`ordered_finish`), each with the release
`remove_owned` and `remove_vector_at` give it.  A `spatial` and a `trie` are
`src/radix_tree.rs` ported (`loft_radix.h`) under `radix_db.rs`' Morton oracle and
`trie_db.rs`' text oracle, with their walks — the outward walk from a point, the box walk by
BIGMIN, the prefix walk — snapshotted as `build_rec_scratch` lays them out.

**Store images.**  `store_persist_copy` is `Stores::persist_copy` ported: the root deep-copied
into a scratch store, its trees laid out in van Emde Boas order (`rtree_relayout`), the store
written from word 0 to past its last live record, padded to the 1024-word floor, and the
`.dschema` sidecar beside it — its text prepared at emit time per root type
(`LayoutIdentity::of_store`), the slot line filled in at run time.  `store_load` is `load_path`:
the sidecar gated (`classify`: the same hash and dump, `Release::admits`), the image adopted
into the slot as `Store::load` builds one (no parked lists, no lazy frees, the allocator state
re-derived from the blocks).  A file of another layout is refused, where the interpreter
converts it; a store holding references is refused at emit time until the reference remap
(`rebase_stored_refs`) is ported; compaction on load is not done (it declines for an image
`store_persist_copy` wrote, which is already dense).  `tests/c_emit.rs` crosses a file written
by each backend with every reader — the interpreter, the C, `--native` — on a store of every
collection kind, and checks the C's image and sidecar against the interpreter's byte for byte.

**Paged loads.**  The working-set loaders read an image a page at a time (`loft_paged.h`, the
port of `paged_reader.rs`): a 64 KiB page is read from the file when a walk first touches it,
a byte past the end reads as 0, and a size or a count is trusted only as far as the record that
holds it (`@FR-S-Paged`), so a malformed image answers and never faults.  A hash entry is found
by the port of `find_hash_entry`, a sorted range by its binary search; a trie or a spatial walk
is the C's own tree walk, the same source pasted over the paged reader, with the fuel and node
bound `PagedTrie` adds.  The entry is copied as `Stores::copy_entry` copies it — its bytes, its
texts and vectors relocated, then filed by the collection's own insert.  Which collections a
loader serves and which entry types it can copy is the Rust store's answer
(`collection_type_of_store`, `is_copyable_entry`), asked when the program is written to C; a
loader that cannot serve its collection runs and answers 0, saying why on stderr, as the
interpreter does.  An `http(s)://` source is not carried.  `tests/c_emit.rs` crosses every
loader with every writer and reader, and
`tests/scripts/2177-a-malformed-image-answers-every-paged-load.loft` mutates every record of
each image.  The key compare, the range search and the sorted insert are
ports of `keys::compare`, `vector::sorted_search` and `vector::sorted_finish`.  A record type
shared by several collections of one struct (`@FR-Col-Group`) turns its vector into an `array`
and its sorted into an `ordered`; a record finished into one member enters every member, and a
record a key collision displaces leaves every keyed member.  A walk over a hash walks a snapshot
in key order; an entry a write displaces or removes while the walk runs drops out of it.

**Mapped files.**  `file_map` answers a FOREIGN store, as `Stores::fs_file_map` does (@PLN174):
a store of its own whose root slot names the one synthetic record `LOFT_FOREIGN_REC`, the
file mapped read-only (`mmap`) behind it, the mapping released with the last span over it.
`loft_at` serves that record from the mapping — the header (size word, length) below byte 8,
the file's bytes past it — so every vector read, the hoisted ones included (`loft_vh.base` is
the element base, `Store::elem_base`), reads a mapping as it reads a record.  A slice bound to
a local of its own (`OpSliceView`) is a VIEW sharing the owner; a clear, a rebind or the free
drops it; a mapped vector copied as a record's field is copied element by element
(`@FR-Const-Foreign`).  Without POSIX the file is read as `read_bytes` reads it, as the
interpreter does without `mmap`.  The emitter defines `LOFT_FOREIGN` only for a unit that
reaches `file_map`, so no other program pays the record test in `loft_at`.

**JSON and `T.parse`.**  `loft_json.h` is `JParser` over the lexer's JSON mode, token for
token: the offsets, the JSON pointer and the message of every error are json.rs's, and
`tests/c_emit.rs` reads some 28 000 documents and their mutations through both, in both
dialects.  `json_parse` writes the tree into a store of its own at the program's JsonValue
layout, and its readers and builders follow `native.rs`.  `T.parse(text)` reads the text as it
writes the store (`read_into`), and `T.parse(json_parse(t))` walks the tree
(`struct_from_jsonvalue`); both read what each type takes, and what an absent or misfit value
leaves, from a table of the program's types (`parse.rs`: field names, declared nullability and
defaults, narrow encodings, variants).  The errors a parse files are the record's `#errors`,
kept per store and carried as the interpreter carries them — by a record copy into another
store's root, a bind, an exchange, and a whole-vector write.  A second differential in
`tests/c_emit.rs` parses some 1 750 documents into a record of every field kind, a vector of
records, the checked cast and the staged form, on the interpreter and as C.  A parsed type
holding a keyed collection, a linked group or a vector of nullable records is refused.

**Files.**  A `File` record keeps the interpreter's fields (size, positions, path, handle,
format); its handle indexes a table of open `FILE *`s, opened on first use, flushed after every
write (the interpreter writes unbuffered, so another handle on the path reads it), and turned
read-write when a write goes through a handle a read opened.  A relative path resolves against
the program's own directory, which the generated `main` passes in (`loft_set_anchor`;
`LOFT_SOURCE_DIR` overrides it, `#cwd` turns it off), after the interpreter's normal form
(`PathText::program`: separators, `.` and `..`, the names no platform takes).  A value written
or read hands the C the VARIABLE where the bytecode hands its stack slot (`OpCreateStack`).

**Generators.**  A function returning `iterator<T>` becomes a frame struct holding every local
and temporary, a creator that fills it from the arguments (a text argument copied, since the
generator outlives the caller's view), a resume function and a release.  The resume function
restores the locals from the frame, jumps to the label after the last `yield`, and at the next
`yield` saves them and returns the value through its `out` pointer; the body between is written
as any other function's.  An iterator value is a handle into the runtime's generator table
(`loft_gen_*`), released by `OpFreeRef` like a store.

**Closures.**  A closure record's release (`OpDropFnRef`) runs the record type's drop cascade,
which runs the drop hooks of what it captured; the unit brings the cascade of every lambda it
reaches, and `loft_drop_closure` picks it by the value's definition.

**`par`.**  A `for x in v par(r = f(x), n)` loop's queue call runs the worker over the rows one
after another — each row's result is the one a thread would compute — into a buffer on a stack
the loop reads by row and pops (`loft_par_*`).  A row goes to the worker by its stored bits
(1, 4 or 8 bytes by the element's width) or as a reference, as `worker_row_arg` hands it; a
vector result gets a fresh buffer per row, a record result mints its own.

**Removed records.**  The allocator never reuses a record, but one a collection removes keeps
its claim word negated, so a stored reference to it reads null (`@FR-H-Removed`) — today the
records a `clear()` or a kept range drops from a vector of shared records.

**The evaluation-stack tail.**  `[value, frees…, return null]` returns the value: the scopes
pass leaves it on the bytecode's stack for `OpReturn`.  The C captures it into a temporary, as
the Rust writer does (`detect_ref_tail_capture`).

**Tests.**  A failed `assert` in a test returns to the test `main` (`setjmp`/`longjmp`), which
prints `FAIL <name>` and exits 1 when any test failed.  A test annotated `@EXPECT_FAIL` passes
only by failing with one of its messages — the annotations are read by the same parser
`loft --tests` uses (`test_runner::parse_annotations`).

## Adding a construct

1. Take the top reason of `c_sweep.py`'s refusal ranking.
2. Read how the interpreter executes it — the `#rust` template in `default/01_code.loft`, the
   op in `src/fill.rs`, the store method it calls — and port that, nulls and edge cases
   included.  A constant operand (a type number, a format) is read at emit time.
3. Probe it in a scratch program on both: the interpreter's output and the C's must be
   byte-identical, and under `-fsanitize=address,undefined` the C must be silent.
4. Sweep, then sweep with `--asan` and `--poison`; all at 0 `differ`, 0 `cc-error`, 0
   `sanitizer`.
5. A sweep that turns up a `differ` is a defect to fix before going on — it is usually an
   older rule the new construct now reaches.

## Not carried yet

The sweep's ranking is the current list.  The large families: a parse into a keyed collection
or a vector of nullable records, the image loaders over a URL, a lazy binding
(`store_bind_lazy`), `store_reclaim`, a linked group with a spatial or trie member,
`parallel { }` blocks, and the native library calls; of the generators, one held in a record
field and one of tuples; of the `&` locals, a link into a store place; of `par`, a worker over
tuples or function values.  A record holding a linked group is not copied yet: its members name one record set,
which a field-by-field copy would split.
