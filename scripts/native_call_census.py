#!/usr/bin/env python3
"""Which runtime helpers does EMITTED code still call out of line?

The emitted program and the loft runtime are two crates and there is no LTO, so a
runtime helper reaches the emitted code inlined only when it is marked `#[inline]`;
every other helper is a real call through the GOT — and on a hot path that call, plus
the caller's register spills around it, is what a "check" costs, not the check itself
(@PLN157 § The floor: the drawing bench's `hash` row spent a third of its time in an
un-inlined `note_format_fault` whose fast path is one `bool` test).

    scripts/native_call_census.py <binary> [--fn <substring>] [--top N]

`<binary>` is a program compiled from `loft --native-emit … --lean` (or the binary
`--native-release` leaves behind); `--fn` restricts the per-function count to functions
whose symbol contains the substring.  Prints the call targets ranked by call SITES —
a site count, not a dynamic count: pair it with `scripts/profile.sh --engine` to know
which sites are hot.  A `#[cold]` slow half showing up here is the design working; the
finding is a FAST-path symbol on the list (`…::new`, `…::drop`, a length read).

Needs `objdump` (binutils).  Symbol names are printed mangled but readable
(`rustfilt` is not assumed); the crate name of the emitted program is taken from the
binary's own `main` symbol.
"""
import argparse
import collections
import re
import subprocess
import sys


def run(*cmd):
    return subprocess.run(cmd, capture_output=True, text=True, check=True).stdout


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("binary")
    ap.add_argument("--fn", default=None, help="only count sites inside functions whose symbol contains this")
    ap.add_argument("--top", type=int, default=40)
    a = ap.parse_args()

    asm = run("objdump", "-d", "--no-show-raw-insn", "-M", "intel", a.binary).split("\n")
    rel = {}
    for line in run("objdump", "-R", a.binary).split("\n"):
        m = re.match(r"^([0-9a-f]+)\s+\S+\s+\*ABS\*\+0x([0-9a-f]+)", line)
        if m:
            rel[int(m.group(1), 16)] = int(m.group(2), 16)
    addr2name = {}
    crate = None
    for line in asm:
        m = re.match(r"^([0-9a-f]+) <(.*)>:", line)
        if m:
            addr2name[int(m.group(1), 16)] = m.group(2)
            # `_RNvCs<hash>_<len><crate>4main` — the emitted program's own `main`.
            mm = re.match(r"^_RNvCs[0-9A-Za-z]+_(\d+)([A-Za-z0-9_]+)4main$", m.group(2))
            if mm and crate is None:
                n = int(mm.group(1))
                crate = mm.group(2)[:n]
    if crate is None:
        sys.exit("no emitted `main` symbol found — is this a loft --native program?")

    cur = None
    sites = collections.Counter()
    fns_with = collections.defaultdict(set)
    emitted_fns = 0
    for line in asm:
        m = re.match(r"^[0-9a-f]+ <(.*)>:", line)
        if m:
            cur = m.group(1)
            if crate in cur:
                emitted_fns += 1
            continue
        if not (cur and crate in cur):
            continue
        if a.fn and a.fn not in cur:
            continue
        m = re.search(r"\bcall\s+QWORD PTR \[rip\+0x[0-9a-f]+\]\s+#\s*([0-9a-f]+)", line)
        if m:
            got = int(m.group(1), 16)
            name = addr2name.get(rel.get(got, -1), "?got:%x" % got)
        else:
            m = re.search(r"\bcall\s+[0-9a-f]+ <([^>]*)>", line)
            if not m:
                continue
            name = re.sub(r"\+0x[0-9a-f]+$", "", m.group(1))
            if crate in name:
                continue  # a call between emitted functions is the program, not the runtime
        sites[name] += 1
        fns_with[name].add(cur)

    total = sum(sites.values())
    print(f"crate {crate}: {emitted_fns} emitted functions, {total} out-of-line call sites, {len(sites)} distinct targets"
          + (f" (inside *{a.fn}*)" if a.fn else ""))
    print(f"{'sites':>6} {'fns':>4}  target")
    for name, n in sites.most_common(a.top):
        print(f"{n:6d} {len(fns_with[name]):4d}  {name[-100:]}")


if __name__ == "__main__":
    main()
