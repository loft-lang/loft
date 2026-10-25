# DEBUG_STORES — history

What [DEBUG_STORES.md](DEBUG_STORES.md) measured once, kept here so the working doc states
only the contract.

- `LOFT_VERIFY_STACK=1`'s calibration: the nullable-local pre-init control is the build at
  `64437246`, where the shadow reports four sites.
- `LOFT_VERIFY_STACK=1`'s calibration over 1106 runnable corpus programs: silent on all of
  them; `handle 12` read as `i64` on loft#1028's control, four sites on loft#1016's, and one
  site each on loft#1373 / #1377 / #1384 (open when measured, so HEAD was their broken
  build).  loft#1070's control answers `4294967198` with the shadow silent: the wrong layout
  is in a heap record and the slot holds a correct handle.
