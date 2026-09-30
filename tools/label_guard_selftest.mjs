// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Holds `.github/scripts/label-guard.js` to real issue-body shapes.
//
// Every way that parser can be wrong is SILENT: the failure is "no label
// applied", which on GitHub is indistinguishable from "the filer did not
// answer".  Nothing else observes it — the workflow only runs on an issue event,
// against a body nobody wrote for a test.  So the cases live here.
//
//   node tools/label_guard_selftest.mjs      (or: make label-guard-test)

import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import path from "node:path";

const here = path.dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const { chooseLabels, CATEGORIES, applicable, nextReleaseConflicts } = require(
  path.join(here, "..", ".github", "scripts", "label-guard.js"),
);

let failed = 0;
const check = (name, body, want) => {
  const got = chooseLabels(body).sort();
  const expect = [...want].sort();
  const ok = got.length === expect.length && got.every((g, i) => g === expect[i]);
  if (!ok) {
    failed += 1;
    console.log(`  FAIL  ${name}\n        want [${expect}]\n        got  [${got}]`);
  } else {
    console.log(`  ok    ${name}`);
  }
};

// ---- the bug FORM, as GitHub renders it into the body -----------------------

// Both headings that contain "workaround", in the order the form emits them.
// The first is a free-text `render: rust` textarea; the second is the dropdown
// that actually sets the label.  A substring match returns the FIRST, which is
// how `wa:` came to be never applied from a form.
const FORM = `### Minimal reproducer

\`\`\`rust
v.len()
\`\`\`

### Workaround

\`\`\`rust
Bind the concat to a local first: w = v + [9]; w.len()
\`\`\`

### Is there a clean workaround?

wa:partial — a workaround exists but it's awkward / loses the intended behaviour

### Who hit it?

hit-by:loft — loft's own instruments: a gate, a sanitizer, a sweep, a follow-on

### Severity

sev:medium — wrong result / hang / miscompile in a real shape, but no corruption

### Area(s)

- [x] \`area:codegen\` — bytecode or native code generation
- [ ] \`area:parser\` — lexer / two-pass parser / type resolution / scope
- [x] \`area:native\` — the --native backend specifically
`;

check("form: every answer becomes its label", FORM, [
  "sev:medium", "wa:partial", "area:codegen", "area:native", "hit-by:loft",
]);

// `hit-by:` is REQUIRED and was enforced by nothing until 2026-08-19 — not the form
// (which had no field) and not the guard.  A blank is not neutral: a consumer filters
// `hit-by:<their project>`, so an unlabelled issue reads as "not established", never
// "nobody".  Both halves are pinned: the answer becomes the label, and leaving it
// unanswered leaves the category unset for the guard to ask about.
check(
  "form: an unanswered hit-by leaves the category unset",
  FORM.replace(/### Who hit it\?\n\nhit-by:loft[^\n]*\n/, "### Who hit it?\n\nNot sure\n"),
  ["sev:medium", "wa:partial", "area:codegen", "area:native"],
);

check(
  "form: an unchecked area box contributes nothing",
  FORM.replace("- [x] `area:native`", "- [ ] `area:native`"),
  ["sev:medium", "wa:partial", "area:codegen", "hit-by:loft"],
);

check(
  "form: an unanswered dropdown leaves its category unset",
  FORM.replace(/### Severity\n\nsev:medium[^\n]*\n/, "### Severity\n\nNot sure\n"),
  ["wa:partial", "area:codegen", "area:native", "hit-by:loft"],
);

// ---- freeform (`gh issue create` / the API, which ignores templates) ---------

// loft#805 as filed: prose, no tokens, no form.  The guard must ask rather than
// guess — this is the case the `### Triage` channel exists for.
const FREEFORM = `## Summary

Building loft from source fails on a machine without \`mold\` installed.

## What happened

\`\`\`
error: linking with \`cc\` failed
\`\`\`
`;
check("freeform: nothing to go on yields nothing", FREEFORM, []);

check(
  "freeform: a ### Triage block is the reporter's channel",
  FREEFORM + "\n### Triage\nsev:low\nwa:clean\narea:native\n",
  ["sev:low", "wa:clean", "area:native"],
);

check(
  "freeform: a Triage block may be fenced, as the guard's comment shows it",
  FREEFORM + "\n```\n### Triage\nsev:low\nwa:clean\narea:native\n```\n",
  ["sev:low", "wa:clean", "area:native"],
);

check(
  "freeform: a label that does not exist is not invented",
  FREEFORM + "\n### Triage\nsev:catastrophic\narea:bogus\nwa:none\n",
  ["wa:none"],
);

// ---- what must NOT become a label ------------------------------------------

// #626: an issue whose body QUOTED this guard's comment was labelled with all
// six mutually exclusive sev/wa tokens.  Ambiguity is not a choice.
check(
  "prose naming every severity picks none of them",
  "It could be sev:high, sev:medium or sev:low — I cannot tell.",
  [],
);

// The guard's own comment, quoted verbatim into a body.  Its example values are
// placeholders precisely so this applies nothing.
const COMMENT_BLOCK =
  "### Triage\n" +
  CATEGORIES.map(
    (c) => `${c.key}<${c.values.map((v) => v.slice(c.key.length)).join("|")}>`,
  ).join("\n");
check("the guard's own example applies nothing when quoted", COMMENT_BLOCK, []);

check(
  "a pasted log cannot contribute labels",
  "Here is the log:\n\n```\nwarning: sev:high in area:codegen\n```\n",
  [],
);

check("an empty body is not an error", "", []);


// ---- loft#1048: a filer's own label outranks a mention in prose --------------
//
// The issue was filed with `sev:medium` and came back also carrying `sev:high`,
// because its only `sev:` token sat in the heading "Why it is not `sev:high`".
// The prose channel cannot tell an assertion from its negation.  The cure is not
// to teach it to: a category the filer has ALREADY answered is not open.

const checkApplicable = (name, chosen, current, want) => {
  const got = applicable(chosen, current).sort();
  const expect = [...want].sort();
  const ok = got.length === expect.length && got.every((g, i) => g === expect[i]);
  if (!ok) {
    failed += 1;
    console.log(`  FAIL  ${name}\n        want [${expect}]\n        got  [${got}]`);
  } else {
    console.log(`  ok    ${name}`);
  }
};

checkApplicable(
  "a prose sev: does not override the sev: the filer set",
  ["sev:high", "area:packages"],
  ["sev:medium", "bug"],
  ["area:packages"],
);
checkApplicable(
  "the same label already present is still skipped",
  ["sev:medium"],
  ["sev:medium"],
  [],
);
checkApplicable(
  "an unanswered single-choice category is still filled",
  ["sev:high", "wa:clean"],
  ["bug"],
  ["sev:high", "wa:clean"],
);
checkApplicable(
  "area: is multi-valued, so a second one is added, not blocked",
  ["area:native"],
  ["area:packages"],
  ["area:native"],
);
checkApplicable(
  "hit-by: is single-choice like sev:, so the filer's stands",
  ["hit-by:loft"],
  ["hit-by:moros"],
  [],
);

// `next-release` parks work past a freeze; a DEFECT may never carry it.
const checkParked = (name, current, want) => {
  const got = nextReleaseConflicts(current);
  const ok = got.length === want.length && got.every((g, i) => g === want[i]);
  if (!ok) {
    failed += 1;
    console.log(`  FAIL  ${name}\n        want [${want}]\n        got  [${got}]`);
  } else {
    console.log(`  ok    ${name}`);
  }
};
checkParked("an enhancement may be parked", ["enhancement", "next-release"], []);
checkParked("no next-release, nothing to judge", ["bug", "sev:high"], []);
checkParked("a bug may not be parked", ["bug", "next-release"], ["bug"]);
checkParked("a severity makes it a defect", ["next-release", "sev:low", "area:parser"], ["sev:low"]);
checkParked("silent-wrong least of all", ["next-release", "silent-wrong", "enhancement"], ["silent-wrong"]);

console.log(failed === 0 ? "label-guard: all cases pass" : `label-guard: ${failed} FAILED`);
process.exit(failed === 0 ? 0 : 1);
