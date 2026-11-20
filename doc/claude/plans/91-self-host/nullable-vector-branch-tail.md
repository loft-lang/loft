# @PLN91 — a nullable vector answered by `if c { [] } else { null }` (prepared subject)

**The family:** "a function returning a nullable vector from this tail" (23 programs at try
b7e35bef5, 2 held by it alone).  This page takes its `if` part as the sole program 2409 has it:
five functions, each ending in `if <cond> { [] } else { null }` with a `vector<…>?` result.
The port refuses at `el_ex_deliver.loft` ~251.

**The rule:** `@FR-O-Buffer`, `(L-Null)`: a nullable vector result is delivered into the caller's
hidden buffer when present, and answers the null sentinel when absent.  Each arm delivers on
its own.

## Rust's form, measured on try b7e35bef5

```
Col-Cons/2409-an-empty-literal-arm-keeps-its-type-past-a-call-in-the-test.loft:22
  fn by_call(k: integer) -> vector<integer>? { if sq(k) == 0 { [] } else { null } }
     __retbuf: vector<integer> hidden          -> vector<integer>["??"]?
     __vdb_1: ref(main_vector<integer>) = null;                         <- head
     [23] if OpEqInt(n_sq(k), 0) {#block
        {#Vector  OpDatabase(__vdb_1, 20);
                  _vec_1 = OpGetField(__vdb_1, 0, 7);  OpSetInt4(__vdb_1, 0, 0);
                  { !! INSERT                                            <- the copy inside the literal
                    OpClearVector(__retbuf)
                    OpAppendVector(__retbuf, _vec_1, 0)
                    OpFreeRef(__vdb_1)
                    __retbuf } }
     } else {#block  OpNullRefSentinel() }
```

The present arm builds the literal in a vector store of its own (`__vdb_1`), copies it into
`__retbuf`, frees that store, and answers the buffer.  The absent arm answers the null sentinel.
Unlike a dense tail (`one_buffer_vec_copy`), the copy is an `Insert` at the end of the literal's
own `#Vector` block.

## What to code

At el_ex_deliver ~251: when the tail is an `if` whose arms are each a vector literal or `null`,
and the result is a nullable vector, deliver per arm:
- a literal arm: build in its `__vdb_N`, then end its `#Vector` block with the `Insert` of
  `OpClearVector(__retbuf)`, `OpAppendVector(__retbuf, _vec_N, 0)`, `OpFreeRef(__vdb_N)` and
  `__retbuf`;
- a `null` arm: `OpNullRefSentinel()`.

The return type is `vector<…>["??"]?`.  Check each of 2409's five conditions (a call, `len`, a
method, a comparison, a local), since the file is about the empty arm keeping its type past a
call in the test.

## Probe (the sole program, hand-computed)

```
kr-suite/rules/collections/Col-Cons/2409-an-empty-literal-arm-keeps-its-type-past-a-call-in-the-test.loft:23, :27, :31, :49, :54
```

Not on this page: the family's other sole program (a-loop-whose-one-growth-is-its-push-writes-
through-t…:35, a tail `out` that borrows a local), and its other arm kinds.
