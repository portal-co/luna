function fib(n) {
  if (n < 2) {
    return n;
  }
  return fib(n - 1) + fib(n - 2);
}
assert(fib(10) === 55);
const apply = (f, x) => f(x);
assert(apply((v) => v * 2, 21) === 42);
function takesTwo(a, b) {
  assert(a === 1);
  assert(typeof b === "undefined");
}
takesTwo(1);
console.log("ok");
