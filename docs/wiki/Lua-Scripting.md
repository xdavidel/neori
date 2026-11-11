# Lua Scripting

Neori's `config.lua` is a real Lua program, and keybinds / event handlers can be Lua
functions. From those functions you can dispatch any IPC Action at runtime through
`neori.action(name, args)`. This turns the config from a declarative description into a
small script the compositor runs.

## `neori.bind(key, function)` — Lua function callbacks

Binds accept three action shapes: a string, a table, or a Lua function.

```lua
-- 1. string shorthand, same as before.
neori.bind("Mod+Q", "close-window")

-- 2. table action, same as before.
neori.bind("Mod+Return", neori.spawn("kitty"))
neori.bind("Mod+W", { toggle_overview = true })

-- 3. Lua function: runs when the key fires.
neori.bind("Mod+F1", function()
    print("Mod+F1 pressed")
end)
```

The compositor:

1. Stores the function in its persistent Lua host at parse time.
2. On key press, calls the function synchronously on the compositor thread.
3. Catches any Lua error, logs it with `warn!`, and briefly flashes the config-error
   notification. The compositor does not crash.

Because the call is synchronous and on the compositor thread, keep it short — do the
slow work with `neori.spawn_sh` or in a `cwc.timer`-style helper the config loads via
`neori.include`. A long-running function blocks input.

Bind options such as `allow_when_locked`, `allow_inhibiting`, `repeat`, `cooldown_ms`,
and `hotkey_overlay_title` don't change meaning on function binds. For safety, a function
bind is **never** counted as "allowed when locked" by the default allowlist (the
compositor cannot predict what the function will do) — set `allow_when_locked = true`
explicitly if you need it.

## `neori.action(name, args)` — dispatch an IPC action from Lua

Inside a Lua function bind (or a Lua event handler — see below), queue any niri-IPC
action for the compositor to run as soon as the function returns.

```lua
neori.bind("Mod+F1", function()
    neori.action("focus-column-right")
    neori.action("spawn", { "notify-send", "jumped right" })
end)
```

Shapes `name` and `args` take:

| form                                               | what it does                                |
|---                                                 |---                                          |
| `neori.action("close-window")`                     | no-arg actions (string form of the key)     |
| `neori.action("focus-workspace", 2)`               | positional primitive arg                    |
| `neori.action("spawn", { "kitty", "-e", "sh" })`   | sequence arg (same as `neori.spawn` does)   |
| `neori.action("create-virtual-output", { name = "sunshine", width = 1920 })` | table arg |

The name accepts both snake- and kebab-case (`close_window` or `close-window`), matching
the config-time action-table keys. Any action the declarative config supports is
available here.

Actions queued by one callback are drained and dispatched together, in FIFO order,
after the callback returns. If the callback errors before `return`, the actions it
queued are still dispatched — once they leave the Lua side they're committed.

## `neori.on(event, function)` — Lua event handlers

Event handlers accept the same three shapes as `neori.bind`; a Lua function argument
works today for all supported events.

```lua
neori.on("startup", function()
    neori.action("spawn", { "waybar" })
    neori.action("spawn", { "mako" })
end)

neori.on("monitor-added", function(connector)
    -- Example: send a notification whenever an output appears, and if it looks like
    -- a Sunshine streaming target, enable adaptive sync on it.
    neori.action("spawn", { "notify-send", "connected", connector })
    if connector == "sunshine" then
        neori.action("output", { name = connector, vrr = true })
    end
end)

neori.on("lid-closed", function()
    neori.action("power-off-monitors")
end)
```

Supported event names and their handler arg:

| event               | arg                                      |
|---                  |---                                       |
| `startup`           | — (no arg)                               |
| `monitor-added`     | string, the connector's name             |
| `monitor-removed`   | string, the connector's name             |
| `lid-open`          | — (no arg)                               |
| `lid-closed`        | — (no arg)                               |
| `tablet-mode-on`    | — (no arg)                               |
| `tablet-mode-off`   | — (no arg)                               |
| `window-opened`     | `Window` userdata — see the next section |
| `window-closed`     | `Window` userdata                        |
| `window-focused`    | `Window` userdata                        |

## Windows

Inside any Lua callback (bind or event), `neori.windows()` returns a snapshot array of
the mapped windows, and `neori.focused_window()` returns the focused one or `nil`.
Window event handlers receive the subject `Window` as their argument.

```lua
neori.on("window-opened", function(w)
    print("opened:", w:id(), w:app_id(), w:title())
    if w:app_id() == "firefox" and w:workspace().name ~= "browser" then
        neori.action("move-window-to-workspace", 2)
    end
end)

neori.on("window-focused", function(w)
    neori.action("spawn-sh", string.format(
        "notify-send 'focus' '%s'", w:app_id() or "?"))
end)

-- Count floating windows on the current output.
neori.bind("Mod+Alt+C", function()
    local n = 0
    for _, w in ipairs(neori.windows()) do
        if w:is_floating() then n = n + 1 end
    end
    neori.action("spawn-sh", "notify-send 'floating: " .. n .. "'")
end)
```

### `Window` methods

Every method returns an immutable snapshot of the window at the time the Window
value was created. Re-fetch via `neori.windows()` / `neori.focused_window()` or a
subsequent event if you need up-to-date state.

| method              | returns                                                    |
|---                  |---                                                         |
| `w:id()`            | stable numeric id (persists across focus/workspace moves)  |
| `w:app_id()`        | string or `nil`                                            |
| `w:title()`         | string or `nil`                                            |
| `w:is_focused()`    | boolean                                                    |
| `w:is_floating()`   | boolean                                                    |
| `w:is_fullscreen()` | boolean                                                    |
| `w:workspace()`     | `{ idx = <number>, name = <string?> }` or `nil`            |
| `w:output()`        | connector-name string or `nil`                             |

Mutation methods (`w:close()`, `w:focus()`, `w:send_to_workspace(n)`, …) are planned
for the next round; for now use `neori.action(name, args)` with the matching IPC
action to mutate state.

## Reload semantics

`neori msg action reload-config` wipes the Lua state and rebuilds it from disk. Any Lua
function callbacks you had registered in the previous instance are dropped. Press the
key chord that was bound to a Lua function in the old config and nothing happens — the
new config decides what that chord does now. Same for `neori.on` handlers.

## Known limitations (today)

- Window userdata is **read-only snapshots**. Mutation (`w:focus()`, `w:close()`,
  `w:send_to_workspace(n)`) is planned for the next round; for now, drive it via
  `neori.action("…")` with the matching IPC action.
- No `Output` / `Workspace` userdata yet — use `w:output()` / `w:workspace()` and
  `neori.action("output", { name = "…", … })` for configuration changes.
- No `neori.timer` — defer work with `neori.action("spawn-sh", "sleep 1 && …")`.
- Hotkey overlay doesn't show Lua-function binds' custom titles yet.
- Function callbacks are synchronous on the compositor thread; don't block.
- `allow_when_locked` and `allow_during_screenshot` opt-outs are off by default for
  function binds (the compositor can't inspect what the function does).

## Example config

A worked example that uses every piece of this API together:

```lua
--
-- ~/.config/neori/config.lua  (fragment)
--

-- Static binds keep working unchanged.
neori.bind("Mod+Q", "close-window")
neori.bind("Mod+Return", neori.spawn("kitty"))
neori.bind("Mod+W", { toggle_overview = true })

-- Function bind that dispatches a single action.
neori.bind("Mod+Alt+T", function()
    neori.action("spawn", { "kitty", "--title", "scratch" })
end)

-- Function bind that composes several actions atomically.
neori.bind("Mod+Alt+F", function()
    neori.action("focus-column-first")
    neori.action("maximize-column")
end)

-- Function bind that decides what to do based on compositor-independent Lua state.
local function dim_or_bright()
    local hour = tonumber(os.date("%H"))
    if hour >= 20 or hour < 7 then
        neori.action("spawn-sh", "brightnessctl set 20%")
    else
        neori.action("spawn-sh", "brightnessctl set 80%")
    end
end
neori.bind("Mod+Alt+B", dim_or_bright)

-- Autostart via neori.on("startup", ...).
neori.on("startup", function()
    neori.action("spawn", { "waybar" })
    neori.action("spawn", { "mako" })
    neori.action("spawn", { "nm-applet", "--indicator" })
end)

-- Create a virtual output for Sunshine on startup, react to connects.
neori.on("startup", function()
    neori.action("create-virtual-output", { name = "sunshine" })
end)

neori.on("monitor-added", function(connector)
    neori.action("spawn-sh", string.format(
        "notify-send 'output connected' '%s'", connector))
    if connector == "DP-3" then
        neori.action("output", {
            name = "DP-3",
            scale = 1.5,
            variable_refresh_rate = true,
        })
    end
end)

neori.on("lid-closed", function()
    -- Blank the internal panel and move its workspaces to the HDMI output.
    neori.action("power-off-monitors")
end)

-- A tiny "scratchpad" pattern: toggle a named kitty.
--
-- On the first press, spawn the window with a known class; on subsequent presses
-- flip whether it's floating. (Window-userdata will let us do this properly later.)
local scratch_spawned = false
neori.bind("Mod+Grave", function()
    if not scratch_spawned then
        neori.action("spawn", { "kitty", "--class", "neori-scratch" })
        scratch_spawned = true
    else
        neori.action("toggle-window-floating")
    end
end)

-- And the existing declarative output / window-rule config still applies.
neori.output("sunshine", {
    create_virtual = true,
    mode = "1920x1080@144.000",
})

neori.window_rule {
    matches = { { app_id = "^neori-scratch$" } },
    open_floating = true,
    open_on_output = "eDP-1",
}
```
