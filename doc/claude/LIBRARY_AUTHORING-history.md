<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->

# LIBRARY_AUTHORING-history.md — the timeline behind LIBRARY_AUTHORING.md

The timeline behind [LIBRARY_AUTHORING.md](LIBRARY_AUTHORING.md): the measurements behind its rules.

---

## 2e. A surface proven only by its own tests — the measurement

**Measured, 2026-08-20, in `lavition_ui`** (moros's, and their agent's own count while
answering @PLN145's `D0` request): **15 of 31 public functions had no production caller.**
`panel_hit_test` — the one function @PLN145 asked to depend on — was *"built, tested green and
invoked by nothing"*, in that tree's own words.

⚠⚠ **The follow-up is stronger than the measurement, and it is why the bar is not a style
rule.** moros gave `panel_hit_test` a caller the next day, and the commit subject is the
finding: *"A click on the panel turned the camera, because `panel_hit_test` had no caller."*
The function was not merely unused — under the one consumer that finally called it, the
program was **wrong**, and no amount of its own green tests could say so, because a test
asks *does it answer what I expect* and a consumer asks *is this the question I have*.
Re-counted 2026-08-21: **13 of 33** public functions still have no production caller, and
that tree's README now names all thirteen and calls them *a proposal*.
