<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# Indexes — history

The record behind [DATABASE_INDEXES.md](DATABASE_INDEXES.md): how its numbers were measured and
what came before.

## The load factor

A hash table was rebuilt at three quarters full (`length * 2 / 3` in the same test) until
2026-09-21, when the threshold became half.  A probe was then a dependent read of the entry with
no fingerprint to reject on (the tag of `(Col-Tag)` came later), and a miss walked to the first
empty bucket: 8.5 buckets at 0.75 against 2.5 at 0.5.  A table just under the old threshold
measured 2.30 key compares a hit and 5.36 a miss.  The test was written then as
`length + RESERVED_WORDS >= room` over the record's word count.
