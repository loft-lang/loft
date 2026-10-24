
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
python3 scripts/c_sweep.py --verbose FILE...   # both outputs of each differing program
python3 scripts/c_sweep.py --mode default      # one macro mode instead of all three
python3 scripts/c_sweep.py --all               # every refusal reason, not the top 25
```

`c_sweep.py` builds every carried program in the three macro modes (below) and puts it in one
bucket: `match`, `differ`, `cc-error`, `sanitizer` (`--asan`) or `refused`, and ranks the
refusals by reason — that ranking is the work list.
`differ`, `cc-error` and `sanitizer` are defects of the backend and must stay at 0; a change
that grows `match` is measured by the sweep and by `--asan` before it is committed.  A program
without `main` is its tests: the C runs every test the way `loft --tests` picks them, and the
two exit statuses are compared.  `tests/c_emit.rs` is the cargo guard: a scalar program checked
against hand-computed values on both the interpreter and the C, a keyed program (hash, sorted,
index, a linked group) checked the same way, a refusal naming its construct, and the overflow
macros against Rust's checked arithmetic in every mode.

## The parts

| file | holds |
|---|---|
| `mod.rs` | `Cx` (one per unit): the C shape of each loft type, the tuple structs, the copy table, the enum name tables, the record default images, the render functions; `emit_unit`, `emit_program`, the test `main` |
| `body.rs` | `FnWriter`: one function's body — statements, expressions, the copy rules, the operators |
| `show.rs` | the `{v}` rendering of a record or vector: one `loft_show{tp}` per type |
| `fnval.rs` | function values: the dispatcher of each signature, one arm per function used as a value |
| `body/keyed.rs` | the keyed collections: filing a record, a lookup by key, a walk over a key range, a removal, a whole-collection write |
| `loft_cc.h` | the macro layer — the ONLY place the C names a compiler — pasted first into every unit |
| `loft_rt.h` | the C runtime, pasted after it |
| `loft_keyed.h` | the keyed collections' runtime, pasted after `loft_rt.h` |

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
length at byte 4 and their elements from byte 8, with the interpreter's growth rule.  The
allocator is a bump allocator: which words a claim gets is not observable, and a store's words
go back together when the store is freed.  The default bytes of a new record are captured by
running `Stores::set_default_value` on a scratch copy of the stores.

**Copies.**  Where the bytecode writer copies a record, the C copies it, decided by the same
questions: a whole-value bind (`record_copy_source`), a call result that is not fresh
(`call_copy`, with the argument stores protected as `protectable_ref_args` protects them), an
element bind.  A deep copy walks a table of the program's types (`loft_types`).

**`&` parameters.**  A link is a pointer to the caller's variable; every read and write of it
goes through the pointer.  A text result's hidden buffer is a link the caller makes for the
call.  A rebind of a `&` record releases the record it displaces.

**Held vectors.**  Before a loop, each vector `opt::header::hoistable_vectors` admits — a loop
that writes no store but in place, so no record moves and no length changes — is held in a
`loft_vh` (store, record, length, element base), and the loop's element reads and in-place
writes go through it.  Each held access carries `LOFT_PROVED(loft_vh_same(…))`, so
`LOFT_VERIFY` catches a held vector that changed.  `LOFT_NO_VECTOR_HOIST=1` holds none and
`LOFT_NO_WRITE_HOIST=1` none in a loop that writes, as for the Rust writer.

**Keyed collections.**  A hash, a sorted and an index keep the interpreter's ORDERS and
answers, in layouts of the C runtime's own (`loft_keyed.h`): nothing observes a layout, because
every walk over a keyed collection is in key order (`@FR-Col-Order`) and every lookup is by key.
A hash is an open-addressed table of record numbers, its entries records of their own; an index
and an `ordered` are a key-ordered array of record numbers; a sorted is a vector kept in key
order, read by a vector's readers.  The key compare, the range search and the sorted insert are
ports of `keys::compare`, `vector::sorted_search` and `vector::sorted_finish`.  A record type
shared by several collections of one struct (`@FR-Col-Group`) turns its vector into an `array`
and its sorted into an `ordered`; a record finished into one member enters every member, and a
record a key collision displaces leaves every keyed member.  A walk over a hash walks a snapshot
in key order; an entry a write displaces or removes while the walk runs drops out of it.

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
4. Sweep, then sweep with `--asan`; both at 0 `differ`, 0 `cc-error`, 0 `sanitizer`.
5. A sweep that turns up a `differ` is a defect to fix before going on — it is usually an
   older rule the new construct now reaches.

## Not carried yet

The sweep's ranking is the current list.  The large families: file and system I/O, iterators and
coroutines, `&` locals, the spatial and trie collections, parallel blocks, and the native library
calls.  A record holding a linked group is not copied yet: its members name one record set,
which a field-by-field copy would split.
