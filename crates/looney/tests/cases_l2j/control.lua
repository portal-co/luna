local x = 0
if x == 0 then
	x = 1
elseif x == 2 then
	x = 3
else
	x = 4
end
assert(x == 1)
local i = 0
while i < 5 do
	i = i + 1
end
assert(i == 5)
local total = 0
for j = 1, 10 do
	total = total + j
end
assert(total == 55)
for j = 10, 1, -2 do
	total = total + 1
end
assert(total == 60)
for j = 1, 0 do
	total = total + 100
end
assert(total == 60)
local k = 0
while true do
	k = k + 1
	if k >= 3 then
		break
	end
end
assert(k == 3)
print("ok")
