## Summary

<!-- 1–3 sentences on what changed and why.  The "why" matters more than the "what" — the diff already shows what. -->

## Test plan

<!-- Tick what applies; add specifics. -->

- [ ] The head is current on `origin/main`; the PR's own `ci.yml` is the full gate
- [ ] New behaviour has a regression test under `tests/`
- [ ] Doc tests under `tests/docs/` updated if user-facing
- [ ] `make wasm-html-test` passes if browser/WASM is touched
- [ ] `make wrap` passes if heap / lifetime is touched

## Notes for the reviewer

<!-- Anything non-obvious about the approach and the trade-offs you considered.  Open issues this PR does not close: the `make work` output. -->
