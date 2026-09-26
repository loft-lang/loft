#!/usr/bin/env python3
# clippy-review — the census behind RELEASE.md § 8 (clippy suppressions).
#
# Answers, for every `#[allow(clippy::…)]` under src/, the two questions the
# release step asks and that a grep cannot: does clippy still fire here without
# it (a suppression on a function that has since shrunk is DEAD), and does
# anything on or above the line say why it is there.  A REPORT, never a gate,
# and never a cleanup — it edits nothing in the checkout.
#
# Method.  In a throwaway worktree (HEAD plus the working tree's uncommitted
# src/ changes) every `allow(` becomes `expect(clippy::allow_attributes, …)` and
# clippy runs the way CI runs it.  The compiler then reports each expectation
# nothing fulfilled (`unfulfilled_lint_expectations`), which is exactly the dead
# list — per lint, per attribute, with none of the guesswork of stripping
# attributes and matching warnings back to them.  The probe lint
# `clippy::allow_attributes` cannot fire once every allow is rewritten, so its
# own unfulfilled report is the receipt that the item COMPILED in that leg:
# a lint that is compiled and not reported is LIVE (measured), and an item no
# leg compiles is UNMEASURED rather than silently absent.
#
# Legs.  `--legs ci` (default) runs CI's three clippy lines; `--legs all` adds the
# two configurations CI never lints — the loft package with debug assertions
# ON (the dev profile strips them, so `#[cfg(debug_assertions)]` code is
# otherwise invisible) and the browser wasm rlib — and lists the warnings that
# hide there.
#
#   make clippy-review                       # the three CI legs, ~1 min warm
#   make clippy-review ARGS="--legs all"     # + debug-assertions + wasm32
#   make clippy-review ARGS="--json out.json --keep"
#   make clippy-review ARGS=--lengths        # only the function-length exemptions, no clippy run
#
# Function-length exemptions are `#[expect(clippy::too_many_lines, reason = …)]`, not
# allows (CODE.md § Functions): clippy itself fails an expectation that stopped
# firing, so none can be dead, and the one open question is the reason.  A reason of
# `inherited` is debt from before the rule — unexplained by definition — and the count
# of those is the function side's scoreboard.
#
# Exit status is 0 whenever the report was produced; a leg that fails to
# compile stops the run with that leg's stderr tail.
import argparse
import collections
import datetime
import json
import os
import pathlib
import re
import shutil
import subprocess
import sys

ATTR_LINE = re.compile(r"^\s*#(!?)\[")
ALLOW_START = re.compile(r"^\s*#(!?)\[allow\(")
CFG_ATTR_ALLOW = re.compile(r"^\s*#\[cfg_attr\((.*),\s*allow\(")
LINT = re.compile(r"\b(?:clippy::)?[a-z][a-z0-9_]*\b")
ITEM = re.compile(
    r"^(pub(?:\([^)]*\))?\s+)?(unsafe\s+|async\s+|const\s+|extern\s+\"C\"\s+)*"
    r"(fn|struct|enum|impl|mod|trait|type|const|static|let|use|extern)\b\s*([A-Za-z_][A-Za-z0-9_]*)?"
)
PROBE = "clippy::allow_attributes"

LEGS = {
    # CI's clippy lines: ci.yml runs the first two, the Makefile `ci:` gate the first
    # and the third (`tests/doc_hygiene.rs` pins that one).
    "all-features": ["--all-targets", "--all-features"],
    "default": [],
    "no-default": ["--no-default-features", "--all-targets"],
    # What CI never lints.
    "debug-assertions": [
        "--all-features", "--lib", "--bins",
        "--config", "profile.dev.package.loft.debug-assertions=true",
    ],
    "wasm-browser": [
        "--target", "wasm32-unknown-unknown", "--lib",
        "--no-default-features", "--features", "random",
    ],
}
CI_LEGS = ["all-features", "default", "no-default"]


def sh(args, cwd=None, check=True, env=None):
    r = subprocess.run(args, cwd=cwd, capture_output=True, text=True, env=env)
    if check and r.returncode != 0:
        sys.exit(f"{' '.join(args)} failed:\n{r.stderr[-2000:]}")
    return r.stdout


def lints_in(text):
    return [t for t in re.findall(r"(?:clippy::)?[a-z][a-z0-9_]*", text) if t != "allow"]


def blame_dates(root, path):
    out = subprocess.run(["git", "blame", "--line-porcelain", "--", path],
                         cwd=root, capture_output=True, text=True).stdout
    dates, cur, ln = {}, None, 0
    for line in out.split("\n"):
        m = re.match(r"^([0-9a-f]{40}) \d+ (\d+)", line)
        if m:
            cur, ln = m.group(1), int(m.group(2))
        elif line.startswith("author-time ") and cur:
            ts = int(line.split()[1])
            dates[ln] = (cur[:8], datetime.datetime.fromtimestamp(ts, datetime.timezone.utc).strftime("%Y-%m-%d"))
    return dates


def crate_roots(root):
    """file -> crate root, for the crate-root-allow redundancy check."""
    manifest = (root / "Cargo.toml").read_text()
    roots = ["src/lib.rs"] + re.findall(r'^\s*path\s*=\s*"(src/[^"]+)"', manifest, re.M)
    roots = [r for r in dict.fromkeys(roots) if (root / r).exists() and r != "src/lib.rs"]
    owner = {}
    for r in roots:
        owner[r] = r
        for mod in re.findall(r"^(?:pub )?mod ([a-z_0-9]+);", (root / r).read_text(), re.M):
            for cand in (f"src/{mod}.rs", f"src/{mod}/mod.rs"):
                if (root / cand).exists():
                    owner[cand] = r
    allows = {}
    for r in roots + ["src/lib.rs"]:
        m = re.search(r"#!\[allow\((.*?)\)\]", (root / r).read_text(), re.S)
        allows[r] = set(re.findall(r"clippy::\w+", m.group(1))) if m else set()
    return owner, allows


def lines_inside_strings(text):
    """0-based indices of lines that BEGIN inside a string literal or block comment.

    An `#![allow(…)]` there is text a generator emits into another file (the
    `src/fill.rs` header, the native prelude), not an attribute of this crate.
    """
    inside, i, n, line = set(), 0, len(text), 0
    state = None  # None | ("str",) | ("raw", hashes) | ("block", depth)
    while i < n:
        c = text[i]
        if c == "\n":
            line += 1
            if state is not None:
                inside.add(line)
            i += 1
            continue
        if state is None:
            if text.startswith("//", i):
                j = text.find("\n", i)
                i = n if j < 0 else j
            elif text.startswith("/*", i):
                state, i = ("block", 1), i + 2
            elif c == '"':
                state, i = ("str",), i + 1
            elif c in "rb" and (text.startswith('r"', i) or text.startswith("r#", i)
                                or text.startswith('br"', i) or text.startswith("br#", i)):
                j = i + (2 if c == "b" else 1)
                hashes = 0
                while text[j] == "#":
                    hashes, j = hashes + 1, j + 1
                if text[j] == '"':
                    state, i = ("raw", hashes), j + 1
                else:
                    i += 1
            elif c == "b" and text.startswith("b'", i):
                i += 2  # byte literal: fall through the char rule below
            elif c == "'":
                if text.startswith("\\", i + 1):
                    j = text.find("'", i + 2)
                    i = (j + 1) if j > 0 else i + 1
                elif i + 2 < n and text[i + 2] == "'":
                    i += 3
                else:
                    i += 1  # a lifetime
            else:
                i += 1
        elif state[0] == "str":
            if c == "\\":
                if i + 1 < n and text[i + 1] == "\n":  # a `\`-continued line
                    line += 1
                    inside.add(line)
                i += 2
            elif c == '"':
                state, i = None, i + 1
            else:
                i += 1
        elif state[0] == "raw":
            if c == '"' and text.startswith("#" * state[1], i + 1):
                state, i = None, i + 1 + state[1]
            else:
                i += 1
        else:  # block comment, nested
            if text.startswith("/*", i):
                state, i = ("block", state[1] + 1), i + 2
            elif text.startswith("*/", i):
                state, i = (None if state[1] == 1 else ("block", state[1] - 1)), i + 2
            else:
                i += 1
    return inside


def scan(root):
    """Every allow attribute under src/, with its justification and item."""
    owner, root_allows = crate_roots(root)
    recs, in_strings = [], []
    for p in sorted((root / "src").rglob("*.rs")):
        rel = str(p.relative_to(root))
        text = p.read_text()
        lines = text.split("\n")
        if not any(ALLOW_START.match(l) or CFG_ATTR_ALLOW.match(l) for l in lines):
            continue
        dates = blame_dates(root, rel)
        quoted = lines_inside_strings(text)
        i = 0
        while i < len(lines):
            line = lines[i]
            m = ALLOW_START.match(line)
            ca = CFG_ATTR_ALLOW.match(line)
            if not m and not ca:
                i += 1
                continue
            if i in quoted or "\\n" in line:
                in_strings.append((rel, i + 1))
                i += 1
                continue
            start = i
            end = i
            while ")]" not in lines[end]:
                end += 1
            block = "\n".join(lines[start:end + 1])
            body = block.split("allow(", 1)[1]
            lints = lints_in(body.rsplit(")", 1)[0] if ca else body.split(")]", 1)[0])
            inner = bool(m and m.group(1) == "!")
            after = lines[end].split(")]", 1)[1]
            inline = "//" in after
            above = lines[start - 1].strip() if start else ""
            above_ok = above.startswith("//")
            above_doc = above.startswith("///") or above.startswith("//!")
            k = start - 1
            while k >= 0 and ATTR_LINE.match(lines[k]):
                k -= 1
            block_above = lines[k].strip().startswith("//") if k >= 0 else False
            cfgs = [ca.group(1)] if ca else []
            for a in range(k + 1, start):
                if "cfg(" in lines[a]:
                    cfgs.append(lines[a].strip())
            t = end + 1
            while not inner and t < len(lines) and (ATTR_LINE.match(lines[t]) or not lines[t].strip()
                                                    or lines[t].strip().startswith("//")):
                if ATTR_LINE.match(lines[t]) and "cfg(" in lines[t]:
                    cfgs.append(lines[t].strip())
                t += 1
            item = "" if inner else (lines[t].strip() if t < len(lines) else "")
            im = ITEM.match(item)
            kind = "inner" if inner else (im.group(3) if im else "expr")
            name = "" if inner else ((im.group(4) or "") if im else "")
            if kind == "impl":
                name = item[:50]
            crate = owner.get(rel, "src/lib.rs")
            clippy = [l for l in lints if l.startswith("clippy::")]
            recs.append(dict(
                file=rel, line=start + 1, end=end + 1, inner=inner, cfg_attr=bool(ca),
                lints=lints, clippy=clippy, inline=inline, above=above_ok, above_doc=above_doc,
                block_above=block_above,
                justified=inline or above_ok,
                comment=(after.strip() if inline else (above if above_ok else ""))[:100],
                cfg=cfgs, kind=kind, name=name, item=item[:70],
                commit=dates.get(start + 1, ("?", "?"))[0], date=dates.get(start + 1, ("?", "?"))[1],
                redundant=[l for l in clippy if l in root_allows.get(crate, set())] if not inner else [],
                legs={},
            ))
            i = end + 1
    return recs, in_strings


LENGTH_EXPECT = re.compile(
    r'#\[expect\(\s*clippy::too_many_lines,\s*reason\s*=\s*"((?:[^"\\]|\\.)*)"\s*,?\s*\)\]')


def length_exemptions(root):
    """(file, line, reason) for every function-length exemption under src/."""
    out = []
    for p in sorted((root / "src").rglob("*.rs")):
        text = p.read_text()
        for m in LENGTH_EXPECT.finditer(text):
            out.append((str(p.relative_to(root)), text.count("\n", 0, m.start()) + 1, m.group(1)))
    return out


def length_report(root):
    ex = length_exemptions(root)
    inherited = [e for e in ex if e[2] == "inherited"]
    reasoned = [e for e in ex if e[2] != "inherited"]
    out = ["## Function-length exemptions", ""]
    out.append(f"`#[expect(clippy::too_many_lines)]`: **{len(ex)}** — `inherited` (debt, unexplained) "
               f"**{len(inherited)}** · with a reason **{len(reasoned)}**.  Clippy fails any that stopped "
               f"firing, so none is dead; the reasons below are for a reader to check against the code.")
    out.append("")
    if inherited:
        per = collections.Counter(f for f, _, _ in inherited)
        out.append("Inherited, by file: " + ", ".join(f"`{f}` {n}" for f, n in per.most_common(12))
                   + (", …" if len(per) > 12 else ""))
        out.append("")
    if reasoned:
        out.append("| where | reason |")
        out.append("|---|---|")
        for f, ln, why in reasoned:
            out.append(f"| `{f}:{ln}` | {why} |")
        out.append("")
    return "\n".join(out) + "\n"


def rewrite(tree, recs):
    """allow( -> expect(clippy::allow_attributes, …) on each recorded attribute."""
    by_file = collections.defaultdict(list)
    for r in recs:
        by_file[r["file"]].append(r["line"] - 1)
    n = 0
    for rel, idxs in by_file.items():
        p = tree / rel
        lines = p.read_text().split("\n")
        for i in idxs:
            lines[i] = lines[i].replace("allow(", f"expect({PROBE}, ", 1)
            n += 1
        p.write_text("\n".join(lines))
    return n


def run_leg(tree, target_dir, name, rewritten):
    env = dict(os.environ, CARGO_TARGET_DIR=str(target_dir))
    args = ["cargo", "clippy", *LEGS[name], "--message-format=json"]
    r = subprocess.run(args, cwd=tree, capture_output=True, text=True, env=env)
    if r.returncode != 0:
        sys.exit(f"leg {name}: `{' '.join(args)}` failed:\n{r.stderr[-3000:]}")
    unfulfilled = set()
    latent = collections.Counter()
    latent_sites = collections.defaultdict(list)
    for line in r.stdout.split("\n"):
        try:
            m = json.loads(line)
        except ValueError:
            continue
        if m.get("reason") != "compiler-message":
            continue
        msg = m["message"]
        code = (msg.get("code") or {}).get("code")
        prim = next((s for s in msg.get("spans", []) if s.get("is_primary")), None)
        if not prim or not prim["file_name"].startswith("src/"):
            continue
        if code == "unfulfilled_lint_expectations":
            txt = prim["text"][0]["text"][prim["column_start"] - 1:prim["column_end"] - 1]
            unfulfilled.add((prim["file_name"], prim["line_start"], txt))
        elif code and msg.get("level") in ("warning", "error") and not any(
                r["line"] <= prim["line_start"] <= r["end"] for r in rewritten.get(prim["file_name"], ())):
            latent[code] += 1
            latent_sites[code].append(f"{prim['file_name']}:{prim['line_start']}")
    return unfulfilled, latent, latent_sites


def classify(recs, legs_run, results, root):
    index = collections.defaultdict(list)
    for r in recs:
        index[r["file"]].append(r)
    for leg in legs_run:
        for (f, ln, lint) in results[leg]:
            for r in index.get(f, []):
                if r["line"] <= ln <= r["end"]:
                    r["legs"].setdefault(leg, set()).add(lint)
                    break
    # A file-scope `#![allow]` gets its compiled-receipt from the item attributes in
    # its scope: its own probe is fulfilled by any allow the rewrite did not reach
    # (one a macro expands, one in a `\`-continued string), so its silence proves
    # nothing.  For a crate root the scope is every file of that crate.
    owner, _ = crate_roots(root)
    probe_legs = collections.defaultdict(set)
    for r in recs:
        for leg in legs_run:
            if PROBE in r["legs"].get(leg, ()):
                probe_legs[r["file"]].add(leg)
                probe_legs[owner.get(r["file"], "src/lib.rs")].add(leg)
    for r in recs:
        compiled = [leg for leg in legs_run if PROBE in r["legs"].get(leg, ())]
        if r["inner"]:
            compiled = [leg for leg in legs_run if leg in probe_legs[r["file"]]
                        or PROBE in r["legs"].get(leg, ())]
        status = {}
        for lint in r["lints"]:
            if not compiled:
                status[lint] = "unmeasured"
            elif all(lint in r["legs"].get(leg, ()) for leg in compiled):
                status[lint] = "dead"
            else:
                status[lint] = "live"
        r["status"] = status
        r["compiled"] = compiled
        cl = [status[l] for l in r["clippy"]]
        r["verdict"] = ("unmeasured" if not compiled else
                        "dead" if cl and all(s == "dead" for s in cl) else
                        "partly" if "dead" in cl else "live")


def report(recs, in_strings, legs_run, latent, latent_sites, root):
    head = sh(["git", "rev-parse", "--short", "HEAD"], cwd=root).strip()
    today = datetime.date.today().isoformat()
    clippy_recs = [r for r in recs if r["clippy"]]
    outer = [r for r in clippy_recs if not r["inner"]]
    inner = [r for r in clippy_recs if r["inner"]]
    rustc_recs = [r for r in recs if not r["clippy"]]
    mentions = sum(len(r["clippy"]) for r in clippy_recs)
    out = []
    w = out.append
    w(f"# clippy suppression census — {today}, {head}")
    w("")
    w(f"Legs: {', '.join(legs_run)}.  Probe: `{PROBE}` (compiled-receipt).")
    w("")
    w("## Headline")
    w("")
    w(f"- attributes naming a clippy lint: **{len(clippy_recs)}** ({len(outer)} on items, "
      f"{len(inner)} file-scope `#![allow]`, {sum(1 for r in clippy_recs if r['cfg_attr'])} via `cfg_attr`), "
      f"naming {mentions} lint mentions across {len({l for r in clippy_recs for l in r['clippy']})} lints")
    uj = [r for r in outer if not r["justified"]]
    uj_plain = [r for r in outer if not r["inline"] and not (r["above"] and not r["above_doc"])]
    w(f"- justified (a `//` on the line, or a comment on the line above): "
      f"**{len(outer) - len(uj)}** of {len(outer)} on items · unjustified **{len(uj)}** "
      f"(of which {sum(1 for r in uj if r['block_above'])} have a comment above a longer attribute block; "
      f"{len(uj_plain)} if the item's own `///` doc line does not count)")
    vc = collections.Counter(r["verdict"] for r in clippy_recs)
    w(f"- verdict per attribute: dead **{vc['dead']}** · partly dead **{vc['partly']}** · "
      f"live **{vc['live']}** · unmeasured **{vc['unmeasured']}**")
    dl = collections.Counter(l for r in clippy_recs for l, s in r["status"].items() if s == "dead" and l.startswith("clippy::"))
    w(f"- dead lint mentions: **{sum(dl.values())}** of {mentions} — "
      + ", ".join(f"{l.replace('clippy::', '')} {n}" for l, n in dl.most_common()))
    red = [r for r in outer if r["redundant"]]
    w(f"- redundant with a crate-root `#![allow]` (the lint is already off for the whole crate): "
      f"**{len(red)}** attributes — "
      + ", ".join(f"{l.replace('clippy::', '')} {n}" for l, n in
                  collections.Counter(l for r in red for l in r["redundant"]).most_common()))
    if in_strings:
        w(f"- not counted: {len(in_strings)} `#![allow]` lines inside string literals — text a generator "
          f"emits into another file ({', '.join(f'`{f}:{l}`' for f, l in in_strings[:4])}"
          f"{', …' if len(in_strings) > 4 else ''})")
    rd = sum(1 for r in rustc_recs if r["lints"] and all(r["status"].get(l) == "dead" for l in r["lints"]))
    w(f"- outside this census: {len(rustc_recs)} `#[allow]` naming only rustc lints "
      f"(`dead_code` …), of which {rd} are dead in every compiled leg")
    w("")

    def row(r, lints):
        just = "inline" if r["inline"] else ("above" if r["above"] else "—")
        cfg = " ⚠cfg" if r["cfg"] else ""
        return (f"| `{r['file']}:{r['line']}` | {', '.join(l.replace('clippy::', '') for l in lints)} | "
                f"{r['kind']} `{r['name']}`{cfg} | {just} | {r['date']} |")

    out.append(length_report(root))
    w("## Dead suppressions")
    w("")
    w("The lint no longer fires here in any leg that compiles the item; removing the attribute leaves clippy silent.")
    w("")
    w("| where | dead lint(s) | item | justified | since |")
    w("|---|---|---|---|---|")
    for r in sorted(clippy_recs, key=lambda r: (r["file"], r["line"])):
        dead = [l for l in r["clippy"] if r["status"][l] == "dead"]
        if dead:
            w(row(r, dead) + ("" if r["verdict"] == "dead" else "  ← partly"))
    w("")
    w("## Live and unjustified")
    w("")
    w("Clippy still fires here, and nothing on or above the line says why the suppression is the right answer.")
    w("")
    w("| where | lint(s) | item | since |")
    w("|---|---|---|---|")
    for r in sorted(outer, key=lambda r: (r["file"], r["line"])):
        if r["verdict"] in ("live", "partly") and not r["justified"]:
            live = [l for l in r["clippy"] if r["status"][l] == "live"]
            w(f"| `{r['file']}:{r['line']}` | {', '.join(l.replace('clippy::', '') for l in live)} | "
              f"{r['kind']} `{r['name']}`{' ⚠cfg' if r['cfg'] else ''} | {r['date']} |")
    w("")
    w("## Unmeasured")
    w("")
    unm = [r for r in clippy_recs if r["verdict"] == "unmeasured"]
    if unm:
        w("No leg compiled the item (a cfg none of the legs enables):")
        w("")
        for r in unm:
            w(f"- `{r['file']}:{r['line']}` {', '.join(r['clippy'])} — {' '.join(r['cfg']) or r['item']}")
    else:
        w("none — every attribute was compiled by at least one leg.")
    w("")
    w("## File-scope `#![allow]`")
    w("")
    w("Struck: no code in the scope fires the lint beyond what item-level attributes already cover.")
    w("")
    w("| where | lints (dead ones struck) |")
    w("|---|---|")
    for r in inner:
        w(f"| `{r['file']}:{r['line']}` | " + ", ".join(
            (f"~~{l.replace('clippy::', '')}~~" if r["status"][l] == "dead" else l.replace("clippy::", ""))
            for l in r["clippy"]) + " |")
    extra = [leg for leg in legs_run if leg not in CI_LEGS]
    if extra:
        w("")
        w("## Warnings CI never sees")
        w("")
        w("Clippy warnings in code the CI legs do not compile (`-D warnings` would fail on these if the configuration were ever gated):")
        w("")
        for leg in extra:
            if latent[leg]:
                w(f"- **{leg}**: " + "; ".join(
                    f"`{c}` ×{n} ({', '.join(latent_sites[leg][c][:3])}{'…' if n > 3 else ''})"
                    for c, n in latent[leg].most_common()))
            else:
                w(f"- **{leg}**: none")
    return "\n".join(out) + "\n"


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--legs", choices=["ci", "all"], default="ci")
    ap.add_argument("--json", help="write the per-attribute records here")
    ap.add_argument("--keep", action="store_true", help="leave the worktree in place")
    ap.add_argument("--lengths", action="store_true",
                    help="only the function-length exemptions (a scan, no clippy run)")
    a = ap.parse_args()
    root = pathlib.Path(sh(["git", "rev-parse", "--show-toplevel"]).strip())
    if a.lengths:
        sys.stdout.write(length_report(root))
        return
    target_dir = pathlib.Path(os.environ.get("CARGO_TARGET_DIR", root / "target")) / "clippy-review"
    tree = target_dir / "tree"
    recs, in_strings = scan(root)
    if not recs:
        sys.exit("no #[allow] attributes found under src/ — nothing to measure")
    if tree.exists():
        subprocess.run(["git", "worktree", "remove", "--force", str(tree)], cwd=root, capture_output=True)
        shutil.rmtree(tree, ignore_errors=True)
    target_dir.mkdir(parents=True, exist_ok=True)
    sh(["git", "worktree", "add", "--detach", str(tree), "HEAD"], cwd=root)
    try:
        # The working tree's uncommitted edits, so the census reads what is here, not HEAD.
        for rel in sh(["git", "ls-files", "-m", "-o", "--exclude-standard", "--",
                       "src", "build.rs", "Cargo.toml", "Cargo.lock"], cwd=root).split():
            (tree / rel).parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(root / rel, tree / rel)
        for rel in sh(["git", "ls-files", "-d", "--", "src"], cwd=root).split():
            (tree / rel).unlink(missing_ok=True)
        rewrite(tree, recs)
        rewritten = collections.defaultdict(list)
        for r in recs:
            rewritten[r["file"]].append(r)
        legs_run = CI_LEGS if a.legs == "ci" else list(LEGS)
        if "wasm-browser" in legs_run and "wasm32-unknown-unknown" not in sh(["rustup", "target", "list", "--installed"], check=False):
            print("note: wasm32-unknown-unknown is not installed; skipping the wasm-browser leg", file=sys.stderr)
            legs_run.remove("wasm-browser")
        results, latent, sites = {}, {}, {}
        for leg in legs_run:
            print(f"clippy-review: {leg} …", file=sys.stderr)
            results[leg], latent[leg], sites[leg] = run_leg(tree, target_dir, leg, rewritten)
            probes = sum(1 for (_, _, l) in results[leg] if l == PROBE)
            if probes == 0:
                sys.exit(f"leg {leg}: no `{PROBE}` probe was reported — the expectation rewrite did not "
                         f"take effect, so this run cannot tell dead from uncompiled; not reporting")
        classify(recs, legs_run, results, root)
        sys.stdout.write(report(recs, in_strings, legs_run, latent, sites, root))
        if a.json:
            for r in recs:
                r["legs"] = {k: sorted(v) for k, v in r["legs"].items()}
            pathlib.Path(a.json).write_text(json.dumps(recs, indent=1))
    finally:
        if not a.keep:
            subprocess.run(["git", "worktree", "remove", "--force", str(tree)], cwd=root, capture_output=True)


if __name__ == "__main__":
    main()
