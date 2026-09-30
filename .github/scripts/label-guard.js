// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// Which triage labels an issue BODY asks for — the parsing half of
// `.github/workflows/label-guard.yml`.
//
// A file rather than inline YAML because this is the part that can be WRONG in a
// way nothing reports: every failure mode here is "no label applied", which reads
// exactly like "the filer did not answer".  `tools/label_guard_selftest.mjs`
// exercises it against real body shapes; the workflow requires it.
//
// Nothing here touches the network or the API — give it a string, get labels.

"use strict";

/// The only labels that may ever be auto-applied.  A body is written by anyone
/// who can open an issue, so the allowlist is what keeps that from becoming
/// "any label they can spell", including ones that do not exist yet.
const VALID = new Set([
  "sev:high", "sev:medium", "sev:low",
  "wa:clean", "wa:partial", "wa:none",
  "area:parser", "area:codegen", "area:store-lifetime", "area:closures",
  "area:runtime", "area:native", "area:wasm", "area:stdlib", "area:packages",
  "hit-by:loft", "hit-by:moros", "hit-by:routing", "hit-by:dryopea",
  "hit-by:crawler", "hit-by:zerotrust", "hit-by:planets",
]);

/// Required label categories for a loft BUG (`.github/LABELS.md`), with the
/// values a filer may choose — the comment quotes these, so the message and the
/// parser cannot drift apart.
const CATEGORIES = [
  {
    key: "sev:",
    name: "severity — one `sev:high` / `sev:medium` / `sev:low`",
    values: ["sev:high", "sev:medium", "sev:low"],
    single: true,
  },
  {
    key: "wa:",
    name: "workaround — one `wa:clean` / `wa:partial` / `wa:none`",
    values: ["wa:clean", "wa:partial", "wa:none"],
    single: true,
  },
  {
    key: "area:",
    name: "area — one or more `area:*` (parser / codegen / store-lifetime / …)",
    values: [...VALID].filter((l) => l.startsWith("area:")),
    single: false,
  },
  // `hit-by:` was documented as required in LABELS.md ("a find of our own is
  // `hit-by:loft` — not a blank") and enforced by nothing: not the template, which
  // had no field for it, and not this guard.  Issues were filed without it and the
  // omission was invisible until someone read the labels by hand.  A blank is not
  // neutral — a consumer filters `hit-by:<their project>`, and an unlabelled issue
  // reads as "not established", never "nobody", so every count over the gap is a
  // floor rather than a total.
  {
    key: "hit-by:",
    name: "who hit it — one `hit-by:*` (a find of loft's own is `hit-by:loft`, never blank)",
    values: [...VALID].filter((l) => l.startsWith("hit-by:")),
    single: true,
  },
];

/// The heading a filer who CANNOT set labels writes to ask for them.
const TRIAGE_HEADING = "triage";

/// Split a body into its `### <heading>` sections, lowercased heading → text.
function sectionsOf(body) {
  const out = new Map();
  let head = null;
  for (const line of body.split(/\r?\n/)) {
    const m = /^###\s+(.+?)\s*$/.exec(line);
    if (m) {
      head = m[1].toLowerCase();
      out.set(head, []);
    } else if (head) {
      out.get(head).push(line);
    }
  }
  return out;
}

/// Distinct `<key>:<value>` tokens in `text`, restricted to [`VALID`].
function tokens(text, key) {
  const hits = new Set(
    [...text.matchAll(new RegExp("\\b" + key + ":[a-z0-9-]+", "g"))].map((m) => m[0]),
  );
  return [...hits].filter((t) => VALID.has(t));
}

/// The labels `body` asks for.
///
/// Three channels, in the order a body can supply them:
///
///  1. the bug FORM's own answer sections (`### Severity`, …) — what a web filer
///     produces, and what GitHub does not turn into labels for you;
///  2. a `### Triage` section — what a filer who cannot SET labels writes to ask
///     for them, since GitHub restricts labelling to triage permission and an
///     outside reporter has none (loft#805 arrived with no labels at all);
///  3. for `sev:`/`wa:` only, the body at large.
///
/// `sev:` and `wa:` are SINGLE-CHOICE: a value is taken only when its source
/// names exactly one.  Ambiguity must not be guessed — leaving it unset flags the
/// issue, which is the recoverable outcome.
///
/// `area:` is multi-valued and therefore never read from prose: the form's
/// CHECKED boxes, or the `### Triage` block, and nowhere else.  Scanning prose
/// for it is what labelled #626 with six mutually exclusive tokens, because its
/// body quoted a comment that listed them all.
function chooseLabels(body) {
  const text = body || "";
  const sections = sectionsOf(text);
  const section = (want) => {
    for (const [h, lines] of sections) {
      if (h.includes(want)) {
        const t = lines.join("\n").trim();
        // A present-but-blank section must not short-circuit the fallback chain:
        // an unanswered form field is no answer, not an empty one.
        return t === "" ? null : t;
      }
    }
    return null;
  };
  // A pasted log or a quoted example must not contribute labels.
  const unfenced = text.replace(/```[\s\S]*?```/g, "");
  const triage = section(TRIAGE_HEADING);

  // "clean workaround", not "workaround".  TWO form headings contain the latter
  // — the free-text `### Workaround` textarea comes FIRST, so a substring match
  // returned the prose and the dropdown that actually sets `wa:*` was never
  // read.  Silent by construction: the filer answers a required dropdown and the
  // label simply does not appear.
  const sevText = section("severity") ?? triage ?? unfenced;
  const waText = section("clean workaround") ?? triage ?? section("workaround") ?? unfenced;
  // "who hit it", not "hit" — the phrase has to be distinctive enough that no other
  // heading contains it, which is the trap `wa:` fell into (`### Workaround` came
  // first and swallowed the dropdown that actually sets the label).
  const hitText = section("who hit it") ?? triage ?? unfenced;

  const found = new Set();
  for (const [src, key] of [[sevText, "sev"], [waText, "wa"], [hitText, "hit-by"]]) {
    const hits = tokens(src, key);
    if (hits.length === 1) found.add(hits[0]);
  }
  const areaSection = section("area");
  if (areaSection) {
    for (const m of areaSection.matchAll(/-\s*\[[xX]\]\s*`?(area:[a-z0-9-]+)/g)) {
      if (VALID.has(m[1])) found.add(m[1]);
    }
  }
  if (triage) {
    for (const t of tokens(triage, "area")) found.add(t);
  }
  return [...found];
}

/// Which of `chosen` may be ADDED to an issue that already carries `current`.
///
/// An exact-name filter is not enough, because `sev:`, `wa:` and `hit-by:` are
/// SINGLE-CHOICE: an issue already labelled `sev:medium` does not "have"
/// `sev:high`, so a naive filter adds it and the issue ends up carrying two
/// contradictory severities.  Measured on loft#1048, which was filed with
/// `sev:medium` and came back also holding `sev:high` seven seconds later.
///
/// The body that caused it is worth stating, because it is the shape a GOOD issue
/// has: its only `sev:` token sat in the heading *"Why it is not `sev:high`"*.
/// The prose channel cannot tell an assertion from its negation, and teaching it
/// to would be guessing — the parser's own doctrine is that ambiguity is left
/// unset rather than resolved.  So the cure is placed where the question is not
/// ambiguous at all: a category the filer has ALREADY answered is not open, and
/// prose must not reopen it.  A filer's own label always wins over a mention.
function applicable(chosen, current) {
  const has = new Set(current);
  return chosen.filter((label) => {
    if (has.has(label)) return false;
    const cat = CATEGORIES.find((c) => label.startsWith(c.key));
    if (cat && cat.single && [...has].some((l) => l.startsWith(cat.key))) return false;
    return true;
  });
}

/// The label that parks an issue past a release freeze.  The release tag strips it
/// (`release.yml`, job `unpark`), so the issue returns to `make work` by itself.
const NEXT_RELEASE = "next-release";

/// Why `next-release` may NOT sit on this issue — the labels that make it a DEFECT —
/// or `[]` when it may.  A bug is fixed before the release, never parked past it
/// (CLAUDE.md § Bug-filing policy), and `silent-wrong` least of all; without this the
/// one relabel that takes an issue off `make work` would be a way round that rule.
/// Empty when the label is absent: only a parked issue is judged.
function nextReleaseConflicts(current) {
  const has = new Set(current);
  if (!has.has(NEXT_RELEASE)) return [];
  return [...has].filter((l) => l === "bug" || l === "silent-wrong" || l.startsWith("sev:")).sort();
}

module.exports = {
  VALID, CATEGORIES, TRIAGE_HEADING, NEXT_RELEASE, chooseLabels, applicable, nextReleaseConflicts,
};
