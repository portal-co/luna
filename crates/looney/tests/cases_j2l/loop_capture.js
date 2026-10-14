// `let` in for-init is fresh per iteration: closures capture per-iteration
// values (mirrors Lua numeric-for semantics).
let f;
let g;
for (let i = 1; i <= 2; i++) {
  const h = () => i;
  if (i === 1) {
    f = h;
  } else {
    g = h;
  }
}
assert(f() === 1);
assert(g() === 2);
console.log("ok");
