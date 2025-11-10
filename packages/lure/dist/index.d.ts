export type LuaKey<T extends string = string> = `lua$${T}`;
export declare function luaKey<T extends string = string>(value: T): LuaKey<T>;
export interface LuaTableContents {
    [a: LuaKey | number]: LuaObject;
}
export declare class LuaTable {
    #private;
    get metatable(): LuaObject;
    set metatable(v: LuaObject);
    static getMeta(k: any): {
        value: LuaObject;
    } | undefined;
    constructor(contents: LuaTableContents);
    get contents(): LuaTableContents;
}
export type LuaObject = LuaTable | number | string | boolean | ((...args: LuaObject[]) => LuaObject[]) | null;
