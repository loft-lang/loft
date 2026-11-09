#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The Rust front end's state, and where the loft-in-loft elaborator (@PLN91) mirrors it.

The Rust parser decides much of the IR from state it carries across calls — `Parser`'s fields
and the per-function `Function` table — not from the arguments of the function at hand.  A port
that mirrors the functions but not that state rediscovers each channel only when a gate diff
exposes it.  This lists every field with how long it lives:

  scoped   saved and restored around a sub-parse (`mem::replace` / `mem::take` of `self.<f>`):
           context a caller hands down into an expression — the channels to mirror first
  pass     reset between the two passes (`passes.rs`)
  other    program-wide, per-definition or bookkeeping

and the elaborator field that mirrors it: a field of the same name in `El` (selfhost/src/el.loft)
or `Vars` (selfhost/src/el_vars.loft), else the entry in selfhost/front/state-map.tsv
(`<Struct>.<field>\t<port>\t<note>`; a port of `-` records a field the port does not need, the
note saying why).

    scripts/parser_state.py            write doc/claude/plans/91-self-host/parser-state.md
    scripts/parser_state.py --missing  the scoped channels nothing mirrors yet
    scripts/parser_state.py --check    fail on a map entry whose Rust or port field is gone
"""
import argparse, collections, glob, os, re, sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MAP = os.path.join(ROOT, "selfhost", "front", "state-map.tsv")
OUT = os.path.join(ROOT, "doc", "claude", "plans", "91-self-host", "parser-state.md")
RUST = [("Parser", "src/parser/mod.rs"), ("Function", "src/variables/mod.rs")]
PORT = [("El", "selfhost/src/el.loft"), ("Vars", "selfhost/src/el_vars.loft")]


def read(path):
    with open(os.path.join(ROOT, path), encoding="utf-8") as fh:
        return fh.read()


def struct_body(src, name, rust):
    """The lines of `struct <name> {` up to its closing brace at column 0."""
    head = re.compile(r"^pub struct %s \{" % name if rust else r"^pub struct %s \{" % name)
    lines = src.splitlines()
    for i, line in enumerate(lines):
        if head.match(line):
            body = []
            for line2 in lines[i + 1:]:
                if line2.startswith("}"):
                    return body
                body.append(line2)
    sys.exit(f"no `pub struct {name}` found")


def rust_fields(src, name):
    """(field, first doc line) in declaration order."""
    out, doc = [], []
    for line in struct_body(src, name, True):
        s = line.strip()
        if s.startswith("///"):
            doc.append(s[3:].strip())
            continue
        m = re.match(r"^    (?:pub(?:\([a-z]+\))? )?([a-z_][a-z_0-9]*):", line)
        if m:
            first = next((d for d in doc if d), "")
            out.append((m.group(1), first))
        if not s.startswith("#[") and s:
            doc = []
    return out


def port_fields(src, name):
    out = set()
    for line in struct_body(src, name, False):
        m = re.match(r"^\s+(?:pub )?([a-z_][a-z_0-9]*):", line)
        if m:
            out.add(m.group(1))
    return out


def lifetimes():
    """Per Parser field: how many scoped save/restores, and whether `passes.rs` resets it."""
    scoped = collections.Counter()
    for path in glob.glob(os.path.join(ROOT, "src", "parser", "*.rs")):
        for m in re.finditer(r"mem::(?:replace|take)\(\s*&mut self\.([a-z_0-9]+)", read(path)):
            scoped[m.group(1)] += 1
    passes = read("src/parser/passes.rs")
    reset = set(re.findall(r"self\.([a-z_0-9]+)\.clear\(\)", passes))
    reset |= set(re.findall(r"self\.([a-z_0-9]+) = ", passes))
    return scoped, reset


def load_map():
    m = {}
    if os.path.exists(MAP):
        with open(MAP, encoding="utf-8") as fh:
            for n, line in enumerate(fh, 1):
                if not line.strip() or line.startswith("#"):
                    continue
                parts = line.rstrip("\n").split("\t")
                if len(parts) < 2:
                    sys.exit(f"{MAP}:{n}: want `<Struct>.<field>\\t<port>\\t<note>`")
                m[parts[0]] = (parts[1], parts[2] if len(parts) > 2 else "")
    return m


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--missing", action="store_true", help="list the scoped channels nothing mirrors")
    ap.add_argument("--check", action="store_true", help="fail on a stale map entry")
    o = ap.parse_args()
    ports = {n: port_fields(read(p), n) for n, p in PORT}
    scoped, reset = lifetimes()
    mapping = load_map()
    rows = []
    for struct, path in RUST:
        for f, doc in rust_fields(read(path), struct):
            key = f"{struct}.{f}"
            if struct == "Parser":
                life = "scoped" if scoped[f] else "pass" if f in reset else "other"
            else:
                life = "function"
            port, note = mapping.get(key, ("", ""))
            if not port:
                same = [f"{n}.{f}" for n in ("El", "Vars") if f in ports[n]]
                port = same[0] if same else ""
            rows.append((struct, f, life, scoped[f] if struct == "Parser" else 0, port, note, doc))
    if o.check:
        known = {f"{r[0]}.{r[1]}" for r in rows}
        bad = [k for k in mapping if k not in known]
        for k, (port, _) in mapping.items():
            if port not in ("-", "") and "." in port:
                s, f = port.split(".", 1)
                if s in ports and f.split("/")[0] not in ports[s]:
                    bad.append(f"{k} -> {port}")
        for b in bad:
            print(f"stale: {b}")
        sys.exit(1 if bad else 0)
    if o.missing:
        for r in sorted((r for r in rows if r[2] == "scoped" and not r[4]), key=lambda r: -r[3]):
            print(f"{r[3]:3}  {r[1]:32} {r[6][:90]}")
        return
    write(rows)


def write(rows):
    def table(sel):
        lines = ["| field | lives | saves | port | note / Rust doc |", "|---|---|---|---|---|"]
        for s, f, life, n, port, note, doc in sel:
            text = (note or doc).replace("|", "\\|")
            if len(text) > 110:
                text = text[:107] + "…"
            lines.append(f"| `{s}.{f}` | {life} | {n or ''} | {('`' + port + '`') if port else '—'} | {text} |")
        return "\n".join(lines)
    by = collections.Counter((r[0], r[2], bool(r[4])) for r in rows)
    parser = [r for r in rows if r[0] == "Parser"]
    summary = []
    for life in ("scoped", "pass", "other"):
        tot = sum(1 for r in parser if r[2] == life)
        summary.append(f"{life} {by[('Parser', life, True)]} of {tot}")
    func = [r for r in rows if r[0] == "Function"]
    body = f"""<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 — the Rust front end's state and its mirror

GENERATED by `scripts/parser_state.py` — do not edit; edit `selfhost/front/state-map.tsv` or the
structs, then run the script.  What each `Parser` and `Function` field is, how long it lives, and
the elaborator field mirroring it (`—`: none yet).  A *scoped* field is saved and restored around
a sub-parse: context a caller hands down into an expression, invisible from the function that
reads it — mirror it before porting a function that reads it (`--missing` lists those not yet).

Mirrored: `Parser` {", ".join(summary)}; `Function` {sum(1 for r in func if r[4])} of {len(func)}.

## `Parser`, scoped channels

{table(sorted((r for r in parser if r[2] == "scoped"), key=lambda r: (-r[3], r[1])))}

## `Parser`, reset per pass

{table([r for r in parser if r[2] == "pass"])}

## `Parser`, other

{table([r for r in parser if r[2] == "other"])}

## `Function` (per function; the elaborator's `Vars`)

{table(func)}
"""
    with open(OUT, "w", encoding="utf-8") as fh:
        fh.write(body)
    print(f"wrote {os.path.relpath(OUT, ROOT)}: Parser {', '.join(summary)}; "
          f"Function {sum(1 for r in func if r[4])} of {len(func)}")


if __name__ == "__main__":
    main()
