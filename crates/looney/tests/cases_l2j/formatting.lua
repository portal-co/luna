-- Canonical (JS-style) number formatting. This case has a .expected file:
-- stock Lua prints some of these differently ("2.0"), and that is a
-- documented divergence, so the reference run is skipped.
print(4 / 2)
print(7 / 2)
print(2 ^ 53)
print(0.1)
print(1e21)
print(-0)
print(1 / 0)
print(0 - 1 / 0)
print(0 / 0)
print(nil)
print(true)
