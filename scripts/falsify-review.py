#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Per-release aid: which falsification receipts can still be re-validated, and quickly.

Whether a recorded patch actually reintroduces THE defect its guard catches cannot be
checked by CI — it costs an apply, a build and a run per guard, and the answer is a
judgement about channels rather than a pass/fail.  So it is a recurring READ, once per
release, and what decides whether that read takes an afternoon or a week is how well each
guard documents itself.

Measured 2026-09-09 while retrofitting 21 receipts: the guards whose receipt named its
CHANNEL, its INSTRUMENT and a concrete WITNESS were validated in seconds by comparing one
value.  The ones that recorded only prose forced a rebuild and a hand judgement, and three
leak guards read as outright contradictions until the instrument they never mentioned was
armed.  One patch was rejected because the receipt said which channel must NOT move; had it
not said so, a receipt that moves the wrong channel would have been recorded as proof.

So a receipt is scored on four fields, each earned from a failure that cost real time:

  CHANNEL   which channel carries the defect (exit / assert / leak / panic / expectations).
            Without it a good patch and a bad one look the same.
  ARMED     which instrument the measurement needs (`LOFT_STRICT_STORES=1`), or that it
            needs none.  An unarmed run of a leak guard gives a DIFFERENT answer, not a
            weaker one.
  WITNESS   the concrete observation on the control — a value, a count, a leaked shape
            (`answers 99 where the file says 88`, `St1295x42`).  This is what makes the
            re-read quick: you compare one thing, not a whole run.
  HOLDS     what must NOT move ("every assertion passes on both trees").  This is the field
            that rejects a bad patch.

The field tests are heuristics over prose, so a miss is a prompt to LOOK, never a verdict.
This reports; it does not gate, and it exits 0 even when everything is thin.

  scripts/falsify-review.py                  # the worklist and the summary
  scripts/falsify-review.py --since <ref>    # + controls that went unreachable since <ref>
  scripts/falsify-review.py --all            # list every guard, not just the worklist
"""
import re, subprocess, sys, os
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
os.chdir(ROOT)

def sh(*a):
    return subprocess.run(a, capture_output=True, text=True).stdout.strip()

def public_commits():
    refs = sh("git", "for-each-ref", "--format=%(refname)",
              "refs/remotes/origin", "refs/pull").splitlines()
    if not refs:
        return set()
    p = subprocess.run(["git", "rev-list", "--stdin"], input="\n".join(refs),
                       capture_output=True, text=True)
    return set(p.stdout.split())

# A field is satisfied by its explicit LABEL — `HOLDS:` and friends, the canonical form a new
# receipt should use — or by the prose the corpus already carries, so that a well-written older
# receipt is not reported as thin merely for lacking a keyword.  The label alternative is not a
# convenience: without it the standard is unwritable, because an author has to guess which words
# the heuristic likes.  Measured — 12 receipts documented in full read as thin until the labels
# were recognised.
FIELD = {
    "CHANNEL": re.compile(r"^//\s*CHANNEL:|exit \d|assertion failur|leaked|leak `|panicked|"
                          r"free refusal|expectations|INERT|FREE channel|LEAK channel|"
                          r"ASSERT channel", re.I | re.M),
    "ARMED":   re.compile(r"^//\s*ARMED:|LOFT_[A-Z_]+=|instrument armed|WITH THE INSTRUMENT|"
                          r"no instrument", re.I | re.M),
    "WITNESS": re.compile(r"^//\s*WITNESS:|`[^`]*\d[^`]*`|×\d|answered|answers |reads |"
                          r"against the \d|-> \d", re.I | re.M),
    "HOLDS":   re.compile(r"^//\s*HOLDS:|unmoved|unchanged|passes on both|not exit|not asserts|"
                          r"only one|stays at|already correct|must not|B\.\.F", re.I | re.M),
}

def receipt_of(path):
    """The receipt block: the @falsified-at line and the // lines under it."""
    lines = path.read_text(errors="replace").split("\n")
    i = next((k for k, l in enumerate(lines) if "@falsified-at:" in l), None)
    if i is None:
        return None
    j = i
    while j + 1 < len(lines) and lines[j + 1].startswith("//"):
        j += 1
    return "\n".join(lines[i:j + 1])

def main():
    args = sys.argv[1:]
    since = None
    show_all = "--all" in args
    check = "--check" in args
    if "--since" in args:
        since = args[args.index("--since") + 1]

    pub = public_commits()
    rows = []
    for f in sorted(Path("tests/scripts").glob("*.loft")):
        if not f.is_file():
            continue          # `.loft/` is a cache DIRECTORY and matches this glob
        block = receipt_of(f)
        if block is None:
            continue
        m = re.search(r"@falsified-at:\s*([0-9a-f]{7,})", block)
        if not m:
            continue                      # `none — <reason>`: a stated opt-out, not a receipt
        sha = m.group(1)
        full = sh("git", "rev-parse", "--verify", "--quiet", sha + "^{commit}")
        reachable = bool(full) and full in pub
        patch = re.search(r"@falsified-by:\s*(\S+)", block)
        runnable = None
        if patch:
            p = patch.group(1)
            runnable = Path(p).exists() and subprocess.run(
                ["git", "apply", "--check", p], capture_output=True).returncode == 0
        # ARMED is only a question where the instrument CHANGES the answer.  A leak or
        # free-refusal guard scored unarmed reports a different channel, not a weaker one;
        # an exit/assert guard needs nothing and its silence is correct, so demanding the
        # field everywhere floods the worklist and buries the cases that matter.
        # Decide leak-class from everything EXCEPT the HOLDS lines.  HOLDS says what does NOT
        # move, so "leak and panic are equal on both trees" is a statement that the guard is not
        # leak-class — read naively it says the opposite, and then documenting a value guard
        # properly makes the checker demand an instrument field it has no use for.
        # A labelled field owns its CONTINUATION lines too — the prose wraps, and only the
        # first line carries the label — so the whole HOLDS section is dropped, not its head.
        # Dropping one line leaves "a run scored on leaks would learn nothing" behind, which
        # reads as leak-class and re-demands the instrument field.
        LABEL = re.compile(r"^//\s*(CHANNEL|ARMED|WITNESS|HOLDS):", re.I)
        keep, in_holds = [], False
        for l in block.split("\n"):
            m = LABEL.match(l)
            if m:
                in_holds = m.group(1).upper() == "HOLDS"
            elif in_holds and not l.startswith("//"):
                in_holds = False
            if not in_holds:
                keep.append(l)
        channel_text = "\n".join(keep)
        needs_armed = re.search(r"leak|free refusal|BUG \(#306\)|stack-store", channel_text, re.I)
        missing = [k for k, rx in FIELD.items()
                   if not rx.search(block) and (k != "ARMED" or needs_armed)]
        rows.append(dict(f=f, sha=sha, reachable=reachable, patch=bool(patch),
                         runnable=runnable, missing=missing))

    if check:
        # One line per under-documented receipt, sorted, for `tests/falsified_docs.baseline`
        # and the gate that reads it.  The rule lives HERE only — a second copy in Rust would
        # drift from this one, and then the gate and the review would disagree about what a
        # receipt owes.
        for r in sorted(rows, key=lambda r: r["f"].name):
            if r["missing"]:
                print(f"{r['f'].name}\t{','.join(sorted(r['missing']))}")
        return 0

    total = len(rows)
    orphan = [r for r in rows if not r["reachable"]]
    withpatch = [r for r in rows if r["patch"]]
    stale = [r for r in withpatch if r["runnable"] is False]

    print(f"falsification receipts: {total}")
    print(f"  control still publicly reachable : {total - len(orphan)}")
    print(f"  control unreachable              : {len(orphan)}")
    print(f"    of those, carrying a patch     : {len([r for r in orphan if r['patch']])}")
    print(f"  patch receipts total             : {len(withpatch)}"
          f"   (still applying: {len([r for r in withpatch if r['runnable']])})")
    if stale:
        print(f"\n⚠ {len(stale)} patch receipt(s) no longer apply — re-derive or downgrade to a marker:")
        for r in stale:
            print(f"    {r['f'].name}")

    # A receipt that does not say how to score it is a defective guard whatever its control
    # does — the guard claims to catch something and does not record how anyone would check
    # that claim again.  So the worklist is every under-documented receipt, not only the
    # unreachable ones; unreachable FIRST, because those cannot be re-derived from the tree
    # and are the ones a validation stalls on for good.
    work = sorted((r for r in rows if r["missing"]),
                  key=lambda r: (r["reachable"], -len(r["missing"]), r["f"].name))
    thin_orphan = len([r for r in work if not r["reachable"]])
    print(f"\nworklist — the receipt does not say how to score it ({len(work)} of {total})")
    print(f"  {thin_orphan} of them also have an unreachable control, listed first.")
    print(f"  fields: CHANNEL ARMED WITNESS HOLDS\n")
    for r in (work if show_all else work[:25]):
        mark = " " if r["reachable"] else "!"
        print(f" {mark}missing {','.join(r['missing']):<28} {r['f'].name}")
    if not show_all and len(work) > 25:
        print(f"  … and {len(work) - 25} more (--all)")

    if since:
        prev = sh("git", "rev-list", since + "..HEAD", "--count")
        newly = [r for r in orphan
                 if sh("git", "merge-base", "--is-ancestor", r["sha"], since) == ""]
        print(f"\nsince {since} ({prev} commits): {len(orphan)} controls unreachable now.")
        print("  Compare with the same line from last cycle — the DELTA is the inflow rate,")
        print("  which is what says whether recording receipts at falsification is taking.")

    print("\nThis is a report. Validating that a patch reintroduces THE defect is a human read:")
    print("  scripts/falsify.sh <guard> --patch <patch>   (arm the instrument the receipt names)")
    return 0

if __name__ == "__main__":
    sys.exit(main())
