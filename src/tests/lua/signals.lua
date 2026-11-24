-- Signals: property signals get the old value, and objects get their own signals.

log = {}
local function note(text) log[#log + 1] = text end

neori.on("window-title-changed", function(w, old)
    note(tostring(old) .. " -> " .. w:title())
end)
neori.on("window-opened", function(w)
    w:connect("closed", function(win) note("closed " .. win:title()) end)
end)

function run()
    test.output()
    local w = test.window { title = "first" }
    test.eq(#log, 0, "opening isn't a title change")

    test.title(w, "second")
    test.title(w, "third")
    test.eq(log[1], "first -> second")
    test.eq(log[2], "second -> third")

    test.close(w)
    test.eq(log[3], "closed third")
    -- A Window kept from before reads what it had when it was made, not its last state; see the
    -- roadmap ("Last known state").
    test.eq(w:title(), "first")
end
