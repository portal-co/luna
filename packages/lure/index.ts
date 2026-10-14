// lure — JS-side Lua runtime for lua2js output (Luna Subset v0, Phase 1
// surface). Compiled Lua chunks expect these helpers in scope as globals.
// Canonical number formatting is JS-style (see docs/SUBSET.md §Numbers).

// The workspace tsconfig is ESNext-lib only (no DOM/Node types); declare the
// one host global we use.
declare const console: { log(...args: any[]): void };

// ---------------------------------------------------------------------------
// LuaObject: the subset's value domain. `null` is Lua nil.
// ---------------------------------------------------------------------------

export type LuaKey<T extends string = string> = `lua$${T}`;
export function luaKey<T extends string = string>(value: T): LuaKey<T> {
  return `lua$${value}`;
}
export interface LuaTableContents {
  [a: LuaKey | number]: LuaObject;
}
export class LuaTable {
  readonly #contents: LuaTableContents;
  #_metatable: LuaObject = null;
  #didInitMetatable: boolean = false;
  #initMetatable() {
    if (this.#didInitMetatable) return;
    // Capture the table: `#_metatable` is only accessible on a LuaTable
    // receiver, and inside the getter `this` would be the meta handle.
    const self = this;
    LuaTable.#metas.set(self, {
      get value() {
        return self.#_metatable;
      },
      set value(v: LuaObject) {
        self.#_metatable = v;
      },
    });
    this.#didInitMetatable = true;
  }
  get metatable() {
    return this.#_metatable;
  }
  set metatable(v: LuaObject) {
    if (this.#_metatable !== v) {
      this.#initMetatable();
      this.#_metatable = v;
    }
  }
  static readonly #metas: WeakMap<WeakKey, { value: LuaObject }> =
    new WeakMap();
  static getMeta(k: any): { value: LuaObject } | undefined {
    while (k !== undefined && k !== null) {
      if (typeof k === "symbol" || typeof k === "object")
        if (LuaTable.#metas.has(k)) return LuaTable.#metas.get(k)!;
      k = Object.getPrototypeOf(k);
    }
    return;
  }
  constructor(contents: LuaTableContents = {}) {
    this.#contents = contents;
  }
  get contents() {
    return this.#contents;
  }
}
export type LuaObject =
  | LuaTable
  | number
  | string
  | boolean
  | ((...args: LuaObject[]) => LuaObject[])
  | null;

// ---------------------------------------------------------------------------
// Truthiness: in Lua only nil (null) and false are falsy. 0, "", NaN truthy.
// ---------------------------------------------------------------------------

export function luaTruthy(x: LuaObject): boolean {
  return x !== null && x !== false;
}

export function luaNot(x: LuaObject): boolean {
  return !luaTruthy(x);
}

// Short-circuit logic with operand-value results. Operands are thunks so the
// right side is only evaluated when the operator demands it.
export function luaAnd(a: () => LuaObject, b: () => LuaObject): LuaObject {
  const av = a();
  return luaTruthy(av) ? b() : av;
}
export function luaOr(a: () => LuaObject, b: () => LuaObject): LuaObject {
  const av = a();
  return luaTruthy(av) ? av : b();
}

// ---------------------------------------------------------------------------
// Coercions
// ---------------------------------------------------------------------------

function luaTypeName(x: LuaObject): string {
  if (x === null) return "nil";
  if (typeof x === "number") return "number";
  if (typeof x === "string") return "string";
  if (typeof x === "boolean") return "boolean";
  if (typeof x === "function") return "function";
  return "table";
}

function luaError(message: string): never {
  // Phase 5 will route this through Lua `error()` semantics with levels and
  // pcall integration; for now a plain JS exception carries the message.
  throw new Error(message);
}

// Lua string->number coercion (as used by arithmetic): trims whitespace,
// accepts decimal (with exponent) and hex integers; rejects empty strings and
// trailing garbage, unlike the permissive JS Number()/parseFloat().
export function luaToNumber(x: LuaObject): number | null {
  if (typeof x === "number") return x;
  if (typeof x !== "string") return null;
  const trimmed = x.trim();
  if (trimmed === "") return null;
  if (/^[+-]?0[xX][0-9a-fA-F]+$/.test(trimmed)) return parseInt(trimmed, 16);
  if (!/^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/.test(trimmed)) return null;
  const n = Number(trimmed);
  return Number.isNaN(n) ? null : n;
}

// Canonical (JS-style) number formatting: shortest round trip, no trailing
// `.0` for integral values, "Infinity"/"NaN" for the specials.
export function luaNumToStr(n: number): string {
  if (Number.isNaN(n)) return "nan"; // stock Lua prints "nan" lowercase
  if (n === Infinity) return "inf";
  if (n === -Infinity) return "-inf";
  if (n === 0) return "0"; // covers -0
  return String(n);
}

export function luaToStr(x: LuaObject): string {
  if (x === null) return "nil";
  if (typeof x === "number") return luaNumToStr(x);
  if (typeof x === "string") return x;
  if (typeof x === "boolean") return x ? "true" : "false";
  if (typeof x === "function") return "function: 0x00000000";
  return "table: 0x00000000";
}

// ---------------------------------------------------------------------------
// Arithmetic: Lua coerces numeric strings; anything else is an error.
// ---------------------------------------------------------------------------

type ArithOp = "add" | "sub" | "mul" | "div" | "mod" | "pow";

function arithOperands(a: LuaObject, b: LuaObject, op: ArithOp): [number, number] {
  const na = luaToNumber(a);
  if (na === null) {
    return luaError(
      `attempt to perform arithmetic on a ${luaTypeName(a)} value (${op})`,
    );
  }
  const nb = luaToNumber(b);
  if (nb === null) {
    return luaError(
      `attempt to perform arithmetic on a ${luaTypeName(b)} value (${op})`,
    );
  }
  return [na, nb];
}

export function luaAdd(a: LuaObject, b: LuaObject): number {
  const [na, nb] = arithOperands(a, b, "add");
  return na + nb;
}
export function luaSub(a: LuaObject, b: LuaObject): number {
  const [na, nb] = arithOperands(a, b, "sub");
  return na - nb;
}
export function luaMul(a: LuaObject, b: LuaObject): number {
  const [na, nb] = arithOperands(a, b, "mul");
  return na * nb;
}
export function luaDiv(a: LuaObject, b: LuaObject): number {
  const [na, nb] = arithOperands(a, b, "div");
  return na / nb;
}
// Lua `%` is floored modulo (sign of the divisor), unlike JS `%`.
export function luaMod(a: LuaObject, b: LuaObject): number {
  const [na, nb] = arithOperands(a, b, "mod");
  return na - nb * Math.floor(na / nb);
}
export function luaPow(a: LuaObject, b: LuaObject): number {
  const [na, nb] = arithOperands(a, b, "pow");
  return Math.pow(na, nb);
}

// Concat coerces numbers via the canonical formatting.
export function luaConcat(a: LuaObject, b: LuaObject): string {
  const sa =
    typeof a === "string" ? a : typeof a === "number" ? luaNumToStr(a) : null;
  if (sa === null) luaError(`attempt to concatenate a ${luaTypeName(a)} value`);
  const sb =
    typeof b === "string" ? b : typeof b === "number" ? luaNumToStr(b) : null;
  if (sb === null) luaError(`attempt to concatenate a ${luaTypeName(b)} value`);
  return sa + sb;
}

// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------

// Lua `==`: strict by type, tables/functions by identity, NaN ~= NaN. This is
// exactly JS `===` over our value domain (with null as nil).
export function luaEq(a: LuaObject, b: LuaObject): boolean {
  return a === b;
}
export function luaNe(a: LuaObject, b: LuaObject): boolean {
  return a !== b;
}

function compareOperands(
  a: LuaObject,
  b: LuaObject,
  op: "lt" | "le",
): [number, number] | [string, string] {
  if (typeof a === "number" && typeof b === "number") return [a, b];
  if (typeof a === "string" && typeof b === "string") return [a, b];
  return luaError(
    `attempt to compare ${luaTypeName(a)} with ${luaTypeName(b)} (${op})`,
  );
}

export function luaLt(a: LuaObject, b: LuaObject): boolean {
  const [x, y] = compareOperands(a, b, "lt");
  return x < y;
}
export function luaLe(a: LuaObject, b: LuaObject): boolean {
  const [x, y] = compareOperands(a, b, "le");
  return x <= y;
}
export function luaGt(a: LuaObject, b: LuaObject): boolean {
  return luaLt(b, a);
}
export function luaGe(a: LuaObject, b: LuaObject): boolean {
  return luaLe(b, a);
}

// ---------------------------------------------------------------------------
// Length / table access (table support is minimal until Phase 3)
// ---------------------------------------------------------------------------

export function luaLen(x: LuaObject): number {
  if (typeof x === "string") return x.length;
  if (x instanceof LuaTable) {
    // Border semantics: count the contiguous 1-based sequence prefix.
    // Tables with holes are undefined behavior in the subset.
    let n = 0;
    while (x.contents[(n + 1) as 1] !== undefined) n++;
    return n;
  }
  return luaError(`attempt to get length of a ${luaTypeName(x)} value`);
}

function luaTableKey(key: LuaObject): LuaKey | number {
  if (typeof key === "string") return luaKey(key);
  if (typeof key === "number") return key;
  return luaError(`table index is ${luaTypeName(key)}`);
}

export function luaGet(t: LuaObject, key: LuaObject): LuaObject {
  if (t === null) luaError("attempt to index a nil value");
  if (!(t instanceof LuaTable)) {
    luaError(`attempt to index a ${luaTypeName(t)} value`);
  }
  const v = t.contents[luaTableKey(key)];
  return v === undefined ? null : v;
}

export function luaSet(t: LuaObject, key: LuaObject, value: LuaObject): void {
  if (t === null) luaError("attempt to index a nil value");
  if (!(t instanceof LuaTable)) {
    luaError(`attempt to index a ${luaTypeName(t)} value`);
  }
  if (key === null) luaError("table index is nil");
  const k = luaTableKey(key);
  if (value === null) {
    // Assigning nil erases the key.
    delete t.contents[k];
  } else {
    t.contents[k] = value;
  }
}

// ---------------------------------------------------------------------------
// stdlib subset (seeded into globals by embedders / the test harness)
// ---------------------------------------------------------------------------

export function print(...args: LuaObject[]): void {
  // Lua print separates with tabs.
  console.log(args.map(luaToStr).join("\t"));
}

export function luaAssert(v: LuaObject, message?: LuaObject): LuaObject {
  if (!luaTruthy(v)) {
    luaError(typeof message === "string" ? message : "assertion failed!");
  }
  return v;
}

export function tostring(x: LuaObject): string {
  return luaToStr(x);
}

export function tonumber(x: LuaObject): number | null {
  return luaToNumber(x);
}

export function type(x: LuaObject): string {
  return luaTypeName(x);
}
