#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""@PLN155 phase 0 — how many emitted frees are licensed by the deps PROXY alone?

Runs `LOFT_OWN_ORACLE=census` over the corpus and sums the per-file reports.  The compiler
classifies every free it emits by the fact that licensed it (see `run_licence_census` in
`src/ownership_cfg.rs`); this collects the answer over every program the repo has.

The kill criterion the plan states: **if `proxy-alone` is a handful, @PLN155's phases 2-4 are
not worth their cost** and the plan closes with this number as its product, the census staying
as the guard that says when it changes.

Two readings that are NOT the same, and both are printed:

  * per PROGRAM — every free in the stdlib is counted once per file that loads it, which is
    what a running program actually contains;
  * per SITE — each `function:binding` pair once over the whole corpus, so the stdlib's own
    frees do not drown the user code that varies between files.

⚠ `scopes::check` runs once per SOURCE and each run walks every definition known so far, so a
file prints several reports and only the LAST covers the whole program.  This reads the last
block per file, which is what its `over N function(s)` header identifies.

Usage:
    scripts/licence_census.py                 # tests/scripts + tests/docs + examples
    scripts/licence_census.py --limit 200     # a sample, for a quick read
    scripts/licence_census.py --sites         # + the per-site table
    scripts/licence_census.py --control       # the two injection controls (both buckets must move)
"""
import argparse
import collections
import concurrent.futures
import os
import pathlib
import re
import subprocess
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
CORPUS = ["tests/scripts", "tests/docs", "examples"]
HEADER = re.compile(r"OWN-CENSUS over (\d+) function\(s\): (\d+) emitted free")
ROW = re.compile(r"^  ([a-z-]+)\s+(\d+)\s+[\d.]+%$")
EXAMPLE = re.compile(r"^      (\S+) \(v\d+\) dep=")
SPELL = re.compile(r"^  (Op\w+)\s+(\d+)$")
# Two controls, in the two directions a bucket can move, because ONE of them cannot reach the
# bucket the decision reads and the reason is structural:
#
#   ADD  `LOFT_OWN_INJECT_FREE_BORROWED=bview` forces a free of a binding the oracle calls
#        Borrowed.  It moves `oracle-disagrees` (5 -> 6) and the total (30 -> 31).
#   DROP `LOFT_OWN_INJECT_DROP_FREE=__ref_1` suppresses a free that IS emitted.  It moves
#        `delivery-buffer` (1 -> 0) and the total (30 -> 29).
#
# ⚠ That control named `proxy-alone` until 2026-09-08, and it FAILED the day the oracle
# learned to classify delivery buffers (@PLN155 phase 3a) — `n_exists:__ref_1` is a `__ref`
# buffer, so it moved bucket and the control's target went empty.  That is the control doing
# its job: it is pinned to a NAMED bucket precisely so a category changing underneath it is
# loud.  Re-point it, never relax it to "some bucket moved".
#
# The ADD control cannot move `proxy-alone`, and that is not a gap in the census: a
# proxy-alone binding is BY DEFINITION one the proxy already licenses, so its free is emitted
# rather than suppressed, and the over-free injector exists to force a SUPPRESSED free.  There
# is no suppressed proxy-alone free to force.  The drop direction is the one that reaches it.
CONTROL = "doc/claude/plans/94-cfg-ownership-dataflow/probes/08-overfree-positive-control.loft"
CONTROL_VAR = "bview"
CONTROL_DROP_VAR = "__ref_1"
# The bucket that var occupies — `delivery-buffer` since the oracle learned about buffers.
CONTROL_DROP_BUCKET = "delivery-buffer"


def loft_bin():
    for p in ("target/debug/loft", "target/release/loft"):
        if (ROOT / p).exists():
            return str(ROOT / p)
    sys.exit("no loft binary — `cargo build --bin loft` first")


def census(path, env_extra=None):
    """The LAST census block of one file: (functions, {bucket: n}, {op: n}, [sites])."""
    env = dict(os.environ, LOFT_OWN_ORACLE="census", LOFT_NO_CACHE="1", LOFT_TIMEOUT="120")
    env.update(env_extra or {})
    try:
        out = subprocess.run(
            [loft_bin(), "--interpret", str(path)],
            capture_output=True, text=True, env=env, timeout=180, cwd=ROOT,
        ).stderr
    except subprocess.TimeoutExpired:
        return None
    blocks, cur, bucket = [], None, None
    for line in out.splitlines():
        m = HEADER.search(line)
        if m:
            cur, bucket = [int(m.group(1)), {}, {}, []], None
            blocks.append(cur)
            continue
        if cur is None:
            continue
        m = ROW.match(line)
        if m:
            # An example line belongs to the bucket ROW above it, and the compiler prints them
            # in that order.  Attaching them to a position instead of to the current bucket is
            # how the first version counted 0 sites while printing them.
            bucket = m.group(1)
            cur[1][bucket] = int(m.group(2))
            continue
        m = SPELL.match(line)
        if m:
            cur[2][m.group(1)] = int(m.group(2))
            bucket = None
            continue
        m = EXAMPLE.match(line)
        if m and bucket:
            cur[3].append((bucket, m.group(1)))
    return blocks[-1] if blocks else None


def corpus_files(limit):
    files = []
    for d in CORPUS:
        files += sorted((ROOT / d).rglob("*.loft"))
    files = [f for f in files if f.is_file()]
    return files[:limit] if limit else files


def run_control():
    """Prove the buckets MOVE. A census whose categories cannot be moved measures nothing."""
    base = census(ROOT / CONTROL)
    add = census(ROOT / CONTROL, {"LOFT_OWN_INJECT_FREE_BORROWED": CONTROL_VAR})
    drop = census(ROOT / CONTROL, {"LOFT_OWN_INJECT_DROP_FREE": CONTROL_DROP_VAR})
    if not base or not add or not drop:
        sys.exit(f"control did not run — is {CONTROL} still there?")
    print(f"CONTROL {CONTROL}")
    rows = [
        ("oracle-disagrees", f"+free of a borrowed binding ({CONTROL_VAR})",
         base[1].get("oracle-disagrees", 0), add[1].get("oracle-disagrees", 0), "up"),
        (CONTROL_DROP_BUCKET, f"-free of a proxy-licensed binding ({CONTROL_DROP_VAR})",
         base[1].get(CONTROL_DROP_BUCKET, 0), drop[1].get(CONTROL_DROP_BUCKET, 0), "down"),
    ]
    ok = True
    for bucket, what, before, after, way in rows:
        moved = after > before if way == "up" else after < before
        ok &= moved
        print(f"  {bucket:<18} {before} -> {after}   {what}   "
              + ("moves" if moved else "DID NOT MOVE"))
    print("  " + ("PASS — both buckets move" if ok
                  else "FAIL — a bucket the census reports cannot be moved"))
    print("  (the ADD control cannot reach `proxy-alone`: such a binding is one the proxy")
    print("   already licenses, so its free is emitted rather than suppressed.)")
    return 0 if ok else 1


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--limit", type=int, help="only the first N corpus files")
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--sites", action="store_true", help="also print the per-site table")
    ap.add_argument("--control", action="store_true", help="run the injected-free control only")
    a = ap.parse_args()
    if a.control:
        sys.exit(run_control())

    files = corpus_files(a.limit)
    if not files:
        sys.exit("empty corpus — run this from the repo")
    per_program = collections.Counter()
    spellings = collections.Counter()
    sites = collections.defaultdict(set)
    ok, failed = 0, 0
    with concurrent.futures.ThreadPoolExecutor(max_workers=a.jobs) as ex:
        for path, block in zip(files, ex.map(census, files)):
            if block is None:
                failed += 1
                continue
            ok += 1
            per_program.update(block[1])
            spellings.update(block[2])
            for bucket, site in block[3]:
                if site:
                    sites[bucket].add(site)

    total = sum(per_program.values())
    print(f"\n=== @PLN155 phase 0 — the licence census ===\n")
    print(f"  {ok} corpus file(s) censused" + (f", {failed} did not run" if failed else ""))
    print(f"\n  per PROGRAM ({total} emitted frees; the stdlib's are counted once per file)")
    print(f"  {'licensing fact':<20}{'frees':>8}{'share':>9}")
    for bucket, n in per_program.most_common():
        print(f"  {bucket:<20}{n:8}{100 * n / total:8.1f}%")
    # Only `proxy-alone` is listed in full by the compiler; the other buckets emit six
    # examples per file, so a distinct-site count over them would be a count of the SAMPLE.
    # Printing one number that means two things is how a census comes to be quoted wrongly.
    print(f"\n  distinct `function:binding` sites in the category the decision reads")
    print(f"  proxy-alone          {len(sites.get('proxy-alone', ())):8}")
    # WHAT is in the bucket, not only how much.  A count of 2 000 that is one compiler temp
    # repeated says something different from the same count spread over user locals, and the
    # plan's "a handful" cannot be judged without knowing which.  Grouped by the binding's
    # NAME family, with the numeric suffix folded, because that is what distinguishes a
    # generated temp (`__ref_1`, `__ncc_3`) from a name the author wrote.
    fam = collections.Counter()
    for site in sites.get("proxy-alone", ()):
        binding = site.rsplit(":", 1)[-1]
        fam[re.sub(r"_?\d+$", "_N", binding) if re.search(r"\d+$", binding)
            else ("<author-named>" if not binding.startswith("__") else binding)] += 1
    print(f"\n  proxy-alone by binding family (a generated temp vs a name the author wrote)")
    for name, n in fam.most_common(12):
        print(f"  {name:<24}{n:8}")
    authored = sum(n for k, n in fam.items() if not k.startswith("__"))
    print(f"  {'':<24}{'':8}  {authored} of {len(sites.get('proxy-alone', ()))} "
          f"are author-named bindings, not compiler temps")
    print(f"\n  by free spelling")
    for op, n in spellings.most_common():
        print(f"  {op:<24}{n:8}")
    alone = per_program.get("proxy-alone", 0)
    print(f"\n  proxy-alone is {alone} of {total} emitted frees "
          f"({100 * alone / max(1, total):.1f}%), at "
          f"{len(sites.get('proxy-alone', ())):,} distinct site(s).")
    print("  The plan's kill criterion reads this: a handful means @PLN155 phases 2-4 are not")
    print("  worth their cost and the plan closes with this number as its product.")
    if a.sites:
        for bucket, v in sorted(sites.items()):  # `proxy-alone` is complete; the rest sampled
            print(f"\n  -- {bucket} --")
            # `proxy-alone` in full — it is the set the kill criterion is a claim about, and
            # a truncated list cannot support "a handful" either way.
            cap = len(v) if bucket == "proxy-alone" else 40
            for site in sorted(v)[:cap]:
                print(f"    {site}")
            if len(v) > cap:
                print(f"    … and {len(v) - cap} more")
    print()


if __name__ == "__main__":
    main()
