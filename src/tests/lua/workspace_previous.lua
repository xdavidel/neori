-- focus-workspace-previous after moving a window to another workspace returns to where it was
-- moved from.

local function ws_idx()
    return neori.focused_workspace():idx()
end

function run()
    test.output()
    local a = test.window { title = "a" }
    local b = test.window { title = "b" }
    test.eq(ws_idx(), 1)

    -- Move b to workspace 2 and follow it, leaving a behind.
    neori.action("move-column-to-workspace", 2)
    test.refresh()
    test.eq(ws_idx(), 2, "the focus follows the window")
    neori.action("focus-workspace-previous")
    test.refresh()
    test.eq(ws_idx(), 1, "previous goes back to where b came from")
    test.eq(neori.focused_window():title(), "a")

    -- Moving the last window of a workspace away: previous goes back to it, now empty.
    b:focus()
    test.refresh()
    test.eq(ws_idx(), 2)
    neori.action("move-column-to-workspace", 3)
    test.refresh()
    test.eq(ws_idx(), 3)
    neori.action("focus-workspace-previous")
    test.refresh()
    test.eq(ws_idx(), 2)
    test.eq(neori.focused_window(), nil)

    -- And previous again toggles back to where the window is.
    neori.action("focus-workspace-previous")
    test.refresh()
    test.eq(ws_idx(), 3)
    test.eq(neori.focused_window():title(), "b")

    -- The same for a single window and for moving down.
    neori.action("move-window-to-workspace-up")
    test.refresh()
    test.eq(ws_idx(), 2)
    neori.action("focus-workspace-previous")
    test.refresh()
    test.eq(ws_idx(), 3)
end
