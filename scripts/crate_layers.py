#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The module graph of the `loft` library crate, and the layers it is split into (@PLN191).

A crate can only depend on crates below it, so a split of `src/` into crates needs every
reference between top-level modules to point DOWN the layering.  This script reads the
references from the tree — `crate::m`, `use crate::{m, …}`, `super::m` from a top-level file,
the exported macros by name — and answers:

  graph [MOD]     every module's outgoing edges with counts (or one module's in + out)
  scc             the strongly connected components with more than one module: a cycle
                  cannot be split across crates, so each is cut or kept in one crate
  check [-v]      every module assigned to exactly one layer in LAYERS below, and every
                  BACK-EDGE (a reference from a lower layer to a higher one) by file:line;
                  exit 1 when a module is unassigned or a back-edge count exceeds BUDGET
  sites A B       every reference from module A to module B, by file:line
  move MOD LAYER  put module MOD in LAYER (rewrites LAYERS in this file, sorted)
  lower           set every budget the tree has fallen below to the tree's count; it never
                  raises one — an upward reference is cut, not budgeted

Only the library is read: `src/main.rs`, `src/bin/` and `src/gendoc.rs` are binaries, which
sit above every layer.  Comments are skipped; code in `#[cfg(test)]` blocks is counted,
because a test in a lower crate that names a higher one has to move too.
"""

import bisect
import re
import sys
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SRC = ROOT / "src"

# The layers, bottom up.  A module may reference its own layer and any layer below.
# The big modules were placed by subject (core: the IR types, keys, stores, database;
# front: lexer-to-IR; gen: the native generator; runtime: the interpreter and what
# emitted programs call; facade: the tools); every other module sits where the fewest
# references cross a layer upward.  A new module is added to the layer it belongs in.
# `lib` is lib.rs's own items — its macros and host imports, which every layer uses.
LAYERS: list[tuple[str, list[str]]] = [
    ("core", [
        "api_diff", "api_surface", "arena", "base64", "btree", "build_phase", "byte_copy",
        "c_shim", "c_signature", "cache", "cache_gc", "calc", "call_frames", "compact",
        "const_eval", "coroutine_layout", "crash_report", "data", "data_ref", "data_store",
        "database", "diagnostic_render", "diagnostics", "doc_catalogue", "doc_construct",
        "doc_render", "documentation", "exit_record", "exit_vector", "ffi_deliver", "file_access",
        "forward_result", "fxhash", "git_query", "hash", "html_embed", "html_fonts", "install",
        "integrity", "ir_schema", "ir_schema_gen", "json", "keys", "lexer", "lib", "libscan",
        "limits", "literal_append", "lockfile", "log_config", "logger", "loop_shape", "manifest",
        "migrate_long", "narrow", "net", "net_profile", "null_census", "op_census", "op_sets",
        "ops", "package", "package_layout", "page_metrics", "paged_reader", "parallel",
        "placement", "platform", "platform_census", "png_store", "process_run", "pure_reuse",
        "radix_db", "radix_tree", "registry", "registry_advisories", "registry_index",
        "registry_keys", "registry_signing", "repeat_run", "repl_doc", "resolution",
        "resolution_scope", "return_field", "rewrite_census", "runtime_error", "sandbox",
        "schema_sidecar", "script", "self_update", "siphash", "slot_alias", "spatial",
        "stack_census", "stack_verify", "stdlib_sources", "store", "store_budget", "store_census",
        "take_local", "text_calls", "text_run", "timeout", "tls", "trace", "trie_db", "triggers",
        "tuple_links", "variables", "vec_copy", "vector", "verify_self", "wasm", "wasm_assets",
        "wasm_gl",
    ]),
    ("front", [
        "compile", "const_fn", "const_literal", "copy_manifest", "copy_view", "discharge_into",
        "forward_walk", "in_range", "ir_node", "ir_read", "ir_store", "lease", "ownership_cfg",
        "parser", "place_result", "rebind_place", "same_read", "scopes", "single_use", "spent",
        "typedef", "use_analysis",
    ]),
    ("gen", [
        "ctor_literal", "debugger", "engine_host", "generation", "leaf_inline", "live_dispatch",
        "live_reload", "native", "rpc", "value_record",
    ]),
    ("runtime", [
        "c_call", "codegen_runtime", "compiled_stdlib", "create", "extensions", "fill",
        "fuzz_keyed", "fuzz_oracle", "introspect", "loop_kernels", "native_gate", "native_lib",
        "profiler", "stack", "state", "stdlib_ops",
    ]),
    ("facade", [
        "doc_site", "fix_apply", "host", "lib_placement", "lsp", "repl", "serve", "startup_cache",
        "wasm_debug", "wasm_threads",
    ]),
]

# Back-edges allowed per (from-layer, to-layer) while P2 of @PLN191 cuts them: a ratchet.
# A count may only fall; `check` fails when one rises, and when one has fallen below its
# budget without the budget being lowered, so the number here is always the tree's.
BUDGET: dict[tuple[str, str], int] = {
    ("core", "front"): 10,
    ("core", "runtime"): 17,
    ("front", "gen"): 11,
    ("front", "runtime"): 9,
    ("gen", "facade"): 6,
    ("gen", "runtime"): 19,
}

BIN_FILES = {"main.rs", "gendoc.rs"}


def lib_modules() -> dict[str, Path]:
    """Top-level modules declared in lib.rs, mapped to their file or directory."""
    text = (SRC / "lib.rs").read_text()
    mods = {}
    for m in re.finditer(r"^\s*(?:pub(?:\([a-z]+\))?\s+)?mod\s+([a-z_0-9]+)\s*;", text, re.M):
        name = m.group(1)
        if (SRC / f"{name}.rs").exists():
            mods[name] = SRC / f"{name}.rs"
        elif (SRC / name / "mod.rs").exists():
            mods[name] = SRC / name
    return mods


def module_files(mods: dict[str, Path]) -> list[tuple[str, Path, bool]]:
    """(top-level module, file, is_top_file) for every library source file.

    `is_top_file` is true for a file whose `super` is the crate root: `src/x.rs` and
    `src/x/mod.rs`.  A `#[path]`-included file is attributed to the directory it sits in."""
    out = []
    for name, p in mods.items():
        if p.is_file():
            out.append((name, p, True))
            sub = SRC / name
            if sub.is_dir():
                for f in sorted(sub.rglob("*.rs")):
                    out.append((name, f, False))
        else:
            for f in sorted(p.rglob("*.rs")):
                out.append((name, f, f.name == "mod.rs" and f.parent == p))
    out.append(("lib", SRC / "lib.rs", True))
    return out


# Comments, string literals and char literals, matched left to right so a `//` inside a
# string is no comment and a quote inside a comment is no string.  The native generator
# holds Rust it EMITS as strings (`"crate::state::…"`); those are not references.
_NOISE = re.compile(
    r'/(?:/[^\n]*|\*.*?\*/)'      # a line or block comment
    r'|"(?:\\.|[^"\\])*"'          # a string (a `b"…"` byte string from its quote)
    r"|'(?:\\.|[^'\\])'"           # a char literal; a lifetime never closes, so never matches
    r'|r(#*)".*?"\1',                # a raw string, `r#"…"#`
    re.S,
)


def _blank(m: re.Match) -> str:
    s = m.group(0)
    if "\n" not in s:
        return " " * len(s)
    return "\n".join(" " * len(x) for x in s.split("\n"))


def blank_noise(text: str) -> str:
    """The text with every comment and literal blanked, newlines kept, so offsets and line
    numbers are the source's."""
    return _NOISE.sub(_blank, text)


def macro_homes(mods: dict[str, Path]) -> dict[str, str]:
    """`#[macro_export]` macro name -> the module defining it."""
    homes = {}
    for name, p, _top in module_files(mods):
        text = p.read_text()
        for m in re.finditer(r"#\[macro_export\]\s*(?:#\[[^\]]*\]\s*)*macro_rules!\s*([a-z_0-9]+)", text):
            homes[m.group(1)] = name
    return homes


def lib_reexports() -> dict[str, str]:
    """`pub use m::item;` in lib.rs: item -> m, so `crate::item` resolves to m."""
    out = {}
    for m in re.finditer(r"^\s*pub\s+use\s+([a-z_0-9]+)::(?:[a-z_0-9]+::)*([A-Za-z_0-9]+)\s*;",
                         (SRC / "lib.rs").read_text(), re.M):
        out[m.group(2)] = m.group(1)
    return out


USE_GROUP = re.compile(r"use\s+(?:crate|super)::\{")
CRATE_PATH = re.compile(r"crate::([A-Za-z_0-9]+)")
SUPER_PATH = re.compile(r"super::([A-Za-z_0-9]+)")


def group_heads(text: str, start: int) -> tuple[list[str], int]:
    """The depth-1 path heads of a `use crate::{ … }` group starting at `{`."""
    depth = 0
    heads = []
    token = ""
    expecting = True
    i = start
    while i < len(text):
        c = text[i]
        if c == "{":
            depth += 1
            if depth == 1:
                expecting = True
        elif c == "}":
            depth -= 1
            if depth == 0:
                return heads, i
        elif depth == 1 and c == ",":
            expecting = True
        elif depth == 1 and expecting and (c.isalnum() or c == "_"):
            token += c
            if not (text[i + 1].isalnum() or text[i + 1] == "_"):
                heads.append(token)
                token = ""
                expecting = False
        i += 1
    return heads, i


def references(mods: dict[str, Path]):
    """Yield (from_module, to_module, file, line_no) for every cross-module reference."""
    names = set(mods)
    macros = macro_homes(mods)
    reexp = lib_reexports()

    def resolve(head: str) -> str | None:
        if head in names:
            return head
        if head in reexp:
            return reexp[head]
        if head in macros:
            return macros[head]
        if head in ("self", "super", "crate"):
            return None
        return "lib"

    for owner, path, top in module_files(mods):
        text = blank_noise(path.read_text())
        starts = [0] + [m.end() for m in re.finditer("\n", text)]

        def line_of(pos: int) -> int:
            return bisect.bisect_right(starts, pos)

        def bare(pos: int) -> bool:
            """No identifier character (or `::`) right before `pos`: a path that STARTS here."""
            return pos == 0 or not (text[pos - 1].isalnum() or text[pos - 1] in "_:")

        # Every search below starts on a literal, so the regex engine skips ahead with a fast
        # substring search instead of trying a pattern at every character.
        for m in USE_GROUP.finditer(text):
            if not bare(m.start()) or ("super" in m.group(0) and not top):
                continue
            heads, _end = group_heads(text, m.end() - 1)
            for h in heads:
                tgt = resolve(h)
                if tgt and tgt != owner:
                    yield owner, tgt, path, line_of(m.start())
        for m in CRATE_PATH.finditer(text):
            if not (bare(m.start()) or text[m.start() - 1] == "$"):
                continue
            tgt = resolve(m.group(1))
            if tgt and tgt != owner:
                yield owner, tgt, path, line_of(m.start())
        if top:
            for m in SUPER_PATH.finditer(text):
                if not bare(m.start()):
                    continue
                tgt = resolve(m.group(1))
                if tgt and tgt != owner:
                    yield owner, tgt, path, line_of(m.start())
        if macros:
            pos = text.find("!")
            while pos >= 0:
                start = pos
                while start > 0 and (text[start - 1].isalnum() or text[start - 1] == "_"):
                    start -= 1
                name = text[start:pos]
                # `crate::name!` is counted with the paths above
                if name in macros and not text[max(0, start - 2):start] == "::":
                    tgt = macros[name]
                    if tgt != owner:
                        yield owner, tgt, path, line_of(start)
                pos = text.find("!", pos + 1)


def edge_counts(mods):
    edges = defaultdict(lambda: defaultdict(int))
    for a, b, _p, _l in references(mods):
        edges[a][b] += 1
    return edges


def sccs(nodes, edges):
    """Tarjan's algorithm; returns the components, each a sorted list."""
    index = {}
    low = {}
    stack = []
    on = set()
    out = []
    counter = [0]
    sys.setrecursionlimit(10000)

    def visit(v):
        index[v] = low[v] = counter[0]
        counter[0] += 1
        stack.append(v)
        on.add(v)
        for w in edges.get(v, {}):
            if w not in index:
                visit(w)
                low[v] = min(low[v], low[w])
            elif w in on:
                low[v] = min(low[v], index[w])
        if low[v] == index[v]:
            comp = []
            while True:
                w = stack.pop()
                on.discard(w)
                comp.append(w)
                if w == v:
                    break
            out.append(sorted(comp))

    for v in sorted(nodes):
        if v not in index:
            visit(v)
    return out


def layer_of() -> dict[str, tuple[int, str]]:
    out = {}
    for i, (layer, members) in enumerate(LAYERS):
        for m in members:
            out[m] = (i, layer)
    return out


def size(p: Path) -> int:
    if p.is_file():
        n = len(p.read_text().split("\n"))
        sub = p.parent / p.stem
        if sub.is_dir():
            n += sum(len(f.read_text().split("\n")) for f in sub.rglob("*.rs"))
        return n
    return sum(len(f.read_text().split("\n")) for f in p.rglob("*.rs"))


def rewrite_tables(layers, budget) -> None:
    """Write LAYERS and BUDGET back into this file, each layer's modules sorted and wrapped."""
    me = Path(__file__)
    src = me.read_text()
    out = ["LAYERS: list[tuple[str, list[str]]] = ["]
    for name, members in layers:
        words = [f'"{m}",' for m in sorted(members)]
        lines, cur = [], "       "
        for w in words:
            if len(cur) + 1 + len(w) > 98:
                lines.append(cur)
                cur = "       "
            cur += " " + w
        lines.append(cur)
        out.append(f'    ("{name}", [')
        out.extend(lines)
        out.append("    ]),")
    out.append("]")
    a = src.index("LAYERS: list[tuple[str, list[str]]] = [")
    b = src.index("\n]\n", a) + 3
    src = src[:a] + "\n".join(out) + "\n" + src[b:]
    out = ["BUDGET: dict[tuple[str, str], int] = {"]
    for (x, y), n in sorted(budget.items()):
        if n:
            out.append(f'    ("{x}", "{y}"): {n},')
    out.append("}")
    a = src.index("BUDGET: dict[tuple[str, str], int] = {")
    b = src.index("\n}\n", a) + 3
    src = src[:a] + "\n".join(out) + "\n" + src[b:]
    me.write_text(src)


def back_counts(mods) -> dict[tuple[str, str], int]:
    lo = layer_of()
    counts: dict[tuple[str, str], int] = defaultdict(int)
    for a, b, _p, _l in references(mods):
        if a in lo and b in lo and lo[b][0] > lo[a][0]:
            counts[(lo[a][1], lo[b][1])] += 1
    return counts


def main(argv):
    mods = lib_modules()
    cmd = argv[1] if len(argv) > 1 else "check"
    if cmd == "graph":
        edges = edge_counts(mods)
        if len(argv) > 2:
            m = argv[2]
            print(f"{m} -> " + "  ".join(f"{b}({n})" for b, n in sorted(edges[m].items(), key=lambda x: -x[1])))
            ins = {a: e[m] for a, e in edges.items() if m in e}
            print(f"{m} <- " + "  ".join(f"{a}({n})" for a, n in sorted(ins.items(), key=lambda x: -x[1])))
            return 0
        for a in sorted(edges):
            print(f"{a}: " + " ".join(f"{b}({n})" for b, n in sorted(edges[a].items(), key=lambda x: -x[1])))
        return 0
    if cmd == "scc":
        edges = edge_counts(mods)
        nodes = set(mods) | {"lib"}
        for comp in sorted(sccs(nodes, edges), key=len, reverse=True):
            if len(comp) > 1:
                lines = sum(size(mods[c]) for c in comp if c in mods)
                print(f"{len(comp)} modules, {lines} lines: {' '.join(comp)}")
        return 0
    if cmd == "move":
        mod, layer = argv[2], argv[3]
        names = [n for n, _ in LAYERS]
        if layer not in names:
            print(f"no layer {layer!r}; the layers are {' '.join(names)}")
            return 2
        layers = [(n, [m for m in ms if m != mod] + ([mod] if n == layer else [])) for n, ms in LAYERS]
        rewrite_tables(layers, BUDGET)
        print(f"{mod} -> {layer}; run `check` (and `lower` when a budget fell)")
        return 0
    if cmd == "lower":
        counts = back_counts(mods)
        new = dict(BUDGET)
        raised = []
        for key in set(counts) | set(BUDGET):
            have, allowed = counts.get(key, 0), BUDGET.get(key, 0)
            if have < allowed:
                print(f"{key[0]} -> {key[1]}: budget {allowed} -> {have}")
                new[key] = have
            elif have > allowed:
                raised.append(f"{key[0]} -> {key[1]}: {have} upward reference(s), budget {allowed}")
        rewrite_tables(LAYERS, new)
        for r in raised:
            print(f"NOT raised — cut these instead: {r}")
        return 1 if raised else 0
    if cmd == "sites":
        a, b = argv[2], argv[3]
        for x, y, p, l in references(mods):
            if x == a and y == b:
                print(f"{p.relative_to(ROOT)}:{l}")
        return 0
    if cmd == "check":
        lo = layer_of()
        bad = 0
        verbose = "-v" in argv
        unassigned = sorted(set(mods) - set(lo))
        if unassigned:
            print(f"unassigned modules ({len(unassigned)}): {' '.join(unassigned)}")
            print("  add each to the layer it belongs in (LAYERS in scripts/crate_layers.py)")
            bad = 1
        stale = sorted(m for m in lo if m != "lib" and m not in mods)
        if stale:
            print(f"LAYERS names modules lib.rs no longer declares: {' '.join(stale)}")
            bad = 1
        back = defaultdict(list)
        for a, b, p, l in references(mods):
            if a in lo and b in lo and lo[b][0] > lo[a][0]:
                back[(lo[a][1], lo[b][1])].append(f"{p.relative_to(ROOT)}:{l} {a} -> {b}")
        for key in sorted(set(back) | set(BUDGET)):
            sites = back.get(key, [])
            allowed = BUDGET.get(key, 0)
            if len(sites) > allowed:
                bad = 1
                print(f"{key[0]} -> {key[1]}: {len(sites)} upward reference(s), budget {allowed}: "
                      "a lower layer may not name a higher one")
                for s in sites:
                    print(f"    {s}")
            elif len(sites) < allowed:
                bad = 1
                print(f"{key[0]} -> {key[1]}: {len(sites)} upward reference(s), budget {allowed}: "
                      f"lower BUDGET[({key[0]!r}, {key[1]!r})] to {len(sites)}")
            elif verbose:
                print(f"{key[0]} -> {key[1]}: {len(sites)} upward reference(s)")
                for s in sites:
                    print(f"    {s}")
        return bad
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
