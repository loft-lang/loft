<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# STORE_RECORDS-history.md — the record of [STORE_RECORDS.md](STORE_RECORDS.md)

## The rustc writer's own lowerings, moved on 2026-11-24

The rustc writer (`--native-rust`) retired with `@C170`.  These passages described lowerings
only it applied, and the switches that restored each; the records read IR and never saw them.

On rustc `--native`, a call arm inside `output_set`'s `_old_<name>` stash leaves the displaced
store to that stash (`Output::old_stash_var`); `LOFT_NATIVE_INNER_DISPLACED_FREE=1` frees it in
the arm too.  The records read IR, which both spellings share, so they cannot see this one.
Nor the next two, also rustc `--native` only: a call bind that aliases a borrowed view into a
never-free local (`adopt` is `true`) leaves `call_bind_owns` false, so a runtime-Join local's
owned-store tracker does not take the caller's store as its own
(`LOFT_NATIVE_VIEW_ALIAS_OWNS=1` does); and a guarded free of such a local
(`OpFreeRefIfDistinct`) lets its tracker go of the store it releases
(`LOFT_NATIVE_GUARDED_FREE_KEEPS_TRACKER=1` keeps it) — each switch re-frees a sub-rule run's
answer (P-Rule `borrowed`, `mixed_run`).

The rustc writer read `re_bind` for whether a reassignment frees what it displaces
(`LOFT_NATIVE_OWN_DISPLACED_FREE=1` gives it back its own test); its trace names its exits
(`rs-call-bind`, `rs-var-copy`, `rs-value-record`, `rs-tail` for the plain bind).  A reassignment `re_bind` binds plainly takes the rustc writer's plain bind too, not its
runtime copy-or-adopt (`call_bind_taken`; `LOFT_NATIVE_OWN_CALL_BIND=1` gives the arm back): an
owner witness then takes the answer as its own where it is no other argument's store
(`call_bind_declined`), and the plain bind's `_old_` stash stands down where the witness's
`_disp_` stash already releases the displaced store (`old_stash_var`).  A reassignment from
another variable's record copies only where `re_bind` copies it (R2c), the owner witness asking
the same (`var_copy_taken`; `LOFT_NATIVE_OWN_VAR_COPY=1`).

The rustc writer's census
(`--writer native`) is a report and has no table.
