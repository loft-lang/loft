#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The lexer gate (@PLN91 stage 1): does the loft front end's lexer read every file as loft's own?

For each corpus file it runs the Rust lexer as the oracle (`LOFT_TRACE_TOKENS`, src/lexer.rs
`trace_token`: every token the parser scanned, once per place) and the loft lexer
(`front/tokens.loft`, built through the C backend, or `--interpret`), and compares them per file
by place (line, column).  Only the places the oracle has are compared: it records what the parser
scanned, so a file the parser stopped in early, a token it skipped in a look-ahead it never made,
and a token the Rust lexer queues (the `.`/`..` after a number) or ends in an error are absent
there.  Those are the EXTRA tokens, reported apart.  A file AGREES when every oracle token is
present at its place with the same kind and payload.

  selfhost/front/tokens_gate.py [--part NAME]… [--interpret] [--jobs N] [--runner CMD] [--show N]
"""
import argparse, collections, concurrent.futures as cf, glob, hashlib, os, shutil, subprocess, sys, tempfile

PARTS = ["default", "scripts", "docs", "suite", "lib"]


def corpus(root, suite):
    return {
        "default": sorted(glob.glob(os.path.join(root, "default", "*.loft"))),
        "scripts": sorted(glob.glob(os.path.join(root, "tests", "scripts", "*.loft"))),
        "docs": sorted(glob.glob(os.path.join(root, "tests", "docs", "*.loft"))),
        "suite": sorted(glob.glob(os.path.join(suite, "rules", "**", "*.loft"), recursive=True)),
        "lib": sorted(glob.glob(os.path.join(root, "lib", "**", "*.loft"), recursive=True)),
    }


def parse(lines, keep):
    """Trace lines into {file: [(line, col, item)]}, in order, for the files in `keep`."""
    out = collections.defaultdict(list)
    for ln in lines:
        parts = ln.rstrip("\n").split("\t", 3)
        if len(parts) != 4 or parts[0] not in keep:
            continue
        try:
            out[parts[0]].append((int(parts[1]), int(parts[2]), parts[3]))
        except ValueError:
            continue
    return out


def oracle_one(loft, path, tmp, env, i):
    tr = os.path.join(tmp, f"o{i}.tr")
    img = os.path.join(tmp, f"o{i}.img")
    e = dict(env, LOFT_TRACE_TOKENS=tr)
    try:
        subprocess.run([loft, "--emit-image", img, path], env=e, capture_output=True, timeout=180)
    except subprocess.TimeoutExpired:
        pass
    lines = []
    if os.path.exists(tr):
        with open(tr, encoding="utf-8", errors="replace") as f:
            lines = f.readlines()
        os.remove(tr)
    if os.path.exists(img):
        os.remove(img)
    return lines


def places(toks):
    """Mine as {place: item}: the first token at a place, except a `.`/`..` the number lexer
    queued beside another token at the same place (the Rust never traces a queued token).  An
    item is the token and, after a tab, the scan cursor after it."""
    count = collections.Counter((l, c) for l, c, _ in toks)
    out = {}
    for l, c, item in toks:
        if count[(l, c)] > 1 and item.split("\t")[0] in ("tok .", "tok .."):
            continue
        out.setdefault((l, c), item)
    return out


def has_main(toks):
    return any(a[2] == "tok fn" and b[2] == "id main" for a, b in zip(toks, toks[1:]))


def compare(orc, mine):
    o = {}
    for l, c, item in orc:
        o.setdefault((l, c), item)
    m = places(mine)
    missing, wrong = [], []
    for p in sorted(o):
        if p not in m:
            missing.append(p)
        elif m[p] != o[p]:
            wrong.append(p)
    extra = [p for p in m if p not in o]
    first = None
    bad = sorted(missing + wrong)
    if bad:
        p = bad[0]
        first = (p, o[p], m.get(p, "<missing>"))
    return first, len(missing), len(wrong), len(extra)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--part", action="append", choices=PARTS, help="corpus parts (default: all)")
    ap.add_argument("--loft", default="target/release/loft")
    ap.add_argument("--interpret", action="store_true", help="run tokens.loft on --interpret")
    ap.add_argument("--runner", help="a lexer command that takes the files (`@list`) instead")
    ap.add_argument("--jobs", type=int, default=os.cpu_count() or 4)
    ap.add_argument("--show", type=int, default=40, help="failing files to list")
    ap.add_argument("--cache", help="keep each file's oracle trace here, reused while the file "
                    "and the loft binary are unchanged")
    o = ap.parse_args()
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    loft = o.loft if os.path.isabs(o.loft) else os.path.join(root, o.loft)
    suite = os.environ.get("LOFT_SUITE_DIR", os.path.join(os.path.dirname(root), "loft-suite-91"))
    env = dict(os.environ, LOFT_NO_CACHE="1", LOFT_TIMEOUT="120")
    parts = o.part or PARTS
    files = corpus(root, suite)
    todo = [(part, os.path.abspath(f)) for part in parts for f in files[part]]
    keep = {f for _, f in todo}
    tmp = tempfile.mkdtemp(prefix="loft-tokens-", dir=os.environ.get("TMPDIR"))
    try:
        # The loft lexer, over every file in one run.
        listing = os.path.join(tmp, "files.txt")
        with open(listing, "w", encoding="utf-8") as f:
            f.write("\n".join(sorted(keep)) + "\n")
        tokens_src = os.path.join(root, "selfhost", "front", "tokens.loft")
        if o.runner:
            cmd = o.runner.split() + ["@" + listing]
        elif o.interpret:
            cmd = [loft, "--interpret", tokens_src, "@" + listing]
        else:
            out = os.path.join(tmp, "runner")
            os.makedirs(out)
            b = subprocess.run([loft, "--native-c", "--ship", "--out", out, tokens_src],
                               env=env, capture_output=True, text=True, timeout=900)
            exe = os.path.join(out, "tokens")
            if not os.path.exists(exe):
                sys.exit(f"tokens.loft did not build:\n{b.stderr[-3000:]}")
            cmd = [exe, "@" + listing]
        r = subprocess.run(cmd, env=dict(env, LOFT_TIMEOUT="1200"), capture_output=True,
                           text=True, errors="replace", timeout=1800)
        if r.returncode != 0:
            print(f"the loft lexer exited {r.returncode}:\n{r.stderr[-2000:]}", file=sys.stderr)
        mine = parse(r.stdout.splitlines(), keep)
        # The oracle, a run per file (the standard library comes with every run).
        oracle = collections.defaultdict(list)
        want = [f for _, f in todo if "/default/" not in f]
        if any("/default/" in f for f in keep):
            want.append(None)
        def cached(f, i):
            src = f or os.path.join(root, "default", "01_code.loft")
            key = None
            if o.cache:
                st = [os.stat(loft).st_mtime_ns] + [os.stat(x).st_mtime_ns for x in
                      ([src] if f else glob.glob(os.path.join(root, "default", "*.loft")))]
                key = os.path.join(o.cache, hashlib.sha1(f"{src}{st}".encode()).hexdigest())
                if os.path.exists(key):
                    with open(key, encoding="utf-8") as fh:
                        return fh.readlines()
            lines = oracle_one(loft, src, tmp, env, i)
            if key:
                os.makedirs(o.cache, exist_ok=True)
                with open(key, "w", encoding="utf-8") as fh:
                    fh.writelines(ln for ln in lines if ln.split("\t", 1)[0] in keep)
            return lines
        with cf.ThreadPoolExecutor(o.jobs) as ex:
            futs = {ex.submit(cached, f, i): f for i, f in enumerate(want)}
            for fut in cf.as_completed(futs):
                f = futs[fut]
                got = parse(fut.result(), keep)
                for name, toks in got.items():
                    if name == f or (f is None and "/default/" in name):
                        oracle[name] = toks
        # Compare.
        per = collections.defaultdict(lambda: [0, 0])
        fails, extras, empty, scripts = [], 0, 0, []
        for part, f in todo:
            per[part][1] += 1
            if not oracle.get(f):
                empty += 1
            # A file of loose statements is a SCRIPT (src/script.rs, @PLN13): the driver parses
            # its desugaring to one `fn main` under the file's name, and the oracle traces that
            # text, at its own places, instead of the file.
            if has_main(oracle.get(f, [])) and not has_main(mine.get(f, [])):
                scripts.append(f)
                continue
            first, nmiss, nwrong, nextra = compare(oracle.get(f, []), mine.get(f, []))
            extras += nextra
            if first is None:
                per[part][0] += 1
            else:
                fails.append((part, f, first, nmiss, nwrong))
        total = sum(v[0] for v in per.values()), sum(v[1] for v in per.values())
        print(f"{total[0]} of {total[1]} files agree"
              + (f"; {len(scripts)} scripts not comparable" if scripts else ""))
        for part in parts:
            a, n = per[part]
            print(f"  {part:8} {a} of {n}")
        print(f"extra tokens (places the oracle does not have): {extras}; files the oracle traced nothing for: {empty}")
        for part, f, (p, want_item, got), nmiss, nwrong in fails[:o.show]:
            rel = os.path.relpath(f, os.path.dirname(root))
            print(f"{rel}:{p[0]}:{p[1]}  oracle `{want_item}`  loft `{got}`  ({nmiss} missing, {nwrong} differ)")
        for f in scripts:
            print(f"{os.path.relpath(f, os.path.dirname(root))}: a script — the oracle traced its `fn main` desugaring")
        if len(fails) > o.show:
            print(f"… and {len(fails) - o.show} more")
        sys.exit(0 if not fails else 1)
    finally:
        shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    main()
