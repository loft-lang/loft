// Copyright (c) 2026 Jurjen Stellingwerff
// SPDX-License-Identifier: LGPL-3.0-or-later
//
// The PR's "Nightly health" sticky comment — one home for it, required by two callers:
// `ci.yml`'s `Nightly health (informational)` job on every PR run, and `miri.yml`'s
// `pr-head-report` job when a PR-head nightly (`workflow_dispatch` with `pr_head: true`)
// finishes, so the verdict lands on the PR without waiting for its next CI run.
//
// What it reports for the nightly (`miri.yml`): the nightly dispatched on the PR's OWN head
// commit when one exists, otherwise main's latest.  A PR's own CI skips exactly the nightly
// jobs that are heavy (the platform C sweeps, the poison and release-gate sweeps, valgrind,
// the toolchain matrix), so main's latest says nothing about whether this PR fixes or breaks
// them; the dispatched run does.  A push after the dispatch moves the head, and the report
// falls back to main's run rather than show a verdict about other code
// (DEVELOPMENT.md § The PR-head nightly).

const MARKER = '<!-- nightly-status-comment -->';
const REPORT_JOB = 'PR-head nightly report';

const icon = (c) => (c === 'success' ? '✅'
  : (c === 'skipped' || c === 'cancelled') ? '⤵️' : '❌');
const isBad = (j) => j.conclusion
  && !['success', 'skipped', 'cancelled'].includes(j.conclusion);
const newest = (runs) => runs.reduce(
  (best, r) => (!best || r.created_at > best.created_at ? r : best),
  null,
);

// `failOnRed`: paint the calling job red when something needs attention (the PR CI job does,
// so a red nightly shows on the checks list; the nightly's own report job does not, since its
// run is already red).
async function report({ github, context, core }, { prNumber, headSha, failOnRed }) {
  const { owner, repo } = context.repo;
  let attention = '';
  try {
    const runJobs = async (run) => (await github.rest.actions.listJobsForWorkflowRun({
      owner, repo, run_id: run.id, per_page: 100,
    })).data.jobs.filter((j) => j.name !== REPORT_JOB);
    // Ask for a WINDOW and pick the newest in it, rather than trusting position 0 of a
    // one-item page: the run-list endpoint can answer a STALE window at small `per_page`, and
    // the staleness drifts (owner-measured: `per_page=1` once handed back a run twelve days
    // old while `per_page=10` gave that day's).  This reporter then red-flagged a macOS
    // failure eleven green scheduled runs had superseded; a health signal that cries wolf is
    // one nobody reads.
    const latestOnMain = async (workflow_id, extra) => newest((await github.rest.actions
      .listWorkflowRuns({
        owner, repo, workflow_id, branch: context.payload.repository.default_branch,
        status: 'completed', per_page: 10, ...extra,
      })).data.workflow_runs);
    const sections = [];

    // The nightly: the run dispatched on this PR's head, else main's latest.
    const onHead = headSha ? newest((await github.rest.actions.listWorkflowRuns({
      owner, repo, workflow_id: 'miri.yml', head_sha: headSha,
      event: 'workflow_dispatch', per_page: 10,
    })).data.workflow_runs) : null;
    if (onHead) {
      const jobs = await runJobs(onHead);
      const done = onHead.status === 'completed' || jobs.every((j) => j.status === 'completed');
      const link = `[run #${onHead.run_number}](${onHead.html_url})`;
      if (!done) {
        const left = jobs.filter((j) => j.status !== 'completed').length;
        sections.push(`**nightly on this PR's head** (${headSha.slice(0, 9)}): ⏳ running, `
          + `${left} of ${jobs.length} jobs left — ${link}`);
      } else {
        const ran = jobs.filter((j) => j.conclusion !== 'skipped');
        if (ran.some(isBad)) attention = "the nightly on this PR's head";
        sections.push(`**nightly on this PR's head** (${headSha.slice(0, 9)}; the jobs a PR `
          + 'skips — the rest ran in this PR\'s own CI): '
          + ran.map((j) => `${icon(j.conclusion)} ${j.name}`).join(' · ')
          + ` — ${link}`);
      }
    } else {
      const miri = await latestOnMain('miri.yml', {});
      if (miri) {
        const jobs = await runJobs(miri);
        if (jobs.some(isBad)) attention = 'the latest nightly on main';
        sections.push(`**miri nightly on main** (${miri.created_at.slice(0, 10)}): `
          + jobs.map((j) => `${icon(j.conclusion)} ${j.name}`).join(' · ')
          + ` — [run #${miri.run_number}](${miri.html_url})`);
      }
    }
    // @PLN117 browser-threading gates (browser-threads.yml, nightly + on threading-surface
    // pushes) — the only place `par` on real Web Worker threads is proven, so a red there is
    // invisible anywhere else.  A missing workflow (not on main, renamed) is skipped rather
    // than losing the whole report to a 404.
    let threads = null;
    try { threads = await latestOnMain('browser-threads.yml', {}); } catch (e) { /* absent */ }
    if (threads) {
      const jobs = await runJobs(threads);
      if (jobs.some(isBad)) attention ||= 'the latest browser-threading run on main';
      sections.push(`**browser threading** (${threads.created_at.slice(0, 10)}): `
        + jobs.map((j) => `${icon(j.conclusion)} ${j.name}`).join(' · ')
        + ` — [run #${threads.run_number}](${threads.html_url})`);
    }
    // Scheduled daily full-matrix CI — the only place Windows runs.  About twenty jobs, so
    // name the failures and count the rest.
    const daily = await latestOnMain('ci.yml', { event: 'schedule' });
    if (daily) {
      const jobs = await runJobs(daily);
      const bad = jobs.filter(isBad);
      if (bad.length) attention ||= 'the latest daily CI on main';
      const okCount = jobs.filter((j) => j.conclusion === 'success').length;
      sections.push(`**daily full CI incl. Windows** (${daily.created_at.slice(0, 10)}): `
        + (bad.length
          ? `${bad.map((j) => `❌ ${j.name}`).join(' · ')} · ✅ ${okCount} green`
          : `✅ all ${okCount} jobs`)
        + ` — [run #${daily.run_number}](${daily.html_url})`);
    }
    const body = sections.length
      ? `${MARKER}\n🌙 **Nightly health**: ${attention ? '⚠️ needs attention' : 'all clear'}\n\n`
        + `${sections.join('\n')}\n\nPer-*job* status (sees through the toolchain matrix's `
        + '`continue-on-error`). Informational; **not a merge gate**.'
      : `${MARKER}\n🌙 **Nightly health** — no completed nightly run yet. `
        + 'Informational; not a merge gate.';
    const { data: comments } = await github.rest.issues.listComments({
      owner, repo, issue_number: prNumber, per_page: 100,
    });
    const existing = comments.find((c) => c.body && c.body.includes(MARKER));
    if (existing) {
      await github.rest.issues.updateComment({ owner, repo, comment_id: existing.id, body });
    } else {
      await github.rest.issues.createComment({ owner, repo, issue_number: prNumber, body });
    }
    // A red nightly must be VISIBLE, not only a comment: a nightly once sat red for ten
    // straight days while the sticky comment alone made nobody look.  Failing the PR job
    // paints a red ❌ on the checks list; it is not a required check and the job is
    // continue-on-error, so it never blocks a merge.
    if (attention && failOnRed) {
      core.setFailed(`${attention[0].toUpperCase()}${attention.slice(1)} is RED — see the `
        + 'sticky comment for the failing jobs.');
    }
  } catch (e) {
    // Informational only — never fail a run over a reporting hiccup.
    core.warning(`nightly-status reporter skipped: ${e.message}`);
  }
}

module.exports = { report, REPORT_JOB };
