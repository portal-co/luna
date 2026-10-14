-- `function name()` (no `local`) assigns into the enclosing scope.
function gfact(n)
	if n <= 1 then
		return 1
	end
	return n * gfact(n - 1)
end
assert(gfact(5) == 120)
print("ok")
