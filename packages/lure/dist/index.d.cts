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
    constructor(contents?: LuaTableContents);
    get contents(): LuaTableContents;
}
export type LuaObject = LuaTable | number | string | boolean | ((...args: LuaObject[]) => LuaObject[]) | null;
export declare function luaTruthy(x: LuaObject): boolean;
export declare function luaNot(x: LuaObject): boolean;
export declare function luaAnd(a: () => LuaObject, b: () => LuaObject): LuaObject;
export declare function luaOr(a: () => LuaObject, b: () => LuaObject): LuaObject;
export declare function luaCond(c: LuaObject, a: () => LuaObject, b: () => LuaObject): LuaObject;
export declare function luaToNumber(x: LuaObject): number | null;
export declare function luaNumToStr(n: number): string;
export declare function luaToStr(x: LuaObject): string;
export declare function luaAdd(a: LuaObject, b: LuaObject): number;
export declare function luaSub(a: LuaObject, b: LuaObject): number;
export declare function luaMul(a: LuaObject, b: LuaObject): number;
export declare function luaDiv(a: LuaObject, b: LuaObject): number;
export declare function luaMod(a: LuaObject, b: LuaObject): number;
export declare function luaPow(a: LuaObject, b: LuaObject): number;
export declare function luaConcat(a: LuaObject, b: LuaObject): string;
export declare function luaEq(a: LuaObject, b: LuaObject): boolean;
export declare function luaNe(a: LuaObject, b: LuaObject): boolean;
export declare function luaLt(a: LuaObject, b: LuaObject): boolean;
export declare function luaLe(a: LuaObject, b: LuaObject): boolean;
export declare function luaGt(a: LuaObject, b: LuaObject): boolean;
export declare function luaGe(a: LuaObject, b: LuaObject): boolean;
export declare function luaLen(x: LuaObject): number;
export declare function luaGet(t: LuaObject, key: LuaObject): LuaObject;
export declare function luaSet(t: LuaObject, key: LuaObject, value: LuaObject): void;
export declare function print(...args: LuaObject[]): void;
export declare function luaAssert(v: LuaObject, message?: LuaObject): LuaObject;
export declare function tostring(x: LuaObject): string;
export declare function tonumber(x: LuaObject): number | null;
export declare function type(x: LuaObject): string;
