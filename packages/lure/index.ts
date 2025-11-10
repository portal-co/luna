export type LuaKey = `lua$${string}`;
export interface LuaTableContents {
  [a: LuaKey | number]: LuaObject;
}
export class LuaTable {
  readonly #contents: LuaTableContents;
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
