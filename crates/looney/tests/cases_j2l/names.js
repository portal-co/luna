// Names colliding with Lua keywords / runtime helpers get mangled.
let end = 5;
assert(end === 5);
let function_ = 6;
assert(function_ === 6);
end = end + function_;
assert(end === 11);
console.log("ok");
