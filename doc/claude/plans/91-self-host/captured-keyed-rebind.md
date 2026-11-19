# @PLN91 — a captured keyed local rebound after the capture (prepared subject)

**The family:** "a lambda that captures a local of its function: of this type TyHash" (23
sites in 7 programs at try e399d2cb6). Most are L-CapRebind's
`a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named`: a closure captures
a keyed local `c`, then `c` is rebound whole (a call, a literal, `[]`, a variable, a branch, a
field, a self-referencing call), and the closure must still answer the store it was built with
(`@FR-L-CapRebind`, with `@FR-O-Detach`).

**Judged right** on the interpreter (LOFT_POISON), --native-c (leak check) and --native, 200
calls: `c = mk(5); f = fn() -> integer { tot(c) }; c = mk(9); f() * 100 + tot(c)` → 509 (the
closure keeps 5, `c` holds 9). The suite cell carries the hand-computed value for every
rebind spelling.

## Rust's form (`once_call`)

```
___clos_1: ref(__closure_once_call_0) = null;                 <- the closure record, null-init at the top
c: hash<E,["k"]> = null;
OpReplaceKeyed(n_mk(5), c, 32872);                            <- the first bind

f: function([], integer, deps { items: [2] }) = {#fn_ref_with_closure
    ___clos_1 = null;
    OpDatabase(___clos_1, 101);                                <- 101 = __closure_once_call_0, one field per capture
    OpSetDbRef(___clos_1, 0, c);                               <- the capture: c's store, at offset 0
    FnRef(878, ___clos_1) };                                   <- 878 = n___lambda_once_call_0

-- before the rebind of a CAPTURED keyed local: detach it when the live closure still holds its store
if if if OpRefIsNull(___clos_1) false
       else if OpConvBoolFromRef(___clos_1)
              if OpDistinctStore(OpGetDbRef(___clos_1, 0), c) false else true
            else false
   true else false
{ !! INSERT
    OpInitRefSentinel(c)
    OpDatabase(c, 104) }                                       <- 104 = hash<E[k]>: c gets a store of its own
else null;
c: hash<E,["k"]> = null;
OpReplaceKeyed(n_mk(9), c, 32872);                             <- the rebind, as any keyed bind
OpAddInt(OpMulInt(fn_ref[1](), 100), n_tot(c));

n___lambda_once_call_0(__closure: __closure_once_call_0) -> integer
    n_tot(OpGetDbRef(__closure, 0));                           <- the capture read through the record
```

**What to code:** (1) a lambda that captures a keyed local builds its closure record as above:
a `___clos_N` null-init at the function's top, then `OpDatabase(clos, <closure type>)`, one
`OpSetDbRef(clos, <offset>, <captured var>)` per capture, and `FnRef(<lambda>, clos)`. The
lambda reads the capture as `OpGetDbRef(__closure, <offset>)`. (2) A whole-value REBIND of a
captured keyed local, while the closure record is live, is preceded by the detach guard. It's
the three-way `if` chain (record not null, record truthy, and the record's captured slot NOT
distinct from the local), whose taken arm is the `INSERT` of `OpInitRefSentinel(local)` +
`OpDatabase(local, <keyed type>)`, a fresh store for the local. Then the rebind is emitted as
usual. The guard's `else` is `null`. A mutation THROUGH the local (`c += […]`) is not a rebind
and takes no guard.

## The sites (try e399d2cb6)

```
kr-suite/rules/closures/L-CapOwn/a-closure-that-leaves-its-frame-takes-its-captures-with-it-and-releases-each-once.loft::n_of_hash:148
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_escaping:172
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_loop_pass:162
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_loop_some_passes:167
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_not_taken:170
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_once_call:150
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_once_empty:154
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_once_field:153
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_once_join:152
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_once_lit:149
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_once_self:155
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_once_var:151
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_twice:157
kr-suite/rules/closures/L-CapRebind/a-rebind-of-a-captured-local-leaves-the-record-the-store-its-build-named.loft::n_twice_self:158
kr-suite/rules/closures/L-Fn/a-closure-captures-what-its-body-names.loft::n_heap_values:53
kr-suite/rules/ownership/O-Latest/1447b-a-captured-keyed-local-rebinds-into-a-fresh-store.loft::n_hash_cell:54
kr-suite/rules/ownership/O-Latest/1447b-a-captured-keyed-local-rebinds-into-a-fresh-store.loft::n_the_local_itself_moves:120
kr-suite/rules/types/N-Shape/1489-a-capture-is-bound-and-returned-as-a-whole-value.loft::n_keyed_bind_1489:200
kr-try/tests/scripts/505-collection-capture.loft::n_test_hash_lookup:29
kr-try/tests/scripts/505-collection-capture.loft::n_test_iteration:118
kr-try/tests/scripts/505-collection-capture.loft::n_test_multi_capture:75
kr-try/tests/scripts/505-collection-capture.loft::n_test_point_assign:141
kr-try/tests/scripts/682-closure-capture-borrow.loft::n_c682_param_hash:101
```
