export type LuaKey = `lua$${string}`;
export interface LuaTableContents {
    [a: LuaKey | number]: LuaObject;
}
export declare class LuaTable {
    #private;
    constructor(contents: LuaTableContents);
    get contents(): LuaTableContents;
}
export type LuaObject = LuaTable | number | string | boolean | ((...args: LuaObject[]) => LuaObject[]) | null;
