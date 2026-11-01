<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# 91 — history

What [README.md](README.md) said before the 2026-11-01 rewrite around the objective (an
interpreter in loft with no Rust code left), kept as it stood.

## The status paragraph, 2026-10-24 to 2026-11-01

**Open — future epic; strand 5 under way, strand 1 split and re-gated (owner,
2026-10-24).** Multi-phase, design-before-build, gated on cheap de-risking probes before
any large work. **The owner's near-term aim is the COMPILER in loft, for its compile
time:** a `--native` program pays rustc about 16 s on a development box where the same
loop through C compiles in well under a second, and the gate, the test runner and every
library cdylib pay that once per program. So strand 1 is split below: **1a**, the front
end in loft, judged by an IR diff and needing no probe; **1b**, the runtime in loft, which
P-perf still gates. **Re-ordered 2026-10-27 (owner): 1b first** (re-ordered again on
2026-11-01: 1a first, above), as the ultimate dogfood project, reading the same IR in the same types the Rust front end writes; ownership is its
own early track because it is loft's biggest hurdle and its answer to rustc's borrow
checker.  **A reimplementation, not a new language (owner, 2026-10-30):** the same IR, the
same bytecode, the same memory structures, the same operator semantics — one rule, no new
mechanics.  The implementation's intricacy (the scope pass,
the ownership oracle, the bind decisions) is the price of combining ease of use with optimal
code, so it is ported faithfully, not redesigned. Strand 1b: [1b-interpreter.md](1b-interpreter.md); ownership:
[ownership.md](ownership.md). **Not
unrealistic:** the foundations already exist — see [Why this is reachable
today](README.md#why-this-is-reachable-today). The small-board reach (strands 6–7) turns on
reshaping the **store *profile*, not the language** — which reframes the standing
**C54.F** decision ("store model is the wrong shape for bare-metal") as *model vs
profile*: the word-addressed arena + no-GC scope-freeing is close to the canonical
embedded shape; only the *hosted, growable* profile is unfit. This README is the
single source of truth for per-strand status; `@PLN91` carries the summary + labels.

## Phase ordering, until 2026-11-01

1. **De-risking probes first (cheapest falsification before commitment):**
   - **P-front (the gate for 1a, run first):** a lexer and parser in loft that emit the
     codec IR for the programs they accept, diffed against the Rust parser over the
     corpus. Viable when the agreeing set grows with each construct added and no program
     needs a loft feature the language lacks; what it lacks is filed on @PLN179's register
     and fixed in the language (the dogfood rule), never worked around in the compiler.
   - **P-perf (the gate for 1b only):** opcode interpreter inner loop in loft, compiled
     native, vs hand-Rust. Viable if within ~1.5×; ~3× kills 1b, not 1a.
   - **P-cemit — DONE** as @PLN200 B5: the C backend, `scripts/c_sweep.py` the diff
     (@PLN89's role).
   - **P-footprint (the SBC gate / C54.F re-open evidence):** compile the actual `Store`
     `no_std` with a static backing buffer + no-grow, run a trivial compiled program,
     prove **zero `malloc`/grow** and measure flash/RAM on a 32-bit target. The number
     is what re-opens C54.F on the *model-vs-profile* axis.
   - **P-dbref-perf (mainline-native gate for strand 7):** on the *current* native
     backend, prototype static store resolution for one hot deref-heavy type; measure
     runtime + binary size + compile time vs the 12-byte baseline. Confirms the
     common-case speedup independent of the SBC footprint number — and justifies strand 7
     on its own.
2. **Strand 1a (the compiler in loft)** is the trunk and goes FIRST since 2026-11-01
   (owner), in the order each step can be diffed: lexer and parser → scopes and types →
   the IR passes of `opt` → a writer.  Each stage has a byte-identical gate before it has a
   behavioural one: the lexer's token stream against `src/lexer.rs`'s, then the parser's
   image against the Rust front end's `--emit-image`.  **Once the interpreter is complete,
   the loft-written compiler generates C, byte-identical to the C the Rust generator
   (`src/c/`) writes for the same program** (owner, 2026-11-01) — strand 12, its own set of
   steps with every optimisation, the gate before the Rust generator is retired.  Stage 2 of the bootstrap
   (below) is reached when the loft-written compiler emits C for itself.
3. **Strand 1b (the runtime in loft)** went first from 2026-10-27 to 2026-11-01: S0 opened
   the bundle, S1 is P-perf, and ownership runs beside it as track O
   ([1b-interpreter.md](1b-interpreter.md), [ownership.md](ownership.md)).  Its open items
   wait behind 1a's lexer and parser; 1a takes over the O checker as its scope pass.
   **2/3/4** ride 1b (4 falls out for free).
4. **Strand 7 (store profile)** is the SBC lead; **5 (ANSI-C)** and **6 (widths)** enable it.

## Bootstrap staging, until 2026-11-01

`stage 0` = current Rust loft → `stage 1` = loft-in-loft compiled by stage-0's native
backend → `stage 2` = stage-1 emits C/native for itself → **C-bootstrappable on any
host with a C compiler.** The Rust implementation is then retired, not kept as a second
implementation: the end state has no Rust code and no rustc in the build (owner,
2026-11-01).
