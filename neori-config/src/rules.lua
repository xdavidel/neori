-- neori.rules: match objects by their properties when an event fires, then set properties
-- and run a function. Modelled on cwc's cuteful.rules.
--
-- The chunk receives the raw `neori` table, a regex matcher, a regex checker and a table of
-- built-in events (event name -> the class of the object it passes, or false). It returns the
-- module and the property tables, which the tests check against the stubs.

local neori, regex_match, regex_check, events = ...

-- Each class's properties: `get` maps a `where` key to the method that reads it, `set` maps a
-- `set` key to the method that sets it, and `open` (windows only) maps a `set` key to the
-- `open_*` method that sets it before the window opens.
local classes = {
    Window = {
        get = {
            id = "id",
            app_id = "app_id",
            title = "title",
            pid = "pid",
            focused = "is_focused",
            floating = "is_floating",
            fullscreen = "is_fullscreen",
            maximized = "is_maximized",
            urgent = "is_urgent",
            workspace = "workspace",
            output = "output",
            opacity = "opacity",
            parent = "parent",
        },
        set = {
            floating = "float",
            fullscreen = "fullscreen",
            maximized = "maximize",
            workspace = "send_to_workspace",
            output = "send_to_output",
            opacity = "set_opacity",
        },
        open = {
            floating = "open_floating",
            fullscreen = "open_fullscreen",
            maximized = "open_maximized",
            workspace = "open_on_workspace",
            output = "open_on_output",
        },
    },
    Output = {
        get = {
            name = "name",
            make = "make",
            model = "model",
            serial = "serial",
            mode = "mode",
            scale = "scale",
            transform = "transform",
            position = "position",
            focused = "is_focused",
            virtual = "is_virtual",
            off = "is_off",
            vrr_supported = "vrr_supported",
            vrr_enabled = "vrr_enabled",
        },
        set = {
            enabled = "enable",
            mode = "set_mode",
            scale = "set_scale",
            transform = "set_transform",
            position = "set_position",
            vrr = "set_vrr",
        },
    },
    Workspace = {
        get = {
            id = "id",
            idx = "idx",
            name = "name",
            output = "output",
            focused = "is_focused",
        },
        set = {
            name = "set_name",
            output = "send_to_output",
        },
    },
    Device = {
        get = {
            name = "name",
            sysname = "sysname",
            product_id = "product_id",
            vendor_id = "vendor_id",
            pointer = "is_pointer",
            keyboard = "is_keyboard",
            touch = "is_touch",
            touchpad = "is_touchpad",
            output = "mapped_output",
        },
        set = {
            tap = "set_tap",
            natural_scroll = "set_natural_scroll",
            accel = "set_accel",
            accel_profile = "set_accel_profile",
            left_handed = "set_left_handed",
            disabled = "set_disabled",
            scroll_method = "set_scroll_method",
            click_method = "set_click_method",
            dwt = "set_dwt",
            middle_emulation = "set_middle_emulation",
            output = "map_to_output",
        },
    },
    LayerSurface = {
        get = {
            id = "id",
            namespace = "namespace",
            layer = "layer",
            output = "output",
            pid = "pid",
            keyboard_interactivity = "keyboard_interactivity",
        },
        set = {},
    },
}

local RULE_KEYS = {
    id = true,
    where = true,
    where_any = true,
    where_not = true,
    where_not_any = true,
    set = true,
    run = true,
    when = true,
}
local CONDITIONS = { "where", "where_any", "where_not", "where_not_any" }

-- The `open_*` method that sets `key` to `value` before a window opens, if there is one.
-- `open_on_workspace` takes only a name, so an index has to wait until the window opened.
local function opener(key, value)
    if key == "workspace" and type(value) ~= "string" then
        return nil
    end
    return classes.Window.open[key]
end

local function sorted_keys(t)
    local keys = {}
    for key in pairs(t) do
        keys[#keys + 1] = key
    end
    table.sort(keys)
    return table.concat(keys, ", ")
end

local function fail(message)
    error("neori.rules: " .. message, 0)
end

-- The classes of the objects that the rule's events pass, as a list without duplicates.
local function rule_classes(when)
    local list, seen = {}, {}
    for _, event in ipairs(when) do
        local class = events[event]
        if class == nil then
            fail("unknown event '" .. tostring(event) .. "'")
        elseif class == false then
            fail("the event '" .. event .. "' doesn't pass an object to match")
        end
        if not seen[class] then
            seen[class] = true
            list[#list + 1] = class
        end
    end
    return list
end

local function check_value(where_key, key, value)
    local kind = type(value)
    if kind == "string" then
        local problem = regex_check(value)
        if problem then
            fail(where_key .. "." .. key .. ": invalid regex '" .. value .. "': " .. problem)
        end
    elseif kind == "table" then
        if #value == 0 then
            fail(where_key .. "." .. key .. ": a list of alternatives can't be empty")
        end
        for _, alternative in ipairs(value) do
            if type(alternative) == "table" then
                fail(where_key .. "." .. key .. ": alternatives can't be lists themselves")
            end
            check_value(where_key, key, alternative)
        end
    elseif kind ~= "number" and kind ~= "boolean" and kind ~= "function" then
        fail(where_key .. "." .. key .. ": expected a string (regex), number, boolean, function "
            .. "or a list of them, got " .. kind)
    end
end

local function check_conditions(class_name, rule)
    local class = classes[class_name]
    for _, condition in ipairs(CONDITIONS) do
        local values = rule[condition]
        if values ~= nil then
            if type(values) ~= "table" then
                fail("`" .. condition .. "` must be a table of property = value")
            end
            for key, value in pairs(values) do
                if not class.get[key] then
                    fail(condition .. ": " .. class_name .. " has no property '" .. tostring(key)
                        .. "' to match (properties: " .. sorted_keys(class.get) .. ")")
                end
                check_value(condition, key, value)
            end
        end
    end
end

local function check_set(class_name, set)
    if set == nil then
        return
    end
    if type(set) ~= "table" then
        fail("`set` must be a table of property = value")
    end
    local class = classes[class_name]
    for key in pairs(set) do
        if not (class.set[key] or (class.open and class.open[key])) then
            local hint = next(class.set) and " (properties: " .. sorted_keys(class.set) .. ")"
                or ""
            fail("set: " .. class_name .. " has no property '" .. tostring(key) .. "' to set"
                .. hint)
        end
    end
end

-- Checks the rule's shape and every key against the classes of its events, so that a typo is
-- an error when the rule is added instead of a rule that never matches. Returns its events.
local function check_rule(rule)
    if type(rule) ~= "table" then
        fail("a rule must be a table, got " .. type(rule))
    end
    for key in pairs(rule) do
        if not RULE_KEYS[key] then
            fail("unknown rule key '" .. tostring(key) .. "' (keys: " .. sorted_keys(RULE_KEYS)
                .. ")")
        end
    end
    if rule.set == nil and rule.run == nil then
        fail("a rule needs `set`, `run` or both")
    end
    if rule.run ~= nil and type(rule.run) ~= "function" then
        fail("`run` must be a function")
    end
    if rule.id ~= nil and type(rule.id) ~= "string" then
        fail("`id` must be a string")
    end

    local when = rule.when
    if when == nil then
        when = { "window-opened" }
    elseif type(when) == "string" then
        when = { when }
    elseif type(when) ~= "table" or #when == 0 then
        fail("`when` must be an event name or a non-empty list of them")
    end

    for _, class_name in ipairs(rule_classes(when)) do
        check_conditions(class_name, rule)
        check_set(class_name, rule.set)
    end
    for _, event in ipairs(when) do
        if event == "window-opening" then
            for key, value in pairs(rule.set or {}) do
                if not opener(key, value) then
                    fail("set: " .. key .. " = " .. tostring(value) .. " can't be set at "
                        .. "window-opening, before the window exists; leave out `when` to set "
                        .. "it once the window opened")
                end
            end
        end
    end
    return when
end

-- Reads a property for matching: a workspace is its name or index, depending on what it's
-- compared with.
local function read(obj, method, expected)
    local value = obj[method](obj)
    if type(value) == "table" and value.idx ~= nil then
        if type(expected) == "number" then
            return value.idx
        end
        return value.name
    end
    return value
end

local function value_matches(expected, actual, obj)
    local kind = type(expected)
    if kind == "function" then
        return expected(actual, obj) and true or false
    elseif kind == "string" then
        if type(actual) == "number" then
            actual = tostring(actual)
        end
        return type(actual) == "string" and regex_match(expected, actual)
    elseif kind == "boolean" then
        return (actual and true or false) == expected
    end
    return actual == expected
end

-- Whether `obj`'s property `key` matches `expected`, or any of its alternatives.
local function property_matches(obj, get, key, expected)
    local method = get[key]
    if type(expected) == "table" then
        for _, alternative in ipairs(expected) do
            if value_matches(alternative, read(obj, method, alternative), obj) then
                return true
            end
        end
        return false
    end
    return value_matches(expected, read(obj, method, expected), obj)
end

local function all_match(obj, get, values)
    for key, expected in pairs(values or {}) do
        if not property_matches(obj, get, key, expected) then
            return false
        end
    end
    return true
end

local function any_matches(obj, get, values)
    for key, expected in pairs(values) do
        if property_matches(obj, get, key, expected) then
            return true
        end
    end
    return false
end

local function matches(class, obj, rule)
    local get = class.get
    if not all_match(obj, get, rule.where) then
        return false
    end
    if rule.where_any and not any_matches(obj, get, rule.where_any) then
        return false
    end
    if rule.where_not and next(rule.where_not) and all_match(obj, get, rule.where_not) then
        return false
    end
    if rule.where_not_any and any_matches(obj, get, rule.where_not_any) then
        return false
    end
    return true
end

-- The class of a userdata, by the methods it has, for `matches` and `apply`.
local function class_of(obj)
    if type(obj) ~= "userdata" then
        return nil
    end
    local function has(method)
        local ok, value = pcall(function()
            return obj[method]
        end)
        return ok and value ~= nil
    end
    if has("app_id") then
        return "Window"
    elseif has("sysname") then
        return "Device"
    elseif has("namespace") then
        return "LayerSurface"
    elseif has("idx") then
        return "Workspace"
    elseif has("vrr_supported") then
        return "Output"
    end
    return nil
end

-- Applies `set` and calls `run`, at a stage of the rule:
-- - "open": `window-opening` of a rule without `when`: only the keys with an `open_*` method,
--   which take effect before the window's first frame.
-- - "rest": `window-opened` of a rule without `when`: the other keys, and `run`.
-- - "open-run": `window-opening` named in `when`: the `open_*` keys and `run`.
-- - "now": any other event: every key with its setter, and `run`.
local function apply(class, obj, rule, stage)
    for key, value in pairs(rule.set or {}) do
        local method
        if stage == "open" or stage == "open-run" then
            method = opener(key, value)
        elseif stage == "rest" then
            method = not opener(key, value) and class.set[key]
        else
            method = class.set[key]
        end
        if method then
            obj[method](obj, value)
        end
    end
    if rule.run and stage ~= "open" then
        rule.run(obj)
    end
end

local M = {}

-- by_event[event] is the list of { rule = rule, stage = stage } for that event, in the order
-- they were added.
local by_event = {}
local next_id = 1

local function dispatch(event, ...)
    local obj
    for i = 1, select("#", ...) do
        local arg = select(i, ...)
        if type(arg) == "userdata" then
            obj = arg
            break
        end
    end
    if obj == nil then
        return
    end
    local class = classes[events[event]]

    local errors = {}
    -- A copy, so that a rule that adds or removes rules doesn't disturb this loop.
    local entries = { table.unpack(by_event[event]) }
    for _, entry in ipairs(entries) do
        local rule = entry.rule
        local ok, problem = pcall(function()
            if matches(class, obj, rule) then
                apply(class, obj, rule, entry.stage)
            end
        end)
        if not ok then
            errors[#errors + 1] = "rule " .. rule.id .. ": " .. tostring(problem)
        end
    end
    if #errors > 0 then
        error("neori.rules: " .. table.concat(errors, "\n"), 0)
    end
end

local function listen(event, rule, stage)
    if by_event[event] == nil then
        by_event[event] = {}
        neori.on(event, function(...)
            dispatch(event, ...)
        end)
    end
    table.insert(by_event[event], { rule = rule, stage = stage })
end

---Adds a rule and returns its id.
function M.add(rule)
    local when = check_rule(rule)
    if rule.id == nil then
        rule.id = tostring(next_id)
        next_id = next_id + 1
    end

    if rule.when == nil then
        listen("window-opening", rule, "open")
        listen("window-opened", rule, "rest")
    else
        for _, event in ipairs(when) do
            listen(event, rule, event == "window-opening" and "open-run" or "now")
        end
    end
    return rule.id
end

---Removes the rule with this id, or this rule table. Returns whether one was removed.
function M.remove(id)
    local removed = false
    for _, entries in pairs(by_event) do
        for i = #entries, 1, -1 do
            local rule = entries[i].rule
            if rule == id or (type(id) == "string" and rule.id == id) then
                table.remove(entries, i)
                removed = true
            end
        end
    end
    return removed
end

local function object_class(obj)
    local name = class_of(obj)
    if name == nil then
        fail("expected a Window, Output, Workspace, Device or LayerSurface, got " .. type(obj))
    end
    return name
end

---Whether `obj` matches the rule's conditions.
function M.matches(obj, rule)
    local name = object_class(obj)
    check_conditions(name, rule)
    return matches(classes[name], obj, rule)
end

---Applies the rule to `obj` now, if it matches. Returns whether it matched.
function M.apply(obj, rule)
    local name = object_class(obj)
    check_conditions(name, rule)
    check_set(name, rule.set)
    if not matches(classes[name], obj, rule) then
        return false
    end
    apply(classes[name], obj, rule, "now")
    return true
end

return M, classes
