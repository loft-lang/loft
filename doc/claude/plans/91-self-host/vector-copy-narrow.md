# @PLN91 — a whole-vector copy bind of a narrow-integer vector (prepared subject)

**The family:** "a whole-vector copy bind of this element type: TyInteger" (8 sites in 8
programs at try b94e088a2, 3 of them held by it alone): `w = m`, `kept = e`, `bad = img`,
`b = c` (a parameter rebound) where the vector is a `vector<u8>` (every corpus site), or any
other narrow element (`i8`, `u16`, `i16`, `u32`).  The refusal is `el_ex_assign.loft:2453`.

**Judged right** on the interpreter (LOFT_POISON), --native-c and --native-rust (leak check),
values hand-computed; a wrong expected value fails the run.  Probe:
[probes/vector-copy-narrow.loft](probes/vector-copy-narrow.loft), 15 cases.  Split one function
per case on b94e088a2: 14 refused, the `vector<integer>` control (`elem_integer`) agrees.

## Rust's form: the plain vector copy, the element's narrow content row

```
m = bytes(4); w = m          __vdb_1: ref(main_vector<integer(0, 255)>) = null;
                             OpDatabase(__vdb_1, 102);                          <- main_vector<integer(0, 255)>
                             w: vector<integer(0, 255)>["__vdb_1"] = OpGetField(__vdb_1, 0, 100);   <- vector<byte>
                             OpSetInt4(__vdb_1, 0, 0);
                             OpAppendVector(w, m, 30);                           <- `byte` (30): the CONTENT row
the integer control          OpAppendVector(w, m, 0);                            <- `integer` (0)
vector<i8> / u16 / u32       OpAppendVector(w, m, 101)  with  101 = byte<-128,false> / short_raw<0,false> / int_raw<0,false>
b = c (a parameter)          the same four statements into `b`
```

The same as the `vector<integer>` copy in every line but the last argument.

## What to code

In `vec_copy_bind` (el_ex_assign.loft ~2444) the element row `rec` is looked up by NAME,
`schema_type(e, "integer(0, 255)")`, which no row has: a narrow element's row is named for its
storage (`byte`, `byte<-128,false>`, `short_raw<…>`, `int_raw<…>`).  Take a `TyInteger`
element's row from `el_schema::content_type(e, elm)` — the row the vector's own definition
stores (`vector<byte>`'s `Vector(30)`).  The hidden-parameter twin above it (`brec`, ~2412)
reads the row the same way.

**Verified** on a build of b94e088a2 with exactly that change to `rec`: all 15 probe cases
agree (14 were refused), and two of the three programs the refusal holds alone agree whole
(R-RefillBuffer, S-Paged).

**It needs one companion change, or `174-foreign-file` goes refused → DIFFER** (the third
sole program; `--check` fails on a differ).  Its `n_main` then differs only in the hoisted
declarations' order: `_vec_8` sits among the `__vdb_N`.  The cause is a separate pass-1 gap,
found behind the refusal: an empty `[]` handed to a call (`write_bytes(empty, [])`,
`total([])`).  Rust parses it on pass 1 at the parameter's type (`arg expected=vector<int>`,
`route parse_vector`) and creates its `_vec_N` and `_elm_N` THERE; the port creates them on
pass 2 only.  Every `_vec` named after it in the function is then made on pass 2, and the
hoist (newest variable first) lists it among the `__vdb`s.  The state traces show it
directly (`elab_trace.py PROG --fn f --side diff`).  Probe:
[probes/empty-argument-pass1.loft](probes/empty-argument-pass1.loft): `arg_int`, `arg_u8`,
`arg_first` differ on b94e088a2 and on the patched build; `decl_int` and `nonempty_arg` agree.
What to code: on pass 1, an empty `[]` argument creates its `_vec_N` / `_elm_N` as Rust's
`parse_vector` does under the parameter's hint.  (The port elaborates arguments before it
knows the callee, the `vector_hint` comment at el_ex_object.loft ~2566.  An empty literal
needs only the names on pass 1, not the element type.)

The probe's `decl_u8` (`e: vector<u8> = []` handed on) is NOT this gap: its rows come out
swapped (`OpDatabase(__vdb_1, 101)` / `OpGetField(…, 100)` against Rust's `100` / `101`).
That is the narrow declared-empty numbering the porter is fixing now; take it as one more case.

## The sites (try b94e088a2)

```
kr-try/tests/scripts/174-foreign-file.loft::n_main:97                                 w = m
kr-try/tests/scripts/174-foreign-view.loft::n_main:82                                 g1 = e1
kr-try/tests/scripts/a-function-reads-its-vector-parameter-through-one-header.loft::n_f9:55   b = c
kr-suite/rules/binding/Const-Foreign/a-mapped-vector-appended-as-an-element-is-copied.loft:42
kr-suite/rules/rewrites/R-DecodeView/a-decoded-view-answers-what-its-copy-would-….loft:281   own = value (a match capture)
kr-suite/rules/rewrites/R-RefillBuffer/a-live-buffer-is-refilled-where-it-is.loft:51          kept = e
kr-suite/rules/structures/S-Paged/a-paged-range-read-gates-first-and-searches-the-sorted.loft:84   bad = img
kr-suite/rules/types/N-Shape/1493-a-branch-arm-yielding-a-locals-field-delivers-a-copy.loft:155    e = b.items
```
