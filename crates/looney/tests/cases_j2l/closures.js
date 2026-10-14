function counter() {
  let n = 0;
  return function () {
    n = n + 1;
    return n;
  };
}
const c1 = counter();
const c2 = counter();
assert(c1() === 1);
assert(c1() === 2);
assert(c2() === 1);
console.log("ok");
