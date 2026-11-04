<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# ret_adopt: no hand form

`R-RetAdopt`'s IR form cannot be spelled in loft: the result local IS the caller's hidden
buffer.  The nearest spelling, a `&vector<integer>` parameter the callee clears and fills,
reads +27 % on the interpreter, because every push goes through the reference, which the IR
form does not.  Its saving (the witness store's mint and free, the delivery's clear and copy)
is measured on arc C's beside-switch build.
