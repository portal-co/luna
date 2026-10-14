local function fib(n)
	if n < 2 then
		return n
	end
	return fib(n - 1) + fib(n - 2)
end
assert(fib(10) == 55)
local apply = function(f, x)
	return f(x)
end
assert(apply(function(v)
	return v * 2
end, 21) == 42)
local function takes_two(a, b)
	assert(a == 1)
	assert(b == nil)
end
takes_two(1)
print("ok")
