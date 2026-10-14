local function counter()
	local n = 0
	return function()
		n = n + 1
		return n
	end
end
local c1 = counter()
local c2 = counter()
assert(c1() == 1)
assert(c1() == 2)
assert(c2() == 1)
print("ok")
