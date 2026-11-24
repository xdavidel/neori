-- neori.rules: window rules apply as windows open, and rules with `when` on their events.

neori.rules.add {
    where = { app_id = "^pip$" },
    set = { floating = true, opacity = 0.5 },
    run = function(w) ran = (ran or "") .. w:app_id() .. ";" end,
}

neori.rules.add {
    when = { "window-opened", "window-title-changed" },
    where = { title = "^Picture-in-Picture$" },
    set = { floating = true },
}

function run()
    test.output()

    local pip = test.window { app_id = "pip" }
    assert(pip:is_floating())
    test.eq(pip:opacity(), 0.5)
    test.eq(ran, "pip;", "run is called once, after the window opened")

    -- The regex is anchored: "pipe" isn't "pip".
    local pipe = test.window { app_id = "pipe" }
    assert(not pipe:is_floating())

    -- A title that arrives after the window opened.
    local browser = test.window { app_id = "firefox", title = "Loading" }
    assert(not browser:is_floating())
    test.title(browser, "Picture-in-Picture")
    assert(browser:is_floating())
end
