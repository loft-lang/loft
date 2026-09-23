#!/usr/bin/env python3
# Copyright (c) 2026 Jurjen Stellingwerff
# SPDX-License-Identifier: LGPL-3.0-or-later
"""The per-release checklist, generated — with everything the machine can decide
already decided.

RELEASE.md describes the release in prose.  It used to carry the manual steps as well,
across three overlapping partial lists, and the steps that lived in *none* of them --
the Windows `self-update`, the registry splice, `scripts/install.sh` -- are exactly the
ones that got skipped.  Not because anyone decided to skip them: because no list said
them.  Prose cannot be worked through, and three lists cannot be worked through without
missing something.

This is the one list, and it is generated so it cannot drift from the repo it describes.
Three properties make it usable rather than another thing to read:

**Automatic items are MEASURED on every run and can never be ticked.**  "Is `make ci`
green" is not a promise a human gets to make; it is a file with a verdict line in it and
a timestamp, and this reports what that file actually says.  A gate you can tick is a
gate that gets ticked.

**Manual items are the ones a machine genuinely cannot do**, and each carries the exact
command and the answer that counts as a pass.  Those are tickable, because a person
running loft on a Windows box is evidence and nothing else is.

**Conditional items appear only when they apply.**  The VS Code extension pass and the
native-debug gate are per-release rituals for code that most releases do not touch; this
asks git whether they changed since the last tag and stays silent when they did not.  A
checklist that lists work nobody needs to do is one people learn to skim.

Usage:
    scripts/release-checklist.py                    # the list for Cargo.toml's version
    scripts/release-checklist.py --version 2026.9.0
    scripts/release-checklist.py --fetch            # refresh origin/main + tags first
    scripts/release-checklist.py --done M-win-selfupdate --note "on the NUC, 2026-09-02"
    scripts/release-checklist.py --undo M-win-selfupdate
    scripts/release-checklist.py --json

Exit codes: 0 every applicable automatic gate ran and passed; 1 at least one FAILED;
3 none failed but some never ran (UNKNOWN) -- not-yet-evidence, distinct from red,
because a skipped check must never read as a passed one (@PLN156).

Progress on the manual half is kept in `doc/claude/releases/<cycle>/checklist.json`,
committed with the tree -- a tick made on one machine is a tick on every machine
state, never committed, and never consulted for an automatic item.
"""

from __future__ import annotations

import argparse
import datetime
import json
import os
import re
import subprocess
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
def cycle_of(version: str) -> str:
    """The release cycle a Cargo version belongs to: `2026.9.0` -> `2026-09`, which is both the
    `YYYY-MM` branch name and the directory under `doc/claude/releases/`.  A pre-calendar
    version (`0.8.4`) is its own cycle."""
    major, minor, *_ = version.split(".")
    return f"{major}-{int(minor):02d}" if int(major) >= 2000 else version


def state_path(version: str) -> str:
    """Where this version's manual-item record lives: beside the cycle's release write-up,
    committed, so the evidence survives the machine it was gathered on."""
    return os.path.join(ROOT, "doc", "claude", "releases", cycle_of(version), "checklist.json")
REPO = "loft-lang/loft"

# States an item can be in.  `UNKNOWN` is deliberately not a pass: a check that could not
# run and a check that passed are the two answers a release must never confuse (the same
# distinction `loft verify-self` draws with its exit 2).
OK, FAIL, UNKNOWN, TODO, DONE, NA = "OK", "FAIL", "UNKNOWN", "TODO", "DONE", "NA"
# A manual tick whose recorded commit differs from HEAD in what ships (`src/`, `default/`,
# `tests/`) — a `[cand]` claim about a tree that is no longer this one.  Not done, not
# undone: the evidence still exists, it just names another candidate.
STALE = "STALE"

MARK = {
    OK: "[x]", FAIL: "[!]", UNKNOWN: "[?]", TODO: "[ ]", DONE: "[x]", NA: "[-]", STALE: "[~]",
}

# What ships.  A tick's commit is compared with HEAD over these paths only, so a docs-only
# commit after a candidate sweep does not invalidate the sweep.
SHIPPED_PATHS = ["src", "default", "tests", "Cargo.toml", "Cargo.lock", "loft-ffi"]

# The record being worked on (`releases/<cycle>/checklist.json`), set by `main` before the
# items are built so an automatic check can read the waivers it holds.  Keys that start
# with `_` are not item ticks.
RECORD: dict = {}


def sh(*args: str, cwd: str = ROOT, timeout: int = 60) -> tuple[int, str]:
    """Run a command and return (exit code, combined output).  Never raises."""
    try:
        p = subprocess.run(
            args, cwd=cwd, capture_output=True, text=True, timeout=timeout
        )
        return p.returncode, (p.stdout + p.stderr).strip()
    except FileNotFoundError:
        return 127, f"{args[0]}: not installed"
    except subprocess.TimeoutExpired:
        return 124, f"{args[0]}: timed out after {timeout}s"


def cargo_version() -> str:
    with open(os.path.join(ROOT, "Cargo.toml"), encoding="utf-8") as f:
        for line in f:
            m = re.match(r'^version\s*=\s*"([^"]+)"', line)
            if m:
                return m.group(1)
    sys.exit("Cargo.toml has no top-level version")


def newest_mtime(paths: list[str]) -> tuple[float, str]:
    """Most recently modified file under `paths`, and which one it was."""
    best, who = 0.0, ""
    for rel in paths:
        p = os.path.join(ROOT, rel)
        if os.path.isfile(p):
            if os.path.getmtime(p) > best:
                best, who = os.path.getmtime(p), rel
            continue
        for dirpath, dirnames, filenames in os.walk(p):
            dirnames[:] = [d for d in dirnames if d not in {"target", ".git", ".loft"}]
            for fn in filenames:
                fp = os.path.join(dirpath, fn)
                try:
                    m = os.path.getmtime(fp)
                except OSError:
                    continue
                if m > best:
                    best, who = m, os.path.relpath(fp, ROOT)
    return best, who


class Item:
    """One line of the checklist.

    `check` present  -> automatic: re-measured every run, not tickable.
    `check` absent   -> manual: tickable, and `how` is the command to run.

    Two more axes, orthogonal to that one:

    `report`  -> the row must be READ, not be true.  The monthly reviews, the censuses and
                 the perf pass are what RELEASE.md calls "a report, never a blocker"; they
                 were counted in one tally with the valgrind sweep, so "21/27 done" said
                 nothing about what was blocking.  A report row never blocks and never
                 sets the exit code; it is tallied apart.
    `derived` -> a manual row a green gate JOB on HEAD's commit satisfies without a
                 hand-run (the valgrind sweep the nightly already ran on this commit).
                 The callable answers (state, evidence); OK means done.  The hand-run stays
                 as the fallback when the gate cannot run, which is why the row is still
                 tickable.
    """

    def __init__(
        self,
        ident: str,
        title: str,
        how: str,
        passes: str = "",
        check=None,
        applies=True,
        cadence: str = "",
        report: bool = False,
        derived=None,
    ):
        self.id = ident
        self.title = title
        self.how = how
        self.passes = passes
        self.check = check
        self.applies = applies
        self.report = report
        self.derived = derived
        # When this item can be FINISHED, earlier than the release window itself
        # (@PLN156).  The test is whether its evidence stays valid as the tree moves
        # on, because a tick is a claim about the release, not about the day it was
        # made: "mid" = completable at the cycle's halfway point, since it measures
        # overall stability or a process state rather than a release artifact; "cand"
        # = worth RUNNING early as warning, but its tick must name the tag candidate,
        # so it is listed apart at mid and counted nowhere; "pre" = completable in the
        # month's last days as pre-work, so the release does not spill deep into the
        # new month.  Empty = the release window only (it needs the tag, the draft, or
        # the published assets to exist).
        #
        # A row that cannot be finished in a phase does not belong in that phase's
        # tally.  Carrying the sweeps under "mid" made the mid view report 7/12 with
        # a denominator no halfway run could ever reach, which is a gate whose
        # threshold has drifted from its subject — it reads as permanent unfinished
        # work and teaches the reader to skim it.
        self.cadence = cadence
        self.state = NA
        self.evidence = ""

    @property
    def automatic(self) -> bool:
        return self.check is not None

    def resolve(self, state: dict) -> None:
        if not self.applies:
            self.state, self.evidence = NA, "does not apply to this release"
            return
        if self.automatic:
            self.state, self.evidence = self.check()
            return
        rec = state.get(self.id)
        if rec:
            self.state = DONE
            self.evidence = rec.get("at", "")
            if rec.get("commit"):
                self.evidence += f" @ {rec['commit'][:12]}"
            if rec.get("note"):
                self.evidence += " — " + rec["note"]
            # A candidate-bound tick is a claim about ONE tree.  If what ships moved
            # since, the claim is about another candidate — 2026-09's sweeps were ticked
            # on e77ef442 and the tag was b1016d00, and only a by-hand diff said nothing
            # shipped had moved in between.
            if "cand" in self.cadence.split() and rec.get("commit"):
                moved = shipped_moved_since(rec["commit"])
                if moved is None:
                    self.evidence += "  (commit unknown to this clone — cannot tell whether what ships moved)"
                elif moved:
                    self.state = STALE
                    self.evidence += f"  STALE: {moved} changed since — re-run on the candidate"
            return
        if self.derived is not None:
            st, why = self.derived()
            if st == OK:
                self.state, self.evidence = DONE, "covered: " + why
                return
            if why:
                self.evidence = why
        self.state = TODO


def shipped_moved_since(commit: str):
    """What shipped that changed between `commit` and HEAD — the first path, or "" for
    nothing, or None when the commit is not in this clone."""
    code, _ = sh("git", "cat-file", "-e", f"{commit}^{{commit}}")
    if code != 0:
        return None
    code, out = sh("git", "diff", "--name-only", commit, "HEAD", "--", *SHIPPED_PATHS)
    if code != 0:
        return None
    first = out.splitlines()[0] if out else ""
    return first


# --------------------------------------------------------------------------------------
# The automatic checks.  Each returns (state, one-line evidence).  They report what they
# measured, never what they assume: an unreachable network is UNKNOWN, not a pass.
# --------------------------------------------------------------------------------------


def check_version_untagged(version: str):
    """Cargo.toml names this release, and nothing has tagged it yet.

    Both halves, because the checklist is usually generated for a version the tree has
    not reached: reporting "no v2026.9.0 tag yet" as a pass, with a message that names
    Cargo.toml without reading it, states the bump has happened at the exact moment it
    has not.  Cargo.toml is what `make-release.sh` names the bundles from, so a tag
    pushed ahead of the bump builds a release for the previous version.
    """
    code, out = sh("git", "tag", "--list", f"v{version}")
    if code != 0:
        return UNKNOWN, out
    if out.strip():
        return FAIL, f"v{version} is already tagged — this release already happened"
    have = cargo_version()
    if have != version:
        return FAIL, (
            f"Cargo.toml still says {have} — bump it to {version} before tagging, or "
            f"the bundles are built and named for {have}"
        )
    return OK, f"Cargo.toml is {version}; no v{version} tag yet"


def previous_tag(version: str) -> str:
    """The release before this one, as git sees it."""
    code, out = sh("git", "tag", "--list", "v*", "--sort=-v:refname", "--merged", "HEAD")
    if code != 0:
        return ""
    tags = [t for t in out.splitlines() if t.strip() and t != f"v{version}"]
    return tags[0] if tags else ""


def check_changelog(version: str, path: str, label: str, heading: bool):
    """Did this file gain this release's entries?

    Two files, two conventions: CHANGELOG.md cuts a `## YYYY-MM` section per cycle,
    CHANGELOG_TECHNICAL.md accumulates under `## [Unreleased]`.  Only the first has a
    heading worth asserting -- so the shared half of the question is asked with git
    instead: a changelog that has not been touched since the previous tag describes the
    previous release, whatever headings it carries.  That also catches the case a
    heading check cannot see, a patch release under a month section already written.
    """
    fp = os.path.join(ROOT, path)
    if not os.path.isfile(fp):
        return FAIL, f"{path} is missing"
    if heading:
        m = re.match(r"^(\d{4})\.(\d{1,2})\.", version)
        if not m:
            return UNKNOWN, f"cannot derive a month heading from {version}"
        want = f"## {m.group(1)}-{int(m.group(2)):02d}"
        with open(fp, encoding="utf-8") as f:
            if want not in f.read():
                return FAIL, f"{label} has no `{want}` section — write it before tagging"
    prev = previous_tag(version)
    if not prev:
        return OK, f"{label} present (no previous tag to compare against)"
    code, out = sh("git", "diff", "--stat", f"{prev}..HEAD", "--", path)
    if code != 0:
        return UNKNOWN, f"could not diff {path} against {prev}"
    if not out.strip():
        return FAIL, f"{label} is unchanged since {prev} — it describes the LAST release"
    changed = out.strip().splitlines()[-1].strip()
    return OK, f"{label} gained entries since {prev} ({changed})"


def check_tree_clean():
    code, out = sh("git", "status", "--porcelain")
    if code != 0:
        return UNKNOWN, out
    if out:
        n = len(out.splitlines())
        return FAIL, f"{n} uncommitted change(s) — a tag must name a committed tree"
    return OK, "working tree clean"


def check_head_on_main():
    code, _ = sh("git", "merge-base", "--is-ancestor", "origin/main", "HEAD")
    if code == 0:
        _, sha = sh("git", "rev-parse", "--short", "origin/main")
        return OK, f"HEAD contains origin/main ({sha})"
    if code == 1:
        return (
            FAIL,
            "HEAD is BEHIND origin/main — a tag here ships a tree main has moved past "
            "(and a PR from it merges as BLOCKED); rebase first",
        )
    return UNKNOWN, "could not compare against origin/main (run with --fetch)"


def check_ci_verdict():
    """`make ci`'s own verdict line, and whether it still describes this tree.

    The exit code of the wrapper is not the gate's answer -- `result.txt` carries it.
    And a green run against an older tree is not a green run: the timestamp is half the
    claim, so a verdict older than the newest source file reports STALE rather than pass.
    """
    p = os.path.join(ROOT, "result.txt")
    if not os.path.isfile(p):
        return UNKNOWN, "no result.txt — run `make ci`"
    with open(p, encoding="utf-8", errors="replace") as f:
        text = f.read()
    if "CI-RESULT: ALL GATES PASSED" not in text:
        return FAIL, "result.txt does not say `CI-RESULT: ALL GATES PASSED`"
    verdict_at = os.path.getmtime(p)
    src_at, who = newest_mtime(
        ["src", "default", "tests", "Cargo.toml", "Cargo.lock", "loft-ffi"]
    )
    when = datetime.datetime.fromtimestamp(verdict_at).strftime("%Y-%m-%d %H:%M")
    if src_at > verdict_at:
        return FAIL, f"green at {when}, but {who} changed after it — re-run `make ci`"
    return OK, f"ALL GATES PASSED at {when}, newer than every source file"


# What determines the reference's CONTENT.  `doc/loft-reference.typ` is itself generated
# by `gendoc`, so comparing the PDF against it answers a question nobody asked: when the
# real inputs move and nobody re-runs `gendoc`, BOTH derived files stay put and the
# comparison reads green.  The chain is
#   tests/docs/*.loft + default/*.loft + gendoc + Cargo.toml
#     -> doc/loft-reference.typ -> doc/loft-reference.pdf
# `tests/docs/` is where the prose and every example live (which is why page 1 can claim
# every example is an executable part of the test suite), `default/` supplies the stdlib
# API sections, and Cargo.toml supplies the version printed on the title page.
PDF_INPUTS = [
    "tests/docs",
    "default",
    "src/gendoc.rs",
    "src/documentation.rs",
    "Cargo.toml",
]

PDF = os.path.join("doc", "loft-reference.pdf")


def check_reference_pdf():
    """The reference PDF is current against what actually decides its content.

    `make-release.sh` copies this file into every bundle when it exists, and never
    builds it -- so a stale one ships a reference that does not describe the release, in
    all four zips, silently.  Unlike the HTML docs, which the tag's `docs` job
    regenerates from source, nothing rebuilds this: `make pdf` (after `gendoc`) is a
    hand-run step, RELEASE.md § 9.
    """
    pdf = os.path.join(ROOT, PDF)
    if not os.path.isfile(pdf):
        return FAIL, f"no {PDF} — every bundle ships without a reference"
    pdf_at = os.path.getmtime(pdf)
    when = datetime.datetime.fromtimestamp(pdf_at).strftime("%Y-%m-%d %H:%M")
    src_at, who = newest_mtime(PDF_INPUTS)
    if src_at > pdf_at:
        src = datetime.datetime.fromtimestamp(src_at).strftime("%Y-%m-%d %H:%M")
        return FAIL, (
            f"built {when}, but {who} changed at {src} — run `cargo run --bin gendoc && "
            f"make pdf`, or all four bundles ship a stale reference"
        )
    return OK, f"built {when}, newer than every input that decides its content"


def check_reference_pdf_version(version: str):
    """The PDF SAYS it is this release.

    Read out of the shipping artifact rather than off its source, because the two can
    disagree in exactly the case that matters: `gendoc` stamps the title page and the
    document keywords from `CARGO_PKG_VERSION`, so bumping Cargo.toml without
    re-running it leaves a PDF headed "Version <previous>" -- correct-looking, freshly
    dated, and wrong on the one page every reader sees first.  A timestamp cannot catch
    that; the bytes can.
    """
    pdf = os.path.join(ROOT, PDF)
    if not os.path.isfile(pdf):
        return FAIL, f"no {PDF}"
    code, out = sh("pdfinfo", pdf)
    if code == 127:
        return UNKNOWN, "pdfinfo not installed (poppler-utils) — cannot read the PDF"
    if code != 0:
        return FAIL, f"pdfinfo could not read {PDF}: {out.splitlines()[0] if out else ''}"
    keywords = ""
    for line in out.splitlines():
        if line.startswith("Keywords:"):
            keywords = line.split(":", 1)[1].strip()
    tcode, text = sh("pdftotext", "-f", "1", "-l", "1", pdf, "-")
    on_page = f"Version {version}" in text if tcode == 0 else None
    if keywords and keywords != f"v{version}":
        return FAIL, (
            f"the PDF says it is {keywords}, this release is v{version} — re-run "
            f"`cargo run --bin gendoc && make pdf` after the version bump"
        )
    if on_page is False:
        return FAIL, f"the PDF's title page does not say `Version {version}`"
    if not keywords and on_page is None:
        return UNKNOWN, "could not read a version out of the PDF"
    return OK, f"the PDF says v{version}, on its title page and in its metadata"


def _pdf_text():
    """The shipped PDF's text, flattened — or a reason it could not be read."""
    pdf = os.path.join(ROOT, PDF)
    if not os.path.isfile(pdf):
        return None, f"no {PDF}"
    code, out = sh("pdftotext", pdf, "-", timeout=120)
    if code == 127:
        return None, "pdftotext not installed (poppler-utils) — cannot read the PDF"
    if code != 0:
        return None, f"pdftotext failed on {PDF}"
    return re.sub(r"\s+", " ", out), ""


def check_reference_pdf_content(): 
    """What is INSIDE the reference, read out of the shipping bytes.

    Regenerating the PDF is the easy half and a timestamp can police it.  This is the
    other half: a PDF can be freshly built, correctly versioned, and still be missing a
    chapter -- `documentation::get_topic_sources` builds the topic list with `.ok()` and
    `filter_map`, so a topic file it cannot read is DROPPED, silently, and the reference
    simply comes out one chapter shorter.  Nothing downstream notices: the build
    succeeds, the page count is still four figures, and the missing page is only missing
    to the reader.

    So this walks every level-1 part the document has.  The 35 topics, whose headings
    are the topic files' `@NAME` (gendoc emits that, not `@TITLE`).  The four chapters
    that are not topics -- Getting Started, vs Rust, vs Python, Roadmap -- each read
    from a `doc/*.html` file with `if let Ok(...)`, so a missing file takes the chapter
    with it just as quietly.  The Standard Library chapter, which needs asking about
    twice: its heading is pushed unconditionally, so the heading proves only that gendoc
    ran, and an EMPTY chapter carries it just as well as a full one.  And no placeholder
    marker, in a document that ships to readers offline.

    A presence check can pass on a chapter that was dropped but whose name still occurs
    in prose.  That is the residual risk here and it is the right way round: the failure
    it cannot rule out is a false pass on a name collision, not a false alarm.

    The stdlib count is EVIDENCE, not a gate.  The reference does not name every
    `pub fn` -- a good share are documented as methods on their receiver instead -- so
    "every function appears" would be a false failure, and picking a percentage would be
    inventing a threshold.  The count is printed instead, where a DROP is visible to
    whoever reads the line.
    """
    text, err = _pdf_text()
    if text is None:
        return UNKNOWN, err

    missing = []
    docs = os.path.join(ROOT, "tests", "docs")
    for entry in sorted(os.listdir(docs)):
        if not entry.endswith(".loft") or entry.startswith("00-"):
            continue
        path = os.path.join(docs, entry)
        if not os.path.isfile(path):
            continue
        name = None
        with open(path, encoding="utf-8", errors="replace") as f:
            for line in f:
                if line.startswith("// @NAME: "):
                    name = line[len("// @NAME: ") :].strip()
        if not name:
            missing.append(f"{entry} (no @NAME)")
        elif name not in text:
            missing.append(f"{entry} — \"{name}\"")
    if missing:
        return FAIL, (
            f"{len(missing)} topic(s) in tests/docs are NOT in the reference: "
            + "; ".join(missing[:4])
            + (" …" if len(missing) > 4 else "")
        )

    # The parts that are NOT topics.  Four of the five are assembled with
    # `if let Ok(read_to_string(...))` over a `doc/*.html` file, so a missing file
    # removes the whole chapter and says nothing -- the same silent drop as a topic,
    # from a different direction.  `= Standard Library` is the exception: it is pushed
    # unconditionally, so its heading proves nothing about its contents, which is why
    # the emptiness check below exists rather than a presence check alone.
    for part in ("Getting Started", "vs Rust", "vs Python", "Roadmap", "Standard Library"):
        if part not in text:
            return FAIL, f"the reference has no `{part}` chapter"

    for marker in ("TODO", "FIXME", "TBD", "not yet implemented"):
        if marker in text:
            return FAIL, f"the reference ships the placeholder {marker!r}"

    fns = set()
    default = os.path.join(ROOT, "default")
    for entry in sorted(os.listdir(default)):
        if entry.endswith(".loft"):
            with open(os.path.join(default, entry), encoding="utf-8", errors="replace") as f:
                fns.update(re.findall(r"^pub fn (\w+)", f.read(), re.M))
    # Word boundaries, not `in`: a bare substring test counts `map` as present because
    # the chapter list contains "Roadmap", which is enough to keep the empty-chapter
    # guard below from ever reaching 0.  The two agree on the real document (the
    # functions genuinely appear as words); they disagree exactly where it matters.
    named = sum(1 for n in fns if re.search(rf"\b{re.escape(n)}\b", text))
    if named == 0:
        return FAIL, (
            "the Standard Library chapter names no stdlib function — the heading is "
            "emitted unconditionally, so an empty chapter still carries it"
        )
    topics = len(
        [e for e in os.listdir(docs) if e.endswith(".loft") and not e.startswith("00-")]
    )
    return OK, (
        f"{topics} topics + 4 chapters present, no placeholders; "
        f"{named}/{len(fns)} stdlib pub fns named"
    )


def check_reference_review():
    """How much of the reference has been READ against the language as it behaves.

    The three `A-pdf*` checks establish that the document is whole, current and stamped
    with this version; not one of them reads a sentence, so all three stay green on a
    chapter that describes behaviour the language dropped two releases ago.  That is a
    person's judgement and it stays one -- what a script can do is say how much of it
    has been done, so the work can happen the week a chapter changes instead of on tag
    day, where it turns into a skim.  The watermarks live in
    `doc/claude/REFERENCE_REVIEW.md`; `make reference-review` is the worklist.
    """
    code, out = sh(sys.executable, os.path.join(ROOT, "scripts", "reference-review.py"))
    if code != 0:
        return UNKNOWN, "scripts/reference-review.py failed"
    m = re.search(r"(\d+)/(\d+) chapters reviewed at their current source", out)
    if not m:
        return UNKNOWN, "could not read the reference-review count"
    done, total = int(m.group(1)), int(m.group(2))
    if done == total:
        return OK, f"all {total} chapters read at their current source"
    return FAIL, (
        f"{total - done} of {total} chapters owe a read — `make reference-review`; "
        f"the A-pdf checks cannot see a chapter that is merely UNTRUE"
    )


def check_skills_review():
    """How much of the agent-skill set has been READ against the tree as it behaves.

    A skill (.claude/skills/) is loaded INSTEAD of the canonical doc it paraphrases,
    so a stale one steers every future session wrong in the one channel nobody
    cross-checks.  The script decides the mechanical half outright (cited paths, make
    targets and LOFT_* switches resolve) and counts the by-hand half: each skill's
    watermark against movement in the skill OR its cited docs/scripts.  The
    content/usability/conciseness read stays a person's judgement — SKILLS_REVIEW.md
    defines the three axes; `make skills-review` is the worklist.
    """
    code, out = sh(sys.executable, os.path.join(ROOT, "scripts", "skills-review.py"))
    if code != 0:
        return UNKNOWN, "scripts/skills-review.py failed"
    if "BROKEN REFERENCES" in out:
        n = re.search(r"BROKEN REFERENCES \((\d+)\)", out)
        return FAIL, (
            f"{n.group(1) if n else 'some'} cited paths/targets/switches do not resolve "
            f"— stale by construction; `make skills-review` names them"
        )
    m = re.search(r"(\d+)/(\d+) skills reviewed at their current sources", out)
    if not m:
        return UNKNOWN, "could not read the skills-review count"
    done, total = int(m.group(1)), int(m.group(2))
    if done == total:
        return OK, f"all {total} skills read at their current sources"
    return FAIL, (
        f"{total - done} of {total} skills owe a read — `make skills-review`; a skill "
        f"quoting last month's procedure steers every session that loads it"
    )


def check_ignored_tests():
    """Every shipped `#[ignore]` still carries a rationale.

    RELEASE.md's zero-ignore gate: an ignored test is a known failure pulled out of CI,
    so "all green" means less than it looks.  The machine can check that the set is
    small and every entry gives a reason; whether each reason is still ACCEPTABLE is the
    owner's sign-off (M-ignores), and no script can do that half.
    """
    p = os.path.join(ROOT, "tests", "ignored_tests.baseline")
    if not os.path.isfile(p):
        return UNKNOWN, "tests/ignored_tests.baseline is missing"
    entries = []
    with open(p, encoding="utf-8") as f:
        for line in f:
            if line.strip() and not line.startswith("#"):
                entries.append(line.rstrip("\n"))
    bare = [e.split("\t")[0] for e in entries if "\t" not in e or not e.split("\t", 1)[1].strip()]
    if bare:
        return FAIL, "ignored with no rationale: " + ", ".join(bare)
    names = [e.split("\t")[0] for e in entries]
    if not names:
        return OK, "no tests ship ignored"
    return OK, f"{len(names)} ignored, each with a rationale: " + ", ".join(names)


def check_open_deviations():
    """No deviation a release can close ships in one.

    The formal registers (`doc/claude/formal/`) record where the code disobeys its rules.  The
    owner's standing is that every such deviation a release CAN resolve is resolved before it
    ships; the only open entries a release may carry are those whose head says `not resolvable
    in a release`, with the reason in the entry (D-op-1/2: two backends, no executable semantics
    linking them).  Read through `rule_tags.open_register`, the same home the `registers` report
    reads, so the gate and the report cannot disagree about what is open.
    """
    import importlib.util

    spec = importlib.util.spec_from_file_location(
        "rule_tags", os.path.join(ROOT, "scripts", "rule_tags.py"))
    rule_tags = importlib.util.module_from_spec(spec)
    try:
        spec.loader.exec_module(rule_tags)
        live, _drift, _regs = rule_tags.open_register()
    except Exception as e:  # a parser failure is an unknown, never a pass
        return UNKNOWN, f"rule_tags.open_register failed: {e}"
    resolvable = [(f, t, iss) for f, t, iss, unres in live if not unres]
    if not resolvable:
        return OK, f"{len(live)} open, all marked not resolvable in a release"
    shown = ", ".join(
        f"{t} ({', '.join('loft#' + n for n in iss) if iss else 'NO ISSUE'})"
        for _f, t, iss in resolvable)
    return FAIL, f"{len(resolvable)} open deviation(s) a release can resolve: {shown}"


def check_prev_release_in_registry(version: str, network: bool):
    """Did the release before this one reach the signed index?

    Asked with THIS release's version rather than Cargo.toml's, because the checklist is
    read before the bump: left to default, the gate sees a tree still carrying the
    released version, answers "nothing to gate", and renders as a tick over a question
    nobody asked.  That is the whole window in which the answer matters.
    """
    if not network:
        return UNKNOWN, "skipped (--no-network)"
    code, out = sh(
        sys.executable,
        os.path.join(ROOT, "scripts", "check-release-published.py"),
        "--version",
        version,
    )
    lines = [l for l in out.splitlines() if l.strip()]
    if code == 0:
        return OK, lines[-1] if lines else "previous release is in the index"
    # `fail()` writes one `::error title=T::body` line, then the body's later lines.
    first = lines[0] if lines else "check-release-published.py failed"
    m = re.match(r"::error title=([^:]+)::(.*)", first)
    return FAIL, f"{m.group(1)} — {m.group(2)}" if m else first


INDEX_URL = os.environ.get(
    "LOFT_REGISTRY_INDEX",
    "https://raw.githubusercontent.com/loft-lang/registry/main/index.json",
)


def check_this_release_in_registry(version: str, network: bool):
    """THIS release took effect in the signed index — the step-4 'ran AND took effect'
    gate (@PLN156 phase 1).

    Asked against the index directly, not through `loft self-update`'s output, because
    that output cannot carry the question: its `Current` verdict prints the RUNNING
    version whether the index's newest equals it or merely trails it, so a forgotten
    splice reads identically to a landed one from the CLI alone.  Here the entry, every
    published triple's binary, and each binary's `manifest_sha256` (what `verify-self`
    anchors an INSTALLED tree by) are required by name.
    """
    if not network:
        return UNKNOWN, "skipped (--no-network)"
    import urllib.request

    try:
        with urllib.request.urlopen(INDEX_URL, timeout=30) as r:
            index = json.load(r)
    except Exception as e:  # noqa: BLE001 — any transport/parse failure is UNKNOWN
        return UNKNOWN, f"could not fetch the signed index: {e}"
    versions = index.get("packages", {}).get("loft", {}).get("versions", {})
    entry = versions.get(version)
    if entry is None:
        have = ", ".join(sorted(versions)) or "none"
        return FAIL, (
            f"loft {version} is NOT in the signed index (it carries: {have}) — the "
            "registry splice has not landed; `self-update` resolves nothing for it "
            "and no installation of it can ever anchor (RELEASE.md step 4)"
        )
    try:
        triples = published_triples()
    except (RuntimeError, SystemExit) as e:
        return UNKNOWN, f"cannot read PUBLISHED_TRIPLES ({e})"
    binaries = entry.get("binaries", {})
    missing = [t for t in triples if t not in binaries]
    if missing:
        return FAIL, f"the entry lacks binaries for: {', '.join(missing)}"
    unanchored = [t for t in triples if not binaries[t].get("manifest_sha256")]
    if unanchored:
        return FAIL, (
            "no manifest_sha256 on: " + ", ".join(unanchored)
            + " — installations from these bundles can never anchor to the signature"
        )
    return OK, f"{version} in the signed index, {len(triples)} binaries, each anchored"


def local_loft_binary() -> str:
    """The freshest locally built loft, release preferred."""
    candidates = [
        os.path.join(ROOT, "target", "release", "loft"),
        os.path.join(ROOT, "target", "debug", "loft"),
    ]
    have = [p for p in candidates if os.path.isfile(p)]
    if not have:
        return ""
    return max(have, key=os.path.getmtime)


def check_selfupdate_resolves(version: str, network: bool):
    """The command RELEASE.md step 4's postscript gives, run and read instead of
    advised (@PLN156 phase 1): `loft self-update --dry-run --refresh` must RESOLVE.

    The one output this must never let through quietly is `no releases published to
    compare against` — a cache predating the splice prints it in the same words as an
    empty index, and it exits 0 either way.  Content (is THIS version the entry, with
    every triple anchored) is A-registry-this' half; this half proves the user-facing
    resolver — signature verification, trust roots, cache refresh — reaches it.
    """
    if not network:
        return UNKNOWN, "skipped (--no-network)"
    loft = local_loft_binary()
    if not loft:
        return UNKNOWN, "no built loft (target/release or target/debug) — cargo build first"
    code, out = sh(loft, "self-update", "--dry-run", "--refresh", timeout=120)
    if code != 0:
        last = out.splitlines()[-1] if out else str(code)
        return FAIL, f"self-update --dry-run --refresh exited {code}: {last}"
    if "no releases published to compare against" in out:
        return FAIL, (
            "the signed index resolves NOTHING for this binary — the splice has not "
            "landed (or a stale cache survived --refresh); the step-4 omission this "
            "gate exists for"
        )
    if "is the newest release" in out or "is available" in out:
        line = next(
            (l.strip() for l in out.splitlines()
             if "newest release" in l or "is available" in l),
            "resolved",
        )
        return OK, line
    if "not built for" in out:
        return FAIL, "the index has a release but no build for this host"
    return UNKNOWN, "self-update output matched no known verdict — read it by hand"


def check_acquisition(version: str, network: bool):
    """The whole acquisition chain, end to end (@PLN156 phase 2): install.sh over the
    real transport, --version match, signed-index resolution, the verify-self ANCHOR
    line, and a program executed.  scripts/acquisition-chain.sh is the instrument; its
    exit 3 (release not downloadable yet) is UNKNOWN here, because 'could not run' and
    'ran and failed' are the two answers a release must never confuse.
    """
    if not network:
        return UNKNOWN, "skipped (--no-network)"
    code, out = sh(
        "sh", os.path.join(ROOT, "scripts", "acquisition-chain.sh"),
        "--version", version, timeout=600,
    )
    last = out.splitlines()[-1].strip() if out else str(code)
    if code == 0:
        return OK, last
    if code == 3:
        return UNKNOWN, "release not acquirable yet (publish first) — " + last
    return FAIL, last


def check_validator_dryrun(version: str, network: bool):
    """The registry's OWN validator, run against this release's entry BEFORE the splice
    is submitted (@PLN156 phase 3) — the rehearsal that would have exposed 2026.8.0's
    rejection months early.  Exit 3 (structural-only: no published assets yet) is
    UNKNOWN: a structural pass is not the full verdict and must not render as one.
    Exit 4 (already in the live index) is a pass — the live validator covered it.
    """
    if not network:
        return UNKNOWN, "skipped (--no-network)"
    code, out = sh(
        sys.executable, os.path.join(ROOT, "scripts", "validator-dryrun.py"),
        "--version", version, timeout=1800,
    )
    last = out.splitlines()[-1].strip() if out else str(code)
    if code == 0:
        return OK, last
    if code == 4:
        return OK, last
    if code == 3:
        return UNKNOWN, last
    return FAIL, last


def check_draft_assets(version: str, network: bool):
    """The draft the tag built: are all ten assets on it?

    Named individually rather than counted: "10 assets" is true of a draft missing
    windows and carrying two source archives.
    """
    if not network:
        return UNKNOWN, "skipped (--no-network)"
    code, out = sh(
        "gh", "release", "view", f"v{version}", "--json", "assets,isDraft", timeout=90
    )
    if code != 0:
        return UNKNOWN, f"no release v{version} yet (push the tag first)"
    try:
        data = json.loads(out)
    except json.JSONDecodeError:
        return UNKNOWN, "could not parse `gh release view`"
    have = {a["name"] for a in data.get("assets", [])}
    want = [f"loft-{version}-src.zip", f"loft-{version}-registry-entry.json"]
    try:
        triples = published_triples()
    except (RuntimeError, SystemExit) as e:
        return UNKNOWN, f"cannot read PUBLISHED_TRIPLES ({e})"
    if not triples:
        return UNKNOWN, "PUBLISHED_TRIPLES is empty — nothing to expect"
    want += [f"loft-{version}-{t}.zip" for t in triples]
    missing = [w for w in want if w not in have]
    if missing:
        return FAIL, "draft is missing: " + ", ".join(missing)
    draft = "draft" if data.get("isDraft") else "PUBLISHED"
    return OK, f"{len(want)} expected assets present ({draft})"


def published_triples() -> list[str]:
    """The triples a release publishes, through `gen-toolchain-entry.py`'s parser.

    That script already reads `self_update::PUBLISHED_TRIPLES` -- the list the running
    binary matches its host against -- and it is the one the release entry is built from.
    Re-implementing the read here would give the checklist its own idea of which bundles
    to expect, and the first thing a second copy did was return an EMPTY list from a
    regex that missed the `&[&str]`, which made the draft-assets check pass while
    verifying nothing.  A restated predicate that can go quiet is worse than no check.
    """
    import importlib.util

    path = os.path.join(ROOT, "scripts", "gen-toolchain-entry.py")
    spec = importlib.util.spec_from_file_location("gen_toolchain_entry", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load {path}")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.published_triples()


def check_smoke_ran(version: str, network: bool):
    """Did the bundle smoke actually RUN on all four legs, or did one skip?

    The Rosetta skip exits 0 -- correctly, since no release should be blocked by a runner
    image -- so the leg's conclusion is `success` either way.  The distinction lives in
    the warning annotation, which is why this reads annotations rather than conclusions.
    """
    if not network:
        return UNKNOWN, "skipped (--no-network)"
    code, out = sh(
        "gh", "run", "list", "--workflow", "release.yml", "--branch", f"v{version}",
        "--json", "databaseId,conclusion", "--limit", "1", timeout=90,
    )
    if code != 0 or not out.strip():
        return UNKNOWN, "no release.yml run for this tag yet"
    try:
        runs = json.loads(out)
    except json.JSONDecodeError:
        return UNKNOWN, "could not parse `gh run list`"
    if not runs:
        return UNKNOWN, "no release.yml run for this tag yet"
    run_id = runs[0]["databaseId"]
    code, out = sh(
        "gh", "api", f"repos/{REPO}/actions/runs/{run_id}/jobs", "--paginate",
        timeout=90,
    )
    if code != 0:
        return UNKNOWN, "could not read the run's jobs"
    try:
        jobs = json.loads(out).get("jobs", [])
    except json.JSONDecodeError:
        return UNKNOWN, "could not parse the run's jobs"
    build = [j for j in jobs if j["name"].startswith("Build ")]
    if not build:
        return UNKNOWN, "the run has no build legs yet"
    ran, skipped, failed, absent = [], [], [], []
    for j in build:
        leg = j["name"].removeprefix("Build ")
        step = next(
            (s for s in j.get("steps", []) if s["name"] == "Smoke-test the bundle"), None
        )
        # No such step at all: this tag was built before the smoke existed.  Reporting
        # that as a failure is a false red, and a check that is red for a reason nobody
        # can act on is one everybody learns to scroll past.
        if step is None:
            absent.append(leg)
            continue
        if step.get("conclusion") != "success":
            failed.append(leg)
            continue
        ac, ao = sh("gh", "api", f"repos/{REPO}/check-runs/{j['id']}/annotations")
        if ac == 0 and "Bundle smoke skipped" in ao:
            skipped.append(leg)
        else:
            ran.append(leg)
    if len(absent) == len(build):
        return UNKNOWN, (
            f"this run has no bundle-smoke step — v{version} was built before it "
            "existed, so its bundles were never executed in CI"
        )
    if absent:
        return FAIL, "no smoke step on: " + ", ".join(absent)
    if failed:
        return FAIL, "smoke did not pass on: " + ", ".join(failed)
    if skipped:
        return FAIL, (
            "smoke SKIPPED on " + ", ".join(skipped)
            + " — those bundles were never executed; smoke them by hand (M-rosetta)"
        )
    return OK, f"bundle smoke ran and passed on all {len(ran)} legs"


_GATE_RUN: dict = {}   # memo: HEAD sha -> (state, evidence, run or None)
_GATE_JOBS: dict = {}  # memo: run id -> list of jobs, or None when unreadable


def gate_run(network: bool):
    """The newest completed `release-gate.yml` run for HEAD's commit: (state, evidence, run).

    Keyed by COMMIT on purpose: the release evidence RELEASE.md asks for is a run on the
    tag candidate, and a run on any other commit -- last night's `main`, the branch before
    its final fix -- is not that, however green.  A run still in flight is UNKNOWN, not a
    pass, for the same reason a check that could not run is.
    """
    if not network:
        return UNKNOWN, "skipped (--no-network)", None
    code, sha = sh("git", "rev-parse", "HEAD")
    if code != 0:
        return UNKNOWN, "could not read HEAD", None
    if sha in _GATE_RUN:
        return _GATE_RUN[sha]
    short = sha[:12]

    def memo(st, why, run=None):
        _GATE_RUN[sha] = (st, why, run)
        return _GATE_RUN[sha]

    code, out = sh(
        "gh", "run", "list", "--workflow", "release-gate.yml", "--commit", sha,
        "--json", "databaseId,conclusion,status,createdAt,url", "--limit", "10",
        timeout=90,
    )
    if code != 0:
        if "404" in out:
            # A dispatchable workflow has to be on the default branch; until this one has
            # merged, GitHub answers as if it did not exist.
            return memo(UNKNOWN, "release-gate.yml is not on GitHub's default branch yet — merge it, then `make release-gate`")
        return memo(UNKNOWN, f"could not list release-gate runs: {out.splitlines()[-1] if out else code}")
    try:
        runs = json.loads(out or "[]")
    except json.JSONDecodeError:
        return memo(UNKNOWN, "could not parse `gh run list`")
    if not runs:
        return memo(UNKNOWN, f"no release-gate run for {short} — `make release-gate` (the branch must be pushed)")
    run = max(runs, key=lambda r: r.get("createdAt", ""))
    when = run.get("createdAt", "")[:16].replace("T", " ")
    if run.get("status") != "completed":
        return memo(UNKNOWN, f"run for {short} still {run.get('status')} (started {when}) — {run.get('url')}", run)
    if run.get("conclusion") == "success":
        return memo(OK, f"GREEN for {short} at {when} — {run.get('url')}", run)
    return memo(FAIL, f"{run.get('conclusion')} for {short} at {when} — {run.get('url')}", run)


def gate_jobs(run) -> list | None:
    """Every job of a release-gate run, named `<leg> / <job>` (the `verdict` job is bare)."""
    rid = run.get("databaseId")
    if rid in _GATE_JOBS:
        return _GATE_JOBS[rid]
    code, out = sh(
        "gh", "api", f"repos/{REPO}/actions/runs/{rid}/jobs", "--paginate",
        "--jq", ".jobs[] | {name, conclusion}", timeout=90,
    )
    jobs = None
    if code == 0:
        try:
            jobs = [json.loads(line) for line in out.splitlines() if line.strip()]
        except json.JSONDecodeError:
            jobs = None
    _GATE_JOBS[rid] = jobs
    return jobs


def red_legs(jobs) -> dict[str, list[str]]:
    """leg -> the jobs of that leg that did not succeed (skipped excluded: a skipped job
    inside a leg is that leg's own `if:`, and the verdict already refuses a skipped LEG)."""
    out: dict[str, list[str]] = {}
    for j in jobs:
        name, concl = j.get("name", ""), j.get("conclusion")
        if " / " not in name or concl in ("success", "skipped", None):
            continue
        leg = name.split(" / ", 1)[0]
        out.setdefault(leg, []).append(f"{name.split(' / ', 1)[1]}={concl}")
    return out


def check_release_gate(network: bool):
    """The gate's verdict for HEAD's commit, read through the record's WAIVERS.

    A leg red for a reason outside the candidate -- a runner without a device, a package's
    own defect, a toolchain the verifier cannot compare -- is what RELEASE.md § The
    nightlies' table says is CLEARED by recording the reason.  Until 2026-09-23 nothing
    could record it, so the gate could only end red and the release proceeded on hand-run
    substitutes.  `--waive <leg> --note '<why>'` records the leg against the RUN it was red
    in; a waiver names one run, so the next run starts with none.
    """
    st, why, run = gate_run(network)
    if st != FAIL:
        return st, why
    jobs = gate_jobs(run)
    if jobs is None:
        return FAIL, why + " — the `verdict` job names the red legs (the run's jobs could not be read here)"
    red = red_legs(jobs)
    waivers = RECORD.get("_waivers", {}).get(str(run.get("databaseId")), {})
    unwaived = {leg: js for leg, js in red.items() if leg not in waivers}
    waived = [f"{leg} ({waivers[leg].get('note') or 'no reason recorded'})" for leg in red if leg in waivers]
    if not red:
        return FAIL, why + " — no leg reports a red job; open the run"
    if not unwaived:
        return OK, f"GREEN with waived legs for {run.get('databaseId')}: " + "; ".join(waived) + f" — {run.get('url')}"
    detail = "; ".join(f"{leg}: {', '.join(js)}" for leg, js in unwaived.items())
    tail = f"; waived: {', '.join(waived)}" if waived else ""
    return (
        FAIL,
        f"{why} — red legs {detail}{tail}.  Fix, or `--waive <leg> --note '<why>'` for a red "
        f"that is not the candidate's",
    )


def gate_leg_ok(prefixes: list[str], network: bool):
    """Did every gate job whose name starts with one of `prefixes` succeed on HEAD's
    commit?  (state, evidence) for a DERIVED manual row: OK satisfies the row."""
    st, why, run = gate_run(network)
    if run is None or st == UNKNOWN:
        return UNKNOWN, ""
    jobs = gate_jobs(run)
    if jobs is None:
        return UNKNOWN, ""
    hits = [j for j in jobs if any(j.get("name", "").startswith(p) for p in prefixes)]
    if not hits:
        return UNKNOWN, f"the release-gate run has no job named {' / '.join(prefixes)} — run it by hand"
    bad = [f"{j['name']}={j.get('conclusion')}" for j in hits if j.get("conclusion") != "success"]
    if bad:
        return FAIL, "the gate's own job is not green on this commit: " + ", ".join(bad)
    return OK, f"{', '.join(j['name'] for j in hits)} green in release-gate run {run.get('databaseId')} on this commit"


def check_cargo_audit():
    """RUSTSEC advisories over `Cargo.lock`, on this box.  The nightly `audit` job in
    `miri.yml` asks the same question on the schedule; this asks it of THIS tree."""
    code, out = sh("cargo", "audit", "--version", timeout=30)
    if code != 0:
        return UNKNOWN, "cargo-audit is not installed — `cargo install cargo-audit --locked`"
    code, out = sh("cargo", "audit", timeout=300)
    # The report is blank-line-separated blocks; a `Warning:` block is an unmaintained or
    # yanked crate (informational, exit 0), any other `Crate:` block is a vulnerability.
    vulns, warnings = [], 0
    for block in re.split(r"\n\s*\n", out):
        if "Crate:" not in block:
            continue
        if "Warning:" in block:
            warnings += 1
            continue
        crate = re.search(r"^Crate:\s*(\S+)", block, re.M)
        advisory = re.search(r"^ID:\s*(\S+)", block, re.M)
        vulns.append(f"{crate.group(1) if crate else '?'} ({advisory.group(1) if advisory else '?'})")
    if code == 0 and not vulns:
        return OK, "no vulnerable crate in Cargo.lock" + (f" ({warnings} unmaintained/informational warning(s))" if warnings else "")
    if not vulns:
        return UNKNOWN, "cargo audit could not answer: " + (out.splitlines()[-1] if out else f"exit {code}")
    return FAIL, f"{len(vulns)} vulnerable crate(s): " + ", ".join(vulns) + " — `cargo update -p <crate>`, or record why it cannot move"


def changed_since_last_tag(version: str, paths: list[str]) -> bool:
    """Did any of `paths` change since the previous release?

    What makes the per-release editor / native-debug rituals worth their cost is that the
    code under them moved.  When it did not, re-running them proves what the last release
    already proved.
    """
    prev = previous_tag(version)
    if not prev:
        return True  # no previous tag to compare against: assume it needs doing
    code, out = sh("git", "diff", "--name-only", f"{prev}..HEAD", "--", *paths)
    return bool(out.strip()) if code == 0 else True


# --------------------------------------------------------------------------------------


def build_items(version: str, network: bool) -> list[tuple[str, list[Item]]]:
    prev_tag_paths_editor = ["editors/vscode"]
    prev_tag_paths_debug = [
        "src/debugger.rs",
        "src/bin/loft-dap.rs",
        "src/generation",
        "editors/vscode",
    ]
    editor_touched = changed_since_last_tag(version, prev_tag_paths_editor)
    debug_touched = changed_since_last_tag(version, prev_tag_paths_debug)
    # The bundle smoke is read ONCE: `A-smoke` reports it, and `M-rosetta` exists only
    # for a leg it reports as skipped (with no network it cannot say, so the row stays).
    smoke = check_smoke_ran(version, network)
    smoke_skipped = smoke[0] != OK

    before = [
        Item(
            "A-version",
            "Cargo.toml names a version that is not yet tagged",
            "edit Cargo.toml",
            check=lambda: check_version_untagged(version),
        ),
        Item(
            "A-changelog",
            "CHANGELOG.md has this release's section",
            "write it",
            check=lambda: check_changelog(version, "CHANGELOG.md", "CHANGELOG.md", True),
            cadence="pre",
        ),
        Item(
            "A-changelog-tech",
            "CHANGELOG_TECHNICAL.md gained this cycle's entries",
            "write it",
            check=lambda: check_changelog(
                version,
                "doc/claude/CHANGELOG_TECHNICAL.md",
                "CHANGELOG_TECHNICAL.md",
                False,
            ),
            cadence="pre",
        ),
        Item(
            "A-clean",
            "Working tree is clean",
            "commit or stash",
            check=check_tree_clean,
            cadence="mid pre",
        ),
        Item(
            "A-main",
            "HEAD contains origin/main",
            "git fetch && git rebase origin/main",
            check=check_head_on_main,
            cadence="mid pre",
        ),
        Item(
            "A-ci",
            "`make ci` is green ON THIS TREE",
            "make ci",
            check=check_ci_verdict,
            cadence="mid pre",
        ),
        Item(
            "A-registry-prev",
            "The PREVIOUS release reached the signed registry index",
            "scripts/check-release-published.py",
            check=lambda: check_prev_release_in_registry(version, network),
            cadence="mid pre",
        ),
        Item(
            "A-pdf",
            "The reference PDF is current (it ships in every bundle)",
            "cargo run --bin gendoc && make pdf",
            check=check_reference_pdf,
            cadence="pre",
        ),
        Item(
            "A-pdf-version",
            "The reference PDF says it is THIS release",
            "cargo run --bin gendoc && make pdf",
            check=lambda: check_reference_pdf_version(version),
            cadence="pre",
        ),
        Item(
            "A-pdf-content",
            "The reference's CONTENT is whole — every chapter, not just a fresh build",
            "cargo run --bin gendoc && make pdf",
            check=check_reference_pdf_content,
            cadence="pre",
        ),
        # The two watermark reviews are REPORTS: any commit touching a chapter's source
        # re-arms them, so on an active tree they read red most days by construction, and
        # RELEASE.md § What forces a release says docs are never release-coupled.  They are
        # listed so the count is seen, and tallied apart so it never reads as a blocker.
        Item(
            "A-reference-review",
            "Every reference chapter has been read against the shipped language",
            "make reference-review",
            check=check_reference_review,
            cadence="mid pre",
            report=True,
        ),
        Item(
            "A-skills-review",
            "Every agent skill has been read against the tree (content/usability/conciseness)",
            "make skills-review",
            check=check_skills_review,
            cadence="mid pre",
            report=True,
        ),
        Item(
            "A-ignores",
            "Every shipped `#[ignore]` carries a rationale",
            "tests/ignored_tests.baseline",
            check=check_ignored_tests,
            cadence="mid pre",
        ),
        Item(
            "A-audit",
            "No RUSTSEC advisory against a crate in Cargo.lock",
            "cargo audit   # then `cargo update -p <crate>`",
            check=check_cargo_audit,
            cadence="mid pre",
        ),
        Item(
            "M-valgrind",
            "Valgrind-clean on the TAG CANDIDATE",
            "scripts/valgrind-sweep.sh   # interpreter + native, every script and document",
            "GREEN — no invalid access and nothing definitely lost on either backend.  A "
            "possibly-lost record is Rust's interior pointers, not a leak; a leaked STORE "
            "fails the wrap suite itself (TESTING.md § Occasional valgrind pass).  Satisfied "
            "by the gate's own valgrind job on this commit",
            cadence="cand pre",
            derived=lambda: gate_leg_ok(["miri.yml / Valgrind memcheck sweep"], network),
        ),
        # `M-leaks` retired 2026-09-23: it re-ran by hand what the wrap suite hard-fails on
        # every `make ci` and inside the gate — a `tests/scripts` file that leaves a store
        # unfreed, `SCRIPTS_LEAK_ALLOW` empty — with the two `par` scripts in that corpus.
        # A manual re-run of a suite assertion is a row that gets ticked, not a gate.
        Item(
            "M-ignores",
            "Owner sign-off on every ignore AND every skip-list entry",
            "read tests/ignored_tests.baseline, then grep SKIP / NATIVE_SKIP / "
            "SCRIPTS_NATIVE_SKIP / ignored_scripts() in tests/",
            "each traces to a named open blocker.  `A-ignores` checks the rationales "
            "exist; whether they are still acceptable is a judgement",
            cadence="mid pre",
        ),
        Item(
            "M-wasm",
            "The WASM endpoint works — build, runtime, and gallery",
            "make wasm-html-test && make gallery, then open doc/gallery.html",
            "RELEASE.md § WASM endpoint: the browser bundle is how most users meet "
            "loft.  All examples load with NO console errors.  Satisfied by the gate's "
            "`Browser build + probe` job (gallery render check + the brick-buster "
            "console-error test) and the wasm node bridge on this commit",
            cadence="cand pre",
            derived=lambda: gate_leg_ok(
                ["ci.yml / Browser build + probe", "ci.yml / wasm node bridge"], network
            ),
        ),
        Item(
            "M-docs-review",
            "Pre-release documentation review (RELEASE.md steps 1-4 + 8)",
            "load the doc-quality skill first, then walk the steps; step 8 is "
            "`make clippy-review`",
            "stale problem docs removed, code links resolve, every doc reachable, "
            "clippy suppressions measured (dead ones named, live ones explained).  "
            "Steps 5-7 are deferred (2026-05-15)",
            cadence="mid pre",
            report=True,
        ),
        Item(
            "M-monthly-docs",
            "Monthly by-hand documentation review",
            "make libraries-review && make features-review",
            "which libraries owe a review or moved since their watermark — the "
            "monthly cadence makes this a per-release step",
            cadence="mid pre",
            report=True,
        ),
        Item(
            "M-monthly-bugs",
            "Monthly bug review — one rising class, one generalization",
            "make bug-review",
            "which mechanism classes still produce bugs, and whether last cycle's "
            "keystone moved its class",
            cadence="mid pre",
            report=True,
        ),
        Item(
            "M-ops-census",
            "Operator census — does anything still emit each bytecode operator?",
            "make ops-census",
            "an operator is cheap to add and invisible to retire: each one is an entry "
            "in the generated `fill::OPERATORS`, a `#rust` template the interpreter "
            "runs, and a template the native generator rewrites.  Read the two "
            "non-live verdicts.  UNEXERCISED (a site in `src/` emits it, no program in "
            "the tree does) is a test gap and usually the more actionable half — live "
            "emitter, zero coverage.  ORPHAN (never emitted, nothing in `src/` names "
            "it) is a retirement CANDIDATE, never a verdict: confirm before acting, "
            "since the census reads this tree and not a consumer's.  Not a way to free "
            "slots — the table holds 511 and is nowhere near full "
            "(RELEASE.md § The operator census)",
            cadence="mid pre",
            report=True,
        ),
        Item(
            "M-perf-pass",
            "Performance pass — loft AND its libraries, routines pull their weight",
            "make speed; per library: python3 bench/compare.py   # the drawing library's "
            "bench/ is the model (loft#1426); @PLN158 adopts it as the standard",
            "the BAR is gated elsewhere and this pass does not gate it twice: "
            "`scripts/native_ratio.sh` fails a ratio over `bench/ratio_oracle.tsv` in `make "
            "ci`, and an open `D-perf-*` deviation blocks through `A-deviations` — so this is "
            "the READ of what is left over the bar and why.  "
            "The formal contract is formal/performance.md (Perf-Like / Perf-Weight / "
            "Perf-Twin / Perf-Cure): every routine within the bar of its industry-language reference "
            "twin, hashes agreeing across lanes (lanes that disagree are not one "
            "algorithm), and a missing twin is WRITTEN where a hit is expected.  Verify "
            "attribution with the PROFILER, not by eye: `LOFT_PROFILE=1 loft --interpret "
            "bench.loft` with `LOFT_NO_NATIVE_LIBS=1` for library routines (a used library "
            "is a cdylib the sampler cannot enter), `make profile PROFILE_FLAGS=--engine` "
            "for the native side — a slow routine whose profile matches its reference's hot "
            "loop is an engine-class finding (file it, like loft#1426), not a library bug",
            cadence="mid pre",
            report=True,
        ),
        Item(
            "M-file-sizes",
            "Does each doc and source file hold ONE subject, at a length someone can use?",
            "make file-sizes",
            "two readings of one question.  SIZE: length alone is not the defect "
            "(DOC_QUALITY.md rule 4 judges by content — a 70-line module header can be "
            "right), so the split signal decides — a long file whose largest section is "
            "a small share of it holds several comparable subjects and wants splitting "
            "per subject, while one long section is a single subject that is merely long "
            "and should be left alone.  HISTORY: a contract doc that has absorbed its own "
            "change history is holding two subjects, and the second belongs in an "
            "`-history.md` companion — the formal docs are where this concentrates, and a "
            "companion EXISTING does not mean the history moved into it, so read the share "
            "and not the `yes`.  Split what a reader cannot navigate",
            cadence="mid pre",
            report=True,
        ),
        Item(
            "M-liveness",
            "The liveness census — are the gates themselves still live?",
            "make release-liveness",
            "read the report: ignored/skip rationales pointing at CLOSED issues, gates "
            "that have not actually fired recently (with the last fortnight's per-leg "
            "tally — a leg red ten nights in fourteen is what blocks the release gate), "
            "checklist items never run in any recorded cycle.  2026.8.0's rescue was this "
            "census done by hand, once; drift surfaces continuously only if it is read per "
            "release (@PLN156)",
            cadence="mid pre",
            report=True,
        ),
        Item(
            "A-deviations",
            "No open formal deviation a release can resolve",
            "python3 scripts/rule_tags.py registers --issues",
            check=check_open_deviations,
            cadence="pre",
        ),
        Item(
            "M-falsify-receipts",
            "Can each guard still be re-validated, and quickly?",
            "make falsify-review",
            "a recorded patch reintroducing THE defect cannot be gated — an apply, a build "
            "and a run per guard, and the verdict is a judgement about which channel moved "
            "— so it is a read, here, once a cycle.  Score the patch receipts with the "
            "instrument each names, and treat a control that went unreachable since last "
            "cycle as the inflow rate: that number is what says whether recording a patch "
            "at falsification time is taking",
            cadence="mid pre",
            report=True,
        ),
        Item(
            "M-close-plans",
            "Close the plans this release shipped",
            "scripts/close-shipped-plans.sh --range <prev-tag>..HEAD",
            "a plan that shipped and stayed open is one nobody can trust the status of",
            cadence="mid pre",
        ),
        Item(
            "M-changelog-read",
            "Read CHANGELOG.md's top section and confirm it describes THIS release",
            "less CHANGELOG.md",
            "it names what changed, in a user's words, with nothing from the last cycle "
            "left standing as if it were new",
            cadence="pre",
        ),
        Item(
            "M-libs",
            "The shipped libraries still build against this tree",
            "scripts/revalidate_libs_local.sh",
            "every library green — `make ci` says nothing about them.  Satisfied by the "
            "gate's `revalidate-libs.yml / gate` job on this commit",
            cadence="mid pre",
            derived=lambda: gate_leg_ok(["revalidate-libs.yml / gate"], network),
        ),
    ]

    # Every nightly, run deliberately against THIS commit in one CI run with one verdict
    # (`release-gate.yml`).  This used to be six manual items, one per nightly, each
    # dispatched by hand and ticked by a person; keyed by HEAD's commit it is now
    # measured, and a run on any other commit does not count (RELEASE.md § The nightlies).
    nightly_items = [
        Item(
            "A-release-gate",
            "The release gate is GREEN on this commit (every nightly, one run, one verdict)",
            "make release-gate    # dispatches release-gate.yml on the pushed branch and waits",
            check=lambda: check_release_gate(network),
            cadence="pre",
        ),
    ]

    after_tag = [
        Item(
            "A-draft",
            "The draft carries every expected asset",
            f"gh release view v{version}",
            check=lambda: check_draft_assets(version, network),
        ),
        # The per-platform hands-on walkthrough IS this row: each `release.yml` leg unpacks
        # its own zip, asserts `--version`, `verify-self` and every shipped example with
        # empty stderr, and the owner ruled on 2026-09-05 that the bundle smoke is the
        # walkthrough.  `M-hands-linux/-macos/-windows` were retired into it 2026-09-23;
        # what they added — Gatekeeper on an unsigned download, the VS Code grammar
        # symlink — no runner observes and no release has recorded.
        Item(
            "A-smoke",
            "The bundle smoke RAN (did not skip) on every leg — the hands-on walkthrough",
            f"gh run list --workflow release.yml --branch v{version}",
            check=lambda: smoke,
        ),
        Item(
            "M-rosetta",
            "Any bundle the smoke SKIPPED, run by hand",
            "unzip the bundle A-smoke names, then: bin/loft --version && "
            "bin/loft verify-self && bin/loft --interpret examples/*.loft",
            "listed only while A-smoke reports a skip (or cannot read the run); an "
            "unexecuted bundle is the one least likely to work",
            applies=smoke_skipped,
        ),
    ]

    before_publish = [
        Item(
            "M-install-sh",
            "`scripts/install.sh` end-to-end on one platform",
            "sh scripts/install.sh --version " + version,
            "the documented curl|sh path.  CI runs it against a locally built bundle over "
            "file:// (tests/self_update_swap.rs); this run adds the real transport and the "
            "SHIPPED binary's verify-self — read its verdict, the script exits 1 on a "
            "failed one",
        ),
        Item(
            "M-vscode",
            "VS Code extension packages and loads",
            "cd editors/vscode && vsce package, then install the .vsix",
            "listed because editors/vscode changed since the last tag",
            applies=editor_touched,
            cadence="pre",
        ),
        Item(
            "M-ndb",
            "Native-debug gate (gdb / lldb / objdump DWARF)",
            "see doc/claude/plans/34-native-debug/",
            "listed because the debugger / codegen paths changed since the last tag",
            applies=debug_touched,
            cadence="pre",
        ),
        Item(
            "M-publish",
            "Click Publish on the reviewed draft",
            f"gh release view v{version} --web",
            "the click FREEZES the assets — nothing can be added afterwards",
        ),
    ]

    after_publish = [
        Item(
            "A-validator-dryrun",
            "The registry's OWN validator accepts this release's entry (rehearsal of the splice)",
            f"scripts/validator-dryrun.py --version {version}",
            check=lambda: check_validator_dryrun(version, network),
            cadence="pre",
        ),
        Item(
            "M-registry-splice",
            "Splice the generated entry into the registry index and re-sign",
            f"take loft-{version}-registry-entry.json from the published release into "
            "loft-lang/registry's index.json, then scripts/registry-sign.sh",
            "the ONLY step that puts these binaries under a signature.  Forgetting it "
            "is caught on the NEXT release, not by anyone noticing",
        ),
        Item(
            "A-registry-this",
            "THIS release took effect in the signed index (entry + every triple anchored)",
            "the splice landed and re-signed — measured off the index itself",
            check=lambda: check_this_release_in_registry(version, network),
        ),
        Item(
            "A-selfupdate-resolves",
            "`loft self-update --dry-run --refresh` RESOLVES against the signed index",
            "cargo build, then the command — the empty-index message is a FAIL here",
            check=lambda: check_selfupdate_resolves(version, network),
        ),
        Item(
            "A-acquisition",
            "The acquisition chain end-to-end: install.sh → version → resolve → ANCHOR → run",
            f"scripts/acquisition-chain.sh --version {version}",
            check=lambda: check_acquisition(version, network),
        ),
        Item(
            "M-self-update-win",
            "Windows: `loft self-update` from the PREVIOUS release to this one",
            "on a Windows box, install the previous release, then run loft self-update",
            "replacing a RUNNING executable is the one genuinely platform-divergent "
            "step in the chain, and no test can reach it (RELEASE.md § 10)",
        ),
        # M-install-live retired 2026-09-23: `scripts/acquisition-chain.sh` step 6 installs a
        # library with the binary it just acquired, into a fresh LOFT_HOME, against the live
        # index — the trust-root / signing-key skew question, measured by A-acquisition.
        # M-verify-anchored retired 2026-09 (@PLN156 phase 1): A-acquisition asserts the
        # anchor line itself, on an installation it just made over the real transport —
        # the same evidence, measured instead of promised.
        Item(
            "M-pages",
            "The deployed docs site boots in a browser",
            "open the Pages gallery.html and brick-buster.html, watch the console",
            "the release docs job rebuilds the wasm and deploys without loading it; a "
            "glue/wasm mismatch has shipped this way before",
        ),
    ]

    return [
        ("Before the tag", before + nightly_items),
        ("After pushing the tag — CI's own answers", after_tag),
        ("Before clicking Publish", before_publish),
        ("After publishing", after_publish),
    ]


def load_state(version: str) -> dict:
    p = state_path(version)
    if os.path.isfile(p):
        with open(p, encoding="utf-8") as f:
            return json.load(f)
    return {}


def save_state(version: str, state: dict) -> None:
    p = state_path(version)
    os.makedirs(os.path.dirname(p), exist_ok=True)
    with open(p, "w", encoding="utf-8") as f:
        json.dump(state, f, indent=2, sort_keys=True)
        f.write("\n")


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--version", help="release version (default: Cargo.toml's)")
    ap.add_argument("--done", metavar="ID", help="mark a manual item done (records HEAD's commit)")
    ap.add_argument("--undo", metavar="ID", help="un-mark a manual item")
    ap.add_argument("--note", default="", help="evidence to record with --done / --waive")
    ap.add_argument(
        "--waive",
        metavar="LEG",
        help="waive a red release-gate LEG (`ci.yml`, `repro-build.yml`, …) for the newest "
        "run on HEAD's commit — a red that is not the candidate's, with --note saying why",
    )
    ap.add_argument("--unwaive", metavar="LEG", help="withdraw a waiver for HEAD's run")
    ap.add_argument("--fetch", action="store_true", help="refresh origin/main + tags")
    ap.add_argument(
        "--no-network", action="store_true", help="skip every check that needs the net"
    )
    ap.add_argument(
        "--phase",
        choices=["mid", "pre"],
        help="show (and measure) only the items practical at this point of the cycle: "
        "'mid' = the halfway-point stability audit, 'pre' = the month's-last-days "
        "pre-work.  Without it, the full release-window list.",
    )
    ap.add_argument("--json", action="store_true", help="machine-readable output")
    args = ap.parse_args()

    version = args.version or cargo_version()
    network = not args.no_network

    if args.fetch:
        sh("git", "fetch", "--tags", "--quiet", "origin", timeout=120)

    state = load_state(version)
    global RECORD
    RECORD = state
    if args.waive or args.unwaive:
        # A waiver names ONE run — the newest completed release-gate run on HEAD's commit —
        # so the next run starts with none and a leg that stays red is re-justified each
        # time.  Stored beside the ticks under `_waivers`, which no item reads as a tick.
        st, why, run = gate_run(network=True)
        if run is None:
            print(f"no release-gate run to waive a leg of: {why}", file=sys.stderr)
            return 2
        rid = str(run.get("databaseId"))
        leg = args.waive or args.unwaive
        waivers = state.setdefault("_waivers", {}).setdefault(rid, {})
        if args.waive:
            if not args.note:
                print("--waive needs --note: the reason is the record", file=sys.stderr)
                return 2
            _, head = sh("git", "rev-parse", "HEAD")
            waivers[leg] = {
                "at": datetime.datetime.now().strftime("%Y-%m-%d %H:%M"),
                "commit": head,
                "note": args.note,
            }
        else:
            waivers.pop(leg, None)
        save_state(version, state)
    if args.done or args.undo:
        sections = build_items(version, network=False)
        ids = {i.id: i for _, items in sections for i in items}
        for flag, ident in (("--done", args.done), ("--undo", args.undo)):
            if not ident:
                continue
            if ident not in ids:
                print(f"no such item: {ident}", file=sys.stderr)
                return 2
            if ids[ident].automatic:
                print(
                    f"{ident} is measured, not ticked — it reports what the repo says.",
                    file=sys.stderr,
                )
                return 2
            if flag == "--done":
                _, head = sh("git", "rev-parse", "HEAD")
                state[ident] = {
                    "at": datetime.datetime.now().strftime("%Y-%m-%d %H:%M"),
                    "commit": head,
                    "note": args.note,
                }
            else:
                state.pop(ident, None)
        save_state(version, state)

    sections = build_items(version, network)
    early: list[tuple[str, list[Item]]] = []
    if args.phase:
        # The early views: only what is practical NOW is shown or measured, so a
        # mid-cycle audit does not run (or go red on) checks that need the tag, the
        # draft, or the published assets to exist.
        if args.phase == "mid":
            # Candidate-bound rows are worth RUNNING at halfway and cannot be
            # FINISHED there, so they are shown apart and counted nowhere.  Folding
            # them into the tally is what made the mid view unreachable by
            # construction; a reader who cannot finish the list stops reading it.
            early = [
                (name, [i for i in items if "cand" in i.cadence.split()])
                for name, items in sections
            ]
            early = [(name, items) for name, items in early if items]
        sections = [
            (name, [i for i in items if args.phase in i.cadence.split()])
            for name, items in sections
        ]
        sections = [(name, items) for name, items in sections if items]
    for _, items in sections + early:
        for item in items:
            item.resolve(state)

    # The evidence names the commit it was measured on: "these gates ran on this
    # commit", never "nothing was red" (@PLN156 phase 5).
    _, head = sh("git", "rev-parse", "HEAD")
    head = head[:12] if head else "?"

    if args.json:
        print(
            json.dumps(
                {
                    "version": version,
                    "commit": head,
                    "generated_at": datetime.datetime.now().strftime("%Y-%m-%d %H:%M"),
                    "phase": args.phase or "release",
                    "items": [
                        {
                            "id": i.id,
                            "section": name,
                            "title": i.title,
                            "state": i.state,
                            "automatic": i.automatic,
                            "report": i.report,
                            "cadence": i.cadence,
                            "evidence": i.evidence,
                        }
                        for name, items in sections + early
                        for i in items
                    ],
                },
                indent=2,
            )
        )
        return 0

    phase_label = {
        "mid": " — MID-CYCLE stability audit (every counted row below can be FINISHED now)",
        "pre": " — PRE-WORK for the month's last days",
    }.get(args.phase, "")
    print(f"Release checklist — loft {version}   (measured on {head}){phase_label}\n")
    if not args.phase:
        print(
            "  cadence: [mid] can be FINISHED at the cycle's halfway point (overall\n"
            "  stability) · [cand] worth running early, but its tick must name the tag\n"
            "  candidate · [pre] can be finished in the month's last days as pre-work ·\n"
            "  unmarked needs the release window itself.  `--phase mid|pre` works each\n"
            "  view; `--phase mid` lists the [cand] rows apart and counts them nowhere.\n"
            "  class: a GATE row must be true; a [report] row must be READ and never\n"
            "  blocks; a tick records its commit, and [~] is a [cand] tick made on a tree\n"
            "  whose shipped files have since moved.\n"
        )
    for name, items in sections:
        shown = [i for i in items if i.state != NA]
        hidden = len(items) - len(shown)
        section_head = f"## {name}"
        if hidden:
            section_head += f"   ({hidden} item(s) not applicable this release)"
        print(section_head)
        for i in shown:
            kind = "auto" if i.automatic else "    "
            tag = "[" + "+".join(i.cadence.split()) + "]" if i.cadence else ""
            title = i.title + ("  [report]" if i.report else "")
            print(f"  {MARK[i.state]} {kind}  {i.id:<20} {tag:<10} {title}")
            if i.evidence:
                print(f"                            {i.evidence}")
            elif not i.automatic:
                print(f"                            how:  {i.how}")
                if i.passes:
                    print(f"                            pass: {i.passes}")
        print()

    if early:
        # Shown because a halfway run of a sweep is genuine early warning; counted
        # nowhere because its evidence cannot name the tree that ships.
        print("## Early warning — worth running now, the TICK belongs on the candidate")
        for _, items in early:
            for i in items:
                if i.state == NA:
                    continue
                print(f"  {MARK[i.state]}        {i.id:<20} {'[cand]':<10} {i.title}")
                print(f"                            how:  {i.how}")
                if i.passes:
                    print(f"                            pass: {i.passes}")
        print("  (counted nowhere below — a mid-cycle result is warning, not evidence)")
        print()

    applicable = [i for _, items in sections for i in items if i.applies]
    auto = [i for i in applicable if i.automatic and not i.report]
    manual = [i for i in applicable if not i.automatic and not i.report]
    reports = [i for i in applicable if i.report]
    bad = [i for i in auto if i.state == FAIL]
    unknown = [i for i in auto if i.state == UNKNOWN]
    stale = [i for i in manual if i.state == STALE]
    left = [i for i in manual if i.state in (TODO, STALE)]
    unread = [i for i in reports if i.state in (TODO, FAIL, STALE)]

    print(
        f"{len(auto) - len(bad) - len(unknown)}/{len(auto)} automatic gates pass"
        f"{', ' + str(len(unknown)) + ' could not run' if unknown else ''}"
        f"{', ' + str(len(bad)) + ' FAILING' if bad else ''}."
    )
    print(
        f"{len(manual) - len(left)}/{len(manual)} manual gates done"
        f"{', ' + str(len(stale)) + ' STALE (ticked on another candidate)' if stale else ''}."
    )
    # Reports are tallied apart and never block: they must be READ, not be true.
    print(f"{len(reports) - len(unread)}/{len(reports)} reports read.")
    if bad:
        print("\nBlocking: " + ", ".join(i.id for i in bad))
    if stale:
        print("\nStale: " + ", ".join(i.id for i in stale) + " — re-run on this candidate")
    if unread:
        print("\nReports owing a read: " + ", ".join(i.id for i in unread))
    if unknown:
        # UNKNOWN never aggregates into green (@PLN156 phase 5): a check that could
        # not run and a check that passed are the two answers a release must never
        # confuse — several "done" things in 2026.8.0 had simply never been measured.
        print(
            "\nNot green: "
            + ", ".join(i.id for i in unknown)
            + " never ran — UNKNOWN is not a pass"
        )
    if left:
        print("\nNext manual step: " + left[0].id + " — " + left[0].title)
    if not bad and not left and not unknown:
        print("\nEvery gate on this list is answered." + (" Reports still owing a read are listed above." if unread else ""))
    print("\nTick a manual step:  scripts/release-checklist.py --done <ID> --note '...'")
    print("Waive a red gate leg:  scripts/release-checklist.py --waive <leg> --note '<why it is not the candidate's>'")
    # Exit: 1 = a measured gate FAILED; 3 = nothing failed but gates remain unmeasured
    # (UNKNOWN) — distinct so a caller can tell red from not-yet-evidence; 0 only when
    # every applicable automatic gate ran and passed.
    if bad:
        return 1
    return 3 if unknown else 0


if __name__ == "__main__":
    sys.exit(main())
