let x = 1 > 2 ? "big" : "small";
assert(x === "small");
let y = 0 ? 1 : 2;
assert(y === 2);
let z = "" ? "t" : "f";
assert(z === "f");
let w = null ? 1 : undefined ? 2 : 3;
assert(w === 3);
console.log("ok");
