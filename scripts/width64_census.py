#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Where a 32-bit image does 64-bit work: the census the 32-bit port is measured by (@PLN217).

On a 32-bit RISC-V core with a single-precision FPU (the Tanmatsu's ESP32-P4) a `double` is
software and a 64-bit integer a register pair, so 64-bit work leaves three marks in the linked
code, and this reads all three per function from the image itself — what ships, after the
linker dropped what nothing reaches:

  * double  — a call to a soft-float double helper (`__adddf3`, `__muldf3`, `__divdf3`,
              `__extendsfdf2`, `__fixdfsi`, ...);
  * i64     — a call to a 64-bit integer helper (`__muldi3`, `__divdi3`, `__moddi3`, ...);
  * carry   — an inline 64-bit add or subtract: an `add`/`sub` whose carry or borrow an
              adjacent `sltu` computes from the same registers, the pattern a compiler writes
              for the low half of a 64-bit sum.  A heuristic: a 32-bit unsigned compare that
              happens to sit beside an add can count too, so read it as "look here".

Each function is attributed to where it came from — the program, the loft runtime, a library's
C file, ESP-IDF — from the link map, so the report says whose code to change.

Usage:
    scripts/width64_census.py <esp-project-dir>         # e.g. the --native-esp output dir
    scripts/width64_census.py <dir> --top 40            # more of the worst functions
    scripts/width64_census.py <dir> --origin runtime    # one origin's functions only
    scripts/width64_census.py <dir> --json              # machine-readable
"""
import argparse
import glob
import json
import os
import re
import shutil
import subprocess
import sys
from collections import defaultdict

DOUBLE_HELPERS = re.compile(
    r"^__(add|sub|mul|div|neg|eq|ne|lt|le|gt|ge|unord|cmp)df[23]$|^__(extend|trunc)[sd]f[sd]f2$|"
    r"^__fix(uns)?df[sd]i$|^__float(un)?[sd]idf$|^__powidf2$")
INT64_HELPERS = re.compile(r"^__(u?(div|mod)|mul|ashl|ashr|lshr|neg|cmp|ucmp|clz|ctz|popcount|bswap)di[23]$")
# A direct call (`jal ra,<addr> <f>`), and one through a register to a ROM helper, which is how the
# ESP32-P4 reaches libgcc's soft-float and 64-bit routines (`jalr 244(ra) # 4fc00868 <__adddf3>`).
CALL = re.compile(r"\b(?:jal|call|tail|j)\s+(?:\w+,\s*)?[0-9a-f]+\s+<([^>+]+)>|\bjalr\b.*#\s*[0-9a-f]+\s+<([^>+]+)>")
FUNC = re.compile(r"^[0-9a-f]+ <([^>]+)>:$")
INSN = re.compile(r"^\s*[0-9a-f]+:\s+(?:[0-9a-f]{4,8}\s+)?([a-z.]+)\s*(.*)$")


def find_objdump():
    for pat in ("~/.espressif/tools/tools/riscv32-esp-elf/*/riscv32-esp-elf/bin/riscv32-esp-elf-objdump",
                "~/.espressif/tools/riscv32-esp-elf/*/riscv32-esp-elf/bin/riscv32-esp-elf-objdump"):
        hits = sorted(glob.glob(os.path.expanduser(pat)))
        if hits:
            return hits[-1]
    return shutil.which("riscv32-esp-elf-objdump") or shutil.which("riscv64-unknown-elf-objdump")


def origins_from_map(map_path):
    """{function: origin} from the linker map's input sections (`.text.<fn>` lines)."""
    origin, pending = {}, None
    if not os.path.exists(map_path):
        return origin
    with open(map_path, encoding="utf-8", errors="replace") as fh:
        for line in fh:
            m = re.match(r"^\s*\.(?:text|iram1|flash\.text)\.(\S+)(?:\s+0x[0-9a-f]+\s+0x[0-9a-f]+\s+(\S+))?", line)
            if m:
                if m.group(2):
                    origin[m.group(1)] = classify(m.group(2))
                    pending = None
                else:
                    pending = m.group(1)
                continue
            if pending:
                m2 = re.match(r"^\s+0x[0-9a-f]+\s+0x[0-9a-f]+\s+(\S+)", line)
                if m2:
                    origin[pending] = classify(m2.group(1))
                pending = None
    return origin


def classify(obj):
    base = os.path.basename(obj)
    if "loft_app" in base:
        return "loft_app"          # split into program / runtime by name below
    m = re.search(r"\(([^)]+)\)$", base)
    member = m.group(1) if m else base
    lib = re.sub(r"\(.*", "", base)
    if lib.startswith("libmain") or "__idf_main" in obj:
        return member.replace(".obj", "").replace(".o", "")
    if lib.startswith("lib"):
        return lib.replace(".a", "")
    return member


def program_or_runtime(fn):
    """loft_app.c holds both: the program's functions are `n_*` / `t_*` (user and library
    loft), the runtime's are `loft_*` and the C helpers it carries."""
    return "program" if re.match(r"^(n|t)_", fn) else "loft runtime"


def census(elf, origin):
    objdump = find_objdump()
    if not objdump:
        sys.exit("width64_census: no RISC-V objdump (the ESP-IDF toolchain's riscv32-esp-elf-objdump)")
    out = subprocess.run([objdump, "-d", "--no-show-raw-insn", elf], capture_output=True, text=True).stdout
    funcs = defaultdict(lambda: {"double": 0, "i64": 0, "carry": 0, "insns": 0, "calls": defaultdict(int)})
    cur, prev = None, None
    for line in out.splitlines():
        m = FUNC.match(line)
        if m:
            cur, prev = m.group(1), None
            continue
        if cur is None:
            continue
        mi = INSN.match(line)
        if not mi:
            continue
        op, args = mi.group(1), mi.group(2)
        f = funcs[cur]
        f["insns"] += 1
        mc = CALL.search(line)
        if mc:
            callee = mc.group(1) or mc.group(2)
            if DOUBLE_HELPERS.match(callee):
                f["double"] += 1
                f["calls"][callee] += 1
            elif INT64_HELPERS.match(callee):
                f["i64"] += 1
                f["calls"][callee] += 1
        regs = [r.strip() for r in args.split(",")]
        if prev and op == "sltu" and len(regs) == 3:
            pop, pregs = prev
            # add rd,a,b ; sltu t,rd,a|b   — the carry out of a low-half add
            if pop in ("add", "c.add") and len(pregs) == 3 and regs[1] == pregs[0] and regs[2] in pregs[1:]:
                f["carry"] += 1
        if prev and op in ("sub",) and len(regs) == 3:
            pop, pregs = prev
            # sltu t,a,b ; sub rd,a,b      — the borrow of a low-half subtract
            if pop == "sltu" and len(pregs) == 3 and pregs[1:] == regs[1:]:
                f["carry"] += 1
        prev = (op, regs)
    rows = []
    for fn, f in funcs.items():
        if DOUBLE_HELPERS.match(fn) or INT64_HELPERS.match(fn):
            continue
        o = origin.get(fn, "?")
        if o == "loft_app":
            o = program_or_runtime(fn)
        rows.append({"function": fn, "origin": o, "double": f["double"], "i64": f["i64"],
                     "carry": f["carry"], "insns": f["insns"], "calls": dict(f["calls"])})
    return rows


def main():
    ap = argparse.ArgumentParser(description="Where a 32-bit image does 64-bit work.")
    ap.add_argument("project", help="an ESP-IDF project dir (with build/), or its build/ dir, or an .elf")
    ap.add_argument("--top", type=int, default=20)
    ap.add_argument("--origin", help="only this origin's functions (program, loft runtime, tiles, ...)")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    p = a.project
    if os.path.isdir(os.path.join(p, "build")):
        p = os.path.join(p, "build")
    elf = p if p.endswith(".elf") else (sorted(glob.glob(os.path.join(p, "*.elf"))) or [None])[0]
    if not elf:
        sys.exit(f"width64_census: no .elf in {p}")
    mp = os.path.splitext(elf)[0] + ".map"
    rows = census(elf, origins_from_map(mp))
    if a.origin:
        rows = [r for r in rows if r["origin"] == a.origin]
    hot = [r for r in rows if r["double"] or r["i64"] or r["carry"]]
    if a.json:
        print(json.dumps(hot, indent=1))
        return 0
    by = defaultdict(lambda: {"fns": 0, "double": 0, "i64": 0, "carry": 0})
    for r in hot:
        b = by[r["origin"]]
        b["fns"] += 1
        for k in ("double", "i64", "carry"):
            b[k] += r[k]
    tot = {k: sum(r[k] for r in hot) for k in ("double", "i64", "carry")}
    print(f"{os.path.basename(elf)}: {len(hot)} of {len(rows)} functions do 64-bit work — "
          f"{tot['double']} double-helper calls, {tot['i64']} 64-bit integer-helper calls, "
          f"{tot['carry']} inline 64-bit add/subtract carries")
    print(f"\n{'origin':<24}{'functions':>10}{'double':>9}{'i64':>7}{'carry':>8}")
    for o, b in sorted(by.items(), key=lambda kv: -(kv[1]["double"] * 4 + kv[1]["i64"] * 4 + kv[1]["carry"])):
        print(f"{o:<24}{b['fns']:>10}{b['double']:>9}{b['i64']:>7}{b['carry']:>8}")
    print(f"\nThe {a.top} functions with the most:")
    for r in sorted(hot, key=lambda r: -(r["double"] * 4 + r["i64"] * 4 + r["carry"]))[:a.top]:
        helpers = ", ".join(f"{k}x{v}" for k, v in sorted(r["calls"].items(), key=lambda kv: -kv[1])[:3])
        print(f"  {r['double']:>4} dbl {r['i64']:>4} i64 {r['carry']:>4} carry  {r['function'][:60]:<60} "
              f"{r['origin']}{'  (' + helpers + ')' if helpers else ''}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
