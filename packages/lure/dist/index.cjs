"use strict";
// lure — JS-side Lua runtime for lua2js output (Luna Subset v0, Phase 3
// surface). Compiled Lua chunks expect these helpers in scope as globals.
// Canonical number formatting is JS-style (see docs/SUBSET.md §Numbers).
Object.defineProperty(exports, "__esModule", { value: true });
exports.LuaTable = void 0;
exports.luaKey = luaKey;
exports.luaTruthy = luaTruthy;
exports.luaNot = luaNot;
exports.luaAnd = luaAnd;
exports.luaOr = luaOr;
exports.luaCond = luaCond;
exports.luaToNumber = luaToNumber;
exports.luaNumToStr = luaNumToStr;
exports.luaToStr = luaToStr;
exports.luaAdd = luaAdd;
exports.luaSub = luaSub;
exports.luaMul = luaMul;
exports.luaDiv = luaDiv;
exports.luaMod = luaMod;
exports.luaPow = luaPow;
exports.luaConcat = luaConcat;
exports.luaEq = luaEq;
exports.luaNe = luaNe;
exports.luaLt = luaLt;
exports.luaLe = luaLe;
exports.luaGt = luaGt;
exports.luaGe = luaGe;
exports.luaTable = luaTable;
exports.luaTableRecord = luaTableRecord;
exports.luaLen = luaLen;
exports.luaGet = luaGet;
exports.luaSet = luaSet;
exports.luaCall = luaCall;
exports.luaMethodCall = luaMethodCall;
exports.print = print;
exports.luaAssert = luaAssert;
exports.tostring = tostring;
exports.tonumber = tonumber;
exports.type = type;
function luaKey(value) {
    return `lua$${value}`;
}
class LuaTable {
    #contents;
    #_metatable = null;
    #didInitMetatable = false;
    #initMetatable() {
        if (this.#didInitMetatable)
            return;
        // Capture the table: `#_metatable` is only accessible on a LuaTable
        // receiver, and inside the getter `this` would be the meta handle.
        const self = this;
        LuaTable.#metas.set(self, {
            get value() {
                return self.#_metatable;
            },
            set value(v) {
                self.#_metatable = v;
            },
        });
        this.#didInitMetatable = true;
    }
    get metatable() {
        return this.#_metatable;
    }
    set metatable(v) {
        if (this.#_metatable !== v) {
            this.#initMetatable();
            this.#_metatable = v;
        }
    }
    static #metas = new WeakMap();
    static getMeta(k) {
        while (k !== undefined && k !== null) {
            if (typeof k === "symbol" || typeof k === "object")
                if (LuaTable.#metas.has(k))
                    return LuaTable.#metas.get(k);
            k = Object.getPrototypeOf(k);
        }
        return;
    }
    constructor(contents = {}) {
        this.#contents = contents;
    }
    get contents() {
        return this.#contents;
    }
}
exports.LuaTable = LuaTable;
// ---------------------------------------------------------------------------
// Truthiness: in Lua only nil (null) and false are falsy. 0, "", NaN truthy.
// ---------------------------------------------------------------------------
function luaTruthy(x) {
    return x !== null && x !== false;
}
function luaNot(x) {
    return !luaTruthy(x);
}
// Short-circuit logic with operand-value results. Operands are thunks so the
// right side is only evaluated when the operator demands it.
function luaAnd(a, b) {
    const av = a();
    return luaTruthy(av) ? b() : av;
}
function luaOr(a, b) {
    const av = a();
    return luaTruthy(av) ? av : b();
}
// Conditional expression (`c and a or b` lowering / ternary): branches are
// thunks so only the taken one runs.
function luaCond(c, a, b) {
    return luaTruthy(c) ? a() : b();
}
// ---------------------------------------------------------------------------
// Coercions
// ---------------------------------------------------------------------------
function luaTypeName(x) {
    if (x === null)
        return "nil";
    if (typeof x === "number")
        return "number";
    if (typeof x === "string")
        return "string";
    if (typeof x === "boolean")
        return "boolean";
    if (typeof x === "function")
        return "function";
    return "table";
}
function luaError(message) {
    // Phase 5 will route this through Lua `error()` semantics with levels and
    // pcall integration; for now a plain JS exception carries the message.
    throw new Error(message);
}
// Lua string->number coercion (as used by arithmetic): trims whitespace,
// accepts decimal (with exponent) and hex integers; rejects empty strings and
// trailing garbage, unlike the permissive JS Number()/parseFloat().
function luaToNumber(x) {
    if (typeof x === "number")
        return x;
    if (typeof x !== "string")
        return null;
    const trimmed = x.trim();
    if (trimmed === "")
        return null;
    if (/^[+-]?0[xX][0-9a-fA-F]+$/.test(trimmed))
        return parseInt(trimmed, 16);
    if (!/^[+-]?(\d+\.?\d*|\.\d+)([eE][+-]?\d+)?$/.test(trimmed))
        return null;
    const n = Number(trimmed);
    return Number.isNaN(n) ? null : n;
}
// Canonical (JS-style) number formatting: shortest round trip, no trailing
// `.0` for integral values, "Infinity"/"NaN" for the specials.
function luaNumToStr(n) {
    if (Number.isNaN(n))
        return "nan"; // stock Lua prints "nan" lowercase
    if (n === Infinity)
        return "inf";
    if (n === -Infinity)
        return "-inf";
    if (n === 0)
        return "0"; // covers -0
    return String(n);
}
function luaToStr(x) {
    if (x === null)
        return "nil";
    if (typeof x === "number")
        return luaNumToStr(x);
    if (typeof x === "string")
        return x;
    if (typeof x === "boolean")
        return x ? "true" : "false";
    if (typeof x === "function")
        return "function: 0x00000000";
    return "table: 0x00000000";
}
function arithOperands(a, b, op) {
    const na = luaToNumber(a);
    if (na === null) {
        return luaError(`attempt to perform arithmetic on a ${luaTypeName(a)} value (${op})`);
    }
    const nb = luaToNumber(b);
    if (nb === null) {
        return luaError(`attempt to perform arithmetic on a ${luaTypeName(b)} value (${op})`);
    }
    return [na, nb];
}
function luaAdd(a, b) {
    const [na, nb] = arithOperands(a, b, "add");
    return na + nb;
}
function luaSub(a, b) {
    const [na, nb] = arithOperands(a, b, "sub");
    return na - nb;
}
function luaMul(a, b) {
    const [na, nb] = arithOperands(a, b, "mul");
    return na * nb;
}
function luaDiv(a, b) {
    const [na, nb] = arithOperands(a, b, "div");
    return na / nb;
}
// Lua `%` is floored modulo (sign of the divisor), unlike JS `%`.
function luaMod(a, b) {
    const [na, nb] = arithOperands(a, b, "mod");
    return na - nb * Math.floor(na / nb);
}
function luaPow(a, b) {
    const [na, nb] = arithOperands(a, b, "pow");
    return Math.pow(na, nb);
}
// Concat coerces numbers via the canonical formatting.
function luaConcat(a, b) {
    const sa = typeof a === "string" ? a : typeof a === "number" ? luaNumToStr(a) : null;
    if (sa === null)
        luaError(`attempt to concatenate a ${luaTypeName(a)} value`);
    const sb = typeof b === "string" ? b : typeof b === "number" ? luaNumToStr(b) : null;
    if (sb === null)
        luaError(`attempt to concatenate a ${luaTypeName(b)} value`);
    return sa + sb;
}
// ---------------------------------------------------------------------------
// Comparison
// ---------------------------------------------------------------------------
// Lua `==`: strict by type, tables/functions by identity, NaN ~= NaN. This is
// exactly JS `===` over our value domain (with null as nil).
function luaEq(a, b) {
    return a === b;
}
function luaNe(a, b) {
    return a !== b;
}
function compareOperands(a, b, op) {
    if (typeof a === "number" && typeof b === "number")
        return [a, b];
    if (typeof a === "string" && typeof b === "string")
        return [a, b];
    return luaError(`attempt to compare ${luaTypeName(a)} with ${luaTypeName(b)} (${op})`);
}
function luaLt(a, b) {
    const [x, y] = compareOperands(a, b, "lt");
    return x < y;
}
function luaLe(a, b) {
    const [x, y] = compareOperands(a, b, "le");
    return x <= y;
}
function luaGt(a, b) {
    return luaLt(b, a);
}
function luaGe(a, b) {
    return luaLe(b, a);
}
// ---------------------------------------------------------------------------
// Table construction / length / table access
// ---------------------------------------------------------------------------
// `luaTable([v1, v2, ...])` — array-style constructor, keys 1..=n (Lua base).
function luaTable(items = []) {
    const contents = {};
    items.forEach((item, i) => {
        if (item !== null)
            contents[i + 1] = item;
    });
    return new LuaTable(contents);
}
// `luaTableRecord([[k1, v1], [k2, v2], ...])` — record-style constructor with
// arbitrary (already evaluated) keys.
function luaTableRecord(pairs) {
    const table = new LuaTable();
    for (const [key, value] of pairs)
        luaSet(table, key, value);
    return table;
}
function luaLen(x) {
    if (typeof x === "string")
        return x.length;
    if (x instanceof LuaTable) {
        // Border semantics: count the contiguous 1-based sequence prefix.
        // Tables with holes are undefined behavior in the subset.
        let n = 0;
        while (x.contents[(n + 1)] !== undefined)
            n++;
        return n;
    }
    return luaError(`attempt to get length of a ${luaTypeName(x)} value`);
}
function luaTableKey(key) {
    if (typeof key === "string")
        return luaKey(key);
    if (typeof key === "number")
        return key;
    return luaError(`table index is ${luaTypeName(key)}`);
}
function luaGet(t, key) {
    if (t === null)
        luaError("attempt to index a nil value");
    if (!(t instanceof LuaTable)) {
        luaError(`attempt to index a ${luaTypeName(t)} value`);
    }
    const v = t.contents[luaTableKey(key)];
    return v === undefined ? null : v;
}
function luaSet(t, key, value) {
    if (t === null)
        luaError("attempt to index a nil value");
    if (!(t instanceof LuaTable)) {
        luaError(`attempt to index a ${luaTypeName(t)} value`);
    }
    if (key === null)
        luaError("table index is nil");
    const k = luaTableKey(key);
    if (value === null) {
        // Assigning nil erases the key.
        delete t.contents[k];
    }
    else {
        t.contents[k] = value;
    }
}
// Calls go through one helper so a host JS `undefined` return (for example
// `print`) is normalized to Lua nil. This also gives non-callable values a
// Lua-shaped error instead of leaking a host TypeError.
function luaCall(func, args) {
    if (typeof func !== "function") {
        luaError(`attempt to call a ${luaTypeName(func)} value`);
    }
    const result = func(...args);
    return result === undefined ? null : result;
}
// Lua method-call sugar passes the receiver as an explicit first argument.
function luaMethodCall(receiver, key, ...args) {
    return luaCall(luaGet(receiver, key), [receiver, ...args]);
}
// ---------------------------------------------------------------------------
// stdlib subset (seeded into globals by embedders / the test harness)
// ---------------------------------------------------------------------------
function print(...args) {
    // Lua print separates with tabs.
    console.log(args.map(luaToStr).join("\t"));
}
function luaAssert(v, message) {
    if (!luaTruthy(v)) {
        luaError(typeof message === "string" ? message : "assertion failed!");
    }
    return v;
}
function tostring(x) {
    return luaToStr(x);
}
function tonumber(x) {
    return luaToNumber(x);
}
function type(x) {
    return luaTypeName(x);
}
