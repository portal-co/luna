"use strict";
Object.defineProperty(exports, "__esModule", { value: true });
exports.LuaTable = void 0;
class LuaTable {
    #contents;
    constructor(contents) {
        this.#contents = contents;
    }
    get contents() {
        return this.#contents;
    }
}
exports.LuaTable = LuaTable;
