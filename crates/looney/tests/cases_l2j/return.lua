-- `return` exits the whole chunk, even from inside a nested block.
local x = 1
do
	return
end
print("unreachable")
