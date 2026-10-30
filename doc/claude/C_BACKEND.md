
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
loft --native-c prog.loft [args]   # emit, build with LOFT_CC / CC / cc, cache the binary in
                                   #   .loft/cache/native-c-<key>, run it (exit 2 on a refusal);
                                   #   --ship is -O3, --native-debug -O0 -g,
                                   #   --native-emit [out.c] writes the C; LOFT_NATIVE_BACKEND=c
                                   #   makes --native mean this; LOFT_KEEP_NATIVE_C=1 keeps the .c
LOFT_C_LINT=1 loft --native-c p.loft  # compile afresh with -Wall -Wextra (minus the unused
                                   #   family) instead of -w and print the warnings; =strict makes
                                   #   each an error.  On request only, never in a build or a gate:
                                   #   the instrument for a large C99 program once there is a reason
loft --emit-c out.c prog.loft      # write the C; exit 2 with "does not carry this program
                                   #   yet: <fn>: <why>" for a construct not carried
cc -std=c99 out.c -o prog -lm && ./prog
python3 scripts/c_sweep.py         # every corpus program: C against the interpreter
python3 scripts/c_sweep.py --asan  # the same, built with AddressSanitizer + UBSan
python3 scripts/c_sweep.py --poison  # the store poison modes, against the interpreter's
python3 scripts/c_sweep.py --verbose FILE...   # both outputs of each differing program
python3 scripts/c_sweep.py --mode default      # one macro mode instead of all three
python3 scripts/c_sweep.py --all               # every refusal reason and program, not the top 25
python3 scripts/c_sweep.py --stderr            # stderr compared too (a fault's message, location
                                               #   and chain), the driver's `advice:` notes aside
python3 scripts/c_sweep.py --shard K/N         # every N-th program from K: ci.yml's `c-sweep`
                                               #   runs four shards per PR, miri.yml the
                                               #   --poison and --asan sweeps nightly.  The PR's
                                               #   Test job and its Language suite job compile
                                               #   `--native` through the C (LOFT_NATIVE_BACKEND=c,
                                               #   on every leg)
make c-sweep ARGS="…"                          # the release loft built first, then the sweep
python3 scripts/image_probe.py                 # every corpus program: the C written from the
                                               #   program IMAGE (LOFT_EMIT_FROM_IMAGE=1) against
                                               #   the C written from memory, byte for byte (@PLN205)
./scripts/find_problems.sh --subject c         # the C backend's guards (c_emit, c_https) —
                                               #   run it before pushing a writer change as
                                               #   well as the sweep: c_emit COUNTS emitted
                                               #   constructs, and a new holder or twin moves
                                               #   a count while every output still matches
python3 scripts/c_sweep.py --wasm              # the C built for wasm32-wasi, run under wasmtime
python3 scripts/c_sweep.py --no-limit          # no kill at three times the last sweep's time
LOFT_C_WHERE=1 loft --emit-c out.c prog.loft   # a refusal also prints the statement it met
python3 bench/stats.py --lanes rust,c,cship    # every bench program as C, against its Rust twin
```

The C binary reads the run bounds itself: `LOFT_MEMORY_LIMIT` caps the bytes the stores' buffers
hold (`loft_heap_moved`, called before every resize of one; a growth past it stops the run with
101 and the report naming the type and the growth), and `LOFT_TIMEOUT` stops the run with 124
at the deadline (`alarm` under POSIX; without it the driver's own bound ends the program with
itself).

The log calls write what the interpreter's `Logger` writes (`loft_log.h`): the same config
search (`LOFT_LOG_CONF`, `.loft/log.conf`, `log.conf`), records, level overrides, rate limit,
rotation and reload, and `production = true` turns a failed assert or a panic into a logged
record the program continues past, the run then exiting 1.  The log file is opened at the first
record; the interpreter opens it at start, so only a program that never logs differs (no empty
`.loft/log.txt`).

`bench/stats.py`'s `c` lane builds each bench program as C (`-std=c99 -O3`) and requires its
result hash to be the Rust twin's — a disagreement fails the run — and reports its time over the
twin's; `cship` builds it under `LOFT_RELEASE_PASS_PROBE=1`, where the generator writes the
release-pass probe's forms (`loft_wrap_*` in `loft_rt.h`: the processor's arithmetic with every
wrap defined, for exactly the operators the Rust probe changes) — the ship tier's ceiling, as the
Rust `ship` lane has it ([bench/README.md](../../bench/README.md) § The C lanes).

`c_sweep.py` reads `target/release/loft` (or `LOFT_BIN`) for every program, so a rebuild while
it runs mixes two compilers into one result: build first, or point `LOFT_BIN` at a copy.  It
builds every carried program in the three macro modes (below) and puts it in one bucket: `match`, `differ`, `cc-error`, `sanitizer` (`--asan`), `slow` or `refused`, and ranks the
refusals by reason, each with the programs it refused (five, or all of them under `--all`; the
full list in `refused.tsv` beside the run's directories) — that ranking is the work list.  Each program gets a directory of its own
(its unit `p.c`, its binary, and a `tmp/` that is its `TMPDIR` in every run) under the sweep's
scratch, `<temp>/loft-c-sweep-<checkout>/<pid>/` (`scripts/run_scratch.py`): a program that
matched or was refused loses it at once, every other keeps it and the report prints its path
under the verdict, to read the C of a slow or differing program; the next sweep removes it
first.  Every run's time is filed per mode
set (`bench/portal/local/c_sweep/<modes>.tsv`, uncommitted); the next sweep of the same modes
gives each run three times its last time (never less than ten seconds over it) and kills it
past that as `slow` — a hang shows in seconds rather than at the 300 s bound, and its time is
not filed, so the limit holds until it is fixed (`--no-limit` runs to the bound).  Beside the
verdict it lists the runs three times slower than the last sweep and the compiled runs three
times slower than the interpreter — a report, never a verdict.  `--wasm` builds each unit with clang for
`wasm32-wasi` instead and runs it under wasmtime: setjmp through the exception-handling proposal
in its standard encoding (`-mllvm -wasm-enable-sjlj -mllvm -wasm-use-legacy-eh=false`, wasi-libc's
`-lsetjmp`, wasmtime's `-W exceptions=y`), `clock` and `signal` from WASI's emulation
libraries, an 8 MiB stack (wasm-ld's default is 64 KiB), the program's directory and the
temporary directory mapped under their own resolved paths, and the host's environment.  WASI
counts as POSIX in `loft_cc.h` for files, directories and the monotonic clock; what it lacks —
sockets, `mmap`, `realpath` — stands under `LOFT_POSIX_HOST`, so a mapped file is read and a
network fetch answers that the build has no transport.  It
needs clang, lld, wasi-libc and the wasm32 compiler-rt (Ubuntu: `clang lld wasi-libc
libclang-rt-dev-wasm32`); a program linking a `[c] shim` archive is refused, the archive being
built for the host.

**The browser page** (`--native-c --html`, @PLN203 C6).  The C writes the module and the one
`--html` page builder hosts it, as it hosts the Rust writer's: fonts, `[[embed]]` files, library
`host_js`, the import stubs, `wasm-opt --asyncify`.  Three things make the module fit that page.
A library native the page supplies is a `loft_gl` import under its symbol, with the signature the
Rust writer declares — one contract, `src/host_imports.rs`, both writers read
(`c::browser_import_native`); one with no import, a text or heap answer, is refused at build as
the Rust writer refuses it.  The program starts from the exported `loft_start`
(`c::emit_browser_program`), which runs the libc's constructors once and flushes what it
printed, since a page never calls WASI's `_start`; beside it the unit reserves its own asyncify
save region and exports where it is (`loft_asyncify_data` / `_size` / `_ok`), as the
Rust writer's does — the page's controller never picks an address.  And a unit that binds no lazy collection is
built without setjmp (`LOFT_NO_JUMPS`, `LOFT_JMP_BUF`/`LOFT_LONGJMP` in `loft_rt.h`), so without
wasm's exception handling, which asyncify cannot carry.  The page answers the WASI libc with
`doc/loft-wasi.js` — output into the page, clocks, randomness, and files over the page
filesystem preopened as `/` — which `tools/wasm_repro.mjs` loads too.  Guards:
`html_wasm::the_c_page_answers_what_the_rust_page_answers`,
`html_gl_imports::the_c_page_imports_the_vector_argument_host_import`,
`c_emit::native_c_html_page_runs_its_program`, and Brick Buster's C page under the browser
render gate (`html_render::brick_buster_c_page_renders_without_console_errors`, nightly in the
`gallery` job after `make game` draws the pack).  CI installs clang, lld, wasi-libc and the
wasm32 compiler-rt for these; without them each skips.
`differ`, `cc-error`, `sanitizer` and `slow` are defects of the backend and must stay at 0; a change
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
| `loft_io.h` | files and the filesystem |
| `loft_lazy.h` | lazy stores: the bindings, their pins, the fault channel, the miss path (`LOFT_LAZY`) |
| `loft_https.h` | the client connection (a socket, TLS over it), the one HTTP/1.1 client, the WebSocket handshake's SHA-1 and accept key |
| `loft_net.h` | loads over a URL: `file://`, an HTTP GET, a page by `Range`, SHA-256 (`LOFT_NET`) |
| `loft_web.h` | the web library's natives: HTTP, a WebSocket client, the pack builder (`LOFT_WEB`) |
| `deliver.rs` | `deliver(tag, value)`'s byte form: one walker per type the value reaches |

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

The C runtime's stores are the standard of [STORE_MODEL.md](STORE_MODEL.md) (@PLN206): one
model for both runtimes, its open divergences numbered there.

**Shapes.**  Every loft value has one C shape (`CType`): `int32_t` for a local whose type
fits 32 bits in a body written under the 32-bit regime (null `LOFT_NULL32`; widened where the
runtime takes `int64_t`, and computed in 32 bits where both operands are —
[WIDTH32.md § The C backend](WIDTH32.md#the-c-backend), `LOFT_NO_C_INT32=1` turns it off),
`int64_t` for every other integer
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
of it goes through the pointer; a re-point (`@FR-B-Ref-Repoint`) points it at another variable,
a tuple member, or what another link names.  A link into a STORE place (a field, a vector
element) is a store link: a `loft_ref` to the place, read and written through the place's own
get and set — a local bound only to places is one, and a call that hands a place to a `&`
parameter calls the callee's PLACE TWIN (`<name>_pl<mask>`, `--native`'s `_st` twins), the
function written again with that parameter a store link; the IR's own text instances
(`n_app@st1`, `parser/store_text.rs`) take their place the same way, and a function value's
dispatcher has a place variant per signature.  A link that moves between a variable and a
store place whose bytes ARE the C value (an 8-byte integer, a float, a character, a boolean, a
plain enum) holds the place's address, as `--native` does; a narrow or a text place reached
that way is refused.  A text result's hidden buffer is a link the caller makes for the call.  A
rebind of a `&` record releases the record it displaces.

**Held vectors.**  Before a loop the C asks native's loop verdict, `generation::hoist::hoistable`,
with native's tier switches (`lower::switches`), and holds each vector it names in a `loft_vh`
(store, record, length, element base); the loop's element reads, in-place writes and `len` go
through it.  The verdict names the vectors the body indexes and the ones an admitted callee —
store-free or writing only in place — indexes (`@FR-R-Inputs`): such a call goes to the callee's
`__inv` twin (`INV_BIT` in the twin mask), which takes the caller's headers as parameters and
serves its own accesses from them.  A loop that grows a store (a push, a mint, a text set) keeps
the record and the length, which no growth moves; it keeps the base only of a vector whose store
`StoreFacts` proves apart from every store it grows (`@FR-R-Base`), and derives every other
base again at each access (`loft_vh_fresh`, one load).  A loop that grows the return buffer and
would hold a parameter's vectors if their stores were apart is written twice (`@FR-R-Alias`'s
versioned clause, native's `distinct_version`): the copy assuming them apart runs when a test
of the store numbers says so at its entry, the plain one otherwise.  A path the verdict mints
records into (`mint_pushes`, `@FR-R-PushRec`) and no window serves takes a record push header,
`loft_ph` — record, length and room, no pointer, so two vectors of one store each keep theirs
while the other grows: a mint takes the slot past the length, one that does not fit grows the
record the runtime's way, and the finish writes the length back, so the store answers every
other read and no exit syncs anything; the group's reservation is then nothing.  A record
scalar the verdict proves no statement of the loop writes (`@FR-R-Scalar`) is read once into a
local before it.  Each held access carries
`LOFT_PROVED(loft_vh_same(…))`, so `LOFT_VERIFY` catches a held vector that changed.
`LOFT_NO_VECTOR_HOIST=1` holds none, `LOFT_NO_CALLEE_INPUTS=1` writes no twin,
`LOFT_NO_DISTINCT_VERSION=1` no second copy, `LOFT_NO_DISTINCT_GROWTH=1` keeps no base under
growth, `LOFT_NO_RECORD_PUSH=1` binds no push header and `LOFT_NO_SCALAR_HOIST=1` holds no
scalar, as for the Rust writer; `tests/c_emit.rs`
(`a_loop_holds_its_callees_headers_and_rederives_a_base_under_growth`) counts each form against
hand-computed values under `LOFT_VERIFY` and AddressSanitizer.

**Adopted return buffers.**  A vector-returning function whose result local is delivered
through the hidden return buffer builds in that buffer (`@FR-R-RetAdopt`): the C reads the Rust
writer's own analysis, `generation::hoist::ret_adopt`.  The local's bind is the buffer — minted
as its witness would have been when the caller offered none, emptied when it did — the witness
store is never minted, and the exit's clear-and-append copy emits nothing.  The held vectors
above stay sound beside it: the verdict knows the adopted local as the buffer (`StoreFacts`'
`adopted`), and a parameter's vector is held across the buffer's growth only in the versioned
copy, whose entry tested the stores apart.  `LOFT_NO_RETBUF_ADOPT=1` keeps the copies, as for the
Rust writer; `tests/c_emit.rs` counts the adopted binds both ways against hand-computed values.

**Push windows.**  A counted push loop the Rust writer's admission passes
(`hoist::push_loop` + `push_window_ok`, or `mint_loop` + `mint_window_ok` for record mint groups)
reserves its pushes once and opens a `loft_win` on the vector — element 0's address, the length
reached, the room — and every push of that path is a compare, a store and a bump; a record mint
takes the window's next slot.  The record's length is written back at the loop's exit, and the
cold growth arm (a locked store, an absent vector) is the ordinary append.  `LOFT_VERIFY` checks
each push against a fresh derivation.  A mint of a type `hoist::complete_writes` proves every
literal writes completely copies no default image (`@FR-R-CompleteWrite`), unless the function
copies a record into a fresh element's field, where a null source leaves what the fill wrote.  `LOFT_NO_PUSH_FILL=1`,
`LOFT_NO_PUSH_WINDOW=1`, `LOFT_NO_RECORD_PUSH=1` and `LOFT_NO_COMPLETE_WRITE=1` turn the pieces
off, as on native.  A loop the counted rule declines whose scalar pushes, at any depth, reach
ONE path takes a DEEP window where `hoist::deep_window_ok` admits it — the root exclusive,
nothing else in the body naming it or writing a store, no exit but the loop's own end — with no
reservation: a push that does not fit takes the growth arm.  Native builds its deep window on
a held push header, so it opens fewer; the soundness question is the analysis's, and the C
answers it the same.  `LOFT_NO_DEEP_WINDOW=1` keeps the counted windows only.  `[x; n]` fills
as the Rust runtime does: one exact resize, the template copied in doubling blocks.

**Held file cursors.**  A loop that reads one `File` through fixed-width integer reads, where
nothing else in it can see the record (`lower::file_hold::held_file`, `@FR-R-FileHold`), keeps
the record's cursor in a `loft_fcur`: the format, the handle and the position read once, the
bytes served from a 4 KB buffer that a miss refills with one `fread`, and the position given
back on every exit — the loop's end, a `break` or `continue` past it, a `return` (loft has no
exception, so these are all of them; Rust's guard does the same by its drop).  The stream's
own position is recorded as it stands, so a read through the record afterwards seeks to its
`#next`.  An unheld read of up to 15 bytes lands in a stack buffer.  `LOFT_NO_FILE_HOLD=1`
reads every value through the record, as on native.

**Text walks.**  Four native rewrites, each read from the analysis native uses.  A
`for p in t.split(c)` loop takes its pieces from the text through a `loft_ls`, with no vector
built (`@FR-R-LazySplit`, `hoist::lazy_splits`); a source the function writes is walked as a
copy.  A `parts = t.split(c)` read only by its length, its element reads and a walk keeps
the same pieces as slices of a copy of the text in a `loft_stab` (`@FR-R-SplitTable`,
`hoist::split_tables`): the vector is never built and its buffer never minted.  A walked
element, a `?` discharge of a table's element (`hoist::borrowed_discharge_temps`), or a text local every bind of which is a view shape (another
borrowed text or a text parameter, a literal, a field read, a block ending in one), is held as
a `loft_str` view instead of an owned copy (`@FR-R-TextBorrow`,
`opt::text_borrow::borrowed_text_walks` and the plain clause of `hoist::borrowed_store_texts`);
a bind that would view a temporary is refused by name, never left dangling.  A text FIELD that
is an operand of a comparison, an append, a format or a print, or the argument of a callee that
writes no store (`Cx::call_keeps_stores`), is read where the store holds it when no
later operand may write (`FnWriter::text_reads`, R-TextBorrow's operand clause); every other
field read copies (`LOFT_NO_FIELD_VIEW=1` copies them all).  A choice
between such views or literals, as a discharge writes it, is the view it picks, and a
formatted value is appended from its work buffer, as native borrows it.  A `for c in t`
walk takes an ASCII byte without decoding it and asks its null test once
(`@FR-R-CharWalk`, `opt::char_walk::char_walks`).  `LOFT_NO_LAZY_SPLIT=1`,
`LOFT_NO_SPLIT_TABLE=1`, `LOFT_NO_TEXT_BORROW=1` and `LOFT_NO_CHAR_WALK=1` turn them off,
as on native.

**Float text.**  `(F-Float-Text)` (@C156): a value of at most 15 significant digits (6 for a
single) between 1e-6 and 1e15 is the integer nearest |v|·10^k for the least k whose correctly
rounded quotient reads back — no `printf` (`loft_push_shortest_fast`); otherwise the shortest
form takes `printf`'s nearest p-digit candidate from 15 digits up (6 for a single; from one digit for a subnormal), else its
neighbour on the value's other side at the same length; `.P` up to 9 decimals is exact integer
arithmetic; a parse takes Clinger's fast path where the significand fits 2^53 and the power of
ten 10^22, and an integer in JSON is read in place.  Decimal integers are written digit by digit.
Where none applies, `printf` / `strtod` answer, so the C library must round
correctly — glibc and wasi-libc do (both sweeps).

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
BIGMIN, the prefix walk — snapshotted as `build_rec_scratch` lays them out.  A hash with ONE
8-byte integer key looked up by that integer takes `loft_h_find_i64` (`@FR-R-TypedKeyed`): the
same digest, home bucket and walk as `loft_h_find`, the key compared in place; the digest of one
integer key, in every path that hashes one, is SipHash's two words and finish unrolled
(`loft_digest_i64`), which `the_c_hash_table_files_every_entry_where_hash_does` pins to
`hash.rs`'s placement.  `c[k…] = T{…}` is written into the element the collection claims for
it (`@FR-R-InPlaceLiteral`'s keyed clause, `loft_keyed_place_begin` / `_finish`), and over a
hash keyed by one to four integer parts whose element owns no heap, into the entry one probe
finds or files (`@FR-R-KeyedEntry`'s subscript clause, `loft_keyed_entry_ints`).

**Store images.**  `store_persist_copy` is `Stores::persist_copy` ported: the root deep-copied
into a scratch store, its trees laid out in van Emde Boas order (`rtree_relayout`), the store
written from word 0 to past its last live record, padded to the 1024-word floor, and the
`.dschema` sidecar beside it — its text prepared at emit time per root type
(`LayoutIdentity::of_store`), the slot line filled in at run time.  `store_load` is `load_path`:
the sidecar gated (`classify`: the same hash and dump, `Release::admits`), the image adopted
into the slot as `Store::load` builds one (no parked lists, no lazy frees, the allocator state
re-derived from the blocks).  A file of another layout is refused, where the interpreter
converts it — except one written before the null-first key order (`placement::KEY_ORDER`), which a
whole load converts: the gate admits a layout that differs only by `,order=1` and
`loft_rekey_walk` files every `sorted`, `ordered` and `index` whose key moved again in this
build's order (a paged load still refuses it, as the interpreter's does); a store holding references has them re-pointed at its slot from the slot its
sidecar recorded (`rebase_stored_refs`), read off the image's bytes before it is adopted, so a
refusal leaves the store as it was; `store_load_untrusted` validates the block structure first
(`Store::validate_structure`); compaction on load is not done (it declines for an image
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
interpreter does.  An `http(s)://` image is read the same way, a page per `Range` GET
(`HttpRangeProvider`'s port: sized by a `bytes=0-0` probe, its signature read before anything
else, a host that ignores `Range` read through to the page) and gated by its fetched sidecar;
the Rust provider's concurrent batches are not ported, so each page is one round trip.
`tests/c_emit.rs` crosses every
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
records, the checked cast and the staged form, on the interpreter and as C.  A keyed
collection is read as `record_new` / `record_finish` read it, each element filed under its key
(a displaced one released through `loft_types`); a member of a linked group files every element
into every member, and its views take no absent value; a `vector<S?>` element reads a present
object as the `Some` variant's payload (`walk_parsed_into`'s @PLN25 E2 arm).

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
(`loft_gen_*`), released by `OpFreeRef` like a store, and held anywhere a value is
(`(G-Hold)`): a step on a handle in a field, an element, a tuple member or a keyed record reads
the handle's type off the place.  A frame holds the store of every heap argument for its life
(`@FR-G-Arg-Rebind`, `Stores::hold_for_frame`): a free of a held store orphans it, the last
release frees it, and a re-mint or an exchange of one takes a fresh store, so a caller's whole
rebind after the call is not seen.  A tuple holding texts is yielded member by member, the
consumer's texts its own copies.

**Closures.**  A closure record's release (`OpDropFnRef`) runs the record type's drop cascade,
which runs the drop hooks of what it captured; the unit brings the cascade of every lambda it
reaches, and `loft_drop_closure` picks it by the value's definition.

**`par`.**  A `for x in v par(r = f(x), n)` loop's queue call runs the worker over the rows one
after another — each row's result is the one a thread would compute — into a buffer on a stack
the loop reads by row and pops (`loft_par_*`).  A row goes to the worker by its stored bits
(1, 4 or 8 bytes by the element's width) or as a reference, as `worker_row_arg` hands it; a
vector result gets a fresh buffer per row, a record result mints its own; a tuple row is read
member by member at its `__tuple<…>` record's offsets, a function row is its definition
number.  `par_fold` is `run_parallel_fold`'s arithmetic: the rows in `min(threads, rows)`
slices as `parallel_workers` splits them, each folded from `init`, the partials folded from
`init` in slice order.  On a POSIX or Windows host (not `-DLOFT_STRICT`, `-DLOFT_NO_POSIX` or
WASI) a site whose worker answers a scalar, a text, nothing, or a struct or struct-enum record
runs on threads (`loft_par_run`, plan 203 C4): each worker from a copy of the caller's store
table, a record row copied after the join into the caller's result store with the claims it
owns (`loft_par_adopt`), as native's queue copies it.  `LOFT_TRACE_PAR_WORKERS` reports the
workers each loop ran.

**Linked groups.**  A record type several collections of one struct share (`@FR-Col-Group`)
enters every member (`loft_group_finish`); an element write through the vector member hands
the record to the others again (`OpLinkRecord`), a whole vector is indexed into each view
(`OpIndexGroup`, `settle_group`), and a record a repeated key displaces leaves every keyed and
tree member, released when no vector member holds it (`settle_displaced`).  A copy of a record
holding a group copies the vector member and indexes the views over the copy
(`relink_group_views`).

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

**Lazy stores.**  `store_bind_lazy` binds a collection to a `.store` image (`LAZY_STORES.md`):
a keyed lookup that misses on a bound collection fetches that one key through the paged
loader the collection's kind takes — a hash's and a trie's key, a spatial's point as the
degenerate box — and looks again, so the collection stays the only authority on what is
resident.  The binding pins the source (length, modification time, inode) and a fetch after
it moved refuses; the fault channel (`store_lazy_error`, `_faults`, `_clear`, `_fail`) keeps
every failure's count and the first reason.  A kind an image cannot page is refused at the
bind.  A `sqlite:` source is driven as core drives it: the binding carries its collection type's
plan, derived when the unit is written by the same `sql_query::derive_select` the interpreter
calls (the equality query, the filter's prefix, the range query, each column's field), and the
runtime opens SQLite read-only through the system's library, checks the schema once per binding
(columns, affinity, an index serving the lookup), and files each row with the collection's own
insert; `store_lazy_query` and `store_lazy_range` materialise every row not yet resident.  A
program's own driver (`fn lazy_fetch…`) is called on a miss through an adapter per element type,
a fault inside it contained, as on the interpreter; a driver scheme with no driver answers
unreachable.  The miss hook is written only into a unit that binds.

**URL loads.**  `store_load_url` and `store_load_url_trusted` fetch the sidecar and the image
(`file://` from the filesystem, `http://` and `https://` as one HTTP/1.1 GET), gate the layout
as `store_load` does, pin the bytes by SHA-256 for the first, and adopt them validated.

**HTTPS.**  Every fetch of the runtime — the URL loads, the paged loaders' range reads and
`http_fetch` — runs through one HTTP/1.1 client over one client connection (`loft_https.h`: a
socket, or TLS over it; redirects followed, connecting and every read bounded by 30 seconds).  The TLS client is
`src/c/tls/loft_tls.c` over BearSSL's engine, vendored unmodified under `src/c/tls/bearssl/` from
the upstream the crypto library vendors its primitives from.  It is not part of the unit:
`src/c/tls.rs` builds it once per compiler into an archive under `~/.loft/c-tls/` and a unit that
fetches over a URL or pages an image names it on a `link:` line (`LOFT_HTTPS`).  It keeps `src/tls.rs`'s promise —
the machine's trust (`SSL_CERT_FILE` / `SSL_CERT_DIR`, else the system bundle), every chain
validated, the name always checked (a DNS name by BearSSL, an IP address against the
certificate's iPAddress names by `loft_tls.c`) — with two differences: TLS 1.2 only (BearSSL has
no 1.3, so a 1.3-only server refuses the handshake), and no bundled roots (a machine without a
trust store refuses every server, naming where it looked).  `tests/c_https.rs` is the guard.

**`#c` bindings.**  A function bound to a C symbol (`#c "abs" "int(int)"`) is a wrapper that
declares the callee with the author's own signature text — so it agrees with a system header
that declares it too — passes each scalar cast to its declared type, each text as a
NUL-terminated copy and each vector as its element pointer with a count where the signature has
one (`loft_cvec_ptr`, @C107; the C may write through it), and reads the result as `c_call`
does: an integer widened by its signedness, a `char *` copied to its NUL (NULL the null text,
invalid UTF-8 replaced), a handle as the pointer's value.  A package's **`[c] shim`** is its
own `cc` + `ar` archive (`c_shim::cc_archive`) the unit names in a `/* link: */` line, its
sources in `/* shim-source: */` lines for a wasm build; a declared `[c] libs` entry is linked,
the file the package ships by name with a run path to its directory, a system library by its
soname; an `[c] optional-libs` entry is never linked but opened when its package's first symbol
is called (`loft_c_lazy_sym`).  A program written out with `--out` (@PLN195) links none of the
libraries its packages ship: the unit lists each one's package, version and file
(`loft_cboundlibs`), and at start the runtime opens them through `native_bind`'s search order —
`LOFT_LIB_PATH`, the executable's directory, `LOFT_HOME/.loft/lib/<package>-<version>/`, the
system root — or exits 1 with its message, naming the package, the version and every directory
searched (`loft_c_bind_libraries`, POSIX hosts).  A shim stays inside a `--static` program (the
`--ship` default), linked whole; a `--dynamic` one opens the shim's own library beside it.  A
call into an opened library finds its symbol once (`loft_c_bound_sym`).  This is how a library
ships its native part without a Rust crate: ANSI C beside its loft, one source for all three
backends.  A `#native` function — a Rust cdylib's — is NOT reached: the C answers every call
with the interpreter's unloaded-library panic, so a library a C program needs is one whose
native part is a `[c] shim`, a prebuilt C library or a component.  A C build therefore never
builds a Rust cdylib (`native_build::c_target`, set for `--emit-c` and an explicit
`--native-c`; guard `a_c_build_builds_no_rust_native_crate`).  A default run that the C
backend refuses falls back to the interpreter, which loads them, so `LOFT_NATIVE_BACKEND=c`
alone does not set it.  A prebuilt library is asked for a native through its
`loft_register_v1` table first, which may name another symbol (`loft_save_png => n_save_png`),
then by the native's own name (`loft_ffi_lookup`).

**The libraries inside loft's binary.**  A `#native` of an `in_binary` library (`[native]
in_binary = true`: its natives live in loft's own binary, `native::FUNCTIONS`) is a call into
the C runtime when the runtime carries it (`in_binary_native` in `mod.rs`), and refuses the unit
by name when it does not — never the "not loaded" stub, which the interpreter cannot give for a
native of its own binary.  `loft_process.h` carries `lib/process` (`process_run`: one `poll`
drains every pipe, a child leads its own process group, the timeout stops the tree) and
`lib/git` (`git_query`).  The web, server and ssh libraries' natives are not the runtime's:
they are the `net` component (`loft_web.h`, `loft_server.h`, `loft_ssh.h` and the HTTP client
compiled into `libloft_net` by `src/c/component.rs`), which a C unit calls through the loft-ffi
ABI as it calls any prebuilt library (`tests/web_natives.rs`, `tests/server_natives.rs`,
`tests/ssh_natives.rs` the guards; § The SSH client).  `loft_engine.h` carries `lib/engine_host`'s kernel: the listener and
the connector over WebSockets, the UDP state-sync channel, the events, ticks and `post`, the
`D!:` debug channel answering as a build with no live tier, and `http_fetch` on a worker thread
over the runtime's HTTP client (above).  Not carried, so refused by name: the
build swap and the background rebuild (`swap_world`, `swap_start`, `rebuild_*`).  The live tier
— a flip, a live edit, a breakpoint's eval — runs the interpreter inside the program, which a C
build has not got: a C program's swap step serves and `live_flip` answers false, and the edit
loop runs interpreted.  All of it on every host with sockets (`loft_sock.h`: POSIX and
Windows); a Windows listener does not share its port, as `platform::bind_tcp_handover` binds
there.  The kernel closes its sockets at exit, as the Rust kernel's drop does: Windows resets a
socket a process leaves open, discarding what the peer had not read
(`tests/c_emit.rs` `an_engine_host_seat_talks_to_its_listener_on_the_c_and_the_interpreter`).

**`deliver`.**  `deliver(tag, value)` prints the loopback host's line: the value's bytes as
`read_via_descriptor` reads them through its layout, as hex, or the refusal of the first
store-internal kind it reaches.  `expose` and `release` lock and unlock the value's store; a
walk over a hash in a locked store keeps its snapshot in a store of its own
(`build_rec_scratch`).

### The SSH client

`src/c/tls/loft_ssh.c`, built into the TLS archive from the vendored BearSSL, is the ssh
library's client; `src/c/loft_ssh.h` puts it behind the library's eight natives in the `net`
component.  It speaks the slice of SSH-2 an interactive shell needs: curve25519-sha256 with
strict kex, and rekeys the server starts; host keys ssh-ed25519 (an Ed25519 of its own, which
BearSSL lacks), ecdsa-sha2-nistp256 and rsa-sha2-512/256; aes256/128-gcm@openssh.com, else
AES-CTR with HMAC-SHA2-256/512 (encrypt-then-MAC first); password login; one session channel with
a pty, a shell, window-change and flow control both ways.  Every blocking step has a 30 s
deadline and `ssh_recv` never waits.  The host key's signature over the exchange is always
verified; which key the server holds is not checked against a store — a first-use record is the
hardening step left.  `LOFT_SSH_TRACE=1` prints why a connect, a login or a shell failed.

Two instruments, for two questions:

- **The interactions** — `tests/ssh_natives.rs` runs `tests/fixtures/ssh_natives/interact.loft`
  on the interpreter, the Rust writer and the C against `test_sshd.c`, a small server built on
  the client's own packet layer: a refused password, the pty size and a resize, the 256 byte
  values, 3 000 000 bytes past the client's window, 200 000 bytes through an 8 KiB window, the
  exit, and every call on a closed handle a no-op.  It agrees with the client by construction,
  so it says nothing about the protocol.  Where `cl` links units the server is not built.
- **The protocol** — OpenSSH.  `sshd -d -f <config>` runs non-root on a high port with its own
  host keys, one connection per run; pinning `HostKeyAlgorithms`, `Ciphers` and `MACs` in the
  config and answering a bad password covers every host key × cipher × MAC.  A full login needs
  an account with a password, which a Linux runner has: `gh workflow run linux-probe.yml -f
  run=…` that adds one (`useradd`, `chpasswd`, `PasswordAuthentication yes`), runs
  `make components`, and runs `tests/fixtures/ssh_natives/session.loft` with `SSH_HOST`,
  `SSH_PORT`, `SSH_USER`, `SSH_PW` on each backend: login, `stty size` before and after a
  resize, 3 MB out, 200 KB in through a raw-mode reader, and the exit.

## Not carried yet

The work that makes this backend the only compiled target — a driver, diagnostics parity, the
stdlib census, `par` with threads, libraries and hosts, the wasm targets, a perf lane, three
platforms — is [@PLN203](plans/203-c99-default-backend.md), with the measured position it
starts from.

No program of the corpus is refused.  What the C runtime does not have, it answers at run time
the way a build without it answers: HTTPS on a host without
sockets (wasm) or whose compiler could not build the TLS client; SQLite where the system has no
`libsqlite3`.  Refused at emit time: a link re-pointed to a narrow or a text store place,
and a par row of a tuple holding a reference — each by name.  Not
refused and not carried: a `#native` function (above).

### What switching to C loses (@PLN206 P0b)

The capabilities the interpreter's runtime has and the C runtime does not, each with what it
is for (**R**eliability, **E**fficiency, **O**bservability), what C already has in its
place, and the proposed verdict: **port** it to C, **covered** (the C equivalent named, and one
falsifying cell owed that shows it catches what the original catches), or **drop** (loft does
not need it, with the reason).  A verdict stays *proposed* until its check is done; the rustc
path is not removed while a **port** row is open.  The store rows are also numbered in
[STORE_MODEL.md](STORE_MODEL.md).

| Capability (interpreter switch) | For | C has | Proposed verdict | Check owed |
|---|---|---|---|---|
| use-after-free generations (`LOFT_UAF`, `LOFT_UAF_GEN`) | R | `LOFT_STRICT_STORES` (`-DLOFT_STRICT_STORES`, the driver's build under the variable): no freed slot made again, an access through a freed store named at `loft_store_of`, a store live at exit named, either one exit 1 — the interpreter's ground truth rather than the generation stamps (those ride its operand stack, which a C unit does not have) | **ported** | `tests/c_emit.rs` `a_strict_store_c_build_names_a_use_after_free_and_a_store_never_freed`; `scripts/c_sweep.py --strict-stores` over the corpus against the interpreter under the same switch |
| frame-slot verification (`LOFT_VERIFY_STACK`) | R | the C stack; the MemorySanitizer lane | covered | an uninitialised-slot cell MSan flags on C |
| keyed fast-path verifier (`LOFT_KEYED_VERIFY`) | R | `-DLOFT_KEYED_VERIFY` (the driver's build under the variable): each keyed fast path the C has — the index's exact descent, the typed hash lookups, the one-probe entry and insert — checked against its general form, a disagreement stops the run naming both answers; the interpreter-only fast paths (pre-resolved comparison, decorated sort, inline key-order walk) have no C twin to check | **ported** | `tests/c_emit.rs` `a_keyed_verify_c_build_names_a_fast_path_that_answers_wrong` (corrupted descent and typed lookup) and the hash differential built with it (corrupted insert bucket and duplicate); `scripts/c_sweep.py --keyed-verify` |
| key-list verifier (`LOFT_KEY_LIST_VERIFY`) | R | the emitter writes each type's keys as a constant table (`static const loft_key loft_keys<N>[]`) | **drop** (decided): the cached list it re-derives (`@FR-R-KeyList`) does not exist in a C unit | — |
| prefill elision verifier (`LOFT_PREFILL_VERIFY`) | R | `LOFT_POISON_CLAIM` | covered | the field_append cells under `LOFT_POISON_CLAIM` (they pass on C) |
| the copy check (`LOFT_COPY_CHECK`) | R | `-DLOFT_COPY_CHECK` (the driver's build under the variable): after every deep copy — a bind's, `OpCopyRecord`'s, `OpReplaceKeyed`'s — each nested collection's length compared with its source's, a difference named by its field path, warn and continue | **ported** | `tests/c_emit.rs` `a_copy_check_c_build_names_a_copy_that_lost_elements`; `scripts/c_sweep.py --copy-check` |
| the live rebuild's world check (`LOFT_CHECK_WORLD`) | R | — a step of the `loft` driver, not of the runtime: it refuses a build the running world cannot be converted into before any code is generated, for every backend | **covered** (by the driver) — it exists for live rebuild, whose C form is the live-reload row's | — |
| zero-on-claim switch (`LOFT_ZERO_CLAIM`) | R | always zeroes today (D17) | covered once D17 lands: `LOFT_POISON_CLAIM` is the falsifier | D17's sweep |
| schema-id strictness (`LOFT_STRICT_SCHEMA_IDS`) | R | the type table is a static array the emitter writes (`static const loft_tdesc loft_types[…]`) | **drop** (decided): the start-up replay it guards does not exist in a C unit | — |
| the store-memory ceiling (`LOFT_STORE_LIMIT`, every test run) | R | `LOFT_MEMORY_LIMIT` | **ported** (D19c): the runner hands its test ceiling to every compiled child, both writers | `tests/store_memory_limit.rs` `the_default_ceiling_reaches_a_compiled_test` |
| store watch, lock backtrace (`LOFT_WATCH_STORE`, `LOFT_LOCK_BT`) | O | — | **port** when a C defect needs it; drop otherwise | — |
| store log, tags, leak sites, allocation report (`LOFT_STORE_LOG`, `LOFT_STORE_TAG`, `LOFT_LEAK_SITES`, `LOFT_ALLOC_REPORT`) | O | `LOFT_ALLOC_TRACE`, `LOFT_NATIVE_LEAK_CHECK` | **port** `LOFT_LEAK_SITES` (a leak's site is what fixing it needs); the rest drop | a leak cell names its site on C |
| allocator policy switches (`LOFT_NO_WILDERNESS`, `NO_PARK`, `NO_LAZY_FREE`, `NO_TOMBSTONE`, `NO_SLOT_REUSE`, `NO_CARVE_IN_PLACE`, `NO_FREE_FOOTER`, …) | E | each policy always on, or absent (D19b, D25) | **port** the bisect switches for the policies C applies; a policy C lacks is a D row | the performance agent's P1 rows |
| the live debugger (`loft debug`, `--rpc`) | O | — | **port** — needs a design (frames, values and stepping in a C unit) | — |
| live reload / live dispatch | E | — | open — owner decision | — |
| the profilers (`make profile`, `LOFT_NATIVE_CHECKPOINTS`) | E | `c_profile` | covered for time; open for heap by line | `make profile-corpus` on C |
| wasm and browser targets (`--native-wasm`, `--html`, built through rustc) | — | C to wasm through clang (@PLN203) | covered by @PLN203 | @PLN203's browser lane |
| the ship and lean tiers (`--ship`, `--lean`) | E | the `cship` lane | covered | the perf portal's `cship` column |
| native rewrites the C writer does not apply yet | E | the C writer's own `R-` rules | open — `make rewrite-census` on both writers names the gap | the census, per rule |
| mmap-backed stores (`@FR-S-Mapped`) | E | `loft_mapped.h`: `loft_st_open_mapped`, growth and the cut with the file, unmapped on release (POSIX, Windows) | **ported** (D23) | `tests/c_emit.rs` `a_store_bound_to_a_file_holds_the_same_bytes_on_both_runtimes` |
