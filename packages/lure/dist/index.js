export function luaKey(value) {
    return `lua$${value}`;
}
export class LuaTable {
    #contents;
    #_metatable = null;
    #didInitMetatable = false;
    #initMetatable() {
        if (!this.#didInitMetatable)
            LuaTable.#metas.set(this, {
                get value() {
                    return this.#_metatable;
                },
                set value(v) {
                    this.#_metatable = v;
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
        while (k !== undefined) {
            if (typeof k in { symbol: true, object: true })
                if (LuaTable.#metas.has(k))
                    return LuaTable.#metas.get(k);
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
