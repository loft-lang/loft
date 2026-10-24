#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The runtime surface emitted Rust names (@PLN200 B3.5): every item of the loft crate that a
compiled program reaches, counted over a corpus of programs.

    scripts/runtime_surface.py                 # the report on stdout
    scripts/runtime_surface.py --write         # doc/claude/RUNTIME_SURFACE.md
    LOFT_BIN=<loft> scripts/runtime_surface.py # another build (default target/release/loft)

The corpus is every `bench/*/bench.loft` and `tests/scripts/*.loft`, emitted with
`loft introspect <file> --show-rust`, plus the compiled standard library
(`src/compiled_stdlib_gen.rs`), which is library emission.  From each text it reads:

- every path `loft::<module>::<item>` and every `use loft::…` import;
- every bare call of a function that `use loft::codegen_runtime::*` brings in;
- every method call whose name a runtime module defines as a method (`pub fn name(&self …`),
  minus the names of common `std` methods.  This is BY NAME: a method two types define is
  counted for both, so read the method rows as an upper bound.

Each item is attributed to the module that defines it and marked when that module is outside
the runtime set (`scripts/crate_layers.py` RUNTIME): those are what a compiled unit links
beyond the runtime, the first thing to cut before units stop embedding it (@PLN200 B7.3).
A report, never a gate.
"""
import concurrent.futures
import os
import re
import subprocess
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "scripts"))
import crate_layers  # noqa: E402

LOFT = os.environ.get("LOFT_BIN") or str(ROOT / "target" / "release" / "loft")
STD_METHODS = {
    "as_ref", "as_str", "borrow", "borrow_mut", "chars", "clear", "clone", "cmp", "contains",
    "default", "drop", "ends_with", "eq", "extend", "find", "first", "fmt", "from", "get",
    "get_mut", "hash", "insert", "into", "is_empty", "is_none", "is_some", "iter", "iter_mut",
    "last", "len", "map", "max", "min", "new", "next", "partial_cmp", "pop", "push", "push_str",
    "remove", "replace", "reserve", "resize", "split", "starts_with", "take", "to_owned",
    "to_string", "trim", "truncate", "unwrap", "with_capacity", "write", "abs", "contains_key",
}


def corpus() -> list[Path]:
    return sorted(ROOT.glob("bench/*/bench.loft")) + sorted((ROOT / "tests" / "scripts").glob("*.loft"))


def emit(path: Path) -> str:
    r = subprocess.run([LOFT, "introspect", str(path), "--show-rust"], capture_output=True,
                       text=True, timeout=300, cwd=ROOT)
    return r.stdout


def strip_comments(text: str) -> str:
    return "\n".join(l.split("//", 1)[0] if '"' not in l else l for l in text.split("\n"))


def runtime_definitions():
    """(module of each top-level pub item, module(s) of each method name) over the runtime set."""
    items, methods = {}, defaultdict(set)
    mods = crate_layers.lib_modules()
    for owner, path, _top in crate_layers.module_files(mods):
        if owner not in crate_layers.RUNTIME:
            continue
        text = path.read_text(encoding="utf-8")
        for m in re.finditer(r"^pub (?:const |unsafe |extern \"C\" )*fn ([a-z_][a-z0-9_]*)", text, re.M):
            items.setdefault(m.group(1), owner)
        for m in re.finditer(r"^\s+pub (?:const |unsafe )?fn ([a-z_][a-z0-9_]*)(?:<[^>]*>)?\(\s*&(?:'[a-z]+ )?(?:mut )?self", text, re.M):
            if m.group(1) not in STD_METHODS:
                methods[m.group(1)].add(owner)
    return items, methods


def use_paths(text: str) -> list[str]:
    """`use loft::a::{b, c::d};` → ["a::b", "a::c::d"] (one level of braces)."""
    out = []
    for m in re.finditer(r"use loft::([^;]+);", text):
        body = m.group(1).replace("\n", " ")
        g = re.match(r"([A-Za-z_0-9:]*?)::?\{(.*)\}$", body.strip())
        if g:
            out += [f"{g.group(1)}::{p.strip()}".strip(":") for p in g.group(2).split(",") if p.strip()]
        else:
            out.append(body.strip())
    return out


def census():
    items, methods = runtime_definitions()
    runtime_fns = {n for n, o in items.items() if o == "codegen_runtime"}
    files = corpus()
    with concurrent.futures.ThreadPoolExecutor(max_workers=os.cpu_count() or 4) as ex:
        texts = list(ex.map(emit, files))
    texts.append((ROOT / "src" / "compiled_stdlib_gen.rs").read_text(encoding="utf-8"))
    paths, calls, meths = defaultdict(int), defaultdict(int), defaultdict(int)
    for raw in texts:
        text = strip_comments(raw)
        seen_p = {m.group(1) for m in re.finditer(r"\bloft::([a-z_][a-z0-9_]*(?:::[A-Za-z_][A-Za-z0-9_]*)*)", text)}
        seen_p |= set(use_paths(text))
        for p in seen_p:
            paths[p] += 1
        for n in {m.group(1) for m in re.finditer(r"(?<![.:\w])([a-z_][a-z0-9_]*)\(", text)} & runtime_fns:
            calls[n] += 1
        for n in {m.group(1) for m in re.finditer(r"\.([a-z_][a-z0-9_]*)\(", text)} & set(methods):
            meths[n] += 1
    return len(texts), paths, calls, meths, methods


def report() -> str:
    n, paths, calls, meths, methods = census()
    runtime = set(crate_layers.RUNTIME)
    out = [
        "<!-- size-exempt: a generated report (scripts/runtime_surface.py --write) -->",
        "# The runtime surface emitted Rust names",
        "",
        f"GENERATED by `scripts/runtime_surface.py --write` over {n} emitted programs (every",
        "`bench/*/bench.loft` and `tests/scripts/*.loft`, and the compiled standard library).",
        "Each row counts the programs whose emitted Rust names the item.  It is the input to the",
        "C backend's runtime decision (@PLN91) and to the versioned runtime interface (@PLN200",
        "B7.8); method rows are matched by NAME and are an upper bound.  A report, never a gate.",
        "The program rows are the default `--native` emission (`introspect --show-rust`), which",
        "carries the live tier (`live_dispatch`); a library artifact, like the compiled standard",
        "library, does not.",
        "",
    ]
    outside = sorted({p.split("::")[0] for p in paths} - runtime - {"codegen_runtime"} - {""})
    out += ["## Outside the runtime set", "",
            "Modules emitted code names that are not in the runtime set "
            "(`scripts/crate_layers.py` RUNTIME): what a compiled unit links beyond the runtime.",
            ""]
    if not outside:
        out.append("None.")
    else:
        out += ["| module | items (programs) |", "|---|---|"]
        for m in outside:
            its = sorted((p, c) for p, c in paths.items() if p.split("::")[0] == m)
            out.append(f"| `{m}` | " + ", ".join(f"`{p}` ({c})" for p, c in its) + " |")
    out += ["", "## Paths and imports", "", "| module | items (programs) |", "|---|---|"]
    by_mod = defaultdict(list)
    for p, c in paths.items():
        by_mod[p.split("::")[0]].append((p, c))
    for m in sorted(by_mod):
        if m in outside:
            continue
        out.append(f"| `{m}` | " + ", ".join(f"`{p}` ({c})" for p, c in sorted(by_mod[m])) + " |")
    out += ["", "## Runtime functions (`use loft::codegen_runtime::*`)", "",
            f"{len(calls)} functions, by the number of programs that call them:", ""]
    out.append(", ".join(f"`{f}` ({c})" for f, c in sorted(calls.items(), key=lambda x: (-x[1], x[0]))))
    out += ["", "## Methods (by name)", "", "| method | defined in | programs |", "|---|---|---|"]
    for name, c in sorted(meths.items(), key=lambda x: (-x[1], x[0])):
        out.append(f"| `{name}` | {', '.join(sorted(methods[name]))} | {c} |")
    return "\n".join(out) + "\n"


def main(argv: list[str]) -> int:
    text = report()
    if "--write" in argv:
        (ROOT / "doc" / "claude" / "RUNTIME_SURFACE.md").write_text(text, encoding="utf-8")
        print("wrote doc/claude/RUNTIME_SURFACE.md")
    else:
        sys.stdout.write(text)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
