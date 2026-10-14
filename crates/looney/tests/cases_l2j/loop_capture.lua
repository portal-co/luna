-- Numeric-for loop variables are fresh per iteration in Lua; closures
-- capture per-iteration values (JS `let` in for-init matches).
local f
local g
for i = 1, 2 do
	local h = function()
		return i
	end
	if i == 1 then
		f = h
	else
		g = h
	end
end
assert(f() == 1)
assert(g() == 2)
print("ok")
