local obj = {
  value = 3,
  add = function(self, x)
    return self.value + x
  end,
}
assert(obj:add(4) == 7)
local calls = 0
local function make()
  calls = calls + 1
  return obj
end
assert(make():add(1) == 4)
assert(calls == 1)
obj.value = 5
print(obj:add(2))
print("ok")
