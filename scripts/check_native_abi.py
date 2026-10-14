#!/usr/bin/env python3
"""Sweep loft LIBRARY REPOS for `#native` declaration / Rust export ABI mismatches.

    scripts/check_native_abi.py ../loft-libs-net ../loft-libs-core ...

The in-repo guard `tests/native_abi_contract.rs` enforces this rule for loft's own
fixture crates; it cannot see the sibling library repos, which is what this is for.
Same rule, same reasoning — keep the two in step if either changes.

loft emits the extern from the DECLARATION (`integer` -> `i64`, doc/claude/PACKAGES.md)
and calls it directly, so a narrower Rust signature reads undefined register bits — only
on --native, and only for values the narrow type cannot carry.  RETURN mismatches are the
dangerous half; PARAM mismatches are off-contract but usually benign on x86-64.
"""

import re, sys, pathlib, json

LOFT_TO_RUST = {"integer": {"i64"}, "float": {"f64"}, "single": {"f32"},
                "boolean": {"bool", "u8"}, "character": {"u32", "char", "i32"}}

def loft_decls(root):
    """symbol -> (params[loft types], return loft type, file:line)"""
    out = {}
    for f in (p for p in pathlib.Path(root).rglob("*.loft") if p.is_file()):
        sp = str(f)
        if "/.loft/" in sp or "/target/" in sp or not f.is_file():
            continue
        lines = f.read_text(errors="replace").splitlines()
        for i, ln in enumerate(lines):
            if not ln.strip().startswith("#native"):
                continue
            # the declaration is the nearest preceding `fn ...;`
            for j in range(i - 1, max(-1, i - 6), -1):
                m = re.match(r'\s*(?:pub\s+)?fn\s+(\w+)\s*\((.*?)\)\s*(?:->\s*([\w<>?]+))?\s*;', lines[j])
                if not m:
                    continue
                lname, args, ret = m.group(1), m.group(2), m.group(3)
                mm = re.match(r'#native\s+"([^"]+)"', ln.strip())
                sym = mm.group(1) if mm else "n_" + lname
                ptypes = []
                for a in args.split(","):
                    a = a.strip()
                    if not a:
                        continue
                    ptypes.append(a.split(":", 1)[1].strip() if ":" in a else "?")
                out[sym] = (ptypes, ret, f"{f}:{j+1}")
                break
    return out

def rust_exports(root):
    """symbol -> (params[(name,rust type)], rust return, file:line)"""
    out = {}
    live = set()
    for libf in pathlib.Path(root).rglob("src/lib.rs"):
        if "/target/" in str(libf):
            continue
        live.add(libf)
        txt = libf.read_text(errors="replace")
        for m in re.finditer(r'^\s*(?:pub\s+)?mod\s+(\w+)\s*;', txt, re.M):
            cand = libf.parent / f"{m.group(1)}.rs"
            if cand.exists():
                live.add(cand)
    for f in sorted(live):
        sp = str(f)
        if "/target/" in sp or not f.is_file():
            continue
        txt = f.read_text(errors="replace")
        for m in re.finditer(r'pub\s+(?:unsafe\s+)?extern\s+"C"\s+fn\s+(\w+)\s*\(([^)]*)\)\s*(?:->\s*([\w:<>*\s]+?))?\s*\{', txt, re.S):
            name, args, ret = m.group(1), m.group(2), (m.group(3) or "").strip()
            params = []
            for a in args.split(","):
                a = a.strip()
                if not a:
                    continue
                if ":" in a:
                    n, t = a.split(":", 1)
                    params.append((n.strip(), t.strip()))
            line = txt[:m.start()].count("\n") + 1
            out[name] = (params, ret, f"{f}:{line}")
    return out

def audit(repo):
    decls, exports = loft_decls(repo), rust_exports(repo)
    findings = []
    for sym, (ptypes, lret, lloc) in sorted(decls.items()):
        if sym not in exports:
            continue
        rparams, rret, rloc = exports[sym]
        # RETURN
        if lret and lret in LOFT_TO_RUST and rret and rret not in LOFT_TO_RUST[lret]:
            findings.append(("RETURN", sym, f"loft `{lret}` wants {sorted(LOFT_TO_RUST[lret])}, rust returns `{rret}`", rloc))
        # PARAMS — align only when the counts match (text/vector expand to pairs)
        if len(ptypes) == len(rparams):
            for lt, (rn, rt) in zip(ptypes, rparams):
                if lt in LOFT_TO_RUST and rt not in LOFT_TO_RUST[lt]:
                    findings.append(("PARAM", sym, f"arg `{rn}`: loft `{lt}` wants {sorted(LOFT_TO_RUST[lt])}, rust has `{rt}`", rloc))
    return findings

for repo in sys.argv[1:]:
    fs = audit(repo)
    name = pathlib.Path(repo).name
    if not fs:
        print(f"\n=== {name}: CLEAN ===")
        continue
    r = sum(1 for f in fs if f[0] == "RETURN")
    print(f"\n=== {name}: {len(fs)} mismatches ({r} RETURN, {len(fs)-r} PARAM) ===")
    for sev, sym, msg, loc in sorted(fs, key=lambda x: (x[0] != "RETURN", x[1])):
        print(f"  [{sev:6s}] {sym:30s} {msg}\n{'':11s}{loc}")
