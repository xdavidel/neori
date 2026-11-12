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
    -- Auto-send Firefox to the "browser" workspace and fullscreen it.
    if w:app_id() == "firefox" then
        w:send_to_workspace("browser")
        w:fullscreen(true)
    end
end)

-- Toast on focus change.
neori.on("window-focused", function(w)
    neori.action("spawn-sh", string.format(
        "notify-send 'focus' '%s'", w:app_id() or "?"))
end)

-- Close every window whose app_id matches a pattern.
neori.bind("Mod+Alt+K", function()
    for _, w in ipairs(neori.windows()) do
        if (w:app_id() or ""):match("^steam_app_") then
            w:close()
        end
    end
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

Read-only accessors return an immutable snapshot of the window at the time the
Window value was created. Re-fetch via `neori.windows()` / `neori.focused_window()`
or a subsequent event if you need up-to-date state.

Mutation methods enqueue an IPC action against the window's stable id; the
compositor dispatches them after your callback returns. A stale id (window already
closed between snapshot time and the method call) is silently ignored by the
compositor's dispatch, so it's safe to stash a Window in a Lua local and call
methods on it across callbacks.

**Read-only:**

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

**Mutation:**

| method                        | effect                                                   |
|---                            |---                                                       |
| `w:close()`                   | Request client-side close (`Action::CloseWindow`).       |
| `w:focus()`                   | Focus the window (`Action::FocusWindow`).                |
| `w:fullscreen(true)`          | Enter fullscreen if not already (`Action::FullscreenWindow`). |
| `w:fullscreen(false)`         | Exit fullscreen if currently fullscreen.                 |
| `w:fullscreen()`              | Toggle fullscreen unconditionally.                       |
| `w:float(true)`               | Move to the floating layout (idempotent).                |
| `w:float(false)`              | Move to the tiling layout (idempotent).                  |
| `w:float()`                   | Toggle between floating and tiling based on the snapshot. |
| `w:send_to_workspace(n)`      | Move to workspace index `n` (1-based).                   |
| `w:send_to_workspace("name")` | Move to the named workspace.                             |
| `w:send_to_output("eDP-1")`   | Move to the named output.                                |

Set-style calls (`w:fullscreen(true)`, `w:float(false)`, …) skip the IPC round-trip
when the snapshot already matches the requested state, so repeated calls in a Lua
loop don't hammer the compositor. If the snapshot was stale (you stashed `w` and
state changed between calls), a mismatched fullscreen toggle can still land — the
float set-style methods use dedicated `MoveWindowToFloating` / `MoveWindowToTiling`
IPC actions, so they're truly idempotent regardless of staleness.

## Reload semantics

`neori msg action reload-config` wipes the Lua state and rebuilds it from disk. Any Lua
function callbacks you had registered in the previous instance are dropped. Press the
key chord that was bound to a Lua function in the old config and nothing happens — the
new config decides what that chord does now. Same for `neori.on` handlers.

## Outputs

Inside any Lua callback, `neori.outputs()` returns an array of `Output` userdata
and `neori.focused_output()` returns the focused one (or `nil`). The
`monitor-added` / `monitor-removed` event handlers receive an `Output` as their
second argument (the connector-name string is still the first argument, so old
handlers keep working).

```lua
neori.on("monitor-added", function(connector, out)
    if out:name() == "DP-3" then
        out:set_mode("3840x2160@120")
        out:set_scale(1.5)
        out:set_vrr(true)
    end
end)

-- Enumerate every output and print its current mode.
neori.bind("Mod+Alt+O", function()
    for _, out in ipairs(neori.outputs()) do
        print(out:name(), out:mode(), out:scale())
    end
end)

-- Bump the focused output's scale a notch.
neori.bind("Mod+Alt+Plus", function()
    local out = neori.focused_output()
    if out then out:set_scale((out:scale() or 1) * 1.25) end
end)
```

### `Output` methods

Read-only accessors return the snapshot captured at the moment the Output was
constructed. Mutation methods enqueue an `OutputAction`; the compositor drains
and applies them via `apply_transient_output_config` after the callback returns.

**Read-only:**

| method                 | returns                                                       |
|---                     |---                                                            |
| `out:name()`           | connector-name string (e.g. `"eDP-1"`, `"HEADLESS-1"`)        |
| `out:make()`           | string or `nil`                                               |
| `out:model()`          | string or `nil`                                               |
| `out:serial()`         | string or `nil`                                               |
| `out:mode()`           | current mode as `"WxH@HZ.000"` or `nil`                       |
| `out:modes()`          | array of mode strings                                         |
| `out:scale()`          | fractional scale number                                       |
| `out:transform()`      | `"normal"` / `"90"` / `"180"` / `"270"` / `"flipped"` / `"flipped-90"` / `"flipped-180"` / `"flipped-270"` |
| `out:position()`       | `{ x = <number>, y = <number> }` or `nil`                     |
| `out:is_focused()`     | boolean                                                       |
| `out:is_virtual()`     | boolean — true for `HEADLESS-*` outputs                       |
| `out:is_off()`         | boolean                                                       |
| `out:vrr_supported()`  | boolean                                                       |
| `out:vrr_enabled()`    | boolean                                                       |

**Mutation:**

| method                         | effect                                                      |
|---                             |---                                                          |
| `out:on()` / `out:off()`       | Enable or disable the output.                               |
| `out:enable(true/false/())`    | Set-style, with the same idempotence as `w:fullscreen`.     |
| `out:set_mode("WxH@HZ")`       | Pick a specific mode.                                       |
| `out:set_mode("auto")`         | Reset to the output's preferred mode.                       |
| `out:set_scale(1.5)`           | Set the fractional scale.                                   |
| `out:set_scale("auto")`        | Compositor-chosen scale.                                    |
| `out:set_transform("90")`      | Rotation / flip; same string set as `out:transform()`.      |
| `out:set_position({x=.., y=..})` | Set logical position.                                     |
| `out:set_position("auto")`     | Let the compositor place it.                                |
| `out:set_vrr(true)`            | Enable always-on VRR.                                       |
| `out:set_vrr(true, true)`      | Enable on-demand VRR (second arg is `on_demand`).           |
| `out:set_vrr(false)`           | Disable VRR.                                                |

All these go through the same IPC `OutputAction` pipeline that `neori msg output`
uses, so anything you can do from the command line you can now do from a Lua
function.

## Known limitations (today)

- No `Workspace` userdata yet — use `w:workspace()` (returns `{ idx, name }`) and
  `neori.action("move-window-to-workspace", n)` for structural changes.
- Read-only properties on a `Window` are snapshots. A stashed `Window` won't update
  its `app_id` / `title` / `is_fullscreen` fields when the live window changes; call
  `neori.focused_window()` / `neori.windows()` again when you need fresh state.
  Mutation methods (`w:close()`, `w:send_to_workspace()`, …) take effect against
  the compositor's current state — they don't care whether the snapshot was stale.
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
