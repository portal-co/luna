# The Luna Subset (v0)

Normative specification of the language subset shared by the lua2js and js2lua
compilers. The executable form of this document is `crates/looney/src/ir.rs`
(LunaIR) plus its validator: anything outside this document fails lowering or
validation with a positioned error, never a panic.

Status legend: **[v0]** implemented now; **[P2]–[P6]** scheduled (PLAN.md);
**[X]** excluded from the subset (rejected with a clear error).

## 1. Values

| Value      | Lua spelling | JS spelling   | Representation (lure / js.lua)          |
| ---------- | ------------ | ------------- | --------------------------------------- |
| nothing    | `nil`        | `null`        | `null` / `nil`                          |
| absent     | —            | `undefined`   | `undefined` (host) / `jsUndefined` sent.|
| boolean    | `true/false` | `true/false`  | `boolean` / `boolean`                   |
| number     | `1`, `1.5`   | `1`, `1.5`    | f64 both sides (no integer subtype)     |
| string     | `"s"`        | `"s"`         | `string` / `string`                     |
| table/obj  | `{...}` [P3] | `{...}` [P3]  | `LuaTable` / `JSObject` wrapper         |
| function   | [P2]         | [P2]          | host function values                    |

- Assigning `nil` to a table key **erases** it [P3].
- `undefined` is distinct from `null`; reading a missing JS property yields
  `undefined` (never nil-safe) [P3].
- NaN: `NaN ~= NaN` / `NaN !== NaN` in both languages.

## 2. Lexical

- Identifiers are portable-grammar only: ASCII `[A-Za-z_][A-Za-z0-9_]*`.
  JS `$`/unicode identifiers are **[X]** (js2lua rejects with a clear error).
- Name mangling is name-based and total: a source identifier colliding with a
  target reserved word or runtime-helper name is mangled (`lua$<name>` for JS
  output, `<name>_` for Lua output). Mangling is consistent across scopes;
  block scoping of the target language provides shadowing.
- Number literals: decimal integer/float/exponent and hex **integers**
  (`0x10`). Hex floats (`0x1p4`) are **[X]**.
- String literals: single/double quoted with the common escape set
  (`\a \b \f \n \r \t \v \\ \" \' \xXX \ddd \u{XXX}`, escaped newline).
  Lua `\z` is **[X]**. Lua long strings `[[...]]` are supported, including the
  leading-newline skip. JS template literals are **[X]** (use `+`).
- Comments pass through the parser; they are not preserved in output.

## 3. Statements

| Construct            | Lua                       | JS                          | Status |
| -------------------- | ------------------------- | --------------------------- | ------ |
| declaration          | `local x = e`             | `let x = e;` / `const`      | [v0]   |
| uninitialized decl   | `local x` (→ nil)         | `let x;` (→ **undefined**)  | [v0]   |
| assignment           | `x = e`                   | `x = e;`                    | [v0]   |
| call statement       | `f(...)`                  | `f(...);`                   | [v0]   |
| block                | `do ... end`              | `{ ... }`                   | [v0]   |
| return (chunk/fn)    | `return e` (≤ 1 value)    | `return e;` (in fns)        | [v0]*  |
| if/elseif/else       | `if .. elseif .. else`    | `if .. else if .. else`     | [P2]   |
| loops                | `while`, numeric `for`    | `while`, classic `for(;;)`  | [P2]   |
| break                | `break`                   | `break;`                    | [P2]   |
| ternary              | —                         | `c ? a : b`                 | [P2]   |
| multiple assignment  | `a, b = b, a`             | (temporaries)               | [P4]   |
| generic iteration    | `for k,v in ...`          | `for..of`                   | [P5]   |
| errors               | `error`/`pcall`           | `throw`/`try`               | [P5]   |
| `repeat`/`do..while` | —                         | —                           | [X] v0 |
| `goto`, `switch`     | —                         | —                           | [X]    |
| `var`                | —                         | —                           | [X]    |
| destructuring        | —                         | —                           | [X]    |

\* lua2js output is a classic script so chunk-level `return` is legal; js2lua
rejects top-level `return` (JS has none) and accepts it in functions [P2].

Statements are sequenced; block scoping of `local`/`let` is the same shape in
both languages, and shadowing resolves identically.

## 4. Expressions

| Construct         | Lua        | JS             | Lowering                                   |
| ----------------- | ---------- | -------------- | ------------------------------------------ |
| arithmetic        | `+ - * / % ^` | `+ - * / % **` | runtime helpers (coercion matrix §5)      |
| concat            | `..`       | `+` on strings | `luaConcat` / `jsAdd` string path          |
| comparison        | `== ~= < <= > >=` | `=== !== < <= > >=` | strict-only; helpers           |
| logic             | `and or not` | `&& || !`     | thunked helpers; operand-value results     |
| unary             | `- not #`  | `- !` (+ `typeof`) | `-` raw (numbers); `typeof` → helper   |
| calls             | `f(a, b)`  | `f(a, b)`      | direct; multi-return rules [P4]            |
| vararg/spread     | `...`      | `...rest`      | [P4]                                       |
| index             | `t[k]`, `t.k` | `t[k]`, `t.k` | [P3]                                       |
| constructors      | `{...}`    | `{...}`, `[...]` | [P3]                                     |
| functions         | [P2]       | [P2]           |                                            |
| bitwise ops       | —          | —              | [X] v0                                     |
| loose eq (`==`)   | n/a        | —              | [X] (use `===`)                            |
| `??`, `in`, `instanceof`, `,` | — | —      | [X]                                        |
| floor div `//`    | —          | n/a            | [X] v0                                     |

**Precedence note:** emitters never rely on source-language precedence to
preserve semantics: all non-atom operands of emitted raw operators are
parenthesized, and everything semantic goes through helper calls anyway.

## 5. Operator semantics (the coercion matrix)

- **Truthiness** differs and is bridged by helpers in *condition and logic*
  positions: Lua falsy = {nil, false}; JS falsy = {false, 0, -0, NaN, "", null,
  undefined}. `and`/`or`/`&&`/`||` return operand values (not booleans) with
  single evaluation of the left operand (thunk-based).
- **`+`**: Lua coerces numeric *strings* (trimmed, decimal or hex integer);
  other non-numbers are errors. JS: if either ToPrimitive is a string,
  concatenate (canonical formatting); otherwise ToNumber both
  (null→0, true→1, false→0, undefined→NaN, ""→0).
- **Other arithmetic** (`- * / ^`): coerce via ToNumber both directions.
- **`%`**: Lua is *floored* (`-7 % 3 == 2`); JS is *truncated* (`-7 % 2 == -1`).
  Emitters NEVER emit the target's raw `%`: lua2js uses `luaMod`, js2lua uses
  `jsRem`.
- **`^` / `**`**: `Math.pow` / `^` via helpers. Right-associative; binds
  tighter than unary minus in Lua (`-2^2 == -(2^2)`).
- **Concat `..`**: coerces numbers via canonical formatting; right-associative.
- **Equality**: strict by type both directions (`===` shape); tables/functions
  by identity; `0 == -0` is true.
- **Relational**: number-number or string-string only (Lua); JS coerces via
  ToNumber (NaN → false). String comparison is byte/code-unit order (diverges
  from UTF-16 order only outside ASCII — corpus stays ASCII).
- **Length `#`/`.length`**: strings (bytes / UTF-16 units — diverges only
  outside ASCII); array-sequence border for tables [P3]. Holes are **undefined
  behavior**; the corpus and fuzzer must not create them.
- **`typeof`/`type`**: JS spellings ("undefined","object","number","string",
  "boolean","function"); `typeof null === "object"`.

## 6. Numbers and formatting

- Single f64 domain both directions. Integer subtype semantics of Lua 5.3+ are
  **not** modeled (v0): `4/2` is exactly 2, and formatting is canonical.
- **Canonical number→string is JS-style**: shortest round trip; integral
  floats print without `.0`; specials print `Infinity`/`-Infinity`/`NaN`.
  Stock Lua prints `2.0`/`inf`/`nan` — this is the one intentional divergence
  from reference behavior, and corpus cases exercising it use `.expected`
  files (no stock-Lua reference run).
- String→number: trimmed; decimal (with exponent) or hex integer; empty → NaN
  (JS `tonumber` path returns null/nil for unparseable, JS `+x` gives NaN).

## 7. Programs

- A program is a **chunk** (script) — no modules in v0: JS `import`/`export`
  are [X]; Lua `require` is [X] pending module design.
- Globals: references to undeclared names are *runtime globals*: compiled Lua
  output reads them from the seeded JS scope (`print`, `assert`, `tostring`,
  `tonumber`, `type`); compiled JS output reads the js.lua globals (`console`,
  ...). Host embedding APIs for custom globals are [P5].
- stdlib v0 surface: `print`/`console.log`, `assert`, `tostring`, `tonumber`,
  `type`/`typeof`. Growth is by amendment to this document with matching
  runtime implementations on both sides.

## 8. Change log

- v0 (initial): expression core (§4 [v0] rows), chunk statements, two runtimes.
