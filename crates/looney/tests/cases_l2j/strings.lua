local s = "hello" .. " " .. "world"
assert(s == "hello world")
assert(#s == 11)
local t = "x" .. 1.5
assert(t == "x1.5")
local n = 1 .. ""
assert(n == "1")
assert("a" < "b")
assert("abc" <= "abc")
assert("b" > "a")
assert("esc\n\ttab" == "esc\n\ttab")
assert('single \'quote\'' == "single 'quote'")
assert("\65\066\x43" == "ABC")
print("ok")
