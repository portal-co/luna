assert("10" + 5 == 15)
assert("0x10" + 0 == 16)
assert("3" * "4" == 12)
assert("  7  " - 2 == 5)
local x = "n"
x = 7
assert(x == 7)
assert(10 == 10.0)
assert(0 == -0)
print("ok")
