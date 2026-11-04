<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# slice_into: no hand form

`R-SliceInto`'s IR form cannot be spelled in loft: `c.bytes += v[a..b]` on a field compiles
to the same temporary and append as the literal's field (both `OpSliceView` into a temporary,
then `OpAppendVector`); only a LOCAL place, `p += v[a..b]`, already compiles to one
`OpSliceVector` in the parser.  So the move is that parser path extended to field places, and
its saving is measured on arc C's beside-switch build, not here.
