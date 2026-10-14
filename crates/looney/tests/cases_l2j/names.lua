-- Names colliding with JS reserved words / runtime helpers get mangled.
local let = 5
assert(let == 5)
local luaAdd = 99
assert(luaAdd == 99)
local function_ = 6
assert(function_ == 6)
let = let + luaAdd
assert(let == 104)
print("ok")
