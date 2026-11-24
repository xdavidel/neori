-- neori.util on real windows: pick the window to the left of the focused one.

function run()
    test.output()
    local left = test.window { title = "left" }
    local right = test.window { title = "right" }
    test.eq(neori.focused_window():id(), right:id())

    local geometries = {}
    for _, w in ipairs(neori.windows()) do
        if w:id() ~= right:id() then geometries[w] = w:geometry() end
    end
    local target = neori.util.geometry.in_direction("left", geometries, right:geometry())
    test.eq(target:id(), left:id())

    target:focus()
    test.eq(neori.focused_window():id(), left:id())

    local text = neori.util.dump(right:geometry(), "geometry")
    assert(text:match("^geometry : table: 0x%x+\n"), text)
    assert(text:find("width : %d+ %(number%)"), text)
end
