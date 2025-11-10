export class LuaTable {
    #contents;
    constructor(contents) {
        this.#contents = contents;
    }
    get contents() {
        return this.#contents;
    }
}
