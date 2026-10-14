// Function declarations hoist to the top of their block.
assert(even(4) === true);
function even(n) {
  if (n === 0) {
    return true;
  }
  return odd(n - 1);
}
function odd(n) {
  if (n === 0) {
    return false;
  }
  return even(n - 1);
}
console.log("ok");
