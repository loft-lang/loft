<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 207 — A cache that behaves as if it is not there

## Status

Open — design written, nothing built.  Issue: [@PLN207](https://github.com/loft-lang/plans/issues/207).

## Goal

A `#memo` function's result is cached — in the process, on the machine, on a deployment's cache
server — and no program can tell a hit from a computation, except that it is faster.

## The rule

**The cache is implemented as if it is not there: the only observable effect is speed** (owner).
loft has no hidden state — a library's state lives in short-lived objects its caller holds
([API_SURFACE.md § S6](../../API_SURFACE.md)), and a file's top level declares functions, types and
constants, never variables ([LOFT_DECLARATIONS.md](../../LOFT_DECLARATIONS.md)).  The cache is the
one exception, and only because nothing can see it.  Every phase below is judged against this
sentence, and its test is the off-switch: **`LOFT_NO_MEMO=1` must leave every program's stdout,
stderr and exit status unchanged.**

What "not there" demands, each a property a phase must prove:

| Property | What breaks it | Where it is kept |
|---|---|---|
| Same value | A key that misses an input; an impure function admitted | The key is derived from every argument; only proven-pure functions (M0) |
| Same failures | A cached call that should have printed or panicked | A function that prints, panics or reads the clock is refused (M0) |
| No shared identity | Two callers aliasing one cached record | A hit answers a copy (M1); a read-only view only where `@FR-R-Const`'s analysis proves no write |
| No interface | `cache_get`, a hit counter, an invalidate call | None exists; the switch is a tool, not an API |
| No cost to the program | The cache filling memory a program needs; a slow remote | Entries are dropped first under `LOFT_MEMORY_LIMIT` (M3); a tier is consulted only when it has proven cheaper than computing (M5) |
| Same answer everywhere | Another build's entry; a torn file; a lying server; floats that differ by host | The code-identity key (M4); entries checked when read; signed remote entries (M5); M6 |

## Shape

```loft
#memo
fn font_tables(path: text, stamp: integer) -> FontTables {
  parse_ttf(read_bytes(path) ?? [])     // admitted: the file read is covered by (path, stamp)
}

pub fn load_font(path: text) -> Font? {
  t = font_tables(path, mtime(path));   // a hit skips the parse, a miss parses
  if t.units_per_em == 0 { return null; }
  Font { tables: t }                    // the object is the caller's; the cache holds no Font
}
```

The key is `hash(code identity, type layouts, argument bytes)`.  The tiers are tried in order:

| Tier | Holds | Shared by |
|---|---|---|
| process | memory, copies out | the running program |
| machine | a directory under `LOFT_HOME`, one file per key, written by rename | every process on the host: a game's servers, its temporary workers, the next run |
| remote | a cache service, read by key over the [REMOTE_STORES.md](../../REMOTE_STORES.md) transport | every machine of a deployment |

## Composition matrix

The axes the feature touches, each to be crossed by suite cells before its phase closes:

- **Backend:** interpreter · `--native` (Rust) · `--native-c`.
- **Argument kind:** integer · float (NaN, -0.0) · text · vector · struct · nested struct · enum
  with payload · a const view.
- **Result kind:** the same set; plus the refusals — a type holding an `OpDrop` member (a copy would
  duplicate a host resource) and a `#c` handle.
- **Cache state:** off · cold · warm · evicted mid-run · corrupt entry · tier unreachable.
- **Callers:** one · two callers of one key in one frame · inside a `par` body · recursion through
  the memo function.
- **Placement:** in-process · `process` · `remote` ([PLACEMENT.md](../../PLACEMENT.md)).

## Phases

Each phase is as small as possible while still able to go red for a real reason.

**M0 — the purity checker, as a report (S).**  Infer, for every function, whether it is pure:
no write outside its own locals and its result, no print, no panic, no clock, no PRNG, no I/O,
calls only pure functions.  Today purity is declared, never checked: `Purity` defaults to
`Unknown` and is set only by `#pure` / `#impure(…)`.
- Pointers: `src/data.rs:4296` (`enum Purity`), `src/parser/definitions.rs:4021` (the `#pure` and
  `#impure` annotations), `src/scopes/par_writes.rs` (the par analyser that already reads purity),
  `src/shared_call.rs:56`, `formal/rewrites.md` `(R-PureReuse)`.
- Goes red when: a hand-classified cell set (pure, prints, panics, writes a parameter, reads the
  clock, calls an impure function, recursion) disagrees with the report.  Nothing is cached yet.

**M1 — `#memo` in one process, interpreter (M).**  Parse `#memo`; refuse it on a function M0 does
not prove pure, on a result type holding an `OpDrop` member, and on a `#c` call.  Key = argument
bytes canonicalised (NaN to one pattern); a hit copies the cached record into the caller's store.
- Pointers: the copy is `OpCopyRecord`'s path ([DATABASE_STORES.md § Constant store](../../DATABASE_STORES.md));
  the cache's records live in a store of their own, write-locked to programs like `CONST_STORE`
  (`src/database/mod.rs:52`).
- Goes red when: any suite cell prints differently under `LOFT_NO_MEMO=1`; a refusal cell compiles.
- Records the rule as a design-decision entry (`./scripts/idx next-decision`), cited from the
  memo code site and the off-switch guard.

**M2 — the native and C backends (M).**  The same store and the same key on `--native` and
`--native-c`.
- Pointers: `src/generation/` (the Rust writer), `src/c/mod.rs` and the C runtime's store
  (`src/c/loft_store.h`).
- Goes red when: a cell answers differently on any backend, or under the switch on any backend.

**M3 — `par` and memory (S).**  One lock around the table (or one table per worker — measure
both); the cache gives up entries before `LOFT_MEMORY_LIMIT` refuses a program allocation.
- Goes red when: a `par` cell differs from its sequential run; a cell under a tiny cap differs
  from its uncapped run (eviction stress).

**M4 — code identity and the machine tier (M).**  The key gains the function's code identity —
a hash over the canonical IR of the function and everything it can call, plus the loft version
and the type layouts — so another build never reads this build's entries.  Entries are written
to `LOFT_HOME/.loft/memo/<key>` by rename and carry their own hash.
- Pointers: `src/ir_schema.rs:972` (`definition_to_json`, the IR codec that is already canonical
  over the stdlib and the suite: `cargo test --release --test ir_schema_roundtrip`).
- Goes red when: two processes (cold then warm), an edited function between runs, or a truncated
  entry file change any output.

**M5 — the remote tier (M).**  Read by key over the remote-store transport; a deployment's
producers sign entries with its key and a reader accepts only signed ones; a verify mode
recomputes and compares (always in CI).  An unreachable or slow server is a miss.
- Pointers: [REMOTE_STORES.md](../../REMOTE_STORES.md), `src/registry_signing.rs:53`
  (`verify_index`, the Ed25519 check the registry already uses).
- Goes red when: the [PLACEMENT.md](../../PLACEMENT.md)-style parity run — cache off, process,
  machine, remote; cold and warm; corrupt entry; server down; a forged entry — prints anything
  different (`tests/placement_parity.rs` is the gate's shape).

**M6 — floating point across hosts (S, a measurement first).**  `+ - * /` are IEEE everywhere;
`sin`, `exp`, `pow` come from each platform's libm and may differ in the last bit, which a remote
hit would make observable.  Measure them over the CI hosts and backends; then either fold a
numerics id into the key of a function that reaches them, or ship one libm.
- Goes red when: the measurement finds a difference and the key does not carry it.

## Open design questions

- **Keys over large values.**  A glyph lookup keyed by a whole `Font` re-hashes the font's tables
  each call.  An immutable value could carry its content hash, computed once; until then such a
  cache is the object's own state.
- **A value's lifetime in a view.**  M1 copies out.  A view saves the copy but pins the entry; a
  pin that outlives an eviction is the one way a view could be observed.
- **A sandboxed caller.**  Timing is the cache's one channel, so a mod could learn that an input
  was computed before.  Each trust domain gets its own key space.
- **@PLN69** (a caller-held `Cache<T>` for files) stays useful for data that is not a function of
  its inputs; a `#memo` with `mtime` covers the rest.

## Cross-arc dependencies

M0 before everything.  M4 before M5.  M6 before M5 serves a function that reaches libm.

## Documentation

- `LOFT_DECLARATIONS.md` — `#memo`, its admission rule and refusals.
- `DIAGNOSTICS.md` — the refusal codes and `LOFT_NO_MEMO`; the pin in `tests/e1_code_set.rs`.
- The design-decision entry for the rule (M1), and `formal/rewrites.md` beside `(R-PureReuse)`.
- `PLACEMENT.md` — the memo tiers beside the placements; `REMOTE_STORES.md` — the cache read.
- `PERFORMANCE.md` — how to see a hit (a profile, never a program).
- `CHANGELOG.md`; the `loft-write` skill.

## See also

[API_SURFACE.md § S6](../../API_SURFACE.md) · [INTERFACES_DROP.md](../../INTERFACES_DROP.md) ·
[DATABASE_STORES.md § Constant store](../../DATABASE_STORES.md) · [PLACEMENT.md](../../PLACEMENT.md) ·
[REMOTE_STORES.md](../../REMOTE_STORES.md) · [@PLN208](../208-graphics-objects-c/README.md), the
first library to want it.
