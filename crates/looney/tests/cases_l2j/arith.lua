local a = 1 + 2 * 3
assert(a == 7)
local b = 10 / 4
assert(b == 2.5)
local c = 2 ^ 10
assert(c == 1024)
assert(-7 % 3 == 2)
assert(7 % -3 == -2)
assert(-2 ^ 2 == -4)
assert(2 ^ 2 ^ 3 == 256)
assert(1e3 == 1000)
assert(0x10 == 16)
assert(1 / 0 > 1e308)
print("ok")
