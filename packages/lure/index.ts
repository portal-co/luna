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
  static readonly #metas: WeakMap<WeakKey, { value: LuaObject }> =
    new WeakMap();
  static getMeta(k: any): { value: LuaObject } | undefined {
    while (k !== undefined) {
      if (typeof k in { symbol: true, object: true })
        if (LuaTable.#metas.has(k)) return LuaTable.#metas.get(k)!;
      k = Object.getPrototypeOf(k);
    }
    return;
  }
  constructor(contents: LuaTableContents) {
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
