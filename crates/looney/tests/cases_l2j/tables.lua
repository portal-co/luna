local key = "computed"
local t = { 10, 20, label = "ok", [key] = 4 }
local alias = t
local nested = { inner = t }
t[1] = t[1] + 5
nested.inner[2] = 25
assert(alias[1] == 15)
assert(alias[2] == 25)
assert(t.label == "ok")
assert(t[key] == 4)
t.label = nil
assert(t.label == nil)
t.label = "ok"
assert(#t == 2)
print(t[1], t.label, t[key], #t)
print("ok")
