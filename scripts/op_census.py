#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Which bytecode operators does anything still EMIT?  A REPORT, never a gate.

An operator is cheap to add and invisible to retire: a `fn Op…` in `default/*.loft`
becomes an entry in the generated `fill::OPERATORS` table, a `#rust` template the
interpreter runs, and a template the native generator rewrites.  Nothing ever asked
whether a program still reaches one.  This asks, over every `.loft` file in the tree.

Three populations, and the verdict is which of them an operator appears in:

  declared     `fn Op<Name>` in `default/*.loft` — the inventory, in `fill::OPERATORS`
               order (each declaration's position IS its `op_code`)
  stdlib       operators the stdlib's own compiled bodies emit (one carrier program)
  corpus       operators emitted by every function a `.loft` file compiles OUTSIDE the
               stdlib: its functions, and its lambdas, methods and drop cascades

  live         emitted somewhere — the count says how often
  unexercised  never emitted, but `src/` names it — an emitter with NO corpus coverage
               (a test gap), or only a reader left behind (trace it before believing
               either)
  orphan       never emitted and nothing in `src/` names it — the stale candidate

The three tiers matter because only the third is a deletion candidate, and the second
is the more actionable finding: live emitter, zero coverage.

WHY THIS CANNOT BE A GREP.  The parser names most operators by COMPUTING the name —
`format!("Op{}", rename(op))` in `src/parser/mod.rs`, where `rename` maps a token
(`+` → `Add`), and overload resolution then picks the type-suffixed definition from
the argument types.  So `OpEqText` is emitted by every `a == b` on text and the string
`"OpEqText"` appears NOWHERE in `src/`.  A grep-based audit of this tree reports 18
"unused" operators, of which `OpEqText`, `OpRemFloat` and `OpSLeftInt` are emitted by
an eight-line program.  The compiler has to answer, not the text.

HOW IT READS THE ANSWER.  `loft introspect --show-bytecode` runs the shipped
disassembler, so this scrapes no format of its own invention — but it does scrape.
The pattern below is derived from the ONE writer, `State::dump_code`
(`src/state/debug.rs`), which emits per instruction:

    write!(f, "{:4}", rel)                 # offset
    write!(f, "[{}]", stack)               # optional stack annotation
    write!(f, ": [{nr}] ") or ": "         # optional source line
    write!(f, "{}(", &def.name()[2..])     # the name, `Op` stripped

Keying on a narrower pattern silently drops instructions: requiring the `[stack]`
annotation hid nine operators (`Goto`, `Release`, `ParallelBegin`, `RemFloat`,
`NeSingle`, three `Cast*`, `DivSingleNullable`) and reported them as unused.  So the
run VERIFIES itself rather than trusting the pattern — see `_guard`.  An under-report
here is a proposal to delete a live operator, which is the one failure that must be
loud.

The same holds for WHICH FUNCTIONS are read.  A plain `introspect --show-bytecode` lists a
file's `n_` functions and leaves out its lambdas, methods and drop cascades, so an operator
emitted only inside a lambda (`OpFreeRefOrHandUp` is one) read as unexercised.  Every run
therefore asks for `--all-fns` and sorts the functions itself, by the file each
`byte-code for <file>:` header names; `_guard` refuses a run that read no lambda body.

    scripts/op_census.py                 # the report
    scripts/op_census.py --verbose       # every operator with its emission count
    scripts/op_census.py --json          # machine-readable
    scripts/op_census.py --jobs 8        # parallelism (default: CPU count)

Needs a built binary: `target/release/loft`, or `LOFT_BIN=<path>`.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# Derived from `State::dump_code`'s writer — see the module docstring.  Every optional
# field is optional HERE too, or the census under-reports in silence.
INSTR = re.compile(r"^\s*\d+(?:\[\d+\])?: (?:\[\d+\] )?([A-Za-z][A-Za-z0-9]*)\(")

# The header `State::dump_code` writes before each function's instructions, and the files
# whose functions are the stdlib population rather than a program's own.
HEADER = "byte-code for "
STDLIB_FILE = re.compile(r"[/\\]default[/\\][0-9]{2}_[a-z_]+\.loft:")

# `fn OpName(` at the start of a line in the default library.  The declaration ORDER is
# the opcode numbering (`src/create.rs::generate_code_into`), so this is the inventory
# and its length is the table size.
DECL = re.compile(r"^fn (Op[A-Za-z0-9_]+)", re.M)

# Bytecode encoding (`src/state/mod.rs::emit_op`): ops 0-254 are one byte; byte 255 is
# an escape prefix and the interpreter dispatches `OPERATORS[255 + ext]`.  `ext` is a
# u8, so the reachable range is 0..=510.
ONE_BYTE_SLOTS = 255
CEILING = 255 + 256  # 511 addressable opcodes

# Directories that are not this tree's corpus: build output, git internals, and the
# agent worktrees under .claude/ (full copies of the repo — counting them would mean
# measuring another checkout's corpus as if it were this one).
SKIP_DIRS = {"target", ".git", ".claude"}


def loft_binary() -> Path:
    """The binary to ask.  Explicit `LOFT_BIN` wins; otherwise the release build."""
    env = os.environ.get("LOFT_BIN")
    cand = Path(env) if env else ROOT / "target" / "release" / "loft"
    if not cand.is_file() or not os.access(cand, os.X_OK):
        sys.exit(
            f"op_census: no loft binary at {cand}\n"
            "  build one:  cargo build --release --bin loft\n"
            "  or point at one:  LOFT_BIN=<path> scripts/op_census.py"
        )
    return cand


def declared_operators() -> list[str]:
    """The inventory, in declaration (= opcode) order."""
    out: list[str] = []
    for f in sorted(p for p in (ROOT / "default").glob("*.loft") if p.is_file()):
        if not f.is_file():  # `default/.loft` is a DIRECTORY in this tree
            continue
        out += DECL.findall(f.read_text(encoding="utf-8", errors="replace"))
    return out


def corpus_files() -> list[Path]:
    """Every `.loft` file in the tree that is this project's own corpus."""
    files: list[Path] = []
    for path, dirs, names in os.walk(ROOT):
        dirs[:] = [d for d in dirs if d not in SKIP_DIRS]
        for n in names:
            if n.endswith(".loft"):
                p = Path(path) / n
                if p.is_file():
                    files.append(p)
    return sorted(files)


def emitted_in(binary: Path, target: Path | str, stdlib: bool) -> tuple[dict[str, int], int]:
    """Operators the compiled bytecode of `target` emits, with counts, for ONE population:
    the stdlib's own functions (`stdlib`), or every other function the program compiled.
    Also answers how many lambda bodies were read, for `_guard`."""
    cmd = [str(binary), "introspect", "--show-bytecode", "--all-fns", str(target)]
    try:
        run = subprocess.run(
            cmd, capture_output=True, text=True, errors="replace", timeout=120
        )
    except subprocess.TimeoutExpired:
        return {}, 0
    counts: dict[str, int] = {}
    lambdas = 0
    counting = False
    for line in run.stdout.splitlines():
        if line.startswith(HEADER):
            counting = bool(STDLIB_FILE.search(line)) == stdlib
            lambdas += counting and ":n___lambda_" in line
            continue
        if not counting:
            continue
        m = INSTR.match(line)
        if m:
            counts[m.group(1)] = counts.get(m.group(1), 0) + 1
    return counts, lambdas


def names_in_src(ops: list[str]) -> dict[str, list[str]]:
    """Which `src/*.rs` files name each operator, ignoring `//` comments.

    `src/fill.rs` is excluded: it is GENERATED from the declarations, so it names every
    operator by construction and would make every one look reachable.
    """
    found: dict[str, list[str]] = {o: [] for o in ops}
    wanted = set(ops)
    for rs in (ROOT / "src").rglob("*.rs"):
        if rs.name == "fill.rs":
            continue
        try:
            text = rs.read_text(encoding="utf-8", errors="replace")
        except OSError:
            continue
        for line in text.splitlines():
            code = line.split("//", 1)[0]
            if "Op" not in code:
                continue
            for name in set(re.findall(r"\bOp[A-Z][A-Za-z0-9_]*", code)) & wanted:
                rel = str(rs.relative_to(ROOT))
                if rel not in found[name]:
                    found[name].append(rel)
    return found


def _guard(
    declared: list[str], emitted: dict[str, int], contributed: int, total: int, lambdas: int
):
    """Prove the run could have failed.  Exits non-zero when the census is not trustworthy.

    A census that silently reads nothing, or reads a format it does not understand,
    proposes deleting live operators.  Both failures are made loud here:

      * an emitted name that is not a declared operator means the disassembly format or
        the inventory moved — the pattern above is then measuring something else;
      * no contributing file, or a tree where even `ConstInt` never appears, means the
        binary or the pattern is wrong rather than the operators being unused;
      * no lambda body read means the dump is leaving functions out again, and every
        operator emitted only there would be listed as unused.
    """
    bare = {d[2:] for d in declared}
    unknown = sorted(set(emitted) - bare)
    if unknown:
        sys.exit(
            "op_census: emitted names that are not declared operators: "
            f"{', '.join(unknown[:8])}\n"
            "  The disassembly format or the operator inventory has moved; the "
            "extraction pattern in this script (derived from State::dump_code) must be "
            "re-derived before its answer means anything."
        )
    if contributed == 0:
        sys.exit(f"op_census: none of the {total} corpus files produced bytecode")
    if lambdas == 0:
        sys.exit(
            "op_census: no lambda body was read across the whole corpus.\n"
            "  The tree has lambdas, so the dump is leaving functions out — an operator "
            "emitted only inside one would be reported as unused."
        )
    if "ConstInt" not in emitted:
        sys.exit(
            "op_census: `ConstInt` was never emitted across the whole corpus.\n"
            "  That cannot be true of this language — the run is reading nothing."
        )


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--verbose", "-v", action="store_true", help="every operator + count")
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    ap.add_argument("--jobs", "-j", type=int, default=os.cpu_count() or 4)
    args = ap.parse_args()

    binary = loft_binary()
    declared = declared_operators()
    if not declared:
        sys.exit("op_census: no operators declared in default/*.loft")

    files = corpus_files()
    # The stdlib's own bodies, once: every run dumps every definition, so counting them
    # per file would measure the stdlib 3000 times and swamp the per-program answer.
    # The carrier program is written here rather than picked from the corpus: any file
    # taken from the tree may be an error fixture that does not parse, and the stdlib
    # population would then be EMPTY without anything saying so.
    with tempfile.TemporaryDirectory() as tmp:
        carrier = Path(tmp) / "carrier.loft"
        carrier.write_text("fn main() {\n  print(\"x\");\n}\n", encoding="utf-8")
        stdlib, _ = emitted_in(binary, carrier, True)
    if not stdlib:
        sys.exit(
            "op_census: the stdlib probe emitted nothing — a one-line program did not "
            "compile.  The binary is broken or is not a loft binary; every number below "
            "would be an under-report."
        )

    corpus: dict[str, int] = {}
    contributed = 0
    lambdas = 0
    with ThreadPoolExecutor(max_workers=args.jobs) as pool:
        for counts, n in pool.map(lambda f: emitted_in(binary, f, False), files):
            lambdas += n
            if counts:
                contributed += 1
            for k, v in counts.items():
                corpus[k] = corpus.get(k, 0) + v

    emitted = dict(corpus)
    for k, v in stdlib.items():
        emitted[k] = emitted.get(k, 0) + v
    _guard(declared, emitted, contributed, len(files), lambdas)

    src_names = names_in_src([d for d in declared if d[2:] not in emitted])
    live, unexercised, orphan = [], [], []
    for d in declared:
        if d[2:] in emitted:
            live.append((d, emitted[d[2:]]))
        elif src_names.get(d):
            unexercised.append((d, src_names[d]))
        else:
            orphan.append(d)

    report = {
        "declared": len(declared),
        "one_byte_slots_used": min(len(declared), ONE_BYTE_SLOTS),
        "escape_range_used": max(0, len(declared) - ONE_BYTE_SLOTS),
        "ceiling": CEILING,
        "free": CEILING - len(declared),
        "files_scanned": len(files),
        "files_contributed": contributed,
        "live": len(live),
        "unexercised": {d: f for d, f in unexercised},
        "orphan": orphan,
    }
    if args.json:
        print(json.dumps(report, indent=2))
        return 0

    print(
        f"Operator census — {len(declared)} declared, {len(live)} emitted, "
        f"{len(unexercised)} unexercised, {len(orphan)} orphan"
    )
    print(
        f"  table: {report['one_byte_slots_used']} one-byte + "
        f"{report['escape_range_used']} escape-range of {CEILING} "
        f"({report['free']} free)"
    )
    print(f"  corpus: {contributed} of {len(files)} .loft files produced bytecode\n")

    if unexercised:
        print(f"UNEXERCISED — src/ names it, no program in the tree emits it ({len(unexercised)}):")
        for d, where in unexercised:
            print(f"  {d:26s} {', '.join(where[:3])}")
        print()
    if orphan:
        print(f"ORPHAN — never emitted, nothing in src/ names it ({len(orphan)}):")
        for d in orphan:
            print(f"  {d}")
        print()
    if not unexercised and not orphan:
        print("Every declared operator is emitted by something.\n")
    if args.verbose:
        print("LIVE (emission count over the corpus + stdlib):")
        for d, n in sorted(live, key=lambda x: -x[1]):
            print(f"  {n:9d}  {d}")
    else:
        print("`--verbose` lists every live operator with its emission count.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
