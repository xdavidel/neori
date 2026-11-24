-- Handlers that keep undoing each other stop at the depth limit, and the error goes to the Lua
-- log.

bounces = 0
neori.on("window-focused", function(w)
    bounces = bounces + 1
    for _, other in ipairs(neori.windows()) do
        if other:id() ~= w:id() then
            other:focus()
            return
        end
    end
end)

function run()
    test.output()
    test.window { title = "a" }
    test.window { title = "b" }
    test.refresh(100)

    assert(bounces <= 18, "the loop should stop, ran " .. bounces .. " times")
    local stopped = bounces
    test.refresh(20)
    test.eq(bounces, stopped, "it stays stopped")

    local log = test.lua_log()
    assert(log:find("error: Lua window%-focused callback dropped: Lua events caused each other 17"),
        log)
end
