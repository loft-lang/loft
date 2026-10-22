# @PLN199 — a `use`d library compiles whole

Plan: [loft-lang/plans#199](https://github.com/loft-lang/plans/issues/199).  The rule it builds:
**only an uninstantiated generic keeps a loft function interpreted.**  A data structure is never
a reason, because a value has the same layout in both modes.  Without `rustc`, only a library
that is not in the registry runs interpreted.

`LOFT_NO_CACHE=1 LOFT_TRACE_NATIVE_GATE=1 loft script.loft` shows what is left; every step
below ends with that trace losing a line.

## How the pieces fit today

A script that `use`s a library loads the library's loft source and, at the end of the load,
compiles the functions the gate admits into a cdylib (`native-auto/loft_auto_<pkg>_<hash>`).
Each admitted function gets a C bridge, `loft_shared_<name>`; the interpreter calls it
through `OpStaticCall`, and the bridge reads its arguments from `LibArg` slots.

| piece | code |
|---|---|
| the gate: which functions may compile | `src/native_gate.rs` `fn_decline` / `walk_decline` (~290–370) |
| the signature judgment, shared by gate, generator and dispatcher | `native_gate.rs` `classify_bridge_attr` (~211), `is_bridge_type` (~245), `bridge_decline` (~122) |
| the bridge generator | `src/native_lib.rs` `shared_bridge_wrapper` (~876), `bridge_read` (~1139), `bridge_write_ret` (~1178), `hidden_dest_type_name` (~1115) |
| the interpreter's side of the call | `src/extensions.rs` `compute_shared_sig` (~386), `marshal_arg_t` (~355), `shared_store_dispatch` (~1051) |
| which functions are candidates | `native_lib.rs` `library_candidates` (~529) |
| which libraries compile | `src/parser/packages.rs` the two `pending_native_compile` pushes (1564, 1925) |
| artifact keying and adoption | `native_lib.rs` `type_layout_fingerprint` (~2166), `artifact_matches_layout` (~2035), `cached_or_build_shared_cdylib` (~2224) |

Three facts shape the design:

1. **The cdylib links its own copy of libloft.**  `Stores`, `DbRef` and `LibArg` have the same
   layout on both sides (the same rlib builds both), but every `static` and `thread_local!`
   in libloft exists twice.  The cdylib's `NATIVE_COROUTINES`, `par_pool` and `call_frames`
   are not the interpreter's.  Anything both sides must agree on therefore crosses as a
   pointer the host hands over, never as a shared global.
2. **The dispatcher sees only `&mut Stores` and the stack.**  `shared_store_dispatch(stores,
   stack)` has no `State`, so nothing a bridge calls can reach the interpreter.
3. **The cdylib is numbered against one program.**  Generated code writes type ids as literals
   (`OpDatabaseNP(cell, r, 100_i32)`), and `type_layout_fingerprint` hashes the WHOLE type
   table, the script's own structs included.  A second script using the same library builds a
   second cdylib, and a prebuilt cannot serve any script but the one it was built for.

The native signatures each shape compiles to (`loft --native-emit`), which the bridge must
produce from the interpreter's values:

```
n_ret_sorted(cell, n) -> DbRef                 // fresh store, no hidden destination
n_ret_vec(cell, n, out: DbRef) -> DbRef        // hidden destination
n_ret_tuple(cell, q, __retbuf: DbRef) -> DbRef // lowered to a `__tuple<…>` record: crosses today
n_arg_tuple(cell, t: (i64, i64)) -> i64        // tuple by value
n_arg_ref(cell, n: *mut i64)                   // pointer to the caller's slot
n_arg_fn(cell, f: (u32, DbRef), x) -> i64      // (d_nr, closure): the interpreter's pair
n_ret_fn(cell) -> (u32, DbRef)
n_gen(cell, n) -> Box<dyn LoftCoroutine>       // the caller registers it: alloc_coroutine_held
```

## Method for every step

- **Prove the target first** (`loft-codegen`): for a step that changes emission, write the
  intended bridge by hand into the probe's generated `native-auto/*.rs`, build it, and run it
  before touching the generator.
- **A refactor step is byte-identical**: `scripts/introspect_diff.sh` over the corpus, empty.
- **The probe library** `tests/lib/gatefull/` (new, phase A0): one `pub fn` per shape below,
  each called from a test program with hand-computed expected values.  It runs interp ≡ mixed
  ≡ native through `tests/n3_parity.rs`, under `LOFT_STORES=warn`, and once under
  `LOFT_POISON=1`.  Every step adds or flips its cells.
- **The trace guard** `tests/n3_use_native.rs::the_native_gate_trace_names_what_stays_interpreted_and_why`
  points at `gatefull` once A0 lands; each step deletes the line it closed from the expected
  list.  The step is falsified (`make falsify`) by reverting its gate change.

## Phase A — the signature lists

**A0 — the probe library and its guards.**  The interpreted and mixed legs pass
`--interpret`: a bare `loft prog.loft` compiles the whole program.  `tests/lib/gatefull/` with the cells of the
scratch probe this plan was measured on (`plain`, `ret_sorted`, `ret_index`, `ret_hash`,
`ret_tuple`, `arg_tuple`, `arg_ref`, `arg_fn`, `ret_fn`, `local_lambda`, `gen`, `sum_gen`,
`par_sum`, `generic`, `uses_generic`).  The trace test asserts the ten interpreted lines
measured today.  No code change.

**A1 — keyed-collection returns.**  The gate's return list is the only block: `bridge_write_ret`
already writes every keyed type as a `DbRef`, `marshal_arg_t` maps it to `ArgT::Ref`, and the
native function returns a fresh store with no hidden destination.
- Change: `bridge_decline`'s `ret_ok` (`native_gate.rs` ~139) adds `Sorted | Index | Hash |
  Radix | Trie`.
- Check: the returned store is owned by the caller and freed once — `LOFT_STORES=warn` clean,
  and a 1000-call loop holds the live-store count flat (`store_memory()`).

**A2 — a returned text the function does not own** (a field read, a literal; loft#773).  The
inner function returns an owned `String` (`--native-emit`: `c.nm.to_string()`), so the bridge
has no pointer to hand back; what it lacks is the caller's destination.
- Parser: `Parser::crosses_text_bridge` gives a value-position call to a BUFFERLESS text
  function (`Definition::returns_bufferless_text`) of a library to be compiled, or of the
  compiled stdlib (`Parser::compiled_bridges`, named by the driver), the temporary
  `wrap_value_text_dest` makes — `is_cdylib_text_call` reads `def.native`, set only after
  parsing.
- Codegen: `try_text_dest_pass` gains the cdylib arm `w += lib_fn()` needs.
- Gate: the `text_ret_has_buffer` decline is gone.

**A3 — tuple parameters.**  The interpreter holds the tuple's elements contiguously on the
stack; native takes the tuple by value.
- `compute_shared_sig`: a tuple attribute expands to one `pops` entry per element, in element
  order, each `forward = true` (nested tuples flatten recursively; an element must itself be
  marshallable).  The dispatcher's reverse pop then reads the elements in place.
- `shared_bridge_wrapper`, `Marshal` arm: read N consecutive slots and build `(e0, e1, …)`;
  `slot` advances by N.
- `classify_bridge_attr` / `is_bridge_type`: admit `Tuple` whose elements are admitted.
- First prove on the probe that the stack order of a tuple argument matches the order of N
  separate arguments (read `loft --interpret --dump`); if it does not, pop the tuple as one
  block instead.

**A4 — `&T` parameters.**  The interpreter passes a `DbRef` into the stack store; native takes
`*mut T`.
- Dispatcher: pop as `ArgT::Ref` (`marshal_arg_t` adds `RefVar(_)`).
- Bridge: `let pK: *mut i64 = unsafe { (&mut *cell.get()).store_mut(&a[k].dbref)
  .addr_mut::<i64>(a[k].dbref.rec, a[k].dbref.pos) };`.
- Scope: scalar inner types first; `&text`, `&vector` and `&(…)` each a later step that starts
  by reading the native signature it must produce.
- Check: the write reaches the caller (`@FR-B-Ref-Write`), and the stack store is not grown by
  the native body (it pushes nothing onto the interpreter stack), so the pointer stays valid.

## Phase B — the stale declines

**B1 — `parallel { }`.**  `--native` emits it (`n_parallel_block_native`, loft#1054); the gate
still declines `Value::Parallel` on a comment that is no longer true (`native_gate.rs:14-20`).
- First a probe: a `gatefull` function with a `parallel { }` block, called from a script that
  also runs a `par(...)` loop of its own.  Measure that the cdylib's own `par_pool` neither
  deadlocks nor oversubscribes beside the interpreter's workers (the pool is one of the
  duplicated statics).  If it does, the host's pool goes through the host table of C0.
- Then: remove `Decline::Parallel`, its `walk_decline` arm and its `explain_interpreted` text;
  rewrite the module header to say what the gate declines now.

**B2 — callers of a `#c` binding.**  @PLN24 keeps the `#c` definition itself out, and that stays.
What goes is the propagation: `walk_decline`'s `Call` arm turns the callee's `CBinding` into
`Callee`, so every loft wrapper of a C function interprets, though the cdylib already carries a
C-library bind table (`__C_LIBS` / `__C_LIB_SYMS` at the top of every generated cdylib, @PLN197).
- Change: in the `Call` arm, a callee with a non-empty `c_sig` is admitted; `fn_decline`'s own
  `CBinding` answer stays for the definition as a candidate.
- Check: the @PLN197 C-library fixture, a loft wrapper over one of its symbols, called from a
  script: compiled, right value, on the static and the dynamic link.

## Phase C — function values

**C0 — the host table.**  The one channel from the interpreter into a cdylib.
- A `#[repr(C)] pub struct HostTable { abi: u32, call_ref: …, coroutine_adopt: …,
  coroutine_next: … }` in `native_lib.rs`, filled by the host.
- Every generated cdylib exports `loft_shared_init(table: *const HostTable, fns: *const u32,
  n_fns: usize)`, which stores the table in a cdylib-side `OnceLock`.
- `wire_shared_native_fns` (`extensions.rs`) calls it once after `dlopen`, before marking
  any export; an artifact without the symbol is rebuilt, the way an undeclared layout is.

**C1 — fn values cross by name, not by number.**  A `d_nr` means a different function in the
cdylib's build than in the program that loads it.
- The cdylib emits `static __FN_NAMES: &[&str]` listing every function its `CallRef` arms
  name.  `loft_shared_init` receives the host's `d_nr` for each (`fns`), resolved by name at
  wiring.
- `output_call_ref` (`src/generation/emit.rs` ~1542, arms from `fnref::dispatch_arms`) in
  cdylib mode compares the incoming `d_nr` against that table instead of literals; a fn
  value the library RETURNS maps its local index back to the host `d_nr`.
- Marshalling: one `LibArg` carries a fn value (`scalar` = `d_nr`, `dbref` = closure);
  `ArgT::FnRef` pops the interpreter's 20-byte slot (`state/mod.rs` ~1121) and pushes it back.
- With C1 alone, `local_lambda` and `ret_fn` compile, and so does `arg_fn` when the script
  passes a library function.

**C2 — the upcall: a compiled caller, an interpreted callee.**  The `_ =>` arm, which answers
the typed null today (`@FR-L-FnAbsent`), calls `HostTable::call_ref(d_nr, closure, args, n,
ret)` in cdylib mode.  A whole-program `--native` build keeps the null arm, because its set of
functions is closed.
- The host side runs the callee with `State::reenter_ret` (`state/mod.rs` ~8266, "the
  compiled→interp RE-ENTRY thunk"), the primitive `live_dispatch` already drives for native
  programs.  It pushes a frame above `stack_high`, runs to completion, and restores the PC and
  stack.  Arguments are pushed by the callee's attribute types, with the closure last, as
  `fn_call_ref_to` (`state/mod.rs` ~1120) does.
- **The `State` must be reachable.**  `static_call` (`state/mod.rs` ~1401) records `*mut State`
  in a host thread-local around a shared-bridge call, saving and restoring it so that nested
  bridge calls work.  `call_frames::enter_shared_bridge` (~304) becomes that save/restore; it
  is a single value today.
- **The aliasing must be legal.**  The dispatcher holds `&mut Stores` borrowed from the
  `State` while the bridge runs.  For a shared bridge the call goes through a raw-pointer
  entry (`shared_store_dispatch_raw(*mut State)`) that takes `&mut Stores` only around each
  pop and push, never across the bridge call.  Before the upcall, the dispatcher's popped
  stack position is written back to the `State`.
- Probes, written down before any code:
  - a callee that allocates enough stores to grow `Stores::allocations` while the compiled
    caller holds a record across the call;
  - a callee that calls back into the same cdylib (nested bridge);
  - an upcall from inside a `par` worker;
  - a callee that panics.
  Each runs under `LOFT_POISON=1` and the ownership drop gate.

## Phase D — generators: one table

Today a generator handle is a different data structure in each mode: `COROUTINE_STORE`
(`database/mod.rs:51`) indexes `State.coroutines` (bytecode frames), and `NATIVE_COROUTINE_STORE`
indexes the thread-local `NATIVE_COROUTINES` (`codegen_runtime.rs` ~5704), which the cdylib
holds a copy of.  The yield codec is already one (`src/coroutine_layout.rs`).

**D1 — a library generator, iterated by a script.**  This is the case that blocks script's
`walk` / `walk_sorted` (@PLN179).
- Bridge for an `iterator<T>` return: box the returned `Box<dyn LoftCoroutine>` once more into
  a thin pointer and hand it to `HostTable::coroutine_adopt`, which registers it in the HOST's
  `NATIVE_COROUTINES` (the body of `alloc_coroutine_held`, `codegen_runtime.rs:5737`) and
  answers the handle.  The trait object works across the boundary for the same reason
  `Stores` does: one rlib built both sides.
- Interpreter: `coroutine_next` (`state/mod.rs:2317`), `coroutine_exhausted` (~2499) and
  `coroutine_slot_matches` (~2530) answer "exhausted" for any store number but
  `COROUTINE_STORE`.  Each gains a `NATIVE_COROUTINE_STORE` arm that advances through the
  host's `coroutine_next_into` / `_dbref` / `_text` (`codegen_runtime.rs` ~6035–6080), chosen
  by the element layout.  The frees that skip `COROUTINE_STORE` handles (~1848, ~1931,
  `state/io.rs:767`) gain the same arm and drop through the host's table.
- Scope: scalar and `DbRef` elements first (`next_into`), text elements in their own step;
  CL-8 and CL-9 (COROUTINE.md) stay their own items.

**D2 — the cdylib uses the host's table.**  Inside the cdylib, `codegen_runtime`'s table
accessors go through `HostTable` once `loft_shared_init` has run.  This makes one table for
both sides, and a handle made in the cdylib and one made in the host are interchangeable.
A whole-program `--native` binary has no host table and keeps its own.

**D3 — an interpreted generator, iterated by a library.**  A library function with an
`iterator<T>` parameter, handed a script's generator: `HostTable::coroutine_next` on a
`COROUTINE_STORE` handle resumes the frame through the same `State` access as C2.
`reenter_ret` asserts that its callee does not yield, so this is a resume entry of its own
beside it, built on `coroutine_next`, not on `reenter_ret`.

**D4 — one handle: dropped.**  D2 already makes one table for every COMPILED generator — a
library cdylib's generated code calls the host's through `__coro`
(`native_lib::HOST_INIT_SRC`) — and an interpreted generator's frame lives in `State`, which
the runtime crate may not name (`scripts/crate_layers.py`).  Moving interpreted frames into the
runtime's table would break that layering for no behaviour: a handle already advances,
retains, frees and tests the same way from either side (`native_lib::HOST_COROUTINES`).  The
two store numbers (`COROUTINE_STORE`, `NATIVE_COROUTINE_STORE`) stay; reopen only if a shape
appears that needs one handle to change kind.

## Phase E — libraries with a native crate

regex declares both `[library] native = "loft_regex"` (a hand-written cdylib, loaded) and
`[native] crate = "loft-regex"`.  Only the first blocks compilation (`packages.rs` 1564, 1925).
The C-ABI link of `[native] crate` packages into a shared cdylib already exists (@PLN26,
`native_lib.rs` ~1626).

- **E1 — never re-export a bound function.**  `library_candidates` admits a body-less
  `#native` declaration: `walk_decline(Null)` is `None`.  Excluding every definition whose
  `native` symbol is already bound (and not a `loft_shared_` bridge) closes the @PLN24
  hazard for `#native` before E2 can reach it.  This is safe to do now.
- **E2 — compile the loft functions.**  Drop `m.native.is_none()` at both sites.  Check that
  the library's OWN crate is in `data.native_packages` when its cdylib is generated, so its
  `#native` calls become `extern "C"` declarations against the crate's sealed `.so`.  The
  hand-written cdylib keeps serving the interpreter's direct `#native` calls.
- **E3 — check on the library's testbed:** regex's own CI run locally (LIBRARY_AUTHORING.md §
  The testbed), with the trace showing its loft functions compiled.  The legacy rlib link
  (`LOFT_NATIVE_CABI=0`) stays a refusal.

## Phase G — published libraries arrive compiled

Today an artifact serves one program: generated code writes type ids and function numbers as
literals (`OpDatabase(cell, r, 100_i32)`, `(835_u32, DbRef::NULL)`, `835_u32 =>` dispatch arms),
and the key hashes the caller's whole type table (`type_layout_fingerprint`) and the numbers of
every function the artifact reaches (`fn_numbering_fingerprint`, `native_lib.rs` ~2654).  G
makes the artifact a function of the LIBRARY alone, so CI can build it and a machine without
`rustc` can run it.  Every step below is either byte-identical on `--native` (checked with
`scripts/introspect_diff.sh` over the corpus) or changes only library cdylibs.

**G0-0 — a lambda's name is its place in its enclosing function.**  The root of the function
half: a lambda is named from ONE counter across the whole parse (`__lambda_{lambda_counter}`,
`parser/vectors.rs:1643` and `:1949`, the counter at `parser/mod.rs:1150`, reset per pass at
`parser/passes.rs:173` and `:224`), so a library's lambda numbers shift with whatever was parsed
before it, and no table can map them by name.
- Change: the counter becomes per enclosing definition (a `HashMap<u32, u32>` keyed by
  `self.context`, reset where `lambda_counter` is), and the name
  `__lambda_<enclosing stored name>_<k>`; a lambda inside a lambda takes its parent's stable
  name.  Keep the `__lambda_` prefix: `native_lib::is_synthetic_name`, `parser/suggest.rs:82`
  and the closure-record names (`vectors.rs:2397`, `:3423`, `__lambda_` → `__closure_`) read it.
- Check: the suite green on both backends; a new `n3_use_native` test — two scripts that differ
  only in a lambda declared before their `use` give the library's lambdas the same names
  (`LOFT_LOG=static` dump).  Not byte-identical (names change), so it is its own commit.

**G0a — one helper writes every type-id literal (byte-identical).**  `Output::type_lit(tp: u16)
-> String`, which prints today's literal.  The sites: the `known_type` reads in
`generation/mod.rs` (51), `ops/parallel.rs` (9), `dispatch.rs` (8), `hoist.rs` (3), and one each
in `destination.rs`, `decode_view.rs`, `decode_structure.rs`, `coroutine.rs`, `apart_value.rs`;
plus the `name_type(…)` ids written as `{tp}_i32` (5 sites).  One commit per file; each is
checked with `introspect_diff.sh` empty.

**G0b-1 — one helper writes every function-number literal (byte-identical).**
`Output::fn_lit(d_nr: u32) -> String` at the three places a number reaches the code: the `FnRef`
value (`generation/emit.rs:390` and `:392`, `({d_nr}_u32, …)`), the dispatch arm
(`emit.rs:1820`, `{d_nr}_u32 =>`), and the closure drop cascade's arm
(`generation/ops/ref_ops.rs:82`, `{lambda}_u32 =>`).  Checked with `introspect_diff.sh` empty.

**G0b-2 — in a library cdylib, both helpers read tables filled at init.**
- `type_lit` prints `__tid(k)` and `fn_lit` prints `__fid(k)`; `Output` records each `k` with
  the `(package, name)` it stands for, and the generated source ends with
  `static __TID: [AtomicU16; N]` / `__FID: [AtomicU32; M]` and the two name tables.
- A dispatch arm cannot match a non-constant: in library mode it becomes a guard,
  `f if f == __fid(3) =>`, one arm per candidate as now — the fn-ref dispatch and the drop
  cascade alike.
- The names come from the HOST's definitions, not its store: `loft_shared_init` takes a second
  argument, and `HostTable` two entries — `type_id(ctx, pkg, name) -> u16` and
  `fn_id(ctx, pkg, name) -> u32` — answered from the `Data` that
  `extensions::wire_shared_native_fns` already holds.  Qualified by package, so a program's
  own `InputState` beside a library's never answers for it (the loft#1706 class that
  `hidden_dest_type_name` handles per call today).  A name the host lacks makes the init
  answer 0, and the artifact is rebuilt, as an undeclared layout is.
- A relaxed `AtomicU16` load per id, never a `OnceLock` (an acquire load per call cost a
  generator loop 6-8 % in D2).
- Check: the parity guards (`gatefull`, `callback`) on every mode; a struct-allocating loop and a
  fn-value dispatch loop in a cdylib within 2 % of the literal build (`bench.py`-style A/B);
  and the falsifying case — an artifact built for one program, adopted by a second whose types
  and functions are numbered differently (build it, then remove the fingerprint from the key in
  a probe): right answers with G0b-2, wrong ones without.

**G0c — key the artifact on what it references.**  Replace `type_layout_fingerprint(stores)` and
`fn_numbering_fingerprint` in `cached_or_build_shared_cdylib`'s key (`native_lib.rs` ~2509) with
one fingerprint over the recorded `(package, name)` of every type — with its size and field
offsets — and every function — with its signature.  `artifact_matches_layout` probes the same
value.
- A generic instantiated with a type from OUTSIDE the library (`i_<T>_n_…` with a script's `T`)
  is program-specific; it leaves the shared artifact's export set and keeps today's
  per-program artifact, keyed as now.  `library_candidates` (`native_lib.rs` ~529) is where the
  split is made.
- Check: two scripts with different structs using one library build it once (the second run
  creates no file in `native-auto/`); the per-program artifact appears only when the script
  instantiates a library generic with its own type.

**G1 — a reproducible name.**
- G1a: generated source carries no machine path.  `cr_call_push` / `cr_call_push_lean`
  (`generation/mod.rs` ~12590) print the absolute file; print it relative to the package root
  (`<pkg>/src/x.loft`), which is what a stack trace should show anyway.
- G1b: the stem hashes `contributing_fingerprint` (the library's sources), `loft_exe_identity`,
  `cache::host_triple` and the G0c fingerprint — nothing from the program.
- Check: the same library, checked out at two paths, built by the same `loft`, produces the
  same stem and byte-identical `.rs`.

**G2 — the producer.**
- G2a: `BinaryEntry` (`registry_index.rs:123`, parsed at ~400 beside `loft_ffi_fp`) gains
  `loft_exe_id: Option<u64>` and `stem: Option<String>` (the G1 name).  A unit test reads an
  index line with and without them.
- G2b: `loft build-native` (`main.rs:8845`) builds the AUTO cdylib for a package with no
  `[library] native`, and prints the G1 stem and `loft_exe_identity`.
- G2c: the release pipeline (RELEASE_PUBLISH.md) runs G2b over every registry library's latest
  version per released triple and adds the entries; trust is the existing signed index (the
  entry's `sha256`).  A library published between releases is built by the registry's CI.

**G3 — the consumer.**  `fetch_prebuilt` (`install.rs:87`) returns false for a package without
`m.native`.
- G3a: a second branch: no stem in the manifest, a binary entry whose `loft_exe_id` equals
  `cache::loft_exe_identity()` → write it to `native-auto/<stem>` with the dylib's platform
  name.  `cached_or_build_shared_cdylib` then adopts it on its first branch (present, current,
  layout probe passes), with or without `rustc` (`native_lib::rustc_available`, ~2046).
- G3b: the trace names a library that interprets because no prebuilt matches this loft and
  `rustc` is absent.
- Check: a registry fixture with `rustc` hidden from `PATH` runs compiled (the trace's `n of n`);
  the same binary under another `loft_exe_id` is never fetched.

## Phase F — the gate is the rule

- **F1 — the census.**  `LOFT_TRACE_NATIVE_GATE=1` over every registry library's published
  version (`scripts/revalidate_libs_local.sh` already resolves them), counted per decline
  reason, as `make gate-census`: a report, never a gate.  It orders the standing shapes below
  by how many published functions each keeps interpreted.
- **F2 — the generators `--native` cannot carry.**  `coroutine_layout::channel_tag` answers
  `CHANNEL_NONE` for a tuple yield holding a text or a nested tuple (`coroutine_layout.rs` ~181),
  which the generator answers with a `compile_error!` on `--native` too (loft#1132) — so this is
  a backend gap, not a bridge one, and closes `Decline::Yield` for programs and libraries alike:
  a `YieldSlot` for the nested tuple, flattened as the tuple parameters of A3 are.
- **F3… — the standing shapes**, in the census's order, each by the procedure below.
- **F-last.**
  - `Decline` keeps `NotFunction` (which covers an uninstantiated generic) and nothing else:
    `CBinding` is already out of the candidate set (E1), `Yield` closes in F2, `CallRef` with
    the last `upcall_carries` shape.
  - The rule is recorded as a design decision (`./scripts/idx next-decision`), cited by
    `native_gate::fn_decline`, `explain_interpreted` and the `gatefull` guard.
  - The trace guard asserts `n of n` with `GATEFULL_INTERPRETED` empty, and NATIVE.md § N9's
    open row lists what remains, if anything.

## Standing work — the remaining shapes, as needed

Each shape below still declines, and runs interpreted: the answer is right, only slower.  None
is scheduled.  One is lifted when a consumer or a profile meets it — the trace
(`LOFT_TRACE_NATIVE_GATE=1`) names the function and the reason.

| shape | where it declines |
|---|---|
| `&text`, `&vector`, `&(…)` parameters | `native_gate::is_ref_scalar` |
| a narrow scalar (`single`, `character`, a plain enum, a sized `integer`) in a tuple parameter, a generator's yield, a `&T`, or a function value's signature | `is_tuple_leaf`, `generator_crosses`, `is_ref_scalar`, `fnref::upcall_carries` |
| a tuple yield | `generator_crosses` |
| a function value with a `&T` parameter, or a keyed-collection result | `fnref::upcall_carries` |

The procedure, the one every step above followed:
1. A `gatefull` function with the shape, a call in `gatefull_main.loft` with its value computed
   by hand in `gatefull_main.out`, and the function added to `GATEFULL_INTERPRETED`
   (`tests/n3_use_native.rs`) with its reason — the guard then pins the decline.
2. Lift the decline at its home, and carry the shape through whatever the bridge, the
   dispatcher or the call back needs; delete the line from `GATEFULL_INTERPRETED`.
3. The parity guard on every mode (`tests/n3_parity.rs`), the store counts against the
   interpreter, and the guard falsified by reverting the lift.

## Order

Done: A0–A4, B1, B2, C0–C2, D1–D3, E1–E3.  D4 is dropped.  The open steps, each one commit:

G0-0 → G0a (per file) → G0b-1 → G0b-2 → G0c → G1a → G1b → G2a → G2b → G3a → G3b → G2c, and,
independent of G: F1 → F2 → the standing shapes in F1's order → F-last.  G0a and G0b-1 change
no behaviour and can land any time; G0b-2 is the step that makes an artifact shareable, and
G2c, the release job, is last because it publishes.
