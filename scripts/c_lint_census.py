#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The C compiler's warnings on the C the backend writes, over the corpus.

    scripts/c_lint_census.py               # every program scripts/corpus_files.sh lists
    scripts/c_lint_census.py FILE...       # these programs
    scripts/c_lint_census.py --sample 200  # a seeded sample of the corpus (--seed N)
    scripts/c_lint_census.py --runtime     # the runtime's own audit: its helpers no unit calls, and
                                           # the ones nothing in the tree names at all (dead code)
    LOFT_BIN=<loft> LOFT_CC=clang …        # another build, another compiler

Each program is built as the driver builds it with `LOFT_C_LINT=report` (`--check --native-c`:
`-Wall -Wextra`, `src/c_driver.rs` `LINT_WARNINGS`), so the count is the compiler's own over
exactly the code a run compiles.  A warning can mean a wrong program — an uninitialised read, a
value clobbered by `longjmp`, a local whose value nothing reads — so the count is held at zero:
any warning prints with its kind and the source line it names, and the census exits 1.  The one
thing not counted is a runtime helper a unit does not call (a pragma around the runtime: every
unit carries the whole library); `--runtime` lifts it and lists, apart, the helpers no program
of the run calls and the ones no line of `src/c/` names but their own definition.

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
          "-Wunused-but-set-parameter", "-Wunused-label", "-Wunused-value", "-Wunused-const-variable="]
LINE = re.compile(r"^[^\n:]+:\d+:\d+: (?:warning|error): (.*?) \[(-W[^\]]+)\]$")


def corpus() -> list[str]:
    sys.path.insert(0, str(ROOT / "scripts"))
    from corpus import corpus_files
    return sorted(str(p) for p in corpus_files())


# Runtime functions nothing calls yet that stay, each with why.
KEEP = {
    "loft_st_open_mapped": "@PLN206 P4 — a store in the table bound to a file: the call arena a C worker maps",
}


def runtime_names() -> dict:
    """Per function a header under src/c/ defines, whether anything uses it: a mention in a
    header that is not its own definition or prototype (a call, a function pointer, an
    initializer), or the emitter writing it — by name, or by a name it builds (`loft_ser{tp}`
    for `loft_ser5`).  A name pasted by a macro (`LOFT_RXF(walk)`) is used when the macro's
    argument is."""
    # The `net` component's sources export what libraries call through the FFI ABI
    # (`src/c/component.rs`): nothing in a unit names them, and none of them is dead for it.
    exported = {"loft_web.h", "loft_server.h", "loft_ssh.h"}
    every = sorted((ROOT / "src" / "c").glob("*.h"))
    own = "\n".join(p.read_text(errors="replace") for p in every if p.name not in exported)
    text = "\n".join(p.read_text(errors="replace") for p in every)
    # The emitter, and the C that includes the runtime outside a unit (the self-host's shim).
    emit = "\n".join(p.read_text(errors="replace") for p in (ROOT / "src" / "c").rglob("*.rs"))
    emit += "\n".join(p.read_text(errors="replace") for p in (ROOT / "selfhost").rglob("*.c"))
    defs = set(re.findall(r"^(?:static|LOFT_\w+)[^;{=(]*?\b(loft_\w+)\s*\(", own, re.M))
    used = {}
    for f in defs:
        pat = re.compile(r"\b" + re.escape(f) + r"\b")
        decl = re.compile(r"^(?:static|LOFT_\w+)[^;{=(]*?\b" + re.escape(f) + r"\s*\(")
        uses = sum(len(pat.findall(l)) - (1 if decl.match(l) else 0) for l in text.split("\n"))
        # The emitter builds names: `loft_ser{tp}` for `loft_ser5`, `loft_vh_get{at}` for
        # `loft_vh_get_at`.  Any underscore prefix of three parts or more it writes counts.
        parts = f.split("_")
        built = any("_".join(parts[:k]) in emit for k in range(3, len(parts)))
        built = built or any(f'"{"_".join(parts[:k])}"' in emit for k in range(2, len(parts)))
        num = re.match(r"^(loft_[a-z_]*?)(\d+)$", f)
        built = built or (bool(num) and (num.group(1) + "{") in emit)
        used[f] = uses > 0 or bool(pat.search(emit)) or built
    return used


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
    ap.add_argument("--runtime", action="store_true", help="the runtime's unused helpers, apart")
    a = ap.parse_args()
    if not pathlib.Path(LOFT).exists():
        print(f"c_lint_census: no loft binary at {LOFT} (build it, or set LOFT_BIN)", file=sys.stderr)
        return 2
    files = a.files or corpus()
    if a.sample:
        random.seed(a.seed)
        files = random.sample(files, min(a.sample, len(files)))
    kinds, lines_, unused_kinds = collections.Counter(), collections.Counter(), collections.Counter()
    unused_lines = collections.Counter()
    where = collections.defaultdict(list)
    timeouts = []
    with cf.ThreadPoolExecutor(a.jobs) as ex:
        for path, warns, why in ex.map(lambda f: lint(f, a.runtime), files):
            if why:
                timeouts.append(path)
            for kind, msg, src in warns:
                # Under --runtime every helper a unit does not call is the audit's, not the
                # count's: the default run, which keeps the runtime's pragma, counts the rest.
                if a.runtime and kind == "-Wunused-function":
                    unused_kinds["runtime"] += 1
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
    if a.runtime:
        used = runtime_names()
        dead = sorted(f for f, u in used.items() if not u and f not in KEEP)
        print(f"runtime helpers a unit of this run does not call: {unused_kinds['runtime']} (not counted)")
        print(f"runtime functions nothing uses (dead code): {len(dead)}")
        for f in dead:
            print(f"  {f}")
        for f, why in KEEP.items():
            print(f"  kept, unused: {f} ({why})")
    if timeouts:
        print(f"timed out ({len(timeouts)}): {', '.join(timeouts[:5])}")
    return 1 if total or timeouts else 0


if __name__ == "__main__":
    sys.exit(main())
