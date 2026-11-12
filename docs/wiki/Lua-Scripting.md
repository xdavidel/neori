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

Because the call is synchronous and on the compositor thread, keep it short — do slow
work by spawning a helper process (`neori.action("spawn", { … })`) instead of blocking in
Lua. A long-running function blocks input, and there is no Lua-side timer API today.

Function binds take no per-bind options — the compositor accepts `(key, function)` only.
The flags that declarative table binds carry (`allow_when_locked`, `allow_inhibiting`,
`repeat`, `cooldown_ms`, `hotkey_overlay_title`) have no function-bind equivalent today.
In particular, a function bind is **never** counted as "allowed when locked" or
"allowed during screenshot" by the default allowlist — the compositor cannot predict
what the function will do. If you need either, bind a declarative action that already
has those flags set, and route your custom behaviour through that.

Lua's built-in `print(...)` is overridden to append to a log file; see
[`print(...)` output](#print-output) below.

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

| event                | arg                                                 |
|---                   |---                                                  |
| `startup`            | — (no arg)                                          |
| `monitor-added`      | string, the connector's name; second arg: `Output`  |
| `monitor-removed`    | string, the connector's name; second arg: `Output`  |
| `lid-open`           | — (no arg)                                          |
| `lid-closed`         | — (no arg)                                          |
| `tablet-mode-on`     | — (no arg)                                          |
| `tablet-mode-off`    | — (no arg)                                          |
| `window-opening`     | `Window` userdata (pre-placement — only `app_id` and `title` are populated); the handler can call `w:open_*` methods to drive initial placement |
| `window-opened`      | `Window` userdata — see the next section            |
| `window-closed`      | `Window` userdata                                   |
| `window-focused`     | `Window` userdata                                   |
| `workspace-focused`  | `Workspace` userdata — the newly active workspace   |
| `workspace-added`    | `Workspace` userdata — the just-created workspace   |
| `workspace-removed`  | `Workspace` userdata — last-known snapshot of the destroyed workspace |
| `output-focused`     | `Output` userdata — the newly active output         |
| `layout-changed`     | string — the new active keyboard layout's name      |

Workspace / output lifecycle events fire whenever the compositor notices a
change (per refresh tick), so a bind that focuses a different workspace,
plugging in a monitor, or opening a window on a not-yet-materialized workspace
all generate the matching events.

## Other `neori.*` entry points

Beyond the three runtime hooks above (`bind`, `action`, `on`), `neori.*` exposes the
programmatic equivalents of the declarative config. These are what `config.lua` uses to
declare its static setup — they run at parse time, not during callbacks. The compositor
accepts them anywhere in the script, so you can mix declarative blocks with Lua
conditionals to build the config dynamically.

### Action-builder helpers

Return an action table suitable for `neori.bind`, `neori.on`, or `neori.action`:

| call                                                           | what it returns                         |
|---                                                             |---                                      |
| `neori.spawn("kitty", "-e", "sh")`                             | `spawn` action — exec directly          |
| `neori.spawn_sh("waybar | logger")`                            | `spawn` action — via `sh -c`            |
| `neori.create_virtual_output { name = …, width = …, height = …, refresh_rate = … }` | `create-virtual-output` action |
| `neori.remove_virtual_output("name")` or `{ name = "…" }`      | `remove-virtual-output` action          |

Any extra string-keyed fields on the trailing table (`hotkey_overlay_title`,
`cooldown_ms`, `["repeat"] = false`, …) are merged in as bind metadata, so
`neori.bind("Mod+T", neori.spawn("kitty", { hotkey_overlay_title = "Terminal" }))`
works. See [Virtual Outputs](Virtual-Outputs.md) for the create/remove option shapes.

### Declarative config, from Lua

Every top-level section in the declarative config is also callable as `neori.<section>(table)`,
and merges into whatever the file has already set. Repeated calls layer: the last write
wins per field. Entry-collections that take names use the singular form, listed last.

| section setter                                                 | documents                                        |
|---                                                             |---                                               |
| `neori.input { … }`                                            | [Configuration: Input](Configuration:-Input.md)  |
| `neori.layout { … }`                                           | [Configuration: Layout](Configuration:-Layout.md) |
| `neori.cursor { … }`                                           | [Configuration: Input](Configuration:-Input.md) (cursor subsection) |
| `neori.binds { … }`                                            | [Configuration: Key Bindings](Configuration:-Key-Bindings.md) |
| `neori.animations { … }`                                       | [Configuration: Animations](Configuration:-Animations.md) |
| `neori.gestures { … }`                                         | [Configuration: Gestures](Configuration:-Gestures.md) |
| `neori.switch_events { … }`                                    | [Configuration: Switch Events](Configuration:-Switch-Events.md) |
| `neori.recent_windows { … }`                                   | [Configuration: Recent Windows](Configuration:-Recent-Windows.md) |
| `neori.hotkey_overlay { … }`                                   | [Configuration: Miscellaneous](Configuration:-Miscellaneous.md) |
| `neori.overview { … }`, `neori.blur { … }`, `neori.clipboard { … }`, `neori.config_notification { … }`, `neori.xwayland_satellite { … }`, `neori.environment { … }`, `neori.debug { … }` | [Configuration: Miscellaneous](Configuration:-Miscellaneous.md) |
| `neori.prefer_no_csd(true)` / `neori.screenshot_path("…")`     | [Configuration: Miscellaneous](Configuration:-Miscellaneous.md) |
| `neori.window_rule { … }` (per-rule)                           | [Configuration: Window Rules](Configuration:-Window-Rules.md) |
| `neori.layer_rule { … }` (per-rule)                            | [Configuration: Layer Rules](Configuration:-Layer-Rules.md) |
| `neori.output("name", { … })`                                  | [Configuration: Outputs](Configuration:-Outputs.md) |
| `neori.workspace("name", { … })`                               | [Configuration: Named Workspaces](Configuration:-Named-Workspaces.md) |

The plural `neori.outputs` / `neori.workspaces` / `neori.windows` are **runtime lookup
functions** — they return the current snapshot arrays and are documented in the sections
below. There is no `neori.outputs(table)` / `neori.workspaces(table)` section setter;
declare entries one by one with the singular form instead.

### Parse-time utilities

| call                                                           | what it does                            |
|---                                                             |---                                      |
| `neori.set("key", value)`                                      | low-level: write directly into the config data table (escape hatch for a field no section setter covers) |
| `neori.strict(true)` / `neori.strict(false)` / `neori.strict()` | toggle strict parsing — unknown config keys become errors instead of warnings |
| `neori.include("other.lua", "snippets/*.lua", "~/dir/")`       | evaluate other Lua files in the same environment; see [Configuration: Include](Configuration:-Include.md) for path rules |

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

## Pre-placement window rules (`window-opening`)

`neori.on("window-opening", function(w) … end)` fires right **before** the
compositor places a new window. The handler receives a pre-placement `Window`
userdata whose `app_id` and `title` reflect the client's current state; the
rest of the fields (workspace, output, id) are placeholders because the
window hasn't been placed yet. Call any of the `w:open_*` methods to steer
initial placement — they override the matching fields in the static
`window_rule { … }` resolution for this one window.

```lua
neori.on("window-opening", function(w)
    if w:app_id() == "firefox" then
        w:open_on_workspace("browser")
        w:open_fullscreen(true)
    end
    if (w:title() or ""):match("Picture in Picture") then
        w:open_floating(true)
    end
end)
```

`w:open_*` methods only have an effect from inside a `window-opening`
handler — a stashed `Window` from any other callback calling them is a
silent no-op, so a stray write can never leak onto an unrelated future
window.

| method                                | effect                                                       |
|---                                    |---                                                           |
| `w:open_on_output("HDMI-A-1")`        | Open on this output. Overrides the static `open-on-output`.  |
| `w:open_on_workspace("browser")`      | Open on this named workspace. Overrides `open-on-workspace`. |
| `w:open_fullscreen(true/false)`       | Open fullscreen (or not). Default `true` when no arg given.  |
| `w:open_maximized(true/false)`        | Open maximized (full-width). Default `true` when no arg.     |
| `w:open_floating(true/false)`         | Open floating / tiled. Default `true` when no arg.           |

Static `window_rule { … }` entries still apply — the Lua handler composes on
top, and anything it sets wins. The `window-opened` event still fires after
placement, so you can keep using it for post-placement side effects
(notifications, focus swaps, …) that don't belong in placement decisions.

## Pointer & keyboard

Inside any Lua callback, `neori.pointer()` returns the current seat pointer
position as `{ x = …, y = …, output = <connector-name or nil> }`, or `nil` if
the cache hasn't been seeded yet (shouldn't happen in a running compositor).
The coordinates are in the compositor's global logical space — the same space
window / output `position` fields use. `neori.warp(x, y)` queues a cursor warp
to those coordinates; the compositor drains the queue after the callback
returns.

```lua
-- Cursor-follows-focus: snap the pointer to the center of the focused
-- output whenever the active output changes.
neori.on("output-focused", function(out)
    local pos = out:position()
    if not pos then return end
    local mode = (out:mode() or ""):match("^(%d+)x(%d+)")
    -- mode has width/height; if we can't parse it, just center on the origin.
    local w, h = 0, 0
    for a, b in (out:mode() or ""):gmatch("(%d+)x(%d+)") do w, h = tonumber(a), tonumber(b) end
    local scale = out:scale() or 1
    neori.warp(pos.x + (w / scale) / 2, pos.y + (h / scale) / 2)
end)
```

`neori.keyboard_layout()` returns the active xkb layout's name (as it appears
in the compositor's keymap, e.g. `"English (US)"`, `"Russian"`). The
`layout-changed` event fires whenever the active layout switches, with the
new name as its string argument.

```lua
neori.on("layout-changed", function(name)
    neori.action("spawn-sh", string.format(
        "notify-send layout %q", name))
end)
```

## Input devices

`neori.devices()` returns an array of `Device` userdata — one per libinput
device the compositor is tracking (keyboards, mice, touchpads, trackballs,
touch panels). Each snapshot carries metadata and capability bits; mutation
methods call the matching libinput tuning setters on the real device.

```lua
-- Disable the laptop's built-in keyboard whenever an external one is present.
neori.bind("Mod+Alt+K", function()
    local has_external = false
    for _, d in ipairs(neori.devices()) do
        if d:is_keyboard() and not d:name():match("Translated Set") then
            has_external = true
            break
        end
    end
    for _, d in ipairs(neori.devices()) do
        if d:is_keyboard() and d:name():match("Translated Set") then
            d:set_disabled(has_external)
        end
    end
end)

-- Tune every touchpad.
neori.on("startup", function()
    for _, d in ipairs(neori.devices()) do
        if d:is_touchpad() then
            d:set_tap(true)
            d:set_natural_scroll(true)
            d:set_accel(0.3)
        end
    end
end)
```

### `Device` methods

Read-only:

| method               | returns                                                       |
|---                   |---                                                            |
| `d:name()`           | libinput device name string                                   |
| `d:sysname()`        | udev sysname (e.g. `"event5"`) — stable per-boot              |
| `d:product_id()`     | integer                                                       |
| `d:vendor_id()`      | integer                                                       |
| `d:is_pointer()`     | boolean — mouse, trackball, touchpad, etc.                    |
| `d:is_keyboard()`    | boolean                                                       |
| `d:is_touch()`       | boolean — touch panel                                         |
| `d:is_touchpad()`    | boolean — a pointer with non-zero `tap_finger_count`          |

Mutation:

| method                           | effect                                                           |
|---                               |---                                                               |
| `d:set_tap(bool)`                | Touchpad tap-to-click.                                           |
| `d:set_natural_scroll(bool)`     | "Natural" (macOS-style) scroll direction.                        |
| `d:set_accel(f64)`               | Pointer acceleration speed (−1.0 fastest … 1.0 slowest).         |
| `d:set_left_handed(bool)`        | Swap left/right mouse buttons.                                   |
| `d:set_disabled(bool)`           | Set libinput `send_events` to DISABLED / ENABLED.                |

Mutations go directly to libinput; they persist on the device until the next
config reload re-applies the static `input { … }` block, at which point the
config value takes over again. Not every setting is supported by every
device — libinput silently ignores unsupported calls.

## Reload semantics

`neori msg action reload-config` wipes the Lua state and rebuilds it from disk. Any Lua
function callbacks you had registered in the previous instance are dropped. Press the
key chord that was bound to a Lua function in the old config and nothing happens — the
new config decides what that chord does now. Same for `neori.on` handlers.

## `print(...)` output

Lua's built-in `print(...)` is overridden to append to a log file instead of the
process's stdout (which in a Wayland compositor is usually invisible). The file lives at

```
$XDG_STATE_HOME/neori/lua.log    # typically  ~/.local/state/neori/lua.log
```

Each line is prefixed with a UTC timestamp (`[YYYY-MM-DD HH:MM:SS.mmmZ]`), followed by
the arguments tab-separated, exactly as stock `print` would format them. Example (the
visible spacing between args is literal `\t`, so `grep -P '\t'` works):

```
[2026-11-12 09:17:04.221Z]  hello  42  true  nil
```

The file is opened append-only and flushed after every `print`, so `tail -f` works
directly. It is kept open across config reloads — no reopen churn, and the line order
is stable. If the file can't be opened at startup (missing state dir, permission
error), neori logs one `WARN` and `print` falls back to stdout so you still see output
somewhere. No rotation is done; delete or `truncate -s0` the file if it grows.

For structured diagnostics, prefer `error("…")` from a handler — that path goes through
neori's `tracing` log with severity and context attached, alongside the rest of the
compositor's log stream.

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

## Workspaces

Inside any Lua callback, `neori.workspaces()` returns an array of `Workspace`
userdata (global 1-based ordering — the same shape `w:workspace().idx` carries)
and `neori.focused_workspace()` returns the currently focused one (or `nil`).

```lua
-- Rename the active workspace to the focused window's app_id (handy with a
-- scratchpad / project workflow).
neori.bind("Mod+Alt+R", function()
    local ws = neori.focused_workspace()
    local w  = neori.focused_window()
    if ws and w and w:app_id() then ws:set_name(w:app_id()) end
end)

-- Clear the name again.
neori.bind("Mod+Shift+R", function()
    local ws = neori.focused_workspace()
    if ws then ws:unset_name() end
end)

-- Enumerate workspaces per output and print a tree.
neori.bind("Mod+Alt+W", function()
    for _, ws in ipairs(neori.workspaces()) do
        print(string.format(
            "%s/ws%d%s%s",
            ws:output() or "no-output",
            ws:idx(),
            ws:name() and (" [" .. ws:name() .. "]") or "",
            ws:is_focused() and " *" or ""
        ))
        for _, w in ipairs(ws:windows()) do
            print("  ", w:app_id(), w:title())
        end
    end
end)

-- React to workspace / output focus changes.
neori.on("workspace-focused", function(ws)
    neori.action("spawn", { "notify-send", "ws", ws:name() or tostring(ws:idx()) })
end)

neori.on("workspace-added", function(ws)
    print("workspace +", ws:idx(), ws:name() or "unnamed", "on", ws:output() or "-")
end)

neori.on("workspace-removed", function(ws)
    print("workspace -", ws:idx(), ws:name() or "unnamed")
end)

neori.on("output-focused", function(out)
    print("output focus:", out:name())
end)
```

### `Workspace` methods

Read-only accessors return the snapshot captured at the moment the Workspace was
constructed. Mutation methods enqueue an `Action`; the compositor drains and
dispatches them after the callback returns, just like window mutations.

**Read-only:**

| method               | returns                                                           |
|---                   |---                                                                |
| `ws:idx()`           | global 1-based index (matches `w:workspace().idx`)                |
| `ws:id()`            | stable per-session workspace id (unsigned integer)                |
| `ws:name()`          | string or `nil`                                                   |
| `ws:output()`        | connector-name string or `nil` (orphaned workspace)               |
| `ws:is_focused()`    | boolean — is this the globally active workspace?                  |
| `ws:windows()`       | array of `Window` userdata for the windows on this workspace      |

**Mutation:**

| method                              | effect                                                |
|---                                  |---                                                    |
| `ws:focus()`                        | Make this workspace the active one.                   |
| `ws:send_to_output("HDMI-A-1")`     | Move the workspace onto that output.                  |
| `ws:set_name("code")`               | Name (or rename) the workspace.                       |
| `ws:unset_name()`                   | Remove the workspace's name.                          |

Each mutation targets this workspace by its stable `WorkspaceId`, so a stashed
`Workspace` userdata keeps referring to the same workspace even if its global
index shifts (e.g. when workspaces on other monitors come and go). If the
workspace has been destroyed by the time the compositor drains the action, the
action is ignored — Lua methods never raise for that case.

## Known limitations (today)

- Read-only properties on a `Window` are snapshots. A stashed `Window` won't update
  its `app_id` / `title` / `is_fullscreen` fields when the live window changes; call
  `neori.focused_window()` / `neori.windows()` again when you need fresh state.
  Mutation methods (`w:close()`, `w:send_to_workspace()`, …) take effect against
  the compositor's current state — they don't care whether the snapshot was stale.
- Function binds have no way to set a custom hotkey-overlay title — the overlay falls
  back to a generic label because `neori.bind(key, function)` takes no options table.
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
-- flip whether it's floating. See the Window userdata + `window-opening` sections
-- above for a cleaner pattern that matches on `w:app_id()` at open time.
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
