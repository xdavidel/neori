-- Declarations called after the config loaded change the running config: from `neori msg lua`
-- (which is how run() itself runs) and from callbacks.

neori.strict(true)

presses = 0
neori.bind("Mod+Alt+T", function() presses = presses + 1 end)

-- A callback that changes the config, deep inside a section.
neori.layout { struts = { left = 0 } }
neori.bind("Mod+Alt+S", function() neori.layout.struts.left = 200 end)
-- A callback whose change doesn't parse.
neori.bind("Mod+Alt+B", function() neori.layout { gaps = "wide" } end)

local function window_x(w)
    return w:geometry().x
end

function run()
    test.output()
    local w = test.window { title = "a" }
    test.settle()
    local x = window_x(w)

    -- A new bind works right away.
    neori.bind("Mod+Alt+R", function() added = true end)
    test.press("Mod+Alt+R")
    test.eq(added, true, "a bind added at runtime runs")

    -- Binding a key again replaces its bind instead of being an error.
    neori.bind("Mod+Alt+T", function() replaced = true end)
    test.press("Mod+Alt+T")
    test.eq(replaced, true, "the new bind runs")
    test.eq(presses, 0, "the old bind doesn't")

    -- A section change applies to the layout.
    neori.layout { gaps = 64 }
    test.settle()
    assert(window_x(w) > x, "more gaps move the window: " .. window_x(w) .. " vs " .. x)
    local with_gaps = window_x(w)

    -- So does a change deep inside a section, from a callback.
    test.press("Mod+Alt+S")
    test.settle()
    assert(window_x(w) > with_gaps, "the strut moves the window")

    -- A change that doesn't parse is undone, and reported.
    test.press("Mod+Alt+B")
    test.eq(neori.layout.gaps, 64, "the bad gaps are undone")
    local log = test.lua_log()
    assert(log:find("error: Lua config change undone: "), log)
    test.press("Mod+Alt+R")
    test.eq(added, true, "the config still works after a bad change")

    -- Variables set with neori.setenv survive a config change.
    neori.setenv("NEORI_TEST_VAR", "kept")
    neori.layout { gaps = 32 }
    test.refresh()
    test.eq(neori.getenv("NEORI_TEST_VAR"), "kept")

    -- A mode can be declared again.
    neori.mode("resize", function() neori.bind("Escape", "exit-mode") end)
    neori.mode("resize", function() neori.bind("Return", "exit-mode") end)
end
