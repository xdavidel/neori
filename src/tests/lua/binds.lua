-- Function binds run when their key is pressed; their prints and errors go to the Lua log.

presses = 0
neori.bind("Mod+Alt+T", function()
    presses = presses + 1
    print("pressed", presses)
end)
neori.bind("Mod+Alt+E", function()
    error("intentional")
end)

function run()
    test.output()
    test.press("Mod+Alt+T")
    test.press("Mod+Alt+T")
    test.eq(presses, 2)

    test.press("Mod+Alt+E")
    local log = test.lua_log()
    assert(log:find("%] pressed\t1\n.*%] pressed\t2\n"), log)
    assert(log:find("error: Lua bind callback error: runtime error: .*intentional"), log)
end
