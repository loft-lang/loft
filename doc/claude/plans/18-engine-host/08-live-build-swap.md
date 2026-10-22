# Phase 08 — the live build swap: one loop for every host (design + test catalog)

**The vision (user-stated, 2026-06-10).** A totally compiled loft project
where, due to the debugger, parts can be pushed to the interpreter (in the
Rust layer), seamlessly debugged and even CHANGED (body-only edits — no
struct/enum layout changes for now), after which everything still works
with some functions interpreted — and then the server builds a new client
in the background and **swaps to the new build without stopping anything**.
The same loop holds for wasm: **the browser loads the new binary via
websockets and swaps to it.**

**This is the heart of the engine** (user, 2026-06-10: *"if we can do this
on both the native and the wasm build we are golden"*).  Every other piece
of the stack — the store, the dispatch table, the wire classes, the
debugger, the host layers — is judged by whether it serves this loop.

**The invariant.** *The build is replaceable; the state (the store) and the
connections (the host layer) persist.*  Everything below is that one
sentence, applied per host:

| Host | The persistent host layer | The replaceable build | State carrier |
|---|---|---|---|
| native | the OS + the next process (sockets via handover or seat reconnect) | the whole compiled binary | store snapshot at a frame boundary (the durable format) |
| browser | **the page** — `loft-rt`/`loft-gl` keep the WebSocket + canvas open across instances | the whole wasm module, fetched over the WS bulk channel | store export → import across instances (same format) |

The browser is the EASY case: its host layer already exists and owns the
sockets, so "swap" is fetch–snapshot–instantiate–resume with zero
connection churn.  Native must build the equivalent (cutover choreography).

This phase **supersedes 03's per-fn wasm modules as the promotion tier**:
for native hosts the background artifact is a plain full build (no wasmtime
dependency, no bridge tax, no per-fn bookkeeping), and for browsers the
whole-module swap inherits wasm's drop-an-instance unload safety anyway.
03's measurements stand (wasm exec ≈ 1.0× native); its per-fn residual is
retired.  Tier 0 (02's dispatch) is unchanged and remains the substrate.

**Bounds:** a live RELOAD (S2, S3) takes body-only edits — the signature
guard enforces it.  A build SWAP (S5, S6) also takes a changed layout: the
world goes through the store converter, which either carries it or refuses
and leaves the old build serving.

**The after-part — converting one store format to another on class
changes** is @PLN196's converter (step 8): both builds' schemas are values,
so the diff is mechanical — added fields take their defaults (which may read
the record through `$`), dropped fields drop, widenings convert.  A rename
is two swaps: add the new field with a default that reads the old one, then
drop the old one.  A change the converter refuses keeps the old build
serving.

---

## The pipeline, as scenarios with tests

Each scenario names its observable and its test.  The differential rule
(Goal D, extended): at EVERY stage the meaning output must be identical
across interp / mixed / compiled — only timing may differ.

### S1 — the compiled baseline serves the game  ✅ (2026-06-11)

A kernel program (`run`/`run_client`) built `--native`, driven by the
existing e2e clients.  **SHIPPED**: the kernel natives gained TYPED TWINS
(`engine_host::typed`, re-exported through `codegen_runtime`, registered in
`CODEGEN_RUNTIME_FNS`) sharing the kernel internals with the bytecode-stack
natives — one implementation, two calling conventions; the bodies that were
inline in the stack natives (listen/pump×2/tick×2/sync-next×2/client-send)
were factored into shared `_impl`/`_kernel`/`_client` helpers so the queue
machinery cannot fork.  `engine_host_kernel::s1_native_baseline_matches_
interpreted` runs the SAME fixture both ways — transcripts byte-equal.
Process-model finding: a `--native` run spawns the compiled binary as a
GRANDCHILD of the loft driver — test guards must kill the process group
(probe-caught: a rerun connected to the previous run's orphan).

> **Test (extend `engine_host_kernel`/`_audience`):** the same scenario
> transcripts as the interpreted run — the audience-differential pattern
> with the SERVER swapped for its native build.  Pass = byte-equal
> transcripts.

### S2 — the debugger pushes one fn to the interpreter, live  ✅ (2026-06-11)

Compiled callers call through the per-fn dispatch point; the target flips
native → interp at a frame boundary; nothing else changes.  **SHIPPED**:

- **The check lives INSIDE the callee** (`src/live_dispatch.rs` +
  `Output::live_entry_check`): every generated user fn with a dispatchable
  signature opens with `live_flipped(idx)` — one relaxed atomic load when
  off.  This answers the inlining gate **by construction**: an inlined call
  site inlines the check with it; no caller-side table to defeat.
- **The bootstrap world IS the program world**: under `LOFT_LIVE_FLIP=1`
  the generated `main` takes its `Stores` from a full parse of the same
  sources (`boot_stores`; the driver hands paths down via `LOFT_LIVE_SRC/
  _STDLIB/_LIBS`) and skips `init` — so the parked interpreter and the
  compiled code share ONE id-compatible world.  Probed before building:
  the whole probe program ran compiled over the parse-seeded world,
  byte-equal (the type-id-determinism claim held).
- **Sharing is a swap, not an alias**: each dispatch swaps the world into
  the parked `State`, runs `reenter`/`reenter_ret` (the 02 frame contract),
  swaps back.  Probe matrix green first run: args i64/f64/bool/record/
  vector × returns void/i64/f64/bool/DbRef × bodies (field writes / text
  format / iteration / **allocation inside the interp callee, consumed by
  compiled field reads** — the shared world round-trips both directions).
- **Runtime control**: `engine_host::live_flip(name, on)` (typed twin +
  no-op-false interp stack native), plus `LOFT_FLIP_FNS` startup flips and
  the `LOFT_DISPATCH_DEBUG=1` sentinel.
- v1 bounds: dispatchable = scalar/bool/float/record/vector args, void/
  i64/f64/bool/DbRef returns; text/char/fn-ref/`&mut` args, narrow-int/
  text returns fall through to compiled (no check emitted).  Flipped fns'
  CALLEES run interpreted too (bytecode world) — semantically identical,
  slower; per-callee re-dispatch back to native is N9/C71 territory.

> **Test:** `engine_host_kernel::s2_live_flip_under_serving_kernel` — a
> native kernel server flips `bump_events(w: W)` via a WS control input
> mid-serve; the world counter runs 1..=5 continuously across
> compiled→interp→compiled, the transcript is byte-equal to the
> interpreted leg, and the flip is visible ONLY in the sentinel.

### S3 — edit the interpreted fn; the mixed build keeps running  ✅ (2026-06-11)

Tier-0 reload against the compiled baseline: the shadow session parses the
edit, the dispatch target moves to the new bytecode, world state persists.
**SHIPPED** — the integration is two small moves at one chokepoint:

- `live_dispatch::bootstrap` installs the existing reload host
  (`live_reload::install`) over the SAME source path under
  `LOFT_LIVE_RELOAD=1`; `dispatch()` polls it with the world swapped IN
  (new const texts land in the real CONST_STORE) and BEFORE position
  resolution, so the dispatch that follows an edit already runs the new
  body.  Internally throttled (200 ms) — no per-call file stat.
- The flip table stores ONLY `d_nr`; every dispatch resolves the position
  through `state.fn_positions[d_nr]` — the one dispatch home the reload
  patch writes.  (Caught at design time: a cached `(d_nr, pos)` pair would
  have silently pinned the old body — the edit invisible forever.)

**Findings:**

- **Reload application is dispatch-driven in the mixed tier** — an edit to
  a flipped fn is only examined when that fn is CALLED (lazy by
  construction; correct, since the edit's only observable is its calls).
  Residual for S7: the debugger's structured reload feedback wants a
  control-channel poll-now rather than waiting for game traffic.
- **The lenient parser bounds what "broken edit" means**: a missing
  operand (`a + ;`) parses as warning + null (Goal F posture), so it
  RELOADS as a legitimate body.  Only `Level::Error` edits (unknown
  names, type errors) are refused.  A debugger UI should surface the
  warning stream on reload, not assume rejection.
- An edit while flipped, then un-flip → the COMPILED (old) body serves
  again — inherent to the mixed tier; S4/S5's rebuild-and-swap closes it
  (S5 sub-test 5 asserts the reset).

> **Test:** `engine_host_kernel::s3_live_edit_under_native_baseline` —
> native leg (flip + edit under a serving kernel) and interp leg (the
> original tier-0 path) pass the same milestones: step 1 → edit → step
> 100 with a MONOTONE world counter (no restart); broken edit → "kept its
> old body" while serving continues; signature change → rejected.
> Asynchronous application ⇒ the legs assert step timelines + reload
> milestones (stderr as sync point), not byte-equal vectors.

### S4 — the background rebuild  ✅ (2026-06-11)

The serve host compiles the full project (rustc, minutes) keyed to the
source hash, while the old build keeps serving.  **SHIPPED** — the build
pipeline already existed as `loft --check --native` (compiles into the
content-hash cache without running); S4 only orchestrates it:

- `live_dispatch` spawns the DRIVER (`LOFT_LIVE_DRIVER`, handed down at
  spawn like the other live paths) as a background child; stdout/stderr to
  temp FILES (a failed rustc overflows a pipe buffer and would hang the
  poll).  The artifact path now rides the ok line — `ok <src> <artifact>`
  — preferring the DURABLE content-addressed cache path over the per-pid
  temp the miss branch builds into (probe-caught: the temp is clobbered
  by the next same-pid run; the S5 swap needs the durable one).
- **The staleness contract is snapshot-vs-settled, not read-racing**: the
  loft source bytes are snapshotted at REQUEST time; completion compares
  the file as of the status call — wherever an edit lands relative to the
  child's own file read, drift requeues.  Converges when the source
  settles (already-cached content → instant).
- Surface: `rebuild_start() -> boolean` (idempotent while one runs),
  `rebuild_status() -> 0|1|2|3` (idle/building/ready/failed — requeue
  stays 1), `rebuild_artifact() -> text`.  Env-driven, not tier-driven:
  an interp host that exports the live env gets the real thing.
- Failure posture: a failed build warns with the rustc tail and the OLD
  build keeps serving — never a halt.

**Findings:** (1) the binary cache keeps ONE entry per source stem — a
different-content build EVICTS the previous artifact, so S5 must copy or
exec the artifact promptly after ready; (2) fixture-sized programs build
in under a second (reachable-only emission keeps the generated crate
tiny) — the "minutes" case is real projects; build isolation is an OS
process property either way, measured here across the real (small)
window.

> **Test:** `engine_host_kernel::s4_background_rebuild_under_serving_
> kernel` — (a) artifact lands; a repeat request on unchanged source is a
> cache hit on the SAME durable path; (b) the tick counter advances in
> every poll interval of a clean unique-content build; (c) an edit during
> a build requeues ("rebuild stale … requeued") and the ready artifact is
> the settled source's build.

### S5 — the native swap: a new process under a running world  ✅ (2026-06-11)

Cutover at a frame boundary: freeze meaning at tick N, snapshot the store,
boot the new build, hand over (or let seats reconnect), resume at N+1.
Rollback if the new build fails to boot.  **SHIPPED**:

- **Surface:** `swap_world(w)` — name the world record ONCE before `run()`
  (the only program-side line): it becomes the snapshot root, and on a
  resumed boot (`LOFT_RESUME`) the previous build's world is restored INTO
  it in place, so every alias (main's var, the handler captures) sees it.
  `swap_start(artifact)` requests the cutover; `run()`'s per-turn
  `kernel_swap_step` acts at the frame boundary (0 serve / 1 frozen / 2
  retire).
- **The snapshot is a store image** (@PLN196 step 8): when the world is the
  root of its own store, the old build writes that store as it is, with its
  layout sidecar recording the slot (`Stores::write_store_snapshot`), and the
  new build's `swap_world` loads it through
  the store converter (`load_path`).  Hashes, sorted collections and
  references survive; a changed struct converts in place, every added field
  taking its default (a computed one through the drain the parser puts after
  `swap_world`).  **Whether the world can be carried is decided when the next
  instance is COMPILED, never at the swap** (owner, 2026-10-21): `swap_world`
  records the world's layout, the background rebuild hands it to the build
  (`LOFT_CHECK_WORLD`), and the build derives the upgrade map before it
  generates any code.  A refused change fails the build with the reason, so
  `rebuild_status` reports failed and the swap is never offered; a build that
  passed carries a plan already known to hold.  The new build's own refusal
  at `swap_world` (it exits before ready, the old build keeps serving) stays
  only as the backstop.  A world that shares its store
  with other data goes over as JSON (`show_json` out,
  `populate_struct_from_jsonvalue` in: scalars, texts, vectors and inline
  structs; missing fields null, extra ignored).  `LOFT_NO_SWAP_IMAGE=1` sends every
  world as JSON — the first bisect step for a swap that loses or misreads
  data.  Guards: `s5_native_swap_under_running_world` (a world grown by a
  computed field) and `tests/store_upgrade.rs`
  `cell_swap_world_restores_and_converts_a_world_with_a_reference`.
- **Overlap handover, rollback-by-default**: the old build NEVER stops
  serving until the new one is proven.  `SO_REUSEPORT` lets the new
  process bind while the old still listens; the READY file (touched after
  a successful bind) is the proof; the old then closes its sockets and
  retires.  Child dies / never ready → warn + un-freeze + keep serving.
  During the freeze, mechanics stay alive (pump runs; meaning waits).
  The swap child gets ITS OWN process group (a handover target, not part
  of the retiring tree) and `LOFT_FLIP_FNS` is stripped (dispatch reset).
- **Probe-caught en route — `SOCK_CLOEXEC` on kernel sockets**: the raw
  `libc::socket` listener/UDP fds were inherited across exec by EVERY
  spawned child (rebuild driver, swap target); the zombie copy stayed in
  the REUSEPORT group and ate load-balanced SYNs into a backlog nobody
  accepts — post-swap dials failed by hash luck.  Kernel sockets belong
  to one process; both binds are now CLOEXEC.
- **Probe gate 5 verdict (socket handover)**: seat-reconnect, measured —
  the close→serving-reconnect gap is **~35 ms** on loopback (deadline-
  retried dial; the connector gained a 5 s dial-retry window), far under
  the 3 s keepalive timeout.  fd-passing stays unbuilt: zero-gap is not
  worth a unix-only fd + WS-framing-state migration at this gap.
- **Probe gate 3 verdict (durable live world)**: covered at the meaning
  level — the world graph round-trips through the snapshot under live
  churn (the counter crosses exactly); the byte-compare form belongs to
  the full-image store format if/when one is needed.
- v1 bounds: events arriving INSIDE the freeze window die with the old
  process (visible as the connection close — the client can resend on
  reconnect); user_args are not re-passed to the new build; capture-set
  changes between builds fall to the lenient deserializer's defaults.

**Connector half + THE WINDOW VERDICT (2026-06-11):** clients swap too —
`run_client` drives the same swap step (freeze keeps the server connection
AND the debug endpoint pumping), a client's READY form is "connected"
(its serving IS its connection), and the new `swap_retired()` lets a
reconnect wrapper tell retirement from a dropped server (without it the
projector's wrapper SPUN: redial → sticky Done → instant retire → 25k
loops — probe-caught live).  **Measured on the GL projector**: swap→retire
within the same second (≲1 s meaning-freeze, cache-hit artifact), and the
window does NOT blink — the NEW window opens during the old build's freeze
(boot order: window → restore → connect → READY), then the old closes:
an overlap, never an empty screen.  The world's hash fields restore to
defaults and the on-connect `6:` replay heals them (the viewer's
meaning-level recovery); scalars cross intact.  Post-swap the new build
re-announces its control endpoint and is immediately debuggable
(frame-stepping verified on the swapped process).  CI:
`s5_client_swap_under_running_world` (headless; world continuity via the
restored tick counter).

> **Test:** `engine_host_kernel::s5_native_swap_under_running_world` —
> ONE test drives the whole heart pipeline (S2 flip → rollback legs → S3
> edit → S4 rebuild → swap):
> 1. **State continuity** ✅ — the world counter crosses the cutover
>    EXACTLY +1 (nothing lost/duplicated); the tick stamp never resets.
> 2. **Event integrity** ✅ — every pre-swap message round-trips exactly
>    once (replied before the boundary by construction of the freeze).
> 3. **Seat continuity** ✅ — the WS seat's measured gap ~35 ms < the 3 s
>    keepalive timeout; the connector dial now retries across the gap
>    (native seats ride through a swap in a `while run_client` wrapper —
>    the full native-seat differential is an audience-harness residual).
> 4. **Rollback** ✅ — two legs: a missing artifact is refused outright;
>    a build that dies before serving rolls back while the world keeps
>    counting (the old build never stopped listening).
> 5. **Dispatch reset** ✅ — zero `live-dispatch:` lines after the
>    handover marker (the flipped fn runs compiled in the new build).

### S6 — the browser swap: a new module under a living page  ✅ (2026-06-11)

The server pushes the new build over the wire; the page snapshots the
world out of instance A, instantiates B, imports the world, resumes —
the SAME WebSocket object, no reconnect.  **SHIPPED** (the
interpreter-bundle tier: the SCRIPT is the build, the wasm module is the
substrate — the compiled-module variant arrives with the --html/kernel
integration, listed below):

- **The gate fell in two pieces.**  (1) The `Instant::now` panic was a
  timing breadcrumb in `compile::byte_code_from` whose CLOCK READS ran
  even with `LOFT_TIMING` unset — now gated with the print.  (2) Beneath
  it: the 07 page was written against the WRONG API — `compile_and_run`
  is run-to-completion (a frame-yield silently DROPS the parked state);
  the frame-yield pairing is `compile_and_start` + `resume_frame`.  The
  07 one-script differential is fixed + UN-IGNORED (07 fully closed).
- **The page is the persistent host layer**: `loft-rt.js` gained the
  swap hooks — `B!:`-framed build pushes are consumed AT THE HOST (the
  running script never sees them) and handed to `host.onBuildPush`;
  `host.ws_adopt_next(id)` makes the next `ws_connect` ADOPT the living
  socket instead of dialing (B resumes on the SAME WebSocket object);
  `host.ws_opens` counts real constructions (the identity assert);
  `host.onSend` gives the page meaning-level observability.  En route:
  `host.ws_ready` read an undefined table and THREW on every call —
  latent until the adoption path needed it.
- **The wasm-side halves** (`swap_export`/`swap_stage` in wasm.rs +
  `swap_world` registered for the browser): the SAME `swap_world(w)`
  surface as native — the page exports the world of the PARKED instance
  (schema-walked JSON, the S5 snapshot format verbatim), stages it, and
  B's `swap_world` restores it in place.  One snapshot format, three
  hosts.
- **Probe gate 6 verdict (store export/import)**: at world scale the
  JSON snapshot is microseconds — the freeze budget is dominated by B's
  compile (~100–300 ms interpreter-tier), well inside a level-load
  moment; the page never blocks the event loop (the swap runs between
  frames).

> **Test:** `engine_host_connector::s6_browser_swap_under_living_page` —
> headless-chromium; the kernel server relays a pushed build blob
> (`B!:<fnv64>:<script>`, the bulk-channel role, content-agnostic):
> (a) socket identity: ONE open ever (B adopted A's socket) ✅;
> (b) world continuity: B's counter resumes from A's (sync-class
> freshness semantics across the gap, per design) ✅;
> (c) B's marker appears and advances within the window ✅;
> (d) a corrupt push (bad hash) is rejected and A keeps running ✅.
> The page THROWS on any unmet clause with its deadline INSIDE the
> harness window (negative-control verified: no pushes → harness fails).

**The world through the converter (@PLN196 step 8):** the page asks `swap_check(files)`
before it disposes A — B is compiled and checked against A's world, so a change the converter
refuses leaves A serving and the reason in the page — and a world that owns its store goes
over as its image and layout sidecar (`LOFT-IMAGE`), which B's `swap_world` loads through the
converter: an added field takes its default, computed ones included, as on native.  A world
sharing its store still goes as JSON.  The test pushes a refused build (a field changes kind)
and a grown one (`bonus = $.count * 10`, checked against the count at the swap).

**Residual (compiled-module variant):** swapping a whole `--html` wasm
module under the page (store export across INSTANCES rather than runs)
— gated on the --html pipeline meeting the kernel scripts; the host
hooks and the snapshot format are already module-agnostic.

### S7 — the debugger loop end-to-end (@PLN16 6b/6c convergence)  ✅ (2026-06-11)

Breakpoint in a compiled fn → flip + pause (the debug suspension loop
keeps mechanics alive: a mechanics-only mini-pump answers keepalives) →
frame bindings into the REPL over the control channel → edit → resume
mixed → background rebuild → swap → still debuggable.  **SHIPPED**:

- **The control channel** rides the game port: `D!:`-prefixed frames are
  kernel-handled (the script never sees them), gated on
  `LOFT_DEBUG_CONTROL=1` AND a loopback peer.  Commands: `bp <fn>`,
  `flip <fn> <0|1>`, `eval <var>`, `resume`, `reload`, `apply` (the
  editor's Apply: a held edit under `LOFT_LIVE_RELOAD=hold` lands at the
  next poll, through a pause too), `rebuild`, `rebuild?` (a failed build
  answers `3 <why>`), `swap auto|<path>`.  Transport lives in engine_host;
  SEMANTICS in live_dispatch — split by a MAILBOX, because a paused
  dispatch holds the `LIVE` borrow and the control handler runs inside
  the pause's own mini-pump (`try_borrow` direct path when free; posted
  and applied by the pause loop when held).
- **The pause**: `State::reenter_dbg` drives a dispatched call with
  `debug_step(Continue)`; a breakpoint SUSPENDS it and the pause loop
  (holding `&mut State` legally, between resume steps) notifies `D:hit
  <fn> <bindings>`, answers `eval` from the captured frame (the M5e
  `BreakHit` renderer — v1 eval = frame-variable lookup; full
  expressions are the @PLN14 store-resident-env upgrade), applies
  reload-now (the S3 finding closed: the world is swapped IN at a pause,
  so the reload host sees the real CONST_STORE), and pumps kernel
  mechanics so keepalives never lapse.  Probe-caught: `debug_step`'s
  first-op skip (correct when resuming FROM a pause) silently stepped
  over a breakpoint AT the entry pc — `reenter_dbg` checks the entry
  explicitly.
- **Breakpoint identity is the FN NAME** (entry breakpoints v1): bps
  re-resolve through `fn_positions` (`set_breakpoint_fn_current`) after
  every reload — offsets move, identity does not.  Line-keyed bps are a
  residual: a reloaded body's line numbers come from the reload
  snippet's own parse, so line identity needs source-mapping work.
- `bp` implies the flip (a compiled body cannot pause).  The lambda
  boundary surfaced as designed: an edit touching `main`'s lambdas
  warns and keeps the old body — named-fn edits reload.

**Connector half (2026-06-11):** a CLIENT process announces its own
loopback control endpoint under `LOFT_DEBUG_CONTROL=1` (it has no game
port to dial); the command core is process-agnostic and replies are
role-routed; the pause's mini-pump pumps the client kernel (server
keepalives + the control endpoint) so a frozen GL frame never drops its
connection.  Verified live on the audience projector — entry breakpoints
on a per-frame fn give FRAME-STEPPING for free.  `D!:quit` is
reply-then-exit (sending inside the pump's borrow would re-enter the role
cell — probe-caught).

> **Test:** `engine_host_kernel::s7_debugger_loop_end_to_end` — the
> scripted session, every stage in order: hit with bindings (the game
> reply HELD while mechanics stay alive), `eval w`, edit acknowledged
> THROUGH the pause (`D:reload applied`), resume → the held call
> completes on the old body (append-only), the next call runs the new
> body AND hits the re-resolved breakpoint, rebuild driven to ready over
> the channel, `swap auto`, seats reconnect, the debugger re-arms, and
> the post-swap hit shows the RESTORED world (events=101) in its
> bindings before the new build serves 201.

### S8 — the standing differential (cross-cutting)  ✅ (2026-06-11)

> **Test:** one meaning scenario executed in four states — interpreted,
> compiled, mixed (post-S3), post-swap — transcripts byte-equal.  This is
> Goal D's sweep extended to the mixed states; it pins "a target change
> is observable only as speed" permanently.

**SHIPPED** — `engine_host_kernel::s8_standing_four_state_differential`:
the canonical bump scenario (`a b c d` → `got:x#N`) in four tier states —
interpreted / compiled / mixed (`LOFT_FLIP_FNS`, sentinel-verified) /
post-swap (a control-channel **SELF-SWAP** between `b` and `c`: rebuild
the unchanged source → a cache-hit artifact → swap — the process is
REPLACED mid-sequence and the transcript must not show it).  Per-leg
positive controls keep the tiers honest (the mixed leg must really
dispatch interp; the swap leg must really restore the world and hand
over).  Locked into CI with the rest of the suite.

---

## Scenario scoreboard — ALL EIGHT SHIPPED (2026-06-11)

S1 ✅ compiled baseline · S2 ✅ live flip · S3 ✅ live edit (mixed) ·
S4 ✅ background rebuild · S5 ✅ native swap (~35 ms gap) · S6 ✅ browser
swap (living page) · S7 ✅ debugger loop e2e · S8 ✅ standing four-state
differential.  **The heart of the engine is built and pinned.**

Open residuals (each with its trigger):
- **Line-keyed breakpoints** — reload snippets re-number lines; needs
  source-mapping.  Trigger: the @PLN16 editor wanting mid-body stops.
- **Compiled-`--html` browser module swap** — store export across wasm
  instances; host hooks + snapshot format already module-agnostic.
  Trigger: the --html pipeline meeting the kernel scripts.
- **Full-expression eval at a pause** — @PLN14 store-resident env;
  v1 is frame-variable lookup.
- **Snapshot coverage** — reference/hash/sorted world fields restore to
  defaults (the lenient-serialization growth path).
- **Mid-freeze event loss** (S5) — events arriving inside the swap
  window die with the old process; an event journal or fd-handover
  shrinks it.  Trigger: a consumer that cannot tolerate the ~1 s window.

---

## Load-bearing claims to falsify before building (the probe gates)

1. **Re-entry is a dispatch, not a rewrite** (S2) — the 02 slice-1 matrix.
2. **Generated call sites can route through the table** without measurable
   cost when un-flipped (S2) — emit-level probe + the inlining check.
3. **The durable store format round-trips a LIVE world** (S5/S6) — churn a
   world while snapshotting at a boundary; byte-compare after restore.
   (store_durable tests are the seed; the new axis is "mid-run".)
4. **Build-while-serving does not disturb the tick** (S4) — rustc at full
   load vs the stamp chain.
5. **Socket handover choice** (S5) — probe BOTH: fd-passing (zero-gap,
   unix-only, complex) vs seat-reconnect (bounded-gap, portable, needs
   connector reconnect semantics).  Measure the gap; pick per evidence.
6. **Store export/import across wasm instances** (S6) — size + time at a
   realistic world; the page freeze budget is one frame at 30 Hz.

## Sequencing

02's re-entry probe remains the first build step — every scenario stands
on the dispatch table.  Then: S1 (cheap, mostly existing tests) → S2/S3
(the debugger substrate, converges with @PLN16 6b/6c and the control
channel) → S4 → S5 (native swap) → S6 (browser swap; gated on the wasm
time fix) → S7 wired through → S8 locked into CI.  05c's bulk channel can
land just-in-time before S6 — the build artifact is its first consumer.
