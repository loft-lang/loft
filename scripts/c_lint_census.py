#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The C compiler's warnings on the C the backend writes, over the corpus.

    scripts/c_lint_census.py               # every program scripts/corpus_files.sh lists
    scripts/c_lint_census.py FILE...       # these programs
    scripts/c_lint_census.py --sample 200  # a seeded sample of the corpus (--seed N)
    scripts/c_lint_census.py --unused      # also count the `unused` family, apart
    LOFT_BIN=<loft> LOFT_CC=clang …        # another build, another compiler

Each program is built as the driver builds it with `LOFT_C_LINT=report` (`--check --native-c`:
`-Wall -Wextra` minus the `unused` family, `src/c_driver.rs` `LINT_WARNINGS`), so the count is
the compiler's own over exactly the code a run compiles.  A warning can mean a wrong program —
an uninitialised read, a value clobbered by `longjmp`, a truncating conversion — so the count
is held at zero: any warning prints with its kind and the source line it names, and the census
exits 1.  The `unused` family is generated code's by design (a parameter a body ignores, a
helper a unit does not call); `--unused` reports it beside the count, never in it.

Each compiler warns its own set: a count is about the compiler that produced it.
"""
import argparse
import collections
import concurrent.futures as cf
import os
import pathlib
import random
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
LOFT = os.environ.get("LOFT_BIN", str(ROOT / "target" / "release" / "loft"))
UNUSED = ["-Wunused-parameter", "-Wunused-function", "-Wunused-variable", "-Wunused-but-set-variable",
          "-Wunused-but-set-parameter", "-Wunused-label", "-Wunused-value"]
LINE = re.compile(r"^[^\n:]+:\d+:\d+: (?:warning|error): (.*?) \[(-W[^\]]+)\]$")


def corpus() -> list[str]:
    sys.path.insert(0, str(ROOT / "scripts"))
    from corpus import corpus_files
    return sorted(str(p) for p in corpus_files())


def lint(path: str, unused: bool) -> tuple[str, list[tuple[str, str, str]], str]:
    """The warnings building `path` reports: (kind, message, source line); and why the build did
    not run (an emit refusal is no warning — the corpus holds expected-error programs)."""
    env = dict(os.environ, LOFT_C_LINT="all" if unused else "report", LOFT_TIMEOUT="600")
    p = pathlib.Path(path).resolve()
    try:
        r = subprocess.run([LOFT, "--no-warnings", "--check", "--native-c", "--lib", str(ROOT / "lib"), str(p)],
                           capture_output=True, text=True, errors="replace", cwd=p.parent, env=env, timeout=900)
    except subprocess.TimeoutExpired:
        return path, [], "timeout"
    lines = r.stderr.split("\n")
    out = []
    for i, line in enumerate(lines):
        m = LINE.match(line)
        if m:
            src = lines[i + 1].split("|", 1)[-1].strip() if i + 1 < len(lines) else ""
            out.append((m.group(2), re.sub(r"‘[^’]*’|'[^']*'", "‘…’", m.group(1)), src))
    return path, out, ""


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("files", nargs="*")
    ap.add_argument("--sample", type=int, default=0)
    ap.add_argument("--seed", type=int, default=7)
    ap.add_argument("--jobs", type=int, default=max(1, (os.cpu_count() or 2) // 2))
    ap.add_argument("--unused", action="store_true", help="also count the unused family, apart")
    a = ap.parse_args()
    if not pathlib.Path(LOFT).exists():
        print(f"c_lint_census: no loft binary at {LOFT} (build it, or set LOFT_BIN)", file=sys.stderr)
        return 2
    files = a.files or corpus()
    if a.sample:
        random.seed(a.seed)
        files = random.sample(files, min(a.sample, len(files)))
    kinds, lines_, unused_kinds = collections.Counter(), collections.Counter(), collections.Counter()
    where = collections.defaultdict(list)
    timeouts = []
    with cf.ThreadPoolExecutor(a.jobs) as ex:
        for path, warns, why in ex.map(lambda f: lint(f, a.unused), files):
            if why:
                timeouts.append(path)
            for kind, msg, src in warns:
                if kind in UNUSED:
                    unused_kinds[kind] += 1
                    continue
                kinds[kind] += 1
                lines_[(kind, msg, src)] += 1
                if len(where[(kind, msg, src)]) < 3:
                    where[(kind, msg, src)].append(path)
    total = sum(kinds.values())
    print(f"C lint over {len(files)} programs ({os.environ.get('LOFT_CC') or os.environ.get('CC') or 'cc'}): "
          f"{total} warnings")
    for kind, n in kinds.most_common():
        print(f"  {n:6} {kind}")
    for (kind, msg, src), n in lines_.most_common():
        print(f"  {n:6} {kind} {msg}\n         {src[:160]}\n         e.g. {', '.join(where[(kind, msg, src)])}")
    if a.unused:
        print(f"the unused family, apart: {sum(unused_kinds.values())}")
        for kind, n in unused_kinds.most_common():
            print(f"  {n:6} {kind}")
    if timeouts:
        print(f"timed out ({len(timeouts)}): {', '.join(timeouts[:5])}")
    return 1 if total or timeouts else 0


if __name__ == "__main__":
    sys.exit(main())
