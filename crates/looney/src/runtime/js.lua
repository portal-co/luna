-- js.lua — Lua-side JavaScript runtime for js2lua output (Luna Subset v0,
-- Phase 1 surface). Prepended to (or preloaded before) compiled JS chunks.
-- Canonical number formatting is JS-style (see docs/SUBSET.md §Numbers).

-- `undefined` cannot be `nil` (nil is taken by `null`, and Lua tables cannot
-- store nil), so it is a unique sentinel table.
jsUndefined = setmetatable({}, {
  __tostring = function() return "undefined" end,
  __newindex = function() error("cannot modify `undefined`", 2) end,
  __metatable = false,
})
jsNaN = 0 / 0
jsInfinity = math.huge

local function isNaN(x)
  return type(x) == "number" and x ~= x
end

-- JS truthiness: false, nil (null), undefined, 0, -0, NaN, "" are falsy.
function jsTruthy(x)
  if x == nil or x == false or rawequal(x, jsUndefined) then return false end
  local t = type(x)
  if t == "number" then return x ~= 0 and x == x end
  if t == "string" then return #x > 0 end
  return true
end

function jsNot(x)
  return not jsTruthy(x)
end

-- ToNumber (subset: primitives only).
function jsToNumber(x)
  local t = type(x)
  if t == "number" then return x end
  if t == "boolean" then return x and 1 or 0 end
  if x == nil then return 0 end                -- Number(null) === 0
  if rawequal(x, jsUndefined) then return jsNaN end
  if t == "string" then
    local trimmed = x:match("^%s*(.-)%s*$")
    if trimmed == "" then return 0 end         -- Number("") === 0
    local n = tonumber(trimmed)
    if n == nil then return jsNaN end
    return n
  end
  return jsNaN                                 -- objects: Phase 3+
end

-- JS-style number formatting (canonical for both runtimes).
function jsNumToStr(n)
  if isNaN(n) then return "NaN" end
  if n == math.huge then return "Infinity" end
  if n == -math.huge then return "-Infinity" end
  if n == 0 then return "0" end                -- covers -0
  if n % 1 == 0 and math.abs(n) < 1e21 then
    return string.format("%.0f", n)
  end
  return tostring(n)
end

-- ToString (subset: primitives only).
function jsToString(x)
  if rawequal(x, jsUndefined) then return "undefined" end
  if x == nil then return "null" end
  local t = type(x)
  if t == "boolean" then return x and "true" or "false" end
  if t == "number" then return jsNumToStr(x) end
  if t == "string" then return x end
  if t == "function" then return "function () { [native code] }" end
  return "[object Object]"
end

-- `+`: ToPrimitive both sides (identity for our primitives); if either is a
-- string, concatenate; otherwise numeric add.
function jsAdd(a, b)
  if type(a) == "table" and not rawequal(a, jsUndefined) then a = jsToString(a) end
  if type(b) == "table" and not rawequal(b, jsUndefined) then b = jsToString(b) end
  if type(a) == "string" or type(b) == "string" then
    return jsToString(a) .. jsToString(b)
  end
  return jsToNumber(a) + jsToNumber(b)
end

function jsSub(a, b) return jsToNumber(a) - jsToNumber(b) end
function jsMul(a, b) return jsToNumber(a) * jsToNumber(b) end
function jsDiv(a, b) return jsToNumber(a) / jsToNumber(b) end

-- JS `%` is truncated remainder (sign of dividend), unlike Lua's floored `%`.
function jsRem(a, b)
  return math.fmod(jsToNumber(a), jsToNumber(b))
end

function jsPow(a, b)
  return jsToNumber(a) ^ jsToNumber(b)
end

-- Only reachable from IR Concat, which the JS frontend never produces; kept
-- for emitter completeness (JS ToString on both sides).
function jsConcat(a, b)
  return jsToString(a) .. jsToString(b)
end

-- Strict equality: same type, same value; NaN unequal to itself; tables by
-- identity; undefined only equal to itself; null (nil) only equal to null.
function jsStrictEq(a, b)
  if type(a) ~= type(b) then return false end
  return rawequal(a, b)
end

-- Relational operators: string-vs-string compares lexicographically (byte
-- order, matching UTF-16 code units for ASCII); otherwise numeric. JS
-- relational ops yield false when either side coerces to NaN.
local function jsRelNums(a, b)
  local na, nb = jsToNumber(a), jsToNumber(b)
  if isNaN(na) or isNaN(nb) then return nil end -- nil = "unordered"
  return na, nb
end

function jsLt(a, b)
  if type(a) == "string" and type(b) == "string" then return a < b end
  local na, nb = jsRelNums(a, b)
  if na == nil then return false end
  return na < nb
end

function jsLe(a, b)
  if type(a) == "string" and type(b) == "string" then return a <= b end
  local na, nb = jsRelNums(a, b)
  if na == nil then return false end
  return na <= nb
end

function jsGt(a, b) return jsLt(b, a) end
function jsGe(a, b) return jsLe(b, a) end

function jsNeg(x) return -jsToNumber(x) end

function jsLen(x)
  if type(x) == "string" then return #x end
  error("jsLen: unsupported value of type " .. type(x), 2)
end

function jsTypeof(x)
  if rawequal(x, jsUndefined) then return "undefined" end
  if x == nil then return "object" end -- null
  local t = type(x)
  if t == "table" then return "object" end
  return t -- number/string/boolean/function already match JS spellings
end

-- Short-circuit logic with operand-value (not boolean) results. Operands are
-- thunks so the right side is only evaluated when the operator demands it.
function jsAnd(a, b)
  local av = a()
  if jsTruthy(av) then return b() else return av end
end

function jsOr(a, b)
  local av = a()
  if jsTruthy(av) then return av else return b() end
end

-- Minimal property access (plain-table objects; prototype machinery is
-- Phase 3/5). Missing keys read as `undefined`, never nil.
function jsGet(t, k)
  if type(t) ~= "table" then
    error("jsGet: cannot index " .. jsTypeof(t), 2)
  end
  local v = rawget(t, k)
  if v == nil then return jsUndefined end
  return v
end

function jsSet(t, k, v)
  if type(t) ~= "table" then
    error("jsSet: cannot index " .. jsTypeof(t), 2)
  end
  rawset(t, k, v)
  return v
end

-- console.log shim: space-joined JS ToString of each argument.
function jsLog(...)
  local parts = {}
  for i = 1, select("#", ...) do
    parts[i] = jsToString(select(i, ...))
  end
  print(table.concat(parts, " "))
end

console = { log = jsLog }
