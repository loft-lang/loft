# @PLN91 — a vector result answered by a call at the function's tail (prepared subject)

**The family:** "a function returning a vector from this tail" (46 programs at try 4bff1c6cc,
5 held by it alone).  All 5 sole programs end in a CALL (`fn c10() -> vector<integer> {
codes() }`), and the reason reads `call n_<callee>`.  The refusal is `el_expressions.loft`
~3551 and ~3599.  The other sub-shapes (`null`, `block`, `if`, `a borrowing local`) are not on
this page.

**The rule:** `@FR-O-Buffer`, `@FR-F-Ret`: a vector result is delivered into the caller's hidden
buffer.  When the tail is a call, how it is delivered depends on what the CALLEE's answer is.
Rust decides that in one selector, `classify_vector_delivery` (control.rs ~3375), with
`callee_forwards_foreign_store` and `tail_forwards_own_store` as its two callee tests.

## Rust's four deliveries, measured on try 4bff1c6cc

```
Rename: the callee builds into a buffer, so the caller's is handed through.
  a-literal-bodied-function-is-a-constant.loft  fn c10() -> vector<integer> { codes() }
     __ref_1: vector<integer> hidden                    <- the hidden parameter, renamed
     -> vector<integer>["??"]
     { OpClearVector(__ref_1) [64] };  n_codes(__ref_1);

AsIs, a native callee: its answer is a store of its own, returned as is.
  55-stack-trace.loft  fn st_inner() -> vector<StackFrame> { stack_trace() }
     __retbuf hidden (unused)   -> vector<ref(StackFrame)>     [6] n_stack_trace();

AsIs, a callee that forwards a foreign store (st_inner above): no chaining.
  55-stack-trace.loft  fn st_middle() -> vector<StackFrame> { st_inner() }
     __ref_1 = null;  [10] n_st_inner(__ref_1);         <- a buffer of its own; __retbuf unused

CopyBorrow: the callee's answer may be a view of an argument, so it is copied in.
  85-temp-subject-borrow-return-uaf.loft  fn h(seed) -> vector<E> { …; g(Filled { items: inner }) }
     { OpClearVector(__retbuf) [29] } … (the body)
     {#one_buffer_vec_copy  OpClearVector(__retbuf);
        OpAppendVector(__retbuf, n_g({#Object __ref_p2_1 …}, __ref_1), 101);  __retbuf }
     -> vector<ref(E)>["??"]
```

`Rename` applies when the tail call's work refs (`extra`) are not empty and the callee does
not forward a foreign store.  `AsIs` applies when it does forward one (st_middle), or when the
callee is native.  `ForwardCopy` applies to a native forwarder of its own store
(`tail_forwards_own_store`).  `CopyBorrow` applies when the answer may borrow an argument (`g`'s
`match` arm answers its subject's `items`).

## What to code

At the refusal (el_expressions ~3551/3599): when the tail is a call, choose the delivery as
`classify_vector_delivery` does.  The copy form is already built for literal tails
(el_expressions ~3352, ~3596, ~3631: `OpClearVector` + `OpAppendVector` in a
`one_buffer_vec_copy` block), so `CopyBorrow` reuses it.  `Rename` is the renamed hidden
parameter with an `OpClearVector` Insert at entry.  `AsIs` is the call as it stands, with a
local null-initialised buffer when the callee takes one.  The two callee tests read the
callee's BODY shape, which is the same on both passes.

## Probes (the five sole programs, each with hand-computed answers)

```
kr-try/tests/scripts/a-literal-bodied-function-is-a-constant.loft:64     n_c10      (Rename)
kr-try/tests/scripts/55-stack-trace.loft:6, :10                          n_st_inner, n_st_middle (AsIs)
kr-try/tests/scripts/85-temp-subject-borrow-return-uaf.loft:31           n_h        (CopyBorrow)
kr-try/tests/scripts/a-text-of-a-slice-reads-the-span-in-place.loft:51   hello()
kr-try/lib/git/src/git.loft:212                                          tracked_files_in("")
```

`probe.py FILE --tools <try> --show <fn>` prints Rust's form and the port's diff.
