<!-- size-exempt: a record companion, read by anchor and grep (DOC_QUALITY § Maintainer docs 2) -->
# Rule crossings — the verified pairs

The ledger [RULE_CROSSINGS.md](RULE_CROSSINGS.md) describes.  `scripts/crossings.py` reads the
table below, and `scripts/rule_tags.py pairs` skips each pair whose verdict is `holds` or
`fixed loft#N`.  Keep the header row as it is; add one row per pair, and extend a row rather
than adding a second one for the same pair.

## The ledger

| Rule A | Rule B | Verdict | Validated | How | Axes crossed | Guard |
|---|---|---|---|---|---|---|
| @FR-B-Ref-Alias | @FR-G-Own | fixed loft#2420 | 2026-11-06 @ 0de1794d4 | 12-cell probe matrix, interpreter + `--native-c`, value channel and crash; falsified against main (both channels) | link target type: integer, float, text, vector, record, struct-enum, tuple; link source: local, parameter, field; context: straight, `for` loop; the bind-first workaround as control | `loft-suite/rules/coroutines/G-Own/2420-a-yielded-link-yields-its-value.loft` |
| @FR-B-Ref-Uniform | @FR-G-Own | fixed loft#2420 | 2026-11-06 @ 0de1794d4 | as the row above: a `yield` of a `&τ` is the yield of a τ | as the row above | `loft-suite/rules/coroutines/G-Own/2420-a-yielded-link-yields-its-value.loft` |
| @FR-B-Ref-Alias | @FR-T-Cons | holds | 2026-11-06 @ 0de1794d4 | probe cells c01, c02, interpreter + `--native-c`, value channel | member read through a text-member link; a whole vector link as a member | `loft-suite/rules/binding/B-Ref-Alias/a-link-composes-with-tuples-repoints-and-ref-params.loft` |
| @FR-B-Ref-Lvalue | @FR-T-Record | holds | 2026-11-06 @ 0de1794d4 | probe cells c01, c19, interpreter + `--native-c`, value channel | member type: text, vector; write and append through the link | `loft-suite/rules/binding/B-Ref-Alias/a-link-composes-with-tuples-repoints-and-ref-params.loft` |
| @FR-B-Ref-Intro | @FR-T-Ref | holds | 2026-11-06 @ 0de1794d4 | probe cell c16, interpreter + `--native-c`, value channel | `b = &a` over a scalar tuple, destructured, then written through | `loft-suite/rules/binding/B-Ref-Alias/a-link-composes-with-tuples-repoints-and-ref-params.loft` |
| @FR-B-Ref-Repoint | @FR-B-Ref-Write | holds | 2026-11-06 @ 0de1794d4 | probe cells c03, c04, c05, c09, c22, c25, interpreter + `--native-c`, value channel | target type: text, integer, record, vector field, text field; source: local, element, field; context: straight, `if` arm, `for` loop | `loft-suite/rules/binding/B-Ref-Alias/a-link-composes-with-tuples-repoints-and-ref-params.loft` |
| @FR-B-Ref-Write | @FR-B-Ref-Lvalue | holds | 2026-11-06 @ 0de1794d4 | probe cells c06, c07, c08, c21, interpreter + `--native-c`, value and leak channels | whole-record write to a field link; field write to an element link; vector and record-with-text whole writes in a `for` loop (each displaces a store) | `loft-suite/rules/binding/B-Ref-Alias/a-link-composes-with-tuples-repoints-and-ref-params.loft` |
| @FR-B-Ref-Alias | @FR-F-ParamRef | holds | 2026-11-06 @ 0de1794d4 | probe cell c18, interpreter + `--native-c`, value channel | a vector link handed to a `&vector` parameter, in a `for` loop | `loft-suite/rules/binding/B-Ref-Alias/a-link-composes-with-tuples-repoints-and-ref-params.loft` |
