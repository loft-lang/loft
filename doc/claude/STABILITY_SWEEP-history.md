<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# STABILITY_SWEEP history

Findings closed or fixed, moved out of [STABILITY_SWEEP.md](STABILITY_SWEEP.md).  Each entry is
kept as written; the files it names under `src/generation/`, `src/codegen_runtime.rs` and
`src/native_lib.rs`, the loft rlib a native build linked, and the rustc a spawned build resolved
left the tree with the rustc writer (@PLN225, `@C170`).

## Findings log entries moved out on 2026-11-24

### F5/260-1 — native declaration point derived from IR init position (#260; pass-2 entry)

- **Invariant**: a native local is declared exactly once, in scope for every
  use; the IR's `Set(v, Null)` position means *initialization order* only —
  it must not double as the declaration point.
- **Homes today**: `scopes.rs lastuse_reclaim` relocates the null-init for
  watermark benefit; `generation/dispatch.rs`'s `declared` set derives the
  `let mut var_…` placement from wherever that init landed.  Fix A
  (`has_free_before_alloc`) is a GUARD compensating for the coupling — it
  excludes early-return-shadowed stores from reclaim so the derived `let`
  never strands a free (rustc E0425, 92× on the brick-buster `--html`
  build pre-A).
- **Natural home**: the native function emitter, declaring ref-typed locals
  up front FROM THE VARIABLE TABLE (the structure that already knows every
  local and its type); `Set(v, Null)` lowers to a plain assignment.  After
  the move, any IR reordering is safe by construction and the Fix-A guard
  is pass-3 dead code — deleting it restores the watermark reclaim for the
  excluded stores.
- **Instrument before the move**: an env-gated sentinel in
  `lastuse_reclaim` counting excluded stores per function across the suite
  + the brick-buster build — quantifies the recoverable reclaim (if ~zero,
  the move is cleanliness only; if large, it carries Goal-E value).
- **Validation matrix when executed (low-churn window)**: the original 92×
  E0425 `--html` build (now reproducible locally — binaryen + chromium
  installed 2026-06-11, `make game` + `tests/html_render.rs` green), full
  native + wasm suites, ASan gate.
- **Status**: EXECUTED 2026-06-11 (#260 Fix B, branch bugs325).  The
  instrument showed brick-buster's generator forfeited 46/46 owning
  stores; the move landed as a prologue on BOTH backends (native:
  sentinel `let mut` per `__vdb` from the variable table + the
  non-`Block`-body branch covered; interpreter: `OpInitRefSentinel` per
  slot after the frame reserve), `Set(v, Null)` keeps its IR position
  (allocation timing unchanged; native first-Set still emits the named
  `null_named` + `OpDatabase` pair via the `predeclared` set), and the
  Fix-A guard (`has_free_before_alloc` + `contains_free`) is deleted.
  Validated: 92×-E0425 `--html` build clean, full suite, headless GL
  render gate, 177 regression on both backends.


### F9-2 — registry native build-cache laundered stale cdylibs on a loft upgrade (2026-06-12, FIXED)

- **Invariant**: a cached native artifact is served only when the loft
  build that produced it is the one running (the `loft_build_fingerprint`
  rlib-content seam — @PLN11 Arc N / N0).
- **Homes today**: the fingerprint sidecar (the cheap gate) vs cargo's own
  fingerprinting underneath (the assumed authority).  The gap: cargo is
  structurally BLIND to a loft upgrade — the package crate's sources and
  the captured RUSTFLAGS can be byte-identical while the generated ABI
  changed — so the fp-mismatch-triggered `cargo build` no-ops, and
  `auto_build_native`'s success arm then stamped the NEW fingerprint over
  the UNREBUILT artifact: laundered as fresh forever (the store-65535
  sentinel-deref class at some later date).
- **Probe**: a REAL rlib change (temporary const in lib.rs) → the
  native-auto path rebuilt correctly; `~/.loft/build-cache/graphics-0.1.0`
  did NOT rebuild and its sidecar was re-stamped with the new fp.  (An
  earlier sidecar-corruption probe was unfaithful — cargo's no-op is
  CORRECT when the rlib genuinely didn't change.)
- **Fix (the chokepoint)**: `cache::clear_stale_native_target` — an
  existing artifact whose sidecar NAMES ANOTHER loft build clears the
  target before the build, so cargo cannot no-op over it.  Two
  suite-caught narrowings shaped the final scope: (1) an ABSENT sidecar
  does NOT clear — unknown provenance is commonly a legitimate hand-built
  artifact (the documented `cargo build` workflow; the tests/lib fixture
  cdylibs — the first cut wiped those and broke `native_loader`); (2) the
  clear applies only to the REDIRECTED root (`~/.loft/build-cache/…`),
  which is private to the locked resolution path — an in-tree
  `native/target` is shared with direct-path consumers (the fixture
  loaders read the `.so` without the lock; the second cut raced them
  mid-suite), and in-tree crates remain the documented dev workflow
  (`make rebuild-native-cdylibs` / the stub-panic hint).  `fp == 0` keeps
  the existence-only fallback.  Unit-tested
  (`cache::tests::clear_stale_native_target_clears_only_mismatched_artifacts`);
  e2e probed with two real rlib flips + a post-narrowing sidecar flip.
- **Residual boundary (by design)**: a SHIPPED prebuilt
  (`<pkg>/native/<libname>`) wins unconditionally in `resolve_native_lib`
  — that is the package author's pinned-artifact promise, not a cache.
- **The E0514 sibling, diagnosed + guarded (2026-06-12)**: which rustc a
  spawned build resolves is CWD-dependent under rustup (the repo's
  toolchain pin vs the default elsewhere).  Two build families differ:
  a PACKAGE native crate depends on loft-ffi (the C-ABI contract) and
  never the loft rlib — any rustc builds it correctly, cargo re-builds on
  toolchain flips, NO guard (a first cut guarded it and wrongly blocked
  valid /tmp builds) — while `build_shared_cdylib` (the auto-shared
  cdylib for in-repo libs) links the SVH-locked rlib DIRECTLY and is
  doomed under a mismatch (this was the E0514's true source, surfacing
  through main.rs' "could not compile native (…); interpreting it").
  Fix: `cache::rustc_mismatch()` (the memoised stamp-vs-live check,
  hoisted out of main.rs' native path into one home) consulted by
  `build_shared_cdylib` — a one-line reason + interp fallback instead of
  a compiler spew.  Flap-probed: 1.96-cwd builds the FFI crate ✓, refuses
  the rlib-linking cdylibs with the reason ✓; 1.95-cwd builds everything
  natively ✓; back to 1.96-cwd serves caches silently ✓.
- **The class's MILD symptom, diagnosed (2026-06-12)**: the
  `1 stores not freed: Scene×1` warning on `map_export_glb` runs was NOT
  a source bug — it reproduces exactly and only with a CROSS-BUILD
  loft↔cdylib pair (probed both ways: 5 consistent-pair cells × 0 leaks;
  an old binary against new-build cdylibs leaks Scene×1 on demand).  The
  ABI mismatch corrupts store-ownership accounting instead of crashing —
  the gentle face of the store-65535 class.  An earlier "pre-existing on
  main" conclusion was itself a cross-pair measurement (a baseline
  worktree binary run against current-build cdylibs).  The cache fix
  above closes the only user-reachable route; deliberate cross-pairs
  (worktree experiments, hand-built mismatches) remain possible and now
  diagnosable by this signature.

### The armed-channel restoration (2026-06-12) — four stale duals fixed, one LIVE release bug found

The armed-lib-debug channel (`profile.dev.package.loft`
debug-assertions=true) was globally red — worse than dead, it misled
(this session twice nearly drew conclusions from a silently-unarmed
binary).  Peeling the onion, 279/291 corpus files failing → **12**:

- **adopt_store half-cleared the @P317 bitmap-vs-flag dual** (F3 family):
  it cleared the slot's free *bit* but never the store's own `free`
  *flag*, so every armed `validate()` on an adopted bundle store died
  ("Using a freed store" in `ir_store::write_definition`).  Fixed at the
  registration site; release keys on the bitmap and never noticed.
  Callers audited: codec-only (workers adopt via separate plumbing).
- **`Store::open` constructed file stores as `free: true`** then
  self-validated → the warm-load twin of the same dual.  An opened
  file-backed store is in use by definition — flag cleared at
  construction.
- **The claims-vs-headers dual** (the store.rs row's other entry): a
  reopened file store has live headers but an EMPTY in-memory `claims`
  set, so armed `Store::valid` rejected every warm-load read.
  `read_only` is the check's designed exemption (worker stores) — the
  three ir_read loaders now route through one `adopt_read_surface`
  helper that sets it (also guarding the cached artifact against
  mutation).
- **I7 enforced a deleted allocator's invariant**: state/codegen's
  `validate_slots` call still passed scope-aware=V1 after @PLAN53 made
  the scope-blind V2 the ONLY allocator — false-positives on
  legitimately zone-2-placed small vars (`_reduce_acc_N`).  Both call
  sites now agree (skip I7).
- **The set_var width assert's model is incomplete** (pushed ==
  Variable-slot width fails for compensating put-op families: fn-ref —
  already noted in its own comment — Str→String text conversion, and a
  16 B call result into a 12 B DbRef slot in `t_4File_lines`).  Taught
  the text case; demoted the rest to a warning with
  `LOFT_WIDTH_ASSERT=1` escalation until a per-op width table exists.
- **`check_ref_leaks` was uniformly stale on the retbuf shape** — FIXED
  2026-06-12: the runtime trace showed the "missing" frees were
  `OpFreeRefIfDistinct` (the witness-pair conditional release of a
  ref-returning call's work ref) — the gate's `freed` collector matched
  only plain `OpFreeRef`.  It now counts the whole frees-arg-0 family
  (`OpFreeRef` / `OpFreeRefTag` / `OpFreeRefIfDistinct`); all ~130
  corpus false positives gone, no `LOFT_REF_LEAK_WARN` crutch needed
  (the env stays as a deliberate escape hatch for chasing runtime bugs
  past a known leak shape).
- **THE FIND — pass-2 arity growth is a LIVE release crash**: the
  restored channel's first catch.  A forward CALLER of a
  multi-return-site fn halts codegen with "Too few parameters on n_pick
  (got 2, need 4)" on BOTH backends.  CORRECTED mechanism (the rr
  traces): for a FORWARD callee the site's type is Unknown in pass 1,
  so `ref_return` never runs there at all — ALL growth happens
  pass-2-first, and a caller parsed earlier in pass 2 keeps the
  signature arity.  F1 (two-pass determinism) × @PLN55.
  **A per-site pre-provision fix was tried and REVERTED (2026-06-12)**:
  counting syntactic return sites in pass 1 and pre-adding
  `__retbuf2…N` fixed the 2-level cell — but the matrix's next cell
  showed buffer need CASCADES: a caller's single tail call materializes
  one work ref PER hidden buffer of its callee (`use_pick`'s one site
  → ls of three `__ref_N`), so no pass-1 syntactic count can bound a
  forward CHAIN (the 3-level cell crashed one level up).  The class
  fix needs a BETWEEN-PASS buffer-count fixpoint (the
  typedef-resolution slot runs after every pass-1 signature + body
  exists; resolve counts in dependency order) — or caller-LOCAL backing
  for the non-chained work refs so only one buffer ever promotes.
  Demonstrating tests (`#[ignore]`):
  `tests/issues.rs::pass2_arity_growth_forward_caller` +
  `…_forward_chain`; repros `/tmp/p_followups/p14_forward_caller.loft`,
  `p15_three_level.loft`.
  **DESIGN ROUND (2026-06-12) — the stated question is RESOLVED by
  probes; the fixpoint candidate is FALSIFIED:**
  - **Self-recursion diverges**: a multi-site fn whose tail calls
    ITSELF gives `buffers(f) = 2 + buffers(f)` — no finite fixpoint
    exists, every pass adds attrs, codegen halts ("got 5, need 7").
    No forward reference involved.  A between-pass fixpoint CANNOT
    terminate here, so it is unsound for the class, not merely
    incomplete (`casc_recursive.loft`,
    `tests/issues.rs::pass2_arity_growth_self_recursive`).
  - **Mutual recursion crashes** the same way — a call cycle always
    contains a forward edge, so no resolution ORDER fixes it
    (`casc_mutual.loft`, `…_mutual_recursion`).
  - **The per-site ABI leaks into the user-facing fn TYPE**: two fns
    with the identical declared signature `(text) -> S` cannot share a
    fn-ref variable — `fn(text, S) -> S` vs `fn(text, S, S, S) -> S`
    (compile error, `casc_fnref.loft`).  Hidden machinery visible at
    the language surface (Goal F); any multi-buffer design keeps this.
  - **Surviving design — ONE buffer per fn** (H1's own invariant made
    exact): *a heap-returning fn's arity is signature+1, fixed at
    declaration; every return path delivers its value in THE
    `__retbuf`* — chaining the site's call into it when safe, copying
    when the site's value aliases the buffer's current contents.
    Recursion becomes trivial (self-call passes own buffer); the
    fn-ref type unifies; the pass-2 growth assert becomes a true
    invariant.
  **BUILT 2026-06-12 for `Type::Reference` returns** (branch
  windows-probe): `ref_return`'s growth branch now binds later sites
  to the one buffer — a site's OUTERMOST work ref (its tail call's
  buffer arg, `site_value_ref`) substitutes to the buffer var across
  the body (`unregister_work_ref` keeps the orphan out of the preamble
  /free sets — a leftover free flips the tail-`If` emission into the
  @P378 `Return(Null)` trap) and gets the canonical
  `{ buf = call(...); buf }` value shape (`chain_site_set_shape` — a
  bare `{call; var}` block would make the call a witness-freed
  DISCARD); inner refs stay plain locals (binding both aliases the
  outer call's dest with its own argument — the C-a hazard,
  `casc_nested.loft`); named locals read by sibling sites deep-copy
  via `materialize_return_into`.  All four crash cells fixed +
  un-ignored (`tests/issues.rs::pass2_arity_growth_*`); 10-cell
  matrix interp + native green; armed corpus at the 4 pre-existing
  singletons; full suite green incl. the moros glb chain.
  **Found en route (pre-existing, filed)**: #355 — the Vector arm
  still grows per-site, and a forward caller of a multi-site VECTOR
  fn silently returns the WRONG element (both backends); the
  one-buffer binding must extend to the Vector/struct-Enum arms.
  #356 — mid-body `return f(g(x))` (lifted args) returns the null
  sentinel on native (interp masks via eval-stack-top; the scopes
  `Return(Null)` fall-through needs to learn buffer-valued tails).
  Also: fn-refs to struct-returning fns cannot be CALLED at all
  (pre-existing compile error "expects 2 arguments, got 1" — the
  CallRef path never fills hidden buffer args; surfaced by the
  fn-ref-type probe).  Lambdas keep in-place growth (sound — no
  earlier callers).

Remaining armed-corpus failures (5 files, no env crutch; the corpus =
tests/scripts + tests/docs + examples on the armed interp binary —
rebuild it with
`cargo --config 'profile.dev.package.loft.debug-assertions=true' build --bin loft`):

| Site | Files | Read |
|---|---|---|
| ~~`vector.rs:331`~~ | ~~`132-vector-elemset-inline-literal-uaf`~~ | **FIXED 2026-06-14** — see below |
| `codegen.rs:1871` | `166-p390-self-slice-assign` | **VERIFIED over-strict 2026-06-14** — see below |
| ~~`codegen.rs:2560`~~ | ~~`examples/collections.loft`~~ | **Both bugs FIXED 2026-06-14** (empty `{}` + dup-key vector+hash) — see below |
| `compile.rs:306` | `75-native-stub` | now clean on the rebuilt armed binary (the native-stub panic is that test's SUBJECT — expected-fail by design) |
| `control.rs:3487` (pass-2 growth) | `298-multi-return-site-ref-buffer` | the mapped arity-cascade class (see THE FIND above) |

**2026-06-14 armed-residual triage** (branch `stability2nd`; reproduce any of
these with `./target/debug/loft --tests <file>` on the armed binary — the CLI
`--tests` runner is the harness's multi-top-level-entry path, the live shape):

- **`132` — FIXED (was a SILENT use-after-free, interpreter only).**  A
  field-of-element SET to a struct literal carrying a vector
  (`xs[i].field = V{a: [literal]}`) built the nested vector IN PLACE by
  appending via `OpNewRecord`, reusing one element-ref temp `_elm_2` across the
  appends.  `generate_set`'s reassignment path emitted an UNCONDITIONAL pre-Set
  `OpFreeRef` of the old `_elm_2` — but an `OpNewRecord` RHS returns an INTERIOR
  ref into the vector's backing store, so consecutive elements share one store
  and the whole-store free killed the live backing.  Correct bytes survived
  until the next allocation recycled the freed store (NON-deterministic data
  loss under churn; deterministic only under the armed `get_vector` assert).
  The native generator's `if _old.store_nr != new.store_nr` guard already
  protected it.  **Fix** (`src/state/codegen.rs` `generate_set`): route an
  `OpNewRecord` RHS through the runtime-guarded post-free
  (`OpFreeRefIfDistinct`).  Regression: `tests/scripts/372-field-elem-set-
  nested-vector-uaf.loft` (armed-corpus member + release-CI over-fix baselines).
- **`166` — VERIFIED over-strict armed assert (no runtime bug).**  The
  `generate_set` first-assignment self-reference assert
  (`!ir_contains_var(value, v)`) fires on the @P390 text/ref-element self-slice
  (`v = v[a..b]`, `v: vector<text>`): `materialize_iterator` emits
  `v_set(slice_elm, *next)` where `*next` legitimately re-reads `slice_elm`'s
  slot (the materialization writes it before the outer Set re-reads), a benign
  slot reuse.  Stress (50 rounds × 20-elem text vectors) is correct on BOTH
  backends; the assert is over-approximating.  Lower priority (cosmetic for the
  armed channel) — left as a known over-strict assert under the feature freeze.
- **`collections.loft` — TWO bugs.  Bug 1 FIXED 2026-06-14; bug 2 is
  design-level, localized + documented, example corrected to sidestep it.**

  - **Bug 1 — empty `{}` keyed-literal silently corrupts. FIXED.**  loft's empty
    collection literal is `[]`, NOT `{}` (`{}` is an empty Void BLOCK).  In a
    collection-typed struct field, `Counter { by_text: {} }` lowered the `{}` to a
    0-byte empty block that UNDER-FILLED the struct record (sibling vector field
    mis-located → SIGSEGV interp / E0308 native; armed assert
    `generate_call: mutable arg expected 16B but Block "empty block" pushed 0B`).
    Fix (`src/parser/objects.rs` `parse_object_field`): in a collection-field
    position, accept `{}` as the already-primed empty collection and WARN toward
    `[]` (Goal F — warn + continue over silent corruption).  Regression
    `loft-suite/attic/scripts/373-empty-braces-collection-field.loft`.

  - **Bug 2 — duplicate hash key in a vector+hash sibling pair corrupts the
    vector. FIXED.**  `Stores::field` (`src/database/types.rs` ~149) deliberately
    LINKS an index-type field (`hash`/`sorted`/`index`) to a same-content sibling
    field via `other_indexes` — the "two views share records" secondary-index
    pattern (append to `vector<Word>` auto-maintains the `hash<Word[text]>`
    index).  A duplicate key (`["apple","banana","apple"]`) routed through
    `dedup_keyed`, which DELETED the displaced record to enforce the hash's
    unique-key invariant — but the vector still held it positionally → element [0]
    read null / `Unknown record` SIGSEGV (matrix: same struct + same element type
    + a `hash` sibling (not `sorted`) + a KEY-field duplicate; hash elsewhere or
    distinct keys were clean).  Fix (`src/database/search.rs` `dedup_keyed` gains a
    `secondary` flag; `structures.rs` `record_finish` passes `true` for the
    `other_indexes` loop): a SECONDARY index only UNLINKS the stale key→record
    mapping, never deletes the record — the `other_indexes` analogue of @P305's
    `keyed_field_is_linked` update-only path.  Semantics: the vector keeps every
    record (insertion order, duplicates included); the hash keeps one per key, the
    latest appended.  Regression `loft-suite/attic/scripts/374-vector-hash-sibling-dup-key.loft`.

**The `store.rs:1640` row (7 files) is RESOLVED 2026-06-12** — the
"keyed armed UAF" was not a use-after-free; the one assert site hid
THREE distinct mechanisms, two of them real out-of-record writes
silent in release:

- **A — header-as-`room` read via the wrong accessor** (the 4 keyed
  files): a hash bucket's `room` IS its record-size header
  (`hash.rs::add` claims `room` words), but `copy_claims_hash_body` /
  `remove_claims` read it through `get_u32_raw(cur, 0)`, whose
  `valid()` gate rightly rejects data reads below fld 4.  Fixed by a
  dedicated `Store::record_words(rec)` accessor (claims + liveness
  asserts kept, fld-0 allowed) now used by every room read in
  `hash.rs` + `allocation.rs` — the dual-use invariant has one named
  home.
- **B — parallel s_pos array stomped its own record header**
  (`22-threading`, `22c-par-sources`): `run_parallel_text`'s
  per-worker result array claimed `ceil(rows*4/8)` words and wrote
  from fld 0 — index 0 landed on the record's size header.  Fixed:
  claim `ceil((4+rows*4)/8)` words, read/write at `4 + idx*4`.
- **C — OpDatabase under-claims 1-byte payloads — a REAL release
  out-of-record write** (`292-pln17-three-state-boolean`):
  `Stores::claim` takes WORDS; both OpDatabase twins passed the type
  size in BYTES.  Probe matrix: size ≤ 1 byte (a one-boolean struct, a
  closure capturing one boolean) claimed 1 word total = header + type
  tag, ZERO payload bytes — every field write landed one byte past the
  record (silent in release); size ≥ 2 over-allocated ~8× and masked
  the class.  Fixed at both twins with the exact formula
  `1 + ceil(size/8)` (`state/io.rs::database`,
  `codegen_runtime::OpDatabase`); the native twin also gained the
  interp's B2 `enum_parent_size` (it used the variant's own size — a
  latent F4 divergence for unit enum variants).  Guard:
  `loft-suite/attic/scripts/299-single-byte-record.loft` (cross-mode).

Full release suite + armed corpus green after; the armed corpus is
down 12 → 5 files with no new sites.
