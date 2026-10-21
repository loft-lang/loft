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

**A0 — the probe library and its guards.**  `tests/lib/gatefull/` with the cells of the
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
inner function's `Str` points into a store or into the cdylib's static data, and both outlive
the call.
- Bridge (`bridge_write_ret`, the `Text` arm): with no `WorkText` attribute, write the inner
  `Str` into `(*ret).text_ptr/len` instead of requiring a work buffer.  In dest mode it still
  copies into `bridge_text_dest`.
- Dispatcher (`shared_store_dispatch`, the `Some(ArgT::Text)` arm without `self_dest`): push
  `Str { ptr: ret.text_ptr, len: ret.text_len }` instead of the empty `Str` it degrades to.
- Gate: drop the `text_ret_has_buffer` decline.
- Check: the call used in place (an argument, a format hole), which is where loft#773 showed.
  A literal's bytes stay valid because the cdylib handle is never closed (`probe_and_mark_exports`).

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

**D4 — one handle.**  The two store numbers collapse into one host-owned table whose entries
are an interpreted frame or a compiled state machine, behind one `next`.  This is the end
state the owner's rule asks for.  It is a refactor once D1–D3 hold: the probes stay green and
the corpus is byte-identical on `--native`.

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

**G0 — a cdylib numbered by name.**  This is the precondition, and it also removes the
second build when another script uses the same library.
- **G0a**: one helper writes every type-id literal in generation (about 76 `known_type`
  reads across `src/generation/mod.rs`, `dispatch.rs`, `ops/parallel.rs`, `hoist.rs`, …).
  Program mode prints the literal it prints today, so the corpus diff is empty.
- **G0b**: in cdylib mode the helper prints `__tid(k)`, reading a table that
  `loft_shared_init` fills by name from the caller's `Stores::name`.  This is the lookup
  `hidden_dest_type_name` already does per call.
- **G0c**: `type_layout_fingerprint` hashes only the types the library references (name,
  size, alignment, field offsets).  Two scripts with different structs then share one cdylib;
  the check is that the second script's run builds nothing.

**G1 — a reproducible name.**  The artifact name hashes the library's source, `loft_build_fp`,
the triple and the G0c fingerprint, and nothing from the program.  CI can then build the file
the loader will look for.

**G2 — the producer.**  `loft build-native` (`src/main.rs` ~8845) already builds the auto cdylib
and prints `loft_build_fp`.  The registry index's `BinaryEntry` (`src/registry_index.rs` ~123,
parsed ~388–410) gains `loft_build_fp` beside `loft_ffi_fp`.  The loft release pipeline
(RELEASE_PUBLISH.md) runs `build-native` over every registry library's latest version for each
released triple and publishes the entries.  A library version published between loft releases
is built by the registry CI on publish.

**G3 — the consumer.**  `fetch_prebuilt` (`src/install.rs:87`) requires `m.native` today.  It
learns the auto model: no stem, matched on `loft_build_fp`, written to the G1 name in
`native-auto/`.  `cached_or_build_shared_cdylib` adopts it after the layout probe, with or
without `rustc`.  The trace says when a library interprets because no prebuilt matches this
loft build and `rustc` is absent.
- Check: a registry fixture with `rustc` hidden from `PATH` runs compiled.  The same binary
  under another `loft_build_fp` is never loaded.

## Phase F — the gate is the rule

- `Decline` keeps `NotFunction` and an uninstantiated generic, nothing else.
- The rule is recorded as a design decision (`./scripts/idx next-decision`), cited by the
  gate, the trace and the `gatefull` guard.
- The trace guard asserts `n of n` with no per-function line.
- NATIVE.md § N9's open row lists what remains, if anything.

## Order

A0 → A1, A2, B1, B2, E1 (each small and independent) → A3, A4 → C0 → C1 → C2 → D1 → D2 → D3 →
E2/E3 → G0 → G1–G3 → D4 → F.  G0 can start any time after A0; it is the largest refactor and
touches no behaviour until G0b.
