-- Ported to neori as `neori.util.dump` from awesome's `gears.debug.dump_return`, by way of cwc
-- (https://github.com/Cudiph/cwcwm).
--
-- Copyright the awesome authors (https://awesomewm.org), licensed under the GNU GPL version 2 or
-- later; this file is distributed with neori under the GNU GPL version 3 or later.
---------------------------------------------------------------------------
-- Utility functions to make development easier.
--
-- @author Uli Schlachter
-- @copyright 2010 Uli Schlachter
---------------------------------------------------------------------------

local tostring = tostring
local type = type
local pairs = pairs

--- Given a table (or any other data) return a string that contains its
-- tag, value and type. If data is a table then recursively call `dump_raw`
-- on each of its values.
-- @param data Value to inspect.
-- @param shift Spaces to indent lines with.
-- @param tag The name of the value.
-- @tparam[opt=10] int depth Depth of recursion.
-- @return a string which contains tag, value, value type and table key/value
-- pairs if data is a table.
local function dump_raw(data, shift, tag, depth)
    depth = depth == nil and 10 or depth or 0
    local result = ""

    if tag then
        result = result .. tostring(tag) .. " : "
    end

    if type(data) == "table" and depth > 0 then
        shift = (shift or "") .. "  "
        result = result .. tostring(data)
        for k, v in pairs(data) do
            result = result .. "\n" .. shift .. dump_raw(v, shift, k, depth - 1)
        end
    else
        result = result .. tostring(data) .. " (" .. type(data) .. ")"
        if depth == 0 and type(data) == "table" then
            result = result .. " […]"
        end
    end

    return result
end

--- Inspect the value in data.
-- @param data Value to inspect.
-- @param tag The name of the value.
-- @tparam[opt] int depth Depth of recursion.
-- @treturn string A string that contains the expanded value of data.
-- @staticfct neori.util.dump
return function(data, tag, depth)
    return dump_raw(data, nil, tag, depth)
end
