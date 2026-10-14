let x = 1;
{
  let x = 2;
  assert(x === 2);
}
assert(x === 1);
{
  x = 3;
}
assert(x === 3);
let y;
assert(typeof y === "undefined");
const z = 4;
assert(z === 4);
console.log("ok");
