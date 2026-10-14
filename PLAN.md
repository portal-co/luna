# Luna: Bidirectional Lua ↔ JS Compiler Plan

Status: proposed
Date: 2026-10-14
Repo: `@portal-solutions/luna` (workspace: `crates/looney`, `packages/lure`)

## 0. Current state

- `crates/looney` — Rust compiler crate. `src/lib.rs` is empty. Dependencies already
  declared: `full_moon` (Lua parser), the SWC stack (`swc_ecma_ast`, `swc_ecma_parser`
  is declared in workspace deps but not yet in the crate — add it for js2lua),
  `swc_common`, `swc_atoms`, `swc_ecma_utils`, `swc_ecma_visit`, `portal-solutions-swibb`.
- `packages/lure` — TypeScript package intended as the JS-side Lua runtime. Contains an
  initial `LuaTable`/`LuaObject` sketch (`luaKey()` string-key prefixing with `lua$`,
  metatable storage via `WeakMap`, prototype-chain metatable lookup in `getMeta`).
  Package name in `package.json` has a typo (`@portal-solutioms/...`) — fix as part of
  Phase 1.
- Toolchain available on this host: `node` v26, system `lua` 5.5.0 (`/opt/homebrew/bin/lua`),
  `cargo` 1.99.
- Parent cargo patch config exists at `../.cargo/config.toml`
  (`~/gx/Code-local/portal-hot/.cargo/config.toml`). **Never** add a `.cargo/config.toml`
  inside this repo; any new `patch` entries must be migrated into the parent config.

## 1. Goal

Two compilers and two runtimes, built around **one explicitly-defined shared subset**
("Luna Subset", versioned, starting at v0):

1. **lua2js** — Lua source → JavaScript, executed against the **`lure`** JS runtime.
2. **js2lua** — JavaScript source → Lua (the *same* subset, mirrored), executed against
   a new **`js.lua`** Lua runtime (shipped as an embedded Lua source string from the
   `looney` crate, and/or a checked-in file prepended to output).

"Same subset" is enforced structurally: both frontends lower into a single shared IR
(`ir.rs` in `looney`). Each compiler is `parse → lower to IR → validate → emit`.
The IR *is* the subset definition made executable; anything outside the subset fails
lowering with a targeted diagnostic (not a panic).

### Non-goals (v0)

- Full Lua 5.x semantics: coroutines, full stdlib, full metatable machinery, `goto`,
  integer subtype (Lua 5.3+ integers vs floats — v0 is f64-only).
- Full JS semantics: prototypes/classes beyond the mapping in §5, `eval`, `with`,
  getters/setters, `Proxy`, generators/async, typed arrays, `var` hoisting
  (subset is `let`/`const` only), `this` outside method-call form.
- Performance. Correctness helpers (`luaAdd`, `jsAdd`, …) are emitted liberally;
  a later optimizer pass can inline fast paths when operand types are provably known.
- Source maps (track as a stretch goal in Phase 6).

## 2. Repository layout (target)

```text
crates/looney/
  src/
    lib.rs          # public API: compile_lua_to_js(), compile_js_to_lua()
    ir.rs           # LunaIR: the shared subset AST + validator + pretty-printer
    lua2js/
      mod.rs        # full_moon AST -> IR
      emit.rs       # IR -> swc_ecma_ast -> JS text (swc_ecma_codegen)
    js2lua/
      mod.rs        # swc_ecma_ast -> IR
      emit.rs       # IR -> Lua text (hand-rolled emitter, full_moon is parse-only)
    runtime/
      js.lua        # Lua-side JS runtime (include_str!'d into output)
  tests/
    cases_l2j/*.lua       # behavioral corpus for lua2js
    cases_j2l/*.js        # behavioral corpus for js2lua
    cases_roundtrip/*     # sources compiled both ways
    golden/               # snapshot tests of emitted code
packages/lure/      # JS-side Lua runtime (extends existing LuaTable sketch)
docs/
  SUBSET.md         # human-readable subset spec (source of truth for ir.rs validator)
tests/              # (cargo integration harness lives in crates/looney/tests)
PLAN.md             # this file
```

## 3. The Luna Subset (v0)

The normative spec lands in `docs/SUBSET.md` (Phase 0). Summary:

**Values:** nil/undefined, boolean, number (f64 both sides), string, table/object,
function. `nil` in Lua ↔ `null` in JS (matches the existing `lure` `LuaObject` type).
`undefined` appears only on the JS side as "absent property" and is mapped through
sentinels (see §5).

**Statements:** local declaration (`local x` / `let`/`const`), assignment (single
target; multiple assignment is Phase 4), expression statement (calls only),
`if`/`elseif`/`else`, `while`, numeric `for` (Lua) ↔ classic `for(;;)` (JS),
`do`/`{}` block, `return`, `break`.

**Expressions:** literals (nil/bool/number/string), variable reference,
table/object literal (array-style + record-style fields), index (`t[k]`, `t.k`),
call, method call (`a:m(x)` ↔ `a.m(x)` — see §5.4), unary (`-`, `not`/`!`, `#`),
binary arithmetic (`+ - * / % ^`), comparison (`== ~= < <= > >=` / `=== !== < <= > >=`),
logic (`and or not` / `&& || !`), concatenation (`..` Lua-side; JS-side the subset
spelling is template/`+` on strings — lowered to the same IR node).

**Functions:** named and anonymous, closures over upvalues (both languages have
proper closures — direct mapping), fixed parameters, `...` varargs ↔ `...rest`
(Phase 4), multiple returns ↔ array-packed returns via runtime sentinel (Phase 4).

**Iteration (v0):** numeric loops only, plus `for` over array part via runtime
helpers (`luaIpairs` in JS output; `jsArrayEach`-style in Lua output). Generic
`for`/`for...of`/`for...in` arrive with the stdlib/metatable phase.

**Explicitly excluded (v0):** `repeat..until`, `goto`, method *definitions* with
implicit `self` (callers write explicit first param), metatables as language
surface (runtime may use them internally), integer division `//`, bitwise ops
(deferred — mapping exists (`|`→`|`) but precedence/coercion corner cases punt),
JS `switch`, `try` (until Phase 5 `pcall`/`error` ↔ `try`/`throw`), destructuring,
spread, classes-as-syntax (constructor+prototype pattern maps through the object
model instead), `==` (loose) in JS source — subset requires `===`.

## 4. Architecture

```text
Lua source ──full_moon──▶ Lua AST ──┐
                                    ├──▶ LunaIR ──validate──▶ ┬── emit JS (swc) ──▶ JS + lure
JS source ──swc parser──▶ JS AST ───┘                         └── emit Lua (own) ──▶ Lua + js.lua
```

- **`ir.rs`** defines `IrStmt`, `IrExpr`, `IrFunction`, with source spans from both
  frontends (`swc_common::Span` / full_moon ranges normalized to `(u32, u32)`).
- **Validator** walks IR and rejects anything a backend can't emit; this is what
  makes "same subset" checkable rather than aspirational. `cargo test` includes a
  property: every node kind in `ir.rs` has both an emitter arm and a validator arm
  (exhaustiveness enforced by refusing wildcard match arms).
- **lua2js lowering**: mostly mechanical; all Lua-specific semantics are pushed into
  `lure` helper calls (`luaAdd(a,b)`, `luaLen(t)`, `luaIndex(t,k)`, …) so the JS AST
  stays dumb. This keeps the swc surface small: we need expression/statement
  construction + `swc_ecma_codegen` printing, not transforms.
- **js2lua lowering**: same philosophy — JS semantics pushed into `js.lua` helpers
  (`jsAdd`, `jsTruthy`, `jsIndex`, `jsCall`, …). The Lua emitter is hand-rolled with
  an explicit precedence table (do **not** rely on full_moon for output; it is a
  lossless *parser*, not a codegen library).
- **Identifier hygiene (JS side)**: per workspace rules, **never** flatten a swc
  `Ident`/`Id` (`(Atom, SyntaxContext)`) to `String` for scope bookkeeping — key
  scope maps by `Id` and compare with `Id` equality. `String` is only valid for
  genuinely textual domains (property names, module paths, emitted Lua names).
- **Name mangling (Lua side)**: Lua locals/params map to JS `let` bindings; escape
  via `lua$<name>` when the name collides with a JS reserved word or runtime helper
  namespace. JS→Lua: escape JS identifiers that collide with Lua keywords
  (`end`, `local`, …) as `<name>_` and record the mapping in a header comment.
- **Globals**: Lua globals compile to accesses on an explicit `globals` LuaTable
  passed in by the embedder (default: a fresh table seeded with stdlib), *not*
  `globalThis`. JS globals compile to `_ENV`-style accesses on a `jsGlobals` table
  in `js.lua` seeded with the JS stdlib shims (`console`, `Math`, …).

## 5. Semantic mapping (the hard part)

Both directions must agree on every row of this table; the runtimes implement them.

### 5.1 Truthiness
- Lua: only `nil`/`false` are falsy. JS: also `0`, `""`, `NaN`, `null`, `undefined`.
- lua2js: every condition position wraps in `luaTruthy(x)`; `and`/`or` become
  `luaAnd(()=>a, ()=>b)`-style thunks (short-circuit + operand-value return).
- js2lua: every condition wraps in `jsTruthy(x)`; `&&`/`||` likewise thunked.

### 5.2 Operators
- `+`: Lua coerces numeric strings; JS coerces *everything* to string if either side
  is a string. Both compile to runtime helpers (`luaAdd`, `jsAdd`) implementing the
  respective coercion matrix. `- * / % ^` similarly (`luaArith`/`jsArith`), except
  where the validator can prove both operands are number-typed literals — then emit
  the raw operator (with `%` → `luaMod`/`jsRem`: Lua `%` is floored, JS `%` is
  truncated; they differ for negative operands, so never emit raw).
- `^`: right-associative, binds tighter than unary minus in Lua (`-2^2 == -(2^2)`).
  Emit `Math.pow` / a `luaPow` helper; JS has no native `^` (it's xor — js2lua must
  reject `^` in JS source at lowering time, or map `**`).
- `..`: right-associative, coerces numbers via Lua's `tostring` rules → `luaConcat`.
  JS string `+` in the subset is spelled the same IR node; js2lua emits `jsConcat`.
- Equality: Lua `==` is strict-by-type with raw table identity → `luaEq`
  (`Object.is`-ish, `NaN ~= NaN`, tables by reference). JS `===` maps to `jsStrictEq`
  on the Lua side. Loose `==`/`~=` coercion corners are rejected in the subset.
- Length: Lua `#t` → `luaLen` (sequence border semantics; subset constrains tables
  with holes to *undefined* — tests must not depend on it). JS `.length` on
  arrays/strings → `jsLen` on the Lua side.

### 5.3 nil / undefined / null / table erasure
- `nil` ↔ `null` (per existing `lure` `LuaObject`). Assigning `nil` to a table key
  *erases* it → `luaSet(t, k, v)` must `delete` the key when `v === null`, and
  `luaGet` returns `null` for absent keys.
- JS `undefined`: represented on the Lua side by a unique sentinel table
  `jsUndefined` (a Lua table cannot store `nil`; and `nil` is already `null`).
  `jsGet` maps missing properties to `jsUndefined`; `typeof x === "undefined"`
  checks against the sentinel. This asymmetry is documented in SUBSET.md.
- Arrays: Lua sequences are 1-based and stored in the table itself; JS arrays in the
  js2lua direction are wrapped (`JSArray` with metatable) keeping **0-based**
  indexing — do not "fix" bases, that breaks identity/aliasing. SUBSET.md forbids
  assuming one base for the other language's containers.

### 5.4 Functions & calls
- Lua multi-return: expression positions truncate to 1 value; only the last
  expression in `return`/call-args expands. JS encoding: functions return a single
  value normally; multi-return returns a `LuaMulti extends Array` sentinel which
  `luaCall`/`luaReturn` unpack at expansion sites. js2lua: JS single returns map
  directly; `js.lua` provides `jsUnpack`-style helpers for array-spread calls if the
  subset grows `...`.
- Method call: `a:m(x)` desugars in IR to `Call(Index(a,"m"), [a, x])` — i.e. both
  sides lower to the same explicit-receiver form. `this` in JS source is only legal
  inside functions the validator marks as method-shaped (called via property), and
  lowers to the explicit `self` parameter. (Keeps `this` binding rules out of v0.)
- Closures: direct both ways (Lua upvalues ≈ JS closures). Mutation of captured
  locals is safe in both — no boxing needed. Verify with tests capturing loop vars.

### 5.5 Objects / metatables / prototypes (Phase 5)
- `lure` already sketches metatable storage (`WeakMap` + prototype walk). Extend
  `luaGet`/`luaSet` to honor `__index`/`__newindex` (table or function forms).
- js2lua: `js.lua` implements prototype chains as metatable `__index` chains on
  `JSObject`; `new F(...)` → `jsNew(F, ...)` allocating a table whose metatable
  points at `F.prototype`. Classes in JS source remain out-of-subset; the
  constructor-function pattern is the supported spelling.

### 5.6 Numbers & strings formatting
- f64 everywhere. `tostring(number)` differs: Lua prints `3.0` for integral floats
  (5.3+), JS prints `"3"`. The subset defines **JS-style** shortest-round-trip
  formatting as canonical (both runtimes implement `luaNumToStr`/`jsNumToStr`
  accordingly) — document deviation from stock Lua in SUBSET.md. String→number
  (`tonumber`/`Number`) accepts the common grammar only (decimal, exponent, hex
  `0x`); rejects octal quirks and `0b`/`0o` on the Lua side.

## 6. Runtimes

### 6.1 `packages/lure` (JS runtime for lua2js output)
Extend the existing sketch into modules (single-file build via existing `zshy`):
`table.ts` (`LuaTable`, `luaGet/luaSet/luaLen`, `luaKey` string interning — keep the
`lua$` prefix scheme), `ops.ts` (`luaTruthy/luaAdd/luaArith/luaMod/luaPow/luaConcat/
luaEq/luaLt/luaLe`), `multi.ts` (`LuaMulti`), `stdlib.ts` (`print`, `tostring`,
`tonumber`, `type`, `ipairs`/`pairs` (Phase 5), `table.*`, `string.*`, `math.*`
subset matching SUBSET.md). Invariants: `null` is the only nil; helpers never throw
on Lua-legal input (errors are Lua errors → `error()` semantics in Phase 5).

### 6.2 `crates/looney/src/runtime/js.lua` (Lua runtime for js2lua output)
`jsTruthy`, `jsAdd`/`jsArith`/`jsRem`/`jsConcat`, `jsStrictEq`/`jsEq`/`jsTypeof`,
`jsGet`/`jsSet`/`jsLen` (with `jsUndefined` sentinel), `jsNew`, `jsCall` (receiver
handling), `JSObject`/`JSArray` wrappers (metatable-backed, prototype chain),
stdlib shims: `console.log` (→ `print` with JS formatting), `Math.*`, `String.*`,
`Array` method subset (`push/pop/shift/unshift/slice/indexOf/map/filter/reduce` —
mirror exactly the list SUBSET.md blesses), `JSON.parse/stringify` (Phase 5).
Shipped via `include_str!` and prepended to js2lua output unless `--no-runtime`.

## 7. Testing strategy

- **Runner crates**: add dev-dependency `mlua = { version = "0.10", features = ["lua54", "vendored"] }`
  for in-process Lua execution (deterministic across CI; system lua here is 5.5,
  full_moon targets ≤5.4 grammars — vendored 5.4 avoids the skew). JS execution via
  `node` subprocess (already on PATH) with a small wrapper returning stdout/exit.
- **Golden tests**: each corpus file's emitted JS/Lua is snapshotted; review diffs in
  PRs (these double as readable documentation of the lowering).
- **Behavioral matrix** (per corpus case, automated):
  1. Lua case → run under `mlua` (reference) and lua2js+node → **stdout must match**.
  2. JS case → run under `node` (reference) and js2lua+mlua → **stdout must match**.
  3. Round-trip: Lua case → lua2js → (js2lua on emitted output is *not* expected to
     parse — emitted code uses runtime idioms outside the source subset; instead)
     round-trip is **source-level**: for every case that exists in both languages
     (`cases_roundtrip/`), both directions produce matching stdout on the same input.
- **Conformance**: import the subset-compatible slice of the official Lua 5.4 test
  suite (allowlist files; most of `calls.lua`, `locals.lua`, `constructs.lua` early
  sections qualify) and a small Test262 allowlist later (Phase 6).
- **Fuzz/differential (Phase 6)**: generate random subset programs (small expression
  grammar fuzzer in `tests/fuzz.rs`), assert cross-runtime stdout equality.

## 8. Phases & tasks

### Phase 0 — Spec & scaffolding ✅
- [x] Write `docs/SUBSET.md` from §3/§5 (normative; versioned v0).
- [x] `ir.rs` skeleton + validator skeleton + exhaustiveness test.
- [x] Test harness: `mlua` dev-dep, node runner, golden-snapshot util, corpus dirs.
- [x] Fix `packages/lure/package.json` name typo; confirm `build.sh`/`zshy` build.
- Done when: CI-green empty pipeline compiling `return 1+1` both directions to a
  hardcoded stub, harness executes both outputs.
- Landed (2026-10-14): full pipeline, not a stub — see Phase 1.

### Phase 1 — Expression core (both directions) ✅
- [x] IR: literals, varref, binops, unops, `local`/assignment, `return`, `do`, call stmt.
- [x] lure: `ops.ts` + `table.ts` minimal (`luaGet/luaSet/luaLen`, no metatables).
  (Landed as sections of the single-file `packages/lure/index.ts`; split into
  modules deferred until Phase 3 makes the file large enough to need it.)
- [x] js.lua: `jsTruthy/jsAdd/jsStrictEq/jsGet/jsSet`, `jsUndefined`, `console.log`.
- [x] lua2js + js2lua emitters for the above.
- Done when: arithmetic/comparison/truthiness/concat corpus passes the behavioral
  matrix both directions; `-2^2`, `1/0` (Infinity), `0/0` (NaN), negative `%`
  cases pinned by tests.
- Landed (2026-10-14): 13 tests green (`cargo test -p looney`), 7 l2j + 8 j2l
  corpus cases through the full matrix; `lure` builds via zshy.
- Scope-tracking note (beyond plan): both frontends track local scopes so that
  only *local* bindings are name-mangled — runtime globals (`print`, `assert`,
  `console`) pass through unmangled. JS-side scopes are keyed by swc `Id`,
  never flattened strings (workspace hygiene rule).
- Also fixed a pre-existing lure bug: the metatable WeakMap getter captured the
  wrong `this` (`#_metatable` access would throw), and `getMeta` walked past
  `null` prototypes.

### Phase 2 — Control flow & functions
- [ ] `if/elseif/else`, `while`, numeric `for` ↔ `for(;;)` (incl. step, inclusive
  bounds, loop-var capture semantics), `break`.
- [ ] Function decl/expr, params, `return` (single value), recursion, closures,
  mutated upvalues.
- Done when: fib (recursive + iterative), fizzbuzz, closure-counter, loop-capture
  cases pass both directions; golden files reviewed.

### Phase 3 — Tables/objects & calls
- [ ] Table/object literals (array + record + mixed), indexing (incl. string keys,
  numeric keys, computed), assignment through index, `#`/`.length`.
- [ ] Method calls via explicit-receiver lowering; argument truncation/fill-nil.
- [ ] `lure`: `LuaTable` contents access honoring the `lua$` key scheme + numeric keys;
  `js.lua`: `JSObject`/`JSArray` wrappers.
- Done when: matrix/record/nested-table corpus passes; aliasing tests (shared table
  mutation visible across variables) pass both directions.

### Phase 4 — Varargs, multi-return, multiple assignment
- [ ] `...` ↔ `...rest`, `LuaMulti` sentinel + expansion rules (only in tail position).
- [ ] Multiple assignment `a, b = b, a` (RHS evaluated before any store — temp
  allocation in both emitters).
- Done when: swap, multi-return truncation/expansion, vararg forwarding corpus passes.

### Phase 5 — Metatables/prototypes, errors, stdlib parity
- [ ] `__index`/`__newindex` (lure), prototype chains + `new` (js.lua).
- [ ] `error`/`pcall` ↔ `throw`/`try..catch`; stack-trace fidelity is best-effort.
- [ ] stdlib: `string`/`table`/`math` subset (lure) and `String`/`Array`/`Math`/`JSON`
  subset (js.lua) exactly as SUBSET.md lists.
- Done when: OOP-style corpus (account/class pattern) and pcall/try corpus passes.

### Phase 6 — Hardening
- [ ] Lua 5.4 test-suite allowlist import; Test262 mini-allowlist.
- [ ] Differential fuzzer; fix what it finds; document residual divergences in
  `docs/DIVERGENCES.md`.
- [ ] README quickstart, API docs for `compile_*`, example embedder.
- Stretch: source maps, raw-operator fast paths behind a proven-types analysis.

## 9. Risks / corner cases to watch

1. **Number formatting** (`tostring(3.0)` vs `"3"`) — biggest source of stdout
   mismatch; canonicalized to JS style (§5.6) but stock-Lua reference runs under
   `mlua` will disagree on float cases → corpus must normalize or avoid.
2. **`%` sign semantics** (floored vs truncated) — never emit raw `%` for Lua.
3. **Operator precedence/associativity** for `^` and `..` (right-assoc, and `^` vs
   unary minus) — emitter precedence tables must be table-driven and unit-tested.
4. **Table holes / `#` border** — subset declares behavior undefined; keep fuzzer
   from generating holey tables or exclude `#` on them.
5. **JS `this`** — restricted to method-shaped calls; anything else fails lowering.
6. **Hoisting** — JS function declarations hoist, Lua `local function` does not
   (recursion works via upvalue, forward calls don't). Subset: treat JS function
   decls as `const` arrow assigned at point of declaration; reject use-before-decl.
7. **swc `Ident` hygiene** — key all scope bookkeeping by `Id`, never `String`
   (workspace rule; silent binding conflation otherwise).
8. **full_moon grammar version** — pin Lua 5.4 syntax in corpus; system lua is 5.5,
   `mlua` vendored is 5.4; do not rely on system `lua` for tests.
9. **LuLu/network** — `cargo`/`npm` may hit new domains on first fetch; if a network
   error appears, treat as LuLu approval needed, not "no network".
10. **Cargo patches** — must go in `../.cargo/config.toml` (parent), never in-repo.

## 10. Conventions

- Commits: `[AI] ...` prefix (workspace default).
- This plan is a living document: update status checkboxes as phases land; material
  deviations get an entry in a `## Changelog` section appended below.

## Changelog

- 2026-10-14: Phase 0 + Phase 1 landed. Deviations from the plan text:
  (a) `lure` runtime stayed single-file (`index.ts`) instead of splitting into
  `ops.ts`/`table.ts`/... — split when Phase 3 makes it worth it;
  (b) harness executes lure's `index.ts` directly via node 26 type-stripping
  instead of building `dist/` first — no npm build needed for `cargo test`;
  (c) frontends carry explicit local-scope stacks (Id-keyed on the swc side)
  so name mangling only touches locals, never runtime globals;
  (d) `docs/SUBSET.md` gained the authoritative coercion tables (§5) — treat
  it, not this plan, as the semantic source of truth going forward.
