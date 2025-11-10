"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.LuaTable = void 0;
exports.luaKey = luaKey;
function luaKey(value) {
    return `lua$${value}`;
}
class LuaTable {
    #contents;
    static #metas = new WeakMap();
    static getMeta(k) {
        while (k !== undefined) {
            if (typeof k in { symbol: true, object: true })
                if (LuaTable.#metas.has(k))
                    return [LuaTable.#metas.get(k)];
            k = Object.getPrototypeOf(k);
        }
        return;
    }
    constructor(contents) {
        this.#contents = contents;
    }
    get contents() {
        return this.#contents;
    }
}
exports.LuaTable = LuaTable;
