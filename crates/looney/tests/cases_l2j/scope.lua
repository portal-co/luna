local x = 1
do
	local x = 2
	assert(x == 2)
end
assert(x == 1)
do
	x = 3
end
assert(x == 3)
local y
assert(y == nil)
print("ok")
