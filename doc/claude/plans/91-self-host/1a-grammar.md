<!--
Copyright (c) 2026 Jurjen Stellingwerff
SPDX-License-Identifier: LGPL-3.0-or-later
-->

# @PLN91 1a — loft's concrete syntax, as the Rust parser accepts it

The grammar the syntax layer ([1a-parser.md](1a-parser.md)) implements, read from
`src/parser/` (not from the informal grammar in `LOFT_DECLARATIONS.md`, which is a map only).
Each production names the Rust function that parses it — the function elaboration ports for
that node.  Paths are under `src/parser/` unless named; line numbers are approximate (search
the function name).  Surveyed 2026-11-01 on `91-self-host`.

Notation: `'x'` a token or keyword; `Ident`, `Int` (integer, long, wide), `Float` (float,
double, single), `Str`, `Char` literal tokens; `?` `*` `+`, `/` ordered choice, `&x` / `!x`
look-ahead.  Keywords are tokens (lexer.rs `KEYWORDS`); every other word — `self`, `const`,
`value`, `not`, `limit`, `size`, `par`, `rev`, `integer` — is an `Ident` matched in context.
Casing (CamelCase types, lower_case functions) is a diagnostic, never syntax.

## File

```
File       <- FileDirective* UseDecl* TopItem* End                    mod.rs parse_file
FileDirective <- '#' ( 'width' Int / 'package' / 'cwd' / 'link' Ident ) ';'?   mod.rs, width32.rs
UseDecl    <- 'pub'? 'use' ( 'self' '::' Ident / Ident ) ('as' Ident)? ImportSpec? ';'
                                                                      imports.rs parse_use_self, mod.rs
ImportSpec <- '::' ( '*' / '(' ImportName (',' ImportName)* ')' / ImportName )   imports.rs parse_import_spec
ImportName <- Ident ('as' Ident)?
TopItem    <- 'pub'? ( Capability / Enum / TypeAlias / Function / Struct / Interface / Constant )
                                                                      mod.rs, in this order
Capability <- 'capability' Ident ('.' Ident)*                         definitions.rs (no ';')
```

- A `use` after a definition is an error; a directive is read only before the first `use`.
- A script (loose statements, no `fn main`) is a TEXTUAL desugar before parsing
  (`src/script.rs`: definitions hoisted, statements wrapped in `fn main`); the syntax layer
  ports it as a pass over the token stream.
- The leftover `Ident` at top level is a `Constant`, the last alternative.

## Functions

```
Function   <- ('fn' / 'operator') FnName TypeVars? '(' Params ')' ('->' 'const'? Type)? CapLink?
              ( ';' / Block ';'? ) Annotation*                        definitions.rs parse_function
FnName     <- Ident / 'assert' / 'panic'                              (the last two: stdlib only)
TypeVars   <- '<' ( TVar (',' TVar)* )? CloseAngle                    parse_type_var_header
TVar       <- Ident (':' Ident ('+' Ident)*)?
Params     <- ( Param (',' Param)* ','? )?                            parse_arguments
Param      <- Ident (':' '&'? 'const'? Type)? ('=' Expr)? CapLink?
CapLink    <- Ident ('.' Ident)* '#' Ident                            try_cap_link (read/update/append)
Annotation <- '#' ( 'native' Str? / 'rust' Str / 'iterator' Str Str / 'c' Str Str
                  / 'hot' / 'cold' / 'null_safe' / 'builtin' / 'pure' / 'superseded' Str
                  / 'impure' '(' Ident ')' )                          parse_rust, parse_c_binding
```

- Annotations FOLLOW the function they belong to (`fn …;` then `#rust "…"`).  An unknown
  `#name` ends the list.
- A method is a function whose first parameter is named `self`; an operator is
  `operator <name>` with a name from a fixed list (a pass-2 check).

## Struct, enum, alias, interface, constant

```
Struct     <- 'value'? 'struct' Ident TypeVars? '{' SField (',' SField)* ','? '}' ';'?
                                                                      parse_struct
SField     <- 'pub'? 'const'? Ident ':' FieldItem+
Enum       <- 'enum' Ident TypeVars? '{' Variant (',' Variant)* ','? '}' ';'?   parse_enum
Variant    <- Ident Parens? ( '{' VField (',' VField)* ','? '}' )?    parse_enum_values
VField     <- ('#' 'lexeme')? 'pub'? 'const'? Ident ':' FieldItem+
FieldItem  <- 'const' / 'not' 'null' / ('computed' / 'virtual' / 'init' / 'default') '(' Expr ')'
            / ('assert' / 'CHECK') '(' Expr (',' Expr)? ')'
            / Type ('=' Expr)? CapLink*                               parse_field (a loop, any order)
TypeAlias  <- 'type' Ident ('=' Type)? ('size' '(' Int ')')? ';'      parse_typedef
Interface  <- 'interface' Ident '{' IMember* '}' ';'?                 parse_interface
IMember    <- 'type' Ident (':' Ident ('+' Ident)*)? ';'?
            / ( 'op' OpTok / 'operator' Ident / 'fn' Ident ) '(' Params ')' ('->' Type)? ';'?
Constant   <- 'const'? Ident (':' Type)? '=' Expr ';'                 parse_constant
```

- `Variant(…)` (positional) is skipped with an error; `default(…)` is consumed with an error.

## Types

```
Type       <- TupleType / FnType / NamedType                         parse_type_full
TupleType  <- '(' Type (',' Type)+ ','? ')'
FnType     <- 'fn' '(' ( FnTArg (',' FnTArg)* ','? )? ')' ('->' Type)?   parse_fn_type
FnTArg     <- '&'? 'const'? Type
NamedType  <- Ident ('::' Ident)? TypeArgs? Deps? IntSuffix? ('.' Ident)? '?'?   parse_type, parse_type_inner
TypeArgs   <- '<' ( '(' TupleInner / 'fn' FnType / Elem ) CollTail   sub_type_inner (by the outer name)
            / '<' Type (',' Type)* CloseAngle ('::' Ident)?           a template instance
Elem       <- Type '?'?                                               `vector<T?>` only
CollTail   <- ('not' 'null')? ( KeySpec / ',' Type )? CloseAngle      sorted/index/hash/trie/spatial/iterator
KeySpec    <- '[' '-'? Ident (',' '-'? Ident)* ']'                    parse_fields ('-' descending)
Deps       <- '[' Ident (',' Ident)* ']'                              stdlib return types only
IntSuffix  <- ('limit' '(' '-'? Int ',' '-'? Int ')')? ('not' 'null')?   after `integer` only
CloseAngle <- '>' / the first '>' of '>>' or '>='                     lexer.rs has_closing_angle
```

The syntax layer splits `>>` and `>=` into two tokens (`Gt` then a `>`/`=` marked as glued),
so `CloseAngle` is one token and a shift or a `>=` is two glued ones.

## Statements

```
Block      <- '{' ( ';' / Stmt StmtEnd )* '}'                         control.rs parse_block_inner
StmtEnd    <- ';' / &'}' / <after a BlockLike>
BlockLike  <- If / Match / For / While / Parallel / Block
Stmt       <- Expr ('else' Block)?                                    store_else.rs parse_store_else
Expr       <- 'for' ForRest / 'while' Expr Block / 'continue' / 'break' Expr? / 'return' Expr?
            / 'debug_assert' Args? / 'parallel' '{' (Expr ';'?)* '}'
            / 'yield' 'from' Expr / 'yield' Expr / 'const'? Assign    expressions.rs expression_inner
Assign     <- PlaceList '=' Expr                                      parse_place_destructure
            / Ident ':' '&'? 'const'? Type '=' Expr                   parse_assign_inner (typed local)
            / OpExpr (AssignOp OpExpr)?
AssignOp   <- '=' / '+=' / '-=' / '*=' / '/=' / '%='
PlaceList  <- '(' Place (',' Place)+ ')' &('=' !'=')
ForRest    <- ForBind 'in' IterSrc ( 'par' '(' Ident '=' Worker ',' Expr ')' / ('if' Expr)? ) Block
                                                                      collections.rs parse_for
ForBind    <- Ident / '(' Ident (',' Ident)+ ')'
IterSrc    <- 'unordered' '(' Expr ('..' '='? Expr?)? ')' / 'rev' '(' Range ')' / Range
Range      <- OpExpr? ('..' '='? OpExpr?)?                            parse_in_range_body
If         <- 'if' Expr Block ('else' (If / Block))?                  parse_if_expecting
```

- `break e` and `return e` take a value unless a `;` `}` `,` `)` `]` follows.
- An `If`/`Match`/`For`/`While`/block at the START of a statement takes no `[`, `(` or
  binary operator after it (the Rust parser decides by the operand being void).
- `fn` followed by `(` inside a block is a lambda; any other `fn`, and `struct`, `enum`,
  `type`, `interface`, `use`, `pub`, is refused ("definitions must be at file scope").

## Expressions

Precedence climbing over `mod.rs OPERATORS`, loosest first:

| level | operators | associativity |
|---|---|---|
| 0 | `??` | right |
| 1 | `\|\|` `or` | left |
| 2 | `&&` `and` | left |
| 3 | `==` `!=` `<` `<=` `>` `>=`, `is` | none (a second comparison is an error) |
| 4 | `\|` | left |
| 5 | `^` | left |
| 6 | `&` | left |
| 7 | `<<` `>>` | left |
| 8 | `-` `+` | left |
| 9 | `*` `/` `%` | left |
| 10 | `**` | right |
| 11 | `as` Type `?`? | left, a type on the right |

```
OpExpr     <- Unary (BinOp Unary / 'is' IsRhs / 'as' Type '?'?)*     operators.rs parse_operators
IsRhs      <- Ident ('{' Ident (',' Ident)* ','? '}' &'{')?           control.rs parse_is_variant
Unary      <- ('&' / '!' / '~' / '-') Postfix / Postfix               vectors.rs parse_single
Postfix    <- Primary ( '.' (Int / Ident Args?) / '[' Index ']' / Args / '?' / Bang )*
                                                                      operators.rs parse_part_inner
Bang       <- '!' glued to the operand (no space before it)
Args       <- '(' ( Arg (',' Arg)* ','? )? ')'      Arg <- (Ident ':' !':')? Expr
Index      <- '(' Expr (',' Expr)* ')' '..' ( '(' Expr (',' Expr)* ')' / ':' Expr )?
            / (Expr (',' Expr)*)? ('..' '='? (':' Expr / Expr (',' Expr)*)?)?   fields.rs parse_index
Primary    <- '(' Expr (',' (Expr (',' Expr)* ','?)?)? ')'            paren or tuple
            / AnonStruct / Block / '[' VecBody ']' / If / Match
            / 'fn' '(' Params ')' ('->' Type)? Block / '||' Block / '|' (Ident (',' Ident)*)? '|' Block
            / ('sizeof' / 'type_name') '(' (Type &')' / Expr) ')'
            / ('assert' / 'panic') Args
            / 'vector' '<' Ident '>' '.' 'parse' Args
            / Path ( StructLit / Args / '#' VarAttr )?                objects.rs parse_var
            / '$' / Int / Float / StringLit / Char / 'true' / 'false' / 'null'
Path       <- Ident ('::' Ident)?
VarAttr    <- 'break' / 'continue' / Ident ('(' Expr ')')? ('as' Type)?
StructLit  <- '{' ( '..' Expr ','? / Field (',' (Field / '..' Expr ','?))* ','? )? '}'
                                                                      parse_object_inner
AnonStruct <- '{' Field (',' Field)* ','? '}'
Field      <- (Ident / Str) ':' OpExpr
VecBody    <- 'for' ForBind 'in' IterSrc ('if' Expr)? Block
            / ( Item (',' Item)* ','? )?          Item <- OpExpr (';' OpExpr)?
StringLit  <- (StrHole Hole RBrace)* Str          a segment that opens a hole is a StrHole
Hole       <- ( 'for' ForRest / Expr ) (':' Spec)?                    objects.rs parse_string
Spec       <- Fill? Flag* Width? Radix?           kept as its tokens; read in elaboration
```

- `Path { … }` is a struct literal when the body is `{}`, or starts `Ident :` (not `::`) or
  `Ident ,` — and in a control head (`if`/`while` condition, `for` source, `match` subject)
  an empty `{}` only when one of `{ ) ] , . ??` follows.
- `<` after an identifier is the comparison (a call with type arguments is refused).
- `x = …` takes an `OpExpr`; a typed local `x: T = …` a full `Expr`.

## Patterns

```
Match      <- 'match' Expr '{' Arm* '}'                               control.rs parse_match_inner
Arm        <- ArmPats ('if' Expr)? '=>' (Block / Expr) ','?
ArmPats    <- TopPat (('|' / ',') TopPat)*
TopPat     <- 'null' / Ident ('@' / ':') Cap / Alt
Alt        <- Cap ('|' Cap)*
Cap        <- Ident ':' Cap / Postfix
Postfix    <- Group / Point ('*' / '+')?                              the quantifier: a scalar run `xs:T*`
Group      <- '(' Seq ('|' Seq)* ')' ('*' / '+' / '?')? RepSep?
Seq        <- Cap+
RepSep     <- '(' (Literal / Path FieldBlock?) ')'
Point      <- '_' / Slice / Tuple / Range / Literal / Path FieldBlock? / Operand
FieldBlock <- '{' ( PField (',' PField)* ','? )? '}'
PField     <- Ident (':' Alt)?
Tuple      <- '(' Cap (',' Cap)+ ')'
Slice      <- '[' ( SliceElem ','? )* ']'
SliceElem  <- '..' Ident? / Cap
Range      <- Bound '..' '='? Bound
Bound      <- '-'? (Literal / Operand)               an operand above the `|` level
Literal    <- '-'? (Int / Float) / Str / Char / 'true' / 'false'
```

What a pattern identifier MEANS — a binding, a variant, a constant, a sub-rule — is the
subject's type's to decide, in elaboration (`control.rs`: `variant_of`, `pattern_variant_enum`,
`peek_subrule_capture`, `peek_scalar_type_capture`).  The syntax node keeps the spelling.
