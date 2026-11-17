#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""Work through every formal rule's suite cells, one rule at a time, towards a small set that
still covers the rule totally — and record which rules are done.

    scripts/suite_curate.py status            # how far: rules done, per state and chapter
    scripts/suite_curate.py next [N]          # the next N rules in the fixed order (default 3)
    scripts/suite_curate.py plan <rule>       # one rule's evidence and the proposed core
    scripts/suite_curate.py related <rule> <regex>   # every test home's programs matching the rule's
                                              # construct, and the code sites that enforce it
    scripts/suite_curate.py merge <target> <intro> <cell>…   # one clause's cells into one, every
                                              # case and receipt kept (gate E verdicts must agree)
    scripts/suite_curate.py verify <cell>…       # interpreter + C, leak-checked and strict + poison
    scripts/suite_curate.py vacuity <cell>…      # break one expectation per test: exactly it fails?
    scripts/suite_curate.py switches <cell>…     # which LOFT_NO_* cure switches move each cell
    scripts/suite_curate.py land <rule> <chapter/dir> <new> <msg-file> <old>…   # retire merged
                                              # cells: refs, features, suite commit, gate E, pin
    scripts/suite_curate.py done <rule> [note]   # record the rule as curated (after the commit)
    scripts/suite_curate.py init              # (re)write the ledger's rows for every defined rule

THE LEDGER is `index/suite_curation.tsv`: one row per defined rule (`rule_tags.py list`, all of
them — not only the cited ones), with its state, the cell count when it was done, and a note.
States: `todo`; `curated` (the rule's cells are its core, each one needed); `needs-cell` (no cell
names the rule: it goes to the rule-led walk's queue, `rule_tags.py worklist`).

WHAT "COVERS THE RULE TOTALLY" MEANS HERE — the features a rule's cells reach today, which the
core must keep, every one:
  * each composition-axis value the cells reach, over the axes the rule's chapter quantifies
    over (`rule_axes.tsv`, measured by `matrix_axes.census`, as `rule_depth.py` reads them);
  * each defect a cell names (its file name's issue number, a `loft#N` in its header): a past
    defect stays guarded by some cell.
`plan` picks the core greedily — the cell adding the most features not yet held, ties to the
shorter file — and lists every other cell with the route that would retire it:
  * `patch` — the cell has a `@falsified-by` patch receipt: re-score the core against that patch
    (`scripts/falsify.sh <core cell> --patch <patch>`); a core cell that reads `falsified`
    proves the defect is still caught, and the cell may go;
  * `fold`  — no patch receipt: its cases are not proven covered, so they MOVE into a core cell
    (its `test_*` functions), never vanish;
  * `unit`  — a Rust test: the home for what loft cannot observe (a heap layout, an image's
    bytes); its features are not measured, so it is weighed by reading it and by breaking the
    cure it guards (a mutation run) — never folded into a cell;
  * `keep`  — it reaches a feature no core cell does (then it IS core; listed for completeness).
A rule is `done` only after the suite passes on both backends with the curated cells and every
`patch` route was re-scored; the note says what was retired or folded.

THE ORDER is fixed — chapter, then rule, alphabetically — so a pass over the 642 rules is
complete by construction and its position is one row.  A rule with one cell or none is quick
(confirm, or mark `needs-cell`); the effort sits in the ~60 rules with eight cells or more.
"""
import collections
import datetime
import os
import re
import shutil
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(ROOT, "scripts"))
import matrix_axes  # noqa: E402
import rule_depth  # noqa: E402
import rule_tags  # noqa: E402

LEDGER = os.path.join(ROOT, "index", "suite_curation.tsv")
HEADER = "# rule\tchapter\tstate\tcells\tdate\tnote\n"
ISSUE_STEM = re.compile(r"^(\d{3,4})-")
ISSUE = re.compile(r"loft#(\d+)")
PATCH = re.compile(r"@falsified-by:\s*tests/falsified/(\S+\.patch)")


def defined():
    """Every defined rule and its chapter, `rule_tags.py list`'s set."""
    return {t: rule_depth.chapter_of(files) for t, files in rule_tags.defined_rules().items()}


def read_ledger():
    rows = {}
    if os.path.exists(LEDGER):
        for line in open(LEDGER, encoding="utf-8"):
            if line.startswith("#") or not line.strip():
                continue
            f = line.rstrip("\n").split("\t")
            f += [""] * (6 - len(f))
            rows[f[0]] = f
    return rows


def write_ledger(rows):
    with open(LEDGER, "w", encoding="utf-8") as out:
        out.write("# The suite-curation pass over every formal rule (scripts/suite_curate.py; "
                  "TESTING.md § Curating the suite).\n")
        out.write(HEADER)
        for key in order(rows):
            out.write("\t".join(rows[key]) + "\n")


def order(rows):
    return sorted(rows, key=lambda t: (rows[t][1], t))


def cells_of(tag):
    """The cells (and other guard files) that cite `@FR-<tag>`."""
    guard = rule_tags.guard_citations()
    return sorted({f for f, _ in guard.get(tag, [])})


def features(path, axes):
    """What one cell contributes to its rule's coverage: axis values and defects named."""
    full = rule_depth.abs_path(path)
    text = rule_depth.read(full)
    out = set()
    if path.endswith(rule_depth.LOFT_CELL):
        try:
            census = matrix_axes.census(full)
        except OSError:
            census = {}
        for a in axes:
            out |= {f"{a}={v}" for v in census.get(a, set())}
    m = ISSUE_STEM.match(os.path.basename(path))
    if m:
        out.add(f"defect#{m.group(1)}")
    header = "\n".join(text.split("\n")[:40])
    out |= {f"defect#{n}" for n in ISSUE.findall(header)}
    return out, text


def plan(tag):
    chapters = defined()
    if tag not in chapters:
        print(f"no rule @FR-{tag}")
        return 1
    ch = chapters[tag]
    axes = rule_depth.chapter_axes().get(ch, [])
    cells = cells_of(tag)
    print(f"@FR-{tag} ({ch}) — {len(cells)} citing file(s); axes measured: "
          f"{', '.join(axes) or 'none for this chapter'}")
    if not cells:
        print("  no cell names it: mark `needs-cell` (the rule-led walk adds its first guard)")
        return 0
    feats, texts = {}, {}
    for c in cells:
        feats[c], texts[c] = features(c, axes)
    want = set().union(*feats.values())
    core, held = [], set()
    while held != want:
        best = max(cells, key=lambda c: (len(feats[c] - held), -len(texts[c])))
        if not feats[best] - held:
            break
        core.append(best)
        held |= feats[best]
    # A cell that adds nothing (no axis value, no defect) still guards SOMETHING the measured
    # features cannot see — its own cases.  It is never silently dropped: it takes a route.
    print(f"  features the cells reach today: {len(want)}  ({sum(1 for f in want if f.startswith('defect'))} "
          f"defects, {sum(1 for f in want if not f.startswith('defect'))} axis values)")
    print(f"  core: {len(core)} of {len(cells)} cell(s) hold every feature")
    for c in core:
        print(f"    core   {rel(c)}  (+{len(feats[c])})")
    rest = [c for c in cells if c not in core]
    for c in rest:
        m = PATCH.search(texts[c])
        if c.endswith(".rs"):
            # A Rust test (a `#[cfg(test)]` unit test or a tests/*.rs one) is a home of its
            # own for what loft cannot observe — a layout, an image's bytes: it is weighed by
            # reading it and by breaking its cure (a mutation run), not folded into a cell.
            route = "unit   (Rust: read it, break its cure)"
        elif m:
            route = f"patch  tests/falsified/{m.group(1)}"
        else:
            route = "fold   (no patch receipt)"
        print(f"    {route:<46} {rel(c)}")
    if rest:
        print("\n  Retire a `patch` cell only after a core cell re-scores falsified against its patch;\n"
              "  fold a `fold` cell's test_* functions into a core cell.  Then run the rule's cells on\n"
              f"  both backends (make suite ARGS=\"@FR-{tag} --mode both-c\") and record it: done {tag} <note>.")
    return 0


def test_homes():
    """Every place a loft test lives, beyond the rule's own citing cells: the suite's other
    chapters and its attic (the inherited corpus no rule claims yet), this tree's scripts and
    docs, the Rust tests' embedded programs, and the libraries' own test suites."""
    suite = os.environ.get("LOFT_SUITE_DIR") or os.path.join(os.path.dirname(ROOT), "loft-suite")
    homes = [os.path.join(suite, "rules"), os.path.join(suite, "attic"),
             os.path.join(ROOT, "tests", "scripts"), os.path.join(ROOT, "tests", "docs"),
             os.path.join(ROOT, "tests")]
    for d in sorted(os.listdir(os.path.dirname(ROOT))):
        if d.startswith("loft-libs-"):
            homes.append(os.path.join(os.path.dirname(ROOT), d))
    return homes


def related(tag, pattern):
    """The lead list for a thorough pass: programs anywhere that match the rule's construct
    (`pattern`, a regex the curator writes from reading the rule), whether or not they cite it,
    and the code sites that enforce it.  A lead, not a verdict: read each before it counts."""
    rx = re.compile(pattern)
    cited = {os.path.normpath(rule_depth.abs_path(c)) for c in cells_of(tag)}
    hits = collections.defaultdict(list)
    for home in test_homes():
        for dirpath, _dirs, files in os.walk(home):
            if "/target" in dirpath or "/.git" in dirpath or "/native-auto" in dirpath:
                continue
            for name in files:
                if not name.endswith((".loft", ".rs")):
                    continue
                path = os.path.normpath(os.path.join(dirpath, name))
                if home.endswith(os.sep + "tests") and not name.endswith(".rs"):
                    continue
                try:
                    lines = open(path, encoding="utf-8", errors="replace").read().split("\n")
                except OSError:
                    continue
                for i, line in enumerate(lines, 1):
                    if rx.search(line) and not line.lstrip().startswith("//"):
                        hits[path].append(i)
    print(f"@FR-{tag}: {len(cells_of(tag))} citing file(s); /{pattern}/ matches "
          f"{len(hits)} file(s) across every test home")
    for path in sorted(hits, key=lambda p: (p in cited, rel(p))):
        mark = "cites" if path in cited else "     "
        lines = hits[path]
        print(f"  {mark} {rel(path)}:{lines[0]}" + (f"  (+{len(lines) - 1})" if len(lines) > 1 else ""))
    sites = rule_tags.citations_in([rule_tags.SRC, rule_tags.DEFAULT], rule_tags.CITE_EXTS).get(tag, [])
    print(f"\n  enforcing sites ({len(sites)}):")
    for f, n in sorted(sites)[:40]:
        print(f"    {os.path.relpath(rule_depth.abs_path(f), ROOT)}:{n}")
    return 0


GATE_E = os.path.join(ROOT, "selfhost", "front", "elab_gate.baseline.tsv")


def gate_e_verdict(cell):
    """Gate E's verdict on a suite cell: its baseline row's, or `agree` when it has none (the
    baseline lists only the programs the self-hosted elaborator does not agree with yet)."""
    suite = os.path.normpath(os.environ.get("LOFT_SUITE_DIR") or os.path.join(os.path.dirname(ROOT), "loft-suite"))
    item = "loft-suite/" + os.path.relpath(os.path.normpath(os.path.abspath(cell)), suite)
    for line in open(GATE_E, encoding="utf-8"):
        f = line.rstrip("\n").split("\t")
        if len(f) == 3 and f[2] == item:
            return f[1]
    return "agree"


def merge(target, intro_file, cells):
    """Merge suite cells of ONE clause into `target`, every case and receipt kept: each cell's
    header (license lines off) is a section of its own, its code follows verbatim, a `main`
    becomes `test_<the cell's name>`, and a top-level name or enum variant that another section
    already declares takes a per-section prefix (whole word, strings included, so a printed
    variant and its expected text change together).  Refused when gate E's verdicts differ: the
    merged file is refused whole if any part is, and the gate would lose a program it handled.
    Writes `target` only; retiring the cells, gate E's rows and the pin are the caller's."""
    verdicts = {c: gate_e_verdict(c) for c in cells}
    if len(set(verdicts.values())) > 1:
        print("gate E's verdicts differ — not merged:")
        for c, v in verdicts.items():
            print(f"  {v:<8} {os.path.basename(c)}")
        return 1
    intro = open(intro_file, encoding="utf-8").read().rstrip("\n").split("\n")

    def declared(code):
        top = set(re.findall(r"^(?:pub )?(?:fn|struct|enum|value struct|type) ([A-Za-z_0-9]+)", code, re.M))
        variants = set()
        for m in re.finditer(r"^enum [A-Za-z_0-9]+(?:<[^>]*>)?\s*\{", code, re.M):
            # The enum's body, brace-matched: a variant may carry a braced payload.
            depth, j = 1, m.end()
            while j < len(code) and depth:
                depth += {"{": 1, "}": -1}.get(code[j], 0)
                j += 1
            body = code[m.end():j - 1]
            depth, cur, parts = 0, "", []
            for ch in body:
                if ch == "{":
                    depth += 1
                elif ch == "}":
                    depth -= 1
                elif ch == "," and depth == 0:
                    parts.append(cur)
                    cur = ""
                elif depth == 0:
                    cur += ch
            parts.append(cur)
            variants |= {x.strip().split()[0] for x in parts if x.strip() and x.strip()[0].isupper()}
        return top, variants

    taken_top, taken_var = set(), set()
    heads, codes = [], []
    for k, cell in enumerate(cells):
        lines = open(cell, encoding="utf-8").read().split("\n")
        i = next(j for j, l in enumerate(lines) if l and not l.startswith("//"))
        head = lines[2:i]
        while head and head[0].strip() in ("//", ""):
            head.pop(0)
        while head and head[-1].strip() in ("//", ""):
            head.pop()
        code = "\n".join(lines[i:]).strip() + "\n"
        stem = os.path.basename(cell)[:-5]
        top, var = declared(code)
        px = "s" + str(k + 1)
        ren = {}
        for n in top:
            if n in taken_top and not n.startswith("test_"):
                ren[n] = (px.upper() + n) if n[0].isupper() else (px + "_" + n)
        for v in var:
            if v in taken_var:
                ren[v] = px.upper() + v
        for o, nw in sorted(ren.items(), key=lambda x: -len(x[0])):
            code = re.sub(r"\b" + re.escape(o) + r"\b", nw, code)
        if "fn main()" in code:
            tname = "test_" + re.sub(r"[^a-z0-9_]", "_", re.sub(r"^[0-9]+-", "", stem).replace("-", "_"))[:70]
            code = code.replace("fn main()", f"fn {tname}()", 1)
        for t in re.findall(r"^fn (test_[a-z_0-9]+)", code, re.M):
            if t in taken_top:
                raise SystemExit(f"two sections declare {t}")
        t2, v2 = declared(code)
        taken_top |= t2
        taken_var |= v2
        if ren:
            print(f"  {stem}: renamed {ren}")
        heads.append(f"// ── {stem} ──\n//\n" + "\n".join(head))
        codes.append(code)
    out = "\n".join(intro) + "\n//\n" + "\n//\n".join(heads) + "\n\n" + "\n".join(codes)
    open(target, "w", encoding="utf-8").write(out)
    print(f"merged {len(cells)} cell(s) into {os.path.basename(target)} (gate E: {next(iter(verdicts.values()))})")
    return 0


# ---------------------------------------------------------------- the per-cell instruments

def loft_bin():
    """The loft binary the instruments run: `$LOFT`, else this tree's debug build."""
    return os.environ.get("LOFT") or os.path.join(ROOT, "target", "debug", "loft")


def run_cell(cell, mode, env=None):
    """`loft --tests <mode> <cell>` — (exit code, the result line).  The program cache is off
    (a cached image once answered for the wrong build), and the cell's `.loft/` cache beside it
    is removed afterwards so it never reads as a reference or reaches a commit."""
    e = dict(os.environ, LOFT_NO_CACHE="1", **(env or {}))
    r = subprocess.run([loft_bin(), "--tests", mode, cell], env=e, capture_output=True, text=True,
                       timeout=int(os.environ.get("CURATE_TIMEOUT", "600")))
    shutil.rmtree(os.path.join(os.path.dirname(cell), ".loft"), ignore_errors=True)
    line = next((l for l in r.stdout.splitlines() + r.stderr.splitlines() if "test result" in l), "")
    return r.returncode, line.strip()


ARMINGS = {"leak": {"LOFT_LEAK_CHECK": "1"}, "strict": {"LOFT_STRICT_STORES": "1", "LOFT_POISON": "1"}}


def verify(cells):
    """Each cell on the interpreter and C, under the suite's own arming (the leak check) and the
    one a `@catches` file gets (strict stores + poison)."""
    bad = 0
    for c in cells:
        for arm, env in ARMINGS.items():
            for mode in ("--interpret", "--native-c"):
                code, line = run_cell(c, mode, env)
                bad += code != 0
                print(f"{'ok  ' if code == 0 else 'FAIL'} {arm:<6} {mode:<12} {os.path.basename(c)}  {line[:60]}")
    return 1 if bad else 0


def _break_one(text, fn):
    """The cell with ONE expectation of test `fn` broken: the first `== <literal>` in its body, or,
    when the test has none of its own, in the code of its section (from the end of the previous
    test function to this one — a helper it calls).  None when there is none to break."""
    a = text.index("fn " + fn + "(")
    b = text.index("\n}\n", a)
    rx = re.compile(r'assert\((.*?)==\s*("[^"]*"|-?\d+)')
    m, lo = rx.search(text[a:b]), a
    if not m:
        prev = [x.start() for x in re.finditer(r"(?m)^fn test_", text) if x.start() < a]
        lo = text.index("\n}\n", prev[-1]) if prev else next(x.start() for x in re.finditer(r"(?m)^[^/\n]", text))
        m = rx.search(text[lo:a])
        if not m:
            return None
    v = m.group(2)
    nv = ('"X' + v[1:]) if v.startswith('"') else str(int(v) + 1)
    return text[:lo + m.start(2)] + nv + text[lo + m.end(2):]


def vacuity(cells):
    """Can each test fail?  Break one expectation per test and require exactly that one test to
    fail — a test no broken expectation moves proves nothing (TESTING.md § Curating the suite)."""
    bad = 0
    for c in cells:
        text = open(c, encoding="utf-8").read()
        tests = re.findall(r"(?m)^fn (test_[a-z_0-9]+)", text) or (["main"] if "fn main()" in text else [])
        for t in tests:
            broken = _break_one(text, t) if t != "main" else None
            if broken is None:
                print(f"?    {os.path.basename(c)}::{t} — no `==` expectation to break; read it by hand")
                continue
            probe = os.path.join(os.path.dirname(c), ".curate-vacuity.loft")
            open(probe, "w", encoding="utf-8").write(broken)
            try:
                code, line = run_cell(probe, "--interpret")
            finally:
                os.remove(probe)
            one = re.search(r"\b1 failed\b", line) is not None
            bad += not one
            print(f"{'ok  ' if one else 'VAC '} {os.path.basename(c)}::{t}  {line[:50]}")
    return 1 if bad else 0


def cure_switches():
    text = open(os.path.join(ROOT, "src", "keys.rs"), encoding="utf-8").read()
    return sorted(set(re.findall(r'env_set\("(LOFT_NO_[A-Z0-9_]+)"\)', text)))


def switches(cells):
    """Which cure switches make each cell fail: the interpreter over every `LOFT_NO_*` switch, then
    C for a cell the interpreter sweep left unmoved.  A switch that moves a cell is its receipt;
    a cell no switch moves is asked whether it CAN fail (TESTING.md § Curating the suite)."""
    sw = cure_switches()
    for c in cells:
        if run_cell(c, "--interpret", ARMINGS["leak"])[0] != 0:
            print(f"{os.path.basename(c)}: fails with every cure ON — fix it first")
            continue
        hit = [s for s in sw if run_cell(c, "--interpret", {**ARMINGS["leak"], s: "1"})[0] != 0]
        if not hit:
            hit = ["C:" + s for s in sw if run_cell(c, "--native-c", {**ARMINGS["leak"], s: "1"})[0] != 0]
        print(f"{os.path.basename(c)}: {' '.join(hit) if hit else 'NONE — no cure switch moves it'}")
    return 0


def _git(args, cwd, check=True):
    r = subprocess.run(["git"] + args, cwd=cwd, capture_output=True, text=True)
    if check and r.returncode:
        raise SystemExit(f"git {' '.join(args)}: {r.stderr.strip()}")
    return r.stdout


def land(rule, chapter_dir, new, olds, message):
    """Retire `olds` (cell stems under rules/<chapter_dir>/) for the merged cell `new`: refuse while
    anything still names a retired cell; require the rule's feature count unchanged; commit and push
    the suite; replace gate E's rows (one row for the merged cell, or none when gate E agreed);
    re-pin loft's suite pin; commit loft.  Every step is the one the curation pass ran by hand."""
    suite = os.path.normpath(os.environ.get("LOFT_SUITE_DIR") or os.path.join(os.path.dirname(ROOT), "loft-suite"))
    d = os.path.join(suite, "rules", chapter_dir)
    skip = re.compile(r"/\.loft/|history|CHANGELOG|census-data|handoff|/" + re.escape(new) + r"\.loft$|" +
                      "|".join("/" + re.escape(o) + r"\.loft$" for o in olds))
    refs = []
    for o in olds:
        for base in (os.path.join(suite, "rules"), os.path.join(ROOT, "doc"), os.path.join(ROOT, "tests"),
                     os.path.join(ROOT, "scripts"), os.path.join(ROOT, "src")):
            r = subprocess.run(["grep", "-rlF", o, base], capture_output=True, text=True)
            refs += [f"{o} <- {x}" for x in r.stdout.split() if not skip.search(x)]
    if refs:
        print("references to the retired cells — repoint them first:")
        for x in refs:
            print("  " + x)
        return 1

    def features():
        out = subprocess.run([sys.executable, os.path.abspath(__file__), "plan", rule], capture_output=True, text=True).stdout
        m = re.search(r"features the cells reach today: (\d+)", out)
        return m.group(1) if m else "?"
    before = features()
    _git(["rm", "-q"] + [o + ".loft" for o in olds], d)
    _git(["add", new + ".loft"], d)
    after = features()
    if before != after:
        print(f"the rule's feature count moved {before} -> {after}: a merged header dropped a `loft#N` or an axis value")
        return 1
    # An edit made after its file was staged would miss the commit and pass every check that reads
    # the working tree (a moved cell committed without the citation added after the `git mv`).
    if subprocess.run(["git", "diff", "--quiet"], cwd=suite).returncode != 0:
        print("the suite has unstaged changes — stage or drop them first:")
        print(_git(["diff", "--stat"], suite))
        return 1
    trailer = "\n\nCo-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
    _git(["commit", "-q", "-m", message + trailer], suite)
    _git(["push", "-q", "origin", "HEAD:main"], suite)
    head = _git(["rev-parse", "HEAD"], suite).strip()
    rows_path = GATE_E
    lines = open(rows_path, encoding="utf-8").read().split("\n")
    ends = tuple(f"/{chapter_dir}/{o}.loft" for o in olds)
    rows = [l for l in lines if l.endswith(ends)]
    if rows and len(rows) != len(olds):
        raise SystemExit(f"gate E lists {len(rows)} of {len(olds)} retired cells — the verdicts differed")
    lines = [l for l in lines if l not in rows]
    if rows:
        verdict = rows[0].split("\t")[1]
        row = f"elab\t{verdict}\tloft-suite/rules/{chapter_dir}/{new}.loft"
        idx = [i for i, l in enumerate(lines) if l.startswith("elab\t")]
        lines.insert(next((i for i in idx if lines[i].split("\t", 2)[2] > row.split("\t", 2)[2]), idx[-1] + 1), row)
    text = "\n".join(lines)
    m = re.search(r"agree\t(\d+)\tof\t(\d+)", text)
    agree = int(m.group(1)) - (0 if rows else len(olds) - 1)
    text = text.replace(m.group(0), f"agree\t{agree}\tof\t{int(m.group(2)) - (len(olds) - 1)}")
    text = re.sub(r"(?m)^suite\t[0-9a-f]{40}$", "suite\t" + head, text)
    open(rows_path, "w", encoding="utf-8").write(text)
    open(os.path.join(ROOT, "tests", "loft-suite.pin"), "w", encoding="utf-8").write(head + "\n")
    _git(["add", "tests/loft-suite.pin", "selfhost/front/elab_gate.baseline.tsv"], ROOT)
    _git(["commit", "-q", "-m", f"Suite pin {head[:7]}: {message.splitlines()[0]}" + trailer], ROOT)
    print(f"landed: suite {head[:7]}, features {after}, gate E {m.group(0).replace(chr(9), ' ')} -> agree {agree}")
    return 0


def rel(p):
    """A cell's place: `suite:rules/<chapter>/<rule>/<file>` for the language suite, else the
    path under this tree."""
    full = os.path.normpath(rule_depth.abs_path(p))
    suite = os.environ.get("LOFT_SUITE_DIR") or os.path.join(os.path.dirname(ROOT), "loft-suite")
    suite = os.path.normpath(os.path.abspath(suite))
    if full.startswith(suite + os.sep):
        return "suite:" + os.path.relpath(full, suite)
    return os.path.relpath(full, ROOT)


def init():
    rows = read_ledger()
    chapters = defined()
    added = 0
    for tag, ch in chapters.items():
        if tag not in rows:
            rows[tag] = [tag, ch, "todo", "", "", ""]
            added += 1
    gone = [t for t in rows if t not in chapters]
    for t in gone:
        del rows[t]
    write_ledger(rows)
    print(f"ledger: {len(rows)} rules ({added} added, {len(gone)} no longer defined)")
    return 0


def status():
    rows = read_ledger()
    if not rows:
        print("no ledger yet: scripts/suite_curate.py init")
        return 1
    by_state = collections.Counter(r[2] for r in rows.values())
    done = len(rows) - by_state.get("todo", 0)
    print(f"{done} of {len(rows)} rules worked through — " +
          ", ".join(f"{n} {s}" for s, n in sorted(by_state.items())))
    per = collections.defaultdict(lambda: [0, 0])
    for r in rows.values():
        per[r[1]][1] += 1
        if r[2] != "todo":
            per[r[1]][0] += 1
    for ch, (d, n) in sorted(per.items()):
        if 0 < d < n:
            print(f"  {ch:<14} {d}/{n}")
    nxt = [t for t in order(rows) if rows[t][2] == "todo"][:1]
    if nxt:
        print(f"next: @FR-{nxt[0]} ({rows[nxt[0]][1]})")
    return 0


def nxt(n):
    rows = read_ledger()
    todo = [t for t in order(rows) if rows[t][2] == "todo"][:n]
    for t in todo:
        print(f"@FR-{t:<28} {rows[t][1]:<14} {len(cells_of(t))} cell(s)")
    return 0


def done(tag, note):
    rows = read_ledger()
    if tag not in rows:
        print(f"no ledger row for {tag} (init first)")
        return 1
    n = len(cells_of(tag))
    rows[tag][2] = "needs-cell" if n == 0 else "curated"
    rows[tag][3] = str(n)
    rows[tag][4] = datetime.date.today().isoformat()
    rows[tag][5] = note.replace("\t", " ")
    write_ledger(rows)
    print(f"@FR-{tag}: {rows[tag][2]} with {n} cell(s)")
    return 0


def main():
    a = sys.argv[1:]
    if not a or a[0] in ("-h", "--help"):
        print(__doc__)
        return 0
    cmd = a[0]
    tag = a[1].removeprefix("@FR-") if len(a) > 1 else ""
    if cmd == "init":
        return init()
    if cmd == "status":
        return status()
    if cmd == "next":
        return nxt(int(a[1]) if len(a) > 1 else 3)
    if cmd == "plan" and tag:
        return plan(tag)
    if cmd == "verify" and len(a) > 1:
        return verify(a[1:])
    if cmd == "vacuity" and len(a) > 1:
        return vacuity(a[1:])
    if cmd == "switches" and len(a) > 1:
        return switches(a[1:])
    if cmd == "land" and len(a) > 5:
        # land <rule> <chapter/rule-dir> <new-stem> <message-file> <old-stem>…
        return land(a[1].removeprefix("@FR-"), a[2], a[3], a[5:], open(a[4], encoding="utf-8").read().strip())
    if cmd == "merge" and len(a) > 4:
        return merge(a[1], a[2], a[3:])
    if cmd == "related" and tag and len(a) > 2:
        return related(tag, a[2])
    if cmd == "done" and tag:
        return done(tag, " ".join(a[2:]))
    print(__doc__)
    return 2


if __name__ == "__main__":
    sys.exit(main())
