" Copyright (c) 2026 Jurjen Stellingwerff
" SPDX-License-Identifier: LGPL-3.0-or-later
"
" Minimal loft syntax highlighting for Neovim / Vim.  Keywords lex as tokens
" (not identifiers), so loft-lsp's semantic tokens do NOT colour them — this
" file covers the structural lexicon; the LSP's semantic tokens then classify
" identifiers (functions / methods / types / locals) on top.  Keyword groups
" mirror syntaxes/loft.tmLanguage.json (the canonical grammar).

if exists("b:current_syntax")
  finish
endif

syn keyword loftKeyword fn operator struct enum type interface pub use const let
syn keyword loftKeyword if else for while match in return break continue yield
syn keyword loftKeyword and or not as is sizeof
syn keyword loftBoolean true false
syn keyword loftConstant null
syn keyword loftType integer float single text character boolean
syn keyword loftType u8 u16 u32 u64 i8 i16 i32 i64 f32 f64
syn keyword loftType vector sorted hash index spatial iterator
syn keyword loftBuiltin print println len assert debug_assert panic self
syn keyword loftBuiltin par par_light par_fold stack_trace
syn keyword loftBuiltin log_info log_warn log_error log_fatal log_debug log_trace

" `//` line comments; `@`-tracker tags are highlighted specially inside them.
syn match  loftTag      "@\%(GH\|PLN\|PLAN\|F\|I\|P\)\d\+\a\?" containedin=loftComment contained
syn match  loftComment  "//.*$" contains=loftTag,@Spell

" Double-quoted strings with `{expr}` interpolation and `\` escapes.
syn match  loftEscape   "\\." contained
syn match  loftInterp   "{[^}]*}" contained
syn region loftString   start=+"+ skip=+\\"+ end=+"+ contains=loftEscape,loftInterp,@Spell

syn match  loftNumber   "\<\d\+\%(\.\d\+\)\?\>"

" `#rust "..."` / `#native` and other `#`-annotations.
syn match  loftAnnotation "#\w\+"

hi def link loftKeyword     Keyword
hi def link loftBoolean     Boolean
hi def link loftConstant    Constant
hi def link loftType        Type
hi def link loftBuiltin     Function
hi def link loftComment     Comment
hi def link loftTag         SpecialComment
hi def link loftString      String
hi def link loftEscape      SpecialChar
hi def link loftInterp      Special
hi def link loftNumber      Number
hi def link loftAnnotation  PreProc

let b:current_syntax = "loft"
